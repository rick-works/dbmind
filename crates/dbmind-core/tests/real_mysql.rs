//! **真机验证**：连接被掐断后，**第一条只读语句就该成功**。
//!
//! ## 这条验证对着哪个真机问题
//!
//! 用户反馈（2026-10-23）：窗口不关，VPN 断了再连上，然后执行 SQL ——
//! **第一次必报一次** `Communications link failure`（驱动诊断「上次收到包是 44 秒前」），
//! 第二次就正常了。
//!
//! 根因不是「重连慢」，而是**池里那条物理连接已经死了，却没人去探它**：
//! - 内核缓存了 sessionId、宿主缓存了 `Connection`，两边都认为它还活着；
//! - 交互页签的会话是**常驻**的（切回页签不必重新握手），而旧实现把「事务」和「页签」
//!   一起排除在后台保活之外 ⇒ 页签那条连接断了也没人发现；
//! - 「借出前探活」只看闲置 ≥60 秒的会话，44 秒的刚好漏过去。
//!
//! 修法两层（见 `drivers/session_pool.rs` 的 `Pin` 与 `agent_driver` 的
//! `retry_after_disconnect`）：页签常驻会话重新纳入保活；只读语句在连接级失败时
//! 重连重试一次。这条测试验的是**第二层**（第一层是 20 秒一轮的后台行为，单测覆盖）。
//!
//! ## 怎么模拟「链路断了」
//!
//! 用服务端 `KILL <thread_id>` 把那条连接掐掉：效果与「VPN 断了」对客户端是一样的
//! （socket 变死，池里那条约会成僵尸），而且可重复、不依赖网络设备。
//!
//! ## 它会对目标实例做什么
//!
//! - 只**读**：`SELECT CONNECTION_ID()` / `SELECT 1`；
//! - `KILL` **自己的**那条连接（同一个账号，不碰别人的会话）；
//! - 一条 `CREATE TEMPORARY TABLE`，**故意**打在已死的连接上（必然失败，也不会建出表）。
//!
//! 开法：`DBMIND_TEST_REAL_MYSQL=1`（外加 HOST/PORT/USER/PASSWORD，见下方默认值），
//! 或者直接用 `scripts/test/real-mysql-stale.ps1`。没开时只打印一句说明就返回，
//! 所以它夹在 `cargo test -- --ignored` 里也是安全的。

use std::time::Duration;

use dbmind_core::{
    AccessContext, CellValue, ConnectionConfig, ConnectionKind, DbMindEngine, QueryRequest,
};

/// 必须显式开启。默认（包括 `cargo test -- --ignored`）只打印一句说明。
fn enabled() -> bool {
    std::env::var("DBMIND_TEST_REAL_MYSQL")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// 临时 HOME：**绝不能**碰用户真实的 `~/.dbmind`。
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("dbmind-real-mysql-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("建临时目录失败");
        Self(path)
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn first_i64(rows: &[Vec<CellValue>]) -> Option<i64> {
    match rows.first()?.first()? {
        CellValue::Integer(v) => Some(*v),
        CellValue::Decimal(v) | CellValue::Text(v) => v.trim().parse().ok(),
        CellValue::Real(v) => Some(*v as i64),
        _ => None,
    }
}

/// 页签亲和键：不带 `internal` 前缀 ⇒ 交互泳道 ⇒ 会被 `pin_lane(Interactive)` 常驻。
const TAB: &str = "tab-stale-check";

#[test]
#[ignore = "真机验证：要连一台**真实的** MySQL（用 scripts/test/real-mysql-stale.ps1）"]
fn 真机_mysql_断链后第一条只读语句不该失败() {
    if !enabled() {
        // ASCII 的 `[SKIP]` 是给脚本用的（脚本 ASCII-only），用来确认「真的跑了」
        println!("[SKIP] DBMIND_TEST_REAL_MYSQL is not set -- 这一条要连真实 MySQL");
        return;
    }

    let host = env_or("DBMIND_TEST_MYSQL_HOST", "127.0.0.1");
    let port: u16 = env_or("DBMIND_TEST_MYSQL_PORT", "3306").parse().unwrap_or(3306);
    let user = env_or("DBMIND_TEST_MYSQL_USER", "root");
    let password = std::env::var("DBMIND_TEST_MYSQL_PASSWORD").unwrap_or_default();
    let database = env_or("DBMIND_TEST_MYSQL_DATABASE", "mysql");

    let dir = TempDir::new("stale");
    let engine = DbMindEngine::open(&dir.path("dbmind.db")).expect("打开引擎失败");
    let connection = engine
        .add_connection(
            ConnectionConfig::new("real-mysql", ConnectionKind::Mysql)
                .with_host(host.as_str())
                .with_port(port)
                .with_database(database.as_str())
                .with_credentials(user.as_str(), password.as_str()),
        )
        .expect("添加 MySQL 连接失败");
    let id = connection.id.clone();
    let run = |sql: &str, session: &str| {
        engine.execute(
            QueryRequest::new(id.as_str(), sql).with_session(session),
            AccessContext::Desktop,
        )
    };

    // ① 页签泳道上先跑一条，建立会话（并让它成为「页签常驻」）
    let thread_id = run("SELECT CONNECTION_ID()", TAB)
        .unwrap_or_else(|e| {
            panic!(
                "连不上 {user}@{host}:{port}/{database}：{} {}",
                e.code_str(),
                e.message
            )
        })
        .rows;
    let killed_id = first_i64(&thread_id).expect("CONNECTION_ID() 应当回一个整数");
    let first = run("SELECT 1", TAB).expect("预热查询不该失败");
    println!(
        "真机 MySQL：已连上，物理连接 thread_id={killed_id}，页签会话 = {:?}",
        first.session_id
    );

    // ② 从**另一条泳道**把那条连接掐掉 —— 对客户端而言就是「链路没了」。
    //    `internal:` 前缀 ⇒ 与页签不是同一条物理连接（MySQL 允许多条，见 YAML 的 maxConnections）
    let killed = run(&format!("KILL {killed_id}"), "internal:kill-stale");
    assert!(
        killed.is_ok(),
        "KILL 自己的会话不该失败：{:?}",
        killed.err().map(|e| (e.code_str(), e.message))
    );
    // 给服务端一点时间真正收掉那条 socket（真机上是 VPN 断连，这里是 KILL）
    std::thread::sleep(Duration::from_millis(300));

    // ③ 核心断言：**第一条**只读语句必须成功，用户不该看到那个「第一次失败」
    let after_kill = run("SELECT 1", TAB);
    match &after_kill {
        Ok(result) => println!(
            "真机 MySQL：断链后第一条 SELECT 成功（{}ms，会话 {:?}）",
            result.duration_ms, result.session_id
        ),
        Err(err) => panic!(
            "断链后第一条 SELECT 不该失败，实际：{} {} / {}",
            err.code_str(),
            err.message,
            err.detail.as_deref().unwrap_or("-")
        ),
    }
    after_kill.expect("上一步已断言成功");

    // 而且它是真的换了一条物理连接，不是「碰巧又能用」
    let new_id = first_i64(
        &run("SELECT CONNECTION_ID()", TAB)
            .expect("重连后读 CONNECTION_ID 不该失败")
            .rows,
    )
    .expect("CONNECTION_ID() 应当回一个整数");
    assert_ne!(new_id, killed_id, "应当重连到一条新的物理连接");
    println!("真机 MySQL：已重连到新连接 thread_id={new_id}");

    // ④ 写语句**不重试**：再掐一次，`CREATE TEMPORARY TABLE` 必须如实报错
    //    （重试写语句的风险是「响应丢了但已经执行过」，这条守住它）
    assert!(
        run(&format!("KILL {new_id}"), "internal:kill-stale").is_ok(),
        "第二次 KILL 不该失败"
    );
    std::thread::sleep(Duration::from_millis(300));
    let write = run("CREATE TEMPORARY TABLE dbmind_stale_probe (id int)", TAB);
    assert!(
        write.is_err(),
        "写语句不该被自动重试：它可能已经在库里执行过了"
    );
    println!(
        "真机 MySQL：写语句如实报错 ✓ {} {}",
        write.as_ref().err().map(|e| e.code_str()).unwrap_or("-"),
        write.as_ref().err().map(|e| e.message.as_str()).unwrap_or("-")
    );

    // ⑤ 但那之后连接是好的（缓存已清，下一次借用重连）—— 不该把连接搞成永久坏掉
    let recovered = run("SELECT 1", TAB).expect("写语句失败后，下一条读语句应当能重连成功");
    println!(
        "真机 MySQL：随后 SELECT 恢复 ✓（会话 {:?}）",
        recovered.session_id
    );
}

/// 第二层（后台保活）：**页签的空闲会话**断链后，20 秒内就该被摘掉。
///
/// 这一层管的是**写语句**：写不能重试（响应丢了它可能已经执行过），所以只能靠
/// 「根本没握住一条死连接」来保证第一条就成功。旧实现把页签常驻会话排除在保活之外，
/// 这条测试在旧实现下会失败（写语句撞 `Communications link failure`）。
#[test]
#[ignore = "真机验证：跑一轮要等 25 秒（等后台保活发现死连接）"]
fn 真机_mysql_页签空闲会话断链后由保活清理() {
    if !enabled() {
        println!("[SKIP] DBMIND_TEST_REAL_MYSQL is not set -- 这一条要连真实 MySQL");
        return;
    }

    let host = env_or("DBMIND_TEST_MYSQL_HOST", "127.0.0.1");
    let port: u16 = env_or("DBMIND_TEST_MYSQL_PORT", "3306").parse().unwrap_or(3306);
    let user = env_or("DBMIND_TEST_MYSQL_USER", "root");
    let password = std::env::var("DBMIND_TEST_MYSQL_PASSWORD").unwrap_or_default();
    let database = env_or("DBMIND_TEST_MYSQL_DATABASE", "mysql");

    let dir = TempDir::new("keepalive");
    let engine = DbMindEngine::open(&dir.path("dbmind.db")).expect("打开引擎失败");
    let connection = engine
        .add_connection(
            ConnectionConfig::new("real-mysql-keepalive", ConnectionKind::Mysql)
                .with_host(host.as_str())
                .with_port(port)
                .with_database(database.as_str())
                .with_credentials(user.as_str(), password.as_str()),
        )
        .expect("添加 MySQL 连接失败");
    let id = connection.id.clone();
    let run = |sql: &str, session: &str| {
        engine.execute(
            QueryRequest::new(id.as_str(), sql).with_session(session),
            AccessContext::Desktop,
        )
    };

    let thread_id = first_i64(
        &run("SELECT CONNECTION_ID()", TAB)
            .unwrap_or_else(|e| panic!("连不上 {user}@{host}:{port}：{} {}", e.code_str(), e.message))
            .rows,
    )
    .expect("CONNECTION_ID() 应当回一个整数");
    println!("真机 MySQL：页签会话 thread_id={thread_id}");

    // 掐掉它，然后**什么都不做** —— 完全交给后台保活
    assert!(
        run(&format!("KILL {thread_id}"), "internal:kill-keepalive").is_ok(),
        "KILL 不该失败"
    );
    println!("真机 MySQL：已掐断，等 25 秒（保活每 20 秒一轮）…");
    std::thread::sleep(Duration::from_secs(25));

    // 写语句必须**第一条就成功**：保活应当已经把死会话摘掉，这条语句落在新连接上
    let write = run("CREATE TEMPORARY TABLE dbmind_keepalive_probe (id int)", TAB);
    match &write {
        Ok(_) => println!("真机 MySQL：断链 25 秒后，写语句第一条就成功 ✓（保活已清理死会话）"),
        Err(err) => panic!(
            "页签空闲会话断链后，写语句仍然撞在死连接上：{} {} —— 保活没有覆盖交互页签",
            err.code_str(),
            err.message
        ),
    }
}
