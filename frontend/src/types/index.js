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

for (const path of Object.keys(modules)) {
  // 跳过非类型定义文件
  if (path.endsWith('/index.js') || path.endsWith('/base.js')) continue
  const mod = modules[path]
  const def = mod && mod.default ? mod.default : null
  const items = Array.isArray(def) ? def : def ? [def] : []
  for (const it of items) {
    if (!it || !it.code) continue
    DB_TYPES[it.code] = { ...DEFAULT_DEF, ...it }
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

/** 「库」这一层要不要画（Oracle 不画：它就是连接里的服务名 / PDB，永远只有一条） */
export const hideDbLevelOf = (code) => Boolean(byType(code).hideDbLevel)

/**
 * 面向界面的「库.模式」串 —— **只用于显示，不要拿它去发请求**。
 *
 * 对没有「库」这一层的类型（Oracle / DM，见 hideDbLevelOf），库那一段就是连接里填的
 * 服务名（如 FREE），把它写进页签标题、路径、历史上纯属噪音：用户眼里的"库"其实是模式。
 * 所以这里只留模式：`FREE.SYS` → `SYS`；只有服务名没有模式（`FREE`）→ 空串。
 *
 * 其余类型原样返回（MySQL 的库名本身可以带点，不能乱切）。
 */
export const dbDisplayOf = (code, db) => {
  const q = db == null ? '' : String(db)
  if (!q || !hideDbLevelOf(code)) return q
  const i = q.indexOf('.')
  return i > 0 ? q.slice(i + 1) : ''
}

export { DB_TYPES }
