// ===== 模板变量（预定义变量库 + 记住的执行值）=====
// 与「执行时弹窗填参」配套：SQL 里写 :name 占位符，执行前弹窗收值（值按名称记忆）。
//
// **存哪儿：dbmind.db（app_settings 表），不再用 localStorage。**
// 原来存 localStorage 有个致命问题：桌面壳的窗口地址是 `http://127.0.0.1:<端口>`，
// 而端口每次启动都可能是另一个（默认端口被占就用下一个），localStorage 又按 origin
// （含端口）隔离 —— 换个端口打开，用户存的变量就「凭空消失」（2026-11-06 用户实测反馈）。
// 现在两个键都走通用设置接口：
//   sql.tplvars   → [{ name, value, note }]   预定义变量库（管理弹窗编辑的那份）
//   sql.varvalues → { 变量名: 上次填的值 }     执行弹窗里记住的值
// localStorage 里那两个旧键只在启动时当**迁移来源**读一次，搬成功就删掉。
import { getSettings, putAppSetting } from '../api'

export const TPLVAR_LS = 'dbmind_tplvars' // 旧存储键（仅迁移用）
export const VARVALS_LS = 'dbmind_varvalues' // 旧存储键（仅迁移用）
const KEY_TPL = 'sql.tplvars'
const KEY_VALS = 'sql.varvalues'

// 变量名：字母/下划线开头，可含数字、下划线、$（大小写都行；匹配本身大小写不敏感，
// 提取/替换正则都带 i 标志，这里只管「长得像个标识符」）
export const TPLVAR_NAME_RE = /^[A-Za-z_][\w$]*$/

// 进程内缓存：编辑器补全/装饰/执行前预填都在同步路径上读它，不能每次去等网络。
// 启动时 ensureSqlVarStore() 拉一次，之后写库顺手更新缓存。
let tplVars = []
let varValues = {}
let loaded = false
let loading = null

const parseList = (raw) => {
  try {
    const a = JSON.parse(raw || '[]')
    return Array.isArray(a) ? a.filter((v) => v && v.name) : []
  } catch { return [] }
}
const parseMap = (raw) => {
  try {
    const o = JSON.parse(raw || '{}')
    return o && typeof o === 'object' && !Array.isArray(o) ? o : {}
  } catch { return {} }
}
const legacyGet = (key) => { try { return localStorage.getItem(key) } catch { return null } }
const legacyDel = (key) => { try { localStorage.removeItem(key) } catch { /* 存储不可用就算了 */ } }

/** 把老 localStorage 里的数据搬进 dbmind.db（搬成功才删旧键，失败下次启动再试） */
async function migrateLegacy () {
  const lt = legacyGet(TPLVAR_LS)
  if (lt !== null) {
    const list = parseList(lt)
    if (!list.length) {
      legacyDel(TPLVAR_LS)
    } else {
      tplVars = list
      try { await putAppSetting(KEY_TPL, JSON.stringify(list)); legacyDel(TPLVAR_LS) } catch { /* 下次再来 */ }
    }
  }
  const lv = legacyGet(VARVALS_LS)
  if (lv !== null) {
    const map = parseMap(lv)
    if (!Object.keys(map).length) {
      legacyDel(VARVALS_LS)
    } else {
      varValues = map
      try { await putAppSetting(KEY_VALS, JSON.stringify(map)); legacyDel(VARVALS_LS) } catch { /* 下次再来 */ }
    }
  }
}

/**
 * 载入变量库与记住的值（幂等）。
 * 启动时调一次即可；后端暂时不可用（内核还在起）不会锁死，下次调用会重试。
 */
export function ensureSqlVarStore () {
  if (loaded) return Promise.resolve({ tplVars, varValues })
  if (loading) return loading
  loading = (async () => {
    try {
      const all = (await getSettings()) || {}
      const hasT = Object.prototype.hasOwnProperty.call(all, KEY_TPL)
      const hasV = Object.prototype.hasOwnProperty.call(all, KEY_VALS)
      if (hasT) tplVars = parseList(all[KEY_TPL])
      if (hasV) varValues = parseMap(all[KEY_VALS])
      loaded = true
      // 库里还没这两项：从老 localStorage 迁移（首次升级的老用户）
      if (!hasT || !hasV) await migrateLegacy()
    } catch { /* 后端未就绪：保持未 loaded，下次再试 */ } finally { loading = null }
    return { tplVars, varValues }
  })()
  return loading
}

/** 预定义变量库（同步读缓存） */
export function readTplVars () {
  return tplVars
}

/** 按变量名找预定义（大小写不敏感）；没有返回 null */
export function findTplVar (name) {
  const n = String(name || '').toLowerCase()
  return tplVars.find((v) => String(v.name).toLowerCase() === n) || null
}

/** 整表覆盖保存预定义变量库；失败会抛出（调用方给提示），缓存只在写成功后更新 */
export async function saveTplVars (list) {
  const out = (Array.isArray(list) ? list : [])
    .filter((v) => v && v.name)
    .map((v) => ({ name: String(v.name), value: String(v.value || ''), note: String(v.note || '') }))
  await putAppSetting(KEY_TPL, JSON.stringify(out))
  tplVars = out
  loaded = true
  return out
}

/** 执行弹窗里记住的填值（同步读缓存） */
export function readVarValues () {
  return varValues
}

/** 记住若干变量的填值（合并写；失败抛出给调用方，但缓存已更新，本次执行不受影响） */
export async function saveVarValues (map) {
  varValues = { ...varValues, ...(map || {}) }
  loaded = true
  await putAppSetting(KEY_VALS, JSON.stringify(varValues))
  return varValues
}

// —— 默认值的写入口径（substituteVars 同用）——
// 不带引号原样写入的三类：数据库函数（NOW()、COALESCE(x,y) 这类「名(…)」形状）、
// 常见无括号关键字（CURRENT_DATE / CURRENT_TIMESTAMP / SYSDATE…）、数字与 NULL。
const TPL_RAW_KEYWORDS = /^(current_(date|time|timestamp|user|schema|catalog)|localtime|localtimestamp|sysdate|systimestamp|today|now)$/i

/** 该值是否按「原样」写进 SQL（不加热引号）：函数、CURRENT_* 类关键字、数字、NULL */
export function isRawSqlValue (v) {
  const s = String(v == null ? '' : v).trim()
  if (!s) return false
  if (/^null$/i.test(s)) return true
  if (/^-?\d+(\.\d+)?$/.test(s)) return true
  if (TPL_RAW_KEYWORDS.test(s)) return true
  // 函数形状：NOW()、length(x)、COALESCE(a, b) —— 括号必须配平到结尾
  return /^[A-Za-z_][\w$]*\s*\(.*\)$/.test(s)
}
