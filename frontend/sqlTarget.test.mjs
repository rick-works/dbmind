/* sqlTarget 的直测：重点验证 singleTableOf 的保守口径 */
import { singleTableOf, tableSegsOf } from './src/utils/sqlTarget.js'

let bad = 0
const same = (name, got, want) => {
  if (got !== want) bad++
  console.log((got === want ? 'PASS' : 'FAIL'), name, '=>', JSON.stringify(got))
}

// 单表 SELECT：截图场景
same('select * from "DM_ORDER_SUMMARY"', singleTableOf('SELECT\n*\nFROM "DM_ORDER_SUMMARY";'), 'DM_ORDER_SUMMARY')
same('裸名带 WHERE', singleTableOf('select a from d_t where x=1'), 'd_t')
same('反引号两段取末段', singleTableOf('select * from `demo`.`t1`'), 't1')
same('方括号两段取末段', singleTableOf('select * from [dbo].[orders]'), 'orders')
same('小写 from', singleTableOf('select * from t\n-- 注释'), 't')
same('CTE 开头直接放弃', singleTableOf('with c as (select * from t) select * from c'), '')

// 多表 / 子查询 / 聚合：一律放弃（宁可不打码，也不贴错表）
same('JOIN', singleTableOf('select * from a join b on 1=1'), '')
same('逗号多表', singleTableOf('select * from a, b'), '')
same('子查询', singleTableOf('select * from (select * from t) x'), '')
same('UNION', singleTableOf('select * from a union all select * from b'), '')
same('表函数', singleTableOf('select * from f_split(1)'), '')
same('多语句', singleTableOf('select * from a; select * from b;'), '')

// 非 SELECT
same('UPDATE', singleTableOf('update t set a=1'), '')
same('空', singleTableOf(''), '')

// 原有 tableSegsOf 不回归
const segs = tableSegsOf('alter table dbo.[T] add c int')
same('tableSegsOf', segs.join('.'), 'dbo.T')

console.log(bad ? `${bad} 个失败` : '全部通过')
process.exit(bad ? 1 : 0)
