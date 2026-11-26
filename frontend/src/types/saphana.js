/**
 * SAP HANA 类型定义
 *
 * HANA 的 JDBC URL：`jdbc:sap://host:port/?databaseName=<库>`，
 * 系统目录在 SYS schema（SYS.SCHEMAS / SYS.TABLES / SYS.TABLE_COLUMNS），
 * 标识符双引号、未加引号折大写。
 */
export default {
  code: 'SAPHANA',
  webScheme: 'sap',
  formatDialect: '',
  canDropDatabase: false,
  dropIfExists: false,
  // 上游兼容接口的模块路径段：与 plugins/connection-types/saphana.yaml 的 dialect 派生值一致
  module: 'saphana',
  label: 'SAP HANA',
  category: 'OLAP',
  noSql: false,
  schemaLevel: 'schema',
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'saphana.svg',
  fallbackLogo: 'saphana',
  fileType: false,
  defaultPort: 30015,
  databaseRequired: true,
  urlTemplate: 'jdbc:sap://{host}:{port}/?databaseName={database}',
  propStyle: 'query',
  options: [],
  createDatabaseSql() {
    return ''
  },
  showDbDdlSql() {
    return null
  }
}
