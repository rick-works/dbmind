//! `/api/{module}/query/…` —— SQL 执行、多段执行、取消、执行 SQL 文件。
//!
//! 三处与内核语义对齐的关键点：
//!
//! 1. **执行失败返回 200 + `success:false`**，不是 4xx/5xx。上游的结果页要把
//!    数据库端原始错误、耗时、出错语句一起渲染出来；走 HTTP 错误码会让它落到
//!    「网络请求失败」那个分支，用户就看不到真正的原因了。传输层问题才用非 2xx。
//! 2. **多段执行在服务端切句后顺序执行**（`BatchQueryResult`）。内核的安全闸门
//!    **只允许一次一条**（多语句是明确的拒绝项），所以「一次提交多段」只能这么实现 ——
//!    这是刻意的取舍，不是偷懒：放弃那条闸门换来的只是少几次往返。
//! 3. **执行 SQL 文件是有进度的后台任务**，不是一次长请求：文件可能几百条语句，
//!    需要实时日志、可取消。任务存在本进程内（重启即失效，界面会提示任务不存在）。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use axum::extract::{Path, State};
use axum::Json;
use dbmind_core::{AccessContext, DbMindEngine, QueryOptions, QueryRequest};
use serde_json::{json, Value};

use crate::api::error::{XError, XResult};
use crate::api::{blocking, require_record, shape, Params};
use crate::AppState;

#[derive(Default)]
struct Task {
    status: String,
    done: usize,
    total: usize,
    phase: String,
    message: String,
    canceled: bool,
    logs: Vec<String>,
    /// 当前正在执行的 executionId —— 取消时用它把在途语句硬中断掉。
    current_execution: Option<String>,
}

fn tasks() -> &'static Mutex<HashMap<String, Task>> {
    static TASKS: OnceLock<Mutex<HashMap<String, Task>>> = OnceLock::new();
    TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn task_snapshot(task_id: &str) -> XResult<Value> {
    let guard = tasks().lock().unwrap_or_else(|e| e.into_inner());
    let task = guard
        .get(task_id)
        .ok_or_else(|| XError::new(axum::http::StatusCode::NOT_FOUND, "任务不存在或已过期"))?;
    Ok(json!({
        "success": true,
        "taskId": task_id,
        "status": task.status,
        "done": task.done,
        "total": task.total,
        "phase": task.phase,
        "message": task.message,
        "canceled": task.canceled,
        "logs": task.logs,
    }))
}

/// `POST /api/{m}/query/{id}` —— 执行一条（或用户粘贴的多段）SQL。
pub async fn execute(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let sql = body
        .get("sql")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if sql.trim().is_empty() {
        return Err(XError::bad_request("SQL 不能为空"));
    }
    let execution_id = body
        .get("executionId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let size = body
        .get("size")
        .and_then(Value::as_u64)
        .unwrap_or(2000)
        .clamp(1, 100_000) as usize;
    // 页码（从 1 起）。以前只读 size、完全没用 page —— 于是「第 2 页」拿到的还是前 size 行，
    // 界面看着有分页器，点下一页却一直是同一批数据。
    let page = body.get("page").and_then(Value::as_u64).unwrap_or(1).max(1);

    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // 先切库（跨库浏览靠影子连接，见 `scope`），再确保驱动在位。
    // 两条失败都把原因放进结果面板（与 SQL 出错同一处显示），而不是抛 503 让前端只说「请求失败」。
    let target = match crate::api::scope::resolve(&state, &id, &database).await {
        Ok(target) => target,
        Err(err) => return Ok(Json(shape::query_failure_json(&err.message, 0))),
    };
    if let Err(err) = crate::api::driver::ensure_for_connection(&state, &target).await {
        return Ok(Json(shape::query_failure_json(&err.message, 0)));
    }

    // 第 2 页起才套分页壳：第一页照原样执行（否则 create / update 这类语句会被塞进
    // 子查询里直接报错），并且只对「取数类」语句套。
    let base_sql = sql.clone();
    let effective = if page > 1 && looks_like_query(&base_sql) {
        let record = require_record(&state, &id).await?;
        let dialect = crate::api::dialect::Dialect::new(record.kind());
        crate::api::export::paging_sql(&base_sql, (page - 1) * size as u64, size as u64, dialect)
    } else {
        base_sql.clone()
    };

    let engine = state.engine.clone();
    let request = QueryRequest {
        read_only: None,
        connection: target,
        sql: effective,
        options: QueryOptions {
            max_rows: size,
            timeout_ms: 120_000,
        },
        execution_id,
        session: Some("ui:上游".to_string()),
    };
    Ok(Json(match blocking(move || engine.execute(request, AccessContext::Web)).await {
        Ok(result) => {
            let mut json = shape::query_result_json(&result);
            // 总数：只有「被截断」时才需要真去统计 —— 没截断说明返回的就是全部，行数本身就是总数。
            // 统计失败或语句不适合统计（带 order by 等）就当未知，保持 -1：
            // 界面会退化成「已返回 N 行」，绝不瞎报一个数。
            // 只要翻过页就要统计：最后一页往往不足一页（truncated=false），
            // 那时若拿本页行数当总数，分页器会突然从 1026 页缩成 1 页。
            if result.truncated || page > 1 {
                if let Some(total) = crate::api::export::count_rows(&state, &id, &database, &base_sql).await {
                    if let Some(object) = json.as_object_mut() {
                        object.insert("totalCount".to_string(), Value::from(total));
                        // hasMore 是"这一页之后还有没有"，不是"本页有没有装满"：
                        // 末页恰好装满时后者会误报"还有"。有总数就能算准。
                        object.insert(
                            "hasMore".to_string(),
                            {
                                // 先绑到变量：直接写 x as u64 < y 会被当成泛型参数（u64<…>）而编译失败
                                let limit = page * size as u64;
                                Value::from(total >= 0 && limit < total as u64)
                            },
                        );
                    }
                }
            }
            json
        }
        Err(err) => shape::query_failure_json(&err.message, 0),
    }))
}

/// 看起来是不是「取数」语句 —— 只有这类才适合套分页壳。
///
/// 不解析 SQL，只看首个词：套壳是为了分页，DDL/DML 套进去只会报错，
/// 而它们在「第几页」这个问题上没有意义。
fn looks_like_query(sql: &str) -> bool {
    let head = sql
        .trim_start()
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        head.as_str(),
        "select" | "with" | "show" | "describe" | "desc" | "explain"
    )
}

/// `POST /api/{m}/query/{id}/batch` —— 多段执行，每段一个结果（结果1/结果2…）。
pub async fn execute_batch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let sql = body
        .get("sql")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if sql.trim().is_empty() {
        return Err(XError::bad_request("SQL 不能为空"));
    }
    let execution_id = body
        .get("executionId")
        .and_then(Value::as_str)
        .map(str::to_string);

    // 切句用内核的实现（按协议分派：SQL 引号/注释内的分号不切，Redis 一行一条，ES 一行一请求）
    let record = require_record(&state, &id).await?;
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let target = match crate::api::scope::resolve(&state, &id, database).await {
        Ok(target) => target,
        Err(err) => {
            return Ok(Json(json!({
                "success": false,
                "message": err.message,
                "executeTime": 0,
                "results": [],
            })))
        }
    };
    if let Err(err) = crate::api::driver::ensure_installed(&state, record.kind()).await {
        return Ok(Json(json!({
            "success": false,
            "message": err.message,
            "executeTime": 0,
            "results": [],
        })));
    }
    let statements = dbmind_core::split_statements(record.kind().protocol(), &sql);
    if statements.is_empty() {
        return Err(XError::bad_request("SQL 不能为空"));
    }

    let mut results: Vec<Value> = Vec::new();
    let mut success = true;
    let mut failed_at: Option<usize> = None;
    let started = std::time::Instant::now();

    for (index, statement) in statements.iter().enumerate() {
        let engine = state.engine.clone();
        let target = target.clone();
        // 只有一个 executionId：交给第一段，这样「取消」至少能中断当前那条
        let exec = if index == 0 { execution_id.clone() } else { None };
        let statement = statement.clone();
        let outcome = blocking(move || {
            engine.execute(
                QueryRequest {
                    read_only: None,
                    connection: target,
                    sql: statement,
                    options: QueryOptions {
                        max_rows: 2000,
                        timeout_ms: 120_000,
                    },
                    execution_id: exec,
                    session: Some("ui:上游".to_string()),
                },
                AccessContext::Web,
            )
        })
        .await;
        match outcome {
            Ok(result) => results.push(shape::query_result_json(&result)),
            Err(err) => {
                success = false;
                failed_at = Some(index);
                results.push(shape::query_failure_json(&err.message, 0));
                break;
            }
        }
    }

    let message = match failed_at {
        Some(index) => format!("第 {} 条语句执行失败", index + 1),
        None => format!("共执行 {} 条语句", results.len()),
    };
    Ok(Json(json!({
        "success": success,
        "message": message,
        "executeTime": started.elapsed().as_millis() as i64,
        "results": results,
    })))
}

/// `POST /api/{m}/query/cancel/{executionId}` —— 取消在途语句（硬中断，不是「标记一下」）。
pub async fn cancel(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> XResult<Json<Value>> {
    let cancelled = state.engine.cancel(&execution_id);
    Ok(Json(json!({
        "success": true,
        "cancelled": cancelled,
        // 已经跑完（或压根没有）的 id 会返回 false —— 界面据此区分「取消了」与「已经结束了」
        "message": if cancelled { "已发送取消" } else { "该执行已结束或不存在" },
    })))
}

/// `POST /api/{m}/query/run-file-task/{id}` —— 执行 SQL 文件（逐句、有进度、可取消）。
pub async fn run_file_start(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let sql = body
        .get("sql")
        .or_else(|| body.get("content"))
        .or_else(|| body.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if sql.trim().is_empty() {
        return Err(XError::bad_request("没有收到 SQL 内容"));
    }
    let stop_on_error = body
        .get("stopOnError")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let record = require_record(&state, &id).await?;
    crate::api::driver::ensure_installed(&state, record.kind()).await?;
    // 文件里的语句也按用户选中的库执行
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let target = crate::api::scope::resolve(&state, &id, &database).await?;
    let statements = dbmind_core::split_statements(record.kind().protocol(), &sql);
    let task_id = format!(
        "sqlfile_{}_{}",
        chrono_like_timestamp(),
        statements.len()
    );

    {
        let mut guard = tasks().lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(
            task_id.clone(),
            Task {
                status: "running".to_string(),
                done: 0,
                total: statements.len(),
                phase: "执行".to_string(),
                message: format!("共 {} 条语句", statements.len()),
                canceled: false,
                logs: Vec::new(),
                current_execution: None,
            },
        );
    }

    let engine = state.engine.clone();
    let background_id = task_id.clone();
    std::thread::Builder::new()
        .name("上游-run-sql-file".to_string())
        .spawn(move || {
            // 注意传的是 target 而不是 id：切库之后要在目标库上跑
            run_statements(engine, background_id, target, statements, stop_on_error);
        })
        .map_err(|e| XError::internal(format!("启动执行线程失败：{e}")))?;

    Ok(Json(json!({ "success": true, "taskId": task_id })))
}

fn chrono_like_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn run_statements(
    engine: std::sync::Arc<DbMindEngine>,
    task_id: String,
    connection: String,
    statements: Vec<String>,
    stop_on_error: bool,
) {
    let total = statements.len();
    for (index, statement) in statements.iter().enumerate() {
        // 每一次加锁都在自己的作用域里**取完就放**。
        //
        // 这里踩过一次：早先把 guard 留在循环体里，紧接着又 `tasks().lock()` ——
        // `std::sync::Mutex` 不可重入，于是执行线程自己把自己锁死，状态接口永不返回，
        // 客户端只能等到超时。界面上的表现是「执行 SQL 文件后一直转圈」，看不出根因。
        let canceled = {
            let guard = tasks().lock().unwrap_or_else(|e| e.into_inner());
            guard.get(&task_id).map(|task| task.canceled).unwrap_or(true)
        };
        if canceled {
            finish_task(&task_id, "canceled", "已取消");
            return;
        }

        let execution_id = DbMindEngine::next_execution_id();
        {
            let mut guard = tasks().lock().unwrap_or_else(|e| e.into_inner());
            if let Some(task) = guard.get_mut(&task_id) {
                task.current_execution = Some(execution_id.clone());
            }
        }

        // 执行期间**不持锁**：这条语句可能跑几分钟，持锁会让状态查询全部排队
        let outcome = engine.execute(
            QueryRequest {
                read_only: None,
                connection: connection.clone(),
                sql: statement.clone(),
                options: QueryOptions {
                    max_rows: 1000,
                    timeout_ms: 300_000,
                },
                execution_id: Some(execution_id),
                session: Some("ui:上游".to_string()),
            },
            AccessContext::Web,
        );

        let preview: String = statement.chars().take(80).collect();
        let ellipsis = if statement.chars().count() > 80 { "…" } else { "" };
        let failure = {
            let mut guard = tasks().lock().unwrap_or_else(|e| e.into_inner());
            match guard.get_mut(&task_id) {
                None => return,
                Some(task) => match &outcome {
                    Ok(_) => {
                        task.logs
                            .push(format!("[{}/{total}] OK  {preview}{ellipsis}", index + 1));
                        None
                    }
                    Err(err) => {
                        task.logs
                            .push(format!("[{}/{total}] 失败：{}", index + 1, err.message));
                        task.done = index;
                        Some(err.message.clone())
                    }
                },
            }
        };

        if let Some(message) = failure {
            if stop_on_error {
                finish_task(&task_id, "error", &message);
                return;
            }
        } else {
            let mut guard = tasks().lock().unwrap_or_else(|e| e.into_inner());
            if let Some(task) = guard.get_mut(&task_id) {
                task.done = index + 1;
                task.message = format!("已执行 {}/{}", index + 1, total);
            }
        }
    }
    finish_task(&task_id, "success", &format!("全部完成：{total} 条"));
}

fn finish_task(task_id: &str, status: &str, message: &str) {
    let mut guard = tasks().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(task) = guard.get_mut(task_id) {
        task.status = status.to_string();
        task.phase = "完成".to_string();
        task.message = message.to_string();
        task.current_execution = None;
        task.logs.push(message.to_string());
    }
}

pub async fn run_file_status(Path(task_id): Path<String>) -> XResult<Json<Value>> {
    Ok(Json(task_snapshot(&task_id)?))
}

pub async fn run_file_cancel(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    let current = {
        let mut guard = tasks().lock().unwrap_or_else(|e| e.into_inner());
        match guard.get_mut(&task_id) {
            Some(task) => {
                task.canceled = true;
                task.message = "正在取消…".to_string();
                task.current_execution.clone()
            }
            None => return Err(XError::new(axum::http::StatusCode::NOT_FOUND, "任务不存在或已过期")),
        }
    };
    // 在途语句要硬中断，否则「取消」要等它自己跑完
    if let Some(execution_id) = current {
        state.engine.cancel(&execution_id);
    }
    Ok(Json(json!({ "success": true, "message": "已取消" })))
}

/// 保留：`Params` 在文件内暂未用到，但同模块 handler 会用到。
#[allow(dead_code)]
fn _keep(raw: Option<&str>) -> Params {
    Params::parse(raw)
}
