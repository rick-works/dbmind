/**
 * 虚拟滚动「可视行窗口」计算 —— 结果表格（SqlQueryView）、表数据（TableDataView）、
 * NoSQL 文档（NoSqlDataView）三处共用，避免三份实现各自漂移。
 *
 * ## 为什么必须**同步**算，而不能用 requestAnimationFrame 节流
 * 老实现是「scroll 事件 → rAF 回调里算窗口 → 写响应式」。rAF 回调要等到**下一帧**才执行，
 * 它引发的 DOM 更新自然更晚 —— 也就是说浏览器已经按新的 scrollTop 把**旧内容**滚过去并画出来了，
 * 下一帧才补上该出现的行。慢速滚动看不出来，但下面几种"跳着滚"会把可视区整段甩到已渲染窗口之外：
 *   - 拖滚动条拇指、点滚动条空白处（一帧跳几百上千像素）
 *   - PageDown / PageUp（200 条一页时一跳就是十几行）
 *   - 触控板/滚轮的连续快速翻页
 * 结果就是**一条空白**（浅色主题下就是"白屏闪现"）。
 *
 * 现在改成在 scroll 处理函数里同步算：Vue 的更新调度走微任务，会在同一帧绘制**之前**刷完，
 * DOM 先补好再画。**正因为是同步的**，这里不再需要"按跳跃距离多渲染几行"那层补偿 ——
 * 无论跳多远，窗口都是按**新位置**算出来的，可视区必然被覆盖。
 *
 * ## 为什么要按「块」换窗口（丝滑的关键）
 * 每换一次窗口，Vue 就要重建/比对一整屏的行 ×列 vnode：30 列 × 五十来行 = 一千五百多个格子。
 * 若"每滚过一行就换一次"，行高 26px 时每滚 26px 就重渲染一遍 —— 快速滚动时每秒上百次，
 * 帧预算根本不够，手感的反馈就是"发涩、跟不上"。所以窗口按 `ROW_WINDOW_CHUNK_PX` 像素一块：
 * 同一个块内滚动，窗口**完全不变**（一个 vnode 都不用重建），只做合成器滚动。
 * 块边界留出足够的上下缓冲（`buffer`），所以块内滚到底也不会露白。
 */

/** 基础缓冲行数（可视区上下各多渲染几行） */
export const ROW_WINDOW_BUFFER = 12

/**
 * 换窗口的粒度（像素）：累计滚动不足这么多时，窗口保持不变。
 * 120px ≈ 26px 行高下的 5 行 —— 重渲染次数降到 1/5，而上下各 12 行缓冲远够盖住这 120px。
 */
export const ROW_WINDOW_CHUNK_PX = 120

/**
 * 算出这一帧该渲染哪一段行。
 *
 * @param {object} o
 * @param {number} o.scrollTop     当前滚动位置（scrollTop）
 * @param {number} o.clientHeight  可视区高度
 * @param {number} o.rowH          行高（**必须**与 CSS 里的行高同源，否则定位会漂）
 * @param {number} o.total         总行数
 * @param {number} [o.buffer]      基础缓冲行数
 * @returns {{start:number,end:number}} 左闭右开区间 [start, end)
 */
export function rowWindow({ scrollTop, clientHeight, rowH, total, buffer = ROW_WINDOW_BUFFER }) {
  if (!total || total <= 0) return { start: 0, end: 0 }
  const h = Math.max(1, rowH)
  const top = Math.max(0, scrollTop)
  const visible = Math.ceil((clientHeight || 1) / h)
  const firstVisible = Math.floor(top / h)
  const chunk = Math.max(1, Math.ceil(ROW_WINDOW_CHUNK_PX / h))
  // 块号只由「想覆盖的最上面一行」决定 → 同一块内 start/end 都不变（窗口稳定，不重渲染）
  const band = Math.max(0, Math.floor((firstVisible - buffer) / chunk))
  const start = Math.max(0, Math.min(band * chunk, Math.max(0, total - 1)))
  // 覆盖范围：本块 + 一整屏 + 上下缓冲（都从块首算起，所以块内恒定）
  const end = Math.min(total, start + chunk + visible + buffer * 2 + 1)
  return { start, end: Math.max(end, Math.min(total, start + 1)) }
}
