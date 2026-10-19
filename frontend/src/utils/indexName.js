/**
 * 自动生成的索引名：**统一** `<表名>_<字段名>_idx`，所有数据库一套。
 *
 * 为什么要有这个约定：以前各处各写一套 —— 结构编辑器的索引页签是 `<字段>_idx`、
 * 新建表对话框是 `idx_<字段>`、删主键时给自增列补的兜底索引是 `_上游_auto_<字段>`。
 * 用户在索引页签里看到的是 `_上游_auto_id` 这种既不像人名也不像机器名的东西，
 * 而同一张表在另一个对话框里又是 `idx_id`。统一成「表名 + 字段名 + _idx」后
 * 一眼能看出这个索引属于谁、按哪列建的。
 *
 * 处理细节：
 * · 去掉各方言的引号字符（` " [ ]）—— 名字里带引号会把 SQL 的引号弄坏；
 * · 空格统一成下划线：索引名在各库都不宜含空格，写进脚本也更容易读；
 * · 截到 60 个字符：各库上限不一（Oracle 30 字节 / PG 63 / MySQL 64 字符），
 *   过长的中文表名+字段名拼起来可能超限，宁可截断也不要让数据库报"名字太长"。
 */
const clean = (s) => String(s === undefined || s === null ? '' : s)
  .trim()
  .replace(/[`"[\]]/g, '')
  .replace(/\s+/g, '_')

/** 索引名统一 <= 60 字符（见上面注释） */
const MAX_LEN = 60

/**
 * @param {string} table   表名（不带引号）
 * @param {string} column  字段名（不带引号）；为空时拼成 `<表名>_col_idx`
 * @returns {string} 例如 users + id -> users_id_idx
 */
export const autoIndexName = (table, column) => {
  const name = [clean(table), clean(column) || 'col', 'idx'].filter(Boolean).join('_')
  return name.length > MAX_LEN ? name.slice(0, MAX_LEN) : name
}
