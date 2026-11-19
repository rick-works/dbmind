<template>
  <!-- 工具调用的展开体：参数按「键 → 值」摊平（原始 JSON 一坨没人看），
       结果里的制表符文本还原成表格，长内容自己滚，不把结论顶出屏幕。 -->
  <div class="td-wrap">
    <div v-if="rows.length" class="td-sec">
      <!-- 「参数」是**区块标题**，不是某个参数的名字 —— 后面跟一个条数徽标，
           否则它长得和下面「键 → 值」的行一模一样，用户会以为这行永远没值 -->
      <div class="td-h">
        {{ $t('ai.toolArgs') }}
        <span class="td-h-n">{{ $t('ai.toolArgsCount', { n: rows.length }) }}</span>
      </div>
      <div v-for="a in rows" :key="a.key" class="td-row">
        <span class="td-k">{{ a.label }}</span>
        <pre v-if="a.multi" class="td-v td-code">{{ a.text }}</pre>
        <span v-else class="td-v">{{ a.text }}</span>
      </div>
    </div>
    <div v-if="view" class="td-sec">
      <div class="td-h">
        {{ $t('ai.toolResult') }}
        <span v-if="step.ok === false" class="td-bad">{{ $t('ai.toolFailed') }}</span>
      </div>
      <table v-if="view.head" class="td-table">
        <thead>
          <tr><th v-for="(h, hi) in view.head" :key="hi">{{ h }}</th></tr>
        </thead>
        <tbody>
          <tr v-for="(row, ri) in view.rows" :key="ri">
            <td v-for="(cell, ci) in row" :key="ci">{{ cell }}</td>
          </tr>
        </tbody>
      </table>
      <pre v-else class="td-pre" :class="{ 'is-bad': step.ok === false }">{{ view.text }}</pre>
      <div v-if="view.more" class="td-more">{{ $t('ai.toolMoreRows', { n: view.more }) }}</div>
    </div>
    <div v-if="!rows.length && !view" class="td-empty">{{ step.running ? $t('ai.toolRunning') : $t('ai.toolNoDetail') }}</div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { t, te } from '../../utils/i18n'

const props = defineProps({
  /** 时间线里的一个 tool 条目：{ tool, args, ok, durationMs, result, running } */
  step: { type: Object, required: true }
})

const MAX_ROWS = 40

/** 参数名翻成人话；没收录的原样显示（英文键名本身也是可读的） */
const labelOf = (key) => (te('ai.arg.' + key) ? t('ai.arg.' + key) : key)

/** 值怎么摊：字符串原样（多行/过长走 <pre>），数组/对象格式化成缩进 JSON */
const asText = (value) => {
  if (value == null) return ''
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  if (Array.isArray(value) && value.every((v) => ['string', 'number', 'boolean'].includes(typeof v))) {
    const joined = value.join(', ')
    if (joined.length <= 120) return joined
  }
  try { return JSON.stringify(value, null, 2) } catch { return String(value) }
}

const rows = computed(() => {
  const args = props.step.args
  if (!args || typeof args !== 'object') return []
  return Object.keys(args).map((key) => {
    const text = asText(args[key])
    return { key, label: labelOf(key), text, multi: text.includes('\n') || text.length > 140 }
  })
})

/**
 * 结果形态：
 *  - 制表符分隔的多行文本（list_tables / describe_table 这类）→ 真表格；
 *  - 其余原样按 <pre> 展示（保留缩进与换行，SQL 报错也看得清）。
 * 行数太多只列前 40 行并提示还有多少（结果本身已有 600 字上限，这里主要是防呆）。
 */
const view = computed(() => {
  // 变更单的结果就是「已生成…单号…等审批」，下方卡片已经把这些全列出来了（SQL/依据/风险/撤销），
  // 再摊一遍就是重复内容 —— 这里不再显示结果，只看参数就够。
  if (props.step.tool === 'propose_change') return null
  const text = String(props.step.result == null ? '' : props.step.result).trim()
  if (!text) return null
  const lines = text.split('\n').filter((l) => l.trim())
  // 制表符表格：按**最宽的一行**定列数再补齐，不能要求「每行格数完全相等」。
  // 踩过的坑：describe_table 的结果里「标记」列整列为空，末行的收尾制表符被整体 trim
  // 吃掉 ⇒ 末行比表头少一格，整块被判成普通文本，用户看到的就是一坨空格对齐的原文。
  const grid = lines.map((line) => line.split('\t').map((c) => c.trim()))
  const width = grid.reduce((w, r) => Math.max(w, r.length), 0)
  const tabular = grid.filter((r) => r.length >= 2).length
  if (grid.length >= 2 && width >= 2 && grid[0].length >= 2 && tabular >= Math.ceil(grid.length / 2)) {
    const rows = grid.map((r) => {
      const a = r.slice(0, width)
      while (a.length < width) a.push('')
      return a
    })
    const body = rows.slice(1)
    return {
      head: rows[0],
      rows: body.slice(0, MAX_ROWS),
      more: body.length > MAX_ROWS ? body.length - MAX_ROWS : 0
    }
  }
  return { text }
})
</script>

<style scoped>
.td-wrap {
  display: flex; flex-direction: column; gap: 10px;
  padding: 8px 10px 10px;
  animation: tdIn .16s ease;
}
@keyframes tdIn { from { opacity: 0; transform: translateY(-3px); } to { opacity: 1; transform: none; } }
.td-sec { display: flex; flex-direction: column; gap: 4px; }
.td-sec + .td-sec { padding-top: 8px; border-top: 1px dashed var(--dc-border); }
.td-h {
  display: flex; align-items: center; gap: 6px;
  font-size: 11px; font-weight: 600;
  color: var(--dc-text-weak);
}
/* 标题后面的条数徽标：把「这是一个区块标题」和「这是一行参数」分开 */
.td-h-n {
  padding: 0 6px; border-radius: 999px;
  font-size: 10px; font-weight: 500; line-height: 16px;
  background: var(--dc-bg-hover); color: var(--dc-text-dim);
}
.td-bad { color: var(--dc-danger, #d64545); font-weight: 500; }
/* 一行一个参数：键固定宽（对齐），值占满剩余宽度 */
.td-row {
  display: grid; grid-template-columns: 64px minmax(0, 1fr);
  gap: 10px; align-items: start;
  font-size: 12px; line-height: 1.6;
}
/* 键名比区块标题缩进一点：一眼分得清「参数 / 结果」是章节头，键名才是数据 */
.td-k { color: var(--dc-text-weak); padding-left: 4px; }
.td-v { color: var(--dc-text); word-break: break-word; white-space: pre-wrap; }
/* 底色用 --dc-code-bg（比气泡亮一档的蓝灰）而不是 --dc-bg-deep：
   深色主题下 bg-deep/#14151b 在展开体里像挖了个黑坑（用户反馈「背景不要这么黑」）。 */
.td-code {
  margin: 0; padding: 6px 8px; border-radius: 6px;
  background: var(--dc-code-bg); border: 1px solid var(--dc-border);
  font-size: 11.5px; line-height: 1.55; max-height: 220px; overflow: auto;
}
.td-pre {
  margin: 0; padding: 6px 8px; border-radius: 6px;
  background: var(--dc-code-bg); border: 1px solid var(--dc-border);
  font-size: 11.5px; line-height: 1.55; white-space: pre-wrap; word-break: break-word;
  max-height: 240px; overflow: auto;
}
/* 失败结果：左侧红条 + 淡红底，一眼看出这不是正常输出 */
.td-pre.is-bad {
  border-color: rgba(214, 69, 69, .45);
  background: rgba(214, 69, 69, .08);
  color: var(--dc-danger, #d64545);
}
/* 结果表格：细边框、表头压暗、数字列右对齐，别做成「数据大表」那么重 */
.td-table {
  width: 100%; border-collapse: collapse; table-layout: fixed;
  font-size: 11.5px; line-height: 1.5;
  border: 1px solid var(--dc-border); border-radius: 6px; overflow: hidden;
}
.td-table th, .td-table td {
  padding: 4px 8px; text-align: left;
  border-bottom: 1px solid var(--dc-border);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.td-table th {
  background: var(--dc-code-bg); color: var(--dc-text-weak);
  font-weight: 600;
}
.td-table tr:last-child td { border-bottom: none; }
.td-table td { color: var(--dc-text-mid); }
.td-more, .td-empty { font-size: 11px; color: var(--dc-text-dim); }
</style>
