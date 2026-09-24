<template>
  <div class="nosql-view">
    <div class="toolbar">
      <el-input v-model="keyword" clearable size="small" style="width: 260px"
                @keyup.enter="load(1)" @clear="load(1)">
        <template #prefix><el-icon><Search /></el-icon></template>
      </el-input>
      <el-button size="small" :icon="Search" @click="load(1)">{{ $t('nsql.query') }}</el-button>
      <span class="spacer" />
      <span v-if="loading" class="loading-text">
        <el-icon class="is-loading"><Loading /></el-icon> {{ $t('nsql.loading') }}
        <el-button size="small" text type="danger" @click="stop">{{ $t('nsql.stop') }}</el-button>
      </span>
      <span class="info">{{ result.message || '' }}</span>
    </div>
    <el-alert v-if="result.hasMore" type="warning" :closable="false" show-icon class="has-more-alert">
      <template #title>{{ $t('nsql.partialTitle', { n: result.rows.length }) }}</template>
    </el-alert>
    <div class="table-wrap" ref="gridRef" v-loading="loading">
      <div v-if="result?.rows?.length" class="data-table-wrap" ref="tableWrapRef" tabindex="0"
           @scroll="onTableScroll" @keydown="onGridKeydown"
           @mousemove="onTableMove" @mousedown="onTableDown" @mouseleave="onTableLeave"
           @dblclick="onTableDblClick"
           @contextmenu.prevent="onGridContextMenu">
        <!-- 总宽用内联 width 显式给出（与 SQL 结果表格同一套）：
             table-layout:fixed 下浏览器会按内容算 max-content 再把余量摊回各列，
             <col> 上写的小宽度会被撑回去 —— 表现就是「列宽拖不窄」。 -->
        <table class="data-table" :class="{ 'col-resizing': colResizing }"
               :style="{ width: tableWidth + 'px' }">
          <colgroup>
            <!-- 行号列（Excel 行头）：单击选中整行、按住拖动连选，双击看整行详情 —— 与 SQL 结果表格同一套 -->
            <col class="row-sel-col" style="width: 40px" />
            <col v-for="(col, ci) in columns" :key="'c' + ci"
                 :style="{ width: (colWidths[ci] || defaultColWidth(col)) + 'px' }" />
          </colgroup>
          <thead>
            <tr>
              <th class="row-sel-th" @contextmenu.prevent.stop="onGridContextMenu($event)" />
              <th v-for="(col, ci) in columns" :key="'h' + ci"
                  :title="col + $t('nsql.colTitleSuffix')"
                  :class="{ 'col-selected': selectedCols.has(col) }"
                  @click="onHeaderClick(col)"
                  @contextmenu.prevent.stop="onHeaderContextMenu($event, col)">{{ col }}</th>
            </tr>
          </thead>
          <tbody>
            <!-- 窗口化渲染：上方占位行，撑起未渲染区域的高度（固定行高） -->
            <tr v-if="padTop > 0" class="vp-pad-row" aria-hidden="true">
              <td :colspan="columns.length + 1" :style="{ height: padTop + 'px' }" />
            </tr>
            <tr v-for="(row, i) in visibleRows" :key="vpStart + i"
                :class="{ 'row-alt': (vpStart + i) % 2 === 1, selected: selectedRows.has(vpStart + i) }">
              <!-- key 用绝对行号（vpStart+i）：行在窗口内滑动时保持同一 key，DOM 可复用，只有进出窗口的行才增删。
                   写成 `vtStart` 是**错的** —— 那个名字属于别的表格组件，本组件里没有，模板里会算成 NaN，
                   于是每行拿到同一个 key：Vue 复用错 DOM，表现为「表格多出一行 / 点一次查询数据一直往上加」。 -->
              <td class="row-sel-td" :title="$t('nsql.rowNumTitle', { n: vpStart + i + 1 })"
                  @mousedown.prevent="onRowNumDown(vpStart + i, $event)"
                  @mouseenter="onRowNumEnter(vpStart + i)"
                  @dblclick.stop="openRowDetail(vpStart + i)"
                  @contextmenu.prevent.stop="onRowContextMenu($event, vpStart + i)">
                <span class="row-num-tx">{{ vpStart + i + 1 }}</span>
              </td>
              <td v-for="(col, ci) in columns" :key="'d' + ci"
                  :class="{ 'null-cell': row[col] == null, 'col-selected': selectedCols.has(col), 'active-cell': isActiveCell(vpStart + i, col) }"
                  :title="cellText(row, col)"
                  @mousedown="onCellDown(vpStart + i, col, $event)"
                  @mouseenter="onCellEnter(vpStart + i, col)"
                  @contextmenu.prevent.stop="onCellContextMenu($event, vpStart + i, col)">
                <span v-if="isJsonCell(row, col)" class="json-cell">{{ cellText(row, col) }}</span>
                <span v-else>{{ cellText(row, col) }}</span>
              </td>
            </tr>
            <!-- 窗口化渲染：下方占位行 -->
            <tr v-if="padBottom > 0" class="vp-pad-row" aria-hidden="true">
              <td :colspan="columns.length + 1" :style="{ height: padBottom + 'px' }" />
            </tr>
          </tbody>
        </table>
      </div>
      <el-empty v-else :description="isRedisAllKeys ? $t('nsql.emptyKeys') : $t('nsql.emptyData')" :image-size="80" />
    </div>
    <div v-if="result?.rows?.length" class="pager">
      <!-- 选区信息 + 快捷出口：与 SQL 结果表格底栏同一套语义（选中什么就复制/导出什么） -->
      <span class="sel-info">{{ selectionText }}</span>
      <span class="spacer" />
      <el-button size="small" :icon="DocumentCopy" @click="copySelection('tsv')">{{ $t('nsql.copySelection') }}</el-button>
      <el-button size="small" :icon="Download" @click="exportCsv()">{{ $t('nsql.exportCsv') }}</el-button>
      <span class="result-time">
        {{ loading ? formatElapsed(elapsedTime) : (result.executeTime ? formatElapsed(result.executeTime) : '') }}
      </span>
      <el-pagination background layout="total, sizes, prev, pager, next, jumper" :total="total"
                     :page-size="size" :current-page="page" :page-sizes="pageSizes"
                     size="small"
                     @current-change="load" @size-change="s => { size = s; load(1) }" />
    </div>
    <!-- 右键菜单：与 SQL 结果表格**共用同一个组件**，外观与交互天然一致 -->
    <GridContextMenu :visible="ctx.visible" :x="ctx.x" :y="ctx.y" :items="ctx.items"
                     @select="onCtxSelect" @close="ctx.visible = false" />
    <!-- 整行详情（双击行号 / 右键「查看整行详情」）：与 SQL 结果表格同一个口径 -->
    <el-dialog v-model="rowDetail.visible" :title="rowDetail.title" width="620px" append-to-body>
      <pre class="row-detail-pre">{{ rowDetail.text }}</pre>
      <template #footer>
        <el-button @click="rowDetail.visible = false">{{ $t('common.close') }}</el-button>
        <el-button type="primary" @click="copyText(rowDetail.text, $t('nsql.copiedRowDetail'))">{{ $t('common.copy') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, computed, watch, nextTick, onBeforeUnmount } from 'vue'
import { Search, Loading, Download, DocumentCopy } from '@element-plus/icons-vue'
import { ElMessage } from 'element-plus'
import { getQuerySettings } from '../../utils/settings'
import { noSqlDocuments, cancelNoSql } from '../../api'
import { t } from '../../utils/i18n'
import GridContextMenu from './GridContextMenu.vue'

const props = defineProps({ conn: Object, database: String, collection: String, kind: String })

const isRedisAllKeys = computed(() => props.kind === 'redis-db' || (props.collection === '*' && props.conn?.type === 'REDIS'))

const querySettings = getQuerySettings()

const keyword = ref('')
const page = ref(1)
const size = ref(querySettings.pageSize || 200)
const loading = ref(false)
const result = ref({ rows: [], columns: [], message: '', notices: [], executeTime: 0 })
const total = ref(0)
const gridRef = ref(null)
const elapsedTime = ref(0)
let queryTimer = null

const formatElapsed = (ms) => {
  if (ms < 1000) return `${ms}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

const pageSizes = computed(() => {
  const base = [50, 100, 200, 500, 1000]
  if (querySettings.pageSize > 0 && !base.includes(querySettings.pageSize)) base.unshift(querySettings.pageSize)
  return base.sort((a, b) => a - b)
})

const columns = computed(() => result.value.columns || [])

// 单元格文本：原模板对同一格要算 2~3 次（class / title / 内容），嵌套文档还要每次都 JSON.stringify。
// 这里统一成一个函数，并用 WeakMap 缓存序列化结果（row 对象被换掉后缓存自动回收，且不写入响应式对象、不触发额外依赖）。
const jsonCache = new WeakMap()
const isJsonCell = (row, col) => {
  const v = row[col]
  return v !== null && v !== undefined && typeof v === 'object'
}
const cellText = (row, col) => {
  const v = row[col]
  if (v === null || v === undefined) return 'NULL'
  if (typeof v === 'object') {
    let m = jsonCache.get(row)
    if (!m) { m = new Map(); jsonCache.set(row, m) }
    if (!m.has(col)) m.set(col, JSON.stringify(v))
    return m.get(col)
  }
  return String(v)
}

// ========== 窗口化渲染（固定行高）：避免上千行 × 多列产生大量 DOM 节点 ==========
// 思路与 TableDataView / SqlQueryView 一致：只渲染可视区附近的若干行，
// 上下用占位行撑起剩余高度。单元格 nowrap 保证每行等高，故可用固定行高定位。
const VP_ROW_H = 32        // 单行高度（px）：td padding 6px + 12px 字号，单元格 nowrap 保证等高
const VP_BUFFER = 12       // 可视区上下各多渲染的缓冲行
const VP_THRESHOLD = 200   // 行数超过该值才启用窗口化（默认每页 200，小表不虚拟化）
const vpStart = ref(0)
const vpEnd = ref(0)
let vpRaf = 0
const virtualEnabled = computed(() => (result.value?.rows?.length || 0) > VP_THRESHOLD)
const visibleRows = computed(() => {
  const all = result.value?.rows || []
  if (!virtualEnabled.value) return all
  const start = Math.min(vpStart.value, Math.max(0, all.length - 1))
  const end = Math.max(start, Math.min(vpEnd.value, all.length))
  return all.slice(start, end)
})
const padTop = computed(() =>
  virtualEnabled.value ? Math.min(vpStart.value, result.value.rows.length) * VP_ROW_H : 0)
const padBottom = computed(() => {
  if (!virtualEnabled.value) return 0
  return Math.max(0, result.value.rows.length - Math.min(vpEnd.value, result.value.rows.length)) * VP_ROW_H
})
// 实际承载滚动的容器（.data-table-wrap 自身 overflow:auto）
const scrollHost = () => gridRef.value?.querySelector('.data-table-wrap')
const syncViewport = () => {
  const all = result.value?.rows || []
  const total = all.length
  if (!total) { vpStart.value = 0; vpEnd.value = 0; return }
  if (!virtualEnabled.value) { vpStart.value = 0; vpEnd.value = total; return }
  const host = scrollHost()
  const s = host ? host.scrollTop : 0
  const clientH = (host && host.clientHeight) || 1
  const first = Math.max(0, Math.floor(s / VP_ROW_H) - VP_BUFFER)
  const count = Math.ceil(clientH / VP_ROW_H) + VP_BUFFER * 2
  vpStart.value = Math.min(first, Math.max(0, total - 1))
  vpEnd.value = Math.min(total, vpStart.value + count)
}
const onTableScroll = () => {
  if (vpRaf) return
  vpRaf = requestAnimationFrame(() => { vpRaf = 0; syncViewport() })
}

const effectiveKeyword = computed(() => {
  if (isRedisAllKeys.value && !keyword.value) return undefined
  return keyword.value || undefined
})

let cancelController = null
let execId = null
let stopRequested = false

const load = async (p) => {
  if (!props.conn) return
  page.value = p || 1
  loading.value = true
  stopRequested = false
  execId = 'n_' + Date.now() + '_' + Math.random().toString(36).slice(2, 8)
  cancelController = new AbortController()
  const queryStart = Date.now()
  elapsedTime.value = 0
  if (queryTimer) clearInterval(queryTimer)
  queryTimer = setInterval(() => { elapsedTime.value = Date.now() - queryStart }, 100)
  try {
    const res = await noSqlDocuments(props.conn.id, {
      database: props.database,
      collection: props.collection,
      page: page.value, size: size.value, keyword: effectiveKeyword.value,
      executionId: execId
    }, cancelController.signal)
    if (stopRequested) {
      result.value = { rows: [], columns: [], message: t('nsql.canceled'), notices: [], success: false, executeTime: elapsedTime.value }
      total.value = 0
      return
    }
    result.value = res
    // 查询返回后回到顶部并重置可视窗口（窗口化渲染依赖滚动位置）
    vpStart.value = 0
    vpEnd.value = Math.min(virtualEnabled.value ? 1000 : (res.rows?.length || 0), res.rows?.length || 0)
    if (typeof res.totalCount === 'number' && res.totalCount >= 0) {
      total.value = res.totalCount
    } else {
      total.value = parseInt(res.notices?.[0]?.split('=')[1] || res.rows?.length || 0, 10)
    }
    nextTick(() => {
      const host = scrollHost()
      if (host) host.scrollTop = 0
      syncViewport()
    })
  } catch (e) {
    if (stopRequested) {
      result.value = { rows: [], columns: [], message: t('nsql.canceled'), notices: [], success: false, executeTime: elapsedTime.value }
      total.value = 0
      return
    }
    ElMessage.error(t('nsql.queryFailed', { detail: e.message }))
  } finally {
    if (queryTimer) { clearInterval(queryTimer); queryTimer = null }
    loading.value = false
    cancelController = null
    execId = null
  }
}

const stop = () => {
  if (!loading.value) return
  stopRequested = true
  const id = execId
  if (id) cancelNoSql(id).catch(() => {})
  if (cancelController) cancelController.abort()
  loading.value = false
}

watch(() => [props.conn?.id, props.database, props.collection], () => { page.value = 1; load(1) }, { immediate: true })

// ========== 列宽拖拽 ==========
const colWidths = ref({})
const colResizing = ref(false)
let drag = null
let lastHover = null
let rafId = null

const defaultColWidth = (name) => Math.min(480, String(name || '').length * 15 + 90)

// 列宽的**唯一一条口径**：按「表头 + 前 N 行」的真实文本宽算，clamp 到 [MIN, MAX]。
// 与 SQL 结果表格一致 —— **不刻意撑满容器**（右边留白）。
// 原来这里按比例把余量摊回各列（"让表格始终沾满容器宽度"），于是列宽永远满屏、
// 拖窄了还会被撑回去，手感与 SQL 结果表格不一致（用户要求"跟 sql 查询结果保持一致"）。
const MIN_COL_WIDTH = 60
const MAX_COL_WIDTH = 480
const naturalColWidth = (ci, limit) => {
  const wrap = gridRef.value?.querySelector('.data-table-wrap')
  const ths = wrap?.querySelectorAll('thead th') || []
  const sampleRows = wrap?.querySelectorAll('tbody tr') || []
  const sample = Math.min(sampleRows.length, limit)
  let max = ths[ci] ? ths[ci].scrollWidth : 0
  for (let r = 0; r < sample; r++) {
    const td = sampleRows[r]?.querySelectorAll('td')[ci]
    if (td) max = Math.max(max, td.scrollWidth)
  }
  const natural = max > 0 ? max + 26 : defaultColWidth(columns.value[ci])
  return Math.max(MIN_COL_WIDTH, Math.min(MAX_COL_WIDTH, Math.round(natural)))
}

const measureColumns = () => {
  const wrap = gridRef.value?.querySelector('.data-table-wrap')
  const count = columns.value.length
  if (!wrap || !count) return
  const next = {}
  for (let i = 0; i < count; i++) next[i] = naturalColWidth(i, 10)
  colWidths.value = next
}

// ========== 总宽 / 双击自适应 / 右键菜单（对齐 SQL 结果表格） ==========
// 总宽 = 各列宽之和，用内联 width 给出：table-layout:fixed 下不给的话，
// 浏览器按内容算 max-content 再把余量摊回各列 —— 列会被撑回去，拖不窄。
const tableWidth = computed(() => {
  let w = 0
  columns.value.forEach((col, ci) => { w += colWidths.value[ci] || defaultColWidth(col) })
  return Math.round(w)
})

/** 单列自适应：量这一列（表头 + 前 30 行）的文本宽。双击列缘与右键「列宽自适应」共用。 */
const autoFitCol = (ci) => {
  if (ci < 0 || ci >= columns.value.length) return
  colWidths.value = { ...colWidths.value, [ci]: naturalColWidth(ci, 30) }
}
const onTableDblClick = (e) => {
  const ci = edgeColIdx(e)
  if (ci < 0) return
  e.preventDefault()
  autoFitCol(ci)
}

// ========== 选区（行 / 列 / 单元格）：与 SQL 结果表格同一套语义 ==========
// 单击格 = 单选该格；按住拖 = 框选；点行号 = 选整行（拖 = 连选）；点表头 = 选整列（Ctrl = 加选）。
// 没有选区时，「复制 / 导出」默认作用于**全部**数据 —— 与 SQL 结果表格的口径一致。
const tableWrapRef = ref(null)
const selectedRows = ref(new Set())
const selectedCols = ref(new Set())
const activeCell = ref(null)
let cellDrag = null

const clearSelection = () => {
  selectedRows.value = new Set()
  selectedCols.value = new Set()
  activeCell.value = null
}
const isActiveCell = (rowIdx, col) =>
  !!activeCell.value && activeCell.value.row === rowIdx && activeCell.value.col === col
const selectionText = computed(() => {
  const rows = selectedRows.value.size
  const cols = selectedCols.value.size
  if (!rows && !cols) return t('nsql.noSelection')
  return t('nsql.selectionInfo', { rows: (rows || (result.value?.rows?.length || 0)), cols: (cols || columns.value.length) })
})
/** 当前选区解析成「行下标 + 列名」；没选就取全部。 */
const scopeOf = (fallbackRow, fallbackCol) => {
  const all = result.value?.rows || []
  let rowIdx = selectedRows.value.size
    ? [...selectedRows.value].filter(i => i >= 0 && i < all.length).sort((a, b) => a - b)
    : []
  let cols = selectedCols.value.size
    ? columns.value.filter(c => selectedCols.value.has(c))
    : []
  // 只有「单个单元格」被点中时，就复制那一格（与 SQL 结果表格一致）
  if (!rowIdx.length && !cols.length && fallbackRow != null && fallbackRow >= 0 && fallbackCol) {
    rowIdx = [fallbackRow]
    cols = [fallbackCol]
  }
  if (!rowIdx.length) rowIdx = all.map((_, i) => i)
  if (!cols.length) cols = columns.value.slice()
  return { rowIdx, cols, all }
}

const onCellDown = (rowIdx, col, e) => {
  if (e.button !== 0) return
  activeCell.value = { row: rowIdx, col }
  if (e.shiftKey || e.ctrlKey || e.metaKey) {
    const rows = new Set(selectedRows.value)
    const cols = new Set(selectedCols.value)
    rows.has(rowIdx) ? rows.delete(rowIdx) : rows.add(rowIdx)
    cols.has(col) ? cols.delete(col) : cols.add(col)
    selectedRows.value = rows
    selectedCols.value = cols
    return
  }
  cellDrag = { r0: rowIdx, c0: col, moved: false }
  selectedRows.value = new Set([rowIdx])
  selectedCols.value = new Set([col])
  const onUp = () => {
    document.removeEventListener('mouseup', onUp)
    cellDrag = null
  }
  document.addEventListener('mouseup', onUp)
}
const onCellEnter = (rowIdx, col) => {
  if (!cellDrag) return
  cellDrag.moved = true
  const r0 = Math.min(cellDrag.r0, rowIdx)
  const r1 = Math.max(cellDrag.r0, rowIdx)
  const i0 = columns.value.indexOf(cellDrag.c0)
  const i1 = columns.value.indexOf(col)
  const rows = new Set()
  for (let r = r0; r <= r1; r++) rows.add(r)
  const cols = new Set()
  for (let i = Math.min(i0, i1); i <= Math.max(i0, i1); i++) cols.add(columns.value[i])
  selectedRows.value = rows
  selectedCols.value = cols
}
/** 行号列：单击选整行、按住拖连选（Ctrl 加选）。 */
const onRowNumDown = (rowIdx, e) => {
  activeCell.value = null
  if (e.ctrlKey || e.metaKey) {
    const rows = new Set(selectedRows.value)
    rows.has(rowIdx) ? rows.delete(rowIdx) : rows.add(rowIdx)
    selectedRows.value = rows
    return
  }
  const drag = { r0: rowIdx }
  selectedRows.value = new Set([rowIdx])
  selectedCols.value = new Set()
  const onMove = (ev) => {
    const el = document.elementFromPoint(ev.clientX, ev.clientY)
    const td = el && el.closest ? el.closest('.row-sel-td') : null
    const host = td ? td.closest('tbody') : null
    if (!td || !host) return
    const all = [...host.querySelectorAll('.row-sel-td')]
    const idx = all.indexOf(td)
    const start = all.findIndex(x => x === td)
    if (start < 0) return
    // 用行号文本反推行下标，避免窗口化下 DOM 顺序与行号不一致
    const text = td.querySelector('.row-num-tx')?.textContent || ''
    const abs = parseInt(text, 10) - 1
    if (!Number.isFinite(abs) || abs < 0) return
    const rows = new Set()
    const from = Math.min(drag.r0, abs)
    const to = Math.max(drag.r0, abs)
    for (let r = from; r <= to; r++) rows.add(r)
    selectedRows.value = rows
  }
  const onUp = () => {
    document.removeEventListener('mousemove', onMove)
    document.removeEventListener('mouseup', onUp)
  }
  document.addEventListener('mousemove', onMove)
  document.addEventListener('mouseup', onUp)
}
const onRowNumEnter = (rowIdx) => {
  // 拖动连选由 onRowNumDown 里的 document 监听处理（窗口化时行是动态的）
}
const onHeaderClick = (col, e) => {
  activeCell.value = null
  if ((e && (e.ctrlKey || e.metaKey))) {
    const cols = new Set(selectedCols.value)
    cols.has(col) ? cols.delete(col) : cols.add(col)
    selectedCols.value = cols
    return
  }
  selectedCols.value = new Set([col])
  selectedRows.value = new Set()
}

// ========== 复制 / 导出 ==========
const cellValue = (row, col) => {
  const v = row ? row[col] : null
  return v == null ? '' : String(v)
}
const copyText = async (text, tip) => {
  if (!text) return
  try {
    if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable')
    await navigator.clipboard.writeText(text)
    ElMessage.success(tip || t('sqlq.copied'))
  } catch {
    ElMessage.error(t('sqlq.copyFailed'))
  }
}
const csvCell = (v) => {
  const s = String(v ?? '')
  return /[",\n]/.test(s) ? '"' + s.replace(/"/g, '""') + '"' : s
}
/** 把当前选区渲染成各种格式（口径与 SQL 结果表格的「复制为」一致）。 */
const selectionAs = (format, fallbackRow, fallbackCol) => {
  const { rowIdx, cols, all } = scopeOf(fallbackRow, fallbackCol)
  const cell = (r, c) => cellValue(all[r], c)
  if (format === 'json') {
    return JSON.stringify(rowIdx.map(r => {
      const o = {}
      for (const c of cols) o[c] = all[r] ? all[r][c] : null
      return o
    }), null, 2)
  }
  if (format === 'markdown') {
    const head = '| ' + cols.join(' | ') + ' |'
    const sep = '| ' + cols.map(() => '---').join(' | ') + ' |'
    const body = rowIdx.map(r => '| ' + cols.map(c => cell(r, c).replace(/\|/g, '\\|')).join(' | ') + ' |')
    return [head, sep, ...body].join('\n')
  }
  if (format === 'csv') {
    return [cols.map(csvCell).join(',')]
      .concat(rowIdx.map(r => cols.map(c => csvCell(cell(r, c))).join(',')))
      .join('\n')
  }
  if (format === 'redis') {
    // Redis 场景最实用的写法：把 key/value 两列还原成可粘贴执行的命令
    const ki = cols.findIndex(c => /^key$/i.test(c))
    const vi = cols.findIndex(c => /^value$/i.test(c))
    if (ki >= 0 && vi >= 0) {
      return rowIdx.map(r => {
        const k = cell(r, cols[ki])
        const v = cell(r, cols[vi])
        return v === '' ? 'DEL ' + k : 'SET ' + k + ' ' + JSON.stringify(v)
      }).join('\n')
    }
    // 没有 key/value 列（Mongo / ES 的文档视图）：退回制表符
  }
  return [cols.join('\t')]
    .concat(rowIdx.map(r => cols.map(c => cell(r, c)).join('\t')))
    .join('\n')
}
const copySelection = (format, fallbackRow, fallbackCol) => {
  const text = selectionAs(format, fallbackRow, fallbackCol)
  const tip = format === 'tsv' ? t('sqlq.copied') : t('nsql.copiedAs', { fmt: format.toUpperCase() })
  return copyText(text, tip)
}
/** 导出当前选区为 CSV 文件（本地生成，不经过后端）。 */
const exportCsv = () => {
  const text = selectionAs('csv')
  if (!text) return
  const blob = new Blob(['\ufeff' + text], { type: 'text/csv;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')
  a.href = url
  a.download = (props.collection === '*' ? (props.database || 'redis') : props.collection) + '-' + stamp + '.csv'
  document.body.appendChild(a)
  a.click()
  document.body.removeChild(a)
  setTimeout(() => URL.revokeObjectURL(url), 1000)
  ElMessage.success(t('nsql.exportedCsv'))
}
/** 行详情：把这一行按「列名：值」列出（双击行号 / 右键）。 */
const rowDetail = ref({ visible: false, title: '', text: '' })
const openRowDetail = (rowIdx) => {
  const all = result.value?.rows || []
  const row = all[rowIdx]
  if (!row) return
  const text = columns.value.map(c => t('sqlq.cellPair', { col: c, val: (row[c] == null ? 'NULL' : String(row[c])) })).join('\n')
  rowDetail.value = { visible: true, title: t('sqlq.rowDetailTitle', { n: rowIdx + 1 }), text }
}
/** 表格键盘：Ctrl+C 复制选区、Ctrl+A 全选（与 SQL 结果表格一致）。 */
const onGridKeydown = (e) => {
  if ((e.ctrlKey || e.metaKey) && (e.key === 'c' || e.key === 'C')) {
    e.preventDefault()
    copySelection('tsv')
  } else if ((e.ctrlKey || e.metaKey) && (e.key === 'a' || e.key === 'A')) {
    e.preventDefault()
    selectedRows.value = new Set((result.value?.rows || []).map((_, i) => i))
    selectedCols.value = new Set(columns.value)
  } else if (e.key === 'Escape') {
    clearSelection()
  }
}

// ========== 右键菜单（复用 SQL 结果表格的 GridContextMenu.vue） ==========
const ctx = ref({ visible: false, x: 0, y: 0, items: [], row: -1, col: '' })
const openCtx = (e, row, col, items) => {
  ctx.value = { visible: true, x: e.clientX, y: e.clientY, items, row, col }
}
// 用函数而不是模块级常量：常量只在模块加载时求值一次，切换语言后菜单文案不会跟着变
const copyAsMenu = () => ({
  label: t('sqlq.ctxCopyAs'),
  key: 'copy-as',
  children: [
    { label: t('nsql.menuTsv'), key: 'copy-as-tsv' },
    { label: 'CSV', key: 'copy-as-csv' },
    { label: 'JSON', key: 'copy-as-json' },
    { label: t('nsql.menuMarkdown'), key: 'copy-as-markdown' },
    { label: t('nsql.menuRedis'), key: 'copy-as-redis' }
  ]
})
const onGridContextMenu = (e) => openCtx(e, -1, '', [
  { label: t('common.copy'), key: 'copy-sel', shortcut: 'Ctrl+C' },
    copyAsMenu(),
  { label: t('common.selectAll'), key: 'select-all', shortcut: 'Ctrl+A' },
  { divided: true },
  { label: t('nsql.menuExportCsv'), key: 'export-csv' }
])
const onHeaderContextMenu = (e, col) => openCtx(e, -1, col, [
  { label: t('sqlq.ctxColFit'), key: 'col-fit' },
  { divided: true },
  { label: t('nsql.menuCopyHeader'), key: 'copy-header' },
  { label: t('nsql.menuCopyCol'), key: 'copy-col' },
  { label: t('nsql.menuSelectCol'), key: 'select-col' }
])
const onRowContextMenu = (e, rowIdx) => openCtx(e, rowIdx, '', [
  { label: t('nsql.menuRowDetail'), key: 'row-detail' },
  { divided: true },
  { label: t('nsql.menuCopyRow'), key: 'copy-row' },
  { label: t('nsql.menuSelectRow'), key: 'select-row' },
  { divided: true },
  COPY_AS
])
const onCellContextMenu = (e, rowIdx, col) => openCtx(e, rowIdx, col, [
  { label: t('common.copy'), key: 'copy-cell', shortcut: 'Ctrl+C' },
    copyAsMenu(),
  { divided: true },
  { label: t('nsql.menuCopyRow'), key: 'copy-row' },
  { label: t('nsql.menuCopyHeader'), key: 'copy-header' },
  { label: t('nsql.menuSelectRow'), key: 'select-row' },
  { label: t('nsql.menuSelectCol'), key: 'select-col' },
  { divided: true },
  { label: t('nsql.menuExportCsv'), key: 'export-csv' }
])
const onCtxSelect = (key) => {
  const rows = result.value?.rows || []
  const { row, col } = ctx.value
  const record = row >= 0 ? rows[row] : null
  if (key.startsWith('copy-as-')) return copySelection(key.slice('copy-as-'.length), row, col)
  if (key === 'copy-cell') return copyText(cellValue(record, col), t('sqlq.copyCell'))
  if (key === 'copy-sel') return copySelection('tsv', row, col)
  if (key === 'copy-row') return copyText(columns.value.map(c => cellValue(record, c)).join('\t'), t('nsql.copiedRow'))
  if (key === 'copy-header') return copyText(col, t('sqlq.copyColNames'))
  if (key === 'copy-col') return copyText([col].concat(rows.map(r => cellValue(r, col))).join('\n'), t('nsql.copiedCol'))
  if (key === 'export-csv') return exportCsv()
  if (key === 'col-fit') return autoFitCol(columns.value.indexOf(col))
  if (key === 'row-detail') return openRowDetail(row)
  if (key === 'select-row') {
    selectedRows.value = new Set([row])
    return
  }
  if (key === 'select-col') {
    selectedCols.value = new Set([col])
    return
  }
  if (key === 'select-all') {
    selectedRows.value = new Set(rows.map((_, i) => i))
    selectedCols.value = new Set(columns.value)
  }
}

watch(() => columns.value.join('\u0001'), async () => {
  colWidths.value = {}
  if (result.value?.rows?.length) {
    await nextTick()
    measureColumns()
  }
})

const edgeColIdx = (e) => {
  const cell = e.target.closest('th, td')
  if (!cell || !cell.closest('table')) return -1
  const rect = cell.getBoundingClientRect()
  const x = e.clientX
  const cols = columns.value
  let ci = -1
  if (x >= rect.right - 10 && x <= rect.right + 8) ci = cell.cellIndex
  else if (cell.cellIndex > 0 && x >= rect.left - 8 && x <= rect.left + 10) ci = cell.cellIndex - 1
  if (ci < 0 || ci >= cols.length) return -1
  return ci
}

const onTableMove = (e) => {
  const wrap = gridRef.value?.querySelector('.data-table-wrap')
  if (!wrap) return
  if (drag) { wrap.style.cursor = 'col-resize'; return }
  const cell = e.target.closest('th, td')
  if (lastHover && lastHover !== cell) {
    lastHover.style.cursor = ''
  }
  const ci = edgeColIdx(e)
  if (ci >= 0 && cell) {
    cell.style.cursor = 'col-resize'
    lastHover = cell
  } else {
    lastHover = null
  }
  wrap.style.cursor = ci >= 0 ? 'col-resize' : ''
}

const onTableLeave = () => {
  const wrap = gridRef.value?.querySelector('.data-table-wrap')
  if (!wrap) return
  if (!drag) wrap.style.cursor = ''
  if (lastHover) { lastHover.style.cursor = ''; lastHover = null }
}

const onTableDown = (e) => {
  if (e.button !== 0) return
  const ci = edgeColIdx(e)
  if (ci < 0) return
  e.preventDefault()
  const wrap = gridRef.value?.querySelector('.data-table-wrap')
  if (!Object.keys(colWidths.value).length) measureColumns()
  const table = wrap?.querySelector('table')
  const colEls = table?.querySelectorAll('colgroup col')
  const colEl = colEls?.[ci] || null
  if (colEl) colEl.style.willChange = 'width'
  drag = {
    ci,
    startW: colWidths.value[ci] || defaultColWidth(columns.value[ci]),
    currentW: colWidths.value[ci] || defaultColWidth(columns.value[ci]),
    colEl
  }
  colResizing.value = true
  if (wrap) { wrap.classList.add('col-resizing'); wrap.style.cursor = 'col-resize' }
  document.addEventListener('mousemove', onDragMove)
  document.addEventListener('mouseup', onDragEnd)
}

const onDragMove = (e) => {
  if (!drag) return
  const w = Math.max(0, drag.currentW + e.movementX)
  drag.currentW = w
  if (rafId) cancelAnimationFrame(rafId)
  rafId = requestAnimationFrame(() => {
    if (drag?.colEl) drag.colEl.style.width = w + 'px'
  })
}

const onDragEnd = () => {
  if (drag) {
    const finalW = Math.round(Math.max(0, drag.currentW))
    colWidths.value = { ...colWidths.value, [drag.ci]: finalW }
    if (drag.colEl) drag.colEl.style.willChange = ''
  }
  if (rafId) { cancelAnimationFrame(rafId); rafId = null }
  drag = null
  colResizing.value = false
  const wrap = gridRef.value?.querySelector('.data-table-wrap')
  if (wrap) {
    wrap.classList.remove('col-resizing')
    wrap.style.cursor = ''
    if (lastHover) { lastHover.style.cursor = ''; lastHover = null }
  }
  document.removeEventListener('mousemove', onDragMove)
  document.removeEventListener('mouseup', onDragEnd)
}

onBeforeUnmount(() => {
  if (queryTimer) { clearInterval(queryTimer); queryTimer = null }
  stop() // 卸载时中止进行中的文档查询并取消后端任务
  onDragEnd()
})
</script>

<style scoped>
.nosql-view { height: 100%; display: flex; flex-direction: column; padding: 8px; }
/* ===== 选区样式（对齐 SQL 结果表格）：行号列 / 选中列 / 当前格 ===== */
.row-sel-col { width: 40px; }
.row-sel-th, .row-sel-td {
  width: 40px; text-align: center; user-select: none;
  color: var(--dc-text-dim); font-size: 12px;
}
.row-sel-td { cursor: pointer; }
.row-sel-td:hover, .row-sel-th:hover { background: var(--dc-bg-soft); }
.data-table tr.selected .row-sel-td { color: var(--dc-primary); font-weight: 600; }
.data-table td.col-selected { background: color-mix(in srgb, var(--dc-primary) 8%, transparent); }
.data-table th.col-selected { color: var(--dc-primary); }
.data-table td.active-cell { outline: 2px solid var(--dc-primary); outline-offset: -2px; }
.sel-info { font-size: 12px; color: var(--dc-text-dim); }
.row-detail-pre {
  margin: 0; max-height: 50vh; overflow: auto; white-space: pre-wrap; word-break: break-all;
  font-family: var(--dc-mono-font); font-size: 12.5px; line-height: 1.6;
}
.toolbar { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; }
.spacer { flex: 1; }
.loading-text { display: flex; align-items: center; gap: 6px; color: var(--dc-primary); font-size: 13px; }
.info { font-size: 13px; color: var(--dc-text-dim); }
.has-more-alert { margin-bottom: 6px; }
.table-wrap { flex: 1; min-height: 0; background: var(--dc-bg-card); border: 1px solid var(--dc-border); border-radius: var(--dc-radius); overflow: hidden; display: flex; flex-direction: column; }
.pager { display: flex; align-items: center; justify-content: space-between; padding: 6px 12px; border-top: 1px solid var(--dc-border); background: var(--dc-bg-soft); flex-shrink: 0; gap: 12px; }
.result-time { font-size: 13px; color: var(--dc-text-dim); font-weight: 500; }
.json-cell { font-family: var(--dc-mono-font, monospace); font-size: 13px; color: #5aa0d8; }

/* 原生表格样式 —— 与 SqlQueryView 保持一致 */
.data-table-wrap { flex: 1; min-height: 0; overflow: auto; contain: layout paint; }
.data-table-wrap.col-resizing, .data-table-wrap.col-resizing * { cursor: col-resize !important; user-select: none; }
.data-table { position: relative; width: 100%; table-layout: fixed; border-collapse: collapse; font-size: 13px; }
.data-table thead { position: sticky; top: 0; z-index: 2; }
.data-table th { background: var(--dc-bg-table-head); color: var(--dc-text-strong); font-weight: 600; text-align: left; padding: 8px 10px; border: 1px solid var(--dc-border); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.data-table td { padding: 6px 10px; border: 1px solid var(--dc-border); color: var(--dc-text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.data-table tbody tr.row-alt td { background: var(--dc-bg-soft); }
.data-table tbody tr:hover td { background: var(--dc-primary-wash); }
/* 窗口化占位行：透明、无边框，且不被 hover 着色 */
.data-table tbody tr.vp-pad-row td { background: transparent !important; border: none; padding: 0; }
.data-table tbody tr.vp-pad-row:hover td { background: transparent !important; }
.data-table .fill-col { padding: 0; border: none; background: transparent !important; min-width: 1px; }
.null-cell { color: var(--dc-text-dim); font-style: italic; }
</style>
