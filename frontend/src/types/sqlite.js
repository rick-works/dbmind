import { formParts } from './base'

/** SQLite 类型定义（本地文件型） */
export default {
  code: 'SQLITE',
  label: 'SQLite',
  category: 'RELATIONAL_FILE',
  noSql: false,
  schemaLevel: 'none',
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'sqlite.svg',
  fileType: true,
  defaultPort: 0,
  buildJdbcUrl(f) {
    const { file } = formParts(f)
    return `jdbc:sqlite:${file || ':memory:'}`
  },
  // SQLite 数据库即文件，新建即新建文件，不在 SQL 层用 CREATE DATABASE
  createDatabaseSql() {
    return ''
  },
  // SQLite 确实没有 `CREATE DATABASE`（一个文件就是一个库），
  // 但「这个库里有什么」的完整脚本恰恰是 sqlite_master 里现成的 —— 用 group_concat 拼成一份，
  // 比一句「无建库语句」有用得多（用户拿到的是一份可直接执行的库结构脚本）
  showDbDdlSql() {
    return "select '-- SQLite 一个文件就是一个库；以下是库内全部对象的 DDL' || char(10) || char(10) || " +
      "coalesce(group_concat(sql, char(10) || char(10)), '-- （库里还没有任何对象）') as ddl " +
      "from (select sql from sqlite_master where sql is not null order by (type = 'table') desc, name)"
  }
}
