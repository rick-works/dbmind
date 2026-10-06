//! 跨平台「系统代理」探测：让命令行侧的行为跟浏览器一致。
//!
//! 背景：Windows / macOS 的图形程序默认遵循**系统代理**，浏览器能访问 GitHub；
//! 而命令行工具（curl、Rust ureq 等）不读这个设置 —— 同一网络环境下浏览器能下载
//! 安装包、DBmind 却连不上（表现为「正在下载…」卡住不动）。
//!
//! 按平台读取各自的系统代理配置，转成 `http://host:port` 交给 ureq：
//!
//! - **环境变量**（最高优先级，跨平台通用，也是 CI / 容器里唯一的入口）
//! - **Windows**：注册表 `HKCU\...\Internet Settings` 的 `ProxyEnable` + `ProxyServer`
//!   （浏览器用的就是这项；飞鸟加速 / Clash 等启动时自动写入，用户无需手动设置）
//! - **macOS**：`scutil --proxy` 的 HTTPSProxy/HTTPProxy + 端口
//! - **Linux**：环境变量之外再尝试 GNOME 的 `gsettings org.gnome.system.proxy`
//!
//! 拿到候选后做一次 TCP 探测（能连上才用），避免「配置写着代理、实际没开」
//! 时把所有请求都拖死。

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// 探测超时：只判断「本机端口能不能连上」，不能等太久
const PROBE_TIMEOUT: Duration = Duration::from_millis(350);

/// 读一个环境变量（跳过空值）
fn env_any(keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Ok(v) = std::env::var(k) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// 规整成 `http://host:port`
///
/// 系统里可能写成 `127.0.0.1:17963`、`http://127.0.0.1:17963/`、
/// `http=1.2.3.4:80;https=127.0.0.1:17963`（WinINET 的多协议写法）。
fn normalize(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // WinINET 多协议写法 `http=host:port;https=host:port`：优先 https，其次 http
    let picked: Option<String> = if trimmed.contains('=') && (trimmed.contains("https=") || trimmed.contains("http=")) {
        let pick = |scheme: &str| -> Option<String> {
            trimmed.split(';').find_map(|kv| {
                kv.trim()
                    .strip_prefix(&format!("{scheme}="))
                    .map(|v| v.trim().trim_start_matches("http://").trim_start_matches("https://").to_string())
            })
        };
        pick("https").or_else(|| pick("http"))
    } else {
        None
    };
    let mut s: &str = picked.as_deref().unwrap_or(trimmed);
    s = s.trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_start_matches("socks5://")
        .trim_start_matches("socks5h://")
        .trim_end_matches('/');
    if let Some(pos) = s.find('/') {
        s = &s[..pos];
    }
    if s.is_empty() {
        return None;
    }
    Some(format!("http://{s}"))
}

/// 候选地址（按优先级）
fn candidates() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(v) = env_any(&[
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]) {
        if let Some(n) = normalize(&v) {
            out.push(n);
        }
    }
    if let Some(n) = platform_proxy() {
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

#[cfg(windows)]
fn platform_proxy() -> Option<String> {
    // 用 reg.exe 读，免去引入 winreg 依赖
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    let out = std::process::Command::new("reg")
        .args(["query", KEY, "/v", "ProxyEnable"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let on = text
        .lines()
        .find(|l| l.contains("ProxyEnable"))
        .and_then(|l| l.split_whitespace().last())
        .map(|v| v == "0x1")
        .unwrap_or(false);
    if !on {
        return None;
    }
    let out = std::process::Command::new("reg")
        .args(["query", KEY, "/v", "ProxyServer"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let raw = text
        .lines()
        .find(|l| l.contains("ProxyServer"))
        .and_then(|l| l.split("REG_SZ").nth(1))
        .map(|v| v.trim().to_string())?;
    normalize(&raw)
}

#[cfg(target_os = "macos")]
fn platform_proxy() -> Option<String> {
    // scutil --proxy 输出示例：
    //   <dictionary> { HTTPSEnable : 1  HTTPSProxy : 127.0.0.1  HTTPSPort : 7890 ... }
    let out = std::process::Command::new("scutil").arg("--proxy").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let grab = |key: &str| -> Option<String> {
        let re = format!("{} : ", key);
        text.lines()
            .find(|l| l.contains(&re))
            .and_then(|l| l.split(&re).nth(1))
            .map(|v| v.trim().to_string())
    };
    let on = |k: &str| grab(k).as_deref() == Some("1");
    let pick = |enabled: bool, hk: &str, pk: &str| -> Option<String> {
        if !enabled {
            return None;
        }
        let host = grab(hk)?;
        let port = grab(pk).unwrap_or_else(|| "80".into());
        normalize(&format!("{}:{}", host, port))
    };
    pick(on("HTTPSEnable"), "HTTPSProxy", "HTTPSPort")
        .or_else(|| pick(on("HTTPEnable"), "HTTPProxy", "HTTPPort"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_proxy() -> Option<String> {
    // GNOME：gsettings org.gnome.system.proxy mode / https host,port
    let mode = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.system.proxy", "mode"])
        .output()
        .ok()?;
    if !String::from_utf8_lossy(&mode.stdout).contains("manual") {
        return None;
    }
    let read = |schema: &str, key: &str| -> Option<String> {
        let o = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.system.proxy", schema, key])
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&o.stdout).to_string();
        s.split('\'').nth(1).map(|v| v.to_string())
    };
    for scheme in ["https", "http"] {
        if let (Some(h), Some(p)) = (read(scheme, "host"), read(scheme, "port")) {
            if !h.is_empty() {
                if let Some(n) = normalize(&format!("{}:{}", h, p)) {
                    return Some(n);
                }
            }
        }
    }
    None
}

#[cfg(not(any(windows, unix)))]
fn platform_proxy() -> Option<String> {
    None
}

/// 该代理地址本地是否有人监听
fn alive(url: &str) -> bool {
    let hp = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let hp = hp.split('/').next().unwrap_or(hp);
    if let Ok(mut addrs) = hp.to_socket_addrs() {
        for addr in addrs.by_ref() {
            if !matches!(addr, SocketAddr::V4(_)) {
                continue;
            }
            if TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok() {
                return true;
            }
        }
    }
    false
}

/// 当前应当使用的代理（已规整 + 已探测存活）；没有可用代理返回 `None`（直连）
pub fn resolve() -> Option<String> {
    for c in candidates() {
        if alive(&c) {
            tracing::info!(target: "dbmind::update", proxy = %c, "检测到可用系统代理，更新请求将走代理");
            return Some(c);
        }
    }
    None
}

/// 供诊断/日志用：候选列表（不探测）
pub fn describe() -> String {
    let list = candidates();
    if list.is_empty() {
        "none".into()
    } else {
        list.join(", ")
    }
}