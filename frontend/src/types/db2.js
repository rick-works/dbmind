import { formParts } from './base'

/** IBM DB2 类型定义 */
export default {
  code: 'DB2',
  label: 'DB2',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'db2.svg',
  fileType: false,
  defaultPort: 50000,
  buildJdbcUrl(f) {
    const { host, port, db } = formParts(f)
    if (!host) return ''
    const p = port || 50000
    return `jdbc:db2://${host}:${p}/${db}`
  },
  // DB2 建库是 CLP 命令（CREATE DATABASE）且不能在 SQL 会话内执行，SQL 层不支持
  createDatabaseSql() {
    return ''
  },
  // 无「查询建库语句」接口，右键菜单不提供
  showDbDdlSql() {
    return null
  }
}
