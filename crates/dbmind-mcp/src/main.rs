//! DBmind MCP 壳（stdio，JSON-RPC 2.0）。
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
    /// 读一条设置（**每次现读**：壳是长驻进程，设置页改完下一次调用就要生效，
    /// 不能在启动时快照一次了事）。
    fn setting(&self, key: &str) -> Option<String> {
        self.engine
            .settings()
            .ok()?
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// 布尔设置（缺省 fallback；'true'/'1' 视为开）。
    fn flag(&self, key: &str, fallback: bool) -> bool {
        match self.setting(key).as_deref() {
            Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            _ => fallback,
        }
    }

    /// 正整数设置（非法/缺省 → fallback）。
    fn num(&self, key: &str, fallback: u64) -> u64 {
        self.setting(key)
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(fallback)
    }

    /// 工具属于哪个类别（设置里按类别开关）。
    fn category_of(name: &str) -> &'static str {
        match name {
            "dbmind_list_connections" | "dbmind_list_types" | "dbmind_list_tables"
            | "dbmind_list_databases" | "dbmind_search_tables" | "dbmind_describe_table" => {
                Store::KEY_MCP_TOOLS_STRUCTURE
            }
            "dbmind_execute_query" | "dbmind_cancel_query" => Store::KEY_MCP_TOOLS_QUERY,
            _ => Store::KEY_MCP_TOOLS_HISTORY,
        }
    }

    /// 该工具当前是否开放（tools/list 过滤 + dispatch 兜底拦截共用）。
    fn tool_enabled(&self, name: &str) -> bool {
        self.flag(Self::category_of(name), true)
    }

    /// 读一次脱敏配置。
    ///
    /// **每次现读**（与其它设置同口径）：壳是长驻进程，用户在设置页改完规则，
    /// 下一次工具调用就该生效 —— 启动时快照一次会让「改了没反应」变成常态。
    /// 解析失败按「未启用」处理：一条坏配置不该让查询直接失败。
    fn mask_config(&self) -> dbmind_core::mask::MaskConfig {
        let mut config = self
            .setting(dbmind_core::mask::KEY)
            .and_then(|text| serde_json::from_str::<dbmind_core::mask::MaskConfig>(&text).ok())
            .unwrap_or_default();
        config.normalize();
        config
    }

    /// AES 密钥：规则真用到 AES 才取。
    ///
    /// 没有存储的密钥时**生成一把并落盘**，与 web 侧同口径 —— 两边谁先用到谁建，
    /// 另一处直接读同一把；否则同一列在 MCP 与 AI 上下文里会是两份解不开的密文。
    fn mask_key(&self, config: &dbmind_core::mask::MaskConfig) -> dbmind_core::mask::MaskKey {
        if !config.uses_aes() {
            return [0u8; 32];
        }
        if let Some(text) = self.setting(dbmind_core::mask::KEY_AES) {
            if !text.trim().is_empty() {
                return dbmind_core::mask::key_from_text(&text);
            }
        }
        let text = dbmind_core::mask::random_key_text();
        if let Err(err) = self.engine.set_setting(dbmind_core::mask::KEY_AES, &text) {
            // 落盘失败仍用这一把：本次调用结果是可解的，只是重启后换成另一把
            eprintln!("AES 密钥落盘失败（本次仍可用）: {err}");
        }
        dbmind_core::mask::key_from_text(&text)
    }

    /// 规则里的「连接」可能是 **id 也可能是名称**（界面存 id，MCP 客户端习惯传名称），
    /// 所以两种写法都要试 —— 只比一种，就会出现「设置了规则、MCP 客户端照样拿到明文」。
    fn mask_conn_keys(&self, connection: &str) -> Vec<String> {
        let mut keys = vec![connection.to_string()];
        if let Ok(Some(record)) = self.engine.find_connection(connection) {
            if !record.id.trim().is_empty() {
                keys.push(record.id);
            }
            if !record.config.name.trim().is_empty() {
                keys.push(record.config.name);
            }
        }
        keys
    }

    /// 对 `dbmind_execute_query` 的结果行应用 `Mcp` 通道的脱敏规则。
    ///
    /// 返回（脱敏后的行，被脱敏的列数）。列数只为让客户端知道「这是摘要不是原文」。
    fn mask_mcp_rows(&self, connection: &str, rows: Vec<Value>) -> (Vec<Value>, usize) {
        let config = self.mask_config();
        if config.rules.is_empty() || !config.channel_on(dbmind_core::mask::MaskChannel::Mcp) {
            return (rows, 0);
        }
        let key = self.mask_key(&config);
        let conns = self.mask_conn_keys(connection);
        mask_rows(rows, &config, &key, &conns)
    }

    /// connection 参数：客户端没传时回落到「默认连接」设置（空 = 照旧报缺参）。
    fn connection_arg(&self, args: &Value) -> dbmind_core::Result<String> {
        match args.get("connection").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => Ok(s.to_string()),
            _ => match self.setting(Store::KEY_MCP_DEFAULT_CONNECTION).as_deref() {
                Some(s) if !s.trim().is_empty() => Ok(s.to_string()),
                _ => required_str(args, "connection"),
            },
        }
    }

    /// 设置页那个「默认库」：**只对默认连接生效**。
    ///
    /// 为什么不无条件套用：模型换了一条连接还硬塞这个库名，那个库在别的服务器上多半
    /// 不存在 —— 报「库不存在」比让它自己列库更难懂。
    ///
    /// 判据放宽到「用的就是默认连接」，而不是「没传 connection」：模型有把 connection
    /// 显式重复一遍的习惯（哪怕用的就是默认那条），按传参判会让用户觉得「默认库没生效」。
    fn default_database_arg(
        &self,
        record: &dbmind_core::ConnectionRecord,
    ) -> Option<String> {
        let configured = self
            .setting(Store::KEY_MCP_DEFAULT_DATABASE)
            .unwrap_or_default();
        let configured = configured.trim().to_string();
        if configured.is_empty() {
            return None;
        }
        let default_connection = self
            .setting(Store::KEY_MCP_DEFAULT_CONNECTION)
            .unwrap_or_default();
        let default_connection = default_connection.trim();
        if default_connection.is_empty() {
            return None;
        }
        // 设置页存的是名字，客户端可能传 id：两种都认
        let is_default = record.config.name.trim() == default_connection
            || record.id.trim() == default_connection;
        if is_default {
            Some(configured)
        } else {
            None
        }
    }

    /// `database` / `schema` 的最终取值：客户端传的优先，其次默认连接上的「默认库」。
    fn scope_of(
        &self,
        args: &Value,
        record: &dbmind_core::ConnectionRecord,
    ) -> (Option<String>, Option<String>) {
        let (database, schema) = scope_args(args, record);
        if database.is_some() || schema.is_some() {
            return (database, schema);
        }
        match self.default_database_arg(record) {
            Some(filled) => split_scope(Some(filled), None, record.kind().traits().schema_aware),
            None => (None, None),
        }
    }

    /// 取连接记录（元数据工具要判断「这个连接有没有绑定库」）。
    fn record_of(
        &self,
        connection: &str,
    ) -> dbmind_core::Result<dbmind_core::ConnectionRecord> {
        self.engine.find_connection(connection)?.ok_or_else(|| {
            DbMindError::new(
                ErrorCode::QueryInvalid,
                format!(
                    "连接不存在：{connection}。先调 dbmind_list_connections 看有哪些（名称或 id 都认）"
                ),
            )
        })
    }

    /// 「这个连接没指定库」的守门：**报错，而不是给一个空清单**。
    ///
    /// 这是本轮最关键的一条。MySQL 在没选库时 `list_tables` 会**成功返回 0 张表**
    /// （不是报错），模型据此认定「这个连接里没有表」，转头去几百条连接里瞎猜 ——
    /// 实测就是这样烧掉 4 分钟、最后什么都没查成。
    ///
    /// 「空结果」和「我不知道」是两件事，必须分开。报错里直接带上可用库清单，
    /// 模型下一步就能自己纠正，不需要用户介入。
    fn require_database(
        &self,
        record: &dbmind_core::ConnectionRecord,
        database: Option<&str>,
    ) -> dbmind_core::Result<()> {
        if database.is_some() || record.kind().traits().single_database {
            return Ok(());
        }
        let bound = record.config.database.as_deref().unwrap_or("").trim();
        if !bound.is_empty() {
            return Ok(());
        }
        // 非 SQL 类型（Mongo / Redis / ES）没有「库」这一层，不该被这条规则牵连
        if databases_sql(record.kind().key()).is_none() {
            return Ok(());
        }
        let names = self.databases_of(record).unwrap_or_default();
        let shown: Vec<&str> = names.iter().take(30).map(String::as_str).collect();
        let more = names.len().saturating_sub(shown.len());
        let list = match (shown.is_empty(), more) {
            (true, _) => "（库清单取不到，先调 dbmind_list_databases）".to_string(),
            (false, 0) => shown.join(", "),
            (false, rest) => format!("{} …（另有 {rest} 个，见 dbmind_list_databases）", shown.join(", ")),
        };
        let example = shown.first().copied().unwrap_or("库名");
        Err(DbMindError::new(
            ErrorCode::QueryInvalid,
            format!(
                "连接「{}」没有指定库，按库取表只会得到一个空清单 —— 那**不代表**没有表。\n\
                 可用库：{list}\n\
                 二选一：① 传 database 参数（例如 database = \"{example}\"）；\
                 ② 在 SQL 里写全限定名（例如 {example}.某表）。",
                record.config.name
            ),
        ))
    }

    /// 跑一条只读元数据 SQL，取**第一列**当字符串清单（库 / 模式共用）。
    fn scalar_column(
        &self,
        record: &dbmind_core::ConnectionRecord,
        sql: &str,
    ) -> dbmind_core::Result<Vec<String>> {
        // internal：这类系统探测不该进用户的查询历史
        let request = QueryRequest::new(record.id.clone(), sql.to_string()).with_internal(true);
        let result = self.engine.execute(request, AccessContext::Mcp)?;
        let mut names: Vec<String> = result
            .rows
            .iter()
            .filter_map(|row| row.first())
            .filter_map(|cell| match cell_to_json(cell) {
                Value::String(text) => Some(text),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }

    /// 这个连接上有哪些库。
    fn databases_of(
        &self,
        record: &dbmind_core::ConnectionRecord,
    ) -> dbmind_core::Result<Vec<String>> {
        match databases_sql(record.kind().key()) {
            Some(sql) => self.scalar_column(record, sql),
            // 没有「库」这一层的类型（SQLite 一个文件一个库 / NoSQL）：给空清单，不是错误
            None => Ok(Vec::new()),
        }
    }

    /// `dbmind_list_databases` 的载荷：库清单 + 当前绑定 + 怎么定位的提示。
    fn describe_databases(
        &self,
        record: &dbmind_core::ConnectionRecord,
    ) -> dbmind_core::Result<Value> {
        let traits = record.kind().traits();
        let bound = record.config.database.clone().unwrap_or_default();
        let all = self.databases_of(record)?;
        let (system, user): (Vec<String>, Vec<String>) =
            all.into_iter().partition(|name| is_system_database(name));
        let mut payload = json!({
            "connection": record.config.name,
            "kind": record.kind().key(),
            "boundDatabase": bound,
            // 设置页「默认库」：不传 connection / database 时会自动用它
            "settingsDefaultDatabase": self.default_database_arg(record).unwrap_or_default(),
            "schemaAware": traits.schema_aware,
            "databases": user,
            // 系统库单列出来：模型不必处理，但「它们存在」是事实
            "systemDatabases": system,
        });
        // 模式是「库」下的一层：连接没绑库时查到的是登录默认库的模式，给了反而误导
        if traits.schema_aware && !bound.trim().is_empty() {
            if let Some(sql) = schemas_sql(record.kind().key()) {
                if let Ok(schemas) = self.scalar_column(record, sql) {
                    payload["schemas"] = json!(schemas);
                }
            }
        }
        payload["hint"] = json!(if traits.single_database {
            "这个类型一个文件就是一个库，不需要按库定位"
        } else if bound.trim().is_empty() {
            "本连接没有指定库：元数据工具要传 database 参数（database = \"库名\"）；SQL 里也可以直接写 库.表"
        } else if traits.schema_aware {
            "本连接已绑定库；database 参数可切库，模式可以写成 database = \"库.模式\" 或单独传 schema"
        } else {
            "本连接已绑定库；database 参数可以切库，SQL 里也可以直接写 库.表"
        });
        Ok(payload)
    }

    /// 执行失败时，若这个连接压根没绑库，补一句「可能跟没选库有关」的提示。
    ///
    /// 只加一句**固定文案**，不额外查库清单：错误路径上再打一次数据库，
    /// 遇上「驱动没就绪 / 主机不通」时会把 1 秒的报错拖成 30 秒的超时。
    /// 需要库清单的模型自己会去调 `dbmind_list_databases`。
    fn explain_unbound(&self, connection: &str, err: DbMindError) -> DbMindError {
        let Ok(Some(record)) = self.engine.find_connection(connection) else {
            return err;
        };
        let bound = record.config.database.as_deref().unwrap_or("").trim();
        if !bound.is_empty() || record.kind().traits().single_database {
            return err;
        }
        if databases_sql(record.kind().key()).is_none() {
            return err;
        }
        DbMindError::new(
            err.code,
            format!(
                "{}\n\n提示：连接「{}」没有指定库。若错误是「找不到表 / 没有选中库」，请把 SQL 写成\
                 全限定名（库.表），或先调 dbmind_list_databases 挑库、再用 database 参数重试。",
                err.message, record.config.name
            ),
        )
    }

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
                // 这段是模型读到的第一条提示：把「连接 → 库 → 表」的定位顺序写在这里，
                // 比让它从工具报错里自己摸索快得多（实测它会在几百条连接里瞎猜）。
                "instructions": "DBmind 数据库工作台，连接的是用户在 DBmind 里配好的数据连接（名称或 id 都认）。\
定位顺序是「连接 → 库 → 表」：连接未必指定了库，若 list_tables 报「没有指定库」，先调 \
dbmind_list_databases 拿库清单，再给 database 参数（PG / SQL Server / Kingbase 还可写「库.模式」或单独传 schema）。\
默认只读；写语句需用户在 DBmind 中开启「允许 AI 写入」。"
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
        // 类别开关兜底拦截：tools/list 过滤了清单，但客户端可能缓存了旧清单硬调 —— 这里再拦一道
        if !self.tool_enabled(name) {
            return Err(DbMindError::new(
                ErrorCode::QueryInvalid,
                "该工具类别已在 DBmind 设置 → MCP 中关闭",
            ));
        }
        match name {
            "dbmind_list_connections" => {
                let records = self.engine.list_connections()?;
                // 影子连接（跨库浏览时按需生成的「父 ▸ 库」记录）**不给模型看**：
                // 界面上它们本来就是实现细节（web 的 `/api/connections` 也会滤掉），
                // 而一条连接能展开出十几个影子 —— 361 条清单里 172 条是影子，
                // 模型在这一堆里挑连接纯属浪费上下文，还容易挑到「某个库的副本」。
                //
                // 但影子携带的信息是有用的：它说明**这个库被展开过**。所以折成父连接上的
                // `expandedDatabases`，既减噪又不丢线索。
                let mut expanded: HashMap<String, Vec<String>> = HashMap::new();
                for record in &records {
                    let Some(base) = record.shadow_of() else {
                        continue;
                    };
                    let database = record.config.database.clone().unwrap_or_default();
                    let database = database.trim().to_string();
                    // 系统库展开过也不值得占上下文：这条线索的用途是「业务表在哪」
                    if database.is_empty() || is_system_database(&database) {
                        continue;
                    }
                    expanded.entry(base.to_string()).or_default().push(database);
                }

                let mut list = Vec::new();
                for record in &records {
                    if record.is_shadow() {
                        continue;
                    }
                    let mut value = serde_json::to_value(record)?;
                    // 口令类字段**一律不外发**：数据库口令在内核里已 `skip_serializing`，但 SSH 隧道的
                    // 口令 / 口令短语存在 `extra` 这个自由 JSON 里（键 `ssh.password` / `keyPassphrase`）。
                    // 不在这里抹掉，「列出连接（不含口令）」就是一句假话 —— 明文会随工具结果
                    // 进模型上下文（实测 355 条连接里带出了 SSH 口令）。
                    strip_secrets(&mut value);
                    if let Some(mut databases) = expanded.get(&record.id).cloned() {
                        databases.sort();
                        databases.dedup();
                        value["expandedDatabases"] = json!(databases);
                    }
                    list.push(value);
                }
                Ok(json!({
                    // 带上总数：模型据此判断「要不要先筛一遍」而不是把整份清单背下来
                    "total": list.len(),
                    "connections": list,
                }))
            }
            "dbmind_list_types" => Ok(serde_json::to_value(self.engine.types())?),
            "dbmind_list_tables" => {
                let connection = self.connection_arg(args)?;
                let record = self.record_of(&connection)?;
                let (database, schema) = self.scope_of(args, &record);
                // 没给库就按连接自身的库查（老行为），给了才走 scoped —— 两路都不写缓存键之外的东西
                self.require_database(&record, database.as_deref())?;
                let tables = if database.is_none() && schema.is_none() {
                    // 结构**一律现读**：AI 拿到 5 分钟前的结构会写出不存在的表名，
                    // 而 AI 调元数据的频率本来就很低 —— 慢一点换准，值。
                    self.engine.list_tables_fresh(&connection)?
                } else {
                    self.engine.list_tables_scoped(
                        &connection,
                        database.as_deref(),
                        schema.as_deref(),
                    )?
                };
                Ok(json!({
                    "connection": record.config.name,
                    "database": database.unwrap_or_else(|| record.config.database.clone().unwrap_or_default()),
                    "schema": schema,
                    "tables": tables,
                }))
            }
            "dbmind_search_tables" => {
                let connection = self.connection_arg(args)?;
                let record = self.record_of(&connection)?;
                let (database, schema) = self.scope_of(args, &record);
                self.require_database(&record, database.as_deref())?;
                let keyword = required_str(args, "keyword")?.to_lowercase();
                let all = if database.is_none() && schema.is_none() {
                    self.engine.list_tables_fresh(&connection)?
                } else {
                    self.engine.list_tables_scoped(
                        &connection,
                        database.as_deref(),
                        schema.as_deref(),
                    )?
                };
                // 模型经常只知道「大概有张 order 表」：模糊过滤省它翻全量清单，
                // 结果截到 50 条 —— 全量它也记不住。
                let matched: Vec<_> = all
                    .iter()
                    .filter(|t| {
                        serde_json::to_string(t)
                            .map(|s| s.to_lowercase().contains(&keyword))
                            .unwrap_or(false)
                    })
                    .take(50)
                    .collect();
                Ok(json!({ "matched": matched.len(), "tables": matched }))
            }
            "dbmind_describe_table" => {
                let connection = self.connection_arg(args)?;
                let record = self.record_of(&connection)?;
                let (database, schema) = self.scope_of(args, &record);
                self.require_database(&record, database.as_deref())?;
                let table = required_str(args, "table")?;
                let columns = if database.is_none() && schema.is_none() {
                    self.engine.list_columns_fresh(&connection, &table)?
                } else {
                    self.engine.list_columns_scoped(
                        &connection,
                        &table,
                        database.as_deref(),
                        schema.as_deref(),
                    )?
                };
                Ok(json!({ "table": table, "columns": columns }))
            }
            "dbmind_list_databases" => {
                let connection = self.connection_arg(args)?;
                let record = self.record_of(&connection)?;
                Ok(self.describe_databases(&record)?)
            }
            "dbmind_execute_query" => {
                let connection = self.connection_arg(args)?;
                let sql = required_str(args, "sql")?;
                let mut options = QueryOptions::default();
                // 行数与超时的默认值/上限都来自设置页（MCP 页签）：客户端没传用设置的默认；
                // 传了也会被设置的「最大行数」夹住 —— 界面设 100 行，模型要 10 万行也拿不到。
                let cap = self.num(Store::KEY_MCP_MAX_ROWS, 2000).min(dbmind_core::QueryOptions::HARD_MAX_ROWS as u64);
                options.max_rows = match args.get("max_rows").and_then(|v| v.as_u64()) {
                    Some(n) => n.min(cap) as usize,
                    None => cap as usize,
                };
                options.timeout_ms = match args.get("timeout_ms").and_then(|v| v.as_u64()) {
                    Some(ms) => ms,
                    None => self.num(Store::KEY_MCP_TIMEOUT_SECS, 30) * 1000,
                };
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
                        // 克隆一份：下面的脱敏还要按连接名/ id 去匹配规则
                        QueryRequest::new(connection.clone(), sql)
                            .with_options(options)
                            .with_execution_id(execution_id.clone()),
                        AccessContext::Mcp,
                    )
                };
                self.running.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
                let result = match outcome {
                    Ok(result) => result,
                    // 失败也要看得懂：空库下 `select * from orders` 只会报
                    // `No database selected`（MySQL）/ `Invalid object name`（SQL Server），
                    // 模型看不出根因是「没选库」，于是换一条连接再试 —— 一换就是几分钟。
                    Err(err) => return Err(self.explain_unbound(&connection, err)),
                };
                // 字段脱敏（`Mcp` 通道）：这里返回的是真实数据行，一旦发出去就是出站流量 ——
                // 与 AI 上下文用同一份规则与算法（`dbmind_core::mask`），否则「设置里配了、
                // MCP 客户端却拿到明文」就是最糟的那种不一致。
                let (rows, masked) = self.mask_mcp_rows(&connection, render_rows(&result));
                Ok(json!({
                    "executionId": result.execution_id,
                    "statementKind": result.statement_kind.as_str(),
                    "columns": result.columns,
                    "rowCount": result.row_count,
                    "truncated": result.truncated,
                    "durationMs": result.duration_ms,
                    "notices": result.notices,
                    // 有多少列按规则被打了码：客户端据此知道「看到的是摘要，不是原文」
                    "maskedColumns": masked,
                    "rows": rows,
                }))
            }
            "dbmind_cancel_query" => {
                let execution_id = required_str(args, "execution_id")?;
                let hit = self.engine.cancel(&execution_id);
                Ok(json!({ "executionId": execution_id, "cancelled": hit }))
            }
            "dbmind_list_history" => {
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                let connection = args.get("connection").and_then(|v| v.as_str());
                Ok(serde_json::to_value(self.engine.history(limit, connection)?)?)
            }
            "dbmind_server_info" => {
                // 让模型自检环境：写通道开没开、生产保护在不在、哪些类别开放 ——
                // 免得它拿着写语句反复撞墙（或以为能写其实不能）。
                Ok(json!({
                    "version": dbmind_core::VERSION,
                    "protocolVersion": PROTOCOL_VERSION,
                    "aiWriteEnabled": self.setting(Store::KEY_AI_WRITE_ENABLED).as_deref() == Some("true"),
                    // 生产保护默认开启：键缺失时也报 true（否则模型会以为保护没开）
                    "productionProtection": self.flag(Store::KEY_PROTECT_PRODUCTION, true),
                    "blockDangerous": self.setting(Store::KEY_BLOCK_DANGEROUS).as_deref() == Some("true"),
                    "maxWriteRows": self.setting(Store::KEY_MAX_WRITE_ROWS).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0),
                    "defaultConnection": self.setting(Store::KEY_MCP_DEFAULT_CONNECTION).unwrap_or_default(),
                    "defaultDatabase": self.setting(Store::KEY_MCP_DEFAULT_DATABASE).unwrap_or_default(),
                    "toolCategories": {
                        "structure": self.flag(Store::KEY_MCP_TOOLS_STRUCTURE, true),
                        "query": self.flag(Store::KEY_MCP_TOOLS_QUERY, true),
                        "history": self.flag(Store::KEY_MCP_TOOLS_HISTORY, true),
                    }
                }))
            }
            other => Err(DbMindError::new(
                ErrorCode::Internal,
                format!("未知工具: {other}"),
            )),
        }
    }

    fn tools(&self) -> Vec<Value> {
        // 按设置页的类别开关过滤：关掉的类别连清单里都不出现 —— 模型不会去调一个看不见的工具
        let all: Vec<Value> = vec![
            json!({
                "name": "dbmind_list_connections",
                "description": "列出 DBmind 中已配置的数据连接（不含口令）。每条连接可能带 expandedDatabases（用户展开过的库，只是线索、不一定全，以 dbmind_list_databases 为准）。返回值里的 name 或 id 都能当其它工具的 connection 参数。",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            }),
            json!({
                "name": "dbmind_list_types",
                "description": "列出支持的连接类型及其运行时模式与能力（来自 YAML 类型目录）。",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            }),
            json!({
                "name": "dbmind_list_tables",
                "description": "列出某个库下的表与视图。连接没绑定库时必须传 database —— 否则返回的是带可用库清单的**错误**，而不是空清单（空清单会被误读成「没有表」）。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string", "description": "连接名称或 id，省略时用设置里的默认连接" },
                        "database": { "type": "string", "description": "库名；schema 层级的类型可写「库.模式」。省略时用连接绑定的库" },
                        "schema": { "type": "string", "description": "模式（PG / SQL Server / Kingbase），也可直接写进 database" }
                    },
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "dbmind_list_databases",
                "description": "列出连接上的库（connection 已绑定库时附带模式清单）。不确定该操作哪个库时先调它。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string", "description": "省略时用默认连接" }
                    },
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "dbmind_search_tables",
                "description": "按关键字模糊搜索表/视图（名称匹配，最多返回 50 条）—— 比翻全量清单省事。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string", "description": "省略时用默认连接" },
                        "keyword": { "type": "string", "description": "表名关键字，不区分大小写" },
                        "database": { "type": "string", "description": "库名（schema 层级可写「库.模式」），省略时用连接绑定的库" },
                        "schema": { "type": "string", "description": "模式名" }
                    },
                    "required": ["keyword"],
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "dbmind_describe_table",
                "description": "查看表结构（列名、类型、是否可空、主键、默认值）。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string", "description": "省略时用默认连接" },
                        "table": { "type": "string" },
                        "database": { "type": "string", "description": "库名（schema 层级可写「库.模式」），省略时用连接绑定的库" },
                        "schema": { "type": "string", "description": "模式名" }
                    },
                    "required": ["table"],
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "dbmind_execute_query",
                "description": "执行一条 SQL。默认只允许只读语句（SELECT/SHOW/EXPLAIN 等）；写语句需用户在 DBmind 中开启允许 AI 写入。行数与超时的上限由设置页约束。命中用户在 DBmind 里配置的「字段脱敏」规则时，相应列返回的是摘要/密文（maskedColumns 会写明列数）。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string", "description": "省略时用默认连接" },
                        "sql": { "type": "string" },
                        "max_rows": { "type": "integer", "minimum": 1, "maximum": dbmind_core::QueryOptions::HARD_MAX_ROWS, "description": "单页最大行数；会被设置页的「单次最大行数」进一步限制" },
                        "timeout_ms": { "type": "integer", "minimum": 1000, "description": "超时毫秒，默认取设置页的值" }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }
            }),
            json!({
                "name": "dbmind_cancel_query",
                "description": "取消一条正在执行的查询（用 execute_query 返回的 executionId）。",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "execution_id": { "type": "string" }
                    },
                    "required": ["execution_id"],
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
            json!({
                "name": "dbmind_server_info",
                "description": "查看 DBmind MCP 服务自身：版本、AI 写入是否开启、生产保护等安全开关状态、开放的工具类别。写语句被拒时先查这里。",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            }),
        ];
        all.into_iter()
            .filter(|tool| {
                tool.get("name")
                    .and_then(|v| v.as_str())
                    .map(|n| self.tool_enabled(n))
                    .unwrap_or(true)
            })
            .collect()
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
        CellValue::Decimal(v) => json!(v),
        CellValue::Text(v) => json!(v),
        CellValue::Bytes(data) => json!(format!("<blob {}B>", data.len())),
        CellValue::Blob { len } => json!(format!("<blob {len}B>")),
    }
}

/// 结果行按 `Mcp` 通道打码（[`Server::mask_mcp_rows`] 的纯函数部分，便于单测）。
///
/// 边界与 AI 侧一致，照实说：**表名未知**（JOIN / 子查询别名都可能），所以只应用
/// 「表为空 = 任意表」的规则；**库名也未知**，所以只靠「连接 + 列名」命中 ——
/// 钉了表或库的规则在这里不参与，宁可不匹配，也不把无关列打成 `***` 让客户端失明。
///
/// `conns` 是同一个连接的几种写法（调用方传的名字 / id）：规则里存的是 id、客户端习惯传
/// 名字，只比一种就会出现「设置了规则、客户端照样拿到明文」。
fn mask_rows(
    rows: Vec<Value>,
    config: &dbmind_core::mask::MaskConfig,
    key: &dbmind_core::mask::MaskKey,
    conns: &[String],
) -> (Vec<Value>, usize) {
    let mut masked: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let Value::Object(fields) = row else {
                return row;
            };
            let mut object = serde_json::Map::with_capacity(fields.len());
            for (column, value) in fields {
                // 只处理「值本身就是原文」的类型：NULL 保持 NULL（没信息可泄露，
                // 换成 *** 反而让客户端以为这列有值）；二进制早已是 `<blob nB>` 占位。
                let plain = match &value {
                    Value::String(text) => Some(text.clone()),
                    Value::Number(number) => Some(number.to_string()),
                    _ => None,
                };
                let method = plain.as_ref().and_then(|_| {
                    conns.iter().find_map(|conn| {
                        config.method_for(
                            dbmind_core::mask::MaskChannel::Mcp,
                            conn,
                            "",
                            None,
                            &column,
                        )
                    })
                });
                match (method, plain) {
                    (Some(method), Some(text)) => {
                        masked.insert(column.clone());
                        object.insert(
                            column,
                            Value::String(dbmind_core::mask::mask_value(&text, method, key)),
                        );
                    }
                    _ => {
                        object.insert(column, value);
                    }
                }
            }
            Value::Object(object)
        })
        .collect();
    (out, masked.len())
}

/// 常见系统库：列出来但单独归类，模型不必去动它们。
const SYSTEM_DATABASES: [&str; 20] = [
    "information_schema",
    "performance_schema",
    "mysql",
    "sys",
    "master",
    "tempdb",
    "model",
    "msdb",
    "postgres",
    "template0",
    "template1",
    "sysibm",
    "syscat",
    "sysstat",
    "syspublic",
    "sysproc",
    "dbsnmp",
    "outln",
    "xdb",
    "sysman",
];

fn is_system_database(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    SYSTEM_DATABASES.contains(&lower.as_str())
}

/// 「库」这一层的清单 SQL。
///
/// 与 web 的 `api::dialect.rs`（`Dialect::databases`）是**同一份口径** —— 那边给界面
/// 的树用，这边给模型用。MCP 是独立 crate、不依赖 web，所以只能各存一份：改口径时
/// 两边一起改。这里额外要求「没选库也能跑通」，因为它的使命正是帮模型挑一个库。
fn databases_sql(kind: &str) -> Option<&'static str> {
    match kind {
        // show databases 不需要先选库，正好能在「未绑库」的连接上跑
        "mysql" | "mariadb" | "doris" | "clickhouse" => Some("show databases"),
        "sqlserver" => Some(
            "select name from sys.databases where state = 0 and has_dbaccess(name) = 1 order by name",
        ),
        "postgresql" | "kingbase" => Some(
            "select datname as name from pg_database where datistemplate = false order by datname",
        ),
        // DB2 没有「库」这一层：schema 就是它的库
        "db2" => Some("select schemaname as name from syscat.schemata order by schemaname"),
        "oracle" | "dm" => Some("select username as name from all_users order by username"),
        "h2" => Some("select schema_name as name from information_schema.schemata order by schema_name"),
        "derby" => Some("select schemaname as name from sys.sysschemas order by schemaname"),
        // SQLite / 非 SQL 类型没有「库清单」这回事
        _ => None,
    }
}

/// 「模式」这一层的清单 SQL（只有 schema 层级的类型有）。
///
/// 模式是**库内**的一层，所以这些语句只能在连接已绑定库时执行。
fn schemas_sql(kind: &str) -> Option<&'static str> {
    match kind {
        "sqlserver" => Some("select name from sys.schemas order by name"),
        "postgresql" | "kingbase" => Some(
            "select nspname as name from pg_namespace where nspname not like 'pg\\_%' order by nspname",
        ),
        "db2" => Some("select schemaname as name from syscat.schemata order by schemaname"),
        "oracle" | "dm" => Some("select username as name from all_users order by username"),
        "h2" => Some(
            "select schema_name as name from information_schema.schemata order by schema_name",
        ),
        "derby" => Some("select schemaname as name from sys.sysschemas order by schemaname"),
        _ => None,
    }
}

/// `database` / `schema` 两个参数（MCP 侧新增的定位能力）。
///
/// `database` 允许写成 `库.模式`（与界面树、同步接口同一口径）：schema 层级的类型
/// 直接拆成两层 —— 让模型记两套写法，它只会记错一套。
fn scope_args(
    args: &Value,
    record: &dbmind_core::ConnectionRecord,
) -> (Option<String>, Option<String>) {
    let text = |key: &str| {
        args.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    split_scope(
        text("database"),
        text("schema"),
        record.kind().traits().schema_aware,
    )
}

/// [`scope_args`] 的纯逻辑部分（可单测）。
///
/// 只在 schema 层级的类型上拆 `库.模式`：MySQL 的 `192.168.2.216 ▸ api_demo` 这种
/// 名字里带点、库名本身也可能带点，一律拆会把库名拆坏。
fn split_scope(
    database: Option<String>,
    schema: Option<String>,
    schema_aware: bool,
) -> (Option<String>, Option<String>) {
    match (database, schema) {
        (Some(database), None) if schema_aware => match database.split_once('.') {
            Some((db, schema)) if !db.is_empty() && !schema.is_empty() => {
                (Some(db.to_string()), Some(schema.to_string()))
            }
            _ => (Some(database), None),
        },
        (database, schema) => (database, schema),
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

/// 递归抹掉连接 JSON 里的口令类字段。
///
/// 为什么需要它：`ConnectionRecord` 里数据库口令有 `skip_serializing` 兜着，但 `extra` 是
/// **自由 JSON** —— SSH 隧道的 `password` / `keyPassphrase` 就原样躺在里面，序列化挡不住。
/// 只看**键名**（不看值）：「`authType: "password"`」这种是普通取值，不该被误伤成 `***`。
fn strip_secrets(value: &mut Value) {
    const KEYS: [&str; 4] = ["password", "passphrase", "secret", "token"];
    match value {
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                let lower = key.to_ascii_lowercase();
                if KEYS.iter().any(|k| lower.contains(k)) && matches!(item, Value::String(_)) {
                    *item = Value::String("***".to_string());
                } else {
                    strip_secrets(item);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(strip_secrets),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbmind_core::mask::{MaskConfig, MaskMethod, MaskRule};

    fn config(mcp: bool, rules: Vec<MaskRule>) -> MaskConfig {
        MaskConfig {
            llm: false,
            mcp,
            grid: false,
            export: false,
            enabled: false,
            rules,
        }
    }

    fn rule(table: Option<&str>, column: &str, method: MaskMethod) -> MaskRule {
        MaskRule {
            connection_id: None,
            database: None,
            schema: None,
            table: table.map(str::to_string),
            column: column.to_string(),
            method,
            note: None,
        }
    }

    #[test]
    fn masks_values_by_column() {
        let rows = vec![
            json!({ "phone": "13812348000", "id": 7, "name": "张三", "note": null }),
            json!({ "phone": "13812348001", "id": 8, "name": "李四", "note": "ok" }),
        ];
        let cfg = config(true, vec![rule(None, "*phone", MaskMethod::Md5)]);
        let (out, masked) = mask_rows(rows, &cfg, &[0u8; 32], &["conn_a".to_string()]);
        assert_eq!(masked, 1);
        let first = out[0]["phone"].as_str().unwrap();
        assert_eq!(first.len(), 32);
        assert_ne!(first, "13812348000");
        // 数字列、NULL、其它列一律不动
        assert_eq!(out[0]["id"], json!(7));
        assert!(out[0]["note"].is_null());
        assert_eq!(out[0]["name"], json!("张三"));
        // 摘要长度 32、且不含原文
        let digest = out[1]["phone"].as_str().unwrap();
        assert_eq!(digest.len(), 32);
        assert!(!digest.contains("138"));
    }

    #[test]
    fn mcp_switch_off_keeps_plaintext() {
        let rows = vec![json!({ "phone": "13812348000" })];
        let cfg = config(false, vec![rule(None, "phone", MaskMethod::Star)]);
        let (out, masked) = mask_rows(rows, &cfg, &[0u8; 32], &["conn_a".to_string()]);
        assert_eq!(masked, 0);
        assert_eq!(out[0]["phone"], json!("13812348000"));
    }

    #[test]
    fn table_pinned_rules_do_not_hit_unknown_tables() {
        let rows = vec![json!({ "phone": "13812348000" })];
        let cfg = config(true, vec![rule(Some("users"), "phone", MaskMethod::Star)]);
        let (out, masked) = mask_rows(rows, &cfg, &[0u8; 32], &["conn_a".to_string()]);
        // 工具结果不知道表名：钉表规则不参与，宁可不匹配也不乱打码
        assert_eq!(masked, 0);
        assert_eq!(out[0]["phone"], json!("13812348000"));
    }

    #[test]
    fn connection_list_hides_ssh_secrets() {
        // 真实形状：SSH 口令存在 extra.ssh.password 里（自由 JSON，序列化挡不住）
        let mut list = json!([{
            "id": "conn_1",
            "name": "生产库",
            "username": "qa",
            "extra": {
                "authType": "password",
                "ssh": { "enabled": true, "user": "rick", "password": "Datacvg123!", "keyPassphrase": "p@ss" }
            }
        }]);
        strip_secrets(&mut list);
        let conn = &list[0];
        assert_eq!(conn["extra"]["ssh"]["password"], json!("***"));
        assert_eq!(conn["extra"]["ssh"]["keyPassphrase"], json!("***"));
        // 非机密的保持原样：`authType` 的值是 "password" 但键不是口令字段，不能误伤
        assert_eq!(conn["extra"]["authType"], json!("password"));
        assert_eq!(conn["extra"]["ssh"]["user"], json!("rick"));
        assert_eq!(conn["name"], json!("生产库"));
    }

    #[test]
    fn connection_id_or_name_both_work() {
        let rows = vec![json!({ "phone": "13812348000" })];
        let mut pinned = rule(None, "phone", MaskMethod::Star);
        pinned.connection_id = Some("conn_1".to_string());
        let cfg = config(true, vec![pinned]);
        // 客户端传的是名称，规则存的是 id：两种写法都要能命中
        let (_, hit) = mask_rows(
            rows.clone(),
            &cfg,
            &[0u8; 32],
            &["生产库".to_string(), "conn_1".to_string()],
        );
        assert_eq!(hit, 1);
        // 不在这个连接上：不命中
        let (_, miss) = mask_rows(rows, &cfg, &[0u8; 32], &["别的库".to_string()]);
        assert_eq!(miss, 0);
    }

    #[test]
    fn database_and_schema_split_only_for_schema_types() {
        // SQL Server / PG：`库.模式` 拆两层
        let (db, schema) = split_scope(Some("pipeline.dbo".into()), None, true);
        assert_eq!(db.as_deref(), Some("pipeline"));
        assert_eq!(schema.as_deref(), Some("dbo"));

        // MySQL：库名里的点不拆（库名确实可能带点，拆了就查不到）
        let (db, schema) = split_scope(Some("api_demo".into()), None, false);
        assert_eq!(db.as_deref(), Some("api_demo"));
        assert_eq!(schema, None);

        // 显式给了 schema 就以它为准，不再拆 database
        let (db, schema) = split_scope(Some("a.b".into()), Some("public".into()), true);
        assert_eq!(db.as_deref(), Some("a.b"));
        assert_eq!(schema.as_deref(), Some("public"));

        // 只有库名、没有点：原样
        let (db, schema) = split_scope(Some("pipeline".into()), None, true);
        assert_eq!(db.as_deref(), Some("pipeline"));
        assert_eq!(schema, None);

        // 首尾都是空：两个都没有
        assert_eq!(split_scope(None, None, true), (None, None));
    }

    #[test]
    fn databases_sql_covers_the_unbound_case() {
        // 这些类型「没选库」也能跑，正是 require_database 报错时要用的那一步
        assert_eq!(databases_sql("mysql"), Some("show databases"));
        assert_eq!(databases_sql("clickhouse"), Some("show databases"));
        assert!(databases_sql("sqlserver").unwrap().contains("sys.databases"));
        assert!(databases_sql("postgresql").unwrap().contains("pg_database"));
        // SQLite 一个文件一个库：没有库清单，也就不会被「未指定库」拦住
        assert_eq!(databases_sql("sqlite"), None);
        // 模式的语句只在 schema 层级存在
        assert!(schemas_sql("sqlserver").is_some());
        assert_eq!(schemas_sql("mysql"), None);
    }

    #[test]
    fn system_databases_are_recognized_case_insensitively() {
        assert!(is_system_database("INFORMATION_SCHEMA"));
        assert!(is_system_database(" master "));
        assert!(!is_system_database("api_demo"));
    }
}
