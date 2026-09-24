//! `/api/{m}/import/*` —— 文件导入（CSV / JSON → 表）。
//!
//! ## 流程与三个约定
//!
//! `POST {m}/import/{id}`（multipart，字段名 `file`）→ `{taskId}` → 轮询进度 → 可取消。
//! 请求线程只做「解析 + 校验」这件事，**写入放到任务里**：几十万行的插入要几十秒，
//! 让它占着一个 HTTP 连接（还得担心网关超时）不如立刻返回 taskId。
//!
//! 1. **解析失败在 HTTP 阶段就报**。文件不是 CSV、表头为空、没带 file 字段 ——
//!    这些都是「调用方立刻能改」的问题，应该以 400 + 一句人话出现，
//!    而不是变成一个「任务失败」让用户去猜是哪一步错了。
//! 2. **空单元格 = NULL**（不是空串）。CSV 里 `a,,c` 的中间那格语义上就是「没有值」，
//!    写成 `''` 会污染判空逻辑（`where col is null` 再也查不到）。
//! 3. **多行 VALUES 只在支持的方言上用**。Oracle/DB2/Derby 不支持
//!    `values (...),(...)`，对它们退化成一行一条 —— 慢，但正确；
//!    反过来「统一都用多行」会在那些库上直接报语法错误。
//!
//! 支持 **CSV / JSON / Excel(.xlsx)** 三种。
//!
//! `.xlsx` 的读取在 `xlsx` 模块（zip + sharedStrings + 单元格类型 + 日期判定），
//! 这里只做与 CSV 一致的收尾 —— 两条路径共用 `assemble`，免得同一个文件格式
//! 在「空表头怎么补」「多一列怎么办」上长出两种行为。
//!
//! `.xls`（2003 的二进制 BIFF 格式）**不在支持范围**：它与 `.xlsx` 是两套东西，
//! 解析它要另写一套记录级解析。收到时会明确说清并给出「另存为 .xlsx 或 CSV」的指引。

use axum::extract::{Multipart, Path, RawQuery, State};
use axum::Json;
use dbmind_core::{AccessContext, QueryOptions, QueryRequest};
use serde_json::{json, Value};

use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::{blocking, require_record, Params};
use crate::AppState;

/// 上传上限。
///
/// axum 默认只收 2 MB，而一个正常的数据文件动辄几十 MB —— 不放大上限，
/// 用户会看到「文件太大」这种莫名其妙的失败。但也不能不设上限：
/// 那样任何请求都能把进程内存吃光。
pub const IMPORT_LIMIT: usize = 256 * 1024 * 1024;

/// 每批写入多少行（一条语句塞多少行）。
const BATCH_ROWS: usize = 200;

/// `POST /api/{m}/import/{id}` —— 提交导入。
pub async fn start(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
    mut multipart: Multipart,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let mut database = params.get("database").unwrap_or_default();
    let mut table = params.get("table").unwrap_or_default();
    let mut mode = params.get("mode").unwrap_or_else(|| "append".to_string());
    let mut filename = String::new();
    let mut bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| XError::bad_request(format!("解析上传表单失败：{e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        let file_name = field.file_name().map(str::to_string);
        // 无论要不要这个字段，都必须把它读完 —— 否则后续字段读不到（流式解析）
        let data = field
            .bytes()
            .await
            .map_err(|e| XError::bad_request(format!("读取上传内容失败：{e}")))?;
        match name.as_str() {
            "file" => {
                filename = file_name.unwrap_or_default();
                bytes = Some(data.to_vec());
            }
            "database" => database = String::from_utf8_lossy(&data).trim().to_string(),
            "table" => table = String::from_utf8_lossy(&data).trim().to_string(),
            "mode" => {
                let value = String::from_utf8_lossy(&data).trim().to_string();
                if !value.is_empty() {
                    mode = value;
                }
            }
            _ => {}
        }
    }

    let bytes = bytes.ok_or_else(|| {
        XError::bad_request("没有收到上传文件（multipart 的文件字段名应为 file）")
    })?;
    if bytes.is_empty() {
        return Err(XError::bad_request("上传的文件是空的"));
    }
    if !matches!(mode.as_str(), "append" | "overwrite" | "create") {
        return Err(XError::bad_request(format!(
            "不支持的导入模式 {mode}（可选 append / overwrite / create）"
        )));
    }
    if table.trim().is_empty() {
        table = table_from_filename(&filename)
            .ok_or_else(|| XError::bad_request("无法从文件名推断目标表名，请显式传 table 参数"))?;
    }

    let parsed = parse_file(&filename, &bytes)?;
    let record = require_record(&state, &id).await?;
    let label = record.config.name.clone();
    let columns = parsed.columns.clone();
    let rows = parsed.rows;
    let total = rows.len() as i64;
    if total == 0 {
        return Err(XError::bad_request("文件里没有数据行（只有表头）"));
    }

    let registry = state.tasks.clone();
    let task = registry.spawn("import", "imp_", move |task| {
        let state = state.clone();
        let id = id.clone();
        async move {
            task.set_total(total);
            task.step(format!("开始导入 {total} 行到 {table}"));

            let record = require_record(&state, &id).await.map_err(|e| e.message)?;
            let dialect = Dialect::new(record.kind());
            let target_name = dialect.quote(&table);
            let column_list = columns
                .iter()
                .map(|name| dialect.quote(name))
                .collect::<Vec<_>>()
                .join(", ");

            if mode == "create" {
                let columns: Vec<String> = columns
                    .iter()
                    .map(|name| format!("{} text", dialect.quote(name)))
                    .collect();
                let ddl = format!(
                    "create table if not exists {target_name} ({})",
                    columns.join(", ")
                );
                task.step(format!("建表（若不存在）：{table}"));
                run_sql(&state, &id, &database, &ddl)
                    .await
                    .map_err(|err| err.message)?;
            }
            if mode == "overwrite" {
                task.step(format!("清空目标表 {table}"));
                run_sql(&state, &id, &database, &format!("delete from {target_name}"))
                    .await
                    .map_err(|err| err.message)?;
            }

            let multi_row = supports_multi_row(dialect);
            let batch = if multi_row { BATCH_ROWS } else { 1 };
            let mut written = 0u64;
            let mut index = 0usize;
            while index < rows.len() {
                // 任务工作体的错误类型是 String（它不该认识 HTTP），所以这里不套 XError
                task.check_canceled()?;
                let end = (index + batch).min(rows.len());
                let chunk = &rows[index..end];
                let sql = build_insert(&target_name, &column_list, chunk, dialect, multi_row);
                run_sql(&state, &id, &database, &sql).await.map_err(|err| {
                    // 报错要能定位到「哪一批」：几十万行的导入里，
                    // 只说「插入失败」等于什么也没说
                    let hint = match explain_failure(&err.message) {
                        Some(text) => format!("\n原因：{text}"),
                        None => String::new(),
                    };
                    format!(
                        "写入第 {}-{} 行失败：{}{}\n出错语句（截断）：{}",
                        index + 1,
                        end,
                        err.message,
                        hint,
                        truncate(&sql, 400)
                    )
                })?;
                written += chunk.len() as u64;
                index = end;
                task.set_done(written);
                task.set_phase(format!("已写入 {written}/{total} 行"));
                if written % (BATCH_ROWS as u64 * 10) == 0 {
                    task.log(format!("已写入 {written} 行"));
                }
            }

            let message = format!("导入完成：{written} 行 → {table}（连接 {label}）");
            task.set_message(message.clone());
            Ok(Some(json!({
                "success": true,
                "count": written,
                "table": table,
                "message": message,
                "cancelled": false,
            })))
        }
    });

    Ok(Json(json!({ "success": true, "taskId": task.id })))
}

/// `GET /api/{m}/import/task/{taskId}` —— 进度。
pub async fn status(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    let Some(task) = state.tasks.get(&task_id) else {
        return Ok(Json(json!({
            "success": false,
            "status": "notfound",
            "message": "任务不存在或已过期（任务只保留 30 分钟）",
        })));
    };
    let mut view = task.snapshot();
    view["success"] = json!(true);
    // 导入没有产物文件，但结果的形状必须与上游一致（前端读 result.count 显示「写入多少行」）
    view["result"] = task.result().unwrap_or_else(|| {
        let canceled = task.is_canceled();
        json!({
            "success": false,
            "count": task.done(),
            "message": if canceled { "已取消" } else { "进行中" },
            "cancelled": canceled,
        })
    });
    Ok(Json(view))
}

/// `POST /api/{m}/import/cancel/{taskId}` —— 请求取消。
pub async fn cancel(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    match state.tasks.get(&task_id) {
        Some(task) => {
            task.cancel();
            Ok(Json(json!({ "success": true, "message": "已请求取消，当前批次写完后停止" })))
        }
        None => Ok(Json(json!({ "success": false, "message": "任务不存在或已过期" }))),
    }
}

// ------------------------------------------------------------------ 执行

async fn run_sql(state: &AppState, conn: &str, database: &str, sql: &str) -> Result<(), XError> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let engine = state.engine.clone();
    let request = QueryRequest {
        read_only: None,
        connection: target,
        sql: sql.to_string(),
        options: QueryOptions {
            max_rows: 1,
            timeout_ms: 300_000,
        },
        execution_id: None,
        session: Some("internal:browse".to_string()),
    };
    blocking(move || engine.execute(request, AccessContext::Web)).await?;
    Ok(())
}

/// 这个方言支持 `values (...),(...)` 多行插入吗？
fn supports_multi_row(dialect: Dialect) -> bool {
    matches!(
        dialect.kind.key(),
        "mysql" | "mariadb" | "doris" | "postgresql" | "kingbase" | "sqlite" | "sqlserver"
            | "h2" | "clickhouse"
    )
}

/// 拼插入语句。
///
/// `multi_row=false` 时调用方保证只传一行：内核一次只执行一条语句，
/// 拼多条进去只会执行第一条 —— 那是**静默丢数据**，比报错坏得多。
fn build_insert(
    table: &str,
    column_list: &str,
    rows: &[Vec<Option<String>>],
    dialect: Dialect,
    multi_row: bool,
) -> String {
    let tuple = |cells: &Vec<Option<String>>| {
        let values: Vec<String> = cells
            .iter()
            .map(|cell| match cell {
                None => "null".to_string(),
                Some(text) => dialect.literal(text),
            })
            .collect();
        format!("({})", values.join(", "))
    };
    let head = format!("insert into {table} ({column_list}) values ");
    if multi_row {
        let tuples: Vec<String> = rows.iter().map(tuple).collect();
        format!("{head}{}", tuples.join(", "))
    } else {
        let first = rows
            .first()
            .map(tuple)
            .unwrap_or_else(|| "(null)".to_string());
        format!("{head}{first}")
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit).collect();
    format!("{head}…")
}

/// 给「只有 DBA 读得懂」的数据库报错补一句人话。
///
/// 用户看到的现象只是「导入失败」，很容易以为导入功能坏了；而有些错误其实
/// 与导入的数据完全无关（比如目标表上的触发器把这张表自己锁住了）。这里只补
/// **确实见过、且说清原因就能动手解决**的那几类，不认识的错误原样透传。
fn explain_failure(db_message: &str) -> Option<&'static str> {
    // MySQL 1442：触发器改了「触发它的那张表」。典型写法是 BEFORE INSERT 里
    // 又 `insert into 同一张表 select ...`（建触发器时不报错），此后**任何**
    // INSERT 都会被数据库拒掉 —— 与导入的文件、列、行数都无关。
    if db_message.contains("errorCode:1442")
        || db_message.contains("already used by statement which invoked")
    {
        return Some(
            "目标表上有触发器会写回它自己（例如 BEFORE INSERT 里又 insert 同一张表），\
             MySQL 不允许这种自引用（错误 1442）—— 也就是说任何 INSERT 都会被拒，\
             与导入的文件内容无关。处理办法二选一：\
             ① 在左侧树「Triggers」里删掉该触发器；\
             ② 在 SQL 编辑器里执行 drop trigger 库名.触发器名。删掉后再导入即可。",
        );
    }
    None
}

// ------------------------------------------------------------------ 解析

/// 解析结果：表头 + 数据行（`None` = NULL）。
struct Parsed {
    columns: Vec<String>,
    rows: Vec<Vec<Option<String>>>,
}

fn parse_file(filename: &str, bytes: &[u8]) -> XResult<Parsed> {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".xlsx") {
        return parse_xlsx(bytes);
    }
    if lower.ends_with(".xls") {
        // `.xls` 是 2003 的二进制格式（BIFF），与 `.xlsx`（zip + XML）是两套完全不同的东西：
        // 解析它得另写一套记录级解析。如实说清，并给出可执行的替代做法。
        return Err(XError::bad_request(
            "`.xls`（Excel 2003 二进制格式）尚未支持：请在 Excel 里「另存为 .xlsx 或 CSV」再导入",
        ));
    }
    if lower.ends_with(".json") || looks_like_json(bytes) {
        return parse_json(bytes);
    }
    parse_csv(bytes)
}

/// `.xlsx` 解析：zip / XML 的细节都在 `xlsx` 模块里，这里只做与 CSV 一致的收尾。
fn parse_xlsx(bytes: &[u8]) -> XResult<Parsed> {
    assemble(crate::api::xlsx::parse(bytes)?, "xlsx")
}

fn looks_like_json(bytes: &[u8]) -> bool {
    matches!(first_meaningful_byte(bytes), Some(b'{') | Some(b'['))
}

fn first_meaningful_byte(bytes: &[u8]) -> Option<u8> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    bytes
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
}

/// 从文件名推断表名（`orders_2026.csv` → `orders_2026`）。
fn table_from_filename(filename: &str) -> Option<String> {
    let stem = std::path::Path::new(filename)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())?;
    let cleaned = stem.trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

/// CSV 解析（RFC 4180：引号包裹、双引号转义、字段内可有换行）。
///
/// 手写而不是拉一个 crate：这里只需要「读成字符串矩阵」这一件事，
/// 引号状态机不过几十行，而多数 CSV crate 对「空字段 vs NULL」的语义各有主张。
fn parse_csv(bytes: &[u8]) -> XResult<Parsed> {
    let text = String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes));
    let mut records: Vec<Vec<Option<String>>> = Vec::new();
    let mut record: Vec<Option<String>> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut has_field = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                quoted = true;
                has_field = true;
            }
            ',' => {
                record.push(none_if_empty(std::mem::take(&mut field)));
                has_field = false;
            }
            '\r' => {}
            '\n' => {
                if has_field || !record.is_empty() || !field.is_empty() {
                    record.push(none_if_empty(std::mem::take(&mut field)));
                    records.push(std::mem::take(&mut record));
                }
                has_field = false;
            }
            other => {
                field.push(other);
                has_field = true;
            }
        }
    }
    if quoted {
        return Err(XError::bad_request("CSV 里有没闭合的引号"));
    }
    if has_field || !record.is_empty() || !field.is_empty() {
        record.push(none_if_empty(field));
        records.push(record);
    }

    assemble(records, "CSV")
}

/// 各格式共用的收尾：跳空行 → 首行当表头 → 空表头补 `column_N` → 行对齐。
///
/// 抽出来是因为 CSV 与 xlsx 的**行为必须一致**：两条路径各写一遍，
/// 迟早长出「CSV 给空表头补 column_N、xlsx 不补」这种只在一边出现的差异。
fn assemble(records: Vec<Vec<Option<String>>>, what: &str) -> XResult<Parsed> {
    let mut records = records.into_iter().filter(|row| row.iter().any(Option::is_some));
    let header = records
        .next()
        .ok_or_else(|| XError::bad_request(format!("{what} 是空的（连表头都没有）")))?;
    let columns: Vec<String> = header
        .into_iter()
        .enumerate()
        .map(|(index, cell)| cell.unwrap_or_else(|| format!("column_{}", index + 1)))
        .collect();
    if columns.iter().all(|name| name.trim().is_empty()) {
        return Err(XError::bad_request(format!("{what} 首行（表头）是空的")));
    }
    let rows = records
        .map(|mut row| {
            // 列数对齐：多出来的截掉，少的补 NULL（否则插入语句的列数与值数对不上）
            row.truncate(columns.len());
            while row.len() < columns.len() {
                row.push(None);
            }
            row
        })
        .collect();
    Ok(Parsed { columns, rows })
}

fn none_if_empty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// JSON 解析：`[{...}, {...}]`。
fn parse_json(bytes: &[u8]) -> XResult<Parsed> {
    let text = String::from_utf8_lossy(bytes);
    let value: Value =
        serde_json::from_str(&text).map_err(|e| XError::bad_request(format!("JSON 解析失败：{e}")))?;
    let items = match value {
        Value::Array(items) => items,
        Value::Object(map) => vec![Value::Object(map)],
        _ => return Err(XError::bad_request("JSON 顶层应该是对象数组")),
    };
    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    for item in items {
        let object = match item {
            Value::Object(object) => object,
            _ => return Err(XError::bad_request("JSON 数组的元素应该是对象")),
        };
        for key in object.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
        rows.push(
            columns
                .iter()
                .map(|key| object.get(key).and_then(json_cell))
                .collect(),
        );
    }
    if columns.is_empty() {
        return Err(XError::bad_request("JSON 里没有可导入的字段"));
    }
    Ok(Parsed { columns, rows })
}

fn json_cell(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => {
            if text.is_empty() {
                None
            } else {
                Some(text.clone())
            }
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        other => Some(other.to_string()),
    }
}
