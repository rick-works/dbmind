//! `/api/compare/*` —— 结构与数据对比（异步任务）。
//!
//! ## 形状
//!
//! `POST /api/compare`（提交）→ `GET /api/compare/task/{taskId}`（进度 + result）→
//! `POST /api/compare/cancel/{taskId}`。单端点，不分库类型前缀：连接 ID 自己说明了一切。
//!
//! ## 四个决定
//!
//! 1. **结构差异分五组而不是一组**：类型、长度、可空性、默认值各自成组。
//!    合成一句「结构不同」等于让用户自己再去逐列比对 —— 而那正是他点这个按钮想省掉的事。
//! 2. **长度从类型名里拆**：`varchar(255)` 的 255。只比类型名会把
//!    `varchar(50)` 与 `varchar(255)` 判成相同，而这恰恰是最常见的「结构漂移」。
//! 3. **两种连接可以不同**（源/目标各一套），对比跨库、跨实例都成立 ——
//!    `scope::resolve` 负责把「连接 + 库名」变成真正可执行的连接。
//! 4. **样本有上限**（`sampleLimit`，10~500）：对比结果可能是几十万行差异，
//!    全塞给界面只会让浏览器卡死。所以**计数给全量、样本给前 N 条**，
//!    并明确标出「还有更多没显示」（`*Truncated`）。
//!
//! 二进制列不参与数据对比（`excludedColumns` 里如实列出）：把 blob 转成文本再比，
//! 结果没有任何意义，还会把内存吃光。

use std::collections::{BTreeSet, HashMap};

use axum::extract::{Path, State};
use axum::Json;
use dbmind_core::CellValue;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::meta::run_sql_in;
use crate::api::shape;
use crate::api::{blocking, require_record};
use crate::AppState;

/// 单侧最多取多少行参与对比。
///
/// 这是「内存换准确」的边界：对比要在内存里建 key → 行 的索引，
/// 不设上限的话，两张千万行的表一对比就是一次 OOM。到了上限就明确标出「结果不完整」。
const MAX_ROWS_PER_SIDE: usize = 200_000;

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CompareRequest {
    #[serde(default)]
    source_connection_id: String,
    #[serde(default)]
    target_connection_id: Option<String>,
    #[serde(default)]
    compare_mode: Option<String>,
    #[serde(default)]
    key_columns: Option<String>,
    #[serde(default)]
    sample_limit: Option<u64>,
    #[serde(default)]
    source_database: Option<String>,
    #[serde(default)]
    source_schema: Option<String>,
    #[serde(default)]
    source_table: String,
    #[serde(default)]
    target_database: Option<String>,
    #[serde(default)]
    target_schema: Option<String>,
    #[serde(default)]
    target_table: String,
    #[serde(default)]
    source_condition: Option<String>,
    #[serde(default)]
    target_condition: Option<String>,
}

impl CompareRequest {
    fn target_connection(&self) -> String {
        self.target_connection_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.source_connection_id.clone())
    }

    fn target_table(&self) -> String {
        if self.target_table.trim().is_empty() {
            self.source_table.clone()
        } else {
            self.target_table.clone()
        }
    }

    /// 「连接 + 库 + 模式」→ scope 认的库名（schema 层级的类型拼成 `库.模式`）。
    fn source_scope(&self) -> String {
        join_scope(&self.source_database, &self.source_schema)
    }

    fn target_scope(&self) -> String {
        join_scope(&self.target_database, &self.target_schema)
    }

    fn wants_structure(&self) -> bool {
        matches!(self.compare_mode.as_deref().unwrap_or("both"), "both" | "structure")
    }

    fn wants_data(&self) -> bool {
        matches!(self.compare_mode.as_deref().unwrap_or("both"), "both" | "data")
    }

    fn keys(&self) -> Vec<String> {
        self.key_columns
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .collect()
    }

    fn sample_limit(&self) -> usize {
        self.sample_limit.unwrap_or(100).clamp(10, 500) as usize
    }
}

fn join_scope(database: &Option<String>, schema: &Option<String>) -> String {
    let database = database.clone().unwrap_or_default();
    let schema = schema.clone().unwrap_or_default();
    if schema.trim().is_empty() {
        return database;
    }
    if database.trim().is_empty() {
        return schema;
    }
    format!("{database}.{schema}")
}

/// 表名（带上模式，用于元数据查询）。
fn qualified(table: &str, schema: &Option<String>) -> String {
    let schema = schema.clone().unwrap_or_default();
    if schema.trim().is_empty() || table.contains('.') {
        table.to_string()
    } else {
        format!("{schema}.{table}")
    }
}

// ------------------------------------------------------------------ 结构对比

struct ColumnFacts {
    name: String,
    /// 基础类型（去掉括号里的长度）
    base_type: String,
    /// 类型里括号中的长度（`varchar(255)` → 255）
    length: Option<i64>,
    nullable: bool,
    default_value: String,
    binary: bool,
}

fn facts_of(name: &str, type_name: Option<&str>, nullable: bool, default_value: Option<&str>) -> ColumnFacts {
    let raw = type_name.unwrap_or("").trim().to_string();
    let lower = raw.to_ascii_lowercase();
    let (base_type, length) = match (lower.find('('), lower.find(')')) {
        (Some(open), Some(close)) if close > open => {
            let inner = &lower[open + 1..close];
            let length = inner
                .split(',')
                .next()
                .and_then(|value| value.trim().parse::<i64>().ok());
            (lower[..open].trim().to_string(), length)
        }
        _ => (lower.clone(), None),
    };
    let binary = lower.contains("blob")
        || lower.contains("binary")
        || lower.contains("bytea")
        || lower.contains("image");
    ColumnFacts {
        name: name.to_string(),
        base_type,
        length,
        nullable,
        default_value: default_value.unwrap_or("").trim().to_string(),
        binary,
    }
}

async fn columns_facts(
    state: &AppState,
    conn: &str,
    scope: &str,
    table: &str,
    schema: &Option<String>,
) -> XResult<Vec<ColumnFacts>> {
    let target = crate::api::scope::resolve(state, conn, scope).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;

    // 先试「模式.表」，失败再退回「表」：JDBC 的 `DatabaseMetaData.getColumns`
    // 是把 schema 与 table 分开传的，把 `dbo.orders` 当表名交给它，各家驱动表现不一
    // （有的当字面表名找不到、有的会自己拆）。所以两条路都试，哪条通算哪条。
    let qualified_name = qualified(table, schema);
    let mut columns = {
        let engine = state.engine.clone();
        let name = qualified_name.clone();
        let target = target.clone();
        blocking(move || engine.list_columns_fresh(&target, &name)).await?
    };
    if columns.is_empty() && qualified_name != table {
        let engine = state.engine.clone();
        let name = table.to_string();
        let target = target.clone();
        columns = blocking(move || engine.list_columns_fresh(&target, &name)).await?;
    }
    if columns.is_empty() {
        return Err(XError::bad_request(format!("表 {qualified_name} 不存在或没有列信息")));
    }
    Ok(columns
        .iter()
        .map(|column| {
            facts_of(
                &column.name,
                column.type_name.as_deref(),
                column.nullable,
                column.default_value.as_deref(),
            )
        })
        .collect())
}

fn diff_pair(column: &str, source: impl Into<Value>, target: impl Into<Value>) -> Value {
    json!({ "column": column, "source": source.into(), "target": target.into() })
}

fn compare_structure(source: &[ColumnFacts], target: &[ColumnFacts]) -> Value {
    let source_map: HashMap<String, &ColumnFacts> = source
        .iter()
        .map(|column| (column.name.to_ascii_lowercase(), column))
        .collect();
    let target_map: HashMap<String, &ColumnFacts> = target
        .iter()
        .map(|column| (column.name.to_ascii_lowercase(), column))
        .collect();

    let mut common = Vec::new();
    let mut only_source = Vec::new();
    let mut only_target = Vec::new();
    let mut type_differences = Vec::new();
    let mut length_differences = Vec::new();
    let mut nullable_differences = Vec::new();
    let mut default_differences = Vec::new();

    for column in source {
        match target_map.get(&column.name.to_ascii_lowercase()) {
            Some(other) => {
                common.push(column.name.clone());
                if column.base_type != other.base_type {
                    type_differences.push(json!({
                        "column": column.name,
                        "sourceType": column.base_type,
                        "targetType": other.base_type,
                    }));
                } else if column.length != other.length {
                    // 类型相同才比长度：`varchar` vs `int` 的长度差没有意义（已在类型差异里说过）
                    length_differences.push(diff_pair(
                        &column.name,
                        json!(column.length),
                        json!(other.length),
                    ));
                }
                if column.nullable != other.nullable {
                    nullable_differences.push(diff_pair(
                        &column.name,
                        json!(column.nullable),
                        json!(other.nullable),
                    ));
                }
                if column.default_value != other.default_value {
                    default_differences.push(diff_pair(
                        &column.name,
                        json!(column.default_value),
                        json!(other.default_value),
                    ));
                }
            }
            None => only_source.push(column.name.clone()),
        }
    }
    for column in target {
        if !source_map.contains_key(&column.name.to_ascii_lowercase()) {
            only_target.push(column.name.clone());
        }
    }

    let diff_count = only_source.len()
        + only_target.len()
        + type_differences.len()
        + length_differences.len()
        + nullable_differences.len()
        + default_differences.len();
    json!({
        "sourceColumns": source.len(),
        "targetColumns": target.len(),
        "commonColumns": common,
        "onlyInSource": only_source,
        "onlyInTarget": only_target,
        "typeDifferences": type_differences,
        "lengthDifferences": length_differences,
        "nullableDifferences": nullable_differences,
        "defaultDifferences": default_differences,
        "diffCount": diff_count,
    })
}

// ------------------------------------------------------------------ 数据对比

struct Side {
    columns: Vec<String>,
    rows: Vec<Vec<CellValue>>,
    truncated: bool,
}

async fn fetch_side(
    state: &AppState,
    conn: &str,
    scope: &str,
    table: &str,
    schema: &Option<String>,
    condition: &Option<String>,
    columns: &[String],
    dialect: Dialect,
) -> XResult<Side> {
    if columns.is_empty() {
        return Err(XError::bad_request("没有可对比的列"));
    }
    let list = columns
        .iter()
        .map(|name| dialect.quote(name))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!("select {list} from {}", qualified(table, schema));
    if let Some(condition) = condition {
        let condition = condition.trim();
        if !condition.is_empty() {
            // 条件原样进语句：这是用户明确输入的比较范围（与造数的 sql 规则同理）
            sql.push_str(&format!(" where {condition}"));
        }
    }
    let result = run_sql_in(state, conn, scope, sql, MAX_ROWS_PER_SIDE).await?;
    Ok(Side {
        columns: result.columns.iter().map(|column| column.name.clone()).collect(),
        rows: result.rows.clone(),
        truncated: result.truncated,
    })
}

/// 主键值 → 一个可比较的键。
///
/// 归一规则集中在 `shape`：**数值跨变体**（MySQL `INT` → `Integer`、Oracle `NUMBER(p,s)` → `Real`）
/// 与**日期时间跨表示**（`2024-01-01` / `2024-01-01T00:00` / `...T00:00:00`）都必须得到同一个键 ——
/// 否则跨类型对比会把整表判成「仅源 / 仅目标」，而且不报错。
fn key_of(row: &[CellValue], indexes: &[usize]) -> String {
    shape::row_key(row, indexes)
}

fn row_object(columns: &[String], row: &[CellValue], only: Option<&BTreeSet<usize>>) -> Value {
    let mut object = Map::new();
    for (index, name) in columns.iter().enumerate() {
        if let Some(only) = only {
            if !only.contains(&index) {
                continue;
            }
        }
        object.insert(
            name.clone(),
            row.get(index).map(shape::cell_to_value).unwrap_or(Value::Null),
        );
    }
    Value::Object(object)
}

/// 两格是否相等（跨类型语义，规则见 `shape::cells_equal`）：
/// 数值跨变体按数值比、日期时间按规范形式比，其余仍按类型严格比
/// （`INT 3` 与 `VARCHAR '3'` 依然是两种值）。
fn cell_equal(left: Option<&CellValue>, right: Option<&CellValue>) -> bool {
    shape::cells_equal(left, right)
}

#[allow(clippy::too_many_arguments)]
fn compare_data(
    source: &Side,
    target: &Side,
    keys: &[String],
    excluded: &[String],
    limit: usize,
    source_truncated: bool,
    target_truncated: bool,
) -> XResult<Value> {
    let find = |columns: &[String], name: &String| {
        columns
            .iter()
            .position(|column| column.eq_ignore_ascii_case(name))
    };
    let mut key_indexes = Vec::new();
    for key in keys {
        let source_index = find(&source.columns, key)
            .ok_or_else(|| XError::bad_request(format!("源表结果里没有关键列 {key}")))?;
        if find(&target.columns, key).is_none() {
            return Err(XError::bad_request(format!("目标表结果里没有关键列 {key}")));
        }
        key_indexes.push(source_index);
    }

    // 参与比值的列：两边都有、不是关键列、不是二进制
    let mut compare_indexes: Vec<usize> = Vec::new();
    let mut excluded_columns: Vec<String> = excluded.to_vec();
    let mut excluded_set: BTreeSet<String> = excluded.iter().map(|name| name.to_ascii_lowercase()).collect();
    for (index, name) in source.columns.iter().enumerate() {
        if key_indexes.contains(&index) {
            continue;
        }
        if !target.columns.iter().any(|column| column.eq_ignore_ascii_case(name)) {
            if !excluded_set.contains(&name.to_ascii_lowercase()) {
                excluded_set.insert(name.to_ascii_lowercase());
                excluded_columns.push(name.clone());
            }
            continue;
        }
        compare_indexes.push(index);
    }

    let target_columns = &target.columns;
    let mut print_indexes: HashMap<String, usize> = HashMap::new();
    for (index, name) in target_columns.iter().enumerate() {
        print_indexes.insert(name.to_ascii_lowercase(), index);
    }

    let mut target_map: HashMap<String, &Vec<CellValue>> = HashMap::new();
    let mut target_key_indexes = Vec::new();
    for key in keys {
        target_key_indexes.push(print_indexes[&key.to_ascii_lowercase()]);
    }
    for row in &target.rows {
        target_map.insert(key_of(row, &target_key_indexes), row);
    }

    let mut only_in_source = 0u64;
    let mut different = 0u64;
    let mut same = 0u64;
    let mut different_columns: BTreeSet<String> = BTreeSet::new();
    let mut only_source_samples: Vec<Value> = Vec::new();
    let mut diff_samples: Vec<Value> = Vec::new();
    let mut source_keys: BTreeSet<String> = BTreeSet::new();

    for row in &source.rows {
        let key = key_of(row, &key_indexes);
        source_keys.insert(key.clone());
        match target_map.get(&key) {
            None => {
                only_in_source += 1;
                if only_source_samples.len() < limit {
                    only_source_samples.push(row_object(&source.columns, row, None));
                }
            }
            Some(other) => {
                let mut changed: BTreeSet<usize> = BTreeSet::new();
                for index in &compare_indexes {
                    let name = &source.columns[*index];
                    let other_index = print_indexes[&name.to_ascii_lowercase()];
                    if !cell_equal(row.get(*index), other.get(other_index)) {
                        changed.insert(*index);
                        different_columns.insert(name.clone());
                    }
                }
                if changed.is_empty() {
                    same += 1;
                } else {
                    different += 1;
                    if diff_samples.len() < limit {
                        // 差异样本：关键列 + 每个不同列的「源 / 目标」两个值。
                        // 这样一行就能看出「原来是 3、现在是 4」，比只给一个值有用得多。
                        let mut object = Map::new();
                        for index in key_indexes.iter() {
                            object.insert(
                                source.columns[*index].clone(),
                                row.get(*index).map(shape::cell_to_value).unwrap_or(Value::Null),
                            );
                        }
                        for index in &changed {
                            let name = &source.columns[*index];
                            let other_index = print_indexes[&name.to_ascii_lowercase()];
                            object.insert(
                                format!("{name}（源）"),
                                row.get(*index).map(shape::cell_to_value).unwrap_or(Value::Null),
                            );
                            object.insert(
                                format!("{name}（目标）"),
                                other
                                    .get(other_index)
                                    .map(shape::cell_to_value)
                                    .unwrap_or(Value::Null),
                            );
                        }
                        diff_samples.push(Value::Object(object));
                    }
                }
            }
        }
    }

    let mut only_in_target = 0u64;
    let mut only_target_samples: Vec<Value> = Vec::new();
    for row in &target.rows {
        let key = key_of(row, &target_key_indexes);
        if source_keys.contains(&key) {
            continue;
        }
        only_in_target += 1;
        if only_target_samples.len() < limit {
            only_target_samples.push(row_object(&target.columns, row, None));
        }
    }

    Ok(json!({
        "keyColumns": keys,
        "sourceRows": source.rows.len(),
        "targetRows": target.rows.len(),
        "onlyInSource": only_in_source,
        "onlyInTarget": only_in_target,
        "different": different,
        "same": same,
        "differentColumns": different_columns.into_iter().collect::<Vec<_>>(),
        "excludedColumns": excluded_columns,
        "onlyInSourceSamples": only_source_samples,
        "onlyInTargetSamples": only_target_samples,
        "diffSamples": diff_samples,
        "onlyInSourceTruncated": only_in_source > limit as u64 || source_truncated,
        "onlyInTargetTruncated": only_in_target > limit as u64 || target_truncated,
        "diffTruncated": different > limit as u64,
    }))
}

// ------------------------------------------------------------------ 处理器

/// `POST /api/compare` —— 提交对比任务。
pub async fn start(
    State(state): State<AppState>,
    Json(body): Json<CompareRequest>,
) -> XResult<Json<Value>> {
    if body.source_table.trim().is_empty() {
        return Err(XError::bad_request("缺少 sourceTable 参数"));
    }
    if body.wants_data() && body.keys().is_empty() {
        return Err(XError::bad_request(
            "数据对比需要指定关键列（keyColumns）：没有它无法判断「哪一行对应哪一行」",
        ));
    }
    let source_conn = body.source_connection_id.clone();
    if source_conn.trim().is_empty() {
        return Err(XError::bad_request("缺少 sourceConnectionId 参数"));
    }
    require_record(&state, &source_conn).await?;
    let target_conn = body.target_connection();
    require_record(&state, &target_conn).await?;

    let registry = state.tasks.clone();
    let task = registry.spawn("compare", "cmp_", move |task| {
        let state = state.clone();
        async move {
            let source_scope = body.source_scope();
            let target_scope = body.target_scope();
            let target_table = body.target_table();
            task.set_context(json!({
                "sourceTable": body.source_table,
                "targetTable": target_table,
            }));
            let source_kind = require_record(&state, &source_conn)
                .await
                .map_err(|e| e.message)?
                .kind();
            let source_dialect = Dialect::new(source_kind);
            let target_dialect = Dialect::new(
                require_record(&state, &target_conn)
                    .await
                    .map_err(|e| e.message)?
                    .kind(),
            );

            let source_facts = columns_facts(
                &state,
                &source_conn,
                &source_scope,
                &body.source_table,
                &body.source_schema,
            )
            .await
            .map_err(|e| e.message)?;
            let target_facts = columns_facts(
                &state,
                &target_conn,
                &target_scope,
                &target_table,
                &body.target_schema,
            )
            .await
            .map_err(|e| e.message)?;

            let mut result = Map::new();
            result.insert("success".to_string(), json!(true));
            result.insert(
                "compareMode".to_string(),
                json!(body.compare_mode.clone().unwrap_or_else(|| "both".to_string())),
            );

            if body.wants_structure() {
                // 结构对比是"一次性算完"的，没有逐表进度可分；但**分母必须给**、结束要补分子，
                // 否则进度条会一直停在 0/1 —— 用户看到的是"卡住了"，而其实早就算完了
                // （实测截图就是这样：12 张公共表已对比完、却显示 (0/1)）。
                task.set_total(1);
                // 只说阶段名：前缀「正在对比：」由界面统一给，写成「对比表结构」会渲染成
                // 「正在对比：对比表结构」（同 sync 那边刚修的问题）
                task.step("表结构");
                let structure = compare_structure(&source_facts, &target_facts);
                task.set_done(1);
                task.set_phase(format!(
                    "结构差异 {} 处",
                    structure["diffCount"].as_i64().unwrap_or(0)
                ));
                result.insert("structure".to_string(), structure);
            }

            if body.wants_data() {
                let keys = body.keys();
                // 用公共列（两边都有）参与对比：只有一边有的列没法比
                let target_names: Vec<String> = target_facts
                    .iter()
                    .map(|column| column.name.to_ascii_lowercase())
                    .collect();
                let mut columns: Vec<String> = Vec::new();
                let mut excluded: Vec<String> = Vec::new();
                for column in &source_facts {
                    if !target_names.contains(&column.name.to_ascii_lowercase()) {
                        continue;
                    }
                    if column.binary {
                        // 二进制列不参与：转成文本再比没有任何意义
                        excluded.push(column.name.clone());
                        continue;
                    }
                    columns.push(column.name.clone());
                }
                task.step(format!("读取源表 {}（最多 {} 行）", body.source_table, MAX_ROWS_PER_SIDE));
                let source_side = fetch_side(
                    &state,
                    &source_conn,
                    &source_scope,
                    &body.source_table,
                    &body.source_schema,
                    &body.source_condition,
                    &columns,
                    source_dialect,
                )
                .await
                .map_err(|e| e.message)?;
                task.check_canceled()?;
                task.step(format!("读取目标表（源 {} 行）", source_side.rows.len()));
                let target_side = fetch_side(
                    &state,
                    &target_conn,
                    &target_scope,
                    &target_table,
                    &body.target_schema,
                    &body.target_condition,
                    &columns,
                    target_dialect,
                )
                .await
                .map_err(|e| e.message)?;
                task.check_canceled()?;
                task.set_total(source_side.rows.len() as i64);
                task.set_done(source_side.rows.len() as u64);
                task.step("对比数据");
                let data = compare_data(
                    &source_side,
                    &target_side,
                    &keys,
                    &excluded,
                    body.sample_limit(),
                    source_side.truncated,
                    target_side.truncated,
                )
                .map_err(|e| e.message)?;
                task.set_phase(format!(
                    "数据对比完成：{} 行不同 / {} 行相同 / 仅源 {} / 仅目标 {}",
                    data["different"], data["same"], data["onlyInSource"], data["onlyInTarget"]
                ));
                result.insert("data".to_string(), data);
            }

            task.set_message("对比完成".to_string());
            Ok(Some(Value::Object(result)))
        }
    });

    Ok(Json(json!({ "success": true, "taskId": task.id })))
}

/// `GET /api/compare/task/{taskId}` —— 进度。
pub async fn status(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    let Some(task) = state.tasks.get(&task_id) else {
        return Ok(Json(json!({
            "success": false,
            "status": "notfound",
            "message": "任务不存在或已过期（任务只保留 30 分钟）",
        })));
    };
    let mut view = task.snapshot();
    view["success"] = json!(true);
    //上游这个域叫 `current`（当前在做什么），我们的快照里叫 `phase`
    view["current"] = view["phase"].clone();
    if let Some(result) = task.result() {
        view["result"] = result;
    }
    Ok(Json(view))
}

/// `POST /api/compare/cancel/{taskId}` —— 请求取消。
pub async fn cancel(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    match state.tasks.get(&task_id) {
        Some(task) => {
            task.cancel();
            Ok(Json(json!({ "success": true, "message": "已请求取消" })))
        }
        None => Ok(Json(json!({ "success": false, "message": "任务不存在或已过期" }))),
    }
}
