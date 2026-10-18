import { mysqlFamilyOptions } from './presets.js'

/**
 * MariaDB 类型定义（MySQL 语法家族）
 *
 * 与 MySQL 的差别：模板里没有 serverTimezone（MariaDB 驱动按 JVM 时区走，
 * 需要时用启动参数 -Duser.timezone 指定），SSL 走 sslMode 系列选项。
 */
export default {
  code: 'MARIADB',
  label: 'MariaDB',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: 'mariadb.svg',
  fileType: false,
  defaultPort: 3306,
  databasePh: 'cd.dbPhOptional',
  // SSL 走驱动默认（sslMode=disable）；字符集/批量写/零日期/多语句都固定在模板里
  urlTemplate: 'jdbc:mariadb://{host}:{port}/{database}?useUnicode=true&characterEncoding=UTF-8&connectTimeout=10000&socketTimeout=0&rewriteBatchedStatements=true&useCursorFetch=true&zeroDateTimeBehavior=CONVERT_TO_NULL&allowMultiQueries=true',
  options: mysqlFamilyOptions(),
  // 表单里已删掉的选项：老连接里的残留值在回显时丢掉（默认值已在模板里）
  dropParams: ['sslMode', 'characterEncoding', 'rewriteBatchedStatements',
    'zeroDateTimeBehavior', 'allowMultiQueries'],
  createDatabaseSql(name, f = {}) {
    const n = String(name ?? '').trim().replace(/`/g, '``')
    return `CREATE DATABASE \`${n}\` CHARACTER SET ${f?.charset || 'utf8mb4'} COLLATE ${f?.collation || 'utf8mb4_general_ci'}`
  },
  showDbDdlSql(db) {
    return `SHOW CREATE DATABASE \`${String(db ?? '').replace(/`/g, '``')}\``
  }
}
