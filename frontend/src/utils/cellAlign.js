/**
 * 单元格对齐：**由列的数据类型（表头）决定靠左还是靠右**。
 *
 *   - 数值类型（int / bigint / decimal / numeric / float / double / number / money / bit…）→ 右对齐
 *   - 其余类型（字符、文本、JSON、二进制、布尔、日期时间…）→ 左对齐（返回空串，走默认样式）
 *
 * 两条明确的产品约定：
 *   1. **不居中**：以前布尔值居中、`VARCHAR` 列里存成 `8888` 这种"看着像数字"的值也靠右，
 *      结果同一列里 `user1` 靠左、`8888` 靠右，很乱；现在只认列类型，文本列一律靠左。
 *   2. **同一列所有单元格（含 NULL）必须对齐一致**：所以网格里不要按单个单元格的值判，
 *      统一用 [`columnAlignClasses`] 按列算一次、缓存住。
 *
 * 列类型拿不到时（SQL 结果集的查询端点经常不带 `columnTypes`）才退回"按值兜底"：
 * 这一列的非空值多数像数字 → 右对齐，否则左对齐 —— 仍然是**整列**一个结论。
 */
import { numericTypeHint } from './dbType'

const NUM_RE = /^[+-]?(\d+(\.\d+)?|\.\d+)([eE][+-]?\d+)?$/

/**
 * 字段类型 → 对齐类。
 *
 * 传入各库的原始类型串（`INT` / `VARCHAR(50)` / `Nullable(Int32)` / `LowCardinality(String)`…），
 * 判定复用 `utils/dbType.js` 的 `numericTypeHint`（数值家族 → `true`，文本 / 时间 / 布尔 / 二进制 → `false`，
 * 认不出来 → `null`）。认不出来的按左对齐处理，与「类型说不清就当文本」的直觉一致。
 *
 * @param {string} colType 原始类型串（可为空）
 * @returns {''|'al-r'} `al-r` = 右对齐；`''` = 左对齐（默认样式）
 */
export const alignByType = (colType) => (numericTypeHint(colType) === true ? 'al-r' : '')

/**
 * 单个值的对齐（**优先看列类型**，只有类型认不出来时才按值兜底）。
 *
 * 网格里一般都改用 `columnAlignClasses` 的列级结论；这个函数留给"手里只有值"的场景
 * （以及 `columnAlignClasses` 内部判定一列值的多数派）。
 *
 * @param {*} v 单元格原始值
 * @param {string} [colType] 字段类型（原始类型串）
 * @returns {''|'al-r'}
 */
export const cellAlignClass = (v, colType) => {
  const hint = numericTypeHint(colType)
  // 类型说了算：文本列里的 `8888` 也必须靠左，数字列里的字符串 `'12'` 也必须靠右
  if (hint !== null) return hint ? 'al-r' : ''
  // 类型拿不到 / 认不出来：只能看值本身（布尔、空串、NULL 一律靠左）
  if (v == null || typeof v === 'boolean') return ''
  if (typeof v === 'number' || typeof v === 'bigint') return 'al-r'
  const s = String(v).trim()
  if (!s) return ''
  // 数字一定以数字 / 正负号 / 小数点开头 —— 先用首字符挡掉绝大多数文本，
  // 省掉每格一次正则（大表每次重绘上千个单元格）
  const c0 = s.charCodeAt(0)
  const numericHead = (c0 >= 48 && c0 <= 57) || c0 === 43 || c0 === 45 || c0 === 46
  if (!numericHead) return ''
  return NUM_RE.test(s) ? 'al-r' : ''
}

/** 一列最多看多少个非空值来定调：够判断"这列整体怎么排"就行，不必扫完整列 */
const SAMPLE_LIMIT = 200

/**
 * 按列算对齐类：**表头类型说了算**，类型拿不到时看这一列值的多数派。
 *
 * 为什么要按列算：
 *   · 同一列里不能出现"值靠右、NULL 靠左"（用户反馈"null 和数字对齐呀"）；
 *   · SQL 结果集经常没有 `columnTypes`，只能从值上推断，而且必须**整列**一个结论。
 *
 * 规则：
 *   1. 类型明确是数值 → `al-r`；类型明确不是数值（文本 / 日期 / 布尔 / 二进制…）→ `''` 左对齐；
 *   2. 类型认不出来 → 数一下这一列非空值里"像数字"的占比，过半（严格大于一半）→ `al-r`，否则左对齐。
 *
 * **按列算一次、由调用方缓存在 computed 里**：表里 NULL 可能有上千个，
 * 每个格子现扫一遍自己那一列是 O(行数²)。
 *
 * @param {Array<object>} rows 已加载的行（桌面端是当前页，最多几百行）
 * @param {Array<{name: string, type?: string}>} cols 列（名字 + 原始类型串）
 * @returns {Object} 列名 → 对齐类（`''` 表示左对齐，走默认样式）
 */
export const columnAlignClasses = (rows, cols) => {
  const map = Object.create(null)
  for (const c of cols || []) {
    const name = c && c.name
    if (!name) continue
    const hint = numericTypeHint(c.type)
    if (hint !== null) { map[name] = hint ? 'al-r' : ''; continue }
    let seen = 0
    let numeric = 0
    for (const r of rows || []) {
      const v = r ? r[name] : null
      if (v == null) continue
      seen++
      if (cellAlignClass(v, '') === 'al-r') numeric++
      if (seen >= SAMPLE_LIMIT) break
    }
    map[name] = seen > 0 && numeric * 2 > seen ? 'al-r' : ''
  }
  return map
}
