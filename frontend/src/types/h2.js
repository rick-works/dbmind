import { h2Options } from './presets.js'

/**
 * H2 类型定义（本地文件型）
 *
 * 与 plugins/connection-types/h2.yaml 一致：`jdbc:h2:file:{filePath}`。
 * MODE / AUTO_SERVER 这类引擎设置只能写在 URL 上（不是连接属性），走结构化选项。
 */
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
  urlTemplate: 'jdbc:h2:file:{filePath}',
  propStyle: 'semicolon',
  options: h2Options(),
  // H2 的「数据库」即文件/内存，概念上对应 SCHEMA，不在 SQL 层用 CREATE DATABASE 建库
  createDatabaseSql() {
    return ''
  },
  // 文件/内存型，无建库语句
  showDbDdlSql() {
    return null
  }
}
