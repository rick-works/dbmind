/**
 * Elasticsearch 类型定义（NoSQL，HTTP 协议）
 *
 * 索引即顶层，没有「库」的概念；协议（http/https）由表单的「协议」下拉决定。
 */
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
  // 索引即顶层，库名可留空
  databasePh: 'cd.dbPhOptional',
  protocols: ['http', 'https'],
  fallbackLogo: 'elasticsearch',
  buildJdbcUrl(f) {
    const host = String(f?.host ?? '').trim()
    if (!host) return ''
    const p = f?.port || 9200
    return `${f?.esProtocol || 'http'}://${host}:${p}`
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
