/**
 * 可视化过滤条件工具的公共逻辑：操作符定义与 SQL 片段生成。
 * 供 ConditionBuilder.vue（预览）与 CompareDialog.vue（提交参数）复用，保证一致。
 */

export const COND_OPS = [
  { v: '=', lKey: 'cop.eq' }, { v: '!=', lKey: 'cop.ne' },
  { v: '>', lKey: 'cop.gt' }, { v: '>=', lKey: 'cop.ge' },
  { v: '<', lKey: 'cop.lt' }, { v: '<=', lKey: 'cop.le' },
  { v: 'like', lKey: 'cop.like' }, { v: 'notlike', lKey: 'cop.notlike' },
  { v: 'in', lKey: 'cop.in' }, { v: 'notin', lKey: 'cop.notin' },
  { v: 'isnull', lKey: 'cop.isnull' }, { v: 'isnotnull', lKey: 'cop.isnotnull' },
  { v: 'between', lKey: 'cop.between' }
]

// 需要输入值的操作符
export const needValue = op => !['isnull', 'isnotnull'].includes(op)

const NUM = ['INT', 'DECIMAL', 'NUMERIC', 'FLOAT', 'DOUBLE', 'REAL', 'NUMBER', 'BIGINT', 'SMALLINT', 'TINYINT', 'BIT']
const isNum = t => NUM.some(k => (t || '').includes(k))

const escRaw = s => String(s ?? '').replace(/'/g, "''")

// 按字段类型决定是否给值加引号（数值不加，其余加引号）
function lit(item, raw, columns) {
  const col = (columns || []).find(c => c.name === item.field)
  const t = (col?.type || '').toUpperCase()
  if (col && isNum(t)) return String(raw)
  return `'${escRaw(raw)}'`
}

function buildItem(it, columns) {
  if (!it.field) return null
  const f = it.field
  const val = it.value
  switch (it.op) {
    case '=': return `${f} = ${lit(it, val, columns)}`
    case '!=': return `${f} <> ${lit(it, val, columns)}`
    case '>': return `${f} > ${lit(it, val, columns)}`
    case '>=': return `${f} >= ${lit(it, val, columns)}`
    case '<': return `${f} < ${lit(it, val, columns)}`
    case '<=': return `${f} <= ${lit(it, val, columns)}`
    case 'like': return `${f} LIKE '%${escRaw(val)}%'`
    case 'notlike': return `${f} NOT LIKE '%${escRaw(val)}%'`
    case 'in': {
      const vals = (val || '').split(',').map(s => s.trim()).filter(Boolean).map(s => lit(it, s, columns))
      return vals.length ? `${f} IN (${vals.join(', ')})` : null
    }
    case 'notin': {
      const vals = (val || '').split(',').map(s => s.trim()).filter(Boolean).map(s => lit(it, s, columns))
      return vals.length ? `${f} NOT IN (${vals.join(', ')})` : null
    }
    case 'between': return `${f} BETWEEN ${lit(it, val, columns)} AND ${lit(it, it.value2, columns)}`
    case 'isnull': return `${f} IS NULL`
    case 'isnotnull': return `${f} IS NOT NULL`
    default: return null
  }
}

/**
 * 把 { logic, items } 条件模型转换为 SQL WHERE 片段。
 * 空条件返回空字符串。
 */
export function buildConditionSql(cond, columns) {
  const logic = cond?.logic || 'AND'
  const items = cond?.items || []
  const parts = items.map(it => buildItem(it, columns)).filter(Boolean)
  return parts.join(` ${logic} `)
}
