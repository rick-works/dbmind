import { pgFamilyOptions } from './presets.js'

/**
 * RisingWave 类型定义（流式数据库，走 PostgreSQL 协议 + PG 驱动）
 *
 * 与 postgresql.js 的差别：
 *   - 默认端口 4566（PG wire；dashboard 是 5691）
 *   - 默认库 dev、默认用户 root（无密码）
 *   - 归 OLAP：它面向实时分析（物化视图持续更新），不是事务库
 *   - 没有二级索引 / 触发器 / 事件，所以索引与结构编辑相关能力关闭
 */
export default {
  code: 'RISINGWAVE',
  // 上游兼容接口的模块路径段：与 plugins/connection-types/risingwave.yaml 的 dialect 派生值一致（guard 校验）
  module: 'postgresql',
  label: 'RisingWave',
  category: 'OLAP',
  noSql: false,
  schemaLevel: 'schema',
  defaultSchema: 'public',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'risingwave.svg',
  fallbackLogo: 'risingwave',
  fileType: false,
  defaultPort: 4566,
  databasePh: 'cd.dbPhOptional',
  keepEmptyDbSlash: true,
  // 与 postgresql 同一条模板（同一个驱动）
  urlTemplate: 'jdbc:postgresql://{host}:{port}/{database}?sslmode=prefer&reWriteBatchedInserts=true&stringtype=unspecified&tcpKeepAlive=true',
  options: pgFamilyOptions(),
  dropParams: ['ApplicationName', 'sslmode', 'currentSchema', 'reWriteBatchedInserts',
    'stringtype', 'tcpKeepAlive'],
  createDatabaseSql(name) {
    const n = String(name ?? '').trim().replace(/"/g, '""')
    return `CREATE DATABASE "${n}"`
  },
  showDbDdlSql(db) {
    return `SELECT 'CREATE DATABASE ' || quote_ident(datname) AS ddl FROM pg_database WHERE datname = '${String(db ?? '')}'`
  }
}
