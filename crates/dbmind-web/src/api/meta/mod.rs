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

pub(crate) use axum::extract::{Path, RawQuery, State};
pub(crate) use axum::http::StatusCode;
pub(crate) use axum::Json;
pub(crate) use dbmind_core::{
    AccessContext, CellValue, ColumnDetail, ConnectionRecord, DbMindError, ErrorCode, QueryOptions,
    QueryRequest, QueryResult,
};
pub(crate) use serde_json::{json, Map, Value};

pub(crate) use crate::api::dialect::{Dialect, Meta};
pub(crate) use crate::api::error::{XError, XResult};
pub(crate) use crate::api::{blocking, conn as conn_api, require_record, shape, Params};
pub(crate) use crate::AppState;

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
pub(crate) const CONNECT_RETRY_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

// ------------------------------------------------------------------ 基础工具

pub(crate) async fn ctx_of(state: &AppState, id: &str) -> XResult<(ConnectionRecord, Dialect)> {
    let record = require_record(state, id).await?;
    let dialect = Dialect::new(record.kind());
    Ok((record, dialect))
}

/// 执行一条语句（内部用）。默认行数给足，免得「看一张大表」被截断。
// 域子模块：门面只留 HTTP 编排与公共工具，拼 SQL 的细节见各文件。
// 子文件经 `use super::*` 继承这里的 use 项，所以原 imports 一律 pub(crate) use 上收。
mod browse;
mod data;
mod objects;
mod users;

// handler 与域内工具全部平铺回 meta:: 下（api/mod.rs 的路由 meta::xxx 不用改）
pub(crate) use browse::*;
pub(crate) use data::*;
pub(crate) use objects::*;
pub(crate) use users::*;

// nosql.rs 有 `pub use meta::test` 的对外再导出：glob 的 pub(crate) use 可见性只有 crate 内，
// 显式 pub use 才能保持 test 的对外可见性。
pub use browse::test;

pub async fn run_sql(state: &AppState, conn: &str, sql: String, max_rows: usize) -> XResult<QueryResult> {
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
    let base = require_record(state, conn).await?;
    let dialect = Dialect::new(base.kind());
    let sql = if dialect.wants_use_switch() {
        let (db, _) = split_db_schema(database, &dialect);
        if db.is_empty() {
            sql
        } else {
            format!("use {}; {}", dialect.quote(&db), sql)
        }
    } else {
        sql
    };
    // SQL Server：这条入口（表数据 / 行数 / 导出 / 行编辑…）拼出来的 SQL 里表名都不带模式，
    // 而它没有会话级「默认模式」开关 —— 不补前缀就一律按 dbo 解析，`test` 模式下的表直接
    // 报 `Invalid object name`（用户实测：双击 test 模式里的表加载失败）。
    // 与编辑器同一条改写规则（见 `mssql_scope`），其它方言、没选模式时原样返回。
    let sql = crate::api::mssql_scope::apply(state, conn, database, dialect, sql).await;
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
    run_sql_in_inner(
        state,
        conn,
        database,
        sql,
        max_rows,
        Some("internal:rowcount"),
        false,
    )
    .await
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
    // 「库.模式」里的模式层要跟着一起下到驱动：SQL Server 改表/建表不写模式会落到默认模式
    // （dbo），PG 同理（写路径不是编辑器那条会话，search_path 未必在目标模式上）。
    let schema = {
        let (_, dialect) = ctx_of(state, conn).await?;
        split_db_schema(database, &dialect).1
    };
    let started = std::time::Instant::now();
    let outcome = run_write_sql_target(state, &target, sql.clone(), max_rows, schema).await;
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
    schema: Option<String>,
) -> XResult<QueryResult> {
    let engine = state.engine();
    let request = QueryRequest {
        schema,
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
pub(crate) async fn run_meta_sql(
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
            if db.is_empty() {
                sql
            } else {
                format!("use {}; {}", dialect.quote(&db), sql)
            }
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
///
/// `pub(crate)`：AI 巡检要一次拿到全库索引（见 `ai/tools.rs` 的 patrol_scan），
/// 复用它才能保证「界面上看到的索引」和「巡检时的判断依据」是同一份数据。
pub(crate) async fn meta_list(
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

/// 「库[.模式]」串里的**模式名**，给 [`Dialect::table_ref`] / [`Dialect::count_sql_in`] 用。
///
/// 与 [`split_db_schema`] 的差别只有一条：`db_is_schema` 的方言（Oracle / DM）树上第一层
/// 就是模式，**不带点**也可能是模式（达梦的 `database=DEMO`）。
/// 但"不带点"也可能是连接自己的服务名 / 库名（Oracle 的 `FREE`）—— 那种情况表示"没选模式"，
/// 必须给 `None`，否则会去找一个叫 FREE 的模式（表都查不到）。
/// 判据：整串与连接配置里的 database 相同 ⇒ 是连接固有目标，不是模式。
pub(crate) fn scope_schema_of(
    database: &str,
    dialect: &Dialect,
    own_db: Option<&str>,
) -> Option<String> {
    let (db_only, schema) = split_db_schema(database, dialect);
    if schema.is_some() {
        return schema;
    }
    if !dialect.db_is_schema() {
        return None;
    }
    let bare = db_only.trim();
    if bare.is_empty() {
        return None;
    }
    let own = own_db.map(str::trim).unwrap_or_default();
    if !own.is_empty() && bare.eq_ignore_ascii_case(own) {
        return None;
    }
    Some(bare.to_string())
}

// ------------------------------------------------------------------ 特性与结构清单

/// 各类型的**候选类型清单**（结构编辑器 / 新建表的类型下拉）。
/// 统一口径：**不带死参数** —— 长度 / 精度由编辑器的「长度 / 精度」列给
///（回显已有列时按真实类型拆进两列；新增行对 char 系预填 255）。
/// 特例保留参数：`(max)` 这类编辑器表达不了的形态（SQL Server 的 NVARCHAR(max)）。
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
    let database = body.get("database").and_then(Value::as_str).unwrap_or_default();

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
                    let _ = run_write_sql_in(&state, &id, &database, "ROLLBACK".to_string(), 1, true).await;
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

#[cfg(test)]
mod scope_tests {
    use super::*;
    use dbmind_core::ConnectionKind;

    // 达梦真机（2026-11-18）踩出来的两类问题，各留两条断言防回归：
    // ① 树上「库」就是模式，裸模式名（`DEMO`）必须能当模式用；
    // ② 表引用必须补模式前缀，否则落回登录用户的默认模式（`无效的表或视图名[USERS]`）。
    #[test]
    fn 裸模式名只对_db_is_schema_方言生效() {
        let dm = Dialect::new(ConnectionKind::Dm);
        let oracle = Dialect::new(ConnectionKind::Oracle);
        let mysql = Dialect::new(ConnectionKind::Mysql);
        // 达梦：连接的库名本来就是空的 ⇒ 裸值一律当模式
        assert_eq!(scope_schema_of("DEMO", &dm, None).as_deref(), Some("DEMO"));
        // Oracle：裸值等于连接自己的服务名 ⇒ 表示「没选模式」，不能当模式用
        assert_eq!(scope_schema_of("FREE", &oracle, Some("FREE")), None);
        // 带点的一律取后半段（Oracle 的 服务名.模式）
        assert_eq!(
            scope_schema_of("FREE.SYS", &oracle, Some("FREE")).as_deref(),
            Some("SYS")
        );
        // 其它方言不认模式层
        assert_eq!(scope_schema_of("demo", &mysql, None), None);
    }

    #[test]
    fn 表引用对_db_is_schema_方言补模式前缀() {
        let dm = Dialect::new(ConnectionKind::Dm);
        let oracle = Dialect::new(ConnectionKind::Oracle);
        let mysql = Dialect::new(ConnectionKind::Mysql);
        assert_eq!(dm.table_ref(Some("DEMO"), "USERS"), "\"DEMO\".\"USERS\"");
        // 没有模式时保持原样（按会话默认模式解析）
        assert_eq!(dm.table_ref(None, "USERS"), "\"USERS\"");
        // 已经是限定名的表名不重复加前缀
        assert_eq!(dm.table_ref(Some("DEMO"), "OTHER.T"), "\"OTHER\".\"T\"");
        // 其余方言不加前缀：目标库由影子连接 / `use [库]` / search_path 决定
        assert_eq!(mysql.table_ref(Some("demo"), "t"), "`t`");
        assert_eq!(oracle.table_ref(None, "USERS"), "\"USERS\"");
    }
}
