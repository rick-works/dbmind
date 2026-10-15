// ===== 顶栏菜单配置（显示哪些 + 顺序）=====
// 真身存 dbmind.db（ui.topmenu，经 settings.persistUI），localStorage 是同步缓存；
// 启动水合完成后 settings 会广播 dbmind-ui-hydrated，这里重读一次。
// 消费方：MainView（渲染顶栏）、SettingsView 通用页签（配置界面）。
import { ref, computed } from 'vue'
import { Switch, Promotion, DataAnalysis, Box, Monitor, Clock, Plus, User, Tickets } from '@element-plus/icons-vue'
import { persistUI } from './settings'
import { t } from './i18n'

export const TOP_MENU_ITEMS = [
  // 默认顺序（产品定版）：前 6 个直显在顶栏，其余进「更多」下拉（见 MainView TOP_MENU_MAX）。
  // 顺序用户仍可在「设置 → 通用 → 顶栏菜单」里调。
  { id: 'newScript', icon: Plus, label: () => t('nav.newScript') },
  // 脚本管理：跨连接/跨库浏览、搜索、增删改查用户保存的脚本（增/改在 SQL 编辑器里进行）
  { id: 'scripts', icon: Tickets, label: () => t('nav.scripts') },
  { id: 'users', icon: User, label: () => t('nav.userManage') },
  { id: 'sync', icon: Promotion, label: () => t('nav.sync') },
  { id: 'compare', icon: Switch, label: () => t('nav.compare') },
  // 驱动管理：直达「设置 → 驱动管理」页（看每种数据源的驱动装没装 / 缺哪些 / 换镜像源 / 手动上传）
  { id: 'drivers', icon: Box, label: () => t('nav.drivers') },
  { id: 'bgCenter', icon: Clock, label: () => t('sync.bgCenter') },
  { id: 'monitor', icon: Monitor, label: () => t('nav.monitor') },
  { id: 'governance', icon: DataAnalysis, label: () => t('nav.governance') }
]

const TOPMENU_LS = 'dbmind_topmenu'
// 配置版本：默认顺序整体改动时 +1 —— 老配置（无版本号或版本号旧）**一次性**切到新默认，
// 之后用户在设置里的开关/排序照常保留（保存时带上当前版本号）。
const TOPMENU_VERSION = 2
const readTopMenuCfg = () => {
  const allIds = TOP_MENU_ITEMS.map((m) => m.id)
  const hiddenOf = (raw) => (Array.isArray(raw.hidden) ? raw.hidden : []).filter((id) => allIds.includes(id))
  try {
    const raw = JSON.parse(localStorage.getItem(TOPMENU_LS) || '{}')
    // 旧版本配置：直接采用**新默认顺序**（保留用户的隐藏项），不按老 order 排 ——
    // 否则老用户看到的还是上一版默认布局，产品改了默认顺序等于没改。
    if (raw.v !== TOPMENU_VERSION) {
      return { v: TOPMENU_VERSION, order: [...allIds], hidden: hiddenOf(raw) }
    }
    // order 只保留合法 id；**配置里没有的新项按它在默认顺序里的位置插入**，
    // 而不是一律追加到末尾 —— 否则新增的入口永远跑到最后，摆不到产品想要的位置。
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
    return { v: TOPMENU_VERSION, order, hidden: hiddenOf(raw) }
  } catch { return { v: TOPMENU_VERSION, order: [...allIds], hidden: [] } }
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
