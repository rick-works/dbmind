//! `/api/sync/*` —— 数据同步（把源对象的**结构**与**数据**搬到目标）。
//!
//! ## 两条入口
//!
//! | 入口 | 场景 | 形态 |
//! |---|---|---|
//! | `POST /api/sync` | 单表同步 | 同步返回 `{inserted, updated, skipped, errors, …}` |
//! | `POST /api/sync/db` | 多对象同步 | 异步任务（进度 + 逐对象结果 + 可取消） |
//!
//! ## 四个决定
//!
//! 1. **逐对象给结果，不吞错**。整库同步里某张表失败很正常（没权限、类型不支持、
//!    主键冲突），把它记进 `results[]` 里继续做下一张 —— 但**绝不算成成功**。
//!    界面上一眼能看到「哪张表失败、为什么」。
//! 2. **`upsert` 用「先分后写」**：先把目标表已有的主键读进来，源行分成「新增」与
//!    「更新」两拨，再批量插入 + 逐行更新。比起「一条一条先 UPDATE 再 INSERT」省一半往返，
//!    而且不依赖任何方言特有的 upsert 语法（`ON CONFLICT` / `ON DUPLICATE KEY` 各家不同）。
//! 3. **没有主键就不做 upsert**：只能退化成「仅插入」并**明确写在结果里**。
//!    没有键的「更新」只能靠全列匹配，那会改错行 —— 宁可不做。
//! 4. **不支持的同步能力要列出来**（视图/函数/过程/触发器/事件）。
//!    它们会出现在结果里，状态是 `skipped` + 一句原因，而不是消失不见了。
//! 5. **按页流式，不设总行数上限**：读一页（`PAGE` 行）就写一页。
//!    早先是「整表读进内存 → 再写」，所以不得不留个 20 万行的上限 ——
//!    而大表同步到一半被截断，界面上只是一句轻描淡写的注记，比慢一点糟糕得多。
//!    流式之后：内存占用只与一页有关、第一页写完即可见、取消在页边界就能生效，
//!    上限自然也就没有存在的理由了。

use axum::extract::{Path, State};
use axum::Json;
use dbmind_core::{CellValue, ColumnDetail, TableKind};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::export::sql_literal;
use crate::api::meta::run_sql_in;
use crate::api::{blocking, require_record};
use crate::AppState;

/// 拉数据时的翻页大小 —— **同步没有总行数上限**（见文件头决定 5）。
///
/// 原来这里有个 `MAX_ROWS = 200_000`：因为旧实现是「先把整表读进内存，再开始写」，
/// 不留个上限内存会被撑爆。现在改成**按页流式**（读一页写一页），
/// 内存占用只与这一页有关、与表大小无关，上限就没有存在理由了：
/// 大表同步到 20 万行处被静默截断，比慢一点糟糕得多。
const PAGE: u64 = 5_000;
/// 一条语句最多拼多少个值元组（再大就可能顶到驱动的语句长度或包体上限）。
const MAX_TUPLES_PER_STATEMENT: u64 = 500;
/// 进度状态最短刷新间隔：写一页更新一次就够，**不必每行**都更新
/// （20 万行就是 20 万次加锁 + 20 万次前端快照变化，纯属白烧 CPU）。
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(300);

// ------------------------------------------------------------------ 请求

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncRequest {
    #[serde(default)]
    source_connection_id: String,
    #[serde(default)]
    target_connection_id: Option<String>,
    #[serde(default)]
    source_database: Option<String>,
    #[serde(default)]
    source_schema: Option<String>,
    #[serde(default)]
    source_table: Option<String>,
    #[serde(default)]
    target_database: Option<String>,
    #[serde(default)]
    target_schema: Option<String>,
    #[serde(default)]
    target_table: Option<String>,
    /// 单表接口里可能是 `"true"` 字符串，整库接口里是布尔 —— 两种都认（见 `flag`）
    #[serde(default)]
    sync_structure: Option<Value>,
    #[serde(default)]
    sync_data: Option<Value>,
    /// 单表接口的写入模式
    #[serde(default)]
    mode: Option<String>,
    /// 整库接口的写入模式
    #[serde(default)]
    data_mode: Option<String>,
    #[serde(default)]
    object_policy: Option<String>,
    #[serde(default)]
    batch_size: Option<u64>,
    #[serde(default)]
    tables: Vec<String>,
    #[serde(default)]
    views: Vec<String>,
    #[serde(default)]
    functions: Vec<String>,
    #[serde(default)]
    procedures: Vec<String>,
    #[serde(default)]
    triggers: Vec<String>,
    #[serde(default)]
    events: Vec<String>,
}

/// 布尔字段的宽容解析：前端的 `syncStructure` 在单表接口里是字符串、整库接口里是布尔。
fn flag(value: &Option<Value>, fallback: bool) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => matches!(text.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        Some(Value::Null) | None => fallback,
        Some(other) => other.as_bool().unwrap_or(fallback),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Policy {
    /// 目标已存在 ⇒ 保留结构，只同步数据
    Merge,
    /// 目标已存在 ⇒ 先删再按源结构重建
    Drop,
}

#[derive(Clone, Copy, PartialEq)]
enum DataMode {
    Upsert,
    Insert,
    TruncateInsert,
}

#[derive(Clone)]
struct Opts {
    policy: Policy,
    data_mode: DataMode,
    sync_structure: bool,
    sync_data: bool,
    batch: u64,
}

impl SyncRequest {
    fn target_connection(&self) -> String {
        self.target_connection_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.source_connection_id.clone())
    }

    fn opts(&self) -> Opts {
        let policy = match self.object_policy.as_deref().unwrap_or("merge").trim() {
            "drop" => Policy::Drop,
            // `skip` 在上游的老版本里表示「不动已有对象」，语义上等同于「保留结构只同步数据」
            _ => Policy::Merge,
        };
        let raw_mode = self
            .data_mode
            .clone()
            .or_else(|| self.mode.clone())
            .unwrap_or_else(|| "upsert".to_string());
        let data_mode = match raw_mode.trim() {
            "insert" => DataMode::Insert,
            "truncate_insert" => DataMode::TruncateInsert,
            _ => DataMode::Upsert,
        };
        Opts {
            policy,
            data_mode,
            sync_structure: flag(&self.sync_structure, true),
            sync_data: flag(&self.sync_data, true),
            batch: self.batch_size.unwrap_or(500).clamp(1, 5000),
        }
    }
}

/// 一侧的连接信息。
#[derive(Clone)]
struct Side {
    conn: String,
    scope: String,
    schema: Option<String>,
}

fn scope_of(database: &Option<String>, schema: &Option<String>) -> String {
    let database = database.clone().unwrap_or_default();
    let schema = schema.clone().unwrap_or_default();
    if schema.trim().is_empty() {
        return database;
    }
    if database.trim().is_empty() {
        return schema;
    }
    format!("{database}.{schema}")
}

fn qualified(table: &str, schema: &Option<String>) -> String {
    let schema = schema.clone().unwrap_or_default();
    if schema.trim().is_empty() || table.contains('.') {
        table.to_string()
    } else {
        format!("{schema}.{table}")
    }
}

// ------------------------------------------------------------------ 元数据

/// 取表的列（先试「模式.表」，失败退回「表」；不存在时返回空而不是报错 ——
/// 「目标表不存在」是一个正常的同步前置条件，不是异常）。
async fn columns_of(
    state: &AppState,
    side: &Side,
    table: &str,
) -> XResult<Vec<ColumnDetail>> {
    let target = crate::api::scope::resolve(state, &side.conn, &side.scope).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let names = {
        let qualified = qualified(table, &side.schema);
        if qualified == table {
            vec![table.to_string()]
        } else {
            vec![qualified, table.to_string()]
        }
    };
    for name in names {
        let engine = state.engine.clone();
        let target = target.clone();
        let found = blocking(move || engine.list_columns_fresh(&target, &name)).await;
        if let Ok(mut columns) = found {
            if !columns.is_empty() {
                // **必须补类型长度**：内核给的是裸类型名（MySQL 的 JDBC metadata 里
                // `name` 列就是 `varchar`，长度在另一列），直接拿去拼建表语句会得到
                // `\`name\` varchar not null` —— MySQL 直接报语法错（实测：跨库同步
                // 建表失败「You have an error in your SQL syntax」，而例程/视图那两条路是好的）。
                // 与 `/columns` 接口共用同一套补全，口径一致。
                crate::api::meta::enrich_column_types(
                    state,
                    &side.conn,
                    &side.scope,
                    table,
                    &mut columns,
                )
                .await;
                return Ok(columns);
            }
        }
    }
    Ok(Vec::new())
}

async fn run_sql(state: &AppState, side: &Side, sql: &str) -> XResult<()> {
    let target = crate::api::scope::resolve(state, &side.conn, &side.scope).await?;
    let engine = state.engine.clone();
    let request = dbmind_core::QueryRequest {
        read_only: None,
        connection: target,
        sql: sql.to_string(),
        options: dbmind_core::QueryOptions {
            max_rows: 1,
            timeout_ms: 600_000,
        },
        execution_id: None,
        session: Some("internal:browse".to_string()),
    };
    blocking(move || engine.execute(request, dbmind_core::AccessContext::Web)).await?;
    Ok(())
}

/// 拉**一页**数据（`offset` 起，最多 `PAGE` 行）。
///
/// 调用方自己循环翻页 —— 同步是"读一页写一页"（见 `sync_table`），
/// upsert 前读目标主键也是逐页累积（见那里的循环）。这样内存占用只与一页有关。
async fn fetch_page(
    state: &AppState,
    conn: &str,
    scope: &str,
    select_base: &str,
    dialect: Dialect,
    offset: u64,
) -> XResult<Vec<Vec<CellValue>>> {
    let page = dialect.limit_clause(offset, PAGE);
    let sql = if dialect.needs_order_by_for_paging() {
        format!("{select_base} order by (select null) {page}")
    } else {
        format!("select * from ({select_base}) dbmind_page {page}")
    };
    let result = run_sql_in(state, conn, scope, sql, PAGE as usize).await?;
    Ok(result.rows)
}

/// 拼「读取某张表某些列」的 select 前缀（翻页时反复用它）。
fn select_base(side: &Side, table: &str, columns: &[String], dialect: Dialect) -> String {
    let list = columns
        .iter()
        .map(|name| dialect.quote(name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("select {list} from {}", qualified(table, &side.schema))
}

// ------------------------------------------------------------------ 单表同步

struct ObjResult {
    ty: String,
    name: String,
    status: &'static str,
    message: String,
    inserted: u64,
    updated: u64,
}

impl ObjResult {
    fn ok(ty: &str, name: &str, message: impl Into<String>, inserted: u64, updated: u64) -> Self {
        Self {
            ty: ty.to_string(),
            name: name.to_string(),
            status: "ok",
            message: message.into(),
            inserted,
            updated,
        }
    }

    fn skipped(ty: &str, name: &str, message: impl Into<String>) -> Self {
        Self {
            ty: ty.to_string(),
            name: name.to_string(),
            status: "skipped",
            message: message.into(),
            inserted: 0,
            updated: 0,
        }
    }

    fn failed(ty: &str, name: &str, message: impl Into<String>) -> Self {
        Self {
            ty: ty.to_string(),
            name: name.to_string(),
            status: "error",
            message: message.into(),
            inserted: 0,
            updated: 0,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "type": self.ty,
            "name": self.name,
            "status": self.status,
            "message": self.message,
            "inserted": self.inserted,
            "updated": self.updated,
        })
    }
}

fn multi_row(dialect: Dialect) -> bool {
    matches!(
        dialect.kind.key(),
        "mysql" | "mariadb" | "doris" | "postgresql" | "kingbase" | "sqlite" | "sqlserver"
            | "h2" | "clickhouse"
    )
}

/// 单格 → SQL 字面量：目标列是**真布尔**时走布尔字面量。
///
/// 为什么需要：内核把布尔表示为 0/1 整数，而 PG / Oracle 的 boolean 列不接受整数 `0`
/// （报"列是 boolean，表达式是 integer"）—— 跨类型建表时我们自己就会给目标建出 boolean 列，
/// 所以这条必须跟上。
fn literal_for(cell: &CellValue, dialect: Dialect, target_type: Option<&str>) -> String {
    if let (CellValue::Integer(value), Some(type_name)) = (cell, target_type) {
        if Dialect::is_boolean_type(type_name) {
            return dialect.bool_literal(*value != 0).to_string();
        }
    }
    sql_literal(cell, dialect)
}

fn build_insert(
    table: &str,
    columns: &[String],
    types: &[Option<String>],
    rows: &[Vec<CellValue>],
    dialect: Dialect,
) -> String {
    let tuples: Vec<String> = rows
        .iter()
        .map(|row| {
            let values: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(index, cell)| {
                    literal_for(cell, dialect, types.get(index).and_then(|ty| ty.as_deref()))
                })
                .collect();
            format!("({})", values.join(", "))
        })
        .collect();
    format!(
        "insert into {table} ({}) values {}",
        columns
            .iter()
            .map(|name| dialect.quote(name))
            .collect::<Vec<_>>()
            .join(", "),
        tuples.join(", ")
    )
}

fn build_update(
    table: &str,
    columns: &[String],
    types: &[Option<String>],
    row: &[CellValue],
    key_indexes: &[usize],
    dialect: Dialect,
) -> String {
    let type_at = |index: usize| types.get(index).and_then(|ty| ty.as_deref());
    let assignments: Vec<String> = columns
        .iter()
        .enumerate()
        .filter(|(index, _)| !key_indexes.contains(index))
        .map(|(index, name)| {
            let value = row
                .get(index)
                .map(|cell| literal_for(cell, dialect, type_at(index)))
                .unwrap_or_else(|| "null".to_string());
            format!("{} = {value}", dialect.quote(name))
        })
        .collect();
    let where_clause: Vec<String> = key_indexes
        .iter()
        .map(|index| {
            let value = row
                .get(*index)
                .map(|cell| literal_for(cell, dialect, type_at(*index)))
                .unwrap_or_else(|| "null".to_string());
            format!("{} = {value}", dialect.quote(&columns[*index]))
        })
        .collect();
    // 更新语句按目标方言选：ClickHouse 只认 `ALTER TABLE ... UPDATE ... WHERE`
    dialect.update_sql(table, &assignments.join(", "), &where_clause.join(" and "))
}

/// 主键值 → 可比较的键（归一规则集中在 `shape`）。
///
/// 这里也必须归一：upsert 是"先读目标已有关键值、再拿源行的键来比"，
/// 两侧的同一列完全可能是不同变体（目标 `NUMBER` 回 real、源 `int` 回 integer），
/// 不归一会导致**每一行都被判成新增**（于是重复插入），而界面上仍显示同步成功。
fn key_of(row: &[CellValue], indexes: &[usize]) -> String {
    crate::api::shape::row_key(row, indexes)
}

/// 同步一张表。
#[allow(clippy::too_many_arguments)]
async fn sync_table(
    state: &AppState,
    task: Option<&std::sync::Arc<crate::api::tasks::Task>>,
    src: &Side,
    tgt: &Side,
    table: &str,
    target_table: &str,
    opts: &Opts,
) -> ObjResult {
    let ty = "table";
    let source_columns = match columns_of(state, src, table).await {
        Ok(columns) => columns,
        Err(err) => return ObjResult::failed(ty, table, format!("读取源表结构失败：{}", err.message)),
    };
    if source_columns.is_empty() {
        return ObjResult::failed(ty, table, "源表不存在或没有列信息");
    }
    let target_columns = columns_of(state, tgt, target_table)
        .await
        .unwrap_or_default();
    let target_kind = crate::api::dialect::Dialect::new(
        match require_record(state, &tgt.conn).await {
            Ok(record) => record.kind(),
            Err(err) => return ObjResult::failed(ty, table, err.message),
        },
    );
    // 源的方言在建表时就要用（跨类型要把列类型翻译到目标），所以提前到这里取
    let source_kind = Dialect::new(
        match require_record(state, &src.conn).await {
            Ok(record) => record.kind(),
            Err(err) => return ObjResult::failed(ty, table, err.message),
        },
    );
    // `None` = 同类型（照抄）；`Some(别的类型)` = 跨类型，类型与默认值都要翻译
    let cross_kind = if source_kind.kind == target_kind.kind {
        None
    } else {
        Some(source_kind.kind)
    };
    let target_name = target_kind.quote(target_table);
    let mut created = false;

    // 注释（表 + 字段）从**源表**取一次，供下面的「自动建表 / 删除重建」写进建表语句。
    // 内核的列元数据不带注释（`ColumnDetail` 没有 comment 字段），只有方言的元数据查询
    // 取得到 —— 取不到就是空表/None，**不编造**：注释拿不到时建表语句与以前完全一样。
    let src_column_comments =
        crate::api::meta::table_comments_map(state, &src.conn, &src.scope, table).await;
    let src_table_comment =
        crate::api::meta::table_comment_of(state, &src.conn, &src.scope, table).await;

    if target_columns.is_empty() {
        if !opts.sync_structure {
            return ObjResult::skipped(ty, table, "目标表不存在，且未勾选「目标表不存在时自动建表」");
        }
        // 带上源表的表注释与字段注释（见上面取注释那一段）
        let ddl = target_kind.create_table_with_comments(
            &target_name,
            &source_columns,
            cross_kind,
            &src_column_comments,
            src_table_comment.as_deref(),
        );
        if let Err(err) = run_sql(state, tgt, &ddl).await {
            return ObjResult::failed(ty, table, format!("建表失败：{}", err.message));
        }
        created = true;
    } else if opts.policy == Policy::Drop {
        if !opts.sync_structure {
            return ObjResult::failed(
                ty,
                table,
                "选择了「删除重建」，但没有勾选自动建表：那样会删掉目标表且建不回来",
            );
        }
        if let Err(err) = run_sql(state, tgt, &format!("drop table {target_name}")).await {
            return ObjResult::failed(ty, table, format!("删除目标表失败：{}", err.message));
        }
        // 带上源表的表注释与字段注释（见上面取注释那一段）
        let ddl = target_kind.create_table_with_comments(
            &target_name,
            &source_columns,
            cross_kind,
            &src_column_comments,
            src_table_comment.as_deref(),
        );
        if let Err(err) = run_sql(state, tgt, &ddl).await {
            return ObjResult::failed(ty, table, format!("重建目标表失败：{}", err.message));
        }
        created = true;
    }

    if !opts.sync_data {
        let message = if created { "已按源结构建表（未同步数据）" } else { "已跳过（未勾选同步数据）" };
        return ObjResult::ok(ty, table, message, 0, 0);
    }

    let target_columns = if created {
        source_columns.clone()
    } else {
        target_columns
    };
    // 只同步两边都有的列：目标多出来的列让它走自己的默认值/自增
    let mut columns: Vec<String> = Vec::new();
    // 二进制列：内核的结果集**只传长度、不传字节**（见 dbmind-core `CellValue` 的设计注释 ——
    // 结果集要跨进程/HTTP 传输）。传过去只可能是一列 NULL，而旧实现正是静默这么干的：
    // 数据丢了，界面上还是「同步成功」。这里明确把它排除在数据同步之外，并在结果里说清楚。
    let mut binary_columns: Vec<String> = Vec::new();
    for column in &source_columns {
        if !target_columns
            .iter()
            .any(|other| other.name.eq_ignore_ascii_case(&column.name))
        {
            continue;
        }
        if Dialect::is_binary_type(column.type_name.as_deref().unwrap_or("")) {
            binary_columns.push(column.name.clone());
            continue;
        }
        columns.push(column.name.clone());
    }
    if columns.is_empty() {
        let why = if binary_columns.is_empty() {
            "源表与目标表没有同名列，无法同步数据"
        } else {
            "需要同步的列全是二进制列：结果集只传长度不传字节，暂不支持同步二进制数据"
        };
        return ObjResult::skipped(ty, table, why);
    }
    let missing: Vec<String> = source_columns
        .iter()
        .filter(|column| !columns.iter().any(|name| name.eq_ignore_ascii_case(&column.name)))
        .filter(|column| {
            !binary_columns
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&column.name))
        })
        .map(|column| column.name.clone())
        .collect();

    // 主键：优先用目标的（目标是写入方），没有再用源的
    let mut key_columns: Vec<String> = target_columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.clone())
        .filter(|name| columns.iter().any(|column| column.eq_ignore_ascii_case(name)))
        .collect();
    if key_columns.is_empty() {
        key_columns = source_columns
            .iter()
            .filter(|column| column.primary_key)
            .map(|column| column.name.clone())
            .filter(|name| columns.iter().any(|column| column.eq_ignore_ascii_case(name)))
            .collect();
    }
    let key_indexes: Vec<usize> = key_columns
        .iter()
        .filter_map(|key| columns.iter().position(|column| column.eq_ignore_ascii_case(key)))
        .collect();

    // 目标侧各列的类型（按同名列对齐）：写值时要用它判"是不是真布尔列"。
    // 刚建出来的表按**映射后**的目标类型算，已存在的表用目标库自己报的类型。
    let target_types: Vec<Option<String>> = columns
        .iter()
        .map(|name| {
            if created {
                source_columns
                    .iter()
                    .find(|column| column.name.eq_ignore_ascii_case(name))
                    .and_then(|column| column.type_name.clone())
                    .map(|ty| target_kind.map_type(cross_kind, &ty))
            } else {
                target_columns
                    .iter()
                    .find(|column| column.name.eq_ignore_ascii_case(name))
                    .and_then(|column| column.type_name.clone())
            }
        })
        .collect();

    if let Some(task) = task {
        task.check_canceled().ok();
        task.set_phase(format!("读取源表 {table} 数据"));
    }

    let mut note = String::new();
    if !missing.is_empty() {
        note.push_str(&format!("（目标缺少 {} 列，未同步）", missing.join("/")));
    }
    // 源侧 select 前缀：下面按页反复用它（见 fetch_page / select_base）
    let base_select = select_base(src, table, &columns, source_kind);

    // 源与目标**是同一张表**（自同步）时，清空会把待读的数据删掉；那种"清空"本身也没有意义。
    // 旧实现是「先把整表读进内存、再清目标」，顺序上不怕；现在改成边读边写，必须显式区分。
    let same_table = src.conn == tgt.conn
        && src.scope.eq_ignore_ascii_case(&tgt.scope)
        && table.eq_ignore_ascii_case(target_table);
    if opts.data_mode == DataMode::TruncateInsert {
        if same_table {
            note.push_str("（源与目标是同一张表，已跳过清空）");
        } else {
            // 清空语句按**目标方言**选：ClickHouse 不吃裸 `delete from`（实测 position 26 语法错），
            // SQLite / Derby 又没有 `truncate` —— 见 `Dialect::clear_table_sql`
            let clear = target_kind.clear_table_sql(&target_name);
            if let Err(err) = run_sql(state, tgt, &clear).await {
                return ObjResult::failed(ty, table, format!("清空目标表失败：{}", err.message));
            }
        }
    }

    let mut inserted = 0u64;
    let mut updated = 0u64;
    // 计时只为在结果里给出吞吐（行/秒）：优化前后快了多少，界面上能直接看出来
    let started = std::time::Instant::now();
    let mut last_report = started;
    let effective_mode = if opts.data_mode == DataMode::Upsert && key_indexes.is_empty() {
        // 没有主键 ⇒ 判断不了「这一行是谁」，upsert 无从下手。
        //
        // 这里**不能**悄悄退化成「仅插入」：那样第二次同步会把整表再插一遍（行长翻倍），
        // 而界面上显示的却是「同步成功」——静默的数据翻倍比直接报错坏得多。
        // 唯一能安全放行的情况是目标表本来就是空的（首次同步），那时插入与 upsert 等价。
        let target_rows = target_kind.count_sql(&target_name);
        let empty = match run_sql_in(state, &tgt.conn, &tgt.scope, target_rows, 1).await {
            Ok(result) => result
                .rows
                .first()
                .and_then(|row| row.first())
                .and_then(|cell| match cell {
                    CellValue::Integer(value) => Some(*value),
                    _ => None,
                })
                .unwrap_or(-1)
                == 0,
            Err(_) => false,
        };
        if !empty {
            return ObjResult::failed(
                ty,
                table,
                format!(
                    "目标表 {target_table} 没有主键，无法按主键 upsert（目标表里已有数据）。\
                     请改用「仅插入」或「清空后导入」，或给目标表加上主键"
                ),
            );
        }
        note.push_str("（目标表没有主键，且当前为空：已按仅插入处理）");
        DataMode::Insert
    } else {
        opts.data_mode
    };

    // upsert：先把目标已有关键值读进来，源行分成「新增」与「更新」两拨。
    // 这里**逐页读、不设上限**：旧实现用 MAX_ROWS 截断，大表下会把「已有行」误判成「新增」，
    // 于是重复插入 —— 那是静默的数据错误，比慢一点糟得多。
    let mut existing: std::collections::HashSet<String> = std::collections::HashSet::new();
    if effective_mode == DataMode::Upsert {
        let key_positions: Vec<usize> = (0..key_indexes.len()).collect();
        let key_list = key_columns
            .iter()
            .map(|name| target_kind.quote(name))
            .collect::<Vec<_>>()
            .join(", ");
        let base = format!("select {key_list} from {target_name}");
        let mut key_offset = 0u64;
        loop {
            let page =
                match fetch_page(state, &tgt.conn, &tgt.scope, &base, target_kind, key_offset).await {
                    Ok(rows) => rows,
                    Err(err) => {
                        return ObjResult::failed(
                            ty,
                            table,
                            format!(
                                "读取目标表主键失败（upsert 需要它来判断新增还是更新）：{}",
                                err.message
                            ),
                        )
                    }
                };
            let got = page.len();
            for row in &page {
                existing.insert(key_of(row, &key_positions));
            }
            if got < PAGE as usize {
                break;
            }
            key_offset += PAGE;
        }
    }

    // SQL Server 的 identity 列**不接受显式插入**，而同步写过去的正是显式的键值。
    // 我们自己刚建的表（`identity(1,1)`）尤其需要它 —— 否则第一行就报
    // "不能为 identity 列插入显式值"。开关是会话级的，写完就关。
    let identity_on = created
        && target_kind.kind.key() == "sqlserver"
        && source_columns.iter().any(|column| column.auto_increment);
    if identity_on {
        if let Err(err) = run_sql(state, tgt, &format!("set identity_insert {target_name} on")).await {
            return ObjResult::failed(ty, table, format!("开启 identity_insert 失败：{}", err.message));
        }
    }

    let per_statement = if multi_row(target_kind) {
        opts.batch.min(MAX_TUPLES_PER_STATEMENT)
    } else {
        1
    };
    // ===== 流式写入：读一页、写一页 =====
    // 旧实现是「整表读进内存 → 再写」，代价是三件事捆在一起：
    //   · 必须有 20 万行的上限，否则内存扛不住；
    //   · 第一行要等整表读完才开始写；
    //   · 取消请求也要等整表读完才可能被检查到。
    // 改成页循环之后这三件事一起消失。跨页不再拼接批次（那只会让"已写行数"与真正
    // 落盘的行数差出一页，没有别的好处）。
    let mut pending_insert: Vec<Vec<CellValue>> = Vec::new();
    let mut offset = 0u64;
    loop {
        // 取消是协作式的：**每页**检查一次（原来每张表只有开头一次机会）
        if let Some(task) = task {
            if task.is_canceled() {
                note.push_str("（已取消：已写入的部分保留，不回滚）");
                break;
            }
        }
        let page = match fetch_page(state, &src.conn, &src.scope, &base_select, source_kind, offset).await
        {
            Ok(rows) => rows,
            Err(err) => return ObjResult::failed(ty, table, format!("读取源数据失败：{}", err.message)),
        };
        let got = page.len();
        if got == 0 {
            break;
        }
        for row in page {
            let is_existing = effective_mode != DataMode::Insert
                && !key_indexes.is_empty()
                && existing.contains(&key_of(&row, &key_indexes));
            if is_existing {
                // 更新逐行做：跨方言的批量 UPDATE 没有统一写法，硬拼会踩到方言差异
                let sql =
                    build_update(&target_name, &columns, &target_types, &row, &key_indexes, target_kind);
                match run_sql(state, tgt, &sql).await {
                    Ok(()) => updated += 1,
                    Err(err) => {
                        return ObjResult::failed(ty, table, format!("更新失败：{}", err.message))
                    }
                }
                continue;
            }
            pending_insert.push(row);
            if pending_insert.len() as u64 >= per_statement {
                match run_sql(
                    state,
                    tgt,
                    &build_insert(&target_name, &columns, &target_types, &pending_insert, target_kind),
                )
                .await
                {
                    Ok(()) => {
                        inserted += pending_insert.len() as u64;
                        pending_insert.clear();
                    }
                    Err(err) => {
                        return ObjResult::failed(ty, table, format!("插入失败：{}", err.message))
                    }
                }
            }
        }
        // 一页写完就把残余批次落盘
        if !pending_insert.is_empty() {
            match run_sql(
                state,
                tgt,
                &build_insert(&target_name, &columns, &target_types, &pending_insert, target_kind),
            )
            .await
            {
                Ok(()) => {
                    inserted += pending_insert.len() as u64;
                    pending_insert.clear();
                }
                Err(err) => return ObjResult::failed(ty, table, format!("插入失败：{}", err.message)),
            }
        }
        // 进度**按时间节流**：写一页更新一次足矣。旧实现每行都写一次 phase，
        // 20 万行就是 20 万次加锁 + 20 万次前端快照变化 —— 那是纯白烧的 CPU。
        if let Some(task) = task {
            if last_report.elapsed() >= PROGRESS_INTERVAL {
                task.set_phase(format!("表 {table}：已写 {} 行", inserted + updated));
                last_report = std::time::Instant::now();
            }
        }
        if got < PAGE as usize {
            break;
        }
        offset += PAGE;
    }
    // 因取消而中途退出时，手里可能还压着最后一批没落盘的插入
    if !pending_insert.is_empty() {
        match run_sql(
            state,
            tgt,
            &build_insert(&target_name, &columns, &target_types, &pending_insert, target_kind),
        )
        .await
        {
            Ok(()) => inserted += pending_insert.len() as u64,
            Err(err) => return ObjResult::failed(ty, table, format!("插入失败：{}", err.message)),
        }
    }
    if identity_on {
        // 关掉它。这里失败也不影响已写入的数据（开关是会话级的，留着最多让后续显式插入继续可用）
        let _ = run_sql(state, tgt, &format!("set identity_insert {target_name} off")).await;
    }

    // 结果里带上吞吐：快了多少是能量出来的，不用凭感觉
    let elapsed = started.elapsed();
    let written = inserted + updated;
    let rate = if written > 0 && elapsed.as_millis() >= 50 {
        format!("，{:.0} 行/秒", written as f64 / elapsed.as_secs_f64())
    } else {
        String::new()
    };
    let message = format!(
        "新增 {inserted} 行，更新 {updated} 行{}{}{}",
        if created { "（已建表）" } else { "" },
        rate,
        note
    );
    ObjResult::ok(ty, table, message, inserted, updated)
}

/// 视图：只有能拿到完整 `CREATE VIEW` 的类型才同步（见文件头决定 4）。
async fn sync_view(
    state: &AppState,
    src: &Side,
    tgt: &Side,
    name: &str,
    opts: &Opts,
) -> ObjResult {
    let source_kind = Dialect::new(
        match require_record(state, &src.conn).await {
            Ok(record) => record.kind(),
            Err(err) => return ObjResult::failed("view", name, err.message),
        },
    );
    let Some(ddl_sql) = source_kind.ddl(name).sql() else {
        return ObjResult::skipped(
            "view",
            name,
            format!("{} 拿不到完整的视图定义，未同步", source_kind.kind.key().to_ascii_uppercase()),
        );
    };
    let result = match run_sql_in(state, &src.conn, &src.scope, ddl_sql, 5).await {
        Ok(result) => result,
        Err(err) => return ObjResult::failed("view", name, format!("读取视图定义失败：{}", err.message)),
    };
    let mut definition = None;
    for row in result.rows {
        for value in row {
            if let CellValue::Text(text) = value {
                // 行首 create 才算定义（`contains` 会把 MySQL 的 sql_mode 列也算进来，
                // 那列里含 NO_AUTO_CREATE_USER —— 详见 meta::looks_like_ddl）
                if crate::api::meta::looks_like_ddl(&text) {
                    definition = Some(text);
                }
            }
        }
    }
    let Some(definition) = definition else {
        return ObjResult::skipped("view", name, "源库里没有取到该视图的建视图语句");
    };
    let target_kind = Dialect::new(
        match require_record(state, &tgt.conn).await {
            Ok(record) => record.kind(),
            Err(err) => return ObjResult::failed("view", name, err.message),
        },
    );
    let target_name = target_kind.quote(name);
    if columns_of(state, tgt, name).await.map(|c| !c.is_empty()).unwrap_or(false)
        && opts.policy != Policy::Drop
    {
        return ObjResult::skipped("view", name, "目标已存在同名视图（保留结构策略下不动它）");
    }
    if opts.policy == Policy::Drop {
        let _ = run_sql(state, tgt, &format!("drop view if exists {target_name}")).await;
    }
    match run_sql(state, tgt, &definition).await {
        Ok(()) => ObjResult::ok("view", name, "已按源定义建视图", 0, 0),
        Err(err) => ObjResult::failed("view", name, format!("建视图失败：{}", err.message)),
    }
}

/// 例程 / 触发器 / 事件：定义同样从 `object_source` 取 —— 与「查看定义」「导出」同一条路。
///
/// 原先这四类在同步里是**一律跳过**的（`UNSUPPORTED_KINDS`），可它们跟视图没有本质区别：
/// 都能从源库取到完整 `create` 文本、都能在目标库执行。跳过的唯一后果是"整库同步后
/// 目标库少了这些对象"，而用户看报告时只得到一句"尚未实现"。
///
/// 「删除重建」策略需要先删目标对象：例程没有 `create or replace` 的通用写法，
/// 所以这里用 `drop X if exists`（方言支持才用，不支持就跳过并说明——
/// 见 `Dialect::drop_object_sql`）。
async fn sync_object_source(
    state: &AppState,
    src: &Side,
    tgt: &Side,
    kind: &str,
    name: &str,
    opts: &Opts,
) -> ObjResult {
    let source_kind = match require_record(state, &src.conn).await {
        Ok(record) => Dialect::new(record.kind()),
        Err(err) => return ObjResult::failed(kind, name, err.message),
    };
    let Some(sql) = source_kind.object_source(kind, name).sql() else {
        return ObjResult::skipped(
            kind,
            name,
            format!(
                "{} 取不到该对象的定义，未同步",
                source_kind.kind.key().to_ascii_uppercase()
            ),
        );
    };
    let result = match run_sql_in(state, &src.conn, &src.scope, sql, 5).await {
        Ok(result) => result,
        Err(err) => {
            return ObjResult::failed(kind, name, format!("读取定义失败：{}", err.message))
        }
    };
    // 取"像 DDL"的那个值：各家的定义列名不一样（definition / sql / Create View …），
    // 按内容判断比按列名稳（与导出、查看定义同一套判据）
    let mut definition = None;
    for row in result.rows {
        for value in row {
            if let CellValue::Text(text) = value {
                // 行首 create 才算定义（`contains` 会把 MySQL 的 sql_mode 列也算进来，
                // 那列里含 NO_AUTO_CREATE_USER —— 详见 meta::looks_like_ddl）
                if crate::api::meta::looks_like_ddl(&text) {
                    definition = Some(text);
                }
            }
        }
    }
    let Some(definition) = definition else {
        return ObjResult::skipped(kind, name, "源库里没有取到该对象的定义");
    };
    let target_kind = match require_record(state, &tgt.conn).await {
        Ok(record) => Dialect::new(record.kind()),
        Err(err) => return ObjResult::failed(kind, name, err.message),
    };
    if opts.policy == Policy::Drop {
        match target_kind.drop_object_sql(kind, name) {
            Some(drop_sql) => {
                // 目标没有这个对象时 `if exists` 不会报错，所以不看结果
                let _ = run_sql(state, tgt, &drop_sql).await;
            }
            None => {
                return ObjResult::skipped(
                    kind,
                    name,
                    format!(
                        "「删除重建」要先删目标对象，而 {} 不支持 `drop {kind} if exists` —— \
                         请先在目标库删除后再同步",
                        target_kind.kind.key().to_ascii_uppercase()
                    ),
                )
            }
        }
    }
    match run_sql(state, tgt, &definition).await {
        Ok(()) => ObjResult::ok(kind, name, "已按源定义创建", 0, 0),
        Err(err) => ObjResult::failed(kind, name, format!("创建失败：{}", err.message)),
    }
}

// ------------------------------------------------------------------ 处理器

/// `POST /api/sync` —— 单表同步（同步返回结果）。
pub async fn table(
    State(state): State<AppState>,
    Json(body): Json<SyncRequest>,
) -> XResult<Json<Value>> {
    let source_table = body
        .source_table
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| XError::bad_request("缺少 sourceTable 参数"))?;
    if body.source_connection_id.trim().is_empty() {
        return Err(XError::bad_request("缺少 sourceConnectionId 参数"));
    }
    let opts = body.opts();
    let target_table = body
        .target_table
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| source_table.clone());
    let src = Side {
        conn: body.source_connection_id.clone(),
        scope: scope_of(&body.source_database, &body.source_schema),
        schema: body.source_schema.clone(),
    };
    let tgt = Side {
        conn: body.target_connection(),
        scope: scope_of(&body.target_database, &body.target_schema),
        schema: body.target_schema.clone(),
    };
    require_record(&state, &src.conn).await?;
    require_record(&state, &tgt.conn).await?;
    let mode = body
        .data_mode
        .clone()
        .or_else(|| body.mode.clone())
        .unwrap_or_else(|| "upsert".to_string());

    let outcome = sync_table(&state, None, &src, &tgt, &source_table, &target_table, &opts).await;
    let success = outcome.status == "ok";
    Ok(Json(json!({
        "success": success,
        "inserted": outcome.inserted,
        "updated": outcome.updated,
        "skipped": if outcome.status == "skipped" { 1 } else { 0 },
        "errors": if success { 0 } else { 1 },
        "total": 1,
        "mode": mode,
        "message": outcome.message,
    })))
}

/// `POST /api/sync/db` —— 多对象同步（异步任务）。
pub async fn db(State(state): State<AppState>, Json(body): Json<SyncRequest>) -> XResult<Json<Value>> {
    if body.source_connection_id.trim().is_empty() {
        return Err(XError::bad_request("缺少 sourceConnectionId 参数"));
    }
    require_record(&state, &body.source_connection_id).await?;
    let target_conn = body.target_connection();
    require_record(&state, &target_conn).await?;

    // 对象清单：显式选中的；一个都没选就按「同步源库全部表」理解（界面允许直接点开始）
    let mut objects: Vec<(String, String)> = Vec::new();
    for name in &body.tables {
        objects.push(("table".to_string(), name.clone()));
    }
    for name in &body.views {
        objects.push(("view".to_string(), name.clone()));
    }
    for (kind, names) in [
        ("function", &body.functions),
        ("procedure", &body.procedures),
        ("trigger", &body.triggers),
        ("event", &body.events),
    ] {
        for name in names {
            objects.push((kind.to_string(), name.clone()));
        }
    }
    if objects.is_empty() {
        let src = Side {
            conn: body.source_connection_id.clone(),
            scope: scope_of(&body.source_database, &body.source_schema),
            schema: body.source_schema.clone(),
        };
        let target = crate::api::scope::resolve(&state, &src.conn, &src.scope).await?;
        crate::api::driver::ensure_for_connection(&state, &target).await?;
        let engine = state.engine.clone();
        let tables = blocking(move || engine.list_tables_fresh(&target)).await?;
        for table in tables
            .into_iter()
            .filter(|table| table.kind == TableKind::Table)
        {
            objects.push(("table".to_string(), table.name));
        }
    }
    if objects.is_empty() {
        return Err(XError::bad_request("没有要同步的对象"));
    }

    let opts = body.opts();
    let src = Side {
        conn: body.source_connection_id.clone(),
        scope: scope_of(&body.source_database, &body.source_schema),
        schema: body.source_schema.clone(),
    };
    let tgt = Side {
        conn: target_conn,
        scope: scope_of(&body.target_database, &body.target_schema),
        schema: body.target_schema.clone(),
    };
    let total = objects.len();

    let registry = state.tasks.clone();
    let task = registry.spawn("sync", "sync_", move |task| {
        let state = state.clone();
        let src = src.clone();
        let tgt = tgt.clone();
        let opts = opts.clone();
        async move {
            task.set_total(total as i64);
            let mut results: Vec<Value> = Vec::new();
            let mut ok = 0u64;
            let mut skipped = 0u64;
            let mut errors = 0u64;
            let mut inserted = 0u64;
            let mut updated = 0u64;
            for (index, (kind, name)) in objects.iter().enumerate() {
                if task.is_canceled() {
                    // 取消：剩下的对象标成 canceled（而不是假装没这回事）
                    for (kind, name) in objects.iter().skip(index) {
                        results.push(json!({
                            "type": kind, "name": name, "status": "canceled",
                            "message": "已取消", "inserted": 0, "updated": 0,
                        }));
                    }
                    break;
                }
                // 这里只说**对象名**：前缀「正在同步：」与「（已完成/总数）」由界面统一给。
                // 之前这里自己又写了一遍「同步 X（1/5）」，界面上就变成
                // 「正在同步：同步 X（1/5）（0/5）」—— 前缀和计数都重复了。
                task.set_phase(name.to_string());
                let outcome = match kind.as_str() {
                    "table" => {
                        sync_table(&state, Some(&task), &src, &tgt, name, name, &opts).await
                    }
                    "view" => sync_view(&state, &src, &tgt, name, &opts).await,
                    // 例程 / 触发器 / 事件：与视图同一条路（见 sync_object_source）
                    "procedure" | "function" | "trigger" | "event" => {
                        sync_object_source(&state, &src, &tgt, kind, name, &opts).await
                    }
                    other => ObjResult::skipped(other, name, "该对象类型不支持同步（结构、数据都没动）"),
                };
                match outcome.status {
                    "ok" => ok += 1,
                    "skipped" => skipped += 1,
                    _ => errors += 1,
                }
                inserted += outcome.inserted;
                updated += outcome.updated;
                if !outcome.message.is_empty() {
                    task.log(format!("{}：{}", name, outcome.message));
                }
                results.push(outcome.to_json());
                task.set_done(index as u64 + 1);
            }
            let summary = json!({
                "total": total,
                "ok": ok,
                "skipped": skipped,
                "cancelled": if task.is_canceled() { 1 } else { 0 },
                "errors": errors,
                "inserted": inserted,
                "updated": updated,
            });
            // 取消也是有结果的结果：文案上要和「完成」分开，
            // 否则界面会拿一句"同步完成"去报一个被取消的任务。
            let message = format!(
                "同步{}：成功 {ok} / 跳过 {skipped} / 失败 {errors}，新增 {inserted} 行，更新 {updated} 行",
                if task.is_canceled() { "已停止" } else { "完成" }
            );
            task.set_message(message.clone());
            Ok(Some(json!({
                "success": errors == 0,
                "summary": summary,
                "results": results,
                "message": message,
            })))
        }
    });

    Ok(Json(json!({
        "success": true,
        "taskId": task.id,
        "status": "running",
        "done": 0,
        "total": total,
        "current": "",
    })))
}

/// `GET /api/sync/task/{taskId}` —— 进度 + 逐对象结果。
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
    view["current"] = view["phase"].clone();
    if let Some(result) = task.result() {
        view["result"] = result;
    }
    Ok(Json(view))
}

/// `POST /api/sync/cancel/{taskId}` —— 请求取消。
pub async fn cancel(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    match state.tasks.get(&task_id) {
        Some(task) => {
            task.cancel();
            Ok(Json(json!({ "success": true, "message": "已请求取消，当前对象处理完后停止" })))
        }
        None => Ok(Json(json!({ "success": false, "message": "任务不存在或已过期" }))),
    }
}
