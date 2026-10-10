/**
 * 虚拟滚动「可视行窗口」计算 —— 结果表格（SqlQueryView）、表数据（TableDataView）、
 * NoSQL 文档（NoSqlDataView）三处共用，避免三份实现各自漂移。
 *
 * ## 为什么必须**同步**算，而不能用 requestAnimationFrame 节流
 * 老实现是「scroll 事件 → rAF 回调里算窗口 → 写响应式」。rAF 回调要等到**下一帧**才执行，
 * 它引发的 DOM 更新自然更晚 —— 也就是说浏览器已经按新的 scrollTop 把**旧内容**滚过去并画出来了，
 * 下一帧才补上该出现的行。慢速滚动看不出来，但下面几种"跳着滚"会把可视区整段甩到已渲染窗口之外：
 *   - 拖滚动条拇指、点滚动条空白处（一帧跳几百上千像素）
 *   - PageDown / PageUp（200 条一页时一跳就是十几二十行）
 *   - 触控板/滚轮的连续快速翻页
 * 结果就是**一条空白**（浅色主题下就是"白屏闪现"），看着卡、不跟手。
 *
 * 现在改成在 scroll 处理函数里同步算：Vue 的更新调度走微任务，会在同一帧绘制**之前**刷完，
 * DOM 先补好再画，白条不再出现。（窗口没变化时写 ref 是空操作 —— Vue 的 ref 会先比较值，
 * 相同就不触发更新，所以慢速滚动几乎零开销。）
 *
 * 缓冲行数是**跟着这一跳的距离走**的：跳得越远，移动方向前方多渲染几行，进一步留出余量。
 */

/** 基础缓冲行数（可视区上下各多渲染几行） */
export const ROW_WINDOW_BUFFER = 12

/**
 * 跳跃时额外多渲染的行数上限。
 * 拖滚动条一帧可以跳几千像素，不封顶就会一次渲染上千行 —— 那比白条更糟（直接卡住）。
 */
const MAX_EXTRA_BUFFER = 120

/**
 * 算出这一帧该渲染哪一段行。
 *
 * @param {object} o
 * @param {number} o.scrollTop     当前滚动位置（scrollTop）
 * @param {number} o.clientHeight  可视区高度
 * @param {number} o.rowH          行高（**必须**与 CSS 里的行高同源，否则定位会漂）
 * @param {number} o.total         总行数
 * @param {number} [o.prevScrollTop] 上一次算窗口时的滚动位置；用于按跳跃距离放宽缓冲
 * @param {number} [o.buffer]      基础缓冲行数
 * @returns {{start:number,end:number}} 左闭右开区间 [start, end)
 */
export function rowWindow({
  scrollTop,
  clientHeight,
  rowH,
  total,
  prevScrollTop = scrollTop,
  buffer = ROW_WINDOW_BUFFER
}) {
  if (!total || total <= 0) return { start: 0, end: 0 }
  const h = Math.max(1, rowH)
  const top = Math.max(0, scrollTop)
  const firstVisible = Math.floor(top / h)
  // 这一跳跨过了多少行 → 额外多渲染同样多行（快速滚动时前方也是实心的）
  const jumped = Math.ceil(Math.abs(top - prevScrollTop) / h)
  const buf = buffer + Math.min(jumped, MAX_EXTRA_BUFFER)
  const visible = Math.ceil((clientHeight || 1) / h)
  const start = Math.max(0, Math.min(firstVisible - buf, Math.max(0, total - 1)))
  // +1：可视区下缘那一行常常只露出一部分，多带一行免得边缘缺一条
  const end = Math.min(total, firstVisible + visible + buf + 1)
  return { start, end: Math.max(end, Math.min(total, start + 1)) }
}
