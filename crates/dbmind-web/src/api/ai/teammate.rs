//! **AI 同事**：让助手从「你问它答」变成「有记忆、有值班、交付变更单、每步可审计」。
//!
//! ## 四根柱子
//!
//! | 能力 | 落点 | 一句话 |
//! |---|---|---|
//! | 记忆 | `ai_memories` + [`memory_context`] | 说过的口径、习惯不用重复交代 |
//! | 变更单 | `ai_changes` + 批准/执行/撤销 | AI 只提案，**执行必须人点** |
//! | 值班 | `ai_tasks` + 后台 [`spawn_scheduler`] | 到点自己巡检，有异常才吵你 |
//! | 审计 | `ai_events` | 谁让它干的、它干了什么、撤没撤 |
//!
//! ## 一条不能越的线
//!
//! 产品决定：**AI 永不直接执行写操作**。它能做的最后一件事是产出一张 `pending` 的变更单；
//! 执行走 [`change_apply`]，且还要总开关 `safety.aiWriteEnabled`（默认关）打开才允许。
//! 这样即使模型被提示词注入骗了，也顶多"提了一个坏单子"，而不是"改了一张表"。
//!
//! ## 为什么规则巡检不调模型
//!
//! 值班的价值在于**不吵人**。结构性问题（无主键、无索引）用规则就能判出来，一次 0 token；
//! 交给模型只会让它每天编一段"总体来看比较健康"的废话，还要按字收费。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{Duration as ChronoDuration, Local};
use serde_json::{json, Value};

use dbmind_core::{AiChangeRow, AiJobRow, AiMemoryRow, AiTaskRow, AiThreadRow, Store};

use crate::api::error::{XError, XResult};
use crate::api::meta::run_sql_in_write;
use crate::api::require_record;
use crate::AppState;

/// 记忆注入上限：记忆是"提示"，不是"文档"，超过这个数就该去人工清理了。
const MEMORY_INJECT: usize = 12;

fn store() -> XResult<Arc<Store>> {
    dbmind_core::global_store().ok_or_else(|| XError::internal("主库未就绪，AI 同事的持久化不可用"))
}

fn text(body: &Value, key: &str) -> String {
    body.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn need(body: &Value, key: &str) -> XResult<String> {
    let value = text(body, key);
    if value.is_empty() {
        return Err(XError::bad_request(format!("缺少 {key} 参数")));
    }
    Ok(value)
}

fn now() -> String {
    Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// 本地 id（`thr_20261031T101502123`）。会话只需要"同一台机器上唯一"，
/// 用时间戳+毫秒足够，不必引入 uuid 依赖。
fn new_id(prefix: &str) -> String {
    format!(
        "{prefix}_{}",
        Local::now().format("%Y%m%dT%H%M%S%3f")
    )
}

/// 下次该跑的时间（`every_minutes` 之后）。时间串统一由这里和 `now_iso` 生成，
/// 所以 `ai_task_due` 里可以直接按字符串比较。
fn next_run(every_minutes: u64) -> String {
    (Local::now() + ChronoDuration::minutes(every_minutes.max(1) as i64))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

// ------------------------------------------------------------------ 会话

fn thread_json(row: &AiThreadRow) -> Value {
    json!({
        "id": row.id,
        "title": row.title,
        "connectionId": row.connection_id,
        "database": row.database_name,
        "createdAt": row.created_at,
        "updatedAt": row.updated_at,
    })
}

/// `GET /api/ai/threads` —— 会话列表（最近 50 个）。
pub async fn thread_list(State(_state): State<AppState>) -> XResult<Json<Value>> {
    let rows = store()?.ai_thread_list(50).unwrap_or_default();
    Ok(Json(json!({
        "success": true,
        "threads": rows.iter().map(thread_json).collect::<Vec<_>>(),
    })))
}

/// `POST /api/ai/threads` —— 新建会话（已有同名 id 就更新标题与连接）。
pub async fn thread_create(State(_state): State<AppState>, Json(body): Json<Value>) -> XResult<Json<Value>> {
    let id = text(&body, "id");
    let title = text(&body, "title");
    let row = AiThreadRow {
        id: if id.is_empty() { new_id("thr") } else { id },
        title: if title.is_empty() { "新的对话".to_string() } else { title },
        connection_id: text(&body, "connectionId"),
        database_name: text(&body, "database"),
        created_at: String::new(),
        updated_at: String::new(),
    };
    store()?.ai_thread_save(&row)?;
    Ok(Json(json!({ "success": true, "id": row.id })))
}

/// `GET /api/ai/threads/{id}` —— 会话详情（含消息，按时间正序）。
pub async fn thread_get(State(_state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    let store = store()?;
    let Some(thread) = store.ai_thread_get(&id)? else {
        return Err(XError::bad_request("会话不存在"));
    };
    let messages: Vec<Value> = store
        .ai_message_list(&id, 200)
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            json!({
                "role": row.role,
                "content": row.content,
                "usage": if row.usage.is_empty() { Value::Null } else {
                    serde_json::from_str::<Value>(&row.usage).unwrap_or(Value::Null)
                },
                "createdAt": row.created_at,
            })
        })
        .collect();
    Ok(Json(json!({
        "success": true,
        "thread": thread_json(&thread),
        "messages": messages,
    })))
}

/// `DELETE /api/ai/threads/{id}` —— 删会话（连带消息）。
pub async fn thread_delete(State(_state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    store()?.ai_thread_delete(&id)?;
    Ok(Json(json!({ "success": true })))
}

// ------------------------------------------------------------------ 对话落库
//
// 为什么放在这里而不是 chat.rs：三件事（建会话 / 追加消息 / 蒸馏记忆）都是「记账」，
// 与「怎么问模型」无关。放在一处才能保证两条入口（普通 / 流式）行为完全一致 ——
// 以前只在普通那边加，流式就会悄悄少一半记录。

/// 会话标题：拿第一句问题截断。列表里要能认出「这是哪次对话」，
/// 用 id 或时间戳都认不出来。
fn thread_title(question: &str) -> String {
    let flat = question.split_whitespace().collect::<Vec<_>>().join(" ");
    let title: String = flat.chars().take(30).collect();
    if title.trim().is_empty() {
        "新的对话".to_string()
    } else {
        title
    }
}

/// 这一轮该写进哪个会话。
///
/// - 传了 `thread_id` ⇒ 用它（顺手刷新 `updated_at`，列表按它排序）
/// - 没传但 `persist` ⇒ 新建一个（标题取问题前 30 字）
/// - 都没有 ⇒ `None`，调用方就不落库（一次性接口不产生垃圾会话）
///
/// **失败一律返回 `None` 并告警**：记账失败不能让用户看不到回答。
pub fn begin_turn(
    thread_id: Option<&str>,
    persist: bool,
    connection_id: &str,
    database: &str,
    question: &str,
) -> Option<String> {
    let store = dbmind_core::global_store()?;
    if let Some(id) = thread_id.map(str::trim).filter(|id| !id.is_empty()) {
        // 会话可能已经被用户删了：touch 影响 0 行也不影响这一轮继续跑
        if let Err(err) = store.ai_thread_touch(id) {
            tracing::warn!("刷新 AI 会话时间失败：{}", err);
        }
        return Some(id.to_string());
    }
    if !persist {
        return None;
    }
    let id = new_id("thr");
    let row = AiThreadRow {
        id: id.clone(),
        title: thread_title(question),
        connection_id: connection_id.to_string(),
        database_name: database.to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    };
    match store.ai_thread_save(&row) {
        Ok(()) => Some(id),
        Err(err) => {
            tracing::warn!("新建 AI 会话失败（这一轮照常回答）：{}", err);
            None
        }
    }
}

/// 往会话里追加一条消息。`usage` 只在 assistant 那一侧有值（前端要显示 token 消耗）。
pub fn append_message(
    thread_id: &str,
    role: &str,
    content: &str,
    usage: Option<&crate::api::ai::config::TokenUsage>,
) {
    let Some(store) = dbmind_core::global_store() else {
        return;
    };
    // 空内容不记：模型抽风返回空串时，库里多一条空消息只会让「继续上次」看起来断片
    if content.trim().is_empty() {
        return;
    }
    let usage = usage
        .map(|one| serde_json::to_string(one).unwrap_or_default())
        .unwrap_or_default();
    if let Err(err) = store.ai_message_add(thread_id, role, content, &usage) {
        tracing::warn!("写入 AI 会话消息失败（不影响回答）：{}", err);
    }
}

// ------------------------------------------------------------------ 记忆

fn memory_json(row: &AiMemoryRow) -> Value {
    json!({
        "id": row.id,
        "scope": row.scope,
        "connectionId": row.connection_id,
        "database": row.database_name,
        "fact": row.fact,
        "source": row.source,
        "hits": row.hits,
        "createdAt": row.created_at,
    })
}

/// `GET /api/ai/memory?connectionId=&limit=` —— 记忆列表。
pub async fn memory_list(State(_state): State<AppState>, axum::extract::Query(query): axum::extract::Query<Value>) -> XResult<Json<Value>> {
    let conn = query.get("connectionId").and_then(Value::as_str).unwrap_or("");
    let limit = query
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(200)
        .clamp(1, 500) as usize;
    let rows = store()?
        .ai_memory_list(Some(conn), limit)
        .unwrap_or_default();
    Ok(Json(json!({
        "success": true,
        "memories": rows.iter().map(memory_json).collect::<Vec<_>>(),
    })))
}

/// `POST /api/ai/memory` —— 手工记一条（`fact` 必填）。
///
/// 手工记的一律标 `source=user`：模型推断的那些是"它以为"，人工写的是"说好了"，
/// 界面上要能分开 —— 用户可以一键清掉所有 AI 推断而不动自己写的。
pub async fn memory_add(State(_state): State<AppState>, Json(body): Json<Value>) -> XResult<Json<Value>> {
    let fact = need(&body, "fact")?;
    let scope = {
        let scope = text(&body, "scope");
        if scope.is_empty() { "connection".to_string() } else { scope }
    };
    let row = AiMemoryRow {
        id: String::new(),
        scope,
        connection_id: text(&body, "connectionId"),
        database_name: text(&body, "database"),
        fact,
        source: "user".to_string(),
        hits: 0,
        created_at: String::new(),
    };
    let id = store()?.ai_memory_add(&row)?;
    Ok(Json(json!({ "success": true, "id": id })))
}

/// `DELETE /api/ai/memory/{id}`
pub async fn memory_delete(State(_state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    store()?.ai_memory_delete(&id)?;
    Ok(Json(json!({ "success": true })))
}

/// 给提示词用的记忆段落。没有记忆就返回空串（调用方据此决定要不要加空行）。
pub fn memory_context(connection_id: &str, database: &str) -> String {
    let Some(store) = dbmind_core::global_store() else {
        return String::new();
    };
    let rows = store
        .ai_memory_list(Some(connection_id), MEMORY_INJECT)
        .unwrap_or_default();
    let rows: Vec<&AiMemoryRow> = rows
        .iter()
        .filter(|row| {
            // 库级记忆只在对应库说话时注入；连接级与全局的不挑库
            row.database_name.is_empty()
                || database.is_empty()
                || row.database_name.eq_ignore_ascii_case(database)
        })
        .take(MEMORY_INJECT)
        .collect();
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::from("【已确认的约定（用户说过或采纳过，按其执行）】\n");
    for row in rows {
        let mark = if row.source == "user" { "" } else { "（推断）" };
        out.push_str(&format!("- {}{mark}\n", row.fact));
    }
    out
}

/// 从一轮对话里提炼记忆（模型推断，异步、可失败）。
///
/// 只在用户话里出现"记住 / 口径 / 单位 / 规则 / 以后 / 别名"这类信号词时才跑：
/// 每轮都跑一次提炼，等于给每次问答加一次调用，成本与延迟都不划算。
pub async fn memory_extract(state: &AppState, req: &crate::api::ai::AiRequest, user_text: &str) {
    const SIGNALS: [&str; 8] = ["记住", "口径", "单位", "规则", "以后", "统一", "别名", "别用"];
    if user_text.chars().count() < 6 {
        return;
    }
    if !SIGNALS.iter().any(|signal| user_text.contains(signal)) {
        return;
    }
    let prompt = format!(
        "从下面这句话里提炼需要长期记住的事实（口径、单位、别名、禁用项、命名习惯）。\
         最多 3 条；每条一句话、可独立理解：必须写清「什么是什么」，不许用「这个 / 它 / 上述」\
         之类指代（下一个人看不到原话）；没有就给空数组。只输出 JSON：{{\"facts\":[\"...\"]}}\n\n{user_text}"
    );
    let system = "你是数据库团队的记录员，只输出 JSON。";
    let Ok(model) = crate::api::ai::config::resolve(req.model_id.as_deref()) else {
        return;
    };
    let messages = vec![crate::api::ai::config::Msg {
        role: "user".to_string(),
        content: prompt,
    }];
    // 提炼这一次不需要用量（不进界面统计），所以走不带 usage 的 `chat`
    let Ok(raw) = crate::api::ai::run_blocking(move || {
        crate::api::ai::config::chat(&model, system, &messages)
    })
    .await
    else {
        return;
    };
    // 两个都得有：只找到左括号或只找到右括号都不是一段完整 JSON
    let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) else {
        return;
    };
    if end <= start {
        return;
    }
    let Ok(parsed) = serde_json::from_str::<Value>(&raw[start..=end]) else {
        return;
    };
    let Some(store) = dbmind_core::global_store() else {
        return;
    };
    for fact in parsed
        .get("facts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .take(3)
    {
        let fact = fact.as_str().unwrap_or_default().trim().to_string();
        if fact.chars().count() < 4 || fact.chars().count() > 120 {
            continue;
        }
        let _ = store.ai_memory_add(&AiMemoryRow {
            id: String::new(),
            scope: "connection".to_string(),
            connection_id: req.connection_id.clone().unwrap_or_default(),
            database_name: req.database.clone().unwrap_or_default(),
            fact,
            source: "ai".to_string(),
            hits: 0,
            created_at: String::new(),
        });
    }
    let _ = state;
}

// ------------------------------------------------------------------ 变更单

fn change_json(row: &AiChangeRow) -> Value {
    json!({
        "id": row.id,
        "jobId": row.job_id,
        "threadId": row.thread_id,
        "connectionId": row.connection_id,
        "database": row.database_name,
        "title": row.title,
        "reason": row.reason,
        "sql": row.sql,
        "undoSql": row.undo_sql,
        "risk": row.risk,
        "status": row.status,
        "approvedBy": row.approved_by,
        "approvedAt": row.approved_at,
        "appliedAt": row.applied_at,
        "error": row.error,
        "createdAt": row.created_at,
        "updatedAt": row.updated_at,
    })
}

/// `GET /api/ai/changes?status=` —— 变更单列表（默认全部，最近 100 张）。
pub async fn change_list(
    State(_state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<Value>,
) -> XResult<Json<Value>> {
    let status = query.get("status").and_then(Value::as_str).unwrap_or("");
    let rows = store()?.ai_change_list(Some(status), 100).unwrap_or_default();
    Ok(Json(json!({
        "success": true,
        "changes": rows.iter().map(change_json).collect::<Vec<_>>(),
    })))
}

/// 双人复核开关（默认关）：打开后「批准」与「二次确认」必须是两个人。
const TWO_PERSON_KEY: &str = "ai.approval.twoPerson";

/// 从请求里取「谁在操作」。桌面端没有登录体系，默认记 `user`；
/// 前端可以做一个小输入框把名字带上 —— 审计流水里「谁批的」比「有人批了」有用得多。
fn approver_of(query: &Value) -> String {
    query
        .get("approver")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("user")
        .to_string()
}

/// 批准人链（`rick → lily`）：两次确认要落在同一行里，才判得出「是不是同一个人」。
fn approvers_of(existing: &str) -> Vec<String> {
    existing
        .split('→')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

fn chain_push(existing: &str, who: &str) -> String {
    let mut parts = approvers_of(existing);
    if parts.iter().all(|part| part != who) {
        parts.push(who.to_string());
    }
    parts.join(" → ")
}

/// 建一张待确认的变更单（**AI 能做的最后一件事**）。
///
/// 撤销语句与风险等级在这里一次算好：用户点开单子要先看到「能撤回吗」「有多危险」，
/// 而不是等执行完才知道撤不了。
///
/// 端点和智能体工具共用这一份 —— 两条路各算一次风险，迟早会出现
/// 「智能体提的高危单子没有撤销语句」这种自相矛盾。
pub async fn create_change(
    state: &AppState,
    conn: &str,
    database: &str,
    thread_id: &str,
    job_id: &str,
    title: &str,
    reason: &str,
    sql: &str,
) -> XResult<(String, String, Option<String>)> {
    let kind = require_record(state, conn).await?.kind().key().to_string();
    let undo = crate::api::ai::action::undo_for(&kind, sql);
    let (risk, _items) = crate::api::ai::action::risk_of(sql);
    let title = if title.trim().is_empty() {
        "AI 建议的变更".to_string()
    } else {
        title.trim().to_string()
    };
    let row = AiChangeRow {
        id: String::new(),
        job_id: job_id.to_string(),
        thread_id: thread_id.to_string(),
        connection_id: conn.to_string(),
        database_name: database.to_string(),
        title: title.clone(),
        reason: reason.to_string(),
        undo_sql: undo.clone().unwrap_or_default(),
        risk: risk.to_string(),
        sql: sql.to_string(),
        status: "pending".to_string(),
        ..AiChangeRow::default()
    };
    let store = store()?;
    let id = store.ai_change_add(&row)?;
    let _ = store.ai_event_add(thread_id, &id, "ai", "propose", &title);
    Ok((id, risk.to_string(), undo))
}

/// `POST /api/ai/changes` —— 建一张变更单。
pub async fn change_create(State(state): State<AppState>, Json(body): Json<Value>) -> XResult<Json<Value>> {
    let sql = need(&body, "sql")?;
    let conn = need(&body, "connectionId")?;
    let (id, risk, undo) = create_change(
        &state,
        &conn,
        &text(&body, "database"),
        &text(&body, "threadId"),
        &text(&body, "jobId"),
        &text(&body, "title"),
        &text(&body, "reason"),
        &sql,
    )
    .await?;
    Ok(Json(json!({
        "success": true,
        "id": id,
        "undoSql": undo,
        "risk": risk,
    })))
}

/// `POST /api/ai/changes/{id}/approve?approver=名字` —— 人工批准（还不执行）。
pub async fn change_approve(
    State(_state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<Value>,
) -> XResult<Json<Value>> {
    let store = store()?;
    let Some(change) = store.ai_change_get(&id)? else {
        return Err(XError::bad_request("变更单不存在"));
    };
    if change.status != "pending" {
        return Err(XError::bad_request(format!("这张单子当前是 {}，只能批准 pending 的", change.status)));
    }
    let who = approver_of(&query);
    store.ai_change_set_status(&id, "approved", &chain_push(&change.approved_by, &who), "")?;
    let _ = store.ai_event_add(&change.thread_id, &id, &who, "approve", &change.title);
    Ok(Json(json!({
        "success": true,
        "status": "approved",
        "next": if change.risk.eq_ignore_ascii_case("high") { "confirm" } else { "apply" },
    })))
}

/// `POST /api/ai/changes/{id}/confirm?approver=名字` —— 高危变更的**二次确认**。
///
/// 为什么单独一个端点而不是「approve 两次」：界面上这是两个不同的按钮
/// （「批准」和「确认执行高危变更」），而且打开双人复核后，这两次必须是两个人。
pub async fn change_confirm(
    State(_state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<Value>,
) -> XResult<Json<Value>> {
    let store = store()?;
    let Some(change) = store.ai_change_get(&id)? else {
        return Err(XError::bad_request("变更单不存在"));
    };
    if change.status != "approved" {
        return Err(XError::bad_request(format!(
            "这张单子当前是 {}：先批准，再二次确认",
            change.status
        )));
    }
    let who = approver_of(&query);
    let two_person = store
        .get_setting(TWO_PERSON_KEY)
        .ok()
        .flatten()
        .map(|value| value.trim() == "true")
        .unwrap_or(false);
    if two_person && approvers_of(&change.approved_by).iter().any(|part| *part == who) {
        return Err(XError::bad_request(format!(
            "双人复核已开启：批准人（{}）不能自己确认，请换一个人确认",
            change.approved_by
        )));
    }
    store.ai_change_set_status(&id, "confirmed", &chain_push(&change.approved_by, &who), "")?;
    let _ = store.ai_event_add(&change.thread_id, &id, &who, "confirm", &change.title);
    Ok(Json(json!({ "success": true, "status": "confirmed", "approvers": chain_push(&change.approved_by, &who) })))
}

/// `POST /api/ai/changes/{id}/reject` —— 人工拒绝。
pub async fn change_reject(
    State(_state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<Value>,
) -> XResult<Json<Value>> {
    let store = store()?;
    let Some(change) = store.ai_change_get(&id)? else {
        return Err(XError::bad_request("变更单不存在"));
    };
    let who = approver_of(&query);
    store.ai_change_set_status(&id, "rejected", &change.approved_by, "")?;
    let _ = store.ai_event_add(&change.thread_id, &id, &who, "reject", &change.title);
    Ok(Json(json!({ "success": true, "status": "rejected" })))
}

/// `POST /api/ai/changes/{id}/apply` —— 执行。**三道闸门缺一不可**：
///
/// 1. 状态：普通变更要 `approved`，**高危变更要 `confirmed`**（多一次人工动作）
/// 2. 总开关 `safety.aiWriteEnabled` 打开（用户明确允许 AI 相关写操作）
/// 3. SQL 非空
///
/// 任何一层不满足都直接拒绝，不「尽力而为」。
pub async fn change_apply(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<Value>,
) -> XResult<Json<Value>> {
    let store = store()?;
    let Some(change) = store.ai_change_get(&id)? else {
        return Err(XError::bad_request("变更单不存在"));
    };
    let high = change.risk.eq_ignore_ascii_case("high");
    let ready = if high {
        change.status == "confirmed"
    } else {
        change.status == "approved"
    };
    if !ready {
        return Err(XError::bad_request(if high {
            format!(
                "这是高危变更（{}），当前是 {}：需要「批准 → 二次确认」之后才能执行",
                change.risk, change.status
            )
        } else {
            format!(
                "这张单子当前是 {}：先批准再执行（AI 不会自己动手）",
                change.status
            )
        }));
    }
    if !store.ai_write_allowed() {
        return Err(XError::bad_request(
            "总开关 safety.aiWriteEnabled 未开启：设置 → 安全里打开后才能执行变更单",
        ));
    }
    let who = approver_of(&query);
    let started = std::time::Instant::now();
    match run_sql_in_write(&state, &change.connection_id, &change.database_name, change.sql.clone(), 0).await {
        Ok(result) => {
            let elapsed = started.elapsed().as_millis() as u64;
            let affected = result.affected_rows.map(|rows| rows as i64).unwrap_or(-1);
            store.ai_change_set_status(&id, "applied", &change.approved_by, "")?;
            let _ = store.ai_event_add(
                &change.thread_id,
                &id,
                &who,
                "apply",
                &format!("{}（影响 {affected} 行，{elapsed}ms）", change.title),
            );
            Ok(Json(json!({
                "success": true,
                "status": "applied",
                "affectedRows": affected,
                "elapsedMs": elapsed,
                "undoSql": change.undo_sql,
            })))
        }
        Err(err) => {
            // 失败也要落状态：不然这张单子会永远停在 approved，用户以为还没跑过
            store.ai_change_set_status(&id, "failed", &change.approved_by, &err.message)?;
            let _ = store.ai_event_add(&change.thread_id, &id, &who, "apply-failed", &err.message);
            Err(err)
        }
    }
}

/// `POST /api/ai/changes/{id}/undo` —— 撤销（执行反向语句）。
pub async fn change_undo(State(state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    let store = store()?;
    let Some(change) = store.ai_change_get(&id)? else {
        return Err(XError::bad_request("变更单不存在"));
    };
    if change.status != "applied" {
        return Err(XError::bad_request(format!("这张单子当前是 {}，只有已执行的才能撤销", change.status)));
    }
    if change.undo_sql.trim().is_empty() {
        return Err(XError::bad_request("这个动作没有可靠的反向语句，无法撤销 —— 请从备份恢复"));
    }
    if !store.ai_write_allowed() {
        return Err(XError::bad_request("总开关 safety.aiWriteEnabled 未开启"));
    }
    match run_sql_in_write(&state, &change.connection_id, &change.database_name, change.undo_sql.clone(), 0).await {
        Ok(_) => {
            store.ai_change_set_status(&id, "undone", "user", "")?;
            let _ = store.ai_event_add(&change.thread_id, &id, "user", "undo", &change.undo_sql);
            Ok(Json(json!({ "success": true, "status": "undone", "undoSql": change.undo_sql })))
        }
        Err(err) => {
            let _ = store.ai_event_add(&change.thread_id, &id, "user", "undo-failed", &err.message);
            Err(err)
        }
    }
}

// ------------------------------------------------------------------ 值班任务

fn task_json(row: &AiTaskRow) -> Value {
    json!({
        "id": row.id,
        "title": row.title,
        "prompt": row.prompt,
        "connectionId": row.connection_id,
        "database": row.database_name,
        "everyMinutes": row.every_minutes,
        "enabled": row.enabled,
        "lastRunAt": row.last_run_at,
        "nextRunAt": row.next_run_at,
        "lastStatus": row.last_status,
        "createdAt": row.created_at,
    })
}

/// `GET /api/ai/tasks`
pub async fn task_list(State(_state): State<AppState>) -> XResult<Json<Value>> {
    let rows = store()?.ai_task_list().unwrap_or_default();
    Ok(Json(json!({
        "success": true,
        "tasks": rows.iter().map(task_json).collect::<Vec<_>>(),
    })))
}

/// `POST /api/ai/tasks` —— 新建/更新一条值班任务。
///
/// `prompt` 留空 = 每天跑一次**规则巡检**（0 token）；填了 = 到点让模型按这句话出一份摘要。
pub async fn task_save(State(_state): State<AppState>, Json(body): Json<Value>) -> XResult<Json<Value>> {
    let every = body
        .get("everyMinutes")
        .and_then(Value::as_u64)
        .unwrap_or(1440)
        .clamp(5, 60 * 24 * 30);
    let row = AiTaskRow {
        id: text(&body, "id"),
        title: {
            let title = text(&body, "title");
            if title.is_empty() { "每日结构巡检".to_string() } else { title }
        },
        prompt: text(&body, "prompt"),
        connection_id: need(&body, "connectionId")?,
        database_name: text(&body, "database"),
        every_minutes: every,
        enabled: body.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        last_run_at: None,
        next_run_at: next_run(if text(&body, "runNow").is_empty() { every } else { 0 }),
        last_status: String::new(),
        created_at: String::new(),
    };
    let id = store()?.ai_task_save(&row)?;
    Ok(Json(json!({ "success": true, "id": id })))
}

/// `DELETE /api/ai/tasks/{id}`
pub async fn task_delete(State(_state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    store()?.ai_task_delete(&id)?;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/ai/tasks/{id}/run` —— 立即跑一次（界面上的「现在就检查」）。
pub async fn task_run(State(state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    let store = store()?;
    let Some(task) = store.ai_task_list()?.into_iter().find(|row| row.id == id) else {
        return Err(XError::bad_request("任务不存在"));
    };
    let job = run_task(&state, &task).await?;
    store.ai_task_mark_run(&id, "ok", &next_run(task.every_minutes))?;
    Ok(Json(json!({ "success": true, "job": job })))
}

/// 后台调度：每 60 秒看一眼有没有到点的任务。
///
/// 为什么是轮询而不是 cron 表达式：桌面应用可能整夜没开，醒来时补跑一次就够 ——
/// 用 60 秒粒度的轮询既简单又天然做到「错过的不补、到点的立刻跑」。
pub fn spawn_scheduler(state: AppState) {
    tokio::spawn(async move {
        // 启动先等一会儿：让连接与宿主先就绪，别一开机就跟用户抢 Java 宿主
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        tracing::info!("AI 值班调度器已启动（每 60 秒检查一次到点任务）");
        loop {
            if let Err(err) = scheduler_tick(&state).await {
                tracing::warn!("AI 值班任务调度失败：{}", err.message);
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

/// 一轮调度（也供测试直接调用，避免等 60 秒）。
pub async fn scheduler_tick(state: &AppState) -> XResult<()> {
    let store = store()?;
    let due = store.ai_task_due(&now(), 3).unwrap_or_default();
    for task in due {
        let status = match run_task(state, &task).await {
            Ok(_) => "ok".to_string(),
            Err(err) => format!("failed: {}", err.message),
        };
        let _ = store.ai_task_mark_run(&task.id, &status, &next_run(task.every_minutes));
    }
    Ok(())
}

/// 跑一条任务，产出 job（就是界面「待办」里的一张卡片）。
async fn run_task(state: &AppState, task: &AiTaskRow) -> XResult<Value> {
    let store = store()?;
    let mut steps: Vec<Value> = Vec::new();
    let started = std::time::Instant::now();

    let (status, result) = if task.prompt.trim().is_empty() {
        // 规则巡检：0 token，只看结构性问题
        match patrol_rules(state, &task.connection_id, &task.database_name).await {
            Ok((summary, detail)) => {
                let issues = detail["issues"].as_array().map(|list| list.len()).unwrap_or(0);
                steps.push(json!({
                    "step": "规则巡检",
                    "tables": detail["scanned"],
                    "issues": issues,
                }));
                // 有问题 ⇒ `alert`（待办里排最前，用户才看得见）；没事 ⇒ `done`（不打扰）。
                // 以前两个分支都写 `done`，「有异常才吵你」等于没做。
                let status = if issues == 0 { "done" } else { "alert" };
                (status, summary)
            }
            Err(err) => {
                steps.push(json!({ "step": "规则巡检失败", "error": err.message }));
                ("failed", format!("巡检没跑成：{}", err.message))
            }
        }
    } else {
        match ask_task(state, task).await {
            Ok((answer, usage)) => {
                steps.push(json!({ "step": "模型摘要", "usage": usage }));
                ("done", answer)
            }
            Err(err) => {
                steps.push(json!({ "step": "模型摘要失败", "error": err.message }));
                ("failed", format!("这次没跑成：{}", err.message))
            }
        }
    };

    steps.push(json!({ "step": "完成", "elapsedMs": started.elapsed().as_millis() as u64 }));
    let job = AiJobRow {
        id: String::new(),
        title: task.title.clone(),
        goal: task.prompt.clone(),
        connection_id: task.connection_id.clone(),
        database_name: task.database_name.clone(),
        kind: "patrol".to_string(),
        status: status.to_string(),
        steps: serde_json::to_string(&steps).unwrap_or_else(|_| "[]".to_string()),
        result,
        created_at: String::new(),
        updated_at: String::new(),
    };
    let id = store.ai_job_add(&job)?;
    let _ = store.ai_event_add("", "", "ai", "task-run", &format!("{} → {}", task.title, status));
    Ok(json!({
        "id": id,
        "title": job.title,
        "status": job.status,
        "result": job.result,
        "steps": steps,
    }))
}

/// 规则巡检：**复用** `tools::patrol_scan`。
///
/// 以前这里另写了一份「只看主键」的版本，于是出现「界面巡检报 12 个问题、
/// 值班同事说没问题」这种自相矛盾。同一套规则只有一份实现才不会漂。
async fn patrol_rules(state: &AppState, conn: &str, database: &str) -> XResult<(String, Value)> {
    let scan = crate::api::ai::tools::patrol_scan(state, conn, database).await?;
    let items = scan.get("items").cloned().unwrap_or_else(|| json!([]));
    let summary_obj = scan.get("summary").cloned().unwrap_or_else(|| json!({}));
    let scanned = summary_obj.get("scanned").and_then(Value::as_u64).unwrap_or(0);
    let tables = summary_obj.get("tables").and_then(Value::as_u64).unwrap_or(0);
    let list = items.as_array().cloned().unwrap_or_default();

    let mut summary = String::new();
    if list.is_empty() {
        summary.push_str(&format!(
            "巡检了 {scanned} 张表（共 {tables} 张），没有发现结构性问题。\n"
        ));
    } else {
        summary.push_str(&format!(
            "巡检了 {scanned} 张表（共 {tables} 张），{} 张有问题：\n",
            list.len()
        ));
        for item in &list {
            let table = item["table"].as_str().unwrap_or("-");
            let detail = item["issues"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("；");
            summary.push_str(&format!("- {table}：{detail}\n"));
        }
        summary.push_str("\n这些只是建议：可以右键表 →「索引建议」试建，或让我生成变更单由你确认。\n");
    }
    Ok((
        summary,
        json!({ "tables": tables, "scanned": scanned, "issues": list }),
    ))
}

/// 自定义任务：把 prompt 当问题问一次（带表结构 + 记忆）。
async fn ask_task(state: &AppState, task: &AiTaskRow) -> XResult<(String, Value)> {
    use crate::api::ai::config::{chat_with_usage, resolve, Msg};
    use crate::api::ai::{system_with_context, AiRequest};

    let req = AiRequest {
        connection_id: Some(task.connection_id.clone()),
        database: Some(task.database_name.clone()),
        prompt: Some(task.prompt.clone()),
        ..AiRequest::default()
    };
    let model = resolve(req.model_id.as_deref())?;
    let base = "你是值班的数据库同事。根据当前库的真实结构回答问题，结论要能直接执行或直接判断；\
                没数据支撑就说不确定，不要编。";
    let system = system_with_context(state, &req, base).await;
    let messages = vec![Msg {
        role: "user".to_string(),
        content: task.prompt.clone(),
    }];
    let (answer, usage) = crate::api::ai::run_blocking(move || chat_with_usage(&model, &system, &messages)).await?;
    Ok((answer, usage.map(|one| json!(one)).unwrap_or(Value::Null)))
}

// ------------------------------------------------------------------ 待办 / 审计

fn job_json(row: &AiJobRow) -> Value {
    json!({
        "id": row.id,
        "title": row.title,
        "goal": row.goal,
        "connectionId": row.connection_id,
        "database": row.database_name,
        "kind": row.kind,
        "status": row.status,
        "steps": serde_json::from_str::<Value>(&row.steps).unwrap_or(json!([])),
        "result": row.result,
        "createdAt": row.created_at,
        "updatedAt": row.updated_at,
    })
}

/// `GET /api/ai/jobs?limit=` —— 待办列表（最近的活）。
pub async fn job_list(
    State(_state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<Value>,
) -> XResult<Json<Value>> {
    let limit = query.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 200) as usize;
    let rows = store()?.ai_job_list(limit).unwrap_or_default();
    Ok(Json(json!({
        "success": true,
        "jobs": rows.iter().map(job_json).collect::<Vec<_>>(),
    })))
}

/// `GET /api/ai/jobs/{id}`
pub async fn job_get(State(_state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    let Some(job) = store()?.ai_job_get(&id)? else {
        return Err(XError::bad_request("任务不存在"));
    };
    Ok(Json(json!({ "success": true, "job": job_json(&job) })))
}

/// `POST /api/ai/jobs/{id}/dismiss` —— 待办「忽略」。
///
/// 为什么不删：忽略是「我看过了，这轮不用管」，记录留着 —— 下次巡检发现同样问题
/// 仍会再报（这正是值班该做的），而删掉就把「曾经提醒过」的证据一起丢了。
pub async fn job_dismiss(State(_state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    let store = store()?;
    let Some(job) = store.ai_job_get(&id)? else {
        return Err(XError::bad_request("待办不存在"));
    };
    store.ai_job_update(&id, "dismissed", &job.steps, &job.result)?;
    let _ = store.ai_event_add("", "", "user", "job-dismiss", &job.title);
    Ok(Json(json!({ "success": true, "status": "dismissed" })))
}

/// `GET /api/ai/events?limit=` —— 审计流水。
pub async fn event_list(
    State(_state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<Value>,
) -> XResult<Json<Value>> {
    let limit = query.get("limit").and_then(Value::as_u64).unwrap_or(100).clamp(1, 500) as usize;
    let rows = store()?.ai_event_list(limit).unwrap_or_default();
    Ok(Json(json!({
        "success": true,
        "events": rows.iter().map(|row| json!({
            "id": row.id,
            "time": row.time,
            "threadId": row.thread_id,
            "changeId": row.change_id,
            "actor": row.actor,
            "action": row.action,
            "detail": row.detail,
        })).collect::<Vec<_>>(),
    })))
}

/// CSV 单元格：一律加引号，内部引号翻倍。
///
/// 必须包：`detail` 里经常有逗号、分号、甚至换行（错误信息），不包会直接把整张表错位。
fn csv_cell(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

/// `GET /api/ai/events/export` —— 审计流水导出成 CSV（Excel 双击能开）。
///
/// 为什么要有这个：出问题时用户问的是「谁在什么时候让 AI 干了什么」，
/// 一屏一屏翻接口不现实；一份能发给别人的流水才有对账价值。
pub async fn event_export(State(_state): State<AppState>) -> XResult<Response> {
    let rows = store()?.ai_event_list(5000).unwrap_or_default();
    // 前置 BOM：Excel 靠它认出 UTF-8，否则中文全变乱码
    let mut csv = String::from("\u{feff}时间,操作人,动作,会话,变更单,详情\n");
    for row in rows {
        csv.push_str(&format!(
            "{},{},{},{},{},{}\n",
            csv_cell(&row.time),
            csv_cell(&row.actor),
            csv_cell(&row.action),
            csv_cell(&row.thread_id),
            csv_cell(&row.change_id),
            csv_cell(&row.detail)
        ));
    }
    let name = format!("ai-audit-{}.csv", Local::now().format("%Y%m%d%H%M%S"));
    Ok(Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "text/csv; charset=UTF-8")
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .body(axum::body::Body::from(csv))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}

// ------------------------------------------------------------------ 预算 / 反馈 / 概览

/// 预算键：本月 token 上限（0 = 不限制）。
const BUDGET_KEY: &str = "ai.budget.monthlyTokens";

fn month_start() -> String {
    Local::now().format("%Y-%m-01").to_string()
}

fn budget_of(store: &Store) -> u64 {
    store
        .get_setting(BUDGET_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// 预算状态：超了就该降级（少塞上下文 / 换便宜模型），别等月底账单来了才发现。
pub fn budget_state(store: &Store) -> Value {
    let limit = budget_of(store);
    let used = store.ai_tokens_since(&month_start()).unwrap_or(0);
    json!({
        "monthlyTokens": limit,
        "usedThisMonth": used,
        "over": limit > 0 && used >= limit,
        "ratio": if limit > 0 { (used as f64 / limit as f64).min(1.0) } else { 0.0 },
        "since": month_start(),
    })
}

/// `GET /api/ai/budget`
pub async fn budget_get(State(_state): State<AppState>) -> XResult<Json<Value>> {
    let store = store()?;
    Ok(Json(json!({ "success": true, "budget": budget_state(&store) })))
}

/// `POST /api/ai/budget` —— 设置本月上限（`monthlyTokens: 0` = 不限制）。
pub async fn budget_set(State(_state): State<AppState>, Json(body): Json<Value>) -> XResult<Json<Value>> {
    let store = store()?;
    let limit = body.get("monthlyTokens").and_then(Value::as_u64).unwrap_or(0);
    store.set_setting(BUDGET_KEY, &limit.to_string())?;
    Ok(Json(json!({ "success": true, "budget": budget_state(&store) })))
}

/// `POST /api/ai/feedback` —— 一键反馈。
///
/// `down` 是"这条错了"：把用户给的纠正写进记忆（下次不再犯）；
/// `up` 只是记一笔，不改任何状态 —— 免得模型把自己的猜测越记越牢。
pub async fn feedback(State(_state): State<AppState>, Json(body): Json<Value>) -> XResult<Json<Value>> {
    let verdict = text(&body, "verdict");
    let note = text(&body, "note");
    let store = store()?;
    let mut saved = Value::Null;
    if verdict == "down" && !note.is_empty() {
        let id = store.ai_memory_add(&AiMemoryRow {
            id: String::new(),
            scope: "connection".to_string(),
            connection_id: text(&body, "connectionId"),
            database_name: text(&body, "database"),
            fact: format!("纠正：{note}"),
            source: "user".to_string(),
            hits: 0,
            created_at: String::new(),
        })?;
        saved = json!(id);
    }
    let _ = store.ai_event_add(
        &text(&body, "threadId"),
        "",
        "user",
        if verdict == "down" { "feedback-down" } else { "feedback-up" },
        &note,
    );
    Ok(Json(json!({ "success": true, "memoryId": saved })))
}

/// `GET /api/ai/teammate` —— 面板顶部要的几个数字（一次拿全，省三次往返）。
pub async fn overview(State(_state): State<AppState>) -> XResult<Json<Value>> {
    let store = store()?;
    let changes = store.ai_change_list(Some("pending"), 100).unwrap_or_default();
    let approved = store.ai_change_list(Some("approved"), 100).unwrap_or_default();
    let jobs = store.ai_job_list(20).unwrap_or_default();
    let memories = store.ai_memory_list(None, 500).unwrap_or_default();
    let tasks = store.ai_task_list().unwrap_or_default();
    Ok(Json(json!({
        "success": true,
        "pendingChanges": changes.len(),
        "approvedChanges": approved.len(),
        "jobs": jobs.len(),
        "memories": memories.len(),
        "tasks": tasks.len(),
        "writeAllowed": store.ai_write_allowed(),
        "budget": budget_state(&store),
        "lastJob": jobs.first().map(job_json),
    })))
}
