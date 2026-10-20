//! `/api/sync/*` —— 数据同步（把源对象的**结构**与**数据**搬到目标）。
//!
//! ## 两条入口
//!
//! | 入口 | 场景 | 形态 |
//! |---|---|---|
//! | `POST /api/sync` | 单表同步 | 同步返回 `{inserted, updated, skipped, errors, …}` |
//! | `POST /api/sync/db` | 多对象同步 | 异步任务（进度 + 逐对象结果 + 可取消） |
//!
//! ## 与 `transfer` 模块的分工
//!
//! 本文件只管**结构与编排**：解析请求、建表/索引/注释、同实例直拷、视图/例程同步、
//! 逐对象结果。真正的**数据搬运**在 [`crate::api::transfer`]——那里是 DataX 风格的
//! Reader/Writer 插件（按连接类型取实现）+ 多通道流水线（分片并发读 → 有界通道 →
//! 多连接并发写）。想给某种库换取数或落库方式，只改 `transfer::reader_for` /
//! `writer_for`，这个文件一行都不用动。
//!
//! ## 四个决定
//!
//! 1. **逐对象给结果，不吞错**。整库同步里某张表失败很正常（没权限、类型不支持、
//!    主键冲突），把它记进 `results[]` 里继续做下一张 —— 但**绝不算成成功**。
//!    界面上一眼能看到「哪张表失败、为什么」。
//! 2. **`upsert` 用「先分后写」**：先把目标表已有的主键读进来，源行分成「新增」与
//!    「更新」两拨，再批量插入 + 逐行更新。比起「一条一条先 UPDATE 再 INSERT」省一半往返，
//!    而且不依赖任何方言特有的 upsert 语法（`ON CONFLICT` / `ON DUPLICATE KEY` 各家不同）。
//! 3. **没有主键就不做 upsert**：只能退化成「仅插入」并**明确写在结果里**。
//!    没有键的「更新」只能靠全列匹配，那会改错行 —— 宁可不做。
//! 4. **不支持的同步能力要列出来**（视图/函数/过程/触发器/事件）。
//!    它们会出现在结果里，状态是 `skipped` + 一句原因，而不是消失不见了。
//! 5. **按页流式，不设总行数上限**：读一页（`PAGE` 行）就写一页。
//!    早先是「整表读进内存 → 再写」，所以不得不留个 20 万行的上限 ——
//!    而大表同步到一半被截断，界面上只是一句轻描淡写的注记，比慢一点糟糕得多。
//!    流式之后：内存占用只与一页有关、第一页写完即可见、取消在页边界就能生效，
//!    上限自然也就没有存在的理由了。

use axum::extract::{Path, State};
use axum::Json;
use dbmind_core::{CellValue, ColumnDetail, TableKind};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::export::sql_literal;
use crate::api::meta::{run_sql_in, run_sql_in_keyed};
use crate::api::{blocking, require_record};
use crate::AppState;

/// 拉数据时的翻页大小 —— **同步没有总行数上限**（见文件头决定 5）。
///
/// 原来这里有个 `MAX_ROWS = 200_000`：因为旧实现是「先把整表读进内存，再开始写」，
/// 不留个上限内存会被撑爆。现在改成**按页流式**（读一页写一页），
/// 内存占用只与这一页有关、与表大小无关，上限就没有存在理由了：
/// 大表同步到 20 万行处被静默截断，比慢一点糟糕得多。
/// 1 万的来历是最初「整表进内存」时代的保守值。现在按页流式后，
/// 页大小只影响内存峰值与往返次数：主键 keyset 分页每页只扫本页，
/// 调大不会引入深分页退化。5 万 ≈ 每页往返次数降到原来的 1/5，
/// 千万级大表的同步吞吐明显提升；内存峰值（单页行数据）在典型行宽下几十 MB，可控。
/// 写侧另有 MAX_TUPLES_PER_STATEMENT = 2000 独立控制单语句大小，两者互不影响。
const PAGE: u64 = 50_000;
/// 一条语句最多拼多少个值元组。
///
/// 500 → 2000 → 5000 的演进依据：每条语句有 ~174ms 的固定链路开销
///（权限/审计/agent RPC，见 run_sql），批越小这条税交得越勤。
/// 5000 行 ≈ 500KB 一条，仍在 MySQL `max_allowed_packet` 默认值内
///（旧版 4MB、新版 64MB）；再往上单语句解析/回滚成本开始吃掉收益，先停在这。
const MAX_TUPLES_PER_STATEMENT: u64 = 5000;
/// Stream Load 单请求的**字节**上限：行数上限（5 万）管不住行宽，宽行表一批能拼出
/// 85MB+ 的请求体。实测这台单 BE（HDD）每次 Stream Load 有 **~10 秒固定开销**
///（事务/publish 延迟），但 48MB 大批的 publish 代价陡增（实测反而比 16MB 慢近一倍）
/// —— 16MB/批 ≈ 14s/批，是实测甜点：进度每十几秒跳一格，吞吐最优。
pub(crate) const STREAM_LOAD_MAX_BYTES: usize = 16 * 1024 * 1024;

/// 流水线**内部交接**的截止时间（读取侧发页 / 写入侧取批 / 等协程收尾 / 等并发槽位）。
///
/// 为什么要有它：数据库调用自己是有硬截止时间的（内核发给宿主的是 `timeoutMs + 2s`，
/// 超时即 `QueryTimeout`，取消还会经 `Statement.cancel()` 打到驱动），可**流水线内部的
/// 交接**原先一个截止时间都没有 —— `send().await` / `acquire().await` / `handle.await`
/// 只要对端不再响应，就会一直挂着：任务停在 running、界面显示「任务仍在后台收尾」、
/// 而进程侧 0% CPU、一条数据库连接都没有（真机挂过一次，21 分钟）。
///
/// 取值：比单次数据库调用的上限（直拷路径 600s）再宽一点 —— 正常的慢只会慢在**调用**上，
/// 不会慢在"没人来取我刚读到的这一页"上；这里等不到人就是真卡住了。
const PIPE_WAIT: std::time::Duration = std::time::Duration::from_secs(660);

// ------------------------------------------------------------------ 请求

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncRequest {
    #[serde(default)]
    source_connection_id: String,
    #[serde(default)]
    target_connection_id: Option<String>,
    #[serde(default)]
    source_database: Option<String>,
    #[serde(default)]
    source_schema: Option<String>,
    #[serde(default)]
    source_table: Option<String>,
    #[serde(default)]
    target_database: Option<String>,
    #[serde(default)]
    target_schema: Option<String>,
    #[serde(default)]
    target_table: Option<String>,
    /// 单表接口里可能是 `"true"` 字符串，整库接口里是布尔 —— 两种都认（见 `flag`）
    #[serde(default)]
    sync_structure: Option<Value>,
    #[serde(default)]
    sync_data: Option<Value>,
    /// 单表接口的写入模式
    #[serde(default)]
    mode: Option<String>,
    /// 整库接口的写入模式
    #[serde(default)]
    data_mode: Option<String>,
    #[serde(default)]
    object_policy: Option<String>,
    /// 遇错停止：默认 false = 跳过出错对象继续（每对象独立记账）；true = 第一个错就停
    #[serde(default)]
    stop_on_error: Option<bool>,
    /// 建表时把源表的索引一起建到目标（默认开）
    #[serde(default)]
    include_indexes: Option<bool>,
    /// 目标库不存在时自动创建（默认开）
    #[serde(default)]
    auto_create_db: Option<bool>,
    /// 行数上限：>0 时每张表只搬前 N 行（抽样/试跑用）；0 或缺省 = 不限
    #[serde(default)]
    row_limit: Option<u64>,
    /// 行过滤条件：直接拼进源侧 SELECT 的 WHERE（用户自己保证语义，`;` 会被剥掉防多语句）
    #[serde(default)]
    where_clause: Option<String>,
    // batch_size 已删除：写入批次由后端按目标方言自动决定（见 sync_table 的 per_statement），
    // 每种库的甜点值差一个数量级，界面让用户猜没有意义。旧客户端传了也被 serde 忽略。
    #[serde(default)]
    tables: Vec<String>,
    #[serde(default)]
    views: Vec<String>,
    #[serde(default)]
    functions: Vec<String>,
    #[serde(default)]
    procedures: Vec<String>,
    #[serde(default)]
    triggers: Vec<String>,
    #[serde(default)]
    events: Vec<String>,
}

/// 布尔字段的宽容解析：前端的 `syncStructure` 在单表接口里是字符串、整库接口里是布尔。
fn flag(value: &Option<Value>, fallback: bool) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => matches!(text.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        Some(Value::Null) | None => fallback,
        Some(other) => other.as_bool().unwrap_or(fallback),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Policy {
    /// 目标已存在 ⇒ 保留结构，只同步数据
    Merge,
    /// 目标已存在 ⇒ 先删再按源结构重建
    Drop,
}

#[derive(Clone, Copy, PartialEq)]
enum DataMode {
    Upsert,
    Insert,
    TruncateInsert,
}

#[derive(Clone)]
struct Opts {
    policy: Policy,
    data_mode: DataMode,
    sync_structure: bool,
    sync_data: bool,
    stop_on_error: bool,
    include_indexes: bool,
    auto_create_db: bool,
    row_limit: u64,
    where_clause: String,
}

impl SyncRequest {
    fn target_connection(&self) -> String {
        self.target_connection_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.source_connection_id.clone())
    }

    fn opts(&self) -> Opts {
        let policy = match self.object_policy.as_deref().unwrap_or("merge").trim() {
            "drop" => Policy::Drop,
            // `skip` 在上游的老版本里表示「不动已有对象」，语义上等同于「保留结构只同步数据」
            _ => Policy::Merge,
        };
        let raw_mode = self
            .data_mode
            .clone()
            .or_else(|| self.mode.clone())
            .unwrap_or_else(|| "upsert".to_string());
        let data_mode = match raw_mode.trim() {
            "insert" => DataMode::Insert,
            "truncate_insert" => DataMode::TruncateInsert,
            _ => DataMode::Upsert,
        };
        Opts {
            policy,
            data_mode,
            sync_structure: flag(&self.sync_structure, true),
            sync_data: flag(&self.sync_data, true),
            stop_on_error: self.stop_on_error.unwrap_or(false),
            include_indexes: self.include_indexes.unwrap_or(true),
            auto_create_db: self.auto_create_db.unwrap_or(true),
            row_limit: self.row_limit.unwrap_or(0),
            // WHERE 由用户手写：剥掉分号防多语句（注解 -- 与 /* */ 由执行器按方言处理，
            // 这里只做最小防护）；空白条件视为未设置
            where_clause: self
                .where_clause
                .as_deref()
                .unwrap_or("")
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string(),
        }
    }
}

/// 一侧的连接信息。
#[derive(Clone)]
struct Side {
    conn: String,
    scope: String,
    schema: Option<String>,
    /// catalog 方言（Doris）：表名必须带 `catalog.库` 全限定（连接级库不可用）
    catalog: bool,
}

impl Side {
    /// 表名带上这一侧的**库上下文**：优先显式 schema（SQL Server 的 dbo 等），
    /// 否则用 scope（Doris 的 `internal.xms` 这类全限定库）。
    ///
    /// 为什么必须：Doris 的连接级库名不可用（URL 带 `internal.xms`、带裸 `xms` 都握手失败，
    /// 见 scope::resolve 的 catalog 分支），语句里的表名必须自己全限定
    /// （`internal.xms.sales`，Doris 认三段式）。MySQL/SQL Server 等类型的
    /// `db.table` / `db.schema.table` 全限定天然合法，行为不变。
    /// 同 [`Side::scoped`]，但**不加引号** —— 拼出来的名字是喂给 JDBC `getColumns` 这类
    /// **元数据 API** 的，那边要裸的 schema/table，加了引号反而找不到表（同 compare.rs 的 qualified_meta）。
    fn scoped_meta(&self, table: &str) -> String {
        let prefix = if self.catalog {
            self.scope.trim().to_string()
        } else {
            self.schema
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_default()
        };
        if prefix.is_empty() || table.contains('.') {
            table.to_string()
        } else {
            format!("{prefix}.{table}")
        }
    }
    fn scoped(&self, table: &str, dialect: Dialect) -> String {
        // 前缀规则（真机踩坑总结）：
        // - Doris（catalog 方言）：必须带 `catalog.库` 全限定 —— 它的连接级库不可用；
        // - **schema 方言（SQL Server/PG）绝不带库名**：影子连接已切到目标库，
        //   `库.表` 两段名在 SQL Server 里被解析成 `schema.表` 而报「对象不存在」；
        //   用户显式选了模式（dbo 等）才带模式前缀；
        // - 其余（MySQL 系）：影子已切库，裸名即可。
        let prefix = if self.catalog {
            self.scope.trim().to_string()
        } else {
            self.schema
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_default()
        };
        // 标识符按方言加引号：保留字 / 含特殊字符的表名裸拼会直接语法错
        //（目标侧本来就走 dialect.quote，这里补上，两侧口径一致）。
        if prefix.is_empty() || table.contains('.') {
            dialect.quote(table)
        } else {
            dialect.quote(&format!("{prefix}.{table}"))
        }
    }
}

fn scope_of(database: &Option<String>, schema: &Option<String>) -> String {
    let database = database.clone().unwrap_or_default();
    let schema = schema.clone().unwrap_or_default();
    if schema.trim().is_empty() {
        return database;
    }
    if database.trim().is_empty() {
        return schema;
    }
    format!("{database}.{schema}")
}


// ------------------------------------------------------------------ 元数据

/// 取表的列（先试「模式.表」，失败退回「表」；不存在时返回空而不是报错 ——
/// 「目标表不存在」是一个正常的同步前置条件，不是异常）。
async fn columns_of(
    state: &AppState,
    side: &Side,
    table: &str,
) -> XResult<Vec<ColumnDetail>> {
    let target = crate::api::scope::resolve(state, &side.conn, &side.scope).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let names = {
        let qualified = side.scoped_meta(table);
        if qualified == table {
            vec![table.to_string()]
        } else {
            vec![qualified, table.to_string()]
        }
    };
    for name in names {
        let engine = state.engine();
        let target = target.clone();
        let found = blocking(move || engine.list_columns_fresh(&target, &name)).await;
        if let Ok(mut columns) = found {
            if !columns.is_empty() {
                // **必须补类型长度**：内核给的是裸类型名（MySQL 的 JDBC metadata 里
                // `name` 列就是 `varchar`，长度在另一列），直接拿去拼建表语句会得到
                // `\`name\` varchar not null` —— MySQL 直接报语法错（实测：跨库同步
                // 建表失败「You have an error in your SQL syntax」，而例程/视图那两条路是好的）。
                // 与 `/columns` 接口共用同一套补全，口径一致。
                crate::api::meta::enrich_column_types(
                    state,
                    &side.conn,
                    &side.scope,
                    table,
                    &mut columns,
                )
                .await;
                return Ok(columns);
            }
        }
    }
    Ok(Vec::new())
}

/// 数据同步/对比的默认会话亲和键（读页、事务控制、串行写都落在这条泳道上）。
pub(crate) const SESSION_SYNC: &str = "internal:browse";

/// 「目标库不存在时自动创建」（默认开）：查目标连接的库清单，缺则按方言 CREATE。
/// 返回一句话结果（给任务日志用）；失败**不阻断** —— 后面建表时数据库真缺会明确报错，
/// 比这里瞎猜原因强。库名从 tgt.scope 拆：schema 方言（`库.模式`）取库段；
/// Doris 的全限定（internal.ods）在 scope::resolve 已限定 internal，取末段。
async fn ensure_target_db(state: &AppState, tgt: &Side, opts: &Opts) -> Option<String> {
    if !opts.auto_create_db {
        return None;
    }
    let target_kind = require_record(state, &tgt.conn)
        .await
        .ok()
        .map(|r| Dialect::new(r.kind()))?;
    // 库名从 tgt.scope 拆，方向按方言：
    // - Doris（catalog 方言）的全限定是 `catalog.库` ⇒ 取**末段**（internal.cm_tgt → cm_tgt；
    //   取首段会拿到 `internal` —— 它永远「存在」，自动建库就永远不触发，真机踩过）；
    // - schema 方言（SQL Server 的 `库.模式`）⇒ 取**首段**。
    let db_name = if target_kind.catalog_level() {
        tgt.scope.rsplit('.').next().unwrap_or("").trim().to_string()
    } else {
        tgt.scope.split('.').next().unwrap_or("").trim().to_string()
    };
    if db_name.is_empty() {
        return None;
    }
    let exists = match target_kind.databases() {
        crate::api::dialect::Meta::Sql(sql) => {
            run_sql_in(state, &tgt.conn, "", sql, 5000)
                .await
                .map(|r| {
                    crate::api::meta::rows_of(&r)
                        .iter()
                        .any(|row| {
                            row.values()
                                .filter_map(Value::as_str)
                                .any(|v| v.eq_ignore_ascii_case(&db_name))
                        })
                })
                .unwrap_or(true) // 查不到清单就当存在，别误建
        }
        _ => true,
    };
    if exists {
        return None;
    }
    let create_db = match target_kind.kind.key() {
        // MySQL 系建库**必须显式 utf8mb4**：服务器默认库字符集常是 latin1，之后中文
        // INSERT 直接报 Incorrect string value 1366（真机踩过）—— 库里先别埋雷。
        // Doris 的字符集由表级控制，无此问题。
        "mysql" | "mariadb" => {
            format!(
                "create database if not exists {} default charset utf8mb4",
                target_kind.quote(&db_name)
            )
        }
        "doris" => {
            format!("create database if not exists {}", target_kind.quote(&db_name))
        }
        "sqlserver" => {
            // db_id 的参数是**字符串字面量**（N'..'）—— 用方括号会被当成列引用
            //（真机：「列名 'sync_audit' 无效」），只有 CREATE DATABASE 用方括号
            format!(
                "if db_id(N'{}') is null create database {}",
                // 字面量里的单引号要写成两个；标识符走方言引号（库名可能带引号/方括号）
                db_name.replace("'", "''"),
                target_kind.quote(&db_name)
            )
        }
        _ => format!("create database {}", target_kind.quote(&db_name)),
    };
    // ⚠ 两个坑都在这条建库语句上：
    // 1. 必须走**主连接**（scope 空）—— 影子连接的 URL 就指向这个还不存在的库，
    //    走它发 CREATE DATABASE 是「用连不上的连接去修连不上的原因」（真机踩过）；
    // 2. 必须走**可写执行**（run_target，read_only: None）—— 元数据链路的 run_sql_in
    //    是强制只读会话，ClickHouse 会直接拒绝建库（READONLY，真机踩过；Doris 宽容才没暴露）。
    match run_target(state, &tgt.conn, &create_db).await {
        Ok(_) => Some(format!("目标库 {db_name} 不存在，已自动创建")),
        Err(err) => Some(format!("目标库 {db_name} 自动创建失败：{}", err.message)),
    }
}

/// 直接对**已解析**的目标执行。`sync_table` 在开头解析一次，循环里的每条语句
/// 复用结果 —— 原来每条 `run_sql` 都重新 `require_record`，单表几百条语句
/// 就是几百次白查（每条语句固定链路开销里它占一份）。
pub(crate) async fn run_target(state: &AppState, target: &str, sql: &str) -> XResult<dbmind_core::QueryResult> {
    run_target_in(state, target, sql, SESSION_SYNC).await
}

/// 同 [`run_target`]，但显式指定会话亲和键。
///
/// **为什么需要**：内核的会话池按「连接 + 亲和键」分泳道，而**带亲和键的泳道上限恒为 1**
/// （`agent_driver::lane_limit`：亲和就是「这一个页签只许一条物理连接」的语义）。
/// 于是并发写若都传同一个键，N 个 writer 会全部排在**同一条** JDBC 连接上 ——
/// 并发池看着有 6 个 worker，实际吞吐等于单连接（真机复现：4 个并发请求各写 1 万行，
/// 总耗时正好是单个的 4 倍）。给每个 writer 一个**不同的**键，才是 N 条真实连接。
pub(crate) async fn run_target_in(
    state: &AppState,
    target: &str,
    sql: &str,
    session: &str,
) -> XResult<dbmind_core::QueryResult> {
    let engine = state.engine();
    let request = dbmind_core::QueryRequest {
        schema: None,
        read_only: None,
        connection: target.to_string(),
        sql: sql.to_string(),
        options: dbmind_core::QueryOptions {
            max_rows: 1,
            timeout_ms: 600_000,
        },
        execution_id: None,
        session: Some(session.to_string()),
        // 数据同步是界面功能驱动的批量读写：不进查询历史（一次同步会下发几百条语句）
        internal: true,
    };
    let result = blocking(move || engine.execute(request, dbmind_core::AccessContext::Web)).await?;
    Ok(result)
}

/// 拉**一页**数据（`offset` 起，最多 `PAGE` 行）。
///
/// 调用方自己循环翻页 —— 同步是"读一页写一页"（见 `sync_table`），
/// upsert 前读目标主键也是逐页累积（见那里的循环）。这样内存占用只与一页有关。
/// `cursor`：主键游标分页（keyset）。`Some((键列名, 上一页最后一行的键值))` 时生成
/// `where (k1 > v1) or (k1 = v1 and k2 > v2) … order by 键 limit n` —— **每页只扫本页**，
/// 千万级深分页不再 O(N²) 退化（OFFSET 越深扫得越多，是同步 1100 万行只有 1500 行/秒的主因）。
/// 行值构造符 `(a,b) > (x,y)` 只有 MySQL/PG 认，SQL Server/Doris 不支持 ⇒ 展开成等价的逐列链。
/// `None`（无主键的表）退回原 OFFSET 分页 —— 正确性不变，只是慢。
pub(crate) async fn fetch_page(
    state: &AppState,
    conn: &str,
    scope: &str,
    select_base: &str,
    dialect: Dialect,
    offset: u64,
    cursor: Option<(&[String], &[CellValue])>,
    extra_where: &str,
    page_rows: u64,
    session: &str,
) -> XResult<Vec<Vec<CellValue>>> {
    // 游标值先渲染成 SQL 字面量（`page_sql` 只拼字符串，便于单测）
    let cursor_lits: Option<(&[String], Vec<String>)> = match cursor {
        Some((keys, last)) if !keys.is_empty() && last.len() == keys.len() => {
            Some((keys, last.iter().map(|cell| literal_for(cell, dialect, None)).collect()))
        }
        _ => None,
    };
    let sql = page_sql(
        select_base,
        dialect,
        offset,
        cursor_lits.as_ref().map(|(keys, lits)| (*keys, lits.as_slice())),
        extra_where,
        page_rows,
    );
    // 传输的**源端拉数**（一页一页翻到完）是长任务：走独立会话键，不占元数据那条连接。
    // 分片并发时每片给**不同的键**，否则几片会挤在同一条物理连接上排队。
    let result = run_sql_in_keyed(state, conn, scope, sql, page_rows as usize, session).await?;
    Ok(result.rows)
}

/// 拼「一页」的 SELECT（有键走 keyset 游标，无键退回 OFFSET）。
///
/// 单独抽成纯函数是为了**能被单测盯住**：这里曾漏掉一个分支 —— 用户没写过滤条件时，
/// keyset 条件被整段丢掉，第二页把第一页又读一遍。表现是「传到第一页（5 万行）就开始
/// 报主键冲突 / 整表被重复插入」，而日志里一句话都没有（真机复现：SQLite→H2 6 万行
/// 只落了 5 万行且报主键冲突，20 万行跑 151 秒）。用户过滤条件与游标条件是**两回事**，
/// 必须各自独立判断有没有。
fn page_sql(
    select_base: &str,
    dialect: Dialect,
    offset: u64,
    cursor: Option<(&[String], &[String])>,
    extra_where: &str,
    page_rows: u64,
) -> String {
    let filtered = |base: &str, cond: &str| match (extra_where.trim().is_empty(), cond.is_empty()) {
        (true, true) => base.to_string(),
        (true, false) => format!("{base} where ({cond})"),
        (false, true) => format!("{base} where ({extra_where})"),
        (false, false) => format!("{base} where ({extra_where}) and ({cond})"),
    };
    match cursor {
        Some((keys, last)) if !keys.is_empty() && last.len() == keys.len() => {
            let order = keys
                .iter()
                .map(|key| dialect.quote(key))
                .collect::<Vec<_>>()
                .join(", ");
            // 行值构造符 `(a,b) > (x,y)` 只有 MySQL/PG 认，SQL Server/Doris 不支持
            // ⇒ 展开成等价的逐列 or 链：(a > x) or (a = x and b > y)
            let mut conditions: Vec<String> = Vec::new();
            for depth in 0..keys.len() {
                let mut parts: Vec<String> = Vec::new();
                for (position, key) in keys.iter().enumerate() {
                    let quoted = dialect.quote(key);
                    let value = &last[position];
                    if position < depth {
                        parts.push(format!("{quoted} = {value}"));
                    } else {
                        parts.push(format!("{quoted} > {value}"));
                        break;
                    }
                }
                conditions.push(format!("({})", parts.join(" and ")));
            }
            let keyset = conditions.join(" or ");
            format!(
                "{} order by {order} {}",
                filtered(select_base, &keyset),
                dialect.limit_clause(0, page_rows)
            )
        }
        _ => {
            let page = dialect.limit_clause(offset, page_rows);
            if dialect.needs_order_by_for_paging() {
                format!("{} order by (select null) {page}", filtered(select_base, ""))
            } else {
                format!("select * from ({}) dbmind_page {page}", filtered(select_base, ""))
            }
        }
    }
}

/// 建表后把**源表的非主键索引**搬到目标（「建表包含索引」选项）。
///
/// 索引清单走方言的 `indexes()` 元数据查询（与树上「索引」节点同一份 SQL）。
/// 宽容策略：查不到清单 / 解析不了列 / 目标方言拒绝某条 CREATE —— 都**静默跳过**
/// 并计数，不让索引同步反过来挡住数据同步（数据才是主体）。
async fn sync_indexes(
    state: &AppState,
    src: &Side,
    source_kind: Dialect,
    tgt_target: &str,
    target_kind: Dialect,
    target_name: &str,
    table: &str,
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
) -> usize {
    // 源索引清单，统一成 (名称, 唯一, 列清单)。两条路：
    // - MySQL 系（含 Doris）：`SHOW INDEX FROM 表` —— **必须走这条**，Doris 的
    //   information_schema.statistics 不反映 CREATE INDEX 建的二级索引（真机实测返回空），
    //   SHOW INDEX 才看得到；同名索引多行（一列一行），按 Seq_in_index 聚合成列序；
    // - 其它方言：走 dialect.indexes() 的整库元数据查询（原逻辑）。
    let mut indexes: Vec<(String, bool, Vec<String>)> = Vec::new();
    let mysql_family = matches!(source_kind.kind.key(), "mysql" | "mariadb" | "doris");
    if mysql_family {
        let show_sql = format!("show index from {}", src.scoped(table, Dialect::new(source_kind.kind.clone())));
        if let Ok(result) = run_sql_in(state, &src.conn, &src.scope, show_sql, 5000).await {
            // 大小写不敏感取列（有的驱动把列名折成大写）
            let cell = |row: &serde_json::Map<String, Value>, key: &str| -> Option<String> {
                row.get(key)
                    .or_else(|| {
                        row.iter()
                            .find(|(name, _)| name.eq_ignore_ascii_case(key))
                            .map(|(_, v)| v)
                    })
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };
            // 聚合：Key_name → (non_unique, [(seq, column)])
            let mut agg: Vec<(String, i64, Vec<(i64, String)>)> = Vec::new();
            for row in crate::api::meta::rows_of(&result) {
                let name = match cell(&row, "Key_name") {
                    Some(n) if !n.is_empty() => n,
                    _ => continue,
                };
                let col = match cell(&row, "Column_name") {
                    Some(c) if !c.is_empty() => c,
                    _ => continue,
                };
                let seq = cell(&row, "Seq_in_index")
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(1);
                let non_unique = cell(&row, "Non_unique")
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(1);
                match agg.iter_mut().find(|(n, _, _)| *n == name) {
                    Some((_, _, cols)) => cols.push((seq, col)),
                    None => agg.push((name, non_unique, vec![(seq, col)])),
                }
            }
            for (name, non_unique, mut cols) in agg {
                cols.sort_by_key(|(seq, _)| *seq);
                indexes.push((name, non_unique == 0, cols.into_iter().map(|(_, c)| c).collect()));
            }
        }
    } else if let crate::api::dialect::Meta::Sql(sql) = source_kind.indexes() {
        if let Ok(result) = run_sql_in(state, &src.conn, &src.scope, sql, 5000).await {
            for row in crate::api::meta::rows_of(&result) {
                // 只看这张表的索引（表名大小写不敏感 —— 有的驱动会折大写）
                let row_table = row.get("table").and_then(Value::as_str).unwrap_or("");
                if !row_table.eq_ignore_ascii_case(table) {
                    continue;
                }
                let name = row.get("name").and_then(Value::as_str).unwrap_or("");
                let unique = row.get("non_unique").and_then(crate::api::shape::value_to_i64) == Some(0)
                    || row.get("unique").and_then(Value::as_bool) == Some(true)
                    || row.get("isUnique").and_then(crate::api::shape::value_to_i64) == Some(1);
                // 列清单：PG 系从 indexdef 里抠括号；解析不出就跳过，不编造
                let columns: Vec<String> = match row.get("columns").and_then(Value::as_str) {
                    Some(s) if !s.trim().is_empty() => s
                        .split(',')
                        .map(|c| c.trim().to_string())
                        .filter(|c| !c.is_empty())
                        .collect(),
                    _ => {
                        let def = row.get("sql").and_then(Value::as_str).unwrap_or("");
                        let def = if def.is_empty() {
                            row.get("indexdef").and_then(Value::as_str).unwrap_or("")
                        } else {
                            def
                        };
                        match def.split_once('(') {
                            Some((_, rest)) => rest
                                .split(')')
                                .next()
                                .unwrap_or("")
                                .split(',')
                                .map(|c| c.trim().trim_matches(['"', '`', '[', ']']).to_string())
                                .filter(|c| !c.is_empty())
                                .collect(),
                            None => continue,
                        }
                    }
                };
                if columns.is_empty() {
                    continue;
                }
                indexes.push((name.to_string(), unique, columns));
            }
        }
    }
    let mut created = 0usize;
    for (name, unique, columns) in indexes {
            let idx_name = target_kind.quote(name.as_str());
            let col_list = columns
                .iter()
                .map(|c| target_kind.quote(c))
                .collect::<Vec<_>>()
                .join(", ");
            let unique_kw = if unique { "unique " } else { "" };
            // **索引名保持源名**（用户口径：源叫什么目标就叫什么；重名/不支持时静默跳过）
            let idx_sql = match target_kind.kind.key() {
                // Doris：二级索引用 BITMAP 写法（普通 create index 不带 USING 会失败）
                "doris" => format!("create index {} on {} ({}) using bitmap", idx_name, target_name, col_list),
                // ClickHouse：二级索引必须带 TYPE 与 GRANULARITY，从 MySQL 索引推导不出类型 ——
                // 硬建只会失败，明确跳过并留日志（数据同步不受影响）
                "clickhouse" => {
                    if let Some(task) = task {
                        task.log(format!("表 {}：跳过索引 {}（ClickHouse 二级索引需要指定 TYPE，无法从源索引推导）", table, name));
                    }
                    continue;
                }
                _ => format!("create {}index {} on {} ({})", unique_kw, idx_name, target_name, col_list),
            };
            if run_target(state, tgt_target, &idx_sql).await.is_ok() {
                created += 1;
            }
        }
    if created > 0 {
        if let Some(task) = task {
            task.log(format!("表 {table}：已创建 {created} 个索引"));
        }
    }
    created
}

// ------------------------------------------------------------------ Doris Stream Load 快速通道

/// Doris **Stream Load**：把一批行以 JSON 数组直接 HTTP PUT 给 FE，绕过 INSERT 的
/// 语句大小限制 —— 20 万行从分钟级降到秒级（MySQL 协议 1000~2000 行/批是吞吐瓶颈）。
///
/// - 端口推导：连接里配的是 MySQL 协议端口（Doris 惯例 9030），Stream Load 走 FE 的
///   HTTP 端口，惯例是 **http 端口 = mysql 端口 - 1000**（9030 → 8030）；
/// - 值一律按字符串传（JSON 里也是字符串），Doris 按目标列类型自动转换；
/// - FE 可能 307 重定向到 BE：ureq 跟随重定向并重发 body（label 保证幂等）；
/// - 失败返回 Err，调用方**回退原 INSERT 管道** —— 导入失败不挡数据同步。
/// 标准 base64（Basic 认证用）：不为这一个函数引 base64 crate，30 行自己写。
fn basic_auth(user: &str, pass: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input = format!("{}:{}", user, pass).into_bytes();
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    format!("Basic {}", out)
}

/// 解析 Stream Load 的 307 Location（FE → BE），返回下一步要请求的 URL。
///
/// 真机在 Location 上踩了两个坑：
/// 1. **BE 向 FE 注册的主机是 127.0.0.1**（单机部署 BE 没配外网 IP）—— Location 是
///    `http://...@127.0.0.1:8040/...`，从客户端看 127.0.0.1 是「我自己」，连接必然
///    被拒（os error 10061）。环回主机要改写成 **FE 的主机**（端口仍用 Location 给的
///    BE http 端口，如 8040）；
/// 2. 某些版本的 FE 会把**凭据嵌进 Location**（`root:***@host`）—— 剥掉，认证统一由
///    我们自己的 Authorization 头携带（每跳重发时都带上）。
fn resolve_stream_redirect(fe_url: &str, location: &str) -> String {
    let loc = location.trim();
    let (scheme, rest) = if let Some(r) = loc.strip_prefix("http://") {
        ("http://", r)
    } else if let Some(r) = loc.strip_prefix("https://") {
        ("https://", r)
    } else {
        // 相对路径：拼到 FE 的 scheme://authority 后
        return match fe_url.find("://").and_then(|p| fe_url[p + 3..].find('/').map(|q| p + 3 + q)) {
            Some(slash) => format!("{}{}", &fe_url[..slash], loc),
            None => format!("{fe_url}{loc}"),
        };
    };
    // authority = [userinfo@]host[:port]，path 从第一个 '/' 开始
    let (raw_authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    // 剥 userinfo（取最后一个 '@' 之后的部分；没有 '@' 时原样）
    let authority = raw_authority.rsplit('@').next().unwrap_or(raw_authority);
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h, p),
        None => (authority, if scheme == "https://" { "443" } else { "80" }),
    };
    let host = match host {
        "127.0.0.1" | "localhost" | "::1" | "[::1]" => {
            // 环回主机 → 换成 FE 的主机（FE URL 是我们自己拼的，绝无 userinfo）
            let r = &fe_url[fe_url.find("://").map(|p| p + 3).unwrap_or(0)..];
            let fe_authority = match r.find('/') { Some(i) => &r[..i], None => r };
            fe_authority.split(':').next().unwrap_or(fe_authority)
        }
        other => other,
    };
    format!("{scheme}{host}:{port}{path}")
}

/// Stream Load CSV 的**列分隔符 / 行分隔符**：用 `\x01` / `\x02` 控制字符（header 里写
/// 十六进制转义，Doris 会解析），业务文本里几乎不可能出现 —— 文本值无需任何转义即可
/// 直写，比 JSON 少了键名与引号包裹，BE 解析也快一个量级（实测 1.1MB/s → 数 MB/s）。
const CSV_COL_SEP: &str = "\\x01";
const CSV_ROW_SEP: &str = "\\x02";
const CSV_COL_SEP_BYTE: u8 = 0x01;
const CSV_ROW_SEP_BYTE: u8 = 0x02;

/// 数据里出现了与分隔符相同的字符吗？（极罕见：控制字符混进文本）—— 有则整批退回
/// JSON 编码（JSON 对任意字符都安全），保证正确性优先于速度。
pub(crate) fn rows_need_json_fallback(rows: &[Vec<dbmind_core::CellValue>]) -> bool {
    rows.iter().any(|row| {
        row.iter().any(|cell| match cell {
            dbmind_core::CellValue::Text(s) => {
                s.bytes().any(|b| b == CSV_COL_SEP_BYTE || b == CSV_ROW_SEP_BYTE)
            }
            _ => false,
        })
    })
}

/// 一批行编码成 Stream Load 的 **CSV 体**：`\x01` 列分隔、`\x02` 行分隔；
/// NULL → `\N`（Doris 的空值标记）；BLOB 只带长度（与 JSON 路径同口径，`b:len`）。
fn encode_stream_csv(rows: &[Vec<dbmind_core::CellValue>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rows.len() * 64);
    for row in rows {
        for (idx, cell) in row.iter().enumerate() {
            if idx > 0 {
                out.push(CSV_COL_SEP_BYTE);
            }
            match cell {
                dbmind_core::CellValue::Null => out.extend_from_slice(b"\\N"),
                dbmind_core::CellValue::Integer(n) => out.extend_from_slice(n.to_string().as_bytes()),
                dbmind_core::CellValue::Real(x) => out.extend_from_slice(x.to_string().as_bytes()),
                dbmind_core::CellValue::Text(s) => out.extend_from_slice(s.as_bytes()),
                dbmind_core::CellValue::Blob { len } => {
                    out.extend_from_slice(format!("b:{len}").as_bytes())
                }
            }
        }
        out.push(CSV_ROW_SEP_BYTE);
    }
    out
}

/// 一批行编码成 Stream Load 的 **JSON 体**（CSV 的回退路径，见 rows_need_json_fallback）。
fn encode_stream_json(
    columns: &[String],
    rows: &[Vec<dbmind_core::CellValue>],
) -> Result<Vec<u8>, String> {
    use serde_json::Value;
    let mut arr: Vec<Value> = Vec::with_capacity(rows.len());
    for row in rows {
        let mut obj = serde_json::Map::new();
        for (idx, col) in columns.iter().enumerate() {
            let v = match row.get(idx) {
                None | Some(dbmind_core::CellValue::Null) => Value::Null,
                Some(dbmind_core::CellValue::Integer(n)) => Value::from(*n),
                Some(dbmind_core::CellValue::Real(x)) => Value::from(*x),
                Some(dbmind_core::CellValue::Text(s)) => Value::from(s.as_str()),
                Some(dbmind_core::CellValue::Blob { len }) => Value::from(format!("b:{len}")),
            };
            obj.insert(col.clone(), v);
        }
        arr.push(Value::Object(obj));
    }
    serde_json::to_vec(&arr).map_err(|e| e.to_string())
}

/// Doris 返回的数字字段可能是 JSON number 也可能是字符串（版本差异），两种都认。
fn doris_number(v: Option<&serde_json::Value>) -> u64 {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_u64().or_else(|| n.as_f64().map(|v| v as u64)).unwrap_or(0),
        Some(serde_json::Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// Doris Stream Load 推送一批行。返回**实际载入的行数**（`NumberLoadedRows`）——
/// 类型不匹配等被 BE 过滤的行不计入，由调用方按脏数据记账（对齐 DataX 的 errorLimit 思路）。
pub(crate) async fn doris_stream_load(
    http_port: u16,
    be_port: u16,
    host: &str,
    user: &str,
    password: &str,
    database: &str,
    table: &str,
    columns: &[String],
    rows: &[Vec<dbmind_core::CellValue>],
) -> Result<u64, String> {
    use serde_json::Value;
    if rows.is_empty() || host.is_empty() || http_port == 0 {
        return Ok(0);
    }
    // CSV 优先；文本里混入分隔符控制字符时整批退回 JSON（正确性优先）
    let use_json = rows_need_json_fallback(rows);
    let body = if use_json {
        encode_stream_json(columns, rows)?
    } else {
        encode_stream_csv(rows)
    };
    tracing::info!(rows = rows.len(), bytes = body.len(), csv = !use_json, "Stream Load 编码完成");
    let fe_url = format!("http://{}:{}/api/{}/{}/_stream_load", host, http_port, database, table);
    let label = format!(
        "dbmind_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let user_owned = user.to_string();
    let pass_owned = password.to_string();
    let host_owned = host.to_string();
    let db_owned = database.to_string();
    let tbl_owned = table.to_string();
    crate::api::blocking(move || -> dbmind_core::Result<u64> {
        // **入口候选**：优先直连 BE（SHOW BACKENDS 的真实 http 端口）—— FE 会 307 重定向，
        // 而重定向链路有两个坑：body 要对着 FE 白发一遍（FE 不读 body 就 307）；且
        // `Expect: 100-continue` 下 BE 先回 100 Interim，ureq 把它当终站响应返回、body
        // 根本没发出去 —— BE 等 body、我们等响应，**互相死锁**到 600s 超时（真机「卡住
        // 很久」的根因）。所以这里**绝不带 Expect 头**，且直连 BE 一跳到终站。
        // 直连失败（端口探测不到/连不上）才退回 FE + 手动重定向；再失败交给调用方回退 INSERT。
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(std::time::Duration::from_secs(600))
            .build();
        let mut starts: Vec<String> = Vec::new();
        if be_port > 0 {
            starts.push(format!("http://{}:{}/api/{}/{}/_stream_load", host_owned, be_port, db_owned, tbl_owned));
        }
        starts.push(fe_url.clone());
        for start in starts {
            let mut url = start;
            for _hop in 0..5 {
                let hop_t0 = std::time::Instant::now();
                tracing::info!(url = %url, bytes = body.len(), "Stream Load 发起请求");
                let req = agent
                    .put(&url)
                    .set("Authorization", &basic_auth(&user_owned, &pass_owned))
                    .set("label", &label);
                // CSV：控制字符做分隔符（无键名、无引号包裹，BE 解析快一个量级）；
                // JSON 兜底（数据里混入了分隔符控制字符时）
                let req = if use_json {
                    req.set("format", "json").set("strip_outer_array", "true")
                } else {
                    req.set("format", "csv")
                        .set("column_separator", CSV_COL_SEP)
                        .set("line_delimiter", CSV_ROW_SEP)
                };
                let resp = match req.send_bytes(&body) {
                    Ok(resp) => resp,
                    // ureq 对 >=400 默认返回 Err(Status)，把响应体捞出来才有诊断价值
                    Err(ureq::Error::Status(_, resp)) => resp,
                    // 传输层失败（连不上/连接被断）：换下一个候选入口
                    Err(e) => {
                        tracing::warn!(url = %url, ms = hop_t0.elapsed().as_millis() as u64, error = %e, "Stream Load 传输失败");
                        break;
                    }
                };
                // 307/308：FE → BE 的重定向（兜底链路），换 URL 重发同一请求
                if resp.status() == 307 || resp.status() == 308 {
                    tracing::info!(status = resp.status(), ms = hop_t0.elapsed().as_millis() as u64, "Stream Load 重定向");
                    if let Some(loc) = resp.header("Location") {
                        url = resolve_stream_redirect(&url, loc);
                        continue;
                    }
                }
                tracing::info!(status = resp.status(), ms = hop_t0.elapsed().as_millis() as u64, "Stream Load 终站响应");
                let ctype = resp.header("content-type").unwrap_or("").to_string();
                let text = resp.into_string().unwrap_or_default();
                // 用 chars 截前缀：直接按字节切片会把多字节中文切在中间（panic）
                let head: String = text.chars().take(500).collect();
                let v: Option<Value> = serde_json::from_str(&text).ok();
                match v {
                    Some(v) if v.get("Status").and_then(Value::as_str) == Some("Success") => {
                        // 脏数据记账：类型不匹配等被 BE 过滤的行不算成功载入 ——
                        // 返回实际载入行数，调用方把差额计入失败（对齐 DataX 的 errorLimit 思路）
                        let loaded = doris_number(v.get("NumberLoadedRows"));
                        return Ok(loaded);
                    }
                    Some(v) => {
                        let msg_head: String = v.get("Message").and_then(Value::as_str).unwrap_or(&head).chars().take(500).collect();
                        return Err(dbmind_core::DbMindError::new(
                            dbmind_core::ErrorCode::QueryFailed,
                            format!("Stream Load 失败：{msg_head}"),
                        ))
                    }
                    None => {
                        return Err(dbmind_core::DbMindError::new(
                            dbmind_core::ErrorCode::QueryFailed,
                            format!("Stream Load 响应不是 JSON（content-type={ctype}）：{head}"),
                        ))
                    }
                }
            }
        }
        Err(dbmind_core::DbMindError::new(
            dbmind_core::ErrorCode::QueryFailed,
            "Stream Load 所有入口均不可达（BE 直连与 FE 重定向都失败）".to_string(),
        ))
    })
    .await
    .map_err(|e| format!("工作线程异常：{:?}", e))
}

#[cfg(test)]
mod paging_tests {
    use super::*;
    use dbmind_core::ConnectionKind;

    fn cursor_page(kind: ConnectionKind, keys: &[&str], lits: &[&str], user_where: &str) -> String {
        let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        let lits: Vec<String> = lits.iter().map(|v| v.to_string()).collect();
        page_sql(
            "select `id`, `name` from `t`",
            Dialect::new(kind),
            0,
            Some((keys.as_slice(), lits.as_slice())),
            user_where,
            PAGE,
        )
    }

    /// 回归：用户没写过滤条件时，游标条件**不能**被丢掉（否则第二页重复第一页）。
    #[test]
    fn 无用户过滤时第二页仍带游标条件() {
        let sql = cursor_page(ConnectionKind::Mysql, &["id"], &["50000"], "");
        assert!(sql.contains("id > 50000"), "第二页丢了游标条件：{sql}");
        assert_eq!(sql.matches(" where ").count(), 1, "where 只该有一个：{sql}");
        assert!(sql.contains("order by id"), "缺 order by：{sql}");
    }

    /// 用户过滤条件与游标条件要**同时**在，用 and 相连（顺序无关）。
    #[test]
    fn 用户过滤与游标条件并存() {
        let sql = cursor_page(ConnectionKind::Mysql, &["id"], &["100"], "id <= 100000");
        assert!(sql.contains("id <= 100000"), "丢了用户条件：{sql}");
        assert!(sql.contains("id > 100"), "丢了游标条件：{sql}");
        assert!(sql.contains(" and "), "两个条件应当 and 组合：{sql}");
    }

    /// 多列键展开成 or 链，不能出现行值比较 `(a, b) > (x, y)`（SQL Server/Oracle 不认）。
    #[test]
    fn 多列键展开成or链() {
        let sql = cursor_page(ConnectionKind::Sqlserver, &["a", "b"], &["1", "2"], "");
        assert!(sql.contains("(a > 1) or (a = 1 and b > 2)"), "多列游标展开不对：{sql}");
        assert!(!sql.contains(") > ("), "不能出现行值比较：{sql}");
    }

    /// 无键（cursor = None）退回 OFFSET：不能带 where，且要包一层派生表。
    #[test]
    fn 无键退回offset分页() {
        let sql = page_sql("select `id` from `t`", Dialect::new(ConnectionKind::Mysql), 100000, None, "", PAGE);
        assert!(!sql.contains(" where"), "无键分页不该出现游标 where：{sql}");
        assert!(sql.contains("dbmind_page"), "应包一层派生表：{sql}");
        // 用户条件仍然要生效
        let sql = page_sql("select `id` from `t`", Dialect::new(ConnectionKind::Mysql), 0, None, "`id` < 10", PAGE);
        assert!(sql.contains("`id` < 10"), "无键分页丢了用户条件：{sql}");
    }
}

#[cfg(test)]
mod stream_load_tests {
    use super::*;

    #[test]
    fn csv_编码_空值与各类型() {
        let rows = vec![
            vec![CellValue::Integer(1), CellValue::Text("张三,说\"你好\"".into()), CellValue::Null],
            vec![CellValue::Real(1.5), CellValue::Text("多行\n文本".into()), CellValue::Blob { len: 7 }],
        ];
        let out = encode_stream_csv(&rows);
        let s = String::from_utf8(out).unwrap();
        // 列分隔 0x01、行分隔 0x02；NULL → \N；文本原样（含逗号/引号/换行都无需转义）
        assert_eq!(
            s,
            "1\u{1}张三,说\"你好\"\u{1}\\N\u{2}1.5\u{1}多行\n文本\u{1}b:7\u{2}"
        );
    }

    #[test]
    fn 含分隔符字符的行要退回json() {
        assert!(!rows_need_json_fallback(&[vec![CellValue::Text("普通文本".into())]]));
        assert!(rows_need_json_fallback(&[vec![CellValue::Text("坏\u{1}文本".into())]]));
        assert!(rows_need_json_fallback(&[vec![CellValue::Text("坏\u{2}文本".into())]]));
    }

    #[test]
    fn doris数字字段两种形态都认() {
        use serde_json::json;
        assert_eq!(doris_number(Some(&json!(123))), 123);
        assert_eq!(doris_number(Some(&json!("456"))), 456);
        assert_eq!(doris_number(Some(&json!(null))), 0);
    }

    #[test]
    fn 日期时间值规范化() {
        // 驱动在秒为 00 时省略 :00（真机：MySQL → SQL Server 整表 241）
        assert_eq!(
            normalize_datetime_text("2026-07-09T07:00", "datetime2"),
            Some("2026-07-09 07:00:00".to_string())
        );
        // T 分隔符统一为空格
        assert_eq!(
            normalize_datetime_text("2026-07-09T07:00:10", "datetime2"),
            Some("2026-07-09 07:00:10".to_string())
        );
        // DateTime64(9) 的 9 位小数截断到 6 位
        assert_eq!(
            normalize_datetime_text("2024-06-15T12:30:45.123456789", "datetime2"),
            Some("2024-06-15 12:30:45.123456".to_string())
        );
        // 纯 DATE 目标只留日期段
        assert_eq!(
            normalize_datetime_text("2024-06-15", "date"),
            Some("2024-06-15".to_string())
        );
        assert_eq!(
            normalize_datetime_text("2024-06-15T12:30:45", "date"),
            Some("2024-06-15".to_string())
        );
        // 非 datetime2 目标列（如 nvarchar）不处理
        assert_eq!(normalize_datetime_text("2026-07-09T07:00", "nvarchar"), None);
        // 不是日期形态的文本不处理
        assert_eq!(normalize_datetime_text("随便的文本", "datetime2"), None);
        assert_eq!(normalize_datetime_text("", "datetime2"), None);
        // TIME 目标补齐秒
        assert_eq!(
            normalize_datetime_text("2026-07-09T07:00", "time"),
            Some("07:00:00".to_string())
        );
    }
}

// ------------------------------------------------------------------ 单表同步

/// 目标表**已存在**时，把源侧的表注释 / 列注释**差异补齐**（源有、目标没有或不一致才动）。
///
/// 「源表有注释的，目标就也要有」—— 之前注释只在建表那一刻写进去，目标表若已存在
///（Merge 策略），源后来补的注释永远过不去。逐条 ALTER 的方言差异：
/// - 表注释：Doris/CH `MODIFY COMMENT`；MySQL `COMMENT=`；PG/Oracle/DM `COMMENT ON TABLE`；
/// - 列注释：Doris/MySQL `MODIFY COLUMN <原定义> COMMENT`（这两个改注释必须整列重写，
///   类型用**目标现有**的完整类型文本，避免把精度/长度改丢）；PG/Oracle/DM/CH `COMMENT ON COLUMN`；
/// - SQL Server 的注释挂在扩展属性上（新增/更新是两条系统过程），先不做，缺了不挡数据。
async fn update_comments(
    state: &AppState,
    src: &Side,
    source_kind: Dialect,
    tgt: &Side,
    target_kind: Dialect,
    tgt_target: &str,
    target_name: &str,
    table: &str,
    source_columns: &[dbmind_core::ColumnDetail],
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
) {
    let key = target_kind.kind.key();
    // ---- 表注释 ----
    let src_table = crate::api::meta::table_comment_of(state, &src.conn, &src.scope, table)
        .await
        .unwrap_or_default();
    if !src_table.trim().is_empty() {
        let tgt_table = crate::api::meta::table_comment_of(state, &tgt.conn, &tgt.scope, table)
            .await
            .unwrap_or_default();
        if tgt_table.trim() != src_table.trim() {
            let lit = target_kind.literal(&src_table);
            let sql = match key {
                "doris" => format!("alter table {target_name} modify comment {lit}"),
                "clickhouse" => format!("alter table {target_name} modify comment {lit}"),
                "mysql" | "mariadb" => format!("alter table {target_name} comment={lit}"),
                "postgresql" | "kingbase" | "oracle" | "dm" => {
                    format!("comment on table {target_name} is {lit}")
                }
                _ => String::new(),
            };
            if !sql.is_empty() && run_target(state, tgt_target, &sql).await.is_ok() {
                if let Some(task) = task {
                    task.log(format!("表 {table}：表注释已同步"));
                }
            }
        }
    }
    // ---- 列注释 ----
    let src_cols = crate::api::meta::table_comments_map(state, &src.conn, &src.scope, table).await;
    if src_cols.is_empty() {
        return;
    }
    let tgt_cols = crate::api::meta::table_comments_map(state, &tgt.conn, &tgt.scope, table).await;
    let mut updated = 0usize;
    for (column, comment) in &src_cols {
        if comment.trim().is_empty() {
            continue;
        }
        let same = tgt_cols
            .get(column)
            .map(|existing| existing.trim() == comment.trim())
            .unwrap_or(false);
        if same {
            continue;
        }
        let qcol = target_kind.quote(column);
        let lit = target_kind.literal(comment);
        // Doris / MySQL 的 MODIFY 需要整列定义：用**源列**的类型翻译到目标方言
        //（目标表多半由同步所建，类型即源的类型映射）；找不到源列就跳过该列
        let full_def = source_columns
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(column))
            .map(|c| {
                let raw = c.type_name.clone().unwrap_or_else(|| "text".to_string());
                let mapped = target_kind.map_type(Some(source_kind.kind), &raw);
                let null = if c.nullable { " NULL" } else { " NOT NULL" };
                format!("{} {mapped}{null}", target_kind.quote(&c.name))
            });
        let sql = match key {
            "clickhouse" => format!("alter table {target_name} modify column {qcol} comment {lit}"),
            "postgresql" | "kingbase" | "oracle" | "dm" => {
                format!("comment on column {target_name}.{qcol} is {lit}")
            }
            "doris" | "mysql" | "mariadb" => match &full_def {
                Some(def) => format!("alter table {target_name} modify column {def} comment {lit}"),
                None => continue,
            },
            _ => continue, // SQL Server 等：注释走扩展属性，先不做
        };
        if run_target(state, tgt_target, &sql).await.is_ok() {
            updated += 1;
        }
    }
    if updated > 0 {
        if let Some(task) = task {
            task.log(format!("表 {table}：已同步 {updated} 个列注释"));
        }
    }
}

struct ObjResult {
    ty: String,
    name: String,
    status: &'static str,
    message: String,
    inserted: u64,
    updated: u64,
}

impl ObjResult {
    fn ok(ty: &str, name: &str, message: impl Into<String>, inserted: u64, updated: u64) -> Self {
        Self {
            ty: ty.to_string(),
            name: name.to_string(),
            status: "ok",
            message: message.into(),
            inserted,
            updated,
        }
    }

    fn skipped(ty: &str, name: &str, message: impl Into<String>) -> Self {
        Self {
            ty: ty.to_string(),
            name: name.to_string(),
            status: "skipped",
            message: message.into(),
            inserted: 0,
            updated: 0,
        }
    }

    fn failed(ty: &str, name: &str, message: impl Into<String>) -> Self {
        Self {
            ty: ty.to_string(),
            name: name.to_string(),
            status: "error",
            message: message.into(),
            inserted: 0,
            updated: 0,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "type": self.ty,
            "name": self.name,
            "status": self.status,
            "message": self.message,
            "inserted": self.inserted,
            "updated": self.updated,
        })
    }
}

fn multi_row(dialect: Dialect) -> bool {
    matches!(
        dialect.kind.key(),
        "mysql" | "mariadb" | "doris" | "postgresql" | "kingbase" | "sqlite" | "sqlserver"
            | "h2" | "clickhouse"
    )
}

/// 单格 → SQL 字面量：目标列是**真布尔**时走布尔字面量；日期时间列先做**文本规范化**。
///
/// 布尔：内核把布尔表示为 0/1 整数，而 PG / Oracle 的 boolean 列不接受整数 `0`
/// （报"列是 boolean，表达式是 integer"）—— 跨类型建表时我们自己就会给目标建出 boolean 列，
/// 所以这条必须跟上。
///
/// 日期时间规范化（真机：MySQL → SQL Server 全表 241）：JDBC 驱动回传的 MySQL
/// datetime 是 ISO 文本，且**秒为 00 时省略 `:00`**（`2026-07-09T07:00`）—— SQL Server
/// 的 datetime2 拒绝这个形态（「从字符串转换日期和/或时间时，转换失败」241），分钟粒度的
/// 表（网闸通行记录这类）整表全炸。这里按目标列类型把值统一成方言最稳的形态：
/// `T` → 空格、缺秒补 `:00`、小数超 6 位截断（datetime2 上限 7 位，9 位的 DateTime64 也炸）。
pub(crate) fn literal_for(cell: &CellValue, dialect: Dialect, target_type: Option<&str>) -> String {
    if let (CellValue::Integer(value), Some(type_name)) = (cell, target_type) {
        if Dialect::is_boolean_type(type_name) {
            return dialect.bool_literal(*value != 0).to_string();
        }
    }
    if let (CellValue::Text(text), Some(type_name)) = (cell, target_type) {
        if let Some(normalized) = normalize_datetime_text(text, type_name) {
            return dialect.literal(&normalized);
        }
    }
    sql_literal(cell, dialect)
}

/// 日期时间目标列的值规范化。返回 `Some(规范文本)` 表示按文本字面量写入（已转义），
/// `None` 表示不是日期时间列（或值形态不需要处理），走原路。
fn normalize_datetime_text(text: &str, target_type: &str) -> Option<String> {
    let base = target_type
        .split('(')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let family = match base.as_str() {
        "date" => "date",
        "time" => "time",
        "datetime" | "datetime2" | "smalldatetime" | "timestamp" | "timestamptz"
        | "datetimeoffset" | "datetime64" => "datetime",
        _ => return None,
    };
    let t = text.trim();
    // 只处理「长得像日期时间」的文本；其它内容原样走 sql_literal（转义照旧）
    let looks_like = t.len() >= 8
        && t.as_bytes()[4] == b'-'
        && t.as_bytes()[7] == b'-'
        && t.chars().nth(4).is_some()
        && t.chars().take(4).all(|c| c.is_ascii_digit());
    if !looks_like {
        return None;
    }
    // 统一 T 分隔符 → 空格（各数据库对空格形态的接受度最高）
    let mut s = t.replacen('T', " ", 1).replacen('t', " ", 1);
    if family == "date" {
        // 目标是纯 DATE：只留日期段
        let day = s.split([' ', 'T']).next().unwrap_or(&s).to_string();
        return Some(day);
    }
    if family == "time" {
        // 目标是 TIME：只留时间段（T 后或空格后），补齐 hh:mm:ss
        let time_part = s.split_once(' ').map(|x| x.1).unwrap_or(&s).to_string();
        return Some(pad_time(&time_part));
    }
    // datetime：日期段 + 时间段；时间段补齐 hh:mm:ss[.fff…]
    let (date_part, time_part) = match s.split_once(' ') {
        Some((d, tm)) => (d.to_string(), tm.to_string()),
        None => (s.clone(), String::new()),
    };
    if time_part.is_empty() {
        s = format!("{date_part} 00:00:00");
    } else {
        s = format!("{date_part} {}", pad_time(&time_part));
    }
    // 小数超 6 位截断（datetime2 上限 7 位，9 位的 DateTime64(9) 直接 241）
    if let Some(dot) = s.find('.') {
        let digits = s[dot + 1..].chars().filter(|c| c.is_ascii_digit()).count();
        if digits > 6 {
            s = format!("{}{}", &s[..dot], &s[dot..dot + 7]);
        }
    }
    Some(s)
}

/// 时间段补齐到至少 `hh:mm:ss`（驱动在秒为 00 时会省略 `:00`）。
fn pad_time(time_part: &str) -> String {
    let (core, frac) = match time_part.split_once('.') {
        Some((c, f)) => (c, Some(f)),
        None => (time_part, None),
    };
    let mut parts: Vec<String> = core.split(':').map(str::to_string).collect();
    while parts.len() < 3 {
        parts.push("00".to_string());
    }
    let mut out = parts.join(":");
    if let Some(f) = frac {
        out.push('.');
        out.push_str(f);
    }
    out
}

/// 给 INSERT 语句包上**同批的** `SET IDENTITY_INSERT ON/OFF`。
///
/// 为什么拼进同一批而不是单独执行：`IDENTITY_INSERT` 是**会话级**的，连接池里
/// 每条语句可能落在不同会话 —— 单独执行"开"，写入落到另一个会话照样报
/// 「当 IDENTITY_INSERT 为 OFF 时不能插入显式值」（真机踩过）。拼在同一批里
/// 开、写、关必然同一个会话，稳定生效。多语句分号批对 JDBC 是单次提交。
fn with_identity(sql: String, identity_on: bool, target_name: &str) -> String {
    if identity_on {
        format!(
            "set identity_insert {target_name} on; {sql}; set identity_insert {target_name} off"
        )
    } else {
        sql
    }
}

/// 每条 INSERT 语句的**字节**上限：语句就是 MySQL 协议里的一个 packet，
/// 超过目标的 `max_allowed_packet` 直接被拒（真机：Doris FE 默认只有 1MB，
/// 宽行 2000 行能拼出 7.3MB → 「Packet for query is too large (7,334,180 > 1,048,576)」）。
/// 行数上限管不住行宽（JSON/长文本列一行几 KB），必须字节 + 行数**双重约束**。
fn insert_stmt_cap(kind_key: &str) -> usize {
    match kind_key {
        // Doris FE 的 max_allowed_packet 默认 1MB → 512KB 留足语句头/转义余量
        "doris" => 512 * 1024,
        // MySQL/MariaDB 旧版默认 4MB（8.0 起动态 64MB）→ 3MB
        "mysql" | "mariadb" => 3 * 1024 * 1024,
        // 其余方言没有 1MB 级 packet 限制，这里只防单语句内存尖峰
        _ => 16 * 1024 * 1024,
    }
}

/// 单行 tuple 在 SQL 里的体积**上界**粗估：Text 按字符数 ×2（转义/引号翻倍余量），
/// 数值/时间列按 40 估；逐字符精算不值得，估大了只是多拆一条语句。
pub(crate) fn estimate_row_bytes(row: &[CellValue]) -> usize {
    row.iter()
        .map(|c| match c {
            CellValue::Text(s) => s.len() * 2 + 8,
            CellValue::Blob { len } => *len * 2 + 8,
            _ => 40,
        })
        .sum::<usize>()
        + 8
}

/// 单行在 Stream Load **JSON 体**里的体积粗估：字符串基本原样进 JSON（无需 ×2 转义），
/// 加上键名与逗号/引号开销按每列 24 字节估。
pub(crate) fn json_row_bytes(row: &[CellValue]) -> usize {
    row.iter()
        .map(|c| match c {
            CellValue::Text(s) => s.len() + 24,
            CellValue::Blob { len } => *len + 24,
            _ => 24,
        })
        .sum::<usize>()
        + 8
}

/// 生成一批 INSERT **语句列表**（(行数, SQL)，调用方逐条发送）。
///
/// 拆分是**双重约束**，谁先到谁触发：
/// - 行数：SQL Server 多行 VALUES 单条语句限 1000 行（解析器硬限制，真机踩过）；
/// - 字节：单条语句估出来的体积不许超过目标方言的 packet 上限（见 insert_stmt_cap）——
///   行数上限对宽行（JSON 列等）无能为力，2000 行也能拼出 7MB。
/// 拆出来的每条语句各自独立发送，单条失败不影响之前已落盘的语句（部分成功语义）。
fn build_insert_batch(
    table: &str,
    columns: &[String],
    types: &[Option<String>],
    rows: &[Vec<CellValue>],
    dialect: Dialect,
    kind_key: &str,
) -> Vec<(usize, String)> {
    const SQLSERVER_VALUES_LIMIT: usize = 1000;
    let row_limit = if kind_key == "sqlserver" { SQLSERVER_VALUES_LIMIT } else { usize::MAX };
    let byte_cap = insert_stmt_cap(kind_key);
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut start = 0usize;
    let mut acc_bytes = 0usize;
    let mut acc_rows = 0usize;
    for (i, row) in rows.iter().enumerate() {
        let row_bytes = estimate_row_bytes(row);
        if acc_rows >= row_limit || (acc_rows > 0 && acc_bytes + row_bytes > byte_cap) {
            out.push((i - start, build_insert(table, columns, types, &rows[start..i], dialect)));
            start = i;
            acc_bytes = 0;
            acc_rows = 0;
        }
        acc_bytes += row_bytes;
        acc_rows += 1;
    }
    if start < rows.len() {
        out.push((rows.len() - start, build_insert(table, columns, types, &rows[start..], dialect)));
    }
    out
}

/// 把一批行按「行数+字节拆分」写成 INSERT 语句并逐条发送（**部分成功语义**）。
///
/// **脏数据降级**（对齐 DataX）：某条语句失败时**降级为该语句块逐行重试** ——
/// 坏行（日期转换失败、超长、类型不匹配…）单独计失败并记样例错误，好行照常落库，
/// 不让一行烂数据拖死整批（真机：SQL Server 日期转换 241，一个坏值炸掉 11,574 行）。
/// 逐行重试里若**过半仍失败**，判定为系统性问题（连接断/权限/表没了），立即中止并上抛
/// —— 不值得把几千行逐行磨完。
/// 返回成功写入的行数。串行管道与并发写池共用这一份实现，杜绝两处逻辑漂移。
pub(crate) async fn write_insert_rows(
    state: &AppState,
    tgt_target: &str,
    session: &str,
    target_name: &str,
    columns: &[String],
    target_types: &[Option<String>],
    target_kind: Dialect,
    rows: &[Vec<CellValue>],
    identity_on: bool,
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
    table: &str,
) -> Result<u64, String> {
    let stmts = build_insert_batch(target_name, columns, target_types, rows, target_kind, target_kind.kind.key());
    let mut ok_rows = 0u64;
    let mut failed_rows = 0u64;
    let mut sample_err: Option<String> = None;
    let mut start_row = 0usize;
    for (n_rows, stmt) in &stmts {
        match run_target_in(state, tgt_target, &with_identity(stmt.clone(), identity_on, target_name), session).await {
            Ok(_) => ok_rows += *n_rows as u64,
            Err(_batch_err) => {
                // 降级：这一语句块的行逐行重试
                let chunk = &rows[start_row..start_row + *n_rows as usize];
                let mut chunk_failed = 0u64;
                for row in chunk {
                    let single = build_insert(target_name, columns, target_types, std::slice::from_ref(row), target_kind);
                    match run_target_in(state, tgt_target, &with_identity(single, identity_on, target_name), session).await {
                        Ok(_) => ok_rows += 1,
                        Err(row_err) => {
                            chunk_failed += 1;
                            if sample_err.is_none() {
                                sample_err = Some(row_err.message.chars().take(200).collect());
                            }
                        }
                    }
                }
                failed_rows += chunk_failed;
                // 过半失败 ⇒ 系统性问题（连接断/权限/表没了），逐行磨完没有意义
                if chunk_failed * 2 > *n_rows as u64 {
                    if let Some(task) = task {
                        task.add_rows_failed(rows.len() as u64 - ok_rows);
                    }
                    return Err(sample_err.unwrap_or_else(|| _batch_err.message));
                }
            }
        }
        start_row += *n_rows as usize;
    }
    if failed_rows > 0 {
        if let Some(task) = task {
            task.add_rows_failed(failed_rows);
            let head = sample_err.unwrap_or_default();
            task.log(format!(
                "表 {table}：{} 行写入失败（脏数据，已单独计失败），样例错误：{head}",
                failed_rows
            ));
        }
    }
    // 写入计数在这里统一报（串行管道与并发写池共用本函数 —— 之前并发写池的重构
    // 把调用方的 add_rows_written 弄丢了，界面上「传输数」全程为 0，真机踩过）
    if ok_rows > 0 {
        if let Some(task) = task {
            task.add_rows_written(ok_rows);
        }
    }
    Ok(ok_rows)
}

/// upsert 更新批的大小：**一条 RPC 里连发多少条 UPDATE**。
///
/// 逐行 UPDATE 是「第二次同步特别慢」的根因：每行一次内核→宿主调用，本地库约 1ms、
/// 远端库 20~100ms，10 万行要更新的表就是几十分钟（DataX/Kettle 的更新是批量下发的）。
/// 支持一条语句里带多条 SQL 的方言攒成一批发一次；其余方言保持逐行 —— 它们的驱动
/// 不接受一次 execute 多条语句，硬拼只会整批语法错。
/// 500 条/批 ≈ 40KB 的批文本：再大对 RTT 的摊薄收益已经很小，
/// 而单批失败时的逐行降级代价会跟着变大。
pub(crate) const UPDATE_BATCH: usize = 500;

/// 目标方言能否在**一次 execute** 里执行多条 SQL（分号分隔的批）。
///
/// MySQL/MariaDB/Doris 的 JDBC URL 带 `allowMultiQueries=true`（见连接类型 YAML），
/// SQL Server 的多语句批是 T-SQL 原生语义，PG/Kingbase 的 pgjdbc 允许一次发送多条。
/// Oracle/DB2/DM 会报「无效字符」，H2/Derby 不认多语句，ClickHouse 的
/// `ALTER TABLE … UPDATE` 也不是这个形状 —— 这些保持逐行。
pub(crate) fn supports_multi_statement(kind: &str) -> bool {
    matches!(
        kind,
        "mysql" | "mariadb" | "doris" | "sqlserver" | "postgresql" | "kingbase"
    )
}

/// 把攒下的 UPDATE 语句**一次**发出去；整批失败时**降级逐行重试**（与 INSERT 同一套
/// 脏数据策略：一行坏数据不该拖死一整批）。UPDATE 本身幂等，重试是安全的。
/// 返回成功更新的行数。
pub(crate) async fn write_update_batch(
    state: &AppState,
    tgt_target: &str,
    session: &str,
    stmts: &mut Vec<String>,
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
    table: &str,
) -> Result<u64, String> {
    if stmts.is_empty() {
        return Ok(0);
    }
    let total = stmts.len() as u64;
    let mut ok = 0u64;
    let joined = stmts.join(";\n");
    match run_target_in(state, tgt_target, &joined, session).await {
        Ok(_) => ok = total,
        Err(batch_err) => {
            let mut failed = 0u64;
            let mut sample: Option<String> = None;
            for sql in stmts.iter() {
                match run_target_in(state, tgt_target, sql, session).await {
                    Ok(_) => ok += 1,
                    Err(err) => {
                        failed += 1;
                        if sample.is_none() {
                            sample = Some(err.message.chars().take(200).collect());
                        }
                    }
                }
            }
            // 过半失败 ⇒ 系统性问题（连接断/权限/表被删），别再逐行磨
            if failed * 2 > total {
                stmts.clear();
                return Err(sample.unwrap_or_else(|| batch_err.message));
            }
            if failed > 0 {
                if let Some(task) = task {
                    task.add_rows_failed(failed);
                    task.log(format!(
                        "表 {table}：upsert 批量更新里 {failed} 行失败（脏数据，已逐行重试过），样例错误：{}",
                        sample.unwrap_or_default()
                    ));
                }
            }
        }
    }
    stmts.clear();
    Ok(ok)
}

fn build_insert(
    table: &str,
    columns: &[String],
    types: &[Option<String>],
    rows: &[Vec<CellValue>],
    dialect: Dialect,
) -> String {
    let tuples: Vec<String> = rows
        .iter()
        .map(|row| {
            let values: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(index, cell)| {
                    literal_for(cell, dialect, types.get(index).and_then(|ty| ty.as_deref()))
                })
                .collect();
            format!("({})", values.join(", "))
        })
        .collect();
    format!(
        "insert into {table} ({}) values {}",
        columns
            .iter()
            .map(|name| dialect.quote(name))
            .collect::<Vec<_>>()
            .join(", "),
        tuples.join(", ")
    )
}

pub(crate) fn build_update(
    table: &str,
    columns: &[String],
    types: &[Option<String>],
    row: &[CellValue],
    key_indexes: &[usize],
    dialect: Dialect,
) -> String {
    let type_at = |index: usize| types.get(index).and_then(|ty| ty.as_deref());
    let assignments: Vec<String> = columns
        .iter()
        .enumerate()
        .filter(|(index, _)| !key_indexes.contains(index))
        .map(|(index, name)| {
            let value = row
                .get(index)
                .map(|cell| literal_for(cell, dialect, type_at(index)))
                .unwrap_or_else(|| "null".to_string());
            format!("{} = {value}", dialect.quote(name))
        })
        .collect();
    let where_clause: Vec<String> = key_indexes
        .iter()
        .map(|index| {
            let value = row
                .get(*index)
                .map(|cell| literal_for(cell, dialect, type_at(*index)))
                .unwrap_or_else(|| "null".to_string());
            format!("{} = {value}", dialect.quote(&columns[*index]))
        })
        .collect();
    // 更新语句按目标方言选：ClickHouse 只认 `ALTER TABLE ... UPDATE ... WHERE`
    dialect.update_sql(table, &assignments.join(", "), &where_clause.join(" and "))
}

/// 主键值 → 可比较的键（归一规则集中在 `shape`）。
///
/// 这里也必须归一：upsert 是"先读目标已有关键值、再拿源行的键来比"，
/// 两侧的同一列完全可能是不同变体（目标 `NUMBER` 回 real、源 `int` 回 integer），
/// 不归一会导致**每一行都被判成新增**（于是重复插入），而界面上仍显示同步成功。
pub(crate) fn key_of(row: &[CellValue], indexes: &[usize]) -> String {
    crate::api::shape::row_key(row, indexes)
}

/// 同步一张表（外层：MySQL 系目标的事务包裹）。
///
/// autocommit 模式下**每条 INSERT 都刷一次盘**（实测 2000 行/条要 1.2 秒，大头是 fsync）——
/// 把整张表的写入包进一个事务，只在结尾 commit 一次，写入吞吐能翻数倍。
/// 语义保持：成功/失败/取消都 **commit**（「已写入的部分保留，不回滚」是既有约定，
/// 失败时保留的正是事务里已执行成功的那些行）。
/// 按目标方言开事务包批：MySQL/MariaDB（autocommit 开关）、PostgreSQL/Kingbase、SQLite、SQL Server
/// （begin/commit）。Doris 的事务没有实际意义，ClickHouse / Oracle / 达梦 / DB2 没有可用的 SQL 级
/// begin —— 这些照旧逐条提交（慢，但功能不缺）。
#[allow(clippy::too_many_arguments)]
async fn sync_table(
    state: &AppState,
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
    src: &Side,
    tgt: &Side,
    table: &str,
    target_table: &str,
    opts: &Opts,
) -> ObjResult {
    // 事务由**写入插件**自己管（`transfer::writer`）：每路 writer 在自己的物理会话上
    // begin/commit。改造前这里在一条会话上 `set autocommit = 0` 再让 N 路 writer 往
    // **别的**会话写 —— 那层外层事务根本管不到，等于没开（真机踩过）。
    // 结构操作（建表/索引）也不该和数据写挤在一个事务里：DDL 会隐式提交。
    sync_table_inner(state, task, src, tgt, table, target_table, opts).await
}

/// 目标端「表内写并发」的并行度（1 = 串行）。**依据类型声明，不靠猜**：
///
/// - `singleConnectionPool` 的类型（H2 / Derby / SQLite / SQL Server / Redis）声明了
///   「只允许一条物理连接」—— 多开只会排队，反而多一次排队/调度开销，保持 1；
/// - 其余（MySQL / MariaDB / PostgreSQL / Kingbase / Oracle / DB2 / ClickHouse …）取
///   类型的 `maxConnections`（YAML 里 MySQL/PG 是 4），再夹到 6 —— 再多压不出吞吐，
///   只会把目标库的行锁竞争放大成「互相等」。
///
/// 这个数直接决定写池的 worker 数：每个 worker 用**自己的会话键**（见 `run_target_in`），
/// 才会真的落到不同连接上。
pub(crate) fn write_parallelism(kind: dbmind_core::ConnectionKind) -> usize {
    if kind.single_connection_pool() {
        return 1;
    }
    kind.max_connections().clamp(1, 6)
}

/// 目标方言的「开事务 / 提交 / 收尾复位」三条语句；`None` = 该方言不批事务（照旧逐条提交）。
/// MySQL 系靠 autocommit 开关；PG/SQLite 用标准 begin/commit；SQL Server 用 begin transaction。
/// Doris / ClickHouse / Oracle / 达梦 / DB2 故意不列 —— 它们没有可用的 SQL 级 begin，
/// 硬上会直接把写入路径打挂。
pub(crate) fn txn_batch(kind: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match kind {
        "mysql" | "mariadb" => Some(("set autocommit = 0", "commit", "set autocommit = 1")),
        "postgresql" | "kingbase" | "sqlite" => Some(("begin", "commit", "")),
        "sqlserver" => Some(("begin transaction", "commit", "")),
        _ => None,
    }
}

/// 同步一张表（内层：实际传输逻辑）。
#[allow(clippy::too_many_arguments)]
async fn sync_table_inner(
    state: &AppState,
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
    src: &Side,
    tgt: &Side,
    table: &str,
    target_table: &str,
    opts: &Opts,
) -> ObjResult {
    let ty = "table";
    // Navicat 式阶段日志：每张表的「读结构 → 建表 → 拉数据 → 完成」都进任务日志，
    // 界面上能看出 339 张表各自进行到哪一步（此前只有完成时一条，中途日志是空的）
    if let Some(task) = task {
        task.log(format!("表 {table}：读取结构"));
    }
    let source_columns = match columns_of(state, src, table).await {
        Ok(columns) => columns,
        Err(err) => return ObjResult::failed(ty, table, format!("读取源表结构失败：{}", err.message)),
    };
    if source_columns.is_empty() {
        return ObjResult::failed(ty, table, "源表不存在或没有列信息");
    }
    let target_columns = columns_of(state, tgt, target_table)
        .await
        .unwrap_or_default();
    // record 保存下来：Doris Stream Load 需要 host / 端口 / 凭据（见 doris_stream_load）
    let tgt_record = match require_record(state, &tgt.conn).await {
        Ok(record) => Some(record),
        Err(err) => return ObjResult::failed(ty, table, err.message),
    };
    let target_kind = crate::api::dialect::Dialect::new(
        tgt_record.as_ref().expect("上一行已保证 Some").kind(),
    );
    // 源的方言在建表时就要用（跨类型要把列类型翻译到目标），所以提前到这里取
    let source_kind = Dialect::new(
        match require_record(state, &src.conn).await {
            Ok(record) => record.kind(),
            Err(err) => return ObjResult::failed(ty, table, err.message),
        },
    );
    // `None` = 同类型（照抄）；`Some(别的类型)` = 跨类型，类型与默认值都要翻译
    let cross_kind = if source_kind.kind == target_kind.kind {
        None
    } else {
        Some(source_kind.kind)
    };
    let target_name = target_kind.quote(target_table);
    // 目标连接只解析一次，循环里的每条语句复用（见 run_target 的注释）
    let tgt_target = match crate::api::scope::resolve(state, &tgt.conn, &tgt.scope).await {
        Ok(target) => target,
        Err(err) => return ObjResult::failed(ty, table, format!("解析目标连接失败：{}", err.message)),
    };
    let mut created = false;

    // 注释（表 + 字段）从**源表**取一次，供下面的「自动建表 / 删除重建」写进建表语句。
    // 内核的列元数据不带注释（`ColumnDetail` 没有 comment 字段），只有方言的元数据查询
    // 取得到 —— 取不到就是空表/None，**不编造**：注释拿不到时建表语句与以前完全一样。
    let src_column_comments =
        crate::api::meta::table_comments_map(state, &src.conn, &src.scope, table).await;
    let src_table_comment =
        crate::api::meta::table_comment_of(state, &src.conn, &src.scope, table).await;

    if target_columns.is_empty() {
        if !opts.sync_structure {
            return ObjResult::skipped(ty, table, "目标表不存在，且未勾选「目标表不存在时自动建表」");
        }
        if let Some(task) = task {
            task.log(format!("表 {table}：创建表"));
        }
        // 带上源表的表注释与字段注释（见上面取注释那一段）
        let ddl = target_kind.create_table_with_comments(
            &target_name,
            &source_columns,
            cross_kind,
            &src_column_comments,
            src_table_comment.as_deref(),
        );
        if let Err(err) = run_target(state, &tgt_target, &ddl).await {
            let mut msg = format!("建表失败：{}", err.message);
            // 「Row size too large (> 8126)」（MySQL 1118）：目标表列太多/太宽，DDL 这边已经做到
            // 头了（长文本列早就是 longtext、行格式也显式 dynamic），剩下能帮上忙的是**报出规模** ——
            // 多少列、其中多少列是长文本，用户才知道该精简列、还是该换个目标库。
            if err.message.to_ascii_lowercase().contains("row size too large") {
                let total = source_columns.len();
                let wide = source_columns
                    .iter()
                    .filter(|c| {
                        let t = c.type_name.clone().unwrap_or_default().to_ascii_lowercase();
                        ["string", "text", "blob", "clob", "json"]
                            .iter()
                            .any(|key| t.contains(key))
                    })
                    .count();
                msg.push_str(&format!(
                    "（本表共 {total} 列，其中 {wide} 列是长文本。MySQL/InnoDB 单行上限 8126 字节，\
                     列太多时无论怎么映射类型都建不出来：① 只同步真正需要的列；\
                     ② 目标改用 Doris / PostgreSQL / ClickHouse（它们没有这个行宽限制）；\
                     ③ 若目标 MySQL 是 5.6/5.7，确认行格式为 DYNAMIC —— 本工具建表已显式指定。）"
                ));
            }
            return ObjResult::failed(ty, table, msg);
        }
        created = true;
    } else if opts.policy == Policy::Drop {
        if !opts.sync_structure {
            return ObjResult::failed(
                ty,
                table,
                "选择了「删除重建」，但没有勾选自动建表：那样会删掉目标表且建不回来",
            );
        }
        if let Some(task) = task {
            task.log(format!("表 {table}：删除表"));
        }
        if let Err(err) = run_target(state, &tgt_target, &format!("drop table {target_name}")).await {
            return ObjResult::failed(ty, table, format!("删除目标表失败：{}", err.message));
        }
        if let Some(task) = task {
            task.log(format!("表 {table}：创建表"));
        }
        // 带上源表的表注释与字段注释（见上面取注释那一段）
        let ddl = target_kind.create_table_with_comments(
            &target_name,
            &source_columns,
            cross_kind,
            &src_column_comments,
            src_table_comment.as_deref(),
        );
        if let Err(err) = run_target(state, &tgt_target, &ddl).await {
            return ObjResult::failed(ty, table, format!("重建目标表失败：{}", err.message));
        }
        created = true;
    }
        // 建表后才能补的注释（SQL Server 的扩展属性等）：create_table_with_comments
        // 写不进去的方言在这里逐条执行 —— 之前 SQL Server 目标的注释全空（真机反馈）。
        // 失败静默：注释是有价值的补充，不该让它挡住数据同步。
        // schema 只有 SQL Server 那条路要用（扩展属性必须点名 schema）；PG / Oracle / DM / DB2
        // 的 `comment on table <名>` 不带 schema 也会落到当前 search_path 的表上 ——
        // 之前这里硬写 "dbo"，那些库拼出来的语句必然报错，而失败是静默的（注释全丢）。
        let comment_schema = if target_kind.kind.key() == "sqlserver" { "dbo" } else { "" };
        let post_comments = target_kind.post_create_comments(
            target_name.trim_matches(|c| c == '"' || c == '`' || c == ']'),
            &src_column_comments,
            src_table_comment.as_deref(),
            comment_schema,
        );
        for sql in &post_comments {
            let _ = run_target(state, &tgt_target, sql).await;
        }
        if !post_comments.is_empty() {
            if let Some(task) = task {
                task.log(format!("表 {}：已补充注释（{} 条）", table, post_comments.len()));
            }
        }
    // 建表（含删除重建）成功后：把源表的非主键索引搬过来（「建表包含索引」选项）
    if created && opts.include_indexes {
        sync_indexes(
            state,
            src,
            source_kind,
            &tgt_target,
            target_kind,
            &target_name,
            table,
            task,
        )
        .await;
    }
    // 目标表**已存在**（走不到建表，注释也就没机会带上）：把源侧的**表注释 + 列注释**
    // 差异补过去 —— 源有注释而目标没有/不一致的，逐条 ALTER 补齐。
    if !created && opts.sync_structure {
        update_comments(
            state, src, source_kind, tgt, target_kind, &tgt_target, &target_name, table,
            &source_columns, task,
        )
        .await;
    }

    if !opts.sync_data {
        let message = if created { "已按源结构建表（未同步数据）" } else { "已跳过（未勾选同步数据）" };
        return ObjResult::ok(ty, table, message, 0, 0);
    }
    if let Some(task) = task {
        task.log(format!("表 {table}：拉取数据"));
    }

    let target_columns = if created {
        source_columns.clone()
    } else {
        target_columns
    };
    // 只同步两边都有的列：目标多出来的列让它走自己的默认值/自增
    let mut columns: Vec<String> = Vec::new();
    // 二进制列：内核的结果集**只传长度、不传字节**（见 dbmind-core `CellValue` 的设计注释 ——
    // 结果集要跨进程/HTTP 传输）。传过去只可能是一列 NULL，而旧实现正是静默这么干的：
    // 数据丢了，界面上还是「同步成功」。这里明确把它排除在数据同步之外，并在结果里说清楚。
    let mut binary_columns: Vec<String> = Vec::new();
    for column in &source_columns {
        if !target_columns
            .iter()
            .any(|other| other.name.eq_ignore_ascii_case(&column.name))
        {
            continue;
        }
        if Dialect::is_binary_type(column.type_name.as_deref().unwrap_or("")) {
            binary_columns.push(column.name.clone());
            continue;
        }
        columns.push(column.name.clone());
    }
    if columns.is_empty() {
        let why = if binary_columns.is_empty() {
            "源表与目标表没有同名列，无法同步数据"
        } else {
            "需要同步的列全是二进制列：结果集只传长度不传字节，暂不支持同步二进制数据"
        };
        return ObjResult::skipped(ty, table, why);
    }
    let missing: Vec<String> = source_columns
        .iter()
        .filter(|column| !columns.iter().any(|name| name.eq_ignore_ascii_case(&column.name)))
        .filter(|column| {
            !binary_columns
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&column.name))
        })
        .map(|column| column.name.clone())
        .collect();

    // 主键：优先用目标的（目标是写入方），没有再用源的
    let mut key_columns: Vec<String> = target_columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.clone())
        .filter(|name| columns.iter().any(|column| column.eq_ignore_ascii_case(name)))
        .collect();
    if key_columns.is_empty() {
        key_columns = source_columns
            .iter()
            .filter(|column| column.primary_key)
            .map(|column| column.name.clone())
            .filter(|name| columns.iter().any(|column| column.eq_ignore_ascii_case(name)))
            .collect();
    }
    let key_indexes: Vec<usize> = key_columns
        .iter()
        .filter_map(|key| columns.iter().position(|column| column.eq_ignore_ascii_case(key)))
        .collect();

    // 目标侧各列的类型（按同名列对齐）：写值时要用它判"是不是真布尔列"。
    // 刚建出来的表按**映射后**的目标类型算，已存在的表用目标库自己报的类型。
    let target_types: Vec<Option<String>> = columns
        .iter()
        .map(|name| {
            if created {
                source_columns
                    .iter()
                    .find(|column| column.name.eq_ignore_ascii_case(name))
                    .and_then(|column| column.type_name.clone())
                    .map(|ty| target_kind.map_type(cross_kind, &ty))
            } else {
                target_columns
                    .iter()
                    .find(|column| column.name.eq_ignore_ascii_case(name))
                    .and_then(|column| column.type_name.clone())
            }
        })
        .collect();

    if let Some(task) = task {
        task.check_canceled().ok();
        task.set_phase(format!("读取源表 {table} 数据"));
    }

    let mut note = String::new();
    if !missing.is_empty() {
        note.push_str(&format!("（目标缺少 {} 列，未同步）", missing.join("/")));
    }
    // 源侧 select 前缀：下面按页反复用它（见 fetch_page / select_base）

    // 源与目标**是同一张表**（自同步）时，清空会把待读的数据删掉；那种"清空"本身也没有意义。
    // 旧实现是「先把整表读进内存、再清目标」，顺序上不怕；现在改成边读边写，必须显式区分。
    let same_table = src.conn == tgt.conn
        && src.scope.eq_ignore_ascii_case(&tgt.scope)
        && table.eq_ignore_ascii_case(target_table);
    if opts.data_mode == DataMode::TruncateInsert {
        if same_table {
            note.push_str("（源与目标是同一张表，已跳过清空）");
        } else {
            // 清空语句按**目标方言**选：ClickHouse 不吃裸 `delete from`（实测 position 26 语法错），
            // SQLite / Derby 又没有 `truncate` —— 见 `Dialect::clear_table_sql`
            let clear = target_kind.clear_table_sql(&target_name);
            if let Err(err) = run_target(state, &tgt_target, &clear).await {
                return ObjResult::failed(ty, table, format!("清空目标表失败：{}", err.message));
            }
        }
    }

    let mut inserted = 0u64;
    let mut updated = 0u64;
    // 计时只为在结果里给出吞吐（行/秒）：优化前后快了多少，界面上能直接看出来
    let started = std::time::Instant::now();
    let effective_mode = if opts.data_mode == DataMode::Upsert && key_indexes.is_empty() {
        // 没有主键 ⇒ 判断不了「这一行是谁」，upsert 无从下手。
        //
        // 这里**不能**悄悄退化成「仅插入」：那样第二次同步会把整表再插一遍（行长翻倍），
        // 而界面上显示的却是「同步成功」——静默的数据翻倍比直接报错坏得多。
        // 唯一能安全放行的情况是目标表本来就是空的（首次同步），那时插入与 upsert 等价。
        let target_rows = target_kind.count_sql(&target_name);
        let empty = match run_sql_in(state, &tgt.conn, &tgt.scope, target_rows, 1).await {
            Ok(result) => result
                .rows
                .first()
                .and_then(|row| row.first())
                .and_then(|cell| match cell {
                    CellValue::Integer(value) => Some(*value),
                    // ClickHouse 的 count(*) 回来是**浮点**（Real）：只认 Integer 会把
                    // 「空表」误判成「已有数据」，于是对一个空目标表报「无法按主键 upsert」
                    // （真机：CH 同步到空表却被拒）。Text 兜住把计数渲染成字符串的驱动。
                    CellValue::Real(value) => Some(*value as i64),
                    CellValue::Text(text) => text.trim().parse::<i64>().ok(),
                    _ => None,
                })
                .unwrap_or(-1)
                == 0,
            Err(_) => false,
        };
        if !empty {
            return ObjResult::failed(
                ty,
                table,
                format!(
                    "目标表 {target_table} 没有主键，无法按主键 upsert（目标表里已有数据）。\
                     请改用「仅插入」或「清空后导入」，或给目标表加上主键"
                ),
            );
        }
        note.push_str("（目标表没有主键，且当前为空：已按仅插入处理）");
        DataMode::Insert
    } else {
        opts.data_mode
    };

    // upsert：先把目标已有关键值读进来，源行分成「新增」与「更新」两拨。
    // 这里**逐页读、不设上限**：旧实现用 MAX_ROWS 截断，大表下会把「已有行」误判成「新增」，
    // 于是重复插入 —— 那是静默的数据错误，比慢一点糟得多。
    let mut existing: std::collections::HashSet<String> = std::collections::HashSet::new();
    if effective_mode == DataMode::Upsert {
        let key_positions: Vec<usize> = (0..key_indexes.len()).collect();
        let key_list = key_columns
            .iter()
            .map(|name| target_kind.quote(name))
            .collect::<Vec<_>>()
            .join(", ");
        let base = format!("select {key_list} from {target_name}");
        let mut key_offset = 0u64;
        // 目标键读取同样走 keyset（目标键列即主键，唯一有序）——千万级目标表不再深分页
        let mut last_target_key: Option<Vec<CellValue>> = None;
        loop {
            let cursor = match last_target_key.as_ref() {
                Some(values) if !key_columns.is_empty() => {
                    Some((key_columns.as_slice(), values.as_slice()))
                }
                _ => None,
            };
            let page =
                match fetch_page(state, &tgt.conn, &tgt.scope, &base, target_kind, key_offset, cursor, "", PAGE, "internal:rowcount")
                    .await
                {
                    Ok(rows) => rows,
                    Err(err) => {
                        return ObjResult::failed(
                            ty,
                            table,
                            format!(
                                "读取目标表主键失败（upsert 需要它来判断新增还是更新）：{}",
                                err.message
                            ),
                        )
                    }
                };
            let got = page.len();
            for row in &page {
                existing.insert(key_of(row, &key_positions));
            }
            if got < PAGE as usize {
                break;
            }
            // 目标键查询只选了键列 ⇒ 整行就是键值序列（顺序与 key_columns 一致）
            if !key_columns.is_empty() {
                last_target_key = page.last().cloned();
            }
            key_offset += PAGE;
        }
    }

    // SQL Server 的 identity 列**不接受显式插入**，而同步写过去的正是显式的键值
    //（删除重建建的表带 identity(1,1)、目标已存在的 identity 表同样）。
    //
    // **不做任何预判**（真机教训：用源列的 auto_increment 判断，ClickHouse 源压根没有
    // 这个概念，判断永远 false，products 直接全灭）—— 只要目标是 SQL Server 就开：
    // - 目标表有 identity 列 → 开关生效，显式键值能插；
    // - 没有 → 数据库报「没有标识属性」，**降级为说明继续写**（本来就不需要这个开关）；
    // - 权限等错误同样降级 —— 数据真写不进去自会报行级错误，不该让保险开关判死刑。
    // 其它数据库（MySQL 的 AUTO_INCREMENT、PG 的 serial 等）显式插入自增列本来就合法，
    // 无需此开关，所以不处理 —— 全部数据库都不会因 identity 报错。
    // identity 开关现在**拼进每条 INSERT 的同一批**（见 with_identity），
    // 不再单独执行 SET —— 会话级开关与写入不同会话时等于没开（真机踩过）。
    //
    // 目标表**是否真有 identity 列**用一条元数据查询确认（sys.identity_columns）：
    // - 有 → 拼 SET 开/关，显式键值能插；
    // - 没有 → **纯 INSERT**（本来就不需要开关）—— 没有这次预查的话，SET 本身会失败
    //   并把整批拖死（SQL Server 批里任一语句失败即中止，真机踩过两张表全灭）；
    // - 预查失败（权限等）按"没有"处理 —— 走纯 INSERT，有 identity 的表会报行级错误，
    //   不静默丢数据。
    let identity_on = target_kind.kind.key() == "sqlserver" && {
        let probe = format!(
            "select count(*) as n from sys.identity_columns where object_id = object_id('{}')",
            target_name.replace('\'', "''")
        );
        match run_target(state, &tgt_target, &probe).await {
            Ok(result) => result
                .rows
                .first()
                .and_then(|row| row.first())
                .map(|v| matches!(v, dbmind_core::CellValue::Integer(n) if *n > 0))
                .unwrap_or(false),
            Err(_) => false,
        }
    };

    // 单条 INSERT 的行数按**目标方言**决定（这是传输吞吐的最大杠杆）。
    // 界面不再让用户填批次 —— 每种库的甜点值差着一个数量级（CH 落 part、Oracle 有
    // 单语句表达式上限、MySQL 看 max_allowed_packet），让用户猜毫无意义，由这里统一拍板：
    // - ClickHouse：每次 INSERT 落一个 part，小批次是 CH 写入的头号杀手 —— 固定 5 万行/条；
    // - MySQL / MariaDB / PG 系：max_allowed_packet 默认 4MB，1 万行（约 1MB）封顶；
    // - Doris：FE 的 max_allowed_packet 默认**只有 1MB**（真机实测 1.48MB 直接被拒），
    //   按典型行宽 ~300B 压到 2000 行/条（≈600KB）；
    // - Oracle / DM / DB2：上限是**单语句表达式总数**（列数 × 行数 ≤ 65535）——
    //   固定 500 行在宽表（130 列 × 500 = 65000）直接爆，窄表又太保守 ——
    //   按「500 与 65535/列数 取小」动态定，下限 10 防极端宽表算出 0；
    // - SQL Server：多行 VALUES 单条语句的**行值表达式上限是 1000**（T-SQL 解析器硬限制，
    //   超了直接报「行值表达式的数目超过了允许的最大行数 1000」—— 真机踩过）——
    //   但**一批里可以放多条 INSERT**（连接已开 allowMultiQueries、安全闸门已放行
    //   INSERT 批，见 build_insert_batch），所以批目标仍取 1 万行，由它拆成 10 条/批；
    // - 其它：维持通用上限。
    let per_statement = if multi_row(target_kind) {
    match target_kind.kind.key() {
    "clickhouse" => 50_000,
    "doris" => 2_000,
    "sqlserver" => 10_000,
    "mysql" | "mariadb" | "postgresql" | "kingbase" => 10_000,
    "oracle" | "dm" | "db2" => 500.min((65_535 / (columns.len() as u64).max(1)).max(10)),
    _ => MAX_TUPLES_PER_STATEMENT,
    }
    } else {
    1
    };
    // **Doris Stream Load 快速通道**：纯插入/清空重建时启用 —— 数据以 JSON 走 HTTP
    // 直推 FE，绕过 MySQL 协议 1000~2000 行/批的吞吐瓶颈；失败自动回退 INSERT 管道。
    let use_stream = target_kind.kind.key() == "doris"
        && effective_mode != DataMode::Upsert
        && tgt_record.is_some();
    // Stream Load 单请求 5 万行（JSON 体积 ≈ 15MB，FE 无语句大小限制）
    let (stream_host, stream_port, stream_user, stream_pass) = tgt_record
        .as_ref()
        .map(|r| {
            let cfg = &r.config;
            let port = cfg.port.unwrap_or(9030);
            let http_port = if port > 1000 { port - 1000 } else { 8030 };
            (cfg.host.clone().unwrap_or_default(), http_port, cfg.username.clone().unwrap_or_default(), cfg.password.clone().unwrap_or_default())
        })
        .unwrap_or_default();
    // **BE 的真实 http 端口**（SHOW BACKENDS 的 HttpPort，真机 8040）：拿到后 Stream Load
    // 直连 BE，不走 FE 的 307 重定向 —— 重定向链路 body 要发两遍（FE 不读 body 就 307），
    // 还踩过 ureq 的 100-continue 死锁（见 doris_stream_load）。查询失败不挡：退回老推导
    // 端口 + 重定向兜底，再不行还有 INSERT 管道。
    let be_http_port: u16 = if use_stream {
        match run_target(state, &tgt_target, "show backends").await {
            Ok(result) => {
                let idx_of = |name: &str| {
                    result.columns.iter().position(|c| c.name.eq_ignore_ascii_case(name))
                };
                match (idx_of("HttpPort"), idx_of("Alive")) {
                    (Some(http_idx), Some(alive_idx)) => result
                        .rows
                        .iter()
                        .filter(|row| {
                            matches!(row.get(alive_idx),
                                Some(dbmind_core::CellValue::Text(s)) if s.trim().eq_ignore_ascii_case("true"))
                        })
                        .find_map(|row| match row.get(http_idx) {
                            Some(dbmind_core::CellValue::Text(s)) => s.trim().parse().ok(),
                            Some(dbmind_core::CellValue::Integer(n)) if *n > 0 => Some(*n as u16),
                            _ => None,
                        })
                        .unwrap_or(0),
                    _ => 0,
                }
            }
            Err(_) => 0,
        }
    } else {
        0
    };
    if use_stream && be_http_port > 0 {
        if let Some(task) = task {
            task.log(format!("表 {table}：Stream Load 直连 BE http 端口 {be_http_port}"));
        }
    }
    // ===== 流式写入：读一页、写一页 =====
    // 旧实现是「整表读进内存 → 再写」，代价是三件事捆在一起：
    //   · 必须有 20 万行的上限，否则内存扛不住；
    //   · 第一行要等整表读完才开始写；
    //   · 取消请求也要等整表读完才可能被检查到。
    // 改成页循环之后这三件事一起消失。跨页不再拼接批次（那只会让"已写行数"与真正
    // 落盘的行数差出一页，没有别的好处）。
    //
    // ===== 同实例快速通道（MySQL → MySQL）=====
    // 源与目标落在**同一个 MySQL 实例**（server_uuid 相同）且是纯插入/清空后导入时，
    // 直接 `insert into 目标(列) select 列 from 源` —— 数据不经过应用层，
    // 20 万行从分钟级降到秒级。不同实例 / upsert 模式 / 非 MySQL 系走原管道。
    if opts.sync_data
        && effective_mode != DataMode::Upsert
        && matches!(source_kind.kind.key(), "mysql" | "mariadb")
        && matches!(target_kind.kind.key(), "mysql" | "mariadb")
    {
        async fn read_uuid(state: &AppState, conn: &str, scope: &str) -> String {
            let Ok(result) = run_sql_in(state, conn, scope, "select @@server_uuid as u".to_string(), 1).await
            else {
                return String::new()
            };
            result
                .rows
                .first()
                .and_then(|row| row.first())
                .and_then(|cell| match cell {
                    CellValue::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        }
        let src_uuid = read_uuid(state, &src.conn, &src.scope).await;
        let tgt_uuid = read_uuid(state, &tgt.conn, &tgt.scope).await;
        if !src_uuid.is_empty() && src_uuid == tgt_uuid {
            if let Some(task) = task {
                task.log(format!("表 {table}：同实例，直拷（insert … select）"));
            }
            let col_list_src = columns
                .iter()
                .map(|name| source_kind.quote(name))
                .collect::<Vec<_>>()
                .join(", ");
            let col_list_tgt = columns
                .iter()
                .map(|name| target_kind.quote(name))
                .collect::<Vec<_>>()
                .join(", ");
            // 直拷也带上用户的行过滤条件（同实例 insert…select 同样只搬过滤后的行）
            // ⚠ 源表引用**必须带源库名限定**：这条 SQL 在目标连接上执行（目标影子已切到
            // 目标库），不能复用 src.scoped 的裸名口径（那是「在源连接上执行」的规则）——
            // 裸名会被解析到目标库，`from 目标表` 读到刚建好的空表，直拷插进去 0 行
            //（真机：同实例双库同步 500 行 → 0 行，还报「直拷完成」）
            let src_qualified = if src.scope.trim().is_empty() || table.contains('.') {
                table.to_string()
            } else {
                format!("{}.{table}", src.scope.trim())
            };
            let where_sql = if opts.where_clause.is_empty() {
                src_qualified
            } else {
                format!("{} where {}", src_qualified, opts.where_clause)
            };
            let insert_sql = format!(
                "insert into {target_name} ({col_list_tgt}) select {col_list_src} from {where_sql}"
            );
            tracing::info!(sql = %insert_sql, "同实例直拷 SQL");
            // 直拷同样要过 identity：同批 SET 开/关（会话级开关必须与写入同一批）
            let insert_sql = with_identity(
                insert_sql,
                target_kind.kind.key() == "sqlserver",
                &target_name,
            );
            // 防御：先结束目标会话上可能残留的旧事务 —— 池化会话是粘性的，此前某次
            // 同步若没走到 commit，autocommit=0 的旧事务会一直开着，REPEATABLE READ
            // 的快照停在很久以前（直拷/回查全部看到旧世界，真机抓到过 src_visible=0）
            let _ = run_target(state, &tgt_target, "commit").await;
            // 诊断：直拷前在**目标会话**上确认源表可见性与行数
            if let Ok(result) = run_target(state, &tgt_target, &format!("select count(*) as n from {where_sql}")).await {
                tracing::info!(src_visible = ?result.rows.first().and_then(|r| r.first()), "直拷前源表回查");
            } else {
                tracing::warn!("直拷前源表回查失败（目标会话看不到源表）");
            }
            let copy_result = run_target(state, &tgt_target, &insert_sql).await;
            let mut copied = match &copy_result {
                // INSERT … SELECT 的影响行数就是传输行数（比再发一条 count 快且准）
                Ok(result) => {
                    tracing::info!(affected = ?result.affected_rows, rows = result.rows.len(), "直拷结果");
                    result.affected_rows.unwrap_or(0) as u64
                }
                Err(err) => return ObjResult::failed(ty, table, format!("同实例直拷失败：{}", err.message)),
            };
            // MySQL 驱动对 INSERT…SELECT 可能回 0（数据实际已写入，真机：目标 3 行、
            // affected=0）—— 影响行数不可信时回查目标表行数兜底。
            // 计数单元格可能是 Integer 也可能是 Text（驱动的 BIGINT 回传差异），两种都认。
            if copied == 0 {
                if let Ok(result) = run_target(state, &tgt_target, &format!("select count(*) as n from {target_name}")).await {
                    tracing::info!(count = ?result.rows.first().and_then(|r| r.first()), "直拷兜底回查");
                    if let Some(cell) = result.rows.first().and_then(|row| row.first()) {
                        match cell {
                            dbmind_core::CellValue::Integer(n) => copied = *n as u64,
                            dbmind_core::CellValue::Text(t) => {
                                if let Ok(n) = t.trim().parse::<u64>() {
                                    copied = n;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            if let Some(task) = task {
                task.add_rows_written(copied);
            }
            // 日志只报「执行到哪一步」—— 行数在进度卡上（rowsWritten 平滑递增），不进日志刷屏
            let message = format!(
                "直拷完成{}{}",
                if created { "（已建表）" } else { "" },
                note
            );
            return ObjResult::ok(ty, table, message, copied, 0);
        }
    }
    // ===== 数据搬运：交给 transfer 引擎（DataX 风格的 Reader/Writer 插件）=====
    //
    // 这一段原来是内联的「预取协程 + 主循环 + 写池」（1200 行，读写方言细节与并发/重试/
    // 进度/取消全缠在一起）。现在职责切开：
    //   · 本层（sync）只管**结构**：建表、索引、注释、序列回拨、对象级结果；
    //   · transfer 引擎管**搬行**：按主键分片并发读 → 有界通道 → 多连接并发写。
    // 想给某一种库换取数/落库方式（Doris 的 Stream Load、同实例的 INSERT…SELECT），
    // 只需要在 `transfer::reader_for` / `transfer::writer_for` 里换实现，这里一行不用改。

    // 主键游标（keyset）分页：有主键时替代 OFFSET —— 深分页每页只扫本页，
    // 千万级行不再 O(N²) 退化。无主键退回 OFFSET（cursor 传 None）。
    let paging_keys: Vec<String> = key_indexes.iter().map(|i| columns[*i].clone()).collect();
    if paging_keys.is_empty() {
        // 没有可用的键：分页只能退回 OFFSET，第 N 页要让源库白扫前 N×5 万行（O(N²)）。
        // 这个代价用户看不见，只会觉得「越大越慢」，所以明说一句怎么解。
        if let Some(task) = task {
            task.log(format!(
                "表 {table}：源表没有主键/唯一键，分页只能用 OFFSET（每页都要重扫前面所有行），大表会越来越慢；建议在同步配置里指定主键列"
            ));
        }
    }

    let plan = crate::api::transfer::TablePlan {
        state: state.clone(),
        task: task.cloned(),
        src_conn: src.conn.clone(),
        src_scope: src.scope.clone(),
        source_kind,
        src_from: src.scoped(table, source_kind),
        table: table.to_string(),
        columns: columns.clone(),
        tgt_conn: tgt.conn.clone(),
        tgt_scope: tgt.scope.clone(),
        tgt_target: tgt_target.clone(),
        target_kind,
        target_name: target_name.clone(),
        target_types,
        key_columns,
        key_indexes,
        paging_keys,
        where_clause: opts.where_clause.clone(),
        row_limit: opts.row_limit,
        upsert: effective_mode == DataMode::Upsert,
        existing: std::sync::Arc::new(existing),
        per_statement,
        identity_on,
        stream: if use_stream {
            Some(crate::api::transfer::StreamCfg {
                fe_http_port: stream_port,
                be_http_port,
                host: stream_host.clone(),
                user: stream_user.clone(),
                pass: stream_pass.clone(),
            })
        } else {
            None
        },
    };
    let stats = match crate::api::transfer::run(plan).await {
        Ok(stats) => stats,
        Err(msg) => return ObjResult::failed(ty, table, msg),
    };
    inserted += stats.inserted;
    updated += stats.updated;
    note.push_str(&stats.note);

    // **PG / KingBase 的序列回拨**：显式插入自增列的值后，目标库的序列还停在起点 ——
    // 之后业务插入新行时序列从 1 开始生成主键，**必然撞上刚同步进来的数据**（迁移工具的经典坑）。
    // 同步完成后把每个自增列的序列拨到 max(col)+1；失败降级为说明（不影响已写入的数据）。
    if matches!(target_kind.kind.key(), "postgresql" | "kingbase") {
        for column in source_columns.iter().filter(|c| c.auto_increment) {
            let quoted = target_kind.quote(&column.name);
            // setval 的前两个参数是字符串字面量：单引号包，内部单引号翻倍转义
            let lit = |s: &str| format!("'{}'", s.replace('\'', "''"));
            let sql = format!(
                "select setval(pg_get_serial_sequence({tbl}, {col_lit}), coalesce(max({quoted}), 0) + 1, false) from {target_name}",
                tbl = lit(&target_name),
                col_lit = lit(&column.name),
            );
            if let Err(err) = run_target(state, &tgt_target, &sql).await {
                note.push_str(&format!("（序列回拨失败 {col}：{msg}）",
                    col = column.name, msg = err.message));
            }
        }
    }

    // 日志只报步骤；行数走 rowsWritten/rowsRead 计数（进度卡平滑递增），不进文本
    let _ = started;
    let message = format!(
        "传输完成{}{}",
        if created { "（已建表）" } else { "" },
        note
    );
    ObjResult::ok(ty, table, message, inserted, updated)
}

/// 视图：只有能拿到完整 `CREATE VIEW` 的类型才同步（见文件头决定 4）。
async fn sync_view(
    state: &AppState,
    src: &Side,
    tgt: &Side,
    name: &str,
    opts: &Opts,
) -> ObjResult {
    let source_kind = Dialect::new(
        match require_record(state, &src.conn).await {
            Ok(record) => record.kind(),
            Err(err) => return ObjResult::failed("view", name, err.message),
        },
    );
    // 源侧的模式也带上（Oracle 的 get_ddl 不带 owner 会去会话默认模式里找，跨模式必失败）
    let src_schema = crate::api::meta::split_db_schema(&src.scope, &source_kind).1;
    let Some(ddl_sql) = source_kind.ddl(name, src_schema.as_deref()).sql() else {
        return ObjResult::skipped(
            "view",
            name,
            format!("{} 拿不到完整的视图定义，未同步", source_kind.kind.key().to_ascii_uppercase()),
        );
    };
    let result = match run_sql_in(state, &src.conn, &src.scope, ddl_sql, 5).await {
        Ok(result) => result,
        Err(err) => return ObjResult::failed("view", name, format!("读取视图定义失败：{}", err.message)),
    };
    let mut definition = None;
    for row in result.rows {
        for value in row {
            if let CellValue::Text(text) = value {
                // 行首 create 才算定义（`contains` 会把 MySQL 的 sql_mode 列也算进来，
                // 那列里含 NO_AUTO_CREATE_USER —— 详见 meta::looks_like_ddl）
                if crate::api::meta::looks_like_ddl(&text) {
                    definition = Some(text);
                }
            }
        }
    }
    let Some(definition) = definition else {
        return ObjResult::skipped("view", name, "源库里没有取到该视图的建视图语句");
    };
    let target_kind = Dialect::new(
        match require_record(state, &tgt.conn).await {
            Ok(record) => record.kind(),
            Err(err) => return ObjResult::failed("view", name, err.message),
        },
    );
    let target_name = target_kind.quote(name);
    let tgt_target = match crate::api::scope::resolve(state, &tgt.conn, &tgt.scope).await {
        Ok(target) => target,
        Err(err) => return ObjResult::failed("view", name, format!("解析目标连接失败：{}", err.message)),
    };
    if columns_of(state, tgt, name).await.map(|c| !c.is_empty()).unwrap_or(false)
        && opts.policy != Policy::Drop
    {
        return ObjResult::skipped("view", name, "目标已存在同名视图（保留结构策略下不动它）");
    }
    if opts.policy == Policy::Drop {
        let _ = run_target(state, &tgt_target, &format!("drop view if exists {target_name}")).await;
    }
    match run_target(state, &tgt_target, &definition).await {
        Ok(_) => ObjResult::ok("view", name, "已按源定义建视图", 0, 0),
        Err(err) => ObjResult::failed("view", name, format!("建视图失败：{}", err.message)),
    }
}

/// 例程 / 触发器 / 事件：定义同样从 `object_source` 取 —— 与「查看定义」「导出」同一条路。
///
/// 原先这四类在同步里是**一律跳过**的（`UNSUPPORTED_KINDS`），可它们跟视图没有本质区别：
/// 都能从源库取到完整 `create` 文本、都能在目标库执行。跳过的唯一后果是"整库同步后
/// 目标库少了这些对象"，而用户看报告时只得到一句"尚未实现"。
///
/// 「删除重建」策略需要先删目标对象：例程没有 `create or replace` 的通用写法，
/// 所以这里用 `drop X if exists`（方言支持才用，不支持就跳过并说明——
/// 见 `Dialect::drop_object_sql`）。
async fn sync_object_source(
    state: &AppState,
    src: &Side,
    tgt: &Side,
    kind: &str,
    name: &str,
    opts: &Opts,
) -> ObjResult {
    let source_kind = match require_record(state, &src.conn).await {
        Ok(record) => Dialect::new(record.kind()),
        Err(err) => return ObjResult::failed(kind, name, err.message),
    };
    let src_schema = crate::api::meta::split_db_schema(&src.scope, &source_kind).1;
    let Some(sql) = source_kind.object_source(kind, name, src_schema.as_deref()).sql() else {
        return ObjResult::skipped(
            kind,
            name,
            format!(
                "{} 取不到该对象的定义，未同步",
                source_kind.kind.key().to_ascii_uppercase()
            ),
        );
    };
    let result = match run_sql_in(state, &src.conn, &src.scope, sql, 5).await {
        Ok(result) => result,
        Err(err) => {
            return ObjResult::failed(kind, name, format!("读取定义失败：{}", err.message))
        }
    };
    // 取"像 DDL"的那个值：各家的定义列名不一样（definition / sql / Create View …），
    // 按内容判断比按列名稳（与导出、查看定义同一套判据）
    let mut definition = None;
    for row in result.rows {
        for value in row {
            if let CellValue::Text(text) = value {
                // 行首 create 才算定义（`contains` 会把 MySQL 的 sql_mode 列也算进来，
                // 那列里含 NO_AUTO_CREATE_USER —— 详见 meta::looks_like_ddl）
                if crate::api::meta::looks_like_ddl(&text) {
                    definition = Some(text);
                }
            }
        }
    }
    let Some(definition) = definition else {
        return ObjResult::skipped(kind, name, "源库里没有取到该对象的定义");
    };
    let target_kind = match require_record(state, &tgt.conn).await {
        Ok(record) => Dialect::new(record.kind()),
        Err(err) => return ObjResult::failed(kind, name, err.message),
    };
    let tgt_target = match crate::api::scope::resolve(state, &tgt.conn, &tgt.scope).await {
        Ok(target) => target,
        Err(err) => return ObjResult::failed(kind, name, format!("解析目标连接失败：{}", err.message)),
    };
    if opts.policy == Policy::Drop {
        match target_kind.drop_object_sql(kind, name) {
            Some(drop_sql) => {
                // 目标没有这个对象时 `if exists` 不会报错，所以不看结果
                let _ = run_target(state, &tgt_target, &drop_sql).await;
            }
            None => {
                return ObjResult::skipped(
                    kind,
                    name,
                    format!(
                        "「删除重建」要先删目标对象，而 {} 不支持 `drop {kind} if exists` —— \
                         请先在目标库删除后再同步",
                        target_kind.kind.key().to_ascii_uppercase()
                    ),
                )
            }
        }
    }
    match run_target(state, &tgt_target, &definition).await {
        Ok(_) => ObjResult::ok(kind, name, "已按源定义创建", 0, 0),
        Err(err) => ObjResult::failed(kind, name, format!("创建失败：{}", err.message)),
    }
}

// ------------------------------------------------------------------ 处理器

/// `POST /api/sync` —— 单表同步（同步返回结果）。
pub async fn table(
    State(state): State<AppState>,
    Json(body): Json<SyncRequest>,
) -> XResult<Json<Value>> {
    let source_table = body
        .source_table
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("缺少 sourceTable 参数"))?;
    if body.source_connection_id.trim().is_empty() {
        return Err(XError::bad_request("缺少 sourceConnectionId 参数"));
    }
    let opts = body.opts();
    let target_table = body
        .target_table
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| source_table.clone());
    let src = Side {
        conn: body.source_connection_id.clone(),
        scope: scope_of(&body.source_database, &body.source_schema),
        schema: body.source_schema.clone(),
        catalog: false,
    };
    let tgt = Side {
        conn: body.target_connection(),
        scope: scope_of(&body.target_database, &body.target_schema),
        schema: body.target_schema.clone(),
        catalog: false,
    };
    require_record(&state, &src.conn).await?;
    require_record(&state, &tgt.conn).await?;
    // catalog 方言（Doris）的表名要带 `catalog.库` 全限定（见 Side::scoped）
    let src = Side {
        catalog: Dialect::new(require_record(&state, &src.conn).await?.kind()).catalog_level(),
        ..src
    };
    let tgt = Side {
        catalog: Dialect::new(require_record(&state, &tgt.conn).await?.kind()).catalog_level(),
        ..tgt
    };
    let mode = body
        .data_mode
        .clone()
        .or_else(|| body.mode.clone())
        .unwrap_or_else(|| "upsert".to_string());

    // 单表路径同样自动建库（无任务日志渠道，静默执行；失败由建表时的明确报错兜底）
    let _ = ensure_target_db(&state, &tgt, &opts).await;
    let outcome = sync_table(&state, None, &src, &tgt, &source_table, &target_table, &opts).await;
    let success = outcome.status == "ok";
    Ok(Json(json!({
        "success": success,
        "inserted": outcome.inserted,
        "updated": outcome.updated,
        "skipped": if outcome.status == "skipped" { 1 } else { 0 },
        "errors": if success { 0 } else { 1 },
        "total": 1,
        "mode": mode,
        "message": outcome.message,
    })))
}

/// `POST /api/sync/db` —— 多对象同步（异步任务）。
pub async fn db(State(state): State<AppState>, Json(body): Json<SyncRequest>) -> XResult<Json<Value>> {
    if body.source_connection_id.trim().is_empty() {
        return Err(XError::bad_request("缺少 sourceConnectionId 参数"));
    }
    require_record(&state, &body.source_connection_id).await?;
    let target_conn = body.target_connection();
    require_record(&state, &target_conn).await?;

    // 对象清单：显式选中的；一个都没选就按「同步源库全部表」理解（界面允许直接点开始）
    let mut objects: Vec<(String, String)> = Vec::new();
    for name in &body.tables {
        objects.push(("table".to_string(), name.clone()));
    }
    for name in &body.views {
        objects.push(("view".to_string(), name.clone()));
    }
    for (kind, names) in [
        ("function", &body.functions),
        ("procedure", &body.procedures),
        ("trigger", &body.triggers),
        ("event", &body.events),
    ] {
        for name in names {
            objects.push((kind.to_string(), name.clone()));
        }
    }
    if objects.is_empty() {
        let src = Side {
            conn: body.source_connection_id.clone(),
            scope: scope_of(&body.source_database, &body.source_schema),
            schema: body.source_schema.clone(),
            catalog: false,
        };
        let target = crate::api::scope::resolve(&state, &src.conn, &src.scope).await?;
        crate::api::driver::ensure_for_connection(&state, &target).await?;
        let engine = state.engine();
        let tables = blocking(move || engine.list_tables_fresh(&target)).await?;
        for table in tables
            .into_iter()
            .filter(|table| table.kind == TableKind::Table)
        {
            objects.push(("table".to_string(), table.name));
        }
    }
    if objects.is_empty() {
        return Err(XError::bad_request("没有要同步的对象"));
    }

    let opts = body.opts();
    let src = Side {
        conn: body.source_connection_id.clone(),
        scope: scope_of(&body.source_database, &body.source_schema),
        schema: body.source_schema.clone(),
        catalog: false,
    };
    let tgt = Side {
        conn: target_conn,
        scope: scope_of(&body.target_database, &body.target_schema),
        schema: body.target_schema.clone(),
        catalog: false,
    };
    /// 对象级并发的**自适应并行度**（不写死）：
    ///
    /// 两个因素决定最优张数：
    /// 1. **目标库类型** —— 会话结构与写入语义差别很大：
    ///    - 嵌入式/文件引擎（SQLite/Derby/H2）：会话池只有一条连接，并行只会互相排队；
    ///    - Doris：每次导入落一个 part，并发太高小 part 合并压力大，适度即可；
    ///    - Oracle/DM/DB2：会话开销大、锁行为保守，2 路稳；
    ///    - MySQL/PG/SQL Server/ClickHouse 等网络型库：4 路能吃满带宽（再高压垮库反而慢）。
    /// 2. **对象数** —— 只有 2 张表时开 4 路毫无意义，并行度不超过对象数。
    fn adaptive_concurrency(target_kind_key: &str, objects: usize) -> usize {
        if objects <= 1 {
            return 1;
        }
        let by_kind = match target_kind_key {
            "sqlite" | "derby" | "h2" => 1,
            "doris" => 3,
            "oracle" | "dm" | "db2" => 2,
            _ => 4,
        };
        by_kind.min(objects).max(1)
    }
    // catalog 方言（Doris）的表名要带 `catalog.库` 全限定（见 Side::scoped）
    let src = Side {
        catalog: Dialect::new(require_record(&state, &src.conn).await?.kind()).catalog_level(),
        ..src
    };
    let tgt = Side {
        catalog: Dialect::new(require_record(&state, &tgt.conn).await?.kind()).catalog_level(),
        ..tgt
    };
    let total = objects.len();

    let registry = state.tasks.clone();
    let task = registry.spawn("sync", "sync_", move |task| {
        let state = state.clone();
        let src = src.clone();
        let tgt = tgt.clone();
        let opts = opts.clone();
        async move {
            task.set_total(total as i64);
            // 目标库不存在时自动创建（默认开，见 ensure_target_db）
            if let Some(msg) = ensure_target_db(&state, &tgt, &opts).await {
                task.log(msg);
            }
            // ===== 对象级**并行**同步 =====
            // 单对象内部已经是「读一页写一页」的预取流水线，多对象再并行同时搬 ——
            // 并行度**自适应**（见 adaptive_concurrency）：按目标库类型与对象数动态定，
            // 不写死。计数与日志都走 Task 的锁内计数器，并发安全；结果按**提交顺序**
            // 回填槽位，界面上的结果表顺序稳定。
            let target_kind_key = require_record(&state, &tgt.conn)
                .await
                .map(|record| record.kind().key().to_string())
                .unwrap_or_default();
            let sync_concurrency = adaptive_concurrency(&target_kind_key, total);
            let slots: Vec<std::sync::Arc<std::sync::Mutex<Option<Value>>>> = objects
                .iter()
                .map(|_| std::sync::Arc::new(std::sync::Mutex::new(None)))
                .collect();
            let counters = std::sync::Arc::new(std::sync::Mutex::new((
                0u64, 0u64, 0u64, 0u64, 0u64,
            )));
            let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(sync_concurrency));
            let mut handles = Vec::new();
            for (index, (kind, name)) in objects.iter().enumerate() {
                let state = state.clone();
                let src = src.clone();
                let tgt = tgt.clone();
                let opts = opts.clone();
                let task = task.clone();
                let kind = kind.clone();
                let name = name.clone();
                let slot = slots[index].clone();
                let counters = counters.clone();
                let sem = sem.clone();
                handles.push(tokio::spawn(async move {
                    // 遇错停止 / 取消：还没开跑的对象不再执行（已开跑的照常跑完当前页）
                    let blocked = task.is_canceled()
                        || (opts.stop_on_error && {
                            let (_, _, errors, _, _) = *counters.lock().unwrap();
                            errors > 0
                        });
                    let permit = tokio::time::timeout(PIPE_WAIT, sem.acquire_owned()).await.ok().and_then(|r| r.ok());
                    if blocked {
                        if task.is_canceled() {
                            *slot.lock().unwrap() = Some(json!({
                                "type": kind, "name": name, "status": "canceled",
                                "message": "已取消", "inserted": 0, "updated": 0,
                            }));
                        } else {
                            *slot.lock().unwrap() = Some(json!({
                                "type": kind, "name": name, "status": "skipped",
                                "message": "遇错停止：未执行", "inserted": 0, "updated": 0,
                            }));
                            task.log(format!("{}：遇错停止，未执行", name));
                        }
                        task.add_done(1);
                        drop(permit);
                        return;
                    }
                    task.set_phase(name.to_string());
                    let outcome = match kind.as_str() {
                        "table" => {
                            sync_table(&state, Some(&task), &src, &tgt, &name, &name, &opts).await
                        }
                        "view" => sync_view(&state, &src, &tgt, &name, &opts).await,
                        // 例程 / 触发器 / 事件：与视图同一条路（见 sync_object_source）
                        "procedure" | "function" | "trigger" | "event" => {
                            sync_object_source(&state, &src, &tgt, &kind, &name, &opts).await
                        }
                        other => ObjResult::skipped(other, &name, "该对象类型不支持同步（结构、数据都没动）"),
                    };
                    {
                        let mut c = counters.lock().unwrap();
                        match outcome.status.as_ref() {
                            "ok" => c.0 += 1,
                            "skipped" => c.1 += 1,
                            _ => c.2 += 1,
                        }
                        c.3 += outcome.inserted;
                        c.4 += outcome.updated;
                    }
                    if !outcome.message.is_empty() {
                        // 日志带对象类型前缀（表/视图/存储过程…），与各步骤日志「表 xxx：…」一致；
                        // 存储过程/函数/触发器/事件沿用中文叫法
                        let kind_label = match outcome.ty.as_str() {
                            "table" => "表",
                            "view" => "视图",
                            "procedure" => "存储过程",
                            "function" => "函数",
                            "trigger" => "触发器",
                            "event" => "事件",
                            other => other,
                        };
                        task.log(format!("{kind_label} {name}：{}", outcome.message));
                    }
                    *slot.lock().unwrap() = Some(outcome.to_json());
                    task.add_done(1);
                    drop(permit);
                }));
            }
            for handle in handles {
                let _ = tokio::time::timeout(PIPE_WAIT, handle).await;
            }
            let (ok, skipped, errors, inserted, updated) = *counters.lock().unwrap();
            let results: Vec<Value> = slots
                .iter()
                .filter_map(|slot| slot.lock().unwrap().take())
                .collect();
            let _ = task.is_canceled();
            let summary = json!({
                "total": total,
                "ok": ok,
                "skipped": skipped,
                "cancelled": if task.is_canceled() { 1 } else { 0 },
                "errors": errors,
                "inserted": inserted,
                "updated": updated,
            });
            // 取消也是有结果的结果：文案上要和「完成」分开，
            // 否则界面会拿一句"同步完成"去报一个被取消的任务。
            // 行数不进文案 —— 进度卡（读取/传输/失败）平滑递增就是账本
            let message = format!(
                "同步{}：成功 {ok} / 跳过 {skipped} / 失败 {errors}",
                if task.is_canceled() { "已停止" } else { "完成" }
            );
            task.set_message(message.clone());
            Ok(Some(json!({
                "success": errors == 0,
                "summary": summary,
                "results": results,
                "message": message,
            })))
        }
    });

    Ok(Json(json!({
        "success": true,
        "taskId": task.id,
        "status": "running",
        "done": 0,
        "total": total,
        "current": "",
    })))
}

/// `GET /api/sync/task/{taskId}` —— 进度 + 逐对象结果。
pub async fn status(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    let Some(task) = state.tasks.get(&task_id) else {
        return Ok(Json(json!({
            "success": false,
            "status": "notfound",
            "message": "任务不存在或已过期（任务只保留 30 分钟）",
        })));
    };
    let mut view = task.snapshot();
    view["success"] = json!(true);
    view["current"] = view["phase"].clone();
    if let Some(result) = task.result() {
        view["result"] = result;
    }
    Ok(Json(view))
}

/// `POST /api/sync/cancel/{taskId}` —— 请求取消。
pub async fn cancel(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    match state.tasks.get(&task_id) {
        Some(task) => {
            task.cancel();
            Ok(Json(json!({ "success": true, "message": "已请求取消，当前对象处理完后停止" })))
        }
        None => Ok(Json(json!({ "success": false, "message": "任务不存在或已过期" }))),
    }
}
