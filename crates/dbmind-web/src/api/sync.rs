//! `/api/sync/*` —— 数据同步（把源对象的**结构**与**数据**搬到目标）。
//!
//! ## 两条入口
//!
//! | 入口 | 场景 | 形态 |
//! |---|---|---|
//! | `POST /api/sync` | 单表同步 | 同步返回 `{inserted, updated, skipped, errors, …}` |
//! | `POST /api/sync/db` | 多对象同步 | 异步任务（进度 + 逐对象结果 + 可取消） |
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
use crate::api::meta::run_sql_in;
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
/// 进度状态最短刷新间隔：写一页更新一次就够，**不必每行**都更新
/// （20 万行就是 20 万次加锁 + 20 万次前端快照变化，纯属白烧 CPU）。
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(300);

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
    fn scoped(&self, table: &str) -> String {
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
        if prefix.is_empty() || table.contains('.') {
            table.to_string()
        } else {
            format!("{prefix}.{table}")
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
        let qualified = side.scoped(table);
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

async fn run_sql(state: &AppState, side: &Side, sql: &str) -> XResult<()> {
    let target = crate::api::scope::resolve(state, &side.conn, &side.scope).await?;
    run_target(state, &target, sql).await.map(|_| ())
}

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
            format!("if db_id(N'{}') is null create database [{}]", db_name, db_name)
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
async fn run_target(state: &AppState, target: &str, sql: &str) -> XResult<dbmind_core::QueryResult> {
    let engine = state.engine();
    let request = dbmind_core::QueryRequest {
        read_only: None,
        connection: target.to_string(),
        sql: sql.to_string(),
        options: dbmind_core::QueryOptions {
            max_rows: 1,
            timeout_ms: 600_000,
        },
        execution_id: None,
        session: Some("internal:browse".to_string()),
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
async fn fetch_page(
    state: &AppState,
    conn: &str,
    scope: &str,
    select_base: &str,
    dialect: Dialect,
    offset: u64,
    cursor: Option<(&[String], &[CellValue])>,
    extra_where: &str,
) -> XResult<Vec<Vec<CellValue>>> {
    // 用户的行过滤条件：拼进 SELECT 的 WHERE。与 keyset 游标共存时用 and 组合
    //（keyset 分支自己要写 where，两者条件相与语义才对）。
    let filtered = |base: &str, cond: &str| {
        if extra_where.is_empty() {
            base.to_string()
        } else {
            format!("{base} where ({extra_where}){}", if cond.is_empty() { String::new() } else { format!(" and ({cond})") })
        }
    };
    let sql = match cursor {
        Some((keys, last)) if !keys.is_empty() && last.len() == keys.len() => {
            let order = keys
                .iter()
                .map(|key| dialect.quote(key))
                .collect::<Vec<_>>()
                .join(", ");
            let mut conditions: Vec<String> = Vec::new();
            for depth in 0..keys.len() {
                let mut parts: Vec<String> = Vec::new();
                for (position, key) in keys.iter().enumerate() {
                    let quoted = dialect.quote(key);
                    let value = literal_for(&last[position], dialect, None);
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
                dialect.limit_clause(0, PAGE)
            )
        }
        _ => {
            let page = dialect.limit_clause(offset, PAGE);
            if dialect.needs_order_by_for_paging() {
                format!("{} order by (select null) {page}", filtered(select_base, ""))
            } else {
                format!("select * from ({}) dbmind_page {page}", filtered(select_base, ""))
            }
        }
    };
    let result = run_sql_in(state, conn, scope, sql, PAGE as usize).await?;
    Ok(result.rows)
}

/// 拼「读取某张表某些列」的 select 前缀（翻页时反复用它）。
fn select_base(side: &Side, table: &str, columns: &[String], dialect: Dialect) -> String {
    let list = columns
        .iter()
        .map(|name| dialect.quote(name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("select {list} from {}", side.scoped(table))
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
        let show_sql = format!("show index from {}", src.scoped(table));
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
                let unique = row.get("non_unique").and_then(Value::as_i64) == Some(0)
                    || row.get("unique").and_then(Value::as_bool) == Some(true)
                    || row.get("isUnique").and_then(Value::as_i64) == Some(1);
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

/// 单格 → SQL 字面量：目标列是**真布尔**时走布尔字面量。
///
/// 为什么需要：内核把布尔表示为 0/1 整数，而 PG / Oracle 的 boolean 列不接受整数 `0`
/// （报"列是 boolean，表达式是 integer"）—— 跨类型建表时我们自己就会给目标建出 boolean 列，
/// 所以这条必须跟上。
fn literal_for(cell: &CellValue, dialect: Dialect, target_type: Option<&str>) -> String {
    if let (CellValue::Integer(value), Some(type_name)) = (cell, target_type) {
        if Dialect::is_boolean_type(type_name) {
            return dialect.bool_literal(*value != 0).to_string();
        }
    }
    sql_literal(cell, dialect)
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

/// 生成一批 INSERT。SQL Server 的多行 VALUES **单条语句限 1000 行**（解析器硬限制），
/// 但一批里可以放多条 INSERT（连接开了 allowMultiQueries、安全闸门放行 INSERT 批）——
/// 超过 1000 行时拆成多条拼分号，批大小不受限，固定链路开销也不会因为拆语句而翻倍。
fn build_insert_batch(
    table: &str,
    columns: &[String],
    types: &[Option<String>],
    rows: &[Vec<CellValue>],
    dialect: Dialect,
    kind_key: &str,
) -> String {
    const SQLSERVER_VALUES_LIMIT: usize = 1000;
    if kind_key == "sqlserver" && rows.len() > SQLSERVER_VALUES_LIMIT {
        return rows
            .chunks(SQLSERVER_VALUES_LIMIT)
            .map(|chunk| build_insert(table, columns, types, chunk, dialect))
            .collect::<Vec<_>>()
            .join(";\n");
    }
    build_insert(table, columns, types, rows, dialect)
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

fn build_update(
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
fn key_of(row: &[CellValue], indexes: &[usize]) -> String {
    crate::api::shape::row_key(row, indexes)
}

/// 同步一张表（外层：MySQL 系目标的事务包裹）。
///
/// autocommit 模式下**每条 INSERT 都刷一次盘**（实测 2000 行/条要 1.2 秒，大头是 fsync）——
/// 把整张表的写入包进一个事务，只在结尾 commit 一次，写入吞吐能翻数倍。
/// 语义保持：成功/失败/取消都 **commit**（「已写入的部分保留，不回滚」是既有约定，
/// 失败时保留的正是事务里已执行成功的那些行）。
/// 仅 MySQL / MariaDB 启用（Doris 的事务没有实际意义；其它方言各有自己的提交语义）。
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
    let use_txn = match require_record(state, &tgt.conn).await {
        Ok(record) => {
            opts.sync_data && matches!(Dialect::new(record.kind()).kind.key(), "mysql" | "mariadb")
        }
        Err(err) => return ObjResult::failed("table", table, err.message),
    };
    if !use_txn {
        return sync_table_inner(state, task, src, tgt, table, target_table, opts).await;
    }
    let Ok(tgt_target) = crate::api::scope::resolve(state, &tgt.conn, &tgt.scope).await else {
        return ObjResult::failed("table", table, "解析目标连接失败");
    };
    if let Some(task) = task {
        task.log(format!("表 {table}：开启事务（整表一次提交）"));
    }
    if let Err(err) = run_target(state, &tgt_target, "set autocommit = 0").await {
        // 开不了事务就退回 autocommit 逐条写入 —— 慢，但功能不缺
        if let Some(task) = task {
            task.log(format!("表 {table}：开启事务失败，退回逐条提交（{}）", err.message));
        }
        return sync_table_inner(state, task, src, tgt, table, target_table, opts).await;
    }
    let result = sync_table_inner(state, task, src, tgt, table, target_table, opts).await;
    // 无论成功/失败/取消都 commit：与「已写入的部分保留」的既有语义一致
    let _ = run_target(state, &tgt_target, "commit").await;
    let _ = run_target(state, &tgt_target, "set autocommit = 1").await;
    if let Some(task) = task {
        task.log(format!("表 {table}：提交事务"));
    }
    result
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
    let target_kind = crate::api::dialect::Dialect::new(
        match require_record(state, &tgt.conn).await {
            Ok(record) => record.kind(),
            Err(err) => return ObjResult::failed(ty, table, err.message),
        },
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
            return ObjResult::failed(ty, table, format!("建表失败：{}", err.message));
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
        let post_comments = target_kind.post_create_comments(
            target_name.trim_matches(|c| c == '"' || c == '`' || c == ']'),
            &src_column_comments,
            src_table_comment.as_deref(),
            "dbo",
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
    let base_select = select_base(src, table, &columns, source_kind);

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
    let mut last_report = started;
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
                match fetch_page(state, &tgt.conn, &tgt.scope, &base, target_kind, key_offset, cursor, "")
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
            let where_sql = if opts.where_clause.is_empty() {
                src.scoped(table)
            } else {
                format!("{} where {}", src.scoped(table), opts.where_clause)
            };
            let insert_sql = format!(
                "insert into {target_name} ({col_list_tgt}) select {col_list_src} from {where_sql}"
            );
            // 直拷同样要过 identity：同批 SET 开/关（会话级开关必须与写入同一批）
            let insert_sql = with_identity(
                insert_sql,
                target_kind.kind.key() == "sqlserver",
                &target_name,
            );
            let copy_result = run_target(state, &tgt_target, &insert_sql).await;
            let mut copied = match &copy_result {
                // INSERT … SELECT 的影响行数就是传输行数（比再发一条 count 快且准）
                Ok(result) => result.affected_rows.unwrap_or(0) as u64,
                Err(err) => return ObjResult::failed(ty, table, format!("同实例直拷失败：{}", err.message)),
            };
            // MySQL 驱动对 INSERT…SELECT 可能回 0（数据实际已写入，真机：目标 3 行、
            // affected=0）—— 影响行数不可信时回查目标表行数兜底。
            // 计数单元格可能是 Integer 也可能是 Text（驱动的 BIGINT 回传差异），两种都认。
            if copied == 0 {
                if let Ok(result) = run_target(state, &tgt_target, &format!("select count(*) as n from {target_name}")).await {
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
    let mut pending_insert: Vec<Vec<CellValue>> = Vec::new();

    // 主键游标（keyset）分页：有主键时替代 OFFSET —— 深分页每页只扫本页，

    // 千万级行不再 O(N²) 退化。无主键退回 OFFSET（cursor 传 None）。

    let paging_keys: Vec<String> = key_indexes.iter().map(|i| columns[*i].clone()).collect();



    // ===== 读取/写入流水线 =====

    // 原来是「读一页 → 写一页」串行：总耗时 = 读取 + 写入。

    // 预取协程提前拉页（最多在途 4 页），主循环边收边写 —— 总耗时 ≈ max(读取, 写入)。
    // 在途 2 → 4：写入侧偶有慢批（大行宽/远端库），缓冲更深时读协程不会空转等待，
    // 读写流水线咬合更紧；内存代价 ≈ 多缓存 2 页行数据（可控）。

    // 取消：主循环退出（drop rx）后，预取协程的 send 失败而退出（最多多拉几页在途，无害）。

    enum PageMsg {

    Page(Vec<Vec<CellValue>>),

    Failed(String),

    }

    let (page_tx, mut page_rx) = tokio::sync::mpsc::channel::<PageMsg>(4);

    let prefetch_state = state.clone();

    let prefetch_src = src.clone();

    let prefetch_base = base_select.clone();

    let prefetch_keys = paging_keys.clone();

    let prefetch_key_positions = key_indexes.clone();

    let prefetch_kind = source_kind;

    let prefetch_where = opts.where_clause.clone();

    let prefetch_task = tokio::spawn(async move {

        let mut offset = 0u64;

        let mut last_key: Option<Vec<CellValue>> = None;

        loop {

            let cursor = match last_key.as_ref() {

                Some(values) if !prefetch_keys.is_empty() => {

                    Some((prefetch_keys.as_slice(), values.as_slice()))

                }

                _ => None,

            };

            match fetch_page(

                &prefetch_state,

                &prefetch_src.conn,

                &prefetch_src.scope,

                &prefetch_base,

                prefetch_kind,

                offset,

                cursor,

                &prefetch_where,

            )

            .await

            {

                Ok(rows) => {

                    let got = rows.len();

                    if !prefetch_keys.is_empty() {

                        last_key = rows.last().map(|row| {

                            prefetch_key_positions

                                .iter()

                                .filter_map(|i| row.get(*i).cloned())

                                .collect::<Vec<CellValue>>()

                        });

                    }

                    if page_tx.send(PageMsg::Page(rows)).await.is_err() {

                        break;

                    }

                    if got < PAGE as usize {

                        break;

                    }

                    offset += PAGE;

                }

                Err(err) => {

                    let _ = page_tx.send(PageMsg::Failed(err.message)).await;

                    break;

                }

            }

        }

    });



    // 行数上限的消费计数（按真正进入写入处理的行数累计）+ 是否已命中上限
    let mut consumed: u64 = 0;
    let mut limit_hit = false;
    let mut canceled = false;

    let mut read_error: Option<String> = None;

    'outer: while let Some(msg) = page_rx.recv().await {

        match msg {

            PageMsg::Failed(err) => {

                read_error = Some(err);

                break 'outer;

            }

            PageMsg::Page(page) => {

                // 取消是协作式的：**每页**检查一次（原来每张表只有开头一次机会）

                if let Some(task) = task {

                    if task.is_canceled() {

                        canceled = true;

                        break 'outer;

                    }

                }

                let got = page.len();

                if got == 0 {

                    break;

                }


                if let Some(task) = task {

                    // 每页一条日志太刷屏（大表几百条），读取量在进度卡上平滑递增就够

                    task.add_rows_read(got as u64);

                }

                // 行数上限：本页按**剩余配额**截断 —— 预取协程可能整页拉回来，
                // 多出来的行直接丢弃；截断即命中上限，处理完这页就收（不再拉下一页）。
                // 消费计数（consumed）在下面的行循环里累加。
                let mut page = page;

                if opts.row_limit > 0 {

                    let remaining = opts.row_limit.saturating_sub(consumed);

                    if remaining == 0 {

                        break 'outer;

                    }

                    if got as u64 > remaining {

                        page.truncate(remaining as usize);

                        note.push_str(&format!(

                            "（已达行数上限 {}，仅同步了前 {} 行）",

                            opts.row_limit, opts.row_limit

                        ));

                        limit_hit = true;

                    }

                }
        for row in page {
            consumed += 1;
            let is_existing = effective_mode != DataMode::Insert
                && !key_indexes.is_empty()
                && existing.contains(&key_of(&row, &key_indexes));
            if is_existing {
                // 更新逐行做：跨方言的批量 UPDATE 没有统一写法，硬拼会踩到方言差异
                let sql =
                    build_update(&target_name, &columns, &target_types, &row, &key_indexes, target_kind);
                match run_target(state, &tgt_target, &sql).await {
                    Ok(_) => {
                        updated += 1;
                        if let Some(task) = task { task.add_rows_written(1); }
                    }
                    Err(err) => {
                        if let Some(task) = task { task.add_rows_failed(1); }
                        return ObjResult::failed(ty, table, format!("更新失败：{}", err.message))
                    }
                }
                continue;
            }
            pending_insert.push(row);
            if pending_insert.len() as u64 >= per_statement {
                let batch_rows = pending_insert.len() as u64;
                match run_sql(
                    state,
                    tgt,
                    &with_identity(
                        build_insert_batch(&target_name, &columns, &target_types, &pending_insert, target_kind, target_kind.kind.key()),
                        identity_on,
                        &target_name,
                    ),
                )
                .await
                {
                    Ok(()) => {
                        inserted += batch_rows;
                        if let Some(task) = task {
                            task.add_rows_written(batch_rows);
                        }
                        pending_insert.clear();
                    }
                    Err(err) => {
                        if let Some(task) = task { task.add_rows_failed(batch_rows); }
                        return ObjResult::failed(ty, table, format!("插入失败：{}", err.message))
                    }
                }
            }
        }
        // **跨页攒批**：残余批次不在这里落盘，攒满 per_statement 才发一条 ——
        // 页尾 flush 会把 ClickHouse 的 5 万行大批次打碎成 1 万行/次（每次落一个 part），
        // 那是 CH 目标慢的主因。攒批期间「已写行数」略滞后于真实落盘，无实际影响。
        // 进度**按时间节流**：写一页更新一次足矣。旧实现每行都写一次 phase，
        // 20 万行就是 20 万次加锁 + 20 万次前端快照变化 —— 那是纯白烧的 CPU。
        if let Some(task) = task {
            if last_report.elapsed() >= PROGRESS_INTERVAL {
                // 只报「执行到哪一步」，不报数字 —— 数字在进度卡上平滑递增
                task.set_phase(format!("表 {table}：传输数据"));
                last_report = std::time::Instant::now();
            }
        }
                if got < PAGE as usize {
                    break;
                }
                // 上限已命中（本页被截断）：处理完这页就收，不再继续拉
                if limit_hit {
                    break 'outer;
                }
            }
        }
    }
    // 流水线收尾：丢掉 rx 让预取协程退出，并等它结束
    drop(page_rx);
    let _ = prefetch_task.await;
    if canceled {
        note.push_str("（已取消：已写入的部分保留，不回滚）");
    }
    if let Some(err) = read_error {
        return ObjResult::failed(ty, table, format!("读取源数据失败：{}", err));
    }
    // 因取消而中途退出时，手里可能还压着最后一批没落盘的插入
    if !pending_insert.is_empty() {
        match run_sql(
            state,
            tgt,
            &with_identity(
                build_insert_batch(&target_name, &columns, &target_types, &pending_insert, target_kind, target_kind.kind.key()),
                identity_on,
                &target_name,
            ),
        )
        .await
        {
            Ok(()) => {
                inserted += pending_insert.len() as u64;
                if let Some(task) = task { task.add_rows_written(pending_insert.len() as u64); }
            }
            Err(err) => {
                if let Some(task) = task { task.add_rows_failed(pending_insert.len() as u64); }
                return ObjResult::failed(ty, table, format!("插入失败：{}", err.message));
            }
        }
    }
    // identity 的 OFF 已随每批写入自带（见 with_identity），无需单独关闭

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
    let Some(ddl_sql) = source_kind.ddl(name).sql() else {
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
    let Some(sql) = source_kind.object_source(kind, name).sql() else {
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
                    let permit = sem.acquire_owned().await.ok();
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
                        task.log(format!("{}：{}", name, outcome.message));
                    }
                    *slot.lock().unwrap() = Some(outcome.to_json());
                    task.add_done(1);
                    drop(permit);
                }));
            }
            for handle in handles {
                let _ = handle.await;
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
