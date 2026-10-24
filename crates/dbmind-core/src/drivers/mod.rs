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
use crate::types::{
    CellValue, ColumnDetail, ConnectReport, ConnectionConfig, QueryOptions, QueryResult, TableInfo,
};
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
    /// 在哪个 **schema** 下执行这条语句（PG / SQL Server / Kingbase 的「库.模式」里那一层）。
    ///
    /// 代理会"临时切 schema → 执行 → 切回"（`connection.setSchema`），因为元数据 SQL
    /// 用的 `current_schema()` / `::regclass` / `object_id()` 只看连接自己的 schema。
    /// `None` = 不动它，行为与以前完全一致（用户自己的查询永远走这一支）。
    pub schema: Option<&'a str>,
}

/// 一次内部 INSERT 预编译批处理的上下文。
///
/// 这不是普通 SQL 查询的替身：宿主只接受单条 INSERT、逐行绑定参数，并为整批
/// 提供可回滚的成功/失败边界。放在专用 trait 方法里，公开 SQL 入口就不会因为
/// 出现 `?` 占位符而意外切到另一套执行语义。
pub struct PreparedBatchCall<'a> {
    pub config: &'a ConnectionConfig,
    pub sql: &'a str,
    /// 与 INSERT 的列一一对应的目标列类型。
    pub target_types: &'a [Option<String>],
    /// 每行的单元格，列数必须与 `target_types` 一致。
    pub rows: &'a [Vec<CellValue>],
    pub options: &'a QueryOptions,
    pub execution_id: &'a str,
    pub cancel: &'a CancelToken,
    pub read_only: bool,
    pub session: Option<&'a str>,
    /// 调用方**已经在这个会话上开了事务**（同步的写入插件会先发 `begin transaction`）。
    ///
    /// 宿主据此改用「保存点」划批，而不是自己 `setAutoCommit(false)` + `commit`：
    /// 后者会把调用方的事务就地提交掉，等调用方收尾再发 `commit` 时 SQL Server 直接报
    /// `The COMMIT TRANSACTION request has no corresponding BEGIN TRANSACTION`
    /// （真机踩到：数据其实已经进去了，结果却报失败）。
    pub in_transaction: bool,
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
    /// 要读的 **schema**（PG / SQL Server / Kingbase 的「库.模式」里的模式那一层）。
    ///
    /// 为什么放在这里而不是加进 trait 方法的参数：这些元数据方法（列 / 表 / 索引）
    /// 都会往下走到代理，而"看哪一层 schema"对它们是同一件事；加进 `MetaCall`
    /// 就不用逐个改签名（改签名会波及 CLI / MCP / AI / 备份等十来处调用）。
    /// 非分层库（MySQL 系、SQLite、Doris…）恒为 `None`。
    pub schema: Option<&'a str>,
    }

pub trait Driver: Send + Sync {
    fn kind(&self) -> ConnectionKind;

    /// 连通性自检（不做任何写操作）。
    fn test(&self, config: &ConnectionConfig, read_only: bool) -> Result<ConnectReport>;

    fn query(&self, call: QueryCall<'_>) -> Result<QueryResult>;

    /// 内部 INSERT 的预编译批处理。默认不支持，调用方必须回退到普通 SQL。
    ///
    /// 只允许数据传输这类内核内部链路调用；实现方还必须自行校验 SQL 为单条
    /// INSERT。返回值是整批成功写入的行数。
    fn prepared_insert_batch(&self, _call: PreparedBatchCall<'_>) -> Result<usize> {
        Err(DbMindError::new(
            ErrorCode::DriverNotImplemented,
            "当前数据库驱动不支持预编译批量写入",
        ))
    }

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

    /// 断开**泳道名包含 `needle`** 的空闲会话。默认 0 = 无会话池。
    ///
    /// 用途：数据传输给每张表一套**专属亲和键**（`internal:sync-w<表>-<n>`、
    /// `internal:sync-r<表>-<n>`、`internal:sync-u<表>`），表与表之间因此不会串事务。
    /// 但**跑完不还回去**的话，宿主连接数会随表数**单调增长**：几百张表的整库同步
    /// 很快顶到 `session.maxPerHost`（默认 32），之后每开一条新会话都得先挤掉一条
    /// 空闲会话 —— 被挤掉的常常是**正在用**的那条（事务已开、这一瞬恰好没在跑语句），
    /// 用户看到的是「会话不存在（agent 可能已重启）」「写入连接已断」，整表剩余行被
    /// 放弃；同时每次用到那条泳道都要重连（真机实测一条连接 1.5~6.4 秒）。
    ///
    /// 按 `sid`（每张表唯一）释放之后，连接数只与「同时跑几张表」有关，
    /// 与「一共多少张表」无关，配额也就不会被占满。
    fn disconnect_lanes(&self, _needle: &str) -> usize {
        0
    }

    /// 把「**事务中**的批量会话」标成受保护 / 解除保护（按亲和键匹配泳道）。默认 0 = 无会话池。
    ///
    /// 为什么需要（真机故障）：批量写的每路 writer 自己开事务（`set autocommit=0`），
    /// 但它在**批与批之间**会把槽位还回池里 —— 那一刻它是「空闲」的，而事务还开着、
    /// 里面压着一整页未提交的行。配额顶满时回收按「最久未用的空闲会话」挑中它并关掉，
    /// 服务端随即回滚这些行，写侧却以为写成功了，用户看到
    /// 「写入连接已断：会话不存在（agent 可能已重启）：…|s=internal:sync-w…」。
    /// 这类会话必须和交互式事务一样排除在回收 / 空闲清扫 / 保活之外。
    ///
    /// 配对规则：`begin` 成功后 hold，`commit` / `rollback` / 收尾置 false；
    /// 任务收尾的 [`Driver::disconnect_lanes`] 再按 `sid` 兜底全解，避免异常路径漏掉。
    fn hold_tx_lane(&self, _affinity: &str, _hold: bool) -> usize {
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

    /// 断开**泳道名含 `needle`** 的空闲会话（按类型分发，见 [`Driver::disconnect_lanes`]）。
    pub fn disconnect_lanes(&self, kind: ConnectionKind, needle: &str) -> usize {
        self.drivers
            .iter()
            .filter(|driver| driver.kind() == kind)
            .map(|driver| driver.disconnect_lanes(needle))
            .sum()
    }

    /// 标记 / 解除「事务中的批量会话」保护（按类型分发，见 [`Driver::hold_tx_lane`]）。
    pub fn hold_tx_lane(&self, kind: ConnectionKind, affinity: &str, hold: bool) -> usize {
        self.drivers
            .iter()
            .filter(|driver| driver.kind() == kind)
            .map(|driver| driver.hold_tx_lane(affinity, hold))
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
