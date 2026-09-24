import { formParts } from './base'

/** Redis 类型定义（NoSQL） */
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
  buildJdbcUrl(f) {
    const { host, port, db } = formParts(f)
    if (!host) return ''
    const p = port || 6379
    return `redis://${host}:${p}/${db || '0'}`
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
