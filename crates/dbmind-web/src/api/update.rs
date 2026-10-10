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
    /// 过程说明（换源 / 续传 / 第几次重试）。**不是错误** —— 前端画在进度条下方，
    /// 让"怎么变慢了、怎么又重新开始了"有解释，而不是看起来像卡住。
    note: Option<String>,
    /// 实际在用的下载地址（可能已切到镜像），失败时可复制去手动下载
    url: Option<String>,
    /// 下载完是否用 GitHub 官方 sha256 校验过（有 digest 时才有机会为 true）
    verified: bool,
    /// 走的是镜像、而这个版本又没有官方 digest ⇒ 内容无法自证，界面要如实提示
    unverified: bool,
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

/// 构造一个带代理（若系统代理可用）的 ureq Agent。
///
/// `timeout_read` 是「卡住」与「慢」的分水岭：只设总超时的话，网络中断或黑洞连接会让一个
/// read 一直阻塞到总超时（1800 秒）—— 界面上就是永远停在某个百分比、看着像死机。
/// 单次读超时一设，停滞 30 秒即报错，上层才好自动重试 / 续传 / 换源。
fn agent() -> ureq::Agent {
    let builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30));
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

/// 挑中的安装包：地址、文件名、大小，以及 GitHub 官方算出的 sha256（`assets[].digest`）。
///
/// **为什么盯着 digest**：真机实测国内直连 `github.com/.../releases/download` 是**完全不通用**
/// （21 秒超时、0 字节），只能走镜像；而镜像能替换包内容。好在 GitHub 的 API 响应里带着它
/// 自己算的 sha256，而 `api.github.com` 直连是通的 —— 于是"镜像下字节 + 官方值校验"
/// 既不用逼用户开代理，也不用把安装包交给未知中间人。这是本项目允许默认使用镜像的前提。
#[derive(Debug, Clone)]
struct Installer {
    url: String,
    name: String,
    size: u64,
    /// 形如 `sha256:<64 hex>`；早期资源可能没有（那就不校验，并如实告诉用户）
    sha256: Option<String>,
}

/// 从发行版里挑出最合适的安装包（优先级见 `asset_score`）
fn pick_installer(v: &Value) -> Option<Installer> {
    let mut target: Option<Installer> = None;
    for asset in v["assets"].as_array()? {
        let name = asset["name"].as_str().unwrap_or("");
        let url = asset["browser_download_url"].as_str().unwrap_or("");
        if url.is_empty() {
            continue;
        }
        let score = asset_score(name);
        let best = target.as_ref().map(|i| asset_score(&i.name)).unwrap_or(0);
        if score > 0 && score > best {
            target = Some(Installer {
                url: url.to_string(),
                name: name.to_string(),
                size: asset["size"].as_u64().unwrap_or(0),
                sha256: asset["digest"]
                    .as_str()
                    .and_then(|d| d.strip_prefix("sha256:"))
                    .map(|h| h.trim().to_ascii_lowercase())
                    .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())),
            });
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
            let inst = pick_installer(&v);
            let download = inst.as_ref().map(|i| i.url.clone()).unwrap_or_default();
            Json(json!({
                "success": true,
                "current": current,
                "latest": latest,
                "hasNew": has_new,
                "name": v["name"].as_str().unwrap_or(""),
                "notes": v["body"].as_str().unwrap_or(""),
                "publishedAt": v["published_at"].as_str().unwrap_or(""),
                "download": download,
                // 安装包信息：文件名/大小/官方 sha256（有就说明这个版本能被校验）
                "assetName": inst.as_ref().map(|i| i.name.clone()).unwrap_or_default(),
                "assetSize": inst.as_ref().map(|i| i.size).unwrap_or(0),
                "assetSha256": inst.as_ref().and_then(|i| i.sha256.clone()),
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
            let inst = pick_installer(&v).ok_or_else(|| {
                DbMindError::new(ErrorCode::Internal, "最新发行版没有可下载的安装包（只有源码包）")
            })?;
            Ok::<_, DbMindError>((inst, tag))
        });
        let (inst, tag) = match picked {
            Ok(x) => x,
            Err(e) => {
                let mut p = cell.lock().unwrap();
                p.status = "failed".into();
                p.error = Some(e.message);
                return;
            }
        };
        let dest = resolve_dir(want_dir.as_deref()).join(&inst.name);
        if let Some(dir) = dest.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                let mut p = cell.lock().unwrap();
                p.status = "failed".into();
                p.error = Some(format!("创建临时目录失败：{e}"));
                return;
            }
        }
        match download_with_progress(&inst, &dest, &cell) {
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
/// 自动手段全失败后给用户的手动兜底地址
const RELEASE_PAGE: &str = "https://github.com/rick-works/dbmind/releases";

/// 默认镜像前缀，按实测速度排序（2026-10 国内直连、无代理）：
///
/// | 源 | 结果 |
/// |---|---|
/// | 直连 github.com/releases/download | **不通**（21s 超时、0 字节） |
/// | ghproxy.net | 可用，~220 KB/s |
/// | gh-proxy.com | 可用，~74 KB/s |
/// | ghfast.top / gh.llkk.cc / github.moeyy.xyz | 不通 |
///
/// 之所以敢默认开：字节流可以被镜像替换，但**sha256 来自 GitHub API**（直连可达），
/// 下完一定校验（见 `download_with_progress`）—— 中间人换不了 API 的响应。
/// `DBMIND_UPDATE_MIRRORS` 可整表替换（逗号分隔，`<前缀><原地址>` 或含 `{url}` 的模板），
/// 设成 `none` 则完全只用直连。
const DEFAULT_MIRRORS: &[&str] = &["https://ghproxy.net/", "https://gh-proxy.com/"];

/// 生效的镜像前缀表（环境变量优先于内置默认）
fn mirror_prefixes() -> Vec<String> {
    match std::env::var("DBMIND_UPDATE_MIRRORS") {
        Ok(v) => {
            let v = v.trim();
            if v.eq_ignore_ascii_case("none") || v.is_empty() {
                Vec::new()
            } else {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
        }
        Err(_) => DEFAULT_MIRRORS.iter().map(|s| s.to_string()).collect(),
    }
}

/// 下载源候选：**直连永远第一**，其后按镜像表拼接；逐个试，直连成功就轮不到镜像。
fn download_sources(url: &str) -> Vec<String> {
    let mut out = vec![url.to_string()];
    for prefix in mirror_prefixes() {
        out.push(if prefix.contains("{url}") {
            prefix.replace("{url}", url)
        } else {
            format!("{prefix}{url}")
        });
    }
    out
}

/// 从校验工具的输出里挑出 sha256。
///
/// 三种工具的格式都不一样，而且**不能被路径里的字符骗到**（那是安全校验，宁可判失败也不能认错）：
/// - Windows `certutil -hashfile x SHA256`：哈希单独一行，老版本还会分段加空格；
/// - macOS `shasum -a 256 x` / Linux `sha256sum x`：`<哈希>  <文件名>`，文件名可能带十六进制字符。
///
/// 所以先按"整行只含十六进制与空格"严格匹配（certutil 那条），再退回"按空白切词、取长度 64 的纯十六进制词"。
fn parse_sha256(out: &str) -> Option<String> {
    let strict = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_hexdigit() || c == ' '))
        .map(|l| l.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>())
        .find(|l| l.len() == 64);
    if let Some(h) = strict {
        return Some(h.to_ascii_lowercase());
    }
    out.lines()
        .flat_map(|l| l.split_whitespace())
        .find(|tok| tok.len() == 64 && tok.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|tok| tok.to_ascii_lowercase())
}

/// 从 URL 里抠出主机名，用于进度说明（"正在从 ghproxy.net 下载"比"从第 2 个源下载"有用）
fn host_of(url: &str) -> String {
    url.split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

/// 用**系统自带**工具算 sha256：Windows `certutil`、macOS `shasum`、Linux `sha256sum`。
///
/// 为什么不引 sha2 crate：这条链路只在下载完成后跑一次，而把依赖面控制住比省一次进程开销
/// 更重要（内核刻意不引网络依赖也是同一个考虑）。三种工具在各自平台上都必有。
fn sha256_file(path: &std::path::Path) -> dbmind_core::Result<String> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("certutil");
        c.args(["-hashfile", &path.to_string_lossy(), "SHA256"]);
        dbmind_core::hide_console(&mut c);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("shasum");
        c.args(["-a", "256", &path.to_string_lossy()]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("sha256sum");
        c.arg(path.to_string_lossy().as_ref());
        c
    };
    let out = cmd
        .output()
        .map_err(|e| DbMindError::new(ErrorCode::Internal, format!("调用系统校验工具失败：{e}")))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    parse_sha256(&text)
        .ok_or_else(|| {
            DbMindError::new(
                ErrorCode::Internal,
                format!("无法解析校验结果：{}", text.trim().chars().take(160).collect::<String>()),
            )
        })
}

/// 断点续传用的临时文件（下完才改名成最终安装包）。
///
/// 直接写最终文件名的话，中途断掉会留下一个「看着像装好了」的半个包 ——
/// 用户双击只会得到一句莫名其妙的错误。`.part` 一眼就知道是没下完的。
fn part_path(dest: &std::path::Path) -> std::path::PathBuf {
    let mut s = dest.as_os_str().to_os_string();
    s.push(".part");
    std::path::PathBuf::from(s)
}

fn human_bytes(n: u64) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    if n < 1024 * 1024 {
        return format!("{:.0} KB", n as f64 / 1024.0);
    }
    format!("{:.1} MB", n as f64 / 1024.0 / 1024.0)
}

/// 下载安装包：**续传 + 重试 + 换源**，全部失败才报错。
///
/// 为什么值得这么绕：安装包几十 MB，而「下到一半断了 / 慢得像停了」是常态，
/// 偏偏这时候机器上往往**没有代理**（用户报的正是这个场景）。于是：
///   1. 先写 `<安装包>.part`，成功才改名 —— 中断不留半个"安装包"；
///   2. 重试带 `Range` 续传（回 206 就接着下，回 200 就从头来）；
///   3. 停滞 30 秒由 `timeout_read` 触发报错，不会挂到总超时；
///   4. 大文件开局 15 秒还拿不到 512 KB，判定"这个源太慢"，直接换下一个源。
fn download_with_progress(
    inst: &Installer,
    dest: &std::path::Path,
    cell: &Arc<std::sync::Mutex<Progress>>,
) -> dbmind_core::Result<()> {
    let sources = download_sources(&inst.url);
    let part = part_path(dest);
    let mut errs: Vec<String> = Vec::new();
    for (si, src) in sources.iter().enumerate() {
        let host = host_of(src);
        for attempt in 1..=2u32 {
            let done = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            let note = if si > 0 {
                Some(if done > 0 {
                    format!("直连不通，已改用镜像 {host}，并从已下载的 {} 继续", human_bytes(done))
                } else {
                    format!("直连不通，已改用镜像 {host}")
                })
            } else if attempt > 1 {
                Some(format!("第 {attempt} 次重试（已下载 {}，接着下）", human_bytes(done)))
            } else if done > 0 {
                Some(format!("从已下载的 {} 继续", human_bytes(done)))
            } else {
                // 别让用户对着一动不动的 0% 猜：明说在连哪个源
                Some(format!("正在连接下载源 {host} …"))
            };
            cell.lock().unwrap().note = note;
            let before = done;
            // 慢速判定只在**后面还有别的源可试**时生效：否则一个 220 KB/s 能用的镜像
            // 偶尔掉到 30 KB/s 就被判"太慢"掐掉，等于把本来能下完的活干成失败。
            let can_switch = si + 1 < sources.len();
            match download_once(src, &part, cell, can_switch) {
                Ok(()) => {
                    // 校验：**这一步是允许默认走镜像的前提**。sha256 来自 GitHub API
                    // （直连可达），镜像换不了它；对不上就直接丢弃，绝不去执行一个可疑的安装包。
                    if let Some(want) = &inst.sha256 {
                        let got = sha256_file(&part)?;
                        if !got.eq_ignore_ascii_case(want) {
                            let _ = std::fs::remove_file(&part);
                            return Err(DbMindError::new(
                                ErrorCode::Internal,
                                format!(
                                    "安装包校验失败（期望 {want}，实得 {got}），文件已丢弃。\
                                     可能是下载源被篡改或传输损坏，请换网络重试"
                                ),
                            ));
                        }
                        cell.lock().unwrap().verified = true;
                    } else if si > 0 {
                        // 走镜像又没官方值可对 ⇒ 内容无法自证，如实标记（界面会提示）
                        cell.lock().unwrap().unverified = true;
                    }
                    std::fs::rename(&part, dest).map_err(|e| {
                        DbMindError::new(ErrorCode::Internal, format!("保存安装包失败：{e}"))
                    })?;
                    return Ok(());
                }
                Err(e) => {
                    let after = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
                    tracing::warn!(
                        target: "dbmind::update",
                        url = %src, attempt, error = %e.message, "安装包下载失败"
                    );
                    errs.push(e.message);
                    // 一个字节都没拿到 ⇒ 这个源根本连不上（被墙 / 黑洞），同源重试纯属浪费时间，
                    // 直接换下一个源；只有"下着下着断了"才值得同源重试。
                    if after <= before {
                        break;
                    }
                    // 有进度却断了：多半是网络抖动，同源再试一次（带上 Range 续传），稍等避开瞬时故障
                    if attempt == 1 {
                        std::thread::sleep(Duration::from_secs(3));
                    }
                }
            }
        }
    }
    {
        let mut p = cell.lock().unwrap();
        p.note = None;
        p.url = Some(inst.url.clone());
    }
    Err(DbMindError::new(
        ErrorCode::Internal,
        format!(
            "在线更新下载失败：{}。\n可改用：① 启动代理后重试（软件会自动使用系统代理）；\
             ② 用环境变量 DBMIND_UPDATE_MIRRORS 指定自己信任的镜像；\
             ③ 直接到发布页手动下载 {RELEASE_PAGE}",
            errs.join("；")
        ),
    ))
}

/// 下**一次**（某个源的一次尝试）：续传偏移、进度上报、慢速判定都在这里。
fn download_once(
    url: &str,
    part: &std::path::Path,
    cell: &Arc<std::sync::Mutex<Progress>>,
    // 这个源后面还有别的源可换吗？没有的话就不做"慢速掐断"（宁可慢，也别失败）
    allow_slow_abort: bool,
) -> dbmind_core::Result<()> {
    use std::io::Write;
    let offset = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    let mut req = agent()
        .get(url)
        .set("User-Agent", "dbmind-updater")
        .timeout(Duration::from_secs(1800));
    if let Some(tk) = github_token() {
        req = req.set("Authorization", &format!("Bearer {tk}"));
    }
    if offset > 0 {
        req = req.set("Range", &format!("bytes={offset}-"));
    }
    let resp = req.call().map_err(|e| {
        DbMindError::new(
            ErrorCode::Internal,
            format!("连接下载地址失败：{e}（若在使用代理，请确认代理程序已启动）"),
        )
    })?;
    // 206 = 服务端认了 Range（接着下）；200 = 不认，只能从零重来
    let resuming = resp.status() == 206 && offset > 0;
    if offset > 0 && !resuming {
        // 不续传却把新数据追加到旧文件后面，会得到一个坏包 —— 先清掉
        let _ = std::fs::remove_file(part);
    }
    let total = if resuming {
        // Content-Range: bytes 100-999/1000 → 取斜杠后的总长度
        resp.header("Content-Range")
            .and_then(|v| v.rsplit('/').next().map(str::to_string))
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(0)
    } else {
        resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0)
    };
    let base = if resuming { offset } else { 0 };
    {
        let mut p = cell.lock().unwrap();
        p.url = Some(url.to_string());
        p.total = total;
        p.received = base;
        if base > 0 {
            p.note = Some(format!("从已下载的 {} 继续", human_bytes(base)));
        }
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true);
    if resuming {
        opts.append(true);
    } else {
        opts.write(true);
    }
    let mut file = opts
        .open(part)
        .map_err(|e| DbMindError::new(ErrorCode::Internal, format!("写安装包失败：{e}")))?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut got: u64 = 0;
    let mut last_report = Instant::now();
    let attempt_started = Instant::now();
    // 只有大文件才判"慢"：小文件按字节数判会把正常慢速误判成故障
    let judge_slow = allow_slow_abort && (total == 0 || total > 4 * 1024 * 1024);
    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            DbMindError::new(
                ErrorCode::Internal,
                format!("下载中断（已下 {}）：{e}", human_bytes(base + got)),
            )
        })?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| DbMindError::new(ErrorCode::Internal, format!("写安装包失败：{e}")))?;
        got += n as u64;
        // 每 200ms 上报一次（进度条要顺，又不能频繁加锁）
        if last_report.elapsed() >= Duration::from_millis(200) {
            let secs = cell.lock().unwrap().started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.001);
            let mut p = cell.lock().unwrap();
            p.received = base + got;
            p.speed = (p.received as f64 / secs.max(0.001)) as u64;
            last_report = Instant::now();
        }
        // 慢速判定：开局 15 秒还拿不到 512 KB，就别在这儿耗着了（换源比死等强）
        if judge_slow && got < 512 * 1024 && attempt_started.elapsed() > Duration::from_secs(15) {
            return Err(DbMindError::new(
                ErrorCode::Internal,
                format!("下载速度过慢（15 秒仅 {}），已换下一个下载源", human_bytes(base + got)),
            ));
        }
    }
    {
        let mut p = cell.lock().unwrap();
        p.received = base + got;
        p.speed = 0;
    }
    if total > 0 && base + got + 1024 < total {
        return Err(DbMindError::new(
            ErrorCode::Internal,
            format!("安装包不完整（{}/{} 字节）", base + got, total),
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
        "note": p.note,
        "url": p.url,
        "verified": p.verified,
        "unverified": p.unverified,
        "installer": p.installer,
        "version": p.version,
        "proxy": p.proxy,
    }))
}

/// POST /api/update/dismiss —— 清掉**已结束**（done / failed）的任务状态。
///
/// 为什么必须有它：状态只活在进程里，而前端只在 `running` 时才该弹进度窗。
/// 失败后如果不清，用户再点更新图标只会反复弹回同一个报错页 —— 必须重启软件
/// 才能恢复（真机踩过）。运行中的任务**不能**清，否则会把正在下载的进度抹掉。
pub async fn dismiss() -> Json<Value> {
    let mut p = progress_cell().lock().unwrap();
    if p.status == "running" {
        return Json(json!({ "success": false, "message": "下载仍在进行中" }));
    }
    *p = Progress {
        status: "idle".into(),
        ..Default::default()
    };
    Json(json!({ "success": true }))
}