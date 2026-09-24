//! `/api/drivers`、`/api/settings` —— 系统级端点。
//!
//! 三条与上游语义对齐的取舍：
//!
//! 1. **驱动状态按大写类型码作键**。`ConnectionDialog` 读的是 `driverStatus[form.type]`，
//!    而 `form.type` 是 `MYSQL` 这种大写码；给内核的小写 key 会让「驱动已就绪」角标永远不亮。
//! 2. **只通告前端类型注册表里存在的类型**（16 个，见 `dialect::exposed_type_codes`）。
//!    内核还支持 DuckDB，但前端没有 `types/duckdb.js`，通告出去只会多出一张没有 Logo、
//!    没有默认端口、引用符也不对的卡片。
//! 3. **授权状态恒为「已授权」**。本项目是完整版，没有试用期与激活码这一层；
//!    如实回答「不锁定」，而不是把门禁做成永远弹窗。

use axum::extract::State;
use axum::Json;
use dbmind_core::ErrorCode;
use serde_json::{json, Value};

use crate::api::dialect;
use crate::api::error::{XError, XResult};
use crate::api::driver::{expected_jars, jar_names};
use crate::api::{blocking, Params};
use crate::AppState;

/// 驱动下载镜像（`api::driver` 下载时要读它换仓库根，故为 pub）。
pub const KEY_DRIVER_MIRROR: &str = "driver.mirror";
const KEY_ALLOW_LEGACY_TLS: &str = "jdbc.allowLegacyTls";

fn category_of(descriptor: &dbmind_core::TypeDescriptor) -> &'static str {
    match descriptor.protocol {
        dbmind_core::RuntimeProtocol::Mongodb
        | dbmind_core::RuntimeProtocol::Redis
        | dbmind_core::RuntimeProtocol::Elasticsearch => "NOSQL",
        _ if descriptor.local_file => "RELATIONAL_FILE",
        _ => "RELATIONAL",
    }
}

/// 驱动类型清单（「新建数据源」弹窗与类型注册表都用它）。
pub async fn driver_types(State(state): State<AppState>) -> XResult<Json<Value>> {
    let exposed = dialect::exposed_type_codes();
    let types: Vec<Value> = state
        .engine
        .types()
        .iter()
        .filter(|t| exposed.contains(&t.key.to_ascii_uppercase().as_str()))
        .map(|t| {
            json!({
                "code": t.key.to_ascii_uppercase(),
                "key": t.key,
                "label": t.label,
                "category": category_of(t),
                "defaultPort": t.default_port,
                // native = 内核自带（无需下载驱动）；agent = 需要那个 JVM 驱动包
                "builtin": matches!(t.runtime_mode, dbmind_core::RuntimeMode::Native),
                "driverClass": t.kind.jdbc_driver_class(),
                "dialect": t.dialect,
                // 内核的额外信号原样带上，界面想用就有（不影响上面这些字段）
                "protocol": t.protocol.as_str(),
                "agentKey": t.agent_key,
                "implemented": t.kind.implemented(),
            })
        })
        .collect();
    Ok(Json(Value::Array(types)))
}

/// 驱动就绪状态：`{ [TYPE_CODE]: {code, builtin, ready, installed, agentKey, host, hostReady} }`。
///
/// 就绪 = 内核自带（native）**或**（该类型的驱动包已安装 且 它所属的宿主进程能起来）。
pub async fn driver_status(State(state): State<AppState>) -> XResult<Json<Value>> {
    let exposed = dialect::exposed_type_codes();
    let types = state.engine.types();
    let engine = state.engine.clone();
    let report = blocking(move || Ok(engine.driver_report())).await?;

    let mut out = serde_json::Map::new();
    for t in types.iter() {
        let code = t.key.to_ascii_uppercase();
        if !exposed.contains(&code.as_str()) {
            continue;
        }
        let native = matches!(t.runtime_mode, dbmind_core::RuntimeMode::Native);
        let entry = report.entries.iter().find(|e| e.kind == t.kind);
        let installed = entry.map(|e| e.installed).unwrap_or(false);
        let host_id = entry.and_then(|e| e.host.clone());
        let host = host_id
            .as_ref()
            .and_then(|id| report.agents.iter().find(|a| &a.id == id));
        let host_ready = host.map(|a| a.ready).unwrap_or(true);
        let ready = native || (installed && host_ready);
        // 驱动文件层面的清单：设置页「驱动下载」要显示「需要哪些 jar / 现有哪些 / 还缺哪些 / 放在哪」。
        // 都是本机目录列举（每类型一个驱动目录），开销极小。
        let expected = expected_jars(t.kind);
        let jars = t.agent_key.as_deref().map(jar_names).unwrap_or_default();
        let missing: Vec<String> = expected
            .iter()
            .filter(|name| !jars.contains(name))
            .cloned()
            .collect();
        let driver_dir = t
            .agent_key
            .as_deref()
            .map(|key| dbmind_core::driver_dir(key).display().to_string());
        out.insert(
            code.clone(),
            json!({
                "code": code,
                "builtin": native,
                "ready": ready,
                "installed": installed,
                "agentKey": t.agent_key,
                "host": host_id,
                "hostReady": host_ready,
                "label": t.label,
                // 驱动文件清单（设置页用来渲染列表、缺件提示与「打开驱动目录」）
                "artifacts": expected,
                "jars": jars,
                "missing": missing,
                "driverDir": driver_dir,
                // 未就绪时把「缺什么」直接给出来，别让用户对着灰按钮猜
                "reason": if ready { Value::Null } else {
                    host.and_then(|a| a.reason.clone()).map(Value::String).unwrap_or(Value::Null)
                },
            }),
        );
    }
    Ok(Json(Value::Object(out)))
}

fn paths_json(store_path: &str) -> Value {
    let data_dir = std::path::Path::new(store_path)
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| store_path.to_string());
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    json!({
        "os": if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" },
        "osLabel": if cfg!(windows) { "Windows" } else if cfg!(target_os = "macos") { "macOS" } else { "Linux" },
        "homeDir": home,
        "dataDir": data_dir,
        "defaultDataDir": data_dir,
        "driverDir": format!("{data_dir}/drivers"),
        "defaultDriverDir": format!("{data_dir}/drivers"),
        // 必须报**真实**的导出目录：以前这里写的是 `{dataDir}/work`，而导出其实落在
        // `<home>/exports` —— 于是设置页的「打开目录」打开的是一个空文件夹。
        "exportDir": crate::api::tasks::export_dir().display().to_string(),
        // 是否被用户改过（空值时前端显示占位符"默认"，而不是把默认路径当作用户设置）
        "exportDirConfigured": crate::api::tasks::configured_export_dir().is_some(),
    })
}

pub async fn paths_get(State(state): State<AppState>) -> XResult<Json<Value>> {
    let engine = state.engine.clone();
    let summary = blocking(move || Ok(engine.runtime_summary())).await?;
    Ok(Json(paths_json(&summary.store_path)))
}

/// 迁移数据目录需要「先关库、搬文件、再开库」，内核没有这套编排，如实拒绝。
pub async fn paths_put(Json(body): Json<Value>) -> XResult<Json<Value>> {
    // 导出目录是**可以直接改**的（其余路径不行：dataDir 要搬迁、driverDir 要重装驱动）。
    // 以前这个接口对任何请求都回 NOT_IMPLEMENTED，设置页里那一行只能是只读的。
    if let Some(dir) = body.get("exportDir").and_then(Value::as_str) {
        let applied = crate::api::tasks::set_export_dir(dir)
            .map_err(|e| XError::new(axum::http::StatusCode::BAD_REQUEST, e))?;
        return Ok(Json(json!({
            "success": true,
            "exportDir": applied.display().to_string(),
        })));
    }
    let requested = body
        .get("dataDir")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    Err(XError::new(
        axum::http::StatusCode::NOT_IMPLEMENTED,
        if requested.is_empty() {
            "迁移存储目录尚未接入".to_string()
        } else {
            format!("迁移存储目录到「{requested}」尚未接入：需要先关闭当前元数据库再搬迁（Phase 2）")
        },
    ))
}

/// 在系统文件管理器里打开目录（设置页的「打开目录」）。
///
/// 注意参数顺序：`Json` 会消费请求体，必须是最后一个提取器（axum 的硬性要求）。
pub async fn open_dir(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let requested = body
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let target = if requested.is_empty() {
        let engine = state.engine.clone();
        let summary = blocking(move || Ok(engine.runtime_summary())).await?;
        std::path::Path::new(&summary.store_path)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or(summary.store_path)
    } else {
        requested
    };
    if !std::path::Path::new(&target).exists() {
        // 目录不存在时先建出来：用户点的是「打开导出目录」，而导出还没发生过
        std::fs::create_dir_all(&target)?;
    }
    #[cfg(windows)]
    {
        std::process::Command::new("e上游r")
            .arg(&target)
            .spawn()
            .map_err(|e| XError::internal(format!("打开目录失败：{e}")))?;
    }
    #[cfg(not(windows))]
    {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        std::process::Command::new(opener)
            .arg(&target)
            .spawn()
            .map_err(|e| XError::internal(format!("打开目录失败：{e}")))?;
    }
    Ok(Json(json!({ "success": true, "path": target })))
}

pub async fn mirror_get(State(state): State<AppState>) -> XResult<Json<Value>> {
    let engine = state.engine.clone();
    let value = blocking(move || engine.get_setting(KEY_DRIVER_MIRROR)).await?;
    Ok(Json(json!({ "mirror": value.unwrap_or_default() })))
}

pub async fn mirror_put(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let mirror = body
        .get("mirror")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let engine = state.engine.clone();
    let saved = mirror.clone();
    blocking(move || engine.set_setting(KEY_DRIVER_MIRROR, &saved)).await?;
    // 这一项**真的会生效**：驱动下载时会读它换掉仓库根
    // （见 `api::driver::download` 与内核的 `driver_artifact_url_with_base`）——
    // 界面上写着「保存后立即生效」，所以它必须是真的。
    tracing::info!(mirror = %mirror, "驱动下载镜像已更新");
    Ok(Json(json!({ "mirror": mirror })))
}

pub async fn legacy_tls_get(State(state): State<AppState>) -> XResult<Json<Value>> {
    let engine = state.engine.clone();
    let value = blocking(move || engine.get_setting(KEY_ALLOW_LEGACY_TLS)).await?;
    let allow = value.as_deref() == Some("true");
    Ok(Json(json!({ "allowLegacyTls": allow, "relaxed": false })))
}

pub async fn legacy_tls_put(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let allow = body
        .get("allowLegacyTls")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let engine = state.engine.clone();
    blocking(move || engine.set_setting(KEY_ALLOW_LEGACY_TLS, if allow { "true" } else { "false" }))
        .await?;
    // 真正让旧库连得上的是宿主 JVM 的安全参数，内核没有把这一项下推给宿主的通道，
    // 所以这里只落盘偏好并说明现状（Phase 3 会在拉起宿主时带上）。
    Ok(Json(json!({ "allowLegacyTls": allow, "relaxed": false })))
}

/// 供后续 handler 复用的错误码判断（连接不存在等）。
pub fn is_not_found(err: &dbmind_core::DbMindError) -> bool {
    matches!(err.code, ErrorCode::ConnNotFound)
}

/// 保留 `Params` 的可见性（`open_dir` 用不上查询串，但模块内其余 handler 会）。
#[allow(dead_code)]
fn _keep_params() {
    let _ = Params::parse(None);
}
