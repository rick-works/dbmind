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
//! - **查询**不是幂等的 ⇒ 进程/响应异常一律不重试，交由上层决定。

use super::session_pool::{SessionBudget, SessionPool};
use super::{Driver, MetaCall, QueryCall};
use crate::agent::{self, AgentHost, AgentQueryResult};
use crate::error::{DbMindError, ErrorCode, Result};
use crate::statement::classify;
use crate::types::{ColumnDetail, ConnectReport, ConnectionConfig, QueryResult, TableInfo};
use crate::{CancelToken, ConnectionKind, RuntimeMode};
use serde_json::{json, Map};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 结构浏览这类幂等调用等一条空闲会话的预算。
///
/// 为什么要有上限而不是无限等：单连接类型的元数据要搭**同一条**（见
/// `metadata_session_key`），此时一条长查询就会把结构浏览挡住 ——
/// 与其让界面无声地转圈，不如到点后明确说「这条连接的并发上限是 1」。
const METADATA_POOL_WAIT: Duration = Duration::from_secs(30);

/// 连通性自检等一条空闲会话的预算（自检本身也该有兜底）。
const SELF_CHECK_POOL_WAIT: Duration = Duration::from_secs(30);

/// 一次调用的泳道参数。
///
/// 收成一个结构是为了**别让参数一路加到签名读不懂**（也过不了 clippy 的 7 参数线）：
/// 这三个都是「与池 / 重试有关」的零碎开关，放在一起最自然。
struct LaneCall<'a> {
    read_only: bool,
    /// 幂等调用（结构浏览、自检）才重试；查询不重试（见模块文档）
    retry_on_stale: bool,
    /// 只有查询带令牌：**排队等会话时也要能被取消**（见 `SessionPool::acquire_cancellable`）
    cancel: Option<&'a CancelToken>,
}

pub struct AgentDriver {
    kind: ConnectionKind,
    host: Arc<AgentHost>,
    /// 会话池：把「一个连接」映射到「一到多条物理会话」（见 `session_pool` 模块文档）
    pool: SessionPool,
    /// 全局会话配额（跨连接、跨类型、跨宿主；由引擎在设置变更时更新）
    budget: Arc<SessionBudget>,
}

impl AgentDriver {
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
        Self {
            kind,
            host,
            pool: SessionPool::new(budget.clone(), closer),
            budget,
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
    pub(crate) fn session_key(&self, config: &ConnectionConfig, read_only: bool) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}",
            self.kind.key(),
            config.name,
            config.host.as_deref().unwrap_or("-"),
            config
                .resolved_port()
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".into()),
            config.database.as_deref().unwrap_or("-"),
            config.resolved_file().unwrap_or("-"),
            read_only
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
            let url = agent::render_jdbc_url(template, config)?;
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
                         mssql-jdbc_auth-<驱动版本>.{arch}.dll 放到 {} 后重启 DBMind（库路径只在 \
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
                if !extra.is_empty() {
                    params.insert("params".to_string(), serde_json::Value::Object(extra.clone()));
                }
            }
        } else {
            // 原生协议：宿主自带驱动库，这里只给连接字段
            params.insert("host".to_string(), json!(config.host));
            params.insert("port".to_string(), json!(config.resolved_port().unwrap_or(0)));
            params.insert("database".to_string(), json!(config.database));
            params.insert("username".to_string(), json!(config.username));
            params.insert("password".to_string(), json!(config.password));
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
            return Err(DbMindError::new(
                ErrorCode::DriverNotImplemented,
                format!("驱动宿主不支持 agentKey={agent_key}"),
            )
            .with_detail(
                "该类型需要专属协议宿主（如 Redis / Elasticsearch 各有自己的协议），\
                 接入方式见 plugins/connection-types 中该类型的声明",
            ));
        }

        // 文件型连接先补齐父目录（嵌入式引擎在父目录缺失时会给你一个临时空库）
        if self.kind.local_file() {
            ensure_file_parent(config)?;
        }

        let params = self.connect_params(session_id, config, read_only)?;
        let result = self
            .host
            .call(None, "connect", params, Some(Duration::from_secs(25)))?;
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
        let session_id = slot.session_id().to_string();
        self.ensure_session(config, call.read_only, &session_id, false)?;
        match op(&session_id) {
            Err(err) if call.retry_on_stale && err.code == ErrorCode::ConnConnectFailed => {
                tracing::warn!(target: "dbmind::agent", "会话已失效，重连后重试一次幂等调用");
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
                Err(err)
            }
            other => other,
        }
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
                cancel: None,
            },
            op,
        )
    }
}

impl Driver for AgentDriver {
    fn kind(&self) -> ConnectionKind {
        self.kind
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
        // 查询**不重试**（见模块文档）：会话失效时如实报错，由上层决定怎么办。
        // 带上取消令牌：排队等会话时也要能被取消（见 SessionPool::acquire_cancellable）。
        self.with_lane(
            call.config,
            &lane,
            limit,
            wait,
            LaneCall {
                read_only: call.read_only,
                retry_on_stale: false,
                cancel: Some(call.cancel),
            },
            |key| {
                let request_id = call.execution_id.to_string();
                // 取消 = 通知宿主中断语句，不是「等下一行结果再检查」
                let finished = Arc::new(AtomicBool::new(false));
                let watcher = {
                    let host = self.host.clone();
                    let cancel = call.cancel.clone();
                    let finished = finished.clone();
                    let request_id = request_id.clone();
                    std::thread::Builder::new()
                        .name("dbmind-agent-cancel".to_string())
                        .spawn(move || {
                            while !finished.load(Ordering::SeqCst) {
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
                                std::thread::sleep(Duration::from_millis(80));
                            }
                        })
                        .ok()
                };

                let mut params = Map::new();
                params.insert("sessionId".to_string(), json!(key));
                params.insert("sql".to_string(), json!(call.sql));
                params.insert("maxRows".to_string(), json!(call.options.max_rows));
                params.insert("timeoutMs".to_string(), json!(call.options.timeout_ms));
                // 要裸值紧凑行：每格一个 {"t":...,"v":...} 时，一行 12 列 ≈ 500 字节文本，
                // 20 万行就是 ~100MB 的 JSON 要逐格拼、逐格解（实测 ≈0.23ms/行，导出的瓶颈）。
                // 宿主只在收到这个标记时才换格式（老宿主忽略它，照旧返回 ✓）。
                params.insert("compact".to_string(), json!(true));

                // 内核等待比宿主自身超时略长：让宿主先给出结构化错误，而不是内核单方面判超时
                let outcome = self.host.call(Some(&request_id), "query", params, Some(wait));

                finished.store(true, Ordering::SeqCst);
                if let Some(handle) = watcher {
                    let _ = handle.join();
                }

                let value = outcome?;
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
