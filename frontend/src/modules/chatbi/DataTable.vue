<template>
  <div class="dt-wrap">
    <table class="dt">
      <thead>
        <tr>
          <th v-for="col in columns" :key="col" :class="{ num: isNumeric(col) }">{{ col }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(row, i) in shown" :key="i">
          <td v-for="col in columns" :key="col" :class="{ num: isNumeric(col) }">
            <span v-if="isEmpty(row[col])" class="dt-null">NULL</span>
            <template v-else>{{ format(col, row[col]) }}</template>
          </td>
        </tr>
      </tbody>
    </table>
    <div v-if="total > shown.length" class="dt-more">
      仅显示前 {{ shown.length }} 行，共 {{ total }} 行 —— 需要全量请用「送到编辑器」打开 SQL 查
    </div>
  </div>
</template>

<script setup>
// 结果明细表：问数的"兜底证据"。
// 图表可能把 3000 行压成 12 个点，用户要能顺着表核对到底（数对不上是这类功能最常见的质疑）。
import { computed } from 'vue'

const props = defineProps({
  answer: { type: Object, default: () => ({ columns: [], columnTypes: [], rows: [] }) },
  limit: { type: Number, default: 100 },
})

const columns = computed(() => (props.answer?.columns || []).filter((c) => c && c !== '__row'))
const allRows = computed(() => props.answer?.rows || [])
const total = computed(() => props.answer?.rowCount ?? allRows.value.length)
const shown = computed(() => allRows.value.slice(0, props.limit))

const isEmpty = (value) => value === null || value === undefined

const numericTypes = ['int', 'dec', 'num', 'float', 'double', 'real', 'money', 'bigint']
const isNumeric = (column) => {
  const index = (props.answer?.columns || []).indexOf(column)
  const type = String(props.answer?.columnTypes?.[index] || '').toLowerCase()
  return numericTypes.some((token) => type.includes(token))
}

const format = (column, value) => {
  if (typeof value === 'boolean') return value ? 'true' : 'false'
  if (typeof value === 'number' && isNumeric(column)) {
    return Number.isInteger(value) ? value.toLocaleString('zh-CN') : String(value)
  }
  const text = String(value)
  // 长文本（JSON / 备注）不撑破列宽
  return text.length > 160 ? `${text.slice(0, 160)}…` : text
}
</script>

<style scoped>
.dt-wrap { overflow: auto; max-height: inherit; }
.dt { width: 100%; border-collapse: collapse; font-size: 12px; }
.dt th, .dt td {
  padding: 5px 10px; border-bottom: 1px solid var(--dc-border);
  white-space: nowrap; text-align: left; font-weight: 400;
}
.dt th {
  position: sticky; top: 0; z-index: 1;
  background: var(--dc-bg-raised); color: var(--dc-text-2); font-size: 11px;
}
.dt tbody tr:hover { background: var(--dc-bg-card); }
.dt td.num, .dt th.num { text-align: right; font-variant-numeric: tabular-nums; }
.dt-null { color: var(--dc-text-2); opacity: .6; font-style: italic; }
.dt-more {
  padding: 6px 10px; font-size: 11px; color: var(--dc-text-2);
  background: var(--dc-bg-raised); position: sticky; bottom: 0;
}
</style>
