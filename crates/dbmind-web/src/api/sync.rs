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

use crate::api::dialect::{Dialect, DirectRefDepth};
use crate::api::error::{XError, XResult};
use crate::api::export::sql_literal;
use crate::api::meta::{run_sql_in, run_sql_in_keyed, run_sql_in_write};
use crate::api::transfer::spec::{ExprLimit, Savepoint};
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
/// 写侧由 `insert_batch_ranges` 按目标方言的行数、表达式数与字节数独立切分。
#[cfg(test)]
const PAGE: u64 = 50_000;
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
    /// 同时同步几个对象（并发表数）。
    ///
    /// 缺省 = 自适应（按目标/源类型与对象数定，见 `adaptive_concurrency`）；显式给了就
    /// 以用户为准（夹到 1..=32）。为手机器弱/源目标同实例时，把它调小（如 4）能直接
    /// 降低双方的连接与 IO 争抢。
    #[serde(default)]
    max_concurrency: Option<usize>,
    /// 每表**读**并发（片数）：缺省 / 0 = 自动（按源库类型与并发表数切预算）。
    ///
    /// 显式给了就以用户为准（夹到 1..=64）：既决定开工时的片数，也**封死尾部弹性**
    /// 的上限 —— 用户填 1 就该始终是一条读连接，不能被「别的表退出」偷偷扩成 6 片。
    #[serde(default)]
    src_slices: Option<usize>,
    /// 每表**写**并发（路数）：缺省 / 0 = 自动（按目标库类型与并发表数切预算）。
    #[serde(default)]
    tgt_workers: Option<usize>,
    /// 跳过目标端 binlog（`SET sql_log_bin = 0`）：写放大减半，但目标库若参与
    /// 主从复制，从库会缺这批数据 —— 界面默认不勾。
    #[serde(default)]
    skip_binlog: Option<bool>,
    /// 单条 INSERT 的**行数**上限；0 / 缺省 = 按目标方言默认（见 `insert_rows_default`）。
    ///
    /// 界面上会按目标库把默认值填出来、允许用户改（每种库的甜点值差一个数量级：
    /// ClickHouse 写 part 要 1 万行，MySQL 在丢包链路上 1000 行更稳）。
    /// `batchSize` 是旧客户端的写法，一并认。
    #[serde(default)]
    insert_rows: Option<usize>,
    #[serde(default)]
    batch_size: Option<usize>,
    /// 单次数据库操作的超时（秒），对齐 DBX 的「操作超时」：
    /// - 缺省 / 0 = **继承连接**（沿用内核默认 600 秒，驱动自己的 socketTimeout 优先）
    /// - 负数 = 不限制（内部按 24 小时兜底，防进程被无限挂住）
    /// - 正数 = 该秒数
    ///
    /// 注意它只决定「多久没响应算超时」，**不决定「慢就中断」**：到点前每次探活通过都会
    /// 继续等（见 `run_write_stmt` / `transfer::engine::wait_write`）。
    #[serde(default)]
    timeout_seconds: Option<i64>,
    /// 行数上限：>0 时每张表只搬前 N 行；0 或缺省 = 不限。
    /// **界面已不再提供**（用户口径：同步就是不限，抽样不是同步的事），保留字段只是
    /// 为了老客户端 / 脚本调用仍能编译 —— 新调用方不要传。
    #[serde(default)]
    row_limit: Option<u64>,
    /// 限速：每秒最多搬多少**行**（0 = 不限）。与 `rate_limit_bytes` 取更慢的那条曲线。
    #[serde(default)]
    rate_limit_rows: Option<u64>,
    /// 限速：每秒最多搬多少**字节**（0 = 不限）。界面按 MB/s 填，前端换算成字节再提交。
    #[serde(default)]
    rate_limit_bytes: Option<u64>,
    /// 目标表名映射（**源表名 → 目标表名**）：没列到的表同名，留空也算同名。
    ///
    /// 目标表不存在时按新名建表，索引/注释/写入/日志全部跟着新名走 —— 换名是在
    /// 计划生成前一次性替换的，不是到最后写库才换（那样建表会在旧名下多出一张表）。
    #[serde(default)]
    table_renames: Option<std::collections::HashMap<String, String>>,
    /// 前置 SQL：**结构处理完、开始搬数据前**在目标连接上执行（DataX 的 preSql）。
    /// 多条用「引号之外的分号或换行」分隔，见 [`split_user_sql`]。
    #[serde(default)]
    pre_sql: Option<String>,
    /// 后置 SQL：本对象**数据搬完后**在目标连接上执行（DataX 的 postSql）。
    #[serde(default)]
    post_sql: Option<String>,
    /// 行过滤：**按表设置**，键是表名（与 `tables` 里的一致），值是拼到该表源侧 SELECT
    /// 的 WHERE 后面的条件。没列到的表 = 全量同步。
    ///
    /// 为什么必须按表：一份全局条件在多表同步里语义不成立 —— `amount > 100` 拼到
    /// 没有 amount 列的表上，整张表直接报错（真机：一次 339 张表同步，除了几张订单表
    /// 全挂）。所以界面上是一张表一个输入框。
    #[serde(default)]
    where_clauses: Option<std::collections::HashMap<String, String>>,
    /// 旧的**全局**行过滤：拼到所有表的 SELECT 上。界面已移除（见 `where_clauses`），
    /// 保留只为兼容老客户端与单表接口。
    #[serde(default)]
    where_clause: Option<String>,
    // batch_size 已删除：写入批次由后端按目标方言自动决定（见 insert_batch_ranges），
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
    /// 限速表（行/秒 + 字节/秒，取更慢的曲线）；`None` = 不限速。
    ///
    /// 整库同步时**所有对象共用同一个** —— 限速是「整条链路」的意愿，不是每张表各一份
    /// （否则 10 张表并发就是 10 倍速率）。
    pacer: Option<std::sync::Arc<crate::api::transfer::pacer::Pacer>>,
    /// 目标表名映射（源表名 → 目标表名）
    table_renames: std::collections::HashMap<String, String>,
    /// 前置 / 后置 SQL（目标连接上执行；空串 = 不执行）
    pre_sql: String,
    post_sql: String,
    /// 按表设置的行过滤（表名 → WHERE）；缺省的表走 `where_clause`
    where_by_table: std::collections::HashMap<String, String>,
    /// 全局行过滤（旧客户端 / 单表接口），界面已不再产生
    where_clause: String,
    /// 本表最多几路**写**会话（0 = 方言默认值）。由整库接口按「对象并发数」切分预算，
    /// 见 `TablePlan::workers_cap`
    tgt_workers: usize,
    /// 本表最多几片**读**会话（0 = 方言默认值），同上
    src_slices: usize,
    /// 尾部弹性的并发配额（整库同步才有；单表同步为 `None`）：
    /// 别的对象结束后调度侧调高它，已在跑的表的引擎就地补令牌扩片
    grow: Option<std::sync::Arc<crate::api::transfer::GrowCaps>>,
    /// 单条 INSERT 的行数上限（0 = 按目标方言默认，见 [`insert_rows_default`]）
    insert_rows: usize,
    /// 单次数据库操作超时（秒）：0 = 继承连接，<0 = 不限制，>0 = 该秒数
    op_timeout_secs: i64,
    /// 跳过目标端 binlog（界面勾选 / 环境变量 `DBMIND_SYNC_SKIP_BINLOG`）
    skip_binlog: bool,
}

/// 操作超时（秒）换算成毫秒 —— 界面上只有一个「超时时间」旋钮（对齐 DBX），
/// 内部所有「多久没响应算超时」都以它为准：
/// - `0`（继承连接）：沿用内核默认 600 秒（驱动上的 socketTimeout 仍然优先，那个我们改不了）
/// - `<0`（不限制）：按 24 小时兜底，只为防止进程被一条永远不回来的语句挂死
/// - `>0`：就是用户填的秒数
///
/// **它是上限，不是「到点就中断」**：到点前每轮探活通过都会继续等（见 `run_write_stmt`）。
pub(crate) fn op_timeout_ms(secs: i64) -> u64 {
    match secs {
        s if s > 0 => (s as u64).saturating_mul(1000).min(86_400_000),
        s if s < 0 => 86_400_000,
        _ => 600_000,
    }
}

impl Opts {
    /// 这张表该用哪条行过滤：**先按表精确匹配**（界面就是按表设置的），
    /// 没有再回退到全局条件（旧客户端 / 单表接口），都没有就是不过滤。
    fn where_for(&self, table: &str) -> String {
        if let Some(clause) = self.where_by_table.get(table) {
            return clause.clone();
        }
        // 表名可能带库/模式前缀（`库.表`、`库.模式.表`）：末段再匹配一次
        if let Some(last) = table.rsplit('.').next() {
            if last != table {
                if let Some(clause) = self.where_by_table.get(last) {
                    return clause.clone();
                }
            }
        }
        self.where_clause.clone()
    }

    /// 这张表要写到目标端的**哪个表名**（界面「表名映射」页签）。
    ///
    /// 键是**源表名**（界面按源表名列输入框），匹配不到就看 `fallback`（单表接口可以用
    /// 显式传入的目标表名）；都没有 = 同名。改名在这里一次性定死，后面建表、建索引、
    /// 写库、日志全部用新名 —— 否则会出现「结构建在新名下、数据写进旧表」这种半拉子结果。
    fn target_table_for(&self, table: &str, fallback: &str) -> String {
        let pick = |name: &str| -> Option<String> {
            self.table_renames.get(name).map(|to| to.trim().to_string())
        };
        // 先按完整表名，再按末段（表名可能带 `库.` 前缀）
        if let Some(to) = pick(table).filter(|to| !to.is_empty()) {
            return to;
        }
        if let Some(last) = table.rsplit('.').next() {
            if last != table {
                if let Some(to) = pick(last).filter(|to| !to.is_empty()) {
                    return to;
                }
            }
        }
        fallback.to_string()
    }

    /// 设定本表的会话预算（整库同步按并发对象数切分，见 `db()`）。
    fn with_session_caps(mut self, tgt_workers: usize, src_slices: usize) -> Self {
        self.tgt_workers = tgt_workers;
        self.src_slices = src_slices;
        self
    }

    /// 挂上尾部弹性配额（见 [`GrowCaps`]）：整库同步的任务里所有表共用同一份。
    fn with_grow(mut self, grow: std::sync::Arc<crate::api::transfer::GrowCaps>) -> Self {
        self.grow = Some(grow);
        self
    }
}

/// 用户手写的 WHERE 归一：去首尾空白、剥掉尾部分号（防多语句）；空白条件 = 未设置。
/// 注解（`--` / `/* */`）不在这里处理，由执行器按方言各自处理。
fn normalize_where(raw: &str) -> String {
    raw.trim().trim_end_matches(';').trim().to_string()
}

/// 把用户填的前置/后置 SQL 脚本切成一条条语句。
///
/// 分隔符是**引号之外**的分号或换行（界面提示「多条用分号/换行分隔」）；`--` 行注释整行
/// 丢弃。为什么不能简单 `split(';')`：`insert into t values ('a;b')` 里的分号是数据，
/// 切开会把一条正常语句切成两条语法错误 —— 用户写的脚本在我们这里被切坏，比不执行还糟。
fn split_user_sql(script: &str) -> Vec<String> {
    let mut statements: Vec<String> = Vec::new();
    let mut current = String::new();
    // 当前是否在引号里（'…' / "…" / `…`）；引号内的分号与换行都是内容
    let mut quote: Option<char> = None;
    let mut chars = script.chars().peekable();
    while let Some(ch) = chars.next() {
        if quote.is_none() {
            // 行注释：`--` 之后到行尾都不发送（用户常用来写备注）
            if ch == '-' && chars.peek() == Some(&'-') {
                while let Some(next) = chars.next() {
                    if next == '\n' {
                        break;
                    }
                }
                push_user_stmt(&mut statements, &mut current);
                continue;
            }
        }
        match quote {
            Some(open) => {
                current.push(ch);
                if ch == open {
                    quote = None;
                }
            }
            None => match ch {
                '\'' | '"' | '`' => {
                    quote = Some(ch);
                    current.push(ch);
                }
                ';' | '\n' | '\r' => push_user_stmt(&mut statements, &mut current),
                _ => current.push(ch),
            },
        }
    }
    push_user_stmt(&mut statements, &mut current);
    statements
}

/// 收一条语句：去掉首尾空白，空语句（连续分号、纯空白行）直接丢。
fn push_user_stmt(out: &mut Vec<String>, current: &mut String) {
    let sql = current.trim().trim_end_matches(';').trim();
    if !sql.is_empty() {
        out.push(sql.to_string());
    }
    current.clear();
}

/// 在**目标连接**上按顺序执行用户 SQL（前置/后置），失败返回带序号的人话。
///
/// 为什么走「可写会话」（[`run_target_write`]）：用户脚本多数是清表、改约束、写打点，
/// 目标连接若被标成只读，普通会话会被驱动直接拒绝执行。
async fn run_user_sql(state: &AppState, target: &str, script: &str, label: &str) -> Result<(), String> {
    let statements = split_user_sql(script);
    for (index, sql) in statements.iter().enumerate() {
        if let Err(err) = run_target_write(state, target, sql).await {
            // 只报数据库/驱动的原话（哪一条、什么标签进 tracing）
            tracing::warn!(label = label, index = index, sql = %sql, error = %err.message, "用户 SQL 执行失败");
            return Err(err.message);
        }
        tracing::debug!(label, sql = %sql, "用户 SQL 执行成功");
    }
    Ok(())
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
            // 按表过滤：空白/只剩分号的条件直接丢掉（等于该表不设过滤）
            where_by_table: self
                .where_clauses
                .iter()
                .flat_map(|map| map.iter())
                .filter_map(|(table, clause)| {
                    let cleaned = normalize_where(clause);
                    if cleaned.is_empty() {
                        None
                    } else {
                        Some((table.trim().to_string(), cleaned))
                    }
                })
                .collect(),
            where_clause: normalize_where(self.where_clause.as_deref().unwrap_or("")),
            // 0 = 自动：由 `db()` 按「并发表数」切预算；单表接口直接用方言默认。
            // 界面显式填了（1..=64）就以用户为准，见 `db()` 里的 `*_declared`。
            tgt_workers: self.tgt_workers.map(|n| n.min(64)).unwrap_or(0),
            src_slices: self.src_slices.map(|n| n.min(64)).unwrap_or(0),
            grow: None,
            // 0 = 让每张表按**目标方言**的默认值来（界面填了就以此为准）；
            // 夹到 1..=100_000：太小只剩往返开销，太大等于把报文重新放大。
            insert_rows: self
                .insert_rows
                .or(self.batch_size)
                .unwrap_or(0)
                .min(100_000),
            // 夹到 [-1, 86400]：负数是「不限制」，0 是「继承连接」，正数最多一天
            op_timeout_secs: self
                .timeout_seconds
                .unwrap_or(0)
                .clamp(-1, 86_400),
            // 界面勾选优先；环境变量是排查/批处理用的逃生阀（两者任一为真即开）
            skip_binlog: self.skip_binlog.unwrap_or(false)
                || std::env::var_os("DBMIND_SYNC_SKIP_BINLOG").is_some(),
            // 两条限速曲线（0 = 该曲线不限），全任务共用一份时间表
            pacer: crate::api::transfer::pacer::Pacer::new(
                self.rate_limit_rows.unwrap_or(0),
                self.rate_limit_bytes.unwrap_or(0),
            ),
            // 表名映射：只收「真的改成了别的名字」的条目（留空/改回原名都当没写）
            table_renames: self
                .table_renames
                .iter()
                .flat_map(|map| map.iter())
                .filter_map(|(from, to)| {
                    let from = from.trim().to_string();
                    let to = to.trim().to_string();
                    if from.is_empty() || to.is_empty() || from == to {
                        None
                    } else {
                        Some((from, to))
                    }
                })
                .collect(),
            pre_sql: self.pre_sql.clone().unwrap_or_default().trim().to_string(),
            post_sql: self.post_sql.clone().unwrap_or_default().trim().to_string(),
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

/// 取表的列（先试「模式.表」，失败退回「表」；表真的不存在时返回空而不是报错 ——
/// 「目标表不存在」是一个正常的同步前置条件，不是异常）。
///
/// **失败与「不存在」必须分开**（真机故障）：网络断掉之后，这里每次查询都会失败，
/// 旧实现把 `Err` 吞成空 Vec，上层于是报「源表不存在或没有列信息」—— 那是**我们编的**
/// 结论，用户看到的是一整屏「表不存在」，而真实报错（Communications link failure /
/// 会话不存在）一行都没有，既没法排查也没法自动重试（链路级错误才允许整表重跑）。
/// 现在：所有候选名都查失败 ⇒ 原样上抛第一条真实错误；只有**查询成功且确实是空**才返回空。
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
    let mut first_err: Option<XError> = None;
    for name in names {
        let engine = state.engine();
        let target = target.clone();
        let found = blocking(move || engine.list_columns_fresh(&target, &name)).await;
        let mut columns = match found {
            Ok(columns) => columns,
            Err(err) => {
                if first_err.is_none() {
                    first_err = Some(err);
                }
                continue;
            }
        };
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
    match first_err {
        Some(err) => Err(err),
        None => Ok(Vec::new()),
    }
}

/// 取**源表结构**，遇到「等会话槽位超时」时退避重试几次。
///
/// 为什么需要重试：读页走的是**独立亲和泳道**（`internal:sync-r-<slot>-<片>`，
/// 每条泳道恒 1 条会话，见 `transfer/reader.rs`），但「取表结构」用的是连接默认泳道 ——
/// 那条泳道的并发上限就是这条连接的会话上限（MySQL 声明 4）。同步跑到一半时，
/// 监控面板轮询、表数据页签、展开库的行数回填会把那几条占住几十秒；结构读取排队
/// 满 30 秒（`agent_driver::METADATA_POOL_WAIT`）就报「等待可用连接超时」，
/// 于是一张**毫无问题**的表被判失败（真机截图：400 对象的任务里红字满屏，
/// 而数据搬运本身一直是 36k 行/s 正常跑）。
///
/// 占位者通常只持续几秒，所以这里只对**这一种**错误重试（间隔 1s / 3s，共 3 轮）；
/// 表不存在、没权限、链路断这些错误立刻原样上抛 —— 对它们重试只会拖慢任务、
/// 还会把「真实报错」淹掉（那正是上一版要修的东西）。
async fn columns_of_retry(
    state: &AppState,
    side: &Side,
    table: &str,
) -> XResult<Vec<ColumnDetail>> {
    /// 每一轮重试前的等待（第一轮不等待）。最后一次失败就当真空闲不下来，如实报错。
    const RETRY_WAIT_MS: [u64; 3] = [0, 1_000, 3_000];
    let mut last: Option<XError> = None;
    for (round, wait_ms) in RETRY_WAIT_MS.iter().enumerate() {
        if *wait_ms > 0 {
            tracing::debug!(
                target: "dbmind::sync",
                table,
                round,
                "取源表结构的会话槽位被占满，稍后重试"
            );
            tokio::time::sleep(std::time::Duration::from_millis(*wait_ms)).await;
        }
        match columns_of(state, side, table).await {
            Ok(columns) => return Ok(columns),
            // 「等待可用连接超时」是会话池自己的排队超时（不是数据库的错误），
            // 换个时刻通常就能拿到位置；其余错误立刻结束，不做无意义的等待。
            Err(err) if err.message.contains("等待可用连接超时") => last = Some(err),
            Err(err) => return Err(err),
        }
    }
    Err(last.unwrap_or_else(|| XError::internal("取源表结构失败：重试耗尽")))
}

/// 数据同步/对比的默认会话亲和键（读页、事务控制、串行写都落在这条泳道上）。
pub(crate) const SESSION_SYNC: &str = "internal:browse";

/// 目标端 DDL / 清空专用会话键 —— **整个任务共用一条物理连接**。
///
/// 为什么不能借用 [`SESSION_SYNC`]（`internal:browse`）：那条泳道的元数据会话是
/// `read_only = true`，写语句要求 `read_only = false`，两者会话键不同 ⇒ 内核给**每张表**
/// 再开一条新连接。整库 6 表并发时，DDL 连接 + 每表读写连接一起把目标连接配置的
/// 并发上限占满，日志里出现 165 次「等待可用连接超时：这条连接的并发上限是 N」，
/// 于是「删除重建失败 → 降级保留结构」，后续灌数据也连带失败（真机：整库跑完
/// 112 张表行数对不上，其中不少目标 0 行）。
///
/// DDL 本来就不并行受益（同一张表的建表/清空/建索引必须串行），全任务一条连接
/// 既够用又不会挤占读写连接。
pub(crate) const SESSION_SYNC_DDL: &str = "internal:sync-ddl";

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
        Err(err) => Some(err.message),
    }
}

/// 直接对**已解析**的目标执行。`sync_table` 在开头解析一次，循环里的每条语句
/// 复用结果 —— 原来每条 `run_sql` 都重新 `require_record`，单表几百条语句
/// 就是几百次白查（每条语句固定链路开销里它占一份）。
pub(crate) async fn run_target(state: &AppState, target: &str, sql: &str) -> XResult<dbmind_core::QueryResult> {
    run_target_in(state, target, sql, SESSION_SYNC).await
}

/// 明确要求「可写会话」的目标端执行（清空/重建这类 DDL-DML）。
///
/// 为什么不能复用 [`run_target`]：它的 `read_only` 是 `None`，由内核按上下文决定。
/// 目标连接若被标成只读（连接设置里的只读开关，或会话池里复用到一条只读会话），
/// 驱动会在语句层直接拒绝：`Connection is read-only. Queries leading to data
/// modification are not allowed.` —— 真机就报在收尾对账的「清空重传」上：
/// 行数对不上、想重传，却在 truncate 这一步失败，只能放弃重传（数据就一直是少的）。
pub(crate) async fn run_target_write(
    state: &AppState,
    target: &str,
    sql: &str,
) -> XResult<dbmind_core::QueryResult> {
    let engine = state.engine();
    let request = dbmind_core::QueryRequest {
        schema: None,
        // 写语句：显式要求可写会话（见函数注释）
        read_only: Some(false),
        connection: target.to_string(),
        sql: sql.to_string(),
        options: dbmind_core::QueryOptions {
            max_rows: 1,
            timeout_ms: 600_000,
        },
        execution_id: None,
        // **全任务共用一条 DDL 连接**（见 [`SESSION_SYNC_DDL`]）。
        session: Some(SESSION_SYNC_DDL.to_string()),
        internal: true,
    };
    let result = blocking(move || engine.execute(request, dbmind_core::AccessContext::Web)).await?;
    Ok(result)
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
    run_target_in_rows(state, target, sql, session, 1).await
}

/// 目标端判活（对象级流水的「无进展」判定用）：另开会话跑 `select 1`，5 秒不回算不通。
///
/// 全任务共用一个探活会话键：只判「链路是否还能通」，不需要按对象隔离；
/// 共用一条物理连接也避免「几百个对象各开一条探活连接」把目标端连接数顶满。
pub(crate) async fn probe_target_link(state: &AppState, target: &str) -> bool {
    const PROBE_KEY: &str = "internal:sync-probe-obj";
    matches!(
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            run_target_in(state, target, "select 1", PROBE_KEY),
        )
        .await,
        Ok(Ok(_))
    )
}

/// 对象级判活 + 一次复核（判定「断」会中止整张表，不能让偶发丢包触发）。
pub(crate) async fn probe_target_link_confirmed(state: &AppState, target: &str) -> bool {
    if probe_target_link(state, target).await {
        return true;
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    probe_target_link(state, target).await
}

/// 标记 / 解除「这条写会话正开着事务」的保护（见 `DbMindEngine::hold_tx_lane`）。
///
/// 不加 `async`：只是会话池里的一次标记，不碰网络。失败**不算错误** ——
/// 保护的作用是「少一次被回收的机会」，退化成旧行为远好过因此让整表失败。
///
/// 为什么必须有（真机故障）：写会话在批与批之间把槽位还回池里，那一刻它「空闲」，
/// 但 `set autocommit=0` 打开的事务还压着一整页未提交的行。配额顶满时按「最久未用的
/// 空闲会话」回收就会挑中它 —— 宿主关掉连接、服务端回滚这些行，写侧却以为写成功了。
pub(crate) fn hold_target_tx(state: &AppState, target: &str, session: &str, hold: bool) {
    if let Err(err) = state.engine().hold_tx_lane(target, session, hold) {
        tracing::debug!(
            target: "dbmind::sync",
            session = %session,
            hold,
            error = %err.message,
            "标记/解除写事务会话保护失败（已忽略）"
        );
    }
}

/// 同 [`run_target_in`]，但允许返回多行。仅用于按批查询目标已有键这类
/// 内部读取，不能把写语句放进来拿多行。
pub(crate) async fn run_target_in_rows(
    state: &AppState,
    target: &str,
    sql: &str,
    session: &str,
    max_rows: usize,
) -> XResult<dbmind_core::QueryResult> {
    run_target_in_rows_timeout(state, target, sql, session, max_rows, 600_000).await
}

/// DDL（建索引 / 改表）的墙钟上限：**必须比搬数据短得多**。
///
/// 为什么不能沿用 600 秒：慢盘上给 214 万行建一个 `varchar(64)` 二级索引，实测
/// **超过 5 分钟还没建完**（直接在同机 mariadb 客户端跑也一样），任务就整段卡在这里
/// —— 用户看到的是「同步一直不结束」（真机 21:05 数据搬完、21:15 还没收尾）。
///
/// 索引没建成的后果只是「目标表少一个索引」，数据早就写好了，所以超时该放弃并继续，
/// 界面会照常报一句「N 个索引未能创建（可在目标库手工补建）」。
///
/// 取 300 秒（原来 180）：千万行的表建一个二级索引，慢盘上三分钟是常态；
/// 它**不跟界面的「超时时间」联动** —— 那个旋钮管的是「数据传不回来」，
/// 而建索引失败是可以事后补的，两者的取舍不同。
const DDL_TIMEOUT_MS: u64 = 300_000;

/// 目标端执行 **DDL**：建索引这类可能很慢的语句，用 [`DDL_TIMEOUT_MS`] 而不是 600 秒。
pub(crate) async fn run_target_ddl(
    state: &AppState,
    target: &str,
    sql: &str,
) -> XResult<dbmind_core::QueryResult> {
    run_target_in_rows_timeout(state, target, sql, "internal:sync-ddl", 1, DDL_TIMEOUT_MS).await
}

async fn run_target_in_rows_timeout(
    state: &AppState,
    target: &str,
    sql: &str,
    session: &str,
    max_rows: usize,
    timeout_ms: u64,
) -> XResult<dbmind_core::QueryResult> {
    let engine = state.engine();
    let request = dbmind_core::QueryRequest {
        schema: None,
        read_only: None,
        connection: target.to_string(),
        sql: sql.to_string(),
        options: dbmind_core::QueryOptions {
            max_rows,
            timeout_ms,
        },
        execution_id: None,
        session: Some(session.to_string()),
        // 数据同步是界面功能驱动的批量读写：不进查询历史（一次同步会下发几百条语句）
        internal: true,
    };
    let result = blocking(move || engine.execute(request, dbmind_core::AccessContext::Web)).await?;
    Ok(result)
}

/// 目标端专用的预编译 INSERT。数据与目标列类型一起下发给 JDBC 宿主，
/// 由 `PreparedStatement` 绑定后整批执行；只有能确认回滚的失败才会返回给调用方
/// 作为可安全降级的错误。
async fn run_target_prepared_insert(
    state: &AppState,
    target: &str,
    session: &str,
    target_name: &str,
    columns: &[String],
    target_types: &[Option<String>],
    target_kind: Dialect,
    rows: &[Vec<CellValue>],
    in_transaction: bool,
) -> XResult<usize> {
    let engine = state.engine();
    let sql = build_prepared_insert(target_name, columns, target_kind);
    let request = dbmind_core::QueryRequest {
        schema: None,
        read_only: None,
        connection: target.to_string(),
        sql,
        options: dbmind_core::QueryOptions {
            max_rows: 1,
            timeout_ms: 600_000,
        },
        execution_id: None,
        session: Some(session.to_string()),
        internal: true,
    };
    let types = target_types.to_vec();
    let values = rows.to_vec();
    // 方言保存点语句：SQL Server 用 `save transaction` / `rollback transaction`，
    // 因为写入插件是用 SQL 文本开的 `begin transaction`，驱动侧 autocommit 仍是 true，
    // 宿主的 `connection.setSavepoint()` 必然抛错（见 `Spec::savepoint_sql`）。
    let savepoint_sql = crate::api::transfer::plugins::spec_data_for(target_kind.kind.key())
        .savepoint_sql();
    let result = blocking(move || {
        // `in_transaction`：写入插件已经在目标会话上发过 `begin transaction`，
        // 宿主不能替它 commit（否则收尾的 commit 会报「没有对应的 BEGIN」）。
        engine.execute_prepared_insert_batch_in(
            request,
            types,
            values,
            in_transaction,
            savepoint_sql,
            dbmind_core::AccessContext::Web,
        )
    })
    .await?;
    Ok(result.affected_rows.unwrap_or(0))
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
    order_keys: &[String],
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
        order_keys,
        extra_where,
        page_rows,
    );
    // 排障开关：`DBMIND_SYNC_SQL_LOG=1` 时把源端**每一页的 SELECT 原文**写进日志。
    // 「漏行 + 主键冲突」这种症状只看计数查不出来（真机：一张 5.8 万行的表每天只搬过去
    // 一半、另一些行报 Duplicate entry），必须看见 keyset 条件与 order by 是否自洽。
    // 默认关，零噪声。
    if std::env::var("DBMIND_SYNC_SQL_LOG")
        .map(|value| value != "0" && !value.is_empty())
        .unwrap_or(false)
    {
        tracing::warn!(target: "dbmind::sql", "{}", sql);
    }
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
    order_keys: &[String],
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
            // 多列键优先用行值比较 `(a,b,c) > (x,y,z)`：语义与逐列 or 链相同，但优化器
            // 只扫「本页那一段」。真机 EXPLAIN（6 列主键、81 万行、一页 652 行）：
            // or 链 range 预估扫 **457,379** 行，行值比较 index 只扫 **652** 行 ——
            // 那张表因此从「每页扫几十万行、68 分钟」变成每页只碰本页。
            // 不认这个语法的方言（SQL Server / Oracle / Doris / Derby）仍展开成 or 链。
            let keyset = if keys.len() > 1 && dialect.supports_row_value_compare() {
                let left = keys
                    .iter()
                    .map(|key| dialect.quote(key))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({left}) > ({})", last.join(", "))
            } else {
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
                conditions.join(" or ")
            };
            format!(
                "{} order by {order} {}",
                filtered(select_base, &keyset),
                dialect.limit_clause(0, page_rows)
            )
        }
        _ => {
            let page = dialect.limit_clause(offset, page_rows);
            // **首页（cursor = None）同样必须按分页键排序！**
            //
            // 这是「漏行 + 主键冲突」的真凶：不排序时引擎按自己的物理顺序（InnoDB 就是
            // 主键序）返回，`rows.last()` 取到的根本不是「最大值」，下一页的 keyset 就从
            // 一个错误的位置往后读 —— 比它小的行永远读不到（漏行），比它大的行被再读一遍
            // （目标端主键冲突）。真机 `xingwei_sales_vice`：首页 1 万行按主键序回来、
            // 末行是 2017-01-14，而这一万行里最大已经到 2018-08，于是第二页与首页重叠
            // 16 行、整表 5.8 万行只搬过去一半。日志里那句话一点都不明显 ——
            // 只有把每页 SELECT 原文打出来才看得见缺陷（`DBMIND_SYNC_SQL_LOG=1`）：
            //   `select * from (select …) dbmind_page limit 10000 offset 0`
            let order = order_keys
                .iter()
                .map(|key| dialect.quote(key))
                .collect::<Vec<_>>()
                .join(", ");
            if !order.is_empty() {
                format!("{} order by {order} {page}", filtered(select_base, ""))
            } else if dialect.needs_order_by_for_paging() {
                // 无分页键（OFFSET 分页）且方言要求 order by 才能写 offset/fetch：占位排序
                format!("{} order by (select null) {page}", filtered(select_base, ""))
            } else {
                format!("select * from ({}) dbmind_page {page}", filtered(select_base, ""))
            }
        }
    }
}

/// 索引创建的时机（见 `sync_table_inner` 的「索引后置」）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum IndexFilter {
    /// 一次建完（不灌数据时用）。
    All,
    /// 只建唯一索引 —— 灌数据**之前**建：`replace` / `on duplicate key update` 的去重语义
    /// 依赖目标端的唯一索引，推迟会让冲突行变成重复行。
    UniqueOnly,
    /// 只建非唯一（二级）索引 —— 灌数据**之后**建：批量写入期间不用逐行维护二级 B 树，
    /// 建索引时也只全表扫一遍（百万行级新建表能省一截）。
    NonUniqueOnly,
}

/// 单表**精确**行数（终态校验用）。校验要的是准数，所以直接 `count(*)`，
/// 不用统计视图里的估算值。
///
/// `condition` 是拼在 `where` 后面的行过滤：**带过滤的同步必须带上同一份条件**，
/// 否则拿「源表整表行数」去比「只搬了一部分的表」，必然判成不一致 —— 然后白白清空重传，
/// 把用户刚同步过去的数据又删一遍重灌（真机：按表设了 `where` 的库每次收尾都白跑一轮）。
async fn count_table(
    state: &AppState,
    conn: &str,
    scope: &str,
    table_expr: &str,
    condition: &str,
) -> Option<u64> {
    let condition = condition.trim();
    let where_sql = if condition.is_empty() {
        String::new()
    } else {
        format!(" where {condition}")
    };
    let sql = format!("select count(*) as n from {table_expr}{where_sql}");
    let result = run_sql_in(state, conn, scope, sql, 600_000).await.ok()?;
    let value = result.rows.first()?.first()?;
    Some(match value {
        CellValue::Integer(n) => (*n).max(0) as u64,
        CellValue::Real(n) => n.max(0.0) as u64,
        CellValue::Text(text) => text.trim().parse().ok()?,
        _ => return None,
    })
}

/// 源表体量（行数**估算**），给整库同步的调度排序用。
///
/// 只做估算：拿不到就返回空（排序退化成清单原序），绝不为了排序去 `count(*)` 全表扫 ——
/// 那是把「为了快」变成了「先慢一次」。各方言的估算口径都是现成的统计视图：
/// `information_schema.tables` / `system.tables` / `pg_class.reltuples`。
async fn source_table_rows(
    state: &AppState,
    src: &Side,
    kind: Dialect,
) -> std::collections::HashMap<String, i64> {
    let mut out = std::collections::HashMap::new();
    let (database, schema) = match src.scope.split_once('.') {
        Some((db, schema)) => (db.to_string(), schema.to_string()),
        None => (src.scope.clone(), "public".to_string()),
    };
    let quote = |text: &str| text.replace('\'', "''");
    let sql = match kind.kind.key() {
        "mysql" | "mariadb" | "doris" => format!(
            "select table_name as t, table_rows as n from information_schema.tables \
             where table_schema = '{}'",
            quote(&database)
        ),
        "clickhouse" => format!(
            "select name as t, total_rows as n from system.tables where database = '{}'",
            quote(&database)
        ),
        // Kingbase 与 PG 同构（系统目录一样），只是驱动不同
        "postgresql" | "kingbase" => format!(
            "select c.relname as t, coalesce(c.reltuples, 0)::bigint as n from pg_class c \
             join pg_namespace ns on ns.oid = c.relnamespace \
             where ns.nspname = '{}' and c.relkind = 'r'",
            quote(&schema)
        ),
        // Oracle / DM：`num_rows` 是统计信息里的估算（可能为 NULL，按 0 处理）
        "oracle" | "dm" => {
            let owner = schema.to_uppercase();
            format!(
                "select table_name as t, num_rows as n from all_tables where owner = '{}'",
                quote(&owner)
            )
        }
        // DB2：`CARD` 是统计信息里的行数估算
        "db2" => format!(
            "select tabname as t, card as n from syscat.tables where tabschema = '{}'",
            quote(&schema.to_uppercase())
        ),
        // SQL Server：分区统计里 index_id 0/1 分别是堆和聚集索引，加起来就是表行数
        "sqlserver" => format!(
            "select t.name as t, sum(p.rows) as n from sys.dm_db_partition_stats p \
             join sys.tables t on t.object_id = p.object_id \
             where p.index_id in (0, 1) and schema_name(t.schema_id) = '{}' group by t.name",
            quote(&schema)
        ),
        _ => return out,
    };
    let Ok(result) = run_sql_in(state, &src.conn, &src.scope, sql, 5000).await else {
        return out;
    };
    for row in crate::api::meta::rows_of(&result) {
        let pick = |key: &str| {
            row.iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value)
        };
        let (Some(name), Some(count)) = (pick("t"), pick("n")) else {
            continue;
        };
        let name = match name {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        if name.is_empty() {
            continue;
        }
        out.insert(name.to_lowercase(), crate::api::shape::value_to_i64(count).unwrap_or(0));
    }
    out
}

/// 取源表的索引清单，统一成 `(名称, 是否唯一, 列清单)`。
///
/// 与 `sync_indexes` 里的取法是**同一套口径**（MySQL 系走 `SHOW INDEX`，因为 Doris 的
/// information_schema.statistics 不反映 `CREATE INDEX` 建的二级索引；其它方言走
/// `dialect.indexes()` 的整库元数据查询）。区别只在：这里只服务「分页键能不能用某个
/// 唯一索引」，所以解析更窄，解析不出的行直接丢。
async fn source_index_list(
    state: &AppState,
    src: &Side,
    source_kind: Dialect,
    table: &str,
) -> Vec<(String, bool, Vec<String>)> {
    let mut out: Vec<(String, bool, Vec<String>)> = Vec::new();
    let cell = |row: &serde_json::Map<String, Value>, key: &str| -> Option<String> {
        let value = row
            .get(key)
            .or_else(|| row.iter().find(|(name, _)| name.eq_ignore_ascii_case(key)).map(|(_, v)| v))?;
        match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(if *flag { "1" } else { "0" }.to_string()),
            Value::Null => None,
            other => Some(other.to_string()),
        }
    };
    if matches!(source_kind.kind.key(), "mysql" | "mariadb" | "doris") {
        let sql = format!("show index from {}", src.scoped(table, source_kind));
        let Ok(result) = run_sql_in(state, &src.conn, &src.scope, sql, 5000).await else {
            return out;
        };
        // Key_name → (non_unique, [(seq, column)])
        let mut agg: Vec<(String, i64, Vec<(i64, String)>)> = Vec::new();
        for row in crate::api::meta::rows_of(&result) {
            let Some(name) = cell(&row, "Key_name").filter(|n| !n.is_empty()) else {
                continue;
            };
            let Some(column) = cell(&row, "Column_name").filter(|c| !c.is_empty()) else {
                continue;
            };
            let seq = cell(&row, "Seq_in_index").and_then(|s| s.parse::<i64>().ok()).unwrap_or(1);
            let non_unique = cell(&row, "Non_unique").and_then(|s| s.parse::<i64>().ok()).unwrap_or(1);
            match agg.iter_mut().find(|(n, _, _)| *n == name) {
                Some((_, _, cols)) => cols.push((seq, column)),
                None => agg.push((name, non_unique, vec![(seq, column)])),
            }
        }
        for (name, non_unique, mut cols) in agg {
            if name.eq_ignore_ascii_case("PRIMARY") {
                continue;
            }
            cols.sort_by_key(|(seq, _)| *seq);
            out.push((name, non_unique == 0, cols.into_iter().map(|(_, c)| c).collect()));
        }
        return out;
    }
    if let crate::api::dialect::Meta::Sql(sql) = source_kind.indexes() {
        let Ok(result) = run_sql_in(state, &src.conn, &src.scope, sql, 5000).await else {
            return out;
        };
        for row in crate::api::meta::rows_of(&result) {
            let row_table = cell(&row, "table").unwrap_or_default();
            if !row_table.eq_ignore_ascii_case(table) {
                continue;
            }
            let name = cell(&row, "name").unwrap_or_default();
            let unique = cell(&row, "non_unique").and_then(|v| v.parse::<i64>().ok()) == Some(0)
                || cell(&row, "unique").map(|v| v == "1" || v.eq_ignore_ascii_case("true")) == Some(true)
                || cell(&row, "isUnique").and_then(|v| v.parse::<i64>().ok()) == Some(1);
            let columns: Vec<String> = cell(&row, "columns")
                .unwrap_or_default()
                .split(',')
                .map(|c| c.trim().trim_matches(['"', '`', '[', ']']).to_string())
                .filter(|c| !c.is_empty())
                .collect();
            if columns.is_empty() {
                continue;
            }
            out.push((name, unique, columns));
        }
    }
    out
}

/// 无主键表的分页键候补：源表上的**唯一索引**。
///
/// 整库同步里大量 ETL 表没有主键，但有唯一索引（业务单号 + 日期之类）。有它就能走
/// keyset 翻页，把 OFFSET 的 O(N²) 退化成「每页只扫本页」；过去只认主键，这类表一律
/// 掉进深翻页（真机：357 万行的 `project_lease_info_di` 单表跑了 40 分钟）。
///
/// 只接受**列全部落在同步列里、且都 NOT NULL** 的唯一索引：唯一索引允许多行 NULL，
/// 拿它当游标会漏掉那些行（keyset 里 `null > x` 恒为假，整段被跳过）。
async fn unique_paging_key(
    state: &AppState,
    src: &Side,
    source_kind: Dialect,
    table: &str,
    columns: &[String],
    nullable: &[String],
) -> Option<Vec<String>> {
    let mut candidates: Vec<Vec<String>> = Vec::new();
    for (_, unique, cols) in source_index_list(state, src, source_kind, table).await {
        if !unique || cols.is_empty() {
            continue;
        }
        if cols.iter().any(|c| nullable.iter().any(|n| n.eq_ignore_ascii_case(c))) {
            continue;
        }
        if cols.iter().any(|c| !columns.iter().any(|s| s.eq_ignore_ascii_case(c))) {
            continue;
        }
        candidates.push(cols);
    }
    candidates.sort_by_key(|cols| cols.len());
    candidates.into_iter().next()
}

/// 建表后把**源表的非主键索引**搬到目标（「建表包含索引」选项）。
///
/// 索引清单走方言的 `indexes()` 元数据查询（与树上「索引」节点同一份 SQL）。
/// 宽容策略：查不到清单 / 解析不了列 / 目标方言拒绝某条 CREATE —— 都**静默跳过**
/// 并计数，不让索引同步反过来挡住数据同步（数据才是主体）。
#[allow(clippy::too_many_arguments)]
async fn sync_indexes(
    state: &AppState,
    src: &Side,
    source_kind: Dialect,
    tgt_target: &str,
    target_kind: Dialect,
    target_name: &str,
    table: &str,
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
    filter: IndexFilter,
    declared_primary: &[String],
    source_columns: &[ColumnDetail],
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
            // 大小写不敏感取列（有的驱动把列名折成大写）。
            // **不能只认 `as_str`**：Non_unique / Seq_in_index 是数字列，宿主把结果集回成
            // JSON 数字，`as_str()` 直接 None ⇒ 落到下面的 `unwrap_or(1)` ⇒ 所有索引都被
            // 当成非唯一（真机：源表唯一索引 uk_email 在目标端建成了普通索引，配上
            // 索引后置还会落错时机）。数字/布尔一律转成字符串。
            let cell = |row: &serde_json::Map<String, Value>, key: &str| -> Option<String> {
                let value = row.get(key).or_else(|| {
                    row.iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(key))
                        .map(|(_, v)| v)
                })?;
                match value {
                    Value::String(text) => Some(text.clone()),
                    Value::Number(number) => Some(number.to_string()),
                    Value::Bool(flag) => Some(if *flag { "1" } else { "0" }.to_string()),
                    Value::Null => None,
                    other => Some(other.to_string()),
                }
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
                // PRIMARY 不搬：建表语句本来就带主键，再 `create index PRIMARY` 必然失败。
                // 以前失败是静默的，现在失败会进日志（见下面 `failed`），必须先剔掉这个噪音。
                if name.eq_ignore_ascii_case("PRIMARY") {
                    continue;
                }
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
    // 源表没有真正主键时，某个唯一索引已经被建表 DDL 当主键写进去了：这里不能再建一遍。
    // 两套同名约束的代价不是"多一个索引"那么轻 —— 每次插入要维护两棵 B 树，而且
    // `mysql_has_secondary_unique` 会把它当成"额外唯一索引"，让本可走原生 upsert 的
    // 表降级成两阶段（真机：winneryun_huike_data_hour 目标端就是这样双份约束）。
    // 比较列集合而不是列序：唯一约束与列序无关（源是 (SITE_KEY, DATA_TIME)、
    // 建出来的主键是 (DATA_TIME, SITE_KEY)）。
    if !declared_primary.is_empty() {
        let mut declared: Vec<String> = declared_primary
            .iter()
            .map(|column| column.to_ascii_lowercase())
            .collect();
        declared.sort();
        indexes.retain(|(_, unique, cols)| {
            if !*unique || cols.len() != declared.len() {
                return true;
            }
            let mut current: Vec<String> = cols.iter().map(|c| c.to_ascii_lowercase()).collect();
            current.sort();
            current != declared
        });
    }
    // 索引后置：按阶段只建该建的那一半（见 `IndexFilter` 的注释）
    if filter != IndexFilter::All {
        let want_unique = filter == IndexFilter::UniqueOnly;
        indexes.retain(|(_, unique, _)| *unique == want_unique);
    }
    if indexes.is_empty() {
        return 0;
    }
    // 建索引是链路里的一步，**开始与结果都进界面日志**：
    // 索引后置之后这一步发生在数据灌完之后，大表上可能跑几分钟、期间行数不动，
    // 以前只写文件日志，用户看到的是「数据都搬完了却不知道在干嘛」。
    // 只打「N 个」的汇总，不逐条刷屏（400 张表的整库同步也扛得住）。
    let total = indexes.len();
    if let Some(task) = task {
        task.log(format!("表 {table}：创建索引（{total} 个）"));
    }
    tracing::debug!(table, total, "开始创建索引");
    let mut created = 0usize;
    // 建失败的索引：不逐条刷屏，最后汇总成一行（见下面的 `failed`）。
    let mut failed: Vec<String> = Vec::new();
    // ClickHouse 这类「源索引推导不出目标索引」的跳过项，也要在日志里交代一句，
    // 否则用户看到「创建索引（3 个）」之后就没有下文了。
    let mut skipped: Vec<String> = Vec::new();

    // MySQL / MariaDB：把这一批索引合并成**一条** ALTER（服务器只扫一遍表）。
    //
    // 为什么：逐条 `create index` 每条都要扫一遍全表（真机：1 亿行的表 6 个索引，
    // 光建索引就多花好几倍时间，用户看到的是「跑到后面越来越慢」而不是数据量大）；
    // 合并成一条 ALTER 只扫一次。失败再退回逐条建（隔离重名/不支持的索引）。
    if total > 1 && matches!(target_kind.kind.key(), "mysql" | "mariadb") {
        let mut parts: Vec<String> = Vec::with_capacity(total);
        for (name, unique, columns) in &indexes {
            let col_list = columns
                .iter()
                .map(|c| target_kind.quote(c))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!(
                "add {}index {} ({})",
                if *unique { "unique " } else { "" },
                target_kind.quote(name.as_str()),
                col_list
            ));
        }
        let sql = format!("alter table {target_name} {}", parts.join(", "));
        // 建索引用 **DDL 短超时**：慢盘上这一步能拖到十分钟以上，任务会一直「不结束」
        //（见 `DDL_TIMEOUT_MS` 的注释）。超时按「未建成」处理，不影响已同步的数据。
        match run_target_ddl(state, tgt_target, &sql).await {
            Ok(_) => created = total,
            Err(err) => {
                tracing::warn!(table, error = %err.message, "合并建索引失败，改为逐条创建");
            }
        }
    }

    if created == 0 {
        for (name, unique, columns) in indexes {
            let idx_name = target_kind.quote(name.as_str());
            let col_list = columns
                .iter()
                .map(|c| target_kind.quote(c))
                .collect::<Vec<_>>()
                .join(", ");
            let unique_kw = if unique { "unique " } else { "" };
            // ClickHouse 的 ORDER BY 就是源表主键 —— 与它完全重复的索引建了也用不上
            //（CH 不会拿它剪枝），直接跳过并说明原因。
            if target_kind.kind.key() == "clickhouse"
                && same_key_columns(&columns, declared_primary)
            {
                tracing::debug!(table, index = %name, "跳过：与 ClickHouse 排序键（ORDER BY）完全重复");
                skipped.push(name.clone());
                continue;
            }
            // **索引名保持源名**（用户口径：源叫什么目标就叫什么；重名/不支持时静默跳过）
            let idx_sql = match target_kind.kind.key() {
                // Doris：二级索引用 BITMAP 写法（普通 create index 不带 USING 会失败）
                "doris" => Some(format!("create index {} on {} ({}) using bitmap", idx_name, target_name, col_list)),
                // ClickHouse：没有 btree 索引，只有**跳数索引**（data skipping index），
                // 必须带 TYPE 与 GRANULARITY —— 建法见 `clickhouse_skip_index`
                "clickhouse" => clickhouse_skip_index(target_kind, target_name, &name, &columns, source_columns),
                _ => Some(format!("create {}index {} on {} ({})", unique_kw, idx_name, target_name, col_list)),
            };
            let Some(idx_sql) = idx_sql else {
                tracing::debug!(table, index = %name, "跳过：ClickHouse 跳数索引推不出可用表达式");
                skipped.push(name.clone());
                continue;
            };
            match run_target_ddl(state, tgt_target, &idx_sql).await {
                Ok(_) => {
                    created += 1;
                    // CH 的 ADD INDEX 只对**之后**写入的 part 生效，表里已有数据要
                    // MATERIALIZE（按索引表达式扫一遍）才吃得到。回填失败不算索引建失败：
                    // 索引已经挂上，只是历史数据没覆盖，日志里说清楚就行。
                    if target_kind.kind.key() == "clickhouse" {
                        let sql = format!(
                            "alter table {} materialize index {}",
                            target_name, idx_name
                        );
                        if let Err(err) = run_target_ddl(state, tgt_target, &sql).await {
                            tracing::warn!(table, index = %name, error = %err.message, "MATERIALIZE INDEX 失败");
                            if let Some(task) = task {
                                // 只报原话（索引名进 tracing）
                                tracing::warn!(table, index = %name, error = %err.message, "索引历史数据回填失败");
                                task.log(format!("表 {table}：{}", brief_error(&err.message)));
                            }
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(table, index = %name, error = %err.message, "创建索引失败（已跳过）");
                    failed.push(name.clone());
                }
            }
        }
    }
    if created > 0 {
        // 结果行：让「创建索引（N 个）」这一对有始有终，用户能确认这一步到底做完了没
        if let Some(task) = task {
            task.log(format!("表 {table}：索引创建完成（{created} 个）"));
        }
        tracing::debug!(table, created, "二级索引已创建");
    }
    if !skipped.is_empty() {
        if let Some(task) = task {
            task.log(format!("表 {table}：{} 个索引已跳过", skipped.len()));
        }
    }
    if !failed.is_empty() {
        if let Some(task) = task {
            let shown = failed.iter().take(5).cloned().collect::<Vec<_>>().join("、");
            let more = if failed.len() > 5 { format!(" 等 {} 个", failed.len()) } else { String::new() };
            task.log(format!(
                "表 {table}：{} 个索引未能创建（{shown}{more}）",
                failed.len()
            ));
        }
    }
    created
}

/// 两组列名是不是同一组（大小写不敏感、与列序无关）。
fn same_key_columns(a: &[String], b: &[String]) -> bool {
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    let mut left: Vec<String> = a.iter().map(|c| c.to_ascii_lowercase()).collect();
    let mut right: Vec<String> = b.iter().map(|c| c.to_ascii_lowercase()).collect();
    left.sort();
    right.sort();
    left == right
}

/// ClickHouse 的「二级索引」不是 btree，而是**跳数索引**（data skipping index）：
/// 只用于查询剪枝，不保证唯一性 —— MySQL 的唯一索引搬过去也只是「能加速查」的近似。
///
/// 建法与 MySQL 完全不同，必须带 TYPE 与 GRANULARITY：
/// `alter table t add index idx expr type minmax granularity 4`（CH 24.8 真机实测通过）。
/// 从源库 btree 索引能推出来的映射：
/// - 文本列 → `bloom_filter`（等值 / IN 过滤，varchar 索引最常见的用法）
/// - 数值、日期等 → `minmax`（范围过滤）
/// - 多列 → `minmax` 作用在**元组**上（`(a, b)`，真机实测 CH 接受）
/// 列名在源表里找不到（改名/不存在）时按 `minmax`：任何可比较类型都能建，最保守。
fn clickhouse_skip_index(
    target_kind: Dialect,
    target_name: &str,
    name: &str,
    columns: &[String],
    source_columns: &[ColumnDetail],
) -> Option<String> {
    if columns.is_empty() {
        return None;
    }
    let is_text = |column: &str| {
        source_columns
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(column))
            .map(|c| Dialect::is_textual_type(c.type_name.as_deref().unwrap_or("")))
            .unwrap_or(false)
    };
    let kind = if columns.len() == 1 && is_text(&columns[0]) {
        "bloom_filter"
    } else {
        "minmax"
    };
    let expr = if columns.len() == 1 {
        target_kind.quote(&columns[0])
    } else {
        format!(
            "({})",
            columns
                .iter()
                .map(|c| target_kind.quote(c))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    Some(format!(
        "alter table {} add index {} {} type {} granularity 4",
        target_name,
        target_kind.quote(name),
        expr,
        kind
    ))
}

/// 「索引后置」的收尾：数据已经落库（或确定一行都不搬），补建**非唯一**二级索引。
///
/// 建表时只先建了唯一索引（见 `IndexFilter`），这里是另一半。`deferred` 为 false 时
/// 什么都不做 —— 仅结构同步 / 目标表本来就存在（没走建表、索引也不归这里管）都是这种。
#[allow(clippy::too_many_arguments)]
async fn create_deferred_indexes(
    state: &AppState,
    src: &Side,
    source_kind: Dialect,
    tgt_target: &str,
    target_kind: Dialect,
    target_name: &str,
    table: &str,
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
    deferred: bool,
    declared_primary: &[String],
    source_columns: &[ColumnDetail],
) {
    if !deferred {
        return;
    }
    // 链路里的一步：数据搬完后补建二级索引（大表上可能几分钟，期间行数不动）。
    // 界面日志由 sync_indexes 统一打「创建索引（N 个）→ 索引创建完成（N 个）」，
    // 这里不再重复喊一句 —— 而且没有非唯一索引时那句会变成空头支票。
    tracing::debug!(table, "数据已灌完，创建二级索引");
    sync_indexes(
        state,
        src,
        source_kind,
        tgt_target,
        target_kind,
        target_name,
        table,
        task,
        IndexFilter::NonUniqueOnly,
        declared_primary,
        source_columns,
    )
    .await;
}

/// MySQL 的 `ON DUPLICATE KEY UPDATE` 会命中**任意**唯一索引，而不是只命中
/// 用户选中的主键。目标表若还有其它唯一索引，源行可能因为另一个唯一键冲突
/// 而更新到另一行。探测失败同样保守回退：不能拿"不知道"换"可能更新错行"。
async fn mysql_has_secondary_unique(
    state: &AppState,
    conn: &str,
    scope: &str,
    target_name: &str,
) -> bool {
    let sql = format!("show index from {target_name}");
    let Ok(result) = run_sql_in(state, conn, scope, sql, 5000).await else {
        return true;
    };
    for row in crate::api::meta::rows_of(&result) {
        let cell = |key: &str| {
            row.get(key)
                .or_else(|| row.iter().find(|(name, _)| name.eq_ignore_ascii_case(key)).map(|(_, v)| v))
        };
        let key_name = cell("Key_name").and_then(Value::as_str).unwrap_or("");
        let non_unique = cell("Non_unique")
            .and_then(crate::api::shape::value_to_i64)
            .unwrap_or(1);
        if non_unique == 0 && !key_name.eq_ignore_ascii_case("PRIMARY") {
            return true;
        }
    }
    false
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

/// 数据里出现了 CSV 特殊字符吗？控制分隔符、反斜杠（Doris 的转义符）以及
/// `\N` 空值标记都可能改变原值，有则整批退回 JSON 编码。
pub(crate) fn rows_need_json_fallback(rows: &[Vec<dbmind_core::CellValue>]) -> bool {
    rows.iter().any(|row| {
        row.iter().any(|cell| match cell {
            dbmind_core::CellValue::Text(s) => {
                s.contains('\\') || s.bytes().any(|b| b == CSV_COL_SEP_BYTE || b == CSV_ROW_SEP_BYTE)
            }
            _ => false,
        })
    })
}

/// 列名含逗号、控制字符等时，CSV 的 `columns` 头无法无损表达，改走 JSON 键。
fn columns_need_json_fallback(columns: &[String]) -> bool {
    columns.iter().any(|name| {
        name.chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
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
            dbmind_core::CellValue::Decimal(s) => out.extend_from_slice(s.as_bytes()),
                dbmind_core::CellValue::Text(s) => out.extend_from_slice(s.as_bytes()),
                // 含二进制列的表**不会**走 Stream Load（见 use_stream 的判定），
                // 这里只是兜底：真走到就是提示语，绝不是"把 b:len 当数据写进去"。
                dbmind_core::CellValue::Bytes(data) => {
                    out.extend_from_slice(format!("b:{}", data.len()).as_bytes())
                }
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
                Some(dbmind_core::CellValue::Decimal(s)) => Value::from(s.as_str()),
                Some(dbmind_core::CellValue::Text(s)) => Value::from(s.as_str()),
                Some(dbmind_core::CellValue::Bytes(data)) => {
                    Value::from(format!("b:{}", data.len()))
                }
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

/// URL 里的 `host:port` —— 日志里只报这一小段，不贴整条 URL（一条日志塞两个完整
/// URL 会把真正的原因挤到屏幕外）。
fn url_authority(url: &str) -> String {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    match rest.find('/') {
        Some(index) => rest[..index].to_string(),
        None => rest.to_string(),
    }
}

/// 把 Stream Load 的 URL 换成「FE 主机 + 原端口原路径」。
///
/// 为什么需要：BE 向 FE 注册的地址经常是**容器内网 IP**（`172.29.0.14`）或 `127.0.0.1`，
/// 从客户端看这两个都到不了（一个不可路由、一个是自己）。同端口换到 FE 主机往往就能通
/// （docker 把 8040 映射到宿主时），所以在传输层失败后值得再试一次。
/// 已经指向 FE 主机、或 URL 解析不出 authority 时返回 None（不再重试，避免死循环）。
fn same_port_on_fe_host(url: &str, fe_host: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    let fe_host = fe_host.trim();
    if fe_host.is_empty() || host.eq_ignore_ascii_case(fe_host) {
        return None;
    }
    Some(match port {
        Some(port) => format!("http://{fe_host}:{port}{path}"),
        None => format!("http://{fe_host}{path}"),
    })
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
    // Stream Load 的 URL 段是**裸库名**；catalog scope 可能是 `internal.ods`。
    let database = database.rsplit('.').next().unwrap_or(database);
    // CSV 优先；文本里混入分隔符控制字符时整批退回 JSON（正确性优先）
    let use_json = rows_need_json_fallback(rows) || columns_need_json_fallback(columns);
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
    // Stream Load 的 `columns` 头用**反引号**包列名：它是 SQL 片段而不是纯名字，列名撞上
    // 保留字（order / desc / group / key…）时不加引号会被当成关键字解析。
    let columns_header = columns
        .iter()
        .map(|name| format!("`{}`", name.replace('`', "``")))
        .collect::<Vec<_>>()
        .join(", ");
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
        // 入口候选：**有 BE 端口就只打 BE**。
        //
        // 以前 BE 失败还会再打一次 FE，实际上帮不上忙：Doris 的 FE 只回 307 把客户端重定向到
        // 同一个 BE（`enable_stream_load_redirect` 默认开），而 FE 还要求请求带
        // `Expect: 100-continue`，不带就直接回 `{"status":"FAILED","msg":"There is no
        // 100-continue header"}`；带上则 ureq 2.x 处理不了 1xx 中间响应（会把 `100 Continue`
        // 当终站响应）。于是那一跳要么白等一个网络超时、要么只换来一句看不懂的报错。
        // 只有在**拿不到 BE 端口**时才退回 FE（此时可能配了 FE 代理转发）。
        let mut starts: Vec<String> = Vec::new();
        if be_port > 0 {
            starts.push(format!("http://{}:{}/api/{}/{}/_stream_load", host_owned, be_port, db_owned, tbl_owned));
        } else {
            starts.push(fe_url.clone());
        }
        // 传输层失败的具体原因（连接被拒 / 超时 / DNS）—— 最终错误里必须带上：只回一句
        // 「不可达」用户既不知道连的哪个端口，也不知道为什么（真机反馈「报错」但无从下手）。
        // 逐个入口记下失败原因（BE 直连 / FE 重定向），最后一起报 —— 只报「最后一个」
        // 会把更有信息量的那条（例如 FE 的 JSON 报错）丢掉
        let mut errors: Vec<String> = Vec::new();
        for start in starts {
            let mut url = start;
            // 每个入口只换一次主机：BE 注册的地址常常是**容器内网 IP**（如 172.29.0.14），
            // 客户端根本到不了 —— 这时用 FE 的主机名 + 同一端口再试一次（docker 把 8040
            // 映射出来就能通）；仍然不通才认定这个入口不可用。
            let mut host_swapped = false;
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
                        // CSV 按位置解释；目标表可能有额外列，必须显式给列清单。
                        .set("columns", &columns_header)
                };
                let resp = match req.send_bytes(&body) {
                    Ok(resp) => resp,
                    // ureq 对 >=400 默认返回 Err(Status)，把响应体捞出来才有诊断价值
                    Err(ureq::Error::Status(_, resp)) => resp,
                    // 传输层失败（连不上/连接被断）：换下一个候选入口
                    Err(e) => {
                        tracing::warn!(url = %url, ms = hop_t0.elapsed().as_millis() as u64, error = %e, "Stream Load 传输失败");
                        // ureq 的 Display 把完整 URL 也带上了（`http://…: Connection Failed: …`），
                        // 这里只留原因本身，避免一条日志里贴两遍 URL
                        let reason = {
                            let text = e.to_string();
                            text.strip_prefix(url.as_str())
                                .map(|rest| rest.trim_start_matches([':', ' ']).to_string())
                                .unwrap_or(text)
                                .chars()
                                .take(160)
                                .collect::<String>()
                        };
                        errors.push(format!("{} 连接失败（{reason}）", url_authority(&url)));
                        // BE 宣称的是内网/容器地址时，换 FE 主机 + 同一端口重试一次
                        if !host_swapped {
                            if let Some(alt) = same_port_on_fe_host(&url, &host_owned) {
                                tracing::info!(from = %url, to = %alt, "Stream Load 换 FE 主机重试");
                                host_swapped = true;
                                url = alt;
                                continue;
                            }
                        }
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
                // **BE 与 FE 的 JSON 键大小写不同**：BE 回 `{"Status":"Success","Message":…}`，
                // FE 回 `{"status":"FAILED","msg":"There is no 100-continue header"}`。
                // 只认大写那套的话，FE 的错误会被当成「响应不是 JSON」整段贴进日志（真机就是
                // 一大串 `{"status":"FAILED",...}`），用户看不出这是一句人能读懂的原因。
                let field = |v: &Value, name: &str| -> Option<String> {
                    v.get(name)
                        .or_else(|| v.get(&name.to_ascii_lowercase()))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                };
                match v {
                    Some(v) if field(&v, "Status").as_deref() == Some("Success") => {
                        // 脏数据记账：类型不匹配等被 BE 过滤的行不算成功载入 ——
                        // 返回实际载入行数，调用方把差额计入失败（对齐 DataX 的 errorLimit 思路）
                        let loaded = doris_number(v.get("NumberLoadedRows").or_else(|| v.get("numberLoadedRows")));
                        return Ok(loaded);
                    }
                    Some(v) => {
                        let msg_head: String = field(&v, "Message")
                            .unwrap_or_else(|| head.clone())
                            .chars()
                            .take(300)
                            .collect();
                        // FE 的「There is no 100-continue header」= FE 只做 307 转发，且要求
                        // 客户端带 Expect 头。翻译成人话，否则用户完全不知道这是什么。
                        let msg_head = if msg_head.to_ascii_lowercase().contains("100-continue") {
                            format!(
                                "{msg_head}（FE 只做 307 转发、要求带 Expect 头；\
                                 这条链路的瓶颈其实是 BE 的 HTTP 端口能不能直连，见下面的提示）"
                            )
                        } else {
                            msg_head
                        };
                        return Err(dbmind_core::DbMindError::new(
                            dbmind_core::ErrorCode::QueryFailed,
                            msg_head,
                        ))
                    }
                    None => {
                        // 按用户口径只把响应体原样报出去；content-type 这类上下文进 tracing
                        tracing::debug!(ctype = %ctype, body = %head, "Stream Load 响应不是 JSON");
                        return Err(dbmind_core::DbMindError::new(
                            dbmind_core::ErrorCode::QueryFailed,
                            head,
                        ))
                    }
                }
            }
        }
        Err(dbmind_core::DbMindError::new(
            dbmind_core::ErrorCode::QueryFailed,
            format!(
                "Stream Load 入口都不可用：{}。\
                 常见原因：Doris 跑在 Docker / 防火墙后面，只映射了 MySQL 协议端口（如 9030），\
                 BE 的 HTTP 端口（8040）没有映射 —— 这种部署下 Stream Load 用不了，\
                 数据会自动改走 INSERT 管道（不影响正确性，只是导入慢一些）；\
                 想让导入快起来，把 BE 的 http 端口（show backends 的 HttpPort）也映射出来即可",
                if errors.is_empty() { "没有拿到具体错误".to_string() } else { errors.join("；") }
            ),
        ))
    })
    .await
    .map_err(|e| {
        let detail = e.detail.clone().unwrap_or_default();
        if detail.trim().is_empty() {
            e.message
        } else {
            format!("{}（{detail}）", e.message)
        }
    })
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
            keys.as_slice(),
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

    /// 首页（无游标）也**必须**按分页键排序。不排序时 `rows.last()` 不是最大值，
    /// 下一页的 keyset 从错误位置开始 —— 漏行 + 目标端主键冲突（真机 5.8 万行只搬一半）。
    #[test]
    fn 首页无游标也必须按分页键排序() {
        let keys = vec!["XF_MALLORGID".to_string(), "XF_TXDATE".to_string()];
        let sql = page_sql(
            "select `a`, `b` from `t`",
            Dialect::new(ConnectionKind::Mysql),
            0,
            None,
            keys.as_slice(),
            "",
            PAGE,
        );
        assert!(sql.contains("order by XF_MALLORGID, XF_TXDATE"), "首页丢了 order by：{sql}");
        assert!(sql.contains(&format!("limit {PAGE}")), "缺 limit：{sql}");
        // 用户过滤条件与排序要同时在
        let sql = page_sql(
            "select `a` from `t`",
            Dialect::new(ConnectionKind::Mysql),
            0,
            None,
            keys.as_slice(),
            "`a` > 1",
            PAGE,
        );
        assert!(sql.contains("`a` > 1"), "首页丢了用户条件：{sql}");
        assert!(sql.contains("order by"), "首页丢了 order by：{sql}");
    }

    /// 无键（cursor = None + 无分页键）退回 OFFSET：不能带 where，且要包一层派生表。
    #[test]
    fn 无键退回offset分页() {
        let sql = page_sql(
            "select `id` from `t`",
            Dialect::new(ConnectionKind::Mysql),
            100000,
            None,
            &[],
            "",
            PAGE,
        );
        assert!(!sql.contains(" where"), "无键分页不该出现游标 where：{sql}");
        assert!(sql.contains("dbmind_page"), "应包一层派生表：{sql}");
        // 用户条件仍然要生效
        let sql = page_sql(
            "select `id` from `t`",
            Dialect::new(ConnectionKind::Mysql),
            0,
            None,
            &[],
            "`id` < 10",
            PAGE,
        );
        assert!(sql.contains("`id` < 10"), "无键分页丢了用户条件：{sql}");
    }

    /// 无键 + SQL Server：它写 offset/fetch **必须**带 order by，走占位排序而不是包派生表。
    #[test]
    fn 无键分页在sqlserver走占位排序() {
        let sql = page_sql(
            "select [id] from [t]",
            Dialect::new(ConnectionKind::Sqlserver),
            200,
            None,
            &[],
            "",
            PAGE,
        );
        assert!(sql.contains("order by (select null)"), "SQL Server 缺占位 order by：{sql}");
        assert!(!sql.contains("dbmind_page"), "SQL Server 不该包派生表：{sql}");
    }

    /// 认行值比较的方言（MySQL/MariaDB/PG）必须用它 —— 优化器扫描范围差三个数量级。
    /// 一旦有人把这里改回 or 链，性能会从「每页只碰本页」退化成「每页扫几十万行」。
    #[test]
    fn 多列键在支持的方言用行值比较() {
        let sql = cursor_page(ConnectionKind::Mysql, &["a", "b"], &["1", "2"], "");
        assert!(sql.contains("(a, b) > (1, 2)"), "MySQL 该用行值比较：{sql}");
        assert!(!sql.contains(" or "), "行值比较不该再展开 or 链：{sql}");
        // 方言不认时（SQL Server）同一条游标必须退回 or 链，见 `多列键展开成or链`。
        let sql = cursor_page(ConnectionKind::Postgresql, &["a", "b"], &["1", "2"], "");
        assert!(sql.contains("(a, b) > (1, 2)"), "PG 该用行值比较：{sql}");
    }
}

#[cfg(test)]
mod obj_result_tests {
    use super::*;

    /// 失败却留空原因时，界面只剩「未返回原因（详见同步日志）」，而日志里也不写 ——
    /// 用户对着 15 张红表无从下手（真机 2026-10-27 整库同步）。
    #[test]
    fn 失败结果不能留空原因() {
        let result = ObjResult::failed("table", "etl_wide_sales_tender", "");
        assert!(!result.message.trim().is_empty(), "失败原因不能为空");
        assert!(
            result.message.contains("etl_wide_sales_tender"),
            "兜底文案要带上对象名，否则一屏十几条分不清是谁：{}",
            result.message
        );
    }

    /// 有真实原因（数据库原话）时一个字都不能改。
    #[test]
    fn 有原因时原样保留() {
        let result = ObjResult::failed("table", "t", "Duplicate entry '1' for key 'PRIMARY'");
        assert_eq!(result.message, "Duplicate entry '1' for key 'PRIMARY'");
    }

    /// 写到一半失败：**已写入的行数要带出去**，界面不能只剩 +0
    ///（真机 2026-10-27：15 张失败表全 +0，分不清是没开始写还是写了一半）。
    #[test]
    fn 失败时带出已写入行数() {
        let result = ObjResult::failed_with("table", "t", "写入超过 600 秒仍未响应", 1_234, 0);
        assert_eq!(result.status, "error");
        assert_eq!(result.inserted, 1_234, "已写入的行数要显示出来");
        // 完全没写过（建表/取列就失败）仍然是 0，不能凭空编
        assert_eq!(ObjResult::failed("table", "t", "boom").inserted, 0);
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
            vec![CellValue::Decimal("12345678901234567890.1234567890".into())],
        ];
        let out = encode_stream_csv(&rows);
        let s = String::from_utf8(out).unwrap();
        // 列分隔 0x01、行分隔 0x02；NULL → \N；文本原样（含逗号/引号/换行都无需转义）
        assert_eq!(
            s,
            "1\u{1}张三,说\"你好\"\u{1}\\N\u{2}1.5\u{1}多行\n文本\u{1}b:7\u{2}\
             12345678901234567890.1234567890\u{2}"
        );
    }

    #[test]
    fn 含分隔符字符的行要退回json() {
        assert!(!rows_need_json_fallback(&[vec![CellValue::Text("普通文本".into())]]));
        assert!(rows_need_json_fallback(&[vec![CellValue::Text("坏\u{1}文本".into())]]));
        assert!(rows_need_json_fallback(&[vec![CellValue::Text("坏\u{2}文本".into())]]));
        assert!(rows_need_json_fallback(&[vec![CellValue::Text("\\N".into())]]));
        assert!(rows_need_json_fallback(&[vec![CellValue::Text("C:\\temp".into())]]));
        assert!(columns_need_json_fallback(&["a,b".to_string()]));
        assert!(!columns_need_json_fallback(&["normal_name".to_string()]));
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
        // 截小数不能吞掉尾随时区
        assert_eq!(
            normalize_datetime_text(
                "2024-06-15T12:30:45.123456789+08:00",
                "datetimeoffset"
            ),
            Some("2024-06-15 12:30:45.123456+08:00".to_string())
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
                tracing::debug!(table, "表注释已同步");
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
        tracing::debug!(table, updated, "列注释已同步");
    }
}

/// 对象类型的中文叫法（界面日志前缀用）。
///
/// 统一一处：开始行与结果行必须用同一个词，否则同一张表在日志里一会儿「表」一会儿
/// 「table」，用户对不上。
fn kind_label(ty: &str) -> &str {
    match ty {
        "table" => "表",
        "view" => "视图",
        "procedure" => "存储过程",
        "function" => "函数",
        "trigger" => "触发器",
        "event" => "事件",
        other => other,
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
        Self::failed_with(ty, name, message, 0, 0)
    }

    /// 已经写进去一部分行才失败：把**实际已写入的行数**带出来。
    ///
    /// 过去失败行的「新增/更新」恒为 +0（构造函数写死 0），用户分不清「压根没开始写」
    /// 和「写到一半断了」—— 真机 2026-10-27 那 15 张失败表全是 +0。
    /// 口径见 `transfer::WriteDiag::rows_written`：已发出且写成功的行，断连时可能随事务回滚。
    fn failed_with(
        ty: &str,
        name: &str,
        message: impl Into<String>,
        inserted: u64,
        updated: u64,
    ) -> Self {
        let message = message.into();
        // 失败却给不出原因 = 界面上只剩「未返回原因（详见同步日志）」，等于什么都没说
        //（真机 2026-10-27：整库同步 15 张表全红，说明列一模一样，日志里也查不到）。
        // 传输引擎里「没有数据库原话」的三处超时已各自给出具体原因
        //（`transfer::engine` 的 `*_timeout_message`），这里兜住其余漏网的空串。
        let message = if message.trim().is_empty() {
            format!("{ty} {name} 同步失败，但没有拿到具体原因（详见任务日志与 tracing 日志）")
        } else {
            message
        };
        Self {
            ty: ty.to_string(),
            name: name.to_string(),
            status: "error",
            message,
            inserted,
            updated,
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
    if let Some(type_name) = target_type {
        if Dialect::is_boolean_type(type_name) {
            let boolean = match cell {
                CellValue::Integer(value) => Some(*value != 0),
                CellValue::Real(value) => Some(*value != 0.0),
                CellValue::Decimal(value) | CellValue::Text(value) => match value.trim() {
                    "0" | "false" | "FALSE" => Some(false),
                    "1" | "true" | "TRUE" => Some(true),
                    _ => None,
                },
                _ => None,
            };
            if let Some(boolean) = boolean {
                return dialect.bool_literal(boolean).to_string();
            }
        }
        if Dialect::is_numeric_type(type_name) {
            if let CellValue::Decimal(value) | CellValue::Text(value) = cell {
                if let Some(value) = plain_decimal(value) {
                    // 数值目标列必须写裸数字；加引号时 PostgreSQL 等会拒绝 text→numeric。
                    return value.to_string();
                }
            }
        }
        if Dialect::is_textual_type(type_name) {
            if matches!(
                cell,
                CellValue::Integer(_) | CellValue::Real(_) | CellValue::Decimal(_)
            ) {
                // 数值 → 文本列：先转十进制文本再加字符串引号，避免 PG 把裸数字视为类型不匹配。
                return dialect.literal(&cell.to_display());
            }
        }
    }
    if let (CellValue::Text(text), Some(type_name)) = (cell, target_type) {
        if let Some(normalized) = normalize_datetime_text(text, type_name) {
            return dialect.literal(&normalized);
        }
    }
    sql_literal(cell, dialect)
}

/// 合法的普通十进制字面量（可带符号/小数点/指数）。只用于目标列已是数值类型的场景，
/// 返回去掉首尾空白的原文本，确保不经过 f64。
fn plain_decimal(text: &str) -> Option<&str> {
    let value = text.trim();
    if value.is_empty() {
        return None;
    }
    let mut chars = value.chars().peekable();
    if matches!(chars.peek(), Some('+' | '-')) {
        chars.next();
    }
    let mut digits = 0usize;
    let mut dot = false;
    while let Some(c) = chars.peek().copied() {
        if c.is_ascii_digit() {
            digits += 1;
            chars.next();
        } else if c == '.' && !dot {
            dot = true;
            chars.next();
        } else {
            break;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(chars.peek(), Some('e' | 'E')) {
        chars.next();
        if matches!(chars.peek(), Some('+' | '-')) {
            chars.next();
        }
        let mut exp_digits = 0usize;
        while let Some(c) = chars.next() {
            if !c.is_ascii_digit() {
                return None;
            }
            exp_digits += 1;
        }
        if exp_digits == 0 {
            return None;
        }
    } else if chars.next().is_some() {
        return None;
    }
    Some(value)
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
    // 小数超 6 位截断（datetime2 上限 7 位，9 位的 DateTime64(9) 直接 241）。
    // 只截小数数字本身，尾随的时区偏移要保留。
    if let Some(dot) = s.find('.') {
        let digit_start = dot + 1;
        let digit_end = s[digit_start..]
            .find(|c: char| !c.is_ascii_digit())
            .map(|offset| digit_start + offset)
            .unwrap_or(s.len());
        if digit_end.saturating_sub(digit_start) > 6 {
            let mut clipped = s[..digit_start + 6].to_string();
            clipped.push_str(&s[digit_end..]);
            s = clipped;
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
pub(crate) fn insert_stmt_cap(kind_key: &str) -> usize {
    // 调优/排查逃生阀：`DBMIND_SYNC_STMT_CAP`（字节）覆盖方言默认值。
    // 健康链路上报文是越大越省往返，劣化链路（丢包/PMTU 黑洞）上大报文会撞重传退避 ——
    // 两者结论相反，所以留一个不改代码就能对比的旋钮；下限仍按 STMT_CAP_MIN 夹住。
    if let Some(cap) = std::env::var("DBMIND_SYNC_STMT_CAP")
        .ok()
        .and_then(|text| text.trim().parse::<usize>().ok())
        .filter(|bytes| *bytes >= STMT_CAP_MIN)
    {
        return cap;
    }
    // 数值由各数据源自己的写插件声明（`plugins/<kind>_writer.rs` 的 `SPEC`）：
    // - Doris 512KB（FE 的 max_allowed_packet 默认 1MB，留足语句头/转义余量）；
    // - ClickHouse / 本地嵌入式（SQLite/H2/Derby）8MB（列存按 part 写、本地没有丢包）；
    // - MySQL 系 1MB（一条几 MB 的 INSERT 在公网/VPN 上一丢包就整条重传，真机撞的是 8MB 一条）；
    // - 其余 4MB（没有 packet 硬限制，但仍受链路丢包约束）。
    crate::api::transfer::plugins::spec_data_for(kind_key).stmt_cap()
}

/// 起步时的单条 INSERT 上限（**先小后大**）：不管方言允许多大，头几批都从 512KB 开始。
///
/// 为什么：`insert_stmt_cap` 是「协议允许的上限」，不代表「这条链路吃得下」。一条语句
/// 一旦撞上丢包，代价是几十秒白等 + 一段回滚 + 一次重跑；而小报文只要能跑通，几条语句
/// 之后 `adapt_stmt_cap` 就会按实测耗时（<1 秒/条）翻倍上去 —— 健康链路只多开头几次往返，
/// 劣化链路则停在真正安全的报文大小上，不会一上来就把链路撞断。
pub(crate) const STMT_CAP_START: usize = 512 * 1024;

/// 各方言默认的**每批条数**（一条 INSERT 里的条数）上限（`DBMIND_SYNC_INSERT_ROWS` 可统一覆盖）。
///
/// 判据不是「协议最多能塞多少行」，而是「这条报文丢了要重传多久」：
/// 一条远端语句丢一个包就要整条重传几十秒，行数砍一半，重传代价就砍一半。
/// 所以**远端库统一 1000 行**（与 DBX / Navicat / Kettle commit size 同量级），
/// 本地文件/内存库没有丢包问题，给 5000 行省往返。
///
/// 注意叠加关系：宽表还会先被字节上限切开（见 [`insert_stmt_cap`]），
/// Oracle / DM / DB2 另受「派生表表达式总数 65,535」限制（见 `insert_batch_ranges_with_cap`），
/// 三者取小 —— 所以宽表上实际每行数会更少。
pub(crate) fn insert_rows_default(kind_key: &str) -> usize {
    // 数值由各数据源的写插件声明（`plugins/<kind>_writer.rs` 的 `SPEC`）：
    // ClickHouse 1 万行（一次 INSERT = 一次 part 写入）、Doris 2000 行（512KB 报文刚好用满）、
    // PG 2000 行、SQL Server 1000 行（多行 VALUES 解析器硬限制）、
    // 本地嵌入式 5000 行、其余（含 MySQL 系）1000 行。
    crate::api::transfer::plugins::spec_data_for(kind_key).rows()
}

fn insert_rows_cap(kind_key: &str) -> usize {
    std::env::var("DBMIND_SYNC_INSERT_ROWS")
        .ok()
        .and_then(|text| text.trim().parse::<usize>().ok())
        .filter(|rows| *rows >= 1)
        .unwrap_or_else(|| insert_rows_default(kind_key))
}

/// 单条 INSERT 的字节**下限**：再拆分就只剩往返开销了。
///
/// 曾取 128KB（"128KB 已足够走满一条千兆链路"），真机翻车：目标库在一条 TAP VPN 隧道后面，
/// 上行报文 **60KB 稳过、121KB 必挂**（客户端发出后收不到任何 ACK，TCP 重传退避约 19 秒后
/// 连接重置，驱动只报 `Communications link failure`，看日志像"服务端把我踢了"，实际是包丢了）。
/// 128KB 的下限让 `DBMIND_SYNC_STMT_CAP` 想调小也调不动，只能改代码，所以下限降到 32KB：
/// 链路健康时耗时会迅速把 cap 翻倍回方言上限（见 `adapt_stmt_cap`），代价只是开头几次往返。
pub(crate) const STMT_CAP_MIN: usize = 32 * 1024;

/// 本任务**实际生效**的单条 INSERT 字节上限：方言默认值与任务级建议值取小。
///
/// `hint` 是同一任务里别的表撞出来的结论（见 `GrowCaps::stmt_cap_hint`）：
/// 一条链路证明过「大于 N 字节的报文会卡住」，后面的表就不该再从方言默认值（MySQL 3MB）
/// 开始重新试一遍。`hint == 0` = 还没学到任何结论。
pub(crate) fn effective_stmt_cap(kind_key: &str, hint: usize) -> usize {
    let kind_cap = insert_stmt_cap(kind_key);
    if hint == 0 {
        kind_cap
    } else {
        kind_cap.min(hint.max(STMT_CAP_MIN))
    }
}

/// 按「平均每条语句的耗时」调整单条 INSERT 的字节上限。
///
/// 为什么需要：语句就是 MySQL 协议里的一个 packet，报文越大、丢一个包要重传的代价越高。
/// 真机（走 SSH 隧道 + 云端 RDS，链路有 PMTU 黑洞）：20 万行的一页被压成 3MB 一条语句，
/// 单条耗时 113 秒（176 行/秒）；而同一任务里别的表能跑到 1 万行/秒 —— 差的就是报文大小
/// 撞上丢包后的重传退避。反过来在健康链路上小报文只是多几次往返。
///
/// 判据取「每条语句」而不是「每页」：一条语句慢就减半，快就翻倍，最终收敛到
/// 「一条语句 1~5 秒」；上限仍是方言的 packet 上限，不会比原来更激进。
fn adapt_stmt_cap(cap: usize, per_stmt_ms: u64, max_cap: usize) -> usize {
    if per_stmt_ms > 5_000 {
        (cap / 2).max(STMT_CAP_MIN)
    } else if per_stmt_ms < 1_000 {
        (cap.saturating_mul(2)).min(max_cap)
    } else {
        cap
    }
}

/// 单行 tuple 在 SQL 里的体积**上界**粗估：Text 按字符数 ×2（转义/引号翻倍余量），
/// 数值/时间列按 40 估；逐字符精算不值得，估大了只是多拆一条语句。
pub(crate) fn estimate_row_bytes(row: &[CellValue]) -> usize {
    row.iter()
        .map(|c| match c {
            CellValue::Decimal(s) => s.len() * 2 + 8,
            CellValue::Text(s) => s.len() * 2 + 8,
            // 二进制走十六进制字面量：1 字节 → 2 个字符
            CellValue::Bytes(data) => data.len() * 2 + 8,
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
            CellValue::Decimal(s) => s.len() + 24,
            CellValue::Text(s) => s.len() + 24,
            // JSON 体里二进制用 base64（4/3 膨胀）
            CellValue::Bytes(data) => data.len() / 3 * 4 + 24,
            CellValue::Blob { len } => *len + 24,
            _ => 24,
        })
        .sum::<usize>()
        + 8
}

/// 把一批行切成 INSERT 子批（起点, 行数）。只算范围，不提前拼 SQL：
/// 预编译路径成功时就不需要付出字符串字面量渲染的成本，失败回退时也只渲染
/// 当前子批。
///
/// 拆分是**双重约束**，谁先到谁触发：
/// - 行数：按方言给（见 [`insert_rows_default`]）—— 远端库统一 1000 行一条
///   （SQL Server 的多行 VALUES 本身就是 1000 行解析器硬限制），本地嵌入式 5000 行；
/// - 表达式：Oracle / DM / DB2 的 `(值1, 值2, …)` 派生表有 65,535 个表达式上限，
///   行数必须随列数收窄（130 列时每批约 504 行）；
/// - 字节：单条语句估出来的体积不许超过目标方言的 packet 上限（见 insert_stmt_cap）——
///   行数上限对宽行（JSON 列等）无能为力，宽行会先被字节上限切开。
///
/// 写路径实际调用的是 [`insert_batch_ranges_with_cap`]（字节上限按实测耗时自适应），
/// 这个包装只剩测试在用，故按测试构建裁剪，免得常驻 `never used` 警告。
#[cfg(test)]
fn insert_batch_ranges(
    rows: &[Vec<CellValue>],
    kind_key: &str,
) -> Vec<(usize, usize)> {
    insert_batch_ranges_with_cap(
        rows,
        kind_key,
        insert_stmt_cap(kind_key),
        insert_rows_cap(kind_key),
    )
}

/// 同 [`insert_batch_ranges`]，但字节上限与行数上限由调用方给：写线程的字节上限按实测耗时
/// 自适应（见 `adapt_stmt_cap`），行数上限来自界面（0 = 方言默认，见 [`insert_rows_cap`]）。
fn insert_batch_ranges_with_cap(
    rows: &[Vec<CellValue>],
    kind_key: &str,
    byte_cap: usize,
    rows_cap: usize,
) -> Vec<(usize, usize)> {
    const SQLSERVER_VALUES_LIMIT: usize = 1000;
    let columns = rows.first().map(Vec::len).unwrap_or(1).max(1);
    // 0 = 调用方没指定 ⇒ 按方言默认（界面把它显示出来只是「填好默认值」，不是必须传）
    let rows_cap = if rows_cap > 0 {
        rows_cap
    } else {
        insert_rows_cap(kind_key)
    };
    let row_limit = match crate::api::transfer::plugins::spec_data_for(kind_key).expr_limit() {
        ExprLimit::SqlServer => SQLSERVER_VALUES_LIMIT.min(rows_cap),
        // 这些方言的单条多行 INSERT 用派生表展开，表达式总数是硬限制。
        // 10 的下限只防止极端宽表算出 0；真正写不进去时照常逐行降级并报错。
        ExprLimit::OracleLike => (65_535 / columns).clamp(10, 10_000).min(rows_cap),
        ExprLimit::None => rows_cap,
    };
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut acc_bytes = 0usize;
    let mut acc_rows = 0usize;
    for (i, row) in rows.iter().enumerate() {
        let row_bytes = estimate_row_bytes(row);
        if acc_rows >= row_limit || (acc_rows > 0 && acc_bytes + row_bytes > byte_cap) {
            out.push((start, i - start));
            start = i;
            acc_bytes = 0;
            acc_rows = 0;
        }
        acc_bytes += row_bytes;
        acc_rows += 1;
    }
    if start < rows.len() {
        out.push((start, rows.len() - start));
    }
    out
}

/// 预编译批处理只用于"提交/回滚边界由驱动可靠提供"的 JDBC 方言。
///
/// MySQL 系的多行 INSERT 本来就能在一个 packet 内完成，ClickHouse/Doris 也各有
/// 更快的原生批量通道；把它们塞进 executeBatch 反而多一层绑定开销。其余类型
/// 继续走已验证的字面量批处理，功能上仍是任意源/目标。
///
/// **DB2 不在名单里**（2026-10 真机实测）：jcc 的 `executeBatch` 一次 200 行时整批
/// 通信中断（`[jcc][103][10843] 不可恢复的通信断开`，SQLState 08001），且失败后连接
/// 已关闭、`rollback` 报 08003 ⇒ 宿主只能判「无法确认回滚，禁止重试」，整张表同步失败。
/// 同样的行数拆成字面量 INSERT 逐条发送完全正常（每条报文小）。预编译批省下的那点
/// 绑定开销，远不值得把一条 VPN / 长肥管道上的链路变成「整表必失败」。
fn prepared_insert_candidate(kind_key: &str) -> bool {
    // 名单由各数据源的写插件声明（`SPEC.prepared()`）：SQL Server / Oracle / DM / H2。
    // **DB2 不在里面**（见上面那段真机结论），MySQL 系有自己的多行 INSERT 快路径。
    crate::api::transfer::plugins::spec_data_for(kind_key).prepared_batch()
}

const PREPARED_BATCH_ROLLED_BACK: &str = "PREPARED_BATCH_ROLLED_BACK";

/// 事务内做"批量失败 → 逐行重试"时需要的保存点。
///
/// PostgreSQL 在一个语句失败后会把整个事务标记为 aborted；没有保存点时，
/// 后续逐行重试全部只会得到 `current transaction is aborted`。SQL Server
/// 的部分错误也有同样效果。只在确实已经开了事务时使用；否则 SAVEPOINT
/// 本身会在 PG 里开启一个没人负责提交的事务。
fn savepoint_sql(kind: &str, name: &str) -> Option<(String, String, Option<String>)> {
    // 语法家族由写插件的 `SPEC.savepoint(...)` 声明：PG 系要 release，SQL Server 只有两条。
    match crate::api::transfer::plugins::spec_data_for(kind).savepoint() {
        Savepoint::Pg => Some((
            format!("savepoint {name}"),
            format!("rollback to savepoint {name}"),
            Some(format!("release savepoint {name}")),
        )),
        Savepoint::MsSql => Some((
            format!("save transaction {name}"),
            format!("rollback transaction {name}"),
            None,
        )),
        Savepoint::None => None,
    }
}

/// 错误是不是「这条连接已经不可用」这一类（区别于脏数据）。
///
/// 为什么必须单独判：写入失败后本模块会**退化成逐行重试**（对齐 DataX 的脏数据降级：
/// 一行烂数据不该炸掉整批）。但连接断了的时候，每一行都要各自等一次驱动超时 ——
/// 真机实测（SSH 隧道 + 链路 PMTU 黑洞）：每条 3MB 的 INSERT 在 19 秒后报
/// `Communications link failure`，9 千行逐行重试就是 9 千 × 19 秒 ≈ 48 小时，
/// 界面上只有「读取数一直在涨、传输数不动、失败数还是 0」——
/// 也就是用户说的「卡住」。这类错误必须**立刻上抛**，让这张表失败、其它表继续。
fn is_link_level_error(err: &XError) -> bool {
    if err.code.as_deref() == Some("DBMIND-CONN-0003") {
        return true;
    }
    is_link_level_message(&err.message)
}

/// 写语句「慢」与「连接已断」的分界：**定期探活**，而不是给一条按报文大小算的死线。
///
/// 为什么改（用户口径：Navicat / DBX 不会中断，我们也不许）：
/// 原来按报文大小给死线（20 秒 + 5 秒/MB，封顶 120 秒），到点就判「连接半开」⇒
/// 回滚本段、换连接重跑。可慢不等于断：一条 3MB 的 INSERT 在丢包链路上正常也要
/// 几十秒，被判定中断后又要整段重读重写，界面上就是「写入中断」反复刷。
///
/// 现在：每 [`WRITE_PROBE_TICK`] 秒另开一条会话跑一次 `select 1` ——
/// - 探活**通** ⇒ 链路活着，这条语句只是慢，**继续等**（Navicat / DBX 的行为）；
/// - 探活**不通** ⇒ 真的断了，立刻返回链路级错误，上层回滚本片换连接重跑。
/// 这样「真断」的发现速度（≤ tick + 探活超时）比原来的死线还快，但又不会误杀慢语句。
const WRITE_PROBE_TICK: std::time::Duration = std::time::Duration::from_secs(20);

/// 探活本身的墙钟：`select 1` 都回不来，就是链路/目标不可用。
///
/// 取 10 秒：`select 1` 只有几百字节，但丢一个包就要走重传退避（RTO 0.2→0.4→0.8…），
/// 5 秒会把「正在重传但最终会通」误判成断。配合下面的一次复核，偶发抖动不会触发回滚。
const WRITE_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

// 单条写语句的**绝对**上限不写死：由界面的「超时时间」决定（见 [`op_timeout_ms`]）——
// 0 = 继承连接（内核默认 600 秒）、负 = 不限制、正 = 用户填的秒数。
// 它与 DBX 的「操作超时（秒），丢弃连接」是同一个语义，只是我们到点**先探活**：
// 探活通说明只是慢，继续等；探活不通才算断。

/// 目标端判活：另开一条会话跑 `select 1`。
///
/// 会话名由写会话名派生（`internal:sync-probe-<写会话>`）：内核的会话泳道是
/// 「同键一条物理连接」，**不能**跟正在跑语句的那条共用键 —— 否则探活会排在
/// 那条慢语句后面，永远探不出来。
async fn probe_write_link(state: &AppState, target: &str, session: &str) -> bool {
    let probe = format!("internal:sync-probe-{session}");
    matches!(
        tokio::time::timeout(
            WRITE_PROBE_TIMEOUT,
            run_target_in(state, target, "select 1", &probe),
        )
        .await,
        Ok(Ok(_))
    )
}

/// 判活 + **一次复核**：判「断」要付代价（回滚该段、换连接重跑），
/// 不能让链路抖一下（丢个包、RDS 抽一下）就触发。
async fn probe_write_link_confirmed(state: &AppState, target: &str, session: &str) -> bool {
    if probe_write_link(state, target, session).await {
        return true;
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    probe_write_link(state, target, session).await
}

/// 目标端执行一条写语句：慢就等（探活确认链路还活着），只有探活不通才判「断」——
/// 上层据此回滚本片、**换一条新连接**重跑（见 `transfer::engine`）。
async fn run_write_stmt(
    state: &AppState,
    target: &str,
    sql: &str,
    session: &str,
    // 单条语句最多等多久（来自界面「超时时间」；探活通也到这个点为止）
    hard_limit: std::time::Duration,
    // 同一份超时换算成的毫秒数，一并交给内核/宿主：否则内核写死的 600 秒会先到，
    // 界面选「不限制」或 600 秒以上就等于没生效。
    timeout_ms: u64,
) -> XResult<dbmind_core::QueryResult> {
    let started = std::time::Instant::now();
    // 语句自己的 future 必须**原地 pin 住**：只有确认断链时才丢掉它（丢弃 = 取消该语句），
    // 探活通过时继续等同一个 future —— 这才是「等」而不是「重发」。
    let pending = run_target_in_rows_timeout(state, target, sql, session, 1, timeout_ms);
    tokio::pin!(pending);
    loop {
        tokio::select! {
            result = &mut pending => return result,
            _ = tokio::time::sleep(WRITE_PROBE_TICK) => {
                let elapsed = started.elapsed();
                // 报错只陈述事实：多久没返回、探活通不通。原因推断与调参建议属于内部判断，
                // 进 tracing 或根本不写（用户口径：日志里只留数据库原话与可核对的事实）。
                if !probe_write_link_confirmed(state, target, session).await {
                    return Err(XError::internal(format!(
                        "写语句 {} 秒未返回，探活不通",
                        elapsed.as_secs()
                    )));
                }
                if elapsed >= hard_limit {
                    return Err(XError::internal(format!(
                        "写语句 {} 秒未返回（已达超时时间）",
                        elapsed.as_secs()
                    )));
                }
                tracing::debug!(
                    target: "dbmind::sync",
                    session,
                    elapsed_ms = elapsed.as_millis() as u64,
                    bytes = sql.len(),
                    "写语句仍在执行（探活通过），继续等待"
                );
            }
        }
    }
}

/// 去掉驱动塞进 message 的**连接号**记号：`…：(conn=1721) Duplicate entry …`
///
/// MariaDB / MySQL Connector/J 会把服务端连接号拼进报错正文（不一定在行首 ——
/// 我们的 message 是 `SQL 执行失败：(conn=1721) Duplicate entry …`），这个号每建一次
/// 连接就变（1662 / 1721 / 1733…），对用户没有任何意义 —— 用户口径是「这种多余的输出
/// 都不要」。只剥这一层记号，数据库报错的正文一个字都不动。
fn strip_conn_tag(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    loop {
        let Some(start) = rest.find("(conn=") else {
            out.push_str(rest);
            break;
        };
        let after = &rest[start + 6..];
        let Some(end) = after.find(')') else {
            out.push_str(rest);
            break;
        };
        if end == 0 || !after[..end].chars().all(|c| c.is_ascii_digit()) {
            // 不是连接号：原样保留到这一处，继续往后找（rest 一定前进，不会空转）
            out.push_str(&rest[..start + 6]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..start]);
        rest = after[end + 1..].trim_start();
    }
    out
}

/// 砍掉宿主报错里跟在「会话不存在」后面的**内部会话键**。
///
/// 宿主原话是 `会话不存在：internal:sync-wslot00-0#0`，但走到界面上时尾巴会变成
/// 整条会话键 `mysql|连接名|rm-xxx.rds.aliyuncs.com|3306|库|模式|false|ssh-user@host:22/password#3`
/// —— 用户截图反馈：一屏都是这种串，主机名、库名、SSH 账号全摊在外面，还说不出到底哪里错了。
///
/// 处理方式：只保留宿主给的事实「会话不存在」，后面的会话键整段去掉（它是内部标识，
/// 对我们排查也没有额外信息 —— 本轮同步的源/目标在界面上本来就看得见）。
fn strip_session_key(message: &str) -> String {
    const MARK: &str = "会话不存在";
    if let Some(start) = message.find(MARK) {
        let tail = &message[start + MARK.len()..];
        // 只有后面真的跟了会话键（带 `|` 分段）才裁，避免误伤正常句子
        if tail.contains('|') {
            return message[..start + MARK.len()].to_string();
        }
    }
    message.to_string()
}

/// 给用户看的错误正文：剥掉三层噪音，只留数据库原话。
///
/// ① 我们自己加的 `SQL 执行失败：` 标签 —— 日志外面已经有「写入失败：」在说这件事，
///    再来一层就是套娃（用户口径：「这几个字也删掉」）；
/// ② 驱动拼进去的 `(conn=1721)` 连接号 —— 每重连一次就变，对排查无用；
/// ③ 内部会话键（见 [`strip_session_key`]）—— 不是数据库给的信息，界面上一无是处。
///
/// 三层可能交替出现（`SQL 执行失败：(conn=1) SQL 执行失败：…`），所以循环剥到没有为止。
fn error_text(message: &str) -> String {
    let mut text = strip_session_key(message);
    loop {
        let clean = strip_session_key(&strip_conn_tag(&text));
        let Some(rest) = clean.strip_prefix("SQL 执行失败：") else {
            return clean;
        };
        text = rest.trim_start().to_string();
    }
}

/// 日志里只放**原始报错的首行**（超长再截断）。
///
/// 驱动的 message 常带长尾巴（泳道键 `mysql|host|3306|db|...|s=internal:sync-wxxx-1#0`、
/// 「内核将会按需重新连接」这类内部建议），原样铺到界面上既看不懂也读不完 ——
/// 用户口径是「报错就给真实的报错信息」，那我们就把原文留着，只砍掉这些噪音。
fn brief_error(message: &str) -> String {
    let first = error_text(message.lines().next().unwrap_or(""));
    match first.char_indices().nth(180) {
        Some((cut, _)) => format!("{}…", &first[..cut]),
        None => first,
    }
}

/// 「这条连接已经不可用」类错误的特征串（大小写无关）。
///
/// 与 [`is_link_level_error`] 共用一份清单：那条判 `XError`，这条判
/// `transfer::run` 上抛的字符串（写路径只往下传 message，不带错误码）。
pub(crate) fn is_link_level_message(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    [
        "communications link failure",
        "连接已断开",
        "connection reset",
        "broken pipe",
        "socket closed",
        "connection is closed",
        "no operations allowed after connection closed",
        // 宿主确认「无法确认回滚」时连接已经废了，逐行重试没有意义
        "无法确认回滚",
        // 单批墙钟兜底（连接半开：响应永远不来）
        "未返回",
        "半开",
        // 宿主那边这条写会话已经不在了（被配额回收 / 宿主重启）。它不是脏数据：
        // 会话没了 ⇒ 这条语句一定没执行、它所在的事务也已被服务端回滚，
        // 「整表重跑」是安全的（而且必须重跑：那一整页的行都随事务一起没了）。
        // 不认这条的话，用户看到的就是「写入连接已断：会话不存在（agent 可能已重启）」后
        // 本表直接判失败 —— 前面写进事务的几万行全部消失，而任务计数里还算成功。
        "会话不存在",
        // 慢批自愈（探活通但目标端写不动）：这一页的事务会被回滚，换连接 + 缩页重试是安全的
        "写入卡住",
        // ---- 网络本身断了（不是数据库报错）----
        // 真机：任务跑到 85% 时链路抖了十几秒，隧道建连超时，这几张表直接被判失败，
        // 前面 2.5 小时的进度白等。这类错误**重试一定安全**（读幂等、目标端事务回滚），
        // 网络回来就该接着跑，而不是把表标红让人手工重跑。
        "连不上 ssh 跳板机",
        "跳板机地址无法解析",
        "ssh 会话建立失败",
        "ssh 转发通道打开失败",
        "隧道监听已退出",
        "本地转发端口已失效",
        "connection timed out",
        "connect timed out",
        "connection refused",
        "connection aborted",
        "network is unreachable",
        "no route to host",
        "host is unreachable",
        "i/o timeout",
        "operation timed out",
    ]
    .iter()
    .any(|key| text.contains(key))
}

/// 把一批行按「行数+字节拆分」写成 INSERT 语句并逐条发送（**部分成功语义**）。
///
/// **脏数据降级**（对齐 DataX）：某条语句失败时**二分拆批**定位坏行 ——
/// 坏行（日期转换失败、超长、类型不匹配…）单独计失败并记样例错误，好行照常落库，
/// 不让一行烂数据拖死整批（真机：SQL Server 日期转换 241，一个坏值炸掉 11,574 行）。
/// 二分而非逐行：2 万行的批里 1 个坏值，逐行要 2 万次往返（分钟级，期间写会话被独占），
/// 二分只要 log₂N 层。二分中若**过半失败**，判定为系统性问题（连接断/权限/表没了），
/// 立即中止并上抛 —— 不值得把几千行磨完。
/// `diag` 用于把「最近批耗时 / 失败样例」回传给卡顿日志（见 `transfer::engine`）。
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
    in_transaction: bool,
    diag: &crate::api::transfer::WriteDiag,
    stmt_cap: &std::sync::atomic::AtomicUsize,
    // 任务级报文上限建议（整库同步才有；`None` = 单表同步，各写各的）
    hint: Option<&crate::api::transfer::GrowCaps>,
    // 单条 INSERT 的行数上限（界面可改；0 = 按方言默认）
    rows_cap: usize,
    // 界面「超时时间」（秒）：0 = 继承连接、<0 = 不限制、>0 = 该秒数
    op_timeout_secs: i64,
) -> Result<u64, String> {
    let stmt_timeout_ms = op_timeout_ms(op_timeout_secs);
    let stmt_hard_limit = std::time::Duration::from_millis(stmt_timeout_ms);
    // 语句字节上限按**每路 writer** 记（同表不同写线程可能一快一慢），
    // 这一批结束时按平均每条语句耗时调整，影响下一批（见 `adapt_stmt_cap`）。
    // 任务级建议值（别的表撞出来的结论）在这里生效：上限只能更小，不会被放大回去。
    let max_cap = effective_stmt_cap(
        target_kind.kind.key(),
        hint.map(|grow| grow.stmt_cap_hint()).unwrap_or(0),
    );
    let byte_cap = stmt_cap
        .load(std::sync::atomic::Ordering::Relaxed)
        .clamp(STMT_CAP_MIN, max_cap);
    let ranges = insert_batch_ranges_with_cap(rows, target_kind.kind.key(), byte_cap, rows_cap);
    let stmt_count = ranges.len().max(1);
    let batch_started = std::time::Instant::now();
    let mut ok_rows = 0u64;
    let mut failed_rows = 0u64;
    let mut sample_err: Option<String> = None;
    let mut savepoint_seq = 0u64;
    let mut use_prepared =
        !identity_on && prepared_insert_candidate(target_kind.kind.key());
    for (start_row, n_rows) in ranges {
        let chunk = &rows[start_row..start_row + n_rows];
        if use_prepared {
            match run_target_prepared_insert(
                state,
                tgt_target,
                session,
                target_name,
                columns,
                target_types,
                target_kind,
                chunk,
                in_transaction,
            )
            .await
            {
                Ok(written) => {
                    ok_rows += written as u64;
                    continue;
                }
                Err(err) if err.code.as_deref() == Some("DBMIND-DRV-0001") => {
                    // 驱动声明不支持：后面的子批不必再逐个试探。
                    use_prepared = false;
                    tracing::debug!(
                        table,
                        error = %err.message,
                        "目标驱动不支持预编译批量写入，改用 SQL 批处理"
                    );
                }
                Err(err) if err.message.contains(PREPARED_BATCH_ROLLED_BACK) => {
                    // 宿主确认整批已回滚，才允许走原来的字面量/逐行降级；否则
                    // 部分成功后再重试会产生重复数据。
                    tracing::warn!(table, "预编译子批失败（已回滚），本批改用 SQL 批量并逐批拆分定位坏行");
                }
                Err(err) => {
                    return Err(err.message);
                }
            }
        }

        let stmt = build_insert(
            target_name,
            columns,
            target_types,
            chunk,
            target_kind,
        );
        let sp = if in_transaction {
            let name = format!("dbmind_insert_{savepoint_seq}");
            savepoint_seq += 1;
            savepoint_sql(target_kind.kind.key(), &name)
        } else {
            None
        };
        if let Some((begin, _, _)) = sp.as_ref() {
            if let Err(err) = run_target_in(state, tgt_target, begin, session).await {
                return Err(err.message);
            }
        }
        let insert_sql = with_identity(stmt, identity_on, target_name);
        match run_write_stmt(state, tgt_target, &insert_sql, session, stmt_hard_limit, stmt_timeout_ms)
            .await
        {
            Ok(_) => {
                ok_rows += n_rows as u64;
                if let Some((_, _, Some(release))) = sp.as_ref() {
                    // 释放失败**不能**判整表失败：这批数据已经写进去了，
                    // 这里只是回收保存点。以前直接 return Err，界面显示「失败」
                    // 而目标端其实有数据，用户会以为白跑一趟。
                    if let Err(err) = run_target_in(state, tgt_target, release, session).await {
                        tracing::warn!(table, error = %err.message, "释放保存点失败（数据已写入，继续）");
                    }
                }
            }
            Err(batch_err) => {
                // 连接级错误**不能**退化成逐行重试（见 is_link_level_error）：
                // 断了的连接上每一行都要各等一次驱动超时，表现就是「一行不写也不报错」。
                if is_link_level_error(&batch_err) {
                    // 断连的常见真因不是"服务端把我踢了"，而是**上行报文太大**撞上链路的
                    // 重传退避：真机实测同一条 TAP 隧道，60KB 的 INSERT 稳过、121KB 必挂，
                    // 客户端发完收不到 ACK，19 秒后连接重置，驱动只报 Communications link
                    // failure。这里把单条语句的字节上限减半，后面重跑该片就用更小的包探路；
                    // 只在失败时下调，健康链路不会因此变慢。
                    let shrunk = (byte_cap / 2).max(STMT_CAP_MIN);
                    if shrunk < byte_cap {
                        stmt_cap.store(shrunk, std::sync::atomic::Ordering::Relaxed);
                    }
                    // 结论同步给**整个任务**：后面的表直接从更小的报文开工，
                    // 不必各自再断一次（几百张表各撞一次 = 几百条吓人日志）。
                    // 返回值（本任务共享后的最终值）只服务日志，而日志不再报文上限，故忽略。
                    if let Some(grow) = hint {
                        let _ = grow.note_stmt_cap(shrunk);
                    }
                    if let Some(task) = task {
                        // **不记失败行**：这一批所在的分片会被上层回滚并重跑（片级重试），
                        // 这些行随后会被重新写入。以前在这里 `add_rows_failed(left)`，
                        // 重跑成功后它们就成了永远抹不掉的「10 万行写入失败」
                        //（真机：目标表一行不缺、结果却报失败 10 万）。
                        // 日志只报**原始报错**，不拼「写入失败：」这类前缀；
                        // 行数 / 报文上限 / 是否沿用都属于内部量，不写。
                        task.log(format!("表 {table}：{}", brief_error(&batch_err.message)));
                    }
                    return Err(brief_error(&batch_err.message));
                }
                if let Some((_, rollback, _)) = sp.as_ref() {
                    if let Err(err) = run_target_in(state, tgt_target, rollback, session).await {
                        return Err(err.message);
                    }
                }
                if let Some((_, _, Some(release))) = sp.as_ref() {
                    if let Err(err) = run_target_in(state, tgt_target, release, session).await {
                        return Err(err.message);
                    }
                }
                // 降级：**二分拆批**定位坏行（原来是逐行重试）。
                //
                // 为什么不再逐行：2 万行的批里只要有 1 个坏值，逐行就是 2 万次往返
                //（每次一条 INSERT + 保存点），唯一的写会话被占住几十秒到几分钟，
                // 读侧只能卡在「写池 30 秒没收下新的一批」上 —— 真机上一张 92 万行的表
                // 就这样落后 40 万行、失败数缓慢爬到 12.7 万。
                // 二分只需 log₂(2 万)≈15 层、约 2×15 次尝试就能把坏行逼出来。
                let mut stack: Vec<(usize, usize)> = vec![(start_row, n_rows)];
                let mut chunk_failed = 0u64;
                while let Some((sub_start, sub_len)) = stack.pop() {
                    let sub = &rows[sub_start..sub_start + sub_len];
                    let stmt = build_insert(target_name, columns, target_types, sub, target_kind);
                    let sub_sp = if in_transaction {
                        let name = format!("dbmind_insert_{savepoint_seq}");
                        savepoint_seq += 1;
                        savepoint_sql(target_kind.kind.key(), &name)
                    } else {
                        None
                    };
                    if let Some((begin, _, _)) = sub_sp.as_ref() {
                        if let Err(err) = run_target_in(state, tgt_target, begin, session).await {
                            return Err(err.message);
                        }
                    }
                    let sub_sql = with_identity(stmt, identity_on, target_name);
                    match run_write_stmt(
                        state,
                        tgt_target,
                        &sub_sql,
                        session,
                        stmt_hard_limit,
                        stmt_timeout_ms,
                    )
                    .await
                    {
                        Ok(_) => {
                            ok_rows += sub_len as u64;
                            if let Some((_, _, Some(release))) = sub_sp.as_ref() {
                                // 已经写成功，释放保存点失败不该把它改判成失败。
                                if let Err(err) = run_target_in(state, tgt_target, release, session).await {
                                    tracing::warn!(table, error = %err.message, "释放保存点失败（数据已写入，继续）");
                                }
                            }
                        }
                        Err(sub_err) => {
                            // 连接级错误直接收摊：继续拆下去只会把「一秒能报的错」
                            // 变成几小时的「卡住」（见 is_link_level_error）。
                            if is_link_level_error(&sub_err) {
                                if let Some(task) = task {
                                    // 同上：不记失败行，交给上层「回滚本片并重跑」；
                                    // 也不报剩余行数（内部量，用户拿到没用），不拼前缀
                                    task.log(format!("表 {table}：{}", brief_error(&sub_err.message)));
                                }
                                return Err(brief_error(&sub_err.message));
                            }
                            if let Some((_, rollback, _)) = sub_sp.as_ref() {
                                if let Err(err) = run_target_in(state, tgt_target, rollback, session).await {
                                    return Err(err.message);
                                }
                            }
                            if let Some((_, _, Some(release))) = sub_sp.as_ref() {
                                if let Err(err) = run_target_in(state, tgt_target, release, session).await {
                                    return Err(err.message);
                                }
                            }
                            if sub_len <= 1 {
                                // 单行仍失败 = 真坏行：计失败、留样例，兄弟区间继续写
                                chunk_failed += 1;
                                // 留**完整**的数据库原文：MySQL 的 `Duplicate entry '…' for
                                // key '表.索引'` 带上长主键值就两百多字符，截在 200 恰好把
                                // 索引名/后半截键值切掉，用户拿不到能对号的错误（真机反馈）。
                                let text: String =
                                    error_text(&sub_err.message).chars().take(500).collect();
                                if sample_err.is_none() {
                                    sample_err = Some(text.clone());
                                    // 同时喂给写池诊断：卡顿日志里能直接看到「坏在哪」
                                    diag.note_sample_err(&text);
                                }
                                // 坏行过半就**提前收摊**：坏行占 45% 这类情况，
                                // 二分树会展开到接近 2×行数 次尝试（与逐行同量级），
                                // 越拆越慢。判成系统性问题（表结构不兼容/权限/连接语义变了）
                                // 比磨完几万次更有用。
                                if chunk_failed * 2 > n_rows as u64 {
                                    if let Some(task) = task {
                                        task.add_rows_failed(rows.len() as u64 - ok_rows);
                                    }
                                    // 用户口径：括号里的解释一律不要，只留失败行数 + 数据库原文。
                                    return Err(format!(
                                        "本批 {n_rows} 行里有 {chunk_failed} 行写入失败：{text}"
                                    ));
                                }
                            } else {
                                // 保序拆半：先压后半、再压前半，弹出时先处理前半，
                                // 写入顺序与源表一致（定位日志也读得顺）。
                                let half = sub_len / 2;
                                stack.push((sub_start + half, sub_len - half));
                                stack.push((sub_start, half));
                            }
                        }
                    }
                }
                failed_rows += chunk_failed;
                if chunk_failed > 0 {
                    if let Some(task) = task {
                        // 用户口径：**只要程序/数据库报出的原始错误**。不要「二分定位」这类
                        // 实现术语，也不要「其余已写入」这类自造的叙述 —— 失败了多少行 +
                        // 数据库原文，两样就够：`Duplicate entry …` / `Data too long …` /
                        // `Incorrect string value …` 各自对应完全不同的处理动作。
                        // 只有统计（多少行没写进去）+ 数据库原话；原话过一遍 brief_error，
                        // 免得又冒出「SQL 执行失败：」「(conn=…)」这类套壳
                        let why = sample_err
                            .as_deref()
                            .map(brief_error)
                            .unwrap_or_default();
                        if why.is_empty() {
                            task.log(format!(
                                "表 {table}：本批 {n_rows} 行里有 {chunk_failed} 行写入失败"
                            ));
                        } else {
                            task.log(format!(
                                "表 {table}：本批 {n_rows} 行里有 {chunk_failed} 行写入失败：{why}"
                            ));
                        }
                    }
                }
                // 过半失败 ⇒ 系统性问题（连接断/权限/表没了），逐行磨完没有意义
                if chunk_failed * 2 > n_rows as u64 {
                    if let Some(task) = task {
                        task.add_rows_failed(rows.len() as u64 - ok_rows);
                    }
                    return Err(sample_err.unwrap_or_else(|| error_text(&batch_err.message)));
                }
            }
        }
    }
    if failed_rows > 0 {
        if let Some(task) = task {
            // 只累计计数：每条失败的原因与行数已经在各自的批日志里报过（数据库原文），
            // 这里再打一条汇总就是重复输出（用户口径：多余的输出都不要）。
            task.add_rows_failed(failed_rows);
        }
    }
    // 写入计数在这里统一报（串行管道与并发写池共用本函数 —— 之前并发写池的重构
    // 把调用方的 add_rows_written 弄丢了，界面上「传输数」全程为 0，真机踩过）
    if ok_rows > 0 {
        if let Some(task) = task {
            task.add_rows_written(ok_rows);
        }
    }
    // 整批写完：按平均每条语句耗时把字节上限调一档（慢→减半，快→翻倍，见 `adapt_stmt_cap`）
    let per_stmt_ms = batch_started.elapsed().as_millis() as u64 / stmt_count as u64;
    let next_cap = adapt_stmt_cap(byte_cap, per_stmt_ms, max_cap);
    if next_cap != byte_cap {
        stmt_cap.store(next_cap, std::sync::atomic::Ordering::Relaxed);
        tracing::debug!(
            target: "dbmind::sync",
            table,
            per_stmt_ms,
            from = byte_cap,
            to = next_cap,
            "调整单条 INSERT 的字节上限"
        );
    }
    Ok(ok_rows)
}

/// 生成一批**原生 upsert** 语句（(行数, SQL)）。拆分约束与 INSERT 完全一致：
/// 行数（SQL Server 单语句 1000 行）+ 字节（目标方言的 packet 上限，宽行不能靠行数兜住）。
fn build_upsert_batch(
    table: &str,
    columns: &[String],
    types: &[Option<String>],
    key_indexes: &[usize],
    rows: &[Vec<CellValue>],
    dialect: Dialect,
    // 界面指定的单条行数上限（0 = 按方言默认）
    rows_cap: usize,
) -> Vec<(usize, String)> {
    // 原生 upsert 与 INSERT 同一粒度：方言上限（SQL Server 1000 行 / 其它）、
    // 界面填的行数、方言默认三者取小。
    let rows_cap = if rows_cap > 0 {
        rows_cap
    } else {
        insert_rows_cap(dialect.kind.key())
    };
    let row_limit = dialect.upsert_row_limit().min(rows_cap);
    let byte_cap = insert_stmt_cap(dialect.kind.key());
    let render = |slice: &[Vec<CellValue>]| -> Option<String> {
        let rendered: Vec<Vec<String>> = slice
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(i, cell)| {
                        literal_for(cell, dialect, types.get(i).and_then(|t| t.as_deref()))
                    })
                    .collect()
            })
            .collect();
        dialect.upsert_sql(table, columns, key_indexes, &rendered)
    };
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut start = 0usize;
    let mut acc_bytes = 0usize;
    let mut acc_rows = 0usize;
    for (i, row) in rows.iter().enumerate() {
        let row_bytes = estimate_row_bytes(row);
        if acc_rows >= row_limit || (acc_rows > 0 && acc_bytes + row_bytes > byte_cap) {
            if let Some(sql) = render(&rows[start..i]) {
                out.push((i - start, sql));
            }
            start = i;
            acc_bytes = 0;
            acc_rows = 0;
        }
        acc_bytes += row_bytes;
        acc_rows += 1;
    }
    if start < rows.len() {
        if let Some(sql) = render(&rows[start..]) {
            out.push((rows.len() - start, sql));
        }
    }
    out
}

/// 把一批行用**原生 upsert** 写进目标（一条语句里 `on duplicate key update` /
/// `on conflict … do update` / `merge into`），返回写入行数。
///
/// 与 `write_insert_rows` 共用同一套「脏数据降级」策略：整块失败就逐行重试，
/// 好行照常落库、坏行单独计失败；过半失败判定为系统性问题直接上抛。
/// 差别只有一处：语句本身就能处理「已存在的行」，所以不需要事先知道目标主键。
pub(crate) async fn write_upsert_rows(
    state: &AppState,
    tgt_target: &str,
    session: &str,
    target_name: &str,
    columns: &[String],
    target_types: &[Option<String>],
    key_indexes: &[usize],
    target_kind: Dialect,
    rows: &[Vec<CellValue>],
    identity_on: bool,
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
    table: &str,
    in_transaction: bool,
    // 单条 INSERT 的行数上限（界面可改；0 = 按方言默认）
    rows_cap: usize,
) -> Result<u64, String> {
    let stmts = build_upsert_batch(
        target_name,
        columns,
        target_types,
        key_indexes,
        rows,
        target_kind,
        rows_cap,
    );
    if stmts.is_empty() {
        return Err(format!(
            "{} 不支持原生 upsert，且没有可回退的两阶段写入",
            target_kind.kind.key()
        ));
    }
    let mut ok_rows = 0u64;
    let mut failed_rows = 0u64;
    let mut sample_err: Option<String> = None;
    // 脏数据明细文件（第一次真坏行时创建，之后所有坏行追加进去）
    let mut start_row = 0usize;
    let mut savepoint_seq = 0u64;
    for (n_rows, stmt) in &stmts {
        let sp = if in_transaction {
            let name = format!("dbmind_upsert_{savepoint_seq}");
            savepoint_seq += 1;
            savepoint_sql(target_kind.kind.key(), &name)
        } else {
            None
        };
        if let Some((begin, _, _)) = sp.as_ref() {
            if let Err(err) = run_target_in(state, tgt_target, begin, session).await {
                return Err(err.message);
            }
        }
        match run_target_in(state, tgt_target, &with_identity(stmt.clone(), identity_on, target_name), session).await {
            Ok(_) => {
                ok_rows += *n_rows as u64;
                if let Some((_, _, Some(release))) = sp.as_ref() {
                    // 数据已写入，释放保存点失败只记日志（理由见 write_insert_rows）。
                    if let Err(err) = run_target_in(state, tgt_target, release, session).await {
                        tracing::warn!(table, error = %err.message, "释放 upsert 保存点失败（数据已写入，继续）");
                    }
                }
            }
            Err(batch_err) => {
                if let Some((_, rollback, _)) = sp.as_ref() {
                    if let Err(err) = run_target_in(state, tgt_target, rollback, session).await {
                        return Err(err.message);
                    }
                }
                if let Some((_, _, Some(release))) = sp.as_ref() {
                    if let Err(err) = run_target_in(state, tgt_target, release, session).await {
                        return Err(err.message);
                    }
                }
                // 先把**批次本身的错误**记下来：降级逐行重试之后，行的错误往往是
                // 「事务已被中止」这类二次错误，真正的原因（语法/类型/约束）只有这里能看到。
                tracing::warn!(
                    table,
                    error = %batch_err.message.chars().take(400).collect::<String>(),
                    "upsert 批次失败，改为逐行重试"
                );
                let chunk = &rows[start_row..start_row + *n_rows as usize];
                let mut chunk_failed = 0u64;
                for row in chunk {
                    let row_sp = if in_transaction {
                        let name = format!("dbmind_upsert_row_{savepoint_seq}");
                        savepoint_seq += 1;
                        savepoint_sql(target_kind.kind.key(), &name)
                    } else {
                        None
                    };
                    if let Some((begin, _, _)) = row_sp.as_ref() {
                        if let Err(err) = run_target_in(state, tgt_target, begin, session).await {
                            return Err(err.message);
                        }
                    }
                    let single = build_upsert_batch(
                        target_name,
                        columns,
                        target_types,
                        key_indexes,
                        std::slice::from_ref(row),
                        target_kind,
                        rows_cap,
                    );
                    let sql = match single.first() {
                        Some((_, sql)) => sql.clone(),
                        None => {
                            if let Some((_, rollback, _)) = row_sp.as_ref() {
                                let _ = run_target_in(state, tgt_target, rollback, session).await;
                            }
                            if let Some((_, _, Some(release))) = row_sp.as_ref() {
                                let _ = run_target_in(state, tgt_target, release, session).await;
                            }
                            chunk_failed += 1;
                            continue;
                        }
                    };
                    match run_target_in(state, tgt_target, &with_identity(sql, identity_on, target_name), session).await {
                        Ok(_) => {
                            ok_rows += 1;
                            if let Some((_, _, Some(release))) = row_sp.as_ref() {
                                // 数据已写入，释放保存点失败只记日志。
                                if let Err(err) = run_target_in(state, tgt_target, release, session).await {
                                    tracing::warn!(table, error = %err.message, "释放 upsert 行保存点失败（数据已写入，继续）");
                                }
                            }
                        }
                        Err(row_err) => {
                            // 连接级错误不能当坏行：断网时每一行都会失败一次，逐行磨下去
                            // 就是把「一秒能报的错」拖成十几分钟（真机实测：断开目标库 25 秒，
                            // 这里磨了 659 秒、还留下 34 行「写入失败」，数据其实一行没少）。
                            // 直接上抛，交给上层回滚本片 → 表级重跑。
                            if is_link_level_error(&row_err) {
                                if let Some(task) = task {
                                    task.log(format!("表 {table}：{}", brief_error(&row_err.message)));
                                }
                                return Err(brief_error(&row_err.message));
                            }
                            if let Some((_, rollback, _)) = row_sp.as_ref() {
                                if let Err(err) = run_target_in(state, tgt_target, rollback, session).await {
                                    return Err(err.message);
                                }
                            }
                            if let Some((_, _, Some(release))) = row_sp.as_ref() {
                                if let Err(err) = run_target_in(state, tgt_target, release, session).await {
                                    return Err(err.message);
                                }
                            }
                            chunk_failed += 1;
                            if sample_err.is_none() {
                                sample_err =
                                    Some(error_text(&row_err.message).chars().take(500).collect());
                            }
                        }
                    }
                }
                failed_rows += chunk_failed;
                if chunk_failed * 2 > *n_rows as u64 {
                    if let Some(task) = task {
                        task.add_rows_failed(rows.len() as u64 - ok_rows);
                    }
                    return Err(sample_err.unwrap_or_else(|| error_text(&batch_err.message)));
                }
            }
        }
        start_row += *n_rows as usize;
    }
    if failed_rows > 0 {
        if let Some(task) = task {
            task.add_rows_failed(failed_rows);
            task.log(format!(
                "表 {table}：{failed_rows} 行写入失败：{}",
                sample_err.unwrap_or_default()
            ));
        }
    }
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
    // 由写插件的 `SPEC.multi_statement()` 声明（MySQL 系带 allowMultiQueries、T-SQL 原生、
    // pgjdbc 允许一次多条）。Oracle/DB2/DM 报「无效字符」，H2/Derby/ClickHouse 不认。
    crate::api::transfer::plugins::spec_data_for(kind).multi_statement()
}

/// 把攒下的 UPDATE 语句**一次**发出去；整批失败时**降级逐行重试**（与 INSERT 同一套
/// 脏数据策略：一行坏数据不该拖死一整批）。UPDATE 本身幂等，重试是安全的。
/// 返回成功更新的行数。
pub(crate) async fn write_update_batch(
    state: &AppState,
    tgt_target: &str,
    kind: &str,
    session: &str,
    stmts: &mut Vec<String>,
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
    table: &str,
    in_transaction: bool,
) -> Result<u64, String> {
    if stmts.is_empty() {
        return Ok(0);
    }
    let total = stmts.len() as u64;
    let mut ok = 0u64;
    let joined = stmts.join(";\n");
    let mut savepoint_seq = 0u64;
    let sp = if in_transaction {
        let name = format!("dbmind_update_{savepoint_seq}");
        savepoint_seq += 1;
        savepoint_sql(kind, &name)
    } else {
        None
    };
    if let Some((begin, _, _)) = sp.as_ref() {
        if let Err(err) = run_target_in(state, tgt_target, begin, session).await {
            return Err(err.message);
        }
    }
    match run_target_in(state, tgt_target, &joined, session).await {
        Ok(_) => {
            ok = total;
            if let Some((_, _, Some(release))) = sp.as_ref() {
                // 更新已经写进去了，释放保存点失败只记日志。
                if let Err(err) = run_target_in(state, tgt_target, release, session).await {
                    tracing::warn!(table, error = %err.message, "释放更新保存点失败（数据已写入，继续）");
                }
            }
        }
        Err(batch_err) => {
            if let Some((_, rollback, _)) = sp.as_ref() {
                if let Err(err) = run_target_in(state, tgt_target, rollback, session).await {
                    return Err(err.message);
                }
            }
            if let Some((_, _, Some(release))) = sp.as_ref() {
                if let Err(err) = run_target_in(state, tgt_target, release, session).await {
                    return Err(err.message);
                }
            }
            let mut failed = 0u64;
            let mut sample: Option<String> = None;
            for sql in stmts.iter() {
                let row_sp = if in_transaction {
                    let name = format!("dbmind_update_row_{savepoint_seq}");
                    savepoint_seq += 1;
                    savepoint_sql(kind, &name)
                } else {
                    None
                };
                if let Some((begin, _, _)) = row_sp.as_ref() {
                    if let Err(err) = run_target_in(state, tgt_target, begin, session).await {
                        return Err(err.message);
                    }
                }
                match run_target_in(state, tgt_target, sql, session).await {
                    Ok(_) => {
                        ok += 1;
                        if let Some((_, _, Some(release))) = row_sp.as_ref() {
                            // 这一行已经更新成功，释放保存点失败只记日志。
                            if let Err(err) = run_target_in(state, tgt_target, release, session).await {
                                tracing::warn!(table, error = %err.message, "释放更新行保存点失败（数据已写入，继续）");
                            }
                        }
                    }
                    Err(err) => {
                        if let Some((_, rollback, _)) = row_sp.as_ref() {
                            if let Err(rb_err) = run_target_in(state, tgt_target, rollback, session).await {
                                return Err(rb_err.message);
                            }
                        }
                        if let Some((_, _, Some(release))) = row_sp.as_ref() {
                            if let Err(rb_err) = run_target_in(state, tgt_target, release, session).await {
                                return Err(rb_err.message);
                            }
                        }
                        failed += 1;
                        if sample.is_none() {
                            sample = Some(error_text(&err.message).chars().take(500).collect());
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

fn build_prepared_insert(table: &str, columns: &[String], dialect: Dialect) -> String {
    format!(
        "insert into {table} ({}) values ({})",
        columns
            .iter()
            .map(|name| dialect.quote(name))
            .collect::<Vec<_>>()
            .join(", "),
        std::iter::repeat_n("?", columns.len())
            .collect::<Vec<_>>()
            .join(", ")
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

/// 两阶段 upsert 的**有界**目标键查询：只查当前源页里出现的键，
/// 不再把目标整表主键拉进一个 HashSet。
///
/// 会话必须是 **`internal:sync-u{sid}`**（两阶段更新用的那条），不能借 0 号 writer：
/// 宿主只对 tables/columns/事务语句按 sessionId 加锁，`query` 不加锁 —— 用 writer0 的会话
/// 会让这条 SELECT 与正在进行的 INSERT 压在**同一条 JDBC Connection** 上（驱动不是线程安全的，
/// Oracle 上就是这么把连接搅坏的）。而 update 会话与 lookup 都在派发线程里顺序执行
/// （本函数返回后才调 `writer.update`）。它本身是每表一条常驻连接，`db()` 的会话
/// 预算已经为两阶段 upsert 预留。
pub(crate) async fn lookup_existing_keys(
    plan: &crate::api::transfer::TablePlan,
    rows: &[Vec<CellValue>],
) -> Result<std::collections::HashSet<String>, String> {
    if rows.is_empty() || plan.key_indexes.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    // 单列 IN 列表在 MySQL/ClickHouse 可以更大；复合键的 OR 链仍保守限 500。
    let lookup_rows = if plan.key_indexes.len() == 1 {
        match plan.target_kind.kind.key() {
            "clickhouse" => 2_000,
            "mysql" | "mariadb" | "doris" => 1_000,
            _ => 500,
        }
    } else {
        500
    };
    let key_columns = plan
        .key_indexes
        .iter()
        .map(|index| plan.target_kind.quote(&plan.target_columns[*index]))
        .collect::<Vec<_>>();
    let key_positions: Vec<usize> = (0..plan.key_indexes.len()).collect();
    let session = format!("internal:sync-u{}", plan.sid);
    let mut found = std::collections::HashSet::new();
    for chunk in rows.chunks(lookup_rows) {
        let lits: Vec<Vec<String>> = chunk
            .iter()
            .map(|row| {
                plan.key_indexes
                    .iter()
                    .map(|index| {
                        literal_for(
                            &row[*index],
                            plan.target_kind,
                            plan.target_types.get(*index).and_then(|ty| ty.as_deref()),
                        )
                    })
                    .collect()
            })
            .collect();
        let predicate = if key_columns.len() == 1 {
            format!(
                "{} in ({})",
                key_columns[0],
                lits.iter().map(|vals| vals[0].clone()).collect::<Vec<_>>().join(", ")
            )
        } else {
            lits.iter()
                .map(|vals| {
                    let eq = key_columns
                        .iter()
                        .zip(vals)
                        .map(|(column, value)| format!("{column} = {value}"))
                        .collect::<Vec<_>>()
                        .join(" and ");
                    format!("({eq})")
                })
                .collect::<Vec<_>>()
                .join(" or ")
        };
        let sql = format!(
            "select {}, count(*) as n from {} where {} group by {}",
            key_columns.join(", "),
            plan.target_name,
            predicate,
            key_columns.join(", ")
        );
        let result = run_target_in_rows(
            &plan.state,
            &plan.tgt_target,
            &sql,
            &session,
            chunk.len(),
        )
        .await
        .map_err(|e| e.message)?;
        for row in &result.rows {
            let key = key_of(row, &key_positions);
            let count = row
                .get(key_positions.len())
                .and_then(|cell| match cell {
                    CellValue::Integer(n) => Some(*n as u64),
                    CellValue::Real(n) => Some(*n as u64),
                    CellValue::Text(text) => text.trim().parse::<u64>().ok(),
                    _ => None,
                })
                .unwrap_or(0);
            if count > 1 {
                return Err(format!(
                    "目标表同一个比对键对应 {count} 行（键 {key}）；两阶段 upsert 已中止，避免批量更新错行"
                ));
            }
            if count == 1 {
                found.insert(key);
            }
        }
    }
    Ok(found)
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
    slot: usize,
    reuse: bool,
    progress: &std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> ObjResult {
    // 事务由**写入插件**自己管（`transfer::writer`）：每路 writer 在自己的物理会话上
    // begin/commit。改造前这里在一条会话上 `set autocommit = 0` 再让 N 路 writer 往
    // **别的**会话写 —— 那层外层事务根本管不到，等于没开（真机踩过）。
    // 结构操作（建表/索引）也不该和数据写挤在一个事务里：DDL 会隐式提交。
    let sid = sync_slot_sid(slot);
    let mut result =
        sync_table_inner(state, task, src, tgt, table, target_table, opts, &sid, progress).await;
    // 后置 SQL（DataX 的 postSql）：本对象**数据搬完后**在目标连接上执行。
    //
    // 放在这个收口函数而不是 `sync_table_inner` 的每个 return 里：直拷 / 引擎 / 只同步
    // 结构三条路都从这里出去，不会漏；而 inner 里有十几个提前返回的分支，逐个加必然漏一个。
    // 失败 ⇒ 这张表判失败：用户写了后置 SQL 就说明它是流程的一部分（改约束、补索引、写打点），
    // 悄悄失败还报「同步成功」比直接报错糟得多。
    if result.status == "ok" && !opts.post_sql.is_empty() {
        let target_table = opts.target_table_for(table, target_table);
        match crate::api::scope::resolve(state, &tgt.conn, &tgt.scope).await {
            Ok(target) => {
                if let Err(msg) = run_user_sql(state, &target, &opts.post_sql, "后置 SQL").await {
                    // 数据其实已经搬完了，只是后置 SQL 失败 —— 行数照实带出去，
                    // 别让「表写完了但后置 SQL 出错」显示成一行都没过去
                    result = ObjResult::failed_with(
                        "table", table, msg, result.inserted, result.updated,
                    );
                } else if let Some(task) = task {
                    task.log(format!("表 {target_table}：执行后置 SQL"));
                }
            }
            Err(err) => {
                result = ObjResult::failed_with(
                    "table",
                    table,
                    err.message,
                    result.inserted,
                    result.updated,
                );
            }
        }
    }
    // 会话怎么处置：
    // - **成功 + 允许复用**（整库同步的并发槽位）⇒ 留着，给同一槽位的下一张表用。
    //   每张表 1~3 条连接，经 SSH 隧道建一条要 1.5~6.4 秒 —— 几百张表光建连就吃掉
    //   几十分钟，而槽位上的表是**串行**的，复用不会串事务。
    // - **失败/取消，或不复用**（单表同步）⇒ 立刻释放。失败那条连接可能带着未提交
    //   事务或半开状态，交给下一张表只会把脏状态传下去；顺带也避免连接数随表数
    //   单调增长、顶满 `session.maxPerHost` 后开始「挤掉正在用的那条」。
    if !(reuse && result.status == "ok") {
        release_table_sessions(state, src, tgt, &sid).await;
    }
    result
}

/// 并发**槽位**的会话后缀（`slot00`、`slot01`…，**定宽两位**）。
///
/// 整库同步同时跑 N 张表，调度侧给每张表发一个 `0..N-1` 的槽位号；同一槽位上的表
/// **串行**执行，所以用同一组会话键是安全的，而且能把物理连接**复用**起来（不必每张
/// 表重建）。表与表之间仍然互不干扰：写侧在表结束时 commit 并把 autocommit 复位。
///
/// 为什么定宽：释放会话时是按「泳道名包含这个串」匹配的（`Driver::disconnect_lanes`），
/// `slot3` 是 `slot30` 的前缀，不定宽会把 30 号槽位的连接一起关掉。
fn sync_slot_sid(slot: usize) -> String {
    format!("slot{slot:02}")
}

/// 目标端「写入能力」体检：把会直接限制吞吐的参数写进任务日志（**只读，不改配置**）。
///
/// 为什么要做：用户看到的现象是「同步越跑越慢」，而真凶常常是目标库
/// `innodb_buffer_pool_size` 只有 128MB、`innodb_io_capacity` 只有 200 ——
/// 这些参数同步工具改不了，但不点出来，用户只能反复怀疑工具。
/// 实测：5000 万行同步到 128MB buffer pool 的 MariaDB，吞吐从 23.8k 行/秒掉到 18.6k。
async fn warn_target_write_limits(state: &AppState, target: &str) {
    let sql = "show variables where variable_name in \
               ('innodb_buffer_pool_size','innodb_io_capacity','innodb_flush_log_at_trx_commit')";
    let Ok(result) = run_target_in_rows(state, target, sql, "internal:sync-preflight", 16).await
    else {
        return;
    };
    let mut buffer_pool_mb = 0u64;
    let mut io_capacity = 0u64;
    let mut trx_flush_one = false;
    for row in &result.rows {
        let Some(name) = row.first().map(|value| value.to_display()) else {
            continue;
        };
        let value_text = row.get(1).map(|value| value.to_display()).unwrap_or_default();
        match name.to_ascii_lowercase().as_str() {
            "innodb_buffer_pool_size" => {
                buffer_pool_mb = value_text.parse::<u64>().unwrap_or(0) / 1024 / 1024;
            }
            "innodb_io_capacity" => io_capacity = value_text.parse::<u64>().unwrap_or(0),
            "innodb_flush_log_at_trx_commit" => trx_flush_one = value_text.trim() == "1",
            _ => {}
        }
    }
    let mut hints: Vec<String> = Vec::new();
    if buffer_pool_mb > 0 && buffer_pool_mb < 1024 {
        hints.push(format!(
            "innodb_buffer_pool_size 只有 {buffer_pool_mb}MB（大表导入建议 2~4GB，否则越写越慢）"
        ));
    }
    if io_capacity > 0 && io_capacity < 1000 {
        hints.push(format!(
            "innodb_io_capacity 只有 {io_capacity}（SSD 建议 2000，刷脏页跟不上会成为瓶颈）"
        ));
    }
    if trx_flush_one {
        hints.push(
            "innodb_flush_log_at_trx_commit=1（每次提交都 fsync；导入期间可临时设 2）".to_string(),
        );
    }
    if !hints.is_empty() {
        // 这是**优化建议**，不是本次同步的结果 —— 只进文件日志，界面日志只留结果
        tracing::info!(
            hints = %hints.join("；"),
            "目标端写入参数偏保守（改成建议值通常能把吞吐翻倍；DBMind 不会去改目标库配置）"
        );
    }
}

/// 表结束时释放它的**槽位**会话（源、目标两侧各一次）。
///
/// 失败只记日志：收尾是**优化与止血**，不该把一张已经同步完的表判成失败。
async fn release_table_sessions(state: &AppState, src: &Side, tgt: &Side, sid: &str) {
    let mut conns: Vec<String> = vec![src.conn.clone()];
    if tgt.conn != src.conn {
        conns.push(tgt.conn.clone());
    }
    for conn in conns {
        let engine = state.engine();
        let needle = sid.to_string();
        let outcome = blocking(move || engine.disconnect_lanes(&conn, &needle)).await;
        match outcome {
            Ok(0) => {}
            Ok(closed) => tracing::debug!(
                target: "dbmind::sync",
                sid = %sid,
                closed,
                "表已结束，释放其专属会话"
            ),
            Err(err) => tracing::warn!(
                target: "dbmind::sync",
                sid = %sid,
                error = %err.message,
                "释放表会话失败（已忽略）"
            ),
        }
    }
}

/// 并发槽位的租约：`Drop` 时把号还回池子 —— 任务 panic / 提前 return 也不会漏号。
struct SlotLease {
    pool: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    id: usize,
}

impl Drop for SlotLease {
    fn drop(&mut self) {
        if let Ok(mut free) = self.pool.lock() {
            free.push(self.id);
        }
    }
}

/// 「在跑的表」计数租约：`Drop` 时归还 —— 弹性会话预算（见 `db()` 里 active_tables 那段）
/// 靠它统计当前并发张数，表 panic / 提前返回也不会漏减。
struct ActiveGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl ActiveGuard {
    fn enter(pool: &std::sync::Arc<std::sync::atomic::AtomicUsize>) -> (Self, usize) {
        let active = pool.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        (Self(pool.clone()), active)
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// 一条表最多自动重连重跑几次。
///
/// 为什么不只给 1 次：尾部大表在云数据库公网 / 丢包链路上「连断两次」是常态
/// （真机日志里 2 秒内连报两张表：代理回收会话表，重连后第一条语句又撞上另一条死连接）。
/// 一次都不多给，等于把「链路抖一下」直接变成「这张表失败」，而重跑的成本只是时间。
/// 用满之后仍断，才判定链路真的不可用。
const LINK_RETRY_MAX: usize = 5;

/// 每次重连之前等多久（秒）。阶梯式：抖动十几秒能自愈，真断网也来得及恢复
/// （累计约 67 秒；纯 1/2/4 秒的指数退避在「网线拔了两分钟」面前太短）。
const LINK_RETRY_BACKOFF_SECS: [u64; LINK_RETRY_MAX] = [2, 5, 10, 20, 30];

/// 断连后能不能**自动重连重跑**（前提见 `sync_table_inner` 里那段注释）。
///
/// 单独抽成纯函数是为了能被单测钉死：这里判错一次，要么把数据写重复、要么白跑一遍
/// 大表。
///
/// `overwrite_safe` = 本次重跑前**会把目标表清空**（见 [`can_clear_before_retry`]）：这种
/// 模式下「已经提交过几路」「退回过逐条提交」都不再是障碍 —— 断网时把一张跑了两小时的大表
/// 直接标红，比清空重来糟糕得多。合并 / 追加模式下重跑会写重复，仍然严格按下面四条硬条件来判。
fn can_retry_after_link_break(
    attempts: usize,
    txn_supported: bool,
    begin_failed: bool,
    committed: usize,
    canceled: bool,
    message: &str,
    overwrite_safe: bool,
) -> bool {
    // 次数上限：再断就是链路真的不行，重跑只会拖长任务
    attempts < LINK_RETRY_MAX
        // 用户点了停止就别再跑
        && !canceled
        // 脏数据 / 类型不匹配重跑必然再失败
        && is_link_level_message(message)
        && (overwrite_safe
            || (
                // 方言没有事务批（Doris / ClickHouse / Oracle / DB2…）⇒ 已写的行留在目标表里
                txn_supported
                // 退回过逐条提交 ⇒ 行已经 commit，重跑就是重复
                && !begin_failed
                // 已经有一路提交成功 ⇒ 目标表里已有本次任务的数据（多路写是多个独立事务），
                // 回滚只能回滚未提交的，重跑必然重复。这条是「不丢数据」的反面：**不能写重**。
                && committed == 0
            ))
}

/// 重跑前能不能把目标表清空。
///
/// 「清空重灌」与「删除重建」两种模式的语义都是「目标以源为准」，清空后重跑不会写重复：
/// - `truncate_insert`：本来就要先清空；
/// - 删除重建（`Policy::Drop`）：第一轮已经删表重建过，重跑前清一次等价于从头再来
///   （真机踩过：用户选「删除重建」跑到中途链路抖了 25 秒，第二轮没法清表 ⇒ 直接判死）。
///
/// 源与目标是同一张表时**不能清** —— 那会把待读的数据删掉。
fn can_clear_before_retry(data_mode: DataMode, policy: Policy, same_table: bool) -> bool {
    !same_table && (data_mode == DataMode::TruncateInsert || policy == Policy::Drop)
}

/// 直拷逃生阀：个别库的 `insert … select` 会被触发器/审计/权限拦住时，用它整体退回管道。
///
/// 语义与其它 `DBMIND_SYNC_*` 开关一致：**设了就不直拷**（`0/false/off` 视为没设）。
fn direct_copy_disabled() -> bool {
    std::env::var("DBMIND_SYNC_NO_DIRECT_COPY")
        .map(|value| {
            let value = value.trim();
            !(value == "0"
                || value.eq_ignore_ascii_case("false")
                || value.eq_ignore_ascii_case("off"))
        })
        .unwrap_or(false)
}

/// 读一个连接的「实例指纹」；查不到就是空串（调用方按「不同实例」处理）。
async fn instance_fingerprint(state: &AppState, conn: &str, scope: &str, sql: &str) -> String {
    let Ok(result) = run_sql_in(state, conn, scope, sql.to_string(), 1).await else {
        return String::new();
    };
    result
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(|cell| match cell {
            CellValue::Text(text) => Some(text.trim().to_string()),
            CellValue::Integer(n) => Some(n.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

/// 源表在**目标会话**上的限定引用；`None` = 这次不该直拷（不同实例 / 不同引擎）。
///
/// 这条 `insert … select` 在目标连接上执行，所以源表名必须是目标会话认得的全限定名：
/// - SQL Server 要**三段** `库.模式.表`（两段会被解析成「模式.表」，于是在目标库里找源表）；
/// - MySQL / ClickHouse / Doris 是 `库.表`；
/// - PG / Oracle / DB2 / 达梦 / Kingbase 是 `模式.表`（跨库引用在这些库里不存在）；
/// - 嵌入式（H2 / Derby / SQLite / DuckDB）只有一张库，裸表名。
///
/// 限定层级由源方言自己声明（[`Dialect::direct_ref_depth`]），这里不做库名分支。
async fn direct_source_ref(
    state: &AppState,
    src: &Side,
    tgt: &Side,
    source_kind: Dialect,
    target_kind: Dialect,
    table: &str,
) -> Option<String> {
    tracing::debug!(
        table,
        src = %src.conn,
        tgt = %tgt.conn,
        source = source_kind.kind.key(),
        target = target_kind.kind.key(),
        scope = %src.scope,
        schema = ?src.schema,
        "直拷判定"
    );
    // 引擎要一致：MySQL ↔ MariaDB 的语句完全兼容，可以互相引用；其余不跨引擎。
    let same_family = source_kind.kind == target_kind.kind
        || matches!(
            (source_kind.kind.key(), target_kind.kind.key()),
            ("mysql", "mariadb") | ("mariadb", "mysql")
        );
    if !same_family {
        return None;
    }
    // 同一条连接 = 同一个物理会话，肯定是同一个实例；不同连接才需要指纹。
    if src.conn != tgt.conn {
        let Some(fingerprint_sql) = source_kind.instance_fingerprint_sql() else {
            return None;
        };
        let left = instance_fingerprint(state, &src.conn, &src.scope, fingerprint_sql).await;
        let right = instance_fingerprint(state, &tgt.conn, &tgt.scope, fingerprint_sql).await;
        if left.is_empty() || left != right {
            tracing::debug!(table, %left, %right, "两个连接不是同一实例，走管道传输");
            return None;
        }
    }
    // 用户已经把全限定名写进表名（`库.模式.表`）时不再拼前缀
    if table.contains('.') {
        return Some(source_kind.quote(table));
    }
    let scope = src.scope.trim();
    let explicit_schema = src.schema.as_deref().map(str::trim).unwrap_or("");
    let (first, second) = match scope.split_once('.') {
        Some((left, right)) => (left.trim(), right.trim()),
        None => (scope, ""),
    };
    let mut parts: Vec<&str> = Vec::new();
    match source_kind.direct_ref_depth() {
        DirectRefDepth::DatabaseSchema => {
            if first.is_empty() {
                // 没选库 ⇒ 目标会话的当前库就是源库，只剩模式这一层
                let schema = if explicit_schema.is_empty() { second } else { explicit_schema };
                if !schema.is_empty() {
                    parts.push(schema);
                }
            } else {
                parts.push(first);
                // 两段名在 T-SQL 里是「模式.表」：跨库必须给模式，缺省按 dbo
                let schema = if explicit_schema.is_empty() {
                    if second.is_empty() { "dbo" } else { second }
                } else {
                    explicit_schema
                };
                parts.push(schema);
            }
        }
        DirectRefDepth::Database => {
            // Doris 的 scope 可能带 catalog（`internal.xms`），交给 quote 逐段加引号
            if !scope.is_empty() {
                parts.push(scope);
            }
        }
        DirectRefDepth::Schema => {
            // 只有**真的知道模式**时才限定。`scope` 在 PG / Oracle 上就是库名
            //（schema 是另选的），拿它当模式会拼出 `"demo"."orders"` 这种不存在的
            // 引用 —— 真机就因此白退了一次管道。不知道模式时写裸名：目标会话与源
            // 同库同 search_path，解析结果和读元数据时完全一致。
            let schema = if explicit_schema.is_empty() { second } else { explicit_schema };
            if !schema.is_empty() {
                parts.push(schema);
            }
        }
        DirectRefDepth::None => {}
    }
    parts.push(table);
    let reference = source_kind.quote(&parts.join("."));
    tracing::debug!(table, %reference, "同实例，走 insert … select 直拷");
    Some(reference)
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
    // 三条语句由写插件的 `SPEC.txn(...)` 声明；`None` = 该方言没有可用的 SQL 级 begin
    //（Doris / ClickHouse / Oracle / 达梦 / DB2 / H2 / Derby），照旧逐条提交。
    crate::api::transfer::plugins::spec_data_for(kind)
        .txn()
        .map(|txn| (txn.begin, txn.commit, txn.end))
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
    sid: &str,
    progress: &std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> ObjResult {
    let ty = "table";
    // 目标表名映射（界面「表名映射」）：在**最前面**一次换掉，后面「读目标表结构 / 建表 /
    // 建索引 / 写数据 / 日志」全部用新名。放到最后再换的话，结构会建在旧名下、数据写进
    // 新表，用户看到的是一张空表 + 一句「同步成功」。
    let mapped_table = opts.target_table_for(table, target_table);
    let target_table = mapped_table.as_str();
    // 行过滤**按表取**：界面上一张表一个条件（见 `SyncRequest::where_clauses`）。
    // 这里一次算好，同实例直拷与 transfer 引擎共用同一个值 —— 两条路各自去读
    // `opts` 很容易分叉（曾经直拷路径漏掉过滤，抽样的表整表搬过去了）。
    let table_where = opts.where_for(table);
    // 界面日志的粒度：**每一步的开始 + 最终结果**，中间的细枝末节（每条索引、每次保存点、
    // 每 10 次分段提交）都不打。
    //
    // 为什么「开始」必须打：整库同步 400 个对象时，一张表从读到写完要几十秒，
    // 只打「完成」的话界面上会有长达半分钟的空白（真机截图反馈：读计数在涨、
    // 同步日志 0 条，用户以为卡死了）。开始行还带出「卡在哪一步」。
    // 链路日志按「删除目标表 → 创建目标表 → 读取源数据 → 传输数据到目标表 → 传输完成」
    // 走，每一步只打一条（详见 transfer/engine.rs 里后三步）。这里不先打「开始」——
    // 它和下一步之间没有信息量，只会让每张表多一行。
    tracing::debug!(table, "读取结构");
    let source_columns = match columns_of_retry(state, src, table).await {
        Ok(columns) => columns,
        Err(err) => return ObjResult::failed(ty, table, err.message),
    };
    if source_columns.is_empty() {
        // 查不到列：**让数据库自己说原因**（表不存在 / 没权限 / 视图失效都会带真实报错），
        // 不要由我们编一句「源表不存在或没有列信息」—— 那句在现场只会误导。
        let probe = format!("select 1 from {} where 1 = 0", src.scoped_meta(table));
        return match run_sql_in(state, &src.conn, &src.scope, probe, 1).await {
            Ok(_) => ObjResult::failed(ty, table, format!("源表 {table} 取不到列信息")),
            Err(err) => ObjResult::failed(ty, table, err.message),
        };
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
    // 目标表名带**用户选定的模式**（PG 的 sales / SQL Server 的非 dbo / Doris 的 catalog.库）。
    //
    // 以前这里是裸 `quote(target_table)`：写语句在目标连接上执行，裸名会落到连接的默认
    // search_path/schema 上 —— 结构建到了 `sales`，数据却写进 `public` 里那张同名表，
    // 或者直接报「表不存在」。模式越不默认，错得越彻底（而且目标端可能真有同名表，
    // 于是「同步成功」但数据进了另一张表，比报错更难发现）。
    let target_name = tgt.scoped(target_table, target_kind);
    // 目标连接只解析一次，循环里的每条语句复用（见 run_target 的注释）
    let tgt_target = match crate::api::scope::resolve(state, &tgt.conn, &tgt.scope).await {
        Ok(target) => target,
        Err(err) => return ObjResult::failed(ty, table, err.message),
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
        tracing::debug!(table, "创建表");
        // 带上源表的表注释与字段注释（见上面取注释那一段）
        let ddl = target_kind.create_table_with_comments(
            &target_name,
            &source_columns,
            cross_kind,
            &src_column_comments,
            src_table_comment.as_deref(),
        );
        // 建表语句进文件日志：目标方言拒绝时（1071 键太长 / 1067 默认值非法 / 1118 行太宽），
        // 界面上只有一句数据库原文，靠着这一行才能还原「我们到底发了什么」。
        tracing::debug!(table, %ddl, "目标表建表语句");
        if let Err(err) = run_target(state, &tgt_target, &ddl).await {
            // 建表失败**只报数据库/驱动的原话**（用户口径：不拼任何东西）。
            // 以前这里按错误内容追加三段补救建议（Row size too large / key was too long /
            // invalid default value），那些话长得像数据库报错，用户分不清哪句是 MySQL 说的；
            // 成因与对策留在代码注释与 tracing 里，界面上只有方言原话。
            tracing::debug!(table, error = %err.message, "建表失败（只报方言原话，不追加建议）");
            return ObjResult::failed(ty, table, err.message);
        }
        created = true;
        if let Some(task) = task {
            task.log(format!("表 {table}：创建目标表"));
        }
    } else if opts.policy == Policy::Drop
        || (opts.sync_structure && opts.data_mode == DataMode::TruncateInsert)
    {
        // 「覆盖式同步」：`truncate_insert` 的意图是「目标以源为准」。目标表若已存在，
        // 它是**历史结构** —— 真机整库同步里 33 张表与源不一致（216 处字符集不同：
        // 源 utf8mb4 / 目标 utf8；29 处长度不同：varchar(3000) → varchar(600)），
        // truncate 只清数据不动列定义，写进去必然撞 `Data too long` /
        // `Incorrect string value` / `Duplicate entry`（唯一索引也不同），
        // 一天下来就攒出几千行「写入失败」。所以这里按源重建。
        // 重建失败（例如目标表被外键引用）不阻断：退回「保留结构 + 清空灌数」，
        // 数据至少能进去，日志里写明原因。
        let forced = opts.policy != Policy::Drop;
        if !opts.sync_structure {
            return ObjResult::failed(
                ty,
                table,
                "选择了「删除重建」，但没有勾选自动建表：那样会删掉目标表且建不回来",
            );
        }
        // 链路日志第 1 步
        if let Some(task) = task {
            task.log(format!("表 {table}：删除目标表"));
        }
        tracing::debug!(table, forced, "删除目标表（按源结构重建）");
        let mut rebuilt = true;
        // PG 的 `drop table` 遇到「有视图依赖它」会直接失败（真机：MySQL → PG 整库同步，
        // 目标库已有视图引用该表 ⇒ `cannot drop table … because other objects depend on it`），
        // 于是重建被降级成"保留旧结构"，列定义根本没更新。PG 支持 `cascade`，依赖的视图
        // 会在后面的视图阶段重建成新版，正是「以源为准」想要的结果。
        let drop_sql = if target_kind.kind.key() == "postgresql" {
            format!("drop table if exists {target_name} cascade")
        } else {
            format!("drop table {target_name}")
        };
        if let Err(err) = run_target(state, &tgt_target, &drop_sql).await {
            if !forced {
                return ObjResult::failed(ty, table, err.message);
            }
            rebuilt = false;
            // 这是「降级继续」的过程细节：真要出问题，后面坏的行/表会以结果形式报出来
            tracing::warn!(
                table,
                error = %err.message,
                "删除重建失败，改为保留目标结构继续（列定义不一致时后续可能报 Data too long）"
            );
        }
        if rebuilt {
            tracing::debug!(table, "按源结构重建目标表");
            // 带上源表的表注释与字段注释（见上面取注释那一段）
            let ddl = target_kind.create_table_with_comments(
                &target_name,
                &source_columns,
                cross_kind,
                &src_column_comments,
                src_table_comment.as_deref(),
            );
            tracing::debug!(table, %ddl, "目标表建表语句（重建）");
            if let Err(err) = run_target(state, &tgt_target, &ddl).await {
                return ObjResult::failed(ty, table, err.message);
            }
            created = true;
            // 链路日志第 2 步
            if let Some(task) = task {
                task.log(format!("表 {table}：创建目标表"));
            }
        }
    }
        // 建表后才能补的注释（SQL Server 的扩展属性等）：create_table_with_comments
        // 写不进去的方言在这里逐条执行 —— 之前 SQL Server 目标的注释全空（真机反馈）。
        // 失败静默：注释是有价值的补充，不该让它挡住数据同步。
        // schema 只有 SQL Server 那条路要用（扩展属性必须点名 schema）；PG / Oracle / DM / DB2
        // 的 `comment on table <名>` 不带 schema 也会落到当前 search_path 的表上 ——
        // 之前这里硬写 "dbo"，那些库拼出来的语句必然报错，而失败是静默的（注释全丢）。
        // 表名一律传**未加引号的裸名 + 模式**：`post_create_comments` 内部按方言自己加引号
        // （传 `"sales"."tbl"` 进去会被当成一个完整的标识符再用尖引号包一遍）。
        let comment_schema = tgt
            .schema
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(if target_kind.kind.key() == "sqlserver" { "dbo" } else { "" });
        let post_comments = target_kind.post_create_comments(
            target_table,
            &src_column_comments,
            src_table_comment.as_deref(),
            comment_schema,
        );
        for sql in &post_comments {
            let _ = run_target(state, &tgt_target, sql).await;
        }
        if !post_comments.is_empty() {
            tracing::debug!(table, count = post_comments.len(), "已补充注释");
        }
    // 建表（含删除重建）成功后：把源表的非主键索引搬过来（「建表包含索引」选项）
    //
    // **索引后置**：要灌数据时只先建唯一索引 —— `replace` / `on duplicate key update`
    // 的去重语义靠目标端的唯一索引，推迟会把冲突行写成重复行；非唯一的二级索引推到
    // 数据灌完之后再建（`deferred_indexes`），批量写入期间不必逐行维护二级 B 树，
    // 建索引时也只全表扫一遍。小表收益小但没有额外代价（建索引本身就快）。
    // 不灌数据（仅结构同步）时一次建完，行为与以前完全一致。
    let mut deferred_indexes = false;
    // 源表「被当成主键的那几列」：源没有真正主键时，建表 DDL 会把某个唯一索引升格成
    // 主键写进去（`create_table_with_comments` 只看列的 primary_key 标记）。建索引时
    // 必须跳过同一组列的唯一索引，否则目标表会同时有 `PRIMARY` 和同名唯一索引两套约束。
    let declared_primary: Vec<String> = source_columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.clone())
        .collect();
    if created && opts.include_indexes {
        let filter = if opts.sync_data {
            IndexFilter::UniqueOnly
        } else {
            IndexFilter::All
        };
        sync_indexes(
            state,
            src,
            source_kind,
            &tgt_target,
            target_kind,
            &target_name,
            table,
            task,
            filter,
            &declared_primary,
            &source_columns,
        )
        .await;
        deferred_indexes = opts.sync_data;
    }
    // 目标表**已存在**（走不到建表，注释也就没机会带上）：把源侧的**表注释 + 列注释**
    // 差异补过去 —— 源有注释而目标没有/不一致的，逐条 ALTER 补齐。
    if !created && opts.sync_structure {
        update_comments(
            state, src, source_kind, tgt, target_kind, &tgt_target, &target_name, table,
            &source_columns,
        )
        .await;
    }

    if !opts.sync_data {
        let message = if created { "已按源结构建表（未同步数据）" } else { "已跳过（未勾选同步数据）" };
        return ObjResult::ok(ty, table, message, 0, 0);
    }
    // 前置 SQL（DataX 的 preSql）：**结构已就绪、数据还没搬** —— 清空目标表、禁用触发器、
    // 写批次打点这类操作只有放在这一档才有意义（放建表前会撞「表不存在」，放搬完后太晚）。
    // 直拷与引擎两条路都在这个点之后，所以两条路都能覆盖。
    if !opts.pre_sql.is_empty() {
        if let Err(msg) = run_user_sql(state, &tgt_target, &opts.pre_sql, "前置 SQL").await {
            return ObjResult::failed(ty, table, msg);
        }
        if let Some(task) = task {
            task.log(format!("表 {target_table}：执行前置 SQL"));
        }
    }
    // 「读取源数据」由 transfer 引擎在真正发第一页查询时打（单表同步、直拷两条路
    // 都走它），这里不重复。
    tracing::debug!(table, "开始搬运数据");

    let target_columns = if created {
        source_columns.clone()
    } else {
        target_columns
    };
    // 只同步两边都有的列：目标多出来的列让它走自己的默认值/自增
    //
    // **二进制列也同步**：内核已回真字节（`CellValue::Bytes`，base64，单值上限 8 MB），
    // 写入侧按方言拼十六进制字面量（见 `export::binary_literal`）。旧实现把 BLOB/BINARY
    // 排除在数据同步之外 —— 一张 20 列的表里漏掉 1 个 BLOB，界面照样显示「同步成功」。
    let mut columns: Vec<String> = Vec::new();
    for column in &source_columns {
        if !target_columns
            .iter()
            .any(|other| other.name.eq_ignore_ascii_case(&column.name))
        {
            continue;
        }
        columns.push(column.name.clone());
    }
    if columns.is_empty() {
        // 一行都不会搬：索引后置的那一半照样补齐，否则这张新表比「正常建表」少了二级索引
        create_deferred_indexes(
            state, src, source_kind, &tgt_target, target_kind, &target_name, table, task,
            deferred_indexes, &declared_primary, &source_columns,
        )
        .await;
        return ObjResult::skipped(ty, table, "源表与目标表没有同名列，无法同步数据");
    }
    let missing: Vec<String> = source_columns
        .iter()
        .filter(|column| !columns.iter().any(|name| name.eq_ignore_ascii_case(&column.name)))
        .map(|column| column.name.clone())
        .collect();

    // upsert 只能按**目标端真实存在且完整的唯一键**匹配。源主键只说明两侧
    // 逻辑上可能对应，不能证明目标表有唯一约束；用它 UPDATE 可能命中多行。
    let target_primary: Vec<String> = target_columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.clone())
        .collect();
    let target_common: Vec<String> = target_primary
        .iter()
        .filter(|name| columns.iter().any(|column| column.eq_ignore_ascii_case(name)))
        .cloned()
        .collect();
    // 原生 upsert 依赖目标端**完整**的主键约束：复合主键只同步到其中一部分列时，
    // 目标端并没有这组部分键的唯一定义，不能拿它做 ON CONFLICT / MERGE。
    let target_key_ready = !target_primary.is_empty() && target_common.len() == target_primary.len();
    let mut key_columns: Vec<String> = if target_key_ready {
        target_common
    } else {
        Vec::new()
    };
    // 两阶段 upsert 不依赖目标声明的唯一约束（ClickHouse/Doris 元数据通常不把
    // 主键字段标成 primary_key）。此时可用完整的源主键按值查目标；lookup 会
    // GROUP BY 键并拒绝重复目标行，因此不会把多行误当成一行更新。
    if !target_key_ready {
        let source_primary: Vec<String> = source_columns
            .iter()
            .filter(|column| column.primary_key)
            .map(|column| column.name.clone())
            .collect();
        let source_common: Vec<String> = source_primary
            .iter()
            .filter(|name| columns.iter().any(|column| column.eq_ignore_ascii_case(name)))
            .cloned()
            .collect();
        if !source_primary.is_empty() && source_common.len() == source_primary.len() {
            key_columns = source_common;
        }
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

    // 与 `columns` 一一对应的**目标侧真实列名**：源 `order_no` 在目标上可能叫 `ORDER_NO`
    // （Oracle 折大写）或 `OrderNo`（历史表）。写语句必须用目标那个名字 —— 以前直接用源名
    // 加引号，目标表已存在时 INSERT/UPDATE 会报「列不存在」，而结构明明是好的。
    let target_column_names: Vec<String> = columns
        .iter()
        .map(|name| {
            if created {
                // 刚由本工具建的表：建表 DDL 用的就是源列名（已按目标方言 quote 过）
                name.clone()
            } else {
                target_columns
                    .iter()
                    .find(|other| other.name.eq_ignore_ascii_case(name))
                    .map(|other| other.name.clone())
                    .unwrap_or_else(|| name.clone())
            }
        })
        .collect();

    if let Some(task) = task {
        // 取消必须在**每张表开始搬数据之前**生效。以前这里是 `check_canceled().ok()` ——
        // 结果被丢掉，点了「停止」之后所有已经拿到并发许可的表仍然会把整表搬完
        //（真机：点了停止，进度还在涨，重启界面看它还在「同步中」）。
        if let Err(reason) = task.check_canceled() {
            return ObjResult::skipped(ty, table, reason);
        }
        task.set_phase(format!("读取源表 {table} 数据"));
    }

    let mut note = String::new();
    if !missing.is_empty() {
        note.push_str(&format!("（目标缺少 {} 列，未同步）", missing.join("/")));
    }


    // 本表的会话后缀由 `sync_table` 生成并传进来（它还要按这个串做收尾释放）。

    // 源与目标**是同一张表**（自同步）时，清空会把待读的数据删掉；那种"清空"本身也没有意义。
    // 旧实现是「先把整表读进内存、再清目标」，顺序上不怕；现在改成边读边写，必须显式区分。
    let same_table = src.conn == tgt.conn
        && src.scope.eq_ignore_ascii_case(&tgt.scope)
        && table.eq_ignore_ascii_case(target_table);
    if opts.data_mode == DataMode::TruncateInsert {
        if same_table {
            tracing::info!(table = %table, "源与目标是同一张表：跳过清空");
        } else {
            // 清空语句按**目标方言**选：ClickHouse 不吃裸 `delete from`（实测 position 26 语法错），
            // SQLite / Derby 又没有 `truncate` —— 见 `Dialect::clear_table_sql`
            let clear = target_kind.clear_table_sql(&target_name);
            if let Err(err) = run_target_write(state, &tgt_target, &clear).await {
                return ObjResult::failed(ty, table, err.message);
            }
        }
    }

    let mut inserted = 0u64;
    let mut updated = 0u64;
    // 计时只为在结果里给出吞吐（行/秒）：优化前后快了多少，界面上能直接看出来
    let started = std::time::Instant::now();
    let mut effective_mode = opts.data_mode;
    if opts.data_mode == DataMode::Upsert && !target_key_ready {
        // 目标端没有可用且完整的唯一键时，先确认是否为空。空目标是首次全量导入，
        // 可以直接走 INSERT：ClickHouse/Doris 能少掉整页键查询，Doris 还能走
        // Stream Load；非空时才尝试按源主键两阶段匹配（lookup 会拒绝重复键）。
        let target_probe = if target_kind.needs_order_by_for_paging() {
            format!(
                "select 1 as one from {target_name} order by (select null) {}",
                target_kind.limit_clause(0, 1)
            )
        } else {
            format!(
                "select 1 as one from {target_name} {}",
                target_kind.limit_clause(0, 1)
            )
        };
        let empty = match run_sql_in(state, &tgt.conn, &tgt.scope, target_probe, 1).await {
            Ok(result) => result.rows.is_empty(),
            Err(_) => false,
        };
        if empty {
            effective_mode = DataMode::Insert;
        } else if key_indexes.is_empty() {
            return ObjResult::failed(
                ty,
                table,
                format!("目标表 {target_table} 没有可用且完整的 upsert 主键"),
            );
        }
    }

    // upsert 怎么落库：
    // - **Native**：MySQL `on duplicate key update`、PG/SQLite/H2 `on conflict … do update`、
    //   Oracle/DM/DB2/SQL Server `merge into`。目标端自己判「插入还是更新」，本进程既不读
    //   目标主键、也不逐行发 UPDATE —— 大表 upsert 的最大一笔开销就此消失；
    // - **TwoPhase**：没有原生语法的方言（ClickHouse / Doris / Derby）只查询当前源页
    //   出现的键（500 行一批），再分「新增 INSERT / 命中 UPDATE」两拨；不再把目标
    //   整表主键装进内存。
    // MySQL/MariaDB 的原生 upsert 还会被其它唯一索引截获；只有目标主键确实
    // 可用、且没有额外唯一索引时才走它。
    let mysql_secondary_unique = if effective_mode == DataMode::Upsert
        && target_key_ready
        && matches!(target_kind.kind.key(), "mysql" | "mariadb")
    {
        mysql_has_secondary_unique(state, &tgt.conn, &tgt.scope, &target_name).await
    } else {
        false
    };
    let native_upsert_safe = target_key_ready && !mysql_secondary_unique;
    let upsert_mode = if effective_mode == DataMode::Upsert {
        if target_kind.supports_native_upsert() && native_upsert_safe {
            crate::api::transfer::UpsertMode::Native
        } else {
            crate::api::transfer::UpsertMode::TwoPhase
        }
    } else if effective_mode == DataMode::TruncateInsert
        && target_key_ready
        && target_kind.supports_native_upsert()
    {
        // **幂等插入**：清空后导入时表本来就在等这批数据，正常路径下
        // `on duplicate key update` 与纯 insert 等价（开销也一样，都要维护唯一索引）；
        // 但链路上断一次再重跑时，"重跑区间"与"已提交区间"可能叠一小段，
        // 同一行被读两遍 —— 纯 insert 会报 `Duplicate entry` 把它算成脏数据丢掉，
        // 而 upsert 会把它改写成同样的值，行数是对的。
        // 真机代价：`winneryun_huike_data_hour`（源无主键、只有唯一索引）连续断了三次，
        // 341 行被判失败、整表最后 0 行，就是缺这一层幂等。
        crate::api::transfer::UpsertMode::Native
    } else {
        crate::api::transfer::UpsertMode::Off
    };
    // 「回退两阶段 upsert」是实现细节（用户口径：结果里只写数据事实，不写实现说明），
    // 进 tracing 排查用即可。
    if effective_mode == DataMode::Upsert && (mysql_secondary_unique || !target_key_ready) {
        tracing::debug!(
            target: "dbmind::sync",
            table = %table,
            secondary_unique = mysql_secondary_unique,
            key_ready = target_key_ready,
            "upsert 走两阶段匹配"
        );
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
        // `object_id()` 要的是**未加引号**的 `schema.table`：把带引号的 `"dbo"."tbl"` 塞进
        // 字符串里查不到对象 ⇒ identity_on 恒 false ⇒ 显式键值插 identity 列必然报错。
        let probe = format!(
            "select count(*) as n from sys.identity_columns where object_id = object_id('{}')",
            tgt.scoped_meta(target_table).replace('\'', "''")
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

    // **Doris Stream Load 快速通道**：纯插入/清空重建时启用 —— 数据以 JSON 走 HTTP
    // 直推 FE，绕过 MySQL 协议 1000~2000 行/批的吞吐瓶颈；失败自动回退 INSERT 管道。
    // 能力位由目标写插件声明（`SPEC.stream_load()`，目前只有 Doris）：同步层不认数据库名字。
    let use_stream = crate::api::transfer::plugins::spec_data_for(target_kind.kind.key())
        .stream_load()
        && effective_mode != DataMode::Upsert
        && tgt_record.is_some()
        // 含二进制列的表**不走 Stream Load**：CSV/JSON 体里没法原样表达 BLOB
        //（现在编码出来只是一个 `b:len` 占位），走了就是静默写坏数据；
        // 退回 INSERT 管道后由 `export::binary_literal` 拼 hex 字面量，字节才真的过得去。
        && !columns
            .iter()
            .any(|name| {
                source_columns
                    .iter()
                    .find(|column| column.name.eq_ignore_ascii_case(name))
                    .map(|column| {
                        Dialect::is_binary_type(column.type_name.as_deref().unwrap_or(""))
                    })
                    .unwrap_or(false)
            })
        // 本任务里已经判定 Stream Load 不可用（见 Task::stream_unavailable）：后面的表
        // 连 `show backends` 都不必再查，直接走 INSERT 管道
        && !task.map(|task| task.stream_unavailable()).unwrap_or(false);
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
        tracing::debug!(table, be_http_port, "Stream Load 直连 BE http 端口");
    }
    // ===== 流式写入：读一页、写一页 =====
    // 旧实现是「整表读进内存 → 再写」，代价是三件事捆在一起：
    //   · 必须有 20 万行的上限，否则内存扛不住；
    //   · 第一行要等整表读完才开始写；
    //   · 取消请求也要等整表读完才可能被检查到。
    // 改成页循环之后这三件事一起消失。跨页不再拼接批次（那只会让"已写行数"与真正
    // 落盘的行数差出一页，没有别的好处）。
    //
    // ===== 同实例快速通道（任意关系型源/目标）=====
    // 源与目标落在**同一个实例**且是纯插入/清空后导入时，直接
    //   insert into 目标(列) select 列 from 源
    // —— 数据由服务端自己搬，一个字节都不经过应用层。
    //
    // 这不是「优化一点点」：真机口径（实验室 SQL Server 20 万行 6 列，客户端到库
    // 23 ms、单条连接上行 340 KB/s）走管道 **239 秒**，直拷 **2 秒**；同一个数据源
    // 走 MySQL 时 5 秒。上行被逐连接限速时，调批大小/并发都只能逼近那条链路的物理
    // 上限，只有「不搬字节」才能跨过去。
    //
    // 判据两条：**同一个实例** + **语句在目标会话上引用得到源表**（见 `direct_source_ref`）。
    // `row_limit`（每表行数上限）只由管道按页实现；直拷是一条 `insert … select`，
    // 想截断就得给每家的方言各拼一次 LIMIT，拼错就是「用户设了上限却搬了全表」。
    // 这个场景很少见，直接不走直拷，退回管道（那里一定尊重上限）。
    if opts.sync_data
        && effective_mode != DataMode::Upsert
        && !same_table
        && opts.row_limit == 0
        && !direct_copy_disabled()
    {
        // 带标签的块：源表在目标会话上引用不到时 `break 'direct` 静静退回管道，
        // 不能让一条「试探性」的语句把整张表判失败。
        'direct: {
            let Some(src_qualified) =
                direct_source_ref(state, src, tgt, source_kind, target_kind, table).await
            else {
                break 'direct;
            };
            tracing::debug!(table, %src_qualified, "同实例直拷：源表引用");
            tracing::debug!(table, "同实例，走 insert … select 直拷");
            let col_list_src = columns
                .iter()
                .map(|name| source_kind.quote(name))
                .collect::<Vec<_>>()
                .join(", ");
            let col_list_tgt = target_column_names
                .iter()
                .map(|name| target_kind.quote(name))
                .collect::<Vec<_>>()
                .join(", ");
            // 直拷也带上用户的行过滤条件（同实例 insert…select 同样只搬过滤后的行）
            // 源表引用由 `direct_source_ref` 按方言限定（SQL Server 必须三段名，
            // 两段名会被当成「模式.表」⇒ 在目标库里找源表，读到空表）。
            let where_sql = if table_where.is_empty() {
                src_qualified
            } else {
                format!("{} where {}", src_qualified, table_where)
            };
            let insert_sql = format!(
                "insert into {target_name} ({col_list_tgt}) select {col_list_src} from {where_sql}"
            );
            tracing::info!(sql = %insert_sql, "同实例直拷 SQL");
            // 直拷同样要过 identity：同批 SET 开/关（会话级开关必须与写入同一批）。
            // 用前面探出来的 `identity_on` 而不是「目标是 SQL Server」—— 目标表**没有**
            // identity 列时 SET 本身就会失败，并把整批拖死（与管道路径同一口径）。
            let insert_sql = with_identity(insert_sql, identity_on, &target_name);
            // 直拷是一条 `insert … select` 大语句，发出去就**没法从中间打断**（只能等它
            // 自己跑完或超时），所以发之前先看一眼取消标志：点了停止就别再开始了。
            if let Some(task) = task {
                if let Err(reason) = task.check_canceled() {
                    return ObjResult::skipped(ty, table, reason);
                }
            }
            // 防御：先结束目标会话上可能残留的旧事务 —— 池化会话是粘性的，此前某次
            // 同步若没走到 commit，autocommit=0 的旧事务会一直开着，REPEATABLE READ
            // 的快照停在很久以前（直拷/回查全部看到旧世界，真机抓到过 src_visible=0）
            let _ = run_target(state, &tgt_target, "commit").await;
            // 直拷前先确认**目标会话看得见源表**（跨库权限不足、限定名拼错、源表刚被删）。
            // 用 `limit 1` 而不是 `count(*)`：千万行的表上 count 是一次全扫，而直拷本身
            // 才刚开始 —— 探测的代价必须远小于被探测的事。
            let probe_sql = if target_kind.needs_order_by_for_paging() {
                format!(
                    "select 1 as one from {where_sql} order by (select null) {}",
                    target_kind.limit_clause(0, 1)
                )
            } else {
                format!(
                    "select 1 as one from {where_sql} {}",
                    target_kind.limit_clause(0, 1)
                )
            };
            if let Err(err) = run_target(state, &tgt_target, &probe_sql).await {
                // 这条 `insert … select` 必然失败。宁可按管道慢一遍，也不能把这张表
                // 判成失败 —— 直拷是加速通道，不该成为新的失败来源。
                tracing::warn!(table, error = %err.message, "直拷前源表探测失败，改用管道传输");
                break 'direct;
            }
            let copy_result = run_target(state, &tgt_target, &insert_sql).await;
            let mut copied = match &copy_result {
                // INSERT … SELECT 的影响行数就是传输行数（比再发一条 count 快且准）
                Ok(result) => {
                    tracing::info!(affected = ?result.affected_rows, rows = result.rows.len(), "直拷结果");
                    result.affected_rows.unwrap_or(0) as u64
                }
                // 直拷失败也只报方言原话（`copy_result` 是借用，这里要 clone）
                Err(err) => return ObjResult::failed(ty, table, err.message.clone()),
            };
            if copied == 0 {
                // 个别驱动把 INSERT…SELECT 的 affectedRows 回成 0（MySQL 真机偶发），
                // 这时补一条源侧 count 兜底 —— 数的是源表（同一 where），不是目标表，
                // 所以不会把历史行算进来。
                if let Ok(result) =
                    run_target(state, &tgt_target, &format!("select count(*) as n from {where_sql}"))
                        .await
                {
                    copied = result
                        .rows
                        .first()
                        .and_then(|row| row.first())
                        .and_then(|cell| match cell {
                            CellValue::Integer(n) => Some((*n).max(0) as u64),
                            CellValue::Real(n) => Some((*n).max(0.0) as u64),
                            CellValue::Text(t) => t.trim().parse::<u64>().ok(),
                            _ => None,
                        })
                        .unwrap_or(0);
                }
            }
            if let Some(task) = task {
                // 直拷也是「读了 N 行、写了 N 行」：只记写会让进度卡出现
                // 「读取 0 / 传输 48」这种自相矛盾的画面（真机截图）
                task.add_rows_read(copied);
                task.add_rows_written(copied);
            }
            // 索引后置的收尾：行已经落到目标，这时补建非唯一二级索引最省
            create_deferred_indexes(
                state, src, source_kind, &tgt_target, target_kind, &target_name, table, task,
                deferred_indexes, &declared_primary, &source_columns,
            )
            .await;
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
    // 读侧分页与 upsert 键解耦：即使目标没有可用主键、只能 Insert，仍可以用
    // 完整的源主键做 keyset 分页，避免无键表的 OFFSET 深翻页退化。
    let source_primary: Vec<String> = source_columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.clone())
        .collect();
    let source_common: Vec<String> = source_primary
        .iter()
        .filter(|name| columns.iter().any(|column| column.eq_ignore_ascii_case(name)))
        .cloned()
        .collect();
    let paging_keys = if !source_primary.is_empty() && source_common.len() == source_primary.len() {
        source_common
    } else {
        // 无主键：退而求其次，用源表的唯一索引当 keyset 游标（列必须全部在同步列里，
        // 且都不能为 NULL —— 唯一索引允许多行 NULL，拿它翻页会漏行）
        let nullable: Vec<String> = source_columns
            .iter()
            .filter(|column| column.nullable)
            .map(|column| column.name.clone())
            .collect();
        unique_paging_key(state, src, source_kind, table, &columns, &nullable)
            .await
            .unwrap_or_default()
    };
    if paging_keys.is_empty() {
        // 没有可用的键 ⇒ 分页只能退回 OFFSET（第 N 页要让源库白扫前 N 页）。
        // 这句**不再进界面日志**：紧接着 transfer 引擎会按实际分片方式打一句
        // 「…按行号范围切成 N 段（每段 OFFSET 定位…）」，两条并列时用户看到的是
        // 自相矛盾（先说不按主键、又说按主键切块）。细节留在文件日志里，够排障用。
        tracing::debug!(table, "无主键且没有可用的唯一索引（列需全部纳入同步且非空），退回 OFFSET 分页");
    }

    // 写池诊断（在途批数 / 最近批耗时 / 失败样例 / 是否退回过逐条提交）：
    // 卡顿日志、二分定位与「断连能不能整表重跑」共用这一份，所以放在重试循环**外面**。
    let diag = std::sync::Arc::new(crate::api::transfer::WriteDiag::default());
    // 整表重跑时 `transfer::run` 的计时器会重新归零：这两份累计量跨轮共享，
    // 保证界面上的「耗时」是用户真正等的时间，而不是只剩最后一轮。
    // - `elapsed_base_ms`：前几轮已经耗掉的墙钟（不管成功失败）
    // - `wait_ms`：其中判死 / 回滚 / 换连接 / 提交超时这些没在搬数据的部分
    let elapsed_base_ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let wait_ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    // 一次尝试 = 一份新计划：reader / writer 都按计划重建，会话键（`sid`）不变 ——
    // 连接断过的话内核已经忘了那条会话，下次用时会自动重连。
    let make_plan = || crate::api::transfer::TablePlan {
        state: state.clone(),
        task: task.cloned(),
        sid: sid.to_string(),
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
        target_table: target_table.to_string(),
        target_columns: target_column_names.clone(),
        workers_cap: opts.tgt_workers,
        slices_cap: opts.src_slices,
        insert_rows: opts.insert_rows,
        op_timeout_secs: opts.op_timeout_secs,
        pacer: opts.pacer.clone(),
        grow: opts.grow.clone(),
        target_types: target_types.clone(),
        key_columns: key_columns.clone(),
        key_indexes: key_indexes.clone(),
        paging_keys: paging_keys.clone(),
        src_primary: source_primary.clone(),
        where_clause: table_where.clone(),
        row_limit: opts.row_limit,
        upsert: upsert_mode,
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
        diag: diag.clone(),
        progress: progress.clone(),
        elapsed_base_ms: elapsed_base_ms.clone(),
        wait_ms: wait_ms.clone(),
        // 界面「跳过目标端 binlog」+ 环境变量逃生阀，两者在 `opts()` 里已合并
        skip_binlog: opts.skip_binlog,
    };
    // 重跑前要用的快照/判据：行数快照用于回退进度（上一次的行随事务回滚，不能计两遍）
    let counters_before = task.map(|t| (t.rows_read(), t.rows_written(), t.rows_failed()));
    // 重跑前会不会把目标表清空（「清空重灌」/「删除重建」两种模式都会，见函数注释）。
    // 它同时决定「断连后能不能整表重跑」—— 清空过就不存在写重复的问题。
    let overwrite_safe = can_clear_before_retry(opts.data_mode, opts.policy, same_table);
    let mut link_retries = 0usize;
    let stats = loop {
        // 整表重跑：上一次读出的行随事务回滚，进度从 0 重算（否则「无进展」会被历史值掩盖）
        progress.store(0, std::sync::atomic::Ordering::SeqCst);
        // 「已写入行数」同理：上一轮的行已随事务回滚，不清零的话失败时界面会虚高
        //（这个数字现在会显示在失败行的「新增」列，见 `WriteDiag::rows_written`）
        diag.rows_written.store(0, std::sync::atomic::Ordering::SeqCst);
        let attempt_started = std::time::Instant::now();
        let outcome = crate::api::transfer::run(make_plan()).await;
        // 这一轮不论成败都算进基准：失败那几轮同样占用了用户的墙钟（重跑 = 数据重读重写）
        elapsed_base_ms.fetch_add(
            attempt_started.elapsed().as_millis() as u64,
            std::sync::atomic::Ordering::SeqCst,
        );
        match outcome {
            Ok(stats) => break stats,
            Err(msg) => {
                // ---- 断连 ⇒ **自动重连重跑**（最多 `LINK_RETRY_MAX` 次） ----
                // 为什么敢重跑：目标端是「整表一个事务」（`txn_batch` 有定义）且事务真的
                // 开起来了（没退回过 autocommit）⇒ 连接一断，服务端把本次已写的行全部
                // 回滚，目标表里没有半截数据，从头再写一遍不会重复、也不会缺。
                // 反过来说，下面任何一条不成立就**绝不能重跑**（要么写重复，要么白跑）：
                //   · 方言没有事务批（Doris / ClickHouse / Oracle / DB2…）⇒ 已写的行还在；
                //   · 退回过逐条提交 ⇒ 行已 commit，重跑就是重复；
                //   · 不是链路级错误（脏数据 / 类型不匹配）⇒ 重跑必然再失败；
                //   · 已取消 / 已经用满重跑次数。
                // **例外**：清空重灌（TruncateInsert）在重跑前会把目标表清掉，所以
                // 「已提交过几路 / 逐条提交过」都不再是重复风险 —— 这三条由
                // `can_retry_after_link_break` 的 `overwrite_safe` 参数放行。真机上
                // 出现过跑到 85%、2.5 小时时链路抖十几秒，几张表直接判红（用户要求自动重连）。
                let can_retry = can_retry_after_link_break(
                    link_retries,
                    txn_batch(target_kind.kind.key()).is_some(),
                    diag.begin_failed
                        .load(std::sync::atomic::Ordering::SeqCst),
                    diag.committed.load(std::sync::atomic::Ordering::SeqCst),
                    task.map(|t| t.is_canceled()).unwrap_or(false),
                    &msg,
                    // 「清空重灌 / 删除重建」在重跑前都会把目标表清掉（见下面清表那段），
                    // 所以「已提交过几路 / 逐条提交过」都不再是重复风险。
                    overwrite_safe,
                );
                if !can_retry {
                    // 已经写进去的行要带出去：失败行过去恒显示 +0，用户看不出写到哪儿断的
                    return ObjResult::failed_with(ty, table, msg, diag.rows_written(), 0);
                }
                link_retries += 1;
                // 退避阶梯 2/5/10/20/30 秒（见 LINK_RETRY_BACKOFF_SECS）
                let backoff = LINK_RETRY_BACKOFF_SECS
                    .get(link_retries - 1)
                    .copied()
                    .unwrap_or(30);
                if let Some(task) = task {
                    // 先报数据库/驱动的原话
                    task.log(format!("表 {table}：{msg}"));
                    // 再明说「会自动重连」：不说的话用户看到报错就去点重新开始，
                    // 这一整张表前面搬的行全白费
                    task.log(format!(
                        "表 {table}：链路中断，{backoff} 秒后自动重连（第 {link_retries}/{LINK_RETRY_MAX} 次）"
                    ));
                }
                tracing::warn!(
                    target: "dbmind::sync",
                    table = %table,
                    attempt = link_retries,
                    max = LINK_RETRY_MAX,
                    "链接级失败：回滚后整表重跑"
                );
                // 退避：链路真的不行时，别把几次尝试挤在同一秒里再撞几次
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                // 先把这组会话放掉：断掉的那条连接可能还没被内核标记失效（宿主报的是
                // JDBC 层的 Communications link failure，不一定带连接级错误码），
                // 不释放就会拿同一条死连接重跑一遍。释放后下一次调用必然重连。
                release_table_sessions(state, src, tgt, sid).await;
                // 清空重来：「清空重灌 / 删除重建」重跑前都要让目标表回到空表（上一次的清空
                // 已提交，这里再确认一次）；追加 / upsert 模式**不能清空** —— 那会把目标表
                // 现有数据删掉，而事务回滚已经保证了没有半截数据，重跑分别是「追加一遍」与
                // 「幂等重放」。
                if overwrite_safe {
                    let clear = target_kind.clear_table_sql(&target_name);
                    // 清表本身也可能撞在**还没恢复的链路上**：真机实测目标库停机 25 秒，
                    // 第 1 次重连后 2 秒就来清表 ⇒ `Connection refused`，旧实现直接判失败，
                    // 5 次重连预算形同虚设（清表是重跑的第一步，第一步就撞死在死链上）。
                    // 这里按链路错误继续等，和整表重跑共用同一份重连预算（link_retries）。
                    loop {
                        match run_target_write(state, &tgt_target, &clear).await {
                            Ok(_) => {
                                // 表已清空 ⇒ 上一轮「已提交过 N 路 / 退回逐条提交」的重复
                                // 风险一并消失，标记归零（否则判定会拿历史值误杀后续轮次）。
                                diag.committed.store(0, std::sync::atomic::Ordering::SeqCst);
                                diag.begin_failed.store(false, std::sync::atomic::Ordering::SeqCst);
                                break;
                            }
                            Err(err) => {
                                let may_wait = is_link_level_message(&err.message)
                                    && link_retries < LINK_RETRY_MAX
                                    && !task.map(|t| t.is_canceled()).unwrap_or(false);
                                if !may_wait {
                                    // 清表这一步就失败：前几轮可能已经写进去过行，照实带出
                                    return ObjResult::failed_with(
                                        ty, table, err.message, diag.rows_written(), 0,
                                    );
                                }
                                link_retries += 1;
                                let wait = LINK_RETRY_BACKOFF_SECS
                                    .get(link_retries - 1)
                                    .copied()
                                    .unwrap_or(30);
                                if let Some(task) = task {
                                    task.log(format!("表 {table}：{}", brief_error(&err.message)));
                                    task.log(format!(
                                        "表 {table}：目标端还没恢复，{wait} 秒后重试清空（第 {link_retries}/{LINK_RETRY_MAX} 次）"
                                    ));
                                }
                                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                                release_table_sessions(state, src, tgt, sid).await;
                            }
                        }
                    }
                }
                if let (Some(task), Some((read, written, failed))) = (task, counters_before) {
                    task.reset_rows(read, written, failed);
                }
            }
        }
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
                note.push_str(&format!("（序列回拨失败 {}：{}）", column.name, err.message));
            }
        }
    }

    // 索引后置的收尾：数据全部落库后，再补建非唯一二级索引（见上面建表后那一段）
    create_deferred_indexes(
        state, src, source_kind, &tgt_target, target_kind, &target_name, table, task,
        deferred_indexes, &declared_primary, &source_columns,
    )
    .await;

    // 日志只报步骤；行数走 rowsWritten/rowsRead 计数（进度卡平滑递增），不进文本
    let _ = started;
    let message = format!(
        "传输完成{}{}",
        if created { "（已建表）" } else { "" },
        note
    );
    ObjResult::ok(ty, table, message, inserted, updated)
}

/// 一侧的「库/模式」名 —— DDL 里做限定前缀时用的那个：显式 schema 优先，
/// 否则从 scope 拆（`库.模式` 取模式，单段就是库名）。
fn side_schema_name(kind: &Dialect, scope: &str, schema: &Option<String>) -> Option<String> {
    if let Some(value) = schema.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        return Some(value.to_string());
    }
    let (db, sch) = crate::api::meta::split_db_schema(scope, kind);
    match sch.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(schema) => Some(schema),
        None => Some(db.trim().to_string()).filter(|d| !d.is_empty()),
    }
}

/// 只改写「代码段」，字符串字面量与注释原样保留。
fn map_sql_code(text: &str, f: impl Fn(&str) -> String) -> String {
    let bytes: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 16);
    let mut code = String::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let ch = bytes[i];
        // 字符串字面量：只认 SQL 标准/MySQL 的单引号 `'…'`（`''` 是转义）。
        // 双引号**不能**当字面量跳过 —— 它在 PG / SQL Server / Oracle 里是标识符，
        // `"src_db"."t"` 这种库名限定正是要改写的对象（真机：视图定义里带双引号库名时漏改）。
        if ch == '\'' {
            out.push_str(&f(&code));
            code.clear();
            let quote = ch;
            out.push(ch);
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                out.push(c);
                i += 1;
                if c == quote {
                    if i < bytes.len() && bytes[i] == quote {
                        out.push(quote);
                        i += 1;
                        continue;
                    }
                    break;
                }
            }
            continue;
        }
        // 行注释：-- 或 #
        if ch == '#' || (ch == '-' && i + 1 < bytes.len() && bytes[i + 1] == '-') {
            out.push_str(&f(&code));
            code.clear();
            while i < bytes.len() {
                let c = bytes[i];
                out.push(c);
                i += 1;
                if c == '\n' {
                    break;
                }
            }
            continue;
        }
        // 块注释
        if ch == '/' && i + 1 < bytes.len() && bytes[i + 1] == '*' {
            out.push_str(&f(&code));
            code.clear();
            while i < bytes.len() {
                let c = bytes[i];
                out.push(c);
                i += 1;
                if c == '*' && i < bytes.len() && bytes[i] == '/' {
                    out.push('/');
                    i += 1;
                    break;
                }
            }
            continue;
        }
        code.push(ch);
        i += 1;
    }
    out.push_str(&f(&code));
    out
}

/// 把源库里取到的 `CREATE VIEW / PROCEDURE / TRIGGER / EVENT` 定义改写到目标库。
///
/// 真机踩到的两个坑（整库同步时视图全挂，日志只留一句「建视图失败」）：
/// 1. **顺序**：视图在表还没建出来的时候就执行了 —— 由调用方保证（见 `db()` 里的
///    `tables_finished` 闸门）。
/// 2. **库名限定**：定义里带着源库名（`dmx_hk_src`.`t_department`），目标库叫别的名字时
///    直接 `Table 'dmx_hk_src.t_department' doesn't exist`；此外 MySQL 的定义带
///    `DEFINER=`u`@`h``，换台机器那个账号往往不存在，会报 SUPER 权限不足。
///    这里把库名换成目标库、并去掉 DEFINER 子句（换库/换实例时它没有任何意义）。
fn rewrite_definition(
    definition: &str,
    src_db: Option<&str>,
    tgt_db: Option<&str>,
    strip_definer: bool,
) -> String {
    let mut text = definition.to_string();
    if strip_definer {
        // DEFINER=`user`@`host` —— **必须连 @host 一起吃掉**：只抹掉 `user` 会留下
        // `@`%` SQL SECURITY DEFINER VIEW …` 这种残句，MySQL 直接语法错
        //（真机：整库同步所有视图一起挂，报错还指在 `@`%`` 上，很难看出是这里）。
        let ident = r#"(?:`(?:[^`]|``)*`|'(?:[^']|'')*'|"[^"]*"|[^\s@]+)"#;
        let re = regex::Regex::new(&format!(r#"(?i)\bDEFINER\s*=\s*{ident}(?:\s*@\s*{ident})?\s*"#)).unwrap();
        text = re.replace_all(&text, "").into_owned();
    }
    let (Some(from), Some(to)) = (src_db, tgt_db) else {
        return text;
    };
    if from.eq_ignore_ascii_case(to) || from.is_empty() || to.is_empty() {
        return text;
    }
    let from_q = format!("`{}`", from);
    let to_q = format!("`{}`", to);
    let from_dq = format!("\"{}\"", from);
    let to_dq = format!("\"{}\"", to);
    let from_br = format!("[{}]", from);
    let to_br = format!("[{}]", to);
    // 裸 `库名.`（前面不能紧贴标识符字符，避免改到 `my_dmx_hk_src.x` 这类名字里）
    let bare = regex::Regex::new(&format!(
        r#"(?i)(^|[^A-Za-z0-9_$`"\[\]]){}\."#,
        regex::escape(from)
    ))
    .unwrap();
    let bare_to = format!("${{1}}{}.", to);
    map_sql_code(&text, |code| {
        let code = code.replace(&from_q, &to_q).replace(&from_dq, &to_dq).replace(&from_br, &to_br);
        bare.replace_all(&code, bare_to.as_str()).into_owned()
    })
}

/// 跨类型同步「对象定义」（视图 / 存储过程 / 函数 / 触发器 / 事件）时的两步适配：
/// ① 摘掉 MySQL 专有子句；② 把反引号标识符换成目标方言的引号。
///
/// 真机踩到的两类错（MySQL → 目标库整库同步，视图/函数全失败）：
/// - **ClickHouse**：`CREATE ALGORITHM=UNDEFINED DEFINER=… SQL SECURITY INVOKER VIEW …`
///   报 `Code: 62. Syntax error: failed at position 8 ('ALGORITHM')`；
/// - **PostgreSQL**：定义里的 `CREATE VIEW `v_x`` 里的反引号 PG 不认，报
///   `ERROR: syntax error at or near "`" 位置 13`；函数则报 `syntax error at or near "" position: 17`。
///
/// ①只处理「头部」（到第一个 ` VIEW ` 关键字为止），视图体里同样出现的字样不会被误伤；
/// 同类型同步（MySQL → MySQL）两步都不生效，一个字都不动。
fn adapt_object_definition(definition: &str, source: &Dialect, target: &Dialect) -> String {
    let stripped = strip_mysql_view_options(definition, source, target);
    adapt_identifier_quoting(&stripped, source, target)
}

/// ① 摘掉 MySQL 视图头部专有的 `ALGORITHM=` / `SQL SECURITY …`（见 [`adapt_object_definition`]）。
fn strip_mysql_view_options(definition: &str, source: &Dialect, target: &Dialect) -> String {
    let mysql_like = |dialect: &Dialect| matches!(dialect.kind.key(), "mysql" | "mariadb");
    if !mysql_like(source) || mysql_like(target) {
        return definition.to_string();
    }
    // to_ascii_lowercase 不会改变字节长度，所以位置可以直接切原文
    let Some(at) = definition.to_ascii_lowercase().find(" view ") else {
        return definition.to_string();
    };
    let mut head = definition[..at].to_string();
    // 长的先抹：否则只抹掉 `sql security` 会留下一个孤零零的 `invoker`
    for clause in [
        "algorithm=undefined",
        "algorithm=merge",
        "algorithm=temptable",
        "sql security definer",
        "sql security invoker",
    ] {
        while let Some(pos) = head.to_ascii_lowercase().find(clause) {
            head.replace_range(pos..pos + clause.len(), "");
        }
    }
    let head = head.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{head}{}", &definition[at..])
}

/// 方言的标识符引号字符：MySQL 家族用反引号，SQL Server 用方括号，其余用双引号。
///
/// 不能拿 `Dialect::quote("_")` 反推 —— 它对"朴素名字"是不加引号的（`_` 正好是朴素名字），
/// 首字符会取成 `_` 本身。
fn quote_mark(dialect: &Dialect) -> char {
    match dialect.kind.key() {
        "mysql" | "mariadb" | "doris" | "clickhouse" => '`',
        "sqlserver" => '[',
        _ => '"',
    }
}

/// ② 把源端的反引号标识符换成目标方言的引号（只改**代码段**，字符串字面量与注释原样保留）。
///
/// MySQL 用反引号，PG / Oracle / H2 / Derby / DB2 用双引号，SQL Server 习惯方括号。
/// 同类型或目标本来就用反引号（MySQL / MariaDB / Doris / ClickHouse）时原样返回。
fn adapt_identifier_quoting(definition: &str, source: &Dialect, target: &Dialect) -> String {
    let (src_mark, tgt_mark) = (quote_mark(source), quote_mark(target));
    if src_mark != '`' || tgt_mark == '`' {
        return definition.to_string();
    }
    let re = regex::Regex::new(r"`((?:[^`]|``)*)`").unwrap();
    map_sql_code(definition, |code| {
        re.replace_all(code, |caps: &regex::Captures<'_>| {
            let inner = caps[1].replace("``", "`");
            match tgt_mark {
                '[' => format!("[{}]", inner.replace(']', "]]")),
                _ => format!("\"{}\"", inner.replace('"', "\"\"")),
            }
        })
        .into_owned()
    })
}

/// 视图：只有能拿到完整 `CREATE VIEW` 的类型才同步（见文件头决定 4）。
async fn sync_view(
    state: &AppState,
    src: &Side,
    tgt: &Side,
    name: &str,
    opts: &Opts,
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
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
        Err(err) => return ObjResult::failed("view", name, err.message),
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
    // 定义里带源库名 / DEFINER，原样拿到目标执行会失败（见 rewrite_definition 的注释）
    let src_db = side_schema_name(&source_kind, &src.scope, &src.schema);
    let tgt_db = side_schema_name(&target_kind, &tgt.scope, &tgt.schema);
    let definition = rewrite_definition(
        &definition,
        src_db.as_deref(),
        tgt_db.as_deref(),
        matches!(source_kind.kind.key(), "mysql" | "mariadb"),
    );
    // 跨类型：MySQL 专有的 `ALGORITHM=` / `SQL SECURITY …` 目标端不认。
    // 真机：MySQL → ClickHouse 报 `Code: 62. Syntax error: failed at position 8 ('ALGORITHM')`。
    let definition = adapt_object_definition(&definition, &source_kind, &target_kind);
    let target_name = target_kind.quote(name);
    let tgt_target = match crate::api::scope::resolve(state, &tgt.conn, &tgt.scope).await {
        Ok(target) => target,
        Err(err) => return ObjResult::failed("view", name, err.message),
    };
    if columns_of(state, tgt, name).await.map(|c| !c.is_empty()).unwrap_or(false)
        && opts.policy != Policy::Drop
    {
        return ObjResult::skipped("view", name, "目标已存在同名视图（保留结构策略下不动它）");
    }
    if opts.policy == Policy::Drop {
        if let Some(task) = task {
            task.log(format!("视图 {name}：删除目标视图"));
        }
        let _ = run_target(state, &tgt_target, &format!("drop view if exists {target_name}")).await;
    }
    if let Some(task) = task {
        task.log(format!("视图 {name}：创建视图"));
    }
    match run_target(state, &tgt_target, &definition).await {
        Ok(_) => ObjResult::ok("view", name, "同步完成", 0, 0),
        Err(err) => ObjResult::failed("view", name, err.message),
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
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
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
            return ObjResult::failed(kind, name, err.message)
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
    // 例程（过程 / 函数 / 触发器 / 事件）的**语法与运行时语言都是方言私有的**：
    // 把 MySQL 的 `CREATE FUNCTION … BEGIN … END` 发给 PG，即使引号改对了也照样语法错
    //（真机：MySQL → PG 整库同步，8 个函数、4 个存储过程全失败，日志只有一句 syntax error）。
    // 这类对象跨类型只能跳过并说明原因，硬试只会留一堆看不懂的报错。
    fn family(key: &str) -> &str {
        if matches!(key, "mysql" | "mariadb") {
            "mysql"
        } else {
            key
        }
    }
    if family(source_kind.kind.key()) != family(target_kind.kind.key()) {
        return ObjResult::skipped(
            kind,
            name,
            format!(
                "跨类型不搬{}：{} 与 {} 的{}语法与运行时语言不兼容（引号、流程控制、系统包都不同），需人工改写后再建",
                kind_label(kind),
                source_kind.kind.key().to_ascii_uppercase(),
                target_kind.kind.key().to_ascii_uppercase(),
                kind_label(kind),
            ),
        );
    }
    let definition = rewrite_definition(
        &definition,
        side_schema_name(&source_kind, &src.scope, &src.schema).as_deref(),
        side_schema_name(&target_kind, &tgt.scope, &tgt.schema).as_deref(),
        matches!(source_kind.kind.key(), "mysql" | "mariadb"),
    );
    // 跨类型：MySQL 专有的 `ALGORITHM=` / `SQL SECURITY …` 目标端不认（真机 ClickHouse 语法错）
    let definition = adapt_object_definition(&definition, &source_kind, &target_kind);
    let tgt_target = match crate::api::scope::resolve(state, &tgt.conn, &tgt.scope).await {
        Ok(target) => target,
        Err(err) => return ObjResult::failed(kind, name, err.message),
    };
    if opts.policy == Policy::Drop {
        match target_kind.drop_object_sql(kind, name) {
            Some(drop_sql) => {
                if let Some(task) = task {
                    task.log(format!("{} {name}：删除目标{}", kind_label(kind), kind_label(kind)));
                }
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
    if let Some(task) = task {
        task.log(format!("{} {name}：创建{}", kind_label(kind), kind_label(kind)));
    }
    match run_target(state, &tgt_target, &definition).await {
        Ok(_) => ObjResult::ok(kind, name, "同步完成", 0, 0),
        Err(err) => ObjResult::failed(kind, name, err.message),
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
    let src_record = require_record(&state, &src.conn).await?;
    let tgt_record = require_record(&state, &tgt.conn).await?;
    crate::api::ensure_data_transfer(&src_record)?;
    crate::api::ensure_data_transfer(&tgt_record)?;
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
    // 单表同步没有并发槽位概念：固定 0 号，且用完就释放（`reuse = false`）
    // 单表接口没有外层收尾监控，进度计数器只是满足 `TablePlan` 的字段
    let single_progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let outcome = sync_table(
        &state, None, &src, &tgt, &source_table, &target_table, &opts, 0, false, &single_progress,
    )
    .await;
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
    let source_record = require_record(&state, &body.source_connection_id).await?;
    crate::api::ensure_data_transfer(&source_record)?;
    let target_conn = body.target_connection();
    let target_record = require_record(&state, &target_conn).await?;
    crate::api::ensure_data_transfer(&target_record)?;

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
    // 调度顺序：按源表体量**降序**（大表先跑）。
    //
    // 之前按清单原序（库里的字母序）跑：小表先扫完，大表全挤在末尾，8 路并发跑到最后
    // 只剩一张大表 → 并发退化成单流，整库速度从 2 万行/s 掉到 3.8 千行/s（真机实测，
    // 最后 12 张大表占全库 59% 体积）。让大表先起跑，它就能和别的表共享并发窗口。
    // 非表对象（视图/例程）排最后；拿不到体量的方言保持原序。
    let sizes = source_table_rows(&state, &src, Dialect::new(source_record.kind())).await;
    if !sizes.is_empty() {
        objects.sort_by(|a, b| {
            let weight = |object: &(String, String)| -> i64 {
                if object.0 != "table" {
                    return -1;
                }
                sizes.get(&object.1.to_lowercase()).copied().unwrap_or(0).max(0)
            };
            weight(b).cmp(&weight(a))
        });
    }

    /// 对象级并发的**自适应并行度**（不写死）：
    ///
    /// 两个因素决定最优张数：
    /// 1. **目标库类型** —— 会话结构与写入语义差别很大：
    ///    - 嵌入式/文件引擎（SQLite/Derby/H2）：会话池只有一条连接，并行只会互相排队；
    ///    - Doris：每次导入落一个 part，并发太高小 part 合并压力大，适度即可；
    ///    - Oracle/DM/DB2：会话开销大、锁行为保守，2 路稳；
    ///    - MySQL/PG/SQL Server/ClickHouse 等网络型库：4 路能吃满带宽（再高压垮库反而慢）。
    /// 2. **对象数** —— 只有 2 张表时开 4 路毫无意义，并行度不超过对象数。
    fn adaptive_concurrency(target_kind: dbmind_core::ConnectionKind, objects: usize) -> usize {
        if objects <= 1 {
            return 1;
        }
        let target_kind_key = target_kind.key();
        // 嵌入式/文件引擎：会话池只有一条连接，并行只会互相排队。
        // **不能拿 single_connection_pool() 判这件事** —— 它标的是连接级语义，
        // SQL Server 在 plugins/connection-types/sqlserver.yaml 里也标了 true，
        // 照它早退会把网络型库的整库同步退化成一张一张跑（实测 4 表严格串行、
        // 日志「并发 1 张表」，与上面注释里「SQL Server 属网络型库、跑 4 路」自相矛盾）。
        if matches!(target_kind_key, "sqlite" | "h2" | "derby" | "duckdb") {
            return 1;
        }
        let by_kind = match target_kind_key {
            "doris" => 3,
            "oracle" | "dm" | "db2" => 3,
            // 网络型库（MySQL/PG/SQL Server/ClickHouse…）：**对象级**并发拉到 8。
            // 这里管的是「同时同步几张表」，不是「一张表开几条写连接」—— 跟表数据量无关。
            // 真实场景几乎都是「几十张小表」（均值一两万行），4 张表串成 12 轮，
            // 每轮都要重付建表/探测/建连的固定开销，吞吐被固定成本吃掉（真机：48 张表
            // 80 万行 4 分钟 ≈ 3.2k 行/秒）。8 张同时跑，固定成本摊掉一半。
            _ => 8,
        };
        // 逃生阀 / 调优开关：DBMIND_SYNC_OBJECTS=n 直接压过上面的自适应值。
        // 大机器想再压一压、或者库里连接数吃紧想退回去，都不用改代码重编。
        let wanted = std::env::var("DBMIND_SYNC_OBJECTS")
            .ok()
            .and_then(|text| text.trim().parse::<usize>().ok())
            .filter(|n| *n >= 1)
            .unwrap_or(by_kind);
        wanted.min(objects).max(1)
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

    // ---- 任务级会话预算：并发对象数 × 每表会话数 ----
    // 每张表的会话数是**切分**出来的：单表同步时独享方言声明的写并发
    //（`write_parallelism`，MySQL/PG = 4），同时跑 8 张表时每表 1 路 —— 这样
    // 「几十张小表」能并行摊掉建连/建表/探测的固定开销，而单张大表仍吃得到多路写。
    let target_kind = Dialect::new(require_record(&state, &tgt.conn).await?.kind());
    let source_kind = Dialect::new(require_record(&state, &src.conn).await?.kind());
    // 每表写并发：界面显式设了就用它（夹 1..=64），否则取目标方言声明值。
    // 它同时是「整任务的写并发预算」，尾部弹性扩路也不会超过它（用户填 1 就该始终 1 路）。
    let explicit_tgt = body.tgt_workers.filter(|n| *n >= 1).map(|n| n.min(64));
    let target_declared = explicit_tgt.unwrap_or_else(|| write_parallelism(target_kind.kind).max(1));
    // 两阶段 upsert 每张表还要一条专属会话做目标键查询/逐行更新。没有原生
    // upsert 的目标要把它算进预算，否则对象并发 × (writer + lookup) 会翻倍。
    let native_upsert = target_kind.supports_native_upsert();
    // 对象级预算 = 「同时几张表」的上限。方言声明的 4 是**每表**写并发的甜点值，
    // 不是整任务的连接上限：几十张小表并行时 8 张同时跑（8 条写 + 8 条读）对
    // MySQL/PG 这类网络库完全正常，比 4 张串 12 轮快得多（见 adaptive_concurrency）。
    let object_budget = target_declared.max(8);
    let target_object_cap = if native_upsert {
        object_budget
    } else {
        // 两阶段 upsert 每张表还要一条专属会话做键查询，预算对半分
        (object_budget / 2).max(1)
    };
    // 自适应值：同时受源端与目标端类型、对象总数约束（弱机自动算出 1）。
    let adaptive = adaptive_concurrency(target_kind.kind, total)
        .min(adaptive_concurrency(source_kind.kind, total))
        .min(target_object_cap)
        .max(1);
    // ===== 表级并发：界面可设，缺省 2 张 =====
    //
    // 单表内部本来就有「2 片读 + 2 路写」（见下面的 src_slices / tgt_workers），
    // 同时 2 张表已经能把源端读和目标端写都喂饱；再往上加，目标端的连接数、
    // 行锁与 IO 争抢成倍上升 —— 真机那次「后期几乎每张表都卡 601 秒」正是
    // 手工把并发表数设成 8 的时候（8 表 ×（2 读 + 1 写）= 24 条会话压同一个库）。
    //
    // 所以界面上**允许调小，也允许调大**（用户要求「自己能设」），但缺省仍是 2；
    // 弱机（SQLite/H2/Derby 等单连接池方言）自适应会给 1，缺省路径取小。
    // 逃生阀 DBMIND_SYNC_CONCURRENCY 只在界面没填时生效。
    let env_concurrency = std::env::var("DBMIND_SYNC_CONCURRENCY")
        .ok()
        .and_then(|text| text.trim().parse::<usize>().ok())
        .filter(|n| *n >= 1)
        .unwrap_or(2);
    // 每表读并发：界面显式设了就把它当成**本次任务的读预算**（`source_declared`），
    // 后面的份额切分与尾部弹性扩片都受它约束。
    let explicit_src = body.src_slices.filter(|n| *n >= 1).map(|n| n.min(64));
    let source_declared = explicit_src.unwrap_or_else(|| write_parallelism(source_kind.kind).max(1));
    let reserved_sessions = usize::from(!native_upsert);
    // 表数：界面显式给了就以用户为准（不再被「固定 2 张」压回）
    let explicit_objects = body.max_concurrency.filter(|n| *n >= 1).map(|n| n.min(64));
    let sync_concurrency = match explicit_objects {
        Some(n) => n.min(total.max(1)),
        // 缺省路径：每张**在跑**的表至少 2 路读 —— 实测单流只吃到链路的一半
        // （4.0 MB/s ≈ 4,400 行/秒），两路并发才接近打满（9.2 MB/s）。
        // 宁可少开一张表，也不让在读的表退化成单流（真机：re_sale 单表 1 路、
        // 页 3711，读 2256 秒只前进几万行）。单连接池的源（declared = 1）会被
        // 这个 min 还原成 1 表 1 片。
        None => env_concurrency
            .min(adaptive)
            .min(total.max(1))
            .min((source_declared / 2).max(1)),
    };
    let concurrency_source = if explicit_objects.is_some() {
        "（界面设置）"
    } else {
        "（自动）"
    };
    // 每表读片：显式值原样当份额；缺省按「读预算 / 并发表数」切，且不低于 2 片
    let src_slices = if explicit_src.is_some() {
        source_declared
    } else {
        (source_declared / sync_concurrency).max(2).min(source_declared)
    };
    let tgt_workers = if explicit_tgt.is_some() {
        target_declared
    } else {
        (target_declared / sync_concurrency)
            .saturating_sub(reserved_sessions)
            .max(1)
    };
    let opts = opts.with_session_caps(tgt_workers, src_slices);
    // 尾部弹性配额：整库同步的所有表共用一份（见 `GrowCaps`）。初始值就是上面的份额，
    // 表任务里的监控协程会在**收尾阶段**按「在跑的表数」把它调高。
    let grow = std::sync::Arc::new(crate::api::transfer::GrowCaps::default());
    let opts = opts.with_grow(grow.clone());

    let registry = state.tasks.clone();
    let task = registry.spawn("sync", "sync_", move |task| {
        let state = state.clone();
        let src = src.clone();
        let tgt = tgt.clone();
        let opts = opts.clone();
        async move {
            // 目标端写入参数体检（只有 MySQL 系有这几项）：只提示，不改用户库配置
            if matches!(target_kind.kind.key(), "mysql" | "mariadb") {
                if let Ok(preflight) =
                    crate::api::scope::resolve(&state, &tgt.conn, &tgt.scope).await
                {
                    warn_target_write_limits(&state, &preflight).await;
                }
            }
            task.set_total(total as i64);
            // 表与依赖表对象的数量：决定要不要等闸门（见下面的 tables_done）
            let table_count = objects.iter().filter(|(kind, _)| kind == "table").count();
            // 任务级开始行：第一个对象真正启动前界面不该是空白
            //（真机反馈：读计数已经在涨、日志区还停在「等待同步开始…」）。
            //
            // 明细按类型逐个列（表/视图/存储过程/函数/触发器/事件），只列非零项：
            // 「其余为视图/例程」这种含糊说法在整库同步时等于没说 —— 用户看不出
            // 到底要搬几张表、几个例程，也无从判断清单选得对不对。
            let mut parts: Vec<String> = Vec::new();
            for kind in ["table", "view", "procedure", "function", "trigger", "event"] {
                let count = objects.iter().filter(|(ty, _)| ty == kind).count();
                if count > 0 {
                    parts.push(format!("{} {count} 个", kind_label(kind)));
                }
            }
            let detail = if parts.is_empty() {
                String::new()
            } else {
                format!("（{}）", parts.join("，"))
            };
            // 每批行数 / 单语句超时**不进界面日志**：那是配置参数，不是「同步了什么」
            //（用户口径：第一行只讲要搬哪些对象，别在后面念参数）。值仍然进 tracing，
            // 排查「界面填的没生效」时照样查得到。
            let batch_rows = if opts.insert_rows > 0 {
                opts.insert_rows
            } else {
                insert_rows_default(target_kind.kind.key())
            };
            let timeout_note = match opts.op_timeout_secs {
                s if s < 0 => "不限制".to_string(),
                0 => "继承连接".to_string(),
                s => format!("{s} 秒"),
            };
            task.log(format!("开始同步：{total} 个对象{detail}"));
            // 生效的并发表数也写一行（只一行）：用户问过「默认不是 2 张吗，怎么这么多一起开工」，
            // 而实际值可能来自界面的「并发表数」，也可能是重跑时沿用了旧任务的设置 ——
            // 不写出来就只能靠猜（默认 2 张，显式设置以用户为准）。
            task.log(format!("同时搬 {sync_concurrency} 张表{concurrency_source}"));
            // 调度细节只进 tracing（debug）：它们是「内部怎么排兵布阵」，用户看不懂也不需要看
            tracing::debug!(
                target: "dbmind::sync",
                table_count,
                total,
                sync_concurrency,
                concurrency_source,
                batch_rows,
                timeout_note = %timeout_note,
                tgt_workers = opts.tgt_workers,
                src_slices = opts.src_slices,
                target_declared,
                "整库同步开始（表先于视图/例程，会话预算如上）"
            );
            // 目标库不存在时自动创建（默认开，见 ensure_target_db）
            if let Some(msg) = ensure_target_db(&state, &tgt, &opts).await {
                task.log(msg);
            }
            // ===== 对象级**并行**同步 =====
            // 单对象内部已经是「读一页写一页」的预取流水线，多对象再并行同时搬 ——
            // 并行度**自适应**（见 adaptive_concurrency）：按目标库类型与对象数动态定，
            // 不写死。计数与日志都走 Task 的锁内计数器，并发安全；结果按**提交顺序**
            // 回填槽位，界面上的结果表顺序稳定。
            // `sync_concurrency` 与每表会话预算都在 spawn 前算好（见上），这里直接用。
            let slots: Vec<std::sync::Arc<std::sync::Mutex<Option<Value>>>> = objects
                .iter()
                .map(|_| std::sync::Arc::new(std::sync::Mutex::new(None)))
                .collect();
            // 每张表的「已读行数」：收尾时按**有没有进展**判对象超时，而不是拿固定总时长
            // 判死（5000 万行有键表要 30 分钟以上，无主键 500 万行要 11 分钟）
            let progresses: Vec<std::sync::Arc<std::sync::atomic::AtomicU64>> = objects
                .iter()
                .map(|_| std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)))
                .collect();
            let counters = std::sync::Arc::new(std::sync::Mutex::new((
                0u64, 0u64, 0u64, 0u64, 0u64,
            )));
            let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(sync_concurrency));
            // 并发**槽位**池：槽位号决定会话键（`sync_slot_sid`），表跑完把号还回去，
            // 同一槽位的下一张表就**复用同一组物理连接**（省掉每表 1~3 次建连；
            // 隧道下一次 1.5~6.4 秒，几百张表就是几十分钟）。数量 = 并发上限，
            // 所以连接总数恒为「并发 × 每表会话数」，与表的总数无关。
            let slots_free: std::sync::Arc<std::sync::Mutex<Vec<usize>>> =
                std::sync::Arc::new(std::sync::Mutex::new(
                    (0..sync_concurrency.max(1)).collect::<Vec<usize>>(),
                ));
            // 在跑的表数：弹性会话预算的分母。整库同步的**尾部**常常只剩一两张大表，
            // 固定切分（并发 8 张时各 1 路写）会让它们单路爬完最后一大段 ——
            // 每退一张表，省下的写路数就分给还在跑的（见下面 sync_table 调用处）。
            let active_tables = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            // ---- 依赖表的对象要等**所有表**跑完再跑 ----
            // 原来 48 张表和视图/存储过程混在一个并发池里，视图几乎总是抢在表前面：
            // 定义里引用的表还没建出来，直接就 `Table 'dmx_hk_src.t_department' doesn't exist`
            //（真机截图）。触发器更糟 —— 抢在灌数据之前建好，搬数据时会被真的触发，数据被二次加工。
            // 所以：非表对象先等这个闸门（表全跑完 = 建完 + 数据搬完），再抢并发许可。
            // 注意**不能**先拿许可再等 —— 那会让许可都堵在等闸门的视图手里，表反而开不了工。
            let (tables_done_tx, tables_done_rx) = tokio::sync::watch::channel(table_count == 0);
            let tables_left = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(table_count));
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
                let progress = progresses[index].clone();
                let counters = counters.clone();
                let sem = sem.clone();
                let slots_free = slots_free.clone();
                let tables_done_rx = tables_done_rx.clone();
                let tables_done_tx = tables_done_tx.clone();
                let tables_left = tables_left.clone();
                let active_tables = active_tables.clone();
                handles.push(tokio::spawn(async move {
                    let is_table = kind == "table";
                    // 遇错停止 / 取消：还没开跑的对象不再执行（已开跑的照常跑完当前页）
                    let blocked = task.is_canceled()
                        || (opts.stop_on_error && {
                            let (_, _, errors, _, _) = *counters.lock().unwrap();
                            errors > 0
                        });
                    // 非表对象：等所有表收尾（取消/遇错停止时不白等，下面 blocked 会直接返回）
                    if !is_table && !blocked && !*tables_done_rx.borrow() {
                        let mut rx = tables_done_rx.clone();
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(6 * 3600),
                            rx.wait_for(|done| *done),
                        )
                        .await;
                    }
                    // 表跑完了（含失败/跳过）就给闸门放行 —— 两个出口都要记
                    let release_gate = |left: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
                                        tx: &tokio::sync::watch::Sender<bool>| {
                        if left.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                            let _ = tx.send(true);
                        }
                    };
                    let permit = tokio::time::timeout(PIPE_WAIT, sem.acquire_owned()).await.ok().and_then(|r| r.ok());
                    // 拿到许可 ⇒ 池里一定有空号；租约在本次对象结束（含 panic）时归还
                    let lease = {
                        let mut free = slots_free.lock().unwrap_or_else(|e| e.into_inner());
                        SlotLease {
                            id: free.pop().unwrap_or(0),
                            pool: slots_free.clone(),
                        }
                    };
                    if blocked {
                        if is_table {
                            release_gate(&tables_left, &tables_done_tx);
                        }
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
                    // 表的「开始」行由 `sync_table` 自己打（单表同步那条路也要有）；
                    // 这里补非表对象 —— 视图/例程排在所有表之后，没这条就只剩「等待」。
                    if kind != "table" {
                        task.log(format!("{} {name}：开始", kind_label(&kind)));
                    }
                    let outcome = match kind.as_str() {
                        "table" => {
                            // 弹性会话预算只在**收尾阶段**放宽（剩余表数 ≤ 并发上限）：
                            // 任务前中段并发一直是满的，此时放宽只会放大建连风暴
                            //（SSH 跳板机对瞬时连接数有限流，真机踩过整批卡死）；
                            // 到尾部只剩一两张大表时，才把省下的写路数让给在跑的表。
                            let (guard, active) = ActiveGuard::enter(&active_tables);
                            let winding_down = tables_left
                                .load(std::sync::atomic::Ordering::SeqCst)
                                <= sync_concurrency;
                            let workers =
                                (target_declared / active).saturating_sub(reserved_sessions).max(1);
                            let slices = (source_declared / active).max(1);
                            let table_opts = if winding_down
                                && (workers > opts.tgt_workers || slices > opts.src_slices)
                            {
                                tracing::debug!(
                                    target: "dbmind::sync",
                                    table = %name,
                                    active,
                                    workers,
                                    slices,
                                    "尾部放宽会话预算"
                                );
                                opts.clone().with_session_caps(workers, slices)
                            } else {
                                opts.clone()
                            };
                            let outcome = sync_table(
                                &state, Some(&task), &src, &tgt, &name, &name, &table_opts,
                                lease.id, true, &progress,
                            )
                            .await;
                            drop(guard);
                            outcome
                        }
                        "view" => {
                            sync_view(&state, &src, &tgt, &name, &opts, Some(&task)).await
                        }
                        // 例程 / 触发器 / 事件：与视图同一条路（见 sync_object_source）
                        "procedure" | "function" | "trigger" | "event" => {
                            sync_object_source(
                                &state,
                                &src,
                                &tgt,
                                &kind,
                                &name,
                                &opts,
                                Some(&task),
                            )
                            .await
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
                    // 对象级失败**当场**上报进度卡：建表失败这类一行都没读到，行失败计数
                    // 永远是 0 ⇒ 界面在运行中一直显示「失败 0」，要到终态结果表才跳
                    //（真机：3 张表漏建、进度卡「失败 0」）。
                    if !matches!(outcome.status.as_ref(), "ok" | "skipped") {
                        task.add_objects_failed(1);
                    }
                    // 表的数据搬运结果已经由 transfer 引擎打过一行（含行数/耗时/速率），
                    // 这里再打「传输完成（已建表）」就是纯重复 —— 400 张表就是 800 条。
                    // 跳过 / 失败 / 只建表没搬数据 仍要打：那是用户真正要看的结果。
                    let already_reported =
                        outcome.ty == "table" && outcome.message.starts_with("传输完成");
                    if !outcome.message.is_empty() && !already_reported {
                        // 日志带对象类型前缀（表/视图/存储过程…），与「表 xxx：开始」一致；
                        // 原因本身过 brief_error：只留数据库/驱动原话（剥「SQL 执行失败：」、
                        // 「(conn=…)」、内部会话键，取首行）
                        task.log(format!(
                            "{} {name}：{}",
                            kind_label(&outcome.ty),
                            brief_error(&outcome.message)
                        ));
                    }
                    *slot.lock().unwrap() = Some(outcome.to_json());
                    task.add_done(1);
                    if is_table {
                        release_gate(&tables_left, &tables_done_tx);
                    }
                    drop(permit);
                }));
            }
            // ---- 尾部弹性：把退出的表腾出来的配额还给**还在跑的表** ----
            // 表开始时就按「当时并排几张表」把会话预算切死了；尾部只剩一两张大表时，
            // 别的表退出的那份白扔 —— 真机实测 700 万行同步：两表并行 30k 行/秒，
            // 小表一结束、只剩 500 万行那张，掉到 18k 行/秒（当初只分到 2 片读 + 1 路写）。
            // 这里按「在跑的表数」重算份额，已在跑的表由引擎补令牌就地扩片，不重切表。
            let grow_task = {
                let grow = grow.clone();
                let active = active_tables.clone();
                let left = tables_left.clone();
                tokio::spawn(async move {
                    let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
                    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    loop {
                        tick.tick().await;
                        // 在跑的表数**每拍都要刷**（不受下面「只在收尾阶段放宽」的限制）：
                        // 表内读预取要靠它判断「这张表是不是独占链路」。
                        let active_now = active.load(std::sync::atomic::Ordering::SeqCst).max(1);
                        grow.set_active(active_now);
                        // 只在**收尾阶段**放宽（剩余表数 ≤ 并发上限）：任务前中段并发是满的，
                        // 放宽只会放大建连风暴（SSH 跳板机对瞬时连接数有限流，真机整批卡死过）
                        if left.load(std::sync::atomic::Ordering::SeqCst) > sync_concurrency {
                            continue;
                        }
                        let slices = (source_declared / active_now).max(1);
                        // **写路数不再按「剩几张表」放大**：尾部弹性原本会把每表的写路数
                        // 从 target_declared/并发 提到 target_declared/active_now（只剩 1 张表时
                        // 就是全部），真机上后期的表现是目标端被更多并发写压垮 —— 一页几万行
                        // 同时开 N 个事务，磁盘 IOPS 打满、单页写入从几秒恶化到十几分钟。
                        // 读片可以扩（读侧不压目标端，多读的页只是排在有界队列里），
                        // 写路数保持开工时按并发切的份额（`tgt_workers`）。
                        grow.set(slices, tgt_workers);
                    }
                })
            };
            // 对象级「无进展」判死前先探活：链路还通就说明只是这一页/这一批慢
            //（真机 400 对象的后段宽表，一页十几分钟 —— 按时间判死会把它们全报成中断）。
            let probe_target_str = match crate::api::scope::resolve(&state, &tgt.conn, &tgt.scope).await
            {
                Ok(target) => target,
                Err(err) => {
                    tracing::warn!(error = %err.message, "解析目标连接失败，无进展判定不做探活");
                    String::new()
                }
            };
            for (index, mut handle) in handles.into_iter().enumerate() {
                // 超时 / 协程 panic **不能静默丢弃**：那个对象既没有结果行、也没有 add_done，
                // 于是界面显示「完成 400/401」、errors=0、进度条永远差一格，用户只能怀疑
                // 「最后一张表没同步」。
                //
                // 判据是「**无进展**超过 PIPE_WAIT」，不是总时长：有键 5000 万行要跑半小时、
                // 无主键 500 万行（OFFSET 翻页 7.2k 行/秒）要 11.4 分钟 —— 按总时长判会把
                // 一直在搬数据的表整表放弃，这正是「完成 400/401」的真身。
                let progress = progresses[index].clone();
                let mut seen = progress.load(std::sync::atomic::Ordering::SeqCst);
                let mut idle_since = std::time::Instant::now();
                let joined = loop {
                    match tokio::time::timeout(std::time::Duration::from_secs(30), &mut handle).await
                    {
                        Ok(result) => break Some(result),
                        Err(_) => {
                            let now = progress.load(std::sync::atomic::Ordering::SeqCst);
                            if now != seen {
                                seen = now;
                                idle_since = std::time::Instant::now();
                            } else if idle_since.elapsed() >= PIPE_WAIT {
                                // 先探活：目标端还连得上 ⇒ 只是这个对象慢，继续等。
                                // 宽表/大字段一页在丢包链路上要十几分钟，按「无进展」直接判死
                                // 就是用户看到的「大表前面都不报错，越到后面越报中断」。
                                if !probe_target_str.is_empty()
                                    && probe_target_link_confirmed(&state, &probe_target_str).await
                                {
                                    idle_since = std::time::Instant::now();
                                    let (kind, name) = (
                                        objects[index].0.clone(),
                                        objects[index].1.clone(),
                                    );
                                    task.log(format!(
                                        "{} {name}：{} 分钟没有进度，目标端探活正常，继续等待",
                                        kind_label(&kind),
                                        PIPE_WAIT.as_secs() / 60
                                    ));
                                    continue;
                                }
                                // 探活也不通 ⇒ 真卡死了。
                                // 必须真的掐掉：JoinHandle 被丢弃只是**脱离**协程，它稍后跑完
                                // 还会再调一次 add_done，进度就会超过总数（完成 402/401）。
                                handle.abort();
                                break None;
                            }
                        }
                    }
                };
                let bad: Option<String> = match joined {
                    None => Some(format!(
                        "对象 {} 分钟没有任何进展，目标端探活不通",
                        PIPE_WAIT.as_secs() / 60
                    )),
                    Some(Ok(())) => None,
                    Some(Err(_)) => {
                        Some("对象任务异常终止（驱动异常或内存不足）".to_string())
                    }
                };
                if let Some(reason) = bad {
                    let (kind, name) = (
                        objects[index].0.clone(),
                        objects[index].1.clone(),
                    );
                    // 失败的表协程已经死掉，走不到 release_gate —— 计数得在这里替它扣掉，
                    // 否则闸门永远开不了：等表的视图/存储过程会一直干等到 6 小时上限，
                    // 最后全体报超时（比原本少一行结果还难看）。
                    // 只扣自己这一次，所以不会重复扣；扣到 1 说明它是最后一张表，放行闸门。
                    if kind == "table"
                        && tables_left.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) == 1
                    {
                        let _ = tables_done_tx.send(true);
                    }
                    *slots[index].lock().unwrap() = Some(json!({
                        "type": kind, "name": name, "status": "error",
                        "message": reason, "inserted": 0, "updated": 0,
                    }));
                    {
                        let mut c = counters.lock().unwrap();
                        c.2 += 1; // errors
                    }
                    task.add_objects_failed(1);
                    task.add_done(1);
                    task.log(format!("{name}：{reason}"));
                }
            }
            // 所有表都已收尾：弹性监控到此为止（否则它会一直空转）
            grow_task.abort();
            // ---- 视图之间也可能互相引用：首轮没建成的，串行重试一轮 ----
            // 表已经全部建好，剩下的依赖只可能来自**别的视图**（v2 建在 v1 上）。
            // 并发首轮里 v1 可能还没建出来，v2 先执行就必然 `Table 'db.v1' doesn't exist`
            //（真机必现，概率取决于谁先抢到连接）。串行重试让「被依赖的先成功」逐步收敛。
            if !task.is_canceled() {
                let failed_views: Vec<usize> = objects
                    .iter()
                    .enumerate()
                    .filter(|(index, (kind, _))| {
                        kind == "view"
                            && slots[*index]
                                .lock()
                                .map(|slot| {
                                    slot.as_ref()
                                        .and_then(|value| value.get("status"))
                                        .and_then(Value::as_str)
                                        == Some("error")
                                })
                                .unwrap_or(false)
                    })
                    .map(|(index, _)| index)
                    .collect();
                if !failed_views.is_empty() {
                    tracing::info!(
                        pending = failed_views.len(),
                        "视图依赖：首轮未建成，等其它视图建好后串行重试"
                    );
                    for index in failed_views {
                        let name = objects[index].1.clone();
                        let outcome =
                            sync_view(&state, &src, &tgt, &name, &opts, Some(&task)).await;
                        if outcome.status == "error" {
                            // 重试也失败：**必须把原因打出来**。否则日志最后停在
                            // 「创建视图」一行，收尾却是「失败 1」，中间的原因全丢了。
                            task.log(format!("视图 {name}：{}", outcome.message));
                            continue;
                        }
                        *slots[index].lock().unwrap() = Some(outcome.to_json());
                        {
                            let mut c = counters.lock().unwrap();
                            if outcome.status == "skipped" {
                                c.1 += 1;
                            } else {
                                c.0 += 1;
                            }
                            c.2 = c.2.saturating_sub(1);
                        }
                        // 视图依赖重试成功：把首轮记在进度卡上的那一次失败撤掉
                        task.sub_objects_failed(1);
                        tracing::debug!(view = %name, "视图依赖重试成功");
                    }
                }
            }
            // ---- 终态校验：对象都建出来了吗、行数都对得上吗 ----
            //
            // 为什么非要有这一步：链路断一次就会重跑一段，重跑区间与已提交区间叠一小段时，
            // 目标会多出几行；视图建了一半、目标表压根没建出来…… 这些都不会让任务本身变成
            // 「失败」，只会体现在「两边的对象 / 行数对不上」。所以收尾时逐对象核对一遍，
            // 并把**明细**写进日志（哪个对象不一致、差多少行），不一致的清空重传一次。
            // 用户要的就是这一句答案：400 个对象、8100 万行，到底都到齐了没有。
            // 代价：每张表两条 count(*)（百万行级几十毫秒）+ 一次元数据列举，整库约一分钟。
            let mut verify_report = Value::Null;
            if !task.is_canceled() && opts.sync_data {
                task.set_phase("校验".to_string());
                // 只有「目标被整体重建」时两边行数才可比：
                //   · 「删除后重建」「清空后导入」⇒ 目标就是源的一份副本；
                //   · 「保留目标 / 追加 / 合并」⇒ 目标本来就可能多出历史行，比了必然"不一致"，
                //     而"不一致"的动作是 truncate + 重传 —— 那是**删用户数据**，绝不能做；
                //   · 设了「每表行数上限」时引擎只搬前 N 行，源端 count 是全表，同样不可比。
                // 不可比时只核对「对象建出来没有」，不做行数判定。
                let full_copy = (opts.policy == Policy::Drop
                    || opts.data_mode == DataMode::TruncateInsert)
                    && opts.row_limit == 0;
                if !full_copy {
                    task.log("校验：本次是增量/合并模式或设了行数上限，只核对对象是否建出来（不比较行数）");
                }
                // 目标端现有对象清单（一次元数据调用）：用来判「表 / 视图到底建出来没有」。
                // 拿不到就退化成只比行数 —— 不因为一次元数据失败就把整轮校验废掉。
                let target_names: Option<std::collections::HashSet<String>> =
                    match crate::api::scope::resolve(&state, &tgt.conn, &tgt.scope).await {
                        Ok(resolved) => {
                            let engine = state.engine();
                            blocking(move || engine.list_tables_fresh(&resolved))
                                .await
                                .ok()
                                .map(|list| {
                                    list.into_iter()
                                        .map(|table| table.name.to_ascii_lowercase())
                                        .collect()
                                })
                        }
                        Err(_) => None,
                    };
                let mut checked = 0usize;
                // (槽位, 对象名, 原因)：先把明细收齐，日志才是「先列问题、再补」
                let mut bad: Vec<(usize, String, String)> = Vec::new();
                for (index, (kind, name)) in objects.iter().enumerate() {
                    if task.is_canceled() {
                        break;
                    }
                    if kind != "table" && kind != "view" {
                        continue;
                    }
                    let status = slots[index]
                        .lock()
                        .unwrap()
                        .as_ref()
                        .and_then(|value| value.get("status"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    // 失败 / 跳过的对象已经记在结果表里，不在这里重复报一遍
                    if status.as_deref() != Some("ok") {
                        continue;
                    }
                    checked += 1;
                    task.set_phase(format!("校验 {checked}/{total}"));
                    // 目标名可能是映射出来的（界面的「表名映射」），核对必须按目标名来
                    let target_name = opts.target_table_for(name, name);
                    if let Some(names) = &target_names {
                        if !names.contains(&target_name.to_ascii_lowercase()) {
                            let reason = "目标端没有这个对象".to_string();
                            task.log(format!(
                                "{} {name}：{reason}",
                                if kind == "view" { "视图" } else { "表" }
                            ));
                            bad.push((index, name.clone(), reason));
                            continue;
                        }
                    }
                    // 视图只有「在不在」可核对：行数取决于定义，比了没有意义
                    if kind != "table" {
                        continue;
                    }
                    // 增量 / 合并 / 有行数上限：行数不可比（见 full_copy 的说明）
                    if !full_copy {
                        continue;
                    }
                    let src_table = src.scoped(name, source_kind);
                    let tgt_table = tgt.scoped(&target_name, target_kind);
                    let (want, got) = (
                        count_table(
                            &state,
                            &src.conn,
                            &src.scope,
                            &src_table,
                            &opts.where_for(name),
                        )
                        .await,
                        count_table(&state, &tgt.conn, &tgt.scope, &tgt_table, "").await,
                    );
                    match (want, got) {
                        (Some(want), Some(got)) if want == got => {}
                        (Some(want), Some(got)) => {
                            let gap = if got < want {
                                format!("少 {}", want - got)
                            } else {
                                format!("多 {}", got - want)
                            };
                            let reason = format!("源 {want} 行 / 目标 {got} 行（{gap}）");
                            task.log(format!("表 {name}：不一致，{reason}"));
                            bad.push((index, name.clone(), reason));
                        }
                        // 源端读不到行数：不是「一致」，但也不是能自动修的东西 —— 如实说一句
                        (None, _) => {
                            task.log(format!("表 {name}：读不到源端行数，本次无法核对"));
                        }
                        (_, None) => {
                            let reason = "目标端读不到行数（表可能没建出来）".to_string();
                            task.log(format!("表 {name}：{reason}"));
                            bad.push((index, name.clone(), reason));
                        }
                    }
                }
                let mismatched = bad.len();
                let mut fixed = 0usize;
                if mismatched > 0 {
                    task.log(format!("校验：{mismatched} 个对象不一致，开始重传"));
                }
                for (index, name, _reason) in &bad {
                    if task.is_canceled() {
                        break;
                    }
                    task.set_phase(format!("重传 {name}"));
                    let kind = objects[*index].0.clone();
                    let target_name = opts.target_table_for(name, name);
                    if kind == "view" {
                        let outcome =
                            sync_view(&state, &src, &tgt, name, &opts, Some(&task)).await;
                        let rebuilt = outcome.status != "error";
                        if rebuilt {
                            fixed += 1;
                            task.log(format!("视图 {name}：已重建"));
                        } else {
                            task.log(format!("视图 {name}：{}", outcome.message));
                        }
                        *slots[*index].lock().unwrap() = Some(outcome.to_json());
                        continue;
                    }
                    let tgt_table = tgt.scoped(&target_name, target_kind);
                    // 表不存在时没法 truncate（会报 Table doesn't exist）：直接让 sync_table 建
                    let exists = target_names
                        .as_ref()
                        .map(|names| names.contains(&target_name.to_ascii_lowercase()))
                        .unwrap_or(true);
                    // 只有「目标就是源的副本」时才允许清空重灌；增量/合并模式下这条表本来就
                    // 该保留历史行，truncate 等于替用户删数据（不可比时也不会走到这里，双保险）
                    if exists && full_copy {
                        // SQLite 没有 truncate；其它方言 ATOMIC/隐式提交都一样快
                        let clear = if target_kind.kind.key() == "sqlite" {
                            format!("delete from {tgt_table}")
                        } else {
                            format!("truncate table {tgt_table}")
                        };
                        // 可写会话：`run_sql_in` 走只读会话（`read_only: Some(true)`），
                        // truncate 会被驱动直接拒绝（`Connection is read-only...`），
                        // 于是「发现不一致 → 清空重传」永远停在跳过重传这一步。
                        if let Err(err) =
                            run_sql_in_write(&state, &tgt.conn, &tgt.scope, clear, 600_000).await
                        {
                            task.log(format!("表 {name}：{}", err.message));
                            continue;
                        }
                    }
                    // 重跑要走正常的对象流程（会话租约、进度、结果行都要有）
                    let lease = {
                        let mut free = slots_free.lock().unwrap_or_else(|e| e.into_inner());
                        SlotLease {
                            id: free.pop().unwrap_or(0),
                            pool: slots_free.clone(),
                        }
                    };
                    let outcome = sync_table(
                        &state,
                        Some(&task),
                        &src,
                        &tgt,
                        name,
                        name,
                        &opts,
                        lease.id,
                        true,
                        &progresses[*index],
                    )
                    .await;
                    *slots[*index].lock().unwrap() = Some(outcome.to_json());
                    // 重传本身失败（建表/写入报错）：原因必须打出来。
                    // 否则日志只剩「重传后仍不一致（源 X 行 / 目标 Y 行）」，用户只能看到差多少、
                    // 看不到为什么 —— 真机上那几百万行的缺口就是被这句吞掉的。
                    if outcome.status == "error" {
                        task.log(format!("表 {name}：重传失败：{}", outcome.message));
                    }
                    // 复检：重传后再数一遍，「补上了没有」也写进日志
                    let src_table = src.scoped(name, source_kind);
                    let want = count_table(
                        &state,
                        &src.conn,
                        &src.scope,
                        &src_table,
                        &opts.where_for(name),
                    )
                    .await;
                    let got = count_table(&state, &tgt.conn, &tgt.scope, &tgt_table, "").await;
                    match (want, got) {
                        (Some(want), Some(got)) if want == got => {
                            fixed += 1;
                            task.log(format!("表 {name}：已重传，行数对齐（{want} 行）"));
                        }
                        (Some(want), Some(got)) => {
                            task.log(format!(
                                "表 {name}：重传后仍不一致（源 {want} 行 / 目标 {got} 行）"
                            ));
                        }
                        _ => {
                            task.log(format!("表 {name}：重传后读不到行数，无法确认"));
                        }
                    }
                }
                if checked == 0 {
                    task.log("校验：没有需要核对的对象");
                } else if mismatched == 0 {
                    task.log(format!(
                        "校验完成：{checked} 个对象全部一致（都存在，行数相等）"
                    ));
                } else {
                    task.log(format!(
                        "校验完成：{checked} 个对象中 {mismatched} 个不一致，重传后 {fixed} 个已对齐"
                    ));
                }
                verify_report = json!({
                    "checked": checked,
                    "mismatch": mismatched,
                    "fixed": fixed,
                });
            }
            let (ok, skipped, errors, inserted, updated) = *counters.lock().unwrap();
            // 终态对账：进度卡的「读取 / 传输」必须 ≥ 结果表的新增+更新合计。
            // 各条写入路径各行其是（同实例直拷、多值批、逐行 update），漏记一次就会出现
            // 「读取 0 / 传输 0，结果表里却写着 +48 行」这种自相矛盾的收尾（真机截图）。
            // 只增不减：运行中的平滑递增与真实速率不受影响，取消的任务也不改。
            if !task.is_canceled() {
                let rows = inserted + updated;
                task.set_rows_written_at_least(rows);
                task.set_rows_read_at_least(rows);
            }
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
                // 收尾校验的结果（对象是否存在 + 行数是否相等）：界面可以据此加一行
                "verify": verify_report,
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

#[cfg(test)]
mod savepoint_tests {
    use super::*;

    #[test]
    fn 事务内脏行降级使用可回滚保存点() {
        let pg = savepoint_sql("postgresql", "sp1").expect("PG 必须有保存点");
        assert_eq!(pg.0, "savepoint sp1");
        assert_eq!(pg.1, "rollback to savepoint sp1");
        assert_eq!(pg.2.as_deref(), Some("release savepoint sp1"));

        let mssql = savepoint_sql("sqlserver", "sp2").expect("SQL Server 必须有保存点");
        assert_eq!(mssql.0, "save transaction sp2");
        assert_eq!(mssql.1, "rollback transaction sp2");
        assert!(mssql.2.is_none());

        assert!(savepoint_sql("mysql", "sp3").is_none());
        assert!(savepoint_sql("oracle", "sp4").is_none());
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    /// 断连整表重跑的前提，一个都不能松（合并 / 追加模式）。
    #[test]
    fn 断连整表重跑的前提() {
        let link = "写入连接已断：连接失败：Communications link failure";
        assert!(can_retry_after_link_break(
            0, true, false, 0, false, link, false
        ));
        // 尾部偶尔连断两次是常态：前 LINK_RETRY_MAX 次都要给重跑机会
        assert!(can_retry_after_link_break(
            LINK_RETRY_MAX - 1,
            true,
            false,
            0,
            false,
            link,
            false
        ));

        // ① 次数用满
        assert!(!can_retry_after_link_break(
            LINK_RETRY_MAX,
            true,
            false,
            0,
            false,
            link,
            false
        ));
        // ② 方言没有事务批（Doris / ClickHouse / Oracle / DB2…）：已写的行还在
        assert!(!can_retry_after_link_break(
            0, false, false, 0, false, link, false
        ));
        // ③ 退回过逐条提交：行已 commit，重跑就是重复
        assert!(!can_retry_after_link_break(
            0, true, true, 0, false, link, false
        ));
        // ④ 已经有一路提交成功（提交阶段断连）：目标表已有本次数据，重跑就是重复
        assert!(!can_retry_after_link_break(
            0, true, false, 1, false, link, false
        ));
        // ⑤ 用户点了停止
        assert!(!can_retry_after_link_break(
            0, true, false, 0, true, link, false
        ));
        // ⑥ 脏数据 / 类型不匹配：重跑必然再失败
        assert!(!can_retry_after_link_break(
            0,
            true,
            false,
            0,
            false,
            "Incorrect integer value: 'abc' for column 'id' at row 1",
            false
        ));
        // ⑦ 宿主/驱动报的其它链路错误也要认出来（不止 MySQL 那一条文案）
        assert!(can_retry_after_link_break(
            0,
            true,
            false,
            0,
            false,
            "connection reset by peer",
            false
        ));
    }

    /// 清空重灌（TruncateInsert）时，重跑前会先清表 ⇒ 已提交过几路不再是障碍。
    ///
    /// 真机背景：任务跑到 85%、2.5 小时，链路抖了十几秒，几张表被判失败。
    /// 那种模式下标红毫无意义 —— 清空重来才是用户要的「自动重连」。
    #[test]
    fn 清空重灌模式允许断连后整表重跑() {
        let link = "表 xingwen_hubs.data_hour：连不上 SSH 跳板机 192.168.2.127:22";
        assert!(is_link_level_message(link), "跳板机建连失败必须算链路错误");
        // 清空重灌：即便已经提交过几路、方言没有事务批，也能清表重来
        assert!(can_retry_after_link_break(0, true, false, 3, false, link, true));
        assert!(can_retry_after_link_break(1, false, true, 3, false, link, true));
        // 但次数上限与取消仍然有效
        assert!(!can_retry_after_link_break(
            LINK_RETRY_MAX,
            true,
            false,
            0,
            false,
            link,
            true
        ));
        assert!(!can_retry_after_link_break(
            0, true, false, 0, true, link, true
        ));
        // 脏数据重跑必然再失败：清空模式也不许无限重来
        assert!(!can_retry_after_link_break(
            0,
            true,
            false,
            0,
            false,
            "Incorrect integer value: 'abc' for column 'id' at row 1",
            true
        ));
        // 网络类错误的常见变体都要能认出来
        for msg in [
            "跳板机地址无法解析：bad.host:22",
            "SSH 会话建立失败：Auth failed",
            "Connection timed out: connect",
            "java.net.ConnectException: Connection refused",
            "No route to host",
            "本地转发端口已失效（监听线程已退出），重建隧道",
        ] {
            assert!(is_link_level_message(msg), "应算链路错误：{msg}");
        }
    }

    /// 重跑前能不能清空目标表：「清空重灌」与「删除重建」都可以，源目标同表 / 追加合并都不行。
    ///
    /// 真机背景：选「删除重建」跑到中途链路抖了 25 秒，第二轮因为「不能清表 ⇒ 不许重跑」
    /// 直接判红，5 次重连预算一次都没用上。
    #[test]
    fn 清表模式决定能否整表重跑() {
        assert!(can_clear_before_retry(
            DataMode::TruncateInsert,
            Policy::Merge,
            false
        ));
        assert!(can_clear_before_retry(
            DataMode::Upsert,
            Policy::Drop,
            false
        ));
        // 源与目标是同一张表：清空会把待读的数据删掉
        assert!(!can_clear_before_retry(
            DataMode::TruncateInsert,
            Policy::Drop,
            true
        ));
        // 合并 / 追加：不能清目标表现有数据
        assert!(!can_clear_before_retry(
            DataMode::Upsert,
            Policy::Merge,
            false
        ));
        assert!(!can_clear_before_retry(
            DataMode::Insert,
            Policy::Merge,
            false
        ));
    }

    /// 界面只留宿主的事实原话，内部会话键（含主机名 / 库名 / SSH 账号）必须剥掉。
    #[test]
    fn 会话不存在的内部会话键不上界面() {
        let raw = "SQL 执行失败：(conn=1721) 会话不存在：\
                   mysql|测试-xingwei_kechuan > xingwei_kechuan|\
                   rm-uf68dg99o0c9yfdxpo mysql.cn-shanghai.rds.aliyuncs.com|3306|\
                   xingwei_kechuan|xingwei_kechuan|false|ssh-rick@192.168.2.127:22/password#3";
        assert_eq!(brief_error(raw), "会话不存在");

        // 正常句子里恰好出现「会话不存在」但没有会话键时，一个字都不动
        let plain = "表 foo 同步失败：会话不存在，请重新连接";
        assert_eq!(brief_error(plain), plain);
    }

    /// 「会话不存在」必须算链路级错误（否则整表直接判失败，事务里几万行静默消失）。
    ///
    /// 真机文案（用户截图）：`写入连接已断：会话不存在（agent 可能已重启）：…|s=internal:sync-wslot00-0#0`。
    /// 语义上它比 Communications link failure 更「干净」：会话都没了 ⇒ 这条语句一定没发出去，
    /// 且它所在事务已被服务端回滚 ⇒ 整表重跑既不重复也不缺行。
    #[test]
    fn 会话不存在算链路级可整表重跑() {
        let msg = "写入连接已断：会话不存在（agent 可能已重启）：\
                   mysql|demo|127.0.0.1|3306|u|demo|false|-|s=internal:sync-wslot00-0#0";
        assert!(is_link_level_message(msg));
        assert!(can_retry_after_link_break(
            0, true, false, 0, false, msg, false
        ));
        // 没有事务批的方言照旧不许重跑（已写的行还留在目标表里）
        assert!(!can_retry_after_link_break(
            0, false, false, 0, false, msg, false
        ));
        // 已经提交过一路 / 用户取消，同样不许
        assert!(!can_retry_after_link_break(
            0, true, false, 1, false, msg, false
        ));
        assert!(!can_retry_after_link_break(
            0, true, false, 0, true, msg, false
        ));
    }
}

#[cfg(test)]
mod prepared_batch_tests {
    use super::*;
    use dbmind_core::ConnectionKind;

    #[test]
    fn 预编译只启用能可靠回滚的方言() {
        for kind in ["sqlserver", "oracle", "dm", "h2"] {
            assert!(prepared_insert_candidate(kind), "{kind} 应启用预编译批");
        }
        // db2 的 executeBatch 大批量会让连接被掐断（实测 08001，回滚也失败），
        // 保持字面量路径 —— 见 `prepared_insert_candidate` 的注释。
        for kind in ["mysql", "mariadb", "postgresql", "clickhouse", "doris", "sqlite", "db2"] {
            assert!(!prepared_insert_candidate(kind), "{kind} 应继续走已验证的批量路径");
        }
    }

    #[test]
    fn 单条语句的字节上限按实测耗时自适应() {
        let max = insert_stmt_cap("mysql");
        // 慢（>5 秒/条）⇒ 减半；到下限就停
        assert_eq!(adapt_stmt_cap(max, 6_000, max), max / 2);
        assert_eq!(adapt_stmt_cap(STMT_CAP_MIN, 60_000, max), STMT_CAP_MIN);
        // 快（<1 秒/条）⇒ 翻倍；不超过方言 packet 上限
        assert_eq!(adapt_stmt_cap(256 * 1024, 100, max), 512 * 1024);
        assert_eq!(adapt_stmt_cap(max, 100, max), max);
        // 1~5 秒：维持不变（已在甜点区间）
        assert_eq!(adapt_stmt_cap(1024 * 1024, 3_000, max), 1024 * 1024);
    }

    /// 任务级建议值直接决定开工时的报文上限：一张表踩过的坑要被后面的表继承，
    /// 且不会被压到 STMT_CAP_MIN 以下（小报文只是多几次往返，语法上必须有下限）。
    #[test]
    fn 报文上限取方言默认与任务级建议的较小值() {
        assert_eq!(effective_stmt_cap("mysql", 0), 1024 * 1024);
        assert_eq!(effective_stmt_cap("mysql", 512 * 1024), 512 * 1024);
        // 建议值比方言默认值还大 ⇒ 仍然用方言上限（不会被放大）
        assert_eq!(effective_stmt_cap("mysql", 8 * 1024 * 1024), 1024 * 1024);
        // 建议值小到不合理 ⇒ 夹到下限
        assert_eq!(effective_stmt_cap("mysql", 10), STMT_CAP_MIN);
        // 各方言默认值不同，Doris 的 FE packet 只有 1MB
        assert_eq!(effective_stmt_cap("doris", 0), 512 * 1024);
    }

    #[test]
    fn 子批范围按行数切分且不丢行() {
        let rows = vec![vec![CellValue::Integer(1)]; 2_001];
        let ranges = insert_batch_ranges(&rows, "sqlserver");
        assert_eq!(ranges, vec![(0, 1_000), (1_000, 1_000), (2_000, 1)]);
    }

    #[test]
    fn 子批范围按字节切分宽行() {
        let wide = vec![CellValue::Text("x".repeat(9 * 1024 * 1024))];
        let rows = vec![wide.clone(), wide];
        let ranges = insert_batch_ranges(&rows, "oracle");
        assert_eq!(ranges, vec![(0, 1), (1, 1)]);
    }

    #[test]
    fn oracle_宽表按表达式总数切分子批() {
        let rows = vec![vec![CellValue::Integer(1); 130]; 1_000];
        let ranges = insert_batch_ranges(&rows, "oracle");
        assert_eq!(ranges, vec![(0, 504), (504, 496)]);
        assert!(ranges.iter().all(|(_, rows)| rows * 130 <= 65_535));
    }

    #[test]
    fn 预编译_insert_使用目标方言列名和占位符() {
        let columns = vec!["id".to_string(), "select".to_string()];
        let sql = build_prepared_insert(
            "[dbo].[t]",
            &columns,
            Dialect::new(ConnectionKind::Sqlserver),
        );
        assert_eq!(
            sql,
            "insert into [dbo].[t] (id, [select]) values (?, ?)"
        );
    }

    #[test]
    fn 跨类型字面量按目标列语义转换() {
        let pg = Dialect::new(ConnectionKind::Postgresql);
        assert_eq!(
            literal_for(&CellValue::Integer(3), pg, Some("character varying")),
            "'3'"
        );
        assert_eq!(
            literal_for(
                &CellValue::Decimal("12345678901234567890.1234567890".into()),
                pg,
                Some("numeric(38,10)")
            ),
            "12345678901234567890.1234567890"
        );
        assert_eq!(
            literal_for(&CellValue::Text("not-a-number".into()), pg, Some("numeric")),
            "'not-a-number'"
        );
        assert_eq!(
            literal_for(&CellValue::Decimal("0".into()), pg, Some("boolean")),
            "false"
        );
    }
}

#[cfg(test)]
mod definition_tests {
    use super::*;

    /// MySQL 的 `SHOW CREATE VIEW` 一定带 `DEFINER=`u`@`h``，换机器同步时那个账号不存在。
    /// 曾经只抹掉了 `u`，留下 `@`h` SQL SECURITY DEFINER VIEW …` —— 语法错（真机）。
    #[test]
    fn 去掉整套_definer() {
        let src = "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v` AS select 1";
        let out = rewrite_definition(src, None, None, true);
        assert!(!out.contains("DEFINER="), "DEFINER 子句没抹掉: {out}");
        assert!(!out.contains("@`%`"), "host 没被一起抹掉: {out}");
        assert!(out.starts_with("CREATE ALGORITHM=UNDEFINED SQL SECURITY"), "{out}");
    }

    #[test]
    fn 库名限定改写到目标库() {
        let src = "CREATE VIEW `v` AS select `id` from `src_db`.`t` join \"src_db\".\"x\" on 1=1";
        let out = rewrite_definition(src, Some("src_db"), Some("tgt_db"), true);
        assert!(out.contains("`tgt_db`.`t`"), "{out}");
        assert!(out.contains("\"tgt_db\".\"x\""), "{out}");
        assert!(!out.contains("src_db"), "{out}");
    }

    #[test]
    fn 裸库名前缀也改写但不动字面量() {
        let src = "CREATE VIEW v AS select * from src_db.t where t.name = 'src_db.t'";
        let out = rewrite_definition(src, Some("src_db"), Some("tgt_db"), true);
        assert!(out.contains("from tgt_db.t"), "{out}");
        // 字符串字面量里的内容不能被改
        assert!(out.contains("'src_db.t'"), "字面量被误改: {out}");
    }

    /// 库名相同时一个字都不动（同库同名同步是绝大多数场景）
    #[test]
    fn 同名库不改写() {
        let src = "CREATE VIEW v AS select * from demo.t";
        assert_eq!(rewrite_definition(src, Some("demo"), Some("demo"), true), src);
    }

    /// 跨类型建视图：MySQL 的 `ALGORITHM=` / `SQL SECURITY …` 目标端不认
    ///（真机 MySQL → ClickHouse 报 `Code: 62. Syntax error: failed at position 8 ('ALGORITHM')`）。
    #[test]
    fn 跨类型同步视图去掉_mysql_专有子句() {
        let mysql = Dialect::new(dbmind_core::ConnectionKind::Mysql);
        let clickhouse = Dialect::new(dbmind_core::ConnectionKind::Clickhouse);
        let raw = "CREATE ALGORITHM=UNDEFINED DEFINER=`u`@`%` SQL SECURITY INVOKER VIEW `v` AS select 1";
        let out = adapt_object_definition(&rewrite_definition(raw, None, None, true), &mysql, &clickhouse);
        assert_eq!(out, "CREATE VIEW `v` AS select 1");
        // 同类型（MySQL → MySQL）必须原样保留，别把 MySQL 自己也搞坏
        assert!(adapt_object_definition(raw, &mysql, &mysql).contains("ALGORITHM="));
        // 视图体里的同名字样不能被误伤（只在头部做替换）
        let body = "CREATE ALGORITHM=MERGE SQL SECURITY DEFINER VIEW `v` AS \
                    select 'sql security invoker' as note from t where x = 'algorithm=merge'";
        let out = adapt_object_definition(body, &mysql, &clickhouse);
        assert!(out.starts_with("CREATE VIEW `v`"), "{out}");
        assert!(out.contains("'sql security invoker'"), "字面量被误改：{out}");
        assert!(out.contains("'algorithm=merge'"), "字面量被误改：{out}");
    }

    /// 跨类型建视图/例程时，MySQL 的反引号要换成目标方言的引号。
    /// 真机：MySQL → PostgreSQL 整库同步，视图报 `ERROR: syntax error at or near "`" 位置 13`，
    /// 函数报 `syntax error at or near "" position: 17` —— 都是反引号惹的。
    #[test]
    fn 跨类型对象定义的标识符改用目标引号() {
        let mysql = Dialect::new(dbmind_core::ConnectionKind::Mysql);
        let pg = Dialect::new(dbmind_core::ConnectionKind::Postgresql);
        let sqlserver = Dialect::new(dbmind_core::ConnectionKind::Sqlserver);
        let raw = "CREATE VIEW `v_x` AS select `t`.`a` from `db1`.`t` where `t`.`b` = '`not_ident`'";
        assert_eq!(
            adapt_object_definition(raw, &mysql, &pg),
            "CREATE VIEW \"v_x\" AS select \"t\".\"a\" from \"db1\".\"t\" where \"t\".\"b\" = '`not_ident`'",
            "反引号要换双引号，且不能动字符串字面量"
        );
        // SQL Server 习惯方括号
        assert_eq!(
            adapt_object_definition("CREATE VIEW `v` AS select 1", &mysql, &sqlserver),
            "CREATE VIEW [v] AS select 1"
        );
        // 同类型（MySQL → MySQL）一个字都不动
        assert_eq!(adapt_object_definition(raw, &mysql, &mysql), raw);
        // 目标本来就用反引号（Doris / ClickHouse）也不用动
        let doris = Dialect::new(dbmind_core::ConnectionKind::Doris);
        assert_eq!(adapt_object_definition(raw, &mysql, &doris), raw);
    }
}


#[cfg(test)]
mod 用户脚本与映射测试 {
    use super::*;

    /// 前后置 SQL 的分割必须尊重引号：`'a;b'` 里的分号是**数据**，不是分隔符。
    /// 切坏的后果是「用户正常的脚本在我们这里变成语法错误」。
    #[test]
    fn 用户_sql_按引号之外的分隔符切分() {
        let script = "delete from t;\n-- 备注整行丢掉\ninsert into t values ('a;b');\nselect 1";
        let parts = split_user_sql(script);
        assert_eq!(
            parts,
            vec![
                "delete from t".to_string(),
                "insert into t values ('a;b')".to_string(),
                "select 1".to_string(),
            ]
        );
        assert!(split_user_sql("  ;; \n").is_empty(), "空语句不该发出去");
    }

    /// 表名映射：按**源表名**匹配（也认带库前缀的末段），留空/没写等于同名。
    #[test]
    fn 表名映射按源表名生效() {
        let body = SyncRequest {
            table_renames: Some(std::collections::HashMap::from([
                ("t_order".to_string(), "t_order_new".to_string()),
                ("t_blank".to_string(), "   ".to_string()),
            ])),
            ..Default::default()
        };
        let opts = body.opts();
        assert_eq!(opts.target_table_for("t_order", "t_order"), "t_order_new");
        assert_eq!(opts.target_table_for("db1.t_order", "db1.t_order"), "t_order_new");
        assert_eq!(opts.target_table_for("t_blank", "t_blank"), "t_blank");
        // 没有映射时，单表接口显式给的目标表名说了算
        assert_eq!(opts.target_table_for("t_keep", "explicit"), "explicit");
    }

    /// 前端提交的字段名必须与后端一一对上（`rename_all = "camelCase"`）。
    /// 对不上的表现是「界面填了、后端静默忽略」，在界面上完全看不出来。
    #[test]
    fn 前端载荷字段名对得上() {
        let payload = serde_json::json!({
            "sourceConnectionId": "c1",
            "targetConnectionId": "c2",
            "rateLimitRows": 5000,
            "rateLimitBytes": 2097152,
            "tableRenames": { "t_order": "t_order_new" },
            "preSql": "delete from t_order",
            "postSql": "analyze table t_order"
        });
        let body: SyncRequest = serde_json::from_value(payload).expect("载荷应能解析");
        let opts = body.opts();
        assert!(opts.pacer.is_some(), "rateLimitRows/Bytes 没被认出来");
        assert_eq!(
            opts.table_renames.get("t_order").map(String::as_str),
            Some("t_order_new"),
            "tableRenames 没被认出来"
        );
        assert_eq!(opts.pre_sql, "delete from t_order");
        assert_eq!(opts.post_sql, "analyze table t_order");
    }

    /// 限速：两条曲线都为 0 时**不建**限速表（热路径完全不介入）。
    #[test]
    fn 不限速时不建限速表() {
        let body = SyncRequest::default();
        assert!(body.opts().pacer.is_none());
        let body = SyncRequest {
            rate_limit_rows: Some(5_000),
            ..Default::default()
        };
        assert!(body.opts().pacer.is_some());
    }
}
