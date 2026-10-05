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
use dbmind_core::{AccessContext, DbMindEngine, QueryOptions, QueryRequest, TxAction};
use serde_json::{json, Value};

use crate::api::error::{XError, XResult};
use crate::api::{blocking, require_record, shape};
use crate::AppState;

/// 编辑器执行的超时（毫秒）：读设置 `query.timeoutSecs`（1..=600，缺省 120）。
///
/// 为什么从写死 120 秒改成可配：跑大报表/复杂分析的人嫌它不够，误发笛卡尔积的人
/// 又要干等满两分钟 —— 两类用户要的不是同一个数字，那就该放到设置里去。
/// 键名用内核导出的 [`dbmind_core::Store`] 常量，杜绝两边各写一份字符串漂移。
async fn editor_timeout_ms(state: &AppState) -> u64 {
    let engine = state.engine();
    let raw = blocking(move || engine.get_setting(dbmind_core::Store::KEY_QUERY_TIMEOUT))
        .await
        .ok()
        .flatten();
    let secs = raw
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|secs| secs.clamp(1, 600))
        .unwrap_or(120);
    secs * 1000
}

/// 全局默认返回行数上限（100..=100000，缺省 2000）。
///
/// 以前各执行入口各写死一个数字（编辑器 2000、批量 2000、NoSQL 浏览另有自己的），
/// 用户在设置里找不到「一次最多拿回多少行」。统一成一个键后：界面请求的 `size`
/// 还会被它再夹一层 —— 分页大小是「想要多少」，这里是「最多给多少」。
pub(crate) async fn max_rows_cap(state: &AppState) -> usize {
    let engine = state.engine();
    let raw = blocking(move || engine.get_setting(dbmind_core::Store::KEY_QUERY_MAX_ROWS))
        .await
        .ok()
        .flatten();
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .map(|n| n.clamp(100, 100_000))
        .unwrap_or(2000)
}

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

    // 第 2 页起才追加分页子句（见 `paging_sql`：直接追加，不过派生表），且只对
    // 「取数类」语句套。统计也要用「去掉末尾分号」的版本：带分号的原句进任何
    // 子查询/CTE 都是语法错误 —— 用户输入的 SQL 十有八九带分号（前端也允许），
    // 所以这一步不能省。执行本身不受影响，因此只在改写处用去尾分号的版本。
    let base_sql = sql.clone();
    let bare = base_sql.trim().trim_end_matches(';').trim().to_string();
    let effective = if page > 1 && looks_like_query(&bare) {
        let record = require_record(&state, &id).await?;
        let dialect = crate::api::dialect::Dialect::new(record.kind());
        // page/size 来自请求体没有上限，saturating 防乘法溢出（与 export.rs 同一纪律）
        crate::api::export::paging_sql(
            &bare,
            page.saturating_sub(1).saturating_mul(size as u64),
            size as u64,
            dialect,
        )
    } else {
        base_sql.clone()
    };

    let engine = state.engine();
    let timeout_ms = editor_timeout_ms(&state).await;
    // 全局默认上限再夹一层：分页大小是「想要多少」，设置里的 maxRows 是「最多给多少」
    let max_rows = size.min(max_rows_cap(&state).await);
    let request = QueryRequest {
        read_only: None,
        connection: target,
        sql: effective,
        options: QueryOptions {
            max_rows,
            timeout_ms,
        },
        execution_id,
        session: Some("ui:上游".to_string()),
        // **来源决定进不进历史**：SQL 编辑器（默认）进历史 —— 首页「最近查询」就是给它的；
        // 界面功能驱动的执行（建库/删目录/表属性探测等）带 internal=true，不混进用户的账本
        internal: body.get("internal").and_then(Value::as_bool).unwrap_or(false),
    };
    Ok(Json(match blocking(move || engine.execute(request, AccessContext::Web)).await {
        Ok(result) => {
            let mut json = shape::query_result_json(&result);
            // 总数统计**已移出主链路**：以前这里被截断/翻页时会同步跑 `count_rows`
            //（三层兜底串行，每层最多 120s）—— 大 JOIN 的 COUNT 能把整个查询响应
            // 卡到百秒级（真机：数据 1s 就绪，却等 COUNT 等了 163s+），其他工具
            // 只发 LIMIT 当然快。现在数据先回，前端拿到 totalCount=-1 后再异步调
            // `/query/{id}/count` 补总数（那边有 10s 总预算，算不出就放弃）。
            //
            // 这里只剩一处口径修正：第 2 页起 shape 会把「本页行数」当 totalCount
            //（!truncated 时 rowCount 即 totalCount），末页不足一页时是错的 —— 置 -1
            // 表示未知，等异步计数回填。
            if page > 1 {
                if let Some(object) = json.as_object_mut() {
                    object.insert("totalCount".to_string(), Value::from(-1));
                }
            }
            json
        }
        Err(err) => shape::query_failure_json(&err.message, 0),
    }))
}

/// `POST /api/{m}/query/{id}/tx` —— 会话级事务控制（事务模式）。
///
/// body: `{ action: "begin" | "commit" | "rollback" }`。会话键与 SQL 编辑器完全一致
/// （`ui:上游`）⇒ begin 之后的编辑器语句全部落在同一条 autocommit=false 的物理连接上，
/// 直到 commit/rollback。全 agent 数据源通用（宿主走 JDBC 标准接口，无方言语法）。
pub async fn tx(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let action = match body.get("action").and_then(Value::as_str) {
        Some("begin") => TxAction::Begin,
        Some("commit") => TxAction::Commit,
        Some("rollback") => TxAction::Rollback,
        _ => return Err(XError::bad_request("action 必须是 begin / commit / rollback")),
    };
    // **必须与 execute 解析出同一个目标**（同样的 database → 同一条连接记录）：
    // 泳道键由「连接记录 + 只读性 + 亲和键」组成，目标不同 = 泳道不同 = 事务落在
    // 另一条物理连接上（真机实测：begin 在 23232，查询在 23233，autocommit 白设）。
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let target = match crate::api::scope::resolve(&state, &id, &database).await {
        Ok(target) => target,
        Err(err) => return Ok(Json(json!({ "success": false, "message": err.message }))),
    };
    if let Err(err) = crate::api::driver::ensure_for_connection(&state, &target).await {
        return Ok(Json(json!({ "success": false, "message": err.message })));
    }
    // 与 execute 的 `session: Some("ui:上游")` 保持同一把钥匙：事务绑定编辑器会话
    let engine = state.engine();
    let outcome = engine.tx_control(&target, "ui:上游", action);
    Ok(Json(match outcome {
        Ok(mut value) => {
            if let Some(object) = value.as_object_mut() {
                object.insert("success".to_string(), Value::from(true));
                object.insert("action".to_string(), Value::from(action.as_sql()));
            }
            value
        }
        Err(err) => json!({ "success": false, "message": err.message }),
    }))
}

/// `POST /api/{m}/query/{id}/count` —— 只算总数，不取数。
///
/// 给前端的**异步计数**用：主执行接口不再同步 COUNT（见 `execute` 里的注释），
/// 数据回显后前端拿原句来这里补总数。带 **10s 总预算**：`count_rows` 三层兜底
/// 是串行的（派生表 COUNT → CTE COUNT → 二分探测），放任不管的话一条大 JOIN
/// 能把三层都拖满 —— 反正 10 秒算不出的总数，用户多半也不愿意等。
pub async fn count(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let sql = body
        .get("sql")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // 列数由前端从刚拿到的结果里带来，省掉 count_rows 内部「探一次列数」的往返；
    // 没带就传 0（内部会自己探，代价是原句 max_rows=1 跑一遍，很小）。
    let ncols = body.get("ncols").and_then(Value::as_u64).unwrap_or(0) as usize;
    if sql.trim().is_empty() {
        return Ok(Json(json!({ "success": false, "totalCount": -1 })));
    }
    // 与主执行同一纪律：统计也要「去掉末尾分号」的原句（带分号进派生表是语法错误）
    let bare = sql.trim().trim_end_matches(';').trim().to_string();
    let total = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        crate::api::export::count_rows(&state, &id, &database, &bare, ncols),
    )
    .await
    .ok() // 超时 → None
    .flatten() // 内部失败 → None
    .unwrap_or(-1);
    Ok(Json(json!({ "success": true, "totalCount": total })))
}

/// 看起来是不是「取数」语句 —— 只有这类才适合套分页壳。
///
/// 不解析 SQL，只看首个词：套壳是为了分页，DDL/DML 套进去只会报错，
/// 而它们在「第几页」这个问题上没有意义。
fn looks_like_query(sql: &str) -> bool {
    // split_whitespace 本身跳过前导空白，不必先 trim_start
    let head = sql
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
    // 多段执行共用同一个超时/上限：循环里拿不到 async 上下文，先在这里算好
    let timeout_ms = editor_timeout_ms(&state).await;
    let max_rows = max_rows_cap(&state).await;

    for (index, statement) in statements.iter().enumerate() {
        let engine = state.engine();
        let target = target.clone();
        // 只有一个 executionId：交给第一段，这样「取消」至少能中断当前那条
        let exec = if index == 0 { execution_id.clone() } else { None };
        let statement = statement.clone();
        let bare = statement.trim().trim_end_matches(';').trim().to_string();
        let outcome = blocking(move || {
            engine.execute(
                QueryRequest {
                    read_only: None,
                    connection: target,
                    sql: statement,
                    options: QueryOptions {
                        max_rows,
                        timeout_ms,
                    },
                    execution_id: exec,
                    session: Some("ui:上游".to_string()),
                    // 编辑器里的多段执行：要进历史
                    internal: false,
                },
                AccessContext::Web,
            )
        })
        .await;
        match outcome {
            Ok(result) => {
                let mut json = shape::query_result_json(&result);
                // 与单段执行同一口径：**不再同步 COUNT**（见 `execute` 里的注释 ——
                // 大 JOIN 的计数能拖百秒级，批量逐段同步数会把每段都卡一遍）。
                // 截断时 shape 给 -1，前端对当前展示的 tab 异步调 `/count` 补总数。
                // 段落原文一并返回：翻到某段的第 N 页时，前端**只重跑这一段**
                // （整批重跑会把写入类语句再执行一遍，绝不能干）
                if let Some(o) = json.as_object_mut() {
                    o.insert("sql".to_string(), Value::from(bare.as_str()));
                }
                results.push(json);
            }
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
    let cancelled = state.engine().cancel(&execution_id);
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

    let engine = state.engine();
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
                // 「执行 SQL 文件」也是用户从编辑器发起的（与多段执行同一口径）：要进历史
                internal: false,
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
        state.engine().cancel(&execution_id);
    }
    Ok(Json(json!({ "success": true, "message": "已取消" })))
}

