import { db2Options } from './presets.js'

/**
 * IBM DB2 类型定义
 *
 * DB2 的连接属性写在 URL 的 `:key=value;` 段里（不是 `?a=b`），
 * 所以 propStyle 用 'db2'，预览才和驱动实际收到的形状一致。
 */
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
  // DB2 的 JDBC URL 末段就是库名，缺了连不上 —— 这一项必填
  databaseRequired: true,
  databasePh: 'cd.dbPhDb2',
  databaseRequiredMsg: 'cd.needDbDb2',
  // 服务端原文报错（含 SQLCODE 说明）固定打开，排查快很多；SSL 用驱动默认（关）
  urlTemplate: 'jdbc:db2://{host}:{port}/{database}:retrieveMessagesFromServerOnGetMessage=true;',
  propStyle: 'db2',
  options: db2Options(),
  // 表单里已删掉的选项：历史残留值在回显时丢掉（默认值已在模板里）
  dropParams: ['currentSchema', 'sslConnection', 'retrieveMessagesFromServerOnGetMessage'],
  // DB2 建库是 CLP 命令（CREATE DATABASE）且不能在 SQL 会话内执行，SQL 层不支持
  createDatabaseSql() {
    return ''
  },
  // 无「查询建库语句」接口，右键菜单不提供
  showDbDdlSql() {
    return null
  }
}
