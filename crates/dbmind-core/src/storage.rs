//! 本地元数据库（SQLite 单库）。
//!
//! 连接、历史、设置、结构缓存**都在一个库里**：可事务、可查询、可迁移，
//! 不像散落的 JSON 文件那样会互相漂移。库文件默认 `~/.dbmind/dbmind.db`。

use crate::error::{DbMindError, ErrorCode, Result};
use crate::paths;
use crate::types::*;
use crate::ConnectionKind;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};

const SETTINGS_SAFETY_PRODUCTION: &str = "safety.protectProduction";
const SETTINGS_SAFETY_AI_WRITE: &str = "safety.aiWriteEnabled";
/// 危险语句拦截：无 WHERE 的 UPDATE/DELETE、TRUNCATE、DROP 一律拒绝。
const SETTINGS_SAFETY_BLOCK_DANGEROUS: &str = "safety.blockDangerousStatements";
/// 写操作影响行数上限：UPDATE/DELETE 预估影响行数超过该值时拒绝（0 = 不限制）。
const SETTINGS_SAFETY_MAX_WRITE_ROWS: &str = "safety.maxWriteRows";
// 注意：**没有** `ui.theme` 与 `query.defaultPageSize` —— 这两个键曾经在这里种过种子，
// 但全仓没有任何读取点（主题在前端 localStorage `dbmind_theme`、页大小在 `dbmind_query`），
// 纯属「看起来后端也管了一份」的误导，已删。要加后端设置键时，先想好谁消费它。
/// 全局会话配额：每个宿主进程最多持有多少条物理会话（0 = 不限制）。
const SETTINGS_SESSION_MAX_PER_HOST: &str = "session.maxPerHost";
/// 空闲会话回收时长（秒；0 = 不按空闲回收，只保留「配额满时回收」）。
const SETTINGS_SESSION_IDLE_TIMEOUT: &str = "session.idleTimeoutSecs";
/// 结构缓存的有效期（秒；设置页可改，配合「立即刷新」按钮两条路都能走）。
const SETTINGS_SCHEMA_TTL: &str = "schema.ttlSecs";
/// SQL 编辑器执行的超时（秒；以前在 web 层写死 120 秒，跑大报表与误发大查询都不好受）。
const SETTINGS_QUERY_TIMEOUT: &str = "query.timeoutSecs";
/// 允许旧版 TLS（TLSv1/1.1）：web 层 `sys.rs` 也有一份同名常量（那里拿不到私有模块），
/// 键名字符串必须保持一致 —— 引擎开机与设置变更时靠它同步给宿主 spawn 层。
pub const SETTINGS_ALLOW_LEGACY_TLS: &str = "jdbc.allowLegacyTls";
/// 查询历史保留条数（0 = 不按条数清理）。表此前只增不减，几年下来就是几十万行。
const SETTINGS_HISTORY_MAX_ENTRIES: &str = "history.maxEntries";
/// 查询历史保留天数（0 = 永不按时间清理）。
const SETTINGS_HISTORY_RETENTION_DAYS: &str = "history.retentionDays";
/// SSH 隧道空闲回收（秒；0 = 不按空闲回收）。
const SETTINGS_TUNNEL_IDLE_TIMEOUT: &str = "tunnel.idleTimeoutSecs";
/// 全局默认返回行数上限（编辑器/数据浏览的兜底上限；前端还要再被分页大小夹一层）。
const SETTINGS_QUERY_MAX_ROWS: &str = "query.maxRows";
/// 日志级别（trace/debug/info/warn/error；web 壳在运行时热切换 tracing 过滤器）。
pub const SETTINGS_LOG_LEVEL: &str = "log.level";
/// 审计日志级别（all / write / error / off）—— 设置 → 日志里用户自选的记录范围。
pub const SETTINGS_AUDIT_LEVEL: &str = "audit.level";
/// MCP 壳的默认连接：客户端不传 connection 参数时兜底用它（空 = 不兜底，报缺参）。
pub const SETTINGS_MCP_DEFAULT_CONNECTION: &str = "mcp.defaultConnection";
/// MCP 单次查询最大行数（1..=HARD_MAX_ROWS；壳把客户端传的 max_rows 夹到这个值）。
pub const SETTINGS_MCP_MAX_ROWS: &str = "mcp.maxRows";
/// MCP 查询默认超时（秒；客户端没传 timeout_ms 时用它）。
pub const SETTINGS_MCP_TIMEOUT_SECS: &str = "mcp.timeoutSecs";
/// MCP 工具类别开关：结构浏览（连接/类型/表清单/表结构/搜表）。
pub const SETTINGS_MCP_TOOLS_STRUCTURE: &str = "mcp.toolsStructure";
/// MCP 工具类别开关：SQL 执行（执行/取消）。
pub const SETTINGS_MCP_TOOLS_QUERY: &str = "mcp.toolsQuery";
/// MCP 工具类别开关：历史与诊断（查询历史/服务信息）。
pub const SETTINGS_MCP_TOOLS_HISTORY: &str = "mcp.toolsHistory";

const SELECT_CONNECTION: &str = "SELECT id, name, kind, host, port, database_name, username, password, \
     file_path, color, extra, read_only, created_at, updated_at FROM connections";

pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Store {
    /// 打开（不存在则创建）指定路径的元数据库。
    pub fn open(path: &Path) -> Result<Self> {
        paths::ensure_parent(path)?;
        let conn = Connection::open(path).map_err(|e| {
            DbMindError::new(
                ErrorCode::StorageUnavailable,
                format!("无法打开本地库 {}", path.display()),
            )
            .with_detail(e.to_string())
        })?;
        let store = Self::from_connection(conn, path.to_path_buf())?;
        Ok(store)
    }

    /// 打开默认库（`~/.dbmind/dbmind.db`）。
    pub fn open_default() -> Result<Self> {
        Self::open(&paths::default_store_path())
    }

    /// 内存库：测试与「不落盘」的临时会话用。
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|e| {
            DbMindError::new(ErrorCode::StorageFailed, "无法创建内存元数据库").with_detail(e.to_string())
        })?;
        Self::from_connection(conn, PathBuf::from(":memory:"))
    }

    fn from_connection(conn: Connection, path: PathBuf) -> Result<Self> {
        // WAL 让读取不被写事务阻塞（多壳并发访问同一库时很关键）
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA)?;
        Self::migrate_columns(&conn)?;
        let store = Self {
            conn: Mutex::new(conn),
            path,
        };
        store.seed_settings()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ------------------------------------------------------------ 连接

    pub fn list_connections(&self) -> Result<Vec<ConnectionRecord>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!("{SELECT_CONNECTION} ORDER BY name COLLATE NOCASE"))?;
        let mut out = Vec::new();
        for row in stmt.query_map([], raw_connection)? {
            out.push(row?.into_record()?);
        }
        Ok(out)
    }

    /// 按 id 或名称查找（UI 两种都传，这里统一兜住）。
    pub fn find_connection(&self, id_or_name: &str) -> Result<Option<ConnectionRecord>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!("{SELECT_CONNECTION} WHERE id = ?1 OR name = ?2 LIMIT 1"))?;
        let mut rows = stmt.query(params![id_or_name, id_or_name])?;
        match rows.next()? {
            Some(row) => Ok(Some(raw_connection(row)?.into_record()?)),
            None => Ok(None),
        }
    }

    pub fn require_connection(&self, id_or_name: &str) -> Result<ConnectionRecord> {
        self.find_connection(id_or_name)?
            .ok_or_else(|| DbMindError::new(ErrorCode::ConnNotFound, format!("找不到连接「{id_or_name}」")))
    }

    pub fn insert_connection(&self, config: &ConnectionConfig) -> Result<ConnectionRecord> {
        config.validate()?;
        if self.find_connection(&config.name)?.is_some() {
            return Err(DbMindError::new(
                ErrorCode::ConnInvalid,
                format!("已存在同名连接「{}」", config.name),
            ));
        }
        let id = new_id("conn");
        let now = now_iso();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO connections (id, name, kind, host, port, database_name, username, password, \
             file_path, color, extra, read_only, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, ?12, ?12)",
            params![
                id,
                config.name,
                config.kind.key(),
                config.host,
                config.resolved_port(),
                config.database,
                config.username,
                config.password,
                config.file_path,
                config.color,
                config.extra.as_ref().map(|v| v.to_string()),
                now,
            ],
        )?;
        drop(conn);
        self.require_connection(&id)
    }

    pub fn update_connection(&self, id: &str, config: &ConnectionConfig) -> Result<ConnectionRecord> {
        config.validate()?;
        let existing = self.require_connection(id)?;
        if let Some(other) = self.find_connection(&config.name)? {
            if other.id != existing.id {
                return Err(DbMindError::new(
                    ErrorCode::ConnInvalid,
                    format!("已存在同名连接「{}」", config.name),
                ));
            }
        }
        let conn = self.lock();
        conn.execute(
            "UPDATE connections SET name = ?2, kind = ?3, host = ?4, port = ?5, database_name = ?6, \
             username = ?7, password = ?8, file_path = ?9, color = ?10, extra = ?11, updated_at = ?12 \
             WHERE id = ?1",
            params![
                existing.id,
                config.name,
                config.kind.key(),
                config.host,
                config.resolved_port(),
                config.database,
                config.username,
                config.password,
                config.file_path,
                config.color,
                config.extra.as_ref().map(|v| v.to_string()),
                now_iso(),
            ],
        )?;
        drop(conn);
        self.require_connection(&existing.id)
    }

    pub fn delete_connection(&self, id_or_name: &str) -> Result<bool> {
        let Some(record) = self.find_connection(id_or_name)? else {
            return Ok(false);
        };
        let conn = self.lock();
        let affected = conn.execute("DELETE FROM connections WHERE id = ?1", params![record.id])?;
        conn.execute(
            "DELETE FROM schema_cache WHERE connection_id = ?1",
            params![record.id],
        )?;
        Ok(affected > 0)
    }

    // ------------------------------------------------------------ 结构缓存
    //
    // 为什么需要：结构浏览是**最高频**的元数据操作，而每次都要打数据库
    // （JDBC 的 DatabaseMetaData、Mongo 的抽样、Redis 的 SCAN、ES 的 mapping）。
    // 结构变动远低于浏览频率，缓存收益大。
    //
    // 代价是「同事刚加的表我看不到」—— 所以必须给出**明确的作废规则**与过期时间，
    // 并把「这是缓存」告诉界面（`schema_cache_info`），而不是让它悄悄变旧。

    /// 整个连接的对象清单在缓存里的保留键（单表列的键是 `table:<名字>`）。
    pub const SCHEMA_OBJECTS_KEY: &str = "__objects__";

    /// 单表列的缓存键。
    pub fn schema_columns_key(table: &str) -> String {
        format!("table:{table}")
    }

    /// 读结构缓存；`ttl_secs` 内才算命中。
    pub fn fresh_schema(&self, connection_id: &str, key: &str, ttl_secs: i64) -> Result<Option<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT payload, cached_at FROM schema_cache \
             WHERE connection_id = ?1 AND object_name = ?2",
        )?;
        let row = stmt
            .query_row(params![connection_id, key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .optional()?;
        let Some((payload, cached_at)) = row else {
            return Ok(None);
        };
        Ok(is_fresh(&cached_at, ttl_secs).then_some(payload))
    }

    /// 写结构缓存（覆盖写）。
    pub fn put_schema(&self, connection_id: &str, key: &str, payload: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO schema_cache (connection_id, object_name, payload, cached_at) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(connection_id, object_name) DO UPDATE SET \
             payload = excluded.payload, cached_at = excluded.cached_at",
            params![connection_id, key, payload, now_iso()],
        )?;
        Ok(())
    }

    /// 作废缓存：`key` 为 None 时作废整个连接。
    pub fn invalidate_schema(&self, connection_id: &str, key: Option<&str>) -> Result<usize> {
        let conn = self.lock();
        let affected = match key {
            Some(key) => conn.execute(
                "DELETE FROM schema_cache WHERE connection_id = ?1 AND object_name = ?2",
                params![connection_id, key],
            )?,
            None => conn.execute(
                "DELETE FROM schema_cache WHERE connection_id = ?1",
                params![connection_id],
            )?,
        };
        Ok(affected)
    }

    /// 缓存概览（界面据此显示「缓存于 N 秒前」）。
    pub fn schema_cache_info(&self, connection_id: &str) -> Result<crate::types::SchemaCacheInfo> {
        let conn = self.lock();
        let mut stmt =
            conn.prepare("SELECT COUNT(*), MAX(cached_at) FROM schema_cache WHERE connection_id = ?1")?;
        let (entries, latest): (i64, Option<String>) =
            stmt.query_row(params![connection_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(crate::types::SchemaCacheInfo {
            entries: entries.max(0) as usize,
            age_secs: latest.as_deref().and_then(age_secs),
            latest_cached_at: latest,
        })
    }

    pub fn set_read_only(&self, id_or_name: &str, read_only: bool) -> Result<ConnectionRecord> {
        let record = self.require_connection(id_or_name)?;
        let conn = self.lock();
        conn.execute(
            "UPDATE connections SET read_only = ?2, updated_at = ?3 WHERE id = ?1",
            params![record.id, read_only, now_iso()],
        )?;
        drop(conn);
        self.require_connection(&record.id)
    }

    // ------------------------------------------------------------ 历史

    pub fn record_history(&self, entry: &NewHistoryEntry) -> Result<String> {
        let id = new_id("hist");
        let conn = self.lock();
        conn.execute(
            "INSERT INTO query_history (id, connection_id, connection_name, sql, status, row_count, \
             duration_ms, error_code, created_at, kind, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                entry.connection_id,
                entry.connection_name,
                entry.sql,
                entry.status.as_str(),
                entry.row_count as i64,
                entry.duration_ms as i64,
                entry.error_code,
                now_iso(),
                entry.kind,
                entry.source,
            ],
        )?;
        Ok(id)
    }

    pub fn list_history(&self, limit: usize, connection_id: Option<&str>) -> Result<Vec<HistoryEntry>> {
        let conn = self.lock();
        let limit = limit.clamp(1, 1000) as i64;
        let mut out = Vec::new();
        match connection_id {
            Some(cid) => {
                let mut stmt = conn.prepare(
                    "SELECT id, connection_id, connection_name, sql, status, row_count, duration_ms, \
                     error_code, created_at FROM query_history WHERE connection_id = ?1 \
                     ORDER BY created_at DESC, rowid DESC LIMIT ?2",
                )?;
                for row in stmt.query_map(params![cid, limit], raw_history)? {
                    out.push(row?);
                }
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id, connection_id, connection_name, sql, status, row_count, duration_ms, \
                     error_code, created_at FROM query_history \
                     ORDER BY created_at DESC, rowid DESC LIMIT ?1",
                )?;
                for row in stmt.query_map(params![limit], raw_history)? {
                    out.push(row?);
                }
            }
        }
        Ok(out)
    }

    pub fn clear_history(&self) -> Result<usize> {
        let conn = self.lock();
        Ok(conn.execute("DELETE FROM query_history", [])?)
    }

    /// 轻量列迁移：老库补 kind / source 两列，并按 SQL 首词回填 kind。
    /// 声明式建表管不了已存在的表 —— 这是仓库里第一个「改列」式迁移，保持极简：
    /// PRAGMA table_info 探测缺列才 ALTER，重复打开库零开销。
    fn migrate_columns(conn: &Connection) -> Result<()> {
        let has_col = |name: &str| -> Result<bool> {
            let mut stmt = conn.prepare("PRAGMA table_info(query_history)")?;
            let mut hit = false;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                if row.get::<_, String>(1)? == name {
                    hit = true;
                    break;
                }
            }
            Ok(hit)
        };
        if !has_col("kind")? {
            conn.execute_batch(
                "ALTER TABLE query_history ADD COLUMN kind TEXT NOT NULL DEFAULT 'query';
                 UPDATE query_history SET kind = CASE
                   WHEN upper(ltrim(sql)) LIKE 'SELECT %' OR upper(ltrim(sql)) LIKE 'WITH %'
                     OR upper(ltrim(sql)) LIKE 'SHOW %' OR upper(ltrim(sql)) LIKE 'EXPLAIN %' THEN 'query'
                   WHEN upper(ltrim(sql)) LIKE 'INSERT %' OR upper(ltrim(sql)) LIKE 'UPDATE %'
                     OR upper(ltrim(sql)) LIKE 'DELETE %' OR upper(ltrim(sql)) LIKE 'MERGE %'
                     OR upper(ltrim(sql)) LIKE 'REPLACE %' THEN 'write'
                   WHEN upper(ltrim(sql)) LIKE 'CREATE %' OR upper(ltrim(sql)) LIKE 'ALTER %'
                     OR upper(ltrim(sql)) LIKE 'DROP %' OR upper(ltrim(sql)) LIKE 'TRUNCATE %' THEN 'ddl'
                   WHEN upper(ltrim(sql)) LIKE 'BEGIN %' OR upper(ltrim(sql)) LIKE 'COMMIT %'
                     OR upper(ltrim(sql)) LIKE 'ROLLBACK %' THEN 'tx'
                   ELSE 'exec' END;",
            )?;
        }
        if !has_col("source")? {
            conn.execute_batch("ALTER TABLE query_history ADD COLUMN source TEXT NOT NULL DEFAULT 'ui';")?;
        }
        Ok(())
    }

    /// 统一日志：合并 query_history 与 ai_audit 两个来源，按时间倒序。
    /// kind 筛选（空 = 全部）、q 关键字（LIKE 语句/提示词与连接名）。
    pub fn list_logs(
        &self,
        limit: usize,
        kind: Option<&str>,
        q: Option<&str>,
    ) -> Result<Vec<LogEntry>> {
        let conn = self.lock();
        let limit = limit.clamp(1, 2000) as i64;
        let mut out: Vec<LogEntry> = Vec::new();

        let want_exec = matches!(kind, None | Some("exec"))
            || matches!(kind, Some("query" | "write" | "ddl" | "tx"));
        if want_exec {
            let like = format!("%{}%", q.unwrap_or("").replace('%', ""));
            let mut stmt = conn.prepare(
                "SELECT sql, COALESCE(connection_name, connection_id, ''), status, row_count, \
                 duration_ms, error_code, created_at, kind FROM query_history \
                 WHERE (?1 = '' OR kind = ?1) AND (?2 = '' OR sql LIKE ?2 OR connection_name LIKE ?2) \
                 ORDER BY created_at DESC, rowid DESC LIMIT ?3",
            )?;
            let kind_arg = kind.unwrap_or("");
            let mut rows = stmt.query(params![kind_arg, like, limit])?;
            while let Some(row) = rows.next()? {
                out.push(LogEntry {
                    kind: row.get(7)?,
                    sql: row.get(0)?,
                    connection: row.get(1)?,
                    status: row.get(2)?,
                    row_count: row.get::<_, i64>(3)?.max(0) as u64,
                    duration_ms: row.get::<_, i64>(4)?.max(0) as u64,
                    error_code: row.get(5)?,
                    created_at: row.get(6)?,
                });
            }
        }

        // AI 调用（ai_audit：time/kind/prompt）—— 按级别与筛选参与合并
        let want_ai = matches!(kind, None | Some("ai"));
        if want_ai {
            let like = format!("%{}%", q.unwrap_or("").replace('%', ""));
            let mut stmt = conn.prepare(
                "SELECT prompt, kind, time FROM ai_audit \
                 WHERE (?1 = '' OR ?1 = 'ai') AND prompt LIKE ?2 \
                 ORDER BY time DESC LIMIT ?3",
            )?;
            let kind_arg = kind.unwrap_or("");
            let mut rows = stmt.query(params![kind_arg, like, limit])?;
            while let Some(row) = rows.next()? {
                out.push(LogEntry {
                    kind: "ai".into(),
                    sql: row.get(0)?,
                    connection: "AI".into(),
                    status: "ok".into(),
                    row_count: 0,
                    duration_ms: 0,
                    error_code: None,
                    created_at: row.get(2)?,
                });
            }
        }

        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out.truncate(limit as usize);
        Ok(out)
    }

    /// 清空全部日志（执行历史 + AI 审计），返回删除总行数。
    pub fn clear_logs(&self) -> Result<usize> {
        let conn = self.lock();
        let n1 = conn.execute("DELETE FROM query_history", [])?;
        let n2 = conn.execute("DELETE FROM ai_audit", [])?;
        Ok(n1 + n2)
    }

    /// 表行数（白名单；表不存在按 0 —— 老库可能还没建这张表）。
    pub fn table_row_count(&self, table: &str) -> Result<u64> {
        const ALLOWED: &[&str] = &[
            "schema_cache", "query_history", "ai_audit", "ai_usage_days",
            "ai_usage_models", "kb_docs", "kb_vectors",
        ];
        if !ALLOWED.contains(&table) {
            return Err(DbMindError::new(
                ErrorCode::StorageFailed,
                format!("未允许的统计表: {table}"),
            ));
        }
        let conn = self.lock();
        let exists: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![table],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists == 0 {
            return Ok(0);
        }
        // 表名来自上面的白名单字面量，不是用户输入 —— 拼接是安全的
        Ok(conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)
    }

    /// 清空白名单表，返回删除的行数。
    pub fn purge_table(&self, table: &str) -> Result<usize> {
        const ALLOWED: &[&str] = &["ai_audit", "ai_usage_days", "ai_usage_models", "kb_docs", "kb_vectors"];
        if !ALLOWED.contains(&table) {
            return Err(DbMindError::new(
                ErrorCode::StorageFailed,
                format!("未允许的清理表: {table}"),
            ));
        }
        let conn = self.lock();
        // 同上：表名是白名单字面量
        Ok(conn.execute(&format!("DELETE FROM {table}"), [])?)
    }

    /// 按设置清理历史：`max_entries` 之外的旧行 + `retention_days` 天之前的旧行
    /// （0 = 该维度不启用）。返回删除的行数。
    ///
    /// 为什么要有上限：`query_history` 此前**只增不减** —— 每次编辑器执行都插一行，
    /// 一年下来几十万行，拖慢的正是它自己的索引。按条数保「最近 N 条」、按天数保
    /// 「最近 N 天」，两个维度都留了 0 = 不启用的口子（有人就想永久留着）。
    pub fn prune_history(&self, max_entries: usize, retention_days: u32) -> Result<usize> {
        let conn = self.lock();
        let mut removed = 0i64;
        if retention_days > 0 {
            // created_at 全部出自 `now_iso()`（同一格式、同一时区偏移），字符串比较即时间比较
            let cutoff = (chrono::Local::now() - chrono::Duration::days(retention_days as i64))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            removed += conn.execute(
                "DELETE FROM query_history WHERE created_at < ?1",
                params![cutoff],
            )? as i64;
        }
        if max_entries > 0 {
            removed += conn.execute(
                "DELETE FROM query_history WHERE id NOT IN \
                 (SELECT id FROM query_history ORDER BY created_at DESC, rowid DESC LIMIT ?1)",
                params![max_entries as i64],
            )? as i64;
        }
        Ok(removed as usize)
    }

    /// 把元数据库做一份**一致性快照**到 `path`（`VACUUM INTO`，迁移数据目录用）。
    ///
    /// 为什么不用直接复制文件：库正开着（WAL 模式），裸拷 `dbmind.db` 可能拿到
    /// 一个没有 wal 的中间态。`VACUUM INTO` 在读事务里产出一个干净的独立文件。
    /// 目标已存在则失败 —— 调用方（迁移）负责先清场。
    pub fn backup_into(&self, path: &Path) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "VACUUM INTO ?1",
            params![path.to_string_lossy()],
        )
        .map_err(|e| {
            DbMindError::new(
                ErrorCode::StorageFailed,
                format!("快照元数据库到 {} 失败: {e}", path.display()),
            )
        })?;
        Ok(())
    }

    // ------------------------------------------------------------ 设置

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.lock();
        let value = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(value)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, now_iso()],
        )?;
        Ok(())
    }

    pub fn get_bool_setting(&self, key: &str, default: bool) -> Result<bool> {
        Ok(self
            .get_setting(key)?
            .map(|v| v == "true" || v == "1")
            .unwrap_or(default))
    }

    /// 读一个数值设置。解析不出来时用默认值而不是报错：设置表是用户/旧版本写的，
    /// 一个坏值不该让引擎起不来（界面上还能改回去）。
    pub fn get_usize_setting(&self, key: &str, default: usize) -> Result<usize> {
        Ok(self
            .get_setting(key)?
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(default))
    }

    pub fn all_settings(&self) -> Result<Vec<(String, String)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT key, value FROM app_settings ORDER BY key")?;
        let mut out = Vec::new();
        for row in stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))? {
            out.push(row?);
        }
        Ok(out)
    }

    /// 安全策略相关设置的键名（供壳层读写，避免各处硬编码字符串）。
    pub const KEY_PROTECT_PRODUCTION: &'static str = SETTINGS_SAFETY_PRODUCTION;
    pub const KEY_AI_WRITE_ENABLED: &'static str = SETTINGS_SAFETY_AI_WRITE;
    pub const KEY_BLOCK_DANGEROUS: &'static str = SETTINGS_SAFETY_BLOCK_DANGEROUS;
    pub const KEY_MAX_WRITE_ROWS: &'static str = SETTINGS_SAFETY_MAX_WRITE_ROWS;
    pub const KEY_SESSION_MAX_PER_HOST: &'static str = SETTINGS_SESSION_MAX_PER_HOST;
    pub const KEY_SESSION_IDLE_TIMEOUT: &'static str = SETTINGS_SESSION_IDLE_TIMEOUT;
    pub const KEY_SCHEMA_TTL: &'static str = SETTINGS_SCHEMA_TTL;
    pub const KEY_QUERY_TIMEOUT: &'static str = SETTINGS_QUERY_TIMEOUT;
    pub const KEY_ALLOW_LEGACY_TLS: &'static str = SETTINGS_ALLOW_LEGACY_TLS;
    pub const KEY_HISTORY_MAX_ENTRIES: &'static str = SETTINGS_HISTORY_MAX_ENTRIES;
    pub const KEY_HISTORY_RETENTION_DAYS: &'static str = SETTINGS_HISTORY_RETENTION_DAYS;
    pub const KEY_TUNNEL_IDLE_TIMEOUT: &'static str = SETTINGS_TUNNEL_IDLE_TIMEOUT;
    pub const KEY_QUERY_MAX_ROWS: &'static str = SETTINGS_QUERY_MAX_ROWS;
    pub const KEY_LOG_LEVEL: &'static str = SETTINGS_LOG_LEVEL;
    pub const KEY_AUDIT_LEVEL: &'static str = SETTINGS_AUDIT_LEVEL;
    pub const KEY_MCP_DEFAULT_CONNECTION: &'static str = SETTINGS_MCP_DEFAULT_CONNECTION;
    pub const KEY_MCP_MAX_ROWS: &'static str = SETTINGS_MCP_MAX_ROWS;
    pub const KEY_MCP_TIMEOUT_SECS: &'static str = SETTINGS_MCP_TIMEOUT_SECS;
    pub const KEY_MCP_TOOLS_STRUCTURE: &'static str = SETTINGS_MCP_TOOLS_STRUCTURE;
    pub const KEY_MCP_TOOLS_QUERY: &'static str = SETTINGS_MCP_TOOLS_QUERY;
    pub const KEY_MCP_TOOLS_HISTORY: &'static str = SETTINGS_MCP_TOOLS_HISTORY;

    fn seed_settings(&self) -> Result<()> {
        let conn = self.lock();
        for (key, value) in [
            (SETTINGS_SAFETY_PRODUCTION, "false"),
            (SETTINGS_SAFETY_AI_WRITE, "false"),
            (SETTINGS_SCHEMA_TTL, "300"),
            (SETTINGS_QUERY_TIMEOUT, "120"),
            (SETTINGS_HISTORY_MAX_ENTRIES, "1000"),
            (SETTINGS_HISTORY_RETENTION_DAYS, "30"),
            (SETTINGS_TUNNEL_IDLE_TIMEOUT, "1800"),
            (SETTINGS_QUERY_MAX_ROWS, "2000"),
            (SETTINGS_LOG_LEVEL, "info"),
            (SETTINGS_AUDIT_LEVEL, "all"),
            (SETTINGS_MCP_MAX_ROWS, "2000"),
            (SETTINGS_MCP_TIMEOUT_SECS, "30"),
            (SETTINGS_MCP_TOOLS_STRUCTURE, "true"),
            (SETTINGS_MCP_TOOLS_QUERY, "true"),
            (SETTINGS_MCP_TOOLS_HISTORY, "true"),
        ] {
            conn.execute(
                "INSERT OR IGNORE INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                params![key, value, now_iso()],
            )?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- 行映射

struct RawConnection {
    id: String,
    name: String,
    kind: String,
    host: Option<String>,
    port: Option<i64>,
    database_name: Option<String>,
    username: Option<String>,
    password: Option<String>,
    file_path: Option<String>,
    color: Option<String>,
    extra: Option<String>,
    read_only: bool,
    created_at: String,
    updated_at: String,
}

impl RawConnection {
    fn into_record(self) -> Result<ConnectionRecord> {
        let kind = ConnectionKind::from_key(&self.kind).ok_or_else(|| {
            DbMindError::new(
                ErrorCode::ConnTypeUnknown,
                format!(
                    "存储中的连接类型「{}」未在 plugins/connection-types 中登记",
                    self.kind
                ),
            )
        })?;
        let extra = self
            .extra
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .and_then(|s| serde_json::from_str(s).ok());
        Ok(ConnectionRecord {
            id: self.id,
            config: ConnectionConfig {
                name: self.name,
                kind,
                host: self.host,
                port: self.port.and_then(|p| u16::try_from(p).ok()),
                database: self.database_name,
                username: self.username,
                password: self.password,
                file_path: self.file_path,
                color: self.color,
                extra,
            },
            read_only: self.read_only,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn raw_connection(row: &Row<'_>) -> rusqlite::Result<RawConnection> {
    Ok(RawConnection {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        host: row.get(3)?,
        port: row.get(4)?,
        database_name: row.get(5)?,
        username: row.get(6)?,
        password: row.get(7)?,
        file_path: row.get(8)?,
        color: row.get(9)?,
        extra: row.get(10)?,
        read_only: row.get::<_, i64>(11)? != 0,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

fn raw_history(row: &Row<'_>) -> rusqlite::Result<HistoryEntry> {
    let status: String = row.get(4)?;
    Ok(HistoryEntry {
        id: row.get(0)?,
        connection_id: row.get(1)?,
        connection_name: row.get(2)?,
        sql: row.get(3)?,
        status: HistoryStatus::parse(&status),
        row_count: row.get::<_, i64>(5)?.max(0) as usize,
        duration_ms: row.get::<_, i64>(6)?.max(0) as u64,
        error_code: row.get(7)?,
        created_at: row.get(8)?,
    })
}

// ---------------------------------------------------------------- AI 状态（设置 / 用量 / 审计）

/// 「ai_models」的一行：一个可用的模型端点。
#[derive(Debug, Clone, Default)]
pub struct AiModelRow {
    /// 界面上给这条模型起的标识（原来存在 json 里的 `id`）。
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u64,
    pub embedding: bool,
}

/// 「ai_settings」单行 + 它带的一组模型。
#[derive(Debug, Clone, Default)]
pub struct AiSettingsRow {
    pub enabled: bool,
    pub privacy_mode: String,
    pub audit_enabled: bool,
    pub models: Vec<AiModelRow>,
}

/// 用量账本的一格：某个模型在某一天的数字。
#[derive(Debug, Clone, Default)]
pub struct AiUsageRow {
    pub model_key: String,
    pub day: String,
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl Store {
    /// 读 AI 设置。**返回 None 表示库里从来没有过**（不是"配过但为空"）——
    /// 调用方靠这个区分「要不要从旧 json 导入」和「给什么默认值」。
    pub fn ai_settings(&self) -> Result<Option<AiSettingsRow>> {
        let conn = self.lock();
        let head = conn
            .query_row(
                "SELECT enabled, privacy_mode, audit_enabled FROM ai_settings WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? != 0,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? != 0,
                    ))
                },
            )
            .optional()?;
        let Some((enabled, privacy_mode, audit_enabled)) = head else {
            return Ok(None);
        };
        let models = {
            let mut stmt = conn.prepare(
                "SELECT id, name, base_url, api_key, model, max_tokens, embedding \
                 FROM ai_models ORDER BY ordinal",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(AiModelRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    base_url: row.get(2)?,
                    api_key: row.get(3)?,
                    model: row.get(4)?,
                    max_tokens: row.get::<_, i64>(5)?.max(0) as u64,
                    embedding: row.get::<_, i64>(6)? != 0,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(Some(AiSettingsRow {
            enabled,
            privacy_mode,
            audit_enabled,
            models,
        }))
    }

    /// 整体替换 AI 设置（单行 + 模型表），一个事务里做完：
    /// 半截保存比不保存更糟 —— 界面会看到"一半新一半旧"的配置。
    pub fn save_ai_settings(&self, settings: &AiSettingsRow) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let now = now_iso();
        tx.execute(
            "INSERT INTO ai_settings (id, enabled, privacy_mode, audit_enabled, updated_at) \
             VALUES (1, ?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET enabled = ?1, privacy_mode = ?2, \
                audit_enabled = ?3, updated_at = ?4",
            params![
                settings.enabled as i64,
                settings.privacy_mode,
                settings.audit_enabled as i64,
                now
            ],
        )?;
        tx.execute("DELETE FROM ai_models", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO ai_models (ordinal, id, name, base_url, api_key, model, max_tokens, embedding) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for (index, model) in settings.models.iter().enumerate() {
                stmt.execute(params![
                    index as i64,
                    model.id,
                    model.name,
                    model.base_url,
                    model.api_key,
                    model.model,
                    model.max_tokens as i64,
                    model.embedding as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 记一次调用：次数必加；token 只在真的拿到了用量时加（没回 usage 就一分不补）。
    pub fn bump_ai_usage(
        &self,
        model_key: &str,
        day: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO ai_usage_days (day, calls) VALUES (?1, 1) \
             ON CONFLICT(day) DO UPDATE SET calls = calls + 1",
            params![day],
        )?;
        conn.execute(
            "INSERT INTO ai_usage_models \
                (model_key, day, calls, prompt_tokens, completion_tokens, total_tokens) \
             VALUES (?1, ?2, 1, ?3, ?4, ?5) \
             ON CONFLICT(model_key, day) DO UPDATE SET \
                calls = calls + 1, \
                prompt_tokens = prompt_tokens + ?3, \
                completion_tokens = completion_tokens + ?4, \
                total_tokens = total_tokens + ?5",
            params![
                model_key,
                day,
                prompt_tokens as i64,
                completion_tokens as i64,
                total_tokens as i64
            ],
        )?;
        Ok(())
    }

    /// 按天取用量（`from_day` 含当天，UTC 无关：日期串就是本地日期）。
    pub fn ai_usage_days(&self, from_day: &str) -> Result<Vec<(String, u64)>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT day, calls FROM ai_usage_days WHERE day >= ?1 ORDER BY day")?;
        let rows = stmt.query_map(params![from_day], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?.max(0) as u64))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 同一时间窗内按模型汇总（总 token 降序、并列按次数降序）。
    ///
    /// 这些数字以前要在内存里把「模型 → 日期 → 计数」两层对象全遍历一遍才算得出来；
    /// 进了库一条 GROUP BY 就够了，也就是把文件搬进库最实在的那点收益。
    pub fn ai_usage_models(&self, from_day: &str) -> Result<Vec<AiUsageRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT model_key, SUM(calls), SUM(prompt_tokens), SUM(completion_tokens), \
                    SUM(total_tokens) \
             FROM ai_usage_models WHERE day >= ?1 GROUP BY model_key \
             ORDER BY SUM(total_tokens) DESC, SUM(calls) DESC",
        )?;
        let rows = stmt.query_map(params![from_day], |row| {
            Ok(AiUsageRow {
                model_key: row.get(0)?,
                day: String::new(),
                calls: row.get::<_, i64>(1)?.max(0) as u64,
                prompt_tokens: row.get::<_, i64>(2)?.max(0) as u64,
                completion_tokens: row.get::<_, i64>(3)?.max(0) as u64,
                total_tokens: row.get::<_, i64>(4)?.max(0) as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 导入旧账本（一次性迁移用）：整天与整模型的数字直接落下来，不做累加。
    pub fn import_ai_usage(&self, days: &[(String, u64)], models: &[AiUsageRow]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        {
            let mut day_stmt = tx.prepare(
                "INSERT INTO ai_usage_days (day, calls) VALUES (?1, ?2) \
                 ON CONFLICT(day) DO UPDATE SET calls = ?2",
            )?;
            for (day, calls) in days {
                day_stmt.execute(params![day, *calls as i64])?;
            }
        }
        {
            let mut model_stmt = tx.prepare(
                "INSERT INTO ai_usage_models \
                    (model_key, day, calls, prompt_tokens, completion_tokens, total_tokens) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(model_key, day) DO UPDATE SET calls = ?3, prompt_tokens = ?4, \
                    completion_tokens = ?5, total_tokens = ?6",
            )?;
            for row in models {
                model_stmt.execute(params![
                    row.model_key,
                    row.day,
                    row.calls as i64,
                    row.prompt_tokens as i64,
                    row.completion_tokens as i64,
                    row.total_tokens as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 追加一条 AI 审计记录。
    pub fn insert_ai_audit(&self, kind: &str, prompt: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO ai_audit (time, kind, prompt) VALUES (?1, ?2, ?3)",
            params![now_iso(), kind, prompt],
        )?;
        Ok(())
    }

    /// 审计记录条数（导入迁移判断与诊断用）。
    pub fn ai_audit_count(&self) -> Result<u64> {
        let conn = self.lock();
        let count = conn.query_row("SELECT COUNT(*) FROM ai_audit", [], |row| {
            row.get::<_, i64>(0)
        })?;
        Ok(count.max(0) as u64)
    }
}

/// 「ai_glossary」的一行。
#[derive(Debug, Clone, Default)]
pub struct AiGlossaryRow {
    pub id: String,
    pub term: String,
    pub definition: String,
    pub mapping: String,
    pub connection_id: String,
}

/// 「ai_examples」的一行。`hits` 是召回次数（越常用的问法越该优先命中）。
#[derive(Debug, Clone, Default)]
pub struct AiExampleRow {
    pub id: String,
    pub question: String,
    pub sql: String,
    pub connection_id: String,
    pub database_name: String,
    pub hits: u64,
    pub ts: String,
}

/// 「ai_quality_rules」的一行：某张表的整套规则。
#[derive(Debug, Clone, Default)]
pub struct AiQualityRuleRow {
    pub connection_id: String,
    pub database_name: String,
    pub table_name: String,
    /// 规则数组的 JSON（规则体由各检查器自己定义）。
    pub rules: String,
    pub saved_at: String,
}

impl Store {
    /// 术语表（按 term 排序，界面与提示词的顺序都稳定）。
    pub fn ai_glossary(&self) -> Result<Vec<AiGlossaryRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, term, definition, mapping, connection_id FROM ai_glossary ORDER BY term",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AiGlossaryRow {
                id: row.get(0)?,
                term: row.get(1)?,
                definition: row.get(2)?,
                mapping: row.get(3)?,
                connection_id: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 采纳示例（召回次数降序，并列按时间新的在前）。
    pub fn ai_examples(&self) -> Result<Vec<AiExampleRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, question, sql, connection_id, database_name, hits, created_at \
             FROM ai_examples ORDER BY hits DESC, created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AiExampleRow {
                id: row.get(0)?,
                question: row.get(1)?,
                sql: row.get(2)?,
                connection_id: row.get(3)?,
                database_name: row.get(4)?,
                hits: row.get::<_, i64>(5)?.max(0) as u64,
                ts: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 整体替换团队知识。业务层是「读出来 → 改数组 → 写回去」的用法，
    /// 所以这里就按整体替换来做：一个事务，两张表一起换，不会出现"术语换了、示例没换"。
    pub fn replace_ai_knowledge(
        &self,
        glossary: &[AiGlossaryRow],
        examples: &[AiExampleRow],
    ) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM ai_glossary", [])?;
        tx.execute("DELETE FROM ai_examples", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO ai_glossary (id, term, definition, mapping, connection_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for row in glossary {
                stmt.execute(params![
                    row.id,
                    row.term,
                    row.definition,
                    row.mapping,
                    row.connection_id
                ])?;
            }
        }
        {
            let mut stmt = tx.prepare(
                "INSERT INTO ai_examples \
                    (id, question, sql, connection_id, database_name, hits, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for row in examples {
                stmt.execute(params![
                    row.id,
                    row.question,
                    row.sql,
                    row.connection_id,
                    row.database_name,
                    row.hits as i64,
                    row.ts
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 全部质量规则（按连接/库/表排序）。
    pub fn ai_quality_rules(&self) -> Result<Vec<AiQualityRuleRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT connection_id, database_name, table_name, rules, saved_at \
             FROM ai_quality_rules ORDER BY connection_id, database_name, table_name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AiQualityRuleRow {
                connection_id: row.get(0)?,
                database_name: row.get(1)?,
                table_name: row.get(2)?,
                rules: row.get(3)?,
                saved_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 整体替换质量规则（同样是"读出来 → 改 → 写回去"的用法，删除也走这里）。
    pub fn replace_ai_quality_rules(&self, rows: &[AiQualityRuleRow]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM ai_quality_rules", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO ai_quality_rules \
                    (connection_id, database_name, table_name, rules, saved_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for row in rows {
                stmt.execute(params![
                    row.connection_id,
                    row.database_name,
                    row.table_name,
                    row.rules,
                    row.saved_at
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

/// 「kb_list」的一行（知识库的元数据与统计）。
#[derive(Debug, Clone, Default)]
pub struct KbInfoRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
    pub doc_count: u64,
    pub chunk_count: u64,
    pub char_count: u64,
}

/// 「kb_docs」的一行。`parents` / `chunks` 是 JSON 文本（整体读写）。
#[derive(Debug, Clone, Default)]
pub struct KbDocRow {
    pub id: String,
    pub title: String,
    pub source: String,
    pub created_at: String,
    pub char_count: u64,
    pub raw: String,
    pub parents: String,
    pub chunks: String,
}

/// 「kb_vectors」的一行：一份文档的全部块向量。
#[derive(Debug, Clone, Default)]
pub struct KbVectorRow {
    pub model: String,
    pub dim: usize,
    pub vectors: Vec<Vec<f32>>,
}

/// 把 `Vec<Vec<f32>>` 打包成小端二进制（前端/接口看不到它，只是存储形态）。
fn pack_vectors(vectors: &[Vec<f32>]) -> Vec<u8> {
    let total: usize = vectors.iter().map(|row| row.len()).sum();
    let mut bytes = Vec::with_capacity(total * 4);
    for row in vectors {
        for value in row {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

/// 按维度把二进制还原成 `Vec<Vec<f32>>`；dim 为 0 或长度不整时返回空。
fn unpack_vectors(bytes: &[u8], dim: usize) -> Vec<Vec<f32>> {
    if dim == 0 || bytes.len() % (dim * 4) != 0 {
        return Vec::new();
    }
    let mut out: Vec<Vec<f32>> = Vec::with_capacity(bytes.len() / (dim * 4));
    for chunk in bytes.chunks(dim * 4) {
        let mut row = Vec::with_capacity(dim);
        for i in 0..dim {
            let start = i * 4;
            let mut buf = [0u8; 4];
            buf.copy_from_slice(&chunk[start..start + 4]);
            row.push(f32::from_le_bytes(buf));
        }
        out.push(row);
    }
    out
}

impl Store {
    /// 全部知识库（按创建时间排序，与目录版的顺序一致）。
    pub fn kb_infos(&self) -> Result<Vec<KbInfoRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, created_at, updated_at, doc_count, chunk_count, \
                    char_count FROM kb_list ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(KbInfoRow {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                doc_count: row.get::<_, i64>(5)?.max(0) as u64,
                chunk_count: row.get::<_, i64>(6)?.max(0) as u64,
                char_count: row.get::<_, i64>(7)?.max(0) as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 整体替换知识库列表（上层是"读出来 → 改 → 写回去"，一个事务保证不会半截）。
    pub fn kb_put_infos(&self, rows: &[KbInfoRow]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM kb_list", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO kb_list \
                    (id, name, description, created_at, updated_at, doc_count, chunk_count, char_count) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for row in rows {
                stmt.execute(params![
                    row.id,
                    row.name,
                    row.description,
                    row.created_at,
                    row.updated_at,
                    row.doc_count as i64,
                    row.chunk_count as i64,
                    row.char_count as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 知识库配置（JSON 文本）；`kb_id` 空串 = 全局默认。
    pub fn kb_get_config(&self, kb_id: &str) -> Result<Option<String>> {
        let conn = self.lock();
        let payload = conn
            .query_row(
                "SELECT payload FROM kb_config WHERE kb_id = ?1",
                params![kb_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(payload)
    }

    pub fn kb_put_config(&self, kb_id: &str, payload: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO kb_config (kb_id, payload, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(kb_id) DO UPDATE SET payload = ?2, updated_at = ?3",
            params![kb_id, payload, now_iso()],
        )?;
        Ok(())
    }

    /// 某个库的全部文档（按创建时间排序）。
    pub fn kb_docs(&self, kb_id: &str) -> Result<Vec<KbDocRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT doc_id, title, source, created_at, char_count, raw, parents, chunks \
             FROM kb_docs WHERE kb_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![kb_id], |row| {
            Ok(KbDocRow {
                id: row.get(0)?,
                title: row.get(1)?,
                source: row.get(2)?,
                created_at: row.get(3)?,
                char_count: row.get::<_, i64>(4)?.max(0) as u64,
                raw: row.get(5)?,
                parents: row.get(6)?,
                chunks: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 单篇文档；不存在返回 None（调用方据此给出"文档不存在"）。
    pub fn kb_doc(&self, kb_id: &str, doc_id: &str) -> Result<Option<KbDocRow>> {
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT doc_id, title, source, created_at, char_count, raw, parents, chunks \
                 FROM kb_docs WHERE kb_id = ?1 AND doc_id = ?2",
                params![kb_id, doc_id],
                |row| {
                    Ok(KbDocRow {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        source: row.get(2)?,
                        created_at: row.get(3)?,
                        char_count: row.get::<_, i64>(4)?.max(0) as u64,
                        raw: row.get(5)?,
                        parents: row.get(6)?,
                        chunks: row.get(7)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// 写入/覆盖一篇文档。
    pub fn kb_put_doc(&self, kb_id: &str, row: &KbDocRow) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO kb_docs \
                (kb_id, doc_id, title, source, created_at, char_count, raw, parents, chunks) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(kb_id, doc_id) DO UPDATE SET title = ?3, source = ?4, \
                created_at = ?5, char_count = ?6, raw = ?7, parents = ?8, chunks = ?9",
            params![
                kb_id,
                row.id,
                row.title,
                row.source,
                row.created_at,
                row.char_count as i64,
                row.raw,
                row.parents,
                row.chunks
            ],
        )?;
        Ok(())
    }

    /// 删一篇文档（连带它的向量）。
    pub fn kb_delete_doc(&self, kb_id: &str, doc_id: &str) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM kb_docs WHERE kb_id = ?1 AND doc_id = ?2",
            params![kb_id, doc_id],
        )?;
        tx.execute(
            "DELETE FROM kb_vectors WHERE kb_id = ?1 AND doc_id = ?2",
            params![kb_id, doc_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 删一个库：它的配置、文档、向量一起清掉（对应目录版的 remove_dir_all）。
    pub fn kb_delete(&self, kb_id: &str) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for table in ["kb_config", "kb_docs", "kb_vectors"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE kb_id = ?1"),
                params![kb_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 某篇文档的向量；没有记录返回 None（界面据此显示"需要向量化"）。
    pub fn kb_vectors(&self, kb_id: &str, doc_id: &str) -> Result<Option<KbVectorRow>> {
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT model, dim, payload FROM kb_vectors WHERE kb_id = ?1 AND doc_id = ?2",
                params![kb_id, doc_id],
                |row| {
                    let dim = row.get::<_, i64>(1)?.max(0) as usize;
                    let payload = row.get::<_, Vec<u8>>(2)?;
                    Ok((row.get::<_, String>(0)?, dim, payload))
                },
            )
            .optional()?;
        Ok(row.map(|(model, dim, payload)| KbVectorRow {
            model,
            dim,
            vectors: unpack_vectors(&payload, dim),
        }))
    }

    /// 写入/覆盖某篇文档的向量。维度取第一块的长度（各块维度本来就该一致）。
    pub fn kb_put_vectors(&self, kb_id: &str, doc_id: &str, model: &str, vectors: &[Vec<f32>]) -> Result<()> {
        let dim = vectors.first().map(|row| row.len()).unwrap_or(0);
        let payload = pack_vectors(vectors);
        let conn = self.lock();
        conn.execute(
            "INSERT INTO kb_vectors (kb_id, doc_id, model, dim, payload) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(kb_id, doc_id) DO UPDATE SET model = ?3, dim = ?4, payload = ?5",
            params![kb_id, doc_id, model, dim as i64, payload],
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------- 全局句柄

/// 进程内唯一的主库句柄。
///
/// 为什么要有它：AI 设置这类读写是"环境式"的 —— 对话管线深处要用它，
/// 手上却只有一堆与存储无关的参数。把 Store 一路透传下去要改几十处签名，
/// 而那些函数跟存储毫无关系。装一次全局句柄，签名一个都不用动。
///
/// 注意：**刻意不在 Store 构造里自动装**。测试会造大量内存库，自动装会互相串；
/// 装不装由壳层决定（`serve()` 启动时装一次；数据目录迁移热切换时**换新**）。
static GLOBAL_STORE: OnceLock<RwLock<Arc<Store>>> = OnceLock::new();

/// 装全局主库句柄（重复调用=**换新**：迁移热切换后，AI 设置这类「环境式」读写
/// 要跟着指向新库，不能还盯着的旧库）。
pub fn install_global_store(store: Arc<Store>) {
    let cell = GLOBAL_STORE.get_or_init(|| RwLock::new(store.clone()));
    *cell.write().unwrap_or_else(|e| e.into_inner()) = store;
}

/// 取全局主库句柄；没装（单测、或只用内核不开壳）返回 None。
pub fn global_store() -> Option<Arc<Store>> {
    GLOBAL_STORE
        .get()
        .map(|cell| cell.read().unwrap_or_else(|e| e.into_inner()).clone())
}

// ---------------------------------------------------------------- 工具

pub(crate) fn new_id(prefix: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}_{nanos:x}{seq:04x}")
}

pub(crate) fn now_iso() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// 缓存时间是否在 TTL 内。
///
/// 时间解析不了时**按过期处理**：宁可多查一次数据库，也不要拿一条读不懂的时间戳
/// 当「刚缓存的」用。
pub(crate) fn is_fresh(cached_at: &str, ttl_secs: i64) -> bool {
    age_secs(cached_at).map(|age| age <= ttl_secs).unwrap_or(false)
}

/// 缓存距今秒数；解析失败返回 None。
pub(crate) fn age_secs(cached_at: &str) -> Option<i64> {
    let at = chrono::DateTime::parse_from_rfc3339(cached_at).ok()?;
    Some(
        (chrono::Local::now() - at.with_timezone(&chrono::Local))
            .num_seconds()
            .max(0),
    )
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS connections (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    kind          TEXT NOT NULL,
    host          TEXT,
    port          INTEGER,
    database_name TEXT,
    username      TEXT,
    password      TEXT,
    file_path     TEXT,
    color         TEXT,
    extra         TEXT,
    read_only     INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS query_history (
    id              TEXT PRIMARY KEY,
    connection_id   TEXT,
    connection_name TEXT,
    sql             TEXT NOT NULL,
    status          TEXT NOT NULL,
    row_count       INTEGER NOT NULL DEFAULT 0,
    duration_ms     INTEGER NOT NULL DEFAULT 0,
    error_code      TEXT,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_history_created ON query_history(created_at);
CREATE INDEX IF NOT EXISTS idx_history_connection ON query_history(connection_id);

CREATE TABLE IF NOT EXISTS app_settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS schema_cache (
    connection_id TEXT NOT NULL,
    object_name   TEXT NOT NULL,
    payload       TEXT NOT NULL,
    cached_at     TEXT NOT NULL,
    PRIMARY KEY (connection_id, object_name)
);

-- AI 设置：单行（id 恒为 1）+ 一组模型端点。
-- 以前是 ~/.dbmind/ai-config.json（含 apiKey 明文）—— 搬进库只是换了存放位置，
-- 明文这一点没变，备份/外发时同样要当机密看待。
CREATE TABLE IF NOT EXISTS ai_settings (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    enabled       INTEGER NOT NULL DEFAULT 0,
    privacy_mode  TEXT NOT NULL DEFAULT 'allow',
    audit_enabled INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT NOT NULL
);

-- ordinal 既是顺序也是主键：模型列表的顺序就是界面上的顺序，整体替换时不会乱。
CREATE TABLE IF NOT EXISTS ai_models (
    ordinal    INTEGER PRIMARY KEY,
    id         TEXT NOT NULL DEFAULT '',
    name       TEXT NOT NULL DEFAULT '',
    base_url   TEXT NOT NULL DEFAULT '',
    api_key    TEXT NOT NULL DEFAULT '',
    model      TEXT NOT NULL DEFAULT '',
    max_tokens INTEGER NOT NULL DEFAULT 0,
    embedding  INTEGER NOT NULL DEFAULT 0
);

-- 用量账本两级：每天的总次数 / 每个模型每天的次数与 token。
-- 分开两张表是为了能直接用 SQL 汇总（「这个月谁烧的 token 最多」）。
CREATE TABLE IF NOT EXISTS ai_usage_days (
    day   TEXT PRIMARY KEY,
    calls INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS ai_usage_models (
    model_key         TEXT NOT NULL,
    day               TEXT NOT NULL,
    calls             INTEGER NOT NULL DEFAULT 0,
    prompt_tokens     INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (model_key, day)
);

-- 审计日志：以前是只增不减的 ai-audit.log（JSONL），现在有表就有上限可言了。
CREATE TABLE IF NOT EXISTS ai_audit (
    id     INTEGER PRIMARY KEY AUTOINCREMENT,
    time   TEXT NOT NULL,
    kind   TEXT NOT NULL,
    prompt TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ai_audit_time ON ai_audit(time);

-- 团队知识：术语表 + 采纳示例（以前是一个 ai-knowledge.json 里的两个数组）。
-- 上限由业务层把关（200 / 300）：这不是数据库表越大越好，是一份"给提示词用的速查表"，
-- 堆太多只会挤占上下文。落成表的好处是能按 connectionId 过滤、按 hits 排序。
CREATE TABLE IF NOT EXISTS ai_glossary (
    id            TEXT PRIMARY KEY,
    term          TEXT NOT NULL DEFAULT '',
    definition    TEXT NOT NULL DEFAULT '',
    mapping       TEXT NOT NULL DEFAULT '',
    connection_id TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_ai_glossary_conn ON ai_glossary(connection_id);

CREATE TABLE IF NOT EXISTS ai_examples (
    id            TEXT PRIMARY KEY,
    question      TEXT NOT NULL DEFAULT '',
    sql           TEXT NOT NULL DEFAULT '',
    connection_id TEXT NOT NULL DEFAULT '',
    database_name TEXT NOT NULL DEFAULT '',
    hits          INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ai_examples_conn ON ai_examples(connection_id);

-- 数据质量规则：按 连接/库/表 一条，rules 是规则数组的 JSON
-- （规则体是插件化的：不同检查器参数各异，硬拆成列只会一直加列）。
CREATE TABLE IF NOT EXISTS ai_quality_rules (
    connection_id TEXT NOT NULL,
    database_name TEXT NOT NULL DEFAULT '',
    table_name    TEXT NOT NULL DEFAULT '',
    rules         TEXT NOT NULL DEFAULT '[]',
    saved_at      TEXT NOT NULL,
    PRIMARY KEY (connection_id, database_name, table_name)
);

-- 知识库主表（原来是一个 index.json 数组）。
-- 统计字段（doc_count / chunk_count / char_count）由 refresh 重算后写回。
CREATE TABLE IF NOT EXISTS kb_list (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    doc_count   INTEGER NOT NULL DEFAULT 0,
    chunk_count INTEGER NOT NULL DEFAULT 0,
    char_count  INTEGER NOT NULL DEFAULT 0
);

-- 知识库配置：kb_id 为空串表示「全局默认」（原来是根目录下的 config.json）。
-- 配置字段多而扁平，且总是整体读写，所以按一份 JSON 存。
CREATE TABLE IF NOT EXISTS kb_config (
    kb_id      TEXT PRIMARY KEY,
    payload    TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 文档：原文 + 父块 + 子块。分块是"整体读写"的大块数据（分块列表只整存整取），
-- 拆成"一块一行"只会让读写变成 N 次往返，换不来任何查询能力。
CREATE TABLE IF NOT EXISTS kb_docs (
    kb_id      TEXT NOT NULL,
    doc_id     TEXT NOT NULL,
    title      TEXT NOT NULL DEFAULT '',
    source     TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    char_count INTEGER NOT NULL DEFAULT 0,
    raw        TEXT NOT NULL DEFAULT '',
    parents    TEXT NOT NULL DEFAULT '[]',
    chunks     TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (kb_id, doc_id)
);
CREATE INDEX IF NOT EXISTS idx_kb_docs_kb ON kb_docs(kb_id, created_at);

-- 向量：一份文档的所有块向量拼成一段小端 f32 二进制。
-- 这是唯一一处值得用 BLOB 的地方：JSON 文本要写成 "0.123456789"，
-- 同样的数字是二进制的好几倍大，解析还慢。
CREATE TABLE IF NOT EXISTS kb_vectors (
    kb_id   TEXT NOT NULL,
    doc_id  TEXT NOT NULL,
    model   TEXT NOT NULL DEFAULT '',
    dim     INTEGER NOT NULL DEFAULT 0,
    payload BLOB NOT NULL,
    PRIMARY KEY (kb_id, doc_id)
);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn sqlite_config(name: &str) -> ConnectionConfig {
        ConnectionConfig::new(name, ConnectionKind::Sqlite).with_file(format!("./{name}.db"))
    }

    #[test]
    fn 结构缓存读写与作废() {
        let store = store();
        store.put_schema("c1", Store::SCHEMA_OBJECTS_KEY, "[]").unwrap();
        assert_eq!(
            store
                .fresh_schema("c1", Store::SCHEMA_OBJECTS_KEY, 300)
                .unwrap()
                .as_deref(),
            Some("[]")
        );

        // 覆盖写：同一个键再写一次是更新，不是插入两条
        store
            .put_schema("c1", Store::SCHEMA_OBJECTS_KEY, "[{\"name\":\"t\"}]")
            .unwrap();
        assert_eq!(
            store
                .fresh_schema("c1", Store::SCHEMA_OBJECTS_KEY, 300)
                .unwrap()
                .as_deref(),
            Some("[{\"name\":\"t\"}]")
        );
        assert_eq!(store.schema_cache_info("c1").unwrap().entries, 1);

        let info = store.schema_cache_info("c1").unwrap();
        assert!(info.age_secs.unwrap() >= 0, "应能算出缓存年龄");
        assert!(info.latest_cached_at.is_some());

        // 负 TTL 必然过期（同秒内 age=0，用 0 测不出来）
        assert!(
            store
                .fresh_schema("c1", Store::SCHEMA_OBJECTS_KEY, -1)
                .unwrap()
                .is_none(),
            "过期就应视为未命中"
        );

        assert_eq!(
            store
                .invalidate_schema("c1", Some(Store::SCHEMA_OBJECTS_KEY))
                .unwrap(),
            1
        );
        assert!(store
            .fresh_schema("c1", Store::SCHEMA_OBJECTS_KEY, 300)
            .unwrap()
            .is_none());

        // 整连接作废：清单 + 单表列一起清
        store.put_schema("c1", Store::SCHEMA_OBJECTS_KEY, "[]").unwrap();
        store
            .put_schema("c1", &Store::schema_columns_key("t"), "[]")
            .unwrap();
        assert_eq!(store.invalidate_schema("c1", None).unwrap(), 2);
        assert_eq!(store.schema_cache_info("c1").unwrap().entries, 0);
    }

    #[test]
    fn 读不懂的时间戳按过期处理() {
        assert!(is_fresh(&now_iso(), 300), "刚写入的缓存必然命中");
        assert!(!is_fresh("not-a-time", 300), "解析不了就不能当新鲜的用");
        let old = (chrono::Local::now() - chrono::Duration::seconds(600))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        assert!(!is_fresh(&old, 300), "10 分钟前的缓存应过期");
        assert!(is_fresh(&old, 3600), "TTL 更长时应仍命中");
    }

    #[test]
    fn 删连接会清掉它的结构缓存() {
        let store = store();
        let record = store.insert_connection(&sqlite_config("c")).unwrap();
        store
            .put_schema(&record.id, Store::SCHEMA_OBJECTS_KEY, "[]")
            .unwrap();
        assert!(store.delete_connection(&record.id).unwrap());
        assert_eq!(store.schema_cache_info(&record.id).unwrap().entries, 0);
    }

    #[test]
    fn 连接增删改查() {
        let s = store();
        assert!(s.list_connections().unwrap().is_empty());

        let created = s.insert_connection(&sqlite_config("demo")).unwrap();
        assert_eq!(created.name(), "demo");
        assert!(!created.read_only);
        assert_eq!(s.list_connections().unwrap().len(), 1);

        // 按名称能找到，按 id 也能找到
        assert!(s.find_connection("demo").unwrap().is_some());
        assert!(s.find_connection(&created.id).unwrap().is_some());

        let updated = s
            .update_connection(&created.id, &sqlite_config("demo2").with_file("./other.db"))
            .unwrap();
        assert_eq!(updated.name(), "demo2");
        assert_eq!(updated.config.resolved_file(), Some("./other.db"));

        let ro = s.set_read_only("demo2", true).unwrap();
        assert!(ro.read_only);

        assert!(s.delete_connection(&created.id).unwrap());
        assert!(s.list_connections().unwrap().is_empty());
        assert!(!s.delete_connection("不存在").unwrap());
    }

    #[test]
    fn 同名连接被拒绝() {
        let s = store();
        s.insert_connection(&sqlite_config("dup")).unwrap();
        let err = s.insert_connection(&sqlite_config("dup")).unwrap_err();
        assert_eq!(err.code, ErrorCode::ConnInvalid);
    }

    #[test]
    fn 非法配置不落库() {
        let s = store();
        let bad = ConnectionConfig::new("bad", ConnectionKind::Postgresql); // 缺 host
        assert_eq!(
            s.insert_connection(&bad).unwrap_err().code,
            ErrorCode::ConnInvalid
        );
        assert!(s.list_connections().unwrap().is_empty());
    }

    #[test]
    fn 找不到连接时给出稳定错误码() {
        let s = store();
        let err = s.require_connection("ghost").unwrap_err();
        assert_eq!(err.code, ErrorCode::ConnNotFound);
        assert_eq!(err.code_str(), "DBMIND-CONN-0001");
    }

    #[test]
    fn 历史记录与清理() {
        let s = store();
        let conn = s.insert_connection(&sqlite_config("h")).unwrap();
        for i in 0..3 {
            s.record_history(&NewHistoryEntry {
                connection_id: Some(conn.id.clone()),
                connection_name: Some(conn.name().to_string()),
                sql: format!("select {i}"),
                status: if i == 2 {
                    HistoryStatus::Error
                } else {
                    HistoryStatus::Ok
                },
                row_count: i,
                duration_ms: 5,
                error_code: if i == 2 {
                    Some("DBMIND-QUERY-0002".into())
                } else {
                    None
                },
            })
            .unwrap();
        }
        let all = s.list_history(10, None).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all.iter().filter(|h| h.status == HistoryStatus::Error).count(), 1);
        assert_eq!(s.list_history(10, Some(&conn.id)).unwrap().len(), 3);
        assert_eq!(s.list_history(10, Some("other")).unwrap().len(), 0);
        assert_eq!(s.clear_history().unwrap(), 3);
        assert!(s.list_history(10, None).unwrap().is_empty());
    }

    #[test]
    fn 设置读写与默认值() {
        let s = store();
        assert!(!s.get_bool_setting(Store::KEY_AI_WRITE_ENABLED, true).unwrap());
        s.set_setting(Store::KEY_AI_WRITE_ENABLED, "true").unwrap();
        assert!(s.get_bool_setting(Store::KEY_AI_WRITE_ENABLED, false).unwrap());
        assert!(!s.all_settings().unwrap().is_empty());
    }
}
