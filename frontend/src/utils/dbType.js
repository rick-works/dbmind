// ===== 数据库列类型分类（数值 / 非数值）=====
//
// 为什么单独抽出来：以前每个视图各写一份「以 int / decimal 开头」的正则，于是
//   · ClickHouse 的类型是 `UInt64`、`Nullable(Decimal(18, 2))` 这种写法，一个都匹配不上，
//     数字列被判成文本 —— 表头图标变成文本图标、框选一片数字格底栏只出「选中 N 格」，
//     求和 / 均值 / 最大最小全都不显示（真机反馈）；
//   · 同一件事还各判各的：对齐（utils/cellAlign.js）早就写对了（认 `u?int\d*` 与包装），
//     结果同一个格子「右对齐但不算数值」，看着自相矛盾。
// 现在只有这一份实现，谁要用谁 import。

/// 数值家族：允许前缀噪声（`unsigned big int`）与包装（`Nullable(...)`、`LowCardinality(...)`），
/// 所以两边都用「非字母边界」而不是 `^` / `$`，并把 `u?int\d*` 这种变体也算进去。
export const NUMERIC_TYPE_RE = /(^|[^a-z])(u?int\d*|bigint|smallint|tinyint|mediumint|integer|serial\d*|bigserial|decimal|numeric|number|float\d*|double|real|money|smallmoney|year|bit)([^a-z]|$)/i

/// 明确**不是**数值的家族：文本 / 时间 / 二进制 / JSON / 枚举…
/// `FixedString` 也会落在这里（含 `string`），它不该被当成数字。
export const NOT_NUMERIC_TYPE_RE = /(char|text|clob|string|json|uuid|date|time|bool|binary|blob|byte|enum|set|interval|geo|point)/i

/// 严格数字字面量。带千分位（"1,234"）或单位（"12 ms"）都**不算** —— 硬转成数字只会得到 NaN。
export const NUMERIC_LITERAL_RE = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/

/**
 * 类型名给出的"是不是数值"提示：
 * - `true`  ：明确属于数值家族
 * - `false` ：明确不是（文本 / 时间 / 二进制…）
 * - `null`  ：认不出来（自定义类型、拿不到类型）—— 交给**值本身**判断
 */
export const numericTypeHint = (type) => {
  const s = String(type == null ? '' : type).trim()
  if (!s) return null
  if (NOT_NUMERIC_TYPE_RE.test(s)) return false
  if (NUMERIC_TYPE_RE.test(s)) return true
  return null
}

/// 值是不是严格数字字面量（null / undefined / 空串都算不是）
export const isNumericLiteral = (v) => (v == null ? false : NUMERIC_LITERAL_RE.test(String(v).trim()))
