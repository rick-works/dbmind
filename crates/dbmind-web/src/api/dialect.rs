//! 方言层：**按数据库类型生成元数据 SQL**。
//!
//! 为什么这一层存在：DBMind 内核只提供两个元数据能力（`list_tables` / `list_columns`，
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
#[derive(Default)]
pub struct UserPrivileges {
    /// 全局：`grant <这些> on *.* to ...`
    pub global: Vec<String>,
    /// 库级：`grant <这些> on \`db\`.* to ...`
    pub databases: Vec<(String, Vec<String>)>,
    /// 表级：`grant <这些> on \`db\`.\`tbl\` to ...`
    pub tables: Vec<(String, String, Vec<String>)>,
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
            if plain && !is_reserved(s) {
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
        matches!(self.key(), "postgresql" | "kingbase" | "sqlserver")
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
            "select table_name as table_name, engine as engine, \
             table_collation as charset, coalesce(table_comment, '') as comment \
             from {catalog}.information_schema.tables where table_schema = {}",
            self.literal(database)
        ))
    }

    /// schema 清单（只有 schema 层级的类型需要）。
    pub fn schemas(&self) -> Meta {
        match self.key() {
            "postgresql" | "kingbase" => Meta::Sql(
                "select nspname as name from pg_namespace \
                 where nspname not like 'pg\\_%' and nspname <> 'information_schema' order by nspname"
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
            _ => Meta::Absent,
        }
    }

    /// 某个 schema 下的表/视图（schema 层级专用；非 schema 类型走内核的 DatabaseMetaData）。
    pub fn tables_in_schema(&self, schema: &str) -> Meta {
        match self.key() {
            // PostgreSQL / KingbaseES：`reltuples` 是**估算值**（未 ANALYZE 时是 -1），
            // 拿不到就给 null —— 界面上留空，不要用 0 冒充「空表」。
            // 精确行数由前端展开后的批量 COUNT(*) 回填（见 `/table-count`）。
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select c.relname as name, \
                 case when c.relkind = 'v' then 'VIEW' else 'BASE TABLE' end as type, \
                 case when c.reltuples < 0 then null else c.reltuples::bigint end as rows \
                 from pg_class c join pg_namespace n on n.oid = c.relnamespace \
                 where n.nspname = {} and c.relkind in ('r','p','v','f') order by c.relname",
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
            // SQL Server：`sys.database_principals` 是库内主体，权限在 `sys.database_permissions`
            "sqlserver" => Meta::Sql(format!(
                // 别名必须是 `type_desc`（不是 `type`）：前端标签表按 `type_desc` 认「类型」，
                // 对不上就退回显示原始键名 —— 用户看到的是「TYPE  SQL_USER」这种半英文
                "select dp.name as name, dp.type_desc as type_desc, \
                 isnull(sp.name, '') as login_name, \
                 convert(varchar(30), dp.create_date, 120) as create_date, \
                 isnull(dp.default_schema_name, '') as default_schema, \
                 cast(coalesce(sp.is_disabled, 0) as int) as disabled, \
                 'create user ' + quotename(dp.name) + \
                   case when sp.name is null then '' else ' for login ' + quotename(sp.name) end as create_user_sql, \
                 isnull(stuff(( \
                   select char(10) + 'grant ' + p.permission_name + ' on ' + \
                     case when p.class_desc = 'DATABASE' then 'database::' + db_name() \
                          else quotename(object_schema_name(p.major_id)) + '.' + quotename(object_name(p.major_id)) end + \
                     ' to ' + quotename(dp2.name) + ';' \
                   from sys.database_permissions p \
                   join sys.database_principals dp2 on dp2.principal_id = p.grantee_principal_id \
                   where p.grantee_principal_id = dp.principal_id \
                   for xml path(''), type).value('.', 'nvarchar(max)'), 1, 1, ''), '') + \
                 isnull(( \
                   select char(10) + 'alter role ' + quotename(r.name) + ' add member ' + quotename(dp3.name) + ';' \
                   from sys.database_role_members m \
                   join sys.database_principals r on r.principal_id = m.role_principal_id \
                   join sys.database_principals dp3 on dp3.principal_id = m.member_principal_id \
                   where m.member_principal_id = dp.principal_id \
                   for xml path(''), type).value('.', 'nvarchar(max)'), '') as grants_text \
                 from sys.database_principals dp \
                 left join sys.server_principals sp on sp.sid = dp.sid \
                 where dp.name = {n} and dp.type in ('S', 'U', 'G')"
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
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select r.rolname as name, \
                 cast(r.rolsuper as int) as is_superuser, \
                 cast(r.rolcanlogin as int) as can_login, \
                 cast(r.rolcreatedb as int) as can_create_db, \
                 'create role ' + quote_ident(r.rolname) + ' login' as create_user_sql, \
                 isnull(string_agg(case when a.privilege_type is not null then \
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
                "select name, storage, \
                 arrayStringConcat(auth_type, ', ') as auth_type, \
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
        let grantee = format!(
            "concat(quote({}), '@', quote({}))",
            self.literal(user),
            self.literal(host)
        );
        match self.key() {
            "mysql" | "mariadb" | "doris" => Meta::Sql(format!(
                "select 'global' as scope, privilege_type as privilege, '' as db_name, '' as table_name \
                   from information_schema.user_privileges where grantee = {grantee} \
                 union all \
                 select 'schema', privilege_type, table_schema, '' \
                   from information_schema.schema_privileges where grantee = {grantee} \
                 union all \
                 select 'table', privilege_type, table_schema, table_name \
                   from information_schema.table_privileges where grantee = {grantee} \
                 union all \
                 select distinct 'global', 'GRANT OPTION', '', '' \
                   from information_schema.user_privileges \
                  where grantee = {grantee} and is_grantable = 'YES' \
                 union all \
                 select distinct 'schema', 'GRANT OPTION', table_schema, '' \
                   from information_schema.schema_privileges \
                  where grantee = {grantee} and is_grantable = 'YES' \
                 union all \
                 select distinct 'table', 'GRANT OPTION', table_schema, table_name \
                   from information_schema.table_privileges \
                  where grantee = {grantee} and is_grantable = 'YES'"
            )),
            // ClickHouse：`show grants for` 每行就是一条可执行的 GRANT 语句，
            // 由 grant_line 的 clickhouse 分支**原样**带回 —— 不做任何解析重组。
            "clickhouse" => {
                Meta::Sql(format!("show grants for {}", self.quote(user)))
            }
            _ => Meta::Unwritten,
        }
    }

/// 用户新建 / 改密 / 删除（**多条语句用 `;\n` 连接**，由调用方拆分逐条执行）。
    ///
    /// 只生成有把握的那几件事。**细粒度权限（库/表/模式/对象级 GRANT）不在这里做**：
    /// 那是一片权限矩阵，拼错就是「授多了」或「授错了」——调用方会在收到这类入参时明确拒绝，
    /// 而不是默默建好用户却什么都没授（界面成功提示会把"什么都没做"说成成功）。
    pub fn user_action_sql(
        &self,
        action: &str,
        user: &str,
        password: &str,
        host: &str,
        roles: &[String],
        privileges: &UserPrivileges,
        reset_privileges: bool,
    ) -> Meta {
        let u = self.quote(user);
        let pwd = self.literal(password);
        let host = if host.trim().is_empty() { "%" } else { host };
        let h = self.literal(host);
        let role_parts: Vec<String> = roles.iter().map(|role| self.quote(role)).collect();
        match self.key() {
            "sqlserver" => Meta::Sql(match action {
                // 不带口令 = 建「映射到登录名」的库用户；带口令 = 建包含数据库用户（contained）
                "create" => {
                    let mut sql = if password.is_empty() {
                        format!("create user {u}")
                    } else {
                        format!("create user {u} with password = {pwd}")
                    };
                    for role in &role_parts {
                        sql.push_str(&format!(";\nalter role {role} add member {u}"));
                    }
                    sql
                }
                "alter" => {
                    let mut parts: Vec<String> = Vec::new();
                    if !password.is_empty() {
                        parts.push(format!("alter user {u} with password = {pwd}"));
                    }
                    for role in &role_parts {
                        parts.push(format!("alter role {role} add member {u}"));
                    }
                    if parts.is_empty() {
                        return Meta::Unwritten;
                    }
                    parts.join(";\n")
                }
                "drop" => format!("drop user {u}"),
                _ => return Meta::Unwritten,
            }),
            // MySQL 的用户是 (user, host) 二元组 —— 少了 host 就会建错主体
            "mysql" | "mariadb" | "doris" => {
                let full = format!("{u}@{h}");
                let mut parts: Vec<String> = Vec::new();
                match action {
                    "create" => {
                        let mut create = format!("create user {full}");
                        if !password.is_empty() {
                            create.push_str(&format!(" identified by {pwd}"));
                        }
                        parts.push(create);
                    }
                    "alter" => {
                        // 只改权限、不改密码是常见操作 ⇒ ALTER 只在给了密码时才生成。
                        // （以前这里没密码就返回 Unwritten，等于「改权限」这条路根本走不通）
                        if !password.is_empty() {
                            parts.push(format!("alter user {full} identified by {pwd}"));
                        }
                    }
                    "drop" => return Meta::Sql(format!("drop user {full}")),
                    _ => return Meta::Unwritten,
                }
                // 权限**以表单为准**：先清空该用户的授权，再按表单重授。
                // 只 grant 不 revoke 的话，「取消勾选」只是看起来取消了 —— 库里权限还在。
                //
                // 只用这一条合体形式，不逐目标 `revoke ... on \`db\`.*`：
                // 实测这条 `revoke all privileges, grant option from <用户>` **连库级/表级
                // 授权一起清掉**（清完再看只剩 USAGE），而逐目标 revoke 反而会在
                // 「该目标本来就没授权」时报 1141（`There is no such grant defined`）。
                if reset_privileges {
                    parts.push(format!("revoke all privileges, grant option from {full}"));
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
                        "grant {} on {}.* to {full}{}",
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
                        "grant {} on {}.{} to {full}{}",
                        items.join(", "),
                        self.quote(database),
                        self.quote(table),
                        tail(with_option)
                    ));
                }
                let (global_items, global_option) = split_grant_option(&privileges.global);
                if !global_items.is_empty() {
                    parts.push(format!(
                        "grant {} on *.* to {full}{}",
                        global_items.join(", "),
                        tail(global_option)
                    ));
                }
                Meta::Sql(parts.join(";\n"))
            }
            // PG 的「用户」就是带 login 的角色（MySQL 语境的 host 在这里没有意义，忽略）
            "postgresql" | "kingbase" => Meta::Sql(match action {
                "create" => {
                    let mut sql = format!("create role {u} login");
                    if !password.is_empty() {
                        sql.push_str(&format!(" password {pwd}"));
                    }
                    for role in &role_parts {
                        sql.push_str(&format!(";\ngrant {role} to {u}"));
                    }
                    sql
                }
                "alter" => {
                    let mut parts: Vec<String> = Vec::new();
                    if !password.is_empty() {
                        parts.push(format!("alter role {u} password {pwd}"));
                    }
                    for role in &role_parts {
                        parts.push(format!("grant {role} to {u}"));
                    }
                    if parts.is_empty() {
                        return Meta::Unwritten;
                    }
                    parts.join(";\n")
                }
                "drop" => format!("drop role {u}"),
                _ => return Meta::Unwritten,
            }),
            // ClickHouse：用户没有 (user, host) 二元组，host 参数没有意义（忽略）。
            // 细粒度授权同样不在这里做 —— 调用方对非白名单入参会明确拒绝。
            "clickhouse" => Meta::Sql(match action {
                "create" => {
                    let mut sql = format!("create user {u}");
                    if !password.is_empty() {
                        sql.push_str(&format!(" identified by {pwd}"));
                    }
                    for role in &role_parts {
                        sql.push_str(&format!(";\ngrant {role} to {u}"));
                    }
                    sql
                }
                "alter" => {
                    let mut parts: Vec<String> = Vec::new();
                    if !password.is_empty() {
                        parts.push(format!("alter user {u} identified by {pwd}"));
                    }
                    for role in &role_parts {
                        parts.push(format!("grant {role} to {u}"));
                    }
                    if parts.is_empty() {
                        return Meta::Unwritten;
                    }
                    parts.join(";\n")
                }
                "drop" => format!("drop user {u}"),
                _ => return Meta::Unwritten,
            }),
            _ => Meta::Unwritten,
        }
    }

    /// 表/视图的建表语句。
    pub fn ddl(&self, table: &str) -> Meta {
        let t = self.quote(table);
        let lit = self.literal(table);
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
                   and c.relnamespace = (select oid from pg_namespace where nspname = current_schema()) \
                 group by c.relname"
            )),
            // Oracle：`dbms_metadata.get_ddl` 就是官方给完整 DDL 的入口（含存储属性），
            // 比手拼可靠；按存在性自动选 TABLE / VIEW。DM 走手工拼列（它的 dbms_metadata 支持面不确定）
            "oracle" => Meta::Sql(format!(
                "select dbms_metadata.get_ddl( \
                   case when exists (select 1 from user_views where view_name = upper({lit})) \
                        then 'VIEW' else 'TABLE' end, \
                   upper({lit})) as ddl from dual"
            )),
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
                "select table_name as table_name, engine as engine, \
                 table_collation as charset, coalesce(table_comment, '') as comment \
                 from information_schema.tables where table_schema = {db}"
            )),
            // ClickHouse：`system.tables` 一次把引擎、排序键、分区键、注释都给全
            // —— 这三项正是它「建表时确定、之后改不了」的部分，所以界面只读展示。
            "clickhouse" => Meta::Sql(format!(
                "select name as table_name, engine as engine, \
                 sorting_key as sortingKey, partition_key as partitionKey, \
                 coalesce(comment, '') as comment \
                 from system.tables where database = {db}"
            )),
            // PG / Kingbase：没有引擎与表级字符集，只有表注释（同 `table_comments` 的写法）
            "postgresql" | "kingbase" => Meta::Sql(format!(
                "select c.relname as table_name, \
                 coalesce(obj_description(c.oid, 'pg_class'), '') as comment \
                 from pg_class c join pg_namespace n on n.oid = c.relnamespace \
                 where n.nspname = {db} and c.relkind = 'r'"
            )),
            // Oracle / DM：`user_tab_comments` 就是当前 schema 的表注释
            "oracle" | "dm" => Meta::Sql(
                "select table_name as table_name, coalesce(comments, '') as comment \
                 from user_tab_comments where table_type = 'TABLE'"
                    .to_string(),
            ),
            // SQL Server：表注释存在扩展属性 `MS_Description`（与列注释同一处）
            "sqlserver" => Meta::Sql(
                "select t.name as table_name, \
                 cast(ep.value as nvarchar(4000)) as comment \
                 from sys.tables t \
                 left join sys.extended_properties ep \
                   on ep.major_id = t.object_id and ep.minor_id = 0 and ep.name = 'MS_Description'"
                    .to_string(),
            ),
            // H2 / DB2 的 information_schema 都带 remarks（就是表注释）
            "h2" => Meta::Sql(
                "select table_name as table_name, coalesce(remarks, '') as comment \
                 from information_schema.tables where table_schema = 'PUBLIC'"
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
                 group_concat(column_name order by seq_in_index separator ',') as `columns` \
                 from information_schema.statistics where table_schema = database() \
                 group by table_name, index_name, non_unique order by table_name, index_name"
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
                "select indexname as name, tablename as \"table\", indexdef \
                 from pg_indexes where schemaname not in ('pg_catalog','information_schema') \
                 order by tablename, indexname"
                    .to_string(),
            ),
            "sqlserver" => Meta::Sql(
                "select i.name as name, o.name as \"table\", i.is_unique as isUnique \
                 from sys.indexes i join sys.objects o on o.object_id = i.object_id \
                 where o.type = 'U' and i.name is not null order by o.name, i.name"
                    .to_string(),
            ),
            "oracle" | "dm" => Meta::Sql(
                "select index_name as \"name\", table_name as \"table\" from user_indexes \
                 order by table_name, index_name"
                    .to_string(),
            ),
            "db2" => Meta::Sql(
                "select indexname as \"name\", tabname as \"table\" from syscat.indexes \
                 where tabschema = current schema order by tabname, indexname"
                    .to_string(),
            ),
            // Derby **没有 syscat schema**（那是 DB2 的）：老写法在这里报
            // `Table/View 'SYSCAT.INDEXES' does not exist`（实测）。
            // 索引在 sys.sysconglomerates（isindex 标记），表名要连 sys.systables，
            // 且必须按 schema 过滤 —— 不然会把系统索引（SYSCONGLOMERATES_INDEX1 之类）全列出来。
            "derby" => Meta::Sql(
                "select c.conglomeratename as \"name\", t.tablename as \"table\" \
                 from sys.sysconglomerates c \
                 join sys.systables t on t.tableid = c.tableid \
                 join sys.sysschemas s on s.schemaid = t.schemaid \
                 where c.isindex = true and s.schemaname = current schema \
                 order by t.tablename, c.conglomeratename"
                    .to_string(),
            ),
            "h2" => Meta::Sql(
                "select index_name as \"name\", table_name as \"table\" from information_schema.indexes \
                 order by table_name, index_name"
                    .to_string(),
            ),
            "clickhouse" => Meta::Sql(
                "select name, table, type from system.data_skipping_indices order by table, name"
                    .to_string(),
            ),
            _ => Meta::Unwritten,
        }
    }

    pub fn procedures(&self) -> Meta {
        match self.key() {
            // SQLite 没有存储过程这个概念 —— 空数组是事实。
            "sqlite" => Meta::Absent,
            // Doris 也没有存储过程（它只有 catalog 级的 UDF，不在库的 routines 里）。
            // 之前跟着 MySQL 一起来了，于是树上画出「存储过程 / 函数」两个分类、点进去恒为空。
            "doris" => Meta::Absent,
            "mysql" | "mariadb" => Meta::Sql(
                "select routine_name as `name`, routine_type as routineType \
                 from information_schema.routines where routine_schema = database() \
                 order by routine_name"
                    .to_string(),
            ),
            "postgresql" | "kingbase" => Meta::Sql(
                "select p.proname as name, \
                 case when p.prokind = 'f' then 'FUNCTION' else 'PROCEDURE' end as routineType \
                 from pg_proc p join pg_namespace n on n.oid = p.pronamespace \
                 where n.nspname not in ('pg_catalog','information_schema') order by p.proname"
                    .to_string(),
            ),
            "sqlserver" => Meta::Sql(
                "select name, 'PROCEDURE' as routineType from sys.procedures \
                 union all select name, 'FUNCTION' as routineType from sys.objects \
                 where type in ('FN','IF','TF') order by name"
                    .to_string(),
            ),
            "oracle" | "dm" => Meta::Sql(
                "select object_name as \"name\", object_type as \"routineType\" from user_objects \
                 where object_type in ('PROCEDURE','FUNCTION') order by object_name"
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

    pub fn triggers(&self) -> Meta {
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
            "postgresql" | "kingbase" => Meta::Sql(
                "select t.tgname as name, c.relname as \"table\" from pg_trigger t \
                 join pg_class c on c.oid = t.tgrelid where not t.tgisinternal order by t.tgname"
                    .to_string(),
            ),
            "sqlserver" => Meta::Sql(
                "select t.name as name, o.name as \"table\" from sys.triggers t \
                 left join sys.objects o on o.object_id = t.parent_id order by t.name"
                    .to_string(),
            ),
            "oracle" | "dm" => Meta::Sql(
                "select trigger_name as \"name\", table_name as \"table\" from user_triggers \
                 order by trigger_name"
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
            "derby" => Meta::Sql(
                "select triggername as \"name\" from sys.systriggers order by triggername".to_string(),
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
                "select event_name as `name`, status, interval_value, interval_field \
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
            "mysql" | "mariadb" | "doris" => Meta::Sql(
                "select user as `name`, host from mysql.user order by user, host".to_string(),
            ),
            "postgresql" | "kingbase" => Meta::Sql(
                "select rolname as name, rolsuper as isSuper from pg_roles order by rolname"
                    .to_string(),
            ),
            "sqlserver" => Meta::Sql(
                "select name from sys.database_principals where type in ('S','U') order by name"
                    .to_string(),
            ),
            "oracle" | "dm" => {
                Meta::Sql("select username as \"name\" from all_users order by username".to_string())
            }
            "clickhouse" => {
                Meta::Sql("select name from system.users order by name".to_string())
            }
            _ => Meta::Absent,
        }
    }

    /// 对象定义（右键「编辑」用它取到 SQL 再转 ALTER）。
    pub fn object_source(&self, object_type: &str, name: &str) -> Meta {
        let name_literal = self.literal(name);
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
                 where c.relname = {name_literal} and n.nspname not in ('pg_catalog','information_schema')"
            )),
            ("postgresql" | "kingbase", "procedure" | "function") => Meta::Sql(format!(
                "select pg_get_functiondef(p.oid) as definition from pg_proc p \
                 where p.proname = {name_literal} limit 1"
            )),
            // PG 的触发器定义（含 `ON <表>` 与 WHEN 条件），`pg_get_triggerdef` 就是官方入口
            ("postgresql" | "kingbase", "trigger") => Meta::Sql(format!(
                "select pg_get_triggerdef(t.oid) as definition from pg_trigger t \
                 join pg_class c on c.oid = t.tgrelid \
                 join pg_namespace n on n.oid = c.relnamespace \
                 where t.tgname = {name_literal} and not t.tgisinternal \
                   and n.nspname not in ('pg_catalog','information_schema')"
            )),
            // Oracle / DM：`dbms_metadata.get_ddl` 给的是 `CREATE OR REPLACE ...` 全文，
            // 比逐行拼 `user_source` 可靠（后者还要自己补换行与结尾分号）
            ("oracle" | "dm", "view") => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('VIEW', upper({name_literal})) as definition from dual"
            )),
            ("oracle" | "dm", "procedure") => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('PROCEDURE', upper({name_literal})) as definition from dual"
            )),
            ("oracle" | "dm", "function") => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('FUNCTION', upper({name_literal})) as definition from dual"
            )),
            ("oracle" | "dm", "trigger") => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('TRIGGER', upper({name_literal})) as definition from dual"
            )),
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
            "truncate" => {
                if self.key() == "sqlite" {
                    Meta::Sql(format!("delete from {t}"))
                } else {
                    Meta::Sql(format!("truncate table {t}"))
                }
            }
            "drop" => Meta::Sql(format!("drop table {t}")),
            "rename" => match new_name {
                Some(new_name) => {
                    let n = self.quote(new_name);
                    match self.key() {
                        // MySQL 系与 DB2 有各自的 RENAME TABLE
                        "mysql" | "mariadb" | "db2" => Meta::Sql(format!("rename table {t} to {n}")),
                        // SQL Server **没有** `ALTER TABLE … RENAME TO` —— 改表名只有 `sp_rename`。
                        // 之前它落进下面的通用分支，用户点「重命名表」只会拿到一句语法错。
                        // 两个细节：sp_rename 收的是**裸名**（它自己解析，不要带标识符引号）；
                        // 并且**不会更新引用该表的视图/存储过程**（前端会给这句提醒）。
                        "sqlserver" => Meta::Sql(format!(
                            "exec sp_rename {}, {}",
                            self.literal(table),
                            self.literal(new_name)
                        )),
                        // PostgreSQL / SQLite / Oracle / H2 / Derby / Doris / ClickHouse
                        // 都是 `ALTER TABLE … RENAME TO`
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
    /// 只有能**内联**注释的方言（MySQL / MariaDB / Doris / ClickHouse）会写进语句；
    /// PG / Oracle / SQL Server 的注释必须另发 `comment on …`，本轮不写（不猜语法），
    /// SQLite / Derby 没有注释这个概念，如实跳过。
    pub fn create_table_with_comments(
        &self,
        table: &str,
        columns: &[ColumnDetail],
        source: Option<ConnectionKind>,
        column_comments: &std::collections::HashMap<String, String>,
        table_comment: Option<&str>,
    ) -> String {
        let cross = matches!(source, Some(kind) if kind != self.kind);
        // (是否主键列, 行文本)：Doris 要求键列排最前，所以要记得住哪几行是键
        let mut lines: Vec<(bool, String)> = Vec::new();
        let mut primaries: Vec<String> = Vec::new();
        for column in columns {
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
            if !column.nullable {
                line.push_str(" not null");
            }
            if let Some(default) = &column.default_value {
                if !default.trim().is_empty() {
                    if let Some(literal) = self.default_literal(default, &type_name, cross) {
                        line.push_str(" default ");
                        line.push_str(&literal);
                    }
                }
            }
            // 自增：只在主键列上写（MySQL 的 auto_increment 就要求它是键，别的库写了也无害）
            if column.auto_increment && column.primary_key {
                if let Some(clause) = auto_increment_clause(self.kind, family) {
                    line.push(' ');
                    line.push_str(clause);
                }
            }
            // 字段注释：**内联**进列定义（只有这几个方言收这种写法）。
            // PG / Oracle / SQL Server 必须另发 `comment on column …`，不能内联；
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
                " default charset=utf8mb4".to_string() + &comment
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
    fn is_textual_type(type_name: &str) -> bool {
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
        if cross {
            // 先看**类型**：文本列的默认值是"内容"，可能恰好是叫 `now()` 的字符串 —— 那种要加引号，
            // 不能当成时间函数翻译掉。
            if !Self::is_textual_type(type_name) {
                if let Some(portable) = portable_time_default(&upper) {
                    return Some(portable.to_string());
                }
                if looks_like_call(&upper) {
                    return None;
                }
            }
        } else if upper.contains("CURRENT_") {
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
            "oracle" | "dm" => Meta::Sql(format!(
                "select column_name as column_name, coalesce(comments, '') as comment \
                 from user_col_comments where table_name = upper({quoted})"
            )),
            "h2" => Meta::Sql(format!(
                "select column_name as column_name, coalesce(remarks, '') as comment \
                 from information_schema.columns \
                 where table_name = upper({quoted})"
            )),
            "clickhouse" => Meta::Sql(format!(
                "select name as column_name, coalesce(comment, '') as comment \
                 from system.columns where database = currentDatabase() and table = {quoted}"
            )),
            "db2" => Meta::Sql(format!(
                "select colname as column_name, coalesce(remarks, '') as comment \
                 from syscat.colcomments \
                 where tabschema = current schema and tabname = upper({quoted})"
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
            // 自增三路取并：12c+ 的标识列在 `user_tab_identity_cols`，老写法是
            // 序列 + 默认值 `.nextval`（触发器赋值那种**查不出来**，只能靠默认值）。
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
                           where cc.table_name = c.table_name and cc.column_name = c.column_name), '') as comment, \
                 case when c.data_default like '%.nextval%' then 1 \
                      when exists (select 1 from user_tab_identity_cols i \
                                   where i.table_name = c.table_name and i.column_name = c.column_name) then 1 \
                      else 0 end as is_auto, \
                 c.column_id as ordinal \
                 from user_tab_columns c where c.table_name = upper({quoted})"
            )),
            // DM（达梦）：只给类型与注释。
            //
            // **不自增检测**：达梦的标识列（`IDENTITY(1,1)`）记在哪张系统视图上，我没有实例可验证；
            // 而 `user_tab_identity_cols` 在达梦上是否存在同样未验证 —— 一旦那句 SQL 报错，
            // 整张表的类型/注释补全都会一起丢掉（比少一个自增标记更糟）。所以这里留白，
            // 而不是拿一个没验证过的视图去赌。
            "dm" => Meta::Sql(format!(
                "select column_name as column_name, \
                 case \
                   when data_type = 'NUMBER' and data_precision is not null \
                     then 'number(' || data_precision || coalesce(',' || data_scale, '') || ')' \
                   when data_type in ('VARCHAR2', 'NVARCHAR2', 'CHAR', 'NCHAR') \
                     then lower(data_type) || '(' || data_length || ')' \
                   else lower(data_type) \
                 end as type_text \
                 from user_tab_columns where table_name = upper({quoted})"
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
            "clickhouse" => Meta::Sql(format!(
                "select name as column_name, type as type_text \
                 from system.columns where database = currentDatabase() and table = {quoted}"
            )),
            // H2：information_schema.columns 有长度/精度，自己拼回 `varchar(50)`。
            // 长度大于 100 万的不当"长度"用（CLOB 一类会报 2^31-1，拼进类型文本没意义）
            "h2" => Meta::Sql(format!(
                "select column_name as column_name, \
                 case \
                   when character_maximum_length is not null and character_maximum_length <= 1000000 \
                     then lower(data_type) || '(' || character_maximum_length || ')' \
                   when numeric_precision is not null and numeric_scale is not null \
                     then lower(data_type) || '(' || numeric_precision || ',' || numeric_scale || ')' \
                   when numeric_precision is not null \
                     then lower(data_type) || '(' || numeric_precision || ')' \
                   else lower(data_type) \
                 end as type_text \
                 from information_schema.columns where table_name = upper({quoted})"
            )),
            // DB2：syscat.columns 的 typename/length/scale
            "db2" => Meta::Sql(format!(
                "select colname as column_name, \
                 case \
                   when rtrim(typename) in ('VARCHAR','CHARACTER','CHAR','VARGRAPHIC','GRAPHIC','BINARY','VARBINARY') \
                     then lower(rtrim(typename)) || '(' || length || ')' \
                   when rtrim(typename) = 'DECIMAL' \
                     then lower(rtrim(typename)) || '(' || length || ',' || scale || ')' \
                   else lower(rtrim(typename)) \
                 end as type_text \
                 from syscat.columns \
                 where tabschema = current schema and tabname = upper({quoted})"
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
            "oracle" | "dm" => Meta::Sql(
                "select (select count(*) from user_tables) as tables, \
                 to_char(round(coalesce(sum(bytes), 0) / 1024 / 1024, 2)) || ' MB' as sizeText \
                 from user_segments"
                    .to_string(),
            ),
            // H2：表数能拿到；占用空间不在 information_schema 里，如实给「—」
            "h2" => Meta::Sql(
                "select count(*) as tables, '—' as sizeText \
                 from information_schema.tables where table_schema = 'PUBLIC'"
                    .to_string(),
            ),
            // DB2：syscat.tables 的页数（4KB 页）换算成 MB（数据页 + 索引页）
            "db2" => Meta::Sql(
                "select count(*) as tables, \
                 cast(sum(coalesce(data_object_pages, 0) + coalesce(index_object_pages, 0)) * 4 / 1024 as decimal(12,1)) || ' MB' as sizeText \
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
            Some(inner) if raw.trim().ends_with(')') => {
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
            "decimal32" | "decimal64" | "decimal128" | "decimal256" => Self::Decimal {
                precision: 38,
                scale: first.unwrap_or(0).min(38) as u8,
            },
            // 无符号整型：映射到有符号的"大一档"，超范围的一律落到定点，避免溢出
            "uint8" => Self::Int { bytes: 2 },
            "uint16" => Self::Int { bytes: 4 },
            "uint32" => Self::Int { bytes: 8 },
            "uint64" => Self::Decimal { precision: 20, scale: 0 },
            // ClickHouse 的整型/浮点族（Int32 / Float64 …）：之前没认，落到 Unknown，
            // 目标 Derby/H2 这类按 Unknown→CLOB 渲染的方言，整列全成 CLOB（真机踩过）
            "int8" => Self::Int { bytes: 1 },
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
        assert!(
            ddl.contains("ID number(19) not null generated by default as identity"),
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
}
