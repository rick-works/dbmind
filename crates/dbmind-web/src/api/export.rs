//! `/api/{m}/export/*` —— 数据导出与整库转储。
//!
//! ## 这一块的形状是谁定的
//!
//! 导出有三条路径，全都得在：
//!
//! | 路径 | 语义 | 返回 |
//! |---|---|---|
//! | `POST {m}/export/{id}` | **同步**导出当前一页（必须给 page/size） | 文件字节 |
//! | `POST {m}/export/task/{id}` | **异步**导出全部（后端自己分页） | `{taskId}` |
//! | `POST {m}/export/db\|dump-task/{id}` | 整库转储（可选对象范围） | 字节 / `{taskId}` |
//!
//! 前端的导出菜单只给 CSV / Excel / JSON 三项，而 Excel 是主路径 ——
//! 少了它，「导出」在界面上是一半死的（能点，但是 501）。
//!
//! ## 四个必须说清的决定
//!
//! 1. **全部导出走「包一层子查询再分页」**，不往用户 SQL 后面直接拼 `limit`：
//!    界面拼出来的 SQL 自带 `order by`（有的还带过滤），直接拼会撞车。
//!    SQL Server 例外 —— 它的子查询里不允许 `ORDER BY`，所以改成直接跟在原语句后面
//!    补 `order by (select null)` + `offset/fetch`（见 `paging_sql`）。
//! 2. **`total` 拿不到就给 -1**（界面会显示成不确定进度），不要给 0：
//!    0 会被读成「总量 0」，进度条直接满格。
//! 3. **整库转储的 csv/json/excel 一律打包成 zip**：
//!    一个库几十张表，散落成几十个文件没法下载。
//! 4. **没实现的对象类型要说出来**：存储过程/触发器/事件/函数的 DDL 生成还没做，
//!    那就把它们写进任务的 `logs` 与结果 `notices`，而不是让用户以为「本来就导不出」。
//!    少一个文件是小事，以为「备份完整」才是大事。

use std::fs::File;
use std::io::{BufWriter, Cursor, Write};
use std::path::{Path as FsPath, PathBuf};

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use dbmind_core::CellValue;
use serde_json::{json, Map, Value};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::meta::{rows_of, run_sql_in};
use crate::api::shape;
use crate::api::tasks::{self, Artifact, Task};
use crate::api::xlsx::{SheetSpec, Workbook};
use crate::api::{blocking, require_record};
use crate::AppState;

/// 「导出全部」每页取多少行。
///
/// 太小 ⇒ 大表要多跑几十趟（每趟都是一次网络往返 + 驱动解析）；太大 ⇒ 单次响应把
/// 驱动的行缓存顶爆。5000 是这两者之间的常见折中。
/// 默认每页行数。
///
/// 为什么从 5000 提到 50000（实测，用 DBMIND_EXPORT_PROFILE=1 量的）：
/// 每次向宿主取一页都有约 340ms 的固定开销（管道往返 + 建 Statement + JDBC 执行），
/// 这部分与行数无关。20 万行按 5000/页要 41 次取数 ≈ 14 秒纯开销；按 50000/页只要 5 次。
/// 实测同一批 4 万行：1 次取数 2403ms，10 次取数 5537ms —— 页开大是这一轮最大的一刀。
/// 再大收益就平了（4 万行一次取数已到 60us/行的下限），所以 50000 够。
const PAGE_ROWS: u64 = 50000;

/// 页内进度上报的行数粒度。
///
/// 原来只在整页（PAGE_ROWS 行）写完后更新一次 → 界面上的行数只能 5000 一跳、日志更粗
/// （每 10 页才一条），看着就像卡住。改成页内按行上报后数字连续往上滚，
/// 取消检查也提到同一频率（点取消能立刻停）。
const REPORT_ROWS: u64 = 500;

/// 写一条日志的行数间隔。日志区是"过程流水"：太密会刷屏（MAX_LOGS 只有 200 条），
/// 太疏就像没在动（原来每 10 页才一条，大表上几分钟不见一行）。
const LOG_ROWS: u64 = 5000;

// ------------------------------------------------------------------ 格式

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Csv,
    Json,
    Xlsx,
    SqlInsert,
    SqlUpdate,
    SqlDelete,
    Ddl,
}

impl Default for Format {
    fn default() -> Self {
        Format::Csv
    }
}

impl Format {
    /// 前端可能传来的格式名（含历史别名）。
    pub fn parse(raw: &str) -> XResult<Self> {
        Ok(match raw.trim().to_ascii_lowercase().as_str() {
            "" | "csv" => Format::Csv,
            "json" => Format::Json,
            "excel" | "xlsx" => Format::Xlsx,
            // `sql` 就是 INSERT 语句，与 sql-insert 同义；
            // 整库转储时它表示「DDL + 数据」，见 `export_dump`
            "sql" | "sql-insert" => Format::SqlInsert,
            "sql-update" => Format::SqlUpdate,
            "sql-delete" => Format::SqlDelete,
            "ddl" | "sql-ddl" => Format::Ddl,
            other => return Err(XError::bad_request(format!("不支持的导出格式：{other}"))),
        })
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Xlsx => "xlsx",
            Format::SqlInsert | Format::SqlUpdate | Format::SqlDelete | Format::Ddl => "sql",
        }
    }

    pub fn media(self) -> &'static str {
        match self {
            Format::Csv => "text/csv; charset=UTF-8",
            Format::Json => "application/json; charset=UTF-8",
            Format::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            _ => "text/plain; charset=UTF-8",
        }
    }

    /// 名字给人看（报错文案里用）。
    pub fn label(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Json => "JSON",
            Format::Xlsx => "Excel",
            Format::SqlInsert => "SQL INSERT",
            Format::SqlUpdate => "SQL UPDATE",
            Format::SqlDelete => "SQL DELETE",
            Format::Ddl => "DDL",
        }
    }

    /// UPDATE / DELETE 靠主键定位行，没有主键列就没法生成 —— 要提前报错。
    fn needs_pk(self) -> bool {
        matches!(self, Format::SqlUpdate | Format::SqlDelete)
    }

    /// 这个格式是否要读数据行。
    ///
    /// DDL 只查元数据、天然没有「第几页」，所以同步导出对 page/size 的硬要求得把它排除掉 ——
    /// 否则「导出建表语句」会收到一句「必须指定 page 与 size」，而用户根本无从理解。
    fn needs_rows(self) -> bool {
        self != Format::Ddl
    }

    /// 整库转储时是否需要打包（数据类格式，一个表一个文件）。
    fn packed(self) -> bool {
        matches!(self, Format::Csv | Format::Json | Format::Xlsx)
    }
}

// ------------------------------------------------------------------ 请求

#[derive(Default, Clone)]
pub struct ExportReq {
    pub database: String,
    pub sql: String,
    pub table: String,
    pub format: Format,
    pub page: Option<u64>,
    pub size: Option<u64>,
    pub pk: Vec<String>,
    /// 整库转储的对象范围（空 = 全部表）
    pub tables: Vec<String>,
    pub views: Vec<String>,
    pub procedures: Vec<String>,
    pub functions: Vec<String>,
    pub triggers: Vec<String>,
    pub events: Vec<String>,
    pub include_drop: bool,
    pub include_data: bool,
    /// true = 导出全部（异步），false = 只导当前一页（同步）
    pub all: bool,
}

impl ExportReq {
    pub fn parse(body: &Value, all: bool) -> XResult<Self> {
        let options = body.get("options").cloned().unwrap_or(Value::Null);
        Ok(Self {
            database: field_str(body, "database"),
            sql: field_str(body, "sql"),
            table: field_str(body, "table"),
            format: Format::parse(&field_str(body, "format"))?,
            page: field_u64(body, "page"),
            size: field_u64(body, "size"),
            pk: split_list(&field_str(body, "pkColumns")),
            tables: field_list(body, "tables"),
            views: field_list(body, "views"),
            procedures: field_list(body, "procedures"),
            functions: field_list(body, "functions"),
            triggers: field_list(body, "triggers"),
            events: field_list(body, "events"),
            include_drop: option_bool(&options, "includeDrop", true),
            include_data: option_bool(&options, "includeData", true),
            all,
        })
    }

    /// 建任务之前的**前置校验**。
    ///
    /// 为什么不在任务里才发现：任务一旦创建，界面上出现的是「任务失败」+ 一段错误，
    /// 而参数不对本该立刻以「参数不对」的样子出现（HTTP 400 + 一句人话）。
    fn validate(&self) -> XResult<()> {
        if self.format.needs_pk() && self.pk.is_empty() {
            return Err(XError::bad_request(format!(
                "{} 导出需要主键列（用于生成精确的 WHERE 条件），当前没有取到主键",
                self.format.label()
            )));
        }
        if self.sql.trim().is_empty() && self.table.trim().is_empty() && self.tables.is_empty() {
            return Err(XError::bad_request("缺少 sql 或 table 参数"));
        }
        Ok(())
    }
}

fn field_str(body: &Value, key: &str) -> String {
    match body.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn field_u64(body: &Value, key: &str) -> Option<u64> {
    match body.get(key) {
        Some(Value::Number(number)) => number.as_u64().or_else(|| number.as_f64().map(|f| f as u64)),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

fn option_bool(body: &Value, key: &str, fallback: bool) -> bool {
    match body.get(key).and_then(Value::as_bool) {
        Some(value) => value,
        None => fallback,
    }
}

/// 数组形式的字符串列表；也认「逗号分隔的一个字符串」。
fn field_list(body: &Value, key: &str) -> Vec<String> {
    match body.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(text) => Some(text.clone()),
                Value::Null => None,
                other => Some(other.to_string()),
            })
            .filter(|item| !item.trim().is_empty())
            .collect(),
        Some(Value::String(text)) => split_list(text),
        _ => Vec::new(),
    }
}

fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

// ------------------------------------------------------------------ 写入器

/// 把「列 + 行」写成某种格式。
///
/// 为什么是 trait 而不是一堆 `if`：七种格式各自有状态（CSV 的表头、JSON 的逗号、
/// SQL 的列序、xlsx 的容器），塞进一个 match 就得在每个分支里重写一遍状态机，
/// 而其中只有「写一行」这半句是相同的。
/// `Send` 是硬要求：导出全部跑在 tokio 任务里，sink 会**跨 await 点存活**
/// （每拉一页就在同一个 sink 上写一行），非 Send 的 sink 会让整个 future 不能跨线程。
trait RowSink: Send {
    fn begin(&mut self, columns: &[String]) -> Result<(), String>;
    fn row(&mut self, cells: &[CellValue]) -> Result<(), String>;
    fn finish(&mut self) -> Result<(), String>;
}

fn io_error(err: std::io::Error) -> String {
    format!("写入导出文件失败：{err}")
}

/// 让 `Box<dyn RowSink>` 自身也是 `RowSink`。
///
/// 有了它，灌数据的函数就能写 `&mut S where S: RowSink`，而不是
/// `&mut Box<dyn RowSink + 'a>` —— 后者在借用检查里是**不变**的（invariant），
/// 传一个 `Box<dyn RowSink + 'static>` 进去会直接编译不过。
impl<T: RowSink + ?Sized> RowSink for Box<T> {
    fn begin(&mut self, columns: &[String]) -> Result<(), String> {
        (**self).begin(columns)
    }

    fn row(&mut self, cells: &[CellValue]) -> Result<(), String> {
        (**self).row(cells)
    }

    fn finish(&mut self) -> Result<(), String> {
        (**self).finish()
    }
}

/// CSV（RFC 4180 的实用子集：只有含分隔符/引号/换行的字段才加引号）。
struct CsvSink<W: Write> {
    out: W,
}

impl<W: Write + Send> RowSink for CsvSink<W> {
    fn begin(&mut self, columns: &[String]) -> Result<(), String> {
        // UTF-8 BOM：CSV 里没有声明编码的地方，少了它 Excel 会按本地代码页解，中文全是乱码
        self.out.write_all(&[0xEF, 0xBB, 0xBF]).map_err(io_error)?;
        let header: Vec<String> = columns.iter().map(|name| csv_field(name)).collect();
        write_line(&mut self.out, &header)
    }

    fn row(&mut self, cells: &[CellValue]) -> Result<(), String> {
        let fields: Vec<String> = cells.iter().map(csv_value).collect();
        write_line(&mut self.out, &fields)
    }

    fn finish(&mut self) -> Result<(), String> {
        self.out.flush().map_err(io_error)
    }
}

/// JSON 数组（一行一个对象，便于用 `head`/`grep` 看大文件）。
struct JsonSink<W: Write> {
    out: W,
    columns: Vec<String>,
    first: bool,
}

impl<W: Write + Send> RowSink for JsonSink<W> {
    fn begin(&mut self, columns: &[String]) -> Result<(), String> {
        self.columns = columns.to_vec();
        self.first = true;
        self.out.write_all(b"[").map_err(io_error)
    }

    fn row(&mut self, cells: &[CellValue]) -> Result<(), String> {
        let mut object = Map::new();
        for (index, name) in self.columns.iter().enumerate() {
            let value = cells.get(index).map(shape::cell_to_value).unwrap_or(Value::Null);
            object.insert(name.clone(), value);
        }
        let text = serde_json::to_string(&Value::Object(object)).map_err(|e| e.to_string())?;
        let separator = if self.first { "\n  " } else { ",\n  " };
        self.first = false;
        self.out.write_all(separator.as_bytes()).map_err(io_error)?;
        self.out.write_all(text.as_bytes()).map_err(io_error)
    }

    fn finish(&mut self) -> Result<(), String> {
        self.out.write_all(b"\n]\n").map_err(io_error)?;
        self.out.flush().map_err(io_error)
    }
}

/// INSERT 语句：一行一条。
///
/// 不做多行 `VALUES (...), (...)` 合并：Oracle / DB2 / Derby 对多行 VALUES 支持不一致，
/// 而导出文件是要拿去别的库执行的 —— 兼容性比体积重要。
struct SqlInsertSink<W: Write> {
    out: W,
    dialect: Dialect,
    table: String,
    columns: Vec<String>,
}

impl<W: Write + Send> RowSink for SqlInsertSink<W> {
    fn begin(&mut self, columns: &[String]) -> Result<(), String> {
        self.columns = columns.to_vec();
        let list: Vec<String> = self.columns.iter().map(|name| self.dialect.quote(name)).collect();
        writeln!(
            self.out,
            "-- 由 DBMind 导出于 {}\n-- 目标表 {}\n",
            tasks::stamp(),
            self.table
        )
        .map_err(io_error)?;
        // 列清单在表头写一次就够了（每行都重写一遍列名纯属浪费体积）
        self.out
            .write_all(format!("-- ({})\n", list.join(", ")).as_bytes())
            .map_err(io_error)
    }

    fn row(&mut self, cells: &[CellValue]) -> Result<(), String> {
        let values: Vec<String> = cells.iter().map(|cell| sql_literal(cell, self.dialect)).collect();
        let columns: Vec<String> = self.columns.iter().map(|name| self.dialect.quote(name)).collect();
        writeln!(
            self.out,
            "insert into {} ({}) values ({});",
            self.table,
            columns.join(", "),
            values.join(", ")
        )
        .map_err(io_error)
    }

    fn finish(&mut self) -> Result<(), String> {
        self.out.write_all(b"\n").map_err(io_error)?;
        self.out.flush().map_err(io_error)
    }
}

/// UPDATE 语句（按主键定位）。
struct SqlUpdateSink<W: Write> {
    out: W,
    dialect: Dialect,
    table: String,
    columns: Vec<String>,
    pk: Vec<String>,
    /// 主键列在结果里的下标（begin 时解析一次）
    pk_index: Vec<(usize, String)>,
}

impl<W: Write + Send> RowSink for SqlUpdateSink<W> {
    fn begin(&mut self, columns: &[String]) -> Result<(), String> {
        self.columns = columns.to_vec();
        self.pk_index = resolve_pk(columns, &self.pk)?;
        writeln!(self.out, "-- 由 DBMind 导出于 {}", tasks::stamp()).map_err(io_error)
    }

    fn row(&mut self, cells: &[CellValue]) -> Result<(), String> {
        let mut assignments = Vec::new();
        for (index, name) in self.columns.iter().enumerate() {
            if self.pk_index.iter().any(|(pk_index, _)| *pk_index == index) {
                continue;
            }
            let value = cells.get(index).map(|cell| sql_literal(cell, self.dialect)).unwrap_or_else(|| "NULL".to_string());
            assignments.push(format!("{} = {value}", self.dialect.quote(name)));
        }
        writeln!(
            self.out,
            "update {} set {} where {};",
            self.table,
            assignments.join(", "),
            where_clause(cells, &self.pk_index, self.dialect)
        )
        .map_err(io_error)
    }

    fn finish(&mut self) -> Result<(), String> {
        self.out.write_all(b"\n").map_err(io_error)?;
        self.out.flush().map_err(io_error)
    }
}

/// DELETE 语句（按主键定位）。
struct SqlDeleteSink<W: Write> {
    out: W,
    dialect: Dialect,
    table: String,
    pk: Vec<String>,
    pk_index: Vec<(usize, String)>,
}

impl<W: Write + Send> RowSink for SqlDeleteSink<W> {
    fn begin(&mut self, columns: &[String]) -> Result<(), String> {
        self.pk_index = resolve_pk(columns, &self.pk)?;
        writeln!(self.out, "-- 由 DBMind 导出于 {}", tasks::stamp()).map_err(io_error)
    }

    fn row(&mut self, cells: &[CellValue]) -> Result<(), String> {
        writeln!(
            self.out,
            "delete from {} where {};",
            self.table,
            where_clause(cells, &self.pk_index, self.dialect)
        )
        .map_err(io_error)
    }

    fn finish(&mut self) -> Result<(), String> {
        self.out.write_all(b"\n").map_err(io_error)?;
        self.out.flush().map_err(io_error)
    }
}

/// Excel（流式写进 zip 容器）。
struct XlsxSink<W: Write + std::io::Seek> {
    book: Option<Workbook<W>>,
    name: String,
}

impl<W: Write + std::io::Seek + Send> RowSink for XlsxSink<W> {
    fn begin(&mut self, columns: &[String]) -> Result<(), String> {
        let mut book = self
            .book
            .take()
            .ok_or_else(|| "xlsx：工作簿已被收尾".to_string())?;
        let spec = SheetSpec {
            name: self.name.clone(),
            headers: columns.to_vec(),
        };
        book.begin_sheet(&spec)?;
        self.book = Some(book);
        Ok(())
    }

    fn row(&mut self, cells: &[CellValue]) -> Result<(), String> {
        match self.book.as_mut() {
            Some(book) => book.row(cells),
            None => Err("xlsx：工作簿已被收尾".to_string()),
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        let book = self
            .book
            .take()
            .ok_or_else(|| "xlsx：工作簿已被收尾".to_string())?;
        // 容器部件在这里补齐；返回的底层 writer（文件）随作用域丢弃即可
        let _inner = book.finish()?;
        Ok(())
    }
}

/// 文本类写入器（CSV / JSON / SQL）。
fn make_text_sink<'a, W: Write + Send + 'a>(
    format: Format,
    out: W,
    table: &str,
    pk: &[String],
    dialect: Dialect,
) -> XResult<Box<dyn RowSink + 'a>> {
    let sink: Box<dyn RowSink + 'a> = match format {
        Format::Csv => Box::new(CsvSink { out }),
        Format::Json => Box::new(JsonSink {
            out,
            columns: Vec::new(),
            first: true,
        }),
        Format::SqlInsert => Box::new(SqlInsertSink {
            out,
            dialect,
            table: dialect.quote(table),
            columns: Vec::new(),
        }),
        Format::SqlUpdate => Box::new(SqlUpdateSink {
            out,
            dialect,
            table: dialect.quote(table),
            columns: Vec::new(),
            pk: pk.to_vec(),
            pk_index: Vec::new(),
        }),
        Format::SqlDelete => Box::new(SqlDeleteSink {
            out,
            dialect,
            table: dialect.quote(table),
            pk: pk.to_vec(),
            pk_index: Vec::new(),
        }),
        Format::Xlsx | Format::Ddl => {
            return Err(XError::internal("内部错误：该格式不是文本写入器"));
        }
    };
    Ok(sink)
}

fn make_xlsx_sink<'a, W: Write + std::io::Seek + Send + 'a>(
    out: W,
    sheet: &str,
) -> Box<dyn RowSink + 'a> {
    Box::new(XlsxSink {
        book: Some(Workbook::new(out)),
        name: sheet.to_string(),
    })
}

/// 主键列 → 结果集里的下标。找不到就报错：宁可不导，也不要生成一条
/// `where 1=1` 式的语句 —— UPDATE/DELETE 少了 WHERE 是要出人命的。
fn resolve_pk(columns: &[String], pk: &[String]) -> Result<Vec<(usize, String)>, String> {
    let mut resolved = Vec::new();
    for name in pk {
        let index = columns
            .iter()
            .position(|column| column.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("导出结果里没有主键列 {name}，无法生成 UPDATE/DELETE"))?;
        resolved.push((index, name.clone()));
    }
    if resolved.is_empty() {
        return Err("缺少主键列，无法生成 UPDATE/DELETE".to_string());
    }
    Ok(resolved)
}

fn where_clause(cells: &[CellValue], pk_index: &[(usize, String)], dialect: Dialect) -> String {
    pk_index
        .iter()
        .map(|(index, name)| {
            let value = cells
                .get(*index)
                .map(|cell| sql_literal(cell, dialect))
                .unwrap_or_else(|| "NULL".to_string());
            format!("{} = {value}", dialect.quote(name))
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

fn csv_value(cell: &CellValue) -> String {
    match cell {
        CellValue::Null => String::new(),
        CellValue::Text(text) => csv_field(text),
        CellValue::Blob { len } => csv_field(&format!("[blob {len} B]")),
        other => match shape::cell_to_value(other) {
            Value::String(text) => csv_field(&text),
            other => other.to_string(),
        },
    }
}

fn csv_field(value: &str) -> String {
    if value.contains(['"', ',', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn write_line<W: Write>(out: &mut W, fields: &[String]) -> Result<(), String> {
    out.write_all(fields.join(",").as_bytes()).map_err(io_error)?;
    out.write_all(b"\r\n").map_err(io_error)
}

/// 单格值 → SQL 字面量（导出与造数共用一份，免得「同一个值两处转义不一样」）。
pub(crate) fn sql_literal(cell: &CellValue, dialect: Dialect) -> String {
    match cell {
        CellValue::Null => "NULL".to_string(),
        CellValue::Integer(value) => value.to_string(),
        CellValue::Real(value) => {
            if value.fract() == 0.0 && value.abs() < 1e15 {
                format!("{}", *value as i64)
            } else {
                value.to_string()
            }
        }
        CellValue::Text(text) => dialect.literal(text),
        // 二进制不塞进 SQL 文本：几 MB 的 hex 会把文件撑爆，且各家字面量语法不一。
        // 留一句注释，别让「这一列怎么是空的」成为下一个问题。
        CellValue::Blob { len } => format!("/* blob {len} B 未导出 */ NULL"),
    }
}

// ------------------------------------------------------------------ SQL 拼装

/// 表名限定：schema 层级的类型（SQL Server / PostgreSQL）在树上把「库.模式」拼成一个字符串，
/// 而模式这一层必须体现在表名里，否则 SQL Server 会按**登录名的默认模式**去找表。
pub(crate) fn qualified(table: &str, database: &str, dialect: Dialect) -> String {
    let table = table.trim();
    if table.is_empty() || table.contains('.') {
        return dialect.quote(table);
    }
    if dialect.is_schema_aware() {
        if let Some((_, schema)) = database.split_once('.') {
            if !schema.is_empty() {
                return dialect.quote(&format!("{schema}.{table}"));
            }
        }
    }
    dialect.quote(table)
}

/// 由请求推导出要执行的查询。
fn base_sql(req: &ExportReq, dialect: Dialect) -> XResult<String> {
    let sql = req.sql.trim().trim_end_matches(';').trim();
    if !sql.is_empty() {
        return Ok(sql.to_string());
    }
    if req.table.trim().is_empty() {
        return Err(XError::bad_request("缺少 sql 或 table 参数"));
    }
    Ok(format!(
        "select * from {}",
        qualified(&req.table, &req.database, dialect)
    ))
}

/// 分页后的查询（见文件头决定 1）。
///
/// 默认**直接把分页子句追加到语句尾部**，而不是包一层
/// `select * from (…) dbmind_page`：派生表要求输出列名互不相同，而数据库允许
/// 重复输出列（`SELECT *, now(), now()` 这种没起别名的写法是合法 SQL）——
/// 一包就报 `Duplicate column name`，用户原句明明能跑（真机踩过：编辑器翻页报 1060）。
/// 追加在顶层是合法的（含 ORDER BY 的语句），也不改变任何输出列。
/// 只有语句自己已带顶层分页子句（`LIMIT` / `FETCH`）时才退回包子查询 ——
/// 直接追加会出现两个 LIMIT 的语法错误。
pub(crate) fn paging_sql(base: &str, offset: u64, size: u64, dialect: Dialect) -> String {
    let page = dialect.limit_clause(offset, size);
    if dialect.needs_order_by_for_paging() {
        // SQL Server：OFFSET/FETCH 必须紧跟 ORDER BY，而子查询里又不许 ORDER BY
        if has_order_by(base) {
            format!("{base} {page}")
        } else {
            format!("{base} order by (select null) {page}")
        }
    } else if has_top_level_paging_clause(base) {
        format!("select * from ({base}) dbmind_page {page}")
    } else {
        format!("{base} {page}")
    }
}

/// 语句的**顶层**（括号深度 0、字符串/标识符字面量之外）是否出现给定短语，
/// 词边界判定（`limits` 不算 `limit`）。与 [`strip_top_level_order_by`] 同一标准。
fn has_top_level_phrase(sql: &str, phrases: &[&str]) -> bool {
    let b = sql.as_bytes();
    let lb = sql.to_ascii_lowercase().into_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' { i += 2; continue; }
            if c == q { quote = None; }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => { quote = Some(c); i += 1; }
            b'(' => { depth += 1; i += 1; }
            b')' => { depth -= 1; i += 1; }
            _ => {
                let word_start = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
                if depth == 0 && word_start {
                    let after = |n: usize| {
                        let j = i + n;
                        j >= b.len() || !(b[j].is_ascii_alphanumeric() || b[j] == b'_')
                    };
                    for phrase in phrases {
                        let p = phrase.as_bytes();
                        if lb[i..].starts_with(p) && after(p.len()) {
                            return true;
                        }
                    }
                }
                i += 1;
            }
        }
    }
    false
}

/// 语句的**顶层**有没有自己的分页子句（`limit` / `fetch`）。
fn has_top_level_paging_clause(sql: &str) -> bool {
    has_top_level_phrase(sql, &["limit", "fetch"])
}

/// 顶层有没有 ORDER BY。只认**顶层**：子查询里的 ORDER BY 或字符串字面量里的
/// "order by" 都不算 —— SQL Server 分页靠这个决定要不要补 `order by (select null)`，
/// 误判会在语法层面炸掉。
fn has_order_by(sql: &str) -> bool {
    has_top_level_phrase(sql, &["order by"])
}

/// 剥掉**顶层**的 ORDER BY 子句（含其尾随的 LIMIT/OFFSET），供 count 包子查询用。
///
/// 为什么不再直接拒绝带 ORDER BY 的语句：用户最常写的就是 `SELECT … ORDER BY …`，
/// 一拒绝他们永远拿不到真实总数。MySQL/PG/Oracle 的派生表都允许 ORDER BY
/// （SQL Server 不允许 —— 所以先剥掉再数，而不是数一个注定报错的语句）。
///
/// 用括号深度扫描找**第一个顶层** `order by`（避开子查询与字符串字面量里的），
/// 从那里截断到结尾 —— 尾随的 LIMIT/OFFSET 属于分页语义，count 要的是全量。
/// 顶层带 UNION 的语句剥不了（ORDER BY 作用于整个 union），返回 None ——
/// 调用方（`count_rows`）会跳过派生表/CTE 改走二分探测，照样能数出精确值。
fn strip_top_level_order_by(base: &str) -> Option<String> {
    let b = base.as_bytes();
    let lb = base.to_ascii_lowercase().into_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut cut: Option<usize> = None;
    let mut has_top_union = false;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' { i += 2; continue; }
            if c == q { quote = None; }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => { quote = Some(c); i += 1; }
            b'(' => { depth += 1; i += 1; }
            b')' => { depth -= 1; i += 1; }
            _ => {
                let word_start = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
                if depth == 0 && word_start {
                    let after = |n: usize| {
                        let j = i + n;
                        j >= b.len() || !(b[j].is_ascii_alphanumeric() || b[j] == b'_')
                    };
                    if lb[i..].starts_with(b"order by") && after(8) {
                        if cut.is_none() { cut = Some(i); }
                    } else if lb[i..].starts_with(b"union") && after(5) {
                        has_top_union = true;
                    }
                }
                i += 1;
            }
        }
    }
    if has_top_union {
        return None;
    }
    match cut {
        Some(pos) => Some(base[..pos].trim_end().to_string()),
        None => Some(base.trim_end().to_string()),
    }
}

/// 提取取数语句里「顶层 `from` 起的尾部」（from joins / where … 直到语句结尾）。
///
/// 给 [`count_rows`] 的②层用：派生表在「重名列」上必死（`select *` 关联两张有同名列表
/// 就报 `Duplicate column name`），而这类查询绝大多数是
/// `select 列表 from joins [where …] [limit …]` 的形状 —— 把 from 起的片段整体借过来
/// 直接 `select count(*) from <片段>`，没有派生表，重名列不碍事；
/// 对无 GROUP BY / DISTINCT 的取数语句，数出来的就是**关联后的总行数**，与包壳等价。
///
/// 扫描与 [`strip_top_level_order_by`] 同一套纪律：引号感知（字符串字面量里的 from
/// 不算）、括号感知（子查询里的 from 深度 > 0 不算）、字边界（`information` 里的
/// `from` 片段不算）。遇到下面这些顶层结构返回 None —— `count(*)` 的语义会变，不硬数：
/// - `group by` / `having`：count 到的是分组数，不是行数；
/// - `distinct`（紧跟 select）：去重后的行数 ≠ 关联后的行数。
fn extract_countable_from(base: &str) -> Option<String> {
    let b = base.as_bytes();
    let lb = base.to_ascii_lowercase().into_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut from_pos: Option<usize> = None;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => {
                quote = Some(c);
                i += 1;
            }
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            _ => {
                let word_start =
                    i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
                let after = |n: usize| {
                    let j = i + n;
                    j >= b.len() || !(b[j].is_ascii_alphanumeric() || b[j] == b'_')
                };
                if depth == 0 && word_start {
                    if lb[i..].starts_with(b"distinct") && after(8) {
                        return None;
                    }
                    if lb[i..].starts_with(b"group") && after(5) {
                        return None;
                    }
                    if lb[i..].starts_with(b"having") && after(6) {
                        return None;
                    }
                    if lb[i..].starts_with(b"from") && after(4) && from_pos.is_none() {
                        from_pos = Some(i);
                    }
                }
                i += 1;
            }
        }
    }
    let pos = from_pos?;
    // 跳过 `from` 关键字本身（4 字节），只返回其后的片段 —— 调用方拼的是
    // `select count(*) from {片段}`，这里多带一个 from 就是 `from from` 语法错误
    //（真机踩过：JOIN 计数因此全部 -1）。
    Some(base[pos + 4..].trim().to_string())
}

/// 总量（拿不到就给 None ⇒ 调用方填 -1）。四层兜底，走到哪层算到哪；
/// 每层算的都是**同一个东西**（结果集的总行数），差别只在「怎么包」——
/// 越靠前的层越通用，越靠后的层越是为了绕开特定的语法限制：
///
/// ① 标准路 —— 剥掉顶层 ORDER BY 后包派生表计数。重复输出列（`SELECT *, now(), now()`
/// 这种没起别名的写法、`select *` 关联两张有同名列的表）会让它报
/// `Duplicate column name`：派生表要求列名互不相同，而数据库允许输出重名列 ——
/// 这不是 SQL 有错，是**计数壳**不适配。
///
/// ② 裸 FROM 直数 —— [`extract_countable_from`] 把顶层 `from …` 尾部整体借过来，
/// `select count(*) from <from 尾部>`。**不过派生表**，重名列不碍事；对无
/// GROUP BY / DISTINCT 的取数语句与①语义完全等价（都是数行数），且只扫一遍，
/// 效率与直接 count 一致 —— `select *` 关联查询的总数基本都走这层。
///
/// ③ CTE 显式列名表兜底 —— `WITH dbmind_count(c0, c1, …) AS (原句) SELECT COUNT(*) …`，
/// 按位置给输出列改名，重名列就不碍事了（MySQL 8+ / PG / SQLite / H2 / SQL Server 都认）。
/// 顶层 UNION 剥不了 ORDER BY（①②不适用），直接用 CTE 包整个 union 计数，语义正确。
///
/// ④ 二分探测兜底 —— 上面三条路都走不通时（**MySQL 5.7 没有 CTE** + 重名列就是这种），
/// 用「第 offset 行存不存在」二分出精确总数：`原句 + 分页子句(offset, 1)`，分页子句
/// 直接追加、不过派生表，重名列不碍事。代价是约 2×log₂(N) 次执行，只在这个罕见角落才走。
///
/// `ncols`：结果集列数（调用方刚执行过的结果里就有）。传 0 表示未知 —— 先拿原句探一次
/// （`max_rows=1`，代价极小）再走 ③。
pub(crate) async fn count_rows(
    state: &AppState,
    id: &str,
    database: &str,
    base: &str,
    ncols: usize,
) -> Option<i64> {
    let stripped = strip_top_level_order_by(base);
    if let Some(b) = &stripped {
        let sql = format!("select count(*) as cnt from ({b}) dbmind_count");
        if let Some(v) = try_count(state, id, database, sql).await {
            return Some(v);
        }
        // ② 裸 FROM 直数：派生表被重名列挡死时的主出口（真机：location 视图与
        // 合同表都有 RNUM，①必报 Duplicate column name —— ②一次扫描直接数完）
        if let Some(from_part) = extract_countable_from(b) {
            let sql = format!("select count(*) from {from_part}");
            if let Some(v) = try_count(state, id, database, sql).await {
                return Some(v);
            }
        }
    }
    let mut n = ncols;
    if n == 0 {
        n = run_sql_in(state, id, database, base.to_string(), 1)
            .await
            .ok()?
            .columns
            .len();
    }
    if n > 0 {
        let cols: Vec<String> = (0..n).map(|i| format!("c{i}")).collect();
        let sql = format!(
            "with dbmind_count({}) as ({}) select count(*) as cnt from dbmind_count",
            cols.join(", "),
            stripped.as_deref().unwrap_or(base)
        );
        if let Some(v) = try_count(state, id, database, sql).await {
            return Some(v);
        }
    }
    count_by_probe(state, id, database, stripped.as_deref().unwrap_or(base)).await
}

/// 二分计数：探测「第 offset 行存不存在」，找出行的总数。
///
/// 探测用 [`paging_sql`]（offset, 1）—— 分页子句追加在语句尾部，不经过派生表，
/// 对重名列免疫。语义：probe(offset)=true ⇔ 总行数 > offset。指数阶段找上界
/// （最多到 2⁴⁰ 行，再大不奉陪），二分阶段收口到精确值。
async fn count_by_probe(
    state: &AppState,
    id: &str,
    database: &str,
    base: &str,
) -> Option<i64> {
    let dialect = {
        let record = crate::api::require_record(state, id).await.ok()?;
        crate::api::dialect::Dialect::new(record.kind())
    };
    let probe = |offset: u64| async move {
        let sql = paging_sql(base, offset, 1, dialect);
        let result = run_sql_in(state, id, database, sql, 1).await.ok()?;
        Some(result.row_count > 0)
    };
    // 指数上界：count ∈ [low, high]（low 已证实有行，high 尚未证实）
    let mut low: u64 = 0;
    let mut high: u64 = 1;
    while probe(high).await? {
        low = high + 1;
        high = high.saturating_mul(2);
        if high > 1 << 40 {
            return None;
        }
    }
    while low < high {
        let mid = low + (high - low) / 2;
        match probe(mid).await? {
            true => low = mid + 1,
            false => high = mid,
        }
    }
    Some(low as i64)
}

/// 跑一条计数语句并取第一行第一列的整数；任何失败都算「统计不出」。
async fn try_count(state: &AppState, id: &str, database: &str, sql: String) -> Option<i64> {
    let result = run_sql_in(state, id, database, sql, 1).await.ok()?;
    rows_of(&result)
        .first()
        .and_then(|row| row.values().next())
        .and_then(Value::as_i64)
}

/// 表的建表语句：优先问数据库自己（`show create table` / `sqlite_master` / H2 的 `script`），
/// 拿不到就按列元数据拼一个 —— 拼出来的 DDL 会少掉索引与约束，
/// 所以调用方必须把这件事写进 notices（`synthesized` 标记就是给它的）。
///
/// **实现已收敛到 `meta::build_table_ddl`**：原先这里另写一份，判据还更窄 ——
/// 只认含 `create table` 的值，于是 H2 的 `CREATE CACHED TABLE ...`（H2 2.x 的写法）
/// 一个都匹配不上，H2 表的导出直接报"没有取到建表语句"。同一件事两套判据，必然有一套是错的。
pub(crate) async fn ddl_of(
    state: &AppState,
    id: &str,
    table: &str,
    database: &str,
) -> XResult<(String, bool)> {
    crate::api::meta::build_table_ddl(state, id, database, table).await
}

// ------------------------------------------------------------------ 导出执行

/// 一次导出的结果概要。
pub struct Outcome {
    pub rows: u64,
    pub total: i64,
    pub notices: Vec<String>,
}

/// 导出「一个查询」（当前页 或 全部）。
async fn export_query(
    state: &AppState,
    id: &str,
    req: &ExportReq,
    path: &FsPath,
    task: Option<&std::sync::Arc<Task>>,
) -> XResult<Outcome> {
    let record = require_record(state, id).await?;
    let dialect = Dialect::new(record.kind());

    if req.format == Format::Ddl {
        if req.table.trim().is_empty() {
            return Err(XError::bad_request("导出 DDL 需要 table 参数"));
        }
        let (ddl, synthesized) = ddl_of(state, id, &req.table, &req.database).await?;
        write_text(path, &ddl)?;
        let notices = if synthesized {
            vec!["建表语句是按列元数据拼的：索引、外键与部分默认值表达式不在其中".to_string()]
        } else {
            Vec::new()
        };
        return Ok(Outcome {
            rows: 0,
            total: -1,
            notices,
        });
    }

    let base = base_sql(req, dialect)?;
    let page_size = req.size.unwrap_or(PAGE_ROWS).max(1);
    let start_offset = req
        .page
        .map(|page| page.saturating_sub(1))
        .unwrap_or(0)
        .saturating_mul(page_size);
    // 先亮一句「正在统计」，再去 count(*)。
    //
    // count(*) 在大表上、或宿主正忙着生成大 xlsx 时可能卡几十秒，而这整段时间界面是
    // **完全静默**的（total=-1、phase 为空 → 只显示兜底文案，看着就是"点了导出没反应"），
    // 更糟的是取消也无效（取消检查长在取数循环里，根本走不到那儿）。实测报上来的现象
    // 正是"界面没反应、取消也没反应"，所以这里必须先出声、并给一次取消机会。
    if let Some(task) = task {
        task.set_total(-1);
        task.set_phase("正在统计总行数…");
        task.check_canceled().map_err(XError::internal)?;
    }
    let total = if req.all {
        // 列数未知（还没取数）：传 0，count_rows 内部会先用 max_rows=1 探一次
        count_rows(state, id, &req.database, &base, 0)
            .await
            .unwrap_or(-1)
    } else {
        -1
    };
    if let Some(task) = task {
        task.set_total(total);
        task.set_phase(if total >= 0 {
            format!("准备导出 {total} 行")
        } else {
            "准备导出（总量未知，按页推进）".to_string()
        });
    }

    let file = File::create(path).map_err(|e| XError::internal(format!("无法创建导出文件：{e}")))?;
    let sink_name = if req.table.trim().is_empty() { "export" } else { &req.table };
    let mut sink: Box<dyn RowSink> = match req.format {
        Format::Xlsx => make_xlsx_sink(file, sink_name),
        other => make_text_sink(other, BufWriter::new(file), sink_name, &req.pk, dialect)?,
    };

    // 首页用小页：整页取数要 1 秒以上（宿主→内核的逐行管线 ≈0.23ms/行），第一屏就等一整页
    // 会让"开始导出"看起来没反应；前三页各给 1/5 页先把进度推起来，之后回到整页满速跑。
    let full_page = page_size;
    // 渐进小页只用于异步全量导出（为了让第一屏尽快有进度）。
    // 同步「导出当页」必须严格按请求的 size 取 —— 否则用户要 200 行、实际只给 20 行，
    // 文件看着正常但内容少一截，无从察觉。
    let mut page_size = if req.size.is_some() || !req.all {
        full_page
    } else {
        (full_page / 10).max(1)
    };
    let mut offset = start_offset;
    let mut written = 0u64;
    let mut page = 0u64;
    let mut next_report = REPORT_ROWS;
    let mut next_log = LOG_ROWS;
    // 耗时剖析：设 DBMIND_EXPORT_PROFILE=1 时把「取数」与「写盘」的净耗时打到 stderr。
    // 默认零开销（只读一次环境变量），平时不打印；要判断"慢在哪一段"时直接开它。
    // 做它的原因：页大小实验只能说明"每页固定开销已接近 0"，但分不清剩下的是
    // 宿主取数还是内核写盘 —— 这两者对应的优化方向完全不同。
    let profile = std::env::var_os("DBMIND_EXPORT_PROFILE").is_some();
    let mut fetch_ms = 0f64;
    let mut write_ms = 0f64;
    loop {
        if let Some(task) = task {
            task.check_canceled().map_err(XError::internal)?;
        }
        let sql = paging_sql(&base, offset, page_size, dialect);
        let t_fetch = std::time::Instant::now();
        let result = run_sql_in(state, id, &req.database, sql, page_size as usize).await?;
        if profile {
            fetch_ms += t_fetch.elapsed().as_secs_f64() * 1000.0;
        }
        let got = result.rows.len() as u64;
        if page == 0 {
            let columns: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
            sink.begin(&columns)?;
        }
        // 逐行写盘，并且**页内也上报**：以前只在整页写完后更新一次，界面上只能 5000 一跳。
        let t_write = std::time::Instant::now();
        for row in &result.rows {
            sink.row(row)?;
            written += 1;
            if let Some(task) = task {
                if written >= next_report {
                    next_report = written + REPORT_ROWS;
                    task.check_canceled().map_err(XError::internal)?;
                    task.set_done(written);
                    task.set_phase(format!("已导出 {written} 行"));
                }
                if written >= next_log {
                    next_log = written + LOG_ROWS;
                    task.log(format!("已导出 {written} 行"));
                }
            }
        }
        if profile {
            write_ms += t_write.elapsed().as_secs_f64() * 1000.0;
        }
        page += 1;
        if let Some(task) = task {
            task.set_done(written);
            task.set_phase(format!("已导出 {written} 行"));
        }
        // 不足一页 ⇒ 到底了；同步导出只要一页
        if got < page_size || !req.all {
            break;
        }
        // 前两页小步快跑，之后换成整页
        if page >= 2 {
            page_size = full_page;
        }
        // 偏移必须按【本页实际读到的行数】推进。
        //
        // 这里踩过一个大坑：渐进策略会把 page_size 从"小页"改成"整页"，如果仍然写
        // offset += page_size，那么改大之后那一次就相当于凭空多跳了一整页 —— 前两页
        // 各只读了 5000 行，第三页却从 offset=55000 开始，中间 45000 行被整段跳过，
        // 而导出的行数看起来"正常"（只是少），用户根本无从察觉。
        offset += got;
    }
    let t_finish = std::time::Instant::now();
    sink.finish().map_err(XError::internal)?;
    if profile {
        let busy = fetch_ms + write_ms;
        let finish_ms = t_finish.elapsed().as_secs_f64() * 1000.0;
        let rate = if busy > 0.0 { written as f64 / (busy / 1000.0) } else { 0.0 };
        let share = if busy + finish_ms > 0.0 { fetch_ms / (busy + finish_ms) * 100.0 } else { 0.0 };
        eprintln!(
            "[export-profile] 行={written} 页={page} 取数={fetch_ms:.0}ms 写盘={write_ms:.0}ms 收尾={finish_ms:.0}ms 取数占比={share:.0}% 净速={rate:.0} 行/秒"
        );
    }

    // 自检：统计与实际导出的行数对不上就如实写进日志（以前会静默少行，用户无从察觉）
    if let Some(task) = task {
        if total >= 0 && written != total as u64 {
            task.log(format!(
                "注意：统计为 {total} 行，实际导出 {written} 行（差异通常来自表在导出期间被改动）"
            ));
        }
    }

    Ok(Outcome {
        rows: written,
        total,
        notices: Vec::new(),
    })
}

/// 库里的表清单（走内核的 `DatabaseMetaData`，schema 层级由影子连接负责）。
async fn list_tables(state: &AppState, id: &str, database: &str) -> XResult<Vec<String>> {
    let target = crate::api::scope::resolve(state, id, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let engine = state.engine();
    let tables = blocking(move || engine.list_tables_fresh(&target)).await?;
    Ok(tables
        .into_iter()
        .filter(|table| table.kind == dbmind_core::TableKind::Table)
        .map(|table| table.name)
        .collect())
}

/// 整库转储。
async fn export_dump(
    state: &AppState,
    id: &str,
    req: &ExportReq,
    path: &FsPath,
    task: Option<&std::sync::Arc<Task>>,
) -> XResult<Outcome> {
    let record = require_record(state, id).await?;
    let dialect = Dialect::new(record.kind());
    let tables = if req.tables.is_empty() {
        list_tables(state, id, &req.database).await?
    } else {
        req.tables.clone()
    };
    let mut notices = Vec::new();

    // csv / json / xlsx 是「一个表一个文件」，包**里没有地方放 DDL 文本** ——
    // 视图与例程的定义自然也进不去。说清楚这一点，比让用户以为"导出了但找不到"强。
    // （.sql 单文件格式会导出它们，见下面那段循环。）
    if req.format.packed() {
        let mut missing: Vec<String> = Vec::new();
        for (count, name) in [
            (req.views.len(), "视图"),
            (req.procedures.len(), "存储过程"),
            (req.functions.len(), "函数"),
            (req.triggers.len(), "触发器"),
            (req.events.len(), "事件"),
        ] {
            if count > 0 {
                missing.push(format!("{count} 个{name}"));
            }
        }
        if !missing.is_empty() {
            notices.push(format!(
                "{} 的定义未随包导出：这种格式每个表一个文件，放不下 DDL；需要定义请选「SQL 脚本」格式",
                missing.join("、")
            ));
        }
    }

    if let Some(task) = task {
        task.set_total(tables.len() as i64);
        task.set_phase(format!("共 {} 张表", tables.len()));
        for notice in &notices {
            task.log(notice.clone());
        }
    }

    let mut written = 0u64;
    if req.format.packed() {
        // csv / json / excel ⇒ 一个表一个文件，打包成 zip
        let file = File::create(path).map_err(|e| XError::internal(format!("无法创建导出包：{e}")))?;
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (index, table) in tables.iter().enumerate() {
            if let Some(task) = task {
                task.check_canceled().map_err(XError::internal)?;
                task.set_phase(format!("导出表 {}（{}/{}）", table, index + 1, tables.len()));
            }
            let name = format!("{}.{}", safe_file_name(table), req.format.ext());
            zip.start_file(&name, options)
                .map_err(|e| XError::internal(format!("写入导出包失败：{e}")))?;
            if req.format == Format::Xlsx {
                // xlsx 是随机访问容器（要先回填中央目录），先攒在内存再塞进包
                let mut cursor = Cursor::new(Vec::new());
                {
                    let mut sink = make_xlsx_sink(&mut cursor, table);
                    let rows = pump_rows(state, id, req, table, dialect, &mut sink, task, &mut notices).await?;
                    written += rows;
                    // 必须收尾：xlsx 的容器部件（[Content_Types].xml / workbook / 关系 / 样式）
                    // 是在 finish() 里补写的。漏掉它，包内就只剩一个孤零零的 worksheet 流，
                    // Excel 打开会说文件损坏 —— 普通导出那条路有调，整库转储这条一直漏着。
                    sink.finish().map_err(XError::internal)?;
                }
                zip.write_all(&cursor.into_inner())
                    .map_err(|e| XError::internal(format!("写入导出包失败：{e}")))?;
            } else {
                let mut sink = make_text_sink(req.format, &mut zip, table, &req.pk, dialect)?;
                let rows = pump_rows(state, id, req, table, dialect, &mut sink, task, &mut notices).await?;
                written += rows;
            }
            if let Some(task) = task {
                task.set_done(index as u64 + 1);
            }
        }
        zip.finish().map_err(|e| XError::internal(format!("打包失败：{e}")))?;
    } else {
        // sql / ddl ⇒ 单一 .sql 文件：DDL（可选 DROP）+ 数据
        let file = File::create(path).map_err(|e| XError::internal(format!("无法创建导出文件：{e}")))?;
        let mut out = BufWriter::new(file);
        writeln!(
            out,
            "-- DBMind 整库转储\n-- 连接 {} ｜ 库 {} ｜ 时间 {}\n",
            record.config.name,
            if req.database.is_empty() { "(连接默认)" } else { &req.database },
            tasks::stamp()
        )
        .map_err(io_error)?;

        // 视图 / 例程 / 触发器 / 事件：定义都从 `object_source` 取 ——
        // 与右键「查看定义」是同一条路，那边显示得出来，这边就该导得出去。
        // （早前这里只导出视图，其余类型在开头写一句"N 个存储过程未导出"就完了：
        //  用户既不知道是哪一个，也不知道为什么。现在按对象逐个说明。）
        for (kind, label, names) in [
            ("view", "视图", &req.views),
            ("procedure", "存储过程", &req.procedures),
            ("function", "函数", &req.functions),
            ("trigger", "触发器", &req.triggers),
            ("event", "事件", &req.events),
        ] {
            for name in names {
                match dialect.object_source(kind, name).sql() {
                    Some(sql) => {
                        let result = run_sql_in(state, id, &req.database, sql, 5).await?;
                        let mut found = false;
                        for row in rows_of(&result) {
                            for value in row.values() {
                                if let Some(text) = value.as_str() {
                                    // 行首必须是 create —— 否则 `show create procedure` 的
                                    // sql_mode 列（含 NO_AUTO_CREATE_USER）也会被写进脚本，
                                    // 导出的文件重新导入会报错（见 looks_like_ddl 的注释）
                                    if crate::api::meta::looks_like_ddl(text) {
                                        writeln!(out, "{};\n", text.trim_end_matches(';'))
                                            .map_err(io_error)?;
                                        found = true;
                                    }
                                }
                            }
                        }
                        if !found {
                            notices.push(format!("{label} {name} 的定义没取到，未导出"));
                        }
                    }
                    None => notices.push(format!("{label} {name} 的定义查询尚未实现，未导出")),
                }
            }
        }

        for (index, table) in tables.iter().enumerate() {
            if let Some(task) = task {
                task.check_canceled().map_err(XError::internal)?;
                task.set_phase(format!("导出表 {}（{}/{}）", table, index + 1, tables.len()));
            }
            let (ddl, synthesized) = ddl_of(state, id, table, &req.database).await?;
            if synthesized {
                let notice = format!("表 {table} 的建表语句是按列元数据拼的（可能少索引/外键）");
                if !notices.contains(&notice) {
                    notices.push(notice.clone());
                }
                if let Some(task) = task {
                    task.log(notice);
                }
            }
            if req.include_drop {
                writeln!(out, "drop table if exists {};", qualified(table, &req.database, dialect))
                    .map_err(io_error)?;
            }
            writeln!(out, "{};\n", ddl.trim().trim_end_matches(';')).map_err(io_error)?;

            if req.include_data && req.format != Format::Ddl {
                let mut sink = make_text_sink(Format::SqlInsert, &mut out, table, &req.pk, dialect)?;
                written += pump_rows(state, id, req, table, dialect, &mut sink, task, &mut notices).await?;
            }
            if let Some(task) = task {
                task.set_done(index as u64 + 1);
            }
        }
        out.flush().map_err(io_error)?;
    }

    Ok(Outcome {
        rows: written,
        total: tables.len() as i64,
        notices,
    })
}

/// 把一张表的数据灌进写入器（分页拉到取完为止）。
#[allow(clippy::too_many_arguments)]
async fn pump_rows<S>(
    state: &AppState,
    id: &str,
    req: &ExportReq,
    table: &str,
    dialect: Dialect,
    sink: &mut S,
    task: Option<&std::sync::Arc<Task>>,
    notices: &mut Vec<String>,
) -> XResult<u64>
where
    S: RowSink + ?Sized,
{
    let base = format!(
        "select * from {}",
        qualified(table, &req.database, dialect)
    );
    let page_size = req.size.unwrap_or(PAGE_ROWS).max(1);
    // 与 export_query 同策略：每张表前三页用小页，快速把进度推起来
    let full_page = page_size;
    let mut page_size = if req.size.is_some() { full_page } else { (full_page / 10).max(1) };
    let mut offset = 0u64;
    let mut written = 0u64;
    let mut pages = 0u64;
    let mut next_report = REPORT_ROWS;
    let mut next_log = LOG_ROWS;
    loop {
        if let Some(task) = task {
            task.check_canceled().map_err(XError::internal)?;
        }
        let sql = paging_sql(&base, offset, page_size, dialect);
        let result = match run_sql_in(state, id, &req.database, sql, page_size as usize).await {
            Ok(result) => result,
            Err(err) => {
                // 单张表失败不该让整库转储作废：记下来继续，最后统一汇报
                notices.push(format!("表 {table} 导出失败：{}", err.message));
                if let Some(task) = task {
                    task.log(format!("表 {table} 导出失败：{}", err.message));
                }
                return Ok(written);
            }
        };
        if offset == 0 {
            let columns: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
            sink.begin(&columns).map_err(XError::internal)?;
        }
        // 与 export_query 同一套做法：页内按行上报，数字连续往上滚、取消也立刻响应。
        // 这里**不设 done**：整库转储的 total/done 以「表」为单位（外层按表推进），
        // 行数只体现在 phase 文案里，混进来会把百分比算坏。
        for row in &result.rows {
            sink.row(row).map_err(XError::internal)?;
            written += 1;
            if let Some(task) = task {
                if written >= next_report {
                    next_report = written + REPORT_ROWS;
                    task.check_canceled().map_err(XError::internal)?;
                    task.set_phase(format!("表 {table}：已导出 {written} 行"));
                }
                if written >= next_log {
                    next_log = written + LOG_ROWS;
                    task.log(format!("表 {table}：已导出 {written} 行"));
                }
            }
        }
        let got = result.rows.len() as u64;
        if got < page_size {
            break;
        }
        pages += 1;
        if pages >= 2 {
            page_size = full_page;
        }
        // 偏移必须按【本页实际读到的行数】推进。
        //
        // 这里踩过一个大坑：渐进策略会把 page_size 从"小页"改成"整页"，如果仍然写
        // offset += page_size，那么改大之后那一次就相当于凭空多跳了一整页 —— 前两页
        // 各只读了 5000 行，第三页却从 offset=55000 开始，中间 45000 行被整段跳过，
        // 而导出的行数看起来"正常"（只是少），用户根本无从察觉。
        offset += got;
    }
    Ok(written)
}

fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    if cleaned.trim().is_empty() {
        "table".to_string()
    } else {
        cleaned
    }
}

fn write_text(path: &FsPath, text: &str) -> XResult<()> {
    std::fs::write(path, text).map_err(|e| XError::internal(format!("无法写入导出文件：{e}")))
}

/// 同步导入的临时文件（读完就删，不留垃圾在产物目录里）。
fn temp_path(ext: &str) -> XResult<PathBuf> {
    let dir = dbmind_core::paths::work_dir().join("tmp");
    dbmind_core::paths::ensure_dir(&dir)?;
    Ok(dir.join(format!("sync_{}.{}", tasks::stamp(), ext)))
}

fn file_response(bytes: Vec<u8>, media: &str, filename: &str) -> Response {
    (
        [
            (header::CONTENT_TYPE, media.to_string()),
            (header::CONTENT_DISPOSITION, tasks::content_disposition(filename)),
        ],
        bytes,
    )
        .into_response()
}

fn export_name(req: &ExportReq) -> String {
    let base = if !req.table.trim().is_empty() {
        safe_file_name(&req.table)
    } else {
        "export".to_string()
    };
    format!("{base}_{}.{}", tasks::stamp(), req.format.ext())
}

// ------------------------------------------------------------------ 处理器

/// `POST /api/{m}/export/{id}` —— 同步导出**当前一页**。
///
/// page/size 必须显式给（少一个就直接报错）：
/// 因为「同步」意味着「结果立刻回给你」，没有分页边界就等于把整库塞进一次响应。
pub async fn single(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Response> {
    let req = ExportReq::parse(&body, false)?;
    if req.format.needs_rows() && (req.page.unwrap_or(0) == 0 || req.size.unwrap_or(0) == 0) {
        return Err(XError::bad_request(
            "同步导出必须指定 page 与 size（都从 1 起）；要一次导出全部请用 /export/task/{id}",
        ));
    }
    req.validate()?;
    let path = temp_path(req.format.ext())?;
    let outcome = export_query(&state, &id, &req, &path, None).await;
    let bytes = std::fs::read(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    let outcome = outcome?;
    tracing::info!(rows = outcome.rows, "同步导出完成");
    Ok(file_response(bytes, req.format.media(), &export_name(&req)))
}

/// `POST /api/{m}/export/db/{id}` —— 同步整库转储。
pub async fn db(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Response> {
    let req = ExportReq::parse(&body, false)?;
    req.validate()?;
    let ext = if req.format.packed() { "zip" } else { req.format.ext() };
    let path = temp_path(ext)?;
    let outcome = export_dump(&state, &id, &req, &path, None).await;
    let bytes = std::fs::read(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    let outcome = outcome?;
    tracing::info!(rows = outcome.rows, "同步整库转储完成");
    let media = if req.format.packed() {
        "application/zip"
    } else {
        req.format.media()
    };
    let name = format!("dump_{}.{ext}", tasks::stamp());
    Ok(file_response(bytes, media, &name))
}

/// `POST /api/{m}/export/task/{id}` —— 异步导出全部。
pub async fn task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let req = ExportReq::parse(&body, true)?;
    req.validate()?;
    // 先探一次连接：连接不通就没必要建任务（否则用户看到的是「任务失败」而不是「连不上」）
    let record = require_record(&state, &id).await?;
    // 先把注册表拿出来再 spawn：`state.tasks.spawn(...)` 与「把 state 移进闭包」会同时借用/移动，
    // 编译器不允许
    let registry = state.tasks.clone();
    let task = registry.spawn("export", "exp_", move |task| {
        let state = state.clone();
        let id = id.clone();
        let req = req.clone();
        let label = record.config.name.clone();
        async move {
            let path = tasks::artifact_path(&task.id, req.format.ext())
                .map_err(|e| e)?;
            let outcome = export_query(&state, &id, &req, &path, Some(&task))
                .await
                .map_err(|err| err.message)?;
            let filename = export_name(&req);
            task.set_message(format!(
                "导出完成：{} 行（连接 {label}），已保存到 {}",
                outcome.rows,
                path.display()
            ));
            for notice in &outcome.notices {
                task.log(notice.clone());
            }
            task.set_artifact(Artifact {
                path,
                media_type: req.format.media().to_string(),
                filename,
            });
            Ok(Some(json!({
                "rows": outcome.rows,
                "total": outcome.total,
                "notices": outcome.notices,
            })))
        }
    });
    Ok(Json(json!({ "success": true, "taskId": task.id })))
}

/// `POST /api/{m}/export/dump-task/{id}` —— 异步整库转储。
pub async fn dump_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> XResult<Json<Value>> {
    let req = ExportReq::parse(&body, true)?;
    req.validate()?;
    let record = require_record(&state, &id).await?;
    let registry = state.tasks.clone();
    let task = registry.spawn("dump", "dump_", move |task| {
        let state = state.clone();
        let id = id.clone();
        let req = req.clone();
        let label = record.config.name.clone();
        async move {
            let ext = if req.format.packed() { "zip" } else { req.format.ext() };
            let path = tasks::artifact_path(&task.id, ext).map_err(|e| e)?;
            let outcome = export_dump(&state, &id, &req, &path, Some(&task))
                .await
                .map_err(|err| err.message)?;
            let media = if req.format.packed() {
                "application/zip".to_string()
            } else {
                req.format.media().to_string()
            };
            let filename = format!("dump_{}.{ext}", tasks::stamp());
            task.set_message(format!(
                "整库转储完成：{} 张表 / {} 行（连接 {label}），已保存到 {}",
                outcome.total.max(0),
                outcome.rows,
                path.display()
            ));
            for notice in &outcome.notices {
                task.log(notice.clone());
            }
            task.set_artifact(Artifact {
                path,
                media_type: media,
                filename,
            });
            Ok(Some(json!({
                "rows": outcome.rows,
                "tables": outcome.total,
                "notices": outcome.notices,
            })))
        }
    });
    Ok(Json(json!({ "success": true, "taskId": task.id })))
}

/// `GET /api/{m}/export/task/{taskId}` —— 进度。
///
/// 「任务不存在」按约定回 **HTTP 200 + `{success:false,status:"notfound"}`**：
/// 前端的轮询器就是按 `success` 分支的，回 404 会让它把「任务过期」当成「网络故障」。
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
    if let Some(artifact) = task.artifact() {
        // 路径也写在 message 里（兼容只认 message 的调用方）；这里额外给两个字段，界面能直接显示「去哪儿拿文件」
        view["fileName"] = json!(artifact.filename);
        view["filePath"] = json!(artifact.path.display().to_string());
    }
    if let Some(result) = task.result() {
        view["result"] = result;
    }
    Ok(Json(view))
}

/// `POST /api/{m}/export/cancel/{taskId}` —— 请求取消。
pub async fn cancel(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    match state.tasks.get(&task_id) {
        Some(task) => {
            task.cancel();
            Ok(Json(json!({ "success": true, "message": "已请求取消，任务会在当前批次结束后停止" })))
        }
        None => Ok(Json(json!({ "success": false, "message": "任务不存在或已过期" }))),
    }
}

/// `GET /api/{m}/export/download/{taskId}` —— 取回产物。
pub async fn download(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Response {
    let Some(task) = state.tasks.get(&task_id) else {
        return (StatusCode::NOT_FOUND, "任务不存在或已过期").into_response();
    };
    let Some(artifact) = task.artifact() else {
        return (StatusCode::NOT_FOUND, "任务尚未完成，没有可下载的文件").into_response();
    };
    match std::fs::read(&artifact.path) {
        Ok(bytes) => {
            // 取回即删：服务端这份只是中转，用户那份由他自己选位置另存。
            // 不删的话临时目录会随每次导出增长（而且没人会来看）。
            let _ = std::fs::remove_file(&artifact.path);
            file_response(bytes, &artifact.media_type, &artifact.filename)
        }
        Err(err) => {
            tracing::error!(path = %artifact.path.display(), error = %err, "导出产物读不到");
            (
                StatusCode::NOT_FOUND,
                format!("导出文件不存在：{}", artifact.path.display()),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod strip_order_by_tests {
    use super::strip_top_level_order_by;

    #[test]
    fn 剥掉顶层order_by() {
        // 常规：剥掉 ORDER BY
        assert_eq!(
            strip_top_level_order_by("SELECT a FROM t ORDER BY a").as_deref(),
            Some("SELECT a FROM t")
        );
        // 没有 ORDER BY：原样返回
        assert_eq!(
            strip_top_level_order_by("SELECT a FROM t").as_deref(),
            Some("SELECT a FROM t")
        );
        // 子查询里的 ORDER BY 属于内层，不动
        assert_eq!(
            strip_top_level_order_by("SELECT a FROM (SELECT b FROM t ORDER BY b) x ORDER BY a").as_deref(),
            Some("SELECT a FROM (SELECT b FROM t ORDER BY b) x")
        );
        // 字符串字面量里的 order by 不是子句
        assert_eq!(
            strip_top_level_order_by("SELECT a FROM t WHERE b = 'order by x'").as_deref(),
            Some("SELECT a FROM t WHERE b = 'order by x'")
        );
        // 顶层 UNION：不统计（截断会算错），交给调用方保持未知
        assert_eq!(
            strip_top_level_order_by("SELECT a FROM t UNION SELECT b FROM u ORDER BY a"),
            None
        );
    }
}
