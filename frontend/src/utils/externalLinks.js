// ===== 外链统一走**系统默认浏览器** =====
//
// 为什么需要：桌面壳是 Tauri 的 WebView，`window.open(url)` 与 `<a target="_blank">`
// 在里面**什么都不做**（既不弹新窗，也不交给系统浏览器）—— 用户点「前往下载页」、
// AI 回答里的链接、关于页的 GitHub 链接，看起来都像"点了没反应"。
// 壳里唯一可靠的出路是让后端去喊系统（见 crates/dbmind-web 的 `sys::open_url`）。
//
// 这里做两件事：
//   1. 导出 `openExternal(url)`：给代码主动调用（如更新弹窗的「前往下载页」）；
//   2. 装一个全局点击拦截：页面上任何 http(s) 链接（含 Markdown 渲染出来的 `<a>`）
//      都改走后端 —— 一处实现覆盖全部外链，不必逐个改链接。
import { openExternalUrl } from '../api'

/** 交给系统浏览器打开；失败静默（打不开也不该弹一堆报错打扰用户） */
export const openExternal = (url) => {
  const u = String(url || '').trim()
  if (!/^https?:\/\//i.test(u)) return
  openExternalUrl(u).catch(() => { /* 忽略 */ })
}

/** 安装全局点击拦截（main.js 启动时调一次；自身幂等） */
export const installExternalLinkHandler = () => {
  if (installExternalLinkHandler._done) return
  installExternalLinkHandler._done = true
  document.addEventListener(
    'click',
    (e) => {
      const el = e.target
      const a = el && el.closest ? el.closest('a[href]') : null
      if (!a) return
      const href = a.getAttribute('href') || ''
      if (!/^https?:\/\//i.test(href)) return // 站内路由 / 页内锚点照旧
      // 捕获阶段先拦下来：别让组件自身的处理再走一遍无效的 window.open
      e.preventDefault()
      e.stopPropagation()
      openExternal(href)
    },
    true
  )
}
