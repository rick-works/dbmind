//! DBmind 桌面壳（Tauri）。
//!
//! ## 为什么是「壳 + 本地 HTTP」，而不是 IPC 命令
//!
//! 前端整份代码都按 HTTP 契约写（`frontend/src/api/index.js` 的 `baseURL` 是空的相对路径），
//! 桌面版沿用这条链路，于是 Web 版与桌面版共享**同一份前端产物、同一套错误结构、同一份契约**。
//! 给桌面单独发明一套 IPC，等于把每个接口实现两遍，然后眼看着它们漂开 ——
//! 数据库客户端真正难对付的从来不是传输方式，而是"两套实现行为不一致"。
//! （内核侧 `dbmind_web::spawn_embedded` 的注释也是这个口径。）
//!
//! ## 端口怎么定
//!
//! 先探默认端口上是不是**已经有同族实例**在跑（用户可能先前用 `dbmind-web.exe` 起过，
//! 或者双击了两次图标）：
//!
//! - 是 → **复用它**，不再起第二份引擎。这不是省内存的问题：两份进程同时写同一份
//!   SQLite 元数据库，迟早出事。
//! - 否 → 在默认端口上起内嵌实例；端口被别人占了就往后找一个空闲的。
//!
//! 探测认的是 `GET /api/dbmind/health` 的响应内容，而不是"端口能连上" ——
//! 端口上蹲着别的程序时，必须**另找端口**而不是把窗口指向它。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use std::path::Path;

use sysinfo::System;
use tauri::{Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

/// 读 `java -version` 的主版本号（`17.0.20` → 17；`1.8.0_401` → 8）。
fn java_major(exe: &Path) -> Option<u32> {
    let mut cmd = std::process::Command::new(exe);
    dbmind_core::hide_console(&mut cmd); // GUI 子系统里不藏的话，探测版本会闪一个黑窗
    let out = cmd.arg("-version").output().ok()?;
    // java 把版本打进 stderr，别读 stdout
    let text = String::from_utf8_lossy(&out.stderr);
    let num = text.split("version \"").nth(1)?.split('"').next()?;
    let mut parts = num.split('.');
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        return parts.next()?.parse().ok();
    }
    Some(first)
}

/// 挑一个 Java 17+ 固定成 `DBMIND_JAVA`。
///
/// 为什么壳要管这件事：内核按 `DBMIND_JAVA` > `JAVA_HOME` > `PATH` 解析 Java，而
/// **双击启动**的桌面壳不经过 `scripts/start.ps1`（那里会挑 17+ 并临时设好环境变量），
/// 于是它用机器默认的 Java —— 很常见是 8 / 11，表现是「所有走 Java 宿主的数据源
/// （SQL Server / Mongo / Redis / ES / JDBC）都报 DBMIND-DRV-0002 驱动宿主未就绪」，
/// 而提示里看不出根因是版本。用户实测撞过，所以这里做一次主动选择。
///
/// 顺序：已有的 DBMIND_JAVA > exe 旁的 jre / jdk（便携版可自带）> JAVA_HOME >
/// 宿主用的 Java 可执行文件名：Windows 是 `java.exe`，其它平台是 `java`。
///
/// ⚠️ 这里（以及下面的 JAVA_HOME / PATH 查找）原先一律写死 `java.exe` —— 在 macOS / Linux
/// 上等于"永远找不到 Java"，连包自带的 `jre/` 也形同虚设。见 release-notes/v1.2.2.md。
fn java_bin_name() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// PATH > 常见安装目录。多个候选时取版本最高的那个。
fn ensure_java() {
    let java_bin = java_bin_name();

    if let Some(existing) = std::env::var_os("DBMIND_JAVA") {
        let path = PathBuf::from(existing);
        if path.is_file() {
            eprintln!("Java 宿主：沿用已有的 DBMIND_JAVA = {}", path.display());
            return;
        }
    }

    // 包**自带**的运行时（资源根下的 jre/ 或 jdk/）单独成一层：只要 ≥17 就直接用它，
    // **不**参与后面的"取版本最高"。
    //
    // 这一层是必须的：包自带 JRE 的用意就是"不依赖目标机器装了什么"，
    // 若还跟系统 Java 比版本，用户机器上有个更新的 Java 就会顶掉它，
    // 自带运行时等于白打（实测踩过：包里带 17、机器上有 25，壳选了 25）。
    //
    // 资源根由 `dbmind_core::paths::app_resource_roots` 给：Windows 在 exe 旁、
    // macOS 在 `.app/Contents/Resources/`、Linux 在 `usr/lib/<产品名>/`。
    let bundled_java =
        |sub: &str| dbmind_core::paths::find_resource_path(&format!("{sub}/bin/{java_bin}"));
    for sub in ["jre", "jdk"] {
        let Some(bundled) = bundled_java(sub) else {
            continue;
        };
        if let Some(major) = java_major(&bundled) {
            if major >= 17 {
                eprintln!("Java 宿主：使用自带的运行时 {}（版本 {major}）", bundled.display());
                std::env::set_var("DBMIND_JAVA", &bundled);
                return;
            }
            eprintln!("Java 宿主：自带的运行时版本过低（{major}），改为查找系统上的 Java 17+");
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    // 系统上找：JAVA_HOME / PATH / 常见安装目录 —— 这几处**取版本最高的那个**
    // 自带运行时版本不合规时，仍把它作为候选（至少有个可用的）
    for sub in ["jre", "jdk"] {
        if let Some(candidate) = bundled_java(sub) {
            candidates.push(candidate);
        }
    }
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let candidate = PathBuf::from(home).join("bin").join(java_bin);
        if candidate.is_file() {
            candidates.push(candidate);
        }
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for entry in std::env::split_paths(&path_var) {
            let candidate = entry.join(java_bin);
            if candidate.is_file() {
                candidates.push(candidate);
            }
        }
    }
    // 常见安装位置（**仅 Windows**）：JDK 装在这里时通常没有配进 JAVA_HOME / PATH。
    // macOS 的 JDK 在 /Library/Java/JavaVirtualMachines，`/usr/bin/java` 会挂过去，
    // 上面 PATH 那一条已经覆盖，所以这里不需要按平台再列一遍目录。
    if cfg!(windows) {
        for root in [
            "C:\\Program Files\\Java",
            "C:\\Program Files\\Eclipse Adoptium",
            "C:\\Program Files\\Microsoft\\jdk",
            "D:\\develop\\tools",
        ] {
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    let candidate = entry.path().join("bin").join(java_bin);
                    if candidate.is_file() {
                        candidates.push(candidate);
                    }
                }
            }
        }
    }

    let mut best: Option<(u32, PathBuf)> = None;
    for candidate in candidates {
        if let Some(major) = java_major(&candidate) {
            if major >= 17 && best.as_ref().map(|(m, _)| major > *m).unwrap_or(true) {
                best = Some((major, candidate));
            }
        }
    }

    match best {
        Some((major, path)) => {
            eprintln!("Java 宿主：选中 {}（版本 {major}）", path.display());
            // 进程内设置：内核 spawn java 时继承当前进程环境，所以对它同样有效。
            // 只在起内核之前调用一次，此时还没有别的线程读环境，安全。
            std::env::set_var("DBMIND_JAVA", &path);
        }
        None => eprintln!(
            "Java 宿主：没找到 Java 17+ —— 走 Java 宿主的数据源会报 DBMIND-DRV-0002，\
             可安装 JDK 17+ 或设置 DBMIND_JAVA 指向它"
        ),
    }
}

/// 退出时清场：把**本项目的**其它进程与 Java 宿主一并收掉。
///
/// 为什么需要它：桌面壳有两种运行形态 ——
///   · 自己起内嵌内核（同进程，壳退端口自然释放，无需处理）；
///   · **复用**了外部已经跑着的 `dbmind-web.exe`（多个壳共享一份引擎）。
///     后者的进程与端口**不会**随壳退出而消失。
/// 另外 Java 宿主（Mongo / Redis / ES / JDBC 的连接都靠它）是内核 spawn 出来的
/// **独立子进程**，壳一退就成了孤儿进程，白占内存。
///
/// 匹配口径刻意收窄，避免误伤：
///   · `dbmind-web.exe` —— 要求可执行文件路径**与本程序在同一目录**；
///   · `java` —— 要求命令行里出现 `dbmind-agent`（我们四个宿主 jar 的名字特征），
///     这样别人家的 JVM 不会被牵连。
fn cleanup_child_processes() {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    // System::new_all() 一步拿全量进程信息：各版本 sysinfo 都有这个入口，
    // 而"按需刷新"的 API（RefreshKind / ProcessesToUpdate）跨版本差异很大（踩过 API 不存在的坑）。
    // 退出清场只跑一次，全量刷新那点开销无所谓。
    let sys = System::new_all();

    let me = std::process::id();
    let mut killed = 0usize;

    for (pid, process) in sys.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == me {
            continue;
        }

        let exe_path = process.exe();
        let cmd = process.cmd().iter().map(|a| a.to_string_lossy().to_lowercase()).collect::<Vec<_>>().join(" ");

        let is_our_web = process
            .name()
            .to_string_lossy()
            .eq_ignore_ascii_case(WEB_EXE_NAME)
            && match (&exe_dir, exe_path) {
                (Some(dir), Some(exe)) => exe.parent().map(|p| p == dir.as_path()).unwrap_or(false),
                _ => false,
            };
        let is_our_java = process.name().to_string_lossy().to_lowercase().contains("java")
            && cmd.contains("dbmind-agent");

        if is_our_web || is_our_java {
            let what = if is_our_web { "dbmind-web" } else { "java 宿主" };
            if process.kill() {
                eprintln!("退出清场：已结束 {what}（PID {pid_u32}）");
                killed += 1;
            } else {
                eprintln!("退出清场：结束 {what}（PID {pid_u32}）失败（可能已自行退出）");
            }
        }
    }

    if killed == 0 {
        eprintln!("退出清场：没有需要结束的其它进程");
    }
}

/// 与 Web 版一致的默认端口（前端 dev 代理也指这里，见 vite.config.mjs）。
const DEFAULT_PORT: u16 = 20361;
/// 默认端口被占用时，往后尝试的端口个数
const PORT_SCAN: u16 = 20;

/// 独立内核进程的文件名（退出清场按它匹配）。
///
/// 原先这里写死 `dbmind-web.exe` —— 在 mac / Linux 上**永远匹配不到**（那边的可执行文件
/// 没有 `.exe` 后缀），于是残留的内核进程不会被清掉，会一直占着默认端口；之后每次启动
/// 都"复用"它。这正是要按版本校验复用的另一个理由，但名字也该按平台写对。
#[cfg(windows)]
const WEB_EXE_NAME: &str = "dbmind-web.exe";
#[cfg(not(windows))]
const WEB_EXE_NAME: &str = "dbmind-web";
/// 等内嵌内核就绪的上限
const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// 一次极简的 HTTP 探测：确认该端口上跑的是 **DBmind**，而不只是"有东西在监听"。
///
/// 手写报文是为了不给壳添一个 HTTP 客户端依赖 —— 这里只需要一个 GET，
/// 而壳层依赖越多，编译越慢、供应链越长。
fn dbmind_health(port: u16) -> Option<String> {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).ok()?;
    let mut stream = stream;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
    let request = format!(
        "GET /api/dbmind/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut body = String::new();
    // 读超时**不当失败**：读超时只是"对端没关连接"，而我们靠 `Connection: close` 与
    // Content-Length 本来就能拿到完整正文 —— 用 `.ok()?` 会把已经读到的正文一起丢掉，
    // 于是明明健康的实例被判成"不是 DBmind"→ 不复用 → 平白多起一个内核。
    // 这里忽略读错误，只用已读到的内容判断。
    let _ = stream.read_to_string(&mut body);
    // 别用 `"ok":true` 硬匹配：不同 serde 版本/格式化方式会带空格
    if body.contains("\"ok\"") && body.contains("true") {
        Some(body)
    } else {
        None
    }
}

fn is_dbmind(port: u16) -> bool {
    dbmind_health(port).is_some()
}

/// 从 health 正文里取 `version`（老内核可能没这个字段 → None）。
///
/// 手工取而不引 `serde_json`：壳层刻意保持零依赖（编译快、供应链短），
/// 而 health 的正文格式是我们自己定的（`{"ok":true,"version":"1.2.20"}`）。
fn health_version(body: &str) -> Option<String> {
    // 匹配**完整前缀** `"version":"`，而不是裸 `"version"`：health 正文将来只要多一个
    // 名字里带 version 的键（`dbVersion` / `proxyVersion`…），裸匹配就会取到那个值 →
    // 与本机版本不符 → 判定「不复用」→ 用户开第二个窗口会平白多起一个内核。
    let key = "\"version\":\"";
    let rest = &body[body.find(key)? + key.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// 定下这次要连哪个端口，以及**要不要自己起内核**。
///
/// `allow_reuse` = 允许复用"已经在跑的另一个实例"的内核。复用要满足**两条**，缺一就自己起：
///  ① 端口上答 health 的确实是 DBmind；
///  ② **版本与本机这份一致**。
///
/// 为什么要看版本（mac 白屏的根因）：应用窗口指向的是 `http://127.0.0.1:<port>` 这个内嵌内核，
/// 而不是内嵌静态资源。若复用了**旧版本的常驻内核**——最常见的就是刚被更新脚本 `pkill` 掉、
/// 还没退干净的旧实例——轻则界面是旧版，重则它在窗口导航那一刻死掉：端口没人监听 →
/// **macOS 的 WKWebView 加载失败时什么都不画（纯白）**，而 Windows 的 WebView2 会画一张错误页。
/// 这就是"同样更新，只有 mac 白屏"的来源。
fn resolve_port(allow_reuse: bool) -> (u16, bool) {
    if allow_reuse {
        if let Some(body) = dbmind_health(DEFAULT_PORT) {
            let found = health_version(&body);
            // 没有 version 字段（很旧的内核）也**不复用**：宁可贵一点自己起一个，
            // 也不要把窗口挂在一个来路不明的内核上。
            if found.as_deref() == Some(env!("CARGO_PKG_VERSION")) {
                return (DEFAULT_PORT, false);
            }
            eprintln!(
                "端口 {DEFAULT_PORT} 上的内核版本是 {}（本机 {}）→ 不复用，另起一个",
                found.as_deref().unwrap_or("未知"),
                env!("CARGO_PKG_VERSION")
            );
        }
    }
    for port in DEFAULT_PORT..DEFAULT_PORT.saturating_add(PORT_SCAN) {
        if port_free(port) {
            return (port, true);
        }
    }
    // 全占满：仍把默认端口交给调用方，让 `spawn_embedded` 报出真实错误
    (DEFAULT_PORT, true)
}

/// 等内嵌内核就绪。
///
/// 不等的话，窗口会比后端先就绪，前端首屏会先拿到一次连接失败 ——
/// 它自己会重试，但用户看到的是"白屏闪一下"。通常几百毫秒就好。
/// 等内核就绪；**不打印任何东西**，由调用方决定说什么
/// （复用探活失败是预期内的分支，不该刷一条吓人的超时日志）。
fn wait_ready_until(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if is_dbmind(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    false
}

/// 复用别人内核时的探活窗口。
///
/// 只要确认"这一刻还在"就够了 —— 旧实例正在退出时（更新脚本 pkill 掉的那一个）会在几十毫秒内消失。
/// 这里若用满 `READY_TIMEOUT`，「复用失败 → 自己起一个内核」这条兜底路径就会白等十几秒，
/// 而且全程没有窗口（用户会以为卡死了）。
const REUSE_PROBE: Duration = Duration::from_secs(4);

fn wait_ready(port: u16) {
    if wait_ready_until(port, READY_TIMEOUT) {
        return;
    }
    // 超时不拦着开窗口：前端会显示"连不上后端"，比一个沉默的白窗口有用
    eprintln!("内核在 {READY_TIMEOUT:?} 内未就绪，仍打开窗口");
    // 但**要留下证据**：macOS 上这一刻窗口就是纯白，用户在应用里看不到任何提示，
    // 而壳的 stderr 在 .app 双击启动时没人看得到。写一份文件日志，
    // 下次"更新完白屏"可以直接看它（也可能只是内核起得慢，所以措辞要分清）。
    append_shell_log(&format!(
        "内核在 {READY_TIMEOUT:?} 内未就绪（端口 {port}）——窗口仍会打开；\
         若界面空白，请检查该端口是否有残留的旧内核，或前端产物 web/index.html 是否缺失"
    ));
}

/// 往 `~/.dbmind/logs/shell.log` 追加一行（失败就算了，绝不影响启动）。
///
/// 为什么需要它：macOS 上"内核没起来"的表现是**纯白窗口**，而壳的 stderr 在双击 .app 启动时
/// 无处可见；线上排查这个只能靠一个落盘的文件。桌面壳以前不写任何日志文件（只有 java 宿主写），
/// 这里补上最小的一条通道。
fn append_shell_log(line: &str) {
    use std::io::Write;
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) else {
        return;
    };
    let dir = std::path::PathBuf::from(home).join(".dbmind").join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("shell.log"))
    else {
        return;
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(file, "[{stamp}] {line}");
}

/// 前端已经确认「关闭」：置位后放行真正的关闭，不再拦截询问。
/// 用静态量而不是 Tauri 状态对象 —— 窗口事件回调里读它最省事，不依赖 app 句柄。
static ALLOW_CLOSE: AtomicBool = AtomicBool::new(false);

/// 把主窗口叫回来（托盘菜单「显示主窗口」与托盘左键都走它）
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// 页面回应「关闭窗口」时的选择：
///   close = 真的关闭（连带退出应用）；tray = 最小化到托盘；其它 = 取消（什么都不做）
#[tauri::command]
fn close_action(window: tauri::WebviewWindow, action: String) {
    match action.as_str() {
        "close" => {
            // 显式退出，而不是只 close 窗口：**有托盘时"关掉最后一个窗口"不会结束进程**
            // （托盘图标还在 → 应用继续活着，用户会以为没关掉）。走 app.exit 仍会触发
            // RunEvent::Exit，子进程清场逻辑不受影响。
            ALLOW_CLOSE.store(true, Ordering::SeqCst);
            window.app_handle().exit(0);
        }
        "tray" => {
            let _ = window.hide();
        }
        _ => {}
    }
}

fn main() {
    // 必须在起内核**之前**：这个选择是通过进程环境变量传给内核的
    ensure_java();

    let (mut port, mut need_spawn) = resolve_port(true);

    // 即便决定复用，也要**先确认这一刻它真的可用**：更新脚本会在同一时间把旧实例关掉，
    // 于是"刚才还有人答 health"到开窗那一刻可能已经失效。不等就开窗 → 窗口导航到死端口 →
    // macOS 上是纯白（WKWebView 加载失败不画任何东西），Windows 上是一张错误页。
    if !need_spawn {
        // 复用前先探活，但窗口要短：这一刻它可能正在退出（更新脚本恰好同时把旧内核关掉），
        // 确认"还在"就够了 —— 用满 15 秒会让兜底路径白等（全程没有窗口）。
        if wait_ready_until(port, REUSE_PROBE) {
            eprintln!("检测到 {port} 端口已有**同版本** DBmind 实例，复用它");
        } else {
            eprintln!("{port} 端口上的实例已不可用（可能正在退出）→ 改为自己起一个内核");
            let (fallback, _) = resolve_port(false);
            port = fallback;
            need_spawn = true;
        }
    }

    if need_spawn {
        // 库位置与 CLI 一致（默认 ~/.dbmind，可用 DBMIND_STORE 覆盖）：
        // 桌面版不另起一份数据，用户换壳不换数据。
        let store: Option<PathBuf> = std::env::var_os("DBMIND_STORE").map(PathBuf::from);
        if let Err(err) = dbmind_web::spawn_embedded(port, store) {
            eprintln!("内嵌内核启动失败：{err}");
        }
        wait_ready(port);
    }

    let origin = format!("http://127.0.0.1:{port}");
    tauri::Builder::default()
        .setup(move |app| {
            // 在线更新要装新版时，安装器会覆盖正在运行的 exe —— 它会**强杀**本进程来腾出文件锁。
            // 强杀不走 RunEvent::Exit，退出清场（结束 java 宿主）就不会执行，那些 java 会变成孤儿
            // 留在后台。注册这个钩子：内核在开装前通知我们，我们走 app.exit(0) 优雅退出、清场照跑，
            // 随后安装器接管（Windows 的 /R、macOS 脚本里的 open 会把新版拉起来）。
            {
                let handle = app.handle().clone();
                dbmind_web::api::update::set_install_hook(Box::new(move || {
                    let _ = handle.exit(0);
                }));
            }

            // 窗口在这里建、而不是写在 tauri.conf.json 里：端口是**运行时**定的。
            let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(origin.parse()?))
                .title("DBmind")
                .inner_size(1440.0, 900.0)
                .min_inner_size(1024.0, 640.0)
                .center()
                // 页面加载结果打出来（重定向 stderr 就能拿到）：
                // "窗口开着"不等于"页面加载成功" —— 白窗口同样是开着的，
                // 而这类故障在现场只能靠猜。留一行日志，排查时有据可查。
                .on_page_load(|_window, payload| {
                    if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                        eprintln!("前端页面加载完成: {}", payload.url());
                    }
                });

            // 系统标题栏怎么处理，**必须按平台分**（踩过）：
            //  · Windows / Linux：整条标题栏都不要（decorations(false)）。拖动与最小化/最大化/
            //    关闭由页面自绘 —— 顶栏右侧那三个按钮（`.win-acts`），权限见 capabilities/default.json。
            //  · macOS：**不能**用 decorations(false)。那样连系统红绿灯也一起没了，而前端偏偏
            //    又因为 isMacShell 不去画自己的按钮（它的注释假设"macOS 保留系统红绿灯"），
            //    两头一凑：窗口既没有标题栏、也关不掉最小化不了（用户反馈"上面的菜单没有"）。
            //    改用 Overlay：红绿灯仍在、标题文字隐藏、内容铺到标题栏下面 ——
            //    顶栏那 84px 左边距（`.topbar.is-mac`）正是留给红绿灯的。
            //    ⚠️ Overlay 要求 decorations 保持 true（默认），所以这条分支**不能**再关装饰。
            // 无边框怎么做得按平台分（这一带改过三轮，结论写在下面，别再翻回去）：
            //  · Windows / Linux：整条系统标题栏都不要（decorations(false)），拖动与
            //    最小化/最大化/关闭由页面自绘（顶栏右侧的 .win-acts）。
            //  · macOS：用 **Overlay** —— 内容铺到标题栏下面，系统那三个红绿灯就浮在我们顶栏上，
            //    **与菜单同一行**。这条是用户明确要的：试过保留系统标题栏，三个圆点会单独占
            //    一行（"那三个按钮不要单独一行"）。
            //    代价：它们的纵向位置由系统钉在标题栏那条 **28pt** 条带里（圆点中心距顶 14pt），
            //    而 wry 的 inset_traffic_lights 只动横向、动不了纵向 —— 所以
            //    **顶栏在 macOS 上必须按 28px 走**（见 MainView 的 .topbar.is-mac）：
            //    40px 的话我们的内容中心在 20px，比圆点低 6px，就成"圆点飘在上边"了。
            //    全屏时系统会把圆点连同条带一起收走，顶栏给它们留的左内缩由 .topbar.is-mac-fs
            //    撤掉（前端订阅窗口是否全屏）。
            #[cfg(target_os = "macos")]
            let builder = builder
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true);
            #[cfg(not(target_os = "macos"))]
            let builder = builder.decorations(false);

            let window = builder.build()?;

            // 判据：无边框后内外尺寸只差一圈**窗口边框**（十几个逻辑像素，随 DPI 取整）；
            // 若还带着系统标题栏，会**再多出约 32 个逻辑像素**（标题栏本身）。
            // 所以看的是「高度差远小于 32」，不是「两者相等」—— 实测 150% 缩放下是 2160x1350 vs
            // 2182x1363（差 13），那就是已经去掉了。
            if let (Ok(inner), Ok(outer)) = (window.inner_size(), window.outer_size()) {
                eprintln!(
                    "窗口尺寸 inner={}x{}  outer={}x{}（无边框时两者应相同）",
                    inner.width, inner.height, outer.width, outer.height
                );
            }

            // ── 关闭窗口前先问一句：直接关（退出）还是最小化到托盘 ──
            // 拦在**壳**里，而不是只在页面上拦：右上角 ×、Alt+F4、任务栏右键关闭、macOS 红点……
            // 所有关闭途径最后都走到 WindowEvent::CloseRequested，这里一处兜住；
            // 只在页面上拦只能盖住自绘的那一个按钮。
            // 页面收到事件后弹自己的对话框（与界面同一套观感），再用 close_action 命令回复。
            {
                let w = window.clone();
                window.on_window_event(move |ev| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = ev {
                        if ALLOW_CLOSE.load(Ordering::SeqCst) {
                            return; // 前端已确认关闭：放行
                        }
                        api.prevent_close();
                        let _ = w.emit("dbmind:close-requested", ());
                    }
                });
            }

            // ── 托盘 ──
            // 既然提供"最小化到托盘"，就必须留一条回去的路，否则窗口一藏就再也叫不出来。
            // 菜单两项够用：显示主窗口 / 退出 DBmind（退出走 app.exit，仍会触发 RunEvent::Exit 清场）。
            // Linux 侧托盘依赖 libayatana-appindicator（CI 构建时装了 dev 包，deb/AppImage 自带运行时）。
            let show_item = tauri::menu::MenuItemBuilder::with_id("show", "显示主窗口").build(app)?;
            let quit_item = tauri::menu::MenuItemBuilder::with_id("quit", "退出 DBmind").build(app)?;
            let tray_menu = tauri::menu::MenuBuilder::new(app)
                .items(&[&show_item, &quit_item])
                .build()?;
            let mut tray = tauri::tray::TrayIconBuilder::new()
                .menu(&tray_menu)
                // Windows / Linux 的习惯是"左键点图标把窗口叫回来"，所以菜单只走右键
                .show_menu_on_left_click(false)
                .tooltip("DBmind");
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, ev| match ev.id().as_ref() {
                "show" => show_main_window(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, ev| {
                if let tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    button_state: tauri::tray::MouseButtonState::Up,
                    ..
                } = ev
                {
                    show_main_window(tray.app_handle());
                }
            })
            .build(app)?;

            Ok(())
        })
        // 用 build + run(回调) 而不是 run(context)：要在应用真正退出时清场。
        // 右上角 ×、Alt+F4、任务栏右键关闭……最终都会走到这里 —— 一处兜住全部关闭途径。
        // 页面回应「关闭窗口」的选择（关闭 / 最小化到托盘）走这个命令
        .invoke_handler(tauri::generate_handler![close_action])
        .build(tauri::generate_context!())
        .expect("Tauri 初始化失败")
        .run(|_app, event| {
            if let RunEvent::Exit = event {
                cleanup_child_processes();
            }
        });
}
