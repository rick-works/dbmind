//! 给模型用的**预算版**表结构上下文。
//!
//! ## 为什么要单独一份
//!
//! `mod.rs` 的 `schema_context` 是**全量语义**：没点名表时把整库每张表的列结构 + 2 行样例
//! 全拼进去（"数据字典/预热"要的就是全量）。但把它直接喂给模型会出大事 ——
//! 真机一次 AI 修复实测 `promptTokens = 282224`（28 万）：
//!
//! - 单次成本约 0.3 元，用户一天点几十次就是几十块；
//! - 首字延迟十几秒，看着像卡死；
//! - 上千张无关表把模型的注意力稀释掉，生成的 SQL 反而更容易编字段。
//!
//! 所以模型侧走这个模块，做三件事：
//!
//! 1. **先挑表**：调用方点名的 > 从 SQL / 问题里认出来的 > 默认前几张；其余表只留表名。
//! 2. **按预算展开**：默认 12000 tokens（`DBMIND_AI_CTX_TOKENS` 可覆盖），
//!    超了就停在**最后一张完整的表**上 —— 绝不切半张表，切一半的列结构会让模型
//!    写出根本不存在的字段，比不给还糟。
//! 3. **明说裁掉了什么**：末尾告诉模型「另有 N 张表只列了表名」。它会主动让你指明表名，
//!    而不是硬着头皮编一张出来。
//!
//! ## 为什么估 token 不用 tokenizer
//!
//! 引一个分词器要多几 MB 依赖、每种模型还不一样。这里的用途只有一个「别超太多」，
//! 按 `chars * 0.4` 粗估（中文约 1 字 1 token、英文约 4 字符 1 token，混排取中间）——
//! 偏保守（估多），宁可少塞几张表也不超预算。

use dbmind_core::{ColumnDetail, TableKind};
use serde_json::Value;

use crate::api::{blocking, require_record};
use crate::AppState;

/// 小库的门槛：表数不超过这个值才把表名整体列出来。
///
/// 为什么不是「列前 150 个」：实测账本里，一个 395 张表的库，清单独占 2250 字符 ≈ 900 token
/// （整个请求的 43%），而模型拿这 150 个名字**写不出一句正确 SQL**（没有列结构）。
/// 小库列全名是划算的（300 字符、且大概率覆盖到目标表）；大库列名字只是在烧钱，
/// 所以只留「共 N 张」这一句 —— 模型据此知道库很大，会来问表名，而不是照着陌生的名字瞎编。
const SMALL_DB_TABLES: usize = 40;
/// 没点名时默认展开几张表的列结构。
const DEFAULT_EXPAND: usize = 8;
/// 点名 / 命中时最多展开几张（防前端一次传几百张）。
const MAX_EXPAND: usize = 40;
/// 每张表给几行样例（`limit_clause` 要 u64，`run_sql_in` 要 usize，统一在这里声明）。
///
/// 1 行足够让模型看出取值格式（日期写法、枚举值），第 2 行的边际收益远小于它的 token。
const SAMPLE_ROWS: u64 = 1;
/// 样例行最多带几列。宽表（几十个 `XF_*` 列那种）一行的样例就能写掉几百字符，
/// 而前 20 列已经足够看出日期格式与枚举取值 —— 但**列结构本身一行都不少**（那才是写 SQL 的依据）。
const SAMPLE_MAX_COLS: usize = 20;

/// 预算版上下文。
pub struct Budgeted {
    pub text: String,
    /// 库里一共多少张表
    pub tables: usize,
    /// 服务器版本串（取不到为空）
    pub version: String,
    /// 实际展开了几张表
    pub expanded: usize,
    /// 实际展开的表名（排障用：判断「它到底看没看到那张表」）
    pub expanded_names: Vec<String>,
    /// 实际送出去多少列
    pub columns: usize,
    /// 估算 token（用于日志与排障）
    pub estimated_tokens: usize,
    /// 这段上下文是不是「全库清单」（表名 + 注释，没有列结构）
    pub catalog: bool,
}

/// 估 token。按字符种类分开算，而不是「一律 0.4/字符」。
///
/// **老公式（`字符数 * 2 / 5`）把中文当成了 0.4 token/字，实测低估约 3 倍**：
/// 同一个修复请求，账本显示 780，而模型返回的 `promptTokens` 是 2345。后果是 12k 的预算闸门
/// 实际放到 36k 才拦 —— 「上下文有上限」这件事只是名义上成立。
/// 校准口径（BPE）：中日韩文字与全角标点约 4/5 token/字，ASCII/拉丁约 1/4 token/字符。
/// 保守（略高估）比较好：宁可少塞一张表，也不要账面好看、账单难看。
pub fn estimate_tokens(text: &str) -> usize {
    let mut wide = 0usize;
    let mut narrow = 0usize;
    for ch in text.chars() {
        if is_wide(ch) {
            wide += 1;
        } else {
            narrow += 1;
        }
    }
    wide * 4 / 5 + narrow / 4 + 1
}

/// 中日韩文字 / 全角标点 / 全角字符：BPE 里基本一字一 token。
fn is_wide(ch: char) -> bool {
    matches!(
        ch as u32,
        // 部首/标点/假名/汉字（含扩展 A）/朝鲜文音节：0x2E80..=0x9FFF 已覆盖 CJK 标点
        0x2E80..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFFEF
    )
}

/// 上下文预算，默认 12000 tokens（**真实 token**，不是老公式那种按字符数打折的估算）。
pub fn budget_tokens() -> usize {
    std::env::var("DBMIND_AI_CTX_TOKENS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value >= 1_000)
        .unwrap_or(12_000)
}

/// 构建上下文的选项。用结构体而不是一串 bool：这几个开关的含义差别很大，
/// 调用点写成 `Options { names: false, .. }` 比 `build(.., false, true)` 可读得多。
#[derive(Clone, Copy)]
pub struct Options {
    /// 是否列出未展开的表名（只在**小库**生效，见 [`SMALL_DB_TABLES`]）
    pub names: bool,
    /// 一张表都没认出来时，要不要兜底展开前几张（大库下默认不兜底）
    pub fallback: bool,
    /// **库级问题**：给全库清单（表名 + 表注释），而不是某几张表的列结构
    pub catalog: bool,
    /// token 预算；None = [`budget_tokens`]
    pub budget: Option<usize>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            names: true,
            fallback: false,
            catalog: false,
            budget: None,
        }
    }
}

/// 全库清单最多占多少字符（≈3300 token）。
///
/// 定这个数是按真机量的：395 张表的「名字 + 注释」约 8.5k 字符，9000 恰好**整库放得下**
/// —— 库级问题的答案质量全看覆盖面，少一半表就会漏掉半个业务域。更大的库（几千张）
/// 会在末尾注明「共 N 张，此处列 M 张」，让模型知道还有多少没看到。
const CATALOG_CHARS: usize = 9_000;

/// 这个问题是不是在问**整个库**（而不是某几张表）？
///
/// 判据分两档：
/// - 「有哪些表 / 表清单 / 多少张表 / 所有表」这类**明确的列清单请求**，命中即可；
/// - 其余要「提到库」+「问整体是什么」（干嘛/业务/用途/概览/架构/梳理）。
///
/// 为什么不能只看"干嘛"：「这个表是干嘛的」也含这两个字，而它要的是那张表的列结构。
pub fn wants_catalog(question: &str) -> bool {
    let q = question.to_ascii_lowercase();
    const LISTING: &[&str] = &[
        "有哪些表", "什么表", "表清单", "表列表", "多少张表", "几张表", "所有表", "全部表",
        "list tables", "all tables", "how many tables",
    ];
    if LISTING.iter().any(|word| q.contains(word)) {
        return true;
    }
    const OBJECT: &[&str] = &["库", "database", "schema"];
    const INTENT: &[&str] = &[
        "干嘛", "干什么", "做什么", "什么业务", "业务", "用途", "概览", "总览", "架构", "梳理",
        "什么系统", "什么项目", "overview", "purpose", "what is",
    ];
    OBJECT.iter().any(|word| q.contains(word)) && INTENT.iter().any(|word| q.contains(word))
}

impl Options {
    /// 「已经给了 SQL」的场景（修复/解释/优化/改写）：表名清单是纯噪声，只留总数。
    pub fn omit_names() -> Self {
        Self {
            names: false,
            ..Self::default()
        }
    }

    /// 批量生成类（数据字典）：要的就是一批表的列结构，认不出也兜底展开，预算放宽。
    pub fn bulk() -> Self {
        Self {
            names: false,
            fallback: true,
            catalog: false,
            budget: Some(32_000),
        }
    }
}

/// 构建给模型的表结构上下文（预算内）。取不到表就返回空文本，让调用方降级。
pub async fn build(
    state: &AppState,
    conn: &str,
    database: &str,
    focus: &[String],
    hint: &str,
    options: Options,
) -> crate::api::error::XResult<Budgeted> {
    let budget = options.budget.unwrap_or_else(budget_tokens);
    let with_names = options.names;
    let fallback_expand = options.fallback;
    let record = require_record(state, conn).await?;
    let kind = record.kind();
    let dialect = crate::api::dialect::Dialect::new(kind);

    let target = crate::api::scope::resolve(state, conn, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
    // 服务器版本：**必须给模型**。5.7 没有窗口函数/CTE，2012 之前的 SQL Server 没有
    // OFFSET FETCH —— 同一句 SQL 换个版本就是语法错，而模型不知道版本时只会按最新版写。
    let version = server_version(state, conn, database, kind).await;
    let tables_target = target.clone();
    let tables = {
        // engine 是 Arc：每次用都取一份，不要 move 进闭包后再用第二次
        let engine = state.engine();
        blocking(move || engine.list_tables_fresh(&tables_target)).await?
    };
    let names: Vec<String> = tables
        .iter()
        .filter(|table| table.kind == TableKind::Table)
        .map(|table| table.name.clone())
        .collect();

    // 挑表：**聚焦的表 + 从 SQL/问题/历史里认出来的表，求并集** > 小库默认前几张。
    //
    // 求并集而不是「聚焦优先、有聚焦就不看问题」：真机反馈「我一次问了 a、b 两张表，
    // 你只答了 a」—— 因为右键聚焦过 a，范围被钉死在 a 上了。并集之后，问到几张就展开几张。
    let small_db = names.len() <= SMALL_DB_TABLES;
    let mut expanded: Vec<String> = focus
        .iter()
        .filter(|name| names.iter().any(|known| known.eq_ignore_ascii_case(name)))
        .cloned()
        .collect();
    if !hint.trim().is_empty() {
        for name in mentioned(hint, &names, MAX_EXPAND) {
            if !expanded.iter().any(|hit| hit.eq_ignore_ascii_case(&name)) {
                expanded.push(name);
            }
        }
    }
    if expanded.is_empty() && (small_db || fallback_expand) {
        // 大库**默认不做**这个兜底：395 张表按字母序取前 8 张，与问题几乎必然无关，
        // 白白送出去 8 张无关表的列结构（实测 1600 字符），还会把模型带偏成"照着无关表编 SQL"。
        // 小库（≤40 张）取前几张则很可能就覆盖到目标表，成本也低；
        // 数据字典（fallback_expand=true）本来就要一批表的列结构，也走这条。
        expanded = names.iter().take(DEFAULT_EXPAND).cloned().collect();
    }
    expanded.truncate(MAX_EXPAND);
    // 大库里一张表都没认出来（既没点名、问题里也没提到表名）⇒ 与其只回一句「共 395 张表」
    // 让模型说「推断不了」，不如把**全库清单（表名 + 注释）**给它当最小可用上下文。
    // 没有列结构时它会来问表名，而不是照着陌生的名字编字段。
    //
    // 只在 `names` 开着时兜底：`omit_names` 服务的是「已经递了 SQL」的修复/解释流程，
    // 那种请求要的是**精简**（少 1 万字符的表清单就少一份干扰），表名清单本来就是噪声。
    let catalog =
        options.catalog || (with_names && expanded.is_empty() && !small_db && !names.is_empty());

    let mut text = String::new();
    // 版本一定要写进来：模型靠它决定能不能用窗口函数 / CTE / JSON 函数 / OFFSET FETCH。
    // 取不到时不是留空，而是要求它避开新特性 —— 留空等于默许它按最新版写。
    let version_part = if version.is_empty() {
        " ｜ 版本未知（避开窗口函数 / CTE / JSON 函数等版本相关特性）".to_string()
    } else {
        format!(" ｜ 版本 {version}")
    };
    text.push_str(&format!(
        "【目标数据库】类型 {}{} ｜ 标识符引用风格 {} ｜ 分页写法 `{}`\n",
        kind.key().to_ascii_uppercase(),
        version_part,
        match kind.key() {
            "mysql" | "mariadb" | "doris" | "clickhouse" => "反引号",
            "sqlserver" => "方括号",
            _ => "双引号",
        },
        dialect.limit_clause(0, 10)
    ));
    if catalog {
        // 库级问题（「这个库是干嘛的」）：给**全库清单 + 表注释**。
        // 这是唯一既便宜又有效的依据 —— 395 张表的列结构要几十万 token，而「表名 + 注释」
        // 几千 token 就能让模型把业务说清楚。以前这种情况一张表都不展开、只给个总数，
        // 模型只能回「上下文里只有两张表，推断不了整个库」，然后让用户自己去敲 SQL。
        let comments = crate::api::meta::table_comment_map(state, conn, database).await;
        let mut lines: Vec<String> = Vec::new();
        let mut used = 0usize;
        for name in &names {
            let comment = comments
                .get(&name.to_ascii_lowercase())
                .map(|text| text.trim())
                .filter(|text| !text.is_empty());
            let line = match comment {
                Some(comment) => format!("  {name} —— {comment}"),
                None => format!("  {name}"),
            };
            if used + line.chars().count() > CATALOG_CHARS {
                break;
            }
            used += line.chars().count() + 1;
            lines.push(line);
        }
        text.push_str(&format!(
            "【全库清单】共 {} 张表，此处列 {} 张（格式：表名 —— 注释，不含列结构）\n",
            names.len(),
            lines.len()
        ));
        text.push_str(&lines.join("\n"));
        text.push('\n');
        // 把「这里没有列结构」说白了：否则模型会照着表名猜字段，再编出一条跑不通的 SQL。
        text.push_str("（这里没有列结构：先讲清业务；要写 SQL 请先问明具体的表名，不要凭表名猜字段。）\n");
    } else {
        text.push_str("【表清单】");
        if with_names && small_db {
            text.push_str(&names.join(", "));
        } else {
            // 只留总数：模型据此知道"这库很大、别以为就这几张表"，但不为看不到结构的名字付钱。
            // 也顺带告诉它下一步该说什么（问表名），而不是硬着头皮编。
            text.push_str(&format!(
                "共 {} 张表，仅展开用到的那几张（需要其它表的字段请先说明表名）",
                names.len()
            ));
        }
        text.push('\n');
    }

    // 逐表展开：超预算就整体丢掉这一张（不切半张），并把剩余表数写进提示
    let mut columns_total = 0usize;
    let mut done = 0usize;
    for table in &expanded {
        let mark = text.len();
        let columns_target = target.clone();
        let name = table.clone();
        let engine = state.engine();
        let Ok(columns) = blocking(move || engine.list_columns_fresh(&columns_target, &name)).await
        else {
            continue;
        };
        if columns.is_empty() {
            continue;
        }
        text.push_str(&format!("【表 {table}】\n"));
        for column in &columns {
            text.push_str(&column_line(column));
            text.push('\n');
        }
        let sample_sql = format!(
            "select * from {} {}",
            dialect.quote(table),
            dialect.limit_clause(0, SAMPLE_ROWS)
        );
        if let Ok(result) =
            crate::api::meta::run_sql_in(state, conn, database, sample_sql, SAMPLE_ROWS as usize)
                .await
        {
            for row in crate::api::meta::rows_of(&result) {
                let mut pairs: Vec<String> = row
                    .iter()
                    .take(SAMPLE_MAX_COLS)
                    .map(|(key, value)| format!("{key}={}", short(value)))
                    .collect();
                if row.len() > SAMPLE_MAX_COLS {
                    pairs.push("…".to_string());
                }
                text.push_str(&format!("  样例：{}\n", pairs.join(", ")));
            }
        }
        if estimate_tokens(&text) > budget {
            // 这一张已经放不下了：回滚到拼之前，保证模型看到的每张表都是完整的
            text.truncate(mark);
            break;
        }
        columns_total += columns.len();
        done += 1;
    }

    let missing = names.len().saturating_sub(done);
    if done == 0 && !names.is_empty() && !catalog {
        // 大库 + 问题里没点到表 + 又不是库级问题：这时给模型的正确动作是「问表名」
        text.push_str("\n（还没有展开任何表结构：请先说明要用的表名，再据此写 SQL。）\n");
    } else if done > 0 && missing > 0 {
        text.push_str(&format!(
            "\n（只展开了 {done} 张表；需要其它表的字段请先说明表名。）\n"
        ));
    }

    Ok(Budgeted {
        estimated_tokens: estimate_tokens(&text),
        text: crate::api::ai::wrap_schema(&text),
        tables: names.len(),
        version,
        expanded: done,
        expanded_names: expanded.iter().take(done).cloned().collect(),
        columns: columns_total,
        catalog,
    })
}

/// 读一次服务器版本串；方言没有统一写法或查询失败都返回空串（不阻断上下文构建）。
async fn server_version(
    state: &AppState,
    conn: &str,
    database: &str,
    kind: dbmind_core::ConnectionKind,
) -> String {
    let dialect = crate::api::dialect::Dialect::new(kind);
    let Some(sql) = dialect.version_sql() else {
        return String::new();
    };
    let Ok(result) = crate::api::meta::run_meta_sql(state, conn, database, sql.to_string(), 1).await
    else {
        return String::new();
    };
    let raw = crate::api::meta::rows_of(&result)
        .into_iter()
        .next()
        .and_then(|row| row.values().next().cloned())
        .map(|value| match value {
            Value::String(text) => text,
            other => other.to_string(),
        })
        .unwrap_or_default();
    // 只留第一行、压掉连续空白、截 60 字符：`@@version` 回来是四行版权声明，
    // 整段塞进提示词既占 token 又没信息量
    let first = raw.lines().next().unwrap_or("").trim();
    let squeezed = first.split_whitespace().collect::<Vec<_>>().join(" ");
    squeezed.chars().take(60).collect()
}

fn column_line(column: &ColumnDetail) -> String {
    let mut line = format!(
        "  {} {}",
        column.name,
        column.type_name.clone().unwrap_or_else(|| "?".to_string())
    );
    if !column.nullable {
        line.push_str(" not null");
    }
    if column.primary_key {
        line.push_str(" 主键");
    }
    if let Some(default) = &column.default_value {
        if !default.trim().is_empty() {
            line.push_str(&format!(" default {}", default.trim()));
        }
    }
    line
}

/// 样例值截断。24 字符足够看出格式（`2026-10-30 12:00:00`、`SUCCESS`、`12345`），
/// 再长基本是 JSON 串或备注文本，对写 SQL 没有帮助却照样按 token 计费。
fn short(value: &Value) -> String {
    let text = match value {
        Value::Null => "NULL".to_string(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    text.chars().take(24).collect()
}

/// 问题/SQL 里点到的表名（按首次出现位置排序，取前 `limit` 个）。
///
/// 这是本模块最关键的一步：`fix` / `optimize` 的入参本身就带着 SQL，里面必有表名 ——
/// 认出来就等于"用户点名了这些表"，展开它们的列结构，其余的表只用表名占位。
fn mentioned(text: &str, names: &[String], limit: usize) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut hits: Vec<(usize, String)> = Vec::new();
    for name in names {
        // 太短的表名（t / id / a）在自然语言里几乎必被误命中，直接不参与
        if name.chars().count() < 3 {
            continue;
        }
        if let Some(position) = find_ident(&lower, &name.to_ascii_lowercase()) {
            hits.push((position, name.clone()));
        }
    }
    hits.sort_by_key(|(position, _)| *position);
    hits.into_iter().map(|(_, name)| name).take(limit).collect()
}

/// 按「标识符边界」找子串：`order` 不能命中 `orders` / `reorder_id`。
fn find_ident(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let mut from = 0usize;
    while from < haystack.len() {
        let Some(offset) = haystack[from..].find(needle) else {
            return None;
        };
        let position = from + offset;
        let before = haystack[..position].chars().next_back();
        let after = haystack[position + needle.len()..].chars().next();
        let boundary = |ch: Option<char>| match ch {
            None => true,
            Some(ch) => !(ch.is_alphanumeric() || ch == '_'),
        };
        if boundary(before) && boundary(after) {
            return Some(position);
        }
        from = position + needle.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_estimate_matches_real_tokenizer() {
        // ASCII ≈ 1/4 token/字符（老公式在这里是对的）
        let ascii = estimate_tokens(&"a".repeat(1000));
        assert!((240..=270).contains(&ascii), "ascii={ascii}");

        // 中文 ≈ 4/5 token/字。老公式（0.4/字符）低估一倍多，真机 promptTokens 也因此对不上
        let cjk = estimate_tokens(&"数".repeat(100));
        assert!((75..=85).contains(&cjk), "cjk={cjk}");

        // 实测校准点：489 字符的人格提示词，真实 promptTokens 约 280
        let base = include_str!("../../../prompts/common.default-system.md");
        let estimate = estimate_tokens(base);
        assert!((240..=320).contains(&estimate), "base={estimate}");
    }

    /// 「这个库是干嘛的」这类问题必须切到全库清单，而「这个表是干嘛的」不能。
    #[test]
    fn catalog_intent_is_detected_without_over_reaching() {
        for q in [
            "看下我连的这个数据库 是干嘛的",
            "这个库有哪些表？",
            "帮我梳理全库的业务架构",
            "库里一共多少张表",
            "what is this database for?",
            "list tables please",
        ] {
            assert!(wants_catalog(q), "「{q}」应走全库清单");
        }
        for q in [
            "这个表是干嘛的",
            "aibee_data_day 有哪些字段",
            "统计 2023 年每个月的销售额",
            "帮我优化这条 SQL",
            "为什么这里会全表扫描",
        ] {
            assert!(!wants_catalog(q), "「{q}」不该走全库清单");
        }
    }

    #[test]
    fn mentioned_respects_identifier_boundary() {
        let names = vec![
            "order".to_string(),
            "orders".to_string(),
            "user".to_string(),
        ];
        // "orders" 里含 "order"，但边界不对，不该被当成 order
        let hit = mentioned("select * from orders where id = 1", &names, 8);
        assert_eq!(hit, vec!["orders".to_string()]);

        // 真正的 order 表要被认出来，且排在 orders 之后（后出现）
        let hit = mentioned("select * from orders join order on 1=1", &names, 8);
        assert_eq!(hit, vec!["orders".to_string(), "order".to_string()]);
    }

    #[test]
    fn mentioned_skips_too_short_names() {
        let names = vec!["t".to_string(), "a".to_string()];
        assert!(mentioned("select * from t", &names, 8).is_empty());
    }
}
