/**
 * 让 el-select 的宽度跟着「当前选中值」走（连接名 / 库名很长时不再被截成 "qj_x_dmp_yingx…"）。
 *
 * 为什么不用 CSS 的 `width: max-content`：Element 把选中项放在 `.el-select__selection`
 * （flex: 1，flex-basis: 0）里，max-content 算下来只有 suffix 图标那么宽，最后被 min-width
 * 兜住 —— 表现就是"写了自适应，宽度还是没变、名字照样被截断"（已实测）。
 *
 * 所以这里直接量：先把宽度复位成 auto（否则上一次写死的宽度会让文本出省略号，量出的
 * scrollWidth 偏小、越量越窄），读选中项的 scrollWidth —— 元素是 overflow: hidden，
 * scrollWidth 就是**未截断**的内容宽 —— 再加上 wrapper 内边距与前后缀（库图标、箭头）宽度，
 * clamp 到 [min, max] 后写回 style.width。
 *
 * 调用时机：值变更 / 选项变更 / 字号变更之后，在 nextTick 里调（DOM 文本已更新）。
 */

/** 单个下拉：`el` 传 Element 组件实例的 $el（也就是 .el-select 根节点） */
export function fitSelectWidth(el, { min = 120, max = 280 } = {}) {
  if (!el || !el.isConnected) return
  const wrapper = el.querySelector('.el-select__wrapper')
  if (!wrapper) return

  // 1) 复位，避免"上一轮的固定宽度"参与这一轮测量
  el.style.width = 'auto'

  // 2) 文本宽：取所有 selected-item 里最宽的那个（选中项 / placeholder / 输入框各占一个）
  let textW = 0
  for (const item of el.querySelectorAll('.el-select__selected-item')) {
    if (item.scrollWidth > textW) textW = item.scrollWidth
  }

  // 3) 内边距 + 前后缀图标（Element 各版本的类名不同，两种都认）
  const cs = getComputedStyle(wrapper)
  const pad = (parseFloat(cs.paddingLeft) || 0) + (parseFloat(cs.paddingRight) || 0)
  const prefix = el.querySelector('.el-select__prefix, .el-input__prefix')
  const suffix = el.querySelector('.el-select__suffix, .el-input__suffix')
  const prefixW = prefix ? prefix.getBoundingClientRect().width + 6 : 0
  const suffixW = suffix ? suffix.getBoundingClientRect().width : 0

  // 4) 收口：太短的名字不至于缩成一根线，太长的名字也不至于把整条工具条挤爆
  const w = Math.min(max, Math.max(min, Math.ceil(textW + pad + prefixW + suffixW) + 4))
  el.style.width = w + 'px'
}

/** 批量版本：`els` 里可以有 null（v-if 没渲染的下拉） */
export function fitSelectWidths(pairs) {
  for (const [el, opts] of pairs) fitSelectWidth(el, opts)
}
