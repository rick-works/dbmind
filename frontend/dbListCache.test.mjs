/* dbListCache 的直测（node 直接跑，不依赖构建） */
import {
  isValidDbListPair, readDbListCache, writeDbListCache, DB_LIST_TTL_MS,
} from './src/utils/dbListCache.js'

const mem = () => {
  const m = new Map()
  return {
    getItem: k => (m.has(k) ? m.get(k) : null),
    setItem: (k, v) => m.set(k, String(v)),
    removeItem: k => m.delete(k),
    _raw: m,
  }
}

let bad = 0
const check = (name, cond) => {
  if (!cond) bad++
  console.log((cond ? 'PASS' : 'FAIL'), name)
}

// 形态校验
check('普通方言：裸库名配套', isValidDbListPair(['demo', 'test'], []))
check('catalog 方言：全限定名配套', isValidDbListPair(['internal.ods', 'mysql_catalog.demo'], ['internal', 'mysql_catalog']))
check('修复目标：internal. 前缀但 cats 为空（脏缓存）', !isValidDbListPair(['internaloishi_dw', 'oishi_dw'], []))
check('cats 有但 list 是裸名（错位）', !isValidDbListPair(['ods'], ['internal', 'mysql_catalog']))
check('全限定名前缀不在 cats（串库）', !isValidDbListPair(['bogus.ods'], ['internal']))
check('点号后为空', !isValidDbListPair(['internal.'], ['internal']))
check('空 list', !isValidDbListPair([], []))
check('非字符串项', !isValidDbListPair(['ok', 123], []))
check('cats 非数组', !isValidDbListPair(['ods'], 'internal'))

// 读写 + 版本 + TTL
const s = mem()
check('写入合法清单', writeDbListCache(s, 'c1', ['a', 'b'], []) === true)
check('读回一致', JSON.stringify(readDbListCache(s, 'c1')) === JSON.stringify({ list: ['a', 'b'], cats: [], fresh: true }))
check('形态不配套时拒绝写入', writeDbListCache(s, 'c2', ['internal.x'], []) === false)
check('拒绝写入后读不到', readDbListCache(s, 'c2') === null)

// 老版本（v1）缓存即使形态合法也必须作废 —— 这正是内部拼错名字的来源
s.setItem('xplore.dblist.v2.c3', JSON.stringify({ ts: Date.now(), v: 1, list: ['internaloishi_dw'], cats: [] }))
check('v1 旧缓存作废', readDbListCache(s, 'c3') === null)

// 坏 JSON
s.setItem('xplore.dblist.v2.c4', '{bad')
check('坏 JSON 作废', readDbListCache(s, 'c4') === null)

// TTL
s.setItem('xplore.dblist.v2.c5', JSON.stringify({ ts: Date.now() - DB_LIST_TTL_MS - 1, v: 2, list: ['a'], cats: [] }))
check('过期缓存仍可读但 fresh=false', readDbListCache(s, 'c5')?.fresh === false)

console.log(bad ? `${bad} 个失败` : '全部通过')
process.exit(bad ? 1 : 0)
