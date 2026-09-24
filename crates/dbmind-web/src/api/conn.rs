//! `/api/connections` —— 连接管理。
//!
//!上游的前端**只用 POST 保存**（新建与编辑都走 `POST /api/connections`，
//! body 里带 id 就是编辑），所以 POST 必须做成 upsert；PUT 保留是为了与上游的
//! 契约完全对齐（第三方脚本可能用它）。
//!
//! 这里有一条**刻意的拒绝策略**：连接弹窗里有几项内核没有对应能力
//! （SSH 隧道、与主机对不上的自定义 JDBC URL）。丢掉字段后假装保存成功是最坏的选择 ——
//! 用户会以为配好了，然后往「网络 / 权限」方向查很久。所以宁可当场报错说清原因。

use axum::extract::{Path, RawQuery, State};
use axum::Json;
use dbmind_core::{ConnectionConfig, ConnectionKind};
use serde_json::{json, Map, Value};

use crate::api::error::{XError, XResult};
use crate::api::shape;
use crate::api::{blocking, require_record, Params};
use crate::AppState;

fn text(body: &Value, key: &str) -> String {
    body.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn opt(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

///上游的类型码（`MYSQL`）→ 内核 kind。两边的类型集合是同一套，只有大小写之差。
fn kind_of_type_code(code: &str) -> XResult<ConnectionKind> {
    ConnectionKind::from_key(&code.trim().to_ascii_lowercase())
        .ok_or_else(|| XError::bad_request(format!("不支持的数据库类型：{code}")))
}

/// 库名留空时的取值。
///
/// 对「服务器上管着多个库」的类型（MySQL / PostgreSQL / SQL Server / ClickHouse…）
/// 必须保留成**空串**，不能变成 `None`：内核渲染 URL 时 `{database}` 拿到 `None` 会直接报
/// 「缺少 {database} 对应的字段」，拿到空串才能渲染出 `databaseName=`（SQL Server）
/// 或 `jdbc:mysql://host:3306/` —— 也就是「连到服务器、不指定默认库」。
/// 这正是「不填库名就列出所有有权限的库」的前提。
///
/// 反过来，Oracle / DB2 / 文件型不能这么干：它们的 `database` 是服务名或文件，空串会把 URL 改坏，
/// 所以那些类型仍然给 `None`，让内核那条「缺少字段」的明确错误照旧抛出来。
fn database_value(kind: ConnectionKind, body: &Value) -> Option<String> {
    let value = text(body, "database");
    if value.is_empty() && crate::api::dialect::Dialect::new(kind).switchable_database() {
        return Some(String::new());
    }
    opt(value)
}

///上游`ConnectionInfo`（请求体）→ 内核 `ConnectionConfig`。
pub fn config_from_body(body: &Value) -> XResult<ConnectionConfig> {
    let type_code = text(body, "type");
    if type_code.is_empty() {
        return Err(XError::bad_request("缺少连接类型 type"));
    }
    let kind = kind_of_type_code(&type_code)?;
    let name = text(body, "name");
    if name.is_empty() {
        return Err(XError::bad_request("连接名称不能为空"));
    }

    // ---- 内核没有的字段：明确拒绝，不静默丢弃 ----
    if body
        .get("sshEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(XError::bad_request(
            "SSH 隧道尚未接入（内核没有隧道层）。请使用「主机 + 端口 + 账号」直连，\
             或先自行建立隧道再把地址填成本地端口 —— 这里明确报错，而不是丢掉字段后假装保存成功",
        ));
    }
    let host = text(body, "host");
    let jdbc_url = text(body, "jdbcUrl");
    // 连接弹窗会**自动**从 host/port 生成一条 JDBC URL 放进表单，那种 URL 只是展示用，
    // 真实信息都在 host/port 里，丢掉没有损失。所以只拒绝「和 host 对不上的自定义 URL」。
    if !jdbc_url.is_empty() && !host.is_empty() && !jdbc_url.contains(&host) {
        return Err(XError::bad_request(
            "自定义 JDBC URL 尚未接入：内核按「类型 + 主机 + 端口 + 库名」自行拼 URL。\
             请改填主机与端口（URL 会随之变化），或为本项目贡献 URL 模板",
        ));
    }

    // ---- 界面偏好 + 驱动参数一并装进 extra ----
    // extra.params 会一路透传到 JDBC 宿主的 Properties（自签证书的 SQL Server 靠它：
    // trustServerCertificate=true / encrypt=false）；environment/env 是树的分组与环境角标。
    let mut extra = Map::new();
    let mut params = Map::new();
    if let Some(list) = body.get("params").and_then(Value::as_array) {
        for item in list {
            let key = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            if key.is_empty() {
                continue;
            }
            let value = item.get("value").and_then(Value::as_str).unwrap_or_default();
            params.insert(key, Value::String(value.to_string()));
        }
    }
    if !params.is_empty() {
        extra.insert("params".to_string(), Value::Object(params));
    }
    let environment = text(body, "environment");
    if !environment.is_empty() {
        extra.insert("environment".to_string(), Value::String(environment));
    }
    let env = text(body, "env");
    if !env.is_empty() {
        extra.insert("env".to_string(), Value::String(env));
    }
    // 备注：与 environment/env 同样放 `extra` 里。`extra` 的约定是「驱动只读 params，
    // 其余键无人过问」，所以界面偏好放这里安全（见 shape 模块顶部的说明）。
    let note = text(body, "note");
    if !note.is_empty() {
        extra.insert("note".to_string(), Value::String(note));
    }
    // 认证方式（SQL Server 的「SQL Server 身份验证 / Windows 身份验证」）也必须存下来。
    // 以前这里漏了：界面上选了 Windows 验证，保存后再打开又变回 SQL 验证，
    // 而宿主那边照样拿用户名口令去登 —— 用户看到的报错就是
    // 「用户 'sa' 登录失败」（明明选的是 Windows 验证）。同样放 extra 里。
    let auth_type = text(body, "authType");
    if !auth_type.is_empty() {
        extra.insert("authType".to_string(), Value::String(auth_type));
    }

    Ok(ConnectionConfig {
        name,
        kind,
        host: opt(host),
        port: body.get("port").and_then(Value::as_u64).map(|p| p as u16),
        database: database_value(kind, body),
        username: opt(text(body, "username")),
        // 空口令 ⇒ None：内核的更新语义是「缺省即保留」，这正是「留空表示不修改」要的
        password: opt(text(body, "password")),
        file_path: opt(text(body, "filePath")),
        color: opt(text(body, "color")),
        extra: if extra.is_empty() {
            None
        } else {
            Some(Value::Object(extra))
        },
    })
}

pub async fn list(State(state): State<AppState>) -> XResult<Json<Value>> {
    let engine = state.engine.clone();
    let records = blocking(move || engine.list_connections()).await?;
    // 影子连接（跨库浏览时按需生成的、指向某个库的连接）**不给用户看**：
    // 它们是实现细节，出现在树里只会让人以为「多了几条连接」。
    Ok(Json(Value::Array(
        records
            .iter()
            .filter(|record| !crate::api::scope::is_shadow(record))
            .map(shape::connection_json)
            .collect(),
    )))
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    let record = require_record(&state, &id).await?;
    Ok(Json(shape::connection_json(&record)))
}

/// 新建 / 保存（带 id 即编辑）。
pub async fn create(State(state): State<AppState>, Json(body): Json<Value>) -> XResult<Json<Value>> {
    let config = config_from_body(&body)?;
    let id = text(&body, "id");
    let engine = state.engine.clone();
    let record = blocking(move || {
        if id.is_empty() {
            return engine.add_connection(config);
        }
        match engine.update_connection(&id, config.clone()) {
            // 前端会拿「已删除的连接」的旧表单再保存一次，这时应当新建而不是报 404
            Err(err) if matches!(err.code, dbmind_core::ErrorCode::ConnNotFound) => {
                engine.add_connection(config)
            }
            other => other,
        }
    })
    .await?;
    apply_read_only(&state, &record, &body).await;
    Ok(Json(shape::connection_json(&record)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let config = config_from_body(&body)?;
    let engine = state.engine.clone();
    let record = blocking(move || engine.update_connection(&id, config)).await?;
    apply_read_only(&state, &record, &body).await;
    Ok(Json(shape::connection_json(&record)))
}

/// 「只读连接」开关的落库。
///
/// 它是**连接记录**上的属性（`ConnectionRecord.read_only`），不在 `ConnectionConfig` 里，
/// 所以不能跟着 `add_connection / update_connection` 一起写 —— 走内核专门的 `set_read_only`：
/// 它会**同时**更新存储与内存策略，避免「库里改了、策略没改」的漂移（内核注释里写明这点）。
/// 开着只读的连接，写操作会被内核直接拦下（报「安全拦截」），不是界面上的软提示。
///
/// **缺省即保留**：只有请求里**带了** `readOnly` 才动它。导入的连接、老版本前端都不传这个字段，
/// 不这样写会把它们静默改成可写 —— 那正好是最危险的方向。
async fn apply_read_only(
    state: &AppState,
    record: &dbmind_core::ConnectionRecord,
    body: &Value,
) {
    let Some(flag) = body.get("readOnly").and_then(Value::as_bool) else {
        return;
    };
    if flag == record.read_only {
        return;
    }
    let engine = state.engine.clone();
    let id = record.id.clone();
    if let Err(err) = blocking(move || engine.set_read_only(&id, flag)).await {
        tracing::warn!(error = %err.message, "保存只读标记失败");
    }
}

pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    let engine = state.engine.clone();
    let target = id.clone();
    let removed = blocking(move || engine.remove_connection(&target)).await?;
    // 级联删掉这条连接的影子，否则它们会变成树上看不见、却一直占着会话的垃圾
    let shadows = crate::api::scope::delete_shadows(&state, &id).await.unwrap_or(0);
    Ok(Json(json!({ "success": true, "removed": removed, "shadowsRemoved": shadows })))
}

/// 复制连接。
///
/// 与「前端读出来再 Post 一遍」相比，这里**服务端直接复制**，因此**口令也一起带过去**
/// （口令只写不读，前端从来拿不到它）。上游的界面语义就是「复制成一个能直接用的连接」。
pub async fn copy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let src = require_record(&state, &id).await?;
    let name = params
        .get("name")
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("{} 副本", src.config.name));
    let mut config = src.config.clone();
    config.name = name;
    let engine = state.engine.clone();
    let record = blocking(move || engine.add_connection(config)).await?;
    Ok(Json(shape::connection_json(&record)))
}
