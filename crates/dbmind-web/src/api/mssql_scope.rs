//! SQL Server 的「当前模式」到底按哪个解析。
//!
//! 背景：SQL Server **没有** `SET SCHEMA` / `search_path` 这类会话级开关。未限定的表名
//! 一律按「该库下当前用户的默认模式」（通常 `dbo`）解析。JDBC 的 `Connection.setSchema()`
//! 在 mssql-jdbc 上不起作用（宿主里那次调用失败后被静默 `trace` 掉了），于是编辑器里选了
//! `test` 模式、SQL 写 `select * from [D_DATE]`，查到的仍是 `dbo.D_DATE` —— 用户看到的
//! 就是「选了 test 还是查 dbo」。
//!
//! 这里做的是**受控改写**：只给「FROM / JOIN / UPDATE / INTO 后面、在选中模式下确实存在、
//! 且自己没写前缀」的表名补上 `[模式].`。守住四条边界，避免把 SQL 改坏：
//!   1. 不是 SQL Server、或没选模式 ⇒ 原样返回（零开销）；
//!   2. 名字里已带点（`dbo.order`）⇒ 尊重用户写的前缀，不动；
//!   3. 临时表（`#t`）、表变量（`@t`）、CTE 名、函数调用（名字后紧跟 `(`）、
//!      派生子查询（括号开头）⇒ 不动；
//!   4. 名字必须在目标模式的 `sys.objects` 里真的存在，否则不动（可能是同义词 / 拼写指向
//!      dbo 的表 —— 宁可维持现状，也不要把它改成一个不存在的对象）。
//!
//! DDL 的落点（`create/alter/drop/truncate table`）是例外：**新表当然还不在元数据里**，
//! 所以那一处只要没写前缀就直接补，这正是「在 test 模式下建表却建到 dbo」的修法。

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::dialect::Dialect;
use crate::AppState;

/// 元数据缓存的存活时间：schema 里加减表不算高频，60 秒足够避免逐句查一遍
const CACHE_TTL: Duration = Duration::from_secs(60);

type CacheEntry = (Instant, std::sync::Arc<HashSet<String>>);

fn cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 给一条 SQL 补上「选中模式」前缀；不满足条件时原样返回。
///
/// `database` 是前端传的「库.模式」（schema 型方言的约定），模式就从它取。
pub(crate) async fn apply(
    state: &AppState,
    conn: &str,
    database: &str,
    dialect: Dialect,
    sql: String,
) -> String {
    if dialect.key() != "sqlserver" {
        return sql;
    }
    let (db, schema) = super::meta::split_db_schema(database, &dialect);
    let Some(schema) = schema.filter(|s| !s.trim().is_empty()) else {
        return sql;
    };
    // 没有 FROM / JOIN / DDL 的表引用就没什么可改的 —— 先做一次字符串级快筛，
    // 避免每条 `select 1` 都去查一遍元数据
    if !may_reference_table(&sql) {
        return sql;
    }
    let known = match objects_of(state, conn, &db, &schema).await {
        Some(set) => set,
        // 取不到清单（权限/连接抖动）：保持原样执行，绝不因为「补前缀」让语句失败
        None => return sql,
    };
    rewrite(&sql, &schema, &known)
}

fn may_reference_table(sql: &str) -> bool {
    let lower = sql.to_ascii_lowercase();
    ["from", "join", "into", "update", "table"]
        .iter()
        .any(|kw| lower.contains(kw))
}

/// 目标模式下的表/视图/同义词名（小写，便于大小写不敏感比对）。
async fn objects_of(
    state: &AppState,
    conn: &str,
    db: &str,
    schema: &str,
) -> Option<std::sync::Arc<HashSet<String>>> {
    let key = format!("{conn}\u{1}{db}\u{1}{schema}");
    if let Ok(map) = cache().lock() {
        if let Some((at, set)) = map.get(&key) {
            if at.elapsed() < CACHE_TTL {
                return Some(set.clone());
            }
        }
    }
    let lit = format!("N'{}'", schema.replace('\'', "''"));
    let sql = format!(
        "select name from sys.objects where schema_id = schema_id({lit}) and type in ('U','V','SN')"
    );
    // **必须走 run_meta_sql**（而不是 run_sql_in）：后者自己也会调用本模块的 `apply`，
    // 形成「改写 → 查元数据 → 改写」的递归，async fn 会被编译器拒（E0733）。这条查询是
    // 方言无关的 sys.objects，正好属于 run_meta_sql 的用途。
    let result = super::meta::run_meta_sql(state, conn, db, sql, 20000).await.ok()?;
    let mut set = HashSet::new();
    for row in result.rows {
        if let Some(dbmind_core::CellValue::Text(name)) = row.first() {
            set.insert(name.trim().to_ascii_lowercase());
        }
    }
    let set = std::sync::Arc::new(set);
    if let Ok(mut map) = cache().lock() {
        map.insert(key, (Instant::now(), set.clone()));
    }
    Some(set)
}

// ------------------------------------------------------------------ 纯文本改写

/// 出现这些关键字 ⇒ 下一个标识符是表引用位置
const TABLE_INTRO: &[&str] = &["from", "join", "apply", "update", "into"];
/// 出现这些关键字 ⇒ FROM 列表结束（逗号后面不再是表）
const FROM_TERMINATORS: &[&str] = &[
    "where", "group", "order", "having", "union", "except", "intersect", "set", "values",
    "select", "on", "using", "limit", "offset", "fetch", "option", "for", "window",
];
/// DDL 前缀词：后面跟 `table` 时，那个表名是"落点"
const DDL_LEAD: &[&str] = &["create", "alter", "drop", "truncate"];

/// 表引用位置的种类（决定 `(` 怎么解释、要不要查元数据）
const POS_NONE: u8 = 0;
const POS_FROM: u8 = 1; // from / join / apply：后面可能是表值函数
const POS_TARGET: u8 = 2; // update / insert into：后面可能是列清单
const POS_DDL: u8 = 3; // create/alter/drop/truncate table：新对象还不在元数据里

/// 给未限定表名补 `[schema].`。纯函数，便于单测。
pub(crate) fn rewrite(sql: &str, schema: &str, known: &HashSet<String>) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let n = chars.len();
    let ctes = collect_ctes(&chars);
    let quoted_schema = format!("[{}].", schema.replace(']', "]]"));

    let mut out = String::with_capacity(sql.len() + 32);
    let mut i = 0usize;
    let mut expect_table = false;
    // 位置种类：DDL 落点（新对象还不在元数据里，只要有模式就补）/ FROM / 写语句目标
    let mut expect_kind: u8 = POS_NONE;
    let mut in_from = false;
    let mut prev_kw: Option<String> = None;

    while i < n {
        let c = chars[i];

        // 单引号字符串：原样拷贝（里面的 from / 逗号 都不是语法）
        if c == '\'' {
            let start = i;
            i += 1;
            while i < n {
                if chars[i] == '\'' {
                    if i + 1 < n && chars[i + 1] == '\'' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.extend(&chars[start..i]);
            continue;
        }
        // 行注释 / 块注释
        if c == '-' && i + 1 < n && chars[i + 1] == '-' {
            let start = i;
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            out.extend(&chars[start..i]);
            continue;
        }
        if c == '/' && i + 1 < n && chars[i + 1] == '*' {
            let start = i;
            i += 2;
            while i + 1 < n && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(n);
            out.extend(&chars[start..i]);
            continue;
        }

        // 标识符（`[..]` / `".."` / 裸名）
        if let Some(ident) = read_ident(&chars, i) {
            let name = ident.name.to_ascii_lowercase();
            let bare = ident.quoted.is_none();

            if expect_table {
                let after = skip_blanks(&chars, ident.end);
                let next = chars.get(after).copied();
                let qualified = next == Some('.');
                // `(` 的含义看位置：FROM 后面是**表值函数**（`from f_split(…)`，不能补前缀），
                // 而 `create table t (…)` / `insert into t (a,b)` 后面是**列清单**（必须补）。
                let call = next == Some('(') && expect_kind == POS_FROM;
                let needs = expect_kind != POS_NONE
                    && !qualified
                    && !call
                    && !name.starts_with('#')
                    && !name.starts_with('@')
                    && !ctes.contains(&name)
                    && (expect_kind == POS_DDL || known.contains(&name));
                if needs {
                    out.push_str(&quoted_schema);
                }
                out.extend(&chars[ident.start..ident.end]);
                i = ident.end;
                expect_table = false;
                expect_kind = POS_NONE;
                continue;
            }

            if bare {
                if TABLE_INTRO.contains(&name.as_str()) {
                    expect_table = true;
                    in_from = true;
                    expect_kind = if matches!(name.as_str(), "into" | "update") {
                        POS_TARGET
                    } else {
                        POS_FROM
                    };
                } else if name == "table"
                    && prev_kw.as_deref().is_some_and(|k| DDL_LEAD.contains(&k))
                {
                    expect_table = true;
                    in_from = false;
                    expect_kind = POS_DDL;
                } else if FROM_TERMINATORS.contains(&name.as_str()) {
                    in_from = false;
                    expect_table = false;
                    expect_kind = POS_NONE;
                }
                prev_kw = Some(name);
            }

            out.extend(&chars[ident.start..ident.end]);
            i = ident.end;
            continue;
        }

        match c {
            // FROM 列表里的逗号：下一个还是表
            ',' => {
                if in_from {
                    expect_table = true;
                    expect_kind = POS_FROM;
                }
                out.push(c);
                i += 1;
            }
            // 括号开头不可能是表引用位置（子查询 / 表达式）；括号内部照常扫
            '(' | ')' => {
                expect_table = false;
                expect_kind = POS_NONE;
                out.push(c);
                i += 1;
            }
            // 语句边界：状态全部复位（脚本里多句很常见）
            ';' => {
                expect_table = false;
                expect_kind = POS_NONE;
                in_from = false;
                prev_kw = None;
                out.push(c);
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

struct Ident {
    /// 原文（含引号）在 chars 里的区间
    start: usize,
    end: usize,
    /// 去引号后的名字
    name: String,
    /// Some(quote) 表示带引号的标识符（`[x]` / `"x"`），不当关键字看
    quoted: Option<char>,
}

fn read_ident(chars: &[char], i: usize) -> Option<Ident> {
    let c = *chars.get(i)?;
    if c == '[' {
        let mut j = i + 1;
        let mut name = String::new();
        while j < chars.len() {
            if chars[j] == ']' {
                if chars.get(j + 1) == Some(&']') {
                    name.push(']');
                    j += 2;
                    continue;
                }
                j += 1;
                break;
            }
            name.push(chars[j]);
            j += 1;
        }
        return Some(Ident { start: i, end: j, name, quoted: Some('[') });
    }
    if c == '"' {
        let mut j = i + 1;
        let mut name = String::new();
        while j < chars.len() {
            if chars[j] == '"' {
                if chars.get(j + 1) == Some(&'"') {
                    name.push('"');
                    j += 2;
                    continue;
                }
                j += 1;
                break;
            }
            name.push(chars[j]);
            j += 1;
        }
        return Some(Ident { start: i, end: j, name, quoted: Some('"') });
    }
    if !(c.is_alphanumeric() || matches!(c, '_' | '#' | '@' | '$')) {
        return None;
    }
    let mut j = i;
    let mut name = String::new();
    while j < chars.len() {
        let ch = chars[j];
        if ch.is_alphanumeric() || matches!(ch, '_' | '#' | '@' | '$') {
            name.push(ch);
            j += 1;
        } else {
            break;
        }
    }
    if j == i {
        return None;
    }
    Some(Ident { start: i, end: j, name, quoted: None })
}

fn skip_blanks(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// CTE 的名字：`with a as (…), b as (…)`。判断口径很宽 —— 任何「标识符 + as + (」都算，
/// 宁可少改一处（CTE 引用本来也不该补前缀），也不要把它改成 `[schema].[cte]`。
fn collect_ctes(chars: &[char]) -> HashSet<String> {
    let mut out = HashSet::new();
    let n = chars.len();
    let mut i = 0usize;
    while i < n {
        let Some(ident) = read_ident(chars, i) else {
            i += 1;
            continue;
        };
        if ident.quoted.is_none() && ident.name.eq_ignore_ascii_case("as") {
            i = ident.end;
            continue;
        }
        let mut j = skip_blanks(chars, ident.end);
        // 允许列名清单：`cte (a, b) as (…)`
        if chars.get(j) == Some(&'(') {
            while j < n && chars[j] != ')' {
                j += 1;
            }
            j = skip_blanks(chars, (j + 1).min(n));
        }
        if j + 1 < n
            && (chars[j] == 'a' || chars[j] == 'A')
            && (chars[j + 1] == 's' || chars[j + 1] == 'S')
        {
            let k = skip_blanks(chars, j + 2);
            if chars.get(k) == Some(&'(') {
                out.insert(ident.name.to_ascii_lowercase());
            }
        }
        i = ident.end.max(i + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_ascii_lowercase()).collect()
    }

    #[test]
    fn unqualified_table_gets_schema_prefix() {
        let known = set(&["d_date", "orders"]);
        let got = rewrite("select * from [D_DATE] where 1=1", "test", &known);
        assert_eq!(got, "select * from [test].[D_DATE] where 1=1");
    }

    #[test]
    fn explicit_prefix_is_respected() {
        let known = set(&["orders"]);
        let got = rewrite("select * from dbo.[orders]", "test", &known);
        assert_eq!(got, "select * from dbo.[orders]");
        let got = rewrite(
            "select * from [dbo].[orders] o join demo.orders x on 1=1",
            "test",
            &known,
        );
        assert_eq!(got, "select * from [dbo].[orders] o join demo.orders x on 1=1");
    }

    #[test]
    fn unknown_name_is_left_alone() {
        let known = set(&["orders"]);
        let got = rewrite("select * from dbo_only", "test", &known);
        assert_eq!(got, "select * from dbo_only", "不在目标模式里就别改");
    }

    #[test]
    fn subquery_function_temp_table_untouched() {
        let known = set(&["orders", "f_split"]);
        for sql in [
            "select * from (select 1 as a) x",
            "select * from f_split('a,b')",
            "select * from #tmp",
            "select * from @tv",
        ] {
            assert_eq!(rewrite(sql, "test", &known), sql);
        }
        assert_eq!(
            rewrite("select a, count(*) from orders group by a", "test", &known),
            "select a, count(*) from [test].orders group by a"
        );
    }

    #[test]
    fn cte_and_comma_list() {
        let known = set(&["orders", "items", "c"]);
        let got = rewrite("with c as (select * from orders) select * from c", "test", &known);
        assert_eq!(got, "with c as (select * from [test].orders) select * from c");
        let got = rewrite("select * from orders, items where 1=1", "test", &known);
        assert_eq!(got, "select * from [test].orders, [test].items where 1=1");
    }

    #[test]
    fn ddl_target_always_prefixed() {
        let known = set(&[]);
        let got = rewrite("create table [t1] ([id] varchar(20))", "test", &known);
        assert_eq!(got, "create table [test].[t1] ([id] varchar(20))");
        let got = rewrite("alter table [dbo].[t1] add c int", "test", &known);
        assert_eq!(got, "alter table [dbo].[t1] add c int");
    }

    #[test]
    fn multi_statement_resets_state() {
        let known = set(&["a", "b"]);
        let got = rewrite("select * from a; select * from b", "test", &known);
        assert_eq!(got, "select * from [test].a; select * from [test].b");
    }

    #[test]
    fn keywords_in_strings_and_comments_ignored() {
        let known = set(&["orders"]);
        let got = rewrite(
            "select 'from orders' as x -- from orders\nfrom orders",
            "test",
            &known,
        );
        assert_eq!(
            got,
            "select 'from orders' as x -- from orders\nfrom [test].orders"
        );
    }
}
