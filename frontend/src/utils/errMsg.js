// 后端错误消息提取：优先后端统一返回 {success:false,message} 的 message 字段，
// 其次兼容 {error}/{msg}，再退化为 axios Error.message / Error.toString() / 原始字符串 / 调用方 fallback。
// 替代散落在各组件里拼写的 `e?.response?.data?.message || e?.message || e` 链。
export const errMsg = (e, fallback = '') => {
  let msg = e?.response?.data?.message
  if (!msg && e?.response?.data && typeof e.response.data === 'object') {
    msg = e.response.data.error || e.response.data.msg ||
      (Object.keys(e.response.data).length ? JSON.stringify(e.response.data) : '')
  }
  if (!msg) msg = e?.message
  if (!msg) msg = typeof e === 'string' ? e : ''
  if (!msg && e && typeof e.toString === 'function') {
    const s = e.toString()
    msg = s && s !== '[object Object]' ? s : ''
  }
  return msg || fallback
}
