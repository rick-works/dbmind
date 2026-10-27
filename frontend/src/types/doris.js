import { mysqlFamilyOptions } from './presets.js'

/**
 * Apache Doris 类型定义（MySQL 协议，走 MySQL 驱动）
 *
 * 与 MySQL 的区别只在服务端：Doris 内部固定 UTF-8、不支持 CHARACTER SET/COLLATE，
 * 也不认 MySQL 的 prepare/游标那套，所以选项里给的是两边都有的公共项。
 */
export default {
  code: 'DORIS',
  label: 'Apache Doris',
  // 列存分析引擎（MySQL 协议只是兼容层）：归 OLAP，不跟 MySQL 一起算关系型
  category: 'OLAP',
  noSql: false,
  schemaLevel: 'none',
  // 连接下还有一层 catalog（树里多一层、列库要带 catalog 参数）—— 只有 Doris 是这样
  catalogs: true,
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: 'Doris.svg',
  fileType: false,
  defaultPort: 9030,
  databasePh: 'cd.dbPhOptional',
  // 与 MySQL 同一条模板（同一个驱动）：默认值见 mysql.js 的注释
  urlTemplate: 'jdbc:mysql://{host}:{port}/{database}?useUnicode=true&characterEncoding=UTF-8&useSSL=false&serverTimezone=Asia/Shanghai&allowPublicKeyRetrieval=true&connectTimeout=10000&socketTimeout=600000&rewriteBatchedStatements=true&useCursorFetch=true&zeroDateTimeBehavior=CONVERT_TO_NULL&allowMultiQueries=true',
  options: mysqlFamilyOptions(),
  // 表单里已删掉的选项：老连接里的残留值在回显时丢掉（默认值已在模板里）
  dropParams: ['useSSL', 'characterEncoding', 'serverTimezone', 'allowPublicKeyRetrieval',
    'rewriteBatchedStatements', 'zeroDateTimeBehavior', 'allowMultiQueries'],
  createDatabaseSql(name) {
    // Doris 不支持 CHARACTER SET / COLLATE 子句（内部固定 UTF-8）
    const n = String(name ?? '').trim().replace(/`/g, '``')
    return `CREATE DATABASE \`${n}\``
  },
  showDbDdlSql(db) {
    // catalog 方言：树/页签里的库是 `catalog.库` 全限定名（如 internal.ods）。
    // 执行时后端已 `USE internal.ods` 进入该库，SHOW CREATE DATABASE 只要**裸库名** ——
    // 整串丢进去会报 Unknown database 'internal.ods'（真机踩过）。
    const s = String(db ?? '')
    const i = s.indexOf('.')
    const name = i > 0 ? s.slice(i + 1) : s
    return `SHOW CREATE DATABASE \`${name.replace(/`/g, '``')}\``
  }
}
