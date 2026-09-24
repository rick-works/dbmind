// 主题设置：localStorage 持久化
// mode: system=跟随系统 | light=浅色 | dark=深色（默认跟随系统）
// 运行时以 <html data-theme="light|dark"> 驱动全局 CSS 变量，另做 window resize/matchMedia 监听。

export const themeKey = 'dbmind_theme'
export const themeDefaults = { mode: 'system' }

const readJSON = (key) => {
  try {
    const raw = localStorage.getItem(key)
    return raw ? { ...JSON.parse(raw) } : {}
  } catch { return {} }
}

export const getThemeSettings = () => ({ ...themeDefaults, ...readJSON(themeKey) })
export const saveThemeSettings = (patch) => {
  const next = { ...getThemeSettings(), ...patch }
  try { localStorage.setItem(themeKey, JSON.stringify(next)) } catch { /* 忽略写失败 */ }
  return next
}

// system 模式下跟随系统 prefers-color-scheme
export const systemPrefersDark = () =>
  window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches

export const resolveTheme = (mode) => {
  const m = mode || getThemeSettings().mode
  if (m === 'light' || m === 'dark') return m
  return systemPrefersDark() ? 'dark' : 'light'
}

let resolved = 'dark'
const listeners = new Set()

export const getResolvedTheme = () => resolved
export const onResolvedThemeChange = (fn) => {
  listeners.add(fn)
  return () => listeners.delete(fn)
}

// 把「主题模式」报给桌面壳（Electron 才有）：主进程会转存，用于启动页（loading.html）
// 与窗口底色 —— 否则设置里选了浅色，下次启动仍先闪一下深色启动页。
// 传的是模式而不是解析结果：system 交给主进程在启动那一刻按系统外观决定。
let pushedTheme = ''
const pushThemeToShell = (mode) => {
  const m = (mode === 'light' || mode === 'dark') ? mode : 'system'
  if (m === pushedTheme) return
  pushedTheme = m
  try { window.上游Win?.setTheme?.(m) } catch { /* 浏览器里没有桌面壳，忽略 */ }
}

// 应用到 <html>，广播变化（供 Monaco 等外部组件联动）
export const applyTheme = (mode) => {
  pushThemeToShell(mode || getThemeSettings().mode)
  const next = resolveTheme(mode)
  if (resolved === next) {
    // 即使主题未变，也保证 html 属性存在（首次进入/刷新）
    document.documentElement.setAttribute('data-theme', resolved)
    document.documentElement.style.colorScheme = resolved
    return resolved
  }
  resolved = next
  document.documentElement.setAttribute('data-theme', resolved)
  document.documentElement.style.colorScheme = resolved
  listeners.forEach((fn) => { try { fn(resolved) } catch { /* ignore */ } })
  return resolved
}

let mediaCleanup = null
let inited = false
export const initTheme = () => {
  // 只初始化一次：下面挂的 matchMedia / storage 监听是匿名函数，无法单独摘除，
  // 重复调用会把监听叠加起来（同一段逻辑被执行多次）
  if (inited) return
  inited = true
  applyTheme(getThemeSettings().mode)
  const mq = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null
  if (mq && typeof mq.addEventListener === 'function' && !mediaCleanup) {
    const onChange = () => { if (getThemeSettings().mode === 'system') applyTheme('system') }
    mq.addEventListener('change', onChange)
    mediaCleanup = () => mq.removeEventListener('change', onChange)
  }
  window.addEventListener('storage', (e) => {
    if (e.key === themeKey) {
      try { applyTheme(JSON.parse(e.newValue || '{}').mode || 'system') } catch { applyTheme('system') }
    }
  })
}
