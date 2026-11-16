//! 用户与权限：用户/角色清单、账号详情、user-action（建/改/删/授权/撤权）与授权行解析。
//!
//! 方言差异经 `dialect.user_action_sql` / `user_grants` / `role_*` 等 trait 方法分发；
//! grant 行的解析与白名单校验（伪造权限名必须被拒）在这里，守门单测在文件尾。

use super::*;

pub async fn users(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    // 参数照旧解析（老客户端会带 database），但**不再使用** —— 见下面注释
    let _params = Params::parse(raw.as_deref());
    let (_, dialect) = ctx_of(&state, &id).await?;
    // 用户清单都是**全局目录**（mysql.user / pg_roles / all_users / system.users /
    // SQL Server 的 sys.server_principals 登录名），与"当前库"无关，一律不带库上下文。
    // 带库的后果很实在：Doris 实测连接带着 `test` 库打开用户管理时，驱动先 `use test`，
    // 直接报「Unknown database 'test'」（errCode 2 / 1049），整个清单都拉不出来。
    // （SQL Server 以前是唯一例外 —— 那时列的是库用户；现在改成登录名，见 dialect.users。）
    let database = String::new();
    let value = meta_list(&state, &id, &database, dialect.users(), "用户清单")
        .await
        .map_err(|err| permission_hint(err, "用户清单"))?;
    // **角色不进用户列表**（用户要求）：角色在「编辑 / 授权」的「角色」页签里看与管理 ——
    // 那一页本来就要列出全部角色（能授才有意义），新建 / 删除也放在那里，同一处闭环。
    // 这里的清单只有账号：MySQL/MariaDB 的角色账号在 `dialect.users()` 的 SQL 里已排除，
    // Oracle / ClickHouse / Doris 的账号表本来就不含角色。
    let normalized = match value {
        Value::Array(rows) => Value::Array(
            rows.into_iter()
                .map(|row| {
                    let mut out = row.as_object().cloned().unwrap_or_default();
                    // 归一成布尔（`is_true_flag` 里写了为什么：PG 回来的是数字 1，
                    // 字符串 "0" 在前端又是真值）
                    let is_super = out.get("isSuper").map(is_true_flag).unwrap_or(false);
                    out.insert("isSuper".to_string(), Value::Bool(is_super));
                    out.entry("comment".to_string()).or_insert_with(|| {
                        Value::String(if is_super {
                            "超级用户".into()
                        } else {
                            String::new()
                        })
                    });
                    Value::Object(out)
                })
                .collect(),
        ),
        other => other,
    };
    Ok(Json(normalized))
}

// ------------------------------------------------------------------ 角色清单

/// 「目录读不到」时给一句人话（**只改文案，不改判定**）。
///
/// 用户管理/角色管理读的都是**管理员目录**（`mysql.user` / `mysql.role_edges` /
/// `dba_roles` / `system.users` …）。普通账号去查会拿到驱动原文，例如
/// `SELECT command denied to user 'x'@'1.2.3.4' for table 'user'` —— 界面上看起来
/// 像"这个功能坏了"，其实只是"这个账号没权限"。认得出的权限类错误改写成
/// 「当前连接账号没有查看 X 的权限」，其余错误原样返回。
fn permission_hint(err: XError, feature: &str) -> XError {
    // 各家原文长这样（都实测过）：
    //   MySQL   `SELECT command denied to user 'x'@'1.2.3.4' for table 'user'`
    //   PG      `permission denied for table pg_authid`
    //   Oracle  `ORA-00942: 表或视图 "SYS"."DBA_ROLES" 不存在`（无数据字典权限时就是这个）
    //   ClickHouse `Not enough privileges. … SELECT(name) ON system.users`
    //   DB2     `SQLCODE=-551 … does not have the privilege`
    // 统一小写后比对，避免大小写差异把某家漏掉。
    const DENIED: [&str; 11] = [
        "denied to user",
        "access denied",
        "permission denied",
        "not authorized",
        "insufficient privilege",
        "not enough privileges",
        "access_denied",
        "ora-00942",
        "ora-01031",
        "sqlcode=-551",
        "sqlstate=42501",
    ];
    let lower = err.message.to_ascii_lowercase();
    if !DENIED.iter().any(|needle| lower.contains(needle)) {
        return err;
    }
    let hint =
        format!("当前连接账号没有查看「{feature}」的权限。请改用有该目录读权限的账号，或让 DBA 授权后重连。");
    XError::with_detail(
        StatusCode::FORBIDDEN,
        format!("{hint}原始提示：{}", err.message),
        hint,
        err.message.clone(),
    )
}

/// 角色清单（「用户管理」页的「角色管理」页签用）。
///
/// 返回 `{ supported, roles: [{ name, host, comment }], message }` —— 三个字段各有分工：
/// · `supported=false`：这个**服务端版本**没有角色，或这个方言根本没有「角色」这个概念。
///   界面据此**只显示「用户管理」一个页签**（用户要求：不支持角色就别摆那个页签）。
///   最典型的是 MySQL 5.7 / MariaDB 10.0.4 以前：`mysql.role_edges` 这张表根本不存在，
///   `role_inventory()` 的 SQL 一查就报「表不存在」—— 那不是错误，是这个版本真没有角色。
/// · `roles`：角色清单本身（MySQL 的角色是 `名字@主机`，其余方言只有名字）。
/// · `message`：拿不到清单时的**原始原因**（只在 `supported=false` 时有值，界面按需展示）。
pub async fn roles(State(state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    let (_, dialect) = ctx_of(&state, &id).await?;
    let sql = match dialect.role_inventory() {
        Meta::Sql(sql) => sql,
        // 这个概念在方言里就没有（SQLite / H2 / DB2 / Derby / SQL Server：角色不是独立主体
        // 或压根没有角色）：不是"还没实现"，是"没有这个东西"，两者在这里都是"不显示页签"。
        _ => return Ok(Json(json!({ "supported": false, "roles": [], "message": "" }))),
    };
    // 角色是**全局目录**（mysql.role_edges / pg_roles / dba_roles / system.roles / SHOW ROLES），
    // 与用户清单同一个理由：一律不带库上下文（带库会让 Doris 报 Unknown database）。
    let database = String::new();
    let result = match run_sql_in(&state, &id, &database, sql, META_MAX_ROWS).await {
        Ok(result) => result,
        // 版本没有角色（或者这台库上读不到角色目录）：如实说"没有角色清单"，
        // 不把一条红色报错甩到用户面前 —— 但也**不静默**，原因随 message 带回去。
        Err(e) => {
            return Ok(Json(json!({
                "supported": false,
                "roles": [],
                "message": permission_hint(e, "角色清单").message,
            })))
        }
    };
    let mut out: Vec<Value> = Vec::new();
    for row in rows_of(&result) {
        // Doris 的 `SHOW ROLES` 列名是 Name；其余方言由 SQL 起别名 role_name / role_host。
        // 列名大小写各家不同，一律按名字忽略大小写取（与 user_info 同一条经验）。
        let name = pick_field(&row, &["role_name", "Name", "rolename"]);
        if name.trim().is_empty() {
            continue;
        }
        out.push(json!({
            "name": name,
            "host": pick_field(&row, &["role_host", "host"]),
            // Doris 的 SHOW ROLES 带 Comment；其余方言没有这一列 → 空串（界面显示破折号）
            "comment": pick_field(&row, &["comment", "description"]),
        }));
    }
    Ok(Json(json!({ "supported": true, "roles": out, "message": "" })))
}

pub async fn user_info(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let name = params.get("name").unwrap_or_default();
    if name.trim().is_empty() {
        return Ok(Json(json!({ "error": "缺少 name 参数" })));
    }
    let _ = params.get("database");
    let (_, dialect) = ctx_of(&state, &id).await?;
    // 与用户清单同一条规矩：用户详情是全局目录查询（mysql.user / pg_roles / all_users /
    // system.users / SQL Server 的 sys.server_principals 登录名），**不带库上下文** ——
    // Doris 带着不存在的库打开会直接「Unknown database」；
    // SQL Server 更狠：界面顺手带上当前库（"default"）时，驱动先 `use default`，
    // 直接报「Cannot open database "default" requested by the login」，整套用户管理打不开。
    let database = String::new();
    // `role=1`：编辑**角色**自身的权限。角色不是账号（Oracle / CH / Doris 根本不在
    // 账号目录里），不能拿 `user_info` 去查 —— 那会得到"没有找到用户"。
    let as_role = params
        .get("role")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    // 界面传来的 `name` 是**列表里那一行的标签**：MySQL 系是 `名字@主机`（角色也是账号形状），
    // 其余方言就是裸名字。角色这条路走的是 `role_info` / `role_grants`，它们只要**角色名** ——
    // 不切开的话 MySQL 的 `role_info` 会拿 `user = 'r@%'` 去比，结果 0 行，
    // 界面表现为「没有找到用户」（实测 MySQL 8.0.46 的角色页打不开）。
    // 账号那条路不用动：`user_info` 自己会按最后一个 `@` 拆成两列。
    let role_label = name.trim();
    let role_name = if as_role && dialect.kind.key() == "mysql" {
        role_label.rsplit_once('@').map(|(n, _)| n).unwrap_or(role_label)
    } else {
        role_label
    };
    let info_meta = if as_role {
        dialect.role_info(role_name)
    } else {
        dialect.user_info(name.trim())
    };
    let sql = match info_meta {
        Meta::Sql(sql) => sql,
        _ => {
            return Ok(Json(json!({
                "error": format!(
                    "{} 的用户详情尚未接入（要按其系统表单独实现）",
                    dialect.kind.key().to_ascii_uppercase()
                )
            })))
        }
    };
    let result = run_sql_in(&state, &id, &database, sql, 1).await?;
    let Some(row) = rows_of(&result).into_iter().next() else {
        return Ok(Json(json!({ "error": format!("没有找到用户「{name}」") })));
    };
    // 权限明细：能**逐行查**的方言（MySQL 系）走第二条查询 + 本地拼行 ——
    // 不让 SQL 用 `group_concat` 拼一行，因为它有 1024 的会话上限会静默截断（见 `user_grants`）。
    // 其余方言（PG / SQL Server）本来就在 SQL 里用 `string_agg` / `for xml path` 拼好了
    // `grants_text`，且没有长度上限，所以沿用。
    // 列名大小写各家不同：MySQL 回来的是小写，Oracle 是 `NAME` —— 必须按名字取、忽略大小写。
    // 曾经写死 `row.get("name")`：Oracle 上取不到 → 账号名成了空串 →
    // 权限查询变成 `grantee = upper('')` → 一行都查不到，界面显示成"这个用户没有权限"
    // （真机上就是这么误导人的：明明刚授过权）。
    let account_user = pick_field(&row, &["name", "user", "username"]);
    let account_host = pick_field(&row, &["host"]);
    let grant_meta = if as_role {
        dialect.role_grants(&account_user, &account_host)
    } else {
        dialect.user_grants(&account_user, &account_host)
    };
    let (mut grants, grant_rows): (Vec<Value>, Vec<Value>) = match grant_meta {
        Meta::Sql(query) => {
            let detail = expand_grant_rows(
                run_sql_in(&state, &id, &database, query, 10_000)
                    .await
                    .map(|result| rows_of(&result))
                    .unwrap_or_default(),
            );
            let lines = detail
                .iter()
                .filter_map(|item| grant_line(item, &dialect, &account_user, &account_host))
                .map(|line| json!(line))
                .collect();
            // 结构化的一份（`scope / privilege / owner / object`）：界面要按层级回填成
            // 可编辑的表单，再据此算"哪些被取消了"（撤销清单）。只有拼好的
            // `GRANT ...` 文本没法可靠反解 —— 权限名里就带空格（`CREATE ANY TABLE`）。
            // 只有 4 列形状的方言（MySQL 的三张视图 / Oracle 的三本字典）才给，其余空数组。
            (lines, grant_rows_of(&detail))
        }
        // `grants_text` 是换行分隔的文本，界面会对它 `.map()` —— 必须转成数组，
        // 给字符串会在前端直接抛异常
        _ => (
            // 列名大小写各家不同（H2 未加引号时会把别名全大写成 GRANTS_TEXT）：
            // 按名字忽略大小写取，别再写死小写 —— 否则权限整段被当成普通字段丢掉，
            // 界面上就是"这个用户没有权限"。
            pick_field(&row, &["grants_text"])
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| json!(line))
                .collect(),
            Vec::new(),
        ),
    };
    // **角色**这一层：它不在 information_schema 的三张权限视图里
    // （见 dialect.user_roles 的说明），单独查一次补进结构化行 —— 否则「角色」在界面上
    // 彻底隐身，用户授过的角色既看不见也撤不掉。
    // 覆盖所有有独立角色主体的方言：MySQL/MariaDB（role_edges / roles_mapping）、
    // ClickHouse（system.role_grants）、以及「有角色但本用户不一定有」的
    // Oracle / Doris / PostgreSQL（它们只需要候选清单，`user_roles` 返回 Unwritten）。
    let mut available_roles: Vec<Value> = Vec::new();
    let mut grant_rows = grant_rows;
    // 角色**也能被授角色**（PG 的成员关系可以层层嵌套），所以 as_role 时同样要查这一层 ——
    // 只在用户表单里查的话，"角色管理 → 编辑权限"里永远看不到它继承了哪些角色，
    // 更撤不掉（PG 实测：勾选状态全空，取消勾选也就无从发生）。
    // 有独立角色主体的方言在 dialect.user_roles / role_inventory 里以 Meta::Sql 表达，
    // 没有的返回 Unwritten —— 三态即能力，不再按 key 列名单。
    {
        if let Meta::Sql(q) = dialect.user_roles(&account_user, &account_host) {
            if let Ok(res) = run_sql_in(&state, &id, &database, q, 1000).await {
                for item in rows_of(&res) {
                    let rn = pick_field(&item, &["role_name"]);
                    let rh = pick_field(&item, &["role_host"]);
                    if rn.is_empty() {
                        continue;
                    }
                    // MySQL 的角色是账号形状（`名字@host`）；MariaDB 的角色**没有 host**，
                    // 拼成 `名字@` 会在界面上多出一列对不上的假角色（真机回归发现）。
                    let mysql_style = dialect.kind.key() == "mysql";
                    let label = if mysql_style && !rh.is_empty() {
                        format!("{rn}@{rh}")
                    } else {
                        rn.clone()
                    };
                    grant_rows.push(json!({
                        "scope": "role",
                        "privilege": label,
                        "owner": rn.clone(),
                        "object": if mysql_style { rh.clone() } else { String::new() },
                    }));
                    // 文本那一份也补上（否则「原始授权」里看不到角色，只剩下结构化行）。
                    // 三档形状（MySQL 的 `'r'@'h'` / PG 的 `"r"` / 其余字面量）在 dialect.role_grant_text。
                    grants.push(json!(dialect.role_grant_text(
                        &rn,
                        &rh,
                        &account_user,
                        &account_host
                    )));
                }
            }
        }
        if let Meta::Sql(q) = dialect.role_inventory() {
            if let Ok(res) = run_sql_in(&state, &id, &database, q, 1000).await {
                for item in rows_of(&res) {
                    // Doris 的 `SHOW ROLES` 列名是 Name（其余方言由 SQL 起别名 role_name）
                    let rn = pick_field(&item, &["role_name", "Name"]);
                    let rh = pick_field(&item, &["role_host"]);
                    if rn.is_empty() {
                        continue;
                    }
                    let dup = available_roles.iter().any(|v| {
                        v.get("name").and_then(Value::as_str) == Some(rn.as_str())
                            && v.get("host").and_then(Value::as_str) == Some(rh.as_str())
                    });
                    if !dup {
                        available_roles.push(json!({ "name": rn, "host": rh }));
                    }
                }
            }
        }
    }
    let mut out = Map::new();
    for (key, value) in row {
        // 同上：大小写不敏感（H2 会把未加引号的别名大写成 GRANTS_TEXT）
        if key.eq_ignore_ascii_case("grants_text") {
            continue;
        }
        // 口令散列不进界面：MySQL 的用户详情是 `select * from mysql.user`，
        // 里面带着认证串（5.7 的 `Password` / 8.0 的 `authentication_string`）。
        // 展示出来只会被复制、截图、进日志 —— 真要看得在 SQL 编辑器里自己查。
        if matches!(
            key.to_ascii_lowercase().as_str(),
            "authentication_string" | "password"
        ) {
            continue;
        }
        // 列名原样带出去：界面同时认 `createUserSql` 与 `create_user_sql`，
        // 其余键走 `fieldLabelMap` 渲染，不需要在这里做驼峰转换
        out.insert(key, value);
    }
    // SQL Server：服务器角色得从 `alter server role [x] add member [u];` 里拆出来，
    // 界面「服务器角色」页签是按**结构化字段**回填勾选的（文本行不反解）。
    // 不拆的话 sa 明明是 sysadmin，页签里却一个勾都没有。
    // 注意位置：必须在 `grants` 被移进 out 之前处理。
    let mssql_server_roles: Vec<Value> = if dialect.kind.key() == "sqlserver" {
        grants
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|line| {
                let lower = line.to_ascii_lowercase();
                let start = lower.find("alter server role ")? + "alter server role ".len();
                let rest = &line[start..];
                let end = rest.to_ascii_lowercase().find(" add member")?;
                let name = rest[..end].trim().trim_matches(|c| c == '[' || c == ']');
                if name.is_empty() {
                    None
                } else {
                    Some(json!(name))
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    out.insert("grants".to_string(), Value::Array(grants));
    // 结构化的一份，界面按层级回填表单、并据此算撤销清单（见上面的注释）
    out.insert("grantRows".to_string(), Value::Array(grant_rows));
    // MySQL 系的「角色」页要用它列候选（服务器上出现过的角色）——
    // 没这个清单，界面上只能看到"已授的"，授不了新的。
    if !available_roles.is_empty() {
        out.insert("availableRoles".to_string(), Value::Array(available_roles));
    }
    if as_role {
        // 界面据此隐藏"角色""密码/属性"这些对角色没意义的部分
        out.insert("isRole".to_string(), json!(true));
    }
    if !mssql_server_roles.is_empty() {
        out.insert("serverRoles".to_string(), Value::Array(mssql_server_roles));
    }
    // SQL Server 的 Schema / 对象权限是**库级**目录（sys.database_permissions），
    // 而用户管理入口是连接级（不带库）。这里取一次在线用户库清单，逐库查出该登录名的
    // 显式授权，每条都带上库名 —— 界面上那两个页签本来就是**行级选库**的形态。
    // 只收显式 GRANT / WITH GRANT（state 为 G/W）：DENY 不是"拥有"，不能显示成勾选。
    if dialect.kind.key() == "sqlserver" {
        let who = dialect.literal(name.trim());
        let mut schema_rows: Vec<Value> = Vec::new();
        let mut object_rows: Vec<Value> = Vec::new();
        if let Ok(list) = run_sql_in(
            &state,
            &id,
            "",
            "select name from sys.databases where state = 0 and database_id > 4".to_string(),
            200,
        )
        .await
        {
            for db_row in rows_of(&list) {
                let db = pick_field(&db_row, &["name"]);
                if db.trim().is_empty() {
                    continue;
                }
                // 库名不能拼进 SQL 标识符位置（这里是作为 run_sql_in 的 database 参数，安全）
                // 注意对象级（class = 1）的 `major_id` 是 **object_id**，不是 schema_id：
                // 用 `schema_name()` 会拿到 NULL（实测 employees 的 SELECT 回显成空模式，
                // 前端「模式」列空白，而且保存时会被「schema 为空就跳过」静默丢掉），
                // 必须用 `object_schema_name()`。schema 级（class = 3）才是 schema_id。
                let sql = format!(
                    "select 'schema' as scope, db_name() as db_name, \
                       schema_name(p.major_id) as schema_name, '' as object_name, \
                       p.permission_name as permission_name \
                     from sys.database_permissions p \
                     join sys.database_principals u on u.principal_id = p.grantee_principal_id \
                     where u.name = {who} and p.class = 3 and p.state in ('G','W') \
                     union all \
                     select 'object', db_name(), object_schema_name(p.major_id), \
                       object_name(p.major_id), p.permission_name \
                     from sys.database_permissions p \
                     join sys.database_principals u on u.principal_id = p.grantee_principal_id \
                     where u.name = {who} and p.class = 1 and p.state in ('G','W')"
                );
                if let Ok(result) = run_sql_in(&state, &id, &db, sql, 5_000).await {
                    for item in rows_of(&result) {
                        let scope = pick_field(&item, &["scope"]);
                        let row = json!({
                            "database": pick_field(&item, &["db_name"]),
                            "schema": pick_field(&item, &["schema_name"]),
                            "object": pick_field(&item, &["object_name"]),
                            "permission": pick_field(&item, &["permission_name"]),
                        });
                        if scope.trim().eq_ignore_ascii_case("schema") {
                            schema_rows.push(row);
                        } else if !pick_field(&item, &["object_name"]).trim().is_empty() {
                            object_rows.push(row);
                        }
                    }
                }
            }
        }
        if !schema_rows.is_empty() {
            out.insert("schemaPrivileges".to_string(), Value::Array(schema_rows));
        }
        if !object_rows.is_empty() {
            out.insert("objectPrivileges".to_string(), Value::Array(object_rows));
        }
    }
    // MySQL 顺带带回服务器版本：**5.7 没有 CREATE ROLE / DROP ROLE**（8.0 才引入），
    // 界面若照 8.0 的清单展示，勾上保存必 1064 语法错（实测 5.7.42）。前端按版本隐藏。
    // 只查纯 MySQL —— MariaDB 版本串是 10.x、Doris 有自己的角色体系，不能按 5.x 一刀切。
    let server_version = if dialect.kind.key() == "mysql" {
        match run_meta_sql(&state, &id, "", "select version() as v".to_string(), 1).await {
            Ok(r) => rows_of(&r)
                .into_iter()
                .next()
                .map(|row| pick_field(&row, &["v", "version"]))
                .unwrap_or_default(),
            Err(_) => String::new(),
        }
    } else {
        String::new()
    };
    out.insert("serverVersion".to_string(), json!(server_version));
    Ok(Json(Value::Object(out)))
}

/// `POST /api/{m}/{id}/user-action` —— 用户新建 / 改密 / 删除。
///
/// **细粒度权限如实拒绝**：界面会把 `privileges / dbPrivileges / tablePrivileges /
/// schemaPrivileges / objectPrivileges / serverRoles / grants` 一起发过来，而这些 GRANT
/// 我们还没实现。若默默忽略，界面会弹「用户创建成功」，用户以为权限也授好了 ——
/// 那比直接报错糟得多（他会带着错误认知继续用）。所以：只应用 `roles`（有明确语法），
/// 其余只要非空就**在动数据库之前**拒绝，并给出替代做法。
pub async fn user_action(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let action = body
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let user = body
        .get("userName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if user.is_empty() {
        return Err(XError::bad_request("缺少 userName"));
    }
    let password = body.get("password").and_then(Value::as_str).unwrap_or("");
    let host = body.get("host").and_then(Value::as_str).unwrap_or("");
    let database = body.get("database").and_then(Value::as_str).unwrap_or("");
    let preview_only = body.get("previewOnly").and_then(Value::as_bool).unwrap_or(false);
    let options = body.get("options").cloned().unwrap_or_else(|| json!({}));
    // 可改的用户属性（配额 / 账户锁定 / 表空间 / 开关）—— 只带改动过的项。
    // 这个文件统一是"现场取字段"的写法（没有专门的请求体结构体），这里跟着来：
    // 认不出来的键会被忽略，但**不会**因此吞掉别的键（各字段单独取）。
    let attrs = {
        let a = options.get("attrs");
        let flag = |k: &str| a.and_then(|v| v.get(k)).and_then(Value::as_bool);
        let num = |k: &str| a.and_then(|v| v.get(k)).and_then(Value::as_i64);
        let text = |k: &str| {
            a.and_then(|v| v.get(k))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        crate::api::dialect::UserAttrs {
            account_locked: flag("accountLocked"),
            password_expired: flag("passwordExpired"),
            max_queries_per_hour: num("maxQueriesPerHour"),
            max_updates_per_hour: num("maxUpdatesPerHour"),
            max_connections_per_hour: num("maxConnectionsPerHour"),
            max_user_connections: num("maxUserConnections"),
            auth_plugin: text("authPlugin"),
            default_tablespace: text("defaultTablespace"),
            temporary_tablespace: text("temporaryTablespace"),
            profile: text("profile"),
            disabled: flag("disabled"),
            default_schema: text("defaultSchema"),
            default_database: text("defaultDatabase"),
            login_name: text("loginName"),
            can_login: flag("canLogin"),
            is_superuser: flag("isSuperuser"),
            can_create_db: flag("canCreateDb"),
        }
    };
    let (_, dialect) = ctx_of(&state, &id).await?;
    // 与 user_info 同一条规矩（用户详情是全局目录，不带库上下文）：只有 SQL Server
    // 需要 database 上下文。**必须在这里兜底** —— 客户端传什么都不信：实测从 Oracle
    // 连接切到 MySQL 后，残留的库名（Oracle Free 版的 PDB 就叫 FREE）跟着进了
    // user-action，写通道执行 `use free` 直接 1049 Unknown database 'free'。
    let database = if dialect.kind.key() == "sqlserver" {
        database.to_string()
    } else {
        String::new()
    };
    // TiDB 兼容 MySQL 的用户体系与 GRANT 语法（mysql.user / db / tables_priv 同形）
    let mysql_family = matches!(dialect.kind.key(), "mysql" | "mariadb" | "doris" | "tidb");
    let mssql_family = dialect.kind.key() == "sqlserver";
    let oracle_family = matches!(dialect.kind.key(), "oracle");
    let pg_family = matches!(dialect.kind.key(), "postgresql" | "kingbase");

    // 哪些权限入参已经真正实现：MySQL 系连接的三层（全局/库级/表级）—— 由
    // `user_action_sql` 拼出 `grant`/`revoke`；其余（模式级、对象级、服务器角色、
    // ClickHouse 的授权项）仍未实现。**只有确实支持的才放行**，不支持的一律明确拒绝，
    // 而不是默默建好用户却什么都没授。
    // Doris 没有例程（连 mysql.procs_priv 这张表都没有，dialect.rs 的 user_grants
    // 也排除了它）。它在 mysql_family 白名单里，若不放行这里会被照单全收、拼出
    // MySQL 的 `grant execute on procedure …` —— 在 Doris 上执行必炸。明确拒绝。
    if matches!(dialect.kind.key(), "doris" | "tidb")
        && options
            .get("routinePrivileges")
            .and_then(Value::as_array)
            .map(|a| !a.is_empty())
            .unwrap_or(false)
    {
        let why = if dialect.kind.key() == "tidb" {
            "TiDB 的例程级授权（grant execute on procedure）未验证过，不支持"
        } else {
            "Doris 没有存储过程/函数，不支持"
        };
        return Err(XError::bad_request(format!(
            "{why}例程级授权（本次没有对数据库做任何改动）"
        )));
    }
    const UNSUPPORTED: [(&str, &str); 8] = [
        ("privileges", "全局权限"),
        ("dbPrivileges", "库级权限"),
        ("tablePrivileges", "表级权限"),
        ("schemaPrivileges", "模式级权限"),
        ("objectPrivileges", "对象级权限"),
        ("serverRoles", "服务器角色"),
        ("grants", "授权项"),
        ("revoke", "撤销授权"),
    ];
    if let Some(object) = options.as_object() {
        for (key, label) in UNSUPPORTED {
            let filled = object
                .get(key)
                .map(|value| match value {
                    Value::Array(items) => !items.is_empty(),
                    Value::String(text) => !text.trim().is_empty(),
                    Value::Null => false,
                    _ => true,
                })
                .unwrap_or(false);
            if !filled {
                continue;
            }
            // MySQL 系：全局 / 库级 / 表级三层。
            // Oracle：系统权限（`privileges`）/ 角色（`roles`，不在本表里）/ 对象权限
            // （`tablePrivileges`，它的 `database` 就是模式名）+ 撤销清单（`revoke`，
            // Oracle 没有"一把全撤"，只能逐条点名）。
            // `revoke` = 界面按"原来有、现在没有了"算好的**撤销清单**。
            // MySQL 也要收（授权改成只出差量之后，撤销不再靠 `revoke all`）。
            let handled = dialect.grant_input_scopes().contains(&key);
            if !handled {
                return Err(XError::bad_request(format!(
                    "「{label}」的自动分配尚未实现：请先只做「新建用户 / 改密 / 删除用户」，\
                     再到查询页用 GRANT 授权（本次没有对数据库做任何改动）"
                )));
            }
        }
    }
    // 权限名会被**拼进 SQL**，所以逐个过白名单。校验器按方言走 trait：
    // Oracle 两本白名单分开（系统/对象）、CH 独立名单、H2 只有表级。
    let global_valid: fn(&str) -> bool = dialect.grantable_privilege_check();
    let object_valid: fn(&str) -> bool = dialect.object_privilege_check();
    // SQL Server 的「Schema 级 / 对象级」改为**行级选库**专用解析（不再借 MySQL 的
    // 库级/表级形状）：每行自带「数据库」，授权语句前会 `use [库];` —— 用户管理入口
    // 没有库上下文（database 传空），不选库就会把授权打到连接默认目录（master）上。
    // 库名 / 模式名 / 对象名都会拼进 SQL：一律过 is_plain_ident 白名单。
    let privileges = crate::api::dialect::UserPrivileges {
        global: if mssql_family {
            // SQL Server 的"全局"= 服务器角色（角色名也会拼进 SQL，所以过名字白名单）
            checked_privileges(options.get("serverRoles"), crate::api::dialect::is_plain_ident)?
        } else {
            checked_privileges(options.get("privileges"), global_valid)?
        },
        databases: if mssql_family {
            // mssql 的 schema 级不再走 MySQL 的库级形状（见下方 schema_grants）
            Vec::new()
        } else if oracle_family {
            // Oracle 没有"库级授权"这一层（库就是实例）：给了就明确拒绝，
            // 不默默忽略 —— 界面会照实说明，不会让人以为授上了。
            if options
                .get("dbPrivileges")
                .and_then(Value::as_array)
                .map(|a| !a.is_empty())
                .unwrap_or(false)
            {
                return Err(XError::bad_request(
                    "Oracle 没有「数据库级」授权这一层：对象权限请写在「对象权限」里（模式.对象）",
                ));
            }
            Vec::new()
        } else if pg_family {
            // PG 的「库级」= 模式级（USAGE / CREATE），database 字段就是模式名
            privilege_db_rows(
                options.get("dbPrivileges"),
                crate::api::dialect::is_pg_schema_priv,
            )?
        } else {
            privilege_db_rows(options.get("dbPrivileges"), global_valid)?
        },
        // PG 的**库级**（CONNECT / CREATE / TEMPORARY ON DATABASE）：与上面被借用来
        // 装模式级的 `databases` 分开，界面也是两页 —— 库是集群里的库，模式是库里的一层。
        pg_databases: if pg_family {
            privilege_db_rows(
                options.get("pgDbPrivileges"),
                crate::api::dialect::is_pg_database_priv,
            )?
        } else {
            Vec::new()
        },
        tables: if mssql_family {
            // mssql 的对象级不再走 MySQL 的表级形状（见下方 object_grants）
            Vec::new()
        } else if pg_family {
            privilege_table_rows(
                options.get("tablePrivileges"),
                crate::api::dialect::is_pg_table_priv,
            )?
        } else {
            privilege_table_rows(options.get("tablePrivileges"), object_valid)?
        },
        schema_grants: if mssql_family {
            mssql_schema_grant_rows(
                options.get("schemaPrivileges"),
                crate::api::dialect::is_mssql_privilege,
            )?
        } else {
            Vec::new()
        },
        object_grants: if mssql_family {
            mssql_object_grant_rows(
                options.get("objectPrivileges"),
                crate::api::dialect::is_mssql_privilege,
            )?
        } else {
            Vec::new()
        },
        routines: if mysql_family {
            privilege_routine_rows(options.get("routinePrivileges"), global_valid)?
        } else if pg_family {
            privilege_routine_rows(
                options.get("routinePrivileges"),
                crate::api::dialect::is_pg_routine_priv,
            )?
        } else {
            Vec::new()
        },
    };

    // Oracle 的撤销清单：界面按"原来有、现在没有了"算好传进来。
    // 名字同样要过白名单（与授予同一个理由：会被拼进 SQL）。
    // ClickHouse 与 MySQL 的 scope 形状一致（global/db/table），同一套解析直接共用。
    // H2 的 scope 形状与 MySQL 表级一致（owner=schema、object=表），直接共用同一套解析
    // 撤销清单：界面按"原来有、现在没有了"算好传进来，名字逐项过白名单
    // （privilege/owner/object/kind/db 都会被拼进 `revoke ...`）。scope 的合法形状
    // 与白名单按方言走 trait（is_revocable）—— handler 不再列四份同构循环。
    let revokes: Vec<crate::api::dialect::GrantRevoke> = {
        let mut out: Vec<crate::api::dialect::GrantRevoke> = Vec::new();
        for item in options
            .get("revoke")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let scope = item
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_lowercase();
            let privilege = item
                .get("privilege")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if privilege.is_empty() {
                continue;
            }
            let owner = item
                .get("owner")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let object = item
                .get("object")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            // 例程级撤销：kind = PROCEDURE / FUNCTION（要拼进 `on procedure ...`，必须校验）
            let kind = item
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_ascii_uppercase();
            // 行级选库：schema/object 级撤销可带目标库（SQL Server 用，其它方言行里没有）
            let db = item
                .get("database")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if !dialect.is_revocable(&scope, &privilege, &owner, &object, &kind)
                || (!db.is_empty() && !crate::api::dialect::is_plain_ident(&db))
            {
                return Err(XError::bad_request(format!(
                    "「{privilege}」不在可撤销的白名单里（或缺少{}）：已拒绝执行",
                    dialect.revoke_missing_hint()
                )));
            }
            // 兼容 revokePayload 的 'server-role' 写法：统一成 SQL 构造器认的 'serverrole'
            let scope = if scope == "server-role" {
                "serverrole".to_string()
            } else {
                scope
            };
            out.push(crate::api::dialect::GrantRevoke {
                scope,
                privilege,
                owner,
                object,
                db,
                kind,
            });
        }
        out
    };
    let roles: Vec<String> = options
        .get("roles")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    // MySQL 系的角色是「账号形状」（`名字@host`），会拼进 `grant 'r'@'h' to …`。
    // 名字与 host 都只放行常规字符（`%` 与 `.` 是 host 的通配/域名写法）。
    if matches!(dialect.kind.key(), "mysql" | "mariadb") {
        for r in &roles {
            let ok = !r.trim().is_empty()
                && r.trim()
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '%' | '@' | '$'));
            if !ok {
                return Err(XError::bad_request(format!(
                    "角色引用「{r}」里有不认识的字符：已拒绝执行（角色会拼进授权语句）"
                )));
            }
        }
    }

    // 授权**只出差量**：界面把"原来有、现在没有了"的算好放在 `revoke` 里，后端逐条点名撤销，
    // 勾选的那些再逐条授予 —— 不再"先 revoke all privileges, grant option 再按表单全量重授"
    // （那种做法会把用户别处的授权也一并抹掉再重建，用户明确要求不要这样做）。
    // 只有在客户端**没交** `revoke` 这份清单时（老调用方），才退回"一把全撤"。
    let diff_mode = options.get("revoke").is_some();
    let reset_privileges = mysql_family && action == "alter" && !diff_mode;

    let sql = match dialect.user_action_sql(
        &action,
        &user,
        password,
        host,
        &roles,
        &privileges,
        &revokes,
        reset_privileges,
        &attrs,
    ) {
        Meta::Sql(sql) => sql,
        _ => {
            // SQL Server 的"用户"是**服务器登录名**，`create login` 不接受空口令。
            // 不单说这一句的话，用户只会看到「SQLSERVER 不支持「create」」—— 他明明只是忘了填密码。
            if dialect.kind.key() == "sqlserver" && action == "create" && password.is_empty() {
                return Err(XError::bad_request(
                    "SQL Server 新建用户就是新建登录名，必须设置密码（CREATE LOGIN 不接受空口令）",
                ));
            }
            return Err(XError::bad_request(format!(
                "{} 不支持「{action}」这个用户操作（也可能是缺参数，例如改密需要 password）",
                dialect.kind.key().to_ascii_uppercase()
            )));
        }
    };
    if preview_only {
        // 与「对象重命名」保持一致：先能只看语句、不动数据库
        return Ok(Json(json!({ "success": true, "sql": sql, "previewOnly": true })));
    }
    let statements: Vec<String> = dbmind_core::split_statements(dialect.kind.protocol(), &sql)
        .into_iter()
        .filter(|statement| !statement.trim().is_empty())
        .collect();
    let mut done = 0usize;
    for statement in &statements {
        // 建/删用户、授权都是写库：走跟随连接策略的入口
        if let Err(err) = run_write_sql_in(&state, &id, &database, statement.clone(), 1, true).await {
            // 文案与 alter 那条同一口径（见上面那段注释）：只留数据库原话。
            // 已建好的用户不会被自动回滚 —— 但**只在真有语句生效时**才补这一句，
            // 「第 1/1 条执行失败」这种零信息前缀不再出现（前端自己会加"保存失败"这类动作前缀）。
            let reason = grant_error_hint(err.message.strip_prefix("SQL 执行失败：").unwrap_or(&err.message));
            let reason = reason.as_str();
            let tip = if done == 0 {
                String::new()
            } else {
                format!("（前 {done} 条已生效）")
            };
            return Ok(Json(json!({
                "success": false,
                "message": format!("{reason}{tip}"),
            })));
        }
        done += 1;
    }
    Ok(Json(json!({
        "success": true,
        "message": format!("已执行 {done} 条语句"),
        "sql": sql,
    })))
}

/// 把「授权被数据库拒绝」（MySQL 的 1044 / 1045）翻译成能照着做的提示。
///
/// 真机反馈：给用户勾上 `mysql` 库的 ALL PRIVILEGES，弹
/// `Access denied for user 'root'@'%' to database 'mysql'`，用户以为是工具把 SQL 写错了。
/// 其实这是 MySQL 的规则：**GRANT 只能转授你自己已经有的权限**（还要带 GRANT OPTION），
/// 当前连接账号对这个库没有权限时就报 1044。`mysql` 这类系统库尤其常见 —— 多数环境
/// 只给 root@localhost 全权，远端 `root@'%'` 往往只拿到业务库。
///
/// 只改**文案**，不改行为：同一台库里给业务库授同样的权限是能成功的（实验室实测），
/// 所以不能一刀切禁止对系统库授权。
fn grant_error_hint(reason: &str) -> String {
    let lower = reason.to_ascii_lowercase();
    if !(lower.contains("access denied for user") && lower.contains(" to database ")) {
        return reason.to_string();
    }
    let account = reason
        .split("for user ")
        .nth(1)
        .and_then(|rest| rest.split(" to database ").next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("当前连接账号");
    let database = reason
        .split(" to database ")
        .nth(1)
        .map(|rest| rest.trim().trim_matches('\'').trim().to_string())
        .unwrap_or_default();
    let system_note = if matches!(
        database.as_str(),
        "mysql" | "sys" | "performance_schema" | "information_schema"
    ) {
        format!("；`{database}` 是系统库，很多环境只给 root@localhost 全权，远端账号只拿到业务库")
    } else {
        String::new()
    };
    let what = if database.is_empty() {
        "对目标库没有权限".to_string()
    } else {
        format!("对 `{database}` 库没有权限")
    };
    format!(
        "{reason} —— 授权只能转授「你自己已经有的权限」：连接账号 {account} {what}{system_note}。\
         请换用对该库有权限的账号连接后重试，或改为授权业务库"
    )
}

/// `user_grants` 的结果 -> `[{scope, privilege, owner, object}]`。
///
/// 只认 4 列形状（MySQL 的三张视图 / Oracle 的三本字典都长这样）；列名对不上
/// （PG / SQL Server 走的是 `grants_text` 那条路）就返回空数组 —— 界面对空数组
/// 就是"没有可编辑的授权明细"，而不会去猜。
/// 从结果行里按**列名**取值（忽略大小写，按候选顺序取第一个命中的）。
///
/// 为什么需要它：列名的大小写由方言决定（MySQL 小写、Oracle 大写），
/// 而 `serde_json::Map` 的键是**区分大小写**的 —— 写死一个大小写就有一半方言取不到值，
/// 表现还不是报错，而是"字段是空的"（比报错更难查）。
fn pick_field(row: &Map<String, Value>, names: &[&str]) -> String {
    for name in names {
        for (key, value) in row.iter() {
            if key.eq_ignore_ascii_case(name) {
                if let Some(text) = value.as_str() {
                    return text.to_string();
                }
            }
        }
    }
    String::new()
}

/// 把 `user_grants` 查回来的行**展开**成「一行一权限」：
///
/// 1. MySQL 系的例程行（`mysql.procs_priv`）：一行带回空格分隔的原始 `Proc_priv`
///    （`Execute Alter Routine Grant`）—— SQL 侧不按 LIKE 拆三段，因为那条 UNION
///    必须压在一个 TCP 段以内（见 `Dialect::user_grants` 里关于 VPN 满尺寸包被丢的
///    实测注释），拆分在这里做：Execute→EXECUTE、Alter Routine→ALTER ROUTINE、
///    Grant→GRANT OPTION。
/// 2. 带 `grant_option` 列的行（information_schema 三张视图，值为 YES/NO）：
///    YES 时**多补一行** `GRANT OPTION` —— 与旧版「GRANT OPTION 三段 UNION」等价。
///
/// 没有 `grant_option` 列的方言（Oracle / PG …）原样通过，不受影响。
fn expand_grant_rows(rows: Vec<Map<String, Value>>) -> Vec<Map<String, Value>> {
    let mut out: Vec<Map<String, Value>> = Vec::with_capacity(rows.len());
    // GRANT OPTION 去重：旧 SQL 用 `select distinct` 保证同一目标只出一条
    // （demo1 对 dify 的十几个权限都 grantable ⇒ 不去重就是十几条重复的
    //  `GRANT GRANT OPTION ON dify.*`，界面勾选会被灌进一堆同名项）
    let mut seen_option: std::collections::HashSet<String> = std::collections::HashSet::new();
    for row in rows {
        let grantable = pick_field(&row, &["grant_option"]).eq_ignore_ascii_case("yes");
        let scope = pick_field(&row, &["scope"]).to_ascii_lowercase();
        let privilege = pick_field(&row, &["privilege"]);
        let routine = scope == "routine";
        // GRANT OPTION：结构上仍属于同一行所指向的目标，补一行同目标、权限名为 GRANT OPTION 的记录
        if grantable {
            let key = format!(
                "{}|{}|{}",
                scope,
                pick_field(&row, &["db_name"]),
                pick_field(&row, &["table_name"])
            );
            if seen_option.insert(key) {
                let mut r = row.clone();
                r.insert("privilege".to_string(), json!("GRANT OPTION"));
                r.remove("grant_option");
                out.push(r);
            }
        }
        if routine {
            // Proc_priv 是 SET 列：MySQL 存成**逗号分隔**的 `Execute,Grant`
            // （GRANT OPTION 成员渲染成 'Grant'！），也可能是空格分隔的老格式。
            // 只按空白切会把 'Execute,Grant' 整个当成未知 token —— 回显时
            // GRANT OPTION 勾选丢失（实测）。逗号和空白都当分隔符。
            let mut tokens = privilege
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .peekable();
            while let Some(tok) = tokens.next() {
                let name = if tok.eq_ignore_ascii_case("execute") {
                    "EXECUTE".to_string()
                } else if tok.eq_ignore_ascii_case("alter") {
                    // `Alter Routine` 是两个词，吞掉后面的 Routine
                    if let Some(next) = tokens.peek() {
                        if next.eq_ignore_ascii_case("routine") {
                            tokens.next();
                        }
                    }
                    "ALTER ROUTINE".to_string()
                } else if tok.eq_ignore_ascii_case("grant") {
                    // 同理 `Grant Option`
                    if let Some(next) = tokens.peek() {
                        if next.eq_ignore_ascii_case("option") {
                            tokens.next();
                        }
                    }
                    "GRANT OPTION".to_string()
                } else {
                    tok.to_string()
                };
                let mut r = row.clone();
                r.insert("privilege".to_string(), json!(name));
                out.push(r);
            }
            continue;
        }
        out.push(row);
    }
    out
}

fn grant_rows_of(rows: &[Map<String, Value>]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for row in rows {
        let scope = pick_field(row, &["scope"]).to_ascii_lowercase();
        if scope.is_empty() {
            continue;
        }
        out.push(json!({
            "scope": scope,
            "privilege": pick_field(row, &["privilege"]),
            "owner": pick_field(row, &["owner", "db_name"]),
            "object": pick_field(row, &["object", "object_name", "table_name"]),
        }));
    }
    out
}

/// 把 `user_grants` 的一行（scope / privilege / db_name / table_name）拼成
/// `GRANT <权限> ON <目标> TO <账号>;` 文本 —— 与 MySQL `SHOW GRANTS` 同形，
/// 界面（`parseMySqlGrants`）按它分全局 / 库级 / 表级三层。
fn grant_line(
    row: &Map<String, Value>,
    dialect: &crate::api::dialect::Dialect,
    account_user: &str,
    account_host: &str,
) -> Option<String> {
    // 单列文本结果 = 每行**本身就是**一条完整的 GRANT 语句，原样带回即可
    // （没有 scope/privilege 列，也不需要重组）：
    //   · ClickHouse：`show grants for` 就是这个形状；
    //   · MariaDB 的**角色**：它的 information_schema 三张权限视图根本不列角色
    //     （实测 `user_privileges` 里查不到），只能读 `show grants for '<role>'`，
    //     同样是单列 —— 少了这条分支，角色权限会整段消失（界面显示成"没有权限"）。
    // 判据用「有没有 scope 列」而不是写死方言：结构化那条路（MySQL 角色 / MariaDB 账号）
    // 的行都带 scope，不会被误判成文本。
    // 单列文本结果 = 每行本身就是一条完整的 GRANT 语句，原样带回即可
    // （ClickHouse 的 `show grants for` / MariaDB 的角色都是这个形状 ——
    // 判据用「有没有 scope 列」，结构化路径的行都带 scope，不会误判）。
    if !row.keys().any(|k| k.eq_ignore_ascii_case("scope")) {
        return row.values().find_map(Value::as_str).map(str::to_string);
    }
    // 列名一律**按名字取、忽略大小写**：MySQL 回来的是小写，Oracle 是 `PRIVILEGE` /
    // `OWNER` / `TABLE_NAME`。写死一种大小写就有一半方言取不到值，而且不报错
    // （表现是"授权明细是空的"，比报错难查得多）。
    let scope = pick_field(row, &["scope"]).to_ascii_lowercase();
    let privilege = pick_field(row, &["privilege"]);
    let owner = pick_field(row, &["owner", "db_name"]);
    let object = pick_field(row, &["object", "object_name", "table_name"]);
    if privilege.is_empty() || account_user.is_empty() {
        return None;
    }
    // scope → GRANT 文本的形状按方言走 trait（MySQL 通用 / Oracle / PG 系 / H2 各自覆盖）。
    dialect.grant_line_text(&scope, &privilege, &owner, &object, account_user, account_host)
}

/// 权限名走白名单（它们会被拼进 SQL），大小写不敏感、统一大写、去重。
///
/// `ALL PRIVILEGES` 单独归一：它和具体权限并列（`GRANT ALL PRIVILEGES, SELECT ON ...`）
/// 是有歧义的写法，而语义上 `ALL` 已经覆盖后者 —— 只留 `ALL PRIVILEGES` 一条，
/// 生成出来的语句既短又不会有歧义。
/// 权限名会被**拼进授权语句**，所以不在白名单里的一律拒绝、绝不拼。
/// 白名单由调用方给：各家不同，**同一家不同层级也不同**（Oracle 的对象权限与
/// 系统权限就是两张表），写死一张表的下场是"Oracle 的系统权限全被拒"。
fn checked_privileges(value: Option<&Value>, valid: fn(&str) -> bool) -> XResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for item in value.and_then(Value::as_array).cloned().unwrap_or_default() {
        let name = item.as_str().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        if !valid(name) {
            return Err(XError::bad_request(format!(
                "不认识的权限名「{name}」：已拒绝执行 —— 权限名会拼进授权语句，\
                 不在白名单里的一律不拼"
            )));
        }
        let upper = name.to_ascii_uppercase();
        if !out.contains(&upper) {
            out.push(upper);
        }
    }
    if out.iter().any(|name| name == "ALL PRIVILEGES") {
        return Ok(vec!["ALL PRIVILEGES".to_string()]);
    }
    Ok(out)
}

/// 库级权限行：`[{database, privileges}]`。
fn privilege_db_rows(value: Option<&Value>, valid: fn(&str) -> bool) -> XResult<Vec<(String, Vec<String>)>> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for row in value.and_then(Value::as_array).cloned().unwrap_or_default() {
        let database = row
            .get("database")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if database.is_empty() {
            continue;
        }
        out.push((database, checked_privileges(row.get("privileges"), valid)?));
    }
    Ok(out)
}

/// 表级权限行：`[{database, table, privileges}]`。
fn privilege_table_rows(
    value: Option<&Value>,
    valid: fn(&str) -> bool,
) -> XResult<Vec<(String, String, Vec<String>)>> {
    let mut out: Vec<(String, String, Vec<String>)> = Vec::new();
    for row in value.and_then(Value::as_array).cloned().unwrap_or_default() {
        let database = row
            .get("database")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let table = row
            .get("table")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if database.is_empty() || table.is_empty() {
            continue;
        }
        out.push((database, table, checked_privileges(row.get("privileges"), valid)?));
    }
    Ok(out)
}

/// MySQL 例程级权限行：`[{database, routine, kind(PROCEDURE|FUNCTION), privileges}]`。
/// 对**单个**存储过程/函数授 EXECUTE —— 库名/例程名会被拼进 SQL（过 is_plain_ident），
/// 权限名过白名单后再收窄到例程层合法的子集（EXECUTE / GRANT OPTION，其余丢弃）。
fn privilege_routine_rows(
    value: Option<&Value>,
    valid: fn(&str) -> bool,
) -> XResult<Vec<(String, String, String, Vec<String>)>> {
    let mut out: Vec<(String, String, String, Vec<String>)> = Vec::new();
    for row in value.and_then(Value::as_array).cloned().unwrap_or_default() {
        let database = row
            .get("database")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let routine = row
            .get("routine")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let kind = row
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("PROCEDURE")
            .trim()
            .to_ascii_uppercase();
        if database.is_empty() || routine.is_empty() {
            continue;
        }
        if !matches!(kind.as_str(), "PROCEDURE" | "FUNCTION") {
            continue;
        }
        for (label, name) in [("库名", &database), ("例程名", &routine)] {
            if !crate::api::dialect::is_plain_ident(name) {
                return Err(XError::bad_request(format!(
                    "{label}「{name}」不合法：已拒绝执行 —— 名字会拼进授权语句，不在白名单里的一律不拼"
                )));
            }
        }
        let privs: Vec<String> = checked_privileges(row.get("privileges"), valid)?
            .into_iter()
            // 双重过滤：例程层合法的名字（EXECUTE / GRANT OPTION）**且**过方言白名单 ——
            // PG 的 is_pg_routine_priv 只认 EXECUTE，GRANT OPTION 在这里就会被挡掉
            // （PG 里它是 WITH GRANT OPTION 子句，拼成权限名是语法错误）。
            .filter(|p| valid(p) && matches!(p.as_str(), "EXECUTE" | "GRANT OPTION"))
            .collect();
        if privs.is_empty() {
            continue;
        }
        out.push((database, routine, kind, privs));
    }
    Ok(out)
}

/// SQL Server 的 Schema 级授权行：`[{database(库，可空), schema, privileges}]` —— **行级选库**。
/// 库非空时后端会在 GRANT 前加 `use [库];`；库名/模式名会拼进 SQL，一律过 is_plain_ident。
fn mssql_schema_grant_rows(
    value: Option<&Value>,
    valid: fn(&str) -> bool,
) -> XResult<Vec<(String, String, Vec<String>)>> {
    let mut out: Vec<(String, String, Vec<String>)> = Vec::new();
    for row in value.and_then(Value::as_array).cloned().unwrap_or_default() {
        let db = row
            .get("database")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let schema = row
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if schema.is_empty() {
            continue;
        }
        if !db.is_empty() && !crate::api::dialect::is_plain_ident(&db) {
            return Err(XError::bad_request(format!(
                "库名「{db}」不合法：已拒绝执行 —— 名字会拼进授权语句，不在白名单里的一律不拼"
            )));
        }
        if !crate::api::dialect::is_plain_ident(&schema) {
            return Err(XError::bad_request(format!(
                "模式名「{schema}」不合法：已拒绝执行 —— 名字会拼进授权语句，不在白名单里的一律不拼"
            )));
        }
        out.push((db, schema, checked_privileges(row.get("privileges"), valid)?));
    }
    Ok(out)
}

/// SQL Server 的对象级授权行：`[{database(库，可空), schema, object, privileges}]` —— 同上。
fn mssql_object_grant_rows(
    value: Option<&Value>,
    valid: fn(&str) -> bool,
) -> XResult<Vec<(String, String, String, Vec<String>)>> {
    let mut out: Vec<(String, String, String, Vec<String>)> = Vec::new();
    for row in value.and_then(Value::as_array).cloned().unwrap_or_default() {
        let db = row
            .get("database")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let schema = row
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let object = row
            .get("object")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if schema.is_empty() || object.is_empty() {
            continue;
        }
        for (label, name) in [("库名", &db), ("模式名", &schema), ("对象名", &object)] {
            if !name.is_empty() && !crate::api::dialect::is_plain_ident(name) {
                return Err(XError::bad_request(format!(
                    "{label}「{name}」不合法：已拒绝执行 —— 名字会拼进授权语句，不在白名单里的一律不拼"
                )));
            }
        }
        out.push((
            db,
            schema,
            object,
            checked_privileges(row.get("privileges"), valid)?,
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialect(key: &str) -> crate::api::dialect::Dialect {
        let kind = dbmind_core::ConnectionKind::from_key(key).expect("未知的连接类型");
        crate::api::dialect::Dialect::new(kind)
    }

    /// 授权被拒（MySQL 1044）要翻成人话：账号、库名、为什么、怎么办，四样都得有；
    /// 别的错误原样返回，别乱加解释（真机：用户看着 `Access denied ... to database 'mysql'`
    /// 以为是工具把 SQL 写错了）。
    #[test]
    fn 授权被拒的提示带出账号与库名() {
        let raw = "Access denied for user 'root'@'%' to database 'mysql'";
        let out = grant_error_hint(raw);
        assert!(out.starts_with(raw), "{out}");
        assert!(out.contains("'root'@'%'"), "{out}");
        assert!(out.contains("`mysql` 库没有权限"), "{out}");
        assert!(out.contains("系统库"), "{out}");
        assert!(out.contains("只能转授"), "{out}");
        // 无关错误原样透传
        assert_eq!(
            grant_error_hint("Table 'x' doesn't exist"),
            "Table 'x' doesn't exist"
        );
    }

    /// 测试用的白名单：涵盖 SELECT/INSERT/UPDATE/DELETE/ALL PRIVILEGES。
    fn 测试白名单(s: &str) -> bool {
        matches!(
            s.to_ascii_uppercase().as_str(),
            "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "ALL PRIVILEGES"
        )
    }

    /// 权限名会被**拼进授权语句**，所以只认白名单：伪造值必须被拒，
    /// 而不是「拼进去试试」——拼错了就是授多了或授错了。
    #[test]
    fn 权限名只认白名单() {
        assert_eq!(
            checked_privileges(Some(&json!(["select", "INSERT"])), 测试白名单).unwrap(),
            vec!["SELECT", "INSERT"]
        );
        assert!(checked_privileges(Some(&json!(["HACK; drop database dify"])), 测试白名单).is_err());
        assert!(checked_privileges(Some(&json!(["TOTALLY_FAKE"])), 测试白名单).is_err());
        assert!(checked_privileges(None, 测试白名单).unwrap().is_empty());
        // 表单的「全部权限」就是 ALL PRIVILEGES（合法写法）；与具体权限并列时归一成 ALL
        assert_eq!(
            checked_privileges(Some(&json!(["ALL PRIVILEGES", "SELECT"])), 测试白名单).unwrap(),
            vec!["ALL PRIVILEGES"]
        );
    }

    /// 库级/表级权限行：缺库名或表名的行直接跳过 —— 宁可少授，也不拼出半截 SQL。
    #[test]
    fn 权限行的解析与过滤() {
        let rows = privilege_table_rows(
            Some(&json!([
                { "database": "dify", "table": "boxoffice", "privileges": ["SELECT"] },
                { "database": "dify", "privileges": ["SELECT"] }
            ])),
            测试白名单,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "dify");
        assert_eq!(rows[0].1, "boxoffice");

        let dbs = privilege_db_rows(
            Some(&json!([
                { "database": "", "privileges": ["SELECT"] },
                { "database": "dify", "privileges": ["SELECT", "UPDATE"] }
            ])),
            测试白名单,
        )
        .unwrap();
        assert_eq!(dbs.len(), 1);
        assert_eq!(dbs[0].1, vec!["SELECT", "UPDATE"]);
    }
}
