//! **字段脱敏**的 HTTP 层 + 设置读写。
//!
//! 规则判定（`MaskConfig::method_for`）与算法（MD5 / SHA-256 / AES-256-GCM）都在
//! `dbmind_core::mask`：同一套规则要喂给三个出口 ——
//!
//! 1. `context::build` 里每张展开表的 **1 行样例值**（真实数据，发给大模型）；
//! 2. `agent::run` 的只读工具结果（智能体自己查回来的行，也发给大模型）；
//! 3. MCP 服务（`dbmind-mcp`）返回给外部客户端的数据。
//!
//! 三个出口共用一份判定，才不会出现「界面上配了、某一处没生效」——那种 bug 用户
//! 只会说「脱敏是假的」。界面里的结果表格由前端调同一套算法（`utils/masking.js`）。
//!
//! 这里只负责：把配置存进设置（`ai.maskRules`）、AES 密钥文本（`ai.maskKey`）、
//! 以及给设置页用的三个开关。

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::blocking;
use crate::api::error::XResult;
use crate::AppState;

// 判定与算法都在 core：MCP 服务与前端表格要用同一套，复制两份迟早漂移
pub use dbmind_core::mask::{
    aes_decrypt, key_from_text, mask_value, random_key_text, MaskChannel, MaskConfig, MaskKey,
    MaskMethod, MaskRule, KEY, KEY_AES,
};

/// 读配置。解析失败按「未启用」处理 —— 一条坏规则不该让整个 AI 面板报错。
pub async fn load(state: &AppState) -> MaskConfig {
    let engine = state.engine();
    let raw = match blocking(move || engine.get_setting(KEY)).await {
        Ok(value) => value,
        Err(_) => None,
    };
    let mut config = raw
        .as_deref()
        .and_then(|text| serde_json::from_str::<MaskConfig>(text).ok())
        .unwrap_or_default();
    // 老版本只有一个 `enabled` 开关，读回来先并进 llm，后续一律按新形状走
    config.normalize();
    config
}

/// 写配置。
pub async fn save(state: &AppState, config: &MaskConfig) -> XResult<()> {
    let text = serde_json::to_string(config)?;
    let engine = state.engine();
    blocking(move || engine.set_setting(KEY, &text)).await?;
    Ok(())
}

/// 设置里原样的密钥文本（用户改过就是他自己写的那串）。
async fn key_setting(state: &AppState) -> Option<String> {
    let engine = state.engine();
    let raw = blocking(move || engine.get_setting(KEY_AES)).await.ok().flatten()?;
    let text = raw.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// 读已经存在的那把 AES 密钥（只读，不生成）。
pub async fn read_key(state: &AppState) -> Option<MaskKey> {
    key_setting(state).await.map(|text| key_from_text(&text))
}

/// 把密钥文本写进设置（设置页里手填的也走这里，原样存）。
pub async fn save_key_text(state: &AppState, text: &str) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    let engine = state.engine();
    if let Err(error) = blocking(move || engine.set_setting(KEY_AES, &text)).await {
        tracing::warn!(?error, "AES 密钥落盘失败：本次运行内可用，重启会读到旧值");
    }
}

/// 取密钥文本；没有就生成一把默认的（base64 的 32 字节随机数）并落盘。
///
/// 密钥跟着数据目录走（存在设置里），迁移/备份会把钥匙一起带走 ——
/// 换了钥匙，之前发出去的密文就解不回来了。
pub async fn ensure_key_text(state: &AppState) -> String {
    if let Some(text) = key_setting(state).await {
        return text;
    }
    let text = random_key_text();
    save_key_text(state, &text).await;
    text
}

/// 取 32 字节密钥；没有就生成默认的那把。
pub async fn ensure_key(state: &AppState) -> MaskKey {
    key_from_text(&ensure_key_text(state).await)
}

/// 规则用得上 AES 才取密钥；用不上就给一把全 0（不落盘，也不影响其它方式）。
pub async fn load_key(state: &AppState, config: &MaskConfig) -> MaskKey {
    if !config.uses_aes() {
        return [0u8; 32];
    }
    ensure_key(state).await
}

/// `GET /api/ai/mask-rules`
pub async fn get_rules(State(state): State<AppState>) -> XResult<Json<Value>> {
    let config = load(&state).await;
    // 第一次打开设置页就备好默认密钥：界面上那一栏始终有值，用户可以照用也可以改
    let aes_key = ensure_key_text(&state).await;
    Ok(Json(json!({
        "llm": config.llm,
        "mcp": config.mcp,
        "grid": config.grid,
        "export": config.export,
        "rules": config.rules,
        "count": config.rules.len(),
        "aesKey": aes_key,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesBody {
    #[serde(default)]
    pub llm: bool,
    #[serde(default)]
    pub mcp: bool,
    #[serde(default)]
    pub grid: bool,
    #[serde(default)]
    pub export: bool,
    #[serde(default)]
    pub rules: Vec<MaskRule>,
    /// 界面上的 AES 密钥：原样存（空/缺省表示不改）
    #[serde(default)]
    pub aes_key: Option<String>,
}

/// `PUT /api/ai/mask-rules` —— 整份覆盖（规则条数不多，前端一次全量提交最不容易出错）。
pub async fn put_rules(
    State(state): State<AppState>,
    Json(body): Json<RulesBody>,
) -> XResult<Json<Value>> {
    let rules: Vec<MaskRule> = body
        .rules
        .into_iter()
        // 列名是唯一必填项：空列名的规则永远匹配不到东西，存下来只会让人以为生效了
        .filter(|rule| !rule.column.trim().is_empty())
        .map(|rule| MaskRule {
            connection_id: clean(rule.connection_id.as_deref()).map(str::to_string),
            database: clean(rule.database.as_deref()).map(str::to_string),
            schema: clean(rule.schema.as_deref()).map(str::to_string),
            table: clean(rule.table.as_deref()).map(str::to_string),
            column: rule.column.trim().to_string(),
            method: rule.method,
            note: clean(rule.note.as_deref()).map(str::to_string),
        })
        .collect();
    let config = MaskConfig {
        llm: body.llm,
        mcp: body.mcp,
        grid: body.grid,
        export: body.export,
        enabled: false,
        rules,
    };
    save(&state, &config).await?;
    // 界面上的密钥一起存：用户可以改成自己的口令，改完后续加密就用它
    if let Some(key) = body.aes_key.as_deref() {
        save_key_text(&state, key).await;
    }
    let aes_key = ensure_key_text(&state).await;
    tracing::info!(
        llm = config.llm,
        mcp = config.mcp,
        grid = config.grid,
        export = config.export,
        count = config.rules.len(),
        "字段脱敏配置已更新"
    );
    Ok(Json(json!({
        "success": true,
        "llm": config.llm,
        "mcp": config.mcp,
        "grid": config.grid,
        "export": config.export,
        "count": config.rules.len(),
        "aesKey": aes_key,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewBody {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub method: MaskMethod,
}

/// `POST /api/ai/mask/preview` —— 单个值的掩码预览（界面已不展示，保留给排查与集成）。
pub async fn preview(State(state): State<AppState>, Json(body): Json<PreviewBody>) -> Json<Value> {
    let key = if body.method.needs_key() {
        ensure_key(&state).await
    } else {
        [0u8; 32]
    };
    let aes_key = ensure_key_text(&state).await;
    Json(json!({
        "input": body.value,
        "method": body.method,
        "result": mask_value(&body.value, body.method, &key),
        "aesKey": aes_key,
    }))
}

fn clean(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}
