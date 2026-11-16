
/**
 * DuckDB 类型定义（本地文件型，嵌入式 OLAP）
 *
 * 与 plugins/connection-types/duckdb.yaml 一致：`jdbc:duckdb:{filePath}`。
 * 单文件数据库、单连接池（singleConnectionPool），没有 host / 端口 ——
 * 连接表单按 fileType 隐藏主机配置，只选本地文件。
 * SQL 是 ANSI 方言（引用符双引号），内核侧 dialect 落 generic 的 DuckDB 实现。
 */
export default {
  code: 'DUCKDB',
  // 上游兼容接口的模块路径段：与 plugins/connection-types/duckdb.yaml 的 dialect 派生值一致（guard 校验）
  module: 'duckdb',
  label: 'DuckDB',
  category: 'RELATIONAL_FILE',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: 'main',
  quoteStyle: 'DOUBLE_QUOTE',
  // 官方品牌 SVG（assets/duckdb.svg）；内联鸭形只作文件缺失时的兜底
  logo: 'duckdb.svg',
  fileType: true,
  fallbackLogo: 'duckdb',
  urlTemplate: 'jdbc:duckdb:{filePath}',
  // DuckDB 无建库概念：库即文件，仅在打开文件后 CREATE SCHEMA
  createDatabaseSql() {
    return ''
  },
  showDbDdlSql() {
    return null
  }
}
