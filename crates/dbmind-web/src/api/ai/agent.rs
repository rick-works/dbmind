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
use dbmind_core::{is_read_only, AccessContext, AiJobRow, QueryOptions, QueryRequest};
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
///
/// 这个数不是「一条消息的上限」而是**复利**：工具结果会留在对话里，第 6 轮时前 5 次的
/// 结果都要重发一遍 —— 4000 字符 × 6 轮 ≈ 2.4 万字符（近 1 万 token）只为看过的行。
/// 3200 仍然能回几十行（够定位问题），但把复利砍掉两成。
const TOOL_RESULT_LIMIT: usize = 3200;
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
    /// 会话 id（可选）：变更单会挂到这个会话下，审计时能顺藤摸瓜找到当时的对话
    #[serde(default)]
    pub thread_id: Option<String>,
    /// 界面语言（`en` / `zh`）：英文界面下智能体的回答与变更单说明也用英文
    #[serde(default)]
    pub lang: Option<String>,
    /// 之前的对话轮次（`[{role, content}]`）。
    ///
    /// **智能体以前完全不收历史**：用户说「继续」它只能回答「上次的上下文我这边没有」，
    /// 然后再从头体检一遍 —— 又贵又不接话。多轮是对话的基本预期，这里收下并按
    /// 双预算裁剪（智能体还有工具结果要占上下文，所以比普通对话更紧）。
    #[serde(default)]
    pub history: Vec<config::Msg>,
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

///
/// 说明文字刻意压到一行：这份 schema 跟着**每一轮**请求走（默认 6 轮 = 白付 6 次），
/// 而模型只需要知道「什么情况下该调、参数是什么」。删掉的是举例与客套，语义一条没少。
fn tool_defs() -> Value {
    json!([
        { "type": "function", "function": {
            "name": "list_tables",
            "description": "列出当前库的表与视图（可按关键字过滤）。不确定有什么表时先调。",
            "parameters": { "type": "object", "properties": {
                "keyword": { "type": "string", "description": "表名关键字，可空" } }, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "describe_table",
            "description": "看一张表的字段（名/类型/可空/主键）。写 SQL 前先调。",
            "parameters": { "type": "object", "properties": {
                "table": { "type": "string", "description": "表名" } }, "required": ["table"] }
        }},
        { "type": "function", "function": {
            "name": "run_readonly_sql",
            "description": "跑一条只读 SQL，最多回 100 行；select/show/explain/desc 之外的语句会被拒。",
            "parameters": { "type": "object", "properties": {
                "sql": { "type": "string", "description": "只读 SQL" } }, "required": ["sql"] }
        }},
        { "type": "function", "function": {
            "name": "patrol_tables",
            "description": "结构巡检：无主键/过度可空/超长文本。纯规则不耗 token，问「这库有什么问题」先调。",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "suggest_indexes",
            "description": "给主键/索引建议，附可执行 DDL 与理由（只读规则，不改库）。table 留空扫整库。",
            "parameters": { "type": "object", "properties": {
                "table": { "type": "string", "description": "只看这一张表，可空" } }, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "sync_status",
            "description": "最近的同步任务状态（进度/失败行数/原因）。",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "backup_status",
            "description": "最近的备份/还原任务状态（成败/产物/耗时）。",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }},
        { "type": "function", "function": {
            "name": "propose_change",
            "description": "把一条 SQL 变成待用户确认的变更单（本工具不执行）。改数据/改结构必须先调它，返回单号、风险等级、撤销语句。",
            "parameters": { "type": "object", "properties": {
                "sql": { "type": "string", "description": "单条 SQL" },
                "title": { "type": "string", "description": "一句话说明做什么" },
                "reason": { "type": "string", "description": "依据（来自哪次查询）" } }, "required": ["sql"] }
        }}
    ])
}

fn system_prompt(kind: &str, database: &str, table_hint: &str) -> String {
    let db = if database.trim().is_empty() {
        "连接默认库".to_string()
    } else {
        database.to_string()
    };
    format!(
        "你是数据库智能体，靠工具「看」真实库来回答。\n\
         连接类型：{kind}；当前库：{db}。\n\
         {table_hint}\n\
         规则：\n\
         1. 先用 list_tables / describe_table 看清表和字段再写 SQL，不猜表名列名；\n\
         2. 查询用 run_readonly_sql（写/改/删会被拒）；**任何改数据或改结构的想法，\n\
            一律交给 propose_change 生成待确认的变更单**，绝不自己执行，也不要只把 SQL\n\
            写在回答里让用户自己贴（那样就没有风险分级、撤销语句和审计记录了）；\n\
         3. 问「库里有什么问题 / 哪张表慢」：先 patrol_tables 和 suggest_indexes，\n\
            再针对可疑表跑几条 SQL 拿证据，最后把值得做的改动写成变更单；\n\
         4. 问同步 / 备份情况：同步用 sync_status，备份用 backup_status，别猜；\n\
         5. SQL 贴合上面的方言；结果被截断就加 where / limit 再查；\n\
         6. 结论只来自工具返回的数据，**不编数字**，查不到就说查不到；\n\
         7. 中文、简短：先结论，再列证据，最后说要做什么（已生成的变更单 id 一并给出）；\n\
         8. 上面若有对话历史，**接着历史里的结论做**：用户说「继续 / 然后呢 / 就按这个来」\n\
             就沿最近一条结论往下推进，不重头再体检一遍，也绝不说「我没有上文」；\n\
             确实分不清指哪件事时，按最近一条结论先做一步，再问一句确认。"
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
        schema: None,
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

/// 结构化结果 → 给模型看的紧凑 JSON（pretty 只为可读，不额外增加含义）。
fn to_text(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// 执行一次工具调用，返回 `(是否成功, 给模型看的文本)`。
///
/// `changes` 是**出参**：`propose_change` 生成的变更单同时要回给前端（用户要点按钮），
/// 所以顺手收进来，最后随响应一起返回 —— 不靠模型在回答里「复述」id。
async fn execute_tool(
    state: &AppState,
    conn: &str,
    database: &str,
    protocol: dbmind_core::RuntimeProtocol,
    thread_id: &str,
    name: &str,
    args: &Value,
    changes: &mut Vec<Value>,
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
        "patrol_tables" => match crate::api::ai::tools::patrol_scan(state, conn, database).await {
            Ok(value) => (true, to_text(&value)),
            Err(err) => (false, err.message),
        },
        "suggest_indexes" => {
            let only = args
                .get("table")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let only = if only.is_empty() { None } else { Some(only) };
            match crate::api::ai::tools::index_suggestions(state, conn, database, only.as_deref()).await {
                Ok(value) => (true, to_text(&value)),
                Err(err) => (false, err.message),
            }
        }
        "sync_status" => {
            let rows = state.tasks.recent(&["sync"], 5);
            if rows.is_empty() {
                (true, "最近没有同步任务".to_string())
            } else {
                (true, to_text(&Value::Array(rows)))
            }
        }
        "backup_status" => {
            let rows = state.tasks.recent(&["backup", "restore"], 5);
            if rows.is_empty() {
                (true, "最近没有备份或还原任务".to_string())
            } else {
                (true, to_text(&Value::Array(rows)))
            }
        }
        "propose_change" => {
            let sql = args.get("sql").and_then(Value::as_str).unwrap_or("").trim();
            if sql.is_empty() {
                return (false, "缺少 sql 参数".to_string());
            }
            let title = args.get("title").and_then(Value::as_str).unwrap_or("");
            let reason = args.get("reason").and_then(Value::as_str).unwrap_or("");
            match crate::api::ai::teammate::create_change(
                state, conn, database, thread_id, "", title, reason, sql,
            )
            .await
            {
                Ok((id, risk, undo)) => {
                    changes.push(json!({
                        "id": id,
                        "title": if title.trim().is_empty() { "AI 建议的变更" } else { title.trim() },
                        "sql": sql,
                        "risk": risk,
                        "undoSql": undo,
                        "status": "pending",
                    }));
                    let reversible = if undo.is_some() {
                        "有撤销语句，执行后可以撤销。"
                    } else {
                        "这个动作没有可靠的撤销语句，执行前请先备份。"
                    };
                    let extra = if risk == "high" {
                        "，高危单还需要再点一次「确认」"
                    } else {
                        ""
                    };
                    (
                        true,
                        format!(
                            "已生成待确认的变更单 {id}（风险 {risk}）。它**尚未执行**：{reversible}\
                             请在回答里提示用户点「批准」{extra}。"
                        ),
                    )
                }
                Err(err) => (false, err.message),
            }
        }
        other => (false, format!("未知工具：{other}")),
    }
}

/// 一次循环的产出。
struct LoopOutcome {
    answer: String,
    rounds: u32,
    stopped: String,
}

/// 落一次 checkpoint：状态 + 已走步骤 + 当前答案。
///
/// 没有这一步，「第二天回来接着跑」就无从谈起 —— 中断时内存里的 messages/steps 全丢，
/// 待办里只剩一句「失败」，点进去什么都看不到。
fn checkpoint_job(job_id: &str, status: &str, steps: &[Value], result: &str) {
    if job_id.is_empty() {
        return;
    }
    let Some(store) = dbmind_core::global_store() else {
        return;
    };
    let text = serde_json::to_string(steps).unwrap_or_else(|_| "[]".to_string());
    let _ = store.ai_job_update(job_id, status, &text, result);
}

/// 多轮循环本体：**首次运行与中断续跑共用这一份**。
///
/// 参数多是因为它就是在操作一套运行态（消息、步骤、变更单、预算）；
/// 拆成结构体只会让调用点多一层包装，读起来更绕。
#[allow(clippy::too_many_arguments)]
async fn run_loop(
    state: &AppState,
    conn: &str,
    database: &str,
    protocol: dbmind_core::RuntimeProtocol,
    thread_id: &str,
    model: &config::ModelEntry,
    system: &str,
    messages: &mut Vec<Value>,
    steps: &mut Vec<Value>,
    changes: &mut Vec<Value>,
    max_rounds: u32,
    job_id: &str,
) -> XResult<LoopOutcome> {
    let mut answer = String::new();
    let mut rounds_used = 0u32;
    let mut stopped = "finished";

    for round in 1..=max_rounds {
        rounds_used = round;
        let model_call = model.clone();
        let system_call = system.to_string();
        let messages_call = messages.clone();
        // 同步 HTTP 在 blocking 线程池里跑：tokio 的工作线程不能被一次模型调用占住
        let message = match run_blocking(move || {
            config::chat_tools(&model_call, &system_call, &messages_call, &tool_defs())
        })
        .await
        {
            Ok(message) => message,
            Err(err) => {
                // 模型调用失败（断网 / 上游 5xx / 超时）：把已走到的步骤落库，
                // 状态置 failed —— 界面据此显示「失败可重试」，点一下能接着跑
                checkpoint_job(job_id, "failed", steps, &err.message);
                return Err(err);
            }
        };
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
            let (ok, result) = execute_tool(
                state,
                conn,
                database,
                protocol,
                thread_id,
                &name,
                &args,
                changes,
            )
            .await;
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
        // 每轮结束落一次：即使下一轮模型调用挂掉，这一轮的成果也保住了
        checkpoint_job(job_id, "running", steps, &answer);
        if round == max_rounds {
            stopped = "maxRounds";
            answer = content;
        }
    }

    Ok(LoopOutcome {
        answer,
        rounds: rounds_used,
        stopped: stopped.to_string(),
    })
}

/// 没形成结论时如实说没查完：附最后一轮的工具结果，用户接着问一句就能续上。
fn unfinished_note(steps: &[Value], max_rounds: u32) -> String {
    let last = steps
        .last()
        .and_then(|step| step.get("result"))
        .and_then(Value::as_str)
        .unwrap_or("");
    format!(
        "已达到最大工具调用轮数（{max_rounds}），还没有形成结论。\n最后一次「{}」返回：\n{}",
        steps
            .last()
            .and_then(|step| step.get("tool"))
            .and_then(Value::as_str)
            .unwrap_or("工具"),
        last
    )
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
    // 预算闸门：降级时连「表清单预探」都省掉 —— 那也是一次元数据往返，
    // 而超预算时最该省的就是上下文
    let downgraded = crate::api::ai::chat::gate_budget()?;
    let model = config::resolve_with_budget(req.model_id.as_deref(), downgraded)?;
    let max_rounds = req
        .max_rounds
        .unwrap_or(DEFAULT_ROUNDS)
        .clamp(1, MAX_ROUNDS);

    // 表清单先探一次当「开场提示」：能省掉一整轮工具往返，
    // 而且模型知道表名后，第一句 SQL 就不容易写错
    let table_hint = if downgraded {
        String::new()
    } else {
        match tables_of(&state, &conn, &database).await {
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
        }
    };
    // 语言约束：智能体的回答/变更单说明同样跟随界面语言
    let system = crate::api::ai::with_language(
        system_prompt(dialect.kind.key(), &database, &table_hint),
        req.lang.as_deref(),
    );

    // 待办**在跑之前**就建出来：中断才有东西可「继续」。
    // 以前是跑完才落库，中途断掉等于什么都没发生 —— 第二天回来接不上。
    let job_id = {
        let job = AiJobRow {
            id: String::new(),
            title: question.chars().take(30).collect(),
            goal: question.clone(),
            connection_id: conn.clone(),
            database_name: database.clone(),
            kind: "agent".to_string(),
            status: "running".to_string(),
            steps: "[]".to_string(),
            result: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        dbmind_core::global_store()
            .and_then(|store| store.ai_job_add(&job).ok())
            .unwrap_or_default()
    };

    let thread_id = req.thread_id.clone().unwrap_or_default();
    // 历史对话：6 条 / 4000 字符封顶（比普通对话的 6000 更紧 —— 智能体自己还要装
    // 工具定义与工具结果，两者抢同一份上下文）。**只收 user/assistant**，客户端塞来的
    // system 一律丢弃，否则等于绕过这里的 system 规则。
    let past = crate::api::ai::bound_history(&req.history, 6, 4000, 2000);
    let mut messages: Vec<Value> = Vec::with_capacity(past.len() + 1);
    for msg in &past {
        messages.push(json!({ "role": msg.role, "content": msg.content }));
    }
    messages.push(json!({ "role": "user", "content": question }));
    let mut steps: Vec<Value> = Vec::new();
    let mut changes: Vec<Value> = Vec::new();

    let outcome = run_loop(
        &state,
        &conn,
        &database,
        protocol,
        &thread_id,
        &model,
        &system,
        &mut messages,
        &mut steps,
        &mut changes,
        max_rounds,
        &job_id,
    )
    .await?;
    let mut answer = outcome.answer;
    if outcome.stopped == "maxRounds" && answer.trim().is_empty() {
        answer = unfinished_note(&steps, max_rounds);
    }

    // 收尾：有待确认的变更单 ⇒ `waiting`（界面排在待办最上面）；纯查询 ⇒ `done`。
    // 顺手把变更单挂到这张 job 上，待办点进去才看得到它产出的单子。
    let status = if changes.is_empty() { "done" } else { "waiting" };
    checkpoint_job(&job_id, status, &steps, &answer);
    if let Some(store) = dbmind_core::global_store() {
        let ids: Vec<String> = changes
            .iter()
            .filter_map(|change| change.get("id").and_then(Value::as_str).map(str::to_string))
            .collect();
        let _ = store.ai_change_attach_job(&ids, &job_id);
    }

    Ok(Json(json!({
        "success": true,
        "content": answer,
        "rounds": outcome.rounds,
        "maxRounds": max_rounds,
        "stopped": outcome.stopped,
        "steps": steps,
        "changes": changes,
        "jobId": job_id,
        "threadId": if thread_id.is_empty() { Value::Null } else { json!(thread_id) },
        "model": model.model,
        // 降级标记：超预算时上下文被压掉，前端据此提示「本次未读表结构」
        "budget": if downgraded { json!("downgraded") } else { Value::Null },
        // 「只读」现在指**智能体自己不写库**：它能出的最后一招是一张待确认的变更单，
        // 执行必须由人在界面上点。留着这个字段是为了让前端文案不用改。
        "readOnly": true,
    })))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResumeReq {
    #[serde(default)]
    pub model_id: Option<String>,
    /// 这次最多再跑几轮（默认沿用 6）
    #[serde(default)]
    pub max_rounds: Option<u32>,
}

/// `POST /api/ai/jobs/{id}/resume` —— 接着上次跑。
///
/// 做法是**重放**：把已落库的步骤还原成 `assistant(tool_calls) + tool(result)` 消息对，
/// 再交给同一个 `run_loop` 继续。不重新执行已完成的工具（那些查询可能很贵，
/// 而结论已经在步骤里了），所以「续跑」的代价就是一轮模型调用。
pub async fn resume(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(req): Json<ResumeReq>,
) -> XResult<Json<Value>> {
    let store =
        dbmind_core::global_store().ok_or_else(|| XError::bad_request("数据库未就绪"))?;
    let job = store
        .ai_job_get(&id)?
        .ok_or_else(|| XError::bad_request("待办不存在（可能已被清理）"))?;
    if job.kind != "agent" {
        return Err(XError::bad_request("只有智能体任务可以继续跑"));
    }
    if job.connection_id.trim().is_empty() {
        return Err(XError::bad_request("这张待办没有关联连接，无法继续"));
    }

    let record = crate::api::require_record(&state, &job.connection_id).await?;
    let kind = record.kind();
    let dialect = Dialect::new(kind);
    let protocol = kind.protocol();
    let max_rounds = req
        .max_rounds
        .unwrap_or(DEFAULT_ROUNDS)
        .clamp(1, MAX_ROUNDS);
    let model = config::resolve_with_budget(req.model_id.as_deref(), false)?;

    // 重放已走步骤。工具结果在落库时截到 600 字，重放时按同一口径告知模型
    // 「这是摘要」，免得它以为拿到了完整结果。
    let saved: Vec<Value> = serde_json::from_str(&job.steps).unwrap_or_default();
    let mut messages: Vec<Value> = vec![json!({ "role": "user", "content": job.goal })];
    let mut steps: Vec<Value> = Vec::new();
    for (index, step) in saved.iter().enumerate() {
        let name = step.get("tool").and_then(Value::as_str).unwrap_or("tool");
        let args = step.get("args").cloned().unwrap_or_else(|| json!({}));
        let result = step
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let call_id = format!("call_{index}");
        messages.push(json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string()),
                }
            }]
        }));
        messages.push(json!({
            "role": "tool",
            "tool_call_id": format!("call_{index}"),
            "name": name,
            "content": truncate(result),
        }));
        steps.push(step.clone());
    }

    let table_hint = match tables_of(&state, &job.connection_id, &job.database_name).await {
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
    let system = system_prompt(dialect.kind.key(), &job.database_name, &table_hint);

    checkpoint_job(&id, "running", &steps, &job.result);
    let mut changes: Vec<Value> = Vec::new();
    // `ai_jobs` 没有 thread_id 列（表是早先定的）：续跑出的变更单只挂 job、
    // 不挂会话。要挂会话得先给表加列并迁移，不值得为一个归属字段动表。
    let outcome = run_loop(
        &state,
        &job.connection_id,
        &job.database_name,
        protocol,
        "",
        &model,
        &system,
        &mut messages,
        &mut steps,
        &mut changes,
        max_rounds,
        &id,
    )
    .await?;

    let mut answer = outcome.answer;
    if outcome.stopped == "maxRounds" && answer.trim().is_empty() {
        answer = unfinished_note(&steps, max_rounds);
    }
    let ids: Vec<String> = changes
        .iter()
        .filter_map(|change| change.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let _ = store.ai_change_attach_job(&ids, &id);
    // 状态要看**这张 job 下还挂着几张待确认单**，不能只看本次新产出的：
    // 续跑时变更单是上一轮就出好的，若按「本次新增」判断就会把 waiting 误写回 done，
    // 待办里那张单子也就从「等你确认」变成「已完成」了。
    let pending = store
        .ai_change_list(Some("pending"), 500)
        .unwrap_or_default()
        .iter()
        .filter(|change| change.job_id == id)
        .count();
    let status = if changes.is_empty() && pending == 0 { "done" } else { "waiting" };
    checkpoint_job(&id, status, &steps, &answer);

    Ok(Json(json!({
        "success": true,
        "resumed": true,
        "jobId": id,
        "content": answer,
        "rounds": outcome.rounds,
        "maxRounds": max_rounds,
        "stopped": outcome.stopped,
        "steps": steps,
        "changes": changes,
        "model": model.model,
        "readOnly": true,
    })))
}
