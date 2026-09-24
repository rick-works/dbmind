/**
 * 对象树的「先给缓存、后台校准」缓存（stale-while-revalidate）。
 *
 * ## 为什么需要
 *
 * 每个连接/库的**第一次**访问要付 5~10 秒（Java 宿主冷启 + JDBC 建连）。实测同一接口：
 *
 *   /databases   首次 5089 ms   第二次 86 ms
 *   /tables      首次 10074 ms  第二次 215 ms
 *
 * 而用户的操作路径是「展开连接 → 展开库 → 展开分类」，**每一步都在付这笔钱**；
 * 内核或页面一重启，内存缓存清空，又得从头付一遍 —— 体感就是"树很慢、不跟手"。
 *
 * ## 为什么不能像页签那样"存下来就一直用"
 *
 * 这些是**服务端状态**。存成权威值就会「一次存错，永远错」：新建的库看不见、
 * 后端学会了列库也看不见，用户还没有任何自愈办法（本项目踩过，所以会话快照里
 * 刻意排除了 `dbsByConn`）。这里的口径是：
 *
 *   ① 命中缓存 → **立刻**用它渲染（人不用等）；
 *   ② **同时**照常发真实请求 → 回来就合并/更新（数据永远由后端说了算）；
 *   ③ 超时的缓存不再直接用（`stale`），让调用方自己决定。
 *
 * 一句话：**缓存只负责快，不负责对。**
 *
 * ## 为什么放 localStorage 而不是 sessionStorage
 *
 * 桌面版关掉再开也要能秒开，而 TTL 保证不会拿到隔夜的东西。
 */

/**
 * 缓存键前缀，**带版本号**。
 *
 * 为什么要带版本：这里的缓存是"先渲染、后台校准"，校准失败或还没跑完时，
 * 屏幕上就是缓存里的那份内容。一旦缓存内容的**语义**变了（比如"库清单"里混进过
 * schema 名、或者键的构成改了），旧数据不会自己消失 —— 它只是一个字符串，
 * 会被继续当成合法内容渲染出来，看上去就像新代码没生效。
 *
 * 改语义时把版本 +1 即可：旧键从此不再被读取，用户不需要手动清缓存
 * （`clearSchemaCache` 也只清本版本，不会误伤）。
 *
 * v2：早期版本的 SQL Server 库清单缓存里混入过 schema 名（db_owner / dbo 等），
 *     那些键必须作废。
 */
const PREFIX = 'dbmind.schema-cache.v2.'

/** 与参考实现（dbx）同量级的 TTL：15 分钟。 */
export const SCHEMA_CACHE_TTL_MS = 15 * 60 * 1000

/**
 * 读缓存。返回 `{ value, cachedAt, stale }`，没有则 `null`。
 * 任何异常（JSON 坏掉、被禁用）都当成"没有缓存"。
 */
export function readSchemaCache(key) {
  try {
    const raw = localStorage.getItem(PREFIX + key)
    if (!raw) return null
    const parsed = JSON.parse(raw)
    if (!parsed || !('value' in parsed)) return null
    const cachedAt = parsed.cachedAt || 0
    return { value: parsed.value, cachedAt, stale: Date.now() - cachedAt > SCHEMA_CACHE_TTL_MS }
  } catch {
    return null
  }
}

/**
 * 写缓存。写不进去（配额满、隐私模式）就静默放弃 ——
 * 缓存只是加速手段，绝不能因为它把正常功能弄坏。
 */
export function writeSchemaCache(key, value) {
  const payload = JSON.stringify({ value, cachedAt: Date.now() })
  try {
    localStorage.setItem(PREFIX + key, payload)
  } catch {
    // 大概率是配额满：清掉本模块自己的缓存腾地方，再试一次
    clearSchemaCache()
    try { localStorage.setItem(PREFIX + key, payload) } catch { /* 还是不行就算了 */ }
  }
}

/**
 * 删掉单个键。
 *
 * 用于「刷新某一类」：那次请求拿到的就是最新的，旧值必须丢掉 ——
 * 否则下次展开又会先渲染一次旧的（计数已经变了、清单里却找不到新建的表）。
 */
export function removeSchemaCache(key) {
  try {
    localStorage.removeItem(PREFIX + key)
  } catch { /* ignore */ }
}

/** 清掉本模块写的全部缓存（不影响其它 localStorage 内容）。 */
export function clearSchemaCache() {
  try {
    const keys = []
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i)
      if (k && k.startsWith(PREFIX)) keys.push(k)
    }
    keys.forEach(k => localStorage.removeItem(k))
  } catch { /* ignore */ }
}

/** 连接结构变了（新建/删除库、刷新对象等）→ 清掉该连接相关的缓存 */
export function invalidateSchemaCache(connId) {
  try {
    const suffix = ':' + connId
    const keys = []
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i)
      if (k && k.startsWith(PREFIX) && k.includes(suffix)) keys.push(k)
    }
    keys.forEach(k => localStorage.removeItem(k))
  } catch { /* ignore */ }
}
