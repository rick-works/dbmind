// SQL 格式化公共入口：统一承载 sql-formatter 配置，并支持把「自定义关键字」
// 注入所选方言的同名关键字集合，从而决定该关键字的换行/缩进/大小写行为。
import {
  format, formatDialect,
  bigquery, clickhouse, db2, db2i, duckdb, hive, mariadb, mysql, tidb, n1ql,
  plsql, postgresql, redshift, spark, sqlite, sql, trino, transactsql,
  singlestoredb, snowflake
} from 'sql-formatter'

// 方言名 → sql-formatter 方言对象（与设置面板下拉保持一致）
const dialectMap = {
  bigquery, clickhouse, db2, db2i, duckdb, hive, mariadb, mysql, tidb, n1ql,
  plsql, postgresql, redshift, spark, sqlite, sql, trino, transactsql,
  singlestoredb, snowflake
}

// 主方言解析失败时按此顺序尝试其余方言（如标准 SQL 不识别 [表名] 方括号，改用 T-SQL 可成功）。
const DIALECT_FALLBACKS = ['transactsql', 'postgresql', 'mysql', 'mariadb', 'plsql', 'sqlite', 'db2', 'clickhouse']

// 数据库连接类型码 → sql-formatter 方言（用于查询编辑器按连接自动识别；未列出的类型返回 '' → 使用标准 SQL）
export const connDialectOf = (typeCode) => ({
  MYSQL: 'mysql', MARIADB: 'mariadb', POSTGRESQL: 'postgresql', SQLSERVER: 'transactsql',
  ORACLE: 'plsql', SQLITE: 'sqlite', DB2: 'db2', CLICKHOUSE: 'clickhouse',
  DORIS: 'mysql', KINGBASE: 'postgresql', DM: 'plsql'
})[String(typeCode || '').toUpperCase()] || ''

// 拆分为关键字数组：支持英文/中文逗号、分号、换行分隔；多词短语（如 LEFT JOIN）请用空格连接
export const splitKeywords = (text) =>
  String(text || '')
    .split(/[,，;；\n\r]+/)
    .map((s) => s.trim())
    .filter(Boolean)
    .filter((s, i, arr) => arr.indexOf(s) === i)

const norm = (v, fallback) => (v === undefined || v === null || v === '' ? fallback : v)

// 由应用设置项构建 sql-formatter 的通用 FormatOptions
export const buildSqlFormatOptions = (s) => ({
  keywordCase: norm(s.sqlKeywordCase, 'upper'),
  identifierCase: norm(s.sqlIdentifierCase, 'preserve'),
  dataTypeCase: norm(s.sqlDataTypeCase, 'preserve'),
  functionCase: norm(s.sqlFunctionCase, 'preserve'),
  indentStyle: norm(s.sqlIndentStyle, 'standard'),
  tabWidth: norm(s.sqlTabWidth, 2),
  useTabs: !!s.sqlUseTabs,
  expressionWidth: norm(s.sqlExpressionWidth, 50),
  linesBetweenQueries: typeof s.sqlLinesBetweenQueries === 'number' ? s.sqlLinesBetweenQueries : 2,
  logicalOperatorNewline: norm(s.sqlLogicalOperatorNewline, 'before'),
  newlineBeforeSemicolon: !!s.sqlNewlineBeforeSemicolon,
  denseOperators: !!s.sqlDenseOperators
})

// 排版规则数据模型：sqlKeywordRules = [{ kw: 关键字/短语, cs: upper=大写 | lower=小写 | keep=保持原样, ly: 排版方式 }]
// ly 各值：
//   none=内联（不换行）；clause/join/setop=对应 sql-formatter 内置关键字角色（前换行 / 前换行+缩进 / 前后各换行）；
//   afterBreak/afterBreakIndent=先按内联排版，再在关键字后断行（后换行 / 后换行+缩进，由 applyBreakAfter 后处理）。
//
// 注意 `clause`（前换行）：sql-formatter 的 reservedClauses 角色**除前换行外**还会把该子句的
// 内容整体挪到下一行并缩进（`FROM\n  users u`）。标签只承诺「前换行」，用户看到的是「前后各换行」——
// 所以这里用 applyBreakBeforeOnly 把「关键字独占一行」的情况收回成 `FROM users u`。
// 旧版 spaceBefore/spaceAfter/spaceAround（加空格）与 none 输出相同已下线，历史数据读取时一律视作 none。
const LY_LAYOUT = {
  none: null,
  afterBreak: null, afterBreakIndent: null,
  clause: 'reservedClauses', join: 'reservedJoins', setop: 'reservedSetOperations'
}
const LEGACY_MODE_LY = { kw: 'none', clause: 'clause', join: 'join', setop: 'setop' }
const RULE_BUCKETS = ['reservedKeywords', 'reservedClauses', 'reservedJoins', 'reservedSetOperations']

// 读取排版规则（兼容 v2 旧结构 { kw, mode }，mode 自动映射为 cs=upper + 对应 ly）
const rulesOf = (s) => {
  const list = Array.isArray(s.sqlKeywordRules) ? s.sqlKeywordRules : []
  const out = []
  for (const r of list) {
    const kw = r && String(r.kw || '').trim()
    if (!kw) continue
    const ly = r && Object.prototype.hasOwnProperty.call(LY_LAYOUT, r.ly)
      ? r.ly
      : (LEGACY_MODE_LY[r && r.mode] || 'none')
    const cs = (r && r.cs) === 'keep' ? 'keep' : (r && r.cs) === 'lower' ? 'lower' : 'upper'
    out.push({ kw: kw.toUpperCase(), cs, ly })
  }
  return out
}

// 把「排版规则」作用到所选方言的关键字集合上：
// 1) 先把规则里的词从各内置集合中移除（避免双重身份，也支持「保持原样=不当作关键字」）；
// 2) 再按排版方式放回对应集合：换行类 → clause/join/setop 集合；
//    内联类（不换行）→ 大写则放入普通关键字集合（内联大写），保持原样则不放入。
const applyRuleKeywords = (dialect, rules) => {
  const t = dialect.tokenizerOptions || {}
  const arr = (v) => (Array.isArray(v) ? v : [])
  const buckets = {
    reservedKeywords: arr(t.reservedKeywords).slice(),
    reservedClauses: arr(t.reservedClauses).slice(),
    reservedJoins: arr(t.reservedJoins).slice(),
    reservedSetOperations: arr(t.reservedSetOperations).slice()
  }
  for (const r of rules) {
    for (const key of RULE_BUCKETS) {
      buckets[key] = buckets[key].filter((w) => String(w).toUpperCase() !== r.kw)
    }
    const target = LY_LAYOUT[r.ly]
    if (r.cs === 'keep') {
      // 保持原样：非换行类不放入任何集合，引擎视作普通词按输入原样保留；
      // 换行类仍需放入对应集合才能获得换行，大小写再由 applyKeepCase 还原。
      if (target) buckets[target].push(r.kw)
    } else {
      // 大写 / 小写：都作为关键字获得与内置关键字一致的排版；大小写由 keywordCase 或后处理决定。
      if (target) buckets[target].push(r.kw)
      else buckets.reservedKeywords.push(r.kw)
    }
  }
  return {
    ...t,
    reservedClauses: buckets.reservedClauses,
    reservedJoins: buckets.reservedJoins,
    reservedSetOperations: buckets.reservedSetOperations,
    reservedKeywords: buckets.reservedKeywords
  }
}

const escapeReg = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

// 引号字符串保护占位（轻量格式化用）
const QUOTED_RE = /(['"`])(?:\\\1|.)*?\1/g

// 轻量格式化：当 sql-formatter 解析失败（如 SQL 不完整）时的降级处理。
// 只做保守的大小写转换与空白规范化，避免破坏未完成的语句。
const formatSqlLite = (text, s) => {
  const kwCase = norm(s.sqlKeywordCase, 'upper')
  if (!text || !text.trim()) return text
  const quotes = []
  const protectedText = String(text).replace(QUOTED_RE, (m) => {
    quotes.push(m)
    return `\0${quotes.length - 1}\0`
  })
  // 行内多个空白合并，去除行首行尾空白
  let out = protectedText
    .replace(/[ \t]+/g, ' ')
    .replace(/^[ \t]+|[ \t]+$/gm, '')
  // 关键字大小写
  if (kwCase !== 'preserve') {
    const kwList = keywordCandidates('sql')
    if (kwList.length) {
      const kwPattern = new RegExp('\\b(' + kwList.map(escapeReg).join('|') + ')\\b', 'gi')
      out = out.replace(kwPattern, (m) => (kwCase === 'upper' ? m.toUpperCase() : m.toLowerCase()))
    }
  }
  // 恢复引号字符串
  if (quotes.length) {
    out = out.replace(/\0(\d+)\0/g, (_, i) => quotes[Number(i)])
  }
  return out.replace(/\n[ \t]+/g, '\n').trim()
}

// 换行类关键字由全局 keywordCase 统一大写；要让某个关键字「保持原样」，
// 在格式化结果里把它还原成输入 SQL 里的实际写法（只影响该词，不波及其它关键字）。
const phrasePat = (kw) => (/[A-Za-z0-9_]/.test(kw) && !/\s/.test(kw))
  ? '\\b' + escapeReg(kw) + '\\b'
  : escapeReg(kw).replace(/ /g, '\\s+')
const applyKeepCase = (output, input, rules) => {
  let out = output
  for (const r of rules) {
    if (r.cs !== 'keep' || !LY_LAYOUT[r.ly]) continue
    const pat = phrasePat(r.kw)
    const inMatch = new RegExp(pat, 'i').exec(input)
    if (!inMatch) continue
    const want = inMatch[0]
    const outPat = phrasePat(want.toUpperCase())
    out = out.replace(new RegExp(outPat, 'g'), want)
  }
  return out
}

// 「小写」关键字：作为关键字参与排版后，在结果里把它统一还原为小写（只影响该词）。
const applyLowerCase = (output, rules) => {
  let out = output
  for (const r of rules) {
    if (r.cs !== 'lower') continue
    const low = r.kw.toLowerCase()
    const outPat = phrasePat(r.kw)
    out = out.replace(new RegExp(outPat, 'gi'), low)
  }
  return out
}

// 「大写」关键字：规则显式要求大写时，不受全局关键字大小写（如统一小写/保持原样）影响，
// 在结果里把它统一还原为大写（只影响该词）。
const applyUpperCase = (output, rules) => {
  let out = output
  for (const r of rules) {
    if (r.cs !== 'upper') continue
    const up = r.kw.toUpperCase()
    const outPat = phrasePat(r.kw)
    out = out.replace(new RegExp(outPat, 'gi'), up)
  }
  return out
}

// 「后换行」排版：关键字先按内联排版，再在同一行关键字之后断行，把后续内容移到新行。
// 后换行=与关键字所在行同缩进；后换行+缩进=再向下一级缩进。
const applyBreakAfter = (text, rules, tabWidth) => {
  let out = String(text)
  const targets = rules.filter((r) => r.ly === 'afterBreak' || r.ly === 'afterBreakIndent')
  for (const r of targets) {
    const re = new RegExp(phrasePat(r.kw), 'i')
    const lines = out.split('\n')
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i]
      const m = re.exec(line)
      if (!m) continue
      const after = line.slice(m.index + m[0].length)
      if (!after.trim()) continue
      const indent = /^\s*/.exec(line)[0]
      const extra = r.ly === 'afterBreakIndent' ? ' '.repeat(Math.max(0, tabWidth || 2)) : ''
      lines[i] = line.slice(0, m.index + m[0].length) + '\n' + indent + extra + after.trimStart()
    }
    out = lines.join('\n')
  }
  return out
}

// 「前换行」只该在关键字**前**断行：把「关键字独占一行」时紧随其后的内容接回关键字后面。
//
// 为什么需要它：`clause` 走的是 sql-formatter 的 reservedClauses 角色，该角色在换行之后还会
// 把子句内容另起一行并缩进（实测 `FROM\n  users u`），于是「前换行」看起来成了「前后各换行」。
// 只并**紧邻的那一行**、且仅当关键字行除了关键字什么都没有时动手 —— 这样多表子句
// （`FROM users u,` + 缩进续行 / 后续 JOIN 等）不会被动，缩进层级也保持引擎给的样子。
const applyBreakBeforeOnly = (text, rules) => {
  const targets = rules.filter((r) => r.ly === 'clause')
  if (!targets.length) return text
  let lines = String(text).split('\n')
  for (const r of targets) {
    const alone = new RegExp('^\\s*' + phrasePat(r.kw) + '\\s*$', 'i')
    const out = []
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i]
      // 下一行必须有内容才并：空行（用户要求的关键字间空行）保持原样
      if (alone.test(line) && i + 1 < lines.length && lines[i + 1].trim()) {
        out.push(line.replace(/\s+$/, '') + ' ' + lines[i + 1].trim())
        i++
        continue
      }
      out.push(line)
    }
    lines = out
  }
  return lines.join('\n')
}

// 格式化 SQL：若配置了关键字排版规则，则克隆所选方言并注入对应集合，
// 让用户关键字获得与内置同类关键字一致的排版/大小写行为；失败时回退到普通 format。
// language 为 sql-formatter 方言名（如 mysql/transactsql），缺省用标准 SQL。
export const formatSql = (text, s, language) => {
  const primary = norm(language, 'sql')
  const options = buildSqlFormatOptions(s)
  const rules = rulesOf(s)
  const run = (language) => {
    const base = dialectMap[language] || sql
    if (rules.length) {
      return formatDialect(text, { ...options, dialect: { ...base, tokenizerOptions: applyRuleKeywords(base, rules) } })
    }
    return format(text, { ...options, language })
  }
  // 主方言失败时按优先级尝试其余方言（方括号等特殊语法提升容错）；
  // 全部失败则降级为轻量格式化（至少完成关键字大小写与空白规范化）。
  const candidates = [primary, ...DIALECT_FALLBACKS.filter((l) => l !== primary)]
  let out
  let firstErr
  for (const language of candidates) {
    try { out = run(language); break } catch (e) { if (!firstErr) firstErr = e }
  }
  if (out === undefined && firstErr) out = formatSqlLite(text, s)
  const keepRules = rules.filter((r) => r.cs === 'keep' && LY_LAYOUT[r.ly])
  if (keepRules.length) out = applyKeepCase(out, text, keepRules)
  const lowerRules = rules.filter((r) => r.cs === 'lower')
  if (lowerRules.length) out = applyLowerCase(out, lowerRules)
  const upperRules = rules.filter((r) => r.cs === 'upper')
  if (upperRules.length) out = applyUpperCase(out, upperRules)
  if (rules.some((r) => r.ly === 'clause')) {
    out = applyBreakBeforeOnly(out, rules)
  }
  if (rules.some((r) => r.ly === 'afterBreak' || r.ly === 'afterBreakIndent')) {
    out = applyBreakAfter(out, rules, options.tabWidth)
  }
  return out
}

// 常用关键字（放在候选表最前面）
const COMMON_KEYWORDS = [
  'SELECT', 'FROM', 'WHERE', 'HAVING', 'ORDER BY', 'GROUP BY', 'LIMIT', 'OFFSET',
  'JOIN', 'INNER JOIN', 'LEFT JOIN', 'RIGHT JOIN', 'FULL JOIN', 'CROSS JOIN', 'OUTER JOIN', 'ON', 'AS', 'USING',
  'UNION', 'UNION ALL', 'INTERSECT', 'EXCEPT',
  'INSERT', 'INSERT INTO', 'UPDATE', 'DELETE', 'DELETE FROM', 'MERGE', 'INTO', 'VALUES', 'SET',
  'CREATE', 'CREATE TABLE', 'CREATE VIEW', 'ALTER', 'ALTER TABLE', 'DROP', 'DROP TABLE', 'DROP VIEW', 'TRUNCATE', 'VIEW', 'INDEX',
  'AND', 'OR', 'NOT', 'IN', 'IS', 'IS NULL', 'IS NOT NULL', 'NULL', 'BETWEEN', 'LIKE', 'ILIKE', 'EXISTS', 'ANY', 'ALL',
  'CASE', 'WHEN', 'THEN', 'ELSE', 'END', 'DISTINCT', 'ASC', 'DESC',
  'PRIMARY KEY', 'FOREIGN KEY', 'REFERENCES', 'DEFAULT', 'UNIQUE', 'CHECK', 'CONSTRAINT',
  'WITH', 'RECURSIVE', 'OVER', 'PARTITION BY'
]

// 关键字候选：常用关键字放最前，其余来自所选方言的内置关键字（去重后按字母排序）
const CATALOG_KEYS = ['reservedSelect', 'reservedClauses', 'reservedSetOperations', 'reservedJoins', 'reservedKeywordPhrases', 'reservedKeywords']
const catalogCache = new Map()
export const keywordCandidates = (language) => {
  const lang = norm(language, 'sql')
  if (catalogCache.has(lang)) return catalogCache.get(lang)
  const dialect = dialectMap[lang] || sql
  const t = dialect.tokenizerOptions || {}
  const set = new Set()
  for (const key of CATALOG_KEYS) {
    for (const w of t[key] || []) set.add(String(w).toUpperCase())
  }
  const rest = [...set].filter((w) => !COMMON_KEYWORDS.includes(w)).sort()
  const list = [...new Set([...COMMON_KEYWORDS, ...rest])]
  catalogCache.set(lang, list)
  return list
}

// 用于「效果预览」关键字高亮：构造匹配所选方言关键字（含自定义关键字）的正则，
// 统一按单词匹配（多词短语拆成单词），并对引号内的文本加前缀/后缀断言避免误伤字符串。
const escRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
export const sqlKeywordPattern = (language, extra = []) => {
  const words = new Set()
  const add = (w) => { const s = String(w).trim(); if (s) s.split(/\s+/).forEach((p) => { if (p) words.add(p.toUpperCase()) }) }
  keywordCandidates(language).forEach(add)
  ;(Array.isArray(extra) ? extra : []).forEach(add)
  const pat = [...words].map(escRe).sort((a, b) => b.length - a.length).join('|')
  return new RegExp("(?<!['\"])\\b(?:" + pat + ")\\b(?!['\"])", 'gi')
}
