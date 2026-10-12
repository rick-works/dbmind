/**
 * 界面尺寸偏好：顶部菜单文字 / 顶部菜单图标 / 左侧树文字。
 *
 * 存放：localStorage 的 `dbmind_appearance`，并按 settings.js 的 UI_BACKEND_KEYS 镜像到后端
 * （与主题、语言、顶栏菜单同一套机制）。
 *
 * 应用方式：写到 :root 上的三个 CSS 变量，组件样式里用
 *   var(--dc-nav-fs, 14px) / var(--dc-nav-icon, 16px) / var(--dc-tree-fs, 14px) 去读。
 * 这样"改完立刻全局生效"，不用把值一层层传给组件，也不怕漏掉某个用到的地方。
 */
import { persistUI } from './settings'

export const APPEARANCE_KEY = 'dbmind_appearance'

export const appearanceDefaults = {
  navFontSize: 14, // 顶部菜单文字（px）
  navIconSize: 16, // 顶部菜单图标（px）
  treeFontSize: 14,
  tabFontSize: 14  // 工作区页签（TAB 栏）文字（px） // 左侧树文字（px）
}

// 取值范围：只做收敛，避免手填 0 或 200 把界面搞坏。
// 上限刻意不放大：这几个位置是"微调"，不是"缩放整个应用"。
const RANGES = {
  navFontSize: [12, 18],
  navIconSize: [12, 22],
  treeFontSize: [12, 18],
  tabFontSize: [12, 18]
}

const clampOne = (key, v) => {
  const [min, max] = RANGES[key]
  const n = Math.round(Number(v))
  return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : appearanceDefaults[key]
}

export const getAppearance = () => {
  let raw = {}
  try {
    raw = JSON.parse(localStorage.getItem(APPEARANCE_KEY) || '{}') || {}
  } catch {
    raw = {}
  }
  return {
    navFontSize: clampOne('navFontSize', raw.navFontSize),
    navIconSize: clampOne('navIconSize', raw.navIconSize),
    treeFontSize: clampOne('treeFontSize', raw.treeFontSize),
    tabFontSize: clampOne('tabFontSize', raw.tabFontSize)
  }
}

/** 把尺寸写到 :root 的 CSS 变量上（不传则按当前存储应用） */
export const applyAppearance = (a = getAppearance()) => {
  const root = document.documentElement
  root.style.setProperty('--dc-nav-fs', a.navFontSize + 'px')
  root.style.setProperty('--dc-nav-icon', a.navIconSize + 'px')
  root.style.setProperty('--dc-tree-fs', a.treeFontSize + 'px')
  root.style.setProperty('--dc-tab-fs', a.tabFontSize + 'px')
  // 左侧树的图标跟着文字走（+1px）：只放大文字、图标原地不动会显得头重脚轻
  root.style.setProperty('--dc-tree-icon', a.treeFontSize + 1 + 'px')
}

/** 保存并立即应用；返回收敛后的值（供表单回填，避免输入框里留着越界的数字） */
export const saveAppearance = (a) => {
  const next = {
    navFontSize: clampOne('navFontSize', a && a.navFontSize),
    navIconSize: clampOne('navIconSize', a && a.navIconSize),
    treeFontSize: clampOne('treeFontSize', a && a.treeFontSize),
    tabFontSize: clampOne('tabFontSize', a && a.tabFontSize)
  }
  persistUI(APPEARANCE_KEY, JSON.stringify(next))
  applyAppearance(next)
  return next
}
