// ===== 模板变量（预定义变量库）=====
// 与「执行时弹窗填参」配套：SQL 里写 :name 占位符，执行前弹窗收值（值按名称记忆）。
// 这里是**预定义**那半边：变量名 → 默认值/备注，存 localStorage（与片段、记住的执行值
// 同风格，纯前端零后端）。执行弹窗里没记住过的变量用默认值预填；编辑器里打 : 也按这份
// 名单弹补全。管理入口 = SQL 编辑器工具条「变量」按钮（TplVarDialog）。
export const TPLVAR_LS = 'dbmind_tplvars'
// 变量名：字母/下划线开头，可含数字、下划线、$（大小写都行；匹配本身大小写不敏感，
// 提取/替换正则都带 i 标志，这里只管「长得像个标识符」）
export const TPLVAR_NAME_RE = /^[A-Za-z_][\w$]*$/

export function readTplVars () {
  try {
    const a = JSON.parse(localStorage.getItem(TPLVAR_LS) || '[]')
    return Array.isArray(a) ? a.filter((v) => v && v.name) : []
  } catch { return [] }
}

export function writeTplVars (list) {
  try { localStorage.setItem(TPLVAR_LS, JSON.stringify(list || [])) } catch { /* 存储不可用就算了 */ }
}

/** 按变量名找预定义（大小写不敏感）；没有返回 null */
export function findTplVar (name) {
  const n = String(name || '').toLowerCase()
  return readTplVars().find((v) => String(v.name).toLowerCase() === n) || null
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
