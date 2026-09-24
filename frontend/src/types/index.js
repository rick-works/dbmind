/**
 * 数据库类型注册表入口
 *
 * 自动收集本目录（除 base/index 外）所有 .js 模块并按其 code 建立索引。
 * 新增数据库类型 = 在本目录新增一个 .js 文件，无需修改本文件。
 *
 * 调用示例：
 *   import { byType, isNoSql, labelOf, quoteStyleOf, schemaLevelOf } from '../types'
 *   byType('SQLSERVER').schemaLevel          // 'schema'
 *   isNoSql(conn.type)                        // 替代 ['MONGODB','REDIS',...].includes()
 *   quoteStyleOf(type)                        // 'BACKTICK' | 'BRACKET' | 'DOUBLE_QUOTE'
 */
import DEFAULT_DEF from './base'
import { t } from '../utils/i18n'

const modules = import.meta.glob('./*.js', { eager: true })

/** code -> 类型定义（已合入默认能力） */
const DB_TYPES = {}
/** 收集到的类型码顺序列表 */
const ALL_CODES = []

for (const path of Object.keys(modules)) {
  // 跳过非类型定义文件
  if (path.endsWith('/index.js') || path.endsWith('/base.js')) continue
  const mod = modules[path]
  const def = mod && mod.default ? mod.default : null
  const items = Array.isArray(def) ? def : def ? [def] : []
  for (const it of items) {
    if (!it || !it.code) continue
    DB_TYPES[it.code] = { ...DEFAULT_DEF, ...it }
    if (!ALL_CODES.includes(it.code)) ALL_CODES.push(it.code)
  }
}

/** 取某类型的完整定义；未知类型返回默认能力（不抛错） */
export const byType = (code) => DB_TYPES[code] || { ...DEFAULT_DEF, code }

/** 是否 NoSQL 类型 */
export const isNoSql = (code) => Boolean(byType(code).noSql)

/** 类型展示名 */
export const labelOf = (code) => { const d = byType(code); return d.labelKey ? t(d.labelKey) : (d.label || code || '—') }

/** 标识符引号风格：BACKTICK / BRACKET / DOUBLE_QUOTE */
export const quoteStyleOf = (code) => byType(code).quoteStyle

/** 是否需要 schema 层级导航：'none' | 'schema' */
export const schemaLevelOf = (code) => byType(code).schemaLevel

/** 通用能力取值：cap(type, 'defaultPort') */
export const cap = (code, key) => byType(code)[key]

export { DB_TYPES, ALL_CODES }
