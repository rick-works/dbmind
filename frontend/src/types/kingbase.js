import { pgFamilyOptions } from './presets.js'

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
  databasePh: 'cd.dbPhOptional',
  // 与 PostgreSQL 同源：pgjdbc 系要求 URL 有路径段，库名空着也保留 `/`
  keepEmptyDbSlash: true,
  // 金仓驱动是 PG 驱动分支，认同一套连接属性；默认值见 postgresql.js 的注释
  urlTemplate: 'jdbc:kingbase8://{host}:{port}/{database}?sslmode=prefer&reWriteBatchedInserts=true&stringtype=unspecified&tcpKeepAlive=true',
  options: pgFamilyOptions(),
  // 与 PostgreSQL 同源：程序名/SSL/类型映射等已从表单删掉，历史残留值不回显
  dropParams: ['ApplicationName', 'sslmode', 'currentSchema', 'reWriteBatchedInserts',
    'stringtype', 'tcpKeepAlive'],
  createDatabaseSql(name) {
    const n = String(name ?? '').trim().replace(/"/g, '""')
    return `CREATE DATABASE "${n}"`
  },
  showDbDdlSql(db) {
    return `SELECT 'CREATE DATABASE ' || quote_ident(datname) || ' WITH OWNER ' || quote_ident(pg_catalog.pg_get_userbyid(datdba)) || ' ENCODING ' || quote_literal(pg_encoding_to_char(encoding)) || ' LC_COLLATE ' || quote_literal(datcollate) || ' LC_CTYPE ' || quote_literal(datctype) || ';' AS ddl FROM pg_database WHERE datname = '${String(db ?? '')}'`
  }
}
