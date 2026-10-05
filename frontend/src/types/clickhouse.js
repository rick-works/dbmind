import { formParts } from './base'

/** ClickHouse 类型定义 */
export default {
  code: 'CLICKHOUSE',
  label: 'ClickHouse',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: 'ClickHouse.svg',
  fileType: false,
  defaultPort: 8123,
  buildJdbcUrl(f) {
    const { host, port, db, ct, st } = formParts(f)
    if (!host) return ''
    const p = port || 8123
    // compress=0 禁用 LZ4 压缩：驱动缺 lz4-java 时握手会失败（Magic is not correct）
    return `jdbc:clickhouse://${host}:${p}/${db}?connect_timeout=${ct}&socket_timeout=${st}&compress=0`
  },
  // ClickHouse 使用反引号标识符
  createDatabaseSql(name) {
    const n = String(name ?? '').trim().replace(/`/g, '``')
    return `CREATE DATABASE \`${n}\``
  },
  // ClickHouse 原生支持 SHOW CREATE DATABASE
  showDbDdlSql(db) {
    return `SHOW CREATE DATABASE \`${String(db ?? '').replace(/`/g, '``')}\``
  }
}
