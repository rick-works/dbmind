/**
 * 让 el-select 的宽度跟着「当前选中值」走（连接名 / 库名很长时不再被截成 "qj_x_dmp_yingx…"）。
 *
 * 为什么不用 CSS 的 `width: max-content`：Element 把选中项放在 `.el-select__selection`
 * （flex: 1，flex-basis: 0）里，max-content 算下来只有 suffix 图标那么宽，最后被 min-width
 * 兜住 —— 表现就是"写了自适应，宽度还是没变、名字照样被截断"（已实测）。
 *
 * 为什么不「先把宽度复位成 auto 再读 scrollWidth」（上一版的写法）：下拉在 `display:flex`
 * 的行里是 flex 子项，宽度一交给 auto 就被拉满整行剩余空间，选中项（flex:1）跟着变宽 ⇒
 * scrollWidth 恒等于"当前可用宽"，量出来只会越量越宽、永不回缩（实测 130px 复位后变 203px）。
 *
 * 为什么不用 Range 量文本：Range 的矩形会被 `overflow: hidden` 的截断框裁掉，名字被截断时
 * 量到的是"看得见的那截"，越量越窄（实测 information_schema 只量到 125px，框给 167px 仍然截断）。
 *
 * 所以这里用一个离屏 span（复制选中项的字体度量）量**完整文字**宽度：跟盒子被撑多宽、是否
 * 被裁都无关。再加上 wrapper 内边距、前后缀（库图标、箭头）与 flex gap，clamp 到 [min, max]
 * 后写回 style.width。
 *
 * **优先按「值」量**：调用方把要显示的文字（连接名 / 库名）传进来（`text`），比读 DOM 可靠 ——
 * 组件挂载那一帧 Element 还没把选中项渲染出来，此时读 DOM 读到空串只能兜到 min，
 * 之后又没有事件再触发重量，于是首帧就固定成"截断宽度"（设置页 默认连接 实测 140px 永远不够）。
 *
 * 调用时机：值变更 / 选项变更 / 字号变更之后，在 nextTick 里调。
 */

/** 离屏测量节点：整个页面复用一个，避免每次量都往 body 里塞元素 */
let probeEl = null
function measureText(text, cs) {
  if (!text) return 0
  if (!probeEl) {
    probeEl = document.createElement('span')
    probeEl.setAttribute('aria-hidden', 'true')
    probeEl.style.cssText = 'position:absolute;left:-9999px;top:0;visibility:hidden;white-space:pre;pointer-events:none;'
    document.body.appendChild(probeEl)
  }
  const s = probeEl.style
  // 只复制影响字宽的属性（字体简写在各浏览器上不可靠，逐项给）
  s.fontFamily = cs.fontFamily
  s.fontSize = cs.fontSize
  s.fontWeight = cs.fontWeight
  s.fontStyle = cs.fontStyle
  s.fontVariant = cs.fontVariant
  s.fontStretch = cs.fontStretch
  s.letterSpacing = cs.letterSpacing
  s.wordSpacing = cs.wordSpacing
  s.textTransform = cs.textTransform
  probeEl.textContent = text
  return probeEl.getBoundingClientRect().width
}

/** 显示选中值/占位符的那个节点（Element 2.x 两种结构都认） */
function labelOf(el) {
  return el.querySelector('.el-select__placeholder')
    || el.querySelector('.el-select__selected-item:not(.el-select__input-wrapper)')
}

/**
 * 还没显示出来时（弹窗淡入 / 页签未激活 / 面板 display:none）先别写宽度：这时前后缀图标
 * 量出来是 0，写进去的会比需要的窄（实测「默认连接」在对话框动画期间被写成 140px，名字永远截断）。
 * 用 ResizeObserver 等它真的有了尺寸再量一次 —— 比"等 N 帧"可靠：页签什么时候被点开不可预知。
 * 量完立刻断开，避免"改宽度 → 触发 resize → 又量"的自激循环。
 */
function fitWhenSized(el, opts) {
  if (typeof ResizeObserver === 'undefined') return
  const ro = new ResizeObserver(() => {
    if (!el.isConnected) { ro.disconnect(); return }
    if (el.getBoundingClientRect().width === 0) return
    ro.disconnect()
    fitSelectWidth(el, { ...opts, _wait: true })
  })
  // 不用超时收摊：页签可能过很久才被点开（挂载时它一直是 display:none）。
  // 元素被移除时 RO 会再回调一次（尺寸归零），那里断开即可，不会泄漏。
  ro.observe(el)
}

/**
 * 单个下拉：`el` 传 Element 组件实例的 $el（也就是 .el-select 根节点）。
 * `text` 可选：调用方已知要显示的字符串时传进来，比读 DOM 可靠（挂载首帧 DOM 还是空的）。
 */
export function fitSelectWidth(el, { min = 120, max = 280, text, _wait } = {}) {
  if (!el || !el.isConnected) return
  const wrapper = el.querySelector('.el-select__wrapper')
  if (!wrapper) return
  if (el.getBoundingClientRect().width === 0) {
    if (!_wait) fitWhenSized(el, { min, max, text })
    return
  }

  const label = labelOf(el)
  const shown = typeof text === 'string' ? text : (label ? (label.textContent || '').trim() : '')
  // 字体度量拿不到 label（还没渲染）时退回 wrapper 的（同一个字号族）
  const textW = measureText(shown, getComputedStyle(label || wrapper))

  // 内边距 + 前后缀图标（Element 各版本的类名不同，两种都认）
  const cs = getComputedStyle(wrapper)
  const pad = (parseFloat(cs.paddingLeft) || 0) + (parseFloat(cs.paddingRight) || 0)
  const prefix = el.querySelector('.el-select__prefix, .el-input__prefix')
  const suffix = el.querySelector('.el-select__suffix, .el-input__suffix')
  const prefixW = prefix ? prefix.getBoundingClientRect().width : 0
  const suffixW = suffix ? suffix.getBoundingClientRect().width : 0
  // wrapper 是 flex + gap:6px（selection / suffix 之间的间距），漏掉这 6px 就会差一点、
  // 长名字照样显示省略号（实测 information_schema 172 需要 169，按 167 给就截断）
  const gap = parseFloat(cs.columnGap) || parseFloat(cs.gap) || 0
  const gaps = (prefix ? gap : 0) + (suffix ? gap : 0)

  // 收口：太短的名字不至于缩成一根线，太长的名字也不至于把整条工具条挤爆
  let w = Math.min(max, Math.max(min, Math.ceil(textW + pad + prefixW + suffixW + gaps) + 4))
  el.style.width = w + 'px'

  // 自校正：按「值」量出来的可能与实际渲染差几像素（图标宽度、字体回退），
  // 真出现省略号就把差额补上（只补一次，不会来回抖）
  if (label && label.scrollWidth > label.clientWidth + 1) {
    w = Math.min(max, w + (label.scrollWidth - label.clientWidth) + 2)
    el.style.width = w + 'px'
  }
}

/** 批量版本：`els` 里可以有 null（v-if 没渲染的下拉） */
export function fitSelectWidths(pairs) {
  for (const [el, opts] of pairs) fitSelectWidth(el, opts)
}
