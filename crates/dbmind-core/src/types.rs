//! 内核的对外数据模型。壳层（CLI/Web/MCP/桌面）全部复用这一套结构，
//! 因此字段变更只有一个地方要改。

use crate::error::{DbMindError, ErrorCode, Result};
use crate::sql::StatementKind;
use crate::ConnectionKind;
use serde::{Deserialize, Serialize};

// ------------------------------------------------------------------ 单元格

/// 单元格值。
///
/// Blob 只带长度不带内容：结果集要跨 HTTP/IPC 传输，带上原始二进制会把
/// 响应体撑爆；需要内容时由上层按需再取（`objectSource` 能力）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "snake_case")]
pub enum CellValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob { len: usize },
}

impl CellValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            CellValue::Null => "null",
            CellValue::Integer(_) => "integer",
            CellValue::Real(_) => "real",
            CellValue::Text(_) => "text",
            CellValue::Blob { .. } => "blob",
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, CellValue::Null)
    }

    /// 面向文本输出的渲染（CLI 表格、导出、复制）。
    pub fn to_display(&self) -> String {
        match self {
            CellValue::Null => "NULL".to_string(),
            CellValue::Integer(v) => v.to_string(),
            CellValue::Real(v) => v.to_string(),
            CellValue::Text(v) => v.clone(),
            CellValue::Blob { len } => format!("<blob {len}B>"),
        }
    }
}

impl std::fmt::Display for CellValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_display())
    }
}

// ------------------------------------------------------------------ 连接

/// 连接配置（不含 id / 时间戳，纯用户可编辑部分）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub name: String,
    pub kind: ConnectionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// 口令**不外发**：只进内核与磁盘，任何 JSON 响应都不包含它。
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    /// 文件型数据库的路径（SQLite / DuckDB / H2 等）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

impl ConnectionConfig {
    /// 连接级并发上限覆盖（存在 `extra.maxConnections`）。
    ///
    /// 为什么放在 `extra` 而不是新加一列：`connections` 表只有
    /// `CREATE TABLE IF NOT EXISTS`、**没有迁移机制**，加列在老库上不会生效；
    /// 而 `extra` 本来就是为「可选的前向兼容配置」留的袋子。
    /// 类型化只发生在内核边界：读进来立刻收窄成 `Option<usize>`，写出去仍是普通 JSON。
    pub fn max_connections_override(&self) -> Option<usize> {
        self.extra
            .as_ref()
            .and_then(|extra| extra.get("maxConnections"))
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value >= 1)
    }

    /// 设置连接级并发上限覆盖（`None` 表示回到类型默认）。
    pub fn with_max_connections(mut self, max_connections: Option<usize>) -> Self {
        let mut extra = match self.extra.take() {
            Some(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };
        match max_connections {
            Some(value) => {
                extra.insert("maxConnections".to_string(), serde_json::json!(value));
            }
            None => {
                extra.remove("maxConnections");
            }
        }
        self.extra = if extra.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(extra))
        };
        self
    }

    pub fn new(name: impl Into<String>, kind: ConnectionKind) -> Self {
        Self {
            name: name.into(),
            kind,
            host: None,
            port: None,
            database: None,
            username: None,
            password: None,
            file_path: None,
            color: None,
            extra: None,
        }
    }

    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    pub fn with_database(mut self, database: impl Into<String>) -> Self {
        self.database = Some(database.into());
        self
    }

    pub fn with_credentials(mut self, user: impl Into<String>, password: impl Into<String>) -> Self {
        self.username = Some(user.into());
        self.password = Some(password.into());
        self
    }

    pub fn with_file(mut self, path: impl Into<String>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    /// 端口：显式给出优先，否则取类型默认值。
    pub fn resolved_port(&self) -> Option<u16> {
        self.port.or_else(|| self.kind.default_port())
    }

    /// 文件型数据库的落盘路径。
    pub fn resolved_file(&self) -> Option<&str> {
        self.file_path.as_deref().or(self.database.as_deref())
    }

    /// 配置合法性校验。**在写库之前**调用，保证存储里不存在半残配置。
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(DbMindError::new(ErrorCode::ConnInvalid, "连接名称不能为空"));
        }
        if self.host.as_deref().is_some_and(|h| h.trim().is_empty()) {
            return Err(DbMindError::new(ErrorCode::ConnInvalid, "主机名不能是空白"));
        }
        if self.kind.requires_file() {
            match self.resolved_file() {
                Some(p) if !p.trim().is_empty() => {}
                _ => {
                    return Err(DbMindError::new(
                        ErrorCode::ConnInvalid,
                        format!("{} 是文件型数据库，必须指定文件路径", self.kind.label()),
                    ))
                }
            }
        } else if self.unsupported_host_requirement() {
            return Err(DbMindError::new(
                ErrorCode::ConnInvalid,
                format!("{} 需要主机名", self.kind.label()),
            ));
        }
        if self.kind.runtime_mode() == crate::RuntimeMode::Agent && self.kind.agent_key().is_none() {
            return Err(DbMindError::new(
                ErrorCode::ConnInvalid,
                format!("{} 声明了 agent 运行时但缺少 agentKey", self.kind.label()),
            ));
        }
        Ok(())
    }

    fn unsupported_host_requirement(&self) -> bool {
        !self.kind.local_file() && self.host.is_none()
    }
}

/// 落库后的连接记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionRecord {
    pub id: String,
    #[serde(flatten)]
    pub config: ConnectionConfig,
    pub read_only: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// 「影子连接」标记（存在连接记录的 `extra` 里）：值是**主连接 id**。
///
/// 影子连接是跨库浏览按需生成的记录 —— 同配置、只把库名换掉，让任何类型都能跨库
/// （见 web 层的 `scope`）。用户看不到它（`/api/connections` 会滤掉），
/// 但它是一条**真实的独立连接**，于是它也有自己的 `read_only` 字段。
///
/// 用 `extra` 而不是新加列：`connections` 表只有 `CREATE TABLE IF NOT EXISTS`、
/// **没有迁移机制**，加列在老库上不会生效（同 `maxConnections` 的理由）。
pub const EXTRA_SHADOW_FOR: &str = "shadowFor";

/// 连接的**目录/分组**（存在 `extra` 里）：树按它分组，自由文本（含预置的 `PROD` 等）。
///
/// 字段名是 `group`（中文界面叫「分组」）—— 曾经叫 `environment`，与「环境标识」
/// `env`（[`EXTRA_ENV_BADGE`]）太容易混淆。旧数据里的 `environment` 键在读取侧**仍然认**
/// （见 `ConnectionRecord::group`），保存时写入新键，老库无需迁移。
pub const EXTRA_GROUP: &str = "group";
/// 旧版分组键：只读不写，保存连接时自然消失。
pub const EXTRA_GROUP_LEGACY: &str = "environment";

/// 连接的**环境标识角标**（存在 `extra` 里）：`DEV` / `TEST` / `PROD` / `STAGING` / `UAT`。
///
/// ⚠️ 与 [`EXTRA_GROUP`] 是**两个不同的字段**：分组决定连接挂在树的哪个目录下，
/// 角标是那枚彩色小徽章（红=生产）。生产保护闸门**两个都认** ——
/// 以前只查目录，用户把角标设成生产、目录还在「本地分组」，保护就完全没生效（真机踩过）。
pub const EXTRA_ENV_BADGE: &str = "env";

impl ConnectionRecord {
    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub fn kind(&self) -> ConnectionKind {
        self.config.kind
    }

    /// 这条记录是影子的话，返回它所属的**主连接 id**。
    pub fn shadow_of(&self) -> Option<&str> {
        self.config
            .extra
            .as_ref()?
            .get(EXTRA_SHADOW_FOR)?
            .as_str()
    }

    /// 是不是影子记录（用户看不到的那些按需生成的记录）。
    pub fn is_shadow(&self) -> bool {
        self.shadow_of().is_some()
    }

    /// 连接的**分组**（树上的目录）：新键 `group`，旧数据回落到 `environment`。
    pub fn group(&self) -> Option<String> {
        let extra = self.config.extra.as_ref()?;
        extra
            .get(EXTRA_GROUP)
            .or_else(|| extra.get(EXTRA_GROUP_LEGACY))
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    /// 这条数据源有没有被标注为**生产环境**。
    ///
    /// 三处标注**任一**为生产即算：环境标识角标（`extra.env`，树上的红徽章）、
    /// 分组（`extra.group`，预置的 `PROD` 目录）、旧版分组键（`environment`）。
    /// 大小写不敏感，也认手输的「生产」字样。生产保护闸门的作用域就靠它划定；
    /// 影子连接继承主连接的 `extra`，跨库浏览到的生产库同样受保护。
    pub fn is_prod_environment(&self) -> bool {
        let is_prod = |s: &str| s.eq_ignore_ascii_case("PROD") || s == "生产";
        [EXTRA_ENV_BADGE, EXTRA_GROUP, EXTRA_GROUP_LEGACY].iter().any(|key| {
            self.config
                .extra
                .as_ref()
                .and_then(|v| v.get(*key))
                .and_then(|v| v.as_str())
                .map(|s| is_prod(s))
                .unwrap_or(false)
        })
    }
}

/// 连接自检结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectReport {
    pub connection_id: String,
    pub kind: ConnectionKind,
    pub runtime_mode: crate::RuntimeMode,
    pub latency_ms: u64,
    pub server_version: Option<String>,
    pub message: String,
}

// ------------------------------------------------------------------ 结构

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableKind {
    Table,
    View,
}

impl TableKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TableKind::Table => "table",
            TableKind::View => "view",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub name: String,
    pub kind: TableKind,
    pub row_estimate: Option<i64>,
}

/// 结构缓存的概览（界面据此显示「缓存于 N 秒前」）。
///
/// 为什么必须让界面看到：缓存意味着**可能不是实时结构**，
/// 把这件事藏起来就是骗人 —— 用户会以为「没看到那张新表」是权限或连接问题。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaCacheInfo {
    /// 缓存条目数（0 = 没有缓存，下次浏览会打数据库）
    pub entries: usize,
    /// 最近一次写缓存的时间（RFC3339）；无缓存时为 None
    pub latest_cached_at: Option<String>,
    /// 距现在多少秒；时间解析失败时为 None
    pub age_secs: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDetail {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    pub nullable: bool,
    pub primary_key: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    /// 是否自增 / identity / serial。
    ///
    /// 为什么要有这个位：跨类型同步建表时，自增信息**必须跟着列走**，
    /// 否则 MySQL `auto_increment`、PG `serial`、SQL Server `identity` 建到目标库全都会退化成普通列
    /// （数据本身还能同步 —— 源的主键值是显式写过来的 —— 但目标表失去自增能力）。
    /// 各家元数据都能便宜地拿到它（见 `meta::enrich_column_types` 的 `is_auto`）。
    #[serde(default)]
    pub auto_increment: bool,
}

// ------------------------------------------------------------------ 查询

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryOptions {
    /// 单页最大行数：超过即截断并置 `truncated`，避免把内存与前端拖死。
    pub max_rows: usize,
    /// 超时（毫秒）。
    pub timeout_ms: u64,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            max_rows: 2000,
            timeout_ms: 30_000,
        }
    }
}

impl QueryOptions {
    pub fn with_max_rows(mut self, max_rows: usize) -> Self {
        self.max_rows = max_rows;
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub const HARD_MAX_ROWS: usize = 100_000;

    /// 夹紧到内核允许的区间（防止壳层绕过限制）。
    pub fn clamp(mut self) -> Self {
        self.max_rows = self.max_rows.clamp(1, Self::HARD_MAX_ROWS);
        self.timeout_ms = self.timeout_ms.clamp(1_000, 600_000);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRequest {
    /// 连接 id 或名称。
    pub connection: String,
    pub sql: String,
    #[serde(default)]
    pub options: QueryOptions,
    /// 由调用方提供的执行 id（Web 壳用它做取消）；缺省则内核生成。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    /// **会话亲和键**：同一个键的语句落在**同一条物理会话**上。
    ///
    /// 界面每个页面给一个自己的键 ⇒ 每个标签页一个数据库会话：`SET`、临时表、
    /// （将来的）事务都只属于那个页面，不会串到别的页面，也不会被别的页面的
    /// 并发请求挤到另一条连接上。缺省（CLI / MCP / 未传）沿用「按连接复用」的老行为。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// 显式指定这条语句用**只读会话**执行；缺省按连接策略算（`policy.is_read_only`）。
    ///
    /// 为什么需要它：结构浏览那条路（`metadata_session_key`）对非 connection-scoped 的类型
    /// **强制**用只读会话（`session_key(config, true)`），而 `execute` 走的是连接策略标记。
    /// 两者一旦不同就是**两条物理会话** —— 实测一次 `/tables` 建了两条连接（元数据一条、
    /// 随后的行数估算一条，各 ~300ms），虽然同库同连接、做的还是同一件事。
    ///
    /// 内部浏览类语句（全是只读 SELECT）填 `Some(true)`，即可与结构浏览共用一条会话。
    /// 缺省 `None` = 行为与以前完全一致，用户自己的查询不受影响。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    /// **程序自己发的调用**（结构浏览、元数据探测、表格预览分页、界面功能驱动的 DDL/DML、
    /// 智能体的中间查询）标记它 ⇒ 内核**不写查询历史**。
    ///
    /// 历史是「**用户执行过什么**」的账本（首页的「最近查询 / 查询统计」就是受害者）：元数据
    /// 与浏览语句混进来会把真实操作淹掉 —— 实机上表现为「最近查询里全是 `show databases`
    /// 和取表选项的 `select table_name …`，看不到我敲过的 SQL」。
    ///
    /// 为什么单开一个字段、而不是复用会话键前缀（`INTERNAL_SESSION_PREFIX`）：
    /// **亲和键会参与会话键的计算**，给这类语句打上 `internal:` 前缀会让它们落到另一条物理会话上
    /// （实测一次 `/tables` 因此建了两条连接、各 ~300ms，见 `meta::run_sql_in` 的注释）。
    /// 「记不记历史」与「走哪条连接」是两件事，不该互相绑。
    ///
    /// 判据由调用方**显式声明**，不去猜 SQL 文本长什么样：猜错的两个方向都很糟 ——
    /// 漏标 = 真实操作被淹没；误标 = 用户敲过的语句查不到。
    #[serde(default)]
    pub internal: bool,
}

impl QueryRequest {
    pub fn new(connection: impl Into<String>, sql: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
            sql: sql.into(),
            options: QueryOptions::default(),
            execution_id: None,
            session: None,
            read_only: None,
            internal: false,
        }
    }

    /// 标记这是**程序自己发的调用** ⇒ 不进查询历史（见 `internal` 字段的说明）。
    pub fn with_internal(mut self, internal: bool) -> Self {
        self.internal = internal;
        self
    }

    /// 绑定会话亲和（界面每个页面一个键 ⇒ 每个标签页一条数据库会话）。
    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    pub fn with_execution_id(mut self, id: impl Into<String>) -> Self {
        self.execution_id = Some(id.into());
        self
    }

    pub fn with_options(mut self, options: QueryOptions) -> Self {
        self.options = options;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub execution_id: String,
    pub connection_id: String,
    pub connection_name: String,
    /// 语句类型（读/写/DDL），前端据此决定是否提示「已修改 N 行」。
    pub statement_kind: StatementKind,
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<CellValue>>,
    pub row_count: usize,
    /// 写语句影响行数。
    pub affected_rows: Option<usize>,
    pub truncated: bool,
    pub duration_ms: u64,
    pub notices: Vec<String>,
    /// 这条语句跑在**哪条物理会话**上（内核命名的会话 id）。
    ///
    /// 两个用途：出问题时能回答「它到底连到哪儿去了」；界面把会话显示出来之后，
    /// 「每个标签页一条会话」才是**看得见**的事实，而不是一句设计说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 这条读语句的结果**来自哪个对象**（表）；判定不出为 None。
    ///
    /// 存在意义：界面要回答「能不能就地改这一行」。只有当结果确实是某张表的原始行
    /// （单表、无 JOIN/UNION、无聚合、列名未被别名改写）才给得出来 ——
    /// 猜错会让「改一行」变成改错行，所以宁可给 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_object: Option<String>,
    /// 结果里能**唯一定位一行**的列（就地编辑用）；判定不出来为 None。
    ///
    /// 与 `source_object` 是两个独立条件：知道「这份结果来自哪个对象」
    /// 不等于知道「这一行是谁」。
    /// - SQL：这里不给 —— 主键来自结构元数据（`list_columns` 的 `primary_key`），那里才权威；
    /// - Mongo：由**宿主**给 —— `_id` 的真实 BSON 类型只有宿主知道；
    /// - ES：由内核推（`statement::derived_row_identity`）—— 命中里的 `_id` 天生是字符串。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_identity: Option<RowIdentity>,
}

/// 「这一行是谁」：列名 + 比较时该按什么类型处理。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowIdentity {
    pub column: String,
    pub value_type: IdentityValueType,
}

/// 行标识值的类型。
///
/// **存在的唯一理由**：MongoDB 的 ObjectId 与「恰好长得一样的字符串」在结果里
/// 是一模一样的十六进制串，写回时按错类型比较会**匹配不到任何行** ——
/// 表现出来是「保存成功、0 行受影响」，比直接报错更糟，因为它看着像成功了。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IdentityValueType {
    /// MongoDB ObjectId（写回时是 `ObjectId("…")`）
    ObjectId,
    /// 字符串（Mongo 的字符串 `_id`、ES 的 `_id`）
    String,
    /// 整数（Mongo 允许用数字当 `_id`）
    Integer,
}

impl QueryResult {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

// ------------------------------------------------------------------ 历史

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Ok,
    Error,
    Canceled,
}

impl HistoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            HistoryStatus::Ok => "ok",
            HistoryStatus::Error => "error",
            HistoryStatus::Canceled => "canceled",
        }
    }

    /// 从存储中的字符串还原。
    ///
    /// 刻意不叫 `from_str`：那会与 `std::str::FromStr` 混淆，读者会以为它能失败，
    /// 而这里的语义是「无法识别一律当失败」。
    pub fn parse(s: &str) -> Self {
        match s {
            "ok" => HistoryStatus::Ok,
            "canceled" => HistoryStatus::Canceled,
            _ => HistoryStatus::Error,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub sql: String,
    pub status: HistoryStatus,
    pub row_count: usize,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub created_at: String,
}

/// 写历史用的入参（id 与时间戳由存储层生成）。
#[derive(Debug, Clone)]
pub struct NewHistoryEntry {
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub sql: String,
    pub status: HistoryStatus,
    pub row_count: usize,
    pub duration_ms: u64,
    pub error_code: Option<String>,
    /// 操作分类：query / write / ddl / tx / exec（由 SQL 首词推导，见 engine::classify_kind）
    pub kind: &'static str,
    /// 调用来源（desktop / web / cli / mcp / ai / system）
    pub source: &'static str,
}

/// 统一日志条目：合并 query_history 与 ai_audit 两个来源，供日志管理界面展示。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// 分类：query / write / ddl / tx / exec / ai
    pub kind: String,
    /// 语句或 AI 提示词（截断后）
    pub sql: String,
    pub connection: String,
    pub status: String,
    pub row_count: u64,
    pub duration_ms: u64,
    pub error_code: Option<String>,
    pub created_at: String,
}

// ------------------------------------------------------------------ 调用来源

/// 调用来源。安全策略按来源分级：AI/MCP 默认只读，且**壳层开关不能绕过**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessContext {
    Desktop,
    Web,
    Cli,
    Mcp,
    Ai,
}

impl AccessContext {
    pub fn as_str(self) -> &'static str {
        match self {
            AccessContext::Desktop => "desktop",
            AccessContext::Web => "web",
            AccessContext::Cli => "cli",
            AccessContext::Mcp => "mcp",
            AccessContext::Ai => "ai",
        }
    }

    /// 是否属于「机器/AI 通道」——这些通道默认不允许写。
    pub fn is_ai_like(self) -> bool {
        matches!(self, AccessContext::Mcp | AccessContext::Ai)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 文件型连接必须有路径() {
        let cfg = ConnectionConfig::new("local", ConnectionKind::Sqlite);
        let err = cfg.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::ConnInvalid);

        let ok = cfg.clone().with_file("./demo.db");
        ok.validate().unwrap();
    }

    #[test]
    fn 网络型连接必须有主机() {
        let cfg = ConnectionConfig::new("pg", ConnectionKind::Postgresql);
        assert_eq!(cfg.validate().unwrap_err().code, ErrorCode::ConnInvalid);
        cfg.with_host("127.0.0.1").validate().unwrap();
    }

    #[test]
    fn 端口回落到类型默认值() {
        let cfg = ConnectionConfig::new("pg", ConnectionKind::Postgresql).with_host("h");
        assert_eq!(cfg.resolved_port(), ConnectionKind::Postgresql.default_port());
        assert_eq!(cfg.clone().with_port(6543).resolved_port(), Some(6543));
    }

    #[test]
    fn 选项会被夹紧到内核允许区间() {
        let clamped = QueryOptions {
            max_rows: 0,
            timeout_ms: 10,
        }
        .clamp();
        assert_eq!(clamped.max_rows, 1);
        assert_eq!(clamped.timeout_ms, 1_000);

        let clamped = QueryOptions {
            max_rows: usize::MAX,
            timeout_ms: u64::MAX,
        }
        .clamp();
        assert_eq!(clamped.max_rows, QueryOptions::HARD_MAX_ROWS);
        assert_eq!(clamped.timeout_ms, 600_000);
    }

    #[test]
    fn 口令不随序列化外泄() {
        let cfg = ConnectionConfig::new("pg", ConnectionKind::Postgresql)
            .with_host("h")
            .with_credentials("u", "secret");
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("secret"), "口令不应出现在 JSON 里：{json}");
        // 但反序列化仍可接收（导入配置时用）
        let back: ConnectionConfig = serde_json::from_str(&json).unwrap();
        assert!(back.password.is_none());
    }
}
