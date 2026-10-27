/**
 * MongoDB 类型定义（NoSQL，内核原生协议）
 *
 * 这里展示的是 mongodb 标准连接串（不是 JDBC —— 内核走 Mongo 协议）。
 * 账号口令由宿主按连接字段使用，库名（database）就是默认鉴权/操作库。
 */
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
  // 宿主支持 SCRAM 认证（用户名/密码拼进连接串），所以必须能填 —— 以前藏起来时
  // 开了认证的实例只能连失败。用户名/密码都可留空（本地无认证实例）
  credentials: true,
  usernameRequired: false,
  passwordRequired: false,
  // 库名可留空（连接后仍能列库、选库）
  databasePh: 'cd.dbPhOptional',
  buildJdbcUrl(f) {
    const host = String(f?.host ?? '').trim()
    if (!host) return ''
    const p = f?.port || 27017
    const db = String(f?.database ?? '').trim()
    return `mongodb://${host}:${p}/${db}`
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
