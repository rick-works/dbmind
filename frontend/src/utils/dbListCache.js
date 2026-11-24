/**
 * SQL 编辑器「数据库下拉」的会话缓存：带一致性校验 + 版本号。
 *
 * ## 为什么要校验
 *
 * 这个缓存里同时存了 `list`（库名，catalog 方言下是 `catalog.库` 全限定名）和
 * `cats`（catalog 清单）。两者必须配套：
 *
 * - `cats` 为空 ⇒ `list` 必须是裸库名；
 * - `cats` 非空 ⇒ `list` 每一项都得是 `cats` 里某个 catalog 前缀的全限定名。
 *
 * 一旦错位，下拉会把全限定名当成裸名直接渲染 —— 真机上就出现过
 * `internaloishi_dw` 这种串了前缀、又丢了点号的怪名字，而且和左侧树
 * （走 `schemaCache.js` 的 stale-while-revalidate）长期对不上。
 *
 * ## 版本号
 *
 * 缓存内容语义变了（v1 无校验，v2 要求 list/cats 配套）之后，旧键必须自然作废，
 * 不能靠用户手动清缓存。与 `schemaCache.js` 的 `v3.` 前缀是同一个套路。
 *
 * ## 口径
 *
 * 缓存只负责「秒开」，不负责对 —— 命中后先渲染，但调用方仍要发校准请求。
 */
export const DB_LIST_CACHE_VERSION = 2
const PREFIX = 'xplore.dblist.v2.'
export const DB_LIST_TTL_MS = 60 * 1000

/** list/cats 是否配套。纯函数，便于单测。 */
export function isValidDbListPair(list, cats) {
  if (!Array.isArray(list) || !list.length) return false
  if (!list.every(x => typeof x === 'string' && x.trim())) return false
  if (!Array.isArray(cats) || !cats.every(x => typeof x === 'string' && x.trim())) return false

  if (cats.length) {
    const set = new Set(cats)
    for (const full of list) {
      const i = full.indexOf('.')
      // 全限定名必须「前缀是已知 catalog」且「点号后还有库名」
      if (i <= 0 || !set.has(full.slice(0, i)) || !full.slice(i + 1)) return false
    }
  } else {
    // 单下拉形态不允许残留 catalog 前缀（internal 是 Doris 默认目录名）。
    // 脏值有两种形态：`internal.x`（带点）与 `internalx`（拼接时点号被吃掉，
    // 真机截图里就是 `internaloishi_dw`）。后者用「internal 紧跟字母」判定，
    // 保留 `internal_` / 单独 `internal` 这类可能的真实库名。
    for (const name of list) {
      const lower = name.toLowerCase()
      if (lower.startsWith('internal.') || /^internal(?![._]|$)/.test(lower)) return false
    }
  }
  return true
}

/** 读缓存。坏数据/旧版本/形态不配套一律当没有。 */
export function readDbListCache(storage, key, now = Date.now()) {
  if (!storage) return null
  try {
    const raw = storage.getItem(PREFIX + key)
    if (!raw) return null
    const it = JSON.parse(raw)
    if (!it || it.v !== DB_LIST_CACHE_VERSION) return null
    if (!isValidDbListPair(it.list, it.cats)) return null
    return { list: it.list, cats: it.cats, fresh: now - (it.ts || 0) < DB_LIST_TTL_MS }
  } catch {
    return null
  }
}

/** 写缓存。形态不配套时不写 —— 宁可没有缓存，也不要下次渲染出错的名字。 */
export function writeDbListCache(storage, key, list, cats, now = Date.now()) {
  if (!storage || !isValidDbListPair(list, cats)) return false
  try {
    storage.setItem(PREFIX + key, JSON.stringify({ v: DB_LIST_CACHE_VERSION, ts: now, list, cats }))
    return true
  } catch {
    return false
  }
}
