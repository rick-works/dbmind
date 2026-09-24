import { formParts } from './base'

/** Apache Doris 类型定义（MySQL 语法家族） */
export default {
  code: 'DORIS',
  label: 'Apache Doris',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: 'Doris.svg',
  fileType: false,
  defaultPort: 9030,
  buildJdbcUrl(f) {
    const { host, port, db, charset, ct, st } = formParts(f)
    if (!host) return ''
    const p = port || 9030
    return `jdbc:mysql://${host}:${p}/${db}?useUnicode=true&characterEncoding=${charset}&useSSL=false&serverTimezone=Asia/Shanghai&allowPublicKeyRetrieval=true&connectTimeout=${ct}&socketTimeout=${st}&rewriteBatchedStatements=true&useCursorFetch=true`
  },
  createDatabaseSql(name) {
    // Doris 不支持 CHARACTER SET / COLLATE 子句（内部固定 UTF-8）
    const n = String(name ?? '').trim().replace(/`/g, '``')
    return `CREATE DATABASE \`${n}\``
  },
  showDbDdlSql(db) {
    return `SHOW CREATE DATABASE \`${String(db ?? '').replace(/`/g, '``')}\``
  }
}
