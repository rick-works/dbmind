import { formParts } from './base'

/** Apache Derby 类型定义（本地文件型） */
export default {
  code: 'DERBY',
  label: 'Apache Derby',
  category: 'RELATIONAL_FILE',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'derby.svg',
  fileType: true,
  defaultPort: 1527,
  fileHint: 'tdef.derbyFileHint',
  buildJdbcUrl(f) {
    const { file } = formParts(f)
    return file ? `jdbc:derby:${file};create=true` : 'jdbc:derby:memory:上游;create=true'
  },
  // Derby 连接时自动建库（;create=true），无需 SQL 层 CREATE DATABASE
  createDatabaseSql() {
    return ''
  },
  // 连接即建库，无独立建库语句
  showDbDdlSql() {
    return null
  }
}
