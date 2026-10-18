import { clickhouseOptions } from './presets.js'

/**
 * ClickHouse 类型定义
 *
 * 模板里两条「不能删」的固定项，注释见 plugins/connection-types/clickhouse.yaml：
 *   compress=0                       缺 lz4-java 时握手会失败
 *   result_overflow_mode=break       行数上限触发时截断返回，而不是整条查询报 Code 396
 *   http_connection_provider=...     规避 Windows 上 Apache HttpClient 的首连 2s+ 开销
 */
export default {
  code: 'CLICKHOUSE',
  label: 'ClickHouse',
  // 列存分析引擎：不是关系型库（无事务 / 外键 / 二级索引），单列 OLAP 分类
  category: 'OLAP',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: 'ClickHouse.svg',
  fileType: false,
  defaultPort: 8123,
  databasePh: 'cd.dbPhOptional',
  urlTemplate: 'jdbc:clickhouse://{host}:{port}/{database}?compress=0&result_overflow_mode=break&http_connection_provider=HTTP_URL_CONNECTION',
  options: clickhouseOptions(),
  // 表单里已删掉的选项（压缩 / HTTP 实现 / 最大执行时间）：前两项已在模板里，
  // 最后一项驱动默认就是 0（不限），历史残留值在回显时丢掉
  dropParams: ['compress', 'http_connection_provider', 'max_execution_time'],
  // ClickHouse 使用反引号标识符
  createDatabaseSql(name) {
    const n = String(name ?? '').trim().replace(/`/g, '``')
    return `CREATE DATABASE \`${n}\``
  },
  // ClickHouse 原生支持 SHOW CREATE DATABASE
  showDbDdlSql(db) {
    return `SHOW CREATE DATABASE \`${String(db ?? '').replace(/`/g, '``')}\``
  }
}
