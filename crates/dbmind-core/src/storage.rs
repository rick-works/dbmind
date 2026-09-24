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
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

const SETTINGS_SAFETY_PRODUCTION: &str = "safety.protectProduction";
const SETTINGS_SAFETY_AI_WRITE: &str = "safety.aiWriteEnabled";
const SETTINGS_UI_THEME: &str = "ui.theme";
const SETTINGS_PAGE_SIZE: &str = "query.defaultPageSize";
/// 全局会话配额：每个宿主进程最多持有多少条物理会话（0 = 不限制）。
const SETTINGS_SESSION_MAX_PER_HOST: &str = "session.maxPerHost";
/// 空闲会话回收时长（秒；0 = 不按空闲回收，只保留「配额满时回收」）。
const SETTINGS_SESSION_IDLE_TIMEOUT: &str = "session.idleTimeoutSecs";

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
             duration_ms, error_code, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
    pub const KEY_THEME: &'static str = SETTINGS_UI_THEME;
    pub const KEY_PAGE_SIZE: &'static str = SETTINGS_PAGE_SIZE;
    pub const KEY_SESSION_MAX_PER_HOST: &'static str = SETTINGS_SESSION_MAX_PER_HOST;
    pub const KEY_SESSION_IDLE_TIMEOUT: &'static str = SETTINGS_SESSION_IDLE_TIMEOUT;

    fn seed_settings(&self) -> Result<()> {
        let conn = self.lock();
        for (key, value) in [
            (SETTINGS_SAFETY_PRODUCTION, "false"),
            (SETTINGS_SAFETY_AI_WRITE, "false"),
            (SETTINGS_UI_THEME, "system"),
            (SETTINGS_PAGE_SIZE, "2000"),
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

// ---------------------------------------------------------------- 全局句柄

/// 进程内唯一的主库句柄。
///
/// 为什么要有它：AI 设置这类读写是"环境式"的 —— 对话管线深处要用它，
/// 手上却只有一堆与存储无关的参数。把 Store 一路透传下去要改几十处签名，
/// 而那些函数跟存储毫无关系。装一次全局句柄，签名一个都不用动。
///
/// 注意：**刻意不在 Store 构造里自动装**。测试会造大量内存库，自动装会互相串；
/// 装不装由壳层决定（`serve()` 启动时装一次）。
static GLOBAL_STORE: OnceLock<Arc<Store>> = OnceLock::new();

/// 装全局主库句柄（幂等：重复调用以第一次为准）。
pub fn install_global_store(store: Arc<Store>) {
    let _ = GLOBAL_STORE.set(store);
}

/// 取全局主库句柄；没装（单测、或只用内核不开壳）返回 None。
pub fn global_store() -> Option<Arc<Store>> {
    GLOBAL_STORE.get().cloned()
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
