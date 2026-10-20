/**
 * 功能性弹窗「拖右下角改大小」——
 *
 * 为什么用全局挂载而不是给每个 `el-dialog` 加属性：
 * Element Plus 没有 resize 能力，要加就得改几十个调用点（数据传输 / 数据对比 / 用户表单 /
 * 设置 / 关于 …），新增一个弹窗就会漏掉。这里在 body 上挂一个 MutationObserver，
 * 谁被挂到 DOM 上就给谁装手柄 —— 以后新写的弹窗自动继承。
 *
 * **消息弹窗（ElMessage / ElMessageBox）不参与**（用户要求）：那些是"看一眼点一下"的，
 * 给它们装缩放手柄只是噪声。MessageBox 渲染的是 `.el-message-box`（不是 `.el-dialog`），
 * 这里再显式排除一层，免得 Element 以后换实现时误伤。
 *
 * 交互：右下角手柄拖动 = 同时改宽高；双击手柄 = 恢复默认大小。
 * 尺寸限制在 [最小 420×240, 视口 - 24px] 之间，拖不出屏幕。
 */

const MIN_W = 420
const MIN_H = 240
const EDGE = 24
/** 已处理过的元素（destroy-on-close 的弹窗每次都是新元素，会重新装一遍，正确） */
const done = new WeakSet()

const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v))

function enhance(dialog) {
  if (done.has(dialog)) return
  if (!dialog.classList.contains('el-dialog')) return
  // 消息类弹窗 / 全屏态（全屏时尺寸由全屏样式说了算）不装
  if (dialog.closest('.el-message-box__wrapper') || dialog.classList.contains('is-message-box')) return
  if (dialog.classList.contains('is-fullscreen')) return
  done.add(dialog)
  dialog.classList.add('dc-resizable')

  const grip = document.createElement('div')
  grip.className = 'dc-resize-grip'
  grip.title = '拖动改变大小（双击恢复默认）'
  dialog.appendChild(grip)

  grip.addEventListener('pointerdown', (ev) => {
    ev.preventDefault()
    ev.stopPropagation()
    // 以**当前渲染尺寸**为起点写进内联样式：不改之前弹窗宽度来自 width 属性、高度来自内容，
    // 拖第一下时这两者必须先落成具体像素，否则会出现"拖动时尺寸突跳"
    const rect = dialog.getBoundingClientRect()
    const startW = rect.width
    const startH = rect.height
    const startX = ev.clientX
    const startY = ev.clientY
    if (!dialog.style.width) dialog.style.width = startW + 'px'
    if (!dialog.style.height) dialog.style.height = startH + 'px'
    document.body.classList.add('dc-resizing')

    // 监听挂在 window 上（而不是用手柄的 setPointerCapture）：指针移出手柄也能继续拖，
    // 且不依赖 pointerId 的捕获语义（合成事件里捕获会抛 NotFoundError，手柄就"拖不动"了）
    const onMove = (e) => {
      const w = clamp(startW + (e.clientX - startX), MIN_W, window.innerWidth - EDGE)
      const h = clamp(startH + (e.clientY - startY), MIN_H, window.innerHeight - EDGE)
      dialog.style.width = w + 'px'
      dialog.style.height = h + 'px'
    }
    const onUp = () => {
      document.body.classList.remove('dc-resizing')
      window.removeEventListener('pointermove', onMove, true)
      window.removeEventListener('pointerup', onUp, true)
      window.removeEventListener('pointercancel', onUp, true)
    }
    window.addEventListener('pointermove', onMove, true)
    window.addEventListener('pointerup', onUp, true)
    window.addEventListener('pointercancel', onUp, true)
  })

  // 双击恢复默认：清掉内联尺寸，回到 width 属性 / 内容撑开
  grip.addEventListener('dblclick', (ev) => {
    ev.preventDefault()
    ev.stopPropagation()
    dialog.style.width = ''
    dialog.style.height = ''
  })
}

export function installDialogResize() {
  if (typeof window === 'undefined' || typeof MutationObserver === 'undefined') return
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
