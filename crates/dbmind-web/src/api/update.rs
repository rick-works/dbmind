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
    /// 用户点了「换个更快的源」：下载循环看到就退出（保留已下载的 .part，随后换源续传）
    cancel: bool,
    /// 当前用的是第几个源（0 基）、一共几个源；界面显示"源 2/4"
    source_index: usize,
    source_total: usize,
    /// 当前这条源走不走代理（末尾那条"直连不走代理"是 false）
    using_proxy: bool,
}

/// 低于这个速度且持续 20 秒以上，就断开重连（新连接往往能换到更好的线路 / 节点）
const SLOW_FLOOR: u64 = 150 * 1024;

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
fn agent(use_proxy: bool) -> ureq::Agent {
    build_agent(use_proxy, 30)
}

/// 按用途构造客户端：`read_secs` 是单次读超时。
///
/// 测速必须用**很短**的读超时（2 秒）：它沿用的是下载那条通道的话，一个不响应的源
/// 会把探测拖到 30 秒，四个源加起来让用户白等十几秒（真机就是这样，看着像卡住）。
fn build_agent(use_proxy: bool, read_secs: u64) -> ureq::Agent {
    let builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10).min(Duration::from_secs(read_secs)))
        .timeout_read(Duration::from_secs(read_secs));
    if !use_proxy {
        // 显式不走代理：用户的代理节点有时**比直连还慢**（实测同机 120KB/s vs 3.8MB/s），
        // 所以源列表末尾留一条直连的路，用户点"换源"能轮到它。
        return builder.build();
    }
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
    let mut req = agent(true)
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
            source_index: req.source_index.unwrap_or(0),
            source_total: 0,
            ..Default::default()
        };
    }

    let cell = progress_cell().clone();
    let want_dir = req.dir.clone();
    let start_index = req.source_index.unwrap_or(0);
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
        match download_with_progress(&inst, &dest, &cell, start_index) {
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
    /// 从第几个下载源开始（用户点「换个更快的源」时传当前源 +1）。
    /// 仍带官方 sha256 校验，且从已下载的 .part 续传。
    #[serde(default)]
    pub source_index: Option<usize>,
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

/// 一个候选下载源。`use_proxy` 单独标出来：代理节点有时比直连慢得多，
/// 所以除了"直连 / 镜像（都走代理）"，列表末尾还留一条**直连不走代理**的路。
#[derive(Clone)]
struct Source {
    url: String,
    use_proxy: bool,
}

/// 源列表：直连（走代理）→ 各镜像（走代理）→ 直连（**不走代理**）。
///
/// 顺序即优先级：正常情况下第一条就成了；失败或过慢才依次往后。
/// 最后那条"不走代理"只在代理这条路有问题时才有意义，所以放最后。
fn build_sources(url: &str) -> Vec<Source> {
    let mut out = vec![Source {
        url: url.to_string(),
        use_proxy: true,
    }];
    for prefix in mirror_prefixes() {
        let u = if prefix.contains("{url}") {
            prefix.replace("{url}", url)
        } else {
            format!("{prefix}{url}")
        };
        out.push(Source { url: u, use_proxy: true });
    }
    out.push(Source {
        url: url.to_string(),
        use_proxy: false,
    });
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

/// 给一个源"测速"：只取前 256 KB，返回近似速度（字节/秒）；失败返回 None。
///
/// 为什么必须测：四个候选（直连经代理 / 各镜像 / 直连不走代理）里哪个快，取决于当时的网络、
/// 代理节点、运营商 —— 只有现场量才知道。让用户自己去点"换个源"是把这件事推给了他。
fn probe_source(src: &Source) -> Option<u64> {
    let deadline = Instant::now();
    let resp = build_agent(src.use_proxy, 2)
        .get(&src.url)
        .set("User-Agent", "dbmind-updater")
        .set("Range", "bytes=0-393215")
        .timeout(Duration::from_secs(3))
        .call()
        .ok()?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 64 * 1024];
    let mut got: u64 = 0;
    // **只在收到第一个字节之后才计时**：把建连 / TLS 握手算进去的话，链路很快但握手慢的源
    // 会被误判成"慢"（真机：探到 197 KB/s，真下起来 4 MB/s）。
    let mut t_first: Option<Instant> = None;
    while got < 384 * 1024 {
        // 硬上限：再慢就不再等了（测速只需知道"谁快"，不需要跑完）
        if deadline.elapsed() > Duration::from_secs(3) {
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if t_first.is_none() {
                    t_first = Some(Instant::now());
                }
                got += n as u64;
            }
            Err(_) => break,
        }
    }
    let secs = t_first?.elapsed().as_secs_f64();
    if got == 0 || secs <= 0.0 {
        return None;
    }
    Some((got as f64 / secs) as u64)
}

/// 并发给所有候选源测速，返回**按"探通没探通 + 快慢"排好序**的源列表。
///
/// 并发而不是一个一个试：串行最坏要等 4×4 秒，用户会觉得"点了没反应"。
/// 代价是所有源各多下 256 KB（约 1 MB），换来的是"不用挑、直接走最快的路"。
///
/// **为什么返回整份排序，而不是只挑最快的那个**：只挑最快时，一旦**所有**源都没探通
/// （免费镜像被限流、代理没装、直连被墙 —— 真机就撞上过），调用方只能退回"原顺序"，
/// 而原顺序的头一条恰恰是**已知不通**的 github 直连：用户盯着 0 字节干等十几秒才轮到镜像。
/// 排好序之后，探通的源必定排在没探通的前面（同样探通则快的在前），
/// 于是"能下就先下、下不动再退回去挨个试"，不会再把时间浪费在明知不通的路上。
fn rank_sources(sources: &[Source]) -> Vec<Source> {
    let handles: Vec<_> = sources
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, s)| std::thread::spawn(move || (i, probe_source(&s), s)))
        .collect();
    let mut probed: Vec<(usize, Option<u64>, Source)> = handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .collect();
    // 探通的在前（按速度降序），没探通的在后（保持原顺序，便于排查时看"原定优先级"）
    probed.sort_by(|a, b| match (a.1, b.1) {
        (Some(x), Some(y)) => y.cmp(&x).then(a.0.cmp(&b.0)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });
    probed.into_iter().map(|(_, _, s)| s).collect()
}

/// 下载安装包：**续传 + 重试 + 自动选源 + 慢连接重连**，全部失败才报错。
///
/// 为什么值得这么绕：安装包几十 MB，而「下到一半断了 / 慢得像停了」是常态，于是：
///   1. 先写 `<安装包>.part`，成功才改名 —— 中断不留半个"安装包"；
///   2. 重试带 `Range` 续传（回 206 就接着下，回 200 就从头来）；
///   3. 停滞 30 秒由 `timeout_read` 触发报错，不会挂到总超时；
///   4. **慢而不死也要管**：速度低于 `SLOW_FLOOR` 持续 20 秒，主动断开重连 ——
///      长连接会在代理链路上悄悄劣化（实测同一条代理：旧连接 120 KB/s、新连接 3 MB/s），
///      换条连接往往比等它自己恢复快得多；
///   5. 用户点「换个更快的源」时从下一个源开始（`start_index`），绕一圈轮换。
fn download_with_progress(
    inst: &Installer,
    dest: &std::path::Path,
    cell: &Arc<std::sync::Mutex<Progress>>,
    start_index: usize,
) -> dbmind_core::Result<()> {
    let all = build_sources(&inst.url);
    // start_index 保留给排查用（0 = 正常；非 0 = 从第 n 个源起，跳过前面的）
    let start = start_index % all.len();
    // **自动选源**：现场并发测速，探通的排前面（最快的在最前），没探通的留在后面兜底。
    // 用户不需要、也不应该自己挑源 —— 哪个快只有现场量得准。
    // 测速要 3 秒左右，这段时间进度条停在 0 B：一句话都不说，用户看到的就是"点了没反应"
    //（真机反馈）。所以只补一条等待提示，连上第一个字节后立刻清掉（见 `download_once`）。
    cell.lock().unwrap().note = Some("正在选择下载源…".to_string());
    let sources = rank_sources(&all);
    let source_total = sources.len();
    let _ = start;
    let part = part_path(dest);
    // 上一次可能已经把整包下完了（"取消"来得太晚、改名失败……），留下一个**完整**的 `.part`。
    // 这时再带 `Range: bytes=<全长>-` 去要，服务端一律回 **416**（区间越界）—— 四个源全失败，
    // 用户看到的是一堵 416（真机踩过）。所以先看本地这份够不够大：够就先校验，过了直接用，
    // 没过（或尺寸对不上）就删掉重下。
    if inst.size > 0 {
        if let Ok(meta) = std::fs::metadata(&part) {
            if meta.len() >= inst.size {
                let ok = match &inst.sha256 {
                    Some(want) => sha256_file(&part)
                        .map(|got| got.eq_ignore_ascii_case(want))
                        .unwrap_or(false),
                    None => meta.len() == inst.size,
                };
                if ok {
                    std::fs::rename(&part, dest).map_err(|e| {
                        DbMindError::new(ErrorCode::Internal, format!("保存安装包失败：{e}"))
                    })?;
                    if inst.sha256.is_some() {
                        cell.lock().unwrap().verified = true;
                    }
                    return Ok(());
                }
                tracing::warn!(target: "dbmind::update", "本地 .part 尺寸够但校验不过，丢弃重下");
                let _ = std::fs::remove_file(&part);
            }
        }
    }
    let mut errs: Vec<String> = Vec::new();
    for (si, src) in sources.iter().enumerate() {
        // 取消要**立刻**停：否则每个源还要再试两次、各等 3 秒，用户会以为"点了取消没反应"
        if cell.lock().unwrap().cancel {
            return Err(DbMindError::new(ErrorCode::Internal, "已取消当前下载（已下载的部分保留）".to_string()));
        }
        // 顺序是"绕一圈"后的：只有 si==0 才意味着还在原定起点上
        let is_start = si == 0;
        let host = host_of(&src.url);
        let via = if src.use_proxy { "" } else { "（不走代理）" };
        for attempt in 1..=2u32 {
            let done = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            let note = if !is_start {
                Some(if done > 0 {
                    format!("已换到 {host}{via}，并从已下载的 {} 继续", human_bytes(done))
                } else {
                    format!("已换到下载源 {host}{via}")
                })
            } else if attempt > 1 {
                Some(format!("第 {attempt} 次重试（已下载 {}，接着下）", human_bytes(done)))
            } else if done > 0 {
                Some(format!("从已下载的 {} 继续", human_bytes(done)))
            } else {
                // 一上来、一个字节都还没有：给一条**等待型**提示。正常网络下它一闪而过，
                // 但这个源其实连不上时要等满一次连接超时（10 秒）—— 那 10 秒界面若一个字都没有，
                // 用户判定就是"点了没反应"（真机反馈）。连上后由 `download_once` 立刻清掉。
                Some(format!("正在连接 {host}{via}…"))
            };
            {
                let mut p = cell.lock().unwrap();
                p.note = note;
                p.url = Some(src.url.clone());
                p.source_index = (start + si) % source_total;
                p.source_total = source_total;
                p.using_proxy = src.use_proxy;
            }
            let before = done;
            // 允许"慢速重连"的条件：同源还有一次重试机会，或后面还有别的源。
            // 最后一条路上不能再掐（宁可慢，也别把一个能下完的活干成失败）。
            let can_retry = attempt == 1 || si + 1 < sources.len();
            match download_once(&src.url, src.use_proxy, &part, cell, can_retry) {
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
                    } else if src.url != inst.url {
                        // 用的不是官方地址（多半是镜像）又没官方 digest 可对 ⇒ 内容无法自证，如实标记。
                        // 注意判据是**实际地址**：自动选源之后，第一个用的可能就是镜像。
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
                        url = %src.url, attempt, error = %e.message, "安装包下载失败"
                    );
                    // 先取出两个判定再 push（message 是 String，push 会把它 move 走）
                    let slow_abort = e.message.contains("速度过慢");
                    let canceled = e.message.starts_with("已取消");
                    errs.push(e.message);
                    // 取消：整个任务到此为止，不必再换源重试
                    if canceled {
                        return Err(DbMindError::new(ErrorCode::Internal, "已取消当前下载（已下载的部分保留）".to_string()));
                    }
                    // 一个字节都没拿到 ⇒ 这个源根本连不上（被墙 / 黑洞），同源重试纯属浪费时间，
                    // 直接换下一个源；只有"下着下着断了"才值得同源重试。
                    if after <= before {
                        break;
                    }
                    // 有进度却断了 / 太慢被主动断开：同源再试一次（带上 Range 续传）。
                    // 慢速重连**不等待**（等的就是新连接），真断了才稍等避开瞬时故障。
                    if attempt == 1 && !slow_abort {
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
    // 每个源的报错里常带着几百字符的签名 URL，全量拼进界面就是一堵墙 —— 只留前两条、各截断。
    let brief: Vec<String> = errs
        .iter()
        .take(2)
        .map(|e| {
            let one = e.split_whitespace().collect::<Vec<_>>().join(" ");
            if one.chars().count() > 120 {
                one.chars().take(120).collect::<String>() + "…"
            } else {
                one
            }
        })
        .collect();
    let more = if errs.len() > brief.len() {
        format!("（另有 {} 个下载源也失败）", errs.len() - brief.len())
    } else {
        String::new()
    };
    Err(DbMindError::new(
        ErrorCode::Internal,
        format!(
            "下载失败：{}{more}。\n可以点「重试」再来一次（已下载的部分会保留）；\
             或到发布页手动下载：{RELEASE_PAGE}",
            brief.join("；")
        ),
    ))
}

/// 下**一次**（某个源的一次尝试）：续传偏移、进度上报、慢速判定都在这里。
fn download_once(
    url: &str,
    use_proxy: bool,
    part: &std::path::Path,
    cell: &Arc<std::sync::Mutex<Progress>>,
    // 还允许再试一次吗？（允许才做慢速断开重连；最后一条路上宁可慢也不掐）
    can_retry: bool,
) -> dbmind_core::Result<()> {
    use std::io::Write;
    let offset = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    let mut req = agent(use_proxy)
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
    if resp.status() == 416 && offset > 0 {
        // 416 = 我这边要的区间越界（本地那份已经 >= 远端大小）。留着它只会让**每个源**都回 416，
        // 直接丢弃，让下一次尝试从 0 开始。
        let _ = std::fs::remove_file(part);
        return Err(DbMindError::new(
            ErrorCode::Internal,
            "本地已下载部分不小于远端文件，已丢弃并从零重下".to_string(),
        ));
    }
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
        } else if p.note.as_deref().map(|n| n.starts_with("正在")).unwrap_or(false) {
            // 连上了（响应头与长度都拿到了）：清掉"正在选择下载源 / 正在连接 X"这类**等待型**提示。
            // 判据用前缀："正在…"是等待（连上就没意义了），而"已换到…/第 N 次重试…/从已下载的…"
            // 是**解释型**，要留到这次下载结束 —— 它们回答的是"数字在动，可为什么我又在等"。
            p.note = None;
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
    // 速度用**最近 6 秒的滑窗**算，而不是"任务开始至今的平均"：
    // 平均值会骗人 —— 从 25 MB 处续传的任务头几秒显示成几百 KB/s，长连接劣化时又看不出"现在变慢了"。
    let mut window: std::collections::VecDeque<(Instant, u64)> = std::collections::VecDeque::new();
    window.push_back((Instant::now(), base));
    // 只有大文件才判"慢"：小文件按字节数判会把正常慢速误判成故障
    let judge_slow = can_retry && (total == 0 || total > 4 * 1024 * 1024);
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
            let now = Instant::now();
            let total_now = base + got;
            window.push_back((now, total_now));
            while window.len() > 2 && now.duration_since(window[0].0) > Duration::from_secs(6) {
                window.pop_front();
            }
            let (t0, b0) = window[0];
            let dt = now.duration_since(t0).as_secs_f64();
            let speed = if dt >= 0.5 { ((total_now - b0) as f64 / dt) as u64 } else { 0 };
            {
                let mut p = cell.lock().unwrap();
                p.received = total_now;
                p.speed = speed;
            }
            last_report = now;
            // 慢连接看门狗：**不是"开局慢"就换**，而是"一直慢"才断开重连（带 Range 续传）。
            // 长连接在代理链路上会悄悄劣化（实测同一条代理：旧连接 120 KB/s、新连接 3 MB/s），
            // 重连往往几秒就恢复 —— 比死等它自己好起来快得多，也不必从 0 重下。
            if judge_slow
                && dt >= 1.0
                && speed > 0
                && speed < SLOW_FLOOR
                && attempt_started.elapsed() > Duration::from_secs(20)
            {
                return Err(DbMindError::new(
                    ErrorCode::Internal,
                    format!("连接速度过慢（约 {}/秒），已断开重连并接着下", human_bytes(speed)),
                ));
            }
        }
        // 用户点了「换个更快的源」：立刻退出，保留 .part 让下一次从断点续传
        if cell.lock().unwrap().cancel {
            return Err(DbMindError::new(
                ErrorCode::Internal,
                format!("已取消当前下载（已下载的 {} 保留，换源后会接着下）", human_bytes(base + got)),
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
        "sourceIndex": p.source_index,
        "sourceTotal": p.source_total,
        "usingProxy": p.using_proxy,
        "installer": p.installer,
        "version": p.version,
        "proxy": p.proxy,
    }))
}

/// POST /api/update/cancel —— 取消正在进行的下载（用户要换源时用）。
///
/// 只**打个标记**，由下载循环在下一批数据时看到并退出：已经写的 `.part` 保留，
/// 所以"换个更快的源重试"能从已下载的位置接着下 —— 不必从头再来一遍几十兆。
pub async fn cancel() -> Json<Value> {
    let mut p = progress_cell().lock().unwrap();
    if p.status != "running" {
        return Json(json!({ "success": false, "message": "当前没有正在进行的下载" }));
    }
    p.cancel = true;
    p.note = Some("正在取消当前下载…".into());
    Json(json!({ "success": true }))
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