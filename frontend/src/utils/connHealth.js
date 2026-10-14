/**
 * 连接健康记忆：记下「最近连不上的连接」，一段时间内不再去预热它。
 *
 * <p>为什么需要：启动时的后台预热是**并行**打几条连接的，而一条连不上的库很贵 ——
 * 实测一条已下线的 Doris 要等满一次建连超时（10 秒），而这段等待不只是它自己慢：
 * 同一个代理进程里别的请求会被排在后面一起拖住（实测：那条死连接把 Oracle 的首次
 * 展开从 0.3 秒拖到 6.4 秒）。所以连不上的连接要**记住**，别每次启动都拿别人陪它等。
 *
 * <p>只影响预热，不影响使用：用户真点开它时该连就照连、该报错就照报错。
 * 过期时间取 30 分钟 —— 库修好了（或被掐掉的会话恢复了）不至于一直躲着它，
 * 而一个真的下线的库也不会每次启动都拖一遍。
 */

const KEY = 'dbmind.prewarm-unreachable'
/** 记忆时长：超过它就允许再试一次 */
export const UNREACHABLE_TTL_MS = 30 * 60 * 1000

/** 读出还“在期”的失败记录（解析失败 / 结构不对一律当空） */
export function readUnreachableIds() {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object') return []
    const now = Date.now()
    const out = []
    for (const [id, at] of Object.entries(parsed)) {
      const when = Number(at)
      if (!id || !Number.isFinite(when) || now - when > UNREACHABLE_TTL_MS) continue
      out.push(id)
    }
    return out
  } catch (e) {
    return []
  }
}

/** 清理过期项后写回（顺手把 30 分钟前的记录丢掉，别让这个键无限长大） */
function writeIds(ids) {
  try {
    const now = Date.now()
    const next = {}
    for (const id of ids) next[id] = now
    localStorage.setItem(KEY, JSON.stringify(next))
  } catch (e) { /* 隐私模式 / 配额满：记不住就算了，只是可能多等一次 */ }
}

/** 记住「这条连不上」 */
export function markUnreachable(connectionId) {
  const id = String(connectionId || '').trim()
  if (!id) return
  writeIds(readUnreachableIds().concat(id))
}

/** 连上了：把之前的失败记录撤掉 */
export function clearUnreachable(connectionId) {
  const id = String(connectionId || '').trim()
  if (!id) return
  writeIds(readUnreachableIds().filter(x => x !== id))
}
