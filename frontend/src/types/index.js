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

/** 类型码统一口径：库里一律按大写登记，调用方传小写也能认（`mysql` / `MYSQL` 等价） */
const upperOf = (code) => String(code || '').toUpperCase()

/** 是否 NoSQL 类型 */
export const isNoSql = (code) => Boolean(byType(code).noSql)

/** 类型展示名 */
export const labelOf = (code) => { const d = byType(code); return d.labelKey ? t(d.labelKey) : (d.label || code || '—') }

/** 标识符引号风格：BACKTICK / BRACKET / DOUBLE_QUOTE */
export const quoteStyleOf = (code) => byType(code).quoteStyle

/**
 * sql-formatter 方言名；空串 = 标准 SQL。
 *
 * 取代 `utils/sqlFormat.js` 里那张手写的「类型码 → 方言」表：新增同族类型
 * （GreatSQL / StarRocks / HighGo…）不再需要去改格式化模块。
 */
export const formatDialectOf = (code) => byType(code).formatDialect || ''

/**
 * 连接悬浮卡里 `scheme://host:port` 的 scheme（纯展示）。
 *
 * 空串表示用类型码小写兜底；文件型数据库由调用方按 `fileType` 直接跳过。
 */
export const webSchemeOf = (code) => byType(code).webScheme || ''

/**
 * 库节点右键是否给「新建模式」。
 *
 * 只有「模式独立于用户」的类型才成立：PG / Kingbase / SQL Server。
 * Oracle / DM 的模式就是用户（先建用户才有模式），MySQL 系的库本身就是命名空间。
 */
export const canCreateSchemaOf = (code) => Boolean(byType(code).canCreateSchema)

/**
 * 库节点右键是否给「删除数据库」。
 *
 * 只认真有 `DROP DATABASE` 语法的类型；没有这条语句的类型（Oracle / DM / H2 / Derby /
 * SQLite）界面直接说明「为什么没有、该怎么做」，而不是让用户收一句语法错误。
 */
export const canDropDatabaseOf = (code) => Boolean(byType(upperOf(code)).canDropDatabase)

/** 删库前是否先踢掉占着该库的其它会话（PG 系的 `pg_terminate_backend`） */
export const dropDbKickSessionsOf = (code) => Boolean(byType(upperOf(code)).dropDbKickSessions)

/** `DROP <对象>` 是否认 `IF EXISTS`（Oracle / DM / DB2 / Derby 不认） */
export const dropIfExistsOf = (code) => byType(upperOf(code)).dropIfExists !== false

/**
 * 「改对象定义」时必须 DROP + CREATE 的 kind；空数组 = 能用 ALTER / CREATE OR REPLACE。
 *
 * 各类型自己声明的优先（**不要**为了少写几行把 MySQL 家族换成「方言是 mysql」的推导：
 * GreatSQL / Databend 之类还没逐个验证过 ALTER 行为）。PG 语法族补 `trigger` ——
 * `ALTER TRIGGER` 只能改名 / 改依赖，改不了触发体，这条是方言事实，不逐个品牌声明，
 * 新增 PG 分支（HighGo / Vastbase / RisingWave…）自动生效。
 */
export const dropCreateKindsOf = (code) => {
  const c = upperOf(code)
  const declared = byType(c).dropCreateKinds
  if (Array.isArray(declared) && declared.length) return declared
  return formatDialectOf(c) === 'postgresql' ? ['trigger'] : []
}

/**
 * 上游兼容接口的模块路径段（`/api/{module}/{id}/…`）。
 *
 * 以前这张表手写在 `api/index.js` 的 `TYPE_TO_MODULE`，与后端
 * `dialect.rs::module_of_type_code` 必须逐条一致，漏一条就是该类型所有操作 404。
 * 现在两侧都由清单派生：后端读 `plugins/connection-types/*.yaml`（build.rs 生成），
 * 前端读各类型定义里的 `module` 字段（`npm run guard` 会比对两边是否一致）。
 *
 * @returns {string|null} 未知类型返回 null（调用方据此报「不支持的类型」）
 */
export const moduleOfCode = (code) => byType(code).module || null

/** 是否需要 schema 层级导航：'none' | 'schema' */
export const schemaLevelOf = (code) => byType(code).schemaLevel

/** 「库」这一层要不要画（Oracle 不画：它就是连接里的服务名 / PDB，永远只有一条） */
export const hideDbLevelOf = (code) => Boolean(byType(code).hideDbLevel)

/**
 * 连接下是否还有一层 catalog（目前只有 Doris）。
 *
 * 用途是**省掉一次往返**：`/catalogs` 对其余类型恒返回空数组，却排在列库之前串行执行，
 * 等于每次展开连接都白付一次请求。默认 false，未知类型也按 false 处理。
 */
export const hasCatalogsOf = (code) => Boolean(byType(code).catalogs)

/**
 * 面向界面的「库.模式」串 —— **只用于显示，不要拿它去发请求**。
 *
 * 对没有「库」这一层的类型（Oracle / DM，见 hideDbLevelOf），库那一段就是连接里填的
 * 服务名（如 FREE），把它写进页签标题、路径、历史上纯属噪音：用户眼里的"库"其实是模式。
 * 所以这里只留模式：`FREE.SYS` → `SYS`。
 *
 * 没有点号的值**原样返回**：这类类型的库名可以留空（达梦的连接串就不写库名），
 * 树上的模式节点于是直接带裸模式名（`DEMO`），它不是"服务名噪音"而是用户要看的那一层；
 * 老实现一律返回空串，页签路径/表详情标题里就整段丢掉模式。
 *
 * 其余类型原样返回（MySQL 的库名本身可以带点，不能乱切）。
 */
export const dbDisplayOf = (code, db) => {
  const q = db == null ? '' : String(db)
  if (!q || !hideDbLevelOf(code)) return q
  const i = q.indexOf('.')
  return i > 0 ? q.slice(i + 1) : q
}

export { DB_TYPES }
