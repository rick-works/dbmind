// 应用本地设置（与设置页默认值保持唯一来源）
// 说明：编辑器设置、查询设置、通知设置保存在 localStorage；
// 驱动镜像配置由后端持久化（settings.json），前端通过 API 读写。
import { ref } from 'vue'

export const editorDefaults = {
  fontSize: 14, tabSize: 4, wordWrap: true, lineNumbers: true, autoSave: false,
  autoCloseBrackets: true,  // 括号/引号自动补齐（SQL 语言定义没有 autoClosingPairs，需强制 always）
  quickSuggest: true,       // 智能补全：输入时自动弹出表名列名/函数建议
  gridFontSize: 14,  // 结果表格字号（与 SQL 编辑器独立）
  // SQL 格式化全局默认：关键字排版规则之外的统一外观（规则未覆盖的关键字/大小写均取此处默认）。
  // 方言不在此配置——查询编辑器按连接类型自动识别，设置预览固定用标准 SQL。
  sqlKeywordCase: 'preserve',       // 关键字大小写：preserve=保持原样（默认，不动用户写法）| upper=大写 | lower=小写
  sqlIdentifierCase: 'preserve',    // 标识符大小写
  sqlDataTypeCase: 'preserve',      // 数据类型大小写
  sqlFunctionCase: 'preserve',      // 函数名大小写
  sqlTabWidth: 2,                   // 缩进宽度（空格个数）
  sqlUseTabs: false,                // 是否用 Tab 缩进（开启后忽略缩进宽度）
  sqlExpressionWidth: 50,           // 表达式超过该宽度自动折行
  sqlLinesBetweenQueries: 2,        // 多条语句之间的空行数
  sqlLogicalOperatorNewline: 'before', // 逻辑运算符 AND/OR 换行位置：before=前换行 | after=后换行
  sqlNewlineBeforeSemicolon: false, // 分号是否独占一行
  sqlDenseOperators: false,         // 运算符两侧是否不加空格（a=b+1）
  // 关键字排版规则：每条 { kw: 关键字/短语, cs: upper=大写 | lower=小写 | keep=保持原样, ly: none=内联 | clause=前换行 | join=前换行+缩进 | afterBreak=后换行 | afterBreakIndent=后换行+缩进 | setop=前后各换行 }
  // 不配置任何规则时，所有关键字均按上述全局默认排版
  sqlKeywordRules: []
}
// timeoutSecs / cacheTtlSecs / maxRows / historyMax / historyDays 存**后端**（app_settings），
// 默认值与后端种子一致 —— 界面在拿到后端值之前先用它们占位，避免表单闪一下空值
export const queryDefaults = {
  pageSize: 200, confirmDanger: true,
  timeoutSecs: 120, cacheTtlSecs: 300,
  maxRows: 2000, historyMax: 1000, historyDays: 30,
  nullStyle: 'null'  // NULL 单元格显示：'null'=NULL | 'paren'=(NULL) | 'blank'=空白
}
export const notifyDefaults = { success: true, autoClose: true }

const readJSON = (key) => {
  try {
    const raw = localStorage.getItem(key)
    return raw ? { ...JSON.parse(raw) } : {}
  } catch { return {} }
}

// v2 规则 mode → v3 排版 ly 的映射（mode 语义：所有关键字统一按关键字处理即大写）
const LEGACY_MODE_LY = { kw: 'none', clause: 'clause', join: 'join', setop: 'setop' }
const LY_VALUES = ['none', 'clause', 'join', 'afterBreak', 'afterBreakIndent', 'setop']

const normalizeRules = (rules) => {
  const seen = new Set()
  return (Array.isArray(rules) ? rules : []).reduce((out, r) => {
    if (!r || typeof r !== 'object') return out
    const kw = String(r.kw || '').trim()
    if (!kw) return out
    const key = kw.toUpperCase()
    if (seen.has(key)) return out
    seen.add(key)
    const ly = LY_VALUES.includes(r.ly) ? r.ly : (LEGACY_MODE_LY[r.mode] || 'none')
    out.push({ kw, cs: r.cs === 'keep' ? 'keep' : (r.cs === 'lower' ? 'lower' : 'upper'), ly })
    return out
  }, [])
}

// 迁移：把历史缓存就地整理为 v3 结构（sqlKeywordRules 逐条归一化 + 清理已下线的 sqlLanguage 残留）
export const migrateEditor = (raw) => {
  if (!raw || typeof raw !== 'object') return raw
  let changed = false
  // v1 遗留：一个关键字串 + 一种排版方式 → 展开成规则数组
  const hasLegacy = raw.sqlCustomKeyword !== undefined || raw.sqlCustomKeywordMode !== undefined
  if (hasLegacy) {
    const mode = ['join', 'setop'].includes(raw.sqlCustomKeywordMode)
      ? raw.sqlCustomKeywordMode
      : (raw.sqlCustomKeywordMode === 'kw' ? 'kw' : 'clause')
    const words = String(raw.sqlCustomKeyword || '')
      .split(/[,，;；\n\r]+/)
      .map((s) => s.trim())
      .filter(Boolean)
    if (words.length) {
      raw.sqlKeywordRules = words.map((kw) => ({ kw, cs: 'upper', ly: LEGACY_MODE_LY[mode] }))
    }
    delete raw.sqlCustomKeyword
    delete raw.sqlCustomKeywordMode
    changed = true
  }
  // v2/v3 规则统一归一化
  if (raw.sqlKeywordRules !== undefined) {
    const before = JSON.stringify(raw.sqlKeywordRules)
    raw.sqlKeywordRules = normalizeRules(raw.sqlKeywordRules)
    if (before !== JSON.stringify(raw.sqlKeywordRules)) changed = true
  }
  // 旧全局「SQL 方言」已下线：格式化方言改由连接类型自动识别，清理旧缓存中的残留值
  if ('sqlLanguage' in raw) { delete raw.sqlLanguage; changed = true }
  // 旧默认值一次性迁移（字号 12 → 14、关键字大小写 upper → preserve）：
  // **必须只跑一次** —— 以前没有门槛，每次读设置都重跑，用户把关键字改成「大写」
  // 或字号改成 12，保存的瞬间就被这条迁回旧默认（设置「保存后又变回去」的真凶）。
  // 门槛标志落盘后，用户改成的任何值（包括恰好等于旧默认的值）都不再被碰。
  if (!raw._defaultsMigrated) {
    if (raw.fontSize === 12) { raw.fontSize = 14; changed = true }
    if (raw.gridFontSize === 13) { raw.gridFontSize = 14; changed = true }
    if (raw.sqlKeywordCase === 'upper') { raw.sqlKeywordCase = 'preserve'; changed = true }
    raw._defaultsMigrated = true
    changed = true
  }
  if (changed) { try { localStorage.setItem('dbmind_editor', JSON.stringify(raw)) } catch { /* 忽略写失败 */ } }
  return raw
}

export const getEditorSettings = () => {
  const stored = readJSON('dbmind_editor')
  migrateEditor(stored)
  return { ...editorDefaults, ...stored }
}

// 编辑器设置的**共享响应式快照**：两个 Monaco 宿主（SqlQueryView / SqlCodeEditor）读这份
// ref，设置页保存后调 `reloadEditorSettings()` 刷新 —— 选项改动即刻生效，不必重开标签页
// （vue-monaco-editor 的 wrapper 会 watch options 并调 `editor.updateOptions`）。
// 之前各组件在 setup 时各自快照一次，改完设置对已打开的编辑器毫无作用。
export const editorSettingsLive = ref(getEditorSettings())
export const reloadEditorSettings = () => { refreshLive(editorSettingsLive, getEditorSettings()) }

// 共享快照**原地刷新**：保持对象身份，只逐键改值。
// 真机踩过 —— 整体替换 `.value` 会让 v-memo 网格在启动水合后的重补丁里拿到
// 空容器，报「insertBefore/minimap of null」把工作区打挂；原地改走细粒度依赖，
// 只有真正读到变更键的组件会重渲染。
const refreshLive = (live, next) => {
  const cur = live.value
  if (!cur || typeof cur !== 'object') { live.value = next; return }
  Object.keys(next).forEach((k) => {
    const nv = next[k]
    const ov = cur[k]
    if (ov === nv) return
    // 数组/对象等值也跳过：新引用会无谓触发重渲染（rules 数组每次都是新建的）
    if (nv && ov && typeof nv === 'object' && typeof ov === 'object' && JSON.stringify(nv) === JSON.stringify(ov)) return
    cur[k] = nv
  })
}

export const getQuerySettings = () => ({ ...queryDefaults, ...readJSON('dbmind_query') })
export const getNotifySettings = () => ({ ...notifyDefaults, ...readJSON('dbmind_notify') })

// ---------- UI 设置持久化（真身在 dbmind.db 的 app_settings 表） ----------
//
// localStorage 从「真身」降级为**同步缓存**：改动先落本地（同步、界面即时生效），
// 防抖 600ms 推后端；启动时 hydrateUIFromBackend() 从后端水合覆盖本地 ——
// 换浏览器 / 清站点数据 / 换机器都不丢设置。
// 后端还没有对应键时（首次升级），把本地现有值推上去，老配置不丢。
//
// 动态 import 引 api：本模块被 i18n / theme / shortcuts 静态引用，
// 静态引 api 会形成 i18n → settings → api → i18n 循环。
const UI_BACKEND_KEYS = {
  'ui.editor': 'dbmind_editor',
  'ui.query': 'dbmind_query',
  'ui.notify': 'dbmind_notify',
  'ui.theme': 'dbmind_theme',
  'ui.locale': 'dbmind_locale',
  'ui.shortcuts': 'dbmind_shortcuts',
  'ui.topmenu': 'dbmind_topmenu'
}
const pushTimers = {}
/** 落一层 UI 设置：`raw` 是**原样**写进 localStorage 的字符串（调用方自己序列化）。 */
export const persistUI = (lsKey, raw) => {
  try { localStorage.setItem(lsKey, raw) } catch { /* 隐私模式等写失败忽略 */ }
  const bk = Object.keys(UI_BACKEND_KEYS).find((k) => UI_BACKEND_KEYS[k] === lsKey)
  if (!bk) return
  clearTimeout(pushTimers[bk])
  pushTimers[bk] = setTimeout(() => {
    import('../api')
      .then(({ putSetting }) => putSetting(bk, raw).catch(() => {}))
      .catch(() => {})
  }, 600)
}
/** 启动时调用：后端有值 → 覆盖本地缓存；后端没有 → 把本地值推上去（首次升级迁移）。 */
export const hydrateUIFromBackend = async () => {
  let s = null
  try {
    const api = await import('../api')
    s = await api.getSettings()
  } catch { return } // 后端不可达：沿用本地缓存，改动时再同步
  for (const [bk, lsKey] of Object.entries(UI_BACKEND_KEYS)) {
    const raw = s?.[bk]
    if (typeof raw === 'string' && raw) {
      try { if (localStorage.getItem(lsKey) !== raw) localStorage.setItem(lsKey, raw) } catch { /* 忽略 */ }
    } else {
      const cur = localStorage.getItem(lsKey)
      if (cur) {
        import('../api')
          .then(({ putSetting }) => putSetting(bk, cur).catch(() => {}))
          .catch(() => {})
      }
    }
  }
  // 通知各消费方「后端的值已经落进本地缓存」——它们在模块初始化时读的可能是
  // 水合前的旧缓存（如顶栏菜单配置在 MainView setup 里加载），此刻应重读一次
  try { window.dispatchEvent(new Event('dbmind-ui-hydrated')) } catch { /* 非浏览器环境忽略 */ }
}

// 查询设置的**共享响应式快照**：NULL 显示样式在网格渲染里读这份 ref，
// 设置页改完（配合网格 v-memo 的依赖项）无需重跑查询即可生效。
export const querySettingsLive = ref(getQuerySettings())
export const reloadQuerySettings = () => { refreshLive(querySettingsLive, getQuerySettings()) }
