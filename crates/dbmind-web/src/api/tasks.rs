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
//! 3. **任务不持久化**：进程重启即消失（上游同样如此）。TTL 30 分钟后回收内存条目，
//!    **磁盘上的产物文件保留** —— 用户还能从导出目录里把文件拿走。
//! 4. **取消是协作式的**：`cancel()` 只落一个标志，由工作循环在分页/分批之间检查。
//!    强杀线程的代价（连接状态、半截文件）比多跑一批大得多。

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// 任务条目在内存里保留多久（过期只回收条目，产物文件不删）。
pub const TASK_TTL: Duration = Duration::from_secs(30 * 60);

/// 每个任务最多保留多少行日志（界面上就是个滚动文本框，攒太多只是占内存）。
const MAX_LOGS: usize = 200;

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
}

impl Task {
    fn new(id: String, kind: String) -> Self {
        Self {
            id,
            kind,
            status: Mutex::new(TaskStatus::Running),
            canceled: AtomicBool::new(false),
            done: Mutex::new(0),
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
        }
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
            "phase": lock(&self.phase).clone(),
            "message": lock(&self.message).clone(),
            "canceled": self.is_canceled(),
            "logs": lock(&self.logs).clone(),
            // 单调序号：前端据此判断"有没有新行"，不受 MAX_LOGS 裁剪影响
            "logsSeq": self.logs_seq.load(Ordering::Relaxed),
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
        lock(&self.tasks).get(id).cloned()
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
        tokio::spawn(async move {
            // 兜底守卫：工作体若 panic，状态不会永远停在 running ——
            // 那会让界面无限轮询下去，比直接报错难受得多。
            let guard = FinishGuard(running.clone(), false);
            let outcome = work(running).await;
            guard.finish(outcome);
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
        // 标记「已处理」，让 Drop 不再改状态
        self.1 = true;
    }
}

impl Drop for FinishGuard {
    fn drop(&mut self) {
        if !self.1 && self.0.status() == TaskStatus::Running {
            self.0.set_status(TaskStatus::Error);
            self.0.set_message("任务异常终止（工作线程崩溃）");
        }
    }
}

/// 产物目录：`<home>/exports`。
/// 导出目录设置文件（一行一个路径）。
///
/// 为什么不放进内核的设置表：`export_dir()` 是个**拿不到 AppState** 的纯函数
/// （下载、产物路径、清理都在调它），而设置表要走 engine 句柄。导出目录本质就是
/// "这台机器上的一个路径"，用一行文件表达最直接，不必为它把 engine 穿得到处都是。
pub fn export_dir_file() -> PathBuf {
    dbmind_core::paths::home_dir().join("export-dir.txt")
}

/// 用户在设置页指定的导出目录（没设过给 None）。
pub fn configured_export_dir() -> Option<PathBuf> {
    let text = std::fs::read_to_string(export_dir_file()).ok()?;
    let line = text.lines().next()?.trim().to_string();
    if line.is_empty() {
        None
    } else {
        Some(PathBuf::from(line))
    }
}

/// 设置导出目录（空串 = 恢复默认）。返回生效后的目录。
pub fn set_export_dir(dir: &str) -> Result<PathBuf, String> {
    let dir = dir.trim();
    if dir.is_empty() {
        let _ = std::fs::remove_file(export_dir_file());
        return Ok(export_dir());
    }
    let path = PathBuf::from(dir);
    dbmind_core::paths::ensure_dir(&path)
        .map_err(|e| format!("无法创建导出目录 {}：{e}", path.display()))?;
    std::fs::write(export_dir_file(), path.display().to_string())
        .map_err(|e| format!("无法保存导出目录设置：{e}"))?;
    Ok(path)
}

/// 产物目录：用户在设置页指定的优先，否则 `<home>/exports`。
pub fn export_dir() -> PathBuf {
    configured_export_dir().unwrap_or_else(|| dbmind_core::paths::home_dir().join("exports"))
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
