//! Writer 插件：吃行批次、批量落库、脏行降级、事务提交。
//!
//! `SqlWriter` 覆盖所有走 JDBC 的目标（批量 INSERT），并且把 Doris 的
//! **Stream Load** 快通道收进同一个插件：`stream` 有配置时优先 HTTP 直推 BE，
//! 一失败就本表内回退 INSERT —— 这与改造前在 `sync_table_inner` 里写的
//! `stream_fallback` 分支是同一套语义，只是不再和读写编排混在一起。
//!
//! 落库的三条底线（改造前后一致，don't regress）：
//! - **批量**：按目标方言的行数/表达式数/字节上限切成大子批，不是一行一条；
//! - **部分成功**：一条语句失败 → 二分拆批定位坏行（比逐行快 log₂N 倍：
//!   2 万行的批里 1 个坏值，逐行要 2 万次往返，二分约 30 次），坏行单独计失败
//!   并记样例错误，好行照常落库（对齐 DataX 的 errorLimit 思路：脏数据可见，不静默吞掉）；
//! - **事务**：每路 writer 自己开/提交（`internal:sync-w{i}` 是一条独立连接），
//!   不跨连接拼事务 —— 改造前那层「在 A 连接上 set autocommit=0、在 B 连接上写」
//!   的事务等于没开（真机踩过）。

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use dbmind_core::CellValue;

use super::{BF, GrowCaps, StreamCfg, TablePlan, UpsertMode, WriteDiag};
use crate::api::dialect::Dialect;
use crate::api::sync::{
    build_update, doris_stream_load, estimate_row_bytes, json_row_bytes, rows_need_json_fallback,
    run_target_in, supports_multi_statement, txn_batch, write_insert_rows, write_update_batch,
    write_upsert_rows, write_parallelism, STREAM_LOAD_MAX_BYTES, UPDATE_BATCH,
};
use crate::api::tasks::Task;
use crate::AppState;

/// 落库插件。
pub trait Writer: Send + Sync {
    fn name(&self) -> &'static str;
    /// 并发写连接数（单连接池的库会自动是 1）
    fn workers(&self) -> usize;
    /// 并发写连接数的**上限**：尾部弹性扩路时最多能开到几路。
    ///
    /// 默认等于 [`Writer::workers`]（不扩容）；`SqlWriter` 返回方言声明的写并发，
    /// 引擎据此判断「这一轮能不能再补令牌」。
    fn workers_ceiling(&self) -> usize {
        self.workers()
    }
    /// 第 `worker` 路开始（开事务；不支持事务的方言是空操作）
    fn begin<'a>(&'a self, worker: usize) -> BF<'a, ()>;
    /// 写一批**新增**行，返回成功落库的行数
    fn write<'a>(&'a self, worker: usize, rows: &'a [Vec<CellValue>]) -> BF<'a, Result<u64, String>>;
    /// 写一批**已存在**的行（upsert 的更新；内部攒批，凑满一批才真发）
    fn update<'a>(&'a self, rows: &'a [Vec<CellValue>]) -> BF<'a, Result<u64, String>>;
    /// 收尾：把攒着的更新发出去（返回真正写掉的行数）
    fn flush<'a>(&'a self) -> BF<'a, Result<u64, String>>;
    /// 第 `worker` 路结束（提交/复位）。
    ///
    /// **提交失败必须报错**：以前返回 `()` 且把 `commit` 的 Err 丢掉 —— 事务提交失败
    /// 时界面照样显示「同步成功」，实际一行都没落盘（或只落了一半）。
    fn finish<'a>(&'a self, worker: usize) -> BF<'a, Result<(), String>>;
    /// 第 `worker` 路**放弃**：回滚本路未提交的行并复位 autocommit。
    ///
    /// 与 [`Writer::finish`] 的唯一差别是**不提交**。写侧出错（脏数据、连接断、超时）
    /// 必须走这里：整表重跑的前提是「本次尝试写的行全部回滚」，否则追加模式重跑会写重复。
    /// 以前这里走的也是 `finish`（即出错也提交），于是「断连整表重跑」在追加模式下
    /// 真的可能写重复 —— 这是本轮修掉的一个正确性缺口。
    ///
    /// 不返回 `Result`：放弃路径已经在报错，回滚再失败也没有更好的处理
    /// （连接多半已经断了，服务端会自行回滚未提交事务）。
    fn abort<'a>(&'a self, worker: usize) -> BF<'a, ()>;

    /// 换一条**全新物理连接**：链路级失败/单批超时后调用。
    ///
    /// 半开连接上那条语句还挂在宿主里，紧接着的重试若复用同一会话就会再卡一次；
    /// 会话名带代数后缀 ⇒ 内核当新会话建连，旧的那条由服务端超时回收。
    fn renew<'a>(&'a self) -> BF<'a, ()> {
        Box::pin(async {})
    }

    /// 单批写超时后：把**单条语句的字节上限**减半，让紧接着的重试用更小的报文。
    ///
    /// 为什么必须减：劣化链路（PMTU 黑洞 / 静默丢包）上大报文会一直撞重传退避 ——
    /// 报文不小下去，重试就是「再等一次同样的超时」，界面上表现为同一张表反复
    /// 「写入中断」。真机口径：同一条 TAP 隧道 60KB 的 INSERT 稳过、121KB 必挂。
    ///
    /// 返回 `(旧, 新)` 字节上限，供界面日志说明重试已经拆小到什么程度。
    /// 默认实现不调整（返回 `(0, 0)`）。
    fn shrink_stmt(&self, _worker: usize) -> (usize, usize) {
        (0, 0)
    }
}

/// 一张表最多换几次写连接（见 `Writer::renew`）。
const MAX_RENEW: u64 = 2;

pub struct SqlWriter {
    state: AppState,
    task: Option<std::sync::Arc<Task>>,
    tgt_target: String,
    scope: String,
    target_name: String,
    target_table: String,
    table: String,
    /// 目标侧列名（写入只用这个；源列名留给 reader）
    target_columns: Vec<String>,
    target_types: Vec<Option<String>>,
    kind: Dialect,
    identity_on: bool,
    /// upsert 才需要：主键列下标 + 是否支持一条语句多条 UPDATE
    upsert: UpsertMode,
    key_indexes: Vec<usize>,
    /// 本表的会话后缀（见 `TablePlan::sid`）—— 并发多表不能共用同一条物理连接
    sid: String,
    multi_statement: bool,
    pending_updates: tokio::sync::Mutex<Vec<String>>,
    /// upsert 的更新走一条串行会话，它自己的事务（攒 500 条一提交 → 一次 fsync，
    /// 不包事务的话每批 500 条就 autocommit 一次，20 万行的更新会多出几百次提交）
    update_txn_open: AtomicBool,
    txn: Option<(&'static str, &'static str, &'static str)>,
    txn_open: Vec<AtomicBool>,
    stream: Option<StreamCfg>,
    stream_failed: AtomicBool,
    workers: usize,
    /// 尾部弹性扩路的**上限**（方言声明的写并发）：`txn_open` / `stmt_caps` 按它预分配，
    /// 运行时补发的 worker 编号才不会越界落到「没有事务标记」的默认分支。
    workers_ceiling: usize,
    /// 每路 writer 的「单条 INSERT 字节上限」（按实测耗时自适应，见 `sync::adapt_stmt_cap`）：
    /// 链路慢/丢包时把报文拆小，比死守方言的 packet 上限快一个数量级
    stmt_caps: Vec<AtomicUsize>,
    /// 写池诊断（二分定位到的坏行样例写这里，卡顿日志会读）
    diag: Arc<WriteDiag>,
    /// 任务级共享配额（整库同步才有）：这里只用来收敛**报文上限**与**链路暂停**，
    /// 让一张表踩过的坑立刻对剩下几百张表生效（见 `GrowCaps::stmt_cap_hint`）。
    grow: Option<Arc<GrowCaps>>,
    /// 写会话**代数**：链路级失败/单批超时后 +1，下一次调用用的是全新会话名
    /// ⇒ 内核/宿主新建物理连接。
    ///
    /// 为什么必须换名字：一条半开的连接（RDS 踢了 / 链路静默丢包）在宿主里
    /// 那条语句还挂着，内核侧的会话缓存仍然是「活着」的，紧接着的重试会**复用
    /// 同一条卡死的连接**，于是再挂一次看门狗 —— 真机上表现为「每张表都卡五分钟」。
    generation: AtomicU64,
}

impl SqlWriter {
    pub fn new(plan: &TablePlan) -> Self {
        let kind_key = plan.target_kind.kind.key();
        let base = if plan.stream.is_some() {
            // Stream Load 的收益来自「在途并发」：下一个请求的上传与上一个 BE publish 重叠
            3
        } else {
            write_parallelism(plan.target_kind.kind)
        };
        // 任务级会话预算（见 `TablePlan::workers_cap`）：并发多表时每张表少开几路，
        // 整次任务的目标连接数不超过方言声明的并发数。
        let workers = if plan.workers_cap > 0 { base.min(plan.workers_cap) } else { base };
        // 调优/排查逃生阀：坏链路（丢包、PMTU 黑洞）单条流吞吐有上限，多开几路能堆总吞吐。
        let workers = std::env::var("DBMIND_SYNC_WRITERS")
            .ok()
            .and_then(|text| text.trim().parse::<usize>().ok())
            .filter(|n| *n >= 1)
            .unwrap_or(workers);
        // 尾部弹性的上限：方言声明的写并发（单连接池的库仍是 1）。运行时补发的 worker
        // 编号一定 ≤ 它，所以数组按它预分配就够 —— 扩容只是「多给几路会话」，不改结构。
        let workers_ceiling = workers.max(base).max(1);
        // 起始报文上限：方言默认值、**本任务已学到的建议值**、起步值 512KB 三者取小。
        // 这一步是「一次踩坑、全任务收敛」的落地点：同一任务里某张表已经证明过大报文
        // 会卡住，之后开工的表直接从小的开始，不必各自再撞一次。
        // 起步值同样重要：方言上限（MySQL 1MB）只是协议允许值，不是这条链路吃得下的值 ——
        // 先用小报文探路，实测快了 `adapt_stmt_cap` 自己会翻倍上去（用户口径：不要中断）。
        let stmt_cap = crate::api::sync::effective_stmt_cap(
            kind_key,
            plan.grow
                .as_ref()
                .map(|grow| grow.stmt_cap_hint())
                .unwrap_or(0),
        )
        .min(crate::api::sync::STMT_CAP_START);
        Self {
            state: plan.state.clone(),
            task: plan.task.clone(),
            tgt_target: plan.tgt_target.clone(),
            scope: plan.tgt_scope.clone(),
            target_name: plan.target_name.clone(),
            target_table: plan.target_table.clone(),
            table: plan.table.clone(),
            target_columns: plan.target_columns.clone(),
            target_types: plan.target_types.clone(),
            kind: plan.target_kind,
            identity_on: plan.identity_on,
            upsert: plan.upsert,
            key_indexes: plan.key_indexes.clone(),
            sid: plan.sid.clone(),
            multi_statement: supports_multi_statement(kind_key),
            pending_updates: tokio::sync::Mutex::new(Vec::new()),
            update_txn_open: AtomicBool::new(false),
            txn: txn_batch(kind_key),
            txn_open: (0..workers_ceiling).map(|_| AtomicBool::new(false)).collect(),
            stream: plan.stream.clone(),
            stream_failed: AtomicBool::new(false),
            stmt_caps: (0..workers_ceiling)
                .map(|_| AtomicUsize::new(stmt_cap))
                .collect(),
            grow: plan.grow.clone(),
            workers: workers.max(1),
            workers_ceiling,
            diag: plan.diag.clone(),
        generation: AtomicU64::new(0),
        }
    }

    /// 每路 writer 一条**独立会话**：会话池里「亲和键不同」才会给不同物理连接，
    /// 全用同一个键的话 N 路并发会挤在一条连接上排队（改造前的真实瓶颈）。
    ///
    /// `sid` 必须带上（见 `TablePlan::sid`）：只按 worker 编号分键时，**不同表**的
    /// 同名 worker 会落到同一条连接上，多个表的 begin/commit 互相串。
    fn session(&self, worker: usize) -> String {
        let gen = self.generation.load(Ordering::SeqCst);
        if gen == 0 {
            format!("internal:sync-w{}-{worker}", self.sid)
        } else {
            // 代数后缀仍以 sid 开头：收尾时按 `contains(sid)` 清会话照样命中
            format!("internal:sync-w{}-{worker}-g{gen}", self.sid)
        }
    }

    /// upsert 的更新批走一条**本表专属**的串行会话（同 `sid` 隔离理由）。
    fn update_session(&self) -> String {
        let gen = self.generation.load(Ordering::SeqCst);
        if gen == 0 {
            format!("internal:sync-u{}", self.sid)
        } else {
            format!("internal:sync-u{}-g{gen}", self.sid)
        }
    }
}

impl Writer for SqlWriter {
    fn name(&self) -> &'static str {
        if self.stream.is_some() {
            "sql-writer+stream-load"
        } else {
            "sql-writer"
        }
    }

    fn workers(&self) -> usize {
        self.workers
    }

    fn workers_ceiling(&self) -> usize {
        self.workers_ceiling
    }

    fn begin<'a>(&'a self, worker: usize) -> BF<'a, ()> {
        Box::pin(async move {
            let session = self.session(worker);
            // 目标端写会话的**锁等待上限**：MySQL 的元数据锁（MDL）默认能等一年
            //（`lock_wait_timeout` = 31536000），行锁默认 50 秒。真机上遇到「同步卡住、
            // 一行都不写、也一个错误都没有」——就是 INSERT 排在某个没释放的锁后面，
            // 而 MDL 那条路没有超时。压到 60 秒后同样场景会**报错退出**（Lock wait timeout），
            // 界面至少能看到是哪张表被什么挡住了，而不是永远显示「同步中」。
            if matches!(self.kind.kind.key(), "mysql" | "mariadb") {
                let _ = run_target_in(
                    &self.state,
                    &self.tgt_target,
                    "set session lock_wait_timeout = 60, session innodb_lock_wait_timeout = 60",
                    &session,
                )
                .await;
                // 迁移期会话加速（DataX / Kettle 用户手工做的同一套，这里默认只做安全的那几条）：
                // - `foreign_key_checks = 0`：跨表同步顺序不保证，外键校验只会制造失败；
                // - `sql_log_bin = 0` 会断开目标端 binlog（省一半写放大），但它会**影响主从复制**，
                //   所以只在用户显式开了 `DBMIND_SYNC_SKIP_BINLOG=1` 时才做，且权限不足就静默跳过；
                // - `unique_checks = 0` 故意不做：唯一约束失效会静默写进重复数据，得不偿失。
                let _ = run_target_in(
                    &self.state,
                    &self.tgt_target,
                    "set session foreign_key_checks = 0",
                    &session,
                )
                .await;
                if std::env::var_os("DBMIND_SYNC_SKIP_BINLOG").is_some() {
                    let _ = run_target_in(
                        &self.state,
                        &self.tgt_target,
                        "set session sql_log_bin = 0",
                        &session,
                    )
                    .await;
                }
            }
            let Some((begin, _, _)) = self.txn else {
                return;
            };
            match run_target_in(&self.state, &self.tgt_target, begin, &session).await {
                Ok(_) => {
                    if let Some(open) = self.txn_open.get(worker) {
                        open.store(true, Ordering::SeqCst);
                    }
                }
                Err(err) => {
                    // 开不了事务就退回 autocommit 逐条写 —— 慢，但功能不缺。
                    // 错误要说出来：退回逐条提交会让每条 INSERT 各等一次提交
                    // （目标端开了 semi-sync 时每条都要等 slave ACK），吞吐差一个量级。
                    //
                    // 同时**记下这件事**：逐条提交意味着断连后已写的行留在目标表里，
                    // 这张表就不能「整表重跑」（会写重复），见 `WriteDiag::begin_failed`。
                    self.diag.begin_failed.store(true, Ordering::SeqCst);
                    if let Some(task) = &self.task {
                        task.log(format!(
                            "表 {}：开启事务失败（{}），退回逐条提交（大表会明显变慢）",
                            self.table, err.message
                        ));
                    }
                }
            }
        })
    }

    fn write<'a>(&'a self, worker: usize, rows: &'a [Vec<CellValue>]) -> BF<'a, Result<u64, String>> {
        Box::pin(async move {
            let in_transaction = self
                .txn_open
                .get(worker)
                .map(|open| open.load(Ordering::SeqCst))
                .unwrap_or(false);
            // ---- 原生 upsert：一条语句搞定「插入或更新」----
            // 不再需要「先把目标全表主键拉进内存 → 命中的逐行 UPDATE」那两步：
            // 那两步在大表上是纯开销（目标主键全量跨网 + 每行一次更新）。
            if self.upsert == UpsertMode::Native {
                return write_upsert_rows(
                    &self.state,
                    &self.tgt_target,
                    &self.session(worker),
                    &self.target_name,
                    &self.target_columns,
                    &self.target_types,
                    &self.key_indexes,
                    self.kind,
                    rows,
                    self.identity_on,
                    self.task.as_ref(),
                    &self.table,
                    in_transaction,
                )
                .await;
            }
            // 同一条任务里只要有一处判定 Stream Load 不可用（端口没映射 / BE 不通），
            // 后面的表就别再试了 —— 否则几十张表各写一条「失败」日志，根因却只有一个
            let stream_unavailable = self
                .task
                .as_ref()
                .map(|task| task.stream_unavailable())
                .unwrap_or(false);
            if let Some(stream) = self.stream.as_ref() {
                if !stream_unavailable && !self.stream_failed.load(Ordering::SeqCst) && !rows.is_empty() {
                    // **按字节切块**：行数管不住行宽（宽行表一页能拼出 85MB，BE 的 publish
                    // 代价陡增，实测反而更慢）。16MB/块是实测甜点。
                    let mut start = 0usize;
                    let mut loaded_total = 0u64;
                    let mut stream_error: Option<String> = None;
                    // 文本里含分隔控制字符的批会走 JSON（体积约等于 CSV 的 1.3 倍），
                    // 字节上限按实际编码方式算，别让宽行表把单请求推到 20MB+
                    let json_mode = rows_need_json_fallback(rows);
                    while start < rows.len() {
                        let mut end = start;
                        let mut bytes = 0usize;
                        while end < rows.len() {
                            let row_bytes = if json_mode {
                                json_row_bytes(&rows[end])
                            } else {
                                estimate_row_bytes(&rows[end])
                            };
                            if end > start && bytes + row_bytes > STREAM_LOAD_MAX_BYTES {
                                break;
                            }
                            bytes += row_bytes;
                            end += 1;
                        }
                        match doris_stream_load(
                            stream.fe_http_port,
                            stream.be_http_port,
                            &stream.host,
                            &stream.user,
                            &stream.pass,
                            &self.scope,
                            &self.target_table,
                            &self.target_columns,
                            &rows[start..end],
                        )
                        .await
                        {
                            // **按 BE 实际载入行数记账**：类型不匹配等被过滤的行计入失败，不静默吞
                            Ok(loaded) => {
                                loaded_total += loaded;
                                if let Some(task) = &self.task {
                                    let filtered = (end - start) as u64 - loaded;
                                    if filtered > 0 {
                                        task.add_rows_failed(filtered);
                                        task.log(format!(
                                            "表 {}：Stream Load 本批 {filtered} 行被目标过滤（类型不匹配等），已计入失败数",
                                            self.table
                                        ));
                                    }
                                }
                            }
                            Err(err) => {
                                stream_error = Some(err);
                                break;
                            }
                        }
                        start = end;
                    }
                    if stream_error.is_none() {
                        return Ok(loaded_total);
                    }
                    // 首次失败（端口不通/权限/格式）：本表内回退 INSERT 管道，数据不丢。
                    // 已经上成功的块不再重发，只把**剩下的行**走 INSERT。
                    // 措辞用「改用」而不是「失败」：这属于快通道不可用后的降级，数据没丢
                    //（真机反馈：日志里看到「失败」两个字就以为同步坏了）。
                    self.stream_failed.store(true, Ordering::SeqCst);
                    let message = format!(
                        "表 {}：Stream Load 快通道不可用（{}），剩余 {} 行改用 INSERT 管道导入（数据不受影响）",
                        self.table,
                        stream_error.unwrap_or_default(),
                        rows.len() - start
                    );
                    match &self.task {
                        // 只让**第一个**失败的表写这条日志：并发的其它表都会走 INSERT，
                        // 根因只有一个，重复几十条只会淹没真正的错误
                        Some(task) if task.disable_stream_load() => task.log(message),
                        Some(_) => {}
                        None => tracing::warn!(%message, "Stream Load 快通道不可用"),
                    }
                    let rest = write_insert_rows(
                        &self.state,
                        &self.tgt_target,
                        &self.session(worker),
                        &self.target_name,
                        &self.target_columns,
                        &self.target_types,
                        self.kind,
                        &rows[start..],
                        self.identity_on,
                        self.task.as_ref(),
                        &self.table,
                        in_transaction,
                        &self.diag,
                        &self.stmt_caps[worker.min(self.stmt_caps.len() - 1)],
                        self.grow.as_deref(),
                    )
                    .await?;
                    return Ok(loaded_total + rest);
                }
            }
            write_insert_rows(
                &self.state,
                &self.tgt_target,
                &self.session(worker),
                &self.target_name,
                &self.target_columns,
                &self.target_types,
                self.kind,
                rows,
                self.identity_on,
                self.task.as_ref(),
                &self.table,
                in_transaction,
                &self.diag,
                &self.stmt_caps[worker.min(self.stmt_caps.len() - 1)],
                self.grow.as_deref(),
            )
            .await
        })
    }

    fn update<'a>(&'a self, rows: &'a [Vec<CellValue>]) -> BF<'a, Result<u64, String>> {
        Box::pin(async move {
            if self.upsert != UpsertMode::TwoPhase || rows.is_empty() {
                return Ok(0);
            }
            // 更新走一条本表专属的串行会话：先把它的事务开起来（一次 fsync 覆盖整表的更新）
            let usid = self.update_session();
            if let Some((begin, _, _)) = self.txn {
                if !self.update_txn_open.load(Ordering::SeqCst) {
                    if run_target_in(&self.state, &self.tgt_target, begin, &usid).await.is_ok() {
                        self.update_txn_open.store(true, Ordering::SeqCst);
                    }
                }
            }
            if self.multi_statement {
                // 支持多语句批的方言：攒够 UPDATE_BATCH 条一次发（一 RTT 打一批）
                let mut pending = self.pending_updates.lock().await;
                for row in rows {
                    pending.push(build_update(
                        &self.target_name,
                        &self.target_columns,
                        &self.target_types,
                        row,
                        &self.key_indexes,
                        self.kind,
                    ));
                }
                if pending.len() < UPDATE_BATCH {
                    return Ok(0);
                }
                let mut batch = std::mem::take(&mut *pending);
                drop(pending);
                return write_update_batch(
                    &self.state,
                    &self.tgt_target,
                    self.kind.kind.key(),
                    &usid,
                    &mut batch,
                    self.task.as_ref(),
                    &self.table,
                    self.update_txn_open.load(Ordering::SeqCst),
                )
                .await;
            }
            // 驱动不支持一次多条 SQL：保持逐行（慢，但功能不缺）
            let mut ok = 0u64;
            let mut failed = 0u64;
            let mut sample: Option<String> = None;
            for row in rows {
                let sql = build_update(
                    &self.target_name,
                    &self.target_columns,
                    &self.target_types,
                    row,
                    &self.key_indexes,
                    self.kind,
                );
                match crate::api::sync::run_target(&self.state, &self.tgt_target, &sql).await {
                    Ok(_) => ok += 1,
                    Err(err) => {
                        failed += 1;
                        if sample.is_none() {
                            sample = Some(err.message);
                        }
                    }
                }
            }
            if let Some(task) = &self.task {
                if failed > 0 {
                    task.add_rows_failed(failed);
                    task.log(format!(
                        "表 {}：upsert 逐行更新失败 {failed} 行，样例错误：{}",
                        self.table,
                        sample.as_deref().unwrap_or_default()
                    ));
                }
            }
            if failed > 0 && failed * 2 > rows.len() as u64 {
                return Err(sample.unwrap_or_else(|| "upsert 更新大量失败".to_string()));
            }
            Ok(ok)
        })
    }

    fn flush<'a>(&'a self) -> BF<'a, Result<u64, String>> {
        Box::pin(async move {
            let mut written = 0u64;
            let mut failure: Option<String> = None;
            let usid = self.update_session();
            let mut pending = self.pending_updates.lock().await;
            if !pending.is_empty() {
                let mut batch = std::mem::take(&mut *pending);
                drop(pending);
                match write_update_batch(
                    &self.state,
                    &self.tgt_target,
                    self.kind.kind.key(),
                    &usid,
                    &mut batch,
                    self.task.as_ref(),
                    &self.table,
                    self.update_txn_open.load(Ordering::SeqCst),
                )
                .await
                {
                    Ok(rows) => written = rows,
                    Err(msg) => failure = Some(msg),
                }
            }
            // 关掉更新事务：无论成功失败都提交 —— 与「已写入的部分保留」的既有语义一致
            if self.update_txn_open.swap(false, Ordering::SeqCst) {
                if let Some((_, end, restore)) = self.txn {
                    if let Err(err) = run_target_in(&self.state, &self.tgt_target, end, &usid).await {
                        if failure.is_none() {
                            failure = Some(format!("提交事务失败：{}", err.message));
                        }
                    }
                    if !restore.is_empty() {
                        if let Err(err) = run_target_in(&self.state, &self.tgt_target, restore, &usid).await {
                            if failure.is_none() {
                                failure = Some(format!("恢复 autocommit 失败：{}", err.message));
                            }
                        }
                    }
                }
            }
            match failure {
                Some(msg) => Err(msg),
                None => Ok(written),
            }
        })
    }

    fn finish<'a>(&'a self, worker: usize) -> BF<'a, Result<(), String>> {
        Box::pin(async move {
            let Some((_, end, restore)) = self.txn else {
                return Ok(());
            };
            if !self.txn_open.get(worker).map(|open| open.load(Ordering::SeqCst)).unwrap_or(false) {
                return Ok(());
            }
            let session = self.session(worker);
            // 提交失败**不能咽掉**：以前这里 `let _ =`，commit 报错也当成功，
            // 界面显示「同步成功」而目标端要么没数据、要么只落了一半。
            let mut failure: Option<String> = None;
            // 「提交过」要在**发起提交之后**立刻记，而不是等结果：commit 报错时这一片
            // 是否已经落盘是未知的（服务端提交成功但响应丢了、或提交到一半断连）。
            // 上层「能不能整表重跑」必须按最坏情况判断 —— 记下它，就不许再重跑。
            self.diag.committed.fetch_add(1, Ordering::SeqCst);
            if let Err(err) = run_target_in(&self.state, &self.tgt_target, end, &session).await {
                failure = Some(format!("提交事务失败：{}", err.message));
            }
            // 恢复 autocommit 失败同样要报（否则连接池里留着一个开着事务的脏会话）
            if !restore.is_empty() {
                if let Err(err) = run_target_in(&self.state, &self.tgt_target, restore, &session).await {
                    if failure.is_none() {
                        failure = Some(format!("恢复 autocommit 失败：{}", err.message));
                    }
                }
            }
            match failure {
                Some(msg) => Err(msg),
                None => Ok(()),
            }
        })
    }

    fn abort<'a>(&'a self, worker: usize) -> BF<'a, ()> {
        Box::pin(async move {
            let open = self
                .txn_open
                .get(worker)
                .map(|flag| flag.load(Ordering::SeqCst))
                .unwrap_or(false);
            if !open {
                return;
            }
            let session = self.session(worker);
            // `rollback` 对这里列出的方言（MySQL / MariaDB / PG / Kingbase / SQLite /
            // SQL Server）都是合法语句；不支持的方言 `txn` 本来就是 None，走不到这。
            if let Err(err) =
                run_target_in(&self.state, &self.tgt_target, "rollback", &session).await
            {
                // 连接已经断了的话回滚必然失败 —— 服务端会自行回滚未提交事务，
                // 这不是错误，记一条 debug 就够
                tracing::debug!(
                    target: "dbmind::sync",
                    table = %self.table,
                    error = ?err,
                    "回滚写事务失败（连接多半已断，服务端会自行回滚）"
                );
            }
            if let Some((_, _, restore)) = self.txn {
                if !restore.is_empty() {
                    let _ = run_target_in(&self.state, &self.tgt_target, restore, &session).await;
                }
            }
            if let Some(flag) = self.txn_open.get(worker) {
                flag.store(false, Ordering::SeqCst);
            }
        })
    }

    fn renew<'a>(&'a self) -> BF<'a, ()> {
        Box::pin(async move {
            // 这一代的连接已经不可信：连「有没有开事务」都不再作数，
            // 全部标记复位，攒着的 upsert 也丢掉（它们会随本段重跑一起重来）。
            for flag in &self.txn_open {
                flag.store(false, Ordering::SeqCst);
            }
            self.update_txn_open.store(false, Ordering::SeqCst);
            self.pending_updates.lock().await.clear();
            // 换连接不是免费的：半开的那条在宿主里还占着一条物理连接和会话预算
            //（它自己不会退，只能等服务端/系统回收）。换两次还不行就认了，
            // 免得把整条连接的会话预算耗光、连累同时并发的其它表。
            if self.generation.load(Ordering::SeqCst) >= MAX_RENEW {
                tracing::warn!(
                    target: "dbmind::sync",
                    table = %self.table,
                    "已经换过 {MAX_RENEW} 次写连接，继续复用当前会话（避免耗光会话预算）"
                );
                return;
            }
            let next = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
            tracing::warn!(
                target: "dbmind::sync",
                table = %self.table,
                generation = next,
                "写连接已弃用，后续重试改用全新会话（避免复用半开的连接）"
            );
        })
    }

    fn shrink_stmt(&self, worker: usize) -> (usize, usize) {
        let slot = &self.stmt_caps[worker.min(self.stmt_caps.len() - 1)];
        let old = slot.load(Ordering::SeqCst);
        // 与 `write_insert_rows` 的断连路径同一口径：减半、不低于 STMT_CAP_MIN。
        // 只在超时/断连时收紧，健康链路上报文大小不受影响（耗时会把它翻倍回去）。
        let mut new = (old / 2).max(crate::api::sync::STMT_CAP_MIN);
        // 把结论写给**整个任务**：后面开工的表直接用这个上限，
        // 不必各自再撞一次同样的超时（这才是「不再频繁刷中断」的关键）。
        if let Some(grow) = &self.grow {
            let (_, hint) = grow.note_stmt_cap(new);
            if hint > 0 && hint < new {
                new = hint;
            }
        }
        if new < old {
            slot.store(new, Ordering::SeqCst);
        }
        (old, new)
    }
}
