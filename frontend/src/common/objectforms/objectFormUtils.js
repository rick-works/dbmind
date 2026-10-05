import { t } from '../../utils/i18n'
// ==================== 可视化对象表单共享工具 ====================
// 全部基于后端下发的 features（方言能力），前端不感知具体数据库类型

/** 是否支持某能力 */
export const has = (f, k) => !!f?.[k]

/** 当前方言风格：mysql / pg / sqlite / oracle / mssql / clickhouse / generic */
export const ddlStyleOf = (f) => f?.ddlStyle || 'generic'

/** 标识符转义（按 quoteStyle: BACKTICK / BRACKET / DOUBLE_QUOTE） */
export const qt = (f, name) => {
  const s = String(name ?? '')
  const style = f?.quoteStyle
  if (style === 'BACKTICK') return '`' + s.replace(/`/g, '``') + '`'
  if (style === 'BRACKET') return '[' + s.replace(/\]/g, ']]') + ']'
  return '"' + s.replace(/"/g, '""') + '"'
}

/** 单引号字符串转义 */
export const sq = (v) => String(v ?? '').replace(/'/g, "''")

/** 类型 + 长度 → 完整类型串（避免重复括号） */
export const buildType = (type, len) => {
  const base = (type || '').trim() || 'VARCHAR'
  const l = String(len ?? '').trim()
  if (l && !base.includes('(')) return `${base}(${l})`
  return base
}

/** 解析完整类型串 → { type, length, rest } */
export const parseType = (raw) => {
  const s = (raw || '').trim()
  const m = s.match(/^([A-Za-z_]+)\s*\(([^)]*)\)(.*)$/)
  if (m) return { type: m[1].toUpperCase(), length: m[2].trim(), rest: m[3] }
  const parts = s.split(/\s+/)
  return { type: (parts[0] || '').toUpperCase(), length: '', rest: parts.slice(1).join(' ') }
}

/** 默认值 → SQL 片段（null 处理 + 引号包裹） */
export const defaultPart = (dv, useBare = false) => {
  const s = String(dv ?? '').trim()
  if (s === '') return ''
  if (/^(null|true|false|current_timestamp|now\(\)|current_date|current_time)$/i.test(s)) return ` DEFAULT ${s}`
  if (/^-?\d+(\.\d+)?$/.test(s) && !useBare) return ` DEFAULT ${s}`
  return ` DEFAULT '${sq(s)}'`
}

/** Oracle / 达梦类型映射 */
export const oracleType = (t, len) => {
  const up = (t || '').toUpperCase()
  const map = {
    VARCHAR: 'VARCHAR2', VARCHAR2: 'VARCHAR2', CHAR: 'CHAR', TEXT: 'CLOB',
    INTEGER: 'NUMBER(10)', BIGINT: 'NUMBER(19)', SMALLINT: 'NUMBER(5)',
    DECIMAL: 'NUMBER', NUMERIC: 'NUMBER', FLOAT: 'NUMBER', REAL: 'BINARY_DOUBLE', DOUBLE: 'BINARY_DOUBLE',
    DATE: 'DATE', TIME: 'DATE', TIMESTAMP: 'TIMESTAMP', DATETIME: 'TIMESTAMP',
    BOOLEAN: 'NUMBER(1)', BLOB: 'BLOB', JSON: 'CLOB'
  }
  const base = map[up] || up
  const l = String(len ?? '').trim()
  if (base === 'NUMBER' && up === 'DECIMAL' && l) return `NUMBER(${l})`
  if ((up === 'VARCHAR' || up === 'VARCHAR2' || up === 'CHAR') && l) return `${base}(${l})`
  if (up === 'NUMERIC' && l) return `NUMBER(${l})`
  return base
}

/** 复制文本 */
export const copyText = async (text, tip = t('common.copied')) => {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    return false
  }
}

/** 生成一个临时唯一 id（用于行 key） */
export const uid = () => Math.random().toString(36).slice(2, 9)

/** 从 SHOW CREATE 类 DDL 中解析 AS SELECT 之后的查询体（视图编辑用） */
export const extractViewSelect = (ddl) => {
  if (!ddl) return ''
  const s = ddl.replace(/;+\s*$/, '').trim()
  // 找到视图定义中的 AS：其后跳过空白与注释（-- / /* */）应当就是 SELECT/WITH，
  // 这样 AS 后面的注释会随查询体一起回显，不会被丢掉（避免匹配列别名里的 AS）
  const re = /\bAS\b/gi
  let m
  while ((m = re.exec(s))) {
    const rest = s.slice(m.index + m[0].length)
    if (/^(?:\s|--[^\n]*\n?|\/\*[\s\S]*?\*\/)*(?=SELECT\b|WITH\b)/i.test(rest)) {
      return rest.replace(/^\s+/, '').replace(/;+\s*$/, '').trim()
    }
  }
  return s
}

/**
 * 规整例程/触发器语句体：MySQL/PG/Oracle/MSSQL/SQLite 的例程体内语句
 * 均必须以分号结束，若用户只填了逻辑没写末尾分号，这里自动补上。
 * 仅处理行尾注释（-- 或 #），复杂多语句缺分号仍需用户自行书写。
 */
export const ensureRoutineBodyEnd = (raw) => {
  let b = String(raw ?? '').trim()
  if (!b) return b
  // 分离行尾注释，分号补在注释之前，避免分号被注释吞掉
  const m = b.match(/^(.*?)(\s*--[^\n]*|\s*#[^\n]*)$/)
  const core = (m ? m[1] : b).trim()
  if (!core) return b
  if (/;\s*$/.test(core)) return b
  const rest = m ? b.slice(m[1].length) : ''
  return core + ';' + rest
}

/** 从过程/触发器 DDL 中提取主体（BEGIN...END / AS ... BEGIN / $$...$$ 之后） */
export const extractRoutineBody = (ddl) => {
  if (!ddl) return ''
  const s = ddl.replace(/;+\s*$/, '').trim()
  // $$ ... $$（PG）
  const dollar = s.match(/\$\$([\s\S]*?)\$\$/)
  if (dollar) return dollar[1].trim()
  // BEGIN ... END
  const beginIdx = s.search(/\bBEGIN\b/i)
  if (beginIdx >= 0) {
    return s.slice(beginIdx + 5).replace(/\bEND\b\s*$/i, '').trim()
  }
  // AS 之后
  const asIdx = s.search(/\bAS\b/i)
  if (asIdx >= 0) {
    const after = s.slice(asIdx + 2).trim()
    return after.replace(/^\s*(BEGIN)?/, '').replace(/\bEND\b\s*$/i, '').trim()
  }
  return s
}

// ==================== 例程 DDL 结构化解析（SQL Server / 多行签名） ====================
// 背景：SQL Server 的 DDL 常带 schema 限定名（[dbo].[f]）、跨行签名、
// 表值返回（RETURNS @t TABLE(...)）与 \r\n\r\n 空行；早期用简单正则解析会导致
// 参数丢失、返回类型被错认成语句体里的 "end"，最终生成的 SQL 与原定义不一致。
// 这里用「扫描器 + 偏移量」的方式解析，同时记录各片段在原文中的位置，
// 供保存时按原 DDL 精确拼接（保留注释 / 格式 / WITH 选项）。

/** 跳过字符串、引号标识符、注释；返回跳过后的下标（未跳过返回 -1） */
const skipNonCode = (s, i) => {
  const ch = s[i]
  if (ch === "'") {
    let j = i + 1
    while (j < s.length) {
      if (s[j] === "'" && s[j + 1] === "'") { j += 2; continue }
      if (s[j] === "'") return j + 1
      j++
    }
    return s.length
  }
  if (ch === '"' || ch === '`') {
    let j = i + 1
    while (j < s.length && s[j] !== ch) j++
    return j + 1
  }
  if (ch === '[') {
    let j = i + 1
    while (j < s.length && s[j] !== ']') j++
    return j + 1
  }
  if (ch === '-' && s[i + 1] === '-') { let j = i; while (j < s.length && s[j] !== '\n') j++; return j }
  if (ch === '/' && s[i + 1] === '*') { let j = i + 2; while (j < s.length && !(s[j] === '*' && s[j + 1] === '/')) j++; return Math.min(j + 2, s.length) }
  if (ch === '#') { let j = i; while (j < s.length && s[j] !== '\n') j++; return j }
  return -1
}

/** 顶层（括号外）查找从 from 开始的第一个匹配 token，返回下标，找不到返回 -1 */
const findTopLevel = (s, from, re) => {
  let depth = 0
  for (let i = from; i < s.length;) {
    const skip = skipNonCode(s, i)
    if (skip > i) { i = skip; continue }
    const ch = s[i]
    if (ch === '(') { depth++; i++; continue }
    if (ch === ')') { if (depth > 0) depth--; i++; continue }
    if (depth === 0) {
      const m = re.exec(s.slice(i, i + 32))
      if (m && m.index === 0) return i
    }
    i++
  }
  return -1
}

/** 跳过空白与注释（-- / # / 块注释），返回第一个"有效代码"位置 */
const skipTrivia = (s, i) => {
  let j = i
  for (;;) {
    while (j < s.length && /\s/.test(s[j])) j++
    const skip = skipNonCode(s, j)
    if (skip > j) { j = skip; continue }
    return j
  }
}

/** 匹配括号：openIdx 指向 '('，返回对应 ')' 的下标，失败返回 -1 */
const matchParen = (s, openIdx) => {
  let depth = 0
  for (let i = openIdx; i < s.length;) {
    const skip = skipNonCode(s, i)
    if (skip > i) { i = skip; continue }
    const ch = s[i]
    if (ch === '(') { depth++; i++; continue }
    if (ch === ')') { depth--; if (depth === 0) return i; i++; continue }
    i++
  }
  return -1
}

/** 匹配 BEGIN ... END（把 CASE 也计入层级，兼容 T-SQL 的 CASE ... END） */
const matchBeginEnd = (s, beginIdx) => {
  let depth = 1
  let i = beginIdx + 5
  while (i < s.length) {
    const skip = skipNonCode(s, i)
    if (skip > i) { i = skip; continue }
    const rest = s.slice(i, i + 8)
    if (/^BEGIN\b/i.test(rest)) { depth++; i += 5; continue }
    if (/^CASE\b/i.test(rest)) { depth++; i += 4; continue }
    if (/^END\b/i.test(rest)) { depth--; if (depth === 0) return i; i += 3; continue }
    i++
  }
  return -1
}

/** 按分隔符切分顶层片段（忽略括号/引号内的分隔符） */
const splitTopLevel = (s, sep = ',') => {
  const out = []
  let cur = ''
  let depth = 0
  for (let i = 0; i < s.length;) {
    const skip = skipNonCode(s, i)
    if (skip > i) { cur += s.slice(i, skip); i = skip; continue }
    const ch = s[i]
    if (ch === '(') { depth++; cur += ch; i++; continue }
    if (ch === ')') { if (depth > 0) depth--; cur += ch; i++; continue }
    if (ch === sep && depth === 0) { out.push(cur); cur = ''; i++; continue }
    cur += ch
    i++
  }
  out.push(cur)
  return out
}

/** 拆分带引号的限定名：[dbo].[f_split] / "s"."n" / `db`.`n` / s.n / n */
const splitQualified = (raw) => {
  const parts = []
  let cur = ''
  let quote = ''
  for (const ch of String(raw || '')) {
    if (quote) { if (ch === quote) quote = ''; else cur += ch; continue }
    if (ch === '[') { quote = ']'; continue }
    if (ch === '"' || ch === '`') { quote = ch; continue }
    if (ch === '.') { parts.push(cur); cur = ''; continue }
    cur += ch
  }
  parts.push(cur)
  return parts.map((x) => x.trim()).filter((x) => x !== '')
}

/** SQL Server 参数：@name TYPE [= default] [OUTPUT|READONLY] */
const parseMssqlParams = (raw) => splitTopLevel(raw, ',').map((t) => {
  const str = t.trim()
  if (!str) return null
  const m = str.match(/^@([^\s]+)\s+([\s\S]+)$/)
  if (!m) return null
  let type = m[2].trim()
  let mode = 'IN'
  if (/\s+OUT(?:PUT)?\s*$/i.test(type)) {
    mode = 'OUT'
    type = type.replace(/\s+OUT(?:PUT)?\s*$/i, '').trim()
  }
  // 去掉默认值（如 @a INT = 0）
  const dv = findTopLevel(type, 0, /^=/)
  if (dv >= 0) type = type.slice(0, dv).trim()
  return { mode, name: m[1], type, length: '' }
}).filter(Boolean)

/**
 * 解析例程 DDL → 结构化字段 + 各片段偏移（offset 基于已归一化为 \n 的文本）
 * 返回 null 表示无法可靠解析（调用方回退旧逻辑）
 */
export const parseRoutineDdl = (ddl, style = 'generic') => {
  if (!ddl) return null
  const text = String(ddl).replace(/\r\n?/g, '\n')
  const head = /(CREATE|ALTER)\s+(?:DEFINER\s*=\s*\S+\s+)?(?:OR\s+REPLACE\s+)?(FUNCTION|PROCEDURE|PROC)\b/i.exec(text)
  if (!head) return null
  const isFn = /FUNCTION/i.test(head[2])
  let i = head.index + head[0].length
  while (i < text.length && /\s/.test(text[i])) i++
  // 名称 token（支持 [dbo].[f] / "s"."n" / `n` / n），不再假设后面紧跟 '('
  const NAME_TOKEN = /^(?:\[[^\]]*\]|"[^"]*"|`[^`]*`|[\w$#@]+)(?:\s*\.\s*(?:\[[^\]]*\]|"[^"]*"|`[^`]*`|[\w$#@]+))*/
  const nameTok = NAME_TOKEN.exec(text.slice(i))
  const nameText = nameTok ? nameTok[0] : ''
  const nameEnd = i + nameText.length
  const nameParts = splitQualified(nameText)
  const name = nameParts.length ? nameParts[nameParts.length - 1] : ''
  if (!name) return null
  const schema = nameParts.slice(0, -1).join('.')

  // 参数：优先括号形式；SQL Server 还支持无括号的参数列表（到顶层 AS 为止）
  let params = []
  let paramsSpan = null
  let paramsParen = false
  const pStart = nameEnd
  let p = pStart
  while (p < text.length && /\s/.test(text[p])) p++
  if (text[p] === '(') {
    const close = matchParen(text, p)
    if (close > p) {
      paramsSpan = [p, close + 1]
      paramsParen = true
      const rawParams = text.slice(p + 1, close).trim()
      if (rawParams) {
        if (style === 'mssql') params = parseMssqlParams(rawParams)
        else {
          params = splitTopLevel(rawParams, ',').map((t) => {
            const str = t.trim()
            if (!str) return null
            if (style === 'oracle') {
              const m = str.match(/^(\w+)\s+(?:(IN)\s+|(OUT)\s+|(IN\s+OUT)\s+)?([A-Za-z_][\w ]*(?:\([^)]*\))?)/i)
              if (!m) return null
              return { mode: m[4] ? 'INOUT' : (m[3] ? 'OUT' : 'IN'), name: m[1], type: m[5].trim(), length: '' }
            }
            const m = str.match(/^(?:(IN|OUT|INOUT)\s+)?([\w$#@]+)\s+([A-Za-z_][\w ]*(?:\([^)]*\))?)/i)
            if (!m) return null
            const pt = parseType(m[3])
            return { mode: m[1] || 'IN', name: m[2], type: pt.type, length: pt.length }
          }).filter(Boolean)
        }
      }
    }
  } else if (style === 'mssql' && !isFn) {
    // CREATE PROCEDURE [dbo].[p] @a INT, @b INT OUTPUT AS ...（无括号）
    const asPos = findTopLevel(text, p, /^AS\b/i)
    let ps = p
    let pe = asPos >= 0 ? asPos : text.length
    while (pe > ps && /\s/.test(text[pe - 1])) pe--
    const rawParams = text.slice(ps, pe)
    if (rawParams && !/^WITH\b/i.test(rawParams.trim())) {
      paramsSpan = [ps, pe]
      params = parseMssqlParams(rawParams)
    }
  }

  // 返回类型（函数）：RETURNS <spec> [AS]
  let returnSpec = ''
  let retSpan = null
  const afterParams = paramsSpan ? paramsSpan[1] : nameEnd
  if (isFn && style !== 'clickhouse' && style !== 'sqlite') {
    const rel = findTopLevel(text, afterParams, /^RETURNS?\b/i)
    if (rel >= 0) {
      const kwEnd = rel + (/^RETURNS/i.test(text.slice(rel, rel + 8)) ? 7 : 6)
      let start = kwEnd
      while (start < text.length && /\s/.test(text[start])) start++
      const asIdx = findTopLevel(text, start, /^AS\b/i)
      let end = asIdx >= 0 ? asIdx : text.length
      while (end > start && /\s/.test(text[end - 1])) end--
      if (end > start) {
        returnSpec = text.slice(start, end)
        retSpan = [start, end]
      }
    }
  }

  // 语句体：AS 之后（PG 的 $$...$$、外层的 BEGIN...END、或裸语句体）
  let body = ''
  let bodySpan = null
  let hasBeginEnd = false
  const afterRet = retSpan ? retSpan[1] : afterParams
  const asIdx = findTopLevel(text, afterRet, /^AS\b/i)
  const afterAs = asIdx >= 0 ? asIdx + 2 : afterRet
  // AS 之后可能先有注释（-- xxx）再是 BEGIN，这里先跳过空白与注释定位首行代码
  const bodyStart = skipTrivia(text, afterAs)
  const rest = text.slice(bodyStart)
  const dollar = /^\$([A-Za-z_]*)\$/.exec(rest)
  if (dollar) {
    const tag = dollar[0]
    const end = rest.indexOf(tag, tag.length)
    if (end > 0) {
      bodySpan = [bodyStart + tag.length, bodyStart + end]
      body = text.slice(bodySpan[0], bodySpan[1]).trim()
    }
  } else if (/^BEGIN\b/i.test(rest)) {
    hasBeginEnd = true
    const endIdx = matchBeginEnd(text, bodyStart)
    let innerStart = bodyStart + 5
    let innerEnd = endIdx >= 0 ? endIdx : text.length
    while (innerStart < innerEnd && /\s/.test(text[innerStart])) innerStart++
    while (innerEnd > innerStart && /\s/.test(text[innerEnd - 1])) innerEnd--
    bodySpan = [innerStart, innerEnd]
    body = text.slice(innerStart, innerEnd)
  } else {
    let end = text.length
    while (end > bodyStart && /[\s;]/.test(text[end - 1])) end--
    bodySpan = [bodyStart, end]
    body = text.slice(bodyStart, end)
  }

  return {
    retType: isFn ? 'FUNCTION' : 'PROCEDURE',
    name, schema, params, returnSpec, body,
    spans: {
      kw: [head.index, head.index + head[1].length],
      params: paramsSpan,
      paramsParen,
      ret: retSpan,
      body: bodySpan,
      hasBeginEnd
    },
    // 归一化（\n）后的原文：拼接时以它为基准，保证偏移一致
    text,
    // 原文换行风格，拼接后还原，做到「不改动即零差异」
    crlf: /\r\n/.test(String(ddl))
  }
}

/** 按偏移量替换片段（edits: [[start, end, text]]，互不重叠） */
export const applySpans = (text, edits) => {
  const list = edits.filter((e) => e && e[0] >= 0 && e[1] >= e[0]).sort((a, b) => b[0] - a[0])
  let out = text
  for (const [s, e, v] of list) out = out.slice(0, s) + v + out.slice(e)
  return out
}
