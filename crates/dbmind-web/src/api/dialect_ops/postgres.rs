//! PostgreSQL 语法族：PostgreSQL / KingbaseES。

use super::*;

// ==================== KingbaseES 的 sys_catalog 适配 ====================
//
// 背景（2026-11-17 在实验室的 KingbaseES V008R003C002B0320 上真机核对）：
// 人大金仓把 PG 的系统目录整套改名成了 `sys_*`，**没有 `pg_catalog` 兼容层** ——
// `select count(*) from pg_catalog.pg_class` 直接报「关系不存在」。
// 于是「PostgreSQL 方言的目录 SQL 原样拿到金仓上跑」整片失败：模式、表、列、DDL、
// 索引、用户六个页签全空，报错清一色是 `关系 "pg_namespace"/"pg_class" 不存在`。
//
// 处理办法：金仓的 impl 把 PG 那份目录 SQL 过一遍**词法级改名**（只作用于我们自己
// 产出的元数据 SQL，绝不碰用户 SQL）。逐个函数核对过是否存在同名 `sys_` 版本，
// 没有的（`format_type` / `col_description` / `obj_description` / `aclexplode`）保持原样。
//
// 注意 `pg_indexes` 这个**视图**在金仓里存在但定义本身引用了 `pg_class`，一查就报错，
// 所以索引走 `KB_INDEXES` 直接从 `sys_index` 拼，不做改名。
const KB_WORDS: &[(&str, &str)] = &[
    // —— 目录模式 ——
    ("pg_catalog", "sys_catalog"),
    // —— 目录表 ——
    ("pg_namespace", "sys_namespace"),
    ("pg_attribute", "sys_attribute"),
    ("pg_attrdef", "sys_attrdef"),
    ("pg_constraint", "sys_constraint"),
    ("pg_database", "sys_database"),
    ("pg_description", "sys_description"),
    ("pg_auth_members", "sys_auth_members"),
    ("pg_stat_activity", "sys_stat_activity"),
    ("pg_stat_database", "sys_stat_database"),
    ("pg_stat_user_tables", "sys_stat_user_tables"),
    // 监控「锁等待」面板要从锁视图取 mode（`pg_stat_activity` 没有这一列）
    ("pg_locks", "sys_locks"),
    ("pg_tablespace", "sys_tablespace"),
    ("pg_settings", "sys_settings"),
    ("pg_trigger", "sys_trigger"),
    ("pg_roles", "sys_roles"),
    ("pg_class", "sys_class"),
    ("pg_index", "sys_index"),
    ("pg_proc", "sys_proc"),
    ("pg_type", "sys_type"),
    // —— 函数（都在真机上验过同名 sys_ 版本可用）——
    ("pg_total_relation_size", "sys_total_relation_size"),
    ("pg_relation_size", "sys_relation_size"),
    // 监控「表空间 Top 20」要单独取索引占用，漏了这一项整块面板报「函数不存在」
    ("pg_indexes_size", "sys_indexes_size"),
    ("pg_database_size", "sys_database_size"),
    ("pg_size_pretty", "sys_size_pretty"),
    ("pg_get_userbyid", "sys_get_userbyid"),
    ("pg_get_indexdef", "sys_get_indexdef"),
    ("pg_get_viewdef", "sys_get_viewdef"),
    ("pg_get_functiondef", "sys_get_functiondef"),
    ("pg_get_triggerdef", "sys_get_triggerdef"),
    ("pg_get_constraintdef", "sys_get_constraintdef"),
    ("pg_get_serial_sequence", "sys_get_serial_sequence"),
    ("pg_get_expr", "sys_get_expr"),
    ("pg_blocking_pids", "sys_blocking_pids"),
    ("pg_backend_pid", "sys_backend_pid"),
    ("pg_terminate_backend", "sys_terminate_backend"),
    ("pg_postmaster_start_time", "sys_postmaster_start_time"),
    ("pg_control_system", "sys_control_system"),
];

/// 把一条 PG 目录 SQL 改成金仓可执行的写法。
///
/// 按**标识符边界**逐词替换：`pg_index` 不会误伤 `pg_indexes`，字符串字面量里的
/// `'pg_catalog'` 也会被改成 `'sys_catalog'`（金仓的系统模式就叫这个，正是想要的）。
///
/// `pub(crate)` 是给**绕开 `DialectOps` 手写 SQL** 的几处用的（监控总览、容量排行、
/// 同步的行数估算、比对的外键定义）：它们把 PG 的语句写死在 `match kind` 里，
/// 金仓跟着 PG 分支走就会整条报「关系 pg_class 不存在」。只允许用在这些**我们自己的**
/// 元数据语句上，用户 SQL 一律不许过它。
pub(crate) fn kb_catalog_sql(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len() + 32);
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch.is_ascii_alphanumeric() || ch == '_' {
            let start = i;
            while i < bytes.len() {
                let c = bytes[i] as char;
                if c.is_ascii_alphanumeric() || c == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let word = &sql[start..i];
            match KB_WORDS.iter().find(|(from, _)| *from == word) {
                Some((_, to)) => out.push_str(to),
                None => out.push_str(word),
            }
        } else {
            out.push(ch);
            i += 1;
        }
    }
    out
}

/// 目录 SQL 走一遍 [`kb_catalog_sql`]；非 SQL 形态（Absent / Unwritten）原样返回。
fn kb_meta(meta: Meta) -> Meta {
    match meta {
        Meta::Sql(sql) => Meta::Sql(kb_catalog_sql(&sql)),
        other => other,
    }
}

/// 金仓的模式清单：系统模式名是 `sys_*`，且 `information_schema` 是**小写**的
/// （金仓是 case-insensitive 模式，未加引号的标识符折成大写），所以过滤条件两条都要写。
/// 金仓模式清单。文本来自 `dbmind_core::scope_sql`（MCP 用的是同一份）。
const KB_SCHEMAS: &str = dbmind_core::scope_sql::KB_SCHEMAS;

/// 金仓的索引清单：`sys_indexes` 视图自身引用了不存在的 `pg_class`，只能自己拼。
const KB_INDEXES: &str = "select n.nspname as schema, ic.relname as name, tc.relname as \"table\", \
     sys_get_indexdef(i.indexrelid) as indexdef \
     from sys_catalog.sys_index i \
     join sys_catalog.sys_class ic on ic.oid = i.indexrelid \
     join sys_catalog.sys_class tc on tc.oid = i.indrelid \
     join sys_catalog.sys_namespace n on n.oid = tc.relnamespace \
     where n.nspname not in ('sys_catalog','information_schema') \
     order by tc.relname, ic.relname";

const KB_DATABASES: &str = dbmind_core::scope_sql::KB_DATABASES;

const KB_DATABASE_STATS: &str = "select (select count(*) from information_schema.tables \
     where table_schema not in ('sys_catalog','information_schema')) as tables, \
     sys_size_pretty(sys_database_size(current_database())) as sizeText";

const KB_FINGERPRINT: &str = "select system_identifier::text as v from sys_control_system()";

impl DialectOps for PostgresOps {
    fn is_revocable(&self, scope: &str, privilege: &str, owner: &str, object: &str, kind: &str) -> bool {
        match scope {
            "schema" => crate::api::dialect::is_pg_schema_priv(privilege) && !owner.is_empty(),
            // 库级：撤销 `revoke connect on database "db" from "u"`（owner = 库名）
            "database" => crate::api::dialect::is_pg_database_priv(privilege) && !owner.is_empty(),
            "table" => {
                crate::api::dialect::is_pg_table_priv(privilege) && !owner.is_empty() && !object.is_empty()
            }
            // 角色成员关系：这里是**角色名**不是权限名，只放行常规字符
            "role" => crate::api::dialect::is_plain_ident(privilege),
            "routine" | "function" => {
                crate::api::dialect::is_pg_routine_priv(privilege)
                    && !owner.is_empty()
                    && !object.is_empty()
                    && matches!(kind, "PROCEDURE" | "FUNCTION")
            }
            _ => false,
        }
    }

    fn grant_input_scopes(&self) -> &'static [&'static str] {
        // 模式（USAGE/CREATE）/ 表（七项）/ 例程（EXECUTE）+ 撤销差量。
        // 「数据库权限」tab 对 PG 交的是**模式级**授权（dbPrivileges 的 database 字段就是模式名）。
        &["dbPrivileges", "tablePrivileges", "routinePrivileges", "revoke"]
    }

    fn grant_line_text(
        &self,
        scope: &str,
        privilege: &str,
        owner: &str,
        object: &str,
        account_user: &str,
        _account_host: &str,
    ) -> Option<String> {
        pg_grant_line_text(self, scope, privilege, owner, object, account_user)
    }

    // 默认转义符就是 `\`，显式 escape 反而可能被判重复声明
    fn like_escape_suffix(&self) -> &'static str {
        ""
    }
    fn object_source(&self, object_type: &str, name: &str, schema: Option<&str>) -> Meta {
        pg_object_source(self, object_type, name, schema)
    }

    fn post_create_comments(
        &self,
        table: &str,
        column_comments: &std::collections::HashMap<String, String>,
        table_comment: Option<&str>,
        schema: &str,
    ) -> Vec<String> {
        comment_on_statements(self, table, column_comments, table_comment, schema)
    }

    fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        pg_ddl(self, table, schema)
    }

    fn user_action_sql(
        &self,
        action: &str,
        _as_role: bool,
        user: &str,
        password: &str,
        _host: &str,
        roles: &[String],
        privileges: &UserPrivileges,
        revokes: &[GrantRevoke],
        _reset_privileges: bool,
        attrs: &UserAttrs,
    ) -> Meta {
        pg_user_action(self, action, user, password, roles, privileges, revokes, attrs)
    }

    fn users(&self) -> Meta {
        pg_users()
    }

    fn user_info(&self, name: &str) -> Meta {
        pg_user_info(self, name)
    }

    fn user_grants(&self, user: &str, _host: &str) -> Meta {
        pg_user_grants(self, user)
    }

    fn user_roles(&self, user: &str, _host: &str) -> Meta {
        pg_user_roles(self, user)
    }

    fn role_info(&self, name: &str) -> Meta {
        pg_role_info(self, name)
    }

    fn role_grants(&self, name: &str, _host: &str) -> Meta {
        pg_role_grants(self, name)
    }

    fn role_inventory(&self) -> Meta {
        pg_role_inventory()
    }

    fn schemas_sql(&self) -> Option<&'static str> {
        Some(PG_SCHEMAS)
    }

    fn tables_in_schema_sql(&self, schema: &str) -> Option<String> {
        Some(pg_tables_in_schema(self, schema))
    }

    fn procedures(&self, schema: Option<&str>) -> Meta {
        pg_procedures(self, schema)
    }

    fn triggers(&self, schema: Option<&str>) -> Meta {
        pg_triggers(self, schema)
    }

    fn table_options_sql(&self, database: &str) -> Option<String> {
        Some(pg_table_options(self, database))
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        Some(PG_INDEXES)
    }

    fn upsert_sql(
        &self,
        table: &str,
        columns: &[String],
        key_indexes: &[usize],
        rows: &[Vec<String>],
    ) -> Option<String> {
        let p = upsert_parts(self, columns, key_indexes, rows)?;
        Some(on_conflict_upsert(&p, table))
    }
    fn supports_drop_object(&self, kind: &str) -> bool {
        matches!(kind, "view" | "function" | "trigger")
    }
    fn rename_object_sql(&self, kind: &str, from: &str, to: &str, schema: Option<&str>) -> Option<String> {
        pg_rename_object_sql(self, kind, from, to, schema)
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        pg_column_types(quoted)
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        // RisingWave 没有 `pg_database_size` / `pg_size_pretty`：如实报「未接入」，
        // 别拼一条必炸的 SQL 让整个统计卡片变 500
        if !self.supports_pg_catalog_stats() {
            return None;
        }
        Some(pg_database_stats())
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        pg_table_comments(quoted)
    }
    fn databases_sql(&self) -> Option<&'static str> {
        Some(dbmind_core::scope_sql::PG_DATABASES)
    }
    fn folds_ident_lower(&self) -> bool {
        true
    }
    fn is_schema_aware(&self) -> bool {
        true
    }
    fn switchable_database(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "postgresql"
    }

    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::POSTGRES
    }

    fn supports_pg_catalog_stats(&self) -> bool {
        // RisingWave：`pg_class` 没有 `reltuples`，也没有 `pg_total_relation_size` /
        // `pg_size_pretty` / `pg_database_size`（真机逐项探测过，见 trait 上的注释）
        !self.risingwave
    }

    fn pg_table_visibility_privileges(&self) -> &'static str {
        // RisingWave 只认这四种表级权限，TRUNCATE / REFERENCES / TRIGGER 会在执行期报
        // `Invalid parameter privilege: unrecognized`（见 trait 上的注释）
        if self.risingwave {
            "SELECT,INSERT,UPDATE,DELETE"
        } else {
            "SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER"
        }
    }

    fn drop_table_sql(&self, quoted_table: &str) -> String {
        // 目标库里已有视图引用这张表时，裸 drop 直接失败 ⇒ 重同步整张表红灯。
        // `cascade` 连依赖视图一起清掉 —— 表都要重建了，视图本来就该失效。
        format!("drop table if exists {quoted_table} cascade")
    }

    fn table_rows_estimate_sql(&self, _database: &str, schema: &str) -> Option<String> {
        if !self.supports_pg_catalog_stats() {
            // RisingWave：没有 reltuples ⇒ 整库同步不按估算行数排序（小表先跑），
            // 比拼一条必炸的 SQL 强
            return None;
        }
        // `reltuples` 未 ANALYZE 时是 -1，按 0 处理；openGauss 目录同名，直接共用
        let schema = schema.replace('\'', "''");
        Some(format!(
            "select c.relname as t, coalesce(c.reltuples, 0)::bigint as n from pg_class c \
             join pg_namespace ns on ns.oid = c.relnamespace \
             where ns.nspname = '{schema}' and c.relkind = 'r'"
        ))
    }

    fn column_type_names(&self) -> &'static [&'static str] {
        editor::POSTGRES_COLUMN_TYPES
    }

    fn index_type_names(&self) -> &'static [&'static str] {
        editor::POSTGRES_INDEX_TYPES
    }
    fn supports_native_upsert(&self) -> bool {
        true
    }
    fn version_sql(&self) -> Option<&'static str> {
        Some("select version() as v")
    }
    fn supports_row_value_compare(&self) -> bool {
        true
    }
    fn direct_ref_depth(&self) -> DirectRefDepth {
        DirectRefDepth::Schema
    }
    fn instance_fingerprint_sql(&self) -> Option<&'static str> {
        Some("select system_identifier::text as v from pg_control_system()")
    }
}

impl DialectOps for KingbaseOps {
    fn is_revocable(&self, scope: &str, privilege: &str, owner: &str, object: &str, kind: &str) -> bool {
        match scope {
            "schema" => crate::api::dialect::is_pg_schema_priv(privilege) && !owner.is_empty(),
            // 库级：撤销 `revoke connect on database "db" from "u"`（owner = 库名）
            "database" => crate::api::dialect::is_pg_database_priv(privilege) && !owner.is_empty(),
            "table" => {
                crate::api::dialect::is_pg_table_priv(privilege) && !owner.is_empty() && !object.is_empty()
            }
            // 角色成员关系：这里是**角色名**不是权限名，只放行常规字符
            "role" => crate::api::dialect::is_plain_ident(privilege),
            "routine" | "function" => {
                crate::api::dialect::is_pg_routine_priv(privilege)
                    && !owner.is_empty()
                    && !object.is_empty()
                    && matches!(kind, "PROCEDURE" | "FUNCTION")
            }
            _ => false,
        }
    }

    fn grant_input_scopes(&self) -> &'static [&'static str] {
        // 模式（USAGE/CREATE）/ 表（七项）/ 例程（EXECUTE）+ 撤销差量。
        // 「数据库权限」tab 对 PG 交的是**模式级**授权（dbPrivileges 的 database 字段就是模式名）。
        &["dbPrivileges", "tablePrivileges", "routinePrivileges", "revoke"]
    }

    fn grant_line_text(
        &self,
        scope: &str,
        privilege: &str,
        owner: &str,
        object: &str,
        account_user: &str,
        _account_host: &str,
    ) -> Option<String> {
        pg_grant_line_text(self, scope, privilege, owner, object, account_user)
    }

    // 默认转义符就是 `\`，显式 escape 反而可能被判重复声明
    fn like_escape_suffix(&self) -> &'static str {
        ""
    }

    fn object_source(&self, object_type: &str, name: &str, schema: Option<&str>) -> Meta {
        kb_meta(pg_object_source(self, object_type, name, schema))
    }

    fn post_create_comments(
        &self,
        table: &str,
        column_comments: &std::collections::HashMap<String, String>,
        table_comment: Option<&str>,
        schema: &str,
    ) -> Vec<String> {
        comment_on_statements(self, table, column_comments, table_comment, schema)
    }

    fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        kb_meta(pg_ddl(self, table, schema))
    }

    fn user_action_sql(
        &self,
        action: &str,
        _as_role: bool,
        user: &str,
        password: &str,
        _host: &str,
        roles: &[String],
        privileges: &UserPrivileges,
        revokes: &[GrantRevoke],
        _reset_privileges: bool,
        attrs: &UserAttrs,
    ) -> Meta {
        pg_user_action(self, action, user, password, roles, privileges, revokes, attrs)
    }

    fn users(&self) -> Meta {
        kb_meta(pg_users())
    }

    fn user_info(&self, name: &str) -> Meta {
        kb_meta(pg_user_info(self, name))
    }

    fn user_grants(&self, user: &str, _host: &str) -> Meta {
        kb_meta(pg_user_grants(self, user))
    }

    fn user_roles(&self, user: &str, _host: &str) -> Meta {
        kb_meta(pg_user_roles(self, user))
    }

    fn role_info(&self, name: &str) -> Meta {
        kb_meta(pg_role_info(self, name))
    }

    fn role_grants(&self, name: &str, _host: &str) -> Meta {
        kb_meta(pg_role_grants(self, name))
    }

    fn role_inventory(&self) -> Meta {
        kb_meta(pg_role_inventory())
    }

    fn schemas_sql(&self) -> Option<&'static str> {
        // 金仓的系统模式叫 `sys_*`（没有 pg_catalog），过滤条件也得跟着改
        Some(KB_SCHEMAS)
    }

    fn tables_in_schema_sql(&self, schema: &str) -> Option<String> {
        Some(kb_catalog_sql(&pg_tables_in_schema(self, schema)))
    }

    fn procedures(&self, schema: Option<&str>) -> Meta {
        kb_meta(pg_procedures(self, schema))
    }

    fn triggers(&self, schema: Option<&str>) -> Meta {
        kb_meta(pg_triggers(self, schema))
    }

    fn table_options_sql(&self, database: &str) -> Option<String> {
        Some(kb_catalog_sql(&pg_table_options(self, database)))
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        // `sys_indexes` 视图自身引用了不存在的 pg_class，只能从 sys_index 拼
        Some(KB_INDEXES)
    }

    fn upsert_sql(
        &self,
        table: &str,
        columns: &[String],
        key_indexes: &[usize],
        rows: &[Vec<String>],
    ) -> Option<String> {
        let p = upsert_parts(self, columns, key_indexes, rows)?;
        Some(on_conflict_upsert(&p, table))
    }
    fn supports_drop_object(&self, kind: &str) -> bool {
        matches!(kind, "view" | "function" | "trigger")
    }
    fn rename_object_sql(&self, kind: &str, from: &str, to: &str, schema: Option<&str>) -> Option<String> {
        pg_rename_object_sql(self, kind, from, to, schema)
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        kb_meta(pg_column_types(quoted))
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        Some(KB_DATABASE_STATS)
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        kb_meta(pg_table_comments(quoted))
    }
    fn databases_sql(&self) -> Option<&'static str> {
        Some(KB_DATABASES)
    }
    fn folds_ident_lower(&self) -> bool {
        true
    }
    fn is_schema_aware(&self) -> bool {
        true
    }
    fn switchable_database(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "kingbase"
    }

    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::POSTGRES
    }

    fn drop_table_sql(&self, quoted_table: &str) -> String {
        // 金仓与 PG 的 `cascade` 行为一致，但原实现只给 PG 加了 —— 保持一致不动，
        // 免得「清场」顺手删掉用户手工建的视图
        format!("drop table if exists {quoted_table}")
    }

    fn table_rows_estimate_sql(&self, _database: &str, schema: &str) -> Option<String> {
        // 金仓把系统目录整套改名成 `sys_*`（没有 pg_catalog 兼容层），
        // 同一条语句必须过一遍词法改名，否则整库同步拿不到行数估算
        let schema = schema.replace('\'', "''");
        let sql = format!(
            "select c.relname as t, coalesce(c.reltuples, 0)::bigint as n from pg_class c \
             join pg_namespace ns on ns.oid = c.relnamespace \
             where ns.nspname = '{schema}' and c.relkind = 'r'"
        );
        Some(kb_catalog_sql(&sql))
    }

    fn column_type_names(&self) -> &'static [&'static str] {
        editor::POSTGRES_COLUMN_TYPES
    }

    fn index_type_names(&self) -> &'static [&'static str] {
        editor::POSTGRES_INDEX_TYPES
    }
    fn supports_native_upsert(&self) -> bool {
        true
    }
    fn version_sql(&self) -> Option<&'static str> {
        Some("select version() as v")
    }
    fn direct_ref_depth(&self) -> DirectRefDepth {
        DirectRefDepth::Schema
    }
    fn instance_fingerprint_sql(&self) -> Option<&'static str> {
        Some(KB_FINGERPRINT)
    }
}

/// PG 协议族的列注释查询（PostgresOps 与 KingbaseOps 共用）。
///
/// 注释正文在 `col_description(oid, attnum)` 里，不在 information_schema。
fn pg_table_comments(quoted: &str) -> Meta {
    Meta::Sql(format!(
        "select a.attname as column_name, coalesce(col_description(a.attrelid, a.attnum), '') as comment \
         from pg_attribute a \
         where a.attrelid = {quoted}::regclass and a.attnum > 0 and not a.attisdropped"
    ))
}

/// PG 协议族的库级统计（PostgresOps / KingbaseOps 共用）。
fn pg_database_stats() -> &'static str {
    "select (select count(*) from information_schema.tables \
     where table_schema not in ('pg_catalog','information_schema')) as tables, \
     pg_size_pretty(pg_database_size(current_database())) as sizeText"
}

/// PG 协议族的列类型（PostgresOps / KingbaseOps 共用）。
///
/// 用 `udt_name`（`varchar` / `int4` / `numeric`），比 `data_type` 的
/// `character varying` 更贴近界面要的短名；长度按「字符」给。
///
/// 自增有两种写法，都要认：`GENERATED AS IDENTITY`（标准 SQL，PG 10+，
/// information_schema 的 is_identity 直接给）与 `serial`（等价于默认值取
/// `nextval('...'::regclass)`）。少认一种，界面就会把自增列显示成普通列。
fn pg_column_types(quoted: &str) -> Meta {
    Meta::Sql(format!(
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
    ))
}

/// PG 协议族的改名语句（PostgresOps / KingbaseOps 共用）。
///
/// 触发器要在所属表上改（`alter trigger … on <表>`），这里不支持 —— 返回 `None`
/// 让调用方改走「改写定义正文」。
fn pg_rename_object_sql(
    ops: &dyn DialectOps,
    kind: &str,
    from: &str,
    to: &str,
    schema: Option<&str>,
) -> Option<String> {
    if !matches!(kind, "view" | "procedure" | "function") {
        return None;
    }
    let qualified = match schema.map(str::trim).filter(|s| !s.is_empty()) {
        Some(schema) => format!("{}.{}", ops.quote(schema), ops.quote(from)),
        None => ops.quote(from),
    };
    Some(format!("alter {kind} {qualified} rename to {}", ops.quote(to)))
}

// ==================== PG 族：表选项 / 索引 ====================

/// 估算行数（reltuples，未 ANALYZE 时是 -1，压成 0）、属主、总占用（表+索引+toast）、
/// 表注释 —— 对象清单页一屏能看到表有多大、归谁。
fn pg_table_options(ops: &dyn DialectOps, database: &str) -> String {
    let db = ops.literal(database);
    // 行数与占用都来自统计（`reltuples` / `pg_total_relation_size` + `pg_size_pretty`）；
    // 没有这些的方言（RisingWave）只能给 null —— 列名保持不变，前端按「没拿到」跳过。
    let (rows_col, size_col) = if ops.supports_pg_catalog_stats() {
        (
            "greatest(c.reltuples, 0)::bigint::text as rows",
            "pg_size_pretty(pg_total_relation_size(c.oid)) as size",
        )
    } else {
        ("cast(null as text) as rows", "cast(null as text) as size")
    };
    format!(
        "select c.relname as table_name, \
         {rows_col}, \
         pg_get_userbyid(c.relowner) as owner, \
         {size_col}, \
         coalesce(obj_description(c.oid, 'pg_class'), '') as comment \
         from pg_class c join pg_namespace n on n.oid = c.relnamespace \
         where n.nspname = {db} and c.relkind in ('r','p')"
    )
}

const PG_INDEXES: &str = "select schemaname as schema, indexname as name, tablename as \"table\", indexdef \
     from pg_indexes where schemaname not in ('pg_catalog','information_schema') \
     order by tablename, indexname";

// ==================== PG 族：schema / 表 / 例程 / 触发器 ====================

/// `pg_namespace` 是**公开目录**，所有角色都能看到全部 schema 名 —— 不按权限收窄的话，
/// 树上会列出一堆点进去空、连建表都不允许的模式（实测：给低权限账号建一个无 USAGE 的
/// schema，它就出现在清单里）。
///
/// 判据用 `USAGE`：没有它连按名字定位对象都不行，等价于「这个模式对它不可用」。
///
/// 曾经为了与 Navicat 对齐（它不过滤，Supabase 库里 30 个模式全列）改成过「全部列出、
/// 有权限的排前面」，但那样点进去全是空 —— 用户 2026-11-11 明确选择**不显示没权限的**。
/// 要与 Navicat 对齐时别再改这里，先问清楚「列出来但打不开」是不是想要的。
/// PG 族模式清单。口径「不显示没权限的」+ 文本真源都收在 `dbmind_core::scope_sql`。
const PG_SCHEMAS: &str = dbmind_core::scope_sql::PG_SCHEMAS;

/// 带 schema 就只看它，不带就排除系统模式 —— 例程与触发器同一口径。
fn pg_schema_filter(ops: &dyn DialectOps, schema: Option<&str>, fallback: &str) -> String {
    match schema {
        Some(s) if !s.trim().is_empty() => format!("and n.nspname = {}", ops.literal(s.trim())),
        _ => fallback.to_string(),
    }
}

/// `reltuples` 是**估算值**（未 ANALYZE 时是 -1），拿不到就给 null —— 界面上留空，
/// 不要用 0 冒充「空表」。精确行数由前端展开后的批量 COUNT(*) 回填。
///
/// `pg_class` 同样是公开目录 —— 不加权限判断就会把「这个账号一张都读不了」的表/视图
/// 列出来（实测低权限账号看到了没授过任何权限的表与视图）。判据是「表级权限里
/// **任意一种**」：只授了 INSERT 的表也应当能看到，只看 SELECT 会把它藏起来（那样更糟：
/// 用户明明能写，却在树里找不到）。具体清单按方言给（RisingWave 只有 DML 四种，
/// 见 [`DialectOps::pg_table_visibility_privileges`]）。
fn pg_tables_in_schema(ops: &dyn DialectOps, schema: &str) -> String {
    // RisingWave 的 `pg_class` 没有 `reltuples` ⇒ 这一列连「选中」都不行，
    // 整棵表清单 `Failed to bind expression`。改成 null 交给前端 COUNT(*) 回填。
    let rows_col = if ops.supports_pg_catalog_stats() {
        "nullif(c.reltuples, -1)::bigint as rows"
    } else {
        "cast(null as bigint) as rows"
    };
    format!(
        "select c.relname as name, \
         case when c.relkind = 'v' then 'VIEW' else 'BASE TABLE' end as type, \
         {rows_col} \
         from pg_class c join pg_namespace n on n.oid = c.relnamespace \
         where n.nspname = {} and c.relkind in ('r','p','v','f') \
           and has_table_privilege(c.oid, '{}') \
         order by c.relname",
        ops.literal(schema),
        ops.pg_table_visibility_privileges()
    )
}

/// `pg_proc` 也是公开目录：把 EXECUTE 从 PUBLIC 收回的 `secret_fn` 照样会被列出来
/// （实测低权限账号看到了它）。PG 默认把 EXECUTE 授给 PUBLIC，所以这条过滤通常
/// "不改变结果"，只在真收权时生效 —— 正是要的效果。
fn pg_procedures(ops: &dyn DialectOps, schema: Option<&str>) -> Meta {
    let filter = pg_schema_filter(
        ops,
        schema,
        "and n.nspname not in ('pg_catalog','information_schema')",
    );
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

/// PG 的触发器清单原先**既不看模式也不看权限**：`pg_trigger` 是全库公开目录，一句话
/// 就把所有 schema 的触发器全列出来 —— 树上「触发器」节点混着 pg_catalog 与别人
/// schema 的东西，还包含当前账号根本读不了的表的触发器。改成与 procedures 同口径，
/// 再用「表级任意权限」把读不到的表上的触发器滤掉。
fn pg_triggers(ops: &dyn DialectOps, schema: Option<&str>) -> Meta {
    let filter = pg_schema_filter(
        ops,
        schema,
        "and n.nspname not in ('pg_catalog','information_schema') \
         and n.nspname not like 'pg\\_%'",
    );
    Meta::Sql(format!(
        "select t.tgname as name, c.relname as \"table\" from pg_trigger t \
         join pg_class c on c.oid = t.tgrelid \
         join pg_namespace n on n.oid = c.relnamespace \
         where not t.tgisinternal {filter} \
           and has_table_privilege(c.oid, '{}') \
         order by t.tgname",
        ops.pg_table_visibility_privileges()
    ))
}

// ==================== PG 族：用户 / 权限 / 角色 ====================

/// PG 的角色与用户是同一张表：**不能登录**的就是纯角色（rolcanlogin = false），
/// 它们只出现在「角色」页签里，不进用户清单。
///
/// `isSuper` 必须加双引号：PG 会把未加引号的别名折成 `issuper`，前端读 `row.isSuper`
/// 就永远拿不到（超级用户标签不显示）。
fn pg_users() -> Meta {
    Meta::Sql(
        "select rolname as name, rolsuper as \"isSuper\" from pg_roles \
         where rolcanlogin order by rolname"
            .to_string(),
    )
}

/// 这一条原来是从 SQL Server 那份**抄过来的**，两处语法在 PG 上都不成立：
///   · `'a' + 'b'`：PG 的字符串拼接是 `||`，`+` 只有数值有 —— 直接报
///     `operator does not exist: unknown + text`（42883），整条详情查询失败；
///   · `isnull(x, '')`：那是 SQL Server 的函数，PG 只有 `coalesce`（42883 同名错误）。
/// 结果是「PG 连用户清单都看得到、点进去却是没有找到用户」。
fn pg_user_info(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
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
         where r.rolname = {} \
         group by r.rolname, r.rolsuper, r.rolcanlogin, r.rolcreatedb",
        ops.literal(name)
    ))
}

/// 模式（USAGE/CREATE）/ 表 / 例程（EXECUTE）/ 库四层，各来自一张 information_schema 视图
/// 或 `pg_database` 的 ACL。grantee 是**裸角色名**（没有 MySQL 的 `'u'@'h'` 形状）。
/// information_schema 只列「显式授给这个角色」的权限：通过角色继承来的不在内 ——
/// 与 SUPERUSER 全能也不在内，如实展示。
///
/// 整句要压在一个 TCP 段以内（理由见 `mysql_user_grants` 的 VPN 实测注释）。
fn pg_user_grants(ops: &dyn DialectOps, user: &str) -> Meta {
    let who = ops.literal(user);
    Meta::Sql(format!(
        "select 'schema' as scope, privilege_type as privilege, object_name as db_name, '' as table_name \
         from information_schema.usage_privileges where grantee = {who} and object_type = 'SCHEMA' \
           and object_name not in ('information_schema','pg_catalog') \
         union all select 'table', privilege_type, table_schema, table_name \
         from information_schema.role_table_grants where grantee = {who} \
           and table_schema not in ('information_schema','pg_catalog') \
         union all select case when p.prokind = 'p' then 'procedure' else 'function' end as scope, \
           g.privilege_type as privilege, g.specific_schema as db_name, g.routine_name as table_name \
         from information_schema.role_routine_grants g join pg_proc p on p.proname = g.routine_name \
         join pg_namespace pn on pn.oid = p.pronamespace and pn.nspname = g.specific_schema \
         where g.grantee = {who} and g.specific_schema not in ('information_schema','pg_catalog') \
         union all select 'database' as scope, a.privilege_type as privilege, d.datname as db_name, '' as table_name \
         from pg_database d cross join lateral aclexplode(coalesce(d.datacl, acldefault('d', d.datdba))) a \
         join pg_roles r on r.oid = a.grantee where r.rolname = {who} and not d.datistemplate"
    ))
}

/// 成员关系在 `pg_auth_members`（member = 被授主体，roleid = 被授的角色）。
/// **information_schema 里查不到这一层** —— `role_usage_grants` 只讲 USAGE 权限、
/// `enabled_roles` 只看当前会话，所以 PG 的「角色」页签原来一个勾都没有
/// （授过的角色既看不见也撤不掉）。与 MySQL 一样单独一条短查询：不塞进 user_grants。
fn pg_user_roles(ops: &dyn DialectOps, user: &str) -> Meta {
    Meta::Sql(format!(
        "select r.rolname as role_name, '' as role_host from pg_auth_members m \
         join pg_roles r on r.oid = m.roleid \
         join pg_roles u on u.oid = m.member where u.rolname = {} order by 1",
        ops.literal(user)
    ))
}

fn pg_role_info(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
        "select rolname as name, '' as host from pg_roles where rolname = {}",
        ops.literal(name)
    ))
}

/// information_schema 只显示「当前用户是授予者/被授予者」的那部分，读别的角色的权限永远是空
/// —— 必须直接解 ACL 数组（aclexplode）。`coalesce(acl, acldefault(...))`：ACL 为 NULL =
/// 从没单独授过，此时按"只有属主有权限"的默认 ACL 展开（角色是属主时才会命中）。
/// 注意：模式/表两级用的是**当前库**的目录（pg_namespace / pg_class），其它库里的对象
/// 要在那个库上打开才看得到；库级（pg_database）是全局的。
fn pg_role_grants(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
        "select 'schema' as scope, a.privilege_type as privilege, n.nspname as db_name, '' as table_name \
           from pg_namespace n cross join lateral aclexplode(coalesce(n.nspacl, acldefault('n', n.nspowner))) a \
           join pg_roles r on r.oid = a.grantee \
          where r.rolname = {who} and n.nspname not in ('information_schema','pg_catalog') \
         union all \
         select 'table' as scope, a.privilege_type, n.nspname, c.relname \
           from pg_class c join pg_namespace n on n.oid = c.relnamespace \
           cross join lateral aclexplode(coalesce(c.relacl, acldefault('r', c.relowner))) a \
           join pg_roles r on r.oid = a.grantee \
          where r.rolname = {who} and c.relkind in ('r','v','m','p','f') \
            and n.nspname not in ('information_schema','pg_catalog') \
         union all \
         select 'database' as scope, a.privilege_type, d.datname, '' \
           from pg_database d cross join lateral aclexplode(coalesce(d.datacl, acldefault('d', d.datdba))) a \
           join pg_roles r on r.oid = a.grantee \
          where r.rolname = {who} and not d.datistemplate",
        who = ops.literal(name)
    ))
}

/// 纯角色 = 不能登录的那些；`pg_*` 是系统预置角色（pg_monitor / pg_read_all_data…），
/// 它们在上面的「角色」页有专门一栏（pgRoles），不该混进"可删除的角色"清单。
fn pg_role_inventory() -> Meta {
    Meta::Sql(
        "select rolname as role_name, '' as role_host from pg_roles \
         where not rolcanlogin and left(rolname, 3) <> 'pg_'"
            .to_string(),
    )
}

// ==================== 用户增删改的拼装件 ====================

/// PostgreSQL / Kingbase 的用户增删改 + 授权撤销（两方言一字不差，共用一份）。
///
/// PG 的「用户」就是带 login 的角色：`create role … login`，授权与撤销也是同一套
/// `grant … on schema|table|function` 语句。MySQL 语境的 host 在这里没有意义。
fn pg_user_action(
    ops: &dyn DialectOps,
    action: &str,
    user: &str,
    password: &str,
    roles: &[String],
    privileges: &UserPrivileges,
    revokes: &[GrantRevoke],
    attrs: &UserAttrs,
) -> Meta {
    let u = ops.quote(user);
    let pwd = ops.literal(password);
    let role_parts: Vec<String> = roles.iter().map(|role| ops.quote(role)).collect();
    // PG 的「用户」就是带 login 的角色（MySQL 语境的 host 在这里没有意义，忽略）。
    // 细粒度授权（模式 / 表 / 例程）与撤销：create 与 alter 共用一套拼法 ——
    // 模式 `grant usage on schema "s" to "u"`；表 `on table "s"."t"`；
    // 例程 `on function|procedure "s"."r"`（PG 11+ 签名可省略，同名歧义由数据库报错）。
    // 权限名已在 meta.rs 过白名单（is_pg_schema_priv / is_pg_table_priv / EXECUTE）。
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
                ops.quote(db)
            ));
        }
        for (schema, items) in &privileges.databases {
            if schema.trim().is_empty() || items.is_empty() {
                continue;
            }
            pg_parts.push(format!(
                "grant {} on schema {} to {u}",
                items.join(", ").to_lowercase(),
                ops.quote(schema)
            ));
        }
        for (schema, table, items) in &privileges.tables {
            if schema.trim().is_empty() || table.trim().is_empty() || items.is_empty() {
                continue;
            }
            pg_parts.push(format!(
                "grant {} on table {}.{} to {u}",
                items.join(", ").to_lowercase(),
                ops.quote(schema),
                ops.quote(table)
            ));
        }
        for (schema, routine, kind, items) in &privileges.routines {
            if schema.trim().is_empty() || routine.trim().is_empty() || items.is_empty() {
                continue;
            }
            let object_type = if kind.eq_ignore_ascii_case("procedure") {
                "procedure"
            } else {
                "function"
            };
            pg_parts.push(format!(
                "grant {} on {object_type} {}.{} to {u}",
                items.join(", ").to_lowercase(),
                ops.quote(schema),
                ops.quote(routine)
            ));
        }
        for r in revokes {
            match r.scope.as_str() {
                "schema" if !r.owner.trim().is_empty() => {
                    pg_parts.push(format!(
                        "revoke {} on schema {} from {u}",
                        r.privilege.to_lowercase(),
                        ops.quote(&r.owner)
                    ));
                }
                // 库级撤销：`revoke connect on database "db" from "u"`
                "database" if !r.owner.trim().is_empty() => {
                    pg_parts.push(format!(
                        "revoke {} on database {} from {u}",
                        r.privilege.to_lowercase(),
                        ops.quote(&r.owner)
                    ));
                }
                "table" if !r.owner.trim().is_empty() && !r.object.trim().is_empty() => {
                    pg_parts.push(format!(
                        "revoke {} on table {}.{} from {u}",
                        r.privilege.to_lowercase(),
                        ops.quote(&r.owner),
                        ops.quote(&r.object)
                    ));
                }
                // 角色成员关系（「服务器权限」页里那些角色项）：`revoke "r" from "u"`。
                // 少了这条，取消勾选一个角色会**静默无效** —— 界面显示已取消、
                // 服务器上还在（角色那一层就永远撤不掉）。
                "role" if !r.privilege.trim().is_empty() => {
                    pg_parts.push(format!("revoke {} from {u}", ops.quote(&r.privilege)));
                }
                "routine" | "function" if !r.owner.trim().is_empty() && !r.object.trim().is_empty() => {
                    let object_type = if r.kind.eq_ignore_ascii_case("procedure") {
                        "procedure"
                    } else {
                        "function"
                    };
                    pg_parts.push(format!(
                        "revoke {} on {object_type} {}.{} from {u}",
                        r.privilege.to_lowercase(),
                        ops.quote(&r.owner),
                        ops.quote(&r.object)
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
            flags.push(if attrs.can_login.unwrap_or(true) {
                "login"
            } else {
                "nologin"
            });
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

// ==================== 建表语句的拼装件 ====================

/// PostgreSQL / Kingbase 的建表语句查询（两方言一字不差，共用一份）。
///
/// `format_type` 直接给出带长度/精度的完整类型、`pg_get_expr` 还原默认值；
/// 视图用 `pg_get_viewdef`。**模式名必须显式替掉 current_schema()**：会话默认
/// 模式不可信（见 trait `ddl` 的文档）。
fn pg_ddl(ops: &dyn DialectOps, table: &str, schema: Option<&str>) -> Meta {
    let lit = ops.literal(table);
    let schema_lit = schema
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| ops.literal(s));
    let ns = match &schema_lit {
        Some(s) => s.clone(),
        None => "current_schema()".to_string(),
    };
    // PostgreSQL：`format_type` 直接给出带长度/精度的完整类型、`pg_get_expr` 还原默认值；
    // 视图用 `pg_get_viewdef`
    Meta::Sql(format!(
        "select 'create view ' || quote_ident(relname) || ' as' || chr(10) || pg_get_viewdef(oid) as ddl \
         from pg_class where relname = {lit} and relkind = 'v' \
           and relnamespace = (select oid from pg_namespace where nspname = {ns}) \
         union all \
         select 'create table ' || quote_ident(c.relname) || ' (' || chr(10) || \
         string_agg( \
           '  ' || quote_ident(a.attname) || ' ' || format_type(a.atttypid, a.atttypmod) || \
           case when a.attnotnull then ' not null' else '' end || \
           case when d.adbin is null then '' \
                else ' default ' || pg_get_expr(d.adbin, d.adrelid) end, \
           ',' || chr(10) order by a.attnum) || chr(10) || ');' as ddl \
         from pg_class c \
         join pg_attribute a on a.attrelid = c.oid and a.attnum > 0 and not a.attisdropped \
         left join pg_attrdef d on d.adrelid = c.oid and d.adnum = a.attnum \
         where c.relname = {lit} and c.relkind = 'r' \
           and c.relnamespace = (select oid from pg_namespace where nspname = {ns}) \
         group by c.relname"
    ))
}

// ==================== 对象定义的拼装件 ====================

/// PG / Kingbase 的对象定义查询（两方言一字不差，共用一份）。
/// `pg_get_viewdef` / `pg_get_functiondef` / `pg_get_triggerdef` 就是官方入口；
/// **模式名必须显式替掉会话默认**（空串 = 保持原行为，不动没给模式的调用方）。
fn pg_object_source(ops: &dyn DialectOps, object_type: &str, name: &str, schema: Option<&str>) -> Meta {
    let name_literal = ops.literal(name);
    let schema_lit = schema
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| ops.literal(s));
    let ns_filter = match &schema_lit {
        Some(s) => format!(" and n.nspname = {s}"),
        None => String::new(),
    };
    // pg_proc 那条没有 n 别名，单独一个（按 pronamespace 比）
    let proc_ns_filter = match &schema_lit {
        Some(s) => format!(" and p.pronamespace = (select oid from pg_namespace where nspname = {s})"),
        None => String::new(),
    };
    match object_type {
        "view" => Meta::Sql(format!(
            "select pg_get_viewdef(c.oid) as definition from pg_class c \
             join pg_namespace n on n.oid = c.relnamespace \
             where c.relname = {name_literal} and n.nspname not in ('pg_catalog','information_schema'){ns_filter}"
        )),
        "procedure" | "function" => Meta::Sql(format!(
            "select pg_get_functiondef(p.oid) as definition from pg_proc p \
             where p.proname = {name_literal}{proc_ns_filter} limit 1"
        )),
        // PG 的触发器定义（含 `ON <表>` 与 WHEN 条件）
        "trigger" => Meta::Sql(format!(
            "select pg_get_triggerdef(t.oid) as definition from pg_trigger t \
             join pg_class c on c.oid = t.tgrelid \
             join pg_namespace n on n.oid = c.relnamespace \
             where t.tgname = {name_literal} and not t.tgisinternal \
               and n.nspname not in ('pg_catalog','information_schema'){ns_filter}"
        )),
        // PG 没有 MySQL 那种事件调度器
        "event" => Meta::Absent,
        _ => Meta::Unwritten,
    }
}

/// PG 系授权回显行 → GRANT 文本（PostgresOps / KingbaseOps 共用）。
fn pg_grant_line_text<T: DialectOps>(
    ops: &T,
    scope: &str,
    privilege: &str,
    owner: &str,
    object: &str,
    account_user: &str,
) -> Option<String> {
    let who = ops.quote(account_user);
    match scope {
        // 库级（CONNECT / CREATE / TEMPORARY ON DATABASE）：owner 列是库名
        "database" if !owner.is_empty() => Some(format!(
            "GRANT {privilege} ON DATABASE {} TO {who};",
            ops.quote(owner)
        )),
        "schema" if !owner.is_empty() => Some(format!(
            "GRANT {privilege} ON SCHEMA {} TO {who};",
            ops.quote(owner)
        )),
        "table" if !owner.is_empty() && !object.is_empty() => Some(format!(
            "GRANT {privilege} ON TABLE {}.{} TO {who};",
            ops.quote(owner),
            ops.quote(object)
        )),
        "function" | "procedure" if !owner.is_empty() && !object.is_empty() => {
            let object_type = if scope == "procedure" {
                "PROCEDURE"
            } else {
                "FUNCTION"
            };
            Some(format!(
                "GRANT {privilege} ON {object_type} {}.{} TO {who};",
                ops.quote(owner),
                ops.quote(object)
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::kb_catalog_sql;

    /// 改名只按**整词**替换：`pg_index` 不许把 `pg_indexes` 改坏，也不许碰用户自己的
    /// 标识符 —— 这条防线坏了就等于去改用户的表名。
    #[test]
    fn 改名只动目录名() {
        let out = kb_catalog_sql("select * from pg_indexes where t = 'pg_index' and c = my_pg_class");
        assert!(out.contains("from pg_indexes"), "{out}");
        assert!(out.contains("'sys_index'"), "{out}");
        assert!(out.contains("my_pg_class"), "{out}");
    }

    /// 目录表 / 函数 / 模式字面量三类都要改名：监控总览、容量排行、同步行数估算、
    /// 比对外键定义四处都复用这个函数，漏一类就是那一路「关系不存在」。
    #[test]
    fn 目录表与函数都改成_sys_前缀() {
        let out = kb_catalog_sql(
            "select c.relname, pg_total_relation_size(c.oid), pg_indexes_size(c.oid), \
             pg_terminate_backend(1) from pg_class c \
             join pg_catalog.pg_namespace n on n.oid = c.relnamespace \
             where n.nspname not in ('pg_catalog','information_schema')",
        );
        for word in [
            "from sys_class",
            "sys_total_relation_size(",
            "sys_indexes_size(",
            "sys_terminate_backend(",
            "sys_catalog.sys_namespace",
            "'sys_catalog'",
        ] {
            assert!(out.contains(word), "缺 {word}: {out}");
        }
        assert!(!out.contains("pg_"), "还剩 pg_ 目录名: {out}");
    }
}
