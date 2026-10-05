import { formParts } from './base'

/** 达梦 DM 类型定义 */
export default {
  code: 'DM',
  label: 'DM', labelKey: 'tdef.dm',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'dm.svg',
  fileType: false,
  defaultPort: 5236,
  buildJdbcUrl(f) {
    const { host, port, db } = formParts(f)
    if (!host) return ''
    const p = port || 5236
    return `jdbc:dm://${host}:${p}?schema=${db}&compatibleMode=mysql`
  },
  // 达梦建库依赖 dminit / 管理工具，SQL 层不提供 CREATE DATABASE，标记为不支持
  createDatabaseSql() {
    return ''
  },
  // 无「查询建库语句」接口，右键菜单不提供
  showDbDdlSql() {
    return null
  }
}
