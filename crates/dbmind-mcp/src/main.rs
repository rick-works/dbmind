//! DBMind MCP 壳（stdio，JSON-RPC 2.0）。
//!
//! 与其它壳一样只调用内核；区别在于**来源标记为 `Mcp`**，于是内核的安全策略
//! 默认只放行只读语句 —— 这一点不依赖客户端的自觉，也不受 `--allow-write` 之外的
//! 任何参数影响，而 `--allow-write` 又无法越过「生产保护」和「只读连接」。

use dbmind_core::{AccessContext, DbMindEngine, DbMindError, ErrorCode, QueryOptions, QueryRequest, Store};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Mutex;

const PROTOCOL_VERSION: &str = "2024-11-05";

struct Server {
    engine: DbMindEngine,
    /// 在飞的 `tools/call`：JSON-RPC 请求 id → 这次调用用的内核 executionId。
    ///
    /// 为什么需要这层映射：MCP 的取消通知带的是**请求 id**（`params.requestId`），
    /// 而内核取消用的是 executionId —— 不建映射，取消就无处落地（这正是此前
    /// `notifications/cancelled` 被直接丢弃的原因）。
    ///
    /// 顺带记住「已被要求取消」：通知可能比工作线程真正开始执行还早，
    /// 那时内核还没登记这次执行，只能由壳自己先记下。
    running: Mutex<HashMap<String, Running>>,
    /// stdout 串行化：读循环与工作线程都会写响应（JSON-RPC 一行一条，不能交错）。
    out: Mutex<std::io::Stdout>,
}

/// 一次在飞的 `tools/call`。
struct Running {
    execution_id: String,
    cancelled: bool,
}

/// 工具调用的并发上限。
///
/// 为什么要有这道闸：`tools/call` 一旦交给线程执行，一个乱发请求的客户端就能把线程数
/// 拉起来（每线程都有栈与请求副本）。这里用**固定工作池 + 队列**：并发恒为这个数，
/// 多出来的调用在壳里排队 —— 而且**排队中也能被取消**（见 `enqueue`）。
///
/// 取 4 的理由：与内核那边的默认对齐（单连接类型上限 1，其余类型默认 4，
/// 见 YAML 的 `maxConnections`）。壳再多也只是排队，不如先把并发对齐内核的常见上限。
///
/// 代价如实说：这是**有界池**，一个快工具也可能排在几条慢查询后面（队头阻塞）。
/// 换来的是「客户端乱发也打不爆壳」。
const TOOL_WORKERS: usize = 4;

/// 一个待执行的 `tools/call`。
struct Job {
    id: Value,
    request: Value,
}

impl Job {
    /// 是不是会落成一次「内核执行」的调用（只有这种才有可取消的东西）。
    fn is_query(&self) -> bool {
        self.request.pointer("/params/name").and_then(|v| v.as_str()) == Some("dbmind_execute_query")
    }
}

fn main() {
    let mut store: Option<PathBuf> = None;
    let mut allow_write = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--store" => store = args.next().map(PathBuf::from),
            "--allow-write" => allow_write = true,
            "--version" => {
                println!("dbmind-mcp {}", dbmind_core::VERSION);
                return;
            }
            "--help" | "-h" => {
                println!("dbmind-mcp [--store <path>] [--allow-write]");
                println!(
                    "  默认只允许只读语句；--allow-write 放开 AI 通道写入（仍受只读连接与生产保护约束）"
                );
                return;
            }
            other => {
                eprintln!("未知参数: {other}");
                std::process::exit(2);
            }
        }
    }

    let engine = match store {
        Some(path) => DbMindEngine::open(&path),
        None => DbMindEngine::open_default(),
    };
    let engine = match engine {
        Ok(engine) => engine,
        Err(err) => {
            eprintln!("无法打开元数据库: {err}");
            std::process::exit(1);
        }
    };

    if allow_write {
        if let Err(err) = engine.set_setting(Store::KEY_AI_WRITE_ENABLED, "true") {
            eprintln!("启用 AI 写入失败: {err}");
        }
    }

    let server = Server {
        engine,
        running: Mutex::new(HashMap::new()),
        out: Mutex::new(std::io::stdout()),
    };
    server.serve();
}

impl Server {
    fn serve(&self) {
        let stdin = std::io::stdin();
        let (sender, receiver) = std::sync::mpsc::channel::<Job>();
        let receiver = Mutex::new(receiver);
        // 用 scope 而不是 detached 线程：stdin 关闭后**等所有工作线程把响应写完**再退出，
        // 否则客户端会看到一个「永远没回话」的请求（冒烟脚本正是这样读响应的）。
        std::thread::scope(|scope| {
            // 固定工作池：并发有上限，主循环永远不阻塞（否则取消读不到）
            for _ in 0..TOOL_WORKERS {
                let receiver = &receiver;
                scope.spawn(move || loop {
                    // 只在 recv 期间持有这把锁；取到活就放开，真正干活是并发的
                    let job = {
                        let guard = receiver.lock().unwrap_or_else(|e| e.into_inner());
                        guard.recv()
                    };
                    match job {
                        Ok(job) => {
                            if let Some(response) = self.call_tool(job.id, &job.request) {
                                self.write(&response);
                            }
                        }
                        // 通道关闭（主循环结束）⇒ 收工。手上的活已经干完了
                        Err(_) => break,
                    }
                });
            }

            for line in stdin.lock().lines() {
                let line = match line {
                    Ok(line) => line,
                    Err(_) => break,
                };
                // 容忍行首的 BOM：Windows 上往子进程 stdin 写字符串很容易带出 `\uFEFF`，
                // 于是**第一个**请求莫名报「JSON 解析失败」，而那些客户端本身没做错什么。
                let line = line.trim_start_matches('\u{feff}');
                if line.trim().is_empty() {
                    continue;
                }
                let request: Value = match serde_json::from_str(line) {
                    Ok(value) => value,
                    Err(err) => {
                        self.write(&jsonrpc_error(
                            Value::Null,
                            -32700,
                            &format!("JSON 解析失败: {err}"),
                        ));
                        continue;
                    }
                };

                // `tools/call` 可能是长语句：**投给工作池，主循环继续读 stdin** ——
                // 否则紧随其后的 `notifications/cancelled` 永远读不到，取消就是空话。
                if let Some(id) = request.get("id") {
                    if request.get("method").and_then(|m| m.as_str()) == Some("tools/call") {
                        self.enqueue(&sender, id.clone(), request);
                        continue;
                    }
                }

                if let Some(response) = self.handle(&request) {
                    self.write(&response);
                }
            }

            // 主循环结束 ⇒ 关掉通道 ⇒ 工作线程把手上的活干完就退出（scope 负责 join）
            drop(sender);
        });
    }

    /// 投递一个 `tools/call` 到工作池。
    ///
    /// **查询类在投递时就登记 executionId**：排队中的调用也要能被取消 ——
    /// 否则「取消一个还没轮到的请求」会静默落空（它甚至还没到工作线程手里）。
    fn enqueue(&self, sender: &std::sync::mpsc::Sender<Job>, id: Value, request: Value) {
        let job = Job { id, request };
        if job.is_query() {
            let execution_id = DbMindEngine::next_execution_id();
            let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
            running.insert(
                id_key(&job.id),
                Running {
                    execution_id,
                    cancelled: false,
                },
            );
        }
        // 发送失败说明通道已关闭（壳正在退出），此时没有响应也无所谓
        let _ = sender.send(job);
    }

    /// 取某个请求已登记的 executionId（投递时登记，见 `enqueue`）。
    fn execution_id_of(&self, key: &str) -> Option<String> {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .map(|entry| entry.execution_id.clone())
    }

    /// 写一条响应（一行一条 JSON）—— 所有写都经过这里，保证不交错。
    fn write(&self, message: &Value) {
        let line = serde_json::to_string(message).unwrap_or_else(|_| "{}".to_string());
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    /// MCP 取消通知：`params.requestId` 是**请求 id**，先映射成 executionId 再取消。
    ///
    /// 查不到就静默返回：取消一个已经结束（或根本不是查询）的请求不该报错 ——
    /// 通知没有响应通道，报错也无处可去。
    fn cancel(&self, request: &Value) {
        let Some(request_id) = request.pointer("/params/requestId") else {
            return;
        };
        let key = id_key(request_id);
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = running.get_mut(&key) else {
            return;
        };
        entry.cancelled = true;
        let execution_id = entry.execution_id.clone();
        drop(running);

        let hit = self.engine.cancel(&execution_id);
        // 通知没有响应通道：把结果写到 stderr，排查时看得到
        eprintln!("取消请求：requestId={key} executionId={execution_id} 命中={hit}");
    }

    fn handle(&self, request: &Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or_default();

        // 通知（无 id）不需要响应
        if id.is_none() {
            if method == "notifications/cancelled" {
                // 这一句就是此前缺失的那一环：取消通知要**真的**落到内核上
                self.cancel(request);
            }
            return None;
        }
        let id = id.unwrap();

        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "dbmind", "version": dbmind_core::VERSION },
                "instructions": "DBMind 数据库工作台。默认只读；需要写入时请让用户在 DBMind 中开启「允许 AI 写入」。"
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": self.tools() })),
            "tools/call" => return self.call_tool(id, request),
            "resources/list" => Ok(json!({ "resources": [] })),
            "prompts/list" => Ok(json!({ "prompts": [] })),
            "shutdown" => Ok(json!({})),
            other => Err((-32601, format!("不支持的方法: {other}"))),
        };

        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => jsonrpc_error(id, code, &message),
        })
    }

    fn call_tool(&self, id: Value, request: &Value) -> Option<Value> {
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let args = params.get("arguments").cloned().unwrap_or(json!({}));

        let key = id_key(&id);
        match self.dispatch(&key, &name, &args) {
            Ok(payload) => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{ "type": "text", "text": serde_json::to_string_pretty(&payload).unwrap_or_default() }],
                    "isError": false
                }
            })),
            Err(err) => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    // 业务错误回在 content 里，模型能读到码与原因并自行纠正
                    "content": [{ "type": "text", "text": err.to_string() }],
                    "isError": true
                }
            })),
        }
    }

    fn dispatch(&self, key: &str, name: &str, args: &Value) -> dbmind_core::Result<Value> {
        match name {
            "dbmind_list_connections" => Ok(serde_json::to_value(self.engine.list_connections()?)?),
            "dbmind_list_types" => Ok(serde_json::to_value(self.engine.types())?),
            "dbmind_list_tables" => {
                let connection = required_str(args, "connection")?;
                // 结构**一律现读**：AI 拿到 5 分钟前的结构会写出不存在的表名，
                // 而 AI 调元数据的频率本来就很低 —— 慢一点换准，值。
                Ok(serde_json::to_value(self.engine.list_tables_fresh(&connection)?)?)
            }
            "dbmind_describe_table" => {
                let connection = required_str(args, "connection")?;
                let table = required_str(args, "table")?;
                let columns = self.engine.list_columns_fresh(&connection, &table)?;
                Ok(json!({ "table": table, "columns": columns }))
            }
            "dbmind_execute_query" => {
                let connection = required_str(args, "connection")?;
                let sql = required_str(args, "sql")?;
                let mut options = QueryOptions::default();
                if let Some(max_rows) = args.get("max_rows").and_then(|v| v.as_u64()) {
                    options.max_rows = max_rows as usize;
                }
                if let Some(timeout_ms) = args.get("timeout_ms").and_then(|v| v.as_u64()) {
                    options.timeout_ms = timeout_ms;
                }
                // executionId 在**投递时**就已登记（这样排队中也才能被取消，见 `enqueue`）；
                // 万一没有（理论上到不了这里），现生成一个，别让这次调用白失败。
                let execution_id = self
                    .execution_id_of(key)
                    .unwrap_or_else(DbMindEngine::next_execution_id);

                // 再看一眼有没有人已经要求取消：**排队期间被取消的调用不该再执行** ——
                // 那一刻内核还没登记这次执行，`engine.cancel` 会落空，只能在壳里拦下。
                let already_cancelled = self
                    .running
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(key)
                    .map(|entry| entry.cancelled)
                    .unwrap_or(false);

                let outcome = if already_cancelled {
                    Err(DbMindError::new(ErrorCode::QueryCanceled, "查询已被取消"))
                } else {
                    self.engine.execute(
                        QueryRequest::new(connection, sql)
                            .with_options(options)
                            .with_execution_id(execution_id.clone()),
                        AccessContext::Mcp,
                    )
                };
                self.running.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
                let result = outcome?;
                Ok(json!({
                    "executionId": result.execution_id,
                    "statementKind": result.statement_kind.as_str(),
                    "columns": result.columns,
                    "rowCount": result.row_count,
                    "truncated": result.truncated,
                    "durationMs": result.duration_ms,
                    "notices": result.notices,
                    "rows": render_rows(&result),
                }))
            }
            "dbmind_list_history" => {
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                let connection = args.get("connection").and_then(|v| v.as_str());
                Ok(serde_json::to_value(self.engine.history(limit, connection)?)?)
            }
            other => Err(DbMindError::new(
                ErrorCode::Internal,
                format!("未知工具: {other}"),
            )),
        }
    }

    fn tools(&self) -> Vec<Value> {
        vec![
            json!({
                "name": "dbmind_list_connections",
                "description": "列出 DBMind 中已配置的数据连接（不含口令）。",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            }),
            json!({
                "name": "dbmind_list_types",
                "description": "列出支持的连接类型及其运行时模式与能力（来自 YAML 类型目录）。",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            }),
            json!({
                "name": "dbmind_list_tables",
                "description": "列出指定连接下的表与视图。",
                "inputSchema": {
                    "type": "object",
                    "properties": { "connection": { "type": "string", "description": "连接名称或 id" } },
                    "required": ["connection"],
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "dbmind_describe_table",
                "description": "查看表结构（列名、类型、是否可空、主键、默认值）。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string" },
                        "table": { "type": "string" }
                    },
                    "required": ["connection", "table"],
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "dbmind_execute_query",
                "description": "执行一条 SQL。默认只允许只读语句（SELECT/SHOW/EXPLAIN 等）；写语句需用户在 DBMind 中开启允许 AI 写入。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string" },
                        "sql": { "type": "string" },
                        "max_rows": { "type": "integer", "minimum": 1, "maximum": dbmind_core::QueryOptions::HARD_MAX_ROWS, "description": "单页最大行数，默认 2000；内核硬上限 100000，传更大也只会按上限截断" },
                        "timeout_ms": { "type": "integer", "minimum": 1000, "description": "超时毫秒，默认 30000" }
                    },
                    "required": ["connection", "sql"],
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "dbmind_list_history",
                "description": "查看近期查询历史（含失败与取消）。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                        "connection": { "type": "string" }
                    },
                    "additionalProperties": false
                }
            }),
        ]
    }
}

/// 把结果集转成「对象数组」，对模型最友好。
fn render_rows(result: &dbmind_core::QueryResult) -> Vec<Value> {
    result
        .rows
        .iter()
        .map(|row| {
            let mut object = serde_json::Map::new();
            for (i, column) in result.columns.iter().enumerate() {
                let value = row.get(i).cloned().unwrap_or(dbmind_core::CellValue::Null);
                object.insert(column.name.clone(), cell_to_json(&value));
            }
            Value::Object(object)
        })
        .collect()
}

fn cell_to_json(cell: &dbmind_core::CellValue) -> Value {
    use dbmind_core::CellValue;
    match cell {
        CellValue::Null => Value::Null,
        CellValue::Integer(v) => json!(v),
        CellValue::Real(v) => json!(v),
        CellValue::Text(v) => json!(v),
        CellValue::Blob { len } => json!(format!("<blob {len}B>")),
    }
}

fn required_str(args: &Value, key: &str) -> dbmind_core::Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| DbMindError::new(ErrorCode::QueryInvalid, format!("缺少必填参数「{key}」")))
}

fn jsonrpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// JSON-RPC 的 id 可以是字符串也可以是数字；统一成字符串当键。
///
/// 登记与查找必须用**同一个**函数：否则「登记时是 `7`、查找时是 `"7"`」这种不一致
/// 会让取消静默失效 —— 而它看起来完全正常（通知被收下、没有报错、只是没取消成）。
fn id_key(id: &Value) -> String {
    match id {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
