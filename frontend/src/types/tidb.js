import { mysqlFamilyOptions } from './presets.js'

/**
 * TiDB 类型定义（MySQL 协议，走 MySQL 驱动）
 *
 * 与 MySQL 的差别只在服务端：默认端口 4000、分布式事务、默认 utf8mb4；
 * 表/列/索引/权限这些都按 MySQL 语法走，所以连接表单、URL 模板、建库语句
 * 全部复用 MySQL 那一套。
 */
export default {
  code: 'TIDB',
  webScheme: 'mysql',
  formatDialect: 'mysql',
  canDropDatabase: true,
  // 例程 / 触发器 / 事件按 MySQL 协议没有 ALTER（改定义只能 DROP + CREATE）
  dropCreateKinds: ['procedure', 'function', 'trigger', 'event'],
  // 上游兼容接口的模块路径段：与 plugins/connection-types/tidb.yaml 的 dialect 派生值一致（guard 校验）
  module: 'mysql',
  label: 'TiDB',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: 'tidb.svg',
  fileType: false,
  defaultPort: 4000,
  databasePh: 'cd.dbPhOptional',
  // 与 MySQL 同一条模板（同一个驱动）：默认值说明见 mysql.js 的注释
  urlTemplate: 'jdbc:mysql://{host}:{port}/{database}?useUnicode=true&characterEncoding=UTF-8&useSSL=false&serverTimezone=Asia/Shanghai&allowPublicKeyRetrieval=true&connectTimeout=10000&socketTimeout=600000&rewriteBatchedStatements=true&useCursorFetch=true&zeroDateTimeBehavior=CONVERT_TO_NULL&allowMultiQueries=true',
  options: mysqlFamilyOptions(),
  // 表单里已删掉的选项：老连接的残留值在回显时丢掉（默认值已写进上面的模板）
  dropParams: ['useSSL', 'characterEncoding', 'serverTimezone', 'allowPublicKeyRetrieval',
    'rewriteBatchedStatements', 'zeroDateTimeBehavior', 'allowMultiQueries'],
  createDatabaseSql(name, f = {}) {
    const n = String(name ?? '').trim().replace(/`/g, '``')
    // TiDB 支持 CHARACTER SET / COLLATE，但默认就是 utf8mb4 —— 不填就不写，避免覆盖服务器默认
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
