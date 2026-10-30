/**
 * SQL 脚本的本地存储读写（「脚本管理」弹窗 / 左侧树脚本节点 / 编辑器保存共用一份口径）。
 *
 * 存储：`localStorage['dc_scripts:<connId>:<db>']`，值是 `[{ id, name, content, time }]`。
 *
 * 为什么要单独一层：脚本的读、写、改名、删除原先分散在编辑器（保存）、脚本管理弹窗（删除）、
 * 左侧树（删除）三处各写一遍，key 的拼法、同名判定、变更通知稍有出入就会「一处改了另一处看不见」。
 * 这里只做存储本身，**不碰 UI**（提示语与确认框留在调用方）。
 */

export const SCRIPT_PREFIX = 'dc_scripts:'

/// 存储键：`<连接 id>:<库>`；有模式层级的库传 `库.模式`（与编辑器保存时一致）
export const scriptKey = (connId, db) => `${SCRIPT_PREFIX}${connId}:${db}`

/// 读某个连接某个库下的脚本清单（读不出来就是空数组，不抛）
export const readScripts = (connId, db) => {
  try {
    const raw = localStorage.getItem(scriptKey(connId, db))
    const list = raw ? JSON.parse(raw) : []
    return Array.isArray(list) ? list.filter(Boolean) : []
  } catch { return [] }
}

export const writeScripts = (connId, db, list) => {
  try {
    localStorage.setItem(scriptKey(connId, db), JSON.stringify(Array.isArray(list) ? list : []))
    return true
  } catch { return false }
}

/// 同一连接同一库下是否已有同名脚本（`exceptId` = 改名时排除自己）
export const hasSameName = (connId, db, name, exceptId) =>
  readScripts(connId, db).some(s => s.id !== exceptId && String(s.name || '') === String(name))

/**
 * 重命名。返回值：
 * - `'ok'`   改成功（`time` 保持不动 —— 它表示脚本**内容**的更新时间，改名不该把它顶到最前）
 * - `'empty'` 新名字是空串
 * - `'dup'`  同一库下已有同名脚本
 * - `'gone'` 脚本已经不在了（另一处刚删掉）
 */
export const renameScript = (connId, db, scriptId, nextName) => {
  const name = String(nextName || '').trim()
  if (!name) return 'empty'
  if (hasSameName(connId, db, name, scriptId)) return 'dup'
  const list = readScripts(connId, db)
  if (!list.some(s => s.id === scriptId)) return 'gone'
  writeScripts(connId, db, list.map(s => (s.id === scriptId ? { ...s, name } : s)))
  return 'ok'
}

/// 删除；脚本已经不在了也返回 true（幂等，批量删除时不会因为一条没了就中断）
export const removeScript = (connId, db, scriptId) => {
  const list = readScripts(connId, db)
  writeScripts(connId, db, list.filter(s => s.id !== scriptId))
  return true
}

/// 通知树 / 编辑器刷新（MainView 与 SqlQueryView 都在监听）
export const notifyScriptsChanged = (connId, db) => {
  try {
    window.dispatchEvent(new CustomEvent('dc-scripts-changed', { detail: { connId, db } }))
  } catch { /* 非浏览器环境忽略 */ }
}
