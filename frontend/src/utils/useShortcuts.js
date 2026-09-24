import { onMounted, onBeforeUnmount } from 'vue'
import { loadShortcuts, matchesShortcut, isShortcutSuppressed } from './shortcuts'

// 当前是否有可见的模态弹层（el-dialog / el-message-box）。隐藏的（display:none）不计。
function hasVisibleOverlay() {
  const els = document.querySelectorAll('.el-overlay, .el-message-box')
  for (const el of els) {
    if (el.getClientRects().length) return true
  }
  return false
}

// 在某个视图作用域内挂载"可自定义快捷键"分发。
// rootRef: 视图根元素（仅当可见且焦点在其内时响应，避免多个页签互相干扰）
// actions: { actionId: () => void }
export function useShortcutScope(rootRef, actions) {
  const onKey = (e) => {
    // 设置页正在录制新键位时，全部视图暂停响应
    if (isShortcutSuppressed()) return
    const root = rootRef.value
    if (!root) return
    // 视图必须可见（隐藏页签 getClientRects 为空）
    if (!root.getClientRects().length) return

    const ae = document.activeElement
    if (ae && ae !== document.body && !root.contains(ae)) return
    // 焦点落在弹层（对话框/消息框/下拉浮层等）内时不响应，避免误触
    if (ae && ae !== document.body && ae.closest && ae.closest('.el-overlay, .el-message-box, .el-popper')) return
    // 焦点在 body 但存在可见弹层（如设置弹窗遮罩）时不响应，避免背后页签被触发
    if (ae === document.body && hasVisibleOverlay()) return

    const map = loadShortcuts()
    for (const [id, fn] of Object.entries(actions)) {
      if (matchesShortcut(e, map[id])) {
        e.preventDefault()
        e.stopPropagation()
        if (e.stopImmediatePropagation) e.stopImmediatePropagation()
        fn(e)
        return
      }
    }
  }
  onMounted(() => window.addEventListener('keydown', onKey, true))
  onBeforeUnmount(() => window.removeEventListener('keydown', onKey, true))
}
