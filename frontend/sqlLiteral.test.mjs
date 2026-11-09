import { classifyColType, buildLiteral, inferKind, replaceVars, varContextOf } from './src/utils/sqlLiteral.js'

let pass = 0, fail = 0
const eq = (got, want, label) => {
  if (got === want) { pass++; console.log('  ok', label) }
  else { fail++; console.log('  FAIL', label, '\n    got :', JSON.stringify(got), '\n    want:', JSON.stringify(want)) }
}

console.log('== classifyColType ==')
eq(classifyColType('Nullable(String)'), 'string', 'CH Nullable(String) -> string')
eq(classifyColType('LowCardinality(String)'), 'string', 'CH LowCardinality(String)')
eq(classifyColType('UInt32'), 'number', 'CH UInt32 -> number')
eq(classifyColType('Int64'), 'number', 'CH Int64')
eq(classifyColType('Decimal(10, 2)'), 'number', 'CH Decimal')
eq(classifyColType('Float64'), 'number', 'CH Float64')
eq(classifyColType('Bool'), 'bool', 'CH Bool')
eq(classifyColType('varchar(50)'), 'string', 'MySQL varchar')
eq(classifyColType('int unsigned'), 'number', 'MySQL int unsigned')
eq(classifyColType('bigint'), 'number', 'bigint')
eq(classifyColType('DECIMAL(10,2)'), 'number', 'SQL DECIMAL')
eq(classifyColType('interval'), 'string', 'PG interval 不是数字（int 前缀陷阱）')
eq(classifyColType('character varying(64)'), 'string', 'PG varchar')
eq(classifyColType('timestamp'), 'string', 'timestamp -> string 字面量')
eq(classifyColType('DateTime64(3)'), 'string', 'CH DateTime64')
eq(classifyColType('UUID'), 'string', 'UUID')
eq(classifyColType('int2'), 'number', 'PG int2')
eq(classifyColType(''), 'string', '空 -> string')

console.log('== buildLiteral ==')
eq(buildLiteral('1110057272', 'string', 'clickhouse'), "'1110057272'", '数字 -> CH 字符串字面量')
eq(buildLiteral('1110057272', 'auto', ''), '1110057272', 'auto 数字原样（旧行为）')
eq(buildLiteral('abc', 'auto', ''), "'abc'", 'auto 字符串')
eq(buildLiteral('12a3', 'number', ''), null, '非法数字 -> null')
eq(buildLiteral('123.45', 'number', ''), '123.45', '小数')
eq(buildLiteral('true', 'bool', ''), 'TRUE', 'bool true')
eq(buildLiteral('0', 'bool', ''), 'FALSE', 'bool 0')
eq(buildLiteral('是', 'bool', ''), 'TRUE', 'bool 是')
eq(buildLiteral('xyz', 'bool', ''), null, 'bool 非法')
eq(buildLiteral("it's", 'string', 'pg'), "'it''s'", 'PG 只翻引号')
eq(buildLiteral('C:\\temp\\', 'string', 'mysql'), "'C:\\\\temp\\\\'", 'MySQL 反斜杠翻倍')
eq(buildLiteral('C:\\temp\\', 'string', 'clickhouse'), "'C:\\\\temp\\\\'", 'CH 反斜杠翻倍')
eq(buildLiteral('C:\\temp\\', 'string', 'postgresql'), "'C:\\temp\\'", 'PG 不动反斜杠')
eq(buildLiteral('NOW()', 'string', ''), 'NOW()', '函数任何 kind 原样')
eq(buildLiteral('CURRENT_DATE', 'number', ''), 'CURRENT_DATE', '关键字原样')
eq(buildLiteral('NULL', 'string', ''), 'NULL', 'NULL')
eq(buildLiteral('', 'string', ''), null, '空 -> null')

console.log('== varContextOf ==')
eq(varContextOf('where vbeln = ').col, 'vbeln', 'col =')
eq(varContextOf('where t1.order_no = ').col, 'order_no', 'alias.col =')
eq(varContextOf('and x in (').list, 'paren', 'in (')
eq(varContextOf('and x in ').list, 'bare', 'bare in')
eq(varContextOf('limit '), null, 'limit 无列')

console.log('== replaceVars ==')
const values = { vbeln: '1110057272', flag: 'true', cnt: '10', name: "O'Brien", path: 'C:\\tmp' }
const warns = []
const sub = (sql, opts = {}) => replaceVars(sql, values, { dialect: 'clickhouse', ...opts, onWarn: (v) => warns.push(v) })

// 用户报错场景：CH String 列 + 数字输入 → 按列类型生成字符串
eq(sub('select 1\nfrom t\nwhere vbeln = :vbeln', { colTypes: { vbeln: 'string' } }),
  "select 1\nfrom t\nwhere vbeln = '1110057272'", '列 String -> 字符串字面量')
// 无列上下文：auto -> 数字原样（LIMIT 等场景）
eq(sub('select 1 limit :cnt'), 'select 1 limit 10', 'auto 数字原样')
// 显式类型最高优先
eq(sub('where vbeln = :vbeln', { colTypes: { vbeln: 'string' }, varTypes: { vbeln: 'number' } }),
  'where vbeln = 1110057272', '显式 number 覆盖列推断')
// bool 列
eq(sub('where flag = :flag', { colTypes: { flag: 'bool' } }), 'where flag = TRUE', 'bool 列')
// in 列表展开（括号已有 / 值带逗号）
eq(sub('where x in (:vbeln)', { colTypes: { x: 'string' } }), "where x in ('1110057272')", 'in ( ) 单值')
eq(sub('where x in :vbeln', { colTypes: { x: 'string' } }), "where x in ('1110057272')", '裸 in 补括号')
// 多值：换一组 values 再测逗号展开
const warns2 = []
const sub2 = (sql, vals, opts = {}) => replaceVars(sql, vals, { dialect: 'clickhouse', ...opts, onWarn: (v) => warns2.push(v) })
eq(sub2('where x in (:ids)', { ids: 'a, b ,c' }, { colTypes: { x: 'string' } }),
  "where x in ('a', 'b', 'c')", 'in 列表逗号展开')
eq(sub2('where x in (:ids)', { ids: '1,2,3' }, { colTypes: { x: 'number' } }),
  'where x in (1, 2, 3)', 'in 数字列表')
// 字符串转义
eq(sub2('where name = :name', { name: "O'Brien" }, {}), "where name = 'O''Brien'", '引号翻倍')
eq(sub2('where path = :path', { path: 'C:\\tmp' }, { dialect: 'mysql' }), "where path = 'C:\\\\tmp'", 'MySQL \\ 翻倍')
// 引号内不替换
eq(sub2("select ':vbeln' from t", { vbeln: '1110057272' }, { colTypes: { vbeln: 'string' } }),
  "select ':vbeln' from t", '字符串字面量内不替换')
// 大小写不敏感的变量名 + 大小写不敏感的列匹配
eq(sub2('where VBELN = :Vbeln', { Vbeln: '1110057272' }, { colTypes: { vbeln: 'string' } }),
  "where VBELN = '1110057272'", '变量名大小写不敏感')
// 非法数字降级 + 警告
const warns3 = []
const got3 = replaceVars('where n = :n', { n: 'abc' }, { colTypes: { n: 'number' }, onWarn: (v) => warns3.push(v) })
eq(got3, "where n = 'abc'", '非法数字降级字符串')
eq(warns3.length, 1, '降级有警告')
// 空值：占位符原样
eq(sub2('where x = :missing', {}), 'where x = :missing', '无值原样保留')
// :: 强转不命中
eq(sub2("select 'x'::int", {}, {}), "select 'x'::int", ':: 强转不命中')

console.log(`\n== ${pass} ok / ${fail} fail ==`)
process.exit(fail ? 1 : 0)
