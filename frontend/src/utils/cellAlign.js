/**
 * 单元格对齐（按 Excel 规则）：
 *   - 数字（含「看起来是数字」的值）→ 右对齐
 *   - 日期 / 时间 → 右对齐（Excel 对日期同样右对齐）
 *   - 布尔（true/false）→ 居中
 *   - 文本、NULL、空值 → 左对齐（返回空串，走默认样式）
 *
 * 用法：`:class="[cellAlignClass(row[col]), { ...其它类 }]"`，配合
 *      `.data-table td.al-r { text-align: right }` / `.al-c { text-align: center }`。
 */

const NUM_RE = /^[+-]?(\d+(\.\d+)?|\.\d+)([eE][+-]?\d+)?$/
const DATE_RE = /^\d{4}[-/.]\d{1,2}([-/.]\d{1,2})?([ T]\d{1,2}:\d{2}(:\d{2}(\.\d+)?)?)?$/
const TIME_RE = /^\d{1,2}:\d{2}(:\d{2}(\.\d+)?)?$/

/**
 * 字段类型 → 对齐。值本身没有类型信息时用它（目前就是 NULL 单元格）：
 * 传入的是各库的原始类型串（INT / VARCHAR(50) / Nullable(Int32) / decimal(18,4)…），
 * 统一小写并剥掉括号后按关键字判断：
 *   - 布尔 → 居中；数字 / 日期时间 → 靠右；其余（字符、大对象、JSON…）→ 靠左（返回空串走默认值）
 * 边界判断用 (^|[^a-z]) 避免「point / varbit / char」里的 int、bit 被误命中。
 */
export const alignByType = (colType) => {
  const t = String(colType || '').toLowerCase().replace(/[()]/g, ' ')
  if (!t.trim()) return ''
  if (/bool|boolean/.test(t)) return 'al-c'
  if (/(^|[^a-z])(u?int\d*|tinyint|smallint|mediumint|bigint|integer|serial\d*|bigserial|decimal|numeric|number|float\d*|double|real|money|smallmoney|year|bit)([^a-z]|$)/.test(t)) return 'al-r'
  if (/date|time|interval/.test(t)) return 'al-r'
  return ''
}

/**
 * @param {*} v 单元格原始值
 * @param {string} [colType] 字段类型（原始类型串），值取不到时按它对齐
 * @returns {''|'al-r'|'al-c'} 对齐用的 class（'' = 左对齐，默认）
 */
export const cellAlignClass = (v, colType) => {
  // NULL 没有值可判断，就跟着字段类型走（数字 / 日期列的 NULL 同样靠右，与 Excel 一致），
  // 不再一律居中 —— 一列里右对齐的数字中间夹几个居中的 NULL，看着很乱
  if (v == null) return alignByType(colType)
  if (typeof v === 'number' || typeof v === 'bigint') {
    // NaN / Infinity 也归到数字一侧，与 Excel 一致
    return 'al-r'
  }
  if (typeof v === 'boolean') return 'al-c'
  const s = String(v).trim()
  if (!s) return ''
  // 数字 / 日期 / 时间一定以数字（或正负号、小数点）开头 ——
  // 先用首字符挡掉绝大多数文本，省掉每格 3 次正则匹配
  // （大表每次重绘上千个单元格，文本列占多数，这里是纯浪费）
  const c0 = s.charCodeAt(0)
  const numericHead = (c0 >= 48 && c0 <= 57) || c0 === 43 || c0 === 45 || c0 === 46
  if (!numericHead) return ''
  if (NUM_RE.test(s)) return 'al-r'
  if (DATE_RE.test(s) || TIME_RE.test(s)) return 'al-r'
  return ''
}
