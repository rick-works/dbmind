/* 脱敏集成：SQL 解析表名 + 规则命中（复现「双击脱敏、SQL 查询明文」的修复）
 *
 * masking.js 顶部 `import { getMaskRules } from '../api'` 是给浏览器用的目录导入，
 * node ESM 不支持 —— 这里在临时副本里换成 stub 后动态加载，测的还是同一份源码。
 */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const src = path.join(HERE, 'src/utils/masking.js')
let body = fs.readFileSync(src, 'utf8')
body = body.replace("import { getMaskRules } from '../api'", 'const getMaskRules = async () => null')
const tmp = path.join(HERE, '_masking_tmp.mjs')
fs.writeFileSync(tmp, body, 'utf8')
const { setMaskConfig, gridMethods } = await import('file:///' + tmp.replace(/\\/g, '/'))
const { singleTableOf } = await import('file:///' + path.join(HERE, 'src/utils/sqlTarget.js').replace(/\\/g, '/'))

const cfg = {
  llm: false, mcp: false, grid: true, export: false,
  rules: [
    { connectionId: 'conn_18e739e16a6a92b4001c', database: 'demo', schema: null,
      table: 'DM_ORDER_SUMMERY', column: 'MANDT', method: 'md5', note: null }
  ]
}
setMaskConfig(cfg)

const table = singleTableOf('SELECT\n*\nFROM "DM_ORDER_SUMMERY";')
const cols = gridMethods({
  connectionId: 'conn_18e739e16a6a92b4001c', database: 'demo',
  table, columns: ['MANDT', 'VBELN', 'POSNR']
}, cfg)

let bad = 0
const check = (name, cond) => { if (!cond) bad++; console.log(cond ? 'PASS' : 'FAIL', name) }
check('解析出表名 DM_ORDER_SUMMERY', table === 'DM_ORDER_SUMMERY')
check('MANDT 命中 md5', cols.MANDT === 'md5')
check('其它列不脱敏', cols.VBELN === undefined && cols.POSNR === undefined)

// 拼错表名（或 JOIN 解析不出表）时不命中 —— 宁可不打码，也不贴错表
const wrong = singleTableOf('SELECT * FROM "DM_ORDER_SUMMARY";')
check('拼错表名不误命中', gridMethods({
  connectionId: 'conn_18e739e16a6a92b4001c', database: 'demo', table: wrong, columns: ['MANDT']
}, cfg).MANDT === undefined)

fs.unlinkSync(tmp)
console.log(bad ? `${bad} 个失败` : '全部通过')
process.exit(bad ? 1 : 0)
