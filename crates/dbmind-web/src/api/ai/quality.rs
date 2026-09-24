//! `/api/ai/quality/*` —— **质量规则引擎**（可配置业务规则 → 扫描 → 违规明细 → 报告）。
//!
//! 这一块**完全不调模型**：规则的语义是确定的（「用户名不能为空」就是 `is null`），
//! 判定必须可复现、可解释。模型只在「生成规则建议」那一步出场（见 `governance::quality_rules`）。
//!
//! ## 形状（前端读哪些字段）
//!
//! | 端点 | 请求 | 响应 |
//! |---|---|---|
//! | `types`（GET） | — | `{types:[{type,label,hint,params:[{key,label,kind,options?}]}]}` |
//! | `rules/saved` | `{connectionId,database,table}` | `{rules:[…]}` |
//! | `rules/save` | `{connectionId,database,table,rules}` | `{success,count}` |
//! | `rules/delete` | `{connectionId,database,table}` | `{success}` |
//! | `configured` | `{connectionId,database}` | `{tables:[{table,ruleCount}]}` |
//! | `scan` | `{connectionId,database,tables?}` | `{tableCount,totalRules,passed,failed,skipped,errors,score,tables:[…details]}` |
//! | `violations` | `{…,offset,limit,withTotal,format}` | `{columns,rows,total,rowCount,content,filename,exportSql}` |
//! | `report` | `{format,scan}` | `{success,content,filename}` |
//!
//! ## 三个决定
//!
//! 1. **状态用统一枚举**：`passed` / `failed` / `skipped` / `unsupported` / `error`。
//!    「这条规则这个库不支持」和「执行时炸了」是两回事，混在一起用户没法判断该找谁。
//! 2. **`violations` 一次返回三种视图**（分页表格 / 本页文本 / 全量 SQL）：
//!    三个按钮要的数据本来就是同一次查询的三种呈现，分三次查只会让「表在两次查询之间变了」
//!    这种诡异问题出现。
//! 3. **规则按「连接 + 库 + 表」存**，一个 JSON 文件。规则是配置不是数据，
//!    为它建表、迁移、备份不划算。
//!
//! ## 已知边界
//!
//! 正则类规则（`pattern`/`regex`/`email`/`phone`/`id_card`/`bank_card`）**依赖方言的正则能力**：
//! SQL Server 与 SQLite 没有内置正则，这两类库上会给出 `unsupported` 状态并说明原因 ——
//! 而不是悄悄放行（那会让「全部通过」变成假象）。


use axum::Json;
use dbmind_core::ColumnDetail;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::ai::tools::{columns_of, tables_of};
use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::meta::{rows_of, run_sql_in};
use crate::api::require_record;
use crate::api::shape;
use crate::AppState;

/// 一次扫描最多跑多少条规则（防止「500 张表 × 每表 20 条规则」把库拖死）。
const MAX_RULES_PER_SCAN: usize = 400;
/// 违规明细分页的默认与上限。
const VIOLATION_DEFAULT_LIMIT: usize = 50;
const VIOLATION_MAX_LIMIT: usize = 500;

// ------------------------------------------------------------------ 规则类型目录

/// 规则类型目录：`kind` 决定前端渲染什么控件（select / number / text / list）。
pub fn type_catalog() -> Value {
    let text = |key: &str, label: &str| json!({ "key": key, "label": label, "kind": "text" });
    let number = |key: &str, label: &str| json!({ "key": key, "label": label, "kind": "number" });
    json!({
        "types": [
            { "type": "not_null", "label": "非空", "hint": "该列不允许为 NULL", "params": [] },
            { "type": "not_empty", "label": "非空白", "hint": "该列不允许为 NULL 或空字符串", "params": [] },
            { "type": "unique", "label": "唯一", "hint": "该列取值不允许重复", "params": [] },
            { "type": "enum", "label": "枚举", "hint": "取值必须落在给定集合内",
              "params": [ { "key": "values", "label": "可选值", "kind": "list" } ] },
            { "type": "max_length", "label": "最大长度", "hint": "文本长度不得超过该值",
              "params": [ number("max", "最大长度") ] },
            { "type": "min_length", "label": "最小长度", "hint": "文本长度不得小于该值",
              "params": [ number("min", "最小长度") ] },
            { "type": "range", "label": "数值范围", "hint": "数值须落在给定区间内",
              "params": [ number("min", "最小值"), number("max", "最大值") ] },
            { "type": "pattern", "label": "格式", "hint": "须匹配给定正则表达式",
              "params": [ text("regex", "正则表达式") ] },
            { "type": "regex", "label": "正则", "hint": "须匹配给定正则表达式（与「格式」同义）",
              "params": [ text("regex", "正则表达式") ] },
            { "type": "null_ratio", "label": "空值率", "hint": "空值比例不得超过阈值（%）",
              "params": [ number("max", "最大空值率(%)") ] },
            { "type": "constant", "label": "常量列", "hint": "非空取值应全部相同（用于识别无意义列）", "params": [] },
            { "type": "empty_column", "label": "空列", "hint": "该列应全为空（用于识别废弃列）", "params": [] },
            { "type": "email", "label": "邮箱", "hint": "须为邮箱格式", "params": [] },
            { "type": "phone", "label": "手机号", "hint": "须为 11 位手机号", "params": [] },
            { "type": "id_card", "label": "身份证", "hint": "须为 18 位身份证号", "params": [] },
            { "type": "bank_card", "label": "银行卡", "hint": "须为 16~19 位数字", "params": [] }
        ]
    })
}

/// `GET /api/ai/quality/types`。
pub async fn types() -> XResult<Json<Value>> {
    Ok(Json(type_catalog()))
}

// ------------------------------------------------------------------ 存储

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QualityReq {
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub table: Option<String>,
    #[serde(default)]
    pub tables: Option<Vec<String>>,
    #[serde(default)]
    pub rules: Option<Vec<Value>>,
    #[serde(default)]
    pub offset: Option<u64>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub with_total: Option<bool>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub scan: Option<Value>,
}

impl QualityReq {
    fn conn(&self) -> XResult<String> {
        self.connection_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| XError::bad_request("缺少 connectionId 参数"))
    }

    fn db(&self) -> String {
        self.database.clone().unwrap_or_default()
    }

    fn table(&self) -> XResult<String> {
        self.table
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| XError::bad_request("缺少 table 参数"))
    }
}

fn store_key(conn: &str, database: &str, table: &str) -> String {
    format!("{conn}|{database}|{table}")
}

/// 读全部质量规则，形状保持原来的 json：
/// `{ "连接|库|表": { connectionId, database, table, rules, savedAt } }`。
///
/// 上层（`rules_delete` 的 `map.remove`、`configured` 的遍历、扫描/报告）一行都不用改。
fn load_store() -> Value {
    let Some(store) = dbmind_core::global_store() else {
        return json!({});
    };
    let mut doc = serde_json::Map::new();
    for row in store.ai_quality_rules().unwrap_or_default() {
        // 规则体坏了只让这一条空着，不连累其它表的规则
        let rules = serde_json::from_str::<Value>(&row.rules).unwrap_or_else(|_| json!([]));
        doc.insert(
            store_key(&row.connection_id, &row.database_name, &row.table_name),
            json!({
                "connectionId": row.connection_id,
                "database": row.database_name,
                "table": row.table_name,
                "rules": rules,
                "savedAt": row.saved_at,
            }),
        );
    }
    Value::Object(doc)
}

/// 整体写回（**删除也走这里** —— 上层删掉一个键之后照样调它）。
pub(crate) fn save_store(doc: &Value) -> XResult<()> {
    let Some(store) = dbmind_core::global_store() else {
        return Err(XError::internal("主库未就绪，质量规则无法保存"));
    };
    let mut rows: Vec<dbmind_core::AiQualityRuleRow> = Vec::new();
    if let Some(map) = doc.as_object() {
        for entry in map.values() {
            let text = |key: &str| {
                entry
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            let rules = entry.get("rules").cloned().unwrap_or_else(|| json!([]));
            rows.push(dbmind_core::AiQualityRuleRow {
                connection_id: text("connectionId"),
                database_name: text("database"),
                table_name: text("table"),
                rules: serde_json::to_string(&rules).unwrap_or_else(|_| "[]".to_string()),
                saved_at: text("savedAt"),
            });
        }
    }
    store
        .replace_ai_quality_rules(&rows)
        .map_err(|e| XError::internal(format!("写入主库失败：{e}")))
}

/// 读某张表的规则（不存在返回空数组）。
pub fn saved_rules(conn: &str, database: &str, table: &str) -> Vec<Value> {
    let doc = load_store();
    doc.get(store_key(conn, database, table))
        .and_then(|entry| entry.get("rules"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `POST /api/ai/quality/rules/saved`。
pub async fn rules_saved(Json(req): Json<QualityReq>) -> XResult<Json<Value>> {
    let conn = req.conn()?;
    let rules = saved_rules(&conn, &req.db(), &req.table()?);
    Ok(Json(json!({ "success": true, "rules": rules, "count": rules.len() })))
}

/// `POST /api/ai/quality/rules/save` —— 整表覆盖保存。
pub async fn rules_save(Json(req): Json<QualityReq>) -> XResult<Json<Value>> {
    let conn = req.conn()?;
    let database = req.db();
    let table = req.table()?;
    let mut rules = req.rules.clone().unwrap_or_default();
    if rules.is_empty() {
        return Err(XError::bad_request("没有可保存的规则"));
    }
    // 规范化：补齐 enabled / params，免得扫描时到处判空
    for rule in rules.iter_mut() {
        if rule.get("enabled").is_none() {
            rule["enabled"] = json!(true);
        }
        if rule.get("params").is_none() {
            rule["params"] = json!({});
        }
        if rule.get("level").is_none() {
            rule["level"] = json!("中");
        }
    }
    let mut doc = load_store();
    doc[store_key(&conn, &database, &table)] = json!({
        "connectionId": conn,
        "database": database,
        "table": table,
        "rules": rules,
        "savedAt": chrono::Local::now().to_rfc3339(),
    });
    save_store(&doc)?;
    // 两个键都给：`count` 是这里一直以来的返回（文档也按它写），`ruleCount` 是
    // `configured` 用的名字，而前端「已保存 N 条规则」那句提示读的是后者 ——
    // 只回 `count` 时界面上会拼出「已保存 undefined 条规则」（实测）。
    // 命名不统一是根源，这里先让两边都能用，避免再出现一次"字段名对不上"。
    Ok(Json(json!({
        "success": true,
        "count": rules.len(),
        "ruleCount": rules.len(),
    })))
}

/// `POST /api/ai/quality/rules/delete` —— 删除整张表的规则。
pub async fn rules_delete(Json(req): Json<QualityReq>) -> XResult<Json<Value>> {
    let conn = req.conn()?;
    let key = store_key(&conn, &req.db(), &req.table()?);
    let mut doc = load_store();
    let Some(map) = doc.as_object_mut() else {
        return Err(XError::internal("质量规则存储损坏"));
    };
    if map.remove(&key).is_none() {
        return Err(XError::bad_request("这张表没有已保存的规则"));
    }
    save_store(&doc)?;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/ai/quality/configured` —— 该连接/库下已配置规则的表。
pub async fn configured(Json(req): Json<QualityReq>) -> XResult<Json<Value>> {
    let conn = req.conn()?;
    let database = req.db();
    let doc = load_store();
    let mut tables: Vec<Value> = Vec::new();
    if let Some(map) = doc.as_object() {
        for entry in map.values() {
            if entry.get("connectionId").and_then(Value::as_str) != Some(conn.as_str()) {
                continue;
            }
            if entry.get("database").and_then(Value::as_str) != Some(database.as_str()) {
                continue;
            }
            let rule_count = entry
                .get("rules")
                .and_then(Value::as_array)
                .map(|rules| rules.len())
                .unwrap_or(0);
            tables.push(json!({
                "table": entry.get("table").and_then(Value::as_str).unwrap_or(""),
                "ruleCount": rule_count,
                "savedAt": entry.get("savedAt").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    tables.sort_by(|a, b| {
        a["table"]
            .as_str()
            .unwrap_or("")
            .cmp(b["table"].as_str().unwrap_or(""))
    });
    let rules_total: usize = tables
        .iter()
        .map(|item| item["ruleCount"].as_u64().unwrap_or(0) as usize)
        .sum();
    Ok(Json(json!({
        "success": true,
        "tables": tables,
        "count": tables.len(),
        "rules": rules_total,
    })))
}

// ------------------------------------------------------------------ 规则引擎

struct EvalOutcome {
    status: &'static str,
    violations: i64,
    message: String,
}

/// 方言的长度函数（`length` / `len`）。
fn length_fn(kind: &str) -> &'static str {
    if kind == "sqlserver" || kind == "derby" {
        "len"
    } else {
        "length"
    }
}

/// 正则条件的方言写法：`col <regex-op> pattern`。
///
/// `None` = 这个方言没有正则能力（SQL Server / SQLite / Derby），
/// 调用方必须回一个 `unsupported` 状态 —— 见文件头「已知边界」。
fn regex_condition(kind: &str, column: &str, pattern: &str, dialect: Dialect) -> Option<String> {
    let pattern = dialect.literal(pattern);
    Some(match kind {
        "mysql" | "mariadb" | "doris" => format!("{column} not regexp {pattern}"),
        "postgresql" | "kingbase" => format!("{column} !~ {pattern}"),
        "oracle" | "dm" => format!("not regexp_like({column}, {pattern})"),
        "h2" => format!("not regexp_like({column}, {pattern})"),
        "clickhouse" => format!("not match({column}, {pattern})"),
        _ => return None,
    })
}

fn param_str(params: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(value) = params.get(*key) {
            match value {
                Value::String(text) if !text.trim().is_empty() => return text.trim().to_string(),
                Value::Number(number) => return number.to_string(),
                Value::Array(items) => {
                    let joined = items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .collect::<Vec<_>>()
                        .join(",");
                    if !joined.trim().is_empty() {
                        return joined;
                    }
                }
                _ => {}
            }
        }
    }
    String::new()
}

fn param_num(params: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        match params.get(*key) {
            Some(Value::Number(number)) => return number.as_f64(),
            Some(Value::String(text)) => {
                if let Ok(value) = text.trim().parse::<f64>() {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

/// 一条规则在该表上产生的「违规行」条件与判定方式。
enum RuleKind {
    /// 逐行判定：`where <condition>` 的行就是违规行
    Rows(String),
    /// 表级指标：先算指标，超阈值则**把所有行**列为违规（附指标说明）
    Metric { probe: String, violated: Box<dyn Fn(f64) -> bool + Send>, label: String },
    /// 方言不支持
    Unsupported(String),
}

fn build_rule(
    rule: &Value,
    table: &str,
    table_quoted: &str,
    columns: &[ColumnDetail],
    dialect: Dialect,
) -> RuleKind {
    let column_name = rule
        .get("column")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if column_name.is_empty() {
        return RuleKind::Unsupported("规则没有指定字段".to_string());
    }
    if !columns
        .iter()
        .any(|column| column.name.eq_ignore_ascii_case(&column_name))
    {
        return RuleKind::Unsupported(format!("字段 {column_name} 在表 {table} 里不存在"));
    }
    let column = dialect.quote(&column_name);
    let params = rule.get("params").cloned().unwrap_or(json!({}));
    let rule_type = rule
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let kind = dialect.kind.key();

    // 格式类规则统一走这里：方言有正则能力就用正则，没有就如实报「不支持」
    let text_pattern = |pattern: String| -> RuleKind {
        match regex_condition(kind, &column, &pattern, dialect) {
            Some(condition) => RuleKind::Rows(format!("{column} is not null and {condition}")),
            None => RuleKind::Unsupported(format!(
                "{} 没有内置正则能力，无法判定格式规则（可在应用层校验，或改用长度/枚举规则）",
                kind.to_ascii_uppercase()
            )),
        }
    };

    match rule_type.as_str() {
        "not_null" => RuleKind::Rows(format!("{column} is null")),
        "not_empty" => RuleKind::Rows(format!("{column} is null or trim({column}) = ''")),
        "unique" => RuleKind::Rows(format!(
            "{column} is not null and {column} in (select {column} from {table_quoted} \
             where {column} is not null group by {column} having count(*) > 1)"
        )),
        "enum" => {
            let values = param_str(&params, &["values", "list", "enum", "options"]);
            if values.is_empty() {
                return RuleKind::Unsupported("枚举规则没有填写可选值".to_string());
            }
            let list = values
                .split([',', '，', '|'])
                .map(|item| dialect.literal(item.trim()))
                .filter(|item| item != "''")
                .collect::<Vec<_>>()
                .join(", ");
            if list.is_empty() {
                return RuleKind::Unsupported("枚举规则的取值列表是空的".to_string());
            }
            RuleKind::Rows(format!(
                "{column} is not null and {column} not in ({list})"
            ))
        }
        "max_length" | "min_length" => {
            let Some(bound) = param_num(&params, &["max", "min", "length", "value"]) else {
                return RuleKind::Unsupported("长度规则没有填写阈值".to_string());
            };
            let function = length_fn(kind);
            let operator = if rule_type == "max_length" { ">" } else { "<" };
            RuleKind::Rows(format!(
                "{column} is not null and {function}({column}) {operator} {}",
                bound as i64
            ))
        }
        "range" => {
            let min = param_num(&params, &["min", "minValue", "from"]);
            let max = param_num(&params, &["max", "maxValue", "to"]);
            if min.is_none() && max.is_none() {
                return RuleKind::Unsupported("数值范围规则没有填写上下限".to_string());
            }
            let mut parts = Vec::new();
            if let Some(min) = min {
                parts.push(format!("{column} < {min}"));
            }
            if let Some(max) = max {
                parts.push(format!("{column} > {max}"));
            }
            RuleKind::Rows(format!(
                "{column} is not null and ({})",
                parts.join(" or ")
            ))
        }
        "pattern" | "regex" => {
            let pattern = param_str(&params, &["regex", "pattern", "value"]);
            if pattern.is_empty() {
                return RuleKind::Unsupported("格式规则没有填写正则表达式".to_string());
            }
            text_pattern(pattern)
        }
        "email" => text_pattern(r"^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$".to_string()),
        "phone" => text_pattern(r"^1[3-9][0-9]{9}$".to_string()),
        "id_card" => text_pattern(r"^[0-9]{17}[0-9Xx]$".to_string()),
        "bank_card" => text_pattern(r"^[0-9]{16,19}$".to_string()),
        "null_ratio" => {
            let Some(threshold) = param_num(&params, &["max", "ratio", "value"]) else {
                return RuleKind::Unsupported("空值率规则没有填写阈值".to_string());
            };
            let function = length_fn(kind);
            let _ = function;
            RuleKind::Metric {
                probe: format!(
                    "select count(*) as total, count({column}) as present from {table_quoted}"
                ),
                violated: Box::new(move |ratio| ratio > threshold),
                label: format!("空值率超过 {threshold}%"),
            }
        }
        "constant" => RuleKind::Metric {
            probe: format!(
                "select count(*) as total, count(distinct {column}) as distinct_count \
                 from {table_quoted} where {column} is not null"
            ),
            violated: Box::new(|distinct| distinct <= 1.0),
            label: "该列取值恒定（可能没有业务意义）".to_string(),
        },
        "empty_column" => RuleKind::Metric {
            probe: format!("select count({column}) as present from {table_quoted}"),
            violated: Box::new(|present| present == 0.0),
            label: "该列全为空".to_string(),
        },
        other => RuleKind::Unsupported(format!("未知规则类型：{other}")),
    }
}

/// 规则的中文名（报告与违规明细里显示）。
fn rule_label(rule_type: &str) -> String {
    let catalog = type_catalog();
    catalog["types"]
        .as_array()
        .and_then(|types| {
            types
                .iter()
                .find(|item| item["type"] == json!(rule_type))
                .and_then(|item| item["label"].as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| rule_type.to_string())
}

/// 逐条规则求违规行数。
async fn eval_rules(
    state: &AppState,
    conn: &str,
    database: &str,
    table: &str,
    rules: &[Value],
    columns: &[ColumnDetail],
) -> XResult<(Vec<Value>, i64, i64, i64, i64, i64)> {
    let dialect = Dialect::new(require_record(state, conn).await?.kind());
    let table_quoted = dialect.quote(table);
    let mut details: Vec<Value> = Vec::new();
    let (mut passed, mut failed, mut skipped, mut errors) = (0i64, 0i64, 0i64, 0i64);

    for rule in rules.iter().take(MAX_RULES_PER_SCAN) {
        let column = rule
            .get("column")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let rule_type = rule
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let level = rule
            .get("level")
            .and_then(Value::as_str)
            .unwrap_or("中")
            .to_string();
        let rule_message = rule
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let enabled = rule.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        if !enabled {
            skipped += 1;
            details.push(json!({
                "column": column, "type": rule_type, "level": level,
                "status": "skipped", "violations": 0,
                "ruleMessage": rule_message, "message": "规则已停用",
            }));
            continue;
        }
        let outcome = match build_rule(rule, table, &table_quoted, columns, dialect) {
            RuleKind::Unsupported(reason) => EvalOutcome {
                status: "unsupported",
                violations: 0,
                message: reason,
            },
            RuleKind::Rows(condition) => {
                let sql = format!("select count(*) as cnt from {table_quoted} where {condition}");
                match run_sql_in(state, conn, database, sql, 1).await {
                    Ok(result) => {
                        // ⚠️ 取不到违规数时**绝不能当成 0**：0 的含义是"一条违规都没有"（规则通过），
                        // 而"取不到"必须报出去。以前这里写 `as_i64().unwrap_or(0)`，
                        // 而 ClickHouse 的 count 回来是浮点 → 恒取不到 → **规则永远判通过**（假通过）。
                        let count = rows_of(&result)
                            .first()
                            .and_then(|row| row.get("cnt"))
                            .and_then(shape::value_to_i64);
                        match count {
                            Some(count) => EvalOutcome {
                                status: if count == 0 { "passed" } else { "failed" },
                                violations: count,
                                message: String::new(),
                            },
                            None => EvalOutcome {
                                status: "error",
                                violations: 0,
                                message: "违规数取不到（该引擎返回的数值形态解析不了）".to_string(),
                            },
                        }
                    }
                    Err(err) => EvalOutcome {
                        status: "error",
                        violations: 0,
                        message: err.message,
                    },
                }
            }
            RuleKind::Metric {
                probe,
                violated,
                label,
            } => match run_sql_in(state, conn, database, probe, 1).await {
                Ok(result) => {
                    let row = rows_of(&result).into_iter().next().unwrap_or_default();
                    let total = row.get("total").and_then(Value::as_f64).unwrap_or(0.0);
                    let present = row.get("present").and_then(Value::as_f64).unwrap_or(0.0);
                    let distinct = row
                        .get("distinct_count")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    let value = if rule_type == "null_ratio" {
                        if total > 0.0 {
                            (total - present) * 100.0 / total
                        } else {
                            0.0
                        }
                    } else if rule_type == "constant" {
                        distinct
                    } else {
                        present
                    };
                    // 指标型规则违规时，违规行数按整表算 —— 它说的就是「这张表这列有问题」
                    let bad = violated(value);
                    EvalOutcome {
                        status: if bad { "failed" } else { "passed" },
                        violations: if bad { total as i64 } else { 0 },
                        message: if bad {
                            format!(
                                "{label}（实测值 {:.2}）",
                                value
                            )
                        } else {
                            String::new()
                        },
                    }
                }
                Err(err) => EvalOutcome {
                    status: "error",
                    violations: 0,
                    message: err.message,
                },
            },
        };
        match outcome.status {
            "passed" => passed += 1,
            "failed" => failed += 1,
            "skipped" => skipped += 1,
            "unsupported" => skipped += 1,
            _ => errors += 1,
        }
        details.push(json!({
            "column": column,
            "type": rule_type,
            "level": level,
            "status": outcome.status,
            "violations": outcome.violations,
            "ruleMessage": rule_message,
            "message": outcome.message,
        }));
    }
    Ok((details, passed, failed, skipped, errors, 0))
}

fn score_of(passed: i64, failed: i64, errors: i64) -> i64 {
    let exec = passed + failed + errors;
    if exec == 0 {
        100
    } else {
        ((passed as f64) * 100.0 / exec as f64).round() as i64
    }
}

/// `POST /api/ai/quality/scan`。
pub async fn scan(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<QualityReq>,
) -> XResult<Json<Value>> {
    let conn = req.conn()?;
    let database = req.db();
    let all_tables = tables_of(&state, &conn, &database).await?;

    // 要扫的表：请求指定 > 已配置规则的表
    let targets: Vec<String> = match &req.tables {
        Some(list) if !list.is_empty() => list.clone(),
        _ => {
            let doc = load_store();
            let mut names = Vec::new();
            if let Some(map) = doc.as_object() {
                for entry in map.values() {
                    if entry.get("connectionId").and_then(Value::as_str) != Some(conn.as_str())
                        || entry.get("database").and_then(Value::as_str) != Some(database.as_str())
                    {
                        continue;
                    }
                    if let Some(table) = entry.get("table").and_then(Value::as_str) {
                        names.push(table.to_string());
                    }
                }
            }
            names
        }
    };
    if targets.is_empty() {
        return Err(XError::bad_request(
            "这个库下还没有配置任何质量规则：先在「规则配置」里为表添加规则",
        ));
    }

    let mut tables_result: Vec<Value> = Vec::new();
    let (mut total_rules, mut total_passed, mut total_failed, mut total_skipped, mut total_errors) =
        (0i64, 0i64, 0i64, 0i64, 0i64);
    for table in &targets {
        if !all_tables.iter().any(|item| &item.name == table) {
            tables_result.push(json!({
                "table": table, "rules": 0, "passed": 0, "failed": 0, "skipped": 0,
                "errors": 0, "score": 100, "details": [],
                "message": "表不存在（已跳过）",
            }));
            continue;
        }
        let rules = saved_rules(&conn, &database, table);
        if rules.is_empty() {
            tables_result.push(json!({
                "table": table, "rules": 0, "passed": 0, "failed": 0, "skipped": 0,
                "errors": 0, "score": 100, "details": [],
                "message": "未配置规则",
            }));
            continue;
        }
        let columns = columns_of(&state, &conn, &database, table).await?;
        let (details, passed, failed, skipped, errors, _) =
            eval_rules(&state, &conn, &database, table, &rules, &columns).await?;
        total_rules += rules.len() as i64;
        total_passed += passed;
        total_failed += failed;
        total_skipped += skipped;
        total_errors += errors;
        tables_result.push(json!({
            "table": table,
            "rules": rules.len(),
            "passed": passed,
            "failed": failed,
            "skipped": skipped,
            "errors": errors,
            "score": score_of(passed, failed, errors),
            "details": details,
        }));
    }

    Ok(Json(json!({
        "success": true,
        "tableCount": tables_result.len(),
        "totalRules": total_rules,
        "passed": total_passed,
        "failed": total_failed,
        "skipped": total_skipped,
        "errors": total_errors,
        "score": score_of(total_passed, total_failed, total_errors),
        "scanTime": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "tables": tables_result,
    })))
}

/// `POST /api/ai/governance/quality/check` —— 单表试跑（规则可自带，不读存储）。
pub async fn check_one(
    axum::extract::State(state): axum::extract::State<AppState>,
    body: Json<Value>,
) -> XResult<Json<Value>> {
    let value = body.0;
    let conn = value
        .get("connectionId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if conn.is_empty() {
        return Err(XError::bad_request("缺少 connectionId 参数"));
    }
    let database = value
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let table = value
        .get("table")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if table.is_empty() {
        return Err(XError::bad_request("缺少 table 参数"));
    }
    let rules = value
        .get("rules")
        .and_then(Value::as_array)
        .cloned()
        .filter(|list| !list.is_empty())
        .unwrap_or_else(|| saved_rules(&conn, &database, &table));
    if rules.is_empty() {
        return Err(XError::bad_request("这张表没有可检查的规则"));
    }
    let columns = columns_of(&state, &conn, &database, &table).await?;
    let (details, passed, failed, skipped, errors, _) =
        eval_rules(&state, &conn, &database, &table, &rules, &columns).await?;
    Ok(Json(json!({
        "success": true,
        "table": table,
        "rules": rules.len(),
        "passed": passed,
        "failed": failed,
        "skipped": skipped,
        "errors": errors,
        "score": score_of(passed, failed, errors),
        "details": details,
    })))
}

// ------------------------------------------------------------------ 违规明细

/// 把逐行判定类规则合成一条 `union all` 查询。
fn union_sql(
    state_dialect: Dialect,
    table: &str,
    columns: &[ColumnDetail],
    rules: &[Value],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let dialect = state_dialect;
    let table_quoted = dialect.quote(table);
    let mut names: Vec<String> = Vec::new();
    let mut selects: Vec<String> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for rule in rules {
        let column_name = rule
            .get("column")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let rule_type = rule
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let enabled = rule.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        if !enabled || column_name.is_empty() {
            continue;
        }
        let label = format!("{}·{}", column_name, rule_label(&rule_type));
        let RuleKind::Rows(condition) =
            build_rule(rule, table, &table_quoted, columns, dialect)
        else {
            continue;
        };
        let projection: Vec<String> = columns
            .iter()
            .map(|column| dialect.quote(&column.name))
            .collect();
        let literal = dialect.literal(&label);
        selects.push(format!(
            "select {literal} as rule_name, {} from {table_quoted} where {condition}",
            projection.join(", ")
        ));
        if labels.is_empty() {
            names.push("规则".to_string());
            for column in columns {
                names.push(column.name.clone());
            }
        }
        labels.push(label);
    }
    (names, selects, labels)
}

/// 从二维数据生成 CSV / SpreadsheetML 文本（报告与违规导出共用）。
fn to_text(columns: &[String], rows: &[Vec<Value>], format: &str) -> String {
    let cell = |value: &Value| -> String {
        match value {
            Value::Null => String::new(),
            Value::String(text) => text.clone(),
            other => other.to_string(),
        }
    };
    if format == "excel" || format == "xls" {
        // SpreadsheetML：Excel 能直接打开，且**是纯文本**（前端走 downloadText，
        // 返回二进制流反而要改前端）
        let mut xml = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <?mso-application progid=\"Excel.Sheet\"?>\n\
             <Workbook xmlns=\"urn:schemas-microsoft-com:office:spreadsheet\" \
             xmlns:ss=\"urn:schemas-microsoft-com:office:spreadsheet\">\
             <Worksheet ss:Name=\"Sheet1\"><Table>",
        );
        xml.push_str("<Row>");
        for name in columns {
            xml.push_str(&format!(
                "<Cell><Data ss:Type=\"String\">{}</Data></Cell>",
                escape_xml(name)
            ));
        }
        xml.push_str("</Row>");
        for row in rows {
            xml.push_str("<Row>");
            for value in row {
                let is_number = matches!(value, Value::Number(_));
                xml.push_str(&format!(
                    "<Cell><Data ss:Type=\"{}\">{}</Data></Cell>",
                    if is_number { "Number" } else { "String" },
                    escape_xml(&cell(value))
                ));
            }
            xml.push_str("</Row>");
        }
        xml.push_str("</Table></Worksheet></Workbook>");
        return xml;
    }
    let mut csv = String::from("\u{FEFF}");
    csv.push_str(
        &columns
            .iter()
            .map(|name| escape_csv(name))
            .collect::<Vec<_>>()
            .join(","),
    );
    csv.push('\n');
    for row in rows {
        csv.push_str(
            &row.iter()
                .map(|value| escape_csv(&cell(value)))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push('\n');
    }
    csv
}

fn escape_csv(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .chars()
        .map(|c| if (c as u32) < 0x20 && c != '\t' && c != '\n' { ' ' } else { c })
        .collect()
}



/// `POST /api/ai/quality/violations` —— 违规行明细（表格 / 本页文本 / 全量 SQL 三合一）。
pub async fn violations(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<QualityReq>,
) -> XResult<Json<Value>> {
    let conn = req.conn()?;
    let database = req.db();
    let table = req.table()?;
    let dialect = Dialect::new(require_record(&state, &conn).await?.kind());
    let columns = columns_of(&state, &conn, &database, &table).await?;
    if columns.is_empty() {
        return Err(XError::bad_request(format!("表 {table} 不存在或没有列信息")));
    }
    let rules = saved_rules(&conn, &database, &table);
    if rules.is_empty() {
        return Err(XError::bad_request("这张表没有已保存的规则，无法生成违规明细"));
    }
    let (names, selects, labels) = union_sql(dialect, &table, &columns, &rules);
    if selects.is_empty() {
        return Ok(Json(json!({
            "success": true,
            "columns": [],
            "rows": [],
            "total": 0,
            "rowCount": 0,
            "message": "该表的规则都不是逐行判定类型（如空值率/常量列），没有可列出的违规行",
        })));
    }
    let union = selects.join(" union all ");
    let limit = req
        .limit
        .unwrap_or(VIOLATION_DEFAULT_LIMIT as u64)
        .clamp(1, VIOLATION_MAX_LIMIT as u64);
    let offset = req.offset.unwrap_or(0);

    let paged = format!(
        "select * from ({union}) dbmind_violations {}",
        dialect.limit_clause(offset, limit)
    );
    let result = run_sql_in(&state, &conn, &database, paged, limit as usize).await?;
    // 按**结果列的位置**取值，而不是按名字查：
    // 首列在 SQL 里叫 `rule_name`、在界面上显示为「规则」，按名字查会全部取成 null
    let rows: Vec<Vec<Value>> = result
        .rows
        .iter()
        .map(|row| row.iter().map(shape::cell_to_value).collect())
        .collect();

    // total 只在需要时统计：规则多的时候每条一次 COUNT(*) 正是翻页慢的主因
    let need_total = req.with_total.unwrap_or(true);
    let total = if need_total {
        let count_sql = format!("select count(*) as cnt from ({union}) dbmind_violations");
        match run_sql_in(&state, &conn, &database, count_sql, 1).await {
            // 与 Err 分支同一口径：取不到就是「未知」(-1)，不要写 0 ——
            // 0 表示"一条违规都没有"，而这里恰恰是不知道有几条。
            Ok(result) => rows_of(&result)
                .first()
                .and_then(|row| row.get("cnt"))
                .and_then(shape::value_to_i64)
                .unwrap_or(-1),
            Err(_) => -1,
        }
    } else {
        -1
    };

    let format = req.format.clone().unwrap_or_else(|| "excel".to_string());
    let content = to_text(&names, &rows, &format);
    Ok(Json(json!({
        "success": true,
        "table": table,
        "ruleLabels": labels,
        "columns": names,
        "rows": rows,
        "rowCount": rows.len(),
        "total": total,
        "content": content,
        "filename": format!(
            "quality-violations-{}-{}.{}",
            table,
            chrono::Local::now().format("%Y%m%d%H%M%S"),
            if format == "csv" { "csv" } else { "xls" }
        ),
        "exportSql": union,
    })))
}

// ------------------------------------------------------------------ 报告

/// `POST /api/ai/quality/report` —— 由扫描结果生成报告文本（csv / excel）。
pub async fn report(Json(req): Json<QualityReq>) -> XResult<Json<Value>> {
    let format = req
        .format
        .clone()
        .unwrap_or_else(|| "csv".to_string())
        .to_ascii_lowercase();
    let scan = req
        .scan
        .clone()
        .ok_or_else(|| XError::bad_request("缺少 scan 参数（报告由扫描结果生成）"))?;
    let columns: Vec<String> = ["表", "字段", "规则", "等级", "状态", "违规行数", "说明"]
        .iter()
        .map(|name| name.to_string())
        .collect();
    // 返回 String 而不是 &str：有几个分支是字面量（'static）、另一些是入参的生命周期，
    // 闭包推不出统一的返回生命周期
    let status_text = |status: &str| -> String {
        match status {
            "passed" => "通过",
            "failed" => "失败",
            "skipped" => "已跳过",
            "unsupported" => "不支持",
            "error" => "异常",
            other => other,
        }
        .to_string()
    };
    let mut rows: Vec<Vec<Value>> = Vec::new();
    for table in scan
        .get("tables")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let table_name = table
            .get("table")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let details = table
            .get("details")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if details.is_empty() {
            rows.push(vec![
                json!(table_name),
                json!("—"),
                json!("—"),
                json!("—"),
                json!(table.get("message").and_then(Value::as_str).unwrap_or("未配置规则")),
                json!(0),
                json!(""),
            ]);
            continue;
        }
        for detail in details {
            let rule_type = detail.get("type").and_then(Value::as_str).unwrap_or("");
            rows.push(vec![
                json!(table_name),
                detail.get("column").cloned().unwrap_or(json!("—")),
                json!(rule_label(rule_type)),
                detail.get("level").cloned().unwrap_or(json!("—")),
                json!(status_text(
                    detail.get("status").and_then(Value::as_str).unwrap_or("")
                )),
                detail.get("violations").cloned().unwrap_or(json!(0)),
                json!(
                    detail
                        .get("ruleMessage")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                        .or_else(|| detail.get("message").and_then(Value::as_str))
                        .unwrap_or("")
                ),
            ]);
        }
    }
    let content = to_text(&columns, &rows, &format);
    Ok(Json(json!({
        "success": true,
        "content": content,
        "filename": format!(
            "data-quality-report-{}.{}",
            chrono::Local::now().format("%Y%m%d%H%M%S"),
            if format == "csv" { "csv" } else { "xls" }
        ),
        "rowCount": rows.len(),
    })))
}

/// 路由。
pub fn routes() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/ai/quality/types", get(types))
        .route("/api/ai/quality/rules/saved", post(rules_saved))
        .route("/api/ai/quality/rules/save", post(rules_save))
        .route("/api/ai/quality/rules/delete", post(rules_delete))
        .route("/api/ai/quality/configured", post(configured))
        .route("/api/ai/quality/scan", post(scan))
        .route("/api/ai/quality/violations", post(violations))
        .route("/api/ai/quality/report", post(report))
}
