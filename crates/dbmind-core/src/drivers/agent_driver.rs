//! agent 驱动：**所有 `runtimeMode=agent` 的类型都由它承载**。
//!
//! 两种连接方式，取决于类型是否带 JDBC 元数据：
//!
//! | 类型 | 连接参数 | 谁提供「怎么连」 |
//! |---|---|---|
//! | JDBC 系（13 个） | `driverClass` + `url`（由 URL 模板渲染） + 驱动 jar 列表 | YAML 的 `jdbc` 块 |
//! | 原生协议（MongoDB） | 直接给 host/port/database/账号 + `protocol` | 宿主自己按协议拼连接串 |
//!
//! 为什么原生协议不也放 YAML：JDBC 的「怎么连」本质是一个 URL 模板，可以纯数据化；
//! 而 Mongo 的连接串与认证语义由驱动库决定，把它的细节抄进 YAML 只会变成
//! 第二份真相。因此这里只传**规范化的连接字段**，拼接交给宿主 —— 宿主本来就懂自己的协议。
//!
//! 重试策略（与上游那次踩坑的直接对应）：
//! - **连接与元数据**是幂等的 ⇒ 会话失效可重建重试一次；
//! - **写语句**不是幂等的 ⇒ 进程/响应异常一律不重试，交由上层决定；
//! - **只读语句**（`sql::is_read_only`，已排除 `SELECT … INTO` / `nextval` 这类带副作用的）
//!   在连接级失败时重连重试一次。理由：这种失败意味着语句压根没发到服务端，
//!   重跑没有副作用 —— 而它正是「窗口不关、VPN 断了再连，第一条语句必报一次」的成因。

use super::session_pool::{Pin, SessionBudget, SessionPool};
use super::{Driver, LoadDataCall, MetaCall, PreparedBatchCall, QueryCall, TxAction};
use crate::agent::{self, AgentHost, AgentQueryResult};
use crate::error::{DbMindError, ErrorCode, Result};
use crate::statement::classify;
use crate::types::{
    CellValue, ColumnDetail, ConnectReport, ConnectionConfig, QueryResult, TableInfo,
};
use crate::{CancelToken, ConnectionKind, RuntimeMode};
use serde_json::{json, Map};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// 结构浏览这类幂等调用等一条空闲会话的预算。
///
/// 为什么要有上限而不是无限等：单连接类型的元数据要搭**同一条**（见
/// `metadata_session_key`），此时一条长查询就会把结构浏览挡住 ——
/// 与其让界面无声地转圈，不如到点后明确说「这条连接的并发上限是 1」。
const METADATA_POOL_WAIT: Duration = Duration::from_secs(30);

/// 连通性自检等一条空闲会话的预算（自检本身也该有兜底）。
const SELF_CHECK_POOL_WAIT: Duration = Duration::from_secs(30);

/// 会话闲置超过这么久，借出前先探一次活。
///
/// 为什么要这一步（真机现象，整库同步跑到 90% 时集中爆发）：日志里几张表接连报
/// `Communications link failure`，驱动自带的诊断是「**收发都是 188 秒前**」——
/// 那条连接在池里躺了 3 分钟没有流量，被云数据库的公网代理 / NAT 回收了会话表
/// （MySQL 的 `wait_timeout` 通常远超 3 分钟，掐断发生在中间设备）。而内核这边只缓存
/// sessionId、宿主只缓存 `Connection`，两边都不知道它已经死了 ⇒ 下一次借出时
/// （最后阶段只剩大表，轮到谁谁才用它）第一条语句就撞上死连接。写路径非幂等，
/// 于是整表回滚重跑 —— 用户看到的就是「前面都好好的，到 90% 开始报错」。
///
/// 探活只是最轻的一句 SQL（一次往返），代价远小于重跑一张大表；
/// 探不通就作废这条会话，下一次 `ensure_session` 必然重连。
const STALE_PROBE_AFTER: Duration = Duration::from_secs(60);

/// 探活语句自身的超时：它只是为了排除「死连接」，不该在链路上再等半分钟。
const STALE_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// 后台保活的间隔。
///
/// 20 秒是「远小于任何链路/代理的空闲阈值」与「别把 ping 变成流量负担」的折中：
/// 实测被掐的阈值是 180 秒左右，20 秒有 9 倍余量；一条 ping 在本地库约 1ms、
/// 云库约 20ms，一个连接每分钟 3 次可以忽略。
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);

/// 只保活「最近 10 分钟内用过」的会话：更久没用的交给空闲回收，不该为它们白花流量。
const KEEPALIVE_WINDOW: Duration = Duration::from_secs(600);

/// 一轮最多保活多少条。用户同时开几十个连接时不至于让一轮 ping 拖成长尾。
const KEEPALIVE_MAX_SESSIONS: usize = 64;

/// 逃生阀：`DBMIND_NO_KEEPALIVE=1` 关掉后台保活。
///
/// 排查「链路明明没问题、吞吐却异常」时用它把保活这层摘干净（它本身只该带来
/// 每 20 秒一次往返）；也供不欢迎任何额外语句的受控环境使用。
fn keepalive_disabled() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var("DBMIND_NO_KEEPALIVE").is_ok_and(|v| v.trim() == "1"))
}

/// 逃生阀：`DBMIND_NO_LINK_PROBE=1` 关掉「借出前探活」（后台保活不受影响）。
fn link_probe_disabled() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var("DBMIND_NO_LINK_PROBE").is_ok_and(|v| v.trim() == "1"))
}

/// 错误是不是「宿主那边没有这条会话」（宿主文案：`会话不存在` / `会话不存在：<id>`，
/// 有时带后缀`（该会话上次失败：…）`）。
///
/// 语义上它比 `Communications link failure` **更干净**：会话都不在了 ⇒ 语句一定没发出去，
/// 所以写语句也能安全重试（见 `with_lane` 里那条 arm）。两个错误码都是
/// `DBMIND-CONN-0003`，只能靠文案区分。
fn is_session_gone(message: &str) -> bool {
    message.contains("会话不存在")
}

/// 一次调用的泳道参数。
///
/// 收成一个结构是为了**别让参数一路加到签名读不懂**（也过不了 clippy 的 7 参数线）：
/// 这三个都是「与池 / 重试有关」的零碎开关，放在一起最自然。
struct LaneCall<'a> {
    read_only: bool,
    /// 幂等调用（结构浏览、自检）才重试；查询不重试（见模块文档）
    retry_on_stale: bool,
    /// **只读语句**在连接级失败（`DBMIND-CONN-0003`）时可以重连后重试一次。
    ///
    /// 为什么只读可以破例：写语句不重试的唯一理由是「响应丢了，它可能已经在库里执行过」，
    /// 这个担心对 `select` 不成立（`sql::is_read_only` 还会把
    /// `SELECT … INTO`、`nextval()` 之类带副作用的语句排除掉）。
    ///
    /// 真机场景：窗口不关、VPN 断了再连上，池里那条连接已经死了，内核和宿主却都还“记着”它
    /// —— 第一条语句必然撞死，用户看到一次 `Communications link failure`，第二次才成功。
    retry_after_disconnect: bool,
    /// 只有查询带令牌：**排队等会话时也要能被取消**（见 `SessionPool::acquire_cancellable`）
    cancel: Option<&'a CancelToken>,
}

pub struct AgentDriver {
    kind: ConnectionKind,
    host: Arc<AgentHost>,
    /// 会话池：把「一个连接」映射到「一到多条物理会话」（见 `session_pool` 模块文档）。
    ///
    /// 用 `Arc` 是为了让后台保活线程能弱引用它（池随驱动释放，线程自然退出）。
    pool: Arc<SessionPool>,
    /// 全局会话配额（跨连接、跨类型、跨宿主；由引擎在设置变更时更新）
    budget: Arc<SessionBudget>,
    /// 挂着**未提交事务**的泳道（事务模式：begin 加入，commit/rollback 移除）。
    ///
    /// 用途：连接断开重连后，新连接默认 autocommit=true —— 必须马上重新对齐，
    /// 否则后续语句逐条自动提交，用户以为还在事务里（见 `with_lane` 的对齐钩子）。
    tx_lanes: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl AgentDriver {
    /// 断开本类型当前所有**空闲**会话（权限/密码在数据库侧变更后调用，
    /// 旧会话还带着旧的全局权限快照；见 `session_pool::SessionPool::close_all_idle`）。
    pub fn disconnect_all(&self) -> usize {
        if let Ok(mut lanes) = self.tx_lanes.lock() {
            lanes.clear();
        }
        self.pool.clear_pins();
        self.pool.close_all_idle()
    }

    /// 只断**指定连接**的空闲会话：泳道 key 的第二段是连接名（`kind|name|host|...`），
    /// 按它过滤就不会误伤同类型其它连接（它们共用一个驱动池）。
    pub fn disconnect_connection(&self, name: &str) -> usize {
        let dropped: Vec<String> = {
            let mut lanes = self.tx_lanes.lock().unwrap_or_else(|e| e.into_inner());
            let dropped = lanes
                .iter()
                .filter(|lane| lane.split('|').nth(1) == Some(name))
                .cloned()
                .collect();
            for lane in &dropped {
                lanes.remove(lane);
            }
            dropped
        };
        for lane in dropped {
            self.pool.unpin_lane(&lane);
        }
        // 批量写事务的保护也按连接名兜底解除（会话池只认「不回收」，不知道谁还在用）
        self.pool
            .unpin_matching(|lane| lane.split('|').nth(1) == Some(name));
        self.pool
            .close_idle_where(|lane| lane.split('|').nth(1) == Some(name))
    }

    /// 标记 / 解除「事务中的批量会话」保护：见 [`Driver::hold_tx_lane`]。
    pub fn hold_tx_lane(&self, affinity: &str, hold: bool) -> usize {
        self.pool.hold_tx_lane(affinity, hold)
    }

    pub fn new(kind: ConnectionKind, host: Arc<AgentHost>, budget: Arc<SessionBudget>) -> Self {
        // 配额回收空闲会话时要**真的把连接关掉**，而「怎么关」只有驱动知道（它拿着宿主）。
        // 所以做成回调交给预算：预算负责挑「最久未用的空闲会话」，驱动负责关。
        let closer: Arc<dyn Fn(&str) + Send + Sync> = {
            let host = host.clone();
            Arc::new(move |session_id: &str| {
                if let Err(err) = host.disconnect(session_id) {
                    // 关不掉不致命：内核已经不再复用它（缓存已清），
                    // 宿主进程退出时那几条连接也会一起清掉
                    tracing::warn!(
                        target: "dbmind::agent",
                        session = %session_id,
                        error = %err,
                        "回收空闲会话时关闭失败"
                    );
                }
            })
        };
        let pool = Arc::new(SessionPool::new(budget.clone(), closer));
        // 保活线程只弱引用池：驱动释放后它自己退出，不会吊住内存（也不会泄漏一个线程去 ping 已关闭的连接）
        spawn_keepalive(kind, host.clone(), Arc::downgrade(&pool));
        Self {
            kind,
            host,
            pool,
            budget,
            tx_lanes: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// 生效的并发上限，三层取最小：**类型声明**（YAML 的 `maxConnections`）→
    /// **连接级覆盖**（`extra.maxConnections`）→ **全局配额**（设置里的每宿主上限）。
    ///
    /// 亲和泳道恒为 **1**：那条会话就是某个页面的会话，同一时刻只该有一个请求在它上面跑
    /// （否则「语句按顺序落在同一个会话上」这句话就不成立了）。
    pub(crate) fn lane_limit(&self, config: &ConnectionConfig, affinity: Option<&str>) -> usize {
        if affinity.is_some() {
            return 1;
        }
        // 单连接类型恒为 1：连接级覆盖只能**收窄**，不能突破类型声明 ——
        // 对它们开第二条就是另一个世界（嵌入式引擎会看到空库，或干脆加锁失败）。
        if self.kind.single_connection_pool() {
            return 1;
        }
        // 并发上限 = 连接级覆盖（`extra.maxConnections`，界面不暴露）或**类型声明**
        // （`plugins/connection-types/*.yaml` 的 `maxConnections`），再由全局总量夹一次。
        // 各类型的默认值口径见那些 YAML：关系型统一 8，ClickHouse/Doris 8，单连接池恒 1。
        let declared = config
            .max_connections_override()
            .unwrap_or_else(|| self.kind.max_connections());
        self.budget.clamp(declared)
    }

    fn agent_key(&self) -> Result<&'static str> {
        self.kind.agent_key().ok_or_else(|| {
            DbMindError::new(
                ErrorCode::DriverNotImplemented,
                format!("{} 声明为 agent 运行时但没有 agentKey", self.kind.label()),
            )
        })
    }

    /// 会话键：连接配置决定会话，只读标记也参与（只读会话在宿主侧也是只读的）。
    ///
    /// 开了 SSH 隧道时**隧道配置也进键**：宿主会话里存的是「127.0.0.1:<本地端口>」这条
    /// 已经建好的连接，改了跳板机（换了地址、用户、私钥）却不换会话键的话，
    /// 界面会继续用旧隧道那条会话 —— 用户看到的是"改了 SSH 配置没生效"。
    /// 指纹是纯计算（不连跳板机、不含口令），没开隧道时它是空的，键与以前逐字节一致。
    pub(crate) fn session_key(&self, config: &ConnectionConfig, read_only: bool) -> String {
        let tunnel = crate::tunnel::tunnel_key(config)
            .map(|key| format!("|{key}"))
            .unwrap_or_default();
        format!(
            "{}|{}|{}|{}|{}|{}|{}{}",
            self.kind.key(),
            config.name,
            config.host.as_deref().unwrap_or("-"),
            config
                .resolved_port()
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".into()),
            config.database.as_deref().unwrap_or("-"),
            config.resolved_file().unwrap_or("-"),
            read_only,
            tunnel
        )
    }

    /// 泳道键 = 连接（含只读性）+ **会话亲和**。
    ///
    /// 亲和只对**查询**生效：结构浏览仍走不带亲和的泳道 —— 对象树是全局视图，
    /// 不该变成「某个标签页的私有视图」（否则同一个库在不同页面看到的对象还不一样）。
    ///
    /// 单连接类型**忽略亲和**：它们只有一条物理连接可给（再开一条就是另一个世界，
    /// 嵌入式引擎会因此看到空库或加锁失败），所以那些类型上所有调用本来就在同一条会话上。
    pub(crate) fn lane_key(
        &self,
        config: &ConnectionConfig,
        read_only: bool,
        session: Option<&str>,
    ) -> String {
        let base = self.session_key(config, read_only);
        match session {
            Some(key) if !key.is_empty() && !self.kind.single_connection_pool() => {
                // 把分隔符换掉：泳道键用 `|` 分段，键里再出现 `|` 就可能拼出
                // 与另一条泳道相同的字符串（会话串线是最难查的那类问题）
                format!("{base}|s={}", key.replace('|', "_"))
            }
            _ => base,
        }
    }

    /// 组装 connect 参数。**分协议**：JDBC 给 URL，原生协议给规范化字段。
    pub(crate) fn connect_params(
        &self,
        session_key: &str,
        config: &ConnectionConfig,
        read_only: bool,
    ) -> Result<Map<String, serde_json::Value>> {
        let agent_key = self.agent_key()?;
        // SSH 隧道：把「宿主该连的地址」换成本地转发端口（跳板机由内核打通）。
        // 宿主因此完全不需要知道 SSH 的存在，Java 与原生协议一视同仁。
        // 取不到隧道（没开）就用原配置；建不通会**在这里**带着原因失败，
        // 而不是让宿主去连一个没人转发的端口。
        let tunneled = crate::tunnel::local_port_for(config)?;
        let rewritten;
        let target = match tunneled {
            Some(port) => {
                // 日志里给一句「这条连接是经隧道走的」：出问题时第一件要知道的
                // 就是「到底走没走隧道」（描述文本里不含任何凭据）。
                if let Some(desc) = crate::tunnel::describe(config) {
                    tracing::debug!(
                        target: "dbmind::agent",
                        tunnel = %desc,
                        local_port = port,
                        "本次连接经 SSH 隧道"
                    );
                }
                rewritten = crate::tunnel::with_local_endpoint(config, port);
                &rewritten
            }
            None => config,
        };
        let mut params = Map::new();
        params.insert("sessionId".to_string(), json!(session_key));
        params.insert("agentKey".to_string(), json!(agent_key));
        params.insert("protocol".to_string(), json!(self.kind.protocol().as_str()));
        params.insert("readOnly".to_string(), json!(read_only));
        // 全局配额（每宿主进程）：交给宿主执行 —— 它是唯一知道「哪条会话此刻空闲」
        // 的地方，淘汰必须由它挑（见宿主侧逻辑）。0 = 不限制。
        params.insert("maxSessions".to_string(), json!(self.budget.cap()));

        if self.kind.is_jdbc() {
            // 驱动 jar 是 JDBC 的可插拔部分：由驱动商店管理
            let jars = agent::installed_driver_jars(agent_key);
            let driver_class = self.kind.jdbc_driver_class().ok_or_else(|| {
                DbMindError::new(
                    ErrorCode::DriverNotImplemented,
                    format!("{} 缺少 jdbc.driverClass 声明", self.kind.label()),
                )
            })?;
            let template = self.kind.jdbc_url_template().ok_or_else(|| {
                DbMindError::new(
                    ErrorCode::DriverNotImplemented,
                    format!("{} 缺少 jdbc.urlTemplate 声明", self.kind.label()),
                )
            })?;
            // 用 `target` 而不是 `config` 渲染：开了隧道时它指向 127.0.0.1:<本地端口>
            let url = agent::render_jdbc_url(template, target)?;
            // Windows 上的「Windows 验证」走原生 SSPI，需要一份**不在驱动 jar 里**的原生库
            // （`mssql-jdbc_auth-<驱动版本>.x64.dll`）。缺了它驱动只会丢一句
            // 「没有为集成身份验证配置驱动程序」—— 既没说要哪个文件、也没说放哪里，
            // 用户只能猜。这里提前拦住，把文件名和目录直接给出来。
            // （库路径只在 Java 宿主**启动时**读取，所以补了文件要重启。）
            if cfg!(windows) && agent::is_windows_auth(config) && url.starts_with("jdbc:sqlserver:") {
                let key = self.kind.agent_key().unwrap_or("sqlserver");
                if agent::ensure_native_auth_library(key).is_none() {
                    let arch = agent::native_arch();
                    return Err(DbMindError::new(
                        ErrorCode::ConnConnectFailed,
                        "Windows 集成验证需要一份原生认证库，当前驱动目录里没有",
                    )
                    .with_detail(format!(
                        "把 Microsoft JDBC Driver 发行包里 auth\\{arch}\\ 下的 \
                         mssql-jdbc_auth-<驱动版本>.{arch}.dll 放到 {} 后重启 DBmind（库路径只在 \
                         Java 宿主启动时读取）；只想先连上也可以改用「SQL Server 身份验证」。",
                        agent::driver_dir(key).display()
                    )));
                }
            }
            params.insert("driverClass".to_string(), json!(driver_class));
            params.insert("url".to_string(), json!(url));
            params.insert(
                "driverJars".to_string(),
                json!(jars.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()),
            );
            // 账号与口令：JDBC 的 URL 模板里**不含**它们（各驱动都从连接属性取），
            // 宿主侧正是按 "username"/"password" 读的 —— 早先这里漏了这两行，
            // 于是所有需要认证的 JDBC 类型都登不上去。宿主层测试没发现，
            // 是因为它只覆盖 H2 文件库（那玩意儿不需要认证）。
            //
            // Windows 集成认证：只给主体名（可以是 user@REALM），**不给口令** ——
            // 凭据由 Kerberos 票据提供；带上口令反而会把驱动拉回 SQL 认证。
            // 没选这种认证方式时一字不改，还是原来的 username + password。
            if agent::is_windows_auth(config) {
                params.insert("username".to_string(), json!(config.username));
                params.insert("password".to_string(), json!(""));
            } else {
                params.insert("username".to_string(), json!(config.username));
                params.insert("password".to_string(), json!(config.password));
            }
            // 连接级驱动参数（`extra.params`）透传给驱动，形如
            // `{"trustServerCertificate": "true"}` —— 自签证书的 SQL Server 就得靠它。
            if let Some(extra) = config
                .extra
                .as_ref()
                .and_then(|e| e.get("params"))
                .and_then(|v| v.as_object())
            {
                // `socketTimeout=0` 是「永不超时」，不是「用默认值」：连接被中间设备静默断开
                // （VPN/NAT 空闲回收、TAP 隧道丢包）后永远发现不了，只能等写批的兜底超时 ——
                // 真机表现是「同一张表卡 5 分钟才报错，然后整表重跑」。老连接里存的就是 0
                // （表单曾经允许填 0），所以在**建连这一层**统一丢掉它，让 URL 模板里的
                // 默认值（10 分钟）生效，不必让用户逐条重存连接。
                let mut cleaned = extra.clone();
                let zero = cleaned
                    .get("socketTimeout")
                    .and_then(|v| v.as_str())
                    .map(|v| v.trim().parse::<u64>().unwrap_or(1) == 0)
                    .unwrap_or(false);
                if zero {
                    cleaned.remove("socketTimeout");
                }
                if !cleaned.is_empty() {
                    params.insert("params".to_string(), serde_json::Value::Object(cleaned));
                }
            }
        } else {
            // 原生协议：宿主自带驱动库，这里只给连接字段
            // （同样用 `target`：开了隧道时它指向本地转发端口）
            params.insert("host".to_string(), json!(target.host));
            params.insert("port".to_string(), json!(target.resolved_port().unwrap_or(0)));
            params.insert("database".to_string(), json!(target.database));
            params.insert("username".to_string(), json!(target.username));
            params.insert("password".to_string(), json!(target.password));
        }
        Ok(params)
    }

    /// 建立（或复用）会话。`refresh` 为真时强制重连 —— 自检必须验证配置本身。
    ///
    /// `session_id` 由**会话池**给出（`<泳道>#<槽位>`）：宿主侧正是它决定用哪条物理连接，
    /// 所以这里不再自己从配置推键 —— 否则池化就白做了（大家还是回到同一条连接上）。
    fn ensure_session(
        &self,
        config: &ConnectionConfig,
        read_only: bool,
        session_id: &str,
        refresh: bool,
    ) -> Result<Option<String>> {
        if !refresh {
            if let Some((_, version)) = self.host.cached_session(session_id) {
                return Ok(version);
            }
        }

        let agent_key = self.agent_key()?;

        // JDBC 的驱动 jar 由驱动商店提供 ⇒ 缺了要教用户怎么装；
        // 专属宿主（MongoDB）自带驱动 ⇒ 只检查宿主本身是否就绪。
        if self.kind.is_jdbc() && agent::installed_driver_jars(agent_key).is_empty() {
            return Err(DbMindError::new(
                ErrorCode::DriverNotReady,
                format!("{} 的 JDBC 驱动未安装（agentKey={agent_key}）", self.kind.label()),
            )
            .with_detail(format!(
                "把驱动 jar 放入 {}，或执行 `dbmind driver fetch {agent_key}`（坐标 {}）",
                agent::driver_dir(agent_key).display(),
                self.kind.jdbc_artifact().unwrap_or("-")
            )));
        }

        let availability = self.host.availability();
        if !availability.ready {
            return Err(DbMindError::new(
                ErrorCode::DriverNotReady,
                format!("{} 的驱动宿主未就绪", self.kind.label()),
            )
            .with_detail(availability.reason.unwrap_or_else(|| "-".to_string())));
        }

        if !self.host.supports(agent_key)? {
            // 走到这里说明**声明本身**对不上（宿主声明里没有这个 key），不是进程暂时没起来 ——
            // 启动/重启期间的询问已由 `AgentHost::supports` 的静态声明兜住。
            // 文案也要照实说：以前这里写「该类型需要专属协议宿主」，于是 JDBC 类型偶尔
            // 撞上启动窗口时，用户被指引去翻 plugins/connection-types（那里根本没有问题）。
            return Err(DbMindError::new(
                ErrorCode::DriverNotImplemented,
                format!("宿主 {} 未声明支持 agentKey={agent_key}", self.host.spec().id),
            )
            .with_detail(
                "这是类型声明与宿主不匹配（不是驱动缺失、也不是宿主没起来）：\
                 核对 plugins/connection-types 里该类型的 agentKey，\
                 与该宿主声明的 agent_keys 是否一致",
            ));
        }

        // 文件型连接先补齐父目录（嵌入式引擎在父目录缺失时会给你一个临时空库）
        if self.kind.local_file() {
            ensure_file_parent(config)?;
        }

        let params = self.connect_params(session_id, config, read_only)?;
        // 建连超时 25 → 60 秒：走 SSH 隧道 + 云端 RDS 时，冷启动一条连接实测 1.5~6.4 秒，
        // 链路抖一下（TAP/加速器链路会丢满 MSS 的包，重传退避 ~19 秒）就容易越过 25 秒 ——
        // 报错**原样透传**（用户口径：不要我们自造的说明与追加信息）。
        // 建连耗时只进 tracing，排查「慢还是断」时去文件日志里看。
        let started = std::time::Instant::now();
        let result = self
            .host
            .call(None, "connect", params, Some(Duration::from_secs(60)))
            .map_err(|err| {
                tracing::warn!(
                    target: "dbmind::agent",
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    code = %err.code,
                    "建连失败"
                );
                let mut out = DbMindError::new(err.code, err.message.clone());
                out.detail = err.detail;
                out
            })?;
        let version = result
            .get("serverVersion")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        // 宿主为了守住配额可能**淘汰**了别的空闲会话（见 `evicted_sessions`）。
        // 把内核的会话缓存清掉即可：下次调用 `ensure_session` 会发现没有缓存并重连。
        // 被淘汰的绝不会是正在执行语句的会话 —— 宿主只挑空闲的。
        for evicted in evicted_sessions(&result) {
            tracing::debug!(
                target: "dbmind::agent",
                session = %evicted,
                "宿主按配额淘汰了空闲会话（下次用到时会重连）"
            );
            self.host.forget_session(&evicted);
        }
        self.host.cache_session(session_id, session_id, version.clone());
        Ok(version)
    }

    /// 元数据该用哪条会话：**哪个作用域** + 以什么只读性。
    ///
    /// 两种情况必须与数据会话**同一条物理连接**（否则看到的就是另一份世界）：
    ///
    /// - `metadataConnectionScoped: true`（如 SQL Server 的 `#temp`）：对象是**会话级**的；
    /// - `singleConnectionPool: true`（嵌入式/文件引擎）：那条声明就是「只允许一条物理连接」，
    ///   再开一条可能落到**另一个引擎实例**上。native 驱动早就按这条做了（元数据搭同一条），
    ///   这里把 agent 侧补齐。
    ///
    /// 代价如实说：这样一来，单连接类型的结构浏览跑在**数据那条会话**上（与数据同权限），
    /// 换来的是「这条连接真的只有一条物理连接」。其余类型仍用独立的只读会话。
    pub(crate) fn metadata_session_key(&self, config: &ConnectionConfig, read_only: bool) -> (String, bool) {
        if self.kind.metadata_connection_scoped() || self.kind.single_connection_pool() {
            (self.session_key(config, read_only), read_only)
        } else {
            (self.session_key(config, true), true)
        }
    }

    /// 在指定泳道上执行一次调用。
    ///
    /// `retry_on_stale` **只对幂等调用为真**（结构浏览、自检）：查询不是幂等的，
    /// 响应丢失时它可能已经在数据库侧执行过了，重试等于再跑一遍（见模块文档）。
    fn with_lane<T>(
        &self,
        config: &ConnectionConfig,
        lane: &str,
        limit: usize,
        wait: Duration,
        call: LaneCall<'_>,
        op: impl Fn(&str) -> Result<T>,
    ) -> Result<T> {
        // 槽位守卫活到闭包结束：这条物理会话在调用期间**只属于我们**
        let slot = self.pool.acquire_cancellable(lane, limit, wait, call.cancel)?;
        // 交互式泳道**常驻**：编辑器页签/用户手点查询都用这套键（`s=<页签键>`），
        // 批量与内部功能统一走 `s=internal:...`（见 `is_interactive_lane`）。
        // 常驻 = 不参与配额回收、不参与空闲清扫 —— 用户切回来再查不必重新握手
        //（走 SSH 隧道建一条连接实测 1.5~6.4 秒，正是「点一下就转圈」的来源）。
        // 显式「关闭连接」仍断得掉（`close_idle_where` 不看 pin）；事务泳道另有自己的 pin。
        if is_interactive_lane(lane) {
            self.pool.pin_lane(lane, Pin::Interactive);
        }
        let session_id = slot.session_id().to_string();
        // 事务泳道不能「悄悄重连」：旧连接上的未提交修改已经随断开回滚了，
        // 在新连接上重新 autocommit=false 只会让用户以为事务还在。
        let tx_open = self
            .tx_lanes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(lane);
        if tx_open && self.host.cached_session(&session_id).is_none() {
            if let Ok(mut lanes) = self.tx_lanes.lock() {
                lanes.remove(lane);
            }
            self.pool.unpin_lane(lane);
            return Err(DbMindError::new(
                ErrorCode::QueryFailed,
                "事务连接已断开，未提交的修改已回滚；请重新开启事务",
            )
            .with_detail("事务所在物理连接已不在（被回收、连接断开或宿主重启）。"));
        }
        // 躺太久的空闲会话先探一次活（原因见 `STALE_PROBE_AFTER`）。
        //
        // 必须放在 `ensure_session` **之前**：它只查内核这边的会话缓存，缓存命中就等于
        // 「宿主那条连接还活着」，而真实情况可能早就被链路掐掉了。探不通就丢掉缓存，
        // 下面那次 `ensure_session` 于是走真正的重连。
        //
        // 事务泳道不做：那条连接上有未提交的修改，探活失败只能如实报错（上面已经拦了），
        // 「悄悄换一条重连」会让用户以为事务还在。
        if !tx_open
            && !link_probe_disabled()
            && slot.idle_for() >= STALE_PROBE_AFTER
            && self.idle_session_is_dead(&session_id)
        {
            // 探不通**才**丢缓存（下次 `ensure_session` 会真重连）。
            //
            // 这里以前写反了（`!probe_idle_session`）：探活说「连接已断」时反而**保留**死会话的
            // id，探活通过时倒把好会话丢掉。后果就是整库同步跑到后半程、长时间没用的表被复用
            // 时，第一条语句必撞「会话不存在（该会话上次失败：Communications link failure）」。
            tracing::debug!(
                target: "dbmind::agent",
                session = %session_id,
                "空闲会话本已断开，丢掉缓存（下次用到时重连）"
            );
            self.host.forget_session(&session_id);
        }
        self.ensure_session(config, call.read_only, &session_id, false)?;
        // 事务模式对齐：正常复用同一会话时它本来就是 autocommit=false。
        // 若上面的缓存检查没发现失联、但宿主侧连接确实换了，这里仍按原逻辑
        // 重新关一次 autocommit；真正失联的情况已在上面直接报错，不再静默继续。
        if self.tx_lanes.lock().expect("tx_lanes").contains(lane) {
            let mut params = Map::new();
            params.insert("sessionId".to_string(), json!(session_id));
            params.insert("autoCommit".to_string(), json!(false));
            if let Err(err) = self.host.call(None, "setautocommit", params, Some(Duration::from_secs(10))) {
                tracing::warn!(target: "dbmind::agent", lane = %lane, error = %err, "重连后恢复事务模式失败");
            }
        }
        match op(&session_id) {
            // 「会话不存在」= 宿主那边连这条会话都没有 ⇒ 语句**压根没发出去**。
            // 这与「连接中途断了」有本质区别（后者可能已经在库里执行过，写语句不能重试），
            // 所以这一类**写语句也可以安全重试**，而且必须重试：内核这边还缓存着那条死会话的
            // id，不重试同一个泳道会一直撞这句话。
            //
            // 真机现象（用户截图）：点「重新同步失败对象」，第一张表秒报
            // 「会话不存在（该会话上次失败：Communications link failure）」，
            // 而同一连接上后面几张表跑得好好的 —— 死的只是这一条泳道的会话。
            Err(err) if !tx_open && is_session_gone(&err.message) => {
                tracing::warn!(
                    target: "dbmind::agent",
                    session = %session_id,
                    error = %err.message,
                    "会话在宿主侧已不存在（语句没发出去），重连后重试一次"
                );
                self.host.forget_session(&session_id);
                self.ensure_session(config, call.read_only, &session_id, true)?;
                op(&session_id)
            }
            Err(err) if call.retry_on_stale && err.code == ErrorCode::ConnConnectFailed => {
                tracing::warn!(target: "dbmind::agent", "会话已失效，重连后重试一次幂等调用");
                self.host.forget_session(&session_id);
                self.ensure_session(config, call.read_only, &session_id, true)?;
                op(&session_id)
            }
            // **只读**语句撞上死连接 ⇒ 重连后重试一次（用户不该为此看到一次失败）。
            //
            // 这条必须排在下一条前面：两者错误码相同，区别只在「重跑有没有副作用」。
            // 事务泳道不参与（`!tx_open`）：事务里悄悄换一条连接，等于把未提交的修改丢掉。
            Err(err)
                if call.retry_after_disconnect
                    && !tx_open
                    && err.code == ErrorCode::ConnConnectFailed =>
            {
                tracing::warn!(
                    target: "dbmind::agent",
                    session = %session_id,
                    "只读语句撞到已失效的连接，重连后重试一次"
                );
                self.host.forget_session(&session_id);
                self.ensure_session(config, call.read_only, &session_id, true)?;
                op(&session_id)
            }
            // 连接级失败（宿主说这条会话/连接没了）⇒ **清掉内核这边的会话缓存，但不重试本次调用**。
            //
            // 为什么必须清：查询不是幂等的（上面那条 arm 只对幂等调用生效），所以「重试」不能做；
            // 可**不清缓存**的话，下一次调用会拿同一个 sessionId 再问一次宿主，而宿主那边这条连接
            // 已经没了 —— 于是用户这条连接会**一直坏下去**，直到他自己重新连一次。
            // 真机 Redis 验证抓到的就是这个（宿主不自愈 ⇒ 会话永久失效）；宿主侧也各自做了自愈，
            // 这里是**兜底的那一层**：宿主没自愈时，用户最多重跑一次那条语句，而不是重连整条连接。
            Err(err) if err.code == ErrorCode::ConnConnectFailed => {
                tracing::warn!(
                    target: "dbmind::agent",
                    session = %session_id,
                    "会话连接已失效，已清掉内核缓存（下一次调用会重连；本次不重试）"
                );
                self.host.forget_session(&session_id);
                // **重建前先作废 SSH 隧道**：换网/断网后旧 TCP 是半开的，本地监听还活着、
                // 池里的会话 `authenticated()` 也还是 true —— 不拆隧道的话，下面这次重建
                // 又落进同一条死链路，用户只能去重启 VPN 客户端（真机 2026-10-27）。
                invalidate_tunnel_throttled(config);
                // 顺手把连接重建一次。**不重试语句**（写语句不能重试），但这样能拿到真实错误：
                // 宿主此刻可能只会说「会话不存在」，而它上一次失败的原因（Communications
                // link failure / RDS 主动断开…）以及这次重建失败的原因，才是用户要看的。
                // 重建失败就把它原样上抛 —— 比「会话不存在」这种二手结论有用得多。
                if let Err(rebuild) = self.ensure_session(config, call.read_only, &session_id, true) {
                    tracing::warn!(
                        target: "dbmind::agent",
                        session = %session_id,
                        error = %rebuild.message,
                        "会话失效后重建连接失败，按重建的错误上报"
                    );
                    return Err(rebuild);
                }
                Err(err)
            }
            other => other,
        }
    }

    /// 这条空闲会话是不是**已经确定断了**（见 `probe_session`）。
    ///
    /// 只有 `Dead` 才算：`Unknown`（语句级报错，例如 PG 事务 aborted）说明连接本身没问题，
    /// 丢掉缓存等于把用户还在用的连接悄悄换掉。
    fn idle_session_is_dead(&self, session_id: &str) -> bool {
        // true = 超时也算死：用户马上要用这条连接，宁可多花一次重连，
        // 也不能把一条半开的死连接交出去（那正是「网络恢复了还是连不上」的成因）。
        probe_session(self.kind, &self.host, session_id, true) == ProbeOutcome::Dead
    }



    /// 幂等调用（结构浏览等）：会话失效时重建并重试一次。
    ///
    /// 用哪条会话由 `metadata_session_key` 决定（可能与数据会话同一条，也可能是独立只读会话）。
    fn with_session<T>(
        &self,
        config: &ConnectionConfig,
        read_only: bool,
        op: impl Fn(&str) -> Result<T>,
    ) -> Result<T> {
        let (lane, lane_read_only) = self.metadata_session_key(config, read_only);
        // 结构浏览不带亲和（树是全局视图），上限按连接/类型/配额算
        let limit = self.lane_limit(config, None);
        self.with_lane(
            config,
            &lane,
            limit,
            METADATA_POOL_WAIT,
            LaneCall {
                read_only: lane_read_only,
                retry_on_stale: true,
                // 幂等调用已经由 `retry_on_stale` 覆盖，这里不再叠加
                retry_after_disconnect: false,
                cancel: None,
            },
            op,
        )
    }
}

/// 最轻的探活/保活语句。按方言给：Oracle 必须带 `from dual`，DB2 / Derby 用
/// `sysibm.sysdummy1`，其余（MySQL / PG / SQL Server / ClickHouse / Doris / H2 /
/// SQLite / MongoDB 走宿主原生）都认裸 `select 1`。
///
/// 探活与保活共用同一句：它必须**无副作用**，且能在事务里安全执行
/// （`select 1` 不改变事务状态，也不提交）。
fn probe_sql(kind: ConnectionKind) -> &'static str {
    match kind.key() {
        "oracle" => "select 1 from dual",
        "db2" | "derby" => "select 1 from sysibm.sysdummy1",
        _ => "select 1",
    }
}

/// 一次探活的结果。
///
/// 为什么不用 `bool`：**「探活失败」不等于「连接死了」**。会话里可能正处在一个
/// 用户自己 `BEGIN` 起来、又被别的原因搞成 aborted 的事务里（PG 上任何语句都报
/// 25P02），或者临时表/权限刚被改掉 —— 一句 `select 1` 失败就把会话作废，
/// 会把用户**还在用**的连接悄悄换掉。所以只有「连接级失败」才判死。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeOutcome {
    /// 还能用
    Alive,
    /// 连接确实没了（`DBMIND-CONN-0003`）：该作废
    Dead,
    /// 说不清（语句级错误）：**保持原样**，别动它
    Unknown,
}

/// 链路级失败后作废这条连接的 SSH 隧道（见 `crate::tunnel::invalidate`）。
///
/// 限频 60 秒：一批表连着失败时不能每次都拆隧道重建 —— 一次重建 1.5~6.4 秒，
/// 还要额外吃跳板机的会话配额，撞上 `HANDSHAKE_MAX` 反而更糟。
fn invalidate_tunnel_throttled(config: &ConnectionConfig) {
    type LastMap = std::sync::Mutex<std::collections::HashMap<String, Instant>>;
    static LAST: std::sync::OnceLock<LastMap> = std::sync::OnceLock::new();

    // 没配 SSH 隧道就什么都不用做（`tunnel_key` 只在 enabled 时才返回 Some）
    let Some(key) = crate::tunnel::tunnel_key(config) else {
        return;
    };
    let map = LAST.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    {
        let mut guard = map.lock().unwrap_or_else(|err| err.into_inner());
        if guard
            .get(&key)
            .is_some_and(|last| last.elapsed() < Duration::from_secs(60))
        {
            return;
        }
        guard.insert(key, Instant::now());
    }
    // 出锁再拆：拆隧道要关 SSH 会话（发报文），不该占着这张表
    crate::tunnel::invalidate(config);
}

/// 用一句最轻的 SQL 验证「这条会话还能不能用」。
///
/// **绝不把错误上抛**：探活是预防性检查，不是业务语句。把「可能坏了」当「一定坏」
/// 判给用户（一次同步失败、一条查询报错）比多花一次往返贵得多。
///
/// `timeout_is_dead`：超时算不算「连接已死」。**只有借出前探活传 true**。
/// 半开链路（换 WiFi / 断 VPN 隧道）唯一的表现就是「`select 1` 迟迟不返回」，不判死
/// 的话死连接会被无限复用 —— 用户必须重启 VPN 客户端才能恢复（真机 2026-10-27）。
/// 后台保活传 false：它每 20 秒一发，高负载时排队超时不该把好会话摘掉（会变成连接抖动）。
fn probe_session(
    kind: ConnectionKind,
    host: &AgentHost,
    session_id: &str,
    timeout_is_dead: bool,
) -> ProbeOutcome {
    let mut params = Map::new();
    params.insert("sessionId".to_string(), json!(session_id));
    params.insert("sql".to_string(), json!(probe_sql(kind)));
    params.insert("maxRows".to_string(), json!(1));
    match host.call(None, "query", params, Some(STALE_PROBE_TIMEOUT)) {
        Ok(_) => {
            tracing::debug!(
                target: "dbmind::agent",
                session = %session_id,
                "空闲会话探活通过（链路仍然可用）"
            );
            ProbeOutcome::Alive
        }
        // 超时：半开链路没有 RST/FIN，驱动不会报错，只会一直等 —— 这是它唯一的信号。
        // 必须排在通用分支前面，否则会被当成「语句级错误」保留会话，死连接被无限复用。
        Err(err) if timeout_is_dead && err.code == ErrorCode::QueryTimeout => {
            tracing::warn!(
                target: "dbmind::agent",
                session = %session_id,
                timeout_ms = STALE_PROBE_TIMEOUT.as_millis() as u64,
                error = %err.message,
                "空闲会话探活超时（链路多半已半开），作废后重连"
            );
            ProbeOutcome::Dead
        }
        Err(err) if err.code == ErrorCode::ConnConnectFailed => {
            tracing::debug!(
                target: "dbmind::agent",
                session = %session_id,
                error = %err,
                "空闲会话探活失败（连接已断开），作废后按需重连"
            );
            ProbeOutcome::Dead
        }
        Err(err) => {
            // 语句级错误：连接是好的，只是这条会话的状态不适合跑 `select 1`
            tracing::debug!(
                target: "dbmind::agent",
                session = %session_id,
                code = %err.code_str(),
                error = %err.message,
                "空闲会话探活未通过但不是连接问题，保留会话"
            );
            ProbeOutcome::Unknown
        }
    }
}

/// 后台保活：每隔 [`KEEPALIVE_INTERVAL`] 给「最近活跃、当前空闲」的会话发一句最轻的 SQL。
///
/// 为什么必须有它（真机现象，整库同步跑到 90% 时集中爆发）：尾部只剩大表时，池里的
/// 连接会长时间没有流量；云数据库的公网代理 / NAT 把「3 分钟无流量」的会话表回收掉，
/// 而内核与宿主都只会「记着」它 —— 下一次借出时第一条语句就撞死连接
/// （`Communications link failure`，驱动诊断「收发都是 188 秒前」）。写路径非幂等，
/// 一张大表就要整表回滚重跑。
///
/// 「借出前探活」只能事后发现；保活是**让它根本不失效**：20 秒一句 ping，远小于任何
/// 链路/代理的空闲阈值（实测 180 秒）。
///
/// 三条边界（都很硬）：
/// - 宿主没在跑就什么都不做 —— 这条路径绝不能为了 ping 拉起一个 JVM；
/// - 只挑**空闲**会话 —— 绝不打断正在跑的语句；
/// - 只挑**非常驻**会话（见 `SessionPool::idle_sessions_recent`）—— 用户事务里多一句
///   `select 1` 本身安全，但探活误判会把事务当成「连接已断」丢掉，代价太大。
fn spawn_keepalive(kind: ConnectionKind, host: Arc<AgentHost>, pool: Weak<SessionPool>) {
    if keepalive_disabled() {
        tracing::debug!(target: "dbmind::agent", "后台保活已按 DBMIND_NO_KEEPALIVE 关闭");
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("dbmind-keepalive".to_string())
        .spawn(move || loop {
            std::thread::sleep(KEEPALIVE_INTERVAL);
            // 驱动（连同池）已经释放 ⇒ 线程退休
            let Some(pool) = pool.upgrade() else { return };
            if !host.is_running() {
                // 宿主已经不在了 ⇒ 它上面所有会话必然失效。**绝不为了 ping 拉起 JVM**，
                // 但必须清掉内核的会话缓存：不清的话下一次借出会先撞一次
                // 「会话不存在」，用户看到一次失败；清掉之后下一次调用直接重连。
                host.forget_all_sessions();
                continue;
            }
            let sessions = pool.idle_sessions_recent(KEEPALIVE_WINDOW, KEEPALIVE_MAX_SESSIONS);
            for session_id in sessions {
                // 先占住再发 ping：JDBC 连接不是线程安全的，与业务语句同时用同一条
                // 会把驱动内部状态搅坏。占不到（刚好被借走）就跳过，下一轮再说。
                if !pool.take_idle_session(&session_id) {
                    continue;
                }
                // 只有「连接确实没了」才作废；语句级错误（比如会话正处在
                // aborted 的事务里）不动它 —— 见 `ProbeOutcome`。
                // 超时这里**不**判死（传 false）：保活每 20 秒一发，高负载时排队超时
                // 会把好会话摘掉，变成连接抖动；半开链路留给「借出前探活」兜。
                if probe_session(kind, &host, &session_id, false) != ProbeOutcome::Dead {
                    pool.release_heartbeat(&session_id);
                    continue;
                }
                // 连接没了 ⇒ 作废：宿主侧忘掉它，池里摘掉槽位（我们正占着，可以硬摘）。
                // 这样下一次借用必然重连，而不是再撞一次同一条死连接。
                host.forget_session(&session_id);
                pool.remove_session(&session_id);
                tracing::debug!(
                    target: "dbmind::agent",
                    session = %session_id,
                    "保活发现会话已失效，已作废（下次借用会重连）"
                );
            }
        });
    if let Err(err) = spawned {
        // 起不来只是少了这层保护（借出前探活与整表重跑仍在），不该让连接建不起来
        tracing::warn!(
            target: "dbmind::agent",
            error = %err,
            "后台保活线程启动失败（连接仍可用的，但空闲久了可能被链路掐断）"
        );
    }
}

/// 这条泳道是不是**交互式**的（编辑器页签、用户手点查询）。
///
/// 约定：交互请求一定带亲和键（`session_key` 末尾拼 `|s=<键>`），而批量/内部功能
/// 的键统一以 `internal` 开头（`internal:browse`、`internal:sync-w…`、`internal:rowcount`…）。
/// 于是「有亲和键且不是 internal」就是交互式 —— 这类连接要**常驻**，不参与回收，
/// 手感才对得上 Navicat（切回页签直接出结果，不重新握手）。
fn is_interactive_lane(lane: &str) -> bool {
    match lane.rsplit_once("|s=") {
        Some((_, key)) => !key.starts_with("internal"),
        None => false,
    }
}

impl Driver for AgentDriver {
    fn kind(&self) -> ConnectionKind {
        self.kind
    }

    fn disconnect_all(&self) -> usize {
        // 与固有方法同体：dyn 分发（Registry 按类型调用）走这里
        AgentDriver::disconnect_all(self)
    }

    fn disconnect_connection(&self, name: &str) -> usize {
        AgentDriver::disconnect_connection(self, name)
    }

    /// 按泳道名释放（传输任务收尾）：见 [`Driver::disconnect_lanes`]。
    ///
    /// 顺带把匹配到的**事务泳道**摘掉并解除 pin：这些泳道的所有者（传输任务）
    /// 已经结束，留着 pin 只会让配额永远回收不出它们。
    fn disconnect_lanes(&self, needle: &str) -> usize {
        let dropped: Vec<String> = {
            let mut lanes = self.tx_lanes.lock().unwrap_or_else(|e| e.into_inner());
            let hit: Vec<String> = lanes
                .iter()
                .filter(|lane| lane.contains(needle))
                .cloned()
                .collect();
            for lane in &hit {
                lanes.remove(lane);
            }
            hit
        };
        for lane in dropped {
            self.pool.unpin_lane(&lane);
        }
        // 批量写事务的保护同样按 `sid` 兜底全解：任务异常结束（panic / 取消 / 提前 return）
        // 时若留着 pin，这几条泳道就永远回收不出去，配额会被慢慢吃光。
        self.pool.unpin_matching(|lane| lane.contains(needle));
        self.pool.close_idle_where(|lane| lane.contains(needle))
    }

    /// 标记 / 解除「事务中的批量会话」保护：见 [`Driver::hold_tx_lane`]。
    fn hold_tx_lane(&self, affinity: &str, hold: bool) -> usize {
        AgentDriver::hold_tx_lane(self, affinity, hold)
    }

    /// 会话级事务控制（事务模式，全 agent 数据源通用）。
    ///
    /// 走与查询同一条**亲和泳道**（limit=1）⇒ 事务精确落在编辑器那条物理连接上；
    /// begin 后内核把泳道记进 `tx_lanes`，连接断开重建时 `with_lane` 会自动
    /// 重新对齐 autocommit（commit/rollback 后移除，连接交回 autocommit）。
    fn tx_control(
        &self,
        config: &ConnectionConfig,
        read_only: bool,
        session: &str,
        action: TxAction,
    ) -> Result<serde_json::Value> {
        let lane = self.lane_key(config, read_only, Some(session));
        let wait = Duration::from_secs(15);
        let outcome = self.with_lane(
            config,
            &lane,
            1,
            wait,
            LaneCall {
                read_only,
                retry_on_stale: false,
                // 事务控制本身重试没有意义（旧连接上的事务已经随断开回滚了）
                retry_after_disconnect: false,
                cancel: None,
            },
            |key| {
                let mut params = Map::new();
                params.insert("sessionId".to_string(), json!(key));
                if action == TxAction::Begin {
                    params.insert("autoCommit".to_string(), json!(false));
                }
                let value = self.host.call(None, action.method(), params, Some(wait))?;
                {
                    let mut lanes = self.tx_lanes.lock().expect("tx_lanes");
                    match action {
                        TxAction::Begin => {
                            lanes.insert(lane.clone());
                        }
                        TxAction::Commit | TxAction::Rollback => {
                            lanes.remove(&lane);
                        }
                    }
                }
                Ok(value)
            },
        );
        if outcome.is_ok() {
            match action {
                TxAction::Begin => self.pool.pin_lane(&lane, Pin::Transaction),
                // 交互页签提交完也要**继续常驻**（否则两次查询之间有一个被回收的窗口，
                // 又来一次 1.5~6.4 秒的握手）；批量泳道照旧解除保护。
                // 注意这里要**降级**成 `Interactive` 而不是保持事务保护：事务已经结束，
                // 这条会话可以重新参与保活（否则它又会变成「断了没人探」的那种）。
                TxAction::Commit | TxAction::Rollback => {
                    if is_interactive_lane(&lane) {
                        self.pool.unpin_lane(&lane);
                        self.pool.pin_lane(&lane, Pin::Interactive);
                    } else {
                        self.pool.unpin_lane(&lane);
                    }
                }
            }
        }
        outcome
    }

    fn test(&self, config: &ConnectionConfig, read_only: bool) -> Result<ConnectReport> {
        let started = Instant::now();
        // 自检要验证**配置本身** ⇒ 在一个槽位上强制重连（不复用池里已有的会话）。
        // 单连接类型上它会排在正在执行的请求后面 —— 这是那条声明的应有之义，
        // 但「等不到」会明确报出来，而不是无限期挂着。
        let lane = self.session_key(config, read_only);
        let limit = self.lane_limit(config, None);
        let slot = self.pool.acquire(&lane, limit, SELF_CHECK_POOL_WAIT)?;
        let version = self.ensure_session(config, read_only, slot.session_id(), true)?;
        // 把真实目标显示出来：「我到底连到了哪里」是最常见的事故来源
        let target = if self.kind.is_jdbc() {
            self.kind
                .jdbc_url_template()
                .map(|template| agent::render_jdbc_url(template, config))
                .transpose()?
                .unwrap_or_default()
        } else {
            format!(
                "{}://{}:{}/{}",
                self.kind.protocol().as_str(),
                config.host.as_deref().unwrap_or("-"),
                config.resolved_port().unwrap_or(0),
                config.database.as_deref().unwrap_or("-")
            )
        };
        Ok(ConnectReport {
            connection_id: String::new(),
            kind: self.kind,
            runtime_mode: RuntimeMode::Agent,
            latency_ms: started.elapsed().as_millis() as u64,
            server_version: version,
            message: format!("agent 连接成功（{target}）"),
        })
    }

    fn query(&self, call: QueryCall<'_>) -> Result<QueryResult> {
        let started = Instant::now();
        let lane = self.lane_key(call.config, call.read_only, call.session);
        let limit = self.lane_limit(call.config, call.session);
        // 等连接的预算与查询本身一致：多花的 2s 是留给宿主「先给出结构化错误」的余量
        let wait = Duration::from_millis(call.options.timeout_ms.saturating_add(2_000));
        // 写语句**不重试**（见模块文档）：响应丢失时它可能已经在库里执行过。
        // 只读语句例外：连接已经死了、语句根本没发出去，重跑一次既没副作用，
        // 又正好把「VPN 断连后第一条必失败」抹掉（见 `LaneCall::retry_after_disconnect`）。
        // 带上取消令牌：排队等会话时也要能被取消（见 SessionPool::acquire_cancellable）。
        self.with_lane(
            call.config,
            &lane,
            limit,
            wait,
            LaneCall {
                read_only: call.read_only,
                retry_on_stale: false,
                retry_after_disconnect: crate::statement::is_read_only(
                    self.kind.protocol(),
                    call.sql,
                ),
                cancel: Some(call.cancel),
            },
            |key| {
                let request_id = call.execution_id.to_string();
                // 取消 = 通知宿主中断语句，不是「等下一行结果再检查」
                //
                // **为什么用通道而不是「睡眠 + 轮询一个 AtomicBool」**：旧实现是
                // `loop { if finished { break }; sleep(80ms) }`，而调用结束时又必须
                // `join()` 这个线程 —— 于是**每条语句都白等 0~80ms**（平均 40ms）。
                // 本地库实测一次 `select 1` 从 ~30ms 涨到 ~115ms，全是这个 join 的税；
                // 写侧一条 INSERT 也交一次，是「批量写比 DataX 慢」里最隐蔽的一笔固定开销。
                // 换成通道后：调用一结束就 `send`，watcher 的 `recv_timeout` **立刻**返回
                // Disconnected/Ok，join 几乎零成本；取消的响应粒度仍是 80ms。
                let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
                let watcher = {
                    let host = self.host.clone();
                    let cancel = call.cancel.clone();
                    let request_id = request_id.clone();
                    std::thread::Builder::new()
                        .name("dbmind-agent-cancel".to_string())
                        .spawn(move || loop {
                            match done_rx.recv_timeout(Duration::from_millis(80)) {
                                // 调用已结束（或发送端已丢弃）⇒ 收工
                                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                                Err(RecvTimeoutError::Timeout) => {
                                    if cancel.is_cancelled() {
                                        // 只看「宿主有没有找到那条语句」决定要不要再试：
                                        // - `Ok(false)`：宿主还没登记这条语句 ⇒ 这次取消可能先到了，
                                        //   下一轮再发（否则用户点了取消会**什么都不发生**）；
                                        // - `Ok(true)`：已经打断，收工；
                                        // - `Err`：宿主不支持取消之类，别反复打扰。
                                        match host.cancel(&request_id) {
                                            Ok(false) => {}
                                            _ => break,
                                        }
                                    }
                                }
                            }
                        })
                        .ok()
                };

                let mut params = Map::new();
                params.insert("sessionId".to_string(), json!(key));
                // 元数据类语句（DDL / 索引 / 注释 / 列类型补全）带上目标 schema：代理会
                // "临时切过去 → 执行 → 切回"，好让那句 SQL 里的 current_schema() / ::regclass /
                // object_id() 都看对那一层。**用户自己的查询恒为 None**，行为与以前完全一致。
                if let Some(schema) = call.schema.filter(|s| !s.is_empty()) {
                    params.insert("schema".to_string(), json!(schema));
                }
                params.insert("sql".to_string(), json!(call.sql));
                params.insert("maxRows".to_string(), json!(call.options.max_rows));
                params.insert("timeoutMs".to_string(), json!(call.options.timeout_ms));
                // 要裸值紧凑行：每格一个 {"t":...,"v":...} 时，一行 12 列 ≈ 500 字节文本，
                // 20 万行就是 ~100MB 的 JSON 要逐格拼、逐格解（实测 ≈0.23ms/行，导出的瓶颈）。
                // 宿主只在收到这个标记时才换格式（老宿主忽略它，照旧返回 ✓）。
                params.insert("compact".to_string(), json!(true));

                // 内核等待比宿主自身超时略长：让宿主先给出结构化错误，而不是内核单方面判超时
                let outcome = self.host.call(Some(&request_id), "query", params, Some(wait));

                // 叫醒 watcher（不 join 也能保证它不会再调 cancel：send 之后它立刻 break）
                let _ = done_tx.send(());
                if let Some(handle) = watcher {
                    let _ = handle.join();
                }

                let value = match outcome {
                    Ok(value) => value,
                    Err(err) if err.message.contains("未知方法：preparedBatch") => {
                        // 升级内核但尚未替换宿主 jar 的兼容路径：老宿主不认识这个
                        // 专用方法时，按“不支持预编译批”回退，不让同步直接失败。
                        return Err(DbMindError::new(
                            ErrorCode::DriverNotImplemented,
                            "当前 JDBC 宿主版本不支持预编译批量写入",
                        )
                        .with_detail(err.detail.unwrap_or_default()));
                    }
                    Err(err) => return Err(err),
                };
                let agent_result: AgentQueryResult = serde_json::from_value(value)?;
                Ok(QueryResult {
                    execution_id: request_id,
                    connection_id: String::new(),
                    connection_name: String::new(),
                    // 语句类型由内核判定（宿主不做只读判断，避免两处规则漂移）
                    statement_kind: classify(self.kind.protocol(), call.sql),
                    columns: agent_result.columns,
                    rows: agent_result.rows,
                    row_count: agent_result.row_count,
                    affected_rows: agent_result.affected_rows.map(|v| v.max(0) as usize),
                    truncated: agent_result.truncated,
                    // 由内核统一判定（见 engine::execute），宿主不掺和
                    source_object: None,
                    // 行标识则**由宿主给**（只有它知道 `_id` 的真实 BSON 类型），内核不推翻
                    row_identity: agent_result.identity,
                    duration_ms: started.elapsed().as_millis() as u64,
                    notices: agent_result.notices,
                    // 说清楚「这条语句跑在哪条物理会话上」：既方便排障，
                    // 也让「每个标签页一条会话」在界面上是**看得见**的事实
                    session_id: Some(key.to_string()),
                })
            },
        )
    }

    fn prepared_insert_batch(&self, call: PreparedBatchCall<'_>) -> Result<usize> {
        let lane = self.lane_key(call.config, call.read_only, call.session);
        let limit = self.lane_limit(call.config, call.session);
        let wait = Duration::from_millis(call.options.timeout_ms.saturating_add(2_000));
        self.with_lane(
            call.config,
            &lane,
            limit,
            wait,
            LaneCall {
                read_only: call.read_only,
                retry_on_stale: false,
                // 批处理里是写语句，重跑会重复写入
                retry_after_disconnect: false,
                cancel: Some(call.cancel),
            },
            |key| {
                let request_id = call.execution_id.to_string();
                let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
                let watcher = {
                    let host = self.host.clone();
                    let cancel = call.cancel.clone();
                    let request_id = request_id.clone();
                    std::thread::Builder::new()
                        .name("dbmind-agent-batch-cancel".to_string())
                        .spawn(move || loop {
                            match done_rx.recv_timeout(Duration::from_millis(80)) {
                                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                                Err(RecvTimeoutError::Timeout) => {
                                    if cancel.is_cancelled() {
                                        match host.cancel(&request_id) {
                                            Ok(false) => {}
                                            _ => break,
                                        }
                                    }
                                }
                            }
                        })
                        .ok()
                };

                let rows: Vec<serde_json::Value> = call
                    .rows
                    .iter()
                    .map(|row| {
                        serde_json::Value::Array(
                            row.iter()
                                .map(|cell| match cell {
                                    CellValue::Null | CellValue::Blob { .. } => {
                                        serde_json::Value::Null
                                    }
                                    // 预编译参数也要能带二进制：Java 宿主认这个带标签对象
                                    // 并走 `setBytes`（见 bindPreparedValue）。写成 null 就是
                                    // 静默丢数据 —— BLOB 列在 SQL Server/Oracle 上全是这样丢的。
                                    CellValue::Bytes(data) => {
                                        json!({ "t": "bytes", "v": data })
                                    }
                                    CellValue::Integer(value) => json!(value),
                                    CellValue::Real(value) => json!(value),
                                    CellValue::Decimal(value) => json!(value),
                                    CellValue::Text(value) => json!(value),
                                })
                                .collect(),
                        )
                    })
                    .collect();
                let mut params = Map::new();
                params.insert("sessionId".to_string(), json!(key));
                params.insert("sql".to_string(), json!(call.sql));
                params.insert(
                    "types".to_string(),
                    serde_json::to_value(call.target_types)?,
                );
                params.insert("rows".to_string(), serde_json::Value::Array(rows));
                params.insert("timeoutMs".to_string(), json!(call.options.timeout_ms));
                // 调用方已开事务：宿主只能用保存点划批，不能自己 commit（见
                // `PreparedBatchCall::in_transaction`）。
                if call.in_transaction {
                    params.insert("inTransaction".to_string(), json!(true));
                }
                // 方言保存点语句：宿主在「JDBC 保存点建不出来」时改用它们划批
                //（SQL Server 的真机组合：SQL 文本开的事务 + 驱动认为 autocommit=true）。
                if let Some((save, undo)) = call.savepoint_sql {
                    params.insert("savepointSave".to_string(), json!(save));
                    params.insert("savepointRollback".to_string(), json!(undo));
                }

                let outcome = self
                    .host
                    .call(Some(&request_id), "preparedBatch", params, Some(wait));
                let _ = done_tx.send(());
                if let Some(handle) = watcher {
                    let _ = handle.join();
                }
                let value = outcome?;
                if value.get("supported").and_then(|v| v.as_bool()) == Some(false) {
                    // 宿主会带 `reason`（究竟缺 batchUpdates / transactions / savepoints）。
                    // 丢掉它，排障就只剩「不支持」这三个字 —— SQL Server 这一路正是这样
                    // 被误判了很久（metadata 三个能力位其实都是 true）。
                    let reason = value
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .unwrap_or("宿主未给出原因");
                    return Err(DbMindError::new(
                        ErrorCode::DriverNotImplemented,
                        format!("目标 JDBC 驱动不支持可回滚的预编译批量写入：{reason}"),
                    ));
                }
                if value.get("sessionReset").and_then(|v| v.as_bool()) == Some(false) {
                    // 数据已经提交，但会话无法恢复 autocommit。丢掉宿主与内核两侧的
                    // 缓存，让下一次调用重建一条干净连接；本次不能报失败，否则调用方
                    // 会把已经提交的数据再写一遍。
                    self.host.forget_session(key);
                }
                let affected = value
                    .get("affectedRows")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                Ok(affected)
            },
        )
    }

    fn load_data_local(&self, call: LoadDataCall<'_>) -> Result<u64> {
        let lane = self.lane_key(call.config, call.read_only, call.session);
        let limit = self.lane_limit(call.config, call.session);
        let wait = Duration::from_millis(call.options.timeout_ms.saturating_add(2_000));
        self.with_lane(
            call.config,
            &lane,
            limit,
            wait,
            LaneCall {
                read_only: call.read_only,
                // 装载是写语句：既不能换会话重试（重跑会重复写入），也不能在失败后
                // 假设「没写进去」—— 由调用方按错误类型决定是否回退 INSERT。
                retry_on_stale: false,
                retry_after_disconnect: false,
                cancel: None,
            },
            |key| {
                let mut params = Map::new();
                params.insert("sessionId".to_string(), json!(key));
                params.insert("sql".to_string(), json!(call.sql));
                params.insert("data".to_string(), json!(call.data));
                params.insert("timeoutMs".to_string(), json!(call.options.timeout_ms));
                let value = self.host.call(
                    Some(call.execution_id),
                    "loadDataLocal",
                    params,
                    Some(wait),
                )?;
                if value.get("supported").and_then(|v| v.as_bool()) == Some(false) {
                    return Err(DbMindError::new(
                        ErrorCode::DriverNotImplemented,
                        "目标 JDBC 驱动不支持 LOAD DATA LOCAL INFILE",
                    ));
                }
                Ok(value
                    .get("affectedRows")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0))
            },
        )
    }

    fn list_tables(&self, call: MetaCall<'_>) -> Result<Vec<TableInfo>> {
        self.with_session(call.config, call.read_only, |session_id| {
            let mut params = Map::new();
            params.insert("sessionId".to_string(), json!(session_id));
            let value = self.host.call(None, "tables", params, None)?;
            Ok(agent::map_tables(value))
        })
    }

    fn list_columns(&self, call: MetaCall<'_>, table: &str) -> Result<Vec<ColumnDetail>> {
        self.with_session(call.config, call.read_only, |session_id| {
            let mut params = Map::new();
            params.insert("sessionId".to_string(), json!(session_id));
            params.insert("table".to_string(), json!(table));
            // schema 只在 PG / SQL Server / Kingbase 这类**分层**库里非空。
            // 代理收到它就用 `connection.setSchema()` 让 getColumns / getPrimaryKeys
            // 都看这一层 —— 否则它们只看连接的默认 schema（PG=public / SQLServer=dbo），
            // 别的 schema 下的表就会"能列出表、字段却是空的"（用户反馈的正是这个）。
            // 不带这个参数时行为与以前完全一致（老代理收到未知键会忽略）。
            if let Some(schema) = call.schema.filter(|s| !s.is_empty()) {
                params.insert("schema".to_string(), json!(schema));
            }
            let value = self.host.call(None, "columns", params, None)?;
            let columns: Vec<ColumnDetail> = serde_json::from_value(value)?;
            Ok(columns)
        })
    }
}

/// 从 connect 回执里取出**被宿主按配额淘汰**的会话 id。
///
/// 为什么淘汰由宿主挑：只有它知道「这条会话此刻有没有正在执行的语句」。
/// 内核这边只需要知道「哪些会话没了」——把缓存清掉，下次调用自然重连。
pub(crate) fn evicted_sessions(result: &serde_json::Value) -> Vec<String> {
    result
        .get("evictedSessions")
        .and_then(|value| value.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// 文件型类型：确保数据库文件的父目录存在。
///
/// 为什么由内核来做：H2 / Derby 这类嵌入式引擎在父目录不存在时**不报错**，
/// 而是给你一个临时空库 —— 用户看到的现象是「建表成功，但下一个语句说没有表」，
/// 极难自行定位（实测踩过）。与其让人猜，不如提前把目录建好。
pub(crate) fn ensure_file_parent(config: &ConnectionConfig) -> Result<()> {
    let Some(path) = config.resolved_file() else {
        return Ok(());
    };
    let path = std::path::Path::new(path);
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() || parent.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(parent).map_err(|err| {
        DbMindError::new(
            ErrorCode::ConnInvalid,
            format!("无法创建数据文件目录：{}", parent.display()),
        )
        .with_detail(err.to_string())
    })
}

#[cfg(test)]
mod lane_tests {
    use super::is_interactive_lane;

    /// 「会话不存在」要能认出来（含宿主带后缀的写法），别的连接错误不能误判。
    ///
    /// 判错的后果不对称：把 `Communications link failure` 误判成「会话不存在」会给写语句
    /// 重试一次（可能写两遍）；反过来漏判则这条泳道永久坏掉（真机：重传第一张表秒失败）。
    #[test]
    fn 会话不存在只认宿主原话() {
        use super::is_session_gone;
        assert!(is_session_gone("会话不存在"));
        assert!(is_session_gone("会话不存在：sess-abc"));
        assert!(is_session_gone(
            "会话不存在（该会话上次失败：Communications link failure）"
        ));
        assert!(!is_session_gone("Communications link failure"));
        assert!(!is_session_gone("写入连接已断：连接失败：Communications link failure"));
    }

    /// 交互泳道只认「有亲和键且不是 internal」—— 判错会把批量会话也 pin 死，
    /// 配额再也回收不出来（正是「会话不存在」那类故障的翻版）。
    #[test]
    fn 交互泳道只认非_internal_的亲和键() {
        let base = "mysql|本地|c.host|3306|u|db|false|-";
        assert!(is_interactive_lane(&format!("{base}|s=ui:tab-1")));
        assert!(is_interactive_lane(&format!("{base}|s=tab-abc")));
        assert!(!is_interactive_lane(&format!("{base}|s=internal:sync-w0-0")));
        assert!(!is_interactive_lane(&format!("{base}|s=internal:browse")));
        // 没有亲和键（元数据浏览）不 pin
        assert!(!is_interactive_lane(base));
    }
}
