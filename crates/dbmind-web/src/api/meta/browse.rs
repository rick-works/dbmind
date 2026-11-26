//! 元数据浏览：库/模式/表/列/注释/统计/对象搜索。
//!
//! 界面语义（分页、关键字、排序）在这里翻译成方言 SQL；
//! 方言差异全部走 `DialectOps` trait 的委托调用，这里不再有按 key 的分叉。

use super::*;

use crate::api::dialect_ops::editor;

async fn features_json(state: &AppState, id: &str) -> XResult<Value> {
    let (record, dialect) = ctx_of(state, id).await?;
    let kind = record.kind();
    // 能力探测**只认「有 SQL」**，不能只判「没落 Unwritten」：
    // `Meta::Absent` 表示该类型**根本没有这个概念**（SQL Server 没有事件、SQLite 没有存储过程、
    // ClickHouse / H2 没有触发器），而 `Unwritten` 表示概念在、只是还没写。
    // 用 `!Unwritten` 会把 Absent 也判成 true —— 实测 SQL Server 报 `events=true`、
    // SQLite 报 `procs=true`，树于是画出了这些分类：要么恒为 0，要么点进去直接报「未实现」。
    // 这两种情况在界面上应当是**同一个结论**：别显示。
    let has = |meta: Meta| matches!(meta, Meta::Sql(_));
    let index_ok = has(dialect.indexes());
    // 能力探测只关心「有没有 SQL」，不关心查哪个模式 —— 传 None
    let proc_ok = has(dialect.procedures(None));
    let trigger_ok = has(dialect.triggers(None));
    let event_ok = has(dialect.events());
    let user_ok = has(dialect.users());
    // `supportsDdl` 的语义是「能不能**取回**建表语句」，不等于「方言写了 show create」：
    // 没写的那两款（DB2 / Derby）`/ddl` 会走第 2 档「按列元数据自拼」（见 build_table_ddl），
    // 脚本可用、只是少索引/外键/注释。原先这里只认 `Sql` ⇒ 这两类的「查看 DDL」菜单
    // 被整个藏掉，用户以为功能缺失。`Absent`（根本没这个概念）仍然算不支持。
    let ddl_ok = !matches!(dialect.ddl("__probe__", None), Meta::Absent);
    // 「能列出用户」和「能看详情 / 能改」是两档能力：
    // Oracle / DM / ClickHouse 有用户列表，但 user_info / user_action_sql 未实现 ——
    // 树里有用户节点、右键「查看 / 编辑 / 删除」却必然 501。
    let user_detail_ok = has(dialect.user_info("__probe__"));
    let user_manage_ok = has(dialect.user_action_sql(
        "create",
        "__probe__",
        // 口令给占位值而不是空串：Oracle 这类"必须带口令才建得了用户"的方言，
        // 空口令会被 create 分支判成 Unwritten —— 于是**明明实现了却报成没实现**
        // （真机现象：SQL 补好了，页面上的「新增用户」仍然置灰）。
        "__probe__",
        "%",
        &[],
        &crate::api::dialect::UserPrivileges::default(),
        // 撤销清单一律为空 —— 探测只关心"这个方言有没有生成 SQL 的能力"
        &[],
        false,
        // 属性（配额 / 锁定 / 表空间这些）同样与"有没有生成 SQL 的能力"无关
        &crate::api::dialect::UserAttrs::default(),
    ));
    // 各类对象「查看 / 编辑」靠 `object_source`（取定义）；没实现的那几类
    // （ClickHouse / DB2 / H2 / Derby）点下去必然 501 —— 界面据此不摆这两个菜单项。
    let object_ddl_kinds: Vec<&str> = ["view", "procedure", "function", "trigger", "event"]
        .into_iter()
        .filter(|object_kind| has(dialect.object_source(object_kind, "__probe__", None)))
        .collect();
    // 结构编辑相关的能力挂在同一份 payload 上（界面一次性取，避免多一次往返）。
    // 全部来自 `dialect_ops::editor`：按方言族归属，新增同族品牌不用改这里。
    let edit = editor::for_kind(kind);
    let collations: Value = if edit.collations_json.is_empty() {
        json!({})
    } else {
        // 静态数据写成 JSON 文本，解析失败也不该让整个 /features 挂掉（前端拿到空对象会
        // 把排序规则退化成输入框，比整页报错好）
        serde_json::from_str(edit.collations_json).unwrap_or_else(|_| json!({}))
    };
    Ok(json!({
        "type": kind.key().to_ascii_uppercase(),
        "columnTypes": editor::column_types(kind),
        // 索引类型清单：前端「索引 → 类型」那两处下拉读的就是它（`features.indexTypes`）。
        // 空清单 ⇒ 下拉无项、`v-if="indexTypes.length"` 那一列不显示。
        "indexTypes": editor::index_types(kind),
        // 存储引擎：清单外的值前端照样能手输，所以宁可给一份有用的小名单也不给空数组
        // （空数组会让「存储引擎」整行退化成纯手输）。
        "engines": editor::storage_engines(kind),
        "supportsDatabases": true,
        "supportsSchemas": dialect.is_schema_aware(),
        "supportsIndexes": index_ok,
        "supportsProcedures": proc_ok,
        "supportsTriggers": trigger_ok,
        "supportsEvents": event_ok,
        "supportsUsers": user_ok,
        // 用户「详情 / 管理」是另一档能力（Oracle/DM/ClickHouse 只有列表）
        "supportsUserDetail": user_detail_ok,
        "supportsUserManage": user_manage_ok,
        // 哪些对象类型能取到定义（= 能「查看 / 编辑」）
        "objectDdlKinds": object_ddl_kinds,
        // 监控按方言实现（见 `api::monitor`）：已接入的给真数据，
        // 没接入的监控接口会返回 supported:false + 一句原因，界面照这句话显示
        "supportsMonitor": crate::api::monitor::supported(kind.key()),
        "supportsExport": true,
        "supportsImport": true,
        "supportsTableData": true,
        "supportsDdl": ddl_ok,
        // 「能不能**执行** DDL」与「能不能**取回**建表语句」是两件事：
        // DROP / TRUNCATE / 改名 这类操作所有 SQL 类都能跑（Derby、DB2 取不到 SHOW CREATE，
        // 但 DROP TABLE 完全可用），NoSQL 类则完全没有这个概念。界面按它门控**写操作**，
        // 不能拿 supportsDdl（= 取定义）当依据 —— 那会把 Derby / DB2 的删除表整个藏掉。
        "supportsDDLExec": kind.is_jdbc() || kind.key() == "sqlite",
        "supported": kind.implemented(),
        // 结构编辑器要的键（ddlStyle / quoteStyle / supportsColumnModify / ...）——
        // 展开在顶层，界面按扁平键读取
        "ddlStyle": edit.ddl_style,
        "quoteStyle": edit.quote_style,
        "supportsColumnModify": edit.supports_column_modify,
        "supportsComment": edit.supports_comment,
        "supportsTableOptions": edit.supports_table_options,
        "supportsAutoIncrement": edit.supports_auto_increment,
        "supportsIndexRebuild": edit.supports_index_rebuild,
        "doris": edit.doris,
        "charsets": edit.charsets,
        "collations": collations,
    }))
}

/// `POST /api/{m}/test` —— 测试连接。
///
/// 直接拿**表单里的那份配置**去连（内核 `test_config`），**不往连接库里写任何东西**。
///
/// 早前这里是「建一条临时连接 → 测 → 删掉」（那时内核只有按 id 自检的接口）。那条路有个
/// 不能接受的失败模式：删除一旦失败（进程被杀、请求中断、驱动占用），临时连接就**永久留在
/// 用户的连接列表里** —— 对象浏览器里凭空多出几行 `名字 (probe exec_…)`，用户从没建过它们，
/// 却得自己琢磨"这是什么"。改成不落库之后，这种残留从机制上不可能再出现。
pub async fn test(State(state): State<AppState>, Json(mut body): Json<Value>) -> XResult<Json<Value>> {
    // 测试连通性不该先要求起个名字，但连接校验要求 name 非空 —— 缺了就补个占位名。
    // 这个名字只用于让校验通过与错误文案可读，**不会被持久化**。
    let name_missing = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .is_empty();
    if name_missing {
        if let Some(object) = body.as_object_mut() {
            object.insert(
                "name".to_string(),
                json!(format!("probe-{}", dbmind_core::new_execution_id())),
            );
        }
    }
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // 已保存的连接：口令拿不到明文，留空时用服务端存着的那份补齐；
    // 只读标记同理 —— 它是**连接记录**上的属性（`ConnectionRecord.read_only`），不在配置里。
    //
    // **必须先取记录、再构造配置**：SSH 口令与私钥口令短语同样不回显，客户端交回来的
    // 是空串，「空 = 沿用已存的那份」要靠 `existing` 里的 `extra.ssh` 才能判定。
    // 顺序反了的话，编辑一条已配 SSH 的连接、不动口令直接点「测试连接」，
    // 会得到一句「SSH 口令是空的」——而用户明明没改过它。
    let existing = if id.is_empty() {
        None
    } else {
        require_record(&state, &id).await.ok()
    };
    let mut config = conn_api::config_from_body(
        &body,
        existing.as_ref().and_then(|record| record.config.extra.as_ref()),
    )?;
    let mut read_only = false;
    if let Some(record) = &existing {
        if config.password.is_none() {
            config.password = record.config.password.clone();
        }
        read_only = record.read_only;
    }
    // 驱动没装就先把驱动下下来：连接弹窗上写着「首次连接将自动从 Maven 中心下载驱动」，
    // 这句话必须是真的 —— 否则用户点「测试连接」只会看到「驱动未就绪」，像是坏了。
    if let Err(err) = crate::api::driver::ensure_installed(&state, config.kind).await {
        return Ok(Json(json!({ "success": false, "message": err.message })));
    }

    let engine = state.engine();
    // 这里**不走** `crate::api::blocking`：那一层会把 message / detail 拼成一句、还带上
    // 错误码前缀（`[DBMIND-CONN-0003] 连接失败：…`）。连接测试要的是「数据库原话」，
    // 说明由前端按关键字补一句 —— 两段分开给，界面才好排版。
    let tested = tokio::task::spawn_blocking(move || engine.test_config(&config, read_only)).await;

    Ok(Json(match tested {
        Ok(Ok(report)) => json!({
            "success": true,
            "latencyMs": report.latency_ms,
            "serverVersion": report.server_version.unwrap_or_default(),
            "runtimeMode": report.runtime_mode.as_str(),
            "message": report.message,
        }),
        Ok(Err(err)) => json!({
            "success": false,
            "code": err.code.as_str(),
            // 配置类错误（缺 {database} 之类的占位符）仍补一句「去填哪个框」；
            // 驱动原话里的 `ClientConnectionId: <guid>` 之类的内部标识一并剪掉
            "message": crate::api::error::with_hint(&err.message),
            "detail": err.detail.as_deref().map(crate::api::error::scrub_noise),
        }),
        Err(join) => json!({
            "success": false,
            "message": format!("连接测试线程异常：{join}"),
        }),
    }))
}

pub async fn features(State(state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    Ok(Json(features_json(&state, &id).await?))
}

/// 库清单：**服务器上当前账号有权限访问的全部库**（文件型就是它自己那一个）。
///
/// 连接里的「数据库名称」因此可以留空 —— 留空表示「连到服务器、不指定默认库」，
/// 库列表照样能列出来（见 `conn.rs` 的 `database_value`）。
/// catalog 清单（只有 catalog 层级的类型有：目前是 Doris）。
///
/// **其余类型一律返回空数组** —— 前端见空就不画 catalog 这一层，树的形状与现在完全一样。
/// 用「返回空」而不是「不提供接口」，是为了让前端只有一条判断：有没有 catalog。
pub async fn catalogs(State(state): State<AppState>, Path(id): Path<String>) -> XResult<Json<Value>> {
    let (_, dialect) = ctx_of(&state, &id).await?;
    if let Meta::Sql(sql) = dialect.catalogs() {
        if let Ok(result) = run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await {
            let names: Vec<String> = rows_of(&result)
                .iter()
                .map(catalog_column_text)
                .filter(|name| !name.trim().is_empty())
                .collect();
            if !names.is_empty() {
                return Ok(Json(json!(names)));
            }
        }
    }
    Ok(Json(json!([])))
}

pub async fn databases(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let (record, dialect) = ctx_of(&state, &id).await?;
    // catalog 层级（Doris）：库挂在 catalog 下面，`?catalog=xxx` 时列的是**那个 catalog 的库**。
    // 走 `<catalog>.information_schema.schemata`，不需要切会话（切会话是状态，容易串）。
    let params = Params::parse(raw.as_deref());
    let catalog = params.get("catalog").unwrap_or_default();
    if !catalog.trim().is_empty() {
        if let Meta::Sql(sql) = dialect.databases_in_catalog(catalog.trim()) {
            match run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await {
                Ok(result) => {
                    let names: Vec<String> = rows_of(&result)
                        .iter()
                        .map(first_column_text)
                        .filter(|name| !name.trim().is_empty())
                        // Doris 2.1 的库列表会把**内置目录名（internal）**混进来 —— 它不是库
                        //（访问报 Unknown database），树里会多出一个空壳「internal」节点，剔除
                        .filter(|name| !(dialect.catalog_level() && name.eq_ignore_ascii_case(catalog.trim())))
                        .collect();
                    if !names.is_empty() {
                        return Ok(Json(json!(names)));
                    }
                    tracing::warn!(connection = %id, catalog = %catalog, "该 catalog 下列不出任何库");
                }
                Err(err) => {
                    // **别把失败说成"这个 catalog 里没有库"**：树上一个子节点都不出、没有任何提示，
                    // 用户只会以为库被删了。与下面 databases() 同一口径：原样把原因交给界面。
                    tracing::warn!(connection = %id, catalog = %catalog, error = %err.message, "catalog 库清单查询失败");
                    return Err(err);
                }
            }
            // 查询成功但确实没有库（而不是退回"所有库"，那会串到别的 catalog）
            return Ok(Json(json!([])));
        }
    }
    if let Meta::Sql(sql) = dialect.databases() {
        match run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await {
            Ok(result) => {
                let names: Vec<String> = rows_of(&result)
                    .iter()
                    .map(first_column_text)
                    .filter(|name| !name.trim().is_empty())
                    // 同上：catalog 方言下剔除与目录同名的泄漏条目（默认目录就是 internal）
                    .filter(|name| !(dialect.catalog_level() && name.eq_ignore_ascii_case("internal")))
                    .collect();
                if !names.is_empty() {
                    return Ok(Json(json!(names)));
                }
                tracing::warn!(connection = %id, "库清单查询返回空，退回单库模式");
            }
            Err(err) => {
                // **连不上 ≠ 列不出来**：连接类错误必须原样抛给界面。
                //
                // 「退回单库模式」只适用于"连上了、但这条查询拿不到库清单"（权限不足、方言差异），
                // 那种情况下退回配置里的库名，界面至少还能用。
                // 但如果连都连不上还退回，界面就会拿到一个 200 + `["demo"]`，把失败当成成功
                // —— 用户改错主机/端口保存后，树照常展开、还能点开来看，误以为改动没生效
                //（真机上就是这么被问到的）。
                if err.is_conn_failure() {
                    return Err(err);
                }
                tracing::warn!(connection = %id, error = %err.message, "库清单查询失败，退回单库模式");
            }
        }
    }
    // 走到这里说明这个方言没有「列库 SQL」（DB2 / Derby / H2 …），下面返回的单库名只是**占位**。
    // 以前直接 200 返回它 = 把「连不上」包装成「连接成功」：网络断了树照样展开、还能点开来看，
    // 用户完全看不出问题（真机反馈：「不管什么原因，连接失败就不应该打开」）。
    // 所以先真连一次（最便宜的探针语句，走引擎会话池），连不上就把错误原样抛给界面。
    // 本地文件库（SQLite）没有「网络不通」这回事，跳过 —— 免得为探针凭空创建一个空文件。
    if !record.kind().local_file() {
        if let Err(err) = run_meta_sql(&state, &id, "", "values 1".to_string(), 1).await {
            if err.is_conn_failure() {
                return Err(err);
            }
            tracing::warn!(
                connection = %id,
                error = %err.message,
                "连通性探针失败（非连接类错误），仍退回单库模式"
            );
        }
    }
    let name = record
        .config
        .database
        .clone()
        .filter(|database| !database.is_empty())
        .unwrap_or_else(|| {
            if record.kind().local_file() {
                "main".to_string()
            } else {
                "(default)".to_string()
            }
        });
    Ok(Json(json!([name])))
}

pub async fn schemas(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params.get("database").unwrap_or_default();
    let (_, dialect) = ctx_of(&state, &id).await?;
    match dialect.schemas() {
        Meta::Absent => Ok(Json(json!([]))),
        Meta::Unwritten => Err(not_written(&state, &id, "schema 清单").await),
        Meta::Sql(sql) => {
            // schema 清单是**某个库内部**的概念（SQL Server 的 sys.schemas、PostgreSQL 的
            // pg_namespace 都是按库查的），所以要切到目标库去查
            let result = run_meta_sql(&state, &id, &database, sql, META_MAX_ROWS).await?;
            let names: Vec<String> = rows_of(&result)
                .iter()
                .map(first_column_text)
                .filter(|name| !name.trim().is_empty())
                .collect();
            Ok(Json(json!(names)))
        }
    }
}

/// 方言「属主 + 显式授权」清单 → 允许出现在界面上的表名集合（小写）。
///
/// `None` = 这个方言不需要过滤，或者清单取不到 / 为空。**空集合按「目录没覆盖到」处理**：
/// DB2 的组与角色授权不一定落在 `SYSCAT.TABAUTH` 里，H2 的属主表也不出现在授权目录中；
/// 若据此把整棵树清空，用户会以为"表都没了" —— 宁可少过滤，不可少给表。
async fn visible_table_set(
    state: &AppState,
    id: &str,
    database: &str,
    dialect: Dialect,
) -> Option<std::collections::HashSet<String>> {
    let Meta::Sql(sql) = dialect.visible_tables() else {
        return None;
    };
    let result = run_sql_in(state, id, database, sql, META_MAX_ROWS).await.ok()?;
    let set: std::collections::HashSet<String> = rows_of(&result)
        .iter()
        .map(|row| text_ci(row, "name").to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    if set.is_empty() {
        None
    } else {
        Some(set)
    }
}

/// 表 / 视图清单。
///
/// 两条路径：schema 层级（PostgreSQL / SQL Server / KingbaseES）按 schema 查；
/// 其余类型走内核的 `DatabaseMetaData`（各驱动都验过，比在这里重写一遍稳）。
pub async fn tables(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let (record, dialect) = ctx_of(&state, &id).await?;
    let database = params.get("database").unwrap_or_default();
    // Oracle / DM（`db_is_schema`）树上的「库」就是模式，而且**可以不带点**（达梦 `database=DEMO`）。
    // 只按 `split_db_schema` 拆会得到 None，接着掉进 JDBC 通用路径 —— 真机上达梦 DEMO 模式
    // 列出来的是 SYS 的三张系统表（HISTOGRAMS_TABLE / ##PLAN_TABLE / ##HISTOGRAMS_TABLE），
    // 建在 DEMO 里的 USERS 根本不见（用户看到的「表树是错乱的」就是这条）。
    let schema = params
        .get("schema")
        .filter(|s| !s.is_empty())
        .or_else(|| scope_schema_of(&database, &dialect, record.config.database.as_deref()));

    // catalog 层级（Doris）：树上「库」的名字是 `catalog.库`，对象查 <catalog>.information_schema，
    // 不需要也不能切会话 —— JDBC URL 表达不了 catalog，硬塞进 database 会把 URL 弄坏。
    if dialect.catalog_level() {
        if let Some((catalog, db)) = database.split_once('.') {
            if let Meta::Sql(sql) = dialect.tables_in_catalog(catalog, db) {
                // 库名传**空**：SQL 已经用 `<catalog>.information_schema` 全限定了，
                // 不需要切库；而把 `internal.ods` 传下去会去建一条 database='internal.ods'
                // 的影子连接 —— JDBC URL 变成 `.../internal.ods`，Doris 直接拒绝（实测 500）。
                let result = run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await?;
                let rows = rows_of(&result);
                // 降级：部分 JDBC catalog（实测 SAP HANA）的 information_schema.tables
                // 在 Doris 2.1.8 上透传不全 —— schemata 能列 schema、tables 却 0 行，
                // 树里就成"表 0"点不开。Doris 的 `show tables from catalog.db` 走另一套
                // 元数据路径，对它有效（实验室用 MySQL catalog 实测两条都返回表）。
                // 只在 information_schema 一条都没查到时兜底，正常 catalog 零回归。
                let (source_rows, via_show) = if rows.is_empty() {
                    match dialect.show_tables_in_catalog(catalog, db) {
                        Meta::Sql(show_sql) => {
                            // 降级失败也别把整个接口打挂：原路径已经拿到空清单，静默沿用。
                            match run_meta_sql(&state, &id, "", show_sql, META_MAX_ROWS).await {
                                Ok(showed) => (rows_of(&showed), true),
                                Err(_) => (rows, false),
                            }
                        }
                        _ => (rows, false),
                    }
                } else {
                    (rows, false)
                };
                let tables: Vec<Value> = source_rows
                    .iter()
                    .map(|row| {
                        // information_schema 路径有 name/type 两列；show tables 路径只有
                        // 一列 `Tables_in_<库>`，名字得从第一列取（first_column_text 会
                        // 先认 name、再退回首个值，正好两条路都兼容）。
                        let name = if via_show { first_column_text(row) } else { text_ci(row, "name") };
                        let kind = text_ci(row, "type");
                        json!({
                            "schema": db,
                            "name": name,
                            "type": if kind.to_ascii_uppercase().contains("VIEW") { "VIEW" } else { "TABLE" },
                            "comment": Value::Null,
                            // Doris 的 TABLE_ROWS 多是估算甚至 0，这里不猜：留空，
                            // 由前端的批量 COUNT(*) 回填（见 /table-count）
                            "rows": Value::Null,
                            "engine": Value::Null,
                            "charset": Value::Null,
                            "createTime": Value::Null,
                            "columns": [],
                        })
                    })
                    .collect();
                let mut payload = Value::Array(tables);
                // 表选项回显（引擎 / 排序规则 / **表注释**）：上面那条 `tables_in_catalog`
                // 只回名字与类型，注释得单独查。不补的后果与 schema 层级那条一样 ——
                // 注释改完保存**确实成功**，重开表却还是空的 ⇒ 看着就是"修改不生效"。
                if let Meta::Sql(sql) = dialect.table_options_in_catalog(catalog, db) {
                    // 库名同样传空：SQL 已经用 `<catalog>.information_schema` 全限定（同上）
                    if let Ok(result) = run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await {
                        shape::merge_table_options(&mut payload, &rows_of(&result));
                    }
                }
                return Ok(Json(payload));
            }
        }
    }

    if let (Some(schema), true) = (schema.as_deref(), dialect.is_schema_aware()) {
        let meta = dialect.tables_in_schema(schema);
        let result = match meta {
            Meta::Sql(sql) => run_meta_sql(&state, &id, &database, sql, META_MAX_ROWS).await?,
            _ => return Err(not_written(&state, &id, "按 schema 列出表").await),
        };
        let tables: Vec<Value> = rows_of(&result)
            .iter()
            .map(|row| {
                let kind = text_ci(row, "type");
                json!({
                    "schema": schema,
                    "name": text_ci(row, "name"),
                    "type": if kind.to_ascii_uppercase().contains("VIEW") { "VIEW" } else { "TABLE" },
                    "comment": Value::Null,
                    // 行数来自方言查询（SQL Server 是 sys.partitions 的**真实**行数，
                    // PG/Kingbase 是 reltuples 估算），取不到就是 null —— 界面上留空。
                    // 以前这里写死 0：schema 型数据库（SQL Server/PG/Kingbase）的树上
                    // 每张表都是 0，和「真的是空表」长得一模一样。
                    "rows": cell_int(row, "rows"),
                    "engine": Value::Null,
                    "charset": Value::Null,
                    "createTime": Value::Null,
                    "columns": [],
                })
            })
            .collect();
        let mut payload = Value::Array(tables);
        // 表选项回显（**表注释**）：`tables_in_schema` 那条 SQL 不带注释，而这一层正是
        // 最需要补的 —— SQL Server 的表注释存在 `sys.extended_properties` 里
        // （mssql-jdbc **不填 REMARKS**，走 JDBC 的 getTables 永远拿不到），
        // PG / Kingbase 的表注释在 `obj_description()` 里，同样不在列清单里。
        // 不补的实测后果：注释改完保存**确实成功**（库里值变了），重开表却还是空的
        // ⇒ 用户看到"修改表注释没用"。方言 SQL 取不到就保持原样，不影响列表本身。
        if let Meta::Sql(sql) = dialect.table_options(schema) {
            if let Ok(result) = run_meta_sql(&state, &id, &database, sql, META_MAX_ROWS).await {
                shape::merge_table_options(&mut payload, &rows_of(&result));
            }
        }
        return Ok(Json(payload));
    }

    // 表数据/元数据都按目标库查：切库靠影子连接（见 `scope`），这里只负责把库名传下去
    let target = crate::api::scope::resolve(&state, &id, &database).await?;
    let engine = state.engine();
    let mut tables = blocking(move || engine.list_tables_fresh(&target)).await?;
    // 内核走 `getTables` 的方言（H2 / DB2 / Derby）目录不看权限，这里再与「属主 + 显式授权」
    // 求交一次 —— 否则低权限账号会看到一张都读不了的表（实测 H2：未授权表照列，执行即报错）。
    if let Some(allowed) = visible_table_set(&state, &id, &database, dialect).await {
        tables.retain(|table| allowed.contains(&table.name.to_ascii_lowercase()));
    }
    // 行数：先给**估算值**（一次查询拿全库，不扫表），精确值由前端按需回填（`/table-count`）。
    // 内核的 `TableInfo.row_estimate` 各驱动都没填，所以这里按方言补一次；
    // 拿不到就保持 null —— 树上不显示数字，而不是显示一个 0 冒充「空表」。
    if let Meta::Sql(sql) = dialect.table_rows(&database) {
        if let Ok(result) = run_sql_in(&state, &id, &database, sql, META_MAX_ROWS).await {
            let mut estimates: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
            for row in rows_of(&result) {
                let name = text_ci(&row, "table_name");
                if name.is_empty() {
                    continue;
                }
                if let Some(count) = cell_int(&row, "rows") {
                    estimates.insert(name.to_ascii_lowercase(), count);
                }
            }
            for table in tables.iter_mut() {
                if let Some(count) = estimates.get(&table.name.to_ascii_lowercase()) {
                    table.row_estimate = Some(*count);
                }
            }
        }
    }
    // 表选项回显（引擎 / 排序规则 / 表注释）：内核的 `TableInfo` 只有名字与行数，
    // 而界面的「基本信息」要显示**这张表真实的**表选项 —— 不回填的话前端只能回退到
    // 「能力清单的第一项」，看着像回显、其实是默认值（表是 MyISAM 也显示 InnoDB，
    // 改别的项保存时会顺手把引擎改掉；表注释则永远是空的）。
    // 取不到就保持原样（null，界面回退到默认），不影响列表本身。
    let mut payload = shape::tables_json(&tables);
    // schema 层级的类型在没有 schema 时（库名里没有点）也会落到这条路上，那时方言 SQL
    // 要的是**模式名**（PG 的 `n.nspname`），给库名只会查不到 —— 取不到就当没有，
    // 不会比原来更差。
    let options_scope = schema.as_deref().unwrap_or(database.as_str());
    if let Meta::Sql(sql) = dialect.table_options(options_scope) {
        if let Ok(result) = run_sql_in(&state, &id, &database, sql, META_MAX_ROWS).await {
            shape::merge_table_options(&mut payload, &rows_of(&result));
        }
    }
    Ok(Json(payload))
}

/// 批量真实行数（`SELECT COUNT(*)`）：树里先用估算值渲染，随后异步回填精确值。
pub async fn table_count(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params.get("database").unwrap_or_default();
    let mut names = params.all("tables");
    if names.is_empty() {
        names = params.all("tables[]");
    }
    let (record, dialect) = ctx_of(&state, &id).await?;
    // Oracle / DM：`database` 就是模式（db_is_schema），COUNT 的表引用必须带模式前缀，
    // 否则一律按登录用户的默认模式解析 —— 真机：达梦上整批 COUNT 全部失败，界面行数恒空。
    let scope_schema = scope_schema_of(&database, &dialect, record.config.database.as_deref());
    let mut out = Map::new();
    // 去空 + 去重：同一张表只数一次（前端把同一张表重复带过来时不该数两遍）
    let mut queue: Vec<String> = Vec::new();
    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() || queue.iter().any(|kept| kept == trimmed) {
            continue;
        }
        queue.push(trimmed.to_string());
    }
    // 一批 COUNT 合成**一条** `union all` 语句。
    //
    // 为什么：这些 COUNT 全落在同一条后台会话上（见 [`run_sql_in_background`]），
    // 逐表发就是「一张表一次往返」。桌面端连远程库时 RTT 10~30ms，200 张表光往返
    // 就要 2~6 秒，而小表的 COUNT 本身只要 1~2ms —— 往返才是成本。
    // 实测（demo 库，热态）：40 张表 1.05s → 0.5s，95 张表 2.8s → 1.8s。
    //
    // 批大小**自适应**：一条语句里塞太多 COUNT，碰上大表会长时间占着会话，
    // 单条超时就把整批一起丢掉（失败会退回逐表，但白等一次超时）。
    const COUNT_BATCH_MAX: usize = 10;
    let mut batch = COUNT_BATCH_MAX;
    let mut offset = 0usize;
    while offset < queue.len() {
        let end = (offset + batch).min(queue.len());
        let chunk: Vec<String> = queue[offset..end].to_vec();
        offset = end;
        let started = std::time::Instant::now();
        let sql = chunk
            .iter()
            .map(|name| {
                format!(
                    "select {} as tbl, count(*) as cnt from {}",
                    dialect.literal(name),
                    dialect.table_ref(scope_schema.as_deref(), name)
                )
            })
            .collect::<Vec<_>>()
            .join(" union all ");
        let batched = run_sql_in_background(&state, &id, &database, sql, chunk.len()).await;
        let Ok(result) = batched else {
            // 批里任何一张表出错（权限 / 视图 / 表被删）都会拖垮整条语句：
            // 这一批退回到「逐表各发一条」的老路径，行为与改造前完全一致。
            for name in &chunk {
                let sql = dialect.count_sql_in(scope_schema.as_deref(), name);
                if let Ok(result) = run_sql_in_background(&state, &id, &database, sql, 1).await {
                    if let Some(count) = count_value_of(&result) {
                        out.insert(name.clone(), json!(count));
                    }
                }
            }
            batch = 1; // 有表非法 ⇒ 后续别再合并，省得反复整批失败
            continue;
        };
        let rows = rows_of(&result);
        for (index, name) in chunk.iter().enumerate() {
            // 先按 `tbl` 列对回去；驱动把别名改了大写之类对不上时退回**位置**
            //（union all 各分支的输出顺序与入参顺序一致）。
            let row = rows
                .iter()
                .find(|row| text_ci(row, "tbl").trim() == name.as_str())
                .or_else(|| rows.get(index));
            if let Some(count) = row.and_then(|row| count_value_of_row(row)) {
                out.insert(name.clone(), json!(count));
            }
        }
        // 单张表统计失败（权限/视图）不该让整次回填失败
        if started.elapsed().as_millis() > 2500 {
            batch = (batch / 2).max(1); // 慢库（大表多）自动缩批，避免撞超时
        } else if started.elapsed().as_millis() < 400 {
            batch = (batch * 2).min(COUNT_BATCH_MAX); // 快库逐步放大
        }
    }
    Ok(Json(Value::Object(out)))
}

/// 一行 count 结果 → 行数：按 `cnt` 列取，取不到退回第一个能读成整数的值。
///
/// ⚠️ 取不到时返回 `None`（上游整项跳过），**绝不写 0**：0 表示「这张表是空的」，
/// 而"取不到"是「我不知道」—— 界面上是"显示 0"与"不显示"的区别。
/// 实测：以前这里是 `as_i64().unwrap_or(0)`，而 ClickHouse 的 count 回来是浮点（Real），
/// 取不出来 → 那 5 张表（真值 4 / 216 / 95000 / 0 / 0）在树上一律显示 0。
fn count_value_of_row(row: &Map<String, Value>) -> Option<i64> {
    cell_int(row, "cnt").or_else(|| row.values().find_map(shape::value_to_i64))
}

/// 单条 COUNT 的结果集 → 行数（取第一行）。
fn count_value_of(result: &QueryResult) -> Option<i64> {
    rows_of(result).first().and_then(|row| count_value_of_row(row))
}

/// `GET /api/{m}/{id}/stats` —— 库级统计（表数 + 占用空间）。
///
/// 前端当前并不调它（上游里同样是「后端有、前端不用」），但形状保持一致：
/// 键名统一成 `tables` / `sizeText`，其余由方言自定义的键原样带出去。
/// 拿不到信息时给 `—` 而不是 0 —— 0 会被读成「这个库是空的」。
pub async fn stats(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params.get("database").unwrap_or_default();
    let (_, dialect) = ctx_of(&state, &id).await?;
    let sql = match dialect.database_stats() {
        Meta::Sql(sql) => sql,
        _ => return Err(not_written(&state, &id, "库级统计").await),
    };
    let result = run_meta_sql(&state, &id, &database, sql, 2).await?;
    let row = rows_of(&result).into_iter().next().unwrap_or_default();
    let mut out = Map::new();
    for (key, value) in row {
        // 键名统一小写：H2 之类会把结果列名大写返回（`TABLES`），
        // 于是下面的 `entry("tables")` 认不出它、又插入一个默认的 0 ——
        // 界面就看到「2 张表」被显示成 0（实测）
        out.insert(key.to_ascii_lowercase(), value);
    }
    // 但对外的键名是**驼峰** `sizeText`：上一步折成小写后，`entry("sizeText")` 永远命中
    // 不到 `sizetext`，于是同一个对象里同时出现 `sizetext: "10 MB"` 与 `sizeText: "—"`
    // —— 前端读后者，库大小恒显示「—」（金仓真机实测，PG 一样中招）。
    // 把规范键名搬回来，再补默认值。
    if let Some(value) = out.remove("sizetext") {
        out.insert("sizeText".to_string(), value);
    }
    out.entry("tables".to_string()).or_insert(json!(0));
    out.entry("sizeText".to_string()).or_insert(json!("—"));
    Ok(Json(Value::Object(out)))
}

pub async fn columns(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let table = params
        .get("table")
        .ok_or_else(|| XError::bad_request("缺少 table 参数"))?;
    let database = params.get("database").unwrap_or_default();
    let (record, dialect) = ctx_of(&state, &id).await?;
    // 「库.模式」里的**模式**那一层（PG / SQL Server / Kingbase 的树上会拼在库名后面）。
    // 不拆出来会怎样（用户反馈）：这些库取列走 JDBC `getColumns`，而它只认连接默认 schema
    // （PG=public、SQLServer=dbo）—— 别的 schema 下的表就"能列出表、字段却是空的"。
    // Oracle / DM 还要认**裸模式名**（达梦 `database=DEMO`），见 scope_schema_of 的说明。
    let schema = scope_schema_of(&database, &dialect, record.config.database.as_deref());
    // 外部 catalog（`catalog.库`，Doris 系）：scope::resolve 对它明确拒绝（JDBC 连接
    // 无法指定 catalog），而内核的列元数据（JDBC getColumns）也表达不了三段名 ——
    // 走 `desc \`c\`.\`d\`.\`t\``。高级搜索的条件列、表格列头都靠这个接口。
    if let Some((catalog, db)) = external_catalog_of(&dialect, &database) {
        let cols = external_table_columns(&state, &id, &dialect, &catalog, &db, &table).await?;
        let payload: Vec<Value> = cols
            .iter()
            .enumerate()
            .map(|(i, (name, type_name, nullable))| {
                json!({
                    "name": name,
                    "type": type_name,
                    "nullable": nullable,
                    "key": "",
                    "defaultValue": Value::Null,
                    "comment": Value::Null,
                    "extra": "",
                    "autoIncrement": false,
                    "primaryKey": false,
                    "sortKey": false,
                    "ordinal": i,
                })
            })
            .collect();
        return Ok(Json(Value::Array(payload)));
    }
    // 金仓不走 JDBC `getColumns()`：V8R3 服务端**没有** `pg_catalog` 兼容层，而
    // kingbase8 驱动（9.0.0 / 9.0.1 实测一致，驱动自报 V8R6）内部那条取列 SQL 引用了
    // `pg_catalog.pg_namespace` ⇒ 每一次取列都 500「关系 PG_CATALOG.PG_namespace 不存在」，
    // 表现就是「树上能列出表，点开表结构全红」。这里直接读系统目录，绕开驱动。
    // 目录查询自己失败（表名/模式拼错返回 0 行）时返回 None，继续走下面的 JDBC 路径，
    // 让上层报出真实错误，而不是把「没这一列」显示成空字段表。
    let mut columns = match kingbase_columns(&state, &id, &database, &table, schema.as_deref()).await {
        Some(columns) => columns,
        None => {
            let target = crate::api::scope::resolve(&state, &id, &database).await?;
            let engine = state.engine();
            // 表名要进闭包（移动），而下面补类型时还要再用一次，所以先复制一份
            let queried = table.clone();
            let schema_for_call = schema.clone();
            blocking(move || {
                engine.list_columns_fresh_in(&target, &queried, schema_for_call.as_deref())
            })
            .await?
        }
    };
    // 「补类型 / 注释 / 自增」这条也走「库.模式」：它的方言 SQL 用的是 `current_schema()` /
    // `::regclass` / `object_id()`（只看连接自己的 schema），所以 run_sql_in 会把 schema 交给
    // 代理"临时切过去再切回"（见 QueryRequest::schema 的说明）—— 这正是"指定 schema 时一次全对"的关键。
    let extras = enrich_column_types(&state, &id, &database, &table, &mut columns).await;
    let mut payload = shape::columns_json(&columns);
    // 注释 / 自增 / 额外属性：内核的列元数据里没有这几位，
    // 而 `shape::column_json` 之前把 `comment` 写死 null、`autoIncrement` 写死 false ——
    // 于是界面**永远认不出已有的自增列与列注释**。这里用方言元数据覆盖掉那几个占位值。
    if let Some(list) = payload.as_array_mut() {
        for item in list.iter_mut() {
            let Some(object) = item.as_object_mut() else {
                continue;
            };
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            let Some(extra) = extras.get(&name) else {
                continue;
            };
            if let Some(comment) = &extra.0 {
                if !comment.trim().is_empty() {
                    object.insert("comment".to_string(), json!(comment));
                }
            }
            if extra.1 {
                // 界面对 MySQL 走 `extra` 匹配、对其它方言走 `autoIncrement` 布尔，
                // 两个都填上，省得两边各判一次
                object.insert("autoIncrement".to_string(), json!(true));
                object.insert("extra".to_string(), json!("auto_increment"));
            }
        }
    }
    Ok(Json(payload))
}

/// 金仓（KingbaseES）的列清单：直接读 `sys_catalog`，不经 JDBC `getColumns()`。
///
/// 为什么必须绕开驱动：金仓 V8R3 服务端**没有** `pg_catalog` 兼容层，而 kingbase8 驱动
/// （9.0.0 / 9.0.1 实测表现一致，驱动自报版本 V8R6）内部那条取列 SQL 硬编码了
/// `pg_catalog.pg_namespace` ⇒ 每一次取列都 500「关系 PG_CATALOG.PG_namespace 不存在」。
/// 症状是「树上能列出表，点开表结构全红」，而 `data` / `ddl` / `users` 都正常 —— 很容易
/// 被误判成权限问题。这里直接查 `sys_class` / `sys_attribute` / `sys_attrdef` / `sys_constraint`。
///
/// 返回 `None` 的情形：不是金仓，或者目录查不到行（表名/模式不对）—— 交回调用方走原路径，
/// 让上层报出真实错误，而不是把「没有这张表」显示成一张空字段表。
pub(crate) async fn kingbase_columns(
    state: &AppState,
    id: &str,
    database: &str,
    table: &str,
    schema: Option<&str>,
) -> Option<Vec<ColumnDetail>> {
    let (_, dialect) = ctx_of(state, id).await.ok()?;
    if dialect.kind.key() != "kingbase" {
        return None;
    }
    let table_lit = dialect.literal(table);
    // 模式名比较必须**大小写不敏感**：金仓把未加引号的标识符折成大写，
    // 而目录里存的是小写（`public`）；`current_schema()` 在金仓上回来的是 `PUBLIC`
    // —— 精确比较会 0 行，然后退回 JDBC 又 500（真机踩出来的）。
    let ns_oid = match schema.map(str::trim).filter(|name| !name.is_empty()) {
        Some(schema) => format!(
            "(select oid from pg_namespace where lower(nspname) = lower({}))",
            dialect.literal(schema)
        ),
        // 不指定模式时**不能依赖 `current_schema()`**：金仓的 JDBC 会话上它可能回来 NULL
        // （search_path 与 ksql 不一致），子查询随即 0 行 —— 于是又退回会 500 的驱动路径。
        // 改成「按表名挑一个非系统模式的 oid」：表名在库里通常唯一，且与树上的用法一致。
        None => format!(
            "(select c2.relnamespace from pg_class c2 \
              join pg_namespace n2 on n2.oid = c2.relnamespace \
              where c2.relname = {table_lit} and c2.relkind in ('r','p','v') \
                and lower(n2.nspname) not in ('sys_catalog','information_schema') \
              order by lower(n2.nspname) limit 1)"
        ),
    };
    // 写 PG 版语句再过一遍词法改名（`pg_class` → `sys_class` …）。
    // `format_type` / `col_description` 金仓保留同名函数，改名表里刻意没有它们。
    let sql = format!(
        "select a.attname as name, \
         format_type(a.atttypid, a.atttypmod) as type_text, \
         case when a.attnotnull then 0 else 1 end as nullable, \
         sys_get_expr(d.adbin, d.adrelid) as default_value, \
         case when exists (select 1 from pg_constraint con \
                            where con.conrelid = c.oid and con.contype = 'p' \
                              and a.attnum = any(con.conkey)) then 1 else 0 end as is_pk \
         from pg_class c \
         join pg_attribute a on a.attrelid = c.oid and a.attnum > 0 and not a.attisdropped \
         left join pg_attrdef d on d.adrelid = c.oid and d.adnum = a.attnum \
         where c.relname = {table_lit} and c.relnamespace = {ns_oid} \
         order by a.attnum"
    );
    let sql = crate::api::dialect_ops::kb_catalog_sql(&sql);
    let result = run_sql_in(state, id, database, sql, 1000).await.ok()?;
    let rows = rows_of(&result);
    if rows.is_empty() {
        return None;
    }
    // 布尔位一律宽容解析：金仓回来的是 0/1，改名前的 PG 版本是 true/false
    let flag = |row: &Map<String, Value>, key: &str| -> bool {
        row.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case(key)
                && match v {
                    Value::Bool(b) => *b,
                    Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
                    Value::String(s) => matches!(s.trim(), "1" | "t" | "T" | "true" | "TRUE"),
                    _ => false,
                }
        })
    };
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let name = text_ci(row, "name");
        if name.is_empty() {
            continue;
        }
        let type_text = text_ci(row, "type_text");
        let default_value = text_ci(row, "default_value");
        out.push(ColumnDetail {
            name,
            type_name: if type_text.is_empty() { None } else { Some(type_text) },
            nullable: flag(row, "nullable"),
            primary_key: flag(row, "is_pk"),
            // 默认值只在「确实有」时给：空串会让界面显示一个空的默认值输入框
            default_value: if default_value.is_empty() {
                None
            } else {
                Some(default_value)
            },
            // 自增交给 `enrich_column_types`（它的 is_identity / nextval 判定已在金仓上验过）
            auto_increment: false,
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 结果集的**列注释**（`列名(小写) → 注释`）：SQL 编辑器对单表查询在表头第二行显示
/// 字段注释 —— 各方言的注释 SQL 由 `dialect.table_comments` 出（MySQL/Doris 走
/// information_schema、PG/Kingbase 走 pg_description、SQL Server 走 extended_properties、
/// Oracle/DM 走 all_col_comments、ClickHouse 走 system.columns……），这里只做转发；
/// 方言不支持列注释时返回空表（前端不渲染注释行）。
pub async fn column_comments(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let table = params
        .get("table")
        .ok_or_else(|| XError::bad_request("缺少 table 参数"))?;
    let database = params.get("database").unwrap_or_default();
    // 查询失败要报出来：以前静默给空表，界面把「读不到注释」显示成「这张表没写注释」，
    // 用户会去重新补一遍注释，其实是被权限/方言挡了。
    let map = table_comments_result(&state, &id, &database, &table).await?;
    Ok(Json(json!(map)))
}

/// 一张表的**列注释**：`列名(小写) → 注释`（取不到就是空表，不编造）。
///
/// 内核的 `ColumnDetail` 不带注释 —— 注释只在方言的元数据查询里取得到，所以按需取一次。
/// **数据同步的「目标表不存在时自动建表」用它**：把源表的字段注释带到目标表建表语句里
/// （以前这条链路一个注释都不带，目标表建出来注释全空，还得人工补一遍）。
pub async fn table_comments_map(
    state: &AppState,
    conn: &str,
    database: &str,
    table: &str,
) -> std::collections::HashMap<String, String> {
    // 同步建表这类「带带注释更好」的场景用宽松版：失败就当没有注释，不能因此让整次建表失败
    table_comments_result(state, conn, database, table)
        .await
        .unwrap_or_default()
}

/// 同 [`table_comments_map`]，但**区分「方言不支持注释」与「查询失败」**：
/// 前者返回空表（正常），后者把错误原样抛出，让 `/column-comments` 能告诉界面
/// 「注释读取失败」而不是显示成「这张表没写注释」。
pub async fn table_comments_result(
    state: &AppState,
    conn: &str,
    database: &str,
    table: &str,
) -> XResult<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    let record = require_record(state, conn).await?;
    let dialect = Dialect::new(record.kind());
    let Some(sql) = dialect.table_comments(table).sql() else {
        return Ok(out); // 方言没有注释概念：不是失败
    };
    let result = run_sql_in(state, conn, database, sql, META_MAX_ROWS).await?;
    for row in rows_of(&result) {
        let name = text_ci(&row, "column_name");
        let comment = text_ci(&row, "comment");
        if !name.is_empty() && !comment.is_empty() {
            out.insert(name.to_ascii_lowercase(), comment);
        }
    }
    Ok(out)
}

/// **全库**表注释：`表名(小写) → 注释`，一次查询拿完。
///
/// 用途：AI 回答「这个库是干嘛的」这类**库级问题**时，表注释是唯一既便宜又有效的依据 ——
/// 395 张表的列结构要几十万 token，而「表名 + 注释」几千 token 就能让模型说清业务。
/// 取不到就返回空表（上层退化成只用表名），不编造、不报错。
pub async fn table_comment_map(
    state: &AppState,
    conn: &str,
    database: &str,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let Ok(record) = require_record(state, conn).await else {
        return out;
    };
    let dialect = Dialect::new(record.kind());
    // 库名可能带模式（`db.schema`）：方言的表选项 SQL 要的是**模式名**
    // （PG 是 `n.nspname`、SQL Server 是 schema），给整串只会查不到。
    // Oracle / DM 的裸模式名（达梦 `DEMO`）同样要当模式用，见 scope_schema_of。
    let (db_only, _) = split_db_schema(database, &dialect);
    let schema = scope_schema_of(database, &dialect, record.config.database.as_deref());
    let scope = schema.as_deref().unwrap_or(db_only.as_str());
    let Meta::Sql(sql) = dialect.table_options(scope) else {
        return out;
    };
    let Ok(result) = run_sql_in(state, conn, database, sql, META_MAX_ROWS).await else {
        return out;
    };
    for row in rows_of(&result) {
        let name = text_ci(&row, "table_name");
        let comment = text_ci(&row, "comment");
        if !name.is_empty() && !comment.is_empty() {
            out.insert(name.to_ascii_lowercase(), comment);
        }
    }
    out
}

/// 一张表的**表注释**（取不到给 `None`）。同 `table_comments_map`，供同步建表复用。
pub async fn table_comment_of(state: &AppState, conn: &str, database: &str, table: &str) -> Option<String> {
    let record = require_record(state, conn).await.ok()?;
    let dialect = Dialect::new(record.kind());
    let Meta::Sql(sql) = dialect.table_options(database) else {
        return None;
    };
    let result = run_sql_in(state, conn, database, sql, META_MAX_ROWS).await.ok()?;
    for row in rows_of(&result) {
        if text_ci(&row, "table_name").eq_ignore_ascii_case(table) {
            let comment = text_ci(&row, "comment");
            if !comment.is_empty() {
                return Some(comment);
            }
        }
    }
    None
}

/// 用方言自己的元数据把列类型补成**带长度/精度的完整文本**（`nvarchar` → `nvarchar(50)`）。
///
/// 界面靠这段文本拆出「长度/精度」两个输入框，并且在生成 ALTER 时直接用它 ——
/// 少了参数，改这一行的任何属性都会产出 `ALTER COLUMN c nvarchar NULL` 这种
/// 「没长度」的语句（SQL Server / MySQL / Oracle 上会报错或把列变成 1 字符宽）。
///
/// **失败不改行为**：拿不到就保留内核给的类型名。为了一条增强元数据把「看列」这件事
/// 整个弄失败是本末倒置；拿不到时的风险由前端那道「类型需要长度但没填」的校验挡住。
pub(crate) async fn enrich_column_types(
    state: &AppState,
    id: &str,
    database: &str,
    table: &str,
    columns: &mut [ColumnDetail],
) -> std::collections::HashMap<String, (Option<String>, bool)> {
    // 列名(小写) → (注释, 是否自增)
    let mut extras: std::collections::HashMap<String, (Option<String>, bool)> =
        std::collections::HashMap::new();
    let Ok(record) = require_record(state, id).await else {
        return extras;
    };
    let dialect = Dialect::new(record.kind());
    let Some(sql) = dialect.column_types(table).sql() else {
        return extras;
    };
    let Ok(result) = run_sql_in(state, id, database, sql, 1000).await else {
        return extras;
    };
    let mut by_name: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for row in rows_of(&result) {
        // **先把结果列名折成小写**：有些库把结果列名大写返回（H2 就是 `COLUMN_NAME`），
        // 而下面按 `column_name` / `type_text` / `comment` 取 —— 大小写对不上时
        // `row.get` 静默返回 None，整张表的「类型带长度 + 列注释 + 自增」补全全部失效
        // （实测：H2 上 `/columns` 只给 `CHARACTER VARYING`，注释也丢）。
        // 不用逐键改 `text_ci`：这里把这一行的键统一折小写，后面所有取值都不用再想大小写。
        let row: Map<String, Value> = row
            .into_iter()
            .map(|(key, value)| (key.to_ascii_lowercase(), value))
            .collect();
        let name = row
            .get("column_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        if name.is_empty() {
            continue;
        }
        let text = row
            .get("type_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        // 同名取第一条：SQL 里已按「默认 schema 优先」排序
        if !text.is_empty() {
            by_name.entry(name.clone()).or_insert(text);
        }
        let comment = row.get("comment").and_then(Value::as_str).map(str::to_string);
        // `is_auto` 各家给的是 0/1（布尔在某些方言上会被当成数字），两种都认
        let is_auto = row
            .get("is_auto")
            .map(|value| match value {
                Value::Bool(flag) => *flag,
                Value::Number(number) => number.as_i64().unwrap_or(0) != 0,
                Value::String(text) => {
                    let t = text.trim().to_ascii_lowercase();
                    t == "1" || t == "true" || t == "yes"
                }
                _ => false,
            })
            .unwrap_or(false);
        if comment.as_deref().map(|c| !c.trim().is_empty()).unwrap_or(false) || is_auto {
            extras.insert(name, (comment, is_auto));
        }
    }
    for column in columns.iter_mut() {
        let key = column.name.to_lowercase();
        if let Some(text) = by_name.get(&key) {
            column.type_name = Some(text.clone());
        }
        // 自增位一并回填：跨类型同步建表要靠它还原 auto_increment / serial / identity
        if let Some((_, is_auto)) = extras.get(&key) {
            column.auto_increment = *is_auto;
        }
    }
    extras
}

/// 库内对象搜索：表名命中 + 列名命中。
///
/// 返回形状按界面取值定：`tables` 是**字符串数组**（表名），
/// `columns` 是 `{table, column}` 对象数组（见 `DbSearchDialog.vue`）。
/// 列名搜索要在每张表上取一次列，所以有表数上限 —— 几百张表的库不该把浏览器拖死。
pub async fn search_objects(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    const MAX_TABLES: usize = 60;
    const MAX_COLUMNS: usize = 200;
    let params = Params::parse(raw.as_deref());
    let keyword = params.get("keyword").unwrap_or_default().to_lowercase();
    if keyword.trim().is_empty() {
        return Ok(Json(json!({ "tables": [], "columns": [] })));
    }

    let database = params.get("database").unwrap_or_default();
    let (record, dialect) = ctx_of(&state, &id).await?;
    // Oracle / DM 的「库」层就是模式：不把它当模式用，就会落到下面「枚举全部模式」的分支，
    // 一次搜索 60 秒起（真机）而且结果跨模式。（裸模式名也算模式，见 scope_schema_of）
    let schema = scope_schema_of(&database, &dialect, record.config.database.as_deref());
    let target = crate::api::scope::resolve(&state, &id, &database).await?;

    // 表/视图清单：schema 层级的类型**必须**与树用同一个来源（`dialect.tables_in_schema`）。
    // 走内核 `getTables` 的话，PG 会把当前账号一张都读不了的表也返回 —— 结果是
    // 「树上被权限收掉的对象，全局搜索里还能搜到」，正是权限过滤的反面。
    // 拿不到方言 SQL / 方言执行失败时才回退内核（回退是少了过滤，不是少了表）。
    let mut table_names: Vec<String> = Vec::new();
    let mut from_dialect = false;
    if dialect.is_schema_aware() {
        // 给了模式就查那一个；没给（树上的「库」节点，PG/SQL Server/Oracle 都可能是这种情况）
        // 就**枚举当前账号能用的模式**逐个查 —— 直接交给内核会在 PG 上列出全库所有 schema，
        // 等于绕过权限过滤。模式数量有上限（见 SCHEMA_CAP），避免极端库把一次搜索放大成几百条 SQL。
        const SCHEMA_CAP: usize = 40;
        let scopes: Vec<String> = match schema.as_deref() {
            Some(one) => vec![one.to_string()],
            None => match dialect.schemas() {
                Meta::Sql(sql) => match run_meta_sql(&state, &id, &database, sql, META_MAX_ROWS).await {
                    Ok(result) => rows_of(&result)
                        .iter()
                        .map(|row| first_column_text(row))
                        .filter(|name| !name.trim().is_empty())
                        .take(SCHEMA_CAP)
                        .collect(),
                    Err(_) => Vec::new(),
                },
                _ => Vec::new(),
            },
        };
        for scope in scopes {
            let Meta::Sql(sql) = dialect.tables_in_schema(&scope) else {
                continue;
            };
            let Ok(result) = run_meta_sql(&state, &id, &database, sql, META_MAX_ROWS).await else {
                continue;
            };
            for row in rows_of(&result) {
                let name = text_ci(&row, "name");
                if !name.trim().is_empty() {
                    table_names.push(name);
                }
            }
            from_dialect = true;
        }
    }
    if !from_dialect {
        let engine = state.engine();
        let connection = target.clone();
        table_names = blocking(move || engine.list_tables_fresh(&connection))
            .await?
            .into_iter()
            .map(|table| table.name)
            .collect();
        // 同 `/tables`：H2 / DB2 / Derby 的 `getTables` 不看权限，搜索也必须求交，
        // 否则「树上没有的表，搜索结果里能点进去」。
        if let Some(allowed) = visible_table_set(&state, &id, &database, dialect).await {
            table_names.retain(|name| allowed.contains(&name.to_ascii_lowercase()));
        }
    }
    let hit_tables: Vec<String> = table_names
        .iter()
        .filter(|name| name.to_lowercase().contains(&keyword))
        .cloned()
        .collect();

    // 列名搜索保持「逐表 list_columns」不变：内核按会话缓存了列元数据，热态每张表
    // 只有 0.3ms 左右（实测 demo 库 95 张表 ~90ms），改成一条 information_schema 查询
    // 反而更慢（ClickHouse / Doris 热态会从 ~70ms 涨到 ~160ms），故不做此优化。
    let mut columns: Vec<Value> = Vec::new();
    for name in table_names.iter().take(MAX_TABLES) {
        if columns.len() >= MAX_COLUMNS {
            break;
        }
        let engine = state.engine();
        let table = name.clone();
        let connection = target.clone();
        let cols = match blocking(move || engine.list_columns(&connection, &table)).await {
            Ok(cols) => cols,
            // 单张表拉列失败（权限/视图）不该让整次搜索失败
            Err(_) => continue,
        };
        for column in cols {
            if column.name.to_lowercase().contains(&keyword) {
                columns.push(json!({
                    "table": name,
                    "column": column.name,
                    "type": column.type_name.unwrap_or_default(),
                    "comment": Value::Null,
                }));
            }
        }
    }
    Ok(Json(json!({ "tables": hit_tables, "columns": columns })))
}

// ------------------------------------------------------------------ 表数据

/// 「这张表在哪」——表名可能带 schema 前缀，逐段加引号。
/// LIKE 模式里的通配符转义：用户文本里的 `%`/`_` 要按字面匹配，得先逃掉。
/// **不转义单引号** —— 产出必然再过一遍 `dialect.literal`，那里会转；在这里先转
/// 会被二次翻倍（`O'Brien` → `O''''Brien`），含引号的搜索永远匹配不到。
pub(crate) fn escape_like(value: &str) -> String {
    // 必须先转义反斜杠本身：否则用户文本 `a\%` 会产出 `a\\%`，LIKE 里 `\\` 是字面
    // 反斜杠、`%` 仍是通配符，搜出一堆并不含该关键字的行（显式 ESCAPE '\' 同理）。
    value.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// `escape_like()` 产出的是 `\%` / `\_`，但**只有 MySQL/PG 族默认把 `\` 当 LIKE 转义符**；
/// SQLite / SQL Server / Oracle / DM / DB2 / H2 / Derby 没有默认转义符，`\_` 会被理解成
/// 「一个反斜杠 + 任意一个字符」⇒ 关键字里带 `_` 或 `%`（`user_name`、`50%`）时一条都搜不到，
/// 界面显示「没有数据」，看起来像库里真没有。这些方言必须显式声明 escape。
pub(crate) fn like_escape(dialect: &Dialect) -> &'static str {
    dialect.like_escape_suffix()
}

/// 高级搜索里的一条条件 → SQL 片段。拼不出来（列名空、数值列填了非数字）就返回 None。
///
/// 安全上有两条硬规矩：
/// * 列名一律走 `dialect.quote`（保留字、大小写都属于方言的事）；
/// * 字符串值一律走 `dialect.literal`（转义单引号）；数值列则**只允许数字字符**
///   并直接写裸数字 —— `int = '548'` 在 PostgreSQL 上会直接报错，
///   顺带也堵住了"把任意串塞进数值比较"这条路。
fn filter_condition(dialect: &Dialect, item: &Value) -> Option<String> {
    let column_name = item.get("col").and_then(Value::as_str)?.trim();
    if column_name.is_empty() {
        return None;
    }
    let op = item.get("op").and_then(Value::as_str).unwrap_or("eq");
    let kind = item.get("colType").and_then(Value::as_str).unwrap_or("string");
    // 列名来自**目录元数据**（前端拿 /columns 的返回值填的），必须逐字加引号：
    // quote() 会在 Oracle/DB2/H2/Derby 上把 `day` 折成 `DAY`，小写引号列直接
    // `ORA-00904: "DAY": 标识符无效`（豫园 11g 实测）。
    let column = dialect.quote_exact(column_name);

    // 空值判断不需要值
    match op {
        "is_null" => return Some(format!("{column} is null")),
        "is_not_null" => return Some(format!("{column} is not null")),
        _ => {}
    }

    let text = item.get("value").and_then(Value::as_str).unwrap_or_default();
    match op {
        "between" => {
            let low = scalar_for(dialect, kind, text)?;
            let high = scalar_for(
                dialect,
                kind,
                item.get("value2").and_then(Value::as_str).unwrap_or_default(),
            )?;
            Some(format!("{column} between {low} and {high}"))
        }
        "contains" | "not_contains" | "starts_with" | "ends_with" => {
            let pattern = match op {
                "starts_with" => format!("{}%", escape_like(text)),
                "ends_with" => format!("%{}", escape_like(text)),
                _ => format!("%{}%", escape_like(text)),
            };
            let keyword = if op == "not_contains" { "not like" } else { "like" };
            Some(format!(
                "{column} {keyword} {}{}",
                dialect.literal(&pattern),
                like_escape(dialect)
            ))
        }
        _ => {
            let value = scalar_for(dialect, kind, text)?;
            let operator = match op {
                "ne" => "<>",
                "gt" => ">",
                "lt" => "<",
                "gte" => ">=",
                "lte" => "<=",
                _ => "=",
            };
            Some(format!("{column} {operator} {value}"))
        }
    }
}

/// 条件里的一个值 → SQL 标量：日期列先试显式转换（Oracle 族，见 [`temporal_literal`]），
/// 其余按数值 / 字符串处理。
fn scalar_for(dialect: &Dialect, kind: &str, raw: &str) -> Option<String> {
    if let Some(literal) = temporal_literal(dialect, kind, raw) {
        return Some(literal);
    }
    scalar_of(dialect, kind, raw)
}

/// 一个值 → SQL 标量：数值列直接写数字（且必须真是数字），其余转义加引号。
fn scalar_of(dialect: &Dialect, kind: &str, raw: &str) -> Option<String> {
    if kind == "number" {
        let trimmed = raw.trim();
        let looks_numeric = !trimmed.is_empty()
            && trimmed
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
            && trimmed.parse::<f64>().is_ok();
        return looks_numeric.then(|| trimmed.to_string());
    }
    Some(dialect.literal(raw))
}

/// 整组高级搜索条件 → 一个 SQL 条件。
///
/// 两级结构（前端「多套条件」）：`item.group` 是组序号、`item.join` 是**组内**连接符、
/// `item.groupJoin` 是这一组与**上一组**的连接符，拼出来形如
/// `((a AND b) OR (c AND d))`。没有 `group` 字段的老请求全部落到 0 号组，
/// 与以前的"逐条从左往右"完全等价。
///
/// 之所以要显式加括号：SQL 里 AND 优先级高于 OR，不加括号时 `a AND b OR c` 会被读成
/// `(a AND b) OR c`，而用户按界面那样理解可能是 `a AND (b OR c)` —— 这种"看起来一样、
/// 筛出来不一样"的差异最难查，所以在源头就消掉。
pub(crate) fn filter_clause(dialect: &Dialect, filters: &[Value]) -> Option<String> {
    struct Acc {
        idx: u64,
        between: &'static str,
        joined: Option<String>,
    }

    let join_of = |item: &Value, key: &str| -> &'static str {
        match item.get(key).and_then(Value::as_str) {
            Some(v) if v.eq_ignore_ascii_case("or") => "or",
            _ => "and",
        }
    };

    let mut accs: Vec<Acc> = Vec::new();
    for item in filters {
        let Some(condition) = filter_condition(dialect, item) else {
            continue;
        };
        let idx = item.get("group").and_then(Value::as_u64).unwrap_or(0);
        let join = join_of(item, "join");
        let between = join_of(item, "groupJoin");

        let slot = match accs.iter().position(|a| a.idx == idx) {
            Some(pos) => &mut accs[pos],
            None => {
                accs.push(Acc {
                    idx,
                    between,
                    joined: None,
                });
                accs.last_mut().expect("刚 push 过")
            }
        };
        slot.joined = Some(match slot.joined.take() {
            None => format!("({condition})"),
            Some(previous) => format!("({previous} {join} ({condition}))"),
        });
    }

    let mut accs = accs.into_iter();
    let first = accs.next()?;
    let mut combined = first.joined?;
    for acc in accs {
        let Some(group) = acc.joined else { continue };
        combined = format!("({combined} {} ({group}))", acc.between);
    }
    Some(combined)
}

/// 排序键解析：多列 `orderColumns=age:ASC,name:DESC`（按**优先级**从左到右），
/// 老前端只传单列 `orderColumn`/`orderDir` 也认（前者非空时优先）。
///
/// 为什么在这里解析而不是直接拼字符串：方向必须是白名单（`asc`/`desc`），
/// 否则 `orderColumns=x:;drop table t` 这类就顺着 URL 进 SQL 了。
/// 列名本身交给 `dialect.quote()` 兜住，这里的过滤只是防脏值。
pub(crate) fn parse_order_specs(params: &Params) -> Vec<(String, String)> {
    const MAX_KEYS: usize = 4;
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(raw) = params.get("orderColumns") {
        for part in raw.split(',') {
            let mut it = part.splitn(2, ':');
            let col = it.next().unwrap_or_default().trim();
            if col.is_empty() {
                continue;
            }
            let dir = match it.next().unwrap_or("asc").trim() {
                d if d.eq_ignore_ascii_case("desc") => "desc",
                _ => "asc",
            };
            // 同列重复只留第一次：`a:ASC,a:DESC` 这种在前端不该出现，后端也别拼出两条
            if out.iter().any(|(c, _)| c == col) {
                continue;
            }
            out.push((col.to_string(), dir.to_string()));
            if out.len() >= MAX_KEYS {
                break;
            }
        }
    }
    if out.is_empty() {
        // 先绑到变量再 trim：`unwrap_or_default()` 返回的是 owned String，
        // 直接对临时值取 &str 会 E0716（temporary value dropped while borrowed）
        let raw_col = params.get("orderColumn").unwrap_or_default();
        let col = raw_col.trim();
        if !col.is_empty() {
            let desc = params
                .get("orderDir")
                .map(|d| d.eq_ignore_ascii_case("desc"))
                .unwrap_or(false);
            out.push((col.to_string(), if desc { "desc" } else { "asc" }.to_string()));
        }
    }
    out
}

/// 外部 catalog 专用执行：不切库（JDBC 表达不了 `catalog.db`，`scope::resolve` 对它明确拒绝），

#[cfg(test)]
mod tests {
    use super::*;

    fn dialect(key: &str) -> crate::api::dialect::Dialect {
        let kind = dbmind_core::ConnectionKind::from_key(key).expect("未知的连接类型");
        crate::api::dialect::Dialect::new(kind)
    }
    /// 高级搜索按日期列筛选时也要走显式转换（同一类故障的另一条入口）。
    #[test]
    fn 高级搜索的日期条件走_to_date() {
        let oracle = dialect("oracle");
        let filters = vec![serde_json::json!({
            "join": "AND", "col": "day", "op": "between",
            "value": "2026-01-01", "value2": "2026-01-31", "colType": "date"
        })];
        let clause = filter_clause(&oracle, &filters).unwrap_or_default();
        assert!(
            clause.contains("to_date('2026-01-01', 'YYYY-MM-DD')"),
            "日期条件没用 to_date：{clause}"
        );
        assert!(
            clause.contains("to_date('2026-01-31', 'YYYY-MM-DD')"),
            "上界也没用 to_date：{clause}"
        );
    }

    /// 守门：每个已实现的关系型（protocol=sql）类型都必须在 per-key 能力清单里**显式表态**。
    ///
    /// `column_types` / `edit_capabilities` 这批清单按 `kind.key()` 分派，新类型漏配的
    /// 后果是**静默降级**：表结构编辑器整页只读、列类型下拉为空 —— 路由照常走（它按
    /// module 分派），表面一切正常，用户进来才发现什么都点不动。TiDB 加回来时就这么
    /// 漏过一次（YAML/manifest/路由三处都对，就是这批清单没人记得补）。
    #[test]
    fn 已实现的关系型类型不得落进能力清单的_generic_兜底() {
        let mut checked = 0;
        for d in dbmind_core::all_descriptors() {
            // 只守「界面可见」的类型：EXPOSED_TYPE_CODES 是 YAML 的 exposed 过滤结果，
            // 内核实现了但没暴露的（duckdb）不在界面上，漏配无碍 —— 等哪天它
            // exposed=true 暴露进「新建数据源」，这条守门自然把它罩住。
            let exposed = dbmind_core::EXPOSED_TYPE_CODES
                .iter()
                .any(|(c, _)| *c == d.key.to_ascii_uppercase());
            if d.protocol.as_str() != "sql" || !exposed {
                continue;
            }
            checked += 1;
            let key = d.key;
            let kind = dbmind_core::ConnectionKind::from_key(key).expect("类型 key 必须能还原枚举");
            assert!(
                !editor::column_types(kind).is_empty(),
                "{key} 的列类型清单为空 —— 方言族的 column_type_names 漏了这条"
            );
            let edit = editor::for_kind(kind);
            assert_ne!(
                edit.ddl_style, "generic",
                "{key} 的结构编辑能力落进了 generic 兜底 —— 方言族没有 edit_capabilities 实现"
            );
            // MySQL 族的索引类型/存储引擎清单必须显式表态（TiDB 当年全漏，下拉整列消失）：
            // 族内新类型与 MySQL 行为一致，漏配只会静默给空 —— 这里强制非空。
            if d.dialect == "MySQL" {
                assert!(
                    !editor::index_types(kind).is_empty(),
                    "{key} 的索引类型清单为空 —— 方言族没有 index_type_names"
                );
                assert!(
                    !editor::storage_engines(kind).is_empty(),
                    "{key} 的存储引擎清单为空 —— 方言族没有 storage_engines"
                );
            }
        }
        assert!(
            checked >= 10,
            "遍历到的关系型类型只有 {checked} 个，清单或过滤条件可能坏了"
        );
    }
}
