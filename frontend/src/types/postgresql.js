import { pgFamilyOptions } from './presets.js'

/** PostgreSQL 类型定义（schema 层级） */
export default {
  code: 'POSTGRESQL',
  label: 'PostgreSQL',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'schema',
  defaultSchema: 'public',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'PostgreSQL.svg',
  fileType: false,
  defaultPort: 5432,
  // 库名留空时按驱动默认连 postgres 库；这里由用户在表单里填（不再静默替换）
  databasePh: 'cd.dbPhOptional',
  // pgjdbc 的 URL 语法要求必须有路径段：库名空着也得保留 `...:5432/?x=y`，
  // 去掉斜杠会直接报 Unable to parse URL（实测）
  keepEmptyDbSlash: true,
  // 表单里不再出现的驱动参数，默认值固定在这条模板上：
  //   sslmode=prefer —— 能加密就加密，服务器不支持就退回明文（内网与云库都能连）
  //   reWriteBatchedInserts=true —— 批量插入合并成多值 INSERT
  //   stringtype=unspecified —— 遇到不认识的类型按字符串读，而不是抛「未知类型」
  //   tcpKeepAlive=true —— 长连接的 NAT 超时不会静默断链
  urlTemplate: 'jdbc:postgresql://{host}:{port}/{database}?sslmode=prefer&reWriteBatchedInserts=true&stringtype=unspecified&tcpKeepAlive=true',
  options: pgFamilyOptions(),
  // 表单里已删掉的选项（含更早删掉的「程序名」）：老连接里的残留值在回显时丢掉
  dropParams: ['ApplicationName', 'sslmode', 'currentSchema', 'reWriteBatchedInserts',
    'stringtype', 'tcpKeepAlive'],
  createDatabaseSql(name, f = {}) {
    const n = String(name ?? '').trim().replace(/"/g, '""')
    return `CREATE DATABASE "${n}" WITH ENCODING = '${f?.encoding || 'UTF8'}'`
  },
  showDbDdlSql(db) {
    return `SELECT 'CREATE DATABASE ' || quote_ident(datname) || ' WITH OWNER ' || quote_ident(pg_catalog.pg_get_userbyid(datdba)) || ' ENCODING ' || quote_literal(pg_encoding_to_char(encoding)) || ' LC_COLLATE ' || quote_literal(datcollate) || ' LC_CTYPE ' || quote_literal(datctype) || ';' AS ddl FROM pg_database WHERE datname = '${String(db ?? '')}'`
  }
}
