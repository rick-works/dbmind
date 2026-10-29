//! `/api/{module}/{id}/…` —— 关系型元数据、表数据与表操作。
//!
//! 这一层是「上游界面 ↔ DBmind 内核」落差最大的地方，处理原则有三条：
//!
//! 1. **能走内核就走内核**。表/列清单用内核实现在走 `DatabaseMetaData` 的版本（各驱动都验过），
//!    比在兼容层重写一遍元数据 SQL 稳。
//! 2. **内核没有的按方言拼 SQL 执行**（索引 / 过程 / 触发器 / 事件 / 用户 / 建表语句）。
//!    这本来是上游Java 后端各方言模块的职责，落点不同、职责相同。方言覆盖率见 `dialect.rs`。
//! 3. **分不清「没有」和「没写」的地方，一律按「没写」报错**。空数组会被界面读成
//!    「这个库里没有索引」，那是另一种坏消息。
//!
//! 分页/关键字/排序这类**界面语义**也在这里翻译成 SQL：内核只认「一条语句」，
//! 没有「给这张表分页」这种概念。

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::Json;
use dbmind_core::{
    AccessContext, CellValue, ColumnDetail, ConnectionKind, ConnectionRecord, DbMindError,
    ErrorCode, QueryOptions, QueryRequest, QueryResult,
};
use serde_json::{json, Map, Value};

use crate::api::dialect::{Dialect, Meta};
use crate::api::error::{XError, XResult};
use crate::api::{blocking, conn as conn_api, require_record, shape, Params};
use crate::AppState;

/// 元数据查询的默认行上限：元数据是「列表」，给足但不放任。
/// 元数据清单（表/视图、例程、触发器、用户…）的**行数上界**。
///
/// 以前这里是写死的 5000，于是**静默截断**：Oracle 的 SYS 有 8180 个视图，
/// 树上只给 4562（表 438 + 视图 4562 正好 5000），少的三千多个一声不响 ——
/// 「看起来就是全部」比少几行更坏。
///
/// 现在**不设人为上限**，直接取引擎的硬顶（`QueryOptions::HARD_MAX_ROWS`，
/// 内核每请求都会 `clamp` 到这个值）：真实库里再大的 schema 也到不了，
/// 而物理上仍有一条线兜底 —— 真撞上了（见 `meta_list` 的 warn 与界面上的 `+`）
/// 也还是"如实说出来"，不是悄悄少给。所有数据源共用这一条。
pub(crate) const META_MAX_ROWS: usize = QueryOptions::HARD_MAX_ROWS;

/// 「连接失效后重连重试一次」只在这个窗口内做。
///
/// 重试本来是给**会话被服务端掐掉**这种情况用的：下一次调用立刻被拒（毫秒级），
/// 重连一次很值得。但如果失败是**超时**（TCP/握手连不上），重试就是把同一段等待
/// 再交一遍学费 —— 真机实测一条连不上的 MySQL：10 秒连接超时 × 2 = 20 秒，
/// 用户点开连接一直转圈，还以为是自己库慢。这条线只对「等得已经够久了」的失败生效，
/// 与方言无关，所有数据源一视同仁。
const CONNECT_RETRY_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

// ------------------------------------------------------------------ 基础工具

pub(crate) async fn ctx_of(state: &AppState, id: &str) -> XResult<(ConnectionRecord, Dialect)> {
    let record = require_record(state, id).await?;
    let dialect = Dialect::new(record.kind());
    Ok((record, dialect))
}

/// 执行一条语句（内部用）。默认行数给足，免得「看一张大表」被截断。
pub async fn run_sql(
    state: &AppState,
    conn: &str,
    sql: String,
    max_rows: usize,
) -> XResult<QueryResult> {
    run_sql_in(state, conn, "", sql, max_rows).await
}

/// 从「库.模式」里取出 schema（非分层库、或没写模式时是 `None`）。
///
/// 为什么不给 `run_sql_in` / `run_meta_sql` 加参数、而是从 `database` 里拆：
/// 这两个入口有 **30 多处**调用方（DDL / 索引 / 注释 / 列类型补全 / 库清单 / 统计…），
/// 而它们传下来的本来就是同一个「库.模式」字符串 —— 在这里拆一次，所有元数据 SQL 一次性
/// 都对（代理会"临时切 schema → 执行 → 切回"），调用方一行都不用改。
///
/// 取不到方言（连接刚被删掉之类）不报错：schema 只是"更精确"，缺了顶多退回老行为。
async fn meta_schema_of(state: &AppState, conn: &str, database: &str) -> Option<String> {
    let dialect = Dialect::new(require_record(state, conn).await.ok()?.kind());
    split_db_schema(database, &dialect).1
}

/// 同上，但**切到指定库**执行 —— 跨库浏览全靠它。
///
/// `database` 为空 = 用连接自己绑定的那个库。非空时由 `scope::resolve` 决定是直接用主连接、
/// 还是换成一条指向该库的影子连接；这件事对调用方完全透明。
pub async fn run_sql_in(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
    max_rows: usize,
) -> XResult<QueryResult> {
    run_sql_in_inner(state, conn, database, sql, max_rows, None, false).await
}

/// 同 [`run_sql_in`]，但显式要求**可写会话**。
///
/// 只给「本来就是写」的语句用：清空目标表、对账不一致后的重传前置清空。
/// 这些语句过去走 [`run_sql_in`]（`read_only: Some(true)`），在 MySQL 上直接被驱动拒绝
/// （`Connection is read-only...`），表现为「行数不一致 → 清空重传 → 跳过重传」，
/// 差异永远补不上。
pub(crate) async fn run_sql_in_write(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
    max_rows: usize,
) -> XResult<QueryResult> {
    // 与会话同步的 DDL 共用那一条写连接：不能传 `None`（那是元数据会话，`read_only=true`，
    // 标记不同照样要另开连接），也不能每张表一条（会把目标连接的并发上限占满）。
    run_sql_in_inner(
        state,
        conn,
        database,
        sql,
        max_rows,
        Some(crate::api::sync::SESSION_SYNC_DDL),
        true,
    )
    .await
}

async fn run_sql_in_inner(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
    max_rows: usize,
    session: Option<&str>,
    write: bool,
) -> XResult<QueryResult> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    // 元数据也是「连上去查」：驱动没装就先下（离线时只失败一次，原因会被缓存）
    crate::api::driver::ensure_for_connection(state, &target).await?;
    // SQL 层切库兜底（SQL Server）：与 run_meta_sql 同一套双保险 —— 影子连接负责把执行
    // 落到目标库，这里再用 `use [库];` 兜一层，行数/索引/DDL/用户操作驱动的语句同理
    // 不会因为落回主连接而打错库（用户实测过表清单串台）。
    let sql = {
        let base = require_record(state, conn).await?;
        let dialect = Dialect::new(base.kind());
        if dialect.wants_use_switch() {
            let (db, _) = split_db_schema(database, &dialect);
            if db.is_empty() { sql } else { format!("use {}; {}", dialect.quote(&db), sql) }
        } else {
            sql
        }
    };
    let engine = state.engine();
    let schema = meta_schema_of(state, conn, database).await;
    let request = QueryRequest {
        schema,
        connection: target,
        sql,
        options: QueryOptions {
            max_rows,
            timeout_ms: 120_000,
        },
        execution_id: None,
        // 刻意**不**给会话亲和键（`session: None`），让它复用元数据那条会话；
        // 唯一的例外是 [`run_sql_in_background`]（行数回填）带独立键 `internal:rowcount` ——
        // 回填几十条 COUNT 不能和用户的元数据请求挤同一条连接（见该函数注释）。
        //
        // 原来这里写的是 `Some("internal:browse")`，本意是「别和用户页签的会话抢」——
        // 那份隔离是对的（用户查询自带 key，仍互不干扰），但代价被低估了：
        // 亲和键会参与会话键的计算，于是**元数据和浏览落在两条物理会话上**。
        // 实测（真机 MySQL，宿主 trace）：一次 `/tables` 请求建了两条会话 ——
        // 元数据一条、随后的「行数估算」一条（建连 306ms），而它们本来就是同一条库里
        // 同一件事。会话一旦建好就会复用，所以这是**每个连接一次**的白开销。
        //
        // 去掉之后两者共用一条会话（内核本来就是单连接泳道，同一会话上串行执行）。
        // 浏览类语句都是只读 SELECT，不会污染会话状态。
        session: session.map(String::from),
        // 还要把只读标记对齐：结构浏览对这类库走的是**只读会话**
        // （`metadata_session_key` 里 `session_key(config, true)`），而 `execute` 缺省按
        // 连接策略算 —— 标记不一致的话，会话键的第 7 段不同，照样是两条连接。
        //
        // 默认只读：这条入口绝大多数语句是只读 SELECT。但**清空表 / 断连重跑前清空**
        // 这类写语句必须显式走 [`run_sql_in_write`]（`write = true`）—— 否则驱动会直接
        // 拒绝：`Connection is read-only. Queries leading to data modification are not
        // allowed.`（真机：收尾对账发现行数不一致、要清空重传时，truncate 就在这里失败，
        // 于是差异永远补不上）。
        read_only: if write { Some(false) } else { Some(true) },
        // **不进查询历史**：这条路上跑的全是元数据 / 结构浏览 / 表格预览，
        // 以及界面功能（表结构编辑、清空表、建用户、终止会话…）驱动的语句 ——
        // 都不是「用户在 SQL 编辑器里敲的 SQL」。标记在**这个共用入口**上，
        // 比让几十个调用方各自记得打标记可靠（首页「最近查询」曾被它们淹掉）。
        internal: true,
    };
    blocking(move || engine.execute(request, AccessContext::Web)).await
}
/// 同 [`run_sql_in`]，但带**独立的会话亲和键** —— 给「行数回填」这类后台批量任务用。
/// 回填一批要串行跑几十条 `COUNT(*)`，以前与表结构 / 表清单等元数据**共用同一个
/// 会话键**（也就共用同一条 JDBC 连接）：树里把几个库都展开时，回填把连接占满，
/// 用户点开「编辑表结构」的取列请求只能排在几十条 COUNT 后面 —— 服务器一忙就
/// 撞上元数据 30 秒超时（真机回归：v1.2.28 修了预热与 ClickHouse 建连后仍复现）。
/// 独立亲和键 ⇒ 独立物理会话 ⇒ 回填再忙也不堵用户的元数据请求。
/// 代价是每个库多一条连接（一条给回填、一条给元数据），换来互不阻塞。
pub(crate) async fn run_sql_in_background(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
    max_rows: usize,
) -> XResult<QueryResult> {
    run_sql_in_inner(state, conn, database, sql, max_rows, Some("internal:rowcount"), false).await
}

/// 同 [`run_sql_in`]，但会话亲和键由调用方给 —— 用于「一个任务并发开几条读连接」的场景
/// （数据传输的并行分片）。**每片必须给不同的键**，否则几片会挤在同一条物理连接上排队：
/// 分片就退化成「多打几次往返」，比不分片还慢（真机实测 20 万行 4 片 11s vs 单流 5s，
/// 就是因为四片共用 `internal:rowcount` 这个键）。
pub(crate) async fn run_sql_in_keyed(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
    max_rows: usize,
    session: &str,
) -> XResult<QueryResult> {
    run_sql_in_inner(state, conn, database, sql, max_rows, Some(session), false).await
}


/// 同 [`run_sql_in`]，但**跟随连接的读写策略**（`read_only: None`）——
/// 给界面功能里那些**会写库**的语句用：还原、表结构编辑、清空/删除表、终止会话、建用户。
///
/// 为什么必须分开：`run_sql_in` 服务于结构浏览，走的是**只读会话**（省一条连接、语义也对）；
/// 但 JDBC 的只读是**连到驱动上**的 —— MySQL 会直接拒绝
/// `Connection is read-only. Queries leading to data modification are not allowed`。
/// 还原曾因此全军覆没：连接明明**没开**只读开关，还原的第一句 `drop table if exists`
/// 却被驱动拦下 —— 因为语句被塞进了结构浏览那条只读会话里。
///
/// 同样 `internal: true`（这些是界面功能驱动的语句，不是用户在编辑器里敲的 SQL）。
pub async fn run_write_sql_in(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
    max_rows: usize,
    audit: bool,
) -> XResult<QueryResult> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let started = std::time::Instant::now();
    let outcome = run_write_sql_target(state, &target, sql.clone(), max_rows).await;
    // 界面功能驱动的写库动作补记审计（internal 不进执行历史，但改库必须留痕）
    if audit {
        let engine = state.engine();
        let (ok, err_code) = match &outcome {
            Ok(_) => (true, None),
            Err(err) => (false, Some(err.message.clone())),
        };
        engine.audit_external(conn, &sql, ok, started.elapsed().as_millis() as u64, err_code);
    }
    outcome
}

/// 与 [`run_write_sql_in`] 同一条执行路径，区别是**目标作用域已经解析好**。
///
/// 解析一次 `scope::resolve` 要查连接记录、按方言拼出影子连接；驱动就绪检查还要
/// 读一遍驱动目录。对「一条语句一次调用」的场景（表结构编辑、终止会话）这点开销
/// 无所谓，但**还原是几万到几十万条语句**：每条都重来一遍，光这些固定开销就能把
/// 还原拖成几十分钟（实测 20 万条语句的备份文件，大头压根不在数据库上）。
/// 调用方解析一次、把 `target` 传进来复用即可 —— 执行语义与 `run_write_sql_in`
/// 完全一致（同样跟随连接策略、同样 `internal: true`），只是不做审计
/// （还原的审计由调用方按「整次一条」记录，见 `backup::spawn_restore`）。
pub(crate) async fn run_write_sql_target(
    state: &AppState,
    target: &str,
    sql: String,
    max_rows: usize,
) -> XResult<QueryResult> {
    let engine = state.engine();
    let request = QueryRequest {
        schema: None,
        connection: target.to_string(),
        sql,
        options: QueryOptions {
            max_rows,
            timeout_ms: 120_000,
        },
        execution_id: None,
        session: None,
        // 跟随连接策略：连接没标只读 ⇒ 可写会话（还原/改结构才落得下去）；
        // 连接标了只读 ⇒ 语句在闸门就被拦下，根本走不到驱动。
        read_only: None,
        internal: true,
    };
    blocking(move || engine.execute(request, AccessContext::Web)).await
}

/// 与 `crate::api::blocking` 干同一件事，唯一区别是**保留内核错误码**。
///
/// 兼容层那个 `blocking` 会把内核错误压成 `XError{status, message}` —— 界面只需要 message
/// （见 `api/error.rs` 的头注），但**重试判据需要错误码**：只有码能区分「连接失效
/// （该重连重试）」和「SQL 写错了（重试没用）」。所以在这一处保留原始错误。
async fn run_kernel<T, F>(task: F) -> dbmind_core::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> dbmind_core::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(task).await {
        Ok(result) => result,
        Err(err) => Err(DbMindError::new(
            ErrorCode::Internal,
            format!("工作线程异常：{err}"),
        )),
    }
}

/// 结构浏览用的方言 SQL：**只读、幂等**，连接失效时重连重试一次。
///
/// 为什么要在这里自己补一次重试：内核的**查询**路径按设计不重试（查询不幂等，响应丢了
/// 可能已经在库里执行过，见 `agent_driver::with_lane` 的模块文档），而本文件里这批
/// 方言 SQL（`dialect.rs` 的 `Meta::Sql`）是**我们自己写死的只读元数据查询**，完全幂等。
///
/// 不补的实测后果（真机 MySQL 抓到的）：`wait_timeout` 把连接切掉之后，
/// ① 「库清单」一次失败就退回单库模式，树上于是出现一个 `(default)` **冒充真库**；
/// ② 索引/过程/触发器等分类整体变空，用户读成「这个库没有索引」—— 又一个「看着像空、
/// 其实断了」。宿主已经在连接失效时摘掉坏会话（见 `dbmind-agent-jdbc` 的
/// `metadataFailure`），所以这里重试的这一次会**真的重连**。
async fn run_meta_sql(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
    max_rows: usize,
) -> XResult<QueryResult> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    // SQL 层切库兜底（SQL Server）：这批元数据 SQL 全是「库内限定」（from sys.objects …），
    // 正确性完全押在「执行真的落在影子连接上」。万一落回主连接（旧影子落库 / 会话复用
    // 异常），就会查出**默认库**的表 —— 两个库节点显示同一份清单（用户实测的"串台"）。
    // 影子连接是第一道保险，这里补第二道：显式 `use [库];`。USE 到同一个库对影子无副作用。
    let sql = {
        let base = require_record(state, conn).await?;
        let dialect = Dialect::new(base.kind());
        if dialect.wants_use_switch() {
            let (db, _) = split_db_schema(database, &dialect);
            if db.is_empty() { sql } else { format!("use {}; {}", dialect.quote(&db), sql) }
        } else {
            sql
        }
    };
    let engine = state.engine();
    // 与结构浏览共用**同一条只读会话**。
    //
    // 这个函数查的全是只读元数据（库清单 / schema 清单 / 索引·过程·触发器分类 / 库级统计），
    // 而结构浏览对非 connection-scoped 的类型走的是 `session_key(config, true)`。
    // 两边对齐才只有一条物理连接 —— 否则同一库同一件事各占一条（各 ~300ms，见
    // `run_sql_in` 的同类注释与实测）。所以这里既去掉 `internal:browse` 亲和键
    // （它会参与会话键计算，正是分裂的原因），也把只读标记对齐。
    // 另：元数据**不进查询历史**（`internal`），理由见 `run_sql_in`。
    let schema = meta_schema_of(state, conn, database).await;
    let first = QueryRequest {
        schema: schema.clone(),
        read_only: Some(true),
        connection: target.clone(),
        sql: sql.clone(),
        options: QueryOptions {
            max_rows,
            timeout_ms: 120_000,
        },
        execution_id: None,
        session: None,
        internal: true,
    };
    let engine_first = engine.clone();
    let attempt_started = std::time::Instant::now();
    match run_kernel(move || engine_first.execute(first, AccessContext::Web)).await {
        // 连接失败但**不是快速失败**（撞的是连接超时）：不再重试。
        // 见 CONNECT_RETRY_WINDOW 的说明 —— 重试只会让用户多等一整段超时。
        Err(err)
            if err.code == ErrorCode::ConnConnectFailed
                && attempt_started.elapsed() >= CONNECT_RETRY_WINDOW =>
        {
            tracing::warn!(
                connection = %conn,
                waited_ms = attempt_started.elapsed().as_millis() as u64,
                "元数据查询连接失败且是超时（不是被拒），不再重试"
            );
            Err(err.into())
        }
        Err(err) if err.code == ErrorCode::ConnConnectFailed => {
            tracing::warn!(connection = %conn, "元数据查询遇到连接失效，重连后重试一次");
            // 重试那次与首次保持完全一致（同只读会话）：会话失效时宿主已摘掉坏会话，
            // 这一次会真的重连
            let retry = QueryRequest {
                schema,
                read_only: Some(true),
                connection: target,
                sql,
                options: QueryOptions {
                    max_rows,
                    timeout_ms: 120_000,
                },
                execution_id: None,
                session: None,
                internal: true,
            };
            blocking(move || engine.execute(retry, AccessContext::Web))
                .await
                .map_err(Into::into)
        }
        other => other.map_err(Into::into),
    }
}

/// 取一行的 **catalog 名**。
///
/// 与 `first_column_text` 的唯一区别：`show catalogs` 的**第一列是 `CatalogId`**（数字），
/// 名字在第二列 `CatalogName` —— 直接取第一列会得到 `0` / `21275` 这种 ID，
/// 界面上就成了一堆数字节点。Doris 又不支持把 `show` 包成子查询去挑列（实测语法错误），
/// 所以只能在结果里按列名选：先认这几个已知列名，都没有才退回第一列
/// （保住"万一以后换成别的写法"时仍能用）。
fn catalog_column_text(row: &Map<String, Value>) -> String {
    for key in ["CatalogName", "catalog_name", "catalogname", "name"] {
        let text = text_ci(row, key);
        if !text.is_empty() {
            return text;
        }
    }
    first_column_text(row)
}

/// 取一行的第一列文本（各家元数据视图的列名不一致，能按 name 取就按 name 取）。
fn first_column_text(row: &Map<String, Value>) -> String {
    let named = text_ci(row, "name");
    if !named.is_empty() {
        return named;
    }
    match row.values().next() {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// 结果集 → 「列名 → 值」的对象数组（元数据接口都按列名取值）。
pub fn rows_of(result: &QueryResult) -> Vec<Map<String, Value>> {
    let names: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
    result
        .rows
        .iter()
        .map(|raw| {
            let mut row = Map::new();
            for (i, name) in names.iter().enumerate() {
                row.insert(
                    (*name).to_string(),
                    raw.get(i).map(shape::cell_to_value).unwrap_or(Value::Null),
                );
            }
            row
        })
        .collect()
}

pub fn first_text(result: &QueryResult) -> String {
    let row = match result.rows.first() {
        Some(row) => row,
        None => return String::new(),
    };
    match row.first() {
        Some(CellValue::Text(text)) => text.clone(),
        Some(other) => match shape::cell_to_value(other) {
            Value::Null => String::new(),
            Value::String(s) => s,
            other => other.to_string(),
        },
        None => String::new(),
    }
}

fn text_of(row: &Map<String, Value>, key: &str) -> String {
    match row.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// 大小写不敏感地取一个字段（各家元数据视图的列名大小写不一致）。
pub(crate) fn text_ci(row: &Map<String, Value>, key: &str) -> String {
    if row.contains_key(key) {
        return text_of(row, key);
    }
    let lower = key.to_ascii_lowercase();
    for (k, v) in row {
        if k.to_ascii_lowercase() == lower {
            return match v {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
        }
    }
    String::new()
}

/// 按 `Meta` 三档取元数据列表（`database` 非空时切到目标库去查）。
async fn meta_list(
    state: &AppState,
    id: &str,
    database: &str,
    meta: Meta,
    feature: &str,
) -> XResult<Value> {
    match meta {
        Meta::Absent => Ok(Value::Array(vec![])),
        Meta::Unwritten => Err(not_written(state, id, feature).await),
        Meta::Sql(sql) => {
            let result = run_meta_sql(state, id, database, sql, META_MAX_ROWS).await?;
            let rows = rows_of(&result);
            // 到顶 = 很可能被截断了。这里只写一条**警告**（界面上的 `5000+` 由前端按
            // META_MAX_ROWS 显示）：静默截断最坏的地方不是少几行，而是"看起来就是全部"。
            if rows.len() >= META_MAX_ROWS {
                tracing::warn!(
                    connection = %id,
                    feature,
                    limit = META_MAX_ROWS,
                    "元数据清单到达行数上限，结果可能已被截断"
                );
            }
            Ok(Value::Array(rows.into_iter().map(Value::Object).collect()))
        }
    }
}

/// 「这个概念存在、但本类型的语句还没写」——报错时把类型名带上，别让人以为是权限问题。
async fn not_written(state: &AppState, id: &str, feature: &str) -> XError {
    let key = require_record(state, id)
        .await
        .map(|r| r.kind().key())
        .unwrap_or("未知");
    XError::not_implemented(&format!("{feature}（{key}）"))
}

/// 库名参数可能是 `库.模式`（上游的 schema 节点就是这么拼的），拆开。
/// 取一个整数列（列名大小写不敏感 —— 有的驱动会把别名转成大写）。
///
/// 返回 `Option` 而不是 `unwrap_or(0)`：这里代表「行数」，而 0 与「取不到」是**两件事** ——
/// 前者是「这张表是空的」，后者是「我不知道」。混成 0 就是「树上每张表都显示 0」那个 bug 的形状。
fn cell_int(row: &Map<String, Value>, key: &str) -> Option<i64> {
    let mut found = row.get(key);
    if found.is_none() {
        let lower = key.to_ascii_lowercase();
        found = row
            .iter()
            .find(|(name, _)| name.to_ascii_lowercase() == lower)
            .map(|(_, value)| value);
    }
    // 转换统一走 shape::value_to_i64 —— 那是"整数 / 浮点 / 字符串"三种形态的唯一处理处。
    // 别在这里再抄一份：抄一份的下场就是别处漏掉其中一种（table_count 的历史）。
    shape::value_to_i64(found?)
}

pub(crate) fn split_db_schema(database: &str, dialect: &Dialect) -> (String, Option<String>) {
    if !dialect.is_schema_aware() {
        return (database.to_string(), None);
    }
    match database.split_once('.') {
        Some((db, schema)) if !schema.is_empty() => (db.to_string(), Some(schema.to_string())),
        _ => (database.to_string(), None),
    }
}

// ------------------------------------------------------------------ 特性与结构清单

/// 各类型的**候选类型清单**（结构编辑器 / 新建表的类型下拉）。
/// 统一口径：**不带死参数** —— 长度 / 精度由编辑器的「长度 / 精度」列给
///（回显已有列时按真实类型拆进两列；新增行对 char 系预填 255）。
/// 特例保留参数：`(max)` 这类编辑器表达不了的形态（SQL Server 的 NVARCHAR(max)）。
fn column_types(kind: ConnectionKind) -> Vec<&'static str> {
    match kind.key() {
        // 类型清单的**大小写跟该库自身元数据的报法一致**：MySQL 系（含 Doris）的
        // information_schema 一律回大写（`VARCHAR`），列表就写大写 —— 否则「下拉里是 varchar、
        // 回显出来是 VARCHAR」，同一个类型两种写法（用户反馈）。
        // PG 系的元数据本来就回小写（`varchar`/`timestamptz`），那里保持小写才是一致的。
        "mysql" | "mariadb" => vec![
            "INT", "BIGINT", "SMALLINT", "TINYINT", "DECIMAL", "FLOAT", "DOUBLE",
            "VARCHAR", "CHAR", "TEXT", "LONGTEXT", "DATE", "DATETIME", "TIMESTAMP",
            "TIME", "JSON", "BLOB",
        ],
        // Doris 与 MySQL 同一套类型，但它自己的元数据报 `string`（它的 VARCHAR 别名）——
        // 清单里带上 STRING，前端的"回显向清单对齐"才能把它收进同一种写法（见 typeName.js）。
        "doris" => vec![
            "INT", "BIGINT", "SMALLINT", "TINYINT", "DECIMAL", "FLOAT", "DOUBLE",
            "VARCHAR", "STRING", "CHAR", "TEXT", "LONGTEXT", "DATE", "DATETIME", "TIMESTAMP",
            "TIME", "JSON", "BLOB",
        ],
        // PG 系：前面的标准写法给人看，后面这几个是 **PG 自己 `udt_name` 报出来的写法**
        // （int2/int4/int8/float4/float8/varchar/bool）—— 少一个，回显就会是清单外的名字，
        // "下拉 = 回显"就断了（见 typeName.js）。
        "postgresql" | "kingbase" => vec![
            "integer", "bigint", "smallint", "numeric", "real", "double precision",
            "character varying", "text", "boolean", "date", "timestamp", "timestamptz",
            "time", "jsonb", "uuid", "bytea",
            "int2", "int4", "int8", "float4", "float8", "varchar", "bool",
        ],
        "sqlserver" => vec![
            "int", "bigint", "smallint", "tinyint", "decimal", "float", "real",
            "nvarchar", "varchar", "nvarchar(max)", "bit", "date", "datetime2",
            "datetimeoffset", "uniqueidentifier", "varbinary(max)",
        ],
        "oracle" | "dm" => vec![
            "NUMBER", "FLOAT", "BINARY_FLOAT", "VARCHAR2",
            "NVARCHAR2", "CHAR", "CLOB", "BLOB", "DATE", "TIMESTAMP", "RAW",
        ],
        "clickhouse" => vec![
            // Decimal / FixedString / DateTime64 的参数由「长度 / 精度」列给
            //（重建时 Decimal 缺省 38,6、FixedString 缺省 16、DateTime64 缺省 3，见前端 clickhouseType）
            "UInt8", "UInt16", "UInt32", "UInt64", "Int8", "Int16", "Int32", "Int64",
            "Float32", "Float64", "Decimal", "String", "FixedString",
            "Date", "DateTime", "DateTime64", "UUID", "Array(String)",
        ],
        "sqlite" => vec!["INTEGER", "REAL", "TEXT", "BLOB", "NUMERIC", "VARCHAR", "DATETIME"],
        "h2" => vec!["INT", "BIGINT", "DECIMAL", "DOUBLE", "VARCHAR", "CLOB", "BLOB", "DATE", "TIMESTAMP", "BOOLEAN"],
        "db2" => vec!["INTEGER", "BIGINT", "DECIMAL", "DOUBLE", "VARCHAR", "CLOB", "BLOB", "DATE", "TIMESTAMP"],
        "derby" => vec!["INTEGER", "BIGINT", "DECIMAL", "DOUBLE", "VARCHAR", "CLOB", "BLOB", "DATE", "TIMESTAMP"],
        _ => vec![],
    }
}

/// 方言能力表：界面据此决定显示哪些操作入口。
///
/// `supportsExport` / `supportsImport` 刻意给 **true**：导出导入还没落地（Phase 2），
/// 但**菜单必须照上游的样子在**（界面不变），点下去会拿到一条明确的
/// 「尚未接入」而不是一个不存在的入口 —— 藏在 features 里假装不支持才是更坏的做法。
/// **结构编辑器**需要的方言能力。
///
/// 这几个键不是"锦上添花"，少一个的后果是**整行变只读**：界面拿 `ddlStyle` 决定用哪套
/// ALTER 生成器、拿 `supportsColumnModify` 决定已加载行能不能改。落到 `generic` 分支时，
/// 类型/长度/可空/主键/默认值**全部点不动，而且不给任何解释** ——
/// 用户看到的就是「主键和可空为什么不能选」。
///
/// 诚实性要求：只声明**真的能生成正确 SQL** 的能力。界面会用这些开关直接产出 DDL，
/// 声明得比实现靠前，等于让用户去撞数据库的报错。
fn edit_capabilities(kind: &str) -> Value {
    // (ddlStyle, quoteStyle, 可改已有列, 列注释, 表选项, 自增, 索引重建)
    let (style, quote, modify, comment, table_options, auto_inc, rebuild) = match kind {
        "mysql" | "mariadb" => ("mysql", "BACKTICK", true, true, true, true, true),
        "doris" => ("doris", "BACKTICK", true, true, true, true, true),
        "postgresql" | "kingbase" => ("pg", "DOUBLE", true, true, false, false, true),
        // Oracle 12c+ 的标识列能查到（user_tab_identity_cols），自增如实给 true。
      // DM 单独拆出来给 false：达梦的标识列记在哪张系统视图上、没有实例可验证（见 dialect.rs 里
      // column_types 的 dm 分支注释）。原先跟着 Oracle 一起 true，结果界面上给了个**永远勾不上**的
      // 自增勾选框 —— 列已有 IDENTITY 也显示成没自增，等于骗人（用户反馈排查出来的同类问题）。
      // 与其留一个查不出来的勾选框，不如如实不显示这一列。
      "oracle" => ("oracle", "DOUBLE", true, true, false, true, true),
      "dm" => ("oracle", "DOUBLE", true, true, false, false, true),
        "sqlserver" => ("mssql", "BRACKET", true, true, false, true, true),
        // SQLite 没有 ALTER COLUMN：只能加列或重建表，如实说不能改
        "sqlite" => ("sqlite", "DOUBLE", false, false, false, false, false),
        "clickhouse" => ("clickhouse", "DOUBLE", true, true, false, false, false),
        // H2：**实测过**才敢开。在 H2 2.3 上逐条验过 pg 风格的
        // `alter table t alter column c type varchar(100)`、`numeric(12,3)`、
        // `set default`、`rename column` 全部可用，列注释 `comment on column` 也可用。
        // 索引重建 / 表选项 H2 没有对应语法，如实给 false。
        "h2" => ("pg", "DOUBLE", true, true, false, false, false),
        // Derby：**实测过**（Derby 10.16）才开。`ALTER COLUMN c SET DATA TYPE …`、
        // `SET DEFAULT …` / `DROP DEFAULT`、`NULL` / `NOT NULL`、`RENAME COLUMN a TO b`、
        // `ADD COLUMN`、`DROP COLUMN` 逐条验过可用（前端 `derby` 这一套生成器就是照它写的）。
        // 列注释 Derby 没有（comment=false）；表选项/自增/索引重建也没对应语法，如实给 false。
        "derby" => ("derby", "DOUBLE", true, false, false, false, false),
        // DB2 LUW：列属性可以改 —— `ALTER COLUMN … SET DATA TYPE` / `SET|DROP NOT NULL` /
        // `SET|DROP DEFAULT`、`RENAME COLUMN`、`ADD|DROP COLUMN`、`COMMENT ON COLUMN` 都有。
        // 原先 DB2 落到 generic、supportsColumnModify=false，整个字段表灰成只读
        //（用户看到的就是「DB2 的表结构什么都点不动」）。
        // 表选项 / 自增列 / 索引重建：DB2 没有引擎字符集这套，IDENTITY 的在线增删也没实测，如实 false。
        "db2" => ("db2", "DOUBLE", true, true, false, false, false),
        // 非关系型：界面按「不支持在线改结构」处理（没实测过就不声明 ——
        // 声明得比实现靠前，等于让用户去撞数据库的报错）
        _ => ("generic", "DOUBLE", false, false, false, false, false),
    };
    // (字符集清单, 字符集 → 排序规则映射)
    let (charsets, collations): (Vec<&str>, Value) = match kind {
        "mysql" | "mariadb" | "doris" => (
            vec![
                // 常用字符集（MySQL/MariaDB 的服务端标准集合在版本间很稳定）
                "utf8mb4", "utf8mb3", "utf8", "latin1", "latin2", "latin5", "latin7", "gbk", "gb18030",
                "big5", "ascii", "binary", "ucs2", "utf16", "utf16le", "utf32", "cp1250", "cp1251",
                "cp1256", "cp1257", "euckr", "sjis", "cp932", "greek", "hebrew", "koi8r", "koi8u",
                "tis620", "macroman", "armscii8", "geostd8", "keybcs2", "swe7",
            ],
            json!({
                // 排序规则**逐字符集写全**：这里只列真实存在的名字 —— 生成或臆造的名字会让建表直接失败。
                // 覆盖 utf8mb4 / utf8mb3 / utf8 / latin1 的完整常用集合，其余给该字符集的关键几条。
                "utf8mb4": [
                    "utf8mb4_0900_ai_ci", "utf8mb4_0900_as_ci", "utf8mb4_0900_as_cs", "utf8mb4_0900_bin",
                    "utf8mb4_general_ci", "utf8mb4_bin", "utf8mb4_unicode_ci", "utf8mb4_unicode_520_ci",
                    "utf8mb4_croatian_ci", "utf8mb4_czech_ci", "utf8mb4_danish_ci", "utf8mb4_esperanto_ci",
                    "utf8mb4_estonian_ci", "utf8mb4_hungarian_ci", "utf8mb4_icelandic_ci",
                    "utf8mb4_latvian_ci", "utf8mb4_lithuanian_ci", "utf8mb4_persian_ci", "utf8mb4_polish_ci",
                    "utf8mb4_romanian_ci", "utf8mb4_sinhala_ci", "utf8mb4_slovak_ci", "utf8mb4_slovenian_ci",
                    "utf8mb4_spanish_ci", "utf8mb4_spanish2_ci", "utf8mb4_swedish_ci", "utf8mb4_turkish_ci",
                    "utf8mb4_vietnamese_ci",
                ],
                "utf8mb3": [
                    "utf8mb3_general_ci", "utf8mb3_bin", "utf8mb3_unicode_ci", "utf8mb3_unicode_520_ci",
                    "utf8mb3_croatian_ci", "utf8mb3_czech_ci", "utf8mb3_danish_ci", "utf8mb3_esperanto_ci",
                    "utf8mb3_estonian_ci", "utf8mb3_general_mysql500_ci", "utf8mb3_hungarian_ci",
                    "utf8mb3_icelandic_ci", "utf8mb3_latvian_ci", "utf8mb3_lithuanian_ci",
                    "utf8mb3_persian_ci", "utf8mb3_polish_ci", "utf8mb3_romanian_ci", "utf8mb3_sinhala_ci",
                    "utf8mb3_slovak_ci", "utf8mb3_slovenian_ci", "utf8mb3_spanish_ci", "utf8mb3_spanish2_ci",
                    "utf8mb3_swedish_ci", "utf8mb3_turkish_ci", "utf8mb3_vietnamese_ci",
                ],
                "utf8": [
                    "utf8_general_ci", "utf8_bin", "utf8_unicode_ci", "utf8_unicode_520_ci",
                    "utf8_general_mysql500_ci", "utf8_czech_ci", "utf8_danish_ci", "utf8_icelandic_ci",
                    "utf8_latvian_ci", "utf8_lithuanian_ci", "utf8_polish_ci", "utf8_romanian_ci",
                    "utf8_slovak_ci", "utf8_slovenian_ci", "utf8_spanish_ci", "utf8_spanish2_ci",
                    "utf8_swedish_ci", "utf8_turkish_ci", "utf8_vietnamese_ci",
                ],
                "latin1": [
                    "latin1_swedish_ci", "latin1_german1_ci", "latin1_german2_ci", "latin1_danish_ci",
                    "latin1_bin", "latin1_general_ci", "latin1_general_cs", "latin1_spanish_ci",
                ],
                "latin2": ["latin2_general_ci", "latin2_czech_cs", "latin2_hungarian_ci", "latin2_croatian_ci", "latin2_bin"],
                "latin5": ["latin5_turkish_ci", "latin5_bin"],
                "latin7": ["latin7_general_ci", "latin7_general_cs", "latin7_estonian_cs", "latin7_bin"],
                "gbk": ["gbk_chinese_ci", "gbk_bin", "gbk_unicode_520_ci"],
                "gb18030": ["gb18030_chinese_ci", "gb18030_bin", "gb18030_unicode_520_ci"],
                "big5": ["big5_chinese_ci", "big5_bin"],
                "ascii": ["ascii_general_ci", "ascii_bin"],
                "binary": ["binary"],
                "ucs2": ["ucs2_general_ci", "ucs2_bin", "ucs2_unicode_ci", "ucs2_unicode_520_ci", "ucs2_general_mysql500_ci"],
                "utf16": ["utf16_general_ci", "utf16_bin", "utf16_unicode_ci", "utf16_unicode_520_ci"],
                "utf16le": ["utf16le_general_ci", "utf16le_bin"],
                "utf32": ["utf32_general_ci", "utf32_bin", "utf32_unicode_ci", "utf32_unicode_520_ci"],
                "cp1250": ["cp1250_general_ci", "cp1250_czech_cs", "cp1250_croatian_ci", "cp1250_polish_ci", "cp1250_bin"],
                "cp1251": ["cp1251_general_ci", "cp1251_general_cs", "cp1251_bulgarian_ci", "cp1251_ukrainian_ci", "cp1251_bin"],
                "cp1256": ["cp1256_general_ci", "cp1256_bin"],
                "cp1257": ["cp1257_general_ci", "cp1257_lithuanian_ci", "cp1257_bin"],
                "euckr": ["euckr_korean_ci", "euckr_bin"],
                "sjis": ["sjis_japanese_ci", "sjis_bin"],
                "cp932": ["cp932_japanese_ci", "cp932_bin"],
                "greek": ["greek_general_ci", "greek_bin"],
                "hebrew": ["hebrew_general_ci", "hebrew_bin"],
                "koi8r": ["koi8r_general_ci", "koi8r_bin"],
                "koi8u": ["koi8u_general_ci", "koi8u_bin"],
                "tis620": ["tis620_thai_ci", "tis620_bin"],
                "macroman": ["macroman_general_ci", "macroman_bin"],
                "armscii8": ["armscii8_general_ci", "armscii8_bin"],
                "geostd8": ["geostd8_general_ci", "geostd8_bin"],
                "keybcs2": ["keybcs2_general_ci", "keybcs2_bin"],
                "swe7": ["swe7_swedish_ci", "swe7_bin"],
            }),
        ),
        _ => (Vec::new(), json!({})),
    };
    json!({
        "ddlStyle": style,
        "quoteStyle": quote,
        "supportsColumnModify": modify,
        "supportsComment": comment,
        "supportsTableOptions": table_options,
        "supportsAutoIncrement": auto_inc,
        "supportsIndexRebuild": rebuild,
        "doris": kind == "doris",
        // 字符集 / 排序规则：MySQL 系的取值是服务端标准集合（utf8mb4 这些版本间很稳定），
        // 这里给常青名单；其它方言（没有「表级字符集」这个概念）如实给空。
        // 前端下拉带 `allow-create`：名单外的值照样能手输，
        // 所以给一份有用的名单比给空数组好 —— 空数组会让整行变成纯手输。
        // （`collations` 只需覆盖常见字符集：查不到对应项的字符集在前端会退化成输入框。）
        "charsets": charsets,
        "collations": collations,
    })
}

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
    // 结构编辑相关的能力挂在同一份 payload 上（界面一次性取，避免多一次往返）
    let edit = edit_capabilities(kind.key());
    Ok(json!({
        "type": kind.key().to_ascii_uppercase(),
        "columnTypes": column_types(kind),
        // 索引类型清单：前端「索引 → 类型」那两处下拉读的就是它（`features.indexTypes`）。
        // 原先恒为空数组 ⇒ 下拉无项、`v-if="indexTypes.length"` 的那一列**整列不显示**（白写了）。
        // 这里按方言给一份可用的小名单；下拉带 allow-create 的话，名单外的值照样能手输。
        "indexTypes": match kind.key() {
            "mysql" | "mariadb" => vec!["BTREE", "HASH", "FULLTEXT", "SPATIAL"],
            "doris" => vec!["BTREE", "INVERTED", "BITMAP", "BLOOMFILTER"],
            "postgresql" | "kingbase" => vec!["btree", "hash", "gist", "gin", "spgist", "brin"],
            "oracle" | "dm" => vec!["NORMAL", "UNIQUE", "BITMAP", "FUNCTION-BASED NORMAL"],
            "sqlserver" => vec!["CLUSTERED", "NONCLUSTERED", "UNIQUE", "COLUMNSTORE"],
            "clickhouse" => vec!["minmax", "set", "bloom_filter", "ngrambf_v1", "tokenbf_v1"],
            // SQLite 的 CREATE INDEX **没有**类型子句（只有 UNIQUE 前缀，而那是独立开关），
            // 原先给了 ["INDEX","UNIQUE"] ⇒ 下拉里两个选项选了都不进 SQL，纯误导。如实给空，
            // 前端 `v-if="indexTypes.length"` 就不会显示这一列。
            // H2 / Derby / DB2 同理（没有索引类型概念），一并落到下面的空分支。
            _ => Vec::new(),
        },
        // 存储引擎：MySQL 系的标准引擎是服务端常量清单（InnoDB 等一直是这些名字）；
        // 其它方言没有这个概念，如实给空数组。前端下拉带 `allow-create`，
        // 清单外的值照样能手输 —— 所以宁可给一份有用的小名单，也不给空数组
        // （空数组会让「存储引擎」整行退化成纯手输）。
        "engines": match kind.key() {
            "mysql" | "mariadb" | "doris" => vec![
                "InnoDB", "MyISAM", "MEMORY", "CSV", "ARCHIVE", "BLACKHOLE", "MRG_MYISAM", "FEDERATED",
            ],
            // ClickHouse：清单给的是**能直接落进 SQL 的完整写法** —— MergeTree 族（含 Replicated*
            // 副本版）必须带 `()`，Log/TinyLog/Memory/Null 这些无参引擎不能带。前端选中后原样拼
            // `ENGINE = <值>`，不再按引擎名猜着补括号（那份猜测遇上带参引擎就会补错）。
            // **Replicated 系列必须列全**：漏了的后果是编辑表结构里 `ReplicatedMergeTree` 在清单里
            // 找不到同名项，原样显示成不带括号的样子（用户实测踩到过）。
            // 这份清单与前端 `frontend/src/common/objectforms/chEngines.js` 保持一致。
            "clickhouse" => vec![
                "MergeTree()", "ReplacingMergeTree()", "SummingMergeTree()", "AggregatingMergeTree()",
                "CollapsingMergeTree()", "VersionedCollapsingMergeTree()", "GraphiteMergeTree()",
                "ReplicatedMergeTree()", "ReplicatedReplacingMergeTree()", "ReplicatedSummingMergeTree()",
                "ReplicatedAggregatingMergeTree()", "ReplicatedCollapsingMergeTree()",
                "ReplicatedVersionedCollapsingMergeTree()", "ReplicatedGraphiteMergeTree()",
                "Log", "TinyLog", "StripeLog", "Set", "Join", "Memory", "Null",
            ],
            _ => Vec::new(),
        },
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
        "ddlStyle": edit["ddlStyle"],
        "quoteStyle": edit["quoteStyle"],
        "supportsColumnModify": edit["supportsColumnModify"],
        "supportsComment": edit["supportsComment"],
        "supportsTableOptions": edit["supportsTableOptions"],
        "supportsAutoIncrement": edit["supportsAutoIncrement"],
        "supportsIndexRebuild": edit["supportsIndexRebuild"],
        "doris": edit["doris"],
        "charsets": edit["charsets"],
        "collations": edit["collations"],
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
            // 配置类错误（缺 {database} 之类的占位符）仍补一句「去填哪个框」
            "message": crate::api::error::with_hint(&err.message),
            "detail": err.detail,
        }),
        Err(join) => json!({
            "success": false,
            "message": format!("连接测试线程异常：{join}"),
        }),
    }))
}

pub async fn features(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> XResult<Json<Value>> {
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
pub async fn catalogs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> XResult<Json<Value>> {
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
            if let Ok(result) = run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await {
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
            }
            // 列不出来就当这个 catalog 没有库（而不是退回"所有库"，那会串到别的 catalog）
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
    let result = run_sql_in(state, id, database, sql, META_MAX_ROWS)
        .await
        .ok()?;
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
    let (_, dialect) = ctx_of(&state, &id).await?;
    let database = params.get("database").unwrap_or_default();
    let (_, schema_in_db) = split_db_schema(&database, &dialect);
    let schema = params.get("schema").filter(|s| !s.is_empty()).or(schema_in_db);

    // catalog 层级（Doris）：树上「库」的名字是 `catalog.库`，对象查 <catalog>.information_schema，
    // 不需要也不能切会话 —— JDBC URL 表达不了 catalog，硬塞进 database 会把 URL 弄坏。
    if dialect.catalog_level() {
        if let Some((catalog, db)) = database.split_once('.') {
            if let Meta::Sql(sql) = dialect.tables_in_catalog(catalog, db) {
                // 库名传**空**：SQL 已经用 `<catalog>.information_schema` 全限定了，
                // 不需要切库；而把 `internal.ods` 传下去会去建一条 database='internal.ods'
                // 的影子连接 —— JDBC URL 变成 `.../internal.ods`，Doris 直接拒绝（实测 500）。
                let result = run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await?;
                let tables: Vec<Value> = rows_of(&result)
                    .iter()
                    .map(|row| {
                        let kind = text_ci(row, "type");
                        json!({
                            "schema": db,
                            "name": text_ci(row, "name"),
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
    let (_, dialect) = ctx_of(&state, &id).await?;
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
                    dialect.quote(name)
                )
            })
            .collect::<Vec<_>>()
            .join(" union all ");
        let batched = run_sql_in_background(&state, &id, &database, sql, chunk.len()).await;
        let Ok(result) = batched else {
            // 批里任何一张表出错（权限 / 视图 / 表被删）都会拖垮整条语句：
            // 这一批退回到「逐表各发一条」的老路径，行为与改造前完全一致。
            for name in &chunk {
                let sql = dialect.count_sql(name);
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
    rows_of(result)
        .first()
        .and_then(|row| count_value_of_row(row))
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
    out.entry("tables".to_string()).or_insert(json!(0));
    out.entry("sizeText".to_string()).or_insert(json!("—"));
    Ok(Json(Value::Object(out)))
}

pub async fn columns(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {    let params = Params::parse(raw.as_deref());
    let table = params
        .get("table")
        .ok_or_else(|| XError::bad_request("缺少 table 参数"))?;
    let database = params.get("database").unwrap_or_default();
    let (_, dialect) = ctx_of(&state, &id).await?;
    // 「库.模式」里的**模式**那一层（PG / SQL Server / Kingbase 的树上会拼在库名后面）。
    // 不拆出来会怎样（用户反馈）：这些库取列走 JDBC `getColumns`，而它只认连接默认 schema
    // （PG=public、SQLServer=dbo）—— 别的 schema 下的表就"能列出表、字段却是空的"。
    let (_, schema) = split_db_schema(&database, &dialect);
    let target = crate::api::scope::resolve(&state, &id, &database).await?;
    let engine = state.engine();
    // 表名要进闭包（移动），而下面补类型时还要再用一次，所以先复制一份
    let queried = table.clone();
    let schema_for_call = schema.clone();
    let mut columns = blocking(move || {
        engine.list_columns_fresh_in(&target, &queried, schema_for_call.as_deref())
    })
    .await?;
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
    let map = table_comments_map(&state, &id, &database, &table).await;
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
    let mut out = std::collections::HashMap::new();
    let Ok(record) = require_record(state, conn).await else {
        return out;
    };
    let dialect = Dialect::new(record.kind());
    let Some(sql) = dialect.table_comments(table).sql() else {
        return out;
    };
    let Ok(result) = run_sql_in(state, conn, database, sql, META_MAX_ROWS).await else {
        return out;
    };
    for row in rows_of(&result) {
        let name = text_ci(&row, "column_name");
        let comment = text_ci(&row, "comment");
        if !name.is_empty() && !comment.is_empty() {
            out.insert(name.to_ascii_lowercase(), comment);
        }
    }
    out
}

/// 一张表的**表注释**（取不到给 `None`）。同 `table_comments_map`，供同步建表复用。
pub async fn table_comment_of(
    state: &AppState,
    conn: &str,
    database: &str,
    table: &str,
) -> Option<String> {
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
        let comment = row
            .get("comment")
            .and_then(Value::as_str)
            .map(str::to_string);
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
    let (_, dialect) = ctx_of(&state, &id).await?;
    let (db_only, mut schema) = split_db_schema(&database, &dialect);
    // Oracle / DM 的「库」层就是模式：不把它当模式用，就会落到下面「枚举全部模式」的分支，
    // 一次搜索 60 秒起（真机）而且结果跨模式。
    if schema.is_none() && dialect.db_is_schema() && !db_only.trim().is_empty() {
        schema = Some(db_only);
    }
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
fn escape_like(value: &str) -> String {
    value.replace('%', "\\%").replace('_', "\\_")
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
    let column = dialect.quote(column_name);

    // 空值判断不需要值
    match op {
        "is_null" => return Some(format!("{column} is null")),
        "is_not_null" => return Some(format!("{column} is not null")),
        _ => {}
    }

    let text = item.get("value").and_then(Value::as_str).unwrap_or_default();
    match op {
        "between" => {
            let low = scalar_of(dialect, kind, text)?;
            let high = scalar_of(dialect, kind, item.get("value2").and_then(Value::as_str).unwrap_or_default())?;
            Some(format!("{column} between {low} and {high}"))
        }
        "contains" | "not_contains" | "starts_with" | "ends_with" => {
            let pattern = match op {
                "starts_with" => format!("{}%", escape_like(text)),
                "ends_with" => format!("%{}", escape_like(text)),
                _ => format!("%{}%", escape_like(text)),
            };
            let keyword = if op == "not_contains" { "not like" } else { "like" };
            Some(format!("{column} {keyword} {}", dialect.literal(&pattern)))
        }
        _ => {
            let value = scalar_of(dialect, kind, text)?;
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

/// 整组高级搜索条件 → 一个 SQL 条件（各组已按行里的 且/或 拼好）。
///
/// 逐条从左往右拼并**层层加括号**：与界面上一行一行读下来的意思一致。
/// （SQL 里 AND 优先级高于 OR，不加括号时 `a AND b OR c` 会被读成 `(a AND b) OR c`，
/// 而用户按界面那样理解可能是 `a AND (b OR c)` —— 这种"看起来一样、筛出来不一样"
/// 的差异最难查，所以在源头就消掉。）
fn filter_clause(dialect: &Dialect, filters: &[Value]) -> Option<String> {
    let mut combined: Option<String> = None;
    for item in filters {
        let Some(condition) = filter_condition(dialect, item) else {
            continue;
        };
        combined = Some(match combined {
            None => format!("({condition})"),
            Some(previous) => {
                let is_or = item
                    .get("join")
                    .and_then(Value::as_str)
                    .map(|join| join.eq_ignore_ascii_case("or"))
                    .unwrap_or(false);
                let join = if is_or { "or" } else { "and" };
                format!("({previous} {join} ({condition}))")
            }
        });
    }
    combined
}

/// 排序键解析：多列 `orderColumns=age:ASC,name:DESC`（按**优先级**从左到右），
/// 老前端只传单列 `orderColumn`/`orderDir` 也认（前者非空时优先）。
///
/// 为什么在这里解析而不是直接拼字符串：方向必须是白名单（`asc`/`desc`），
/// 否则 `orderColumns=x:;drop table t` 这类就顺着 URL 进 SQL 了。
/// 列名本身交给 `dialect.quote()` 兜住，这里的过滤只是防脏值。
fn parse_order_specs(params: &Params) -> Vec<(String, String)> {
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

/// 分页浏览表数据（`/data`）。
///
/// 界面语义（分页 / 关键字模糊 / 排序）在这里翻成 SQL —— 内核只认「一条语句」。
/// `keyword` 会横跨所有文本列做 `LIKE`，列清单从内核的列元数据取（不猜类型）。
pub async fn data(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let table = params
        .get("table")
        .ok_or_else(|| XError::bad_request("缺少 table 参数"))?;
    let database = params.get("database").unwrap_or_default();
    let page: u64 = params
        .get("page")
        .and_then(|p| p.parse().ok())
        .filter(|p| *p > 0)
        .unwrap_or(1);
    let size: u64 = params
        .get("size")
        .and_then(|s| s.parse().ok())
        .filter(|s| *s > 0 && *s <= 100_000)
        .unwrap_or(50);
    let keyword = params.get("keyword").unwrap_or_default();
    // 排序键（多列，按优先级）；空表示不排序
    let order_specs = parse_order_specs(&params);
    // 高级搜索的条件（界面「添加条件」那一排）：
    // `[{"join":"AND","col":"id","op":"eq","value":"548","colType":"number"}]`
    // 这个参数以前被整个丢掉 —— 界面点了「应用筛选」却还是整张表，就是这么来的。
    let filters: Vec<Value> = params
        .get("filters")
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();

    let (_, dialect) = ctx_of(&state, &id).await?;
    let quoted = dialect.quote(&table);
    // 所有筛选条件（关键字模糊 + 高级搜索）都汇到这里，统一用 AND 串起来；
    // 同一份 where 既给数据查询、也给总条数统计 —— 否则会出现"筛出 1 行、总数说 10000"。
    let mut where_groups: Vec<String> = Vec::new();

    if !keyword.trim().is_empty() {
        let engine = state.engine();
        let name = table.clone();
        let connection = id.clone();
        let columns = blocking(move || engine.list_columns_fresh(&connection, &name)).await?;
        let text_columns: Vec<String> = columns
            .iter()
            .filter(|c| {
                let t = c
                    .type_name
                    .clone()
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                ["CHAR", "TEXT", "CLOB", "STRING", "JSON", "UUID", "ENUM", "XML", "NCHAR"]
                    .iter()
                    .any(|probe| t.contains(probe))
            })
            .map(|c| c.name.clone())
            .collect();
        if text_columns.is_empty() {
            return Err(XError::bad_request(format!(
                "表「{table}」没有可作为模糊匹配的文本列，无法按关键字筛选"
            )));
        }
        let pattern = dialect.literal(&format!("%{}%", escape_like(&keyword)));
        let conditions: Vec<String> = text_columns
            .iter()
            .map(|c| format!("{} like {pattern}", dialect.quote(c)))
            .collect();
        where_groups.push(format!("({})", conditions.join(" or ")));
    }
    // 高级搜索：条件本身由 filter_clause 负责翻译与括号；这里只把它并进 where
    if let Some(clause) = filter_clause(&dialect, &filters) {
        where_groups.push(clause);
    }

    let where_suffix = if where_groups.is_empty() {
        String::new()
    } else {
        format!(" where {}", where_groups.join(" and "))
    };
    let mut sql = format!("select * from {quoted}{where_suffix}");

    if !order_specs.is_empty() {
        // 多列：`order by "age" asc, "name" desc` —— 顺序即优先级，列名走方言 quote，方向已白名单
        let parts: Vec<String> = order_specs
            .iter()
            .map(|(col, dir)| format!("{} {dir}", dialect.quote(col)))
            .collect();
        sql.push_str(&format!(" order by {}", parts.join(", ")));
    } else if dialect.needs_order_by_for_paging() {
        // SQL Server 的 OFFSET/FETCH 必须有 ORDER BY
        sql.push_str(" order by (select null)");
    }
    sql.push_str(&format!(
        " {}",
        dialect.limit_clause(page.saturating_sub(1).saturating_mul(size), size)
    ));

    let result = match run_sql_in(&state, &id, &database, sql, size as usize).await {
        Ok(result) => result,
        Err(err) => return Ok(Json(shape::query_failure_json(&err.message, 0))),
    };
    let mut payload = shape::query_result_json(&result);

    // 真实总条数：分页控件要用它。统计失败就当「未知」（-1），不假装是当前页的行数。
    // 必须带上与数据查询**同一份 where** —— 否则筛出 1 行而总数显示 10000。
    let count_sql = format!("{}{where_suffix}", dialect.count_sql(&table));
    let total = match run_sql_in(&state, &id, &database, count_sql, 1).await {
        Ok(count_result) => rows_of(&count_result)
            .first()

            .map(|row| cell_int(row, "cnt").or_else(|| row.values().find_map(shape::value_to_i64)).unwrap_or(-1))
            .unwrap_or(-1),
        Err(_) => -1,
    };
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("totalCount".to_string(), json!(total));
        obj.insert("page".to_string(), json!(page));
        obj.insert("size".to_string(), json!(size));
    }
    Ok(Json(payload))
}

/// 一行数据 → 值字面量。
fn sql_value(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "NULL".to_string(),
        Some(Value::Bool(b)) => {
            if *b {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => format!("'{}'", s.replace('\'', "''")),
        Some(other) => format!("'{}'", other.to_string().replace('\'', "''")),
    }
}

fn keys_of(row: &Map<String, Value>) -> Vec<String> {
    row.keys().cloned().collect()
}

fn same_value(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), None) | (None, Some(Value::Null)) => true,
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// 表的就地编辑（新增 / 修改 / 删除）。
///
/// 与上游一致的取舍：**一行一条语句、顺序执行**。内核的安全闸门只允许一次执行一条，
/// 所以做不到「全部成功或全部回滚」—— 于是如实报告「执行到第几条失败”，绝不假装原子。
pub async fn data_save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let table = body
        .get("table")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if table.is_empty() {
        return Err(XError::bad_request("缺少 table"));
    }
    let (_, dialect) = ctx_of(&state, &id).await?;
    let quoted = dialect.quote(&table);
    let pk_columns: Vec<String> = body
        .get("pkColumns")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut statements: Vec<String> = Vec::new();

    let inserts = body.get("inserts").and_then(Value::as_array).cloned().unwrap_or_default();
    for row in &inserts {
        let Some(obj) = row.as_object() else { continue };
        let columns = keys_of(obj);
        if columns.is_empty() {
            continue;
        }
        let names: Vec<String> = columns.iter().map(|c| dialect.quote(c)).collect();
        let values: Vec<String> = columns
            .iter()
            .map(|c| sql_value(obj.get(c)))
            .collect();
        statements.push(format!(
            "insert into {quoted} ({}) values ({})",
            names.join(", "),
            values.join(", ")
        ));
    }

    let updates = body.get("updates").and_then(Value::as_array).cloned().unwrap_or_default();
    for item in &updates {
        let original = item.get("original").and_then(Value::as_object);
        let row = item.get("row").and_then(Value::as_object);
        let (Some(original), Some(row)) = (original, row) else { continue };
        let changed: Vec<String> = keys_of(row)
            .into_iter()
            .filter(|c| !same_value(original.get(c), row.get(c)))
            .collect();
        if changed.is_empty() {
            continue; // 改回原值等于没改，不留一条什么都没变的 UPDATE
        }
        let assignments: Vec<String> = changed
            .iter()
            .map(|c| format!("{} = {}", dialect.quote(c), sql_value(row.get(c))))
            .collect();
        let keys: Vec<String> = if pk_columns.is_empty() {
            keys_of(original)
        } else {
            pk_columns.clone()
        };
        let conditions: Vec<String> = keys
            .iter()
            .map(|c| format!("{} = {}", dialect.quote(c), sql_value(original.get(c))))
            .collect();
        statements.push(format!(
            "update {quoted} set {} where {}",
            assignments.join(", "),
            conditions.join(" and ")
        ));
    }

    let deletes = body.get("deletes").and_then(Value::as_array).cloned().unwrap_or_default();
    for row in &deletes {
        let Some(obj) = row.as_object() else { continue };
        let keys: Vec<String> = if pk_columns.is_empty() {
            keys_of(obj)
        } else {
            pk_columns.clone()
        };
        let conditions: Vec<String> = keys
            .iter()
            .map(|c| format!("{} = {}", dialect.quote(c), sql_value(obj.get(c))))
            .collect();
        statements.push(format!(
            "delete from {quoted} where {}",
            conditions.join(" and ")
        ));
    }

    // 就地编辑要落到目标库上（跨库时就是那条影子连接）
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let target = crate::api::scope::resolve(&state, &id, &database).await?;

    if statements.is_empty() {
        return Ok(Json(json!({
            "success": true,
            "columns": [], "columnTypes": [], "rows": [],
            "rowCount": 0, "totalCount": -1, "affectedRows": 0, "executeTime": 0,
            "message": "没有需要提交的变更", "notices": [], "hasMore": false,
        })));
    }

    let engine = state.engine();
    let (affected, executed, error) = blocking(move || {
        let mut affected: i64 = 0;
        let mut executed = 0usize;
        for sql in statements {
            let request = QueryRequest {
                schema: None,
                read_only: None,
                connection: target.clone(),
                sql,
                options: QueryOptions {
                    max_rows: 1,
                    timeout_ms: 120_000,
                },
                execution_id: None,
                session: Some("internal:browse".to_string()),
                // 表格「就地编辑」提交的增删改 —— 界面功能驱动的写入，不进查询历史
                // （与编辑器里手写 INSERT/UPDATE 是两回事）
                internal: true,
            };
            match engine.execute(request, AccessContext::Web) {
                Ok(result) => {
                    affected += result.affected_rows.unwrap_or(0) as i64;
                    executed += 1;
                }
                Err(err) => return Ok((affected, executed, Some(err.message))),
            }
        }
        Ok((affected, executed, None))
    })
    .await?;

    let (success, message) = match &error {
        None => (true, format!("已提交 {executed} 条语句，影响 {affected} 行")),
        // 同一口径：前端已经加了「保存失败：」这层动作前缀，这里再报「第几条失败（共几条…）」
        // 就是套娃（用户实测看到的原话是「保存失败：第 1 条失败（共 1 条，已成功 0 条）：…」）。
        // 只留原因；「已成功 N 条」只在真有语句生效时才补。
        Some(err) => {
            let reason = err.strip_prefix("SQL 执行失败：").unwrap_or(err);
            (
                false,
                if executed == 0 {
                    reason.to_string()
                } else {
                    format!("{reason}（前 {executed} 条已成功）")
                },
            )
        }
    };
    Ok(Json(json!({
        "success": success,
        "columns": [], "columnTypes": [], "rows": [],
        "rowCount": 0, "totalCount": -1,
        "affectedRows": affected,
        "executeTime": 0,
        "message": message,
        "notices": [], "hasMore": false,
    })))
}

// ------------------------------------------------------------------ 建表语句与对象

/// 建表语句 / 视图定义（界面读 `res.ddl`）。
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
            return Err(
                oracle_builtin_view_error(state, id, database, table, schema.as_deref(), dialect, err)
                    .await,
            )
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
    for line in text.lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with("--") || line.starts_with("/*") {
            continue;
        }
        return line.to_ascii_lowercase().starts_with("create");
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
    let names: Vec<String> = result
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect();
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
            return Err(
                oracle_builtin_view_error(&state, &id, &database, &name, schema.as_deref(), dialect, err)
                    .await,
            )
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
                                        Value::String(
                                            c.trim().trim_matches(['"', '`', '[', ']']).to_string(),
                                        )
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
fn is_true_flag(value: &Value) -> bool {
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
                    out.entry("comment".to_string())
                        .or_insert_with(|| Value::String(if is_super { "超级用户".into() } else { String::new() }));
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
    let hint = format!(
        "当前连接账号没有查看「{feature}」的权限。请改用有该目录读权限的账号，或让 DBA 授权后重连。"
    );
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
pub async fn roles(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> XResult<Json<Value>> {
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

// ------------------------------------------------------------------ 表操作

/// 执行结构变更（表结构 / 索引 / 表选项编辑都走它）。
pub async fn alter(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let sql = body
        .get("sql")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if sql.trim().is_empty() {
        return Err(XError::bad_request("缺少 sql"));
    }
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default();

    // **必须按语句拆分后逐条执行**。
    //
    // 界面的「编辑表结构」把整段脚本一次发过来（`scriptParts.join(';\n')`），
    // 而内核的安全规则是「一次只允许执行一条语句」—— 整段直接丢下去的话，
    // 凡是产生两句以上的改动（改字段 + 改注释、改主键的 DROP+ADD…）都会失败在
    // 「检测到 2 条，请拆分后执行」上，而这个对话框只有一个提交按钮，没法自己拆。
    let record = require_record(&state, &id).await?;
    let statements: Vec<String> = dbmind_core::split_statements(record.kind().protocol(), &sql)
        .into_iter()
        .filter(|statement| !statement.trim().is_empty())
        .collect();
    if statements.is_empty() {
        return Err(XError::bad_request("sql 里没有可执行的语句"));
    }

    let kind_key = record.kind().key();
    // 事务兜底：**DDL 可回滚**的方言（PG / SQLite / SQL Server / DB2 / Derby / H2）把整批变更包进
    // 一个事务 —— 中途失败整体 ROLLBACK，不会留下「删了一半」的结构。用户的要求正是这一条：
    // 基础信息 / 字段定义 / 索引，**修改失败都不要破坏原来的结构**。
    //
    // MySQL / Oracle / Doris / ClickHouse 的 DDL 会隐式提交（Oracle 甚至显式提交），事务对它们
    // 没有意义 —— 它们的原子性靠界面把整批变更**合并成一条 ALTER**，见
    // `dialect::ddl_transactional` 的注释。实测（MySQL）：START TRANSACTION + INSERT + ROLLBACK
    // 之后 COUNT 归 0 ⇒ 这条链路是持久会话，事务确实生效，所以这句兜底对可回滚方言是真管用的。
    let mut in_tx = crate::api::dialect::ddl_transactional(kind_key);
    if in_tx {
        // BEGIN 失败就不能再当作"在事务里"：下面若真出错，报「已回滚」就是撒谎
        // （根本没有事务，前面的改动已经落库）。这时按"没包事务"如实说「前 N 条已生效」。
        let began = run_write_sql_in(
            &state,
            &id,
            &database,
            crate::api::dialect::tx_begin(kind_key).to_string(),
            1,
            true,
        )
        .await
        .is_ok();
        if !began {
            in_tx = false;
            tracing::warn!("事务 BEGIN 失败：本批变更不做整体回滚，出错时按实际生效条数如实报告");
        }
    }

    let mut executed = 0usize;
    let mut last = Value::Null;
    for (index, statement) in statements.iter().enumerate() {
        // 表结构编辑是**写库**：走跟随连接策略的入口（只读会话会被驱动拒绝，见 run_write_sql_in）
        match run_write_sql_in(&state, &id, &database, statement.clone(), 1, true).await {
            Ok(result) => {
                executed += 1;
                last = shape::query_result_json(&result);
            }
            Err(err) => {
                // 包了事务的先整体回滚：库就跟没执行过一样（这才叫"失败不破坏原结构"）。
                // 没包事务的（MySQL/Oracle/Doris/CH）往下如实说明"前 N 条已生效"。
                if in_tx {
                    let _ = run_write_sql_in(&state, &id, &database, "ROLLBACK".to_string(), 1, true)
                        .await;
                }
                let mut payload = shape::query_failure_json(&err.message, 0);
                if let Some(object) = payload.as_object_mut() {
                    object.insert("failedSql".to_string(), json!(statement));
                    object.insert("failedIndex".to_string(), json!(index + 1));
                    object.insert("totalStatements".to_string(), json!(statements.len()));
                    // 回滚过就一条都没生效，别报一个会误导人的"已执行 N 条"
                    object.insert(
                        "executedStatements".to_string(),
                        json!(if in_tx { 0 } else { executed }),
                    );
                    // 文案只留「为什么失败」。原先这里是
                    // 「第 1/1 条执行失败：SQL 执行失败：<数据库原话>（前 0 条已生效，DDL 无法自动回滚）」
                    // —— 三层前缀 + 一句常识说明，用户看到的是一条又长又套娃的提示，
                    // 真正有用的只有中间那句数据库原话（用户实测截图就是这么弹的）。
                    //
                    // 三处收紧：
                    // ① 条数不进文案 ——「第 1/1 条」在单语句时零信息；而且 payload 里本来就有
                    //    failedIndex / totalStatements / executedStatements 三个结构化字段，
                    //    界面要展示条数直接读它们，不必写进给人看的这句话里；
                    // ② 「已生效」只在**真的执行过**前面的语句时才说（这时库已经不是原样了，
                    //    才是用户需要知道的），「DDL 无法自动回滚」是常识，删；
                    // ③ 去掉最内层的「SQL 执行失败：」前缀 —— 外面已经说了"保存失败"，
                    //    再说一遍是同一件事（前端会加动作前缀）。
                    let reason = err.message.strip_prefix("SQL 执行失败：").unwrap_or(&err.message);
                    let tip = if in_tx {
                        // 已经整体回滚：表结构保持原样，这正是用户要的结果，说一句就够
                        "（已回滚，表结构保持原样）".to_string()
                    } else if executed == 0 {
                        String::new()
                    } else {
                        format!("（前 {executed} 条已生效，无法自动回滚）")
                    };
                    object.insert("message".to_string(), json!(format!("{reason}{tip}")));
                }
                return Ok(Json(payload));
            }
        }
    }
    if in_tx {
        if let Err(err) = run_write_sql_in(&state, &id, &database, "COMMIT".to_string(), 1, true).await {
            // **绝不能吞**：COMMIT 才是真正落库的那一步 —— 延迟约束校验、序列化失败、磁盘满、
            // 锁等待超时都可能在这一刻才报错。原先这里是 `let _ =`，于是接口照样回
            // 「N 条语句全部执行成功」，而库其实**整体回滚了**：用户以为改好了，实际没改（静默假成功）。
            let _ = run_write_sql_in(&state, &id, &database, "ROLLBACK".to_string(), 1, true).await;
            let mut payload = shape::query_failure_json(&err.message, 0);
            if let Some(object) = payload.as_object_mut() {
                object.insert("failedIndex".to_string(), json!(statements.len()));
                object.insert("totalStatements".to_string(), json!(statements.len()));
                // 提交失败 = 一条都没生效，别报会误导人的「已执行 N 条」
                object.insert("executedStatements".to_string(), json!(0));
                let reason = err.message.strip_prefix("SQL 执行失败：").unwrap_or(&err.message);
                object.insert(
                    "message".to_string(),
                    json!(format!("{reason}（提交失败，已整体回滚，表结构保持原样）")),
                );
            }
            return Ok(Json(payload));
        }
    }
    if let Some(object) = last.as_object_mut() {
        object.insert("totalStatements".to_string(), json!(statements.len()));
        object.insert("executedStatements".to_string(), json!(executed));
        object.insert(
            "message".to_string(),
            json!(format!("{executed} 条语句全部执行成功")),
        );
    }
    Ok(Json(last))
}

/// 表级危险操作：清空 / 截断 / 删除 / 重命名（服务端按方言生成语句，不接受整段 SQL）。
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
    let new_name = body
        .get("newName")
        .and_then(Value::as_str)
        .map(str::to_string);
    let (_, dialect) = ctx_of(&state, &id).await?;
    let sql = match dialect.table_action(&table, &action, new_name.as_deref()) {
        Meta::Sql(sql) => sql,
        _ => {
            return Err(XError::bad_request(format!(
                "不支持的表操作：{action}（重命名需要提供 newName）"
            )))
        }
    };
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default();
    // 这里的 `1` 只限制**结果集行数**：这几条语句（delete / truncate / drop / rename）
    // 都不返回结果集，所以给多少都无所谓。
    //
    // 但**不要**指望靠它给 DML 加限制：宿主曾经把 `maxRows` 也套在 DELETE 上
    // （Microsoft 驱动用会话级的 `SET ROWCOUNT` 实现 `setMaxRows`），于是「清空表」
    // 只删掉 maxRows+1 行、接口照样回 success —— 静默少删。已在宿主侧修正：
    // 只有判定会返回结果集的语句才设上限（见 agents/dbmind-agent-jdbc 的 mayReturnRows）。
    // 清空 / 截断 / 删除 / 重命名表都是写库：走跟随连接策略的入口
    Ok(Json(match run_write_sql_in(&state, &id, database, sql, 1, true).await {
        Ok(result) => shape::query_result_json(&result),
        Err(err) => shape::query_failure_json(&err.message, 0),
    }))
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
    let preview_only = body
        .get("previewOnly")
        .and_then(Value::as_bool)
        .unwrap_or(false);
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
            "mysql" | "mariadb" | "doris" | "clickhouse" => {
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
            "mysql" | "mariadb" | "doris" => {
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
            "mysql" | "mariadb" | "doris" => {
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
        other => {
            return Err(XError::bad_request(format!(
                "不支持重命名「{other}」这种对象"
            )))
        }
    };
    if preview_only {
        return Ok(Json(json!({ "sql": sql, "native": true })));
    }
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match run_write_sql_in(&state, &id, database, sql.clone(), 1, true).await {
        Ok(_) => Ok(Json(json!({ "success": true, "message": format!("已重命名为「{new_name}」"), "sql": sql }))),
        Err(err) => Ok(Json(json!({ "success": false, "message": err.message, "sql": sql }))),
    }
}

// ------------------------------------------------------------------ 用户管理

/// `GET /api/{m}/{id}/user-info?database=&name=` —— 用户详情（`UserDetailView.vue` 用）。
///
/// 契约按界面取值定：**响应体本身就是详情对象** ——
/// `name` 进标题；`createUserSql` + `grants`（数组，每项一句授权）拼成「完整 SQL」；
/// 其余键被自动渲染成明细项。因此「没实现的方言」返回 `{error}` 而不是 HTTP 错误码
/// （界面专门认这个字段，HTTP 码只会变成一句「加载失败」）。
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
    if matches!(
        dialect.kind.key(),
        "mysql" | "mariadb" | "clickhouse" | "oracle" | "doris" | "postgresql" | "kingbase"
    ) {
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
                    let label = if mysql_style && !rh.is_empty() { format!("{rn}@{rh}") } else { rn.clone() };
                    grant_rows.push(json!({
                        "scope": "role",
                        "privilege": label,
                        "owner": rn.clone(),
                        "object": if mysql_style { rh.clone() } else { String::new() },
                    }));
                    // 文本那一份也补上（否则「原始授权」里看不到角色，只剩下结构化行）。
                    // 拼法按方言分三档 —— 曾经一律按 MySQL 写成 `TO 'u'@'h'`：
                    // MariaDB / CH / Oracle / PG 的角色没有 host，那条语句里就多出一个
                    // 凭空出现的 `@`（`GRANT 'r' TO 'u'@;`），复制出去是语法错。
                    let pg_style = matches!(dialect.kind.key(), "postgresql" | "kingbase");
                    let (role_ref, grantee) = if mysql_style {
                        (
                            format!("{}@{}", dialect.literal(&rn), dialect.literal(&rh)),
                            format!("{}@{}", dialect.literal(&account_user), dialect.literal(&account_host)),
                        )
                    } else if pg_style {
                        // PG/KB 的角色是**标识符**（`grant "r" to "u"`），单引号在这里不合法
                        (dialect.quote(&rn), dialect.quote(&account_user))
                    } else {
                        // MariaDB / ClickHouse / Oracle：账号形状是字符串，用字面量
                        (dialect.literal(&rn), dialect.literal(&account_user))
                    };
                    grants.push(json!(format!("GRANT {role_ref} TO {grantee};")));
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
                if name.is_empty() { None } else { Some(json!(name)) }
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
    let preview_only = body
        .get("previewOnly")
        .and_then(Value::as_bool)
        .unwrap_or(false);
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
    let mysql_family = matches!(dialect.kind.key(), "mysql" | "mariadb" | "doris");
    let mssql_family = dialect.kind.key() == "sqlserver";
    let oracle_family = matches!(dialect.kind.key(), "oracle");
    let clickhouse_family = dialect.kind.key() == "clickhouse";
    let pg_family = matches!(dialect.kind.key(), "postgresql" | "kingbase");
    // H2：只有表级授权这一层（没有全局/库级）
    let h2_family = dialect.kind.key() == "h2";

    // 哪些权限入参已经真正实现：MySQL 系连接的三层（全局/库级/表级）—— 由
    // `user_action_sql` 拼出 `grant`/`revoke`；其余（模式级、对象级、服务器角色、
    // ClickHouse 的授权项）仍未实现。**只有确实支持的才放行**，不支持的一律明确拒绝，
    // 而不是默默建好用户却什么都没授。
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
            let handled = (mysql_family
                && matches!(key, "privileges" | "dbPrivileges" | "tablePrivileges" | "routinePrivileges" | "revoke"))
                || (oracle_family && matches!(key, "privileges" | "tablePrivileges" | "revoke"))
                // ClickHouse：与 MySQL 系同构的三层（全局 ON *.* / 库 ON db.* / 表 ON db.table）
                // + 撤销差量，见 user_action_sql 的 clickhouse 分支。
                || (clickhouse_family
                    && matches!(key, "privileges" | "dbPrivileges" | "tablePrivileges" | "revoke"))
                // SQL Server：四层（服务器角色 / 数据库角色 / Schema 级 / 对象级）+ 撤销差量。
                // `roles`（数据库角色）本来就不在这张表里，一直可以用（真机验证过）。
                || (mssql_family
                    && matches!(key, "serverRoles" | "schemaPrivileges" | "objectPrivileges" | "revoke"))
                // PostgreSQL/KingbaseES：模式（USAGE/CREATE）/ 表（七项）/ 例程（EXECUTE）
                // + 撤销差量。「数据库权限」tab 对 PG 交的是**模式级**授权（dbPrivileges
                // 的 database 字段就是模式名），见 user_action_sql 的 pg 分支。
                || (pg_family
                    && matches!(key, "dbPrivileges" | "tablePrivileges" | "routinePrivileges" | "revoke"))
                // H2：只有表级授权（SELECT/INSERT/UPDATE/DELETE/ALL）+ 撤销差量
                || (h2_family && matches!(key, "tablePrivileges" | "revoke"));
            if !handled {
                return Err(XError::bad_request(format!(
                    "「{label}」的自动分配尚未实现：请先只做「新建用户 / 改密 / 删除用户」，\
                     再到查询页用 GRANT 授权（本次没有对数据库做任何改动）"
                )));
            }
        }
    }
    // Doris 没有例程（连 mysql.procs_priv 这张表都没有，dialect.rs 的 user_grants
    // 也排除了它）。它在 mysql_family 白名单里，若不放行这里会被照单全收、拼出
    // MySQL 的 `grant execute on procedure …` —— 在 Doris 上执行必炸。明确拒绝。
    if dialect.kind.key() == "doris"
        && options
            .get("routinePrivileges")
            .and_then(Value::as_array)
            .map(|a| !a.is_empty())
            .unwrap_or(false)
    {
        return Err(XError::bad_request(
            "Doris 没有存储过程/函数，不支持例程级授权（本次没有对数据库做任何改动）",
        ));
    }
    // 权限名会被**拼进 SQL**，所以逐个过白名单（见 `is_mysql_privilege`）
    // Oracle：系统权限那一层（`privileges`）收的是**系统权限或角色**，
    // 对象权限（`tablePrivileges`）是另一张白名单 —— 两者的校验器必须分开给。
    let global_valid: fn(&str) -> bool = if oracle_family {
        crate::api::dialect::is_oracle_grantable
    } else if clickhouse_family {
        crate::api::dialect::is_clickhouse_privilege
    } else {
        crate::api::dialect::is_mysql_privilege
    };
    let object_valid: fn(&str) -> bool = if oracle_family {
        crate::api::dialect::is_oracle_object_priv
    } else if clickhouse_family {
        crate::api::dialect::is_clickhouse_privilege
    } else if h2_family {
        crate::api::dialect::is_h2_table_priv
    } else {
        crate::api::dialect::is_mysql_privilege
    };
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
            if options.get("dbPrivileges").and_then(Value::as_array).map(|a| !a.is_empty()).unwrap_or(false) {
                return Err(XError::bad_request(
                    "Oracle 没有「数据库级」授权这一层：对象权限请写在「对象权限」里（模式.对象）",
                ));
            }
            Vec::new()
        } else if pg_family {
            // PG 的「库级」= 模式级（USAGE / CREATE），database 字段就是模式名
            privilege_db_rows(options.get("dbPrivileges"), crate::api::dialect::is_pg_schema_priv)?
        } else {
            privilege_db_rows(options.get("dbPrivileges"), global_valid)?
        },
        // PG 的**库级**（CONNECT / CREATE / TEMPORARY ON DATABASE）：与上面被借用来
        // 装模式级的 `databases` 分开，界面也是两页 —— 库是集群里的库，模式是库里的一层。
        pg_databases: if pg_family {
            privilege_db_rows(options.get("pgDbPrivileges"), crate::api::dialect::is_pg_database_priv)?
        } else {
            Vec::new()
        },
        tables: if mssql_family {
            // mssql 的对象级不再走 MySQL 的表级形状（见下方 object_grants）
            Vec::new()
        } else if pg_family {
            privilege_table_rows(options.get("tablePrivileges"), crate::api::dialect::is_pg_table_priv)?
        } else {
            privilege_table_rows(options.get("tablePrivileges"), object_valid)?
        },
        schema_grants: if mssql_family {
            mssql_schema_grant_rows(options.get("schemaPrivileges"), crate::api::dialect::is_mssql_privilege)?
        } else {
            Vec::new()
        },
        object_grants: if mssql_family {
            mssql_object_grant_rows(options.get("objectPrivileges"), crate::api::dialect::is_mssql_privilege)?
        } else {
            Vec::new()
        },
        routines: if mysql_family {
            privilege_routine_rows(options.get("routinePrivileges"), global_valid)?
        } else if pg_family {
            privilege_routine_rows(options.get("routinePrivileges"), crate::api::dialect::is_pg_routine_priv)?
        } else {
            Vec::new()
        },
    };

    // Oracle 的撤销清单：界面按"原来有、现在没有了"算好传进来。
    // 名字同样要过白名单（与授予同一个理由：会被拼进 SQL）。
    // ClickHouse 与 MySQL 的 scope 形状一致（global/db/table），同一套解析直接共用。
    // H2 的 scope 形状与 MySQL 表级一致（owner=schema、object=表），直接共用同一套解析
    let revokes: Vec<crate::api::dialect::GrantRevoke> = if mysql_family || clickhouse_family || h2_family {
        // MySQL：scope = global（全局，`on *.*`）/ db（库级，owner=库）/ table（表级，owner=库 + object=表）
        // 名字一样要过白名单 —— 会被拼进 SQL（与授予同一个理由）。
        let mut out: Vec<crate::api::dialect::GrantRevoke> = Vec::new();
        for item in options.get("revoke").and_then(Value::as_array).cloned().unwrap_or_default() {
            let scope = item.get("scope").and_then(Value::as_str).unwrap_or("global").trim().to_lowercase();
            let privilege = item.get("privilege").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if privilege.is_empty() {
                continue;
            }
            let owner = item.get("owner").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let object = item.get("object").and_then(Value::as_str).unwrap_or("").trim().to_string();
            // 例程级撤销：kind = PROCEDURE / FUNCTION（要拼进 `on procedure ...`，必须校验）
            let kind = item.get("kind").and_then(Value::as_str).unwrap_or("").trim().to_ascii_uppercase();
            let ok = match scope.as_str() {
                // 库级 / 表级走的都是对象权限白名单（与 dbPrivileges / tablePrivileges 同一个校验器）
                "db" => object_valid(&privilege) && !owner.is_empty(),
                "table" | "object" | "schema" => object_valid(&privilege) && !owner.is_empty() && !object.is_empty(),
                "routine" => {
                    object_valid(&privilege)
                        && !owner.is_empty()
                        && !object.is_empty()
                        && matches!(kind.as_str(), "PROCEDURE" | "FUNCTION")
                }
                // 角色（MySQL 8 / MariaDB）是 `名字@host` 的账号形状，不是权限名：
                // 只做字符集校验（会拼进 `revoke 'r'@'h' from …`，由 literal 转义）
                "role" => is_role_ref(&privilege),
                "global" | "server-role" => global_valid(&privilege),
                _ => false,
            };
            if !ok {
                return Err(XError::bad_request(format!(
                    "「{privilege}」不在可撤销的白名单里（或缺少库/表名）：已拒绝执行"
                )));
            }
            out.push(crate::api::dialect::GrantRevoke { scope, privilege, owner, object, db: String::new(), kind });
        }
        out
    } else if mssql_family {
        // SQL Server：scope = serverrole（服务器角色，privilege=角色名）/ role（数据库角色，同上）
        // / schema（架构级，owner=架构）/ object（对象级，owner=架构 + object=对象）
        let mut out: Vec<crate::api::dialect::GrantRevoke> = Vec::new();
        for item in options.get("revoke").and_then(Value::as_array).cloned().unwrap_or_default() {
            let scope = item.get("scope").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
            let privilege = item.get("privilege").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if privilege.is_empty() {
                continue;
            }
            let owner = item.get("owner").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let object = item.get("object").and_then(Value::as_str).unwrap_or("").trim().to_string();
            // 行级选库：schema/object 级撤销可带目标库（空 = 连接当前目录，旧行为）
            let db = item.get("database").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let ok = match scope.as_str() {
                // 角色名是标识符、不是权限名 —— 两者都会拼进 SQL，都不许乱来。
                // 'server-role'（带连字符）是 revokePayload 里的形状，兼容一下，
                // 省得两处口径漂移又出一个"不在白名单"
                "serverrole" | "server-role" | "role" => crate::api::dialect::is_plain_ident(&privilege),
                "schema" => crate::api::dialect::is_mssql_privilege(&privilege) && !owner.is_empty(),
                "object" => {
                    crate::api::dialect::is_mssql_privilege(&privilege)
                        && !owner.is_empty()
                        && !object.is_empty()
                }
                _ => false,
            };
            if !ok || (!db.is_empty() && !crate::api::dialect::is_plain_ident(&db)) {
                return Err(XError::bad_request(format!(
                    "「{privilege}」不在可撤销的白名单里（或缺少架构/对象名）：已拒绝执行"
                )));
            }
            // 兼容 revokePayload 的 'server-role' 写法：统一成 SQL 构造器认的 'serverrole'
            let scope = if scope == "server-role" { "serverrole".to_string() } else { scope };
            out.push(crate::api::dialect::GrantRevoke { scope, privilege, owner, object, db, kind: String::new() });
        }
        out
    } else if oracle_family {
        let mut out: Vec<crate::api::dialect::GrantRevoke> = Vec::new();
        for item in options.get("revoke").and_then(Value::as_array).cloned().unwrap_or_default() {
            let scope = item.get("scope").and_then(Value::as_str).unwrap_or("global").trim().to_lowercase();
            let privilege = item.get("privilege").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if privilege.is_empty() {
                continue;
            }
            let owner = item.get("owner").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let object = item.get("object").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let ok = match scope.as_str() {
                "object" => crate::api::dialect::is_oracle_object_priv(&privilege)
                    && !owner.is_empty() && !object.is_empty(),
                // 预置角色之外还允许**自定义角色**（「新建角色」建出来的那些）：
                // 名字同样只放行常规字符（会拼进 `revoke <角色> from …`）。
                "role" => crate::api::dialect::is_oracle_role(&privilege)
                    || crate::api::dialect::is_plain_ident(&privilege),
                "global" => crate::api::dialect::is_oracle_system_priv(&privilege),
                _ => false,
            };
            if !ok {
                return Err(XError::bad_request(format!(
                    "「{privilege}」不在可撤销的白名单里（或缺少模式/对象名）：已拒绝执行"
                )));
            }
            out.push(crate::api::dialect::GrantRevoke { scope, privilege, owner, object, db: String::new(), kind: String::new() });
        }
        out
    } else if pg_family {
        // PostgreSQL/KingbaseES：scope = schema（owner=模式）/ table（owner=模式 + object=表）
        // / routine|function（owner=模式 + object=例程 + kind=PROCEDURE|FUNCTION）
        let mut out: Vec<crate::api::dialect::GrantRevoke> = Vec::new();
        for item in options.get("revoke").and_then(Value::as_array).cloned().unwrap_or_default() {
            let scope = item.get("scope").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
            let privilege = item.get("privilege").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if privilege.is_empty() {
                continue;
            }
            let owner = item.get("owner").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let object = item.get("object").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let kind = item.get("kind").and_then(Value::as_str).unwrap_or("").trim().to_ascii_uppercase();
            let ok = match scope.as_str() {
                "schema" => crate::api::dialect::is_pg_schema_priv(&privilege) && !owner.is_empty(),
                // PG 库级：撤销 `revoke connect on database "db" from "u"`（owner = 库名）
                "database" => crate::api::dialect::is_pg_database_priv(&privilege) && !owner.is_empty(),
                "table" => {
                    crate::api::dialect::is_pg_table_priv(&privilege)
                        && !owner.is_empty()
                        && !object.is_empty()
                }
                // 角色成员关系（取消勾选一个角色 → `revoke "r" from "u"`）：
                // 这里是**角色名**不是权限名，只放行常规字符（会拼进 SQL）
                "role" => crate::api::dialect::is_plain_ident(&privilege),
                "routine" | "function" => {
                    crate::api::dialect::is_pg_routine_priv(&privilege)
                        && !owner.is_empty()
                        && !object.is_empty()
                        && matches!(kind.as_str(), "PROCEDURE" | "FUNCTION")
                }
                _ => false,
            };
            if !ok {
                return Err(XError::bad_request(format!(
                    "「{privilege}」不在可撤销的白名单里（或缺少模式/对象名）：已拒绝执行"
                )));
            }
            out.push(crate::api::dialect::GrantRevoke { scope, privilege, owner, object, db: String::new(), kind });
        }
        out
    } else {
        Vec::new()
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
                && r.trim().chars().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '%' | '@' | '$')
                });
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
            )))
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
            let tip = if done == 0 { String::new() } else { format!("（前 {done} 条已生效）") };
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
    if dialect.kind.key() == "clickhouse"
        || !row.keys().any(|k| k.eq_ignore_ascii_case("scope"))
    {
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
    // Oracle 的三层：系统权限 / 角色都不带 ON，对象权限带 `ON "模式"."对象"`，
    // 账号也不写成 `'u'@'h'`（Oracle 没有 host 这一维）。
    if dialect.kind.key() == "oracle" {
        return match scope.as_str() {
            "object" if !owner.is_empty() && !object.is_empty() => Some(format!(
                "GRANT {privilege} ON \"{owner}\".\"{object}\" TO \"{account_user}\";"
            )),
            "role" | "global" => Some(format!("GRANT {privilege} TO \"{account_user}\";")),
            _ => None,
        };
    }
    // PostgreSQL/KingbaseES：模式 / 表 / 例程三层（read-back 的 scope 就是 schema/table/function）。
    // 账号没有 host 这一维；例程带 `ON FUNCTION "模式"."名字"`（签名可省略，歧义由数据库报错）。
    if matches!(dialect.kind.key(), "postgresql" | "kingbase") {
        let who = dialect.quote(account_user);
        return match scope.as_str() {
            // 库级（CONNECT / CREATE / TEMPORARY ON DATABASE）：owner 列是库名
            "database" if !owner.is_empty() => Some(format!(
                "GRANT {privilege} ON DATABASE {} TO {who};",
                dialect.quote(&owner)
            )),
            "schema" if !owner.is_empty() => {
                Some(format!("GRANT {privilege} ON SCHEMA {} TO {who};", dialect.quote(&owner)))
            }
            "table" if !owner.is_empty() && !object.is_empty() => Some(format!(
                "GRANT {privilege} ON TABLE {}.{} TO {who};",
                dialect.quote(&owner),
                dialect.quote(&object)
            )),
            "function" | "procedure" if !owner.is_empty() && !object.is_empty() => {
                let object_type = if scope == "procedure" { "PROCEDURE" } else { "FUNCTION" };
                Some(format!(
                    "GRANT {privilege} ON {object_type} {}.{} TO {who};",
                    dialect.quote(&owner),
                    dialect.quote(&object)
                ))
            }
            _ => None,
        };
    }
    // H2：只有表级一层（schema.table），账号也没有 host 这一维 —— 落到下面 MySQL 那条
    // 通用分支会拼成 `TO '""'@''`（实测），既不是合法 H2 语法，也没法照着执行。
    // 空用户名（H2 默认用户）统一写成空标识符 `""`。
    if dialect.kind.key() == "h2" {
        let raw = account_user.trim();
        let who = if raw.is_empty() || raw == "\"\"" || raw == "(default)" {
            "\"\"".to_string()
        } else {
            dialect.quote(raw)
        };
        return match scope.as_str() {
            "table" if !owner.is_empty() && !object.is_empty() => Some(format!(
                "GRANT {privilege} ON {}.{} TO {who};",
                dialect.quote(&owner),
                dialect.quote(&object)
            )),
            "global" => Some(format!("GRANT {privilege} TO {who};")),
            _ => None,
        };
    }
    let target = match scope.as_str() {
        "global" => "*.*".to_string(),
        "schema" if !owner.is_empty() => format!("{}.*", dialect.quote(&owner)),
        "table" if !owner.is_empty() && !object.is_empty() => {
            format!("{}.{}", dialect.quote(&owner), dialect.quote(&object))
        }
        // 例程级：object 列是 `类型:名字`（user_grants 里 concat 出来的），还原成
        // `ON PROCEDURE `db`.`p`` —— 类型字是 MySQL 语法的一部分，必须带上
        "routine" if !owner.is_empty() && !object.is_empty() => {
            let (kind, routine) = object.split_once(':').unwrap_or(("PROCEDURE", &object));
            let routine_type = if kind.eq_ignore_ascii_case("function") { "function" } else { "procedure" };
            format!("{routine_type} {}.{}", dialect.quote(&owner), dialect.quote(routine))
        }
        // 角色（MySQL 8 / MariaDB）：角色本身就是账号形状，语句里没有 ON 子句
        "role" if !owner.is_empty() => {
            let role_host = if object.is_empty() { "%" } else { object.as_str() };
            return Some(format!(
                "GRANT {}@{} TO {}@{};",
                dialect.literal(&owner),
                dialect.literal(role_host),
                dialect.literal(account_user),
                dialect.literal(account_host)
            ));
        }
        _ => return None,
    };
    Some(format!(
        "GRANT {privilege} ON {target} TO {}@{};",
        dialect.literal(account_user),
        dialect.literal(account_host)
    ))
}

/// MySQL 的角色引用（`名字@host`）：只放行常规字符，`%` 与 `.` 是 host 的通配/域名写法。
fn is_role_ref(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '%' | '@' | '$'))
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
fn privilege_table_rows(value: Option<&Value>, valid: fn(&str) -> bool) -> XResult<Vec<(String, String, Vec<String>)>> {
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
        let database = row.get("database").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let routine = row.get("routine").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let kind = row.get("kind").and_then(Value::as_str).unwrap_or("PROCEDURE").trim().to_ascii_uppercase();
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
        let db = row.get("database").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let schema = row.get("schema").and_then(Value::as_str).unwrap_or("").trim().to_string();
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
        let db = row.get("database").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let schema = row.get("schema").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let object = row.get("object").and_then(Value::as_str).unwrap_or("").trim().to_string();
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
        out.push((db, schema, object, checked_privileges(row.get("privileges"), valid)?));
    }
    Ok(out)
}


#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(grant_error_hint("Table 'x' doesn't exist"), "Table 'x' doesn't exist");
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
