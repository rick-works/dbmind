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
/// 这是「内存换准确」的兜底：对比要在内存里建 key → 行 的索引，千万行的表
/// 一对比就是 OOM —— 200 万（原 20 万的 10 倍）配合分页读取，常规业务表
/// 等于没有限制；真要对比更大规模得走导出+外部工具。到了上限明确标出「结果不完整」。

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
        let engine = state.engine();
        let name = qualified_name.clone();
        let target = target.clone();
        blocking(move || engine.list_columns_fresh(&target, &name)).await?
    };
    if columns.is_empty() && qualified_name != table {
        let engine = state.engine();
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


/// 单侧数据流：协程按 keyset 分页拉取（每页 5 万），页经 channel 交给对比循环。
/// **内存只驻留当前页** —— 行数无上限、不会 OOM（流式边拉边比）。
struct SideStream {
    rx: tokio::sync::mpsc::Receiver<Result<SidePage, String>>,
    columns: Vec<String>,
    page_rows: Vec<Vec<CellValue>>,
    page_idx: usize,
    exhausted: bool,
    pub error: Option<String>,
    pub rows_seen: u64,
}

impl SideStream {
    fn new(rx: tokio::sync::mpsc::Receiver<Result<SidePage, String>>) -> Self {
        Self {
            rx,
            columns: Vec::new(),
            page_rows: Vec::new(),
            page_idx: 0,
            exhausted: false,
            error: None,
            rows_seen: 0,
        }
    }

    /// 取下一行（页尽自动翻页；行 **swap** 出来零拷贝 —— 大页不产生整页克隆）。
    /// 返回 None = 流结束（正常读完或出错，error 字段区分）。
    async fn next_row(&mut self) -> Option<Vec<CellValue>> {
        loop {
            if self.page_idx < self.page_rows.len() {
                let row = std::mem::take(&mut self.page_rows[self.page_idx]);
                self.page_idx += 1;
                self.rows_seen += 1;
                return Some(row);
            }
            if self.exhausted {
                return None;
            }
            match self.rx.recv().await {
                Some(Ok(page)) => {
                    if self.columns.is_empty() {
                        self.columns = page.columns;
                    }
                    self.page_rows = page.rows;
                    self.page_idx = 0;
                }
                Some(Err(err)) => {
                    self.error = Some(err);
                    self.exhausted = true;
                }
                None => {
                    self.exhausted = true;
                }
            }
        }
    }
}

struct SidePage {
    columns: Vec<String>,
    rows: Vec<Vec<CellValue>>,
}

/// 表行数（count(*)）：给流式对比提供**真实分母** —— 进度条按具体百分比增长
/// （用户口径：与数据传输一致，而不是一直流动动画）。带过滤条件时 count 同样带上，
/// 分母与参与对比的行集一致。count 失败返回 0 = 分母未知 → 前端流动条兜底。
async fn count_rows(
    state: &AppState,
    conn: &str,
    scope: &str,
    table: &str,
    schema: Option<&str>,
    condition: Option<&str>,
) -> u64 {
    let where_part = condition
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(|c| format!(" where {c}"))
        .unwrap_or_default();
    let schema_owned = schema.map(|s| s.to_string());
    let sql = format!("select count(*) from {}{where_part}", qualified(table, &schema_owned));
    match run_sql_in(state, conn, scope, sql, 1).await {
        Ok(result) => match result.rows.first().and_then(|r| r.first()) {
            Some(CellValue::Integer(v)) => (*v).max(0) as u64,
            Some(CellValue::Real(v)) => (*v).max(0.0) as u64,
            Some(CellValue::Text(txt)) => txt.trim().parse::<u64>().unwrap_or(0),
            _ => 0,
        },
        Err(_) => 0,
    }
}


/// keyset 分页的字面量：按排序键类型生成（数值不带引号、文本转义引号），
/// 让数据库按与内存 `SortKey` 一致的序比较（数值序/文本序）。
fn keyset_literal(part: &shape::SortPart) -> String {
    match part {
        shape::SortPart::Null => "null".to_string(),
        shape::SortPart::Num(v) => format!("{}", v.0),
        shape::SortPart::Time(t) => format!("'{}'", keyset_time_literal(t)),
        shape::SortPart::Text(t) => format!("'{}'", t.replace('\'', "''")),
    }
}

/// 规范时间 `yyyy-MM-ddTHH:mm:ss[.ffffff]` → **各方言都接受**的 SQL 字面量。
///
/// 内存归一键用 `T` 分隔，但这个形式不能直接回灌 SQL：
/// ClickHouse 的 `Date` 列对带时间（甚至带 `T`）的字符串转换直接报
/// `Cannot convert string ... to type Date`（真机踩过）；`T` 分隔在
/// MySQL/ClickHouse 的 `DateTime` 上也不认。规则：
/// - 整点零分零秒 → 去掉时间部分，只留 `'yyyy-MM-dd'`（Date/DateTime/MySQL 全兼容）
/// - 其余 → `T` 换成空格（`'yyyy-MM-dd HH:mm:ss'`，各库通用）
fn keyset_time_literal(t: &str) -> String {
    if t.len() == 19 && t.ends_with("T00:00:00") {
        t[..10].to_string()
    } else if let Some(stripped) = t.strip_suffix(".000000") {
        stripped.replacen('T', " ", 1)
    } else {
        t.replacen('T', " ", 1)
    }
}

/// 单侧数据拉取协程：keyset 分页（`WHERE … AND (key) > (last) ORDER BY key`），
/// 跨页全局有序（不只是页内）；内存里只有当前页。
fn spawn_side_stream(
    state: AppState,
    conn: String,
    scope: String,
    table: String,
    schema: Option<String>,
    condition: Option<String>,
    columns: Vec<String>,
    dialect: Dialect,
    key_cols: Vec<String>,
    task: std::sync::Arc<crate::api::tasks::Task>,
    tx: tokio::sync::mpsc::Sender<Result<SidePage, String>>,
) {
    tokio::spawn(async move {
        let list = columns
            .iter()
            .map(|name| dialect.quote(name))
            .collect::<Vec<_>>()
            .join(", ");
        let mut sql = format!("select {list} from {}", qualified(&table, &schema));
        if let Some(condition) = condition.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            sql.push_str(&format!(" where {condition}"));
        }
        let order_cols = if key_cols.is_empty() {
            columns
                .iter()
                .map(|name| dialect.quote(name))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            key_cols
                .iter()
                .map(|name| dialect.quote(name))
                .collect::<Vec<_>>()
                .join(", ")
        };
        // **sql 只拼到 where 为止（不含 order by）** —— keyset 分页条件必须插在
        // order by 之前：曾经把 order by 提前并进 sql，第二页变成
        // `... order by ... where ((keys) > (...))` → MySQL 语法错（真机踩过）。
        let page_size = 50_000usize;
        let mut last_key: Option<Vec<shape::SortPart>> = None;
        let mut columns_out: Option<Vec<String>> = None;
        loop {
            if task.check_canceled().is_err() {
                break;
            }
            // keyset 条件：上一页最后一行的键之后（首页不带，取从头开始的前 N 行）
            let page_sql = match &last_key {
                None => format!("{sql} order by {order_cols} limit {page_size}"),
                Some(key) => {
                    let lits: Vec<String> = key.iter().map(keyset_literal).collect();
                    let keyset = format!(
                        "({order_cols}) > ({lits})",
                        order_cols = order_cols,
                        lits = lits.join(", ")
                    );
                    let glue = if sql.to_lowercase().contains(" where ") { " and" } else { " where" };
                    format!("{sql}{glue} ({keyset}) order by {order_cols} limit {page_size}")
                }
            };
            match run_sql_in(&state, &conn, &scope, page_sql, page_size).await {
                Ok(result) => {
                    if columns_out.is_none() {
                        columns_out = Some(result.columns.iter().map(|c| c.name.clone()).collect());
                    }
                    let got = result.rows.len();
                    let last = result.rows.last().map(|row| {
                        shape::row_sort_key(row, &sort_positions(&key_cols, &columns))
                    });
                    let _ = tx
                        .send(Ok(SidePage {
                            columns: columns_out.clone().unwrap_or_default(),
                            rows: result.rows,
                        }))
                        .await;
                    // 不足一页 = 读完
                    if got < page_size {
                        break;
                    }
                    last_key = last;
                }
                Err(err) => {
                    let _ = tx.send(Err(err.message)).await;
                    break;
                }
            }
        }
    });
}

/// 比对键在**输出列清单**里的下标（流式页的行按这个序取键值）
fn sort_positions(key_cols: &[String], columns: &[String]) -> Vec<usize> {
    key_cols
        .iter()
        .filter_map(|key| columns.iter().position(|c| c.eq_ignore_ascii_case(key)))
        .collect()
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

/// 流式对比：两侧数据**边拉边比**（双指针消费各自的分页流），
/// 内存只驻留两侧的当前页 —— 行数无上限、不会 OOM（全量数据对比的完整实现）。
///
/// 前提：两侧的流各自按**比对键的数据库排序**流出（keyset 分页跨页全局有序），
/// 内存里用同规则的 `SortKey`（数值序/文本序）做双指针比较 —— 键序一致才能正确配对。
/// 后续写闸门语义不变；进度：每消费一行 done+1（total 在两侧行数已知后设为总数）。
#[allow(clippy::too_many_arguments)]
async fn compare_streamed(
    state: &AppState,
    source_conn: String,
    source_scope: String,
    source_table: String,
    source_schema: Option<String>,
    source_condition: Option<String>,
    source_dialect: Dialect,
    target_conn: String,
    target_scope: String,
    target_table: String,
    target_schema: Option<String>,
    target_condition: Option<String>,
    target_dialect: Dialect,
    columns: &[String],
    keys: &[String],
    excluded: &[String],
    limit: usize,
    task: &std::sync::Arc<crate::api::tasks::Task>,
    est_total: u64,
) -> XResult<(Value, u64, u64)> {
    if columns.is_empty() {
        return Err(XError::bad_request("没有可对比的列"));
    }
    // 比对键必须落在对比列清单里（页里才取得到键值）
    for key in keys {
        if !columns.iter().any(|c| c.eq_ignore_ascii_case(key)) {
            return Err(XError::bad_request(format!("比对键 {key} 不在参与对比的列里")));
        }
    }

    let (src_tx, src_rx) = tokio::sync::mpsc::channel::<Result<SidePage, String>>(2);
    spawn_side_stream(
        state.clone(), source_conn, source_scope, source_table, source_schema,
        source_condition, columns.to_vec(), source_dialect, keys.to_vec(),
        task.clone(), src_tx,
    );
    let (tgt_tx, tgt_rx) = tokio::sync::mpsc::channel::<Result<SidePage, String>>(2);
    spawn_side_stream(
        state.clone(), target_conn, target_scope, target_table, target_schema,
        target_condition, columns.to_vec(), target_dialect, keys.to_vec(),
        task.clone(), tgt_tx,
    );
    let mut source = SideStream::new(src_rx);
    let mut target = SideStream::new(tgt_rx);

    // 双指针消费（先各取一行把流"点燃"，同时拿到首页列序）
    let mut s_row = source.next_row().await;
    let mut t_row = target.next_row().await;
    if let Some(err) = source.error.clone() {
        return Err(XError::bad_request(format!("读取源数据失败：{err}")));
    }
    if let Some(err) = target.error.clone() {
        return Err(XError::bad_request(format!("读取目标数据失败：{err}")));
    }
    if s_row.is_none() && t_row.is_none() {
        return Err(XError::bad_request("两边的表都没有数据"));
    }

    // 列序初始化（首页列即全列序）
    let src_cols = source.columns.clone();
    let tgt_cols = target.columns.clone();
    let find = |cols: &[String], name: &str| {
        cols.iter().position(|c| c.eq_ignore_ascii_case(name))
    };
    let mut src_key_idx = Vec::new();
    for key in keys {
        let idx = find(&src_cols, key)
            .ok_or_else(|| XError::bad_request(format!("源表结果里没有关键列 {key}")))?;
        src_key_idx.push(idx);
    }
    let mut tgt_key_idx = Vec::new();
    for key in keys {
        let idx = find(&tgt_cols, key)
            .ok_or_else(|| XError::bad_request(format!("目标表结果里没有关键列 {key}")))?;
        tgt_key_idx.push(idx);
    }
    // 参与比值的列：两边都有、不是关键列
    let mut compare_pairs: Vec<(usize, usize, String)> = Vec::new();
    let mut excluded_columns: Vec<String> = excluded.to_vec();
    let mut excluded_set: BTreeSet<String> = excluded.iter().map(|n| n.to_ascii_lowercase()).collect();
    for (index, name) in src_cols.iter().enumerate() {
        if src_key_idx.contains(&index) {
            continue;
        }
        match find(&tgt_cols, name) {
            Some(tgt_index) => compare_pairs.push((index, tgt_index, name.clone())),
            None => {
                if !excluded_set.contains(&name.to_ascii_lowercase()) {
                    excluded_set.insert(name.to_ascii_lowercase());
                    excluded_columns.push(name.clone());
                }
            }
        }
    }

    let mut only_in_source = 0u64;
    let mut only_in_target = 0u64;
    let mut different = 0u64;
    let mut same = 0u64;
    let mut different_columns: BTreeSet<String> = BTreeSet::new();
    let mut only_source_samples: Vec<Value> = Vec::new();
    let mut only_target_samples: Vec<Value> = Vec::new();
    let mut diff_samples: Vec<Value> = Vec::new();

    let mut last_pct: u64 = 0;
    loop {
        task.check_canceled()?;
        // **行级进度上报**：每消费一行（源+目标）就把进度分子 +2（done），
        // 分母是开始时 count 的两侧行数之和（total）—— 进度条按真实百分比增长，
        // 与数据传输一致（用户口径：不要一直流动动画）
        task.add_done(2);
        // **按百分比台阶记进度日志**（每跨过 10% 一条）：两侧行数不同时，
        // 「已比对 5 万行」这种绝对数会与单侧行数对不上（真机反馈），百分比才有意义
        let seen = source.rows_seen + target.rows_seen;
        if est_total > 0 {
            let pct = (seen * 100 / est_total) as u64;
            if pct >= last_pct + 10 {
                task.log(format!("进度 {}%（两侧累计 {} 行）", pct, seen));
                last_pct = pct;
            }
        }
        match (s_row.as_ref(), t_row.as_ref()) {
            (Some(s), Some(t)) => {
                let sk = shape::row_sort_key(s, &src_key_idx);
                let tk = shape::row_sort_key(t, &tgt_key_idx);
                match sk.cmp(&tk) {
                    std::cmp::Ordering::Less => {
                        // 源有、目标没有（键序在目标流里已经过去了）
                        only_in_source += 1;
                        if only_source_samples.len() < limit {
                            only_source_samples.push(row_object(&src_cols, s, None));
                        }
                        s_row = source.next_row().await;
                    }
                    std::cmp::Ordering::Greater => {
                        only_in_target += 1;
                        if only_target_samples.len() < limit {
                            only_target_samples.push(row_object(&tgt_cols, t, None));
                        }
                        t_row = target.next_row().await;
                    }
                    std::cmp::Ordering::Equal => {
                        let mut changed: BTreeSet<usize> = BTreeSet::new();
                        for (s_index, t_index, name) in &compare_pairs {
                            if !cell_equal(s.get(*s_index), t.get(*t_index)) {
                                changed.insert(*s_index);
                                different_columns.insert(name.clone());
                            }
                        }
                        if changed.is_empty() {
                            same += 1;
                        } else {
                            different += 1;
                            if diff_samples.len() < limit {
                                // 差异样本：关键列 + 每个不同列的「源 / 目标」两个值，
                                // 一行就能看出「原来是 3、现在是 4」
                                let mut object = Map::new();
                                for idx in &src_key_idx {
                                    object.insert(
                                        src_cols[*idx].clone(),
                                        s.get(*idx).map(shape::cell_to_value).unwrap_or(Value::Null),
                                    );
                                }
                                for (s_index, t_index, name) in &compare_pairs {
                                    if !changed.contains(s_index) {
                                        continue;
                                    }
                                    object.insert(
                                        format!("{name}（源）"),
                                        s.get(*s_index).map(shape::cell_to_value).unwrap_or(Value::Null),
                                    );
                                    object.insert(
                                        format!("{name}（目标）"),
                                        t.get(*t_index).map(shape::cell_to_value).unwrap_or(Value::Null),
                                    );
                                }
                                diff_samples.push(Value::Object(object));
                            }
                        }
                        s_row = source.next_row().await;
                        t_row = target.next_row().await;
                    }
                }
            }
            (Some(s), None) => {
                only_in_source += 1;
                if only_source_samples.len() < limit {
                    only_source_samples.push(row_object(&src_cols, s, None));
                }
                s_row = source.next_row().await;
            }
            (None, Some(t)) => {
                only_in_target += 1;
                if only_target_samples.len() < limit {
                    only_target_samples.push(row_object(&tgt_cols, t, None));
                }
                t_row = target.next_row().await;
            }
            (None, None) => break,
        }
    }

    if let Some(err) = source.error.clone() {
        return Err(XError::bad_request(format!("读取源数据失败：{err}")));
    }
    if let Some(err) = target.error.clone() {
        return Err(XError::bad_request(format!("读取目标数据失败：{err}")));
    }

    let data = json!({
        "keyColumns": keys,
        "sourceRows": source.rows_seen,
        "targetRows": target.rows_seen,
        "onlyInSource": only_in_source,
        "onlyInTarget": only_in_target,
        "different": different,
        "same": same,
        "differentColumns": different_columns.into_iter().collect::<Vec<_>>(),
        "excludedColumns": excluded_columns,
        "onlyInSourceSamples": only_source_samples,
        "onlyInTargetSamples": only_target_samples,
        "diffSamples": diff_samples,
        "onlyInSourceTruncated": only_in_source > limit as u64,
        "onlyInTargetTruncated": only_in_target > limit as u64,
        "diffTruncated": different > limit as u64,
    });
    Ok((data, source.rows_seen, target.rows_seen))
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

            task.log(format!(
                "对比开始：{} . {} → {} . {}",
                body.source_database.as_deref().unwrap_or("-"), body.source_table,
                body.target_database.as_deref().unwrap_or("-"), body.target_table
            ));
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
                task.step(format!("读取源表 {}", body.source_table));
                // **先 count 两侧行数做分母**：进度条按真实百分比增长（与数据传输一致）。
                // count 失败/为 0 → 分母未知，保持流动动画兜底。
                let src_count = count_rows(
                    &state, &source_conn, &source_scope, &body.source_table,
                    body.source_schema.as_deref(), body.source_condition.as_deref(),
                ).await;
                let tgt_count = count_rows(
                    &state, &target_conn, &target_scope, &target_table,
                    body.target_schema.as_deref(), body.target_condition.as_deref(),
                ).await;
                let mut est_total = src_count + tgt_count;
                task.set_total(if est_total > 0 { est_total as i64 } else { -1 });
                task.log(format!(
                    "行数统计：源 {} 行 / 目标 {} 行",
                    src_count, tgt_count
                ));
                // 读取与对比**流式融合**：两侧各自分页拉取（协程），主循环双指针边收边比 ——
                // 内存只驻留当前页，行数无上限（进度 done 逐行累计，total 总量未知时条为流动动画）
                // 读取与对比**流式融合**：两侧各自分页拉取（协程），主循环双指针边收边比 ——
                // 内存只驻留当前页，行数无上限（done 逐行累加，进度条按 count 分母走百分比）
                let (data, source_rows, target_rows) = compare_streamed(
                    &state,
                    source_conn.clone(),
                    source_scope.clone(),
                    body.source_table.clone(),
                    body.source_schema.clone(),
                    body.source_condition.clone(),
                    source_dialect,
                    target_conn.clone(),
                    target_scope.clone(),
                    target_table.clone(),
                    body.target_schema.clone(),
                    body.target_condition.clone(),
                    target_dialect,
                    &columns,
                    &keys,
                    &excluded,
                    body.sample_limit(),
                    &task,
                    est_total as u64,
                )
                .await
                .map_err(|e| e.message)?;
                // 校正分母/分子为**实际读取行数**（count 估算与真实可能有并发差）
                est_total = source_rows + target_rows;
                task.set_total(est_total as i64);
                task.set_done((source_rows + target_rows) as u64);
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
