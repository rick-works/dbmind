//! SQLite 原生驱动（M1 的端到端垂直切片）。
//!
//! 刻意把「只读连接」落到物理层：只读连接用 `SQLITE_OPEN_READ_ONLY` 打开，
//! 即使安全策略漏判，数据库自己也会拒绝写 —— 两层防护，不靠单点正确。

use super::session_pool::{SessionBudget, SessionSource};
use super::{Driver, MetaCall, QueryCall};
use crate::error::{query_error, DbMindError, ErrorCode, Result};
use crate::statement::classify;
use crate::types::{
    CellValue, ColumnDetail, ColumnMeta, ConnectReport, ConnectionConfig, QueryResult, TableInfo, TableKind,
};
use crate::{CancelToken, ConnectionKind, RuntimeMode};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// 等原生连接时的轮询间隔：太密是空转，太疏则「取消」响应慢。
/// 与 agent 侧会话池的轮询同量级（100ms 上下）。
const LOCK_POLL: Duration = Duration::from_millis(50);

/// 结构浏览等幂等调用等连接的预算（它们没有自己的超时参数）。
const META_LOCK_WAIT: Duration = Duration::from_secs(30);

/// 取一条原生连接的使用权（同一文件只有一个连接对象）。
///
/// 为什么不能直接 `lock()`：原生单连接类型上，第二条语句会**阻塞在连接锁上** ——
/// 直接 `lock()` 的语义就是「一直等到前一条跑完，而且期间对取消毫无反应」。
/// 这里改成 `try_lock` 轮询，于是等待期间能做到两件事：
///
/// 1. **认取消令牌**：用户点「取消」立刻有反应（与 agent 侧会话池等待的要求一致）；
/// 2. **有上限**：等不到就明确报出来，而不是无限期挂着（与「连接被占满要能说清」一致）。
fn lock_connection<'a>(
    handle: &'a Mutex<Connection>,
    cancel: Option<&CancelToken>,
    wait: Duration,
) -> Result<MutexGuard<'a, Connection>> {
    let deadline = Instant::now() + wait;
    loop {
        if cancel.is_some_and(CancelToken::is_cancelled) {
            return Err(super::session_pool::cancelled_while_waiting());
        }
        match handle.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => return Ok(poisoned.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        if Instant::now() >= deadline {
            return Err(DbMindError::new(
                ErrorCode::QueryTimeout,
                "等待这条连接上的前一条语句结束超时",
            ));
        }
        std::thread::sleep(LOCK_POLL);
    }
}

/// 池里的一条物理连接。
struct Pooled {
    handle: Arc<Mutex<Connection>>,
    /// 打开时是否可写（决定能否直接拿来处理写请求）
    writable: bool,
    /// 最后一次**被取用**的时刻（全局配额回收时挑最久未用的）。
    ///
    /// 为什么记「取用」而不是「归还」：原生侧没有归还钩子（释放就是锁守卫 drop）。
    /// 也不会有害：真正在跑的连接是 busy 的（`try_lock` 失败），回收根本不会挑它。
    last_used: Instant,
    /// 是否可被全局配额回收。
    ///
    /// 内存库（`:memory:`）**不可回收**：关掉连接等于把用户的库删了 ——
    /// 那不是「重开一下就有」的状态，而是数据本身。它也因此不计入配额
    /// （计入又不可回收，会把「配额满」变成用户解不开的结）。
    reclaimable: bool,
}

/// 原生连接池：按文件复用**一条**连接，并把「空闲/回收」暴露给全局配额。
///
/// 与 agent 侧的 `SessionPool` 是同一件事的两种形态：agent 按泳道分槽位，
/// 原生侧一个文件就是一条（单连接语义由文件键本身保证）。
struct NativePool {
    entries: Mutex<HashMap<String, Pooled>>,
}

impl NativePool {
    fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pooled>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 对外的会话 id（与 `QueryResult::session_id` 同一形状）：原生侧「会话」就是那个文件。
fn session_id_of(path: &str) -> String {
    format!("file:{path}")
}

/// 从对外会话 id 反解回文件路径。
fn path_of(session_id: &str) -> Option<&str> {
    session_id.strip_prefix("file:")
}

/// 内存库（`:memory:` 及其 URI 变体）：关掉连接就把库删了。
fn is_ephemeral(path: &str) -> bool {
    path == ":memory:" || path.starts_with("file::memory:") || path.contains("mode=memory")
}

impl SessionSource for NativePool {
    fn oldest_idle(&self) -> Option<(String, Instant)> {
        let entries = self.lock();
        let mut best: Option<(String, Instant)> = None;
        for (path, entry) in entries.iter() {
            if !entry.reclaimable {
                continue;
            }
            // 「空闲」= 此刻没人持有这条连接的锁（`try_lock` 不阻塞；
            // 中毒也当作忙 —— 保守一点，绝不误关）
            if entry.handle.try_lock().is_err() {
                continue;
            }
            let better = best
                .as_ref()
                .map(|(_, when)| entry.last_used < *when)
                .unwrap_or(true);
            if better {
                best = Some((session_id_of(path), entry.last_used));
            }
        }
        best
    }

    fn take_idle(&self, session_id: &str) -> bool {
        let Some(path) = path_of(session_id) else {
            return false;
        };
        let mut entries = self.lock();
        let Some(entry) = entries.get(path) else {
            return false;
        };
        if !entry.reclaimable || entry.handle.try_lock().is_err() {
            return false;
        }
        // 移出表即「关」：`Pooled` 一 drop，连接句柄也随之释放
        //（下一条语句会自动重开 —— 文件型的库在磁盘上，重开就回来了）。
        entries.remove(path);
        true
    }
}

/// 池键（= 一条物理连接的身份）：文件型引擎按**文件路径**复用同一条句柄。
///
/// 它同时是对外的「会话 id」：原生驱动没有会话复用一说（一条连接就一个），
/// 但对使用者来说「这条语句跑在哪个库上」才是有意义的那个问题。
fn pool_key(config: &ConnectionConfig) -> Result<String> {
    Ok(config
        .resolved_file()
        .ok_or_else(|| {
            DbMindError::new(
                ErrorCode::ConnInvalid,
                "SQLite 连接缺少文件路径（filePath 或 database）",
            )
        })?
        .to_string())
}

pub struct SqliteDriver {
    /// 单连接池：声明 `singleConnectionPool` 的类型在这里保证**只有一条**物理连接。
    ///
    /// 为什么必须复用而不是每次新开：SQLite 的 `:memory:` 库与临时表都是
    /// **连接级**作用域 —— 新开一条连接就是另一个空库，于是
    /// `create table t` 之后 `select * from t` 会报「no such table」（实测复现过）。
    /// 声明单连接的正是这些嵌入式/文件引擎。
    pool: Arc<NativePool>,
    /// 全局会话配额：原生连接同样要计入，否则「全局」这句话有个说不出口的例外。
    budget: Arc<SessionBudget>,
}

impl Default for SqliteDriver {
    fn default() -> Self {
        // 独立预算（只有这一条驱动）：单测与临时用法不需要外部配置
        Self::new(Arc::new(SessionBudget::default()))
    }
}

impl SqliteDriver {
    pub fn new(budget: Arc<SessionBudget>) -> Self {
        let pool = Arc::new(NativePool::new());
        // 原生侧的「关闭」就是把条目从表里摘掉（连接随 `Pooled` 一起 drop），
        // 所以 closer 是空操作 —— 与 agent 侧要显式通知宿主不同。
        budget.register(
            Arc::downgrade(&(pool.clone() as Arc<dyn SessionSource>)),
            Arc::new(|_: &str| {}),
        );
        Self { pool, budget }
    }

    /// 取物理连接。
    ///
    /// - 声明 `singleConnectionPool` 的类型：同一个文件复用**同一条**；
    /// - 未声明的：每次新开、用完即关（留给将来接别的原生驱动）；
    /// - 新开一条之前要过**全局配额**（内存库除外，见 `is_ephemeral`）。
    fn handle(&self, config: &ConnectionConfig, want_write: bool) -> Result<Arc<Mutex<Connection>>> {
        let path = pool_key(config)?;
        let reclaimable = !is_ephemeral(&path);

        if !self.kind().single_connection_pool() {
            // 不驻留 ⇒ 不占配额（用完即关，攒不下来）
            return Ok(Arc::new(Mutex::new(Self::open(config, !want_write)?)));
        }

        // 已经开过这条文件了？直接复用（单连接语义）
        {
            let mut entries = self.pool.lock();
            if let Some(existing) = entries.get_mut(&path) {
                // 能读就一定复用；只有「现存是只读、这次却要写」才重开。
                // 正常路径走不到这一步：可写连接上的元数据也按可写取（见 meta_want_write）。
                if !want_write || existing.writable {
                    existing.last_used = Instant::now();
                    return Ok(existing.handle.clone());
                }
            }
        }

        // 要新开一条：先过全局配额。**必须在拿 entries 锁之前**做 ——
        // 配额满时会去回收别的池的会话，持着自己这把锁去锁别人就是死锁。
        if reclaimable {
            self.admit_session()?;
        }

        let handle = Arc::new(Mutex::new(Self::open(config, !want_write)?));
        let mut entries = self.pool.lock();
        // 拿锁后再看一眼：并发的两个请求可能同时走到这里，先到的那条已经开好了。
        // 这条竞态在单连接语义下不能含糊 —— 否则同一个文件会有两条连接
        //（内存库更糟：后开的那条看不见先开的那条写进去的东西）。
        let raced = entries
            .get(&path)
            .filter(|existing| !want_write || existing.writable)
            .map(|existing| existing.handle.clone());
        if let Some(existing) = raced {
            drop(entries);
            if reclaimable {
                self.budget.release_one(); // 预留了却没用上，还回去
            }
            return Ok(existing);
        }
        entries.insert(
            path,
            Pooled {
                handle: handle.clone(),
                writable: want_write,
                last_used: Instant::now(),
                reclaimable,
            },
        );
        Ok(handle)
    }

    /// 新开一条原生连接前过全局配额。
    ///
    /// 满员时先试着回收一条**空闲**会话（可能是 agent 侧的，也可能是别的文件的），
    /// 再试一次。仍不成则明确报错 —— 原生路径没有「等别人归还」这一说：
    /// 配额只能靠回收腾出来，空等大概率白等。
    fn admit_session(&self) -> Result<()> {
        if self.budget.try_reserve() {
            return Ok(());
        }
        if self.budget.reclaim_idle() && self.budget.try_reserve() {
            return Ok(());
        }
        Err(DbMindError::new(
            ErrorCode::QueryTimeout,
            format!(
                "全局会话配额已满（{} / {}），打不开新的 SQLite 连接",
                self.budget.live(),
                self.budget.cap()
            ),
        )
        .with_detail(
            "设置项 session.maxPerHost 是「所有连接加起来的物理会话上限」。\
             内核会优先回收**空闲**会话；正在执行语句的会话与内存库（`:memory:`）不可回收。\
             可以把上限调大，或减少同时打开的连接。"
                .to_string(),
        ))
    }

    /// 元数据要用写连接吗。
    ///
    /// 单连接类型必须搭数据那条连接，否则看不到**连接级作用域**里的对象
    /// （临时表、`:memory:` 库里的表）—— 用户会以为表丢了。
    /// 其余类型照旧只读打开：浏览结构不该要求写权限。
    fn meta_want_write(&self, read_only: bool) -> bool {
        self.kind().single_connection_pool() && !read_only
    }

    fn open(config: &ConnectionConfig, read_only: bool) -> Result<Connection> {
        let path = config.resolved_file().ok_or_else(|| {
            DbMindError::new(
                ErrorCode::ConnInvalid,
                "SQLite 连接缺少文件路径（filePath 或 database）",
            )
        })?;

        let flags = if read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        };
        Connection::open_with_flags(path, flags).map_err(|e| {
            DbMindError::new(
                ErrorCode::ConnConnectFailed,
                format!("打开 SQLite 文件失败：{path}"),
            )
            .with_detail(e.to_string())
        })
    }

    fn cell(value: ValueRef<'_>) -> CellValue {
        match value {
            ValueRef::Null => CellValue::Null,
            ValueRef::Integer(v) => CellValue::Integer(v),
            ValueRef::Real(v) => CellValue::Real(v),
            ValueRef::Text(v) => CellValue::Text(String::from_utf8_lossy(v).into_owned()),
            // 只回长度：结果集要跨进程/HTTP 传输，不能把二进制塞进去
            ValueRef::Blob(v) => CellValue::Blob { len: v.len() },
        }
    }
}

impl Driver for SqliteDriver {
    fn kind(&self) -> ConnectionKind {
        ConnectionKind::Sqlite
    }

    fn test(&self, config: &ConnectionConfig, read_only: bool) -> Result<ConnectReport> {
        let started = Instant::now();
        let conn = Self::open(config, read_only)?;
        let version: String = conn
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .map_err(query_error)?;
        Ok(ConnectReport {
            connection_id: String::new(),
            kind: ConnectionKind::Sqlite,
            runtime_mode: RuntimeMode::Native,
            latency_ms: started.elapsed().as_millis() as u64,
            server_version: Some(format!("SQLite {version}")),
            message: if read_only {
                "只读打开成功".to_string()
            } else {
                "连接成功".to_string()
            },
        })
    }

    fn query(&self, call: QueryCall<'_>) -> Result<QueryResult> {
        let started = Instant::now();
        let handle = self.handle(call.config, !call.read_only)?;
        // 原生驱动的「会话」就是那个文件（单连接类型：一条连接对应一个库）。
        // 与池内对外的会话 id 必须**同一形状** —— 配额回收是按 id 摘会话的。
        let session_id = pool_key(call.config).ok().map(|key| session_id_of(&key));
        // 同一文件上只有一条连接：第二条语句会排队。等待预算与 agent 侧同形
        // （查询自身的超时 + 2s），而且**等待期间认取消令牌**（见 `lock_connection`）。
        let wait = Duration::from_millis(call.options.timeout_ms.saturating_add(2_000));
        let conn = lock_connection(&handle, Some(call.cancel), wait)?;
        let statement_kind = classify(self.kind().protocol(), call.sql);

        let (columns, rows, truncated, affected_rows) = if statement_kind.is_read_only() {
            let mut stmt = conn.prepare(call.sql).map_err(query_error)?;
            // 先取列信息并转成自有数据（借用结束后才能可变借用 stmt 去取行）
            let columns: Vec<ColumnMeta> = stmt
                .columns()
                .iter()
                .map(|c| ColumnMeta {
                    name: c.name().to_string(),
                    type_name: c.decl_type().map(|s| s.to_string()),
                })
                .collect();
            let names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();

            let mut rows_out: Vec<Vec<CellValue>> = Vec::new();
            let mut truncated = false;
            {
                let mut rows = stmt.query([]).map_err(query_error)?;
                while let Some(row) = rows.next().map_err(query_error)? {
                    // 取消与超时在「行流」上检查：足以覆盖绝大多数长查询
                    if call.cancel.is_cancelled() {
                        return Err(DbMindError::new(ErrorCode::QueryCanceled, "查询已被取消"));
                    }
                    if started.elapsed().as_millis() as u64 > call.options.timeout_ms {
                        return Err(DbMindError::new(
                            ErrorCode::QueryTimeout,
                            format!("查询超过 {} ms 未完成，已中止", call.options.timeout_ms),
                        ));
                    }
                    let mut values = Vec::with_capacity(names.len());
                    for i in 0..names.len() {
                        values.push(Self::cell(row.get_ref(i).map_err(query_error)?));
                    }
                    rows_out.push(values);
                    // **多取一行**才敢说「被截断」：旧写法是「取满上限就置位」，于是
                    // 结果**恰好等于上限**时也会说「已截断」—— 界面上平白多一句话，而那份
                    // 结果其实是完整的。语义与 agent 侧一致（JDBC 宿主用
                    // `setMaxRows(maxRows + 1)` 从结果集那一头做同一件事）。
                    if rows_out.len() > call.options.max_rows {
                        rows_out.pop();
                        truncated = true;
                        break;
                    }
                }
            }
            (columns, rows_out, truncated, None)
        } else {
            // 写/DDL：走 execute（返回影响行数；DDL 通常为 0）
            let affected = conn.execute(call.sql, []).map_err(query_error)?;
            (Vec::new(), Vec::new(), false, Some(affected))
        };

        Ok(QueryResult {
            execution_id: call.execution_id.to_string(),
            connection_id: String::new(),
            connection_name: String::new(),
            statement_kind,
            columns,
            row_count: rows.len(),
            rows,
            affected_rows,
            truncated,
            // 由内核统一判定（见 engine::execute），驱动不掺和
            source_object: None,
            // SQL 的行标识来自结构元数据（主键），结果本身不携带
            row_identity: None,
            session_id,
            duration_ms: started.elapsed().as_millis() as u64,
            notices: Vec::new(),
        })
    }

    fn list_tables(&self, call: MetaCall<'_>) -> Result<Vec<TableInfo>> {
        let handle = self.handle(call.config, self.meta_want_write(call.read_only))?;
        let conn = lock_connection(&handle, None, META_LOCK_WAIT)?;
        let mut stmt = conn
            .prepare(
                "SELECT name, type FROM sqlite_master \
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name COLLATE NOCASE",
            )
            .map_err(query_error)?;
        let rows = stmt
            .query_map([], |row| {
                let name: String = row.get(0)?;
                let kind: String = row.get(1)?;
                Ok(TableInfo {
                    name,
                    kind: if kind == "view" {
                        TableKind::View
                    } else {
                        TableKind::Table
                    },
                    row_estimate: None,
                })
            })
            .map_err(query_error)?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(query_error)?);
        }
        Ok(out)
    }

    fn list_columns(&self, call: MetaCall<'_>, table: &str) -> Result<Vec<ColumnDetail>> {
        let handle = self.handle(call.config, self.meta_want_write(call.read_only))?;
        let conn = lock_connection(&handle, None, META_LOCK_WAIT)?;
        // 用表值函数形式的 pragma，避免手工拼表名带来的注入面
        let mut stmt = conn
            .prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1) ORDER BY cid")
            .map_err(query_error)?;
        let rows = stmt
            .query_map([table], |row| {
                let type_name: Option<String> = row.get(1)?;
                let primary_key = row.get::<_, i64>(4)? != 0;
                // SQLite 没有独立的"自增"标志：`integer primary key` 就是 rowid 的别名，
                // 会自动分配递增值，所以按这个判（其它的主键类型不算）。
                let auto_increment = primary_key
                    && type_name
                        .as_deref()
                        .is_some_and(|text| text.eq_ignore_ascii_case("integer"));
                Ok(ColumnDetail {
                    name: row.get(0)?,
                    type_name,
                    nullable: row.get::<_, i64>(2)? == 0,
                    default_value: row.get(3)?,
                    primary_key,
                    auto_increment,
                })
            })
            .map_err(query_error)?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(query_error)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::QueryOptions;
    use crate::{CancelToken, ConnectionKind};

    /// `singleConnectionPool` 的意义在这一组测试里：SQLite 的 `:memory:` 库与
    /// 临时表都是**连接级**作用域，新开一条连接就是另一个空世界。
    fn memory_config() -> ConnectionConfig {
        ConnectionConfig::new("mem", ConnectionKind::Sqlite).with_file(":memory:")
    }

    fn exec(
        driver: &SqliteDriver,
        config: &ConnectionConfig,
        sql: &str,
        read_only: bool,
    ) -> Result<QueryResult> {
        let options = QueryOptions::default();
        let cancel = CancelToken::new();
        driver.query(QueryCall {
            config,
            sql,
            options: &options,
            execution_id: "test",
            cancel: &cancel,
            read_only,
            session: None,
        })
    }

    fn run(driver: &SqliteDriver, config: &ConnectionConfig, sql: &str) -> QueryResult {
        exec(driver, config, sql, false).expect("语句应执行成功")
    }

    fn meta<'a>(config: &'a ConnectionConfig, read_only: bool) -> MetaCall<'a> {
        MetaCall { config, read_only }
    }

    fn temp_db(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dbmind-sqlite-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("test.db")
    }

    #[test]
    fn 内存库的语句走同一条连接() {
        let driver = SqliteDriver::default();
        let config = memory_config();

        run(&driver, &config, "create table t(a integer)");
        run(&driver, &config, "insert into t values (1), (2)");
        let result = run(&driver, &config, "select count(*) as n from t");

        assert_eq!(
            result.rows[0][0],
            CellValue::Integer(2),
            "新开连接的话这里会是 0 行（换了个空库）"
        );
        assert_eq!(driver.pool.lock().len(), 1, "只应有一条物理连接");
    }

    #[test]
    fn 临时表在同一个连接里可见() {
        let driver = SqliteDriver::default();
        let config = memory_config();

        run(&driver, &config, "create temp table tmp(a integer)");
        run(&driver, &config, "insert into tmp values (7)");
        let result = run(&driver, &config, "select a from tmp");

        assert_eq!(result.rows[0][0], CellValue::Integer(7));
    }

    /// 元数据必须搭数据那条连接，否则连接级作用域里的对象在结构浏览里看不见 ——
    /// 用户会以为表丢了。
    #[test]
    fn 元数据能看到连接级作用域里的对象() {
        let driver = SqliteDriver::default();
        let config = memory_config();
        run(&driver, &config, "create table t(a integer)");

        let tables = driver.list_tables(meta(&config, false)).expect("结构浏览应成功");
        assert!(
            tables.iter().any(|item| item.name == "t"),
            "应看到刚建的表：{tables:?}"
        );

        let columns = driver
            .list_columns(meta(&config, false), "t")
            .expect("列信息应成功");
        assert_eq!(columns.len(), 1);
        assert_eq!(driver.pool.lock().len(), 1, "始终只有一条物理连接");
    }

    /// 只读连接：数据与元数据复用**同一条只读**连接 —— 单连接与「只读打开」的双保险都在。
    #[test]
    fn 只读连接只开一条只读连接() {
        let file = temp_db("ro").to_string_lossy().to_string();
        let config = ConnectionConfig::new("ro", ConnectionKind::Sqlite).with_file(file);

        // 先备好数据（另一条可写连接，用完即释放）
        {
            let setup = SqliteDriver::default();
            run(&setup, &config, "create table t(a integer)");
        }

        let driver = SqliteDriver::default();
        let result = exec(&driver, &config, "select count(*) as n from t", true).expect("只读查询应成功");
        assert_eq!(result.rows[0][0], CellValue::Integer(0));

        let tables = driver.list_tables(meta(&config, true)).expect("只读元数据应成功");
        assert!(tables.iter().any(|item| item.name == "t"));

        let pool = driver.pool.lock();
        assert_eq!(pool.len(), 1, "只读连接也只应有一条");
        assert!(
            pool.values().all(|item| !item.writable),
            "只读连接不应以可写方式打开"
        );
    }

    /// 可写连接上的元数据应当复用**已存在的可写连接**，而不是另开一条只读的。
    #[test]
    fn 可写连接上的元数据复用可写连接() {
        let file = temp_db("rw").to_string_lossy().to_string();
        let config = ConnectionConfig::new("rw", ConnectionKind::Sqlite).with_file(file);

        let driver = SqliteDriver::default();
        run(&driver, &config, "create table t(a integer)");
        driver.list_tables(meta(&config, false)).expect("结构浏览应成功");

        let pool = driver.pool.lock();
        assert_eq!(pool.len(), 1);
        assert!(pool.values().all(|item| item.writable));
    }

    /// 文件型原生连接**计入**全局配额，且**可被回收**：回收后下一条语句自动重开。
    /// 代价如实：连接级状态（临时表）会丢 —— 磁盘上的数据不会。
    #[test]
    fn 文件型原生会话计入配额并可被回收() {
        let budget = Arc::new(SessionBudget::new(1));
        let driver = SqliteDriver::new(budget.clone());
        let file_a = temp_db("quota-a").to_string_lossy().to_string();
        let file_b = temp_db("quota-b").to_string_lossy().to_string();
        let a = ConnectionConfig::new("a", ConnectionKind::Sqlite).with_file(file_a.clone());
        let b = ConnectionConfig::new("b", ConnectionKind::Sqlite).with_file(file_b.clone());

        run(&driver, &a, "create temp table tt(x integer)");
        run(&driver, &a, "insert into tt values (1)");
        assert_eq!(budget.live(), 1, "文件型连接要计入配额");

        // B 也要一条：配额 1 已满 ⇒ 先回收 A 那条空闲连接
        run(&driver, &b, "select 1");
        assert_eq!(budget.live(), 1, "回收一条又开一条，总数不变");
        assert_eq!(driver.pool.lock().len(), 1);

        // A 再查：连接是重开的，临时表随之消失 —— 这就是回收的真实代价。
        // 原因在 `detail` 里（sqlite 侧的 message 是统一的「SQL 执行失败」，
        // 具体原因由驱动放进 detail —— JDBC 那边反过来放在 message，别按一边写死）
        let err = exec(&driver, &a, "select * from tt", false).unwrap_err();
        let said = format!("{} {}", err.message, err.detail.as_deref().unwrap_or_default());
        assert!(said.contains("no such table"), "{said}");

        // 但磁盘上的数据不受影响（这是我们敢回收文件型连接的前提）
        run(&driver, &a, "create table persisted(x integer)");
        run(&driver, &a, "insert into persisted values (1)");
        let rows = run(&driver, &a, "select count(*) as n from persisted");
        assert_eq!(rows.rows[0][0], CellValue::Integer(1));

        let _ = std::fs::remove_file(&file_a);
        let _ = std::fs::remove_file(&file_b);
    }

    /// 内存库**不计入配额、也不可回收**：关掉它等于把用户的库删了；
    /// 而「计入却回收不掉」会把配额变成用户解不开的结。
    #[test]
    fn 内存库不计入配额也不可回收() {
        let budget = Arc::new(SessionBudget::new(1));
        let driver = SqliteDriver::new(budget.clone());
        let memory = memory_config();
        let file = temp_db("quota-ephemeral").to_string_lossy().to_string();
        let on_disk = ConnectionConfig::new("d", ConnectionKind::Sqlite).with_file(file.clone());

        run(&driver, &memory, "create table kept(x integer)");
        run(&driver, &memory, "insert into kept values (1)");
        assert_eq!(budget.live(), 0, "内存库不占配额");

        // 配额只有 1，但别的连接照样开得起来（没被内存库占住）
        run(&driver, &on_disk, "select 1");
        assert_eq!(budget.live(), 1);

        // 可回收名单里只有磁盘上那条，内存库不在其中
        let idle = driver.pool.oldest_idle().expect("磁盘上那条应可回收");
        assert!(
            !idle.0.contains(":memory:"),
            "内存库不该出现在可回收名单里：{}",
            idle.0
        );

        // 内存库的数据必须还在
        let rows = run(&driver, &memory, "select count(*) as n from kept");
        assert_eq!(rows.rows[0][0], CellValue::Integer(1));

        let _ = std::fs::remove_file(&file);
    }

    /// 回收只挑**空闲**的原生连接：正在跑语句的那条绝不能摘。
    #[test]
    fn 回收不碰正在使用的原生连接() {
        let budget = Arc::new(SessionBudget::new(2));
        let driver = SqliteDriver::new(budget.clone());
        let file_a = temp_db("busy-a").to_string_lossy().to_string();
        let file_b = temp_db("busy-b").to_string_lossy().to_string();
        let a = ConnectionConfig::new("a", ConnectionKind::Sqlite).with_file(file_a.clone());
        let b = ConnectionConfig::new("b", ConnectionKind::Sqlite).with_file(file_b.clone());

        // 直接持住连接锁，模拟「这条正在执行语句」。
        // 用**可写**打开：只读打开一个还不存在的库文件会被 SQLite 直接拒掉
        //（这不是测试的细节，是真实语义 —— 只读连接不该创建库）。
        let busy = driver.handle(&a, true).expect("应能取到连接");
        let guard = busy.lock().unwrap_or_else(|e| e.into_inner());
        run(&driver, &b, "select 1");

        assert_eq!(
            driver.pool.oldest_idle().map(|(id, _)| id),
            Some(session_id_of(&file_b)),
            "可回收的应当是空闲的那条（b）"
        );
        assert!(
            !driver.pool.take_idle(&session_id_of(&file_a)),
            "正在用的连接摘不掉"
        );

        drop(guard);
        assert!(driver.pool.take_idle(&session_id_of(&file_a)), "空闲之后才摘得掉");

        let _ = std::fs::remove_file(&file_a);
        let _ = std::fs::remove_file(&file_b);
    }
}
