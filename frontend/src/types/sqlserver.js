import { formParts } from './base'

/** SQL Server 类型定义（schema 层级，默认 dbo） */
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
  // 认证方式：账号密码 / Windows 集成身份验证（后端按运行平台选择原生 SSPI 或 JavaKerberos）
  authTypes: [
    { value: 'sqlserver', label: 'SQL Server', labelKey: 'tdef.authSqlServer' },
    { value: 'windows', label: 'Windows', labelKey: 'tdef.authWindows' }
  ],
  buildJdbcUrl(f) {
    const { host, port, db, cts } = formParts(f)
    if (!host) return ''
    const p = port || 1433
    const auth = f?.authType === 'windows' ? ';integratedSecurity=true' : ''
    return `jdbc:sqlserver://${host}:${p};databaseName=${db}${auth};encrypt=false;trustServerCertificate=true;loginTimeout=${cts}`
  },
  createDatabaseSql(name) {
    return `CREATE DATABASE [${String(name ?? '').trim()}]`
  },
  showDbDdlSql(db) {
    return `SELECT 'CREATE DATABASE [' + name + '];' AS ddl FROM sys.databases WHERE name = '${String(db ?? '')}'`
  }
}
