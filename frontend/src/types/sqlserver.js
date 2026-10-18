import { mssqlOptions } from './presets.js'

/**
 * SQL Server 类型定义（schema 层级，默认 dbo）
 *
 * 注意 URL 分隔符：SQL Server 用 `;key=value`，不是 `?a=b&c=d`
 * （propStyle: 'semicolon'，与 mssql-jdbc 的语法一致）。
 * 命名实例不再单列字段：一律用 host:port，命名实例填它实际监听的端口即可。
 */
export default {
  code: 'SQLSERVER',
  label: 'SQL Server',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'schema',
  defaultSchema: 'dbo',
  quoteStyle: 'BRACKET',
  logo: 'sqlserver.svg',
  fileType: false,
  defaultPort: 1433,
  // allowMultiQueries=true：内核执行脚本时一条请求里可能带多条语句（后端 render_jdbc_url
  // 也会给 sqlserver 补这一条，这里写进模板是为了让预览和真实连接一字不差）
  databasePh: 'cd.dbPhOptional',
  urlTemplate: 'jdbc:sqlserver://{host}:{port};databaseName={database};encrypt=false;trustServerCertificate=true;loginTimeout=10;allowMultiQueries=true',
  propStyle: 'semicolon',
  // 认证方式：账号密码 / Windows 集成身份验证（后端按运行平台选择原生 SSPI 或 JavaKerberos）
  authTypes: [
    { value: 'sqlserver', label: 'SQL Server', labelKey: 'tdef.authSqlServer' },
    { value: 'windows', label: 'Windows', labelKey: 'tdef.authWindows' }
  ],
  options: mssqlOptions(),
  // 表单里已删掉的选项：历史残留值在回显时丢掉。
  // 其中 encrypt/trustServerCertificate 本来就在上面的模板里，
  // 其余（Unicode 发送 / 结果缓冲 / 取数方式 / 多子网 / 程序名）都是驱动默认值。
  dropParams: ['applicationName', 'encrypt', 'trustServerCertificate', 'sendStringParametersAsUnicode',
    'responseBuffering', 'selectMethod', 'multiSubnetFailover'],
  createDatabaseSql(name) {
    return `CREATE DATABASE [${String(name ?? '').trim()}]`
  },
  showDbDdlSql(db) {
    return `SELECT 'CREATE DATABASE [' + name + '];' AS ddl FROM sys.databases WHERE name = '${String(db ?? '')}'`
  }
}
