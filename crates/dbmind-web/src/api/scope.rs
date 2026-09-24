//! 「连接 + 库名」→ 实际执行用的连接（**影子连接**）。
//!
//! ## 为什么需要它
//!
//! 内核的连接模型是「一条连接绑定一个目标库」：JDBC URL 里的库名在连接建立时就定了。
//! 而上游的树是「连接 → 库（服务器上的每一个）→ 表」，SQL 编辑器也会带上当前选中的库。
//! 于是「点开另一个库」这件事，在内核里没有对应的动作。
//!
//! 有些数据库（PostgreSQL）**根本不允许**在一条连接里查询别的库 —— 官方要求每个库一条连接；
//! MySQL / SQL Server 虽然能用 `USE` / 三段式名，但内核没有「切库」这个动作可调。
//!
//! 所以这里用**影子连接**：切到某个库时，按需生成一条同配置、只把 `database` 换掉的连接记录，
//! 用完留着复用。于是：
//!
//! - 所有类型（含 PostgreSQL）的跨库浏览都成立 —— 因为影子就是一条**真实的独立连接**；
//! - 不用改内核，也不用给每种类型写一套「切库方言」；
//! - 影子对用户不可见（`/api/connections` 会滤掉），删主连接时级联清理。
//!
//! 代价要说清楚：**每个被展开的库会占一条物理会话**。展开十几个库时，
//! 宿主的会话池（`session.maxPerHost`，默认 32）是有可能被占满的 —— 这时后续连接会排队，
//! 表现为「点开新库要等一会儿」。用「只展开在用的库」即可回避，将来可以按空闲回收影子。

use dbmind_core::ConnectionRecord;
use serde_json::{json, Map, Value};

use crate::api::dialect::Dialect;
use crate::api::error::XResult;
use crate::api::{blocking, require_record};
use crate::AppState;

/// 影子标记（存在连接记录的 `extra` 里）。
pub const EXTRA_SHADOW_FOR: &str = "shadowFor";
pub const EXTRA_SHADOW_DATABASE: &str = "shadowDatabase";

/// 这条记录是不是影子（用户不该看到它）。
pub fn is_shadow(record: &ConnectionRecord) -> bool {
    record
        .config
        .extra
        .as_ref()
        .and_then(|extra| extra.get(EXTRA_SHADOW_FOR))
        .is_some()
}

fn shadow_name(base_name: &str, database: &str) -> String {
    format!("{base_name} ▸ {database}")
}

async fn find_shadow(state: &AppState, name: &str, base_id: &str) -> XResult<Option<String>> {
    let engine = state.engine.clone();
    let name = name.to_string();
    let found = blocking(move || engine.find_connection(&name)).await?;
    Ok(found
        .filter(|record| is_shadow(record) && shadow_base(record).as_deref() == Some(base_id))
        .map(|record| record.id))
}

fn shadow_base(record: &ConnectionRecord) -> Option<String> {
    record
        .config
        .extra
        .as_ref()
        .and_then(|extra| extra.get(EXTRA_SHADOW_FOR))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// 把「连接 + 库名」解析成可执行用的连接 id。
///
/// 库名为空、或就是这个连接本身的库、或该类型不支持切库 ⇒ 原样返回主连接 id。
pub async fn resolve(state: &AppState, base_id: &str, database: &str) -> XResult<String> {
    let base = require_record(state, base_id).await?;
    let dialect = Dialect::new(base.kind());
    let raw = database.trim().to_string();
    // schema 层级的类型（SQL Server / PostgreSQL / KingbaseES）在树上多一层「模式」，
    // 而这一层是前端把「库.模式」拼成一个字符串传下来的（见 MainView 的 db + '.' + schema）。
    // 影子连接只认**库名**，所以这里按第一个点拆开 —— 否则会去找一个名叫
    // `pipeline.dbo` 的库，然后连不上（实机上就是这样冒出 `local ▸ pipeline.dbo` 这种影子的）。
    let wanted = if dialect.is_schema_aware() {
        match raw.split_once('.') {
            Some((database, _schema)) if !database.is_empty() => database.to_string(),
            _ => raw,
        }
    } else {
        raw
    };
    let stored = base.config.database.clone();

    if wanted.is_empty() {
        // 「不指定库」有两种情形要分开：
        // - 连接里存着空串 ⇒ 直接用它（内核会把 URL 渲染成 `databaseName=`，连到服务器不指定默认库）；
        // - 连接里是 NULL（老记录，或早期版本把空串存成了 NULL）⇒ 内核渲染 URL 会报
        //   「缺少 {database} 对应的字段」—— 用户想**留空**反而被拦住。这时换成一条
        //   database = "" 的影子连接，让「留空」真正可用，不用逼他回去点一次保存。
        if stored.is_none() && dialect.switchable_database() {
            return ensure_shadow(state, &base, "", UNSET_DATABASE_LABEL).await;
        }
        return Ok(base.id);
    }

    if let Some(current) = stored.as_deref() {
        if !current.trim().is_empty() && wanted.eq_ignore_ascii_case(current.trim()) {
            return Ok(base.id);
        }
    }
    // 文件型（SQLite/H2/Derby 一个文件就是一个库）与非服务器库语义的类型（Oracle/DM/DB2）
    // 不能靠改 `database` 切库 —— 硬切会把 URL 改坏。这时老实用主连接，由调用方决定要报什么错。
    if !dialect.switchable_database() {
        return Ok(base.id);
    }
    ensure_shadow(state, &base, &wanted, &wanted).await
}

/// 「未指定库」在影子连接名字里的占位（要唯一且看得出是被合成的）。
const UNSET_DATABASE_LABEL: &str = "（未指定库）";

/// 取用/创建一条指向 `database` 的影子连接。
async fn ensure_shadow(
    state: &AppState,
    base: &ConnectionRecord,
    database: &str,
    label: &str,
) -> XResult<String> {
    let name = shadow_name(&base.config.name, label);
    if let Some(existing) = find_shadow(state, &name, &base.id).await? {
        return Ok(existing);
    }

    let mut config = base.config.clone();
    config.name = name.clone();
    // 空串不是 NULL：这正是让内核渲染出「没有库名」的 JDBC URL 的关键
    config.database = Some(database.to_string());
    let mut extra = match config.extra.take() {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    extra.insert(EXTRA_SHADOW_FOR.to_string(), json!(base.id));
    extra.insert(EXTRA_SHADOW_DATABASE.to_string(), json!(database));
    config.extra = Some(Value::Object(extra));

    let engine = state.engine.clone();
    match blocking(move || engine.add_connection(config)).await {
        Ok(record) => Ok(record.id),
        Err(err) => {
            // 并发下可能刚好被别人建好了 —— 那不是错误，取用即可
            if let Some(existing) = find_shadow(state, &name, &base.id).await? {
                return Ok(existing);
            }
            Err(err)
        }
    }
}

/// 删除某条连接的全部影子（删主连接时级联调用，否则影子会变成树上看不见的垃圾）。
pub async fn delete_shadows(state: &AppState, base_id: &str) -> XResult<usize> {
    let engine = state.engine.clone();
    let records = blocking(move || engine.list_connections()).await?;
    let mut removed = 0;
    for record in records {
        if shadow_base(&record).as_deref() != Some(base_id) {
            continue;
        }
        let engine = state.engine.clone();
        let id = record.id.clone();
        if blocking(move || engine.remove_connection(&id)).await.unwrap_or(false) {
            removed += 1;
        }
    }
    Ok(removed)
}
