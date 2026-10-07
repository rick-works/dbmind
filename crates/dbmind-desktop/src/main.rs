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
use std::time::{Duration, Instant};

use std::path::Path;

use sysinfo::System;
use tauri::{RunEvent, WebviewUrl, WebviewWindowBuilder};

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
/// PATH > 常见安装目录。多个候选时取版本最高的那个。
fn ensure_java() {
    if let Some(existing) = std::env::var_os("DBMIND_JAVA") {
        let path = PathBuf::from(existing);
        if path.is_file() {
            eprintln!("Java 宿主：沿用已有的 DBMIND_JAVA = {}", path.display());
            return;
        }
    }

    // 包**自带**的运行时（exe 旁的 jre/ 或 jdk/）单独成一层：只要 ≥17 就直接用它，
    // **不**参与后面的"取版本最高"。
    //
    // 这一层是必须的：包自带 JRE 的用意就是"不依赖目标机器装了什么"，
    // 若还跟系统 Java 比版本，用户机器上有个更新的 Java 就会顶掉它，
    // 自带运行时等于白打（实测踩过：包里带 17、机器上有 25，壳选了 25）。
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for sub in ["jre/bin/java.exe", "jdk/bin/java.exe"] {
                let bundled = dir.join(sub);
                if !bundled.is_file() {
                    continue;
                }
                if let Some(major) = java_major(&bundled) {
                    if major >= 17 {
                        eprintln!("Java 宿主：使用自带的运行时 {}（版本 {major}）", bundled.display());
                        std::env::set_var("DBMIND_JAVA", &bundled);
                        return;
                    }
                    eprintln!("Java 宿主：自带的运行时版本过低（{major}），改为查找系统上的 Java 17+");
                }
            }
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    // 系统上找：JAVA_HOME / PATH / 常见安装目录 —— 这几处**取版本最高的那个**
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // 自带运行时版本不合规时，仍把它作为候选（至少有个可用的）
            for sub in ["jre/bin/java.exe", "jdk/bin/java.exe"] {
                let candidate = dir.join(sub);
                if candidate.is_file() {
                    candidates.push(candidate);
                }
            }
        }
    }
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let candidate = PathBuf::from(home).join("bin/java.exe");
        if candidate.is_file() {
            candidates.push(candidate);
        }
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for entry in std::env::split_paths(&path_var) {
            let candidate = entry.join("java.exe");
            if candidate.is_file() {
                candidates.push(candidate);
            }
        }
    }
    // 常见安装位置：JDK 装在这里时通常**没有**配进 JAVA_HOME / PATH
    for root in [
        "C:\\Program Files\\Java",
        "C:\\Program Files\\Eclipse Adoptium",
        "C:\\Program Files\\Microsoft\\jdk",
        "D:\\develop\\tools",
    ] {
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let candidate = entry.path().join("bin/java.exe");
                if candidate.is_file() {
                    candidates.push(candidate);
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
            .eq_ignore_ascii_case("dbmind-web.exe")
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
/// 等内嵌内核就绪的上限
const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// 一次极简的 HTTP 探测：确认该端口上跑的是 **DBmind**，而不只是"有东西在监听"。
///
/// 手写报文是为了不给壳添一个 HTTP 客户端依赖 —— 这里只需要一个 GET，
/// 而壳层依赖越多，编译越慢、供应链越长。
fn is_dbmind(port: u16) -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(400)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
    let request = format!(
        "GET /api/dbmind/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut body = String::new();
    // 读失败（对端不是 HTTP、被截断）就当不是
    if stream.read_to_string(&mut body).is_err() {
        return false;
    }
    // 别用 `"ok":true` 硬匹配：不同 serde 版本/格式化方式会带空格
    body.contains("\"ok\"") && body.contains("true")
}

fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// 定下这次要连哪个端口，以及**要不要自己起内核**。
fn resolve_port() -> (u16, bool) {
    if is_dbmind(DEFAULT_PORT) {
        return (DEFAULT_PORT, false);
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
fn wait_ready(port: u16) {
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if is_dbmind(port) {
            return;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    // 超时不拦着开窗口：前端会显示"连不上后端"，比一个沉默的白窗口有用
    eprintln!("内嵌内核在 {READY_TIMEOUT:?} 内未就绪，仍打开窗口");
}

fn main() {
    // 必须在起内核**之前**：这个选择是通过进程环境变量传给内核的
    ensure_java();

    let (port, need_spawn) = resolve_port();

    if need_spawn {
        // 库位置与 CLI 一致（默认 ~/.dbmind，可用 DBMIND_STORE 覆盖）：
        // 桌面版不另起一份数据，用户换壳不换数据。
        let store: Option<PathBuf> = std::env::var_os("DBMIND_STORE").map(PathBuf::from);
        if let Err(err) = dbmind_web::spawn_embedded(port, store) {
            eprintln!("内嵌内核启动失败：{err}");
        }
        wait_ready(port);
    } else {
        eprintln!("检测到 {port} 端口已有 DBmind 实例，复用它");
    }

    let origin = format!("http://127.0.0.1:{port}");
    tauri::Builder::default()
        .setup(move |app| {
            // 窗口在这里建、而不是写在 tauri.conf.json 里：端口是**运行时**定的。
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(origin.parse()?))
                .title("DBmind")
                .inner_size(1440.0, 900.0)
                .min_inner_size(1024.0, 640.0)
                .center()
                // 去掉系统标题栏（那条深色横条与应用的浅色界面不搭）。
                // 代价是拖动与最小化/最大化/关闭**全都没有了**，所以前端有配套的
                // DesktopTitleBar.vue（拖动区 + 三个按钮），权限见 capabilities/default.json。
                .decorations(false)
                // 页面加载结果打出来（重定向 stderr 就能拿到）：
                // "窗口开着"不等于"页面加载成功" —— 白窗口同样是开着的，
                // 而这类故障在现场只能靠猜。留一行日志，排查时有据可查。
                .on_page_load(|_window, payload| {
                    if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                        eprintln!("前端页面加载完成: {}", payload.url());
                    }
                })
                .build()?;

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
            Ok(())
        })
        // 用 build + run(回调) 而不是 run(context)：要在应用真正退出时清场。
        // 右上角 ×、Alt+F4、任务栏右键关闭……最终都会走到这里 —— 一处兜住全部关闭途径。
        .build(tauri::generate_context!())
        .expect("Tauri 初始化失败")
        .run(|_app, event| {
            if let RunEvent::Exit = event {
                cleanup_child_processes();
            }
        });
}
