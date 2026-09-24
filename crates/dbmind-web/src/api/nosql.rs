//! `/api/{mongodb|redis|elasticsearch}/…` —— NoSQL 浏览与命令执行。
//!
//! NoSQL 的「库 / 集合 / 文档」在三种协议里各是一回事，映射关系写在这里：
//!
//! | 协议 | 库 | 集合 | 文档 |
//! |---|---|---|---|
//! | MongoDB | 连接绑定的库 | `listCollections` | `db.<c>.find({}).limit(n)` |
//! | Elasticsearch | 连接绑定的库 | 索引（`_cat/indices`） | `GET /<索引>/_search` |
//! | Redis | `dbN`（**配了「DB 编号」就只列那一个**；没配则按 `CONFIG GET databases` 列全部） | 该库里的键 | 按键的类型选读命令（GET/LRANGE/HGETALL/…） |
//!
//! Redis 那一列是刻意做实的：键有类型，`TYPE` 之后按类型取才拿得到有意义的值；
//! 一律 `TYPE k` 只会让每张「表」都显示一行 `string`。

use axum::extract::{Path, RawQuery, State};
use axum::Json;
use dbmind_core::{AccessContext, ConnectionKind, QueryOptions, QueryRequest};
use serde_json::{json, Value};

use crate::api::error::{XError, XResult};
use crate::api::{blocking, meta, require_record, shape, Params};
use crate::AppState;

/// 测试连接：与关系型共用同一套实现（上游的 NoSQL 模块也有 `/test`）。
pub use crate::api::meta::test;

const DEFAULT_PAGE_SIZE: usize = 100;

/// 「库」清单。
pub async fn databases(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> XResult<Json<Value>> {
    let record = require_record(&state, &id).await?;
    if record.kind() == ConnectionKind::Redis {
        // Redis 的「库」就是 dbN：宿主把 INFO keyspace 的每个 dbN 当作一张「表」报上来。
        // 关键是 keyspace **只报有键的库** —— 空库（包括用户配的那个、以及刚建好还没写数据的）
        // 它一个都不报，于是「不配编号」时左边树里只有零星几个库。
        // 「DB 编号」填了没有？Redis 不在 `switchable_database` 名单里，所以留空落库就是 None ——
        // 「留空」与「填了 0」是两件事，必须分得开（留空 = 连的是整个实例）。
        let configured = record.config.database.as_deref().unwrap_or("").trim().to_string();

        // 配了编号 ⇒ **只列它**，一个都不多。连接绑定的就是这一个库，再把 keyspace 里
        // 那些「恰好有键」的库一起挂上去，界面就分不清「我连的是哪个」了
        // —— 实测：配了 8，树里却还挂着 db0，第一眼就像配错了。
        // 这条路径连 keyspace 都不用查。
        if let Ok(number) = configured.parse::<u32>() {
            return Ok(Json(json!(vec![format!("db{number}")])));
        }

        // 没配编号（或填的不是数字）⇒ 树里列出**全部**库（db0…dbN-1）：
        // 没配就是「连的这个实例」，只列有键的那几个会让空库无处可点。
        let engine = state.engine.clone();
        // 闭包是 `move` 的：连接 id 克隆一份进去，后面还要拿它去问 CONFIG（同 `documents` 的写法）
        let connection = id.clone();
        let keyspace: Vec<String> = blocking(move || engine.list_tables_fresh(&connection))
            .await?
            .into_iter()
            .map(|t| t.name)
            .collect();
        // 总库数问服务端（CONFIG GET databases）；问不到时**不猜**，退回原来的
        // 「只列 keyspace 里有的」—— 凭空造出 16 个不存在的库比少列几个更糟。
        if let Some(total) = redis_db_count(&state, &id).await {
            let mut all: Vec<String> = (0..total).map(|n| format!("db{n}")).collect();
            for name in keyspace {
                if !all.iter().any(|existing| existing == &name) {
                    all.push(name);
                }
            }
            return Ok(Json(json!(all)));
        }
        return Ok(Json(json!(if keyspace.is_empty() {
            vec!["db0".to_string()]
        } else {
            keyspace
        })));
    }
    let name = record
        .config
        .database
        .clone()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "(default)".to_string());
    Ok(Json(json!([name])))
}

/// 服务端一共配了几个库（`CONFIG GET databases`，Redis 默认 16）。
///
/// 这是**唯一**能问到「全部库」的地方：`INFO keyspace` 只列有键的库。
/// 拿不到就返回 `None` —— 托管 Redis（ElastiCache 这类）常把 CONFIG 禁掉，
/// 那种情况下不能退化成「假设 16 个」：会凭空列出服务端根本不存在的库。
async fn redis_db_count(state: &AppState, id: &str) -> Option<usize> {
    let engine = state.engine.clone();
    let request = QueryRequest {
        read_only: None,
        connection: id.to_string(),
        sql: "CONFIG GET databases".to_string(),
        options: QueryOptions {
            max_rows: 8,
            timeout_ms: 10_000,
        },
        execution_id: None,
        // 不要会话亲和：Redis 是单连接池类型，泳道亲和本来就会被忽略，写在这里只会
        // 让人以为"这条查询独占一条会话"。保持 `None` 与结构浏览落在同一条泳道上。
        session: None,
    };
    let result = blocking(move || engine.execute(request, AccessContext::Web))
        .await
        .ok()?;
    let payload = shape::query_result_json(&result);
    let rows = payload.get("rows")?.as_array()?;
    // 结果行是「列名 → 值」的对象（见 `shape` 模块的说明）。列名各家实现可能不同
    // （field / key / name），所以不看列名，只认那个**能当数字读**的值。
    for row in rows {
        let Some(object) = row.as_object() else {
            continue;
        };
        for value in object.values() {
            let parsed = value
                .as_u64()
                .map(|n| n as usize)
                .or_else(|| value.as_str().and_then(|text| text.trim().parse::<usize>().ok()));
            if let Some(count) = parsed {
                if count > 0 && count <= 1024 {
                    return Some(count);
                }
            }
        }
    }
    None
}

/// 「集合」清单（Mongo 的 collection / ES 的索引 / Redis 的键）。
pub async fn collections(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params
        .get("database")
        .unwrap_or_default()
        .to_string();
    let record = require_record(&state, &id).await?;
    let engine = state.engine.clone();

    if record.kind() == ConnectionKind::Redis {
        // 键就是「集合」；内核的列元数据正好是「键 + 类型」（SCAN 上限 500）
        let columns = blocking(move || engine.list_columns_fresh(&id, &database)).await?;
        let items: Vec<Value> = columns
            .into_iter()
            .map(|c| json!({ "name": c.name, "comment": c.type_name.unwrap_or_default() }))
            .collect();
        return Ok(Json(Value::Array(items)));
    }

    let tables = blocking(move || engine.list_tables_fresh(&id)).await?;
    let items: Vec<Value> = tables
        .into_iter()
        .map(|t| json!({ "name": t.name, "comment": Value::Null }))
        .collect();
    Ok(Json(Value::Array(items)))
}

/// 「文档」浏览：按协议生成读命令，交给内核的通用执行。
pub async fn documents(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let collection = params.get("collection").unwrap_or_default();
    let database = params.get("database").unwrap_or_default();
    let size = params
        .get("size")
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|s| *s > 0 && *s <= 10_000)
        .unwrap_or(DEFAULT_PAGE_SIZE);
    if collection.trim().is_empty() {
        return Err(XError::bad_request("缺少 collection 参数"));
    }

    let record = require_record(&state, &id).await?;
    // Redis：读键之前先把会话切到该库（读命令本身不带库名，全靠会话状态 —— 见 `redis_select_db`），
    // 否则读的是**别的库**里同名键的值，或者干脆读成 NULL。
    if record.kind() == ConnectionKind::Redis {
        let target = redis_target_db(record.config.database.as_deref(), &database);
        if let Err(message) = redis_select_db(&state, &id, &target).await {
            return Ok(Json(shape::query_failure_json(&message, 0)));
        }
    }
    // Redis 的「列出该库所有键」模式：界面点开 `dbN` 走的就是它
    // （见 `NoSqlDataView` 的 `isRedisAllKeys` —— 它把 collection 传成 `*`）。
    // 以前这里被当成「读一个名叫 `*` 的键」⇒ `GET *` ⇒ nil ⇒ 界面上只有一行 NULL
    // （用户的原话是"列表还是没出来"）。它要的其实是**键清单**，与 `/collections` 同一份数据。
    if record.kind() == ConnectionKind::Redis && collection.trim() == "*" {
        let page = params
            .get("page")
            .and_then(|p| p.parse::<usize>().ok())
            .filter(|p| *p > 0)
            .unwrap_or(1);
        let pattern = params.get("keyword").unwrap_or_default().to_string();
        let engine = state.engine.clone();
        let connection = id.clone();
        let db = database.to_string();
        let keys = blocking(move || engine.list_columns_fresh(&connection, &db)).await?;
        let mut items: Vec<(String, String)> = keys
            .into_iter()
            .map(|c| (c.name, c.type_name.unwrap_or_default()))
            // 关键字按 Redis 自己的 MATCH 语义（`*` / `?`）过滤，与 SCAN ... MATCH 一致
            .filter(|(key, _)| pattern.is_empty() || redis_glob_match(&pattern, key))
            .collect();
        let total = items.len();
        let page_items: Vec<(String, String)> = items
            .drain(..)
            .skip((page - 1) * size)
            .take(size)
            .collect();
        // 值：字符串键一条 `MGET` 全取回，非字符串键按键类型补读（有上限）
        let values = redis_page_values(&state, &id, &database, &page_items).await;
        let rows: Vec<Value> = page_items
            .iter()
            .zip(values)
            .map(|((key, kind), value)| {
                json!({
                    "key": key,
                    "type": kind,
                    "value": match value {
                        Some(text) => Value::String(text),
                        None => Value::Null,
                    },
                })
            })
            .collect();
        let returned = rows.len();
        return Ok(Json(json!({
            "columns": ["key", "type", "value"],
            "columnTypes": ["", "", ""],
            "rows": rows,
            "rowCount": returned,
            // 分页靠 totalCount：界面优先读它（否则会去解析 notices 里的统计）
            "totalCount": total,
            "hasMore": (page - 1) * size + returned < total,
            "truncated": false,
            "affectedRows": -1,
            "success": true,
            "message": format!("Query OK, {returned} rows returned"),
            "notices": [],
            "executeTime": 0,
            "failedSql": Value::Null,
            "sqlState": Value::Null,
            "errorCode": 0,
        })));
    }

    let protocol = record.kind().protocol();
    let command = match protocol {
        dbmind_core::RuntimeProtocol::Mongodb => {
            format!("db.{collection}.find({{}}).limit({size})")
        }
        dbmind_core::RuntimeProtocol::Elasticsearch => format!("GET /{collection}/_search"),
        dbmind_core::RuntimeProtocol::Redis => {
            redis_read_command(&state, &id, &database, &collection, size).await?
        }
        _ => format!("select * from {collection} limit {size}"),
    };

    let engine = state.engine.clone();
    // 与结构浏览共用同一条会话。三种协议的宿主行为不同，必须分开看
    // （`ConnectionKind::metadata_connection_scoped` / `single_connection_pool` 是判据）：
    //
    // - **Redis** 是单连接池：无论怎么传都只有那一条物理连接，且泳道亲和被忽略
    //   ⇒ 保持 `None`，与结构浏览的 `session_key(config, read_only)` 一致
    //   （给它 `Some(true)` 反而会与元数据的 false 不一致）。
    // - **Mongo / Elasticsearch** 不是单连接池，而结构浏览对它们**强制只读会话**
    //   （`metadata_session_key` 里的 `session_key(config, true)`）⇒ 这里要显式对齐
    //   `Some(true)`，否则同一件事（比如展开集合/索引）会各占一条连接。
    //
    // 两种情况都**不要**再给亲和键：亲和会参与会话键计算，正是分裂的来源。
    let read_only = match protocol {
        dbmind_core::RuntimeProtocol::Redis => None,
        _ => Some(true),
    };
    let request = QueryRequest {
        read_only,
        connection: id,
        sql: command,
        options: QueryOptions {
            max_rows: size,
            timeout_ms: 60_000,
        },
        execution_id: None,
        session: None,
    };
    Ok(Json(match blocking(move || engine.execute(request, AccessContext::Web)).await {
        Ok(result) => shape::query_result_json(&result),
        Err(err) => shape::query_failure_json(&err.message, 0),
    }))
}

/// Redis 的 `MATCH` 通配：只认 `*`（任意串）与 `?`（单字符），与 `SCAN ... MATCH` 同一套语义。
///
/// 为什么自己实现而不是把 pattern 交给服务端：键清单走的是内核的列元数据（SCAN + 上限），
/// 它不接受 pattern；而单发一条 `SCAN 0 MATCH …` 的回复是「游标 + 嵌套数组」，
/// 宿主按单列渲染出来是 `[B@1a2b`（实测），拿不到键名。所以过滤放在这一层做。
/// 经典两指针 + 星号回溯，线性复杂度。
fn redis_glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    // star = 最近一个 `*` 的位置，mark = 那次回溯时 `*` 匹配到的文本位置
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while ti < txt.len() {
        if pi < pat.len() && (pat[pi] == '?' || pat[pi] == txt[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star = pi;
            mark = ti;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

/// 非字符串键最多补读多少个「值预览」：每个键一条命令，所以要上限。
const VALUE_PREVIEW_MAX: usize = 20;

/// 键名发给 Redis 前统一用双引号包起来：键里可能有空格或引号，不包会被切成两个参数。
/// 宿主的切词认双引号（实测 `GET "name"` 有值），反斜杠与引号在这里转义。
fn quote_key(key: &str) -> String {
    format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""))
}

/// 跑一条命令并把**数组回复**取成「一行一个值」（`MGET` 用）。失败返回 `None`（该列留空）。
async fn redis_array_reply(state: &AppState, id: &str, sql: String) -> Option<Vec<Option<String>>> {
    let engine = state.engine.clone();
    let request = QueryRequest {
        read_only: None,
        connection: id.to_string(),
        sql,
        options: QueryOptions {
            max_rows: 1000,
            timeout_ms: 30_000,
        },
        execution_id: None,
        session: Some(SESSION_BROWSE.to_string()),
    };
    let result = blocking(move || engine.execute(request, AccessContext::Web))
        .await
        .ok()?;
    let payload = shape::query_result_json(&result);
    let rows = payload.get("rows")?.as_array()?;
    Some(
        rows.iter()
            .map(|row| match row.get("value") {
                None | Some(Value::Null) => None,
                Some(Value::String(text)) => Some(text.clone()),
                Some(other) => Some(other.to_string()),
            })
            .collect(),
    )
}

/// 把一条读命令的结果压成一行文本：哈希 ⇒ `f1=v1, f2=v2`，列表 ⇒ `a, b, c`。
async fn redis_reply_summary(state: &AppState, id: &str, sql: String) -> Option<String> {
    let engine = state.engine.clone();
    let request = QueryRequest {
        read_only: None,
        connection: id.to_string(),
        sql,
        options: QueryOptions {
            max_rows: 200,
            timeout_ms: 30_000,
        },
        execution_id: None,
        session: Some(SESSION_BROWSE.to_string()),
    };
    let result = blocking(move || engine.execute(request, AccessContext::Web))
        .await
        .ok()?;
    let payload = shape::query_result_json(&result);
    let columns: Vec<String> = payload
        .get("columns")?
        .as_array()?
        .iter()
        .filter_map(|c| c.as_str().map(str::to_string))
        .collect();
    let mut parts: Vec<String> = Vec::new();
    for row in payload.get("rows")?.as_array()? {
        let Some(object) = row.as_object() else {
            continue;
        };
        let cell = |name: &str| match object.get(name) {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
        };
        // 两列（field/value）= 成对回复 ⇒ 合成 `field=value`；单列直接取值
        parts.push(if columns.len() >= 2 {
            format!("{}={}", cell(&columns[0]), cell(&columns[1]))
        } else {
            cell(columns.first().map(String::as_str).unwrap_or("value"))
        });
    }
    Some(parts.join(", "))
}

/// 一页键的「值」列。
///
/// 字符串键用**一条 `MGET`** 全取回（一次往返，与键数无关）；非字符串键 MGET 会给 nil
/// （Redis 对类型不符的键返回 nil，不报错），再按键类型补读，最多 [`VALUE_PREVIEW_MAX`] 个 ——
/// 每个键一条命令，不设上限的话一页 200 个键要串行 200 次，界面直接卡住。
/// 取不到的格如实留 NULL（不编假值）。
async fn redis_page_values(
    state: &AppState,
    id: &str,
    database: &str,
    items: &[(String, String)],
) -> Vec<Option<String>> {
    let mut values: Vec<Option<String>> = vec![None; items.len()];
    if items.is_empty() {
        return values;
    }
    let strings: Vec<(usize, String)> = items
        .iter()
        .enumerate()
        .filter(|(_, (_, kind))| kind.is_empty() || kind == "string")
        .map(|(index, (key, _))| (index, key.clone()))
        .collect();
    if !strings.is_empty() {
        let command = format!(
            "MGET {}",
            strings
                .iter()
                .map(|(_, key)| quote_key(key))
                .collect::<Vec<_>>()
                .join(" ")
        );
        if let Some(reply) = redis_array_reply(state, id, command).await {
            // 数组回复按元素逐行渲染 ⇒ 行序就是键序
            for (slot, (index, _)) in reply.into_iter().zip(strings.iter()) {
                values[*index] = slot;
            }
        }
    }
    let mut fetched = 0usize;
    for (index, (key, kind)) in items.iter().enumerate() {
        if values[index].is_some() || kind.is_empty() || kind == "string" {
            continue;
        }
        if fetched >= VALUE_PREVIEW_MAX {
            break;
        }
        fetched += 1;
        // 与「看某个键」走同一套：按键类型选读命令（GET/LRANGE/HGETALL/SMEMBERS/…）
        if let Ok(command) = redis_read_command(state, id, database, key, 5).await {
            if let Some(summary) = redis_reply_summary(state, id, command).await {
                values[index] = Some(summary);
            }
        }
    }
    values
}

/// 按键的类型选读命令（内核的列元数据里带着每个键的类型）。
async fn redis_read_command(
    state: &AppState,
    id: &str,
    database: &str,
    key: &str,
    size: usize,
) -> XResult<String> {
    let engine = state.engine.clone();
    let db = database.to_string();
    let connection = id.to_string();
    let columns = blocking(move || engine.list_columns_fresh(&connection, &db))
        .await
        .unwrap_or_default();
    let kind = columns
        .into_iter()
        .find(|c| c.name == key)
        .and_then(|c| c.type_name)
        .unwrap_or_default()
        .to_ascii_lowercase();
    Ok(match kind.as_str() {
        "list" => format!("LRANGE {key} 0 {}", size.saturating_sub(1)),
        "hash" => format!("HGETALL {key}"),
        "set" => format!("SMEMBERS {key}"),
        "zset" => format!("ZRANGE {key} 0 {} WITHSCORES", size.saturating_sub(1)),
        // string / 未知类型都按字符串读：拿不到时 Redis 会如实回一个错误，不会静默给空
        _ => format!("GET {key}"),
    })
}

/// 删除集合 / 键 / 索引。
pub async fn delete_collection(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let collection = params.get("collection").unwrap_or_default();
    let database = params.get("database").unwrap_or_default().to_string();
    if collection.trim().is_empty() {
        return Err(XError::bad_request("缺少 collection 参数"));
    }
    let record = require_record(&state, &id).await?;
    // Redis：删键前同样先切库 —— 库没切过去，就是在**别的库**里删同名键（静默删错数据）
    if record.kind() == ConnectionKind::Redis {
        let target = redis_target_db(record.config.database.as_deref(), &database);
        redis_select_db(&state, &id, &target)
            .await
            .map_err(XError::bad_request)?;
    }
    let command = match record.kind().protocol() {
        dbmind_core::RuntimeProtocol::Mongodb => format!("db.{collection}.drop()"),
        dbmind_core::RuntimeProtocol::Elasticsearch => format!("DELETE /{collection}"),
        dbmind_core::RuntimeProtocol::Redis => format!("DEL {collection}"),
        _ => return Err(XError::not_implemented("删除集合（SQL 协议走表删除）")),
    };
    meta::run_sql(&state, &id, command, 1).await?;
    Ok(Json(json!({ "success": true, "message": format!("已删除 {collection}") })))
}

/// 浏览类请求共用的会话键：同一个键的语句落在**同一条物理会话**上
/// （见 `QueryRequest::session` 的说明）。
///
/// ⚠️ 它**只对非单连接池的类型有效** —— `lane_key` 里单连接池类型会忽略亲和。
/// Redis 恰好是单连接池（`ConnectionKind::Redis.single_connection_pool() == true`），
/// 所以这个键在 Redis 上**不起作用**；而 Redis 也**不需要**它：那个类型总共只有一条物理
/// 会话，`SELECT n` 改的库状态天然作用于随后的命令。
///
/// ⚠️ 反过来**千万不要**给 Redis 的浏览查询设 `read_only: Some(true)`：
/// 结构浏览对单连接池 / connection-scoped 的类型走 `metadata_session_key` 的**第一分支**，
/// 用的是**连接策略标记**（默认 false）。这里若显式设 true，会话键的第 7 段就对不上，
/// 反而**多出一条物理会话** —— 而 Redis 的库是**会话状态**，分成两条会让 `SELECT` 失效，
/// 表现是「在 db3 里执行的命令跑到了 db0」这类串库。保持 `None`（跟随连接策略）才是对的。
///
/// 对照：MySQL / PostgreSQL / Mongo / ES 那些**非**单连接池的类型走第二分支，
/// 结构浏览对它们**强制只读**，所以那边的浏览查询要显式 `Some(true)`（见 `documents`）。
const SESSION_BROWSE: &str = "internal:browse";

/// 执行一条命令（Mongo 命令 / Redis 命令 / ES REST 请求）。
pub async fn execute(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params.get("database").unwrap_or_default().to_string();
    let command = body
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if command.trim().is_empty() {
        return Err(XError::bad_request("命令不能为空"));
    }
    let record = require_record(&state, &id).await?;
    // Redis：先把会话切到目标库再发命令，**这一步不能省**。
    // 少了它，命令一律落在连接的默认库上（真机实测：`database=db3` 时 `DBSIZE`
    // 返回的是 db0 的 2），界面看到的正是「库里明明有值，查出来却是 NULL」。
    if record.kind() == ConnectionKind::Redis {
        let target = redis_target_db(record.config.database.as_deref(), &database);
        if let Err(message) = redis_select_db(&state, &id, &target).await {
            return Ok(Json(shape::query_failure_json(&message, 0)));
        }
    }
    let engine = state.engine.clone();
    let execution_id = dbmind_core::DbMindEngine::next_execution_id();
    let request = QueryRequest {
        read_only: None,
        connection: id,
        sql: command,
        options: QueryOptions {
            max_rows: 2000,
            timeout_ms: 120_000,
        },
        execution_id: Some(execution_id),
        // 用户自己发的命令（Redis 命令 / Mongo 命令 / ES REST —— **可能是写**）：
        // 刻意留在独立泳道上（Redis 上是唯一的泳道，因为它是单连接池），
        // 不与只读的结构浏览会话合并 —— 扫的是「隔离写与读」，不是省那一条连接。
        session: Some(SESSION_BROWSE.to_string()),
    };
    Ok(Json(match blocking(move || engine.execute(request, AccessContext::Web)).await {
        Ok(result) => shape::query_result_json(&result),
        Err(err) => shape::query_failure_json(&err.message, 0),
    }))
}

/// 命令该落在哪个库：请求里给了就用它，没给就回落到**连接自己绑定的库**。
fn redis_target_db(configured: Option<&str>, database: &str) -> String {
    match database.trim() {
        "" => configured.unwrap_or("").trim().to_string(),
        given => given.to_string(),
    }
}

/// 把 Redis 会话切到目标库（`SELECT n`）。
///
/// Redis 的「库」不是连接参数，而是**会话状态** —— 宿主注释里写得很清楚：
/// 「一个会话 = 一条 Jedis 长连接 + 当前库（SELECT 会改它）」。所以要在指定库里执行，
/// 必须先 SELECT，且**必须与随后的命令共用同一个会话键**（见 [`SESSION_BROWSE`]），
/// 否则 SELECT 不作用到命令上。
///
/// 三条口径：
/// - 目标是 `dbN`（或 `N`）就照原样传：宿主两种写法都认（`parseDatabase` 会剥掉 `db` 前缀）；
/// - 目标为空（连接没绑库）⇒ 切到 `0`：会话是**长连接**，库状态会留在上面，
///   不显式切回去的话，上一个页签选过的库会漏给下一个请求 —— 查到别的库还以为查对了；
/// - SELECT 失败如实报错（库号越界属于输入问题），不静默忽略。
async fn redis_select_db(state: &AppState, id: &str, database: &str) -> Result<(), String> {
    let target = match database.trim() {
        "" => "0".to_string(),
        given => given.to_string(),
    };
    let engine = state.engine.clone();
    let request = QueryRequest {
        read_only: None,
        connection: id.to_string(),
        sql: format!("SELECT {target}"),
        options: QueryOptions {
            max_rows: 1,
            timeout_ms: 10_000,
        },
        execution_id: None,
        session: Some(SESSION_BROWSE.to_string()),
    };
    blocking(move || engine.execute(request, AccessContext::Web))
        .await
        .map(|_| ())
        .map_err(|err| err.message)
}

/// 取消在途命令（Mongo/ES 宿主是「标志位」语义，Redis 基本是毫秒级）。
pub async fn cancel(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> XResult<Json<Value>> {
    let cancelled = state.engine.cancel(&execution_id);
    Ok(Json(json!({
        "success": true,
        "cancelled": cancelled,
        "message": if cancelled { "已发送取消" } else { "该执行已结束或不存在" },
    })))
}

/// 监控：内核没有（Phase 3），给 `supported:false` 让界面隐藏该页。
pub async fn monitor(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> XResult<Json<Value>> {
    require_record(&state, &id).await?;
    Ok(Json(json!({
        "supported": false,
        "message": "实时监控尚未接入（Phase 3）",
    })))
}
