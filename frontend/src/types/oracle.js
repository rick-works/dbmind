import { formParts } from './base'

/** Oracle 类型定义 */
export default {
  code: 'ORACLE',
  label: 'Oracle',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'schema',
  // 「库」这一层对 Oracle 是噪音：它就是**连接里填的服务名 / PDB**（换 PDB 得换连接），
  // 永远只有一条。树与编辑器都用这个标记决定「不画这一层」。
  hideDbLevel: true,
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'oracle.svg',
  fileType: false,
  defaultPort: 1521,
  buildJdbcUrl(f) {
    const { host, port, db } = formParts(f)
    if (!host) return ''
    const p = port || 1521
    return `jdbc:oracle:thin:@${host}:${p}:${db || 'ORCL'}`
  },
  // Oracle 建库属于实例级操作（dbca / CREATE DATABASE 需 SYSDBA 与初始化参数），不在普通连接 SQL 层支持
  createDatabaseSql() {
    return ''
  },
  // 无「查询建库语句」接口，右键菜单不提供
  showDbDdlSql() {
    return null
  }
}
