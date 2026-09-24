import { formParts } from './base'

/** Elasticsearch 类型定义（NoSQL） */
export default {
  code: 'ELASTICSEARCH',
  label: 'Elasticsearch',
  category: 'NOSQL',
  noSql: true,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: '',
  fileType: false,
  defaultPort: 9200,
  credentials: false,
  protocols: ['http', 'https'],
  fallbackLogo: 'elasticsearch',
  buildJdbcUrl(f) {
    const { host, port, esProtocol } = formParts(f)
    if (!host) return ''
    const p = port || 9200
    return `${esProtocol}://${host}:${p}`
  },
  // Elasticsearch 无「数据库」概念（索引即顶层），不需要建库语句
  createDatabaseSql() {
    return ''
  },
  // NoSQL：索引即顶层，无建库语句
  showDbDdlSql() {
    return null
  }
}
