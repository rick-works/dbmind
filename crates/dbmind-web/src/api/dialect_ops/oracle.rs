//! Oracle 语法族：Oracle / 达梦（DM 与 Oracle 高度一致）。

use super::*;

impl DialectOps for OracleOps {
    fn is_revocable(&self, scope: &str, privilege: &str, owner: &str, object: &str, kind: &str) -> bool {
        let _ = kind;
        let scope = if scope.is_empty() { "global" } else { scope };
        match scope {
            "object" => {
                crate::api::dialect::is_oracle_object_priv(privilege)
                    && !owner.is_empty()
                    && !object.is_empty()
            }
            // 预置角色之外还允许**自定义角色**（「新建角色」建出来的那些）
            "role" => {
                crate::api::dialect::is_oracle_role(privilege)
                    || crate::api::dialect::is_plain_ident(privilege)
            }
            "global" => crate::api::dialect::is_oracle_system_priv(privilege),
            _ => false,
        }
    }

    fn grantable_privilege_check(&self) -> fn(&str) -> bool {
        crate::api::dialect::is_oracle_grantable
    }

    fn object_privilege_check(&self) -> fn(&str) -> bool {
        crate::api::dialect::is_oracle_object_priv
    }

    fn grant_input_scopes(&self) -> &'static [&'static str] {
        // 系统权限（privileges）/ 对象权限（tablePrivileges，database 字段就是模式名）
        // + 撤销清单（Oracle 没有「一把全撤」，只能逐条点名）。
        // 没有「库级授权」这一层（库就是实例），dbPrivileges 由调用方明确拒绝。
        &["privileges", "tablePrivileges", "revoke"]
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
        // Oracle 的三层：系统权限 / 角色都不带 ON，对象权限带 `ON "模式"."对象"`，
        // 账号也不写成 `'u'@'h'`（Oracle 没有 host 这一维）。
        match scope {
            "object" if !owner.is_empty() && !object.is_empty() => Some(format!(
                "GRANT {privilege} ON \"{owner}\".\"{object}\" TO \"{account_user}\";"
            )),
            "role" | "global" => Some(format!("GRANT {privilege} TO \"{account_user}\";")),
            _ => None,
        }
    }

    fn temporal_row_literal(&self, hint: &str, value: &str) -> Option<String> {
        oracle_style_temporal_literal(self, hint, value)
    }
    fn object_source(&self, object_type: &str, name: &str, schema: Option<&str>) -> Meta {
        let name_literal = self.literal(name);
        let schema_lit = schema
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| self.literal(s));
        match object_type {
            "view" => Meta::Sql(oracle_get_ddl("VIEW", &name_literal, &schema_lit)),
            "procedure" => Meta::Sql(oracle_get_ddl("PROCEDURE", &name_literal, &schema_lit)),
            "function" => Meta::Sql(oracle_get_ddl("FUNCTION", &name_literal, &schema_lit)),
            "trigger" => Meta::Sql(oracle_get_ddl("TRIGGER", &name_literal, &schema_lit)),
            // Oracle 没有事件调度器
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

    fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        let lit = self.literal(table);
        let schema_lit = schema
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| self.literal(s));
        match &schema_lit {
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
        attrs: &UserAttrs,
    ) -> Meta {
        let u = self.quote(user);
        // Oracle：`create user / alter user ... identified by / drop user`。
        // 与 PG、ClickHouse 一样忽略 host（Oracle 没有 (user, host) 二元组）。
        //
        // 两条刻意的取舍：
        // · 建完顺手 `grant create session` —— 不然新建的用户**连不上去**，等于没建好；
        //   这不是"多授了权限"，是建一个有登录能力的用户的最小集合。
        // · 删除用**不带 cascade** 的 `drop user` —— 带 cascade 会把该用户 schema 下的
        //   所有对象一起删掉，那是不可逆的破坏；他自己的对象就该由 ORA-01922 拦下来、
        //   让管理员显式去执行 `drop user ... cascade`，而不是我们替他决定。
        Meta::Sql(match action {
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
                if let Some(v) = attrs
                    .default_tablespace
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    create.push_str(&format!(" default tablespace {}", self.quote(v)));
                }
                if let Some(v) = attrs
                    .temporary_tablespace
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
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
                parts.extend(oracle_grant_parts(&u, roles, privileges));
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
                parts.extend(oracle_revoke_parts(&u, revokes));
                parts.extend(oracle_grant_parts(&u, roles, privileges));
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
        })
    }

    fn users(&self) -> Meta {
        oracle_users()
    }

    fn user_info(&self, name: &str) -> Meta {
        oracle_user_info(self, name)
    }

    fn user_grants(&self, user: &str, _host: &str) -> Meta {
        oracle_user_grants(self, user)
    }

    fn role_info(&self, name: &str) -> Meta {
        oracle_role_info(self, name)
    }

    fn role_grants(&self, name: &str, _host: &str) -> Meta {
        oracle_role_grants(self, name)
    }

    fn role_inventory(&self) -> Meta {
        oracle_role_inventory()
    }

    fn schemas_sql(&self) -> Option<&'static str> {
        Some(ORACLE_SCHEMAS)
    }

    fn tables_in_schema_sql(&self, schema: &str) -> Option<String> {
        Some(oracle_tables_in_schema(self, schema))
    }

    fn procedures(&self, schema: Option<&str>) -> Meta {
        oracle_procedures(self, schema)
    }

    fn triggers(&self, schema: Option<&str>) -> Meta {
        oracle_triggers(self, schema)
    }

    fn table_options_sql(&self, database: &str) -> Option<String> {
        Some(oracle_table_options(self, database))
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        Some(ORACLE_INDEXES)
    }

    fn upsert_sql(
        &self,
        table: &str,
        columns: &[String],
        key_indexes: &[usize],
        rows: &[Vec<String>],
    ) -> Option<String> {
        let p = upsert_parts(self, columns, key_indexes, rows)?;
        Some(oracle_upsert(&p, table, rows))
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        // Oracle：NUMBER 用 precision/scale，字符类型用 data_length（**字节**，
        // 字节语义下 VARCHAR2(50 CHAR) 会显示成 200 —— 如实反映存储宽度，不做猜测换算）。
        //
        // 自增只看 12c+ 的标识列（`user_tab_identity_cols`）。**不能**用
        // `c.data_default like '%.nextval%'` 认「序列 + 默认值」的老写法：
        // `data_default` 是 LONG，Oracle 对 LONG 做 LIKE 直接
        // `ORA-00932（数据类型为 LONG，与预期 CHAR 不兼容）`，整条查询连同
        // 类型/注释一起失败 —— 症状就是「列注释改完保存成功、界面还是空」
        // （真机 SYSTEM 实测）。少数用序列默认值的老库因此不显示自增，如实接受。
        Meta::Sql(format!(
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
        ))
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        Some(oracle_database_stats())
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        // `as comment` 在 Oracle 里是**关键字**，不加双引号直接 ORA-00923
        // （整条查询失败 ⇒ 列注释永远为空，看着就是「改完注释没回显」）
        Meta::Sql(format!(
            "select column_name as column_name, coalesce(comments, '') as \"comment\" \
             from user_col_comments where table_name = upper({quoted})"
        ))
    }
    fn folds_ident_upper(&self) -> bool {
        true
    }
    fn is_schema_aware(&self) -> bool {
        true
    }
    fn db_is_schema(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "oracle"
    }
    fn supports_native_upsert(&self) -> bool {
        true
    }
    fn upsert_row_limit(&self) -> usize {
        // MERGE 的源把每一行展开成 `select … from dual` 再 UNION ALL，
        // 分支数太多会先撞解析器/表达式上限；500 与旧 INSERT 路径的保守批大小一致。
        500
    }
    fn wrap_paging(&self, base: &str, offset: u64, size: u64) -> Option<String> {
        let end = offset.saturating_add(size);
        Some(format!(
            "select * from (select a__.*, rownum rn__ from (\n{base}\n) a__ where rownum <= {end}) where rn__ > {offset}"
        ))
    }
    fn limit_clause(&self, offset: u64, size: u64) -> String {
        format!("offset {offset} rows fetch next {size} rows only")
    }
    fn version_sql(&self) -> Option<&'static str> {
        Some("select banner as v from v$version where rownum = 1")
    }
    fn direct_ref_depth(&self) -> DirectRefDepth {
        DirectRefDepth::Schema
    }
}

impl DialectOps for DmOps {
    fn grantable_privilege_check(&self) -> fn(&str) -> bool {
        crate::api::dialect::is_oracle_grantable
    }

    fn object_privilege_check(&self) -> fn(&str) -> bool {
        crate::api::dialect::is_oracle_object_priv
    }

    fn temporal_row_literal(&self, hint: &str, value: &str) -> Option<String> {
        oracle_style_temporal_literal(self, hint, value)
    }

    fn object_source(&self, object_type: &str, name: &str, _schema: Option<&str>) -> Meta {
        let name_literal = self.literal(name);
        match object_type {
            // DM：三参 `get_ddl` 没真机验证过（它的 dbms_metadata 支持面不确定），
            // 先保持两句 —— 与「达梦的例程清单先不动」同一个原则
            "view" => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('VIEW', upper({name_literal})) as definition from dual"
            )),
            "procedure" => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('PROCEDURE', upper({name_literal})) as definition from dual"
            )),
            "function" => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('FUNCTION', upper({name_literal})) as definition from dual"
            )),
            "trigger" => Meta::Sql(format!(
                "select dbms_metadata.get_ddl('TRIGGER', upper({name_literal})) as definition from dual"
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

    fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        let _ = schema;
        let lit = self.literal(table);
        // Oracle/DM 通用兜底：手工拼列（listagg）+ 视图原文（user_views.text 两家都有）
        Meta::Sql(format!(
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
        ))
    }

    fn users(&self) -> Meta {
        oracle_users()
    }

    fn user_info(&self, name: &str) -> Meta {
        oracle_user_info(self, name)
    }

    fn procedures(&self, _schema: Option<&str>) -> Meta {
        dm_procedures()
    }

    fn triggers(&self, _schema: Option<&str>) -> Meta {
        dm_triggers()
    }

    fn table_options_sql(&self, _database: &str) -> Option<String> {
        Some(dm_table_options())
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        Some(ORACLE_INDEXES)
    }

    fn upsert_sql(
        &self,
        table: &str,
        columns: &[String],
        key_indexes: &[usize],
        rows: &[Vec<String>],
    ) -> Option<String> {
        let p = upsert_parts(self, columns, key_indexes, rows)?;
        Some(oracle_upsert(&p, table, rows))
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        // DM（达梦）：类型 + 注释（与 Oracle 同一套写法 —— 达梦是 Oracle 兼容的，
        // 连表注释那边用的也是 `user_col_comments`）。
        // 注释这一项**曾经漏了**：select 里根本没取 comment，界面上「注释」列却有
        // （supportsComment 为真），于是那一格永远是空的（用户反馈排查出来的同类问题）。
        //
        // **不自增检测**：达梦的标识列（`IDENTITY(1,1)`）记在哪张系统视图上，没有实例可验证；
        // 而 `user_tab_identity_cols` 在达梦上是否存在同样未验证 —— 一旦那句 SQL 报错，
        // 整张表的类型/注释补全都会一起丢掉（比少一个自增标记更糟）。所以这里留白，
        // 而不是拿一个没验证过的视图去赌。
        Meta::Sql(format!(
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
        ))
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        Some(oracle_database_stats())
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        // 达梦与 Oracle 同一套数据字典，`comment` 同样要加双引号
        Meta::Sql(format!(
            "select column_name as column_name, coalesce(comments, '') as \"comment\" \
             from user_col_comments where table_name = upper({quoted})"
        ))
    }
    fn folds_ident_upper(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "dm"
    }
    fn supports_native_upsert(&self) -> bool {
        true
    }
    fn upsert_row_limit(&self) -> usize {
        500
    }
    fn limit_clause(&self, offset: u64, size: u64) -> String {
        format!("offset {offset} rows fetch next {size} rows only")
    }
    fn version_sql(&self) -> Option<&'static str> {
        Some("select banner as v from v$version where rownum = 1")
    }
    fn direct_ref_depth(&self) -> DirectRefDepth {
        DirectRefDepth::Schema
    }
}

/// Oracle 协议族的库级统计（OracleOps / DmOps 共用）。
///
/// `sum(bytes)` 是聚合，`(select count(*) …)` 是标量子查询，混在同一个 select 列表里
/// 又没 group by ⇒ ORA-00937「不是单组分组函数」（真机实测）。所以写成两个互不依赖的
/// 标量子查询 + `from dual`。
fn oracle_database_stats() -> &'static str {
    "select (select count(*) from user_tables) as tables, \
     (select to_char(round(coalesce(sum(bytes), 0) / 1024 / 1024, 2)) || ' MB' \
      from user_segments) as sizeText \
     from dual"
}

/// Oracle 协议族的 MERGE（OracleOps / DmOps 共用）。
///
/// 源是 `select … from dual union all …`：Oracle 的 MERGE 不认 `values (…) as src`；
/// 表别名也**不能带 AS**（`merge into t as tgt` 是 ORA-00933）。
fn oracle_upsert(p: &UpsertParts, table: &str, rows: &[Vec<String>]) -> String {
    let branches = rows
        .iter()
        .map(|row| {
            let exprs = p
                .quoted
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{} as {}", row[i], c))
                .collect::<Vec<_>>()
                .join(", ");
            format!("select {exprs} from dual")
        })
        .collect::<Vec<_>>()
        .join(" union all ");
    let mut sql = format!(
        "merge into {table} tgt using ({branches}) src on ({})",
        p.on_clause()
    );
    if !p.payload.is_empty() {
        let sets = p
            .payload
            .iter()
            .map(|i| {
                let c = p.col(*i);
                format!("{c} = src.{c}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" when matched then update set {sets}"));
    }
    sql.push_str(&format!(
        " when not matched then insert ({}) values ({})",
        p.col_list,
        p.src_cols()
    ));
    sql
}

// ==================== Oracle 族：表选项 / 索引 ====================

/// Oracle：估算行数（num_rows，统计信息）、表空间、最近 ANALYZE 时间、注释。
///
/// **别名 `rows` / `comment` 必须带双引号**：Oracle 里两者都是关键字，写 `as rows`
/// 直接报 ORA-00923 —— 整条查询失败，上层是 `if let Ok(result)`，错误被静默吞掉，
/// 症状就是「表注释改完保存成功、重开还是空」（真机 SYSTEM 实测：`select 1 as rows
/// from dual` 都过不去）。
///
/// owner 用**调用方给的模式名**而不是「当前用户」：这层是模式级清单，用 user_tables
/// 只看得见连接账号自己的表 —— 浏览别人的模式时注释永远空。给的值在 all_users 里查不到
/// （例如传的是服务名 FREEPDB1）时退回当前模式。
fn oracle_table_options(ops: &dyn DialectOps, database: &str) -> String {
    let db = ops.literal(database);
    format!(
        "select t.table_name as table_name, to_char(t.num_rows) as \"rows\", \
         t.tablespace_name as tablespace, \
         to_char(t.last_analyzed, 'yyyy-mm-dd hh24:mi:ss') as lastAnalyzed, \
         coalesce(c.comments, '') as \"comment\" \
         from all_tables t \
         left join all_tab_comments c on c.owner = t.owner and c.table_name = t.table_name \
         where t.owner = coalesce((select username from all_users where username = {db}), \
           sys_context('USERENV', 'CURRENT_SCHEMA'))"
    )
}

/// DM：同一套数据字典（user_tables / user_tab_comments），别名同样加引号，
/// 但 owner 维持「当前用户」—— 达梦没有可验证的实例，不动它的语义。
fn dm_table_options() -> String {
    "select t.table_name as table_name, to_char(t.num_rows) as \"rows\", \
     t.tablespace_name as tablespace, \
     to_char(t.last_analyzed, 'yyyy-mm-dd hh24:mi:ss') as lastAnalyzed, \
     coalesce(c.comments, '') as \"comment\" \
     from user_tables t \
     left join user_tab_comments c on c.table_name = t.table_name"
        .to_string()
}

/// Oracle / DM：原来只查 user_indexes 的 name/table，**不查列** —— 所以「包含字段」
/// 恒为空。列与列序在 user_ind_columns 里；顺带把 uniqueness 归一成 `isUnique`
/// （0/1，前端归一化认这个键）、index_type 作为「类型」列。
const ORACLE_INDEXES: &str = "select i.index_name as \"name\", i.table_name as \"table\", \
     case when i.uniqueness = 'UNIQUE' then 1 else 0 end as \"isUnique\", \
     i.index_type as \"type\", \
     listagg(c.column_name, ',') within group (order by c.column_position) as \"columns\" \
     from user_indexes i \
     join user_ind_columns c on c.index_name = i.index_name and c.table_name = i.table_name \
     group by i.index_name, i.table_name, i.uniqueness, i.index_type \
     order by i.table_name, i.index_name";

// ==================== Oracle 族：schema / 表 / 例程 / 触发器 ====================

/// Oracle 的「库」那层是服务名 / PDB，表实际挂在**用户（schema）**下面 —— 树里列
/// `all_users`，就是 Navicat 左侧那一列。不开这层的话连 system 时看不到 DBMIND.T_TEST。
///
/// 但 `all_users` 也是**所有**用户（含 SYS / OUTLN 这些与当前账号毫无关系的），
/// 逐个点进去恒空。收窄成「当前用户 + 至少有一个我能看见的对象」——
/// `all_objects` 的语义正好是「当前账号有权限访问的对象」。当前用户必须无条件保留：
/// 刚建好、还没建对象的空 schema 也得能导航过去。
const ORACLE_SCHEMAS: &str = "select username as name from all_users u \
     where username = sys_context('USERENV','CURRENT_SCHEMA') \
        or exists (select 1 from all_objects o \
                   where o.owner = u.username and rownum = 1) \
     order by username";

/// 表与视图一起列（union all 两段），并且**必须带上 `type` 列** —— 前端就是按
/// `type === 'VIEW'` 把视图分到「视图」分类的：少了这一列，视图会被当成表混进「表」里，
/// 「视图」节点永远空（真机踩过：SYSTEM 下 41 个对象、视图 0）。
/// `rows` / `comment` / `type` 都加双引号：前两个在 Oracle 里是关键字，不加直接 ORA-00923。
/// SQL 写成**一整行**：续行拼接被搅坏过一次（报 ORA-03405），别再拆。
fn oracle_tables_in_schema(ops: &dyn DialectOps, schema: &str) -> String {
    format!(
        "select t.table_name as name, t.num_rows as \"rows\", cc.comments as \"comment\", 'TABLE' as \"type\" from all_tables t left join all_tab_comments cc on cc.owner = t.owner and cc.table_name = t.table_name where t.owner = {owner} union all select v.view_name as name, null as \"rows\", cc.comments as \"comment\", 'VIEW' as \"type\" from all_views v left join all_tab_comments cc on cc.owner = v.owner and cc.table_name = v.view_name where v.owner = {owner} order by name",
        owner = ops.literal(&schema.to_uppercase())
    )
}

/// 以前用 `user_objects`，它只认**会话当前模式**，而例程请求根本不收 schema ——
/// 树上点哪个模式都是 0（连 SYS 都是 0，一眼假）。改成 `all_objects` + `owner` 明确
/// 过滤：判据来自请求本身，不赌「代理有没有真的切 schema」。两边都 upper()：
/// 未加引号的标识符在字典里是大写，但用户也可能建了带引号的小写模式。
///
/// PACKAGE 不带 BODY：EXECUTE 只授予说明（spec），对 BODY 授权是语法错。
fn oracle_procedures(ops: &dyn DialectOps, schema: Option<&str>) -> Meta {
    match schema {
        Some(s) if !s.trim().is_empty() => Meta::Sql(format!(
            "select object_name as \"name\", object_type as \"routineType\" \
             from all_objects where object_type in ('PROCEDURE','FUNCTION','PACKAGE') \
             and upper(owner) = upper({}) order by object_name",
            ops.literal(s.trim())
        )),
        _ => Meta::Sql(
            "select object_name as \"name\", object_type as \"routineType\" from user_objects \
             where object_type in ('PROCEDURE','FUNCTION','PACKAGE') order by object_name"
                .to_string(),
        ),
    }
}

fn oracle_triggers(ops: &dyn DialectOps, schema: Option<&str>) -> Meta {
    match schema {
        Some(s) if !s.trim().is_empty() => Meta::Sql(format!(
            "select trigger_name as \"name\", table_name as \"table\" from all_triggers \
             where upper(owner) = upper({}) order by trigger_name",
            ops.literal(s.trim())
        )),
        _ => Meta::Sql(
            "select trigger_name as \"name\", table_name as \"table\" from user_triggers \
             order by trigger_name"
                .to_string(),
        ),
    }
}

/// DM（达梦）这里**不动**：它的数据字典大小写跟 Oracle 不完全一致，没在真机上验证过
/// owner 过滤，宁可不改，也不要造一个「看着像修好了」的改动。
fn dm_procedures() -> Meta {
    Meta::Sql(
        "select object_name as \"name\", object_type as \"routineType\" from user_objects \
         where object_type in ('PROCEDURE','FUNCTION') order by object_name"
            .to_string(),
    )
}

fn dm_triggers() -> Meta {
    Meta::Sql(
        "select trigger_name as \"name\", table_name as \"table\" from user_triggers \
         order by trigger_name"
            .to_string(),
    )
}

// ==================== Oracle 族：用户 / 权限 / 角色 ====================

fn oracle_users() -> Meta {
    Meta::Sql("select username as \"name\" from all_users order by username".to_string())
}

/// `dba_users` 一行含状态与表空间；授权拼成 grants_text 文本（系统权限 + 角色，
/// 与 PG 的 grants_text 同一交付形态）。用 `dba_*` 而不是 `all_users`：后者没有
/// 状态/表空间字段，详情页会没内容可看。`upper()` 兜住大小写 —— Oracle 里用户名按大写存。
fn oracle_user_info(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
        "select username as name, account_status, \
         to_char(created, 'YYYY-MM-DD HH24:MI:SS') as created, \
         nvl(default_tablespace, '') as default_tablespace, \
         nvl(temporary_tablespace, '') as temporary_tablespace, \
         nvl(profile, '') as profile, \
         nvl((select listagg('grant ' || privilege || ' to ' || grantee || ';', chr(10)) \
           within group (order by privilege) from dba_sys_privs s where s.grantee = u.username), '') || \
         nvl((select chr(10) || listagg('grant ' || granted_role || ' to ' || grantee || ';', chr(10)) \
           within group (order by granted_role) from dba_role_privs r where r.grantee = u.username), '') as grants_text \
         from dba_users u where username = upper({})",
        ops.literal(name)
    ))
}

/// 三张字典视图正好对应三层 —— 系统权限 / 角色 / 对象权限。列形状与 MySQL 那三张视图
/// **保持一致**（scope, privilege, owner, object），前端一套解析器就能吃两家。
///
/// 视图用 `dba_*`：它们要 SELECT_CATALOG_ROLE —— 没有权限时这句会失败，界面会**如实报
/// "读不到现有权限"**（而不是显示成"什么都没有"，那会让人误以为可以放心保存）。
/// SQL 一律写成**单行**：续行的反斜杠拼接被搅坏过。
/// 权限名用 `PRIVILEGE` 而不是 `PRIVILEGE_CN`，避免用到只有中文环境才有的列。
fn oracle_user_grants(ops: &dyn DialectOps, user: &str) -> Meta {
    Meta::Sql(format!(
        "select 'global' as scope, privilege as privilege, '' as owner, '' as object_name from dba_sys_privs where grantee = upper({lit}) \
         union all select 'role', granted_role, '', '' from dba_role_privs where grantee = upper({lit}) \
         union all select 'object', privilege, owner, table_name from dba_tab_privs where grantee = upper({lit})",
        lit = ops.literal(user)
    ))
}

fn oracle_role_info(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
        "select role as name, '' as host from dba_roles where role = upper({})",
        ops.literal(name)
    ))
}

/// Oracle 有两本角色字典：系统权限与对象权限（角色套角色不在权限矩阵里）。
fn oracle_role_grants(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
        "select 'global' as scope, '' as owner, '' as object, p.privilege as privilege, p.admin_option as grant_option \
           from role_sys_privs p where p.role = upper({lit}) \
         union all \
         select 'object' as scope, t.owner as owner, t.table_name as object, t.privilege as privilege, t.grantable as grant_option \
           from role_tab_privs t where t.role = upper({lit})",
        lit = ops.literal(name)
    ))
}

/// `DBA_ROLES` 里有 100+ 个 Oracle 自带角色（DBA / CONNECT / DV_* …），全灌进用户清单
/// 既吵又危险（删掉 DBA 是真事故）。`ORACLE_MAINTAINED='N'` 正好筛出**用户自己建的**
/// 角色（12c+ 才有这一列；11g 上这句会失败，调用方按"拿不到就不补"处理，清单照常）。
fn oracle_role_inventory() -> Meta {
    Meta::Sql(
        "select role as role_name, '' as role_host from dba_roles where oracle_maintained = 'N'".to_string(),
    )
}

// ==================== Oracle 用户管理的拼装件 ====================

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
            parts.push(format!("grant {} to {u}", oracle_role_ident(raw)));
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
            quote_oracle(owner),
            quote_oracle(object)
        ));
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
                if owner.is_empty() || object.is_empty() || !crate::api::dialect::is_oracle_object_priv(&name)
                {
                    continue;
                }
                parts.push(format!(
                    "revoke {name} on {}.{} from {u}",
                    quote_oracle(owner),
                    quote_oracle(object)
                ));
            }
            "role" => {
                let raw = row.privilege.trim();
                // 预置角色 + 用户自定义角色（后端建的那些），同一套写法见 oracle_grant_parts
                if crate::api::dialect::is_oracle_role(&name) || crate::api::dialect::is_plain_ident(raw) {
                    parts.push(format!("revoke {} from {u}", oracle_role_ident(raw)));
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
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if plain {
        s.to_ascii_uppercase()
    } else {
        quote_oracle(s)
    }
}

// ==================== 对象定义的拼装件 ====================

/// Oracle 的 `dbms_metadata.get_ddl` 拼装（`CREATE OR REPLACE ...` 全文，
/// 比逐行拼 `user_source` 可靠）。**必须带 owner**（第三个参数）—— 不带就按会话
/// 默认模式找，在 SYS 下双击一个过程而会话是 SYSTEM 时会报 ORA-31603
/// 「对象在方案 SYSTEM 中未找到」。owner 也 `upper()`：未加引号的标识符在字典里
/// 是大写，但用户可能建了带引号的小写模式。
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

/// Oracle/DM 行编辑取值的日期字面量：`to_date`/`to_timestamp` 显式格式串。
///
/// 豫园 Oracle 11g 的真机故障：表格里改一行，`day` 列原值 `2026-02-01T00:00` 原样
/// 回传 ⇒ `ORA-01861: 文字与格式字符串不匹配`（会话 NLS 默认 `DD-MON-RR`）。
pub(crate) fn oracle_style_temporal_literal<T: DialectOps>(
    ops: &T,
    hint: &str,
    value: &str,
) -> Option<String> {
    let hint = hint.to_ascii_uppercase();
    if !hint.contains("DATE") && !hint.contains("TIME") {
        return None;
    }
    // `timestamp` / `datetime` 走 to_timestamp（保留小数秒）；纯 `date` 走 to_date
    let is_timestamp = hint.contains("TIMESTAMP") || hint.starts_with("DATETIME");

    let text = value.trim();
    let (date_part, time_part) = match text.find(|c: char| c == ' ' || c == 'T') {
        Some(index) => (&text[..index], text[index + 1..].trim()),
        None => (text, ""),
    };
    // 只认 `YYYY-MM-DD`：别的写法（`02-01-2026`、带时区、自然语言）不猜
    let date_ok = date_part.len() == 10
        && date_part.as_bytes()[4] == b'-'
        && date_part.as_bytes()[7] == b'-'
        && date_part.chars().filter(char::is_ascii_digit).count() == 8;
    if !date_ok {
        return None;
    }
    if !time_part.is_empty()
        && !time_part
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ':' | '.'))
    {
        return None;
    }

    let mut normalized = date_part.to_string();
    let mut format = String::from("YYYY-MM-DD");
    if !time_part.is_empty() {
        let (clock, fraction) = match time_part.split_once('.') {
            Some((clock, fraction)) => (clock, Some(fraction)),
            None => (time_part, None),
        };
        if clock.is_empty() || clock.matches(':').count() > 2 {
            return None;
        }
        let with_seconds = clock.matches(':').count() == 2;
        normalized.push(' ');
        normalized.push_str(clock);
        format.push_str(" HH24:MI");
        if with_seconds {
            format.push_str(":SS");
        }
        // 小数秒只有 TIMESTAMP 有；DATE 列丢掉（Oracle 的 DATE 只到秒）
        if is_timestamp {
            if let Some(fraction) = fraction {
                if !fraction.is_empty() {
                    normalized.push('.');
                    normalized.push_str(fraction);
                    format.push_str(".FF");
                }
            }
        }
    }

    let literal = ops.literal(&normalized);
    let func = if is_timestamp { "to_timestamp" } else { "to_date" };
    Some(format!("{func}({literal}, '{format}')"))
}
