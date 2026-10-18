/**
 * Redis 类型定义（NoSQL，内核原生协议）
 *
 * 连接串里的 `/N` 是 DB 编号（SELECT N），不是库名。
 */
export default {
  code: 'REDIS',
  label: 'Redis',
  category: 'NOSQL',
  noSql: true,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'redis.svg',
  fileType: false,
  defaultPort: 6379,
  credentials: false,
  databaseLabel: 'tdef.dbNo',
  databasePh: 'cd.dbPhOptional',
  buildJdbcUrl(f) {
    const host = String(f?.host ?? '').trim()
    if (!host) return ''
    const p = f?.port || 6379
    return `redis://${host}:${p}/${f?.database || '0'}`
  },
  // Redis 的「库」只是数字编号（SELECT n），不存在创建概念
  createDatabaseSql() {
    return ''
  },
  // NoSQL：库为数字编号，无建库语句
  showDbDdlSql() {
    return null
  }
}
