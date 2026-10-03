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
    // catalog 方言：树/页签里的库是 `catalog.库` 全限定名（如 internal.ods）。
    // 执行时后端已 `USE internal.ods` 进入该库，SHOW CREATE DATABASE 只要**裸库名** ——
    // 整串丢进去会报 Unknown database 'internal.ods'（真机踩过）。
    const s = String(db ?? '')
    const i = s.indexOf('.')
    const name = i > 0 ? s.slice(i + 1) : s
    return `SHOW CREATE DATABASE \`${name.replace(/`/g, '``')}\``
  }
}
