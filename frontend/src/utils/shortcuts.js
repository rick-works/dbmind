// ===== 可自定义快捷键 =====
// 存储：localStorage['dbmind_shortcuts']，结构 { [actionId]: 'Ctrl+Enter' | '' }
// '' 表示未绑定。所有键以 '+'-joined token 保存，如 'Shift+Alt+F'、'F5'。

export const SHORTCUTS_STORAGE = 'dbmind_shortcuts'
export const SHORTCUTS_EVT = '上游-shortcuts-changed'

// ⚠️ 这里**不再存中文文案**，只存 id / 分组 / 默认键位。
// 界面上显示的组名与说明由字典给，键名由 id 直接派生：
//   分组  key = 'editor'      → 字典键 'shortcut.group.editor'
//   动作  id  = 'query.run'   → 字典键 'shortcut.query.run.label' / 'shortcut.query.run.desc'
// 为什么派生而不是给每条再加 i18nKey 字段：id 本身就是唯一且带命名空间的，
// 手工维护 20×2 个键名迟早会漏一个；派生则不可能漏 —— 缺键时界面直接显示键名，一眼可见。
export const SHORTCUT_GROUPS = [
  { key: 'editor' },
  { key: 'sqlSel' },
  { key: 'grid' }
]

export const SHORTCUT_DEFS = [
  { id: 'query.run', group: 'editor', default: 'Ctrl+Enter' },
  { id: 'query.runAll', group: 'editor', default: 'Ctrl+Shift+Enter' },
  { id: 'query.format', group: 'editor', default: 'Shift+Alt+F' },
  { id: 'query.comment', group: 'editor', default: 'Ctrl+/' },
  { id: 'query.save', group: 'editor', default: 'Ctrl+S' },
  { id: 'query.stop', group: 'editor', default: 'Ctrl+F2' },
  { id: 'query.upper', group: 'sqlSel', default: 'Ctrl+Shift+U' },
  { id: 'query.lower', group: 'sqlSel', default: 'Ctrl+Shift+L' },
  { id: 'query.oneLine', group: 'sqlSel', default: 'Ctrl+Shift+M' },
  { id: 'query.copyInList', group: 'sqlSel', default: 'Ctrl+Shift+X' },
  { id: 'query.copyOneLine', group: 'sqlSel', default: 'Alt+Shift+L' },
  { id: 'query.selectStatement', group: 'sqlSel', default: 'Ctrl+Shift+Y' },
  { id: 'data.refresh', group: 'grid', default: 'F5' },
  { id: 'data.save', group: 'grid', default: 'Ctrl+S' },
  { id: 'data.addRow', group: 'grid', default: 'Alt+Insert' },
  { id: 'data.deleteRow', group: 'grid', default: 'Ctrl+Delete' },
  { id: 'data.revert', group: 'grid', default: 'Ctrl+Shift+Z' },
  { id: 'data.fillDown', group: 'grid', default: 'Ctrl+D' },
  { id: 'data.clearCells', group: 'grid', default: 'Delete' },
  { id: 'data.autoFit', group: 'grid', default: 'Alt+Shift+A' }
]

const DEFAULTS = {}
SHORTCUT_DEFS.forEach(d => { DEFAULTS[d.id] = d.default })

let cache = null

function build(map) {
  const out = {}
  Object.keys(DEFAULTS).forEach(id => { out[id] = DEFAULTS[id] })
  if (map) {
    Object.keys(map).forEach(id => {
      if (id in DEFAULTS) out[id] = typeof map[id] === 'string' ? map[id] : DEFAULTS[id]
    })
  }
  return out
}

export function loadShortcuts() {
  if (cache) return cache
  try {
    cache = build(JSON.parse(localStorage.getItem(SHORTCUTS_STORAGE) || '{}'))
  } catch {
    cache = build(null)
  }
  return cache
}

export function saveShortcuts(map) {
  const full = build(map)
  try {
    localStorage.setItem(SHORTCUTS_STORAGE, JSON.stringify(full))
  } catch { /* ignore */ }
  cache = full
  window.dispatchEvent(new Event(SHORTCUTS_EVT))
  return full
}

export function resetShortcutCache() {
  cache = null
}

// 录制改键期间置为 true，令各视图的快捷键分发暂停响应，避免设置弹窗背后误触发
let suppressed = false
export function setShortcutSuppressed(v) {
  suppressed = !!v
}
export function isShortcutSuppressed() {
  return suppressed
}

// 把 KeyboardEvent 规范化为键位 token 字符串（无法识别时返回 null）
const MOD_KEYS = new Set(['Control', 'Shift', 'Alt', 'Meta'])
const SPECIAL_KEYS = new Set([
  'Enter', 'Tab', 'Space', 'Esc', 'Backspace', 'Delete', 'Insert',
  'Home', 'End', 'PageUp', 'PageDown',
  'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'
])

export function eventToKeys(e) {
  if (e.isComposing) return null
  if (MOD_KEYS.has(e.key)) return null
  const mods = []
  // Ctrl / Cmd 视为同一修饰符（兼容 mac 的 Cmd）
  if (e.ctrlKey || e.metaKey) mods.push('Ctrl')
  if (e.altKey) mods.push('Alt')
  if (e.shiftKey) mods.push('Shift')

  let main = null
  const k = e.key
  if (k === 'Escape') main = 'Esc'
  else if (k === ' ') main = 'Space'
  else if (/^[a-zA-Z0-9]$/.test(k)) main = k.toUpperCase()
  else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(k)) main = k
  else if (SPECIAL_KEYS.has(k)) main = k
  else if (k && k.length === 1) main = k
  if (!main) return null
  return [...mods, main].join('+')
}

// 判断某次 keydown 是否命中配置的键位
export function matchesShortcut(e, keys) {
  if (!keys) return false
  const cur = eventToKeys(e)
  return cur === keys
}

// 键位字符串拆成 token 数组，用于渲染 <kbd>
export function keysTokens(keys) {
  if (!keys) return []
  return keys.split('+')
}
