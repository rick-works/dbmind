//! 驱动层：**按 runtimeMode 分派到不同物理实现**。
//!
//! 所有壳都只经过 `DriverRegistry::resolve`，因此「某类型没实现」是一个**明确的
//! 错误码**（DBMIND-DRV-0001），而不是静默失败或降级到别的东西。

pub mod agent_driver;
pub mod session_pool;
pub mod sqlite;

pub use session_pool::SessionBudget;

#[cfg(test)]
mod agent_driver_tests;

use crate::error::{DbMindError, ErrorCode, Result};
use crate::types::{ColumnDetail, ConnectReport, ConnectionConfig, QueryOptions, QueryResult, TableInfo};
use crate::{CancelToken, ConnectionKind, RuntimeMode};
use std::sync::Arc;

/// 一次查询调用的全部上下文，避免 trait 方法参数爆炸。
pub struct QueryCall<'a> {
    pub config: &'a ConnectionConfig,
    pub sql: &'a str,
    pub options: &'a QueryOptions,
    pub execution_id: &'a str,
    pub cancel: &'a CancelToken,
    /// 连接是否为只读——驱动应据此以只读方式打开物理连接（双保险）。
    pub read_only: bool,
    /// 会话亲和键（见 `QueryRequest::session`）：同一个键的语句落在**同一条物理会话**上。
    ///
    /// `None` = 不要求亲和，按「这个连接复用一条」处理（CLI / MCP 的默认行为）。
    /// 单连接类型会**忽略**它 —— 那些类型只有一条物理连接可给（见驱动实现）。
    pub session: Option<&'a str>,
}

/// 一次元数据调用的上下文。
///
/// 为什么元数据也要带上只读性：**元数据该用哪条物理连接，取决于连接的只读性
/// 与类型的池化声明**：
///
/// - `singleConnectionPool` 的类型（嵌入式/文件引擎）只允许一条连接，
///   元数据必须搭数据那条，否则看不到连接级作用域里的东西（临时表、`:memory:` 库）；
/// - `metadataConnectionScoped` 的类型（如 SQL Server 的 `#temp`）还要求**会话**同作用域。
///
/// 这两件事都不是驱动自己猜得出来的，所以由调用方（内核）明确传下去。
pub struct MetaCall<'a> {
    pub config: &'a ConnectionConfig,
    /// 该连接（引擎层面）是否为只读。
    pub read_only: bool,
}

pub trait Driver: Send + Sync {
    fn kind(&self) -> ConnectionKind;

    /// 连通性自检（不做任何写操作）。
    fn test(&self, config: &ConnectionConfig, read_only: bool) -> Result<ConnectReport>;

    fn query(&self, call: QueryCall<'_>) -> Result<QueryResult>;

    fn list_tables(&self, call: MetaCall<'_>) -> Result<Vec<TableInfo>>;

    fn list_columns(&self, call: MetaCall<'_>, table: &str) -> Result<Vec<ColumnDetail>>;

    /// 主动断开**空闲**会话（返回断开条数）。默认 0 = 该驱动无会话池。
    ///
    /// 用途：数据库侧授权/密码变更后，内核连接池里的旧会话还带着旧的**全局权限快照**
    ///（MySQL 的全局权限变更只对新建连接生效），断开重连即可拿到新权限，
    /// 用户不必重启整个应用。正忙的会话不能打断 —— 由各驱动自己保证。
    fn disconnect_all(&self) -> usize {
        0
    }

    /// 只断**某个连接**（按连接名匹配泳道）的空闲会话。默认 0 = 无会话池。
    ///
    /// 「关闭连接」的精确版本：同类型的其它连接共用一个驱动池，
    /// 断整个类型会让它们也丢掉缓存会话（下次用时重建，几百毫秒）——
    /// 功能上无害但没必要，界面上其它连接的图标明明还亮着。
    fn disconnect_connection(&self, _name: &str) -> usize {
        0
    }

    /// 会话级事务控制（事务模式）：begin 关掉该会话的 autocommit，
    /// commit / rollback 提交或回滚。
    ///
    /// 默认不支持（`DBMIND-DRV-0001`）—— 目前只有走 agent 宿主的驱动实现了它
    /// （`setautocommit` / `commit` / `rollback` RPC，全部走 JDBC 标准接口，
    /// ⇒ **所有 agent 类型的数据源通用**，不碰任何方言语法）。
    fn tx_control(
        &self,
        _config: &ConnectionConfig,
        _read_only: bool,
        _session: &str,
        _action: TxAction,
    ) -> Result<serde_json::Value> {
        Err(DbMindError::new(
            ErrorCode::DriverNotImplemented,
            "当前数据库类型不支持会话级事务",
        ))
    }
}

/// 事务控制动作（见 [`Driver::tx_control`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxAction {
    /// 开启手动事务（autocommit=false）
    Begin,
    Commit,
    Rollback,
}

impl TxAction {
    /// 宿主侧 RPC 方法名（见 agent-jdbc 的 dispatch 方法表）。
    pub fn method(self) -> &'static str {
        match self {
            TxAction::Begin => "setautocommit",
            TxAction::Commit => "commit",
            TxAction::Rollback => "rollback",
        }
    }

    /// 历史留痕用的语句文本。
    pub fn as_sql(self) -> &'static str {
        match self {
            TxAction::Begin => "BEGIN",
            TxAction::Commit => "COMMIT",
            TxAction::Rollback => "ROLLBACK",
        }
    }
}

pub struct DriverRegistry {
    drivers: Vec<Box<dyn Driver>>,
    /// agent 宿主：一个宿主承载一类 agentKey（JDBC 宿主是通用的，MongoDB 是专属的）。
    hosts: Vec<Arc<crate::AgentHost>>,
    /// 全局会话配额：跨连接、跨类型、跨宿主共享给所有 agent 驱动（见 `SessionBudget`）。
    session_budget: Arc<SessionBudget>,
}

impl DriverRegistry {
    /// 内置驱动集合。
    ///
    /// 这里**没有**逐个类型列举数据库：凡是 YAML 声明 `runtimeMode=agent` 的类型，
    /// 都按「类型 → 宿主」规则自动绑定（专属宿主优先，其次通用宿主）。
    /// 新增数据库类型时本函数不需要改动 —— 「类型是数据、不是代码」的兑现点。
    pub fn with_builtin() -> Self {
        let hosts: Vec<Arc<crate::AgentHost>> = crate::AgentHostSpec::all()
            .into_iter()
            .map(|spec| Arc::new(crate::AgentHost::new(spec)))
            .collect();

        let session_budget = Arc::new(SessionBudget::default());
        let mut drivers: Vec<Box<dyn Driver>> =
            vec![Box::new(sqlite::SqliteDriver::new(session_budget.clone()))];
        for kind in ConnectionKind::ALL.iter().copied() {
            if kind.runtime_mode() != RuntimeMode::Agent {
                continue;
            }
            if let Some(host) = pick_host(&hosts, kind) {
                drivers.push(Box::new(agent_driver::AgentDriver::new(
                    kind,
                    host,
                    session_budget.clone(),
                )));
            }
        }
        Self {
            drivers,
            hosts,
            session_budget,
        }
    }

    /// 全局会话配额：引擎在启动与设置变更时写它，驱动**实时**读到（共享原子量）。
    pub fn session_budget(&self) -> Arc<SessionBudget> {
        self.session_budget.clone()
    }

    /// 断开指定类型的全部**空闲**会话，返回断开条数。
    ///
    /// 会话按「类型驱动」缓存（同一类型的多条连接共用一个驱动池，lane 区分具体连接），
    /// 所以这里是**按类型**断：同类型的其它连接只是丢了缓存的会话，下次用时重建
    ///（几百毫秒），无副作用。典型用途见 [`Driver::disconnect_all`]。
    pub fn disconnect_all(&self, kind: ConnectionKind) -> usize {
        self.drivers
            .iter()
            .filter(|driver| driver.kind() == kind)
            .map(|driver| driver.disconnect_all())
            .sum()
    }

    /// 只断**指定连接**（按名字）的空闲会话，返回断开条数。
    ///
    /// 「关闭连接」走这里 —— 同类型的其它连接不受影响（见 [`Driver::disconnect_connection`]）。
    pub fn disconnect_connection(&self, kind: ConnectionKind, name: &str) -> usize {
        self.drivers
            .iter()
            .filter(|driver| driver.kind() == kind)
            .map(|driver| driver.disconnect_connection(name))
            .sum()
    }

    /// 全部 agent 宿主（供壳层显示状态、或显式关闭）。
    pub fn hosts(&self) -> &[Arc<crate::AgentHost>] {
        &self.hosts
    }

    /// 某类型该由哪个宿主承载（不启动进程）。
    pub fn host_for(&self, kind: ConnectionKind) -> Option<Arc<crate::AgentHost>> {
        pick_host(&self.hosts, kind)
    }

    pub fn register(&mut self, driver: Box<dyn Driver>) {
        self.drivers.push(driver);
    }

    pub fn implemented(&self, kind: ConnectionKind) -> bool {
        self.drivers.iter().any(|d| d.kind() == kind)
    }

    /// 解析类型的执行驱动；未接入的类型给出**可解释**的错误。
    pub fn resolve(&self, kind: ConnectionKind) -> Result<&dyn Driver> {
        self.drivers
            .iter()
            .map(|d| d.as_ref())
            .find(|d| d.kind() == kind)
            .ok_or_else(|| unimplemented_driver(kind))
    }
}

/// 未接入类型时的统一错误。文案里带上「该怎么接」，减少来回沟通。
pub fn unimplemented_driver(kind: ConnectionKind) -> DbMindError {
    let mode = kind.runtime_mode();
    let hint = match mode {
        RuntimeMode::Native => format!(
            "需要在 crates/dbmind-core/src/drivers 下新增原生驱动，并在 DriverRegistry::with_builtin 注册；\
             同时把 plugins/connection-types/{}.yaml 的 runtimeMode 保持为 native",
            kind.key()
        ),
        RuntimeMode::Agent => format!(
            "该类型声明走 agent 运行时（agentKey = {}），需要先接入外置驱动进程协议",
            kind.agent_key().unwrap_or("<未声明>")
        ),
        RuntimeMode::External => "该类型声明走 external 运行时，需要安装对应插件".to_string(),
        RuntimeMode::File => "该类型声明走 file 运行时，需要接入文件型引擎子进程".to_string(),
    };
    DbMindError::new(
        ErrorCode::DriverNotImplemented,
        format!(
            "{}（{} 运行时）的执行路径尚未接入内核",
            kind.label(),
            mode.as_str()
        ),
    )
    .with_detail(hint)
}

/// 当前内核已实现的类型（由生成代码的 `implemented()` 引用）。
///
/// 注意区分两个概念：`implemented` 表示「内核有执行路径」，
/// 驱动是否**就绪**（JDBC jar 是否装了 / 专属宿主是否构建了）是运行期事实，
/// 未就绪会返回 `DBMIND-DRV-0002` 并说明怎么补——不静默失败，也不误报可用。
pub fn builtin_kinds() -> Vec<ConnectionKind> {
    let mut kinds = vec![ConnectionKind::Sqlite];
    kinds.extend(
        ConnectionKind::ALL
            .iter()
            .copied()
            .filter(|k| k.runtime_mode() == RuntimeMode::Agent && crate::agent::has_host_for(*k)),
    );
    kinds
}

/// 类型 → 宿主：**专属宿主优先**（按 YAML 的 agentKey 精确认领），其次通用宿主。
fn pick_host(hosts: &[Arc<crate::AgentHost>], kind: ConnectionKind) -> Option<Arc<crate::AgentHost>> {
    if let Some(key) = kind.agent_key() {
        if let Some(host) = hosts.iter().find(|h| h.spec().declares(key)) {
            return Some(host.clone());
        }
    }
    if kind.is_jdbc() {
        return hosts.iter().find(|h| h.spec().generic).cloned();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::all_descriptors;

    #[test]
    fn 已实现的类型能解析出驱动() {
        let registry = DriverRegistry::with_builtin();
        assert!(registry.resolve(ConnectionKind::Sqlite).is_ok());
        assert!(registry.implemented(ConnectionKind::Sqlite));
        // JDBC 类型由 agent 宿主统一承载（注册依据是 YAML 的 runtimeMode + jdbc 元数据）
        assert!(registry.resolve(ConnectionKind::Postgresql).is_ok());
        assert!(registry.resolve(ConnectionKind::H2).is_ok());
        assert!(registry.implemented(ConnectionKind::Oracle));
    }

    #[test]
    fn 所有声明为_agent_的类型都有宿主() {
        let registry = DriverRegistry::with_builtin();
        // 走到这一步的意义：DRV-0001「未接入」不再出现 —— 每个声明都有承载它的宿主
        for kind in ConnectionKind::ALL
            .iter()
            .copied()
            .filter(|k| k.runtime_mode() == RuntimeMode::Agent)
        {
            assert!(registry.resolve(kind).is_ok(), "{} 应能解析出驱动", kind.key());
            assert!(
                registry.host_for(kind).is_some(),
                "{} 应绑定到某个宿主",
                kind.key()
            );
        }
        assert!(
            registry.resolve(ConnectionKind::Sqlite).is_ok(),
            "SQLite 是内核原生实现"
        );
        // 专属宿主不能被通用宿主「抢走」
        assert_eq!(
            registry.host_for(ConnectionKind::Mongodb).unwrap().spec().id,
            "mongodb"
        );
        assert_eq!(
            registry.host_for(ConnectionKind::Redis).unwrap().spec().id,
            "redis"
        );
        assert_eq!(
            registry
                .host_for(ConnectionKind::Elasticsearch)
                .unwrap()
                .spec()
                .id,
            "elasticsearch"
        );
    }

    #[test]
    fn 未接入的类型错误码仍可构造() {
        // 17 种类型已全部接入，但这条错误路径必须保留：新增类型时它是唯一的失败出口
        // （先加 YAML 声明，再补宿主 —— 中间那段时间用户看到的就是它）
        let err = unimplemented_driver(ConnectionKind::Redis);
        assert_eq!(err.code, ErrorCode::DriverNotImplemented);
        assert_eq!(err.code_str(), "DBMIND-DRV-0001");
        assert!(err.detail.is_some(), "应给出如何接入的提示");
    }

    /// 守卫：**已接入的非 JDBC 类型必须显式声明非 sql 协议**。
    ///
    /// 少了这条，将来接入 Redis 时忘了在 YAML 写 `protocol`，闸门就会拿 SQL 词法
    /// 去判 Redis 命令 —— 默认虽然保守（判成 Unknown ⇒ 按非只读处理），但那是运气，
    /// 不是设计。
    #[test]
    fn 已接入的非_jdbc_类型必须声明非_sql_协议() {
        for kind in ConnectionKind::ALL.iter().copied().filter(|k| k.implemented()) {
            if kind.runtime_mode() == RuntimeMode::Agent && !kind.is_jdbc() {
                assert_ne!(
                    kind.protocol(),
                    crate::RuntimeProtocol::Sql,
                    "{} 已接入但不是 JDBC 类型，必须在 YAML 里声明 protocol",
                    kind.key()
                );
            }
        }
    }

    #[test]
    fn 文件型类型也由_agent_承载() {
        let registry = DriverRegistry::with_builtin();
        // DuckDB 有官方 JDBC 驱动 ⇒ 走通用宿主，而不是专属 sidecar
        assert!(registry.resolve(ConnectionKind::Duckdb).is_ok());
        assert!(ConnectionKind::Duckdb.is_jdbc());
        assert!(ConnectionKind::Duckdb.local_file());
        assert_eq!(ConnectionKind::Duckdb.runtime_mode(), RuntimeMode::Agent);
    }

    #[test]
    fn 类型目录与已实现集合一致() {
        let descriptors = all_descriptors();
        assert!(!descriptors.is_empty());
        assert_eq!(
            descriptors.iter().filter(|d| d.implemented).count(),
            builtin_kinds().len(),
            "implemented 标记应与 builtin_kinds 一致"
        );
        // 目录顺序稳定（按 YAML order）
        let orders: Vec<i32> = descriptors.iter().map(|d| d.driver_store_order).collect();
        assert!(orders.windows(2).all(|w| w[0] <= w[1] || w[0] >= w[1]));
    }
}
