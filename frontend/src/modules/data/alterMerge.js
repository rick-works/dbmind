// 表结构变更脚本的「合并」：把连续的 `ALTER TABLE 本表 …` 子句并成**一条** ALTER，其余语句保持独立。
//
// 为什么并：这些方言的 DDL 没有事务（MySQL / Oracle / Doris / ClickHouse 都会隐式提交），
// 逐条执行时中间那条失败，前面已经删掉的东西就再也回不来 —— 用户实测过两次：
// 「索引增加一个字段，因为先删除，执行报错后原来的索引也直接没了」。
// 而**一条 ALTER 里的多个子句是原子的**（真机验证：任一句非法则整条一句都不生效，
// 索引与列都保持原样）。并成一条就没有中间态。
//
// 为什么单独一个模块：这个函数一旦静默失效，界面就会退回"先删后建"，用户那边是**丢索引**这种
// 重后果；放在纯模块里才能直接跑断言（见仓库里那次 node 断言）。
//
// 为什么不能"有一段不能并就整批退回逐条"：Oracle/DM 的 `DROP INDEX` 天生不是 ALTER 形状，
// 整批退回等于让它们的列变更又变成非原子。这里只把**那一段**留在外面，其余照并。

/** 表名去引号：MySQL 反引号 / PG 双引号 / SQL Server 方括号 */
export const bareTable = (s) => String(s || '').replace(/[`"[\]]/g, '')

/* 识别 `ALTER TABLE <表> <子句…>`。表名各方言引号不同，所以**去引号后比对**，
   不做"字符串精确前缀"匹配 —— 早先那种写法只要表名写法与 qt() 输出有半点差别
   （引号、大小写、schema 前缀），就整段匹配不上、合并静默失效（用户实测到的第二次丢索引）。 */
const ALTER_TABLE_RE = /^ALTER\s+TABLE\s+([`"[\]\w.$]+)\s+([\s\S]+)$/i

/**
 * @param {string}   table  本表名（已带方言引号，例如 `` `test` ``）
 * @param {string}   kind   方言 key：mysql / mariadb / doris / clickhouse / oracle / dm / pg …
 * @param {string[]} parts  逐条语句（按执行顺序）
 * @returns {string[]}      合并后的语句列表（能并的并成一条，顺序不变）
 */
export const mergeAlterRuns = ({ table, kind, parts = [] }) => {
  const prefix = 'ALTER TABLE ' + table + ' '
  const want = bareTable(table)
  const out = []
  let run = []
  const flush = () => {
    if (run.length) out.push(prefix.trimEnd() + ' ' + run.join(', '))
    run = []
  }
  for (const stmt of parts) {
    // 一条语句里可能塞了两句（例如 MySQL 新增自增列时会带 `;\nALTER TABLE … AUTO_INCREMENT=n`）
    for (const piece of String(stmt).split(';\n')) {
      const one = piece.trim()
      if (!one) continue
      // 注释行（例如 Doris 那条"分桶列改不了"）：独立保留，执行端会忽略
      if (one.startsWith('--')) { flush(); out.push(one); continue }
      const m = ALTER_TABLE_RE.exec(one)
      if (!m || bareTable(m[1]) !== want) { flush(); out.push(one); continue }
      const clause = m[2]
      // Oracle/DM：RENAME COLUMN 是独立形式，不能与其它 alter_table_clause 并列
      if ((kind === 'oracle' || kind === 'dm') && /^RENAME\s/i.test(clause)) {
        flush(); out.push(one); continue
      }
      run.push(clause)
    }
  }
  flush()
  return out
}
