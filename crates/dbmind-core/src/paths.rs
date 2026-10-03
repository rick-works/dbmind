//! 应用目录解析。`DBMIND_HOME` 可整体重定向（测试与多实例都靠它）。

use crate::error::{DbMindError, ErrorCode, Result};
use std::path::{Path, PathBuf};

pub const HOME_ENV: &str = "DBMIND_HOME";

/// 用户级基目录（放「数据目录指针文件」的地方）。
fn base_dir() -> PathBuf {
    let base = std::env::var("USERPROFILE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("HOME").ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| ".".to_string());
    PathBuf::from(base)
}

/// 指针文件：内容是一个目录路径。设置页「迁移数据目录」写它，重启后 [`home_dir`] 按它落位。
pub fn home_pointer_path() -> PathBuf {
    base_dir().join(".dbmind-home")
}

/// 应用数据目录：`$DBMIND_HOME`（显式覆盖，测试/多实例用），
/// 否则指针文件 `.dbmind-home` 指向的目录（迁移过），否则 `~/.dbmind`（默认）。
pub fn home_dir() -> PathBuf {
    if let Ok(custom) = std::env::var(HOME_ENV) {
        if !custom.trim().is_empty() {
            return PathBuf::from(custom);
        }
    }
    if let Some(home) = pointer_home() {
        return home;
    }
    base_dir().join(".dbmind")
}

/// 读指针文件：内容为空/目录不存在时忽略（迁移到 U 盘又拔了的场景不能让应用起不来）。
fn pointer_home() -> Option<PathBuf> {
    let raw = std::fs::read_to_string(home_pointer_path()).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let home = PathBuf::from(trimmed);
    (home.is_absolute() && home.is_dir()).then_some(home)
}

/// 默认的元数据库路径。
pub fn default_store_path() -> PathBuf {
    home_dir().join("dbmind.db")
}

/// **出厂默认**数据目录（`~/.dbmind`）：不看指针、不看 `DBMIND_HOME`。
///
/// 为什么要与 [`home_dir`] 分开：迁移过后 `home_dir()` 指向新家，若把「当前目录」
/// 当「默认目录」，设置页的「恢复默认」就永远填回当前值 —— 按钮等于失效。
/// 恢复默认的目标必须是这个不随迁移变的出厂值。
pub fn factory_default_home() -> PathBuf {
    base_dir().join(".dbmind")
}

/// 历史 SQL 文件与导出产物的落盘目录。
pub fn work_dir() -> PathBuf {
    home_dir().join("work")
}

/// Java 宿主专用的**自有 hosts 文件**（由 `agent::prepare_agent_hosts` 生成）。
///
/// 为什么需要它：MySQL 驱动在 `ConnectionImpl` 构造里会调 `InetSocketAddress.getHostName()`，
/// 也就是**把服务端 IP 反解成主机名**。企业 DNS 一般没有这些 PTR 记录，而 Java 对
/// **失败**的解析结果又不缓存 —— 于是每新建一条会话都白等一次 DNS 超时。
///
/// 实测（真机 MySQL 5.7 / Connector-J 9.1.0 / JDK 25，同一进程内连建四条会话条条如此）：
/// 反解一次 4573ms，而 `nslookup` 143ms 就回了 NXDOMAIN —— 时间全花在客户端的等待上。
/// 与此同时纯 TCP 建连只要 29~47ms，所以「连数据库慢 5 秒」与网络和数据库都无关。
///
/// JDK 9+ 的 `-Djdk.net.hosts.file` 可以让 JVM 用一份**自有**的 hosts 文件：
/// 把连接用到的 IP 映射到占位名后，同一次反解 **4573ms → 16ms**。
/// 不需要管理员权限，也不改系统配置。
pub fn agent_hosts_path() -> PathBuf {
    home_dir().join("agent-hosts")
}

/// 「允许旧版 TLS」的 java.security 覆盖文件（内容与用途见 `agent.rs`）。
pub fn legacy_tls_properties_path() -> PathBuf {
    home_dir().join("legacy-tls.security")
}

pub fn ensure_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| {
        DbMindError::new(
            ErrorCode::StorageFailed,
            format!("创建目录失败: {}", dir.display()),
        )
        .with_detail(e.to_string())
    })
}

/// 确保父目录存在（写文件前调用）。
pub fn ensure_parent(file: &Path) -> Result<()> {
    if let Some(parent) = file.parent() {
        if !parent.as_os_str().is_empty() {
            ensure_dir(parent)?;
        }
    }
    Ok(())
}
