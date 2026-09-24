/**
 * 「最近使用的连接」——存 id 列表，最多 5 条。
 *
 * 为什么只存 id：连接本身随时可能被改名/改地址，存快照会显示过期信息；
 * 只记顺序，每次展示时与当前连接列表求交，失效的 id 自然被丢掉。
 */

const KEY = 'dbmind.recent-connection-ids'
export const MAX_RECENT = 5

/** 读出来（解析失败 / 类型不对一律当空） */
export function readRecentConnectionIds() {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    const out = []
    const seen = new Set()
    for (const item of parsed) {
      if (typeof item !== 'string') continue
      const id = item.trim()
      if (!id || seen.has(id)) continue
      seen.add(id)
      out.push(id)
      if (out.length >= MAX_RECENT) break
    }
    return out
  } catch (e) {
    return []
  }
}

/** 记一次使用：新用的排最前，去重、截断到 5 条 */
export function recordRecentConnection(connectionId) {
  const id = String(connectionId || '').trim()
  if (!id) return readRecentConnectionIds()
  const next = [id].concat(readRecentConnectionIds().filter(x => x !== id)).slice(0, MAX_RECENT)
  try {
    localStorage.setItem(KEY, JSON.stringify(next))
  } catch (e) { /* 隐私模式 / 配额满：记不住就算了，不影响主流程 */ }
  return next
}

/** 从当前连接列表里挑出最近用过的（按记录顺序），id 已失效的自动跳过 */
export function rankRecentConnections(connections, limit = MAX_RECENT) {
  const byId = new Map((connections || []).map(c => [String(c.id), c]))
  const out = []
  for (const id of readRecentConnectionIds()) {
    const conn = byId.get(id)
    if (conn) out.push(conn)
    if (out.length >= limit) break
  }
  return out
}
