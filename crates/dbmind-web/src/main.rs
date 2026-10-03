//! DBMind HTTP 壳的启动入口。**薄**：只解析参数、初始化日志、交给库启动。

use dbmind_web::Options;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    // 写 stderr：它不缓冲，日志立刻落到文件里。
    // 默认写 stdout 在重定向到文件时是块缓冲的 —— 排查「服务刚起来还算好，过一会儿就不行」
    // 这类问题时，最需要看的那几行恰好还卡在缓冲区里，等于没日志。
    // 过滤器包了 reload 层：设置页的「日志级别」可以运行时热切换（见 lib.rs）。
    dbmind_web::init_tracing();

    let mut options = Options {
        // 默认托管仓库内的前端产物（构建后为 frontend/dist）；可用 --dist 覆盖
        dist: std::env::current_dir().ok().map(|d| d.join("frontend/dist")),
        ..Options::default()
    };

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => {
                if let Some(value) = args.next() {
                    options.host = value;
                }
            }
            "--port" => {
                if let Some(value) = args.next().and_then(|v| v.parse().ok()) {
                    options.port = value;
                }
            }
            "--store" => options.store = args.next().map(PathBuf::from),
            "--dist" => options.dist = args.next().map(PathBuf::from),
            "--version" => {
                println!("dbmind-web {}", dbmind_core::VERSION);
                return;
            }
            "--help" | "-h" => {
                println!("dbmind-web [--host 127.0.0.1] [--port 20361] [--store <path>] [--dist <dir>]");
                println!("  /api/…       上游兼容契约（原版前端）");
                println!("  /api/dbmind/… 内核原生契约（CLI / 桌面壳 / 冒烟脚本）");
                return;
            }
            other => {
                eprintln!("未知参数: {other}");
                std::process::exit(2);
            }
        }
    }

    if let Err(message) = dbmind_web::serve(options).await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
