import { mysqlFamilyOptions } from './presets.js'

/**
 * Databend 类型定义（云原生数仓，走 MySQL 协议 + MySQL 驱动）
 *
 * 与 mysql.js 的差别：
 *   - 默认端口 3307（Databend 默认把 MySQL 兼容端口放在 3307，HTTP 8000）
 *   - 建库不支持 CHARACTER SET / COLLATE，用最朴素的 CREATE DATABASE
 *   - 归 OLAP 分组（列存分析引擎，MySQL 协议只是兼容层）
 */
export default {
  code: 'DATABEND',
  webScheme: 'mysql',
  formatDialect: 'mysql',
  canDropDatabase: true,
  // 上游兼容接口的模块路径段：与 plugins/connection-types/databend.yaml 的 dialect 派生值一致（guard 校验）
  module: 'mysql',
  label: 'Databend',
  category: 'OLAP',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: 'databend.svg',
  fallbackLogo: 'databend',
  fileType: false,
  defaultPort: 3307,
  databasePh: 'cd.dbPhOptional',
  // 与 MySQL 同一条模板（同一个驱动）
  urlTemplate: 'jdbc:mysql://{host}:{port}/{database}?useUnicode=true&characterEncoding=UTF-8&useSSL=false&serverTimezone=Asia/Shanghai&allowPublicKeyRetrieval=true&connectTimeout=10000&socketTimeout=600000&rewriteBatchedStatements=true&useCursorFetch=true&zeroDateTimeBehavior=CONVERT_TO_NULL&allowMultiQueries=true',
  options: mysqlFamilyOptions(),
  dropParams: ['useSSL', 'characterEncoding', 'serverTimezone', 'allowPublicKeyRetrieval',
    'rewriteBatchedStatements', 'zeroDateTimeBehavior', 'allowMultiQueries'],
  createDatabaseSql(name) {
    // Databend 建库不认 CHARACTER SET / COLLATE
    const n = String(name ?? '').trim().replace(/`/g, '``')
    return `CREATE DATABASE \`${n}\``
  },
  showDbDdlSql(db) {
    return `SHOW CREATE DATABASE \`${String(db ?? '').replace(/`/g, '``')}\``
  }
}
