/* v-fast-tip：快速悬浮提示（原生 title 要悬 ~1 秒才出，用户嫌慢）。
   用法：v-fast-tip="'文案'" 或 v-fast-tip="() => 动态文案"。
   规则：**只在元素带 .icon-only（收成图标态）时出现** —— 按钮有文字时提示没有信息增量，
   也避免和原生 title 叠加。指令挂载时记回调，鼠标进入 120ms 后在元素正下方居中弹出。 */
let tipEl = null
const ensureTip = () => {
  if (!tipEl) {
    tipEl = document.createElement('div')
    tipEl.className = 'fast-tip'
    document.body.appendChild(tipEl)
  }
  return tipEl
}
const hide = (el) => {
  const st = el.__fastTip
  if (!st) return
  if (st.timer) { clearTimeout(st.timer); st.timer = null }
  if (tipEl) tipEl.classList.remove('on')
}
export default {
  mounted(el, binding) {
    const st = el.__fastTip = { timer: null, value: binding.value }
    const textOf = () => {
      try { return typeof st.value === 'function' ? st.value() : st.value } catch { return '' }
    }
    st.enter = () => {
      if (!el.classList.contains('icon-only')) return
      if (st.timer) clearTimeout(st.timer)
      st.timer = setTimeout(() => {
        const text = textOf()
        if (!text) return
        const t = ensureTip()
        t.textContent = text
        t.classList.add('on')
        const r = el.getBoundingClientRect()
        const tw = t.offsetWidth
        let left = r.left + r.width / 2 - tw / 2
        left = Math.max(6, Math.min(left, window.innerWidth - tw - 6))
        t.style.left = left + 'px'
        t.style.top = Math.min(r.bottom + 6, window.innerHeight - 34) + 'px'
      }, 120)
    }
    st.leave = () => hide(el)
    el.addEventListener('mouseenter', st.enter)
    el.addEventListener('mouseleave', st.leave)
    el.addEventListener('mousedown', st.leave)
  },
  updated(el, binding) {
    if (el.__fastTip) el.__fastTip.value = binding.value
  },
  unmounted(el) {
    hide(el)
    if (el.__fastTip) {
      el.removeEventListener('mouseenter', el.__fastTip.enter)
      el.removeEventListener('mouseleave', el.__fastTip.leave)
      delete el.__fastTip
    }
  },
}
