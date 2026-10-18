/**
 * 连接 id → 连接名 的**持久记忆**。
 *
 * 为什么需要：页签 / 脚本记录里只存了连接 id。连接被删除后重新导入（或从别的机器导入
 * 同一套连接）会生成**新的 id**，于是旧页签的 id 对不上任何现存连接 —— 用户看到的却是
 * 「连接明明还在」（名字一样）。这里把见过的 id→名字留存下来，id 失效时按名字找回，
 * 避免误报「连接已不存在」。
 *
 * 只增不删：连接被删掉时它的名字也要留着，否则连"按名字找回"的线索都没了。
 */
const KEY = 'dbmind.connNames'
const MAX = 500

const read = () => {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) || '{}')
    return v && typeof v === 'object' ? v : {}
  } catch { return {} }
}

/** 用一次连接列表刷新记忆（在拿到 /api/connections 后调用） */
export const rememberConnNames = (list) => {
  if (!Array.isArray(list) || !list.length) return
  try {
    const map = read()
    let changed = false
    for (const c of list) {
      if (!c || c.id == null || !c.name) continue
      const id = String(c.id)
      if (map[id] !== c.name) { map[id] = c.name; changed = true }
    }
    if (!changed) return
    // 条目过多时按插入顺序丢老的（对象键序即插入序）
    const keys = Object.keys(map)
    if (keys.length > MAX) {
      for (const k of keys.slice(0, keys.length - MAX)) delete map[k]
    }
    localStorage.setItem(KEY, JSON.stringify(map))
  } catch { /* 隐私模式 / 配额满：只是少一条兜底线索，不影响功能 */ }
}

/** 查某个连接 id 上次见过的名字（查不到返回 ''） */
export const connNameOf = (id) => {
  if (id == null || id === '') return ''
  return read()[String(id)] || ''
}
