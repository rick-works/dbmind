/**
 * SQL 多段判定/拆分工具（前端侧）。
 *
 * 作用：执行前判断编辑器内 SQL 是否包含多条语句（分号分隔），决定是走
 * 「批量执行、结果以 tab 逐条展示」还是「单条执行（保留分页）」；同时给编辑器的
 * 「共 N 条语句」面包屑、语句高亮框提供每段的字符偏移。
 *
 * 与后端 `dbmind_core::sql::split_statements` 的拆分规则保持一致（**必须一致**：
 * 前端显示 2 条、后端只切成 1 条的话，「执行全部」会整段报语法错）：
 *
 * 1. 引号（' " `）、行注释（--）、块注释（斜杠星号）里的分号不会触发拆分；
 *    `BEGIN … END` / `CASE … END` 块里的分号也不算。
 * 2. **没写分号也分段**：行首出现语句起始关键字、且上一条看起来已经写完时自动断开
 *    （见 `implicitBreak`）。规则刻意保守 —— 拿不准就不切。
 *
 * `#` 只在**行首且后面跟空白**时按 MySQL/MariaDB 注释处理；SQL Server 的
 * 临时表 `#temp` 后面是字母，不会被吞掉。后端 `split_statements` 用同一规则。
 */

/** 能开启一条新语句的关键字（还要通过上下文判定才真的切） */
const IMPLICIT_HEADS = new Set([
  'SELECT', 'WITH', 'INSERT', 'REPLACE', 'UPDATE', 'DELETE', 'MERGE', 'CREATE', 'ALTER', 'DROP',
  'TRUNCATE', 'RENAME', 'COMMENT', 'GRANT', 'REVOKE', 'CALL', 'EXEC', 'EXECUTE', 'SHOW', 'DESC',
  'DESCRIBE', 'EXPLAIN', 'ANALYZE', 'VACUUM', 'OPTIMIZE', 'USE', 'SET', 'COMMIT', 'ROLLBACK',
  'START', 'SAVEPOINT', 'RELEASE', 'PREPARE', 'DEALLOCATE'
])

/** 行首这些词其实是上一条语句的延续（UNION SELECT / AND x / FROM t …），绝不切 */
const IMPLICIT_CONTINUE = new Set([
  'UNION', 'ALL', 'INTERSECT', 'EXCEPT', 'MINUS', 'AS', 'INTO', 'VALUES', 'FROM', 'JOIN', 'ON',
  'USING', 'WHERE', 'AND', 'OR', 'NOT', 'IN', 'EXISTS', 'BETWEEN', 'LIKE', 'IS', 'THEN', 'ELSE',
  'WHEN', 'CASE', 'HAVING', 'GROUP', 'ORDER', 'BY', 'LIMIT', 'OFFSET', 'RETURNING', 'DO', 'WITH'
])

const isWordChar = (ch) => !!ch && /[A-Za-z0-9_$]/.test(ch)

/** 从 from 起跳过空白读下一个词并大写（判 BEGIN 后是不是 WORK/TRANSACTION、END 后是不是 IF/LOOP） */
function nextWordAt(s, from) {
  let i = from
  while (i < s.length && /\s/.test(s[i])) i++
  let w = ''
  while (i < s.length && isWordChar(s[i])) { w += s[i]; i++ }
  return w.toUpperCase()
}

/**
 * 「没写分号也分段」的判定：word 是行首刚读完的词，其余是当前语句的上下文。
 * 与后端 `implicit_break` 逐条对齐。
 */
function implicitBreak({ word, prevText, paren, depth, lastCode, lastWord, headWord, sawSelect, routine, mainStarted, attachSelect, nextWord }) {
  if (!stripLeadingComments(prevText) || paren !== 0 || depth !== 0) return false
  // 例程/触发器 DDL：`CREATE TRIGGER … BEFORE INSERT`、`CREATE PROCEDURE …` 里的
  // INSERT / SET / SELECT 都是语法的一部分，绝不能当成新语句开头。
  if (routine) return false
  if (!IMPLICIT_HEADS.has(word)) return false
  // 上一条的最后一个字符必须「像个结尾」：字/数字/右括号/闭合引号
  const tailOk = !!lastCode && (/[A-Za-z0-9_$)\]}'"`]/.test(lastCode))
  if (!tailOk) return false
  if (IMPLICIT_CONTINUE.has((lastWord || '').toUpperCase())) return false
  const head = (headWord || '').toUpperCase()
  if (head === 'BEGIN') return false
  // INSERT … SELECT / CREATE TABLE … AS SELECT：**主查询还没开始**时那个 SELECT
  // 属于同一条语句；已经出现过顶层 SELECT 后再顶格起 SELECT 就是新语句。
  if (word === 'SELECT' && !sawSelect
    && ['INSERT', 'REPLACE', 'MERGE', 'CREATE', 'EXPLAIN'].includes(head)) return false
  // CTE：`WITH cte AS (…)` 之后的主语句可以是 SELECT / INSERT / UPDATE / DELETE …
  //   - 主语句还没开始时，顶格出现的 DML/SELECT 都属于这条 WITH 语句；
  //   - 主语句是 INSERT/REPLACE/MERGE 时，紧跟的那个 SELECT 也属于它（INSERT … SELECT）；
  //   - 其余情况（主语句已是 SELECT，或 INSERT 的 SELECT 已经写过）再顶格 SELECT = 新语句。
  if (head === 'WITH') {
    if (!mainStarted && ['INSERT', 'REPLACE', 'MERGE', 'UPDATE', 'DELETE'].includes(word)) return false
    if (!mainStarted && word === 'SELECT') return false
    if (mainStarted && attachSelect && word === 'SELECT') return false
  }
  // UPDATE t SET … / INSERT … SET … / CREATE TRIGGER … SET … 里的 SET 属于上一条
  if (word === 'SET' && ['UPDATE', 'INSERT', 'CREATE'].includes(head)) return false
  // COMMENT 只有后面跟 ON 时才是新语句（PG / Oracle 的 `COMMENT ON TABLE … IS …`）。
  // ClickHouse 建表的表注释写在最后一行（`… \nCOMMENT '说明'`），MySQL 的表选项是
  // `COMMENT='…'` —— 都属于上一条 CREATE TABLE；误切会把 `COMMENT '说明'` 单独发出去，
  // ClickHouse 报 `failed at position 1 ('COMMENT')`（真机踩过）。
  if (word === 'COMMENT' && String(nextWord || '').toUpperCase() !== 'ON') return false
  return true
}

/**
 * 按「真正结束语句的位置」把 SQL 拆段，并给出每段在原文中的字符偏移。
 * 编辑器右键的「选中当前语句」用它把光标所在语句整段选中。
 *
 * @param {string} sql 原始 SQL
 * @returns {Array<{start:number,end:number,text:string}>} start/end 为原文偏移（已去掉首尾空白）
 */
export function splitSqlStatementRanges(sql) {
  const s = sql || ''
  const list = []
  let stmtStart = 0      // 当前语句起始偏移
  let quote = 0          // ' " `，0 表示不在引号内
  let dollar = null      // 当前美元引号 tag（$$ / $body$）
  let line = false       // 行注释（--）
  let block = false      // 块注释
  // ---- 无分号分段用的上下文（与后端同名变量一一对应）----
  let paren = 0          // 括号深度：子查询里的 SELECT 不能切
  let depth = 0          // BEGIN/CASE…END 块深度：块里的分号与关键字都不切
  let lineStart = true   // 是否在行首（只见过空白）
  let lastWord = ''      // 上一条语句最后一个词
  let headWord = ''      // 当前语句第一个词
  let lastCode = ''      // 最后一个非空白代码字符
  let word = ''          // 正在读的词
  let wordStart = 0      // 该词起始偏移
  let wordAtLineStart = false
  // ---- 例程 / 触发器 DDL（CREATE PROCEDURE/FUNCTION/TRIGGER/PACKAGE）----
  let routine = false     // 当前语句是例程类 DDL：体内的分号/关键字都不能当语句边界
  let decl = false        // Oracle 风格声明段（AS/IS 之后、BEGIN 之前）：`v NUMBER := 0;` 的分号不是语句结束
  let beginSeen = false   // 例程体已经开始（BEGIN）
  let dollarBody = false  // 体是美元引号字面量（AS $$ … $$）：声明段到字面量就结束了
  let sawSelect = false   // 当前语句已出现过顶层 SELECT（判 INSERT…SELECT 的主查询是否已开始）
  let mainStarted = false // WITH 语句的主语句已开始（CTE 部分之后）
  let attachSelect = false// 主语句是 INSERT/REPLACE/MERGE，紧跟的 SELECT 仍属于它
  let i = 0
  const n = s.length

  const pushRange = (from, to) => {
    const raw = s.slice(from, to)
    const text = raw.trim()
    // 空段、以及**纯注释段**才算不上一条语句。
    // 关键：不能只判"开头是不是注释"—— 注释里的分号不算分隔符，所以一条注释后面紧跟的
    // 真语句会和它分进**同一段**（`-- 注释\nSELECT …;`），那一段开头是 -- 但**含真语句**，
    // 按开头丢掉就会少算（真机现象：9 条里注释掉 2 条，结果集是 7 条，顶部却显示"共 5 条语句"）。
    if (!stripLeadingComments(text)) return
    const start = from + (raw.length - raw.trimStart().length)
    list.push({ start, end: start + text.length, text })
  }

  while (i < n) {
    const ch = s[i]
    const next = i + 1 < n ? s[i + 1] : ''

    if (line) {
      if (ch === '\n') { line = false; lineStart = true }
      i++
      continue
    }
    if (block) {
      if (ch === '*' && next === '/') { block = false; i += 2 }
      else i++
      continue
    }
    if (dollar) {
      if (s.startsWith(dollar, i)) {
        i += dollar.length
        dollar = null
        lastCode = '$'
        lastWord = ''       // 美元引号串本身就是一个「词」
        // 例程体就是这段字面量 ⇒ 声明段结束，后面的 `;`（LANGUAGE …;）才是语句终止符
        if (routine) { decl = false; dollarBody = true }
      } else i++
      continue
    }
    if (quote) {
      if (ch === quote) {
        if (next === quote) i += 2 // 转义引号 '' / ""
        else {
          quote = 0
          lastCode = ch
          // 引号/字符串本身也是一个「词」：清掉 lastWord，否则 ``FROM `users` `` 这条
          // 语句的 lastWord 还停在上一个关键字 FROM 上，而 FROM 是「续接词」白名单里的，
          // 于是下一条无分号的 SELECT 永远切不开（真机：两条 SELECT 粘成一条报语法错）。
          lastWord = ''
          i++
        }
      } else i++
      continue
    }
    // 美元引号开始（$$ 或 $tag$）：必须排在「词」之前 —— `$` 也算词字符，
    // 放在后面会被当成普通单词吃掉，函数体里的分号就全都暴露出来把函数切碎。
    if (ch === '$' && !word) {
      let j = i + 1
      while (j < n && /[A-Za-z0-9_]/.test(s[j])) j++
      if (j < n && s[j] === '$') {
        dollar = s.substring(i, j + 1)
        lastCode = '$'
        lastWord = ''
        i = j + 1
        continue
      }
    }
    // 词：先攒起来，读完再判定（判定时要把「这个词」排除在上一条之外）
    if (isWordChar(ch)) {
      if (!word) { wordAtLineStart = lineStart; wordStart = i }
      word += ch
      lineStart = false
      i++
      continue
    }
    if (word) {
      const upper = word.toUpperCase()
      if (wordAtLineStart && implicitBreak({
        word: upper,
        prevText: s.slice(stmtStart, wordStart),
        paren, depth, lastCode, lastWord, headWord, sawSelect, routine, mainStarted, attachSelect,
        nextWord: nextWordAt(s, i)
      })) {
        pushRange(stmtStart, wordStart)
        stmtStart = wordStart
        paren = 0
        depth = 0
        headWord = ''
        lastWord = ''
        lastCode = ''
        sawSelect = false
        routine = false
        decl = false
        beginSeen = false
        dollarBody = false
        mainStarted = false
        attachSelect = false
      }
      // CREATE [OR REPLACE] PROCEDURE / FUNCTION / TRIGGER / PACKAGE：整条是一条语句
      if (['PROCEDURE', 'FUNCTION', 'TRIGGER', 'PACKAGE'].includes(upper)
        && ['CREATE', 'ALTER'].includes(headWord)) routine = true
      // Oracle 的声明段：AS/IS 之后、BEGIN 之前（`v NUMBER := 0;` 这些分号不是语句边界）
      if (routine && depth === 0 && (upper === 'AS' || upper === 'IS')) decl = true
      if (upper === 'BEGIN') {
        // SQL Server 的 BEGIN TRAN、SQLite 的 BEGIN IMMEDIATE 都是事务起点，
        // 后面没有配对的 END；漏掉它们会把整个脚本吞成一条语句。
        if (!['', 'WORK', 'TRAN', 'TRANSACTION', 'DEFERRED', 'IMMEDIATE', 'EXCLUSIVE', 'DISTRIBUTED']
          .includes(nextWordAt(s, i))) { depth++; beginSeen = true }
      } else if (upper === 'CASE') {
        depth++
      } else if (upper === 'END') {
        if (!['IF', 'LOOP', 'WHILE', 'REPEAT'].includes(nextWordAt(s, i))) depth = Math.max(0, depth - 1)
      }
      // 顶层 SELECT 出现即为「主查询已开始」：此后顶格的 SELECT 就是新语句
      if (upper === 'SELECT' && paren === 0) sawSelect = true
      // CTE 主语句的进度（只认顶层词，CTE 体内的 SELECT 不算）
      if (paren === 0 && depth === 0 && headWord === 'WITH') {
        if (!mainStarted) {
          if (['INSERT', 'REPLACE', 'MERGE'].includes(upper)) { mainStarted = true; attachSelect = true }
          else if (['SELECT', 'UPDATE', 'DELETE'].includes(upper)) mainStarted = true
        } else if (attachSelect && upper === 'SELECT') {
          attachSelect = false
        }
      }
      if (!headWord) headWord = upper
      lastWord = upper
      lastCode = word[word.length - 1]
      word = ''
      wordAtLineStart = false
    }

    if (ch === '#' && lineStart && next && /\s/.test(next)) {
      // 行首的 `# 注释`（MySQL/MariaDB）；`#temp` 后面是字母，不在此列。
      line = true
      i++
      continue
    }
    if (ch === '\'' || ch === '"' || ch === '`') { quote = ch; lastCode = ch; lineStart = false; i++; continue }
    if (ch === '-' && next === '-') { line = true; i += 2; continue }
    // 其余位置的 `#` 不算注释（SQL Server 的 `#temp` 临时表、以及与后端一致）
    if (ch === '/' && next === '*') { block = true; i += 2; continue }
    if (ch === ';') {
      // 例程声明段里的分号（Oracle：`v NUMBER := 0;` 在 BEGIN 之前）不是语句结束符
      const inDecl = routine && decl && !beginSeen && !dollarBody
      if (depth === 0 && !inDecl) {
        pushRange(stmtStart, i)
        stmtStart = i + 1
        paren = 0
        depth = 0
        headWord = ''
        lastWord = ''
        lastCode = ''
        sawSelect = false
        routine = false
        decl = false
        beginSeen = false
        dollarBody = false
        mainStarted = false
        attachSelect = false
      } else {
        lastCode = ';'
      }
      lineStart = false
      i++
      continue
    }
    if (ch === '(') { paren++; lastCode = '('; lineStart = false; i++; continue }
    if (ch === ')') { paren = Math.max(0, paren - 1); lastCode = ')'; lineStart = false; i++; continue }
    if (ch === '\n') { lineStart = true; i++; continue }
    if (!/\s/.test(ch)) { lastCode = ch; lineStart = false }
    i++
  }
  pushRange(stmtStart, n)
  return list
}

/**
 * 去掉开头的行注释（--）与块注释（斜杠星号 … 星号斜杠），返回剩下的正文。
 * 用途只有一个：判断"这一段里到底有没有真语句"。
 * 返回空串 ⇒ 纯注释/空白，不算一条语句；返回非空 ⇒ 哪怕只有半句，也要算。
 *
 * `#` 不在此列（与后端一致，见文件头：SQL Server 的 `#temp` 是表名）。
 */
function stripLeadingComments(text) {
  let t = String(text || '').trimStart()
  for (;;) {
    if (t.startsWith('--')
      || (t.startsWith('#') && (t.length === 1 || /\s/.test(t[1])))) {
      const nl = t.indexOf('\n')
      if (nl < 0) return ''
      t = t.slice(nl + 1).trimStart()
      continue
    }
    if (t.startsWith('/*')) {
      const end = t.indexOf('*/')
      if (end < 0) return ''
      t = t.slice(end + 2).trimStart()
      continue
    }
    return t
  }
}

/** 只要每段文本（执行链路沿用这个） */
export function splitSqlStatements(sql) {
  // 一定要滤掉空白段：`SELECT …;` 这种**末尾带分号**的写法会多切出一段空的，
  // 于是「一条语句」被算成「多段」。

  // 这一步不是洁癖 —— 调用方按段数决定走哪条链路：
  //   段数 > 1 ⇒ 批量接口（不传分页参数，后端也不统计总数）
  //   段数 = 1 ⇒ 单条接口（带 page/size，能翻页、能显示「共 N 条」）
  // 少滤这一个空段，用户就会看到：条数没了、翻页也没了。
  return splitSqlStatementRanges(sql)
    .map(r => r.text)
    .filter(text => text.trim().length > 0)
}
