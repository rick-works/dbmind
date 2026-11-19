/**
 * AI 面板「底栏已选状态」与「当前对话指针」的持久化（**跨刷新、跨重启**）。
 *
 * <p>为什么不用 localStorage：桌面壳的窗口地址是 `http://127.0.0.1:<端口>`，端口每次启动
 * 都可能落到另一个（默认端口被占就用下一个），而 localStorage 按 origin（含端口）隔离
 * —— 换个端口打开，用户选好的连接/库/聚焦表就「凭空消失」（SQL 模板变量踩过同一个坑）。
 * 所以和模板变量一样落 dbmind.db 的 app_settings：
 *   ai.composer.<mode> → { connId, database, schema, tables, modelId, agentMode, kbScope, kbPicked }
 *   ai.session.<mode>  → { tab, chatThreadId, askThreadId }
 *
 * <p>`<mode>` 是 `studio`（中央工作区）/ `panel`（右侧窄面板）：两个实例可能同时挂着
 * （中央页签 + 常驻窄面板），共用一个键会互相覆盖，所以各记各的。
 *
 * <p>读：`ensureAiPanelPrefs()` 面板挂载时拉一次，之后同步读缓存（渲染路径不能等网络）；
 * 写：`saveAiComposer / saveAiSession` 先更新缓存，再合并成一次 PUT（勾表、切库这种
 * 连续操作只落一次盘）。
 */
import { getSettings, putAppSetting } from '../../api'

const KEY_CTX = 'ai.composer'
const KEY_SESS = 'ai.session'
const MODES = ['studio', 'panel']
/** 写库合并窗口：底栏连续操作（勾表、切库）只打一次请求 */
const FLUSH_DELAY = 400

const emptyComposer = () => ({
  connId: '',
  database: '',
  schema: '',
  tables: [],
  modelId: '',
  agentMode: false,
  kbScope: 'none',
  kbPicked: []
})

const emptySession = () => ({ tab: 'chat', chatThreadId: '', askThreadId: '' })

const str = (v) => (typeof v === 'string' ? v : '')
const strArr = (v) => (Array.isArray(v) ? v.filter((x) => typeof x === 'string' && x) : [])

const parseComposer = (raw) => {
  const base = emptyComposer()
  if (!raw) return base
  let obj = raw
  if (typeof raw === 'string') {
    try { obj = JSON.parse(raw) } catch { return base }
  }
  if (!obj || typeof obj !== 'object' || Array.isArray(obj)) return base
  return {
    connId: str(obj.connId),
    database: str(obj.database),
    schema: str(obj.schema),
    tables: strArr(obj.tables),
    modelId: str(obj.modelId),
    agentMode: obj.agentMode === true,
    kbScope: ['all', 'none', 'custom'].includes(obj.kbScope) ? obj.kbScope : 'none',
    kbPicked: strArr(obj.kbPicked)
  }
}

const parseSession = (raw) => {
  const base = emptySession()
  if (!raw) return base
  let obj = raw
  if (typeof raw === 'string') {
    try { obj = JSON.parse(raw) } catch { return base }
  }
  if (!obj || typeof obj !== 'object' || Array.isArray(obj)) return base
  return {
    tab: str(obj.tab) || 'chat',
    chatThreadId: str(obj.chatThreadId),
    askThreadId: str(obj.askThreadId)
  }
}

// 进程内缓存：面板渲染/挂载时同步读它，不能每次去等网络
const cache = { studio: emptyComposer(), panel: emptyComposer() }
const session = { studio: emptySession(), panel: emptySession() }

let loaded = false
let loading = null

/** 拉一次库里的值（幂等）；后端未就绪时不锁死，下次调用还会重试 */
export function ensureAiPanelPrefs () {
  if (loaded) return Promise.resolve(true)
  if (loading) return loading
  loading = (async () => {
    try {
      const all = (await getSettings()) || {}
      MODES.forEach((mode) => {
        cache[mode] = parseComposer(all[`${KEY_CTX}.${mode}`])
        session[mode] = parseSession(all[`${KEY_SESS}.${mode}`])
      })
      loaded = true
    } catch { /* 后端未就绪：保持未 loaded，下次再试 */ } finally { loading = null }
    return loaded
  })()
  return loading
}

/** 是否已经从库里读到过（没读到就别回写，免得把库里的值用空值盖掉） */
export const aiPanelPrefsReady = () => loaded

export const readAiComposer = (mode = 'studio') => cache[mode] || emptyComposer()
export const readAiSession = (mode = 'studio') => session[mode] || emptySession()

const timers = new Map()
const schedule = (key, value) => {
  const prev = timers.get(key)
  if (prev) clearTimeout(prev)
  timers.set(key, setTimeout(() => {
    timers.delete(key)
    putAppSetting(key, JSON.stringify(value)).catch(() => { /* 设置写不进去不影响当前使用 */ })
  }, FLUSH_DELAY))
}

/** 合并保存底栏状态（缓存立即生效，落库合并成一次 PUT） */
export function saveAiComposer (mode, patch) {
  const key = `${KEY_CTX}.${mode === 'panel' ? 'panel' : 'studio'}`
  const next = { ...readAiComposer(mode), ...(patch || {}) }
  cache[mode === 'panel' ? 'panel' : 'studio'] = next
  if (loaded) schedule(key, next)
}

/** 记下「现在打开的是哪一段对话」：刷新后按这个指针接回来 */
export function saveAiSession (mode, patch) {
  const m = mode === 'panel' ? 'panel' : 'studio'
  const next = { ...readAiSession(m), ...(patch || {}) }
  session[m] = next
  if (loaded) schedule(`${KEY_SESS}.${m}`, next)
}
