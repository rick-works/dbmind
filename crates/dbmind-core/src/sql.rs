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
    /// PostgreSQL 美元引号（`$$` / `$tag$`）：体内的 `;` 不是分隔符。
    /// PG 的函数体（`AS $$ … $$`）靠它才不会被切碎。
    Dollar,
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

/// 能**开启一条新语句**的关键字。
///
/// 只在「行首 + 上一条看起来已经写完」时才真的切（见 `implicit_break`），
/// 所以把它们列全一点是安全的；真正决定切不切的是上下文判定。
const IMPLICIT_HEADS: &[&str] = &[
    "SELECT", "WITH", "INSERT", "REPLACE", "UPDATE", "DELETE", "MERGE", "CREATE", "ALTER", "DROP",
    "TRUNCATE", "RENAME", "COMMENT", "GRANT", "REVOKE", "CALL", "EXEC", "EXECUTE", "SHOW", "DESC",
    "DESCRIBE", "EXPLAIN", "ANALYZE", "VACUUM", "OPTIMIZE", "USE", "SET", "COMMIT", "ROLLBACK",
    "START", "SAVEPOINT", "RELEASE", "PREPARE", "DEALLOCATE",
];

/// 行首出现这些词时，它们其实是**上一条语句的延续**（`UNION SELECT`、`AND x`、`FROM t`…），
/// 见到就绝不切 —— 误切会把一条完整语句拆成两条，比不切危险得多。
const IMPLICIT_CONTINUE: &[&str] = &[
    "UNION", "ALL", "INTERSECT", "EXCEPT", "MINUS", "AS", "INTO", "VALUES", "FROM", "JOIN", "ON",
    "USING", "WHERE", "AND", "OR", "NOT", "IN", "EXISTS", "BETWEEN", "LIKE", "IS", "THEN", "ELSE",
    "WHEN", "CASE", "HAVING", "GROUP", "ORDER", "BY", "LIMIT", "OFFSET", "RETURNING", "DO", "WITH",
];

/// 「没写分号也分段」的判定：`word` 是行首刚读完的词，其余是当前语句的上下文。
///
/// 保守原则：**拿不准就不切**。宁可把两条并成一条报语法错（用户加个分号就好），
/// 也不要把一条拆成两条分开执行 —— 后者会执行半个语句，甚至删掉一半数据。
fn implicit_break(
    word: &str,
    cur: &str,
    paren: i32,
    depth: i32,
    last_code: Option<char>,
    last_word: &str,
    head_word: &str,
    saw_select: bool,
    routine: bool,
    main_started: bool,
    attach_select: bool,
    next_word: &str,
) -> bool {
    if cur.trim().is_empty() || paren != 0 || depth != 0 {
        return false;
    }
    // 例程/触发器 DDL：`CREATE TRIGGER … BEFORE INSERT`、`CREATE PROCEDURE …` 里的
    // INSERT / SET / SELECT 都是语法的一部分，绝不能当成新语句开头。
    if routine {
        return false;
    }
    if !IMPLICIT_HEADS.contains(&word) {
        return false;
    }
    // 上一条语句的最后一个字符必须「像个结尾」：字/数字/右括号/闭合引号。
    // 逗号、运算符、左括号、点号结尾说明话没说完（`SELECT a,` 后面接 `FROM` 之类）。
    let tail_ok = match last_code {
        Some(c) => {
            c.is_alphanumeric() || c == '_' || c == '$' || c == ')' || c == ']' || c == '\'' || c == '"' || c == '`'
        }
        None => false,
    };
    if !tail_ok {
        return false;
    }
    let prev = last_word.to_ascii_uppercase();
    if IMPLICIT_CONTINUE.contains(&prev.as_str()) {
        return false;
    }
    let head = head_word.to_ascii_uppercase();
    // 匿名块（BEGIN … END）里不猜，交给分号
    if head == "BEGIN" {
        return false;
    }
    // `INSERT … SELECT` / `CREATE TABLE … AS SELECT` 是**一条**语句：
    // 主查询还没开始时那个 SELECT 不算新语句；已经出现过顶层 SELECT 后再顶格起 SELECT 就是新语句。
    if word == "SELECT"
        && !saw_select
        && matches!(head.as_str(), "INSERT" | "REPLACE" | "MERGE" | "CREATE" | "EXPLAIN")
    {
        return false;
    }
    // CTE：`WITH cte AS (…)` 之后的主语句可以是 SELECT / INSERT / UPDATE / DELETE …
    //   - 主语句还没开始时，顶格出现的 DML/SELECT 都属于这条 WITH 语句；
    //   - 主语句是 INSERT/REPLACE/MERGE 时，紧跟的那个 SELECT 也属于它（INSERT … SELECT）；
    //   - 其余情况再顶格 SELECT = 新语句。
    if head == "WITH" {
        if !main_started && matches!(word, "INSERT" | "REPLACE" | "MERGE" | "UPDATE" | "DELETE") {
            return false;
        }
        if !main_started && word == "SELECT" {
            return false;
        }
        if main_started && attach_select && word == "SELECT" {
            return false;
        }
    }
    // `UPDATE t SET …` / `INSERT … SET …` / `CREATE TRIGGER … SET …` 里的 SET 属于上一条
    if word == "SET" && matches!(head.as_str(), "UPDATE" | "INSERT" | "CREATE") {
        return false;
    }
    // `COMMENT` 只有后面跟 `ON` 时才是**新语句**（PG / Oracle 的 `COMMENT ON TABLE … IS …`）。
    // ClickHouse 建表把表注释写在最后一行（`… \nCOMMENT '说明'`），MySQL 的表选项是
    // `COMMENT='…'` —— 这两处都属于上一条 CREATE TABLE，误切会把 `COMMENT '说明'`
    // 单独发出去，ClickHouse 报 `failed at position 1 ('COMMENT')`（真机踩过）。
    if word == "COMMENT" && next_word != "ON" {
        return false;
    }
    true
}

/// 跳过一个从 `start` 开始的括号组，返回右括号之后的 token 下标。
fn skip_paren_group(toks: &[Tok], start: usize) -> Option<usize> {
    if !matches!(toks.get(start), Some(Tok::LParen)) {
        return None;
    }
    let mut depth = 0i32;
    for (index, token) in toks.iter().enumerate().skip(start) {
        match token {
            Tok::LParen => depth += 1,
            Tok::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// 找 `WITH ...` 的主语句关键字。
///
/// CTE 定义里的 `SELECT` / `INSERT` 都包在括号里，不能拿它们当主语句；
/// 这里只跳过顶层括号组，返回 CTE 列表之后的第一个词法关键字。
///
/// 解析不出来返回 `None`，两个调用方的处理**方向不同**，别混：
/// - `classify` 落到 `StatementKind::Unknown` ⇒ 只读连接上照样被拦（保守）；
/// - `danger_reason` 直接返回 `None` ⇒ 只是不做「无 WHERE 全表更新」这类提醒，
///   真正的只读闸门仍由 `classify` 把着。
fn with_main_keyword(sql: &str) -> Option<String> {
    let toks = tokenize(sql);
    if !matches!(toks.first(), Some(Tok::Word(word)) if eq_word(word, "with")) {
        return None;
    }

    let mut i = 1usize;
    if matches!(toks.get(i), Some(Tok::Word(word)) if eq_word(word, "recursive")) {
        i += 1;
    }

    loop {
        match toks.get(i) {
            Some(Tok::Word(_)) | Some(Tok::Quoted(_)) => i += 1,
            _ => return None,
        }
        if matches!(toks.get(i), Some(Tok::LParen)) {
            i = skip_paren_group(&toks, i)?;
        }
        if !matches!(toks.get(i), Some(Tok::Word(word)) if eq_word(word, "as")) {
            return None;
        }
        i += 1;
        if matches!(toks.get(i), Some(Tok::Word(word)) if eq_word(word, "not")) {
            i += 1;
        }
        if matches!(toks.get(i), Some(Tok::Word(word)) if eq_word(word, "materialized")) {
            i += 1;
        }
        i = skip_paren_group(&toks, i)?;

        if matches!(toks.get(i), Some(Tok::Comma)) {
            i += 1;
            continue;
        }
        return match toks.get(i) {
            Some(Tok::Word(word)) => Some(word.to_ascii_uppercase()),
            _ => None,
        };
    }
}

/// `WITH` 的 CTE 前缀之后，主语句从哪个字节开始。
fn main_statement_start(sql: &str, main_keyword: &str) -> Option<usize> {
    top_level_keyword_pos(sql, main_keyword)
}

/// 按 `;` 拆分语句，**正确处理引号、注释与语句块**。
///
/// 三类 `;` 不算分隔符：`';'`（引号内）、`-- ;` / `/* ; */`（注释内）、
/// 以及 `BEGIN ... END` / `CASE ... END` **块内部**的（见下）。
///
/// 另外支持**不写分号也分段**：行首出现 `SELECT` / `INSERT` / `CREATE` … 这类语句起始关键字、
/// 且上一条在语法上已经收尾（括号与块都闭合、末尾不是逗号/运算符）时，自动在那里断开。
/// 判定见 [`implicit_break`]，前端 `utils/sqlSplit.js` 必须与本函数**同一套规则**，
/// 否则编辑器显示「2 条语句」而执行时后端只按分号切成 1 条。
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
    // ---- 无分号分段用的上下文 ----
    let mut paren: i32 = 0;           // 括号深度：子查询里的 SELECT 不能切
    let mut line_start = true;        // 当前是否在「行首」（只见过空白）
    let mut last_word = String::new(); // 上一条语句的最后一个词（判 UNION/AND 等续接词）
    let mut head_word = String::new(); // 当前语句的第一个词（判 INSERT…SELECT 等）
    let mut last_code: Option<char> = None; // 最后一个非空白字符（判语句是否收尾）
    let mut word_at_line_start = false;     // 正在读的这个词是否位于行首
    // ---- 例程 / 触发器 DDL 与 CTE 的进度（前端 sqlSplit.js 同名变量一一对应）----
    let mut routine = false;      // CREATE PROCEDURE/FUNCTION/TRIGGER/PACKAGE：整条是一条语句
    let mut decl = false;         // Oracle 声明段（AS/IS 之后、BEGIN 之前）：段里的 `;` 不是边界
    let mut begin_seen = false;   // 例程体已经开始
    let mut dollar_body = false;  // 体是美元引号字面量（`AS $$ … $$`）
    let mut saw_select = false;   // 当前语句已出现过顶层 SELECT
    let mut main_started = false; // WITH 的主语句已开始
    let mut attach_select = false;// 主语句是 INSERT/REPLACE/MERGE，紧跟的 SELECT 仍属于它
    // 当前正在读的单词：识别 BEGIN / END / CASE 用
    let mut word = String::new();
    // 美元引号的结束标记（`$$` / `$tag$`）
    let mut dollar_tag = String::new();
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];
        match state {
            State::Normal => {
                // 美元引号 `$$` / `$tag$`：必须排在「词」之前 —— `$` 也是词字符，
                // 放到后面会被当成普通单词吃掉，函数体里的分号就全都暴露出来把函数切碎。
                if c == '$' && word.is_empty() {
                    let mut j = i + 1;
                    while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j] == '$' {
                        dollar_tag = chars[i..=j].iter().collect();
                        cur.push_str(&dollar_tag);
                        last_code = Some('$');
                        last_word.clear();
                        state = State::Dollar;
                        i = j + 1;
                        continue;
                    }
                }
                if is_word_char(c) {
                    if word.is_empty() {
                        word_at_line_start = line_start;
                    }
                    word.push(c);
                    line_start = false;
                    i += 1;
                    continue;
                }
                if !word.is_empty() {
                    let upper = word.to_ascii_uppercase();
                    // 先判「这里能不能断」：此时 cur 里还没有这个词，正好干净
                    if word_at_line_start
                        && implicit_break(
                            &upper, &cur, paren, depth, last_code, &last_word, &head_word,
                            saw_select, routine, main_started, attach_select, &next_word(&chars, i),
                        )
                    {
                        let trimmed = cur.trim();
                        if !trimmed.is_empty() {
                            out.push(trimmed.to_string());
                        }
                        cur.clear();
                        depth = 0;
                        paren = 0;
                        head_word.clear();
                        last_word.clear();
                        saw_select = false;
                        routine = false;
                        decl = false;
                        begin_seen = false;
                        dollar_body = false;
                        main_started = false;
                        attach_select = false;
                        // last_code 不用清：紧接着就会被这个词的末字符覆盖
                    }
                    // CREATE [OR REPLACE] PROCEDURE / FUNCTION / TRIGGER / PACKAGE：整条是一条语句
                    if matches!(upper.as_str(), "PROCEDURE" | "FUNCTION" | "TRIGGER" | "PACKAGE")
                        && matches!(head_word.as_str(), "CREATE" | "ALTER")
                    {
                        routine = true;
                    }
                    // Oracle 的声明段：AS/IS 之后、BEGIN 之前（`v NUMBER := 0;` 这些分号不是语句边界）
                    if routine && depth == 0 && matches!(upper.as_str(), "AS" | "IS") {
                        decl = true;
                    }
                    match upper.as_str() {
                        // `BEGIN` 既可能开语句块，也可能是事务起点（`BEGIN;` / `BEGIN WORK` /
                        // `BEGIN TRANSACTION` / SQL Server 的 `BEGIN TRAN` / SQLite 的
                        // `BEGIN IMMEDIATE`）。后者没有配对的 END，误判会把后面所有语句吞成一条。
                        "BEGIN" => {
                            if !matches!(
                                next_word(&chars, i).as_str(),
                                "" | "WORK" | "TRAN" | "TRANSACTION" | "DEFERRED" | "IMMEDIATE"
                                    | "EXCLUSIVE" | "DISTRIBUTED"
                            )
                            {
                                depth += 1;
                                begin_seen = true;
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
                    // 顶层 SELECT 出现即为「主查询已开始」：此后顶格的 SELECT 就是新语句
                    if upper == "SELECT" && paren == 0 {
                        saw_select = true;
                    }
                    // CTE 主语句的进度（只认顶层词，CTE 体内的 SELECT 不算）
                    if paren == 0 && depth == 0 && head_word == "WITH" {
                        if !main_started {
                            if matches!(upper.as_str(), "INSERT" | "REPLACE" | "MERGE") {
                                main_started = true;
                                attach_select = true;
                            } else if matches!(upper.as_str(), "SELECT" | "UPDATE" | "DELETE") {
                                main_started = true;
                            }
                        } else if attach_select && upper == "SELECT" {
                            attach_select = false;
                        }
                    }
                    if head_word.is_empty() {
                        head_word = upper.clone();
                    }
                    last_word = upper;
                    last_code = word.chars().last();
                    word_at_line_start = false;
                    cur.push_str(&word);
                    word.clear();
                }
                if c == '\'' {
                    state = State::SingleQuote;
                    cur.push(c);
                    last_code = Some(c);
                    line_start = false;
                } else if c == '#'
                    && line_start
                    && chars.get(i + 1).is_some_and(|next| next.is_whitespace())
                {
                    // 行首的 `# 注释`（MySQL/MariaDB）。`#temp` 后面是字母，
                    // 不能误吞；行中间的 `#` 也不处理，避免影响 PostgreSQL 位运算。
                    state = State::LineComment;
                    cur.push(' ');
                } else if c == '"' {
                    state = State::DoubleQuote;
                    cur.push(c);
                    last_code = Some(c);
                    line_start = false;
                } else if c == '`' {
                    state = State::Backtick;
                    cur.push(c);
                    last_code = Some(c);
                    line_start = false;
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
                    // 例程声明段里的分号（Oracle：`v NUMBER := 0;` 在 BEGIN 之前）不是语句结束符
                    let in_decl = routine && decl && !begin_seen && !dollar_body;
                    if depth == 0 && !in_decl {
                        let trimmed = cur.trim();
                        if !trimmed.is_empty() {
                            out.push(trimmed.to_string());
                        }
                        cur.clear();
                        paren = 0;
                        depth = 0;
                        head_word.clear();
                        last_word.clear();
                        last_code = None;
                        saw_select = false;
                        routine = false;
                        decl = false;
                        begin_seen = false;
                        dollar_body = false;
                        main_started = false;
                        attach_select = false;
                    } else {
                        // 块内/声明段：分号属于体内语句，原样留着
                        cur.push(c);
                        last_code = Some(c);
                    }
                } else if c == '(' {
                    paren += 1;
                    cur.push(c);
                    last_code = Some(c);
                    line_start = false;
                } else if c == ')' {
                    paren = (paren - 1).max(0);
                    cur.push(c);
                    last_code = Some(c);
                    line_start = false;
                } else if c == '\n' {
                    cur.push(c);
                    line_start = true;
                } else {
                    cur.push(c);
                    if !c.is_whitespace() {
                        last_code = Some(c);
                        line_start = false;
                    }
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
                        last_code = Some('\'');
                        // 字符串本身也是一个「词」：清掉 last_word，否则
                        // `... WHERE name LIKE 'a%'` 的下一条无分号语句会被 LIKE
                        // （续接词白名单）挡住，永远切不开。前端同一处理。
                        last_word.clear();
                    }
                }
            }
            State::DoubleQuote => {
                cur.push(c);
                if c == '"' {
                    // "" 是转义的双引号（PG/标准 SQL 的标识符写法），不结束
                    if chars.get(i + 1) == Some(&'"') {
                        cur.push('"');
                        i += 1;
                    } else {
                        state = State::Normal;
                        last_code = Some('"');
                        last_word.clear();
                    }
                }
            }
            State::Backtick => {
                cur.push(c);
                if c == '`' {
                    // `` 是转义的反引号（MySQL 标识符），不结束
                    if chars.get(i + 1) == Some(&'`') {
                        cur.push('`');
                        i += 1;
                    } else {
                        state = State::Normal;
                        last_code = Some('`');
                        last_word.clear();
                    }
                }
            }
            State::Dollar => {
                let tag: Vec<char> = dollar_tag.chars().collect();
                if chars.len() >= i + tag.len() && chars[i..i + tag.len()] == tag[..] {
                    cur.push_str(&dollar_tag);
                    i += tag.len() - 1;
                    state = State::Normal;
                    last_code = Some('$');
                    last_word.clear(); // 字面量本身就是一个「词」
                    // 例程体就是这段字面量 ⇒ 声明段结束，后面的 `;`（如 `LANGUAGE plpgsql;`）才是终止符
                    if routine {
                        decl = false;
                        dollar_body = true;
                    }
                } else {
                    cur.push(c);
                }
            }
            State::LineComment => {
                if c == '\n' {
                    state = State::Normal;
                    cur.push('\n');
                    line_start = true;
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

    // 末尾还没落地的词（文件以单词结尾，后面没有分隔符）
    if !word.is_empty() {
        cur.push_str(&word);
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

/// **顶层**（括号深度 0）按词边界找小写关键字，跳过字符串/标识符字面量。
/// 返回字节偏移。`[` 按引号处理（SQL Server 标识符）。
fn top_level_keyword_pos(sql: &str, word: &str) -> Option<usize> {
    let b = sql.as_bytes();
    let l = sql.to_ascii_lowercase();
    let w = word.to_ascii_lowercase();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' | b'`' | b'[' => {
                let close = if b[i] == b'[' { b']' } else { b[i] };
                i += 1;
                while i < b.len() && b[i] != close {
                    if b[i] == b'\\' {
                        i += 1; // 反斜杠转义：宁可多跳一格
                    }
                    i += 1;
                }
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {
                let word_start = i == 0 || !is_word_char(b[i - 1] as char);
                // `i` 是字节下标：中文/日文等非 ASCII 标识符会让它落在字符
                // 中间。先在字符边界上再切，否则 `l[i..]` 会直接 panic。
                if depth == 0 && word_start && l.is_char_boundary(i) && l[i..].starts_with(&w) {
                    let end = i + w.len();
                    if end >= b.len() || !is_word_char(b[end] as char) {
                        return Some(i);
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// 危险语句识别（设置页「危险语句拦截」的判定源）。
///
/// 判三类：**不带 WHERE 的全表 UPDATE/DELETE**、`TRUNCATE`、`DROP`。
/// 这是尽力而为的启发式（不是语法分析器）：认不出的形态一律返回 `None`
/// —— 方向是宁可放过交给其它闸门，不把正常语句错拦。
pub fn danger_reason(sql: &str) -> Option<&'static str> {
    let cleaned = strip_comments(sql);
    let first = first_keyword(&cleaned)?;
    let (keyword, body) = if first == "WITH" {
        let main = with_main_keyword(&cleaned)?;
        let start = main_statement_start(&cleaned, &main)?;
        (main, cleaned.get(start..)?)
    } else {
        (first, cleaned.as_str())
    };
    match keyword.as_str() {
        "TRUNCATE" => Some("这个操作会一次性清空整张表的数据"),
        "DROP" => Some("这个操作会删除整张表（连同里面的数据）"),
        "UPDATE" | "DELETE" => {
            if top_level_keyword_pos(body, "where").is_none() {
                Some("没有指定筛选范围，会作用于表中所有数据")
            } else {
                None
            }
        }
        _ => None,
    }
}

/// [`write_target`] 的结果：目标表（含别名，计数查询原样引用）+ 顶层 WHERE 条件。
pub struct WriteTarget {
    pub table: String,
    pub where_clause: Option<String>,
}

/// 从 `UPDATE` / `DELETE` 抽出「目标表 + WHERE 条件」，供影响行数预估生成
/// `SELECT COUNT(*) FROM {table} [WHERE …]`。
///
/// 只认单表形态：带顶层 `JOIN` / `FROM` / `USING` 的多表写法（MySQL/SQL Server
/// 的多表 DELETE、SQL Server 的 UPDATE…FROM）返回 `None` —— 预估本来就是
/// 尽力而为，解析不了就跳过检查，绝不硬编一个错误的 COUNT。
pub fn write_target(sql: &str) -> Option<WriteTarget> {
    let cleaned = strip_comments(sql);
    let cleaned = cleaned.trim().trim_end_matches(';').trim();
    let first = first_keyword(cleaned)?;
    let (kw, body_start) = if first == "WITH" {
        let main = with_main_keyword(cleaned)?;
        let start = main_statement_start(cleaned, &main)?;
        let end = start + main.len();
        (main, end)
    } else {
        let start = main_statement_start(cleaned, &first)?;
        let end = start + first.len();
        (first, end)
    };
    let body = cleaned
        .get(body_start..)
        .map(str::trim_start)
        .unwrap_or_default();
    match kw.as_str() {
        "UPDATE" => {
            let set_at = top_level_keyword_pos(body, "set")?;
            let table = body.get(..set_at)?.trim();
            if table.is_empty() || has_table_join_keyword(table) {
                return None;
            }
            let where_clause = top_level_keyword_pos(&body[set_at..], "where")
                .and_then(|w| body.get(set_at + w + "where".len()..))
                .map(str::trim)
                .filter(|w| !w.is_empty());
            Some(WriteTarget { table: table.to_string(), where_clause: where_clause.map(str::to_string) })
        }
        "DELETE" => {
            // 只认 `DELETE FROM 表`：表名后面还有第二个顶层 FROM/USING ⇒ 多表形态，放弃
            if !body[..].to_ascii_lowercase().starts_with("from") {
                return None;
            }
            let rest = body.get("from".len()..).map(str::trim_start)?;
            let where_at = top_level_keyword_pos(rest, "where");
            let table = match where_at {
                Some(w) => rest.get(..w)?.trim(),
                None => rest.trim(),
            };
            if table.is_empty() || has_table_join_keyword(table) {
                return None;
            }
            let where_clause = where_at
                .and_then(|w| rest.get(w + "where".len()..))
                .map(str::trim)
                .filter(|w| !w.is_empty());
            Some(WriteTarget { table: table.to_string(), where_clause: where_clause.map(str::to_string) })
        }
        _ => None,
    }
}

/// 表引用片段里是否出现了多表写法的关键字（有就是解析不了的复合形态）。
fn has_table_join_keyword(table: &str) -> bool {
    for kw in ["join", "from", "using", ","] {
        if top_level_keyword_pos(table, kw).is_some() {
            return true;
        }
    }
    false
}

/// `SELECT` 里出现这些函数调用 ⇒ **不是只读**：它们会改服务端状态或终止别人的会话。
///
/// 为什么要单列：`SELECT` 整体按只读放行，而 PG/Kingbase **没有 `KILL` 语句** ——
/// 界面上的「终止会话」就是 `select pg_terminate_backend(pid)`。不拦的话，
/// 只读连接上照样能把别人的会话杀掉；而 MySQL / SQL Server 用的是 `KILL N`
/// （首关键字 `KILL` ⇒ Unknown ⇒ 只读连接上被拦住），两边行为必须一致。
const SIDE_EFFECT_CALLS: &[&str] = &[
    "PG_TERMINATE_BACKEND",
    "PG_CANCEL_BACKEND",
    "NEXTVAL",
    "SETVAL",
    "LO_IMPORT",
    "LO_EXPORT",
    "DBLINK_EXEC",
];

/// 不带等号时**确实只读取**的 pragma（`PRAGMA x;` 是查它当前的值）。
const READ_ONLY_PRAGMAS: &[&str] = &[
    "TABLE_INFO",
    "TABLE_XINFO",
    "TABLE_LIST",
    "DATABASE_LIST",
    "INDEX_LIST",
    "INDEX_XINFO",
    "INDEX_INFO",
    "FOREIGN_KEY_LIST",
    "FOREIGN_KEY_CHECK",
    "INTEGRITY_CHECK",
    "QUICK_CHECK",
    "FREELIST_COUNT",
    "PAGE_COUNT",
    "PAGE_SIZE",
    "ENCODING",
    "SCHEMA_VERSION",
    "USER_VERSION",
    "DATA_VERSION",
    "COMPILE_OPTIONS",
    "COLLATION_LIST",
    "FUNCTION_LIST",
    "PRAGMA_LIST",
    "MODULE_LIST",
    "CACHE_SIZE",
];

/// 已知**只读**的系统 / 诊断过程（`EXEC sp_who2`、`sp_helpdb`、`sp_lock` …）。
///
/// 只列「确定不会改数据」的：`xp_cmdshell`（跑系统命令）、`sp_configure`（改服务器配置）、
/// `sp_executesql` / `sp_execute`（可执行任意动态 SQL）一律**不放行**，仍按「无法归类」处理。
///
/// 为什么需要这张表：生产保护 / 只读连接把「无法归类」当写拦，
/// 于是生产库上连 `sp_who2` 看会话都会被拒 —— 排障就断了（真机反馈）。
const READ_ONLY_PROCEDURES: &[&str] = &[
    "SP_WHO", "SP_WHO2", "SP_WHOISACTIVE", "SP_LOCK", "SP_HELPDB", "SP_HELPTEXT",
    "SP_HELPINDEX", "SP_HELPCONSTRAINT", "SP_HELPSERVER", "SP_HELPUSER", "SP_HELPGROUP",
    "SP_HELPPROTECT", "SP_HELPDBACCESS", "SP_HELP", "SP_COLUMNS", "SP_TABLES",
    "SP_SPACEUSED", "SP_DATABASES", "SP_SERVER_INFO", "SP_STATISTICS", "SP_PKEYS",
    "SP_FKEYS", "SP_STORED_PROCEDURES", "SP_DESCRIBE_UNDECLARED_PARAMETERS",
];

/// 只读的 `DBCC` 子命令（查日志空间、看统计信息、看连接在跑什么…）。
const READ_ONLY_DBCC: &[&str] = &[
    "SQLPERF", "SHOW_STATISTICS", "INPUTBUFFER", "OUTPUTBUFFER", "OPENTRAN", "PROCCACHE",
    "SHOWCONTIG", "CHECKDB", "CHECKTABLE", "CHECKALLOC", "CHECKCATALOG", "CHECKFILEGROUP",
    "CHECKIDENT", "DBINFO", "USEROPTIONS", "TRACESTATUS", "ERRLOG", "SHOWFILESTATS",
];

/// `DBCC` 里**会动服务器状态 / 数据文件**的子命令：清计划缓存、收缩文件、写页 —— 必须继续拦。
const WRITE_DBCC: &[&str] = &[
    "FREEPROCCACHE", "DROPCLEANBUFFERS", "SHRINK", "WRITEPAGE", "UPDATEUSAGE",
    "INDEXDEFRAG", "DBREINDEX", "REINDEX", "CLEARTABLE",
];

/// `EXEC <proc>` / `CALL <proc>` 是否落在只读的系统过程白名单里。
fn read_only_procedure(upper: &str) -> bool {
    READ_ONLY_PROCEDURES.iter().any(|name| upper.contains(name))
}

/// `DBCC <sub>` 归类：只读子命令 → 读；会动服务器状态 / 数据文件的 → 无法归类（照旧拦）。
fn dbcc_kind(upper: &str) -> StatementKind {
    if WRITE_DBCC.iter().any(|word| upper.contains(word)) {
        return StatementKind::Unknown;
    }
    if READ_ONLY_DBCC.iter().any(|word| upper.contains(word)) {
        StatementKind::Read
    } else {
        StatementKind::Unknown
    }
}

/// 判断单条语句的类型。
pub fn classify(sql: &str) -> StatementKind {
    let cleaned = strip_comments(sql);
    let upper = cleaned.to_ascii_uppercase();
    let kw = match first_keyword(sql) {
        Some(k) => k,
        None => return StatementKind::Unknown,
    };

    match kw.as_str() {
        "SELECT" => select_kind(&upper, &cleaned),
        "VALUES" | "TABLE" | "SHOW" | "DESC" | "DESCRIBE" => StatementKind::Read,
        "EXPLAIN" => {
            // EXPLAIN ANALYZE 会真的执行语句 → 按非只读处理
            if upper.contains("ANALYZE") {
                StatementKind::Write
            } else {
                StatementKind::Read
            }
        }
        "PRAGMA" => {
            // `PRAGMA x = y` 是写。不带等号时**只有明确只读的那几个**才是读 ——
            // 反过来的写法（「不含等号即读」）会把 `PRAGMA wal_checkpoint(TRUNCATE)`、
            // `PRAGMA optimize`、`PRAGMA shrink_memory` 这些**真的会动库文件**的判成读，
            // 只读连接上执行它们就是「开了只读还能改库」。
            // 认不出来的归 `Unknown`：只读连接拦下，可写连接照常放行（不误伤）。
            if upper.contains('=') {
                StatementKind::Write
            } else if READ_ONLY_PRAGMAS.iter().any(|name| upper.contains(name)) {
                StatementKind::Read
            } else {
                StatementKind::Unknown
            }
        }
        "WITH" => match with_main_keyword(sql).as_deref() {
            Some("SELECT") | Some("VALUES") | Some("TABLE") => select_kind(&upper, &cleaned),
            Some("INSERT") | Some("UPDATE") | Some("DELETE") | Some("MERGE")
            | Some("REPLACE") | Some("UPSERT") => StatementKind::Write,
            Some("CREATE") | Some("ALTER") | Some("DROP") | Some("RENAME") | Some("COMMENT")
            | Some("GRANT") | Some("REVOKE") => StatementKind::Ddl,
            // 解析不出来时保守按非只读处理，避免 CTE 写语句从只读闸门漏过去。
            _ => StatementKind::Unknown,
        },
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "REPLACE" | "UPSERT" | "COPY" | "LOAD" | "REFRESH"
        | "TRUNCATE" => StatementKind::Write,
        "CREATE" | "ALTER" | "DROP" | "RENAME" | "COMMENT" => StatementKind::Ddl,
        "GRANT" | "REVOKE" => StatementKind::Ddl,
        // 切库只改会话的默认库，不动任何数据。只读连接也必须放行 ——
        // SQL Server 的元数据 SQL 是「库内限定」的，内核会给它前置 `use [库];`；
        // 首条判成「无法归类」时，生产保护会把整条元数据查询拒掉，
        // 界面上的表现就是「打开数据库就报生产不允许」（真机反馈，已验证）。
        "USE" => StatementKind::Read,
        // SET：会话级设置（NOCOUNT / search_path / autocommit …）不改数据；
        // SET GLOBAL / SET PERSIST / SET @@GLOBAL 才是改服务器状态。
        "SET" => {
            // `SET PASSWORD`（MySQL）是改账号口令，别被会话级 SET 的宽松规则放过
            if upper.contains("GLOBAL") || upper.contains("PERSIST") || upper.contains("PASSWORD") {
                StatementKind::Write
            } else {
                StatementKind::Read
            }
        }
        "EXEC" | "EXECUTE" | "CALL" => {
            if read_only_procedure(&upper) {
                StatementKind::Read
            } else {
                StatementKind::Unknown
            }
        }
        "DBCC" => dbcc_kind(&upper),
        _ => {
            // SQL Server 允许省略 EXEC 直接写过程名（`sp_who2` / `sp_helpdb`）：白名单内按只读
            if read_only_procedure(&kw) {
                StatementKind::Read
            } else {
                StatementKind::Unknown
            }
        }
    }
}

/// `SELECT` 本身也会产生写副作用：`SELECT ... INTO` 会建表/写文件，
/// `nextval` / `setval` 会改序列，大对象函数会写服务端状态。
/// 这些在只读连接上必须按非只读处理。
fn select_kind(upper: &str, sql: &str) -> StatementKind {
    if top_level_keyword_pos(sql, "into").is_some()
        || SIDE_EFFECT_CALLS.iter().any(|call| upper.contains(call))
    {
        StatementKind::Write
    } else {
        StatementKind::Read
    }
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
    fn 危险语句识别() {
        assert!(danger_reason("delete from t").is_some());
        assert!(danger_reason("UPDATE t SET a = 1").is_some());
        assert!(danger_reason("truncate table t").is_some());
        assert!(danger_reason("drop table t").is_some());
        // 有 WHERE 的不算危险
        assert!(danger_reason("delete from t where id = 1").is_none());
        assert!(danger_reason("update t set a = 1 where id = 1").is_none());
        // WHERE 藏在字符串/子查询里不算顶层 WHERE
        assert!(danger_reason("update t set a = 'x where y'").is_some());
        assert!(danger_reason("delete from t where id in (select id from u where x = 1)").is_none());
        // 别的语句不归它管
        assert!(danger_reason("select * from t").is_none());
        assert!(danger_reason("insert into t values (1)").is_none());
    }

    #[test]
    fn 写目标解析() {
        let t = write_target("DELETE FROM `sales` WHERE id = 1").unwrap();
        assert_eq!(t.table, "`sales`");
        assert_eq!(t.where_clause.as_deref(), Some("id = 1"));

        let t = write_target("update t set a = 1, b = 2 where x > 10;").unwrap();
        assert_eq!(t.table, "t");
        assert_eq!(t.where_clause.as_deref(), Some("x > 10"));

        // 无 WHERE ⇒ 全表计数
        let t = write_target("delete from t").unwrap();
        assert!(t.where_clause.is_none());

        // 别名要带进计数查询（WHERE 里可能引用别名）
        let t = write_target("update t as a set x = 1 where a.y = 2").unwrap();
        assert_eq!(t.table, "t as a");

        // 复合形态解析不了 ⇒ None（预估跳过，不硬编错误的 COUNT）
        assert!(write_target("delete t1 from t1 join t2 on t1.id = t2.id").is_none());
        // SET 里带子查询无碍：影响行数就是 `COUNT(*) FROM t`
        assert!(write_target("update t set a = (select max(x) from u)").is_some());
        assert!(write_target("insert into t values (1)").is_none());
    }

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
    fn 无分号拆分_建表尾缀注释属于同一条() {
        // ClickHouse：表注释写在最后一行，`COMMENT` 后面是字符串字面量（不是 ON）⇒ 不能切
        let sql = "CREATE TABLE \"t\" (\n  \"id\" String COMMENT 'id'\n)\nENGINE = MergeTree()\nORDER BY (\"id\")\nCOMMENT '测试'";
        assert_eq!(split_statements(sql), vec![sql]);
        // MySQL 的表选项形态 `COMMENT='x'` 同理
        let my = "CREATE TABLE t (id int)\nCOMMENT='x'";
        assert_eq!(split_statements(my), vec![my]);
        // PG / Oracle 的 `COMMENT ON …` 是真语句：没分号也要切开
        let two = "CREATE TABLE t (id int)\nCOMMENT ON TABLE t IS 'x'";
        assert_eq!(
            split_statements(two),
            vec!["CREATE TABLE t (id int)", "COMMENT ON TABLE t IS 'x'"]
        );
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

    /// SELECT 里调用会改服务端状态的函数 ⇒ **不是只读**。
    ///
    /// 钉的是实机语义：PG/Kingbase 没有 `KILL` 语句，界面上的「终止会话」就是
    /// `select pg_terminate_backend(pid)`。判成只读的话，只读连接上照样能把会话杀掉；
    /// 而 MySQL / SQL Server 的 `KILL N` 在只读连接上是被拦住的 —— 两边必须一致。
    #[test]
    fn select_里的终止会话函数不是只读() {
        assert!(!is_read_only("select pg_terminate_backend(12345)"));
        assert!(!is_read_only("SELECT pg_cancel_backend(pid) FROM pg_stat_activity"));
        // 普通查询不受影响
        assert!(is_read_only("select pid, state from pg_stat_activity"));
        assert!(is_read_only("select count(*) from t"));
    }

    /// 不带等号的 PRAGMA：只有明确只读的那几个算读，会动库文件的归「不确定」。
    ///
    /// 「不确定」的处理是安全的：只读连接拦下，可写连接照常放行（不误伤）。
    #[test]
    fn 会动库文件的_pragma_不算只读() {
        assert!(is_read_only("PRAGMA table_info(t)"));
        assert!(is_read_only("PRAGMA foreign_key_check"));
        assert!(is_read_only("PRAGMA integrity_check"));
        assert!(!is_read_only("PRAGMA wal_checkpoint(TRUNCATE)"));
        assert!(!is_read_only("PRAGMA optimize"));
        assert!(!is_read_only("PRAGMA shrink_memory"));
        assert!(!is_read_only("PRAGMA incremental_vacuum"));
        // 带等号本来就是写
        assert!(!is_read_only("PRAGMA journal_mode = WAL"));
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
        // SET 是会话级设置（不改数据），只有 GLOBAL / PERSIST / PASSWORD 才算写
        assert!(is_read_only("SET search_path = public"));
    }

    /// 生产库排障：切库、会话设置、只读的系统过程 / DBCC 不该被当成写操作拦下。
    ///
    /// 背景（真机）：SQL Server 的元数据 SQL 会被内核前置 `use [库];`，`USE` 判成
    /// 「无法归类」后，生产保护把整条元数据查询拒掉 —— 用户看到的是
    /// 「打开数据库就报生产不允许」。
    #[test]
    fn 切库与会话设置与只读诊断语句不算写() {
        assert!(is_read_only("USE master"));
        assert!(is_read_only("use [demo]"));
        assert!(is_read_only("SET NOCOUNT ON"));
        assert!(is_read_only("SET autocommit = 0"));
        assert!(!is_read_only("SET GLOBAL max_connections = 500"));
        assert!(!is_read_only("SET PERSIST max_connections = 500"));
        assert!(!is_read_only("SET PASSWORD FOR u = 'x'"));
        // 不带 EXEC 的过程调用（SQL Server 允许）
        assert!(is_read_only("sp_who2"));
        assert!(is_read_only("exec sp_who"));
        assert!(is_read_only("EXEC sp_helpdb"));
        assert!(is_read_only("DBCC SQLPERF(LOGSPACE)"));
        assert!(is_read_only("dbcc show_statistics(t, idx)"));
        // 会执行任意 SQL / 跑系统命令 / 改服务器状态的照旧拦下
        assert!(!is_read_only("exec sp_executesql N'delete from t'"));
        assert!(!is_read_only("exec xp_cmdshell 'dir'"));
        assert!(!is_read_only("dbcc freeproccache"));
        assert!(!is_read_only("dbcc shrinkfile(1)"));
        assert!(!is_read_only("kill 53"));
        assert!(!is_read_only("exec sp_configure 'show advanced options', 1"));
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

    /// 没写分号：两条 SELECT 连着写也要能各自成段
    #[test]
    fn implicit_split_selects_without_semicolon() {
        let sql = "SELECT id, name\nFROM users\nSELECT id, name\nFROM users";
        let parts = split_statements(sql);
        assert_eq!(parts.len(), 2, "{parts:#?}");
        assert!(parts[0].starts_with("SELECT id"));
        assert!(parts[1].starts_with("SELECT id"));
    }

    /// INSERT … SELECT / CREATE TABLE AS SELECT 是**一条**语句，不能被行首的 SELECT 切开
    #[test]
    fn implicit_split_keeps_insert_select() {
        assert_eq!(split_statements("INSERT INTO t (a)\nSELECT a FROM s").len(), 1);
        assert_eq!(split_statements("CREATE TABLE t AS\nSELECT 1 AS a").len(), 1);
    }

    /// UNION 跨行：行首 SELECT 是续接，不是新语句
    #[test]
    fn implicit_split_keeps_union() {
        assert_eq!(
            split_statements("SELECT a FROM t\nUNION ALL\nSELECT a FROM u").len(),
            1
        );
    }

    /// 括号里的 SELECT（子查询）不切
    #[test]
    fn implicit_split_ignores_subquery() {
        assert_eq!(split_statements("SELECT *\nFROM (\nSELECT a FROM t\n) x").len(), 1);
    }

    /// DDL 连着写（无分号）也要分段
    #[test]
    fn implicit_split_ddl_without_semicolon() {
        assert_eq!(
            split_statements("DROP TABLE IF EXISTS t\nCREATE TABLE t (a INT)\nINSERT INTO t VALUES (1)")
                .len(),
            3
        );
    }

    /// 上一行以逗号/运算符结尾 = 话没说完，不切
    #[test]
    fn implicit_split_requires_complete_tail() {
        assert_eq!(split_statements("SELECT a,\nSELECT b FROM t").len(), 1);
    }

    /// 例程体（BEGIN … END）里即使不写分号也不切
    #[test]
    fn implicit_split_skips_begin_block() {
        assert_eq!(
            split_statements("CREATE PROCEDURE p() BEGIN\nSELECT 1\nEND").len(),
            1
        );
    }

    /// 上一条以**标识符/字符串字面量**结尾时也要能切。
    ///
    /// `FROM \`users\`` 的上一个「词」还停在 FROM 上，而 FROM 是续接词白名单里的 ——
    /// 不在引号闭合时清掉 last_word，两条无分号的 SELECT 就永远粘成一条
    /// （真机：编辑器里两条 SELECT 只隔一个空行，执行报 near 'SELECT ...' 语法错）。
    #[test]
    fn implicit_split_after_quoted_tail() {
        let sql = "SELECT\n  `id`,\n  `name`\nFROM `users`\n\n\
                   SELECT\n  `id`,\n  `name`\nFROM `users`;";
        let parts = split_statements(sql);
        assert_eq!(parts.len(), 2, "{parts:#?}");
        assert!(parts[1].starts_with("SELECT"));
        assert_eq!(split_statements("SELECT * FROM `users`\nSELECT 1").len(), 2);
        assert_eq!(split_statements("SELECT * FROM \"users\"\nSELECT 1").len(), 2);
        assert_eq!(
            split_statements("SELECT * FROM t WHERE name LIKE 'a%'\nSELECT 1").len(),
            2
        );
    }

    /// 老行为不能退化：引号/注释里的分号照旧不算分隔符。
    #[test]
    fn quotes_and_comments_still_do_not_split() {
        assert_eq!(split_statements("INSERT INTO t VALUES ('a;b');").len(), 1);
        assert_eq!(split_statements("SELECT 1; -- ;\nSELECT 2;").len(), 2);
        assert_eq!(split_statements("SELECT /* ; */ 1;").len(), 1);
        // 转义的双引号/反引号（"" / ``）不结束标识符，里面的分号也就不算分隔符
        assert_eq!(split_statements("SELECT \"a\"\"b;c\"; SELECT 1;").len(), 2);
        assert_eq!(split_statements("SELECT `a``b;c`; SELECT 1;").len(), 2);
    }

    /// `#temp` 不是行注释：SQL Server 的临时表就叫 `#temp`，当注释会把整行吞掉。
    ///
    /// 前端 `utils/sqlSplit.js` 必须给出同样的段数（它此前把 `#` 当注释，
    /// `SELECT * FROM #temp; SELECT 1;` 被算成 1 条 → 界面显示 1 条、实际执行 2 条）。
    #[test]
    fn hash_is_not_a_line_comment() {
        assert_eq!(split_statements("SELECT * FROM #temp; SELECT 1;").len(), 2);
        assert_eq!(split_statements("SELECT * FROM #temp\nSELECT 1").len(), 2);
        // 行首 `# ` 是 MySQL/MariaDB 注释；里面的分号不切。
        assert_eq!(split_statements("# DROP TABLE x;\nSELECT 1;").len(), 1);
        assert_eq!(split_statements("#foo;").len(), 1);
        assert_eq!(classify("# note\nSELECT 1"), StatementKind::Read);
    }

    /// 复杂 SQL 不能被切碎（前端 `sqlcomplex-check.mjs` 是同一批用例）。
    #[test]
    fn complex_statements_stay_together() {
        // MySQL 触发器：格式化会把 INSERT 顶到行首 —— 不能被当成新语句
        let trigger = "CREATE TRIGGER trg1 BEFORE\nINSERT\n  ON t1 FOR EACH ROW BEGIN\n  \
                       SET NEW.kind = CASE WHEN NEW.v > 0 THEN 'pos' ELSE 'neg' END;\n  \
                       INSERT INTO audit VALUES (NEW.id);\nEND";
        assert_eq!(split_statements(trigger).len(), 1);

        // Oracle 存储过程：AS 之后、BEGIN 之前的声明语句分号不是语句边界
        let procedure = "CREATE OR REPLACE PROCEDURE p2(p_id IN NUMBER) AS\n  v NUMBER := 0;\n\
                         BEGIN\n  SELECT count(*) INTO v FROM t WHERE id = p_id;\n  \
                         DELETE FROM t WHERE id = p_id;\nEND;";
        assert_eq!(split_statements(procedure).len(), 1);

        // PG 美元引号：壳里不带 BEGIN 的 SQL 函数体，分号也不算
        let pg = "CREATE FUNCTION f2() RETURNS int AS $$ SELECT 1; SELECT 2; $$ LANGUAGE sql;";
        assert_eq!(split_statements(pg).len(), 1);
        let pg_pl = "CREATE OR REPLACE FUNCTION f1() RETURNS void AS $$\nBEGIN\n  PERFORM 1;\n  \
                      DELETE FROM t WHERE a = 0;\nEND\n$$ LANGUAGE plpgsql;";
        assert_eq!(split_statements(pg_pl).len(), 1);

        // WITH 的查询体写完后，无分号的下一条 SELECT 要能切出来
        let cte = "WITH RECURSIVE tree AS (\n  SELECT id FROM nodes\n  UNION ALL\n  \
                   SELECT n.id FROM nodes n JOIN tree t ON n.parent_id = t.id\n)\n\
                   SELECT * FROM tree\n\nSELECT count(*) FROM tree";
        assert_eq!(split_statements(cte).len(), 2);

        // WITH + INSERT 是一条语句，后面顶格的 SELECT 才是新语句
        let with_insert = "WITH t AS (SELECT 1 AS a)\nINSERT INTO log SELECT a FROM t\n\nSELECT * FROM log";
        assert_eq!(split_statements(with_insert).len(), 2);

        // 三条无分号（SELECT / DDL / DML）
        assert_eq!(
            split_statements("SELECT * FROM t1\n\nTRUNCATE TABLE t2\n\nINSERT INTO t2 VALUES (1)").len(),
            3
        );
    }

    /// CTE 的主语句必须按词法判读：换行、Tab 都不能让写语句变成 Read。
    #[test]
    fn with_main_statement_is_classified_by_lexing_not_spacing() {
        for sql in [
            "WITH x AS (SELECT 1) DELETE FROM t",
            "WITH x AS (SELECT 1)\nDELETE FROM t",
            "WITH x AS (SELECT 1)\tDELETE FROM t",
            "WITH x AS (SELECT 1)\nINSERT INTO t SELECT * FROM x",
            "WITH x AS (SELECT 1)\r\nUPDATE t SET a = 1",
        ] {
            assert_eq!(classify(sql), StatementKind::Write, "{sql:?}");
            assert!(!is_read_only(sql), "{sql:?}");
        }
        assert_eq!(
            classify("WITH x AS (SELECT 1)\nSELECT * FROM x"),
            StatementKind::Read
        );
        assert_eq!(
            classify("WITH x AS (SELECT ' DELETE ' AS s) SELECT * FROM x"),
            StatementKind::Read
        );
        let recursive = "with recursive c(x) as (select 1 union all select x+1 from c where x < 10) select x from c";
        assert_eq!(classify(recursive), StatementKind::Read);

        assert!(danger_reason("WITH x AS (SELECT 1)\nDELETE FROM t").is_some());
        let target = write_target("WITH x AS (SELECT 1)\nUPDATE t SET a = 1 WHERE id = 2")
            .expect("CTE UPDATE 应能解析目标");
        assert_eq!(target.table, "t");
        assert_eq!(target.where_clause.as_deref(), Some("id = 2"));
    }

    /// `SELECT ... INTO` / 序列函数会写状态，只读连接不能当作 Read 放行。
    #[test]
    fn select_side_effects_are_not_read_only() {
        for sql in [
            "SELECT * INTO new_table FROM t",
            "SELECT nextval('seq')",
            "SELECT setval('seq', 1)",
            "WITH x AS (SELECT 1) SELECT * INTO new_table FROM x",
        ] {
            assert_eq!(classify(sql), StatementKind::Write, "{sql:?}");
            assert!(!is_read_only(sql), "{sql:?}");
        }
        assert_eq!(classify("SELECT * FROM t WHERE name = 'INTO'"), StatementKind::Read);
    }

    /// 非 ASCII 标识符不能让 `top_level_keyword_pos` 在字符中间切字符串。
    #[test]
    fn non_ascii_identifiers_do_not_panic_in_safety_parser() {
        let sql = "UPDATE 表 SET a = 1 WHERE id = 1";
        let target = write_target(sql).expect("应能解析中文表名");
        assert_eq!(target.table, "表");
        assert_eq!(danger_reason(sql), None);
        assert!(danger_reason("DELETE FROM 用户").is_some());
        assert_eq!(classify(sql), StatementKind::Write);
    }

    /// `BEGIN TRAN` / `BEGIN IMMEDIATE` 是事务起点，不是 `BEGIN ... END` 代码块。
    #[test]
    fn begin_transaction_spellings_do_not_swallow_following_statements() {
        for sql in [
            "BEGIN TRAN;\nUPDATE t SET a = 1;\nCOMMIT;",
            "BEGIN IMMEDIATE;\nUPDATE t SET a = 1;\nCOMMIT;",
            "BEGIN DEFERRED;\nUPDATE t SET a = 1;\nCOMMIT;",
            "BEGIN EXCLUSIVE;\nUPDATE t SET a = 1;\nCOMMIT;",
        ] {
            assert_eq!(split_statements(sql).len(), 3, "{sql:?}");
        }
    }

    /// 真机：AI 生成的「建索引」脚本 —— 每条 `ALTER TABLE …` 后面跟
    /// `ADD INDEX (…),` 换行 `ALGORITHM=INPLACE, LOCK=NONE;`，段间还有 `-- P0-x` 注释。
    ///
    /// 分号是唯一的分块点：`ADD INDEX` 那行**必须**留在同一条里
    /// （断在那里等于把 `ALTER TABLE t` 半句单独发出去，后面那条 ALTER 也会被吞进同一条）。
    #[test]
    fn 多行_alter_加索引按分号切分() {
        let sql = "-- P0-1 收小列表：把 DML 全表列改 ref 查找 + 覆盖索引聚合（当前 13.5s）
ALTER TABLE wineryun_hubs.data_hour
    ADD INDEX idx_site_time (SITE_SITE_ID, DATA_TIME, IN_NUM, OUT_NUM),
    ALGORITHM=INPLACE, LOCK=NONE;
-- P0-2 较小时列表：整条数据无索引，JOIN 键 mail_id 与聚合列一起做覆盖（当前 2.8s）
ALTER TABLE sales.data_hour
    ADD INDEX idx_join_mail_time (mail_id, data_time, in_num, out_num),
    ALGORITHM=INPLACE, LOCK=NONE;
-- P0-3 覆盖聚合索引
ALTER TABLE sales.data_monthly_xf_surcharge
    ADD INDEX idx_xf_surcharge (id, mail_id, contract_grp, xf_surcharge),
    ALGORITHM=INPLACE, LOCK=NONE;
-- P1 数据收录 月度汇总（当月/上月/去年同期），250ms×3
ALTER TABLE etl_wide_income_exocbt
    ADD INDEX idx_income_exocbt_month_type (XF_FINANCIALMONTH, XF_CURRENCY_TYPE, XF_MAILDID, XF_SETTLEAMT),
    ALGORITHM=INPLACE, LOCK=NONE;
";
        let parts = split_statements(sql);
        for (i, p) in parts.iter().enumerate() {
            println!("--- [{i}] ---\n{p}");
        }
        assert_eq!(parts.len(), 4, "应按 4 个分号切成 4 条：{parts:#?}");
        for p in &parts {
            assert!(p.contains("ALTER TABLE"), "每段都得是完整 ALTER：{p:?}");
        }

        // 分号后面的**同行注释**（`… LOCK=NONE;   -- 预估 +80MB`）是上一条的尾注：
        // 下一段不能把它顶到前面 —— 前端语句框就是因此多圈了上一行（真机截图）。
        // 后端本来就把注释剥成空白，这里钉住这个事实，两端切出来的段落才一致。
        let trail = "ALTER TABLE a.t1\n  ADD INDEX i1 (c1),\n  ALGORITHM=INPLACE, LOCK=NONE;   -- 预估 +80MB\n\n-- P1\nALTER TABLE a.t2\n  ADD INDEX i2 (c2),\n  ALGORITHM=INPLACE, LOCK=NONE;   -- 预估 +40MB\n";
        let two = split_statements(trail);
        assert_eq!(two.len(), 2);
        assert!(
            two[1].starts_with("ALTER TABLE a.t2"),
            "第二段不能被上一条的尾注顶到前面：{:?}",
            two[1]
        );
    }
}
