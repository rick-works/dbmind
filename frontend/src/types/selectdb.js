import { mysqlFamilyOptions } from './presets.js'

/**
 * SelectDB 类型定义（Apache Doris 商业发行版，MySQL 协议，走 Doris 的驱动）
 *
 * 与 Doris 同源：内部固定 UTF-8、不支持 CHARACTER SET/COLLATE，连接选项与建库
 * 语句都按 Doris 那套写。catalog 层刻意不开（不同版本对 SHOW CATALOGS 支持不一，
 * 多一层请求只会在老版本上多出一条报错）。
 */
export default {
  code: 'SELECTDB',
  // 与 plugins/connection-types/selectdb.yaml 的 dialect 派生值一致（guard 校验）
  module: 'mysql',
  label: 'SelectDB',
  // 列存分析引擎：归 OLAP
  category: 'OLAP',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: '',
  fallbackLogo: 'selectdb',
  fileType: false,
  defaultPort: 9030,
  databasePh: 'cd.dbPhOptional',
  urlTemplate: 'jdbc:mysql://{host}:{port}/{database}?useUnicode=true&characterEncoding=UTF-8&useSSL=false&serverTimezone=Asia/Shanghai&allowPublicKeyRetrieval=true&connectTimeout=10000&socketTimeout=600000&rewriteBatchedStatements=true&useCursorFetch=true&zeroDateTimeBehavior=CONVERT_TO_NULL&allowMultiQueries=true',
  options: mysqlFamilyOptions(),
  dropParams: ['useSSL', 'characterEncoding', 'serverTimezone', 'allowPublicKeyRetrieval',
    'rewriteBatchedStatements', 'zeroDateTimeBehavior', 'allowMultiQueries'],
  createDatabaseSql(name) {
    const n = String(name ?? '').trim().replace(/`/g, '``')
    return `CREATE DATABASE \`${n}\``
  },
  showDbDdlSql(db) {
    return `SHOW CREATE DATABASE \`${String(db ?? '').replace(/`/g, '``')}\``
  }
}
