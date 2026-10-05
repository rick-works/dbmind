import { formParts } from './base'

/** PostgreSQL 类型定义（schema 层级） */
export default {
  code: 'POSTGRESQL',
  label: 'PostgreSQL',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'schema',
  defaultSchema: 'public',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'PostgreSQL.svg',
  fileType: false,
  defaultPort: 5432,
  buildJdbcUrl(f) {
    const { host, port, db, cts, st } = formParts(f)
    if (!host) return ''
    const p = port || 5432
    return `jdbc:postgresql://${host}:${p}/${db || 'postgres'}?connectTimeout=${cts}&socketTimeout=${st}&reWriteBatchedInserts=true`
  },
  createDatabaseSql(name, f = {}) {
    const n = String(name ?? '').trim().replace(/"/g, '""')
    return `CREATE DATABASE "${n}" WITH ENCODING = '${f?.encoding || 'UTF8'}'`
  },
  showDbDdlSql(db) {
    return `SELECT 'CREATE DATABASE ' || quote_ident(datname) || ' WITH OWNER ' || quote_ident(pg_catalog.pg_get_userbyid(datdba)) || ' ENCODING ' || quote_literal(pg_encoding_to_char(encoding)) || ' LC_COLLATE ' || quote_literal(datcollate) || ' LC_CTYPE ' || quote_literal(datctype) || ';' AS ddl FROM pg_database WHERE datname = '${String(db ?? '')}'`
  }
}
