import { mysqlFamilyOptions } from './presets.js'

/**
 * GreatSQL 类型定义（MySQL 协议，走 MySQL 驱动）
 *
 * 万里开源的 MySQL 分支，协议与语法层兼容 MySQL 8.0：连接表单、URL 模板、
 * 建库语句全部复用 MySQL 那一套。supportLevel=experimental（同族批量注册，
 * 纵深功能未逐一真机验证）。
 */
export default {
  code: 'GREATSQL',
  // 上游兼容接口的模块路径段：与 plugins/connection-types/greatsql.yaml 的 dialect 派生值一致（guard 校验）
  module: 'mysql',
  label: 'GreatSQL',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: '',
  fallbackLogo: 'greatsql',
  fileType: false,
  defaultPort: 3306,
  databasePh: 'cd.dbPhOptional',
  // 与 MySQL 同一条模板（同一个驱动）：默认值说明见 mysql.js 的注释
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
