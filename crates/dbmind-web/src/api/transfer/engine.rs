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

/// 单批写入的墙钟上限：由 [`WriteDiag::batch_watchdog`] 按现场自适应（60s ~ 300s）。
///
/// 为什么必须有：连接的 `socketTimeout=0`（真机上好几条 RDS 连接的高级参数里就是这么写的）
/// 是**永不超时** —— 目标端把连接半开（RDS 常见的 wait_timeout 踢连接 / 中间设备静默丢包）时，
/// 驱动会一直等一个永远不来的响应：`show processlist` 里什么都看不到、也不报错，
/// 只有整条同步「卡住、一行都不写、失败数还是 0」。
///
/// 为什么不能一刀切 300 秒：真机一张 400 对象的任务跑到尾部，几乎每张表都撞一次，
/// 于是「五分钟卡一次」地循环 —— 20 秒判定 + 弃用这条连接 + 重连重跑，代价小得多。
const WRITE_CALL_TIMEOUT_MAX: Duration = Duration::from_secs(300);

/// 回滚/收尾语句自己的墙钟：半开连接上连 `rollback` 都会挂住（真机整条同步就此卡死）。
const ABORT_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// 读一页的墙钟：正常页 <5 秒（32MiB 大页也就 5~6 秒），30 秒足够判定「这条读永远回不来」。
///
/// 为什么从 60 秒收到 30 秒：半开连接上这一次等待是**纯浪费** —— 真机整库任务里慢表
/// 按 60 秒一撞、连撞两三次，单表就烧掉半小时（re_sale 读 2256 秒只前进几万行）。
/// 而链路正常时一页的字节上限是 `PAGE_BYTES_TARGET`（32 MiB），按实测 4 MB/s 约 8 秒，
/// 30 秒留了近 4 倍余量，不会误杀正常大页。
const READ_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// 提交（commit）的墙钟：几十万行的事务提交慢是正常的，但不能无限等 ——
/// 半开连接上 commit 会永远不返回，而「提交过没」是决定能不能重跑的关键。
const COMMIT_CALL_TIMEOUT: Duration = Duration::from_secs(180);

/// 写超时后的**目标端探活**上限（另开一条会话跑 `select 1`）。
///
/// 为什么要探：墙钟只能说明「这条语句没回来」，分不清两种完全不同的故障 ——
/// 一条连接半开（换连接就行），还是整条链路/目标不可用（换连接只会再撞一次）。
/// 探活失败 ⇒ 后者，此时该让**整个任务**停一会儿，而不是几百张表一起重连重跑。
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

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
            // 链路日志第 5 步（终点）：这张表搬完，多少行、花了多久。
            // 读/写分解、并发路数属于调参信息，进 tracing 就够了。
            task.log(format!(
                "表 {}：传输完成，共 {}，耗时 {}",
                plan.table,
                super::万行(done_rows),
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
    /// 本段已提交，返回**下一行的绝对行号**（无键表按 OFFSET 续跑）。
    ///
    /// 无键表没有键值可以比较，但 `limit n offset m` 的 `m` 本身就是游标：本段提交后
    /// 下一段从 `m` 继续即可。过去这里没有它，于是无键表被迫「一片一个事务跑到底」——
    /// 中途断一次整片回滚重读（真机：384 万行、无主键无唯一索引的表，断一次就是几十分钟的
    /// 白工）。现在按 [`TXN_ROWS_DEFAULT`] 正常拆段，断连只重跑当前这一小段。
    ///
    /// 精确性的前提是**源表在同步期间行序不变**（与窗口分片同一个前提）。若源表边写边搬，
    /// OFFSET 会随行序漂移，可能漏行 —— 由收尾的逐表 `count(*)` 对账兜底（不一致清空重传）。
    PausedAt(u64),
}

/// 搬运**一片**：片内按 [`TXN_ROWS_DEFAULT`] 分成多个事务，段与段之间不重不漏
/// （下一段的区间左界 = 上一段最后一行的首列，首屏游标带上完整键走 keyset「严格大于」；
/// 无键表则用 OFFSET 续跑，见 [`SegmentOutcome::PausedAt`]）。
async fn run_slice(ctx: Arc<SliceCtx>, worker: usize, range: RowRange) -> Result<(), String> {
    let cap = txn_rows_cap();
    let mut low = range.low.clone();
    let mut cursor: Option<Cursor> = None;
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
            SegmentOutcome::PausedAt(next_offset) => {
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
                    if let Some(key) = next_cursor.key.clone() {
                        paused = Some(SegmentOutcome::Paused(key));
                        break;
                    }
                    if next_cursor.offset > 0 {
                        paused = Some(SegmentOutcome::PausedAt(next_cursor.offset));
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
            // 失败 ⇒ 回滚这一片（**绝不提交**），并把本次尝试的计数扣回，
            // 这样「重跑一片」对总数与行数上限都是透明的一次。
            //
            // 回滚也要有墙钟：半开连接上连 `rollback` 都能挂住，以前这里无限等，
            // 整条同步就此卡死（写批超时反倒白设了）。
            if tokio::time::timeout(ABORT_CALL_TIMEOUT, ctx.writer.abort(worker))
                .await
                .is_err()
            {
                if let Some(task) = &plan.task {
                    task.log(format!(
                        "表 {}：回滚 {} 秒未返回，连接已弃用（服务端会自行回滚未提交事务）",
                        plan.table,
                        ABORT_CALL_TIMEOUT.as_secs()
                    ));
                }
            }
            // 不管回滚成不成，这条连接都不可信了：下一次尝试换新连接
            ctx.writer.renew().await;
            ctx.inserted.fetch_sub(try_inserted, Ordering::SeqCst);
            ctx.updated.fetch_sub(try_updated, Ordering::SeqCst);
            ctx.consumed.fetch_sub(try_consumed, Ordering::SeqCst);
            // 进度上的「已读」**不扣回**：重跑同样要真读一遍，扣回会让数字往回跳
            //（真机反馈「越跳越小」）；写侧当初也没扣，于是两边对不上，
            // 界面出现「传输数 > 读取数」。现在两个计数都按累计工作量单调递增。
            attempt += 1;
            let retryable = attempt < SLICE_RETRY
                && !canceled
                && !plan.diag.begin_failed.load(Ordering::SeqCst)
                && is_link_level_message(&msg);
            if !retryable {
                return Err(msg);
            }
            // 界面日志：第一次把原因说全，之后同类重试只说「又重试了一次」——
            // 同一段连续重试时把那一长串半开解释重复三遍，会把真正重要的行淹掉
            //（用户反馈「这种提示一直在刷」）。完整原因每次仍进文件日志。
            tracing::warn!(
                target: "dbmind::sync",
                table = %plan.table,
                slice = range.index + 1,
                attempt,
                reason = %msg,
                "写段中断，自动重跑该段"
            );
            if let Some(task) = &plan.task {
                if attempt == 1 {
                    task.log(format!(
                        "表 {}（片 {}）：写入中断（{msg}）；本段事务已回滚，自动换连接重跑该段\
                         （第 {attempt}/{SLICE_RETRY} 次，数据会重新写入，无需处理）",
                        plan.table,
                        range.index + 1,
                    ));
                } else {
                    task.log(format!(
                        "表 {}（片 {}）：再次中断，已重跑该段（第 {attempt}/{SLICE_RETRY} 次，无需处理）",
                        plan.table,
                        range.index + 1,
                    ));
                }
            }
            // 退避 1s / 2s：链路真的不行时别把两次尝试挤在同一秒里再撞一次
            tokio::time::sleep(Duration::from_secs(1u64 << (attempt - 1))).await;
            continue;
        }

        // 成功（或用户取消）⇒ 先把攒着的 upsert 更新发出去，再提交这一片
        let flushed =
            tokio::time::timeout(ctx.plan.diag.batch_watchdog(1024), ctx.writer.flush()).await;
        let flush_err = match flushed {
            Ok(Ok(_)) => None,
            Ok(Err(msg)) => Some(msg),
            Err(_) => Some("发送攒着的 upsert 更新超时（目标端连接可能已半开）".to_string()),
        };
        if let Some(msg) = flush_err {
            let _ = tokio::time::timeout(ABORT_CALL_TIMEOUT, ctx.writer.abort(worker)).await;
            ctx.writer.renew().await;
            ctx.inserted.fetch_sub(try_inserted, Ordering::SeqCst);
            ctx.updated.fetch_sub(try_updated, Ordering::SeqCst);
            ctx.consumed.fetch_sub(try_consumed, Ordering::SeqCst);
            return Err(msg);
        }
        match tokio::time::timeout(COMMIT_CALL_TIMEOUT, ctx.writer.finish(worker)).await {
            Ok(Ok(())) => return Ok(paused.unwrap_or(SegmentOutcome::Finished)),
            // 提交阶段失败：这一段是否落盘已无法确定。**不能重跑**（可能已写入），
            // 如实失败；`WriteDiag::committed` 已由 writer 记下，上层也不会整表重跑。
            Ok(Err(msg)) => return Err(msg),
            Err(_) => {
                return Err(format!(
                    "提交事务 {} 秒未返回（目标端连接可能已半开）：本段是否落盘无法确定，\
                     不会自动重跑；请核对该表行数后再决定是否重跑",
                    COMMIT_CALL_TIMEOUT.as_secs()
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
) -> Result<Page, String> {
    let mut last = String::new();
    for attempt in 0..=READ_RETRY {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(150 * (1 << (attempt - 1)))).await;
        }
        // 源端同样要有墙钟：连接的 socketTimeout=0 时，一条半开的读会一直等到内核
        // 10 分钟的语句超时，界面上就是「读数字一动不动」。一页正常 5 秒内，60 秒足够。
        match tokio::time::timeout(READ_CALL_TIMEOUT, reader.page(range, cursor.clone()))
            .await
            .map_err(|_| {
                format!(
                    "读取一页 {} 秒未返回（源端连接可能已半开 / socketTimeout=0）",
                    READ_CALL_TIMEOUT.as_secs()
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

/// 写一批（带墙钟兜底与写速诊断）。
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
    // 阈值跟着这张表的现场走：正常 1 秒一批 ⇒ 20 秒就判定；真慢的批（十秒级）不会误杀
    let watchdog = ctx.plan.diag.batch_watchdog(rows.len());
    debug_assert!(watchdog <= WRITE_CALL_TIMEOUT_MAX);
    let (outcome, timed_out) = match tokio::time::timeout(watchdog, ctx.writer.write(worker, rows)).await {
        Ok(outcome) => (outcome, false),
        Err(_) => {
            // 半开连接上那条语句还挂在宿主里，**必须换新连接**再重试，
            // 否则下一次调用复用它、再挂一次（真机表现为「每张表都卡一次」）。
            // 同时把**单条语句的字节上限**减半：劣化链路（PMTU 黑洞 / 静默丢包）上大报文
            // 会一直撞重传退避 —— 报文不小下去，重试只是「再等一次同样的超时」，
            // 界面上就成了同一张表反复「写入中断」。
            let (old_cap, new_cap) = ctx.writer.shrink_stmt(worker);
            // 探活：分清「这一条连接半开」和「整条链路/目标不可用」——
            // 后者换连接只会在下一批再撞一次，该让整个任务一起停一会儿；
            // 也顺手给日志一个证据（探不通 ≠ 用户数据有问题）。
            let halt_note = if probe_target(&ctx.plan).await {
                String::new()
            } else {
                match &ctx.plan.grow {
                    Some(grow) => {
                        grow.hold_link(LINK_HOLD_MS);
                        format!(
                            "；目标端探活也不通（链路整体不可用），本任务所有表先停 {} 秒",
                            LINK_HOLD_MS / 1000
                        )
                    }
                    None => "；目标端探活也不通（链路整体不可用）".to_string(),
                }
            };
            ctx.writer.renew().await;
            let shrink_note = if new_cap > 0 && new_cap < old_cap {
                format!("，并把重试的单条语句上限降到 {} KB", new_cap / 1024)
            } else {
                String::new()
            };
            (
                Err(format!(
                    "单批 {} 行写入 {} 秒未返回（该表近期单批通常在 {} 秒内）：目标端连接可能已半开\
                     （RDS wait_timeout 踢连接 / 链路静默丢包），或这条连接没设 socketTimeout。\
                     已弃用该连接{shrink_note}{halt_note}；若同一张表反复出现，把「编辑数据源 → 高级参数」\
                     里的 socketTimeout 改成 120000（毫秒，注意不是 120）",
                    rows.len(),
                    watchdog.as_secs(),
                    (watchdog.as_secs() / 8).max(1),
                )),
                true,
            )
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
