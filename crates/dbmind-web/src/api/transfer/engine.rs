//! 传输编排：**分片 → N 路并发读 → 有界通道（背压）→ M 路并发写 → 计数/取消/容错**。
//!
//! 这张图就是 DataX 的 job 结构，只是把「插件进程」换成了同一个进程里的 trait 对象：
//!
//! ```text
//!   split() ──► [reader#0] ─┐
//!               [reader#1] ─┼─► page channel (有界) ─► dispatcher ─► write channel (有界) ─► [writer#0]
//!               [reader#2] ─┘        ↑ 背压               │ 聚合/截断/取消        ├─► [writer#1]
//!                                                        └─► update 攒批（upsert）
//! ```
//!
//! 为什么读侧也要并发：远端库的单连接吞吐常被「往返延迟 × 每页行数」卡死，一条连接
//! 只能跑 200~300 KB/s；切成 4 片并发读能直接把链路上限吃满（实测 20 万行跨网传输
//! 从 39s 降到 21s，本地库则不受影响 —— 单连接池的库会自动退回 1 片）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dbmind_core::CellValue;
use tokio::sync::mpsc;

use super::{reader_for, writer_for, Page, Stats, TablePlan, UpsertMode};
use crate::api::sync::{key_of, lookup_existing_keys};

/// 单页失败后的重试次数（退避 150ms / 300ms / 600ms）。
///
/// 只对**读**做重试：读是幂等的，重试没有副作用；写不做整批重试（一条语句可能已经
/// 落了一半），而是交给写入层的「语句失败 → 逐行降级」兜住。
const READ_RETRY: usize = 3;
/// 收尾等待上限（与 sync 层一致：给慢批留足时间，避免把还在写的数据判死）
const PIPE_WAIT: Duration = Duration::from_secs(660);
/// 进度状态最短刷新间隔：写一页更新一次就够，**不必每行**都更新
/// （20 万行就是 20 万次加锁 + 20 万次前端快照变化，纯属白烧 CPU）。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(300);

/// 跑完一张表的数据搬运。`Err` = 该表失败（已写入的行保留）。
pub async fn run(plan: TablePlan) -> Result<Stats, String> {
    let mut reader = reader_for(&plan);
    let writer = writer_for(&plan);
    let mut note = String::new();

    // ---- 1. 分片 -----------------------------------------------------------
    let want = reader.max_parallel();
    let ranges = reader.split(want).await?;
    let slices = ranges.len();
    if slices > 1 {
        if let Some(task) = &plan.task {
            task.log(format!(
                "表 {}：按主键切成 {slices} 片并发读（每片独立游标，页 {} 行）",
                plan.table,
                reader.page_rows()
            ));
        }
    }
    let reader: Arc<dyn super::Reader> = Arc::from(reader);

    // ---- 2. 写池（M 路独立连接，各自带事务）--------------------------------
    let workers = writer.workers();
    let (write_tx, write_rx) = mpsc::channel::<Vec<Vec<CellValue>>>(workers + 1);
    let write_rx = Arc::new(tokio::sync::Mutex::new(write_rx));
    let write_err: Arc<tokio::sync::Mutex<Option<String>>> = Arc::new(tokio::sync::Mutex::new(None));
    let shared_inserted = Arc::new(AtomicU64::new(0));
    let mut writer_handles = Vec::with_capacity(workers);
    for worker in 0..workers {
        let rx = write_rx.clone();
        let writer = writer.clone();
        let err = write_err.clone();
        let inserted = shared_inserted.clone();
        writer_handles.push(tokio::spawn(async move {
            writer.begin(worker).await;
            loop {
                // 锁只包 recv：不同 writer 排队领批，锁内不干活
                let batch = { rx.lock().await.recv().await };
                let Some(batch) = batch else { break };
                match writer.write(worker, &batch).await {
                    Ok(rows) => {
                        inserted.fetch_add(rows, Ordering::SeqCst);
                    }
                    Err(msg) => {
                        // 写不动了：记下原因让 dispatcher 收摊（源表再读也是白读）
                        *err.lock().await = Some(msg);
                        break;
                    }
                }
            }
            // 提交/恢复 autocommit 失败必须上抛：以前这里返回值被丢掉，commit 报错
            // 也当成功，界面显示「同步成功」而数据没落盘。
            if let Err(msg) = writer.finish(worker).await {
                let mut slot = err.lock().await;
                if slot.is_none() {
                    *slot = Some(msg);
                }
            }
        }));
    }

    // ---- 3. 读协程（每片一路，独立游标）------------------------------------
    let (page_tx, mut page_rx) = mpsc::channel::<Result<Page, String>>(slices * 2 + 2);
    let mut reader_handles = Vec::with_capacity(slices);
    for range in ranges {
        let reader = reader.clone();
        let tx = page_tx.clone();
        let task = plan.task.clone();
        reader_handles.push(tokio::spawn(async move {
            let mut cursor = None;
            loop {
                if task.as_ref().map(|t| t.is_canceled()).unwrap_or(false) {
                    break;
                }
                let page = match reader.page(&range, cursor.clone()).await {
                    Ok(page) => page,
                    Err(first) => {
                        // 读页失败：退避重试。远端链路偶发 reset / 超时很常见，重试一次
                        // 往往就过去了；三次都失败才判这张表失败。
                        let mut last = first;
                        let mut recovered = None;
                        for attempt in 0..READ_RETRY {
                            tokio::time::sleep(Duration::from_millis(150 * (1 << attempt))).await;
                            match reader.page(&range, cursor.clone()).await {
                                Ok(page) => {
                                    recovered = Some(page);
                                    break;
                                }
                                Err(err) => last = err,
                            }
                        }
                        match recovered {
                            Some(page) => page,
                            None => {
                                let _ = tx.send(Err(last)).await;
                                break;
                            }
                        }
                    }
                };
                let next = page.next.clone();
                cursor = next.clone();
                // 通道满了就阻塞在这里（背压），不再往上游要数据
                if tx.send(Ok(page)).await.is_err() {
                    break;
                }
                if next.is_none() {
                    break; // 这一片读完了
                }
            }
        }));
    }
    drop(page_tx);

    // ---- 4. 分发（单线程，负责截断/取消/分拨 upsert）-----------------------
    let mut updated = 0u64;
    let mut consumed = 0u64;
    let mut canceled = false;
    let mut limit_hit = false;
    let mut last_report = std::time::Instant::now();
    let mut failure: Option<String> = None;
    while let Some(message) = page_rx.recv().await {
        if write_err.lock().await.is_some() {
            break;
        }
        if plan.task.as_ref().map(|t| t.is_canceled()).unwrap_or(false) {
            canceled = true;
            break;
        }
        let rows = match message {
            Ok(page) => page.rows,
            Err(err) => {
                failure = Some(err);
                break;
            }
        };
        let got = rows.len();
        if got == 0 {
            continue;
        }
        if let Some(task) = &plan.task {
            task.add_rows_read(got as u64);
            // 只报「执行到哪一步」，行数在进度卡上平滑递增 —— 每页都写会刷屏
            if last_report.elapsed() >= PROGRESS_INTERVAL {
                task.set_phase(format!("表 {}：传输数据", plan.table));
                last_report = std::time::Instant::now();
            }
        }
        let mut rows = rows;
        // 行数上限按**剩余配额**截断：读协程可能整页拉回来，多出来的丢掉
        if plan.row_limit > 0 {
            let remaining = plan.row_limit.saturating_sub(consumed);
            if remaining == 0 {
                break;
            }
            if got as u64 > remaining {
                rows.truncate(remaining as usize);
                limit_hit = true;
                note.push_str(&format!(
                    "（已达行数上限 {}，仅同步了前 {} 行）",
                    plan.row_limit, plan.row_limit
                ));
            }
        }
        consumed += rows.len() as u64;

        if plan.upsert == UpsertMode::TwoPhase && !plan.key_indexes.is_empty() {
            // 两阶段 upsert：已有键 → 更新（攒批、串行），其余 → 插入池（并发）。
            // 原生 upsert 不走这里 —— 目标端一条语句自己判插入还是更新，行直接进写池。
            let existing = match lookup_existing_keys(&plan, &rows).await {
                Ok(keys) => keys,
                Err(msg) => {
                    failure = Some(msg);
                    break;
                }
            };
            let (existing, fresh): (Vec<_>, Vec<_>) = rows
                .into_iter()
                .partition(|row| existing.contains(&key_of(row, &plan.key_indexes)));
            if !existing.is_empty() {
                match writer.update(&existing).await {
                    Ok(rows) => updated += rows,
                    Err(msg) => {
                        failure = Some(format!("更新失败：{msg}"));
                        break;
                    }
                }
            }
            if !fresh.is_empty() && write_tx.send(fresh).await.is_err() {
                break;
            }
        } else if write_tx.send(rows).await.is_err() {
            break;
        }
        if limit_hit {
            break;
        }
    }

    // ---- 5. 收尾：残余更新 → 关写通道 → 等 writer → 等 reader --------------
    // 先丢掉 page_rx：还卡在 send 上的读协程立刻拿到错误并退出
    drop(page_rx);
    match writer.flush().await {
        Ok(rows) => updated += rows,
        Err(msg) => {
            if failure.is_none() {
                failure = Some(format!("更新失败：{msg}"));
            }
        }
    }
    drop(write_tx);
    for handle in writer_handles {
        // 写协程 panic（驱动异常/OOM）或超时未结束，都不能当成「写完了」：
        // 以前 `let _ = timeout(...)` 把这两种情况一起吞掉，少写的行毫无痕迹。
        match tokio::time::timeout(PIPE_WAIT, handle).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                if failure.is_none() && !canceled {
                    failure = Some("写入侧协程异常终止（驱动异常或内存不足），本次同步数据不完整".to_string());
                }
            }
            Err(_) => {
                if failure.is_none() && !canceled {
                    failure = Some("写入协程超时未结束，本次同步数据不完整".to_string());
                }
            }
        }
    }
    // 读协程 panic（驱动异常/OOM）不能被当成「读完了」：补一条失败
    for handle in reader_handles {
        if handle.await.is_err() && failure.is_none() && !canceled {
            failure = Some("读取侧协程异常终止（驱动异常或内存不足），本次同步数据不完整".to_string());
        }
    }

    let inserted = shared_inserted.load(Ordering::SeqCst);
    if let Some(msg) = write_err.lock().await.take() {
        return Err(msg);
    }
    if let Some(err) = failure {
        return Err(err);
    }
    if canceled {
        note.push_str("（已取消：已写入的部分保留，不回滚）");
    }
    if plan.upsert == UpsertMode::Native && inserted > 0 {
        // 原生 upsert 由服务端判插入还是更新，SQL 层面拿不到「新增/更新」的拆分
        note.push_str("（原生 upsert：新增/更新未区分）");
    }
    // 分发出去了却没算进 inserted/updated 的行 = 逐行降级后仍然失败的行（脏数据）。
    // 取消时 consumed 里还夹着没来得及写的页，差值没有意义，故置 0。
    let failed = if canceled {
        0
    } else {
        consumed.saturating_sub(inserted + updated)
    };
    if failed > 0 {
        // 必须写进结果消息：这些行**没有**落库，界面上的「成功」不能把这件事盖住。
        note.push_str(&format!("（{failed} 行写入失败，详见任务日志）"));
    }
    Ok(Stats {
        inserted,
        updated,
        failed,
        canceled,
        note,
    })
}
