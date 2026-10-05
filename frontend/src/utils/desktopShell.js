import { t } from './i18n'
/**
 * 桌面壳适配层：把「窗口控制」收敛成一套与壳无关的调用。
 *
 * 为什么要有这一层：顶栏那条（`MainView.vue` 的 `.topbar`）同时充当标题栏 ——
 * 壳把系统标题栏关掉之后，最小化/最大化/关闭与拖动都得由页面自己提供。
 * 而不同壳给的桥不一样：历史的上游 Electron 壳注入 `window.上游Win`（已经不在仓库里了），
 * 现在的桌面壳是 Tauri（`crates/dbmind-desktop`），能力要通过 `@tauri-apps/api/window` 拿。
 *
 * 两者能力相同、接口名不同。与其在视图里到处判断"现在是哪个壳"，不如在这里对一次齐：
 * 视图只认 `available / platform / minimize / toggleMaximize / close / onMaximizeChange`。
 *
 * 浏览器里两者都不存在 ⇒ `available = false`：视图不渲染窗口按钮，也不做任何拖动处理，
 * **Web 版因此一行都不受影响**。
 *
 * Tauri 的 API 走**动态 import**：浏览器永远不会请求那份代码（构建上它是独立 chunk）。
 */

/** 旧 Electron 桥（保留兼容：壳虽已不在，但判据留着，成本为零且免得日后重蹈） */
const legacy = typeof window !== 'undefined' ? window.上游Win : null

/** Tauri 2 一定会注入这个内部对象；浏览器里没有 */
const hasTauri = typeof window !== 'undefined' && !!window.__TAURI_INTERNALS__

/** 懒加载并缓存 Tauri 的当前窗口对象 */
let tauriWindowPromise = null
const tauriWindow = () => {
  if (!tauriWindowPromise) {
    tauriWindowPromise = import('@tauri-apps/api/window').then((m) => m.getCurrentWindow())
  }
  return tauriWindowPromise
}

/**
 * 失败时**必须留下痕迹** —— 这里原本是静默 catch，结果"按钮点了没反应、窗口也拖不动"
 * 完全查不出原因（踩过）。现在写 console，并把最后一条失败挂到 document.title 上：
 * 桌面壳没有控制台可看时，看一眼窗口标题就知道是哪一步被拒了。
 * 成功时不动标题，只有真的出问题才会看到它变。
 */
let lastFailure = ''
function reportFailure(step, error) {
  const detail = (error && (error.message || error.toString())) || '未知错误'
  console.error('[desktopShell] ' + step + ' 失败：', error)
  if (lastFailure === detail) return
  lastFailure = detail
  try { document.title = t('ds.windowUnavailable', { detail }) } catch { }
}

/** macOS 上系统会给红绿灯按钮，页面不该再画一套（Tauri 下 decorations 的差异见各平台的壳实现） */
const platform = legacy
  ? legacy.platform
  : (typeof navigator !== 'undefined' && /Mac/i.test(navigator.userAgent) ? 'darwin' : 'win32')

export const desktopShell = {
  /** 是否跑在某个桌面壳里（浏览器里为 false） */
  available: !!legacy || hasTauri,
  platform,

  async minimize() {
    if (legacy) return legacy.minimize()
    if (!hasTauri) return
    try { await (await tauriWindow()).minimize() } catch (e) { reportFailure('最小化', e) }
  },

  async toggleMaximize() {
    if (legacy) return legacy.toggleMaximize()
    if (!hasTauri) return
    try { await (await tauriWindow()).toggleMaximize() } catch (e) { reportFailure('最大化', e) }
  },

  async close() {
    if (legacy) return legacy.close()
    if (!hasTauri) return
    try { await (await tauriWindow()).close() } catch (e) { reportFailure('关闭', e) }
  },

  /**
   * 写窗口标题。
   *
   * 注意：页面上改 `document.title` **不会**改变 Tauri 的窗口标题（那是原生侧的属性，
   * 不像浏览器那样联动 —— 我按浏览器的直觉验证过好几轮，全是在读一个永远不会变的字符串）。
   * 需要真正改标题时必须走这条 IPC。
   */
  async setTitle(text) {
    if (!hasTauri) return
    try { await (await tauriWindow()).setTitle(String(text)) } catch (e) { reportFailure('设置标题', e) }
  },

  /**
   * 拖动窗口：**必须**在鼠标按下的用户手势里调用（浏览器/系统都要求），
   * 所以视图把它挂在 `mousedown` 上，而不是 `click`。
   */
  async startDragging() {
    if (legacy && legacy.startDragging) {
      try { return legacy.startDragging() } catch { }
    }
    if (!hasTauri) return
    try { await (await tauriWindow()).startDragging() } catch (e) { reportFailure('拖动', e) }
  },

  /**
   * 订阅「最大化状态变化」，并**同步**返回一个取消订阅的函数。
   *
   * 返回同步函数是刻意的：调用方（`onMounted` 里）习惯写成
   * `off = winApi.onMaximizeChange(cb)`、之后在 `onUnmounted` 里 `off()`。
   * 但底层（`onResized`）本身是异步的；让调用方去 await 会把它拖成一个异步清理流程，
   * 于是这里把异步藏起来，用一个"还没订上就先记着"的开关兜住。
   */
  onMaximizeChange(callback) {
    if (legacy) return legacy.onMaximizeChange(callback)
    if (!hasTauri) return () => { }

    let unsubscribe = null
    let canceled = false
    ;(async () => {
      try {
        const win = await tauriWindow()
        // 先给个初值，否则首次渲染时按钮图标与真实状态不符
        callback(await win.isMaximized())
        // 双击标题栏、拖到屏幕边缘、系统快捷键……都会改变最大化状态，
        // 所以必须跟着窗口走，不能只在点按钮时自己翻一下
        const off = await win.onResized(async () => {
          try { callback(await win.isMaximized()) } catch { }
        })
        if (canceled) off()
        else unsubscribe = off
      } catch (e) {
        // 这一条是**开机自检**：页面一加载就会调 isMaximized，被拒的话立刻就能看见
        reportFailure('读取窗口状态', e)
      }
    })()
    return () => {
      canceled = true
      if (unsubscribe) unsubscribe()
    }
  }
}
