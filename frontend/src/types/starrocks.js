
import { mysqlFamilyOptions } from './presets.js'

/**
 * StarRocks 类型定义（MySQL 协议，走 MySQL 驱动）
 *
 * Apache Doris 的分叉（早期同源），与 Doris 的差异只在服务端内核：
 * 连接表单、URL 模板、建库语句全部复用 Doris 那一套；服务端行为
 * （内部固定 UTF-8、不支持 CHARACTER SET/COLLATE）也一致。
 */
export default {
  code: 'STARROCKS',
  webScheme: 'mysql',
  formatDialect: 'mysql',
  canDropDatabase: true,
  // 例程 / 触发器 / 事件按 MySQL 协议没有 ALTER（改定义只能 DROP + CREATE）
  dropCreateKinds: ['procedure', 'function', 'trigger', 'event'],
  // 上游兼容接口的模块路径段：与 plugins/connection-types/starrocks.yaml 的 dialect 派生值一致（guard 校验）
  module: 'mysql',
  label: 'StarRocks',
  // 列存分析引擎（MySQL 协议只是兼容层）：归 OLAP，不跟 MySQL 一起算关系型
  category: 'OLAP',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: 'starrocks.svg',
  fileType: false,
  fallbackLogo: 'starrocks',
  defaultPort: 9030,
  databasePh: 'cd.dbPhOptional',
  // 与 MySQL 同一条模板（同一个驱动）：默认值见 mysql.js 的注释
  urlTemplate: 'jdbc:mysql://{host}:{port}/{database}?useUnicode=true&characterEncoding=UTF-8&useSSL=false&serverTimezone=Asia/Shanghai&allowPublicKeyRetrieval=true&connectTimeout=10000&socketTimeout=600000&rewriteBatchedStatements=true&useCursorFetch=true&zeroDateTimeBehavior=CONVERT_TO_NULL&allowMultiQueries=true',
  options: mysqlFamilyOptions(),
  dropParams: ['useSSL', 'characterEncoding', 'serverTimezone', 'allowPublicKeyRetrieval',
    'rewriteBatchedStatements', 'zeroDateTimeBehavior', 'allowMultiQueries'],
  createDatabaseSql(name) {
    // StarRocks 不支持 CHARACTER SET / COLLATE 子句（内部固定 UTF-8，与 Doris 一致）
    const n = String(name ?? '').trim().replace(/`/g, '``')
    return `CREATE DATABASE \`${n}\``
  },
  showDbDdlSql(db) {
    const s = String(db ?? '').replace(/`/g, '``')
    return `SHOW CREATE DATABASE \`${s}\``
  }
}
