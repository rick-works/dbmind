import { pgFamilyOptions } from './presets.js'

/**
 * 海量 Vastbase 类型定义（PostgreSQL 内核的国产发行版）
 *
 * 复用 PostgreSQL 的 schema 层级、标识符引号、驱动与 URL 模板。
 * 现场若必须用 Vastbase 原生驱动，可在设置页手动上传覆盖（坐标见 YAML 注释）。
 */
export default {
  code: 'VASTBASE',
  // 与 plugins/connection-types/vastbase.yaml 的 dialect 派生值一致（guard 校验）
  module: 'postgresql',
  label: 'Vastbase',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'schema',
  defaultSchema: 'public',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'vastbase.svg',
  fallbackLogo: 'vastbase',
  fileType: false,
  defaultPort: 5432,
  databasePh: 'cd.dbPhOptional',
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
