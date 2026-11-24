//! Doris 外部 catalog 的 SQL 表引用改写。
//!
//! 场景：编辑器选中 `clickhouse_catalog.oishi_ods` 后写 `select * from biz_message`。
//! 这条 SQL 不能靠"切库"解决 —— Doris 的 JDBC URL 里 database 带点号握手就挂，
//! 裸库名又会被解析成 internal 下的同名库（真机踩过，查到的是另一张表）。
//!
//! 这里沿用数据浏览（`meta::data::run_external`）已经验证过的口径：**主连接 +
//! 全限定表名**。区别是数据浏览的 SQL 是内部拼的（表名单段），而这里是用户手输的
//! 任意 SQL —— 所以复用 `mssql_scope::rewrite` 那套保守改写：
//!
//! - 只给 FROM / JOIN / UPDATE / INTO 后面、**该 catalog 下确实存在**、且自己没写前缀的
//!   裸表名补上 `` `catalog`.`db`. ``；
//! - 已带前缀、CTE、临时表、表值函数一律不动；
//! - 取不到表清单（权限/连接抖动）时原样返回，**绝不因为改写让本来能跑的语句失败**。
//!
//! 与 SQL Server 那支共用同一个改写器，只是前缀形状不同（这里两段、反引号）。

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::dialect::Dialect;
use super::meta::external_catalog_of;
use crate::AppState;

const CACHE_TTL: Duration = Duration::from_secs(60);
type CacheEntry = (Instant, std::sync::Arc<HashSet<String>>);

fn cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 外部 catalog 的表名清单（走 `<catalog>.information_schema.tables`）。
/// 失败返回 None，调用方据此跳过改写。
async fn tables_of(
    state: &AppState,
    conn: &str,
    catalog: &str,
    db: &str,
    dialect: &Dialect,
) -> Option<std::sync::Arc<HashSet<String>>> {
    let key = format!("{conn}|{catalog}|{db}");
    if let Some((at, set)) = cache().lock().ok()?.get(&key) {
        if at.elapsed() < CACHE_TTL {
            return Some(set.clone());
        }
    }
    let sql = format!(
        "select table_name from {}.information_schema.tables where table_schema = {}",
        dialect.quote_exact(catalog),
        dialect.literal(db)
    );
    // 直接走主连接（不 resolve 影子）：跨 catalog 的元数据只能用三段名在主连接上查，
    // 与 meta::data::run_external 同一口径。
    crate::api::driver::ensure_for_connection(state, conn).await.ok()?;
    let engine = state.engine();
    let request = dbmind_core::QueryRequest {
        schema: None,
        connection: conn.to_string(),
        sql,
        options: dbmind_core::QueryOptions { max_rows: 5000, timeout_ms: 120_000 },
        execution_id: None,
        session: None,
        read_only: Some(true),
        internal: true,
    };
    let result = crate::api::blocking(move || engine.execute(request, dbmind_core::AccessContext::Web))
        .await
        .ok()?;
    let set: HashSet<String> = crate::api::meta::rows_of(&result)
        .iter()
        .filter_map(|row| row.values().next())
        .filter_map(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if set.is_empty() {
        return None;
    }
    let set = std::sync::Arc::new(set);
    if let Ok(mut map) = cache().lock() {
        map.insert(key, (Instant::now(), set.clone()));
    }
    Some(set)
}

/// 给外部 catalog 的查询补全限定表名前缀。
///
/// 返回 `Some(sql)` 表示这条 SQL 要走**主连接**（调用方不要 resolve 影子连接），
/// `None` 表示不是外部 catalog 场景 / 没有可改的表，走原来的执行路径。
pub(crate) async fn apply(
    state: &AppState,
    conn: &str,
    database: &str,
    dialect: Dialect,
    sql: String,
) -> Option<String> {
    let (catalog, db) = external_catalog_of(&dialect, database)?;
    // 表清单只是「更精确」的改写依据，不是前提：连不上 / 没权限查 information_schema 时
    // 仍要改写（宽松模式给所有表引用补前缀），否则又退回 scope 那句硬拒绝。
    // 例外：SQL 里根本没有表引用（select 1 之类）—— 不必改写，直接走原路径。
    if !super::mssql_scope::may_reference_table(&sql) {
        return None;
    }
    let known = tables_of(state, conn, &catalog, &db, &dialect)
        .await
        .unwrap_or_default();
    let prefix = format!(
        "{}.{}.",
        dialect.quote_exact(&catalog),
        dialect.quote_exact(&db)
    );
    Some(super::mssql_scope::rewrite(&sql, &prefix, &known, true, true))
}
