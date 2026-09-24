import { formParts } from './base'

/** 人大金仓 KingbaseES 类型定义（PostgreSQL 兼容，schema 层级） */
export default {
  code: 'KINGBASE',
  label: 'KingbaseES', labelKey: 'tdef.kingbase',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'schema',
  defaultSchema: 'public',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'kingbase.svg',
  fileType: false,
  defaultPort: 54321,
  buildJdbcUrl(f) {
    const { host, port, db } = formParts(f)
    if (!host) return ''
    const p = port || 54321
    return `jdbc:kingbase8://${host}:${p}/${db}?reWriteBatchedInserts=true`
  },
  // 人大金仓兼容 PostgreSQL 语法，双引号标识符
  createDatabaseSql(name) {
    const n = String(name ?? '').trim().replace(/"/g, '""')
    return `CREATE DATABASE "${n}"`
  },
  // 兼容 PostgreSQL：从 pg_database 反查建库语句
  showDbDdlSql(db) {
    return `SELECT 'CREATE DATABASE ' || quote_ident(datname) || ' WITH OWNER ' || quote_ident(pg_catalog.pg_get_userbyid(datdba)) || ' ENCODING ' || quote_literal(pg_encoding_to_char(encoding)) || ' LC_COLLATE ' || quote_literal(datcollate) || ' LC_CTYPE ' || quote_literal(datctype) || ';' AS ddl FROM pg_database WHERE datname = '${String(db ?? '')}'`
  }
}
