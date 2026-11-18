
import { mysqlFamilyOptions } from './presets.js'

/**
 * OceanBase 类型定义（MySQL 模式，走 MySQL 驱动）
 *
 * 语法层兼容 MySQL 5.7/8.0：连接表单、URL 模板、建库语句全部复用 MySQL
 * 那一套。Oracle 模式是另一套方言，不在这个类型里。
 */
export default {
  code: 'OCEANBASE',
  webScheme: 'mysql',
  formatDialect: 'mysql',
  canDropDatabase: true,
  // 例程 / 触发器 / 事件按 MySQL 协议没有 ALTER（改定义只能 DROP + CREATE）
  dropCreateKinds: ['procedure', 'function', 'trigger', 'event'],
  // 上游兼容接口的模块路径段：与 plugins/connection-types/oceanbase.yaml 的 dialect 派生值一致（guard 校验）
  module: 'mysql',
  label: 'OceanBase',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: 'oceanbase.svg',
  fileType: false,
  fallbackLogo: 'oceanbase',
  defaultPort: 2881,
  databasePh: 'cd.dbPhOptional',
  // 与 MySQL 同一条模板（同一个驱动）：默认值见 mysql.js 的注释
  urlTemplate: 'jdbc:mysql://{host}:{port}/{database}?useUnicode=true&characterEncoding=UTF-8&useSSL=false&serverTimezone=Asia/Shanghai&allowPublicKeyRetrieval=true&connectTimeout=10000&socketTimeout=600000&rewriteBatchedStatements=true&useCursorFetch=true&zeroDateTimeBehavior=CONVERT_TO_NULL&allowMultiQueries=true',
  options: mysqlFamilyOptions(),
  dropParams: ['useSSL', 'characterEncoding', 'serverTimezone', 'allowPublicKeyRetrieval',
    'rewriteBatchedStatements', 'zeroDateTimeBehavior', 'allowMultiQueries'],
  createDatabaseSql(name, f = {}) {
    const n = String(name ?? '').trim().replace(/`/g, '``')
    // OB 支持 CHARACTER SET / COLLATE，默认就是 utf8mb4 —— 不填就不写
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
