//! 连接类型的**静态部分**：运行时模式、能力、traits 与描述结构。
//!
//! `ConnectionKind` 枚举本身由 `build.rs` 从 YAML 生成（见 `lib.rs` 的 `include!`），
//! 这里只放它依赖的支撑类型，保证「生成物只含数据，不含设计」。

/// 驱动运行方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    /// 内核内原生物理实现（最快，无外部进程）。
    Native,
    /// 外置驱动进程（JDBC/JVM 等），通过 stdio 协议通信。
    Agent,
    /// 第三方插件提供的进程外实现。
    External,
    /// 需要**专属**子进程托管的文件型引擎。
    ///
    /// 当前没有类型声明它：DuckDB 已归入 `agent`（其官方 JDBC 驱动可在通用宿主里
    /// 内嵌运行，没必要另造进程）。保留此变体是为了让「必须自带进程的引擎」
    /// （例如需要自定义扩展加载、进程内资源隔离的场景）仍有明确落点，
    /// 而不是被迫塞进 agent 语义里。
    File,
}

impl RuntimeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeMode::Native => "native",
            RuntimeMode::Agent => "agent",
            RuntimeMode::External => "external",
            RuntimeMode::File => "file",
        }
    }

    /// 是否需要在宿主机上准备外部进程（用于 UI 提示与就绪检查）。
    pub fn needs_external_process(self) -> bool {
        matches!(
            self,
            RuntimeMode::Agent | RuntimeMode::External | RuntimeMode::File
        )
    }
}

/// 连接协议：决定「一条语句长什么样」、以及由谁判定它是否只读。
///
/// 这是让安全闸门覆盖非 SQL 数据库的关键抽象 —— 闸门在内核，那么内核就必须
/// 懂该协议的语句语义；否则 Mongo 命令会落到「无法归类」，闸门形同虚设。
/// 新增协议（Redis / Elasticsearch）= 新增一个判定模块 + 一份 YAML。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProtocol {
    /// SQL 系（含各种方言）：语句是 SQL 文本。
    Sql,
    /// MongoDB：语句是 JSON 命令或 shell 风格调用（`db.x.find({})`）。
    Mongodb,
    /// Redis：语句是一条命令（`GET k` / `HSET h f v`），无语句分隔符。
    Redis,
    /// Elasticsearch：语句是一条 REST 请求（`GET /logs/_search`）。
    Elasticsearch,
}

impl RuntimeProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeProtocol::Sql => "sql",
            RuntimeProtocol::Mongodb => "mongodb",
            RuntimeProtocol::Redis => "redis",
            RuntimeProtocol::Elasticsearch => "elasticsearch",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "sql" => Some(RuntimeProtocol::Sql),
            "mongodb" => Some(RuntimeProtocol::Mongodb),
            "redis" => Some(RuntimeProtocol::Redis),
            "elasticsearch" => Some(RuntimeProtocol::Elasticsearch),
            _ => None,
        }
    }
}

/// MCP 通道对该类型的方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpMode {
    /// 通过桥接复用内核逻辑（当前实现方式）。
    Bridge,
    /// 该类型有专属 MCP 工具集。
    Native,
    /// 不对 MCP 暴露。
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeTraits {
    pub single_database: bool,
    pub schema_aware: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub query_execution: bool,
    pub metadata_browse: bool,
    pub object_browser: bool,
    pub object_source: bool,
    pub schema_search: bool,
    pub diagram: bool,
    pub table_data_edit: bool,
    pub table_structure_edit: bool,
    pub table_import: bool,
    pub data_transfer: bool,
    pub sql_file_execution: bool,
    pub database_create: bool,
    pub field_lineage: bool,
    pub sql_explain: bool,
    pub user_admin: bool,
    pub driver_management: bool,
}

impl Capabilities {
    /// 按 manifest/YAML 的 camelCase 键取值，便于壳层通用渲染能力矩阵。
    pub fn get(&self, key: &str) -> Option<bool> {
        Some(match key {
            "queryExecution" => self.query_execution,
            "metadataBrowse" => self.metadata_browse,
            "objectBrowser" => self.object_browser,
            "objectSource" => self.object_source,
            "schemaSearch" => self.schema_search,
            "diagram" => self.diagram,
            "tableDataEdit" => self.table_data_edit,
            "tableStructureEdit" => self.table_structure_edit,
            "tableImport" => self.table_import,
            "dataTransfer" => self.data_transfer,
            "sqlFileExecution" => self.sql_file_execution,
            "databaseCreate" => self.database_create,
            "fieldLineage" => self.field_lineage,
            "sqlExplain" => self.sql_explain,
            "userAdmin" => self.user_admin,
            "driverManagement" => self.driver_management,
            _ => return None,
        })
    }

    pub const KEYS: &'static [&'static str] = &[
        "queryExecution",
        "metadataBrowse",
        "objectBrowser",
        "objectSource",
        "schemaSearch",
        "diagram",
        "tableDataEdit",
        "tableStructureEdit",
        "tableImport",
        "dataTransfer",
        "sqlFileExecution",
        "databaseCreate",
        "fieldLineage",
        "sqlExplain",
        "userAdmin",
        "driverManagement",
    ];
}

/// 连接类型的完整描述，是对外（CLI/Web/前端）暴露的类型目录条目。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeDescriptor {
    pub kind: crate::ConnectionKind,
    pub key: &'static str,
    pub label: &'static str,
    pub dialect: &'static str,
    pub runtime_mode: RuntimeMode,
    /// 语句协议（决定安全闸门用哪套判定规则）。
    pub protocol: RuntimeProtocol,
    pub mcp_mode: McpMode,
    pub agent_key: Option<&'static str>,
    pub default_port: Option<u16>,
    pub local_file: bool,
    pub skip_tcp_probe: bool,
    /// 该类型只允许一条物理连接（`singleConnectionPool`）。
    ///
    /// 与「只读」不同：这是**连接级作用域**的表达 —— SQLite 的 `:memory:` 库、
    /// 各引擎的临时表都是连接级的，新开一条连接等于换了个空世界。
    pub single_connection_pool: bool,
    /// 元数据必须与数据会话**同作用域**（`metadataConnectionScoped`），
    /// 例如 SQL Server 的 `#temp` 表是会话级的。
    pub metadata_connection_scoped: bool,
    /// 并发会话上限（`maxConnections`，见 `ConnectionKind::max_connections`）。
    ///
    /// 声明了 `singleConnectionPool` 的类型恒为 1：那条声明就是「只允许一条物理连接」，
    /// 于是并发请求只能排队。其余类型取 YAML 声明值（省略则 4）。
    pub max_connections: usize,
    pub driver_store_order: i32,
    pub support_level: &'static str,
    pub traits: TypeTraits,
    pub capabilities: Capabilities,
    /// 当前内核是否已实现该类型的执行路径（声明了就位 ≠ 驱动已就绪）。
    pub implemented: bool,
}
