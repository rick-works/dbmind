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

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use dbmind_core::CellValue;

use super::{BF, StreamCfg, TablePlan, UpsertMode, WriteDiag};
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
}

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
    /// 每路 writer 的「单条 INSERT 字节上限」（按实测耗时自适应，见 `sync::adapt_stmt_cap`）：
    /// 链路慢/丢包时把报文拆小，比死守方言的 packet 上限快一个数量级
    stmt_caps: Vec<AtomicUsize>,
    /// 写池诊断（二分定位到的坏行样例写这里，卡顿日志会读）
    diag: Arc<WriteDiag>,
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
            txn_open: (0..workers.max(1)).map(|_| AtomicBool::new(false)).collect(),
            stream: plan.stream.clone(),
            stream_failed: AtomicBool::new(false),
            stmt_caps: (0..workers.max(1))
                .map(|_| AtomicUsize::new(crate::api::sync::insert_stmt_cap(kind_key)))
                .collect(),
            workers: workers.max(1),
            diag: plan.diag.clone(),
        }
    }

    /// 每路 writer 一条**独立会话**：会话池里「亲和键不同」才会给不同物理连接，
    /// 全用同一个键的话 N 路并发会挤在一条连接上排队（改造前的真实瓶颈）。
    ///
    /// `sid` 必须带上（见 `TablePlan::sid`）：只按 worker 编号分键时，**不同表**的
    /// 同名 worker 会落到同一条连接上，多个表的 begin/commit 互相串。
    fn session(&self, worker: usize) -> String {
        format!("internal:sync-w{}-{worker}", self.sid)
    }

    /// upsert 的更新批走一条**本表专属**的串行会话（同 `sid` 隔离理由）。
    fn update_session(&self) -> String {
        format!("internal:sync-u{}", self.sid)
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
}
