//! 「库 / 模式清单」SQL：界面树（`dbmind-web`）与 MCP 共用同一份文本。
//!
//! 这几条语句原先各存一份 —— `dialect_ops` 一份（界面用）、`dbmind-mcp` 一份（给模型挑库用）。
//! 两份已经漂开，且**MCP 那份是错的**：
//! - 金仓（KingbaseES）把系统目录改名成 `sys_*`，没有 `pg_catalog` 兼容层 ——
//!   MCP 沿用 PG 的 `pg_database` 会直接报「关系不存在」；
//! - PG 的库清单漏了 `has_database_privilege(datname,'CONNECT')`，模型会挑到连不上的库；
//! - 模式清单漏了 `has_schema_privilege(oid,'USAGE')`，挑到没权限的模式只会得到
//!   `permission denied for schema`。
//!
//! 放这里之后「哪种库怎么列库/列模式」只有一处：新增方言改这里，两边同时正确。

use crate::ConnectionKind;

pub const MYSQL_DATABASES: &str = "show databases";

pub const CLICKHOUSE_DATABASES: &str = "select name from system.databases order by name";

/// PG 族库清单：只列**当前账号能连**的库（`datistemplate=false` 排除模板库）。
pub const PG_DATABASES: &str = "select datname as name from pg_database \
     where datistemplate = false and has_database_privilege(datname, 'CONNECT') order by datname";

/// PG 族模式清单：口径定为「不显示没权限的」（用户 2026-11-11 拍板）。
///
/// Supabase 这类托管库里 `pg_namespace` 有 30 个模式，当前账号只有少数几个有 USAGE；
/// 列出来但点进去每条查询都 `permission denied for schema` —— 不如不显示。
pub const PG_SCHEMAS: &str = "select nspname as name from pg_namespace \
     where nspname not like 'pg\\_%' and nspname <> 'information_schema' \
       and has_schema_privilege(oid, 'USAGE') order by nspname";

/// 金仓：目录是 `sys_catalog.sys_*`（没有 `pg_catalog` 兼容层），过滤也排除 `sys_%`。
pub const KB_DATABASES: &str = "select datname as name from sys_catalog.sys_database \
     where datistemplate = false and has_database_privilege(datname, 'CONNECT') order by datname";

pub const KB_SCHEMAS: &str = "select nspname as name from sys_catalog.sys_namespace \
     where nspname not like 'pg\\_%' and nspname not like 'sys\\_%' \
       and nspname <> 'information_schema' \
       and has_schema_privilege(oid, 'USAGE') order by nspname";

pub const SQLSERVER_DATABASES: &str =
    "select name from sys.databases where state = 0 and has_dbaccess(name) = 1 order by name";

/// SQL Server：固定角色是**数据库级角色**，不是用户建的模式，列出来只会误导
/// （点进去永远是空的）。
pub const SQLSERVER_SCHEMAS: &str = "select name from sys.schemas \
     where name not in ('guest','sys','INFORMATION_SCHEMA', \
       'db_owner','db_accessadmin','db_backupoperator','db_datareader','db_datawriter', \
       'db_ddladmin','db_denydatareader','db_denydatawriter','db_securityadmin') \
     order by name";

/// Oracle / DM：库名就是用户名。不过滤（`/databases` 这一层要列全，界面按它画「库」节点）。
pub const ORACLE_DATABASES: &str = "select username as name from all_users order by username";

/// Oracle / DM 的模式清单：只列「当前用户」与「确实有对象的用户」——
/// 几十个系统账号（`SYS`/`SYSTEM`/`AUDSYS`…）点进去全是空的，纯粹的噪音。
pub const ORACLE_SCHEMAS: &str = "select username as name from all_users u \
     where username = sys_context('USERENV','CURRENT_SCHEMA') \
        or exists (select 1 from all_objects o \
                   where o.owner = u.username and rownum = 1) \
     order by username";

/// DB2 没有「库」这一层：schema 就是它的库（MCP 侧一直这么用）。
pub const DB2_SCHEMAS: &str = "select schemaname as name from syscat.schemata order by schemaname";

pub const H2_SCHEMAS: &str = "select schema_name as \"name\" from information_schema.schemata \
     where schema_name <> 'INFORMATION_SCHEMA' order by schema_name";

pub const DERBY_SCHEMAS: &str = "select schemaname as name from sys.sysschemas order by schemaname";

/// 这种类型的「库」清单 SQL。`None` = 没有库这一层（SQLite / NoSQL）。
pub fn databases_sql(kind: ConnectionKind) -> Option<&'static str> {
    Some(match kind.key() {
        // MySQL 协议族的 `show databases` 不需要先选库，正好能在「未绑库」的连接上跑
        "mysql" | "mariadb" | "tidb" | "oceanbase" | "greatsql" | "goldendb" | "doris"
        | "starrocks" | "selectdb" | "databend" => MYSQL_DATABASES,
        "clickhouse" => CLICKHOUSE_DATABASES,
        "postgresql" | "opengauss" | "risingwave" | "highgo" | "vastbase" | "uxdb" | "gaussdb" => {
            PG_DATABASES
        }
        "kingbase" => KB_DATABASES,
        "sqlserver" => SQLSERVER_DATABASES,
        // DB2 的 schema 就是它的库
        "db2" => DB2_SCHEMAS,
        "oracle" | "dm" => ORACLE_DATABASES,
        "h2" => H2_SCHEMAS,
        "derby" => DERBY_SCHEMAS,
        _ => return None,
    })
}

/// 这种类型的「模式」清单 SQL（只有 schema 层级的类型有）。
pub fn schemas_sql(kind: ConnectionKind) -> Option<&'static str> {
    Some(match kind.key() {
        "sqlserver" => SQLSERVER_SCHEMAS,
        "postgresql" | "opengauss" | "risingwave" | "highgo" | "vastbase" | "uxdb" | "gaussdb" => {
            PG_SCHEMAS
        }
        "kingbase" => KB_SCHEMAS,
        "db2" => DB2_SCHEMAS,
        "oracle" | "dm" => ORACLE_SCHEMAS,
        "h2" => H2_SCHEMAS,
        "derby" => DERBY_SCHEMAS,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每条语句都必须带 `name` 别名：调用方按列名取值，
    /// 少了它整张清单是空数组（界面显示"没有库"），却没有任何报错。
    #[test]
    fn 每条清单语句都带_name_列() {
        for kind in ConnectionKind::ALL {
            for sql in [databases_sql(*kind), schemas_sql(*kind)].into_iter().flatten() {
                // `select name from …` 本来就是 name 列，不必再起别名
                let named = sql.contains("as name")
                    || sql.contains("as \"name\"")
                    || sql.starts_with("show ")
                    || sql.starts_with("select name from");
                assert!(named, "{} 的清单语句没有 name 列：{sql}", kind.key());
            }
        }
    }

    /// 金仓绝不能用 PG 的目录名 —— 它没有 `pg_catalog` 兼容层（真机报「关系不存在」）。
    #[test]
    fn 金仓不用_pg_目录() {
        for sql in [
            databases_sql(ConnectionKind::Kingbase).unwrap(),
            schemas_sql(ConnectionKind::Kingbase).unwrap(),
        ] {
            assert!(
                !sql.contains("pg_database") && !sql.contains("pg_namespace"),
                "金仓的清单语句引用了 PG 目录：{sql}"
            );
        }
    }

    /// PG 族必须过滤权限：列出来但连不上/进不去的库和模式对模型是纯误导。
    #[test]
    fn pg_族带权限过滤() {
        assert!(databases_sql(ConnectionKind::Postgresql)
            .unwrap()
            .contains("has_database_privilege"));
        assert!(schemas_sql(ConnectionKind::Postgresql)
            .unwrap()
            .contains("has_schema_privilege"));
    }
}
