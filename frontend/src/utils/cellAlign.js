/**
 * 单元格对齐：**按 Excel 的标准**，由列的数据类型（表头）决定。
 *
 *   - 数字（整数 / 小数 / 金额 / 百分比 / 位）→ 右对齐
 *   - 日期、时间（含 timestamp / datetime / interval）→ 右对齐（Excel 对日期同样右对齐）
 *   - 布尔（true / false）→ 居中
 *   - 文本、JSON、二进制等 → 左对齐（返回空串，走默认样式）
 *
 * 两条约定：
 *   1. **只认列类型，不按单元格的值猜**：以前 `VARCHAR` 列里存成 `8888` 这种"看着像数字"的值
 *      会单独靠右，同一列里 `user1` 靠左、`8888` 靠右，很乱；现在文本列一律靠左。
 *   2. **同一列所有单元格（含 NULL）必须对齐一致**：所以网格里统一用 [`columnAlignClasses`]
 *      按列算一次、缓存住，不要逐格调用 `cellAlignClass`。
 *
 * 列类型拿不到时（SQL 结果集的查询端点经常不带 `columnTypes`），退回"按这一列值的多数派"
 * 定调 —— 仍然是**整列**一个结论，不会一半左一半右。
 */
import { numericTypeHint } from './dbType'

const NUM_RE = /^[+-]?(\d+(\.\d+)?|\.\d+)([eE][+-]?\d+)?$/
const DATE_RE = /^\d{4}[-/.]\d{1,2}([-/.]\d{1,2})?([ T]\d{1,2}:\d{2}(:\d{2}(\.\d+)?)?)?$/
const TIME_RE = /^\d{1,2}:\d{2}(:\d{2}(\.\d+)?)?$/
const BOOL_TYPE_RE = /(^|[^a-z])bool(ean)?([^a-z]|$)/

/**
 * 字段类型 → 对齐类；**认不出类型时返回 `null`**（与"认出来是文本 → `''`"区分开）。
 *
 * 传入各库的原始类型串（`INT` / `VARCHAR(50)` / `Nullable(Int32)` / `LowCardinality(String)`…）。
 * 数字 / 文本的家族判定复用 `utils/dbType.js` 的 `numericTypeHint`
 * （`bit` 归数字、`string` / `char` / `json` 归文本），日期时间与布尔在这里单独先判。
 *
 * @param {string} colType 原始类型串（可为空）
 * @returns {''|'al-r'|'al-c'|null} `null` = 类型认不出来，交给值判断
 */
const typeAlign = (colType) => {
  const s = String(colType == null ? '' : colType).trim().toLowerCase()
  if (!s) return null
  if (BOOL_TYPE_RE.test(s)) return 'al-c'
  const hint = numericTypeHint(s)
  if (hint === true) return 'al-r'
  // 日期时间在 Excel 里跟数字一样靠右（timestamp / datetime / time / interval 都命中）
  if (/date|time|interval/.test(s)) return 'al-r'
  if (hint === false) return ''
  return null
}

/**
 * 字段类型 → 对齐类（认不出来时按左对齐处理，与"类型说不清就当文本"的直觉一致）。
 *
 * @param {string} colType 原始类型串（可为空）
 * @returns {''|'al-r'|'al-c'} `al-r` 右对齐、`al-c` 居中、`''` 左对齐（默认样式）
 */
export const alignByType = (colType) => {
  const byType = typeAlign(colType)
  return byType === null ? '' : byType
}

/**
 * 单个值的对齐（**优先看列类型**，只有类型认不出来时才按值兜底）。
 *
 * 网格里一般都改用 `columnAlignClasses` 的列级结论；这个函数留给"手里只有值"的场景
 * （以及 `columnAlignClasses` 内部判定一列值的多数派）。
 *
 * @param {*} v 单元格原始值
 * @param {string} [colType] 字段类型（原始类型串）
 * @returns {''|'al-r'|'al-c'}
 */
export const cellAlignClass = (v, colType) => {
  const byType = typeAlign(colType)
  // 类型说了算：文本列里的 `8888` 也必须靠左，数字列里的字符串 `'12'` 也必须靠右
  if (byType !== null) return byType
  // 类型拿不到 / 认不出来：只能看值本身
  if (v == null) return ''
  if (typeof v === 'number' || typeof v === 'bigint') return 'al-r'
  if (typeof v === 'boolean') return 'al-c'
  const s = String(v).trim()
  if (!s) return ''
  if (/^(true|false)$/i.test(s)) return 'al-c'
  // 数字 / 日期 / 时间一定以数字（或正负号、小数点）开头 ——
  // 先用首字符挡掉绝大多数文本，省掉每格一次正则（大表每次重绘上千个单元格）
  const c0 = s.charCodeAt(0)
  const numericHead = (c0 >= 48 && c0 <= 57) || c0 === 43 || c0 === 45 || c0 === 46
  if (!numericHead) return ''
  if (NUM_RE.test(s)) return 'al-r'
  if (DATE_RE.test(s) || TIME_RE.test(s)) return 'al-r'
  return ''
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
 *   1. 类型明确是数字 / 日期 → `al-r`；布尔 → `al-c`；其它（文本 / 二进制…）→ `''` 左对齐；
 *   2. 类型认不出来 → 数一下这一列非空值的多数派（数字/日期 → 右，布尔 → 居中，其余 → 左）。
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
    const byType = typeAlign(c.type)
    if (byType !== null) { map[name] = byType; continue }
    let seen = 0
    let right = 0
    let center = 0
    for (const r of rows || []) {
      const v = r ? r[name] : null
      if (v == null) continue
      seen++
      const cls = cellAlignClass(v, '')
      if (cls === 'al-r') right++
      else if (cls === 'al-c') center++
      if (seen >= SAMPLE_LIMIT) break
    }
    if (seen > 0 && right * 2 > seen) map[name] = 'al-r'
    else if (seen > 0 && center * 2 > seen) map[name] = 'al-c'
    else map[name] = ''
  }
  return map
}
