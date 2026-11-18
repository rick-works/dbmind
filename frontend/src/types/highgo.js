import { pgFamilyOptions } from './presets.js'

/**
 * 瀚高 HighGo 类型定义（PostgreSQL 内核的国产发行版）
 *
 * schema 层级、双引号标识符、PG 驱动与 URL 模板全部复用 PostgreSQL 那一套；
 * 默认端口按瀚高官方 5866。supportLevel=experimental（同族批量注册，
 * 纵深功能未逐一真机验证）。
 */
export default {
  code: 'HIGHGO',
  webScheme: 'postgres',
  formatDialect: 'postgresql',
  canDropDatabase: true,
  dropDbKickSessions: true,
  // 与 plugins/connection-types/highgo.yaml 的 dialect 派生值一致（guard 校验）
  module: 'postgresql',
  label: 'HighGo',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'schema',
  defaultSchema: 'public',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'highgo.svg',
  fallbackLogo: 'highgo',
  fileType: false,
  defaultPort: 5866,
  databasePh: 'cd.dbPhOptional',
  // pgjdbc 要求 URL 必须有路径段，库名空着也保留 `/`
  keepEmptyDbSlash: true,
  urlTemplate: 'jdbc:postgresql://{host}:{port}/{database}?sslmode=prefer&reWriteBatchedInserts=true&stringtype=unspecified&tcpKeepAlive=true',
  options: pgFamilyOptions(),
  dropParams: ['ApplicationName', 'sslmode', 'currentSchema', 'reWriteBatchedInserts',
    'stringtype', 'tcpKeepAlive'],
  createDatabaseSql(name, f = {}) {
    const n = String(name ?? '').trim().replace(/"/g, '""')
    const parts = [`CREATE DATABASE "${n}"`]
    if (f?.owner) parts.push(`OWNER = "${String(f.owner).trim().replace(/"/g, '""')}"`)
    parts.push(`ENCODING = '${f?.encoding || 'UTF8'}'`)
    const limit = Number(f?.connLimit)
    if (Number.isFinite(limit) && limit > 0) parts.push(`CONNECTION LIMIT = ${Math.floor(limit)}`)
    return parts.join(' ')
  },
  showDbDdlSql(db) {
    return `SELECT 'CREATE DATABASE ' || quote_ident(datname) || ' WITH OWNER ' || quote_ident(pg_catalog.pg_get_userbyid(datdba)) || ' ENCODING ' || quote_literal(pg_encoding_to_char(encoding)) || ' LC_COLLATE ' || quote_literal(datcollate) || ' LC_CTYPE ' || quote_literal(datctype) || ';' AS ddl FROM pg_database WHERE datname = '${String(db ?? '')}'`
  }
}
