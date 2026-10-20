/* ClickHouse 常用引擎清单 —— 「新建表」与「编辑表结构」共用这**同一份**，别再各写一份。
 *
 * 清单里给的就是**能直接落进 SQL 的完整写法**：
 *   · MergeTree 族（含 Replicated* 副本版）必须带 `()`（ClickHouse 语法要求，不带就是语法错）；
 *   · Log / TinyLog / StripeLog / Set / Join / Memory / Null 这些无参引擎**不能**带括号。
 * 所以生成 SQL 时原样拼 `ENGINE = <选中的值>` 即可 —— 不需要按引擎名去猜"该不该补括号"
 * （那种猜法遇到手输的 `ReplicatedMergeTree('/p','r')` 就会补错）。
 *
 * 注意：**Replicated 系列必须列全**（ReplicatedMergeTree 等）。漏了它们的后果是：
 * 编辑表结构里这张表的引擎（`system.tables.engine` = `ReplicatedMergeTree`）在清单里
 * 找不到同名项，就原样显示成不带括号的 `ReplicatedMergeTree`（用户实测踩到过）。 */
export const CH_ENGINES = [
  // MergeTree 族：单机版
  'MergeTree()', 'ReplacingMergeTree()', 'SummingMergeTree()', 'AggregatingMergeTree()',
  'CollapsingMergeTree()', 'VersionedCollapsingMergeTree()', 'GraphiteMergeTree()',
  // MergeTree 族：副本版（Replicated*）
  'ReplicatedMergeTree()', 'ReplicatedReplacingMergeTree()', 'ReplicatedSummingMergeTree()',
  'ReplicatedAggregatingMergeTree()', 'ReplicatedCollapsingMergeTree()',
  'ReplicatedVersionedCollapsingMergeTree()', 'ReplicatedGraphiteMergeTree()',
  // 无参引擎（不能带括号）
  'Log', 'TinyLog', 'StripeLog', 'Set', 'Join', 'Memory', 'Null',
]
