//! `/api/ai/agent` —— **智能体：把「问答」变成「自己去查」**。
//!
//! 前 100 多个 AI 接口都是「一问一答」：把已知的元数据塞进提示词，模型吐一段文本。
//! 但很多问题（「哪张表最能反映业务活跃度？」「上周的订单为什么少了？」）的答案
//! **不在提示词里，而在数据里** —— 除非模型能自己动手查，否则它只能猜，而猜数据库的问题
//! 比不回答更糟。
//!
//! 所以这里是唯一一个**多轮循环**：模型提出工具调用 → 我们执行 → 把结果回灌 → 它继续。
//!
//! ## 三个工具，以及为什么只有三个
//!
//! | 工具 | 用途 |
//! |---|---|
//! | `list_tables` | 先有「库里有什么」，才谈得上问「哪张表…」 |
//! | `describe_table` | 列名/类型/主键 —— 写对 SQL 的前提 |
//! | `run_readonly_sql` | 真正取数（最多 100 行） |
//!
//! 不加「写库/改结构/删表」类工具是**刻意的**：智能体的价值在于把「你去看一眼数据」
//! 这件事自动化，而不是把改数据这件事自动化。真需要改，它给出 SQL，由用户自己执行 ——
//! 那时用户看得到、也拦得住。
//!
//! ## 两层只读
//!
//! 第一层是**语句分类**（`statement::is_read_only`）：不是只读就直接拒，并把原因当
//! 工具结果回给模型（它能据此改口，而不是撞墙后卡死）。
//! 第二层是**内核闸门**（`AccessContext::Ai`）：AI 通道在内核里默认按只读处理，
//! 即使第一层被绕过，引擎也会拦。两道锁不是冗余 —— 一层是「体验」、一层是「安全」，
//! 前者可以绕过（比如新语法），后者不行。
//!
//! ## 边界
//!
//! - 最多 `maxRounds` 轮（默认 6，上限 12）。用完还没结论就**如实说没查完**，
//!   并附上最后一轮的工具结果 —— 而不是硬编一个答案。
//! - 工具结果截断到 4000 字符：一次 `select *` 的返回能把上下文挤爆，
//!   那时模型会开始胡说，不如让它看到「结果被截断了，请加 where/limit」。

use std::time::Instant;

use axum::extract::State;
use axum::Json;
use dbmind_core::{is_read_only, AccessContext, QueryOptions, QueryRequest};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::ai::tools::{columns_of, tables_of};
use crate::api::ai::{config, run_blocking};
use crate::api::blocking;
use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::shape;
use crate::AppState;

/// 默认轮数上限。6 轮足够「看表 → 看结构 → 试一条 → 再修一条 → 总结」，
/// 再多基本就是模型在绕圈。
const DEFAULT_ROUNDS: u32 = 6;
const MAX_ROUNDS: u32 = 12;
/// 单次工具结果回灌给模型的最大字符数。
const TOOL_RESULT_LIMIT: usize = 4000;
/// 工具查询最多取多少行。
const TOOL_ROWS: usize = 100;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentReq {
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub database: Option<String>,
    /// 提问：前端不同入口叫 `question` / `prompt` / `text`，都收
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub max_rounds: Option<u32>,
}

impl AgentReq {
    fn question(&self) -> XResult<String> {
        for candidate in [&self.question, &self.prompt, &self.text] {
            if let Some(text) = candidate {
                if !text.trim().is_empty() {
                    return Ok(text.trim().to_string());
                }
            }
        }
        Err(XError::bad_request("请提供问题（question）"))
    }
}

fn tool_defs() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "list_tables",
                "description": "列出当前库里的表和视图。不确定库里有什么表时先调这个。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "keyword": { "type": "string", "description": "可选，按表名过滤的关键字" }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "describe_table",
                "description": "查看一张表的字段定义（名称/类型/是否可空/主键）。写 SQL 前先调这个。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "table": { "type": "string", "description": "表名" }
                    },
                    "required": ["table"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "run_readonly_sql",
                "description": "执行一条只读 SQL 并返回结果（最多 100 行）。只允许 select/show/explain/desc 等读语句；写语句会被拒绝。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "sql": { "type": "string", "description": "只读 SQL" }
                    },
                    "required": ["sql"]
                }
            }
        }
    ])
}

fn system_prompt(kind: &str, database: &str, table_hint: &str) -> String {
    let db = if database.trim().is_empty() {
        "连接默认库".to_string()
    } else {
        database.to_string()
    };
    format!(
        "你是一个数据库智能体，通过工具去「看」真实的库来回答问题。\n\
         当前连接类型：{kind}；当前库：{db}。\n\
         {table_hint}\n\
         工作方式：\n\
         1. 先用 list_tables 看有哪些表、用 describe_table 看字段，再写 SQL —— 不要凭空猜表名和列名。\n\
         2. 只读：run_readonly_sql 只能执行查询，写/改/删会被拒绝。需要改数据时，把 SQL 写进回答里让用户自己执行。\n\
         3. SQL 要贴合上面的连接类型方言；结果被截断就加 where / limit 再查一次。\n\
         4. 结论必须来自工具返回的真实数据，**不要编造数字**；查不到就说查不到。\n\
         5. 最终回答用中文、简短直接，先给结论，再附上你用到的关键 SQL。"
    )
}

/// 助手消息只保留必要字段（避免把 `refusal`/`audio` 之类原样回灌）。
fn assistant_message(message: &Value) -> Value {
    let mut out = json!({ "role": "assistant" });
    out["content"] = message.get("content").cloned().unwrap_or(json!(""));
    if let Some(calls) = message.get("tool_calls") {
        out["tool_calls"] = calls.clone();
    }
    out
}

fn truncate(text: String) -> String {
    if text.chars().count() <= TOOL_RESULT_LIMIT {
        return text;
    }
    let head: String = text.chars().take(TOOL_RESULT_LIMIT).collect();
    format!("{head}\n…（结果超过 {TOOL_RESULT_LIMIT} 字符已截断，请加 where / limit 缩小范围）")
}

/// 把查询结果渲染成模型读得懂的紧凑文本（TSV 比 JSON 省一半 token）。
fn render_rows(result: &dbmind_core::QueryResult) -> String {
    let names: Vec<String> = result
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect();
    let mut out = names.join("\t");
    for row in &result.rows {
        out.push('\n');
        out.push_str(
            &row.iter()
                .map(shape::cell_to_value)
                .map(|value| match value {
                    Value::Null => String::new(),
                    Value::String(text) => text,
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\t"),
        );
    }
    if result.rows.is_empty() {
        out.push_str("\n（没有数据）");
    } else {
        out.push_str(&format!("\n（{} 行）", result.rows.len()));
    }
    out
}

/// 以 **AI 通道** 身份执行一条只读语句。
async fn run_ai_sql(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
) -> XResult<dbmind_core::QueryResult> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let engine = state.engine();
    let request = QueryRequest {
        read_only: None,
        connection: target,
        sql,
        options: QueryOptions {
            max_rows: TOOL_ROWS,
            timeout_ms: 60_000,
        },
        execution_id: None,
        // 会话名带 ai: 前缀：日志与宿主会话里能一眼看出「这条是智能体跑的」
        session: Some("ai:agent".to_string()),
        // **不进查询历史**：这些是智能体自己发的探测语句，不是用户敲的 SQL。
        // 首页「最近查询」的语义是「**我**执行过什么」—— 智能体一轮对话要跑十几条，
        // 混进去就把用户自己的操作淹掉了（对话本身留在 AI 面板里，查得到）。
        internal: true,
    };
    blocking(move || engine.execute(request, AccessContext::Ai)).await
}

/// 执行一次工具调用，返回 `(是否成功, 给模型看的文本)`。
async fn execute_tool(
    state: &AppState,
    conn: &str,
    database: &str,
    protocol: dbmind_core::RuntimeProtocol,
    name: &str,
    args: &Value,
) -> (bool, String) {
    match name {
        "list_tables" => {
            let keyword = args
                .get("keyword")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            match tables_of(state, conn, database).await {
                Ok(tables) => {
                    let lines: Vec<String> = tables
                        .iter()
                        .filter(|table| {
                            keyword.is_empty() || table.name.to_ascii_lowercase().contains(&keyword)
                        })
                        .take(200)
                        .map(|table| {
                            format!(
                                "{}\t{}\t{}",
                                table.name,
                                if table.kind == dbmind_core::TableKind::View {
                                    "视图"
                                } else {
                                    "表"
                                },
                                table
                                    .row_estimate
                                    .map(|rows| format!("约 {rows} 行"))
                                    .unwrap_or_else(|| "行数未知".to_string())
                            )
                        })
                        .collect();
                    if lines.is_empty() {
                        (true, "（没有匹配的表）".to_string())
                    } else {
                        (
                            true,
                            format!("表名\t类型\t行数估算\n{}", lines.join("\n")),
                        )
                    }
                }
                Err(err) => (false, err.message),
            }
        }
        "describe_table" => {
            let table = args.get("table").and_then(Value::as_str).unwrap_or("");
            if table.trim().is_empty() {
                return (false, "缺少 table 参数".to_string());
            }
            match columns_of(state, conn, database, table.trim()).await {
                Ok(columns) => {
                    if columns.is_empty() {
                        return (false, format!("表 {table} 不存在或没有列信息"));
                    }
                    let lines: Vec<String> = columns
                        .iter()
                        .map(|column| {
                            format!(
                                "{}\t{}\t{}\t{}",
                                column.name,
                                column.type_name.clone().unwrap_or_else(|| "?".to_string()),
                                if column.nullable { "可空" } else { "非空" },
                                if column.primary_key { "主键" } else { "" }
                            )
                        })
                        .collect();
                    (
                        true,
                        format!("字段\t类型\t可空\t标记\n{}", lines.join("\n")),
                    )
                }
                Err(err) => (false, err.message),
            }
        }
        "run_readonly_sql" => {
            let sql = args.get("sql").and_then(Value::as_str).unwrap_or("").trim();
            if sql.is_empty() {
                return (false, "缺少 sql 参数".to_string());
            }
            // 第一层：语句分类（复用内核的判定，规则只有一份）。理由要说清楚 ——
            // 模型看到原因才能换个写法继续，而不是原地重试同一条被拒的语句
            if !is_read_only(protocol, sql) {
                return (
                    false,
                    "拒绝执行：这个工具只允许只读查询（select / show / explain / desc / with）。\
                     如果你需要修改数据，请把 SQL 写在最终回答里，由用户自己确认执行。"
                        .to_string(),
                );
            }
            match run_ai_sql(state, conn, database, sql.to_string()).await {
                Ok(result) => (true, render_rows(&result)),
                Err(err) => (false, err.message),
            }
        }
        other => (false, format!("未知工具：{other}")),
    }
}

/// `POST /api/ai/agent`。
pub async fn run(
    State(state): State<AppState>,
    Json(req): Json<AgentReq>,
) -> XResult<Json<Value>> {
    let conn = req
        .connection_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("缺少 connectionId 参数"))?;
    let database = req.database.clone().unwrap_or_default();
    let question = req.question()?;
    let record = crate::api::require_record(&state, &conn).await?;
    let kind = record.kind();
    let dialect = Dialect::new(kind);
    let protocol = kind.protocol();
    let model = config::resolve(req.model_id.as_deref())?;
    let max_rounds = req
        .max_rounds
        .unwrap_or(DEFAULT_ROUNDS)
        .clamp(1, MAX_ROUNDS);

    // 表清单先探一次当「开场提示」：能省掉一整轮工具往返，
    // 而且模型知道表名后，第一句 SQL 就不容易写错
    let table_hint = match tables_of(&state, &conn, &database).await {
        Ok(tables) if !tables.is_empty() => format!(
            "库里已有的表（供参考，仍以 list_tables 的结果为准）：{}",
            tables
                .iter()
                .take(40)
                .map(|table| table.name.clone())
                .collect::<Vec<_>>()
                .join("、")
        ),
        _ => String::new(),
    };
    let system = system_prompt(dialect.kind.key(), &database, &table_hint);

    let mut messages: Vec<Value> = vec![json!({ "role": "user", "content": question })];
    let mut steps: Vec<Value> = Vec::new();
    let mut answer = String::new();
    let mut rounds_used = 0u32;
    let mut stopped = "finished";

    for round in 1..=max_rounds {
        rounds_used = round;
        let model_call = model.clone();
        let system_call = system.clone();
        let messages_call = messages.clone();
        // 同步 HTTP 在 blocking 线程池里跑：tokio 的工作线程不能被一次模型调用占住
        let message =
            run_blocking(move || config::chat_tools(&model_call, &system_call, &messages_call, &tool_defs()))
                .await?;
        let content = message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        messages.push(assistant_message(&message));

        if calls.is_empty() {
            answer = content;
            stopped = "finished";
            break;
        }
        for call in calls {
            let name = call
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // arguments 是**字符串**（OpenAI 规范），有些实现直接给对象，两种都认
            let args = match call
                .get("function")
                .and_then(|function| function.get("arguments"))
            {
                Some(Value::String(text)) => {
                    serde_json::from_str::<Value>(text).unwrap_or_else(|_| json!({}))
                }
                Some(value @ Value::Object(_)) => value.clone(),
                _ => json!({}),
            };
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("call")
                .to_string();
            let started = Instant::now();
            let (ok, result) =
                execute_tool(&state, &conn, &database, protocol, &name, &args).await;
            let duration_ms = started.elapsed().as_millis() as u64;
            steps.push(json!({
                "round": round,
                "tool": name,
                "args": args,
                "ok": ok,
                "durationMs": duration_ms,
                "result": result.chars().take(600).collect::<String>(),
            }));
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "name": name,
                "content": truncate(result),
            }));
        }
        if round == max_rounds {
            stopped = "maxRounds";
            answer = content;
        }
    }

    if stopped == "maxRounds" && answer.trim().is_empty() {
        // 没查完就如实说没查完：附最后一轮的工具结果，用户接着问一句就能续上
        let last = steps
            .last()
            .and_then(|step| step.get("result"))
            .and_then(Value::as_str)
            .unwrap_or("");
        answer = format!(
            "已达到最大工具调用轮数（{max_rounds}），还没有形成结论。\n最后一次「{}」返回：\n{}",
            steps
                .last()
                .and_then(|step| step.get("tool"))
                .and_then(Value::as_str)
                .unwrap_or("工具"),
            last
        );
    }

    Ok(Json(json!({
        "success": true,
        "content": answer,
        "rounds": rounds_used,
        "maxRounds": max_rounds,
        "stopped": stopped,
        "steps": steps,
        "model": model.model,
        "readOnly": true,
    })))
}
