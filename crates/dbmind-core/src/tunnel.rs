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
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use ssh2::Session;

use crate::types::ConnectionConfig;
use crate::{DbMindError, ErrorCode, Result};

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

// ------------------------------------------------------------------ 隧道管理

/// 一条活着的隧道：本地端口 + 停止信号。
struct Tunnel {
    local_port: u16,
    stop: Arc<AtomicBool>,
    /// 日志用：跳板机与目标地址，出问题时能一眼看出是哪条隧道。
    label: String,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // 只置个标记：监听线程下一轮轮询就会退出并关掉监听端口
        self.stop.store(true, Ordering::Relaxed);
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
    let tunnel = open_tunnel(&spec, &target_host, target_port)?;
    let mut map = lock_tunnels();
    // 并发下可能已经有人建好了：让先来的留下，自己这条会被 drop（Drop 会关掉监听）
    let entry = map.entry(key).or_insert(Entry {
        tunnel,
        last_used: now,
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
    let mut map = lock_tunnels();
    // 0 = 不按空闲回收（内网常驻跳板机可以这么配，隧道一直热着）
    let ttl_secs = IDLE_TTL_SECS.load(Ordering::Relaxed);
    if ttl_secs > 0 {
        let ttl = Duration::from_secs(ttl_secs.min(u64::from(u32::MAX)));
        map.retain(|_, entry| now.duration_since(entry.last_used) < ttl);
    }
    let entry = map.get_mut(key)?;
    entry.last_used = now;
    Some(entry.tunnel.local_port)
}

// ------------------------------------------------------------------ 建隧道

fn open_tunnel(spec: &SshConfig, target_host: &str, target_port: u16) -> Result<Arc<Tunnel>> {
    let listener = TcpListener::bind((LOCAL_HOST, 0)).map_err(|err| {
        conn_err("本地端口分配失败，无法建立 SSH 隧道").with_detail(format!("{err}"))
    })?;
    let local_port = listener
        .local_addr()
        .map_err(|err| conn_err("读取本地转发端口失败").with_detail(format!("{err}")))?
        .port();

    // 先探一次：认证不通、跳板机到数据库不可达，都要**在这里**说清楚 ——
    // 否则驱动只会连一个没人转发的本地端口，报出来的是「连接被拒绝」，
    // 排查方向会被带到"数据库挂了"上。
    probe(spec, target_host, target_port)?;

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
        std::thread::Builder::new()
            .name(format!("ssh-tunnel-{local_port}"))
            .spawn(move || accept_loop(listener, stop, spec, target_host, target_port, label))
            .map_err(|err| conn_err("SSH 隧道线程创建失败").with_detail(format!("{err}")))?;
    }
    tracing::info!(target: "dbmind::tunnel", label = %label, port = local_port, "SSH 隧道已建立");
    Ok(Arc::new(Tunnel {
        local_port,
        stop,
        label,
    }))
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

    match &spec.auth {
        SshAuth::Password(password) => {
            session
                .userauth_password(&spec.user, password)
                .map_err(|err| auth_err(spec, &address, &err.to_string()))?;
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
    // 认证之后转非阻塞：转发线程要能交替推进两个方向（见 [`pump`]）
    session.set_timeout(0);
    Ok(session)
}

fn auth_err(spec: &SshConfig, address: &str, detail: &str) -> DbMindError {
    let how = match spec.auth {
        SshAuth::Password(_) => "口令",
        SshAuth::Key { .. } => "私钥",
    };
    conn_err(format!("SSH {how}认证失败：{}@{}", spec.user, address)).with_detail(detail.to_string())
}

/// 探一次：握手 + 认证 + **真的开一条转发通道**。
///
/// 只握手不试通道是不够的：认证成功但「跳板机到数据库」不通时，问题会拖到驱动那边才暴露，
/// 用户看到的是一句「连接被拒绝」，看不出根因在跳板机的路由/白名单上。
fn probe(spec: &SshConfig, target_host: &str, target_port: u16) -> Result<()> {
    let session = open_session(spec)?;
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
    Ok(())
}

// ------------------------------------------------------------------ 转发

fn accept_loop(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    spec: SshConfig,
    target_host: String,
    target_port: u16,
    label: String,
) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _peer)) => {
                // 每条转发连接一条线程、一条独立 SSH 会话。
                //
                // 为什么不共用一条会话：libssh2 的**阻塞读会一直持有会话锁**，
                // 两个方向各占一条线程时，若读线程正在等数据，写线程就拿不到锁，
                // 「本地发往数据库」的字节会一直堆着发不出去（表现为连上后第一条语句
                // 就没有响应）。所以这里每条连接自己一条会话，配合 [`pump`] 的单线程
                // 双向轮询 —— 会话之间互不干扰。
                let spec = spec.clone();
                let target_host = target_host.clone();
                let forward_label = label.clone();
                let spawned = std::thread::Builder::new()
                    .name("ssh-forward".to_string())
                    .spawn(move || {
                        if let Err(err) = forward(stream, &spec, &target_host, target_port) {
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
    tracing::debug!(target: "dbmind::tunnel", label = %label, "SSH 隧道已关闭");
}

fn forward(
    mut local: TcpStream,
    spec: &SshConfig,
    target_host: &str,
    target_port: u16,
) -> std::io::Result<()> {
    let session = open_session(spec).map_err(|err| {
        // 建会话失败不该让整个隧道倒掉（跳板机抖一下、并发上限到了都可能发生）：
        // 这条本地连接关掉，驱动那边会看到连接断开并自行重试。
        std::io::Error::other(format!("SSH 会话建立失败：{}", err.message))
    })?;
    let mut channel = session
        .channel_direct_tcpip(target_host, target_port, None)
        .map_err(|err| std::io::Error::other(format!("SSH 转发通道打开失败：{err}")))?;
    session.set_blocking(false);
    let _ = local.set_nonblocking(true);
    let _ = local.set_nodelay(true);
    let result = pump(&mut local, &mut channel);
    let _ = local.shutdown(Shutdown::Both);
    result
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
