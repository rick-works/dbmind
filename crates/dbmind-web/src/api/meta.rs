//! `/api/{module}/{id}/…` —— 关系型元数据、表数据与表操作。
//!
//! 这一层是「上游界面 ↔ DBmind 内核」落差最大的地方，处理原则有三条：
//!
//! 1. **能走内核就走内核**。表/列清单用内核实现在走 `DatabaseMetaData` 的版本（各驱动都验过），
//!    比在兼容层重写一遍元数据 SQL 稳。
//! 2. **内核没有的按方言拼 SQL 执行**（索引 / 过程 / 触发器 / 事件 / 用户 / 建表语句）。
//!    这本来是上游Java 后端各方言模块的职责，落点不同、职责相同。方言覆盖率见 `dialect.rs`。
//! 3. **分不清「没有」和「没写」的地方，一律按「没写」报错**。空数组会被界面读成
//!    「这个库里没有索引」，那是另一种坏消息。
//!
//! 分页/关键字/排序这类**界面语义**也在这里翻译成 SQL：内核只认「一条语句」，
//! 没有「给这张表分页」这种概念。

use axum::extract::{Path, RawQuery, State};
use axum::Json;
use dbmind_core::{
    AccessContext, CellValue, ColumnDetail, ConnectionKind, ConnectionRecord, DbMindError,
    ErrorCode, QueryOptions, QueryRequest, QueryResult,
};
use serde_json::{json, Map, Value};

use crate::api::dialect::{Dialect, Meta};
use crate::api::error::{XError, XResult};
use crate::api::{blocking, conn as conn_api, require_record, shape, Params};
use crate::AppState;

/// 元数据查询的默认行上限：元数据是「列表」，给足但不放任。
const META_MAX_ROWS: usize = 5000;

// ------------------------------------------------------------------ 基础工具

pub(crate) async fn ctx_of(state: &AppState, id: &str) -> XResult<(ConnectionRecord, Dialect)> {
    let record = require_record(state, id).await?;
    let dialect = Dialect::new(record.kind());
    Ok((record, dialect))
}

/// 执行一条语句（内部用）。默认行数给足，免得「看一张大表」被截断。
pub async fn run_sql(
    state: &AppState,
    conn: &str,
    sql: String,
    max_rows: usize,
) -> XResult<QueryResult> {
    run_sql_in(state, conn, "", sql, max_rows).await
}

/// 同上，但**切到指定库**执行 —— 跨库浏览全靠它。
///
/// `database` 为空 = 用连接自己绑定的那个库。非空时由 `scope::resolve` 决定是直接用主连接、
/// 还是换成一条指向该库的影子连接；这件事对调用方完全透明。
pub async fn run_sql_in(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
    max_rows: usize,
) -> XResult<QueryResult> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    // 元数据也是「连上去查」：驱动没装就先下（离线时只失败一次，原因会被缓存）
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let engine = state.engine();
    let request = QueryRequest {
        connection: target,
        sql,
        options: QueryOptions {
            max_rows,
            timeout_ms: 120_000,
        },
        execution_id: None,
        // 刻意**不**给会话亲和键，让它复用元数据那条会话。
        //
        // 原来这里写的是 `Some("internal:browse")`，本意是「别和用户页签的会话抢」——
        // 那份隔离是对的（用户查询自带 key，仍互不干扰），但代价被低估了：
        // 亲和键会参与会话键的计算，于是**元数据和浏览落在两条物理会话上**。
        // 实测（真机 MySQL，宿主 trace）：一次 `/tables` 请求建了两条会话 ——
        // 元数据一条、随后的「行数估算」一条（建连 306ms），而它们本来就是同一条库里
        // 同一件事。会话一旦建好就会复用，所以这是**每个连接一次**的白开销。
        //
        // 去掉之后两者共用一条会话（内核本来就是单连接泳道，同一会话上串行执行）。
        // 浏览类语句都是只读 SELECT，不会污染会话状态。
        session: None,
        // 还要把只读标记对齐：结构浏览对这类库走的是**只读会话**
        // （`metadata_session_key` 里 `session_key(config, true)`），而 `execute` 缺省按
        // 连接策略算 —— 标记不一致的话，会话键的第 7 段不同，照样是两条连接。
        // 这里的语句全是只读 SELECT，走只读会话既省一条连接，语义也更正确。
        read_only: Some(true),
        // **不进查询历史**：这条路上跑的全是元数据 / 结构浏览 / 表格预览，
        // 以及界面功能（表结构编辑、清空表、建用户、终止会话…）驱动的语句 ——
        // 都不是「用户在 SQL 编辑器里敲的 SQL」。标记在**这个共用入口**上，
        // 比让几十个调用方各自记得打标记可靠（首页「最近查询」曾被它们淹掉）。
        internal: true,
    };
    blocking(move || engine.execute(request, AccessContext::Web)).await
}

/// 同 [`run_sql_in`]，但**跟随连接的读写策略**（`read_only: None`）——
/// 给界面功能里那些**会写库**的语句用：还原、表结构编辑、清空/删除表、终止会话、建用户。
///
/// 为什么必须分开：`run_sql_in` 服务于结构浏览，走的是**只读会话**（省一条连接、语义也对）；
/// 但 JDBC 的只读是**连到驱动上**的 —— MySQL 会直接拒绝
/// `Connection is read-only. Queries leading to data modification are not allowed`。
/// 还原曾因此全军覆没：连接明明**没开**只读开关，还原的第一句 `drop table if exists`
/// 却被驱动拦下 —— 因为语句被塞进了结构浏览那条只读会话里。
///
/// 同样 `internal: true`（这些是界面功能驱动的语句，不是用户在编辑器里敲的 SQL）。
pub async fn run_write_sql_in(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
    max_rows: usize,
) -> XResult<QueryResult> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let engine = state.engine();
    let request = QueryRequest {
        connection: target,
        sql,
        options: QueryOptions {
            max_rows,
            timeout_ms: 120_000,
        },
        execution_id: None,
        session: None,
        // 跟随连接策略：连接没标只读 ⇒ 可写会话（还原/改结构才落得下去）；
        // 连接标了只读 ⇒ 语句在闸门就被拦下，根本走不到驱动。
        read_only: None,
        internal: true,
    };
    blocking(move || engine.execute(request, AccessContext::Web)).await
}

/// 与 `crate::api::blocking` 干同一件事，唯一区别是**保留内核错误码**。
///
/// 兼容层那个 `blocking` 会把内核错误压成 `XError{status, message}` —— 界面只需要 message
/// （见 `api/error.rs` 的头注），但**重试判据需要错误码**：只有码能区分「连接失效
/// （该重连重试）」和「SQL 写错了（重试没用）」。所以在这一处保留原始错误。
async fn run_kernel<T, F>(task: F) -> dbmind_core::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> dbmind_core::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(task).await {
        Ok(result) => result,
        Err(err) => Err(DbMindError::new(
            ErrorCode::Internal,
            format!("工作线程异常：{err}"),
        )),
    }
}

/// 结构浏览用的方言 SQL：**只读、幂等**，连接失效时重连重试一次。
///
/// 为什么要在这里自己补一次重试：内核的**查询**路径按设计不重试（查询不幂等，响应丢了
/// 可能已经在库里执行过，见 `agent_driver::with_lane` 的模块文档），而本文件里这批
/// 方言 SQL（`dialect.rs` 的 `Meta::Sql`）是**我们自己写死的只读元数据查询**，完全幂等。
///
/// 不补的实测后果（真机 MySQL 抓到的）：`wait_timeout` 把连接切掉之后，
/// ① 「库清单」一次失败就退回单库模式，树上于是出现一个 `(default)` **冒充真库**；
/// ② 索引/过程/触发器等分类整体变空，用户读成「这个库没有索引」—— 又一个「看着像空、
/// 其实断了」。宿主已经在连接失效时摘掉坏会话（见 `dbmind-agent-jdbc` 的
/// `metadataFailure`），所以这里重试的这一次会**真的重连**。
async fn run_meta_sql(
    state: &AppState,
    conn: &str,
    database: &str,
    sql: String,
    max_rows: usize,
) -> XResult<QueryResult> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let engine = state.engine();
    // 与结构浏览共用**同一条只读会话**。
    //
    // 这个函数查的全是只读元数据（库清单 / schema 清单 / 索引·过程·触发器分类 / 库级统计），
    // 而结构浏览对非 connection-scoped 的类型走的是 `session_key(config, true)`。
    // 两边对齐才只有一条物理连接 —— 否则同一库同一件事各占一条（各 ~300ms，见
    // `run_sql_in` 的同类注释与实测）。所以这里既去掉 `internal:browse` 亲和键
    // （它会参与会话键计算，正是分裂的原因），也把只读标记对齐。
    // 另：元数据**不进查询历史**（`internal`），理由见 `run_sql_in`。
    let first = QueryRequest {
        read_only: Some(true),
        connection: target.clone(),
        sql: sql.clone(),
        options: QueryOptions {
            max_rows,
            timeout_ms: 120_000,
        },
        execution_id: None,
        session: None,
        internal: true,
    };
    let engine_first = engine.clone();
    match run_kernel(move || engine_first.execute(first, AccessContext::Web)).await {
        Err(err) if err.code == ErrorCode::ConnConnectFailed => {
            tracing::warn!(connection = %conn, "元数据查询遇到连接失效，重连后重试一次");
            // 重试那次与首次保持完全一致（同只读会话）：会话失效时宿主已摘掉坏会话，
            // 这一次会真的重连
            let retry = QueryRequest {
                read_only: Some(true),
                connection: target,
                sql,
                options: QueryOptions {
                    max_rows,
                    timeout_ms: 120_000,
                },
                execution_id: None,
                session: None,
                internal: true,
            };
            blocking(move || engine.execute(retry, AccessContext::Web))
                .await
                .map_err(Into::into)
        }
        other => other.map_err(Into::into),
    }
}

/// 取一行的 **catalog 名**。
///
/// 与 `first_column_text` 的唯一区别：`show catalogs` 的**第一列是 `CatalogId`**（数字），
/// 名字在第二列 `CatalogName` —— 直接取第一列会得到 `0` / `21275` 这种 ID，
/// 界面上就成了一堆数字节点。Doris 又不支持把 `show` 包成子查询去挑列（实测语法错误），
/// 所以只能在结果里按列名选：先认这几个已知列名，都没有才退回第一列
/// （保住"万一以后换成别的写法"时仍能用）。
fn catalog_column_text(row: &Map<String, Value>) -> String {
    for key in ["CatalogName", "catalog_name", "catalogname", "name"] {
        let text = text_ci(row, key);
        if !text.is_empty() {
            return text;
        }
    }
    first_column_text(row)
}

/// 取一行的第一列文本（各家元数据视图的列名不一致，能按 name 取就按 name 取）。
fn first_column_text(row: &Map<String, Value>) -> String {
    let named = text_ci(row, "name");
    if !named.is_empty() {
        return named;
    }
    match row.values().next() {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// 结果集 → 「列名 → 值」的对象数组（元数据接口都按列名取值）。
pub fn rows_of(result: &QueryResult) -> Vec<Map<String, Value>> {
    let names: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
    result
        .rows
        .iter()
        .map(|raw| {
            let mut row = Map::new();
            for (i, name) in names.iter().enumerate() {
                row.insert(
                    (*name).to_string(),
                    raw.get(i).map(shape::cell_to_value).unwrap_or(Value::Null),
                );
            }
            row
        })
        .collect()
}

pub fn first_text(result: &QueryResult) -> String {
    let row = match result.rows.first() {
        Some(row) => row,
        None => return String::new(),
    };
    match row.first() {
        Some(CellValue::Text(text)) => text.clone(),
        Some(other) => match shape::cell_to_value(other) {
            Value::Null => String::new(),
            Value::String(s) => s,
            other => other.to_string(),
        },
        None => String::new(),
    }
}

fn text_of(row: &Map<String, Value>, key: &str) -> String {
    match row.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// 大小写不敏感地取一个字段（各家元数据视图的列名大小写不一致）。
fn text_ci(row: &Map<String, Value>, key: &str) -> String {
    if row.contains_key(key) {
        return text_of(row, key);
    }
    let lower = key.to_ascii_lowercase();
    for (k, v) in row {
        if k.to_ascii_lowercase() == lower {
            return match v {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
        }
    }
    String::new()
}

/// 按 `Meta` 三档取元数据列表（`database` 非空时切到目标库去查）。
async fn meta_list(
    state: &AppState,
    id: &str,
    database: &str,
    meta: Meta,
    feature: &str,
) -> XResult<Value> {
    match meta {
        Meta::Absent => Ok(Value::Array(vec![])),
        Meta::Unwritten => Err(not_written(state, id, feature).await),
        Meta::Sql(sql) => {
            let result = run_meta_sql(state, id, database, sql, META_MAX_ROWS).await?;
            Ok(Value::Array(
                rows_of(&result).into_iter().map(Value::Object).collect(),
            ))
        }
    }
}

/// 「这个概念存在、但本类型的语句还没写」——报错时把类型名带上，别让人以为是权限问题。
async fn not_written(state: &AppState, id: &str, feature: &str) -> XError {
    let key = require_record(state, id)
        .await
        .map(|r| r.kind().key())
        .unwrap_or("未知");
    XError::not_implemented(&format!("{feature}（{key}）"))
}

/// 库名参数可能是 `库.模式`（上游的 schema 节点就是这么拼的），拆开。
/// 取一个整数列（列名大小写不敏感 —— 有的驱动会把别名转成大写）。
///
/// 返回 `Option` 而不是 `unwrap_or(0)`：这里代表「行数」，而 0 与「取不到」是**两件事** ——
/// 前者是「这张表是空的」，后者是「我不知道」。混成 0 就是「树上每张表都显示 0」那个 bug 的形状。
fn cell_int(row: &Map<String, Value>, key: &str) -> Option<i64> {
    let mut found = row.get(key);
    if found.is_none() {
        let lower = key.to_ascii_lowercase();
        found = row
            .iter()
            .find(|(name, _)| name.to_ascii_lowercase() == lower)
            .map(|(_, value)| value);
    }
    // 转换统一走 shape::value_to_i64 —— 那是"整数 / 浮点 / 字符串"三种形态的唯一处理处。
    // 别在这里再抄一份：抄一份的下场就是别处漏掉其中一种（table_count 的历史）。
    shape::value_to_i64(found?)
}

fn split_db_schema(database: &str, dialect: &Dialect) -> (String, Option<String>) {
    if !dialect.is_schema_aware() {
        return (database.to_string(), None);
    }
    match database.split_once('.') {
        Some((db, schema)) if !schema.is_empty() => (db.to_string(), Some(schema.to_string())),
        _ => (database.to_string(), None),
    }
}

// ------------------------------------------------------------------ 特性与结构清单

/// 各类型的**候选类型清单**（结构编辑器 / 新建表的类型下拉）。
/// 统一口径：**不带死参数** —— 长度 / 精度由编辑器的「长度 / 精度」列给
///（回显已有列时按真实类型拆进两列；新增行对 char 系预填 255）。
/// 特例保留参数：`(max)` 这类编辑器表达不了的形态（SQL Server 的 NVARCHAR(max)）。
fn column_types(kind: ConnectionKind) -> Vec<&'static str> {
    match kind.key() {
        "mysql" | "mariadb" | "doris" => vec![
            "int", "bigint", "smallint", "tinyint", "decimal", "float", "double",
            "varchar", "char", "text", "longtext", "date", "datetime", "timestamp",
            "time", "json", "blob",
        ],
        "postgresql" | "kingbase" => vec![
            "integer", "bigint", "smallint", "numeric", "real", "double precision",
            "character varying", "text", "boolean", "date", "timestamp", "timestamptz",
            "time", "jsonb", "uuid", "bytea",
        ],
        "sqlserver" => vec![
            "int", "bigint", "smallint", "tinyint", "decimal", "float", "real",
            "nvarchar", "varchar", "nvarchar(max)", "bit", "date", "datetime2",
            "datetimeoffset", "uniqueidentifier", "varbinary(max)",
        ],
        "oracle" | "dm" => vec![
            "NUMBER", "FLOAT", "BINARY_FLOAT", "VARCHAR2",
            "NVARCHAR2", "CHAR", "CLOB", "BLOB", "DATE", "TIMESTAMP", "RAW",
        ],
        "clickhouse" => vec![
            // Decimal / FixedString / DateTime64 的参数由「长度 / 精度」列给
            //（重建时 Decimal 缺省 38,6、FixedString 缺省 16、DateTime64 缺省 3，见前端 clickhouseType）
            "UInt8", "UInt16", "UInt32", "UInt64", "Int8", "Int16", "Int32", "Int64",
            "Float32", "Float64", "Decimal", "String", "FixedString",
            "Date", "DateTime", "DateTime64", "UUID", "Array(String)",
        ],
        "sqlite" => vec!["INTEGER", "REAL", "TEXT", "BLOB", "NUMERIC", "VARCHAR", "DATETIME"],
        "h2" => vec!["INT", "BIGINT", "DECIMAL", "DOUBLE", "VARCHAR", "CLOB", "BLOB", "DATE", "TIMESTAMP", "BOOLEAN"],
        "db2" => vec!["INTEGER", "BIGINT", "DECIMAL", "DOUBLE", "VARCHAR", "CLOB", "BLOB", "DATE", "TIMESTAMP"],
        "derby" => vec!["INTEGER", "BIGINT", "DECIMAL", "DOUBLE", "VARCHAR", "CLOB", "BLOB", "DATE", "TIMESTAMP"],
        _ => vec![],
    }
}

/// 方言能力表：界面据此决定显示哪些操作入口。
///
/// `supportsExport` / `supportsImport` 刻意给 **true**：导出导入还没落地（Phase 2），
/// 但**菜单必须照上游的样子在**（界面不变），点下去会拿到一条明确的
/// 「尚未接入」而不是一个不存在的入口 —— 藏在 features 里假装不支持才是更坏的做法。
/// **结构编辑器**需要的方言能力。
///
/// 这几个键不是"锦上添花"，少一个的后果是**整行变只读**：界面拿 `ddlStyle` 决定用哪套
/// ALTER 生成器、拿 `supportsColumnModify` 决定已加载行能不能改。落到 `generic` 分支时，
/// 类型/长度/可空/主键/默认值**全部点不动，而且不给任何解释** ——
/// 用户看到的就是「主键和可空为什么不能选」。
///
/// 诚实性要求：只声明**真的能生成正确 SQL** 的能力。界面会用这些开关直接产出 DDL，
/// 声明得比实现靠前，等于让用户去撞数据库的报错。
fn edit_capabilities(kind: &str) -> Value {
    // (ddlStyle, quoteStyle, 可改已有列, 列注释, 表选项, 自增, 索引重建)
    let (style, quote, modify, comment, table_options, auto_inc, rebuild) = match kind {
        "mysql" | "mariadb" => ("mysql", "BACKTICK", true, true, true, true, true),
        "doris" => ("doris", "BACKTICK", true, true, true, true, true),
        "postgresql" | "kingbase" => ("pg", "DOUBLE", true, true, false, false, true),
        "oracle" | "dm" => ("oracle", "DOUBLE", true, true, false, true, true),
        "sqlserver" => ("mssql", "BRACKET", true, true, false, true, true),
        // SQLite 没有 ALTER COLUMN：只能加列或重建表，如实说不能改
        "sqlite" => ("sqlite", "DOUBLE", false, false, false, false, false),
        "clickhouse" => ("clickhouse", "DOUBLE", true, true, false, false, false),
        // H2：**实测过**才敢开。在 H2 2.3 上逐条验过 pg 风格的
        // `alter table t alter column c type varchar(100)`、`numeric(12,3)`、
        // `set default`、`rename column` 全部可用，列注释 `comment on column` 也可用。
        // 索引重建 / 表选项 H2 没有对应语法，如实给 false。
        "h2" => ("pg", "DOUBLE", true, true, false, false, false),
        // Derby：**实测过**（Derby 10.16）才开。`ALTER COLUMN c SET DATA TYPE …`、
        // `SET DEFAULT …` / `DROP DEFAULT`、`NULL` / `NOT NULL`、`RENAME COLUMN a TO b`、
        // `ADD COLUMN`、`DROP COLUMN` 逐条验过可用（前端 `derby` 这一套生成器就是照它写的）。
        // 列注释 Derby 没有（comment=false）；表选项/自增/索引重建也没对应语法，如实给 false。
        "derby" => ("derby", "DOUBLE", true, false, false, false, false),
        // db2 与非关系型：界面按「不支持在线改结构」处理（没实测过就不声明 ——
        // 声明得比实现靠前，等于让用户去撞数据库的报错）
        _ => ("generic", "DOUBLE", false, false, false, false, false),
    };
    // (字符集清单, 字符集 → 排序规则映射)
    let (charsets, collations): (Vec<&str>, Value) = match kind {
        "mysql" | "mariadb" | "doris" => (
            vec![
                "utf8mb4", "utf8mb3", "utf8", "latin1", "gbk", "gb18030", "big5", "ascii", "binary",
                "ucs2", "utf16", "utf16le", "utf32",
            ],
            json!({
                "utf8mb4": ["utf8mb4_general_ci", "utf8mb4_unicode_ci", "utf8mb4_0900_ai_ci", "utf8mb4_bin"],
                "utf8mb3": ["utf8mb3_general_ci", "utf8mb3_unicode_ci", "utf8mb3_bin"],
                "utf8": ["utf8_general_ci", "utf8_unicode_ci", "utf8_bin"],
                "latin1": ["latin1_swedish_ci", "latin1_general_ci", "latin1_bin"],
                "gbk": ["gbk_chinese_ci", "gbk_bin"],
                "gb18030": ["gb18030_chinese_ci", "gb18030_bin"],
                "big5": ["big5_chinese_ci", "big5_bin"],
                "ascii": ["ascii_general_ci", "ascii_bin"],
                "binary": ["binary"],
            }),
        ),
        _ => (Vec::new(), json!({})),
    };
    json!({
        "ddlStyle": style,
        "quoteStyle": quote,
        "supportsColumnModify": modify,
        "supportsComment": comment,
        "supportsTableOptions": table_options,
        "supportsAutoIncrement": auto_inc,
        "supportsIndexRebuild": rebuild,
        "doris": kind == "doris",
        // 字符集 / 排序规则：MySQL 系的取值是服务端标准集合（utf8mb4 这些版本间很稳定），
        // 这里给常青名单；其它方言（没有「表级字符集」这个概念）如实给空。
        // 前端下拉带 `allow-create`：名单外的值照样能手输，
        // 所以给一份有用的名单比给空数组好 —— 空数组会让整行变成纯手输。
        // （`collations` 只需覆盖常见字符集：查不到对应项的字符集在前端会退化成输入框。）
        "charsets": charsets,
        "collations": collations,
    })
}

async fn features_json(state: &AppState, id: &str) -> XResult<Value> {
    let (record, dialect) = ctx_of(state, id).await?;
    let kind = record.kind();
    // 能力探测**只认「有 SQL」**，不能只判「没落 Unwritten」：
    // `Meta::Absent` 表示该类型**根本没有这个概念**（SQL Server 没有事件、SQLite 没有存储过程、
    // ClickHouse / H2 没有触发器），而 `Unwritten` 表示概念在、只是还没写。
    // 用 `!Unwritten` 会把 Absent 也判成 true —— 实测 SQL Server 报 `events=true`、
    // SQLite 报 `procs=true`，树于是画出了这些分类：要么恒为 0，要么点进去直接报「未实现」。
    // 这两种情况在界面上应当是**同一个结论**：别显示。
    let has = |meta: Meta| matches!(meta, Meta::Sql(_));
    let index_ok = has(dialect.indexes());
    let proc_ok = has(dialect.procedures());
    let trigger_ok = has(dialect.triggers());
    let event_ok = has(dialect.events());
    let user_ok = has(dialect.users());
    let ddl_ok = has(dialect.ddl("__probe__"));
    // 「能列出用户」和「能看详情 / 能改」是两档能力：
    // Oracle / DM / ClickHouse 有用户列表，但 user_info / user_action_sql 未实现 ——
    // 树里有用户节点、右键「查看 / 编辑 / 删除」却必然 501。
    let user_detail_ok = has(dialect.user_info("__probe__"));
    let user_manage_ok = has(dialect.user_action_sql(
        "create",
        "__probe__",
        "",
        "%",
        &[],
        &crate::api::dialect::UserPrivileges::default(),
        false,
    ));
    // 各类对象「查看 / 编辑」靠 `object_source`（取定义）；没实现的那几类
    // （ClickHouse / DB2 / H2 / Derby）点下去必然 501 —— 界面据此不摆这两个菜单项。
    let object_ddl_kinds: Vec<&str> = ["view", "procedure", "function", "trigger", "event"]
        .into_iter()
        .filter(|object_kind| has(dialect.object_source(object_kind, "__probe__")))
        .collect();
    // 结构编辑相关的能力挂在同一份 payload 上（界面一次性取，避免多一次往返）
    let edit = edit_capabilities(kind.key());
    Ok(json!({
        "type": kind.key().to_ascii_uppercase(),
        "columnTypes": column_types(kind),
        "indexTypes": Vec::<&str>::new(),
        // 存储引擎：MySQL 系的标准引擎是服务端常量清单（InnoDB 等一直是这些名字）；
        // 其它方言没有这个概念，如实给空数组。前端下拉带 `allow-create`，
        // 清单外的值照样能手输 —— 所以宁可给一份有用的小名单，也不给空数组
        // （空数组会让「存储引擎」整行退化成纯手输）。
        "engines": match kind.key() {
            "mysql" | "mariadb" | "doris" => vec![
                "InnoDB", "MyISAM", "MEMORY", "CSV", "ARCHIVE", "BLACKHOLE", "MRG_MYISAM", "FEDERATED",
            ],
            _ => Vec::new(),
        },
        "supportsDatabases": true,
        "supportsSchemas": dialect.is_schema_aware(),
        "supportsIndexes": index_ok,
        "supportsProcedures": proc_ok,
        "supportsTriggers": trigger_ok,
        "supportsEvents": event_ok,
        "supportsUsers": user_ok,
        // 用户「详情 / 管理」是另一档能力（Oracle/DM/ClickHouse 只有列表）
        "supportsUserDetail": user_detail_ok,
        "supportsUserManage": user_manage_ok,
        // 哪些对象类型能取到定义（= 能「查看 / 编辑」）
        "objectDdlKinds": object_ddl_kinds,
        // 监控按方言实现（见 `api::monitor`）：已接入的给真数据，
        // 没接入的监控接口会返回 supported:false + 一句原因，界面照这句话显示
        "supportsMonitor": crate::api::monitor::supported(kind.key()),
        "supportsExport": true,
        "supportsImport": true,
        "supportsTableData": true,
        "supportsDdl": ddl_ok,
        // 「能不能**执行** DDL」与「能不能**取回**建表语句」是两件事：
        // DROP / TRUNCATE / 改名 这类操作所有 SQL 类都能跑（Derby、DB2 取不到 SHOW CREATE，
        // 但 DROP TABLE 完全可用），NoSQL 类则完全没有这个概念。界面按它门控**写操作**，
        // 不能拿 supportsDdl（= 取定义）当依据 —— 那会把 Derby / DB2 的删除表整个藏掉。
        "supportsDDLExec": kind.is_jdbc() || kind.key() == "sqlite",
        "supported": kind.implemented(),
        // 结构编辑器要的键（ddlStyle / quoteStyle / supportsColumnModify / ...）——
        // 展开在顶层，界面按扁平键读取
        "ddlStyle": edit["ddlStyle"],
        "quoteStyle": edit["quoteStyle"],
        "supportsColumnModify": edit["supportsColumnModify"],
        "supportsComment": edit["supportsComment"],
        "supportsTableOptions": edit["supportsTableOptions"],
        "supportsAutoIncrement": edit["supportsAutoIncrement"],
        "supportsIndexRebuild": edit["supportsIndexRebuild"],
        "doris": edit["doris"],
        "charsets": edit["charsets"],
        "collations": edit["collations"],
    }))
}

/// `POST /api/{m}/test` —— 测试连接。
///
/// 直接拿**表单里的那份配置**去连（内核 `test_config`），**不往连接库里写任何东西**。
///
/// 早前这里是「建一条临时连接 → 测 → 删掉」（那时内核只有按 id 自检的接口）。那条路有个
/// 不能接受的失败模式：删除一旦失败（进程被杀、请求中断、驱动占用），临时连接就**永久留在
/// 用户的连接列表里** —— 对象浏览器里凭空多出几行 `名字 (probe exec_…)`，用户从没建过它们，
/// 却得自己琢磨"这是什么"。改成不落库之后，这种残留从机制上不可能再出现。
pub async fn test(State(state): State<AppState>, Json(mut body): Json<Value>) -> XResult<Json<Value>> {
    // 测试连通性不该先要求起个名字，但连接校验要求 name 非空 —— 缺了就补个占位名。
    // 这个名字只用于让校验通过与错误文案可读，**不会被持久化**。
    let name_missing = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .is_empty();
    if name_missing {
        if let Some(object) = body.as_object_mut() {
            object.insert(
                "name".to_string(),
                json!(format!("probe-{}", dbmind_core::new_execution_id())),
            );
        }
    }
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // 已保存的连接：口令拿不到明文，留空时用服务端存着的那份补齐；
    // 只读标记同理 —— 它是**连接记录**上的属性（`ConnectionRecord.read_only`），不在配置里。
    //
    // **必须先取记录、再构造配置**：SSH 口令与私钥口令短语同样不回显，客户端交回来的
    // 是空串，「空 = 沿用已存的那份」要靠 `existing` 里的 `extra.ssh` 才能判定。
    // 顺序反了的话，编辑一条已配 SSH 的连接、不动口令直接点「测试连接」，
    // 会得到一句「SSH 口令是空的」——而用户明明没改过它。
    let existing = if id.is_empty() {
        None
    } else {
        require_record(&state, &id).await.ok()
    };
    let mut config = conn_api::config_from_body(
        &body,
        existing.as_ref().and_then(|record| record.config.extra.as_ref()),
    )?;
    let mut read_only = false;
    if let Some(record) = &existing {
        if config.password.is_none() {
            config.password = record.config.password.clone();
        }
        read_only = record.read_only;
    }
    // 驱动没装就先把驱动下下来：连接弹窗上写着「首次连接将自动从 Maven 中心下载驱动」，
    // 这句话必须是真的 —— 否则用户点「测试连接」只会看到「驱动未就绪」，像是坏了。
    if let Err(err) = crate::api::driver::ensure_installed(&state, config.kind).await {
        return Ok(Json(json!({ "success": false, "message": err.message })));
    }

    let engine = state.engine();
    let tested = blocking(move || engine.test_config(&config, read_only)).await;

    Ok(Json(match tested {
        Ok(report) => json!({
            "success": true,
            "latencyMs": report.latency_ms,
            "serverVersion": report.server_version.unwrap_or_default(),
            "runtimeMode": report.runtime_mode.as_str(),
            "message": report.message,
        }),
        // 注意别在这里再 with_hint 一次：`blocking` 出来的错误已经过 `XError::from`，
        // 提示会重复追加两遍（实测出现过）。
        Err(err) => json!({ "success": false, "message": err.message }),
    }))
}

pub async fn features(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> XResult<Json<Value>> {
    Ok(Json(features_json(&state, &id).await?))
}

/// 库清单：**服务器上当前账号有权限访问的全部库**（文件型就是它自己那一个）。
///
/// 连接里的「数据库名称」因此可以留空 —— 留空表示「连到服务器、不指定默认库」，
/// 库列表照样能列出来（见 `conn.rs` 的 `database_value`）。
/// catalog 清单（只有 catalog 层级的类型有：目前是 Doris）。
///
/// **其余类型一律返回空数组** —— 前端见空就不画 catalog 这一层，树的形状与现在完全一样。
/// 用「返回空」而不是「不提供接口」，是为了让前端只有一条判断：有没有 catalog。
pub async fn catalogs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> XResult<Json<Value>> {
    let (_, dialect) = ctx_of(&state, &id).await?;
    if let Meta::Sql(sql) = dialect.catalogs() {
        if let Ok(result) = run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await {
            let names: Vec<String> = rows_of(&result)
                .iter()
                .map(catalog_column_text)
                .filter(|name| !name.trim().is_empty())
                .collect();
            if !names.is_empty() {
                return Ok(Json(json!(names)));
            }
        }
    }
    Ok(Json(json!([])))
}

pub async fn databases(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let (record, dialect) = ctx_of(&state, &id).await?;
    // catalog 层级（Doris）：库挂在 catalog 下面，`?catalog=xxx` 时列的是**那个 catalog 的库**。
    // 走 `<catalog>.information_schema.schemata`，不需要切会话（切会话是状态，容易串）。
    let params = Params::parse(raw.as_deref());
    let catalog = params.get("catalog").unwrap_or_default();
    if !catalog.trim().is_empty() {
        if let Meta::Sql(sql) = dialect.databases_in_catalog(catalog.trim()) {
            if let Ok(result) = run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await {
                let names: Vec<String> = rows_of(&result)
                    .iter()
                    .map(first_column_text)
                    .filter(|name| !name.trim().is_empty())
                    // Doris 2.1 的库列表会把**内置目录名（internal）**混进来 —— 它不是库
                    //（访问报 Unknown database），树里会多出一个空壳「internal」节点，剔除
                    .filter(|name| !(dialect.catalog_level() && name.eq_ignore_ascii_case(catalog.trim())))
                    .collect();
                if !names.is_empty() {
                    return Ok(Json(json!(names)));
                }
            }
            // 列不出来就当这个 catalog 没有库（而不是退回"所有库"，那会串到别的 catalog）
            return Ok(Json(json!([])));
        }
    }
    if let Meta::Sql(sql) = dialect.databases() {
        match run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await {
            Ok(result) => {
                let names: Vec<String> = rows_of(&result)
                    .iter()
                    .map(first_column_text)
                    .filter(|name| !name.trim().is_empty())
                    // 同上：catalog 方言下剔除与目录同名的泄漏条目（默认目录就是 internal）
                    .filter(|name| !(dialect.catalog_level() && name.eq_ignore_ascii_case("internal")))
                    .collect();
                if !names.is_empty() {
                    return Ok(Json(json!(names)));
                }
                tracing::warn!(connection = %id, "库清单查询返回空，退回单库模式");
            }
            Err(err) => {
                // 列不出来（权限不足 / 方言差异）不该让整棵树空掉：退回「连接绑定的那个库」，
                // 并把原因写进日志 —— 界面至少还能用。
                tracing::warn!(connection = %id, error = %err.message, "库清单查询失败，退回单库模式");
            }
        }
    }
    let name = record
        .config
        .database
        .clone()
        .filter(|database| !database.is_empty())
        .unwrap_or_else(|| {
            if record.kind().local_file() {
                "main".to_string()
            } else {
                "(default)".to_string()
            }
        });
    Ok(Json(json!([name])))
}

pub async fn schemas(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params.get("database").unwrap_or_default();
    let (_, dialect) = ctx_of(&state, &id).await?;
    match dialect.schemas() {
        Meta::Absent => Ok(Json(json!([]))),
        Meta::Unwritten => Err(not_written(&state, &id, "schema 清单").await),
        Meta::Sql(sql) => {
            // schema 清单是**某个库内部**的概念（SQL Server 的 sys.schemas、PostgreSQL 的
            // pg_namespace 都是按库查的），所以要切到目标库去查
            let result = run_meta_sql(&state, &id, &database, sql, META_MAX_ROWS).await?;
            let names: Vec<String> = rows_of(&result)
                .iter()
                .map(first_column_text)
                .filter(|name| !name.trim().is_empty())
                .collect();
            Ok(Json(json!(names)))
        }
    }
}

/// 表 / 视图清单。
///
/// 两条路径：schema 层级（PostgreSQL / SQL Server / KingbaseES）按 schema 查；
/// 其余类型走内核的 `DatabaseMetaData`（各驱动都验过，比在这里重写一遍稳）。
pub async fn tables(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let (_, dialect) = ctx_of(&state, &id).await?;
    let database = params.get("database").unwrap_or_default();
    let (_, schema_in_db) = split_db_schema(&database, &dialect);
    let schema = params.get("schema").filter(|s| !s.is_empty()).or(schema_in_db);

    // catalog 层级（Doris）：树上「库」的名字是 `catalog.库`，对象查 <catalog>.information_schema，
    // 不需要也不能切会话 —— JDBC URL 表达不了 catalog，硬塞进 database 会把 URL 弄坏。
    if dialect.catalog_level() {
        if let Some((catalog, db)) = database.split_once('.') {
            if let Meta::Sql(sql) = dialect.tables_in_catalog(catalog, db) {
                // 库名传**空**：SQL 已经用 `<catalog>.information_schema` 全限定了，
                // 不需要切库；而把 `internal.ods` 传下去会去建一条 database='internal.ods'
                // 的影子连接 —— JDBC URL 变成 `.../internal.ods`，Doris 直接拒绝（实测 500）。
                let result = run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await?;
                let tables: Vec<Value> = rows_of(&result)
                    .iter()
                    .map(|row| {
                        let kind = text_ci(row, "type");
                        json!({
                            "schema": db,
                            "name": text_ci(row, "name"),
                            "type": if kind.to_ascii_uppercase().contains("VIEW") { "VIEW" } else { "TABLE" },
                            "comment": Value::Null,
                            // Doris 的 TABLE_ROWS 多是估算甚至 0，这里不猜：留空，
                            // 由前端的批量 COUNT(*) 回填（见 /table-count）
                            "rows": Value::Null,
                            "engine": Value::Null,
                            "charset": Value::Null,
                            "createTime": Value::Null,
                            "columns": [],
                        })
                    })
                    .collect();
                let mut payload = Value::Array(tables);
                // 表选项回显（引擎 / 排序规则 / **表注释**）：上面那条 `tables_in_catalog`
                // 只回名字与类型，注释得单独查。不补的后果与 schema 层级那条一样 ——
                // 注释改完保存**确实成功**，重开表却还是空的 ⇒ 看着就是"修改不生效"。
                if let Meta::Sql(sql) = dialect.table_options_in_catalog(catalog, db) {
                    // 库名同样传空：SQL 已经用 `<catalog>.information_schema` 全限定（同上）
                    if let Ok(result) = run_meta_sql(&state, &id, "", sql, META_MAX_ROWS).await {
                        shape::merge_table_options(&mut payload, &rows_of(&result));
                    }
                }
                return Ok(Json(payload));
            }
        }
    }

    if let (Some(schema), true) = (schema.as_deref(), dialect.is_schema_aware()) {
        let meta = dialect.tables_in_schema(schema);
        let result = match meta {
            Meta::Sql(sql) => run_meta_sql(&state, &id, &database, sql, META_MAX_ROWS).await?,
            _ => return Err(not_written(&state, &id, "按 schema 列出表").await),
        };
        let tables: Vec<Value> = rows_of(&result)
            .iter()
            .map(|row| {
                let kind = text_ci(row, "type");
                json!({
                    "schema": schema,
                    "name": text_ci(row, "name"),
                    "type": if kind.to_ascii_uppercase().contains("VIEW") { "VIEW" } else { "TABLE" },
                    "comment": Value::Null,
                    // 行数来自方言查询（SQL Server 是 sys.partitions 的**真实**行数，
                    // PG/Kingbase 是 reltuples 估算），取不到就是 null —— 界面上留空。
                    // 以前这里写死 0：schema 型数据库（SQL Server/PG/Kingbase）的树上
                    // 每张表都是 0，和「真的是空表」长得一模一样。
                    "rows": cell_int(row, "rows"),
                    "engine": Value::Null,
                    "charset": Value::Null,
                    "createTime": Value::Null,
                    "columns": [],
                })
            })
            .collect();
        let mut payload = Value::Array(tables);
        // 表选项回显（**表注释**）：`tables_in_schema` 那条 SQL 不带注释，而这一层正是
        // 最需要补的 —— SQL Server 的表注释存在 `sys.extended_properties` 里
        // （mssql-jdbc **不填 REMARKS**，走 JDBC 的 getTables 永远拿不到），
        // PG / Kingbase 的表注释在 `obj_description()` 里，同样不在列清单里。
        // 不补的实测后果：注释改完保存**确实成功**（库里值变了），重开表却还是空的
        // ⇒ 用户看到"修改表注释没用"。方言 SQL 取不到就保持原样，不影响列表本身。
        if let Meta::Sql(sql) = dialect.table_options(schema) {
            if let Ok(result) = run_meta_sql(&state, &id, &database, sql, META_MAX_ROWS).await {
                shape::merge_table_options(&mut payload, &rows_of(&result));
            }
        }
        return Ok(Json(payload));
    }

    // 表数据/元数据都按目标库查：切库靠影子连接（见 `scope`），这里只负责把库名传下去
    let target = crate::api::scope::resolve(&state, &id, &database).await?;
    let engine = state.engine();
    let mut tables = blocking(move || engine.list_tables_fresh(&target)).await?;
    // 行数：先给**估算值**（一次查询拿全库，不扫表），精确值由前端按需回填（`/table-count`）。
    // 内核的 `TableInfo.row_estimate` 各驱动都没填，所以这里按方言补一次；
    // 拿不到就保持 null —— 树上不显示数字，而不是显示一个 0 冒充「空表」。
    if let Meta::Sql(sql) = dialect.table_rows(&database) {
        if let Ok(result) = run_sql_in(&state, &id, &database, sql, META_MAX_ROWS).await {
            let mut estimates: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
            for row in rows_of(&result) {
                let name = text_ci(&row, "table_name");
                if name.is_empty() {
                    continue;
                }
                if let Some(count) = cell_int(&row, "rows") {
                    estimates.insert(name.to_ascii_lowercase(), count);
                }
            }
            for table in tables.iter_mut() {
                if let Some(count) = estimates.get(&table.name.to_ascii_lowercase()) {
                    table.row_estimate = Some(*count);
                }
            }
        }
    }
    // 表选项回显（引擎 / 排序规则 / 表注释）：内核的 `TableInfo` 只有名字与行数，
    // 而界面的「基本信息」要显示**这张表真实的**表选项 —— 不回填的话前端只能回退到
    // 「能力清单的第一项」，看着像回显、其实是默认值（表是 MyISAM 也显示 InnoDB，
    // 改别的项保存时会顺手把引擎改掉；表注释则永远是空的）。
    // 取不到就保持原样（null，界面回退到默认），不影响列表本身。
    let mut payload = shape::tables_json(&tables);
    // schema 层级的类型在没有 schema 时（库名里没有点）也会落到这条路上，那时方言 SQL
    // 要的是**模式名**（PG 的 `n.nspname`），给库名只会查不到 —— 取不到就当没有，
    // 不会比原来更差。
    let options_scope = schema.as_deref().unwrap_or(database.as_str());
    if let Meta::Sql(sql) = dialect.table_options(options_scope) {
        if let Ok(result) = run_sql_in(&state, &id, &database, sql, META_MAX_ROWS).await {
            shape::merge_table_options(&mut payload, &rows_of(&result));
        }
    }
    Ok(Json(payload))
}

/// 批量真实行数（`SELECT COUNT(*)`）：树里先用估算值渲染，随后异步回填精确值。
pub async fn table_count(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params.get("database").unwrap_or_default();
    let mut names = params.all("tables");
    if names.is_empty() {
        names = params.all("tables[]");
    }
    let (_, dialect) = ctx_of(&state, &id).await?;
    let mut out = Map::new();
    for name in names {
        if name.trim().is_empty() {
            continue;
        }
        let sql = dialect.count_sql(&name);
        if let Ok(result) = run_sql_in(&state, &id, &database, sql, 1).await {
            if let Some(row) = rows_of(&result).first() {
                // 取值走 cell_int（认整数 / 浮点 / 字符串三种形态）。`count_sql` 统一把结果列
                // 命名为 `cnt`，按列名取比"取第一个值"更稳；取不到才退回第一个值。
                //
                // ⚠️ 取不到时**整项跳过**，绝不写 0：0 表示「这张表是空的」，
                // 而"取不到"是「我不知道」—— 界面上是"显示 0"与"不显示"的区别。
                // 实测：以前这里是 `as_i64().unwrap_or(0)`，而 ClickHouse 的 count 回来是
                // 浮点（Real），取不出来 → 那 5 张表（真值 4 / 216 / 95000 / 0 / 0）
                // 在树上一律显示 0。
                let count = cell_int(row, "cnt")
                    .or_else(|| row.values().find_map(shape::value_to_i64));
                if let Some(count) = count {
                    out.insert(name, json!(count));
                }
            }
        }
        // 单张表统计失败（权限/视图）不该让整次回填失败
    }
    Ok(Json(Value::Object(out)))
}

/// `GET /api/{m}/{id}/stats` —— 库级统计（表数 + 占用空间）。
///
/// 前端当前并不调它（上游里同样是「后端有、前端不用」），但形状保持一致：
/// 键名统一成 `tables` / `sizeText`，其余由方言自定义的键原样带出去。
/// 拿不到信息时给 `—` 而不是 0 —— 0 会被读成「这个库是空的」。
pub async fn stats(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params.get("database").unwrap_or_default();
    let (_, dialect) = ctx_of(&state, &id).await?;
    let sql = match dialect.database_stats() {
        Meta::Sql(sql) => sql,
        _ => return Err(not_written(&state, &id, "库级统计").await),
    };
    let result = run_meta_sql(&state, &id, &database, sql, 2).await?;
    let row = rows_of(&result).into_iter().next().unwrap_or_default();
    let mut out = Map::new();
    for (key, value) in row {
        // 键名统一小写：H2 之类会把结果列名大写返回（`TABLES`），
        // 于是下面的 `entry("tables")` 认不出它、又插入一个默认的 0 ——
        // 界面就看到「2 张表」被显示成 0（实测）
        out.insert(key.to_ascii_lowercase(), value);
    }
    out.entry("tables".to_string()).or_insert(json!(0));
    out.entry("sizeText".to_string()).or_insert(json!("—"));
    Ok(Json(Value::Object(out)))
}

pub async fn columns(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {    let params = Params::parse(raw.as_deref());
    let table = params
        .get("table")
        .ok_or_else(|| XError::bad_request("缺少 table 参数"))?;
    let database = params.get("database").unwrap_or_default();
    let target = crate::api::scope::resolve(&state, &id, &database).await?;
    let engine = state.engine();
    // 表名要进闭包（移动），而下面补类型时还要再用一次，所以先复制一份
    let queried = table.clone();
    let mut columns = blocking(move || engine.list_columns_fresh(&target, &queried)).await?;
    let extras = enrich_column_types(&state, &id, &database, &table, &mut columns).await;
    let mut payload = shape::columns_json(&columns);
    // 注释 / 自增 / 额外属性：内核的列元数据里没有这几位，
    // 而 `shape::column_json` 之前把 `comment` 写死 null、`autoIncrement` 写死 false ——
    // 于是界面**永远认不出已有的自增列与列注释**。这里用方言元数据覆盖掉那几个占位值。
    if let Some(list) = payload.as_array_mut() {
        for item in list.iter_mut() {
            let Some(object) = item.as_object_mut() else {
                continue;
            };
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            let Some(extra) = extras.get(&name) else {
                continue;
            };
            if let Some(comment) = &extra.0 {
                if !comment.trim().is_empty() {
                    object.insert("comment".to_string(), json!(comment));
                }
            }
            if extra.1 {
                // 界面对 MySQL 走 `extra` 匹配、对其它方言走 `autoIncrement` 布尔，
                // 两个都填上，省得两边各判一次
                object.insert("autoIncrement".to_string(), json!(true));
                object.insert("extra".to_string(), json!("auto_increment"));
            }
        }
    }
    Ok(Json(payload))
}

/// 结果集的**列注释**（`列名(小写) → 注释`）：SQL 编辑器对单表查询在表头第二行显示
/// 字段注释 —— 各方言的注释 SQL 由 `dialect.table_comments` 出（MySQL/Doris 走
/// information_schema、PG/Kingbase 走 pg_description、SQL Server 走 extended_properties、
/// Oracle/DM 走 all_col_comments、ClickHouse 走 system.columns……），这里只做转发；
/// 方言不支持列注释时返回空表（前端不渲染注释行）。
pub async fn column_comments(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let table = params
        .get("table")
        .ok_or_else(|| XError::bad_request("缺少 table 参数"))?;
    let database = params.get("database").unwrap_or_default();
    let map = table_comments_map(&state, &id, &database, &table).await;
    Ok(Json(json!(map)))
}

/// 一张表的**列注释**：`列名(小写) → 注释`（取不到就是空表，不编造）。
///
/// 内核的 `ColumnDetail` 不带注释 —— 注释只在方言的元数据查询里取得到，所以按需取一次。
/// **数据同步的「目标表不存在时自动建表」用它**：把源表的字段注释带到目标表建表语句里
/// （以前这条链路一个注释都不带，目标表建出来注释全空，还得人工补一遍）。
pub async fn table_comments_map(
    state: &AppState,
    conn: &str,
    database: &str,
    table: &str,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let Ok(record) = require_record(state, conn).await else {
        return out;
    };
    let dialect = Dialect::new(record.kind());
    let Some(sql) = dialect.table_comments(table).sql() else {
        return out;
    };
    let Ok(result) = run_sql_in(state, conn, database, sql, META_MAX_ROWS).await else {
        return out;
    };
    for row in rows_of(&result) {
        let name = text_ci(&row, "column_name");
        let comment = text_ci(&row, "comment");
        if !name.is_empty() && !comment.is_empty() {
            out.insert(name.to_ascii_lowercase(), comment);
        }
    }
    out
}

/// 一张表的**表注释**（取不到给 `None`）。同 `table_comments_map`，供同步建表复用。
pub async fn table_comment_of(
    state: &AppState,
    conn: &str,
    database: &str,
    table: &str,
) -> Option<String> {
    let record = require_record(state, conn).await.ok()?;
    let dialect = Dialect::new(record.kind());
    let Meta::Sql(sql) = dialect.table_options(database) else {
        return None;
    };
    let result = run_sql_in(state, conn, database, sql, META_MAX_ROWS).await.ok()?;
    for row in rows_of(&result) {
        if text_ci(&row, "table_name").eq_ignore_ascii_case(table) {
            let comment = text_ci(&row, "comment");
            if !comment.is_empty() {
                return Some(comment);
            }
        }
    }
    None
}

/// 用方言自己的元数据把列类型补成**带长度/精度的完整文本**（`nvarchar` → `nvarchar(50)`）。
///
/// 界面靠这段文本拆出「长度/精度」两个输入框，并且在生成 ALTER 时直接用它 ——
/// 少了参数，改这一行的任何属性都会产出 `ALTER COLUMN c nvarchar NULL` 这种
/// 「没长度」的语句（SQL Server / MySQL / Oracle 上会报错或把列变成 1 字符宽）。
///
/// **失败不改行为**：拿不到就保留内核给的类型名。为了一条增强元数据把「看列」这件事
/// 整个弄失败是本末倒置；拿不到时的风险由前端那道「类型需要长度但没填」的校验挡住。
pub(crate) async fn enrich_column_types(
    state: &AppState,
    id: &str,
    database: &str,
    table: &str,
    columns: &mut [ColumnDetail],
) -> std::collections::HashMap<String, (Option<String>, bool)> {
    // 列名(小写) → (注释, 是否自增)
    let mut extras: std::collections::HashMap<String, (Option<String>, bool)> =
        std::collections::HashMap::new();
    let Ok(record) = require_record(state, id).await else {
        return extras;
    };
    let dialect = Dialect::new(record.kind());
    let Some(sql) = dialect.column_types(table).sql() else {
        return extras;
    };
    let Ok(result) = run_sql_in(state, id, database, sql, 1000).await else {
        return extras;
    };
    let mut by_name: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for row in rows_of(&result) {
        // **先把结果列名折成小写**：有些库把结果列名大写返回（H2 就是 `COLUMN_NAME`），
        // 而下面按 `column_name` / `type_text` / `comment` 取 —— 大小写对不上时
        // `row.get` 静默返回 None，整张表的「类型带长度 + 列注释 + 自增」补全全部失效
        // （实测：H2 上 `/columns` 只给 `CHARACTER VARYING`，注释也丢）。
        // 不用逐键改 `text_ci`：这里把这一行的键统一折小写，后面所有取值都不用再想大小写。
        let row: Map<String, Value> = row
            .into_iter()
            .map(|(key, value)| (key.to_ascii_lowercase(), value))
            .collect();
        let name = row
            .get("column_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        if name.is_empty() {
            continue;
        }
        let text = row
            .get("type_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        // 同名取第一条：SQL 里已按「默认 schema 优先」排序
        if !text.is_empty() {
            by_name.entry(name.clone()).or_insert(text);
        }
        let comment = row
            .get("comment")
            .and_then(Value::as_str)
            .map(str::to_string);
        // `is_auto` 各家给的是 0/1（布尔在某些方言上会被当成数字），两种都认
        let is_auto = row
            .get("is_auto")
            .map(|value| match value {
                Value::Bool(flag) => *flag,
                Value::Number(number) => number.as_i64().unwrap_or(0) != 0,
                Value::String(text) => {
                    let t = text.trim().to_ascii_lowercase();
                    t == "1" || t == "true" || t == "yes"
                }
                _ => false,
            })
            .unwrap_or(false);
        if comment.as_deref().map(|c| !c.trim().is_empty()).unwrap_or(false) || is_auto {
            extras.insert(name, (comment, is_auto));
        }
    }
    for column in columns.iter_mut() {
        let key = column.name.to_lowercase();
        if let Some(text) = by_name.get(&key) {
            column.type_name = Some(text.clone());
        }
        // 自增位一并回填：跨类型同步建表要靠它还原 auto_increment / serial / identity
        if let Some((_, is_auto)) = extras.get(&key) {
            column.auto_increment = *is_auto;
        }
    }
    extras
}

/// 库内对象搜索：表名命中 + 列名命中。
///
/// 返回形状按界面取值定：`tables` 是**字符串数组**（表名），
/// `columns` 是 `{table, column}` 对象数组（见 `DbSearchDialog.vue`）。
/// 列名搜索要在每张表上取一次列，所以有表数上限 —— 几百张表的库不该把浏览器拖死。
pub async fn search_objects(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    const MAX_TABLES: usize = 60;
    const MAX_COLUMNS: usize = 200;
    let params = Params::parse(raw.as_deref());
    let keyword = params.get("keyword").unwrap_or_default().to_lowercase();
    if keyword.trim().is_empty() {
        return Ok(Json(json!({ "tables": [], "columns": [] })));
    }

    let database = params.get("database").unwrap_or_default();
    let target = crate::api::scope::resolve(&state, &id, &database).await?;
    let engine = state.engine();
    let connection = target.clone();
    let tables = blocking(move || engine.list_tables_fresh(&connection)).await?;
    let hit_tables: Vec<String> = tables
        .iter()
        .filter(|t| t.name.to_lowercase().contains(&keyword))
        .map(|t| t.name.clone())
        .collect();

    let mut columns: Vec<Value> = Vec::new();
    for table in tables.iter().take(MAX_TABLES) {
        if columns.len() >= MAX_COLUMNS {
            break;
        }
        let engine = state.engine();
        let name = table.name.clone();
        let connection = target.clone();
        let cols = match blocking(move || engine.list_columns(&connection, &name)).await {
            Ok(cols) => cols,
            // 单张表拉列失败（权限/视图）不该让整次搜索失败
            Err(_) => continue,
        };
        for column in cols {
            if column.name.to_lowercase().contains(&keyword) {
                columns.push(json!({
                    "table": table.name,
                    "column": column.name,
                    "type": column.type_name.unwrap_or_default(),
                    "comment": Value::Null,
                }));
            }
        }
    }
    Ok(Json(json!({ "tables": hit_tables, "columns": columns })))
}

// ------------------------------------------------------------------ 表数据

/// 「这张表在哪」——表名可能带 schema 前缀，逐段加引号。
/// LIKE 模式里的通配符转义：用户文本里的 `%`/`_` 要按字面匹配，得先逃掉。
/// **不转义单引号** —— 产出必然再过一遍 `dialect.literal`，那里会转；在这里先转
/// 会被二次翻倍（`O'Brien` → `O''''Brien`），含引号的搜索永远匹配不到。
fn escape_like(value: &str) -> String {
    value.replace('%', "\\%").replace('_', "\\_")
}

/// 高级搜索里的一条条件 → SQL 片段。拼不出来（列名空、数值列填了非数字）就返回 None。
///
/// 安全上有两条硬规矩：
/// * 列名一律走 `dialect.quote`（保留字、大小写都属于方言的事）；
/// * 字符串值一律走 `dialect.literal`（转义单引号）；数值列则**只允许数字字符**
///   并直接写裸数字 —— `int = '548'` 在 PostgreSQL 上会直接报错，
///   顺带也堵住了"把任意串塞进数值比较"这条路。
fn filter_condition(dialect: &Dialect, item: &Value) -> Option<String> {
    let column_name = item.get("col").and_then(Value::as_str)?.trim();
    if column_name.is_empty() {
        return None;
    }
    let op = item.get("op").and_then(Value::as_str).unwrap_or("eq");
    let kind = item.get("colType").and_then(Value::as_str).unwrap_or("string");
    let column = dialect.quote(column_name);

    // 空值判断不需要值
    match op {
        "is_null" => return Some(format!("{column} is null")),
        "is_not_null" => return Some(format!("{column} is not null")),
        _ => {}
    }

    let text = item.get("value").and_then(Value::as_str).unwrap_or_default();
    match op {
        "between" => {
            let low = scalar_of(dialect, kind, text)?;
            let high = scalar_of(dialect, kind, item.get("value2").and_then(Value::as_str).unwrap_or_default())?;
            Some(format!("{column} between {low} and {high}"))
        }
        "contains" | "not_contains" | "starts_with" | "ends_with" => {
            let pattern = match op {
                "starts_with" => format!("{}%", escape_like(text)),
                "ends_with" => format!("%{}", escape_like(text)),
                _ => format!("%{}%", escape_like(text)),
            };
            let keyword = if op == "not_contains" { "not like" } else { "like" };
            Some(format!("{column} {keyword} {}", dialect.literal(&pattern)))
        }
        _ => {
            let value = scalar_of(dialect, kind, text)?;
            let operator = match op {
                "ne" => "<>",
                "gt" => ">",
                "lt" => "<",
                "gte" => ">=",
                "lte" => "<=",
                _ => "=",
            };
            Some(format!("{column} {operator} {value}"))
        }
    }
}

/// 一个值 → SQL 标量：数值列直接写数字（且必须真是数字），其余转义加引号。
fn scalar_of(dialect: &Dialect, kind: &str, raw: &str) -> Option<String> {
    if kind == "number" {
        let trimmed = raw.trim();
        let looks_numeric = !trimmed.is_empty()
            && trimmed
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
            && trimmed.parse::<f64>().is_ok();
        return looks_numeric.then(|| trimmed.to_string());
    }
    Some(dialect.literal(raw))
}

/// 整组高级搜索条件 → 一个 SQL 条件（各组已按行里的 且/或 拼好）。
///
/// 逐条从左往右拼并**层层加括号**：与界面上一行一行读下来的意思一致。
/// （SQL 里 AND 优先级高于 OR，不加括号时 `a AND b OR c` 会被读成 `(a AND b) OR c`，
/// 而用户按界面那样理解可能是 `a AND (b OR c)` —— 这种"看起来一样、筛出来不一样"
/// 的差异最难查，所以在源头就消掉。）
fn filter_clause(dialect: &Dialect, filters: &[Value]) -> Option<String> {
    let mut combined: Option<String> = None;
    for item in filters {
        let Some(condition) = filter_condition(dialect, item) else {
            continue;
        };
        combined = Some(match combined {
            None => format!("({condition})"),
            Some(previous) => {
                let is_or = item
                    .get("join")
                    .and_then(Value::as_str)
                    .map(|join| join.eq_ignore_ascii_case("or"))
                    .unwrap_or(false);
                let join = if is_or { "or" } else { "and" };
                format!("({previous} {join} ({condition}))")
            }
        });
    }
    combined
}

/// 分页浏览表数据（`/data`）。
///
/// 界面语义（分页 / 关键字模糊 / 排序）在这里翻成 SQL —— 内核只认「一条语句」。
/// `keyword` 会横跨所有文本列做 `LIKE`，列清单从内核的列元数据取（不猜类型）。
pub async fn data(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let table = params
        .get("table")
        .ok_or_else(|| XError::bad_request("缺少 table 参数"))?;
    let database = params.get("database").unwrap_or_default();
    let page: u64 = params
        .get("page")
        .and_then(|p| p.parse().ok())
        .filter(|p| *p > 0)
        .unwrap_or(1);
    let size: u64 = params
        .get("size")
        .and_then(|s| s.parse().ok())
        .filter(|s| *s > 0 && *s <= 100_000)
        .unwrap_or(50);
    let keyword = params.get("keyword").unwrap_or_default();
    let order_column = params.get("orderColumn").unwrap_or_default();
    let order_dir = params.get("orderDir").unwrap_or_default();
    // 高级搜索的条件（界面「添加条件」那一排）：
    // `[{"join":"AND","col":"id","op":"eq","value":"548","colType":"number"}]`
    // 这个参数以前被整个丢掉 —— 界面点了「应用筛选」却还是整张表，就是这么来的。
    let filters: Vec<Value> = params
        .get("filters")
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();

    let (_, dialect) = ctx_of(&state, &id).await?;
    let quoted = dialect.quote(&table);
    // 所有筛选条件（关键字模糊 + 高级搜索）都汇到这里，统一用 AND 串起来；
    // 同一份 where 既给数据查询、也给总条数统计 —— 否则会出现"筛出 1 行、总数说 10000"。
    let mut where_groups: Vec<String> = Vec::new();

    if !keyword.trim().is_empty() {
        let engine = state.engine();
        let name = table.clone();
        let connection = id.clone();
        let columns = blocking(move || engine.list_columns_fresh(&connection, &name)).await?;
        let text_columns: Vec<String> = columns
            .iter()
            .filter(|c| {
                let t = c
                    .type_name
                    .clone()
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                ["CHAR", "TEXT", "CLOB", "STRING", "JSON", "UUID", "ENUM", "XML", "NCHAR"]
                    .iter()
                    .any(|probe| t.contains(probe))
            })
            .map(|c| c.name.clone())
            .collect();
        if text_columns.is_empty() {
            return Err(XError::bad_request(format!(
                "表「{table}」没有可作为模糊匹配的文本列，无法按关键字筛选"
            )));
        }
        let pattern = dialect.literal(&format!("%{}%", escape_like(&keyword)));
        let conditions: Vec<String> = text_columns
            .iter()
            .map(|c| format!("{} like {pattern}", dialect.quote(c)))
            .collect();
        where_groups.push(format!("({})", conditions.join(" or ")));
    }
    // 高级搜索：条件本身由 filter_clause 负责翻译与括号；这里只把它并进 where
    if let Some(clause) = filter_clause(&dialect, &filters) {
        where_groups.push(clause);
    }

    let where_suffix = if where_groups.is_empty() {
        String::new()
    } else {
        format!(" where {}", where_groups.join(" and "))
    };
    let mut sql = format!("select * from {quoted}{where_suffix}");

    if !order_column.trim().is_empty() {
        let dir = if order_dir.eq_ignore_ascii_case("desc") {
            "desc"
        } else {
            "asc"
        };
        sql.push_str(&format!(" order by {} {dir}", dialect.quote(&order_column)));
    } else if dialect.needs_order_by_for_paging() {
        // SQL Server 的 OFFSET/FETCH 必须有 ORDER BY
        sql.push_str(" order by (select null)");
    }
    sql.push_str(&format!(
        " {}",
        dialect.limit_clause(page.saturating_sub(1).saturating_mul(size), size)
    ));

    let result = match run_sql_in(&state, &id, &database, sql, size as usize).await {
        Ok(result) => result,
        Err(err) => return Ok(Json(shape::query_failure_json(&err.message, 0))),
    };
    let mut payload = shape::query_result_json(&result);

    // 真实总条数：分页控件要用它。统计失败就当「未知」（-1），不假装是当前页的行数。
    // 必须带上与数据查询**同一份 where** —— 否则筛出 1 行而总数显示 10000。
    let count_sql = format!("{}{where_suffix}", dialect.count_sql(&table));
    let total = match run_sql_in(&state, &id, &database, count_sql, 1).await {
        Ok(count_result) => rows_of(&count_result)
            .first()
            .and_then(|row| row.values().next())
            .and_then(Value::as_i64)
            .unwrap_or(-1),
        Err(_) => -1,
    };
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("totalCount".to_string(), json!(total));
        obj.insert("page".to_string(), json!(page));
        obj.insert("size".to_string(), json!(size));
    }
    Ok(Json(payload))
}

/// 一行数据 → 值字面量。
fn sql_value(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "NULL".to_string(),
        Some(Value::Bool(b)) => {
            if *b {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => format!("'{}'", s.replace('\'', "''")),
        Some(other) => format!("'{}'", other.to_string().replace('\'', "''")),
    }
}

fn keys_of(row: &Map<String, Value>) -> Vec<String> {
    row.keys().cloned().collect()
}

fn same_value(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), None) | (None, Some(Value::Null)) => true,
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// 表的就地编辑（新增 / 修改 / 删除）。
///
/// 与上游一致的取舍：**一行一条语句、顺序执行**。内核的安全闸门只允许一次执行一条，
/// 所以做不到「全部成功或全部回滚」—— 于是如实报告「执行到第几条失败”，绝不假装原子。
pub async fn data_save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let table = body
        .get("table")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if table.is_empty() {
        return Err(XError::bad_request("缺少 table"));
    }
    let (_, dialect) = ctx_of(&state, &id).await?;
    let quoted = dialect.quote(&table);
    let pk_columns: Vec<String> = body
        .get("pkColumns")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut statements: Vec<String> = Vec::new();

    let inserts = body.get("inserts").and_then(Value::as_array).cloned().unwrap_or_default();
    for row in &inserts {
        let Some(obj) = row.as_object() else { continue };
        let columns = keys_of(obj);
        if columns.is_empty() {
            continue;
        }
        let names: Vec<String> = columns.iter().map(|c| dialect.quote(c)).collect();
        let values: Vec<String> = columns
            .iter()
            .map(|c| sql_value(obj.get(c)))
            .collect();
        statements.push(format!(
            "insert into {quoted} ({}) values ({})",
            names.join(", "),
            values.join(", ")
        ));
    }

    let updates = body.get("updates").and_then(Value::as_array).cloned().unwrap_or_default();
    for item in &updates {
        let original = item.get("original").and_then(Value::as_object);
        let row = item.get("row").and_then(Value::as_object);
        let (Some(original), Some(row)) = (original, row) else { continue };
        let changed: Vec<String> = keys_of(row)
            .into_iter()
            .filter(|c| !same_value(original.get(c), row.get(c)))
            .collect();
        if changed.is_empty() {
            continue; // 改回原值等于没改，不留一条什么都没变的 UPDATE
        }
        let assignments: Vec<String> = changed
            .iter()
            .map(|c| format!("{} = {}", dialect.quote(c), sql_value(row.get(c))))
            .collect();
        let keys: Vec<String> = if pk_columns.is_empty() {
            keys_of(original)
        } else {
            pk_columns.clone()
        };
        let conditions: Vec<String> = keys
            .iter()
            .map(|c| format!("{} = {}", dialect.quote(c), sql_value(original.get(c))))
            .collect();
        statements.push(format!(
            "update {quoted} set {} where {}",
            assignments.join(", "),
            conditions.join(" and ")
        ));
    }

    let deletes = body.get("deletes").and_then(Value::as_array).cloned().unwrap_or_default();
    for row in &deletes {
        let Some(obj) = row.as_object() else { continue };
        let keys: Vec<String> = if pk_columns.is_empty() {
            keys_of(obj)
        } else {
            pk_columns.clone()
        };
        let conditions: Vec<String> = keys
            .iter()
            .map(|c| format!("{} = {}", dialect.quote(c), sql_value(obj.get(c))))
            .collect();
        statements.push(format!(
            "delete from {quoted} where {}",
            conditions.join(" and ")
        ));
    }

    // 就地编辑要落到目标库上（跨库时就是那条影子连接）
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let target = crate::api::scope::resolve(&state, &id, &database).await?;

    if statements.is_empty() {
        return Ok(Json(json!({
            "success": true,
            "columns": [], "columnTypes": [], "rows": [],
            "rowCount": 0, "totalCount": -1, "affectedRows": 0, "executeTime": 0,
            "message": "没有需要提交的变更", "notices": [], "hasMore": false,
        })));
    }

    let total = statements.len();
    let engine = state.engine();
    let (affected, executed, error) = blocking(move || {
        let mut affected: i64 = 0;
        let mut executed = 0usize;
        for sql in statements {
            let request = QueryRequest {
                read_only: None,
                connection: target.clone(),
                sql,
                options: QueryOptions {
                    max_rows: 1,
                    timeout_ms: 120_000,
                },
                execution_id: None,
                session: Some("internal:browse".to_string()),
                // 表格「就地编辑」提交的增删改 —— 界面功能驱动的写入，不进查询历史
                // （与编辑器里手写 INSERT/UPDATE 是两回事）
                internal: true,
            };
            match engine.execute(request, AccessContext::Web) {
                Ok(result) => {
                    affected += result.affected_rows.unwrap_or(0) as i64;
                    executed += 1;
                }
                Err(err) => return Ok((affected, executed, Some(err.message))),
            }
        }
        Ok((affected, executed, None))
    })
    .await?;

    let (success, message) = match &error {
        None => (true, format!("已提交 {executed} 条语句，影响 {affected} 行")),
        Some(err) => (
            false,
            format!("第 {} 条失败（共 {total} 条，已成功 {executed} 条）：{err}", executed + 1),
        ),
    };
    Ok(Json(json!({
        "success": success,
        "columns": [], "columnTypes": [], "rows": [],
        "rowCount": 0, "totalCount": -1,
        "affectedRows": affected,
        "executeTime": 0,
        "message": message,
        "notices": [], "hasMore": false,
    })))
}

// ------------------------------------------------------------------ 建表语句与对象

/// 建表语句 / 视图定义（界面读 `res.ddl`）。
pub async fn ddl(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let table = params
        .get("table")
        .ok_or_else(|| XError::bad_request("缺少 table 参数"))?;
    let database = params.get("database").unwrap_or_default();
    let (ddl_text, synthesized) = build_table_ddl(&state, &id, &database, &table).await?;
    Ok(Json(
        json!({ "ddl": ddl_text, "sql": ddl_text, "synthesized": synthesized }),
    ))
}

/// 取一张表的建表语句，返回 `(SQL, 是否按列元数据自拼)`。
///
/// `/ddl`、`object-info(type=table)` 与导出的 `ddl_of` **共用这一条路径**。原先三处各写一份、
/// 判据还不一样：导出能"自拼"，「查看 DDL」却直接 501 —— 同一份列元数据、同一个问题两套答案。
///
/// 三档优先级：
/// 1. 方言能给出**原始建表语句**（`show create table` / `sqlite_master` / H2 的 `script`）⇒ 用它，完整；
/// 2. 方言没有那条语句（DB2 / Derby）⇒ **按列元数据自拼**（列 + 主键 + 非空 + 默认值），
///    并把"少了什么"写进脚本注释（调用方无需再猜）；
/// 3. 连列都取不到 ⇒ 报错（这时确实什么都给不了）。
pub(crate) async fn build_table_ddl(
    state: &AppState,
    id: &str,
    database: &str,
    table: &str,
) -> XResult<(String, bool)> {
    let (_, dialect) = ctx_of(state, id).await?;
    let sql = match dialect.ddl(table) {
        Meta::Sql(sql) => sql,
        Meta::Absent => return Ok((String::new(), false)),
        Meta::Unwritten => {
            let ddl = synthesize_ddl(state, id, database, table, dialect).await?;
            // "这是拼出来的"直接写进脚本文本：用户复制出去时这句跟着走，
            // 不会以为拿到了完整建表语句（导出那边是走 notices，这里没有 notices 通道）
            let notice = format!(
                "-- 注意：{} 没有「一条语句拿到完整建表语句」的办法，以下按列元数据拼出\
                 （含列、主键、非空与默认值；可能少索引、外键与注释）\n",
                dialect.kind.key().to_ascii_uppercase()
            );
            return Ok((format!("{notice}{ddl}"), true));
        }
    };
    let result = run_sql_in(state, id, database, sql, 100).await?;
    let mut ddl_text = if dialect.kind.key() == "h2" {
        // H2 的 `script nodata` 返回**整库**语句（它是一条命令，不接受 WHERE，
        // `table <名>` 过滤实测也被忽略），所以在这里按表名筛出属于这张表的那几条
        // —— 建表、主键约束、索引都会提到 `"PUBLIC"."T_PROBE"` 这个限定名，
        // 一并拼起来正好是完整 DDL（别的方言还得靠 ddl_extras 补索引）。
        // 不能交给 `pick_ddl`：它只取第一行，而 H2 脚本第一行是头注释 `-- H2 2.3.232;`。
        let needle = format!("\"{}\"", table.to_ascii_uppercase());
        rows_of(&result)
            .iter()
            .filter_map(|row| row.values().next())
            .filter_map(Value::as_str)
            .filter(|text| text.to_ascii_uppercase().contains(&needle))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        pick_ddl(&result)
    };
    // SQL Server 的注释（扩展属性）/ 索引 / 外键**写不进 create table**，只能作为后续语句追加。
    // 不追加的话，用户复制出去的「建表语句」重建出来是一张没有注释、没有索引、没有外键的空壳 ——
    // 这正是「DDL 不全、注释没出来」的来源。每段都是一次查询返回的可执行脚本；
    // 某段取不到就跳过：主 DDL 已经拿到了，不该因为附加信息失败而整条接口失败。
    if !ddl_text.trim().is_empty() {
        for (label, extra_sql) in dialect.ddl_extras(table) {
            let Ok(extra_result) = run_sql_in(state, id, database, extra_sql, 1).await else {
                continue;
            };
            let extra = pick_ddl(&extra_result);
            if !extra.is_empty() {
                ddl_text.push_str(&format!("\n\n-- {label}\n{extra}\n"));
            }
        }
    }
    if ddl_text.trim().is_empty() {
        return Err(XError::internal(format!("没有取到 {table} 的建表语句")));
    }
    Ok((ddl_text, false))
}

/// 按列元数据自拼建表语句（DB2 / Derby 这类"没有一条语句拿到 DDL"的类型走这里）。
///
/// 与导出、同步用的是**同一个** `create_table_from_columns`：同一份列元数据，
/// 在导出里拼得出来、在「查看 DDL」里却报 501，是说不过去的。
async fn synthesize_ddl(
    state: &AppState,
    id: &str,
    database: &str,
    table: &str,
    dialect: Dialect,
) -> XResult<String> {
    let target = crate::api::scope::resolve(state, id, database).await?;
    let engine = state.engine();
    let name = table.to_string();
    let mut columns = blocking(move || engine.list_columns_fresh(&target, &name)).await?;
    if columns.is_empty() {
        return Err(XError::internal(format!(
            "表 {table} 没有列信息，无法生成建表语句（{}）",
            dialect.kind.key()
        )));
    }
    // **必须先补类型长度**：内核给的列类型只有名字（Derby 是 `VARCHAR`、PG 是 `varchar`），
    // 直接拿去拼 DDL 会得到 `CODE VARCHAR not null` —— 建出来是 1 个字符宽（实测 Derby）。
    // 这条补全与 `/columns` 接口用的是同一个函数，两边口径一致。
    enrich_column_types(state, id, database, table, &mut columns).await;
    // `None` = 同类型：这里是"查看 DDL / 导出"，列的来源就是本连接自己，不需要类型翻译
    Ok(dialect.create_table_from_columns(
        &crate::api::export::qualified(table, database, dialect),
        &columns,
        None,
    ))
}

/// 结果里的某个文本是不是一段 DDL（`create ...`，允许前面有空行/注释）。
///
/// 为什么不能图省事用 `contains("create")`：MySQL 的 `show create procedure` /
/// `show create function` 会**多带一列 `sql_mode`**，其值形如
/// `STRICT_TRANS_TABLES,NO_ZERO_IN_DATE,…,NO_AUTO_CREATE_USER,NO_ENGINE_SUBSTITUTION`
/// —— 里面正好有 `CREATE`，于是那一列也被当成定义写进导出脚本。
/// 实测后果：导出的存储过程后面多出一行 `STRICT_TRANS_TABLES,…;`，
/// 这样的文件**重新导入会直接报语法错**（导出能用、还原不能用是最坏的一种）。
/// 要求"第一个有内容的行以 create 开头"，正好挡住这类"恰好含 create 的配置串"。
pub(crate) fn looks_like_ddl(text: &str) -> bool {
    for line in text.lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with("--") || line.starts_with("/*") {
            continue;
        }
        return line.to_ascii_lowercase().starts_with("create");
    }
    false
}

/// 从结果里取出 DDL 文本（按**列位置**取，不能按 Map 的取值顺序取）。
///
/// 为什么必须按位置：`serde_json` 的 Map 默认**按 key 排序**，而 MySQL 的
/// `show create table` 两列是 `[Table, Create Table]` —— 排序后 `Create Table` 反而在前，
/// 于是老代码那句"取第二列"取到的是**表名**（实测：`sales` 的建表语句只显示了一个 `sales`）。
///
/// 选列顺序：
/// 1. 列名像 DDL 的（跨方言各有各的叫法：MySQL `Create Table` / ClickHouse `statement` /
///    SQLite·H2 `sql` / 本仓自拼的 PG·SQLServer·DM 统一 `ddl`）；
/// 2. 只有一列就用它（Oracle 的 `dbms_metadata.get_ddl` 出来的是长表达式当列名）；
/// 3. 都不匹配时取**值最长的那一列** —— DDL 一定比表名、字符集之类的长，这条兜底
///    比"取第几列"稳得多。
/// 从「`show create …` / `pg_get_*def`」这类结果里挑出**定义原文**那一列。
///
/// 备份的例程/触发器导出也用它（`backup::export_objects`）—— 定义怎么选列，
/// 两处必须同一条规则，否则「查看定义」正常、备份文件里却是表名。
pub(crate) fn pick_ddl(result: &QueryResult) -> String {
    let Some(row) = result.rows.first() else {
        return String::new();
    };
    let values: Vec<String> = row
        .iter()
        .map(|cell| match shape::cell_to_value(cell) {
            Value::String(text) => text,
            Value::Null => String::new(),
            other => other.to_string(),
        })
        .collect();
    if values.is_empty() {
        return String::new();
    }
    let names: Vec<String> = result
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect();
    let index = ddl_column_index(&names, &values);
    values
        .get(index)
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}

/// `pick_ddl` 的选列规则（拆出来是为了能单独读懂"为什么是这一列"）。
fn ddl_column_index(names: &[String], values: &[String]) -> usize {
    const HINTS: [&str; 15] = [
        "createtable",
        "createview",
        // 例程/触发器/事件：`show create procedure` 的列名是 `Create Procedure`，
        // 落不到「最长值」兜底就靠不住 —— 空过程体比 sql_mode 还短（例如
        // `CREATE ... PROCEDURE p() BEGIN END`），那时兜底会挑中 sql_mode。
        "createprocedure",
        "createfunction",
        "createtrigger",
        "createevent",
        // 但 MySQL 的 `show create trigger` 偏偏不叫 `Create Trigger`，
        // 而叫 `SQL Original Statement`（实测：body 短时会被兜底挑成 sql_mode）
        "sqloriginalstatement",
        "createstatement",
        "ddl",
        "sql",
        "statement",
        "ddltext",
        "createtableifnotexists",
        // PG / SQL Server 的 `pg_get_*def`、`object_definition` 都是这个列名
        "definition",
        "create",
    ];
    // 归一化：去掉空格/下划线/横线并转小写（`Create Table` → `createtable`）
    let normalize = |name: &str| {
        name.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase()
    };
    for (index, name) in names.iter().enumerate() {
        if HINTS.contains(&normalize(name).as_str()) {
            return index;
        }
    }
    if values.len() <= 1 {
        return 0;
    }
    values
        .iter()
        .enumerate()
        .max_by_key(|(_, text)| text.len())
        .map(|(index, _)| index)
        .unwrap_or(0)
}

/// 对象定义（视图 / 存储过程 / 函数 / 触发器）——右键「编辑」用它取到 DDL 再转 ALTER。
pub async fn object_info(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let database = params.get("database").unwrap_or_default();
    let name = params.get("name").unwrap_or_default();
    let object_type = params
        .get("type")
        .or_else(|| params.get("kind"))
        .unwrap_or_default()
        .to_lowercase();
    if name.is_empty() {
        return Err(XError::bad_request("缺少 name 参数"));
    }
    // 表节点走的也是这个接口：前端右键「编辑 / 查看定义」时按 `object.kind` 传，
    // 表就是 `table`。而 `object_source` 只覆盖视图 / 例程 / 触发器 / 事件 ——
    // 表会落进兜底分支，两种坏法：
    //   MySQL  → 501，界面直接弹「获取定义失败」
    //   SQL Server → 200 但 ddl 为空，编辑器里一片空白（比报错更难发现）
    // 表的定义本来就是「建表语句」，所以直接复用 `/ddl` 那条路径 ——
    // 顺带把 SQL Server 的注释 / 索引 / 外键追加也一起带过来。
    if matches!(object_type.as_str(), "table" | "tables") {
        let (ddl, synthesized) = build_table_ddl(&state, &id, &database, &name).await?;
        return Ok(Json(json!({
            "ddl": ddl,
            "sql": ddl,
            "definition": ddl,
            "type": "table",
            "name": name,
            "synthesized": synthesized,
        })));
    }
    let (_, dialect) = ctx_of(&state, &id).await?;
    let sql = match dialect.object_source(&object_type, &name) {
        Meta::Sql(sql) => sql,
        Meta::Absent => return Ok(Json(json!({ "ddl": "", "sql": "" }))),
        Meta::Unwritten => {
            return Err(XError::not_implemented(&format!(
                "「对象定义」（{} / {object_type}）",
                dialect.kind.key()
            )))
        }
    };
    let result = run_sql_in(&state, &id, &database, sql, 10).await?;
    // 必须按**列名**取，不能按位置取：`serde_json` 的 Map 默认按键名排序，
    // `show create view` 的四列（View / Create View / character_set_client / ...）
    // 排序后 `Create View` 反而在前，"取第二列"取到的是**视图名** ——
    // 界面上表现为「编辑视图」页签里只有一行 `v1`。选列规则见 `pick_ddl`。
    let definition = pick_ddl(&result);
    Ok(Json(json!({
        "ddl": definition,
        "sql": definition,
        "definition": definition,
        "type": object_type,
        "name": name,
    })))
}

// ------------------------------------------------------------------ 对象清单

pub async fn indexes(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let (_, dialect) = ctx_of(&state, &id).await?;
    let meta = dialect.indexes();
    let value = meta_list(
        &state,
        &id,
        &params.get("database").unwrap_or_default(),
        meta,
        "索引清单",
    )
    .await?;
    // 统一字段：界面按 name / table / unique / columns 取值
    let normalized = match value {
        Value::Array(rows) => Value::Array(
            rows.into_iter()
                .map(|row| {
                    let mut out = row.as_object().cloned().unwrap_or_default();
                    let unique = matches!(
                        out.get("unique"),
                        Some(Value::Bool(true)) | Some(Value::Number(_))
                    ) || out.get("nonUnique").and_then(Value::as_i64) == Some(0)
                        || out.get("isUnique").and_then(Value::as_i64) == Some(1)
                        || out.get("isUnique") == Some(&Value::Bool(true));
                    out.insert("unique".to_string(), Value::Bool(unique));
                    // 列：MySQL 是 group_concat，其余从语句里抠
                    let columns = match out.get("columns") {
                        Some(Value::String(s)) if !s.is_empty() => s
                            .split(',')
                            .map(|c| Value::String(c.trim().to_string()))
                            .collect::<Vec<_>>(),
                        _ => {
                            let definition = text_ci(&out, "sql");
                            let definition = if definition.is_empty() {
                                text_ci(&out, "indexdef")
                            } else {
                                definition
                            };
                            match definition.split_once('(') {
                                Some((_, rest)) => rest
                                    .split(')')
                                    .next()
                                    .unwrap_or("")
                                    .split(',')
                                    .map(|c| {
                                        Value::String(
                                            c.trim().trim_matches(['"', '`', '[', ']']).to_string(),
                                        )
                                    })
                                    .filter(|v| v.as_str().map(|s| !s.is_empty()).unwrap_or(false))
                                    .collect(),
                                None => Vec::new(),
                            }
                        }
                    };
                    out.insert("columns".to_string(), Value::Array(columns));
                    out.remove("sql");
                    out.remove("indexdef");
                    out.insert("_state".to_string(), Value::String("loaded".to_string()));
                    Value::Object(out)
                })
                .collect(),
        ),
        other => other,
    };
    Ok(Json(normalized))
}

pub async fn procedures(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let (_, dialect) = ctx_of(&state, &id).await?;
    meta_list(
        &state,
        &id,
        &params.get("database").unwrap_or_default(),
        dialect.procedures(),
        "存储过程/函数清单",
    )
    .await
    .map(Json)
}

pub async fn triggers(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let (_, dialect) = ctx_of(&state, &id).await?;
    meta_list(
        &state,
        &id,
        &params.get("database").unwrap_or_default(),
        dialect.triggers(),
        "触发器清单",
    )
    .await
    .map(Json)
}

pub async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let (_, dialect) = ctx_of(&state, &id).await?;
    meta_list(
        &state,
        &id,
        &params.get("database").unwrap_or_default(),
        dialect.events(),
        "事件清单",
    )
    .await
    .map(Json)
}

pub async fn users(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let (_, dialect) = ctx_of(&state, &id).await?;
    let value = meta_list(
        &state,
        &id,
        &params.get("database").unwrap_or_default(),
        dialect.users(),
        "用户清单",
    )
    .await?;
    let normalized = match value {
        Value::Array(rows) => Value::Array(
            rows.into_iter()
                .map(|row| {
                    let mut out = row.as_object().cloned().unwrap_or_default();
                    let is_super = matches!(out.get("isSuper"), Some(Value::Bool(true)));
                    out.entry("comment".to_string())
                        .or_insert_with(|| Value::String(if is_super { "超级用户".into() } else { String::new() }));
                    Value::Object(out)
                })
                .collect(),
        ),
        other => other,
    };
    Ok(Json(normalized))
}

// ------------------------------------------------------------------ 表操作

/// 执行结构变更（表结构 / 索引 / 表选项编辑都走它）。
pub async fn alter(
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
        return Err(XError::bad_request("缺少 sql"));
    }
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default();

    // **必须按语句拆分后逐条执行**。
    //
    // 界面的「编辑表结构」把整段脚本一次发过来（`scriptParts.join(';\n')`），
    // 而内核的安全规则是「一次只允许执行一条语句」—— 整段直接丢下去的话，
    // 凡是产生两句以上的改动（改字段 + 改注释、改主键的 DROP+ADD…）都会失败在
    // 「检测到 2 条，请拆分后执行」上，而这个对话框只有一个提交按钮，没法自己拆。
    let record = require_record(&state, &id).await?;
    let statements: Vec<String> = dbmind_core::split_statements(record.kind().protocol(), &sql)
        .into_iter()
        .filter(|statement| !statement.trim().is_empty())
        .collect();
    if statements.is_empty() {
        return Err(XError::bad_request("sql 里没有可执行的语句"));
    }

    let mut executed = 0usize;
    let mut last = Value::Null;
    for (index, statement) in statements.iter().enumerate() {
        // 表结构编辑是**写库**：走跟随连接策略的入口（只读会话会被驱动拒绝，见 run_write_sql_in）
        match run_write_sql_in(&state, &id, &database, statement.clone(), 1).await {
            Ok(result) => {
                executed += 1;
                last = shape::query_result_json(&result);
            }
            Err(err) => {
                // 已执行的那些**不会回滚**（DDL 多数不可回滚），所以要说清
                // 「第几条失败、前几条已生效」，而不是丢一句失败就走 ——
                // 用户需要知道库现在处在什么状态。
                let mut payload = shape::query_failure_json(&err.message, 0);
                if let Some(object) = payload.as_object_mut() {
                    object.insert("failedSql".to_string(), json!(statement));
                    object.insert("failedIndex".to_string(), json!(index + 1));
                    object.insert("totalStatements".to_string(), json!(statements.len()));
                    object.insert("executedStatements".to_string(), json!(executed));
                    object.insert(
                        "message".to_string(),
                        json!(format!(
                            "第 {}/{} 条执行失败：{}（前 {} 条已生效，DDL 无法自动回滚）",
                            index + 1,
                            statements.len(),
                            err.message,
                            executed
                        )),
                    );
                }
                return Ok(Json(payload));
            }
        }
    }
    if let Some(object) = last.as_object_mut() {
        object.insert("totalStatements".to_string(), json!(statements.len()));
        object.insert("executedStatements".to_string(), json!(executed));
        object.insert(
            "message".to_string(),
            json!(format!("{executed} 条语句全部执行成功")),
        );
    }
    Ok(Json(last))
}

/// 表级危险操作：清空 / 截断 / 删除 / 重命名（服务端按方言生成语句，不接受整段 SQL）。
pub async fn table_action(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let table = body
        .get("table")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let action = body
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if table.is_empty() || action.is_empty() {
        return Err(XError::bad_request("缺少 table 或 action"));
    }
    let new_name = body
        .get("newName")
        .and_then(Value::as_str)
        .map(str::to_string);
    let (_, dialect) = ctx_of(&state, &id).await?;
    let sql = match dialect.table_action(&table, &action, new_name.as_deref()) {
        Meta::Sql(sql) => sql,
        _ => {
            return Err(XError::bad_request(format!(
                "不支持的表操作：{action}（重命名需要提供 newName）"
            )))
        }
    };
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default();
    // 这里的 `1` 只限制**结果集行数**：这几条语句（delete / truncate / drop / rename）
    // 都不返回结果集，所以给多少都无所谓。
    //
    // 但**不要**指望靠它给 DML 加限制：宿主曾经把 `maxRows` 也套在 DELETE 上
    // （Microsoft 驱动用会话级的 `SET ROWCOUNT` 实现 `setMaxRows`），于是「清空表」
    // 只删掉 maxRows+1 行、接口照样回 success —— 静默少删。已在宿主侧修正：
    // 只有判定会返回结果集的语句才设上限（见 agents/dbmind-agent-jdbc 的 mayReturnRows）。
    // 清空 / 截断 / 删除 / 重命名表都是写库：走跟随连接策略的入口
    Ok(Json(match run_write_sql_in(&state, &id, database, sql, 1).await {
        Ok(result) => shape::query_result_json(&result),
        Err(err) => shape::query_failure_json(&err.message, 0),
    }))
}

/// 对象重命名（视图 / 索引 / 表 …）。`previewOnly=true` 时只回要执行的语句，不执行。
pub async fn rename_object(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let new_name = body
        .get("newName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let object_type = body
        .get("type")
        .or_else(|| body.get("kind"))
        .or_else(|| body.get("objectKind"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    let preview_only = body
        .get("previewOnly")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if name.is_empty() || new_name.is_empty() {
        return Err(XError::bad_request("重命名需要原名与新名"));
    }
    let (_, dialect) = ctx_of(&state, &id).await?;
    let from = dialect.quote(&name);
    let to = dialect.quote(&new_name);
    let sql = match object_type.as_str() {
        "table" => match dialect.table_action(&name, "rename", Some(&new_name)) {
            Meta::Sql(sql) => sql,
            _ => return Err(XError::not_implemented("重命名表")),
        },
        "view" => match dialect.kind.key() {
            "sqlite" => return Err(XError::bad_request("SQLite 不支持重命名视图（可以删掉重建）")),
            "mysql" | "mariadb" | "doris" => format!("rename table {from} to {to}"),
            _ => format!("alter view {from} rename to {to}"),
        },
        "index" => match dialect.kind.key() {
            "mysql" | "mariadb" | "doris" => {
                return Err(XError::bad_request(
                    "MySQL 重命名索引要写在 ALTER TABLE 里，请在查询页执行",
                ))
            }
            _ => format!("alter index {from} rename to {to}"),
        },
        // 存储过程 / 函数：只有 PG 与 SQL Server 真有「重命名」语法
        "procedure" | "function" => match dialect.kind.key() {
            // PG：签名可省略（同名函数唯一时），有歧义会由数据库明确报错
            "postgresql" | "kingbase" => format!("alter {object_type} {from} rename to {to}"),
            // SQL Server：`sp_rename` 是唯一入口。它**不会更新依赖对象**（视图/存储过程里
            // 引用的旧名字会失效），这句提醒由界面/SQL 预览负责展示，这里只保证语句本身正确
            "sqlserver" => format!(
                "exec sp_rename {}, {}",
                dialect.literal(&name),
                dialect.literal(&new_name)
            ),
            "mysql" | "mariadb" | "doris" => {
                return Err(XError::bad_request(
                    "MySQL 没有重命名存储过程/函数的语法：请用 SHOW CREATE PROCEDURE 取到原文，\
                     DROP 之后用新名字重建",
                ))
            }
            other => {
                return Err(XError::bad_request(format!(
                    "{} 不支持重命名存储过程/函数：请在查询页 DROP 后用新名字重建",
                    other.to_ascii_uppercase()
                )))
            }
        },
        // 触发器：SQL Server 可改（同样有依赖风险）；PG 的 `ALTER TRIGGER` 必须带表名，
        // 而我们这里只有触发器名 —— 与其猜一张表，不如把限制说清楚
        "trigger" => match dialect.kind.key() {
            "sqlserver" => format!(
                "exec sp_rename {}, {}",
                dialect.literal(&name),
                dialect.literal(&new_name)
            ),
            "postgresql" | "kingbase" => {
                return Err(XError::bad_request(
                    "PostgreSQL 重命名触发器需要表名（alter trigger 名 on 表 rename to 新名）——\
                     树节点只带了触发器名，请到查询页执行",
                ))
            }
            other => {
                return Err(XError::bad_request(format!(
                    "{} 不支持重命名触发器：请在查询页 DROP TRIGGER 后用新名字重建",
                    other.to_ascii_uppercase()
                )))
            }
        },
        // 事件调度器：各家都没有「改名字」的语法
        "event" => {
            return Err(XError::bad_request(
                "事件没有重命名语法：请在查询页 DROP EVENT 后用新名字重建（SHOW CREATE EVENT 可取原文）",
            ))
        }
        other => {
            return Err(XError::bad_request(format!(
                "不支持重命名「{other}」这种对象"
            )))
        }
    };
    if preview_only {
        return Ok(Json(json!({ "sql": sql, "native": true })));
    }
    let database = body
        .get("database")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match run_write_sql_in(&state, &id, database, sql.clone(), 1).await {
        Ok(_) => Ok(Json(json!({ "success": true, "message": format!("已重命名为「{new_name}」"), "sql": sql }))),
        Err(err) => Ok(Json(json!({ "success": false, "message": err.message, "sql": sql }))),
    }
}

// ------------------------------------------------------------------ 用户管理

/// `GET /api/{m}/{id}/user-info?database=&name=` —— 用户详情（`UserDetailView.vue` 用）。
///
/// 契约按界面取值定：**响应体本身就是详情对象** ——
/// `name` 进标题；`createUserSql` + `grants`（数组，每项一句授权）拼成「完整 SQL」；
/// 其余键被自动渲染成明细项。因此「没实现的方言」返回 `{error}` 而不是 HTTP 错误码
/// （界面专门认这个字段，HTTP 码只会变成一句「加载失败」）。
pub async fn user_info(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> XResult<Json<Value>> {
    let params = Params::parse(raw.as_deref());
    let name = params.get("name").unwrap_or_default();
    if name.trim().is_empty() {
        return Ok(Json(json!({ "error": "缺少 name 参数" })));
    }
    let database = params.get("database").unwrap_or_default();
    let (_, dialect) = ctx_of(&state, &id).await?;
    let sql = match dialect.user_info(name.trim()) {
        Meta::Sql(sql) => sql,
        _ => {
            return Ok(Json(json!({
                "error": format!(
                    "{} 的用户详情尚未接入（要按其系统表单独实现）",
                    dialect.kind.key().to_ascii_uppercase()
                )
            })))
        }
    };
    let result = run_sql_in(&state, &id, &database, sql, 1).await?;
    let Some(row) = rows_of(&result).into_iter().next() else {
        return Ok(Json(json!({ "error": format!("没有找到用户「{name}」") })));
    };
    // 权限明细：能**逐行查**的方言（MySQL 系）走第二条查询 + 本地拼行 ——
    // 不让 SQL 用 `group_concat` 拼一行，因为它有 1024 的会话上限会静默截断（见 `user_grants`）。
    // 其余方言（PG / SQL Server）本来就在 SQL 里用 `string_agg` / `for xml path` 拼好了
    // `grants_text`，且没有长度上限，所以沿用。
    let account_user = row
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let account_host = row
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let grants: Vec<Value> = match dialect.user_grants(&account_user, &account_host) {
        Meta::Sql(query) => {
            let detail = run_sql_in(&state, &id, &database, query, 10_000)
                .await
                .map(|result| rows_of(&result))
                .unwrap_or_default();
            detail
                .iter()
                .filter_map(|item| grant_line(item, &dialect, &account_user, &account_host))
                .map(|line| json!(line))
                .collect()
        }
        // `grants_text` 是换行分隔的文本，界面会对它 `.map()` —— 必须转成数组，
        // 给字符串会在前端直接抛异常
        _ => row
            .get("grants_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| json!(line))
            .collect(),
    };
    let mut out = Map::new();
    for (key, value) in row {
        if key == "grants_text" {
            continue;
        }
        // 口令散列不进界面：MySQL 的用户详情是 `select * from mysql.user`，
        // 里面带着认证串（5.7 的 `Password` / 8.0 的 `authentication_string`）。
        // 展示出来只会被复制、截图、进日志 —— 真要看得在 SQL 编辑器里自己查。
        if matches!(
            key.to_ascii_lowercase().as_str(),
            "authentication_string" | "password"
        ) {
            continue;
        }
        // 列名原样带出去：界面同时认 `createUserSql` 与 `create_user_sql`，
        // 其余键走 `fieldLabelMap` 渲染，不需要在这里做驼峰转换
        out.insert(key, value);
    }
    out.insert("grants".to_string(), Value::Array(grants));
    Ok(Json(Value::Object(out)))
}

/// `POST /api/{m}/{id}/user-action` —— 用户新建 / 改密 / 删除。
///
/// **细粒度权限如实拒绝**：界面会把 `privileges / dbPrivileges / tablePrivileges /
/// schemaPrivileges / objectPrivileges / serverRoles / grants` 一起发过来，而这些 GRANT
/// 我们还没实现。若默默忽略，界面会弹「用户创建成功」，用户以为权限也授好了 ——
/// 那比直接报错糟得多（他会带着错误认知继续用）。所以：只应用 `roles`（有明确语法），
/// 其余只要非空就**在动数据库之前**拒绝，并给出替代做法。
pub async fn user_action(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let action = body
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let user = body
        .get("userName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if user.is_empty() {
        return Err(XError::bad_request("缺少 userName"));
    }
    let password = body.get("password").and_then(Value::as_str).unwrap_or("");
    let host = body.get("host").and_then(Value::as_str).unwrap_or("");
    let database = body.get("database").and_then(Value::as_str).unwrap_or("");
    let preview_only = body
        .get("previewOnly")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let options = body.get("options").cloned().unwrap_or_else(|| json!({}));
    let (_, dialect) = ctx_of(&state, &id).await?;
    let mysql_family = matches!(dialect.kind.key(), "mysql" | "mariadb" | "doris");

    // 哪些权限入参已经真正实现：MySQL 系连接的三层（全局/库级/表级）—— 由
    // `user_action_sql` 拼出 `grant`/`revoke`；其余（模式级、对象级、服务器角色、
    // ClickHouse 的授权项）仍未实现。**只有确实支持的才放行**，不支持的一律明确拒绝，
    // 而不是默默建好用户却什么都没授。
    const UNSUPPORTED: [(&str, &str); 7] = [
        ("privileges", "全局权限"),
        ("dbPrivileges", "库级权限"),
        ("tablePrivileges", "表级权限"),
        ("schemaPrivileges", "模式级权限"),
        ("objectPrivileges", "对象级权限"),
        ("serverRoles", "服务器角色"),
        ("grants", "授权项"),
    ];
    if let Some(object) = options.as_object() {
        for (key, label) in UNSUPPORTED {
            let filled = object
                .get(key)
                .map(|value| match value {
                    Value::Array(items) => !items.is_empty(),
                    Value::String(text) => !text.trim().is_empty(),
                    Value::Null => false,
                    _ => true,
                })
                .unwrap_or(false);
            if !filled {
                continue;
            }
            let handled = mysql_family
                && matches!(key, "privileges" | "dbPrivileges" | "tablePrivileges");
            if !handled {
                return Err(XError::bad_request(format!(
                    "「{label}」的自动分配尚未实现：请先只做「新建用户 / 改密 / 删除用户」，\
                     再到查询页用 GRANT 授权（本次没有对数据库做任何改动）"
                )));
            }
        }
    }
    // 权限名会被**拼进 SQL**，所以逐个过白名单（见 `is_mysql_privilege`）
    let privileges = crate::api::dialect::UserPrivileges {
        global: checked_privileges(options.get("privileges"))?,
        databases: privilege_db_rows(options.get("dbPrivileges"))?,
        tables: privilege_table_rows(options.get("tablePrivileges"))?,
    };
    let roles: Vec<String> = options
        .get("roles")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    // 「权限以表单为准」：改权限时先清空该用户的授权，再按表单重授 ——
    // 否则「取消勾选」只是看起来取消了（旧授权还在）。清空只由这一条语句负责，
    // 细节（为什么不能逐目标 revoke）见 `user_action_sql` 里的注释。
    let reset_privileges = mysql_family && action == "alter";

    let sql = match dialect.user_action_sql(
        &action,
        &user,
        password,
        host,
        &roles,
        &privileges,
        reset_privileges,
    ) {
        Meta::Sql(sql) => sql,
        _ => {
            return Err(XError::bad_request(format!(
                "{} 不支持「{action}」这个用户操作（也可能是缺参数，例如改密需要 password）",
                dialect.kind.key().to_ascii_uppercase()
            )))
        }
    };
    if preview_only {
        // 与「对象重命名」保持一致：先能只看语句、不动数据库
        return Ok(Json(json!({ "success": true, "sql": sql, "previewOnly": true })));
    }
    let statements: Vec<String> = dbmind_core::split_statements(dialect.kind.protocol(), &sql)
        .into_iter()
        .filter(|statement| !statement.trim().is_empty())
        .collect();
    let mut done = 0usize;
    for statement in &statements {
        // 建/删用户、授权都是写库：走跟随连接策略的入口
        if let Err(err) = run_write_sql_in(&state, &id, &database, statement.clone(), 1).await {
            // 同样要说清「第几条失败、前面几条已生效」——已建好的用户不会被自动回滚
            return Ok(Json(json!({
                "success": false,
                "message": format!(
                    "第 {}/{} 条执行失败：{}（前 {} 条已生效）",
                    done + 1,
                    statements.len(),
                    err.message,
                    done
                ),
            })));
        }
        done += 1;
    }
    Ok(Json(json!({
        "success": true,
        "message": format!("已执行 {done} 条语句"),
        "sql": sql,
    })))
}

/// 把 `user_grants` 的一行（scope / privilege / db_name / table_name）拼成
/// `GRANT <权限> ON <目标> TO <账号>;` 文本 —— 与 MySQL `SHOW GRANTS` 同形，
/// 界面（`parseMySqlGrants`）按它分全局 / 库级 / 表级三层。
fn grant_line(
    row: &Map<String, Value>,
    dialect: &crate::api::dialect::Dialect,
    account_user: &str,
    account_host: &str,
) -> Option<String> {
    // ClickHouse：`show grants for` 的每行**本身就是**一条 GRANT 语句，
    // 原样带回即可 —— 没有 scope/privilege 列，也不需要重组（单列结果）。
    if dialect.kind.key() == "clickhouse" {
        return row.values().find_map(Value::as_str).map(str::to_string);
    }
    let scope = row.get("scope").and_then(Value::as_str).unwrap_or("");
    let privilege = row.get("privilege").and_then(Value::as_str).unwrap_or("");
    let db_name = row.get("db_name").and_then(Value::as_str).unwrap_or("");
    let table_name = row.get("table_name").and_then(Value::as_str).unwrap_or("");
    if privilege.is_empty() || account_user.is_empty() {
        return None;
    }
    let target = match scope {
        "global" => "*.*".to_string(),
        "schema" if !db_name.is_empty() => format!("{}.*", dialect.quote(db_name)),
        "table" if !db_name.is_empty() && !table_name.is_empty() => {
            format!("{}.{}", dialect.quote(db_name), dialect.quote(table_name))
        }
        _ => return None,
    };
    Some(format!(
        "GRANT {privilege} ON {target} TO {}@{};",
        dialect.literal(account_user),
        dialect.literal(account_host)
    ))
}

/// 权限名走白名单（它们会被拼进 SQL），大小写不敏感、统一大写、去重。
///
/// `ALL PRIVILEGES` 单独归一：它和具体权限并列（`GRANT ALL PRIVILEGES, SELECT ON ...`）
/// 是有歧义的写法，而语义上 `ALL` 已经覆盖后者 —— 只留 `ALL PRIVILEGES` 一条，
/// 生成出来的语句既短又不会有歧义。
fn checked_privileges(value: Option<&Value>) -> XResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for item in value.and_then(Value::as_array).cloned().unwrap_or_default() {
        let name = item.as_str().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        if !crate::api::dialect::is_mysql_privilege(name) {
            return Err(XError::bad_request(format!(
                "不认识的权限名「{name}」：已拒绝执行 —— 权限名会拼进授权语句，\
                 不在白名单里的一律不拼"
            )));
        }
        let upper = name.to_ascii_uppercase();
        if !out.contains(&upper) {
            out.push(upper);
        }
    }
    if out.iter().any(|name| name == "ALL PRIVILEGES") {
        return Ok(vec!["ALL PRIVILEGES".to_string()]);
    }
    Ok(out)
}

/// 库级权限行：`[{database, privileges}]`。
fn privilege_db_rows(value: Option<&Value>) -> XResult<Vec<(String, Vec<String>)>> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for row in value.and_then(Value::as_array).cloned().unwrap_or_default() {
        let database = row
            .get("database")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if database.is_empty() {
            continue;
        }
        out.push((database, checked_privileges(row.get("privileges"))?));
    }
    Ok(out)
}

/// 表级权限行：`[{database, table, privileges}]`。
fn privilege_table_rows(value: Option<&Value>) -> XResult<Vec<(String, String, Vec<String>)>> {
    let mut out: Vec<(String, String, Vec<String>)> = Vec::new();
    for row in value.and_then(Value::as_array).cloned().unwrap_or_default() {
        let database = row
            .get("database")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let table = row
            .get("table")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if database.is_empty() || table.is_empty() {
            continue;
        }
        out.push((
            database,
            table,
            checked_privileges(row.get("privileges"))?,
        ));
    }
    Ok(out)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn pick(names: &[&str], values: &[&str]) -> usize {
        let names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
        let values: Vec<String> = values.iter().map(|value| value.to_string()).collect();
        ddl_column_index(&names, &values)
    }

    /// MySQL `show create view` 的真实列名（实测，含 `serde_json` 排序后的样子）。
    ///
    /// 这条守着一次踩过两次的 bug：Map 按键名排序 ⇒「取第几列」毫无意义。
    /// 表现是「编辑视图 v1」页签里只有一行 `v1`（取到了视图名列），
    /// 表的建表语句同理只显示一个 `sales`。两次都让人以为是功能没做。
    #[test]
    fn 视图取到的是_create_view_而不是视图名() {
        let names = [
            "character_set_client",
            "collation_connection",
            "Create View",
            "View",
        ];
        let values = [
            "utf8mb4",
            "utf8mb4_general_ci",
            "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`%` SQL SECURITY DEFINER VIEW `v1` AS select 1",
            "v1",
        ];
        assert_eq!(names[pick(&names, &values)], "Create View");
    }

    /// MySQL `show create trigger` 的 DDL 列**不叫** `Create Trigger`，叫
    /// `SQL Original Statement`；而且 body 常常很短（本例 30 来字符），
    /// 长不过 sql_mode —— 只靠「最长值」兜底会挑出 sql_mode 贴进编辑器。
    #[test]
    fn 触发器取到的是_sql_original_statement_而不是_sql_mode() {
        let names = [
            "Database",
            "SQL Original Statement",
            "Trigger",
            "character_set_client",
            "collation_connection",
            "sql_mode",
        ];
        let values = [
            "上游_lock_probe",
            "CREATE DEFINER=`root`@`%` TRIGGER tr_lock BEFORE INSERT ON t_lock FOR EACH ROW SET NEW.a = IFNULL(NEW.a, 0) + 1",
            "tr_lock",
            "utf8mb4",
            "utf8mb4_unicode_ci",
            "STRICT_TRANS_TABLES,NO_ZERO_IN_DATE,NO_ZERO_DATE,ERROR_FOR_DIVISION_BY_ZERO,NO_AUTO_CREATE_USER,NO_ENGINE_SUBSTITUTION",
        ];
        assert_eq!(names[pick(&names, &values)], "SQL Original Statement");
    }

    /// 例程（`Create Procedure` / `Create Function`）与单列情形（SQLite 的 `sql`）。
    #[test]
    fn 例程与单列情形() {
        let names = ["Create Procedure", "Database", "Procedure", "sql_mode"];
        let values = [
            "CREATE PROCEDURE `p`() BEGIN END",
            "demo",
            "p",
            "ONLY_FULL_GROUP_BY",
        ];
        assert_eq!(names[pick(&names, &values)], "Create Procedure");
        assert_eq!(pick(&["sql"], &["create table t(a int)"]), 0);
    }

    /// 列名都不认识时，退回「值最长的那一列」——DDL 一定比表名、字符集长。
    #[test]
    fn 不认识的列名退回最长值() {
        let names = ["whatever", "table_name"];
        let values = ["CREATE TABLE `t` (a int, b varchar(10))", "t"];
        assert_eq!(names[pick(&names, &values)], "whatever");
    }

    /// 权限名会被**拼进授权语句**，所以只认白名单：伪造值必须被拒，
    /// 而不是「拼进去试试」——拼错了就是授多了或授错了。
    #[test]
    fn 权限名只认白名单() {
        assert_eq!(
            checked_privileges(Some(&json!(["select", "INSERT"]))).unwrap(),
            vec!["SELECT", "INSERT"]
        );
        assert!(checked_privileges(Some(&json!(["HACK; drop database dify"]))).is_err());
        assert!(checked_privileges(Some(&json!(["TOTALLY_FAKE"]))).is_err());
        assert!(checked_privileges(None).unwrap().is_empty());
        // 表单的「全部权限」就是 ALL PRIVILEGES（合法写法）；与具体权限并列时归一成 ALL
        assert_eq!(
            checked_privileges(Some(&json!(["ALL PRIVILEGES", "SELECT"]))).unwrap(),
            vec!["ALL PRIVILEGES"]
        );
    }

    /// 库级/表级权限行：缺库名或表名的行直接跳过 —— 宁可少授，也不拼出半截 SQL。
    #[test]
    fn 权限行的解析与过滤() {
        let rows = privilege_table_rows(Some(&json!([
            { "database": "dify", "table": "boxoffice", "privileges": ["SELECT"] },
            { "database": "dify", "privileges": ["SELECT"] }
        ])))
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "dify");
        assert_eq!(rows[0].1, "boxoffice");

        let dbs = privilege_db_rows(Some(&json!([
            { "database": "", "privileges": ["SELECT"] },
            { "database": "dify", "privileges": ["SELECT", "UPDATE"] }
        ])))
        .unwrap();
        assert_eq!(dbs.len(), 1);
        assert_eq!(dbs[0].1, vec!["SELECT", "UPDATE"]);
    }
}
