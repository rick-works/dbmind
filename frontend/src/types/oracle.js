import { oracleOptions } from './presets.js'

/**
 * Oracle 类型定义
 *
 * 服务名（默认，`@host:port/SERVICE`）与 SID（`@host:port:SID`）是两种语法，
 * 表单里用「连接方式」切换，预览与后端 render_jdbc_url 会按同一规则改写模板。
 *
 * propStyle: 'none' —— thin URL 不吃查询参数，所有驱动参数都以连接属性传入，
 * 所以预览里看不到它们（效果与写在 URL 上相同）。
 */
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
  // 标题由「连接方式」动态决定（服务名 / SID），这里只是兜底
  databaseLabel: 'tdef.dbService',
  // thin URL 里没有服务名就连不上：这一项是**真必填**
  databaseRequired: true,
  databaseRequiredMsg: 'cd.needDbOracle',
  urlTemplate: 'jdbc:oracle:thin:@{host}:{port}/{database}',
  propStyle: 'none',
  options: oracleOptions(),
  // 表单里已删掉的选项：历史连接可能还存着（当年默认就写进了 params），
  // 回显时直接丢掉，免得它们在「附加参数」里冒充用户自己配的东西
  dropParams: ['defaultRowPrefetch', 'remarksReporting', 'v$session.program'],
  // Oracle 建库属于实例级操作（dbca / CREATE DATABASE 需 SYSDBA 与初始化参数），不在普通连接 SQL 层支持
  createDatabaseSql() {
    return ''
  },
  // 无「查询建库语句」接口，右键菜单不提供
  showDbDdlSql() {
    return null
  }
}
