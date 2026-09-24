//! AI 模型配置与调用客户端（**OpenAI 兼容**协议）。
//!
//! ## 配置存在哪、长什么样
//!
//! `<home>/ai-config.json`，与上游同构（`enabled` / `privacyMode` / `auditEnabled` / `models[]`），
//! 这样两边的设置项语义一致：
//!
//! ```json
//! { "enabled": true, "privacyMode": "allow", "auditEnabled": false,
//!   "models": [ { "id": "m1", "name": "DeepSeek", "baseUrl": "https://api.deepseek.com/v1",
//!                 "apiKey": "sk-…", "model": "deepseek-chat", "maxTokens": 0, "embedding": false } ] }
//! ```
//!
//! ## 三个决定
//!
//! 1. **密钥明文落盘，并且不假装它被加密了**。上游用 Base64+XOR 存（`enc:` 前缀）——
//!    那不是加密，只是让人误以为安全。这里老实写明：密钥保存在本机用户目录下的配置文件里，
//!    要更强的话应该接系统密钥串（Windows DPAPI / macOS Keychain），那是另一件事。
//! 2. **读取时绝不回显密钥**，只给 `hasKey`（布尔）。提交时留空 = 沿用旧值 ——
//!    界面上那个密码框是空的时候，用户的意图是「不改」，不是「清空」。
//! 3. **隐私模式真的拦**：`privacyMode=localOnly` 时，只要 baseUrl 不是本机地址就直接拒绝，
//!    并说明拦的是哪个 host。给了开关却只在文档里生效，等于没有这个功能。
//!
//! ##上游协议（三家都兼容 OpenAI 的这一套）
//!
//! - 对话：`POST {baseUrl}/chat/completions`，`Authorization: Bearer <key>`，body
//!   `{model, messages, temperature, stream, max_tokens?}`；
//! - 向量：`POST {baseUrl}/embeddings`，body `{model, input:[...]}`（**每批 10 条**）。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::error::{XError, XResult};

/// 一次 embed 请求最多带多少条文本（与上游一致：批次太大容易被上游拒）。
const EMBED_BATCH: usize = 10;
/// 单次请求超时。
const TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub max_tokens: u64,
    /// 这个模型能不能用来做向量化（`/embeddings`）
    #[serde(default)]
    pub embedding: bool,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    #[serde(default)]
    pub enabled: bool,
    /// `allow` | `localOnly`
    #[serde(default = "default_privacy")]
    pub privacy_mode: String,
    #[serde(default)]
    pub audit_enabled: bool,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

fn default_privacy() -> String {
    "allow".to_string()
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            privacy_mode: default_privacy(),
            audit_enabled: false,
            models: Vec::new(),
        }
    }
}

pub fn config_path() -> PathBuf {
    dbmind_core::paths::home_dir().join("ai-config.json")
}

pub fn load() -> AiSettings {
    match std::fs::read_to_string(config_path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => AiSettings::default(),
    }
}

pub fn save(settings: &AiSettings) -> XResult<()> {
    let path = config_path();
    dbmind_core::paths::ensure_parent(&path)?;
    let text = serde_json::to_string_pretty(settings).map_err(|e| XError::internal(e.to_string()))?;
    std::fs::write(&path, text)
        .map_err(|e| XError::internal(format!("写入 {} 失败：{e}", path.display())))
}

/// `GET /api/ai/config` —— 界面看到的样子（**密钥只给 hasKey**）。
pub fn view() -> Value {
    let settings = load();
    let models: Vec<Value> = settings
        .models
        .iter()
        .map(|model| {
            json!({
                "id": model.id,
                "name": model.name,
                "baseUrl": model.base_url,
                "apiKey": "",
                "hasKey": !model.api_key.trim().is_empty(),
                "supportEmbedding": model.embedding,
                "embedding": model.embedding,
                "model": model.model,
                "maxTokens": model.max_tokens,
            })
        })
        .collect();
    let configured = settings.models.iter().any(|model| usable(model).is_some());
    json!({
        "enabled": settings.enabled,
        "models": models,
        "privacyMode": if settings.privacy_mode.is_empty() { default_privacy() } else { settings.privacy_mode },
        "auditEnabled": settings.audit_enabled,
        "configured": configured,
        "path": config_path().display().to_string(),
    })
}

/// `POST /api/ai/config` —— 保存（apiKey 留空 = 沿用旧值）。
pub fn apply(body: &Value) -> XResult<Value> {
    let mut settings = load();
    if let Some(enabled) = body.get("enabled").and_then(Value::as_bool) {
        settings.enabled = enabled;
    }
    if let Some(mode) = body.get("privacyMode").and_then(Value::as_str) {
        settings.privacy_mode = mode.to_string();
    }
    if let Some(audit) = body.get("auditEnabled").and_then(Value::as_bool) {
        settings.audit_enabled = audit;
    }
    if let Some(models) = body.get("models").and_then(Value::as_array) {
        let old = settings.models.clone();
        let mut next: Vec<ModelEntry> = Vec::new();
        for (index, item) in models.iter().enumerate() {
            let mut id = item.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            if id.is_empty() {
                id = format!("m{}", index + 1);
            }
            let api_key = item
                .get("apiKey")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // 空 = 沿用旧值（见文件头第 2 条）
            let api_key = if api_key.trim().is_empty() {
                old.iter()
                    .find(|model| model.id == id)
                    .map(|model| model.api_key.clone())
                    .unwrap_or_default()
            } else {
                api_key
            };
            next.push(ModelEntry {
                id,
                name: item.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                base_url: item
                    .get("baseUrl")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                api_key,
                model: item.get("model").and_then(Value::as_str).unwrap_or("").to_string(),
                max_tokens: item.get("maxTokens").and_then(Value::as_u64).unwrap_or(0),
                embedding: item
                    .get("embedding")
                    .and_then(Value::as_bool)
                    .or_else(|| item.get("supportEmbedding").and_then(Value::as_bool))
                    .unwrap_or(false),
            });
        }
        settings.models = next;
    }
    save(&settings)?;
    Ok(view())
}

fn usable(model: &ModelEntry) -> Option<&ModelEntry> {
    if model.base_url.trim().is_empty() || model.model.trim().is_empty() {
        return None;
    }
    // 本地模型（Ollama 之类）常常不需要 key：只要求「配了 key」会把它们挡在门外
    Some(model)
}

fn is_local(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("localhost") || lower.contains("127.0.0.1") || lower.contains("://[::1]")
}

/// 选模型：给了 modelId 就用它，否则挑第一个可用的。
pub fn resolve(model_id: Option<&str>) -> XResult<ModelEntry> {
    let settings = load();
    if !settings.enabled {
        return Err(XError::bad_request("请先在「设置」中启用并配置 AI 服务"));
    }
    let wanted = model_id.unwrap_or("").trim();
    let model = if wanted.is_empty() || wanted == "auto" {
        settings
            .models
            .iter()
            .find(|model| usable(model).is_some())
            .cloned()
    } else {
        settings
            .models
            .iter()
            .find(|model| model.id == wanted)
            .cloned()
            .filter(|model| usable(model).is_some())
    };
    let model = model.ok_or_else(|| {
        XError::bad_request("请先在「设置」中配置 AI 模型（接口地址 / API Key / 模型名称需填写完整）")
    })?;
    if settings.privacy_mode == "localOnly" && !is_local(&model.base_url) {
        let host = model
            .base_url
            .split("//")
            .nth(1)
            .unwrap_or(&model.base_url)
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        return Err(XError::bad_request(format!(
            "当前为「仅本地模型」隐私模式，已阻止调用外部模型：{host}"
        )));
    }
    Ok(model)
}

/// 消息体（对话历史与当前提问共用）。
#[derive(Serialize, Deserialize, Clone)]
pub struct Msg {
    pub role: String,
    pub content: String,
}

fn endpoint(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    // 有人会把完整地址填进来（含 /chat/completions）；那样就别再拼一次
    if base.ends_with(path) {
        base.to_string()
    } else {
        format!("{base}{path}")
    }
}

fn describe(err: ureq::Error) -> XError {
    match err {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|value| {
                    value
                        .get("error")
                        .and_then(|error| error.get("message"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| value.get("message").and_then(Value::as_str).map(str::to_string))
                })
                .unwrap_or_else(|| body.chars().take(300).collect());
            XError::internal(format!("模型服务返回 {code}：{detail}"))
        }
        ureq::Error::Transport(err) => {
            XError::internal(format!("连不上模型服务：{err}（检查「设置」里的接口地址与网络）"))
        }
    }
}

/// 读响应体（`into_string` 给的是 `io::Error`，与 ureq 的错误不是一回事）。
fn read_body(response: ureq::Response) -> XResult<String> {
    response
        .into_string()
        .map_err(|e| XError::internal(format!("读取模型响应失败：{e}")))
}

fn messages_json(system: &str, messages: &[Msg]) -> Value {
    let mut out: Vec<Value> = Vec::new();
    let system = system.trim();
    if !system.is_empty() {
        out.push(json!({ "role": "system", "content": system }));
    }
    for message in messages {
        if message.content.trim().is_empty() {
            continue;
        }
        out.push(json!({ "role": message.role, "content": message.content }));
    }
    Value::Array(out)
}

fn request_body(model: &ModelEntry, messages: Value, stream: bool) -> Value {
    let mut body = json!({
        "model": model.model,
        "messages": messages,
        // 数据库场景要的是「稳」，不是「有创意」——温度压低是这类工具的通例
        "temperature": 0.2,
        "stream": stream,
    });
    if model.max_tokens > 0 {
        body["max_tokens"] = json!(model.max_tokens);
    }
    body
}

///上游返回的 token 用量（OpenAI 兼容的 `usage` 字段）。
#[derive(Serialize, Clone, Copy, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

/// 从上游响应（或流式分片）里取 `usage`。
///
/// **取不到就返回 `None`** —— 不估算、不按字数折算：界面上要显示的是"实际花了多少"，
/// 编一个近似值还不如不显示（那会让人误以为成本可预期）。不少兼容实现压根不回
/// `usage`，也不支持流式里的 `include_usage`。
fn parse_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?.as_object()?;
    let number = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let parsed = TokenUsage {
        prompt_tokens: number("prompt_tokens"),
        completion_tokens: number("completion_tokens"),
        total_tokens: number("total_tokens"),
    };
    // 三个字段全是 0 =上游只放了个空壳对象，当作没给
    if parsed.prompt_tokens == 0 && parsed.completion_tokens == 0 && parsed.total_tokens == 0 {
        return None;
    }
    Some(parsed)
}

/// 一次同步对话，返回完整回复。
pub fn chat(model: &ModelEntry, system: &str, messages: &[Msg]) -> XResult<String> {
    chat_with_usage(model, system, messages).map(|(content, _)| content)
}

/// 同 [`chat`]，但把 token 用量一并带出来（`None` =上游没给）。
pub fn chat_with_usage(
    model: &ModelEntry,
    system: &str,
    messages: &[Msg],
) -> XResult<(String, Option<TokenUsage>)> {
    let url = endpoint(&model.base_url, "/chat/completions");
    let body = request_body(model, messages_json(system, messages), false);
    let response = ureq::post(&url)
        .timeout(TIMEOUT)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .set("Authorization", &format!("Bearer {}", model.api_key))
        .send_string(&body.to_string())
        .map_err(describe)?;
    let text = read_body(response)?;
    let value: Value = serde_json::from_str(&text).map_err(|e| {
        XError::internal(format!("模型返回的不是 JSON：{e}（前 200 字：{}）", text.chars().take(200).collect::<String>()))
    })?;
    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if content.trim().is_empty() {
        return Err(XError::internal(format!(
            "模型没有返回内容（原始响应前 300 字：{}）",
            text.chars().take(300).collect::<String>()
        )));
    }
    let usage = parse_usage(&value);
    record_usage(model, usage);
    Ok((content, usage))
}

/// 带工具定义的一次对话：返回 assistant 的 **message 对象**（含 `content` 与 `tool_calls`）。
///
/// 与 `chat` 的三点不同，都是为智能体循环准备的：
/// 1. 消息收 `Vec<Value>` 而不是 `Msg` —— 循环里要原样回填 assistant 的 `tool_calls`
///    和 `role:"tool"` 的回执，`Msg { role, content }` 装不下。
/// 2. **不因为 `content` 为空就报错** —— 「这一轮只调用工具、不说话」是完全合法的回复。
/// 3. 返回原始 message 而不是字符串 —— 调用方要自己判断是「继续调工具」还是「给最终答案」。
pub fn chat_tools(
    model: &ModelEntry,
    system: &str,
    messages: &[Value],
    tools: &Value,
) -> XResult<Value> {
    let url = endpoint(&model.base_url, "/chat/completions");
    let mut all: Vec<Value> = Vec::new();
    let system = system.trim();
    if !system.is_empty() {
        all.push(json!({ "role": "system", "content": system }));
    }
    all.extend(messages.iter().cloned());
    let mut body = request_body(model, Value::Array(all), false);
    if !tools.is_null() {
        body["tools"] = tools.clone();
        body["tool_choice"] = json!("auto");
    }
    let response = ureq::post(&url)
        .timeout(TIMEOUT)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .set("Authorization", &format!("Bearer {}", model.api_key))
        .send_string(&body.to_string())
        .map_err(describe)?;
    let text = read_body(response)?;
    let value: Value = serde_json::from_str(&text).map_err(|e| {
        XError::internal(format!(
            "模型返回的不是 JSON：{e}（前 200 字：{}）",
            text.chars().take(200).collect::<String>()
        ))
    })?;
    let message = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .cloned();
    let Some(message) = message else {
        return Err(XError::internal(format!(
            "模型没有返回 message（原始响应前 300 字：{}）",
            text.chars().take(300).collect::<String>()
        )));
    };
    let has_content = message
        .get("content")
        .and_then(Value::as_str)
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false);
    let has_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| !calls.is_empty())
        .unwrap_or(false);
    if !has_content && !has_calls {
        return Err(XError::internal(format!(
            "模型既没有内容也没有工具调用（原始响应前 300 字：{}）",
            text.chars().take(300).collect::<String>()
        )));
    }
    // 工具循环里**每一轮**都会记一次（那是真实的模型调用次数）；token 取本轮响应的 usage
    record_usage(model, parse_usage(&value));
    Ok(message)
}

/// 流式对话：每收到一个增量就回调一次（**阻塞**，调用方负责放到 blocking 线程里）。
pub fn chat_stream(
    model: &ModelEntry,
    system: &str,
    messages: &[Msg],
    mut on_delta: impl FnMut(&str),
) -> XResult<Option<TokenUsage>> {
    let url = endpoint(&model.base_url, "/chat/completions");
    let mut body = request_body(model, messages_json(system, messages), true);
    // 让上游在**流末尾**追加一个只含 usage 的分片（OpenAI 兼容协议）。
    // 不认这个字段的服务端会忽略它 —— 那时就只有调用次数、没有 token 数（如实显示"未提供"）。
    body["stream_options"] = json!({ "include_usage": true });
    let response = ureq::post(&url)
        .timeout(TIMEOUT)
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream")
        .set("Authorization", &format!("Bearer {}", model.api_key))
        .send_string(&body.to_string())
        .map_err(describe)?;

    let reader = BufReader::new(response.into_reader());
    let mut produced = false;
    // 流末那一帧的 usage（见上面的 stream_options）
    let mut usage: Option<TokenUsage> = None;
    for line in reader.lines() {
        let line = line.map_err(|e| XError::internal(format!("读取模型流失败：{e}")))?;
        let line = line.trim();
        //上游这一跳的协议是 OpenAI 的 SSE：`data: {...}`，结束是 `data: [DONE]`
        if !line.starts_with("data:") {
            continue;
        }
        let payload = line[5..].trim();
        if payload.is_empty() {
            continue;
        }
        if payload == "[DONE]" {
            break;
        }
        let value: Value = match serde_json::from_str(payload) {
            Ok(value) => value,
            // 分片不是完整 JSON 是正常的（上游会切包），跳过而不是报错
            Err(_) => continue,
        };
        // 带 usage 的那一帧通常 choices 为空，单独取（后到的覆盖先前的，取最后一次）
        if let Some(seen) = parse_usage(&value) {
            usage = Some(seen);
        }
        if let Some(delta) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"))
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str)
        {
            if !delta.is_empty() {
                produced = true;
                on_delta(delta);
            }
        }
    }
    if !produced {
        return Err(XError::internal("模型没有返回任何内容（流式响应为空）"));
    }
    record_usage(model, usage);
    Ok(usage)
}

/// 向量化（知识库用）。返回与输入一一对应的向量。
pub fn embed(model: &ModelEntry, texts: &[String]) -> XResult<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let url = endpoint(&model.base_url, "/embeddings");
    let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(EMBED_BATCH) {
        let body = json!({ "model": model.model, "input": chunk });
        let response = ureq::post(&url)
            .timeout(TIMEOUT)
            .set("Content-Type", "application/json")
            .set("Authorization", &format!("Bearer {}", model.api_key))
            .send_string(&body.to_string())
            .map_err(describe)?;
        let text = read_body(response)?;
        let value: Value = serde_json::from_str(&text).map_err(|e| {
            XError::internal(format!("向量服务返回的不是 JSON：{e}"))
        })?;
        let data = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                XError::internal(format!(
                    "向量服务返回里没有 data 字段（原始响应前 300 字：{}）",
                    text.chars().take(300).collect::<String>()
                ))
            })?;
        for item in data {
            let vector = item
                .get("embedding")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .map(|value| value.as_f64().unwrap_or(0.0) as f32)
                        .collect::<Vec<f32>>()
                })
                .unwrap_or_default();
            out.push(vector);
        }
    }
    Ok(out)
}

/// 有没有能向量化的模型（知识库的「向量模式」要用）。
pub fn embedding_model(preferred: Option<&str>) -> Option<ModelEntry> {
    let settings = load();
    if let Some(id) = preferred.filter(|id| !id.trim().is_empty()) {
        if let Some(model) = settings
            .models
            .iter()
            .find(|model| model.id == id && usable(model).is_some())
        {
            return Some(model.clone());
        }
    }
    settings
        .models
        .iter()
        .find(|model| usable(model).is_some() && model.embedding)
        .cloned()
}

// ------------------------------------------------------------------ 用量

fn usage_path() -> PathBuf {
    dbmind_core::paths::home_dir().join("ai-usage.json")
}

/// 用量账本里「按模型」的键：优先 API 的 `model` 串（那才是真正发出去的名字），
/// 空则退回配置里的展示名，都没有就给个占位。
fn usage_model_key(model: &ModelEntry) -> String {
    let api = model.model.trim();
    if !api.is_empty() {
        return api.to_string();
    }
    let name = model.name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    "未命名模型".to_string()
}

/// 给 `target[key]` 加一个增量（不存在按 0 起算）。
fn bump(target: &mut Value, key: &str, delta: u64) {
    if delta == 0 {
        return;
    }
    let current = target.get(key).and_then(Value::as_u64).unwrap_or(0);
    target[key] = json!(current + delta);
}

/// 记一次调用。失败不影响主流程 —— 用量统计不该把一次回答搞失败。
///
/// <p>存两份，**刻意不合并**：
/// * `days`：老结构，只有每天的调用次数 —— 历史文件、旧界面都认它，不动；
/// * `models`：新结构，`模型 → 日期 → { calls, promptTokens, completionTokens, totalTokens }`。
///
/// 分开的好处是老账本**不用迁移**（缺 `models` 就当空），也不会因为多写字段把老字段写坏。
/// token 只在上游回了 `usage` 时才累加 —— 没回就只累加次数，绝不按字数估算。
fn record_usage(model: &ModelEntry, usage: Option<TokenUsage>) {
    let path = usage_path();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut doc = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or_else(|| json!({ "days": {} }));
    if !doc.get("days").is_some_and(Value::is_object) {
        doc["days"] = json!({});
    }
    if !doc.get("models").is_some_and(Value::is_object) {
        doc["models"] = json!({});
    }

    if let Some(days) = doc.get_mut("days").and_then(Value::as_object_mut) {
        let entry = days
            .entry(today.clone())
            .or_insert_with(|| json!({ "calls": 0 }));
        if !entry.is_object() {
            *entry = json!({ "calls": 0 });
        }
        bump(entry, "calls", 1);
    }

    if let Some(models) = doc.get_mut("models").and_then(Value::as_object_mut) {
        let bucket = models.entry(usage_model_key(model)).or_insert_with(|| json!({}));
        if !bucket.is_object() {
            *bucket = json!({});
        }
        let days_of_model = bucket.as_object_mut().expect("上面刚保证过是对象");
        let day = days_of_model
            .entry(today)
            .or_insert_with(|| json!({ "calls": 0 }));
        if !day.is_object() {
            *day = json!({ "calls": 0 });
        }
        bump(day, "calls", 1);
        if let Some(usage) = usage {
            bump(day, "promptTokens", usage.prompt_tokens);
            bump(day, "completionTokens", usage.completion_tokens);
            bump(day, "totalTokens", usage.total_tokens);
        }
    }

    if let Ok(text) = serde_json::to_string_pretty(&doc) {
        let _ = std::fs::write(path, text);
    }
}

/// `GET /api/ai/usage?days=30` —— 按天的调用次数。
pub fn usage(days: u64) -> Value {
    let doc = std::fs::read_to_string(usage_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or_else(|| json!({ "days": {} }));
    let empty = serde_json::Map::new();
    let all = doc.get("days").and_then(Value::as_object).unwrap_or(&empty);
    let today = chrono::Local::now().date_naive();
    let limit = days.clamp(1, 365) as i64;
    let mut series: Vec<Value> = Vec::new();
    let mut total = 0u64;
    for offset in (0..limit).rev() {
        let date = today - chrono::Duration::days(offset);
        let key = date.format("%Y-%m-%d").to_string();
        let calls = all
            .get(&key)
            .and_then(|entry| entry.get("calls"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        total += calls;
        series.push(json!({ "date": key, "calls": calls }));
    }

    // 按模型汇总（同一时间窗）：逐天累加；token 缺失的日子只算次数，不补数字。
    // 排序按总 token 降序、同 token 按次数 —— 用得多的排前面。
    let empty_models = serde_json::Map::new();
    let models_doc = doc
        .get("models")
        .and_then(Value::as_object)
        .unwrap_or(&empty_models);
    let from = today - chrono::Duration::days(limit - 1);
    let mut models: Vec<Value> = Vec::new();
    for (name, bucket) in models_doc {
        let Some(by_day) = bucket.as_object() else { continue };
        let (mut calls, mut prompt, mut completion, mut tokens) = (0u64, 0u64, 0u64, 0u64);
        for (date, entry) in by_day {
            let Ok(parsed) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
                continue;
            };
            if parsed < from {
                continue;
            }
            let number = |key: &str| entry.get(key).and_then(Value::as_u64).unwrap_or(0);
            calls += number("calls");
            prompt += number("promptTokens");
            completion += number("completionTokens");
            tokens += number("totalTokens");
        }
        if calls == 0 && tokens == 0 {
            continue;
        }
        models.push(json!({
            "model": name,
            "calls": calls,
            "promptTokens": prompt,
            "completionTokens": completion,
            "totalTokens": tokens,
        }));
    }
    models.sort_by(|a, b| {
        let key = |value: &Value| {
            (
                value["totalTokens"].as_u64().unwrap_or(0),
                value["calls"].as_u64().unwrap_or(0),
            )
        };
        key(b).cmp(&key(a))
    });

    json!({ "days": series, "total": total, "window": limit, "models": models })
}

/// 追加一条审计记录（`auditEnabled` 打开时）。
pub fn audit(kind: &str, prompt: &str) {
    let settings = load();
    if !settings.audit_enabled {
        return;
    }
    let path = dbmind_core::paths::home_dir().join("ai-audit.log");
    let line = json!({
        "time": chrono::Local::now().to_rfc3339(),
        "kind": kind,
        "prompt": prompt.chars().take(4000).collect::<String>(),
    });
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}
