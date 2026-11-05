// ===== 模板变量（预定义变量库）=====
// 与「执行时弹窗填参」配套：SQL 里写 :name 占位符，执行前弹窗收值（值按名称记忆）。
// 这里是**预定义**那半边：变量名 → 默认值/备注，存 localStorage（与片段、记住的执行值
// 同风格，纯前端零后端）。执行弹窗里没记住过的变量用默认值预填；编辑器里打 : 也按这份
// 名单弹补全。管理入口 = SQL 编辑器工具条「变量」按钮（TplVarDialog）。
export const TPLVAR_LS = 'dbmind_tplvars'
// 官网文档口径：小写字母/下划线开头，可含数字、下划线（与执行链路 extractSqlVars 的
// 大小写不敏感提取一致 —— 这里统一落小写，避免 :Name 与 :name 存成两份）
export const TPLVAR_NAME_RE = /^[a-z_][a-z0-9_$]*$/

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
  return readTplVars().find((v) => v.name === n) || null
}
