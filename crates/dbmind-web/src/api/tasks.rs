//! 异步任务的公共骨架：导出 / 导入 / 整库转储 / 对比 / 同步 / 造数 共用。
//!
//! ## 为什么必须有这一层
//!
//!上游的长耗时操作全是同一套三件套：`POST` 提交拿 `taskId` → 轮询状态 → 可取消
//! （导出还多一步「下载产物」）。六个功能域各写一遍，就是六份进度字段、六份取消标志、
//! 六种「任务不存在」的措辞 —— 而前端的解析器只有一个：任何一处漂了，
//! 表现都是「进度条永远 0%」或「取消点了没反应」，且很难定位到是哪一块漂的。
//!
//! ## 四条约定
//!
//! 1. **快照字段名固定**：`taskId/status/done/total/phase/message/canceled/logs`，
//!    对齐上游的 `task.view()`。少一个字段，界面上就多一块空白。
//! 2. **`total = -1` 表示「总量未知」**，不要拿 0 表示未知 ——
//!    0 会被读成「总量是 0」，进度条直接跳到 100%，看起来像秒完成。
//! 3. **任务执行态不持久化，但终态快照落盘**：进程重启后 running 的任务即消失
//!    （上游同样如此，TTL 30 分钟后回收内存条目）；**进入终态的任务**把快照
//!    （含 result）写到 `<home>/tasks/<id>.json`，重启后 `get` 从磁盘恢复 ——
//!    前端任务中心的执行记录始终可查看，不再因服务重启变成「已中断」。
//! 4. **取消是协作式的**：`cancel()` 只落一个标志，由工作循环在分页/分批之间检查。
//!    强杀线程的代价（连接状态、半截文件）比多跑一批大得多。

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};

/// 任务条目在内存里保留多久（过期只回收条目，产物文件不删）。
pub const TASK_TTL: Duration = Duration::from_secs(30 * 60);

/// 每个任务最多保留多少行日志（界面上就是个滚动文本框，攒太多只是占内存）。
const MAX_LOGS: usize = 200;

/// 卡死看门狗的**心跳间隔**（见 `TaskRegistry::spawn`）。
const STALL_TICK: Duration = Duration::from_secs(30);

/// 多久**毫无进展**才判定卡死 —— 这是**最后一道兜底**，不是主机制。
///
/// 主机制是**每一步的真实截止时间**：数据库调用有（内核发宿主的是 `timeoutMs + 2s`，
/// 超时即 `QueryTimeout`，用户取消还会经 `Statement.cancel()` 打到驱动），流水线内部的
/// 交接也有（见 `sync.rs` 的 `PIPE_WAIT`：发页/取批/等协程/等并发槽位）。
///
/// 兜底存在的理由：万一某条路径漏了截止时间（新写的循环、别的任务类型、我们自己代码里
/// 的锁等待），也不该让界面永远挂在「任务仍在后台收尾」上。它触发时会在任务日志里
/// **明确写成兜底**（提示这条路径缺少截止时间），好据此把真正的截止时间补上 —— 所以它
/// 既是保险，也是一个"该修哪里"的探针。时间放宽到 30 分钟，避免把慢任务误收。
const STALL_LIMIT: Duration = Duration::from_secs(30 * 60);

/// 任务生命周期。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    Running,
    Success,
    Error,
    Canceled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Running => "running",
            TaskStatus::Success => "success",
            TaskStatus::Error => "error",
            TaskStatus::Canceled => "canceled",
        }
    }
}

/// 任务终态快照目录：`<home>/tasks/`。
///
/// 跟用户数据走（`home_dir`），**不是**临时目录 —— 这些快照要跨重启存活：
/// 任务收尾时 `persist_task` 落盘，重启后 [`TaskRegistry::get`] 从磁盘恢复，
/// 前端任务中心的执行记录在服务重启后依然可以查看结果。
fn task_store_dir() -> PathBuf {
    let dir = dbmind_core::paths::home_dir().join("tasks");
    let _ = dbmind_core::paths::ensure_dir(&dir);
    dir
}

/// 任务 id 是否能安全地用作快照文件名（id 来自 URL，必须挡住路径穿越）。
fn safe_task_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 终态任务快照落盘：`<home>/tasks/<id>.json`（`snapshot()` 全量 + `result`）。
///
/// **为什么要落**：任务都在内存里，服务一重启执行记录就只剩「已中断」，
/// 用户点查看什么都看不到（真机反馈）。终态时落一份盘（含 result ——
/// 查看时从它恢复结果界面），重启后从磁盘原样恢复。失败静默：
/// 落盘只是兜底，不能让任务收尾被 IO 问题卡住。
fn persist_task(task: &Task) {
    let mut snap = task.snapshot();
    // snapshot() 不带 kind（排障字段），落盘时补上，恢复后 kind 仍完整
    snap["kind"] = Value::String(task.kind.clone());
    if let Some(result) = task.result() {
        // result 里可能带大样本（对比的差异数据）—— 磁盘不用像 localStorage
        // 那样抠搜，原样落（快照文件按任务一生命周期一份，不会堆积增长）
        snap["result"] = result;
    }
    let path = task_store_dir().join(format!("{}.json", task.id));
    if let Ok(body) = serde_json::to_vec(&snap) {
        let _ = std::fs::write(path, body);
    }
}

/// 任务产物：一个落在磁盘上的文件（导出结果、备份文件）。
///
/// 为什么落磁盘而不是留在内存里：导出的产物动辄几百 MB，
/// 放在内存里等于让「数据量」决定进程内存占用 —— 并发两个人导出就能把服务压死。
#[derive(Clone)]
pub struct Artifact {
    pub path: PathBuf,
    /// 下载时要回的 Content-Type（在任务里存好，下载接口不必再猜一遍）。
    pub media_type: String,
    /// 给用户看到的文件名（带扩展名）。
    pub filename: String,
}

/// 一个任务的运行时状态。
pub struct Task {
    pub id: String,
    /// 任务类别（`export` / `import` / `dump` / `compare` / `sync` / `datagen`），排障用。
    pub kind: String,
    status: Mutex<TaskStatus>,
    canceled: AtomicBool,
    done: Mutex<u64>,
    // 行级实时计数（Navicat 式「读取 / 传输 / 错误」三个数）：done 是**对象数**，
    // 界面上那条「数字在一万一万跳」的痛就是拿对象数/终态 summary 凑的行数 ——
    // 现在按行累加，前端 0.8 秒轮询一次，数字就是平滑递增的
    rows_read: Mutex<u64>,
    rows_written: Mutex<u64>,
    rows_failed: Mutex<u64>,
    /// -1 = 未知（见文件头约定 2）
    total: Mutex<i64>,
    phase: Mutex<String>,
    message: Mutex<String>,
    logs: Mutex<Vec<String>>,
    /// 累计写过的日志行数（只增不减）。
    ///
    /// 为什么需要它：`logs` 会被 MAX_LOGS 裁剪（丢最旧的），长度到上限后就**再也不变长** ——
    /// 前端只靠"数组变长了"判断有没有新行的话，日志区就永远停在那里不动
    /// （导出进度正是这么卡住的）。有了单调序号，前端可以按序号只追加新行。
    logs_seq: AtomicU64,
    result: Mutex<Option<Value>>,
    artifact: Mutex<Option<Artifact>>,
    /// 任务上下文（库名、表名等「这条任务是干什么的」信息）。
    ///
    /// 有些域的进度快照自带 `database`/`table` 字段（造数的前端就读它），
    /// 而任务本身只管进度数字 —— 于是把这类信息挂在上下文里，
    /// 由各域的 status 处理器按自己的形状取出来用。
    context: Mutex<Option<Value>>,
    /// 「需要用户先做个决定」的提示（备份/还原缺命令行工具时用）。
    ///
    /// 任务会**真的停在这里等**：界面上的三选（自动安装 / 我已装好重试 / 跳过用内置）
    /// 才叫真的三选。没有这个机制的话，所谓三选就只是三个没人等的按钮。
    install_prompt: Mutex<Option<Value>>,
    install_decision: Mutex<Option<String>>,
    created: Instant,
    /// 进入终态那一刻的运行时长（毫秒）：elapsedMs 的冻结值 —— 终态后不再走时钟
    finished_elapsed: Mutex<Option<u64>>,
    /// 任务的**墙钟**起止（毫秒时间戳）：前端任务中心的开始/结束时间直接用它们，
    /// 不再由前端拿登记时刻 + 运行时长去推（推算链路任何一环偏差都会显示错，真机踩过）
    started_wall: u64,
    finished_wall: Mutex<Option<u64>>,
}

impl Task {
    fn new(id: String, kind: String) -> Self {
        Self {
            id,
            kind,
            status: Mutex::new(TaskStatus::Running),
            canceled: AtomicBool::new(false),
            done: Mutex::new(0),
            rows_read: Mutex::new(0),
            rows_written: Mutex::new(0),
            rows_failed: Mutex::new(0),
            total: Mutex::new(-1),
            phase: Mutex::new(String::new()),
            message: Mutex::new(String::new()),
            logs: Mutex::new(Vec::new()),
            logs_seq: AtomicU64::new(0),
            result: Mutex::new(None),
            artifact: Mutex::new(None),
            context: Mutex::new(None),
            install_prompt: Mutex::new(None),
            install_decision: Mutex::new(None),
            created: Instant::now(),
            finished_elapsed: Mutex::new(None),
            started_wall: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64).unwrap_or(0),
            finished_wall: Mutex::new(None),
        }
    }

    /// 从**磁盘快照**重建任务（服务重启后 [`TaskRegistry::get`] 的兜底）。
    ///
    /// 还原的是查看所需的终态字段（状态/计数/消息/日志/result/耗时/墙钟）。
    /// 这是「只读空壳」：没有工作体，不能再跑也不能真取消 ——
    /// `cancel()` 之类对它调了也是无效果（状态已是终态）。
    ///
    /// 只恢复**终态**快照：重启瞬间还标着 running 的任务是被打断的，
    /// 恢复成 running 只会让前端无限轮询下去。
    fn restore_from_snapshot(id: &str, snap: &Value) -> Option<Self> {
        let status_str = snap.get("status").and_then(Value::as_str)?;
        let status = match status_str {
            "success" => TaskStatus::Success,
            "error" => TaskStatus::Error,
            "canceled" => TaskStatus::Canceled,
            _ => return None,
        };
        let num = |key: &str| snap.get(key).and_then(Value::as_u64).unwrap_or(0);
        let text = |key: &str| {
            snap.get(key).and_then(Value::as_str).unwrap_or("").to_string()
        };
        let logs: Vec<String> = snap
            .get("logs")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        let logs_seq = snap
            .get("logsSeq")
            .and_then(Value::as_u64)
            .unwrap_or(logs.len() as u64);
        Some(Self {
            id: id.to_string(),
            kind: text("kind"),
            status: Mutex::new(status),
            canceled: AtomicBool::new(status_str == "canceled"),
            done: Mutex::new(num("done")),
            rows_read: Mutex::new(num("rowsRead")),
            rows_written: Mutex::new(num("rowsWritten")),
            rows_failed: Mutex::new(num("rowsFailed")),
            total: Mutex::new(snap.get("total").and_then(Value::as_i64).unwrap_or(-1)),
            phase: Mutex::new(text("phase")),
            message: Mutex::new(text("message")),
            logs: Mutex::new(logs),
            logs_seq: AtomicU64::new(logs_seq),
            result: Mutex::new(snap.get("result").cloned()),
            artifact: Mutex::new(None),
            context: Mutex::new(None),
            install_prompt: Mutex::new(None),
            install_decision: Mutex::new(None),
            created: Instant::now(),
            // 耗时冻结在快照值（终态任务不再走时钟）；墙钟起止照快照还原
            finished_elapsed: Mutex::new(Some(num("elapsedMs"))),
            started_wall: num("startedAtWall"),
            finished_wall: Mutex::new(Some(num("finishedAtWall"))),
        })
    }

    /// 挂起等待用户决策（提示内容会随任务状态回给界面）。
    pub fn set_install_prompt(&self, prompt: Value) {
        *lock(&self.install_prompt) = Some(prompt);
    }

    pub fn clear_install_prompt(&self) {
        *lock(&self.install_prompt) = None;
    }

    pub fn install_prompt(&self) -> Option<Value> {
        lock(&self.install_prompt).clone()
    }

    /// 用户的选择（`confirm` / `manual` / `skip`）。
    pub fn decide_install(&self, decision: &str) {
        *lock(&self.install_decision) = Some(decision.to_string());
    }

    pub fn install_decision(&self) -> Option<String> {
        lock(&self.install_decision).clone()
    }

    /// 等一个决策：要么用户点了按钮，要么超时。
    ///
    /// 超时的默认动作是「跳过、回退内置引擎」——一个没人理的弹窗不该把任务永久挂住，
    /// 而回退内置引擎至少能把备份做出来（这正是「跳过」按钮的语义）。
    pub async fn await_install_decision(&self, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(decision) = self.install_decision() {
                return decision;
            }
            if self.is_canceled() {
                return "cancel".to_string();
            }
            if Instant::now() >= deadline {
                return "skip".to_string();
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// 记录任务上下文（建任务后立刻调用，进度查询时读）。
    pub fn set_context(&self, context: Value) {
        *lock(&self.context) = Some(context);
    }

    pub fn context(&self) -> Option<Value> {
        lock(&self.context).clone()
    }

    pub fn status(&self) -> TaskStatus {
        *lock(&self.status)
    }

    pub fn set_status(&self, status: TaskStatus) {
        // 进入终态时**冻结运行时长**：elapsedMs 的口径是"任务跑了多久"，
        // 若一直用 created.elapsed()，任务结束后数字还会跟着时钟涨
        //（前端统计卡/任务中心就出现"都成功了耗时还在加"——真机踩过）
        if matches!(status, TaskStatus::Success | TaskStatus::Error | TaskStatus::Canceled) {
            let mut fin = lock(&self.finished_elapsed);
            if fin.is_none() {
                *fin = Some(self.created.elapsed().as_millis() as u64);
            }
            let mut wall = lock(&self.finished_wall);
            if wall.is_none() {
                *wall = Some(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64).unwrap_or(0));
            }
        }
        *lock(&self.status) = status;
    }

    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::SeqCst);
    }

    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::SeqCst)
    }

    /// 已取消 ⇒ 抛出一句人话（工作循环在每个可中断点调用它）。
    pub fn check_canceled(&self) -> Result<(), String> {
        if self.is_canceled() {
            return Err("任务已取消".to_string());
        }
        Ok(())
    }

    pub fn set_done(&self, done: u64) {
        *lock(&self.done) = done;
    }
    /// 行级实时计数（读取/写入/失败行数），同步等按行推进的任务用。
    pub fn add_rows_read(&self, n: u64) { *lock(&self.rows_read) += n; }
    pub fn add_rows_written(&self, n: u64) { *lock(&self.rows_written) += n; }
    pub fn add_rows_failed(&self, n: u64) { *lock(&self.rows_failed) += n; }
    pub fn rows_read(&self) -> u64 { *lock(&self.rows_read) }
    /// 已写入行数（看门狗判「还有没有进展」用）。
    pub fn rows_written(&self) -> u64 { *lock(&self.rows_written) }

    pub fn add_done(&self, delta: u64) {
        let mut done = lock(&self.done);
        *done += delta;
    }

    pub fn done(&self) -> u64 {
        *lock(&self.done)
    }

    pub fn set_total(&self, total: i64) {
        *lock(&self.total) = total;
    }

    pub fn set_phase(&self, phase: impl Into<String>) {
        *lock(&self.phase) = phase.into();
    }

    /// 当前状态说明（**同时落一条日志**：界面上既显示当前阶段，也保留过程流水）。
    ///
    /// 为什么不分开调用：这两个东西总是同时更新，分开写迟早有人只更新一个，
    /// 于是「阶段是表 3/8、日志停在表 1」这种自相矛盾的快照会出现。
    pub fn step(&self, phase: impl Into<String>) {
        let phase = phase.into();
        *lock(&self.phase) = phase.clone();
        self.log(phase);
    }

    pub fn log(&self, line: impl Into<String>) {
        let mut logs = lock(&self.logs);
        logs.push(line.into());
        self.logs_seq.fetch_add(1, Ordering::Relaxed);
        if logs.len() > MAX_LOGS {
            let overflow = logs.len() - MAX_LOGS;
            logs.drain(0..overflow);
        }
    }

    /// 日志快照（按写入顺序，最多 [`MAX_LOGS`] 行）。
    ///
    /// 界面上的滚动日志框就是它 —— **不下发的话过程就全被吞了**：
    /// 备份曾因此只在界面上看到「开始任务 / 备份完成 / 汇总」三两行，
    /// 「每张表导了多少行、视图/例程有没有在内」全都无从对账。
    pub fn logs(&self) -> Vec<String> {
        lock(&self.logs).clone()
    }

    /// 日志的**累计行数**（含被 [`MAX_LOGS`] 挤掉的）。前端据此只追加新行，不重复渲染。
    pub fn logs_seq(&self) -> u64 {
        self.logs_seq.load(Ordering::Relaxed)
    }

    pub fn set_message(&self, message: impl Into<String>) {
        *lock(&self.message) = message.into();
    }

    pub fn set_result(&self, result: Value) {
        *lock(&self.result) = Some(result);
    }

    pub fn result(&self) -> Option<Value> {
        lock(&self.result).clone()
    }

    pub fn set_artifact(&self, artifact: Artifact) {
        *lock(&self.artifact) = Some(artifact);
    }

    pub fn artifact(&self) -> Option<Artifact> {
        lock(&self.artifact).clone()
    }

    pub fn expired(&self) -> bool {
        self.created.elapsed() > TASK_TTL
    }

    /// 状态快照（上游的 `task.view()` 形状）。
    pub fn snapshot(&self) -> Value {
        json!({
            "taskId": self.id,
            "status": self.status().as_str(),
            "done": *lock(&self.done),
            "total": *lock(&self.total),
            // 行级实时计数：运行中就有（不等终态 summary），前端统计卡据此跳动
            "rowsRead": *lock(&self.rows_read),
            "rowsWritten": *lock(&self.rows_written),
            "rowsFailed": *lock(&self.rows_failed),
            "phase": lock(&self.phase).clone(),
            "message": lock(&self.message).clone(),
            "canceled": self.is_canceled(),
            "logs": lock(&self.logs).clone(),
            // 单调序号：前端据此判断"有没有新行"，不受 MAX_LOGS 裁剪影响
            "logsSeq": self.logs_seq.load(Ordering::Relaxed),
            // 任务**已运行时长**（毫秒）：终态后取冻结值（不再跟时钟走）；
            // 前端从任务中心/悬浮恢复进度窗时，耗时卡用它续算（开始墙钟 = now - elapsedMs）
            "elapsedMs": lock(&self.finished_elapsed)
                .unwrap_or_else(|| self.created.elapsed().as_millis() as u64),
            // **墙钟起止**：任务中心执行记录的开始/结束时间直接用（权威值，不由前端推算）
            "startedAtWall": self.started_wall,
            "finishedAtWall": *lock(&self.finished_wall),
        })
    }
}

/// 互斥锁中毒不该把请求带崩：数据是「进度数字 + 文本」，没有跨字段不变量，
/// 恢复出来的值最多是晚一拍，比 500 好得多。
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 任务注册表（放在 `AppState` 里，全局一份）。
pub struct TaskRegistry {
    tasks: Mutex<HashMap<String, Arc<Task>>>,
    seq: AtomicU64,
}

impl Default for TaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(1),
        }
    }

    /// 新建任务。`prefix` 是上游的 id 前缀（`exp_` / `dump_` / `imp_` / `cmp_` / `sync_` / `datagen_`）。
    ///
    /// id 里的时间戳用**紧凑且文件系统安全**的写法 —— 它同时会被用作产物文件名，
    /// 带上 `:` 或 `.` 在 Windows 上直接写不出文件。
    pub fn create(&self, kind: &str, prefix: &str) -> Arc<Task> {
        self.gc();
        let stamp = chrono::Local::now().format("%Y%m%d%H%M%S");
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let id = format!("{prefix}{stamp}_{seq}");
        let task = Arc::new(Task::new(id.clone(), kind.to_string()));
        lock(&self.tasks).insert(id, task.clone());
        task
    }

    pub fn get(&self, id: &str) -> Option<Arc<Task>> {
        if let Some(task) = lock(&self.tasks).get(id).cloned() {
            return Some(task);
        }
        // 内存没有（服务重启丢了 / TTL 到期被 GC）→ **磁盘快照兜底**：
        // 终态任务收尾时落过盘（见 `persist_task`），从这里重建一个「只读空壳」
        // 挂回注册表 —— status 接口照常返回，前端执行记录查看无缝恢复结果界面。
        // id 来自 URL，先过文件名安全检查（防路径穿越）
        if !safe_task_id(id) {
            return None;
        }
        let path = task_store_dir().join(format!("{id}.json"));
        let body = std::fs::read(path).ok()?;
        let snap: Value = serde_json::from_slice(&body).ok()?;
        let task = Arc::new(Task::restore_from_snapshot(id, &snap)?);
        lock(&self.tasks).insert(id.to_string(), task.clone());
        Some(task)
    }

    /// 回收过期条目（新建任务时顺手做，不必起后台线程）。
    pub fn gc(&self) {
        lock(&self.tasks).retain(|_, task| !task.expired());
    }

    /// 起一个后台任务。
    ///
    /// 工作体是 **async**（不是线程）：它要调用元数据/执行这些 async helper，
    /// 而那套 helper 内部已经把阻塞调用甩进 blocking 线程池了 —— 再套一层线程只会多一次同步。
    pub fn spawn<W, F>(&self, kind: &str, prefix: &str, work: W) -> Arc<Task>
    where
        W: FnOnce(Arc<Task>) -> F + Send + 'static,
        F: Future<Output = Result<Option<Value>, String>> + Send + 'static,
    {
        let task = self.create(kind, prefix);
        let running = task.clone();
        let handle = tokio::spawn(async move {
            // 兜底守卫：工作体若 panic，状态不会永远停在 running ——
            // 那会让界面无限轮询下去，比直接报错难受得多。
            let guard = FinishGuard(running.clone(), false);
            let outcome = work(running).await;
            guard.finish(outcome);
        });
        // ==================== 卡死看门狗 ====================
        //
        // 为什么必须有这一层：`cancel()` 只是落一个标志位，工作体是在**分页/分批之间**
        // 才查得到它 —— 一旦工作体卡在某个 await 上（最典型：网络切换、对端掉线之后，
        // 某次读或写再也回不来），标志位永远没机会被看到，任务就永远停在 running：
        // 界面上显示「任务仍在后台收尾，已允许关闭窗口」，关掉窗口也只是关掉窗口，
        // 后台那条协程仍然挂着，刷新页面依旧能看到它 —— 真机就这么挂过一次（同步任务
        // 21 分钟无进展，宿主进程 0% CPU、连一条数据库连接都没有）。
        //
        // 判据只看**进展**（完成数 / 读取数 / 写入数 / 日志序号）：这一整段时间里一个
        // 都不动，就说明它不是在慢慢干，而是卡住了。真在干活的慢任务按分页粒度更新计数，
        // 不会误判。
        //
        // 判死后三件事：① 把原因写进任务日志（用户能看到为什么，而不是干等）；
        // ② 落终态 error（前端不再无限轮询，进度卡与任务中心都收口）；
        // ③ `abort()` 真正中止工作体 —— 只是改状态而不停协程，就成了"假结束"。
        let watch = task.clone();
        tokio::spawn(async move {
            let stamp = |t: &Task| (t.done(), t.rows_read(), t.rows_written(), t.logs_seq());
            let mut last = stamp(&watch);
            let mut idle = Duration::ZERO;
            loop {
                tokio::time::sleep(STALL_TICK).await;
                if watch.status() != TaskStatus::Running {
                    return;
                }
                let now = stamp(&watch);
                if now == last {
                    idle += STALL_TICK;
                } else {
                    last = now;
                    idle = Duration::ZERO;
                }
                if idle >= STALL_LIMIT {
                    let minutes = STALL_LIMIT.as_secs() / 60;
                    watch.log(format!(
                        "任务已连续 {minutes} 分钟没有任何进展，兜底看门狗判定为卡死——\
                         这条路径缺少截止时间（正常应由某一步的超时先报出来），\
                         请把本行日志反馈给开发；已强制结束，已写入的部分保留、不回滚"
                    ));
                    watch.set_status(TaskStatus::Error);
                    watch.set_message(format!(
                        "任务长时间无进展（{minutes} 分钟），已强制结束：连接可能已断开"
                    ));
                    persist_task(&watch);
                    handle.abort();
                    return;
                }
            }
        });
        task
    }
}

/// 见 `TaskRegistry::spawn` 的说明。
struct FinishGuard(Arc<Task>, bool);

impl FinishGuard {
    fn finish(mut self, outcome: Result<Option<Value>, String>) {
        let task = self.0.clone();
        match outcome {
            Ok(result) => {
                // **只在工作体没设过结果时才用返回值兜底**。
                //
                // 这里踩过一次坑：备份任务在工作过程中 `set_result` 写入了完整结果
                // （含「哪些对象没导出」的 notices），最后 `return Ok(Some(一句摘要))`
                // 又把它覆盖掉了 —— 界面上「备份完成」一切正常，而那几条重要提示无声消失。
                // 约定：显式设置过就以它为准，返回值只是「没设过时的兜底」。
                if task.result().is_none() {
                    if let Some(result) = result {
                        task.set_result(result);
                    }
                }
                if task.is_canceled() {
                    task.set_status(TaskStatus::Canceled);
                    task.set_message("任务已取消");
                } else {
                    task.set_status(TaskStatus::Success);
                    if lock(&task.message).is_empty() {
                        task.set_message("完成");
                    }
                }
            }
            Err(message) => {
                // 取消走的是 Err("任务已取消") 这条路，语义上不是失败，别报成红色错误
                if task.is_canceled() || message == "任务已取消" {
                    task.set_status(TaskStatus::Canceled);
                } else {
                    task.set_status(TaskStatus::Error);
                }
                task.set_message(message.clone());
                task.log(message);
            }
        }
        // **终态快照落盘**：重启后任务中心执行记录仍可查看（见 persist_task 说明）
        persist_task(&task);
        // 标记「已处理」，让 Drop 不再改状态
        self.1 = true;
    }
}

impl Drop for FinishGuard {
    fn drop(&mut self) {
        if !self.1 && self.0.status() == TaskStatus::Running {
            self.0.set_status(TaskStatus::Error);
            self.0.set_message("任务异常终止（工作线程崩溃）");
            // 异常终止也是终态，同样落盘（前端能看到「崩溃终止」而不是查无此任务）
            persist_task(&self.0);
        }
    }
}

/// 产物目录：系统临时目录下的 `dbmind-exports`。
///
/// 这里只是**中转**：异步导出把文件写在此处，前端取回（GET /export/download/{taskId}）
/// 之后由用户自己选保存位置，**取回即删**。
///
/// 以前它是 `<home>/exports`（设置页还能改），但导出既然已经交给用户选位置，
/// 服务端再留一份用户可见的副本只会堆积（实测攒过好几个 20MB+ 的文件），
/// 还会再弹一句"已保存到 …"。现在改成临时目录，用户机器上看不到任何残留。
///
/// 进程启动后的第一次调用会把整个目录清掉：上一次运行留下的残片
/// （任务被取消、服务中途退出）不该带到下一次。
pub fn export_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("dbmind-exports");
    static SWEEP: std::sync::Once = std::sync::Once::new();
    SWEEP.call_once(|| {
        let _ = std::fs::remove_dir_all(&dir);
    });
    dir
}

/// 产物文件路径（`<home>/exports/<taskId>.<ext>`）。
pub fn artifact_path(task_id: &str, ext: &str) -> Result<PathBuf, String> {
    let dir = export_dir();
    dbmind_core::paths::ensure_dir(&dir).map_err(|e| format!("无法创建导出目录 {}：{e}", dir.display()))?;
    Ok(dir.join(format!("{task_id}.{ext}")))
}

/// `Content-Disposition: attachment` 的值。
///
/// 中文文件名必须走 `filename*=UTF-8''<百分号编码>`：直接写原文，浏览器会按
/// latin-1 解出乱码文件名；而只给 `filename=` 的英文回退值，下载下来就叫 `download`。
pub fn content_disposition(filename: &str) -> String {
    let encoded: String = filename
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect();
    format!("attachment; filename=\"{}\"; filename*=UTF-8''{}", ascii_fallback(filename), encoded)
}

/// 给不支持 `filename*` 的老客户端的纯 ASCII 回退名。
fn ascii_fallback(filename: &str) -> String {
    let cleaned: String = filename
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    if cleaned.trim_matches('_').is_empty() {
        "download".to_string()
    } else {
        cleaned
    }
}

/// 时间戳后缀（产物文件名用）。
pub fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
}
