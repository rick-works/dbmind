/**
 * SQL 多段判定/拆分工具（前端侧）。
 *
 * 作用：执行前判断编辑器内 SQL 是否包含多条语句（分号分隔），决定是走
 * 「批量执行、结果以 tab 逐条展示」还是「单条执行（保留分页）」。
 *
 * 与后端 AbstractRelationalDbService#splitStatements 的拆分规则保持一致：
 * 引号（' " `）、行注释（-- #）、块注释（/ * * / ）、PostgreSQL 美元引号（$tag$…）
 * 内的分号不会触发拆分。
 */

/**
 * 按「真正结束语句的分号」把 SQL 拆段，并给出每段在原文中的字符偏移。
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
  let line = false       // 行注释（-- / #）
  let block = false      // 块注释
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
      if (ch === '\n') line = false
      i++
      continue
    }
    if (block) {
      if (ch === '*' && next === '/') { block = false; i += 2 }
      else i++
      continue
    }
    if (dollar) {
      if (s.startsWith(dollar, i)) { i += dollar.length; dollar = null }
      else i++
      continue
    }
    if (quote) {
      if (ch === quote) {
        if (next === quote) i += 2 // 转义引号 '' / ""
        else { quote = 0; i++ }
      } else i++
      continue
    }
    // 美元引号开始（$$ 或 $tag$）
    if (ch === '$') {
      let j = i + 1
      while (j < n && /[A-Za-z0-9_]/.test(s[j])) j++
      if (j < n && s[j] === '$') {
        dollar = s.substring(i, j + 1)
        i = j + 1
        continue
      }
    }
    if (ch === '\'' || ch === '"' || ch === '`') { quote = ch; i++; continue }
    if (ch === '-' && next === '-') { line = true; i += 2; continue }
    if (ch === '#' && (i === 0 || s[i - 1] === '\n' || /\s/.test(s[i - 1]))) { line = true; i += 1; continue }
    if (ch === '/' && next === '*') { block = true; i += 2; continue }
    if (ch === ';') {
      pushRange(stmtStart, i)
      stmtStart = i + 1
      i++
      continue
    }
    i++
  }
  pushRange(stmtStart, n)
  return list
}

/**
 * 去掉开头的行注释（-- / #）与块注释（斜杠星号 … 星号斜杠），返回剩下的正文。
 * 用途只有一个：判断"这一段里到底有没有真语句"。
 * 返回空串 ⇒ 纯注释/空白，不算一条语句；返回非空 ⇒ 哪怕只有半句，也要算。
 */
function stripLeadingComments(text) {
  let t = String(text || '').trimStart()
  for (;;) {
    if (t.startsWith('--') || t.startsWith('#')) {
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
