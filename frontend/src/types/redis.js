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
  // 宿主（agents/dbmind-agent-redis）本来就支持 AUTH：有用户名走 Redis 6 的 ACL
  // （`AUTH user pass`），没有就走经典 `AUTH pass`。**密码框必须显示出来** ——
  // 以前这里写 false，带密码的实例根本无法连接（实测报 NOAUTH Authentication required）。
  credentials: true,
  // 用户名可留空（= 服务端默认用户 default）；密码也可留空（本地无认证实例）
  usernameRequired: false,
  passwordRequired: false,
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
