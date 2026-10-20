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

use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::Json;
use dbmind_core::CellValue;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

use crate::api::dialect::{Dialect, Meta};
use crate::api::error::{XError, XResult};
use crate::api::meta::{run_sql_in, run_sql_in_background};
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
/// 表名（带上模式，用于**拼 SQL**）。
///
/// 标识符必须走方言引号：保留字 / 含特殊字符 / 大小写敏感的表名裸拼会直接语法错
///（同项目的 `export.rs::qualified` 就是这么做的）。
///
/// ⚠ 只给"拼进 SQL 文本"的地方用；喂给 JDBC `getColumns` 之类**元数据 API** 时不能加引号
///（那边要的是裸的 schema/table，加了引号反而找不到表 —— 见 columns_facts）。
/// 元数据名（schema.table）：喂给 JDBC `getColumns` 这类**元数据 API** 用 ——
/// 那边要的是**裸**的 schema/table，加引号反而找不到表。拼 SQL 请用下面的 qualified。
fn qualified_meta(table: &str, schema: &Option<String>) -> String {
    let schema = schema.clone().unwrap_or_default();
    if schema.trim().is_empty() || table.contains('.') {
        table.to_string()
    } else {
        format!("{schema}.{table}")
    }
}

fn qualified(table: &str, schema: &Option<String>, dialect: Dialect) -> String {
    let table = table.trim();
    if table.is_empty() || table.contains('.') {
        return dialect.quote(table);
    }
    let schema = schema.clone().unwrap_or_default();
    if schema.trim().is_empty() {
        dialect.quote(table)
    } else {
        dialect.quote(&format!("{schema}.{table}"))
    }
}

// ------------------------------------------------------------------ 结构对比

#[derive(Clone)]
struct ColumnFacts {
    name: String,
    /// 基础类型（去掉括号里的长度）
    base_type: String,
    /// 类型参数（`varchar(255)` → 255，`decimal(10,2)` → 10,2）
    length: Option<String>,
    nullable: bool,
    primary_key: bool,
    default_value: String,
    binary: bool,
}

fn facts_of(
    name: &str,
    type_name: Option<&str>,
    nullable: bool,
    primary_key: bool,
    default_value: Option<&str>,
) -> ColumnFacts {
    let raw = type_name.unwrap_or("").trim().to_string();
    let lower = raw.to_ascii_lowercase();
    let (base_type, length) = match (lower.find('('), lower.find(')')) {
        (Some(open), Some(close)) if close > open => {
            let inner = lower[open + 1..close]
                .split(',')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(",");
            (lower[..open].trim().to_string(), Some(inner))
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
        primary_key,
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
    // 这一份是给 JDBC 元数据 API 的，必须**裸名**（见 qualified_meta 的注释）
    let qualified_name = qualified_meta(table, schema);
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
    // MySQL 的元数据类型名常不带长度，结构对比前必须补全，否则 varchar(50)
    // 与 varchar(255) 会被判成相同。
    crate::api::meta::enrich_column_types(state, conn, scope, table, &mut columns).await;
    Ok(columns
        .iter()
        .map(|column| {
            facts_of(
                &column.name,
                column.type_name.as_deref(),
                column.nullable,
                column.primary_key,
                column.default_value.as_deref(),
            )
        })
        .collect())
}

/// 当前双指针对比只对“两侧都是非空、同类型族主键”的键做严格保证。
///
/// 文本键的数据库排序受 collation 影响，而内存里的 `SortPart::Text` 是字节序；
/// 两者顺序不一致时双指针会把同一行判成两边各有一行。这里宁可明确拒绝，
/// 也不能返回一个看起来成功、实际错误的差异结果。返回目标侧真实列名。
fn compare_sort_family(base: &str) -> Option<&'static str> {
    let base = base.to_ascii_lowercase();
    if base.is_empty()
        || ["interval", "time zone", "datetimeoffset", "timestamptz"]
            .iter()
            .any(|part| base.contains(part))
    {
        return None;
    }
    if base.contains("date")
        || base.starts_with("time")
        || base.contains("timestamp")
        || base.contains("datetime")
    {
        return Some("time");
    }
    // 用前缀而不是 contains：`point` 也包含 "int"，把几何类型当成数值键
    // 会让双指针用错比较规则。
    const NUMBER_PREFIXES: [&str; 16] = [
        "tinyint", "smallint", "mediumint", "bigint", "integer", "int", "serial",
        "bigserial", "numeric", "decimal", "dec", "number", "real", "double",
        "float", "money",
    ];
    if NUMBER_PREFIXES.iter().any(|part| base.starts_with(part))
        || base.starts_with("smallmoney")
        || base == "year"
    {
        return Some("number");
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CompareKeyMode {
    Ordered,
    Hashed,
}

#[derive(Debug)]
struct CompareKeyPlan {
    target_keys: Vec<String>,
    mode: CompareKeyMode,
}

fn validate_compare_keys(
    source: &[ColumnFacts],
    target: &[ColumnFacts],
    keys: &[String],
) -> XResult<CompareKeyPlan> {
    let source_pk: BTreeSet<String> = source
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.to_ascii_lowercase())
        .collect();
    let target_pk: BTreeSet<String> = target
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.to_ascii_lowercase())
        .collect();
    let selected: BTreeSet<String> = keys.iter().map(|key| key.to_ascii_lowercase()).collect();
    if selected.len() != keys.len() {
        return Err(XError::bad_request("比对键不能重复"));
    }
    if source_pk.is_empty() || selected != source_pk || selected != target_pk {
        return Err(XError::bad_request(format!(
            "比对键必须完整等于两侧主键。源主键 [{}]，目标主键 [{}]，当前选择 [{}]",
            source_pk.into_iter().collect::<Vec<_>>().join(", "),
            target_pk.into_iter().collect::<Vec<_>>().join(", "),
            selected.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    let mut target_keys = Vec::with_capacity(keys.len());
    let mut mode = CompareKeyMode::Ordered;
    for key in keys {
        let source_col = source
            .iter()
            .find(|column| column.name.eq_ignore_ascii_case(key))
            .ok_or_else(|| XError::bad_request(format!("源表没有比对键 {key}")))?;
        let target_col = target
            .iter()
            .find(|column| column.name.eq_ignore_ascii_case(key))
            .ok_or_else(|| XError::bad_request(format!("目标表没有比对键 {key}")))?;
        if source_col.nullable || target_col.nullable {
            return Err(XError::bad_request(format!(
                "比对键 {key} 可空，当前无法保证 NULL 跨库排序一致；请改用非空主键"
            )));
        }
        let source_family = compare_sort_family(&source_col.base_type);
        let target_family = compare_sort_family(&target_col.base_type);
        // 所有键列必须选择同一种执行模式；混合数值/文本复合键走哈希分区，
        // 不依赖任何一侧的排序规则。
        if source_family.is_none() || source_family != target_family {
            mode = CompareKeyMode::Hashed;
        }
        target_keys.push(target_col.name.clone());
    }
    Ok(CompareKeyPlan {
        target_keys,
        mode,
    })
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
    let mut primary_key_differences = Vec::new();
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
                if column.primary_key != other.primary_key {
                    primary_key_differences.push(diff_pair(
                        &column.name,
                        json!(column.primary_key),
                        json!(other.primary_key),
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
        + primary_key_differences.len()
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
        "primaryKeyDifferences": primary_key_differences,
        "nullableDifferences": nullable_differences,
        "defaultDifferences": default_differences,
        "diffCount": diff_count,
    })
}

fn row_text(row: &Map<String, Value>, key: &str) -> Option<String> {
    row.get(key)
        .or_else(|| {
            row.iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value)
        })
        .and_then(|value| match value {
            Value::String(text) => Some(text.trim().to_string()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        })
}

fn definition_columns(definition: &str) -> Vec<String> {
    let Some(start) = definition.find('(') else {
        return Vec::new();
    };
    let Some(end) = definition.rfind(')') else {
        return Vec::new();
    };
    if end <= start {
        return Vec::new();
    }
    definition[start + 1..end]
        .split(',')
        .map(|column| {
            column
                .trim()
                .trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')
                .to_ascii_lowercase()
        })
        .filter(|column| !column.is_empty())
        .collect()
}

/// 索引只比较**语义签名**（是否唯一 + 列序），忽略各数据库自动生成的索引名。
/// 主键索引已经由列属性差异覆盖，这里不再重复。
async fn table_index_signatures(
    state: &AppState,
    conn: &str,
    scope: &str,
    schema: Option<&str>,
    table: &str,
    dialect: Dialect,
    primary_key: &BTreeSet<String>,
) -> XResult<Option<BTreeSet<String>>> {
    // Doris 的索引查询拿不到稳定的列序，比较结果会造成假差异；索引页签本身
    // 也只做展示。这里如实标记“未比较”，不猜列序。
    if dialect.kind.key() == "doris" {
        return Ok(None);
    }
    let Meta::Sql(sql) = dialect.indexes() else {
        return Ok(None);
    };
    let result = run_sql_in(state, conn, scope, sql, 100_000).await?;
    let table_name = table.rsplit('.').next().unwrap_or(table).to_ascii_lowercase();
    let schema_name = schema.map(str::to_ascii_lowercase);
    let mut signatures = BTreeSet::new();
    for row in crate::api::meta::rows_of(&result) {
        let row_table = row_text(&row, "table")
            .or_else(|| row_text(&row, "table_name"))
            .unwrap_or_default();
        if row_table.rsplit('.').next().unwrap_or(&row_table).to_ascii_lowercase()
            != table_name
        {
            continue;
        }
        if let Some(schema_name) = schema_name.as_deref() {
            if let Some(row_schema) = row_text(&row, "schema") {
                if !row_schema.eq_ignore_ascii_case(schema_name) {
                    continue;
                }
            }
        }
        let definition = row_text(&row, "indexdef")
            .or_else(|| row_text(&row, "sql"))
            .or_else(|| row_text(&row, "definition"))
            .unwrap_or_default();
        if let Some(schema_name) = schema_name.as_deref() {
            if !definition.is_empty()
                && !definition.to_ascii_lowercase().contains(schema_name)
            {
                continue;
            }
        }
        let mut columns = row_text(&row, "columns")
            .map(|text| {
                text.split(',')
                    .map(|column| {
                        column
                            .trim()
                            .trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')
                            .to_ascii_lowercase()
                    })
                    .filter(|column| !column.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| definition_columns(&definition));
        if columns.is_empty() {
            continue;
        }
        let unique = row_text(&row, "isUnique").is_some_and(|value| {
            value == "1" || value.eq_ignore_ascii_case("true")
        })
            || row_text(&row, "nonUnique").and_then(|value| value.parse::<i64>().ok())
                == Some(0)
            || definition.to_ascii_lowercase().contains("unique");
        let column_set: BTreeSet<String> = columns.drain(..).collect();
        if unique && column_set == *primary_key {
            continue;
        }
        signatures.insert(format!(
            "{}:{}",
            if unique { "unique" } else { "index" },
            column_set.into_iter().collect::<Vec<_>>().join(",")
        ));
    }
    Ok(Some(signatures))
}

fn compare_index_signatures(
    source: &BTreeSet<String>,
    target: &BTreeSet<String>,
) -> Value {
    json!({
        "onlyInSource": source.difference(target).cloned().collect::<Vec<_>>(),
        "onlyInTarget": target.difference(source).cloned().collect::<Vec<_>>(),
    })
}

fn normalize_index_identifier(identifier: &str) -> String {
    identifier
        .trim()
        .trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')
        .rsplit('.')
        .next()
        .unwrap_or(identifier)
        .trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']')
        .to_ascii_lowercase()
}

fn foreign_key_signatures(ddl: &str) -> BTreeSet<String> {
    static FK_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = FK_RE.get_or_init(|| {
        regex::Regex::new(
            r"(?is)foreign\s+key\s*\(([^)]*)\)\s*references\s+([^\s(]+)\s*\(([^)]*)\)",
        )
        .expect("外键正则必须可编译")
    });
    let columns = |text: &str| {
        text.split(',')
            .map(normalize_index_identifier)
            .filter(|column| !column.is_empty())
            .collect::<Vec<_>>()
            .join(",")
    };
    re.captures_iter(ddl)
        .filter_map(|capture| {
            let local = columns(capture.get(1)?.as_str());
            let referenced_table = normalize_index_identifier(capture.get(2)?.as_str());
            let referenced = columns(capture.get(3)?.as_str());
            (!local.is_empty() && !referenced_table.is_empty() && !referenced.is_empty()).then(|| {
                format!("{local}->{referenced_table}:{referenced}")
            })
        })
        .collect()
}

async fn table_foreign_key_signatures(
    state: &AppState,
    conn: &str,
    scope: &str,
    table: &str,
    schema: Option<&str>,
    dialect: Dialect,
) -> XResult<Option<BTreeSet<String>>> {
    let table_lit = dialect.literal(table);
    let schema_name = schema
        .map(str::to_string)
        .or_else(|| scope.split_once('.').map(|(_, schema)| schema.to_string()));
    let (sql, metadata_mode) = match dialect.kind.key() {
        "postgresql" | "kingbase" => {
            let schema_filter = schema_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .map(|name| format!(" and n.nspname = {}", dialect.literal(name)))
                .unwrap_or_default();
            (
                Some(format!(
                    "select pg_get_constraintdef(c.oid) as definition \
                     from pg_constraint c join pg_class r on r.oid = c.conrelid \
                     join pg_namespace n on n.oid = r.relnamespace \
                     where c.contype = 'f' and r.relname = {table_lit}{schema_filter}"
                )),
                true,
            )
        }
        "mysql" | "mariadb" => {
            let schema_lit = dialect.literal(
                schema_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or(scope.split('.').next().unwrap_or(scope)),
            );
            (
                Some(format!(
                    "select concat('foreign key (', \
                       group_concat(column_name order by ordinal_position separator ','), \
                       ') references ', referenced_table_name, ' (', \
                       group_concat(referenced_column_name order by ordinal_position separator ','), \
                       ')') as definition \
                     from information_schema.key_column_usage \
                     where table_schema = {schema_lit} and table_name = {table_lit} \
                       and referenced_table_name is not null \
                     group by constraint_name, referenced_table_name"
                )),
                true,
            )
        }
        "sqlserver" => (
            dialect
                .ddl_extras(table)
                .into_iter()
                .find(|(label, _)| label.contains("外键"))
                .map(|(_, sql)| sql),
            true,
        ),
        _ => match dialect.ddl(table, schema) {
            Meta::Sql(sql) => (Some(sql), false),
            _ => match dialect.object_source("table", table, schema) {
                Meta::Sql(sql) => (Some(sql), false),
                _ => (None, false),
            },
        },
    };
    let Some(sql) = sql else {
        return Ok(None);
    };
    let result = run_sql_in(state, conn, scope, sql, 100).await?;
    let ddl = result
        .rows
        .iter()
        .flat_map(|row| row.iter())
        .filter_map(|cell| match cell {
            CellValue::Text(text)
                if metadata_mode
                    || text.to_ascii_lowercase().contains("create table")
                    || text.to_ascii_lowercase().contains("alter table") =>
            {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if ddl.is_empty() && !metadata_mode {
        return Ok(None);
    }
    Ok(Some(foreign_key_signatures(&ddl)))
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
    dialect: Dialect,
) -> u64 {
    // 用户条件**必须整体加括号**：`where a = 1 or b = 2` 再跟 `and` 条件时，
    // and 的优先级高于 or，语义会被悄悄改掉（对比结果对不上，却查不出原因）。
    let where_part = condition
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(|c| format!(" where ({c})"))
        .unwrap_or_default();
    let schema_owned = schema.map(|s| s.to_string());
    let sql = format!("select count(*) from {}{where_part}", qualified(table, &schema_owned, dialect));
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
        // 异常展开时补发失败信号（见 SideGuard 的注释）。正常路径在末尾 disarm()。
        let guard = SideGuard::new(tx.clone());
        let list = columns
            .iter()
            .map(|name| dialect.quote(name))
            .collect::<Vec<_>>()
            .join(", ");
        let mut sql = format!("select {list} from {}", qualified(&table, &schema, dialect));
        // 条件整体加括号（见 count_rows 的注释），并且记住「有没有条件」——
        // 不要再靠 `sql.contains(" where ")` 猜：条件里出现字符串 ' where ' 就猜错了
        let condition = condition
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty());
        if let Some(condition) = condition {
            sql.push_str(&format!(" where ({condition})"));
        }
        let has_condition = condition.is_some();
        // 排序列（同时也是 keyset 游标的列），**保留成 Vec** 供下面展开成逐列条件
        let order_list: Vec<String> = if key_cols.is_empty() {
            columns.iter().map(|name| dialect.quote(name)).collect()
        } else {
            key_cols.iter().map(|name| dialect.quote(name)).collect()
        };
        let order_cols = order_list.join(", ");
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
                // 分页语法按**方言**：Oracle/DB2 不认 `limit`，SQL Server 要 `offset … fetch`，
                // 硬写 `limit` 的结果是「5 万行以内（首页）看着都对，一翻页就报错」，
                // 更糟的是首页直接语法错（Oracle 上整表对比根本跑不起来）。
                None => format!("{sql} order by {order_cols} {}", dialect.limit_clause(0, page_size as u64)),
                Some(key) => {
                    let lits: Vec<String> = key.iter().map(keyset_literal).collect();
                    // **必须展开成逐列 or 链，不能写 SQL 行值比较 `(a,b) > (x,y)`**：
                    // 行值比较只有 MySQL/PG/SQLite 这类认；SQL Server / Oracle / DB2 /
                    // 达梦 / Doris / ClickHouse 都会直接语法错 —— 那些库上超过 5 万行的
                    // 对比会「第二页就报错」，而 5 万行以内看不出问题（首页不带游标）。
                    // `(a,b) > (x,y)` 等价于 `a > x or (a = x and b > y)`，与同步侧
                    // `fetch_page` 的写法保持一致。
                    let mut alts: Vec<String> = Vec::with_capacity(lits.len());
                    for depth in 0..lits.len() {
                        let mut parts: Vec<String> = Vec::with_capacity(depth + 1);
                        for i in 0..=depth {
                            if i < depth {
                                parts.push(format!("{} = {}", order_list[i], lits[i]));
                            } else {
                                parts.push(format!("{} > {}", order_list[i], lits[i]));
                            }
                        }
                        alts.push(format!("({})", parts.join(" and ")));
                    }
                    let keyset = alts.join(" or ");
                    let glue = if has_condition { " and" } else { " where" };
                    format!(
                        "{sql}{glue} ({keyset}) order by {order_cols} {}",
                        dialect.limit_clause(0, page_size as u64)
                    )
                }
            };
            // 对比的对账拉数（一页页翻）是长任务：走独立会话键，不占元数据那条连接
            match run_sql_in_background(&state, &conn, &scope, page_sql, page_size).await {
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
        // 走到这里 = 正常收尾（含上面所有 break）—— 解除守卫，别把"正常读完"报成失败
        guard.disarm();
    });
}

/// 「辅助协程异常终止」必须**说出去**，不能被当成"读完了"。
///
/// 这个协程是数据源侧的产数据方。它 panic 时，通道会随 unwind 一起被丢掉，而主循环把
/// "通道关闭"当成"正常读完"（见 `SideStream::next_row` 的 `None` 分支）—— 于是**少读的那部分
/// 被当成读完了，任务报成功**。这不是"卡住"，比卡住更坏：结果看着是对的，其实是半份。
///
/// 守卫在 Drop 里补发一条 `Err`，主循环就会如实报错、把这次比对判失败。
/// 正常路径在每个退出点之后 `disarm()`，所以"正常结束"不会被误报成失败。
/// 用 `try_send` 而不是 `send`：Drop 里不能 await，而且此刻下游可能已经走了（发不出去就算了，
/// 它本来也不再看这些数据了）。
struct SideGuard(tokio::sync::mpsc::Sender<Result<SidePage, String>>, bool);

impl SideGuard {
    fn new(tx: tokio::sync::mpsc::Sender<Result<SidePage, String>>) -> Self {
        Self(tx, true)
    }

    fn disarm(mut self) {
        self.1 = false;
    }
}

impl Drop for SideGuard {
    fn drop(&mut self) {
        if self.1 {
            let _ = self.0.try_send(Err(
                "数据读取协程异常终止（驱动异常或内存不足），本次比对结果不完整".to_string(),
            ));
        }
    }
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

/// 文本/UUID 等键不依赖数据库排序，而是按内存中的归一键做哈希分区。
/// 每次只把一个桶装进内存，既避免 collation 错配，也避免整表 HashMap 的 OOM。
const COMPARE_HASH_BUCKETS: usize = 64;

struct CompareTempDir(PathBuf);

impl Drop for CompareTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn compare_bucket(key: &str) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish() as usize % COMPARE_HASH_BUCKETS
}

async fn open_compare_buckets(
    dir: &std::path::Path,
    prefix: &str,
) -> Result<Vec<BufWriter<tokio::fs::File>>, String> {
    let mut writers = Vec::with_capacity(COMPARE_HASH_BUCKETS);
    for bucket in 0..COMPARE_HASH_BUCKETS {
        let path = dir.join(format!("{prefix}-{bucket}.jsonl"));
        let file = tokio::fs::File::create(&path)
            .await
            .map_err(|e| format!("创建对比临时文件失败：{e}"))?;
        writers.push(BufWriter::new(file));
    }
    Ok(writers)
}

async fn spool_row(
    writer: &mut BufWriter<tokio::fs::File>,
    row: &[CellValue],
) -> Result<(), String> {
    let bytes = serde_json::to_vec(row).map_err(|e| format!("序列化对比行失败：{e}"))?;
    writer
        .write_all(&bytes)
        .await
        .map_err(|e| format!("写入对比临时文件失败：{e}"))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|e| format!("写入对比临时文件失败：{e}"))
}

#[allow(clippy::too_many_arguments)]
async fn compare_hashed(
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
    source_columns: &[String],
    target_columns: &[String],
    source_keys: &[String],
    target_keys: &[String],
    excluded: &[String],
    limit: usize,
    task: &std::sync::Arc<crate::api::tasks::Task>,
    est_total: u64,
) -> XResult<(Value, u64, u64)> {
    let (src_tx, src_rx) = tokio::sync::mpsc::channel::<Result<SidePage, String>>(2);
    spawn_side_stream(
        state.clone(),
        source_conn,
        source_scope,
        source_table,
        source_schema,
        source_condition,
        source_columns.to_vec(),
        source_dialect,
        source_keys.to_vec(),
        task.clone(),
        src_tx,
    );
    let (tgt_tx, tgt_rx) = tokio::sync::mpsc::channel::<Result<SidePage, String>>(2);
    spawn_side_stream(
        state.clone(),
        target_conn,
        target_scope,
        target_table,
        target_schema,
        target_condition,
        target_columns.to_vec(),
        target_dialect,
        target_keys.to_vec(),
        task.clone(),
        tgt_tx,
    );
    let mut source = SideStream::new(src_rx);
    let mut target = SideStream::new(tgt_rx);
    let mut s_row = source.next_row().await;
    let mut t_row = target.next_row().await;
    if let Some(err) = source.error.clone() {
        return Err(XError::bad_request(format!("读取源数据失败：{err}")));
    }
    if let Some(err) = target.error.clone() {
        return Err(XError::bad_request(format!("读取目标数据失败：{err}")));
    }
    let src_cols = source.columns.clone();
    let tgt_cols = target.columns.clone();
    let find = |cols: &[String], name: &str| {
        cols.iter().position(|column| column.eq_ignore_ascii_case(name))
    };
    let src_key_idx: Vec<usize> = source_keys
        .iter()
        .map(|key| {
            find(&src_cols, key)
                .ok_or_else(|| XError::bad_request(format!("源表结果里没有关键列 {key}")))
        })
        .collect::<XResult<_>>()?;
    let tgt_key_idx: Vec<usize> = target_keys
        .iter()
        .map(|key| {
            find(&tgt_cols, key)
                .ok_or_else(|| XError::bad_request(format!("目标表结果里没有关键列 {key}")))
        })
        .collect::<XResult<_>>()?;

    let root = std::env::temp_dir().join(format!("dbmind-compare-{}", task.id));
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| format!("创建对比临时目录失败：{e}"))?;
    let _guard = CompareTempDir(root.clone());
    let mut source_writers = open_compare_buckets(&root, "source").await.map_err(XError::bad_request)?;
    let mut target_writers = open_compare_buckets(&root, "target").await.map_err(XError::bad_request)?;

    let mut reported = 0u64;
    loop {
        task.check_canceled()?;
        if let Some(row) = s_row.take() {
            let key = shape::row_key(&row, &src_key_idx);
            spool_row(&mut source_writers[compare_bucket(&key)], &row)
                .await
                .map_err(XError::bad_request)?;
            s_row = source.next_row().await;
        }
        if let Some(row) = t_row.take() {
            let key = shape::row_key(&row, &tgt_key_idx);
            spool_row(&mut target_writers[compare_bucket(&key)], &row)
                .await
                .map_err(XError::bad_request)?;
            t_row = target.next_row().await;
        }
        let seen = source.rows_seen + target.rows_seen;
        if seen.saturating_sub(reported) >= 256 {
            task.add_done(seen - reported);
            reported = seen;
        }
        if s_row.is_none() && t_row.is_none() {
            break;
        }
    }
    if let Some(err) = source.error.clone() {
        return Err(XError::bad_request(format!("读取源数据失败：{err}")));
    }
    if let Some(err) = target.error.clone() {
        return Err(XError::bad_request(format!("读取目标数据失败：{err}")));
    }
    for writer in source_writers.iter_mut().chain(target_writers.iter_mut()) {
        writer
            .flush()
            .await
            .map_err(|e| format!("刷新对比临时文件失败：{e}"))?;
    }
    drop(source_writers);
    drop(target_writers);

    let mut compare_pairs: Vec<(usize, usize, String)> = Vec::new();
    let mut excluded_columns: Vec<String> = excluded.to_vec();
    let mut excluded_set: BTreeSet<String> =
        excluded.iter().map(|name| name.to_ascii_lowercase()).collect();
    for (index, name) in src_cols.iter().enumerate() {
        if src_key_idx.contains(&index) {
            continue;
        }
        match find(&tgt_cols, name) {
            Some(t_index) => compare_pairs.push((index, t_index, name.clone())),
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

    for bucket in 0..COMPARE_HASH_BUCKETS {
        task.check_canceled()?;
        task.step(format!("比对哈希分片 {}/{}", bucket + 1, COMPARE_HASH_BUCKETS));
        let target_path = root.join(format!("target-{bucket}.jsonl"));
        let target_file = tokio::fs::File::open(&target_path)
            .await
            .map_err(|e| format!("打开目标对比分片失败：{e}"))?;
        let mut target_lines = BufReader::new(target_file).lines();
        let mut target_map: HashMap<String, Vec<CellValue>> = HashMap::new();
        while let Some(line) = target_lines
            .next_line()
            .await
            .map_err(|e| format!("读取目标对比分片失败：{e}"))?
        {
            if line.trim().is_empty() {
                continue;
            }
            let row: Vec<CellValue> = serde_json::from_str(&line)
                .map_err(|e| format!("解析目标对比分片失败：{e}"))?;
            let key = shape::row_key(&row, &tgt_key_idx);
            if target_map.insert(key.clone(), row).is_some() {
                return Err(XError::bad_request(format!(
                    "目标表比对键不唯一（归一键 {key}）。请检查主键元数据或改用唯一键"
                )));
            }
        }

        let source_path = root.join(format!("source-{bucket}.jsonl"));
        let source_file = tokio::fs::File::open(&source_path)
            .await
            .map_err(|e| format!("打开源对比分片失败：{e}"))?;
        let mut source_lines = BufReader::new(source_file).lines();
        let mut source_seen: HashSet<String> = HashSet::new();
        while let Some(line) = source_lines
            .next_line()
            .await
            .map_err(|e| format!("读取源对比分片失败：{e}"))?
        {
            if line.trim().is_empty() {
                continue;
            }
            let row: Vec<CellValue> = serde_json::from_str(&line)
                .map_err(|e| format!("解析源对比分片失败：{e}"))?;
            let key = shape::row_key(&row, &src_key_idx);
            if !source_seen.insert(key.clone()) {
                return Err(XError::bad_request(format!(
                    "源表比对键不唯一（归一键 {key}）。请检查主键元数据或改用唯一键"
                )));
            }
            let Some(other) = target_map.remove(&key) else {
                only_in_source += 1;
                if only_source_samples.len() < limit {
                    only_source_samples.push(row_object(&src_cols, &row, None));
                }
                continue;
            };
            let mut changed = BTreeSet::new();
            for (s_index, t_index, name) in &compare_pairs {
                if !cell_equal(row.get(*s_index), other.get(*t_index)) {
                    changed.insert(*s_index);
                    different_columns.insert(name.clone());
                }
            }
            if changed.is_empty() {
                same += 1;
            } else {
                different += 1;
                if diff_samples.len() < limit {
                    let mut object = Map::new();
                    for index in &src_key_idx {
                        object.insert(
                            src_cols[*index].clone(),
                            row.get(*index)
                                .map(shape::cell_to_value)
                                .unwrap_or(Value::Null),
                        );
                    }
                    for (s_index, t_index, name) in &compare_pairs {
                        if !changed.contains(s_index) {
                            continue;
                        }
                        object.insert(
                            format!("{name}（源）"),
                            row.get(*s_index)
                                .map(shape::cell_to_value)
                                .unwrap_or(Value::Null),
                        );
                        object.insert(
                            format!("{name}（目标）"),
                            other
                                .get(*t_index)
                                .map(shape::cell_to_value)
                                .unwrap_or(Value::Null),
                        );
                    }
                    diff_samples.push(Value::Object(object));
                }
            }
        }
        for row in target_map.into_values() {
            only_in_target += 1;
            if only_target_samples.len() < limit {
                only_target_samples.push(row_object(&tgt_cols, &row, None));
            }
        }
    }
    if reported < source.rows_seen + target.rows_seen {
        task.add_done(source.rows_seen + target.rows_seen - reported);
    }
    let data = json!({
        "keyColumns": source_keys,
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
    let _ = est_total;
    Ok((data, source.rows_seen, target.rows_seen))
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
    source_columns: &[String],
    target_columns: &[String],
    source_keys: &[String],
    target_keys: &[String],
    excluded: &[String],
    limit: usize,
    task: &std::sync::Arc<crate::api::tasks::Task>,
    est_total: u64,
) -> XResult<(Value, u64, u64)> {
    if source_columns.is_empty() || target_columns.is_empty() {
        return Err(XError::bad_request("没有可对比的列"));
    }
    // 比对键必须落在对比列清单里（页里才取得到键值）
    for key in source_keys {
        if !source_columns.iter().any(|c| c.eq_ignore_ascii_case(key)) {
            return Err(XError::bad_request(format!("比对键 {key} 不在参与对比的列里")));
        }
    }
    for key in target_keys {
        if !target_columns.iter().any(|c| c.eq_ignore_ascii_case(key)) {
            return Err(XError::bad_request(format!("目标比对键 {key} 不在参与对比的列里")));
        }
    }

    let (src_tx, src_rx) = tokio::sync::mpsc::channel::<Result<SidePage, String>>(2);
    spawn_side_stream(
        state.clone(), source_conn, source_scope, source_table, source_schema,
        source_condition, source_columns.to_vec(), source_dialect, source_keys.to_vec(),
        task.clone(), src_tx,
    );
    let (tgt_tx, tgt_rx) = tokio::sync::mpsc::channel::<Result<SidePage, String>>(2);
    spawn_side_stream(
        state.clone(), target_conn, target_scope, target_table, target_schema,
        target_condition, target_columns.to_vec(), target_dialect, target_keys.to_vec(),
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
    // 列序初始化（首页列即全列序）
    let src_cols = source.columns.clone();
    let tgt_cols = target.columns.clone();
    let find = |cols: &[String], name: &str| {
        cols.iter().position(|c| c.eq_ignore_ascii_case(name))
    };
    let mut src_key_idx = Vec::new();
    for key in source_keys {
        let idx = find(&src_cols, key)
            .ok_or_else(|| XError::bad_request(format!("源表结果里没有关键列 {key}")))?;
        src_key_idx.push(idx);
    }
    let mut tgt_key_idx = Vec::new();
    for key in target_keys {
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
    let mut reported_seen: u64 = 0;
    loop {
        task.check_canceled()?;
        let seen = source.rows_seen + target.rows_seen;
        // 按实际消费行数推进；每 256 行更新一次，减少大表上的互斥锁开销。
        if seen.saturating_sub(reported_seen) >= 256 {
            task.add_done(seen - reported_seen);
            reported_seen = seen;
        }
        // **按百分比台阶记进度日志**（每跨过 10% 一条）：两侧行数不同时，
        // 「已比对 5 万行」这种绝对数会与单侧行数对不上（真机反馈），百分比才有意义
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
    if reported_seen < source.rows_seen + target.rows_seen {
        task.add_done(source.rows_seen + target.rows_seen - reported_seen);
    }

    if let Some(err) = source.error.clone() {
        return Err(XError::bad_request(format!("读取源数据失败：{err}")));
    }
    if let Some(err) = target.error.clone() {
        return Err(XError::bad_request(format!("读取目标数据失败：{err}")));
    }

    let data = json!({
        "keyColumns": source_keys,
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
    let source_record = require_record(&state, &source_conn).await?;
    crate::api::ensure_data_transfer(&source_record)?;
    let target_conn = body.target_connection();
    let target_record = require_record(&state, &target_conn).await?;
    crate::api::ensure_data_transfer(&target_record)?;

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
                let mut structure = compare_structure(&source_facts, &target_facts);
                let source_pk: BTreeSet<String> = source_facts
                    .iter()
                    .filter(|column| column.primary_key)
                    .map(|column| column.name.to_ascii_lowercase())
                    .collect();
                let target_pk: BTreeSet<String> = target_facts
                    .iter()
                    .filter(|column| column.primary_key)
                    .map(|column| column.name.to_ascii_lowercase())
                    .collect();
                let (source_indexes, target_indexes) = tokio::join!(
                    table_index_signatures(
                        &state,
                        &source_conn,
                        &source_scope,
                        body.source_schema.as_deref(),
                        &body.source_table,
                        source_dialect,
                        &source_pk,
                    ),
                    table_index_signatures(
                        &state,
                        &target_conn,
                        &target_scope,
                        body.target_schema.as_deref(),
                        &target_table,
                        target_dialect,
                        &target_pk,
                    )
                );
                if let (Ok(Some(source_indexes)), Ok(Some(target_indexes))) =
                    (&source_indexes, &target_indexes)
                {
                    let index_diff = compare_index_signatures(source_indexes, target_indexes);
                    let index_count = index_diff["onlyInSource"]
                        .as_array()
                        .map(Vec::len)
                        .unwrap_or(0)
                        + index_diff["onlyInTarget"]
                            .as_array()
                            .map(Vec::len)
                            .unwrap_or(0);
                    if let Some(obj) = structure.as_object_mut() {
                        obj.insert("indexesCompared".to_string(), json!(true));
                        obj.insert("indexDifferences".to_string(), index_diff);
                        let count = obj
                            .get("diffCount")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            + index_count as u64;
                        obj.insert("diffCount".to_string(), json!(count));
                    }
                } else if let Some(obj) = structure.as_object_mut() {
                    obj.insert("indexesCompared".to_string(), json!(false));
                }
                let (source_fks, target_fks) = tokio::join!(
                    table_foreign_key_signatures(
                        &state,
                        &source_conn,
                        &source_scope,
                        &body.source_table,
                        body.source_schema.as_deref(),
                        source_dialect,
                    ),
                    table_foreign_key_signatures(
                        &state,
                        &target_conn,
                        &target_scope,
                        &target_table,
                        body.target_schema.as_deref(),
                        target_dialect,
                    )
                );
                if let (Ok(Some(source_fks)), Ok(Some(target_fks))) =
                    (&source_fks, &target_fks)
                {
                    let fk_diff = compare_index_signatures(source_fks, target_fks);
                    let fk_count = fk_diff["onlyInSource"]
                        .as_array()
                        .map(Vec::len)
                        .unwrap_or(0)
                        + fk_diff["onlyInTarget"]
                            .as_array()
                            .map(Vec::len)
                            .unwrap_or(0);
                    if let Some(obj) = structure.as_object_mut() {
                        obj.insert("foreignKeysCompared".to_string(), json!(true));
                        obj.insert("foreignKeyDifferences".to_string(), fk_diff);
                        let count = obj
                            .get("diffCount")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            + fk_count as u64;
                        obj.insert("diffCount".to_string(), json!(count));
                    }
                } else if let Some(obj) = structure.as_object_mut() {
                    obj.insert("foreignKeysCompared".to_string(), json!(false));
                }
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
                let mut columns: Vec<String> = Vec::new();
                let mut target_columns: Vec<String> = Vec::new();
                let mut excluded: Vec<String> = Vec::new();
                for column in &source_facts {
                    let Some(target_column) = target_facts
                        .iter()
                        .find(|other| other.name.eq_ignore_ascii_case(&column.name))
                    else {
                        continue;
                    };
                    if column.binary || target_column.binary {
                        // 二进制列不参与：转成文本再比没有任何意义
                        excluded.push(column.name.clone());
                        continue;
                    }
                    columns.push(column.name.clone());
                    target_columns.push(target_column.name.clone());
                }
                let key_plan = validate_compare_keys(&source_facts, &target_facts, &keys)
                    .map_err(|e| e.message)?;
                task.step(format!("读取源表 {}", body.source_table));
                // **先 count 两侧行数做分母**：进度条按真实百分比增长（与数据传输一致）。
                // count 失败/为 0 → 分母未知，保持流动动画兜底。
                // 两侧 count 并发执行：它们是独立连接上的两条独立查询，串行只是白等
                // 一个 RTT × 2（跨网时每个 count 可能就是几百毫秒）。
                let (src_count, tgt_count) = tokio::join!(
                    count_rows(
                        &state, &source_conn, &source_scope, &body.source_table,
                        body.source_schema.as_deref(), body.source_condition.as_deref(),
                        source_dialect,
                    ),
                    count_rows(
                        &state, &target_conn, &target_scope, &target_table,
                        body.target_schema.as_deref(), body.target_condition.as_deref(),
                        target_dialect,
                    )
                );
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
                let compared = match key_plan.mode {
                    CompareKeyMode::Ordered => {
                        compare_streamed(
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
                            &target_columns,
                            &keys,
                            &key_plan.target_keys,
                            &excluded,
                            body.sample_limit(),
                            &task,
                            est_total,
                        )
                        .await
                    }
                    CompareKeyMode::Hashed => {
                        compare_hashed(
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
                            &target_columns,
                            &keys,
                            &key_plan.target_keys,
                            &excluded,
                            body.sample_limit(),
                            &task,
                            est_total,
                        )
                        .await
                    }
                };
                let (data, source_rows, target_rows) = compared.map_err(|e| e.message)?;
                // 校正分母/分子为**实际读取行数**（count 估算与真实可能有并发差）
                est_total = source_rows + target_rows;
                task.set_total(est_total as i64);
                task.set_done(source_rows + target_rows);
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

#[cfg(test)]
mod key_validation_tests {
    use super::*;

    fn column(name: &str, base_type: &str, nullable: bool, primary_key: bool) -> ColumnFacts {
        ColumnFacts {
            name: name.to_string(),
            base_type: base_type.to_string(),
            length: None,
            nullable,
            primary_key,
            default_value: String::new(),
            binary: false,
        }
    }

    #[test]
    fn 非空数值主键可用于严格对比并映射目标列名() {
        let source = vec![column("Id", "bigint", false, true)];
        let target = vec![column("ID", "numeric", false, true)];
        let plan = validate_compare_keys(&source, &target, &["Id".to_string()])
            .expect("非空数值主键应可用");
        assert_eq!(plan.target_keys, vec!["ID".to_string()]);
        assert_eq!(plan.mode, CompareKeyMode::Ordered);
    }

    #[test]
    fn 文本主键走哈希但可空和非主键仍拒绝() {
        let text = vec![column("code", "varchar", false, true)];
        let plan = validate_compare_keys(&text, &text, &["code".to_string()])
            .expect("完整非空文本主键应该可用");
        assert_eq!(plan.mode, CompareKeyMode::Hashed);

        let nullable = vec![column("seq", "bigint", true, true)];
        assert!(validate_compare_keys(&nullable, &nullable, &["seq".to_string()]).is_err());
        let non_key = vec![column("value", "bigint", false, false)];
        assert!(validate_compare_keys(&non_key, &non_key, &["value".to_string()]).is_err());
    }

    #[test]
    fn 复合主键必须完整选择() {
        let source = vec![
            column("tenant_id", "bigint", false, true),
            column("id", "bigint", false, true),
        ];
        let target = source.clone();
        let err = validate_compare_keys(&source, &target, &["id".to_string()])
            .expect_err("复合主键只选一部分必须拒绝");
        assert!(err.message.contains("完整等于"));
        let plan = validate_compare_keys(
            &source,
            &target,
            &["tenant_id".to_string(), "id".to_string()],
        )
        .expect("完整复合主键应可用");
        assert_eq!(plan.target_keys.len(), 2);
        assert_eq!(plan.mode, CompareKeyMode::Ordered);
    }

    #[test]
    fn 类型族用前缀判断避免_point_被当成_int() {
        assert_eq!(compare_sort_family("bigint"), Some("number"));
        assert_eq!(compare_sort_family("decimal"), Some("number"));
        assert_eq!(compare_sort_family("timestamp"), Some("time"));
        assert_eq!(compare_sort_family("point"), None);
        assert_eq!(compare_sort_family("varchar"), None);
    }

    #[test]
    fn 结构对比会报告主键属性变化与小数位变化() {
        let source = vec![ColumnFacts {
            name: "amount".to_string(),
            base_type: "decimal".to_string(),
            length: Some("10,2".to_string()),
            nullable: false,
            primary_key: true,
            default_value: String::new(),
            binary: false,
        }];
        let target = vec![ColumnFacts {
            name: "amount".to_string(),
            base_type: "decimal".to_string(),
            length: Some("10,4".to_string()),
            nullable: false,
            primary_key: false,
            default_value: String::new(),
            binary: false,
        }];
        let diff = compare_structure(&source, &target);
        assert_eq!(diff["primaryKeyDifferences"].as_array().map(Vec::len), Some(1));
        assert_eq!(diff["lengthDifferences"].as_array().map(Vec::len), Some(1));
        assert_eq!(diff["diffCount"].as_u64(), Some(2));
    }

    #[tokio::test]
    async fn 哈希对比临时分片可无损往返特殊文本() {
        let dir = std::env::temp_dir().join(format!(
            "dbmind-compare-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        tokio::fs::create_dir_all(&dir).await.expect("创建临时目录");
        let _guard = CompareTempDir(dir.clone());
        let row = vec![
            CellValue::Text("用户\\N\n第二行".to_string()),
            CellValue::Integer(7),
        ];
        let bucket = compare_bucket("text-key");
        let mut writers = open_compare_buckets(&dir, "source").await.expect("创建分片");
        spool_row(&mut writers[bucket], &row).await.expect("写入分片");
        writers[bucket].flush().await.expect("刷新分片");
        drop(writers);

        let file = tokio::fs::File::open(dir.join(format!("source-{bucket}.jsonl")))
            .await
            .expect("打开分片");
        let mut lines = BufReader::new(file).lines();
        let line = lines.next_line().await.expect("读取行").expect("非空行");
        let restored: Vec<CellValue> = serde_json::from_str(&line).expect("解析行");
        assert_eq!(restored, row);
    }

    #[test]
    fn 索引按唯一性和列序比较而不是名称() {
        let source: BTreeSet<String> = ["index:a,b".to_string()].into_iter().collect();
        let target: BTreeSet<String> = ["unique:a,b".to_string()].into_iter().collect();
        let diff = compare_index_signatures(&source, &target);
        assert_eq!(diff["onlyInSource"].as_array().map(Vec::len), Some(1));
        assert_eq!(diff["onlyInTarget"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn 外键签名忽略约束名和schema() {
        let ddl = r#"
            create table orders (
              id bigint,
              customer_id bigint,
              constraint fk_orders_customer foreign key ("customer_id")
                references "sales"."customers" ("id")
            );
        "#;
        let fks = foreign_key_signatures(ddl);
        assert!(fks.contains("customer_id->customers:id"));
    }
}
