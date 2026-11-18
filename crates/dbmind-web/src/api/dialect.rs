//! 方言层：**按数据库类型生成元数据 SQL**。
//!
//! 为什么这一层存在：DBmind 内核只提供两个元数据能力（`list_tables` / `list_columns`，
//! 走 JDBC `DatabaseMetaData`），而上游的界面要的远不止这些 —— 索引、存储过程、
//! 触发器、事件、用户、建表语句、库/schema 清单，全都要。内核没有这些接口，
//! 短期内的正确做法就是**在兼容层按方言拼 SQL、复用内核的通用执行能力**。
//!
//! 三档语义，必须分清（这也是这个枚举存在的唯一理由）：
//!
//! - `Sql`       —— 这个类型有这个概念，语句在这儿；
//! - `Absent`    —— 这个类型**确实没有**这个概念（SQLite 没有存储过程、Redis 没有索引）
//!                  ⇒ 界面上返回空数组，那是**事实**；
//! - `Unwritten` —— 概念存在，**我们还没写**（PostgreSQL 的建表语句要自拼约束与索引）
//!                  ⇒ 必须报错，绝不能让用户以为是「库里没有」。
//!
//! 把后两者混成一个 `None` 就等于「我不知道」伪装成「这里没有」，那是两种完全不同的坏消息。

use dbmind_core::{ColumnDetail, ConnectionKind};

/// 元数据查询的三档结果。
/// 这个方言的 **DDL 能不能回滚**（能不能把一批变更包进一个事务）。
///
/// 能回滚的：PostgreSQL / KingbaseES / SQLite / SQL Server / DB2 / Derby / H2 —— 它们的事务
/// 覆盖 DDL，中途失败可以整体 ROLLBACK。
///
/// **不能**回滚的：MySQL / MariaDB / Doris / ClickHouse（DDL 会隐式提交）、Oracle / DM
/// （DDL 前后自动提交）—— 事务对它们没有意义。这类方言的原子性靠**前端把整批变更合并成
/// 一条 ALTER**（一条语句里的多个子句是原子的，见 `TableDetailView.vue` 的
/// `mergedAlterStatement`），以及索引修改一律「先建后删 → 改名」（见那边的 `usesTempNameIndexSwap`）。
pub fn ddl_transactional(key: &str) -> bool {
    // 只列**真的有 `BEGIN` 语句**、且 DDL 参与事务的方言。
    //
    // 为什么把 db2 / derby / h2 从这份名单里去掉了：它们没有 `BEGIN` 这个语句（要靠 JDBC 的
    // autoCommit 开关），原来放在这里等于"发了句没人认的 BEGIN"⇒ autoCommit 仍是开着的、
    // 每条 DDL 自己提交 ⇒ 我们却对外宣称"失败整体回滚"——比不做更坏。要做对得走内核那套
    // 事务入口（Driver::tx_control / `setautocommit=false`），那是独立一件事，先如实摘掉。
    matches!(key, "postgresql" | "kingbase" | "opengauss" | "sqlite" | "sqlserver")
}

/// 开事务的语句。T-SQL 里单独的 `BEGIN` 是语句块开始（不是开事务），必须写全。
pub fn tx_begin(key: &str) -> &'static str {
    if key == "sqlserver" {
        "BEGIN TRANSACTION"
    } else {
        "BEGIN"
    }
}

pub enum Meta {
    Sql(String),
    Absent,
    Unwritten,
}

impl Meta {
    pub fn sql(self) -> Option<String> {
        match self {
            Meta::Sql(sql) => Some(sql),
            _ => None,
        }
    }
}

///上游前端的类型码（大写）→ 数据源模块路径段。
///
/// 与 `frontend/src/api/index.js` 里的 `TYPE_TO_MODULE` 必须**逐条一致**：
/// 前端就是拿它拼 `/api/{module}/{id}/…` 的，少一条那个类型的所有操作都会 404。
pub fn module_of_type_code(code: &str) -> Option<&'static str> {
    // 不再手写：EXPOSED_TYPE_CODES 由 build.rs 从 YAML 生成。手写 16 项漏掉 TIDB
    // 曾让「新建数据源」里 TiDB 整个消失（YAML/manifest/生成物三处都有它）。
    let up = code.to_ascii_uppercase();
    dbmind_core::EXPOSED_TYPE_CODES
        .iter()
        .find(|(c, _)| *c == up)
        .map(|(_, m)| *m)
}

/// 关系型 / NoSQL 模块路径段，路由按它们注册 —— **从 build.rs 生成物派生**
/// （plugins YAML 的 dialect/protocol 去重），不再手写。
/// 教训：手写 9 项在 DuckDB 暴露后没跟着加，`/api/duckdb/*` 整条 404
///（exe 里模块字符串都在，就路由清单缺一个 —— 真机接入时实测踩到）。
pub const RELATIONAL_MODULES: &[&str] = dbmind_core::RELATIONAL_MODULES;

pub const NOSQL_MODULES: &[&str] = dbmind_core::NOSQL_MODULES;

/// 前端类型注册表里存在的类型码。
///
/// **从生成表派生**（EXPOSED_TYPE_CODES 由 build.rs 按 YAML 的 exposed 过滤生成）。
/// 不再手写 —— 手写 16 项漏掉 TIDB 导致
/// 「新建数据源」里 TiDB 整个消失（实测）。前端没有的类型，后端就不该通告 ——
/// 这是「单一真源」该有的样子。
pub fn exposed_type_codes() -> Vec<&'static str> {
    dbmind_core::EXPOSED_TYPE_CODES
        .iter()
        .map(|(c, _)| *c)
        .collect()
}

/// H2 2.x 的表级权限：只有这五种，`ALL` 是前四项的简写（`GRANT ALL ON t TO u` 合法）。
/// 名称会被拼进 SQL，所以必须过白名单 —— 和 MySQL/PG 那边同一套规矩。
pub fn is_h2_table_priv(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_uppercase().as_str(),
        "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "ALL"
    )
}

/// H2 的默认用户是**空用户名**：清单里显示成 H2 的空标识符 `""` —— 既不是一行点不动的
/// 空白，又是能直接贴进 SQL 的真写法（`alter user "" set password …`）。用户要求名字
/// 显示必须带引号，不要显示成 `(default)` 那种"没括号"的假名字。
/// 查详情 / 改密 / 删除时再还原成空串（旧的 `(default)` 仍认，兼容老前端缓存的清单）。
pub(crate) fn h2_user(name: &str) -> String {
    let name = name.trim();
    if name == "\"\"" || name == "(default)" {
        String::new()
    } else {
        name.to_string()
    }
}

/// `ConnectionKind` 是 `Copy`，所以方言本身也能按值传来传去 ——
/// 导出/备份那几条链路上要把它带进写入器（生成字面量、拼分页），按引用传会牵出一堆生命周期。
#[derive(Clone, Copy)]
pub struct Dialect {
    pub kind: ConnectionKind,
}

/// 同实例直拷时，源表引用要限定到几层。
///
/// 直拷的语句是 `insert into 目标(列) select 列 from 源`，**在目标会话上执行** ——
/// 源表名不带库/模式限定就会被解析成目标库里的同名表（真机：同实例双库同步读到
/// 刚建好的空表，回报「直拷完成」却只搬了 0 行）。各家的限定层级并不一样，所以
/// 由方言说清楚，而不是在同步层里按库名分支。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectRefDepth {
    /// 三段名 `库.模式.表`（SQL Server：两段名是「模式.表」，跨库必须写三段）
    DatabaseSchema,
    /// 两段名 `库.表`（MySQL 系 / ClickHouse / Doris）
    Database,
    /// 两段名 `模式.表`（PG / Oracle / DB2 / 达梦 / Kingbase —— 没有跨库引用这回事）
    Schema,
    /// 只有一张库，裸表名（嵌入式 H2 / Derby / SQLite / DuckDB）
    None,
}

/// 用户表单提交的三层权限（MySQL 语境）。
///
/// `databases` / `tables` 里的名字都是**授权目标**（要授给这个用户的库/表），
/// 不是「用户拥有的库表」—— 用完整的 `(目标, 权限)` 结构，避免调用方把两者搞混。
/// 一条「要撤销的授权」。只有**没有"一把全撤"语句**的方言才用得上：
/// Oracle 没有 `revoke all privileges from u`，撤销必须逐条点名，所以由界面把
/// "原来有、现在没有了"的那些算好传进来（见 UserForm 的 revoke 计算）。
#[derive(Clone, Debug, Default)]
pub struct GrantRevoke {
    /// `global`（系统权限）| `role`（角色）| `object`（对象权限）
    pub scope: String,
    pub privilege: String,
    pub owner: String,
    pub object: String,
    /// SQL Server 专用：schema/object 级撤销的**目标库**（行级选库）。空 = 不切库。
    pub db: String,
    /// MySQL 专用：routine 级撤销的例程类型（PROCEDURE / FUNCTION）——
    /// `revoke execute on procedure ...` 里类型是语法的一部分，缺了就拼不出合法语句。
    pub kind: String,
}

/// 用户属性：编辑表单「基本信息」里那批**能改的项**（配额 / 账户锁定 / 表空间 / 开关）。
///
/// **只带改动过的**：`None` = 这一项不动。各家支持的子集不同（见 `user_action_sql`
/// 各分支），传了本方言不支持的项会被忽略、不会拼进 SQL —— 前端按方言决定摆哪些控件，
/// 这里是第二道闸。
#[derive(Debug, Default, Clone)]
pub struct UserAttrs {
    // MySQL 系
    pub account_locked: Option<bool>,
    pub password_expired: Option<bool>,
    pub max_queries_per_hour: Option<i64>,
    pub max_updates_per_hour: Option<i64>,
    pub max_connections_per_hour: Option<i64>,
    pub max_user_connections: Option<i64>,
    pub auth_plugin: Option<String>,
    // Oracle / DM
    pub default_tablespace: Option<String>,
    pub temporary_tablespace: Option<String>,
    pub profile: Option<String>,
    // SQL Server
    pub disabled: Option<bool>,
    pub default_schema: Option<String>,
    /// 登录名的默认数据库（`alter login … with default_database = …`）。
    /// 与 `default_schema` 是两件事：后者是库内用户的属性，登录名没有。
    pub default_database: Option<String>,
    pub login_name: Option<String>,
    // PostgreSQL / Kingbase
    pub can_login: Option<bool>,
    pub is_superuser: Option<bool>,
    pub can_create_db: Option<bool>,
}

/// MySQL 的认证插件白名单：插件名会**原样拼进 SQL**（`identified with <插件>`），
/// 所以只认名单内的 —— 与权限名同一个道理。
pub fn is_mysql_auth_plugin(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "mysql_native_password" | "caching_sha2_password" | "sha256_password" | "unix_socket"
    )
}

/// Oracle 系统权限白名单（`grant <这些> to u`）。权限名会被**拼进 SQL**，
/// 所以只放行清单里的 —— 界面传别的名字一律拒绝，不拼进语句。
pub const ORACLE_SYSTEM_PRIVS: [&str; 30] = [
    "CREATE SESSION",
    "CREATE TABLE",
    "CREATE ANY TABLE",
    "ALTER ANY TABLE",
    "DROP ANY TABLE",
    "SELECT ANY TABLE",
    "INSERT ANY TABLE",
    "UPDATE ANY TABLE",
    "DELETE ANY TABLE",
    "CREATE VIEW",
    "CREATE ANY VIEW",
    "CREATE SEQUENCE",
    "CREATE PROCEDURE",
    "CREATE ANY PROCEDURE",
    "CREATE TRIGGER",
    "CREATE ANY TRIGGER",
    "CREATE TYPE",
    "CREATE SYNONYM",
    "CREATE ANY SYNONYM",
    "CREATE DATABASE LINK",
    "CREATE JOB",
    "CREATE MATERIALIZED VIEW",
    "CREATE ROLE",
    "CREATE USER",
    "ALTER USER",
    "DROP USER",
    "ALTER SESSION",
    "UNLIMITED TABLESPACE",
    "DEBUG CONNECT SESSION",
    "QUERY REWRITE",
];

/// Oracle 角色白名单（角色名同样是拼进 SQL 的，必须过白名单）。
pub const ORACLE_ROLES: [&str; 12] = [
    "CONNECT",
    "RESOURCE",
    "DBA",
    "EXP_FULL_DATABASE",
    "IMP_FULL_DATABASE",
    "SELECT_CATALOG_ROLE",
    "EXECUTE_CATALOG_ROLE",
    "DELETE_CATALOG_ROLE",
    "RECOVERY_CATALOG_OWNER",
    "SCHEDULER_ADMIN",
    "OEM_MONITOR",
    "AQ_ADMINISTRATOR_ROLE",
];

/// Oracle 对象权限白名单（`grant <这些> on <owner>.<object> to u`）。
pub const ORACLE_OBJECT_PRIVS: [&str; 13] = [
    "SELECT",
    "INSERT",
    "UPDATE",
    "DELETE",
    "ALTER",
    "INDEX",
    "REFERENCES",
    "READ",
    "EXECUTE",
    "DEBUG",
    "FLASHBACK",
    "ON COMMIT REFRESH",
    "QUERY REWRITE",
];

/// SQL Server 数据库级 / 对象级权限白名单（`grant <这些> on …`）。
/// 权限名会**原样拼进 SQL**，所以只放行清单里的。
pub const MSSQL_PRIVILEGES: [&str; 10] = [
    "SELECT",
    "INSERT",
    "UPDATE",
    "DELETE",
    "EXECUTE",
    "REFERENCES",
    "CONTROL",
    "ALTER",
    "VIEW DEFINITION",
    "TAKE OWNERSHIP",
];

pub fn is_mssql_privilege(name: &str) -> bool {
    in_list(&MSSQL_PRIVILEGES, name) || is_safe_name(name)
}

/// PostgreSQL/KingbaseES 授权白名单（这些名字会被**原样拼进** GRANT/REVOKE）。
/// 模式级只有 USAGE / CREATE；表级七项（PG 标准表权限）；例程级只放 EXECUTE
/// （PG 的 `WITH GRANT OPTION` 是子句不是权限名，界面不给这个勾，名单里就不收）。
pub const PG_SCHEMA_PRIVS: [&str; 2] = ["USAGE", "CREATE"];
pub const PG_TABLE_PRIVS: [&str; 7] = [
    "SELECT",
    "INSERT",
    "UPDATE",
    "DELETE",
    "TRUNCATE",
    "REFERENCES",
    "TRIGGER",
];

/// PG 的**库级**权限（`grant connect on database db to u`）。
///
/// 与模式级（USAGE/CREATE ON SCHEMA）是两层：库级决定"能不能连、能不能在里面建 schema"，
/// 模式级决定"能不能用这个模式"。界面按各库自己的模型分两页，别混成一层。
pub const PG_DATABASE_PRIVS: [&str; 3] = ["CONNECT", "CREATE", "TEMPORARY"];

pub fn is_pg_schema_priv(name: &str) -> bool {
    in_list(&PG_SCHEMA_PRIVS, name)
}

pub fn is_pg_database_priv(name: &str) -> bool {
    // TEMP 是 TEMPORARY 的旧别名，PG 两个都收；界面上只出 TEMPORARY 一列。
    in_list(&PG_DATABASE_PRIVS, name) || name.trim().eq_ignore_ascii_case("TEMP")
}

pub fn is_pg_table_priv(name: &str) -> bool {
    in_list(&PG_TABLE_PRIVS, name)
}

pub fn is_pg_routine_priv(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("EXECUTE")
}

/// 角色名 / 架构名 / 对象名这类**标识符**：只允许字母数字与 `_ $ # @ . -`（会再用 [] 圈起来）。
/// 不是为了防注入（`quote` 已经在做），而是**不给明显不是名字的东西开口子** ——
/// 这些名字同样会被拼进语句。
pub fn is_plain_ident(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n.len() <= 128 && n.chars().all(|c| c.is_alphanumeric() || "_$#@.- ".contains(c))
}

/// MySQL 的角色引用（`名字@host`）：只放行常规字符，`%` 与 `.` 是 host 的通配/域名写法。
pub fn is_role_ref(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '%' | '@' | '$'))
}

fn in_list(list: &[&str], name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    !upper.is_empty() && list.iter().any(|item| item.eq_ignore_ascii_case(&upper))
}

/// 白名单之外的「安全形状」兜底。
///
/// 各家的 SHOW GRANTS / 系统视图会返回**大量清单里没列全**的权限名 —— ClickHouse
/// 的 `UNDROP TABLE` / `dictGet` / `KILL QUERY` / `SOURCES`…（几十个）、MySQL 8 的
/// 动态权限（BACKUP_ADMIN…）、Oracle 的 `ADMINISTER …`。按「一个都不能隐身」
/// 原则界面必须显示并允许取消它们，后端一概拒绝的话，用户对这类账号**什么都保存
/// 不了**（实测 ClickHouse 的 default 用户，动任何一处保存必报「不认识的权限名」）。
///
/// 所以名单校验统一加这条兜底：名字必须是「字母/数字/下划线、词间单个空格」的形状。
/// 拼进 SQL 不会有注入风险（引号/括号/分号/注释/空白换行一律到不了 SQL），
/// 真正写错的权限名由数据库如实报语法/未知错误 —— 与名单内名字同一待遇。
pub fn is_safe_name(name: &str) -> bool {
    let s = name.trim();
    !s.is_empty()
        && s.split(' ')
            .all(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

pub fn is_oracle_system_priv(name: &str) -> bool {
    in_list(&ORACLE_SYSTEM_PRIVS, name) || is_safe_name(name)
}
pub fn is_oracle_role(name: &str) -> bool {
    in_list(&ORACLE_ROLES, name) || is_safe_name(name)
}
pub fn is_oracle_object_priv(name: &str) -> bool {
    in_list(&ORACLE_OBJECT_PRIVS, name) || is_safe_name(name)
}

/// 界面在「系统权限」页签里既能勾系统权限、也能勾角色（Oracle 的授权实际就这两类），
/// 所以这一层的校验器是两者的并集。
pub fn is_oracle_grantable(name: &str) -> bool {
    is_oracle_system_priv(name) || is_oracle_role(name)
}

#[derive(Default)]
pub struct UserPrivileges {
    /// 全局：`grant <这些> on *.* to ...`
    pub global: Vec<String>,
    /// 库级：`grant <这些> on \`db\`.* to ...`
    /// （PG 的「库级」不是这个：它的 database 层见 `pg_databases`，这字段被 PG 借去
    /// 装**模式**级 USAGE/CREATE —— 历史上前端把它当"库权限"页，2026-10 才拆开。）
    pub databases: Vec<(String, Vec<String>)>,
    /// PostgreSQL 专用 —— 真正的库级（库, 权限）：`grant connect on database db to u`。
    pub pg_databases: Vec<(String, Vec<String>)>,
    /// 表级：`grant <这些> on \`db\`.\`tbl\` to ...`
    pub tables: Vec<(String, String, Vec<String>)>,
    /// SQL Server 专用 —— schema 级（库, schema, 权限）授权，**行级选库**。
    /// 用户管理入口没有库上下文（database 传空），不选库就会把 GRANT 打到连接默认
    /// 目录（通常是 master）上 —— 实测踩过。库非空时语句前加 `use [库];`；空 = 旧行为。
    pub schema_grants: Vec<(String, String, Vec<String>)>,
    /// SQL Server 专用 —— 对象级（库, schema, 对象, 权限）授权，同上。
    pub object_grants: Vec<(String, String, String, Vec<String>)>,
    /// MySQL 专用 —— 例程级（库, 例程, 类型, 权限）授权：对**单个**存储过程/函数
    /// 授 `EXECUTE`（语法 `grant execute on procedure `db`.`p` to u`）。库级/全局的
    /// EXECUTE 只能「整个库都能跑」，够不到「只让跑这一个」—— 这是例程层存在的意义。
    pub routines: Vec<(String, String, String, Vec<String>)>,
}

/// MySQL 静态权限白名单。
///
/// 权限名会**原样拼进 SQL**（`grant <名字> on ...`），所以只认名单内的：
/// 名单外的一律让调用方拒绝，而不是拼进去 —— 这是授权语句，
/// 不能给「随便什么字符串」开口子（哪怕它看起来只是给表单用的）。
pub const MYSQL_PRIVILEGES: [&str; 33] = [
    // 表单里「全部权限」那个勾选框的值，也是合法的 grant 写法（三层都可用）
    "ALL PRIVILEGES",
    "ALTER",
    "ALTER ROUTINE",
    "CREATE",
    "CREATE ROLE",
    "CREATE ROUTINE",
    "CREATE TABLESPACE",
    "CREATE TEMPORARY TABLES",
    "CREATE USER",
    "CREATE VIEW",
    "DELETE",
    "DROP",
    "DROP ROLE",
    "EVENT",
    "EXECUTE",
    "FILE",
    "GRANT OPTION",
    "INDEX",
    "INSERT",
    "LOCK TABLES",
    "PROCESS",
    "REFERENCES",
    "RELOAD",
    "REPLICATION CLIENT",
    "REPLICATION SLAVE",
    "SELECT",
    "SHOW DATABASES",
    "SHOW VIEW",
    "SHUTDOWN",
    "SUPER",
    "TRIGGER",
    "UPDATE",
    "USAGE",
];

/// 是否是 MySQL 认得的静态权限名（大小写不敏感）。
pub fn is_mysql_privilege(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    MYSQL_PRIVILEGES.contains(&upper.as_str())
        // 兜底见 is_safe_name：MySQL 8 动态权限（BACKUP_ADMIN 等）不在静态名单里，
        // 但读回/界面都必须能带、能取消 —— 拒绝了用户就没法保存
        || is_safe_name(name)
}

/// ClickHouse 的 RBAC 权限名（与前端 UserForm 的 chPrivs **同一套**）：
/// 权限名会原样拼进 `GRANT / REVOKE`，两端必须对齐。`ALL`（可带 `PRIVILEGES`）
/// 是 CH 的"全部权限"。
pub const CLICKHOUSE_PRIVILEGES: [&str; 11] = [
    "ALL",
    "ALL PRIVILEGES",
    "SELECT",
    "INSERT",
    "ALTER",
    "CREATE",
    "DROP",
    "TRUNCATE",
    "OPTIMIZE",
    "SHOW",
    "SYSTEM",
];

/// 是否是 ClickHouse 认得的权限名（大小写不敏感）。
pub fn is_clickhouse_privilege(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    CLICKHOUSE_PRIVILEGES.contains(&upper.as_str())
        // 兜底见 is_safe_name：CH 的 SHOW GRANTS 会带出几十个清单外权限
        // （UNDROP TABLE / dictGet / KILL QUERY / SOURCES…），拒绝了就没法保存
        || is_safe_name(name)
}

/// 各家的保留字（取并集，大小写不敏感）。
///
/// 为什么要有它：`Dialect::quote` 原本是「纯字母数字就不加引号」，于是库里一条
/// 叫 `user` 的表会让服务端生成的 `delete from user` 直接语法错（SQL Server
/// errorCode 156；MySQL 的 `order` / `key` / `desc`、PG 的 `user` / `table` 同理）。
///
/// 把多方言并成一张表，是因为这里只回答一个问题：「这个标识符**能不能**裸写」。
/// 多判几个（把非保留字也当保留字）只是多打两个引号，不影响正确性；**漏判才出错**。
/// 所以宁多勿少，也不去猜「这个词在当前库里是不是真表名」。
/// 是不是一个「干净」的标识符（只含字母/数字/下划线）。
///
/// 用途：catalog 名会被**拼进 SQL**（`<catalog>.information_schema.tables`），而它来自
/// 请求参数。SQL 里表名的位置**没法用字符串字面量**（`'internal'.xxx` 是语法错），
/// 所以这里用白名单挡一道：不认识的一律当"这条路走不了"，交给调用方回退到通用路径。
fn is_plain_identifier(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

const RESERVED_WORDS: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "ANALYZE",
    "AND",
    "ANY",
    "ARRAY",
    "AS",
    "ASC",
    "ASYMMETRIC",
    "AUTHORIZATION",
    "BEGIN",
    "BETWEEN",
    "BINARY",
    "BOTH",
    "BY",
    "CALL",
    "CASCADE",
    "CASE",
    "CAST",
    "CHECK",
    "COLLATE",
    "COLUMN",
    "COMMENT",
    "COMMIT",
    "CONNECT",
    "CONSTRAINT",
    "CREATE",
    "CROSS",
    "CUBE",
    "CURRENT",
    "CURRENT_DATE",
    "CURRENT_TIME",
    "CURRENT_TIMESTAMP",
    "CURRENT_USER",
    "CURSOR",
    "DATABASE",
    "DATABASES",
    "DEALLOCATE",
    "DEC",
    "DECIMAL",
    "DECLARE",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DESCRIBE",
    "DISTINCT",
    "DIV",
    "DO",
    "DOUBLE",
    "DROP",
    "DUAL",
    "ELSE",
    "ELSEIF",
    "END",
    "ESCAPE",
    "EXCEPT",
    "EXEC",
    "EXECUTE",
    "EXISTS",
    "EXPLAIN",
    "EXTERNAL",
    "FALSE",
    "FETCH",
    "FILE",
    "FLOAT",
    "FOR",
    "FORCE",
    "FOREIGN",
    "FROM",
    "FULL",
    "FULLTEXT",
    "FUNCTION",
    "GRANT",
    "GROUP",
    "GROUPING",
    "HAVING",
    "HIGH_PRIORITY",
    "IF",
    "IGNORE",
    "IN",
    "INDEX",
    "INNER",
    "INOUT",
    "INSERT",
    "INT",
    "INTEGER",
    "INTERSECT",
    "INTERVAL",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "KEYS",
    "KILL",
    "LEADING",
    "LEAVE",
    "LEFT",
    "LEVEL",
    "LIKE",
    "LIMIT",
    "LOCK",
    "LONG",
    "LOOP",
    "MATCH",
    "MERGE",
    "MOD",
    "MODIFY",
    "NATURAL",
    "NOT",
    "NULL",
    "OFFSET",
    "ON",
    "OPTION",
    "OR",
    "ORDER",
    "OUT",
    "OUTER",
    "OVER",
    "PARTITION",
    "PRECISION",
    "PRIMARY",
    "PROCEDURE",
    "PUBLIC",
    "PURGE",
    "RANGE",
    "READ",
    "REAL",
    "RECURSIVE",
    "REFERENCES",
    "REGEXP",
    "RELEASE",
    "RENAME",
    "REPEAT",
    "REPLACE",
    "REQUIRE",
    "RESTRICT",
    "RETURN",
    "RETURNS",
    "REVOKE",
    "RIGHT",
    "ROLLBACK",
    "ROW",
    "ROWNUM",
    "ROWS",
    "SCHEMA",
    "SELECT",
    "SESSION_USER",
    "SET",
    "SHOW",
    "SIGNED",
    "SOME",
    "SPATIAL",
    "SQL",
    "START",
    "STRAIGHT_JOIN",
    "SYMMETRIC",
    "SYSTEM_USER",
    "TABLE",
    "TERMINATED",
    "THEN",
    "TO",
    "TRAILING",
    "TRANSACTION",
    "TRIGGER",
    "TRUE",
    "TRUNCATE",
    "UNDO",
    "UNION",
    "UNIQUE",
    "UNLOCK",
    "UNSIGNED",
    "UPDATE",
    "USAGE",
    "USE",
    "USER",
    "USING",
    "VALUES",
    "VARBINARY",
    "VARCHAR",
    "VARYING",
    "VIEW",
    "WHEN",
    "WHERE",
    "WHILE",
    "WINDOW",
    "WITH",
    "WRITE",
    "XOR",
    "ZEROFILL",
];

/// 是否是（任一受支持方言的）保留字 —— 决定 `quote` 能不能裸写这个名字。
pub fn is_reserved(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    RESERVED_WORDS.contains(&upper.as_str())
}

impl Dialect {
    pub fn new(kind: ConnectionKind) -> Self {
        Self { kind }
    }

    /// 这个类型属于哪个**语法族**（见 `dialect_ops`）。
    ///
    /// 从这一层开始，「按库分支」的判断都收敛到 `DialectOps` 的 impl 里：
    /// 门面方法保持签名不变（调用方零改动），内部委托给族实现。
    /// 新增同源类型（dialect 相同）不需要动任何分支。
    #[inline]
    pub(crate) fn ops(&self) -> &'static dyn crate::api::dialect_ops::DialectOps {
        crate::api::dialect_ops::of(self.kind)
    }

    /// 方言键（`mysql` / `postgresql` / …）。`pub(crate)`：同 crate 的 meta.rs 要据此判断
    /// 「布尔字面量怎么写」「LIKE 要不要显式 escape」这类差异。
    pub(crate) fn key(&self) -> &'static str {
        self.kind.key()
    }

    /// 方言短名（`mysql` / `postgresql` / `sqlserver` …）。
    ///
    /// 给「按方言分支、但不属于方言能力表」的地方用（目前只有二进制字面量的语法差异：
    /// `X'..'` / `0x..` / `hextoraw(..)` / `'\x..'::bytea` 各家都不一样）。
    pub(crate) fn engine_key(&self) -> &'static str {
        self.key()
    }

    /// 这条方言能不能用**一条语句**完成 upsert（原生 upsert）。
    ///
    /// 能的话就不必「先把目标全部主键读进内存 → 命中的逐行 UPDATE」——那是 upsert 最贵的
    /// 两步；不能的方言（ClickHouse / Doris / Derby）继续走两阶段，行为不变。
    pub fn supports_native_upsert(&self) -> bool {
        self.ops().supports_native_upsert()
    }

    /// 原生 upsert 单条语句最多带多少行（调用方按它拆分）。
    pub fn upsert_row_limit(&self) -> usize {
        self.ops().upsert_row_limit()
    }

    /// 生成一条原生 upsert 语句。
    ///
    /// - `table`：已加引号（必要时带模式前缀）的目标表名
    /// - `columns`：**目标侧**列名（未加引号），顺序与 `rows` 里的值一致
    /// - `key_indexes`：主键/唯一键在 `columns` 里的下标
    /// - `rows`：已经按本方言渲染好的字面量
    ///
    /// 各方言形态（都是「一条语句、服务端自己判插入还是更新」）：
    /// - MySQL/MariaDB：`insert … on duplicate key update c = values(c)`
    /// - PG/Kingbase/SQLite：`insert … on conflict (keys) do update set c = excluded.c`
    /// - H2：`merge into t (cols) key (keys) values …`
    /// - Oracle/DM：`merge into t tgt using (select … from dual union all …) src on (…)`
    /// - DB2/SQL Server：`merge into t as tgt using (values …) as src (cols) on (…)`
    ///
    /// 不支持的方言返回 `None`（调用方回退到两阶段）。
    pub fn upsert_sql(
        &self,
        table: &str,
        columns: &[String],
        key_indexes: &[usize],
        rows: &[Vec<String>],
    ) -> Option<String> {
        // 语句形态与列引用都在族实现里，门面只转发
        self.ops().upsert_sql(table, columns, key_indexes, rows)
    }

    /// 标识符加引号：简单名不加，避免把 MySQL 的大小写敏感表名改写掉。
    /// 带 `.` 或 `-` 之类的名字才逐段加引号。
    ///
    /// **保留字必须加引号**（`is_reserved`）：一张叫 `user` 的表会让服务端生成的
    /// `delete from user` 直接语法错（SQL Server errorCode 156，MySQL 的 `order`/`key`
    /// 同理）。实测：库里那张 `user` 表，菜单「清空表」必然失败，而同名手写
    /// `select * from [user]` 完全正常 —— 坏的只是应用自己拼的 SQL。
    pub fn quote(&self, name: &str) -> String {
        self.ops().quote(name)
    }

    /// 逐字加引号：**不动大小写**，简单名也加引号。
    ///
    /// 与 [`Self::quote`] 分工不同 —— `quote()` 翻译的是「用户想写的名字」：上折方言
    /// （Oracle / DB2 / H2 / Derby）里未加引号的名字会被服务端折成大写，所以建对象时
    /// 必须跟着折；而这里的名字来自**目录元数据**（`list_columns` / `list_tables` 的
    /// 返回值），那就是库里真实存在的大小写，加引号锁死才指得准。
    ///
    /// 反例（豫园 Oracle 11g 实测）：列是小写带引号建的 `"day"`，结果集表头点排序 →
    /// 旧实现走 `quote()` → 拼出 `order by day` → 服务端折成 `DAY` →
    /// `ORA-00904: "DAY": 标识符无效`。关键字搜索、高级搜索、就地编辑、导出同理。
    pub fn quote_exact(&self, name: &str) -> String {
        self.ops().quote_exact(name)
    }

    /// 字符串字面量（按方言转义）。
    ///
    /// MySQL 系（`key()=="mysql"`，含 MariaDB/Doris）与 ClickHouse 默认把 `\` 当转义符：
    /// 值里的反斜杠必须**先**翻倍再转义单引号，否则 `a\` 这样以 `\` 收尾的文本会把
    /// 收尾引号转义掉 —— 导出/同步生成的 INSERT/UPDATE 被截断甚至注入
    /// （`\'` 提前闭合字符串）。其余方言（PG/SQLServer/Oracle/SQLite/H2…）里 `\`
    /// 就是普通字符，动了反而改值，不动。
    pub fn literal(&self, value: &str) -> String {
        self.ops().literal(value)
    }

    pub fn is_schema_aware(&self) -> bool {
        // 名单在 `DialectOps` 的族实现里（以前写死在这里，连 YAML 的 traits 都没用上）。
        self.ops().is_schema_aware()
    }

    /// 树里「库」这一层装的其实就是**模式**（Oracle / DM：连接下直接列模式，没有服务名层）。
    ///
    /// 用途：接口只收到 `database=SYSTEM` 这种值、拆不出「库.模式」时，把它当模式用 ——
    /// 否则搜索 / ER 图会退化成「枚举当前账号能看到的**所有**模式」，一行输入等 60 秒，
    /// 结果还会混进别的模式的表（真机：Oracle SYSTEM 下搜一个表名要 63 秒）。
    pub fn db_is_schema(&self) -> bool {
        self.ops().db_is_schema()
    }

    /// 这个类型的元数据 SQL 是「库内限定」的，执行前要补一句 `use [库];` 兜底
    /// （SQL Server 专属：影子连接负责落库，USE 是第二道保险 —— 防止执行落回主连接
    /// 时把默认库的表当成目标库的，两个库节点显示同一份清单，用户实测过）。
    pub fn wants_use_switch(&self) -> bool {
        self.ops().wants_use_switch()
    }

    /// 这个类型的 `database` 是「服务器上的一个库」（服务器管着很多个、可以切换），
    /// 还是「连接固有的目标」（文件型就是一个文件；Oracle 填的其实是服务名/实例）？
    ///
    /// 这一条判断决定两件事：
    /// 1. **空库名是否允许** —— 允许 = 「连到服务器、不指定默认库」，这正是
    ///    「不填库名就列出所有有权限的库」的前提；
    /// 2. **跨库浏览能不能靠换一条连接实现**（见 `scope::resolve`）。
    ///
    /// 对 Oracle / DB2 / 文件型硬切 `database` 会把 JDBC URL 改坏，所以不在名单里。
    pub fn switchable_database(&self) -> bool {
        self.ops().switchable_database()
    }

    /// 是否为「catalog → 库 → 表」三层结构。
    ///
    /// Doris 的 catalog 是比「库」更高的一层：跨 catalog 要用全限定名
    /// `catalog.库.表`，而 `information_schema` 只反映**当前 catalog**。
    /// 目前只有 Doris；其余类型返回 false，走原来的两层结构，行为完全不变。
    pub fn catalog_level(&self) -> bool {
        self.ops().catalog_level()
    }

    /// catalog 清单。
    ///
    /// 用 `show catalogs`，**不要**改回 `information_schema.tables` 取 distinct ——
    /// 那种写法看着更"规整"（列名可控、天然是一列），但它**结构上就发现不了 JDBC catalog**：
    /// `information_schema` 只反映**当前 catalog**（internal），外部 catalog 的表在里面
    /// 一点痕迹都没有。实测：新建了一个指向 MySQL 的 catalog，`?catalog=xxx` 列库、
    /// 点开库列表都完全正常，唯独 catalog 清单里始终只有 `internal` —— 树里永远看不到它。
    ///
    /// `show catalogs` 会把**所有** catalog 列出来（包括一张表都没有的），这才是对的语义。
    /// 它的代价是**第一列是 CatalogId**、名字在第二列 `CatalogName`，而 Doris 不支持把
    /// `show` 包成子查询去挑列（实测语法错误）；所以取名字那一步改成**按列名**选，
    /// 见 `meta.rs` 的 `catalog_column_text`。
    pub fn catalogs(&self) -> Meta {
        match self.ops().catalogs_sql() {
            Some(sql) => Meta::Sql(sql.to_string()),
            None => Meta::Absent,
        }
    }

    /// 指定 catalog 下的库清单。
    ///
    /// `catalog` 由调用方**先校验**（只允许 `[A-Za-z0-9_]`，见 `catalog_literal`）——
    /// 它会被拼进 SQL，不能直接来自请求参数。
    pub fn databases_in_catalog(&self, catalog: &str) -> Meta {
        if !self.catalog_level() || !is_plain_identifier(catalog) {
            return Meta::Absent;
        }
        Meta::Sql(format!(
            "select schema_name as name from {catalog}.information_schema.schemata order by schema_name"
        ))
    }

    /// 服务器上的库清单，**只列当前账号有权限访问的**。
    ///
    /// 权限过滤交给数据库自己：SQL Server 用 `has_dbaccess`，PostgreSQL 用
    /// `has_database_privilege`，MySQL 的 `show databases` 本身就只返回有权限的。
    pub fn databases(&self) -> Meta {
        match self.ops().databases_sql() {
            Some(sql) => Meta::Sql(sql.to_string()),
            None => Meta::Unwritten,
        }
    }

    /// `catalog.库` 形式的表清单（catalog 层级专用）。
    ///
    /// 库里的对象查 `<catalog>.information_schema`，**不需要切会话** ——
    /// 这比先 `SWITCH <catalog>` 稳：后者是会话状态，一旦两条查询落在不同会话上，
    /// 就会在错的 catalog 里查（和 Redis 的 `SELECT` 是同一类坑）。
    pub fn tables_in_catalog(&self, catalog: &str, database: &str) -> Meta {
        if !self.catalog_level() || !is_plain_identifier(catalog) {
            return Meta::Absent;
        }
        // 列名与 `tables_in_schema` 保持一致（`name` / `type`），让调用方沿用同一套映射。
        //
        // ⚠️ 别名**必须加反引号**：Doris 里 `rows` 与 `type` 都是保留字，写 `as rows`
        // 直接语法错（实测：`Encountered: ROWS ... ROWS is keyword`）。反引号是 MySQL 系
        // 的写法，这条 SQL 只服务 Doris，安全。
        //
        // 不查行数：Doris 的 `TABLE_ROWS` 多是估算甚至 0，问了也不可信 ——
        // 留空由前端批量 COUNT(*) 回填（见 /table-count）。
        Meta::Sql(format!(
            "select table_name as `name`, \
             case when table_type like '%VIEW%' then 'VIEW' else 'TABLE' end as `type` \
             from {catalog}.information_schema.tables where table_schema = {} order by table_name",
            self.literal(database)
        ))
    }

    /// `catalog.库` 形式的表选项（catalog 层级专用，列名规则同 `table_options`）。
    ///
    /// 为什么要单独一条：`table_options` 查的是**当前 catalog** 的 `information_schema`，
    /// 而外部 catalog 的表在里面一点痕迹都没有（与 `tables_in_catalog` 同一个理由）。
    /// 少了它，catalog 层级那条路径上的表注释就永远取不到。
    pub fn table_options_in_catalog(&self, catalog: &str, database: &str) -> Meta {
        if !self.catalog_level() || !is_plain_identifier(catalog) {
            return Meta::Absent;
        }
        Meta::Sql(format!(
            // 列要与 `table_options` **对齐**：对象清单页是按库名（`catalog.库`）请求的，
            // 走的正是这条；少一列的效果就是「同一个库，从页内下拉切换时列有值、从树点进去全空」。
            // 真机踩过：Doris 表清单的「创建时间」恒空，因为这里没查 create_time。
            // 视图的假注释（'VIEW'）同 `table_options` 一样判掉。
            "select table_name as table_name, engine as engine, \
             table_collation as charset, \
             case when table_type = 'VIEW' and table_comment = 'VIEW' then '' \
                  else coalesce(table_comment, '') end as comment, \
             create_time as createTime \
             from {catalog}.information_schema.tables where table_schema = {}",
            self.literal(database)
        ))
    }

    /// schema 清单（只有 schema 层级的类型需要）。
    pub fn schemas(&self) -> Meta {
        match self.ops().schemas_sql() {
            Some(sql) => Meta::Sql(sql.to_string()),
            None => Meta::Absent,
        }
    }

    /// 某个 schema 下的表/视图（schema 层级专用；非 schema 类型走内核的 DatabaseMetaData）。
    pub fn tables_in_schema(&self, schema: &str) -> Meta {
        match self.ops().tables_in_schema_sql(schema) {
            Some(sql) => Meta::Sql(sql),
            None => Meta::Unwritten,
        }
    }

    /// 某个用户的详情：**一行**结果，含
    /// `name` / `create_user_sql` / `grants_text`（换行分隔的授权语句）以及若干明细列。
    ///
    /// 为什么把授权拼成换行文本而不是多行：调用方要的是一份「可执行脚本文本」，
    /// 多行结果还要自己逐行收集、顺序还可能变；一行内拼好，取数端只做 split。
    ///
    /// 各家系统表差别很大，**只写有把握的**：SQL Server / MySQL / PostgreSQL。
    /// 其余返回 `Unwritten` —— 用户管理涉及权限，猜错的代价比不显示大得多。
    /// StarRocks 的用户模型与 MySQL **对不上**，相关目录一律按「没有这个概念」处理。
    ///
    /// 两条实测依据（StarRocks 5.1）：
    /// · 它没有 `mysql` 库，`select … from mysql.user` 直接报 `Unknown database 'mysql'`
    ///   —— 也就是 Doris 那套用户清单在这里必然 500；
    /// · `show users` 只给一行 `'root'@'%'` 字符串，权限要读 `show grants` 的文本，
    ///   而 StarRocks 是**角色授予**的 RBAC（`GRANT 'role' TO 'user'`），没有 MySQL 的
    ///   对象级权限矩阵 —— 硬套会画出一张勾不对的勾选表，比"没有入口"更误导。
    ///
    /// Doris 不受影响：它有 `mysql.user`，继续走 MySQL 那套。
    fn starrocks_rbac_only(&self) -> bool {
        self.kind.key() == "starrocks"
    }

    pub fn user_info(&self, name: &str) -> Meta {
        if self.starrocks_rbac_only() {
            return Meta::Absent;
        }
        self.ops().user_info(name)
    }

    /// 用户权限明细：**逐行**返回（全局 / 库级 / 表级各一档），由调用方拼成 `GRANT ...` 文本。
    ///
    /// 为什么不在这里直接用 `group_concat` 拼成一行：`group_concat` 受会话变量
    /// `group_concat_max_len` 限制（服务端默认 **1024**），超长会**静默截断**。
    /// 实测 `root@%`（28 项权限）正好在 1024 处被切断，最后一行成了半截的
    /// `GRANT TRIGGER ON *.* T` —— 界面上于是少了 UPDATE / TRIGGER 两个勾
    /// （用户看到的现象是「root 怎么不是全都有」）。逐行返回没有这个上限。
    ///
    /// 另外 `/*+ SET_VAR(group_concat_max_len = ...) */` 这条路走不通：它只对部分
    /// 优化器相关变量生效，实测长度仍是 1024。
    ///
    /// 已知覆盖：**静态**权限。8.0 的动态权限（`mysql.global_grants`）与角色授予
    /// 不在这三张视图里，需要时按 `role_edges` 另补（5.7 没有那张表，先不引用）。
    pub fn user_grants(&self, user: &str, host: &str) -> Meta {
        if self.starrocks_rbac_only() {
            return Meta::Absent;
        }
        self.ops().user_grants(user, host)
    }

    /// 已授予该账号的**角色**（MySQL 8 / MariaDB 的 RBAC）。
    ///
    /// 两家用的**不是同一张表**（这一点踩过）：MySQL 8.0.1+ 是 `mysql.role_edges`
    /// （FROM_USER/FROM_HOST/TO_USER/TO_HOST），MariaDB 10.0.5+ 是 `mysql.roles_mapping`
    /// （Host/User/Role/Admin_option，Host 是**被授人**的 host）。MariaDB 上读 role_edges
    /// 会直接 1146 —— 那是「角色」页签一片空白、连已授的都看不见。
    /// MariaDB 的角色**没有 host**（`grant 'r'@'%'` 是语法错，实测 1064），
    /// 所以 role_host 一律回空串，界面按裸名字显示。
    /// Doris 的 RBAC 没有这两张表（返回 Unwritten，界面就不出角色这一页）。
    pub fn user_roles(&self, user: &str, host: &str) -> Meta {
        if self.starrocks_rbac_only() {
            return Meta::Absent;
        }
        self.ops().user_roles(user, host)
    }

    /// 这台服务器上**出现过**的角色（界面要能列出「还没授的」，否则看不见也就授不了）。
    ///
    /// MySQL 8 没有独立的角色表（角色就是 `mysql.user` 里 `account_locked='Y'` 且
    /// 认证串为空的账号）：只读 `role_edges` 的话，**没授予过任何人的角色在界面上
    /// 看不见**，等于建了也用不了，所以两者取并集。
    /// MariaDB 的 `CREATE ROLE` 会自己往 `roles_mapping` 写一行（实测），读它即可。
    /// Oracle / ClickHouse 各自有字典：`dba_roles` / `system.roles`（都不带 host）。
    /// Doris 只有 `SHOW ROLES` 这种命令式清单，列名是 `Name`（调用方按候选名取）。
    /// 角色自己的「详情行」（界面复用用户表单来编辑**角色**的权限）。
    ///
    /// 角色和账号在有些方言里根本不在同一张目录（Oracle 的 `dba_roles`、
    /// ClickHouse 的 `system.roles`、Doris 的 `SHOW ROLES`），所以不能拿
    /// `user_info` 硬查 —— 查不到就是"没有找到用户"，角色页永远打不开。
    pub fn role_info(&self, name: &str) -> Meta {
        if self.starrocks_rbac_only() {
            return Meta::Absent;
        }
        self.ops().role_info(name)
    }

    /// 角色**自身拥有**的权限（与 `user_grants` 同形状：scope/privilege/owner/db_name/table_name/grant_option）。
    pub fn role_grants(&self, name: &str, host: &str) -> Meta {
        if self.starrocks_rbac_only() {
            return Meta::Absent;
        }
        self.ops().role_grants(name, host)
    }

    pub fn role_inventory(&self) -> Meta {
        if self.starrocks_rbac_only() {
            return Meta::Absent;
        }
        self.ops().role_inventory()
    }

    /// 用户新建 / 改密 / 属性 / 授权 / 撤销 / 删除（**多条语句用 `;\n` 连接**，
    /// 由调用方拆分逐条执行）。这里只做一件事：把 `alter-role` 归一成 `alter` +
    /// 「这次保存的是角色」，SQL 的拼法全在各族实现里（见 `DialectOps::user_action_sql`）。
    pub fn user_action_sql(
        &self,
        action: &str,
        user: &str,
        password: &str,
        host: &str,
        roles: &[String],
        privileges: &UserPrivileges,
        revokes: &[GrantRevoke],
        reset_privileges: bool,
        attrs: &UserAttrs,
    ) -> Meta {
        // StarRocks 的用户体系是角色 RBAC（见 starrocks_rbac_only 的注释），
        // 不产出 MySQL 形状的建/改/删语句 —— 界面上也不会有入口进来。
        if self.starrocks_rbac_only() {
            return Meta::Absent;
        }
        // **角色**的保存（`alter-role`）：只出差量授权，不出 `alter user`。
        // 角色没有口令 / 锁定 / 配额这些属性，前端在角色模式也不会交它们，
        // 所以把 action 归一成 alter 即可 —— 属性为空时各分支不会拼 alter 语句。
        // 归一**只在这一层做**：各实现只认 create / alter / drop，不重复判断。
        let as_role = action == "alter-role";
        let action = if as_role { "alter" } else { action };
        self.ops().user_action_sql(
            action,
            as_role,
            user,
            password,
            host,
            roles,
            privileges,
            revokes,
            reset_privileges,
            attrs,
        )
    }

    /// 表/视图的建表语句。
    ///
    /// `schema` = 该对象所在的模式（请求里的「库.模式」拆出来的那半；调用方用
    /// [`crate::api::meta::split_db_schema`]）。**不能只靠会话默认模式**：DBMind 的查询
    /// 会话刻意不按"树上选中的模式"切换（不赌代理有没有真的切 schema），于是
    /// 在 SYS 下点一张表、会话默认模式却是 SYSTEM 时，Oracle 会报 ORA-31603
    /// 「对象在方案 SYSTEM 中未找到」—— 这就是真机上报的那个错。
    pub fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        self.ops().ddl(table, schema)
    }

    /// DDL 的「附加语句」：注释 / 索引与唯一约束 / 外键。
    ///
    /// 为什么不能写进 `create table` 里：SQL Server 的表与列注释是**扩展属性**
    /// （`sp_addextendedproperty`），索引与外键也是独立语句。只给 create table，
    /// 用户复制出来的「建表语句」重建出来就是一张没有注释、没有索引、没有外键的空壳。
    ///
    /// 每段都是**一次查询返回可直接执行的语句文本**：拼接与转义（比如注释里的单引号）
    /// 全在数据库里做完，上层只负责换行追加 —— 少一层「Rust 里再拼一遍 SQL」的转义，
    /// 就少一类引号事故。
    ///
    /// MySQL / SQLite 返回空：它们的 `show create table` / `sqlite_master.sql`
    /// 本来就把注释、索引、外键带在一条语句里了。
    pub fn ddl_extras(&self, table: &str) -> Vec<(&'static str, String)> {
        self.ops().ddl_extras(table)
    }

    /// 「当前账号能用的表」清单 —— 给**表清单走 JDBC `getTables`、而目录本身不按权限收窄**
    /// 的方言用（H2 / DB2 / Derby）。
    ///
    /// 为什么需要：实测 H2 里给低权限账号只授了一张表的 SELECT，`DatabaseMetaData.getTables`
    /// 仍然把同一模式里**一张都读不了**的表全列出来（对它执行 `select …` 直接报
    /// `Not enough rights for object`）。DB2 同理（`SYSCAT` 是公开目录）。这两家的目录里
    /// 没有「是否可见」标志，只能把「属主 + 显式授权（含 PUBLIC）」拼出来，再与 JDBC 结果求交。
    ///
    /// `Meta::Absent` = **不需要过滤**：MySQL/Oracle/PG/CH 的清单查询本身已经按权限收窄。
    pub fn visible_tables(&self) -> Meta {
        // H2 / DB2 / Derby 的目录不按权限收窄，要把「属主 + 显式授权」拼出来求交；
        // 其余方言的清单查询本身已按权限收窄，Absent 表示「不用过滤」
        match self.ops().visible_tables() {
            Some(sql) => Meta::Sql(sql.to_string()),
            None => Meta::Absent,
        }
    }

    /// 一张表的**估算行数**（一次查询拿到全库所有表），供树节点「先显示个数字」。
    ///
    /// 为什么需要它：精确值要 `count(*)` 扫表 —— 实测某张 20 万行的表在 MySQL 上
    /// 走 JDBC 宿主要好秒级，树上不能默认对每张表都跑一次。这里取引擎自己维护的统计值，
    /// 代价只有一次查询。
    ///
    /// 两条诚实的约束：
    /// 1. 这是**估算**（InnoDB 的统计值可能滞后，不是精确值）；
    /// 2. **估算为 0 时返回 null**，不显示数字 —— 「统计说 0」与「这张表真的是空的」
    ///    在界面上长得一模一样，宁可先留空，等精确回填补上真实的 0。
    pub fn table_rows(&self, database: &str) -> Meta {
        match self.ops().table_rows_sql(database) {
            Some(sql) => Meta::Sql(sql),
            None => Meta::Unwritten,
        }
    }

    /// 每张表的**真实**表选项：表名 / 引擎 / 排序规则 / 表注释（一次查询拿全库）。
    ///
    /// 为什么需要它：内核的 `TableInfo` 只有名字与行数，界面的「基本信息」却要回显
    /// **这张表真实的**引擎 / 字符集 / 排序规则 / 表注释。不回填的话前端只能回退到
    /// 「能力清单的第一项」—— 看着像回显、其实是默认值：表是 MyISAM 也显示 InnoDB，
    /// 改别的项保存时会顺手把引擎一起改掉。
    ///
    /// `charset` 这一列给的是**排序规则的完整名**（如 `utf8mb4_general_ci`）：
    /// 与 `shape::tables_json` 里该字段的历史含义一致，前端据此拆出字符集与排序规则。
    /// 每张表的**真实**表选项：表名 / 引擎 / 排序规则 / 表注释（一次查询拿全库）。
    ///
    /// 为什么需要它：内核的 `TableInfo` 只有名字与行数，界面的「基本信息」却要回显
    /// **这张表真实的**引擎 / 字符集 / 排序规则 / 表注释。不回填的话前端只能回退到
    /// 「能力清单的第一项」—— 看着像回显、其实是默认值：表是 MyISAM 也显示 InnoDB，
    /// 改别的项保存时会顺手把引擎一起改掉。
    ///
    /// 列名规则：SQL 里用 `as <JSON 键>` 直接把最终键名定下来（`table_name` 只用于配对，
    /// 其余键会原样并入 `/tables` 的每个表对象）；**拿不到就不给这个键**，界面留空。
    /// 各家能拿到的项本来就不同（MySQL 有引擎/字符集、ClickHouse 有排序键/分区键、
    /// PG 只有注释），所以按「查到什么给什么」处理，不硬凑。
    ///
    /// `charset` 这一列给的是**排序规则的完整名**（如 `utf8mb4_general_ci`）：
    /// 与 `shape::tables_json` 里该字段的历史含义一致，前端据此拆出字符集与排序规则。
    pub fn table_options(&self, database: &str) -> Meta {
        // `catalog.库` 这类多段库名的拆解在 Doris 的族实现里（只有它有多段库名）
        match self.ops().table_options_sql(database) {
            Some(sql) => Meta::Sql(sql),
            None => Meta::Unwritten,
        }
    }

    /// 索引清单（全库返回，每条带 `table`，结构页自己按表名过滤）。
    pub fn indexes(&self) -> Meta {
        match self.ops().indexes_sql() {
            Some(sql) => Meta::Sql(sql.to_string()),
            None => Meta::Unwritten,
        }
    }

    /// 例程清单（存储过程 / 函数）。
    ///
    /// `schema` = 要查哪个模式的例程（树上就是模式节点）；`None` = 交给会话当前模式。
    ///
    /// Oracle 以前用 `user_objects`，它只认**会话当前模式**，而例程请求根本不收 schema
    /// —— 树上点哪个模式都是 0（连 SYS 都是 0，一眼假）。这里改成 `all_objects` +
    /// `owner` 明确过滤：判据来自请求本身，不赌「代理有没有真的切 schema」。
    /// 两边都 upper()：未加引号的标识符在字典里是大写，但用户也可能建了带引号的小写模式。
    pub fn procedures(&self, schema: Option<&str>) -> Meta {
        self.ops().procedures(schema)
    }

    /// 触发器清单。`schema` 含义同 `procedures`（Oracle 同样是
    /// `user_triggers` → `all_triggers` + `owner` 过滤）。
    pub fn triggers(&self, schema: Option<&str>) -> Meta {
        self.ops().triggers(schema)
    }

    pub fn events(&self) -> Meta {
        self.ops().events()
    }

    pub fn users(&self) -> Meta {
        if self.starrocks_rbac_only() {
            return Meta::Absent;
        }
        self.ops().users()
    }

    /// 对象定义（右键「编辑」用它取到 SQL 再转 ALTER）。
    ///
    /// `schema` 含义同 [`Self::ddl`]：Oracle 要靠它做 `owner`，PG 要靠它筛 `pg_namespace`。
    /// 不带时：Oracle 的 `get_ddl` 按会话默认模式找（真机就是这么报 ORA-31603 的），
    /// PG 会在所有非系统模式里取第一个同名对象。
    pub fn object_source(&self, object_type: &str, name: &str, schema: Option<&str>) -> Meta {
        self.ops().object_source(object_type, name, schema)
    }

    /// 表级危险操作（清空 / 截断 / 删除 / 重命名）：方言差异（CH 的 TRUNCATE、
    /// DB2 的 IMMEDIATE、SQLite 的退化 DELETE、各家 RENAME 的写法）都在族实现里。
    pub fn table_action(&self, table: &str, action: &str, new_name: Option<&str>) -> Meta {
        self.ops().table_action(table, action, new_name)
    }

    /// 删除视图 / 例程 / 触发器 / 事件（同步的「删除重建」策略要用）。
    ///
    /// `drop X if exists` 不是所有方言都认：MySQL 系、PostgreSQL、SQLite、H2、ClickHouse（视图）、
    /// SQL Server 2016+ 都支持；Oracle / DM / DB2 / Derby 没有这个写法。
    /// 与其拼一句"看着像、执行就报错"的语句，不如返回 `None`，由调用方跳过并说明白。
    pub fn drop_object_sql(&self, kind: &str, name: &str) -> Option<String> {
        if !self.ops().supports_drop_object(kind) {
            return None;
        }
        Some(format!("drop {kind} if exists {}", self.quote(name)))
    }

    /// 视图 / 例程改名（同步要复刻源端「对象名与定义正文不一致」的状态时用）。
    ///
    /// 只有 SQL Server 与 PostgreSQL 有真正的改名语句：
    /// - SQL Server 的 `sp_rename` 是唯一入口，它**只改目录**（`sys.objects.name`），
    ///   不改 `sys.sql_modules.definition` 里的正文 —— 这正是源库里「树上 `test1111`、
    ///   定义里 `CREATE PROCEDURE [test111]`」的来历，同步复刻它得靠同一句 `sp_rename`；
    ///   参数是**字符串字面量**（不是标识符），且**必须带模式**，否则按调用者的默认模式解析。
    /// - PostgreSQL 是 `alter <kind> <old> rename to <new>`；触发器要在所属表上改
    ///   （`alter trigger … on <表>`），这里不支持。
    ///
    /// 其余方言（MySQL 系 / Oracle / DB2 / H2 / Derby / DM）没有改名语句，
    /// 返回 `None` 让调用方改走「改写定义里的名字」这条路。
    pub fn rename_object_sql(
        &self,
        kind: &str,
        from: &str,
        to: &str,
        schema: Option<&str>,
    ) -> Option<String> {
        self.ops().rename_object_sql(kind, from, to, schema)
    }

    /// 标识符引号字符：反引号 / 方括号 / 双引号。
    ///
    /// 给「把源端定义里的引号换成目标方言」这类字面替换用 —— 不能拿 `quote("_")`
    /// 反推（它对朴素名字不加引号，首字符会取成 `_` 本身）。
    pub fn quote_mark(&self) -> char {
        self.ops().quote_mark()
    }

    /// 一条 `INSERT` 能不能带多组 `VALUES (…), (…)`。
    pub fn supports_multi_row_insert(&self) -> bool {
        self.ops().supports_multi_row_insert()
    }

    /// 建库语句（目标库不存在时的自动建库）。
    pub fn create_database_sql(&self, name: &str) -> String {
        // Databend 走 MySQL 协议，但 `create database` **不支持 charset 子句**
        //（语法只有 `[IF NOT EXISTS] <name> [COMMENT = '…']`）—— 跟着 MySQL 族走会建库失败。
        if self.kind.key() == "databend" {
            return format!("create database if not exists {}", self.quote(name));
        }
        self.ops().create_database_sql(name)
    }

    /// 删表语句（整库重同步前的清场）。`quoted_table` 是已按方言加过引号的表名。
    pub fn drop_table_sql(&self, quoted_table: &str) -> String {
        self.ops().drop_table_sql(quoted_table)
    }

    /// 整库同步时「同时搬几张表」的默认并发（对象级，不是单表写连接数）。
    pub fn sync_object_concurrency(&self) -> usize {
        self.ops().sync_object_concurrency()
    }

    /// 表行数估算 SQL（整库同步的调度排序用）。`None` = 没有现成统计视图。
    pub fn table_rows_estimate_sql(&self, database: &str, schema: &str) -> Option<String> {
        // Databend 的 information_schema.tables 没有 `table_rows` 列，跟着 MySQL 族查
        // 只会每次多一次失败往返（排序退回原序）—— 如实说没有。
        if self.kind.key() == "databend" {
            return None;
        }
        self.ops().table_rows_estimate_sql(database, schema)
    }

    /// Oracle（含 11g）的**包装式**分页：`offset … rows fetch next … rows only` 是
    /// 12c 语法，11g（如豫园 11.2.0.4）直接 ORA-00933「SQL 命令未正确结束」。
    /// ROWNUM 包装 9i 起可用，对 12c+ 也完全合法，所以 Oracle 一律走这条路，
    /// 不必探测版本。代价是结果里多一列辅助列 `rn__`，调用方负责用
    /// `shape::strip_paging_aux` 从结果里剥掉。
    ///
    /// 返回 `None` = 该方言没有包装式分页，调用方退回 [`limit_clause`] 后缀。
    pub fn wrap_paging(&self, base: &str, offset: u64, size: u64) -> Option<String> {
        self.ops().wrap_paging(base, offset, size)
    }

    /// 分页子句（`offset` 从 0 开始）。
    pub fn limit_clause(&self, offset: u64, size: u64) -> String {
        self.ops().limit_clause(offset, size)
    }

    /// 取服务器版本的一行 SQL（列别名 `v`）。该方言没有统一写法就返回 None。
    ///
    /// 为什么 AI 提示词里要有版本：**同一句 SQL 换个版本就是语法错**。
    /// MySQL 5.7 没有窗口函数 / CTE（`PARTITION BY`、`WITH` 直接 1064），
    /// SQL Server 2012 之前没有 `OFFSET FETCH`，MySQL 8 才支持 `CHECK` 约束。
    /// 模型不知道版本时只会按"最新版"写，在 5.7 上必然报错 —— 这个坑真机踩过。
    pub fn version_sql(&self) -> Option<&'static str> {
        self.ops().version_sql()
    }

    /// SQL Server 的 `OFFSET` 必须跟 `ORDER BY` 才能用。
    pub fn needs_order_by_for_paging(&self) -> bool {
        self.ops().needs_order_by_for_paging()
    }

    /// 是否支持**行值比较** `(a, b, c) > (x, y, z)`（keyset 分页靠它才能走索引 range）。
    ///
    /// 为什么必须用它而不是展开成 `(a > x) or (a = x and b > y) or …`：
    /// 语义一样，但优化器能算出的扫描范围差**三个数量级**。真机 EXPLAIN（6 列主键、
    /// 81 万行的 `xingwei_kechuan_mdadvancesettlememo_m`，取 652 行一页）：
    ///
    /// | 写法 | type | 预估扫描行数 |
    /// |---|---|---|
    /// | 逐列 or 链 | range | **457,379** |
    /// | 行值比较 | index | **652** |
    ///
    /// or 链每页都要扫几十万行，那张 58 万行的表因此跑了 **68 分钟（896 批）**，
    /// 而且读得慢还会触发缩页（1/4），批次越翻越多 —— 用户感觉就是「越到后面越慢」。
    ///
    /// 只对确定支持且能走索引的方言开启：SQL Server / Oracle / Doris / Derby 不认这个
    /// 语法，ClickHouse 的 tuple 比较语义与 keyset 不是一回事，都继续用 or 链。
    pub fn supports_row_value_compare(&self) -> bool {
        self.ops().supports_row_value_compare()
    }

    /// 同实例直拷：源表在**目标会话**上的限定层级（见 [`DirectRefDepth`]）。
    pub fn direct_ref_depth(&self) -> DirectRefDepth {
        self.ops().direct_ref_depth()
    }

    /// 「两个连接是不是同一个实例」的指纹 SQL：只有指纹真的唯一的方言才给。
    ///
    /// 同**一条连接**不需要指纹（本来就是同一个物理会话）；这条只服务「用户建了两个
    /// 连接、其实指向同一个库」的情况 —— 那种场景下直拷同样是数量级的差别。
    /// 拿不准的方言（Doris / Oracle / DB2 / 嵌入式）一律返回 `None`：宁可走管道，
    /// 也不要把数据插到另一个实例的同名表里。
    pub fn instance_fingerprint_sql(&self) -> Option<&'static str> {
        self.ops().instance_fingerprint_sql()
    }

    /// 清空一张表的语句（"清空后导入"模式用）。
    ///
    /// 为什么不能统一写 `delete from`：**ClickHouse 不吃裸 DELETE** ——
    /// 实测报 `Code: 62 ... Syntax error: failed at position 26 (end of query):
    /// . Expected one of: ON, IN, PARTITION, WHERE`（它要求带 `WHERE`，或者用 TRUNCATE）。
    /// 反过来 SQLite / Derby 没有 `TRUNCATE`，只能 `delete from`。
    /// 其余方言两者都认，选 `truncate`（快得多）。
    pub fn clear_table_sql(&self, table: &str) -> String {
        self.ops().clear_table_sql(table)
    }

    /// 更新一行的语句（upsert 的更新分支用）。
    ///
    /// **ClickHouse 没有标准 `UPDATE ... SET ...`**：它只接受
    /// `ALTER TABLE ... UPDATE ... WHERE ...`（异步 mutation）。写标准 UPDATE 会被它拒掉。
    pub fn update_sql(&self, table: &str, assignments: &str, condition: &str) -> String {
        self.ops().update_sql(table, assignments, condition)
    }

    /// 统计行数。表引用走 [`Self::quote_exact`] —— 调用方传的都来自目录元数据
    /// （树里的表名），用 `quote()` 会把 Oracle 上带引号建的小写表折成大写、报 ORA-00942。
    pub fn count_sql(&self, table: &str) -> String {
        self.count_sql_in(None, table)
    }

    /// 同 [`Self::count_sql`]，但**带模式**（`db_is_schema` 的方言必须用这个，见 [`Self::table_ref`]）。
    pub fn count_sql_in(&self, schema: Option<&str>, table: &str) -> String {
        format!("select count(*) as cnt from {}", self.table_ref(schema, table))
    }

    /// 表 / 视图引用：`select … from`、`insert into`、`update`、`delete from` 后面那一段。
    ///
    /// 两件事：
    /// 1. 名字**逐字加引号**（来自目录元数据，不能折大小写，见 [`Self::quote_exact`]）；
    /// 2. **`db_is_schema` 的方言（Oracle / DM）必须补模式前缀** —— 它们没有可切的会话模式
    ///    （`run_sql_in` 切 schema 在达梦上不生效，见 oracle.rs 的 `dm_owner_probe` 注释），
    ///    不带前缀就按登录用户的默认模式解析。真机：达梦 `database=DEMO` 时
    ///    `select * from "USERS"` 报「无效的表或视图名[USERS]」，`"DEMO"."USERS"` 才正常。
    ///
    /// 其余方言只用表名：目标库由影子连接 / `use [库]` / search_path 决定，多补一段可能指错。
    pub fn table_ref(&self, schema: Option<&str>, table: &str) -> String {
        let table = table.trim();
        if table.is_empty() || table.contains('.') {
            return self.quote_exact(table);
        }
        match schema.map(str::trim).filter(|name| !name.is_empty()) {
            Some(schema) if self.db_is_schema() => self.quote_exact(&format!("{schema}.{table}")),
            _ => self.quote_exact(table),
        }
    }

    /// 按列元数据拼 `CREATE TABLE`。
    ///
    /// 这是**兜底**：大多数类型拿不到「一句原始建表语句」（见 `ddl` 的三档语义），
    /// 但导出/同步都需要一个能建出「列对得上」的语句。拼出来的会少掉索引、外键
    /// 以及部分默认值表达式 —— 调用方有义务把这件事告诉用户，别让人以为拿到了完整结构。
    /// `source` 是**这些列的来源方言**：
    /// - `Some(别的类型)` ⇒ 跨类型同步：类型要**翻译**（MySQL `datetime` 建不到 Oracle 上）、
    ///   默认值要按目标重新解释（见 `default_literal` 的 `cross`）；
    /// - `None` 或与自身相同 ⇒ 同类型照抄，行为与加这个参数之前**完全一致**（零回归）。
    pub fn create_table_from_columns(
        &self,
        table: &str,
        columns: &[ColumnDetail],
        source: Option<ConnectionKind>,
    ) -> String {
        self.create_table_with_comments(table, columns, source, &std::collections::HashMap::new(), None)
    }

    /// 同 `create_table_from_columns`，另把**表注释 + 字段注释**写进建表语句。
    ///
    /// 数据同步的「目标表不存在时自动建表」用它 —— 以前这条链路**一个注释都不带**：
    /// 内核的 `ColumnDetail` 没有 comment 字段，注释只在方言的元数据查询里取得到，
    /// 于是目标表建出来是「结构对、注释全空」，还得人工补一遍。
    /// 注释由调用方按**源表**取好传进来（`column_comments` 的键是列名小写）。
    ///
    /// 只有能**内联**注释的方言（MySQL / MariaDB / Doris / ClickHouse）会写进建表语句；
    /// PG / Oracle / SQL Server 的注释必须**另发**语句 —— 那一步在
    /// [`Dialect::post_create_comments`]（建表之后由 sync.rs 执行 `comment on …` /
    /// `sp_addextendedproperty`）；SQLite / Derby 没有注释这个概念，如实跳过。
    pub fn create_table_with_comments(
        &self,
        table: &str,
        columns: &[ColumnDetail],
        source: Option<ConnectionKind>,
        column_comments: &std::collections::HashMap<String, String>,
        table_comment: Option<&str>,
    ) -> String {
        let cross = matches!(source, Some(kind) if kind != self.kind);
        // Doris 的 KEY 列集合：主键列；没有主键时是**第一列**（兜底 DUPLICATE KEY，见下方分支）
        let has_pk = columns.iter().any(|c| c.primary_key);
        // (是否主键列, 行文本)：Doris 要求键列排最前，所以要记得住哪几行是键
        let mut lines: Vec<(bool, String)> = Vec::new();
        let mut primaries: Vec<String> = Vec::new();
        // 主键列**落成目标方言之后**的类型名：MySQL 建表要据此估算索引键长
        //（见 `mysql_charset_for_key`）—— 跨类型同步时类型已经被翻译过，
        // 用 `columns` 里的原始类型名会算错。
        let mut primary_types: Vec<String> = Vec::new();
        for (col_index, column) in columns.iter().enumerate() {
            // 类型名单独取出来：默认值怎么渲染要按**类型**判（文本列一律加引号，见 default_literal）
            let raw_type = column
                .type_name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "text".to_string());
            let family = TypeFamily::parse(&raw_type);
            // 只有跨类型才翻译；同类型原样照抄
            let mut type_name = if cross { family.render(self.kind) } else { raw_type };
            // 长度是「无界哨兵」（2147483647 / 4294967295）时**绝不能原样拼进 DDL**：
            // 那是 JDBC / information_schema 对无界大文本的占位值，不是真实长度。
            // 原样拼出去的后果按方言不同：SQL Server 2715（exceeds the maximum allowed
            // for any data type (8000)）、MySQL「Display width out of range」、
            // Oracle ORA-00907。真机上整库同步因此漏掉 3 张表（建表失败只在日志里）。
            // 同类型照抄（cross=false）时走的就是这里，所以收敛必须放在类型名定稿之后。
            // tidb 的类型长度语义与 mysql 一致（无界哨兵的收敛按同一套做）
            if let Some(fixed) = unbounded_type_of(
                &type_name,
                if matches!(self.kind.key(), "tidb" | "oceanbase") {
                    "mysql"
                } else {
                    self.kind.key()
                },
            ) {
                type_name = fixed;
            }
            // **Doris 键列不能是无界文本**：STRING/TEXT 做 UNIQUE/DUPLICATE KEY 建表直接报
            // 「String Type should not be used in key column」（真机：ClickHouse 的 String
            // 主键/排序键同步到 Doris 必炸）。键列统一落成有长度的 VARCHAR(65533)
            //（实测 Doris 接受它做键列；无键列长度上限之忧，只是普通列保持 STRING 不动）。
            let is_doris_key =
                matches!(self.kind.key(), "doris" | "starrocks")
                    && (column.primary_key || (!has_pk && col_index == 0));
            if is_doris_key && is_unbounded_text_type(&type_name) {
                type_name = "varchar(65533)".to_string();
            }
            // H2 目标：内核回显的完整类型文本会带 `integer(32,0)` / `double(10,0)` 这种
            // **精度括号**，H2 2.x 对整数/浮点不接受（真机：重建目标表直接语法错）——
            // 整数与浮点类剥掉括号；`character varying(50)` 的括号是合法长度，保留。
            if self.kind.key() == "h2" {
                let lower = type_name.to_ascii_lowercase();
                for base in [
                    "integer", "bigint", "smallint", "tinyint", "double", "real", "float",
                ] {
                    if lower.starts_with(base) && type_name.contains('(') {
                        type_name = base.to_string();
                        break;
                    }
                }
            }
            // ClickHouse 的列**默认就是 NOT NULL**——「不写 not null」对它无效，
            // 可空列必须显式 `Nullable(T)`，否则源全是可空列、建出来全不可空（真机踩过）。
            // 例外：主键/排序键不允许 Nullable（保持原样，源主键本来就不是可空列）；
            // Array/Map/Tuple 这类复合类型也不能包 Nullable。
            if self.kind.key() == "clickhouse" && column.nullable && !column.primary_key {
                let t = type_name.trim();
                let wrapper_ok = !t.starts_with("Array(")
                    && !t.starts_with("Map(")
                    && !t.starts_with("Tuple(")
                    && !t.starts_with("Nullable(");
                if wrapper_ok {
                    type_name = format!("Nullable({t})");
                }
            }
            let mut line = format!("  {} {}", self.quote(&column.name), type_name);
            // 自增子句先算出来：**位置按方言定**，不能一律追加在列定义末尾（见下）。
            let auto_clause = if column.auto_increment && column.primary_key {
                auto_increment_clause(self.kind, family)
            } else {
                None
            };
            // Oracle 的列定义**顺序是硬性的**（真机实测，见 `_uitest/ddl-syntax-probe.py`）：
            //   类型 → [generated … as identity] → [default] → not null
            // 只要 `not null` 写在前面，后面跟 identity 或 default 都是
            // ORA-03076「列定义或内部约束条件中存在意外项 GENERATED / DEFAULT」。
            // DM 按 Oracle 兼容处理。其余方言两种顺序都收（PG / DB2 / H2 / Derby / SQL Server
            // 逐个实测，见 `_uitest/ddl-ident-probe.py`），所以只对 Oracle 系调整，其它零回归。
            let oracle_order = matches!(self.kind.key(), "oracle" | "dm");
            if oracle_order {
                if let Some(clause) = auto_clause {
                    line.push(' ');
                    line.push_str(clause);
                }
            }
            if !oracle_order && !column.nullable {
                line.push_str(" not null");
            }
            let mut has_default = false;
            if let Some(default) = &column.default_value {
                if !default.trim().is_empty() {
                    // Oracle 的 identity 列**不允许再带 default**（ORA-02254「此处不允许
                    // DEFAULT <expression>」）：两者同时在时保留 identity —— 自增是结构能力，
                    // 而同步场景里主键值本来就是显式搬过去的，default 用不上。
                    let identity_blocks_default = oracle_order && auto_clause.is_some();
                    if !identity_blocks_default {
                        if let Some(literal) = self.default_literal(default, &type_name, cross) {
                            line.push_str(" default ");
                            line.push_str(&literal);
                            has_default = true;
                        }
                    }
                }
            }
            // MySQL 的 TIMESTAMP 还有一条历史行为：`explicit_defaults_for_timestamp=OFF`
            //（5.7 的默认值）时，**除第一个 timestamp 列外**，没写 DEFAULT 的 timestamp 列会被
            // 隐式补成 `'0000-00-00 00:00:00'`；而 5.7/8.0 默认 sql_mode 含 NO_ZERO_DATE ⇒
            // 建表直接 1067「Invalid default value for 'create_time'」。
            // 真机就是这一条：源表 `create_time timestamp NULL DEFAULT NULL` 是表里第二个
            // timestamp 列，元数据回的默认值是 NULL（等于没有），我们什么都没写 ⇒ 目标端隐式
            // 零日期 ⇒ 整张表建不出来。
            // 显式写 `null`（可空列，与源表语义一致）或 `current_timestamp`（非空列总得有个值）。
            if matches!(self.kind.key(), "mysql" | "mariadb" | "tidb" | "oceanbase")
                && !has_default
                && is_timestamp_type(&type_name)
            {
                line.push_str(if column.nullable {
                    " null"
                } else {
                    " default current_timestamp"
                });
            }
            // 自增：只在主键列上写（MySQL 的 auto_increment 就要求它是键，别的库写了也无害）。
            // Oracle 那份已在类型后面写过（见上），这里只补其余方言。
            if !oracle_order {
                if let Some(clause) = auto_clause {
                    line.push(' ');
                    line.push_str(clause);
                }
            }
            // Oracle 的 `not null` 收尾（identity 与 default 都已写完，顺序见上）
            if oracle_order && !column.nullable {
                line.push_str(" not null");
            }
            // 字段注释：**内联**进列定义（只有这几个方言收这种写法）。
            // PG / Oracle / SQL Server 由 `post_create_comments` 另发 `comment on column …`；
            // SQLite / Derby 没有注释概念 —— 都留空，宁可没注释也不生成语法错的语句。
            if matches!(
                self.kind.key(),
                "mysql" | "mariadb" | "tidb" | "oceanbase" | "doris" | "starrocks" | "clickhouse"
            ) {
                if let Some(comment) = column_comments.get(&column.name.to_ascii_lowercase()) {
                    if !comment.trim().is_empty() {
                        line.push_str(" comment ");
                        line.push_str(&self.literal(comment));
                    }
                }
            }
            if column.primary_key {
                primaries.push(self.quote(&column.name));
                primary_types.push(type_name.clone());
            }
            lines.push((column.primary_key, line));
        }
        // Doris 的建表语法与 MySQL 是**两套**：没有表级 `primary key`，要写
        // `UNIQUE KEY(...)` / `DUPLICATE KEY(...)`，而且**键列必须排在列清单最前面**；
        // 还必须给 `DISTRIBUTED`，并显式写 `replication_num`（默认 3，单 BE 集群直接报
        // "replication num should be less than the number of available backends"）。
        if self.kind.key() == "doris" {
            let (mut key_lines, mut body_lines): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
            for (is_key, line) in lines {
                if is_key {
                    key_lines.push(line);
                } else {
                    body_lines.push(line);
                }
            }
            key_lines.extend(body_lines);
            let (clause, keys) = if primaries.is_empty() {
                // 没有主键：Doris 也要求一个 KEY —— 用第一列做 DUPLICATE KEY（不聚合、不去重）
                let first = self.quote(&columns.first().map(|c| c.name.clone()).unwrap_or_default());
                ("DUPLICATE KEY", vec![first])
            } else {
                (
                    "UNIQUE KEY",
                    primaries
                        .iter()
                        .filter(|name| !name.is_empty())
                        .cloned()
                        .collect(),
                )
            };
            let buckets = format!("DISTRIBUTED BY HASH({}) BUCKETS 1", keys[0]);
            // Doris 的表注释是 **COMMENT "…" 子句**（KEY 与 DISTRIBUTED 之间）——
            // 不能塞进 PROPERTIES：真机 2.1.5 直接报 Unknown properties: [comment=…]
            //（错误文案还是逗号包裹的，很迷惑）。列级内联 COMMENT 不受影响，两者并存。
            let doris_comment = match table_comment.map(str::trim).filter(|text| !text.is_empty()) {
                Some(comment) => format!("\nCOMMENT {}", self.literal(comment)),
                None => String::new(),
            };
            return format!(
                "create table {table} (\n{}\n)\n{clause}({}){doris_comment}\n{buckets}\nPROPERTIES(\"replication_num\" = \"1\");",
                key_lines.join(",\n"),
                keys.join(", ")
            );
        }
        let body: Vec<String> = lines.into_iter().map(|(_, line)| line).collect();
        // ClickHouse：`primary key` **不能写在列清单里**（那是语法错），它属于表引擎子句；
        // 而 MergeTree 系引擎要求 `ORDER BY`，所以显式给出来（没有主键就用 `tuple()`）。
        if self.kind.key() == "clickhouse" {
            let order = if primaries.is_empty() {
                "tuple()".to_string()
            } else {
                format!("({})", primaries.join(", "))
            };
            // ClickHouse 的表注释：`COMMENT '…'` 子句（跟在 ORDER BY 后）——
            // 之前这个分支漏了它，同步建出来的 CH 表表注释永远是空的（真机踩过）。
            // 列级内联 COMMENT 在上面已写（CH 收这种写法），两者并存。
            let ch_comment = match table_comment.map(str::trim).filter(|text| !text.is_empty()) {
                Some(comment) => format!(" COMMENT {}", self.literal(comment)),
                None => String::new(),
            };
            return format!(
                "create table {table} (\n{}\n)\nENGINE = MergeTree()\nORDER BY {order}{ch_comment};",
                body.join(",\n")
            );
        }
        let mut out = body;
        if !primaries.is_empty() {
            out.push(format!("  primary key ({})", primaries.join(", ")));
        }
        // 表注释：MySQL 系写在表选项里（`) comment='…';`）—— 与字段注释同为内联写法。
        // 其它方言要另发 `comment on table …`，本轮不写（见方法头注），宁可留空。
        let tail = match self.kind.key() {
            // MySQL 系：显式给 utf8mb4 —— 不写的话新表继承**库默认字符集**，实测继承出
            // latin1 后中文数据直接插不进去（Incorrect string value 1366），而源表明明是 utf8
            "mysql" | "mariadb" | "tidb" | "oceanbase" => {
                let comment = match table_comment.map(str::trim).filter(|text| !text.is_empty()) {
                    Some(comment) => format!(" comment={}", self.literal(comment)),
                    None => String::new(),
                };
                // 显式 row_format=dynamic：同步过来的宽表（几百列很常见）在 5.6/5.7 默认的
                // COMPACT 行格式下，每个长文本列内联 768 字节前缀 —— 十几个 TEXT 列就撞上 8126 的
                // 「Row size too large」；DYNAMIC 下只留 ~20 字节的指针。服务器若已是 DYNAMIC，
                // 这句写了也无害（幂等），所以不判断版本、统一带上。
                let charset = mysql_charset_for_key(&primary_types);
                format!(" row_format=dynamic default charset={charset}") + &comment
            }
            // Doris 的表级注释跟在 PROPERTIES 里（见上方 doris 分支自行拼接），这里不动
            _ => String::new(),
        };
        let mut ddl = format!("create table {table} (\n{}\n){tail};", out.join(",\n"));
        // Derby 的 JDBC **不允许语句带分号**（其它的都宽容）—— 目标是 Derby 时剥掉
        if self.kind.key() == "derby" {
            ddl = ddl.trim_end_matches(';').to_string();
        }
        ddl
    }

    /// 把**源库**的类型名翻译成本方言（目标）的类型名 —— 跨类型同步建表的入口。
    ///
    /// `source` 为空或与自身相同 ⇒ 原样返回（调用方不必自己判断要不要翻译）。
    /// 建表**之后**才能补的注释语句（`create_table_with_comments` 写不进去的那些方言）。
    ///
    /// 返回空的方言 = 注释已随建表内联写好（MySQL 系 / Doris / ClickHouse），无需追加。
    /// 返回语句的方言（当前覆盖 SQL Server）逐条执行即可 ——
    /// 之前 SQL Server 的注释完全没写（表注释/字段注释全空，真机反馈）。
    pub fn post_create_comments(
        &self,
        table: &str,
        column_comments: &std::collections::HashMap<String, String>,
        table_comment: Option<&str>,
        schema: &str,
    ) -> Vec<String> {
        self.ops()
            .post_create_comments(table, column_comments, table_comment, schema)
    }

    pub fn map_type(&self, source: Option<ConnectionKind>, source_type: &str) -> String {
        match source {
            Some(kind) if kind != self.kind => TypeFamily::parse(source_type).render(self.kind),
            _ => source_type.to_string(),
        }
    }

    /// 该类型名是不是二进制（blob / bytea / varbinary / image …）。
    ///
    /// 用途：内核的结果集只传长度不传字节，同步时必须把这类列**明确排除**，
    /// 而不是传一列 NULL 过去（那是静默的数据丢失）。
    pub fn is_binary_type(type_name: &str) -> bool {
        matches!(TypeFamily::parse(type_name), TypeFamily::Binary)
    }

    /// 该类型名是不是数值类（整型 / 定点 / 浮点）。
    ///
    /// 用途：文件导入（CSV/Excel）里的值天然都是**文本**。统一加引号在多数库上能被
    /// 隐式转换，但 Derby 明确拒绝（`Columns of type 'INTEGER' cannot hold values of
    /// type 'CHAR'`，真机导入 3 行全失败）—— 所以按目标列类型判断，数值列给裸数字。
    pub fn is_numeric_type(type_name: &str) -> bool {
        matches!(
            TypeFamily::parse(type_name),
            TypeFamily::Int { .. } | TypeFamily::Decimal { .. } | TypeFamily::Real
        )
    }

    /// 该类型名是不是「真布尔」。
    ///
    /// 用途：写入数据时决定 0/1 要不要写成 `true`/`false` ——
    /// PG / Oracle（映射后给 `boolean`）不接受整数 `0`，直接报类型不匹配。
    pub fn is_boolean_type(type_name: &str) -> bool {
        matches!(TypeFamily::parse(type_name), TypeFamily::Bool)
    }

    /// 布尔值在本方言里该写成什么。
    pub fn bool_literal(&self, truth: bool) -> &'static str {
        self.ops().bool_literal(truth)
    }

    /// 高级搜索关键字里 `_`/`%` 的 LIKE 转义尾巴（方言默认转义符差异）。
    pub fn like_escape_suffix(&self) -> &'static str {
        self.ops().like_escape_suffix()
    }

    /// 行编辑取值的日期/时间字面量翻译（Oracle/DM 的 to_date 显式格式串）。
    pub fn temporal_row_literal(&self, hint: &str, value: &str) -> Option<String> {
        self.ops().temporal_row_literal(hint, value)
    }

    /// 「授予角色」语句的形状（user-info 的角色层补行）。
    pub fn role_grant_text(
        &self,
        role_name: &str,
        role_host: &str,
        account_user: &str,
        account_host: &str,
    ) -> String {
        self.ops()
            .role_grant_text(role_name, role_host, account_user, account_host)
    }

    /// user-action 支持的授权入参 key（不在清单里的入参在动数据库之前明确拒绝）。
    pub fn grant_input_scopes(&self) -> &'static [&'static str] {
        self.ops().grant_input_scopes()
    }

    /// 全局/系统层权限名白名单（返回函数指针，喂给通用的 checked_privileges）。
    pub fn grantable_privilege_check(&self) -> fn(&str) -> bool {
        self.ops().grantable_privilege_check()
    }

    /// 对象/表层权限名白名单（与上面分开：Oracle 两本、H2 只有表级）。
    pub fn object_privilege_check(&self) -> fn(&str) -> bool {
        self.ops().object_privilege_check()
    }

    /// 撤销行的 scope 校验（privilege/owner/object/kind 都会拼进 SQL）。
    pub fn is_revocable(&self, scope: &str, privilege: &str, owner: &str, object: &str, kind: &str) -> bool {
        self.ops().is_revocable(scope, privilege, owner, object, kind)
    }

    /// 撤销校验失败提示里「缺少什么」的措辞（只影响文案）。
    pub fn revoke_missing_hint(&self) -> &'static str {
        self.ops().revoke_missing_hint()
    }

    /// 授权回显行 → 一条 `GRANT ...;` 文本（user-info 的「原始授权」列表）。
    #[allow(clippy::too_many_arguments)]
    pub fn grant_line_text(
        &self,
        scope: &str,
        privilege: &str,
        owner: &str,
        object: &str,
        account_user: &str,
        account_host: &str,
    ) -> Option<String> {
        self.ops()
            .grant_line_text(scope, privilege, owner, object, account_user, account_host)
    }

    /// 这个类型名是不是「文本类」（默认值要当字符串处理）。
    ///
    /// 按 SQL 家族的通用词根判，取「基础名」再匹配：`varchar(100)` → `varchar`、
    /// `character varying` → `character`，所以 `contains("char")` 能一并覆盖
    /// char / varchar / nvarchar / nvarchar2 / character varying 这些变体。
    pub(crate) fn is_textual_type(type_name: &str) -> bool {
        let lower = type_name.trim().to_ascii_lowercase();
        let base = lower.split(['(', ' ']).next().unwrap_or("").trim();
        base.contains("char")      // char / varchar / nvarchar / nvarchar2 / character varying
            || base.contains("text")  // text / tinytext / mediumtext / longtext
            || base.contains("clob")  // clob / nclob
            || matches!(base, "json" | "jsonb" | "enum" | "set" | "uuid")
    }

    /// 默认值：数据库回给我们的可能是表达式（`CURRENT_TIMESTAMP`）、带引号的字面量或裸值。
    ///
    /// 判据**以列类型为主**，而不是只看内容 —— 内容会骗人：
    /// 实测源库里有个 `UNIQUE_CODE varchar(100) DEFAULT '(NULL)'`（默认值就是字符串 `(NULL)`），
    /// 而数据库回给元数据时**不带引号**，于是同时命中老判据里的 `contains("NULL")`
    /// 与 `starts_with('(')`，被当成表达式原样输出成 `default (NULL)`：
    /// MySQL 5.7 上直接语法错误（`near '(NULL), LAST_UPDATETIME datetime )'`），
    /// 8.0 上则悄悄变成「表达式默认值」，语义也变了。文本列的默认值就是它的**内容**，必须加引号。
    ///
    /// 仍然原样放行的两类：已经被引号包住的（PG / Oracle 回的就是 `'abc'`）与时间函数。
    /// 其余类型保持原来的保守姿态 —— 只给明显是表达式/关键字/数字的原样放行，
    /// 猜错时宁多加一对引号（那最多是一句语法错误，比静默改变语义好）。
    ///
    /// `cross = true`（源类型 ≠ 目标类型）时另加两条规则，因为**默认值的写法本身也不可移植**：
    /// 1. 「当前时间」这类各家写法不同的（`now()` / `getdate()` / `sysdate` / `CURRENT_TIMESTAMP`）
    ///    统一翻译成**标准写法** `current_timestamp`；「当前日期」同理给 `current_date`；
    /// 2. 翻译不了的函数式默认值（`uuid()` / `newid()` / `nextval(...)` 这类各家完全不同的）
    ///    **直接省略**（返回 `None`）—— 带过去多半是目标库语法错，更糟的是悄悄换个语义。
    /// 同类型（`cross = false`）时这两条都不生效，保持原样照抄。
    fn default_literal(&self, default: &str, type_name: &str, cross: bool) -> Option<String> {
        let trimmed = default.trim();
        let upper = trimmed.to_ascii_uppercase();
        // 零日期默认值（`0000-00-00` / `0000-00-00 00:00:00`）：MySQL 5.7/8.0 的默认 sql_mode
        // 含 NO_ZERO_DATE，建表时直接报 1067「Invalid default value for 'x'」。
        // 源库能建是因为它的 sql_mode 只有 STRICT_TRANS_TABLES（真机：5.7.37）。
        // 处理办法是**省略**这个默认值 —— 这种默认本来就只是为了兼容老数据，
        // 数据是显式搬过去的，不需要它兜底；留着只会让整张表建不出来。
        // 只对时间类型生效：文本列的默认值是内容，`'0000-00-00'` 是合法字符串。
        if !Self::is_textual_type(type_name) && trimmed.trim_matches('\'').trim().starts_with("0000-00-00") {
            return None;
        }
        // 布尔列：跨类型时把 0/1 归一成目标库真认的写法（见 `boolean_literal`）。
        // 必须放在"纯数字原样放行"之前，否则会拼出 `boolean default 0` 让 PG / Oracle 直接拒。
        if cross && Self::is_boolean_type(type_name) {
            // `b'0'` / `b'1'` 是 MySQL 的位字面量，剥引号会把它弄坏，先整体比一次
            let raw = trimmed.trim_start_matches('(').trim_end_matches(')').trim();
            let lowered = raw.to_ascii_lowercase();
            let bare = raw.trim_matches('\'');
            match (lowered.as_str(), bare.to_ascii_lowercase().as_str()) {
                ("b'0'", _) | (_, "0") | (_, "false") => return Some(self.boolean_literal(false)),
                ("b'1'", _) | (_, "1") | (_, "true") => return Some(self.boolean_literal(true)),
                _ => {}
            }
        }
        // ClickHouse **没有 current_timestamp 这个函数**：裸写的 `current_timestamp` 会被当成
        // **列名** ⇒ `Code: 47 Missing columns: 'current_timestamp' … default expression and
        // column type are incompatible`，整张表建不出来（真机：源表
        // `createtime datetime DEFAULT CURRENT_TIMESTAMP` 同步到 CH 必挂）。
        // CH 的写法是 now() / today()，所以跨类型翻译出来的标准写法要在这里再翻一手。
        // 目标方言特定的「当前时间」改写（默认无；CH 没有这两个函数，见族实现）
        if let Some(rewritten) = self
            .ops()
            .rewrite_time_default(&upper, Self::is_textual_type(type_name))
        {
            return Some(rewritten);
        }
        if cross {
            // 先看**类型**：文本列的默认值是"内容"，可能恰好是叫 `now()` 的字符串 —— 那种要加引号，
            // 不能当成时间函数翻译掉。
            if !Self::is_textual_type(type_name) {
                if let Some(portable) = portable_time_default(&upper) {
                    // 标准写法也不是每个库都认（`current_date` 在 MySQL / SQL Server 上直接
                    // 语法错，见 `portable_time_literal` 的注释与实测矩阵）
                    return self.portable_time_literal(portable).map(|s| s.to_string());
                }
                if looks_like_call(&upper) {
                    return None;
                }
            }
        } else if upper.contains("CURRENT_")
            // DB2 的写法是 **`CURRENT TIMESTAMP`（空格，不是下划线）** —— 只认 `CURRENT_`
            // 会把它当普通内容加引号，拼出 `default 'CURRENT TIMESTAMP'`：重建出来是个
            // 写死的字符串，而不是随时间走的默认值。文本列除外（那的默认值就是内容）。
            || (upper.starts_with("CURRENT ") && !Self::is_textual_type(type_name))
        {
            // 同类型原样放行：MySQL 的 `CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP` 是复合写法，
            // 拆开反而丢语义，这种就必须整段照抄
            return Some(trimmed.to_string());
        }
        // 已经被引号包住的**原样放行**：`'abc'`（PG / Oracle 回的就是这样）、
        // `('abc')`（SQL Server 的默认值定义一律带外层括号）。
        // 注意必须先剥掉外层括号再看首字符 —— 否则 SQL Server 那种写法会被当成"内容"再套一层引号，
        // 变成 `'(''abc'')'`，比原来的 bug 还糟。
        let inner = trimmed.trim_start_matches('(').trim_end_matches(')').trim();
        if inner.starts_with('\'') {
            return Some(trimmed.to_string());
        }
        if Self::is_textual_type(type_name) {
            return Some(self.literal(trimmed));
        }
        // 非文本列：裸 NULL / 括号表达式 / 纯数字原样放行
        let expression = upper == "NULL" || upper.starts_with('(') || trimmed.parse::<f64>().is_ok();
        if expression {
            Some(trimmed.to_string())
        } else {
            Some(self.literal(trimmed))
        }
    }

    /// 布尔默认值的写法：`false` / `true` **不是每个方言都认**。
    ///
    /// 真机实测（`_uitest/ddl-syntax-probe.py`，都按 `列类型 default <值>` 建表验的）：
    /// - **Doris**：`boolean default false` 语法错（`errCode = 2, Syntax error in line 1`），
    ///   只收 `default 0` / `default 1`（`default "false"` 也过，但要看 sql_mode 里双引号的含义，
    ///   数字最稳）；
    /// - **SQL Server**：`bit default false` 报 `The name "false" is not permitted in this
    ///   context`，只收 `default 0` / `default 1`；
    /// - 其余：MySQL / MariaDB（tinyint(1) 与 `false` 同义）、PG / H2 / Derby / DB2 的
    ///   `boolean` 都收 `false` / `true`。
    ///
    /// 注意 Oracle 的布尔列落成 `number(1)`、ClickHouse 落成 `UInt8`、SQLite 落成 `integer`，
    /// 都不会走到这里（`is_boolean_type` 判不出来，按数字原样放行本来就是对的）。
    fn boolean_literal(&self, value: bool) -> String {
        self.ops().boolean_literal(value)
    }

    /// 把「当前时间 / 当前日期」的标准写法翻成**目标方言真能接受的默认值表达式**。
    /// 返回 `None` 表示这个方言压根表达不了 —— 只能省略默认值（列照样可空，
    /// 数据是显式搬过去的，不靠默认值兜底）。
    ///
    /// 实测矩阵见 `_uitest/ddl-time-default-check.cjs`（11 类目标库全跑）：
    /// - `current_timestamp`：MySQL / MariaDB / Doris / PG / SQL Server / Oracle / DB2 /
    ///   H2 / Derby / SQLite 都认；**ClickHouse 不认**（当列名 ⇒ `Code 47 Missing columns`，
    ///   那条在 `default_literal` 开头就翻成 `now()` 了，走不到这里）。
    /// - `current_date`：PG / Oracle / DB2 / H2 / Derby / SQLite / MariaDB 认；
    ///   **MySQL 不认**（`DEFAULT CURRENT_DATE` 语法错 —— MySQL 的列默认值只支持字面量与
    ///   CURRENT_TIMESTAMP，表达式默认值要 8.0.13 起且得写成 `(curdate())`，老库没有），
    ///   **SQL Server 不认**（T-SQL 没有 CURRENT_DATE），ClickHouse 用 `today()`。
    fn portable_time_literal(&self, portable: &'static str) -> Option<&'static str> {
        self.ops().portable_time_literal(portable)
    }

    /// 一张表的列注释：返回 `column_name` / `comment` 两列。
    ///
    /// 内置的列元数据不带注释（不是所有方言都能便宜拿到），所以治理扫描按需取一次。
    /// 拿不到就返回 `Unwritten` —— 上层会退化成「只看字段名」，而不是编一个空注释。
    pub fn table_comments(&self, table: &str) -> Meta {
        // 转义留在门面：族实现只拼 SQL，不复制一套 literal 规则
        self.ops().table_comments_sql(&self.literal(table))
    }

    /// 一张表的**完整类型文本**：返回 `column_name` / `type_text` 两列。
    ///
    /// 为什么要单独查一次：内核的列元数据只有类型名（`nvarchar`），**不带长度/精度**。
    /// 界面按 `nvarchar(50)` 这种文本拆分出「长度/精度」两个输入框 —— 只给 `nvarchar` 的后果是：
    /// 用户在那一行改任何别的属性（比如可空），生成的 ALTER 就变成
    /// `ALTER TABLE t ALTER COLUMN c nvarchar NULL`（**没有长度**）——
    /// SQL Server / MySQL / Oracle 上这要么直接报错，要么把列悄悄变成 1 个字符宽。
    /// 所以类型文本必须带参数，`nvarchar(max)` 这种边界也要如实表达。
    ///
    /// 各家写法不同（`column_type` / `udt_name` / `data_type` + 长度列），所以按方言拼。
    /// 拿不到就返回 `Unwritten`：上层保留内核给的类型名，**不编**一个长度出来。
    pub fn column_types(&self, table: &str) -> Meta {
        // 转义留在门面：族实现只拼 SQL，不复制一套 literal 规则
        self.ops().column_types_sql(&self.literal(table))
    }

    /// 库级统计（表数 + 占用空间）。键名统一成 `tables` / `sizeText`：
    /// 各家能拿到的信息本来就不同（文件型库没有「占用空间」的概念），
    /// 能拿到就给，拿不到如实给 `—`，别编一个 0。
    ///
    /// 空值函数一律用 `coalesce`：`ifnull` 是 MySQL/SQLite 的私有写法
    /// （SQL Server、Oracle、DM 上会直接报「无法识别的函数名」），
    /// `isnull` 是 SQL Server 的私有写法。`coalesce` 是 ANSI 标准，各家都认 —— 少一个方言假设。
    pub fn database_stats(&self) -> Meta {
        match self.ops().database_stats_sql() {
            Some(sql) => Meta::Sql(sql.to_string()),
            None => Meta::Unwritten,
        }
    }

    /// 实时监控面板：内核没有监控能力，Phase 3 再接（界面据 `supported:false` 隐藏）。
    pub fn monitor(&self) -> Meta {
        Meta::Unwritten
    }
}

/// 归一后的列类型族 —— **跨类型映射只认它**，不去逐家认类型字符串。
///
/// 为什么这么设计：16 种方言 × 16 种方言 = 256 组映射，逐对写必漏；
/// 先把源类型归一成「族 + 参数」（整型字节数、文本长度、精度、时区…），
/// 再由目标方言渲染一次，就只剩「解析」与「渲染」两张小表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeFamily {
    /// 整型，按字节数区分（1/2/4/8）。
    ///
    /// `unsigned` 只影响**上界**：MySQL 的 `bigint unsigned` 能到 2^64-1，i64 装不下。
    /// 跨类型到不支持 unsigned 的方言时必须**按上界升档**（bigint unsigned → 16 字节档，
    /// 落到 `numeric(20,0)` / `LARGEINT`），否则用户看到的是「Out of range value」。
    Int {
        bytes: u8,
        unsigned: bool,
    },
    /// 定点小数（precision/scale）
    Decimal {
        precision: u8,
        scale: u8,
    },
    /// 浮点
    Real,
    Bool,
    /// 文本：长度可选，`long` 表示"大文本"（text / clob / longtext 这类）
    Text {
        len: Option<u32>,
        long: bool,
    },
    Binary,
    Date,
    /// 纯时间。`fsp` = 小数秒位数：`time(6)` 的 6。
    Time {
        fsp: Option<u8>,
    },
    /// 日期时间。`tz` = 带时区，`fsp` = 小数秒位数：`datetime(6)` / `timestamp(3)` 的 6/3。
    ///
    /// 为什么非要把 `fsp` 带上：跨类型同步会走「解析成族 → 按目标渲染」，早先族里只有
    /// `tz` 一个参数，于是 MariaDB `datetime(6)` → MySQL 被渲染成 `datetime`，
    /// **微秒在建表那一刻就没了**（数据写得再准也存不下，用户看到的是「同步完时间少了微秒」）。
    DateTime {
        tz: bool,
        fsp: Option<u8>,
    },
    Json,
    Uuid,
    /// 认不出来的：跨类型时**降级为文本** —— 宁可能存下，也不要建表就语法错
    Unknown,
}

impl TypeFamily {
    /// 把一个（任何方言的）类型名解析成类型族。认不出就 `Unknown`。
    fn parse(raw: &str) -> Self {
        // 剥掉 ClickHouse 的 `Nullable(T)` 壳：可空性在 ColumnDetail.nullable 里已有，
        // 类型解析只看内层（`Nullable(String)` 直接 parse 会落 Unknown → 目标列成 CLOB）
        let raw = match raw.trim().to_ascii_lowercase().strip_prefix("nullable(") {
            Some(_inner) if raw.trim().ends_with(')') => raw
                .trim()
                .split_once('(')
                .map(|(_, rest)| rest)
                .unwrap_or(raw)
                .trim_end_matches(')')
                .trim()
                .to_string(),
            _ => raw.to_string(),
        };
        let lower = raw.trim().to_ascii_lowercase();
        // MySQL 的 `column_type` 会带 `unsigned` 后缀（`bigint(20) unsigned`）：
        // 它只抬上界，但抬到 2^64-1 就超出 i64 了，渲染时必须升档，不能当没看见
        let unsigned = lower.contains("unsigned");
        let (head, args) = match lower.split_once('(') {
            // 先按**第一个右括号**截断再切逗号：PG / Oracle 的
            // `timestamp(6) with time zone` 里括号后面还跟着两三个词，
            // 直接 trim_end_matches(')') 会把整串留下来、参数全部解析失败（fsp 就丢了）。
            Some((head, rest)) => (
                head.trim().to_string(),
                rest.split(')')
                    .next()
                    .unwrap_or(rest)
                    .split(',')
                    .filter_map(|part| part.trim().parse::<u32>().ok())
                    .collect::<Vec<_>>(),
            ),
            None => (lower.trim().to_string(), Vec::new()),
        };
        // 只留第一个词：`character varying` → character、`double precision` → double、
        // `timestamp with time zone` → timestamp（时区信息另看整串）
        let base = head.split_whitespace().next().unwrap_or("").to_string();
        let first = args.first().copied();
        let second = args.get(1).copied();
        // 小数秒位数（`datetime(6)` / `time(3)` / `DateTime64(3)`）。0 表示「不要括号」；
        // 上限 9 只是先夹住离谱值，真正的上限按目标方言在 `with_fsp` 里再夹一次。
        let fsp = first.filter(|n| *n > 0).map(|n| n.min(9) as u8);
        match base.as_str() {
            // MySQL 的 tinyint(1) 是"布尔"的通行约定，跨类型时按布尔走更贴合语义
            "tinyint" | "int1" => {
                if first == Some(1) {
                    Self::Bool
                } else {
                    Self::Int { bytes: 1, unsigned }
                }
            }
            "smallint" | "int2" => Self::Int { bytes: 2, unsigned },
            "mediumint" | "int3" => Self::Int { bytes: 4, unsigned },
            "int" | "integer" | "int4" | "serial" => Self::Int { bytes: 4, unsigned },
            "bigint" | "int8" | "bigserial" => Self::Int { bytes: 8, unsigned },
            "number" | "numeric" | "decimal" | "dec" => {
                let precision = first.unwrap_or(38).clamp(1, 38) as u8;
                let scale = second.unwrap_or(0).min(precision as u32) as u8;
                Self::Decimal { precision, scale }
            }
            "float" | "double" | "real" | "binary_double" | "binary_float" | "float4" | "float8" => {
                Self::Real
            }
            "bool" | "boolean" => Self::Bool,
            // bit(1) = 布尔；bit(n>1)（MySQL 的位域）当二进制
            "bit" => {
                if first.unwrap_or(1) <= 1 {
                    Self::Bool
                } else {
                    Self::Binary
                }
            }
            "char" | "nchar" | "varchar" | "varchar2" | "nvarchar" | "nvarchar2" | "character" => {
                Self::Text {
                    len: first,
                    long: false,
                }
            }
            "text" | "tinytext" | "mediumtext" | "longtext" | "clob" | "nclob" | "longvarchar" => {
                Self::Text {
                    len: None,
                    long: true,
                }
            }
            "binary" | "varbinary" | "blob" | "tinyblob" | "mediumblob" | "longblob" | "bytea" | "image"
            | "raw" => Self::Binary,
            "date" => Self::Date,
            "time" | "timetz" => Self::Time { fsp },
            "datetime" | "datetime2" | "smalldatetime" => Self::DateTime { tz: false, fsp },
            "timestamp" | "timestamptz" | "datetimeoffset" => Self::DateTime {
                tz: lower.contains("tz") || lower.contains("offset") || lower.contains("with time zone"),
                fsp,
            },
            "json" | "jsonb" => Self::Json,
            "uuid" | "uniqueidentifier" => Self::Uuid,
            // ClickHouse 的大写族（`UInt64` / `String` / `DateTime64` / `FixedString(n)` / `Decimal64(s)`）
            "string" => Self::Text {
                len: None,
                long: true,
            },
            "fixedstring" => Self::Text {
                len: first,
                long: false,
            },
            // ClickHouse 的 `DateTime64(3)` / `DateTime64(3, 'UTC')`：括号里第一个数是小数秒位数
            "datetime64" => Self::DateTime { tz: false, fsp },
            // ClickHouse 的 `Decimal32(S)` / `Decimal64(S)` / `Decimal128(S)` / `Decimal256(S)`：
            // 括号里是 **scale**，精度由位数定（9 / 18 / 38 / 76）。
            // 原先一律写 38 ⇒ `Decimal64(2)` 被渲染成 `decimal(38,2)`：语义没错，但 InnoDB 每列
            // 内联从 9 字节涨到 17 字节 —— 宽表同步时是「Row size too large (> 8126)」的帮凶
            //（用户实测：ClickHouse → MySQL 传输，几百列的表建表被拒）。按位数给准精度。
            "decimal32" => Self::Decimal {
                precision: 9,
                scale: first.unwrap_or(0).min(9) as u8,
            },
            "decimal64" => Self::Decimal {
                precision: 18,
                scale: first.unwrap_or(0).min(18) as u8,
            },
            "decimal128" => Self::Decimal {
                precision: 38,
                scale: first.unwrap_or(0).min(38) as u8,
            },
            "decimal256" => Self::Decimal {
                precision: 76,
                scale: first.unwrap_or(0).min(76) as u8,
            },
            // ClickHouse 的无符号整型：带上 unsigned 位，渲染时才有 UInt64 / numeric(20,0) 之分
            "uint8" => Self::Int {
                bytes: 1,
                unsigned: true,
            },
            "uint16" => Self::Int {
                bytes: 2,
                unsigned: true,
            },
            "uint32" => Self::Int {
                bytes: 4,
                unsigned: true,
            },
            "uint64" => Self::Int {
                bytes: 8,
                unsigned: true,
            },
            // ClickHouse 的整型/浮点族（Int32 / Float64 …）：之前没认，落到 Unknown，
            // 目标 Derby/H2 这类按 Unknown→CLOB 渲染的方言，整列全成 CLOB（真机踩过）
            "int16" => Self::Int {
                bytes: 2,
                unsigned: false,
            },
            "int32" => Self::Int {
                bytes: 4,
                unsigned: false,
            },
            "int64" => Self::Int {
                bytes: 8,
                unsigned: false,
            },
            "float32" => Self::Real,
            "float64" => Self::Real,
            _ => Self::Unknown,
        }
    }

    /// 按目标方言渲染成类型名。
    fn render(self, target: ConnectionKind) -> String {
        let key = target.key();
        // Doris 与 MySQL 共用 JDBC 协议，**但类型系统是另一套**：没有 longtext / blob / time / json，
        // 大文本与二进制一律 STRING。这几个特例单独处理，其余（整型/定点/浮点/日期时间）两边一致，
        // 直接借用 MySQL 的渲染，避免维护两份。
        if key == "doris" || key == "starrocks" {
            // StarRocks 与 Doris 的引擎行为同源：块内 helper（int_type 等）按 doris key 走
            let key = if key == "starrocks" { "doris" } else { key };
            return match self {
                Self::Bool => "BOOLEAN".to_string(),
                Self::Uuid => "VARCHAR(36)".to_string(),
                Self::Json => "STRING".to_string(),
                Self::Binary => "STRING".to_string(),
                Self::Time { .. } => "STRING".to_string(),
                Self::Unknown => "STRING".to_string(),
                Self::Text { len, long } => match len {
                    Some(size) if !long => format!("VARCHAR({})", size.min(65533)),
                    _ => "STRING".to_string(),
                },
                // `DATETIME(p)` 只有 Doris 1.2+ 才认，老版本建表直接失败 —— 跨类型过来时
                // 保守落成不带精度的 `datetime`（同类型同步照抄原文，不受影响）
                Self::DateTime { .. } => "datetime".to_string(),
                // Doris 没有 unsigned：按上界升档（8 字节无符号要 LARGEINT，128 位）
                Self::Int { bytes, unsigned } => Self::int_type(bytes, unsigned, key),
                other => other.render(ConnectionKind::Mysql),
            };
        }
        // TiDB 的类型系统在语法层与 MySQL 完全一致（json / longtext / longblob /
        // decimal(65) / time(n) / datetime(n) 全部认）。render 的各分支按目标 key
        // 手写、漏 tidb 会落进 generic 兜底 —— json 被渲染成 clob，TiDB 建表
        // 直接语法错（真机同步踩过）。归一成 mysql 一并覆盖全部分支；
        // Doris 的特判在上面已 return，不受影响。
        // 语法层兼容族归一（协议兼容库的 kind key 回退到宿主方言 key）：
        // MySQL 系（mariadb / tidb / oceanbase）与 PG 系（kingbase / opengauss）。
        // 新加一种兼容库在这里加一行即可，下面所有分支按宿主 key 命中。
        let key = match key {
            "tidb" | "mariadb" | "oceanbase" => "mysql",
            "kingbase" | "opengauss" => "postgresql",
            _ => key,
        };
        match self {
            Self::Int { bytes, unsigned } => Self::int_type(bytes, unsigned, key),
            Self::Decimal { precision, scale } => {
                let precision = precision.max(1);
                match key {
                    "postgresql" | "kingbase" => format!("numeric({precision},{scale})"),
                    "oracle" | "dm" => format!("number({precision},{scale})"),
                    "clickhouse" => format!("Decimal({precision},{scale})"),
                    "sqlite" => "numeric".to_string(),
                    // MySQL 系精度上限 65、Doris 38：上面 ClickHouse 的 `Decimal256(s)` 解析出来是
                    // 76 位，不夹住目标端会直接报「Too big precision」。夹精度时 scale 一并压进范围。
                    "mysql" | "mariadb" => {
                        let p = precision.min(65);
                        format!("decimal({p},{})", scale.min(p))
                    }
                    "doris" => {
                        let p = precision.min(38);
                        format!("decimal({p},{})", scale.min(p))
                    }
                    _ => format!("decimal({precision},{scale})"),
                }
            }
            Self::Real => match key {
                "mysql" | "mariadb" | "doris" => "double".to_string(),
                "oracle" | "dm" => "binary_double".to_string(),
                "clickhouse" => "Float64".to_string(),
                "sqlserver" => "float".to_string(),
                "sqlite" => "real".to_string(),
                _ => "double precision".to_string(),
            },
            Self::Bool => match key {
                "mysql" | "mariadb" | "doris" => "tinyint(1)".to_string(),
                "oracle" | "dm" => "number(1)".to_string(),
                "sqlserver" => "bit".to_string(),
                "clickhouse" => "UInt8".to_string(),
                "sqlite" => "integer".to_string(),
                _ => "boolean".to_string(),
            },
            Self::Text { len, long } => {
                let wide = long || len.map(|n| n > 4000).unwrap_or(false);
                match key {
                    "mysql" | "mariadb" | "doris" => {
                        if wide {
                            "longtext".to_string()
                        } else {
                            format!("varchar({})", len.unwrap_or(255))
                        }
                    }
                    "oracle" | "dm" => {
                        if wide {
                            "clob".to_string()
                        } else {
                            format!("varchar2({})", len.unwrap_or(255).min(4000))
                        }
                    }
                    "sqlserver" => {
                        if wide {
                            "nvarchar(max)".to_string()
                        } else {
                            format!("nvarchar({})", len.unwrap_or(255))
                        }
                    }
                    "clickhouse" => "String".to_string(),
                    // Derby 没有 text 类型：大文本用 CLOB，短的用 varchar
                    "derby" => {
                        if wide {
                            "clob".to_string()
                        } else {
                            format!("varchar({})", len.unwrap_or(255))
                        }
                    }
                    "sqlite" => "text".to_string(),
                    _ => {
                        if wide {
                            "text".to_string()
                        } else {
                            format!("varchar({})", len.unwrap_or(255))
                        }
                    }
                }
            }
            Self::Binary => match key {
                "mysql" | "mariadb" | "doris" => "longblob".to_string(),
                "postgresql" | "kingbase" => "bytea".to_string(),
                "sqlserver" => "varbinary(max)".to_string(),
                "oracle" | "dm" => "blob".to_string(),
                "clickhouse" => "String".to_string(),
                _ => "blob".to_string(),
            },
            Self::Date => "date".to_string(),
            Self::Time { fsp } => match key {
                // Oracle 没有纯时间类型，用 date（它本身含时分秒）
                "oracle" | "dm" => "date".to_string(),
                "clickhouse" => "String".to_string(),
                // 这几家的 `time(n)` 是标准写法；其余方言不带精度（宁可不带，也不要语法错）
                "mysql" | "mariadb" | "doris" | "postgresql" | "kingbase" | "sqlserver" => {
                    Self::with_fsp("time", fsp, key)
                }
                _ => "time".to_string(),
            },
            Self::DateTime { tz, fsp } => match key {
                "mysql" | "mariadb" | "doris" => Self::with_fsp("datetime", fsp, key),
                "postgresql" | "kingbase" => {
                    Self::with_fsp(if tz { "timestamptz" } else { "timestamp" }, fsp, key)
                }
                "oracle" | "dm" => {
                    // Oracle 的精度写在 WITH TIME ZONE **前面**：`timestamp(6) with time zone`
                    let base = Self::with_fsp("timestamp", fsp, key);
                    if tz {
                        format!("{base} with time zone")
                    } else {
                        base
                    }
                }
                "sqlserver" => Self::with_fsp(if tz { "datetimeoffset" } else { "datetime2" }, fsp, key),
                // ClickHouse 的小数秒是另一个类型名；0 位（或没写）就是普通 DateTime
                "clickhouse" => match fsp {
                    Some(n) => format!("DateTime64({})", n.min(9)),
                    None => "DateTime".to_string(),
                },
                "sqlite" => "datetime".to_string(),
                _ => "timestamp".to_string(),
            },
            Self::Json => match key {
                "mysql" | "mariadb" => "json".to_string(),
                "postgresql" | "kingbase" => "jsonb".to_string(),
                "sqlserver" => "nvarchar(max)".to_string(),
                "clickhouse" => "String".to_string(),
                "sqlite" => "text".to_string(),
                _ => "clob".to_string(),
            },
            Self::Uuid => match key {
                "postgresql" | "kingbase" => "uuid".to_string(),
                "sqlserver" => "uniqueidentifier".to_string(),
                _ => "char(36)".to_string(),
            },
            Self::Unknown => match key {
                "mysql" | "mariadb" | "doris" => "longtext".to_string(),
                "postgresql" | "kingbase" | "sqlite" => "text".to_string(),
                "sqlserver" => "nvarchar(max)".to_string(),
                "clickhouse" => "String".to_string(),
                _ => "clob".to_string(),
            },
        }
    }

    /// 整型按目标方言渲染。
    ///
    /// `unsigned` 只有 MySQL 系原生支持，其余方言按**上界**升档：
    /// `tinyint unsigned`(255) → smallint、`smallint unsigned`(65535) → int、
    /// `int unsigned`(4.29e9) → bigint、`bigint unsigned`(1.8e19) → 16 字节档
    /// （`numeric(20,0)` / `decimal(20,0)` / Doris 的 `LARGEINT` / ClickHouse 的 `UInt64`）。
    /// 不升档的后果不是「精度差一点」，而是目标端直接 `Out of range value`。
    fn int_type(bytes: u8, unsigned: bool, target_key: &str) -> String {
        // MySQL 系原样带 unsigned（同类型同步本来也照抄原文，这里只管跨类型）
        if matches!(target_key, "mysql" | "mariadb") {
            let base = match bytes {
                1 => "tinyint",
                2 => "smallint",
                4 => "int",
                _ => "bigint",
            };
            return if unsigned {
                format!("{base} unsigned")
            } else {
                base.to_string()
            };
        }
        // ClickHouse 本来就是 UIntN / IntN 两套名字，不用升档
        if target_key == "clickhouse" {
            return match (bytes, unsigned) {
                (1, false) => "Int8",
                (1, true) => "UInt8",
                (2, false) => "Int16",
                (2, true) => "UInt16",
                (4, false) => "Int32",
                (4, true) => "UInt32",
                (_, false) => "Int64",
                (_, true) => "UInt64",
            }
            .to_string();
        }
        // 其余方言：无符号按上界升一档，8 字节无符号升到 16 字节档
        let effective = match (bytes, unsigned) {
            (1, true) => 2,
            (2, true) => 4,
            (4, true) => 8,
            (8, true) => 16,
            _ => bytes,
        };
        match target_key {
            "oracle" | "dm" => match effective {
                1 => "number(3)",
                2 => "number(5)",
                4 => "number(10)",
                8 => "number(19)",
                _ => "number(20)",
            }
            .to_string(),
            "doris" => match effective {
                1 => "tinyint",
                2 => "smallint",
                4 => "int",
                8 => "bigint",
                // Doris 的 BIGINT 装不下 2^64-1；LARGEINT 是 128 位有符号
                _ => "largeint",
            }
            .to_string(),
            "postgresql" | "kingbase" => match effective {
                1 | 2 => "smallint",
                4 => "integer",
                8 => "bigint",
                _ => "numeric(20,0)",
            }
            .to_string(),
            "sqlserver" => match effective {
                1 => "tinyint",
                2 => "smallint",
                4 => "int",
                8 => "bigint",
                _ => "decimal(20,0)",
            }
            .to_string(),
            // SQLite 只有 64 位整数：2^64-1 在这里没有精确表示，
            // 保持 integer（不做“看起来像支持”的映射）
            "sqlite" => "integer".to_string(),
            // Derby / DB2 没有 TINYINT，1 字节一律落到 SMALLINT
            "derby" | "db2" => match effective {
                1 | 2 => "smallint",
                4 => "int",
                8 => "bigint",
                _ => "decimal(20,0)",
            }
            .to_string(),
            _ => match effective {
                1 => "tinyint",
                2 => "smallint",
                4 => "int",
                8 => "bigint",
                _ => "decimal(20,0)",
            }
            .to_string(),
        }
    }

    /// 拼带小数秒位数的类型名：`datetime(6)` / `time(3)` / `datetime2(7)`。
    ///
    /// 位数按**目标**方言夹上限：MySQL 最多 6、SQL Server 7、Oracle 9 —— 越界不是
    /// 「精度更高」而是建表直接语法错。没有位数（或 0）就原样返回，与加这个参数之前一致。
    fn with_fsp(base: &str, fsp: Option<u8>, target_key: &str) -> String {
        let Some(fsp) = fsp else {
            return base.to_string();
        };
        let cap = match target_key {
            "mysql" | "mariadb" | "doris" | "postgresql" | "kingbase" => 6,
            "sqlserver" => 7,
            "oracle" | "dm" => 9,
            _ => 6,
        };
        format!("{base}({})", fsp.min(cap))
    }
}

/// 自增列的写法：按目标方言各写一套；认不出的方言返回 `None`
/// （宁可不自增，也不要造出一句语法错）。
fn auto_increment_clause(kind: ConnectionKind, family: TypeFamily) -> Option<&'static str> {
    // 只有整数（或 scale=0 的定点）才谈得上自增
    let integer =
        matches!(family, TypeFamily::Int { .. }) || matches!(family, TypeFamily::Decimal { scale: 0, .. });
    if !integer {
        return None;
    }
    match kind.key() {
        "mysql" | "mariadb" | "tidb" | "oceanbase" => Some("auto_increment"),
        // Doris 是 2.1 才支持 AUTO_INCREMENT，还有"必须是键列 + 开关"等前置条件 ——
        // 写上去风险大于收益（老版本直接语法错），不写。
        "sqlserver" => Some("identity(1,1)"),
        "postgresql" | "kingbase" | "opengauss" | "oracle" | "dm" | "h2" | "db2" | "derby" => {
            Some("generated by default as identity")
        }
        // SQLite 只认 `integer primary key autoincrement`（表级主键写法不同）、
        // ClickHouse 没有自增概念 —— 这两个不写
        _ => None,
    }
}

/// 各家「当前时间 / 当前日期」默认值的写法 → 一个各家都认的标准写法。
fn portable_time_default(upper: &str) -> Option<&'static str> {
    if upper.contains("CURRENT_TIMESTAMP")
        || upper.contains("NOW()")
        || upper.contains("GETDATE()")
        || upper.contains("SYSDATE")
        || upper.contains("LOCALTIMESTAMP")
        || upper.contains("SYSTIMESTAMP")
    {
        return Some("current_timestamp");
    }
    if upper.contains("CURRENT_DATE") || upper.contains("CURDATE()") {
        return Some("current_date");
    }
    None
}

/// 像函数调用 / 序列取值吗？（跨类型时这类默认值一律省略，不猜）
fn looks_like_call(upper: &str) -> bool {
    upper.contains('(') || upper.contains("NEXTVAL")
}

/// 无界文本/二进制类型吗？——这些类型**不能做 Doris 的键列**（STRING/TEXT 无长度，
/// Doris 的 KEY 要求有界），且 `varchar`/`char` 不带长度括号时也无法直接做键列。
fn is_unbounded_text_type(type_name: &str) -> bool {
    let t = type_name.trim().to_ascii_lowercase();
    let base = t.split('(').next().unwrap_or("").trim();
    matches!(
        base,
        "string"
            | "text"
            | "longtext"
            | "mediumtext"
            | "tinytext"
            | "clob"
            | "nclob"
            | "blob"
            | "tinyblob"
            | "mediumblob"
            | "longblob"
            | "image"
            | "bytea"
    ) || ((base == "varchar" || base == "char" || base == "character varying" || base == "nvarchar")
        && !t.contains('('))
}

/// 把「无界长度哨兵」收敛成目标方言能接受的无界写法。
///
/// 长度 **2147483647**（i32::MAX）与 **4294967295**（u32::MAX）都不是真实长度，而是
/// JDBC `ResultSetMetaData.getPrecision()` 与各库 `information_schema` 对
/// `text / ntext / longtext / varchar(max)` 这类**无界**类型给的占位值。
/// 原样拼进建表语句按方言报不同的语法错（SQL Server 2715、MySQL 1210、Oracle ORA-00907），
/// 表现是「目标表建不出来 ⇒ 整表漏同步」，而进度卡的失败数还是 0。
///
/// 只收敛**确实是字符/二进制大对象**的类型：`decimal(2147483647,0)` 之类原样留给数据库报错，
/// 猜错了比报错更难排查。
fn unbounded_type_of(type_name: &str, kind: &str) -> Option<String> {
    /// 真实长度不可能到 2^31 量级：任何超过它的长度都是哨兵值。
    const SENTINEL_MIN: u64 = 2_147_483_647;
    let lower = type_name.trim().to_ascii_lowercase();
    let (raw_base, rest) = lower.split_once('(')?;
    let len = rest
        .split(')')
        .next()
        .unwrap_or("")
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .parse::<u64>()
        .ok()?;
    if len < SENTINEL_MIN {
        return None;
    }
    let base = raw_base.trim();
    // SQL Server 的旧大对象类型：**裸类型名才是合法写法**（`text(2147483647)` 非法，
    // 而 `nvarchar(max)` 会把非 Unicode 数据转一次码，能不改就不改）。
    if matches!(base, "text" | "ntext" | "image") {
        return Some(base.to_string());
    }
    let textual = matches!(
        base,
        "char"
            | "character"
            | "character varying"
            | "varchar"
            | "varchar2"
            | "nvarchar"
            | "nvarchar2"
            | "nchar"
            | "clob"
            | "nclob"
            | "string"
    );
    let binary = matches!(base, "binary" | "varbinary" | "blob" | "bytea" | "raw");
    if !textual && !binary {
        return None;
    }
    let wide = match kind {
        "mysql" | "mariadb" | "doris" => {
            if binary {
                "longblob"
            } else {
                "longtext"
            }
        }
        "postgresql" | "kingbase" => {
            if binary {
                "bytea"
            } else {
                "text"
            }
        }
        "oracle" | "dm" => {
            if binary {
                "blob"
            } else {
                "clob"
            }
        }
        "sqlserver" => {
            if binary {
                "varbinary(max)"
            } else {
                "nvarchar(max)"
            }
        }
        "db2" | "h2" | "derby" => {
            if binary {
                "blob"
            } else {
                "clob"
            }
        }
        "clickhouse" => "String",
        "sqlite" => {
            if binary {
                "blob"
            } else {
                "text"
            }
        }
        _ => {
            if binary {
                "blob"
            } else {
                "text"
            }
        }
    };
    Some(wide.to_string())
}

/// InnoDB（DYNAMIC 行格式）**单个索引**的字节上限。
const MYSQL_MAX_KEY_BYTES: usize = 3072;

/// 类型是不是 MySQL 的 `timestamp`（**不含** `datetime`：后者没有隐式零日期默认值那一套）。
fn is_timestamp_type(type_name: &str) -> bool {
    type_name
        .trim()
        .to_ascii_lowercase()
        .split(['(', ' '])
        .next()
        .unwrap_or("")
        .trim()
        == "timestamp"
}

/// 主键表在 MySQL 上该用哪个字符集建 —— 唯一目的是**让主键装得进 3072 字节**。
///
/// 背景（真机）：源表主键 `varchar(10) + varchar(255) × 3` = 775 字符，
/// 源库是 utf8（3 字节/字符）⇒ 2325 字节，合法；而建表语句一律写
/// `default charset=utf8mb4`（4 字节/字符）⇒ 3100 字节 > 3072，目标端直接报
/// `ERROR 1071 Specified key was too long; max key length is 3072 bytes`，
/// 整张表连结构都建不出来（同步结果里是一行「建表失败」）。
///
/// 取舍：**只有 utf8mb4 装不下、utf8 装得下时才降级**（其余情况仍 utf8mb4，
/// 与以前完全一致）。降级不丢数据 —— 能存进 3 字节字符集的键值本来就不含 4 字节
/// 字符（emoji 等），否则源库自己也存不进去。
/// 两个字符集都装不下（例如 latin1 源的 `varchar(1200)` 主键）时保持 utf8mb4：
/// 这种表已经超过 MySQL 的索引上限，让数据库如实报错，比悄悄把主键改成前缀索引安全。
fn mysql_charset_for_key(primary_types: &[String]) -> &'static str {
    let bytes = |per_char: usize| {
        primary_types
            .iter()
            .map(|ty| index_bytes(ty, per_char))
            .sum::<usize>()
    };
    if bytes(4) <= MYSQL_MAX_KEY_BYTES {
        "utf8mb4"
    } else if bytes(3) <= MYSQL_MAX_KEY_BYTES {
        "utf8"
    } else {
        "utf8mb4"
    }
}

/// 一列在 MySQL 索引里占多少字节（`per_char` = 字符集的字节/字符）。
///
/// 只求「够判断有没有超过 3072」，数值/时间类按固定开销近似即可。
fn index_bytes(type_name: &str, per_char: usize) -> usize {
    let lower = type_name.trim().to_ascii_lowercase();
    // `character varying(64)` 这种带空格的类型名取到 `character` 就够（文本族判据靠 contains）
    let base = lower.split(['(', ' ']).next().unwrap_or("").trim().to_string();
    let len = lower
        .split_once('(')
        .and_then(|(_, rest)| rest.split([')', ',']).next())
        .and_then(|n| n.trim().parse::<usize>().ok());
    // 文本族：字符长度 × 每个字符的字节数
    if base.contains("char") || base.contains("text") || base.contains("clob") {
        return len.unwrap_or(255) * per_char;
    }
    // 二进制族：长度就是字节数
    if base.contains("blob") || base.contains("binary") {
        return len.unwrap_or(255);
    }
    match base.as_str() {
        "tinyint" => 1,
        "smallint" => 2,
        "mediumint" => 3,
        "int" | "integer" | "float" => 4,
        "date" => 3,
        "year" => 1,
        "bigint" | "double" | "real" | "datetime" | "timestamp" | "time" => 8,
        _ => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(kind: ConnectionKind, name: &str) -> String {
        Dialect::new(kind).quote(name)
    }

    fn qx(kind: ConnectionKind, name: &str) -> String {
        Dialect::new(kind).quote_exact(name)
    }

    /// `quote_exact` 只加引号、**不动大小写**：它服务的是目录元数据里的真实名字。
    ///
    /// 守着豫园 Oracle 11g 的一次真机故障：列是小写带引号建的 `"day"`，表头点排序后
    /// 后端用 `quote()` 拼出 `order by day`，服务端折成 `DAY` ⇒
    /// `ORA-00904: "DAY": 标识符无效`。同样的坑在关键字搜索、高级搜索、
    /// 就地编辑、导出的列清单里都存在。
    #[test]
    fn quote_exact_保留大小写与加引号() {
        // 上折方言：小写必须留在引号里，大写同样逐字加引号
        assert_eq!(qx(ConnectionKind::Oracle, "day"), "\"day\"");
        assert_eq!(qx(ConnectionKind::Oracle, "DAY"), "\"DAY\"");
        assert_eq!(qx(ConnectionKind::Oracle, "Name"), "\"Name\"");
        // quote() 的既有行为不变（简单名裸写，交给服务端折叠）
        assert_eq!(q(ConnectionKind::Oracle, "day"), "day");
        assert_eq!(q(ConnectionKind::Oracle, "DAY"), "DAY");

        // 各家的引号形态
        assert_eq!(qx(ConnectionKind::Mysql, "day"), "`day`");
        assert_eq!(qx(ConnectionKind::Sqlserver, "day"), "[day]");
        assert_eq!(qx(ConnectionKind::Postgresql, "Name"), "\"Name\"");

        // 模式限定的名字逐段加引号（schema.table）
        assert_eq!(qx(ConnectionKind::Postgresql, "s.T"), "\"s\".\"T\"");

        // 名字里自带引号要翻倍，别把语句截断
        assert_eq!(qx(ConnectionKind::Oracle, "a\"b"), "\"a\"\"b\"");
        assert_eq!(qx(ConnectionKind::Mysql, "a`b"), "`a``b`");
        assert_eq!(qx(ConnectionKind::Sqlserver, "a]b"), "[a]]b]");
    }

    fn col(name: &str, ty: &str, default: Option<&str>) -> ColumnDetail {
        ColumnDetail {
            name: name.to_string(),
            type_name: Some(ty.to_string()),
            nullable: true,
            primary_key: false,
            default_value: default.map(str::to_string),
            auto_increment: false,
        }
    }

    /// 主键 + 自增的整数列（验跨类型建表时 auto_increment / identity / serial 有没有跟上）
    fn auto_col(name: &str, ty: &str) -> ColumnDetail {
        ColumnDetail {
            name: name.to_string(),
            type_name: Some(ty.to_string()),
            nullable: false,
            primary_key: true,
            default_value: None,
            auto_increment: true,
        }
    }

    /// 跨类型建表必须**保留小数秒精度**。
    ///
    /// 守着一次真实故障：MariaDB 的 `datetime(6)` 同步到 MySQL（库类型名不同 ⇒ 走跨类型
    /// 翻译）被渲染成 `datetime`，目标列只能存到秒 —— 同步「成功」，微秒全丢，而且是在
    /// **建表那一刻**就丢了。族里必须带上 `fsp`，各目标还要按自己的上限夹。
    #[test]
    fn 跨类型保留小数秒精度() {
        let mysql = Dialect::new(ConnectionKind::Mysql);
        let ddl = mysql.create_table_from_columns(
            "t",
            &[col("ts", "datetime(6)", None), col("tm", "time(3)", None)],
            Some(ConnectionKind::Mariadb),
        );
        assert!(ddl.contains("datetime(6)"), "实际：{ddl}");
        assert!(ddl.contains("time(3)"), "实际：{ddl}");

        // ClickHouse 是另一套写法：DateTime64(3) → datetime(3)
        let ddl = mysql.create_table_from_columns(
            "t",
            &[col("ts", "DateTime64(3)", None)],
            Some(ConnectionKind::Clickhouse),
        );
        assert!(ddl.contains("datetime(3)"), "实际：{ddl}");

        // 目标上限要夹住：SQL Server 的 datetime2 只到 7 位、MySQL 只到 6 位
        let mssql = Dialect::new(ConnectionKind::Sqlserver);
        let ddl = mssql.create_table_from_columns(
            "t",
            &[col("ts", "timestamp(9)", None)],
            Some(ConnectionKind::Postgresql),
        );
        assert!(ddl.contains("datetime2(7)"), "实际：{ddl}");
        let ddl = mysql.create_table_from_columns(
            "t",
            &[col("ts", "timestamp(9)", None)],
            Some(ConnectionKind::Postgresql),
        );
        assert!(ddl.contains("datetime(6)"), "实际：{ddl}");

        // Doris 老版本不认 `datetime(6)`：跨类型过来保守落成 datetime（宁可不带精度，
        // 也不要建表就失败）
        let doris = Dialect::new(ConnectionKind::Doris);
        let ddl = doris.create_table_from_columns(
            "t",
            &[col("ts", "datetime(6)", None)],
            Some(ConnectionKind::Mariadb),
        );
        assert!(ddl.contains("ts datetime"), "实际：{ddl}");
        assert!(!ddl.contains("datetime(6)"), "实际：{ddl}");

        // Oracle 的精度写在 WITH TIME ZONE **前面**（写后面是语法错）
        let oracle = Dialect::new(ConnectionKind::Oracle);
        let ddl = oracle.create_table_from_columns(
            "t",
            &[col("ts", "timestamp(6) with time zone", None)],
            Some(ConnectionKind::Postgresql),
        );
        assert!(ddl.contains("timestamp(6) with time zone"), "实际：{ddl}");
    }

    /// MySQL 的 `column_type` 经常带后缀（`decimal(10,2) unsigned` /
    /// `timestamp(6) with time zone`）：括号里的参数不能被后缀带跑。
    ///
    /// 以前参数解析是 `trim_end_matches(')')` 之后再切逗号，后缀还在时整段解析失败 ——
    /// `decimal(10,2) unsigned` 只认出精度 10、scale 丢成 0，跨类型同步变成 `numeric(10,0)`。
    #[test]
    fn 类型后缀不影响括号里的参数() {
        let pg = Dialect::new(ConnectionKind::Postgresql);
        let ddl = pg.create_table_from_columns(
            "t",
            &[col("amt", "decimal(10,2) unsigned", None)],
            Some(ConnectionKind::Mysql),
        );
        assert!(ddl.contains("numeric(10,2)"), "实际：{ddl}");
    }

    /// 无符号整型跨方言要**按上界升档**。
    ///
    /// `bigint unsigned` 能到 2^64-1，i64 装不下：老实现渲染成 `bigint`，写入时目标端
    /// 直接 `Out of range value`（MySQL→PG / Doris 都有用户踩过）。
    #[test]
    fn 无符号整型跨方言按上界升档() {
        let one = |ty: &str| vec![col("v", ty, None)];
        // 同是 MySQL 系：照写 unsigned
        let ddl = Dialect::new(ConnectionKind::Mysql).create_table_from_columns(
            "t",
            &one("bigint(20) unsigned"),
            Some(ConnectionKind::Mariadb),
        );
        assert!(ddl.contains("bigint unsigned"), "实际：{ddl}");

        let pg = Dialect::new(ConnectionKind::Postgresql);
        // 8 字节无符号 → 16 字节档
        assert!(pg
            .create_table_from_columns("t", &one("bigint unsigned"), Some(ConnectionKind::Mysql))
            .contains("numeric(20,0)"));
        // 4 字节无符号 → bigint（PG 的 integer 只到 2^31-1）
        assert!(pg
            .create_table_from_columns("t", &one("int unsigned"), Some(ConnectionKind::Mysql))
            .contains("bigint"));
        // 2 字节无符号 → integer
        assert!(pg
            .create_table_from_columns("t", &one("smallint unsigned"), Some(ConnectionKind::Mysql))
            .contains("integer"));
        // 1 字节无符号 → smallint
        assert!(pg
            .create_table_from_columns("t", &one("tinyint unsigned"), Some(ConnectionKind::Mysql))
            .contains("smallint"));

        // ClickHouse 原生 UIntN，不用升档
        let ch = Dialect::new(ConnectionKind::Clickhouse);
        assert!(ch
            .create_table_from_columns("t", &one("bigint unsigned"), Some(ConnectionKind::Mysql))
            .contains("UInt64"));
        // Doris 没有 unsigned，用 128 位的 LARGEINT 接住
        let doris = Dialect::new(ConnectionKind::Doris);
        assert!(doris
            .create_table_from_columns("t", &one("bigint unsigned"), Some(ConnectionKind::Mysql))
            .contains("largeint"));
        // SQL Server 的 8 字节无符号 → decimal(20,0)
        let mssql = Dialect::new(ConnectionKind::Sqlserver);
        assert!(mssql
            .create_table_from_columns("t", &one("bigint unsigned"), Some(ConnectionKind::Mysql))
            .contains("decimal(20,0)"));
    }

    /// 同类型同步**照抄原文**：`datetime(6)` 进来还是 `datetime(6)`，一个字符都不动。
    #[test]
    fn 同类型建表不改写类型文本() {
        let ddl = Dialect::new(ConnectionKind::Mysql).create_table_from_columns(
            "t",
            &[col("ts", "datetime(6)", None), col("d", "datetime", None)],
            Some(ConnectionKind::Mysql),
        );
        assert!(ddl.contains("datetime(6)"), "实际：{ddl}");
        // 没有小数秒的那列不能被「顺手补成 (6)」
        assert!(ddl.contains("d datetime"), "实际：{ddl}");
        assert!(!ddl.contains("d datetime(6)"), "实际：{ddl}");
    }

    /// 文本列的默认值**必须加引号**，哪怕它长得像表达式。
    ///
    /// 守着一次真实故障：源库里 `UNIQUE_CODE varchar(100) DEFAULT '(NULL)'`
    /// （默认值就是字符串 `(NULL)`），而元数据回出来**不带引号**；
    /// 老判据同时命中 `contains("NULL")` 与 `starts_with('(')`，于是拼出
    /// `default (NULL)` —— 同步到 MySQL 5.7 直接语法错误，8.0 上则悄悄变成表达式默认值。
    #[test]
    fn 文本列的默认值一定要加引号() {
        let ddl = Dialect::new(ConnectionKind::Mysql).create_table_from_columns(
            "`t`",
            &[
                col("UNIQUE_CODE", "varchar(100)", Some("(NULL)")),
                col("NOTE", "text", Some("NULL")),
            ],
            None,
        );
        assert!(ddl.contains("default '(NULL)'"), "实际：{ddl}");
        assert!(!ddl.contains("default (NULL)"), "实际：{ddl}");
        assert!(ddl.contains("default 'NULL'"), "实际：{ddl}");
    }

    /// 非文本列照旧原样放行：时间函数、数字、裸 NULL。
    #[test]
    fn 非文本列默认值原样放行() {
        let ddl = Dialect::new(ConnectionKind::Mysql).create_table_from_columns(
            "`t`",
            &[
                col("A", "datetime", Some("CURRENT_TIMESTAMP")),
                col("B", "int", Some("0")),
                col("C", "int", Some("NULL")),
            ],
            None,
        );
        assert!(ddl.contains("default CURRENT_TIMESTAMP"), "实际：{ddl}");
        assert!(ddl.contains("default 0"), "实际：{ddl}");
        assert!(ddl.contains("default NULL"), "实际：{ddl}");
    }

    /// PG / SQL Server 回出来的默认值自带引号（后者还多一层括号），**不能再套一层**。
    #[test]
    fn 已带引号的默认值不再加引号() {
        let ddl = Dialect::new(ConnectionKind::Mysql).create_table_from_columns(
            "t",
            &[
                col("a", "character varying(20)", Some("'abc'")),
                // SQL Server 的默认值定义一定是 `('abc')` / `((0))` / `(getdate())` 这种带括号的
                col("b", "varchar(20)", Some("('abc')")),
            ],
            None,
        );
        assert!(ddl.contains("default 'abc'"), "实际：{ddl}");
        assert!(ddl.contains("default ('abc')"), "实际：{ddl}");
        assert!(!ddl.contains("''abc''"), "实际：{ddl}");
        assert!(!ddl.contains("(''abc'')"), "实际：{ddl}");
    }

    /// 主键字符长度之和 × 4 字节超过 InnoDB 的 3072 时，建表要退到 utf8（3 字节/字符）。
    ///
    /// 守着一次真机故障：源表主键 `varchar(10)+varchar(255)×3` = 775 字符，
    /// 源库 utf8（2325 字节，合法），建表一律 utf8mb4 ⇒ 3100 字节 ⇒
    /// `ERROR 1071 Specified key was too long; max key length is 3072 bytes`，
    /// 整张表连结构都建不出来。
    #[test]
    fn 主键太长时建表退到utf8() {
        let mut cols = vec![
            col("report_day", "varchar(10)", None),
            col("project_number", "varchar(255)", None),
            col("project_name", "varchar(255)", None),
            col("xf_brandcat", "varchar(255)", None),
            col("xf_description", "varchar(255)", None),
        ];
        for c in cols.iter_mut().filter(|c| c.name != "project_name") {
            c.primary_key = true;
        }
        let ddl = Dialect::new(ConnectionKind::Mysql).create_table_from_columns("t", &cols, None);
        assert!(ddl.contains("default charset=utf8"), "实际：{ddl}");
        assert!(!ddl.contains("utf8mb4"), "实际：{ddl}");
    }

    /// 主键装得下时**不能**动字符集（99% 的表走这条，必须与以前完全一致）。
    #[test]
    fn 主键不长时仍是utf8mb4() {
        let mut cols = vec![col("id", "varchar(64)", None), col("name", "varchar(255)", None)];
        cols[0].primary_key = true;
        let ddl = Dialect::new(ConnectionKind::Mysql).create_table_from_columns("t", &cols, None);
        assert!(ddl.contains("default charset=utf8mb4"), "实际：{ddl}");
    }

    /// 可空的 timestamp 列**必须显式写 null**。
    ///
    /// MySQL 5.7（`explicit_defaults_for_timestamp=OFF`，默认值）会给「表里第二个及以后的
    /// timestamp 列」隐式补 `'0000-00-00 00:00:00'`；默认 sql_mode 含 NO_ZERO_DATE ⇒
    /// 建表直接 1067「Invalid default value for 'create_time'」（真机故障）。
    #[test]
    fn 可空timestamp列显式写null() {
        let ddl = Dialect::new(ConnectionKind::Mysql).create_table_from_columns(
            "t",
            &[col("create_time", "TIMESTAMP", None)],
            None,
        );
        assert!(ddl.contains("create_time TIMESTAMP null"), "实际：{ddl}");
    }

    /// 非空 timestamp 没有默认值时得给一个合法默认（否则同样隐式零日期 → 1067）。
    #[test]
    fn 非空timestamp列给current_timestamp() {
        let mut column = col("create_time", "TIMESTAMP", None);
        column.nullable = false;
        let ddl = Dialect::new(ConnectionKind::Mysql).create_table_from_columns("t", &[column], None);
        assert!(
            ddl.contains("TIMESTAMP not null default current_timestamp"),
            "实际：{ddl}"
        );
    }

    /// 零日期默认值（源库 sql_mode 宽松才存得下）要省略，否则目标端 NO_ZERO_DATE 直接 1067。
    #[test]
    fn 零日期默认值被省略() {
        let ddl = Dialect::new(ConnectionKind::Mysql).create_table_from_columns(
            "t",
            &[
                col("XF_LASTMODTIME", "datetime", Some("0000-00-00 00:00:00")),
                col("D2", "datetime", Some("'0000-00-00 00:00:00'")),
            ],
            None,
        );
        assert!(!ddl.contains("0000-00-00"), "实际：{ddl}");
    }

    /// 布尔列跨类型默认值：MySQL 的 `tinyint(1) default 0` 映射到 PG 是
    /// `boolean default 0`，PG 直接拒 —— `column "status" is of type boolean but default
    /// expression is of type integer`（真机：MySQL → PG 整库同步，`job_info` 建表失败）。
    #[test]
    fn 跨类型布尔默认值翻成_true_false() {
        let pg = Dialect::new(ConnectionKind::Postgresql);
        assert_eq!(pg.default_literal("0", "boolean", true).as_deref(), Some("false"));
        assert_eq!(pg.default_literal("1", "boolean", true).as_deref(), Some("true"));
        assert_eq!(
            pg.default_literal("b'0'", "boolean", true).as_deref(),
            Some("false")
        );
        assert_eq!(
            pg.default_literal("'1'", "boolean", true).as_deref(),
            Some("true")
        );
        // 同类型保持原样（MySQL 的 tinyint 语义，不能被翻成 false/true）
        assert_eq!(pg.default_literal("0", "boolean", false).as_deref(), Some("0"));
        // 非布尔列不受影响
        assert_eq!(pg.default_literal("0", "integer", true).as_deref(), Some("0"));
        // Doris / SQL Server 反过来**不认 false/true**（真机实测：`boolean default false`
        // 在 Doris 上是语法错、`bit default false` 在 SQL Server 上被拒），只收 0/1
        let doris = Dialect::new(ConnectionKind::Doris);
        assert_eq!(
            doris.default_literal("false", "BOOLEAN", true).as_deref(),
            Some("0")
        );
        assert_eq!(doris.default_literal("1", "BOOLEAN", true).as_deref(), Some("1"));
        let mssql = Dialect::new(ConnectionKind::Sqlserver);
        assert_eq!(mssql.default_literal("true", "bit", true).as_deref(), Some("1"));
        assert_eq!(mssql.default_literal("'0'", "bit", true).as_deref(), Some("0"));
    }

    /// MySQL → ClickHouse：`DEFAULT CURRENT_TIMESTAMP` 要翻成 CH 的 `now()`。
    ///
    /// ClickHouse 没有 `current_timestamp` 函数，裸写会被当成列名：
    /// `Code: 47 Missing columns: 'current_timestamp' … default expression and column
    /// type are incompatible` —— 整张表建不出来（真机：源表
    /// `createtime datetime DEFAULT CURRENT_TIMESTAMP`）。
    #[test]
    fn 同步到clickhouse时时间默认值翻成now() {
        let ddl = Dialect::new(ConnectionKind::Clickhouse).create_table_from_columns(
            "t",
            &[
                col("createtime", "datetime", Some("CURRENT_TIMESTAMP")),
                col("updatetime", "datetime", Some("current_timestamp()")),
                col("d", "date", Some("CURRENT_DATE")),
            ],
            Some(ConnectionKind::Mysql),
        );
        assert!(
            ddl.contains("createtime Nullable(DateTime) default now()"),
            "实际：{ddl}"
        );
        assert!(
            ddl.contains("updatetime Nullable(DateTime) default now()"),
            "实际：{ddl}"
        );
        assert!(ddl.contains("default today()"), "实际：{ddl}");
        assert!(!ddl.to_ascii_lowercase().contains("current_"), "实际：{ddl}");
    }

    /// `current_date` 不是每个库都认（11 类目标库实测：见 `portable_time_literal`）：
    /// SQL Server 要 `cast(getdate() as date)`，MySQL 干脆表达不了（省略）。
    #[test]
    fn 当前日期默认值按目标方言翻译() {
        let cols = vec![col("d", "date", Some("CURRENT_DATE"))];

        let sqlserver = Dialect::new(ConnectionKind::Sqlserver).create_table_from_columns(
            "t",
            &cols,
            Some(ConnectionKind::Postgresql),
        );
        assert!(
            sqlserver.contains("default cast(getdate() as date)"),
            "实际：{sqlserver}"
        );

        // MySQL：列默认值只支持字面量与 CURRENT_TIMESTAMP，DATE 列的 current_date 表达不了
        let mysql = Dialect::new(ConnectionKind::Mysql).create_table_from_columns(
            "t",
            &cols,
            Some(ConnectionKind::Postgresql),
        );
        assert!(
            !mysql.to_ascii_lowercase().contains("current_date"),
            "实际：{mysql}"
        );
        // 列定义里不再有默认值（表尾的 `default charset=…` 不算）
        assert!(!mysql.contains("d date default"), "实际：{mysql}");

        // 其余方言保持标准写法
        let pg = Dialect::new(ConnectionKind::Postgresql).create_table_from_columns(
            "t",
            &cols,
            Some(ConnectionKind::Mysql),
        );
        assert!(pg.contains("default current_date"), "实际：{pg}");
    }

    /// 跨类型映射：源类型 → 目标类型；**同类型一律原样照抄**（零回归）。
    #[test]
    fn 跨类型把源类型翻译到目标() {
        let mysql = Dialect::new(ConnectionKind::Mysql);
        let oracle = Dialect::new(ConnectionKind::Oracle);
        let pg = Dialect::new(ConnectionKind::Postgresql);
        // 同类型 / 不给来源 ⇒ 不翻译
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Mysql), "datetime"),
            "datetime"
        );
        assert_eq!(mysql.map_type(None, "tinyint(1)"), "tinyint(1)");
        // MySQL → Oracle：下面这些类型 Oracle 一个都不认
        assert_eq!(
            oracle.map_type(Some(ConnectionKind::Mysql), "datetime"),
            "timestamp"
        );
        assert_eq!(oracle.map_type(Some(ConnectionKind::Mysql), "text"), "clob");
        assert_eq!(
            oracle.map_type(Some(ConnectionKind::Mysql), "varchar(100)"),
            "varchar2(100)"
        );
        assert_eq!(
            oracle.map_type(Some(ConnectionKind::Mysql), "bigint"),
            "number(19)"
        );
        assert_eq!(
            oracle.map_type(Some(ConnectionKind::Mysql), "tinyint(1)"),
            "number(1)"
        );
        assert_eq!(oracle.map_type(Some(ConnectionKind::Mysql), "longblob"), "blob");
        // PG → MySQL：`character varying` / `int4` / `timestamptz` / `bytea` 都不是 MySQL 类型
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Postgresql), "character varying(50)"),
            "varchar(50)"
        );
        assert_eq!(mysql.map_type(Some(ConnectionKind::Postgresql), "int4"), "int");
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Postgresql), "timestamptz"),
            "datetime"
        );
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Postgresql), "bytea"),
            "longblob"
        );
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Postgresql), "boolean"),
            "tinyint(1)"
        );
        // ClickHouse → PG
        assert_eq!(
            pg.map_type(Some(ConnectionKind::Clickhouse), "UInt64"),
            "numeric(20,0)"
        );
        assert_eq!(pg.map_type(Some(ConnectionKind::Clickhouse), "String"), "text");
        // ClickHouse → MySQL：String 该落 longtext（不是大 varchar）；`DecimalNN(s)` 括号里是 scale、
        // 精度由位数定 —— 曾经一律按 38 位算，`Decimal64(2)` 被写成 `decimal(38,2)`，每列内联从
        // 9 字节涨到 17 字节，宽表同步时就是「Row size too large (> 8126)」的帮凶（真机踩过）
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Clickhouse), "String"),
            "longtext"
        );
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Clickhouse), "Nullable(String)"),
            "longtext"
        );
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Clickhouse), "Decimal32(2)"),
            "decimal(9,2)"
        );
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Clickhouse), "Decimal64(2)"),
            "decimal(18,2)"
        );
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Clickhouse), "Decimal128(4)"),
            "decimal(38,4)"
        );
        // 76 位超出 MySQL 的上限 65：必须夹住，否则建表直接报「Too big precision」
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Clickhouse), "Decimal256(4)"),
            "decimal(65,4)"
        );
        // 建表必须显式 row_format=dynamic：老服务器默认 COMPACT，长文本列每列内联 768 字节前缀，
        // 十几个 TEXT 列就撞上 8126 行上限
        let mysql_ddl = mysql.create_table_from_columns("t", &[col("a", "String", None)], None);
        assert!(
            mysql_ddl.to_ascii_lowercase().contains("row_format=dynamic"),
            "实际：{mysql_ddl}"
        );
        // 认不出的族降级为文本：宁可能存下，也不要建表就语法错
        assert_eq!(
            mysql.map_type(Some(ConnectionKind::Postgresql), "geometry"),
            "longtext"
        );
    }

    /// 跨类型建表：类型被翻译、自增跟着走、不可移植的默认值被省略。
    #[test]
    fn 跨类型建表翻译类型与自增() {
        let oracle = Dialect::new(ConnectionKind::Oracle);
        let ddl = oracle.create_table_from_columns(
            "\"T\"",
            &[
                auto_col("ID", "bigint"),
                col("NAME", "varchar(60)", Some("now()")),
                col("CREATED", "datetime", Some("CURRENT_TIMESTAMP")),
                col("SEQ", "int", Some("nextval('s')")),
            ],
            Some(ConnectionKind::Mysql),
        );
        // Oracle 的 identity **必须紧跟类型**：写在 `not null` 之后报 ORA-03076
        //（真机实测，见 `_uitest/ddl-syntax-probe.py`）
        assert!(
            ddl.contains("ID number(19) generated by default as identity not null"),
            "自增与类型都要翻译：{ddl}"
        );
        // 文本列的默认值是"内容"，`now()` 这种要加引号、不能当时间函数翻译掉
        assert!(ddl.contains("NAME varchar2(60) default 'now()'"), "实际：{ddl}");
        // 时间默认值翻译成各家都认的标准写法
        assert!(
            ddl.contains("CREATED timestamp default current_timestamp"),
            "实际：{ddl}"
        );
        // 认不出的函数默认值跨类型时**省略**（`nextval` 各家写法完全不同，带过去就是语法错）
        assert!(!ddl.contains("nextval"), "不可移植的默认值应被省略：{ddl}");
        assert!(
            ddl.lines()
                .any(|line| line.contains("SEQ") && !line.contains("default")),
            "SEQ 这一行不该带 default：{ddl}"
        );
        assert!(!ddl.contains("datetime"), "源类型不能照抄过去：{ddl}");
    }

    /// Doris 建表要走它自己那套语法。
    ///
    /// 守着一次真实故障：MySQL → Doris 同步时 Doris 连报
    /// `line 15: REMARKS longtext, ... Expected: IDENTIFIER`（Doris 没有 longtext）、
    /// `line 6: primary key (product_id) ... Encountered: HEY`（Doris 没有表级 primary key，
    /// `HEY` 是它把 `KEY` 规范化后的显示）、以及
    /// `replication num is 3, available backend num is 1`（默认副本数 3，单 BE 建不出来）。
    #[test]
    fn doris_建表用它的语法() {
        let doris = Dialect::new(ConnectionKind::Doris);
        let ddl = doris.create_table_from_columns(
            "`t`",
            &[
                col("BODY", "longtext", None),
                auto_col("ID", "bigint"),
                col("NAME", "varchar(30)", None),
            ],
            Some(ConnectionKind::Mysql),
        );
        assert!(ddl.contains("UNIQUE KEY(ID)"), "实际：{ddl}");
        assert!(ddl.contains("DISTRIBUTED BY HASH(ID) BUCKETS 1"), "实际：{ddl}");
        assert!(ddl.contains("replication_num"), "必须显式给副本数：{ddl}");
        // 键列必须排在最前（源表里 BODY 排在 ID 前面 —— 这里必须被挪到第一个）
        assert!(ddl.contains("(\n  ID bigint"), "键列要排最前：{ddl}");
        // 类型：没有 longtext；自增：Doris 上不写
        assert!(ddl.contains("BODY STRING"), "实际：{ddl}");
        assert!(!ddl.contains("longtext"), "实际：{ddl}");
        assert!(!ddl.contains("auto_increment"), "实际：{ddl}");
        // 没有主键时用第一列做 DUPLICATE KEY（Doris 也要求一个 KEY）
        let no_pk = doris.create_table_from_columns(
            "`u`",
            &[col("A", "int", None), col("B", "text", None)],
            Some(ConnectionKind::Mysql),
        );
        assert!(no_pk.contains("DUPLICATE KEY(A)"), "实际：{no_pk}");
        assert!(no_pk.contains("B STRING"), "实际：{no_pk}");

        // **键列不能是无界文本**：STRING/TEXT 做 UNIQUE/DUPLICATE KEY 直接被 Doris 拒
        //（「String Type should not be used in key column」，真机：ClickHouse String 键同步必炸）
        // —— 键列要落成有长度的 VARCHAR(65533)，普通列不受影响
        let str_pk = doris.create_table_from_columns(
            "`s`",
            &[
                ColumnDetail {
                    primary_key: true,
                    ..col("MAILID", "string", None)
                },
                col("V", "int", None),
            ],
            Some(ConnectionKind::Clickhouse),
        );
        assert!(str_pk.contains("MAILID varchar(65533)"), "实际：{str_pk}");
        assert!(!str_pk.contains("MAILID STRING"), "实际：{str_pk}");
        assert!(str_pk.contains("V int"), "非键列照常翻译：{str_pk}");

        // 无主键且第一列是无界文本（兜底 DUPLICATE KEY）：同样要落成 VARCHAR
        let first_str = doris.create_table_from_columns(
            "`f`",
            &[col("NOTE", "text", None), col("N", "int", None)],
            Some(ConnectionKind::Mysql),
        );
        assert!(first_str.contains("NOTE varchar(65533)"), "实际：{first_str}");
    }

    /// **逐方言体检**：同一份"典型列"分别建到每个目标方言，逐条断言该方言的硬性约束。
    ///
    /// 为什么要这份用例：跨类型同步的每个目标方言，至少要"被拼过一次、被断言过"，
    /// 而不是等用户真的同步过去才暴露语法错。已暴露的三个都是这么来的 ——
    /// Doris 的 `longtext` + 表级 `primary key`、ClickHouse 的 in-list `primary key`、
    /// 以及 ClickHouse 不接受裸 `delete from`。**新增方言时在这里补一行即可。**
    #[test]
    fn 各目标方言的建表语句符合它的硬性约束() {
        let columns = [
            auto_col("ID", "bigint"),
            col("NAME", "varchar(30)", None),
            col("BODY", "longtext", None),
        ];
        let mut checked = 0;
        for key in [
            "mysql",
            "mariadb",
            "doris",
            "postgresql",
            "kingbase",
            "sqlserver",
            "oracle",
            "dm",
            "db2",
            "h2",
            "derby",
            "clickhouse",
            "sqlite",
        ] {
            let Some(kind) = ConnectionKind::from_key(key) else {
                continue;
            };
            checked += 1;
            let dialect = Dialect::new(kind);
            let ddl = dialect.create_table_from_columns("t", &columns, Some(ConnectionKind::Mysql));
            let lower = ddl.to_lowercase();
            // ① 源方言的类型名不能漏出去（漏了就说明翻译没生效）；MySQL 家族除外
            if !matches!(key, "mysql" | "mariadb" | "oceanbase") {
                assert!(!lower.contains("longtext"), "{key}: 类型没翻译：{ddl}");
            }
            // ② 各家的硬性约束
            match key {
                "doris" => {
                    assert!(ddl.contains("UNIQUE KEY("), "{key}: 缺 UNIQUE KEY：{ddl}");
                    assert!(
                        ddl.contains("DISTRIBUTED BY HASH("),
                        "{key}: 缺 DISTRIBUTED：{ddl}"
                    );
                    assert!(
                        ddl.contains("replication_num"),
                        "{key}: 缺 replication_num：{ddl}"
                    );
                    assert!(!lower.contains("primary key"), "{key}: 不该有表级主键：{ddl}");
                    assert!(!lower.contains("auto_increment"), "{key}: 不写自增：{ddl}");
                }
                "clickhouse" => {
                    assert!(ddl.contains("ENGINE = MergeTree()"), "{key}: 缺引擎：{ddl}");
                    assert!(ddl.contains("ORDER BY"), "{key}: 缺 ORDER BY：{ddl}");
                    assert!(
                        !lower.contains("primary key"),
                        "{key}: primary key 不能在列清单里：{ddl}"
                    );
                    assert!(!lower.contains("auto_increment"), "{key}: 没有自增：{ddl}");
                }
                "postgresql" | "kingbase" | "opengauss" | "oracle" | "dm" | "h2" | "db2" | "derby" => {
                    assert!(
                        ddl.contains("generated by default as identity"),
                        "{key}: 自增没跟上：{ddl}"
                    );
                }
                "sqlserver" => assert!(ddl.contains("identity(1,1)"), "{key}: 自增没跟上：{ddl}"),
                "mysql" | "mariadb" | "oceanbase" => {
                    assert!(ddl.contains("auto_increment"), "{key}: 自增没跟上：{ddl}");
                }
                _ => {}
            }
            // ③ 清空语句：ClickHouse 不能用裸 delete（实测 position 26 语法错），SQLite/Derby 反之
            let clear = dialect.clear_table_sql("t");
            match key {
                "sqlite" | "derby" => {
                    assert!(clear.starts_with("delete"), "{key}: 清空应为 delete：{clear}");
                }
                _ => {
                    assert!(clear.starts_with("truncate"), "{key}: 清空应为 truncate：{clear}");
                }
            }
            // ④ 更新语句：ClickHouse 没有标准 UPDATE，必须 ALTER TABLE ... UPDATE
            let update = dialect.update_sql("t", "a = 1", "k = 2");
            if key == "clickhouse" {
                assert!(
                    update.starts_with("alter table"),
                    "{key}: 要走 ALTER UPDATE：{update}"
                );
            } else {
                assert!(update.starts_with("update "), "{key}: 标准 UPDATE：{update}");
            }
        }
        assert!(checked >= 10, "至少要覆盖 10 个方言，实际 {checked}");
    }

    /// 同类型建表**完全照抄**（含 MySQL 的复合默认值），一个字都不改。
    #[test]
    fn 同类型建表保持原样() {
        let mysql = Dialect::new(ConnectionKind::Mysql);
        let ddl = mysql.create_table_from_columns(
            "`t`",
            &[col(
                "UPDATED",
                "datetime",
                Some("CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP"),
            )],
            Some(ConnectionKind::Mysql),
        );
        assert!(
            ddl.contains("default CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP"),
            "同类型不许翻译：{ddl}"
        );
    }

    /// **保留字必须加引号** —— 这条守着一次真实故障：库里一张叫 `user` 的表，
    /// 菜单「清空表」生成的是 `delete from user`，SQL Server 直接 156 语法错；
    /// 而同一张表手写 `select * from [user]` 完全正常，所以看着像「清空坏了」。
    #[test]
    fn 保留字按方言加引号() {
        assert_eq!(q(ConnectionKind::Sqlserver, "user"), "[user]");
        assert_eq!(q(ConnectionKind::Mysql, "order"), "`order`");
        assert_eq!(q(ConnectionKind::Postgresql, "table"), "\"table\"");
        assert_eq!(q(ConnectionKind::Sqlite, "select"), "\"select\"");
        // 大小写不敏感：树里给的名字未必是大写
        assert_eq!(q(ConnectionKind::Sqlserver, "User"), "[User]");
    }

    /// 普通名保持裸写（既有行为不能变：MySQL 的大小写敏感靠它）。
    #[test]
    fn 普通名不加引号() {
        assert_eq!(q(ConnectionKind::Mysql, "sales"), "sales");
        assert_eq!(q(ConnectionKind::Sqlserver, "chengji"), "chengji");
        assert_eq!(q(ConnectionKind::Postgresql, "biz_message"), "biz_message");
    }

    /// 带特殊字符的名字照旧加引号，并做引号转义。
    #[test]
    fn 特殊字符仍然加引号() {
        assert_eq!(q(ConnectionKind::Mysql, "my table"), "`my table`");
        assert_eq!(q(ConnectionKind::Sqlserver, "a]b"), "[a]]b]");
        assert_eq!(q(ConnectionKind::Postgresql, "a\"b"), "\"a\"\"b\"");
    }

    /// 上折方言：要加引号时先折成大写，避免悄悄造出小写对象。
    /// （Oracle 里 `create table user` 建的是 `USER`，加引号后必须仍是 `USER`。）
    #[test]
    fn 上折方言先折大写再加引号() {
        assert_eq!(q(ConnectionKind::Oracle, "user"), "\"USER\"");
        assert_eq!(q(ConnectionKind::Dm, "order"), "\"ORDER\"");
        assert_eq!(q(ConnectionKind::Db2, "key"), "\"KEY\"");
        // 下折 / 原样方言不动大小写
        assert_eq!(q(ConnectionKind::Postgresql, "user"), "\"user\"");
        assert_eq!(q(ConnectionKind::Mysql, "user"), "`user`");
    }

    /// 多段名逐段处理（库.表）。
    #[test]
    fn 多段名逐段加引号() {
        assert_eq!(q(ConnectionKind::Sqlserver, "dbo.user"), "dbo.[user]");
        assert_eq!(q(ConnectionKind::Mysql, "test.order"), "test.`order`");
        // `public` 本身是保留字（PG 的公共 schema 名），所以它也被加引号 ——
        // 这里是对的：PG 里 schema 就是小写 public，加引号后依然指向它
        assert_eq!(
            q(ConnectionKind::Postgresql, "public.user"),
            "\"public\".\"user\""
        );
        assert_eq!(q(ConnectionKind::Postgresql, "public.sales"), "\"public\".sales");
    }

    /// 下折方言（PG / SQLite）里含大写字母的名字必须加引号，否则会被折成小写。
    ///
    /// 守着一次真机故障：目标 PG 表上有一列叫 `Name`，同步写进去报
    /// `column "name" of relation "t" does not exist` —— 结构明明是好的，
    /// 只是 `quote()` 把普通名裸写了，PG 把它折成了 `name`。
    #[test]
    fn 下折方言的大小写不敏感名要加引号() {
        assert_eq!(q(ConnectionKind::Postgresql, "Name"), "\"Name\"");
        assert_eq!(q(ConnectionKind::Postgresql, "name"), "name");
        assert_eq!(q(ConnectionKind::Sqlite, "Amount"), "\"Amount\"");
        // 上折方言保持原样：全大写照旧裸写，小写普通名照旧裸写（由库自己上折）
        assert_eq!(q(ConnectionKind::Oracle, "NAME"), "NAME");
        assert_eq!(q(ConnectionKind::Oracle, "name"), "name");
        // 不折的方言完全不受影响
        assert_eq!(q(ConnectionKind::Mysql, "Name"), "Name");
    }

    // ==================== 原生 upsert 语句 ====================
    //
    // 这几条是「一条语句搞定插入或更新」的**生成规则**：拼错一个关键字，整张表的
    // upsert 会在真机上直接语法错；而少写 `when matched` / `on conflict` 则更坏 ——
    // 语句仍然成功，只是把「更新」变成了「插入」，行数悄悄翻倍。

    fn upsert(kind: ConnectionKind) -> Option<String> {
        Dialect::new(kind).upsert_sql(
            "t",
            &["id".to_string(), "name".to_string(), "amount".to_string()],
            &[0],
            &[
                vec!["1".to_string(), "'a'".to_string(), "10".to_string()],
                vec!["2".to_string(), "'b'".to_string(), "20".to_string()],
            ],
        )
    }

    #[test]
    fn mysql_用_on_duplicate_key_update() {
        let sql = upsert(ConnectionKind::Mysql).expect("mysql 必须支持原生 upsert");
        assert_eq!(
            sql,
            "insert into t (id, name, amount) values (1, 'a', 10), (2, 'b', 20) \
             on duplicate key update name = values(name), amount = values(amount)"
        );
    }

    #[test]
    fn pg_用_on_conflict_do_update_且键列原样_匹配() {
        let sql = upsert(ConnectionKind::Postgresql).expect("pg 必须支持原生 upsert");
        assert_eq!(
            sql,
            "insert into t (id, name, amount) values (1, 'a', 10), (2, 'b', 20) \
             on conflict (id) do update set name = excluded.name, amount = excluded.amount"
        );
    }

    #[test]
    fn oracle_用_merge_union_all_源() {
        let sql = upsert(ConnectionKind::Oracle).expect("oracle 必须支持原生 upsert");
        assert!(sql.starts_with(
            "merge into t tgt using (select 1 as id, 'a' as name, 10 as amount from dual union all "
        ));
        assert!(sql.contains("on (tgt.id = src.id)"));
        assert!(sql.contains("when matched then update set name = src.name, amount = src.amount"));
        assert!(sql.ends_with(
            "when not matched then insert (id, name, amount) values (src.id, src.name, src.amount)"
        ));
    }

    #[test]
    fn sqlserver_与_db2_用_values_派生表并给别名() {
        let sql = upsert(ConnectionKind::Sqlserver).expect("sqlserver 必须支持原生 upsert");
        assert!(sql.starts_with("merge into t as tgt using (values (1, 'a', 10), (2, 'b', 20)) as src (id, name, amount) on (tgt.id = src.id)"));
        assert!(sql.contains("when matched then update set name = src.name, amount = src.amount"));
        // T-SQL 要求 MERGE 以分号结束（老解析器硬要求，缺了就报语法错）
        assert!(sql.ends_with(';'));

        let db2 = upsert(ConnectionKind::Db2).expect("db2 必须支持原生 upsert");
        assert!(db2.starts_with("merge into t as tgt using (values "));
        assert!(!db2.ends_with(';'), "DB2 不该拼分号（它不接受语句尾分号）");
    }

    /// 分页子句：DB2 / Derby / Oracle / DM 不吃 `limit`，SQL Server 也只能 `offset…fetch`。
    /// 这几个是"翻页才报错"的高发区 —— 第 1 页不拼分页子句，错误要到用户点下一页才暴露。
    #[test]
    fn 分页子句按方言区分_limit_与_offset_fetch() {
        let db2 = Dialect::new(ConnectionKind::Db2).limit_clause(200, 200);
        assert_eq!(db2, "offset 200 rows fetch next 200 rows only", "{db2}");
        assert!(!db2.contains("limit"), "{db2}");
        assert_eq!(
            Dialect::new(ConnectionKind::Derby).limit_clause(0, 200),
            "offset 0 rows fetch next 200 rows only"
        );
        assert_eq!(
            Dialect::new(ConnectionKind::Sqlserver).limit_clause(0, 200),
            "offset 0 rows fetch next 200 rows only"
        );
        assert_eq!(
            Dialect::new(ConnectionKind::Mysql).limit_clause(200, 200),
            "limit 200 offset 200"
        );
        assert_eq!(
            Dialect::new(ConnectionKind::Clickhouse).limit_clause(0, 200),
            "limit 200 offset 0"
        );
    }

    #[test]
    fn oracle与dm原生upsert必须限制单语句行数() {
        assert_eq!(Dialect::new(ConnectionKind::Oracle).upsert_row_limit(), 500);
        assert_eq!(Dialect::new(ConnectionKind::Dm).upsert_row_limit(), 500);
        assert_eq!(Dialect::new(ConnectionKind::Sqlserver).upsert_row_limit(), 1000);
        assert_eq!(
            Dialect::new(ConnectionKind::Postgresql).upsert_row_limit(),
            usize::MAX
        );
    }

    /// `_` 开头的表名不能裸写（Oracle 未引号标识符必须字母开头 → ORA-00911），
    /// 必须加引号，且**不能折大写** —— 字典里是小写就说明当初是带引号建的小写对象
    /// （豫园 11g：`"_REPORT_XINGWEI"` 找不到表，`"_report_xingwei"` 才对）。
    #[test]
    fn 下划线开头的名字必须加引号且不折大小写() {
        let oracle = Dialect::new(ConnectionKind::Oracle);
        assert_eq!(oracle.quote("_report_xingwei"), "\"_report_xingwei\"");
        assert_eq!(oracle.quote("_REPORT_XINGWEI"), "\"_REPORT_XINGWEI\"");
        // 普通名行为不变：全小写裸写（折大写命中常规大写对象）、保留字加引号折大写
        assert_eq!(oracle.quote("report_xingwei"), "report_xingwei");
        assert_eq!(oracle.quote("user"), "\"USER\"");
        // 数字开头同理
        assert_eq!(oracle.quote("1t"), "\"1t\"");
    }

    /// Oracle 分页走 ROWNUM 包装（11g 不认 `offset … fetch …`，12c 语法 ORA-00933），
    /// 其余方言返回 None 退回 limit_clause 后缀。
    #[test]
    fn oracle分页用rownum包装_其余方言返回none() {
        let oracle = Dialect::new(ConnectionKind::Oracle);
        let wrapped = oracle
            .wrap_paging("select * from t", 0, 200)
            .expect("oracle 必须有包装式分页");
        assert!(wrapped.contains("rownum rn__"), "{wrapped}");
        assert!(wrapped.contains("rownum <= 200"), "{wrapped}");
        assert!(wrapped.contains("rn__ > 0"), "{wrapped}");
        let paged = oracle
            .wrap_paging("select * from t", 400, 200)
            .expect("oracle 必须有包装式分页");
        assert!(paged.contains("rownum <= 600"), "{paged}");
        assert!(paged.contains("rn__ > 400"), "{paged}");
        assert!(Dialect::new(ConnectionKind::Mysql)
            .wrap_paging("select 1", 0, 10)
            .is_none());
        assert!(Dialect::new(ConnectionKind::Db2)
            .wrap_paging("select 1", 0, 10)
            .is_none());
    }

    #[test]
    // 名字里的 `_None` 是为了让人一眼看出断言结果；中文测试名是项目惯例，这里只关掉大小写这一条
    #[allow(non_snake_case)]
    fn 没有原生语法的方言返回_None() {
        // ClickHouse / Doris 继续走两阶段（预读主键 + 逐行 UPDATE）。
        // Derby 10.11 起有 MERGE，已经纳入原生 upsert（见下面的专测）。
        assert!(upsert(ConnectionKind::Clickhouse).is_none());
        assert!(upsert(ConnectionKind::Doris).is_none());
        assert!(!Dialect::new(ConnectionKind::Clickhouse).supports_native_upsert());
    }

    #[test]
    fn derby走_merge形式的原生_upsert() {
        let sql = upsert(ConnectionKind::Derby).expect("derby 必须支持原生 upsert");
        assert!(
            sql.starts_with("merge into t as tgt using (values "),
            "实际：{sql}"
        );
        assert!(sql.contains("when matched then update set"), "实际：{sql}");
        assert!(sql.contains("when not matched then insert"), "实际：{sql}");
        assert!(!sql.ends_with(';'), "Derby 不该拼分号");
        assert!(Dialect::new(ConnectionKind::Derby).supports_native_upsert());
        assert_eq!(Dialect::new(ConnectionKind::Derby).upsert_row_limit(), 1000);
    }

    #[test]
    fn 全是主键列时退化成_插入或忽略() {
        let sql = Dialect::new(ConnectionKind::Postgresql)
            .upsert_sql(
                "t",
                &["id".to_string(), "code".to_string()],
                &[0, 1],
                &[vec!["1".to_string(), "'x'".to_string()]],
            )
            .expect("pg 必须支持");
        assert_eq!(
            sql,
            "insert into t (id, code) values (1, 'x') on conflict (id, code) do nothing"
        );
        // Oracle 全键时没有 when matched 子句，但插入分支仍然在
        let ora = Dialect::new(ConnectionKind::Oracle)
            .upsert_sql("t", &["id".to_string()], &[0], &[vec!["1".to_string()]])
            .expect("oracle 必须支持");
        assert!(!ora.contains("when matched"));
        assert!(ora.contains("when not matched then insert (id) values (src.id)"));
    }
}
