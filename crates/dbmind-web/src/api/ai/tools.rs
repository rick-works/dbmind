//! **规则类** AI 工具：不消耗模型、离线可用、结果确定。
//!
//! 这一批回答的是数据库客户端里最实在的几个问题：
//!
//! | 端点 | 回答的问题 |
//! |---|---|
//! | `sql/plan` | 这条语句会怎么跑？（执行计划） |
//! | `sql/try-run` | 这条语句能跑通吗？会返回什么？（只读试跑，限 100 行） |
//! | `ddl/estimate` | 这条 DDL 会有多大影响？要不要挑时间执行？ |
//! | `patrol` | 这批表有哪些共性的坑？（无主键、无索引、列全可空…） |
//! | `index-advisor` | 该给哪些列建索引？为什么？ |
//! | `insight` | 这张表的数据长什么样？（画像 + 可选模型解读） |
//!
//! ## 三条护栏
//!
//! 1. **只读判定用内核的**（`dbmind_core::is_read_only`）。自己写一套正则判定，
//!    迟早出现「界面说只读、内核拒绝执行」这种自相矛盾 —— 而用户只看到一条莫名其妙的报错。
//! 2. **试跑有硬上限**（默认 100、最多 500 行）：试跑的本意是「先看看」，
//!    不是「把表拉到浏览器里」。
//! 3. **巡检/索引建议要说明依据**：只给「建议建索引」而不说为什么，用户没法判断该不该听。

use axum::Json;
use dbmind_core::{ColumnDetail, TableInfo, TableKind};
use serde_json::{json, Value};

use crate::api::ai::read_only;

use crate::api::ai::AiRequest;
use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::meta::{rows_of, run_sql_in};
use crate::api::shape;
use crate::api::{blocking, require_record};
use crate::AppState;

/// 巡检 / 索引建议最多看多少张表（元数据要一张张取，几百张表会把界面拖死）。
const MAX_TABLES: usize = 80;
/// 试跑的默认与上限行数。
const TRY_RUN_DEFAULT: u64 = 100;
const TRY_RUN_MAX: u64 = 500;

fn need_conn(req: &AiRequest) -> XResult<String> {
    req.connection_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("缺少 connectionId 参数"))
}

/// 元数据助手：治理 / 质量两块都要用（`pub(crate)` 供 `governance`/`quality` 复用）。
pub(crate) async fn tables_of(
    state: &AppState,
    conn: &str,
    database: &str,
) -> XResult<Vec<TableInfo>> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let engine = state.engine();
    let tables = blocking(move || engine.list_tables_fresh(&target)).await?;
    Ok(tables
        .into_iter()
        .filter(|table| table.kind == TableKind::Table)
        .collect())
}

pub(crate) async fn columns_of(
    state: &AppState,
    conn: &str,
    database: &str,
    table: &str,
) -> XResult<Vec<ColumnDetail>> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    let engine = state.engine();
    let name = table.to_string();
    blocking(move || engine.list_columns_fresh(&target, &name)).await
}

// ------------------------------------------------------------------ 执行计划 / 试跑

/// `POST /api/ai/sql/plan` —— 执行计划（只读语句）。
pub async fn sql_plan(
    state: axum::extract::State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let sql = req.sql_text();
    if sql.is_empty() {
        return Err(XError::bad_request("缺少 sql 参数"));
    }
    if !read_only(&sql) {
        return Err(XError::bad_request(
            "执行计划只对只读查询开放：这条语句含写操作，请改用「试跑」或直接执行",
        ));
    }
    let conn = need_conn(&req)?;
    let database = req.database.clone().unwrap_or_default();
    let plan = crate::api::ai::chat::explain_sql(&state, &conn, &database, &sql).await?;
    match plan {
        Some(text) => Ok(Json(json!({
            "success": true,
            "plan": text,
            "sql": sql,
        }))),
        None => Err(XError::bad_request(format!(
            "{} 不支持用一条 EXPLAIN 取执行计划（SQL Server 需要 SET SHOWPLAN，Oracle/DB2 需要 PLAN_TABLE）",
            require_record(&state, &conn).await?.kind().key().to_ascii_uppercase()
        ))),
    }
}

/// `POST /api/ai/sql/try-run` —— 只读试跑。
pub async fn sql_try_run(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let sql = req.sql_text();
    if sql.is_empty() {
        return Err(XError::bad_request("缺少 sql 参数"));
    }
    if !read_only(&sql) {
        return Err(XError::bad_request(
            "试跑只接受只读查询（这是为了让你放心点它）：这条语句含写操作",
        ));
    }
    let conn = need_conn(&req)?;
    let database = req.database.clone().unwrap_or_default();
    let limit = req.limit.unwrap_or(TRY_RUN_DEFAULT).clamp(1, TRY_RUN_MAX);
    let result = run_sql_in(&state, &conn, &database, sql.clone(), limit as usize).await?;
    Ok(Json(json!({
        "success": true,
        "result": shape::query_result_json(&result),
        "limit": limit,
    })))
}

// ------------------------------------------------------------------ DDL 影响预估

/// `POST /api/ai/ddl/estimate` —— DDL 影响预估（规则判断，不调模型）。
pub async fn ddl_estimate(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let sql = req.sql_text();
    if sql.is_empty() {
        return Err(XError::bad_request("缺少 sql 参数"));
    }
    let lower = sql.to_ascii_lowercase();
    let mut items: Vec<Value> = Vec::new();
    let mut level = "low";

    // 规则一：DROP / TRUNCATE 是不可逆的
    if lower.contains("drop table") || lower.contains("truncate") {
        items.push(json!({
            "level": "high",
            "title": "不可逆的数据丢失",
            "detail": "DROP/TRUNCATE 会直接删掉数据与结构，且没有回退。执行前请确认已备份。",
        }));
        level = "high";
    }
    if lower.contains("drop column") {
        items.push(json!({
            "level": "high",
            "title": "删列会丢数据",
            "detail": "DROP COLUMN 删除该列的全部数据；对下游 SQL/报表是破坏性变更。",
        }));
        level = "high";
    }
    // 规则二：加列带默认值 / NOT NULL 可能整表重写
    if lower.contains("add column") || lower.contains("add ") && lower.contains(" not null") {
        if lower.contains("default") {
            items.push(json!({
                "level": "medium",
                "title": "加列带默认值可能重写全表",
                "detail": "老版本 MySQL/PostgreSQL 会为已有行回填默认值（整表重写）。\
                           大表上建议先加可空列、分批回填、再加约束。",
            }));
            level = if level == "high" { "high" } else { "medium" };
        } else {
            items.push(json!({
                "level": "low",
                "title": "加可空列通常很快",
                "detail": "只改元数据，一般不重写数据。",
            }));
        }
    }
    // 规则三：改类型 / 加索引（锁表风险）
    if lower.contains("modify column") || lower.contains("alter column") || lower.contains("change column") {
        items.push(json!({
            "level": "medium",
            "title": "改列类型可能需要重建",
            "detail": "类型转换会重写该列数据；期间可能持有表锁，请在低峰执行。",
        }));
        level = if level == "high" { "high" } else { "medium" };
    }
    if (lower.starts_with("create index") || lower.contains("create index"))
        && !lower.contains("concurrently")
    {
        items.push(json!({
            "level": "medium",
            "title": "建索引会锁写入",
            "detail": "PostgreSQL 可用 `CREATE INDEX CONCURRENTLY` 避免长时间阻塞写入；MySQL 8 支持 ONLINE DDL。",
        }));
        level = if level == "high" { "high" } else { "medium" };
    }
    // 规则四：大表提示（用行数估算，取不到就跳过）
    if let (Some(conn), Some(table)) = (
        req.connection_id.clone().filter(|value| !value.trim().is_empty()),
        extract_table_name(&sql),
    ) {
        let database = req.database.clone().unwrap_or_default();
        if let Ok(tables) = tables_of(&state, &conn, &database).await {
            if let Some(found) = tables
                .iter()
                .find(|item| item.name.eq_ignore_ascii_case(&table))
            {
                if let Some(rows) = found.row_estimate {
                    if rows > 1_000_000 {
                        items.push(json!({
                            "level": "high",
                            "title": format!("目标表很大（约 {rows} 行）"),
                            "detail": "大表上的结构变更耗时与锁窗口都会被放大，建议先在影子表上演练。",
                        }));
                        level = "high";
                    } else {
                        items.push(json!({
                            "level": "low",
                            "title": format!("目标表约 {rows} 行"),
                            "detail": "规模不大，常规变更窗口即可。",
                        }));
                    }
                }
            }
        }
    }
    if items.is_empty() {
        items.push(json!({
            "level": "low",
            "title": "常规结构变更",
            "detail": "没有识别到高风险模式（DROP / 改类型 / 建索引 / 大表）。仍建议在测试库先验证。",
        }));
    }
    let summary = format!(
        "影响等级：{}。{}",
        match level {
            "high" => "高",
            "medium" => "中",
            _ => "低",
        },
        items
            .iter()
            .map(|item| item["title"].as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
            .join("；")
    );
    Ok(Json(json!({
        "success": true,
        "level": level,
        "summary": summary,
        "items": items,
    })))
}

/// 从 DDL 里抠出表名（够用就好：这些语句的第一张表就是目标表）。
fn extract_table_name(sql: &str) -> Option<String> {
    let lower = sql.to_ascii_lowercase();
    for keyword in ["table", "index", "view"] {
        if let Some(position) = lower.find(keyword) {
            let rest = &sql[position + keyword.len()..];
            let rest = rest.trim_start();
            let rest = rest
                .strip_prefix("if not exists")
                .unwrap_or(rest)
                .trim_start();
            let rest = rest.strip_prefix("if exists").unwrap_or(rest).trim_start();
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '"' | '`' | '[' | ']'))
                .collect();
            let name = name.trim_matches(|c| matches!(c, '"' | '`' | '[' | ']')).to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

// ------------------------------------------------------------------ 巡检 / 索引建议

/// `POST /api/ai/patrol` —— 表健康巡检。
pub async fn patrol(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let conn = need_conn(&req)?;
    let database = req.database.clone().unwrap_or_default();
    let tables = tables_of(&state, &conn, &database).await?;
    let mut items: Vec<Value> = Vec::new();
    let mut high = 0;
    let mut medium = 0;
    for table in tables.iter().take(MAX_TABLES) {
        let columns = columns_of(&state, &conn, &database, &table.name).await?;
        if columns.is_empty() {
            continue;
        }
        let mut issues: Vec<String> = Vec::new();
        if columns.iter().all(|column| !column.primary_key) {
            issues.push("没有主键：无法按行定位，同步/对比/就地编辑都会失效".to_string());
        }
        let nullable = columns.iter().filter(|column| column.nullable).count();
        if nullable * 2 > columns.len() {
            issues.push(format!(
                "{} / {} 列可空：过度可空往往意味着约束缺失，容易积累脏数据",
                nullable,
                columns.len()
            ));
        }
        if columns.iter().any(|column| {
            column.type_name.as_deref().unwrap_or("").to_ascii_lowercase().contains("char")
                && column.type_name.as_deref().unwrap_or("").to_ascii_lowercase().contains("max")
        }) {
            issues.push("存在超长文本类型（max/longtext）：检索与索引代价高".to_string());
        }
        if issues.is_empty() {
            continue;
        }
        let level = if issues.iter().any(|issue| issue.starts_with("没有主键")) {
            high += 1;
            "high"
        } else {
            medium += 1;
            "medium"
        };
        items.push(json!({
            "table": table.name,
            "level": level,
            "rows": table.row_estimate,
            "columns": columns.len(),
            "issues": issues,
        }));
    }
    Ok(Json(json!({
        "success": true,
        "items": items,
        "summary": {
            "tables": tables.len(),
            "scanned": tables.len().min(MAX_TABLES),
            "high": high,
            "medium": medium,
        },
    })))
}

/// `POST /api/ai/index-advisor` —— 索引建议（规则式，说明依据）。
pub async fn index_advisor(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let conn = need_conn(&req)?;
    let database = req.database.clone().unwrap_or_default();
    let dialect = Dialect::new(require_record(&state, &conn).await?.kind());
    let tables = tables_of(&state, &conn, &database).await?;
    let mut items: Vec<Value> = Vec::new();

    for table in tables.iter().take(MAX_TABLES) {
        let columns = columns_of(&state, &conn, &database, &table.name).await?;
        if columns.is_empty() {
            continue;
        }
        // 规则一：没有主键 ⇒ 建主键（这是最值得做的一件事）
        if columns.iter().all(|column| !column.primary_key) {
            let candidate = columns
                .iter()
                .find(|column| column.name.to_ascii_lowercase() == "id")
                .or_else(|| {
                    columns.iter().find(|column| {
                        let name = column.name.to_ascii_lowercase();
                        !column.nullable && (name.ends_with("id") || name.ends_with("_no") || name.ends_with("_key"))
                    })
                });
            if let Some(column) = candidate {
                items.push(json!({
                    "table": table.name,
                    "column": column.name,
                    "kind": "primary_key",
                    "reason": "该表没有主键：没有主键的表无法可靠地定位/同步单行数据",
                    "ddl": format!(
                        "alter table {} add primary key ({});",
                        dialect.quote(&table.name),
                        dialect.quote(&column.name)
                    ),
                }));
            } else {
                items.push(json!({
                    "table": table.name,
                    "column": Value::Null,
                    "kind": "primary_key",
                    "reason": "该表没有主键，且没有找到合适的主键候选列（可考虑新增自增 id 列）",
                    "ddl": Value::Null,
                }));
            }
        }
        // 规则二：外键风格命名但（很可能）没索引的列
        for column in &columns {
            let name = column.name.to_ascii_lowercase();
            let looks_like_fk = name.ends_with("_id")
                || name.ends_with("_code")
                || name.ends_with("_no");
            if !looks_like_fk || column.primary_key {
                continue;
            }
            items.push(json!({
                "table": table.name,
                "column": column.name,
                "kind": "index",
                "reason": format!("列名 {name} 看起来是关联键：关联查询的驱动列没有索引会导致全表扫描"),
                "ddl": format!(
                    "create index idx_{}_{} on {} ({});",
                    table.name.chars().filter(|c| c.is_alphanumeric()).take(20).collect::<String>(),
                    column.name.chars().filter(|c| c.is_alphanumeric()).take(20).collect::<String>(),
                    dialect.quote(&table.name),
                    dialect.quote(&column.name)
                ),
            }));
            if items.len() >= 100 {
                break;
            }
        }
    }

    Ok(Json(json!({
        "success": true,
        "items": items,
        "summary": {
            "tables": tables.len(),
            "scanned": tables.len().min(MAX_TABLES),
            "suggestions": items.len(),
        },
    })))
}

// ------------------------------------------------------------------ 数据洞察

/// `POST /api/ai/insight` —— 表数据画像（**规则部分不依赖模型**）。
///
/// 前端会同时显示 `content` 与 `stats`：`stats` 是我们算出来的画像，
/// `content` 是模型对画像的解读（没配模型时给一段规则化的结论，功能不残）。
pub async fn insight(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let conn = need_conn(&req)?;
    let database = req.database.clone().unwrap_or_default();
    let table = req
        .table
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("请指定要分析的表名（table）"))?;
    // 洞察的核心结论来自模型。以前未配置时降级成规则画像 —— 用户看到的是
    // 「没报错但只有统计表格」的半成品，还以为是分析完了；与其它 AI 技能一致，
    // 未配置就直接引导去设置。
    if crate::api::ai::config::resolve(req.model_id.as_deref()).is_err() {
        return Err(XError::bad_request("请先在「设置」中启用并配置 AI 服务"));
    }
    let dialect = Dialect::new(require_record(&state, &conn).await?.kind());
    let columns = columns_of(&state, &conn, &database, &table).await?;
    if columns.is_empty() {
        return Err(XError::bad_request(format!("表 {table} 不存在或没有列信息")));
    }

    let count_sql = dialect.count_sql(&table);
    let total = match run_sql_in(&state, &conn, &database, count_sql, 1).await {
        // 取不到行数报 -1（未知），**不是 0**：0 会被读成"这是张空表"，而 AI 会照着这个数
        // 去下结论。以前写 `as_i64().unwrap_or(0)`，而 ClickHouse 的 count 回来是浮点
        // （Real）→ 取不出来 → 恒 0。
        Ok(result) => rows_of(&result)
            .first()
            .and_then(|row| row.values().find_map(shape::value_to_i64))
            .unwrap_or(-1),
        Err(_) => -1,
    };

    let mut stats = format!("表 {table}：{total} 行，{} 列\n", columns.len());
    // 逐列做 null / distinct / 极值画像（列多的时候只看前 20 列，避免几十次往返）
    for column in columns.iter().take(20) {
        let quoted = dialect.quote(&column.name);
        let sql = format!(
            "select count(*) as total, count({quoted}) as non_null, count(distinct {quoted}) as distinct_count, \
             min({quoted}) as min_value, max({quoted}) as max_value from {}",
            dialect.quote(&table)
        );
        if let Ok(result) = run_sql_in(&state, &conn, &database, sql, 1).await {
            if let Some(row) = rows_of(&result).first() {
                let non_null = row.get("non_null").and_then(shape::value_to_i64).unwrap_or(0);
                let distinct = row.get("distinct_count").and_then(shape::value_to_i64).unwrap_or(0);
                let null_ratio = if total > 0 {
                    format!("{:.1}%", (total - non_null) as f64 * 100.0 / total as f64)
                } else {
                    "—".to_string()
                };
                stats.push_str(&format!(
                    "  {} ｜ 空值率 {null_ratio} ｜ 去重值 {distinct} ｜ {}({}) ~ {}({})\n",
                    column.name,
                    column.name,
                    row.get("min_value").cloned().unwrap_or(Value::Null),
                    column.name,
                    row.get("max_value").cloned().unwrap_or(Value::Null),
                ));
            }
        }
    }

    // 入口已确保模型已配置：画像交给模型解读，不再有规则降级路径
    let question = req.ask_text();
    let mut content = String::new();
    let mut last_usage: Option<crate::api::ai::config::TokenUsage> = None;
    if let Ok(model) = crate::api::ai::config::resolve(req.model_id.as_deref()) {
        let system = "你是一名数据分析师。请根据给出的表数据画像，用中文分点给出：数据分布特征、\
                      质量问题（空值/重复/异常极值）、以及值得进一步排查的点。不要编造画像里没有的信息。";
        let user = format!(
            "### 表数据画像\n{stats}\n\n### 用户关注的问题\n{}",
            if question.is_empty() { "（未指定）" } else { &question }
        );
        let messages = vec![crate::api::ai::config::Msg {
            role: "user".to_string(),
            content: user,
        }];
        match crate::api::ai::run_blocking(move || {
            crate::api::ai::config::chat_with_usage(&model, system, &messages)
        })
        .await
        {
            Ok((text, usage)) => {
                content = text;
                last_usage = usage;
            }
            Err(err) => {
                content.push_str(&format!("模型解读失败：{}", err.message));
            }
        }
    }
    Ok(Json(json!({
        "success": true,
        "content": content,
        "stats": stats,
        "rows": total,
        "columns": columns.len(),
        "usage": last_usage,
    })))
}
