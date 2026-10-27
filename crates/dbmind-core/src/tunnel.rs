//! SSH 隧道（跳板机）。
//!
//! 数据库只对跳板机可达时，先连 SSH，再由跳板机把流量转发到数据库。
//!
//! 为什么放在内核：隧道是「连数据库」这件事的一部分。四个壳（Web / 桌面 / CLI / MCP）
//! 必须共用同一套行为 —— 否则同一条连接会出现「CLI 能连上、界面连不上」这种最难查的分歧。
//!
//! 形状：在 `127.0.0.1` 上开一个**本地监听端口**，把每条进来的 TCP 连接经 SSH 的
//! `direct-tcpip` 通道送到「跳板机视角下」的数据库地址。驱动宿主拿到的地址因此是
//! `127.0.0.1:<本地端口>`，它**完全不需要知道 SSH 的存在**（Java 宿主与原生协议都一视同仁）。
//!
//! 三个刻意的取舍，都写在对应的代码旁边：
//!
//! 1. **按「跳板机 + 目标地址」复用隧道**（见 [`local_port_for`]）：影子连接（跨库浏览）
//!    与主连接共享同一条隧道，否则浏览 10 个库就会开 10 条 SSH 连接。
//! 2. **监听只绑回环**（见 [`LOCAL_HOST`]）：绝不对外暴露一个绕过认证的数据库入口。
//! 3. **不做跳板机公钥校验**：首次连接自动接受对方公钥，与常见数据库客户端一致。
//!    这一点如实写在这里，而不是假装做了校验。

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use ssh2::{
    CheckResult, HashType, HostKeyType, KeyboardInteractivePrompt, KnownHostFileKind, Session,
};

use crate::types::ConnectionConfig;
use crate::{paths, DbMindError, ErrorCode, Result};

/// `ConnectionConfig.extra` 里存 SSH 配置的键。
///
/// 放 `extra` 而不是加表列：`connections` 表只有 `CREATE TABLE IF NOT EXISTS`、
/// **没有迁移机制**（见 storage 模块的说明），加列会让老库直接缺字段。
pub const EXTRA_SSH: &str = "ssh";

/// 本机回环。隧道只在回环上监听 —— 绝不对外暴露一个「免认证的数据库入口」。
const LOCAL_HOST: &str = "127.0.0.1";

/// 连跳板机 / 握手 / 认证的超时。
///
/// 跳板机不通时宁可**当场报错**，也不让「测试连接」一直转圈转到用户以为程序卡死。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// TCP keepalive：多久没有字节往来就开始探测。
///
/// 为什么必须开（真机 2026-10-27 用户反馈）：**换 WiFi / 断网后 TCP 是半开的** ——
/// 没有 RST、没有 FIN，libssh2 的读写一直返回 `WouldBlock`，`pump` 那个转发线程
/// 就一直空转不退出，它借走的 SSH 会话也被永久占用；而缓存里的隧道「端口还在 listen」
/// 让 `reuse()` 继续把新连接送进这条死链路。用户看到的就是
/// 「网络明明恢复了、数据库和 SSH 全连不上，必须把 VPN 客户端关掉重开」。
///
/// 开了 keepalive 之后，链路真断时探测包拿不到 ACK，内核在 time + interval×重试
/// 之后把 socket 置为错误态，`pump` 的读写于是报错退出，会话归还/丢弃，下次连库
/// 重新握手。空闲的正常数据库长连接不会因此被断开（对端会正常回 ACK）。
const KEEPALIVE_TIME: Duration = Duration::from_secs(30);

/// keepalive 探测的间隔（失败后每隔这么久重试一次）。
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// 空闲多久之后回收隧道（下次用到会重建）——**默认值**，实际以设置
/// `tunnel.idleTimeoutSecs` 为准（0 = 不按空闲回收）。
///
/// 存在的理由：改了跳板机配置再改回来、或者连接被删掉，隧道如果没有回收，
/// 就会一直占着一条 SSH 连接和一个本地端口，直到进程退出。
/// 曾经写死 30 分钟：云库/RDS 的连接配额金贵（想更短）、内网跳板机又想放久些，
/// 没有统一答案 —— 交给设置，引擎在开机与设置变更时调 [`set_idle_timeout_secs`] 同步。
const DEFAULT_IDLE_TTL: Duration = Duration::from_secs(30 * 60);

/// 空闲回收时长的进程内镜像（秒）。引擎持有真源（存储），这里只读。
static IDLE_TTL_SECS: AtomicU64 = AtomicU64::new(DEFAULT_IDLE_TTL.as_secs());

/// 同步空闲回收时长（秒；0 = 不回收）。下一轮 `reuse` 就按新值 `retain`。
pub fn set_idle_timeout_secs(secs: u64) {
    IDLE_TTL_SECS.store(secs, Ordering::Relaxed);
    tracing::debug!(target: "dbmind::tunnel", secs, "SSH 隧道空闲回收时长已更新");
}

/// 单向缓冲高水位。本地这头写不动（消费慢）时先别再往内存里堆。
const BUFFER_HIGH_WATER: usize = 1 << 20;

/// 转发轮询的休眠间隔：只在一轮里两个方向都没搬动数据时才睡。
const IDLE_POLL: Duration = Duration::from_millis(1);

/// 监听线程的 accept 轮询间隔（见 [`accept_loop`] 里为什么不用阻塞 accept）。
const ACCEPT_POLL: Duration = Duration::from_millis(20);

/// 同时进行的 SSH **握手**上限。
///
/// 为什么限：每条转发连接要占一条独占 SSH 会话（见 [`accept_loop`] 与 [`SessionPool`]），
/// 高并发同步瞬间要建几十条会话；而 sshd 的 `MaxStartups` 默认 `10:30:100` —— 超过 10 条
/// 未认证连接后**随机丢弃**，被丢的那批要等超时重试，整批建连被拖成分钟级（真机：整库同步
/// 并发表 16 时 32 条会话同时握手，全部卡死、任务 8 分钟一行没读）。限流排队依次握手
/// 比被服务端丢弃再重试快得多，而且不依赖跳板机的配置。
const HANDSHAKE_MAX: usize = 8;
static HANDSHAKE_INFLIGHT: AtomicUsize = AtomicUsize::new(0);

/// 累计**成功完成认证**的 SSH 会话数。
///
/// 只为一件事存在：证明「会话池真的省掉了握手」—— 真机一次握手 + 认证 2.7 秒，
/// 光看连接快慢是主观感受，看这个数字变了没有才是证据（真机验证见 `tests/real_ssh.rs`）。
static HANDSHAKE_TOTAL: AtomicUsize = AtomicUsize::new(0);

/// 累计握手次数。第二次连库时若它没变，说明用的就是池里那条会话。
pub fn handshake_total() -> usize {
    HANDSHAKE_TOTAL.load(Ordering::Relaxed)
}

/// 握手名额租约：拿到才开始 `open_session`，`Drop` 归还（含 panic）。
struct HandshakeLease;

impl HandshakeLease {
    fn acquire() -> Self {
        loop {
            let cur = HANDSHAKE_INFLIGHT.load(Ordering::SeqCst);
            if cur < HANDSHAKE_MAX
                && HANDSHAKE_INFLIGHT
                    .compare_exchange(cur, cur + 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                return Self;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }
}

impl Drop for HandshakeLease {
    fn drop(&mut self) {
        HANDSHAKE_INFLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

// ------------------------------------------------------------------ 配置

/// 跳板机的认证方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshAuth {
    Password(String),
    /// 私钥文件（OpenSSH 格式）与可选的口令短语。
    Key { path: String, passphrase: Option<String> },
}

/// 一条已展开的 SSH 隧道配置（只在使用时才构造，纯读 `extra` 见 [`tunnel_key`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
}

fn conn_err(message: impl Into<String>) -> DbMindError {
    DbMindError::new(ErrorCode::ConnConnectFailed, message.into())
}

fn text_raw(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn text(value: &Value, key: &str) -> String {
    text_raw(value, key).trim().to_string()
}

/// 连接上的 `extra.ssh`（没配就是 `None`）。
fn ssh_object(config: &ConnectionConfig) -> Option<&Value> {
    config.extra.as_ref().and_then(|extra| extra.get(EXTRA_SSH))
}

fn port_of(ssh: &Value) -> u16 {
    ssh.get("port")
        .and_then(Value::as_u64)
        .filter(|port| *port > 0 && *port <= u16::MAX as u64)
        .map(|port| port as u16)
        .unwrap_or(22)
}

/// 隧道指纹：**不改配置、不做任何 I/O**，只描述「用哪条隧道」。
///
/// 两个用途：① 作会话键的一部分（改了跳板机就必须换一条宿主会话）；
/// ② 作隧道复用表的键（同一条隧道可以被多个库/影子共享）。
///
/// 刻意**不含口令**：它要进会话键，而会话键会出现在日志与宿主协议里。
pub fn tunnel_key(config: &ConnectionConfig) -> Option<String> {
    let ssh = ssh_object(config)?;
    if !ssh.get("enabled").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let host = text(ssh, "host");
    if host.is_empty() {
        return None;
    }
    let auth = match text(ssh, "authType").as_str() {
        "key" => format!("key:{}", text(ssh, "keyPath")),
        _ => "password".to_string(),
    };
    Some(format!(
        "ssh={}@{}:{}/{auth}",
        text(ssh, "user"),
        host,
        port_of(ssh)
    ))
}

impl SshConfig {
    /// 从连接配置展开。**启用但填不全**时在这里就报错 —— 这种错误拖到认证阶段
    /// 会变成一句无头无尾的「认证失败」，用户只能靠猜。
    pub fn from_config(config: &ConnectionConfig) -> Result<Option<SshConfig>> {
        let Some(ssh) = ssh_object(config) else {
            return Ok(None);
        };
        if !ssh.get("enabled").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(None);
        }
        let host = text(ssh, "host");
        let user = text(ssh, "user");
        if host.is_empty() || user.is_empty() {
            return Err(conn_err("SSH 隧道已启用，但跳板机地址或用户名没填").with_detail(
                "在连接弹窗的「SSH 隧道」页签里补上跳板机地址与登录用户；\
                 不用隧道就把那个开关关掉。",
            ));
        }
        let auth = match text(ssh, "authType").as_str() {
            "key" => {
                let path = text(ssh, "keyPath");
                if path.is_empty() {
                    return Err(conn_err("SSH 认证方式选了「私钥」，但私钥路径是空的").with_detail(
                        "填上私钥文件的完整路径（OpenSSH 格式，通常形如 ~/.ssh/id_rsa）；\
                         带口令短语的私钥请在旁边一并填写。",
                    ));
                }
                SshAuth::Key {
                    path,
                    // 口令短语**不去尾随空白**：它就是口令的一部分
                    passphrase: non_empty(text_raw(ssh, "keyPassphrase")),
                }
            }
            _ => {
                let password = text_raw(ssh, "password");
                if password.is_empty() {
                    return Err(conn_err("SSH 认证方式选了「密码」，但口令是空的").with_detail(
                        "填上跳板机登录口令；编辑已有连接时留空表示沿用已保存的那份 —— \
                         若已存的那份也丢了，就重新输入一次再保存。",
                    ));
                }
                SshAuth::Password(password)
            }
        };
        Ok(Some(SshConfig {
            host,
            port: port_of(ssh),
            user,
            auth,
        }))
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// 把连接配置的「连库地址」换成本地转发端口（其余字段原样保留）。
///
/// 只用于**真正发出去**的连接参数：页面上的「主机」显示读的是连接记录本身，
/// 不会被这里改掉（否则用户会看到自己的连接莫名其妙变成了 127.0.0.1）。
pub fn with_local_endpoint(config: &ConnectionConfig, local_port: u16) -> ConnectionConfig {
    let mut copy = config.clone();
    copy.host = Some(LOCAL_HOST.to_string());
    copy.port = Some(local_port);
    copy
}

// ------------------------------------------------------------------ 主机公钥

/// 本应用自己的「主机公钥钉扎」文件（TOFU：第一次见就记住，之后变了就拦）。
///
/// 为什么不直接写 `~/.ssh/known_hosts`：那是用户和系统 ssh 共用的信任库，应用不该改它。
/// 这里**只读**它（用户在别处已经信任过的指纹直接沿用），自己的学习结果单独存一份。
///
/// 格式刻意做成最简的一行一条，而不是 OpenSSH 格式：比对必须按「主机 + 端口 + 算法」
/// 精确进行。OpenSSH 允许同一主机只记一种算法，拿它做变更判定会在「服务器换了算法」
/// 时误报成「有人冒充」—— 所以那份文件只用来做「精确命中即信任」。
///
/// 行格式：`<host>:<port> <算法名> <base64 公钥>`（`#` 开头是注释）。
pub fn known_hosts_path() -> PathBuf {
    paths::home_dir().join("known_hosts")
}

/// 用户系统的 `~/.ssh/known_hosts`（不存在就是 `None`）。
fn system_known_hosts_path() -> Option<PathBuf> {
    let path = paths::user_home().join(".ssh").join("known_hosts");
    path.is_file().then_some(path)
}

/// 主机公钥算法的标准名字（与 `known_hosts` 里的写法一致）。
fn host_key_name(kind: HostKeyType) -> &'static str {
    match kind {
        HostKeyType::Rsa => "ssh-rsa",
        HostKeyType::Dss => "ssh-dss",
        HostKeyType::Ecdsa256 => "ecdsa-sha2-nistp256",
        HostKeyType::Ecdsa384 => "ecdsa-sha2-nistp384",
        HostKeyType::Ecdsa521 => "ecdsa-sha2-nistp521",
        HostKeyType::Ed25519 => "ssh-ed25519",
        HostKeyType::Unknown => "unknown",
    }
}

/// 一条已知主机记录与这次握手的比对结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostKeyVerdict {
    /// 同一主机、同一算法，公钥一致
    Match,
    /// 同一主机、同一算法，公钥不一致 —— 可能是重装换了密钥，也可能是中间人
    Changed,
    /// 没见过这个「主机 + 端口 + 算法」
    Unknown,
}

/// 在 known_hosts 文本里比对一条记录（纯函数，便于单测）。
///
/// 认三种主机写法：本应用写的 `host:port`、OpenSSH 的 `[host]:port`，以及端口为 22 时的
/// 裸 `host`。带通配符（`*` / `?`）和哈希过的条目（`|1|…`）一律跳过：宁可当成“没见过”
/// 重新学一次，也不要靠猜去匹配 —— 猜错的代价是误报“有人在中间”。
fn check_host_key_text(text: &str, host: &str, port: u16, key_type: &str, key: &str) -> HostKeyVerdict {
    let bracketed = format!("[{host}]:{port}");
    let plain = format!("{host}:{port}");
    let mut changed = false;
    for line in text.lines() {
        // 顺手吃掉 BOM：Windows 上记事本/Set-Content 存的 UTF-8 带 BOM，不处理的话
        // 第一行会变成「\u{feff}host:port …」，匹配不上 —— 就等于**默默跳过了一条钉扎记录**。
        let line = line.trim().trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('@') || line.starts_with("|1|")
        {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(patterns), Some(stored_type), Some(stored_key)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        // 不同算法不算变更：换算法是服务器的正常策略调整
        if stored_type != key_type {
            continue;
        }
        let hit = patterns.split(',').any(|pattern| {
            let pattern = pattern.trim();
            !pattern.contains('*')
                && !pattern.contains('?')
                && (pattern.eq_ignore_ascii_case(&plain)
                    || pattern.eq_ignore_ascii_case(&bracketed)
                    || (port == 22 && pattern.eq_ignore_ascii_case(host)))
        });
        if !hit {
            continue;
        }
        if stored_key == key {
            return HostKeyVerdict::Match;
        }
        changed = true;
    }
    if changed {
        HostKeyVerdict::Changed
    } else {
        HostKeyVerdict::Unknown
    }
}

/// SHA256 指纹，形态与 `ssh-keygen -lf` 一致（`SHA256:…`，去掉尾随的 `=`）。
fn host_key_fingerprint(session: &Session) -> Option<String> {
    let hash = session.host_key_hash(HashType::Sha256)?;
    Some(format!("SHA256:{}", base64(hash).trim_end_matches('=')))
}

/// 标准 base64（带 `=` 补齐）。只为公钥/指纹编码，不引依赖。
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (chunk[0] as u32) << 16 | b1 << 8 | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// 校验跳板机的主机公钥。**必须在发出任何凭据之前**调用（见 `open_session`）。
///
/// 顺序：用户系统 `~/.ssh/known_hosts` 精确命中即信任 → 本应用钉扎文件
/// （一致就通过，变了就拦，没见过就学下来）。
fn verify_host_key(session: &Session, spec: &SshConfig) -> Result<()> {
    let Some((raw, kind)) = session.host_key() else {
        // 拿不到主机公钥时不该把用户挡在外面（libssh2 只在极端情况下返回空）
        tracing::debug!(target: "dbmind::tunnel", "跳板机没有给出主机公钥，跳过校验");
        return Ok(());
    };
    let key_type = host_key_name(kind);
    let key = base64(raw);
    let fingerprint = host_key_fingerprint(session).unwrap_or_default();

    if let Some(system) = system_known_hosts_path() {
        if let Ok(text) = std::fs::read_to_string(&system) {
            if check_host_key_text(&text, &spec.host, spec.port, key_type, &key) == HostKeyVerdict::Match
            {
                return Ok(());
            }
        }
        // 哈希过的条目（`|1|salt|hash`）我们自己解不了，交给 libssh2；它说命中就是真的同一把公钥。
        // 它说 mismatch **不采信**：它的比对不看算法，同一台机器换了算法也会报 mismatch。
        if let Ok(mut hosts) = session.known_hosts() {
            if hosts.read_file(&system, KnownHostFileKind::OpenSSH).is_ok()
                && matches!(
                    hosts.check_port(&spec.host, spec.port, raw),
                    CheckResult::Match
                )
            {
                return Ok(());
            }
        }
    }

    let path = known_hosts_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    match check_host_key_text(&text, &spec.host, spec.port, key_type, &key) {
        HostKeyVerdict::Match => Ok(()),
        HostKeyVerdict::Changed => Err(host_key_changed(spec, &path, &fingerprint)),
        HostKeyVerdict::Unknown => {
            learn_host_key(&path, spec, key_type, &key, &fingerprint);
            Ok(())
        }
    }
}

/// 第一次连某台跳板机：把公钥记下来（TOFU）。
///
/// 写盘失败**不拦连接**：记不住只是下一次还要再学一遍，没有安全损失；
/// 而因为一个目录权限问题连不上数据库，才是真的坑人。
fn learn_host_key(path: &Path, spec: &SshConfig, key_type: &str, key: &str, fingerprint: &str) {
    let line = format!("{}:{} {key_type} {key}\n", spec.host, spec.port);
    let write = || -> std::io::Result<()> {
        paths::ensure_parent(path).map_err(|err| std::io::Error::other(err.message))?;
        if !path.exists() {
            std::fs::write(
                path,
                "# DBMind 记住的 SSH 跳板机公钥（每行 <host>:<port> <算法> <公钥>）\n\
                 # 跳板机换了密钥后，把对应那一行删掉，下次连接会重新记住。\n",
            )?;
        }
        let mut file = std::fs::OpenOptions::new().append(true).create(true).open(path)?;
        file.write_all(line.as_bytes())
    };
    match write() {
        Ok(()) => tracing::info!(
            target: "dbmind::tunnel",
            path = %path.display(),
            host = %spec.host,
            port = spec.port,
            fingerprint,
            "首次连接该跳板机，已记住主机公钥"
        ),
        Err(err) => tracing::warn!(
            target: "dbmind::tunnel",
            path = %path.display(),
            error = %err,
            "记住跳板机主机公钥失败（不影响本次连接）"
        ),
    }
}

fn host_key_changed(spec: &SshConfig, path: &Path, fingerprint: &str) -> DbMindError {
    conn_err(format!(
        "跳板机 {}:{} 的主机公钥和上次记住的不一样",
        spec.host, spec.port
    ))
    .with_detail(format!(
        "跳板机重装系统、换了密钥时会这样；但如果没人动过它，就要当心有人冒充它。\n\
         这次拿到的指纹：{fingerprint}\n\
         核对方式：`ssh-keyscan -p {port} {host} | ssh-keygen -lf -`（与上面这串比对）。\n\
         确认没问题后，把 {file} 里含 {host}:{port} 的那一行删掉，再连一次会重新记住。",
        port = spec.port,
        host = spec.host,
        file = path.display()
    ))
}

// ------------------------------------------------------------------ 会话池

/// 池里最多留几条已认证的 SSH 会话。
///
/// 与 [`HANDSHAKE_MAX`] 对齐：池只解决「握手一次、之后复用」，留太多会白占跳板机的
/// 会话配额（`MaxSessions`）。
const SESSION_POOL_MAX: usize = 8;

/// 在**池里借来的**会话上开第一条通道的超时。
///
/// 为什么比 `CONNECT_TIMEOUT` 短：池里的会话可能已经被跳板机回收（`ClientAliveInterval`
/// 到点踢人）或者链路的 NAT 表项过期，这时「通道打开请求」写出去却永远等不到回复。
/// 3 秒快速判死、重新握手（真机 2.7 秒），比在这儿干等 10 秒好。
const POOLED_CHANNEL_TIMEOUT: Duration = Duration::from_secs(3);

/// 池空时最多等这么久再决定握手。
///
/// 为什么需要等：上一条连接断开后，它的会话要等转发线程收尾、关通道才归还。客户端
/// （尤其是驱动重连）往往几毫秒内就发起下一条连接 —— 不等就会「明明池子里刚有一条
/// 会话，却又付了 2.7 秒握手」。等一小会儿的代价上限是 250ms，而且是并行发生的。
const POOL_WAIT: Duration = Duration::from_millis(250);

/// 池里的会话「睡」了多久就当作不可信、直接握手。
///
/// 闲置的 SSH 会话会被跳板机 `ClientAliveInterval` 踢掉（企业里常见 60~300 秒），
/// 链路设备的空闲表项也会过期。这类死会话发现不了（libssh2 的 keepalive 不主动发），
/// 只能等开通道时超时 —— 那就成了「3 秒超时 + 2.7 秒握手」，比不池化还慢。
/// 一分钟内用过的会话几乎一定还活着；更久的一律重新握手（与改造前同价，绝不更差）。
const POOL_FRESH: Duration = Duration::from_secs(60);

/// 池里的一条会话 + 它是什么时候归还的（见 [`POOL_FRESH`]）。
struct PooledSession {
    session: Session,
    idle_since: Instant,
}

/// 已认证、当前**没有活跃通道**的 SSH 会话池。
///
/// 为什么要有它：原来每条本地连接都要重新握手 + 认证 —— 真机实测**每条 2.7 秒**，
/// 而同一台 RDS 直连只要 0.5 秒。界面打开一条连接会并发发好几个元数据请求、
/// 每次重连也要重新握手，用户看到的就是「打开连接转好几秒、有时候快有时候慢」。
///
/// 为什么不做「一条会话多通道」：libssh2 的阻塞读会持有会话锁，多通道必须改成
/// select 事件循环（见 [`accept_loop`] 的注释），那是另一个量级的改动。这里只保证
/// **握手成本只付一次**：池里的会话仍是独占的（用完还回来），下次直接接着用。
struct SessionPool {
    /// 空闲会话栈：**后进先出**，取到的一定是最近归还的那条（也就最新鲜）。
    idle: Mutex<Vec<PooledSession>>,
    /// 有会话归还时唤醒正在 [`SessionPool::take`] 里等的线程。
    returned: Condvar,
}

impl SessionPool {
    fn new() -> Self {
        Self {
            idle: Mutex::new(Vec::new()),
            returned: Condvar::new(),
        }
    }

    /// 借一条会话：池里有就直接用（**不握手**），池空才建新的。
    ///
    /// 这里只做**不联网**的检查 —— 真正的探活是「开一条通道」，交给 [`forward`]：
    /// 它本来就要开通道，成功即证明会话可用，失败再换新会话重试一次。
    /// 不在这里发 keepalive：libssh2 的 keepalive 要配了间隔才会真发（`keepalive_send`
    /// 在间隔为 0 时是空操作），拿它当探活只会自欺欺人。
    fn take(&self, spec: &SshConfig) -> Result<Session> {
        let deadline = Instant::now() + POOL_WAIT;
        let mut idle = self.idle.lock().unwrap_or_else(|err| err.into_inner());
        loop {
            if let Some(entry) = idle.pop() {
                // 出锁再判断/返回：拿着锁做任何事都只是白挡住别的连接
                drop(idle);
                if entry.idle_since.elapsed() > POOL_FRESH {
                    tracing::debug!(
                        target: "dbmind::tunnel",
                        idle_secs = entry.idle_since.elapsed().as_secs(),
                        "池里的 SSH 会话闲置太久，改为重新握手"
                    );
                    // 栈是后进先出：最上面这条都过期了，下面只会更旧
                    return connect(spec);
                }
                if entry.session.authenticated() {
                    tracing::debug!(target: "dbmind::tunnel", "复用已认证的 SSH 会话");
                    return Ok(entry.session);
                }
                tracing::debug!(target: "dbmind::tunnel", "池里的 SSH 会话已失效，丢弃");
                idle = self.idle.lock().unwrap_or_else(|err| err.into_inner());
                continue;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            // poison 一样取回内部数据：池里只有「一堆会话」，没有会被 panic 写坏的不变量
            let (guard, _) = self
                .returned
                .wait_timeout(idle, left)
                .unwrap_or_else(|err| err.into_inner());
            idle = guard;
        }
        drop(idle);
        connect(spec)
    }

    /// 归还一条健康会话（调用方保证通道已关闭）。
    fn put(&self, session: Session) {
        {
            let mut idle = self.idle.lock().unwrap_or_else(|err| err.into_inner());
            if idle.len() < SESSION_POOL_MAX {
                idle.push(PooledSession {
                    session,
                    idle_since: Instant::now(),
                });
            }
            // 超出上限：session 在块尾 drop，等于关掉那条 SSH 连接
        }
        // 唤醒所有正在等会话的线程：它们会自己抢，抢不到就按超时走握手
        self.returned.notify_all();
    }

    /// 丢掉池里所有会话（隧道回收时调用：不然这些连接会一直占着跳板机的会话配额）。
    fn clear(&self) {
        let drained = {
            let mut idle = self.idle.lock().unwrap_or_else(|err| err.into_inner());
            std::mem::take(&mut *idle)
        };
        // 出锁之后再 drop：关闭 SSH 连接要发报文，不该占着池子的锁
        drop(drained);
    }
}

// ------------------------------------------------------------------ 隧道管理

/// 一条活着的隧道：本地端口 + 停止信号。
struct Tunnel {
    local_port: u16,
    stop: Arc<AtomicBool>,
    /// 日志用：跳板机与目标地址，出问题时能一眼看出是哪条隧道。
    label: String,
    /// 这条隧道共用的**已认证会话池**（隧道回收时跟着关闭）
    pool: Arc<SessionPool>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // 只置个标记：监听线程下一轮轮询就会退出并关掉监听端口
        self.stop.store(true, Ordering::Relaxed);
        // 池里空闲的会话立刻关掉：正转发中的那条连接自己有借用，配不到一起
        self.pool.clear();
        tracing::debug!(
            target: "dbmind::tunnel",
            label = %self.label,
            port = self.local_port,
            "SSH 隧道已回收"
        );
    }
}

struct Entry {
    tunnel: Arc<Tunnel>,
    last_used: Instant,
    /// 上次端口探活的时间（见 [`reuse`]）：探活要建一条本地 TCP，不能每次连库都探
    last_probe: Instant,
}

/// 端口探活的上限：一条本地回环连接，正常是微秒级；连不上就说明没人 listen。
const PROBE_TIMEOUT: Duration = Duration::from_millis(300);
/// 两次探活之间至少隔这么久（隧道是按连接复用的，连库频率很高）
const PROBE_MIN_GAP: Duration = Duration::from_secs(5);

/// 本地转发端口还有人在 listen 吗。
///
/// 为什么需要它：监听线程可能已经退出（`accept` 出错、被系统回收），但条目还留在缓存里，
/// `reuse()` 会把那个**没人接的端口**发给驱动，直到空闲 TTL 到点（默认 30 分钟）——
/// 用户看到的现象是「网络明明恢复了，还是一直连不上」，而且换一张表也一样。
fn port_alive(port: u16) -> bool {
    let addr = SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    match TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) {
        Ok(stream) => {
            // 探活连接立刻关掉：accept 那边会当一条正常转发连接处理，读到 EOF 就结束
            let _ = stream.shutdown(Shutdown::Both);
            true
        }
        Err(_) => false,
    }
}

/// 监听线程退出时把缓存里的这一条摘掉（见 [`port_alive`] 的注释：不摘会一直返回死端口）。
fn retire(key: &str, local_port: u16) {
    let mut map = lock_tunnels();
    // 只摘「同一个端口」的那条：并发重建时新隧道可能已经占了这个 key，别把好的删了
    if map.get(key).map(|entry| entry.tunnel.local_port) == Some(local_port) {
        map.remove(key);
        tracing::warn!(target: "dbmind::tunnel", port = local_port, "隧道监听已退出，从缓存摘除");
    }
}

fn tunnels() -> &'static Mutex<HashMap<String, Entry>> {
    static TUNNELS: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    TUNNELS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 这条连接该往哪个本地端口连；不需要隧道时返回 `None`。
///
/// **按「跳板机 + 目标地址」复用**：同一条连接的所有会话、以及它的影子连接
/// （跨库浏览时按需生成的记录）都落在同一条隧道上。
pub fn local_port_for(config: &ConnectionConfig) -> Result<Option<u16>> {
    let Some(spec) = SshConfig::from_config(config)? else {
        return Ok(None);
    };
    let target_host = config.host.clone().unwrap_or_default();
    let target_port = config.resolved_port().unwrap_or(0);
    if target_host.is_empty() || target_port == 0 {
        return Err(conn_err("SSH 隧道需要一个可转发的目标地址，但连接里没有主机或端口")
            .with_detail("隧道转发的是「跳板机视角下」的数据库地址：请在「基础信息」里\
                          填好主机与端口，再从跳板机上确认这个地址是通的。"));
    }
    let key = format!(
        "{}|->{target_host}:{target_port}",
        tunnel_key(config).unwrap_or_default()
    );

    let now = Instant::now();
    if let Some(entry) = reuse(&key, now) {
        return Ok(Some(entry));
    }

    // 建隧道要跑网络（最长 CONNECT_TIMEOUT），所以**不持锁**：
    // 持锁建隧道会让「另一条连接的握手」陪着一起等。
    let tunnel = open_tunnel(&spec, &target_host, target_port, &key)?;
    let mut map = lock_tunnels();
    // 并发下可能已经有人建好了：让先来的留下，自己这条会被 drop（Drop 会关掉监听）
    let entry = map.entry(key).or_insert(Entry {
        tunnel,
        last_used: now,
        last_probe: now,
    });
    entry.last_used = now;
    Ok(Some(entry.tunnel.local_port))
}

fn lock_tunnels() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    // 锁中毒说明别的线程在持锁时 panic 过；隧道表本身没有会被写坏的不变量，
    // 直接取回内部数据比让整个进程跟着崩要好。
    tunnels().lock().unwrap_or_else(|err| err.into_inner())
}

/// 命中已有隧道就续期并返回本地端口；顺带回收长期没人用的隧道。
fn reuse(key: &str, now: Instant) -> Option<u16> {
    // 持锁只做查表/续期，**不在这里探活**：探活是一次网络动作，持锁会拖住别的连接的握手
    let (port, need_probe) = {
        let mut map = lock_tunnels();
        // 0 = 不按空闲回收（内网常驻跳板机可以这么配，隧道一直热着）
        let ttl_secs = IDLE_TTL_SECS.load(Ordering::Relaxed);
        if ttl_secs > 0 {
            let ttl = Duration::from_secs(ttl_secs.min(u64::from(u32::MAX)));
            map.retain(|_, entry| now.duration_since(entry.last_used) < ttl);
        }
        let entry = map.get_mut(key)?;
        entry.last_used = now;
        let need = now.duration_since(entry.last_probe) >= PROBE_MIN_GAP;
        if need {
            entry.last_probe = now;
        }
        (entry.tunnel.local_port, need)
    };
    if !need_probe {
        return Some(port);
    }
    // 探活：监听线程已经退出的话，这个端口是个死端口 —— 摘掉让 caller 重建。
    // 网络断了、但监听线程还活着的情况探不出来，那种情况靠 `forward` 里的重新握手兜住。
    if port_alive(port) {
        return Some(port);
    }
    retire(key, port);
    tracing::warn!(
        target: "dbmind::tunnel",
        port,
        "本地转发端口已失效（监听线程已退出），重建隧道"
    );
    None
}

// ------------------------------------------------------------------ 建隧道

fn open_tunnel(
    spec: &SshConfig,
    target_host: &str,
    target_port: u16,
    key: &str,
) -> Result<Arc<Tunnel>> {
    let listener = TcpListener::bind((LOCAL_HOST, 0)).map_err(|err| {
        conn_err("本地端口分配失败，无法建立 SSH 隧道").with_detail(format!("{err}"))
    })?;
    let local_port = listener
        .local_addr()
        .map_err(|err| conn_err("读取本地转发端口失败").with_detail(format!("{err}")))?
        .port();

    // 会话池：这条隧道的所有转发连接共用，握手只付一次（见 [`SessionPool`]）
    let pool = Arc::new(SessionPool::new());

    // 先探一次：认证不通、跳板机到数据库不可达，都要**在这里**说清楚 ——
    // 否则驱动只会连一个没人转发的本地端口，报出来的是「连接被拒绝」，
    // 排查方向会被带到"数据库挂了"上。
    // 探活的会话本身是好的，直接放进池子：第一条转发连接就省掉一次握手。
    pool.put(probe(spec, target_host, target_port)?);

    // 用非阻塞 accept：线程要能被 Drop 的停止标记收掉，而阻塞 accept 只能靠
    // 再连一次自己才能唤醒（那会多出一条假连接）。20ms 的轮询对建连延迟无感。
    listener
        .set_nonblocking(true)
        .map_err(|err| conn_err("本地转发端口设置失败").with_detail(format!("{err}")))?;

    let stop = Arc::new(AtomicBool::new(false));
    let label = format!(
        "{}@{}:{} -> {target_host}:{target_port}",
        spec.user, spec.host, spec.port
    );
    // 线程句柄不保留（`?` 直接丢掉即 detach）：停止由 `stop` 标记驱动，不由 join 驱动
    {
        let stop = stop.clone();
        let spec = spec.clone();
        let target_host = target_host.to_string();
        let label = label.clone();
        let pool = pool.clone();
        let key = key.to_string();
        std::thread::Builder::new()
            .name(format!("ssh-tunnel-{local_port}"))
            .spawn(move || {
                accept_loop(listener, stop, spec, target_host, target_port, label, pool, key)
            })
            .map_err(|err| conn_err("SSH 隧道线程创建失败").with_detail(format!("{err}")))?;
    }
    tracing::info!(target: "dbmind::tunnel", label = %label, port = local_port, "SSH 隧道已建立");
    Ok(Arc::new(Tunnel {
        local_port,
        stop,
        label,
        pool,
    }))
}

/// 建一条**新**会话（带握手限流）。
///
/// 限流名额**只在握手期间持有**（块作用域）—— 曾经把租约写进 `forward` 的整个作用域，
/// 结果租约跟着连接一直活到会话关闭，8 条并发会话占满名额后所有新建连全部死等。
fn connect(spec: &SshConfig) -> Result<Session> {
    let _lease = HandshakeLease::acquire();
    open_session(spec)
}

/// 给 SSH 底层 TCP 打开 keepalive（原因见 [`KEEPALIVE_TIME`]）。
///
/// 用**内核级** keepalive 而不是 libssh2 的 `set_keepalive`：后者要有人在循环里调
/// `keepalive_send()` 才会真发包，而我们的转发线程是「本地没数据就睡 1ms」的
/// 非阻塞轮询，根本没有那个时机 —— 拿它当探活只会自欺欺人
/// （`SessionPool::take` 里那条注释讲的就是这件事）。
///
/// 失败只记 debug：受控环境（企业安全软件接管 socket）可能不让设，
/// 那样只是少一层自愈，连接本身照常能用。
fn enable_keepalive(tcp: &TcpStream) {
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(KEEPALIVE_TIME)
        .with_interval(KEEPALIVE_INTERVAL);
    if let Err(err) = socket2::SockRef::from(tcp).set_tcp_keepalive(&keepalive) {
        tracing::debug!(
            target: "dbmind::tunnel",
            error = %err,
            "设置 TCP keepalive 失败（连接可用，但换网后可能不会自愈）"
        );
    }
}

/// 把这条连接的隧道**主动作废**（下次连库重新握手）。
///
/// 什么时候调：链路级失败之后（连不上/会话全丢/探活判死）。换网后旧 TCP 是半开的，
/// 本地监听照样 listen、`port_alive` 照样通过、池里的会话 `authenticated()` 也还是
/// true —— 不主动拆，新连接会被继续送进死链路，用户只能重启 VPN 客户端。
///
/// 只作废**完全匹配**的那一条 key，不碰别的连接。返回是否真的拆了一条。
pub fn invalidate(config: &ConnectionConfig) -> bool {
    let Some(key) = tunnel_key(config) else {
        return false;
    };
    let removed = { lock_tunnels().remove(&key) };
    let Some(entry) = removed else {
        return false;
    };
    // 必须**显式**置停止位并清池：监听线程自己拿着 Arc，只从 map 里摘掉不会触发
    // `Tunnel::drop`，端口会继续 accept，新连接又被送进死链路。
    entry.tunnel.stop.store(true, Ordering::Relaxed);
    entry.tunnel.pool.clear();
    tracing::warn!(
        target: "dbmind::tunnel",
        label = %entry.tunnel.label,
        port = entry.tunnel.local_port,
        "链路级失败：隧道已作废，下次连接会重新握手"
    );
    true
}

/// 建一条 SSH 会话（握手 + 认证）。
fn open_session(spec: &SshConfig) -> Result<Session> {
    let address = format!("{}:{}", spec.host, spec.port);
    let socket = address
        .to_socket_addrs()
        .map_err(|err| {
            conn_err(format!("跳板机地址无法解析：{address}"))
                .with_detail(format!("{err}"))
        })?
        .next()
        .ok_or_else(|| conn_err(format!("跳板机地址无法解析：{address}")))?;
    let tcp = TcpStream::connect_timeout(&socket, CONNECT_TIMEOUT).map_err(|err| {
        conn_err(format!("连不上 SSH 跳板机 {address}")).with_detail(format!(
            "{err}（{} 秒超时；确认地址端口可达、防火墙放行）",
            CONNECT_TIMEOUT.as_secs()
        ))
    })?;
    tcp.set_nodelay(true).ok();
    enable_keepalive(&tcp);

    let mut session = Session::new().map_err(|err| {
        DbMindError::new(ErrorCode::Internal, "SSH 会话初始化失败").with_detail(format!("{err}"))
    })?;
    session.set_tcp_stream(tcp);
    // 超时交给 libssh2：**不要**在 TCP 流上设读超时 —— 那会让空闲一段时间的隧道
    // 被误判成断开（数据库长连接经常长时间没有字节往来）。
    session.set_timeout(CONNECT_TIMEOUT.as_millis() as u32);
    session.handshake().map_err(|err| {
        conn_err(format!("SSH 握手失败：{address}")).with_detail(format!(
            "{err}（对方不是 SSH 服务、或版本/算法不被支持时都会走到这里）"
        ))
    })?;

    // 主机公钥必须在**把任何凭据交出去之前**核验：等口令发完了再拦就没有意义了。
    verify_host_key(&session, spec)?;

    match &spec.auth {
        SshAuth::Password(password) => {
            password_auth(&session, spec, &address, password)?;
        }
        SshAuth::Key { path, passphrase } => {
            let key = Path::new(path);
            if !key.is_file() {
                return Err(conn_err(format!("SSH 私钥文件不存在：{path}"))
                    .with_detail("填私钥文件的**完整路径**；Windows 上注意别把路径写成 \
                                 只有用户 shell 才认的 `~`。"));
            }
            session
                .userauth_pubkey_file(&spec.user, None, key, passphrase.as_deref())
                .map_err(|err| {
                    auth_err(spec, &address, &err.to_string()).with_detail(format!(
                        "私钥：{path}。私钥必须是 OpenSSH 格式（`ssh-keygen -p -f <私钥>` \
                         可原地转换）；带口令短语的私钥要把口令短语一并填上。"
                    ))
                })?;
        }
    }
    if !session.authenticated() {
        return Err(conn_err(format!(
            "SSH 认证未通过：{}@{}",
            spec.user, address
        ))
        .with_detail(
            "跳板机拒绝了这组凭据。逐个确认：用户名是否正确、口令/私钥口令短语是否正确、\
             私钥是否已把公钥加到跳板机的 authorized_keys。",
        ));
    }
    // 认证之后不设超时：空闲的长连接经常长时间没有字节往来，socket 超时会把它们误判成断开。
    // 需要超时的地方（开通道、探活）由调用方临时设一个，用完恢复 0。
    session.set_timeout(0);
    HANDSHAKE_TOTAL.fetch_add(1, Ordering::Relaxed);
    Ok(session)
}

fn auth_err(spec: &SshConfig, address: &str, detail: &str) -> DbMindError {
    let how = match spec.auth {
        SshAuth::Password(_) => "口令",
        SshAuth::Key { .. } => "私钥",
    };
    conn_err(format!("SSH {how}认证失败：{}@{}", spec.user, address)).with_detail(detail.to_string())
}

/// 口令认证；被拒时如果服务器只认 keyboard-interactive，就用同一份口令再试一次。
///
/// 为什么要这一手：加固过的 sshd 常见配置是 `PasswordAuthentication no` +
/// `KbdInteractiveAuthentication yes`（堡垒机、等保环境尤其多）。libssh2 的
/// `userauth_password` **不会**自动回退到 keyboard-interactive（读它的 userauth.c 确认过），
/// 于是用户会遇到「口令明明是对的、命令行 ssh 也能登」，在我们这里却一条认证失败。
fn password_auth(session: &Session, spec: &SshConfig, address: &str, password: &str) -> Result<()> {
    let Err(err) = session.userauth_password(&spec.user, password) else {
        return Ok(());
    };
    let first = err.to_string();
    if !offers_keyboard_interactive(session, &spec.user) {
        return Err(auth_err(spec, address, &first));
    }
    let mut prompt = SamePasswordPrompt { password };
    match session.userauth_keyboard_interactive(&spec.user, &mut prompt) {
        Ok(()) => {
            tracing::debug!(
                target: "dbmind::tunnel",
                user = %spec.user,
                "口令认证被拒，已用 keyboard-interactive 通过"
            );
            Ok(())
        }
        Err(kbi) => Err(auth_err(
            spec,
            address,
            &format!("口令：{first}\nkeyboard-interactive：{kbi}"),
        )),
    }
}

/// 服务器是不是提供 keyboard-interactive（`auth_methods` 回来的是逗号分隔的名字）。
fn offers_keyboard_interactive(session: &Session, user: &str) -> bool {
    session
        .auth_methods(user)
        .map(|methods| {
            methods
                .split(',')
                .any(|method| method.trim() == "keyboard-interactive")
        })
        .unwrap_or(false)
}

/// 键盘交互的应答器：每个提问都回同一个口令。
///
/// 只覆盖「服务器把口令认证伪装成 keyboard-interactive」这一种情况（最常见）。
/// 真要求 OTP/动态令牌的提问这里给不出答案，填了也是认证失败 —— 不会假装成功。
struct SamePasswordPrompt<'a> {
    password: &'a str,
}

impl KeyboardInteractivePrompt for SamePasswordPrompt<'_> {
    fn prompt<'a>(
        &mut self,
        _username: &str,
        _instructions: &str,
        prompts: &[ssh2::Prompt<'a>],
    ) -> Vec<String> {
        prompts.iter().map(|_| self.password.to_string()).collect()
    }
}

/// 探一次：握手 + 认证 + **真的开一条转发通道**。
///
/// 只握手不试通道是不够的：认证成功但「跳板机到数据库」不通时，问题会拖到驱动那边才暴露，
/// 用户看到的是一句「连接被拒绝」，看不出根因在跳板机的路由/白名单上。
///
/// 返回探活的会话：调用方把它放进池子复用，省掉「建隧道」与「第一条连接」之间的第二次握手。
fn probe(spec: &SshConfig, target_host: &str, target_port: u16) -> Result<Session> {
    let session = connect(spec)?;
    let channel = session
        .channel_direct_tcpip(target_host, target_port, None)
        .map_err(|err| {
            conn_err(format!(
                "跳板机无法转发到 {target_host}:{target_port}"
            ))
            .with_detail(format!(
                "{err}\n注意：隧道里的目标地址要填**跳板机视角**下的数据库地址 \
                 —— 从你本机能连通的地址未必是跳板机能连通的那个。"
            ))
        })?;
    drop(channel);
    Ok(session)
}

// ------------------------------------------------------------------ 转发

#[allow(clippy::too_many_arguments)]
fn accept_loop(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    spec: SshConfig,
    target_host: String,
    target_port: u16,
    label: String,
    pool: Arc<SessionPool>,
    key: String,
) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _peer)) => {
                // 每条转发连接一条线程、**独占**一条 SSH 会话（从池子里借，用完还回去）。
                //
                // 为什么仍然独占：libssh2 的**阻塞读会一直持有会话锁**，两个方向各占一条
                // 线程时，若读线程正在等数据，写线程就拿不到锁，「本地发往数据库」的字节
                // 会一直堆着发不出去（表现为连上后第一条语句就没有响应）。真正的多路复用
                // 要改成 select 事件循环，是另一个量级的改动；先拿到「握手只付一次」的收益。
                let spec = spec.clone();
                let target_host = target_host.clone();
                let forward_label = label.clone();
                let pool = pool.clone();
                let spawned = std::thread::Builder::new()
                    .name("ssh-forward".to_string())
                    .spawn(move || {
                        if let Err(err) = forward(stream, &pool, &spec, &target_host, target_port) {
                            tracing::warn!(
                                target: "dbmind::tunnel",
                                label = %forward_label,
                                error = %err,
                                "一条 SSH 转发连接结束"
                            );
                        }
                    });
                if let Err(err) = spawned {
                    tracing::warn!(
                        target: "dbmind::tunnel",
                        label = %label,
                        error = %err,
                        "SSH 转发线程创建失败"
                    );
                }
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => std::thread::sleep(ACCEPT_POLL),
            Err(err) => {
                tracing::warn!(
                    target: "dbmind::tunnel",
                    label = %label,
                    error = %err,
                    "SSH 隧道监听中断，隧道关闭"
                );
                break;
            }
        }
    }
    // 退出前把自己从缓存里摘掉：否则 `reuse()` 会一直把这个**没人 listen 的端口**发给驱动，
    // 网络恢复了也连不上（最长能拖到空闲 TTL，默认 30 分钟）。
    let port = listener.local_addr().map(|addr| addr.port()).unwrap_or(0);
    retire(&key, port);
    tracing::debug!(target: "dbmind::tunnel", label = %label, "SSH 隧道已关闭");
}

fn forward(
    mut local: TcpStream,
    pool: &SessionPool,
    spec: &SshConfig,
    target_host: &str,
    target_port: u16,
) -> std::io::Result<()> {
    // 借会话：池里有就直接接着用（不握手，这就是 2.7 秒 → 0 秒 的来源）。
    // 建会话时的握手限流在 [`connect`] 里，名额只在握手期间持有。
    let mut session = pool.take(spec).map_err(session_err)?;
    let _ = local.set_nonblocking(true);
    let _ = local.set_nodelay(true);

    // 通道要能干净地关掉才能把会话放回池子，所以这里把「开通道 → 搬运 → 关通道」
    // 收在一个作用域里：`Channel` 借用着 `Session`，出作用域才能还回去。
    let (result, closed) = {
        // 第一次开通道顺便当探活：池里的会话可能已经被跳板机回收（只给短超时）
        session.set_blocking(true);
        session.set_timeout(POOLED_CHANNEL_TIMEOUT.as_millis() as u32);
        let mut channel = match session.channel_direct_tcpip(target_host, target_port, None) {
            Ok(channel) => channel,
            Err(err) => {
                tracing::debug!(
                    target: "dbmind::tunnel",
                    error = %err,
                    "池里借来的 SSH 会话开不了通道，重新握手"
                );
                drop(session);
                session = connect(spec).map_err(session_err)?;
                session.set_blocking(true);
                session.set_timeout(CONNECT_TIMEOUT.as_millis() as u32);
                session
                    .channel_direct_tcpip(target_host, target_port, None)
                    .map_err(|err| std::io::Error::other(format!("SSH 转发通道打开失败：{err}")))?
            }
        };
        // 通道已建立：转非阻塞，交给 [`pump`] 在单线程里交替推进两个方向
        session.set_blocking(false);
        let result = pump(&mut local, &mut channel);
        // 关之前转回阻塞：非阻塞下的 close 只是把消息塞进缓冲区就返回，通道会留在半开状态
        session.set_blocking(true);
        let closed = channel.send_eof().is_ok() && channel.close().is_ok();
        (result, closed)
    };
    let _ = local.shutdown(Shutdown::Both);

    // 通道正常关闭、会话仍在认证态，才值得还回池子。
    // `pump` 把通道读写错误都当「连接结束」（返回 Ok），所以看不了 result —— 用关闭结果兜底，
    // 真还回去一条半死的会话，下一次连接开通道失败会重新握手，代价有界。
    if closed && session.authenticated() {
        pool.put(session);
    } else {
        tracing::debug!(target: "dbmind::tunnel", "转发结束后的 SSH 会话已丢弃");
    }
    result
}

/// 把建会话失败转成本地连接层面的 IO 错误。
///
/// 建会话失败不该让整个隧道倒掉（跳板机抖一下、并发上限到了都可能发生）：
/// 这条本地连接关掉，驱动那边会看到连接断开并自行重试。
fn session_err(err: DbMindError) -> std::io::Error {
    std::io::Error::other(format!("SSH 会话建立失败：{}", err.message))
}

/// 远端一侧（隧道对面）需要的能力：读写 + 主动告诉对方「我不再发了」。
///
/// 抽成 trait 只为一件事：让 [`pump`] 能在**不连 SSH** 的情况下被单测覆盖。
/// 这段双向搬运是本模块里唯一自己写的协议逻辑（其余都交给 libssh2），
/// 而本机不一定随时有跳板机可连 —— 把它绑死在 `ssh2::Channel` 上就等于放弃了测试。
trait Remote: Read + Write {
    /// 本端不再发送数据（SSH 的 `send_eof`；普通 TCP 就是半关闭写方向）。
    fn finish_write(&mut self);
}

impl Remote for ssh2::Channel {
    fn finish_write(&mut self) {
        let _ = self.send_eof();
    }
}

impl Remote for TcpStream {
    fn finish_write(&mut self) {
        // 测试用：本地回环对上就是半关闭写方向，效果与 SSH 的 EOF 等价
        let _ = self.shutdown(Shutdown::Write);
    }
}

/// 单个线程里双向搬运字节。
///
/// 为什么不用「两条线程 + `std::io::copy`」：libssh2 的通道读写都会持有会话锁，
/// 阻塞模式下读线程会一直占着锁，写方向就永远等不到机会（详见 [`accept_loop`] 的注释）。
/// 这里把会话设成非阻塞，两个方向在同一个循环里交替推进，谁的锁谁用一下就放。
fn pump<A, B>(local: &mut A, channel: &mut B) -> std::io::Result<()>
where
    A: Read + Write,
    B: Remote,
{
    let mut to_remote: Vec<u8> = Vec::new();
    let mut to_local: Vec<u8> = Vec::new();
    let mut local_open = true;
    let mut remote_open = true;
    let mut buf = vec![0u8; 32 * 1024];

    loop {
        let mut moved = false;

        // 本地 → 远端。远端积压到高水位时先别读本地，避免内存无上限增长。
        if local_open && to_remote.len() < BUFFER_HIGH_WATER {
            match local.read(&mut buf) {
                Ok(0) => {
                    local_open = false;
                    channel.finish_write();
                }
                Ok(n) => {
                    to_remote.extend_from_slice(&buf[..n]);
                    moved = true;
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {}
                Err(_) => {
                    local_open = false;
                    channel.finish_write();
                }
            }
        }
        while !to_remote.is_empty() {
            match channel.write(&to_remote) {
                Ok(0) => break,
                Ok(n) => {
                    to_remote.drain(..n);
                    moved = true;
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                Err(_) => return Ok(()),
            }
        }

        // 远端 → 本地。本地积压到高水位时停止从远端读（背压传给数据库）。
        if remote_open && to_local.len() < BUFFER_HIGH_WATER {
            loop {
                match channel.read(&mut buf) {
                    Ok(0) => {
                        remote_open = false;
                        break;
                    }
                    Ok(n) => {
                        to_local.extend_from_slice(&buf[..n]);
                        moved = true;
                        if to_local.len() >= BUFFER_HIGH_WATER {
                            break;
                        }
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => {
                        remote_open = false;
                        break;
                    }
                }
            }
        }
        while !to_local.is_empty() {
            match local.write(&to_local) {
                Ok(0) => break,
                Ok(n) => {
                    to_local.drain(..n);
                    moved = true;
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                Err(_) => return Ok(()),
            }
        }

        if !local_open && !remote_open && to_remote.is_empty() && to_local.is_empty() {
            return Ok(());
        }
        if !moved {
            std::thread::sleep(IDLE_POLL);
        }
    }
}

/// 关闭所有隧道（进程退出前的清场；也便于测试之间互不干扰）。
pub fn shutdown_all() {
    let mut map = lock_tunnels();
    let count = map.len();
    map.clear();
    if count > 0 {
        tracing::debug!(target: "dbmind::tunnel", count, "已关闭全部 SSH 隧道");
    }
}

/// 描述一条连接的隧道（给日志与诊断用，不含任何凭据）。
pub fn describe(config: &ConnectionConfig) -> Option<String> {
    let ssh = ssh_object(config)?;
    if !ssh.get("enabled").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let target = match config.resolved_port() {
        Some(port) => format!(
            "{}:{}",
            config.host.as_deref().unwrap_or("-"),
            port
        ),
        None => config.host.as_deref().unwrap_or("-").to_string(),
    };
    Some(format!(
        "SSH {}@{}:{} → {target}",
        text(ssh, "user"),
        text(ssh, "host"),
        port_of(ssh)
    ))
}

/// 组装 `extra.ssh`。
///
/// `existing` 是这条连接**已存的那份** `extra`（新建时给 `None`）。
///
/// 两类字段的规矩不一样，这里必须分开处理：
///
/// - 地址 / 端口 / 用户 / 认证方式 / 私钥路径：**以请求为准**。详情接口会把它们回显给
///   界面，界面保存时又原样发回来，所以「清空」能被如实表达。
/// - 口令 / 私钥口令短语：**空 = 沿用已存的那份**。详情接口永远不返回它们
///   （和主口令同一条规矩），若把空串当清空，用户在编辑页里没动过 SSH 口令，
///   保存一次就把已存的弄丢了。
pub fn ssh_extra_from_body(body: &Value, existing: Option<&Value>) -> Option<Value> {
    let previous = existing.and_then(|extra| extra.get(EXTRA_SSH));
    let field = |key: &str| -> String {
        body.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    // 口令不做 trim：前后空白可能是口令的一部分
    let secret = |key: &str| -> String {
        body.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let previous_secret = |key: &str| -> Option<String> {
        previous
            .and_then(|ssh| ssh.get(key))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };

    let enabled = body
        .get("sshEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut ssh = Map::new();
    ssh.insert("enabled".to_string(), json!(enabled));

    let host = field("sshHost");
    if !host.is_empty() {
        ssh.insert("host".to_string(), json!(host));
    }
    if let Some(port) = body
        .get("sshPort")
        .and_then(Value::as_u64)
        .filter(|port| *port > 0 && *port <= u16::MAX as u64)
    {
        ssh.insert("port".to_string(), json!(port));
    }
    let user = field("sshUser");
    if !user.is_empty() {
        ssh.insert("user".to_string(), json!(user));
    }
    let auth_type = {
        let value = field("sshAuthType");
        if value.is_empty() {
            "password".to_string()
        } else {
            value
        }
    };
    ssh.insert("authType".to_string(), json!(auth_type));

    let key_path = field("sshKeyPath");
    if !key_path.is_empty() {
        ssh.insert("keyPath".to_string(), json!(key_path));
    }
    // 口令只在**这次真的填了**的时候覆盖；否则沿用已存的
    let password = secret("sshPassword");
    if let Some(value) = non_empty(password).or_else(|| previous_secret("password")) {
        ssh.insert("password".to_string(), json!(value));
    }
    let passphrase = secret("sshKeyPassphrase");
    if let Some(value) = non_empty(passphrase).or_else(|| previous_secret("keyPassphrase")) {
        ssh.insert("keyPassphrase".to_string(), json!(value));
    }

    // 关掉开关时也保留这几项：下次再打开不必从头填一遍
    Some(Value::Object(ssh))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConnectionKind;

    fn base() -> ConnectionConfig {
        ConnectionConfig::new("公司库", ConnectionKind::Mysql)
            .with_host("10.0.0.9")
            .with_port(3306)
    }

    fn with_ssh(body: Value) -> ConnectionConfig {
        let mut config = base();
        config.extra = Some(json!({ EXTRA_SSH: body }));
        config
    }

    /// 没配 SSH 的连接**一点都不能变**：指纹为空、端口解析为空。
    #[test]
    fn 没开隧道时不给指纹也不解析端口() {
        assert_eq!(tunnel_key(&base()), None);
        assert_eq!(local_port_for(&base()).unwrap(), None);
        // 只关掉开关也一样
        let off = with_ssh(json!({ "enabled": false, "host": "jump", "user": "root" }));
        assert_eq!(tunnel_key(&off), None);
        assert_eq!(local_port_for(&off).unwrap(), None);
    }

    /// 链路级失败要能**主动作废**隧道（换网后旧端口还在 listen，不作废的话新连接
    /// 全被送进死链路，用户只能重启 VPN 客户端 —— 真机 2026-10-27）。
    /// 没缓存过隧道时必须安全返回 false：这条路径是在「已经连不上」时调的，
    /// 不能自己再 panic 一次。
    #[test]
    fn 作废隧道对没建过的连接是空操作() {
        let config = with_ssh(json!({
            "enabled": true, "host": "jump.invalid", "port": 2222,
            "user": "deploy", "authType": "password", "password": "x"
        }));
        assert!(!invalidate(&config), "没建过隧道应返回 false");
        // 没开 SSH 的连接更不能有任何动作
        assert!(!invalidate(&base()));
    }

    /// keepalive 设不上也**不能影响建连**（企业安全软件接管 socket 时可能被拒）。
    #[test]
    fn keepalive_设置不影响已有连接() {
        let listener = std::net::TcpListener::bind((LOCAL_HOST, 0)).expect("bind");
        let addr = listener.local_addr().expect("addr");
        let accepted = std::thread::spawn(move || {
            let _ = listener.accept();
        });
        let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).expect("connect");
        enable_keepalive(&tcp);
        drop(tcp);
        accepted.join().expect("join");
    }

    /// 指纹里**不能有口令**：它要进会话键（会出现在日志与宿主协议里）。
    #[test]
    fn 隧道指纹不含口令() {
        let config = with_ssh(json!({
            "enabled": true, "host": "jump", "port": 2222,
            "user": "deploy", "authType": "password", "password": "s3cr3t"
        }));
        let key = tunnel_key(&config).expect("开了隧道就该有指纹");
        assert!(key.contains("deploy@jump:2222"), "指纹要能读懂：{key}");
        assert!(!key.contains("s3cr3t"), "口令绝不能进指纹：{key}");
    }

    #[test]
    fn base64_编码与标准一致() {
        // RFC 4648 的测试向量
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// 已知主机比对：同一主机、同一算法、公钥不同才算「变了」。
    #[test]
    fn 已知主机比对按主机端口算法三条对齐() {
        let key_a = "AAAAB3NzaC1yc2EAAAADAQAB-aaaa";
        let key_b = "AAAAB3NzaC1yc2EAAAADAQAB-bbbb";
        let text = format!(
            "# 注释行\n\
             @revoked * {}\n\
             jump.example.com:2222 ssh-ed25519 {key_a}\n\
             [other.example.com]:22 ssh-rsa {key_a}\n\
             |1|abcd|efgh ssh-rsa {key_a}\n",
            "AAAAB3NzaC1yc2E"
        );

        // 精确命中
        assert_eq!(
            check_host_key_text(&text, "jump.example.com", 2222, "ssh-ed25519", key_a),
            HostKeyVerdict::Match
        );
        // OpenSSH 的 [host]:port 写法也认
        assert_eq!(
            check_host_key_text(&text, "other.example.com", 22, "ssh-rsa", key_a),
            HostKeyVerdict::Match
        );
        // 同一主机同一算法、公钥不同 ⇒ 变了
        assert_eq!(
            check_host_key_text(&text, "jump.example.com", 2222, "ssh-ed25519", key_b),
            HostKeyVerdict::Changed
        );
        // 同一主机**换了算法** ⇒ 不算变了（服务器正常调整策略）
        assert_eq!(
            check_host_key_text(&text, "jump.example.com", 2222, "ssh-rsa", key_a),
            HostKeyVerdict::Unknown
        );
        // 端口不同 ⇒ 不同主机
        assert_eq!(
            check_host_key_text(&text, "jump.example.com", 22, "ssh-ed25519", key_a),
            HostKeyVerdict::Unknown
        );
        // 哈希条目（|1|…）自己解不了，当没见过；@revoked 也不参与匹配
        assert_eq!(
            check_host_key_text(&text, "hashed.example.com", 22, "ssh-rsa", key_a),
            HostKeyVerdict::Unknown
        );
    }

    /// 带 BOM 的文件第一行也必须能被认出来（否则等于那条钉扎记录不存在，
    /// 换了密钥也发现不了 —— 真机踩到过）。
    #[test]
    fn 带_bom_的第一行也参与比对() {
        let text = "\u{feff}jump:2222 ssh-ed25519 AAAA-key\n";
        assert_eq!(
            check_host_key_text(text, "jump", 2222, "ssh-ed25519", "AAAA-key"),
            HostKeyVerdict::Match
        );
        assert_eq!(
            check_host_key_text(text, "jump", 2222, "ssh-ed25519", "BBBB-other"),
            HostKeyVerdict::Changed
        );
    }

    /// 端口 22 时允许裸 `host` 写法（OpenSSH 默认就是这种），其它端口不接受裸 host。
    #[test]
    fn 端口_22_接受裸主机名写法() {
        let text = "jump ssh-ed25519 AAAAB3NzaC1lZDI1NTE5\n";
        assert_eq!(
            check_host_key_text(text, "jump", 22, "ssh-ed25519", "AAAAB3NzaC1lZDI1NTE5"),
            HostKeyVerdict::Match
        );
        assert_eq!(
            check_host_key_text(text, "jump", 2222, "ssh-ed25519", "AAAAB3NzaC1lZDI1NTE5"),
            HostKeyVerdict::Unknown
        );
    }

    /// 用私钥时，换一把私钥必须换一条隧道（否则改了配置还在用旧隧道）。
    #[test]
    fn 换私钥就换隧道() {
        let a = with_ssh(json!({
            "enabled": true, "host": "jump", "user": "deploy",
            "authType": "key", "keyPath": "/home/a/.ssh/id_rsa"
        }));
        let b = with_ssh(json!({
            "enabled": true, "host": "jump", "user": "deploy",
            "authType": "key", "keyPath": "/home/a/.ssh/id_ed25519"
        }));
        assert_ne!(tunnel_key(&a), tunnel_key(&b));
    }

    /// 启用但填不全时，要在**展开配置**这一步就报错，并且说清缺什么。
    #[test]
    fn 配置不全时报错说清缺什么() {
        let no_host = with_ssh(json!({ "enabled": true, "user": "deploy" }));
        let err = SshConfig::from_config(&no_host).unwrap_err();
        assert!(err.message.contains("地址或用户名"), "{}", err.message);

        let no_password = with_ssh(json!({
            "enabled": true, "host": "jump", "user": "deploy", "authType": "password"
        }));
        let err = SshConfig::from_config(&no_password).unwrap_err();
        assert!(err.message.contains("口令是空的"), "{}", err.message);

        let no_key = with_ssh(json!({
            "enabled": true, "host": "jump", "user": "deploy", "authType": "key"
        }));
        let err = SshConfig::from_config(&no_key).unwrap_err();
        assert!(err.message.contains("私钥路径"), "{}", err.message);
    }

    /// 默认端口是 22，非法端口也用 22（宁可报连接失败，也别拼出一个不可能的端口）。
    #[test]
    fn 跳板机端口缺省与非法值都落回_22() {
        let default_port = with_ssh(json!({ "enabled": true, "host": "jump", "user": "u", "password": "p" }));
        let spec = SshConfig::from_config(&default_port).unwrap().unwrap();
        assert_eq!(spec.port, 22);

        let bogus = with_ssh(json!({
            "enabled": true, "host": "jump", "user": "u", "password": "p", "port": 99999
        }));
        let spec = SshConfig::from_config(&bogus).unwrap().unwrap();
        assert_eq!(spec.port, 22);
    }

    /// 改写只动 host / port：库名、口令、驱动参数、SSH 配置都要原样留着
    /// （驱动参数是连库必需的，SSH 配置丢了下次会话键就算错）。
    #[test]
    fn 改写连库地址不动别的字段() {
        let mut config = base();
        config.database = Some("shop".to_string());
        config.username = Some("app".to_string());
        config.password = Some("pw".to_string());
        config.extra = Some(json!({
            EXTRA_SSH: { "enabled": true, "host": "jump", "user": "deploy", "password": "p" },
            "params": { "trustServerCertificate": "true" }
        }));

        let rewritten = with_local_endpoint(&config, 49152);
        assert_eq!(rewritten.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(rewritten.port, Some(49152));
        assert_eq!(rewritten.database.as_deref(), Some("shop"));
        assert_eq!(rewritten.username.as_deref(), Some("app"));
        assert_eq!(rewritten.password.as_deref(), Some("pw"));
        assert!(rewritten.extra.as_ref().unwrap().get("params").is_some());
        assert!(rewritten.extra.as_ref().unwrap().get(EXTRA_SSH).is_some());
        // 原配置不能被改：页面显示与影子连接都还读它
        assert_eq!(config.host.as_deref(), Some("10.0.0.9"));
        assert_eq!(config.port, Some(3306));
    }

    /// 「空口令 = 沿用已存的那份」：编辑页不回显口令，保存一次不能把它弄丢。
    #[test]
    fn 空口令沿用已存的那份() {
        let existing = json!({
            EXTRA_SSH: {
                "enabled": true, "host": "jump", "user": "deploy",
                "authType": "password", "password": "old-secret"
            }
        });
        // 用户只改了端口，口令框是空的
        let body = json!({
            "sshEnabled": true, "sshHost": "jump", "sshPort": 2200,
            "sshUser": "deploy", "sshAuthType": "password", "sshPassword": ""
        });
        let ssh = ssh_extra_from_body(&body, Some(&existing)).unwrap();
        assert_eq!(ssh["password"], json!("old-secret"), "空口令必须沿用已存的那份");
        assert_eq!(ssh["port"], json!(2200), "非机密字段以请求为准");

        // 真的填了新口令就换掉
        let body = json!({
            "sshEnabled": true, "sshHost": "jump", "sshUser": "deploy",
            "sshAuthType": "password", "sshPassword": "new-secret"
        });
        let ssh = ssh_extra_from_body(&body, Some(&existing)).unwrap();
        assert_eq!(ssh["password"], json!("new-secret"));
    }

    /// 私钥口令短语同一条规矩；换成私钥认证时，已存的口令短语也留着
    /// （用户可能只是临时切密码认证试一下）。
    #[test]
    fn 空口令短语沿用已存的那份() {
        let existing = json!({
            EXTRA_SSH: { "enabled": true, "host": "jump", "user": "d", "authType": "key",
                         "keyPath": "/k/id_rsa", "keyPassphrase": "phrase" }
        });
        let body = json!({
            "sshEnabled": true, "sshHost": "jump", "sshUser": "d",
            "sshAuthType": "key", "sshKeyPath": "/k/id_rsa", "sshKeyPassphrase": ""
        });
        let ssh = ssh_extra_from_body(&body, Some(&existing)).unwrap();
        assert_eq!(ssh["keyPassphrase"], json!("phrase"));
    }

    /// 关掉开关后，地址与用户要留着：下次再打开不必从头填。
    #[test]
    fn 关掉开关也保留已填的信息() {
        let body = json!({
            "sshEnabled": false, "sshHost": "jump", "sshPort": 2200, "sshUser": "deploy"
        });
        let ssh = ssh_extra_from_body(&body, None).unwrap();
        assert_eq!(ssh["enabled"], json!(false));
        assert_eq!(ssh["host"], json!("jump"));
        assert_eq!(ssh["user"], json!("deploy"));
        // 但关着的时候不该被当成"要建隧道"
        let mut config = base();
        config.extra = Some(json!({ EXTRA_SSH: ssh }));
        assert_eq!(tunnel_key(&config), None);
    }

    /// 一对已连上的回环 TCP（模拟「本地这头」与「隧道对面」）。
    fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind((LOCAL_HOST, 0)).expect("回环监听应可用");
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    /// 转发泵的核心承诺：**两个方向的字节都不丢**，并且一端的半关闭要传到另一端。
    ///
    /// 这是本模块里唯一自己写的协议逻辑（其余交给 libssh2），也是最容易悄悄错的地方 ——
    /// 「能连上但第一条语句发不出去」正是这类错误的表现。所以这里用回环 TCP
    /// 把 `pump` 单独跑起来验证，不需要跳板机。
    #[test]
    fn 双向搬运不丢字节且传播半关闭() {
        let (mut local_near, mut local_far) = tcp_pair();
        let (mut remote_near, mut remote_far) = tcp_pair();
        // 与真实路径一致：本地这头与隧道那头都是非阻塞的
        local_near.set_nonblocking(true).unwrap();
        remote_near.set_nonblocking(true).unwrap();
        for stream in [&local_far, &remote_far] {
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
        }

        let worker = std::thread::spawn(move || pump(&mut local_near, &mut remote_near));

        // 两个方向同时灌数据：一次 200KB / 150KB，都远超单次 read 的缓冲
        let up = vec![b'u'; 200 * 1024];
        let down = vec![b'd'; 150 * 1024];
        let up_writer = {
            let payload = up.clone();
            std::thread::spawn(move || {
                local_far.write_all(&payload).unwrap();
                local_far
            })
        };
        let down_writer = {
            let payload = down.clone();
            std::thread::spawn(move || {
                remote_far.write_all(&payload).unwrap();
                remote_far
            })
        };

        let mut got_down = vec![0u8; down.len()];
        let mut got_up = vec![0u8; up.len()];
        // 本地这头会收到「隧道对面发来的」down
        let mut local_far = up_writer.join().unwrap();
        local_far.read_exact(&mut got_down).unwrap();
        // 隧道对面会收到「本地发来的」up
        let mut remote_far = down_writer.join().unwrap();
        remote_far.read_exact(&mut got_up).unwrap();
        assert_eq!(got_up, up, "本地 → 隧道对面的字节必须完整");
        assert_eq!(got_down, down, "隧道对面 → 本地的字节必须完整");

        // 本地这头关闭写方向：对面必须读到 EOF（否则数据库侧会一直等一个不会来的包）
        local_far.shutdown(Shutdown::Write).unwrap();
        let mut sink = [0u8; 1];
        assert_eq!(
            remote_far.read(&mut sink).unwrap(),
            0,
            "本地的半关闭必须传到隧道对面"
        );

        // 对面也关闭：泵应自然收尾，不留悬挂线程
        remote_far.shutdown(Shutdown::Both).unwrap();
        let mut sink = [0u8; 1];
        let _ = local_far.read(&mut sink);
        worker.join().unwrap().expect("转发泵不应报错退出");
    }

    /// 描述文本给日志/诊断用：要能看出是哪台跳板机、转发到哪，且**不含凭据**。
    #[test]
    fn 隧道描述不含凭据() {
        let config = with_ssh(json!({
            "enabled": true, "host": "jump", "port": 22, "user": "deploy",
            "authType": "password", "password": "s3cr3t"
        }));
        let text = describe(&config).unwrap();
        assert!(text.contains("deploy@jump:22"), "{text}");
        assert!(text.contains("10.0.0.9:3306"), "{text}");
        assert!(!text.contains("s3cr3t"), "{text}");
        assert_eq!(describe(&base()), None);
    }
}
