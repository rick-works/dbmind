//! **真机验证**：拿一台**真实的** Redis 验一次「排队中被取消」。
//!
//! ## 为什么不放进宿主层
//!
//! 宿主层（`src/engine.rs` 里那批 `宿主层_*`）用的是**替身**（`deploy/redis-test-server`），
//! 而替身是可以照着我们的实现方式配合的 —— 真机不会。这里连的是真 Redis，本机那台是
//! **3.0.504**（2015 年），比宿主用的 Jedis 还老，正好用来说明「取消不依赖任何新命令」：
//! 宿主的机制是「置令牌 + 渲染阶段检查」（见 `agents/dbmind-agent-redis`），
//! 所以它在老版本上同样成立。
//!
//! 另外它是个**独立的集成测试靶子**：脚本可以用全 ASCII 的 `--test real_redis` 选中它，
//! 不必往命令行里塞中文过滤词（PS 5.1 下中文参数的编码很脆，我在这上面栽过）。
//!
//! ## 它会对目标实例做什么
//!
//! - 一条 `DEBUG SLEEP 2`：Redis 是**单线程**，所以那约 2 秒里**整个实例**都会停住。
//!   这是本验证唯一的侵入性，脚本里有同样的提示 —— 别拿它去指共享实例。
//! - 除此之外**不写任何键**：测试自己比对 `DBSIZE` 的前后值来证明这一点。
//!
//! 开法：`DBMIND_TEST_REAL_REDIS=1`（端口用 `DBMIND_TEST_REAL_REDIS_PORT`，默认 6379），
//! 或者直接用 `scripts/real-redis-check.ps1`。没开时只打印一句说明就返回，
//! 所以它夹在 `cargo test -- --ignored` 里也是安全的。

use std::time::{Duration, Instant};

use dbmind_core::{AccessContext, ConnectionConfig, ConnectionKind, DbMindEngine, ErrorCode, QueryRequest};

/// 必须显式开启。默认（包括 `cargo test -- --ignored`）只打印一句说明。
fn enabled() -> bool {
    std::env::var("DBMIND_TEST_REAL_REDIS")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// 临时 HOME：**绝不能**碰用户真实的 `~/.dbmind`。
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("dbmind-real-redis-{tag}-{}", std::process::id()));
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

/// 真机 Redis 上的「排队中被取消」——与宿主层 ⑨（替身）互为对照。
///
/// 标 `#[ignore]`：默认跑里它连门都不出（那句 env 开关是第二道闸，防的是
/// 「有人只删了 `#[ignore]` 就以为它会在默认跑里验真机」）。
#[test]
#[ignore = "真机验证：要连一台**真实的** Redis（用 scripts/real-redis-check.ps1；替身见宿主层 ⑨）"]
fn 真机_redis_排队中被取消() {
    if !enabled() {
        // ASCII 的 `[SKIP]` 那句是给脚本用的：脚本是 ASCII-only（见 scripts/smoke.ps1 的约定），
        // 它靠这个标记确认「测试真的跑了」，而不是被环境开关悄悄跳过还报成功。
        println!(
            "[SKIP] DBMIND_TEST_REAL_REDIS is not set -- 这一条要连**真实**的 Redis，\
             用 scripts/real-redis-check.ps1，或自己设这个变量"
        );
        return;
    }

    let port: u16 = std::env::var("DBMIND_TEST_REAL_REDIS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6379);

    let dir = TempDir::new("queued-cancel");
    let engine = DbMindEngine::open(&dir.path("dbmind.db")).expect("打开引擎失败");
    let redis = engine
        .add_connection(
            ConnectionConfig::new("real-redis", ConnectionKind::Redis)
                .with_host("127.0.0.1")
                .with_port(port),
        )
        .expect("添加 Redis 连接失败");
    let run = |sql: &str| engine.execute(QueryRequest::new(redis.id.as_str(), sql), AccessContext::Desktop);

    // 先确认这真是一台能说话的 Redis（连不上就直接说清楚，别让它表现成「取消失败」）
    let ping = run("PING").unwrap_or_else(|e| {
        panic!(
            "连不上 127.0.0.1:{port} 上的 Redis：{} {} —— 这一条要的是真机，不是替身",
            e.code_str(),
            e.message
        )
    });
    assert!(
        format!("{:?}", ping.rows).contains("PONG"),
        "PING 应当回 PONG：{:?}",
        ping.rows
    );
    let size_before = run("DBSIZE").expect("读 DBSIZE 失败");
    println!("真机 Redis：PING ✓  DBSIZE = {:?}", size_before.rows);

    // 慢命令：DEBUG SLEEP 2。它会**卡住整个实例**约 2 秒（单线程），这是本验证唯一的侵入。
    let slow_sql = "DEBUG SLEEP 2";
    let slow_id = DbMindEngine::next_execution_id();
    let queued_id = DbMindEngine::next_execution_id();

    let (hit, queued, waited, slow_outcome) = std::thread::scope(|scope| {
        let slow_handle = {
            let id = slow_id.clone();
            let engine = &engine;
            let connection = redis.id.clone();
            scope.spawn(move || {
                engine.execute(
                    QueryRequest::new(connection.as_str(), slow_sql).with_execution_id(id),
                    AccessContext::Desktop,
                )
            })
        };
        // 等它真的占住那唯一一条会话（Redis 是单连接类型）
        let mut spun = Duration::ZERO;
        while spun < Duration::from_secs(5) && !engine.active_executions().iter().any(|e| e == &slow_id) {
            std::thread::sleep(Duration::from_millis(20));
            spun += Duration::from_millis(20);
        }
        // 登记不等于「命令已经发出去了」，留一点余量
        std::thread::sleep(Duration::from_millis(200));

        // 第二条：单连接 ⇒ 只能排队
        let queued_handle = {
            let id = queued_id.clone();
            let engine = &engine;
            let connection = redis.id.clone();
            scope.spawn(move || {
                engine.execute(
                    QueryRequest::new(connection.as_str(), "PING").with_execution_id(id),
                    AccessContext::Desktop,
                )
            })
        };
        std::thread::sleep(Duration::from_millis(200));

        let started = Instant::now();
        let hit = engine.cancel(&queued_id);
        let queued = queued_handle.join().expect("查询线程不应 panic");
        let waited = started.elapsed();
        let slow_outcome = slow_handle.join().expect("查询线程不应 panic");
        (hit, queued, waited, slow_outcome)
    });

    assert!(hit, "取消应当命中排队中的那条");
    let err = queued.expect_err("被取消的排队命令不该返回结果");
    assert_eq!(
        err.code,
        ErrorCode::QueryCanceled,
        "真机上排队被取消也要报「已取消」，实际：{} {} / {}",
        err.code_str(),
        err.message,
        err.detail.as_deref().unwrap_or("-")
    );
    assert!(
        err.message.contains("等待可用连接"),
        "要说清是**排队时**被取消的：{}",
        err.message
    );
    println!("真机 Redis 排队被取消：{waited:?} 就返回了（{slow_sql} 占着那唯一一条会话）");
    assert!(
        waited < Duration::from_secs(2),
        "排队被取消要立刻返回，不能等慢命令跑完：实测 {waited:?}"
    );

    // 慢命令自己是正常结束的：宿主对「正在跑的命令」的边界是**不打断**（与替身那次一致）
    slow_outcome.unwrap_or_else(|e| {
        panic!(
            "{slow_sql} 本身应当正常返回，实际：{} {}",
            e.code_str(),
            e.message
        )
    });

    // **第二条：真超时**（把内核超时降到 1 秒，再跑同一条 2 秒的命令）。
    //
    // 这一条钉的是真机验证第一次就抓到的两件事：
    //   ① 超时要被说成**超时**（QUERY-0003），不能报成「连接已断开」——
    //      否则用户会去查网络，而问题其实在超时配置；
    //   ② 那条坏掉的连接必须**自动重建**：Jedis 在读超时后会把连接关掉，
    //      而内核**不给非幂等查询重试**（`retry_on_stale` 只对幂等调用为真），
    //      所以宿主不自愈的话，这条会话会一直坏到用户手动重连。
    let mut timed = QueryRequest::new(redis.id.as_str(), slow_sql);
    timed.options.timeout_ms = 1000;
    let started = Instant::now();
    let err = engine
        .execute(timed, AccessContext::Desktop)
        .expect_err("1 秒超时跑一条 2 秒的命令，应当超时");
    let elapsed = started.elapsed();
    assert_eq!(
        err.code,
        ErrorCode::QueryTimeout,
        "超时要如实报超时，实际：{} {} / {}",
        err.code_str(),
        err.message,
        err.detail.as_deref().unwrap_or("-")
    );
    assert!(!err.message.is_empty(), "超时也要说人话");
    println!("真机 Redis 超时：{elapsed:?} 返回「{}」", err.code_str());

    // 关键一条：**下一条命令必须照常可用**（连接自愈）
    let healed = run("PING").expect("超时之后会话必须自愈");
    assert!(
        format!("{:?}", healed.rows).contains("PONG"),
        "自愈之后 PING 应当回 PONG：{:?}",
        healed.rows
    );
    println!("真机 Redis：超时之后会话自愈 ✓");

    // **第三条：断链自愈**（宿主层的 ⑫ 用的是替身，替身可能「配合我们」——真机上再验一次）。
    // `QUIT` 是真 Redis 的标准命令：回 `+OK` 然后关连接。切断之后：
    //   ① 紧接着那条如实报连接级错误（CONN-0003 ⇒ 内核会清掉自己的会话缓存）；
    //   ② 再下一条必须自己好起来（宿主重建连接 / 内核重连）。
    let quit = run("QUIT").expect("QUIT 应当正常返回");
    assert!(
        format!("{:?}", quit.rows).contains("OK"),
        "QUIT 应当回 OK：{:?}",
        quit.rows
    );
    let broken = run("PING").expect_err("连接刚被切断，这一条应当失败");
    assert_eq!(
        broken.code,
        ErrorCode::ConnConnectFailed,
        "断链应当报连接级错误（CONN-0003），实际：{} {}",
        broken.code_str(),
        broken.message
    );
    run("PING").expect("断链之后应当自愈");
    println!("真机 Redis：断链之后自愈 ✓（第一条如实报 {}）", broken.code_str());

    // **没写任何键**：这是这条验证敢碰真实实例的前提
    let size_after = run("DBSIZE").expect("读 DBSIZE 失败");
    assert_eq!(
        size_after.rows, size_before.rows,
        "这条验证不该改动真实实例里的任何数据"
    );
    println!("真机 Redis：DBSIZE 前后都是 {:?} ✓", size_before.rows);

    // 取消之后会话照常可用
    let after = run("PING").expect("取消之后会话必须还能用");
    assert!(
        format!("{:?}", after.rows).contains("PONG"),
        "取消之后 PING 仍应回 PONG：{:?}",
        after.rows
    );
    println!("真机 Redis：取消之后会话照常可用 ✓");
}
