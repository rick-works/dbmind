//! 其余单类型方言：DB2 / SQLite / H2 / Derby / DuckDB，以及非 SQL 与兜底。

use super::*;

use crate::api::dialect::{h2_user, is_h2_table_priv};

impl DialectOps for Db2Ops {
    fn is_schema_aware(&self) -> bool {
        true
    }

    fn schemas_sql(&self) -> Option<&'static str> {
        Some(dbmind_core::scope_sql::DB2_SCHEMAS)
    }

    fn tables_in_schema_sql(&self, schema: &str) -> Option<String> {
        let s = schema.replace('\'', "''");
        Some(format!(
            "select tabname as name from syscat.tables \
             where tabschema = '{s}' and type = 'T' order by tabname"
        ))
    }

    fn object_source(&self, object_type: &str, name: &str, _schema: Option<&str>) -> Meta {
        // syscat 里的 TEXT 列就是定义原文（视图/例程/触发器都有）
        let name_literal = self.literal(name);
        match object_type {
            "view" => Meta::Sql(format!(
                "select text as definition from syscat.views \
                 where viewschema = current schema and viewname = upper({name_literal})"
            )),
            "procedure" | "function" => Meta::Sql(format!(
                "select text as definition from syscat.routines \
                 where routineschema = current schema and routinename = upper({name_literal})"
            )),
            "trigger" => Meta::Sql(format!(
                "select text as definition from syscat.triggers \
                 where trigschema = current schema and trigname = upper({name_literal})"
            )),
            "event" => Meta::Absent,
            _ => Meta::Unwritten,
        }
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

    fn table_action(&self, table: &str, action: &str, new_name: Option<&str>) -> Meta {
        let t = self.quote(table);
        match action {
            "clear" => Meta::Sql(format!("delete from {t}")),
            // DB2 的 TRUNCATE 必须显式写 IMMEDIATE：`truncate table t` 直接
            // SQLCODE=-104 SQLSTATE=42601（真机实测，报错信息里就提示
            // END-OF-STATEMENT ...;IMMEDIATE）
            "truncate" => Meta::Sql(format!("truncate table {t} immediate")),
            "drop" => Meta::Sql(format!("drop table {t}")),
            // DB2 有自己的 RENAME TABLE
            "rename" => match new_name {
                Some(new_name) => Meta::Sql(format!("rename table {t} to {}", self.quote(new_name))),
                None => Meta::Unwritten,
            },
            _ => Meta::Unwritten,
        }
    }

    fn users(&self) -> Meta {
        db2_users()
    }

    fn user_info(&self, name: &str) -> Meta {
        db2_user_info(self, name)
    }

    fn procedures(&self, _schema: Option<&str>) -> Meta {
        db2_procedures()
    }

    fn triggers(&self, _schema: Option<&str>) -> Meta {
        db2_triggers()
    }

    fn table_options_sql(&self, _database: &str) -> Option<String> {
        Some(DB2_TABLE_OPTIONS.to_string())
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        Some(DB2_INDEXES)
    }

    fn visible_tables(&self) -> Option<&'static str> {
        Some(DB2_VISIBLE_TABLES)
    }

    fn upsert_sql(
        &self,
        table: &str,
        columns: &[String],
        key_indexes: &[usize],
        rows: &[Vec<String>],
    ) -> Option<String> {
        let p = upsert_parts(self, columns, key_indexes, rows)?;
        Some(merge_values_upsert(&p, table, false))
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        // DB2：syscat.columns 的 typename/length/scale。
        // 列注释也要在这里带出来（`remarks`）：表结构页的「注释」列读的是 `/columns`，
        // 只靠 `table_comments`（SQL 结果表头那条）补不上 —— 编辑表结构时注释格永远是空的。
        Meta::Sql(format!(
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
        ))
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        // syscat.tables 只有 NPAGES（数据页）/ FPAGES（总页数，含索引与溢出）/ CARD（行数）。
        // 原句用的 data_object_pages / index_object_pages 是别的库的列名，
        // DB2 上直接 SQLCODE=-206 SQLSTATE=42703（真机实测）。4KB 页 ⇒ 页数 * 4 / 1024 = MB。
        Some(
            "select count(*) as tables, \
             cast(coalesce(sum(fpages), 0) * 4 / 1024 as decimal(12,1)) || ' MB' as sizeText \
             from syscat.tables where tabschema = current schema and type = 'T'",
        )
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        // 列注释在 **`syscat.columns.remarks`** 里，不是 `syscat.colcomments`
        // —— 后者在 DB2 11.5 上直接 `SQLCODE=-204（未定义的对象名）`，
        // 整条查询失败 ⇒ 列注释永远为空（真机实测）。
        // 表名双向折大写：带引号建的小写表名也要能对上。
        Meta::Sql(format!(
            "select colname as column_name, coalesce(remarks, '') as comment \
             from syscat.columns \
             where tabschema = current schema and upper(tabname) = upper({quoted})"
        ))
    }
    fn folds_ident_upper(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "db2"
    }

    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::DB2
    }

    fn sync_object_concurrency(&self) -> usize {
        3
    }

    fn table_rows_estimate_sql(&self, _database: &str, schema: &str) -> Option<String> {
        // `CARD` 是统计信息里的行数估算
        let schema = schema.to_uppercase().replace('\'', "''");
        Some(format!(
            "select tabname as t, card as n from syscat.tables where tabschema = '{schema}'"
        ))
    }

    fn column_type_names(&self) -> &'static [&'static str] {
        editor::DB2_COLUMN_TYPES
    }
    fn supports_native_upsert(&self) -> bool {
        true
    }
    fn limit_clause(&self, offset: u64, size: u64) -> String {
        format!("offset {offset} rows fetch next {size} rows only")
    }
    fn version_sql(&self) -> Option<&'static str> {
        Some("select service_level as v from sysibm.sysversions fetch first 1 rows only")
    }
    fn direct_ref_depth(&self) -> DirectRefDepth {
        DirectRefDepth::Schema
    }
    fn clear_table_sql(&self, table: &str) -> String {
        // 裸 `truncate table x` 报 SQLCODE=-104（expected "IMMEDIATE"）
        format!("truncate table {table} immediate")
    }
}

impl DialectOps for SqliteOps {
    fn object_source(&self, object_type: &str, name: &str, _schema: Option<&str>) -> Meta {
        let _ = object_type;
        // SQLite 的表/视图/触发器定义都在 sqlite_master.sql 里
        Meta::Sql(format!(
            "select sql from sqlite_master where name = {}",
            self.literal(name)
        ))
    }

    fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        let _ = schema;
        Meta::Sql(format!(
            "select sql from sqlite_master where type in ('table','view') and name = {}",
            self.literal(table)
        ))
    }

    fn table_action(&self, table: &str, action: &str, new_name: Option<&str>) -> Meta {
        let t = self.quote(table);
        match action {
            "clear" => Meta::Sql(format!("delete from {t}")),
            // SQLite 没有 TRUNCATE，截断退化成 DELETE（一样会清空数据，这点必须说清）
            "truncate" => Meta::Sql(format!("delete from {t}")),
            "drop" => Meta::Sql(format!("drop table {t}")),
            "rename" => match new_name {
                Some(new_name) => Meta::Sql(format!("alter table {t} rename to {}", self.quote(new_name))),
                None => Meta::Unwritten,
            },
            _ => Meta::Unwritten,
        }
    }

    fn procedures(&self, _schema: Option<&str>) -> Meta {
        Meta::Absent
    }

    fn triggers(&self, _schema: Option<&str>) -> Meta {
        sqlite_triggers()
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        Some(SQLITE_INDEXES)
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
        matches!(kind, "view" | "trigger")
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        // SQLite：声明类型原样读回（`varchar(50)` 就在表结构里）。
        //
        // 自增的判定标准是「**单列** INTEGER PRIMARY KEY」—— 那是 rowid 的别名，
        // 插入时留空即自动赋值。复合主键里的 INTEGER 不是别名，不能算自增。
        // （`AUTOINCREMENT` 只影响 rowid 复用，不改变「是不是自增」这件事。）
        Meta::Sql(format!(
            "select name as column_name, type as type_text, '' as comment, \
             case when upper(trim(coalesce(type, ''))) = 'INTEGER' and pk = 1 \
                   and (select count(*) from pragma_table_info({quoted}) where pk > 0) = 1 \
                  then 1 else 0 end as is_auto, \
             cid as ordinal \
             from pragma_table_info({quoted})"
        ))
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        // 文件型库没有「占用空间」的概念，如实给「—」，不编一个 0
        Some(
            "select (select count(*) from sqlite_master \
             where type = 'table' and name not like 'sqlite_%') as tables, '—' as sizeText",
        )
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        let _ = quoted;
        // 没有列注释这个概念：给 Absent（空数组）而不是「未实现」，
        // 语句治理那边会退化成按字段名判断，这是事实，不是缺陷。
        Meta::Absent
    }
    fn bool_literal(&self, truth: bool) -> &'static str {
        if truth {
            "1"
        } else {
            "0"
        }
    }
    fn folds_ident_lower(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "sqlite"
    }

    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::SQLITE
    }

    fn sync_object_concurrency(&self) -> usize {
        // 会话池只有一条连接，并行只会互相排队
        1
    }

    fn drop_table_sql(&self, quoted_table: &str) -> String {
        format!("drop table if exists {quoted_table}")
    }

    fn column_type_names(&self) -> &'static [&'static str] {
        editor::SQLITE_COLUMN_TYPES
    }
    fn supports_native_upsert(&self) -> bool {
        true
    }
    fn version_sql(&self) -> Option<&'static str> {
        Some("select sqlite_version() as v")
    }
    fn clear_table_sql(&self, table: &str) -> String {
        format!("delete from {table}")
    }
}

impl DialectOps for H2Ops {
    // 只有表级授权这一层（没有全局/库级）
    fn grant_input_scopes(&self) -> &'static [&'static str] {
        &["tablePrivileges", "revoke"]
    }

    fn object_privilege_check(&self) -> fn(&str) -> bool {
        crate::api::dialect::is_h2_table_priv
    }

    fn is_revocable(&self, scope: &str, privilege: &str, owner: &str, object: &str, kind: &str) -> bool {
        let _ = (kind, object);
        let scope = if scope.is_empty() { "global" } else { scope };
        match scope {
            "table" | "object" | "schema" => {
                crate::api::dialect::is_h2_table_priv(privilege) && !owner.is_empty()
            }
            "role" => crate::api::dialect::is_role_ref(privilege),
            "global" | "server-role" => crate::api::dialect::is_mysql_privilege(privilege),
            _ => false,
        }
    }

    fn revoke_missing_hint(&self) -> &'static str {
        "库/表名"
    }

    // H2 只有表级一层（schema.table），账号也没有 host —— 落到 MySQL 通用分支会
    // 拼成 `TO '""'@''`（实测），既不是合法 H2 语法，也没法照着执行。
    // 空用户名（H2 默认用户）统一写成空标识符 `""`。
    fn grant_line_text(
        &self,
        scope: &str,
        privilege: &str,
        owner: &str,
        object: &str,
        account_user: &str,
        _account_host: &str,
    ) -> Option<String> {
        let raw = account_user.trim();
        let who = if raw.is_empty() || raw == "\"\"" || raw == "(default)" {
            "\"\"".to_string()
        } else {
            self.quote(raw)
        };
        match scope {
            "table" if !owner.is_empty() && !object.is_empty() => Some(format!(
                "GRANT {privilege} ON {}.{} TO {who};",
                self.quote(owner),
                self.quote(object)
            )),
            "global" => Some(format!("GRANT {privilege} TO {who};")),
            _ => None,
        }
    }

    fn object_source(&self, object_type: &str, name: &str, _schema: Option<&str>) -> Meta {
        // 定义都在 information_schema 里（比较用**字符串字面量**，
        // 不能用标识符 —— `where table_name = upper(<标识符>)` 是把列引用当值用了）
        let name_literal = self.literal(name);
        match object_type {
            "view" => Meta::Sql(format!(
                "select view_definition as definition from information_schema.views \
                 where table_name = upper({name_literal})"
            )),
            "procedure" | "function" => Meta::Sql(format!(
                "select routine_definition as definition from information_schema.routines \
                 where routine_name = upper({name_literal})"
            )),
            "event" => Meta::Absent,
            _ => Meta::Unwritten,
        }
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

    /// H2 2.x **没有** `information_schema.tables.sql`（那是 1.x 的字段），老写法在
    /// 2.3 上直接报 `Column "SQL" not found`（实测）。`SCRIPT NODATA` 是 H2 官方的
    /// 导出入口：它是一条**命令**而非常量表（`select script from script nodata` 报
    /// Table "SCRIPT" not found），而且 `table <名>` 这个过滤实测被忽略（照样返回
    /// 整库语句）—— 所以只发命令，按表名筛选放在 Rust 侧（`build_table_ddl`）。
    fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        let _ = (table, schema);
        Meta::Sql("script nodata".to_string())
    }

    fn user_action_sql(
        &self,
        action: &str,
        _as_role: bool,
        user: &str,
        password: &str,
        _host: &str,
        _roles: &[String],
        privileges: &UserPrivileges,
        revokes: &[GrantRevoke],
        _reset_privileges: bool,
        _attrs: &UserAttrs,
    ) -> Meta {
        // H2：`create user` 必须带口令；改密是 `alter user … set password`。
        // 权限矩阵（表级 GRANT）还没接进来 —— 调用方收到权限入参会先明确拒绝。
        let name = h2_user(user);
        // 空用户名（H2 的默认用户）必须写成空标识符 `""`：`quote("")` 返回空串，
        // 拼出来是 `grant … to `（实测 42001 SyntaxError，缺 identifier），
        // 连改密/删除也一样会拼坏。
        let u = if name.is_empty() {
            "\"\"".to_string()
        } else {
            self.quote(&name)
        };
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

    fn users(&self) -> Meta {
        h2_users()
    }

    fn user_info(&self, name: &str) -> Meta {
        h2_user_info(self, name)
    }

    fn user_grants(&self, user: &str, _host: &str) -> Meta {
        h2_user_grants(self, user)
    }

    fn schemas_sql(&self) -> Option<&'static str> {
        Some(H2_SCHEMAS)
    }

    fn is_schema_aware(&self) -> bool {
        true
    }

    fn tables_in_schema_sql(&self, schema: &str) -> Option<String> {
        let s = schema.replace('\'', "''");
        Some(format!(
            "select table_name as name from information_schema.tables \
             where table_schema = '{s}' and table_type = 'BASE TABLE' order by table_name"
        ))
    }

    fn procedures(&self, _schema: Option<&str>) -> Meta {
        h2_procedures()
    }

    fn table_options_sql(&self, _database: &str) -> Option<String> {
        Some(H2_TABLE_OPTIONS.to_string())
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        Some(H2_INDEXES)
    }

    fn visible_tables(&self) -> Option<&'static str> {
        Some(H2_VISIBLE_TABLES)
    }

    fn upsert_sql(
        &self,
        table: &str,
        columns: &[String],
        key_indexes: &[usize],
        rows: &[Vec<String>],
    ) -> Option<String> {
        let p = upsert_parts(self, columns, key_indexes, rows)?;
        Some(format!(
            "merge into {table} ({}) key ({}) values {}",
            p.col_list,
            p.keys.join(", "),
            p.tuples
        ))
    }
    fn supports_drop_object(&self, kind: &str) -> bool {
        matches!(kind, "view")
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        // H2：information_schema.columns 有长度/精度，自己拼回 `varchar(50)`。
        // 长度大于 100 万的不当「长度」用（CLOB 一类会报 2^31-1，拼进类型文本没意义）。
        // 列注释在 H2 的 information_schema.columns 里叫 `remarks`（老的 1.4 与 2.x 都有），
        // 同样**曾经漏取**：界面「注释」列可见却永远空着。
        //
        // **精度只对 numeric / decimal 拼**：H2 给**所有**数值类型都填了
        // numeric_precision/numeric_scale（INTEGER=32,0、BIGINT=64,0、BOOLEAN=1,0…），
        // 早先不看类型就拼，得到 `integer(32,0)` 这种根本执行不了的「类型」。
        // 更麻烦的是它和界面的「现状基线」对不上：编辑器拆出 `INTEGER` + 长度 32，
        // 再拼回去是 `integer(32)` ≠ `integer(32,0)` ⇒ **一打开表就被判成「改 3」**
        // （用户反馈：什么都没改，预览里却有 3 条 ALTER）。
        Meta::Sql(format!(
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
        ))
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        // 表数能拿到；占用空间不在 information_schema 里
        Some(
            "select count(*) as tables, '—' as sizeText \
             from information_schema.tables where table_schema = current_schema",
        )
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        // 表名**双向折大写**再比：H2 把未加引号的标识符存成大写，而带引号建的表
        // （`"my_table"`）就是小写 —— 只写 `table_name = upper(...)` 时后者一行都匹配不上。
        Meta::Sql(format!(
            "select column_name as column_name, coalesce(remarks, '') as comment \
             from information_schema.columns \
             where upper(table_name) = upper({quoted})"
        ))
    }
    fn folds_ident_upper(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "h2"
    }

    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::H2
    }

    fn sync_object_concurrency(&self) -> usize {
        1
    }

    fn drop_table_sql(&self, quoted_table: &str) -> String {
        format!("drop table if exists {quoted_table}")
    }

    fn column_type_names(&self) -> &'static [&'static str] {
        editor::H2_COLUMN_TYPES
    }
    fn supports_native_upsert(&self) -> bool {
        true
    }
    fn version_sql(&self) -> Option<&'static str> {
        Some("select h2version() as v")
    }
}

impl DialectOps for DerbyOps {
    fn is_schema_aware(&self) -> bool {
        true
    }

    fn schemas_sql(&self) -> Option<&'static str> {
        Some(dbmind_core::scope_sql::DERBY_SCHEMAS)
    }

    fn tables_in_schema_sql(&self, schema: &str) -> Option<String> {
        let s = schema.replace('\'', "''");
        Some(format!(
            "select t.tablename as name from sys.systables t \
             join sys.sysschemas sc on t.schemaid = sc.schemaid \
             where sc.schemaname = '{s}' and t.tabletype = 'T' order by t.tablename"
        ))
    }

    fn object_source(&self, object_type: &str, name: &str, _schema: Option<&str>) -> Meta {
        let name_literal = self.literal(name);
        match object_type {
            // Derby 10.15 起 `sys.sysviews` 只有 `tableid` + `viewdefinition`
            // （`viewname` 已不在该表里，按它查会报「Column 'VIEWNAME' ...」——实测），
            // 视图名要连 `sys.systables` 取
            "view" => Meta::Sql(format!(
                "select v.viewdefinition as definition from sys.sysviews v \
                 join sys.systables t on t.tableid = v.tableid \
                 where t.tablename = upper({name_literal})"
            )),
            "trigger" => Meta::Sql(format!(
                "select triggerdefinition as definition from sys.systriggers \
                 where triggername = upper({name_literal})"
            )),
            // 例程在 Derby 里没有目录可查，只能保持「取不到」，如实 Absent
            "procedure" | "function" | "event" => Meta::Absent,
            _ => Meta::Unwritten,
        }
    }

    fn table_action(&self, table: &str, action: &str, new_name: Option<&str>) -> Meta {
        // Derby **没有** `ALTER TABLE … RENAME TO`（真机：`Syntax error: Encountered
        // "rename" at line 1, column 26`），路由与 MySQL 一样是独立的
        // `RENAME TABLE old TO new`（真机验过：改名后新表可查、旧表消失）
        if action == "rename" {
            return match new_name {
                Some(new_name) => Meta::Sql(format!(
                    "rename table {} to {}",
                    self.quote(table),
                    self.quote(new_name)
                )),
                None => Meta::Unwritten,
            };
        }
        let t = self.quote(table);
        match action {
            "clear" => Meta::Sql(format!("delete from {t}")),
            "truncate" => Meta::Sql(format!("truncate table {t}")),
            "drop" => Meta::Sql(format!("drop table {t}")),
            _ => Meta::Unwritten,
        }
    }

    fn users(&self) -> Meta {
        derby_users()
    }

    fn user_info(&self, name: &str) -> Meta {
        derby_user_info(self, name)
    }

    fn procedures(&self, _schema: Option<&str>) -> Meta {
        derby_procedures()
    }

    fn triggers(&self, _schema: Option<&str>) -> Meta {
        derby_triggers()
    }

    fn visible_tables(&self) -> Option<&'static str> {
        Some(DERBY_VISIBLE_TABLES)
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        Some(DERBY_INDEXES)
    }

    fn upsert_sql(
        &self,
        table: &str,
        columns: &[String],
        key_indexes: &[usize],
        rows: &[Vec<String>],
    ) -> Option<String> {
        let p = upsert_parts(self, columns, key_indexes, rows)?;
        Some(merge_values_upsert(&p, table, false))
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        // Derby：`sys.syscolumns` 的 COLUMNDATATYPE 是完整类型文本，但**带着可空性**
        // （实测出来是 `INTEGER NOT NULL`）—— 直接当类型名用会让结构编辑器把它
        // 当成一个类型（改任何属性都会生成 `... c integer not null not null`）。
        // 这里把结尾的 ` NOT NULL` 去掉，只留类型本身。
        // （现代 Derby 更推荐 SYSCS_UTIL，但那条要存储过程，读元数据不值当。）
        Meta::Sql(format!(
            "select columnname as column_name, \
             case when cast(columndatatype as varchar(256)) like '% NOT NULL' \
               then substr(cast(columndatatype as varchar(256)), 1, \
                           length(cast(columndatatype as varchar(256))) - 9) \
               else cast(columndatatype as varchar(256)) end as type_text \
             from sys.syscolumns where referenceid = \
             (select tableid from sys.systables where tablename = upper({quoted}))"
        ))
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        Some("select count(*) as tables, '—' as sizeText from sys.systables where tabletype = 'T'")
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        let _ = quoted;
        // 没有列注释这个概念：给 Absent（空数组）而不是「未实现」，
        // 语句治理那边会退化成按字段名判断，这是事实，不是缺陷。
        Meta::Absent
    }
    fn folds_ident_upper(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "derby"
    }

    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::DERBY
    }

    fn sync_object_concurrency(&self) -> usize {
        1
    }

    fn column_type_names(&self) -> &'static [&'static str] {
        editor::DERBY_COLUMN_TYPES
    }
    fn supports_native_upsert(&self) -> bool {
        true
    }
    fn upsert_row_limit(&self) -> usize {
        1000
    }
    fn limit_clause(&self, offset: u64, size: u64) -> String {
        format!("offset {offset} rows fetch next {size} rows only")
    }
    fn version_sql(&self) -> Option<&'static str> {
        Some("values syscs_util.syscs_get_database_property('derby.version')")
    }
    fn clear_table_sql(&self, table: &str) -> String {
        format!("delete from {table}")
    }
}

impl DialectOps for DuckDbOps {
    fn family(&self) -> &'static str {
        "duckdb"
    }

    fn is_schema_aware(&self) -> bool {
        true
    }

    fn schemas_sql(&self) -> Option<&'static str> {
        Some(dbmind_core::scope_sql::DUCKDB_SCHEMAS)
    }

    fn tables_in_schema_sql(&self, schema: &str) -> Option<String> {
        let s = schema.replace('\'', "''");
        Some(format!(
            "select table_name as name from information_schema.tables \
             where table_schema = '{s}' and table_type = 'BASE TABLE' order by table_name"
        ))
    }

    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::DUCKDB
    }

    fn sync_object_concurrency(&self) -> usize {
        1
    }

    fn column_type_names(&self) -> &'static [&'static str] {
        editor::DUCKDB_COLUMN_TYPES
    }
}

/// SAP HANA：标准 SQL + 双引号标识符（未加引号的标识符上折大写）。
/// 系统目录在 `SYS` schema（SYS.SCHEMAS / SYS.TABLES / SYS.TABLE_COLUMNS）。
/// 没有真机（实验室无 HANA 实例），只实现「能浏览、能查询」的最小面，
/// 其余能力靠保守默认（编辑类一律 generic = 只读），不误开点了会报错的开关。
impl DialectOps for SapHanaOps {
    fn family(&self) -> &'static str {
        "saphana"
    }

    fn folds_ident_upper(&self) -> bool {
        true
    }

    fn column_type_names(&self) -> &'static [&'static str] {
        editor::SAPHANA_COLUMN_TYPES
    }

    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::SAPHANA
    }

    fn schemas_sql(&self) -> Option<&'static str> {
        Some("select schema_name from sys.schemas order by schema_name")
    }

    fn tables_in_schema_sql(&self, schema: &str) -> Option<String> {
        let s = schema.replace('\'', "''");
        Some(format!(
            "select table_name as name, case when table_type like '%VIEW%' then 'VIEW' else 'TABLE' end as type \
             from sys.tables where schema_name = '{s}' order by table_name"
        ))
    }

    fn column_types_sql(&self, quoted: &str) -> Meta {
        // 表名进 SYS.TABLE_COLUMNS 时用目录里的真实大小写：HANA 的 TABLE_NAME 是大写，
        // 用户手写的表名可能是混合大小写 —— 双向 upper 比对最稳。
        Meta::Sql(format!(
            "select column_name as column_name, \
             case when data_type_name in ('NVARCHAR','VARCHAR','CHAR','NCHAR') \
                  then lower(data_type_name) || '(' || length || ')' \
                  when data_type_name in ('DECIMAL','DEC') \
                  then lower(data_type_name) || '(' || length || ',' || scale || ')' \
                  else lower(data_type_name) end as type_text, \
             coalesce(comments, '') as comment \
             from sys.table_columns where upper(table_name) = upper({quoted}) \
             order by position"
        ))
    }

    fn table_comments_sql(&self, quoted: &str) -> Meta {
        Meta::Sql(format!(
            "select column_name as column_name, coalesce(comments, '') as comment \
             from sys.table_columns where upper(table_name) = upper({quoted})"
        ))
    }

    fn table_action(&self, table: &str, action: &str, new_name: Option<&str>) -> Meta {
        let t = self.quote(table);
        match action {
            "clear" | "truncate" => Meta::Sql(format!("truncate table {t}")),
            "drop" => Meta::Sql(format!("drop table {t}")),
            "rename" => match new_name {
                Some(n) => Meta::Sql(format!("rename table {t} to {}", self.quote(n))),
                None => Meta::Unwritten,
            },
            _ => Meta::Unwritten,
        }
    }
}

impl DialectOps for NoSqlOps {
    fn family(&self) -> &'static str {
        "nosql"
    }
}

impl DialectOps for GenericOps {
    fn family(&self) -> &'static str {
        "generic"
    }
}

// ==================== 其余方言：表选项 / 索引 / 可见表 ====================

/// H2 / DB2 的 information_schema 都带 remarks（就是表注释）。
const H2_TABLE_OPTIONS: &str = "select table_name as table_name, coalesce(remarks, '') as comment \
     from information_schema.tables where table_schema = current_schema";

const DB2_TABLE_OPTIONS: &str = "select tabname as table_name, coalesce(remarks, '') as comment \
     from syscat.tables where tabschema = current schema and type = 'T'";

const SQLITE_INDEXES: &str = "select name, tbl_name as \"table\", sql from sqlite_master \
     where type = 'index' and name not like 'sqlite_autoindex%' order by name";

/// H2：列在 information_schema.index_columns；H2 的 GROUP_CONCAT 默认分隔符就是逗号。
/// isUnique 只能从索引类型名上认（H2 的 information_schema.indexes 没有唯一性列）——
/// 尽力而为，认不出就当非唯一，不猜别的。
const H2_INDEXES: &str = "select i.index_name as \"name\", i.table_name as \"table\", \
     case when i.index_type_name like '%UNIQUE%' then 1 else 0 end as \"isUnique\", \
     group_concat(c.column_name order by c.ordinal_position separator ',') as \"columns\" \
     from information_schema.indexes i \
     left join information_schema.index_columns c \
       on c.index_name = i.index_name and c.table_name = i.table_name \
     group by i.index_name, i.table_name, i.index_type_name \
     order by i.table_name, i.index_name";

/// DB2：列在 syscat.indexcoluse（colseq 是列序）。
/// **索引名这列在 SYSCAT.INDEXES 里叫 INDNAME，不是 INDEXNAME** —— 写成 INDEXNAME
/// 会整条查询 500（SQLCODE=-206 SQLSTATE=42703），「索引」页签跟着一片空白。
const DB2_INDEXES: &str = "select i.indname as \"name\", i.tabname as \"table\", \
     case when i.uniquerule = 'U' then 1 else 0 end as \"isUnique\", \
     listagg(c.colname, ',') within group (order by c.colseq) as \"columns\" \
     from syscat.indexes i \
     join syscat.indexcoluse c on c.indschema = i.indschema and c.indname = i.indname \
     where i.tabschema = current schema \
     group by i.indname, i.tabname, i.uniquerule \
     order by i.tabname, i.indname";

/// Derby **没有 syscat schema**（那是 DB2 的）：老写法在这里报
/// `Table/View 'SYSCAT.INDEXES' does not exist`（实测）。
/// 索引在 sys.sysconglomerates（isindex 标记），表名要连 sys.systables，
/// 且必须按 schema 过滤 —— 不然会把系统索引（SYSCONGLOMERATES_INDEX1 之类）全列出来。
/// 列：「包含字段」在 Derby 上**取不到** —— 索引列只在 SYS.SYSCONGLOMERATES.descriptor
/// 这个二进制描述符里，没有可查询的列。如实空着，别为了"看着有东西"去猜一个必然报错的
/// 查询（一旦报错，整个索引页签会 500，反而比空更难解释）。
const DERBY_INDEXES: &str = "select c.conglomeratename as \"name\", t.tablename as \"table\" \
     from sys.sysconglomerates c \
     join sys.systables t on t.tableid = c.tableid \
     join sys.sysschemas s on s.schemaid = t.schemaid \
     where c.isindex = true and s.schemaname = current schema \
     order by t.tablename, c.conglomeratename";

/// H2 2.x：`information_schema.tables` **没有属主列**，属主要从 `schemata.schema_owner`
/// 取；`table_privileges` 只记显式授权（属主不会在里面出现，所以必须并上属主/管理员）。
/// `is_admin` 在 2.3 是 BOOLEAN（写成 `= 1` 会报「BOOLEAN 与 INTEGER 不可比」——实测）。
const H2_VISIBLE_TABLES: &str = "select t.table_name as name from information_schema.tables t \
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
                    and p.grantee in ('PUBLIC', current_user)) )";

/// DB2：`SYSCAT.TABAUTH` 只有显式授权；属主在 `SYSCAT.TABLES.OWNER`。
/// 已知缺口：**组与角色的授权**不一定落在这张目录里（要枚举
/// `AUTH_LIST_GROUPS_FOR_AUTHID`），所以调用方把「过滤结果为空」当作"目录没覆盖"、
/// 保持原样 —— 详见 meta 里的求交逻辑。
const DB2_VISIBLE_TABLES: &str = "select t.tabname as name from syscat.tables t \
     where t.tabschema = current schema and ( \
       t.owner = current user \
       or exists (select 1 from syscat.tabauth a \
                  where a.tabschema = t.tabschema and a.tabname = t.tabname \
                    and a.grantee in ('PUBLIC', current user)) )";

/// Derby：`sys.systableperms` 只在打开 `derby.database.sqlAuthorization` 后才有行，
/// 属性为 NULL/FALSE 时**人人都有权**（默认安装就是这种），此时不能过滤。
/// 属主是「模式属主」`sys.sysschemas.authorizationid`（Derby 的 schema 与用户绑定）。
const DERBY_VISIBLE_TABLES: &str = "select t.tablename as name from sys.systables t \
     join sys.sysschemas s on s.schemaid = t.schemaid \
     where s.schemaname = current schema and ( \
       coalesce(syscs_util.syscs_get_database_property('derby.database.sqlAuthorization'), 'FALSE') <> 'TRUE' \
       or s.authorizationid = current_user \
       or exists (select 1 from sys.systableperms p \
                  where p.tableid = t.tableid \
                    and p.grantee in ('PUBLIC', current_user)) )";

// ==================== 其余方言：schema / 例程 / 触发器 ====================

/// H2：`INFORMATION_SCHEMA.SCHEMATA`（2.x）。H2 的「库」是 catalog（一个实例一个，
/// 名字就是连接里的 database，如 MAIN），表和授权都挂在 schema 上 ——
/// 用户管理的表级授权要选的就是这一层（PUBLIC / 业务 schema）。
const H2_SCHEMAS: &str = dbmind_core::scope_sql::H2_SCHEMAS;

fn h2_procedures() -> Meta {
    Meta::Sql(
        "select routine_name as \"name\", routine_type as \"routineType\" \
         from information_schema.routines order by routine_name"
            .to_string(),
    )
}

/// SQLite 没有存储过程这个概念 —— 空数组是事实（procedures 走默认的 Unwritten 之外，
/// 它单独覆盖成 Absent）；触发器则真的有，就在 sqlite_master 里。
fn sqlite_triggers() -> Meta {
    Meta::Sql(
        "select name, tbl_name as \"table\" from sqlite_master \
         where type = 'trigger' order by name"
            .to_string(),
    )
}

fn db2_procedures() -> Meta {
    Meta::Sql(
        "select routinename as \"name\", routinetype as \"routineType\" from syscat.routines \
         where routineschema = current schema order by routinename"
            .to_string(),
    )
}

fn db2_triggers() -> Meta {
    Meta::Sql(
        "select trigname as \"name\", tabname as \"table\" from syscat.triggers \
         where tabschema = current schema order by trigname"
            .to_string(),
    )
}

/// Derby：例程在 sys.sysaliases（P=过程 / F=函数）。必须按 schema 过滤，否则会把 SYS 里
/// 的系统过程（SQLCAMESSAGE 等）当成用户的例程列出来（实测）。
fn derby_procedures() -> Meta {
    Meta::Sql(
        "select a.alias as \"name\", a.aliastype as \"routineType\" \
         from sys.sysaliases a \
         join sys.sysschemas s on s.schemaid = a.schemaid \
         where a.aliastype in ('P','F') and s.schemaname = current schema \
         order by a.alias"
            .to_string(),
    )
}

/// Derby 没有 syscat（DB2 的 schema）：触发器的系统表是 sys.systriggers。
/// 表名不在该表里（只在定义文本里），所以只给名字 —— 少一列好过报 42X05 让整个清单打不开。
/// 与 procedures 同一口径：不加 schema 过滤会把内核自带的触发器也列出来。
fn derby_triggers() -> Meta {
    Meta::Sql(
        "select t.triggername as \"name\" from sys.systriggers t \
         join sys.sysschemas s on s.schemaid = t.schemaid \
         where s.schemaname = current schema order by t.triggername"
            .to_string(),
    )
}

// ==================== 其余方言：用户清单 / 详情 ====================

/// Derby / DB2：用户由**外部认证**（操作系统 / LDAP）提供，这里只能看不能建 ——
/// `user_action_sql` 保持 Unwritten，界面上的「新增用户」会自动置灰。
fn derby_users() -> Meta {
    Meta::Sql("select username as \"name\" from sys.sysusers order by username".to_string())
}

fn derby_user_info(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
        "select username as \"name\" from sys.sysusers where username = {}",
        ops.literal(name)
    ))
}

/// DB2 LUW 的用户/组授权目录是 SYSCAT.DBAUTH（SYSIBM.SYSUSERAUTH 是 z/OS 的）。
fn db2_users() -> Meta {
    Meta::Sql(
        "select grantee as \"name\" from syscat.dbauth \
         where grantee <> 'PUBLIC' and grantee not like 'SYS%' order by grantee"
            .to_string(),
    )
}

fn db2_user_info(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
        "select grantee as \"name\" from syscat.dbauth where grantee = {}",
        ops.literal(name)
    ))
}

/// H2：`INFORMATION_SCHEMA.USERS`（2.x，实测 2.3.232）。默认用户的用户名是**空串**，
/// 直接显示会是一行点不动的空白 —— 显示成 H2 的空标识符 `""`（带引号，且能直接贴进
/// SQL），查详情 / 改 / 删时再还原（见 `h2_user`）。
fn h2_users() -> Meta {
    Meta::Sql(
        "select case when coalesce(user_name,'') = '' then '\"\"' else user_name end as \"name\" \
         from information_schema.users order by 1"
            .to_string(),
    )
}

/// H2 没有"全局权限"这一层，只有 ADMIN 标志；细粒度授权就是表级 GRANT。
/// 标识符一律双引号（H2 的标准引用符），内部 `"` 转义成 `""`。
fn h2_user_info(ops: &dyn DialectOps, name: &str) -> Meta {
    // user_name 匹配必须大小写不敏感：裸写建的用户会被 H2 折叠成大写存储（实测），
    // 界面回显传的是小写原样 —— 字面量精确匹配 0 行，详情页直接报「没有找到用户」。
    let who = ops.literal(&h2_user(name));
    Meta::Sql(format!(
        "select case when coalesce(user_name,'') = '' then '\"\"' else user_name end as \"name\", \
         case when is_admin then 'true' else 'false' end as \"is_admin\", \
         'create user \"' || replace(case when coalesce(user_name,'') = '' then 'new_user' else user_name end, '\"', '\"\"') || '\" password ''<password>''' as \"create_user_sql\", \
         coalesce((select listagg('grant ' || privilege_type || ' on \"' || replace(table_schema, '\"', '\"\"') || '\".\"' || replace(table_name, '\"', '\"\"') || '\" to \"' || replace(grantee, '\"', '\"\"') || '\"', char(10)) within group (order by table_schema, table_name, privilege_type) \
           from information_schema.table_privileges where grantee = user_name), '') as \"grants_text\" \
         from information_schema.users where upper(user_name) = upper({who})"
    ))
}

/// H2：细粒度授权只有表级一层（没有全局权限，只有 ADMIN 标志）。
fn h2_user_grants(ops: &dyn DialectOps, user: &str) -> Meta {
    Meta::Sql(format!(
        "select 'table' as scope, privilege_type as privilege, table_schema as db_name, \
         table_name as table_name from information_schema.table_privileges \
         where upper(grantee) = upper({})",
        ops.literal(&h2_user(user))
    ))
}
