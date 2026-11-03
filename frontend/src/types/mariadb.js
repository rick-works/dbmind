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
  urlTemplate: 'jdbc:mariadb://{host}:{port}/{database}?useUnicode=true&characterEncoding=UTF-8&connectTimeout=10000&socketTimeout=600000&rewriteBatchedStatements=true&useCursorFetch=true&zeroDateTimeBehavior=CONVERT_TO_NULL&allowMultiQueries=true',
  options: mysqlFamilyOptions(),
  // 表单里已删掉的选项：老连接里的残留值在回显时丢掉（默认值已在模板里）
  dropParams: ['sslMode', 'characterEncoding', 'rewriteBatchedStatements',
    'zeroDateTimeBehavior', 'allowMultiQueries'],
  createDatabaseSql(name, f = {}) {
    const n = String(name ?? '').trim().replace(/`/g, '``')
    // 不选 = 跟随服务器默认（同 MySQL：别把 utf8mb4_general_ci 写死成默认）
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
