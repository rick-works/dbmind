//! **动作层**：让 AI 的建议能落地，而且落错了能反悔。
//!
//! ## 为什么需要这一层
//!
//! 现在 AI 的产出大多是"一段 SQL + 一句建议"：用户得自己复制、自己找地方跑、
//! 跑坏了只能祈祷有备份。索引建议尤其典型 —— 模型（和规则）都能算出「这里该加个索引」，
//! 但"加了会不会更慢、加错了怎么办"没人回答，于是绝大多数建议都停在剪贴板里。
//!
//! 这一层把四步固化成协议，所有"可执行建议"（索引、加列、建表、修数据）都走同一条路：
//!
//! | 阶段 | 端点 | 做什么 |
//! |---|---|---|
//! | propose | 由各 AI 端点返回 `sql` | 提出动作 |
//! | preview | `POST /api/ai/action/plan` | 判风险、算撤销语句，**不执行** |
//! | apply | `POST /api/ai/action/apply` | 执行（必须 `confirm: true`），带上撤销语句 |
//! | undo | `POST /api/ai/action/undo` | 执行撤销语句 |
//!
//! ## 一条必须说清的边界
//!
//! **DDL 在 MySQL/Oracle 等数据库里是隐式提交的，事务回滚兜不住**。所以这里的"可撤销"
//! 是「反向语句可撤销」（`CREATE INDEX` ↔ `DROP INDEX`），不是数据库事务那种回滚。
//! 界面上必须把这句话写出来，否则用户会以为点错了能 Ctrl+Z。
//!
//! 反过来说，**没有反向语句的动作（`DROP TABLE`、`DROP COLUMN`、改类型）就是不可撤销的** ——
//! 这里直接返回 `undoSql: null` + `reversible: false`，界面据此把确认按钮做成红色的。

use std::time::Instant;

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::api::error::{XError, XResult};
use crate::api::meta::run_sql_in_write;
use crate::api::require_record;
use crate::AppState;

/// 取必填字符串字段。
fn need(body: &Value, key: &str) -> XResult<String> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| XError::bad_request(format!("缺少 {key} 参数")))
}

/// `POST /api/ai/action/plan` —— 预检：风险 + 撤销语句，**一行都不执行**。
pub async fn action_plan(State(state): State<AppState>, Json(body): Json<Value>) -> XResult<Json<Value>> {
    let sql = need(&body, "sql")?;
    let conn = need(&body, "connectionId")?;
    // 预检只做静态分析（不连库执行）：风险分级与撤销语句都靠 SQL 文本 + 方言，
    // 所以这里不需要 database —— 将来要接"大表行数估算"再加。
    let kind = require_record(&state, &conn).await?.kind().key().to_string();

    let undo = undo_for(&kind, &sql);
    let (risk, items) = risk_of(&sql);

    Ok(Json(json!({
        "success": true,
        "action": {
            "sql": sql,
            "undoSql": undo,
            // 注意：这里的"可撤销"= 有反向语句可执行，不是事务回滚（DDL 隐式提交）
            "reversible": undo.is_some(),
            "risk": risk,
            "items": items,
            "note": if undo.is_some() {
                "结构变更在多数数据库里是隐式提交，不能靠事务回滚；撤销靠执行反向语句（undoSql）。"
            } else {
                "这个动作没有可靠的反向语句（丢数据/改类型），执行后无法撤销 —— 请先备份。"
            },
        }
    })))
}

/// `POST /api/ai/action/apply` —— 执行动作。必须 `confirm: true`（防止误调用把库改了）。
pub async fn action_apply(State(state): State<AppState>, Json(body): Json<Value>) -> XResult<Json<Value>> {
    let sql = need(&body, "sql")?;
    let conn = need(&body, "connectionId")?;
    let database = body.get("database").and_then(Value::as_str).unwrap_or("");

    // 写操作一律要显式确认：这个端点能被"一键"点到，缺了这个闸门迟早出事
    let confirmed = body.get("confirm").and_then(Value::as_bool).unwrap_or(false);
    if !confirmed {
        return Err(XError::bad_request(
            "执行写操作需要显式确认（confirm: true）：请先调 /api/ai/action/plan 看风险与撤销语句",
        ));
    }

    let kind = require_record(&state, &conn).await?.kind().key().to_string();
    let undo = undo_for(&kind, &sql);

    // 可选：执行前后各取一次执行计划，用来回答"加了索引到底快没快"
    let explain = body
        .get("explainSql")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let before = match &explain {
        Some(query) => explain_text(&state, &conn, database, query).await,
        None => None,
    };

    let started = Instant::now();
    let result = run_sql_in_write(&state, &conn, database, sql.clone(), 0).await?;
    let elapsed = started.elapsed().as_millis() as u64;

    let after = match &explain {
        Some(query) => explain_text(&state, &conn, database, query).await,
        None => None,
    };

    Ok(Json(json!({
        "success": true,
        "executed": sql,
        "undoSql": undo,
        "reversible": undo.is_some(),
        "affectedRows": result.affected_rows.map(|rows| rows as i64).unwrap_or(-1),
        "elapsedMs": elapsed,
        "planBefore": before,
        "planAfter": after,
    })))
}

/// `POST /api/ai/action/undo` —— 执行反向语句，把上一次动作撤掉。
pub async fn action_undo(State(state): State<AppState>, Json(body): Json<Value>) -> XResult<Json<Value>> {
    let sql = need(&body, "sql")?;
    let conn = need(&body, "connectionId")?;
    let database = body.get("database").and_then(Value::as_str).unwrap_or("");

    let started = Instant::now();
    let result = run_sql_in_write(&state, &conn, database, sql.clone(), 0).await?;
    Ok(Json(json!({
        "success": true,
        "executed": sql,
        "affectedRows": result.affected_rows.map(|rows| rows as i64).unwrap_or(-1),
        "elapsedMs": started.elapsed().as_millis() as u64,
    })))
}

/// 取执行计划；取不到就返回 None（SQL Server / Oracle 需要专有语法，别把这事当失败）。
async fn explain_text(state: &AppState, conn: &str, database: &str, sql: &str) -> Option<String> {
    crate::api::ai::chat::explain_sql(state, conn, database, sql)
        .await
        .ok()
        .flatten()
        .map(|explain| explain.text)
}

/// 风险分级（规则式，不调模型）。返回 `(level, items)`。
pub(crate) fn risk_of(sql: &str) -> (&'static str, Vec<Value>) {
    let lower = sql.to_ascii_lowercase();
    let mut items: Vec<Value> = Vec::new();
    let mut level = "low";

    if lower.contains("drop table") || lower.contains("truncate") || lower.contains("drop column") {
        items.push(json!({
            "level": "high",
            "title": "会丢数据且不可撤销",
            "detail": "DROP / TRUNCATE / DROP COLUMN 删除的数据没有反向语句可恢复，请先备份。",
        }));
        level = "high";
    }
    if lower.contains("create index") && !lower.contains("concurrently") {
        items.push(json!({
            "level": "medium",
            "title": "建索引期间会占用写入",
            "detail": "大表建议 PostgreSQL 用 CONCURRENTLY、MySQL 8 用 ONLINE DDL，避开高峰。",
        }));
        level = if level == "high" { "high" } else { "medium" };
    }
    if lower.contains("add column") && lower.contains("default") {
        items.push(json!({
            "level": "medium",
            "title": "加带默认值的列可能重写全表",
            "detail": "老版本 MySQL/PG 会为已有行回填；大表建议先加可空列、分批回填、再加约束。",
        }));
        level = if level == "high" { "high" } else { "medium" };
    }
    if lower.starts_with("update") || lower.starts_with("delete") {
        if !has_where(&lower) {
            // 有没有 WHERE 是「改几行」与「改一整张表」的分界线。MySQL 的
            // sql_safe_updates 默认是关的，所以不带 WHERE 的语句会**真的**扫全表改掉。
            items.push(json!({
                "level": "high",
                "title": "没有 WHERE：会改动整张表",
                "detail": "UPDATE / DELETE 不带筛选条件等于全表改写，且没有反向语句可恢复；\
                           请加上 WHERE，或先在副本上试。执行前会被要求二次确认。",
            }));
            level = "high";
        } else {
            items.push(json!({
                "level": "medium",
                "title": "数据变更无法自动撤销",
                "detail": "行级回滚需要事前快照；这类语句请先在事务里试跑或先导出受影响的行。\
                           影响行数还会被设置里的「单次改动行数上限」再拦一次。",
            }));
            level = if level == "high" { "high" } else { "medium" };
        }
    }
    (level, items)
}

/// 语句里有没有 `where`。
///
/// 先剥掉引号里的字面量：`set note = 'where'` 不是筛选条件，把字面量一起搜会漏判 ——
/// 而漏判的代价是「无 WHERE 的全表更新」被当成安全语句放过去。
/// 反引号也一并剥（那里是标识符，`"where"` 这种列名不该算关键字）。
pub(crate) fn has_where(sql: &str) -> bool {
    let mut out = String::with_capacity(sql.len());
    let mut quote: Option<char> = None;
    for ch in sql.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '\'' || ch == '"' || ch == '`' => quote = Some(ch),
            None => out.push(ch),
        }
    }
    out.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|token| token.eq_ignore_ascii_case("where"))
}

/// 生成反向语句。**只覆盖能可靠反推的形态**，其余一律 `None`：
/// 宁可告诉用户"撤不了"，也不要给一句跑不通的 DROP 把库搞成半截。
pub fn undo_for(kind: &str, sql: &str) -> Option<String> {
    let flat = sql.trim().trim_end_matches(';').trim();
    let tokens: Vec<&str> = flat.split_whitespace().collect();
    let lower: Vec<String> = tokens.iter().map(|token| token.to_ascii_lowercase()).collect();
    if tokens.is_empty() {
        return None;
    }

    // CREATE [UNIQUE] INDEX name ON table (...)
    if lower[0] == "create" {
        let index_at = lower.iter().position(|token| token == "index");
        if let Some(index_at) = index_at {
            let name = unquote(tokens.get(index_at + 1)?);
            let on_at = lower.iter().position(|token| token == "on")?;
            let table = unquote(tokens.get(on_at + 1)?);
            if name.is_empty() || table.is_empty() {
                return None;
            }
            return Some(match kind {
                // MySQL 系：DROP INDEX 要带表
                "mysql" | "mariadb" | "doris" => format!("drop index {name} on {table}"),
                // ClickHouse 的索引是表内对象
                "clickhouse" => format!("alter table {table} drop index {name}"),
                _ => format!("drop index {name}"),
            });
        }
        // CREATE TABLE name (...) → DROP TABLE name
        if lower.get(1).map(String::as_str) == Some("table") {
            let mut at = 2;
            while at < lower.len() && matches!(lower[at].as_str(), "if" | "not" | "exists") {
                at += 1;
            }
            let table = unquote(tokens.get(at)?);
            if !table.is_empty() {
                return Some(format!("drop table {table}"));
            }
        }
        return None;
    }

    // ALTER TABLE t ADD [COLUMN] c ... → DROP COLUMN c
    if lower[0] == "alter" && lower.get(1).map(String::as_str) == Some("table") {
        let table = unquote(tokens.get(2)?);
        let add_at = lower.iter().position(|token| token == "add")?;
        let mut at = add_at + 1;
        if lower.get(at).map(String::as_str) == Some("column") {
            at += 1;
        }
        // `add constraint/primary key/foreign key/index` 不是加列，别硬套
        if matches!(
            lower.get(at).map(String::as_str),
            Some("constraint") | Some("primary") | Some("foreign") | Some("index") | Some("unique") | Some("key")
        ) {
            return None;
        }
        let column = unquote(tokens.get(at)?);
        if table.is_empty() || column.is_empty() {
            return None;
        }
        return Some(format!("alter table {table} drop column {column}"));
    }

    None
}

/// 去掉标识符上的引号/反引号/方括号。
fn unquote(token: &str) -> String {
    token
        .trim_matches(|ch| ch == '`' || ch == '"' || ch == '\'' || ch == '[' || ch == ']')
        .trim_end_matches(';')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_create_index_per_dialect() {
        let sql = "create index idx_mall on aibee_data_day (mall_id)";
        assert_eq!(
            undo_for("mysql", sql).as_deref(),
            Some("drop index idx_mall on aibee_data_day")
        );
        assert_eq!(
            undo_for("postgresql", sql).as_deref(),
            Some("drop index idx_mall")
        );
        assert_eq!(
            undo_for("clickhouse", sql).as_deref(),
            Some("alter table aibee_data_day drop index idx_mall")
        );
    }

    #[test]
    fn undo_alter_add_column() {
        assert_eq!(
            undo_for("mysql", "ALTER TABLE t ADD COLUMN c int default 1").as_deref(),
            Some("alter table t drop column c")
        );
        // ADD 后面不带 COLUMN 也要能认
        assert_eq!(
            undo_for("mysql", "alter table `t` add `c` int").as_deref(),
            Some("alter table t drop column c")
        );
    }

    #[test]
    fn undo_is_none_when_not_reversible() {
        assert_eq!(undo_for("mysql", "drop table t"), None);
        assert_eq!(undo_for("mysql", "alter table t drop column c"), None);
        assert_eq!(undo_for("mysql", "alter table t modify column c bigint"), None);
        // 加约束不是加列，不能生成 drop column
        assert_eq!(undo_for("mysql", "alter table t add constraint fk foreign key (a) references b(id)"), None);
    }

    #[test]
    fn undo_create_table() {
        assert_eq!(
            undo_for("mysql", "create table if not exists demo (id int)").as_deref(),
            Some("drop table demo")
        );
    }

    #[test]
    fn risk_marks_destructive_as_high() {
        assert_eq!(risk_of("drop table t").0, "high");
        assert_eq!(risk_of("create index i on t (c)").0, "medium");
        assert_eq!(risk_of("create table t (id int)").0, "low");
    }

    #[test]
    fn risk_marks_update_without_where_as_high() {
        assert_eq!(risk_of("update t set a = 1").0, "high");
        assert_eq!(risk_of("delete from t").0, "high");
        // 有 WHERE 只到 medium：它是「改数据」，但不是「整表改写」
        assert_eq!(risk_of("update t set a = 1 where id = 2").0, "medium");
        assert_eq!(risk_of("delete from t where id = 2").0, "medium");
    }

    #[test]
    fn where_inside_literal_is_not_a_filter() {
        // 字面量里的 where 不算筛选条件 —— 漏判就是「全表更新」被放过去
        assert!(!has_where("update t set note = 'where'"));
        assert!(!has_where("delete from t"));
        assert!(has_where("delete from t where id > 3"));
        assert!(has_where("UPDATE t SET a=1 WHERE\n id=1"));
    }
}
