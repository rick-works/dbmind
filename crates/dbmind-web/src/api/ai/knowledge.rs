//! **团队知识**（`/api/ai/knowledge`，5 个端点）：术语表 + 采纳示例。
//!
//! 这两样东西是「让 AI 写对 SQL」最便宜的手段，比调大模型、加长提示词都有效：
//!
//! - **术语表**：把业务黑话映射到真实的库表字段（「大客户」= `customers.level >= 3`）。
//!   模型不知道你们内部怎么称呼，但用户提问时用的就是内部叫法；
//! - **采纳示例**：用户点过「采纳」的问答对（问法 + 最终 SQL）。下次遇到相近问法，
//!   直接把这几条塞进上下文，比让模型从零猜字段名可靠得多。
//!
//! 存主库 `~/.dbmind/dbmind.db` 的 ai_glossary + ai_examples 两张表，
//! 容量上限 200 条术语 / 300 条示例（与上游一致）：
//! 这不是数据库表越大越好，是一份**给提示词用的速查表**，堆太多只会挤占上下文。
//!
//! 示例带 `hits` 计数：被召回次数多的排前面 —— 越常用的问法越该优先命中。

use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::ai::kb::KbConfig;
use crate::api::error::{XError, XResult};
use crate::api::tasks;

/// 术语表 / 示例条数的上限。
const MAX_GLOSSARY: usize = 200;
const MAX_EXAMPLES: usize = 300;

/// 读团队知识。没装全局库（单测、纯内核形态）→ 空。
///
/// 返回值刻意保持原来那份 json 的形状：`{ "glossary": [...], "examples": [...] }`。
/// 上层的增删改逻辑与键名一字未变 —— 这也是这次迁移能做到"只换两个函数"的原因。
pub(crate) fn load() -> Value {
    let Some(store) = dbmind_core::global_store() else {
        return json!({ "glossary": [], "examples": [] });
    };
    let glossary: Vec<Value> = store
        .ai_glossary()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            json!({
                "id": row.id,
                "term": row.term,
                "definition": row.definition,
                "mapping": row.mapping,
                "connectionId": row.connection_id,
            })
        })
        .collect();
    let examples: Vec<Value> = store
        .ai_examples()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            json!({
                "id": row.id,
                "question": row.question,
                "sql": row.sql,
                "connectionId": row.connection_id,
                "database": row.database_name,
                "hits": row.hits,
                "ts": row.ts,
            })
        })
        .collect();
    json!({ "glossary": glossary, "examples": examples })
}

/// 写回团队知识（整体替换，两张表一个事务）。
///
/// 为什么整体替换：上层就是"读出来 → 改数组 → 写回去"的用法；而这张速查表
/// 有硬上限（200 / 300 条），整体换的代价可以忽略，换来的是**不会出现半截状态**。
pub(crate) fn save(doc: &Value) -> XResult<()> {
    let Some(store) = dbmind_core::global_store() else {
        return Err(XError::internal("主库未就绪，团队知识无法保存"));
    };
    let glossary: Vec<dbmind_core::AiGlossaryRow> = array_of(doc, "glossary")
        .iter()
        .map(|item| dbmind_core::AiGlossaryRow {
            id: text_of(item, "id"),
            term: text_of(item, "term"),
            definition: text_of(item, "definition"),
            mapping: text_of(item, "mapping"),
            connection_id: text_of(item, "connectionId"),
        })
        .collect();
    let examples: Vec<dbmind_core::AiExampleRow> = array_of(doc, "examples")
        .iter()
        .map(|item| dbmind_core::AiExampleRow {
            id: text_of(item, "id"),
            question: text_of(item, "question"),
            sql: text_of(item, "sql"),
            connection_id: text_of(item, "connectionId"),
            database_name: text_of(item, "database"),
            hits: item.get("hits").and_then(Value::as_u64).unwrap_or(0),
            ts: text_of(item, "ts"),
        })
        .collect();
    store
        .replace_ai_knowledge(&glossary, &examples)
        .map_err(|e| XError::internal(format!("写入主库失败：{e}")))
}

/// 取一个字符串字段：缺失或类型不对都当空串（与老文件里 `unwrap_or_default()` 行为一致）。
fn text_of(item: &Value, key: &str) -> String {
    item.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn array_of<'a>(doc: &'a Value, key: &str) -> Vec<Value> {
    doc.get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeReq {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    term: Option<String>,
    #[serde(default)]
    definition: Option<String>,
    #[serde(default)]
    mapping: Option<String>,
    #[serde(default)]
    connection_id: Option<String>,
    #[serde(default)]
    database: Option<String>,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    sql: Option<String>,
}

/// `GET /api/ai/knowledge` —— 术语表 + 采纳示例。
///
/// 函数名不叫 `get`：那会和 `axum::routing::get` 撞名（同一个值命名空间），
/// 结果是路由注册那一行报一个看起来毫不相干的错。
pub async fn list_all() -> XResult<Json<Value>> {
    let doc = load();
    Ok(Json(json!({
        "glossary": array_of(&doc, "glossary"),
        "examples": array_of(&doc, "examples"),
        "limits": { "glossary": MAX_GLOSSARY, "examples": MAX_EXAMPLES },
    })))
}

/// `POST /api/ai/knowledge/glossary` —— 新增术语（同词覆盖）。
pub async fn add_glossary(Json(req): Json<KnowledgeReq>) -> XResult<Json<Value>> {
    let term = req
        .term
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("请填写术语"))?;
    let mut doc = load();
    let mut list = array_of(&doc, "glossary");
    // 同一个词只留一条：留着旧的不会「更完整」，只会让提示词里出现互相矛盾的两种解释
    list.retain(|item| {
        item.get("term")
            .and_then(Value::as_str)
            .map(|known| !known.eq_ignore_ascii_case(term.trim()))
            .unwrap_or(true)
    });
    if list.len() >= MAX_GLOSSARY {
        return Err(XError::bad_request(format!(
            "术语表最多 {MAX_GLOSSARY} 条，请先删掉一些不再需要的"
        )));
    }
    let item = json!({
        "id": format!("g_{}", tasks::stamp().replace('_', "")),
        "term": term.trim(),
        "definition": req.definition.clone().unwrap_or_default(),
        "mapping": req.mapping.clone().unwrap_or_default(),
        "connectionId": req.connection_id.clone().unwrap_or_default(),
    });
    list.push(item.clone());
    doc["glossary"] = json!(list);
    save(&doc)?;
    Ok(Json(json!({ "success": true, "item": item })))
}

/// `POST /api/ai/knowledge/glossary/delete`。
pub async fn delete_glossary(Json(req): Json<KnowledgeReq>) -> XResult<Json<Value>> {
    let id = req
        .id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("缺少 id 参数"))?;
    let mut doc = load();
    let mut list = array_of(&doc, "glossary");
    let before = list.len();
    list.retain(|item| item.get("id").and_then(Value::as_str) != Some(id.as_str()));
    if list.len() == before {
        return Err(XError::bad_request("没有找到这条术语"));
    }
    doc["glossary"] = json!(list);
    save(&doc)?;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/ai/knowledge/example` —— 采纳一条问答示例。
pub async fn add_example(Json(req): Json<KnowledgeReq>) -> XResult<Json<Value>> {
    let question = req
        .question
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("请填写问法"))?;
    let sql = req
        .sql
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("请填写示例 SQL"))?;
    let mut doc = load();
    let mut list = array_of(&doc, "examples");
    list.retain(|item| {
        item.get("question")
            .and_then(Value::as_str)
            .map(|known| known.trim() != question.trim())
            .unwrap_or(true)
    });
    if list.len() >= MAX_EXAMPLES {
        return Err(XError::bad_request(format!(
            "采纳示例最多 {MAX_EXAMPLES} 条，请先删掉一些不再需要的"
        )));
    }
    let item = json!({
        "id": format!("e_{}", tasks::stamp().replace('_', "")),
        "question": question.trim(),
        "sql": sql.trim(),
        "connectionId": req.connection_id.clone().unwrap_or_default(),
        "database": req.database.clone().unwrap_or_default(),
        "hits": 0,
        "ts": chrono::Local::now().to_rfc3339(),
    });
    list.push(item.clone());
    doc["examples"] = json!(list);
    save(&doc)?;
    Ok(Json(json!({ "success": true, "item": item })))
}

/// `POST /api/ai/knowledge/example/delete`。
pub async fn delete_example(Json(req): Json<KnowledgeReq>) -> XResult<Json<Value>> {
    let id = req
        .id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("缺少 id 参数"))?;
    let mut doc = load();
    let mut list = array_of(&doc, "examples");
    let before = list.len();
    list.retain(|item| item.get("id").and_then(Value::as_str) != Some(id.as_str()));
    if list.len() == before {
        return Err(XError::bad_request("没有找到这条示例"));
    }
    doc["examples"] = json!(list);
    save(&doc)?;
    Ok(Json(json!({ "success": true })))
}

/// 给提示词用的段落：术语表 + 采纳示例（按 `connectionId` 过滤，命中次数多的在前）。
pub fn context_for(connection_id: Option<&str>) -> String {
    let doc = load();
    let filter = |value: &Value| -> bool {
        match connection_id {
            None => true,
            Some(conn) => value
                .get("connectionId")
                .and_then(Value::as_str)
                .map(|known| known.is_empty() || known == conn)
                .unwrap_or(true),
        }
    };
    let mut text = String::new();
    let glossary: Vec<Value> = array_of(&doc, "glossary")
        .into_iter()
        .filter(&filter)
        .collect();
    if !glossary.is_empty() {
        text.push_str("【团队术语（用户提问时用的内部叫法）】\n");
        for item in glossary.iter().take(40) {
            text.push_str(&format!(
                "- {}：{}{}\n",
                item.get("term").and_then(Value::as_str).unwrap_or(""),
                item.get("definition").and_then(Value::as_str).unwrap_or(""),
                item
                    .get("mapping")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(|value| format!("（对应 {value}）"))
                    .unwrap_or_default()
            ));
        }
    }
    let mut examples: Vec<Value> = array_of(&doc, "examples")
        .into_iter()
        .filter(&filter)
        .collect();
    examples.sort_by_key(|item| {
        std::cmp::Reverse(item.get("hits").and_then(Value::as_i64).unwrap_or(0))
    });
    if !examples.is_empty() {
        text.push_str("【团队采纳过的写法（优先沿用）】\n");
        for item in examples.iter().take(10) {
            text.push_str(&format!(
                "问：{}\nSQL：{}\n",
                item.get("question").and_then(Value::as_str).unwrap_or(""),
                item.get("sql").and_then(Value::as_str).unwrap_or("")
            ));
        }
    }
    text
}

/// 记一次「示例被召回」，用于让常用问法排到前面。
pub fn bump_hits(ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    let mut doc = load();
    let mut list = array_of(&doc, "examples");
    for item in list.iter_mut() {
        let matched = item
            .get("id")
            .and_then(Value::as_str)
            .map(|id| ids.iter().any(|wanted| wanted == id))
            .unwrap_or(false);
        if matched {
            let hits = item.get("hits").and_then(Value::as_i64).unwrap_or(0);
            item["hits"] = json!(hits + 1);
        }
    }
    doc["examples"] = json!(list);
    let _ = save(&doc);
}

/// 知识库召回用的默认配置（团队示例不参与向量化，纯关键词就够）。
pub fn default_config() -> KbConfig {
    KbConfig::default()
}

/// 路由。
pub fn routes() -> axum::Router<crate::AppState> {
    axum::Router::new()
        .route("/api/ai/knowledge", get(list_all))
        .route("/api/ai/knowledge/glossary", post(add_glossary))
        .route("/api/ai/knowledge/glossary/delete", post(delete_glossary))
        .route("/api/ai/knowledge/example", post(add_example))
        .route("/api/ai/knowledge/example/delete", post(delete_example))
}
