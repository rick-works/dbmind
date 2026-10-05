//! `/api/datagen/*` —— 生成测试数据（造数）。
//!
//! ## 形状
//!
//! `POST /api/datagen/preview?connectionId=`（同步，10 行，**不写库**）
//! `POST /api/datagen/start?connectionId=`（异步任务，可轮询 / 可取消）
//! `GET  /api/datagen/task/{taskId}` / `POST /api/datagen/cancel/{taskId}`
//!
//! 前端把「字段 → 规则」的整张表发过来（`columns[]`），这里逐列解析成一份**计划**，
//! 再按行填值。这样做而不是「每行重新解析规则」，有两个好处：
//! 规则里的错误（未知规则名、列表为空、区间写反）在**开始生成之前**就能一次性报出来，
//! 而且每条规则只解析一次 —— 造 10 万行时这点开销是真金白银。
//!
//! ## 三个刻意的决定
//!
//! 1. **同一个 seed ⇒ 同一份数据**。随机数按 `(seed, 列序号, 行号)` 派生，
//!    不依赖调用顺序，所以「同样的 seed 再跑一次」在并行、分批下都能复现。
//! 2. **`skip` 是「不写这一列」**，不是「写空\": 自增列/有默认值的列要交给数据库自己填，
//!    写成 NULL 或 0 反而会破坏它们。
//! 3. **`sql` 规则是原样拼接的表达式**，所以必须把它写进 notices 说出来 ——
//!    用户输入的 `RAND()` 直接进 SQL 语句，这是**有意为之的能力**，
//!    但至少要让他知道「这一段没有被转义」。
//!
//! Excel 导入、AI 等域的 `notice` 也走同一套约定：不能悄悄降级。

use axum::extract::{Path, RawQuery, State};
use axum::Json;
use chrono::{Duration as ChronoDuration, Local, NaiveDate, NaiveDateTime};
use dbmind_core::{AccessContext, CellValue, ColumnMeta, QueryOptions, QueryRequest, QueryResult, StatementKind};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::dialect::Dialect;
use crate::api::error::{XError, XResult};
use crate::api::export::sql_literal;
use crate::api::{blocking, require_record, Params};
use crate::AppState;

/// 一批写多少行（一条 INSERT 里放多少个值元组）—— 默认值，不是上限；
/// 用户要多少给多少，只有**每条语句**的方言硬限制必须守（见 `MAX_TUPLES_PER_INSERT`）。
const DEFAULT_BATCH: u64 = 500;
/// 每条 INSERT 语句允许的**值元组**数上限 —— 这是数据库引擎的硬限制，拼多了直接报错：
/// SQL Server 一条多行 INSERT 最多 1000 行（错误 10738：行值表达式的数目超出了允许的最大值）。
/// 其它方言没有这个数，给 u64::MAX（实际仍受 batch 约束）。
fn max_tuples_per_insert(dialect: Dialect) -> u64 {
    if dialect.kind.key() == "sqlserver" { 1000 } else { u64::MAX }
}

// ------------------------------------------------------------------ 请求

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GenRequest {
    #[serde(default)]
    database: String,
    #[serde(default)]
    table: String,
    #[serde(default)]
    rows: Option<u64>,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    batch_size: Option<u64>,
    #[serde(default)]
    clear_before: bool,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    columns: Vec<ColumnRule>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct ColumnRule {
    name: String,
    #[serde(default)]
    rule: String,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    values: Vec<String>,
    #[serde(default)]
    min: Option<String>,
    #[serde(default)]
    max: Option<String>,
    #[serde(default)]
    length: Option<u64>,
    #[serde(default)]
    start: Option<i64>,
    #[serde(default)]
    null_ratio: Option<u32>,
}

impl ColumnRule {
    fn text(&self) -> String {
        self.value.clone().unwrap_or_default().trim().to_string()
    }

    fn null_ratio(&self) -> u32 {
        self.null_ratio.unwrap_or(0).min(100)
    }
}

// ------------------------------------------------------------------ 随机数

/// 可复现的伪随机（splitmix/xorshift 混合）。
///
/// 为什么不用 `rand`：这里要的不是「统计上完美的随机」，而是
/// **「同一个 seed 必然复现同一份数据」**。自己写 20 行，这个性质就是显然的；
/// 换成外部 crate 反而要额外确认它的版本行为是否稳定。
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next() % bound
        }
    }

    fn range(&mut self, low: i64, high: i64) -> i64 {
        if high <= low {
            return low;
        }
        low + self.below((high - low + 1) as u64) as i64
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        if items.is_empty() {
            return "";
        }
        items[self.below(items.len() as u64) as usize]
    }

    fn alpha_num(&mut self, length: usize) -> String {
        const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        (0..length)
            .map(|_| CHARS[self.below(CHARS.len() as u64) as usize] as char)
            .collect()
    }
}

/// 由 `(seed, 列, 行)` 派生该格子的随机源 —— 与生成顺序无关，故可复现。
fn cell_rng(seed: u64, column: usize, row: u64) -> Rng {
    let mut x = seed
        ^ (column as u64).wrapping_mul(0x9E3779B97F4A7C15)
        ^ row.wrapping_mul(0xBF58476D1CE4E5B9);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58476D1CE4E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D049BB133111EB);
    Rng::new(x ^ (x >> 31))
}

// ------------------------------------------------------------------ 值域池

const SURNAMES: &[&str] = &[
    "张", "王", "李", "赵", "刘", "陈", "杨", "黄", "周", "吴", "徐", "孙", "马", "朱", "胡",
    "郭", "何", "林", "罗", "高",
];
const GIVEN: &[&str] = &[
    "伟", "芳", "娜", "敏", "静", "强", "磊", "洋", "勇", "艳", "杰", "娟", "涛", "明", "超",
    "秀英", "霞", "平", "刚", "桂英", "文", "辉", "玲", "鹏", "华",
];
const CITIES: &[&str] = &[
    "北京", "上海", "广州", "深圳", "杭州", "南京", "成都", "重庆", "武汉", "西安", "苏州",
    "天津", "长沙", "郑州", "青岛", "宁波", "厦门", "福州", "合肥", "济南",
];
const DISTRICTS: &[&str] = &[
    "高新区", "经济技术开发区", "新区", "老城区", "工业园区", "滨江区", "海淀区", "朝阳区",
];
const ROADS: &[&str] = &[
    "中山路", "人民路", "解放路", "建设大道", "科技路", "长江路", "和平街", "文化路", "创业路",
];
const INDUSTRIES: &[&str] = &[
    "科技", "信息", "网络", "数据", "智能", "软件", "电子", "医药", "物流", "贸易", "环保",
    "新材", "新能源", "传媒", "教育",
];
const COMPANY_SUFFIX: &[&str] = &["有限公司", "股份有限公司", "集团有限公司", "科技有限公司"];
const WORDS: &[&str] = &[
    "高效", "稳定", "灵活", "可靠", "轻量", "智能", "安全", "敏捷", "清晰", "简洁", "完整",
    "流畅", "统一", "开放", "紧凑",
];
const NOUNS: &[&str] = &[
    "系统", "平台", "引擎", "方案", "服务", "模块", "框架", "工具", "中心", "网关", "数据集",
];
const TITLE_PREFIX: &[&str] = &["关于", "基于", "面向", "深入", "浅析", "重读", "重构"];
const DOMAINS: &[&str] = &["example.com", "test.com", "demo.cn", "corp.com", "mail.cn"];
const URL_PATHS: &[&str] = &["docs", "api/v1", "blog/post", "user/profile", "help", "pricing"];
const USERNAME_WORDS: &[&str] = &[
    "sky", "river", "stone", "cloud", "forest", "bright", "swift", "quiet", "green", "north",
    "ocean", "field", "light", "sharp", "happy",
];

// ------------------------------------------------------------------ 计划

/// 一列的最终生成方式（规则只解析一次）。
#[derive(Clone)]
enum PlanRule {
    /// 不写这一列（交给数据库的默认值/自增）
    Skip,
    Null,
    Fixed(CellValue),
    /// 原样进 SQL 的表达式（`sql` 规则）
    Raw(String),
    Gen(GenSpec),
}

#[derive(Clone)]
enum GenSpec {
    /// 没有合理内容的列（blob）→ 置空
    Nullish,
    Sequence(i64),
    RandomInt(i64, i64),
    RandomDecimal(f64, f64),
    Bool,
    RandomString(usize),
    List(Vec<String>),
    Cycle(Vec<String>),
    Code(String),
    Uuid,
    ChineseName,
    Username,
    Email,
    Phone,
    Company,
    City,
    Address,
    Word,
    Title,
    Ip,
    Url,
    DateRange(i64, i64),
    Now,
}

/// 按目标列类型归纳出的「像什么」——`auto` 规则靠它选生成方式。
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Int,
    Decimal,
    Date,
    Bool,
    Uuid,
    Blob,
    Text,
}

fn classify(type_name: Option<&str>) -> Kind {
    let name = type_name.unwrap_or("").to_ascii_lowercase();
    if name.contains("uuid") || name.contains("uniqueidentifier") {
        return Kind::Uuid;
    }
    if name.contains("blob") || name.contains("binary") || name.contains("bytea") {
        return Kind::Blob;
    }
    if name.contains("bool") || name == "bit" {
        return Kind::Bool;
    }
    if name.contains("date") || name.contains("time") {
        return Kind::Date;
    }
    if name.contains("int") || name.contains("serial") || name.contains("year") {
        return Kind::Int;
    }
    if name.contains("decimal")
        || name.contains("numeric")
        || name.contains("number")
        || name.contains("float")
        || name.contains("double")
        || name.contains("real")
    {
        return Kind::Decimal;
    }
    Kind::Text
}

#[derive(Clone)]
struct Planned {
    name: String,
    rule: PlanRule,
    null_ratio: u32,
    /// 是否原样进 SQL（表达式），不参与转义
    raw: bool,
}

/// 解析规则 → 计划（错误在这里一次性报出）。
fn plan(rules: &[ColumnRule], types: &[(String, Option<String>)]) -> XResult<(Vec<Planned>, Vec<String>)> {
    let mut notices = Vec::new();
    let mut planned = Vec::new();
    if rules.is_empty() {
        return Err(XError::bad_request("没有收到任何字段规则"));
    }
    for (index, rule) in rules.iter().enumerate() {
        let type_name = types
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&rule.name))
            .and_then(|(_, type_name)| type_name.clone());
        let kind = classify(type_name.as_deref());
        let raw_rule = rule.rule.trim().to_ascii_lowercase();
        let (plan_rule, is_raw) = match raw_rule.as_str() {
            "skip" => (PlanRule::Skip, false),
            "null" => (PlanRule::Null, false),
            "" | "auto" => (PlanRule::Gen(auto_spec(kind)), false),
            "sequence" => (PlanRule::Gen(GenSpec::Sequence(rule.start.unwrap_or(1))), false),
            "random_int" => {
                let min = rule.min.as_deref().unwrap_or("").trim().parse().unwrap_or(1);
                let max = rule
                    .max
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .parse()
                    .unwrap_or(10_000);
                // 区间写反了就换过来，而不是生成一堆空的/越界的值
                (PlanRule::Gen(GenSpec::RandomInt(min.min(max), max.max(min))), false)
            }
            "random_decimal" => {
                let min: f64 = rule.min.as_deref().unwrap_or("").trim().parse().unwrap_or(0.0);
                let max: f64 = rule.max.as_deref().unwrap_or("").trim().parse().unwrap_or(1000.0);
                (PlanRule::Gen(GenSpec::RandomDecimal(min.min(max), max.max(min))), false)
            }
            "bool" => (PlanRule::Gen(GenSpec::Bool), false),
            "random_string" => (
                PlanRule::Gen(GenSpec::RandomString(rule.length.unwrap_or(8).clamp(1, 255) as usize)),
                false,
            ),
            "fixed" => (
                PlanRule::Fixed(coerce(&rule.text(), kind)),
                false,
            ),
            "list" | "cycle" => {
                let values: Vec<String> = rule
                    .values
                    .iter()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .collect();
                if values.is_empty() {
                    notices.push(format!("字段 {} 选了「列表」但没填可选值，已按自动规则生成", rule.name));
                    (PlanRule::Gen(auto_spec(kind)), false)
                } else if raw_rule == "list" {
                    (PlanRule::Gen(GenSpec::List(values)), false)
                } else {
                    (PlanRule::Gen(GenSpec::Cycle(values)), false)
                }
            }
            "code" => {
                let prefix = if rule.text().is_empty() {
                    "CODE-".to_string()
                } else {
                    rule.text()
                };
                (PlanRule::Gen(GenSpec::Code(prefix)), false)
            }
            "uuid" => (PlanRule::Gen(GenSpec::Uuid), false),
            "chinese_name" => (PlanRule::Gen(GenSpec::ChineseName), false),
            "username" => (PlanRule::Gen(GenSpec::Username), false),
            "email" => (PlanRule::Gen(GenSpec::Email), false),
            "phone" => (PlanRule::Gen(GenSpec::Phone), false),
            "company" => (PlanRule::Gen(GenSpec::Company), false),
            "city" => (PlanRule::Gen(GenSpec::City), false),
            "address" => (PlanRule::Gen(GenSpec::Address), false),
            "word" => (PlanRule::Gen(GenSpec::Word), false),
            "title" => (PlanRule::Gen(GenSpec::Title), false),
            "ip" => (PlanRule::Gen(GenSpec::Ip), false),
            "url" => (PlanRule::Gen(GenSpec::Url), false),
            "random_date" => {
                let now = Local::now().naive_local();
                let from = rule
                    .min
                    .as_deref()
                    .and_then(parse_date)
                    .unwrap_or_else(|| now - ChronoDuration::days(365));
                let to = rule.max.as_deref().and_then(parse_date).unwrap_or(now);
                let (from, to) = if to < from { (to, from) } else { (from, to) };
                (
                    PlanRule::Gen(GenSpec::DateRange(local_stamp(from), local_stamp(to))),
                    false,
                )
            }
            "now" => (PlanRule::Gen(GenSpec::Now), false),
            "sql" => {
                let expression = rule.text();
                if expression.is_empty() {
                    return Err(XError::bad_request(format!(
                        "字段 {} 选了「SQL 表达式」但表达式是空的",
                        rule.name
                    )));
                }
                notices.push(format!(
                    "字段 {} 使用 SQL 表达式 {expression}（原样进语句，不做转义）",
                    rule.name
                ));
                (PlanRule::Raw(expression), true)
            }
            other => {
                notices.push(format!("字段 {} 的规则「{other}」未识别，已按自动规则生成", rule.name));
                (PlanRule::Gen(auto_spec(kind)), false)
            }
        };
        planned.push(Planned {
            name: rule.name.clone(),
            rule: plan_rule,
            null_ratio: rule.null_ratio(),
            raw: is_raw,
        });
        let _ = index;
    }
    if planned.iter().all(|column| matches!(column.rule, PlanRule::Skip)) {
        return Err(XError::bad_request("所有字段都被设为「不写入」，没有可生成的内容"));
    }
    Ok((planned, notices))
}

fn auto_spec(kind: Kind) -> GenSpec {
    match kind {
        Kind::Int => GenSpec::RandomInt(1, 100_000),
        Kind::Decimal => GenSpec::RandomDecimal(0.0, 10_000.0),
        Kind::Date => GenSpec::DateRange(
            local_stamp(Local::now().naive_local() - ChronoDuration::days(365)),
            local_stamp(Local::now().naive_local()),
        ),
        Kind::Bool => GenSpec::Bool,
        Kind::Uuid => GenSpec::Uuid,
        // blob 没有「合理的随机内容」：置空并让调用方在 notices 里看到
        Kind::Blob => GenSpec::Nullish,
        Kind::Text => GenSpec::RandomString(8),
    }
}

/// 「本地时间的朴素值」→ 时间戳。
///
/// 用 `DateTime::<Local>::from` 而不是 `.and_utc()`：用户填的是「本地日期」，
/// 格式化输出时也按本地时区解释，两边一致才不会出现「填 2020-01-01、出来 2019-12-31」。
fn local_stamp(value: NaiveDateTime) -> i64 {
    use chrono::TimeZone;
    match Local.from_local_datetime(&value).earliest() {
        Some(moment) => moment.timestamp(),
        // 本地时区里不存在的时刻（夏令时跳变那一小时）：退回按 UTC 解释，总比 panic 好
        None => value.and_utc().timestamp(),
    }
}

fn parse_date(text: &str) -> Option<NaiveDateTime> {
    let text = text.trim();
    if let Ok(value) = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S") {
        return Some(value);
    }
    if let Ok(value) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return value.and_hms_opt(0, 0, 0);
    }
    None
}

/// 固定值按目标类型落到合适的 CellValue（省得让数据库去做隐式转换）。
fn coerce(text: &str, kind: Kind) -> CellValue {
    match kind {
        Kind::Int => text.parse::<i64>().map(CellValue::Integer).unwrap_or_else(|_| CellValue::Text(text.to_string())),
        Kind::Decimal => text.parse::<f64>().map(CellValue::Real).unwrap_or_else(|_| CellValue::Text(text.to_string())),
        _ => CellValue::Text(text.to_string()),
    }
}

/// 生成一格的取值。
fn value_of(column: &Planned, row: u64, column_index: usize, seed: u64) -> Option<CellValue> {
    // 空值比例先判：它是对整列的统一约束，优先于具体规则
    if column.null_ratio > 0 {
        let mut rng = cell_rng(seed ^ 0xA5A5_5A5A, column_index, row);
        if rng.below(100) < column.null_ratio as u64 {
            return Some(CellValue::Null);
        }
    }
    match &column.rule {
        PlanRule::Skip => None,
        PlanRule::Null => Some(CellValue::Null),
        PlanRule::Fixed(value) => Some(value.clone()),
        // Raw 在拼语句时单独处理，这里给个占位（不会用到）
        PlanRule::Raw(_) => Some(CellValue::Null),
        PlanRule::Gen(spec) => Some(gen_value(spec, row, column_index, seed)),
    }
}

fn gen_value(spec: &GenSpec, row: u64, column: usize, seed: u64) -> CellValue {
    let mut rng = cell_rng(seed, column, row);
    match spec {
        GenSpec::Nullish => CellValue::Null,
        GenSpec::Sequence(start) => CellValue::Integer(start + row as i64),
        GenSpec::RandomInt(min, max) => CellValue::Integer(rng.range(*min, *max)),
        GenSpec::RandomDecimal(min, max) => {
            let span = (max - min).max(0.0);
            let value = min + (rng.below(1_000_000) as f64 / 1_000_000.0) * span;
            CellValue::Real((value * 100.0).round() / 100.0)
        }
        GenSpec::Bool => CellValue::Integer(rng.below(2) as i64),
        GenSpec::RandomString(length) => CellValue::Text(rng.alpha_num(*length)),
        GenSpec::List(values) => CellValue::Text(rng.pick(
            &values.iter().map(String::as_str).collect::<Vec<_>>(),
        ).to_string()),
        GenSpec::Cycle(values) => CellValue::Text(values[(row as usize) % values.len()].clone()),
        GenSpec::Code(prefix) => CellValue::Text(format!("{prefix}{:05}", row + 1)),
        GenSpec::Uuid => {
            let hex = "0123456789abcdef";
            let mut raw = String::new();
            for index in 0..32 {
                if matches!(index, 8 | 12 | 16 | 20) {
                    raw.push('-');
                }
                let mut pick = rng.below(16) as usize;
                if index == 12 {
                    pick = 4; // 版本位
                }
                raw.push(hex.as_bytes()[pick] as char);
            }
            CellValue::Text(raw)
        }
        GenSpec::ChineseName => CellValue::Text(format!("{}{}", rng.pick(SURNAMES), rng.pick(GIVEN))),
        GenSpec::Username => CellValue::Text(format!(
            "{}{:04}",
            rng.pick(USERNAME_WORDS),
            rng.below(10_000)
        )),
        GenSpec::Email => CellValue::Text(format!(
            "{}{}@{}",
            rng.pick(USERNAME_WORDS),
            rng.below(1000),
            rng.pick(DOMAINS)
        )),
        GenSpec::Phone => {
            let prefix = ["13", "15", "17", "18", "19"];
            CellValue::Text(format!(
                "{}{:08}",
                rng.pick(&prefix),
                rng.below(100_000_000)
            ))
        }
        GenSpec::Company => CellValue::Text(format!(
            "{}{}{}",
            rng.pick(CITIES),
            rng.pick(INDUSTRIES),
            rng.pick(COMPANY_SUFFIX)
        )),
        GenSpec::City => CellValue::Text(rng.pick(CITIES).to_string()),
        GenSpec::Address => CellValue::Text(format!(
            "{}{}{}{}号",
            rng.pick(CITIES),
            rng.pick(DISTRICTS),
            rng.pick(ROADS),
            rng.range(1, 999)
        )),
        GenSpec::Word => CellValue::Text(format!("{}{}", rng.pick(WORDS), rng.pick(NOUNS))),
        GenSpec::Title => CellValue::Text(format!(
            "{}{}{}的实践",
            rng.pick(TITLE_PREFIX),
            rng.pick(WORDS),
            rng.pick(NOUNS)
        )),
        GenSpec::Ip => {
            // 只取公网常见的段，避开 10/127/192.168/172.16-31 这些一看就是内网的
            let first = [58u64, 60, 101, 106, 114, 119, 121, 122, 175, 180, 182, 203, 210, 218, 221];
            CellValue::Text(format!(
                "{}.{}.{}.{}",
                first[rng.below(first.len() as u64) as usize],
                rng.below(256),
                rng.below(256),
                rng.range(1, 254)
            ))
        }
        GenSpec::Url => CellValue::Text(format!(
            "https://{}/{}/{}",
            rng.pick(DOMAINS),
            rng.pick(URL_PATHS),
            rng.below(1000)
        )),
        GenSpec::DateRange(from, to) => {
            let stamp = rng.range(*from, *to);
            CellValue::Text(format_timestamp(stamp))
        }
        GenSpec::Now => CellValue::Text(
            Local::now()
                .naive_local()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        ),
    }
}

fn format_timestamp(stamp: i64) -> String {
    use chrono::TimeZone;
    match Local.timestamp_opt(stamp, 0).single() {
        Some(value) => value.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => String::new(),
    }
}

/// 拼一行的值元组（`skip` 的列不出现）。
fn build_row(planned: &[Planned], row: u64, seed: u64, dialect: Dialect) -> String {
    let mut values = Vec::new();
    for (index, column) in planned.iter().enumerate() {
        if matches!(column.rule, PlanRule::Skip) {
            continue;
        }
        if column.raw {
            if let PlanRule::Raw(expression) = &column.rule {
                values.push(expression.clone());
                continue;
            }
        }
        let value = value_of(column, row, index, seed).unwrap_or(CellValue::Null);
        values.push(sql_literal(&value, dialect));
    }
    format!("({})", values.join(", "))
}

/// 参与写入的列名。
fn written_columns(planned: &[Planned], dialect: Dialect) -> Vec<String> {
    planned
        .iter()
        .filter(|column| !matches!(column.rule, PlanRule::Skip))
        .map(|column| dialect.quote(&column.name))
        .collect()
}

/// 多行插入是否安全（与导入同一套判断）。
fn supports_multi_row(dialect: Dialect) -> bool {
    matches!(
        dialect.kind.key(),
        "mysql" | "mariadb" | "doris" | "postgresql" | "kingbase" | "sqlite" | "sqlserver"
            | "h2" | "clickhouse"
    )
}

fn build_insert(
    table: &str,
    columns: &[String],
    planned: &[Planned],
    from: u64,
    count: u64,
    seed: u64,
    dialect: Dialect,
) -> String {
    let tuples: Vec<String> = (from..from + count)
        .map(|row| build_row(planned, row, seed, dialect))
        .collect();
    format!(
        "insert into {table} ({}) values {}",
        columns.join(", "),
        tuples.join(", ")
    )
}

// ------------------------------------------------------------------ 处理器

/// 取目标表的列（类型信息决定 `auto` 规则怎么生成）。
async fn table_columns(
    state: &AppState,
    conn: &str,
    database: &str,
    table: &str,
) -> XResult<Vec<(String, Option<String>)>> {
    if table.trim().is_empty() {
        return Err(XError::bad_request("缺少 table 参数"));
    }
    let target = crate::api::scope::resolve(state, conn, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    let engine = state.engine();
    let table_name = table.to_string();
    let columns = blocking(move || engine.list_columns_fresh(&target, &table_name)).await?;
    if columns.is_empty() {
        return Err(XError::bad_request(format!("表 {table} 不存在或没有列信息")));
    }
    Ok(columns
        .into_iter()
        .map(|column| (column.name, column.type_name))
        .collect())
}

async fn run_sql(state: &AppState, conn: &str, database: &str, sql: &str) -> Result<(), XError> {
    let target = crate::api::scope::resolve(state, conn, database).await?;
    let engine = state.engine();
    let request = QueryRequest {
        read_only: None,
        connection: target,
        sql: sql.to_string(),
        options: QueryOptions {
            max_rows: 1,
            timeout_ms: 600_000,
        },
        execution_id: None,
        session: Some("internal:browse".to_string()),
        // 生成测试数据是界面功能驱动的写入（不是用户在编辑器里敲的 SQL）：不进查询历史
        internal: true,
    };
    blocking(move || engine.execute(request, AccessContext::Web)).await?;
    Ok(())
}

/// `POST /api/datagen/preview?connectionId=` —— 预览（不写库）。
pub async fn preview(
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
    Json(body): Json<GenRequest>,
) -> XResult<Json<Value>> {
    let conn = Params::parse(raw.as_deref())
        .get("connectionId")
        .ok_or_else(|| XError::bad_request("缺少 connectionId 参数"))?;
    require_record(&state, &conn).await?;
    let rows = body.rows.unwrap_or(10).clamp(1, 100);
    let types = table_columns(&state, &conn, &body.database, &body.table).await?;
    let (planned, mut notices) = plan(&body.columns, &types)?;
    let seed = body.seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(42)
    });
    if planned.iter().any(|column| matches!(column.rule, PlanRule::Gen(GenSpec::Nullish))) {
        notices.push("有 blob/二进制字段被置空：随机二进制没有「合理内容」，硬造只会污染数据".to_string());
    }

    let names: Vec<String> = planned.iter().map(|column| column.name.clone()).collect();
    let type_names: Vec<Option<String>> = planned
        .iter()
        .map(|column| {
            types
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&column.name))
                .and_then(|(_, type_name)| type_name.clone())
        })
        .collect();
    let data: Vec<Vec<CellValue>> = (0..rows)
        .map(|row| {
            planned
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    value_of(column, row, index, seed).unwrap_or(CellValue::Null)
                })
                .collect()
        })
        .collect();

    // 预览复用内核结果形状：前端就是按 QueryResult 渲染的（列名数组 + 行对象数组）
    let result = QueryResult {
        execution_id: "datagen-preview".to_string(),
        connection_id: conn.clone(),
        connection_name: String::new(),
        statement_kind: StatementKind::Read,
        columns: names
            .iter()
            .zip(type_names.iter())
            .map(|(name, type_name)| ColumnMeta {
                name: name.clone(),
                type_name: type_name.clone(),
            })
            .collect(),
        row_count: data.len(),
        rows: data,
        affected_rows: None,
        truncated: false,
        duration_ms: 0,
        notices: notices.clone(),
        session_id: None,
        source_object: None,
        row_identity: None,
    };
    let mut payload = crate::api::shape::query_result_json(&result);
    if let Some(object) = payload.as_object_mut() {
        object.insert("notices".to_string(), json!(notices));
    }
    Ok(Json(payload))
}

/// `POST /api/datagen/start?connectionId=` —— 开始写入（异步任务）。
pub async fn start(
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
    Json(body): Json<GenRequest>,
) -> XResult<Json<Value>> {
    let conn = Params::parse(raw.as_deref())
        .get("connectionId")
        .ok_or_else(|| XError::bad_request("缺少 connectionId 参数"))?;
    // `mode` 是上游协议里的字段（preview / insert）。这里既然只负责写入，
    // 就把「用错接口」这件事明确顶回去 —— 否则调用方会以为自己在预览，实际在写库。
    match body.mode.trim() {
        "" | "insert" | "append" => {}
        "preview" => {
            return Err(XError::bad_request(
                "start 接口不接受 mode=preview：预览请调用 /api/datagen/preview（它不写库）",
            ))
        }
        other => return Err(XError::bad_request(format!("不支持的 mode：{other}"))),
    }
    let record = require_record(&state, &conn).await?;
    // 行数**不设上限**（用户明确要求）：造多少行是业务决定，进度/取消机制本身就能兜住大任务；
    let rows = body.rows.unwrap_or(100).max(1);
    let types = table_columns(&state, &conn, &body.database, &body.table).await?;
    let (planned, notices) = plan(&body.columns, &types)?;
    let batch = body.batch_size.unwrap_or(DEFAULT_BATCH).max(1);
    let seed = body.seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(42)
    });

    let database = body.database.clone();
    let table = body.table.clone();
    let clear_before = body.clear_before;
    let registry = state.tasks.clone();
    // notices 既要进任务（写日志/结果），也要立刻回给调用方（界面在启动后就能提示），
    // 所以给任务一份克隆
    let task_notices = notices.clone();
    let task = registry.spawn("datagen", "datagen_", move |task| {
        let state = state.clone();
        let conn = conn.clone();
        let database = database.clone();
        let table = table.clone();
        let planned = planned.clone();
        let notices = task_notices.clone();
        let label = record.config.name.clone();
        async move {
            task.set_context(json!({ "database": database, "table": table, "rows": rows }));
            task.set_total(rows as i64);
            for notice in &notices {
                task.log(notice.clone());
            }
            let dialect = Dialect::new(
                require_record(&state, &conn)
                    .await
                    .map_err(|e| e.message)?
                    .kind(),
            );
            let target_name = dialect.quote(&table);
            let columns = written_columns(&planned, dialect);
            if columns.is_empty() {
                return Err("所有字段都被设为「不写入」".to_string());
            }

            if clear_before {
                task.step(format!("先清空 {table}"));
                run_sql(&state, &conn, &database, &format!("delete from {target_name}"))
                    .await
                    .map_err(|e| e.message)?;
            }

            let multi = supports_multi_row(dialect);
            // 每条语句的行数 = 用户批大小，但**必须再被方言硬限制夹住**：
            // SQL Server 一条 INSERT 超过 1000 行值直接报 10738（真机踩过：批 5000 全炸）。
            let step = if multi { batch.min(max_tuples_per_insert(dialect)) } else { 1 };
            let started = std::time::Instant::now();
            let mut written = 0u64;
            while written < rows {
                task.check_canceled()?;
                let count = step.min(rows - written);
                let sql = if multi {
                    build_insert(&target_name, &columns, &planned, written, count, seed, dialect)
                } else {
                    build_insert(&target_name, &columns, &planned, written, 1, seed, dialect)
                };
                run_sql(&state, &conn, &database, &sql).await.map_err(|err| {
                    format!(
                        "写入第 {}-{} 行失败：{}",
                        written + 1,
                        written + count,
                        err.message
                    )
                })?;
                written += count;
                task.set_done(written);
                task.set_phase(format!("已写入 {written}/{rows} 行"));
                if written % (step * 10).max(1) == 0 {
                    task.log(format!("已写入 {written} 行"));
                }
            }
            let elapsed = started.elapsed().as_millis() as u64;
            let message = format!("生成完成：{written} 行 → {table}（连接 {label}）");
            task.set_message(message.clone());
            Ok(Some(json!({
                "success": true,
                "rows": written,
                "table": table,
                "affectedRows": written,
                "executeTime": elapsed,
                "notices": notices,
            })))
        }
    });

    Ok(Json(json!({
        "success": true,
        "taskId": task.id,
        "database": body.database,
        "table": body.table,
        "status": "running",
        "done": 0,
        "total": rows,
        "percent": 0,
        "notices": notices,
    })))
}

/// 任务快照 →上游的造数任务形状。
///
/// 字段名必须对上：`stage` 用于日志去重、`percent` 优先于 done/total 显示进度、
/// `error` 才是失败原因（前端不读 `message`）。map 一次，省得前端各读各的。
fn datagen_view(task: &crate::api::tasks::Task) -> Value {
    let snapshot = task.snapshot();
    let done = snapshot["done"].as_u64().unwrap_or(0);
    let total = snapshot["total"].as_i64().unwrap_or(-1);
    let percent = if total > 0 {
        ((done as f64 / total as f64) * 100.0).round().min(100.0) as i64
    } else {
        -1
    };
    let status = snapshot["status"].clone();
    let message = snapshot["message"].clone();
    let mut view = snapshot;
    view["stage"] = view["phase"].clone();
    view["percent"] = json!(percent);
    view["canceled"] = json!(task.is_canceled());
    view["error"] = if status == json!("error") {
        message.clone()
    } else {
        Value::Null
    };
    if let Some(context) = task.context() {
        view["database"] = context.get("database").cloned().unwrap_or(Value::Null);
        view["table"] = context.get("table").cloned().unwrap_or(Value::Null);
    }
    if let Some(result) = task.result() {
        view["affectedRows"] = result.get("affectedRows").cloned().unwrap_or(Value::Null);
        view["executeTime"] = result.get("executeTime").cloned().unwrap_or(Value::Null);
        view["notices"] = result.get("notices").cloned().unwrap_or(json!([]));
    } else {
        view["affectedRows"] = json!(done);
        view["executeTime"] = json!(0);
        view["notices"] = json!([]);
    }
    view
}

/// `GET /api/datagen/task/{taskId}` —— 进度。
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
    let mut view = datagen_view(&task);
    view["success"] = json!(true);
    Ok(Json(view))
}

/// `POST /api/datagen/cancel/{taskId}` —— 请求取消。
pub async fn cancel(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> XResult<Json<Value>> {
    let Some(task) = state.tasks.get(&task_id) else {
        return Ok(Json(json!({
            "success": false,
            "status": "notfound",
            "message": "任务不存在或已过期",
        })));
    };
    task.cancel();
    let mut view = datagen_view(&task);
    view["success"] = json!(true);
    view["message"] = json!("已请求取消，当前批次结束后停止");
    Ok(Json(view))
}
