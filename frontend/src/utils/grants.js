// 用户权限解析纯函数（无组件状态，供 MainView 打开用户编辑页签前组装数据）
// 权限名归一化：Doris 等库使用 XXX_PRIV 体系，统一还原为本工具使用的 MySQL 权限名（大写）
const DORIS_PRIV_MAP = {
  SELECT_PRIV: 'SELECT',
  LOAD_PRIV: 'INSERT',      // Doris 的数据写入（INSERT/UPDATE/DELETE）统一为 LOAD_PRIV
  CREATE_PRIV: 'CREATE',
  DROP_PRIV: 'DROP',
  ALTER_PRIV: 'ALTER',
  ADMIN_PRIV: 'ALL PRIVILEGES',
  GRANT_PRIV: 'GRANT OPTION',
  SHOW_VIEW_PRIV: 'SHOW VIEW',
  NODE_PRIV: 'ALL PRIVILEGES',
  USAGE_PRIV: '',           // 无实际权限，忽略
  USAGE: ''
}
const normalizePriv = (p) => {
  const s = String(p ?? '').trim()
  if (!s) return ''
  const up = s.toUpperCase()
  if (Object.prototype.hasOwnProperty.call(DORIS_PRIV_MAP, up)) return DORIS_PRIV_MAP[up]
  // 其它 *_PRIV 结尾（如 REPOSITORY_PRIV）：去掉后缀后按通用权限名处理
  if (up.endsWith('_PRIV')) return up.slice(0, -'_PRIV'.length)
  return up
}
// Doris 的库/表可能带 catalog 前缀（如 internal.db1.*），取最后一段作为库名
const stripCatalog = (name) => (name && name.includes('.') ? name.split('.').pop() : name)

// MySQL：把 SHOW GRANTS 输出解析为 全局权限 / 库级权限 / 表级权限 三层结构
export const parseMySqlGrants = (grants) => {
  const globalPrivs = new Set()
  const dbPrivMap = new Map()
  const tablePrivMap = new Map()
  for (const g of grants) {
    if (!g || typeof g !== 'string') continue
    const match = g.match(/GRANT\s+(.+?)\s+ON\s+(.+?)\s+TO/i)
    if (!match) continue
    const privPart = match[1].trim()
    const onPart = match[2].trim()
    const privList = privPart.split(',').map(normalizePriv).filter(Boolean)
    if (onPart === '*.*') {
      for (const p of privList) globalPrivs.add(p)
    } else {
      const dbMatch = onPart.match(/^`?([^`]+)`?\.\*$/)
      if (dbMatch) {
        const db = stripCatalog(dbMatch[1])
        const existing = dbPrivMap.get(db) || new Set()
        for (const p of privList) existing.add(p)
        dbPrivMap.set(db, existing)
      } else {
        const tableMatch = onPart.match(/^`?([^`]+)`?\.`?([^`]+)`?$/)
        if (tableMatch) {
          const db = stripCatalog(tableMatch[1])
          const table = tableMatch[2]
          const key = `${db}.${table}`
          const existing = tablePrivMap.get(key) || new Set()
          for (const p of privList) existing.add(p)
          tablePrivMap.set(key, existing)
        }
      }
    }
  }
  return {
    privileges: Array.from(globalPrivs),
    dbPrivileges: Array.from(dbPrivMap.entries()).map(([database, set]) => ({ database, privileges: Array.from(set) })),
    tablePrivileges: Array.from(tablePrivMap.entries()).map(([key, set]) => {
      const [database, table] = key.split('.')
      return { database, table, privileges: Array.from(set) }
    })
  }
}

// SQL Server：把权限详情解析为 服务器角色 / 数据库角色 / Schema 权限 / 对象权限
export const parseSqlServerPerms = (info) => {
  const result = { serverRoles: [], roles: [], schemaPrivileges: [], objectPrivileges: [] }
  if (info.serverRoles && Array.isArray(info.serverRoles)) {
    result.serverRoles = info.serverRoles.filter(r => r && !r.startsWith('--'))
  }
  if (info.roles && Array.isArray(info.roles)) {
    result.roles = info.roles.filter(r => r && !r.startsWith('--'))
  }
  // Schema 权限按 schema 合并
  if (info.schemaPrivileges && Array.isArray(info.schemaPrivileges)) {
    const schemaMap = new Map()
    for (const item of info.schemaPrivileges) {
      const sch = item.schema
      const perm = item.permission
      const state = item.state
      if (!sch || !perm || !state || String(state).toUpperCase() !== 'GRANT') continue
      const existing = schemaMap.get(sch) || new Set()
      existing.add(perm)
      schemaMap.set(sch, existing)
    }
    result.schemaPrivileges = Array.from(schemaMap.entries()).map(([schema, set]) => ({ schema, privileges: Array.from(set) }))
  }
  // 对象权限按 schema+object 合并
  if (info.objectPrivileges && Array.isArray(info.objectPrivileges)) {
    const objMap = new Map()
    for (const item of info.objectPrivileges) {
      const sch = item.schema
      const obj = item.object
      const perm = item.permission
      const state = item.state
      if (!sch || !obj || !perm || !state || String(state).toUpperCase() !== 'GRANT') continue
      const key = `${sch}.${obj}`
      const existing = objMap.get(key) || new Set()
      existing.add(perm)
      objMap.set(key, existing)
    }
    result.objectPrivileges = Array.from(objMap.entries()).map(([key, set]) => {
      const [schema, object] = key.split('.')
      return { schema, object, privileges: Array.from(set) }
    })
  }
  return result
}
