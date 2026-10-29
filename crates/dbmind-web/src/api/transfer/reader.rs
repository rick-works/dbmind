//! Reader 插件：把源表切成若干 range，每片按游标一页页拉。
//!
//! `SqlReader` 是**通用实现**：翻页语法（keyset / OFFSET / `limit`/`top`/`offset fetch`）
//! 与字面量（日期、二进制、转义）都由 `Dialect` 负责，所以 MySQL / PG / Oracle /
//! SQL Server / ClickHouse / Doris / DB2 / H2 / Derby / SQLite 共用这一份 —— 它们
//! 「怎么把数据取出来」的实际差别真的只在这两件事上。
//!
//! 需要完全不同的取数方式时（MongoDB 的游标批读、Kafka 的分区消费、文件的按块读），
//! 实现 [`Reader`] 再在 `transfer::reader_for` 里按连接类型返回即可，引擎不用动。

use std::sync::atomic::{AtomicUsize, Ordering};

use dbmind_core::CellValue;

use super::{BF, TablePlan};
use crate::api::dialect::Dialect;
use crate::api::meta::run_sql_in;
use crate::api::sync::{estimate_row_bytes, fetch_page, literal_for, write_parallelism};
use crate::AppState;

/// 单流初始页 **1 万行**（有键表的上限也是它）；拿到第一页后按**实际行宽 +
/// 实测写速 + 实测读耗时**自适应，但不会超过 `rows_cap`（见 `page_target_rows`）。
///
/// 为什么不从 2 万起步：一页就是一次「读一页 → 写一页 → 等它返回」的判死单位。
/// 真机上「后期每张表都卡 601 秒」的日志里，卡住的页从 2.8 万到 9.8 万行都有 ——
/// 页越大，一条慢语句把整页拖死的时间越长、重试浪费越多。1 万行一页约 10 条
/// INSERT 语句，健康链路上往返多不了几次（实测吞吐仍在 100 万行/分钟量级）。
pub const PAGE_WIDE: usize = 10_000;
/// 分片并发初始页 1 万行：多片同时在途，先控制内存，再按实际行宽自适应 ——
/// 页码是「一次往返」的成本，5 千行/页会让 20 万行变成 40 次往返，在延迟高的链路上
/// 反而比单流慢（真机实测 4 片 5 千行/页 11s vs 单流 5s）。
pub const PAGE_SPLIT: usize = 10_000;
const PAGE_BYTES_TARGET: usize = 32 * 1024 * 1024;
const PAGE_ROWS_MIN: usize = 1_000;
const PAGE_ROWS_MAX: usize = 100_000;
/// 有键表一页的硬上限：**1 万行**（一页 ≈ 10 条 INSERT 语句）。
///
/// 为什么不再允许长到 10 万：页大只在「目标端吃得下」时省往返，一旦目标端在后期
/// 掉速，10 万行一页等于把 100 条语句和它所在的事务一起挂在一次判死上 —— 真机日志
/// 里「单批 98545 / 89489 行写入 601 秒」就是这么来的。1 万行一页，中断重试的
/// 代价小得多；实测吞吐仍在 100 万行/分钟量级（往返不是瓶颈，目标端才是）。
const PAGE_ROWS_MAX_KEYED: usize = 10_000;
/// **无键表**一页的硬上限：2 万行（受 [`PAGE_BYTES_TARGET`] 再压一次，宽表到不了）。
///
/// 无键表只能 `limit n offset m` 翻页，每页都要重扫前面 offset 行 ⇒
/// 总扫描量 ≈ `N² / (2 × 页)`，**页就是吞吐本身**。曾经把它也钉在「每批条数」
/// （SQL Server 1000）上，25 万行的视图于是要扫约 3100 万行、单表 936 秒
/// （真机：VW_F_ICD_MONTHLY_ACTIVITY_REPORT_PEGASYS，269 行/秒，整库 75% 的时间
/// 花在这种地方）。页放到 1~2 万，同一张表快一个数量级。
///
/// 上界不给到 10 万的理由同有键表：一页 = 一次判死单位，页太大时一条慢语句
/// 拖死的时间与内存都成倍。
const PAGE_ROWS_MAX_UNKEYED: usize = 20_000;
/// 读一页**超时**后允许缩到的最小页。
///
/// 比 [`PAGE_ROWS_MIN`] 还低：那个下限是「自适应时别把页调得太碎」，而这里是
/// 「链路真读不回来」的逃生档 —— 一页 3.8 万行 ≈ 6 MB 结果集，在丢包链路上
/// 客户端等不到 ACK，60 秒后整表重跑（真机：`winneryun_huike_data_hour` 反复
/// 「读取一页 60 秒未返回」）。500 行 ≈ 80 KB，小到能过任何黑洞。
const PAGE_ROWS_FLOOR: usize = 500;

/// 固定页行数的逃生阀（`DBMIND_SYNC_PAGE_ROWS`）：设了就**不再自适应**，
/// 一页恒为这个行数（同时成为硬上限，不会几何放大回去）。
///
/// 用途：丢包/高延迟链路上，实测「多大页读得回来」比让算法猜更快。
fn env_page_rows() -> Option<usize> {
    std::env::var("DBMIND_SYNC_PAGE_ROWS")
        .ok()
        .and_then(|text| text.trim().parse::<usize>().ok())
        .filter(|rows| *rows >= 1)
        .map(|rows| rows.min(PAGE_ROWS_MAX))
}
/// 一页**应该写多久**（秒）：读侧按实测写速把页调到这个量级。
///
/// 为什么不按「写路数」一刀切：那样的上限（1 路写只给 2 万行）在快表上把往返次数
/// 抬了几倍，整体反而变慢（用户实报「同步变慢了」）。按写速算才是对的 —— 写得快
/// 说明目标端吃得下，页就该放大到 10 万行，少打几十次往返；写慢说明页再大也只是
/// 把行堆在队列里，缩小页反而能让进度与内存都可控。
const PAGE_TARGET_SECS: u64 = 20;
/// 一页**应该读多久**（毫秒）：页大小还受实测读取耗时约束。
///
/// 为什么：报文越大，丢一个包的重传代价越高（链路有 PMTU 黑洞时是超线性的）。
/// 写侧早就按这个道理拆小语句（见 `sync::adapt_stmt_cap`：3 MB 一条语句 113 秒，
/// 拆小后快十倍），读侧同理 —— 一页读了几十秒就说明页太大，减半比死等更划算。
/// 健康链路上几万行只要零点几秒，这条约束不会生效。
const PAGE_READ_TARGET_MS: u64 = 5_000;

/// 读页的会话键前缀（分片时后面拼片号，整表唯一后缀拼 `sid`）
const SESSION_READ: &str = "internal:sync-r";

/// **每片至少要有的行数**（5 页），低于这个量级就不分片。
///
/// 分片不是白给的：每片一条新连接（远端库的建连 + 认证在跨网链路上要几百毫秒到一两秒），
/// 页小了还要多打几次往返。真机实测（远端 MySQL 20 万行 → 本地 H2）：
/// 单流 4.8~6.8s，4 片 6.3~7.9s —— 链路是带宽瓶颈时，分片只摊到了「多建 3 条连接」的成本。
/// 大表才是分片的战场：那时建连成本被摊薄，服务端并行扫描才真正生效。
/// 门槛从 10 万降到 5 万：50 万行的表原来只能单流，现在 4 片并行（每片 12.5 万行）。
const MIN_ROWS_PER_SLICE: i64 = 50_000;

/// 分页游标：上一页停在哪。
#[derive(Clone, Debug, Default)]
pub struct Cursor {
    /// keyset 游标的键值（`paging_keys` 顺序）；无键表为 `None`
    pub key: Option<Vec<CellValue>>,
    /// 无键表用的 OFFSET
    pub offset: u64,
}

/// 一页数据。
pub struct Page {
    pub rows: Vec<Vec<CellValue>>,
    /// `None` = 这一片读完了
    pub next: Option<Cursor>,
}

/// 一片：`[low, high)` 的键值区间，两侧都可以是开区间。
#[derive(Clone, Debug, Default)]
pub struct RowRange {
    /// 片序号：每片要占一条**独立的物理会话**（会话键不同才会分到不同连接），
    /// 否则几片会挤在一条连接上排队，分片就退化成「多打几次往返」。
    pub index: usize,
    pub low: Option<CellValue>,
    pub high: Option<CellValue>,
    /// **无键表**的行窗口 `[起, 止)`（按 OFFSET 定位）。
    ///
    /// 无主键又没有可用唯一索引时，一列值都拿不到，过去只能整表单流 OFFSET 翻页：
    /// 357 万行的一张表真机跑了 40 分钟，而整库最后 12 张大表占 59% 体积，全卡在这里。
    /// 有总行数就能把表按行号切成 K 段并发拉，每段内 offset 从自己的起点开始 ——
    /// 段的区间互不重叠，合起来覆盖全表，所以不重不漏（前提是同步期间源表不变）。
    pub window: Option<(u64, u64)>,
}

impl RowRange {
    pub fn is_whole(&self) -> bool {
        self.low.is_none() && self.high.is_none() && self.window.is_none()
    }
}

/// 取数插件。
pub trait Reader: Send + Sync {
    /// 插件名（进日志/诊断）
    fn name(&self) -> &'static str;
    /// 当前一页的行数
    fn page_rows(&self) -> usize;
    /// 一个事务最多写多少行（默认 = 当前页大小）。
    ///
    /// **一批 = 一页 = 一个事务**：引擎按这个值决定多久提交一次（`engine::txn_rows_cap`），
    /// `SqlReader` 返回本表的页行数上限 —— 有键表是界面「每批条数」（0 = 目标方言默认），
    /// 无键表是 OFFSET 翻页的大页。这样日志里「一页 N 条」「一条 INSERT」和用户在界面上
    /// 填的「每批条数」是同一个数，不必再记第二套参数。
    fn commit_rows(&self) -> u64 {
        self.page_rows() as u64
    }
    /// 这个源最多能并发开几片（由连接类型声明决定）
    fn max_parallel(&self) -> usize;
    /// 并发片数的**上限**：尾部弹性扩片时最多能切/开到几片。
    ///
    /// 默认等于 [`Reader::max_parallel`]（不扩）；`SqlReader` 返回方言声明的读并发，
    /// 引擎按它决定「细块要切多细」「令牌最多补到几片」。
    fn parallel_ceiling(&self) -> usize {
        self.max_parallel()
    }
    /// 切分。返回 1 片 = 不分片（无键 / 有行数上限 / 单连接池的库）。
    fn split<'a>(&'a mut self, want: usize) -> BF<'a, Result<Vec<RowRange>, String>>;
    /// 拉一片的下一页
    fn page<'a>(&'a self, range: &'a RowRange, cursor: Option<Cursor>) -> BF<'a, Result<Page, String>>;
    /// 读一页失败后收缩页大小（默认什么都不做）。
    ///
    /// 引擎在读超时/链路报错时调用：页太大 ⇒ 结果集太大 ⇒ 丢包链路上读不回来。
    /// 缩半页再试，比「按原页大重试三次、然后整表重跑」划算得多。
    fn shrink_page(&self) {}
}

/// 通用 SQL 取数插件。
pub struct SqlReader {
    state: AppState,
    conn: String,
    scope: String,
    /// `select 列 from 表`（不含 where）
    base: String,
    /// 表表达式（分片探测 min/max/count 用）
    from: String,
    dialect: Dialect,
    paging_keys: Vec<String>,
    key_positions: Vec<usize>,
    where_clause: String,
    row_limit: u64,
    page_rows: AtomicUsize,
    /// `DBMIND_SYNC_PAGE_ROWS` 给定的固定页；有值时分片切换不再改写页大小。
    fixed_page: Option<usize>,
    /// 一页的行数上限（硬上限，见 [`PAGE_ROWS_MAX`]）。读失败收缩时会一起压低，
    /// 免得几何增长/写速自适应马上又把它放大回去（同一段链路反复超时就成了震荡）。
    rows_cap: AtomicUsize,
    /// 一个事务最多写多少行（[`Reader::commit_rows`]）。
    /// 无键表用它把「读页」和「事务粒度」解耦：页是 1~2 万行（省 OFFSET 重扫），
    /// 事务仍是界面的「每批条数」（1000 行），不会变成大事务。
    commit_rows: usize,
    /// 写池诊断：读侧按**实测写速**决定一页多大（见 [`PAGE_TARGET_SECS`]）
    diag: std::sync::Arc<crate::api::transfer::WriteDiag>,
    /// 本表的会话后缀（见 `TablePlan::sid`）：并发整库同步时，不同表的同号分片
    /// 若用同一个会话键就会落到同一条物理连接上，分片退化成排队
    sid: String,
    /// 本表最多几片并发读（0 = 方言默认），见 `TablePlan::slices_cap`
    slices_cap: usize,
    /// 尾部弹性扩片的**上限**（方言声明的读并发）：见 [`Reader::parallel_ceiling`]
    parallel_ceiling: usize,
}

/// 分页键在**取数列**里的位置（按 `columns` 的顺序做大小写不敏感匹配）。
///
/// 任一键找不到就返回 `None`：调用方要退回 OFFSET 分页 —— 少一个位置的 keyset 游标
/// 会变成「按 1 列比较 2 列键」，同样是静默丢行。
fn paging_positions(columns: &[String], paging_keys: &[String]) -> Option<Vec<usize>> {
    paging_keys
        .iter()
        .map(|key| columns.iter().position(|column| column.eq_ignore_ascii_case(key)))
        .collect()
}

impl SqlReader {
    pub fn new(plan: &TablePlan) -> Box<dyn Reader> {
        let columns = plan
            .columns
            .iter()
            .map(|name| plan.source_kind.quote(name))
            .collect::<Vec<_>>()
            .join(", ");
        // 页大小不再按「写并发」一刀切：那个做法（1 路写就给 2 万行）在快表上把
        // 往返次数抬了几倍，整体反而变慢。现在交给**实测写速**：`read_page` 里按
        // 「一页 ≈ PAGE_TARGET_SECS 秒的写入量」调，快则放大到 10 万行、慢则缩到几千行。
        // 逃生阀：`DBMIND_SYNC_PAGE_ROWS` 设了就固定页大小（丢包链路上直接给定值，
        // 不必等算法在超时里摸索）。
        let fixed_page = env_page_rows();
        // 分页键的**行内位置**必须按 `paging_keys` 自己在取数列里找，不能借
        // `plan.key_indexes` —— 那个是**目标端** upsert 键（写库用）的位置。
        // 目标表已存在且主键与源主键不是同一组列时，两套位置会错位，代价不是报错而是
        // **静默丢行**：（源主键 `id` 的值位置取自目标主键 `code`）⇒ keyset 拿 A 列的值
        // 去比较 B 列，跳页跳得毫无察觉。
        let (paging_keys, key_positions) = match paging_positions(&plan.columns, &plan.paging_keys) {
            Some(positions) => (plan.paging_keys.clone(), positions),
            // 分页键不在取数列里：宁可退回 OFFSET（慢但不错）也不能用错列比较
            None => {
                tracing::warn!(
                    table = %plan.table,
                    "分页键不在同步列里，退回 OFFSET 分页"
                );
                (Vec::new(), Vec::new())
            }
        };
        // **有键表**：一批 = 一页 = 一个事务，页的硬上限就是界面「每批条数」
        // （0 = 按目标方言默认：MySQL 1000 / ClickHouse 1 万 / 文件库 5000），夹在 [1000, 1 万]。
        // 用户只需理解一个词：日志里「写入 N 行」的 N 就是他填的「每批条数」。
        //
        // **无键表必须和「每批条数」脱钩**：它只能 `limit n offset m` 翻页，每页重扫
        // 前面 offset 行 ⇒ 总扫描量 ≈ N²/(2·页)，页就是吞吐本身。曾经按每批条数把它
        // 压到 1000 行，25 万行的视图要扫约 3100 万行、单表 15 分钟（真机整库同步里
        // 75% 的时间耗在这）。页放到 [`PAGE_ROWS_MAX_UNKEYED`]，事务粒度仍按每批条数，
        // 见 `commit_rows` —— 不再有「一页 1 万行写 31 秒」那种与配置对不上的日志。
        // 行窗口分片（`DBMIND_SYNC_WINDOW` / 见 `window` 那套）与 OFFSET 续跑仍然有效。
        let batch_rows = if plan.insert_rows > 0 {
            plan.insert_rows
        } else {
            crate::api::sync::insert_rows_default(plan.target_kind.kind.key())
        };
        let keyless = paging_keys.is_empty();
        let page_cap = if keyless {
            batch_rows.clamp(PAGE_ROWS_MIN, PAGE_ROWS_MAX_UNKEYED)
        } else {
            batch_rows.clamp(PAGE_ROWS_MIN, PAGE_ROWS_MAX_KEYED)
        };
        let page_hint = plan.grow.as_ref().map(|grow| grow.page_cap_hint()).unwrap_or(0);
        // 任务级「踩坑即收敛」只对有键表生效：把无键表的页压到 1 万以下，等于让
        // OFFSET 重扫成倍放大 —— 那正是要修的病，宁可不收这个提示。
        let rows_cap = fixed_page.unwrap_or(if page_hint > 0 && !keyless {
            page_cap.min(page_hint)
        } else {
            page_cap
        });
        // 事务粒度：无键表 = 界面的「每批条数」（不会被大页带成大事务），有键表 = 页
        let commit_rows = if keyless { batch_rows.max(1) } else { rows_cap };
        // 尾部弹性扩片的上限：方言声明的读并发（单连接池的库 = 1，多开只会在池里排队）。
        // 调优逃生阀优先；下限是本次分到的份额，避免「cap 比声明还大」时反而被压小。
        let parallel_ceiling = std::env::var("DBMIND_SYNC_SLICES")
            .ok()
            .and_then(|text| text.trim().parse::<usize>().ok())
            .filter(|n| *n >= 1)
            .unwrap_or_else(|| write_parallelism(plan.source_kind.kind).max(1))
            .max(plan.slices_cap)
            .max(1);
        Box::new(Self {
            state: plan.state.clone(),
            conn: plan.src_conn.clone(),
            scope: plan.src_scope.clone(),
            base: format!("select {columns} from {}", plan.src_from),
            from: plan.src_from.clone(),
            dialect: plan.source_kind,
            paging_keys,
            key_positions,
            where_clause: plan.where_clause.clone(),
            row_limit: plan.row_limit,
            page_rows: AtomicUsize::new(fixed_page.unwrap_or(PAGE_WIDE.min(rows_cap))),
            fixed_page,
            rows_cap: AtomicUsize::new(rows_cap),
            commit_rows,
            diag: plan.diag.clone(),
            sid: plan.sid.clone(),
            slices_cap: plan.slices_cap,
            parallel_ceiling,
        })
    }

    fn page_rows_now(&self) -> usize {
        self.page_rows.load(Ordering::SeqCst)
    }

    /// 每片至少要摊到多少行才值得切。
    ///
    /// 分片的成本是「一条新物理连接 + 认证」（跨网几百毫秒到两秒），收益是**读和写都能
    /// 并行**。原来固定 5 万行：真机 20 万行的表按 6 片切，每片 3.3 万行 < 5 万 ⇒
    /// 干脆不分片 ⇒ 整表只剩一条写连接，而这条链路单连接上行 340 KB/s、8 条能到
    /// 2.1 MB/s（同一张表 239 秒 vs 十几秒）。所以门槛改成「每片至少 5 页，且不少于
    /// 5000 行」，上限仍是 5 万 —— 大表才值得切，几十张几千行的小表不该各开一堆连接。
    fn min_rows_per_slice(&self) -> i64 {
        (self.page_rows_now() as i64)
            .saturating_mul(5)
            .clamp(5_000, MIN_ROWS_PER_SLICE)
    }

    fn filter_sql(&self) -> String {
        let trimmed = self.where_clause.trim();
        if trimmed.is_empty() {
            String::new()
        } else {
            format!(" where {trimmed}")
        }
    }

    /// 取一行里的键值（游标用）
    fn key_of(&self, row: &[CellValue]) -> Option<Vec<CellValue>> {
        if self.paging_keys.is_empty() {
            return None;
        }
        Some(
            self.key_positions
                .iter()
                .filter_map(|index| row.get(*index).cloned())
                .collect(),
        )
    }

    /// `min/max` 一次拿到范围：整数/浮点键按值域等分，最省（不用数行、不用扫表）。
    async fn numeric_ranges(&self, want: usize) -> Option<Vec<RowRange>> {
        let key = self.dialect.quote(&self.paging_keys[0]);
        let probe = format!(
            "select min({key}) as mn, max({key}) as mx from {}{}",
            self.from,
            self.filter_sql()
        );
        // 极值查询很轻（有索引直接走两端），失败就退回单流
        let result = run_sql_in(&self.state, &self.conn, &self.scope, probe, 1).await.ok()?;
        let row = result.rows.first()?;
        if row.len() < 2 {
            return None;
        }
        match (&row[0], &row[1]) {
            (CellValue::Integer(lo), CellValue::Integer(hi)) => {
                split_integer(*lo, *hi, want, self.min_rows_per_slice())
            }
            (CellValue::Real(lo), CellValue::Real(hi)) => {
                split_real(*lo, *hi, want, self.min_rows_per_slice())
            }
            _ => None,
        }
    }

    /// 文本/日期/二进制键：没有值域可算，只能「数行 + 按占比采样切点」。
    ///
    /// 采样代价是 `want-1` 次带索引的 `offset` 定位（不是全表扫），对几十万行以上的表
    /// 完全可以接受；行数少（< want*2）时直接不分片。
    async fn sampled_ranges(&self, want: usize) -> Option<Vec<RowRange>> {
        let key = self.dialect.quote(&self.paging_keys[0]);
        let count_sql = format!("select count(*) as n from {}{}", self.from, self.filter_sql());
        let result = run_sql_in(&self.state, &self.conn, &self.scope, count_sql, 1).await.ok()?;
        let total = match result.rows.first()?.first()? {
            CellValue::Integer(n) => *n,
            CellValue::Real(n) => *n as i64,
            CellValue::Text(text) => text.trim().parse::<i64>().ok()?,
            _ => return None,
        };
        if total < (want as i64) * self.min_rows_per_slice() {
            return None;
        }
        let mut bounds: Vec<CellValue> = Vec::with_capacity(want - 1);
        for index in 1..want {
            let offset = total * index as i64 / want as i64;
            let sql = format!(
                "select {key} as k from {}{} order by {key} {}",
                self.from,
                self.filter_sql(),
                self.dialect.limit_clause(offset as u64, 1)
            );
            let probe = run_sql_in(&self.state, &self.conn, &self.scope, sql, 1).await.ok()?;
            let value = probe.rows.first()?.first()?.clone();
            if matches!(value, CellValue::Null) {
                return None;
            }
            bounds.push(value);
        }
        let mut out = Vec::with_capacity(want);
        for index in 0..want {
            let low = if index == 0 { None } else { Some(bounds[index - 1].clone()) };
            let high = if index + 1 == want {
                None
            } else {
                Some(bounds[index].clone())
            };
            out.push(RowRange { index, low, high, window: None });
        }
        Some(out)
    }
}

impl Reader for SqlReader {
    fn name(&self) -> &'static str {
        "sql-reader"
    }

    fn page_rows(&self) -> usize {
        self.page_rows_now()
    }

    fn commit_rows(&self) -> u64 {
        // 用建表时定下的值而不是「当前页」：页会按实测写速自适应，但事务粒度不该跟着抖。
        // 有键表 = 页硬上限（= 界面「每批条数」）；无键表 = 每批条数本身
        //（页是大页，只用来省 OFFSET 重扫，见 `SqlReader::new`）。
        self.commit_rows as u64
    }

    fn max_parallel(&self) -> usize {
        // 排查用的逃生阀：某些源库在并发读下会打架（锁/游标/权限），设了这个环境变量
        // 就退回单流，不必改代码重编。
        if std::env::var_os("DBMIND_SYNC_NO_SPLIT").is_some() {
            return 1;
        }
        // 调优逃生阀：坏链路单条流吞吐有上限，多开几片并发读能堆总吞吐。
        if let Some(n) = std::env::var("DBMIND_SYNC_SLICES")
            .ok()
            .and_then(|text| text.trim().parse::<usize>().ok())
            .filter(|n| *n >= 1)
        {
            return if self.slices_cap > 0 { n.min(self.slices_cap).max(1) } else { n };
        }
        // 源端能不能并行取数，取决于**源库**声明的连接数：单连接池的库（SQLite/H2/
        // Derby/SQL Server）多开只会在池子里排队，反而多一层上下文切换。
        let base = write_parallelism(self.dialect.kind);
        // 任务级预算（见 `TablePlan::slices_cap`）：并发同步多张表时每张表少开几片，
        // 整次任务的源端读连接数也不会超。
        if self.slices_cap > 0 {
            base.min(self.slices_cap).max(1)
        } else {
            base
        }
    }

    fn parallel_ceiling(&self) -> usize {
        // 逃生阀（不并发）在这里同样生效：DBMIND_SYNC_NO_SPLIT 时不扩容
        if std::env::var_os("DBMIND_SYNC_NO_SPLIT").is_some() {
            return 1;
        }
        self.parallel_ceiling.max(self.max_parallel())
    }

    fn split<'a>(&'a mut self, want: usize) -> BF<'a, Result<Vec<RowRange>, String>> {
        Box::pin(async move {
            let whole = vec![RowRange::default()];
            // 不分片的两种情况，理由各不相同：
            // - 无键：只能 OFFSET 翻页，切了也是每片从头扫；
            // - 有行数上限：全局配额要按顺序消费，切了就不知道谁先谁后。
            //
            // **复合主键照切**：分片只按**首列**取值域区间 `k1 >= lo and k1 < hi`（区间互不
            // 重叠、合起来覆盖全表），页内游标仍然用**完整键**做 keyset —— 两者 AND 在一起，
            // 一条行只会落在一个分片里。以前这里要求「恰好一列键」，于是复合主键的大表
            // 永远单流读（几十万行的表只有一条连接在拉）。
            // 无键表（无主键、也没有可用唯一索引）：按行窗口切段并发拉。
            // 由 `DBMIND_SYNC_WINDOW=1` 打开 —— 它改变的是分页语义（段间靠 OFFSET 边界
            // 而不是键值边界隔离），先在整库同步上验证过再考虑默认开。
            if want > 1
                && self.paging_keys.is_empty()
                && self.row_limit == 0
                && self.where_clause.trim().is_empty()
                && window_split_enabled()
            {
                if let Some(ranges) = window_ranges_of(self, want).await {
                    self.page_rows
                        .store(self.fixed_page.unwrap_or(PAGE_SPLIT), Ordering::SeqCst);
                    return Ok(ranges);
                }
            }
            if want <= 1 || self.paging_keys.is_empty() || self.row_limit > 0 {
                self.page_rows
                    .store(self.fixed_page.unwrap_or(PAGE_WIDE), Ordering::SeqCst);
                return Ok(whole);
            }
            let ranges = match self.numeric_ranges(want).await {
                Some(ranges) => Some(ranges),
                None => self.sampled_ranges(want).await,
            };
            match ranges {
                Some(ranges) if ranges.len() > 1 => {
                    // 分片时把页调小：N 片同时在途，页大了内存与延迟都会变差
                    self.page_rows.store(PAGE_SPLIT, Ordering::SeqCst);
                    Ok(ranges)
                }
                _ => {
                    self.page_rows.store(PAGE_WIDE, Ordering::SeqCst);
                    Ok(whole)
                }
            }
        })
    }

    fn page<'a>(&'a self, range: &'a RowRange, cursor: Option<Cursor>) -> BF<'a, Result<Page, String>> {
        Box::pin(async move {
            // 窗口分片的段：首屏从本段起点开始（`cursor.offset` 后续由每页自己递进）
            let (key, offset) = match cursor {
                Some(cursor) => (cursor.key, cursor.offset),
                None => (None, range.window.map(|(begin, _)| begin).unwrap_or(0)),
            };
            // 三件互不相干的条件，各自判断有没有：用户过滤 / 分片区间 / keyset 游标。
            // 曾经把「用户没写过滤」当成「没有条件」整个丢掉，连游标一起扔了 ——
            // 第二页把第一页又读一遍（详见 sync::page_sql 的注释）。
            let mut parts: Vec<String> = Vec::new();
            let user_where = self.where_clause.trim();
            if !user_where.is_empty() {
                parts.push(format!("({user_where})"));
            }
            // `paging_keys` 为空 = 无键表（走 OFFSET）。它也可能不是整表：窗口分片给的是
            // `window = Some(起点, 终点)`，`low`/`high` 都是 None。原来这里只看 `is_whole()`，
            // 于是无键表 + 窗口分片会在 `paging_keys[0]` 上越界 panic
            //（真机：`index out of bounds: the len is 0 but the index is 0`，
            // 协程整个死掉，任务报「传输协程异常终止」）。没有键就没有列区间条件，跳过即可。
            if !range.is_whole() && !self.paging_keys.is_empty() {
                let column = self.dialect.quote(&self.paging_keys[0]);
                if let Some(low) = range.low.as_ref() {
                    parts.push(format!("{column} >= {}", literal_for(low, self.dialect, None)));
                }
                if let Some(high) = range.high.as_ref() {
                    parts.push(format!("{column} < {}", literal_for(high, self.dialect, None)));
                }
            }
            let extra = parts.join(" and ");
            let cursor_arg = match (key.as_ref(), self.paging_keys.is_empty()) {
                (Some(values), false) => Some((self.paging_keys.as_slice(), values.as_slice())),
                _ => None,
            };
            let page_rows = self.page_rows_now();
            // 窗口分片：本页不能越过本段的行号上界（越界就是读到了下一段的数据）
            let mut fetch_rows = page_rows;
            if let Some((_, end)) = range.window {
                let remaining = end.saturating_sub(offset).max(1) as usize;
                fetch_rows = fetch_rows.min(remaining);
            }
            // 每片一条独立会话键：同一把键会被会话池归到同一条泳道（=同一条物理连接），
            // 四片并发就变成排队 —— 这是分片「看起来开了、实际没并发」的经典坑。
            // `sid` 同样必须带上：只按片号分键的话，**两张表**的同号分片会共用一条连接。
            let session = if range.is_whole() {
                format!("{SESSION_READ}-{}", self.sid)
            } else {
                format!("{SESSION_READ}-{}-{}", self.sid, range.index)
            };
            let read_started = std::time::Instant::now();
            let rows = fetch_page(
                &self.state,
                &self.conn,
                &self.scope,
                &self.base,
                self.dialect,
                offset,
                cursor_arg,
                // 首页没有游标，但**排序键必须在**：不排序 → 「最后一行」不是最大值 →
                // 下一页的 keyset 从错误位置开始（漏行 + 主键冲突）。见 `page_sql`。
                self.paging_keys.as_slice(),
                &extra,
                fetch_rows as u64,
                &session,
            )
            .await
            .map_err(|err| err.message)?;
            let read_ms = read_started.elapsed().as_millis().max(1) as u64;

            // 下一页多大：见 `page_target_rows`
            let estimated_bytes = rows.iter().map(|row| estimate_row_bytes(row)).sum::<usize>();
            let target_rows = page_target_rows(
                page_rows,
                estimated_bytes,
                read_ms,
                self.diag.write_rate(),
                self.rows_cap.load(Ordering::SeqCst),
                // 无键表只能 OFFSET 翻页，页小了重扫是 O(N²)，不按写速压
                !self.paging_keys.is_empty(),
            );
            // 小变化不折腾分页；只有明显更宽/更窄/写速换档时才动。
            // 固定页模式（`DBMIND_SYNC_PAGE_ROWS`）下不动：用户给的值就是最终值
            //（否则逃生阀会被自适应悄悄改回去，等于没设）。
            if self.fixed_page.is_none()
                && (target_rows * 2 < page_rows || target_rows > page_rows * 2)
            {
                self.page_rows.store(target_rows, Ordering::SeqCst);
            }
            // 固定页模式（`DBMIND_SYNC_PAGE_ROWS`）：用户给的值既是目标也是**恢复上限**。
            // `shrink_page` 在读/写失败时会把它压到 1/4 保命，而上面这条自适应与本函数
            // 后面的几何增长都被 `fixed_page` 挡住了 ⇒ 不主动爬回去的话，一次抖动就让
            // 「固定页」永久变成 1/4，逃生阀等于失效。每次读成功回升一级（×2），两级到顶。
            if let Some(fixed) = self.fixed_page {
                let now = self.page_rows_now();
                if now < fixed {
                    self.page_rows.store(recover_target(now, fixed), Ordering::SeqCst);
                    // 硬上限（= 事务/批次粒度）同步涨回用户给的值
                    self.rows_cap.store(fixed, Ordering::SeqCst);
                }
            }

            // 无键表只能靠 OFFSET 翻页：`limit n offset m` 每次都要重扫前面 m 行，
            // 固定页大小时总扫描量 ≈ N²/(2·page)（20 万行 2 万/页 = 10 次往返，
            // 累计重扫 90 万行；百万行级直接变成分钟级）。让页**几何增长**（×2）后，
            // offset 累计只到约 2N 就结束，总扫描量回到 O(N)，无主键大表从「越跑越慢」
            // 变成匀速。宽表不翻倍：上面的字节目标已经把它压小了（now < page_rows）。
            // 固定页模式（`DBMIND_SYNC_PAGE_ROWS`）下不几何增长：用户明确要求这个大小。
            if self.paging_keys.is_empty() && self.fixed_page.is_none() {
                let now = self.page_rows_now();
                if now >= page_rows {
                    let grown = now
                        .saturating_mul(2)
                        .min(self.rows_cap.load(Ordering::SeqCst))
                        .min(target_rows.max(now));
                    if grown > now {
                        self.page_rows.store(grown, Ordering::SeqCst);
                    }
                }
            }

            // 窗口段读到自己上界就算这一片结束（末页可能刚好填满，不能用「行数不足」判）
            let window_done = range
                .window
                .map(|(_, end)| offset + rows.len() as u64 >= end)
                .unwrap_or(false);
            let done = rows.len() < fetch_rows || window_done;
            // 诊断：每页实际拿回多少行。`done` 靠「拿回的行数 < 请求行数」判断读完，
            // 一旦服务端/内核在某一层少给，这里就会被误判成「块读完了」。
            tracing::debug!(
                table = %self.sid,
                offset,
                fetch_rows,
                got = rows.len(),
                window_done,
                done,
                "同步读页"
            );
            let next = if done {
                None
            } else {
                Some(Cursor {
                    key: rows.last().and_then(|row| self.key_of(row)),
                    // 推进量用**本页实际行数**，不是请求的 `page_rows`：读满一页时两者相等
                    //（`done == false` 蕴含 `rows.len() == fetch_rows`），但读侧一旦多一层
                    // 截断（驱动行上限、窗口再 clamp、内核少给一行），按请求值推进会直接
                    // 跳过没读到的那些行 —— 症状是「行数少一点点」，几乎查不出来。
                    offset: offset + rows.len() as u64,
                })
            };
            Ok(Page { rows, next })
        })
    }

    fn shrink_page(&self) {
        // 读超时/链路报错后把页**减半**：页太大 ⇒ 结果集太大 ⇒ 丢包链路上客户端等不到 ACK，
        // 60 秒后整表重跑。缩半页重试比「按原页大重试三次再重跑整表」划算得多。
        //
        // 硬上限一起压到 `shrunk × 2` 而不是直接钉死：链路恢复后还能靠几何增长/自适应
        // 涨回来（最多两级翻倍），不至于因为一次抖动就整表单流小页跑到底。
        // 除以 4 而不是减半：超时说明「这一页根本传不回来」，减半往往还在同一个量级上
        // 继续撞墙（真机 re_sale：页 3711、每页等满 60 秒，连撞两次后整表 2346 秒只前进
        // 几万行）。上限一起压到 2 倍以内，别让它几何增长又爬回超时区间；链路恢复后由
        // `by_read` / `by_rate` 两条约束慢慢涨回去。
        //
        // 固定页模式（`DBMIND_SYNC_PAGE_ROWS`）没有那两条约束（自适应与几何增长都被
        // `fixed_page` 挡住），所以这里只许往下缩、并把用户给的值当**恢复上限**：
        // 缩过之后由 `page()` 里的 `recover_target` 每读成功一页回升一级，涨到 `fixed_page` 为止。
        // 过去不区分固定页，一次抖动就把「固定页」永久钉在 1/4 —— 逃生阀等于失效。
        let now = self.page_rows.load(Ordering::SeqCst);
        let (shrunk, cap) = shrink_target(now, self.fixed_page);
        if shrunk < now {
            self.page_rows.store(shrunk, Ordering::SeqCst);
            self.rows_cap.store(cap, Ordering::SeqCst);
        }
    }
}

/// 无键表按「行窗口」切段并发读（默认开启，`DBMIND_SYNC_WINDOW=0` 关闭）。
///
/// 段与段之间靠 OFFSET 边界隔离（不靠键值），所以要求：源表在同步期间不变、无 WHERE 过滤、
/// 没有行数上限。它解决的是「没有主键也没有唯一索引」的表只能单流深翻页的问题 ——
/// 真机上这部分表占整库一半以上体积，是整库「越跑越慢」的主因。
///
/// **默认开**（`DBMIND_SYNC_WINDOW=0` 关闭）。
///
/// 曾经被当成「整库 81/351 张表失败」的嫌疑人（怀疑每张无键表多切 8 段把连接数压垮），
/// 查日志后排除：46 条 `Communications link failure` 全部压在 18:56:06~19:00:14 这 4 分钟
/// 里，之后一条都没有，同期源端 SSH 与目标库端口都正常 —— 是**客户端 VPN 短时中断**。
/// 分片本身在整库 351 张表上跑通过（失败 0）。
///
/// 目标库连接数确实紧张时用 `DBMIND_SYNC_WINDOW=0` 关掉；段内续跑
/// （`SegmentOutcome::PausedAt`）与这个开关无关，关掉后无键表仍是单流 OFFSET，
/// 但已能按事务分段提交、断连只重跑当前这小段。
fn window_split_enabled() -> bool {
    std::env::var("DBMIND_SYNC_WINDOW")
        .map(|value| {
            !(value.trim() == "0"
                || value.eq_ignore_ascii_case("false")
                || value.eq_ignore_ascii_case("off"))
        })
        .unwrap_or(true)
}

/// 无键表的窗口切分：先 `count(*)` 拿总数，再按行号等分成 K 段。
///
/// 分片门槛与有键表同一口径（[`SqlReader::min_rows_per_slice`]），否则建连成本盖过
/// 并行收益。拿不到总数（无权限/统计失败）就返回 `None`，退回整表单流。
async fn window_ranges_of(reader: &SqlReader, want: usize) -> Option<Vec<RowRange>> {
    let total = count_rows_of(reader).await?;
    let slices = want.min(reader.parallel_ceiling()).max(1);
    if total < (reader.min_rows_per_slice() as u64).saturating_mul(slices as u64) {
        return None;
    }
    let bounds = window_bounds(total, slices);
    if bounds.len() <= 1 {
        return None;
    }
    Some(
        bounds
            .into_iter()
            .enumerate()
            .map(|(index, (begin, end))| RowRange {
                index,
                low: None,
                high: None,
                window: Some((begin, end)),
            })
            .collect(),
    )
}

/// 把 `[0, total)` 按行号等分成至多 `slices` 段，返回左闭右开的边界。
///
/// 单独拎出来是为了能被单测钉死：切错的表现是「某几段数据压根没搬」或「同一行搬两遍」，
/// 在整库同步里几乎看不出来（行数只是对不上一点点）。
fn window_bounds(total: u64, slices: usize) -> Vec<(u64, u64)> {
    let slices = slices.max(1);
    if total == 0 {
        return Vec::new();
    }
    let per = total.div_ceil(slices as u64);
    let mut out = Vec::new();
    for index in 0..slices as u64 {
        let begin = index * per;
        if begin >= total {
            break;
        }
        out.push((begin, ((index + 1) * per).min(total)));
    }
    out
}

/// 源表总行数（窗口切分用）。估算不够准 —— 段的边界必须精确，否则会漏行/重行。
async fn count_rows_of(reader: &SqlReader) -> Option<u64> {
    let sql = format!("select count(*) as n from {}", reader.from);
    let result = run_sql_in(&reader.state, &reader.conn, &reader.scope, sql, 5000)
        .await
        .ok()?;
    let value = result.rows.first().and_then(|row| row.first())?;
    Some(match value {
        CellValue::Integer(n) => (*n).max(0) as u64,
        CellValue::Real(n) => n.max(0.0) as u64,
        CellValue::Text(text) => text.trim().parse().ok()?,
        _ => return None,
    })
}

/// 下一页的行数：行宽、写速、读取耗时三个目标取**小者**。
///
/// - 行宽：一页不超过 [`PAGE_BYTES_TARGET`] 字节（窄表自然升到 `cap`，宽表降到 [`PAGE_ROWS_MIN`]）；
/// - 写速：一页 ≈ [`PAGE_TARGET_SECS`] 秒的写入量（写快 → 页大 → 少往返；写慢 → 页小不堆积）。
///   写速样本要等第一批写完才有，`rate = 0` 时只看行宽与读耗时；`rate_known = false` 时完全不看写速
///   （无键表：OFFSET 翻页页小了重扫是 O(N²)，宁可多占点内存）；
/// - 读耗时：一页别超过 [`PAGE_READ_TARGET_MS`] 的读取量（丢包链路上大报文重传代价超线性，
///   读一页要几十秒时，页再大只是把痛苦摊平 —— 拆小反而更快，同 `sync::adapt_stmt_cap`）。
fn page_target_rows(
    page_rows: usize,
    estimated_bytes: usize,
    read_ms: u64,
    rate: u64,
    cap: usize,
    rate_known: bool,
) -> usize {
    let by_bytes = if estimated_bytes == 0 {
        page_rows
    } else {
        ((page_rows as u128 * PAGE_BYTES_TARGET as u128) / estimated_bytes as u128)
            .clamp(PAGE_ROWS_MIN as u128, cap as u128) as usize
    };
    let by_read = ((page_rows as u128 * PAGE_READ_TARGET_MS as u128) / read_ms.max(1) as u128)
        .clamp(PAGE_ROWS_MIN as u128, cap as u128) as usize;
    let mut target = by_bytes.min(by_read);
    if rate > 0 && rate_known {
        let by_rate = rate
            .saturating_mul(PAGE_TARGET_SECS)
            .clamp(PAGE_ROWS_MIN as u64, cap as u64) as usize;
        target = target.min(by_rate);
    }
    target
}

/// 读/写失败后的「新页大小 + 新硬上限」。
///
/// 抽成纯函数是为了能被单测钉住：这里曾经不看 `fixed_page`，把用户
/// `DBMIND_SYNC_PAGE_ROWS` 给的固定页一路压到 1/4 且**涨不回来**
/// （固定页模式下自适应与几何增长都被挡住），逃生阀一次抖动就失效。
///
/// 固定页模式下 `fixed_page` 是**恢复上限**：本次缩到 `shrunk`，硬上限缩到
/// `shrunk × 2`（不钉死，留给链路恢复后爬升），但永远不超过用户给的值。
fn shrink_target(now: usize, fixed: Option<usize>) -> (usize, usize) {
    let shrunk = (now / 4).max(PAGE_ROWS_FLOOR).min(now);
    let grown = shrunk.saturating_mul(2).max(PAGE_ROWS_FLOOR);
    (shrunk, fixed.map(|fixed| grown.min(fixed)).unwrap_or(grown))
}

/// 固定页模式下每读成功一页的回升量（×2，封顶在用户给的值）。
///
/// 只回升一级而不是直接跳到 `fixed_page`：刚撞过超时的链路往往还处在同一个量级上，
/// 一步跳回去等于再撞一次；两级（1/4 → 1/2 → 1）就到顶，代价可以忽略。
fn recover_target(now: usize, fixed: usize) -> usize {
    now.saturating_mul(2).min(fixed)
}

/// 整数键按值域等分：切出来的区间**连续、不重不漏**（相邻两片的边界严格相接）。
///
/// 纯函数单独拎出来是为了能被单测钉死 —— 分片切错的表现是「某几段数据压根没搬」
/// 或者「同一行搬了两遍」，而这两种在整库同步里都很难一眼看出来。
fn split_integer(lo: i64, hi: i64, want: usize, min_rows: i64) -> Option<Vec<RowRange>> {
    if want < 2 || hi <= lo {
        return None;
    }
    let span = (hi as i128) - (lo as i128) + 1;
    // 每片摊不到 `min_rows` 行就别切：建连成本会吃掉并行收益（见 `min_rows_per_slice`）
    if span < (want as i128) * (min_rows as i128) {
        return None;
    }
    let mut out = Vec::with_capacity(want);
    for index in 0..want {
        let low = if index == 0 {
            None
        } else {
            Some(CellValue::Integer((lo as i128 + span * index as i128 / want as i128) as i64))
        };
        let high = if index + 1 == want {
            None
        } else {
            Some(CellValue::Integer(
                (lo as i128 + span * (index + 1) as i128 / want as i128) as i64,
            ))
        };
        out.push(RowRange { index, low, high, window: None });
    }
    Some(out)
}

/// 浮点/数值键同 `split_integer`，边界按比例落在值域上。
fn split_real(lo: f64, hi: f64, want: usize, min_rows: i64) -> Option<Vec<RowRange>> {
    if want < 2 || !(hi > lo) {
        return None;
    }
    let span = hi - lo;
    if span < (want as f64) * (min_rows as f64) {
        return None;
    }
    let mut out = Vec::with_capacity(want);
    for index in 0..want {
        let low = if index == 0 {
            None
        } else {
            Some(CellValue::Real(lo + span * index as f64 / want as f64))
        };
        let high = if index + 1 == want {
            None
        } else {
            Some(CellValue::Real(lo + span * (index + 1) as f64 / want as f64))
        };
        out.push(RowRange { index, low, high, window: None });
    }
    Some(out)
}

/// 给引擎/日志用的一句话描述（诊断时能看出「这张表怎么读的」）。
pub fn describe(range: &RowRange, table: &str) -> String {
    if range.is_whole() {
        format!("{table}：整表单流")
    } else {
        format!(
            "{table}：区间 [{}, {})",
            range.low.as_ref().map(|v| format!("{v:?}")).unwrap_or_else(|| "-∞".to_string()),
            range.high.as_ref().map(|v| format!("{v:?}")).unwrap_or_else(|| "+∞".to_string())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 分片的第一条底线：**相邻两片的边界严格相接**（上一片的 high == 下一片的 low），
    /// 首片下界开放、末片上界开放。任何一处不接，就意味着一整段数据漏搬或重复搬。
    #[test]
    fn 整数分片连续且不重不漏() {
        let ranges =
            split_integer(1, 1_000_000, 4, MIN_ROWS_PER_SLICE).expect("100 万行应能切 4 片");
        assert_eq!(ranges.len(), 4);
        assert!(ranges[0].low.is_none(), "首片应取下界开放");
        assert!(ranges[3].high.is_none(), "末片应取上界开放");
        for pair in ranges.windows(2) {
            assert_eq!(
                format!("{:?}", pair[0].high),
                format!("{:?}", pair[1].low),
                "相邻片边界不相接"
            );
        }
        for (index, range) in ranges.iter().enumerate() {
            assert_eq!(range.index, index, "片号必须连续（会话键靠它区分连接）");
        }
    }

    #[test]
    fn 行数太少或只有一片时不切() {
        // 1000 行切 4 片，每片 250 行 —— 建连成本比省下的时间还多
        assert!(split_integer(1, 1_000, 4, MIN_ROWS_PER_SLICE).is_none());
        assert!(split_integer(1, 1, 4, MIN_ROWS_PER_SLICE).is_none(), "单行表不能切");
        assert!(split_integer(1, 1_000_000, 1, MIN_ROWS_PER_SLICE).is_none(), "want=1 就是不切");
    }

    /// 低速链路上的真实教训：20 万行按 6 片切，每片 3.3 万行 —— 旧的 5 万行门槛
    /// 让整表退化成单流（单连接上行 340 KB/s），新门槛必须肯切。
    #[test]
    fn 中等表在低速链路上也要能分片() {
        let adaptive = 5_000;
        let ranges = split_integer(1, 200_000, 6, adaptive).expect("20 万行按 6 片应能切");
        assert_eq!(ranges.len(), 6);
        // 但同样的表用旧的固定门槛就不再切 —— 这就是当初单流的成因
        assert!(split_integer(1, 200_000, 6, MIN_ROWS_PER_SLICE).is_none());
    }

    #[test]
    fn 浮点分片同样首尾开放() {
        let ranges =
            split_real(0.0, 1_000_000.0, 3, MIN_ROWS_PER_SLICE).expect("应能切 3 片");
        assert_eq!(ranges.len(), 3);
        assert!(ranges[0].low.is_none());
        assert!(ranges[2].high.is_none());
    }

    /// 无键表的窗口切分：段与段首尾相接、合起来正好覆盖 `[0, total)`。
    #[test]
    fn 窗口切分不重不漏() {
        for total in [1u64, 3, 100_001, 3_570_000] {
            for slices in [1usize, 2, 3, 4, 8] {
                let bounds = window_bounds(total, slices);
                assert_eq!(bounds.first().unwrap().0, 0, "首段必须从 0 开始");
                assert_eq!(bounds.last().unwrap().1, total, "末段必须到 total 结束");
                for pair in bounds.windows(2) {
                    assert_eq!(pair[0].1, pair[1].0, "相邻两段必须首尾相接");
                }
                assert!(bounds.iter().all(|(begin, end)| begin < end), "不能出现空段");
                assert!(bounds.len() <= slices, "段数不能超过要求");
            }
        }
        assert!(window_bounds(0, 4).is_empty());
    }

    /// 分页键位置按**取数列**取名，不是目标 upsert 键的位置。
    ///
    /// 目标表已存在且主键与源主键不同时，两套位置会错位 —— keyset 会拿错列的值去比较，
    /// 表现是「静默丢行」，最难查的那种。
    #[test]
    fn 分页键位置按取数列取名() {
        let columns = vec!["code".to_string(), "name".into(), "id".into()];
        // 源主键 id 在第 3 列（目标主键 code 在第 1 列，plan.key_indexes 会是 [0]）
        assert_eq!(paging_positions(&columns, &["ID".to_string()]), Some(vec![2]));
        assert_eq!(
            paging_positions(&columns, &["code".to_string(), "id".to_string()]),
            Some(vec![0, 2])
        );
        // 键不在取数列里 ⇒ None（调用方退回 OFFSET），不能悄悄少一个位置
        assert_eq!(paging_positions(&columns, &["missing".to_string()]), None);
    }

    /// 无键表的整片：不带任何边界条件（分页只能走 OFFSET）
    #[test]
    fn 整片不带边界() {
        let whole = RowRange::default();
        assert!(whole.is_whole());
        assert_eq!(whole.index, 0);
    }

    /// 页大小：没有写速样本时只看行宽（窄表升到上限，宽表降到下限）。
    #[test]
    fn 页大小没有写速样本时按行宽() {
        let cap = PAGE_ROWS_MAX;
        // 20 行 × 40 字节 = 800 字节，远小于 32 MiB ⇒ 直接放到上限
        assert_eq!(page_target_rows(20_000, 20_000 * 40, 200, 0, cap, true), cap);
        // 20 行 × 2 KiB = 40 KiB 一页，32 MiB 只放得下约 16000 行
        let narrow = page_target_rows(20_000, 20_000 * 2048, 200, 0, cap, true);
        assert!((14_000..18_000).contains(&narrow), "宽行应被压到万级：{narrow}");
        // 极宽行（一行 512 KiB）⇒ 落到下限，绝不出现 0
        assert_eq!(
            page_target_rows(20_000, 20_000 * 512 * 1024, 200, 0, cap, true),
            PAGE_ROWS_MIN
        );
    }

    /// 页大小：有写速样本时，一页约等于写速 × 20 秒（写慢就缩，写快就放到上限）。
    #[test]
    fn 页大小按写速换算成二十秒的量() {
        let cap = PAGE_ROWS_MAX;
        // 1 000 行/秒 ⇒ 一页约 2 万行
        assert_eq!(page_target_rows(20_000, 20_000 * 40, 200, 1_000, cap, true), 20_000);
        // 200 行/秒 ⇒ 4 千行（慢目标别囤页）
        assert_eq!(page_target_rows(20_000, 20_000 * 40, 200, 200, cap, true), 4_000);
        // 极慢也不低于下限，极快也不超过上限
        assert_eq!(page_target_rows(20_000, 20_000 * 40, 200, 1, cap, true), PAGE_ROWS_MIN);
        assert_eq!(
            page_target_rows(20_000, 20_000 * 40, 200, 10_000_000, cap, true),
            cap
        );
    }

    /// 无键表不看写速：OFFSET 翻页的页必须能长大，否则重扫是 O(N²)。
    #[test]
    fn 无键表不按写速缩页() {
        let cap = PAGE_ROWS_MAX;
        assert_eq!(page_target_rows(20_000, 20_000 * 40, 200, 200, cap, false), cap);
    }

    /// 读一页太慢（丢包链路）时缩页：20 万行读了 40 秒 ⇒ 只留 5 秒的量。
    #[test]
    fn 读得太慢就缩页() {
        let cap = PAGE_ROWS_MAX;
        // 2 万行读 20 秒 ⇒ 一页收到 5 秒的量（5000 行）
        assert_eq!(page_target_rows(20_000, 20_000 * 40, 20_000, 0, cap, true), 5_000);
        // 窄表 + 快链路（2 万行 0.3 秒）⇒ 不受读耗时约束，仍是上限
        assert_eq!(page_target_rows(20_000, 20_000 * 40, 300, 0, cap, true), cap);
    }

    /// 失败缩页：÷4、不低于逃生下限，硬上限跟着缩到 2 倍以内。
    #[test]
    fn 失败缩页到四分之一() {
        assert_eq!(shrink_target(10_000, None), (2_500, 5_000));
        // 已经很小就不再缩（下限 500 不能反过来把页撑大）
        assert_eq!(shrink_target(400, None), (400, 800));
    }

    /// 固定页模式（`DBMIND_SYNC_PAGE_ROWS`）：失败缩页后必须能爬回用户给的值，
    /// 且回升一步不超过 `fixed_page`。过去不区分固定页 ⇒ 一次抖动后永久 1/4。
    #[test]
    fn 固定页缩完能爬回去() {
        let fixed = 10_000;
        let (shrunk, cap) = shrink_target(fixed, Some(fixed));
        assert_eq!((shrunk, cap), (2_500, 5_000), "硬上限不得超过用户给的值");
        let mut now = shrunk;
        let mut steps = 0;
        while now < fixed {
            let up = recover_target(now, fixed);
            assert!(up > now, "每次成功读页都要前进");
            assert!(up <= fixed, "回升不得超过固定页：{up}");
            now = up;
            steps += 1;
            assert!(steps <= 4, "回升步数应有界");
        }
        assert_eq!(now, fixed);
        // 固定页小于逃生下限时不越界
        assert_eq!(shrink_target(600, Some(600)).1, 600);
        assert_eq!(recover_target(400, 600), 600);
    }
}
