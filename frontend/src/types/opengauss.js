
import { pgFamilyOptions } from './presets.js'

/**
 * openGauss 类型定义（PostgreSQL 兼容，schema 层级）
 *
 * 华为从 PG 9.2 内核衍生（GaussDB 开源版）：连接表单、URL 模板、建库语句
 * 复用 PG 那一套；系统视图/MVCC 的差异在服务端，SQL 生成层照 PG 走。
 */
export default {
  code: 'OPENGAUSS',
  // 上游兼容接口的模块路径段：与 plugins/connection-types/opengauss.yaml 的 dialect 派生值一致（guard 校验）
  module: 'postgresql',
  label: 'openGauss',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'schema',
  defaultSchema: 'public',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: '',
  fileType: false,
  fallbackLogo: 'opengauss',
  defaultPort: 5432,
  databasePh: 'cd.dbPhOptional',
  // PG 系驱动要求 URL 有路径段，库名空着也保留 `/`
  keepEmptyDbSlash: true,
  // openGauss 官方 JDBC（PG 驱动分叉），认同一套连接属性；默认值见 postgresql.js 的注释
  urlTemplate: 'jdbc:opengauss://{host}:{port}/{database}?sslmode=prefer&reWriteBatchedInserts=true&stringtype=unspecified&tcpKeepAlive=true',
  options: pgFamilyOptions(),
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
