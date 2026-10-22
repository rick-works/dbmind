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
//! - 片内再按 [`TXN_ROWS_DEFAULT`] 拆成多个事务：一个事务几百万行会让 undo/redo 膨胀、
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
use crate::api::sync::{is_link_level_message, key_of, lookup_existing_keys};

/// 单页失败后的重试次数（退避 150ms / 300ms / 600ms）。
///
/// 只对**读**做重试：读是幂等的，重试没有副作用；写不做整批重试（一条语句可能已经
/// 落了一半），而是交给写入层的「语句失败 → 二分拆批定位坏行」兜住。
const READ_RETRY: usize = 3;

/// 一片最多重跑几次（片级回滚后重跑，不重复不丢）。
///
/// 片级重跑很便宜（一片 = 1/N 张表），所以给 3 次：链路在尾部常常「连断两次」——
/// 代理会成批回收空闲会话，重连后第一条语句又撞上另一条死连接。3 次之后仍断，
/// 才判定这一片真的搬不上去。
const SLICE_RETRY: usize = 3;

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

/// 单批写入的**墙钟**上限。
///
/// 为什么必须有：连接的 `socketTimeout=0`（真机上好几条 RDS 连接的高级参数里就是这么写的）
/// 是**永不超时** —— 目标端把连接半开（RDS 常见的 wait_timeout 踢连接 / 中间设备静默丢包）时，
/// 驱动会一直等一个永远不来的响应：`show processlist` 里什么都看不到、也不报错，
/// 只有整条同步「卡住、一行都不写、失败数还是 0」（真机 400 张表跑到 4/400，写出 1536 行）。
/// 这里用墙钟兜底，把「永远等」变成「这一片失败、其它片继续」，并在错误里点名 socketTimeout。
const WRITE_CALL_TIMEOUT: Duration = Duration::from_secs(300);

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
    if slices > 1 {
        if let Some(task) = &plan.task {
            task.log(format!(
                "表 {}：按主键切成 {slices} 块（{start_parallel} 路并发，尾部可扩到 {limit} 路；\
                 每块独立游标 + 独立事务，页 {} 行）",
                plan.table,
                reader_plugin.page_rows()
            ));
        }
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
    });

    // ---- 3. 块队列 + 令牌池（读 → 写 → 提交；失败回滚重跑该块）-------------
    //
    // 任务数 = 块数（多于初始并发），实际同时跑几块由令牌池决定：这样尾部补发令牌时，
    // 早已就位、正在等令牌的任务立刻就能领块开跑，不需要再 spawn（也就不需要重切表）。
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
                // 读会话按**通道号**而不是块号：块换一个就换一条连接的话，几十个块会把
                // 会话池堆满（每个块号一条空闲连接）。通道号恒 ≤ limit，读连接就固定这几条。
                range.index = worker;
                let outcome = run_slice(ctx.clone(), worker, range).await;
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
                    if let Some(task) = &plan.task {
                        task.log(format!(
                            "表 {}：其它对象已结束，并发扩到 {target} 路（读片与写连接同步增加）",
                            plan.table
                        ));
                    }
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
                        // 真卡死了：掐掉协程，别让它挂着占连接
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
                        "传输协程 {} 分钟没有任何进展（连接可能已半开），本次同步数据不完整",
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
            let lanes = slots.issued.load(Ordering::SeqCst).max(1);
            task.log(format!(
                "表 {}：耗时画像 —— 读 {:.1}s / 写 {:.1}s / 合计 {:.1}s，{rate} 行/秒\
                 （{lanes} 路并发，页 {} 行）",
                plan.table,
                read_spent as f64 / 1_000.0,
                write_spent as f64 / 1_000.0,
                total_ms as f64 / 1_000.0,
                ctx.reader.page_rows(),
            ));
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

/// 单个事务最多写多少行（到量就提交一次，接着往下搬）。
///
/// 为什么必须封顶：原来「一个块一个事务」，5000 万行切成 8 块 = **每个事务 625 万行**。
/// 大事务会让 undo/redo 一路膨胀、history list 变长、checkpoint 追不上刷盘，表现就是
/// 「越写越慢」（实测 5000 万行从 23.8k 行/秒掉到 18.6k）。DataX（batchSize 默认 2048）
/// 和 Kettle（commit size 默认 1000）都是**小事务高频提交**，这是它们不会越跑越慢的主因。
///
/// 50 万行 ≈ 6~8 秒一批（按 18k 行/秒）：足够摊薄提交开销，又不会把事务堆成灾难。
/// `DBMIND_SYNC_TXN_ROWS` 可覆盖（对比实验用）。
const TXN_ROWS_DEFAULT: u64 = 500_000;
const TXN_ROWS_MIN: u64 = 10_000;

fn txn_rows_cap() -> u64 {
    std::env::var("DBMIND_SYNC_TXN_ROWS")
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .filter(|rows| *rows >= TXN_ROWS_MIN)
        .unwrap_or(TXN_ROWS_DEFAULT)
}

/// 一段搬运的结果：整块读完了，还是「事务到量先提交，剩下的接着搬」。
enum SegmentOutcome {
    Finished,
    /// 本段已提交，返回最后一行**完整键**（下一段从它之后开始）
    Paused(Vec<CellValue>),
}

/// 搬运**一片**：片内按 [`TXN_ROWS_DEFAULT`] 分成多个事务，段与段之间不重不漏
/// （下一段的区间左界 = 上一段最后一行的首列，首屏游标带上完整键走 keyset「严格大于」）。
async fn run_slice(ctx: Arc<SliceCtx>, worker: usize, range: RowRange) -> Result<(), String> {
    let cap = txn_rows_cap();
    let mut low = range.low.clone();
    let mut cursor: Option<Cursor> = None;
    loop {
        let segment = RowRange {
            index: range.index,
            low: low.clone(),
            high: range.high.clone(),
        };
        match run_segment(ctx.clone(), worker, segment, cursor.clone(), cap).await? {
            SegmentOutcome::Finished => return Ok(()),
            SegmentOutcome::Paused(last_key) => {
                // 区间左界用**首列**（`k1 >= 该值`），首屏游标用**完整键**（`(k1,k2) > 该键`）：
                // 两个条件 AND 在一起正好是「这一行之后」，边界行不会被搬两遍。
                low = last_key.first().cloned();
                cursor = Some(Cursor {
                    key: Some(last_key),
                    offset: 0,
                });
            }
        }
    }
}

/// 搬运**一段**：段内串行读写、一个事务；链路级失败回滚并重跑该段。
async fn run_segment(
    ctx: Arc<SliceCtx>,
    worker: usize,
    range: RowRange,
    start_cursor: Option<Cursor>,
    txn_cap: u64,
) -> Result<SegmentOutcome, String> {
    let plan = ctx.plan.clone();
    let mut attempt = 0usize;
    loop {
        // 本次尝试的计数：失败回滚时要从全局里扣回，否则 failed = 读 - 写 会虚高
        let mut try_inserted = 0u64;
        let mut try_updated = 0u64;
        let mut try_consumed = 0u64;
        let mut canceled = false;
        // 重试要从这一段的**起点**重读（本段已回滚，不会重复）
        let mut cursor = start_cursor.clone();
        let mut failure: Option<String> = None;
        let mut last_report = std::time::Instant::now();
        let mut paused: Option<Vec<CellValue>> = None;

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
        let mut prefetch: Option<tokio::task::JoinHandle<Result<Page, String>>> = None;
        loop {
            if plan
                .task
                .as_ref()
                .map(|t| t.is_canceled())
                .unwrap_or(false)
            {
                canceled = true;
                break;
            }
            let read_started = std::time::Instant::now();
            let page = match prefetch.take() {
                Some(handle) => match handle.await {
                    Ok(result) => result,
                    // 预读协程自己炸了（panic）：当作一次读失败，交给上面的重试逻辑
                    Err(err) => Err(format!("预读失败：{err}")),
                },
                None => read_page(&ctx.reader, &range, &cursor).await,
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
                prefetch = Some(tokio::spawn(async move {
                    read_page(&reader, &range_next, &cursor_next).await
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
                // 只有**有键**的表才能「从这一行之后接着读」；无键表只有 OFFSET，
                // 中途断开就要从头重扫，所以干脆不拆段（保持一个事务到底）。
                if let Some(key) = next.as_ref().and_then(|cursor| cursor.key.clone()) {
                    paused = Some(key);
                    break;
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
            // 失败 ⇒ 回滚这一片（**绝不提交**），并把本次尝试的计数扣回，
            // 这样「重跑一片」对总数与行数上限都是透明的一次
            ctx.writer.abort(worker).await;
            ctx.inserted.fetch_sub(try_inserted, Ordering::SeqCst);
            ctx.updated.fetch_sub(try_updated, Ordering::SeqCst);
            ctx.consumed.fetch_sub(try_consumed, Ordering::SeqCst);
            // 进度上的「已读」也要扣回：同一个片会被再读一遍，不扣就虚高
            if let Some(task) = &plan.task {
                task.sub_rows_read(try_consumed);
            }
            attempt += 1;
            let retryable = attempt < SLICE_RETRY
                && !canceled
                && !plan.diag.begin_failed.load(Ordering::SeqCst)
                && is_link_level_message(&msg);
            if !retryable {
                return Err(msg);
            }
            if let Some(task) = &plan.task {
                task.log(format!(
                    "表 {}（片 {}）：写入中断（{msg}）；本段事务已回滚，自动重连重跑该段（第 {}/{SLICE_RETRY} 次）",
                    plan.table,
                    range.index + 1,
                    attempt,
                ));
            }
            // 退避 1s / 2s：链路真的不行时别把两次尝试挤在同一秒里再撞一次
            tokio::time::sleep(Duration::from_secs(1u64 << (attempt - 1))).await;
            continue;
        }

        // 成功（或用户取消）⇒ 先把攒着的 upsert 更新发出去，再提交这一片
        if let Err(msg) = ctx.writer.flush().await {
            ctx.writer.abort(worker).await;
            ctx.inserted.fetch_sub(try_inserted, Ordering::SeqCst);
            ctx.updated.fetch_sub(try_updated, Ordering::SeqCst);
            ctx.consumed.fetch_sub(try_consumed, Ordering::SeqCst);
            return Err(msg);
        }
        match ctx.writer.finish(worker).await {
            Ok(()) => {
                return Ok(match paused {
                    Some(key) => SegmentOutcome::Paused(key),
                    None => SegmentOutcome::Finished,
                });
            }
            Err(msg) => {
                // 提交阶段失败：这一段是否落盘已无法确定。**不能重跑**（可能已写入），
                // 如实失败；`WriteDiag::committed` 已由 writer 记下，上层也不会整表重跑。
                return Err(msg);
            }
        }
    }
}

/// 读一页，带退避重试（读是幂等的，重试没有副作用）。
async fn read_page(
    reader: &Arc<dyn super::Reader>,
    range: &RowRange,
    cursor: &Option<Cursor>,
) -> Result<Page, String> {
    let mut last = String::new();
    for attempt in 0..=READ_RETRY {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(150 * (1 << (attempt - 1)))).await;
        }
        match reader.page(range, cursor.clone()).await {
            Ok(page) => return Ok(page),
            Err(msg) => last = msg,
        }
    }
    Err(last)
}

/// 写一批（带墙钟兜底与写速诊断）。
async fn write_rows(
    ctx: &SliceCtx,
    worker: usize,
    rows: &[Vec<CellValue>],
) -> Result<u64, String> {
    let started = std::time::Instant::now();
    ctx.plan.diag.note_batch_start(rows.len());
    ctx.plan.diag.inflight.fetch_add(1, Ordering::Relaxed);
    let outcome = match tokio::time::timeout(
        WRITE_CALL_TIMEOUT,
        ctx.writer.write(worker, rows),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(_) => Err(format!(
            "单批 {} 行写入 {} 秒仍未返回：目标端连接可能已经半开（RDS 踢连接 / 链路静默丢包），\
             或该连接的 socketTimeout=0（永不超时）。请在「编辑数据源 → 高级参数」里把 \
             socketTimeout 设为 600、writeTimeout 设为 300 后重试",
            rows.len(),
            WRITE_CALL_TIMEOUT.as_secs()
        )),
    };
    ctx.plan.diag.inflight.fetch_sub(1, Ordering::Relaxed);
    ctx.plan.diag.note_batch_done(rows.len(), started.elapsed());
    outcome
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
