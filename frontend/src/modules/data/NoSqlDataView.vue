<template>
  <div class="nosql-view" ref="rootRef">
    <!-- 工具栏：与表预览（TableDataView）同一套布局 —— 左「高级搜索」，右「列显隐 / 导出 / 刷新」 -->
    <div class="toolbar">
      <div class="left">
        <el-button size="small" :icon="advancedOpen ? ArrowUp : ArrowDown" plain
                   @click="advancedOpen = !advancedOpen">{{ $t('tdv.advancedSearch') }}</el-button>
      </div>
      <div class="right">
        <el-dropdown trigger="click" :hide-on-click="false" popper-class="col-vis-dropdown">
          <el-button size="small" text :icon="Operation" :title="$t('sqlq.visibleColsTitle', { shown: visibleColumns.length, total: columns.length })" />
          <template #dropdown>
            <div class="col-vis" @mousedown.stop>
              <div class="col-vis-head">
                <span>{{ $t('sqlq.visibleCols') }}</span>
                <el-button size="small" text type="primary" @click="showAllColumns">{{ $t('common.selectAll') }}</el-button>
              </div>
              <el-checkbox v-for="c in columns" :key="c" :model-value="!hiddenColumns.has(c)"
                           @change="toggleColumnVisible(c)" class="col-vis-item">{{ c }}</el-checkbox>
            </div>
          </template>
        </el-dropdown>
        <el-dropdown trigger="click" @command="onExportCmd">
          <el-button size="small" text :icon="Download" :title="$t('qa.exportBtn')" />
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item command="csv">{{ $t('qa.exportCurCsv') }}</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
        <el-button size="small" text :icon="Refresh" :title="$t('vf.refresh')" @click="load(page)" />
      </div>
    </div>

    <!-- 高级搜索面板：NoSQL 只有关键词一个过滤维度，外观与表预览的筛选面板一致 -->
    <transition name="slide">
      <div v-show="advancedOpen" class="advanced-panel">
        <div class="filter-row">
          <span class="filter-label">{{ $t('nsql.keyword') }}</span>
          <el-input v-model="keyword" clearable size="small" style="width: 260px"
                    :placeholder="$t('nsql.keywordPlaceholder')"
                    @keyup.enter="load(1)" @clear="load(1)" />
          <el-button type="primary" size="small" @click="load(1)">{{ $t('nsql.query') }}</el-button>
          <el-button size="small" @click="resetKeyword">{{ $t('common.clear') }}</el-button>
        </div>
      </div>
    </transition>

    <el-alert v-if="result.hasMore" type="warning" :closable="false" show-icon class="has-more-alert">
      <template #title>{{ $t('nsql.partialTitle', { n: rows.length }) }}</template>
    </el-alert>
    <div class="grid-area" ref="gridRef">
      <!-- 加载遮罩（含取消）：与表预览同一套 -->
      <div v-if="loading" class="grid-loading-overlay">
        <div class="grid-loading-box">
          <el-icon class="is-loading" :size="26"><Loading /></el-icon>
          <span class="grid-loading-text">{{ $t('nsql.loading') }}</span>
          <el-button size="small" @click="stop">{{ $t('nsql.stop') }}</el-button>
        </div>
      </div>
      <div v-if="displayRows.length" class="data-table-wrap" ref="tableWrapRef" tabindex="0"
           @scroll="onTableScroll"
           @mousemove="onTableMove" @mousedown="onTableDown" @mouseleave="onTableLeave"
           @dblclick="onTableDblClick"
           @contextmenu.prevent="onGridContextMenu">
        <!-- 总宽用内联 width 显式给出（与表预览同一套）：
             table-layout:fixed 下不给的话，浏览器会把余量摊回各列 —— 列宽拖不窄 -->
        <table class="data-table" :class="{ 'col-resizing': colResizing }"
               :style="{ width: tableWidth + 'px' }">
          <colgroup>
            <col class="row-num-col" style="width: 40px" />
            <col v-for="col in visibleColumns" :key="'c' + col"
                 :style="{ width: (colWidths[col] || defaultColWidth(col)) + 'px' }" />
          </colgroup>
          <thead>
            <tr :class="{ 'selected': headerSelected, 'row-sel-top': headerSelected, 'row-sel-bottom': selEdges.headerBottom }">
              <!-- 左上角 = 标题行的行头：单击选中标题行，按住往下拖连选数据行（与表预览同一套） -->
              <th class="row-num-th leading-th" :class="{ 'row-num-on': headerSelected }"
                  :title="$t('sqlq.headerRowTitle')"
                  @mousedown.prevent="onHeaderRowDown($event)"><span class="row-num-tx">#</span></th>
              <th v-for="(col, ci) in visibleColumns" :key="'h' + col" :data-gkey="'0:' + ci"
                  :title="col + $t('nsql.colTitleSuffix')"
                  class="sortable"
                  :class="{ 'sort-asc': orderColumn === col && orderDir === 'ASC',
                            'sort-desc': orderColumn === col && orderDir === 'DESC',
                            'col-selected': selectedCols.has(col),
                            'col-sel-l': selEdges.colLeft.has(col),
                            'col-sel-r': selEdges.colRight.has(col) }"
                  @mousedown="onHeaderDown(col, $event)"
                  @click="onHeaderClick(col, $event)"
                  @dblclick="onHeaderDblClick(col, $event)"
                  @contextmenu.prevent.stop="onHeaderContextMenu($event, col)">
                <span class="th-text">
                  <span class="th-line1">
                    <span class="th-type-ic" :title="colTypeTitle(col)"><el-icon><component :is="typeIcon(col)" /></el-icon></span>
                    <span class="th-label">{{ col }}</span>
                  </span>
                </span>
                <!-- 排序（当前页内排序：NoSQL 文档接口不支持 ORDER BY） -->
                <span class="th-sort" :class="{ 'is-sorted': orderColumn === col }"
                      :title="orderColumn === col ? (orderDir === 'ASC' ? $t('sqlq.sortAscTitle') : $t('sqlq.sortDescTitle')) : $t('sqlq.sortNoneTitle')"
                      @mousedown.stop @click.stop="toggleSort(col)">
                  <el-icon v-if="orderColumn !== col"><Sort /></el-icon>
                  <el-icon v-else-if="orderDir === 'ASC'"><SortUp /></el-icon>
                  <el-icon v-else><SortDown /></el-icon>
                </span>
                <span class="col-resizer" :title="$t('tdv.colResizeTip')"
                      @mousedown.stop.prevent="startColResize(ci, $event)"
                      @dblclick.stop="autoFitCol(col)" />
              </th>
            </tr>
          </thead>
          <tbody>
            <!-- 窗口化渲染：上方占位行 -->
            <tr v-if="padTop > 0" class="vp-pad-row" aria-hidden="true">
              <td :colspan="visibleColumns.length + 1" :style="{ height: padTop + 'px' }" />
            </tr>
            <tr v-for="row in visibleRows" :key="rowKey(row)" :data-rid="row._rid"
                :class="{
                  'selected': isRowSelected(row),
                  'row-alt': rowIndex(row) % 2 === 1,
                  'row-sel-top': selEdges.rowTop.has(row._rid),
                  'row-sel-bottom': selEdges.rowBottom.has(row._rid),
                  'col-sel-bottom': row._rid === lastDisplayRid
                }">
              <!-- 行号列（Excel 行头）：单击选中、Ctrl 切换、Shift 连选、按住拖动连选；双击看整行详情 -->
              <td class="row-num-td leading-td" :class="{ 'row-num-on': isRowSelected(row) }"
                  :title="$t('nsql.rowNumTitle', { n: rowIndex(row) + 1 })"
                  @mousedown.prevent="onRowNumDown(row, $event)"
                  @dblclick.stop="openRowDetail(row)"
                  @contextmenu.prevent.stop="onRowContextMenu($event, row)">
                <span class="row-num-tx">{{ rowIndex(row) + 1 }}</span>
              </td>
              <td v-for="(col, ci) in visibleColumns" :key="'d' + col" :data-gkey="(rowIndex(row) + 1) + ':' + ci"
                  :class="{ 'null-cell': row[col] == null, 'col-selected': selectedCols.has(col),
                            'col-sel-l': selEdges.colLeft.has(col), 'col-sel-r': selEdges.colRight.has(col),
                            'active-cell': activeCell && activeCell.row === row && activeCell.col === col && noBulkSelection }"
                  :title="fmtVal(row[col])"
                  @click="onCellClick(row, col, $event)"
                  @dblclick="openCellDetail(row, col)"
                  @contextmenu.prevent.stop="onCellContextMenu($event, row, col)">
                <span v-if="isJsonCell(row, col)" class="json-cell">{{ fmtVal(row[col]) }}</span>
                <span v-else>{{ fmtVal(row[col]) }}</span>
              </td>
            </tr>
            <!-- 窗口化渲染：下方占位行 -->
            <tr v-if="padBottom > 0" class="vp-pad-row" aria-hidden="true">
              <td :colspan="visibleColumns.length + 1" :style="{ height: padBottom + 'px' }" />
            </tr>
          </tbody>
        </table>
      </div>
      <el-empty v-else-if="!loading" :description="isRedisAllKeys ? $t('nsql.emptyKeys') : $t('nsql.emptyData')" :image-size="80" />
      <!-- 底栏：耗时（左）+ 选中区汇总 + 分页（右），与表预览同一套 -->
      <div v-if="displayRows.length" class="pager">
        <span class="load-time">
          {{ loading ? formatElapsed(elapsedTime) : (result.executeTime ? formatElapsed(result.executeTime) : '') }}
        </span>
        <span v-if="selectionSummary" class="sel-summary" :title="$t('sqlq.summaryTitle')">
          <span class="ss-item">选中 <b>{{ selectionSummary.cells }}</b> 格</span>
          <template v-if="selectionSummary.nums">
            <span class="ss-item">{{ $t('sqlq.sum') }} <b>{{ fmtSummaryNum(selectionSummary.sum) }}</b></span>
            <span class="ss-item">{{ $t('sqlq.avg') }} <b>{{ fmtSummaryNum(selectionSummary.avg) }}</b></span>
            <span class="ss-item">{{ $t('sqlq.min') }} <b>{{ fmtSummaryNum(selectionSummary.min) }}</b></span>
            <span class="ss-item">{{ $t('sqlq.max') }} <b>{{ fmtSummaryNum(selectionSummary.max) }}</b></span>
          </template>
        </span>
        <el-pagination background size="small" layout="total, sizes, prev, pager, next, jumper" :total="total"
                       :page-size="size" :current-page="page" :page-sizes="pageSizes"
                       style="margin-left: auto"
                       @current-change="load" @size-change="s => { size = s; load(1) }" />
      </div>
    </div>

    <!-- 右键上下文菜单（teleport 到 body，支持二级子菜单）—— 与表预览同一个组件级实现 -->
    <teleport to="body">
      <div v-if="ctxMenu.visible" class="grid-ctx-menu" :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
           @contextmenu.prevent @mousedown.stop>
        <div v-for="(item, i) in ctxMenu.items" :key="i"
             :class="['ctx-item', { 'ctx-disabled': item.disabled, 'ctx-sep': item.sep, 'ctx-active': item.sub && ctxSub && ctxSub.parentIndex === i }]"
             @click="(item.sep || item.sub || item.disabled) ? null : onCtxItem(item)"
             @mouseenter="onCtxItemHover(item, i, $event)">
          <span class="ctx-label">{{ item.label }}</span>
          <span v-if="item.shortcut" class="ctx-shortcut">{{ item.shortcut }}</span>
          <span v-if="item.sub" class="ctx-arrow">›</span>
        </div>
      </div>
      <div v-if="ctxMenu.visible && ctxSub && ctxSub.items && ctxSub.items.length" class="grid-ctx-menu grid-ctx-sub"
           :style="{ left: ctxSub.x + 'px', top: ctxSub.y + 'px' }"
           @contextmenu.prevent @mousedown.stop>
        <div v-for="(item, i) in ctxSub.items" :key="i"
             :class="['ctx-item', { 'ctx-disabled': item.disabled, 'ctx-sep': item.sep }]"
             @click="(item.sep || item.disabled) ? null : onCtxItem(item)">
          <span class="ctx-label">{{ item.label }}</span>
          <span v-if="item.shortcut" class="ctx-shortcut">{{ item.shortcut }}</span>
        </div>
      </div>
    </teleport>

    <!-- 行 / 单元格详情：双击行号 / 双击单元格 / 右键（与表预览共用 CellDetailDialog） -->
    <CellDetailDialog v-model="detail.visible" :title="detail.title" :text="detail.text" />
  </div>
</template>

<script setup>
import { ref, computed, watch, nextTick, onMounted, onBeforeUnmount } from 'vue'
import { saveBlobAs } from '../../utils/useExportTask'
import { ArrowUp, ArrowDown, Refresh, Download, Operation, Sort, SortUp, SortDown, Loading,
         Histogram, Calendar, Document, Tickets, Grid, Key } from '@element-plus/icons-vue'
import { ElMessage } from 'element-plus'
import { getQuerySettings } from '../../utils/settings'
import { noSqlDocuments, cancelNoSql } from '../../api'
import { t } from '../../utils/i18n'
import { formatDbValue, nullDisplay } from '../../utils/cellValue'
import { useExcelSelection } from '../../utils/excelSelection'
import CellDetailDialog from '../../common/CellDetailDialog.vue'

const props = defineProps({ conn: Object, database: String, collection: String, kind: String })

const isRedisAllKeys = computed(() => props.kind === 'redis-db' || (props.collection === '*' && props.conn?.type === 'REDIS'))

const querySettings = getQuerySettings()

// ===== 工具栏 =====
const keyword = ref('')
const advancedOpen = ref(false)
const resetKeyword = () => { keyword.value = ''; load(1) }
const onExportCmd = (cmd) => { if (cmd === 'csv') exportCsv() }

const page = ref(1)
const size = ref(querySettings.pageSize || 200)
const loading = ref(false)
const result = ref({ rows: [], columns: [], message: '', notices: [], executeTime: 0 })
const total = ref(0)
const rootRef = ref(null)
const gridRef = ref(null)
const tableWrapRef = ref(null)
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

// ===== 列显隐（工具栏 ≡）：只影响展示，复制/导出仍按选区口径取数据 =====
const hiddenColumns = ref(new Set())
const visibleColumns = computed(() => columns.value.filter(c => !hiddenColumns.value.has(c)))
const toggleColumnVisible = (c) => {
  const next = new Set(hiddenColumns.value)
  next.has(c) ? next.delete(c) : next.add(c)
  if (columns.value.length - next.size < 1) return // 至少保留一列
  hiddenColumns.value = next
}
const showAllColumns = () => { hiddenColumns.value = new Set() }

// ===== 行身份（与表预览同款）：_rid 稳定 key + _idx 绝对下标，选区/键盘导航全走 O(1) =====
const rows = ref([]) // 当前行数据（带身份）；展示层是 displayRows（去掉手动隐藏的行）
const hiddenRows = ref(new Set())
const displayRows = computed(() => rows.value.filter(r => !hiddenRows.value.has(r._rid)))
let ridSeq = 0
const nextRid = () => 'r' + (++ridSeq)
const rowIndex = (row) => row._idx
const rowKey = (row) => row._rid
const stampRows = (list) => list.map((r, i) => ({ ...r, _rid: nextRid(), _idx: i }))
const rebuildRowIndex = () => { rows.value.forEach((r, i) => { r._idx = i }) }
// _rid → displayRows 下标：行号拖选时由 DOM 行反查（O(1)， formerly findIndex 扫全表）
const ridToDispIdx = computed(() => {
  const m = new Map()
  displayRows.value.forEach((r, i) => m.set(String(r._rid), i))
  return m
})

// 单元格展示文本：对象序列化为 JSON，NULL 走全局空值样式
const isJsonCell = (row, col) => {
  const v = row[col]
  return v !== null && v !== undefined && typeof v === 'object'
}
const fmtVal = (v) => {
  if (v === null || v === undefined) return nullDisplay()
  if (typeof v === 'object') return JSON.stringify(v)
  return String(formatDbValue(v))
}
// 复制/导出口径：NULL → 空串（与表预览的 TSV/CSV 一致）
const plainVal = (v) => {
  if (v === null || v === undefined) return ''
  if (typeof v === 'object') return JSON.stringify(v)
  return String(v)
}

// ===== 窗口化渲染（固定行高）=====
const VP_ROW_H = 32
const VP_BUFFER = 12
const VP_THRESHOLD = 200
const vpStart = ref(0)
const vpEnd = ref(0)
let vpRaf = 0
const virtualEnabled = computed(() => displayRows.value.length > VP_THRESHOLD)
const visibleRows = computed(() => {
  const all = displayRows.value
  if (!virtualEnabled.value) return all
  const start = Math.min(vpStart.value, Math.max(0, all.length - 1))
  const end = Math.max(start, Math.min(vpEnd.value, all.length))
  return all.slice(start, end)
})
const padTop = computed(() =>
  virtualEnabled.value ? Math.min(vpStart.value, displayRows.value.length) * VP_ROW_H : 0)
const padBottom = computed(() => {
  if (!virtualEnabled.value) return 0
  return Math.max(0, displayRows.value.length - Math.min(vpEnd.value, displayRows.value.length)) * VP_ROW_H
})
const scrollHost = () => {
  const w = tableWrapRef.value
  if (w && w.scrollHeight > w.clientHeight) return w
  return gridRef.value || w
}
const syncViewport = () => {
  const all = displayRows.value
  const n = all.length
  if (!n) { vpStart.value = 0; vpEnd.value = 0; return }
  if (!virtualEnabled.value) { vpStart.value = 0; vpEnd.value = n; return }
  const host = scrollHost()
  const s = host ? host.scrollTop : 0
  const clientH = (host && host.clientHeight) || 1
  const first = Math.max(0, Math.floor(s / VP_ROW_H) - VP_BUFFER)
  const count = Math.ceil(clientH / VP_ROW_H) + VP_BUFFER * 2
  vpStart.value = Math.min(first, Math.max(0, n - 1))
  vpEnd.value = Math.min(n, vpStart.value + count)
}
const onTableScroll = () => {
  if (vpRaf) return
  vpRaf = requestAnimationFrame(() => { vpRaf = 0; syncViewport() })
}

// ===== 排序（当前页内；NoSQL 文档接口不支持 ORDER BY）=====
const orderColumn = ref('')
const orderDir = ref('')
const rawRows = ref([]) // 载入时的原始行序快照：取消排序（第三击）时还原
const toggleSort = (col) => {
  if (orderColumn.value !== col) {
    orderColumn.value = col
    orderDir.value = 'ASC'
  } else if (orderDir.value === 'ASC') {
    orderDir.value = 'DESC'
  } else {
    orderColumn.value = ''
    orderDir.value = ''
  }
  applySort()
}
const applySort = () => {
  if (!orderColumn.value) {
    rows.value = rawRows.value.slice()
    rebuildRowIndex()
    return
  }
  const col = orderColumn.value
  const dir = orderDir.value === 'DESC' ? -1 : 1
  const plain = (v) => (v === null || v === undefined) ? '' : (typeof v === 'object' ? JSON.stringify(v) : v)
  rows.value.sort((a, b) => {
    const va = a[col], vb = b[col]
    if (va == null && vb == null) return 0
    if (va == null) return -dir
    if (vb == null) return dir
    if (typeof va === 'number' && typeof vb === 'number') return (va - vb) * dir
    return String(plain(va)).localeCompare(String(plain(vb)), undefined, { numeric: true }) * dir
  })
  rebuildRowIndex()
}

// ===== 表头类型图标：NoSQL 无列元数据，按列名 + 首个非空值推断（与表预览同一套图标语言）=====
const typeIcon = (col) => {
  if (/^key$/i.test(col)) return Key
  for (const r of rows.value) {
    const v = r[col]
    if (v === null || v === undefined || v === '') continue
    if (typeof v === 'number') return Histogram
    if (typeof v === 'object') return Tickets
    if (/^\d{4}-\d{2}-\d{2}/.test(String(v))) return Calendar
    return Document
  }
  return Grid
}
const colTypeTitle = (col) => {
  if (/^key$/i.test(col)) return 'key'
  for (const r of rows.value) {
    const v = r[col]
    if (v === null || v === undefined || v === '') continue
    if (typeof v === 'number') return 'number'
    if (typeof v === 'object') return 'json'
    if (/^\d{4}-\d{2}-\d{2}/.test(String(v))) return 'date'
    return 'string'
  }
  return ''
}

// ===== 查询 =====
const effectiveKeyword = computed(() => {
  if (isRedisAllKeys.value && !keyword.value) return undefined
  return keyword.value || undefined
})

let cancelController = null
let execId = null
let stopRequested = false

const clearAllSelections = () => {
  selectedSet.value = new Set()
  headerSelected.value = false
  clearColSelect()
  activeCell.value = null
  if (gridSel) gridSel.clear()
}

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
      rows.value = []
      total.value = 0
      return
    }
    result.value = res
    // 新数据到达：重打身份、重置排序与选区（排序是当前页内的）
    rows.value = stampRows(res.rows || [])
    hiddenRows.value = new Set()
    rawRows.value = rows.value.slice()
    orderColumn.value = ''
    orderDir.value = ''
    clearAllSelections()
    if (typeof res.totalCount === 'number' && res.totalCount >= 0) {
      total.value = res.totalCount
    } else {
      total.value = parseInt(res.notices?.[0]?.split('=')[1] || res.rows?.length || 0, 10)
    }
    vpStart.value = 0
    vpEnd.value = Math.min(virtualEnabled.value ? 1000 : (rows.value.length || 0), rows.value.length || 0)
    nextTick(() => {
      const host = scrollHost()
      if (host) host.scrollTop = 0
      syncViewport()
    })
  } catch (e) {
    if (stopRequested) {
      result.value = { rows: [], columns: [], message: t('nsql.canceled'), notices: [], success: false, executeTime: elapsedTime.value }
      rows.value = []
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

watch(() => [props.conn?.id, props.database, props.collection], () => {
  page.value = 1
  hiddenColumns.value = new Set()
  load(1)
}, { immediate: true })

// ===== 列宽拖拽（按列名存储；把手 / 列缘双通道，与表预览同款观感）=====
const colWidths = ref({})
const colResizing = ref(false)
let drag = null
let lastHover = null
let rafId = null
let lastResizeAt = 0

const defaultColWidth = (name) => Math.min(480, String(name || '').length * 15 + 90)

const MIN_COL_WIDTH = 60
const MAX_COL_WIDTH = 480
const naturalColWidth = (col, limit) => {
  const vi = visibleColumns.value.indexOf(col)
  if (vi < 0) return defaultColWidth(col)
  const wrap = tableWrapRef.value
  const ths = wrap?.querySelectorAll('thead th') || []
  const sampleRows = wrap?.querySelectorAll('tbody tr:not(.vp-pad-row)') || []
  const sample = Math.min(sampleRows.length, limit)
  // th/td 的第 0 个是行号列，数据列从 1 开始
  let max = ths[vi + 1] ? ths[vi + 1].scrollWidth : 0
  for (let r = 0; r < sample; r++) {
    const td = sampleRows[r]?.querySelectorAll('td')[vi + 1]
    if (td) max = Math.max(max, td.scrollWidth)
  }
  const natural = max > 0 ? max + 26 : defaultColWidth(col)
  return Math.max(MIN_COL_WIDTH, Math.min(MAX_COL_WIDTH, Math.round(natural)))
}

const measureColumns = () => {
  const wrap = tableWrapRef.value
  if (!wrap || !visibleColumns.value.length) return
  const next = {}
  for (const c of visibleColumns.value) next[c] = naturalColWidth(c, 10)
  colWidths.value = next
}

const tableWidth = computed(() => {
  let w = 0
  visibleColumns.value.forEach((col) => { w += colWidths.value[col] || defaultColWidth(col) })
  return Math.round(w)
})

const autoFitCol = (col) => {
  if (!visibleColumns.value.includes(col)) return
  colWidths.value = { ...colWidths.value, [col]: naturalColWidth(col, 30) }
}
// 选中多列时一起自适应（右键「列宽自适应」在多列选中时走这里）
const autoFitSelectedCols = () => {
  for (const c of visibleColumns.value) {
    if (selectedCols.value.has(c)) autoFitCol(c)
  }
}
const onHeaderDblClick = (col, e) => {
  const rect = e.currentTarget.getBoundingClientRect()
  if (e.clientX < rect.right - 12) return // 只有双击右缘才自适应，避免误触排序/选中
  autoFitCol(col)
}
const onTableDblClick = (e) => {
  const vi = edgeColIdx(e)
  if (vi < 0) return
  e.preventDefault()
  autoFitCol(visibleColumns.value[vi])
}

// 贴列缘检测：返回**可见数据列**下标（cellIndex 含行号列，减 1；贴左缘属于前一列）
const edgeColIdx = (e) => {
  const cell = e.target.closest('th, td')
  if (!cell || !cell.closest('table')) return -1
  const rect = cell.getBoundingClientRect()
  const x = e.clientX
  let vi = -1
  if (x >= rect.right - 10 && x <= rect.right + 8) vi = cell.cellIndex - 1
  else if (cell.cellIndex > 1 && x >= rect.left - 8 && x <= rect.left + 10) vi = cell.cellIndex - 2
  if (vi < 0 || vi >= visibleColumns.value.length) return -1
  return vi
}

const onTableMove = (e) => {
  const wrap = tableWrapRef.value
  if (!wrap) return
  if (drag) { wrap.style.cursor = 'col-resize'; return }
  const cell = e.target.closest('th, td')
  if (lastHover && lastHover !== cell) lastHover.style.cursor = ''
  const vi = edgeColIdx(e)
  if (vi >= 0 && cell) {
    cell.style.cursor = 'col-resize'
    lastHover = cell
  } else {
    lastHover = null
  }
  wrap.style.cursor = vi >= 0 ? 'col-resize' : ''
}
const onTableLeave = () => {
  const wrap = tableWrapRef.value
  if (!wrap) return
  if (!drag) wrap.style.cursor = ''
  if (lastHover) { lastHover.style.cursor = ''; lastHover = null }
}
const onTableDown = (e) => {
  if (e.button !== 0) return
  const vi = edgeColIdx(e)
  if (vi < 0) return
  e.preventDefault()
  startColResize(vi, e)
}
const startColResize = (vi, e) => {
  const col = visibleColumns.value[vi]
  if (!col) return
  const wrap = tableWrapRef.value
  if (!Object.keys(colWidths.value).length) measureColumns()
  const table = wrap?.querySelector('table')
  const colEl = table?.querySelectorAll('colgroup col')?.[vi + 1] || null
  if (colEl) colEl.style.willChange = 'width'
  drag = {
    col,
    startW: colWidths.value[col] || defaultColWidth(col),
    currentW: colWidths.value[col] || defaultColWidth(col),
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
    colWidths.value = { ...colWidths.value, [drag.col]: finalW }
    if (drag.colEl) drag.colEl.style.willChange = ''
    lastResizeAt = Date.now()
  }
  if (rafId) { cancelAnimationFrame(rafId); rafId = null }
  drag = null
  colResizing.value = false
  const wrap = tableWrapRef.value
  if (wrap) {
    wrap.classList.remove('col-resizing')
    wrap.style.cursor = ''
    if (lastHover) { lastHover.style.cursor = ''; lastHover = null }
  }
  document.removeEventListener('mousemove', onDragMove)
  document.removeEventListener('mouseup', onDragEnd)
}

// ===== 行多选（行号列点/拖 + Shift/Ctrl）：与表预览同一套 =====
const selectedSet = ref(new Set())
const lastAnchorIdx = ref(null) // null=无，-1=标题行，>=0=displayRows 下标
const headerSelected = ref(false)
const isRowSelected = (row) => selectedSet.value.has(row._rid)
const toggleRowSelect = (row) => {
  const s = new Set(selectedSet.value)
  if (s.has(row._rid)) s.delete(row._rid); else s.add(row._rid)
  selectedSet.value = s
}
const onRowClick = (row, e) => {
  if (Date.now() - lastResizeAt < 300) return
  if (gridSel?.suppressClick()) return
  const idx = dispIdxOf(row)
  if (e.shiftKey && lastAnchorIdx.value != null) {
    selectRowRange(lastAnchorIdx.value, idx) // Shift 连选：锚点保持不动
    return
  }
  if (e.ctrlKey || e.metaKey) {
    headerSelected.value = false
    toggleRowSelect(row)
  } else {
    headerSelected.value = false
    selectedSet.value = new Set([row._rid])
  }
  lastAnchorIdx.value = idx
}
// 行号列拖动连选：document mousemove + elementFromPoint，拖快不漏行
let rowDrag = null
const selectRowRange = (a, b) => {
  const list = displayRows.value
  const lo = Math.min(a, b)
  const hi = Math.max(a, b)
  headerSelected.value = lo <= -1
  const i1 = Math.max(0, lo)
  const i2 = Math.min(list.length - 1, hi)
  const dragSt = rowDrag
  if (dragSt && dragSt.lo != null && dragSt.count === list.length) {
    // 拖选增量增删：只碰区间两端变化的行，避免整段 Set 替换引发全表重渲染
    const s = selectedSet.value
    for (let i = dragSt.lo; i < i1; i++) s.delete(list[i]._rid)
    for (let i = Math.max(i2 + 1, dragSt.lo); i <= dragSt.hi; i++) s.delete(list[i]._rid)
    for (let i = i1; i <= i2; i++) s.add(list[i]._rid)
    dragSt.lo = i1
    dragSt.hi = i2
    dragSt.count = list.length
    return
  }
  const s = new Set()
  for (let i = i1; i <= i2; i++) s.add(list[i]._rid)
  selectedSet.value = s
  if (dragSt) { dragSt.lo = i1; dragSt.hi = i2; dragSt.count = list.length }
}
const onHeaderRowDown = (e) => {
  if (e.button !== 0) return
  if (gridSel && gridSel.suppressClick()) return
  focusRows()
  if (e.ctrlKey || e.metaKey) {
    headerSelected.value = !headerSelected.value
    lastAnchorIdx.value = -1
    return
  }
  if (e.shiftKey && lastAnchorIdx.value != null) {
    selectRowRange(lastAnchorIdx.value, -1)
  } else {
    headerSelected.value = true
    selectedSet.value = new Set()
    lastAnchorIdx.value = -1
    rowDrag = { anchor: -1, last: -1 }
    document.addEventListener('mousemove', onRowNumDragMove)
    document.addEventListener('mouseup', onRowNumDragEnd, { once: true })
  }
  if (tableWrapRef.value && tableWrapRef.value.focus) tableWrapRef.value.focus({ preventScroll: true })
}
const rangeCoversHeader = computed(() => {
  const r = gridSel?.range?.value
  return !!r && r.r1 === 0
})
// 行/列选中的「外沿」：连续行段/列段只给首尾两端加边线，整块一个框（与框选同一观感）
const selEdges = computed(() => {
  const list = displayRows.value
  const rowTop = new Set()
  const rowBottom = new Set()
  const headOn = headerSelected.value || rangeCoversHeader.value
  // 只扫当前渲染窗口 ±1 行：全表扫描是拖选/滚动发卡的主因
  const from = virtualEnabled.value ? Math.max(0, vpStart.value - 1) : 0
  const to = virtualEnabled.value ? Math.min(list.length, Math.max(vpEnd.value + 1, from + 1)) : list.length
  for (let i = from; i < to; i++) {
    if (!selectedSet.value.has(list[i]._rid)) continue
    if (i === 0 ? !headOn : !selectedSet.value.has(list[i - 1]._rid)) rowTop.add(list[i]._rid)
    if (i === list.length - 1 || !selectedSet.value.has(list[i + 1]._rid)) rowBottom.add(list[i]._rid)
  }
  const cols = visibleColumns.value
  const colLeft = new Set()
  const colRight = new Set()
  let prevOn = false
  cols.forEach((c, i) => {
    const on = selectedCols.value.has(c)
    if (on && !prevOn) colLeft.add(c)
    if (!on && prevOn) colRight.add(cols[i - 1])
    prevOn = on
  })
  if (prevOn) colRight.add(cols[cols.length - 1])
  const headerBottom = headOn && !(list[0] && selectedSet.value.has(list[0]._rid))
  return { rowTop, rowBottom, colLeft, colRight, headerBottom }
})
// 列选中时，只在「最后一行」收底边
const lastDisplayRid = computed(() => {
  const list = displayRows.value
  return list.length ? list[list.length - 1]._rid : null
})

// 整表只有一块选中区域：任何一类新选择开始前，先清掉另外两类
const clearRowSelection = () => {
  if (selectedSet.value.size) selectedSet.value = new Set()
  if (headerSelected.value) headerSelected.value = false
}
const clearColSelection = () => { if (selectedCols.value.size) clearColSelect() }
const clearGridRange = () => { if (gridSel && gridSel.hasSelection()) gridSel.clearRange() }
const focusRows = () => { clearColSelection(); clearGridRange(); activeCell.value = null }
const focusCols = () => { clearRowSelection(); clearGridRange(); activeCell.value = null }
const focusCells = () => { clearRowSelection(); clearColSelection() }

const onRowNumDown = (row, e) => {
  if (e.button !== 0) return
  if (gridSel && gridSel.suppressClick()) return
  focusRows()
  const prevAnchor = lastAnchorIdx.value
  onRowClick(row, e)
  if (tableWrapRef.value && tableWrapRef.value.focus) tableWrapRef.value.focus({ preventScroll: true })
  if (e.ctrlKey || e.metaKey) return
  rowDrag = { anchor: e.shiftKey && prevAnchor != null ? prevAnchor : dispIdxOf(row) }
  document.addEventListener('mousemove', onRowNumDragMove)
  document.addEventListener('mouseup', onRowNumDragEnd, { once: true })
}
let rowDragRaf = 0
let rowDragMoved = false
let rowDragPoint = { x: 0, y: 0 }
let rowDragScrollRaf = 0
const onRowNumDragMove = (e) => {
  if (!rowDrag) return
  rowDragPoint = { x: e.clientX, y: e.clientY }
  rowDragMoved = true
  if (!rowDragScrollRaf) rowDragScrollRaf = requestAnimationFrame(rowDragAutoScroll)
  if (rowDragRaf) return
  rowDragRaf = requestAnimationFrame(() => {
    rowDragRaf = 0
    if (rowDrag) extendRowDragTo(rowDragPoint.x, rowDragPoint.y)
  })
}
const extendRowDragTo = (x, y) => {
  if (!rowDrag) return
  const el = document.elementFromPoint(x, y)
  let idx = -2
  if (el && el.closest) {
    if (el.closest('thead')) idx = -1
    else {
      const tr = el.closest('tr[data-rid]')
      const rid = tr ? tr.getAttribute('data-rid') : null
      if (rid != null) {
        const hit = ridToDispIdx.value.get(String(rid))
        if (hit != null) idx = hit
      }
    }
  }
  if (idx === -2) {
    const host = scrollHost()
    if (!host) return
    const rect = host.getBoundingClientRect()
    if (y >= rect.bottom) idx = displayRows.value.length - 1
    else if (y <= rect.top) idx = -1
    else return
  }
  if (idx === rowDrag.last) return
  rowDrag.last = idx
  selectRowRange(rowDrag.anchor, idx)
}
// 拖到表格上下边缘附近时自动滚动
const rowDragAutoScroll = () => {
  rowDragScrollRaf = 0
  if (!rowDrag) return
  const host = scrollHost()
  if (!host) return
  const rect = host.getBoundingClientRect()
  const EDGE = 26
  let dy = 0
  if (rowDragPoint.y < rect.top + EDGE) dy = -1
  else if (rowDragPoint.y > rect.bottom - EDGE) dy = 1
  if (!dy) return
  host.scrollTop += dy * 24
  extendRowDragTo(rowDragPoint.x, rowDragPoint.y)
  rowDragScrollRaf = requestAnimationFrame(rowDragAutoScroll)
}
const onRowNumDragEnd = () => {
  if (rowDragRaf) {
    cancelAnimationFrame(rowDragRaf)
    rowDragRaf = 0
    if (rowDrag && rowDragMoved) extendRowDragTo(rowDragPoint.x, rowDragPoint.y)
  }
  rowDrag = null
  rowDragMoved = false
  if (rowDragScrollRaf) { cancelAnimationFrame(rowDragScrollRaf); rowDragScrollRaf = 0 }
  document.removeEventListener('mousemove', onRowNumDragMove)
}
onBeforeUnmount(onRowNumDragEnd)

// ===== 选中整列（Excel 式）：单击表头选中整列，Ctrl 加减选，Shift 连选；表头拖动连选多列 =====
const selectedCols = ref(new Set())
const lastColAnchor = ref('')
const clearColSelect = () => { selectedCols.value = new Set(); lastColAnchor.value = '' }
const toggleColSelect = (col) => {
  const s = new Set(selectedCols.value)
  if (s.has(col)) s.delete(col); else s.add(col)
  selectedCols.value = s
  lastColAnchor.value = col
}
const selectColRange = (col) => {
  const cols = visibleColumns.value
  const a = cols.indexOf(lastColAnchor.value)
  const b = cols.indexOf(col)
  if (a < 0 || b < 0) { selectSingleCol(col); return }
  selectedCols.value = new Set(cols.slice(Math.min(a, b), Math.max(a, b) + 1))
}
const selectSingleCol = (col) => {
  selectedCols.value = new Set([col])
  lastColAnchor.value = col
}
const onHeaderClick = (col, e) => {
  if (Date.now() < colSelClickUntil) return // 刚拖过连选，紧接着的这次 click 忽略
  if (e.shiftKey && gridSel && gridSel.hasSelection()) return
  focusCols()
  if (e.ctrlKey || e.metaKey) { toggleColSelect(col); return }
  if (e.shiftKey) { selectColRange(col); return }
  selectSingleCol(col)
}
let colSelDrag = null
let colSelClickUntil = 0
const onHeaderDown = (col, e) => {
  if (e.button !== 0) return
  if (edgeColIdx(e) >= 0) return // 贴缘交给列宽拖拽
  if (e.shiftKey && gridSel && gridSel.hasSelection()) return
  onColSelectDown(col, e)
}
const onColSelectDown = (col, e) => {
  focusCols()
  colSelDrag = { anchor: col, anchorC: visibleColumns.value.indexOf(col), mode: 'cols', moved: false }
  document.addEventListener('mousemove', onColSelectMove)
  document.addEventListener('mouseup', onColSelectEnd, { once: true })
  e.preventDefault() // 拖动时不要选中表头文字
}
const onColSelectMove = (e) => {
  const st = colSelDrag
  if (!st) return
  const el = document.elementFromPoint(e.clientX, e.clientY)
  if (!el || !el.closest) return
  const gkeyOf = (node) => {
    const g = node && node.getAttribute ? node.getAttribute('data-gkey') : null
    if (!g) return null
    const p = g.split(':')
    return { r: Number(p[0]), c: Number(p[1]) }
  }
  // 从表头往下拖进数据区 → 从「连选列」切成「含标题行(行0)的区域框选」
  if (st.mode === 'cols' && el.closest('td[data-gkey]')) {
    st.mode = 'cells'
    focusCells()
  }
  if (st.mode === 'cells') {
    const cell = el.closest('td[data-gkey]') || el.closest('th[data-gkey]')
    const g = gkeyOf(cell)
    if (!g || st.anchorC < 0) return
    st.moved = true
    gridSel.setRange(0, st.anchorC, g.r, g.c) // 锚点固定在标题行，范围自然包含列名
    e.preventDefault()
    return
  }
  const th = el.closest('th[data-gkey]')
  const g = gkeyOf(th)
  if (!g) return
  const cols = visibleColumns.value
  if (st.anchorC < 0 || !cols[g.c]) return
  st.moved = true
  selectedCols.value = new Set(cols.slice(Math.min(st.anchorC, g.c), Math.max(st.anchorC, g.c) + 1))
  lastColAnchor.value = st.anchor
  e.preventDefault()
}
const onColSelectEnd = () => {
  const st = colSelDrag
  colSelDrag = null
  document.removeEventListener('mousemove', onColSelectMove)
  if (st && st.moved) colSelClickUntil = Date.now() + 250
}
onBeforeUnmount(() => {
  colSelDrag = null
  document.removeEventListener('mousemove', onColSelectMove)
})

// ===== 类 Excel 框选（与表预览共用 useExcelSelection）=====
// valueAt/onCopied 用箭头包一层：它们定义在后面，setup 阶段直接引用会 TDZ
const gridSel = useExcelSelection({
  container: tableWrapRef,
  shouldStart: (e) => !e.shiftKey && !e.target.closest('th') && edgeColIdx(e) < 0,
  valueAt: (r, c) => cellTextAt(r, c),
  onCopied: (info) => onGridCopied(info)
})
const hasGridSelection = computed(() => !!gridSel?.range?.value)
const noBulkSelection = computed(() =>
  !selectedSet.value.size && !selectedCols.value.size && !hasGridSelection.value)
// 框选一出现就清掉行、列选中：三类选区只能同时存在一块
watch(() => gridSel?.range?.value, (r) => {
  if (!r) return
  if (selectedSet.value.size) selectedSet.value = new Set()
  if (headerSelected.value) headerSelected.value = false
  if (selectedCols.value.size) clearColSelect()
})

// ===== 活动单元格（键盘导航锚点）=====
const activeCell = ref(null)
const onCellClick = (row, col, e) => {
  if (selectedCols.value.size) clearColSelect()
  activeCell.value = { row, col }
  if (tableWrapRef.value && tableWrapRef.value.focus) tableWrapRef.value.focus({ preventScroll: true })
}
const gkeyOfCell = (row, col) => {
  const c = visibleColumns.value.indexOf(col)
  if (!row || c < 0) return null
  return { r: rowIndex(row) + 1, c }
}
const cellByGkey = (r0, c0) => {
  const row = rows.value[r0 - 1]
  const col = visibleColumns.value[c0]
  return (row && col) ? { row, col } : null
}
// 按 gkey 取值：r0 === 0 为表头行（列名）。走数据而非 DOM，跨屏大选区也能取全
const cellTextAt = (r0, c0) => {
  if (r0 === 0) return visibleColumns.value[c0] || ''
  const hit = cellByGkey(r0, c0)
  if (!hit) return ''
  return plainVal(hit.row[hit.col])
}
const dispIdxOf = (row) => displayRows.value.indexOf(row)
const applyActive = (rIdx, cIdx, extend) => {
  const list = displayRows.value
  const cols = visibleColumns.value
  if (!list.length || !cols.length) return
  const row = list[Math.max(0, Math.min(list.length - 1, rIdx))]
  const col = cols[Math.max(0, Math.min(cols.length - 1, cIdx))]
  if (!row || !col) return
  const from = activeCell.value
  const anchorG = extend ? (gridSel.anchor() || (from ? gkeyOfCell(from.row, from.col) : null)) : null
  if (extend) focusCells()
  activeCell.value = { row, col }
  const g = gkeyOfCell(row, col)
  if (anchorG && g) gridSel.setRange(anchorG.r, anchorG.c, g.r, g.c)
  else gridSel.clearRange()
  ensureActiveVisible()
}
const moveActive = (dr, dc, { edge = false, extend = false } = {}) => {
  const from = activeCell.value
  if (!from) return
  const list = displayRows.value
  const cols = visibleColumns.value
  const r0 = dispIdxOf(from.row)
  const c0 = cols.indexOf(from.col)
  if (r0 < 0 || c0 < 0) return
  let r = r0 + dr
  let c = c0 + dc
  if (edge) {
    if (dr) r = dr > 0 ? list.length - 1 : 0
    if (dc) c = dc > 0 ? cols.length - 1 : 0
  }
  applyActive(r, c, extend)
}
const tabMove = (dir) => {
  const from = activeCell.value
  if (!from) return
  const cols = visibleColumns.value
  let r = dispIdxOf(from.row)
  let c = cols.indexOf(from.col) + dir
  if (c >= cols.length) { c = 0; r += 1 }
  else if (c < 0) { c = cols.length - 1; r -= 1 }
  applyActive(r, c, false)
}
const pageRowStep = () => {
  const host = scrollHost()
  const n = host ? Math.floor(host.clientHeight / (VP_ROW_H || 32)) : 10
  return Math.max(1, n - 1)
}
const ensureActiveVisible = () => {
  if (!activeCell.value || !tableWrapRef.value) return
  nextTick(() => {
    const host = scrollHost()
    const el = tableWrapRef.value.querySelector(`[data-gkey="${(rowIndex(activeCell.value.row) + 1) + ':' + visibleColumns.value.indexOf(activeCell.value.col)}"]`)
    if (el && host) el.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  })
}

// ===== 选中区汇总（底栏，与表预览同一套）=====
const NUMERIC_LITERAL_RE = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/
const fmtSummaryNum = (n) => (n == null || !Number.isFinite(n)) ? '' : (Number.isInteger(n) ? String(n) : Number(n.toFixed(2)).toLocaleString())
const selectionSummary = computed(() => {
  const rect = gridSel && gridSel.range ? gridSel.range.value : null
  const cols = visibleColumns.value || []
  const acc = { cells: 0, nums: 0, sum: 0, min: null, max: null }
  const add = (r, c) => {
    if (r <= 0) return
    const col = cols[c]
    const row = rows.value[r - 1]
    if (col === undefined || !row) return
    acc.cells++
    const v = row[col]
    if (v == null) return
    const n = typeof v === 'number' ? v : (NUMERIC_LITERAL_RE.test(String(v).trim()) ? Number(v) : NaN)
    if (!Number.isFinite(n)) return
    acc.nums++
    acc.sum += n
    acc.min = acc.min == null ? n : Math.min(acc.min, n)
    acc.max = acc.max == null ? n : Math.max(acc.max, n)
  }
  if (rect) {
    for (let r = Math.max(1, rect.r1); r <= rect.r2; r++) {
      for (let c = rect.c1; c <= rect.c2; c++) add(r, c)
    }
  } else if (selectedSet.value.size) {
    const byRid = new Map(rows.value.map((row, i) => [row._rid, i]))
    for (const rid of selectedSet.value) {
      const i = byRid.get(rid)
      if (i == null) continue
      for (let c = 0; c < cols.length; c++) add(i + 1, c)
    }
  } else if (selectedCols.value.size) {
    for (let i = 0; i < rows.value.length; i++) {
      for (let c = 0; c < cols.length; c++) if (selectedCols.value.has(cols[c])) add(i + 1, c)
    }
  } else {
    return null
  }
  if (!acc.cells) return null
  return { cells: acc.cells, nums: acc.nums, sum: acc.sum, avg: acc.nums ? acc.sum / acc.nums : null, min: acc.min, max: acc.max }
})

// ===== 右键上下文菜单（与表预览同一套：由当前选中决定菜单内容）=====
const ctxMenu = ref({ visible: false, x: 0, y: 0, items: [], row: null, col: null })
const ctxSub = ref(null)
let ctxFrom = 'cell'
const closeCtxMenu = () => { ctxMenu.value = { ...ctxMenu.value, visible: false }; ctxSub.value = null }
const onGridContextMenu = (e) => {
  e.preventDefault()
  openCtxMenu(e.clientX, e.clientY, null, null, 'grid')
}
const onHeaderContextMenu = (e, col) => {
  e.preventDefault()
  openCtxMenu(e.clientX, e.clientY, null, col, 'header')
}
const onRowContextMenu = (e, row) => {
  e.preventDefault()
  openCtxMenu(e.clientX, e.clientY, row, null, 'row')
}
const onCellContextMenu = (e, row, col) => {
  e.preventDefault()
  openCtxMenu(e.clientX, e.clientY, row, col, 'cell')
}
const openCtxMenu = (x, y, row, col, from = 'cell') => {
  ctxMenu.value = { visible: false, x, y, items: [], row, col }
  ctxFrom = from
  const fr = gridSel?.range?.value || null
  const frameOn = !!fr && (fr.r1 !== fr.r2 || fr.c1 !== fr.c2)
  const hasSelection = frameOn || selectedSet.value.size > 0 || selectedCols.value.size > 0 || headerSelected.value
  // 表头 / 行头右键只在完全没有选中时才把选中切过去（菜单由选中决定）
  if (from === 'header' && col && !hasSelection) {
    focusCols()
    selectSingleCol(col)
  }
  if (from === 'row' && row && !hasSelection) {
    focusRows()
    selectedSet.value = new Set([row._rid])
    lastAnchorIdx.value = dispIdxOf(row)
  }
  const sel = ctxSelection()
  const hasTarget = !!col || sel.rows.length > 0
  if (!hasTarget) return
  const items = []
  const sep = () => { if (items.length && !items[items.length - 1].sep) items.push({ sep: true }) }
  // 复制为 ▸：CSV/JSON/MARKDOWN 与表预览共用；REDIS 命令是 NoSQL 特有（key/value 两列还原成 SET/DEL）
  const formatSub = [
    { label: 'CSV', command: 'copy-csv' },
    { label: 'JSON', command: 'copy-json' },
    { label: 'MARKDOWN', command: 'copy-markdown' },
    { label: t('nsql.menuRedis'), command: 'copy-redis' }
  ]
  const selColCount = selectedCols.value.size
  const selRowCount = selectedSet.value.size
  const mode = frameOn ? 'range'
    : (selRowCount || headerSelected.value) ? 'rows'
      : selColCount ? 'columns'
        : from === 'header' ? 'columns'
          : from === 'row' ? 'rows'
            : from === 'grid' ? 'range'
              : 'cell'
  if (from === 'grid' && !frameOn) return

  if (mode === 'range') {
    items.push({ label: t('mdk.copy'), command: 'copy-sel', shortcut: 'Ctrl+C' })
    items.push({ label: t('sqlq.ctxCopyAs'), sub: formatSub })
  } else if (mode === 'columns') {
    items.push({ label: t('mdk.copy'), command: 'copy-sel', shortcut: 'Ctrl+C' })
    items.push({ label: t('sqlq.ctxCopyAs'), sub: formatSub })
    items.push({ label: t('sqlq.ctxCopyHeader'), command: 'copy-col-header' })
    sep()
    items.push({ label: t('sqlq.ctxColFit'), command: 'col-fit' })
    items.push({ label: t('sqlq.ctxHideCol'), command: 'hide-col' })
    if (hiddenColumns.value.size) items.push({ label: t('sqlq.ctxShowAllCols'), command: 'show-all-cols' })
  } else if (mode === 'rows') {
    if (row) items.push({ label: t('sqlq.ctxRowDetail'), command: 'row-detail' })
    items.push({ label: t('mdk.copy'), command: 'copy-sel', shortcut: 'Ctrl+C' })
    items.push({ label: t('sqlq.ctxCopyAs'), sub: formatSub })
    sep()
    items.push({ label: t('tdv.hideRows'), command: 'hide-rows' })
    if (hiddenRows.value.size) items.push({ label: t('tdv.showAllRows'), command: 'show-all-rows' })
  } else {
    if (col) items.push({ label: t('mdk.copy'), command: 'copy-cell', shortcut: 'Ctrl+C' })
    if (row && col) {
      sep()
      items.push({ label: t('sqlq.cellDetail'), command: 'cell-detail' })
    }
  }
  if (!items.length) return
  const flat = items.map(it => (it.sub && it.sub.length <= 1 ? it.sub[0] : it))
  ctxMenu.value = { visible: true, x, y, items: flat, row, col }
  ctxSub.value = null
  nextTick(() => {
    const el = document.querySelector('.grid-ctx-menu')
    if (el) {
      const r = el.getBoundingClientRect()
      if (r.right > window.innerWidth) ctxMenu.value.x = window.innerWidth - r.width - 8
      if (r.bottom > window.innerHeight) ctxMenu.value.y = window.innerHeight - r.height - 8
    }
  })
}
const onCtxItem = (item) => {
  const { row, col } = ctxMenu.value
  switch (item.command) {
    case 'copy-cell': copyCell(row, col); break
    case 'row-detail': if (row) openRowDetail(row); break
    case 'cell-detail': if (row && col) openCellDetail(row, col); break
    case 'copy-sel': copyLikeCtrlC(); break
    case 'copy-col-header': copyColHeader(); break
    case 'copy-csv': copyCtx('csv'); break
    case 'copy-json': copyCtx('json'); break
    case 'copy-markdown': copyMarkdown(); break
    case 'copy-redis': copyRedis(); break
    case 'col-fit':
      if (selectedCols.value.size > 1) autoFitSelectedCols()
      else autoFitCol(col)
      break
    case 'hide-col': hideColumnSmart(col); break
    case 'hide-rows': hideRowsSmart(row); break
    case 'show-all-rows': showAllRows(); break
    case 'show-all-cols': showAllColumns(); break
  }
  closeCtxMenu()
}
// 点击别处关闭菜单
const onDocCtxClose = (e) => {
  if (ctxMenu.value.visible && !(e.target.closest && e.target.closest('.grid-ctx-menu'))) closeCtxMenu()
}
document.addEventListener('mousedown', onDocCtxClose)
onBeforeUnmount(() => { document.removeEventListener('mousedown', onDocCtxClose) })
// 二级子菜单：悬停带 sub 的项时在其右侧弹出
const closeCtxSub = () => { ctxSub.value = null }
const onCtxItemHover = (item, i, e) => {
  if (!item.sub || !item.sub.length) { closeCtxSub(); return }
  const el = e && e.currentTarget
  if (!el) return
  const menuEl = el.closest('.grid-ctx-menu')
  const menuRect = menuEl ? menuEl.getBoundingClientRect() : { left: 0, right: 0, top: 0 }
  const itemRect = el.getBoundingClientRect()
  ctxSub.value = { parentIndex: i, items: item.sub, x: menuRect.right + 2, y: itemRect.top, menuLeft: menuRect.left }
  nextTick(() => {
    const subEl = document.querySelector('.grid-ctx-sub')
    if (!subEl) return
    const r = subEl.getBoundingClientRect()
    let x = ctxSub.value.x
    let y = ctxSub.value.y
    if (x + r.width > window.innerWidth - 4) x = Math.max(4, ctxSub.value.menuLeft - r.width - 2)
    if (y + r.height > window.innerHeight - 4) y = Math.max(4, window.innerHeight - r.height - 4)
    ctxSub.value = { ...ctxSub.value, x, y }
  })
}

// ===== 复制栈（与表预览同一条路）=====
const writeClipboard = (text, msg) => {
  if (!text) return
  const done = () => { if (msg) ElMessage.success(msg) }
  const fallback = () => {
    try {
      const ta = document.createElement('textarea')
      ta.value = text
      ta.setAttribute('readonly', '')
      ta.style.cssText = 'position:fixed;left:-9999px;top:0;opacity:0'
      document.body.appendChild(ta)
      ta.select()
      const ok = document.execCommand('copy')
      document.body.removeChild(ta)
      if (ok) done()
      else if (msg) ElMessage.error(t('sqlq.copyFailed'))
    } catch { if (msg) ElMessage.error(t('sqlq.copyFailed')) }
  }
  if (navigator.clipboard?.writeText) navigator.clipboard.writeText(text).then(done).catch(fallback)
  else fallback()
}
// 右键目标的「行集合 / 列集合」：框选 > 行/列选中 > 右键那一格
const ctxSelection = () => {
  const { row, col } = ctxMenu.value
  const range = gridSel?.range?.value || null
  const rangeHasArea = !!range && (range.r1 !== range.r2 || range.c1 !== range.c2)
  if (rangeHasArea && (ctxFrom === 'cell' || ctxFrom === 'grid')) {
    const picked = []
    for (let r = Math.max(range.r1, 1); r <= range.r2; r++) {
      const rr = rows.value[r - 1]
      if (rr && !hiddenRows.value.has(rr._rid)) picked.push(rr)
    }
    const cols = visibleColumns.value.slice(range.c1, range.c2 + 1)
    if (picked.length) return { rows: picked, cols }
  }
  const checked = displayRows.value.filter(r => selectedSet.value.has(r._rid))
  const pickedCols = visibleColumns.value.filter(c => selectedCols.value.has(c))
  if (checked.length || pickedCols.length) {
    return {
      rows: checked.length ? checked : displayRows.value,
      cols: pickedCols.length ? pickedCols : [...visibleColumns.value]
    }
  }
  if (!row) return { rows: [], cols: [] }
  return { rows: [row], cols: col ? [col] : [...visibleColumns.value] }
}
const copyCtx = (mode) => {
  const { rows: pickedRows, cols } = ctxSelection()
  if (!pickedRows.length || !cols.length) { ElMessage.warning(t('sqlq.nothingToCopy')); return }
  if (mode === 'csv') {
    const esc = (s) => /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
    const lines = [cols.map(esc).join(',')]
    for (const r of pickedRows) lines.push(cols.map(c => esc(plainVal(r[c]))).join(','))
    writeClipboard(lines.join('\n'), t('sqlq.copyCsv', { n: pickedRows.length }))
    return
  }
  if (mode === 'json') {
    const obj = pickedRows.map(r => {
      const o = {}
      for (const c of cols) o[c] = r[c] === undefined ? null : r[c]
      return o
    })
    writeClipboard(JSON.stringify(obj.length === 1 ? obj[0] : obj, null, 2), t('sqlq.copyJson', { n: pickedRows.length }))
    return
  }
  if (mode === 'redis') {
    // Redis 场景最实用的写法：把 key/value 两列还原成可粘贴执行的命令
    const ki = cols.findIndex(c => /^key$/i.test(c))
    const vi = cols.findIndex(c => /^value$/i.test(c))
    if (ki >= 0 && vi >= 0) {
      const text = pickedRows.map(r => {
        const k = plainVal(r[cols[ki]])
        const v = plainVal(r[cols[vi]])
        return v === '' ? 'DEL ' + k : 'SET ' + k + ' ' + JSON.stringify(v)
      }).join('\n')
      writeClipboard(text, t('sqlq.copyCsv', { n: pickedRows.length }))
      return
    }
    // 没有 key/value 列（Mongo / ES 文档视图）：退回 JSON
    copyCtx('json')
    return
  }
}
const copyCell = (row, col) => {
  if (!row || !col) return
  writeClipboard(plainVal(row[col]), '已复制单元格')
}
// 当前选区的 TSV 文本（Excel 可直接粘贴）
const selectionAsTsv = () => {
  const cols = visibleColumns.value
  if (selectedCols.value.size) {
    const picked = cols.filter(c => selectedCols.value.has(c))
    if (!picked.length) return ''
    const lines = [picked.join('\t')]
    for (const r of displayRows.value) lines.push(picked.map(c => plainVal(r[c])).join('\t'))
    return lines.join('\n')
  }
  if (selectedSet.value.size || headerSelected.value) {
    const picked = displayRows.value.filter(r => selectedSet.value.has(r._rid))
    const lines = []
    if (headerSelected.value) lines.push(cols.join('\t'))
    for (const r of picked) lines.push(cols.map(c => plainVal(r[c])).join('\t'))
    return lines.join('\n')
  }
  if (activeCell.value) {
    const { row, col } = activeCell.value
    const v = row && col ? row[col] : null
    return v == null ? '' : plainVal(v)
  }
  return ''
}
const copyLikeCtrlC = () => {
  if (gridSel && gridSel.hasSelection()) {
    const tx = gridSel.copy()
    if (tx) writeClipboard(tx, `已复制选区（${tx.split('\n').length} 行）`)
    return
  }
  const text = selectionAsTsv()
  if (!text) { ElMessage.warning(t('sqlq.nothingToCopy')); return }
  writeClipboard(text, `已复制 ${text.split('\n').length} 行`)
}
const copyHint = () => {
  if (selectedCols.value.size) return `已复制 ${selectedCols.value.size} 列（含列名）`
  if (selectedSet.value.size) return `已复制 ${selectedSet.value.size} 行${headerSelected.value ? '（含列名）' : ''}`
  if (headerSelected.value) return t('sqlq.copyColNames')
  return t('sqlq.copyCell')
}
const onGridCopied = ({ rows: nRows, cols: nCols, header }) => {
  ElMessage.success(`已复制选区（${nRows} 行 × ${nCols} 列${header ? '，含列名' : ''}）`)
}
// Ctrl+C 主路径：浏览器 copy 事件（任何焦点下都能用；输入框里不动）
let copyEventAt = 0
const inEditableFocus = () => {
  const ae = document.activeElement
  return !!(ae && (ae.tagName === 'INPUT' || ae.tagName === 'TEXTAREA' || ae.isContentEditable))
}
const onDocCopy = (e) => {
  if (inEditableFocus()) { e.stopImmediatePropagation(); return }
  if (gridSel && gridSel.hasSelection()) return // 框选交给框选模块
  if (!rootRef.value || !rootRef.value.getClientRects().length) return
  const text = selectionAsTsv()
  if (!text) return
  copyEventAt = Date.now()
  e.clipboardData.setData('text/plain', text)
  ElMessage.success(copyHint())
  e.preventDefault()
}
document.addEventListener('copy', onDocCopy)
onBeforeUnmount(() => document.removeEventListener('copy', onDocCopy))
// 复制列名：有选中的列就复制选中的，否则右键那一列；多列制表符分隔（贴 Excel 是横向多格）
const copyColHeader = () => {
  const picked = visibleColumns.value.filter(c => selectedCols.value.has(c))
  const names = picked.length ? picked : (ctxMenu.value.col ? [ctxMenu.value.col] : [])
  if (!names.length) return
  writeClipboard(names.join('\t'), names.length > 1 ? t('sqlq.copyColNamesN', { n: names.length }) : t('sqlq.copyColNameOne', { name: names[0] }))
}
const copyMarkdown = () => {
  const { rows: pickedRows, cols } = ctxSelection()
  if (!pickedRows.length || !cols.length) return
  const esc = (s) => String(s).replace(/\|/g, '\\|').replace(/\r?\n/g, ' ')
  const head = `| ${cols.map(esc).join(' | ')} |`
  const sep = `| ${cols.map(() => '---').join(' | ')} |`
  const body = pickedRows.map(r => `| ${cols.map(c => esc(plainVal(r[c]))).join(' | ')} |`)
  writeClipboard([head, sep, ...body].join('\n'), t('sqlq.copyMarkdown', { n: pickedRows.length }))
}

// ===== 隐藏行/列（右键）：只影响显示 =====
const hideColumn = (col) => {
  if (!col) return
  if (visibleColumns.value.length <= 1) { ElMessage.warning(t('sqlq.keepOneCol')); return }
  hiddenColumns.value = new Set(hiddenColumns.value).add(col)
}
const hideColumnSmart = (col) => {
  const picked = visibleColumns.value.filter(c => selectedCols.value.has(c))
  if (!picked.length) { hideColumn(col); return }
  if (visibleColumns.value.length - picked.length < 1) { ElMessage.warning(t('sqlq.keepOneCol')); return }
  hiddenColumns.value = new Set([...hiddenColumns.value, ...picked])
  clearColSelect()
}
const hideRowsSmart = (row) => {
  const s = new Set(hiddenRows.value)
  if (selectedSet.value.size) {
    for (const r of rows.value) if (selectedSet.value.has(r._rid)) s.add(r._rid)
    selectedSet.value = new Set()
  } else if (row) {
    s.add(row._rid)
  } else {
    return
  }
  hiddenRows.value = s
  activeCell.value = null
  gridSel?.clear()
}
const showAllRows = () => { hiddenRows.value = new Set() }

// ===== 详情弹窗（整行 / 单元格）：双击行号 / 双击单元格 / 右键 =====
const detail = ref({ visible: false, title: '', text: '' })
const openRowDetail = (row) => {
  if (!row) return
  const cols = visibleColumns.value
  const lines = cols.map(c => c + '：' + (row[c] == null ? 'NULL' : String(fmtVal(row[c]))))
  detail.value = { visible: true, title: t('sqlq.rowDetailTitle', { n: rowIndex(row) + 1 }), text: lines.join('\n') }
}
const openCellDetail = (row, col) => {
  if (!row || !col) return
  const v = row[col]
  const text = v == null ? 'NULL' : (typeof v === 'object' ? JSON.stringify(v, null, 2) : String(v))
  detail.value = { visible: true, title: `第 ${rowIndex(row) + 1} 行 · ${col}`, text }
}

// ===== 导出当前页 CSV（本地生成）=====
const exportCsv = () => {
  const cols = visibleColumns.value
  if (!cols.length) return
  const esc = (s) => /[",\n\r]/.test(s) ? `"${String(s).replace(/"/g, '""')}"` : s
  const lines = [cols.map(esc).join(',')]
  for (const r of displayRows.value) lines.push(cols.map(c => esc(plainVal(r[c]))).join(','))
  const blob = new Blob(['\ufeff' + lines.join('\n')], { type: 'text/csv;charset=utf-8' })
  const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')
  saveBlobAs(blob, (props.collection === '*' ? (props.database || 'redis') : props.collection) + '-' + stamp + '.csv')
  ElMessage.success(t('nsql.exportedCsv'))
}

// ===== 表格内键盘导航（类 Excel，与表预览同一套；只读版去掉编辑分支）=====
const onKeyDown = (e) => {
  if (!rootRef.value) return
  const active = document.activeElement
  if (active && active.closest && active.closest('input, textarea, .el-select, .el-dropdown')) return
  // Ctrl/Cmd+A：全选当前页所有行
  if ((e.ctrlKey || e.metaKey) && (e.key === 'a' || e.key === 'A')) {
    const inRoot = rootRef.value.contains(active)
    const onBodyVisible = active === document.body && rootRef.value.getClientRects().length > 0
    if (!inRoot && !onBodyVisible) return
    e.preventDefault()
    focusRows()
    headerSelected.value = false
    if (displayRows.value.length) selectedSet.value = new Set(displayRows.value.map(r => r._rid))
    return
  }
  if (!rootRef.value.contains(active)) return
  // Esc：清空当前这块选区
  if (e.key === 'Escape' &&
      (selectedSet.value.size || headerSelected.value || selectedCols.value.size || gridSel.hasSelection())) {
    e.preventDefault()
    clearRowSelection()
    clearColSelection()
    clearGridRange()
    return
  }
  const mod = e.ctrlKey || e.metaKey
  // Ctrl+C 兜底：个别环境不派发 copy 事件时补写一次（不 preventDefault，别掐掉 copy 事件）
  if (mod && !e.shiftKey && (e.key === 'c' || e.key === 'C')) {
    const startedAt = Date.now()
    setTimeout(() => {
      if (copyEventAt >= startedAt) return
      if (gridSel && gridSel.hasSelection()) return
      if (inEditableFocus()) return
      const text = selectionAsTsv()
      if (text) writeClipboard(text, copyHint())
    }, 0)
    return
  }
  if (!activeCell.value) return
  const { row, col } = activeCell.value
  if (e.key.startsWith('Arrow')) {
    e.preventDefault()
    const dr = e.key === 'ArrowUp' ? -1 : e.key === 'ArrowDown' ? 1 : 0
    const dc = e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowRight' ? 1 : 0
    moveActive(dr, dc, { edge: mod, extend: e.shiftKey })
    return
  }
  if (e.key === 'Home' || e.key === 'End' || e.key === 'PageUp' || e.key === 'PageDown') {
    e.preventDefault()
    const cols = visibleColumns.value
    if (e.key === 'Home') applyActive(mod ? 0 : dispIdxOf(row), 0, e.shiftKey)
    else if (e.key === 'End') applyActive(mod ? displayRows.value.length - 1 : dispIdxOf(row), cols.length - 1, e.shiftKey)
    else moveActive(e.key === 'PageDown' ? pageRowStep() : -pageRowStep(), 0, { extend: e.shiftKey })
    return
  }
  if (e.key === 'Tab') {
    e.preventDefault()
    tabMove(e.shiftKey ? -1 : 1)
  }
}
onMounted(() => window.addEventListener('keydown', onKeyDown))
onBeforeUnmount(() => window.removeEventListener('keydown', onKeyDown))

// 点击表格以外 → 取消当前选中（框选由模块自己清）
const onDocClearPick = (e) => {
  const tg = e.target
  if (!tg || !tg.closest) return
  if (tg.closest('.grid-ctx-menu, .el-popper')) return
  if (tg.closest('.data-table-wrap') && tg.closest('th, td')) return
  if (!rootRef.value || !rootRef.value.contains(tg)) {
    activeCell.value = null
    if (selectedCols.value.size) clearColSelect()
    if (selectedSet.value.size) clearRowSelection()
  }
}
document.addEventListener('mousedown', onDocClearPick)
onBeforeUnmount(() => document.removeEventListener('mousedown', onDocClearPick))

watch(() => columns.value.join('\u0001'), async () => {
  colWidths.value = {}
  if (rows.value.length) {
    await nextTick()
    measureColumns()
  }
})

onBeforeUnmount(() => {
  if (queryTimer) { clearInterval(queryTimer); queryTimer = null }
  stop() // 卸载时中止进行中的文档查询并取消后端任务
  onDragEnd()
})
</script>

<style scoped>
.nosql-view { height: 100%; display: flex; flex-direction: column; padding: 8px; gap: 8px; }
/* ===== 工具栏 / 高级搜索面板（对齐表预览）===== */
.toolbar { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px; }
.toolbar .left, .toolbar .right { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.toolbar .right :deep(.el-button + .el-button) { margin-left: 0; }
.advanced-panel {
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: var(--dc-radius);
  padding: 10px 12px;
}
.filter-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.filter-label { font-size: 12px; color: var(--dc-text-dim); }
.slide-enter-active, .slide-leave-active { transition: max-height .18s ease, opacity .18s ease; overflow: hidden; }
.slide-enter-from, .slide-leave-to { max-height: 0; opacity: 0; }
.slide-enter-to, .slide-leave-from { max-height: 500px; opacity: 1; }

/* ===== 表格容器 / 加载遮罩（对齐表预览）===== */
.grid-area { flex: 1; min-height: 0; position: relative; display: flex; flex-direction: column; background: var(--dc-bg-card); border: 1px solid var(--dc-border); border-radius: var(--dc-radius); overflow: hidden; }
.grid-loading-overlay {
  position: absolute; inset: 0; z-index: 20;
  display: flex; align-items: center; justify-content: center;
  background: color-mix(in srgb, var(--dc-bg-card) 78%, transparent);
}
.grid-loading-box {
  display: flex; align-items: center; gap: 10px;
  padding: 12px 18px; border-radius: 10px;
  background: var(--dc-bg-card); border: 1px solid var(--dc-border);
  box-shadow: var(--dc-shadow-sm, 0 2px 12px rgba(0, 0, 0, 0.08));
  color: var(--dc-primary);
}
.grid-loading-text { font-size: 14px; color: var(--dc-text-mid); }
.has-more-alert { margin: 0; }

/* ===== 原生表格 —— 与表预览（TableDataView）保持一致 ===== */
.data-table-wrap { flex: 1; min-height: 0; overflow: auto; contain: layout paint; }
/* 容器带 tabindex=0（键盘导航要接焦点），点击单元格后 JS 会 focus 它 ——
   必须压掉浏览器默认的黑色 focus 轮廓，否则整块滚动区外围出现两道黑线 */
.data-table-wrap:focus { outline: none; }
.data-table-wrap.col-resizing, .data-table-wrap.col-resizing * { cursor: col-resize !important; user-select: none; }
.data-table { position: relative; width: 100%; table-layout: fixed; border-collapse: collapse; font-size: 13px; }
/* 吸顶表头：top: -1px 盖住滚动时折叠上边框留下的 1px 缝 */
.data-table thead { position: sticky; top: -1px; z-index: 2; }
.data-table th {
  position: relative;
  background: var(--dc-bg-table-head); color: var(--dc-text-strong); font-weight: 600; text-align: left;
  padding: 4px 10px; height: auto; line-height: 1.25; vertical-align: middle;
  border: 1px solid var(--dc-border); white-space: nowrap;
  overflow: hidden;
}
.data-table th.sortable { cursor: pointer; user-select: none; }
.data-table th.sortable:hover { color: var(--dc-text); }
.data-table th.sort-asc, .data-table th.sort-desc { color: var(--dc-primary); }
/* 表头排序按钮：平时淡显，鼠标移到表头才清晰；已排序列主色常亮 */
.data-table th .th-sort {
  display: inline-flex; align-items: center; vertical-align: -1px;
  margin-left: 5px; font-size: 13px; color: var(--dc-text-dim);
  opacity: .35; cursor: pointer; transition: opacity .12s, color .12s;
}
.data-table th:hover .th-sort { opacity: .9; }
.data-table th .th-sort:hover { opacity: 1; color: var(--dc-primary); }
.data-table th .th-sort.is-sorted { opacity: 1; color: var(--dc-primary); }
/* 表头文字块：第一行「类型图标 + 字段名」 */
.data-table th .th-text {
  display: inline-flex; flex-direction: column; justify-content: center;
  vertical-align: middle; overflow: hidden; min-width: 0; max-width: 100%;
}
.data-table th .th-line1 { display: flex; align-items: center; min-width: 0; }
.data-table th .th-label { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
.data-table th .th-type-ic {
  display: inline-flex; align-items: center; vertical-align: middle;
  margin-right: 4px; font-size: 13px; cursor: default; color: var(--dc-text-dim);
}
.data-table th .th-type-ic .el-icon { font-size: 13px; }
/* 列宽拖拽把手 */
.col-resizer {
  position: absolute; top: 0; right: -5px; bottom: 0; width: 10px;
  cursor: col-resize; z-index: 6; user-select: none;
}
.data-table.col-resizing { cursor: col-resize; user-select: none; }
.data-table td {
  padding: 0 10px; height: 32px; line-height: 32px;
  border: 1px solid var(--dc-border); color: var(--dc-text);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  cursor: default;
}
.data-table tbody tr { cursor: pointer; }
/* 斑马纹按真实行号而非 DOM 奇偶：窗口化渲染会插占位行，nth-child 会错位 */
.data-table tbody tr.row-alt td { background-color: var(--dc-bg-soft); }
/* ===== 行 / 列选中：与单元格框选同一套观感（整块淡色填充 + 沿整块外沿画 2px 主色边线）
   注意：一律用 background-color —— background 简写会把这套 background-image 清掉 ===== */
.data-table tbody tr.selected td,
.data-table thead tr.selected th,
.data-table tbody tr td.col-selected,
.data-table th.col-selected {
  --sel-t: 0px; --sel-b: 0px; --sel-l: 0px; --sel-r: 0px;
  background-image:
    linear-gradient(var(--dc-primary), var(--dc-primary)),
    linear-gradient(var(--dc-primary), var(--dc-primary)),
    linear-gradient(var(--dc-primary), var(--dc-primary)),
    linear-gradient(var(--dc-primary), var(--dc-primary));
  background-position: top, bottom, left, right;
  background-size: 100% var(--sel-t), 100% var(--sel-b), var(--sel-l) 100%, var(--sel-r) 100%;
  background-repeat: no-repeat;
}
.data-table tbody tr.selected td,
.data-table thead tr.selected th { background-color: var(--dc-primary-soft) !important; }
.data-table tbody tr.selected.row-sel-top td,
.data-table thead tr.selected.row-sel-top th { --sel-t: 2px; }
.data-table tbody tr.selected.row-sel-bottom td,
.data-table thead tr.selected.row-sel-bottom th { --sel-b: 2px; }
.data-table tbody tr.selected td:first-child,
.data-table thead tr.selected th:first-child { --sel-l: 2px; }
.data-table tbody tr.selected td:last-child,
.data-table thead tr.selected th:last-child { --sel-r: 2px; }
.data-table tbody tr td.col-selected { background-color: var(--dc-primary-soft); }
.data-table th.col-selected { --sel-t: 2px; background-color: var(--dc-primary-soft); }
/* 选中整列：四边都收口成完整矩形（与框选同一套框线语言） */
.data-table th.col-selected.col-sel-l, .data-table td.col-selected.col-sel-l { --sel-l: 2px; }
.data-table th.col-selected.col-sel-r, .data-table td.col-selected.col-sel-r { --sel-r: 2px; }
.data-table tr.col-sel-bottom td.col-selected { --sel-b: 2px; }
.data-table tbody tr:hover td { background-color: var(--dc-primary-wash); }
/* 底栏选中区汇总：弱化显示、数字加粗 */
.sel-summary { display: inline-flex; align-items: center; gap: 10px; font-size: 12px; color: var(--dc-text-dim); flex-wrap: wrap; margin-left: 4px; }
.sel-summary .ss-item { white-space: nowrap; }
.sel-summary .ss-item b { color: var(--dc-text); font-weight: 600; font-variant-numeric: tabular-nums; }
/* 窗口化占位行：只负责撑高 */
.data-table tbody tr.vp-pad-row { cursor: default; }
.data-table tbody tr.vp-pad-row td { padding: 0; border: 0; background: transparent !important; }
.data-table tbody tr.vp-pad-row:hover td { background: transparent !important; }

.row-num-th, .row-num-td {
  width: 40px; min-width: 40px; max-width: 40px;
  text-align: left; padding: 6px 6px;
  color: var(--dc-text-dim); font-size: 12px;
  border-right: 1px solid var(--dc-border);
}
.row-num-th { background-color: var(--dc-bg-table-head); }
.data-table th.row-num-th { color: var(--dc-text-strong); font-size: 13px; }
.data-table th.leading-th, .data-table td.leading-td {
  width: 40px; min-width: 40px; max-width: 40px;
  text-align: center; padding: 0; vertical-align: middle; line-height: 1;
  border-right: 1px solid var(--dc-border);
}
.data-table th.leading-th { background-color: var(--dc-bg-table-head); cursor: pointer; }
.data-table td.leading-td {
  cursor: pointer; user-select: none;
  color: var(--dc-text-dim); font-size: 12px;
}
.data-table td.leading-td:hover { background-color: var(--dc-primary-wash); color: var(--dc-text); }
.data-table td.leading-td.row-num-on {
  background-color: var(--dc-primary-soft);
  color: var(--dc-primary); font-weight: 600;
}
.row-num-tx { display: inline-block; }
/* 活动单元格（键盘导航锚点） */
.data-table td.active-cell { outline: 2px solid var(--dc-primary); outline-offset: -2px; }
.pager { display: flex; align-items: center; justify-content: space-between; padding: 6px 12px; border-top: 1px solid var(--dc-border); background: var(--dc-bg-soft); flex-shrink: 0; gap: 12px; }
.load-time { font-size: 13px; color: var(--dc-text-dim); font-weight: 500; }
.json-cell { font-family: var(--dc-mono-font, monospace); font-size: 13px; color: #5aa0d8; }

/* 「选择显示字段」下拉（popper 挂到 body，slot 内容仍带本组件 scoped 属性） */
.col-vis-dropdown .col-vis { max-height: 340px; overflow: auto; padding: 4px; min-width: 190px; }
.col-vis-dropdown .col-vis-head {
  display: flex; align-items: center; justify-content: space-between;
  padding: 2px 8px 8px; font-size: 13px; font-weight: 600; color: var(--dc-text-mid, #606266);
  border-bottom: 1px solid var(--dc-border, #ebeef5); margin-bottom: 4px;
}
.col-vis-dropdown .col-vis-item {
  display: flex; width: 100%; margin: 0; padding: 4px 8px; box-sizing: border-box;
  border-radius: 4px; height: auto;
}
.col-vis-dropdown .col-vis-item:hover { background: var(--dc-bg-hover, #f5f7fa); }
.col-vis-dropdown .col-vis-item .el-checkbox__label {
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px;
}
</style>

<style>
/* 表格右键上下文菜单（teleport 到 body，需全局样式）—— 与表预览同一套 */
.grid-ctx-menu {
  position: fixed; z-index: 3000; min-width: 168px;
  max-height: calc(100vh - 16px); overflow: auto;
  background: var(--dc-bg-card, #fff);
  border: 1px solid var(--dc-border, #dcdfe6);
  border-radius: 8px; padding: 4px;
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.16);
  font-size: 13px; color: var(--dc-text, #303133);
  user-select: none;
}
.grid-ctx-menu .ctx-item {
  display: flex; align-items: center; gap: 10px;
  padding: 6px 10px; border-radius: 5px; cursor: pointer; white-space: nowrap;
}
.grid-ctx-menu .ctx-item:hover,
.grid-ctx-menu .ctx-item.ctx-active { background: var(--dc-primary-wash, #ecf5ff); }
.grid-ctx-menu.grid-ctx-sub { z-index: 3100; }
.grid-ctx-menu .ctx-item.ctx-disabled { opacity: 0.45; cursor: not-allowed; }
.grid-ctx-menu .ctx-item.ctx-disabled:hover { background: transparent; }
.grid-ctx-menu .ctx-item.ctx-sep { height: 1px; padding: 0; margin: 4px 6px; background: var(--dc-border, #dcdfe6); cursor: default; }
.grid-ctx-menu .ctx-item.ctx-sep:hover { background: var(--dc-border, #dcdfe6); }
.grid-ctx-menu .ctx-label { flex: 1 1 auto; }
.grid-ctx-menu .ctx-shortcut { flex: 0 0 auto; font-size: 12px; color: var(--dc-text-dim, #909399); }
.grid-ctx-menu .ctx-arrow { flex: 0 0 auto; color: var(--dc-text-dim, #909399); }
</style>
