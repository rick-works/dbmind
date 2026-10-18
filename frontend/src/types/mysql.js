import { mysqlFamilyOptions } from './presets.js'

/**
 * MySQL 类型定义
 *
 * urlTemplate 与 plugins/connection-types/mysql.yaml 的 jdbc.urlTemplate **逐字一致**：
 * 表单里的 URL 预览 = 后端真正会用的那条。改一处必须同步另一处。
 *
 * 表单里不再出现的那些驱动参数，默认值都在这条模板上（用户不用懂，也不用改）：
 *   useUnicode / characterEncoding=UTF-8  —— 不乱码
 *   useSSL=false + allowPublicKeyRetrieval=true —— MySQL 8 的 caching_sha2_password
 *                                          在自签/无证书环境下也能认证
 *   serverTimezone=Asia/Shanghai          —— 不报「time zone value … unrecognized」
 *   zeroDateTimeBehavior=CONVERT_TO_NULL  —— 老库的 0000-00-00 读成 NULL，而不是抛错
 *   allowMultiQueries=true                —— 一次发多条（整段脚本执行靠它）
 *   rewriteBatchedStatements / useCursorFetch —— 批量写、大结果集各走快路
 */
export default {
  code: 'MYSQL',
  label: 'MySQL',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'BACKTICK',
  logo: 'mysql.svg',
  fileType: false,
  defaultPort: 3306,
  databasePh: 'cd.dbPhOptional',
  urlTemplate: 'jdbc:mysql://{host}:{port}/{database}?useUnicode=true&characterEncoding=UTF-8&useSSL=false&serverTimezone=Asia/Shanghai&allowPublicKeyRetrieval=true&connectTimeout=10000&socketTimeout=0&rewriteBatchedStatements=true&useCursorFetch=true&zeroDateTimeBehavior=CONVERT_TO_NULL&allowMultiQueries=true',
  options: mysqlFamilyOptions(),
  // 表单里已删掉的选项：老连接可能还存着（当年默认就写进了 params），
  // 回显时丢掉，免得它们在「附加参数」里冒充用户配置。值已写进上面的模板。
  dropParams: ['useSSL', 'characterEncoding', 'serverTimezone', 'allowPublicKeyRetrieval',
    'rewriteBatchedStatements', 'zeroDateTimeBehavior', 'allowMultiQueries'],
  createDatabaseSql(name, f = {}) {
    const n = String(name ?? '').trim().replace(/`/g, '``')
    return `CREATE DATABASE \`${n}\` CHARACTER SET ${f?.charset || 'utf8mb4'} COLLATE ${f?.collation || 'utf8mb4_general_ci'}`
  },
  showDbDdlSql(db) {
    return `SHOW CREATE DATABASE \`${String(db ?? '').replace(/`/g, '``')}\``
  }
}
