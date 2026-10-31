//! AI 结论的**证据链**（Verifiable AI）。
//!
//! 数据库产品做 AI，有一个别的场景拿不到的优势：**结论可以当场验**。
//! 所以只要 AI 给的是"这条能跑 / 修好了 / 这样更快"，我们都尽量真的执行一次，
//! 把「跑了什么、回来多少行、有没有报错」原样带回去 —— 前端点开就能看见，
//! 用户不必凭感觉相信模型。这也是 DBMind 与"套壳聊天"最本质的区别。
//!
//! ## 三条规矩
//!
//! 1. **只读语句真的执行**：走独立会话（不占元数据连接）、行数封顶，拿到真实行列。
//! 2. **写语句绝不执行**：只取执行计划（`EXPLAIN` / `SHOWPLAN` / `PLAN_TABLE`），
//!    证明"语法、对象、权限都对"，但一行数据都不动。
//! 3. **验证失败不是接口错误**：把报错原文放进 `message`，让用户看到
//!    「AI 说修好了，其实还是这个错」—— 这比一句 500 有用得多。
//!
//! 调用方拿到的 `evidence` 是可选字段：没连库、没给 connectionId 时为 `None`，
//! 前端按"未验证"展示，而不是凭空造一个通过。

use serde_json::{json, Value};

use crate::api::meta::run_sql_in_background;
use crate::api::shape;
use crate::AppState;

/// 验证时最多取回多少行。够看形状、又不会把表拉进浏览器。
pub const VERIFY_ROWS: usize = 20;

/// 一条 AI 结论的验证结果。
#[derive(Debug, serde::Serialize)]
pub struct Evidence {
    /// `read` 真跑了一次 / `plan` 只看执行计划 / `skipped` 没法验
    pub kind: &'static str,
    /// 实际执行（或取计划）的语句，前端可原样再跑一遍
    pub sql: String,
    pub ok: bool,
    /// 人话：通过时说明跑了什么；失败时是数据库报错原文
    pub message: String,
    pub elapsed_ms: u64,
    pub columns: Vec<String>,
    pub rows: Vec<Value>,
}

impl Evidence {
    fn skipped(sql: String, message: impl Into<String>) -> Self {
        Self {
            kind: "skipped",
            sql,
            ok: false,
            message: message.into(),
            elapsed_ms: 0,
            columns: Vec::new(),
            rows: Vec::new(),
        }
    }
}

/// 验证一条 SQL（只读真跑 / 写语句看计划），永不返回 Err。
///
/// `conn` 是连接 id，`database` 是库（或 `库.模式`）。失败一律折进 `Evidence`：
/// 这张表的用途就是"把失败也如实展示出来"。
pub async fn verify(state: &AppState, conn: &str, database: &str, sql: &str) -> Evidence {
    let sql = sql.trim().trim_end_matches(';').trim().to_string();
    if sql.is_empty() {
        return Evidence::skipped(sql, "AI 没有给出可验证的语句");
    }
    if conn.trim().is_empty() {
        return Evidence::skipped(sql, "未指定连接，未做验证");
    }

    let started = std::time::Instant::now();

    // 分支一：只读 —— 真跑，拿真实行列
    if crate::api::ai::read_only(&sql) {
        return match run_sql_in_background(state, conn, database, sql.clone(), VERIFY_ROWS).await {
            Ok(result) => {
                let value = shape::query_result_json(&result);
                let columns = value
                    .get("columns")
                    .and_then(|c| c.as_array())
                    .map(|list| {
                        list.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let rows = value
                    .get("rows")
                    .and_then(|r| r.as_array())
                    .cloned()
                    .unwrap_or_default();
                let count = rows.len();
                Evidence {
                    kind: "read",
                    sql,
                    ok: true,
                    message: format!("已试跑：返回 {count} 行（最多取 {VERIFY_ROWS} 行）"),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    columns,
                    rows,
                }
            }
            Err(err) => Evidence {
                kind: "read",
                sql,
                ok: false,
                message: err.message.clone(),
                elapsed_ms: started.elapsed().as_millis() as u64,
                columns: Vec::new(),
                rows: Vec::new(),
            },
        };
    }

    // 分支二：含写操作 —— 一行都不执行，只取执行计划
    match crate::api::ai::chat::explain_sql(state, conn, database, &sql).await {
        Ok(Some(plan)) => {
            let rows: Vec<Value> = plan
                .text
                .lines()
                .take(40)
                .map(|line| json!({ "plan": line }))
                .collect();
            let count = rows.len();
            Evidence {
                kind: "plan",
                sql,
                ok: true,
                message: format!("含写操作，未执行；执行计划校验通过（{count} 行）"),
                elapsed_ms: started.elapsed().as_millis() as u64,
                columns: vec!["plan".to_string()],
                rows,
            }
        }
        // 取不到计划 ≠ 语句有问题：这类库（SQL Server / Oracle / DB2 需要专有语法）我们
        // 本来就不做一条 EXPLAIN，明确说"没验"比假装通过好。
        Ok(None) => Evidence::skipped(sql, "含写操作，未执行；该数据库类型不支持一条 EXPLAIN 校验，本次未验证"),
        Err(err) => Evidence {
            kind: "plan",
            sql,
            ok: false,
            message: err.message.clone(),
            elapsed_ms: started.elapsed().as_millis() as u64,
            columns: Vec::new(),
            rows: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_is_not_ok() {
        let ev = Evidence::skipped("select 1".into(), "未指定连接，未做验证");
        assert_eq!(ev.kind, "skipped");
        assert!(!ev.ok, "没验证过就不能报 ok，否则前端会显示“已验证”");
        assert!(ev.rows.is_empty());
    }
}
