import { db2Options } from './presets.js'

/**
 * IBM DB2 类型定义
 *
 * DB2 的连接属性写在 URL 的 `:key=value;` 段里（不是 `?a=b`），
 * 所以 propStyle 用 'db2'，预览才和驱动实际收到的形状一致。
 */
export default {
  code: 'DB2',
  webScheme: 'db2',
  formatDialect: 'db2',
  // ⚠️ 不能让「删除数据库」亮起来：DB2 的 CREATE/DROP DATABASE 是**实例命令**
  // （db2 create db / db2 drop db，CLP 里敲），SQL 层根本没有这条语句 ——
  // 实测 `drop database x` 报 SQLCODE=-104 / SQLSTATE=42601（unexpected token "database"）。
  canDropDatabase: false,
  // DB2 的 DROP 里没有 IF EXISTS（同一条 -104/42601）
  dropIfExists: false,
  // 上游兼容接口的模块路径段：与 plugins/connection-types/db2.yaml 的 dialect 派生值一致（guard 校验）
  module: 'db2',
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
