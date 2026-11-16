//! DDL 与对象：建表语句回显、对象定义（object-info）、索引/例程/触发器/事件清单、
//! 表动作（truncate/rename/drop）与对象改名。
//!
//! 原则同全局：分不清「没有」和「没写」的一律按「没写」报错（`not_written`）。

use super::*;

pub async fn ddl(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let table = params
        .get("table")
        .ok_or_else(|| XError::bad_request("缺少 table 参数"))?;
    let database = params.get("database").unwrap_or_default();
    let (ddl_text, synthesized) = build_table_ddl(&state, &id, &database, &table).await?;
    Ok(Json(
        json!({ "ddl": ddl_text, "sql": ddl_text, "synthesized": synthesized }),
    ))
}

/// 取一张表的建表语句，返回 `(SQL, 是否按列元数据自拼)`。
///
/// `/ddl`、`object-info(type=table)` 与导出的 `ddl_of` **共用这一条路径**。原先三处各写一份、
/// 判据还不一样：导出能"自拼"，「查看 DDL」却直接 501 —— 同一份列元数据、同一个问题两套答案。
///
/// 三档优先级：
/// 1. 方言能给出**原始建表语句**（`show create table` / `sqlite_master` / H2 的 `script`）⇒ 用它，完整；
/// 2. 方言没有那条语句（DB2 / Derby）⇒ **按列元数据自拼**（列 + 主键 + 非空 + 默认值），
///    并把"少了什么"写进脚本注释（调用方无需再猜）；
/// 3. 连列都取不到 ⇒ 报错（这时确实什么都给不了）。
pub(crate) async fn build_table_ddl(
    state: &AppState,
    id: &str,
    database: &str,
    table: &str,
) -> XResult<(String, bool)> {
    let (_, dialect) = ctx_of(state, id).await?;
    // 模式（「库.模式」拆出来的那半）交给方言：Oracle 靠它做 owner、PG 靠它筛命名空间。
    // 不传的话 Oracle 会按会话默认模式找表，跨模式直接 ORA-31603。
    let schema = split_db_schema(database, &dialect).1;
    let sql = match dialect.ddl(table, schema.as_deref()) {
        Meta::Sql(sql) => sql,
        Meta::Absent => return Ok((String::new(), false)),
        Meta::Unwritten => {
            let ddl = synthesize_ddl(state, id, database, table, dialect).await?;
            // "这是拼出来的"直接写进脚本文本：用户复制出去时这句跟着走，
            // 不会以为拿到了完整建表语句（导出那边是走 notices，这里没有 notices 通道）
            let notice = format!(
                "-- 注意：{} 没有「一条语句拿到完整建表语句」的办法，以下按列元数据拼出\
                 （含列、主键、非空与默认值；可能少索引、外键与注释）\n",
                dialect.kind.key().to_ascii_uppercase()
            );
            return Ok((format!("{notice}{ddl}"), true));
        }
    };
    let result = match run_sql_in(state, id, database, sql, 100).await {
        Ok(result) => result,
        // Oracle 内置视图会以「未找到」的姿势失败，见 oracle_builtin_view_error
        Err(err) => {
            return Err(oracle_builtin_view_error(
                state,
                id,
                database,
                table,
                schema.as_deref(),
                dialect,
                err,
            )
            .await)
        }
    };
    let mut ddl_text = if dialect.kind.key() == "h2" {
        // H2 的 `script nodata` 返回**整库**语句（它是一条命令，不接受 WHERE，
        // `table <名>` 过滤实测也被忽略），所以在这里按表名筛出属于这张表的那几条
        // —— 建表、主键约束、索引都会提到 `"PUBLIC"."T_PROBE"` 这个限定名，
        // 一并拼起来正好是完整 DDL（别的方言还得靠 ddl_extras 补索引）。
        // 不能交给 `pick_ddl`：它只取第一行，而 H2 脚本第一行是头注释 `-- H2 2.3.232;`。
        let needle = format!("\"{}\"", table.to_ascii_uppercase());
        rows_of(&result)
            .iter()
            .filter_map(|row| row.values().next())
            .filter_map(Value::as_str)
            .filter(|text| text.to_ascii_uppercase().contains(&needle))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        pick_ddl(&result)
    };
    // SQL Server 的注释（扩展属性）/ 索引 / 外键**写不进 create table**，只能作为后续语句追加。
    // 不追加的话，用户复制出去的「建表语句」重建出来是一张没有注释、没有索引、没有外键的空壳 ——
    // 这正是「DDL 不全、注释没出来」的来源。每段都是一次查询返回的可执行脚本；
    // 某段取不到就跳过：主 DDL 已经拿到了，不该因为附加信息失败而整条接口失败。
    if !ddl_text.trim().is_empty() {
        for (label, extra_sql) in dialect.ddl_extras(table) {
            let Ok(extra_result) = run_sql_in(state, id, database, extra_sql, 1).await else {
                continue;
            };
            let extra = pick_ddl(&extra_result);
            if !extra.is_empty() {
                ddl_text.push_str(&format!("\n\n-- {label}\n{extra}\n"));
            }
        }
    }
    if ddl_text.trim().is_empty() {
        return Err(XError::internal(format!("没有取到 {table} 的建表语句")));
    }
    Ok((ddl_text, false))
}

/// Oracle 的 `dbms_metadata.get_ddl` 对 **Oracle 自己维护的对象**（`all_objects.oracle_maintained='Y'`，
/// 典型是 SYSTEM 模式里那批 `MVIEW_EVALUATIONS` / `SCHEDULER_JOB_ARGS` 一类内置视图）会报
/// 「ORA-31603 对象 … 在方案 … 中未找到」—— 对象明明就在那儿，只是 Oracle 没给它留定义
/// （真机实测这些视图的 `all_views.text` / `user_views.text` 同样是 NULL，即服务端根本没有这份 SQL）。
///
/// 原样抛出去，用户看到的是「在方案 SYSTEM 中未找到」，只会以为 DBmind 找错了对象或模式；
/// 这里换成一句能解释清楚的话，并说明**换任何工具都取不到**。
/// 其它错误（包括确实不存在的对象）原样返回，不动判定。
async fn oracle_builtin_view_error(
    state: &AppState,
    id: &str,
    database: &str,
    name: &str,
    schema: Option<&str>,
    dialect: Dialect,
    err: XError,
) -> XError {
    if dialect.kind.key() != "oracle" || !err.message.contains("ORA-31603") {
        return err;
    }
    let owner = schema.map(str::trim).filter(|s| !s.is_empty());
    let mut sql = format!(
        "select oracle_maintained as \"maintained\" from all_objects \
         where object_name = upper({}) and object_type = 'VIEW'",
        dialect.literal(name)
    );
    if let Some(owner) = owner {
        sql.push_str(&format!(" and owner = upper({})", dialect.literal(owner)));
    }
    // 判定查询本身失败就按原错误返回：解释不清时不要用一个新错误盖住真因
    let Ok(result) = run_sql_in(state, id, database, sql, 5).await else {
        return err;
    };
    let builtin = rows_of(&result)
        .iter()
        .any(|row| text_ci(row, "maintained").eq_ignore_ascii_case("Y"));
    if !builtin {
        return err;
    }
    let full = match owner {
        Some(owner) => format!("{owner}.{name}"),
        None => name.to_string(),
    };
    XError::new(
        StatusCode::NOT_IMPLEMENTED,
        format!(
            "取不到 {full} 的视图定义：它是 Oracle 内置视图（all_objects.oracle_maintained='Y'），\
             Oracle 没有保存它的 SQL 文本，dbms_metadata.get_ddl 也会报 ORA-31603 —— \
             换任何工具同样取不到，不是 DBmind 找错了对象。"
        ),
    )
}

/// 按列元数据自拼建表语句（DB2 / Derby 这类"没有一条语句拿到 DDL"的类型走这里）。
///
/// 与导出、同步用的是**同一个** `create_table_from_columns`：同一份列元数据，
/// 在导出里拼得出来、在「查看 DDL」里却报 501，是说不过去的。
async fn synthesize_ddl(
    state: &AppState,
    id: &str,
    database: &str,
    table: &str,
    dialect: Dialect,
) -> XResult<String> {
    let target = crate::api::scope::resolve(state, id, database).await?;
    let engine = state.engine();
    let name = table.to_string();
    let mut columns = blocking(move || engine.list_columns_fresh(&target, &name)).await?;
    if columns.is_empty() {
        return Err(XError::internal(format!(
            "表 {table} 没有列信息，无法生成建表语句（{}）",
            dialect.kind.key()
        )));
    }
    // **必须先补类型长度**：内核给的列类型只有名字（Derby 是 `VARCHAR`、PG 是 `varchar`），
    // 直接拿去拼 DDL 会得到 `CODE VARCHAR not null` —— 建出来是 1 个字符宽（实测 Derby）。
    // 这条补全与 `/columns` 接口用的是同一个函数，两边口径一致。
    enrich_column_types(state, id, database, table, &mut columns).await;
    // `None` = 同类型：这里是"查看 DDL / 导出"，列的来源就是本连接自己，不需要类型翻译
    Ok(dialect.create_table_from_columns(
        &crate::api::export::qualified(table, database, dialect),
        &columns,
        None,
    ))
}

/// 结果里的某个文本是不是一段 DDL（`create ...`，允许前面有空行/注释）。
///
/// 为什么不能图省事用 `contains("create")`：MySQL 的 `show create procedure` /
/// `show create function` 会**多带一列 `sql_mode`**，其值形如
/// `STRICT_TRANS_TABLES,NO_ZERO_IN_DATE,…,NO_AUTO_CREATE_USER,NO_ENGINE_SUBSTITUTION`
/// —— 里面正好有 `CREATE`，于是那一列也被当成定义写进导出脚本。
/// 实测后果：导出的存储过程后面多出一行 `STRICT_TRANS_TABLES,…;`，
/// 这样的文件**重新导入会直接报语法错**（导出能用、还原不能用是最坏的一种）。
/// 要求"第一个有内容的行以 create 开头"，正好挡住这类"恰好含 create 的配置串"。
pub(crate) fn looks_like_ddl(text: &str) -> bool {
    // **块注释必须按「跨行状态」处理**，不能只看行首 `/*`：真机上有个函数定义是
    //   /*\n\nSELECT * FROM [fn_save_rollover] (...)\n*/\ncreate function ...
    // 老实现跳过 `/*` 那一行，第二行的 `SELECT` 就让它判定「不是建对象语句」，
    // 于是整库同步把它当成「源库里没有取到该对象的定义」跳掉（而它明明有 4784 字符的定义）。
    // 行注释 `--` 是逐行的，照旧跳过即可。
    let mut in_block = false;
    for raw in text.lines() {
        let mut rest = raw.trim_start();
        loop {
            if in_block {
                match rest.find("*/") {
                    Some(i) => {
                        in_block = false;
                        rest = rest[i + 2..].trim_start();
                    }
                    None => {
                        rest = "";
                        break;
                    }
                }
            } else if rest.starts_with("--") {
                rest = "";
                break;
            } else if let Some(tail) = rest.strip_prefix("/*") {
                in_block = true;
                rest = tail.trim_start();
            } else {
                break;
            }
        }
        if rest.is_empty() {
            continue;
        }
        return rest.to_ascii_lowercase().starts_with("create");
    }
    false
}

/// 从结果里取出 DDL 文本（按**列位置**取，不能按 Map 的取值顺序取）。
///
/// 为什么必须按位置：`serde_json` 的 Map 默认**按 key 排序**，而 MySQL 的
/// `show create table` 两列是 `[Table, Create Table]` —— 排序后 `Create Table` 反而在前，
/// 于是老代码那句"取第二列"取到的是**表名**（实测：`sales` 的建表语句只显示了一个 `sales`）。
///
/// 选列顺序：
/// 1. 列名像 DDL 的（跨方言各有各的叫法：MySQL `Create Table` / ClickHouse `statement` /
///    SQLite·H2 `sql` / 本仓自拼的 PG·SQLServer·DM 统一 `ddl`）；
/// 2. 只有一列就用它（Oracle 的 `dbms_metadata.get_ddl` 出来的是长表达式当列名）；
/// 3. 都不匹配时取**值最长的那一列** —— DDL 一定比表名、字符集之类的长，这条兜底
///    比"取第几列"稳得多。
/// 从「`show create …` / `pg_get_*def`」这类结果里挑出**定义原文**那一列。
///
/// 备份的例程/触发器导出也用它（`backup::export_objects`）—— 定义怎么选列，
/// 两处必须同一条规则，否则「查看定义」正常、备份文件里却是表名。
pub(crate) fn pick_ddl(result: &QueryResult) -> String {
    let Some(row) = result.rows.first() else {
        return String::new();
    };
    let values: Vec<String> = row
        .iter()
        .map(|cell| match shape::cell_to_value(cell) {
            Value::String(text) => text,
            Value::Null => String::new(),
            other => other.to_string(),
        })
        .collect();
    if values.is_empty() {
        return String::new();
    }
    let names: Vec<String> = result.columns.iter().map(|column| column.name.clone()).collect();
    let index = ddl_column_index(&names, &values);
    values
        .get(index)
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}

/// `pick_ddl` 的选列规则（拆出来是为了能单独读懂"为什么是这一列"）。
fn ddl_column_index(names: &[String], values: &[String]) -> usize {
    const HINTS: [&str; 15] = [
        "createtable",
        "createview",
        // 例程/触发器/事件：`show create procedure` 的列名是 `Create Procedure`，
        // 落不到「最长值」兜底就靠不住 —— 空过程体比 sql_mode 还短（例如
        // `CREATE ... PROCEDURE p() BEGIN END`），那时兜底会挑中 sql_mode。
        "createprocedure",
        "createfunction",
        "createtrigger",
        "createevent",
        // 但 MySQL 的 `show create trigger` 偏偏不叫 `Create Trigger`，
        // 而叫 `SQL Original Statement`（实测：body 短时会被兜底挑成 sql_mode）
        "sqloriginalstatement",
        "createstatement",
        "ddl",
        "sql",
        "statement",
        "ddltext",
        "createtableifnotexists",
        // PG / SQL Server 的 `pg_get_*def`、`object_definition` 都是这个列名
        "definition",
        "create",
    ];
    // 归一化：去掉空格/下划线/横线并转小写（`Create Table` → `createtable`）
    let normalize = |name: &str| {
        name.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase()
    };
    for (index, name) in names.iter().enumerate() {
        if HINTS.contains(&normalize(name).as_str()) {
            return index;
        }
    }
    if values.len() <= 1 {
        return 0;
    }
    values
        .iter()
        .enumerate()
        .max_by_key(|(_, text)| text.len())
        .map(|(index, _)| index)
        .unwrap_or(0)
}

/// 对象定义（视图 / 存储过程 / 函数 / 触发器）——右键「编辑」用它取到 DDL 再转 ALTER。
pub async fn object_info(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params.get("database").unwrap_or_default();
    let name = params.get("name").unwrap_or_default();
    let object_type = params
        .get("type")
        .or_else(|| params.get("kind"))
        .unwrap_or_default()
        .to_lowercase();
    if name.is_empty() {
        return Err(XError::bad_request("缺少 name 参数"));
    }
    // 表节点走的也是这个接口：前端右键「编辑 / 查看定义」时按 `object.kind` 传，
    // 表就是 `table`。而 `object_source` 只覆盖视图 / 例程 / 触发器 / 事件 ——
    // 表会落进兜底分支，两种坏法：
    //   MySQL  → 501，界面直接弹「获取定义失败」
    //   SQL Server → 200 但 ddl 为空，编辑器里一片空白（比报错更难发现）
    // 表的定义本来就是「建表语句」，所以直接复用 `/ddl` 那条路径 ——
    // 顺带把 SQL Server 的注释 / 索引 / 外键追加也一起带过来。
    if matches!(object_type.as_str(), "table" | "tables") {
        let (ddl, synthesized) = build_table_ddl(&state, &id, &database, &name).await?;
        return Ok(Json(json!({
            "ddl": ddl,
            "sql": ddl,
            "definition": ddl,
            "type": "table",
            "name": name,
            "synthesized": synthesized,
        })));
    }
    let (_, dialect) = ctx_of(&state, &id).await?;
    // 同 build_table_ddl：模式要传给方言（Oracle 的 owner 过滤就靠它）
    let schema = split_db_schema(&database, &dialect).1;
    let sql = match dialect.object_source(&object_type, &name, schema.as_deref()) {
        Meta::Sql(sql) => sql,
        Meta::Absent => return Ok(Json(json!({ "ddl": "", "sql": "" }))),
        Meta::Unwritten => {
            return Err(XError::not_implemented(&format!(
                "「对象定义」（{} / {object_type}）",
                dialect.kind.key()
            )))
        }
    };
    // Oracle 内置视图（ORACLE_MAINTAINED='Y'）取定义必失败，换成可读解释，见 oracle_builtin_view_error
    let result = match run_sql_in(&state, &id, &database, sql, 10).await {
        Ok(result) => result,
        Err(err) => {
            return Err(oracle_builtin_view_error(
                &state,
                &id,
                &database,
                &name,
                schema.as_deref(),
                dialect,
                err,
            )
            .await)
        }
    };
    // 必须按**列名**取，不能按位置取：`serde_json` 的 Map 默认按键名排序，
    // `show create view` 的四列（View / Create View / character_set_client / ...）
    // 排序后 `Create View` 反而在前，"取第二列"取到的是**视图名** ——
    // 界面上表现为「编辑视图」页签里只有一行 `v1`。选列规则见 `pick_ddl`。
    let definition = pick_ddl(&result);
    Ok(Json(json!({
        "ddl": definition,
        "sql": definition,
        "definition": definition,
        "type": object_type,
        "name": name,
    })))
}

// ------------------------------------------------------------------ 对象清单

pub async fn indexes(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let (_, dialect) = ctx_of(&state, &id).await?;
    let meta = dialect.indexes();
    let value = meta_list(
        &state,
        &id,
        &params.get("database").unwrap_or_default(),
        meta,
        "索引清单",
    )
    .await?;
    // 统一字段：界面按 name / table / unique / columns 取值
    let normalized = match value {
        Value::Array(rows) => Value::Array(
            rows.into_iter()
                .map(|row| {
                    let mut out = row.as_object().cloned().unwrap_or_default();
                    let unique = matches!(
                        out.get("unique"),
                        Some(Value::Bool(true)) | Some(Value::Number(_))
                    ) || out.get("nonUnique").and_then(Value::as_i64) == Some(0)
                        || out.get("isUnique").and_then(Value::as_i64) == Some(1)
                        || out.get("isUnique") == Some(&Value::Bool(true));
                    out.insert("unique".to_string(), Value::Bool(unique));
                    // 列：MySQL 是 group_concat，其余从语句里抠
                    let columns = match out.get("columns") {
                        Some(Value::String(s)) if !s.is_empty() => s
                            .split(',')
                            .map(|c| Value::String(c.trim().to_string()))
                            .collect::<Vec<_>>(),
                        _ => {
                            let definition = text_ci(&out, "sql");
                            let definition = if definition.is_empty() {
                                text_ci(&out, "indexdef")
                            } else {
                                definition
                            };
                            match definition.split_once('(') {
                                Some((_, rest)) => rest
                                    .split(')')
                                    .next()
                                    .unwrap_or("")
                                    .split(',')
                                    .map(|c| {
                                        Value::String(c.trim().trim_matches(['"', '`', '[', ']']).to_string())
                                    })
                                    .filter(|v| v.as_str().map(|s| !s.is_empty()).unwrap_or(false))
                                    .collect(),
                                None => Vec::new(),
                            }
                        }
                    };
                    out.insert("columns".to_string(), Value::Array(columns));
                    out.remove("sql");
                    out.remove("indexdef");
                    out.insert("_state".to_string(), Value::String("loaded".to_string()));
                    Value::Object(out)
                })
                .collect(),
        ),
        other => other,
    };
    Ok(Json(normalized))
}

pub async fn procedures(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let (_, dialect) = ctx_of(&state, &id).await?;
    let database = params.get("database").unwrap_or_default();
    // 把「库.模式」里的模式拆出来交给方言：Oracle 的例程要靠它做 owner 过滤
    // （不拆的话查的是会话当前模式，树上点任何模式都是 0）。
    let schema = split_db_schema(&database, &dialect).1;
    meta_list(
        &state,
        &id,
        &database,
        dialect.procedures(schema.as_deref()),
        "存储过程/函数清单",
    )
    .await
    .map(Json)
}

pub async fn triggers(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let (_, dialect) = ctx_of(&state, &id).await?;
    let database = params.get("database").unwrap_or_default();
    let schema = split_db_schema(&database, &dialect).1; // 同 procedures：模式要传给方言
    meta_list(
        &state,
        &id,
        &database,
        dialect.triggers(schema.as_deref()),
        "触发器清单",
    )
    .await
    .map(Json)
}

pub async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let (_, dialect) = ctx_of(&state, &id).await?;
    meta_list(
        &state,
        &id,
        &params.get("database").unwrap_or_default(),
        dialect.events(),
        "事件清单",
    )
    .await
    .map(Json)
}

/// 「是不是超级用户」这一列各家回来的**形状**不一样，统一折成布尔。
///
/// 实测：PG 的 `rolsuper`（真布尔）经驱动 → JSON 变成了**数字** `"isSuper":1`，
/// SQL Server 的 `is_srvrolemember(...)` 本来就是 int（也是 1）。
/// 原来这里只认 `Value::Bool(true)`，于是 postgres / sa 既不显示「超级用户」标签，
/// 备注也是空的（真机回归发现）。反过来，字符串 `"0"` 在 JS 里是**真值**，
/// 前端 `v-if="row.isSuper"` 会把假的也显示出来 —— 所以必须归一成布尔再出门。
pub(crate) fn is_true_flag(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().map(|v| v != 0).unwrap_or(false),
        Value::String(s) => matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "t" | "y" | "yes"
        ),
        _ => false,
    }
}

pub async fn table_action(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let table = body
        .get("table")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let action = body
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if table.is_empty() || action.is_empty() {
        return Err(XError::bad_request("缺少 table 或 action"));
    }
    let new_name = body.get("newName").and_then(Value::as_str).map(str::to_string);
    let (_, dialect) = ctx_of(&state, &id).await?;
    let sql = match dialect.table_action(&table, &action, new_name.as_deref()) {
        Meta::Sql(sql) => sql,
        _ => {
            return Err(XError::bad_request(format!(
                "不支持的表操作：{action}（重命名需要提供 newName）"
            )))
        }
    };
    let database = body.get("database").and_then(Value::as_str).unwrap_or_default();
    // 这里的 `1` 只限制**结果集行数**：这几条语句（delete / truncate / drop / rename）
    // 都不返回结果集，所以给多少都无所谓。
    //
    // 但**不要**指望靠它给 DML 加限制：宿主曾经把 `maxRows` 也套在 DELETE 上
    // （Microsoft 驱动用会话级的 `SET ROWCOUNT` 实现 `setMaxRows`），于是「清空表」
    // 只删掉 maxRows+1 行、接口照样回 success —— 静默少删。已在宿主侧修正：
    // 只有判定会返回结果集的语句才设上限（见 agents/dbmind-agent-jdbc 的 mayReturnRows）。
    // 清空 / 截断 / 删除 / 重命名表都是写库：走跟随连接策略的入口
    Ok(Json(
        match run_write_sql_in(&state, &id, database, sql, 1, true).await {
            Ok(result) => shape::query_result_json(&result),
            Err(err) => shape::query_failure_json(&err.message, 0),
        },
    ))
}

/// 对象重命名（视图 / 索引 / 表 …）。`previewOnly=true` 时只回要执行的语句，不执行。
pub async fn rename_object(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let new_name = body
        .get("newName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let object_type = body
        .get("type")
        .or_else(|| body.get("kind"))
        .or_else(|| body.get("objectKind"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    let preview_only = body.get("previewOnly").and_then(Value::as_bool).unwrap_or(false);
    if name.is_empty() || new_name.is_empty() {
        return Err(XError::bad_request("重命名需要原名与新名"));
    }
    let (_, dialect) = ctx_of(&state, &id).await?;
    // 模式：请求里的「库.模式」拆出来的那半（没有就是 None）
    let schema = body
        .get("database")
        .and_then(Value::as_str)
        .map(|db| split_db_schema(db, &dialect).1)
        .unwrap_or(None);
    // Oracle 的 `RENAME` **旧名可以带模式，而且必须带** —— DBMind 的会话刻意不按树上
    // 选中的模式切换（与取定义那几处同一个原因），不带就只能改会话自己模式里的对象。
    // 其余方言的改名语法把模式交给会话处理（MySQL/ClickHouse 的会话库是按请求切的）。
    let from = match (dialect.kind.key(), schema.as_deref()) {
        ("oracle", Some(owner)) if !owner.trim().is_empty() => {
            format!("{}.{}", dialect.quote(owner), dialect.quote(&name))
        }
        _ => dialect.quote(&name),
    };
    let to = dialect.quote(&new_name);
    let sql = match object_type.as_str() {
        "table" => match dialect.table_action(&name, "rename", Some(&new_name)) {
            Meta::Sql(sql) => sql,
            _ => return Err(XError::not_implemented("重命名表")),
        },
        "view" => match dialect.kind.key() {
            "sqlite" => return Err(XError::bad_request("SQLite 不支持重命名视图（可以删掉重建）")),
            // MySQL 系：视图与表一样走 `RENAME TABLE`。
            // ClickHouse：视图在它这里**就是一个表**（引擎不同而已），同样只认 `RENAME TABLE` ——
            // `ALTER VIEW … RENAME TO` 它没有（与表改名同一个坑，真机对着两种形式验过）。
            "mysql" | "mariadb" | "doris" | "tidb" | "clickhouse" => {
                format!("rename table {from} to {to}")
            }
            // Oracle：**没有** `ALTER VIEW … RENAME TO` —— 真机报 `ORA-00922: 选项缺失或无效`；
            // 入口是 `RENAME old TO new`，但它的旧名**不能带模式**（带上报 `ORA-01765
            // 不允许指定表的所有者名称`），也就是**只能改自己模式里的对象**。
            // 两条都是造临时视图、走这个接口实测出来的。
            // 所以：会话就是该模式 → 发 `RENAME`；不是 → 如实说清楚（与其发一句注定
            // 失败的 SQL，不如告诉用户该怎么做）。
            "oracle" => {
                let owner = schema.clone().unwrap_or_default();
                let me = run_sql_in(&state, &id, "", "select user from dual".to_string(), 1)
                    .await
                    .map(|result| pick_ddl(&result))
                    .unwrap_or_default();
                let me = me.trim().to_string();
                if !owner.trim().is_empty() && !me.is_empty() && !owner.eq_ignore_ascii_case(&me) {
                    return Err(XError::bad_request(format!(
                        "Oracle 不能重命名别的模式（{owner}）里的视图：它只有 `RENAME 旧名 TO 新名`，\
                         而旧名不允许带模式。请在查询页用 `create or replace view` 重建，\
                         或用 {owner} 账号执行"
                    )));
                }
                format!("rename {} to {to}", dialect.quote(&name))
            }
            // SQL Server：与表改名同一个入口 `sp_rename`（它同样没有 ALTER VIEW … RENAME TO）
            "sqlserver" => format!(
                "exec sp_rename {}, {}",
                dialect.literal(&name),
                dialect.literal(&new_name)
            ),
            // DM / H2 / Derby：没真机验证过改名语法，保持原样（与别处"没验证就不动"同一个原则）
            _ => format!("alter view {from} rename to {to}"),
        },
        "index" => match dialect.kind.key() {
            "mysql" | "mariadb" | "doris" | "tidb" => {
                return Err(XError::bad_request(
                    "MySQL 重命名索引要写在 ALTER TABLE 里，请在查询页执行",
                ))
            }
            _ => format!("alter index {from} rename to {to}"),
        },
        // 存储过程 / 函数：只有 PG 与 SQL Server 真有「重命名」语法
        "procedure" | "function" => match dialect.kind.key() {
            // PG：签名可省略（同名函数唯一时），有歧义会由数据库明确报错
            "postgresql" | "kingbase" => format!("alter {object_type} {from} rename to {to}"),
            // SQL Server：`sp_rename` 是唯一入口。它**不会更新依赖对象**（视图/存储过程里
            // 引用的旧名字会失效），这句提醒由界面/SQL 预览负责展示，这里只保证语句本身正确
            "sqlserver" => format!(
                "exec sp_rename {}, {}",
                dialect.literal(&name),
                dialect.literal(&new_name)
            ),
            "mysql" | "mariadb" | "doris" | "tidb" => {
                return Err(XError::bad_request(
                    "MySQL 没有重命名存储过程/函数的语法：请用 SHOW CREATE PROCEDURE 取到原文，\
                     DROP 之后用新名字重建",
                ))
            }
            other => {
                return Err(XError::bad_request(format!(
                    "{} 不支持重命名存储过程/函数：请在查询页 DROP 后用新名字重建",
                    other.to_ascii_uppercase()
                )))
            }
        },
        // 触发器：SQL Server 可改（同样有依赖风险）；PG 的 `ALTER TRIGGER` 必须带表名，
        // 而我们这里只有触发器名 —— 与其猜一张表，不如把限制说清楚
        "trigger" => match dialect.kind.key() {
            "sqlserver" => format!(
                "exec sp_rename {}, {}",
                dialect.literal(&name),
                dialect.literal(&new_name)
            ),
            "postgresql" | "kingbase" => {
                return Err(XError::bad_request(
                    "PostgreSQL 重命名触发器需要表名（alter trigger 名 on 表 rename to 新名）——\
                     树节点只带了触发器名，请到查询页执行",
                ))
            }
            other => {
                return Err(XError::bad_request(format!(
                    "{} 不支持重命名触发器：请在查询页 DROP TRIGGER 后用新名字重建",
                    other.to_ascii_uppercase()
                )))
            }
        },
        // 事件调度器：各家都没有「改名字」的语法
        "event" => {
            return Err(XError::bad_request(
                "事件没有重命名语法：请在查询页 DROP EVENT 后用新名字重建（SHOW CREATE EVENT 可取原文）",
            ))
        }
        other => return Err(XError::bad_request(format!("不支持重命名「{other}」这种对象"))),
    };
    if preview_only {
        return Ok(Json(json!({ "sql": sql, "native": true })));
    }
    let database = body.get("database").and_then(Value::as_str).unwrap_or_default();
    match run_write_sql_in(&state, &id, database, sql.clone(), 1, true).await {
        Ok(_) => Ok(Json(
            json!({ "success": true, "message": format!("已重命名为「{new_name}」"), "sql": sql }),
        )),
        Err(err) => Ok(Json(
            json!({ "success": false, "message": err.message, "sql": sql }),
        )),
    }
}

// ------------------------------------------------------------------ 用户管理

#[cfg(test)]
mod tests {
    use super::*;

    fn dialect(key: &str) -> crate::api::dialect::Dialect {
        let kind = dbmind_core::ConnectionKind::from_key(key).expect("未知的连接类型");
        crate::api::dialect::Dialect::new(kind)
    }
    fn pick(names: &[&str], values: &[&str]) -> usize {
        let names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
        let values: Vec<String> = values.iter().map(|value| value.to_string()).collect();
        ddl_column_index(&names, &values)
    }

    /// MySQL `show create view` 的真实列名（实测，含 `serde_json` 排序后的样子）。
    ///
    /// 这条守着一次踩过两次的 bug：Map 按键名排序 ⇒「取第几列」毫无意义。
    /// 表现是「编辑视图 v1」页签里只有一行 `v1`（取到了视图名列），
    /// 表的建表语句同理只显示一个 `sales`。两次都让人以为是功能没做。
    #[test]
    fn 视图取到的是_create_view_而不是视图名() {
        let names = [
            "character_set_client",
            "collation_connection",
            "Create View",
            "View",
        ];
        let values = [
            "utf8mb4",
            "utf8mb4_general_ci",
            "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v1` AS select 1",
            "v1",
        ];
        assert_eq!(names[pick(&names, &values)], "Create View");
    }

    /// MySQL `show create trigger` 的 DDL 列**不叫** `Create Trigger`，叫
    /// `SQL Original Statement`；而且 body 常常很短（本例 30 来字符），
    /// 长不过 sql_mode —— 只靠「最长值」兜底会挑出 sql_mode 贴进编辑器。
    #[test]
    fn 触发器取到的是_sql_original_statement_而不是_sql_mode() {
        let names = [
            "Database",
            "SQL Original Statement",
            "Trigger",
            "character_set_client",
            "collation_connection",
            "sql_mode",
        ];
        let values = [
            "上游_lock_probe",
            "CREATE DEFINER=`root`@`%` TRIGGER tr_lock BEFORE INSERT ON t_lock FOR EACH ROW SET NEW.a = IFNULL(NEW.a, 0) + 1",
            "tr_lock",
            "utf8mb4",
            "utf8mb4_unicode_ci",
            "STRICT_TRANS_TABLES,NO_ZERO_IN_DATE,NO_ZERO_DATE,ERROR_FOR_DIVISION_BY_ZERO,NO_AUTO_CREATE_USER,NO_ENGINE_SUBSTITUTION",
        ];
        assert_eq!(names[pick(&names, &values)], "SQL Original Statement");
    }

    /// 例程（`Create Procedure` / `Create Function`）与单列情形（SQLite 的 `sql`）。
    #[test]
    fn 例程与单列情形() {
        let names = ["Create Procedure", "Database", "Procedure", "sql_mode"];
        let values = [
            "CREATE PROCEDURE `p`() BEGIN END",
            "demo",
            "p",
            "ONLY_FULL_GROUP_BY",
        ];
        assert_eq!(names[pick(&names, &values)], "Create Procedure");
        assert_eq!(pick(&["sql"], &["create table t(a int)"]), 0);
    }

    /// 列名都不认识时，退回「值最长的那一列」——DDL 一定比表名、字符集长。
    #[test]
    fn 不认识的列名退回最长值() {
        let names = ["whatever", "table_name"];
        let values = ["CREATE TABLE `t` (a int, b varchar(10))", "t"];
        assert_eq!(names[pick(&names, &values)], "whatever");
    }
}
