import { createApp } from 'vue'
// ==================== Element Plus 按需引入 ====================
// 不再全量 app.use(ElementPlus) 与 dist/index.css：仅注册实际用到的组件与其样式，
// 打包体积显著减小（element chunk 1.17MB → 明显更小）。
import {
  ElAlert, ElAutocomplete, ElButton, ElButtonGroup, ElCheckbox, ElCheckboxGroup, ElDatePicker, ElDialog, ElDivider, ElDrawer,
  ElDropdown, ElDropdownItem, ElDropdownMenu, ElEmpty, ElForm, ElFormItem, ElIcon,
  ElInput, ElInputNumber, ElOption, ElOptionGroup, ElPagination, ElProgress,
  ElRadio, ElRadioButton, ElRadioGroup, ElScrollbar, ElSelect, ElSlider, ElStep, ElSteps, ElSwitch, ElTabPane, ElTable,
  ElTableColumn, ElTabs, ElTag, ElTooltip, ElTree, ElUpload, ElConfigProvider,
  ElMessage
} from 'element-plus'
import 'element-plus/theme-chalk/base.css'
import 'element-plus/es/components/alert/style/css'
import 'element-plus/es/components/autocomplete/style/css'
import 'element-plus/es/components/button/style/css'
import 'element-plus/es/components/button-group/style/css'
import 'element-plus/es/components/checkbox/style/css'
import 'element-plus/es/components/checkbox-group/style/css'
import 'element-plus/es/components/config-provider/style/css'
import 'element-plus/es/components/date-picker/style/css'
import 'element-plus/es/components/dialog/style/css'
import 'element-plus/es/components/divider/style/css'
import 'element-plus/es/components/drawer/style/css'
import 'element-plus/es/components/dropdown/style/css'
import 'element-plus/es/components/dropdown-item/style/css'
import 'element-plus/es/components/dropdown-menu/style/css'
import 'element-plus/es/components/empty/style/css'
import 'element-plus/es/components/form/style/css'
import 'element-plus/es/components/form-item/style/css'
import 'element-plus/es/components/icon/style/css'
import 'element-plus/es/components/input/style/css'
import 'element-plus/es/components/input-number/style/css'
import 'element-plus/es/components/loading/style/css'
import 'element-plus/es/components/message/style/css'
import 'element-plus/es/components/message-box/style/css'
import 'element-plus/es/components/option/style/css'
import 'element-plus/es/components/option-group/style/css'
import 'element-plus/es/components/pagination/style/css'
import 'element-plus/es/components/progress/style/css'
import 'element-plus/es/components/radio/style/css'
import 'element-plus/es/components/radio-button/style/css'
import 'element-plus/es/components/radio-group/style/css'
import 'element-plus/es/components/scrollbar/style/css'
import 'element-plus/es/components/select/style/css'
import 'element-plus/es/components/slider/style/css'
import 'element-plus/es/components/step/style/css'
import 'element-plus/es/components/steps/style/css'
import 'element-plus/es/components/switch/style/css'
import 'element-plus/es/components/tab-pane/style/css'
import 'element-plus/es/components/table/style/css'
import 'element-plus/es/components/table-column/style/css'
import 'element-plus/es/components/tabs/style/css'
import 'element-plus/es/components/tag/style/css'
import 'element-plus/es/components/tooltip/style/css'
import 'element-plus/es/components/tree/style/css'
import 'element-plus/es/components/upload/style/css'
import 'element-plus/es/components/collapse/style/css'
import 'element-plus/es/components/collapse-item/style/css'
// 图标：仅注册项目实际使用的图标（约 79/860 个），避免全量图标打入包内
import {
  ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Bell, BellFilled, Bottom, BottomRight,
  Box, Brush, Calendar, CaretBottom, CaretRight, ChatDotRound, Check, CircleCheck,
  CircleCheckFilled, CircleClose, CircleCloseFilled, CirclePlus, Close, Coin,
  CollectionTag, Connection, CopyDocument, Cpu, DataLine, Delete, Document,
  DocumentAdd, DocumentCopy, DocumentRemove, Download, Edit, EditPen, Expand,
  Files, FirstAidKit, Fold, Folder, FolderOpened, Grid, InfoFilled, Key, Lightning,
  Link, Loading, Lock, MagicStick, Minus, Mouse, Operation, Plus, Pointer,
  Promotion, QuestionFilled, Refresh, RefreshRight, Right, ScaleToOriginal, Search,
  Sell, SetUp, Setting, StarFilled, Switch, SwitchButton, Timer, Top, TopRight,
  TrendCharts, Upload, UploadFilled, User, VideoPause, VideoPlay, View, WarningFilled
} from '@element-plus/icons-vue'
// Monaco 不在首屏加载：整包引入会连带 typescript / css / html 语言服务（约 8MB），
// 且作为主包静态依赖会在 index.html 生成 modulepreload，首屏无条件下载 2.6MB。
// 改为由编辑器组件按需调用 utils/monaco.js 的 ensureMonaco()（单例 Promise），
// 首次打开 SQL 编辑器时才加载。详见该文件注释。
import App from './App.vue'
import router from './router'
import './styles/index.css'
import { getNotifySettings, hydrateUIFromBackend, reloadEditorSettings, reloadQuerySettings } from './utils/settings'
import { initTheme } from './utils/theme'
import { t, initI18n, refreshLocaleFromStorage } from './utils/i18n'
import { resetShortcutCache } from './utils/shortcuts'

// 清理历史遗留的浏览器存储键。
//
// 项目改名前用的前缀（`xplore_*` / `dc_*` / `easydb.*`）、以及已删掉的许可弹窗标记，
// 都还躺在用户浏览器里。这里**不做迁移、直接删**：那几项设置（主题 / 快捷键 / 分组…）
// 已按新键重新设过，留着旧键只会让人对着"这个键是谁写的"发愣。
// 启动时执行、幂等；存储不可用（隐私模式）时静默跳过，绝不影响启动。
const LEGACY_LOCAL_KEYS = [
  'xplore_theme', 'xplore_shortcuts', 'xplore_editor', 'xplore_query', 'xplore_notify',
  'dc_folders', 'easydb.recent-connection-ids', 'ai-panel-width',
  // 许可相关功能已整体移除，这个"弹窗不再提示"标记也没用了
  'dbmind_license_prompt_dismissed'
]
const LEGACY_SESSION_KEYS = ['dc.session', 'dc.currentConnId']
// 带连接 id 的动态键（更名前的库列表缓存）：按前缀扫
const LEGACY_PREFIXES = ['xplore.dblist.']

for (const key of LEGACY_LOCAL_KEYS) {
  try { window.localStorage.removeItem(key) } catch (e) { /* 存储不可用就算了 */ }
}
for (const key of LEGACY_SESSION_KEYS) {
  try { window.sessionStorage.removeItem(key) } catch (e) { /* 同上 */ }
}
for (const store of [window.localStorage, window.sessionStorage]) {
  try {
    for (let i = store.length - 1; i >= 0; i--) {
      const key = store.key(i)
      if (key && LEGACY_PREFIXES.some(prefix => key.startsWith(prefix))) store.removeItem(key)
    }
  } catch (e) { /* 同上 */ }
}

const app = createApp(App)
for (const comp of [
  ElAlert, ElAutocomplete, ElButton, ElButtonGroup, ElCheckbox, ElCheckboxGroup, ElDatePicker, ElDialog, ElDivider, ElDrawer,
  ElDropdown, ElDropdownItem, ElDropdownMenu, ElEmpty, ElForm, ElFormItem, ElIcon,
  ElInput, ElInputNumber, ElOption, ElOptionGroup, ElPagination, ElProgress,
  ElRadio, ElRadioButton, ElRadioGroup, ElScrollbar, ElSelect, ElSlider, ElStep, ElSteps, ElSwitch, ElTabPane, ElTable,
  ElTableColumn, ElTabs, ElTag, ElTooltip, ElTree, ElUpload, ElConfigProvider
]) {
  app.use(comp)
}
const icons = {
  ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Bell, BellFilled, Bottom, BottomRight,
  Box, Brush, Calendar, CaretBottom, CaretRight, ChatDotRound, Check, CircleCheck,
  CircleCheckFilled, CircleClose, CircleCloseFilled, CirclePlus, Close, Coin,
  CollectionTag, Connection, CopyDocument, Cpu, DataLine, Delete, Document,
  DocumentAdd, DocumentCopy, DocumentRemove, Download, Edit, EditPen, Expand,
  Files, FirstAidKit, Fold, Folder, FolderOpened, Grid, InfoFilled, Key, Lightning,
  Link, Loading, Lock, MagicStick, Minus, Mouse, Operation, Plus, Pointer,
  Promotion, QuestionFilled, Refresh, RefreshRight, Right, ScaleToOriginal, Search,
  Sell, SetUp, Setting, StarFilled, Switch, SwitchButton, Timer, Top, TopRight,
  TrendCharts, Upload, UploadFilled, User, VideoPause, VideoPlay, View, WarningFilled
}
for (const [key, component] of Object.entries(icons)) {
  app.component(key, component)
}
app.use(router)

// 全局文案函数：模板里写 `$t('nav.settings')` 即可。
// 挂成 globalProperties 而不是每个组件各自 import —— 文案基本都在模板里，
// 逐个 import 只会让每个文件多一行噪声；而 `t()` 内部读的是响应式 locale，
// 所以切换语言时用到它的组件会正常重渲染，不需要刷新页面。
app.config.globalProperties.$t = t

// 全局错误捕获：打印完整堆栈，便于定位问题
app.config.errorHandler = (err, instance, info) => {
  console.error('[Vue Error]', err, '\ninfo:', info, '\ncomponent:', instance && instance.$.type ? instance.$.type.name || instance.$.type.__name : 'unknown')
  if (err && err.stack) console.error('[Vue Error Stack]', err.stack)
}
window.addEventListener('error', (e) => {
  console.error('[Global Error]', e.error || e.message)
  if (e.error && e.error.stack) console.error('[Global Error Stack]', e.error.stack)
})
window.addEventListener('unhandledrejection', (e) => {
  console.error('[Unhandled Rejection]', e.reason)
  if (e.reason && e.reason.stack) console.error('[Unhandled Rejection Stack]', e.reason.stack)
})

// 全局 Message 统一风格：
//  - 正确/普通提示（success/info）→ 屏幕顶部小条
//  - 错误提示（error）→ 屏幕中央宽幅弹出，信息完整、支持换行、便于查看详情
//  - 轻量校验提示（warning）→ 保持顶部短提示，不打断连续操作
// 注意：element-plus 的 ElMessage.error(msg, appContext) 第二参数是 appContext，
// 不能把 duration/showClose 等 options 作为第二参数传入，否则会被当成 appContext，
// 导致组件渲染时 Object.create(appContext.provides) 崩溃。
// 必须将 options 合并进第一个参数（消息对象）。
// 通知设置（设置页 → dbmind_notify）：success 控制成功提示是否显示；
// autoClose 控制错误提示是否自动关闭（自动关闭时长 8s；关闭则错误常驻中央直至手动关闭）。
// **每次弹出时实时读**：这里曾是启动时的一次性快照，设置页改完要重启才生效 ——
// 而通知恰是最常想立刻关掉的东西。localStorage 读取的开销远小于一次弹层本身。
const notifyNow = () => getNotifySettings()

// 转义 HTML，避免错误详情中特殊字符破坏样式（换行以 <br/> 呈现）
const escHtml = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]))
// 从各种形态的错误对象中提取可读文本，尽量还原完整错误详情
const toErrText = (msg) => {
  if (typeof msg === 'string') return msg
  if (msg instanceof Error) return msg.message || String(msg)
  if (msg && typeof msg === 'object') {
    if (msg.__v_isVNode) return ''
    if (typeof msg.message === 'string') return msg.message
    try { return JSON.stringify(msg) } catch { return String(msg) }
  }
  return msg == null ? '' : String(msg)
}

const mergeMsgOpts = (orig, msg, options, defaults) => {
  const isVNode = msg && typeof msg === 'object' && msg.__v_isVNode
  const payload = (typeof msg === 'string' || isVNode) ? { message: msg } : (msg && typeof msg === 'object' ? msg : {})
  return orig({ ...payload, ...defaults, ...options })
}

const origError = ElMessage.error
ElMessage.error = (msg, options) => {
  const raw = toErrText(msg)
  // Error 对象额外打印完整堆栈到控制台，便于定位根因（界面仍给出行信息）
  if (msg instanceof Error && msg.stack) console.error('[DBmind 错误详情]', msg.stack)
  // 含换行或内容较长时按 HTML 渲染，完整展示多行细节
  const needHtml = /[\r\n]/.test(raw) || raw.length > 100
  const message = needHtml
    ? { dangerouslyUseHTMLString: true, message: escHtml(raw).replace(/\r?\n/g, '<br/>') }
    : raw
  mergeMsgOpts(origError, message, options, { duration: notifyNow().autoClose ? 8000 : 0, showClose: true, grouping: true })
}
const origSuccess = ElMessage.success
ElMessage.success = (msg, options) => {
  if (!notifyNow().success) return
  mergeMsgOpts(origSuccess, msg, options, { duration: 2500, grouping: true })
}
const origInfo = ElMessage.info
ElMessage.info = (msg, options) => {
  if (!notifyNow().success) return
  mergeMsgOpts(origInfo, msg, options, { duration: 2500, grouping: true })
}
// 应用持久化的主题模式（浅色 / 深色 / 跟随系统），并监听系统外观变化
// —— 全部 UI 设置（编辑器/格式化/查询/通知/主题/语言/快捷键）的真身在 dbmind.db。
// **挂载必须同步在前，水合放在挂载之后**：真机踩过 —— await 水合后再挂载，路由
// 异步组件会挂到尚未就绪的 DOM 时序上，成片报「insertBefore/minimap of null」
// 把工作区打挂。水合完成后等「路由就绪 + 下一帧」再刷新共享快照（多数消费方
// 响应式，设置即刻跟上）；后端不可达时沿用本地缓存，绝不阻塞启动。
initTheme()
initI18n()
app.mount('#app')
hydrateUIFromBackend()
  .then(() => router.isReady())
  .then(() => nextTick())
  .then(() => {
    reloadEditorSettings()
    reloadQuerySettings()
    resetShortcutCache()
    refreshLocaleFromStorage()
    applyTheme(getThemeSettings().mode)
  })
  .catch(() => {})
// 「AI 服务未配置」这类请求错误：界面上已有正式引导（弹窗/消息里的「前往设置」链接），
// 但个别自动预取请求的底层 promise 残堆仍会以 uncaught 打进控制台刷屏 ——
// 这里把它降级成 debug；只匹配这一类已知错误，其它未捕获异常照常原样打印。
window.addEventListener('unhandledrejection', (e) => {
  const msg = String(e.reason?.message || e.reason || '')
  if (/status code 400|请先在「设置」|Enable and configure the AI service/.test(msg)) {
    e.preventDefault()
    console.debug('[dbmind] 该请求错误已在界面引导处理:', msg)
  }
})
