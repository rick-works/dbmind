//! 传输编排：**分片 → 每片一个「读游标 + 写连接 + 事务」 → 片间并行、片级重试**。
//!
//! ```text
//!   split() ──► [片#0: reader ──► writer(事务#0)] ─┐
//!               [片#1: reader ──► writer(事务#1)] ─┼─► 逐片提交；失败的片回滚后重跑该片
//!               [片#2: reader ──► writer(事务#2)] ─┘
//! ```
//!
//! ## 为什么把事务绑在「片」上（而不是「一张表一个事务 / 多路写共享」）
//!
//! 一条 JDBC 连接只有一个事务，多路写**不可能**共享同一个事务，只能是 N 个独立事务
//! （`writer` 模块文档里写得很清楚）。以前收尾时统一提交，看起来像「整表一个事务」，
//! 但断连发生在提交阶段时，先提交的那一路已经落盘 —— 再整表重跑就是重复数据。
//!
//! 现在把「事务」和「键值区间」绑在一起：片内单连接单事务，片与片之间键值区间不相交。
//! 于是：
//! - 某片断连 ⇒ 只回滚这一片、只重跑这一片，**不会重复、不会漏**（其它片已提交且不相交）；
//! - 片内读→写串行，天然有背压，不会出现「读飞快、写堆一队列」的假进度；
//! - 片内再按 [`txn_rows_cap`]（= 界面「每批条数」）拆成多个事务：一个事务几百万行会让 undo/redo 膨胀、
//!   checkpoint 追不上刷盘，表现就是「越写越慢」（DataX 用 batchSize、Kettle 用 commit
//!   size，本质都是"小事务高频提交"）；拆分点用 keyset 续上，段与段不重不漏；
//! - 这正 Kettle / DataX 的 channel 模型（一个 channel = 一条读 + 一条写 + 一个事务）。
//!
//! ## 代价（明确写在这里，避免将来当成 bug 排查）
//!
//! 无键表只能切 1 片（`OFFSET` 分页无法保证「片内不重不漏」），所以**无键表的写并发恒为 1**。
//! 实测瓶颈在源端的 OFFSET 扫描（越往后越慢），写侧 1 路够用；只有「快源 + 慢目标」的
//! 无键大表才会感觉到与「多路写」的差别。
//!
//! 为什么读侧也要分片：远端库单连接吞吐常被「往返延迟 × 每页行数」卡死，一条连接只能跑
//! 200~300 KB/s；切 4 片并发读能直接把链路上限吃满（实测 20 万行跨网从 39s 降到 21s，
//! 本地库不受影响 —— 单连接池的库会自动退回 1 片）。

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dbmind_core::CellValue;

use super::reader::Cursor;
use super::{reader_for, writer_for, Page, RowRange, Stats, TablePlan, UpsertMode};
use crate::api::sync::{estimate_row_bytes, key_of, lookup_existing_keys};

/// 单页失败后的重试次数（退避 150ms / 300ms / 600ms）。
///
/// 只对**读**做重试：读是幂等的，重试没有副作用；写不做整批重试（一条语句可能已经
/// 落了一半），而是交给写入层的「语句失败 → 二分拆批定位坏行」兜住。
const READ_RETRY: usize = 3;

/// 收尾等待上限（与 sync 层一致：给慢批留足时间，避免把还在写的数据判死）
const PIPE_WAIT: Duration = Duration::from_secs(660);

/// 进度状态最短刷新间隔：写一页更新一次就够，**不必每行**都更新
/// （20 万行就是 20 万次加锁 + 20 万次前端快照变化，纯属白烧 CPU）。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(300);

/// 等写槽位时多久回来看一次「是不是被取消了」。
///
/// 片数可能多于写并发（4 片 / 1 条连接的单连接池库），多出来的片要在槽位队列里排队。
/// 排队期间不查取消，用户点「停止」就得等前一个片跑完（大表可能几分钟）——
/// 真机上表现为「点了停止没反应」。
const CANCEL_POLL: Duration = Duration::from_millis(300);

/// 段结束时等预读协程收尾的宽限（超过就 abort）。
const PREFETCH_GRACE: Duration = Duration::from_secs(2);

/// 读预取开关（`DBMIND_SYNC_NO_PREFETCH=1` 关掉）。
///
/// 留着开关有两个用处：A/B 对比实验（同一份二进制），以及万一某个驱动的读会话
/// 不允许「读还在飞的时候又发下一条」。
fn prefetch_enabled() -> bool {
    std::env::var_os("DBMIND_SYNC_NO_PREFETCH").is_none()
}

/// 段末预取页**交接**开关（`DBMIND_SYNC_CARRY=0` 关掉，见 [`CarriedPage`]）。
///
/// 与 [`prefetch_enabled`] 一样留个开关：A/B 对照时可以用同一份二进制分别跑
/// 「交接 / 丢弃」两种行为。
fn carry_enabled() -> bool {
    std::env::var("DBMIND_SYNC_CARRY")
        .map(|value| {
            !(value.trim() == "0"
                || value.eq_ignore_ascii_case("false")
                || value.eq_ignore_ascii_case("off"))
        })
        .unwrap_or(true)
}

/// 写批次多久做一次判活（`select 1` 另开会话）。
///
/// **不再按时间判死**（用户口径：Navicat / DBX 不会中断，我们也不许）：
/// 原来一批写到 60~300 秒（`WriteDiag::batch_watchdog`）就判「连接半开」，
/// 回滚整段换连接重跑 —— 可一条慢语句（丢包链路重传 / 目标端在刷盘）本来就要这么久，
/// 判定的结果就是「写入中断」反复刷。现在改成探活：
/// - 通 ⇒ 链路活着，这批只是慢，**继续等**；
/// - 不通 ⇒ 真的断了，弃连接、回滚该段、换连接重跑。
const BATCH_PROBE_TICK: Duration = Duration::from_secs(30);

/// 慢批自愈的检查粒度（只比较时间，不发任何请求）。
///
/// 探活仍是 [`BATCH_PROBE_TICK`]（30 秒）一次 —— 那是「判断断没断」的成本；
/// 而「这一页写了多久」只在本进程里算，2 秒检查一次几乎零成本，
/// 否则 60 秒的阈值实际要等到第 60/90 秒的探活 tick 才判定，白等半分钟。
const SLOW_CHECK_TICK: Duration = Duration::from_secs(2);

// 一批写的**绝对**上限不再写死：由界面的「超时时间」决定（见 `wait_write` 里的
// `hard_limit` / `sync::op_timeout_ms`）—— 0 = 继承连接（内核默认 600 秒）、
// 负 = 不限制、正 = 用户填的秒数。这样界面上只有一个旋钮，与 DBX 的「操作超时」对齐。

/// 回滚/收尾语句自己的墙钟：半开连接上连 `rollback` 都会挂住（真机整条同步就此卡死）。
const ABORT_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// 读一页的上限：跟随界面的「超时时间」，但夹在 **30~180 秒**之间。
///
/// 为什么不像写那样直接放到 600 秒：读是**幂等**的 —— 超时之后会缩页重试
/// （`reader.shrink_page()`），等十分钟才重试反而更亏；而 30 秒对 32MiB 的大页
/// 在慢链路上又确实偏紧（1 MB/s 要 32 秒，会被误判后整页重读）。180 秒是两头都顾到的上限。
fn read_timeout(secs: i64) -> Duration {
    Duration::from_millis(crate::api::sync::op_timeout_ms(secs).clamp(30_000, 180_000))
}

/// 提交（commit）的上限：跟随「超时时间」，夹在 **60~600 秒**之间。
///
/// 为什么宁可多等：提交失败属于「这一段是否落盘无法确定」，**不能自动重跑**
/// （可能已经写进去了，重跑会重复）。所以这里给足时间，别把慢盘上的正常提交判死。
fn commit_timeout(secs: i64) -> Duration {
    Duration::from_millis(crate::api::sync::op_timeout_ms(secs).clamp(60_000, 600_000))
}

/// 写超时后的**目标端探活**上限（另开一条会话跑 `select 1`）。
///
/// 为什么要探：墙钟只能说明「这条语句没回来」，分不清两种完全不同的故障 ——
/// 一条连接半开（换连接就行），还是整条链路/目标不可用（换连接只会再撞一次）。
/// 探活失败 ⇒ 后者，此时该让**整个任务**停一会儿，而不是几百张表一起重连重跑。
///
/// 取 10 秒（原来是 5 秒）：`select 1` 只有几百字节，但**丢一个包就要重传**
/// （RTO 0.2→0.4→0.8…），链路抖一下 5 秒根本不够 —— 探活本身被误判成「断」，
/// 就会白回滚一段再重跑。10 秒 + 下面的一次复核，足够把「偶发丢包」和「真断」分开。
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// 探活失败后整个任务暂停多久（毫秒）。
///
/// 取 15 秒：RDS 抖动/网络瞬断通常在这个量级内恢复；比逐张表各自重连（每次至少
/// 一个建连往返 + 一段回滚）便宜得多，也不会让用户觉得「卡住了」。
const LINK_HOLD_MS: u64 = 15_000;

/// 片任务共享的运行态。
struct SliceCtx {
    plan: Arc<TablePlan>,
    reader: Arc<dyn super::Reader>,
    writer: Arc<dyn super::Writer>,
    /// 已落库（提交）的行数 —— 重跑该片时会把上一次尝试的数扣回
    inserted: Arc<AtomicU64>,
    /// upsert 更新行数（两阶段），同样可扣回
    updated: Arc<AtomicU64>,
    /// 读出行数（含行数上限配额）；重跑该片时扣回，保证「重跑不算多读」
    consumed: Arc<AtomicU64>,
    /// 是否撞过行数上限（用于收尾备注）
    limited: Arc<AtomicBool>,
    /// 读/写耗时画像（毫秒）：各路并发累加，用来回答「瓶颈在读还是在写」。
    ///
    /// 没有这组数就只能靠猜 —— 猜「读慢」去加读并发，实际卡在目标端刷盘，
    /// 越加越慢还多开连接。表的收尾日志会把两个数打出来（见 `run`）。
    read_ms: Arc<AtomicU64>,
    write_ms: Arc<AtomicU64>,
    /// 「传输数据到目标表」这条链路日志是否已经打过（多 worker 共享，只打一次）
    transfer_told: Arc<AtomicBool>,
}

/// 细块数 = 读并发上限的倍数。
///
/// 为什么要有「块」与「并发」两套数：片数是表**开始时**按当时的并发表数定死的，
/// 尾部别的表退出后没法重切。先把键值区间切细（2×），运行时只要往令牌池里补令牌，
/// 立刻就能多开几路读写 —— 每块仍是一个独立事务，块间区间不相交，照样不重不漏。
const BLOCKS_PER_SLICE: usize = 2;

/// 片令牌池：**并发片数 = 池里已发出的令牌数**。
///
/// 与「一片一个任务、任务数定死并发」相比，令牌可以**运行时补发**：尾部只剩一张大表时，
/// 调度侧把省下的配额写进 `GrowCaps`，这里读到更大的目标值就给池子加号，多余的任务
/// （块队列里还有块）立刻并行起来。只增不缩：缩容会把已开的事务白扔。
struct Slots {
    free: std::sync::Mutex<Vec<usize>>,
    notify: tokio::sync::Notify,
    issued: AtomicUsize,
    /// 硬上限（= min(读并发上限, 写并发上限)）
    limit: usize,
}

impl Slots {
    fn new(initial: usize, limit: usize) -> Arc<Self> {
        let limit = limit.max(1);
        let initial = initial.clamp(1, limit);
        Arc::new(Self {
            free: std::sync::Mutex::new((0..initial).collect()),
            notify: tokio::sync::Notify::new(),
            issued: AtomicUsize::new(initial),
            limit,
        })
    }

    /// 把令牌补到 `total` 个（含正在用的）。只增不减，返回补发个数。
    fn grow_to(&self, total: usize) -> usize {
        let want = total.min(self.limit);
        let mut free = match self.free.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let issued = self.issued.load(Ordering::SeqCst);
        if want <= issued {
            return 0;
        }
        for id in issued..want {
            free.push(id);
        }
        self.issued.store(want, Ordering::SeqCst);
        drop(free);
        // 唤醒所有等号的任务；漏唤醒也无所谓 —— 等待本身带 300ms 超时轮询（见 `acquire`）
        self.notify.notify_waiters();
        want - issued
    }

    fn try_acquire(&self) -> Option<usize> {
        let mut free = match self.free.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        free.pop()
    }

    fn release(&self, id: usize) {
        let mut free = match self.free.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        free.push(id);
        drop(free);
        self.notify.notify_one();
    }

    /// 等一个令牌；返回 `None` = 用户点了停止（每 [`CANCEL_POLL`] 回来看一次取消）。
    async fn acquire(&self, plan: &TablePlan) -> Option<usize> {
        loop {
            if plan
                .task
                .as_ref()
                .map(|task| task.is_canceled())
                .unwrap_or(false)
            {
                return None;
            }
            if let Some(id) = self.try_acquire() {
                return Some(id);
            }
            let _ = tokio::time::timeout(CANCEL_POLL, self.notify.notified()).await;
        }
    }
}

/// 跑完一张表的数据搬运。`Err` = 该表失败（已提交的片保留）。
pub async fn run(plan: TablePlan) -> Result<Stats, String> {
    let started = std::time::Instant::now();
    let mut reader_plugin = reader_for(&plan);
    let writer = writer_for(&plan);

    // ---- 1. 分片：按**细块**切，并发由令牌池控制（尾部可扩）-----------------
    //
    // `want` 是**本次分到的并发份额**（初始值），`*_ceiling` 是方言声明的上限
    // （单连接池的库两者都是 1，天然不扩）。块数按上限切细 2 倍：尾部别的表退出后，
    // 只要补令牌就能多开几路，不必重切表；每块仍是一个独立事务（区间不相交）。
    let want = reader_plugin.max_parallel().max(1);
    let slice_ceiling = reader_plugin.parallel_ceiling().max(want);
    let writer_workers = writer.workers().max(1);
    let worker_ceiling = writer.workers_ceiling().max(writer_workers);
    let start_parallel = want.min(worker_ceiling).max(1);
    let limit = slice_ceiling.min(worker_ceiling).max(start_parallel);
    let blocks = (slice_ceiling * BLOCKS_PER_SLICE).max(want).max(1);
    let mut ranges = reader_plugin.split(want).await?;
    if ranges.len() > 1 && blocks > ranges.len() {
        // 再按更细的块切一次：低于「每片最小行数」时 split 会返回单块，此时沿用原片
        if let Ok(finer) = reader_plugin.split(blocks).await {
            if finer.len() > ranges.len() {
                ranges = finer;
            }
        }
    }
    let slices = ranges.len();
    // 块数才是并发的天花板：块少时补令牌没有任务能领（无键表恒 1 块 = 恒单流）
    let limit = limit.min(slices).max(start_parallel);
    // 分片方式是内部策略，**不进界面日志**（界面只留「完成 · 行数 · 耗时」这种结果行）；
    // 文件日志里要能一眼看出「按主键切的 / 按唯一索引切的 / 退回 OFFSET 了」——
    // 无键表越往后越慢，排障第一眼就靠这句。成因也分开说：源表本来没主键 vs 主键列没勾全，
    // 建议完全不同（后者还让用户「去加主键」就是错方向）。
    if slices > 1 && tracing::enabled!(tracing::Level::DEBUG) {
        // `plan.columns` 只有列名（没有 primary_key 标记）⇒ 判断「按主键切的」要拿
        // `src_primary` 比：paging_keys 全部落在源主键列里才是主键分片。
        let from_primary = !plan.paging_keys.is_empty()
            && plan
                .paging_keys
                .iter()
                .all(|key| plan.src_primary.iter().any(|primary| primary.eq_ignore_ascii_case(key)));
        let missing_primary = !plan.src_primary.is_empty()
            && plan
                .src_primary
                .iter()
                .any(|key| !plan.columns.iter().any(|column| column.eq_ignore_ascii_case(key)));
        let how = if plan.paging_keys.is_empty() {
            if missing_primary {
                format!("主键列没有全部纳入同步，按行号范围切成 {slices} 段（每段 OFFSET 定位）")
            } else if plan.src_primary.is_empty() {
                format!("源表没有主键，也没有可当游标的唯一索引，按行号范围切成 {slices} 段")
            } else {
                format!("没有可当游标的唯一索引，按行号范围切成 {slices} 段（每段 OFFSET 定位）")
            }
        } else if from_primary {
            format!("按主键切成 {slices} 段并行搬运")
        } else {
            format!("按唯一索引切成 {slices} 段并行搬运")
        };
        tracing::debug!(table = %plan.table, how = %how, "分片方式");
    }
    let reader: Arc<dyn super::Reader> = Arc::from(reader_plugin);

    // ---- 2. 写槽位：每片独占一条连接（= 一个事务）---------------------------
    //
    // 片数可能多于写并发（4 片 / 1 条连接的单连接池库），用槽位令牌把「同时搬运的片数」
    // 压到写并发以内：拿不到槽位的片排队，拿到才开事务。这样既不会超过连接预算，
    // 也不存在「片 A 等片 B 释放连接」的等待环。
    let slots = Slots::new(start_parallel, limit);

    let inserted = Arc::new(AtomicU64::new(0));
    let updated = Arc::new(AtomicU64::new(0));
    // 已读行数挂在 plan 上（`TablePlan::progress`）：上层用它判「这表还有没有进展」，
    // 而不是拿固定总时长判超时
    let consumed = plan.progress.clone();
    let limited = Arc::new(AtomicBool::new(false));
    let read_ms = Arc::new(AtomicU64::new(0));
    let write_ms = Arc::new(AtomicU64::new(0));
    let plan = Arc::new(plan);
    let ctx = Arc::new(SliceCtx {
        plan: plan.clone(),
        reader,
        writer,
        inserted: inserted.clone(),
        updated: updated.clone(),
        consumed: consumed.clone(),
        limited: limited.clone(),
        read_ms: read_ms.clone(),
        write_ms: write_ms.clone(),
        transfer_told: Arc::new(AtomicBool::new(false)),
    });

    // ---- 3. 块队列 + 令牌池（读 → 写 → 提交；失败回滚重跑该块）-------------
    //
    // 任务数 = 块数（多于初始并发），实际同时跑几块由令牌池决定：这样尾部补发令牌时，
    // 早已就位、正在等令牌的任务立刻就能领块开跑，不需要再 spawn（也就不需要重切表）。
    // 链路日志第 3 步：开始取数（前两步「删除目标表 / 创建目标表」在 sync 层打过）
    if let Some(task) = &ctx.plan.task {
        task.log(format!("表 {}：读取源数据", ctx.plan.table));
    }
    let queue = Arc::new(std::sync::Mutex::new(ranges.into_iter()));
    let mut handles = Vec::with_capacity(slices);
    for _ in 0..slices.max(1) {
        let ctx = ctx.clone();
        let slots = slots.clone();
        let queue = queue.clone();
        handles.push(tokio::spawn(async move {
            loop {
                // 等令牌时也要能取消（见 `CANCEL_POLL`）：排队的块不能拖住「停止」
                let Some(worker) = slots.acquire(&ctx.plan).await else {
                    return Ok(());
                };
                let range = {
                    let mut guard = match queue.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    guard.next()
                };
                let Some(mut range) = range else {
                    // 块领完了：还令牌，任务退出
                    slots.release(worker);
                    return Ok(());
                };
                // 诊断：块号必须在被改成通道号**之前**记下来，否则日志里只剩 0..limit，
                // 看不出「切成 8 块却只跑了 4 块」这类漏块。
                tracing::info!(table = %ctx.plan.table, block = range.index, "同步块开始");
                // 读会话按**通道号**而不是块号：块换一个就换一条连接的话，几十个块会把
                // 会话池堆满（每个块号一条空闲连接）。通道号恒 ≤ limit，读连接就固定这几条。
                let block_no = range.index;
                range.index = worker;
                let outcome = run_slice(ctx.clone(), worker, range).await;
                tracing::info!(
                    table = %ctx.plan.table,
                    block = block_no,
                    ok = outcome.is_ok(),
                    "同步块结束"
                );
                // 还令牌：下一块（或同一块的下一轮重试）要用
                slots.release(worker);
                // 一块失败 ⇒ 这张表已经判失败，本任务不再领块；其它任务照旧把已领的块跑完
                if let Err(msg) = outcome {
                    return Err(msg);
                }
            }
        }));
    }

    // ---- 3b. 尾部弹性：调度侧把配额调高后，就地补令牌 ----------------------
    let grower = plan.grow.clone().map(|caps| {
        let slots = slots.clone();
        let plan = plan.clone();
        let base_workers = writer_workers;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(500));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut current = start_parallel;
            loop {
                tick.tick().await;
                if plan
                    .task
                    .as_ref()
                    .map(|task| task.is_canceled())
                    .unwrap_or(false)
                {
                    continue;
                }
                // 读片与写路必须同时够：片数 = min(两边)，否则多给的片子只能排队等写连接
                let target = caps
                    .slots(want)
                    .min(slice_ceiling)
                    .min(caps.want_workers(base_workers))
                    .min(limit);
                if target > current && slots.grow_to(target) > 0 {
                    current = target;
                    tracing::debug!(table = %plan.table, target, "其它对象已结束，并发扩容");
                }
            }
        })
    });

    // ---- 4. 收尾 -----------------------------------------------------------
    //
    // 超时判据是「**无进展**超过 PIPE_WAIT」，不是「总时长超过 PIPE_WAIT」：
    // 有主键的 5000 万行表要跑 30 分钟以上，无主键 500 万行（OFFSET 翻页 7.2k 行/秒）
    // 要 11 分钟 —— 按总时长判会把一直在搬数据的表整表放弃（真机就是「完成 400/401」）。
    // 有进展就一直等，停滞够久才判死。
    let mut failure: Option<String> = None;
    for mut handle in handles {
        let mut seen = consumed.load(Ordering::SeqCst);
        let mut idle_since = std::time::Instant::now();
        let joined = loop {
            match tokio::time::timeout(Duration::from_secs(30), &mut handle).await {
                Ok(result) => break Some(result),
                Err(_) => {
                    let now = consumed.load(Ordering::SeqCst);
                    if now != seen {
                        seen = now;
                        idle_since = std::time::Instant::now();
                    } else if idle_since.elapsed() >= PIPE_WAIT {
                        // 11 分钟没有任何一页落库：先探活，分清「这一页就是慢」和「连接已断」。
                        //
                        // 为什么必须分（真机 400 对象任务的后段）：前面上百万行的大表一直在跑、
                        // 每页分钟级落库；到后段宽表（大字段/JSON，一页报文上 MB）在丢包链路上
                        // 一页就要十几分钟 —— 按「无进展」直接判死，用户看到的就是「大表前面
                        // 都不报错，越到后面越报『传输协程 11 分钟没有任何进展』」。
                        if probe_target_confirmed(&plan).await {
                            // 链路活着 ⇒ 只是这一页慢，继续等（与 Navicat / DBX 一致）
                            idle_since = std::time::Instant::now();
                            if let Some(task) = &plan.task {
                                task.log(format!(
                                    "表 {}：一页超过 {} 分钟没有落库，但目标端探活正常（链路没断），继续等待",
                                    plan.table,
                                    PIPE_WAIT.as_secs() / 60
                                ));
                            }
                            continue;
                        }
                        // 探活也不通 ⇒ 真卡死了：掐掉协程，别让它挂着占连接
                        handle.abort();
                        break None;
                    }
                }
            }
        };
        match joined {
            None => {
                if failure.is_none() {
                    failure = Some(format!(
                        "传输协程 {} 分钟没有任何进展，且目标端探活不通：链路已断，本次同步数据不完整",
                        PIPE_WAIT.as_secs() / 60
                    ));
                }
            }
            Some(Ok(Ok(()))) => {}
            Some(Ok(Err(msg))) => {
                if failure.is_none() {
                    failure = Some(msg);
                }
            }
            Some(Err(_)) => {
                if failure.is_none() {
                    failure = Some(
                        "传输协程异常终止（驱动异常或内存不足），本次同步数据不完整".to_string(),
                    );
                }
            }
        }
    }

    // 扩容监控跟着本表一起收尾：再补令牌也没有块可领了
    if let Some(grower) = grower {
        grower.abort();
    }

    // 取消状态以收尾这一刻为准：片任务看到「停止」会先撤离，此时它刚写完的页已经计数
    let canceled = plan
        .task
        .as_ref()
        .map(|t| t.is_canceled())
        .unwrap_or(false);

    let mut note = String::new();
    let inserted_rows = inserted.load(Ordering::SeqCst);
    let updated_rows = updated.load(Ordering::SeqCst);
    if let Some(msg) = failure {
        return Err(msg);
    }
    if canceled {
        note.push_str("（已取消：已提交的片保留，未提交的片已回滚）");
    }
    if limited.load(Ordering::SeqCst) && plan.row_limit > 0 {
        note.push_str(&format!(
            "（已达行数上限 {}，仅同步了前 {} 行）",
            plan.row_limit, plan.row_limit
        ));
    }
    // 读/写耗时画像：调参的唯一依据。
    //
    // 口径要注意：这是**各路并发累加**的「请求占用的墙钟」，不等于总耗时。
    // 读 ≫ 写 ⇒ 瓶颈在源端取数（加读片 / 加大页 / 预读）；
    // 写 ≫ 读 ⇒ 瓶颈在目标端落库（加写路 / 加大语句 / 目标库参数）。
    let read_spent = read_ms.load(Ordering::SeqCst);
    let write_spent = write_ms.load(Ordering::SeqCst);
    if let Some(task) = &plan.task {
        let done_rows = inserted_rows + updated_rows;
        if done_rows > 0 {
            let total_ms = started.elapsed().as_millis().max(1) as u64;
            let rate = done_rows.saturating_mul(1_000) / total_ms;
            // 链路日志第 5 步（终点）：这张表搬完，多少行、花了多久。
            // 读/写分解、并发路数属于调参信息，进 tracing 就够了。
            task.log(format!(
                "表 {}：传输完成，共 {}，耗时 {}",
                plan.table,
                super::行数(done_rows),
                super::时长(total_ms),
            ));
            tracing::debug!(
                table = %plan.table,
                read_s = read_spent as f64 / 1_000.0,
                write_s = write_spent as f64 / 1_000.0,
                total_s = total_ms as f64 / 1_000.0,
                rate,
                lanes = slots.issued.load(Ordering::SeqCst).max(1),
                page_rows = ctx.reader.page_rows(),
                "同步耗时画像"
            );
        }
    }

    if plan.upsert == UpsertMode::Native && inserted_rows > 0 {
        // 原生 upsert 由服务端判插入还是更新，SQL 层面拿不到「新增/更新」的拆分
        note.push_str("（原生 upsert：新增/更新未区分）");
    }
    // 读了却没算进 inserted/updated 的行 = 逐行降级后仍失败的行（脏数据）。
    // 取消时 consumed 里夹着没来得及写的页，差值没有意义，故置 0。
    let failed = if canceled {
        0
    } else {
        consumed
            .load(Ordering::SeqCst)
            .saturating_sub(inserted_rows + updated_rows)
    };
    if failed > 0 {
        // 必须写进结果消息：这些行**没有**落库，界面上的「成功」不能把这件事盖住。
        note.push_str(&format!("（{failed} 行写入失败，详见任务日志）"));
    }
    Ok(Stats {
        inserted: inserted_rows,
        updated: updated_rows,
        failed,
        canceled,
        note,
    })
}

/// 单个事务最少多少行（`DBMIND_SYNC_TXN_ROWS` 的下限）：1 行也合法，
/// 只为对比实验留门。
const TXN_ROWS_MIN: u64 = 1;

/// 一个事务最多写多少行 = **界面「每批条数」**（一批 = 一页 = 一个事务）。
///
/// 为什么必须封顶（历史教训）：原来「一个块一个事务」，5000 万行切成 8 块 = 每个事务
/// 625 万行。大事务会让 undo/redo 一路膨胀、history list 变长、checkpoint 追不上刷盘，
/// 表现就是「越写越慢」（实测 5000 万行从 23.8k 行/秒掉到 18.6k）。DataX（batchSize
/// 默认 2048）和 Kettle（commit size 默认 1000）都是小事务高频提交，这是它们不会
/// 越跑越慢的主因。
///
/// 封到多大？以前写死 10 万行，现在跟随 [`super::Reader::commit_rows`]：用户设 1000
/// 就每 1000 条提交一次，概念只剩一个；卡住/断连时最多回滚一批，重跑也只重跑一段
/// （段与段之间 keyset/OFFSET 续跑，不重不漏）。无键表例外 —— 它只能 OFFSET 翻页，
/// 提交粒度跟着大页走，否则每次提交都从 0 重扫（O(N²)）。
///
/// 代价实测过（源 192.168.2.216 跨网 → 实验室 MariaDB，t2m 197 万行，30 秒采样）：
/// - 每 10 万条提交：读取 88.5 万 / 传输 84.4 万；
/// - 每 1000 条提交：读取 88.9 万 / 传输 84.8 万。
/// 提交次数多了 100 倍，吞吐没变 —— 因为一批 1000 行的读写本身就占 0.1 秒量级，
/// commit + begin 各一次往返摊进去可以忽略（目标端 fsync 在本机 SSD 上也只需 1~2ms）。
fn txn_rows_cap(batch_rows: u64) -> u64 {
    std::env::var("DBMIND_SYNC_TXN_ROWS")
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .filter(|rows| *rows >= TXN_ROWS_MIN)
        .unwrap_or_else(|| batch_rows.max(1))
}

/// 一段搬运的结果：整块读完了，还是「事务到量先提交，剩下的接着搬」。
enum SegmentOutcome {
    Finished,
    /// 本段已提交，返回最后一行**完整键**（下一段从它之后开始）；
    /// `next_page` 是**已经飞出去**的下一页预取（见 [`CarriedPage`]）。
    Paused {
        last_key: Vec<CellValue>,
        next_page: Option<tokio::task::JoinHandle<Result<Page, String>>>,
    },
    /// 本段已提交，返回**下一行的绝对行号**（无键表按 OFFSET 续跑）。
    ///
    /// 无键表没有键值可以比较，但 `limit n offset m` 的 `m` 本身就是游标：本段提交后
    /// 下一段从 `m` 继续即可。过去这里没有它，于是无键表被迫「一片一个事务跑到底」——
    /// 中途断一次整片回滚重读（真机：384 万行、无主键无唯一索引的表，断一次就是几十分钟的
    /// 白工）。现在按 [`txn_rows_cap`]（= 界面「每批条数」）正常拆段，断连只重跑当前这一小段。
    ///
    /// 精确性的前提是**源表在同步期间行序不变**（与窗口分片同一个前提）。若源表边写边搬，
    /// OFFSET 会随行序漂移，可能漏行 —— 由收尾的逐表 `count(*)` 对账兜底（不一致清空重传）。
    PausedAt {
        next_offset: u64,
        next_page: Option<tokio::task::JoinHandle<Result<Page, String>>>,
    },
}

/// 段末的预取页要**交给下一段**，不能丢。
///
/// 为什么（真机 `xingwei_sales_vice` 的日志）：事务粒度 = 一页（`txn_rows_cap` = 界面
/// 「每批条数」= 1000，页大小也就被夹在 1000），于是**每页正好是一个段** —— 段末必然
/// 命中 `txn_cap` 当场 `break`，那条已经飞出去（甚至已经读回）的下一页预取被直接丢弃，
/// 下一段再从**同一个游标**重读一遍。132 条分页 SQL 里 66 条是白读的：
/// 日志里成对出现「游标完全相同」的请求，一半的 offset 是上一段残留的 1000
/// （续跑段首屏的 offset 恒为 0 —— 看到这个差值就是预取页被丢了）。
///
/// 代价不只是多一层往返：读侧流量直接翻倍，而预取**本来该省的**「读 N+1 与写 N 重叠」
/// 一点没省下（写完 N 就丢 N+1）。把句柄交给下一段、在其首轮直接 await，等于连提交
/// 那段时间也一起重叠掉，一次读只算一次。
#[allow(clippy::type_complexity)]
type CarriedPage = Option<tokio::task::JoinHandle<Result<Page, String>>>;

/// 本段最终失败时，交接出去的预取已经没人接手：直接掐掉。
///
/// 等价于段末清理里的「读不完就 abort」—— 直接 drop 句柄等于把协程 detach 掉，
/// 它会继续把结果集读完再丢弃，源端会话上挂着一个没人要的长结果集。
fn abort_carried(paused: Option<SegmentOutcome>) {
    let handle = match paused {
        Some(SegmentOutcome::Paused { next_page, .. }) => next_page,
        Some(SegmentOutcome::PausedAt { next_page, .. }) => next_page,
        _ => None,
    };
    if let Some(handle) = handle {
        handle.abort();
    }
}

/// 搬运**一片**：片内按 [`txn_rows_cap`]（= 一批 = 一页）分成多个事务，段与段之间不重不漏
/// （下一段的区间左界 = 上一段最后一行的首列，首屏游标带上完整键走 keyset「严格大于」；
/// 无键表则用 OFFSET 续跑，见 [`SegmentOutcome::PausedAt`]）。
async fn run_slice(ctx: Arc<SliceCtx>, worker: usize, range: RowRange) -> Result<(), String> {
    // 事务粒度由读侧给出（一批 = 一页 = 一个事务）：有键表 = 界面「每批条数」，
    // 无键表 = OFFSET 翻页的大页。片内固定，不随页自适应抖动。
    let cap = txn_rows_cap(ctx.reader.commit_rows());
    let mut low = range.low.clone();
    let mut cursor: Option<Cursor> = None;
    // 上一段提交时**已经飞出去**的下一页预取（见 [`CarriedPage`]）：交给下一段当首屏，
    // 丢一次就等于同一游标白读一遍。
    let mut carried: CarriedPage = None;
    // 无键表按事务提交后要续跑，每段一条日志会把「同步日志」刷成流水账：
    // 只报第一次与之后每 10 次，进度本身由进度卡承担。
    let mut pauses: u32 = 0;
    loop {
        let segment = RowRange {
            index: range.index,
            low: low.clone(),
            high: range.high.clone(),
            window: range.window,
        };
        match run_segment(ctx.clone(), worker, segment, cursor.clone(), carried.take(), cap).await? {
            SegmentOutcome::Finished => return Ok(()),
            SegmentOutcome::Paused { last_key, next_page } => {
                // 区间左界用**首列**（`k1 >= 该值`），首屏游标用**完整键**（`(k1,k2) > 该键`）：
                // 两个条件 AND 在一起正好是「这一行之后」，边界行不会被搬两遍。
                low = last_key.first().cloned();
                cursor = Some(Cursor {
                    key: Some(last_key),
                    offset: 0,
                });
                carried = next_page;
            }
            SegmentOutcome::PausedAt { next_offset, next_page } => {
                // 无键表：`low` 一直是 None（没有键可以抬左界），位置全交给 OFFSET。
                // 段的行号上界仍由 `range.window` 管着，续跑不会越界读到下一段。
                pauses += 1;
                // 阶段进度（无键表每 10 次分段提交报一句）也不进界面：只留结果
                tracing::debug!(
                    table = %ctx.plan.table,
                    next_offset,
                    pauses,
                    "已提交到该位置，继续"
                );
                cursor = Some(Cursor {
                    key: None,
                    offset: next_offset,
                });
                carried = next_page;
            }
        }
    }
}

/// 搬运**一段**：段内串行读写、一个事务；链路级失败回滚并重跑该段。
///
/// `start_page` 是上一段（已提交）交接过来的**首屏预取**：有就直接用，不再重读一遍。
/// 只在**第一次尝试**用得上 —— 失败重试必须从 `start_cursor` 老老实实重读（那一页的行
/// 可能已经跟着回滚一起丢了）。
async fn run_segment(
    ctx: Arc<SliceCtx>,
    worker: usize,
    range: RowRange,
    start_cursor: Option<Cursor>,
    mut start_page: CarriedPage,
    txn_cap: u64,
) -> Result<SegmentOutcome, String> {
    let plan = ctx.plan.clone();
    loop {
        // 本次尝试的计数：失败回滚时要从全局里扣回，否则 failed = 读 - 写 会虚高
        let mut try_inserted = 0u64;
        let mut try_updated = 0u64;
        let mut try_consumed = 0u64;
        // 重试要从这一段的**起点**重读（本段已回滚，不会重复）
        let mut cursor = start_cursor.clone();
        let mut failure: Option<String> = None;
        let mut last_report = std::time::Instant::now();
        let mut paused: Option<SegmentOutcome> = None;

        ctx.writer.begin(worker).await;
        // 读**预取**：写第 N 页的时候，第 N+1 页已经在取了。
        //
        // 依据（5M 单表 4 路实测画像）：读 344s / 写 653s / 合计 258s，两者相加 ≈ 4 路 × 墙钟
        // ⇒ 每路有三分之一墙钟在等源库返回。「读一页 → 写一页」严格串行时这三分之一纯属浪费：
        // 读走源端连接、写走目标端连接，本来就是两条独立的流。
        // A/B（2M 单表，同一份二进制）：预取关 106.9s / 18.7k 行每秒 ⇒ 预取开 83.5s / 24.0k 行每秒，
        // 读耗时 166.7s → 63.6s。
        //
        // **只在独占链路时开**（此刻在跑的表数 ≤ 1）：多表并发时每张表本来就有 2~4 路，
        // 再翻倍等于把在途流数乘二 —— 实测两表（5M + 2M）同时开预取，吞吐从 30.6k 掉到 7k：
        // 丢包链路上流一多就集体退避，反而谁都跑不动。
        //
        // 判据用「在跑的表数」（调度侧每 500ms 刷进 `GrowCaps::active`）而不是「本次任务只
        // 有一张表」：整库同步跑到尾部只剩一张大表时，那正好是它独占链路的时候，预取该开。
        let use_prefetch = prefetch_enabled()
            && plan
                .grow
                .as_ref()
                .map(|caps| caps.active() <= 1)
                .unwrap_or(true);
        // 首屏：上一段交接过来的预取页（`start_page` 只在第一次尝试有值）。
        // 它和本段的「下一页预取」是同一种东西 —— 首轮直接 await，不必再发一次读。
        let mut prefetch: CarriedPage = start_page.take();
        loop {
            if plan
                .task
                .as_ref()
                .map(|t| t.is_canceled())
                .unwrap_or(false)
            {
                break;
            }
            let read_started = std::time::Instant::now();
            let page = match prefetch.take() {
                Some(handle) => match handle.await {
                    Ok(result) => result,
                    // 预读协程自己炸了（panic）：当作一次读失败，交给上面的重试逻辑
                    Err(err) => Err(format!("预读失败：{err}")),
                },
                None => {
                    read_page(
                        &ctx.reader,
                        &range,
                        &cursor,
                        read_timeout(ctx.plan.op_timeout_secs),
                    )
                    .await
                }
            };
            ctx.read_ms
                .fetch_add(read_started.elapsed().as_millis() as u64, Ordering::Relaxed);
            let page = match page {
                Ok(page) => page,
                Err(msg) => {
                    failure = Some(msg);
                    break;
                }
            };
            let next = page.next.clone();
            let mut rows = page.rows;
            // 立刻把**下一屏**的读发出去：下面写这一屏时它就在飞（真正的重叠点）。
            if next.is_some() && use_prefetch {
                let reader = ctx.reader.clone();
                let range_next = range.clone();
                let cursor_next = next.clone();
                let read_limit = read_timeout(ctx.plan.op_timeout_secs);
                prefetch = Some(tokio::spawn(async move {
                    read_page(&reader, &range_next, &cursor_next, read_limit).await
                }));
            }

            if !rows.is_empty() {
                // 行数上限：跨片共享的全局配额，用 CAS 精准领取（不会超发）
                let got = rows.len() as u64;
                let allowed = if plan.row_limit > 0 {
                    let take = take_quota(&ctx.consumed, plan.row_limit, got);
                    if take < got {
                        ctx.limited.store(true, Ordering::SeqCst);
                    }
                    take
                } else {
                    ctx.consumed.fetch_add(got, Ordering::SeqCst);
                    got
                };
                if allowed == 0 {
                    break;
                }
                if allowed < got {
                    rows.truncate(allowed as usize);
                }
                try_consumed += allowed;

                if let Some(task) = &plan.task {
                    // 字节数与行数同步累加：进度卡的「速度」是「行/秒 · MB/s」，
                    // 只记行数那半截永远是 0.0 MB/s（真机截图）。口径与读侧分页的
                    // `estimate_row_bytes` 完全一致，两个数字互相印证。
                    let page_bytes: usize = rows.iter().map(|row| estimate_row_bytes(row)).sum();
                    task.add_bytes_read(page_bytes as u64);
                    task.add_rows_read(allowed);
                    // 只报「执行到哪一步」，行数在进度卡上平滑递增 —— 每页都写会刷屏
                    if last_report.elapsed() >= PROGRESS_INTERVAL {
                        task.set_phase(format!("表 {}：传输数据", plan.table));
                        last_report = std::time::Instant::now();
                    }
                }

                // 两阶段 upsert：已有键 → 更新（攒批、串行），其余 → 插入。
                // 原生 upsert 不走这里 —— 目标端一条语句自己判插入还是更新，行直接进写。
                let mut to_insert = rows;
                if plan.upsert == UpsertMode::TwoPhase && !plan.key_indexes.is_empty() {
                    let existing = match lookup_existing_keys(plan.as_ref(), &to_insert).await {
                        Ok(keys) => keys,
                        Err(msg) => {
                            failure = Some(msg);
                            break;
                        }
                    };
                    let (existing, fresh): (Vec<_>, Vec<_>) = to_insert
                        .into_iter()
                        .partition(|row| existing.contains(&key_of(row, &plan.key_indexes)));
                    if !existing.is_empty() {
                        match ctx.writer.update(&existing).await {
                            Ok(rows) => {
                                try_updated += rows;
                                ctx.updated.fetch_add(rows, Ordering::SeqCst);
                            }
                            Err(msg) => {
                                failure = Some(format!("更新失败：{msg}"));
                                break;
                            }
                        }
                    }
                    to_insert = fresh;
                }

                if !to_insert.is_empty() {
                    let sent = to_insert.len() as u64;
                    let write_started = std::time::Instant::now();
                    let outcome = write_rows(&ctx, worker, &to_insert).await;
                    ctx.write_ms
                        .fetch_add(write_started.elapsed().as_millis() as u64, Ordering::Relaxed);
                    match outcome {
                        Ok(rows) => {
                            // 写入层在「保存点回滚 + 二分重试」路径上可能把同一批的行数
                            // 累计两遍（第一次部分成功、回滚后重试又成功）。返回值超过
                            // 发送行数是没有意义的 —— clamp 掉，否则结果表会显示
                            // 「写入 101 万」而目标表只有 100 万（真机 kill 场景复现）。
                            let written = rows.min(sent);
                            try_inserted += written;
                            ctx.inserted.fetch_add(written, Ordering::SeqCst);
                            // 链路日志第 4 步：第一页真正落库时打一次。
                            // 读和写要各有起点 —— 大表上「读了几分钟」和「在读但没写进去」
                            // 在界面上必须分得开。
                            if written > 0 && !ctx.transfer_told.swap(true, Ordering::SeqCst) {
                                if let Some(task) = &ctx.plan.task {
                                    task.log(format!("表 {}：传输数据到目标表", ctx.plan.table));
                                }
                            }
                        }
                        Err(msg) => {
                            failure = Some(msg);
                            break;
                        }
                    }
                }
            }

            if next.is_none() {
                break; // 这一段读完了
            }
            // 事务到量 ⇒ 提交这一段，剩下的交给下一段（别把一个事务堆到几百万行）
            if txn_cap > 0 && try_inserted + try_updated >= txn_cap {
                // 两条续跑路径：有键的表走 keyset（从这一行**之后**），无键的表走 OFFSET
                // （从下一行的行号）。都能「提交后接着读」，所以无键表也照常拆段 ——
                // 过去它只能一个事务跑到底，断一次就整段重扫。
                if let Some(next_cursor) = next.as_ref() {
                    // 下一页的读（`prefetch`）已经飞出去了、游标就是 `next`：连同句柄一起
                    // 交给下一段，别丢（丢了下一段就从同一游标再读一遍，见 [`CarriedPage`]）。
                    let carried = if carry_enabled() { prefetch.take() } else { None };
                    if let Some(key) = next_cursor.key.clone() {
                        paused = Some(SegmentOutcome::Paused {
                            last_key: key,
                            next_page: carried,
                        });
                        break;
                    }
                    if next_cursor.offset > 0 {
                        paused = Some(SegmentOutcome::PausedAt {
                            next_offset: next_cursor.offset,
                            next_page: carried,
                        });
                        break;
                    }
                }
            }
            cursor = next;
        }

        // 提前退出（取消 / 段满 / 出错 / 读完）时预读协程可能仍在飞：先给它 2 秒收尾
        // （一页正常远小于此，读到了就直接丢掉），收不了再掐 —— 直接 abort 会把一条读
        // 请求砍在半路，源端会话可能留着没读完的结果集。
        if let Some(mut handle) = prefetch.take() {
            if tokio::time::timeout(PREFETCH_GRACE, &mut handle).await.is_err() {
                handle.abort();
            }
        }

        if let Some(msg) = failure {
            // 这一片最终失败：交接出去的预取也不再有人接（下一段不会走），直接掐掉。
            abort_carried(paused.take());
            // 失败 ⇒ 回滚这一片（**绝不提交**），并把本次尝试的计数扣回，
            // 这样「重跑一片」对总数与行数上限都是透明的一次。
            //
            // 回滚也要有墙钟：半开连接上连 `rollback` 都能挂住，以前这里无限等，
            // 整条同步就此卡死（写批超时反倒白设了）。
            // 回滚超时不再单独进界面日志：连接已经不可信（下面 renew 换掉），
            // 失败原因由对象级统一报一条，过程信息一律不写。
            let _ = tokio::time::timeout(ABORT_CALL_TIMEOUT, ctx.writer.abort(worker)).await;
            // 不管回滚成不成，这条连接都不可信了：下一次尝试换新连接
            ctx.writer.renew().await;
            ctx.inserted.fetch_sub(try_inserted, Ordering::SeqCst);
            ctx.updated.fetch_sub(try_updated, Ordering::SeqCst);
            ctx.consumed.fetch_sub(try_consumed, Ordering::SeqCst);
            // 进度上的「已读」**不扣回**：重跑同样要真读一遍，扣回会让数字往回跳
            //（真机反馈「越跳越小」）；写侧当初也没扣，于是两边对不上，
            // 界面出现「传输数 > 读取数」。现在两个计数都按累计工作量单调递增。
            // 段级重跑已关掉（`SLICE_RETRY = 1`）：失败直接上抛，由对象级记一条
            // 「表 X 失败：<原因>」。完整原因进文件日志，界面不再刷「中断 → 回滚 → 重跑」。
            tracing::warn!(
                target: "dbmind::sync",
                table = %plan.table,
                slice = range.index + 1,
                reason = %msg,
                "写段中断"
            );
            return Err(msg);
        }

        // 成功（或用户取消）⇒ 先把攒着的 upsert 更新发出去，再提交这一片
        // 同样是「慢就等、探活不通才判死」：upsert 的更新走另一条串行会话，
        // 它在丢包链路上一样可能几十秒才回来。
        let flushed = wait_write(
            &ctx.plan,
            std::time::Instant::now(),
            "发送攒着的 upsert 更新",
            // upsert 的更新批按 500 条一攒、行数不大：给它 100 秒上限（按 2 万行估），
            // 卡住同样走「回滚这一段 + 换连接重试」（upsert 幂等，重跑不会写重）
            ctx.plan.diag.slow_batch_limit(20_000),
            ctx.writer.flush(),
        )
        .await;
        let flush_err = match flushed {
            Ok(Ok(_)) => None,
            Ok(Err(msg)) => Some(msg),
            Err((reason, _)) => Some(reason),
        };
        if let Some(msg) = flush_err {
            let _ = tokio::time::timeout(ABORT_CALL_TIMEOUT, ctx.writer.abort(worker)).await;
            ctx.writer.renew().await;
            ctx.inserted.fetch_sub(try_inserted, Ordering::SeqCst);
            ctx.updated.fetch_sub(try_updated, Ordering::SeqCst);
            ctx.consumed.fetch_sub(try_consumed, Ordering::SeqCst);
            return Err(msg);
        }
        let commit_limit = commit_timeout(ctx.plan.op_timeout_secs);
        match tokio::time::timeout(commit_limit, ctx.writer.finish(worker)).await {
            Ok(Ok(())) => return Ok(paused.unwrap_or(SegmentOutcome::Finished)),
            // 提交阶段失败：这一段是否落盘已无法确定。**不能重跑**（可能已写入），
            // 如实失败；`WriteDiag::committed` 已由 writer 记下，上层也不会整表重跑。
            Ok(Err(msg)) => return Err(msg),
            Err(_) => {
                return Err(format!(
                    "提交事务 {} 秒未返回（可能已半开）：本段是否落盘无法确定，\
                     不会自动重跑；请核对该表行数后再决定是否重跑",
                    commit_limit.as_secs()
                ));
            }
        }
    }
}

/// 读一页，带退避重试（读是幂等的，重试没有副作用）。
async fn read_page(
    reader: &Arc<dyn super::Reader>,
    range: &RowRange,
    cursor: &Option<Cursor>,
    // 一页最多等多久（跟随界面「超时时间」，见 `read_timeout`）
    hard_limit: Duration,
) -> Result<Page, String> {
    let mut last = String::new();
    for attempt in 0..=READ_RETRY {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(150 * (1 << (attempt - 1)))).await;
        }
        // 源端同样要有墙钟：连接的 socketTimeout=0 时，一条半开的读会一直等到内核
        // 10 分钟的语句超时，界面上就是「读数字一动不动」。一页正常 5 秒内，60 秒足够。
        match tokio::time::timeout(hard_limit, reader.page(range, cursor.clone()))
            .await
            .map_err(|_| {
                format!(
                    "读取一页 {} 秒未返回（源端连接可能已半开 / socketTimeout=0）",
                    hard_limit.as_secs()
                )
            })
            .and_then(|result| result)
        {
            Ok(page) => return Ok(page),
            Err(msg) => {
                // 失败先把页缩小：读超时/链路报错的真因多半是「结果集太大传不回来」
                //（一页几万行 ≈ 几 MB，丢包链路上等不到 ACK）。缩半页再试，
                // 不要拿同一个大页连撞三次，更不要让上层整表重跑。
                reader.shrink_page();
                last = msg;
            }
        }
    }
    Err(last)
}

/// 等一次写动作完成：**慢就等**，只有探活不通（或超过界面「超时时间」）才判死。
///
/// 为什么不再给死线：用户口径是「Navicat / DBX 不中断，我们也不许」。一条语句卡在丢包
/// 重传上要几十秒，这期间链路是好的 —— 按时间判死只会把「慢」变成「中断 + 整段重跑」。
/// 探活的代价是每 [`BATCH_PROBE_TICK`] 一次 `select 1`（另开会话，不排在卡住的语句后面）。
///
/// **慢批自愈**（`stall`）：探活一直通、但这一页写了 `stall`（60~180 秒）还没完成时，
/// 判定「目标端写入卡住」（后期大表 + 目标端 IO 饱和时的典型表现）—— 主动放弃这一页、
/// 换新连接重试，而不是一直等到界面的「超时时间」（默认继承连接 = 600 秒）。
/// 真机代价：400 对象的任务跑到后段，几乎每张表都白等 10 分钟；中断后页会缩到 1/4，
/// 重试通常几十秒就过（数据不会重复：这一页的事务已回滚）。
/// 用户把「超时时间」设成不限制（<0）时**不启用**自愈 —— 那是明确的「一直等」。
///
/// 返回 `Err((原因, 是否链路整体不可用))`：第二个值决定要不要让整个任务暂停。
async fn wait_write<F, T>(
    plan: &TablePlan,
    started: std::time::Instant,
    label: &str,
    stall: Duration,
    pending: F,
) -> Result<T, (String, bool)>
where
    F: std::future::Future<Output = T>,
{
    // 界面的「超时时间」（对齐 DBX）：0 = 继承连接（内核默认 600 秒），<0 = 不限制，
    // >0 = 用户填的秒数。探活仍然每 30 秒做一次 —— 它决定「断没断」，这个上限只决定
    // 「最多等多久」，两者不是一回事。
    let hard_limit = Duration::from_millis(crate::api::sync::op_timeout_ms(plan.op_timeout_secs));
    // 先到先算：用户填的比自愈阈值更短就听用户的（那是更严格的「操作超时」）。
    let soft_limit = if plan.op_timeout_secs < 0 {
        // 界面上明确选了「不限制」：不自愈，继续等（探活仍照常做）
        Duration::from_secs(86_400)
    } else {
        hard_limit.min(stall)
    };
    // 界面日志节流：卡住期间每 2 分钟说一句「还在写」，别让用户以为假死
    let mut last_note = started;
    // 探活节流：判断「断没断」是 30 秒一次的 select 1；软阈值只比较本地时钟，2 秒一次
    let mut last_probe = started;
    let link_broken = |elapsed: Duration| format!("{label}超时 {} 秒", elapsed.as_secs());
    tokio::pin!(pending);
    loop {
        tokio::select! {
            value = &mut pending => return Ok(value),
            _ = tokio::time::sleep(SLOW_CHECK_TICK) => {
                let elapsed = started.elapsed();
                if elapsed >= soft_limit {
                    // 到软阈值先复核链路：真断了就按「链路已断」报（让整个任务停一会儿）
                    if !probe_target_confirmed(plan).await {
                        return Err((link_broken(elapsed), true));
                    }
                    // 探活正常 ⇒ 不是链路问题，是目标端写不动（IO 饱和 / 长事务 / 刷盘 / 锁等待）。
                    // 日志只报「超时多少秒」这一件事实：行数、页大小、上限、是否探活通过
                    // 一律不写（用户口径：这些内部量他没法用，也不想知道）。
                    let text = format!("{label}超时 {} 秒", elapsed.as_secs());
                    return Err((text, false));
                }
                // 定期探活：只为判断「断没断」（不做软阈值判断，那个上面已经按 2 秒精度看过）
                if last_probe.elapsed() >= BATCH_PROBE_TICK {
                    last_probe = std::time::Instant::now();
                    if !probe_target_confirmed(plan).await {
                        return Err((link_broken(elapsed), true));
                    }
                    // 卡住期间在界面上留个脚印（每 2 分钟一条，不刷屏）
                    if elapsed.as_secs() >= 60 && last_note.elapsed() >= Duration::from_secs(120) {
                        last_note = std::time::Instant::now();
                        if let Some(task) = &plan.task {
                            task.log(format!(
                                "{label}：目标端已写 {} 秒（探活正常，仍在等待；超过 {} 秒会自动换连接重试）",
                                elapsed.as_secs(),
                                soft_limit.as_secs()
                            ));
                        }
                    }
                    tracing::debug!(
                        target: "dbmind::sync",
                        table = %plan.table,
                        elapsed_ms = elapsed.as_millis() as u64,
                        what = label,
                        "写入仍在进行（探活通过），继续等待"
                    );
                }
            }
        }
    }
}

/// 写一批（探活式等待 + 写速诊断）。
async fn write_rows(
    ctx: &SliceCtx,
    worker: usize,
    rows: &[Vec<CellValue>],
) -> Result<u64, String> {
    // 任务里刚有别的表探活失败（链路整体不可用）⇒ 先按暂停窗口让一让。
    // 不这么做的话，几百张表会在同一秒各自换连接、各自回滚一段再各自报错。
    if let Some(grow) = &ctx.plan.grow {
        let left = grow.link_hold_left_ms();
        if left > 0 {
            tokio::time::sleep(Duration::from_millis(left.min(LINK_HOLD_MS))).await;
        }
    }
    let started = std::time::Instant::now();
    ctx.plan.diag.note_batch_start(rows.len());
    ctx.plan.diag.inflight.fetch_add(1, Ordering::Relaxed);
    // 这一页的**慢批自愈**阈值：探活一直通、但写到这个时长还没完成 ⇒ 中断重试
    // （见 `wait_write` 的 `stall` 参数；上界 180 秒，而不是界面的 600 秒）。
    //
    // 只在**能从服务端掐断会话**的方言上启用：客户端放弃等待后，服务端那条 INSERT
    // 和它的未提交事务还在跑、还持有行锁，掐不断它的话重试会排在自己的锁后面
    //（1 秒阈值注入实测：页从 1 万缩到 1000 行仍然每批都超时，任务彻底不动）。
    // 掐不断的方言保持旧语义（探活通就一直等），不引入更坏的结果。
    let target_spec = super::plugins::spec_data_for(ctx.plan.target_kind.kind.key());
    let stall = if target_spec.has_kill() {
        // **第一页就卡住**是「目标端/链路此刻有问题」的强信号（真机后期几乎每张表都是
        // 第一页卡满 601 秒，而前期几百万行的表都不卡 —— 卡的从来不是行数，是时刻）。
        // 这时没必要陪它等 60~180 秒，30 秒就换连接重试，让任务尽快回到正常节奏。
        let base = ctx.plan.diag.slow_batch_limit(rows.len());
        if ctx.plan.diag.batches_done.load(Ordering::Relaxed) == 0 {
            base.min(Duration::from_secs(30))
        } else {
            base
        }
    } else {
        Duration::from_secs(86_400)
    };
    // ---- 限速（界面「限速」：行/秒 + MB/秒，两条曲线取更慢的）----
    //
    // 放在**发语句之前**：事后补睡的话突发流量已经打到目标库了，限速就只是自欺欺人。
    // 时间表是全任务共享的（`pacer`），所以多路 writer / 多张表并发的**合计**速率等于
    // 用户填的值；目标端本来就比限速慢时 `pace` 一行都不睡，不会把任务拖得更慢。
    if let Some(pacer) = &ctx.plan.pacer {
        pacer
            .pace(rows.len() as u64, super::pacer::page_bytes(rows))
            .await;
    }
    // 这一批**不再按时间判死**：每 [`BATCH_PROBE_TICK`] 秒探活一次，
    // 通就继续等（慢 ≠ 断），不通才判链路已断 —— 见 `wait_write`。
    let (outcome, timed_out) = match wait_write(
        &ctx.plan,
        started,
        // 文案只说「写入超时 N 秒」：行数/页大小/报文上限都是内部量，用户看了只会问
        // 「为什么是 1 万不是我的 1000」，一个都不往日志里写。
        "写入",
        stall,
        ctx.writer.write(worker, rows),
    )
    .await
    {
        Ok(outcome) => (outcome, false),
        Err((reason, link_down)) => {
            // **先从服务端掐断那条卡住的会话**（MySQL 系）：客户端放弃等待后，
            // 服务端那条语句和未提交事务还在跑、还持着行锁，重试会排在自己的锁后面。
            // 必须在 renew 之前做（renew 换代后 conn_ids 就清空了，旧的 id 找不回来）。
            let killed = ctx.writer.kill_sessions().await;
            // 任务级「踩坑即收敛」：把本任务的页上限压到 1 万行，后面开工的表
            // 直接从小的起步 —— 真机上后期是「几乎每张表都卡一次」，因为每张表
            // 都在用自己的第一页重新踩同一个坑（大页 + 目标端掉速）。
            if !link_down {
                if let Some(grow) = &ctx.plan.grow {
                    grow.note_page_cap(10_000);
                }
            }
            // 半开连接上那条语句还挂在宿主里，**必须换新连接**再重试，
            // 否则下一次调用复用它、再挂一次（真机表现为「每张表都卡一次」）。
            // 同时把**单条语句的字节上限**减半：劣化链路（PMTU 黑洞 / 静默丢包）上大报文
            // 会一直撞重传退避 —— 报文不小下去，重试只是「再等一次同样的超时」。
            let (old_cap, new_cap) = ctx.writer.shrink_stmt(worker);
            ctx.writer.renew().await;
            // **页也要缩**：目标端写不动时（后期大表 + IO 饱和），一页几万行的大事务
            // 只会继续卡；缩到 1/4 再试，单批体量与事务大小一起降下来。
            // 读侧幂等，缩页没有任何副作用（链路恢复后由自适应慢慢涨回去）。
            ctx.reader.shrink_page();
            // 上面这些动作（掐会话、缩页、换连接、报文上限收敛）照做，但**一个字都不写进日志**：
            // 用户要的是失败原因本身（`reason` = 「写入超时 31 秒」或数据库原始报错），
            // 「已断开卡住的会话 / 已换连接重试」这类过程信息只会把真正的原因埋掉。
            let _ = killed;
            let _ = (old_cap, new_cap);
            // 链路整体不可用（探活不通）⇒ 让整个任务一起停一会儿：
            // 不这么做的话，几百张表会在同一秒各自换连接、各自回滚一段再各自报错。
            if link_down {
                if let Some(grow) = &ctx.plan.grow {
                    grow.hold_link(LINK_HOLD_MS);
                }
            }
            (Err(reason), true)
        }
    };
    // `inflight` 的加减由 diag 的收尾方法负责 —— 这里曾再减一次，减成负数（u64 回绕）
    if timed_out {
        ctx.plan.diag.note_batch_timeout(rows.len(), started.elapsed());
    } else {
        ctx.plan.diag.note_batch_done(rows.len(), started.elapsed());
    }
    outcome
}

/// 目标端探活：**另开一条会话**跑 `select 1`，5 秒不回就算不通。
///
/// `false` = 连一条最简单的查询都回不来 ⇒ 这不是「某条连接半开」，而是链路/目标整体
/// 不可用。调用方据此让整个任务退避（见 [`LINK_HOLD_MS`]），而不是几百张表各自重连。
///
/// 会话名按表隔离（`internal:sync-probe{sid}`）：内核的会话泳道是「同键一条物理连接」，
/// 全任务共用一个键时所有探活会排队，链路真挂的时候反而互相拖住。
async fn probe_target(plan: &TablePlan) -> bool {
    let session = format!("internal:sync-probe{}", plan.sid);
    matches!(
        tokio::time::timeout(
            PROBE_TIMEOUT,
            crate::api::sync::run_target_in(&plan.state, &plan.tgt_target, "select 1", &session),
        )
        .await,
        Ok(Ok(_))
    )
}

/// 判活 + **一次复核**：判「断」是要付代价的（回滚一段、换连接重跑），
/// 不能让链路抖一下（丢个包、RDS 抽一下）就触发。
///
/// 第一次不通就隔 1 秒再探一次，两次都不通才算真断；
/// 单次抖动最多多花 `PROBE_TIMEOUT + 1 秒`，换的是「不会白回滚」。
async fn probe_target_confirmed(plan: &TablePlan) -> bool {
    if probe_target(plan).await {
        return true;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    probe_target(plan).await
}

/// 原子地领取行数上限配额：`consumed` 永远不会被推到 `limit` 之后。
fn take_quota(consumed: &AtomicU64, limit: u64, want: u64) -> u64 {
    let mut current = consumed.load(Ordering::SeqCst);
    loop {
        let remaining = limit.saturating_sub(current);
        let take = want.min(remaining);
        if take == 0 {
            return 0;
        }
        match consumed.compare_exchange(
            current,
            current + take,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => return take,
            Err(now) => current = now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Slots;

    /// 尾部弹性的核心不变量：令牌**只增不减**、补发不超过上限、编号不重复。
    /// 补错了的表现是「并发卡在初始值」或者「同一通道被两块同时用」——
    /// 后者会让两条数据流挤在同一条事务里，脏得无声无息。
    #[test]
    fn 令牌池只增不减且不超上限() {
        let slots = Slots::new(2, 4);
        assert_eq!(slots.try_acquire(), Some(1));
        assert_eq!(slots.try_acquire(), Some(0));
        assert_eq!(slots.try_acquire(), None);
        // 补到 3 路
        assert_eq!(slots.grow_to(3), 1);
        assert_eq!(slots.try_acquire(), Some(2));
        // 超过上限的请求被夹到 limit=4
        assert_eq!(slots.grow_to(9), 1);
        assert_eq!(slots.try_acquire(), Some(3));
        assert_eq!(slots.grow_to(9), 0);
        // 还回去的号可以再领，且不会多出第 5 个号
        for id in 0..4 {
            slots.release(id);
        }
        for _ in 0..4 {
            assert!(slots.try_acquire().is_some());
        }
        assert_eq!(slots.try_acquire(), None);
    }

    /// 初始并发夹在 1..=limit：单连接池的库（limit=1）不能因为份额小而变成 0 路（不搬数据）
    #[test]
    fn 初始令牌至少一路且不超过上限() {
        let slots = Slots::new(0, 1);
        assert_eq!(slots.try_acquire(), Some(0));
        assert_eq!(slots.try_acquire(), None);
        assert_eq!(slots.grow_to(4), 0);
    }
}
