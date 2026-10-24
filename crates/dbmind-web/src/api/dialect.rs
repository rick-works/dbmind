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
    matches!(key, "postgresql" | "kingbase" | "sqlite" | "sqlserver")
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
    Some(match code.to_ascii_uppercase().as_str() {
        "MYSQL" | "MARIADB" | "DORIS" => "mysql",
        "POSTGRESQL" | "KINGBASE" => "postgresql",
        "SQLSERVER" => "sqlserver",
        "ORACLE" | "DM" => "oracle",
        "DB2" => "db2",
        "H2" => "h2",
        "DERBY" => "derby",
        "CLICKHOUSE" => "clickhouse",
        "SQLITE" => "sqlite",
        "MONGODB" => "mongodb",
        "REDIS" => "redis",
        "ELASTICSEARCH" => "elasticsearch",
        _ => return None,
    })
}

/// 关系型模块路径段（9 个），路由按它们注册。
pub const RELATIONAL_MODULES: &[&str] = &[
    "mysql",
    "postgresql",
    "sqlserver",
    "oracle",
    "db2",
    "h2",
    "derby",
    "clickhouse",
    "sqlite",
];

/// NoSQL 模块路径段（3 个）。
pub const NOSQL_MODULES: &[&str] = &["mongodb", "redis", "elasticsearch"];

/// 前端类型注册表里存在的类型码（16 个）。
///
/// 故意**不含 DUCKDB**：内核支持它，但 `frontend/src/types/` 里没有 `duckdb.js`，
/// 暴露出去只会让「新建数据源」弹出没有 Logo、没有默认端口、引用符也不对的卡片。
/// 前端没有的类型，后端就不该通告 —— 这是「单一真源」该有的样子。
pub fn exposed_type_codes() -> Vec<&'static str> {
    vec![
        "MYSQL",
        "MARIADB",
        "DORIS",
        "POSTGRESQL",
        "KINGBASE",
        "ORACLE",
        "DM",
        "SQLSERVER",
        "DB2",
        "CLICKHOUSE",
        "SQLITE",
        "H2",
        "DERBY",
        "MONGODB",
        "REDIS",
        "ELASTICSEARCH",
    ]
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
fn h2_user(name: &str) -> String {
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
    "CREATE SESSION", "CREATE TABLE", "CREATE ANY TABLE", "ALTER ANY TABLE", "DROP ANY TABLE",
    "SELECT ANY TABLE", "INSERT ANY TABLE", "UPDATE ANY TABLE", "DELETE ANY TABLE",
    "CREATE VIEW", "CREATE ANY VIEW", "CREATE SEQUENCE", "CREATE PROCEDURE", "CREATE ANY PROCEDURE",
    "CREATE TRIGGER", "CREATE ANY TRIGGER", "CREATE TYPE", "CREATE SYNONYM", "CREATE ANY SYNONYM",
    "CREATE DATABASE LINK", "CREATE JOB", "CREATE MATERIALIZED VIEW", "CREATE ROLE", "CREATE USER",
    "ALTER USER", "DROP USER", "ALTER SESSION", "UNLIMITED TABLESPACE", "DEBUG CONNECT SESSION",
    "QUERY REWRITE"
];

/// Oracle 角色白名单（角色名同样是拼进 SQL 的，必须过白名单）。
pub const ORACLE_ROLES: [&str; 12] = [
    "CONNECT", "RESOURCE", "DBA", "EXP_FULL_DATABASE", "IMP_FULL_DATABASE",
    "SELECT_CATALOG_ROLE", "EXECUTE_CATALOG_ROLE", "DELETE_CATALOG_ROLE", "RECOVERY_CATALOG_OWNER",
    "SCHEDULER_ADMIN", "OEM_MONITOR", "AQ_ADMINISTRATOR_ROLE"
];

/// Oracle 对象权限白名单（`grant <这些> on <owner>.<object> to u`）。
pub const ORACLE_OBJECT_PRIVS: [&str; 13] = [
    "SELECT", "INSERT", "UPDATE", "DELETE", "ALTER", "INDEX", "REFERENCES", "READ",
    "EXECUTE", "DEBUG", "FLASHBACK", "ON COMMIT REFRESH", "QUERY REWRITE"
];

/// SQL Server 数据库级 / 对象级权限白名单（`grant <这些> on …`）。
/// 权限名会**原样拼进 SQL**，所以只放行清单里的。
pub const MSSQL_PRIVILEGES: [&str; 10] = [
    "SELECT", "INSERT", "UPDATE", "DELETE", "EXECUTE", "REFERENCES",
    "CONTROL", "ALTER", "VIEW DEFINITION", "TAKE OWNERSHIP"
];

pub fn is_mssql_privilege(name: &str) -> bool {
    in_list(&MSSQL_PRIVILEGES, name) || is_safe_name(name)
}

/// PostgreSQL/KingbaseES 授权白名单（这些名字会被**原样拼进** GRANT/REVOKE）。
/// 模式级只有 USAGE / CREATE；表级七项（PG 标准表权限）；例程级只放 EXECUTE
/// （PG 的 `WITH GRANT OPTION` 是子句不是权限名，界面不给这个勾，名单里就不收）。
pub const PG_SCHEMA_PRIVS: [&str; 2] = ["USAGE", "CREATE"];
pub const PG_TABLE_PRIVS: [&str; 7] = [
    "SELECT", "INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER",
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
    !n.is_empty()
        && n.len() <= 128
        && n.chars().all(|c| c.is_alphanumeric() || "_$#@.- ".contains(c))
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
        && s.split(' ').all(|w| {
            !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

pub fn is_oracle_system_priv(name: &str) -> bool { in_list(&ORACLE_SYSTEM_PRIVS, name) || is_safe_name(name) }
pub fn is_oracle_role(name: &str) -> bool { in_list(&ORACLE_ROLES, name) || is_safe_name(name) }
pub fn is_oracle_object_priv(name: &str) -> bool { in_list(&ORACLE_OBJECT_PRIVS, name) || is_safe_name(name) }

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
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

const RESERVED_WORDS: &[&str] = &[
    "ADD", "ALL", "ALTER", "ANALYZE", "AND", "ANY", "ARRAY", "AS", "ASC", "ASYMMETRIC",
    "AUTHORIZATION", "BEGIN", "BETWEEN", "BINARY", "BOTH", "BY", "CALL", "CASCADE", "CASE",
    "CAST", "CHECK", "COLLATE", "COLUMN", "COMMENT", "COMMIT", "CONNECT", "CONSTRAINT", "CREATE",
    "CROSS", "CUBE", "CURRENT", "CURRENT_DATE", "CURRENT_TIME", "CURRENT_TIMESTAMP",
    "CURRENT_USER", "CURSOR", "DATABASE", "DATABASES", "DEALLOCATE", "DEC", "DECIMAL", "DECLARE",
    "DEFAULT", "DELETE", "DESC", "DESCRIBE", "DISTINCT", "DIV", "DO", "DOUBLE", "DROP", "DUAL",
    "ELSE", "ELSEIF", "END", "ESCAPE", "EXCEPT", "EXEC", "EXECUTE", "EXISTS", "EXPLAIN",
    "EXTERNAL", "FALSE", "FETCH", "FILE", "FLOAT", "FOR", "FORCE", "FOREIGN", "FROM", "FULL",
    "FULLTEXT", "FUNCTION", "GRANT", "GROUP", "GROUPING", "HAVING", "HIGH_PRIORITY", "IF",
    "IGNORE", "IN", "INDEX", "INNER", "INOUT", "INSERT", "INT", "INTEGER", "INTERSECT",
    "INTERVAL", "INTO", "IS", "JOIN", "KEY", "KEYS", "KILL", "LEADING", "LEAVE", "LEFT", "LEVEL",
    "LIKE", "LIMIT", "LOCK", "LONG", "LOOP", "MATCH", "MERGE", "MOD", "MODIFY", "NATURAL", "NOT",
    "NULL", "OFFSET", "ON", "OPTION", "OR", "ORDER", "OUT", "OUTER", "OVER", "PARTITION",
    "PRECISION", "PRIMARY", "PROCEDURE", "PUBLIC", "PURGE", "RANGE", "READ", "REAL", "RECURSIVE",
    "REFERENCES", "REGEXP", "RELEASE", "RENAME", "REPEAT", "REPLACE", "REQUIRE", "RESTRICT",
    "RETURN", "RETURNS", "REVOKE", "RIGHT", "ROLLBACK", "ROW", "ROWNUM", "ROWS", "SCHEMA",
    "SELECT", "SESSION_USER", "SET", "SHOW", "SIGNED", "SOME", "SPATIAL", "SQL", "START",
    "STRAIGHT_JOIN", "SYMMETRIC", "SYSTEM_USER", "TABLE", "TERMINATED", "THEN", "TO", "TRAILING",
    "TRANSACTION", "TRIGGER", "TRUE", "TRUNCATE", "UNDO", "UNION", "UNIQUE", "UNLOCK", "UNSIGNED",
    "UPDATE", "USAGE", "USE", "USER", "USING", "VALUES", "VARBINARY", "VARCHAR", "VARYING",
    "VIEW", "WHEN", "WHERE", "WHILE", "WINDOW", "WITH", "WRITE", "XOR", "ZEROFILL",
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

    fn key(&self) -> &'static str {
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
        matches!(
            self.key(),
            "mysql" | "mariadb" | "postgresql" | "kingbase" | "sqlite" | "h2" | "oracle" | "dm"
                | "db2" | "sqlserver" | "derby"
        )
    }

    /// 原生 upsert 单条语句最多带多少行（调用方按它拆分）。
    pub fn upsert_row_limit(&self) -> usize {
        match self.key() {
            // T-SQL 的行值构造器单语句上限 1000 行（解析器硬限制，与 INSERT 同一条规矩）
            "sqlserver" => 1000,
            // Oracle/DM 的 MERGE 源会把每一行展开成 `select … from dual`，再拼成
            // UNION ALL。页大小（5 万）直接作为分支数会让解析器/表达式上限先于
            // 数据量爆炸；500 行与旧 INSERT 路径的保守批大小一致。
            "oracle" | "dm" => 500,
            // Derby 的 `values` 表构造器行数一多，解析与优化都比别的库贵；1000 行一条
            // 与它自己的 `insert ... values` 批大小一致，不额外引入风险。
            "derby" => 1000,
            _ => usize::MAX,
        }
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
        if rows.is_empty() || columns.is_empty() || key_indexes.is_empty() {
            return None;
        }
        let q = |name: &str| self.quote(name);
        let col_list = columns.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ");
        let keys = key_indexes.iter().map(|i| q(&columns[*i])).collect::<Vec<_>>();
        let key_set: std::collections::HashSet<usize> = key_indexes.iter().copied().collect();
        // 需要被更新的列 = 除键以外的列（全是键时语句退化成「插入或忽略」）
        let payload: Vec<usize> = (0..columns.len()).filter(|i| !key_set.contains(i)).collect();
        let tuples = rows
            .iter()
            .map(|row| format!("({})", row.join(", ")))
            .collect::<Vec<_>>()
            .join(", ");
        let insert_from_src = |sql: &mut String| {
            let src_cols = (0..columns.len())
                .map(|i| format!("src.{}", q(&columns[i])))
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!(
                " when not matched then insert ({col_list}) values ({src_cols})"
            ));
        };
        match self.key() {
            "mysql" | "mariadb" => {
                if payload.is_empty() {
                    let first = keys.first()?;
                    return Some(format!(
                        "insert into {table} ({col_list}) values {tuples} on duplicate key update {first} = {first}"
                    ));
                }
                let sets = payload
                    .iter()
                    .map(|i| {
                        let c = q(&columns[*i]);
                        format!("{c} = values({c})")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(format!(
                    "insert into {table} ({col_list}) values {tuples} on duplicate key update {sets}"
                ))
            }
            "postgresql" | "kingbase" | "sqlite" => {
                let target = keys.join(", ");
                if payload.is_empty() {
                    return Some(format!(
                        "insert into {table} ({col_list}) values {tuples} on conflict ({target}) do nothing"
                    ));
                }
                let sets = payload
                    .iter()
                    .map(|i| {
                        let c = q(&columns[*i]);
                        format!("{c} = excluded.{c}")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(format!(
                    "insert into {table} ({col_list}) values {tuples} on conflict ({target}) do update set {sets}"
                ))
            }
            "h2" => Some(format!(
                "merge into {table} ({col_list}) key ({}) values {tuples}",
                keys.join(", ")
            )),
            "oracle" | "dm" => {
                // Oracle 的 MERGE 源用 `select … from dual union all`；表别名不能带 AS
                let branches = rows
                    .iter()
                    .map(|row| {
                        let exprs = columns
                            .iter()
                            .enumerate()
                            .map(|(i, c)| format!("{} as {}", row[i], q(c)))
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("select {exprs} from dual")
                    })
                    .collect::<Vec<_>>()
                    .join(" union all ");
                let on = key_indexes
                    .iter()
                    .map(|i| {
                        let c = q(&columns[*i]);
                        format!("tgt.{c} = src.{c}")
                    })
                    .collect::<Vec<_>>()
                    .join(" and ");
                let mut sql = format!("merge into {table} tgt using ({branches}) src on ({on})");
                if !payload.is_empty() {
                    let sets = payload
                        .iter()
                        .map(|i| {
                            let c = q(&columns[*i]);
                            format!("{c} = src.{c}")
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    sql.push_str(&format!(" when matched then update set {sets}"));
                }
                insert_from_src(&mut sql);
                Some(sql)
            }
            // Derby 的 MERGE 与 DB2 同形（`using (values …) as src (列清单)`），
            // 10.11 起支持 —— 以前这里没有它，Derby 目标只能纯 insert，
            // 重跑撞主键就把行判成脏数据（唯一没有幂等的 JDBC 关系型库）。
            "db2" | "sqlserver" | "derby" => {
                let on = key_indexes
                    .iter()
                    .map(|i| {
                        let c = q(&columns[*i]);
                        format!("tgt.{c} = src.{c}")
                    })
                    .collect::<Vec<_>>()
                    .join(" and ");
                let mut sql = format!(
                    "merge into {table} as tgt using (values {tuples}) as src ({col_list}) on ({on})"
                );
                if !payload.is_empty() {
                    let sets = payload
                        .iter()
                        .map(|i| {
                            let c = q(&columns[*i]);
                            format!("{c} = src.{c}")
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    sql.push_str(&format!(" when matched then update set {sets}"));
                }
                insert_from_src(&mut sql);
                // T-SQL 要求 MERGE 语句以分号结束（老版本解析器硬要求）
                if self.key() == "sqlserver" {
                    sql.push(';');
                }
                Some(sql)
            }
            _ => None,
        }
    }

    /// 标识符加引号：简单名不加，避免把 MySQL 的大小写敏感表名改写掉。
    /// 带 `.` 或 `-` 之类的名字才逐段加引号。
    ///
    /// **保留字必须加引号**（`is_reserved`）：一张叫 `user` 的表会让服务端生成的
    /// `delete from user` 直接语法错（SQL Server errorCode 156，MySQL 的 `order`/`key`
    /// 同理）。实测：库里那张 `user` 表，菜单「清空表」必然失败，而同名手写
    /// `select * from [user]` 完全正常 —— 坏的只是应用自己拼的 SQL。
    pub fn quote(&self, name: &str) -> String {
        let wrap = |s: &str| -> String {
            if s.is_empty() {
                return String::new();
            }
            let plain = s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
            // **下折方言**（PG / Kingbase / SQLite）里未加引号的名字会被折成小写：
            // 目标表上叫 `Name` 的列，裸写 `Name` 会被解析成 `name` —— 真机报
            // `column "name" of relation "t" does not exist`，而表结构明明是好的。
            // 这类「含大写字母」的普通名必须逐字加引号；全小写的普通名照旧裸写（行为不变）。
            let needs_case_quote =
                plain && self.folds_ident_lower() && s.chars().any(|c| c.is_ascii_uppercase());
            if plain && !is_reserved(s) && !needs_case_quote {
                return s.to_string();
            }
            // 上折方言（Oracle / DM / DB2 / H2 / Derby）里，未加引号的名字会被折成大写；
            // 一旦加引号就等于固定大小写。所以要加引号的**普通名**先折一次，
            // 让 `rename to user` 仍然建出 `USER`，而不是悄悄造一个小写对象。
            let text = if plain && self.folds_ident_upper() {
                s.to_ascii_uppercase()
            } else {
                s.to_string()
            };
            match self.key() {
                "mysql" | "mariadb" | "doris" | "clickhouse" => {
                    format!("`{}`", text.replace('`', "``"))
                }
                "sqlserver" => format!("[{}]", text.replace(']', "]]")),
                _ => format!("\"{}\"", text.replace('"', "\"\"")),
            }
        };
        name.split('.').map(wrap).collect::<Vec<_>>().join(".")
    }

    /// 未加引号的标识符是否会被折成**小写**（PG 系）。
    ///
    /// 与 [`Self::folds_ident_upper`] 相对：这些库里含大写字母的名字必须加引号，
    /// 否则会被悄悄折成小写、指向另一个（或不存在的）列。
    fn folds_ident_lower(&self) -> bool {
        matches!(self.key(), "postgresql" | "kingbase" | "sqlite")
    }

    /// 未加引号的标识符是否会被折成大写（决定加引号前要不要先上折）。
    fn folds_ident_upper(&self) -> bool {
        matches!(self.key(), "oracle" | "dm" | "db2" | "h2" | "derby")
    }

    /// 字符串字面量（按方言转义）。
    ///
    /// MySQL 系（`key()=="mysql"`，含 MariaDB/Doris）与 ClickHouse 默认把 `\` 当转义符：
    /// 值里的反斜杠必须**先**翻倍再转义单引号，否则 `a\` 这样以 `\` 收尾的文本会把
    /// 收尾引号转义掉 —— 导出/同步生成的 INSERT/UPDATE 被截断甚至注入
    /// （`\'` 提前闭合字符串）。其余方言（PG/SQLServer/Oracle/SQLite/H2…）里 `\`
    /// 就是普通字符，动了反而改值，不动。
    pub fn literal(&self, value: &str) -> String {
        let body = if matches!(self.key(), "mysql" | "clickhouse") {
            value.replace('\\', "\\\\")
        } else {
            value.to_string()
        };
        format!("'{}'", body.replace('\'', "''"))
    }

    pub fn is_schema_aware(&self) -> bool {
        // 这里的名单必须与 YAML 的 `traits.schemaAware` 对齐 —— 以前它写死三个类型，
        // 于是 oracle 明明在 YAML 里写着 schemaAware: true，行为却和 false 一样：
        // 「库.模式」不拆、表/列都按会话默认 schema 取，用户在 system 会话里看不到
        // DBMIND 的表（连表清单都会串成 SYS 的）。
        matches!(self.key(), "postgresql" | "kingbase" | "sqlserver" | "oracle")
    }

    /// 树里「库」这一层装的其实就是**模式**（Oracle / DM：连接下直接列模式，没有服务名层）。
    ///
    /// 用途：接口只收到 `database=SYSTEM` 这种值、拆不出「库.模式」时，把它当模式用 ——
    /// 否则搜索 / ER 图会退化成「枚举当前账号能看到的**所有**模式」，一行输入等 60 秒，
    /// 结果还会混进别的模式的表（真机：Oracle SYSTEM 下搜一个表名要 63 秒）。
    pub fn db_is_schema(&self) -> bool {
        matches!(self.key(), "oracle")
    }

    /// 这个类型的元数据 SQL 是「库内限定」的，执行前要补一句 `use [库];` 兜底
    /// （SQL Server 专属：影子连接负责落库，USE 是第二道保险 —— 防止执行落回主连接
    /// 时把默认库的表当成目标库的，两个库节点显示同一份清单，用户实测过）。
    pub fn wants_use_switch(&self) -> bool {
        matches!(self.key(), "sqlserver")
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
        matches!(
            self.key(),
            "mysql" | "mariadb" | "doris" | "postgresql" | "kingbase" | "sqlserver" | "clickhouse"
        )
    }

    /// 是否为「catalog → 库 → 表」三层结构。
    ///
    /// Doris 的 catalog 是比「库」更高的一层：跨 catalog 要用全限定名
    /// `catalog.库.表`，而 `information_schema` 只反映**当前 catalog**。
    /// 目前只有 Doris；其余类型返回 false，走原来的两层结构，行为完全不变。
    pub fn catalog_level(&self) -> bool {
        matches!(self.key(), "doris")
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
        match self.key() {
            "doris" => Meta::Sql("show catalogs".to_string()),
            _ => Meta::Absent,
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
        match self.key() {
            "sqlserver" => Meta::Sql(
                "select name from sys.databases where state = 0 and has_dbaccess(name) = 1 order by name"
                    .to_string(),
            ),
            "mysql" | "mariadb" | "doris" => Meta::Sql("show databases".to_string()),
            "postgresql" | "kingbase" => Meta::Sql(
                "select datname as name from pg_database where datistemplate = false \
                 and has_database_privilege(datname, 'CONNECT') order by datname"
                    .to_string(),
            ),
            "clickhouse" => {
                Meta::Sql("select name from system.databases order by name".to_string())
            }
            // 文件型（一个文件就是一个库）与尚未覆盖的类型：交给调用方退回单库
            _ => Meta::Unwritten,
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
        match self.key() {
            // PG：`pg_namespace` 是**公开目录**，所有角色都能看到全部 schema 名 ——
            // 不按权限收窄的话，树上会列出一堆点进去空、连建表都不允许的模式（实测：
            // 给低权限账号建一个无 USAGE 的 schema，改造前它就出现在清单里）。
            // 判据用 `USAGE`：没有它连按名字定位对象都不行，等价于「这个模式对它不可用」。
            "postgresql" | "kingbase" => Meta::Sql(
                "select nspname as name from pg_namespace \
                 where nspname not like 'pg\\_%' and nspname <> 'information_schema' \
                   and has_schema_privilege(oid, 'USAGE') order by nspname"
                    .to_string(),
            ),
            // SQL Server：除了 guest / sys / INFORMATION_SCHEMA，还要滤掉**九个固定数据库角色的同名
            // schema**（db_owner、db_accessadmin、db_backupoperator、db_datareader、db_datawriter、
            // db_ddladmin、db_denydatareader、db_denydatawriter、db_securityadmin）。
            // 每个固定角色在库里都对应一个空 schema，SQL Server 建库时自动带上 —— 它们不是给用户
            // 建表用的，列在树里就是九行永远空的噪音（实测用户截图里正是这九个）。
            // `dbo` 要留着：它是正常的默认 schema（用户建的表大多在它下面）。
            "sqlserver" => Meta::Sql(
                "select name from sys.schemas \
                 where name not in ('guest','sys','INFORMATION_SCHEMA', \
                   'db_owner','db_accessadmin','db_backupoperator','db_datareader','db_datawriter', \
                   'db_ddladmin','db_denydatareader','db_denydatawriter','db_securityadmin') \
                 order by name"
                    .to_string(),
            ),
            // 其余类型的 schemaLevel 在前端就是 'none'，树不会请求它；返回空数组是事实。
            // Oracle：「库」那一层是服务名 / PDB，表实际挂在**用户（schema）**下面 ——
            // 树里列 all_users，就是 Navicat 左侧那一列（SYS / SYSTEM / OUTLN / … 都在）。
            // 不开这层的话，表清单只能看到当前会话 schema 自己的表：连 system 时看不到
            // DBMIND.T_TEST 的字段。
            // Oracle：`all_users` 也是**所有**用户（含 SYS / OUTLN 这些与当前账号毫无关系的），
            // 逐个点进去恒空。收窄成「当前用户 + 至少有一个我能看见的对象」——
            // `all_objects` 的语义正好是「当前账号有权限访问的对象」，所以这就是"有权限的 schema"。
            // 当前用户必须无条件保留：刚建好、还没建对象的空 schema 也得能导航过去。
            "oracle" => Meta::Sql(
                "select username as name from all_users u \
                 where username = sys_context('USERENV','CURRENT_SCHEMA') \
                    or exists (select 1 from all_objects o \
                               where o.owner = u.username and rownum = 1) \
                 order by username"
                    .to_string(),
            ),
            // H2：`INFORMATION_SCHEMA.SCHEMATA`（2.x）。H2 的「库」是 catalog（一个实例一个，
            // 名字就是连接里的 database，如 MAIN），表和授权都挂在 schema 上 ——
            // 用户管理的表级授权要选的就是这一层（PUBLIC / 业务 schema）。
            "h2" => Meta::Sql(
                "select schema_name as \"name\" from information_schema.schemata \
                 where schema_name <> 'INFORMATION_SCHEMA' order by schema_name"
                    .to_string(),
            ),
            _ => Meta::Absent,
        }
    }

    /// 某个 schema 下的表/视图（schema 层级专用；非 schema 类型走内核的 DatabaseMetaData）。
    pub fn tables_in_schema(&self, schema: &str) -> Meta {
        match self.key() {
            // PostgreSQL / KingbaseES：`reltuples` 是**估算值**（未 ANALYZE 时是 -1），
            // 拿不到就给 null —— 界面上留空，不要用 0 冒充「空表」。
            // 精确行数由前端展开后的批量 COUNT(*) 回填（见 `/table-count`）。
            // PG：`pg_class` 同样是公开目录 —— 不加权限判断就会把「这个账号一张都读不了」
            // 的表/视图列出来（实测低权限账号看到了没授过任何权限的表与视图）。
            // 判据是「七种表级权限里**任意一种**」：只授了 INSERT 的表也应当能看到，
            // 只看 SELECT 会把它藏起来（那样更糟：用户明明能写，却在树里找不到）。
            // 逗号列表是 `has_table_privilege` 支持的写法（实测），不必写七个 or。
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select c.relname as name, \
                 case when c.relkind = 'v' then 'VIEW' else 'BASE TABLE' end as type, \
                 case when c.reltuples < 0 then null else c.reltuples::bigint end as rows \
                 from pg_class c join pg_namespace n on n.oid = c.relnamespace \
                 where n.nspname = {} and c.relkind in ('r','p','v','f') \
                   and has_table_privilege(c.oid, \
                     'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') \
                 order by c.relname",
                self.literal(schema)
            )),
            // SQL Server：`sys.partitions.rows` 是**实时维护的真实行数**（不是统计估算），
            // 一次 join 就能拿到全 schema，不必每张表去 COUNT(*)。
            // 视图没有自己的行数 → null（留给前端留空，而不是显示 0）。
            //
            // 表注释必须自己 join：**mssql-jdbc 不填 `REMARKS`**（SQL Server 的表注释存在
            // `sys.extended_properties` 里，不是数据库自带的元数据），所以走 JDBC 的
            // getTables 永远拿不到注释 —— 而前端「编辑表结构」的表注释框正是从这个清单里读的
            // （`comment: t?.comment || ''`）。缺这一列的后果实测是：注释改完保存**确实成功**
            // （`/alter` 里那条 sp_updateextendedproperty 真执行了，库里值也变了），
            // 但重新打开表时注释框又是空的 —— 用户看到的就是"修改表注释没用"。
            // `minor_id = 0` 才是表级注释（列级注释的 minor_id 是列号）；`class = 1` 是对象级。
            "sqlserver" => Meta::Sql(format!(
                "select o.name as name, \
                 case when o.type = 'V' then 'VIEW' else 'BASE TABLE' end as type, \
                 case when o.type = 'V' then null else isnull(p.rows, 0) end as rows, \
                 cast(ep.value as nvarchar(4000)) as comment \
                 from sys.objects o \
                 outer apply (select sum(rows) as rows from sys.partitions \
                              where object_id = o.object_id and index_id in (0,1)) p \
                 left join sys.extended_properties ep \
                        on ep.class = 1 and ep.major_id = o.object_id and ep.minor_id = 0 \
                       and ep.name = 'MS_Description' \
                 where o.schema_id = schema_id({}) and o.type in ('U','V') order by o.name",
                self.literal(schema)
            )),
            // Oracle：表与视图一起列（union all 两段），并且**必须带上 `type` 列** ——
            // 前端就是按 `type === 'VIEW'` 把视图分到「视图」分类的：少了这一列，
            // 视图会被当成表混进「表」里，「视图」节点永远是空的。
            // （真机踩过：SYSTEM 下 41 个对象、视图 0 —— 就是这个原因。）
            // `rows` / `comment` / `type` 都加双引号：前两个在 Oracle 里是关键字，
            // 不加会直接 ORA-00923。num_rows 是**统计估算**（没 ANALYZE 过就是 null），
            // 精确行数由前端 COUNT(*) 回填；注释取自 all_tab_comments（表和视图都在里面），
            // 结构编辑器的「表注释」框就是从这个清单里读的。
            // SQL 写成**一整行**：续行拼接被搅坏过一次（报 ORA-03405），别再拆。
            "oracle" => Meta::Sql(format!(
                "select t.table_name as name, t.num_rows as \"rows\", cc.comments as \"comment\", 'TABLE' as \"type\" from all_tables t left join all_tab_comments cc on cc.owner = t.owner and cc.table_name = t.table_name where t.owner = {owner} union all select v.view_name as name, null as \"rows\", cc.comments as \"comment\", 'VIEW' as \"type\" from all_views v left join all_tab_comments cc on cc.owner = v.owner and cc.table_name = v.view_name where v.owner = {owner} order by name",
                owner = self.literal(&schema.to_uppercase())
            )),
_ => Meta::Unwritten,
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
    pub fn user_info(&self, name: &str) -> Meta {
        let n = self.literal(name);
        match self.key() {
            // SQL Server：**服务器登录名**（sys.server_principals）+ 服务器角色 + 服务器级权限。
            // 与 users() 同一口径（连接级、不跟库）—— 见那边的注释。
            // 别名必须是 `type_desc`（不是 `type`）：前端标签表按 `type_desc` 认「类型」。
            "sqlserver" => Meta::Sql(format!(
                "select sp.name as name, sp.type_desc as type_desc, \
                 convert(varchar(30), sp.create_date, 120) as create_date, \
                 isnull(sp.default_database_name, '') as default_database, \
                 cast(sp.is_disabled as int) as disabled, \
                 'create login ' + quotename(sp.name) + ' with password = ''<password>''' as create_user_sql, \
                 isnull(stuff(( \
                   select char(10) + 'alter server role ' + quotename(r.name) + ' add member ' + quotename(sp2.name) + ';' \
                   from sys.server_role_members m \
                   join sys.server_principals r on r.principal_id = m.role_principal_id \
                   join sys.server_principals sp2 on sp2.principal_id = m.member_principal_id \
                   where m.member_principal_id = sp.principal_id \
                   for xml path(''), type).value('.', 'nvarchar(max)'), 1, 1, ''), '') + \
                 isnull(( \
                   select char(10) + 'grant ' + p.permission_name + ' to ' + quotename(sp2.name) + ';' \
                   from sys.server_permissions p \
                   join sys.server_principals sp2 on sp2.principal_id = p.grantee_principal_id \
                   where p.grantee_principal_id = sp.principal_id and p.class = 100 \
                   for xml path(''), type).value('.', 'nvarchar(max)'), '') as grants_text \
                 from sys.server_principals sp \
                 where sp.name = {n} and sp.type in ('S','U','G')"
            )),
            // H2：`INFORMATION_SCHEMA.USERS` + `TABLE_PRIVILEGES`（实测 2.3.232）。
            // H2 没有"全局权限"这一层，只有 ADMIN 标志；细粒度授权就是表级 GRANT。
            // 标识符一律双引号（H2 的标准引用符），内部 `"` 转义成 `""`。
            "h2" => {
                let who = self.literal(&h2_user(name));
                Meta::Sql(format!(
                    "select case when coalesce(user_name,'') = '' then '\"\"' else user_name end as \"name\", \
                     case when is_admin then 'true' else 'false' end as \"is_admin\", \
                     'create user \"' || replace(case when coalesce(user_name,'') = '' then 'new_user' else user_name end, '\"', '\"\"') || '\" password ''<password>''' as \"create_user_sql\", \
                     coalesce((select listagg('grant ' || privilege_type || ' on \"' || replace(table_schema, '\"', '\"\"') || '\".\"' || replace(table_name, '\"', '\"\"') || '\" to \"' || replace(grantee, '\"', '\"\"') || '\"', char(10)) within group (order by table_schema, table_name, privilege_type) \
                       from information_schema.table_privileges where grantee = user_name), '') as \"grants_text\" \
                     from information_schema.users where user_name = {who}"
                ))
            }
            // Derby / DB2：用户来自外部认证，只能看（清单见 users()），没有可执行的建/删语句
            "derby" => Meta::Sql(format!(
                "select username as \"name\" from sys.sysusers where username = {n}"
            )),
            "db2" => Meta::Sql(format!(
                "select grantee as \"name\" from syscat.dbauth where grantee = {n}"
            )),
            // MySQL 的账号是**两列**（user + host），而树里显示与传进来的是拼好的
            // `user@host`（列表就是这么给的）—— 老写法拿 `demo1@%` 去比 `user` 列，
            // 于是点开任何一个用户都是「没有找到用户」。按**最后一个** `@` 切开再比两列：
            // host 部分不含 `@`，而用户名部分可以（`'a@b'@'%'`）。
            "mysql" | "mariadb" | "doris" => {
                let (user, host) = match name.rsplit_once('@') {
                    Some((user, host)) => (user, host),
                    // 只给了用户名（没有 @）时不限制 host，否则会与 `'x'@'localhost'` 擦肩而过
                    None => (name, ""),
                };
                let filter = if host.is_empty() {
                    format!("user = {}", self.literal(user))
                } else {
                    format!(
                        "user = {} and host = {}",
                        self.literal(user),
                        self.literal(host)
                    )
                };
                Meta::Sql(format!(
                    // 权限明细**不在这里聚合**：`group_concat` 受 `group_concat_max_len`
                    // 限制（服务端默认 1024）会静默截断，见 `user_grants`。
                    // 拼接用 concat：MySQL 里 `'a' + 'b'` 是**加法**（结果是 0），
                    // 老写法直接抄了 SQL Server 的 `+`，创建语句会显示成一串数字
                    "select t.user as name, t.host as host, \
                     concat('create user ', quote(t.user), '@', quote(t.host)) as create_user_sql, \
                     t.* \
                     from mysql.user t where {filter} limit 1"
                ))
            }
            // PG / Kingbase：这一条原来是从 SQL Server 那份**抄过来的**，两处语法在 PG 上都不成立：
            //   · `'a' + 'b'`：PG 的字符串拼接是 `||`，`+` 只有数值有 —— 直接报
            //     `operator does not exist: unknown + text`（42883），整条详情查询失败；
            //   · `isnull(x, '')`：那是 SQL Server 的函数，PG 只有 `coalesce`（42883 同名错误）。
            // 结果是「PG 连用户清单都看得到、点进去却是没有找到用户」。这里的 `||` 与 `coalesce`
            // 两家（PG / KingbaseES 的 PG 兼容模式）都认。
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select r.rolname as name, \
                 cast(r.rolsuper as int) as is_superuser, \
                 cast(r.rolcanlogin as int) as can_login, \
                 cast(r.rolcreatedb as int) as can_create_db, \
                 'create role ' || quote_ident(r.rolname) || ' login' as create_user_sql, \
                 coalesce(string_agg(case when a.privilege_type is not null then \
                   'grant ' || a.privilege_type || ' on ' || quote_ident(a.table_schema) || '.' || \
                   quote_ident(a.table_name) || ' to ' || quote_ident(r.rolname) || ';' end, chr(10)), '') as grants_text \
                 from pg_roles r \
                 left join information_schema.table_privileges a on a.grantee = r.rolname \
                 where r.rolname = {n} \
                 group by r.rolname, r.rolsuper, r.rolcanlogin, r.rolcreatedb"
            )),
            // ClickHouse：system.users 一行就是全部属性；授权清单是 `show grants for`
            //（见 user_grants 的 clickhouse 分支）。认证串（auth_string）不进详情 ——
            // 与 MySQL 的 authentication_string 同一个理由：展示出来只会被复制、截图。
            // 只挑**版本稳定**的列：default_roles_* / grantees_* 在新版本改过名
            // （实测 26.8 报 Unknown identifier: grantees_all），别把它们摊进详情。
            "clickhouse" => Meta::Sql(format!(
                // `auth_type` 是 **Enum8**（不是数组）：24.8 实测 arrayStringConcat 会抛
                // Code 43 ILLEGAL_TYPE_OF_ARGUMENT。转成字符串即可；host_ip/host_names
                // 才是 Array(String)，继续用 arrayStringConcat 拼成一行。
                "select name, storage, \
                 toString(auth_type) as auth_type, \
                 arrayStringConcat(host_ip, ', ') as host_ip, \
                 arrayStringConcat(host_names, ', ') as host_names \
                 from system.users where name = {n}"
            )),
            // Oracle / DM：dba_users 一行含状态与表空间；授权拼成 grants_text 文本
            //（系统权限 + 角色，与 PG 的 grants_text 同一交付形态）。
            // 用 dba_* 而不是 all_users：后者没有状态/表空间字段，详情页会没内容可看。
            // `upper()` 兜住大小写 —— Oracle 里用户名按大写存。
            "oracle" | "dm" => Meta::Sql(format!(
                "select username as name, account_status, \
                 to_char(created, 'YYYY-MM-DD HH24:MI:SS') as created, \
                 nvl(default_tablespace, '') as default_tablespace, \
                 nvl(temporary_tablespace, '') as temporary_tablespace, \
                 nvl(profile, '') as profile, \
                 nvl((select listagg('grant ' || privilege || ' to ' || grantee || ';', chr(10)) \
                   within group (order by privilege) from dba_sys_privs s where s.grantee = u.username), '') || \
                 nvl((select chr(10) || listagg('grant ' || granted_role || ' to ' || grantee || ';', chr(10)) \
                   within group (order by granted_role) from dba_role_privs r where r.grantee = u.username), '') as grants_text \
                 from dba_users u where username = upper({n})"
            )),
            _ => Meta::Unwritten,
        }
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
        // grantee 在 information_schema 视图里是**带引号的字符串** `'user'@'host'`：
        // 在 Rust 侧拼好整个串再走 literal() 转义 —— 比 `concat(quote(…))` 短得多。
        // **长度就是正确性**：实测（公司 VPN，路由被隧道接管且上行丢弃满 MSS 的包），
        // 整句超过一个 TCP 段（≈1.4KB）时第一个满尺寸段被丢，重传退避 ~19 秒后连接
        // 被重置，界面表现就是「点编辑/授权没反应」。所以列名/字面量一律从短。
        let grantee = self.literal(&format!("'{}'@'{}'", user, host));
        match self.key() {
            "mysql" | "mariadb" | "doris" => {
                // 例程级授权不在 information_schema 里，只在 mysql.procs_priv：一行多权限、
                // 空格分隔（`Execute Alter Routine Grant`）。**不在 SQL 里按 LIKE 拆成三段** ——
                // 那会把整句顶过一个 TCP 段（见上），拆分挪到 Rust 侧做（expand_grant_rows）。
                // 一行带回原始 Proc_priv + `类型:名字`（PROCEDURE:p / FUNCTION:f）。
                // 第 5 列 is_grantable 直接随行带回（替代「GRANT OPTION 单独三段 UNION」）。
                // Doris 是 MySQL 协议但没有 mysql.procs_priv 这张表，查了必炸 —— 排除。
                let routine_sql = if self.key() == "doris" {
                    String::new()
                } else {
                    format!(
                        " union all select 'routine', Proc_priv, Db, concat(Routine_type, ':', Routine_name), '' from mysql.procs_priv where User = {} and Host = {}",
                        self.literal(user),
                        self.literal(host)
                    )
                };
                Meta::Sql(format!(
                    "select 'global' as scope, privilege_type as privilege, '' as db_name, '' as table_name, is_grantable as grant_option from information_schema.user_privileges where grantee = {grantee} \
                 union all select 'schema', privilege_type, table_schema, '', is_grantable from information_schema.schema_privileges where grantee = {grantee} \
                 union all select 'table', privilege_type, table_schema, table_name, is_grantable from information_schema.table_privileges where grantee = {grantee}{routine_sql}"
                ))
            }
            // PostgreSQL/KingbaseES：模式（USAGE/CREATE）/ 表 / 例程（EXECUTE）三层，
            // 各来自一张 information_schema 视图。grantee 是**裸角色名**（没有 MySQL 的
            // 'u'@'h' 形状）。information_schema 只列「显式授给这个角色」的权限：
            // 通过角色继承来的不在内 —— 与 SUPERUSER 全能也不在内，如实展示。
            // 整句同样要压在一个 TCP 段以内（见上方 mysql 分支的 VPN 实测注释）。
            "postgresql" | "kingbase" => {
                let who = self.literal(user);
                Meta::Sql(format!(
                    "select 'schema' as scope, privilege_type as privilege, object_name as db_name, '' as table_name from information_schema.usage_privileges where grantee = {who} and object_type = 'SCHEMA' and object_name not in ('information_schema','pg_catalog') \
                 union all select 'table', privilege_type, table_schema, table_name from information_schema.role_table_grants where grantee = {who} and table_schema not in ('information_schema','pg_catalog') \
                 union all select case when p.prokind = 'p' then 'procedure' else 'function' end as scope, g.privilege_type as privilege, g.specific_schema as db_name, g.routine_name as table_name from information_schema.role_routine_grants g join pg_proc p on p.proname = g.routine_name join pg_namespace pn on pn.oid = p.pronamespace and pn.nspname = g.specific_schema where g.grantee = {who} and g.specific_schema not in ('information_schema','pg_catalog') \
                 union all select 'database' as scope, a.privilege_type as privilege, d.datname as db_name, '' as table_name from pg_database d cross join lateral aclexplode(coalesce(d.datacl, acldefault('d', d.datdba))) a join pg_roles r on r.oid = a.grantee where r.rolname = {who} and not d.datistemplate"
                ))
            }
            // ClickHouse：`show grants for` 每行就是一条可执行的 GRANT 语句，
            // 由 grant_line 的 clickhouse 分支**原样**带回 —— 不做任何解析重组。
            "clickhouse" => {
                Meta::Sql(format!("show grants for {}", self.quote(user)))
            }
            // Oracle：三张字典视图正好对应三层 —— 系统权限 / 角色 / 对象权限。
            // 列形状与 MySQL 那三张视图**保持一致**（scope, privilege, owner, object），
            // 前端一套解析器就能吃两家，不再各写一份（那条老教训）。
            // 视图用 `dba_*`：它们要 SELECT_CATALOG_ROLE —— 没有权限时这句会失败，
            // 界面会**如实报"读不到现有权限"**（而不是显示成"什么都没有"，那会让人
            // 误以为可以放心保存）。SQL 一律写成**单行**：续行的反斜杠拼接被搅坏过。
            // 注：权限名用 `PRIVILEGE` 而不是 `PRIVILEGE_CN`，避免用到只有中文环境才有的列。
            "oracle" => Meta::Sql(format!(
                "select 'global' as scope, privilege as privilege, '' as owner, '' as object_name from dba_sys_privs where grantee = upper({}) union all select 'role', granted_role, '', '' from dba_role_privs where grantee = upper({}) union all select 'object', privilege, owner, table_name from dba_tab_privs where grantee = upper({})",
                self.literal(user), self.literal(user), self.literal(user)
            )),
            // H2：细粒度授权只有表级一层（H2 没有全局权限，只有 ADMIN 标志）
            "h2" => Meta::Sql(format!(
                "select 'table' as scope, privilege_type as privilege, table_schema as db_name, table_name as table_name from information_schema.table_privileges where grantee = {}",
                self.literal(&h2_user(user))
            )),
            _ => Meta::Unwritten,
        }
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
        match self.key() {
            "mysql" => Meta::Sql(format!(
                "select from_user as role_name, from_host as role_host from mysql.role_edges \
                 where to_user = {} and to_host = {}",
                self.literal(user),
                self.literal(host)
            )),
            "mariadb" => Meta::Sql(format!(
                "select Role as role_name, '' as role_host from mysql.roles_mapping \
                 where User = {} and Host = {}",
                self.literal(user),
                self.literal(host)
            )),
            // ClickHouse：授权关系在 `system.role_grants`（granted_role_name 才是"被授角色"）
            "clickhouse" => Meta::Sql(format!(
                "select granted_role_name as role_name, '' as role_host from system.role_grants \
                 where user_name = {}",
                self.literal(user)
            )),
            // PostgreSQL/KingbaseES：成员关系在 `pg_auth_members`（member = 被授主体，
            // roleid = 被授的角色）。**information_schema 里查不到这一层** ——
            // `role_usage_grants` 只讲 USAGE 权限、`enabled_roles` 只看当前会话，
            // 所以 PG 的「角色」页签原来一个勾都没有（授过的角色既看不见也撤不掉）。
            // 与 MySQL 一样单独一条短查询：不塞进 user_grants 那条已经贴着 TCP 段上限的 UNION。
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select r.rolname as role_name, '' as role_host from pg_auth_members m \
                 join pg_roles r on r.oid = m.roleid \
                 join pg_roles u on u.oid = m.member where u.rolname = {} order by 1",
                self.literal(user)
            )),
            _ => Meta::Unwritten,
        }
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
        match self.key() {
            // MySQL 的角色就是账号（host 维度存在）；MariaDB 的角色 Host 恒为空串
            "mysql" | "mariadb" => Meta::Sql(format!(
                "select user as `name`, host as `host` from mysql.user where user = {} order by host limit 1",
                self.literal(name)
            )),
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select rolname as name, '' as host from pg_roles where rolname = {}",
                self.literal(name)
            )),
            "oracle" => Meta::Sql(format!(
                "select role as name, '' as host from dba_roles where role = upper({})",
                self.literal(name)
            )),
            "clickhouse" => Meta::Sql(format!(
                "select name, '' as host from system.roles where name = {}",
                self.literal(name)
            )),
            // Doris 这版不支持编辑角色权限（权限名是 SELECT_PRIV 那套，与 MySQL 不同名），
            // 这里仍给一行，至少不会把表单卡在"没有找到"
            "doris" => Meta::Sql(format!(
                "select {} as name, '' as host",
                self.literal(name)
            )),
            _ => Meta::Unwritten,
        }
    }

    /// 角色**自身拥有**的权限（与 `user_grants` 同形状：scope/privilege/owner/db_name/table_name/grant_option）。
    pub fn role_grants(&self, name: &str, host: &str) -> Meta {
        match self.key() {
            // MySQL：角色就是账号，权限视图里按 grantee 能查到，直接复用
            "mysql" => self.user_grants(name, host),
            // MariaDB：information_schema **不列角色**（实测 user_privileges 里查不到），
            // 只能读 `show grants for '<role>'`。返回的是**单列文本**（每行一条 GRANT），
            // 由 `grant_line` 原样带回 `grants`，界面（parseMySqlGrants）再分层回填 ——
            // 与 ClickHouse 同一种交付形态（grantRows 为空，结构化列只给"有 4 列形状"的方言）。
            "mariadb" => Meta::Sql(format!("show grants for {}", self.literal(name))),
            // PG：information_schema 只显示「当前用户是授予者/被授予者」的那部分，
            // 读别的角色的权限永远是空 —— 必须直接解 ACL 数组（aclexplode）。
            // `coalesce(acl, acldefault(...))`：ACL 为 NULL = 从没单独授过，
            // 此时按"只有属主有权限"的默认 ACL 展开（角色是属主时才会命中）。
            // 注意：模式/表两级用的是**当前库**的目录（pg_namespace / pg_class），
            // 其它库里的对象要在那个库上打开才看得到；库级（pg_database）是全局的。
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select 'schema' as scope, a.privilege_type as privilege, n.nspname as db_name, '' as table_name \
                   from pg_namespace n cross join lateral aclexplode(coalesce(n.nspacl, acldefault('n', n.nspowner))) a \
                   join pg_roles r on r.oid = a.grantee \
                  where r.rolname = {who} and n.nspname not in ('information_schema','pg_catalog') \
                 union all \
                 select 'table' as scope, a.privilege_type, n.nspname, c.relname \
                   from pg_class c join pg_namespace n on n.oid = c.relnamespace \
                   cross join lateral aclexplode(coalesce(c.relacl, acldefault('r', c.relowner))) a \
                   join pg_roles r on r.oid = a.grantee \
                  where r.rolname = {who} and c.relkind in ('r','v','m','p','f') and n.nspname not in ('information_schema','pg_catalog') \
                 union all \
                 select 'database' as scope, a.privilege_type, d.datname, '' \
                   from pg_database d cross join lateral aclexplode(coalesce(d.datacl, acldefault('d', d.datdba))) a \
                   join pg_roles r on r.oid = a.grantee \
                  where r.rolname = {who} and not d.datistemplate",
                who = self.literal(name)
            )),
            // Oracle 有两本角色字典：系统权限与对象权限（角色套角色不在权限矩阵里）
            "oracle" => Meta::Sql(format!(
                "select 'global' as scope, '' as owner, '' as object, p.privilege as privilege, p.admin_option as grant_option \
                   from role_sys_privs p where p.role = upper({lit}) \
                 union all \
                 select 'object' as scope, t.owner as owner, t.table_name as object, t.privilege as privilege, t.grantable as grant_option \
                   from role_tab_privs t where t.role = upper({lit})",
                lit = self.literal(name)
            )),
            // ClickHouse：system.grants 里 role_name 非空的就是"授给角色"的行
            "clickhouse" => Meta::Sql(format!(
                "select case when database = '' and table = '' then 'global' when table = '' then 'db' else 'table' end as scope, \
                        access_type as privilege, database as db_name, table as table_name, \
                        if(grant_option, 'YES', 'NO') as grant_option \
                   from system.grants where role_name = {lit} and is_partial_revoke = 0",
                lit = self.literal(name)
            )),
            // Doris 的权限名（SELECT_PRIV …）与界面这套列不同名，本版不给回显
            _ => Meta::Unwritten,
        }
    }

    pub fn role_inventory(&self) -> Meta {
        match self.key() {
            "mysql" => Meta::Sql(
                "select role_name, role_host from ( \
                   select from_user as role_name, from_host as role_host from mysql.role_edges \
                   union \
                   select user, host from mysql.user \
                   where account_locked = 'Y' and coalesce(authentication_string, '') = '' \
                 ) t"
                .to_string(),
            ),
            "mariadb" => Meta::Sql(
                "select distinct Role as role_name, '' as role_host from mysql.roles_mapping"
                    .to_string(),
            ),
            // Oracle：`DBA_ROLES` 里有 100+ 个 Oracle 自带角色（DBA / CONNECT / DV_* …），
            // 全灌进用户清单既吵又危险（删掉 DBA 是真事故）。`ORACLE_MAINTAINED='N'`
            // 正好筛出**用户自己建的**角色（12c+ 才有这一列；11g 上这句会失败，
            // 调用方按"拿不到就不补"处理，清单照常）。
            "oracle" => Meta::Sql(
                "select role as role_name, '' as role_host from dba_roles where oracle_maintained = 'N'"
                    .to_string(),
            ),
            "clickhouse" => Meta::Sql(
                "select name as role_name, '' as role_host from system.roles".to_string(),
            ),
            "doris" => Meta::Sql("show roles".to_string()),
            // PG：纯角色 = 不能登录的那些；`pg_*` 是系统预置角色（pg_monitor / pg_read_all_data…），
            // 它们在上面的「角色」页有专门一栏（pgRoles），不该混进"可删除的角色"清单。
            "postgresql" | "kingbase" => Meta::Sql(
                "select rolname as role_name, '' as role_host from pg_roles \
                 where not rolcanlogin and left(rolname, 3) <> 'pg_'"
                    .to_string(),
            ),
            _ => Meta::Unwritten,
        }
    }

/// 用户新建 / 改密 / 删除（**多条语句用 `;\n` 连接**，由调用方拆分逐条执行）。
    ///
    /// 只生成有把握的那几件事。**细粒度权限（库/表/模式/对象级 GRANT）不在这里做**：
    /// 那是一片权限矩阵，拼错就是「授多了」或「授错了」——调用方会在收到这类入参时明确拒绝，
    /// 而不是默默建好用户却什么都没授（界面成功提示会把"什么都没做"说成成功）。
    /// MySQL 的属性子句串：认证（identified …）/ 配额（with max_*）/ 密码过期 / 账户锁定。
    ///
    /// `CREATE USER` 与 `ALTER USER` **共用这一份** —— 两处的子句顺序完全一样
    /// （auth_option → WITH 资源项 → PASSWORD EXPIRE → ACCOUNT LOCK），顺序错了直接报语法错；
    /// 分开写迟早会漂成"新建能带上、改的时候带不上"。
    ///
    /// 返回 `None` = 这个组合不能用：插件不在白名单，或**改**插件却没给口令
    /// （`identified with x` 不带 BY 会把口令置空，用户当场登不进去）。
    fn mysql_attr_clause(&self, password: &str, attrs: &UserAttrs, pwd: &str, is_create: bool) -> Option<String> {
        let mut clause = String::new();
        if let Some(plugin) = attrs.auth_plugin.as_deref() {
            if !is_mysql_auth_plugin(plugin) {
                return None;
            }
            if !is_create && password.is_empty() {
                return None;
            }
            clause.push_str(&format!(" identified with {plugin}"));
            if !password.is_empty() {
                clause.push_str(&format!(" by {pwd}"));
            }
        } else if !password.is_empty() {
            clause.push_str(&format!(" identified by {pwd}"));
        }
        // 配额（0 = 不限制）
        let mut quota: Vec<String> = Vec::new();
        if let Some(v) = attrs.max_queries_per_hour {
            quota.push(format!("max_queries_per_hour {v}"));
        }
        if let Some(v) = attrs.max_updates_per_hour {
            quota.push(format!("max_updates_per_hour {v}"));
        }
        if let Some(v) = attrs.max_connections_per_hour {
            quota.push(format!("max_connections_per_hour {v}"));
        }
        if let Some(v) = attrs.max_user_connections {
            quota.push(format!("max_user_connections {v}"));
        }
        if !quota.is_empty() {
            clause.push_str(&format!(" with {}", quota.join(" ")));
        }
        if let Some(v) = attrs.password_expired {
            clause.push_str(if v { " password expire" } else { " password expire never" });
        }
        if let Some(v) = attrs.account_locked {
            clause.push_str(if v { " account lock" } else { " account unlock" });
        }
        Some(clause)
    }

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
        let u = self.quote(user);
        let pwd = self.literal(password);
        let host = if host.trim().is_empty() { "%" } else { host };
        let h = self.literal(host);
        let role_parts: Vec<String> = roles.iter().map(|role| self.quote(role)).collect();
        // **角色**的保存（`alter-role`）：只出差量授权，不出 `alter user`。
        // 角色没有口令 / 锁定 / 配额这些属性，前端在角色模式也不会交它们，
        // 所以把 action 归一成 alter 即可 —— 属性为空时各分支不会拼 alter 语句。
        let as_role = action == "alter-role";
        let action = if as_role { "alter" } else { action };
        match self.key() {
            "sqlserver" => Meta::Sql(match action {
                // 登录名是**服务器级**主体：`create login` 必须带口令（SQL 登录）。
                // 不带口令建不出来 —— Windows 登录（`from windows`）界面不提供，
                // 与其悄悄建成一个空口令登录，不如明确拒绝（调用方会报"不支持"）。
                "create" => {
                    if password.is_empty() {
                        return Meta::Unwritten;
                    }
                    let mut sql = format!("create login {u} with password = {pwd}");
                    if let Some(db) = attrs
                        .default_database
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        sql.push_str(&format!(", default_database = {}", self.quote(db)));
                    }
                    if attrs.disabled == Some(true) {
                        sql.push_str(&format!(";\nalter login {u} disable"));
                    }
                    // 「全局」在 SQL Server 就是服务器角色（与 alter 分支同一口径）
                    for role in &privileges.global {
                        sql.push_str(&format!(
                            ";\nalter server role {} add member {u}",
                            self.quote(role)
                        ));
                    }
                    // 数据库角色仍按原样追加（对已在该库映射过用户的登录名有效）
                    for role in &role_parts {
                        sql.push_str(&format!(";\nalter role {role} add member {u}"));
                    }
                    sql
                }
                "alter" => {
                    let mut parts: Vec<String> = Vec::new();
                    if !password.is_empty() {
                        parts.push(format!("alter login {u} with password = {pwd}"));
                    }
                    // 禁用 / 启用是登录名属性（库用户没有这个开关）
                    if let Some(v) = attrs.disabled {
                        parts.push(format!(
                            "alter login {u} {}",
                            if v { "disable" } else { "enable" }
                        ));
                    }
                    // 默认数据库同样是登录名属性（不是库里的 default_schema）
                    if let Some(db) = attrs
                        .default_database
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        parts.push(format!(
                            "alter login {u} with default_database = {}",
                            self.quote(db)
                        ));
                    }
                    // 授权（SQL Server 的四层，与界面上的四张表一一对应）：
                    //   privileges.global   = **服务器角色**（服务器级就是它的"全局"）
                    //   roles               = 数据库角色
                    //   privileges.databases= **Schema 级**（(架构, 权限清单)）
                    //   privileges.tables   = **对象级**（(架构, 对象, 权限清单)）
                    for role in &role_parts {
                        parts.push(format!("alter role {role} add member {u}"));
                    }
                    for role in &privileges.global {
                        parts.push(format!("alter server role {} add member {u}", self.quote(role)));
                    }
                    // Schema 级 / 对象级：**行级选库** —— 界面每行自带「数据库」，
                    // 授权在哪个库执行就先 `use [库];` 切过去（连默认目录常是 master，
                    // 不切库会把授权打到错的地方）。库为空 = 旧行为（当前目录执行）。
                    let use_prefix = |db: &str| -> String {
                        if db.is_empty() { String::new() } else { format!("use {}; ", self.quote(db)) }
                    };
                    for (db, schema, privs) in &privileges.schema_grants {
                        if privs.is_empty() {
                            continue;
                        }
                        parts.push(format!(
                            "{}grant {} on schema::{} to {u}",
                            use_prefix(db),
                            privs.join(", "),
                            self.quote(schema)
                        ));
                    }
                    for (db, schema, object, privs) in &privileges.object_grants {
                        if privs.is_empty() {
                            continue;
                        }
                        parts.push(format!(
                            "{}grant {} on {}.{} to {u}",
                            use_prefix(db),
                            privs.join(", "),
                            self.quote(schema),
                            self.quote(object)
                        ));
                    }
                    // 撤销：界面按"原来有、现在没有了"算好传进来（逐条点名，不全删重授）
                    for r in revokes {
                        let pre = use_prefix(&r.db);
                        let sql = match r.scope.as_str() {
                            "serverrole" => format!("alter server role {} drop member {u}", self.quote(&r.privilege)),
                            "role" => format!("alter role {} drop member {u}", self.quote(&r.privilege)),
                            "schema" => format!("{}revoke {} on schema::{} from {u}", pre, r.privilege, self.quote(&r.owner)),
                            "object" => format!("{}revoke {} on {}.{} from {u}", pre, r.privilege, self.quote(&r.owner), self.quote(&r.object)),
                            _ => return Meta::Unwritten,
                        };
                        parts.push(sql);
                    }
                    if parts.is_empty() {
                        return Meta::Unwritten;
                    }
                    parts.join(";\n")
                }
                // 登录名口径：删的是服务器登录名（库用户删不掉这个账号）
                "drop" => format!("drop login {u}"),
                _ => return Meta::Unwritten,
            }),
            // H2：`create user` 必须带口令；改密是 `alter user … set password`。
            // 权限矩阵（表级 GRANT）还没接进来 —— 调用方收到权限入参会先明确拒绝。
            "h2" => {
                let name = h2_user(user);
                // 空用户名（H2 的默认用户）必须写成空标识符 `""`：`quote("")` 返回空串，
                // 拼出来是 `grant … to `（实测 42001 SyntaxError，缺 identifier），
                // 连改密/删除也一样会拼坏。
                let u = if name.is_empty() { "\"\"".to_string() } else { self.quote(&name) };
                let pwd = self.literal(password);
                let mut parts: Vec<String> = Vec::new();
                match action {
                    "create" => {
                        if password.is_empty() {
                            return Meta::Unwritten;
                        }
                        parts.push(format!("create user {u} password {pwd}"));
                    }
                    "alter" => {
                        // 只改口令；其余属性（is_admin 等）不走这里
                        if !password.is_empty() {
                            parts.push(format!("alter user {u} set password {pwd}"));
                        }
                    }
                    "drop" => return Meta::Sql(format!("drop user {u}")),
                    _ => return Meta::Unwritten,
                }
                // 表级授权（H2 唯一的一层权限）：`grant select, update on "S"."T" to "u"`。
                // 目标用 schema + 表名（前端的 database 字段在 H2 里就是 schema）。
                for (schema, table, privs) in &privileges.tables {
                    let (schema, table) = (schema.trim(), table.trim());
                    if schema.is_empty() || table.is_empty() {
                        continue;
                    }
                    let items: Vec<String> = privs
                        .iter()
                        .map(|p| p.trim().to_ascii_uppercase())
                        .filter(|p| is_h2_table_priv(p))
                        .collect();
                    if items.is_empty() {
                        continue;
                    }
                    parts.push(format!(
                        "grant {} on {}.{} to {u}",
                        items.join(", "),
                        self.quote(schema),
                        self.quote(table)
                    ));
                }
                // 撤销：界面按"原来有、现在没有了"点名（scope=table）
                for row in revokes {
                    let priv_name = row.privilege.trim().to_ascii_uppercase();
                    if !is_h2_table_priv(&priv_name) {
                        continue;
                    }
                    let (schema, table) = (row.owner.trim(), row.object.trim());
                    if schema.is_empty() || table.is_empty() {
                        continue;
                    }
                    parts.push(format!(
                        "revoke {priv_name} on {}.{} from {u}",
                        self.quote(schema),
                        self.quote(table)
                    ));
                }
                if parts.is_empty() {
                    return Meta::Unwritten;
                }
                Meta::Sql(parts.join(";\n"))
            }
            // MySQL 的用户是 (user, host) 二元组 —— 少了 host 就会建错主体
            "mysql" | "mariadb" | "doris" => {
                let full = format!("{u}@{h}");
                // 授权语句（grant / revoke）的目标账号一律用**单引号字符串**写法
                // `'user'@'host'`：`quote()` 对普通名字不加任何引号（`dbmind_probe_u@'%'`），
                // 名字里带 `-` 之类的还会变成反引号 —— 那在账号位置是错的。
                let full_lit = format!("{}@{}", self.literal(user), self.literal(host));
                // 授权目标：编辑**角色**的权限时，主体是角色本身 —— MySQL 的角色与账号同形
                // （`'r'@'%'`），MariaDB 的角色没有 host（`'r'`），Doris 必须写
                // `TO ROLE 'r'`（它这版也不提供角色权限编辑，见 role_info 注释）。
                let grantee = if as_role {
                    match self.key() {
                        "mysql" => full_lit.clone(),
                        "doris" => format!("role {}", self.literal(user)),
                        _ => self.literal(user),
                    }
                } else {
                    full_lit.clone()
                };
                let mut parts: Vec<String> = Vec::new();
                match action {
                    "create" => {
                        match self.mysql_attr_clause(password, attrs, &pwd, true) {
                            Some(clause) => parts.push(format!("create user {full}{clause}")),
                            None => return Meta::Unwritten,
                        }
                    }
                    "alter" => {
                        // 与 create 同一份子句（见 mysql_attr_clause），只是多一条：
                        // 「改插件必须连口令一起给」在那里判掉了。
                        match self.mysql_attr_clause(password, attrs, &pwd, false) {
                            Some(clause) => {
                                if !clause.is_empty() {
                                    parts.push(format!("alter user {full}{clause}"));
                                }
                            }
                            None => return Meta::Unwritten,
                        }
                    }
                    "drop" => return Meta::Sql(format!("drop user {full}")),
                    // 新建 / 删除**角色**（MySQL 8 / MariaDB 10.0.5+ 的 RBAC）。
                    // MySQL 的角色是账号形状（`'r'@'h'`）；MariaDB 的角色**没有 host**
                    // （`create role 'r'@'%'` 是 1064 语法错，真机验证过）——
                    // 所以按方言分开拼，不能共用 `full_lit`。
                    // Doris 的角色也没有 host，且名字是字符串（`create role 'r'`）。
                    "create-role" => {
                        // Doris 的角色名是**标识符**（`create role 'r'` 直接 1064 语法错，
                        // 实测它连 STRING 都不收），所以 Doris 走 quote()，MariaDB 走字符串。
                        let who = match self.key() {
                            "mysql" => full_lit.clone(),
                            "doris" => self.quote(user),
                            _ => self.literal(user),
                        };
                        return Meta::Sql(if password.is_empty() {
                            format!("create role {who}")
                        } else {
                            // 带口令建角色只有 MySQL/MariaDB 支持（角色可登录，一般不需要）
                            format!("create role {who} identified by {pwd}")
                        });
                    }
                    "drop-role" => {
                        let who = match self.key() {
                            "mysql" => full_lit.clone(),
                            "doris" => self.quote(user),
                            _ => self.literal(user),
                        };
                        return Meta::Sql(format!("drop role {who}"));
                    }
                    _ => return Meta::Unwritten,
                }
                // 授权**只出差量**：界面把"原来有、现在没有了"的算好传进来，逐条点名撤销；
                // 勾选的再由下面的循环逐条授予。
                //
                // 从前用的是 `revoke all privileges, grant option from <用户>` + 全量重授：
                // 那样确实省事（连库级/表级一并清干净），但会把用户**别处**拿到的授权也抹掉再重建。
                // 逐目标 revoke 以前不敢用，是因为它会在"该目标本来就没授权"时报 1141
                // （`There is no such grant defined`）—— 而现在点名撤的都是**真差量**
                // （只撤"原来确实有、现在取消了"的），撤不到没有的东西上，这个坑自然绕开了。
                // `reset_privileges` 只在客户端没交差量清单（老调用方）时才为真。
                if reset_privileges {
                    parts.push(format!("revoke all privileges, grant option from {grantee}"));
                } else {
                    parts.extend(self.mysql_revoke_parts(&grantee, revokes));
                }
                // 角色（MySQL 8 / MariaDB 的 RBAC）：MySQL 是 `grant 'r'@'h' to 'u'@'%'`
                // （角色本身也是账号形状，名字与 host 分别转义再加 @ 拼起来）；
                // MariaDB / Doris 的角色**没有 host** —— 多写 `@'%'` 在 MariaDB 上是
                // 1064 语法错（真机验证过），只能 `grant 'r' to 'u'@'%'`。
                for r in roles {
                    let (rn, rh) = Self::split_role_ref(r);
                    if rn.is_empty() {
                        continue;
                    }
                    parts.push(if self.key() == "mysql" {
                        format!(
                            "grant {}@{} to {full_lit}",
                            self.literal(&rn),
                            self.literal(&rh)
                        )
                    } else {
                        format!("grant {} to {full_lit}", self.literal(&rn))
                    });
                }
                // `GRANT OPTION` 不是普通权限项，而是 `WITH GRANT OPTION` 子句 ——
                // 拼成 `grant GRANT OPTION on ...` 是错的（MySQL 的 priv_type 里没有它）。
                // 它也是**能被撤销**的：保存时的 `revoke all privileges, grant option from`
                // 会连授权能力一起收回，所以界面必须能看见、能回授。
                let split_grant_option = |privs: &[String]| -> (Vec<String>, bool) {
                    let with_option = privs.iter().any(|name| name == "GRANT OPTION");
                    let items: Vec<String> = privs
                        .iter()
                        .filter(|name| name.as_str() != "GRANT OPTION")
                        .cloned()
                        .collect();
                    (items, with_option)
                };
                let tail = |with_option: bool| if with_option { " with grant option" } else { "" };
                for (database, privs) in &privileges.databases {
                    let (items, with_option) = split_grant_option(privs);
                    if items.is_empty() {
                        continue;
                    }
                    parts.push(format!(
                        "grant {} on {}.* to {grantee}{}",
                        items.join(", "),
                        self.quote(database),
                        tail(with_option)
                    ));
                }
                for (database, table, privs) in &privileges.tables {
                    let (items, with_option) = split_grant_option(privs);
                    if items.is_empty() {
                        continue;
                    }
                    parts.push(format!(
                        "grant {} on {}.{} to {grantee}{}",
                        items.join(", "),
                        self.quote(database),
                        self.quote(table),
                        tail(with_option)
                    ));
                }
                // 例程级：对**单个**存储过程/函数授 EXECUTE —— MySQL 语法里
                // `on procedure `db`.`p`` 的类型字是语法的一部分，不能省。
                for (database, routine, kind, privs) in &privileges.routines {
                    let (items, with_option) = split_grant_option(privs);
                    if items.is_empty() {
                        continue;
                    }
                    let routine_type = if kind.eq_ignore_ascii_case("function") { "function" } else { "procedure" };
                    parts.push(format!(
                        "grant {} on {} {}.{} to {grantee}{}",
                        items.join(", "),
                        routine_type,
                        self.quote(database),
                        self.quote(routine),
                        tail(with_option)
                    ));
                }
                let (global_items, global_option) = split_grant_option(&privileges.global);
                if !global_items.is_empty() {
                    parts.push(format!(
                        "grant {} on *.* to {grantee}{}",
                        global_items.join(", "),
                        tail(global_option)
                    ));
                }
                Meta::Sql(parts.join(";\n"))
            }
            // PG 的「用户」就是带 login 的角色（MySQL 语境的 host 在这里没有意义，忽略）。
            // 细粒度授权（模式 / 表 / 例程）与撤销：create 与 alter 共用一套拼法 ——
            // 模式 `grant usage on schema "s" to "u"`；表 `on table "s"."t"`；
            // 例程 `on function|procedure "s"."r"`（PG 11+ 签名可省略，同名歧义由数据库报错）。
            // 权限名已在 meta.rs 过白名单（is_pg_schema_priv / is_pg_table_priv / EXECUTE）。
            "postgresql" | "kingbase" => {
                let mut pg_parts: Vec<String> = Vec::new();
                if action == "create" || action == "alter" {
                    // 库级：`grant connect on database "db" to "u"`（PG 真正的那一层，
                    // 与下面被借用来装模式的 `databases` 不是一回事）
                    for (db, items) in &privileges.pg_databases {
                        if db.trim().is_empty() || items.is_empty() {
                            continue;
                        }
                        pg_parts.push(format!(
                            "grant {} on database {} to {u}",
                            items.join(", ").to_lowercase(),
                            self.quote(db)
                        ));
                    }
                    for (schema, items) in &privileges.databases {
                        if schema.trim().is_empty() || items.is_empty() {
                            continue;
                        }
                        pg_parts.push(format!(
                            "grant {} on schema {} to {u}",
                            items.join(", ").to_lowercase(),
                            self.quote(schema)
                        ));
                    }
                    for (schema, table, items) in &privileges.tables {
                        if schema.trim().is_empty() || table.trim().is_empty() || items.is_empty() {
                            continue;
                        }
                        pg_parts.push(format!(
                            "grant {} on table {}.{} to {u}",
                            items.join(", ").to_lowercase(),
                            self.quote(schema),
                            self.quote(table)
                        ));
                    }
                    for (schema, routine, kind, items) in &privileges.routines {
                        if schema.trim().is_empty() || routine.trim().is_empty() || items.is_empty() {
                            continue;
                        }
                        let object_type = if kind.eq_ignore_ascii_case("procedure") { "procedure" } else { "function" };
                        pg_parts.push(format!(
                            "grant {} on {object_type} {}.{} to {u}",
                            items.join(", ").to_lowercase(),
                            self.quote(schema),
                            self.quote(routine)
                        ));
                    }
                    for r in revokes {
                        match r.scope.as_str() {
                            "schema" if !r.owner.trim().is_empty() => {
                                pg_parts.push(format!(
                                    "revoke {} on schema {} from {u}",
                                    r.privilege.to_lowercase(),
                                    self.quote(&r.owner)
                                ));
                            }
                            // 库级撤销：`revoke connect on database "db" from "u"`
                            "database" if !r.owner.trim().is_empty() => {
                                pg_parts.push(format!(
                                    "revoke {} on database {} from {u}",
                                    r.privilege.to_lowercase(),
                                    self.quote(&r.owner)
                                ));
                            }
                            "table" if !r.owner.trim().is_empty() && !r.object.trim().is_empty() => {
                                pg_parts.push(format!(
                                    "revoke {} on table {}.{} from {u}",
                                    r.privilege.to_lowercase(),
                                    self.quote(&r.owner),
                                    self.quote(&r.object)
                                ));
                            }
                            // 角色成员关系（「服务器权限」页里那些角色项）：`revoke "r" from "u"`。
                            // 少了这条，取消勾选一个角色会**静默无效** —— 界面显示已取消、
                            // 服务器上还在（角色那一层就永远撤不掉）。
                            "role" if !r.privilege.trim().is_empty() => {
                                pg_parts.push(format!(
                                    "revoke {} from {u}",
                                    self.quote(&r.privilege)
                                ));
                            }
                            "routine" | "function"
                                if !r.owner.trim().is_empty() && !r.object.trim().is_empty() =>
                            {
                                let object_type = if r.kind.eq_ignore_ascii_case("procedure") { "procedure" } else { "function" };
                                pg_parts.push(format!(
                                    "revoke {} on {object_type} {}.{} from {u}",
                                    r.privilege.to_lowercase(),
                                    self.quote(&r.owner),
                                    self.quote(&r.object)
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                Meta::Sql(match action {
                "create" => {
                    // 属性：三个开关（与 alter 同一套字面量）；没给就沿用"可登录"
                    let mut flags: Vec<&str> = Vec::new();
                    flags.push(if attrs.can_login.unwrap_or(true) { "login" } else { "nologin" });
                    if let Some(v) = attrs.is_superuser {
                        flags.push(if v { "superuser" } else { "nosuperuser" });
                    }
                    if let Some(v) = attrs.can_create_db {
                        flags.push(if v { "createdb" } else { "nocreatedb" });
                    }
                    let mut sql = format!("create role {u} {}", flags.join(" "));
                    if !password.is_empty() {
                        sql.push_str(&format!(" password {pwd}"));
                    }
                    for role in &role_parts {
                        sql.push_str(&format!(";\ngrant {role} to {u}"));
                    }
                    for part in &pg_parts {
                        sql.push_str(";\n");
                        sql.push_str(part);
                    }
                    sql
                }
                "alter" => {
                    let mut parts: Vec<String> = Vec::new();
                    if !password.is_empty() {
                        parts.push(format!("alter role {u} password {pwd}"));
                    }
                    // 属性：三个开关合成一条 alter role（PG 的 role 属性就是这些字面开关）
                    let mut flags: Vec<&str> = Vec::new();
                    if let Some(v) = attrs.can_login {
                        flags.push(if v { "login" } else { "nologin" });
                    }
                    if let Some(v) = attrs.is_superuser {
                        flags.push(if v { "superuser" } else { "nosuperuser" });
                    }
                    if let Some(v) = attrs.can_create_db {
                        flags.push(if v { "createdb" } else { "nocreatedb" });
                    }
                    if !flags.is_empty() {
                        parts.push(format!("alter role {u} {}", flags.join(" ")));
                    }
                    for role in &role_parts {
                        parts.push(format!("grant {role} to {u}"));
                    }
                    parts.extend(pg_parts.iter().cloned());
                    if parts.is_empty() {
                        return Meta::Unwritten;
                    }
                    parts.join(";\n")
                }
                // 新建**角色**（不可登录的主体）。PG 里角色与用户是同一张表，
                // `create role` 默认 nologin —— 与"新建用户再取消可登录"等价，但一步到位。
                "create-role" => format!("create role {u}"),
                "drop" => format!("drop role {u}"),
                "drop-role" => format!("drop role {u}"),
                _ => return Meta::Unwritten,
            })
            }
            // ClickHouse：用户没有 (user, host) 二元组，host 参数没有意义（忽略）。
            // 授权与 MySQL 系同构：全局 `ON *.*` / 库 `ON db.*` / 表 `ON db.table`，
            // 权限名过 CLICKHOUSE_PRIVILEGES 白名单（原样拼 SQL，名单外直接跳过）。
            // 撤销按调用方算好的**差量**逐条点名 —— 与 MySQL「不全删重授」同一取舍。
            "clickhouse" => {
                let ch_target = |scope: &str, owner: &str, object: &str| -> Option<String> {
                    match scope {
                        "global" => Some("*.*".to_string()),
                        "db" if !owner.trim().is_empty() => Some(format!("{}.*", self.quote(owner))),
                        "table" if !owner.trim().is_empty() && !object.trim().is_empty() => {
                            Some(format!("{}.{}", self.quote(owner), self.quote(object)))
                        }
                        _ => None,
                    }
                };
                let mut parts: Vec<String> = Vec::new();
                match action {
                    "create" => {
                        let mut sql = format!("create user {u}");
                        if !password.is_empty() {
                            sql.push_str(&format!(" identified by {pwd}"));
                        }
                        parts.push(sql);
                    }
                    "alter" => {
                        if !password.is_empty() {
                            parts.push(format!("alter user {u} identified by {pwd}"));
                        }
                    }
                    "drop" => return Meta::Sql(format!("drop user {u}")),
                    // CH 的角色是独立主体（`system.roles`）：`create role r` / `drop role r`。
                    // 名字走 quote()（必要时反引号），与其它 CH 语句同一套转义。
                    "create-role" => return Meta::Sql(format!("create role {u}")),
                    "drop-role" => return Meta::Sql(format!("drop role {u}")),
                    _ => return Meta::Unwritten,
                }
                for role in &role_parts {
                    parts.push(format!("grant {role} to {u}"));
                }
                for r in revokes {
                    // 角色是独立一层（`revoke role from user`，没有 `on …`）：
                    // 别把它丢进下面按 scope 拼 target 的逻辑里（那里会返回 None 直接跳过）
                    if r.scope.eq_ignore_ascii_case("role") {
                        let role = r.privilege.trim();
                        if !role.is_empty() && is_safe_name(role) {
                            parts.push(format!("revoke {} from {u}", self.quote(role)));
                        }
                        continue;
                    }
                    if !is_clickhouse_privilege(&r.privilege) {
                        continue;
                    }
                    if let Some(target) = ch_target(&r.scope, &r.owner, &r.object) {
                        parts.push(format!("revoke {} on {} from {u}", r.privilege.trim().to_ascii_uppercase(), target));
                    }
                }
                // CH 这层不提供 WITH GRANT OPTION（界面没给这个开关），不需要拆 grant option
                for p in &privileges.global {
                    if !is_clickhouse_privilege(p) {
                        continue;
                    }
                    parts.push(format!("grant {} on *.* to {u}", p.trim().to_ascii_uppercase()));
                }
                for (database, privs) in &privileges.databases {
                    if database.trim().is_empty() {
                        continue;
                    }
                    let names: Vec<String> = privs
                        .iter()
                        .filter(|p| is_clickhouse_privilege(p))
                        .map(|p| p.trim().to_ascii_uppercase())
                        .collect();
                    if names.is_empty() {
                        continue;
                    }
                    parts.push(format!("grant {} on {}.* to {u}", names.join(", "), self.quote(database)));
                }
                for (database, table, privs) in &privileges.tables {
                    if database.trim().is_empty() || table.trim().is_empty() {
                        continue;
                    }
                    let names: Vec<String> = privs
                        .iter()
                        .filter(|p| is_clickhouse_privilege(p))
                        .map(|p| p.trim().to_ascii_uppercase())
                        .collect();
                    if names.is_empty() {
                        continue;
                    }
                    parts.push(format!(
                        "grant {} on {}.{} to {u}",
                        names.join(", "),
                        self.quote(database),
                        self.quote(table)
                    ));
                }
                if parts.is_empty() {
                    return Meta::Unwritten;
                }
                Meta::Sql(parts.join(";\n"))
            }
            // Oracle：`create user / alter user ... identified by / drop user`。
            // 与 PG、ClickHouse 一样忽略 host（Oracle 没有 (user, host) 二元组）。
            //
            // 两条刻意的取舍：
            // · 建完顺手 `grant create session` —— 不然新建的用户**连不上去**，等于没建好；
            //   这不是"多授了权限"，是建一个有登录能力的用户的最小集合。
            // · 删除用**不带 cascade** 的 `drop user` —— 带 cascade 会把该用户 schema 下的
            //   所有对象一起删掉，那是不可逆的破坏；他自己的对象就该由 ORA-01922 拦下来、
            //   让管理员显式去执行 `drop user ... cascade`，而不是我们替他决定。
            "oracle" => Meta::Sql(match action {
                "create" => {
                    let mut parts: Vec<String> = Vec::new();
                    if password.is_empty() {
                        // 没有口令就建不了 Oracle 用户（它必须带 identified by）
                        return Meta::Unwritten;
                    }
                    // ⚠️ 口令要用**双引号标识符**，不能用 `pwd`（单引号字符串字面量）：
                    // Oracle 的 `identified by` 收的是标识符，给字面量会报
                    // `ORA-00988: 密码缺失或无效`（真机上就是这么报的）。
                    // 引号里的内容原样保留 —— 大小写与 @ 这类特殊字符都不会被改写。
                    let pwd_id = self.quote(password);
                    // 属性：表空间 / 概要文件 / 账户锁定（与 alter 同一套写法）
                    let mut create = format!("create user {u} identified by {pwd_id}");
                    if let Some(v) = attrs.default_tablespace.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                        create.push_str(&format!(" default tablespace {}", self.quote(v)));
                    }
                    if let Some(v) = attrs.temporary_tablespace.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                        create.push_str(&format!(" temporary tablespace {}", self.quote(v)));
                    }
                    if let Some(v) = attrs.profile.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                        create.push_str(&format!(" profile {}", self.quote(v)));
                    }
                    if let Some(v) = attrs.account_locked {
                        create.push_str(if v { " account lock" } else { " account unlock" });
                    }
                    parts.push(create);
                    // 建号时就把"能登录 + 界面勾的角色/系统权限/对象权限"一起授了 ——
                    // 建完还得再点一次保存才生效，是让人白跑一趟（用户反馈过）。
                    parts.push(format!("grant create session to {u}"));
                    parts.extend(Self::oracle_grant_parts(&u, roles, privileges));
                    parts.join(";\n")
                }
                "alter" => {
                    // 先撤后授：撤销由界面按"原来有、现在没有了"算好传进来
                    // （Oracle 没有 `revoke all privileges from u` 这种一把全撤）。
                    let mut parts: Vec<String> = Vec::new();
                    if !password.is_empty() {
                        let pwd_id = self.quote(password);
                        parts.push(format!("alter user {u} identified by {pwd_id}"));
                    }
                    // 属性（一次一项最稳）：账户锁定 / 默认表空间 / 临时表空间 / 概要文件。
                    // 表空间与概要文件收的是**标识符**（不是字符串），用 quote 而不是 literal；
                    // 空串当"没填"忽略 —— 否则会拼出 `default tablespace ""` 这种错语句。
                    if let Some(v) = attrs.account_locked {
                        parts.push(format!(
                            "alter user {u} {}",
                            if v { "account lock" } else { "account unlock" }
                        ));
                    }
                    if let Some(v) = attrs
                        .default_tablespace
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        parts.push(format!("alter user {u} default tablespace {}", self.quote(v)));
                    }
                    if let Some(v) = attrs
                        .temporary_tablespace
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        parts.push(format!("alter user {u} temporary tablespace {}", self.quote(v)));
                    }
                    if let Some(v) = attrs.profile.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                        parts.push(format!("alter user {u} profile {}", self.quote(v)));
                    }
                    parts.extend(Self::oracle_revoke_parts(&u, revokes));
                    parts.extend(Self::oracle_grant_parts(&u, roles, privileges));
                    if parts.is_empty() {
                        return Meta::Unwritten;
                    }
                    parts.join(";\n")
                }
                "drop" => format!("drop user {u}"),
                // Oracle 的角色是独立主体（`dba_roles`）：`create role "R"` / `drop role "R"`。
                // `u` 走 quote()，与上面 create/alter/drop user 同一套标识符转义。
                "create-role" => format!("create role {u}"),
                "drop-role" => format!("drop role {u}"),
                _ => return Meta::Unwritten,
            }),
            // DM（达梦）：语法与 Oracle 相近，但没真机验证过，先保持 Unwritten ——
            // 与别处"没验证就不动"同一个原则。
            _ => Meta::Unwritten,
        }
    }

    /// 表/视图的建表语句。
    ///
    /// `schema` = 该对象所在的模式（请求里的「库.模式」拆出来的那半；调用方用
    /// [`crate::api::meta::split_db_schema`]）。**不能只靠会话默认模式**：DBMind 的查询
    /// 会话刻意不按"树上选中的模式"切换（不赌代理有没有真的切 schema），于是
    /// 在 SYS 下点一张表、会话默认模式却是 SYSTEM 时，Oracle 会报 ORA-31603
    /// 「对象在方案 SYSTEM 中未找到」—— 这就是真机上报的那个错。
    pub fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        let t = self.quote(table);
        let lit = self.literal(table);
        // 模式名做**字符串字面量**（有才用）；PG 侧还要它替掉 current_schema()
        let schema_lit = schema
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| self.literal(s));
        let ns = match &schema_lit {
            Some(s) => s.clone(),
            None => "current_schema()".to_string(),
        };
        match self.key() {
            "sqlite" => Meta::Sql(format!(
                "select sql from sqlite_master where type in ('table','view') and name = {}",
                self.literal(table)
            )),
            "mysql" | "mariadb" | "doris" => Meta::Sql(format!("show create table {t}")),
            "clickhouse" => Meta::Sql(format!("show create table {t}")),
            // H2 2.x **没有** `information_schema.tables.sql`（那是 1.x 的字段），
            // 老写法在 2.3 上直接报 `Column "SQL" not found`（实测）。
            // `SCRIPT NODATA` 是 H2 官方的导出入口：它是一条**命令**而非常量表
            // （`select script from script nodata` 报 Table "SCRIPT" not found），
            // 而且 `table <名>` 这个过滤实测被忽略（照样返回整库语句）。
            // 所以这里只发命令，**按表名筛选放在 Rust 侧**（见 `build_table_ddl`）。
            "h2" => Meta::Sql("script nodata".to_string()),
            // SQL Server：没有内置函数，自己拼。两条关键约定：
            // - **视图走 `object_definition`**（它就是原始 create view 文本），表才需要拼列
            // - 用 `for xml path('')` 而不是 `string_agg`：后者要 SQL Server 2017+，
            //   而这里拼的是要贴给用户直接执行的 DDL，不该带一个「你的版本太老」的隐性前提
            // - 分隔符用 `case ... < max(column_id)`：**不能**每列后都加逗号，
            //   否则没有主键的表会以 `,)` 结尾，那在 SQL Server 上是语法错误
            "sqlserver" => Meta::Sql(format!(
                "select object_definition(object_id({lit})) as ddl from sys.views where name = {lit} \
                 union all \
                 select \
                 'create table ' + quotename(schema_name(t.schema_id)) + '.' + quotename(t.name) + ' (' + char(10) + \
                 stuff(( \
                   select char(9) + quotename(c.name) + \
                     case when c.is_computed = 1 \
                          then ' as ' + cc.definition \
                          else ' ' + ty.name + \
                     case \
                       when ty.name in ('varchar','char','varbinary','binary') \
                         then '(' + case when c.max_length = -1 then 'max' else cast(c.max_length as varchar(10)) end + ')' \
                       when ty.name in ('nvarchar','nchar') \
                         then '(' + case when c.max_length = -1 then 'max' else cast(c.max_length / 2 as varchar(10)) end + ')' \
                       when ty.name in ('decimal','numeric') \
                         then '(' + cast(c.precision as varchar(10)) + ',' + cast(c.scale as varchar(10)) + ')' \
                       when ty.name in ('datetime2','datetimeoffset','time') and c.scale <> 7 \
                         then '(' + cast(c.scale as varchar(10)) + ')' \
                       else '' \
                     end + \
                     case when c.is_identity = 1 \
                          then ' identity(' + cast(coalesce(ic.seed_value, 1) as varchar(20)) + ',' \
                               + cast(coalesce(ic.increment_value, 1) as varchar(20)) + ')' else '' end + \
                     case when c.is_nullable = 0 then ' not null' else ' null' end + \
                     case when dc.definition is not null then ' default ' + dc.definition else '' end \
                     end + \
                     case when c.column_id < (select max(column_id) from sys.columns where object_id = t.object_id) \
                          then ',' else '' end + char(10) \
                     from sys.columns c \
                   join sys.types ty on ty.user_type_id = c.user_type_id \
                   left join sys.identity_columns ic on ic.object_id = c.object_id and ic.column_id = c.column_id \
                   left join sys.default_constraints dc on dc.parent_object_id = c.object_id and dc.parent_column_id = c.column_id \
                   left join sys.computed_columns cc on cc.object_id = c.object_id and cc.column_id = c.column_id \
                   where c.object_id = t.object_id \
                   order by c.column_id \
                   for xml path(''), type).value('.', 'nvarchar(max)'), 1, 0, '') + \
                 case when pk.name is null then '' else \
                   char(9) + 'constraint ' + quotename(pk.name) + ' primary key (' + \
                   stuff(( \
                     select quotename(pc.name) + \
                       case when ic2.key_ordinal < (select max(key_ordinal) from sys.index_columns \
                                                    where object_id = t.object_id and index_id = pk.unique_index_id) \
                            then ', ' else '' end \
                     from sys.index_columns ic2 \
                     join sys.columns pc on pc.object_id = ic2.object_id and pc.column_id = ic2.column_id \
                     where ic2.object_id = t.object_id and ic2.index_id = pk.unique_index_id \
                     order by ic2.key_ordinal \
                     for xml path(''), type).value('.', 'nvarchar(max)'), 1, 0, '') + ')' + char(10) end + \
                 ');' as ddl \
                 from sys.tables t \
                 left join sys.key_constraints pk on pk.parent_object_id = t.object_id and pk.type = 'PK' \
                 where t.name = {lit}"
            )),
            // PostgreSQL：`format_type` 直接给出带长度/精度的完整类型、`pg_get_expr` 还原默认值；
            // 视图用 `pg_get_viewdef`
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select 'create view ' || quote_ident(relname) || ' as' || chr(10) || pg_get_viewdef(oid) as ddl \
                 from pg_class where relname = {lit} and relkind = 'v' \
                   and relnamespace = (select oid from pg_namespace where nspname = current_schema()) \
                 union all \
                 select 'create table ' || quote_ident(c.relname) || ' (' || chr(10) || \
                 string_agg( \
                   '  ' || quote_ident(a.attname) || ' ' || format_type(a.atttypid, a.atttypmod) || \
                   case when a.attnotnull then ' not null' else '' end || \
                   coalesce(' default ' || pg_get_expr(d.adbin, d.adrelid), ''), \
                   ',' || chr(10) order by a.attnum) || chr(10) || ');' as ddl \
                 from pg_class c \
                 join pg_attribute a on a.attrelid = c.oid and a.attnum > 0 and not a.attisdropped \
                 left join pg_attrdef d on d.adrelid = c.oid and d.adnum = a.attnum \
                 where c.relname = {lit} and c.relkind = 'r' \
                   and c.relnamespace = (select oid from pg_namespace where nspname = {ns}) \
                 group by c.relname"
            )),
            // Oracle：`dbms_metadata.get_ddl` 就是官方给完整 DDL 的入口（含存储属性），
            // 比手拼可靠；按存在性自动选 TABLE / VIEW。DM 走手工拼列（它的 dbms_metadata 支持面不确定）
            "oracle" => match &schema_lit {
                // 带 owner 的正版：类型判断也换成 all_views（user_views 只看会话自己模式，
                // 在 SYS 下点一张视图而会话是 SYSTEM 时会判成 TABLE，取到的 DDL 还是别人的）
                Some(owner) => Meta::Sql(format!(
                    "select dbms_metadata.get_ddl( \
                       case when exists (select 1 from all_views where view_name = upper({lit}) \
                                          and owner = upper({owner})) \
                            then 'VIEW' else 'TABLE' end, \
                       upper({lit}), upper({owner})) as ddl from dual"
                )),
                None => Meta::Sql(format!(
                    "select dbms_metadata.get_ddl( \
                       case when exists (select 1 from user_views where view_name = upper({lit})) \
                            then 'VIEW' else 'TABLE' end, \
                       upper({lit})) as ddl from dual"
                )),
            },
            // Oracle/DM 通用兜底：手工拼列（listagg）+ 视图原文（user_views.text 两家都有）
            "dm" => Meta::Sql(format!(
                "select 'create or replace view ' || lower(view_name) || ' as' || chr(10) || text as ddl \
                 from user_views where view_name = upper({lit}) \
                 union all \
                 select 'create table ' || lower(table_name) || ' (' || chr(10) || \
                 listagg('  ' || lower(column_name) || ' ' || \
                   case \
                     when data_type = 'NUMBER' and data_precision is not null \
                       then 'number(' || data_precision || coalesce(',' || data_scale, '') || ')' \
                     when data_type in ('VARCHAR2','NVARCHAR2','CHAR','NCHAR') \
                       then lower(data_type) || '(' || data_length || ')' \
                     else lower(data_type) \
                   end || \
                   case when nullable = 'N' then ' not null' else '' end, ',' || chr(10)) \
                   within group (order by column_id) || chr(10) || ');' as ddl \
                 from user_tab_columns where table_name = upper({lit}) \
                 group by table_name"
            )),
            // DB2 / Derby：没有实例可验证，不猜（界面会如实说这个类型还没写）
            _ => Meta::Unwritten,
        }
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
        if self.key() != "sqlserver" {
            return Vec::new();
        }
        let lit = self.literal(table);
        // 注释：`MS_Description` 是 SQL Server 里「注释」的通用约定（SSMS 就用它）。
        // `minor_id = 0` 表示挂在**表**上，否则是某一列。
        //
        // **只处理表（sys.tables）**：视图那边还有一条「从 DDL 转 ALTER」的编辑链路，
        // 往视图定义后面追加 exec 语句会污染那条链路；而表这条链路只把 DDL 拿去显示/复制，
        // 追加是安全的。宁可少给视图的注释，也不动一条已经验过的链路。
        let comments = format!(
            "select stuff(( \
               select char(10) + 'exec sp_addextendedproperty ''MS_Description'', N''' + \
                 replace(cast(ep.value as nvarchar(max)), '''', '''''') + \
                 ''', ''SCHEMA'', N''' + replace(schema_name(t.schema_id), '''', '''''') + \
                 ''', ''TABLE'', N''' + replace(t.name, '''', '''''') + '''' + \
                 case when ep.minor_id = 0 then '' \
                      else ', ''COLUMN'', N''' + replace(c.name, '''', '''''') + '''' end + ';' \
               from sys.extended_properties ep \
               join sys.tables t on t.object_id = ep.major_id \
               left join sys.columns c on c.object_id = ep.major_id and c.column_id = ep.minor_id \
               where ep.class = 1 and ep.name = 'MS_Description' and t.name = {lit} \
               order by ep.minor_id \
               for xml path(''), type).value('.', 'nvarchar(max)'), 1, 1, '') as ddl"
        );
        // 索引与唯一约束：主键已在基础 DDL 里，这里排除掉，免得重复。
        // `desc` 方向、`include (...)` 列、筛选条件（filtered index）都要带上 ——
        // 少了任何一样，重建出来的索引都和原库的不一样，而这种差别很难被发现。
        let indexes = format!(
            "select stuff(( \
               select char(10) + \
                 case when i.is_unique_constraint = 1 then \
                   'alter table ' + quotename(schema_name(t.schema_id)) + '.' + quotename(t.name) + \
                   ' add constraint ' + quotename(i.name) + ' unique (' + \
                   stuff((select quotename(c.name) + \
                            case when ic.is_descending_key = 1 then ' desc' else '' end + \
                            case when ic.key_ordinal < (select max(key_ordinal) from sys.index_columns \
                                                         where object_id = i.object_id and index_id = i.index_id) \
                                 then ', ' else '' end \
                          from sys.index_columns ic \
                          join sys.columns c on c.object_id = ic.object_id and c.column_id = ic.column_id \
                          where ic.object_id = i.object_id and ic.index_id = i.index_id and ic.is_included_column = 0 \
                          order by ic.key_ordinal for xml path(''), type).value('.', 'nvarchar(max)'), 1, 0, '') + ')' \
                 else \
                   'create ' + case when i.is_unique = 1 then 'unique ' else '' end + 'index ' + quotename(i.name) + \
                   ' on ' + quotename(schema_name(t.schema_id)) + '.' + quotename(t.name) + ' (' + \
                   stuff((select quotename(c.name) + \
                            case when ic.is_descending_key = 1 then ' desc' else '' end + \
                            case when ic.key_ordinal < (select max(key_ordinal) from sys.index_columns \
                                                         where object_id = i.object_id and index_id = i.index_id) \
                                 then ', ' else '' end \
                          from sys.index_columns ic \
                          join sys.columns c on c.object_id = ic.object_id and c.column_id = ic.column_id \
                          where ic.object_id = i.object_id and ic.index_id = i.index_id and ic.is_included_column = 0 \
                          order by ic.key_ordinal for xml path(''), type).value('.', 'nvarchar(max)'), 1, 0, '') + ')' + \
                   isnull((select ' include (' + \
                     stuff((select quotename(c2.name) + \
                              case when ic2.index_column_id < (select max(index_column_id) from sys.index_columns \
                                     where object_id = i.object_id and index_id = i.index_id and is_included_column = 1) \
                                   then ', ' else '' end \
                            from sys.index_columns ic2 \
                            join sys.columns c2 on c2.object_id = ic2.object_id and c2.column_id = ic2.column_id \
                            where ic2.object_id = i.object_id and ic2.index_id = i.index_id and ic2.is_included_column = 1 \
                            for xml path(''), type).value('.', 'nvarchar(max)'), 1, 0, '') + ')'), '') + \
                   case when i.has_filter = 1 then ' where ' + i.filter_definition else '' end \
                 end + ';' \
               from sys.indexes i \
               join sys.tables t on t.object_id = i.object_id \
               where t.name = {lit} and i.is_primary_key = 0 and i.type > 0 and i.name is not null \
               order by i.is_unique_constraint, i.name \
               for xml path(''), type).value('.', 'nvarchar(max)'), 1, 1, '') as ddl"
        );
        // 外键：含删除/更新动作。`for xml path('')` 而不是 `string_agg`：后者要 2017+，
        // 而这些都是要贴给用户直接执行的语句，不该带「你的版本太老」的隐性前提。
        let fkeys = format!(
            "select stuff(( \
               select char(10) + 'alter table ' + quotename(schema_name(t.schema_id)) + '.' + quotename(t.name) + \
                 ' add constraint ' + quotename(fk.name) + ' foreign key (' + \
                 stuff((select quotename(pc.name) + \
                          case when fkc.constraint_column_id < (select max(constraint_column_id) from sys.foreign_key_columns \
                                                                 where constraint_object_id = fk.object_id) \
                               then ', ' else '' end \
                        from sys.foreign_key_columns fkc \
                        join sys.columns pc on pc.object_id = fkc.parent_object_id and pc.column_id = fkc.parent_column_id \
                        where fkc.constraint_object_id = fk.object_id \
                        order by fkc.constraint_column_id for xml path(''), type).value('.', 'nvarchar(max)'), 1, 0, '') + \
                 ') references ' + quotename(schema_name(rt.schema_id)) + '.' + quotename(rt.name) + ' (' + \
                 stuff((select quotename(rc.name) + \
                          case when fkc2.constraint_column_id < (select max(constraint_column_id) from sys.foreign_key_columns \
                                                                  where constraint_object_id = fk.object_id) \
                               then ', ' else '' end \
                        from sys.foreign_key_columns fkc2 \
                        join sys.columns rc on rc.object_id = fkc2.referenced_object_id and rc.column_id = fkc2.referenced_column_id \
                        where fkc2.constraint_object_id = fk.object_id \
                        order by fkc2.constraint_column_id for xml path(''), type).value('.', 'nvarchar(max)'), 1, 0, '') + ')' + \
                 case fk.delete_referential_action when 1 then ' on delete cascade' \
                      when 2 then ' on delete set null' when 3 then ' on delete set default' else '' end + ';' \
               from sys.foreign_keys fk \
               join sys.tables t on t.object_id = fk.parent_object_id \
               join sys.tables rt on rt.object_id = fk.referenced_object_id \
               where t.name = {lit} \
               order by fk.name \
               for xml path(''), type).value('.', 'nvarchar(max)'), 1, 1, '') as ddl"
        );
        vec![
            ("注释（扩展属性 MS_Description）", comments),
            ("索引与唯一约束", indexes),
            ("外键", fkeys),
        ]
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
        match self.key() {
            // H2 2.x：`information_schema.tables` **没有属主列**，属主要从 `schemata.schema_owner`
            // 取；`table_privileges` 只记显式授权（属主不会在里面出现，所以必须并上属主/管理员）。
            // `is_admin` 在 2.3 是 BOOLEAN（写成 `= 1` 会报「BOOLEAN 与 INTEGER 不可比」——实测）。
            "h2" => Meta::Sql(
                "select t.table_name as name from information_schema.tables t \
                 where t.table_schema = current_schema and ( \
                   exists (select 1 from information_schema.users u \
                           where u.user_name = current_user and u.is_admin) \
                   or exists (select 1 from information_schema.schemata s \
                              where s.schema_name = t.table_schema \
                                and s.schema_owner = current_user) \
                   or exists (select 1 from information_schema.table_privileges p \
                              where p.table_catalog = t.table_catalog \
                                and p.table_schema = t.table_schema \
                                and p.table_name = t.table_name \
                                and p.grantee in ('PUBLIC', current_user)) )"
                    .to_string(),
            ),
            // DB2：`SYSCAT.TABAUTH` 只有显式授权；属主在 `SYSCAT.TABLES.OWNER`。
            // 已知缺口：**组与角色的授权**不一定落在这张目录里（要枚举 `AUTH_LIST_GROUPS_FOR_AUTHID`），
            // 所以调用方把「过滤结果为空」当作"目录没覆盖"、保持原样 —— 详见 meta 里的求交逻辑。
            "db2" => Meta::Sql(
                "select t.tabname as name from syscat.tables t \
                 where t.tabschema = current schema and ( \
                   t.owner = current user \
                   or exists (select 1 from syscat.tabauth a \
                              where a.tabschema = t.tabschema and a.tabname = t.tabname \
                                and a.grantee in ('PUBLIC', current user)) )"
                    .to_string(),
            ),
            // Derby：`sys.systableperms` 只在打开 `derby.database.sqlAuthorization` 后才有行，
            // 属性为 NULL/FALSE 时**人人都有权**（默认安装就是这种），此时不能过滤。
            // 属主是「模式属主」`sys.sysschemas.authorizationid`（Derby 的 schema 与用户绑定）。
            "derby" => Meta::Sql(
                "select t.tablename as name from sys.systables t \
                 join sys.sysschemas s on s.schemaid = t.schemaid \
                 where s.schemaname = current schema and ( \
                   coalesce(syscs_util.syscs_get_database_property('derby.database.sqlAuthorization'), 'FALSE') <> 'TRUE' \
                   or s.authorizationid = current_user \
                   or exists (select 1 from sys.systableperms p \
                              where p.tableid = t.tableid \
                                and p.grantee in ('PUBLIC', current_user)) )"
                    .to_string(),
            ),
            _ => Meta::Absent,
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
        match self.key() {
            "mysql" | "mariadb" | "doris" => Meta::Sql(format!(
                "select table_name as table_name, \
                 case when table_rows is null or table_rows = 0 then null else table_rows end as rows \
                 from information_schema.tables where table_schema = {}",
                self.literal(database)
            )),
            _ => Meta::Unwritten,
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
        // Doris 的 database 参数是 `catalog.库` 全限定名（树/页签的叫法），而
        // information_schema.tables 的 table_schema 是**裸库名** —— 原样传全限定名
        // 一行都匹配不上，表注释静默丢失（真机踩过：同步建表注释全空）。
        // MySQL 系同名同义（树里就是裸名），不受影响。
        let db_value = if self.key() == "doris" {
            database.rsplit('.').next().unwrap_or(database)
        } else {
            database
        };
        let db = self.literal(db_value);
        match self.key() {
            "mysql" | "mariadb" | "doris" => Meta::Sql(format!(
                // 视图的 `table_comment` 是 MySQL 自己填的**字面量 'VIEW'**（视图没注释时），
                // 原样带出去就是「每个视图的注释都写着 VIEW」——语义上它不是注释而是类型噪音，
                // 这里按「视图 + 值恰为 VIEW」判掉；真表的同名注释不受影响。
                "select table_name as table_name, engine as engine, \
                 table_collation as charset, \
                 case when table_type = 'VIEW' and table_comment = 'VIEW' then '' \
                      else coalesce(table_comment, '') end as comment, \
                 create_time as createTime \
                 from information_schema.tables where table_schema = {db}"
            )),
            // ClickHouse：`system.tables` 一次把引擎、排序键、分区键、注释都给全
            // —— 这三项正是它「建表时确定、之后改不了」的部分，所以界面只读展示。
            //
            // **不要**在这里查 `create_time`：CH 的 `system.tables` 没有这一列
            // （26.8 实测，只有 `metadata_modification_time`，含义是元数据最后修改时间，
            // 拿来冒充建表时间就是撒谎）。写了它的后果不是「少一列」而是**整条 SQL 报
            // Code 47 Unknown expression identifier** ⇒ 引擎/排序键/分区键/注释**一起变空**，
            // 界面上就是「CH 的表除了名字啥都没有」。CH 的建表时间本来就取不到，如实不提供。
            "clickhouse" => Meta::Sql(format!(
                // 排序键去掉最外层括号：CH 存的是 DDL 里的字面量，`order by (id)` 回的就是
                // `(id)`，而界面要的是列/表达式本身（`id`）—— 括号是 DDL 语法不是名字的一部分。
                // 用户反馈：清单与「编辑表结构 → 基本信息」都显示成 `(id)`。
                // 只剥一层：`(a, b)` → `a, b`；本来就是 `id` 的（`order by id`）原样返回。
                "select name as table_name, engine as engine, \
                 replaceRegexpAll(sorting_key, '^\\((.*)\\)$', '\\1') as sortingKey, \
                 partition_key as partitionKey, coalesce(comment, '') as comment \
                 from system.tables where database = {db}"
            )),
            // PG / Kingbase：估算行数（reltuples，未 ANALYZE 时是 -1，压成 0）、属主、
            // 总占用（表+索引+toast）、表注释 —— 对象清单页一屏能看到表有多大、归谁
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select c.relname as table_name, \
                 greatest(c.reltuples, 0)::bigint::text as rows, \
                 pg_get_userbyid(c.relowner) as owner, \
                 pg_size_pretty(pg_total_relation_size(c.oid)) as size, \
                 coalesce(obj_description(c.oid, 'pg_class'), '') as comment \
                 from pg_class c join pg_namespace n on n.oid = c.relnamespace \
                 where n.nspname = {db} and c.relkind in ('r','p')"
            )),
            // Oracle：估算行数（num_rows，统计信息）、表空间、最近 ANALYZE 时间、注释。
            //
            // **别名 `rows` / `comment` 必须带双引号**：Oracle 里两者都是关键字，写 `as rows`
            // 直接报 ORA-00923（未找到要求的 FROM 关键字）—— 整条查询失败，上层是
            // `if let Ok(result)`，于是错误被静默吞掉，症状就是「表注释改完保存成功、重开还是空」
            // （真机 SYSTEM 实测：`select 1 as rows from dual` 都过不去）。旁边的
            // `tables_in_schema` 早就给 `"rows"` / `"comment"` 加了引号，这条漏了。
            //
            // 另外把 owner 从「当前用户」改成**调用方给的模式名**：这层是模式级清单，
            // 用 user_tables 只看得见连接账号自己的表 —— 浏览别人的模式时注释永远空。
            // 给的值在 all_users 里查不到（例如传的是服务名 FREEPDB1）时退回当前模式。
            "oracle" => Meta::Sql(format!(
                "select t.table_name as table_name, to_char(t.num_rows) as \"rows\", \
                 t.tablespace_name as tablespace, \
                 to_char(t.last_analyzed, 'yyyy-mm-dd hh24:mi:ss') as lastAnalyzed, \
                 coalesce(c.comments, '') as \"comment\" \
                 from all_tables t \
                 left join all_tab_comments c on c.owner = t.owner and c.table_name = t.table_name \
                 where t.owner = coalesce((select username from all_users where username = {db}), \
                   sys_context('USERENV', 'CURRENT_SCHEMA'))"
            )),
            // DM：同一套数据字典（user_tables / user_tab_comments），别名同样加引号，
            // 但 owner 维持「当前用户」—— 达梦没有可验证的实例，不动它的语义。
            "dm" => Meta::Sql(
                "select t.table_name as table_name, to_char(t.num_rows) as \"rows\", \
                 t.tablespace_name as tablespace, \
                 to_char(t.last_analyzed, 'yyyy-mm-dd hh24:mi:ss') as lastAnalyzed, \
                 coalesce(c.comments, '') as \"comment\" \
                 from user_tables t \
                 left join user_tab_comments c on c.table_name = t.table_name"
                    .to_string(),
            ),
            // SQL Server：真实行数（sys.partitions 的 heap/聚簇索引行）、建表时间、
            // 表注释仍在扩展属性 `MS_Description`（与列注释同一处）
            "sqlserver" => Meta::Sql(
                "select t.name as table_name, cast(p.rows as varchar(20)) as rows, \
                 convert(varchar(19), t.create_date, 120) as createTime, \
                 cast(ep.value as nvarchar(4000)) as comment \
                 from sys.tables t \
                 join sys.partitions p on p.object_id = t.object_id and p.index_id in (0, 1) \
                 left join sys.extended_properties ep \
                   on ep.major_id = t.object_id and ep.minor_id = 0 and ep.name = 'MS_Description'"
                    .to_string(),
            ),
            // H2 / DB2 的 information_schema 都带 remarks（就是表注释）
            "h2" => Meta::Sql(
                "select table_name as table_name, coalesce(remarks, '') as comment \
                 from information_schema.tables where table_schema = current_schema"
                    .to_string(),
            ),
            "db2" => Meta::Sql(
                "select tabname as table_name, coalesce(remarks, '') as comment \
                 from syscat.tables where tabschema = current schema and type = 'T'"
                    .to_string(),
            ),
            // SQLite / Derby 没有表注释这个概念，如实不给（界面就不会摆这一行）
            _ => Meta::Unwritten,
        }
    }

    /// 索引清单（全库返回，每条带 `table`，结构页自己按表名过滤）。
    pub fn indexes(&self) -> Meta {
        match self.key() {
            "sqlite" => Meta::Sql(
                "select name, tbl_name as \"table\", sql from sqlite_master \
                 where type = 'index' and name not like 'sqlite_autoindex%' order by name"
                    .to_string(),
            ),
            "mysql" | "mariadb" => Meta::Sql(
                "select index_name as `name`, table_name as `table`, non_unique as nonUnique, \
                 index_type as `type`, \
                 group_concat(column_name order by seq_in_index separator ',') as `columns` \
                 from information_schema.statistics where table_schema = database() \
                 group by table_name, index_name, non_unique, index_type order by table_name, index_name"
                    .to_string(),
            ),
            // Doris：**不能**用 MySQL 那种 `group_concat(x order by y separator ',')` 写法 ——
            // 实测报 `Syntax error ... near 'order by seq_in_index separator ',' as columns'`，
            // 整次索引查询 500，界面上「索引」页签于是永远是空的（看着像这张表没索引）。
            // Doris 只认 `group_concat(x, ',')`；代价是列顺序不再由 seq_in_index 保证 ——
            // 它的二级索引本来就少，复合索引列序偶尔不同，也好过整条查询挂掉。
            "doris" => Meta::Sql(
                "select index_name as `name`, table_name as `table`, non_unique as nonUnique, \
                 group_concat(column_name, ',') as `columns` \
                 from information_schema.statistics where table_schema = database() \
                 group by table_name, index_name, non_unique order by table_name, index_name"
                    .to_string(),
            ),
            "postgresql" | "kingbase" => Meta::Sql(
                "select schemaname as schema, indexname as name, tablename as \"table\", indexdef \
                 from pg_indexes where schemaname not in ('pg_catalog','information_schema') \
                 order by tablename, indexname"
                    .to_string(),
            ),
            // 别名 `schema` 必须加双引号：SCHEMA 是 T-SQL 的保留字，写 `as schema` 直接
            // `Incorrect syntax near the keyword 'schema'` —— 整条索引查询 500，
            // 「索引」页签永远是空的（巡检实测）。
            "sqlserver" => Meta::Sql(
                "select schema_name(o.schema_id) as \"schema\", i.name as name, o.name as \"table\", i.is_unique as isUnique, \
                 stuff((select ',' + c.name from sys.index_columns ic \
                  join sys.columns c on c.object_id = ic.object_id and c.column_id = ic.column_id \
                  where ic.object_id = i.object_id and ic.index_id = i.index_id and ic.key_ordinal > 0 \
                  order by ic.key_ordinal for xml path('')), 1, 1, '') as \"columns\" \
                 from sys.indexes i join sys.objects o on o.object_id = i.object_id \
                 where o.type = 'U' and i.name is not null order by o.name, i.name"
                    .to_string(),
            ),
            // Oracle / DM：原来只查 user_indexes 的 name/table，**不查列** —— 所以
            // 「包含字段」恒为空。列与列序在 user_ind_columns 里；顺带把 uniqueness 归一成
            // `isUnique`（0/1，前端归一化认这个键）、index_type 作为「类型」列。
            "oracle" | "dm" => Meta::Sql(
                "select i.index_name as \"name\", i.table_name as \"table\", \
                 case when i.uniqueness = 'UNIQUE' then 1 else 0 end as \"isUnique\", \
                 i.index_type as \"type\", \
                 listagg(c.column_name, ',') within group (order by c.column_position) as \"columns\" \
                 from user_indexes i \
                 join user_ind_columns c on c.index_name = i.index_name and c.table_name = i.table_name \
                 group by i.index_name, i.table_name, i.uniqueness, i.index_type \
                 order by i.table_name, i.index_name"
                    .to_string(),
            ),
            // DB2：同理，列在 syscat.indexcoluse（colseq 是列序）。
            // **索引名这列在 SYSCAT.INDEXES 里叫 INDNAME，不是 INDEXNAME** ——
            // 写成 INDEXNAME 会整条查询 500（SQLCODE=-206 SQLSTATE=42703），
            // 编辑表结构时「索引」页签跟着一片空白。
            "db2" => Meta::Sql(
                "select i.indname as \"name\", i.tabname as \"table\", \
                 case when i.uniquerule = 'U' then 1 else 0 end as \"isUnique\", \
                 listagg(c.colname, ',') within group (order by c.colseq) as \"columns\" \
                 from syscat.indexes i \
                 join syscat.indexcoluse c on c.indschema = i.indschema and c.indname = i.indname \
                 where i.tabschema = current schema \
                 group by i.indname, i.tabname, i.uniquerule \
                 order by i.tabname, i.indname"
                    .to_string(),
            ),
            // Derby **没有 syscat schema**（那是 DB2 的）：老写法在这里报
            // `Table/View 'SYSCAT.INDEXES' does not exist`（实测）。
            // 索引在 sys.sysconglomerates（isindex 标记），表名要连 sys.systables，
            // 且必须按 schema 过滤 —— 不然会把系统索引（SYSCONGLOMERATES_INDEX1 之类）全列出来。
            // 列：「包含字段」在 Derby 上**取不到** —— sys 目录里索引列只在
            // SYS.SYSCONGLOMERATES.descriptor 这个二进制描述符里，没有可查询的列。
            // 如实空着，别为了"看着有东西"去猜一个必然报错的查询（一旦报错，整个索引页签
            // 会 500，反而比空更难解释）。界面上那一列会是"请选择"，不会再假装有值。
            "derby" => Meta::Sql(
                "select c.conglomeratename as \"name\", t.tablename as \"table\" \
                 from sys.sysconglomerates c \
                 join sys.systables t on t.tableid = c.tableid \
                 join sys.sysschemas s on s.schemaid = t.schemaid \
                 where c.isindex = true and s.schemaname = current schema \
                 order by t.tablename, c.conglomeratename"
                    .to_string(),
            ),
            // H2：列在 information_schema.index_columns；H2 的 GROUP_CONCAT 默认分隔符就是逗号。
            // isUnique 只能从索引类型名上认（H2 的 information_schema.indexes 没有唯一性列）——
            // 尽力而为，认不出就当非唯一，不猜别的。
            "h2" => Meta::Sql(
                "select i.index_name as \"name\", i.table_name as \"table\", \
                 case when i.index_type_name like '%UNIQUE%' then 1 else 0 end as \"isUnique\", \
                 group_concat(c.column_name order by c.ordinal_position separator ',') as \"columns\" \
                 from information_schema.indexes i \
                 left join information_schema.index_columns c \
                   on c.index_name = i.index_name and c.table_name = i.table_name \
                 group by i.index_name, i.table_name, i.index_type_name \
                 order by i.table_name, i.index_name"
                    .to_string(),
            ),
            // ClickHouse：原来只取 name/table/type，**不取列** —— 于是「编辑表结构 → 索引」
            // 里的「包含字段」永远是空的（用户反馈"字段没显示出来"）。`expr` 就是跳数索引的
            // 列/表达式，接上 `columns` 就行（实测 `expr as "columns"` 返回 `event_time`）。
            //
            // **不要**加 `where database = currentDatabase()`：原句不带 WHERE，一次列出所有库
            // （界面自己按表名过滤）；加了之后，当前会话库不是 system 时就一行都查不到
            //（实测在 default 库下返回 0 行，等于把功能弄丢）。
            "clickhouse" => Meta::Sql(
                "select name, table, type, expr as \"columns\" from system.data_skipping_indices \
                 order by table, name"
                    .to_string(),
            ),
            _ => Meta::Unwritten,
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
        match self.key() {
            // SQLite 没有存储过程这个概念 —— 空数组是事实。
            "sqlite" => Meta::Absent,
            // Doris 也没有存储过程（它只有 catalog 级的 UDF，不在库的 routines 里）。
            // 之前跟着 MySQL 一起来了，于是树上画出「存储过程 / 函数」两个分类、点进去恒为空。
            "doris" => Meta::Absent,
            "mysql" | "mariadb" => Meta::Sql(
                "select routine_name as `name`, routine_type as routineType, \
                 created, last_altered as lastAltered, \
                 sql_data_access as dataAccess, security_type as securityType \
                 from information_schema.routines where routine_schema = database() \
                 order by routine_name"
                    .to_string(),
            ),
            "postgresql" | "kingbase" => {
                // 带了 schema（树的「库.模式」或用户表单传的模式名）就只列该模式的例程；
                // 没带才回退到「全库非系统模式」。与 MySQL 的 `routine_schema = database()`
                // 同一语义：下拉里列的就是当前模式自己的过程/函数。
                let filter = match schema {
                    Some(s) if !s.trim().is_empty() => {
                        format!("and n.nspname = '{}'", s.trim().replace('\'', "''"))
                    }
                    _ => String::from("and n.nspname not in ('pg_catalog','information_schema')"),
                };
                // 权限：`pg_proc` 也是公开目录，`secret_fn` 这种把 EXECUTE 从 PUBLIC 收回的
                // 例程照样会被列出来（实测低权限账号看到了它）。PG 默认把 EXECUTE 授给
                // PUBLIC，所以这条过滤通常"不改变结果"，只在真收权时生效 —— 正是要的效果。
                Meta::Sql(format!(
                    "select p.proname as name, \
                     case when p.prokind = 'f' then 'FUNCTION' else 'PROCEDURE' end as routineType \
                     from pg_proc p join pg_namespace n on n.oid = p.pronamespace \
                     where true {filter} \
                       and has_function_privilege(p.oid, 'EXECUTE') \
                       and has_schema_privilege(n.oid, 'USAGE') \
                     order by p.proname"
                ))
            }
            "sqlserver" => Meta::Sql(
                "select name, 'PROCEDURE' as routineType from sys.procedures \
                 union all select name, 'FUNCTION' as routineType from sys.objects \
                 where type in ('FN','IF','TF') order by name"
                    .to_string(),
            ),
            "oracle" => match schema {
                Some(schema) if !schema.trim().is_empty() => Meta::Sql(format!(
                    // PACKAGE 不带 BODY：EXECUTE 只授予说明（spec），对 BODY 授权是语法错
                    "select object_name as \"name\", object_type as \"routineType\" from all_objects where object_type in ('PROCEDURE','FUNCTION','PACKAGE') and upper(owner) = upper('{}') order by object_name",
                    schema.trim().replace('\'', "''")
                )),
                _ => Meta::Sql(
                    "select object_name as \"name\", object_type as \"routineType\" from user_objects where object_type in ('PROCEDURE','FUNCTION','PACKAGE') order by object_name"
                        .to_string(),
                ),
            },
            // DM（达梦）这里**不动**：它的数据字典大小写跟 Oracle 不完全一致，
            // 没在真机上验证过 owner 过滤，宁可不改，也不要造一个「看着像修好了」的改动。
            "dm" => Meta::Sql(
                "select object_name as \"name\", object_type as \"routineType\" from user_objects where object_type in ('PROCEDURE','FUNCTION') order by object_name"
                    .to_string(),
            ),
            "db2" => Meta::Sql(
                "select routinename as \"name\", routinetype as \"routineType\" from syscat.routines \
                 where routineschema = current schema order by routinename"
                    .to_string(),
            ),
            // Derby：例程在 sys.sysaliases（P=过程 / F=函数）。同样要按 schema 过滤，
            // 否则会把 SYS 里的系统过程（SQLCAMESSAGE 等）当成用户的例程列出来（实测）。
            "derby" => Meta::Sql(
                "select a.alias as \"name\", a.aliastype as \"routineType\" \
                 from sys.sysaliases a \
                 join sys.sysschemas s on s.schemaid = a.schemaid \
                 where a.aliastype in ('P','F') and s.schemaname = current schema \
                 order by a.alias"
                    .to_string(),
            ),
            "h2" => Meta::Sql(
                "select routine_name as \"name\", routine_type as \"routineType\" \
                 from information_schema.routines order by routine_name"
                    .to_string(),
            ),
            // ClickHouse 没有存储过程这一概念，不是"没实现"—— 返回 Absent（空数组），
            // 别让界面把「本来没有」显示成 501 报错
            "clickhouse" => Meta::Absent,
            _ => Meta::Unwritten,
        }
    }

    /// 触发器清单。`schema` 含义同 `procedures`（Oracle 同样是
    /// `user_triggers` → `all_triggers` + `owner` 过滤）。
    pub fn triggers(&self, schema: Option<&str>) -> Meta {
        match self.key() {
            "sqlite" => Meta::Sql(
                "select name, tbl_name as \"table\" from sqlite_master where type = 'trigger' order by name"
                    .to_string(),
            ),
            // Doris 没有触发器（information_schema.triggers 表本身都不存在）
            "doris" => Meta::Absent,
            "mysql" | "mariadb" => Meta::Sql(
                "select trigger_name as `name`, event_object_table as `table`, action_timing as timing, \
                 event_manipulation as event from information_schema.triggers \
                 where trigger_schema = database() order by trigger_name"
                    .to_string(),
            ),
            // PG 的触发器清单原先**既不看模式也不看权限**：`pg_trigger` 是全库公开目录，
            // 一句话就把所有 schema（含别的库对象可见的部分）的触发器全列出来 —— 树上
            // 「触发器」节点混着 pg_catalog 与别人 schema 的东西，而且包含当前账号
            // 根本读不了的表的触发器。改成与 procedures 同口径：带 schema 就只看它，
            // 不带就排除系统模式；再用「表级任意权限」把读不到的表上的触发器滤掉。
            "postgresql" | "kingbase" => {
                let filter = match schema {
                    Some(s) if !s.trim().is_empty() => {
                        format!("and n.nspname = '{}'", s.trim().replace('\'', "''"))
                    }
                    _ => String::from(
                        "and n.nspname not in ('pg_catalog','information_schema') \
                         and n.nspname not like 'pg\\_%'",
                    ),
                };
                Meta::Sql(format!(
                    "select t.tgname as name, c.relname as \"table\" from pg_trigger t \
                     join pg_class c on c.oid = t.tgrelid \
                     join pg_namespace n on n.oid = c.relnamespace \
                     where not t.tgisinternal {filter} \
                       and has_table_privilege(c.oid, \
                         'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') \
                     order by t.tgname"
                ))
            }
            "sqlserver" => Meta::Sql(
                "select t.name as name, o.name as \"table\" from sys.triggers t \
                 left join sys.objects o on o.object_id = t.parent_id order by t.name"
                    .to_string(),
            ),
            "oracle" => match schema {
                Some(schema) if !schema.trim().is_empty() => Meta::Sql(format!(
                    "select trigger_name as \"name\", table_name as \"table\" from all_triggers where upper(owner) = upper('{}') order by trigger_name",
                    schema.trim().replace('\'', "''")
                )),
                _ => Meta::Sql(
                    "select trigger_name as \"name\", table_name as \"table\" from user_triggers order by trigger_name"
                        .to_string(),
                ),
            },
            // DM 同上：先不动。
            "dm" => Meta::Sql(
                "select trigger_name as \"name\", table_name as \"table\" from user_triggers order by trigger_name"
                    .to_string(),
            ),
            "db2" => Meta::Sql(
                "select trigname as \"name\", tabname as \"table\" from syscat.triggers \
                 where tabschema = current schema order by trigname"
                    .to_string(),
            ),
            // Derby 没有 syscat（DB2 的 schema）：触发器的系统表是 sys.systriggers。
            // 列名 `TRIGGERNAME` 实测存在；表名不在该表里（定义文本里才有所属表），
            // 所以这里只给名字 —— 少一列好过报 42X05 让整个清单打不开。
            // Derby：与 procedures 同一口径 —— 不加 schema 过滤会把内核自带的触发器
            // （SYS 下的那些）也列出来，而当前账号对它们没有任何权限。
            // 列名仍是 TRIGGERNAME，只是多了到 sys.sysschemas 的一跳。
            "derby" => Meta::Sql(
                "select t.triggername as \"name\" from sys.systriggers t \
                 join sys.sysschemas s on s.schemaid = t.schemaid \
                 where s.schemaname = current schema order by t.triggername"
                    .to_string(),
            ),
            // ClickHouse / H2 没有触发器
            _ => Meta::Absent,
        }
    }

    pub fn events(&self) -> Meta {
        match self.key() {
            // Doris 没有事件调度器
            "doris" => Meta::Absent,
            "mysql" | "mariadb" => Meta::Sql(
                "select event_name as `name`, status, interval_value, interval_field, \
                 last_executed as lastExecuted, execute_at as executeAt, ends \
                 from information_schema.events where event_schema = database() order by event_name"
                    .to_string(),
            ),
            // 其余类型确实没有 MySQL 那种「事件」
            _ => Meta::Absent,
        }
    }

    pub fn users(&self) -> Meta {
        match self.key() {
            // Doris 也走 MySQL 协议，`mysql.user` 可读；这里原先漏了它 ——
            // 同文件的 user_info / user_grants / user_action_sql 都已包含 doris，
            // 只有用户清单少一个分支，于是「树的用户节点是空的、详情却能用」。
            //
            // MySQL 8 把**角色**也放在 `mysql.user` 里（角色就是 locked account），
            // 所以清单天然混着两者。区分靠「锁定 + 空认证串」：真被 ACCOUNT LOCK 的用户
            // 仍有认证串（实测 mysql.sys/session/infoschema 都是 70 字节），角色则为空。
            // MariaDB 有现成的 `is_role` 列（10.0.5+）；它的 collation 与字面量不一致，
            // `is_role = 'Y'` 会报 1267，必须转 binary 再比。
            // ⚠️ host 必须**显式起别名**：MariaDB 的驱动按底层列名回 `Host`（大写），
            // 而前端读的是 `row.host` —— 不写别名时 MariaDB 的用户清单里「主机」一列
            // 是空的（真机回归发现）。MySQL 与 MariaDB 在这一处行为不一致，别赌。
            //
            // **角色不进用户清单**（用户要求）：角色在「编辑 / 授权」的「角色」页签里管。
            // MySQL 8 的角色就是 `account_locked='Y'` 且认证串为空的账号，这里排除掉
            // （真被锁的用户仍有认证串，不会被误排除）；MariaDB 直接用 `is_role` 列
            // （collation 与字面量不一致，必须转 binary 再比，否则 1267）。
            // 「超级用户」标记：MySQL / MariaDB 直接取 `Super_priv`，**不要写成
            // `super_priv = 'Y'`** —— MariaDB 的该列是 collation 不同的 CHAR，
            // 与字面量比较会报 1267（实测 `Illegal mix of collations …
            // utf8mb4_general_ci / utf8mb4_uca1400_ai_ci`），整张清单直接 500。
            // 取原值（Y/N）出门，由 Rust 侧的 `is_true_flag` 折成布尔。
            // Doris 没有 Super_priv，管理员是 `Admin_priv`（实测 root/admin 都是 Y）。
            "mysql" => Meta::Sql(
                "select user as `name`, host as `host`, super_priv as `isSuper` from mysql.user \
                 where not (account_locked = 'Y' and coalesce(authentication_string, '') = '') \
                 order by user, host"
                    .to_string(),
            ),
            "mariadb" => Meta::Sql(
                "select user as `name`, host as `host`, super_priv as `isSuper` from mysql.user \
                 where coalesce(cast(is_role as binary), 'N') <> 'Y' \
                 order by user, host"
                    .to_string(),
            ),
            "doris" => Meta::Sql(
                "select user as `name`, host as `host`, admin_priv as `isSuper` from mysql.user \
                 order by user, host"
                    .to_string(),
            ),
            // PG 的角色与用户是同一张表：**不能登录**的就是纯角色（rolcanlogin = false），
            // 它们同样只出现在「角色」页签里，不进用户清单。
            // `isSuper` 必须加双引号：PG 会把未加引号的别名折成 `issuper`，
            // 前端读 `row.isSuper` 就永远拿不到（超级用户标签不显示）。
            "postgresql" | "kingbase" => Meta::Sql(
                "select rolname as name, rolsuper as \"isSuper\" from pg_roles \
                 where rolcanlogin order by rolname"
                    .to_string(),
            ),
            // SQL Server：**服务器登录名**（不是库用户）。
            //
            // 账号体系是「服务器登录名 + 每个库里的映射用户」，而这页是**连接级**（不跟库）。
            // 原来列的是库用户（sys.database_principals），于是打开 demo 这种只有
            // dbo/guest 的库永远是一片空白 —— 用户的原话是「SQL Server 的用户也显示出来吧」。
            // 登录名才是用户眼里的"用户"，也与增删改（create/alter/drop login）同一口径。
            //
            // 只过滤系统内置：`##…` 是证书/策略签名登录，`NT …` / `BUILTIN…` 是机器账号。
            // `sa` 必须保留 —— 那正是用户要找的那个。
            //
            // `isSuper` 用 `is_srvrolemember('sysadmin', name)` 现场算（实测 sa → 1），
            // 这样「超级用户」标签在这一页才有意义；非 sysadmin 账号调用得到 NULL，
            // 折成 false，不会把所有人都标成超级用户。
            "sqlserver" => Meta::Sql(
                "select name, is_srvrolemember('sysadmin', name) as \"isSuper\" \
                 from sys.server_principals \
                 where type in ('S','U','G') \
                   and name not like '##%' \
                   and name not like 'NT %' \
                   and name not like 'BUILTIN%' \
                 order by name"
                    .to_string(),
            ),
            "oracle" | "dm" => {
                Meta::Sql("select username as \"name\" from all_users order by username".to_string())
            }
            "clickhouse" => {
                Meta::Sql("select name from system.users order by name".to_string())
            }
            // H2：`INFORMATION_SCHEMA.USERS`（2.x，实测 2.3.232）。默认用户的用户名是
            // **空串**，直接显示会是一行点不动的空白 —— 显示成 H2 的空标识符 `""`
            // （带引号，且能直接贴进 SQL），查详情 / 改 / 删时再还原（见 `h2_user`）。
            "h2" => Meta::Sql(
                "select case when coalesce(user_name,'') = '' then '\"\"' else user_name end as \"name\" \
                 from information_schema.users order by 1"
                    .to_string(),
            ),
            // Derby / DB2：用户由**外部认证**（操作系统 / LDAP）提供，这里只能看不能建 ——
            // `user_action_sql` 保持 Absent，界面上的「新增用户」会自动置灰。
            "derby" => Meta::Sql("select username as \"name\" from sys.sysusers order by username".to_string()),
            // DB2 LUW 的用户/组授权目录是 SYSCAT.DBAUTH（SYSIBM.SYSUSERAUTH 是 z/OS 的）
            "db2" => Meta::Sql(
                "select grantee as \"name\" from syscat.dbauth \
                 where grantee <> 'PUBLIC' and grantee not like 'SYS%' order by grantee"
                    .to_string(),
            ),
            _ => Meta::Absent,
        }
    }

/// Oracle 的**授予**部分：角色 + 系统权限 + 对象权限。
///
/// 三类分别对应字典里的三层（角色 / 系统权限 / 对象权限），语句形态也各不同：
/// `grant <角色> to u`、`grant <系统权限> to u`、`grant <权限> on <模式>.<对象> to u`。
/// 角色与权限名都来自白名单（调用方已校验过），标识符一律 `quote()`，不留手工拼接的口子。
fn oracle_grant_parts(u: &str, roles: &[String], privileges: &UserPrivileges) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // 角色（`roles` 与 `privileges.global` 都可能带角色名，去重后再发，免得同句两遍）
    for name in roles.iter().chain(privileges.global.iter()) {
        let raw = name.trim();
        let name = raw.to_ascii_uppercase();
        if name.is_empty() || !seen.insert("r:".to_string() + &name) {
            continue;
        }
        if crate::api::dialect::is_oracle_role(&name) || crate::api::dialect::is_oracle_system_priv(&name) {
            parts.push(format!("grant {name} to {u}"));
        } else if crate::api::dialect::is_plain_ident(raw) {
            // **自定义角色**（用户在「新建角色」里建的那些）：预置名单里当然没有。
            // 写法要与建角色时一致 —— 普通名不加引号（Oracle 会折成大写），
            // 带特殊字符的必须原样加双引号，折大写后找不到对象（ORA-01919）。
            parts.push(format!("grant {} to {u}", Self::oracle_role_ident(raw)));
        }
    }
    // 对象权限：`database` 字段对 Oracle 就是**模式名**（树上的「库.模式」拆出来的那半）
    for (owner, object, privs) in &privileges.tables {
        let owner = owner.trim();
        let object = object.trim();
        if owner.is_empty() || object.is_empty() {
            continue;
        }
        let items: Vec<String> = privs
            .iter()
            .map(|p| p.trim().to_ascii_uppercase())
            .filter(|p| crate::api::dialect::is_oracle_object_priv(p))
            .collect();
        if items.is_empty() {
            continue;
        }
        parts.push(format!(
            "grant {} on {}.{} to {u}",
            items.join(", "),
            Self::quote_oracle(owner),
            Self::quote_oracle(object)
        ));
    }
    parts
}

/// 把界面传回来的角色引用拆成 (名字, host)。
///
/// 前端统一传 `名字@host`（来自 `mysql.role_edges` 的 from_user/from_host）；
/// 只有裸名字时按 `%`（与 `grant 'r' to …` 的默认 host 一致）。
fn split_role_ref(raw: &str) -> (String, String) {
    let s = raw.trim();
    match s.rsplit_once('@') {
        Some((n, h)) if !n.trim().is_empty() && !h.trim().is_empty() => {
            (n.trim().to_string(), h.trim().to_string())
        }
        _ => (s.to_string(), "%".to_string()),
    }
}

/// MySQL 的**撤销**部分：界面按"原来有、现在没有了"算好传进来，这里只负责成句。
/// 与 Oracle 那套同一机制，只是作用目标写法不同：
///   scope=global        → `revoke <priv> on *.* from <u>`
///   scope=db            → `revoke <priv> on \`<库>\`.* from <u>`
///   scope=table         → `revoke <priv> on \`<库>\`.\`<表>\` from <u>`
/// `GRANT OPTION` 也走这一层（它是 `revoke ... grant option` 子句的一部分，MySQL 里合法）。
/// `grantee` 是**已转义好的单引号账号**形如 `'u'@'%'`（见上面的 `full_lit`），不是裸名。
fn mysql_revoke_parts(&self, grantee: &str, revokes: &[GrantRevoke]) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    for row in revokes {
        let name = row.privilege.trim().to_ascii_uppercase();
        if name.is_empty() {
            continue;
        }
        let owner = row.owner.trim();
        let object = row.object.trim();
        match row.scope.trim().to_ascii_lowercase().as_str() {
            "db" | "table" | "object" | "schema" => {
                if owner.is_empty() {
                    continue;
                }
                let target = if object.is_empty() {
                    format!("{}.*", self.quote(owner))
                } else {
                    format!("{}.{}", self.quote(owner), self.quote(object))
                };
                parts.push(format!("revoke {name} on {target} from {grantee}"));
            }
            // 例程级：`revoke execute on procedure `db`.`p` from u` —— 类型字是语法的一部分
            "routine" => {
                if owner.is_empty() || object.is_empty() {
                    continue;
                }
                let routine_type = if row.kind.eq_ignore_ascii_case("function") { "function" } else { "procedure" };
                parts.push(format!(
                    "revoke {name} on {routine_type} {}.{} from {grantee}",
                    self.quote(owner),
                    self.quote(object)
                ));
            }
            // 角色：MySQL 是 `revoke 'role'@'host' from 'u'@'%'`（owner = 角色名，
            // object = 角色 host）；MariaDB / Doris 的角色没有 host，只能 `revoke 'r' from …`。
            "role" => {
                if owner.is_empty() {
                    continue;
                }
                if self.key() == "mysql" {
                    let role_host = if object.is_empty() { "%" } else { object };
                    parts.push(format!(
                        "revoke {}@{} from {grantee}",
                        self.literal(owner),
                        self.literal(role_host)
                    ));
                } else {
                    parts.push(format!("revoke {} from {grantee}", self.literal(owner)));
                }
            }
            // 全局（含 GRANT OPTION）
            _ => parts.push(format!("revoke {name} on *.* from {grantee}")),
        }
    }
    parts
}

/// Oracle 的**撤销**部分：界面按"原来有、现在没有了"算好传进来，这里只负责成句。
fn oracle_revoke_parts(u: &str, revokes: &[GrantRevoke]) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    for row in revokes {
        let name = row.privilege.trim().to_ascii_uppercase();
        if name.is_empty() {
            continue;
        }
        match row.scope.trim().to_ascii_lowercase().as_str() {
            "object" => {
                let owner = row.owner.trim();
                let object = row.object.trim();
                if owner.is_empty() || object.is_empty() || !crate::api::dialect::is_oracle_object_priv(&name) {
                    continue;
                }
                parts.push(format!(
                    "revoke {name} on {}.{} from {u}",
                    Self::quote_oracle(owner),
                    Self::quote_oracle(object)
                ));
            }
            "role" => {
                let raw = row.privilege.trim();
                // 预置角色 + 用户自定义角色（后端建的那些），同一套写法见 oracle_grant_parts
                if crate::api::dialect::is_oracle_role(&name) || crate::api::dialect::is_plain_ident(raw) {
                    parts.push(format!("revoke {} from {u}", Self::oracle_role_ident(raw)));
                }
            }
            // 系统权限
            _ => {
                if crate::api::dialect::is_oracle_system_priv(&name) {
                    parts.push(format!("revoke {name} from {u}"));
                }
            }
        }
    }
    parts
}

/// Oracle 的双引号标识符（这里不能借 `Dialect::quote`：那两个函数在 impl 之外使用）。
fn quote_oracle(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}


/// Oracle 主体的写法：**普通名不加引号**（Oracle 自动折成大写，与字典里的
/// `DBA_ROLES.ROLE` 对得上），带特殊字符的才加双引号并原样保留大小写。
///
/// 为什么必须与建角色时同一套：`create role "my-role"` 建出来是小写带连字符，
/// 后面 `grant MY-ROLE to …` 会被折成 `MY_ROLE`（不存在的对象，ORA-01919）。
fn oracle_role_ident(raw: &str) -> String {
    let s = raw.trim();
    let plain = !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if plain {
        s.to_ascii_uppercase()
    } else {
        Self::quote_oracle(s)
    }
}

/// Oracle 的 `dbms_metadata.get_ddl(...)`：有 owner 就把 owner 也带上。
///
/// 为什么非带不可：`get_ddl` 不带 owner 时按**会话默认模式**解析，而 DBMind 的查询会话
/// 刻意不按"树上选中的模式"切换（不赌代理有没有真的切 schema）。于是"在 SYS 下点一个过程"
/// 会去 SYSTEM 里找，报 ORA-31603 —— 真机上报的就是这个。
/// owner 也 `upper()`：未加引号的标识符在字典里是大写，但用户可能建了带引号的小写模式。
fn oracle_get_ddl(kind: &str, name_literal: &str, schema_lit: &Option<String>) -> String {
    match schema_lit {
        Some(owner) => format!(
            "select dbms_metadata.get_ddl('{kind}', upper({name_literal}), upper({owner})) as definition from dual"
        ),
        None => format!(
            "select dbms_metadata.get_ddl('{kind}', upper({name_literal})) as definition from dual"
        ),
    }
}

/// 对象定义（右键「编辑」用它取到 SQL 再转 ALTER）。
    ///
    /// `schema` 含义同 [`Self::ddl`]：Oracle 要靠它做 `owner`，PG 要靠它筛 `pg_namespace`。
    /// 不带时：Oracle 的 `get_ddl` 按会话默认模式找（真机就是这么报 ORA-31603 的），
    /// PG 会在所有非系统模式里取第一个同名对象。
    pub fn object_source(&self, object_type: &str, name: &str, schema: Option<&str>) -> Meta {
        let name_literal = self.literal(name);
        let schema_lit = schema
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| self.literal(s));
        // PG：追加命名空间条件（空串 = 保持原行为，不动没给模式的调用方）
        let ns_filter = match &schema_lit {
            Some(s) => format!(" and n.nspname = {s}"),
            None => String::new(),
        };
        // pg_proc 那条没有 n 别名，单独一个（按 pronamespace 比）
        let proc_ns_filter = match &schema_lit {
            Some(s) => format!(
                " and p.pronamespace = (select oid from pg_namespace where nspname = {s})"
            ),
            None => String::new(),
        };
        match (self.key(), object_type) {
            ("sqlite", _) => Meta::Sql(format!(
                "select sql from sqlite_master where name = {name_literal}"
            )),
            ("mysql" | "mariadb" | "doris", "view") => {
                Meta::Sql(format!("show create view {}", self.quote(name)))
            }
            ("mysql" | "mariadb", "procedure") => {
                Meta::Sql(format!("show create procedure {}", self.quote(name)))
            }
            ("mysql" | "mariadb", "function") => {
                Meta::Sql(format!("show create function {}", self.quote(name)))
            }
            ("mysql" | "mariadb", "trigger") => {
                Meta::Sql(format!("show create trigger {}", self.quote(name)))
            }
            // 事件调度器：MySQL 有 `show create event`，之前整块漏了 —— 树上有「事件」分类，
            // 点进去却是 501，纯粹是分支没写
            ("mysql" | "mariadb", "event") => {
                Meta::Sql(format!("show create event {}", self.quote(name)))
            }
            ("postgresql" | "kingbase", "view") => Meta::Sql(format!(
                "select pg_get_viewdef(c.oid) as definition from pg_class c \
                 join pg_namespace n on n.oid = c.relnamespace \
                 where c.relname = {name_literal} and n.nspname not in ('pg_catalog','information_schema'){ns_filter}"
            )),
            ("postgresql" | "kingbase", "procedure" | "function") => Meta::Sql(format!(
                "select pg_get_functiondef(p.oid) as definition from pg_proc p \
                 where p.proname = {name_literal}{proc_ns_filter} limit 1"
            )),
            // PG 的触发器定义（含 `ON <表>` 与 WHEN 条件），`pg_get_triggerdef` 就是官方入口
            ("postgresql" | "kingbase", "trigger") => Meta::Sql(format!(
                "select pg_get_triggerdef(t.oid) as definition from pg_trigger t \
                 join pg_class c on c.oid = t.tgrelid \
                 join pg_namespace n on n.oid = c.relnamespace \
                 where t.tgname = {name_literal} and not t.tgisinternal \
                   and n.nspname not in ('pg_catalog','information_schema'){ns_filter}"
            )),
            // Oracle：`dbms_metadata.get_ddl` 给的是 `CREATE OR REPLACE ...` 全文，比逐行拼
            // `user_source` 可靠。**必须带 owner**（第三个参数）—— 不带就按会话默认模式找，
            // 在 SYS 下双击一个过程而会话是 SYSTEM 时会报 ORA-31603「对象在方案 SYSTEM 中未找到」。
            ("oracle", "view") => Meta::Sql(Self::oracle_get_ddl("VIEW", &name_literal, &schema_lit)),
            ("oracle", "procedure") => {
                Meta::Sql(Self::oracle_get_ddl("PROCEDURE", &name_literal, &schema_lit))
            }
            ("oracle", "function") => Meta::Sql(Self::oracle_get_ddl("FUNCTION", &name_literal, &schema_lit)),
            ("oracle", "trigger") => Meta::Sql(Self::oracle_get_ddl("TRIGGER", &name_literal, &schema_lit)),
            // DM（达梦）：三参 `get_ddl` 没真机验证过（它的 dbms_metadata 支持面不确定），
            // 先保持两句 —— 与「达梦的例程清单先不动」同一个原则。
            ("dm", "view") => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('VIEW', upper({name_literal})) as definition from dual"
            )),
            ("dm", "procedure") => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('PROCEDURE', upper({name_literal})) as definition from dual"
            )),
            ("dm", "function") => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('FUNCTION', upper({name_literal})) as definition from dual"
            )),
            ("dm", "trigger") => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('TRIGGER', upper({name_literal})) as definition from dual"
            )),
            // 以下三组是**该方言根本没有这类对象**，不是"没实现"：如实返回 `Absent`。
            // 这个区别很重要（见 export.rs 导出循环里的注释）：
            //   `Absent`    → 该方言没有这种东西，界面不摆"查看定义/编辑"，导出**静默跳过**；
            //   `Unwritten` → 功能还没写，导出会逐个对象提示"定义查询尚未实现，未导出"。
            // PG / Kingbase / Oracle / DM / DB2 / H2 / Derby 都没有 MySQL 那种事件调度器；
            // ClickHouse / Doris 没有存储过程/函数/触发器/事件；
            // Derby 的例程没有可查目录（下面 Derby 分支的注释里也说了）。
            ("postgresql" | "kingbase" | "oracle" | "dm" | "db2" | "h2" | "derby", "event") => {
                Meta::Absent
            }
            ("clickhouse" | "doris", "procedure" | "function" | "trigger" | "event") => Meta::Absent,
            ("derby", "procedure" | "function") => Meta::Absent,
            ("sqlserver", "event") => Meta::Absent,
            ("sqlserver", _) => Meta::Sql(format!(
                "select object_definition(object_id({name_literal})) as definition"
            )),
            // ClickHouse：视图/表都有 `show create`，与 `ddl()` 里那条同源
            ("clickhouse", "view") => Meta::Sql(format!("show create view {}", self.quote(name))),
            ("clickhouse", "table") => Meta::Sql(format!("show create table {}", self.quote(name))),
            // H2：定义都在 information_schema 里（比较用**字符串字面量**，
            // 不能用标识符 —— `where table_name = upper(<标识符>)` 是把列引用当值用了）
            ("h2", "view") => Meta::Sql(format!(
                "select view_definition as definition from information_schema.views \
                 where table_name = upper({name_literal})"
            )),
            ("h2", "procedure") | ("h2", "function") => Meta::Sql(format!(
                "select routine_definition as definition from information_schema.routines \
                 where routine_name = upper({name_literal})"
            )),
            // DB2：syscat 里的 TEXT 列就是定义原文（视图/例程/触发器都有）
            ("db2", "view") => Meta::Sql(format!(
                "select text as definition from syscat.views \
                 where viewschema = current schema and viewname = upper({name_literal})"
            )),
            ("db2", "procedure") | ("db2", "function") => Meta::Sql(format!(
                "select text as definition from syscat.routines \
                 where routineschema = current schema and routinename = upper({name_literal})"
            )),
            ("db2", "trigger") => Meta::Sql(format!(
                "select text as definition from syscat.triggers \
                 where trigschema = current schema and trigname = upper({name_literal})"
            )),
            // Derby：`sys.sysviews` / `sys.systriggers` 存着定义原文
            // （例程在 Derby 里没有目录可查，只能保持"取不到"，如实返回 Absent）
            // Derby 10.15 起 `sys.sysviews` 只有 `tableid` + `viewdefinition`
            // （`viewname` 已不在该表里，按它查会报「Column 'VIEWNAME' ...」——实测），
            // 视图名要连 `sys.systables` 取
            ("derby", "view") => Meta::Sql(format!(
                "select v.viewdefinition as definition from sys.sysviews v \
                 join sys.systables t on t.tableid = v.tableid \
                 where t.tablename = upper({name_literal})"
            )),
            ("derby", "trigger") => Meta::Sql(format!(
                "select triggerdefinition as definition from sys.systriggers \
                 where triggername = upper({name_literal})"
            )),
            _ => Meta::Unwritten,
        }
    }

    /// 表级危险操作（清空 / 截断 / 删除 / 重命名）。
    pub fn table_action(&self, table: &str, action: &str, new_name: Option<&str>) -> Meta {
        let t = self.quote(table);
        match action {
            // ClickHouse 的 `DELETE FROM` 是「轻量删除」，老版本/未开启时直接报错；
            // 而「清空整表」本来就有 TRUNCATE 这条正路 —— 按语义选语句，别让用户先撞一次错
            "clear" => {
                if self.key() == "clickhouse" {
                    Meta::Sql(format!("truncate table {t}"))
                } else {
                    Meta::Sql(format!("delete from {t}"))
                }
            }
            // SQLite 没有 TRUNCATE，截断退化成 DELETE（一样会清空数据，这点必须说清）
            "truncate" => match self.key() {
                "sqlite" => Meta::Sql(format!("delete from {t}")),
                // DB2 的 TRUNCATE 必须显式写 IMMEDIATE：`truncate table t` 直接
                // SQLCODE=-104 SQLSTATE=42601（真机实测，报错信息里就提示 END-OF-STATEMENT ...;IMMEDIATE）
                "db2" => Meta::Sql(format!("truncate table {t} immediate")),
                _ => Meta::Sql(format!("truncate table {t}")),
            },
            "drop" => Meta::Sql(format!("drop table {t}")),
            "rename" => match new_name {
                Some(new_name) => {
                    let n = self.quote(new_name);
                    match self.key() {
                        // MySQL 系与 DB2 有各自的 RENAME TABLE
                        "mysql" | "mariadb" | "db2" => Meta::Sql(format!("rename table {t} to {n}")),
                        // Derby **没有** `ALTER TABLE … RENAME TO`（真机：`Syntax error:
                        // Encountered "rename" at line 1, column 26`），路由与 MySQL 一样是
                        // 独立的 `RENAME TABLE old TO new`（真机验过：改名后新表可查、旧表消失）
                        "derby" => Meta::Sql(format!("rename table {t} to {n}")),
                        // ClickHouse **没有** `ALTER TABLE … RENAME TO`：它会报
                        // `Code: 62 … Syntax error: failed at position N (to): to xxx. Expected COLUMN`
                        // （它的 ALTER TABLE RENAME 只认 `RENAME COLUMN old TO new`）。
                        // 改表名要用独立的 `RENAME TABLE a TO b` —— 真机上两种形式都对过：
                        // 前者 Code 62，后者 OK。
                        "clickhouse" => Meta::Sql(format!("rename table {t} to {n}")),
                        // SQL Server **没有** `ALTER TABLE … RENAME TO` —— 改表名只有 `sp_rename`。
                        // 之前它落进下面的通用分支，用户点「重命名表」只会拿到一句语法错。
                        // 两个细节：sp_rename 收的是**裸名**（它自己解析，不要带标识符引号）；
                        // 并且**不会更新引用该表的视图/存储过程**（前端会给这句提醒）。
                        "sqlserver" => Meta::Sql(format!(
                            "exec sp_rename {}, {}",
                            self.literal(table),
                            self.literal(new_name)
                        )),
                        // Doris 的 `ALTER TABLE … RENAME` **不带 TO**：
                        // 写成 `rename to` 直接 `Syntax error in line 1`（真机实测）。
                        "doris" => Meta::Sql(format!("alter table {t} rename {n}")),
                        // PostgreSQL / SQLite / Oracle / H2 / Derby
                        // 都是 `ALTER TABLE … RENAME TO`（ClickHouse 见上面单独一支）
                        _ => Meta::Sql(format!("alter table {t} rename to {n}")),
                    }
                }
                None => Meta::Unwritten,
            },
            _ => Meta::Unwritten,
        }
    }

    /// 删除视图 / 例程 / 触发器 / 事件（同步的「删除重建」策略要用）。
    ///
    /// `drop X if exists` 不是所有方言都认：MySQL 系、PostgreSQL、SQLite、H2、ClickHouse（视图）、
    /// SQL Server 2016+ 都支持；Oracle / DM / DB2 / Derby 没有这个写法。
    /// 与其拼一句"看着像、执行就报错"的语句，不如返回 `None`，由调用方跳过并说明白。
    pub fn drop_object_sql(&self, kind: &str, name: &str) -> Option<String> {
        let supported = match self.key() {
            "clickhouse" => matches!(kind, "view"),
            "sqlite" => matches!(kind, "view" | "trigger"),
            "mysql" | "mariadb" | "doris" => {
                matches!(kind, "view" | "procedure" | "function" | "trigger" | "event")
            }
            "postgresql" | "kingbase" => matches!(kind, "view" | "function" | "trigger"),
            "sqlserver" => matches!(kind, "view" | "procedure" | "function" | "trigger"),
            "h2" => matches!(kind, "view"),
            _ => false,
        };
        if !supported {
            return None;
        }
        Some(format!("drop {kind} if exists {}", self.quote(name)))
    }

    /// 分页子句（`offset` 从 0 开始）。
    pub fn limit_clause(&self, offset: u64, size: u64) -> String {
        match self.key() {
            "sqlserver" => format!("offset {offset} rows fetch next {size} rows only"),
            // Derby 没有 LIMIT 语法（真机：Encountered "limit"），用 SQL 标准的 OFFSET/FETCH
            "oracle" | "dm" | "derby" => format!("offset {offset} rows fetch next {size} rows only"),
            _ => format!("limit {size} offset {offset}"),
        }
    }

    /// SQL Server 的 `OFFSET` 必须跟 `ORDER BY` 才能用。
    pub fn needs_order_by_for_paging(&self) -> bool {
        self.key() == "sqlserver"
    }

    /// 清空一张表的语句（"清空后导入"模式用）。
    ///
    /// 为什么不能统一写 `delete from`：**ClickHouse 不吃裸 DELETE** ——
    /// 实测报 `Code: 62 ... Syntax error: failed at position 26 (end of query):
    /// . Expected one of: ON, IN, PARTITION, WHERE`（它要求带 `WHERE`，或者用 TRUNCATE）。
    /// 反过来 SQLite / Derby 没有 `TRUNCATE`，只能 `delete from`。
    /// 其余方言两者都认，选 `truncate`（快得多）。
    pub fn clear_table_sql(&self, table: &str) -> String {
        match self.key() {
            // 没有 TRUNCATE 的方言
            "sqlite" | "derby" => format!("delete from {table}"),
            _ => format!("truncate table {table}"),
        }
    }

    /// 更新一行的语句（upsert 的更新分支用）。
    ///
    /// **ClickHouse 没有标准 `UPDATE ... SET ...`**：它只接受
    /// `ALTER TABLE ... UPDATE ... WHERE ...`（异步 mutation）。写标准 UPDATE 会被它拒掉。
    pub fn update_sql(&self, table: &str, assignments: &str, condition: &str) -> String {
        match self.key() {
            "clickhouse" => format!("alter table {table} update {assignments} where {condition}"),
            _ => format!("update {table} set {assignments} where {condition}"),
        }
    }

    pub fn count_sql(&self, table: &str) -> String {
        format!("select count(*) as cnt from {}", self.quote(table))
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
        self.create_table_with_comments(
            table,
            columns,
            source,
            &std::collections::HashMap::new(),
            None,
        )
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
            let mut type_name = if cross {
                family.render(self.kind)
            } else {
                raw_type
            };
            // **Doris 键列不能是无界文本**：STRING/TEXT 做 UNIQUE/DUPLICATE KEY 建表直接报
            // 「String Type should not be used in key column」（真机：ClickHouse 的 String
            // 主键/排序键同步到 Doris 必炸）。键列统一落成有长度的 VARCHAR(65533)
            //（实测 Doris 接受它做键列；无键列长度上限之忧，只是普通列保持 STRING 不动）。
            let is_doris_key = self.kind.key() == "doris"
                && (column.primary_key || (!has_pk && col_index == 0));
            if is_doris_key && is_unbounded_text_type(&type_name) {
                type_name = "varchar(65533)".to_string();
            }
            // H2 目标：内核回显的完整类型文本会带 `integer(32,0)` / `double(10,0)` 这种
            // **精度括号**，H2 2.x 对整数/浮点不接受（真机：重建目标表直接语法错）——
            // 整数与浮点类剥掉括号；`character varying(50)` 的括号是合法长度，保留。
            if self.kind.key() == "h2" {
                let lower = type_name.to_ascii_lowercase();
                for base in ["integer", "bigint", "smallint", "tinyint", "double", "real", "float"] {
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
            if self.kind.key() == "clickhouse"
                && column.nullable
                && !column.primary_key
            {
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
            if matches!(self.kind.key(), "mysql" | "mariadb")
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
            if matches!(self.kind.key(), "mysql" | "mariadb" | "doris" | "clickhouse") {
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
                ("UNIQUE KEY", primaries.iter().filter(|name| !name.is_empty()).cloned().collect())
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
            "mysql" | "mariadb" => {
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
        if self.key() != "sqlserver" {
            // PG / Oracle / DM / DB2 / H2 的注释必须是**独立语句**（`comment on …`），
            // 不能像 MySQL 那样内联进列定义 —— 建表段也只对内联方言写注释，所以跨库同步
            // 到这些库时表注释、列注释会**全丢**（真机缺口）。这里补上这一档。
            //
            // 不支持的如实给空：SQLite / Derby 压根没有注释概念；ClickHouse 的注释走
            // `ALTER TABLE … COMMENT COLUMN` 且建表本来就内联写了；MySQL 系已在建表段内联。
            if !matches!(
                self.key(),
                "postgresql" | "kingbase" | "oracle" | "dm" | "db2" | "h2"
            ) {
                return Vec::new();
            }
            let full = if schema.trim().is_empty() {
                self.quote(table)
            } else {
                format!("{}.{}", self.quote(schema), self.quote(table))
            };
            let mut out = Vec::new();
            if let Some(tc) = table_comment.map(str::trim).filter(|c| !c.is_empty()) {
                out.push(format!("comment on table {full} is {}", self.literal(tc)));
            }
            for (name, comment) in column_comments {
                if comment.trim().is_empty() {
                    continue;
                }
                out.push(format!(
                    "comment on column {full}.{} is {}",
                    self.quote(name),
                    self.literal(comment)
                ));
            }
            return out;
        }
        let sq_schema = format!("N{}", self.literal(schema));
        let sq_table = format!("N{}", self.literal(table));
        let mut out = Vec::new();
        let render = |level2: &str, text: &str| -> String {
            let value = format!("N{}", self.literal(text));
            if level2.is_empty() {
                format!("exec sp_addextendedproperty @name=N'MS_Description', @value={}, @level0type=N'SCHEMA', @level0name={}, @level1type=N'TABLE', @level1name={}", value, sq_schema, sq_table)
            } else {
                let col = format!("N{}", self.literal(level2));
                format!("exec sp_addextendedproperty @name=N'MS_Description', @value={}, @level0type=N'SCHEMA', @level0name={}, @level1type=N'TABLE', @level1name={}, @level2type=N'COLUMN', @level2name={}", value, sq_schema, sq_table, col)
            }
        };
        if let Some(tc) = table_comment.map(str::trim).filter(|c| !c.is_empty()) {
            out.push(render("", tc));
        }
        for (name, comment) in column_comments {
            if comment.trim().is_empty() { continue; }
            out.push(render(name, comment));
        }
        let _ = table;
        out
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
        match self.key() {
            "mysql" | "mariadb" | "doris" | "sqlserver" | "clickhouse" | "sqlite" => {
                if truth {
                    "1"
                } else {
                    "0"
                }
            }
            _ => {
                if truth {
                    "true"
                } else {
                    "false"
                }
            }
        }
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
        if self.key() == "clickhouse" && !Self::is_textual_type(type_name) {
            if upper.contains("CURRENT_TIMESTAMP") || upper.starts_with("NOW(") {
                return Some("now()".to_string());
            }
            if upper.contains("CURRENT_DATE") || upper.starts_with("TODAY(") {
                return Some("today()".to_string());
            }
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
        let expression = upper == "NULL"
            || upper.starts_with('(')
            || trimmed.parse::<f64>().is_ok();
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
        match self.key() {
            "doris" | "sqlserver" => {
                if value { "1" } else { "0" }.to_string()
            }
            _ => {
                if value { "true" } else { "false" }.to_string()
            }
        }
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
        match (self.key(), portable) {
            ("sqlserver", "current_date") => Some("cast(getdate() as date)"),
            // 省略而不是硬塞一个不合法写法：否则整张表建不出来（真机实测语法错）
            ("mysql", "current_date") => None,
            (_, other) => Some(other),
        }
    }

    /// 一张表的列注释：返回 `column_name` / `comment` 两列。
    ///
    /// 内置的列元数据不带注释（不是所有方言都能便宜拿到），所以治理扫描按需取一次。
    /// 拿不到就返回 `Unwritten` —— 上层会退化成「只看字段名」，而不是编一个空注释。
    pub fn table_comments(&self, table: &str) -> Meta {
        let quoted = self.literal(table);
        match self.key() {
            "mysql" | "mariadb" | "doris" => Meta::Sql(format!(
                "select column_name as column_name, coalesce(column_comment, '') as comment \
                 from information_schema.columns \
                 where table_schema = database() and table_name = {quoted}"
            )),
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select a.attname as column_name, coalesce(col_description(a.attrelid, a.attnum), '') as comment \
                 from pg_attribute a \
                 where a.attrelid = {quoted}::regclass and a.attnum > 0 and not a.attisdropped"
            )),
            // 这里**故意**用 `isnull` 而不是 `coalesce`：`sys.extended_properties.value` 是
            // `sql_variant`，`coalesce` 要求两侧类型可统一而直接报错；
            // `isnull` 返回第一个参数的类型，正好能接后面的 cast。别顺手统一掉。
            "sqlserver" => Meta::Sql(format!(
                "select c.name as column_name, cast(isnull(ep.value, '') as nvarchar(400)) as comment \
                 from sys.columns c \
                 left join sys.extended_properties ep on ep.major_id = c.object_id and ep.minor_id = c.column_id \
                 where c.object_id = object_id({quoted})"
            )),
            // `as comment` 在 Oracle 里是**关键字**，不加双引号直接 ORA-00923
            // （整条查询失败 ⇒ 列注释永远为空，看着就是「改完注释没回显」）
            "oracle" | "dm" => Meta::Sql(format!(
                "select column_name as column_name, coalesce(comments, '') as \"comment\" \
                 from user_col_comments where table_name = upper({quoted})"
            )),
            // 表名**双向折大写**再比：H2 把未加引号的标识符存成大写，而带引号建的表
            // （`"my_table"`）就是小写 —— 只写 `table_name = upper(...)` 时后者一行都匹配不上。
            "h2" => Meta::Sql(format!(
                "select column_name as column_name, coalesce(remarks, '') as comment \
                 from information_schema.columns \
                 where upper(table_name) = upper({quoted})"
            )),
            "clickhouse" => Meta::Sql(format!(
                "select name as column_name, coalesce(comment, '') as comment \
                 from system.columns where database = currentDatabase() and table = {quoted}"
            )),
            // 列注释在 **`syscat.columns.remarks`** 里，不是 `syscat.colcomments`
            // —— 后者在本机 DB2 11.5 上直接 `SQLCODE=-204（未定义的对象名）`，
            // 整条查询失败 ⇒ 列注释永远为空（真机实测）。
            // 表名双向折大写：带引号建的小写表名也要能对上。
            "db2" => Meta::Sql(format!(
                "select colname as column_name, coalesce(remarks, '') as comment \
                 from syscat.columns \
                 where tabschema = current schema and upper(tabname) = upper({quoted})"
            )),
            // SQLite / Derby 没有列注释这个概念：给 Absent（空数组）而不是报"未实现"，
            // 语句治理那边会退化成按字段名判断，这是事实，不是缺陷
            "sqlite" | "derby" => Meta::Absent,
            _ => Meta::Unwritten,
        }
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
        let quoted = self.literal(table);
        match self.key() {
            // ClickHouse 没有 information_schema.columns，列类型/注释在 system.columns：
            // type 列是 CH 原生写法（Int32 / Nullable(String) …），parse 侧已能剥 Nullable 壳
            "clickhouse" => Meta::Sql(format!(
                "select name as column_name, type as type_text, \
                 coalesce(comment, '') as comment, \
                 0 as is_auto, \
                 position as ordinal \
                 from system.columns where database = currentDatabase() and table = {quoted}"
            )),
            // MySQL 系直接有完整类型文本（含 enum、unsigned 等），不用自己拼
            "mysql" | "mariadb" | "doris" => Meta::Sql(format!(
                "select column_name as column_name, column_type as type_text, \
                 coalesce(column_comment, '') as comment, \
                 case when extra like '%auto_increment%' then 1 else 0 end as is_auto, \
                 ordinal_position as ordinal \
                 from information_schema.columns \
                 where table_schema = database() and table_name = {quoted}"
            )),
            // PG 用 udt_name（`varchar` / `int4` / `numeric`），比 data_type 的
            // `character varying` 更贴近界面要的短名；长度按「字符」给。
            //
            // 自增有两种写法，都要认：`GENERATED AS IDENTITY`（标准 SQL，PG 10+，
            // information_schema 的 is_identity 直接给）与 `serial`（等价于
            // 默认值取 `nextval('...'::regclass)`）。少认一种，界面就会把自增列显示成普通列。
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select column_name as column_name, \
                 case \
                   when character_maximum_length is not null \
                     then udt_name || '(' || character_maximum_length || ')' \
                   when data_type = 'numeric' and numeric_precision is not null \
                     then udt_name || '(' || numeric_precision || ',' || coalesce(numeric_scale, 0) || ')' \
                   else udt_name \
                 end as type_text, \
                 coalesce((select col_description(c.oid, a.attnum) \
                           from pg_class c join pg_attribute a on a.attrelid = c.oid \
                           where c.relname = table_name and a.attname = column_name limit 1), '') as comment, \
                 case when is_identity = 'YES' or column_default like 'nextval%' then 1 else 0 end as is_auto, \
                 ordinal_position as ordinal \
                 from information_schema.columns \
                 where table_name = {quoted} and table_schema = current_schema()"
            )),
            // SQL Server：character_maximum_length 按**字符**给（nvarchar(50) → 50），
            // `-1` 是 `max`；decimal 走 numeric_precision/scale。
            // 默认 schema 优先排序：同名表出现在多个 schema 时，取当前 schema 那条
            "sqlserver" => Meta::Sql(format!(
                "select column_name as column_name, \
                 case \
                   when character_maximum_length = -1 then data_type + '(max)' \
                   when character_maximum_length is not null \
                     then data_type + '(' + cast(character_maximum_length as varchar(10)) + ')' \
                   when data_type in ('decimal', 'numeric') and numeric_precision is not null \
                     then data_type + '(' + cast(numeric_precision as varchar(10)) + ',' \
                          + cast(coalesce(numeric_scale, 0) as varchar(10)) + ')' \
                   else data_type \
                 end as type_text, \
                 cast(coalesce((select cast(ep.value as nvarchar(400)) from sys.extended_properties ep \
                                where ep.major_id = object_id(table_name) \
                                  and ep.minor_id = columnproperty(object_id(table_name), column_name, 'ColumnId') \
                                  and ep.name = 'MS_Description'), '') as nvarchar(400)) as comment, \
                 cast(coalesce(columnproperty(object_id(table_name), column_name, 'IsIdentity'), 0) as int) as is_auto, \
                 ordinal_position as ordinal, \
                 case when table_schema = schema_name() then 0 else 1 end as schema_rank \
                 from information_schema.columns \
                 where table_name = {quoted} \
                 order by schema_rank, ordinal_position"
            )),
            // Oracle：NUMBER 用 precision/scale，字符类型用 data_length（**字节**，
            // 字节语义下 VARCHAR2(50 CHAR) 会显示成 200 —— 如实反映存储宽度，不做猜测换算）。
            //
            // 自增只看 12c+ 的标识列（`user_tab_identity_cols`）。**不能**用
            // `c.data_default like '%.nextval%'` 认「序列 + 默认值」的老写法：
            // `data_default` 是 LONG，Oracle 对 LONG 做 LIKE 直接
            // `ORA-00932（数据类型为 LONG，与预期 CHAR 不兼容）`，整条查询连同
            // 类型/注释一起失败 —— 症状就是「列注释改完保存成功、界面还是空」
            // （真机 SYSTEM 实测）。少数用序列默认值的老库因此不显示自增，如实接受。
            "oracle" => Meta::Sql(format!(
                "select c.column_name as column_name, \
                 case \
                   when c.data_type = 'NUMBER' and c.data_precision is not null \
                     then 'number(' || c.data_precision || coalesce(',' || c.data_scale, '') || ')' \
                   when c.data_type in ('VARCHAR2', 'NVARCHAR2', 'CHAR', 'NCHAR') \
                     then lower(c.data_type) || '(' || c.data_length || ')' \
                   else lower(c.data_type) \
                 end as type_text, \
                 coalesce((select cc.comments from user_col_comments cc \
                           where cc.table_name = c.table_name and cc.column_name = c.column_name), '') as \"comment\", \
                 case when exists (select 1 from user_tab_identity_cols i \
                                   where i.table_name = c.table_name and i.column_name = c.column_name) then 1 \
                      else 0 end as is_auto, \
                 c.column_id as ordinal \
                 from user_tab_columns c where c.table_name = upper({quoted})"
            )),
            // DM（达梦）：类型 + 注释（与 Oracle 同一套写法 —— 达梦是 Oracle 兼容的，
            // 连表注释那边用的也是 `user_col_comments`，见本文件上方 table_comments）。
            // 注释这一项**曾经漏了**：select 里根本没取 comment，界面上「注释」列却有
            // （supportsComment 为真），于是那一格永远是空的（用户反馈排查出来的同类问题）。
            //
            // **不自增检测**：达梦的标识列（`IDENTITY(1,1)`）记在哪张系统视图上，我没有实例可验证；
            // 而 `user_tab_identity_cols` 在达梦上是否存在同样未验证 —— 一旦那句 SQL 报错，
            // 整张表的类型/注释补全都会一起丢掉（比少一个自增标记更糟）。所以这里留白，
            // 而不是拿一个没验证过的视图去赌。
            "dm" => Meta::Sql(format!(
                "select c.column_name as column_name, \
                 case \
                   when c.data_type = 'NUMBER' and c.data_precision is not null \
                     then 'number(' || c.data_precision || coalesce(',' || c.data_scale, '') || ')' \
                   when c.data_type in ('VARCHAR2', 'NVARCHAR2', 'CHAR', 'NCHAR') \
                     then lower(c.data_type) || '(' || c.data_length || ')' \
                   else lower(c.data_type) \
                 end as type_text, \
                 coalesce((select cc.comments from user_col_comments cc \
                           where cc.table_name = c.table_name \
                             and cc.column_name = c.column_name), '') as \"comment\" \
                 from user_tab_columns c where c.table_name = upper({quoted})"
            )),
            // SQLite：声明类型原样读回（`varchar(50)` 就在表结构里）。
            //
            // 自增的判定标准是「**单列** INTEGER PRIMARY KEY」—— 那是 rowid 的别名，
            // 插入时留空即自动赋值。复合主键里的 INTEGER 不是别名，不能算自增。
            // （`AUTOINCREMENT` 只影响 rowid 复用，不改变「是不是自增」这件事。）
            "sqlite" => Meta::Sql(format!(
                "select name as column_name, type as type_text, '' as comment, \
                 case when upper(trim(coalesce(type, ''))) = 'INTEGER' and pk = 1 \
                       and (select count(*) from pragma_table_info({quoted}) where pk > 0) = 1 \
                      then 1 else 0 end as is_auto, \
                 cid as ordinal \
                 from pragma_table_info({quoted})"
            )),
            // H2：information_schema.columns 有长度/精度，自己拼回 `varchar(50)`。
            // 长度大于 100 万的不当"长度"用（CLOB 一类会报 2^31-1，拼进类型文本没意义）
            // 列注释在 H2 的 information_schema.columns 里叫 `remarks`（老的 1.4 与 2.x 都有），
            // 同样**曾经漏取**：界面「注释」列可见却永远空着。
            //
            // **精度只对 numeric / decimal 拼**：H2 给**所有**数值类型都填了
            // numeric_precision/numeric_scale（INTEGER=32,0、BIGINT=64,0、BOOLEAN=1,0…），
            // 早先不看类型就拼，得到 `integer(32,0)` 这种根本执行不了的"类型"。
            // 更麻烦的是它和界面的「现状基线」对不上：编辑器拆出 `INTEGER` + 长度 32，
            // 再拼回去是 `integer(32)` ≠ `integer(32,0)` ⇒ **一打开表就被判成"改 3"**
            // （用户反馈：什么都没改，预览里却有 3 条 ALTER）。
            "h2" => Meta::Sql(format!(
                "select column_name as column_name, \
                 case \
                   when character_maximum_length is not null and character_maximum_length <= 1000000 \
                     then lower(data_type) || '(' || character_maximum_length || ')' \
                   when lower(data_type) in ('numeric','decimal','dec','number') \
                        and numeric_precision is not null \
                     then lower(data_type) || '(' || numeric_precision || ',' \
                          || coalesce(numeric_scale, 0) || ')' \
                   else lower(data_type) \
                 end as type_text, \
                 coalesce(remarks, '') as comment \
                 from information_schema.columns where upper(table_name) = upper({quoted})"
            )),
            // DB2：syscat.columns 的 typename/length/scale
            // 列注释也要在这里带出来（`remarks`）：表结构页的「注释」列读的是 `/columns`，
            // 只靠 `table_comments`（SQL 结果表头那条）补不上 —— 编辑表结构时注释格永远是空的。
            "db2" => Meta::Sql(format!(
                "select colname as column_name, \
                 case \
                   when rtrim(typename) in ('VARCHAR','CHARACTER','CHAR','VARGRAPHIC','GRAPHIC','BINARY','VARBINARY') \
                     then lower(rtrim(typename)) || '(' || length || ')' \
                   when rtrim(typename) = 'DECIMAL' \
                     then lower(rtrim(typename)) || '(' || length || ',' || scale || ')' \
                   else lower(rtrim(typename)) \
                 end as type_text, \
                 coalesce(remarks, '') as comment \
                 from syscat.columns \
                 where tabschema = current schema and upper(tabname) = upper({quoted})"
            )),
            // Derby：遗留目录 sys.syscolumns 里 COLUMNDATATYPE 本来就是完整类型文本。
            // （现代 Derby 更推荐 SYSCS_UTIL，但那条要存储过程，读元数据不值当。）
            // Derby：`sys.syscolumns` 的 COLUMNDATATYPE 是完整类型文本，但**带着可空性**
            // （实测出来是 `INTEGER NOT NULL`）—— 直接当类型名用会让结构编辑器把它
            // 当成一个类型（改任何属性都会生成 `... c integer not null not null`）。
            // 这里把结尾的 ` NOT NULL` 去掉，只留类型本身。
            "derby" => Meta::Sql(format!(
                "select columnname as column_name, \
                 case when cast(columndatatype as varchar(256)) like '% NOT NULL' \
                   then substr(cast(columndatatype as varchar(256)), 1, \
                               length(cast(columndatatype as varchar(256))) - 9) \
                   else cast(columndatatype as varchar(256)) end as type_text \
                 from sys.syscolumns where referenceid = \
                 (select tableid from sys.systables where tablename = upper({quoted}))"
            )),
            _ => Meta::Unwritten,
        }
    }

    /// 库级统计（表数 + 占用空间）。键名统一成 `tables` / `sizeText`：
    /// 各家能拿到的信息本来就不同（文件型库没有「占用空间」的概念），
    /// 能拿到就给，拿不到如实给 `—`，别编一个 0。
    ///
    /// 空值函数一律用 `coalesce`：`ifnull` 是 MySQL/SQLite 的私有写法
    /// （SQL Server、Oracle、DM 上会直接报「无法识别的函数名」），
    /// `isnull` 是 SQL Server 的私有写法。`coalesce` 是 ANSI 标准，各家都认 —— 少一个方言假设。
    pub fn database_stats(&self) -> Meta {
        match self.key() {
            "mysql" | "mariadb" | "doris" => Meta::Sql(
                "select count(*) as tables, \
                 concat(round(coalesce(sum(data_length + index_length), 0) / 1024 / 1024, 2), ' MB') as sizeText \
                 from information_schema.tables where table_schema = database()"
                    .to_string(),
            ),
            "postgresql" | "kingbase" => Meta::Sql(
                "select (select count(*) from information_schema.tables \
                 where table_schema not in ('pg_catalog','information_schema')) as tables, \
                 pg_size_pretty(pg_database_size(current_database())) as sizeText"
                    .to_string(),
            ),
            "sqlserver" => Meta::Sql(
                "select (select count(*) from sys.tables) as tables, \
                 cast(cast(coalesce(sum(size), 0) * 8.0 / 1024 as decimal(10, 1)) as varchar(20)) + ' MB' as sizeText \
                 from sys.database_files"
                    .to_string(),
            ),
            "sqlite" => Meta::Sql(
                "select (select count(*) from sqlite_master \
                 where type = 'table' and name not like 'sqlite_%') as tables, '—' as sizeText"
                    .to_string(),
            ),
            "clickhouse" => Meta::Sql(
                "select count(*) as tables, formatReadableSize(sum(total_bytes)) as sizeText \
                 from system.tables where database = currentDatabase()"
                    .to_string(),
            ),
            // Oracle / DM：`sum(bytes)` 是聚合，而 `(select count(*) ...)` 是标量子查询，
            // 混在同一个 select 列表里又没 group by ⇒ ORA-00937「不是单组分组函数」（真机实测）。
            // 改成两个互相独立的标量子查询 + from dual。
            "oracle" | "dm" => Meta::Sql(
                "select (select count(*) from user_tables) as tables, \
                 (select to_char(round(coalesce(sum(bytes), 0) / 1024 / 1024, 2)) || ' MB' \
                  from user_segments) as sizeText \
                 from dual"
                    .to_string(),
            ),
            // H2：表数能拿到；占用空间不在 information_schema 里，如实给「—」
            "h2" => Meta::Sql(
                "select count(*) as tables, '—' as sizeText \
                 from information_schema.tables where table_schema = current_schema"
                    .to_string(),
            ),
            // DB2：syscat.tables 只有 NPAGES（数据页）/ FPAGES（总页数，含索引与溢出）/
            // CARD（行数）。原句用的 data_object_pages / index_object_pages 是 Oracle
            // 或其它库的列名，DB2 上直接 SQLCODE=-206 SQLSTATE=42703（真机实测）。
            // 4KB 页 ⇒ 页数 * 4 / 1024 = MB。
            "db2" => Meta::Sql(
                "select count(*) as tables, \
                 cast(coalesce(sum(fpages), 0) * 4 / 1024 as decimal(12,1)) || ' MB' as sizeText \
                 from syscat.tables where tabschema = current schema and type = 'T'"
                    .to_string(),
            ),
            // Derby：能拿到的是表数；占用空间不在系统表里（和 SQLite 一样如实给「—」，
            // 而不是编一个看起来像 0 的数字）
            "derby" => Meta::Sql(
                "select count(*) as tables, '—' as sizeText from sys.systables where tabletype = 'T'"
                    .to_string(),
            ),
            _ => Meta::Unwritten,
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
    /// 整型，按字节数区分（1/2/4/8）
    Int { bytes: u8 },
    /// 定点小数（precision/scale）
    Decimal { precision: u8, scale: u8 },
    /// 浮点
    Real,
    Bool,
    /// 文本：长度可选，`long` 表示"大文本"（text / clob / longtext 这类）
    Text { len: Option<u32>, long: bool },
    Binary,
    Date,
    Time,
    /// 日期时间（`tz` = 带时区）
    DateTime { tz: bool },
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
            Some(_inner) if raw.trim().ends_with(')') => {
                raw.trim().split_once('(').map(|(_, rest)| rest).unwrap_or(raw)
                    .trim_end_matches(')')
                    .trim()
                    .to_string()
            }
            _ => raw.to_string(),
        };
        let lower = raw.trim().to_ascii_lowercase();
        let (head, args) = match lower.split_once('(') {
            Some((head, rest)) => (
                head.trim().to_string(),
                rest.trim_end_matches(')')
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
        match base.as_str() {
            // MySQL 的 tinyint(1) 是"布尔"的通行约定，跨类型时按布尔走更贴合语义
            "tinyint" | "int1" => {
                if first == Some(1) {
                    Self::Bool
                } else {
                    Self::Int { bytes: 1 }
                }
            }
            "smallint" | "int2" => Self::Int { bytes: 2 },
            "mediumint" | "int3" => Self::Int { bytes: 4 },
            "int" | "integer" | "int4" | "serial" => Self::Int { bytes: 4 },
            "bigint" | "int8" | "bigserial" => Self::Int { bytes: 8 },
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
                Self::Text { len: first, long: false }
            }
            "text" | "tinytext" | "mediumtext" | "longtext" | "clob" | "nclob" | "longvarchar" => {
                Self::Text { len: None, long: true }
            }
            "binary" | "varbinary" | "blob" | "tinyblob" | "mediumblob" | "longblob" | "bytea"
            | "image" | "raw" => Self::Binary,
            "date" => Self::Date,
            "time" => Self::Time,
            "datetime" | "datetime2" | "smalldatetime" => Self::DateTime { tz: false },
            "timestamp" | "timestamptz" | "datetimeoffset" => Self::DateTime {
                tz: lower.contains("tz")
                    || lower.contains("offset")
                    || lower.contains("with time zone"),
            },
            "json" | "jsonb" => Self::Json,
            "uuid" | "uniqueidentifier" => Self::Uuid,
            // ClickHouse 的大写族（`UInt64` / `String` / `DateTime64` / `FixedString(n)` / `Decimal64(s)`）
            "string" => Self::Text { len: None, long: true },
            "fixedstring" => Self::Text { len: first, long: false },
            "datetime64" => Self::DateTime { tz: false },
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
            // 无符号整型：映射到有符号的"大一档"，超范围的一律落到定点，避免溢出
            "uint8" => Self::Int { bytes: 2 },
            "uint16" => Self::Int { bytes: 4 },
            "uint32" => Self::Int { bytes: 8 },
            "uint64" => Self::Decimal { precision: 20, scale: 0 },
            // ClickHouse 的整型/浮点族（Int32 / Float64 …）：之前没认，落到 Unknown，
            // 目标 Derby/H2 这类按 Unknown→CLOB 渲染的方言，整列全成 CLOB（真机踩过）
            "int16" => Self::Int { bytes: 2 },
            "int32" => Self::Int { bytes: 4 },
            "int64" => Self::Int { bytes: 8 },
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
        if key == "doris" {
            return match self {
                Self::Bool => "BOOLEAN".to_string(),
                Self::Uuid => "VARCHAR(36)".to_string(),
                Self::Json => "STRING".to_string(),
                Self::Binary => "STRING".to_string(),
                Self::Time => "STRING".to_string(),
                Self::Unknown => "STRING".to_string(),
                Self::Text { len, long } => match len {
                    Some(size) if !long => format!("VARCHAR({})", size.min(65533)),
                    _ => "STRING".to_string(),
                },
                other => other.render(ConnectionKind::Mysql),
            };
        }
        match self {
            Self::Int { bytes } => match key {
                "mysql" | "mariadb" | "doris" => match bytes {
                    1 => "tinyint".to_string(),
                    2 => "smallint".to_string(),
                    4 => "int".to_string(),
                    _ => "bigint".to_string(),
                },
                "oracle" | "dm" => match bytes {
                    1 => "number(3)".to_string(),
                    2 => "number(5)".to_string(),
                    4 => "number(10)".to_string(),
                    _ => "number(19)".to_string(),
                },
                "clickhouse" => match bytes {
                    1 => "Int8".to_string(),
                    2 => "Int16".to_string(),
                    4 => "Int32".to_string(),
                    _ => "Int64".to_string(),
                },
                "postgresql" | "kingbase" => match bytes {
                    1 | 2 => "smallint".to_string(),
                    4 => "integer".to_string(),
                    _ => "bigint".to_string(),
                },
                "sqlite" => "integer".to_string(),
                _ => match bytes {
                    1 => "tinyint".to_string(),
                    2 => "smallint".to_string(),
                    4 => "int".to_string(),
                    _ => "bigint".to_string(),
                },
            },
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
            Self::Time => match key {
                // Oracle 没有纯时间类型，用 date（它本身含时分秒）
                "oracle" | "dm" => "date".to_string(),
                "clickhouse" => "String".to_string(),
                _ => "time".to_string(),
            },
            Self::DateTime { tz } => match key {
                "mysql" | "mariadb" | "doris" => "datetime".to_string(),
                "postgresql" | "kingbase" => {
                    if tz {
                        "timestamptz".to_string()
                    } else {
                        "timestamp".to_string()
                    }
                }
                "oracle" | "dm" => {
                    if tz {
                        "timestamp with time zone".to_string()
                    } else {
                        "timestamp".to_string()
                    }
                }
                "sqlserver" => {
                    if tz {
                        "datetimeoffset".to_string()
                    } else {
                        "datetime2".to_string()
                    }
                }
                "clickhouse" => "DateTime".to_string(),
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
}

/// 自增列的写法：按目标方言各写一套；认不出的方言返回 `None`
/// （宁可不自增，也不要造出一句语法错）。
fn auto_increment_clause(kind: ConnectionKind, family: TypeFamily) -> Option<&'static str> {
    // 只有整数（或 scale=0 的定点）才谈得上自增
    let integer = matches!(family, TypeFamily::Int { .. })
        || matches!(family, TypeFamily::Decimal { scale: 0, .. });
    if !integer {
        return None;
    }
    match kind.key() {
        "mysql" | "mariadb" => Some("auto_increment"),
        // Doris 是 2.1 才支持 AUTO_INCREMENT，还有"必须是键列 + 开关"等前置条件 ——
        // 写上去风险大于收益（老版本直接语法错），不写。
        "sqlserver" => Some("identity(1,1)"),
        "postgresql" | "kingbase" | "oracle" | "dm" | "h2" | "db2" | "derby" => {
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
    let base = lower
        .split(['(', ' '])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
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
        let mut cols = vec![
            col("id", "varchar(64)", None),
            col("name", "varchar(255)", None),
        ];
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
        assert_eq!(pg.default_literal("b'0'", "boolean", true).as_deref(), Some("false"));
        assert_eq!(pg.default_literal("'1'", "boolean", true).as_deref(), Some("true"));
        // 同类型保持原样（MySQL 的 tinyint 语义，不能被翻成 false/true）
        assert_eq!(pg.default_literal("0", "boolean", false).as_deref(), Some("0"));
        // 非布尔列不受影响
        assert_eq!(pg.default_literal("0", "integer", true).as_deref(), Some("0"));
        // Doris / SQL Server 反过来**不认 false/true**（真机实测：`boolean default false`
        // 在 Doris 上是语法错、`bit default false` 在 SQL Server 上被拒），只收 0/1
        let doris = Dialect::new(ConnectionKind::Doris);
        assert_eq!(doris.default_literal("false", "BOOLEAN", true).as_deref(), Some("0"));
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
        assert!(ddl.contains("createtime Nullable(DateTime) default now()"), "实际：{ddl}");
        assert!(ddl.contains("updatetime Nullable(DateTime) default now()"), "实际：{ddl}");
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
        assert!(sqlserver.contains("default cast(getdate() as date)"), "实际：{sqlserver}");

        // MySQL：列默认值只支持字面量与 CURRENT_TIMESTAMP，DATE 列的 current_date 表达不了
        let mysql = Dialect::new(ConnectionKind::Mysql).create_table_from_columns(
            "t",
            &cols,
            Some(ConnectionKind::Postgresql),
        );
        assert!(!mysql.to_ascii_lowercase().contains("current_date"), "实际：{mysql}");
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
        assert_eq!(mysql.map_type(Some(ConnectionKind::Mysql), "datetime"), "datetime");
        assert_eq!(mysql.map_type(None, "tinyint(1)"), "tinyint(1)");
        // MySQL → Oracle：下面这些类型 Oracle 一个都不认
        assert_eq!(oracle.map_type(Some(ConnectionKind::Mysql), "datetime"), "timestamp");
        assert_eq!(oracle.map_type(Some(ConnectionKind::Mysql), "text"), "clob");
        assert_eq!(oracle.map_type(Some(ConnectionKind::Mysql), "varchar(100)"), "varchar2(100)");
        assert_eq!(oracle.map_type(Some(ConnectionKind::Mysql), "bigint"), "number(19)");
        assert_eq!(oracle.map_type(Some(ConnectionKind::Mysql), "tinyint(1)"), "number(1)");
        assert_eq!(oracle.map_type(Some(ConnectionKind::Mysql), "longblob"), "blob");
        // PG → MySQL：`character varying` / `int4` / `timestamptz` / `bytea` 都不是 MySQL 类型
        assert_eq!(mysql.map_type(Some(ConnectionKind::Postgresql), "character varying(50)"), "varchar(50)");
        assert_eq!(mysql.map_type(Some(ConnectionKind::Postgresql), "int4"), "int");
        assert_eq!(mysql.map_type(Some(ConnectionKind::Postgresql), "timestamptz"), "datetime");
        assert_eq!(mysql.map_type(Some(ConnectionKind::Postgresql), "bytea"), "longblob");
        assert_eq!(mysql.map_type(Some(ConnectionKind::Postgresql), "boolean"), "tinyint(1)");
        // ClickHouse → PG
        assert_eq!(pg.map_type(Some(ConnectionKind::Clickhouse), "UInt64"), "numeric(20,0)");
        assert_eq!(pg.map_type(Some(ConnectionKind::Clickhouse), "String"), "text");
        // ClickHouse → MySQL：String 该落 longtext（不是大 varchar）；`DecimalNN(s)` 括号里是 scale、
        // 精度由位数定 —— 曾经一律按 38 位算，`Decimal64(2)` 被写成 `decimal(38,2)`，每列内联从
        // 9 字节涨到 17 字节，宽表同步时就是「Row size too large (> 8126)」的帮凶（真机踩过）
        assert_eq!(mysql.map_type(Some(ConnectionKind::Clickhouse), "String"), "longtext");
        assert_eq!(mysql.map_type(Some(ConnectionKind::Clickhouse), "Nullable(String)"), "longtext");
        assert_eq!(mysql.map_type(Some(ConnectionKind::Clickhouse), "Decimal32(2)"), "decimal(9,2)");
        assert_eq!(mysql.map_type(Some(ConnectionKind::Clickhouse), "Decimal64(2)"), "decimal(18,2)");
        assert_eq!(mysql.map_type(Some(ConnectionKind::Clickhouse), "Decimal128(4)"), "decimal(38,4)");
        // 76 位超出 MySQL 的上限 65：必须夹住，否则建表直接报「Too big precision」
        assert_eq!(mysql.map_type(Some(ConnectionKind::Clickhouse), "Decimal256(4)"), "decimal(65,4)");
        // 建表必须显式 row_format=dynamic：老服务器默认 COMPACT，长文本列每列内联 768 字节前缀，
        // 十几个 TEXT 列就撞上 8126 行上限
        let mysql_ddl = mysql.create_table_from_columns("t", &[col("a", "String", None)], None);
        assert!(
            mysql_ddl.to_ascii_lowercase().contains("row_format=dynamic"),
            "实际：{mysql_ddl}"
        );
        // 认不出的族降级为文本：宁可能存下，也不要建表就语法错
        assert_eq!(mysql.map_type(Some(ConnectionKind::Postgresql), "geometry"), "longtext");
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
        assert!(ddl.contains("CREATED timestamp default current_timestamp"), "实际：{ddl}");
        // 认不出的函数默认值跨类型时**省略**（`nextval` 各家写法完全不同，带过去就是语法错）
        assert!(!ddl.contains("nextval"), "不可移植的默认值应被省略：{ddl}");
        assert!(
            ddl.lines().any(|line| line.contains("SEQ") && !line.contains("default")),
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
            &[ColumnDetail { primary_key: true, ..col("MAILID", "string", None) }, col("V", "int", None)],
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
            "mysql", "mariadb", "doris", "postgresql", "kingbase", "sqlserver", "oracle", "dm",
            "db2", "h2", "derby", "clickhouse", "sqlite",
        ] {
            let Some(kind) = ConnectionKind::from_key(key) else {
                continue;
            };
            checked += 1;
            let dialect = Dialect::new(kind);
            let ddl = dialect.create_table_from_columns("t", &columns, Some(ConnectionKind::Mysql));
            let lower = ddl.to_lowercase();
            // ① 源方言的类型名不能漏出去（漏了就说明翻译没生效）；MySQL 家族除外
            if !matches!(key, "mysql" | "mariadb") {
                assert!(!lower.contains("longtext"), "{key}: 类型没翻译：{ddl}");
            }
            // ② 各家的硬性约束
            match key {
                "doris" => {
                    assert!(ddl.contains("UNIQUE KEY("), "{key}: 缺 UNIQUE KEY：{ddl}");
                    assert!(ddl.contains("DISTRIBUTED BY HASH("), "{key}: 缺 DISTRIBUTED：{ddl}");
                    assert!(ddl.contains("replication_num"), "{key}: 缺 replication_num：{ddl}");
                    assert!(!lower.contains("primary key"), "{key}: 不该有表级主键：{ddl}");
                    assert!(!lower.contains("auto_increment"), "{key}: 不写自增：{ddl}");
                }
                "clickhouse" => {
                    assert!(ddl.contains("ENGINE = MergeTree()"), "{key}: 缺引擎：{ddl}");
                    assert!(ddl.contains("ORDER BY"), "{key}: 缺 ORDER BY：{ddl}");
                    assert!(!lower.contains("primary key"), "{key}: primary key 不能在列清单里：{ddl}");
                    assert!(!lower.contains("auto_increment"), "{key}: 没有自增：{ddl}");
                }
                "postgresql" | "kingbase" | "oracle" | "dm" | "h2" | "db2" | "derby" => {
                    assert!(
                        ddl.contains("generated by default as identity"),
                        "{key}: 自增没跟上：{ddl}"
                    );
                }
                "sqlserver" => assert!(ddl.contains("identity(1,1)"), "{key}: 自增没跟上：{ddl}"),
                "mysql" | "mariadb" => {
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
                assert!(update.starts_with("alter table"), "{key}: 要走 ALTER UPDATE：{update}");
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
            &[col("UPDATED", "datetime", Some("CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP"))],
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
        assert_eq!(q(ConnectionKind::Postgresql, "public.user"), "\"public\".\"user\"");
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
        assert!(sql.starts_with("merge into t tgt using (select 1 as id, 'a' as name, 10 as amount from dual union all "));
        assert!(sql.contains("on (tgt.id = src.id)"));
        assert!(sql.contains("when matched then update set name = src.name, amount = src.amount"));
        assert!(sql.ends_with("when not matched then insert (id, name, amount) values (src.id, src.name, src.amount)"));
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

    #[test]
    fn oracle与dm原生upsert必须限制单语句行数() {
        assert_eq!(Dialect::new(ConnectionKind::Oracle).upsert_row_limit(), 500);
        assert_eq!(Dialect::new(ConnectionKind::Dm).upsert_row_limit(), 500);
        assert_eq!(Dialect::new(ConnectionKind::Sqlserver).upsert_row_limit(), 1000);
        assert_eq!(Dialect::new(ConnectionKind::Postgresql).upsert_row_limit(), usize::MAX);
    }

    #[test]
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
        assert!(sql.starts_with("merge into t as tgt using (values "), "实际：{sql}");
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
        assert_eq!(sql, "insert into t (id, code) values (1, 'x') on conflict (id, code) do nothing");
        // Oracle 全键时没有 when matched 子句，但插入分支仍然在
        let ora = Dialect::new(ConnectionKind::Oracle)
            .upsert_sql("t", &["id".to_string()], &[0], &[vec!["1".to_string()]])
            .expect("oracle 必须支持");
        assert!(!ora.contains("when matched"));
        assert!(ora.contains("when not matched then insert (id) values (src.id)"));
    }
}
