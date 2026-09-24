// 轻量 i18n：字典 + 一个响应式 locale，零第三方依赖。
//
// 为什么不引 vue-i18n：本项目**所有文案都是硬编码中文**（57 个 .vue + 41 个 .js，
// 约 6300 行含中文、数千条去重字符串），无论用哪个库，真正的工作量都在"把每一句
// 搬进字典"上 —— 而这件事只能按模块分批做。既然如此，先用一层自己能看懂的小实现
// 把地基和外壳立起来，避免为了几千条文案的迁移再引入一个运行时依赖。
//
// 设计取向（三条，缺一不可）：
// 1. **缺键回落中文**：翻译是分批推进的，没搬过来的键必须显示中文原文，
//    而不是把 `settings.tab.driver` 这种键名甩到界面上。
// 2. **扁平点号键**：`nav.settings` 这种字符串键，字典就是一个普通对象 ——
//    可以直接 grep、可以一眼看出缺了哪个，不需要递归查找函数。
// 3. **改了就全局刷新**：`t()` 读的是响应式 `locale`，模板里用 `$t('x')` 会正常
//    建立依赖，切换语言即时生效，不需要刷新页面。
import { ref } from 'vue'
import zhCN from '../locales/zh-CN'
import enUS from '../locales/en-US'

/** 可切换的语言。`code` 同时用作 <html lang> 与 localStorage 的值。 */
export const LOCALES = [
  { code: 'zh-CN', label: '简体中文', enLabel: 'Chinese (Simplified)' },
  { code: 'en-US', label: 'English', enLabel: 'English' }
]

const STORAGE_KEY = 'dbmind_locale'
/** 回落语言：也是"缺键时显示什么"的答案。 */
const FALLBACK = 'zh-CN'
const DICTS = { 'zh-CN': zhCN, 'en-US': enUS }

/** 把各种写法（en / en-US / zh / zh-Hans-CN）归一到我们支持的两种。 */
const normalize = (code) => {
  const value = String(code || '').toLowerCase()
  if (value.startsWith('en')) return 'en-US'
  if (value.startsWith('zh')) return 'zh-CN'
  return ''
}

const readStored = () => {
  try { return normalize(localStorage.getItem(STORAGE_KEY)) } catch { return '' }
}

/**
 * 首次启动（没存过）时跟随系统语言：英文系统进来就是英文，中文系统照旧中文。
 * 存过就以用户的显式选择为准 —— 之后不再被系统语言覆盖。
 */
const detect = () => {
  const stored = readStored()
  if (stored) return stored
  try { return normalize(navigator.language) || FALLBACK } catch { return FALLBACK }
}

/** 当前语言（响应式）。模板/计算属性读它就会自动跟随切换刷新。 */
export const locale = ref(detect())

/**
 * 取文案。`params` 做 `{name}` 形式的简单插值（够用，不做复数/日期这类重活）。
 *
 * 查不到就回落中文，再查不到才返回键名本身 —— 键名露到界面上是"这里漏了"的
 * 显式信号，比显示空白好排查。
 */
export const t = (key, params) => {
  const k = String(key == null ? '' : key)
  const dict = DICTS[locale.value] || {}
  const fallback = DICTS[FALLBACK] || {}
  let text = Object.prototype.hasOwnProperty.call(dict, k) ? dict[k] : undefined
  if (text === undefined) {
    text = Object.prototype.hasOwnProperty.call(fallback, k) ? fallback[k] : k
  }
  if (!params) return text
  return String(text).replace(/\{(\w+)\}/g, (whole, name) =>
    Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : whole)
}

/**
 * 字典里有没有收录这个键（当前语言或回落语言任一有就算）。
 *
 * 用在"后端也会下发中文文案"的地方：例如质量规则类型目录由后端给出 label/hint，
 * 前端只覆盖自己收录过的那些；没有它就没法区分"收录过但译文恰好相同"和"压根没收录"，
 * 于是要么漏出中文、要么把键名甩到界面上。
 */
export const te = (key) => {
  const k = String(key == null ? '' : key)
  return Object.prototype.hasOwnProperty.call(DICTS[locale.value] || {}, k) ||
    Object.prototype.hasOwnProperty.call(DICTS[FALLBACK] || {}, k)
}

/** 切换语言：写 localStorage、同步 <html lang>、更新标题，其余交给响应式。 */
export const setLocale = (code) => {
  const next = normalize(code) || FALLBACK
  if (next === locale.value) return
  locale.value = next
  try { localStorage.setItem(STORAGE_KEY, next) } catch { /* 隐私模式：忽略 */ }
  applyDocumentLang()
  applyDocumentTitle()
}

const applyDocumentLang = () => {
  try { document.documentElement.setAttribute('lang', locale.value) } catch { /* SSR/测试环境 */ }
}

const applyDocumentTitle = () => {
  try { document.title = t('app.title') } catch { /* 同上 */ }
}

/** 启动时调用一次：把 lang / 标题按当前语言落到位。 */
export const initI18n = () => {
  applyDocumentLang()
  applyDocumentTitle()
}

/** `<script setup>` 里用：`const { t } = useI18n()`。 */
export const useI18n = () => ({ t, te, locale, setLocale, LOCALES })
