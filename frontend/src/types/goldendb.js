import { mysqlFamilyOptions } from './presets.js'

/**
 * GoldenDB 类型定义（分布式数据库，计算节点对外是 MySQL 协议）
 *
 * 方言、请求模块、驱动全部复用 MySQL 那一套；默认端口按 MySQL 惯例 3306，
 * 现场部署改过端口时在表单里改即可。supportLevel=experimental。
 */
export default {
  code: 'GOLDENDB',
  // 与 plugins/connection-types/goldendb.yaml 的 dialect 派生值一致（guard 校验）
  module: 'mysql',
  label: 'GoldenDB',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: '',
  fallbackLogo: 'goldendb',
  fileType: false,
  defaultPort: 3306,
  databasePh: 'cd.dbPhOptional',
  urlTemplate: 'jdbc:mysql://{host}:{port}/{database}?useUnicode=true&characterEncoding=UTF-8&useSSL=false&serverTimezone=Asia/Shanghai&allowPublicKeyRetrieval=true&connectTimeout=10000&socketTimeout=600000&rewriteBatchedStatements=true&useCursorFetch=true&zeroDateTimeBehavior=CONVERT_TO_NULL&allowMultiQueries=true',
  options: mysqlFamilyOptions(),
  dropParams: ['useSSL', 'characterEncoding', 'serverTimezone', 'allowPublicKeyRetrieval',
    'rewriteBatchedStatements', 'zeroDateTimeBehavior', 'allowMultiQueries'],
  createDatabaseSql(name, f = {}) {
    const n = String(name ?? '').trim().replace(/`/g, '``')
    const charset = String(f?.charset || '').trim()
    const collation = String(f?.collation || '').trim()
    const suffix = collation ? ` CHARACTER SET ${charset || collation.split('_')[0]} COLLATE ${collation}`
      : charset ? ` CHARACTER SET ${charset}` : ''
    return `CREATE DATABASE \`${n}\`${suffix}`
  },
  showDbDdlSql(db) {
    return `SHOW CREATE DATABASE \`${String(db ?? '').replace(/`/g, '``')}\``
  }
}
