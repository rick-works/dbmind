//! MySQL 语法族：MySQL / MariaDB / TiDB / Doris / ClickHouse。
//!
//! 这三者共用 MySQL 协议与反引号标识符，差异集中在「有没有原生 upsert」
//! 与「UPDATE 怎么表达」，所以同处一族、各自覆盖差异项。

use super::*;

use crate::api::dialect::{is_clickhouse_privilege, is_mysql_auth_plugin, is_safe_name};

impl DialectOps for MySqlOps {
    fn revoke_missing_hint(&self) -> &'static str {
        "库/表名"
    }

    fn is_revocable(&self, scope: &str, privilege: &str, owner: &str, object: &str, kind: &str) -> bool {
        // MySQL/Oracle 的撤销行缺 scope 时当 global（界面老调用方的形状）
        let scope = if scope.is_empty() { "global" } else { scope };
        match scope {
            // 库级 / 表级走的都是对象权限白名单（与 dbPrivileges / tablePrivileges 同一个校验器）
            "db" => crate::api::dialect::is_mysql_privilege(privilege) && !owner.is_empty(),
            "table" | "object" | "schema" => {
                crate::api::dialect::is_mysql_privilege(privilege) && !owner.is_empty() && !object.is_empty()
            }
            "routine" => {
                crate::api::dialect::is_mysql_privilege(privilege)
                    && !owner.is_empty()
                    && !object.is_empty()
                    && matches!(kind, "PROCEDURE" | "FUNCTION")
            }
            // 角色（MySQL 8 / MariaDB）是 `名字@host` 的账号形状，不是权限名
            "role" => crate::api::dialect::is_role_ref(privilege),
            "global" | "server-role" => crate::api::dialect::is_mysql_privilege(privilege),
            _ => false,
        }
    }

    fn grant_input_scopes(&self) -> &'static [&'static str] {
        &[
            "privileges",
            "dbPrivileges",
            "tablePrivileges",
            "routinePrivileges",
            "revoke",
        ]
    }

    // MySQL 的角色是账号形状（`'r'@'h'`）；MariaDB 的角色没有 host（真机回归发现：
    // 拼成 `名字@` 会在界面上多出一列对不上的假角色），走默认的字面量档。
    fn role_grant_text(
        &self,
        role_name: &str,
        role_host: &str,
        account_user: &str,
        account_host: &str,
    ) -> String {
        if self.mariadb || role_host.is_empty() {
            return format!(
                "GRANT {} TO {};",
                self.literal(role_name),
                self.literal(account_user)
            );
        }
        format!(
            "GRANT {}@{} TO {}@{};",
            self.literal(role_name),
            self.literal(role_host),
            self.literal(account_user),
            self.literal(account_host)
        )
    }

    // 默认转义符就是 `\`，显式 escape 反而可能被判重复声明
    fn like_escape_suffix(&self) -> &'static str {
        ""
    }
    fn object_source(&self, object_type: &str, name: &str, _schema: Option<&str>) -> Meta {
        // 视图/例程/触发器/事件：MySQL 与 MariaDB 的 `show create <类型>` 写法一致
        match object_type {
            "view" | "procedure" | "function" | "trigger" | "event" => {
                Meta::Sql(format!("show create {} {}", object_type, self.quote(name)))
            }
            _ => Meta::Unwritten,
        }
    }

    fn portable_time_literal(&self, portable: &'static str) -> Option<&'static str> {
        // DEFAULT CURRENT_DATE 直接语法错 —— MySQL 的列默认值只支持字面量与
        // CURRENT_TIMESTAMP（表达式默认值要 8.0.13 起且得写 `(curdate())`）。
        // MariaDB 认 current_date（实测矩阵），所以按 mariadb 标志分叉。
        if portable == "current_date" && !self.mariadb {
            None
        } else {
            Some(portable)
        }
    }

    fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        let _ = schema;
        Meta::Sql(format!("show create table {}", self.quote(table)))
    }

    fn table_action(&self, table: &str, action: &str, new_name: Option<&str>) -> Meta {
        let t = self.quote(table);
        match action {
            "clear" => Meta::Sql(format!("delete from {t}")),
            "truncate" => Meta::Sql(format!("truncate table {t}")),
            "drop" => Meta::Sql(format!("drop table {t}")),
            // MySQL 系与 DB2 一样有自己的 RENAME TABLE（ALTER … RENAME TO 也认，
            // 但保持与 DB2 同一款独立语句，行为已在真机对过）
            "rename" => match new_name {
                Some(new_name) => Meta::Sql(format!("rename table {t} to {}", self.quote(new_name))),
                None => Meta::Unwritten,
            },
            _ => Meta::Unwritten,
        }
    }

    fn user_action_sql(
        &self,
        action: &str,
        as_role: bool,
        user: &str,
        password: &str,
        host: &str,
        roles: &[String],
        privileges: &UserPrivileges,
        revokes: &[GrantRevoke],
        reset_privileges: bool,
        attrs: &UserAttrs,
    ) -> Meta {
        let flavor = if self.mariadb {
            MyFlavor::Mariadb
        } else {
            MyFlavor::Mysql
        };
        mysql_user_action(
            self,
            flavor,
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

    fn mariadb_flag(&self) -> bool {
        self.mariadb
    }

    fn users(&self) -> Meta {
        mysql_users(self.mariadb)
    }

    fn user_info(&self, name: &str) -> Meta {
        mysql_user_info(self, name)
    }

    fn user_grants(&self, user: &str, host: &str) -> Meta {
        mysql_user_grants(self, user, host, true)
    }

    fn user_roles(&self, user: &str, host: &str) -> Meta {
        mysql_user_roles(self, user, host)
    }

    fn role_info(&self, name: &str) -> Meta {
        mysql_role_info(self, name)
    }

    fn role_grants(&self, name: &str, host: &str) -> Meta {
        mysql_role_grants(self, name, host)
    }

    fn role_inventory(&self) -> Meta {
        mysql_role_inventory(self.mariadb)
    }

    fn procedures(&self, _schema: Option<&str>) -> Meta {
        mysql_procedures()
    }

    fn triggers(&self, _schema: Option<&str>) -> Meta {
        mysql_triggers()
    }

    fn events(&self) -> Meta {
        mysql_events()
    }

    fn table_rows_sql(&self, database: &str) -> Option<String> {
        Some(mysql_table_rows(self, database))
    }

    fn table_options_sql(&self, database: &str) -> Option<String> {
        Some(mysql_table_options(self, database))
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        Some(MYSQL_INDEXES)
    }

    fn upsert_sql(
        &self,
        table: &str,
        columns: &[String],
        key_indexes: &[usize],
        rows: &[Vec<String>],
    ) -> Option<String> {
        let p = upsert_parts(self, columns, key_indexes, rows)?;
        Some(mysql_upsert(&p, table))
    }
    fn supports_drop_object(&self, kind: &str) -> bool {
        mysql_drop_ok(kind)
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        mysql_column_types(quoted)
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        Some(mysql_database_stats())
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        mysql_table_comments(quoted)
    }
    fn bool_literal(&self, truth: bool) -> &'static str {
        if truth {
            "1"
        } else {
            "0"
        }
    }
    fn databases_sql(&self) -> Option<&'static str> {
        Some(dbmind_core::scope_sql::MYSQL_DATABASES)
    }
    fn wrap_ident(&self, text: &str) -> String {
        format!("`{}`", text.replace('`', "``"))
    }
    fn escapes_backslash(&self) -> bool {
        true
    }
    fn switchable_database(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "mysql"
    }

    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::MYSQL
    }

    fn quote_mark(&self) -> char {
        '`'
    }

    fn create_database_sql(&self, name: &str) -> String {
        // MySQL 系建库**必须显式 utf8mb4**：服务器默认库字符集常是 latin1，之后中文
        // INSERT 直接报 `Incorrect string value` 1366（真机踩过）—— 库里先别埋雷。
        format!(
            "create database if not exists {} default charset utf8mb4",
            self.quote(name)
        )
    }

    fn drop_table_sql(&self, quoted_table: &str) -> String {
        format!("drop table if exists {quoted_table}")
    }

    fn table_rows_estimate_sql(&self, database: &str, _schema: &str) -> Option<String> {
        // `table_rows` 是统计估算（InnoDB 可能偏差 40%），但只用于排序，够用。
        let db = database.replace('\'', "''");
        Some(format!(
            "select table_name as t, table_rows as n from information_schema.tables \
             where table_schema = '{db}'"
        ))
    }

    fn column_type_names(&self) -> &'static [&'static str] {
        editor::MYSQL_COLUMN_TYPES
    }

    fn index_type_names(&self) -> &'static [&'static str] {
        editor::MYSQL_INDEX_TYPES
    }

    fn storage_engines(&self) -> &'static [&'static str] {
        editor::MYSQL_ENGINES
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
        DirectRefDepth::Database
    }
    fn instance_fingerprint_sql(&self) -> Option<&'static str> {
        Some("select @@server_uuid as v")
    }
}

impl DialectOps for DorisOps {
    fn revoke_missing_hint(&self) -> &'static str {
        "库/表名"
    }

    fn is_revocable(&self, scope: &str, privilege: &str, owner: &str, object: &str, kind: &str) -> bool {
        // MySQL/Oracle 的撤销行缺 scope 时当 global（界面老调用方的形状）
        let scope = if scope.is_empty() { "global" } else { scope };
        match scope {
            // 库级 / 表级走的都是对象权限白名单（与 dbPrivileges / tablePrivileges 同一个校验器）
            "db" => crate::api::dialect::is_mysql_privilege(privilege) && !owner.is_empty(),
            "table" | "object" | "schema" => {
                crate::api::dialect::is_mysql_privilege(privilege) && !owner.is_empty() && !object.is_empty()
            }
            "routine" => {
                crate::api::dialect::is_mysql_privilege(privilege)
                    && !owner.is_empty()
                    && !object.is_empty()
                    && matches!(kind, "PROCEDURE" | "FUNCTION")
            }
            // 角色（MySQL 8 / MariaDB）是 `名字@host` 的账号形状，不是权限名
            "role" => crate::api::dialect::is_role_ref(privilege),
            "global" | "server-role" => crate::api::dialect::is_mysql_privilege(privilege),
            _ => false,
        }
    }

    fn grant_input_scopes(&self) -> &'static [&'static str] {
        // Doris 没有例程（连 mysql.procs_priv 这张表都没有），
        // routinePrivileges 不放行 —— 拼出 MySQL 的 `grant execute on procedure …` 必炸。
        &["privileges", "dbPrivileges", "tablePrivileges", "revoke"]
    }

    // 默认转义符就是 `\`，显式 escape 反而可能被判重复声明
    fn like_escape_suffix(&self) -> &'static str {
        ""
    }

    fn object_source(&self, object_type: &str, name: &str, _schema: Option<&str>) -> Meta {
        match object_type {
            // 视图同 MySQL（show create）
            "view" => Meta::Sql(format!("show create view {}", self.quote(name))),
            // Doris 没有例程/触发器/事件：如实 Absent，导出静默跳过
            "procedure" | "function" | "trigger" | "event" => Meta::Absent,
            _ => Meta::Unwritten,
        }
    }

    fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        let _ = schema;
        Meta::Sql(format!("show create table {}", self.quote(table)))
    }

    fn table_action(&self, table: &str, action: &str, new_name: Option<&str>) -> Meta {
        let t = self.quote(table);
        match action {
            "clear" => Meta::Sql(format!("delete from {t}")),
            "truncate" => Meta::Sql(format!("truncate table {t}")),
            "drop" => Meta::Sql(format!("drop table {t}")),
            // Doris 的 `ALTER TABLE … RENAME` **不带 TO**：写成 `rename to` 直接
            // `Syntax error in line 1`（真机实测）。
            "rename" => match new_name {
                Some(new_name) => Meta::Sql(format!("alter table {t} rename {}", self.quote(new_name))),
                None => Meta::Unwritten,
            },
            _ => Meta::Unwritten,
        }
    }

    fn user_action_sql(
        &self,
        action: &str,
        as_role: bool,
        user: &str,
        password: &str,
        host: &str,
        roles: &[String],
        privileges: &UserPrivileges,
        revokes: &[GrantRevoke],
        reset_privileges: bool,
        attrs: &UserAttrs,
    ) -> Meta {
        // StarRocks 没有 `mysql` 库：建/改/删用户与授权都无从生成（见 `users`），
        // 如实 Absent ⇒ features.supportsUserManage=false，界面不摆「用户管理」入口。
        if self.starrocks {
            return Meta::Absent;
        }
        mysql_user_action(
            self,
            MyFlavor::Doris,
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

    fn users(&self) -> Meta {
        // StarRocks 没有 `mysql` 库 —— 读写 `mysql.user` 一律 `Unknown database 'mysql'`
        //（真机实测：整条清单 500）。它的账号在 FE 里、只有 `SHOW USERS` 能看到，
        // 而「编辑 / 授权 / 角色」整条链路都拿不到表结构支撑，所以这一族如实 Absent，
        // 界面按「该类型不支持用户管理」显示，好过点一下就报错。
        if self.starrocks {
            Meta::Absent
        } else {
            mysql_doris_users()
        }
    }

    fn user_info(&self, name: &str) -> Meta {
        if self.starrocks {
            return Meta::Absent;
        }
        mysql_user_info(self, name)
    }

    fn user_grants(&self, user: &str, host: &str) -> Meta {
        if self.starrocks {
            return Meta::Absent;
        }
        mysql_user_grants(self, user, host, false)
    }

    fn role_info(&self, name: &str) -> Meta {
        if self.starrocks {
            return Meta::Absent;
        }
        mysql_doris_role_info(self, name)
    }

    fn role_inventory(&self) -> Meta {
        if self.starrocks {
            return Meta::Absent;
        }
        Meta::Sql("show roles".to_string())
    }

    fn procedures(&self, _schema: Option<&str>) -> Meta {
        Meta::Absent
    }

    fn table_rows_sql(&self, database: &str) -> Option<String> {
        Some(mysql_table_rows(self, bare_database(database)))
    }

    fn table_options_sql(&self, database: &str) -> Option<String> {
        Some(mysql_table_options(self, bare_database(database)))
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        Some(DORIS_INDEXES)
    }

    fn supports_drop_object(&self, kind: &str) -> bool {
        mysql_drop_ok(kind)
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        mysql_column_types(quoted)
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        Some(mysql_database_stats())
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        mysql_table_comments(quoted)
    }
    fn bool_literal(&self, truth: bool) -> &'static str {
        if truth {
            "1"
        } else {
            "0"
        }
    }
    fn boolean_literal(&self, value: bool) -> String {
        if value { "1" } else { "0" }.to_string()
    }
    fn databases_sql(&self) -> Option<&'static str> {
        Some(dbmind_core::scope_sql::MYSQL_DATABASES)
    }
    fn catalogs_sql(&self) -> Option<&'static str> {
        Some("show catalogs")
    }
    fn wrap_ident(&self, text: &str) -> String {
        format!("`{}`", text.replace('`', "``"))
    }
    fn switchable_database(&self) -> bool {
        true
    }
    fn catalog_level(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "doris"
    }

    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::DORIS
    }

    fn quote_mark(&self) -> char {
        '`'
    }

    fn create_database_sql(&self, name: &str) -> String {
        // Doris 的字符集由表级控制，建库带 charset 是多余的（且部分版本不认）
        format!("create database if not exists {}", self.quote(name))
    }

    fn drop_table_sql(&self, quoted_table: &str) -> String {
        format!("drop table if exists {quoted_table}")
    }

    fn sync_object_concurrency(&self) -> usize {
        // 每次导入落一个 part，并发太高会压小 part 合并
        3
    }

    fn table_rows_estimate_sql(&self, database: &str, _schema: &str) -> Option<String> {
        // Doris 的 `information_schema.tables` 与 MySQL 同名同列，直接共用口径
        let db = database.replace('\'', "''");
        Some(format!(
            "select table_name as t, table_rows as n from information_schema.tables \
             where table_schema = '{db}'"
        ))
    }

    fn column_type_names(&self) -> &'static [&'static str] {
        editor::DORIS_COLUMN_TYPES
    }

    fn index_type_names(&self) -> &'static [&'static str] {
        editor::DORIS_INDEX_TYPES
    }

    fn storage_engines(&self) -> &'static [&'static str] {
        editor::DORIS_ENGINES
    }
    fn version_sql(&self) -> Option<&'static str> {
        Some("select version() as v")
    }
    fn direct_ref_depth(&self) -> DirectRefDepth {
        DirectRefDepth::Database
    }
}

impl DialectOps for ClickHouseOps {
    fn revoke_missing_hint(&self) -> &'static str {
        "库/表名"
    }

    fn is_revocable(&self, scope: &str, privilege: &str, owner: &str, object: &str, kind: &str) -> bool {
        let scope = if scope.is_empty() { "global" } else { scope };
        match scope {
            "db" => crate::api::dialect::is_clickhouse_privilege(privilege) && !owner.is_empty(),
            "table" | "object" | "schema" => {
                crate::api::dialect::is_clickhouse_privilege(privilege)
                    && !owner.is_empty()
                    && !object.is_empty()
            }
            "routine" => {
                crate::api::dialect::is_clickhouse_privilege(privilege)
                    && !owner.is_empty()
                    && !object.is_empty()
                    && matches!(kind, "PROCEDURE" | "FUNCTION")
            }
            "role" => crate::api::dialect::is_role_ref(privilege),
            "global" | "server-role" => crate::api::dialect::is_clickhouse_privilege(privilege),
            _ => false,
        }
    }

    fn grant_input_scopes(&self) -> &'static [&'static str] {
        // 与 MySQL 系同构的三层（全局 ON *.* / 库 ON db.* / 表 ON db.table）+ 撤销差量
        &["privileges", "dbPrivileges", "tablePrivileges", "revoke"]
    }

    // 默认转义符就是 `\`，显式 escape 反而可能被判重复声明
    fn like_escape_suffix(&self) -> &'static str {
        ""
    }

    fn object_source(&self, object_type: &str, name: &str, _schema: Option<&str>) -> Meta {
        match object_type {
            // 视图/表都有 show create，与 ddl() 里那条同源
            "view" => Meta::Sql(format!("show create view {}", self.quote(name))),
            "table" => Meta::Sql(format!("show create table {}", self.quote(name))),
            // CH 没有例程/触发器/事件
            "procedure" | "function" | "trigger" | "event" => Meta::Absent,
            _ => Meta::Unwritten,
        }
    }

    fn rewrite_time_default(&self, upper: &str, is_textual_type: bool) -> Option<String> {
        // CH **没有 current_timestamp / current_date**：裸写会被当成**列名**
        // ⇒ `Code: 47 Missing columns: 'current_timestamp' … default expression and
        // column type are incompatible`，整张表建不出来（真机：源表
        // `createtime datetime DEFAULT CURRENT_TIMESTAMP` 同步到 CH 必挂）。
        // CH 的写法是 now() / today()。文本列的默认值是**内容**，不翻。
        if is_textual_type {
            return None;
        }
        if upper.contains("CURRENT_TIMESTAMP") || upper.starts_with("NOW(") {
            return Some("now()".to_string());
        }
        if upper.contains("CURRENT_DATE") || upper.starts_with("TODAY(") {
            return Some("today()".to_string());
        }
        None
    }

    fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        let _ = schema;
        Meta::Sql(format!("show create table {}", self.quote(table)))
    }

    fn table_action(&self, table: &str, action: &str, new_name: Option<&str>) -> Meta {
        let t = self.quote(table);
        match action {
            // CH 的 `DELETE FROM` 是「轻量删除」，老版本/未开启时直接报错；而
            // 「清空整表」本来就有 TRUNCATE 这条正路 —— 按语义选语句
            "clear" | "truncate" => Meta::Sql(format!("truncate table {t}")),
            "drop" => Meta::Sql(format!("drop table {t}")),
            // CH 没有 `ALTER TABLE … RENAME TO`（Code 62），改表名用独立的
            // `RENAME TABLE a TO b`（真机上两种形式都对过：前者 Code 62，后者 OK）
            "rename" => match new_name {
                Some(new_name) => Meta::Sql(format!("rename table {t} to {}", self.quote(new_name))),
                None => Meta::Unwritten,
            },
            _ => Meta::Unwritten,
        }
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
        _attrs: &UserAttrs,
    ) -> Meta {
        let u = self.quote(user);
        let pwd = self.literal(password);
        let role_parts: Vec<String> = roles.iter().map(|role| self.quote(role)).collect();
        // ClickHouse：用户没有 (user, host) 二元组，host 参数没有意义（忽略）。
        // 授权与 MySQL 系同构：全局 `ON *.*` / 库 `ON db.*` / 表 `ON db.table`，
        // 权限名过 CLICKHOUSE_PRIVILEGES 白名单（原样拼 SQL，名单外直接跳过）。
        // 撤销按调用方算好的**差量**逐条点名 —— 与 MySQL「不全删重授」同一取舍。
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
                parts.push(format!(
                    "revoke {} on {} from {u}",
                    r.privilege.trim().to_ascii_uppercase(),
                    target
                ));
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
            parts.push(format!(
                "grant {} on {}.* to {u}",
                names.join(", "),
                self.quote(database)
            ));
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

    fn users(&self) -> Meta {
        Meta::Sql("select name from system.users order by name".to_string())
    }

    fn user_info(&self, name: &str) -> Meta {
        clickhouse_user_info(self, name)
    }

    fn user_grants(&self, user: &str, _host: &str) -> Meta {
        Meta::Sql(format!("show grants for {}", self.quote(user)))
    }

    fn user_roles(&self, user: &str, _host: &str) -> Meta {
        clickhouse_user_roles(self, user)
    }

    fn role_info(&self, name: &str) -> Meta {
        clickhouse_role_info(self, name)
    }

    fn role_grants(&self, name: &str, _host: &str) -> Meta {
        clickhouse_role_grants(self, name)
    }

    fn role_inventory(&self) -> Meta {
        Meta::Sql("select name as role_name, '' as role_host from system.roles".to_string())
    }

    fn procedures(&self, _schema: Option<&str>) -> Meta {
        Meta::Absent
    }

    fn table_options_sql(&self, database: &str) -> Option<String> {
        Some(clickhouse_table_options(self, database))
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        Some(CLICKHOUSE_INDEXES)
    }

    fn supports_drop_object(&self, kind: &str) -> bool {
        matches!(kind, "view")
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        // ClickHouse 没有 information_schema.columns，列类型/注释在 system.columns：
        // type 列是 CH 原生写法（Int32 / Nullable(String) …），parse 侧已能剥 Nullable 壳
        Meta::Sql(format!(
            "select name as column_name, type as type_text, \
             coalesce(comment, '') as comment, \
             0 as is_auto, \
             position as ordinal \
             from system.columns where database = currentDatabase() and table = {quoted}"
        ))
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        Some(
            "select count(*) as tables, formatReadableSize(sum(total_bytes)) as sizeText \
             from system.tables where database = currentDatabase()",
        )
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        Meta::Sql(format!(
            "select name as column_name, coalesce(comment, '') as comment \
             from system.columns where database = currentDatabase() and table = {quoted}"
        ))
    }
    fn bool_literal(&self, truth: bool) -> &'static str {
        if truth {
            "1"
        } else {
            "0"
        }
    }
    fn databases_sql(&self) -> Option<&'static str> {
        Some(dbmind_core::scope_sql::CLICKHOUSE_DATABASES)
    }
    fn wrap_ident(&self, text: &str) -> String {
        format!("`{}`", text.replace('`', "``"))
    }
    fn escapes_backslash(&self) -> bool {
        true
    }
    fn switchable_database(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "clickhouse"
    }

    fn edit_capabilities(&self) -> editor::EditCapabilities {
        editor::CLICKHOUSE
    }

    fn quote_mark(&self) -> char {
        '`'
    }

    fn drop_table_sql(&self, quoted_table: &str) -> String {
        // CH 支持 `IF EXISTS`；不带的话「重同步前清场」在空库上必失败
        format!("drop table if exists {quoted_table}")
    }

    fn table_rows_estimate_sql(&self, database: &str, _schema: &str) -> Option<String> {
        // CH 没有 information_schema，行数在 system.tables（`total_rows` 是实时值）
        let db = database.replace('\'', "''");
        Some(format!(
            "select name as t, total_rows as n from system.tables where database = '{db}'"
        ))
    }

    fn column_type_names(&self) -> &'static [&'static str] {
        editor::CLICKHOUSE_COLUMN_TYPES
    }

    fn index_type_names(&self) -> &'static [&'static str] {
        editor::CLICKHOUSE_INDEX_TYPES
    }

    fn storage_engines(&self) -> &'static [&'static str] {
        editor::CLICKHOUSE_ENGINES
    }
    fn version_sql(&self) -> Option<&'static str> {
        Some("select version() as v")
    }
    fn direct_ref_depth(&self) -> DirectRefDepth {
        DirectRefDepth::Database
    }
    fn update_sql(&self, table: &str, assignments: &str, condition: &str) -> String {
        format!("alter table {table} update {assignments} where {condition}")
    }
    fn delete_sql(&self, table: &str, condition: &str) -> String {
        format!("alter table {table} delete where {condition}")
    }
    fn row_ops_are_async_mutations(&self) -> bool {
        true
    }
}

/// MySQL 协议族的列注释查询（MySqlOps 与 DorisOps 共用同一份写法）。
fn mysql_table_comments(quoted: &str) -> Meta {
    Meta::Sql(format!(
        "select column_name as column_name, coalesce(column_comment, '') as comment \
         from information_schema.columns \
         where table_schema = database() and table_name = {quoted}"
    ))
}

/// MySQL 协议族的库级统计（MySqlOps / DorisOps 共用）。
fn mysql_database_stats() -> &'static str {
    "select count(*) as tables, \
     concat(round(coalesce(sum(data_length + index_length), 0) / 1024 / 1024, 2), ' MB') as sizeText \
     from information_schema.tables where table_schema = database()"
}

/// MySQL 协议族的列类型（MySqlOps / DorisOps 共用）。
///
/// MySQL 系直接有完整类型文本（含 enum、unsigned 等），不用自己拼。
fn mysql_column_types(quoted: &str) -> Meta {
    Meta::Sql(format!(
        "select column_name as column_name, column_type as type_text, \
         coalesce(column_comment, '') as comment, \
         case when extra like '%auto_increment%' then 1 else 0 end as is_auto, \
         ordinal_position as ordinal \
         from information_schema.columns \
         where table_schema = database() and table_name = {quoted}"
    ))
}

/// MySQL 协议族的 `drop … if exists` 覆盖范围（MySqlOps / DorisOps 共用）。
fn mysql_drop_ok(kind: &str) -> bool {
    matches!(kind, "view" | "procedure" | "function" | "trigger" | "event")
}

/// MySQL 协议族的 upsert（MySqlOps 用；Doris 没有原生 upsert，不实现）。
fn mysql_upsert(p: &UpsertParts, table: &str) -> String {
    if p.payload.is_empty() {
        // 全是键：MySQL 没有 `do nothing`，用「把键赋给自己」当空更新（命中的行不变）
        let first = p.keys.first().cloned().unwrap_or_default();
        return format!(
            "insert into {table} ({}) values {} on duplicate key update {first} = {first}",
            p.col_list, p.tuples
        );
    }
    let sets = p
        .payload
        .iter()
        .map(|i| {
            let c = p.col(*i);
            format!("{c} = values({c})")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "insert into {table} ({}) values {} on duplicate key update {sets}",
        p.col_list, p.tuples
    )
}

// ==================== MySQL 族：行数 / 表选项 / 索引 ====================

/// 库名可能是 `catalog.库`（Doris 树与页签的叫法），information_schema 只认裸库名 ——
/// 原样传全限定名一行都匹配不上，表注释静默丢失（真机踩过：同步建表注释全空）。
fn bare_database(database: &str) -> &str {
    database.rsplit('.').next().unwrap_or(database)
}

/// 估算行数（InnoDB 统计值可能滞后）。**0 一律回 null**：「统计说 0」与「表真是空的」
/// 在界面上长得一模一样，宁可先留空，等精确回填补上真实 0。
fn mysql_table_rows(ops: &dyn DialectOps, database: &str) -> String {
    format!(
        "select table_name as table_name, \
         case when table_rows is null or table_rows = 0 then null else table_rows end as rows \
         from information_schema.tables where table_schema = {}",
        ops.literal(database)
    )
}

fn mysql_table_options(ops: &dyn DialectOps, database: &str) -> String {
    let db = ops.literal(database);
    format!(
        // 视图的 `table_comment` 是 MySQL 自己填的**字面量 'VIEW'**（视图没注释时），
        // 原样带出去就是「每个视图的注释都写着 VIEW」—— 语义上它不是注释而是类型噪音，
        // 这里按「视图 + 值恰为 VIEW」判掉；真表的同名注释不受影响。
        "select table_name as table_name, engine as engine, \
         table_collation as charset, \
         case when table_type = 'VIEW' and table_comment = 'VIEW' then '' \
              else coalesce(table_comment, '') end as comment, \
         create_time as createTime \
         from information_schema.tables where table_schema = {db}"
    )
}

/// ClickHouse：`system.tables` 一次把引擎、排序键、分区键、注释都给全 —— 这三项正是它
/// 「建表时确定、之后改不了」的部分，界面只读展示。
fn clickhouse_table_options(ops: &dyn DialectOps, database: &str) -> String {
    let db = ops.literal(database);
    format!(
        // 排序键去掉最外层括号：CH 存的是 DDL 里的字面量，`order by (id)` 回的就是 `(id)`，
        // 而界面要的是列/表达式本身（`id`）—— 括号是 DDL 语法不是名字的一部分。
        // 只剥一层：`(a, b)` → `a, b`；本来就是 `id` 的（`order by id`）原样返回。
        "select name as table_name, engine as engine, \
         replaceRegexpAll(sorting_key, '^\\((.*)\\)$', '\\1') as sortingKey, \
         partition_key as partitionKey, coalesce(comment, '') as comment \
         from system.tables where database = {db}"
    )
}

const MYSQL_INDEXES: &str = "select index_name as `name`, table_name as `table`, non_unique as nonUnique, \
     index_type as `type`, \
     group_concat(column_name order by seq_in_index separator ',') as `columns` \
     from information_schema.statistics where table_schema = database() \
     group by table_name, index_name, non_unique, index_type order by table_name, index_name";

/// Doris：**不能**用 MySQL 那种 `group_concat(x order by y separator ',')` 写法 ——
/// 实测报 `Syntax error ... near 'order by seq_in_index separator ',' as columns'`，
/// 整次索引查询 500，界面上「索引」页签于是永远是空的（看着像这张表没索引）。
/// Doris 只认 `group_concat(x, ',')`；代价是列顺序不再由 seq_in_index 保证 ——
/// 它的二级索引本来就少，复合索引列序偶尔不同，也好过整条查询挂掉。
const DORIS_INDEXES: &str = "select index_name as `name`, table_name as `table`, non_unique as nonUnique, \
     group_concat(column_name, ',') as `columns` \
     from information_schema.statistics where table_schema = database() \
     group by table_name, index_name, non_unique order by table_name, index_name";

/// ClickHouse：`expr` 就是跳数索引的列/表达式，接上 `columns` 才有「包含字段」
/// （原来只取 name/table/type，界面那列永远空）。
///
/// **不要**加 `where database = currentDatabase()`：原句不带 WHERE，一次列出所有库
/// （界面自己按表名过滤）；加了之后当前会话库不是 system 时就一行都查不到。
const CLICKHOUSE_INDEXES: &str =
    "select name, table, type, expr as \"columns\" from system.data_skipping_indices \
     order by table, name";

// ==================== MySQL 族：例程 / 触发器 / 事件 ====================

/// MySQL 没有「例程清单要不要过滤」的问题：`routine_schema = database()` 就是当前库。
/// Doris 不实现（它只有 catalog 级 UDF，不在库的 routines 里）——
/// 跟着 MySQL 一起来的话，树上会画出两个分类、点进去恒为空。
fn mysql_procedures() -> Meta {
    Meta::Sql(
        "select routine_name as `name`, routine_type as routineType, \
         created, last_altered as lastAltered, \
         sql_data_access as dataAccess, security_type as securityType \
         from information_schema.routines where routine_schema = database() \
         order by routine_name"
            .to_string(),
    )
}

fn mysql_triggers() -> Meta {
    Meta::Sql(
        "select trigger_name as `name`, event_object_table as `table`, action_timing as timing, \
         event_manipulation as event from information_schema.triggers \
         where trigger_schema = database() order by trigger_name"
            .to_string(),
    )
}

/// 事件（MySQL 的事件调度器）。其余类型确实没有这种东西 —— 走默认的 Absent。
fn mysql_events() -> Meta {
    Meta::Sql(
        "select event_name as `name`, status, interval_value, interval_field, \
         last_executed as lastExecuted, execute_at as executeAt, ends \
         from information_schema.events where event_schema = database() order by event_name"
            .to_string(),
    )
}

// ==================== MySQL 族：用户 / 权限 / 角色 ====================

/// 用户清单。
///
/// **角色不进用户清单**（用户要求）：角色在「编辑 / 授权」的「角色」页签里管。
/// MySQL 8 的角色就是 `account_locked='Y'` 且认证串为空的账号，这里排除掉
/// （真被锁的用户仍有认证串 —— 实测 mysql.sys/session/infoschema 都是 70 字节，
/// 不会被误排除）；MariaDB 直接用 `is_role` 列。
///
///「超级用户」标记直接取 `Super_priv`，**不要写成 `super_priv = 'Y'`** ——
/// MariaDB 的该列是 collation 不同的 CHAR，与字面量比较会报 1267
/// （实测 `Illegal mix of collations … utf8mb4_general_ci / utf8mb4_uca1400_ai_ci`），
/// 整张清单直接 500。取原值（Y/N）出门，由 Rust 侧的 `is_true_flag` 折成布尔。
///
/// ⚠️ host 必须**显式起别名**：MariaDB 的驱动按底层列名回 `Host`（大写），而前端读的是
/// `row.host` —— 不写别名时 MariaDB 的用户清单里「主机」一列是空的（真机回归发现）。
fn mysql_users(mariadb: bool) -> Meta {
    Meta::Sql(if mariadb {
        "select user as `name`, host as `host`, super_priv as `isSuper` from mysql.user \
         where coalesce(cast(is_role as binary), 'N') <> 'Y' \
         order by user, host"
            .to_string()
    } else {
        "select user as `name`, host as `host`, super_priv as `isSuper` from mysql.user \
         where not (account_locked = 'Y' and coalesce(authentication_string, '') = '') \
         order by user, host"
            .to_string()
    })
}

/// MySQL 的账号是**两列**（user + host），而树里显示与传进来的是拼好的 `user@host`。
/// 老写法拿 `demo1@%` 去比 `user` 列，于是点开任何一个用户都是「没有找到用户」。
/// 按**最后一个** `@` 切开再比两列：host 部分不含 `@`，而用户名部分可以（`'a@b'@'%'`）。
fn mysql_user_info(ops: &dyn DialectOps, name: &str) -> Meta {
    let (user, host) = match name.rsplit_once('@') {
        Some((user, host)) => (user, host),
        // 只给了用户名（没有 @）时不限制 host，否则会与 `'x'@'localhost'` 擦肩而过
        None => (name, ""),
    };
    let filter = if host.is_empty() {
        format!("user = {}", ops.literal(user))
    } else {
        format!("user = {} and host = {}", ops.literal(user), ops.literal(host))
    };
    Meta::Sql(format!(
        // 权限明细**不在这里聚合**：`group_concat` 受 `group_concat_max_len` 限制
        // （服务端默认 1024）会静默截断，见 `mysql_user_grants`。
        // 拼接用 concat：MySQL 里 `'a' + 'b'` 是**加法**（结果是 0），老写法直接抄了
        // SQL Server 的 `+`，创建语句会显示成一串数字。
        "select t.user as name, t.host as host, \
         concat('create user ', quote(t.user), '@', quote(t.host)) as create_user_sql, \
         t.* \
         from mysql.user t where {filter} limit 1"
    ))
}

/// Doris 走 MySQL 协议，`mysql.user` 可读，但**没有 Super_priv**（管理员是 `Admin_priv`，
/// 实测 root/admin 都是 Y）。这里原先漏了 Doris —— user_info / user_grants 都带着它，
/// 只有用户清单少一个分支，于是「树的用户节点是空的、详情却能用」。
/// Doris 也没有角色概念（`role_edges` / `roles_mapping` 都不存在），所以它的清单不过滤角色。
fn mysql_doris_users() -> Meta {
    Meta::Sql(
        "select user as `name`, host as `host`, admin_priv as `isSuper` from mysql.user \
         order by user, host"
            .to_string(),
    )
}

/// 权限明细为什么逐行返回、不用 `group_concat` 拼一行：`group_concat_max_len` 默认
/// **1024**，超长会**静默截断** —— 实测 `root@%`（28 项权限）正好被切在 1024，
/// 最后一行成了半截的 `GRANT TRIGGER ON *.* T`，界面上于是少了 UPDATE / TRIGGER 两个勾
/// （用户看到的现象是「root 怎么不是全都有」）。`/*+ SET_VAR(...) */` 这条路走不通：
/// 只对部分优化器变量生效，实测长度仍是 1024。逐行返回没有这个上限。
///
/// **长度就是正确性**：实测（公司 VPN，路由被隧道接管且上行丢弃满 MSS 的包），整句超过
/// 一个 TCP 段（≈1.4KB）时第一个满尺寸段被丢，重传退避 ~19 秒后连接被重置，界面表现就是
/// 「点编辑/授权没反应」。所以列名/字面量一律从短，拆分挪到 Rust 侧（expand_grant_rows）。
///
/// `with_routines`：例程级授权只在 `mysql.procs_priv`（一行多权限、空格分隔），
/// **不在 SQL 里按 LIKE 拆成三段**（那会把整句顶过一个 TCP 段）。Doris 没有这张表 ——
/// 查了必炸，所以它传 false。
fn mysql_user_grants(ops: &dyn DialectOps, user: &str, host: &str, with_routines: bool) -> Meta {
    // grantee 在 information_schema 视图里是**带引号的字符串** `'user'@'host'`：
    // 在 Rust 侧拼好整个串再走 literal() 转义 —— 比 `concat(quote(…))` 短得多。
    let grantee = ops.literal(&format!("'{}'@'{}'", user, host));
    let routine_sql = if with_routines {
        // 一行带回原始 Proc_priv + `类型:名字`（PROCEDURE:p / FUNCTION:f）；
        // 第 5 列 is_grantable 直接随行带回（替代「GRANT OPTION 单独三段 UNION」）。
        format!(
            " union all select 'routine', Proc_priv, Db, concat(Routine_type, ':', Routine_name), '' \
             from mysql.procs_priv where User = {} and Host = {}",
            ops.literal(user),
            ops.literal(host)
        )
    } else {
        String::new()
    };
    Meta::Sql(format!(
        "select 'global' as scope, privilege_type as privilege, '' as db_name, '' as table_name, is_grantable as grant_option \
         from information_schema.user_privileges where grantee = {grantee} \
         union all select 'schema', privilege_type, table_schema, '', is_grantable \
         from information_schema.schema_privileges where grantee = {grantee} \
         union all select 'table', privilege_type, table_schema, table_name, is_grantable \
         from information_schema.table_privileges where grantee = {grantee}{routine_sql}"
    ))
}

/// 两家用的**不是同一张表**（这一点踩过）：MySQL 8.0.1+ 是 `mysql.role_edges`
/// （FROM_USER/FROM_HOST/TO_USER/TO_HOST），MariaDB 10.0.5+ 是 `mysql.roles_mapping`
/// （Host/User/Role/Admin_option，Host 是**被授人**的 host）。MariaDB 上读 role_edges
/// 会直接 1146 —— 那是「角色」页签一片空白、连已授的都看不见。
/// MariaDB 的角色**没有 host**（`grant 'r'@'%'` 是语法错，实测 1064），
/// 所以 role_host 一律回空串，界面按裸名字显示。
fn mysql_user_roles(ops: &dyn DialectOps, user: &str, host: &str) -> Meta {
    Meta::Sql(if ops.mariadb_flag() {
        format!(
            "select Role as role_name, '' as role_host from mysql.roles_mapping \
             where User = {} and Host = {}",
            ops.literal(user),
            ops.literal(host)
        )
    } else {
        format!(
            "select from_user as role_name, from_host as role_host from mysql.role_edges \
             where to_user = {} and to_host = {}",
            ops.literal(user),
            ops.literal(host)
        )
    })
}

/// MySQL 的角色就是账号（host 维度存在）；MariaDB 的角色 Host 恒为空串。
fn mysql_role_info(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
        "select user as `name`, host as `host` from mysql.user where user = {} order by host limit 1",
        ops.literal(name)
    ))
}

/// Doris 这版不支持编辑角色权限（权限名是 SELECT_PRIV 那套，与 MySQL 不同名），
/// 这里仍给一行，至少不会把表单卡在"没有找到"。
fn mysql_doris_role_info(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!("select {} as name, '' as host", ops.literal(name)))
}

fn mysql_role_grants(ops: &dyn DialectOps, name: &str, host: &str) -> Meta {
    if ops.mariadb_flag() {
        // MariaDB：information_schema **不列角色**（实测 user_privileges 里查不到），
        // 只能读 `show grants for '<role>'`。返回的是**单列文本**（每行一条 GRANT），
        // 由 `grant_line` 原样带回，界面再分层回填 —— 与 ClickHouse 同一种交付形态。
        Meta::Sql(format!("show grants for {}", ops.literal(name)))
    } else {
        // MySQL：角色就是账号，权限视图里按 grantee 能查到，直接复用。
        ops.user_grants(name, host)
    }
}

/// MySQL 8 没有独立的角色表（角色就是 `mysql.user` 里 `account_locked='Y'` 且认证串为空的
/// 账号）：只读 `role_edges` 的话，**没授予过任何人的角色在界面上看不见**，等于建了也用不了，
/// 所以两者取并集。MariaDB 的 `CREATE ROLE` 会自己往 `roles_mapping` 写一行（实测）。
fn mysql_role_inventory(mariadb: bool) -> Meta {
    Meta::Sql(if mariadb {
        "select distinct Role as role_name, '' as role_host from mysql.roles_mapping".to_string()
    } else {
        "select role_name, role_host from ( \
           select from_user as role_name, from_host as role_host from mysql.role_edges \
           union \
           select user, host from mysql.user \
           where account_locked = 'Y' and coalesce(authentication_string, '') = '' \
         ) t"
        .to_string()
    })
}

/// ClickHouse：`system.users` 一行就是全部属性；授权清单是 `show grants for`。
/// 认证串（auth_string）**不进详情** —— 与 MySQL 的 authentication_string 同一个理由：
/// 展示出来只会被复制、截图。只挑**版本稳定**的列：default_roles_* / grantees_* 在
/// 新版本改过名（实测 26.8 报 Unknown identifier: grantees_all）。
fn clickhouse_user_info(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
        // `auth_type` 是 **Enum8**（不是数组）：24.8 实测 arrayStringConcat 会抛
        // Code 43 ILLEGAL_TYPE_OF_ARGUMENT。转成字符串即可；host_ip/host_names
        // 才是 Array(String)，继续用 arrayStringConcat 拼成一行。
        "select name, storage, \
         toString(auth_type) as auth_type, \
         arrayStringConcat(host_ip, ', ') as host_ip, \
         arrayStringConcat(host_names, ', ') as host_names \
         from system.users where name = {}",
        ops.literal(name)
    ))
}

/// 授权关系在 `system.role_grants`（granted_role_name 才是"被授角色"）。
fn clickhouse_user_roles(ops: &dyn DialectOps, user: &str) -> Meta {
    Meta::Sql(format!(
        "select granted_role_name as role_name, '' as role_host from system.role_grants \
         where user_name = {}",
        ops.literal(user)
    ))
}

fn clickhouse_role_info(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
        "select name, '' as host from system.roles where name = {}",
        ops.literal(name)
    ))
}

/// `system.grants` 里 role_name 非空的就是"授给角色"的行。
fn clickhouse_role_grants(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
        "select case when database = '' and table = '' then 'global' when table = '' then 'db' else 'table' end as scope, \
                access_type as privilege, database as db_name, table as table_name, \
                if(grant_option, 'YES', 'NO') as grant_option \
           from system.grants where role_name = {} and is_partial_revoke = 0",
        ops.literal(name)
    ))
}

// ==================== 用户增删改的拼装件 ====================

/// MySQL 系里「角色怎么写」的三种口径。
///
/// - `Mysql`：角色是账号形状，'r'@'h'（`role_edges` 的 from_user/from_host）；
/// - `Mariadb`：角色没有 host —— 多写 `@'%'` 是 1064 语法错（真机验证过）；
/// - `Doris`：主体必须写 `TO ROLE 'r'`，角色名走标识符而不是字符串。
#[derive(Clone, Copy, PartialEq, Eq)]
enum MyFlavor {
    Mysql,
    Mariadb,
    Doris,
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
fn mysql_revoke_parts(
    ops: &dyn DialectOps,
    flavor: MyFlavor,
    grantee: &str,
    revokes: &[GrantRevoke],
) -> Vec<String> {
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
                    format!("{}.*", ops.quote(owner))
                } else {
                    format!("{}.{}", ops.quote(owner), ops.quote(object))
                };
                parts.push(format!("revoke {name} on {target} from {grantee}"));
            }
            // 例程级：`revoke execute on procedure `db`.`p` from u` —— 类型字是语法的一部分
            "routine" => {
                if owner.is_empty() || object.is_empty() {
                    continue;
                }
                let routine_type = if row.kind.eq_ignore_ascii_case("function") {
                    "function"
                } else {
                    "procedure"
                };
                parts.push(format!(
                    "revoke {name} on {routine_type} {}.{} from {grantee}",
                    ops.quote(owner),
                    ops.quote(object)
                ));
            }
            // 角色：MySQL 是 `revoke 'role'@'host' from 'u'@'%'`（owner = 角色名，
            // object = 角色 host）；MariaDB / Doris 的角色没有 host，只能 `revoke 'r' from …`。
            "role" => {
                if owner.is_empty() {
                    continue;
                }
                if flavor == MyFlavor::Mysql {
                    let role_host = if object.is_empty() { "%" } else { object };
                    parts.push(format!(
                        "revoke {}@{} from {grantee}",
                        ops.literal(owner),
                        ops.literal(role_host)
                    ));
                } else {
                    parts.push(format!("revoke {} from {grantee}", ops.literal(owner)));
                }
            }
            // 全局（含 GRANT OPTION）
            _ => parts.push(format!("revoke {name} on *.* from {grantee}")),
        }
    }
    parts
}

/// MySQL 的属性子句串：认证（identified …）/ 配额（with max_*）/ 密码过期 / 账户锁定。
///
/// `CREATE USER` 与 `ALTER USER` **共用这一份** —— 两处的子句顺序完全一样
/// （auth_option → WITH 资源项 → PASSWORD EXPIRE → ACCOUNT LOCK），顺序错了直接报语法错；
/// 分开写迟早会漂成"新建能带上、改的时候带不上"。
///
/// 返回 `None` = 这个组合不能用：插件不在白名单，或**改**插件却没给口令
/// （`identified with x` 不带 BY 会把口令置空，用户当场登不进去）。
fn mysql_attr_clause(password: &str, attrs: &UserAttrs, pwd: &str, is_create: bool) -> Option<String> {
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
        clause.push_str(if v {
            " password expire"
        } else {
            " password expire never"
        });
    }
    if let Some(v) = attrs.account_locked {
        clause.push_str(if v { " account lock" } else { " account unlock" });
    }
    Some(clause)
}

/// MySQL 系（MySQL / MariaDB / Doris）的用户增删改 + 授权撤销。
///
/// 三个方言的差别只在**角色**的写法上（见 [`MyFlavor`]），其余一字不差 ——
/// 这正是收进一个函数的原因：以前这里是 `match self.key()` 三连问，
/// 新增一个 MySQL 系方言要在 600 行里找齐每一处。
fn mysql_user_action(
    ops: &dyn DialectOps,
    flavor: MyFlavor,
    action: &str,
    as_role: bool,
    user: &str,
    password: &str,
    host: &str,
    roles: &[String],
    privileges: &UserPrivileges,
    revokes: &[GrantRevoke],
    reset_privileges: bool,
    attrs: &UserAttrs,
) -> Meta {
    let host = if host.trim().is_empty() { "%" } else { host };
    let u = ops.quote(user);
    let pwd = ops.literal(password);
    let h = ops.literal(host);
    // MySQL 的用户是 (user, host) 二元组 —— 少了 host 就会建错主体
    let full = format!("{u}@{h}");
    // 授权语句（grant / revoke）的目标账号一律用**单引号字符串**写法
    // `'user'@'host'`：`quote()` 对普通名字不加任何引号（`dbmind_probe_u@'%'`），
    // 名字里带 `-` 之类的还会变成反引号 —— 那在账号位置是错的。
    let full_lit = format!("{}@{}", ops.literal(user), ops.literal(host));
    // 授权目标：编辑**角色**的权限时，主体是角色本身 —— MySQL 的角色与账号同形
    // （`'r'@'%'`），MariaDB 的角色没有 host（`'r'`），Doris 必须写
    // `TO ROLE 'r'`（它这版也不提供角色权限编辑，见 role_info 注释）。
    let grantee = if as_role {
        match flavor {
            MyFlavor::Mysql => full_lit.clone(),
            MyFlavor::Doris => format!("role {}", ops.literal(user)),
            _ => ops.literal(user),
        }
    } else {
        full_lit.clone()
    };
    let mut parts: Vec<String> = Vec::new();
    match action {
        "create" => match mysql_attr_clause(password, attrs, &pwd, true) {
            Some(clause) => parts.push(format!("create user {full}{clause}")),
            None => return Meta::Unwritten,
        },
        "alter" => {
            // 与 create 同一份子句（见 mysql_attr_clause），只是多一条：
            // 「改插件必须连口令一起给」在那里判掉了。
            match mysql_attr_clause(password, attrs, &pwd, false) {
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
            let who = match flavor {
                MyFlavor::Mysql => full_lit.clone(),
                MyFlavor::Doris => ops.quote(user),
                _ => ops.literal(user),
            };
            return Meta::Sql(if password.is_empty() {
                format!("create role {who}")
            } else {
                // 带口令建角色只有 MySQL/MariaDB 支持（角色可登录，一般不需要）
                format!("create role {who} identified by {pwd}")
            });
        }
        "drop-role" => {
            let who = match flavor {
                MyFlavor::Mysql => full_lit.clone(),
                MyFlavor::Doris => ops.quote(user),
                _ => ops.literal(user),
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
        parts.extend(mysql_revoke_parts(ops, flavor, &grantee, revokes));
    }
    // 角色（MySQL 8 / MariaDB 的 RBAC）：MySQL 是 `grant 'r'@'h' to 'u'@'%'`
    // （角色本身也是账号形状，名字与 host 分别转义再加 @ 拼起来）；
    // MariaDB / Doris 的角色**没有 host** —— 多写 `@'%'` 在 MariaDB 上是
    // 1064 语法错（真机验证过），只能 `grant 'r' to 'u'@'%'`。
    for r in roles {
        let (rn, rh) = split_role_ref(r);
        if rn.is_empty() {
            continue;
        }
        parts.push(if flavor == MyFlavor::Mysql {
            format!("grant {}@{} to {full_lit}", ops.literal(&rn), ops.literal(&rh))
        } else {
            format!("grant {} to {full_lit}", ops.literal(&rn))
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
            ops.quote(database),
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
            ops.quote(database),
            ops.quote(table),
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
        let routine_type = if kind.eq_ignore_ascii_case("function") {
            "function"
        } else {
            "procedure"
        };
        parts.push(format!(
            "grant {} on {} {}.{} to {grantee}{}",
            items.join(", "),
            routine_type,
            ops.quote(database),
            ops.quote(routine),
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
