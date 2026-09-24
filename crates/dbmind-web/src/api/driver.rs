//! 驱动**按需下载**。
//!
//! 为什么这一层必须有它：上游的界面在连接弹窗里就写着「首次连接将自动从 Maven 中心下载驱动，
//! 请保持网络畅通」—— 那是它 Java 后端的行为。内核只提供「下载一个 jar 到驱动目录」的**能力**
//! （原生契约里的 `POST /api/dbmind/drivers/{key}/fetch`），并不会在连接时自动触发。
//! 不把它接上的后果很具体：用户装好、点「测试连接」、看到一条 `DBMIND-DRV-0001 驱动未安装`，
//! 而他刚在界面上被承诺过会自动下载 —— 看起来就是「这功能坏了」。
//!
//! 两条刻意的设计：
//!
//! 1. **失败要缓存**。离线或代理不通时，下载会卡到超时；如果每次查询都重试一遍，
//!    整个界面会变成一卡一顿。所以失败后把原因记在内存里，后续调用直接返回那条原因（快且同样有信息量），
//!    想再试一次就调 `POST /api/drivers/{code}/install`（会清掉缓存并重新下载）。
//! 2. **只对「需要 JDBC 驱动」的类型做事**。SQLite 是内核自带的原生实现；Mongo/Redis/ES 的
//!    驱动打在各自宿主包里 —— 对它们伸手下载反而是错的。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::Json;
use dbmind_core::{ConnectionKind, DbMindError, ErrorCode};
use serde_json::{json, Value};

use crate::api::error::{XError, XResult};
use crate::api::{blocking, require_record};
use crate::AppState;

/// 该类型的驱动是不是**齐全**（主驱动 + YAML 里声明的额外依赖都在）。
///
/// 老写法只要目录里**有任意一个** jar 就算就绪 —— 于是「下到了瘦包、缺 slf4j」
/// 这种状态在界面上显示成绿灯（「驱动已就绪，可直接连接」），用户点连接才炸。
/// 实测的 ClickHouse 就是这样：1 MB 的 `clickhouse-jdbc` 就位即亮绿灯，
/// 真正连的时候抛 NoClassDefFoundError: org/slf4j/LoggerFactory。
fn installed(kind: ConnectionKind) -> bool {
    let Some(agent_key) = kind.agent_key() else {
        return true;
    };
    let expected = expected_jars(kind);
    if expected.is_empty() {
        return true;
    }
    let present = jar_names(agent_key);
    expected.iter().all(|name| present.contains(name))
}

/// 该类型需要在驱动目录里出现的 jar 文件名（由 YAML 坐标推出）。
///
/// `pub(crate)`：`sys::driver_status` 也要用（设置页要显示「需要哪些 jar / 还缺哪些」）。
pub(crate) fn expected_jars(kind: ConnectionKind) -> Vec<String> {
    kind.jdbc_artifacts()
        .iter()
        .filter_map(|artifact| dbmind_core::driver_jar_name(artifact).ok())
        .collect()
}

/// 驱动目录里现有哪些 jar（只取文件名）。
pub(crate) fn jar_names(agent_key: &str) -> Vec<String> {
    dbmind_core::installed_driver_jars(agent_key)
        .iter()
        .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().to_string()))
        .collect()
}

/// 删掉**同一个 artifact、别的版本**的旧 jar。
///
/// 为什么必须有：宿主的类路径是**整个驱动目录**（`installed_driver_jars` 把目录里所有
/// jar 都给出去），所以 YAML 里把版本号一改，旧版本不会消失、和新版本一起进 classpath，
/// JVM 加载到哪一个是不确定的 —— 表现成「我明明改了版本，行为却是旧的」
/// （实测场景：Derby 从 10.17 降到 10.16，旧 10.17 仍在目录里）。
/// 只按「同一 artifact 名 + 非预期版本」删，用户自己上传的其它 jar 不受影响。
fn prune_other_versions(agent_key: &str, artifact: &str, keep: &str) {
    // 坐标 `group:artifact:version[:classifier]` 的第二段就是 artifact 名
    let Some(artifact_id) = artifact.split(':').nth(1) else {
        return;
    };
    let prefix = format!("{artifact_id}-");
    for path in dbmind_core::installed_driver_jars(agent_key) {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        if name == keep || !name.starts_with(&prefix) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            tracing::info!(jar = %path.display(), keep = %keep, "已清掉同名的旧版本驱动");
        }
    }
}

fn failures() -> &'static Mutex<HashMap<String, String>> {
    static FAILURES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_failure(kind: ConnectionKind) -> Option<String> {
    failures()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(kind.key())
        .cloned()
}

fn remember_failure(kind: ConnectionKind, message: &str) {
    failures()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(kind.key().to_string(), message.to_string());
}

fn clear_failure(kind: ConnectionKind) {
    failures()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(kind.key());
}

/// 真正下载：`group:artifact:version[:classifier]` → Maven Central → 驱动目录。
///
/// **逐个补齐**（已有的跳过）：额外依赖是后来才加的，不该因为"主驱动已在"就
/// 整段跳过 —— 那会让缺依赖的类型永远停在"缺 slf4j"上，只能靠手工清目录。
async fn download(state: &AppState, kind: ConnectionKind) -> XResult<()> {
    let Some(agent_key) = kind.agent_key().map(str::to_string) else {
        return Ok(());
    };
    let label = kind.label().to_string();
    let artifacts: Vec<String> = kind
        .jdbc_artifacts()
        .iter()
        .map(|artifact| artifact.to_string())
        .collect();
    if artifacts.is_empty() {
        return Ok(());
    }
    // 「驱动下载镜像」设置：换掉仓库根。
    // 设置页写着「保存后立即生效」，而原先只落盘、每次仍从 Maven Central 拉 ——
    // 内网/镜像环境下那句话就是空头承诺（jar 拉不回来，用户只会看到驱动下载失败）。
    let engine = state.engine.clone();
    let mirror = blocking(move || engine.get_setting(crate::api::sys::KEY_DRIVER_MIRROR))
        .await
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty());
    let source = if mirror.is_some() { "镜像" } else { "Maven 中心" };
    let outcome = blocking(move || {
        let present = jar_names(&agent_key);
        for artifact in &artifacts {
            let name = dbmind_core::driver_jar_name(artifact)?;
            if present.contains(&name) {
                prune_other_versions(&agent_key, artifact, &name);
                continue;
            }
            let url = dbmind_core::driver_artifact_url_with_base(artifact, mirror.as_deref())?;
            let response = ureq::get(&url).call().map_err(|err| {
                DbMindError::new(
                    ErrorCode::DriverNotReady,
                    format!("从{source}下载 {label} 驱动失败（{name}）：{err}"),
                )
                .with_detail(url.clone())
            })?;
            let mut bytes: Vec<u8> = Vec::new();
            std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes)?;
            let jar = dbmind_core::install_driver_jar(&agent_key, &name, &bytes)?;
            tracing::info!(agent = %agent_key, jar = %jar.display(), bytes = bytes.len(), "驱动已下载");
            prune_other_versions(&agent_key, artifact, &name);
        }
        Ok(())
    })
    .await
    .map(|_| ())
    .map_err(|err| {
        let message = err.message.clone();
        // 缓存到「按 kind 记失败原因」的表里：下次直接返回，不再卡一次超时
        XError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "{message}。可手动把驱动 jar 放进 {}\
                 （或检查网络 / 代理后重试：POST /api/drivers/{}/install）",
                dbmind_core::driver_dir(kind.agent_key().unwrap_or_default()).display(),
                kind.key().to_ascii_uppercase()
            ),
        )
    });
    if outcome.is_ok() {
        // **下载成功就回收宿主**：宿主 JVM 的 classpath 在启动时固定，新下的 jar 不会
        // 自动进去。不回收的话，后补的依赖（如 ClickHouse 的 slf4j-api）永远不生效 ——
        // 界面显示「驱动已就绪」，一连接却 NoClassDefFoundError。
        let engine = state.engine.clone();
        blocking(move || {
            engine.recycle_agent_hosts();
            Ok(())
        })
        .await
        .ok();
    }
    outcome
}

/// 确保该类型可用：驱动已装就什么都不做；没装就下（失败记住原因，不反复重试）。
///
/// 需要 `state`：下载地址要读「驱动下载镜像」这一项设置（见 `download`）。
pub async fn ensure_installed(state: &AppState, kind: ConnectionKind) -> XResult<()> {
    if !kind.is_jdbc() || installed(kind) {
        return Ok(());
    }
    if let Some(message) = cached_failure(kind) {
        return Err(XError::new(StatusCode::SERVICE_UNAVAILABLE, message));
    }
    match download(state, kind).await {
        Ok(()) => Ok(()),
        Err(err) => {
            remember_failure(kind, &err.message);
            Err(err)
        }
    }
}

/// 按连接 id 确保驱动可用（元数据/查询路径的统一入口）。
pub async fn ensure_for_connection(state: &AppState, id: &str) -> XResult<()> {
    let record = require_record(state, id).await?;
    ensure_installed(state, record.kind()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 就绪检查必须「**齐了**才算就绪」。
    ///
    /// 这条守着一次真实故障：ClickHouse 的主驱动默认是瘦包，缺 slf4j 时驱动类装载失败
    /// （NoClassDefFoundError: org/slf4j/LoggerFactory），而旧实现只看「目录里有任意 jar」
    /// —— 界面显示绿灯「驱动已就绪，可直接连接」，用户点连接才炸。
    /// 现在 YAML 里把额外依赖也声明出来，就绪检查要求两份都在。
    #[test]
    fn clickhouse_需要两个_jar_才算就绪() {
        let names = expected_jars(ConnectionKind::Clickhouse);
        assert_eq!(names.len(), 2, "实际：{names:?}");
        assert!(
            names.iter().any(|name| name.contains("clickhouse-jdbc-0.7.2-all")),
            "主驱动必须是 all 分类器包（默认瘦包缺依赖）：{names:?}"
        );
        assert!(
            names.iter().any(|name| name.contains("slf4j-api")),
            "额外依赖也要算进就绪检查：{names:?}"
        );
    }

    /// 只有一个驱动的类型仍只要求那一个 —— 别把规则写成"越多越好"。
    #[test]
    fn 单驱动类型只要求一个_jar() {
        assert_eq!(expected_jars(ConnectionKind::Mysql).len(), 1);
        assert!(
            expected_jars(ConnectionKind::Sqlite).is_empty(),
            "SQLite 走内核原生实现，不需要任何 jar"
        );
    }
}

/// `POST /api/drivers/{code}/install` —— 强制重新下载（清掉失败缓存，用于重试）。
pub async fn install(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> XResult<Json<Value>> {
    let kind = ConnectionKind::from_key(&code.to_ascii_lowercase())
        .ok_or_else(|| XError::bad_request(format!("不支持的数据库类型：{code}")))?;
    clear_failure(kind);
    if !kind.is_jdbc() {
        return Ok(Json(json!({
            "success": true,
            "ready": true,
            "message": format!("{} 不需要额外下载驱动（内核自带或打在宿主包里）", kind.label()),
        })));
    }
    if installed(kind) {
        return Ok(Json(json!({
            "success": true,
            "ready": true,
            "message": format!("{} 驱动已就绪", kind.label()),
        })));
    }
    match download(&state, kind).await {
        Ok(()) => Ok(Json(json!({
            "success": true,
            "ready": true,
            "message": format!("{} 驱动已下载", kind.label()),
        }))),
        Err(err) => Err(err),
    }
}

/// 上传体积上限。驱动 jar 常见几 MB，shaded 包能到几十 MB —— 给到 256 MB 足够宽裕。
pub const DRIVER_UPLOAD_LIMIT: usize = 256 * 1024 * 1024;

/// `POST /api/drivers/{code}/upload` —— 手动上传驱动 jar（multipart，文件字段名 `file`，可多选）。
///
/// 为什么要有它：内网 / 离线机器根本拉不到 Maven 仓库，而「驱动下载」这一页此前只能选镜像源，
/// 用户唯一的出路是**自己找到 `~/.dbmind/drivers/<key>/` 把 jar 拷进去** —— 那不是一个界面该给人的作业。
///
/// 落盘完全复用内核那一步（`install_driver_jar`）：它会挡 `../`、路径分隔符、非 `.jar`、空文件。
/// 这里**不自己再写一遍文件写入** —— 校验漏一项就等于把"往任意路径写文件"交给了调用方。
///
/// 上传成功后与下载成功做同一件事：**回收宿主 JVM**。宿主的 classpath 在启动时固定，
/// 不回收的话刚传上去的 jar 不会进 classpath，界面显示「已就绪」、一连却 NoClassDefFoundError。
///
/// 就绪判定仍按 YAML 声明的文件名严格比对，所以上传的文件名与要求不一致时**如实**返回
/// `ready: false` + 缺哪些文件，而不是假装成功（`missing` 会直接显示在设置页那一行上）。
pub async fn upload(
    State(state): State<AppState>,
    Path(code): Path<String>,
    mut multipart: Multipart,
) -> XResult<Json<Value>> {
    let kind = ConnectionKind::from_key(&code.to_ascii_lowercase())
        .ok_or_else(|| XError::bad_request(format!("不支持的数据库类型：{code}")))?;
    let Some(agent_key) = kind.agent_key().map(str::to_string) else {
        return Err(XError::bad_request(format!(
            "{} 不需要驱动包（内核自带或打在宿主包里），无需上传",
            kind.label()
        )));
    };

    let mut saved: Vec<Value> = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| XError::bad_request(format!("解析上传表单失败：{e}")))?
    {
        // 文件名要在消费掉字段之前取
        let filename = field.file_name().map(str::to_string).unwrap_or_default();
        // 无论要不要这个字段都要把它读完，否则后续字段读不到（流式解析）—— 与 import 同一约定
        let bytes = field
            .bytes()
            .await
            .map_err(|e| XError::bad_request(format!("读取上传文件失败：{e}")))?;
        if filename.is_empty() {
            continue;
        }
        let path = dbmind_core::install_driver_jar(&agent_key, &filename, &bytes)
            .map_err(|err| XError::bad_request(err.message))?;
        tracing::info!(agent = %agent_key, jar = %path.display(), bytes = bytes.len(), "驱动已上传");
        saved.push(json!({
            "file": filename,
            "path": path.display().to_string(),
            "bytes": bytes.len(),
        }));
    }
    if saved.is_empty() {
        return Err(XError::bad_request(
            "没有收到上传文件（multipart 的文件字段名应为 file）",
        ));
    }

    let engine = state.engine.clone();
    blocking(move || {
        engine.recycle_agent_hosts();
        Ok(())
    })
    .await
    .ok();

    let jars = jar_names(&agent_key);
    let missing: Vec<String> = expected_jars(kind)
        .into_iter()
        .filter(|name| !jars.contains(name))
        .collect();
    let ready = missing.is_empty();
    let message = if ready {
        format!("{} 驱动已就绪", kind.label())
    } else {
        format!(
            "已上传 {} 个文件，但仍缺：{}（就绪判定要求文件名与类型声明一致，必要时可改名后重传）",
            saved.len(),
            missing.join("、")
        )
    };
    Ok(Json(json!({
        "success": true,
        "ready": ready,
        "files": saved,
        "jars": jars,
        "missing": missing,
        "driverDir": dbmind_core::driver_dir(&agent_key).display().to_string(),
        "message": message,
    })))
}
