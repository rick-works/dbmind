//! SQL 词法级别的安全判定：**拆语句**与**只读识别**。
//!
//! 这是安全策略的地基，所以它不依赖数据库、不依赖连接，纯字符串处理 ⇒ 可单测。
//! 刻意保守：识别不出来的一律当作「非只读」，宁可多拦不可漏放。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatementKind {
    /// 只读查询（SELECT / WITH...SELECT / SHOW / EXPLAIN 等）
    Read,
    /// 数据变更（INSERT / UPDATE / DELETE / MERGE ...）
    Write,
    /// 结构变更（CREATE / ALTER / DROP / TRUNCATE ...）
    Ddl,
    /// 无法归类（SET、CALL、方言特有语句等）—— 保守起见按非只读处理
    Unknown,
}

impl StatementKind {
    pub fn is_read_only(self) -> bool {
        matches!(self, StatementKind::Read)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            StatementKind::Read => "read",
            StatementKind::Write => "write",
            StatementKind::Ddl => "ddl",
            StatementKind::Unknown => "unknown",
        }
    }
}

#[derive(PartialEq)]
enum State {
    Normal,
    SingleQuote,
    DoubleQuote,
    Backtick,
    LineComment,
    BlockComment,
}

/// 单词字符（关键字/标识符用）。
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// 从 `from` 起跳过空白，读出下一个单词并大写。
///
/// 用来分辨 `BEGIN` 后是不是 `WORK`/`TRANSACTION`、`END` 后是不是 `IF`/`LOOP`。
/// 只跳空白，不识别中间夹的注释 —— 这两处之间夹注释的写法实践中不存在。
fn next_word(chars: &[char], from: usize) -> String {
    let mut i = from;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    let mut word = String::new();
    while i < chars.len() && is_word_char(chars[i]) {
        word.push(chars[i]);
        i += 1;
    }
    word.to_ascii_uppercase()
}

/// 按 `;` 拆分语句，**正确处理引号、注释与语句块**。
///
/// 三类 `;` 不算分隔符：`';'`（引号内）、`-- ;` / `/* ; */`（注释内）、
/// 以及 `BEGIN ... END` / `CASE ... END` **块内部**的（见下）。
///
/// 注释内容会被替换成空白（保持字符位置大致不变，便于后续定位报错）。
pub fn split_statements(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut state = State::Normal;
    // 语句块嵌套深度（`BEGIN`/`CASE` 开、`END` 关）。只有深度为 0 的 `;` 才切分。
    //
    // 为什么必须有这个深度：MySQL 的例程/触发器/事件体是 `BEGIN ... 语句; 语句; ... END`，
    // 体内的 `;` 是**体内语句的分隔符**，不是整个 CREATE 的结束。以前不认语句块，
    // 「编辑函数」生成的脚本会被切成 `DROP ...` / `CREATE ... BEGIN ...` / `END` 三段 ——
    // 第一段执行成功把对象删掉，第二段断在半截必然语法错误 ⇒ **对象被删且建不回来**。
    let mut depth: i32 = 0;
    // 当前正在读的单词：识别 BEGIN / END / CASE 用
    let mut word = String::new();
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];
        match state {
            State::Normal => {
                if is_word_char(c) {
                    word.push(c);
                    cur.push(c);
                    i += 1;
                    continue;
                }
                if !word.is_empty() {
                    match word.to_ascii_uppercase().as_str() {
                        // `BEGIN` 既可能开语句块，也可能是事务起点（`BEGIN;` / `BEGIN WORK` /
                        // `BEGIN TRANSACTION`）。后者没有配对的 END，误判会把后面所有语句吞成一条。
                        "BEGIN" => {
                            if !matches!(next_word(&chars, i).as_str(), "" | "WORK" | "TRANSACTION")
                            {
                                depth += 1;
                            }
                        }
                        // `CASE` 与 `END` 成对（表达式与语句都成对）：算进来才配平。
                        // 只把 `END` 算作关、不算 `CASE` 的开，块里的 `CASE ... END`
                        // 会把深度提前减到 0，后面的 `;` 又在块中间切开。
                        "CASE" => depth += 1,
                        "END" => {
                            // `END IF` / `END LOOP` / `END WHILE` / `END REPEAT` 收的是
                            // 上面没计数的 IF/LOOP/... 语句块；跳过它们以免把深度算塌。
                            // （`END CASE` 不在跳过之列 —— CASE 是计过数的。）
                            if !matches!(next_word(&chars, i).as_str(), "IF" | "LOOP" | "WHILE" | "REPEAT")
                            {
                                depth = (depth - 1).max(0);
                            }
                        }
                        _ => {}
                    }
                    word.clear();
                }
                if c == '\'' {
                    state = State::SingleQuote;
                    cur.push(c);
                } else if c == '"' {
                    state = State::DoubleQuote;
                    cur.push(c);
                } else if c == '`' {
                    state = State::Backtick;
                    cur.push(c);
                } else if c == '-' && chars.get(i + 1) == Some(&'-') {
                    state = State::LineComment;
                    cur.push(' ');
                    cur.push(' ');
                    i += 1;
                } else if c == '/' && chars.get(i + 1) == Some(&'*') {
                    state = State::BlockComment;
                    cur.push(' ');
                    cur.push(' ');
                    i += 1;
                } else if c == ';' {
                    if depth == 0 {
                        let trimmed = cur.trim();
                        if !trimmed.is_empty() {
                            out.push(trimmed.to_string());
                        }
                        cur.clear();
                    } else {
                        // 块内：分号属于体内语句，原样留着
                        cur.push(c);
                    }
                } else {
                    cur.push(c);
                }
            }
            State::SingleQuote => {
                cur.push(c);
                if c == '\'' {
                    // SQL 里的 '' 是转义的单引号，不结束字符串
                    if chars.get(i + 1) == Some(&'\'') {
                        cur.push('\'');
                        i += 1;
                    } else {
                        state = State::Normal;
                    }
                }
            }
            State::DoubleQuote => {
                cur.push(c);
                if c == '"' {
                    state = State::Normal;
                }
            }
            State::Backtick => {
                cur.push(c);
                if c == '`' {
                    state = State::Normal;
                }
            }
            State::LineComment => {
                if c == '\n' {
                    state = State::Normal;
                    cur.push('\n');
                }
            }
            State::BlockComment => {
                if c == '*' && chars.get(i + 1) == Some(&'/') {
                    state = State::Normal;
                    i += 1;
                }
            }
        }
        i += 1;
    }

    let trimmed = cur.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    out
}

/// 去掉注释后的文本（注释以空白替代，保留换行）。
pub fn strip_comments(sql: &str) -> String {
    split_statements(sql).join("\n")
}

/// 取首个关键字（大写）。会跳过前置括号与注释。
pub fn first_keyword(sql: &str) -> Option<String> {
    let cleaned = strip_comments(sql);
    for raw in cleaned.split_whitespace() {
        let token = raw.trim_start_matches('(');
        if token.is_empty() {
            continue;
        }
        let word: String = token
            .chars()
            .take_while(|c| c.is_ascii_alphabetic() || *c == '_')
            .collect::<String>()
            .to_ascii_uppercase();
        if !word.is_empty() {
            return Some(word);
        }
    }
    None
}

/// 判断单条语句的类型。
pub fn classify(sql: &str) -> StatementKind {
    let upper = strip_comments(sql).to_ascii_uppercase();
    let kw = match first_keyword(sql) {
        Some(k) => k,
        None => return StatementKind::Unknown,
    };

    match kw.as_str() {
        "SELECT" | "VALUES" | "TABLE" | "SHOW" | "DESC" | "DESCRIBE" => StatementKind::Read,
        "EXPLAIN" => {
            // EXPLAIN ANALYZE 会真的执行语句 → 按非只读处理
            if upper.contains("ANALYZE") {
                StatementKind::Write
            } else {
                StatementKind::Read
            }
        }
        "PRAGMA" => {
            // PRAGMA x = y 是写；PRAGMA x 是读
            if upper.contains('=') {
                StatementKind::Write
            } else {
                StatementKind::Read
            }
        }
        "WITH" => {
            // CTE 后面仍可能是写语句：WITH t AS (...) INSERT/UPDATE/DELETE ...
            if contains_write_verb(&upper) {
                StatementKind::Write
            } else {
                StatementKind::Read
            }
        }
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "REPLACE" | "UPSERT" | "COPY" | "LOAD" | "REFRESH"
        | "TRUNCATE" => StatementKind::Write,
        "CREATE" | "ALTER" | "DROP" | "RENAME" | "COMMENT" => StatementKind::Ddl,
        "GRANT" | "REVOKE" => StatementKind::Ddl,
        _ => StatementKind::Unknown,
    }
}

fn contains_write_verb(upper_sql: &str) -> bool {
    const VERBS: [&str; 6] = [" INSERT", " UPDATE", " DELETE", " MERGE", " REPLACE", " UPSERT"];
    VERBS.iter().any(|v| upper_sql.contains(v))
}

/// 整段 SQL 是否只读：**必须**是单条、且该条为 Read。
pub fn is_read_only(sql: &str) -> bool {
    let stmts = split_statements(sql);
    !stmts.is_empty() && stmts.iter().all(|s| classify(s).is_read_only())
}

/// 是否为「多条语句」（安全策略据此拒绝整段执行）。
pub fn statement_count(sql: &str) -> usize {
    split_statements(sql).len()
}

// ---------------------------------------------------------------- 结果来源

/// 词法单元：只保留「判断结果来源」需要的区分度。
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// 标识符或关键字（未加引号）
    Word(String),
    /// 引号包裹的标识符（`"x"` / `` `x` `` / `[x]`），值为去引号后的内容
    Quoted(String),
    Star,
    Comma,
    Dot,
    LParen,
    RParen,
    /// 其它符号（运算符、数字字面量、字符串字面量…）：只需知道存在
    Other,
}

fn eq_word(word: &str, expect: &str) -> bool {
    word.eq_ignore_ascii_case(expect)
}

/// 切词：注释直接丢弃，字符串字面量折叠成一个符号（值不参与来源判断）。
fn tokenize(sql: &str) -> Vec<Tok> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];

        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == '\'' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(Tok::Other);
            continue;
        }
        if c == '"' || c == '`' {
            let quote = c;
            i += 1;
            let mut name = String::new();
            while i < chars.len() {
                if chars[i] == quote {
                    if chars.get(i + 1) == Some(&quote) {
                        name.push(quote);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                name.push(chars[i]);
                i += 1;
            }
            out.push(Tok::Quoted(name));
            continue;
        }
        if c == '[' {
            i += 1;
            let mut name = String::new();
            while i < chars.len() && chars[i] != ']' {
                name.push(chars[i]);
                i += 1;
            }
            i += 1;
            out.push(Tok::Quoted(name));
            continue;
        }

        match c {
            '*' => {
                out.push(Tok::Star);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            '.' => {
                out.push(Tok::Dot);
                i += 1;
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            _ if c.is_ascii_alphabetic() || c == '_' || !c.is_ascii() => {
                let mut word = String::new();
                while i < chars.len() {
                    let ch = chars[i];
                    if ch.is_alphanumeric() || ch == '_' || ch == '$' || ch == '#' || !ch.is_ascii() {
                        word.push(ch);
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(Tok::Word(word));
            }
            _ if c.is_ascii_digit() => {
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                    i += 1;
                }
                out.push(Tok::Other);
            }
            _ => {
                out.push(Tok::Other);
                i += 1;
            }
        }
    }
    out
}

/// 这些关键字一出现就说明「结果不是某张表的原始行」。
const MULTI_SOURCE_WORDS: [&str; 12] = [
    "join",
    "inner",
    "left",
    "right",
    "full",
    "cross",
    "natural",
    "union",
    "intersect",
    "except",
    "group",
    "having",
];

/// 结果是否**就是某张表的原始行**；是则返回表名。
///
/// 这是「就地编辑」的前提条件，所以判定刻意严：
/// - 单条语句、单表（无 JOIN / 逗号多表 / UNION）；
/// - `FROM` 后是裸表名（派生表、`schema.table` 一律不接受 —— 我们的列元数据按裸表名取，
///   无法确认是不是同一张表）；
/// - SELECT 列表只能是 `*` 或裸列名（不允许表达式、函数、`AS` 别名）——
///   否则结果里的值**不等于**表里那一列的值，照着它生成 UPDATE 会改错行；
/// - 不允许 GROUP BY / HAVING（聚合结果不是行）。
///
/// 任何一条不满足都返回 None：宁可少一个功能，不可多一次改错数据。
pub fn single_table(sql: &str) -> Option<String> {
    let statements = split_statements(sql);
    if statements.len() != 1 {
        return None;
    }
    let toks = tokenize(&statements[0]);
    // WITH / SHOW / EXPLAIN 的结果都不是「某张表的行」
    if !matches!(toks.first(), Some(Tok::Word(w)) if eq_word(w, "select")) {
        return None;
    }

    // 每个词法单元所处的括号深度：只有深度 0 的关键字才是这条 SELECT 自己的
    let mut depths = Vec::with_capacity(toks.len());
    let mut depth = 0i32;
    for tok in &toks {
        depths.push(depth);
        match tok {
            Tok::LParen => depth += 1,
            Tok::RParen => depth = (depth - 1).max(0),
            _ => {}
        }
    }

    let from_idx = (1..toks.len())
        .find(|i| depths[*i] == 0 && matches!(&toks[*i], Tok::Word(w) if eq_word(w, "from")))?;

    // SELECT 列表：`*` 独占，或若干「裸列名 / 表名.列名」
    let mut item_start = 1usize;
    let mut items = 0usize;
    let mut seen_star = false;
    for i in 1..=from_idx {
        let at_end = i == from_idx;
        let is_separator = !at_end && depths[i] == 0 && toks[i] == Tok::Comma;
        if !at_end && !is_separator {
            continue;
        }
        let item = &toks[item_start..i];
        items += 1;
        match item {
            [Tok::Star] => seen_star = true,
            [Tok::Word(_)] | [Tok::Quoted(_)] => {}
            [Tok::Word(_) | Tok::Quoted(_), Tok::Dot, Tok::Word(_) | Tok::Quoted(_)] => {}
            // 函数、表达式、`AS` 别名、窗口函数……都落到这里
            _ => return None,
        }
        item_start = i + 1;
    }
    if items == 0 || (seen_star && items != 1) {
        return None;
    }

    let name = match toks.get(from_idx + 1) {
        Some(Tok::Word(w)) => w.clone(),
        Some(Tok::Quoted(w)) => w.clone(),
        // `FROM (` = 派生表，或 FROM 后面不是标识符
        _ => return None,
    };
    if matches!(toks.get(from_idx + 2), Some(Tok::Dot)) {
        return None;
    }

    // FROM 之后（深度 0）不允许再出现多来源/聚合关键字
    for (i, tok) in toks.iter().enumerate().skip(from_idx + 2) {
        if depths[i] != 0 {
            continue;
        }
        match tok {
            Tok::Comma => return None,
            Tok::Word(w) if MULTI_SOURCE_WORDS.iter().any(|f| eq_word(w, f)) => return None,
            _ => {}
        }
    }

    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 按分号拆分并保留引号内的分号() {
        assert_eq!(
            split_statements("select 1; select 2"),
            vec!["select 1", "select 2"]
        );
        assert_eq!(split_statements("select ';'"), vec!["select ';'"]);
        assert_eq!(split_statements("select 1;"), vec!["select 1"]);
        assert!(split_statements("   \n  ; ;  ").is_empty());
    }

    #[test]
    fn 注释被剥离且不参与拆分() {
        assert_eq!(split_statements("select 1 -- ; not a split"), vec!["select 1"]);
        assert_eq!(split_statements("select 1 /* ; */ ; select 2").len(), 2);
        assert_eq!(
            first_keyword("-- lead\n/* x */ SELECT 1"),
            Some("SELECT".to_string())
        );
    }

    #[test]
    fn 转义单引号不结束字符串() {
        assert_eq!(
            split_statements("select 'it''s; fine'"),
            vec!["select 'it''s; fine'"]
        );
    }

    #[test]
    fn 只读识别() {
        assert!(is_read_only("select * from t"));
        assert!(is_read_only("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(is_read_only("SHOW TABLES"));
        assert!(is_read_only("EXPLAIN SELECT 1"));
        assert!(!is_read_only("EXPLAIN ANALYZE SELECT 1"));
        assert!(!is_read_only("select 1; drop table t"));
        assert!(!is_read_only("WITH x AS (SELECT 1) DELETE FROM t"));
        assert!(!is_read_only("update t set a = 1"));
        assert!(!is_read_only(""));
        assert!(!is_read_only("SET search_path = public"));
    }

    #[test]
    fn 语句分类() {
        assert_eq!(classify("create table t(a int)"), StatementKind::Ddl);
        assert_eq!(classify("truncate table t"), StatementKind::Write);
        assert_eq!(classify("pragma table_info(t)"), StatementKind::Read);
        assert_eq!(classify("pragma journal_mode = wal"), StatementKind::Write);
        assert_eq!(classify("call do_something()"), StatementKind::Unknown);
    }

    #[test]
    fn 只有单表裸行才给出来源对象() {
        // 可以就地编辑的形态
        let ok = |sql: &str, expect: &str| {
            assert_eq!(single_table(sql), Some(expect.to_string()), "应认出来源：{sql}");
        };
        ok("select * from users", "users");
        ok("select id, name from users where id = 1", "users");
        ok("SELECT u.id FROM users u ORDER BY id LIMIT 10", "users");
        ok("select * from \"users\"", "users");
        ok("select * from `users`", "users");
        ok("select * from [users]", "users");
        ok("select /* 注释 */ * from users -- 尾巴", "users");
        // 子查询在 WHERE 里不影响「这行来自 t」
        ok("select * from users where id in (select id from t2)", "users");
        ok("select * from 订单", "订单");

        // 判不出来的形态：一律 None
        let none = |sql: &str| {
            assert_eq!(single_table(sql), None, "不该认出来源：{sql}");
        };
        none("select 1 as hello");
        none("select count(*) from users");
        none("select upper(name) from users");
        // 别名会把结果列名改掉，值的语义也就不确定了
        none("select name as n from users");
        none("select * from users, orders");
        none("select * from users u join orders o on o.uid = u.id");
        none("select * from users union select * from orders");
        none("select * from (select 1) x");
        none("select * from main.users");
        none("select name, count(*) from users group by name");
        none("with c as (select 1) select * from c");
        none("select * from a; select * from b");
        none("show tables");
        none("update users set name = 'x'");
        none("");
    }

    // ---------- 语句块（BEGIN ... END）不能从中间切开 ----------

    /// 「编辑函数」生成的脚本必须只切成 2 条：`DROP` 与整个 `CREATE`。
    ///
    /// 这是踩过一次事故的用例：以前不认 `BEGIN ... END`，同样这段被切成 3 条 ——
    /// `DROP FUNCTION` 执行成功（函数真的被删），
    /// `CREATE FUNCTION ... BEGIN RETURN ( ... )` 断在半截必然语法错误，
    /// 最后那条 `END` 还没轮到 ⇒ **对象被删且建不回来**（实测丢了 `dify.f1`）。
    #[test]
    fn begin_end_block_keeps_one_statement() {
        let sql = "DROP FUNCTION IF EXISTS `dify`.`f1`;\n\n\
                   CREATE FUNCTION `f1`() RETURNS float BEGIN\n\
                   RETURN ( SELECT count(1) FROM boxoffice WHERE id = 1 );\n\
                   END";
        let parts = split_statements(sql);
        assert_eq!(parts.len(), 2, "切成了 {} 条：{parts:#?}", parts.len());
        assert!(parts[1].contains("RETURN ( SELECT count(1)"));
        assert!(parts[1].trim_end().ends_with("END"));
    }

    /// 过程体里的多条语句、以及体里的 `CASE ... END` 表达式，都留在同一条里。
    #[test]
    fn procedure_body_statements_stay_together() {
        let sql = "CREATE PROCEDURE `p1`() BEGIN \
                   SELECT CASE WHEN 1 = 1 THEN 'A' ELSE 'B' END AS c; \
                   INSERT INTO t VALUES (1); \
                   END";
        assert_eq!(split_statements(sql).len(), 1);
    }

    /// `IF ... END IF;` / `LOOP ... END LOOP;` 收的是另一层块，不能让深度提前归零。
    #[test]
    fn inner_if_block_does_not_close_outer_block() {
        let sql = "CREATE PROCEDURE `p2`() BEGIN \
                   IF x THEN SELECT 1; END IF; \
                   SELECT 2; \
                   END";
        assert_eq!(split_statements(sql).len(), 1);
    }

    /// `BEGIN` 作为事务起点时不是语句块，否则后面所有语句会被吞成一条。
    #[test]
    fn begin_transaction_is_not_a_block() {
        assert_eq!(
            split_statements("BEGIN;\nUPDATE t SET a = 1;\nCOMMIT;").len(),
            3
        );
        assert_eq!(split_statements("BEGIN WORK;\nSELECT 1;").len(), 2);
        assert_eq!(split_statements("BEGIN TRANSACTION;\nSELECT 1;").len(), 2);
    }

    /// 块外的 `CASE ... END` 表达式要能配平回 0，不影响后续切分。
    #[test]
    fn standalone_case_expression_balances() {
        assert_eq!(
            split_statements("SELECT CASE WHEN a THEN 1 END; SELECT 2;").len(),
            2
        );
    }

    /// 老行为不能退化：引号/注释里的分号照旧不算分隔符。
    #[test]
    fn quotes_and_comments_still_do_not_split() {
        assert_eq!(split_statements("INSERT INTO t VALUES ('a;b');").len(), 1);
        assert_eq!(split_statements("SELECT 1; -- ;\nSELECT 2;").len(), 2);
        assert_eq!(split_statements("SELECT /* ; */ 1;").len(), 1);
    }
}
