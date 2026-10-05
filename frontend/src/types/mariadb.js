import { formParts } from './base'

/** MariaDB 类型定义（MySQL 语法家族） */
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
  buildJdbcUrl(f) {
    const { host, port, db, charset, ct, st } = formParts(f)
    if (!host) return ''
    const p = port || 3306
    return `jdbc:mariadb://${host}:${p}/${db}?useUnicode=true&characterEncoding=${charset}&connectTimeout=${ct}&socketTimeout=${st}&rewriteBatchedStatements=true&useCursorFetch=true`
  },
  createDatabaseSql(name, f = {}) {
    const n = String(name ?? '').trim().replace(/`/g, '``')
    return `CREATE DATABASE \`${n}\` CHARACTER SET ${f?.charset || 'utf8mb4'} COLLATE ${f?.collation || 'utf8mb4_general_ci'}`
  },
  showDbDdlSql(db) {
    return `SHOW CREATE DATABASE \`${String(db ?? '').replace(/`/g, '``')}\``
  }
}
