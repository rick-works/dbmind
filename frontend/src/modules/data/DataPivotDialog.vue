<template>
  <el-dialog v-model="visible" :title="$t('pv.title')" width="960px" top="5vh" class="pivot-dlg" @open="onOpen">
    <div class="pivot-cfg">
      <!-- 聚合方式行（在前） -->
      <div class="cfg-row">
        <span class="cfg-label">{{ $t('pv.aggMode') }}</span>
        <el-select v-model="aggFn" size="default" class="cfg-fn" :class="{ 'cfg-fn--muted': aggFn === 'count' }" :teleported="false">
          <el-option :label="$t('pv.aggCount')" value="count" />
          <el-option :label="$t('pv.aggSum')" value="sum" />
          <el-option :label="$t('pv.aggAvg')" value="avg" />
          <el-option :label="$t('pv.aggMin')" value="min" />
          <el-option :label="$t('pv.aggMax')" value="max" />
        </el-select>
        <el-select v-model="aggCols" multiple collapse-tags size="default" class="cfg-sel2"
                   :disabled="aggFn === 'count'" :max-collapse-tags="2" :teleported="false"
                   :placeholder="aggFn === 'count' ? $t('pv.phAggCount') : $t('pv.phAggCols')">
          <el-option v-for="c in numericColumns" :key="c" :label="c" :value="c" />
        </el-select>
      </div>
      <!-- 分组列行（在后） -->
      <div class="cfg-row">
        <span class="cfg-label">{{ $t('pv.groupCols') }}</span>
        <el-select v-model="groupCols" multiple collapse-tags clearable filterable size="default"
                   :placeholder="$t('pv.phGroupCols')" class="cfg-sel" :max-collapse-tags="3" :teleported="false">
          <el-option v-for="c in columns" :key="c" :label="c" :value="c" />
        </el-select>
        <el-input v-model="search" size="default" clearable :placeholder="$t('pv.phSearch')" class="cfg-search"
                  :prefix-icon="Search" :disabled="!result.length" />
      </div>
    </div>

    <div v-if="error" class="pivot-error">{{ error }}</div>

    <div v-if="result.length" class="pivot-out">
      <div class="pivot-toolbar">
        <span class="cfg-label" style="width:auto">TOP</span>
        <el-select v-model="topN" size="small" style="width:82px" :teleported="false">
          <el-option v-for="n in TOPN_OPTIONS" :key="n" :label="n === 0 ? $t('pv.noLimit') : n" :value="n" />
        </el-select>
        <span class="pivot-summary">
          <template v-if="search">{{ $t('pv.matched') }}<b>{{ viewRows.length }}</b> / </template>
          {{ $t('pv.totalGroups', { n: result.length, agg: aggDesc }) }}
          <span class="pivot-tip">{{ $t('pv.tip') }}</span>
        </span>
        <el-button size="small" text :icon="DocumentCopy" @click="copyCsv">{{ $t('common.copy') }}</el-button>
        <el-button size="small" text :icon="Download" @click="exportCsv">{{ $t('pv.exportCsv') }}</el-button>
      </div>
      <div class="pivot-table-wrap">
        <table class="pivot-table">
          <thead>
            <tr>
              <th class="exp-th"></th>
              <th v-for="c in groupCols" :key="'g' + c" class="sort-th" @click="sortBy('group', c)">
                {{ c }}<i v-if="sortIcon('group', c)" class="sort-ic">{{ sortIcon('group', c) }}</i>
              </th>
              <th v-for="m in metricCols" :key="'m' + m" class="agg-th sort-th" @click="sortBy('metric', m)">
                {{ metricLabel(m) }}<i v-if="sortIcon('metric', m)" class="sort-ic">{{ sortIcon('metric', m) }}</i>
              </th>
              <th class="pct-th" v-if="metricCols.length">{{ $t('pv.share') }}</th>
              <th class="bar-th">{{ $t('pv.distribution') }}</th>
            </tr>
          </thead>
          <tbody>
            <template v-for="(row, i) in viewRows" :key="i">
              <tr :class="{ 'row-expanded': expanded === rowKeyStr(row) }">
                <td class="exp-cell">
                  <span class="exp-arrow" :class="{ open: expanded === rowKeyStr(row) }"
                        @click="toggleDetail(row)" :title="$t('pv.tip')">▸</span>
                </td>
                <td v-for="c in groupCols" :key="'gc' + c" class="g-cell">{{ fmtVal(row.key[c]) }}</td>
                <td v-for="m in metricCols" :key="'mv' + m" class="agg-cell">{{ fmtNum(row.vals[m]) }}</td>
                <td class="pct-cell" v-if="metricCols.length">{{ pctText(row, metricCols[0]) }}</td>
                <td class="bar-cell">
                  <div class="bar" :style="{ width: barWidth(row) }"></div>
                </td>
              </tr>
              <tr v-if="expanded === rowKeyStr(row)" class="detail-row">
                <td :colspan="detailColspan">
                  <div class="detail-box">
                    <div class="detail-head">{{ $t('pivot.detailHead', { title: detailTitle(row), n: detailRows(row).length }) }}</div>
                    <table class="detail-table">
                      <thead>
                        <tr><th v-for="c in detailCols" :key="'dc' + c">{{ c }}</th></tr>
                      </thead>
                      <tbody>
                        <tr v-for="(dr, di) in detailRows(row)" :key="di">
                          <td v-for="c in detailCols" :key="'dvc' + c">{{ fmtVal(dr[c]) }}</td>
                        </tr>
                      </tbody>
                    </table>
                  </div>
                </td>
              </tr>
            </template>
          </tbody>
          <tfoot v-if="!search">
            <tr>
              <td class="total-cell"></td>
              <td v-for="c in groupCols" :key="'tc' + c" class="total-cell">{{ $t('pv.total') }}</td>
              <td v-for="m in metricCols" :key="'tm' + m" class="agg-cell total-cell">{{ fmtNum(totals[m]) }}</td>
              <td class="pct-cell total-cell" v-if="metricCols.length">100%</td>
              <td class="bar-cell total-cell"></td>
            </tr>
          </tfoot>
        </table>
      </div>
    </div>
    <el-empty v-else-if="computedOnce && !error" :description="$t('pv.noPivotData')" />
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { saveBlobAs } from '../../utils/useExportTask'
import { ElMessage } from 'element-plus'
import { t } from '../../utils/i18n'
import { Search, DocumentCopy, Download } from '@element-plus/icons-vue'

const props = defineProps({
  modelValue: Boolean,
  columns: { type: Array, default: () => [] },
  rows: { type: Array, default: () => [] }
})
const emit = defineEmits(['update:modelValue'])
const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const groupCols = ref([])
const aggFn = ref('count')
const aggCols = ref([])
const sortMode = ref('desc')
const sortField = ref('__count')
const topN = ref(0)
const search = ref('')
const result = ref([])
const totals = ref({})
const error = ref('')
const computedOnce = ref(false)
const expanded = ref('')

const TOPN_OPTIONS = [0, 10, 20, 50, 100]

// 默认排除明显是 ID/编号/编码的列，避免把分组拉得过细
const ID_LIKE_RE = /(^|_)(id|uuid|guid|phone|mobile|code|sn)$|_id$|_no$/i

const numericColumns = computed(() => {
  const sample = (props.rows || []).slice(0, 50)
  if (!sample.length) return []
  return (props.columns || []).filter(c => {
    let num = 0, total = 0
    for (const r of sample) {
      const v = r[c]
      if (v === null || v === undefined || v === '') continue
      total++
      if (typeof v === 'number') { num++; continue }
      if (typeof v === 'string' && v.trim() !== '' && !isNaN(Number(v))) num++
    }
    return total > 0 && num / total >= 0.6
  })
})

const pad2 = (n) => String(n).padStart(2, '0')
/** 分组取值：直接取原始值 */
const groupValue = (c, row) => row[c]

const metricCols = computed(() => {
  if (aggFn.value === 'count') return ['__count']
  return aggCols.value.slice()
})
const metricLabel = (m) => m === '__count' ? t('pv.count') : `${aggFnLabel.value}(${m})`
const aggFnLabel = computed(() => ({
  count: t('pv.count'), sum: t('sqlq.sum'), avg: t('pv.avg'), min: t('sqlq.min'), max: t('sqlq.max')
}[aggFn.value] || aggFn.value))
const aggDesc = computed(() => {
  if (aggFn.value === 'count') return 'COUNT(*)'
  // 连接符也随语言：中文用顿号、英文用逗号
  const cols = aggCols.value.length ? aggCols.value.join(t('common.listSep')) : t('pv.unselectedCols')
  return `${aggFnLabel.value}(${cols})`
})

const onOpen = () => {
  error.value = ''
  computedOnce.value = false
  search.value = ''
  expanded.value = ''
  // 首次打开给智能默认：分组列跳过 ID 类列；有数值列时默认求和第一列
  if (!groupCols.value.length && props.columns.length) {
    const cand = props.columns.filter(c => !ID_LIKE_RE.test(c))
    groupCols.value = [cand[0] || props.columns[0]]
  }
  if (aggFn.value === 'count' && numericColumns.value.length && !aggCols.value.length) {
    aggFn.value = 'sum'
  }
  if (aggFn.value !== 'count' && !aggCols.value.length) {
    aggCols.value = numericColumns.value.slice(0, 2)
  }
  compute()
}

const isNumeric = (v) => v != null && v !== '' && !isNaN(Number(v))
const fmtVal = (v) => {
  if (v == null || v === '') return t('pv.empty')
  if (v instanceof Date) return `${v.getFullYear()}-${pad2(v.getMonth() + 1)}-${pad2(v.getDate())}`
  return String(v)
}
const fmtNum = (v) => {
  if (v == null || isNaN(v)) return '-'
  if (Number.isInteger(v)) return Number(v).toLocaleString('zh-CN')
  return Number(v).toLocaleString('zh-CN', { maximumFractionDigits: 4 })
}

/** 纯前端分组聚合：复用当前结果集 rows（字段即列名），不消耗后端 */
const compute = () => {
  error.value = ''
  computedOnce.value = true
  if (!groupCols.value.length) { error.value = t('pv.needGroupCol'); result.value = []; return }
  if (aggFn.value !== 'count' && !aggCols.value.length) { error.value = t('pv.needAggCol'); result.value = []; return }
  const gc = groupCols.value
  const metrics = aggFn.value === 'count' ? [] : aggCols.value
  const map = new Map()
  let totalCount = 0
  // 全局累加器：合计行用（avg 的合计 = 全量 sum/全量 n，min/max 的合计 = 全量 min/max）
  const g = {}
  for (const m of metrics) g[m] = { sum: 0, n: 0, min: null, max: null }
  for (const row of props.rows) {
    const keyParts = gc.map(c => fmtVal(groupValue(c, row)))
    const k = keyParts.join('\u0001')
    let entry = map.get(k)
    if (!entry) {
      entry = { key: {}, count: 0, vals: metrics.reduce((o, m) => (o[m] = { sum: 0, n: 0, min: null, max: null }, o), {}) }
      map.set(k, entry)
    }
    gc.forEach((c, i) => { entry.key[c] = keyParts[i] })
    entry.count++
    totalCount++
    for (const m of metrics) {
      const raw = row[m]
      if (!isNumeric(raw)) continue
      const n = Number(raw)
      const acc = entry.vals[m], gacc = g[m]
      acc.sum += n; acc.n++
      gacc.sum += n; gacc.n++
      if (acc.min == null || n < acc.min) acc.min = n
      if (acc.max == null || n > acc.max) acc.max = n
      if (gacc.min == null || n < gacc.min) gacc.min = n
      if (gacc.max == null || n > gacc.max) gacc.max = n
    }
  }
  const calc = (acc) => {
    if (aggFn.value === 'sum') return acc.n ? acc.sum : null
    if (aggFn.value === 'avg') return acc.n ? acc.sum / acc.n : null
    if (aggFn.value === 'min') return acc.min
    if (aggFn.value === 'max') return acc.max
    return null
  }
  const out = []
  for (const e of map.values()) {
    const vals = {}
    if (aggFn.value === 'count') vals.__count = e.count
    else for (const m of metrics) vals[m] = calc(e.vals[m])
    out.push({ key: e.key, vals })
  }
  // 排序：按指定指标升降序，或按分组键（中文排序）
  const pm = metricCols.value.includes(sortField.value) ? sortField.value : metricCols.value[0]
  if (sortMode.value === 'key') {
    out.sort((a, b) => gc.map(c => a.key[c]).join('\u0001').localeCompare(gc.map(c => b.key[c]).join('\u0001'), 'zh-CN'))
  } else {
    const dir = sortMode.value === 'asc' ? 1 : -1
    out.sort((a, b) => dir * ((Number(a.vals[pm]) || 0) - (Number(b.vals[pm]) || 0)))
  }
  // TopN 截断（合计行仍是全量口径）
  const LIMIT = 2000
  let cut = out
  if (topN.value > 0 && out.length > topN.value) cut = out.slice(0, topN.value)
  if (cut.length > LIMIT) cut = cut.slice(0, LIMIT)
  result.value = cut
  // 合计行
  totals.value = {}
  if (aggFn.value === 'count') totals.value.__count = totalCount
  else for (const m of metrics) {
    const ga = g[m]
    totals.value[m] = aggFn.value === 'sum' ? (ga.n ? ga.sum : null)
      : aggFn.value === 'avg' ? (ga.n ? ga.sum / ga.n : null)
      : aggFn.value === 'min' ? ga.min
      : ga.max
  }
  if (!result.value.length) error.value = t('pv.noData')
}

// 配置变化自动重算（搜索只影响展示，不重算）
watch([groupCols, aggFn, aggCols, sortMode, sortField, topN], compute)
// 切到计数时清空数值列选择（计数不需要数值列）
watch(aggFn, (fn) => { if (fn === 'count') aggCols.value = [] })
// 切到计数/换指标时，排序字段跟随第一指标
watch(metricCols, (ms) => { if (!ms.includes(sortField.value)) sortField.value = ms[0] })

const sortBy = (kind, field) => {
  if (kind === 'group') { sortMode.value = 'key'; return }
  if (sortField.value === field && sortMode.value === 'desc') sortMode.value = 'asc'
  else { sortField.value = field; sortMode.value = 'desc' }
}
const sortIcon = (kind, field) => {
  if (sortMode.value === 'key') return kind === 'group' ? '▲' : ''
  if (kind === 'metric' && sortField.value === field) return sortMode.value === 'desc' ? '▼' : '▲'
  return ''
}

/** 展示行：搜索关键字过滤分组键（不改变合计口径） */
const viewRows = computed(() => {
  const kw = search.value.trim().toLowerCase()
  if (!kw) return result.value
  return result.value.filter(r => groupCols.value.some(c => String(r.key[c]).toLowerCase().includes(kw)))
})

// 条形与占比都基于第一个指标
const maxVal = computed(() => {
  const pm = metricCols.value[0]
  return viewRows.value.reduce((mv, r) => Math.max(mv, Number(r.vals[pm]) || 0), 0)
})
const totalVal = computed(() => {
  const pm = metricCols.value[0]
  return viewRows.value.reduce((s, r) => s + (Number(r.vals[pm]) || 0), 0)
})
const barWidth = (row) => {
  if (!maxVal.value) return '0%'
  const pct = ((Number(row.vals[metricCols.value[0]]) || 0) / maxVal.value) * 100
  return Math.max(2, Math.min(100, pct)) + '%'
}
const pctText = (row, m) => {
  const total = totalVal.value
  if (!total) return '-'
  return ((Number(row.vals[m]) || 0) / total * 100).toFixed(1) + '%'
}

// ===== 组明细下钻 =====
const rowKeyStr = (row) => groupCols.value.map(c => row.key[c]).join('\u0001')
const toggleDetail = (row) => { expanded.value = expanded.value === rowKeyStr(row) ? '' : rowKeyStr(row) }
const detailTitle = (row) => groupCols.value.map(c => `${c}=${row.key[c]}`).join(t('common.listSep'))
const detailCols = computed(() => (props.columns || []).slice(0, 10))
const detailColspan = computed(() => 1 + groupCols.value.length + metricCols.value.length + (metricCols.value.length ? 2 : 1))
const detailRows = (row) => {
  const keyStr = rowKeyStr(row)
  const out = []
  for (const r of props.rows) {
    const ks = groupCols.value.map(c => fmtVal(groupValue(c, r))).join('\u0001')
    if (ks === keyStr) { out.push(r); if (out.length >= 50) break }
  }
  return out
}

// ===== CSV =====
const csvLines = () => {
  const head = [...groupCols.value, ...metricCols.value.map(metricLabel)].join(',')
  const esc = (s) => {
    const t = s == null ? '' : String(s)
    return /[",\n]/.test(t) ? `"${t.replace(/"/g, '""')}"` : t
  }
  const body = result.value.map(r => [
    ...groupCols.value.map(c => esc(fmtVal(r.key[c]))),
    ...metricCols.value.map(m => esc(fmtNum(r.vals[m])))
  ].join(','))
  return [head, ...body].join('\r\n')
}
const copyCsv = async () => {
  try {
    await navigator.clipboard.writeText(csvLines())
    ElMessage.success(t('pv.copied', { n: result.value.length }))
  } catch { ElMessage.error(t('pv.copyFailed')) }
}
const exportCsv = () => {
  // 带 BOM：Excel 直接打开不乱码
  const blob = new Blob(['\uFEFF' + csvLines()], { type: 'text/csv;charset=utf-8' })
  saveBlobAs(blob, `pivot-${Date.now()}.csv`)
}
</script>

<style scoped>
.pivot-cfg { display: flex; flex-direction: column; gap: 10px; margin-bottom: 12px; }
.cfg-row { display: flex; align-items: center; gap: 10px; flex-wrap: nowrap; }
.cfg-label { width: 64px; font-size: 14px; color: var(--dc-text); flex-shrink: 0; }
.cfg-sel { min-width: 280px; flex: 1; }
.cfg-search { width: 180px; flex-shrink: 0; }
.cfg-sel2 { min-width: 200px; flex: 1; }
.cfg-fn { width: 150px; flex-shrink: 0; }
/* 计数模式下聚合方式下拉显示为灰字（提示当前不依赖数值列） */
.cfg-fn--muted :deep(.el-select__wrapper) {
  --el-text-color-regular: var(--dc-text-dim, #909399);
  --el-input-text-color: var(--dc-text-dim, #909399);
  color: var(--dc-text-dim, #909399);
}
.cfg-fn--muted :deep(.el-select__wrapper input),
.cfg-fn--muted :deep(.el-select__selected-item),
.cfg-fn--muted :deep(.el-select__placeholder),
.cfg-fn--muted :deep(.el-select__selection),
.cfg-fn--muted :deep(.el-select__selection span) { color: var(--dc-text-dim, #909399) !important; }
.pivot-error { color: #f56c6c; font-size: 14px; margin-bottom: 10px; }
.pivot-toolbar { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; flex-wrap: nowrap; }
.pivot-summary { flex: 1; font-size: 14px; color: var(--dc-text-dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.pivot-summary b { color: var(--dc-text); }
.pivot-tip { font-size: 12px; }
.pivot-table-wrap { max-height: 54vh; overflow: auto; border: 1px solid var(--dc-border); border-radius: 6px; }
.pivot-table { width: 100%; border-collapse: collapse; font-size: 14px; }
.pivot-table th, .pivot-table td { padding: 6px 10px; border-bottom: 1px solid var(--dc-border); text-align: left; }
.pivot-table thead th { position: sticky; top: 0; z-index: 1; background: var(--dc-bg); font-weight: 600; }
.pivot-table tfoot td { position: sticky; bottom: 0; z-index: 1; background: var(--dc-bg-soft, var(--el-fill-color-light)); font-weight: 600; }
.sort-th { cursor: pointer; user-select: none; }
.sort-th:hover { color: var(--dc-primary, var(--el-color-primary)); }
.sort-ic { font-style: normal; font-size: 11px; margin-left: 3px; color: var(--dc-primary, var(--el-color-primary)); }
.agg-th, .agg-cell { text-align: right; font-variant-numeric: tabular-nums; }
.exp-th { width: 32px; }
.exp-cell { width: 32px; text-align: center; }
.exp-arrow { cursor: pointer; color: var(--dc-text-dim); display: inline-block; transition: transform .15s; font-size: 13px; }
.exp-arrow:hover { color: var(--dc-primary, var(--el-color-primary)); }
.exp-arrow.open { transform: rotate(90deg); }
.row-expanded td { background: var(--dc-bg-soft, var(--el-fill-color-lighter)); }
.detail-row td { padding: 0 10px 10px; background: var(--dc-bg-soft, var(--el-fill-color-lighter)); }
.detail-box { border: 1px solid var(--dc-border); border-radius: 6px; background: var(--dc-bg); padding: 8px 10px; max-height: 240px; overflow: auto; }
.detail-head { font-size: 13px; color: var(--dc-text-dim); margin-bottom: 6px; }
.detail-table { width: 100%; border-collapse: collapse; font-size: 13px; }
.detail-table th, .detail-table td { padding: 3px 8px; border-bottom: 1px solid var(--dc-border); text-align: left; white-space: nowrap; }
.detail-table thead th { position: static; background: var(--dc-bg); font-weight: 600; }
.pct-th, .pct-cell { text-align: right; font-variant-numeric: tabular-nums; color: var(--dc-text-dim); }
.total-cell { color: var(--dc-text); }
.bar-th { width: 22%; }
.bar-cell { width: 22%; }
.bar { height: 12px; background: linear-gradient(90deg, var(--dc-primary), #79bbff); border-radius: 3px; min-width: 2px; transition: width .25s ease; }
</style>
