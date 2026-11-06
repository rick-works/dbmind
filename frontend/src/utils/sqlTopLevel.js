/**
 * 找 SQL **顶层**（括号外、字符串/注释外）的 ORDER BY / LIMIT / OFFSET / FETCH 位置。
 *
 * 只有一个用途：结果表格点表头排序时，要把"作用于全部结果"的 ORDER BY 塞进用户那条 SQL ——
 * 做法是**替换**顶层已有的 ORDER BY（没有就追加），而不是外面包一层派生表。
 * 包派生表有两个毛病：
 *   1. Oracle 等方言对子查询里的 ORDER BY 限制多，`(select … order by …)` 直接报语法错；
 *   2. 执行历史、字段注释、报错定位都会指向一条用户并没写过的 SQL。
 *
 * 返回值里的偏移是原串下标，调用方据此做切片替换（-1 表示没有）。
 */
/**
 * 去掉 SQL 末尾的分号/空白/尾部注释：`select … from t;`、`… from t -- 备注` 都是最常见的写法，
 * 不能被当成"多语句"，也不能让 `--` 注释把后面拼上去的 ORDER BY 吞掉。
 */
export function stripTailSemicolon(s) {
  let out = String(s || '').trim()
  for (let i = 0; i < 5; i++) {
    const before = out
    out = out.replace(/[;\s]+$/, '')
    out = out.replace(/(?:--[^\n]*|\/\*[\s\S]*?\*\/)\s*$/, '').trim()
    if (out === before) break
  }
  return out
}

export function findTopLevelTail(sql) {
  const s = String(sql || '')
  const n = s.length
  let i = 0
  let paren = 0
  let quote = 0
  let line = false
  let block = false
  let dollar = null
  let orderByStart = -1
  let limitStart = -1
  let prevWord = ''
  let prevWordStart = -1

  const isWordChar = (ch) => !!ch && /[A-Za-z0-9_$]/.test(ch)

  while (i < n) {
    const ch = s[i]
    const next = i + 1 < n ? s[i + 1] : ''

    if (line) { if (ch === '\n') line = false; i++; continue }
    if (block) { if (ch === '*' && next === '/') { block = false; i += 2 } else i++; continue }
    if (dollar) {
      if (s.startsWith(dollar, i)) { i += dollar.length; dollar = null } else i++
      continue
    }
    if (quote) {
      if (ch === quote) {
        if (next === quote) i += 2
        else { quote = 0; i++ }
      } else i++
      continue
    }

    if (ch === '-' && next === '-') { line = true; i += 2; continue }
    if (ch === '/' && next === '*') { block = true; i += 2; continue }
    if (ch === '\'' || ch === '"' || ch === '`') { quote = ch; i++; continue }
    // PG 的美元引号（$$ … $$ / $body$ … $body$）：里面的 order by 是函数体，不是顶层
    if (ch === '$') {
      let j = i + 1
      while (j < n && /[A-Za-z0-9_]/.test(s[j])) j++
      if (j < n && s[j] === '$') {
        dollar = s.substring(i, j + 1)
        i = j + 1
        continue
      }
    }
    if (ch === '(') { paren++; prevWord = ''; i++; continue }
    if (ch === ')') { paren = Math.max(0, paren - 1); prevWord = ''; i++; continue }

    if (isWordChar(ch)) {
      const start = i
      let w = ''
      while (i < n && isWordChar(s[i])) { w += s[i]; i++ }
      const up = w.toUpperCase()
      if (paren === 0) {
        // ORDER 后面紧跟 BY 才算（`order` 也可能是列名的一部分，但顶层裸写 order by 就是子句）
        if (prevWord === 'ORDER' && up === 'BY' && orderByStart < 0) orderByStart = prevWordStart
        // LIMIT / OFFSET / FETCH FIRST … ROWS ONLY：新的 ORDER BY 必须插在它们前面
        if ((up === 'LIMIT' || up === 'OFFSET' || up === 'FETCH') && limitStart < 0) limitStart = start
      }
      prevWord = up
      prevWordStart = start
      continue
    }
    if (!/\s/.test(ch)) { prevWord = ''; prevWordStart = -1 }
    i++
  }

  return { orderByStart, limitStart }
}

/**
 * 把表头点出来的排序键塞进用户那条 SQL 的**顶层** ORDER BY（作用于全部结果），
 * 而不是排当前页。规则：
 *   - 原来有顶层 ORDER BY ⇒ 整段替换（点表头就是要覆盖它）
 *   - 只有 LIMIT / OFFSET / FETCH ⇒ 插到它们前面（ORDER BY 必须在前）
 *   - 都没有 ⇒ 追加到末尾
 * 子查询里的 order by、字符串/注释里的 "order by" 都不会被误伤（见 findTopLevelTail）。
 *
 * @param {string} sql 用户原始 SQL
 * @param {Array<{col:string,dir:string}>} keys 排序键（方向是 'ASC'/'DESC' 大写，见 utils/multiSort.js；空 = 取消排序）
 */
export function applyTableSort(sql, keys) {
  const base = stripTailSemicolon(sql)
  const list = Array.isArray(keys) ? keys.filter((k) => k && k.col) : []
  if (!base || !list.length) return base
  // 注意：multiSort 的 dir 是 'ASC' / 'DESC'（大写），这里必须大小写不敏感，否则降序永远拼不出来
  const orderBy = 'order by ' + list
    .map((k) => `${k.col} ${/^desc$/i.test(String(k.dir || '')) ? 'desc' : 'asc'}`)
    .join(', ')
  const { orderByStart, limitStart } = findTopLevelTail(base)
  if (orderByStart >= 0) {
    const end = limitStart > orderByStart ? limitStart : base.length
    return (base.slice(0, orderByStart) + orderBy + ' ' + base.slice(end)).trim()
  }
  if (limitStart >= 0) {
    return (base.slice(0, limitStart) + orderBy + ' ' + base.slice(limitStart)).trim()
  }
  return base + '\n' + orderBy
}
