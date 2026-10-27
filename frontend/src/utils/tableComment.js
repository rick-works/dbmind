/**
 * 「修改表注释」的方言 SQL 生成器。
 *
 * 表清单页（ObjectListView）的注释列就地编辑用；口径与「编辑表结构」里
 * TableDetailView.optParts / mssqlCommentSql **保持一致**（那边是建表选项一起改，
 * 这里只改注释），两边各写一份迟早漂开，所以抽到这里。
 *
 * 各库写法（真机验过）：
 *   MySQL / MariaDB   ALTER TABLE `t` COMMENT='…'
 *   Doris             ALTER TABLE `t` MODIFY COMMENT '…'     （2.x 实测可用）
 *   ClickHouse        ALTER TABLE `t` MODIFY COMMENT '…'     （21.3+）
 *   SQL Server        sp_updateextendedproperty / sp_addextendedproperty（MS_Description）
 *   PG / Oracle / DB2 / H2 / Kingbase / DM   COMMENT ON TABLE "t" IS '…'
 */
const QUOTE_BY_TYPE = {
  MYSQL: 'BACKTICK',
  MARIADB: 'BACKTICK',
  DORIS: 'BACKTICK',
  CLICKHOUSE: 'BACKTICK',
  SQLSERVER: 'BRACKET'
}

const quoteIdent = (name, style) => {
  const s = String(name ?? '')
  if (style === 'BACKTICK') return '`' + s.replace(/`/g, '``') + '`'
  if (style === 'BRACKET') return '[' + s.replace(/\]/g, ']]') + ']'
  return '"' + s.replace(/"/g, '""') + '"'
}

/** 单引号字面量转义 */
const sq = (v) => String(v ?? '').replace(/'/g, "''")

/** `库.模式` → { db, schema }；没有点就当作只有库 */
export const splitDbSchema = (database) => {
  const s = String(database ?? '')
  const i = s.indexOf('.')
  if (i < 0) return { db: s, schema: '' }
  return { db: s.slice(0, i), schema: s.slice(i + 1) }
}

/**
 * 生成修改表注释的一条 SQL。
 * @param {object} o
 * @param {string} o.type      连接类型（MYSQL / POSTGRESQL / SQLSERVER …）
 * @param {string} o.database  当前库（可能是 `库.模式`）
 * @param {string} o.table     表名（可能带 `模式.` 前缀）
 * @param {string} o.comment   新注释（空串 = 清空）
 * @returns {string}
 */
export const buildTableCommentSql = ({ type, database = '', table = '', comment = '' }) => {
  const T = String(type || '').toUpperCase()
  const style = QUOTE_BY_TYPE[T] || 'DOUBLE'
  const raw = String(table || '')
  const dot = raw.indexOf('.')
  const nameHasSchema = dot > 0
  const bare = nameHasSchema ? raw.slice(dot + 1) : raw
  const val = sq(comment)

  if (T === 'SQLSERVER') {
    // 模式优先取表名里的前缀，其次取 `库.模式` 的后半段，最后落到 dbo
    const schema = nameHasSchema ? raw.slice(0, dot) : (splitDbSchema(database).schema || 'dbo')
    const tbl = sq(bare)
    const sc = sq(schema)
    const args = `'SCHEMA', N'${sc}', 'TABLE', N'${tbl}', NULL, NULL`
    return `IF EXISTS(SELECT 1 FROM ::fn_listextendedproperty('MS_Description', ${args})) ` +
      `EXEC sp_updateextendedproperty 'MS_Description', N'${val}', ${args} ` +
      `ELSE EXEC sp_addextendedproperty 'MS_Description', N'${val}', ${args}`
  }

  const t = quoteIdent(bare, style)

  if (T === 'DORIS' || T === 'CLICKHOUSE') {
    return `ALTER TABLE ${t} MODIFY COMMENT '${val}'`
  }
  if (T === 'MYSQL' || T === 'MARIADB') {
    return `ALTER TABLE ${t} COMMENT='${val}'`
  }
  // 其余关系型（PG / Oracle / DB2 / H2 / Kingbase / DM）
  return `COMMENT ON TABLE ${t} IS '${val}'`
}
