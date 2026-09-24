import { formParts } from './base'

/** MongoDB 类型定义（NoSQL） */
export default {
  code: 'MONGODB',
  label: 'MongoDB',
  category: 'NOSQL',
  noSql: true,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'mongodb.svg',
  fileType: false,
  defaultPort: 27017,
  credentials: false,
  buildJdbcUrl(f) {
    const { host, port, db, ct, st } = formParts(f)
    if (!host) return ''
    const p = port || 27017
    return `mongodb://${host}:${p}/${db}?connectTimeoutMS=${ct}&serverSelectionTimeoutMS=${st}`
  },
  // MongoDB 库随集合创建隐式生成，不需要显式建库语句
  createDatabaseSql() {
    return ''
  },
  // NoSQL：库随集合隐式生成，无建库语句
  showDbDdlSql() {
    return null
  }
}
