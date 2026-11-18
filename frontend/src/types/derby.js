/**
 * Apache Derby 类型定义（本地文件型）
 *
 * 与 plugins/connection-types/derby.yaml 一致：`jdbc:derby:{filePath};create=true`
 * （Derby 的 filePath 是**数据库目录**，不是单个文件；连接时 create=true 自动建库）
 */
export default {
  code: 'DERBY',
  // Derby 的 DROP 里没有 IF EXISTS
  dropIfExists: false,
  // 上游兼容接口的模块路径段：与 plugins/connection-types/derby.yaml 的 dialect 派生值一致（guard 校验）
  module: 'derby',
  label: 'Apache Derby',
  category: 'RELATIONAL_FILE',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'derby.svg',
  fileType: true,
  defaultPort: 1527,
  fileHint: 'tdef.derbyFileHint',
  urlTemplate: 'jdbc:derby:{filePath};create=true',
  propStyle: 'semicolon',
  // Derby 连接时自动建库（;create=true），无需 SQL 层 CREATE DATABASE
  createDatabaseSql() {
    return ''
  },
  // 连接即建库，无独立建库语句
  showDbDdlSql() {
    return null
  }
}
