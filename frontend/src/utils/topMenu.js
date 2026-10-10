// ===== 顶栏菜单配置（显示哪些 + 顺序）=====
// 真身存 dbmind.db（ui.topmenu，经 settings.persistUI），localStorage 是同步缓存；
// 启动水合完成后 settings 会广播 dbmind-ui-hydrated，这里重读一次。
// 消费方：MainView（渲染顶栏）、SettingsView 通用页签（配置界面）。
import { ref, computed } from 'vue'
import { Switch, Promotion, DataAnalysis, Box, Monitor, Clock, Plus } from '@element-plus/icons-vue'
import { persistUI } from './settings'
import { t } from './i18n'

export const TOP_MENU_ITEMS = [
  { id: 'compare', icon: Switch, label: () => t('nav.compare') },
  { id: 'sync', icon: Promotion, label: () => t('nav.sync') },
  { id: 'governance', icon: DataAnalysis, label: () => t('nav.governance') },
  // 驱动管理：直达「设置 → 驱动管理」页（看每种数据源的驱动装没装 / 缺哪些 / 换镜像源 / 手动上传）
  { id: 'drivers', icon: Box, label: () => t('nav.drivers') },
  { id: 'monitor', icon: Monitor, label: () => t('nav.monitor') },
  { id: 'bgCenter', icon: Clock, label: () => t('sync.bgCenter') },
  { id: 'newScript', icon: Plus, label: () => t('nav.newScript') }
]

const TOPMENU_LS = 'dbmind_topmenu'
const readTopMenuCfg = () => {
  const allIds = TOP_MENU_ITEMS.map((m) => m.id)
  try {
    const raw = JSON.parse(localStorage.getItem(TOPMENU_LS) || '{}')
    // order 只保留合法 id；**老配置里没有的新项按它在默认顺序里的位置插入**，
    // 而不是一律追加到末尾 —— 否则新增的入口永远跑到最后，摆不到产品想要的位置
    //（如「驱动管理」应紧跟在「数据治理」后面）。hidden 也按合法 id 过滤。
    const order = (Array.isArray(raw.order) ? raw.order : []).filter((id) => allIds.includes(id))
    TOP_MENU_ITEMS.forEach((m, i) => {
      if (order.includes(m.id)) return
      // 插到「默认顺序里排在它前面、且用户配置里存在」的那一项之后；前面没有就放最前
      let at = 0
      for (let j = i - 1; j >= 0; j--) {
        const prev = order.indexOf(TOP_MENU_ITEMS[j].id)
        if (prev >= 0) { at = prev + 1; break }
      }
      order.splice(at, 0, m.id)
    })
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
