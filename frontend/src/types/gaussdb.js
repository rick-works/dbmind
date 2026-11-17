import { pgFamilyOptions } from './presets.js'

/**
 * 华为 GaussDB 类型定义（openGauss / PG 协议，走 openGauss 的 JDBC fork）
 *
 * ⚠️ 驱动不能用 PG 官方包：GaussDB/openGauss 默认口令加密是 sha256，PG 驱动不会算，
 * 会直接登录被拒（openGauss 真机踩过）。所以 URL 前缀是 `jdbc:opengauss:`，
 * 选项也按 openGauss 驱动的接受范围收敛。
 */
export default {
  code: 'GAUSSDB',
  // 与 plugins/connection-types/gaussdb.yaml 的 dialect 派生值一致（guard 校验）
  module: 'postgresql',
  label: 'GaussDB',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'schema',
  defaultSchema: 'public',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: '',
  fallbackLogo: 'gaussdb',
  fileType: false,
  defaultPort: 8000,
  databasePh: 'cd.dbPhOptional',
  keepEmptyDbSlash: true,
  // openGauss 的 fork 对 PG 的多数连接参数不认，URL 保持最小集
  urlTemplate: 'jdbc:opengauss://{host}:{port}/{database}?sslmode=disable',
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
