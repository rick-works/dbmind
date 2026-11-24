/**
 * 变更记录的「定位路径」：连接.库.模式.表。
 *
 * 记录页原来只写连接名（`xingwei_kechuan`）—— 一条 ALTER 到底改的是哪张表看不出来。
 * 这里从 SQL 里认目标表，按「能到表就到表、认不出就退到上一级」拼出路径：
 *
 *   alter table aibee_data_hour add primary key (...)   + database=demo
 *     → 实验室-SQLServer.demo.aibee_data_hour
 *   CREATE TABLE dbo.dbmind_probe_ai (...)              + database=demo
 *     → 实验室-SQLServer.demo.dbo.dbmind_probe_ai
 *   create database foo                                 （没有表）
 *     → 实验室-SQLServer.demo        （再没有库就只剩连接名）
 *
 * 只是渲染用的展示，不参与执行；认错表名也只影响这一行字，不影响撤销/执行。
 */

/** 各语句「表名」出现在哪里（顺序有意义：更具体的放前面） */
const TABLE_PATTERNS = [
  // create [unique] index 名 on 表
  /^\s*create\s+(?:unique\s+)?index\s+.+?\s+on\s+([^\s(]+)/i,
  // alter table 表 ...
  /^\s*alter\s+table\s+(?:if\s+exists\s+)?(?:only\s+)?([^\s(]+)/i,
  // create [or replace] table [if not exists] 表 ( ... )
  /^\s*create\s+(?:or\s+replace\s+)?table\s+(?:if\s+not\s+exists\s+)?([^\s(]+)/i,
  /^\s*drop\s+table\s+(?:if\s+exists\s+)?([^\s(;]+)/i,
  /^\s*truncate\s+(?:table\s+)?([^\s(;]+)/i,
  /^\s*delete\s+from\s+([^\s(;]+)/i,
  /^\s*update\s+([^\s(;]+?)\s+set\b/i,
  /^\s*insert\s+(?:into\s+)?([^\s(;]+)/i,
  // 常见的「改结构」：rename / comment on table / create view / 触发器
  /^\s*rename\s+table\s+([^\s]+)\s+to\b/i,
  /^\s*comment\s+on\s+table\s+([^\s]+)/i,
  /^\s*create\s+(?:or\s+replace\s+)?(?:materialized\s+)?view\s+([^\s(]+)/i
]

/** 把 `[dbo].[D_DATE]` / `` `db`.`t` `` / `"s"."t"` 拆成 ['dbo','D_DATE'] */
const splitIdent = (raw) => {
  const text = String(raw || '')
    .replace(/\[([^\]]*)\]/g, '$1')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/"([^"]*)"/g, '$1')
    .trim()
  if (!text) return []
  return text.split('.').map((part) => part.trim()).filter(Boolean)
}

/**
 * 从 SQL 里取目标表的分段（可能 1~3 段：表 / 模式.表 / 库.模式.表）。
 * 认不出返回 []。
 */
export const tableSegsOf = (sql) => {
  const text = String(sql || '')
  if (!text.trim()) return []
  for (const re of TABLE_PATTERNS) {
    const hit = text.match(re)
    if (hit && hit[1]) {
      const segs = splitIdent(hit[1])
      if (segs.length) return segs
    }
  }
  return []
}

/**
 * 保守地取「简单单表 SELECT」的表名（给结果表格的字段脱敏用来命中钉表规则）。
 *
 * 只在**确定**只有一张表时返回名字，拿不准一律返回 ''：
 *   - `select * from t` / `select a,b from "db"."t" where …` / `from t;` / `from t -- 注释`  ✔
 *   - JOIN / 逗号多表 / UNION / 子查询 / `from (select …)` / 表函数  → ''
 * 理由与后端 `mask.rs` 一致：宁可不打码，也不能把 A 表的脱敏规则贴到 B 表的列上。
 * 返回的是**表段**（最后一段），库/模式由调用方的 database 负责。
 */
export const singleTableOf = (sql) => {
  let text = String(sql || '').trim()
  if (!text) return ''
  // 多语句：只有一条才算（sqlSplit 之外的简单判定：两个分号中间还有内容）
  if ((text.match(/;/g) || []).length > 1) return ''
  text = text.replace(/;+\s*$/, '').trim()
  // 只认 select / with 开头的只读查询；with（CTE）里可能藏多张表，直接放弃
  if (!/^select\b/i.test(text)) return ''
  // 顶层危险结构：JOIN / UNION / 子查询 / 逗号多表。字符串常量里出现这些词会误伤，
  // 但那只会导致"少打码"（返回 ''），不会错打，符合保守原则。
  if (/\b(join|union|intersect|except)\b/i.test(text)) return ''
  if (/\(\s*select\b/i.test(text)) return ''
  const from = /\bfrom\s+([`"[\]\w.$]+)([\s\S]*)$/i.exec(text)
  if (!from || !from[1]) return ''
  const rest = from[2] || ''
  // `from f_split(…)` 是表值函数，不是表；补错前缀会把脱敏贴到不存在的表上
  if (/^\s*\(/.test(rest)) return ''
  // FROM 之后若还有顶层逗号（另一张表）—— 简单判定：逗号后面跟着标识符且不在括号里
  if (/^\s*,|[\s)],\s*[`"[\]\w]/i.test(rest)) return ''
  const segs = splitIdent(from[1])
  if (segs.length === 0) return ''
  return segs[segs.length - 1]
}

/**
 * 拼「连接.库.模式.表」这类路径。
 *
 * @param {{sql?:string, database?:string, connectionId?:string}} change 变更单/记录行
 * @param {string} connName 连接名（查不到就传空）
 */
export const changePathOf = (change, connName) => {
  const dbParts = splitIdent(change?.database)
  const segs = tableSegsOf(change?.sql)
  const parts = []
  if (connName) parts.push(String(connName))
  // SQL 自带三段以上（库.模式.表）就不再用记录里的库名，避免拼出两个库
  if (segs.length >= 3) {
    parts.push(...segs)
  } else {
    const merged = [...dbParts, ...segs].filter(
      (part, i, arr) => i === 0 || part.toLowerCase() !== arr[i - 1].toLowerCase()
    )
    parts.push(...merged)
  }
  return parts.join('.')
}

export default changePathOf
