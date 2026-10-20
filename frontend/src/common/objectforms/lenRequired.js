/* 「长度 / 精度是否必填」的**唯一规则表** —— 「新建表」与「编辑表结构」共用这一份，
 * 免得两边各写一套、判断结果不一致（用户要求：所有数据库、两个页面都一样）。
 *
 * 判据是**方言语法本身**：
 *   · 必填 = 不带参数会直接报语法错，或默认值会悄悄改掉精度
 *     （MySQL 的 VARCHAR 不带长度是错；DECIMAL 不带则是 (10,0)，精度被改掉）；
 *   · 选填 = 省略参数是合法且有明确语义的
 *     （PG 的 varchar/numeric、H2/SQLite 的 varchar、CH 的 String、Doris 的 STRING）。
 * 用途：**只做保存前校验**。长度/精度两格的输入框不写任何占位提示 ——
 * 所有数据库、新建表与编辑表结构统一为"不显示"（用户要求：别这个库有提示、另一个库没有）。
 */
export const LEN_REQUIRED = {
  // MySQL / MariaDB（MariaDB 的 ddlStyle 也是 'mysql'）
  mysql: ['VARCHAR', 'CHAR', 'VARBINARY', 'BINARY', 'DECIMAL', 'NUMERIC'],
  // SQL Server：`nvarchar` 不带长度会退化成 1 字符宽，必须挡
  mssql: ['VARCHAR', 'NVARCHAR', 'CHAR', 'NCHAR', 'VARBINARY', 'BINARY', 'DECIMAL', 'NUMERIC'],
  // Oracle：VARCHAR2 必须带长度；NUMBER 可不带（默认任意精度）故不算
  oracle: ['VARCHAR2', 'NVARCHAR2', 'CHAR', 'NCHAR', 'DECIMAL', 'NUMERIC', 'RAW'],
  // Derby：VARCHAR / CHAR / DECIMAL 不带长度是语法错
  derby: ['VARCHAR', 'CHAR', 'DECIMAL', 'NUMERIC'],
  // DB2：`VARCHAR` 不带长度是语法错，DECIMAL 缺省 (5,0) 会把精度悄悄改掉
  db2: ['VARCHAR', 'CHAR', 'GRAPHIC', 'VARGRAPHIC', 'DECIMAL', 'NUMERIC'],
  // ClickHouse：Decimal 必须给 (精度, 标度)，FixedString 必须给定长；
  // String / DateTime64（缺省 3）等不带参数合法，算选填
  clickhouse: ['DECIMAL', 'FIXEDSTRING'],
  // Doris：VARCHAR / CHAR 必须带长度（`VARCHAR` 单写是语法错），Decimal 不给会吃默认精度
  doris: ['VARCHAR', 'CHAR', 'DECIMAL', 'DECIMALV3', 'NUMERIC'],
  // PG / H2 / SQLite 不在此表：varchar / numeric 不带参数分别是"不限长 / 任意精度"，
  // SQLite 的类型更是纯声明 —— 都属合法写法，一律按选填处理。
}

/** 该方言下这个基础类型是否**必须**给长度/精度（`VARCHAR(64)` 里的 64） */
export const needsLength = (ddlStyle, typeBase) => {
  const list = LEN_REQUIRED[ddlStyle]
  if (!list) return false
  const base = String(typeBase || '').trim().toUpperCase().replace(/\(.*/, '').trim()
  return list.includes(base)
}
