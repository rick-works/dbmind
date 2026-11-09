// ===== 模板变量的「类型感知字面量」生成 =====
//
// 背景（2026-11-09 用户实测）：ClickHouse 里 `WHERE vbeln = :vbeln`，列是 String，
// 变量填 1110057272 → 替换成数字字面量 → 报
//   Code 386: There is no supertype for types String, UInt32 (NO_COMMON_TYPE)。
// 字面量的形态应该由**目标列的类型**决定，而不是用户输入的长相。
//
// 三层决策（优先级从高到低）：
//   1. 变量库里显式声明的类型（string / number / bool）—— 用户意志最高；
//   2. `:var` 前面的列（`col = :var` / `col in (:var, :var2)`）按列元数据分类；
//   3. 输入形态推断（auto 兜底，兼容旧口径：数字/NULL/函数/关键字原样，其余字符串）。
//
// 纯函数、零依赖 —— node 直测（复制为 .mjs）与 UI 共用同一实现。

// 数据库函数（NOW()、COALESCE(x,y) 这类「名(…)」形状）与 CURRENT_* 类无括号关键字：
// 用户写的是 SQL 表达式，任何类型声明下都原样写入（与变量库默认值同口径）。
const RAW_KEYWORDS_RE = /^(current_(date|time|timestamp|user|schema|catalog)|localtime|localtimestamp|sysdate|systimestamp|today|now)$/i
const NUM_VALUE_RE = /^-?\d+(\.\d+)?$/
const BOOL_TRUE_RE = /^(true|1|yes|y|是)$/i
const BOOL_FALSE_RE = /^(false|0|no|n|否)$/i

/** 是否为「SQL 表达式形状」的值（原样写入，不加热引号） */
export function isRawSqlExpr (v) {
  const s = String(v == null ? '' : v).trim()
  if (!s) return false
  if (RAW_KEYWORDS_RE.test(s)) return true
  // 函数形状：NOW()、length(x)、COALESCE(a, b) —— 括号必须配平到结尾
  return /^[A-Za-z_][\w$]*\s*\(.*\)$/.test(s)
}

/** auto 兜底的输入形态：number（数字原样）/ raw（NULL/函数原样）/ string（字符串字面量） */
export function inferKind (v) {
  const s = String(v == null ? '' : v).trim()
  if (!s) return ''
  if (/^null$/i.test(s)) return 'raw'
  if (NUM_VALUE_RE.test(s)) return 'number'
  if (isRawSqlExpr(s)) return 'raw'
  return 'string'
}

/** 剥掉 ClickHouse 的 Nullable(x) / LowCardinality(x) 包装再分类 */
const unwrapType = (t) => {
  let s = String(t || '').trim()
  for (;;) {
    const m = /^(nullable|lowcardinality)\s*\((.*)\)$/i.exec(s)
    if (!m) break
    s = m[2].trim()
  }
  return s
}

/**
 * 列类型名 → 字面量形态：'number' | 'bool' | 'string'（空/未知 → 'string'）。
 *
 * 数字要防误伤：PG 的 `interval`（前缀 int）不是数字——数字词必须整词命中；
 * ClickHouse 的 UInt32/Int64/Decimal(p,s)、MySQL 的 int unsigned、标准 SQL 的
 * decimal(10,2) 都要归到 number。其余（字符串/日期/时间/UUID/ENUM/JSON/二进制…）
 * 统一按字符串字面量生成 —— 所有方言里 `col = '…'` 都合法，日期时间也一样。
 */
export function classifyColType (typeName) {
  const t = unwrapType(typeName)
  if (!t) return 'string'
  if (/^bool/i.test(t)) return 'bool'
  const low = t.toLowerCase()
  // 整词匹配数字类型（\b 挡掉 interval/internal 这类前缀陷阱；u?int\d* 覆盖 int2/int4/int8）
  if (/^(u?int\d*|integer|smallint|tinyint|mediumint|bigint|float\d*|double(\s+precision)?|real|decimal|numeric|number|money|bit(\s+varying)?|serial\d*|bigserial|smallserial)\b/.test(low)) return 'number'
  return 'string'
}

// 字符串字面量里反斜杠是转义符的方言：值里带 \ 会把收尾引号吃掉
// （`'C:\temp\'` —— 与后端就地编辑同源的问题），这些方言把 \ 一起翻倍。
const BACKSLASH_DIALECTS = new Set(['mysql', 'mariadb', 'doris', 'clickhouse'])

/**
 * 用户输入 → SQL 字面量。
 * @param kind 'string' | 'number' | 'bool' | 'auto'
 * @returns 字面量字符串；kind=number/bool 且输入不合法时返回 null（调用方决定降级）
 */
export function buildLiteral (value, kind, dialect = '') {
  const s = String(value == null ? '' : value).trim()
  if (!s) return null
  if (/^null$/i.test(s)) return 'NULL'
  // 函数 / CURRENT_* 类：用户写的是 SQL 表达式，任何 kind 下都原样
  if (isRawSqlExpr(s)) return s
  let k = kind
  if (k === 'auto' || !k) k = inferKind(s)
  if (k === 'number') {
    if (NUM_VALUE_RE.test(s)) return s
    return null
  }
  if (k === 'bool') {
    if (BOOL_TRUE_RE.test(s)) return 'TRUE'
    if (BOOL_FALSE_RE.test(s)) return 'FALSE'
    return null
  }
  const q = s.replace(/'/g, "''")
  return BACKSLASH_DIALECTS.has(String(dialect || '').toLowerCase())
    ? `'${q.replace(/\\/g, '\\\\')}'`
    : `'${q}'`
}

// —— `:var` 前缀的列上下文 ——
// `col in (` → 列表展开（括号已有）；`col in` → 列表展开（补括号）；`col =` → 单值。
const CTX_LIST_PAREN = /([A-Za-z_][\w$]*(?:\s*\.\s*[A-Za-z_][\w$]*)*)\s+(?:not\s+in|in)\s*\($/i
const CTX_LIST_BARE = /([A-Za-z_][\w$]*(?:\s*\.\s*[A-Za-z_][\w$]*)*)\s+(?:not\s+in|in)$/i
const CTX_CMP = /([A-Za-z_][\w$]*(?:\s*\.\s*[A-Za-z_][\w$]*)*)\s*(?:=|<>|!=|<=|>=|<|>|like|ilike)$/i
const CTX_IN_LIST_SPLIT = /,/

const lastIdent = (ref) => {
  const parts = String(ref || '').replace(/\s+/g, '').split('.')
  return (parts[parts.length - 1] || '').toLowerCase()
}

/** `:var` 左侧文本 → { col, list: 'paren'|'bare'|false }；无法关联列 → null */
export function varContextOf (before) {
  const b = String(before || '').replace(/\s+$/, '')
  let m = CTX_LIST_PAREN.exec(b)
  if (m) return { col: lastIdent(m[1]), list: 'paren' }
  m = CTX_LIST_BARE.exec(b)
  if (m) return { col: lastIdent(m[1]), list: 'bare' }
  m = CTX_CMP.exec(b)
  if (m) return { col: lastIdent(m[1]), list: false }
  return null
}

/**
 * 执行前的变量替换（类型感知版）。
 *
 * @param values { 变量名: 用户输入 }（键按原名；大小写不敏感兜底）
 * @param opts.dialect   连接方言（connDialectOf 的值：mysql/clickhouse/…），决定转义口径
 * @param opts.colTypes  { 变量名小写: 'number'|'bool'|'string' } —— 由 `:var` 前面的列查元数据得出
 * @param opts.varTypes  { 变量名小写: 'number'|'bool'|'string' } —— 变量库里显式声明的类型，优先级最高
 * @param opts.onWarn    (变量名小写) => void —— 输入无法按目标类型解析时的回调（降级成字符串字面量）
 */
export function replaceVars (text, values, opts = {}) {
  const { dialect = '', colTypes = {}, varTypes = {}, onWarn } = opts
  const vals = values || {}
  const lookup = (name) => {
    if (Object.prototype.hasOwnProperty.call(vals, name)) return vals[name]
    const low = name.toLowerCase()
    if (Object.prototype.hasOwnProperty.call(vals, low)) return vals[low]
    return undefined
  }
  const kindOf = (name, before) => {
    const key = name.toLowerCase()
    const explicit = String(varTypes[key] || '').toLowerCase()
    if (explicit === 'number' || explicit === 'bool' || explicit === 'string') return explicit
    const ctx = varContextOf(before)
    if (ctx && colTypes[ctx.col]) return colTypes[ctx.col]
    return 'auto'
  }
  const lit = (v, kind) => {
    const out = buildLiteral(v, kind, dialect)
    if (out !== null) return out
    // 非法（如数字列填了非数字、布尔列填了自由文本）：降级成字符串字面量并警告 ——
    // 别静默产出坏 SQL，也别把用户的执行直接打断
    if (onWarn) onWarn(String(v).trim())
    return buildLiteral(v, 'string', dialect)
  }
  const replaceSeg = (seg) => seg.replace(/(?<![:@\w]):([a-z_][\w$]*)/gi, (whole, name, offset) => {
    const raw = lookup(name)
    const s = String(raw == null ? '' : raw).trim()
    if (raw === undefined || !s) return whole // 没值：占位符原样保留（既有行为）
    if (/^null$/i.test(s)) return 'NULL'
    if (isRawSqlExpr(s)) return s // 函数 / CURRENT_*：表达式原样
    const kind = kindOf(name, seg.slice(0, offset))
    const ctx = varContextOf(seg.slice(0, offset))
    if (ctx && ctx.list) {
      // IN 列表：`in (:v1, :v2)` 逐个转换；`in (:v)` 值里带逗号也展开成多值列表
      const parts = s.split(CTX_IN_LIST_SPLIT).map((x) => x.trim()).filter(Boolean)
      if (!parts.length) return whole
      const body = parts.map((p) => lit(p, kind)).join(', ')
      return ctx.list === 'paren' ? body : `(${body})`
    }
    return lit(s, kind)
  })
  // 引号外扫描：'...' / "..." / `...` / [...] 内的内容原样保留（与编辑器 mapOutsideQuotes 同口径）
  let out = ''
  let i = 0
  while (i < text.length) {
    const ch = text[i]
    if (ch === "'" || ch === '"' || ch === '`') {
      let j = i + 1
      while (j < text.length) {
        if (text[j] === ch) {
          if (text[j + 1] === ch) { j += 2; continue } // '' / "" 转义
          break
        }
        j++
      }
      out += text.slice(i, Math.min(j + 1, text.length))
      i = j + 1
    } else if (ch === '[') {
      const j = text.indexOf(']', i)
      if (j < 0) { out += replaceSeg(text.slice(i)); break }
      out += text.slice(i, j + 1)
      i = j + 1
    } else {
      let j = i
      while (j < text.length && text[j] !== "'" && text[j] !== '"' && text[j] !== '`' && text[j] !== '[') j++
      out += replaceSeg(text.slice(i, j))
      i = j
    }
  }
  return out
}
