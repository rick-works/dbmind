// SQL 格式化公共入口：统一承载 sql-formatter 配置，并支持把「自定义关键字」
// 注入所选方言的同名关键字集合，从而决定该关键字的换行/缩进/大小写行为。
import {
  format, formatDialect,
  bigquery, clickhouse, db2, db2i, duckdb, hive, mariadb, mysql, tidb, n1ql,
  plsql, postgresql, redshift, spark, sqlite, sql, trino, transactsql,
  singlestoredb, snowflake
} from 'sql-formatter'
import { splitSqlStatementRanges } from './sqlSplit.js'

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

const norm = (v, fallback) => (v === undefined || v === null || v === '' ? fallback : v)

// 「查询之间空行 N 行」——设置里这个值数的是**空行**，而 sql-formatter 的
// linesBetweenQueries 数的是**换行**：N 个空行 = N+1 个换行（0 空行时两条语句紧挨着各占一行）。
// 以前直接把设置值丢给引擎，导致选「1 行」反而一个空行都没有（真机反馈：格式化后空行没了）。
const blankLinesOf = (s) => {
  const n = Number(s && s.sqlLinesBetweenQueries)
  return Number.isFinite(n) ? Math.max(0, Math.min(9, Math.round(n))) : 1
}

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
  linesBetweenQueries: blankLinesOf(s) + 1,
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

// 哪些排版方式"带缩进"—— 只有它们用得上关键字级缩进宽度（规则里的 iw）。
//   join            前换行 + 缩进：关键字整行按一级缩进（引擎给的是全局 tabWidth，规则配了 iw 就按 iw 换算）
//   afterBreakIndent 后换行 + 缩进：关键字后的内容再向下一级缩进（这一级用 iw）
// 其余（内联 / 前换行 / 后换行 / 前后各换行）都不产生额外缩进，设置界面里那一列会置灰。
export const INDENT_LAYOUTS = ['join', 'afterBreakIndent']
export const ruleHasIndent = (ly) => INDENT_LAYOUTS.includes(ly)
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
    // iw：关键字级缩进宽度，null = 未配置（跟随全局 tabWidth）。
    // ⚠️ 0 是**有效值**（该关键字不缩进），不能拿 0 当"未配置"的哨兵 —— 两者必须分开。
    const n = r && r.iw !== undefined && r.iw !== null && String(r.iw).trim() !== '' ? Math.round(Number(r.iw)) : NaN
    const iw = ruleHasIndent(ly) && Number.isFinite(n) && n >= 0 && n <= 12 ? n : null
    out.push({ kw: kw.toUpperCase(), cs, ly, iw })
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
      // 缩进宽度：这条规则单独配了 iw 就用它（0 也算配了 = 不缩进），否则跟随全局 tabWidth（默认 2）
      const width = Number.isFinite(r.iw) ? r.iw : (tabWidth || 2)
      const extra = r.ly === 'afterBreakIndent' ? ' '.repeat(Math.max(0, width)) : ''
      lines[i] = line.slice(0, m.index + m[0].length) + '\n' + indent + extra + after.trimStart()
    }
    out = lines.join('\n')
  }
  return out
}

// 「前换行 + 缩进」的缩进是引擎按**全局** tabWidth 给的（整行缩进一级）。规则里单独配了缩进宽度时，
// 按**层级**换算过去：level × iw —— 这样嵌套里的同名关键字也跟着自己的宽度走，层级关系不被打乱
// （例如子查询里的 FROM 仍比外层深一级）。
// 只认「关键字出现在行首」的行：引擎给这种排版的写法本来就是关键字起头，不会误伤表达式里的同名词。
const applyJoinIndent = (text, rules, tabWidth) => {
  const targets = rules.filter((r) => r.ly === 'join' && Number.isFinite(r.iw))
  if (!targets.length) return text
  const base = Math.max(1, tabWidth || 2)
  let out = String(text)
  for (const r of targets) {
    const re = new RegExp('^([ \\t]*)' + phrasePat(r.kw), 'i')
    out = out
      .split('\n')
      .map((line) => {
        const m = re.exec(line)
        if (!m) return line
        const level = Math.round(m[1].length / base)
        if (level < 1) return line
        return ' '.repeat(level * r.iw) + line.slice(m[1].length)
      })
      .join('\n')
  }
  return out
}

// 「逗号位置」：sql-formatter 15.8.2 **没有**这个选项（FormatOptions 里只有那 14 个），
// 所以只能在引擎输出之后按行搬动逗号。
//   after（默认）字段后：`a,` 换行 `b`  —— 引擎原本的输出就是这样，直接不动
//   before      字段前：`a`   换行 `, b`
// 只处理「行尾逗号 + 下一行是内容行」；下一行以右括号开头时不搬 —— 否则会排出 `, )` 这种一眼错的形状。
// 找出行内 `--` / `#` 行注释的起点（跳过字符串字面量里的假注释）；没有返回 -1
const findLineCommentIndex = (s) => {
  let inS = false
  let inD = false
  for (let i = 0; i < s.length; i++) {
    const ch = s[i]
    if (ch === "'" && !inD) inS = !inS
    else if (ch === '"' && !inS) inD = !inD
    else if (!inS && !inD && ch === '-' && s[i + 1] === '-') return i
    else if (!inS && !inD && ch === '#' && (i === 0 || /\s/.test(s[i - 1]))) return i
  }
  return -1
}

const applyCommaPosition = (text, mode) => {
  if (mode !== 'before' && mode !== 'after') return text
  const lines = String(text).split('\n')
  if (mode === 'before') {
    // 1) 逗号独占一行（sql-formatter 遇到「字段后跟行注释」时会这么排）→ 并入下一行行首
    for (let i = 0; i < lines.length - 1; i++) {
      if (lines[i].trim() !== ',') continue
      const next = lines[i + 1]
      const nt = next.trim()
      if (!nt || nt.startsWith(',')) continue
      lines[i + 1] = /^\s*/.exec(next)[0] + ', ' + nt
      lines.splice(i, 1)
      i--
    }
    // 2) 行尾逗号 → 下一行内容之前，缩进沿用该行
    for (let i = 0; i < lines.length - 1; i++) {
      const cur = lines[i].replace(/\s+$/, '')
      if (!cur.endsWith(',')) continue
      const next = lines[i + 1]
      const nt = next.trim()
      if (!nt || nt.startsWith(',') || /^[)\]}]/.test(nt)) continue
      lines[i] = cur.slice(0, -1).replace(/\s+$/, '')
      lines[i + 1] = /^\s*/.exec(next)[0] + ', ' + nt
    }
    // 3) 逗号贴在行注释**前**（`a, -- 测试1`，多半是上一次「字段后」的产物；
    //    行尾是注释不是逗号，所以判断的是「注释之前那段是否以逗号结尾」）→
    //    挪到下一行行首。**必须从后往前搬**：`a, -- 1` / `b, -- 2` / `c -- 3`
    //    顺序搬会连着动两行，倒着搬每行只搬自己的那一颗。
    for (let i = lines.length - 2; i >= 0; i--) {
      const cur = lines[i].replace(/\s+$/, '')
      const idx = findLineCommentIndex(cur)
      if (idx < 0) continue
      const head = cur.slice(0, idx).replace(/\s+$/, '')
      if (!head.endsWith(',')) continue
      const headClean = head.slice(0, -1).replace(/\s+$/, '')
      if (!headClean) continue
      const next = lines[i + 1]
      const nt = next.trim()
      if (!nt || nt.startsWith(',') || /^[)\]}]/.test(nt)) continue
      // 逗号搬走，**注释留在本行**（`1 a, -- 测试1` → `1 a -- 测试1`）——
      // 刚才第一版把注释整个丢了
      lines[i] = headClean + ' ' + cur.slice(idx).trim()
      lines[i + 1] = /^\s*/.exec(next)[0] + ', ' + nt
    }
    return lines.join('\n')
  }
  // after：把行首的逗号挪回**上一行字段的后面**。上一行带 `--` 行注释时必须
  // 插在注释**之前**（`a, -- 测试1`）—— 塞到注释后面会被注释吞掉，等于把下一列
  // 也注释掉（用户反馈："逗号应该贴在字段的后面，不是注释的后面"）。
  const out = [lines[0]]
  for (let i = 1; i < lines.length; i++) {
    const nt = lines[i].trim()
    if (!nt.startsWith(',')) {
      out.push(lines[i])
      continue
    }
    const prev = (out[out.length - 1] || '').replace(/\s+$/, '')
    // 上一行没内容（空行 / 开括号）就不动，避免拼出非法布局
    if (!prev || /[([{]$/.test(prev)) {
      out.push(lines[i])
      continue
    }
    const idx = findLineCommentIndex(prev)
    if (idx >= 0) {
      out[out.length - 1] = prev.slice(0, idx).replace(/\s+$/, '') + ', ' + prev.slice(idx).trim()
    } else {
      out[out.length - 1] = prev + ','
    }
    // 逗号后面还跟着内容（如 ", b"）就把剩下的留在本行；纯逗号行直接消失
    const rest = nt.slice(1).trim()
    if (rest) out.push(/^\s*/.exec(lines[i])[0] + rest)
  }
  return out.join('\n')
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

// sql-formatter 的已知怪癖：**行内 `-- 注释` 之后的下一行会被硬顶出一个空行**
// （select 列表逐列带注释时，每列之间都多一个空行，用户反馈"格式化会换行"）。
// 收敛：紧跟在「以 `--` 行注释结尾」的行后面的空行删掉。块注释（*/ 结尾）与
// 字符串里的 `--` 不受影响——前者另有排版，后者罕见且误删的代价只是少一个空行。
const endsWithLineComment = (line) => {
  const t = line.trim()
  if (!t || /\*\/\s*$/.test(t)) return false
  return /(^|\s)--(?!\s*$)/.test(t) || /^#/.test(t)
}
const collapseBlankAfterLineComment = (sql) => {
  const lines = sql.split('\n')
  const out = []
  for (let i = 0; i < lines.length; i++) {
    const prev = out[out.length - 1]
    if (lines[i].trim() === '' && prev && endsWithLineComment(prev)) {
      continue // 注释行后面 formatter 硬塞的空行，不要
    }
    out.push(lines[i])
  }
  return out.join('\n')
}

// 按**语句**切开脚本（用的是编辑器/后端同一套拆分器：没写分号也切）。
//
// 为什么必须自己切：sql-formatter 只认 `;` 断句。两条没写分号的 SELECT 会被它当成
// 一条查询 ⇒ 既不换行也不加空行，「查询之间空行」这个设置对无分号脚本完全失效。
// 切片时把两条语句之间的 `;`、空白、纯注释段都留在前一片里（不丢用户内容）。
const statementChunks = (text) => {
  const src = String(text || '')
  const ranges = splitSqlStatementRanges(src)
  if (ranges.length <= 1) return [src]
  return ranges.map((r, i) => {
    const end = i + 1 < ranges.length ? ranges[i + 1].start : src.length
    return src.slice(r.start, end)
  })
}

export const formatSql = (text, s, language) => {
  const primary = norm(language, 'sql')
  const options = buildSqlFormatOptions(s)
  const rules = rulesOf(s)
  const run = (input, language) => {
    const base = dialectMap[language] || sql
    if (rules.length) {
      return formatDialect(input, { ...options, dialect: { ...base, tokenizerOptions: applyRuleKeywords(base, rules) } })
    }
    return format(input, { ...options, language })
  }
  // 主方言失败时按优先级尝试其余方言（方括号等特殊语法提升容错）；
  // 全部失败则降级为轻量格式化（至少完成关键字大小写与空白规范化）。
  const candidates = [primary, ...DIALECT_FALLBACKS.filter((l) => l !== primary)]
  const formatOne = (input) => {
    let out
    let firstErr
    for (const language of candidates) {
      try { out = run(input, language); break } catch (e) { if (!firstErr) firstErr = e }
    }
    if (out === undefined && firstErr) out = formatSqlLite(input, s)
    const keepRules = rules.filter((r) => r.cs === 'keep' && LY_LAYOUT[r.ly])
    if (keepRules.length) out = applyKeepCase(out, input, keepRules)
    const lowerRules = rules.filter((r) => r.cs === 'lower')
    if (lowerRules.length) out = applyLowerCase(out, lowerRules)
    const upperRules = rules.filter((r) => r.cs === 'upper')
    if (upperRules.length) out = applyUpperCase(out, upperRules)
    if (rules.some((r) => r.ly === 'clause')) {
      out = applyBreakBeforeOnly(out, rules)
    }
    if (rules.some((r) => r.ly === 'join' && Number.isFinite(r.iw))) {
      out = applyJoinIndent(out, rules, options.tabWidth)
    }
    if (rules.some((r) => r.ly === 'afterBreak' || r.ly === 'afterBreakIndent')) {
      out = applyBreakAfter(out, rules, options.tabWidth)
    }
    // 全局后处理（与关键字规则无关，只看设置项）。放在最后：它按行搬动逗号，
    // 让上面那些按关键字搬行的规则先落定，避免互相干扰。
    out = applyCommaPosition(out, norm(s.sqlCommaPosition, 'after'))
    out = collapseBlankAfterLineComment(out)
    return out
  }
  // 单条语句：整段交给引擎（与以前完全一致）。
  const chunks = statementChunks(text)
  if (chunks.length <= 1) return formatOne(text)
  // 多条：逐条格式化，再用设置里的**空行数**拼回去（有无分号都一样）。
  const gap = '\n'.repeat(blankLinesOf(s) + 1)
  return chunks
    .map((chunk) => formatOne(chunk).trim())
    .filter((chunk) => chunk)
    .join(gap)
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
