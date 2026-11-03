import { mssqlOptions } from './presets.js'

/**
 * SQL Server 类型定义（schema 层级，默认 dbo）
 *
 * 注意 URL 分隔符：SQL Server 用 `;key=value`，不是 `?a=b&c=d`
 * （propStyle: 'semicolon'，与 mssql-jdbc 的语法一致）。
 * 命名实例不再单列字段：一律用 host:port，命名实例填它实际监听的端口即可。
 */
export default {
  code: 'SQLSERVER',
  label: 'SQL Server',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'schema',
  defaultSchema: 'dbo',
  quoteStyle: 'BRACKET',
  logo: 'sqlserver.svg',
  fileType: false,
  defaultPort: 1433,
  // allowMultiQueries=true：内核执行脚本时一条请求里可能带多条语句（后端 render_jdbc_url
  // 也会给 sqlserver 补这一条，这里写进模板是为了让预览和真实连接一字不差）
  databasePh: 'cd.dbPhOptional',
  urlTemplate: 'jdbc:sqlserver://{host}:{port};databaseName={database};encrypt=false;trustServerCertificate=true;loginTimeout=10;allowMultiQueries=true',
  propStyle: 'semicolon',
  // 认证方式：账号密码 / Windows 集成身份验证（后端按运行平台选择原生 SSPI 或 JavaKerberos）
  authTypes: [
    { value: 'sqlserver', label: 'SQL Server', labelKey: 'tdef.authSqlServer' },
    { value: 'windows', label: 'Windows', labelKey: 'tdef.authWindows' }
  ],
  options: mssqlOptions(),
  // 表单里已删掉的选项：历史残留值在回显时丢掉。
  // 其中 encrypt/trustServerCertificate 本来就在上面的模板里，
  // 其余（Unicode 发送 / 结果缓冲 / 取数方式 / 多子网 / 程序名）都是驱动默认值。
  dropParams: ['applicationName', 'encrypt', 'trustServerCertificate', 'sendStringParametersAsUnicode',
    'responseBuffering', 'selectMethod', 'multiSubnetFailover'],
  /**
   * 建库参数齐全版。返回**语句数组**（NewDbDialog 按序执行）：
   * - COLLATE：不选 = 跟随服务器默认（大多数场景服务器默认就是对的）；
   * - 恢复模式：建库后 ALTER（CREATE 语句本身带不了这个子句）；
   * - 初始大小：CREATE 的文件子句要 FILENAME 绝对路径（猜路径必翻车），
   *   改用建库后 MODIFY FILE 按逻辑名调整 —— 默认逻辑名就是 [库名] 与 [库名_log]，
   *   日志按数据文件的 1/4 跟随。
   */
  createDatabaseSql(name, f = {}) {
    const n = String(name ?? '').trim().replace(/]/g, ']]')
    const out = [`CREATE DATABASE [${n}]${f?.collation ? ` COLLATE ${f.collation}` : ''}`]
    if (f?.recovery) out.push(`ALTER DATABASE [${n}] SET RECOVERY ${f.recovery}`)
    const size = Number(f?.initialSize)
    if (Number.isFinite(size) && size > 0) {
      out.push(`ALTER DATABASE [${n}] MODIFY FILE (NAME = [${n}], SIZE = ${Math.floor(size)} MB)`)
      // 新库的日志文件默认就是 8MB，MODIFY FILE 只能往大改（≤当前值报
      // "Specified size is less than or equal to current size"），小了就不发。
      const logMb = Math.floor(size / 4)
      if (logMb > 8) out.push(`ALTER DATABASE [${n}] MODIFY FILE (NAME = [${n}_log], SIZE = ${logMb} MB)`)
    }
    return out
  },
  showDbDdlSql(db) {
    // 完整建库语句：库名 + 排序规则 + 数据/日志文件（逻辑名、物理路径、初始大小、上限、增长）+ 恢复模式。
    // 文件行用 FOR XML PATH 聚合（兼容 2017 之前的版本，不用 STRING_AGG）；
    // size/growth 是 8KB 页数，/128 = MB；max_size=-1 表示无限。
    // 目录视图列（sys.databases/sys.master_files）是服务器排序规则，与当前库字面量拼接会报
    // "Cannot resolve collation conflict"（实测 2022，库上下文是 SQL_Latin1_General_CP1_CI_AS）——
    // 每个目录列统一 COLLATE DATABASE_DEFAULT 归到当前库排序规则。
    const q = String(db ?? '').replace(/'/g, "''")
    const fileRow = (type, sep, skip) => `STUFF((SELECT '${sep}' + CHAR(13) + CHAR(10) + '  (NAME = [' + f.name COLLATE DATABASE_DEFAULT + '], FILENAME = N''' + f.physical_name COLLATE DATABASE_DEFAULT + ''', SIZE = ' + CAST(f.size / 128 AS varchar(20)) + ' MB, MAXSIZE = ' + CASE WHEN f.max_size = -1 THEN 'UNLIMITED' ELSE CAST(f.max_size / 128 AS varchar(20)) + ' MB' END + ', FILEGROWTH = ' + CASE WHEN f.is_percent_growth = 1 THEN CAST(f.growth AS varchar(20)) + '%' ELSE CAST(f.growth / 128 AS varchar(20)) + ' MB' END + ')'` +
      ` FROM sys.master_files f WHERE f.database_id = d.database_id AND f.type_desc = '${type}' FOR XML PATH(''), TYPE).value('.', 'nvarchar(max)'), 1, ${skip}, '')`
    return `SELECT 'CREATE DATABASE [' + d.name COLLATE DATABASE_DEFAULT + '] COLLATE ' + d.collation_name COLLATE DATABASE_DEFAULT + CHAR(13) + CHAR(10) + 'ON PRIMARY' + ${fileRow('ROWS', '', 2)} + CHAR(13) + CHAR(10) + 'LOG ON' + ${fileRow('LOG', ',', 1)} + ';' + CHAR(13) + CHAR(10) + 'ALTER DATABASE [' + d.name COLLATE DATABASE_DEFAULT + '] SET RECOVERY ' + d.recovery_model_desc COLLATE DATABASE_DEFAULT + ';' AS ddl FROM sys.databases d WHERE d.name = N'${q}'`
  }
}
