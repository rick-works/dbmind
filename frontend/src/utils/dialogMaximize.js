/**
 * 功能性弹窗「右上角全屏 / 还原」——
 *
 * 为什么和 dialogResize 一样用全局挂载、而不是给每个 `el-dialog` 加属性：
 * 全站有几十个 `el-dialog`（数据传输 / 数据对比 / 备份还原 / 数据生成 / 设置 …），
 * 逐个改调用点必然漏，新增一个弹窗也会漏。这里在 body 上挂 MutationObserver，
 * 谁挂到 DOM 上就给谁装按钮，以后新写的弹窗自动继承。
 *
 * **消息弹窗（ElMessage / ElMessageBox）不参与**，太小的弹窗（< 560px，基本是
 * 「填两个字段点确定」的表单）也不装 —— 全屏对它们没意义，只会挡住标题。
 *
 * 全屏的做法：**写内联宽高**（不是 CSS 的 `!important` 覆盖）。用 `!important` 时
 * 拖右下角手柄改了尺寸也不生效（真机反馈「全屏之后再拖动窗口大小没反应」），
 * 写内联则天然可拖，还能记住原来的尺寸（还原时放回去）。`.dc-max` 这个 class 只留
 * 三件事：margin 归到 12px、放开各弹窗自己的 max-width/max-height 上限、overflow。
 *
 * 交互：点按钮切换全屏；全屏态下按 Esc 先还原、不关弹窗（Element 的 Esc 监听在
 * 冒泡阶段，这里在捕获阶段拦掉）；全屏态拖动窗口大小时跟着铺满；全屏态直接拖右下角
 * 手柄 = 先退出全屏、再从当前大小继续拖。
 */

const MIN_WIDTH_TO_ENHANCE = 560
/** 全屏时四周留白 */
const EDGE = 12
const MAX_CLASS = 'dc-max'
/** 已处理过的元素（destroy-on-close 的弹窗每次都是新元素，会重新装一遍，正确） */
const done = new WeakSet()
/** 全屏前该弹窗的内联尺寸，还原时放回去 */
const saved = new WeakMap()

/* 图标用 Material 的 fullscreen / fullscreen_exit（fill 版）：四角括号向外 = 全屏，
   向内 = 还原。手绘描边版在 16px 下会糊成一个十字，别再用。 */
const ICON_MAX =
  '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">' +
  '<path d="M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z"/></svg>'
const ICON_MIN =
  '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">' +
  '<path d="M5 16h3v3h2v-5H5v2zm3-8H5v2h5V5H8v3zm6 11h2v-3h3v-2h-5v5zm2-11V5h-2v5h5V8h-3z"/></svg>'

const isMax = (dialog) => dialog.classList.contains(MAX_CLASS)

/** 铺满视口（留 EDGE 的边） */
function fitViewport(dialog) {
  dialog.style.width = Math.max(320, window.innerWidth - EDGE * 2) + 'px'
  dialog.style.height = Math.max(240, window.innerHeight - EDGE * 2) + 'px'
}

function applyMax(dialog) {
  if (isMax(dialog)) return
  saved.set(dialog, { w: dialog.style.width, h: dialog.style.height })
  dialog.classList.add(MAX_CLASS)
  fitViewport(dialog)
}

function applyRestore(dialog) {
  if (!isMax(dialog)) return
  const prev = saved.get(dialog)
  saved.delete(dialog)
  dialog.classList.remove(MAX_CLASS)
  dialog.style.width = prev ? prev.w : ''
  dialog.style.height = prev ? prev.h : ''
}

/** 退出全屏但**保留当前尺寸**（拖动时的起点就是现在的大小） */
function dropMax(dialog) {
  saved.delete(dialog)
  dialog.classList.remove(MAX_CLASS)
}

function syncBtn(dialog, btn) {
  const on = isMax(dialog)
  btn.innerHTML = on ? ICON_MIN : ICON_MAX
  btn.title = on ? '还原窗口（Esc）' : '全屏'
  btn.setAttribute('aria-label', on ? '还原窗口' : '全屏')
}

/**
 * 对齐到 Element 自带的关闭 ✕ 上。
 *
 * 不能写死 top/right：各弹窗 header 的内边距不一样（设置页 header 59px、✕ 距顶 5px；
 * 数据传输 header 42px、✕ 距顶 1px），写死就会出现「数据传输对齐、设置对不上」。
 * 这里按 ✕ **图标**的实测中心做增量修正 —— 用增量（不是绝对定位）就不必关心弹窗边框/内边距的换算。
 */
function place(dialog, btn) {
  const br = btn.getBoundingClientRect()
  if (!br.width) return
  const cb = dialog.querySelector('.el-dialog__headerbtn')
  if (!cb) return
  const icon = cb.querySelector('svg') || cb
  const ir = icon.getBoundingClientRect()
  const dy = (ir.top + ir.height / 2) - (br.top + br.height / 2)
  if (Math.abs(dy) > 0.4) btn.style.top = ((parseFloat(btn.style.top) || 0) + dy) + 'px'
  const cr = cb.getBoundingClientRect()
  const dx = cr.left - br.right
  if (Math.abs(dx) > 0.4) btn.style.right = ((parseFloat(btn.style.right) || 48) - dx) + 'px'
}

function placeSoon(dialog, btn) {
  requestAnimationFrame(() => {
    place(dialog, btn)
    requestAnimationFrame(() => place(dialog, btn))
  })
}

function setState(dialog, btn, on) {
  const want = typeof on === 'boolean' ? on : !isMax(dialog)
  if (want) applyMax(dialog)
  else applyRestore(dialog)
  syncBtn(dialog, btn)
}

function enhance(dialog) {
  if (done.has(dialog)) return
  if (!dialog.classList.contains('el-dialog')) return
  // 消息类弹窗不装（MessageBox 渲染的是 .el-message-box，这里再兜一层）
  if (dialog.closest('.el-message-box__wrapper') || dialog.classList.contains('is-message-box')) return
  // 已经是全屏属性的弹窗（Element 的 fullscreen），不需要再给一个切换按钮
  if (dialog.classList.contains('is-fullscreen')) return
  // 窄弹窗不装：它们基本是「填两个字段点确定」，全屏只会把标题撑得很空
  const width = dialog.getBoundingClientRect().width ||
    parseFloat(dialog.style.getPropertyValue('--el-dialog-width')) || 0
  if (width && width < MIN_WIDTH_TO_ENHANCE) return

  done.add(dialog)
  const btn = document.createElement('button')
  btn.type = 'button'
  btn.className = 'dc-max-btn'
  syncBtn(dialog, btn)
  btn.addEventListener('click', (ev) => {
    ev.preventDefault()
    ev.stopPropagation()
    setState(dialog, btn)
  })
  // 挂在弹窗根节点（和缩放手柄一样）：绝对定位到右上角、压着 header，不改各弹窗的 header 插槽
  dialog.appendChild(btn)
  placeSoon(dialog, btn)

  // 全屏态直接拖右下角手柄：先退出全屏、保留当前尺寸，手柄那边就从现在的大小开始拖
  dialog.addEventListener('pointerdown', (ev) => {
    if (!isMax(dialog)) return
    if (!ev.target || !ev.target.closest || !ev.target.closest('.dc-resize-grip')) return
    dropMax(dialog)
    syncBtn(dialog, btn)
  }, true)

  // 关闭时复位：Element 关弹窗只是把 `.el-overlay` 设成 display:none，**元素不销毁**，
  // 不主动复位的话下次打开还是全屏（实测重开仍是 1416×876）。这里盯 overlay 的显隐。
  const overlay = dialog.closest('.el-overlay')
  if (overlay) {
    const watcher = new MutationObserver(() => {
      const hidden = getComputedStyle(overlay).display === 'none'
      if (hidden && isMax(dialog)) setState(dialog, btn, false)
      // 每次重新打开都重新对齐一次（标题/宽度可能变了，✕ 的位置也跟着变）
      if (!hidden) placeSoon(dialog, btn)
    })
    watcher.observe(overlay, { attributes: true, attributeFilter: ['style', 'class'] })
  }
}

export function installDialogMaximize() {
  if (typeof window === 'undefined' || typeof MutationObserver === 'undefined') return

  // 全屏态按 Esc：先还原，别让 Element 顺手把弹窗关了
  document.addEventListener('keydown', (ev) => {
    if (ev.key !== 'Escape') return
    const dialog = document.querySelector('.el-dialog.' + MAX_CLASS)
    if (!dialog) return
    ev.stopPropagation()
    ev.preventDefault()
    const btn = dialog.querySelector('.dc-max-btn')
    if (btn) setState(dialog, btn, false)
  }, true)

  // 全屏态改窗口大小：跟着铺满（内联尺寸不会自己变）；顺带把按钮重新对一次位
  window.addEventListener('resize', () => {
    document.querySelectorAll('.el-dialog.' + MAX_CLASS).forEach(fitViewport)
    document.querySelectorAll('.el-dialog .dc-max-btn').forEach((b) => {
      const d = b.closest('.el-dialog')
      if (d) place(d, b)
    })
  })

  const scan = (root) => {
    if (!root || (root.nodeType !== 1 && root.nodeType !== 9 && root.nodeType !== 11)) return
    if (root.nodeType === 1 && root.classList && root.classList.contains('el-dialog')) enhance(root)
    if (!root.querySelectorAll) return
    root.querySelectorAll('.el-dialog').forEach(enhance)
  }
  scan(document)
  const observer = new MutationObserver((records) => {
    for (const rec of records) {
      for (const node of rec.addedNodes) scan(node)
    }
  })
  observer.observe(document.body, { childList: true, subtree: true })
}
