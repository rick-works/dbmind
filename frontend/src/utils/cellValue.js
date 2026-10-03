// 数据格值的**展示**格式化：编辑与复制仍用原始值，只有网格渲染走这里。
import { querySettingsLive } from './settings'
//
// 驱动原样返回 ISO 时间戳 `2026-09-30T16:10:24`，中间的 T 是 ISO 8601 的分隔符，
// 对人眼是噪音 —— 展示时换成空格（`2026-09-30 16:10:24`）。
// 只动长得确定是「日期 T 时间」的字符串，其它值原样返回。
const ISO_DATE_TIME = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/

/** NULL 单元格的显示文本（设置页「查询 → NULL 显示样式」：NULL / (NULL) / 留空）。
 *  网格、行详情、提示气泡统一走这里 —— 以前 'NULL' 在各视图里写死。 */
export const nullDisplay = () => {
  switch (querySettingsLive.value.nullStyle) {
    case 'paren': return '(NULL)'
    case 'blank': return ''
    default: return 'NULL'
  }
}

export const formatDbValue = (v) => {
  if (typeof v !== 'string' || !ISO_DATE_TIME.test(v)) return v
  return v.replace('T', ' ')
}
