//! 在线更新：检查 GitHub 最新 Release，与本地版本比对；下载安装包并拉起安装器。
//!
//! - `GET  /api/update/check`     —— { current, latest, hasNew, name, notes, ... }
//! - `POST /api/update/apply`     —— 启动后台下载任务，立即返回 { started }
//! - `GET  /api/update/progress`  —— 下载进度 { status, received, total, speed, ... }
//!
//! 两个关键设计（都是踩过坑才加的）：
//!
//! 1. **走系统代理**：浏览器能用 GitHub 是因为它遵循系统代理（Windows 注册表 /
//!    macOS scutil / Linux gsettings / 环境变量），而命令行工具不读这个设置，
//!    于是浏览器能下载安装包、DBmind 却卡在 0 字节。这里复用 sysproxy 的探测结果。
//! 2. **下载放后台 + 进度可查**：安装包几十 MB，同步 POST 会让前端长时间无反馈
//!    （用户看到正在下载却不知道有没有在动）。现在下载跑在后台任务里，
//!    前端轮询 /update/progress 画进度条，失败时给出可区分的原因。
//!
//! 另：私有仓库的 GitHub API 需要令牌，读 %USERPROFILE%\.dbmind\github-token；
//! 公开仓库匿名即可。带令牌却被拒（401/403）会自动退回匿名重试。

use axum::Json;
use dbmind_core::{DbMindError, ErrorCode};
use serde_json::{json, Value};
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RELEASE_LATEST: &str = "https://api.github.com/repos/rick-works/dbmind/releases/latest";

// ---------- 下载进度（进程内共享：后台任务写、HTTP 读） ----------

#[derive(Default)]
struct Progress {
    status: String, // idle | running | done | failed
    received: u64,
    total: u64,
    speed: u64,
    started: Option<Instant>,
    error: Option<String>,
    installer: Option<String>,
    version: Option<String>,
    proxy: Option<String>,
}

fn progress_cell() -> &'static Arc<std::sync::Mutex<Progress>> {
    use std::sync::OnceLock;
    static CELL: OnceLock<Arc<std::sync::Mutex<Progress>>> = OnceLock::new();
    CELL.get_or_init(|| {
        Arc::new(std::sync::Mutex::new(Progress {
            status: "idle".into(),
            ..Default::default()
        }))
    })
}

/// 构造一个带代理（若系统代理可用）的 ureq Agent
fn agent() -> ureq::Agent {
    let builder = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10));
    match super::sysproxy::resolve() {
        Some(p) => match ureq::Proxy::new(&p) {
            Ok(proxy) => builder.proxy(proxy).build(),
            Err(e) => {
                tracing::warn!(target: "dbmind::update", proxy = %p, error = %e, "代理地址无效，改为直连");
                builder.build()
            }
        },
        None => builder.build(),
    }
}

/// GitHub 私人令牌（可选）：私有仓库 API 需要；文件不存在则匿名访问
fn github_token() -> Option<String> {
    let base = std::env::var("USERPROFILE").ok()?;
    let token =
        std::fs::read_to_string(std::path::Path::new(&base).join(".dbmind").join("github-token")).ok()?;
    let token = token.trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// 发一次请求并解析 JSON。auth 为 None 时匿名访问。
fn request_release(auth: Option<&str>) -> dbmind_core::Result<Value> {
    let mut req = agent()
        .get(RELEASE_LATEST)
        .set("User-Agent", "dbmind-updater")
        .timeout(Duration::from_secs(10));
    if let Some(tk) = auth {
        req = req.set("Authorization", &format!("Bearer {tk}"));
    }
    let resp = req.call().map_err(|e| {
        let msg = e.to_string();
        let hint = if msg.contains("timed out") || msg.contains("connect") {
            format!("连接 GitHub 失败（{msg}）。若你在使用代理，请确认代理程序已启动；也可稍后重试")
        } else {
            format!("连接更新服务器失败：{msg}")
        };
        DbMindError::new(ErrorCode::Internal, hint)
    })?;
    let mut bytes: Vec<u8> = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| DbMindError::new(ErrorCode::Internal, format!("读取发行版信息失败：{e}")))?;
    serde_json::from_slice::<Value>(&bytes)
        .map_err(|e| DbMindError::new(ErrorCode::Internal, format!("解析发行版信息失败：{e}")))
}

/// 阻塞版拉取最新发行版（供后台下载任务在 blocking 线程里直接调用）
fn fetch_latest_blocking() -> dbmind_core::Result<Value> {
    match github_token() {
        // 带令牌被拒（401/403：过期/被撤销/权限不足）-> 丢掉令牌匿名再来一次
        Some(tk) => match request_release(Some(&tk)) {
            Ok(v) => Ok(v),
            Err(_) => request_release(None),
        },
        None => request_release(None),
    }
}

async fn fetch_latest() -> dbmind_core::Result<Value> {
    crate::api::blocking(fetch_latest_blocking)
        .await
        .map_err(|x| DbMindError::new(ErrorCode::Internal, x.message.clone()))
}

/// 简易 semver 比较：a > b（按数字段逐段比，段数不足补 0）
fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parse(a), parse(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// 附件得分：NSIS 安装包 > MSI > 绿色版 zip > 其他
fn asset_score(name: &str) -> u8 {
    if name.ends_with("-setup.exe") {
        3
    } else if name.ends_with(".msi") {
        2
    } else if name.ends_with(".zip") {
        1
    } else {
        0
    }
}

/// 从发行版里挑出最合适的安装包 (url, name)
fn pick_installer(v: &Value) -> Option<(String, String)> {
    let mut target: Option<(String, String)> = None;
    for asset in v["assets"].as_array()? {
        let name = asset["name"].as_str().unwrap_or("");
        let url = asset["browser_download_url"].as_str().unwrap_or("");
        if url.is_empty() {
            continue;
        }
        let score = asset_score(name);
        let best = target.as_ref().map(|(_, n)| asset_score(n)).unwrap_or(0);
        if score > 0 && score > best {
            target = Some((url.to_string(), name.to_string()));
        }
    }
    target
}

/// GET /api/update/check —— 本地版本 vs GitHub 最新发行版
pub async fn check() -> Json<Value> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    match fetch_latest().await {
        Ok(v) => {
            let latest = v["tag_name"]
                .as_str()
                .unwrap_or("")
                .trim_start_matches('v')
                .to_string();
            let has_new = !latest.is_empty() && version_gt(&latest, &current);
            let download = pick_installer(&v).map(|(u, _)| u).unwrap_or_default();
            Json(json!({
                "success": true,
                "current": current,
                "latest": latest,
                "hasNew": has_new,
                "name": v["name"].as_str().unwrap_or(""),
                "notes": v["body"].as_str().unwrap_or(""),
                "publishedAt": v["published_at"].as_str().unwrap_or(""),
                "download": download,
                "releaseUrl": v["html_url"].as_str().unwrap_or(""),
                "proxy": super::sysproxy::resolve(),
                "proxyCandidates": super::sysproxy::describe(),
            }))
        }
        Err(err) => Json(json!({
            "success": false,
            "current": current,
            "message": err.message,
        })),
    }
}

// ---------- POST /api/update/apply —— 启动后台下载 ----------

pub async fn apply(Json(req): Json<ApplyReq>) -> Json<Value> {
    {
        let p = progress_cell().lock().unwrap();
        if p.status == "running" {
            return Json(json!({ "success": true, "started": false, "message": "下载任务已在进行中" }));
        }
    }
    {
        let mut p = progress_cell().lock().unwrap();
        *p = Progress {
            status: "running".into(),
            started: Some(Instant::now()),
            proxy: super::sysproxy::resolve(),
            ..Default::default()
        };
    }

    let cell = progress_cell().clone();
    let want_dir = req.dir.clone();
    // 下载是纯阻塞 IO（几十 MB），放 blocking 线程池，不占 async worker
    tokio::task::spawn_blocking(move || {
        let picked = fetch_latest_blocking().and_then(|v| {
            let tag = v["tag_name"].as_str().unwrap_or("latest").to_string();
            let (url, name) = pick_installer(&v).ok_or_else(|| {
                DbMindError::new(ErrorCode::Internal, "最新发行版没有可下载的安装包（只有源码包）")
            })?;
            Ok::<_, DbMindError>((url, name, tag))
        });
        let (url, name, tag) = match picked {
            Ok(x) => x,
            Err(e) => {
                let mut p = cell.lock().unwrap();
                p.status = "failed".into();
                p.error = Some(e.message);
                return;
            }
        };
        let dest = resolve_dir(want_dir.as_deref()).join(&name);
        if let Some(dir) = dest.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                let mut p = cell.lock().unwrap();
                p.status = "failed".into();
                p.error = Some(format!("创建临时目录失败：{e}"));
                return;
            }
        }
        match download_with_progress(&url, &dest, &cell) {
            Ok(()) => {
                let installer = dest.to_string_lossy().to_string();
                let ok = launch_installer(&dest);
                let mut p = cell.lock().unwrap();
                p.installer = Some(installer);
                p.version = Some(tag.trim_start_matches('v').to_string());
                if ok {
                    p.status = "done".into();
                } else {
                    p.status = "failed".into();
                    p.error = Some("安装包已下载，但启动安装器失败，请手动运行它".into());
                }
            }
            Err(e) => {
                let mut p = cell.lock().unwrap();
                p.status = "failed".into();
                p.error = Some(e.message);
            }
        }
    });

    Json(json!({ "success": true, "started": true }))
}

/// 下载目标目录：用户指定优先（更新弹窗里选的保存位置），否则用系统临时目录。
///
/// 不做自动清理：文件落在用户自己找得到的目录里，留不留由用户决定
/// (放在 C:UsersRickAppDataLocalTemp 下既难找，也只能等系统临时文件策略被动回收)。
fn resolve_dir(dir: Option<&str>) -> std::path::PathBuf {
    if let Some(d) = dir.map(str::trim).filter(|s| !s.is_empty()) {
        let p = std::path::PathBuf::from(d);
        if p.is_dir() {
            return p;
        }
        if std::fs::create_dir_all(&p).is_ok() {
            return p;
        }
        tracing::warn!(target: "dbmind::update", dir = %p.display(), "指定的保存目录不可用，改用临时目录");
    }
    std::env::temp_dir().join("dbmind-update")
}

/// POST /api/update/apply 的请求体
#[derive(serde::Deserialize, Default)]
pub struct ApplyReq {
    /// 下载保存目录（用户在更新弹窗里选）
    #[serde(default)]
    pub dir: Option<String>,
}
/// 下载并持续写入进度
fn download_with_progress(
    url: &str,
    dest: &std::path::Path,
    cell: &Arc<std::sync::Mutex<Progress>>,
) -> dbmind_core::Result<()> {
    use std::io::Write;
    let mut req = agent()
        .get(url)
        .set("User-Agent", "dbmind-updater")
        .timeout(Duration::from_secs(1800));
    if let Some(tk) = github_token() {
        req = req.set("Authorization", &format!("Bearer {tk}"));
    }
    let resp = req.call().map_err(|e| {
        DbMindError::new(
            ErrorCode::Internal,
            format!("下载安装包失败：{e}（若你在用代理，请确认代理程序已启动）"),
        )
    })?;
    let total = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    {
        let mut p = cell.lock().unwrap();
        p.total = total;
    }
    let mut file = std::fs::File::create(dest)
        .map_err(|e| DbMindError::new(ErrorCode::Internal, format!("写安装包失败：{e}")))?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut received: u64 = 0;
    let mut last_report = Instant::now();
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| DbMindError::new(ErrorCode::Internal, format!("下载中断：{e}")))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| DbMindError::new(ErrorCode::Internal, format!("写安装包失败：{e}")))?;
        received += n as u64;
        // 每 200ms 更新一次，避免频繁加锁
        if last_report.elapsed() >= Duration::from_millis(200) {
            let secs = cell.lock().unwrap().started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.001);
            let mut p = cell.lock().unwrap();
            p.received = received;
            p.speed = (received as f64 / secs) as u64;
            last_report = Instant::now();
        }
    }
    {
        let mut p = cell.lock().unwrap();
        p.received = received;
        p.speed = 0;
    }
    if total > 0 && received + 1024 < total {
        return Err(DbMindError::new(
            ErrorCode::Internal,
            format!("安装包不完整（{}/{} 字节），请重试", received, total),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn launch_installer(path: &std::path::Path) -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &path.to_string_lossy()])
        .creation_flags(0x0000_0008)
        .spawn()
        .is_ok()
}

#[cfg(target_os = "macos")]
fn launch_installer(path: &std::path::Path) -> bool {
    std::process::Command::new("open").arg(path).spawn().is_ok()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn launch_installer(path: &std::path::Path) -> bool {
    std::process::Command::new("xdg-open").arg(path).spawn().is_ok()
}

#[cfg(not(any(windows, unix)))]
fn launch_installer(_path: &std::path::Path) -> bool {
    false
}

/// 调起**系统原生**的目录选择对话框（跨平台，零额外依赖）
///
/// - Windows：PowerShell + FolderBrowserDialog（`-STA` 必需，它是单线程 UI）
/// - macOS：osascript 的 `choose folder`（原生 Finder 面板）
/// - Linux：zenity / kdialog（装了才用）
///
/// 弹窗是阻塞的（可能等用户几分钟），所以调用方要放 blocking 线程池。
/// 用户取消返回 `None`。
#[cfg(windows)]
fn pick_dir_native(title: &str) -> Option<String> {
    // 标题里的单引号按 PowerShell 规则转义（''），避免拼进脚本时被截断
    let t = title.replace('\'', "''");
    let ps = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         $d = New-Object System.Windows.Forms.FolderBrowserDialog; \
         $d.Description = '{t}'; $d.ShowNewFolderButton = $true; \
         if ($d.ShowDialog() -eq 'OK') {{ $d.SelectedPath }}"
    );
    let mut ps_cmd = std::process::Command::new("powershell");
    dbmind_core::hide_console(&mut ps_cmd);
    let out = ps_cmd
        .args(["-NoProfile", "-STA", "-WindowStyle", "Hidden", "-Command", &ps])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // PowerShell 会往 stdout 混入警告行，只取最后一个像绝对路径的行
    s.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && l.len() > 2)
        .map(str::to_string)
}

#[cfg(target_os = "macos")]
fn pick_dir_native(title: &str) -> Option<String> {
    let t = title.replace('"', "");
    let out = std::process::Command::new("osascript")
        .args(["-e", &format!("POSIX path of (choose folder with prompt \"{t}\")")])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn pick_dir_native(title: &str) -> Option<String> {
    // 先 zenity 后 kdialog，都没有就返回 None（前端会提示手动填路径）
    if let Ok(out) = std::process::Command::new("zenity")
        .args(["--file-selection", "--directory", &format!("--title={title}")])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return Some(s);
        }
        return None;
    }
    let out = std::process::Command::new("kdialog")
        .args(["--getexistingdirectory", ".", "--title", title])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// `GET /api/update/pick-dir` —— 弹系统目录选择框，返回用户选的目录（取消则 dir = null）
pub async fn pick_dir(
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    // 标题转成 owned：blocking 闭包要 'static 生命周期
    let title = q.get("title").cloned().unwrap_or_else(|| "Choose a folder".into());
    let picked = crate::api::blocking(move || Ok(pick_dir_native(&title))).await;
    match picked {
        Ok(Some(dir)) => Json(json!({ "success": true, "dir": dir })),
        Ok(None) => Json(json!({ "success": true, "dir": null, "canceled": true })),
        Err(e) => Json(json!({ "success": false, "message": e.message })),
    }
}

/// GET /api/update/dirs —— 常用保存位置候选（跨平台）
///
/// 目录选择是用户的事：把「临时目录 / 桌面 / 下载 / 文档」列出来让他挑一个。
/// 项目里没有系统级文件夹选择对话框（Tauri 的 dialog 只在桌面壳可用，web 版没有），
/// 所以走应用内弹窗 + 允许手填任意路径。
pub async fn dirs() -> Json<Value> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    let home = std::path::PathBuf::from(home);
    let temp_dir = std::env::temp_dir();
    let default_dir = temp_dir.join("dbmind-update");
    let p = |x: std::path::PathBuf| x.to_string_lossy().to_string();
    Json(json!({
        "temp": p(default_dir.clone()),
        "tempRaw": p(temp_dir),
        "desktop": p(home.join("Desktop")),
        "downloads": p(home.join("Downloads")),
        "documents": p(home.join("Documents")),
        "home": p(home),
    }))
}

/// GET /api/update/progress —— 下载进度
pub async fn progress() -> Json<Value> {
    let p = progress_cell().lock().unwrap();
    let elapsed = p.started.map(|s| s.elapsed().as_secs()).unwrap_or(0);
    Json(json!({
        "status": p.status,
        "received": p.received,
        "total": p.total,
        "speed": p.speed,
        "elapsed": elapsed,
        "error": p.error,
        "installer": p.installer,
        "version": p.version,
        "proxy": p.proxy,
    }))
}