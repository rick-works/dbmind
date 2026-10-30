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

/// 表名清单上限。只列名字成本很低（一张表约 15 字符），但 5000 张也会把预算吃光。
const MAX_TABLE_NAMES: usize = 300;
/// 没点名时默认展开几张表的列结构。
const DEFAULT_EXPAND: usize = 8;
/// 点名 / 命中时最多展开几张（防前端一次传几百张）。
const MAX_EXPAND: usize = 40;
/// 每张表给几行样例（`limit_clause` 要 u64，`run_sql_in` 要 usize，统一在这里声明）。
const SAMPLE_ROWS: u64 = 2;

/// 预算版上下文。
pub struct Budgeted {
    pub text: String,
    /// 库里一共多少张表
    pub tables: usize,
    /// 实际展开了几张表
    pub expanded: usize,
    /// 实际送出去多少列
    pub columns: usize,
    /// 估算 token（用于日志与排障）
    pub estimated_tokens: usize,
}

/// 粗估 token：中文约 1 字 1 token、ASCII 约 4 字符 1 token，混排按 0.4/字符保守估。
pub fn estimate_tokens(text: &str) -> usize {
    (text.chars().count() * 2 / 5) + 1
}

/// 上下文预算，默认 12000 tokens。
pub fn budget_tokens() -> usize {
    std::env::var("DBMIND_AI_CTX_TOKENS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value >= 1_000)
        .unwrap_or(12_000)
}

/// 构建给模型的表结构上下文（预算内）。取不到表就返回空文本，让调用方降级。
pub async fn build(
    state: &AppState,
    conn: &str,
    database: &str,
    focus: &[String],
    hint: &str,
) -> crate::api::error::XResult<Budgeted> {
    let budget = budget_tokens();
    let record = require_record(state, conn).await?;
    let kind = record.kind();
    let dialect = crate::api::dialect::Dialect::new(kind);

    let target = crate::api::scope::resolve(state, conn, database).await?;
    crate::api::driver::ensure_for_connection(state, &target).await?;
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

    // 挑表：点名 > 从 SQL/问题里认出来 > 默认前几张
    let mut expanded: Vec<String> = focus
        .iter()
        .filter(|name| names.iter().any(|known| known.eq_ignore_ascii_case(name)))
        .cloned()
        .collect();
    if expanded.is_empty() && !hint.trim().is_empty() {
        expanded = mentioned(hint, &names, MAX_EXPAND);
    }
    if expanded.is_empty() {
        expanded = names.iter().take(DEFAULT_EXPAND).cloned().collect();
    }
    expanded.truncate(MAX_EXPAND);

    let mut text = String::new();
    text.push_str(&format!(
        "【目标数据库】类型 {} ｜ 标识符引用风格 {} ｜ 分页写法 `{}`\n",
        kind.key().to_ascii_uppercase(),
        match kind.key() {
            "mysql" | "mariadb" | "doris" | "clickhouse" => "反引号",
            "sqlserver" => "方括号",
            _ => "双引号",
        },
        dialect.limit_clause(0, 10)
    ));
    text.push_str("【表清单】");
    text.push_str(
        &names
            .iter()
            .take(MAX_TABLE_NAMES)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
    );
    if names.len() > MAX_TABLE_NAMES {
        text.push_str(&format!(
            "（共 {} 张，此处只列前 {MAX_TABLE_NAMES} 张）",
            names.len()
        ));
    }
    text.push('\n');

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
                let pairs: Vec<String> = row
                    .iter()
                    .map(|(key, value)| format!("{key}={}", short(value)))
                    .collect();
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
    if missing > 0 {
        text.push_str(&format!(
            "\n（为控制上下文规模，本次只展开了 {done} 张表的列结构；另有 {missing} 张表只出现在上面的\
             表清单里。需要某张表的字段时，请先说明表名。）\n"
        ));
    }

    Ok(Budgeted {
        estimated_tokens: estimate_tokens(&text),
        text: crate::api::ai::wrap_schema(&text),
        tables: names.len(),
        expanded: done,
        columns: columns_total,
    })
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

fn short(value: &Value) -> String {
    let text = match value {
        Value::Null => "NULL".to_string(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    text.chars().take(40).collect()
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
    fn token_estimate_is_conservative() {
        // 1000 个 ASCII 字符 ≈ 350~400 tokens（0.4/字符），不会低于 /4 的乐观值
        let text = "a".repeat(1000);
        let estimate = estimate_tokens(&text);
        assert!((350..=450).contains(&estimate), "estimate={estimate}");
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
