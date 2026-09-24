import { formParts } from './base'

/** H2 类型定义（本地文件型/内存型） */
export default {
  code: 'H2',
  label: 'H2',
  category: 'RELATIONAL_FILE',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: '',
  fileType: true,
  defaultPort: 9092,
  fallbackLogo: 'h2',
  buildJdbcUrl(f) {
    const { file } = formParts(f)
    return file ? `jdbc:h2:file:${file}` : 'jdbc:h2:mem:上游;DB_CLOSE_DELAY=-1'
  },
  // H2 的「数据库」即文件/内存，概念上对应 SCHEMA，不在 SQL 层用 CREATE DATABASE 建库
  createDatabaseSql() {
    return ''
  },
  // 文件/内存型，无建库语句
  showDbDdlSql() {
    return null
  }
}
