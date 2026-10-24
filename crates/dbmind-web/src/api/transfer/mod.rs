//! 数据传输引擎（DataX 风格的 **Reader / Writer 插件 + 多通道流水线**）。
//!
//! ## 为什么要有这一层
//!
//! 改造前「读一页 → 写一页」的代码全塞在 `sync::sync_table_inner` 里（1000+ 行）：
//! 读的方言细节（分页语法、字面量）、写的行为（批量 INSERT / upsert / Stream Load）、
//! 并发、重试、进度、取消全缠在一起。想给某一种库换一种更快的写法（Doris 走 Stream
//! Load、同实例走 INSERT…SELECT）只能在那口大锅里加 `if`，越加越没人敢动。
//!
//! 这一层把它们拆成三件事：
//!
//! | 角色 | 职责 | 谁实现 |
//! |---|---|---|
//! | [`Reader`] | 把源表切成若干 **range**（DataX 的 split），每片按游标拉页 | `plugins/<kind>_reader.rs` |
//! | [`Writer`] | 吃行批次、批量落库、脏行降级、事务提交 | `plugins/<kind>_writer.rs` |
//! | [`spec::Spec`] | 该库的搬运规格：批量/报文上限/事务语句/保存点/自愈能力 | 写插件文件里的 `SPEC` |
//! | [`engine::run`] | 编排：分片 → N 读 → 有界通道 → M 写 → 计数/取消 | 一份 |
//!
//! 编排层只认 trait，所以「源 MySQL → 目标 Oracle」「源 PG → 目标 ClickHouse」之类的
//! 任意组合都是同一个引擎 + 两个不同的插件实例，不需要在引擎里出现任何数据库名字。
//!
//! ## 新增一种数据库类型
//!
//! **只加两个文件**：`plugins/<kind>_reader.rs` 与 `plugins/<kind>_writer.rs`。
//! 注册表由 `crates/dbmind-web/build.rs` 扫目录自动生成（`OUT_DIR/transfer_plugins.rs`），
//! 引擎、同步层、界面参数默认值都不用改 —— 详见 [`plugins`] 与 [`spec`] 的模块文档。
//!
//! ## 效率上的三个硬约束
//!
//! 1. **读写都并发**：源端按主键值域切 N 片并发拉（远端库单连接常被带宽/延迟卡死，
//!    N 条连接能吃到链路上限）；目标端 N 条独立连接并发写（`internal:sync-w{i}`，
//!    每条连接自带事务）。两侧并发数都由连接类型声明（见 `sync::write_parallelism`），
//!    单连接池的库（SQLite/H2/Derby/SQL Server）自动退回 1，不做无意义的排队。
//! 2. **有界通道 + 背压**：读快了会阻塞在通道上，内存峰值只与「在途页数 × 页大小」有关，
//!    与表大小无关。分片时页大小自动降到 5000 行（多片同时在途，避免几十页堆内存）。
//! 3. **单语句行数按目标方言定**（`sync::insert_batch_ranges` 那套），引擎不掺和。
//!
//! ## 容错
//!
//! - **读**：单页失败退避重试（3 次），仍失败才判该表失败，错误原样上抛；
//!   读协程 panic 也会被兜成失败信号 —— 不允许把「少读了一部分」当成「读完了」。
//! - **写**：一条语句失败 → **二分拆批**定位坏行（坏行单独计失败 + 记样例错误），
//!   好行照常落库；一批里过半失败判系统性问题（连接断/权限/表没了）立即中止。
//!   二分而不是逐行：2 万行的批里有 1 个坏值时，逐行要 2 万次往返（真机上是分钟级，
//!   期间唯一的写会话被占住、读侧只能干等），二分只要 log₂(2 万)≈15 层次 ≈ 30 次尝试。
//! - **取消**：每个页边界检查一次，已写入的部分保留（不回滚）。

pub mod engine;
pub mod pacer;
pub mod plugins;
pub mod reader;
pub mod spec;
pub mod writer;

pub use engine::run;
pub use reader::{Cursor, Page, Reader, RowRange};
pub use writer::Writer;

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::api::dialect::Dialect;
use crate::api::tasks::Task;
use crate::AppState;

/// 写池的运行态诊断：卡顿日志要说清「谁占着写会话、最近一批多久」。
///
/// 为什么放在 `TablePlan` 上而不是当函数参数传：记录方（写池 worker）与报告方
/// （分发侧的卡顿日志）是两个独立协程，需要同一份共享状态；热路径上只有两次
/// `AtomicU64` 读写，`Mutex` 只在记失败样例时碰一下，不参与每行处理。
/// 当前 epoch 毫秒（只用于诊断计时）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[derive(Default)]
pub struct WriteDiag {
    /// 已发出、尚未写完的批数
    pub inflight: AtomicU64,
    /// 最近一批写入的耗时（毫秒）
    pub last_write_ms: AtomicU64,
    /// 最近的「慢批上界」（毫秒，会缓慢衰减）。
    ///
    /// 用途：给**单批墙钟**定一个跟得上现场的阈值 —— 一刀切 300 秒时，
    /// 一条已经半开的连接要白等五分钟才被发现（真机：400 对象的任务跑到尾部，
    /// 几乎每张表都撞一次，五分钟一次地「卡住」）。按「最近批最慢值的 8 倍」算，
    /// 正常 1 秒一批的表 20 秒就能判定，真慢的批（十秒级）也不会被误杀。
    pub recent_max_ms: AtomicU64,
    /// 最近一批的行数
    pub last_batch_rows: AtomicU64,
    /// **正在写的那一批**的开始时刻（epoch 毫秒；0 = 当前没有在写的批）。
    ///
    /// 为什么要它：只报「最近一批用了多久」在写侧卡住时会误导 —— 真机上目标端
    /// 卡了 4 分半，日志一直显示「最近一批 20000 行用了 15.2 秒」（那是上一批），
    /// 用户看到的却是「读写数字三分钟一动不动」。有这一项才能说清「正在写的这一批
    /// 已经跑了几秒」。
    pub batch_started_ms: AtomicU64,
    /// 正在写的那一批的行数
    pub current_batch_rows: AtomicU64,
    /// 目标端写速的滑动估计（行/秒；0 = 还没测出来）。
    ///
    /// 用途：读侧据此决定**一页多大** —— 一页应该「二十秒左右写得完」。写快就放大页
    /// （少几次往返，吞吐才上得去），写慢就缩小页（否则只是把几十万行堆在队列里，
    /// 还会撞上单批 300 秒的墙钟）。真机反例：一刀切按写路数把页压到 2 万行，
    /// 快表被拖慢好几倍。
    pub write_rows_per_sec: AtomicU64,
    /// 最近一条失败样例（二分定位到的坏行错误），卡顿日志里顺手带上
    pub sample_err: Mutex<Option<String>>,
    /// 有过**开事务失败**（writer 退回 autocommit 逐条提交）。
    ///
    /// 用途：断连后能不能「整表重跑」取决于「已写的行是不是会随断连一起回滚」。
    /// 事务真的开起来了 ⇒ 连接一断，服务端回滚，目标表里没有半截数据，重跑是安全的；
    /// 退回 autocommit 过 ⇒ 已经逐条提交了，重跑会写重复，**绝不能重跑**。
    pub begin_failed: AtomicBool,
    /// 本表**已经提交成功**的写路数。
    ///
    /// 断连后能不能整表重跑，取决于它是不是 0：多路写时每路各是一个独立事务
    /// （见 `writer` 模块文档），先写完的那路会先提交；只要提交过一路，目标表里
    /// 就已经有本次任务的数据，**追加模式重跑会写重复** —— 回滚只能回滚未提交的。
    /// 提交阶段中途断连正是这种情况，所以判据必须是「一路都没提交过」。
    pub committed: AtomicUsize,
    /// 本表**成功写完的页数**。
    ///
    /// 用途：`== 0` 表示「这张表还没写进去任何一页」—— 第一页就卡住是目标端/链路
    /// 此刻有问题的强信号（真机日志里后期几乎每张表都是**第一页**卡满超时），
    /// 这时用更短的自愈阈值（30 秒）快速换连接重试，别陪它等 60~180 秒。
    pub batches_done: AtomicU64,
}

impl WriteDiag {
    /// 一批交给写池开写：记下开始时刻，供「正在写的这一批已跑几秒」用。
    pub fn note_batch_start(&self, rows: usize) {
        self.current_batch_rows.store(rows as u64, Ordering::Relaxed);
        self.batch_started_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// 一批写成功收尾：喂给看门狗与写速估计的样本。
    pub fn note_batch_done(&self, rows: usize, elapsed: std::time::Duration) {
        self.note_finish(rows, elapsed, true)
    }

    /// 一批**超时/被打断**收尾：与 [`WriteDiag::note_batch_done`] 的差别是
    /// **不抬高** `recent_max_ms`。
    ///
    /// 为什么：超时那一批的墙钟（60~300 秒）是「连接卡住了」，不是「这批本来就这么慢」。
    /// 拿它当慢批样本会把看门狗抬到 300 秒上限 —— 之后真断连要白等五分钟才判定，
    /// 界面上就是「这张表卡了五分钟」。写速估计照实下调（页要缩小，别再往慢链路塞大页）。
    pub fn note_batch_timeout(&self, rows: usize, elapsed: std::time::Duration) {
        self.note_finish(rows, elapsed, false)
    }

    fn note_finish(&self, rows: usize, elapsed: std::time::Duration, slow_sample: bool) {
        self.inflight.fetch_sub(1, Ordering::Relaxed);
        if slow_sample {
            // 只有**成功**的页才算「这张表已经能写进去」；超时的页不算（它没落库）
            self.batches_done.fetch_add(1, Ordering::Relaxed);
        }
        self.last_batch_rows.store(rows as u64, Ordering::Relaxed);
        let ms = elapsed.as_millis().min(u64::MAX as u128) as u64;
        self.last_write_ms.store(ms, Ordering::Relaxed);
        if slow_sample {
            // 慢批上界：取「本批」与「旧值 × 7/8」的较大者 —— 单批抖动会抬上去，
            // 但很快就会衰减回来，不会因为历史上一次慢批而长期放宽看门狗
            let prev = self.recent_max_ms.load(Ordering::Relaxed) * 7 / 8;
            self.recent_max_ms.store(prev.max(ms), Ordering::Relaxed);
        }
        // 没有在写的批了：清掉开始时刻，避免报告方把它当成「还在写」
        self.batch_started_ms.store(0, Ordering::Relaxed);
        self.current_batch_rows.store(0, Ordering::Relaxed);
        // 写速滑动估计：**变慢立刻认，变快慢慢信**。
        // - 慢下来（sample < old）直接采用：页大小与写速成正比，晚一批认就会多囤一个
        //   大页，真机上表现为「突然几十秒不动」（页大 → 单批撞上 300 秒墙钟）。
        // - 快起来按 1/4 平滑：单批抖动（目标端刷盘、偶尔的锁等待）不该把页瞬间放大。
        let ms = elapsed.as_millis().max(1) as u64;
        let sample = (rows as u64).saturating_mul(1_000) / ms;
        let old = self.write_rows_per_sec.load(Ordering::Relaxed);
        let next = if old == 0 || sample < old { sample } else { (old * 3 + sample) / 4 };
        self.write_rows_per_sec.store(next, Ordering::Relaxed);
    }

    /// 目标端写速估计（行/秒；0 = 还没有样本）。
    pub fn write_rate(&self) -> u64 {
        self.write_rows_per_sec.load(Ordering::Relaxed)
    }

    // 「多久算写入超时」不再由本结构决定：唯一来源是界面「传输参数 → 超时时间」
    // （0 = 继承连接 600 秒 / <0 = 不限制 / >0 = 用户填的秒数），见 `engine::wait_write`。
    // 这里曾经有 `batch_watchdog`（60~300 秒）与 `slow_batch_limit`（60~180 秒，第一页 30 秒）
    // 两套自算阈值，与界面配置互相矛盾 —— 已删除。`recent_max_ms` 仍保留：写速诊断要用。

    /// 正在写的那一批已经跑了多少秒（0 = 当前没有在写的批）。
    pub fn writing_seconds(&self) -> u64 {
        let started = self.batch_started_ms.load(Ordering::Relaxed);
        if started == 0 {
            return 0;
        }
        now_ms().saturating_sub(started) / 1000
    }

    pub fn note_sample_err(&self, msg: &str) {
        let mut slot = match self.sample_err.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if slot.is_none() {
            *slot = Some(msg.chars().take(200).collect());
        }
    }

    pub fn sample_err_text(&self) -> Option<String> {
        match self.sample_err.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

/// trait 方法里用的装箱 Future（不想为一个 `async fn in trait` 引入 `async-trait` 依赖）。
pub type BF<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Doris Stream Load 的连接参数（只有 doris 目标会带，其余为 `None`）。
#[derive(Clone, Debug, Default)]
pub struct StreamCfg {
    /// FE 的 MySQL 协议端口（Stream Load 的兜底入口，会 307 到 BE）
    pub fe_http_port: u16,
    /// BE 的真实 http 端口（`SHOW BACKENDS` 的 HttpPort，0 = 没探到，走 FE 重定向）
    pub be_http_port: u16,
    pub host: String,
    pub user: String,
    pub pass: String,
}

/// 运行中可被「整库调度」**调高**的表内并发（尾部弹性）。
///
/// 为什么需要运行时可变：片数与写路数都是表**开始时**按「当时并排几张表」算死的。
/// 尾部只剩一两张大表时，别的表退出的那份配额就白扔了 —— 真机实测 700 万行同步：
/// 两表并行 30k 行/秒，前面那张小表一结束，剩下的大表往下掉到 18k 行/秒，
/// 因为 5000 万/700 万行的大表当初只分到 2 片读 + 1 路写。
///
/// 调度侧（`sync::db`）每 500ms 按「在跑的表数 / 剩余表数」更新这两个数，表的引擎读到
/// 更大的值就把读片与写路**就地扩容**（令牌池补发 + 细块队列还有余量），不必重切表。
#[derive(Debug, Default)]
pub struct GrowCaps {
    /// 每张表可用的读片数（0 = 未启用弹性，按初始值）
    pub src_slices: AtomicUsize,
    /// 每张表可用的写路数（0 = 未启用）
    pub tgt_workers: AtomicUsize,
    /// 此刻**正在跑**的表数（调度侧每 500ms 刷一次）。
    ///
    /// 用途：表内读预取只在「这张表独占链路」时开。多表并发时每张表已经有 2~4 路，
    /// 再翻倍的在途流会把丢包链路拖进整体退避 —— 实测两表同时开预取从 30.6k 掉到 7k。
    pub active: AtomicUsize,
    /// 本任务学到的「单条写语句字节上限」（0 = 还没学到）。
    ///
    /// 为什么必须在**任务级**：写超时/断连的代价极高（一次就是几十秒白等 + 一段回滚 +
    /// 一条吓人的日志），而报文该缩到多少这件事，一张表撞过就够了 ——
    /// 存在表级的话，几百张表要各自把同样的坑踩一遍，界面上就是「几百条写入中断」。
    pub stmt_cap_hint: AtomicUsize,
    /// 本任务学到的「一页最多多少行」（0 = 还没学到）。
    ///
    /// 和 `stmt_cap_hint` 同一个道理，只是针对**页**：真机日志里后期几乎每张表都
    /// 「单批 7 万行写入 601 秒」，而每张表都是**第一次写就卡** —— 页上限只存在表内的话，
    /// 后面几百张表会各自把同一个坑踩一遍。这里一旦有表卡住，就把任务级页上限压到
    /// 1 万行，后续开工的表直接从小的起步（`SqlReader::new` 读它当硬上限）。
    /// 只降不升：链路/目标端在任务内被证明吃力之后，不该因为某张表跑得顺又放大回去。
    pub page_cap_hint: AtomicUsize,
    /// 链路整体不可达时**暂停到**的时刻（epoch 毫秒；0 = 没暂停）。
    ///
    /// 判定依据：某张表写超时后先探活（`select 1`），连探活都不通 ⇒ 这不是「这条连接
    /// 半开」，而是整条链路/目标不可用。此时所有表都该停手等一会儿，而不是各自换连接
    /// 重连重跑 —— 那会把一次短暂故障放大成一片重连、回滚和红字日志。
    pub link_hold_until_ms: AtomicU64,
}

impl GrowCaps {
    pub fn set(&self, slices: usize, workers: usize) {
        self.src_slices.store(slices, Ordering::SeqCst);
        self.tgt_workers.store(workers, Ordering::SeqCst);
    }

    /// 刷新「在跑的表数」（调度侧调用）。
    pub fn set_active(&self, tables: usize) {
        self.active.store(tables, Ordering::SeqCst);
    }

    /// 在跑的表数；0 = 还没刷过（按「独占」处理，允许预取）。
    pub fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    /// 目标读片数（只升不降：中途缩容会把已开的事务白扔）
    pub fn slots(&self, base: usize) -> usize {
        self.src_slices.load(Ordering::SeqCst).max(base)
    }

    /// 目标写路数（同上）
    pub fn want_workers(&self, base: usize) -> usize {
        self.tgt_workers.load(Ordering::SeqCst).max(base)
    }

    /// 建议的单条写语句字节上限（0 = 还没学到）。
    pub fn stmt_cap_hint(&self) -> usize {
        self.stmt_cap_hint.load(Ordering::SeqCst)
    }

    /// 记下一个**更小**的语句上限建议；返回 `(旧, 新)` —— 没有变小就两边一样。
    ///
    /// 只降不升：一条链路证明过「大于 N 字节的报文会卡住」，这个结论在整个任务里都成立，
    /// 后面的表不该因为一开始跑得顺就又放大回去（那只会再卡一次）。
    pub fn note_stmt_cap(&self, bytes: usize) -> (usize, usize) {
        let mut current = self.stmt_cap_hint.load(Ordering::SeqCst);
        loop {
            if current != 0 && current <= bytes {
                return (current, current);
            }
            match self.stmt_cap_hint.compare_exchange(
                current,
                bytes,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return (current, bytes),
                Err(seen) => current = seen,
            }
        }
    }

    /// 任务级页上限建议（0 = 还没学到）。
    pub fn page_cap_hint(&self) -> usize {
        self.page_cap_hint.load(Ordering::SeqCst)
    }

    /// 记下一个**更小**的页上限建议；返回 `(旧, 新)` —— 没有变小就两边一样。
    ///
    /// 只降不升，理由同 [`GrowCaps::note_stmt_cap`]：这是「这条链路/这台目标端吃不下
    /// 这么大一页」的结论，本任务里对剩下的表同样成立。
    pub fn note_page_cap(&self, rows: usize) -> (usize, usize) {
        let mut current = self.page_cap_hint.load(Ordering::SeqCst);
        loop {
            if current != 0 && current <= rows {
                return (current, current);
            }
            match self.page_cap_hint.compare_exchange(
                current,
                rows,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return (current, rows),
                Err(seen) => current = seen,
            }
        }
    }

    /// 链路整体不可用：让本任务所有表暂停 `ms` 毫秒（返回暂停到的时刻）。
    pub fn hold_link(&self, ms: u64) -> u64 {
        let until = now_ms().saturating_add(ms);
        self.link_hold_until_ms.fetch_max(until, Ordering::SeqCst);
        until
    }

    /// 还要暂停多少毫秒（0 = 不需要暂停）。
    pub fn link_hold_left_ms(&self) -> u64 {
        self.link_hold_until_ms
            .load(Ordering::SeqCst)
            .saturating_sub(now_ms())
    }
}

/// 一张表的**传输计划**：DDL（建表/索引/注释）由 sync 层做完，这里只管搬行。
///
/// 行数：**精确到个位 + 千分位**（`13,472 行`）。
///
/// 以前按万/亿取整（`1.3 万行`），用户直接问「到底多少行」—— 同步结果的行数是要拿去
/// 和源表/目标表核对的数字，取整等于把信息丢掉。
pub(crate) fn 行数(n: u64) -> String {
    format!("{} 行", 千分位(n))
}

/// 整数加千分位：`13472` → `13,472`。
pub(crate) fn 千分位(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 时长：`4 分 35 秒` / `1 小时 02 分`；不足一分钟给到 0.1 秒（`8.4 秒`）。
pub(crate) fn 时长(ms: u64) -> String {
    let secs = ms / 1000;
    if secs >= 3600 {
        format!("{} 小时 {:02} 分", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{} 分 {:02} 秒", secs / 60, secs % 60)
    } else {
        format!("{:.1} 秒", ms as f64 / 1000.0)
    }
}

/// 字段全是「已经解析好的值」——没有 `SyncRequest`、没有 `Opts`，插件不需要知道
/// 用户界面上勾了什么，只按这份计划执行。
pub struct TablePlan {
    pub state: AppState,
    pub task: Option<Arc<Task>>,
    /// 本表的**会话后缀**（会话亲和键的一部分）。
    ///
    /// 为什么必须有：内核会话池按「连接 + 亲和键」分泳道，**亲和键相同的请求落到同一条
    /// 物理连接**。以前所有表都用 `internal:sync-w0`、`internal:sync-r0`，于是整库并发
    /// （4 张表同时跑）时 4 张表的 0 号 writer 其实挤在同一根连接上：
    /// A 表 `begin` 之后 B 表在同一会话里接着写，A 表 `commit` 把 B 表的数据一起提交，
    /// 两边的 `rollback`/`恢复 autocommit` 也会互相踩。数据看着「同步成功」，
    /// 事务语义已经错乱 —— 并发越高越容易踩，且完全无声。
    pub sid: String,

    // ---- 源 ----
    pub src_conn: String,
    pub src_scope: String,
    pub source_kind: Dialect,
    /// 源表名（已 scoped 的 `库.表`，直接进 FROM；也用于日志）
    pub src_from: String,
    /// 日志里显示的表名（与 `src_from` 可能不同：跨类型会加引号）
    pub table: String,
    pub columns: Vec<String>,

    // ---- 目标 ----
    pub tgt_conn: String,
    pub tgt_scope: String,
    /// 已解析出的目标连接 id（见 `scope::resolve`）
    pub tgt_target: String,
    pub target_kind: Dialect,
    /// 已 quote 的目标表名（显式选了模式时带模式前缀）
    pub target_name: String,
    /// 目标表**裸名**：给 Doris Stream Load 这类自己拼 URL 的写入器用。
    /// 不能用 `target_name`（可能带引号/模式），也不能用 `table`（那是源表名）。
    pub target_table: String,
    /// **本表最多几路写会话**（0 = 用方言默认值）。
    ///
    /// 整库同步的总连接数 = 对象并发 × 每表 writer 数。4 张表 × 4 路 = 16 条目标连接，
    /// 而方言声明的并发（`write_parallelism`，MySQL/PG = 4）本意是**整个任务**的上限。
    /// 所以任务启动时按「并发对象数」把预算切给每张表：4 张表各 1 路，单表独享 4 路。
    ///
    /// 为什么不用共享信号量按会话持有：会话是粘性的，writer 在「等下一批数据」时会一直
    /// 攥着许可，而它自己的 reader 又在等另一张表释放读许可 —— 两者会互锁成死环。
    /// 按对象并发数静态切分既守住总连接数，又不存在任何等待环。
    pub workers_cap: usize,
    /// **本表最多几片并发读**（0 = 用方言默认值），切分逻辑同 `workers_cap`
    pub slices_cap: usize,
    /// 单条 INSERT 的**行数**上限（0 = 按目标方言默认，见 `sync::insert_rows_default`）。
    /// 界面上按目标库填默认值、允许用户改；writer 用它切子批。
    pub insert_rows: usize,
    /// 单次数据库操作超时（秒）：0 = 继承连接，<0 = 不限制，>0 = 该秒数。
    /// 界面上的「超时时间」（对齐 DBX）；换算见 `sync::op_timeout_ms`。
    pub op_timeout_secs: i64,
    /// 限速表（界面「限速」：行/秒 + MB/秒，取更慢的曲线）；`None` = 不限速。
    ///
    /// **整个任务共用一份**：每张表几路写、整库几张表同时跑，合计速率都要等于用户
    /// 填的值 —— 所以它存在计划里由调度侧注入，而不是每个 writer 各自算。
    pub pacer: Option<Arc<pacer::Pacer>>,
    /// 与 `columns` 一一对应的**目标侧真实列名**：源列 `order_no` 在目标上可能叫
    /// `ORDER_NO`（Oracle 折大写）或 `OrderNo`（历史表）。写语句必须用目标的名字，
    /// 否则「目标表已存在」这条路上会报列不存在 —— 而结构是好的、只是名字对不上。
    pub target_columns: Vec<String>,
    /// 与 `columns` 一一对应的目标列类型（用于字面量规范化，如日期补 `:00`）
    pub target_types: Vec<Option<String>>,

    // ---- 分页 / 过滤 ----
    pub key_columns: Vec<String>,
    pub key_indexes: Vec<usize>,
    /// 用于 keyset 游标的列（= 真实取到的键列；空 ⇒ 退回 OFFSET 分页）
    pub paging_keys: Vec<String>,
    /// 源表的**主键列**（不论有没有勾进同步列）：只用来决定日志措辞。
    ///
    /// `paging_keys` 为空有两种成因，给用户的建议完全不同：源表本来就没主键（建议加主键），
    /// 或者主键列没全勾上（加主键没用，要把主键列勾进同步）。没有这份信息就只能含糊成
    /// 「无可用主键」，用户照着去加主键会发现「明明有主键」。
    pub src_primary: Vec<String>,
    pub where_clause: String,
    pub row_limit: u64,

    // ---- 写入行为 ----
    pub upsert: UpsertMode,
    pub identity_on: bool,
    pub stream: Option<StreamCfg>,
    /// 写池运行态（在途批数 / 最近批耗时 / 失败样例）：卡顿日志读它
    pub diag: Arc<WriteDiag>,
    /// 尾部弹性并发（见 [`GrowCaps`]）：`None` = 单表同步，不参与整库调度
    pub grow: Option<Arc<GrowCaps>>,
    /// 本表**已读出的行数**（跨块累计，重试会扣回）。
    ///
    /// 用途：上层的「对象超时」判断改为**看有没有进展**而不是看总时长 —— 固定 11 分钟会把
    /// 明明一直在搬数据的表判死：无主键 500 万行只有 7.2k 行/秒（OFFSET 翻页），要 11.4 分钟；
    /// 有主键 5000 万行要 33 分钟。两者都踩过线，被当成「连接半开」整表放弃。
    pub progress: Arc<AtomicU64>,
}

/// upsert 怎么落库。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UpsertMode {
    /// 不是 upsert（仅插入 / 清空后导入）—— 全走 INSERT
    Off,
    /// **原生 upsert**：一条语句里带 `on duplicate key update` / `on conflict … do update` /
    /// `merge into … when matched`。不需要预读目标主键，也没有逐行 UPDATE。
    Native,
    /// 旧的两阶段：先读目标全部主键 → 命中的走 UPDATE、其余走 INSERT。
    /// 只留给没有原生语法的方言（ClickHouse / Doris / Derby），行为与以前完全一致。
    TwoPhase,
}

impl UpsertMode {
    pub fn is_upsert(self) -> bool {
        self != UpsertMode::Off
    }
}

/// 一次表传输的结果。
#[derive(Default, Debug)]
pub struct Stats {
    pub inserted: u64,
    pub updated: u64,
    /// 写失败的行数（脏数据逐行重试后仍未落库）。
    ///
    /// 保存点让「一行坏数据拖死整批」不再发生，好行照常落库 —— 代价是
    /// 「成功」不再是全有全无，**失败行数必须出现在结果消息里**，否则
    /// 界面显示成功、errors=0，用户不会知道少了几行。
    pub failed: u64,
    /// 用户取消（已写入的部分保留）
    pub canceled: bool,
    /// 收尾备注（追加到结果消息里）
    pub note: String,
}

/// 按**源**连接类型取 Reader 插件。
///
/// 每种数据源在 `plugins/<kind>_reader.rs` 里有自己的读插件（见 `plugins` 模块文档）；
/// 没登记的类型落到通用 `SqlReader`（能同步，只是用了最保守的参数）。
/// 引擎只认 trait，所以「源 MySQL → 目标 Oracle」「源 PG → 目标 ClickHouse」都是
/// 同一个引擎 + 两个插件实例，任意组合。
pub fn reader_for(plan: &TablePlan) -> Box<dyn Reader> {
    (plugins::reader_factory(plan.source_kind.kind.key()))(plan)
}

/// 按**目标**连接类型取 Writer 插件。
///
/// 每种数据源在 `plugins/<kind>_writer.rs` 里有自己的写插件 + 搬运规格（批量/事务/自愈）；
/// 落库方式特殊的（Doris 的 Stream Load）也只改它自己那个文件，引擎一行不动。
pub fn writer_for(plan: &TablePlan) -> Arc<dyn Writer> {
    (plugins::writer_factory(plan.target_kind.kind.key()))(plan)
}

/// 界面上「共 N 行」必须是**真实数字**（用户口径：`1.3 万行` 没法核对，他要的是原文）。
#[cfg(test)]
mod 数字格式 {
    use super::*;

    #[test]
    fn 行数千分位且不取整() {
        assert_eq!(千分位(0), "0");
        assert_eq!(千分位(999), "999");
        assert_eq!(千分位(1_000), "1,000");
        assert_eq!(千分位(9_266), "9,266");
        assert_eq!(千分位(13_472), "13,472");
        assert_eq!(千分位(213_210), "213,210");
        assert_eq!(行数(13_472), "13,472 行");
    }

    #[test]
    fn 不足一分钟的耗时给到零点一秒() {
        assert_eq!(时长(8_400), "8.4 秒");
        assert_eq!(时长(65_000), "1 分 05 秒");
    }
}

#[cfg(test)]
mod watchdog_tests {
    use super::*;
    use std::time::Duration;

    /// 超时的那一批要**下调写速**（页得缩小，别再往慢链路塞大页），并且收尾后不残留
    /// 「还在写」的状态。（判超时不再用本结构自算的阈值，只有界面「超时时间」一个来源。）
    #[test]
    fn 超时批下调写速且不残留状态() {
        let diag = WriteDiag::default();
        diag.note_batch_start(1_000);
        diag.note_batch_done(1_000, Duration::from_millis(400));
        let rate_before = diag.write_rate();
        assert!(rate_before > 0);

        diag.note_batch_start(20_000);
        diag.note_batch_timeout(20_000, Duration::from_secs(80));
        assert!(
            diag.write_rate() < rate_before,
            "写速要按超时下调：{} → {}",
            rate_before,
            diag.write_rate()
        );
        // 收尾后不应残留「还在写」的状态
        assert_eq!(diag.writing_seconds(), 0);
    }

    /// 任务级报文上限**只降不升**：一张表踩过的坑要立刻对同一任务里其它表生效
    /// （否则几百张表各撞一次 = 几百条「写入中断」），健康链路的翻倍逻辑也不能把它放大回来。
    #[test]
    fn 报文上限任务级只降不升() {
        let caps = GrowCaps::default();
        assert_eq!(caps.stmt_cap_hint(), 0);
        assert_eq!(caps.note_stmt_cap(3 * 1024 * 1024), (0, 3 * 1024 * 1024));
        assert_eq!(
            caps.note_stmt_cap(1024 * 1024),
            (3 * 1024 * 1024, 1024 * 1024)
        );
        // 更大的值不能覆盖已有结论
        assert_eq!(
            caps.note_stmt_cap(4 * 1024 * 1024),
            (1024 * 1024, 1024 * 1024)
        );
        assert_eq!(caps.stmt_cap_hint(), 1024 * 1024);
    }

    /// 页上限建议任务级**只降不升**：一张表的卡顿要立刻让后面的表用小页，
    /// 否则后期几百张表各自踩一次（真机：几乎每张表都「单批 X 万行 601 秒」）。
    #[test]
    fn 页上限建议任务级只降不升() {
        let caps = GrowCaps::default();
        assert_eq!(caps.page_cap_hint(), 0);
        assert_eq!(caps.note_page_cap(10_000), (0, 10_000));
        assert_eq!(caps.note_page_cap(5_000), (10_000, 5_000));
        // 更大的值不能覆盖已有结论
        assert_eq!(caps.note_page_cap(50_000), (5_000, 5_000));
        assert_eq!(caps.page_cap_hint(), 5_000);
    }

    /// 链路暂停窗口：多次调用取「最晚」的那次，过窗口后自动归零。
    #[test]
    fn 链路暂停按最晚时刻生效() {
        let caps = GrowCaps::default();
        assert_eq!(caps.link_hold_left_ms(), 0);
        caps.hold_link(60);
        assert!(caps.link_hold_left_ms() > 0);
        caps.hold_link(0); // 更早的时刻不能缩短已有暂停
        assert!(caps.link_hold_left_ms() > 0);
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(caps.link_hold_left_ms(), 0);
    }

    /// 起步报文上限必须**不大于**任何方言的协议上限：先小后大，慢链路不该一上来
    /// 就拿几 MB 的语句去撞丢包（用户口径：Navicat 不中断、我们也不许）。
    #[test]
    fn 起步报文上限小于各方言协议上限() {
        // 遍历注册表而不是手写名单：新增一种库时这条测试自动覆盖到它
        for &kind in plugins::KINDS {
            let cap = crate::api::sync::insert_stmt_cap(kind);
            assert!(
                crate::api::sync::STMT_CAP_START <= cap,
                "{kind} 的起步上限（{}）超过了协议上限（{}）",
                crate::api::sync::STMT_CAP_START,
                cap
            );
        }
    }
}
