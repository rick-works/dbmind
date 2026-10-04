// ===== 顶栏菜单配置（显示哪些 + 顺序）=====
// 真身存 dbmind.db（ui.topmenu，经 settings.persistUI），localStorage 是同步缓存；
// 启动水合完成后 settings 会广播 dbmind-ui-hydrated，这里重读一次。
// 消费方：MainView（渲染顶栏）、SettingsView 通用页签（配置界面）。
import { ref, computed } from 'vue'
import { Switch, Promotion, DataAnalysis, Monitor, Clock, Plus } from '@element-plus/icons-vue'
import { persistUI } from './settings'
import { t } from './i18n'

export const TOP_MENU_ITEMS = [
  { id: 'compare', icon: Switch, label: () => t('nav.compare') },
  { id: 'sync', icon: Promotion, label: () => t('nav.sync') },
  { id: 'governance', icon: DataAnalysis, label: () => t('nav.governance') },
  { id: 'monitor', icon: Monitor, label: () => t('nav.monitor') },
  { id: 'bgCenter', icon: Clock, label: () => t('sync.bgCenter') },
  { id: 'newScript', icon: Plus, label: () => t('nav.newScript') }
]

const TOPMENU_LS = 'dbmind_topmenu'
const readTopMenuCfg = () => {
  const allIds = TOP_MENU_ITEMS.map((m) => m.id)
  try {
    const raw = JSON.parse(localStorage.getItem(TOPMENU_LS) || '{}')
    // order 只保留合法 id，老配置里没有的新项**追加**到末尾；hidden 也按合法 id 过滤
    const order = (Array.isArray(raw.order) ? raw.order : []).filter((id) => allIds.includes(id))
    allIds.forEach((id) => { if (!order.includes(id)) order.push(id) })
    return { order, hidden: (Array.isArray(raw.hidden) ? raw.hidden : []).filter((id) => allIds.includes(id)) }
  } catch { return { order: allIds, hidden: [] } }
}

export const topMenuCfg = ref(readTopMenuCfg())
const persistDebounced = (() => {
  let tm = null
  return () => {
    clearTimeout(tm)
    tm = setTimeout(() => persistUI(TOPMENU_LS, JSON.stringify(topMenuCfg.value)), 400)
  }
})()

/** 全部菜单项，按配置顺序排列（设置界面用，含被隐藏的） */
export const topMenuAll = computed(() => {
  const byId = Object.fromEntries(TOP_MENU_ITEMS.map((m) => [m.id, m]))
  return topMenuCfg.value.order.map((id) => byId[id]).filter(Boolean)
})
/** 顶栏实际渲染的项：按 order 且未被隐藏 */
export const topMenuVisible = computed(() => topMenuAll.value.filter((m) => !topMenuCfg.value.hidden.includes(m.id)))

export const toggleTopMenu = (id) => {
  const hidden = new Set(topMenuCfg.value.hidden)
  hidden.has(id) ? hidden.delete(id) : hidden.add(id)
  topMenuCfg.value = { ...topMenuCfg.value, hidden: [...hidden] }
  persistDebounced()
}
export const moveTopMenu = (index, delta) => {
  const order = [...topMenuCfg.value.order]
  const j = index + delta
  if (j < 0 || j >= order.length) return
  ;[order[index], order[j]] = [order[j], order[index]]
  topMenuCfg.value = { ...topMenuCfg.value, order }
  persistDebounced()
}

window.addEventListener('dbmind-ui-hydrated', () => { topMenuCfg.value = readTopMenuCfg() })
