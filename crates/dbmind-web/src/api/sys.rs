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
use dbmind_core::{DbMindEngine, DbMindError};
use serde_json::{json, Value};

use crate::api::dialect;
use crate::api::error::{XError, XResult};
use crate::api::driver::{expected_jars, jar_names};
use crate::api::{blocking, Params};
use crate::AppState;

/// 驱动下载镜像（`api::driver` 下载时要读它换仓库根，故为 pub）。
pub const KEY_DRIVER_MIRROR: &str = "driver.mirror";
// 键名用内核导出的常量（见 query.rs 同款说明），杜绝两边各写一份字符串漂移

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
        .engine()
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
    let types = state.engine().types();
    let engine = state.engine();
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
    // 「默认目录」必须是**出厂值**（~/.dbmind，不随指针/迁移变）——
    // 若给当前目录，迁移过后「恢复默认」永远填回当前值，按钮等于失效
    let default_dir = dbmind_core::paths::factory_default_home();
    // 驱动固定在数据目录下（join 用平台分隔符，避免正反斜杠混排的展示）
    let driver_dir = std::path::Path::new(&data_dir).join("drivers");
    let log_dir = std::path::Path::new(&data_dir).join("logs");
    json!({
        "os": if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" },
        "osLabel": if cfg!(windows) { "Windows" } else if cfg!(target_os = "macos") { "macOS" } else { "Linux" },
        "homeDir": home,
        "dataDir": data_dir,
        "defaultDataDir": default_dir.display().to_string(),
        "driverDir": driver_dir.display().to_string(),
        "defaultDriverDir": driver_dir.display().to_string(),
        "logDir": log_dir.display().to_string(),
    })
}

pub async fn paths_get(State(state): State<AppState>) -> XResult<Json<Value>> {
    let engine = state.engine();
    let summary = blocking(move || Ok(engine.runtime_summary())).await?;
    Ok(Json(paths_json(&summary.store_path)))
}

/// 目标目录与现有目录是不是同一处（任意一边不存在就当不同）。
fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// 迁移数据目录并**返回打开自新主库的引擎**（调用方拿去热切换，立即生效）。
///
/// 为什么可以不重启：引擎会把「该打开哪个库」定死在启动那一刻，但驱动 jar 路径是
/// **每次拉起宿主时**动态解析的（`availability_for` → `paths::home_dir()`，读的是
/// 指针文件落位后的目录），work/prompts 等子目录同理 —— 所以只要把「引擎」整个换掉，
/// 其余一切都自然落到新目录。旧引擎 drop 时由内核收掉它的全部宿主 JVM，不留孤儿。
///
/// 数据安全的两条边：
/// - 快照（`VACUUM INTO`）取自**当前活着的库** —— 保存动作之前的所有写入都在它身上；
/// - 从快照到 [`super::super::AppState::swap_engine`] 之间的几十毫秒里落到旧库的写入会丢
///   （窗口极小；真在意的用户迁移本就该挑空闲时段）。
///
/// 指针文件（`~/.dbmind-home`）**仍然要写**：它管的是「下次启动进程」落位到哪 ——
/// 热切换只管本次进程。
fn prepare_and_open(engine: &DbMindEngine, requested: &str) -> dbmind_core::Result<DbMindEngine> {
    use dbmind_core::paths;
    // 输入不合法用 `QueryInvalid`（通用 400 桶）；操作失败用 `StorageFailed`/`Internal`（500）
    let invalid = |msg: String| {
        DbMindError::new(dbmind_core::ErrorCode::QueryInvalid, msg)
    };
    let failed = |msg: String| DbMindError::new(dbmind_core::ErrorCode::StorageFailed, msg);
    let target = std::path::PathBuf::from(requested.trim());
    if !target.is_absolute() {
        return Err(invalid("数据目录必须是绝对路径".to_string()));
    }
    let current = paths::home_dir();
    if same_dir(&current, &target) {
        return Err(invalid("新目录与当前数据目录相同".to_string()));
    }
    std::fs::create_dir_all(&target).map_err(|e| {
        invalid(format!("无法创建目标目录 {}: {e}", target.display()))
    })?;
    // 可写探测：等迁完才发现只读就白忙了
    let probe = target.join(".dbmind-write-probe");
    std::fs::write(&probe, b"ok")
        .map_err(|e| invalid(format!("目标目录不可写: {e}")))?;
    let _ = std::fs::remove_file(&probe);
    // 目标已有数据：**改名保留**（时间戳 .bak）而不是拒绝，也不是悄悄覆盖 ——
    // 最常见的撞车场景恰恰是「迁回原来的目录」，直接拒绝会把这条最自然的路堵死；
    // 而直接覆盖可能毁掉别人在用的库。.bak 两头都顾住：迁移继续走，旧数据可找回。
    let stamp = chrono::Local::now().format("%Y%m%d%H%M%S");
    let old_db = target.join("dbmind.db");
    if old_db.exists() {
        std::fs::rename(&old_db, target.join(format!("dbmind.db.bak-{stamp}")))
            .map_err(|e| failed(format!("备份目标目录已有的 dbmind.db 失败: {e}")))?;
        for suffix in ["-wal", "-shm"] {
            let side = target.join(format!("dbmind.db{suffix}"));
            if side.exists() {
                let _ = std::fs::rename(&side, target.join(format!("dbmind.db{suffix}.bak-{stamp}")));
            }
        }
    }
    let old_drivers = target.join("drivers");
    if old_drivers.exists() {
        std::fs::rename(&old_drivers, target.join(format!("drivers.bak-{stamp}")))
            .map_err(|e| failed(format!("备份目标目录已有的 drivers 失败: {e}")))?;
    }

    // ① 驱动一并搬（几百 MB 级别，重新下载比拷贝贵得多）——放最前：它不碰库，
    //    失败时连快照都还没做，代价最小
    let drivers = current.join("drivers");
    if drivers.is_dir() {
        copy_dir_all(&drivers, &target.join("drivers"))
            .map_err(|e| failed(format!("复制驱动目录失败: {e}")))?;
    }
    // ② 宿主 hosts 反解文件（丢了只是回退到慢速反解，照搬省事）
    let hosts = current.join("agent-hosts");
    if hosts.is_file() {
        let _ = std::fs::copy(&hosts, target.join("agent-hosts"));
    }
    // ③ 元数据库快照（`VACUUM INTO`：读事务里的一致性拷贝，不怕库开着）——
    //    放在拷驱动**之后**：快照到切换之间的写入窗口被压到只有「开新引擎」这几毫秒
    let new_db = target.join("dbmind.db");
    engine.backup_store_into(&new_db)?;
    // ④ 写指针 —— 这一步成功迁移才算数；此前失败只是白拷一份，现状不受影响
    std::fs::write(
        paths::home_pointer_path(),
        target.to_string_lossy().as_bytes(),
    )
    .map_err(|e| failed(format!("写入数据目录指针失败: {e}")))?;
    // ⑤ 打开新引擎交还给调用方热切换（快照里的库 + 新目录下的驱动）
    let new_engine = DbMindEngine::open(&new_db).map_err(|e| {
        // 开不起来就回滚指针：本次进程继续用旧目录，别留下「指了个坏库」的指针
        let _ = std::fs::remove_file(paths::home_pointer_path());
        failed(format!("打开新数据目录的元数据库失败: {e}"))
    })?;
    tracing::info!(from = %current.display(), to = %target.display(), "数据目录已迁移");
    Ok(new_engine)
}

pub async fn paths_put(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    // 导出目录不是可配置项：导出由用户自己选保存位置，服务端只用临时目录中转，取回即删。
    // driverDir 也不单独迁：驱动跟着数据目录走（见 `prepare_and_open`）。
    let requested = body
        .get("dataDir")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if requested.is_empty() {
        return Err(XError::bad_request("请填写新的数据目录"));
    }
    let engine = state.engine();
    let task_dir = requested.clone();
    let new_engine = blocking(move || prepare_and_open(&engine, &task_dir)).await?;
    // 热切换：新请求从此走新库；旧引擎随 drop 收掉宿主 JVM
    state.swap_engine(new_engine);
    Ok(Json(json!({
        "migrated": true,
        "dataDir": requested,
        "restartRequired": false,
    })))
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
        let engine = state.engine();
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
        std::process::Command::new("explorer")
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
    let engine = state.engine();
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
    let engine = state.engine();
    let saved = mirror.clone();
    blocking(move || engine.set_setting(KEY_DRIVER_MIRROR, &saved)).await?;
    // 这一项**真的会生效**：驱动下载时会读它换掉仓库根
    // （见 `api::driver::download` 与内核的 `driver_artifact_url_with_base`）——
    // 界面上写着「保存后立即生效」，所以它必须是真的。
    tracing::info!(mirror = %mirror, "驱动下载镜像已更新");
    Ok(Json(json!({ "mirror": mirror })))
}

pub async fn legacy_tls_get(State(state): State<AppState>) -> XResult<Json<Value>> {
    let engine = state.engine();
    let value = blocking(move || engine.get_setting(dbmind_core::Store::KEY_ALLOW_LEGACY_TLS)).await?;
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
    let engine = state.engine();
    blocking(move || engine.set_setting(dbmind_core::Store::KEY_ALLOW_LEGACY_TLS, if allow { "true" } else { "false" }))
        .await?;
    // `set_setting` 会把开关同步给内核的宿主 spawn 层（`agent::set_legacy_tls_enabled`），
    // 下次拉起宿主 JVM 时带上 `java.security.properties` 覆盖文件放行 TLSv1/TLSv1.1
    // —— 宿主是长驻进程，所以仍要**重启应用**才对已存在的宿主生效。
    Ok(Json(json!({ "allowLegacyTls": allow, "relaxed": false })))
}

/// 保留 `Params` 的可见性（`open_dir` 用不上查询串，但模块内其余 handler 会）。
#[allow(dead_code)]
fn _keep_params() {
    let _ = Params::parse(None);
}
