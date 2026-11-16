//! SQL Server：`OFFSET … FETCH` 必须带 `ORDER BY`，三段名是「库.模式.表」。

use super::*;

impl DialectOps for SqlServerOps {
    fn object_source(&self, object_type: &str, name: &str, _schema: Option<&str>) -> Meta {
        let name_literal = self.literal(name);
        match object_type {
            // SQL Server 没有事件调度器
            "event" => Meta::Absent,
            // 视图/例程/触发器/表：object_definition 就是原始 create 文本
            _ => Meta::Sql(format!(
                "select object_definition(object_id({name_literal})) as definition"
            )),
        }
    }

    fn portable_time_literal(&self, portable: &'static str) -> Option<&'static str> {
        match portable {
            // T-SQL 没有 CURRENT_DATE；省略而不是硬塞一个不合法写法：
            // 否则整张表建不出来（真机实测语法错）
            "current_date" => Some("cast(getdate() as date)"),
            _ => Some(portable),
        }
    }

    fn ddl_extras(&self, table: &str) -> Vec<(&'static str, String)> {
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

    fn post_create_comments(
        &self,
        table: &str,
        column_comments: &std::collections::HashMap<String, String>,
        table_comment: Option<&str>,
        schema: &str,
    ) -> Vec<String> {
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
            if comment.trim().is_empty() {
                continue;
            }
            out.push(render(name, comment));
        }
        out
    }

    fn ddl(&self, table: &str, schema: Option<&str>) -> Meta {
        let _ = schema;
        let lit = self.literal(table);
        // SQL Server：没有内置函数，自己拼。两条关键约定：
        // - **视图走 `object_definition`**（它就是原始 create view 文本），表才需要拼列
        // - 用 `for xml path('')` 而不是 `string_agg`：后者要 SQL Server 2017+，
        //   而这里拼的是要贴给用户直接执行的 DDL，不该带一个「你的版本太老」的隐性前提
        // - 分隔符用 `case ... < max(column_id)`：**不能**每列后都加逗号，
        //   否则没有主键的表会以 `,)` 结尾，那在 SQL Server 上是语法错误
        Meta::Sql(format!(
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
    ))
    }

    fn table_action(&self, table: &str, action: &str, new_name: Option<&str>) -> Meta {
        // 改表名只有 `sp_rename`（之前落进通用分支只会拿到一句语法错）。两个细节：
        // sp_rename 收的是**裸名**（它自己解析，不要标识符引号）；并且**不会更新**
        // 引用该表的视图/存储过程（前端会给这句提醒）。
        if action == "rename" {
            return match new_name {
                Some(new_name) => Meta::Sql(format!(
                    "exec sp_rename {}, {}",
                    self.literal(table),
                    self.literal(new_name)
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
        let pwd = self.literal(password);
        let role_parts: Vec<String> = roles.iter().map(|role| self.quote(role)).collect();
        Meta::Sql(match action {
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
                    if db.is_empty() {
                        String::new()
                    } else {
                        format!("use {}; ", self.quote(db))
                    }
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
                        "serverrole" => {
                            format!("alter server role {} drop member {u}", self.quote(&r.privilege))
                        }
                        "role" => format!("alter role {} drop member {u}", self.quote(&r.privilege)),
                        "schema" => format!(
                            "{}revoke {} on schema::{} from {u}",
                            pre,
                            r.privilege,
                            self.quote(&r.owner)
                        ),
                        "object" => format!(
                            "{}revoke {} on {}.{} from {u}",
                            pre,
                            r.privilege,
                            self.quote(&r.owner),
                            self.quote(&r.object)
                        ),
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
        })
    }

    fn users(&self) -> Meta {
        sqlserver_users()
    }

    fn user_info(&self, name: &str) -> Meta {
        sqlserver_user_info(self, name)
    }

    fn schemas_sql(&self) -> Option<&'static str> {
        Some(SQLSERVER_SCHEMAS)
    }

    fn tables_in_schema_sql(&self, schema: &str) -> Option<String> {
        Some(sqlserver_tables_in_schema(self, schema))
    }

    fn procedures(&self, _schema: Option<&str>) -> Meta {
        sqlserver_procedures()
    }

    fn triggers(&self, _schema: Option<&str>) -> Meta {
        sqlserver_triggers()
    }

    fn table_options_sql(&self, _database: &str) -> Option<String> {
        Some(sqlserver_table_options())
    }

    fn indexes_sql(&self) -> Option<&'static str> {
        Some(SQLSERVER_INDEXES)
    }

    fn upsert_sql(
        &self,
        table: &str,
        columns: &[String],
        key_indexes: &[usize],
        rows: &[Vec<String>],
    ) -> Option<String> {
        let p = upsert_parts(self, columns, key_indexes, rows)?;
        Some(merge_values_upsert(&p, table, true))
    }
    fn supports_drop_object(&self, kind: &str) -> bool {
        // SQL Server 2016+ 才有 `drop … if exists`
        matches!(kind, "view" | "procedure" | "function" | "trigger")
    }
    fn rename_object_sql(&self, kind: &str, from: &str, to: &str, schema: Option<&str>) -> Option<String> {
        // `sp_rename` 是唯一入口，它**只改目录**（`sys.objects.name`），不改
        // `sys.sql_modules.definition` 里的正文 —— 这正是源库里「树上 test1111、
        // 定义里 CREATE PROCEDURE [test111]」的来历，要复刻它只能靠同一句 sp_rename。
        // 参数是**字符串字面量**（不是标识符），且**必须带模式**，
        // 否则按调用者的默认模式解析（改错对象或报「找不到」）。
        if !matches!(kind, "view" | "procedure" | "function" | "trigger") {
            return None;
        }
        let schema = schema.map(str::trim).filter(|s| !s.is_empty());
        let old = match schema {
            Some(schema) => format!("{schema}.{from}"),
            None => from.to_string(),
        };
        Some(format!(
            "exec sp_rename N'{}', N'{}'",
            old.replace('\'', "''"),
            to.replace('\'', "''")
        ))
    }
    fn column_types_sql(&self, quoted: &str) -> Meta {
        // SQL Server：character_maximum_length 按**字符**给（nvarchar(50) → 50），
        // `-1` 是 `max`；decimal 走 numeric_precision/scale。
        // 默认 schema 优先排序：同名表出现在多个 schema 时，取当前 schema 那条。
        //
        // **`text` / `ntext` / `image` 的 `character_maximum_length` 是 2147483647，不是 -1**。
        // 只按 -1 判「无界」时，这三个旧大对象类型会被拼成 `text(2147483647)`，
        // SQL Server 建表直接报 2715「The size (2147483647) given to the column 'X'
        // exceeds the maximum allowed for any data type (8000)」——真机整库同步里
        // 3 张带 `text` 列的表因此整表漏搬，而界面「失败 0」（建表失败只在日志里）。
        // 这几类本身就是无界的，正确写法就是裸类型名。
        Meta::Sql(format!(
            "select column_name as column_name, \
             case \
               when character_maximum_length = -1 then data_type + '(max)' \
               when data_type in ('text', 'ntext', 'image') then data_type \
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
        ))
    }
    fn database_stats_sql(&self) -> Option<&'static str> {
        Some(
            "select (select count(*) from sys.tables) as tables, \
             cast(cast(coalesce(sum(size), 0) * 8.0 / 1024 as decimal(10, 1)) as varchar(20)) + ' MB' as sizeText \
             from sys.database_files",
        )
    }
    fn table_comments_sql(&self, quoted: &str) -> Meta {
        // 这里**故意**用 `isnull` 而不是 `coalesce`：`sys.extended_properties.value` 是
        // `sql_variant`，`coalesce` 要求两侧类型可统一而直接报错；
        // `isnull` 返回第一个参数的类型，正好能接后面的 cast。别顺手统一掉。
        Meta::Sql(format!(
            "select c.name as column_name, cast(isnull(ep.value, '') as nvarchar(400)) as comment \
             from sys.columns c \
             left join sys.extended_properties ep on ep.major_id = c.object_id and ep.minor_id = c.column_id \
             where c.object_id = object_id({quoted})"
        ))
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
        Some("select name from sys.databases where state = 0 and has_dbaccess(name) = 1 order by name")
    }
    fn wrap_ident(&self, text: &str) -> String {
        format!("[{}]", text.replace(']', "]]"))
    }
    fn is_schema_aware(&self) -> bool {
        true
    }
    fn wants_use_switch(&self) -> bool {
        true
    }
    fn switchable_database(&self) -> bool {
        true
    }
    fn family(&self) -> &'static str {
        "sqlserver"
    }
    fn supports_native_upsert(&self) -> bool {
        true
    }
    fn upsert_row_limit(&self) -> usize {
        // T-SQL 行值构造器单语句上限 1000 行（解析器硬限制）
        1000
    }
    fn limit_clause(&self, offset: u64, size: u64) -> String {
        format!("offset {offset} rows fetch next {size} rows only")
    }
    fn version_sql(&self) -> Option<&'static str> {
        Some("select @@version as v")
    }
    fn needs_order_by_for_paging(&self) -> bool {
        true
    }
    fn direct_ref_depth(&self) -> DirectRefDepth {
        DirectRefDepth::DatabaseSchema
    }
    fn instance_fingerprint_sql(&self) -> Option<&'static str> {
        Some("select convert(nvarchar(256), serverproperty('ServerName')) as v")
    }
}

// ==================== SQL Server：表选项 / 索引 ====================

/// 真实行数（sys.partitions 的 heap/聚簇索引行）、建表时间、表注释仍在扩展属性
/// `MS_Description`（与列注释同一处）。
fn sqlserver_table_options() -> String {
    "select t.name as table_name, cast(p.rows as varchar(20)) as rows, \
     convert(varchar(19), t.create_date, 120) as createTime, \
     cast(ep.value as nvarchar(4000)) as comment \
     from sys.tables t \
     join sys.partitions p on p.object_id = t.object_id and p.index_id in (0, 1) \
     left join sys.extended_properties ep \
       on ep.major_id = t.object_id and ep.minor_id = 0 and ep.name = 'MS_Description'"
        .to_string()
}

/// 别名 `schema` 必须加双引号：SCHEMA 是 T-SQL 的保留字，写 `as schema` 直接
/// `Incorrect syntax near the keyword 'schema'` —— 整条索引查询 500，「索引」页签永远是空的。
const SQLSERVER_INDEXES: &str = "select schema_name(o.schema_id) as \"schema\", i.name as name, \
     o.name as \"table\", i.is_unique as isUnique, \
     stuff((select ',' + c.name from sys.index_columns ic \
      join sys.columns c on c.object_id = ic.object_id and c.column_id = ic.column_id \
      where ic.object_id = i.object_id and ic.index_id = i.index_id and ic.key_ordinal > 0 \
      order by ic.key_ordinal for xml path('')), 1, 1, '') as \"columns\" \
     from sys.indexes i join sys.objects o on o.object_id = i.object_id \
     where o.type = 'U' and i.name is not null order by o.name, i.name";

// ==================== SQL Server：schema / 表 / 例程 / 触发器 ====================

/// 除了 guest / sys / INFORMATION_SCHEMA，还要滤掉**九个固定数据库角色的同名 schema**
/// （db_owner、db_accessadmin、db_backupoperator、db_datareader、db_datawriter、
/// db_ddladmin、db_denydatareader、db_denydatawriter、db_securityadmin）。
/// 每个固定角色在库里都对应一个空 schema，SQL Server 建库时自动带上 —— 它们不是给用户
/// 建表用的，列在树里就是九行永远空的噪音（实测用户截图里正是这九个）。
/// `dbo` 要留着：它是正常的默认 schema（用户建的表大多在它下面）。
const SQLSERVER_SCHEMAS: &str = "select name from sys.schemas \
     where name not in ('guest','sys','INFORMATION_SCHEMA', \
       'db_owner','db_accessadmin','db_backupoperator','db_datareader','db_datawriter', \
       'db_ddladmin','db_denydatareader','db_denydatawriter','db_securityadmin') \
     order by name";

/// `sys.partitions.rows` 是**实时维护的真实行数**（不是统计估算），一次 join 就能拿到
/// 全 schema，不必每张表去 COUNT(*)。视图没有自己的行数 → null（留空，而不是显示 0）。
///
/// 表注释必须自己 join：**mssql-jdbc 不填 `REMARKS`**（SQL Server 的表注释存在
/// `sys.extended_properties` 里），所以走 JDBC 的 getTables 永远拿不到注释 —— 而前端
/// 「编辑表结构」的注释框正是从这个清单里读的。缺这一列的后果实测是：注释改完保存
/// **确实成功**，但重新打开又是空的 —— 用户看到的就是"修改表注释没用"。
/// `minor_id = 0` 才是表级注释（列级是列号）；`class = 1` 是对象级。
fn sqlserver_tables_in_schema(ops: &dyn DialectOps, schema: &str) -> String {
    format!(
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
        ops.literal(schema)
    )
}

fn sqlserver_procedures() -> Meta {
    Meta::Sql(
        "select name, 'PROCEDURE' as routineType from sys.procedures \
         union all select name, 'FUNCTION' as routineType from sys.objects \
         where type in ('FN','IF','TF') order by name"
            .to_string(),
    )
}

fn sqlserver_triggers() -> Meta {
    Meta::Sql(
        "select t.name as name, o.name as \"table\" from sys.triggers t \
         left join sys.objects o on o.object_id = t.parent_id order by t.name"
            .to_string(),
    )
}

// ==================== SQL Server：用户清单 / 详情 ====================

/// **服务器登录名**（不是库用户）。账号体系是「服务器登录名 + 每个库里的映射用户」，
/// 而这页是**连接级**（不跟库）：原来列的是库用户（sys.database_principals），
/// 于是打开 demo 这种只有 dbo/guest 的库永远是一片空白 —— 用户的原话是
/// 「SQL Server 的用户也显示出来吧」。登录名才是用户眼里的"用户"，也与增删改
/// （create/alter/drop login）同一口径。
///
/// 只过滤系统内置：`##…` 是证书/策略签名登录，`NT …` / `BUILTIN…` 是机器账号。
/// `sa` 必须保留 —— 那正是用户要找的那个。
///
/// `isSuper` 用 `is_srvrolemember('sysadmin', name)` 现场算（实测 sa → 1），这样
/// 「超级用户」标签在这一页才有意义；非 sysadmin 账号调用得到 NULL，折成 false，
/// 不会把所有人都标成超级用户。
fn sqlserver_users() -> Meta {
    Meta::Sql(
        "select name, is_srvrolemember('sysadmin', name) as \"isSuper\" \
         from sys.server_principals \
         where type in ('S','U','G') \
           and name not like '##%' \
           and name not like 'NT %' \
           and name not like 'BUILTIN%' \
         order by name"
            .to_string(),
    )
}

/// **服务器登录名**（sys.server_principals）+ 服务器角色 + 服务器级权限，与 `users()`
/// 同一口径（连接级、不跟库）。别名必须是 `type_desc`（不是 `type`）：前端标签表按
/// `type_desc` 认「类型」。
fn sqlserver_user_info(ops: &dyn DialectOps, name: &str) -> Meta {
    Meta::Sql(format!(
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
         where sp.name = {} and sp.type in ('S','U','G')",
        ops.literal(name)
    ))
}
