/* v-fast-tip：快速悬浮提示（原生 title 要悬 ~1 秒才出，用户嫌慢）。
   用法：v-fast-tip="'文案'" 或 v-fast-tip="() => 动态文案"。
   规则：**只在收成图标态时出现** —— 按钮有文字时提示没有信息增量，也避免和原生 title 叠加。
   指令挂载时记回调，鼠标进入 120ms 后在元素正下方居中弹出。

   「收成图标」的判定必须**向上找最近的 .icon-only**，不能只看元素自身：
   el-dropdown 的 data-g 挂在组件根 div 上（JS 加 .icon-only 也是加在它身上），
   而 v-fast-tip 只能挂在内层的触发 button 上 —— 只看自身的话这几个下拉永远不弹
   （AiPanel 的数据源/聚焦表/知识库/模型、SQL 工具条的模型/AI 助手都踩过）。 */
let tipEl = null
const ensureTip = () => {
  if (!tipEl) {
    tipEl = document.createElement('div')
    tipEl.className = 'fast-tip'
    document.body.appendChild(tipEl)
  }
  return tipEl
}
/** 自身或最近的祖先容器是否处于「收成图标」态（el-dropdown 的 .icon-only 在外层 div 上） */
const isIconOnly = (el) => !!(el.closest && el.closest('.icon-only'))

const hide = (el) => {
  const st = el.__fastTip
  if (!st) return
  if (st.timer) { clearTimeout(st.timer); st.timer = null }
  if (tipEl) tipEl.classList.remove('on')
}
export default {
  mounted(el, binding) {
  const st = el.__fastTip = { timer: null, value: binding.value, always: !!binding.modifiers.always }
  const textOf = () => {
    try { return typeof st.value === 'function' ? st.value() : st.value } catch { return '' }
  }
  st.enter = () => {
    // .always：这个提示是无条件要的（底栏「深度思考 / 记事」这类模式入口永远不收成图标，
    // 但光看一个词看不出它做什么，必须悬停就有说明）
    if (!st.always && !isIconOnly(el)) return
      if (st.timer) clearTimeout(st.timer)
      st.timer = setTimeout(() => {
        const text = textOf()
        if (!text) return
        const t = ensureTip()
        t.textContent = text
        t.classList.add('on')
        const r = el.getBoundingClientRect()
        const tw = t.offsetWidth
        const th = t.offsetHeight
        let left = r.left + r.width / 2 - tw / 2
        left = Math.max(6, Math.min(left, window.innerWidth - tw - 6))
        // 默认在下方；AI 底栏的 chip 就贴着窗口底部，下面放不下（长表名列表会换行）时翻到上方，
        // 否则会被夹在视口边缘、文字超出去
        let top = r.bottom + 6
        if (top + th > window.innerHeight - 6) top = Math.max(6, r.top - 6 - th)
        t.style.left = left + 'px'
        t.style.top = top + 'px'
      }, 120)
    }
    st.leave = () => hide(el)
    el.addEventListener('mouseenter', st.enter)
    el.addEventListener('mouseleave', st.leave)
    el.addEventListener('mousedown', st.leave)
  },
  updated(el, binding) {
    if (!el.__fastTip) return
    el.__fastTip.value = binding.value
    el.__fastTip.always = !!binding.modifiers.always
  },
  unmounted(el) {
    hide(el)
    if (el.__fastTip) {
      el.removeEventListener('mouseenter', el.__fastTip.enter)
      el.removeEventListener('mouseleave', el.__fastTip.leave)
      el.removeEventListener('mousedown', el.__fastTip.leave)
      delete el.__fastTip
    }
  },
}
