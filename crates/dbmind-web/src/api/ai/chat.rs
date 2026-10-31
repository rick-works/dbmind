//! 对话与生成类 AI 接口（要调模型的那一批）。
//!
//! 这里所有接口都走同一条路：**提示词模板（可选插表结构）→ 调模型 → 按前端约定解析输出**。
//! 差异只在「用哪个模板」与「输出怎么解析」两件事上，所以抽出一个 `ask` 就够，
//! 每个接口只剩十来行 —— 而「每个接口自己拼一次提示词」的写法，迟早会出现
//! 某一个接口忘了插表结构（模型于是开始编表名）。
//!
//! ## SSE 的帧格式（必须逐字对上，前端是按这个解析的）
//!
//! ```text
//! data:{"d":"SELECT "}\n\n          增量（每个分片一条，data 后无空格）
//! event:done\ndata:{"done":true}\n\n  正常结束
//! data:{"error":"..."}\n\n           出错（**注意没有独立的 event 名**）
//! ```
//!
//! 三条容易错的地方：**不发明细的 `[DONE]`**（那是上游OpenAI 那一跳的东西）、
//! **错误也走 `data:` 而不是 `event:error`**、**分片里的换行必须被 JSON 转义**
//! （否则一个多行分片会直接把 SSE 的帧结构撑破）。

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::api::ai::config::{self, Msg, TokenUsage};
use crate::api::ai::prompts;
use crate::api::ai::teammate;
use crate::api::ai::{run_blocking, system_with_context, system_with_context_opts, with_language, AiRequest};
use crate::api::error::{XError, XResult};
use crate::AppState;

// ------------------------------------------------------------------ 公共底座

/// 一次「模板 → 模型」调用。
///
/// `with_context` 决定要不要把表结构插进 system：写 SQL 的接口需要（否则表名靠猜），
/// 翻译、改写这类接口不需要（它们只看给定 SQL）。
async fn ask(
    state: &AppState,
    req: &AiRequest,
    system_id: &str,
    user_id: &str,
    vars: &[(&str, &str)],
    with_context: bool,
) -> XResult<String> {
    let system_base = prompts::render(system_id, &[])?;
    // user 模板里已经有 {{schemaContext}}（fix / explain / optimize）⇒ system 不再拼一份。
    // 从前两边都拼，同一份表结构在请求里出现两次：promptTokens 直接翻倍，信息量一点没多。
    let schema_in_user = vars.iter().any(|(key, _)| *key == "schemaContext");
    let system = if with_context {
        system_with_context_opts(state, req, &system_base, !schema_in_user).await
    } else {
        // 不走上下文也要守语言：否则英文界面下「翻译/改写」这类调用照样中文
        with_language(system_base, req.lang.as_deref())
    };
    let user = prompts::render(user_id, vars)?;
    let model = config::resolve(req.model_id.as_deref())?;
    config::audit(user_id, &user);
    let messages = vec![Msg {
        role: "user".to_string(),
        content: user,
    }];
    run_blocking(move || config::chat(&model, &system, &messages)).await
}

/// 调用一个**内置在代码里**的提示词（上游里 nl2sql / 数据字典 / 翻译等也是硬编码的）。
/// 同 [`ask`]，但带用量（界面要在结果下展示 token 消耗）。
async fn ask_with_usage(
    state: &AppState,
    req: &AiRequest,
    system_id: &str,
    user_id: &str,
    vars: &[(&str, &str)],
    with_context: bool,
) -> XResult<(String, Option<TokenUsage>)> {
    let system_base = prompts::render(system_id, &[])?;
    // 同 ask：user 里带了 schemaContext 就不再往 system 里重复塞表结构
    let schema_in_user = vars.iter().any(|(key, _)| *key == "schemaContext");
    let system = if with_context {
        system_with_context_opts(state, req, &system_base, !schema_in_user).await
    } else {
        // 不走上下文也要守语言：否则英文界面下「翻译/改写」这类调用照样中文
        with_language(system_base, req.lang.as_deref())
    };
    let user = prompts::render(user_id, vars)?;
    let model = config::resolve(req.model_id.as_deref())?;
    config::audit(user_id, &user);
    let messages = vec![Msg {
        role: "user".to_string(),
        content: user,
    }];
    run_blocking(move || config::chat_with_usage(&model, &system, &messages)).await
}

async fn ask_inline(
    state: &AppState,
    req: &AiRequest,
    system: &str,
    user: String,
    with_context: bool,
) -> XResult<String> {
    let system = if with_context {
        system_with_context(state, req, system).await
    } else {
        with_language(system.to_string(), req.lang.as_deref())
    };
    let model = config::resolve(req.model_id.as_deref())?;
    config::audit("inline", &user);
    let messages = vec![Msg {
        role: "user".to_string(),
        content: user,
    }];
    run_blocking(move || config::chat(&model, &system, &messages)).await
}

/// 同 [`ask_inline`]，但带用量（数据字典等生成类接口要在界面上展示 token 消耗）。
async fn ask_inline_with_usage(
    state: &AppState,
    req: &AiRequest,
    system: &str,
    user: String,
    with_context: bool,
) -> XResult<(String, Option<TokenUsage>)> {
    let system = if with_context {
        system_with_context(state, req, system).await
    } else {
        with_language(system.to_string(), req.lang.as_deref())
    };
    let model = config::resolve(req.model_id.as_deref())?;
    config::audit("inline", &user);
    let messages = vec![Msg {
        role: "user".to_string(),
        content: user,
    }];
    run_blocking(move || config::chat_with_usage(&model, &system, &messages)).await
}

fn need_sql(req: &AiRequest) -> XResult<String> {
    let sql = req.sql_text();
    if sql.is_empty() {
        return Err(XError::bad_request("缺少 sql 参数"));
    }
    Ok(sql)
}

/// 从模型输出里抠出 SQL（去掉 ```sql 围栏这类包裹）。
fn extract_sql(text: &str) -> String {
    let text = text.trim();
    if let Some(start) = text.find("```") {
        let rest = &text[start + 3..];
        let rest = rest.strip_prefix("sql").unwrap_or(rest);
        let rest = rest.trim_start_matches(['\r', '\n']);
        if let Some(end) = rest.find("```") {
            return rest[..end].trim().to_string();
        }
        return rest.trim().to_string();
    }
    text.trim_matches('`').trim().to_string()
}

/// 默认 system 人格（`common.default-system` + 简洁约束）。
fn default_system() -> String {
    let base = prompts::text_of("common.default-system").unwrap_or_default();
    let brevity = prompts::text_of("common.brevity").unwrap_or_default();
    format!("{}\n{}", base.trim(), brevity.trim())
}

/// 记忆蒸馏：用户话里带「记住 / 口径 / 单位 / 规则…」这类信号时，把这一轮里
/// 值得长期记住的事实抽出来存进 `ai_memories`（见 `teammate::memory_extract`）。
///
/// 为什么要 spawn：蒸馏是**一次额外的模型调用**，同步等它等于把回答的响应时间
/// 又拉长一截；而它失败或超时对用户没有任何可见影响（下次仍走已有的记忆）。
fn spawn_memory_extract(state: &AppState, req: &AiRequest, question: &str) {
    let state = state.clone();
    let req = req.clone();
    let question = question.to_string();
    tokio::spawn(async move {
        teammate::memory_extract(&state, &req, &question).await;
    });
}

/// 预算闸门：`Ok(true)` = 本次降级回答（不注入上下文 + 可换便宜模型）。
///
/// 降级不改语义、只减上下文：用户问的问题照答，只是模型看不到表结构和记忆。
/// 拒绝则直接报错 —— 两条路都让用户知道发生了什么，不静默烧钱。
pub(crate) fn gate_budget() -> XResult<bool> {
    match config::budget_gate() {
        config::BudgetGate::Ok => Ok(false),
        config::BudgetGate::Downgrade { used, limit } => {
            tracing::warn!(used, limit, "AI 月度预算已超，本次降级回答");
            config::audit("budget/downgrade", &format!("本月已用 {used} / {limit} tokens"));
            Ok(true)
        }
        config::BudgetGate::Blocked { used, limit } => {
            Err(XError::bad_request(config::budget_block_message(used, limit)))
        }
    }
}

// ------------------------------------------------------------------ 对话

/// `POST /api/ai/chat` —— 普通对话（多轮历史由**前端**带上，服务端不存会话）。
pub async fn chat(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let question = req.ask_text();
    if question.is_empty() {
        return Err(XError::bad_request("请输入内容"));
    }
    let base = req
        .system
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(default_system);
    // 会话记账：先落用户这一句（这样即使模型调用失败，用户问过什么也留得住）
    let thread = teammate::begin_turn(
        req.thread_id.as_deref(),
        req.persist,
        req.connection_id.as_deref().unwrap_or_default(),
        req.database.as_deref().unwrap_or_default(),
        &question,
    );
    if let Some(thread) = &thread {
        teammate::append_message(thread, "user", &question, None);
    }
    // 预算闸门：超了要么降级（不注入上下文 + 可选便宜模型），要么直接拒绝
    let downgraded = gate_budget()?;
    let system = if downgraded {
        // 降级只是不注入上下文，语言约束照旧
        with_language(base.clone(), req.lang.as_deref())
    } else {
        system_with_context(&state, &req, &base).await
    };
    let model = config::resolve_with_budget(req.model_id.as_deref(), downgraded)?;
    // 双预算（条数 + 字符）：原样转发时，一条带长 SQL 的回答就能让下面每一轮都多付几千 token
    let mut messages = req.bounded_history();
    messages.push(Msg {
        role: "user".to_string(),
        content: question.clone(),
    });
    config::audit("chat", &question);
    let (content, usage) =
        run_blocking(move || config::chat_with_usage(&model, &system, &messages)).await?;
    if let Some(thread) = &thread {
        teammate::append_message(thread, "assistant", &content, usage.as_ref());
    }
    spawn_memory_extract(&state, &req, &question);
    Ok(Json(json!({
        "success": true,
        "content": content,
        "usage": usage,
        "threadId": thread,
        // 前端可以据此在回答下标一句「本次已降级」；不读也不影响功能
        "budget": if downgraded { json!("downgraded") } else { Value::Null },
    })))
}

/// `POST /api/ai/chat/stream` —— 流式对话（SSE，见文件头的帧格式）。
pub async fn chat_stream(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Response> {
    let question = req.ask_text();
    if question.is_empty() {
        return Err(XError::bad_request("请输入内容"));
    }
    let base = req
        .system
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(default_system);
    // 预算闸门放在拼上下文之前：降级时那份上下文根本不需要生成（省一次元数据往返）
    let downgraded = gate_budget()?;
    let system = if downgraded {
        // 降级只是不注入上下文，语言约束照旧
        with_language(base.clone(), req.lang.as_deref())
    } else {
        system_with_context(&state, &req, &base).await
    };
    // 未配置模型等错误在这一步就以 JSON 返回（前端能显示出来），而不是开一个流再报错
    let model = config::resolve_with_budget(req.model_id.as_deref(), downgraded)?;
    // 会话记账放在 resolve 之后：模型没配好时直接报错，不该留下一条只有问题、
    // 永远等不到回答的会话（那种会话在列表里点开就是断片）
    let thread = teammate::begin_turn(
        req.thread_id.as_deref(),
        req.persist,
        req.connection_id.as_deref().unwrap_or_default(),
        req.database.as_deref().unwrap_or_default(),
        &question,
    );
    if let Some(thread) = &thread {
        teammate::append_message(thread, "user", &question, None);
    }
    // 同 chat：历史走双预算，流式接口一次都不该多付
    let mut messages = req.bounded_history();
    messages.push(Msg {
        role: "user".to_string(),
        content: question.clone(),
    });
    config::audit("chat/stream", &question);

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(64);
    // 记账与蒸馏都要拿会话 / 请求体的所有权：spawn_blocking 里拿不到 `&req`
    let stream_thread = thread.clone();
    let state_owned = state.clone();
    let req_owned = req.clone();
    let question_owned = question.clone();
    tokio::task::spawn_blocking(move || {
        // 首帧带会话 id：前端从 SSE 里就能拿到，不必先去读响应头
        if let Some(thread) = &stream_thread {
            let _ = tx.blocking_send(Ok(format!("data:{}\n\n", json!({ "thread": thread }))));
        }
        // 降级要说出来：否则用户只会觉得「今天它怎么不看表了」
        if downgraded {
            let _ = tx.blocking_send(Ok(format!(
                "data:{}\n\n",
                json!({ "notice": "本月 AI 用量已达预算上限，本次已降级回答（不注入表结构与记忆）" })
            )));
        }
        // 攒一份完整回答：流结束后要把它落进会话，否则「继续上次」只有提问没有回答
        let mut answer = String::new();
        let outcome = config::chat_stream(&model, &system, &messages, |delta| {
            answer.push_str(delta);
            // 分片整体过一遍 serde_json：换行/控制字符由它负责转义，
            // 手工拼字符串是最容易把 SSE 帧撑破的地方
            let frame = format!("data:{}\n\n", json!({ "d": delta }));
            let _ = tx.blocking_send(Ok(frame));
        });
        let tail = match outcome {
            // 用量单独发一帧（在 done 之前）：前端按 `data:` 帧解析，收到就挂到这条回答下。
            //上游没给 usage 时**不发这一帧** —— 前端据此显示"未提供"，而不是显示 0。
            Ok(usage) => {
                if let Some(thread) = &stream_thread {
                    teammate::append_message(thread, "assistant", &answer, usage.as_ref());
                }
                match usage {
                    Some(usage) => format!(
                        "data:{}\n\nevent:done\ndata:{{\"done\":true}}\n\n",
                        json!({ "usage": usage })
                    ),
                    None => "event:done\ndata:{\"done\":true}\n\n".to_string(),
                }
            }
            // 出错就不落 assistant 消息：半截回答落库会让「继续上次」把中断当成结论
            Err(err) => format!("data:{}\n\n", json!({ "error": err.message })),
        };
        let _ = tx.blocking_send(Ok(tail));
        // 流已收尾，蒸馏慢一点也无所谓；用 Handle 而不是 `tokio::spawn`：
        // 这里在 blocking 线程上，显式拿句柄最不容易踩「不在 runtime 里」的坑
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                teammate::memory_extract(&state_owned, &req_owned, &question_owned).await;
            });
        }
    });

    let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream;charset=UTF-8")
        .header(header::CACHE_CONTROL, "no-cache")
        // 中间有反向代理时不让它缓冲，否则流式就退化成「等完再一次性吐出」
        .header("X-Accel-Buffering", "no");
    // 响应头也带一份会话 id：fetch 拿到响应就能读，不用等第一帧
    if let Some(thread) = &thread {
        builder = builder.header("X-Dbmind-Thread", thread.as_str());
    }
    Ok(builder
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}

/// `POST /api/ai/warmup` —— 预热。
///
/// 只做「读结构 + 拼上下文」这一步（这确实是首次提问里最慢的那一段），
/// **不真的调模型**：用户还在犹豫问什么的时候就把 token 花掉，不是好买卖。
pub async fn warmup(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let Some(conn) = req.connection_id.clone().filter(|value| !value.trim().is_empty()) else {
        return Ok(Json(json!({ "success": true, "warmed": false, "reason": "未指定连接" })));
    };
    let database = req.database.clone().unwrap_or_default();
    let context = crate::api::ai::schema_context(&state, &conn, &database, &req.tables).await?;
    Ok(Json(json!({
        "success": true,
        "warmed": true,
        "tables": context.tables,
        "columns": context.columns,
        "chars": context.text.chars().count(),
    })))
}

// ------------------------------------------------------------------ SQL 生成与解释

/// `POST /api/ai/nl2sql` —— 自然语言 → SQL（**带自校验**）。
pub async fn nl2sql(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let question = req.ask_text();
    if question.is_empty() {
        return Err(XError::bad_request("请输入要用自然语言描述的需求"));
    }
    let system = "你是资深数据库工程师：把需求写成一条可直接执行的 SQL。\
                  只输出 SQL（不解释、不用代码块）；表名与字段只能用结构里真实存在的，不要臆造。";
    let user = format!("### 需求\n{question}");
    let raw = ask_inline(&state, &req, system, user, true).await?;
    let sql = extract_sql(&raw);
    if sql.is_empty() {
        return Err(XError::internal("模型没有给出 SQL"));
    }

    // 自校验：只读语句才做 EXPLAIN（写语句 EXPLAIN 会真的改数据或直接报错）
    let mut verified = false;
    let mut warning: Option<String> = None;
    if let Some(conn) = req.connection_id.clone().filter(|value| !value.trim().is_empty()) {
        let database = req.database.clone().unwrap_or_default();
        if !crate::api::ai::read_only(&sql) {
            warning = Some("生成的语句不是只读查询，未做执行计划校验".to_string());
        } else {
            match explain_sql(&state, &conn, &database, &sql).await {
                Ok(Some(_)) => verified = true,
                Ok(None) => {
                    warning = Some("当前数据库类型不支持用 EXPLAIN 做自校验，请人工确认".to_string())
                }
                Err(err) => warning = Some(format!("执行计划校验未通过：{}", err.message)),
            }
        }
    } else {
        warning = Some("未提供连接，未做执行计划校验".to_string());
    }

    Ok(Json(json!({
        "success": true,
        "sql": sql,
        "verified": verified,
        "warning": warning,
    })))
}

/// 一次 EXPLAIN 的结果。
///
/// `text` 是给人复制、也给模型看的原文；`columns` / `rows` 给界面画成表 ——
/// 只有原文的话，界面上就是一串 `Using where | 10.0 | 3 | ... | ALL` 的天书
/// （真机反馈：「执行计划看不懂是干嘛的」）。
pub struct Explain {
    pub text: String,
    pub columns: Vec<String>,
    pub rows: Vec<Value>,
}

/// 跑一次 EXPLAIN。`Ok(None)` = 这个类型没有可用的 EXPLAIN 自检。
pub async fn explain_sql(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: &str,
) -> XResult<Option<Explain>> {
    let record = crate::api::require_record(state, conn).await?;
    let kind = record.kind().key();
    let statement = match kind {
        "sqlite" => format!("explain query plan {sql}"),
        "mysql" | "mariadb" | "doris" | "postgresql" | "kingbase" | "h2" | "clickhouse" => {
            format!("explain {sql}")
        }
        // SQL Server 用 SET SHOWPLAN_*（有副作用、要成对执行），Oracle/DB2/Derby 是 PLAN_TABLE，
        // 都不是「一句 EXPLAIN」能拿到的 —— 宁可不做，也不发一条会改会话状态的语句
        _ => return Ok(None),
    };
    // 聊天里执行的语句用户可给任意 SQL（可能很重）：走独立会话键，不占元数据那条连接
    let result = crate::api::meta::run_sql_in_background(state, conn, database, statement, 200).await?;
    let shaped = crate::api::shape::query_result_json(&result);
    let columns: Vec<String> = shaped
        .get("columns")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let rows: Vec<Value> = shaped
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .take(50)
        .collect();
    // 原文按列顺序拼 `a | b | c`：与旧输出逐字一致，复制出去仍然能直接看
    let text = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|name| match row.get(name) {
                    Some(Value::Null) | None => "-".to_string(),
                    Some(Value::String(text)) => text.clone(),
                    Some(other) => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Some(Explain {
        text,
        columns,
        rows,
    }))
}

/// `POST /api/ai/explain` —— 解释 SQL。
pub async fn explain(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let sql = need_sql(&req)?;
    let context = context_text(&state, &req, false).await;
    let (content, usage) = ask_with_usage(
        &state,
        &req,
        "sql.explain.system",
        "sql.explain.user",
        &[("schemaContext", &context), ("sql", &sql)],
        true,
    )
    .await?;
    Ok(Json(json!({ "success": true, "content": content, "usage": usage })))
}

/// `POST /api/ai/optimize` —— 优化建议。
pub async fn optimize(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let sql = need_sql(&req)?;
    let context = context_text(&state, &req, false).await;
    let (content, usage) = ask_with_usage(
        &state,
        &req,
        "sql.optimize.system",
        "sql.optimize.user",
        &[("schemaContext", &context), ("sql", &sql)],
        true,
    )
    .await?;
    Ok(Json(json!({ "success": true, "content": content, "usage": usage })))
}

/// `POST /api/ai/fix` —— 按约定的两行格式解析出「原因 + 修复 SQL」。
pub async fn fix(State(state): State<AppState>, Json(req): Json<AiRequest>) -> XResult<Json<Value>> {
    let sql = need_sql(&req)?;
    let error = req.error.clone().unwrap_or_default();
    let context = context_text(&state, &req, false).await;
    let (raw, usage) = ask_with_usage(
        &state,
        &req,
        "sql.fix.system",
        "sql.fix.user",
        &[
            ("schemaContext", &context),
            ("sql", &sql),
            ("error", &error),
        ],
        true,
    )
    .await?;
    let (reason, fixed) = parse_fix(&raw);
    // 证据链：模型说"修好了"，我们就真的验一次 —— 只读语句真跑（拿真实行列），
    // 写语句只取执行计划（一行数据都不动）。失败也照原样返回，让用户看到
    // 「AI 说修好了，其实还是这个错」，而不是把报错吞掉只给一句漂亮话。
    let evidence = crate::api::ai::evidence::verify(
        &state,
        req.connection_id.as_deref().unwrap_or_default(),
        req.database.as_deref().unwrap_or_default(),
        &fixed,
    )
    .await;
    Ok(Json(json!({
        "success": true,
        "errorReason": reason,
        "sql": fixed,
        "content": raw,
        "usage": usage,
        "evidence": evidence,
    })))
}

/// 解析 `sql.fix` 约定的两行输出（模型没守格式时退化为「整段当 SQL，原因缺省」）。
fn parse_fix(text: &str) -> (String, String) {
    let mut reason = String::new();
    let mut sql = String::new();
    for line in text.lines() {
        let line = line.trim();
        for prefix in ["错误原因：", "错误原因:"] {
            if let Some(rest) = line.strip_prefix(prefix) {
                reason = rest.trim().to_string();
            }
        }
        for prefix in ["修复SQL：", "修复SQL:", "修复 SQL：", "修复 SQL:"] {
            if let Some(rest) = line.strip_prefix(prefix) {
                sql = rest.trim().to_string();
            }
        }
    }
    if sql.is_empty() {
        sql = extract_sql(text);
    }
    if reason.is_empty() {
        reason = "（模型未按约定给出错误原因）".to_string();
    }
    (reason, sql)
}

/// `POST /api/ai/rewrite` —— 语义等价改写。
pub async fn rewrite(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let sql = need_sql(&req)?;
    let goal = req
        .ask_text()
        .is_empty()
        .then(|| "在不改变语义的前提下提升可读性与性能".to_string())
        .unwrap_or_else(|| req.ask_text());
    let raw = ask(
        &state,
        &req,
        "sql.rewrite.system",
        "sql.rewrite.user",
        &[("goal", &goal), ("targetSections", ""), ("sql", &sql)],
        false,
    )
    .await?;
    Ok(Json(json!({ "success": true, "content": extract_sql(&raw) })))
}

/// `POST /api/ai/plan` —— 把指令解析成操作计划（严格 JSON）。
pub async fn plan(State(state): State<AppState>, Json(req): Json<AiRequest>) -> XResult<Json<Value>> {
    let command = req.ask_text();
    if command.is_empty() {
        return Err(XError::bad_request("请输入指令"));
    }
    let context = context_text(&state, &req, false).await;
    let context = if context.trim().is_empty() {
        String::new()
    } else {
        format!("### 数据库结构\n{context}\n")
    };
    let tables = String::new();
    let raw = ask(
        &state,
        &req,
        "command.parse.system",
        "command.parse.user",
        &[
            ("context", &context),
            ("tables", &tables),
            ("command", &command),
        ],
        true,
    )
    .await?;
    Ok(Json(json!({ "success": true, "content": extract_json(&raw) })))
}

/// 从模型输出里抠出 JSON（它常会包一层 ```json）。
fn extract_json(text: &str) -> String {
    let text = text.trim();
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
        if end > start {
            return text[start..=end].to_string();
        }
    }
    text.to_string()
}

/// 拼「表结构」段落（模板里的 `{{schemaContext}}`）。
///
/// `expand_default`：一张表都没认出来时，要不要兜底展开前几张。
/// 除了「整库数据字典」（它就是要一批表的列结构），一律 `false` —— 大库下按字母序取前几张
/// 与问题几乎必然无关，既贵又会把模型带偏。
async fn context_text(state: &AppState, req: &AiRequest, expand_default: bool) -> String {
    let Some(conn) = req.connection_id.clone().filter(|value| !value.trim().is_empty()) else {
        return String::new();
    };
    let database = req.database.clone().unwrap_or_default();
    let focus: Vec<String> = if !req.tables.is_empty() {
        req.tables.clone()
    } else {
        req.table.clone().map(|table| vec![table]).unwrap_or_default()
    };
    // 预算版上下文（同 system_with_context 的理由）：默认 12k tokens，只展开"点名 +
    // SQL 里认出来"的表。全量版在真机上曾把一次修复顶到 28 万 tokens。
    // names 一律省略：这个函数服务的正是「已经给了 SQL」的接口，表名清单纯噪声。
    let hint = format!("{} {}", req.ask_text(), req.sql_text());
    let options = if expand_default {
        // 数据字典：要一批表的列结构，预算放宽（它的产出就是字典，砍表等于砍结果）
        crate::api::ai::context::Options::bulk()
    } else {
        crate::api::ai::context::Options::omit_names()
    };
    match crate::api::ai::context::build(state, &conn, &database, &focus, &hint, options).await {
        Ok(context) => context.text,
        Err(err) => format!("（未能读取数据库结构：{}）", err.message),
    }
}

// ------------------------------------------------------------------ 其它生成类

/// `POST /api/ai/diagnose` —— 慢查询诊断（无独立提示词模板，用硬编码的）。
pub async fn diagnose(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let sql = need_sql(&req)?;
    let plan = match req.connection_id.clone().filter(|value| !value.trim().is_empty()) {
        Some(conn) => {
            let database = req.database.clone().unwrap_or_default();
            explain_sql(&state, &conn, &database, &sql)
                .await
                .ok()
                .flatten()
                .map(|explain| explain.text)
                .unwrap_or_default()
        }
        None => String::new(),
    };
    let user = format!(
        "诊断这条 SQL 慢在哪，按「原因 / 影响 / 建议」三段输出。\n\
         ### SQL\n```sql\n{sql}\n```\n\
         ### 执行计划（可能为空）\n{plan}"
    );
    let (content, usage) = ask_inline_with_usage(
        &state,
        &req,
        "你是一名资深数据库性能诊断专家。",
        user,
        true,
    )
    .await?;
    Ok(Json(json!({ "success": true, "content": content, "usage": usage })))
}

/// `POST /api/ai/translate` —— 跨方言翻译。
pub async fn translate(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let sql = need_sql(&req)?;
    let target = req
        .target_dialect
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "PostgreSQL".to_string());
    let user = format!(
        "把下面 SQL 翻译成 {target} 方言，语义保持完全一致。只输出 SQL，不解释。\n```sql\n{sql}\n```"
    );
    let content = ask_inline(
        &state,
        &req,
        "你是跨数据库方言 SQL 翻译专家。",
        user,
        false,
    )
    .await?;
    Ok(Json(json!({
        "success": true,
        "content": extract_sql(&content),
        "targetDialect": target,
    })))
}

/// `POST /api/ai/datadict` —— 数据字典（把结构写成业务可读的说明）。
pub async fn datadict(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    // 字典是唯一需要兜底展开的场景：它就是要「一批表的列结构」，一张都认不出来时取前几张
    let context = context_text(&state, &req, true).await;
    if context.trim().is_empty() {
        return Err(XError::bad_request("请先选择连接（数据字典需要读取表结构）"));
    }
    let table_count = context.matches("【表 ").count();
    let user = format!(
        "按下面的结构输出中文数据字典：每张表一个小节，列字段、类型，以及**按名称推断的业务含义**\
         （推断不出来写「未标注」，不要编）。\n\n{context}"
    );
    let (content, usage) = ask_inline_with_usage(
        &state,
        &req,
        "你是数据治理工程师，把库表结构写成业务能看懂的字典。",
        user,
        false,
    )
    .await?;
    Ok(Json(json!({
        "success": true,
        "content": content,
        "tableCount": table_count,
        "usage": usage,
    })))
}

/// `POST /api/ai/ddl` —— 按描述生成建表 DDL。
pub async fn generate_ddl(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let description = req.ask_text();
    if description.is_empty() {
        return Err(XError::bad_request("请描述要建的表"));
    }
    let user = format!(
        "按需求生成建表 DDL（含主键、必要索引与注释）。只输出 SQL，不解释。\n\
         ### 需求\n{description}"
    );
    let content = ask_inline(
        &state,
        &req,
        "你是资深数据库架构师，擅长按业务需求设计表结构。",
        user,
        true,
    )
    .await?;
    Ok(Json(json!({ "success": true, "content": extract_sql(&content) })))
}

/// `POST /api/ai/filter` —— 自然语言 → WHERE 条件。
pub async fn filter(
    State(state): State<AppState>,
    Json(req): Json<AiRequest>,
) -> XResult<Json<Value>> {
    let description = req.ask_text();
    if description.is_empty() {
        return Err(XError::bad_request("请描述筛选条件"));
    }
    let table = req.table.clone().unwrap_or_default();
    let user = format!(
        "把筛选需求转成 SQL 的 WHERE 条件（**只输出条件本身**，不含 WHERE、不带分号）；\
         字段只能用结构里真实存在的。\n### 目标表\n{table}\n### 需求\n{description}"
    );
    let content = ask_inline(
        &state,
        &req,
        "你是资深数据库工程师，只输出 SQL 条件表达式。",
        user,
        true,
    )
    .await?;
    let condition = content
        .trim()
        .trim_start_matches("where")
        .trim_start_matches("WHERE")
        .trim_end_matches(';')
        .trim()
        .trim_matches('`')
        .to_string();
    Ok(Json(json!({ "success": true, "content": condition })))
}
