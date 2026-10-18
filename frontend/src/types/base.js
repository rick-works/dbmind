import { buildUrlFor } from './jdbc.js'

/**
 * 数据库类型注册表 —— 基础默认定义（base）
 *
 * 前端类型插件化架构：每种数据库类型在 src/types/ 下用一个独立 .js 文件声明其差异，
 * index.js 通过 import.meta.glob 自动收集，新增类型只需新增文件、零改动现有代码。
 *
 * 能力键说明（code 一律用大写类型码，与后端 DatabaseType.name() 一致）：
 *  - label         类型展示名（与后端 DatabaseType.label 保持一致，作为前端单一来源）
 *  - category      大类：RELATIONAL / OLAP / RELATIONAL_FILE / NOSQL（同后端）
 *  - noSql         是否 NoSQL（替代散落各处的白名单判断）
 *  - schemaLevel   导航/查询是否需要 schema 层级：'none' | 'schema'
 *  - defaultSchema schema 层级下默认选中的 schema（如 SQL Server 的 dbo）
 *  - quoteStyle    标识符引号风格：BACKTICK / BRACKET / DOUBLE_QUOTE（同后端 quoteStyle）
 *  - logo          图标 key（P3 由 DbLogo 映射到 assets/type-*.svg）
 *  - fileType      是否本地文件型数据库（无 host/端口，连接表单据此隐藏主机配置）
 *  - defaultPort   默认端口（与后端 DatabaseType 一致；连接表单兜底用）
 *  - credentials   连接表单是否显示用户名/密码（NoSQL 默认 false）
 *  - authTypes     认证方式选项（[{ value, label }]）；非空时连接表单显示「认证方式」下拉，
 *                  如 SQL Server 的 SQL Server 身份验证 / Windows 集成身份验证
 *  - urlTemplate   JDBC URL 模板：**必须与 plugins/connection-types/<type>.yaml 的
 *                  jdbc.urlTemplate 逐字一致**（那是后端真正拿去连库的那条）
 *  - propStyle     驱动参数在预览 URL 里的接法：query（默认）/ semicolon / db2 / none
 *  - options       类型专属连接选项（字段声明见 presets.js 顶部注释）
 *  - buildJdbcUrl  连接表单只读预览用连接串（默认实现 = 模板 + 结构化选项 + 驱动参数）
 *  - createDatabaseSql(name, form)  返回建库执行 SQL；返回空串表示该类型不支持（UI 给出提示）
 *  - showDbDdlSql(db)  返回"获取现有库建库语句"的查询 SQL；返回 null 表示无（UI 显示注释）
 */
export default {
  code: '',
  label: '',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: '',
  fileType: false,
  defaultPort: 0,
  credentials: true,
  authTypes: [],
  databaseLabel: 'tdef.dbName',
  // 库名默认**可留空**：MySQL / PG / SQL Server / ClickHouse 等都允许先连上、再在树里选库。
  // 只有语义上必须给的类型（Oracle 的服务名 / SID、DB2 的库名）在自己文件里置 true。
  databaseRequired: false,
  // 输入框占位提示 / 「必填未填」提示：都是 i18n 键，按类型各写各的，别让 Oracle 的措辞
  // 跑到 MySQL 上（用户反馈过："MySQL 的库名提示里为什么写着服务名 / SID"）。
  databasePh: '',
  databaseRequiredMsg: '',
  fileHint: '',
  protocols: [],
  fallbackLogo: 'db',
  urlTemplate: '',
  propStyle: 'query',
  options: [],
  buildJdbcUrl(f) {
    return buildUrlFor(this, f)
  },
  createDatabaseSql(name) {
    const n = String(name ?? '').trim().replace(/`/g, '``')
    return `CREATE DATABASE \`${n}\``
  },
  showDbDdlSql(db) {
    const n = String(db ?? '').replace(/"/g, '""')
    return `SELECT 'CREATE DATABASE "${n}";' AS ddl`
  }
}
