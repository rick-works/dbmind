//! **真机验证**：拿一台**真实的** SSH 跳板机验一次隧道。
//!
//! ## 为什么必须是真机
//!
//! 隧道这一层「自己写」的部分只有两处：本地端口的转发泵（`pump`）与隧道复用表 ——
//! 后者与 `pump` 都已有单测（见 `tunnel.rs` 的测试模块，用回环 TCP 驱动）。剩下没被
//! 单测覆盖的只有**与真实 SSH 服务端的交互**：握手、认证、`direct-tcpip` 通道。
//! 那些只能连一台真的跳板机来验 —— 本项目不自己实现 SSH，也就不该假装验证过它。
//!
//! ## 它对目标做什么
//!
//! **只读**：默认把「跳板机自己的 SSH 端口」当作转发目标，读一段 banner 来证明
//! 字节真的穿过了隧道。不写任何数据、不改任何配置、不落任何连接记录。
//! 也可以把目标改成别的地址（`DBMIND_TEST_SSH_TARGET_*`），那时只断言「连得上」。
//!
//! ## 怎么开
//!
//! ```text
//! DBMIND_TEST_SSH=1
//! DBMIND_TEST_SSH_HOST=jump.example.com     必填
//! DBMIND_TEST_SSH_USER=deploy               必填
//! DBMIND_TEST_SSH_PORT=22                   可选
//! DBMIND_TEST_SSH_PASSWORD=...              密码认证（与私钥二选一）
//! DBMIND_TEST_SSH_KEY=~/.ssh/id_rsa         私钥认证
//! DBMIND_TEST_SSH_KEY_PASSPHRASE=...        私钥口令短语（可选）
//! DBMIND_TEST_SSH_TARGET_HOST=127.0.0.1     可选，默认跳板机自己
//! DBMIND_TEST_SSH_TARGET_PORT=22            可选，默认跳板机自己的 SSH 端口
//! ```
//!
//! 或者直接用 `scripts/real-ssh-check.ps1`。没开这个开关时只打印一句说明就返回，
//! 所以它夹在 `cargo test -- --ignored` 里也是安全的。
//!
//! ## 一个刻意的特性：**这条验证不需要 JDK、不需要驱动宿主**
//!
//! 隧道完全在内核里（纯 Rust + libssh2），驱动宿主只是去连 `127.0.0.1:<本地端口>`。
//! 所以这里不碰 `agents/*.jar`、不碰 `DBMIND_JAVA` —— 与 `real_redis.rs` 相比，
//! 这一条的失败只会指向 SSH 本身。

use std::io::Read;
use std::net::TcpStream;
use std::time::Duration;

use dbmind_core::tunnel::{self, EXTRA_SSH};
use dbmind_core::{ConnectionConfig, ConnectionKind};
use serde_json::{json, Map, Value};

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 必须显式开启。默认（包括 `cargo test -- --ignored`）只打印一句说明。
fn enabled() -> bool {
    std::env::var("DBMIND_TEST_SSH")
        .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

fn required(name: &str) -> String {
    env(name).unwrap_or_else(|| {
        panic!("缺少环境变量 {name}（这一条要连**真实的**跳板机，见文件头的用法说明）")
    })
}

/// 组装一条「走 SSH 隧道」的连接：目标是跳板机视角下的地址。
fn tunneled(ssh: Value, target_host: &str, target_port: u16) -> ConnectionConfig {
    let mut extra = Map::new();
    extra.insert(EXTRA_SSH.to_string(), ssh);
    let mut config = ConnectionConfig::new("real-ssh-target", ConnectionKind::Mysql)
        .with_host(target_host)
        .with_port(target_port)
        .with_credentials("probe", "probe");
    config.extra = Some(Value::Object(extra));
    config
}

/// 打开隧道 → 读一段 banner → 复用同一条隧道 → 错口令要明确说「认证失败」。
#[test]
#[ignore = "真机验证：要连一台**真实的** SSH 跳板机（用 scripts/real-ssh-check.ps1）"]
fn 真机_ssh_隧道转发可用() {
    if !enabled() {
        // 与 real_redis.rs 同一条约定：ASCII 的 `[SKIP]` 给脚本判定，
        // 免得「被环境开关悄悄跳过」被当成通过。
        println!(
            "[SKIP] DBMIND_TEST_SSH is not set -- 这一条要连**真实的** SSH 跳板机，\
             用 scripts/real-ssh-check.ps1，或自己设这个变量"
        );
        return;
    }

    let ssh_host = required("DBMIND_TEST_SSH_HOST");
    let ssh_user = required("DBMIND_TEST_SSH_USER");
    let ssh_port: u16 = env("DBMIND_TEST_SSH_PORT")
        .and_then(|value| value.parse().ok())
        .unwrap_or(22);
    // 默认拿跳板机**自己的 SSH 端口**当靶子：任何跳板机都一定有它，而且是只读的，
    // 读到的 banner 正好证明「字节真的穿过了隧道」。
    let target_is_self = env("DBMIND_TEST_SSH_TARGET_PORT").is_none();
    let target_host = env("DBMIND_TEST_SSH_TARGET_HOST").unwrap_or_else(|| "127.0.0.1".to_string());
    let target_port: u16 = env("DBMIND_TEST_SSH_TARGET_PORT")
        .and_then(|value| value.parse().ok())
        .unwrap_or(22);

    let mut ssh = Map::new();
    ssh.insert("enabled".to_string(), json!(true));
    ssh.insert("host".to_string(), json!(ssh_host));
    ssh.insert("port".to_string(), json!(ssh_port));
    ssh.insert("user".to_string(), json!(ssh_user));
    match (env("DBMIND_TEST_SSH_KEY"), env("DBMIND_TEST_SSH_PASSWORD")) {
        (Some(key), _) => {
            ssh.insert("authType".to_string(), json!("key"));
            ssh.insert("keyPath".to_string(), json!(key));
            if let Some(passphrase) = env("DBMIND_TEST_SSH_KEY_PASSPHRASE") {
                ssh.insert("keyPassphrase".to_string(), json!(passphrase));
            }
        }
        (None, Some(password)) => {
            ssh.insert("authType".to_string(), json!("password"));
            ssh.insert("password".to_string(), json!(password));
        }
        (None, None) => panic!("要给出 DBMIND_TEST_SSH_PASSWORD 或 DBMIND_TEST_SSH_KEY 其中之一"),
    }
    let ssh = Value::Object(ssh);

    println!(
        "真机 SSH：{ssh_user}@{ssh_host}:{ssh_port} → {target_host}:{target_port}（目标{}）",
        if target_is_self { "= 跳板机自身" } else { "由环境变量指定" }
    );

    let config = tunneled(ssh.clone(), &target_host, target_port);
    let local_port = tunnel::local_port_for(&config)
        .unwrap_or_else(|err| {
            panic!(
                "建隧道失败：{} {} / {}",
                err.code_str(),
                err.message,
                err.detail.as_deref().unwrap_or("-")
            )
        })
        .expect("开了 SSH 开关就必须拿到本地转发端口");
    println!("真机 SSH：隧道已建立，本地端口 {local_port}");

    // ① 字节真的穿过了隧道：从本地端口读一段数据
    let mut stream = TcpStream::connect(("127.0.0.1", local_port)).expect("本地转发端口应可连接");
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .expect("设置读超时应成功");
    let mut banner = [0u8; 128];
    let read = stream.read(&mut banner).expect("隧道应当有数据回来");
    assert!(read > 0, "隧道读到了 0 字节，等于没打通");
    let text = String::from_utf8_lossy(&banner[..read]).to_string();
    println!("真机 SSH：经隧道读到 {:?}", text.trim());
    if target_is_self {
        assert!(
            text.starts_with("SSH-"),
            "目标是跳板机自己的 SSH 端口，应当读到 SSH banner，实际：{text:?}"
        );
    }
    drop(stream);

    // ② 同一条配置再取一次：必须**复用**同一条隧道（否则浏览几个库就会开几条 SSH 连接）
    let again = tunnel::local_port_for(&config)
        .expect("第二次取值不该失败")
        .expect("第二次也该有端口");
    assert_eq!(again, local_port, "同一条隧道配置必须复用同一个本地端口");

    // ③ 认证要能被验出来：错口令必须当场报「认证」而不是留个死端口
    //
    // 先清掉已有隧道 —— 隧道是按「跳板机 + 目标地址」复用的，不清的话这次改动
    // 会被判成「同一条隧道」而直接复用，负例就验不到了。
    tunnel::shutdown_all();
    let mut bad = ssh.clone();
    if let Some(object) = bad.as_object_mut() {
        object.insert("authType".to_string(), json!("password"));
        object.insert("password".to_string(), json!("definitely-not-the-password"));
    }
    let bad_config = tunneled(bad, &target_host, target_port);
    let err = tunnel::local_port_for(&bad_config).expect_err("错口令不该建出隧道");
    let text = format!(
        "{} {} / {}",
        err.code_str(),
        err.message,
        err.detail.as_deref().unwrap_or("-")
    );
    assert!(
        text.contains("SSH") && (text.contains("认证") || text.contains("口令")),
        "错口令要如实说清是 SSH 认证问题，实际：{text}"
    );
    println!("真机 SSH：错口令被如实拦下 ✓（{text}）");

    tunnel::shutdown_all();
}
