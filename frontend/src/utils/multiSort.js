/**
 * 多列排序：状态变更 + 内存比较器（三张网格共用一套口径）。
 *
 * 交互约定（**不需要按 Shift**，直接点表头的排序箭头就支持多列）：
 *   · 点一个还没排序的列 —— 追加到排序链末尾（默认 ASC）；
 *   · 点一个已排序的列 —— 在它自己的位置上循环 ASC → DESC → 移出，
 *     **其余列的顺序与方向都不动**；
 *   · 序号即优先级：A↑①、B↓② → `order by A asc, B desc`；
 *   · 只按某列排序：在底栏排序链里点掉其它列（或点「取消」全部清空）。
 *
 * 表数据预览走**服务端**排序（把 keys 拼给 /data 接口），SQL 结果与 NoSQL 文档走
 * 本文的 `makeRowComparator` 做内存排序 —— 两边都用同一份 keys 结构，切换视图不会"变味"。
 */

/** 最多几级排序：再多用户自己也记不住，且后端 ORDER BY 会很长 */
export const MAX_SORT_KEYS = 4

/** 规范化：方向只允许 ASC/DESC、列名去空、同列只留第一次、截断到 MAX_SORT_KEYS */
export const normalizeSortKeys = (keys) => {
  const out = []
  for (const k of (Array.isArray(keys) ? keys : [])) {
    const col = String((k && k.col) || '').trim()
    if (!col) continue
    if (out.some(x => x.col === col)) continue
    out.push({ col, dir: String(k.dir || '').toUpperCase() === 'DESC' ? 'DESC' : 'ASC' })
    if (out.length >= MAX_SORT_KEYS) break
  }
  return out
}

const indexOfKey = (keys, col) => keys.findIndex(k => k.col === col)

/**
 * 点排序箭头：追加 / 切换该列方向 / 移出该列；**其它列的相对顺序与方向保持不变**。
 * 典型用法：点 A（→ A↑①），点 B（→ B↑②），再点 B（→ B↓②）。
 */
export const toggleSortKey = (keys, col) => {
  const cur = normalizeSortKeys(keys)
  const i = indexOfKey(cur, col)
  if (i < 0) return normalizeSortKeys([...cur, { col, dir: 'ASC' }])
  if (cur[i].dir === 'ASC') {
    const next = cur.slice()
    next[i] = { col, dir: 'DESC' }
    return next
  }
  // 第二击 DESC 后再点 = 把这一列从排序链里摘掉（后面的列自动前移，序号不会出现空号）
  return cur.filter(k => k.col !== col)
}

/** 各列的排序序号（1 起；0 = 未参与排序） */
export const sortIndexOf = (keys, col) => normalizeSortKeys(keys).findIndex(k => k.col === col) + 1

/** 各列的方向（'' / 'ASC' / 'DESC'） */
export const sortDirOf = (keys, col) => {
  const k = normalizeSortKeys(keys).find(x => x.col === col)
  return k ? k.dir : ''
}

/**
 * 内存多键比较器（SQL 结果 / NoSQL 文档用）。
 * · NULL 永远排最后（升降序都不翻转）—— 与表预览的服务端结果习惯一致（多数库 NULLS LAST）；
 *   以前 NoSQL 那边 DESC 时 NULL 会跳到最前，看着像"空值把数据顶下去了"。
 * · 两边都能当数字（且不是空串）时按数值比，否则按 `localeCompare('zh')`（中文/数字混排更自然）。
 * · 只比较 keys 里的列；全相等返回 0（`Array#sort` 在现代引擎是稳定排序，原顺序得以保留）。
 */
export const makeRowComparator = (keys) => {
  const ks = normalizeSortKeys(keys)
  if (!ks.length) return null
  return (a, b) => {
    for (const k of ks) {
      const va = a ? a[k.col] : null
      const vb = b ? b[k.col] : null
      const an = va === null || va === undefined
      const bn = vb === null || vb === undefined
      if (an || bn) {
        if (an && bn) continue
        return an ? 1 : -1
      }
      const na = Number(va)
      const nb = Number(vb)
      const bothNum = !isNaN(na) && !isNaN(nb) && String(va).trim() !== '' && String(vb).trim() !== ''
      let c = bothNum ? na - nb : String(va).localeCompare(String(vb), 'zh')
      if (c) {
        if (k.dir === 'DESC') c = -c
        return c
      }
    }
    return 0
  }
}

/** 服务端参数：`age:ASC,name:DESC`（后端按逗号拆，方向大小写不敏感） */
export const sortKeysToParam = (keys) => normalizeSortKeys(keys)
  .map(k => `${k.col}:${k.dir}`)
  .join(',')
