<template>
  <div class="table-data-view" ref="rootRef">
    <div class="toolbar">
      <div class="left">
        <el-button size="small" :icon="advancedOpen ? ArrowUp : ArrowDown" plain
                   @click="advancedOpen = !advancedOpen"> {{ $t('tdv.advancedSearch') }}{{ activeFilterCount ? `（${activeFilterCount}）` : '' }}
        </el-button>
        <el-button v-if="advancedOpen" size="small" text @click="addFilter">{{ $t('tdv.addFilter') }}</el-button>
        <el-button v-if="advancedOpen" size="small" text @click="resetFilters">{{ $t('common.reset') }}</el-button>
      </div>
      <div class="right">
        <el-button v-if="hasChanges && !readOnly" size="small" type="primary" :icon="Check" @click="saveChanges">{{ $t('common.save') }}</el-button>
        <el-button v-if="hasChanges && !readOnly" size="small" text @click="revertChanges">{{ $t('tdv.revert') }}</el-button>
        <el-button v-if="!readOnly" size="small" text :icon="Plus" :title="$t('shortcut.data.addRow.label')" @click="addRow" />
        <el-button v-if="!readOnly" size="small" text :icon="Minus" :title="selectedSet.size ? $t('tdv.deleteRowsN', { n: selectedSet.size }) : $t('tdv.deleteRow')" :disabled="!selectedSet.size" @click="removeSelectedRows" />
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
        <el-dropdown @command="onExport" style="display:inline-block">
          <el-button size="small" text :icon="Download" :title="$t('qa.exportBtn')" />
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item command="current-csv">{{ $t('qa.exportCurCsv') }}</el-dropdown-item>
              <el-dropdown-item command="current-excel">{{ $t('qa.exportCurExcel') }}</el-dropdown-item>
              <el-dropdown-item divided command="all-csv">{{ $t('qa.exportAllCsv') }}</el-dropdown-item>
              <el-dropdown-item command="all-excel">{{ $t('qa.exportAllExcel') }}</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
        <el-button size="small" text :icon="Refresh" :title="$t('vf.refresh')" @click="refreshData" />
      </div>
    </div>

    <transition name="slide">
      <div v-show="advancedOpen" class="advanced-panel">
        <div v-if="!filters.length" class="empty-tip">
          暂无筛选条件，点击右上 <b>{{ $t('cb.addCondition') }}</b> 开始配置高级搜索
        </div>
        <div v-else class="filter-rows">
          <div v-for="(f, idx) in filters" :key="idx" class="filter-row">
            <el-select v-if="idx > 0" v-model="f.join" size="small" style="width: 80px"
                       @change="onFilterChange">
              <el-option :label="$t('tdv.and')" value="AND" />
              <el-option :label="$t('tdv.or')" value="OR" />
            </el-select>
            <!-- 第一个条件没有「且/或」，用等宽占位保持各行列对齐 -->
            <span v-else class="join-placeholder" aria-hidden="true" />
            <el-select v-model="f.col" size="small" filterable style="width: 180px"
                       @change="onColChange(f)">
              <el-option v-for="c in columnMetas" :key="c.name"
                         :label="c.name" :value="c.name" />
            </el-select>
            <el-select v-model="f.op" size="small" style="width: 150px"
                       @change="onFilterChange">
              <el-option v-for="o in opsOf(f.colType)" :key="o.value"
                         :label="o.label" :value="o.value" />
            </el-select>
            <template v-if="needsTwoValues(f.op)">
              <el-input v-model="f.value" size="small" style="width: 140px"
                        @keyup.enter="load(1)" />
              <span class="between-sep">~</span>
              <el-input v-model="f.value2" size="small" style="width: 140px"
                        @keyup.enter="load(1)" />
            </template>
            <template v-else-if="!isNullOp(f.op)">
              <el-input v-if="f.colType === 'date'" v-model="f.value" size="small" type="date" style="width: 160px" value-format="YYYY-MM-DD"
                        @change="onFilterChange" />
              <el-input v-else v-model="f.value" size="small"
                        style="width: 160px" @keyup.enter="load(1)" />
            </template>
            <el-button size="small" link type="danger" :icon="Delete" :title="$t('tdv.removeFilter')" @click="removeFilter(idx)" />
          </div>
        </div>
        <div v-if="filters.length" class="advanced-foot">
          <el-button type="primary" size="small" @click="load(1)">{{ $t('tdv.applyFilters') }}</el-button>
          <el-button size="small" @click="resetFilters">{{ $t('common.clear') }}</el-button>
        </div>
      </div>
    </transition>

    <div class="grid-area">
      <!-- 加载遮罩：查询/翻页时可取消；其它操作（保存等）只显示进度 -->
      <div v-if="loading" class="grid-loading-overlay">
        <div class="grid-loading-box">
          <el-icon class="is-loading" :size="26"><Loading /></el-icon>
          <span class="grid-loading-text">{{ running ? $t('sqlq.querying') : $t('busy.working') }}</span>
          <el-button v-if="running" size="small" @click="cancelLoad">
            <el-icon style="margin-right:4px"><VideoPause /></el-icon>{{ $t('tree.multiCancel') }} </el-button>
        </div>
      </div>
      <div class="table-scroll" ref="gridRef" @scroll="onTableScroll">
        <div v-if="displayRows.length" class="data-table-wrap" ref="gridWrap" tabindex="0"
             @mousemove="onGridMove" @mouseleave="onGridLeave" @mousedown="onGridDown"
             @scroll="onTableScroll" @contextmenu.prevent="onGridContextMenu">
          <table class="data-table" :class="{ 'col-resizing': colResizing }"
                 :style="{ width: tableWidth + 'px' }">
            <colgroup>
              <col class="row-num-col" />
              <col v-for="col in visibleColumns" :key="col"
                   :style="{ width: renderColWidth(col) + 'px' }" />
            </colgroup>
            <thead>
              <tr :class="{ 'selected': headerSelected, 'row-sel-top': headerSelected, 'row-sel-bottom': selEdges.headerBottom }">
                <!-- 左上角（原全选复选框位置）= 标题行的行头：单击选中标题行，按住往下拖可连选数据行 -->
                <th class="row-num-th leading-th" :class="{ 'row-num-on': headerSelected }"
                    :title="$t('sqlq.headerRowTitle')"
                    @mousedown.prevent="onHeaderRowDown($event)"><span class="row-num-tx">#</span></th>
                <th v-for="(col, ci) in visibleColumns" :key="col" :data-gkey="'0:' + ci"
                    :title="col + $t('sqlq.colTitleSuffix')"
                    :class="{
                      'sortable': true,
                      'sort-asc': orderColumn === col && orderDir === 'ASC',
                      'sort-desc': orderColumn === col && orderDir === 'DESC',
                      'col-selected': selectedCols.has(col),
                      'col-sel-l': selEdges.colLeft.has(col),
                      'col-sel-r': selEdges.colRight.has(col)
                    }"
                    @mousedown="onColDragStart(col, $event)"
                    @click="onHeaderClick(col, $event)"
                    @dblclick="onHeaderDblClick(col, $event)"
                    @contextmenu.prevent.stop="onHeaderContextMenu($event, col)">
                  <!-- 表头两行：右侧竖排「字段名 /（🔑 +）字段注释」，左侧类型徽章垂直居中跨两行。
                       文字块的可用宽度按「列宽 − 表头非文字部分（内边距 + 排序 + 徽章）」算，与列宽测量同一笔账：
                       超宽就在这个宽度里省略成 …，而不是反过来把列撑宽。 -->
                  <span class="th-text" :style="{ maxWidth: labelMaxWidth(col) + 'px' }">
                    <!-- 布局（用户口径）：第一行「类型徽章 + 字段名（主键列再加 🔑）」，
                         第二行注释**顶格**开始（不缩进）；没有注释时第二行不渲染 -->
                    <span class="th-line1">
                      <span class="th-type-ic" :class="typeClass(col)" :title="typeTitle(col)"><el-icon><component :is="typeIcon(col)" /></el-icon></span>
                      <span class="th-label">{{ col }}</span>
                      <span v-if="isPkCol(col)" class="th-pk-ic" :title="$t('tf.colPrimary')"><el-icon><Key /></el-icon></span>
                    </span>
                    <span v-if="colComment(col)" class="th-line2">
                      <span class="th-comment" :title="colComment(col)">{{ colComment(col) }}</span>
                    </span>
                  </span>
                  <span class="th-sort" :class="{ 'is-sorted': orderColumn === col }"
                        :title="orderColumn === col ? (orderDir === 'ASC' ? $t('sqlq.sortAscTitle') : $t('sqlq.sortDescTitle')) : $t('sqlq.sortNoneTitle')"
                        @mousedown.stop @click.stop="toggleSort(col)">
                    <el-icon v-if="orderColumn !== col"><Sort /></el-icon>
                    <el-icon v-else-if="orderDir === 'ASC'"><SortUp /></el-icon>
                    <el-icon v-else><SortDown /></el-icon>
                  </span>
                  <span class="col-resizer" :title="$t('tdv.colResizeTip')"
                        @mousedown.stop.prevent="onColResizeStart(col, $event)"
                        @dblclick.stop="autoFitCol(col)" />
                </th>
              </tr>
            </thead>
            <tbody>
              <!-- 窗口化渲染：上方占位行，撑起未渲染区域的高度 -->
              <tr v-if="padTop > 0" class="vp-pad-row" aria-hidden="true">
                <td :colspan="visibleColumns.length + 1" :style="{ height: padTop + 'px' }" />
              </tr>
              <tr v-for="row in visibleRows" :key="rowKey(row)" :data-rid="row._rid"
                  :class="{
                    'selected': isRowSelected(row),
                    'row-alt': rowIndex(row) % 2 === 1,
                    'row-new': row._editState === 'new',
                    'row-modified': row._editState === 'modified',
                    'row-deleted': row._editState === 'deleted',
                    'row-sel-top': selEdges.rowTop.has(row._rid),
                    'row-sel-bottom': selEdges.rowBottom.has(row._rid),
                    'col-sel-bottom': row._rid === lastDisplayRid
                  }">
                <!-- 行号列（Excel 行头）：单击选中该行、Ctrl 切换、Shift 连选、按住拖动连选多行；双击查看整行详情 -->
                <td class="row-num-td leading-td" :class="{ 'row-num-on': isRowSelected(row) }"
                    :title="'第 ' + (rowIndex(row) + 1) + ' 行（按住拖动可连选多行；双击查看整行详情）'"
                    @mousedown.prevent="onRowNumDown(row, $event)"
                    @dblclick.stop="openRowDetail(row)"
                    @contextmenu.prevent.stop="onRowContextMenu($event, row)">
                  <span class="row-num-tx">{{ rowIndex(row) + 1 }}</span>
                </td>
                <td v-for="(col, ci) in visibleColumns" :key="col" :data-gkey="(rowIndex(row) + 1) + ':' + ci"
                    :class="[cellAlignClass(row[col], colTypeMap[col]), { 'null-cell': row[col] == null, 'col-selected': selectedCols.has(col), 'col-sel-l': selEdges.colLeft.has(col), 'col-sel-r': selEdges.colRight.has(col), 'active-cell': activeCell && activeCell.row === row && activeCell.col === col && noBulkSelection, 'editing': editingCell && editingCell.row === row && editingCell.col === col }]"
                    :title="formatCell(row[col])"
                    @dblclick="startEdit(row, col, $event)"
                    @contextmenu.prevent.stop="onCellContextMenu($event, row, col)"
                    @click="onCellClick(row, col, $event)">
                  <template v-if="editingCell && editingCell.row === row && editingCell.col === col">
                    <input ref="cellInputRef" v-model="editingCell.value"
                           class="cell-input"
                           @keydown.enter="onCellEnter"
                           @keydown.tab.prevent="onCellTab"
                           @keydown.esc="cancelEdit"
                           @blur="confirmEdit" />
                  </template>
                  <template v-else>
                    <!-- v-memo：内容没变就跳过该格的 vnode 创建与 diff。
                         光标移动 / 选中变化时，上千个未变单元格的文本子树不再重建（Vue 仍会 patch 外层 td 的选中类，那一步很轻）。
                         memo key 用原始值 row[col]：值变了 key 必变 → 一定重渲染，绝不会显示旧值。 -->
                    <span v-memo="[row[col], querySettingsLive.nullStyle]">{{ formatCell(row[col]) }}</span>
                  </template>
                </td>
              </tr>
              <!-- 窗口化渲染：下方占位行 -->
              <tr v-if="padBottom > 0" class="vp-pad-row" aria-hidden="true">
                <td :colspan="visibleColumns.length + 1" :style="{ height: padBottom + 'px' }" />
              </tr>
            </tbody>
          </table>
        </div>
        <el-empty v-else :description="$t('tdv.noData')" />
      </div>

      <!-- 分页栏：与表格整合在同一个容器底部 -->
      <div class="pager">
        <span class="load-time">
          {{ loading ? formatElapsed(elapsedTime) + $t('tdv.queryingSuffix') : (elapsedTime > 0 ? formatElapsed(elapsedTime) : '') }}
          <span v-if="hasChanges" class="change-tip">{{ $t('tdv.unsavedChanges') }}</span>
        </span>
        <!-- 选中区汇总（与 SQL 结果表底栏同一套做法）：框选单元格 / 选中整行 / 选中整列时给出
             格子数 / 求和 / 均值 / 最小 / 最大，只统计**数值类型**的列（字符串相加没有意义） -->
        <span v-if="selectionSummary" class="sel-summary" :title="$t('sqlq.summaryTitle')">
          <span class="ss-item">选中 <b>{{ selectionSummary.cells }}</b> 格</span>
          <template v-if="selectionSummary.nums">
            <span class="ss-item">{{ $t('sqlq.sum') }} <b>{{ fmtSummaryNum(selectionSummary.sum) }}</b></span>
            <span class="ss-item">{{ $t('sqlq.avg') }} <b>{{ fmtSummaryNum(selectionSummary.avg) }}</b></span>
            <span class="ss-item">{{ $t('sqlq.min') }} <b>{{ fmtSummaryNum(selectionSummary.min) }}</b></span>
            <span class="ss-item">{{ $t('sqlq.max') }} <b>{{ fmtSummaryNum(selectionSummary.max) }}</b></span>
          </template>
        </span>
        <el-pagination background size="small" layout="total, sizes, prev, pager, next, jumper"
                       :total="total" :page-size="size" :current-page="page"
                       :page-sizes="pageSizes"
                       @current-change="goPage" @size-change="s => { size = s; load(1) }"
                       style="margin-left: auto" />
      </div>

      <!-- 右键上下文菜单（支持二级子菜单） -->
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
    </div>


  </div>

  <TaskProgressDialog
    v-model:visible="exportTask.visible"
    task-kind="export"
    :target-name="props.table || $t('tdv.tableData')"
    :status="exportTask.status"
    :done="exportTask.done"
    :total="exportTask.total"
    :phase="exportTask.phase"
    :message="exportTask.message"
    :logs="exportTask.logs"
    :canceling="exportTask.canceling"
    @cancel="exportTask.cancel(props.conn.id)"
    @close="exportTask.close" />

  <!-- 行详情：双击 / 右键行号查看整行字段明细 -->
  <CellDetailDialog v-model="rowDetail.visible" :title="rowDetail.title" :text="rowDetail.text" />
</template>

<script setup>
import { ref, watch, onMounted, onUnmounted, onBeforeUnmount, computed, nextTick } from 'vue'
import { t } from '../../utils/i18n'
import { querySettingsLive } from '../../utils/settings'
import { ElMessage, ElMessageBox } from 'element-plus'
import { ArrowUp, ArrowDown, Delete, Sort, SortUp, SortDown, Plus, Minus, Check, Refresh, Download, Operation, Loading, VideoPause, Histogram, Calendar, Switch as SwitchIcon, Document, Tickets, Grid, Key } from '@element-plus/icons-vue'
import { getTableData, listColumns, saveTableData, aiFilter, exportData } from '../../api'
import TaskProgressDialog from '../../common/TaskProgressDialog.vue'
import CellDetailDialog from '../../common/CellDetailDialog.vue'
import { useExportTask, saveExportBlob } from '../../utils/useExportTask'
import { getQuerySettings } from '../../utils/settings'
import { useShortcutScope } from '../../utils/useShortcuts'
import { useExcelSelection } from '../../utils/excelSelection'
import { cellAlignClass } from '../../utils/cellAlign'
import { formatDbValue, nullDisplay } from '../../utils/cellValue'
import { quoteStyleOf } from '../../types'

const props = defineProps({ conn: Object, database: String, table: String, readOnly: Boolean })
const emit = defineEmits(['update-table-rows', 'open-query'])

// ========== AI 自然语言筛选 ==========
const aiFilterOpen = ref(false)
const aiFilterText = ref('')
const aiFilterLoading = ref(false)
const aiFilterSql = ref('')
const aiFilterNotes = ref('')

const doAiFilter = async () => {
  if (!props.conn || !props.table) { ElMessage.warning(t('tdv.missingConnOrTable')); return }
  aiFilterLoading.value = true
  aiFilterSql.value = ''
  aiFilterNotes.value = ''
  try {
    const res = await aiFilter({
      connectionId: props.conn.id,
      database: props.database || '',
      table: props.table,
      requirement: aiFilterText.value
    })
    if (res && res.success) {
      aiFilterSql.value = res.sql || ''
      aiFilterNotes.value = res.notes || ''
    } else {
      ElMessage.error(res?.message || t('mv.genFailed'))
    }
  } catch (e) {
    ElMessage.error(e?.message || e?.toString?.() || t('mv.genFailed'))
  }
  aiFilterLoading.value = false
}

const copyAiFilterSql = async () => {
  try { await navigator.clipboard.writeText(aiFilterSql.value); ElMessage.success(t('common.copied')) } catch (e) { ElMessage.error(t('sqlq.copyFailed')) }
}

const openAiFilterInQuery = () => {
  emit('open-query', { connId: props.conn?.id || '', database: props.database || '', sql: aiFilterSql.value })
  aiFilterOpen.value = false
}
const qs = getQuerySettings()

const rows = ref([])
const columns = ref([])
// 列显示/隐藏：hiddenColumns 保存被隐藏的列名，visibleColumns 为实际渲染的列
const hiddenColumns = ref(new Set())
// 被临时隐藏的行（只影响显示，不影响保存/导出；换表或重新加载后自然失效，因为 _rid 会重新生成）
const hiddenRows = ref(new Set())
// 被标记删除的行（_editState==='deleted'）。单独用集合记录，方便 displayRows 只依赖
// 「集合 + rows 身份」而不去读每一行的 _editState 属性 —— 否则每次编辑单元格都会因为
// 读到某行的 _editState 而让 displayRows 重算（重建整张表数组 → 可视窗口整片重建）。
const deletedRids = ref(new Set())
const visibleColumns = computed(() => columns.value.filter(c => !hiddenColumns.value.has(c)))
const toggleColumnVisible = (col) => {
  const s = new Set(hiddenColumns.value)
  if (s.has(col)) {
    s.delete(col)
  } else {
    if (columns.value.length - s.size <= 1) { ElMessage.warning(t('sqlq.keepOneCol')); return }
    s.add(col)
  }
  hiddenColumns.value = s
}
const showAllColumns = () => { hiddenColumns.value = new Set() }
const columnMetas = ref([])
// 列名 → 原始字段类型串：NULL 单元格按字段类型对齐用（模板里逐格取，必须 O(1)）
const colTypeMap = computed(() => {
  const m = {}
  for (const meta of columnMetas.value) {
    if (meta && meta.name) m[meta.name] = meta.type || ''
  }
  return m
})
// ========== 表头字段类型（图标 + 悬浮显示完整类型） ==========
// 列名 → 字段元信息：表头每列都会查类型/主键，建个 Map 做 O(1) 查表（原来每列都 .find 扫一遍）
const metaByName = computed(() => {
  const m = new Map()
  for (const meta of columnMetas.value) if (meta && meta.name) m.set(meta.name, meta)
  return m
})
const metaOf = (col) => metaByName.value.get(col)
const typeTitle = (col) => {
  const m = metaOf(col)
  if (!m) return ''
  return (m.primaryKey ? t('tdv.primaryKey') : '') + (m.type || '')
}
const isPkCol = (col) => !!metaOf(col)?.primaryKey
/** 字段注释（表头第二行用）；没有注释时返回空串，那一行就不渲染 */
const colComment = (col) => String(metaOf(col)?.comment || '').trim()
const typeIcon = (col) => {
  const t = String(metaOf(col)?.type || '').toLowerCase()
  if (/bool/.test(t)) return SwitchIcon
  if (/json/.test(t)) return Tickets
  if (/(blob|binary|bytea|image|raw)/.test(t)) return Document
  if (/^(int|bigint|smallint|tinyint|mediumint|decimal|numeric|float|double|real|number|bit|money)/.test(t)) return Histogram
  if (/^(date|time|datetime|timestamp|year)/.test(t)) return Calendar
  if (/^(char|varchar|text|string|clob|enum|set|uuid|nchar|nvarchar)/.test(t)) return Document
  return Grid
}
// 类型族 → 徽章配色类（与 typeIcon 同一套判定；全局 CSS 按类着色）
const typeClass = (col) => {
  const t = String(metaOf(col)?.type || '').toLowerCase()
  if (/bool/.test(t)) return 'th-t-bool'
  if (/json/.test(t)) return 'th-t-json'
  if (/(blob|binary|bytea|image|raw)/.test(t)) return 'th-t-blob'
  if (/^(int|bigint|smallint|tinyint|mediumint|decimal|numeric|float|double|real|number|bit|money)/.test(t)) return 'th-t-num'
  if (/^(date|time|datetime|timestamp|year)/.test(t)) return 'th-t-date'
  return 'th-t-text'
}
const loading = ref(false)
const running = ref(false) // 数据加载（翻页 / 查询）进行中，可取消
let loadController = null
// 导出走异步任务，显示进度条和实时日志
const exportTask = useExportTask()
const total = ref(0)
const page = ref(1)
const size = ref(qs.pageSize > 0 ? qs.pageSize : 1000)
const pageSizes = computed(() => {
  const base = [1000, 2000, 5000, 10000]
  if (qs.pageSize > 0 && !base.includes(qs.pageSize)) base.unshift(qs.pageSize)
  return base.sort((a, b) => a - b)
})
const orderColumn = ref('')
const orderDir = ref('')
const gridRef = ref(null)
const rootRef = ref(null)
const elapsedTime = ref(0)
let loadTimer = null
const formatElapsed = (ms) => {
  if (ms < 1000) return `${ms}ms`
  return `${(ms / 1000).toFixed(2)}s`
}

const advancedOpen = ref(false)
const filters = ref([])
const activeFilterCount = computed(() => filters.value.length)

// ========== 编辑状态 ==========
const selectedRowIndex = ref(-1)
const editingCell = ref(null)
const cellInputRef = ref(null)

const hasChanges = computed(() => rows.value.some(r => r._editState === 'new' || r._editState === 'modified' || r._editState === 'deleted'))

// 显示的行（过滤掉已删除的和手动隐藏的；都保留在 rows 里用于保存）
// 只依赖 rows 身份 + deletedRids/hiddenRows 两个集合：单元格值编辑只改行内字段，
// 不碰这两个集合，所以 displayRows 不会在逐字符输入时重算（否则重建数组会让可视窗口整片重渲染）
const displayRows = computed(() => rows.value.filter(r => !deletedRids.value.has(r._rid) && !hiddenRows.value.has(r._rid)))

// _rid → displayRows 下标，供拖选时由 DOM 行反查（O(1)）。
// 以前每次 mousemove 都 findIndex 扫全表，上万行时一次拖选就是几十万次比较
const ridToDispIdx = computed(() => {
  const m = new Map()
  displayRows.value.forEach((r, i) => m.set(String(r._rid), i))
  return m
})

// ========== 窗口化渲染（大表只渲染可视区行） ==========
// 行数少时保持原样整表渲染；超过阈值后只渲染可视区 ± 缓冲行，上下用占位行撑起滚动高度。
// 与 SqlQueryView 的虚拟滚动同一思路（固定行高），避免几千行 × 几十列产生的几十万 DOM 节点。
const VP_ROW_H = 32      // 单行高度（px）：td padding 6px + 12px 字号，单元格 nowrap 保证等高
const VP_BUFFER = 12     // 可视区上下各多渲染的缓冲行
// 行数超过该值才启用窗口化。原来是 500 —— 可默认页大小就是 200 行，于是永远够不着：
// 200 行 × 13 列 = 2600 个格子全部渲染，任何一次列宽变化都要给整张表重新排版
// （实测拖动列宽帧间隔中位 24ms、P90 51ms；SQL 结果表只渲染 31 行，同样的动作只要 8ms）。
// 降到 60：小表（≤50 行）行为完全不变，100/200 行/页则只渲染可视区 ~40 行，重排成本随之降到 ~1/5。
const VP_THRESHOLD = 60
const vpStart = ref(0)
const vpEnd = ref(100)
let vpRaf = 0
const virtualEnabled = computed(() => displayRows.value.length > VP_THRESHOLD)
const visibleRows = computed(() => {
  const all = displayRows.value
  if (!virtualEnabled.value) return all
  const start = Math.min(vpStart.value, Math.max(0, all.length - 1))
  const end = Math.max(start, Math.min(vpEnd.value, all.length))
  return all.slice(start, end)
})
const padTop = computed(() => (virtualEnabled.value ? Math.min(vpStart.value, displayRows.value.length) * VP_ROW_H : 0))
const padBottom = computed(() => {
  if (!virtualEnabled.value) return 0
  return Math.max(0, displayRows.value.length - Math.min(vpEnd.value, displayRows.value.length)) * VP_ROW_H
})
// 实际承载滚动的容器：内层 .data-table-wrap 高度为 100% 且 overflow:auto，正常是它滚动；
// 极端情况下（外层被撑高）回退到外层 .table-scroll，避免滚动事件收不到。
const scrollHost = () => {
  const w = gridWrap.value
  if (w && w.scrollHeight > w.clientHeight) return w
  return gridRef.value || w
}
const syncViewport = () => {
  const total = displayRows.value.length
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

// 如果通过单元格框选选中了某行，也允许删除该行
const selectedGridRowIndex = computed(() => {
  const r = gridSel?.range?.value
  if (!r) return -1
  // r1 是 DOM 行号（0 = 表头），窗口化渲染下要加上可视区起始偏移才是 rows 里的真实下标
  const domRow = Math.max(r.r1, 1) - 1
  const dataRow = virtualEnabled.value ? vpStart.value + domRow : domRow
  if (dataRow < 0 || dataRow >= rows.value.length) return -1
  return dataRow
})
const effectiveSelectedRowIndex = computed(() =>
  selectedRowIndex.value >= 0 ? selectedRowIndex.value : selectedGridRowIndex.value
)

// ========== 行身份 ==========
// 旧实现：行号 = rows.indexOf(row)（模板里逐行逐单元格调用 → O(N²×列数)），
//         v-for key = JSON.stringify(_original)（每行每次渲染都做一次全对象序列化）。
// 1000 行 × 30 列时是数千万次线性比较 + 上千次 stringify，是表格卡顿的主因。
// 现在改为加载时给每行打上稳定的 _rid（v-for key）与 _idx（在 rows 中的下标），读取全为 O(1)。
let ridSeq = 0
const nextRid = () => 'r' + (++ridSeq)
const rowIndex = (row) => row._idx
const rowKey = (row) => row._rid
// 从服务端行数据生成带身份信息的行对象（保留 _original 快照用于脏检查与撤销）
const stampRows = (list) => list.map((r, i) => ({
  ...r, _editState: 'original', _original: { ...r }, _rid: nextRid(), _idx: i
}))
// 行被真正移除（撤销新增行）后重排下标，保证行号与选中逻辑仍对应 rows 中的位置
const rebuildRowIndex = () => { rows.value.forEach((r, i) => { r._idx = i }) }

// ========== 行多选（行号列点/拖 + Shift/Ctrl/Cmd 范围与开关） ==========
const selectedSet = ref(new Set())
// 行选择锚点：null = 无，-1 = 标题行，>=0 = 数据行在 displayRows 里的下标
const lastAnchorIdx = ref(null)
const headerSelected = ref(false) // 标题行是否也被选中（点/拖左侧行号列最上面那格）
const isRowSelected = (row) => selectedSet.value.has(row._rid)

// ========== 选中区汇总（底栏，与 SQL 结果表同一套做法）==========
// 优先级：单元格区域 > 选中整行 > 选中整列（同一时刻只有一块选区，见各 focus*/clear* 函数）。
// 计数按"格子数"给（与 Excel 一致），求和/均值/最小/最大只统计**数值类型**的列。
const NUMERIC_SUMMARY_RE = /^(int|bigint|smallint|tinyint|mediumint|decimal|numeric|float|double|real|number|bit|money|serial)/i
// 类型拿不到时的兜底判定：值本身是**严格数字面量**才算（别把字符串硬加起来）。
// 有的源 /columns 返回空（Doris 的 JDBC getColumns 踩过），汇总不能跟着哑掉。
const NUMERIC_LITERAL_RE = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/
/** 汇总数字显示：整数不带小数点，小数最多两位（均值常常是除出来的） */
const fmtSummaryNum = (n) => (n == null || !Number.isFinite(n)) ? '' : (Number.isInteger(n) ? String(n) : Number(n.toFixed(2)).toLocaleString())

const selectionSummary = computed(() => {
  // gridSel.range 是框选模块暴露的响应式矩形；gkey 行 0 是标题行，数据行 = displayRows[r-1]
  // （与模板里的 :data-gkey="(rowIndex(row) + 1) + ':' + ci" 对应）
  const rect = gridSel && gridSel.range ? gridSel.range.value : null
  const cols = visibleColumns.value || []
  const rows = displayRows.value || []
  const acc = { cells: 0, nums: 0, sum: 0, min: null, max: null }
  const add = (r, c) => {
    if (r <= 0) return
    const col = cols[c]
    const row = rows[r - 1]
    if (col === undefined || !row) return
    acc.cells++
    const v = row[col]
    if (v == null) return
    // 数值判定：类型已知按类型；类型缺失（/columns 返回空的源）看值本身 ——
    // 严格数字面量才参与求和，普通文本列不会被误加
    const type = String(colTypeMap.value[col] || '').toLowerCase()
    if (type) {
      if (!NUMERIC_SUMMARY_RE.test(type)) return
    } else if (!NUMERIC_LITERAL_RE.test(String(v).trim())) return
    const n = Number(v)
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
    // 行选集合存的是 _rid（不是下标），先建一张 rid → displayRows 下标的表
    const byRid = new Map(rows.map((row, i) => [row._rid, i]))
    for (const rid of selectedSet.value) {
      const i = byRid.get(rid)
      if (i == null) continue
      for (let c = 0; c < cols.length; c++) add(i + 1, c)
    }
  } else if (selectedCols.value.size) {
    for (let i = 0; i < rows.length; i++) {
      for (let c = 0; c < cols.length; c++) if (selectedCols.value.has(cols[c])) add(i + 1, c)
    }
  } else {
    return null
  }
  if (!acc.cells) return null
  return {
    cells: acc.cells,
    nums: acc.nums,
    sum: acc.sum,
    avg: acc.nums ? acc.sum / acc.nums : null,
    min: acc.min,
    max: acc.max
  }
})
const toggleRowSelect = (row) => {
  const s = new Set(selectedSet.value)
  if (s.has(row._rid)) s.delete(row._rid); else s.add(row._rid)
  selectedSet.value = s
}
const onRowClick = (row, e) => {
  if (Date.now() - lastResizeAt < 300) return
  if (gridSel?.suppressClick()) return
  // 用 displayRows 下标（不是绝对 _idx）：下面的连选范围也按显示行取值，两者必须同一套坐标
  const idx = dispIdxOf(row)
  if (e.shiftKey && lastAnchorIdx.value != null) {
    // Shift 连选：只做扩展，**锚点保持不动** ——
    // 否则第二次 Shift+点会以「上一次点到的那行」为起点（1~8 后再点 9 会变成 8~9）
    // 连选可能从标题行(-1)开始，交给 selectRowRange（它会正确设置 headerSelected）
    selectRowRange(lastAnchorIdx.value, idx)
    return
  }
  if (e.ctrlKey || e.metaKey) {
    // 加减选也算「重新选数据行」：标题行的高亮要一起撤掉
    headerSelected.value = false
    toggleRowSelect(row)
  } else {
    // 单击选中数据行：标题行不能还亮着（否则看着像两块选区）
    headerSelected.value = false
    selectedSet.value = new Set([row._rid])
  }
  lastAnchorIdx.value = idx
}

// ========== 行号列（Excel 行头）：按住拖动连选多行 ==========
// 拖动用 document mousemove + elementFromPoint 判定行，鼠标划到数据区也能继续按行扩展，
// 拖快时不会漏行（相比逐格 mouseenter）
let rowDrag = null
// a/b 为行下标：-1 表示标题行，>=0 为 displayRows 下标
const selectRowRange = (a, b) => {
  const rows = displayRows.value
  const lo = Math.min(a, b)
  const hi = Math.max(a, b)
  headerSelected.value = lo <= -1 // 范围含标题行 → 标题行一起选中
  const i1 = Math.max(0, lo)
  const i2 = Math.min(rows.length - 1, hi)
  // 拖选（rowDrag 存在）时增量增删：每帧只碰「区间两端变化的那几行」。
  // 以前每帧 new Set(整段) 再替换 ref —— ref 一换，所有读过它的单元格（可视区上千格）
  // 全部重渲染；改成在原 Set 上增量修改后，只有成员真的变了的那几行会更新。
  // 区间状态挂在本轮拖拽对象上：每轮拖拽都是新对象，天然不会串到上一次的区间。
  const drag = rowDrag
  if (drag && drag.lo != null && drag.count === rows.length) {
    const s = selectedSet.value
    for (let i = drag.lo; i < i1; i++) s.delete(rows[i]._rid)
    for (let i = Math.max(i2 + 1, drag.lo); i <= drag.hi; i++) s.delete(rows[i]._rid)
    for (let i = i1; i <= i2; i++) s.add(rows[i]._rid)
    drag.lo = i1
    drag.hi = i2
    drag.count = rows.length
    return
  }
  const s = new Set()
  for (let i = i1; i <= i2; i++) s.add(rows[i]._rid)
  selectedSet.value = s
  if (drag) { drag.lo = i1; drag.hi = i2; drag.count = rows.length }
}
// 左上角（原全选复选框位置）= 标题行的行头：单击选中标题行，按住往下拖连选数据行
const onHeaderRowDown = (e) => {
  if (e.button !== 0) return // 右键交给 contextmenu 处理，别当成左键单击去打散选中
  if (editingCell.value) confirmEdit()
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
  if (gridWrap.value && gridWrap.value.focus) gridWrap.value.focus({ preventScroll: true })
}
/**
 * 框选区域是否盖到标题行（第 0 行）。
 *
 * selEdges 只依赖这个布尔值，**不能让它直接读 range 对象**：
 * 框选拖动/自动滚动时 paint() 每帧都会重绘，range 的新旧值一旦参与依赖，
 * selEdges 就会每帧重算并返回新对象，进而把可视区上千个单元格全部重渲染一次。
 * 布尔值在「没变」时不会传播（Vue computed 的值稳定性），所以这里挡得住。
 */
const rangeCoversHeader = computed(() => {
  const r = gridSel?.range?.value
  return !!r && r.r1 === 0
})

// 行/列选中的「外沿」：每条连续选中的行段、列段只给首尾两端加边线，
// 整块合起来是一个框（与单元格框选同一套观感），中间不画内部线
const selEdges = computed(() => {
  const list = displayRows.value
  const rowTop = new Set()
  const rowBottom = new Set()
  // 标题行算「同一块选区的一部分」的两种情况：点/拖标题行行头选中它，
  // 或者框选区域覆盖到了第 0 行（Shift+点表头就会这样）——
  // 两种都要让首行数据的上边线让给标题行，否则会画出上下两条线、看着像两块
  const headOn = headerSelected.value || rangeCoversHeader.value
  // 只扫「当前真正渲染出来的那一段」：窗口化下未渲染的行不需要边线，
  // 全表扫描（一次上万行、每帧重算）纯属白烧 CPU，是拖选/滚动发卡的主因之一。
  // 扫窗口 ±1 行是为了让首尾两行的「上/下边线」判定能读到相邻行。
  const from = virtualEnabled.value ? Math.max(0, vpStart.value - 1) : 0
  const to = virtualEnabled.value ? Math.min(list.length, Math.max(vpEnd.value + 1, from + 1)) : list.length
  for (let i = from; i < to; i++) {
    if (!selectedSet.value.has(list[i]._rid)) continue
    // 上方相邻为「标题行（也选中）」时，首行的上边线让给标题行
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
  // 标题行被选中：上边线永远在它自己身上；下边线只在「紧邻的首个数据行没被选中」时收口
  const headerBottom = headOn && !(list[0] && selectedSet.value.has(list[0]._rid))
  return { rowTop, rowBottom, colLeft, colRight, headerBottom }
})
// 列选中时，只在「最后一行」收底边（中间行不封口）
const lastDisplayRid = computed(() => {
  const list = displayRows.value
  return list.length ? list[list.length - 1]._rid : null
})

// ===== 整表只有一块选中区域：任何一类新选择开始前，先清掉另外两类 =====
// 三类：行选择（selectedSet）/ 列选择（selectedCols）/ 单元格框选（gridSel）
const clearRowSelection = () => {
  if (selectedSet.value.size) selectedSet.value = new Set()
  if (headerSelected.value) headerSelected.value = false
}
const clearColSelection = () => { if (selectedCols.value.size) clearColSelect() }
const clearGridRange = () => { if (gridSel && gridSel.hasSelection()) gridSel.clearRange() }
// 选中行：清掉列、框选与活动单元格 —— 任何时候都只能有一块选区
const focusRows = () => { clearColSelection(); clearGridRange(); activeCell.value = null }
// 选中列：清掉行、框选与活动单元格
const focusCols = () => { clearRowSelection(); clearGridRange(); activeCell.value = null }
// 框选单元格：清掉行与列
const focusCells = () => { clearRowSelection(); clearColSelection() }

const onRowNumDown = (row, e) => {
  if (e.button !== 0) return // 右键交给 contextmenu 处理，别当成左键单击去打散选中
  if (editingCell.value) confirmEdit()
  if (gridSel && gridSel.suppressClick()) return
  focusRows()
  const prevAnchor = lastAnchorIdx.value
  onRowClick(row, e) // 复用：Shift 连选 / Ctrl 切换 / 单击单选
  if (gridWrap.value && gridWrap.value.focus) gridWrap.value.focus({ preventScroll: true })
  if (e.ctrlKey || e.metaKey) return // Ctrl 单击是加减选，不进入拖拽
  rowDrag = { anchor: e.shiftKey && prevAnchor != null ? prevAnchor : dispIdxOf(row) }
  document.addEventListener('mousemove', onRowNumDragMove)
  document.addEventListener('mouseup', onRowNumDragEnd, { once: true })
}
let rowDragRaf = 0
let rowDragMoved = false
const onRowNumDragMove = (e) => {
  if (!rowDrag) return
  // 指针位置立刻更新：自动滚动要用最新位置，不能被合帧延迟
  rowDragPoint = { x: e.clientX, y: e.clientY }
  rowDragMoved = true
  // 自动滚动的调度必须每次都走：它到边缘才开始循环、离开边缘就停，
  // 重新贴回边缘要能再被唤醒（放在下面的提前 return 之后就会漏掉）
  if (!rowDragScrollRaf) rowDragScrollRaf = requestAnimationFrame(rowDragAutoScroll)
  if (rowDragRaf) return
  // 合并到一帧：高刷鼠标的 mousemove 能到 100+ 次/秒，而下面要做的
  // elementFromPoint 会强制同步布局，逐次处理必然掉帧
  rowDragRaf = requestAnimationFrame(() => {
    rowDragRaf = 0
    if (rowDrag) extendRowDragTo(rowDragPoint.x, rowDragPoint.y)
  })
}
// 指针下的行 → 从锚点连选到该行；指针已经离开表格（拖到表头上方/分页栏）时按首/末行处理
const extendRowDragTo = (x, y) => {
  if (!rowDrag) return
  const el = document.elementFromPoint(x, y)
  let idx = -2
  if (el && el.closest) {
    if (el.closest('thead')) idx = -1 // 指到标题行
    else {
      const tr = el.closest('tr[data-rid]')
      // _rid 是字符串（'r12'），按字符串比对；用 Number() 会得到 NaN 永远匹配不上
      const rid = tr ? tr.getAttribute('data-rid') : null
      if (rid != null) {
        const hit = ridToDispIdx.value.get(String(rid))
        if (hit != null) idx = hit
      }
    }
  }
  if (idx === -2) {
    // 指针离开了表格：上方算标题行，下方算最后一行
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
// 拖到表格上下边缘附近时自动滚动，长表也能一路拖到底
let rowDragScrollRaf = 0
let rowDragPoint = { x: 0, y: 0 }
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
  // 松手时把还没落到选区里的最后一帧补上（拖得快时鼠标先松开、帧还没跑）
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

// ========== 列头拖拽排序（reorder） ==========
// 拖拽期间直接操作 DOM class，不写响应式状态，避免每次 dragover 触发整表重渲染导致卡死
let colDragState = null // { col, startX, startY, moved, sourceEl, overEl }
const DRAG_THRESHOLD = 5
const isColDragging = () => !!(colDragState && colDragState.moved)

const onColDragStart = (col, e) => {
  if (e.button !== 0) return
  // 贴右/左缘是列宽缩放热区（交给 .col-resizer 处理），不要启动列排序，
  // 否则用户想缩放却抓到了表头主体 → 触发排序，松手被取消 → 看着像「宽度弹回自适应」
  if (colAtEdge(e)) return
  // Shift+点击表头 = 扩展「单元格区域」（含标题行），交给框选模块，别切到列选中
  if (e.shiftKey && gridSel && gridSel.hasSelection()) return
  if (colAtEdge(e)) return // 靠近列右缘交给列宽拖拽
  // 直接拖动 = 连选多列（Excel 表头行为）；Alt+拖动 = 调整列顺序
  if (!e.altKey) { onColSelectDown(col, e); return }
  colDragState = { col, startX: e.clientX, startY: e.clientY, moved: false, sourceEl: e.currentTarget, overEl: null }
  document.addEventListener('mousemove', onColDragMoveDoc)
  document.addEventListener('mouseup', onColDragEndDoc)
}
const onColDragMoveDoc = (e) => {
  const st = colDragState
  if (!st) return
  if (!st.moved) {
    if (Math.abs(e.clientX - st.startX) < DRAG_THRESHOLD && Math.abs(e.clientY - st.startY) < DRAG_THRESHOLD) return
    st.moved = true
    if (st.sourceEl) st.sourceEl.classList.add('col-dragging')
    document.body.style.cursor = 'grabbing'
  }
  // 指针下的表头单元格 -> 高亮落点
  const el = document.elementFromPoint(e.clientX, e.clientY)
  const th = el && el.closest ? el.closest('th') : null
  if (st.overEl && st.overEl !== th) { st.overEl.classList.remove('col-drag-over'); st.overEl = null }
  const gk = th && th.getAttribute ? th.getAttribute('data-gkey') : null
  if (th && gk && gk.startsWith('0:')) {
    const target = visibleColumns.value[Number(gk.slice(2))]
    if (target && target !== st.col) { th.classList.add('col-drag-over'); st.overEl = th }
  }
  e.preventDefault()
}
const onColDragEndDoc = () => {
  const st = colDragState
  if (!st) return
  if (st.overEl) {
    const gk = st.overEl.getAttribute('data-gkey') || ''
    const target = visibleColumns.value[Number(gk.slice(2))]
    if (st.moved && target && target !== st.col) reorderColumns(st.col, target)
    st.overEl.classList.remove('col-drag-over')
  }
  if (st.sourceEl) st.sourceEl.classList.remove('col-dragging')
  if (st.moved) { lastDragAt = Date.now(); document.body.style.cursor = '' }
  colDragState = null
  document.removeEventListener('mousemove', onColDragMoveDoc)
  document.removeEventListener('mouseup', onColDragEndDoc)
}
const reorderColumns = (source, target) => {
  const vis = [...visibleColumns.value]
  const from = vis.indexOf(source)
  const to = vis.indexOf(target)
  if (from < 0 || to < 0) return
  vis.splice(from, 1)
  vis.splice(to, 0, source)
  // 保持隐藏列在原位，仅重排可见列
  const it = vis[Symbol.iterator]()
  columns.value = columns.value.map(c => hiddenColumns.value.has(c) ? c : it.next().value)
  // 同步元数据顺序，避免类型/主键信息错位
  if (columnMetas.value.length) {
    const map = new Map(columnMetas.value.map(m => [m.name, m]))
    columnMetas.value = columns.value.filter(c => map.has(c)).map(c => map.get(c))
  }
}

// ========== 列宽双击自动适应（按真实文本内容测量） ==========
// 它算出来的就是「默认宽度」——和自动测量共用 naturalColWidth，两条路不可能再算出两个值。
// （历史差异：这里以前采样 50 行、上限写 800，而自动测量是 30 行 / MAX_COL_WIDTH。）
const autoFitCol = (col) => {
  const wrap = gridWrap.value
  if (!wrap) return
  const tableEl = wrap.querySelector('table')
  const fontFamily = tableEl ? getComputedStyle(tableEl).fontFamily : 'sans-serif'
  const w = naturalColWidth(col, fontFamily, displayRows.value.slice(0, COL_SAMPLE_ROWS))
  colWidths.value = { ...colWidths.value, [col]: w }
  manualCols.value = new Set(manualCols.value).add(col)
  saveColWidths()   // 双击自适应也算用户设定，一起持久化
}
// 选中多列时一起自适应
const autoFitSelectedCols = () => {
  for (const c of visibleColumns.value) {
    if (selectedCols.value.has(c)) autoFitCol(c)
  }
}
// 双击表头右缘自适应列宽（与右键「列宽自适应」、结果表行为一致）
const onHeaderDblClick = (col, e) => {
  const rect = e.currentTarget.getBoundingClientRect()
  if (e.clientX < rect.right - 12) return // 只有双击右缘才自适应，避免误触排序/选中
  autoFitCol(col)
}

// ========== 右键上下文菜单 ==========
const ctxMenu = ref({ visible: false, x: 0, y: 0, items: [], row: null, col: null })
const ctxSub = ref(null)
let ctxFrom = 'cell' // 右键来源，决定菜单给「列 / 行 / 区域 / 单元格」哪一套
const closeCtxMenu = () => { ctxMenu.value = { ...ctxMenu.value, visible: false }; ctxSub.value = null }
// 右键来源：header=表头、row=首列复选框那格、cell=数据区、grid=表格空白处
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
// ========== 行详情：双击 / 右键行号查看整行字段明细（复用单元格详情弹窗） ==========
const rowDetail = ref({ visible: false, title: '', text: '' })
const openRowDetail = (row) => {
  if (!row) return
  const cols = visibleColumns.value
  const lines = cols.map(c => {
    const v = row[c]
    // 与网格同口径：ISO 时间串的 `T` 换成空格
    return c + '：' + (v == null ? 'NULL' : String(formatDbValue(v)))
  })
  rowDetail.value = { visible: true, title: t('sqlq.rowDetailTitle', { n: rowIndex(row) + 1 }), text: lines.join('\n') }
}
const onCellContextMenu = (e, row, col) => {
  e.preventDefault()
  openCtxMenu(e.clientX, e.clientY, row, col, 'cell')
}
const openCtxMenu = (x, y, row, col, from = 'cell') => {
  // 先写入目标，ctxSelection 才能据此判断右键位置是否落在当前选区
  ctxMenu.value = { visible: false, x, y, items: [], row, col }
  ctxFrom = from
  // 当前是否存在「真正的选区」：框选区域（单格不算）/ 选中行 / 选中列
  const fr = gridSel?.range?.value || null
  const frameOn = !!fr && (fr.r1 !== fr.r2 || fr.c1 !== fr.c2)
  const hasSelection = frameOn || selectedSet.value.size > 0 || selectedCols.value.size > 0 || headerSelected.value
  // 表头 / 行头右键**只在完全没有选中时**才把选中切过去。已有选区就保持不动，
  // 这样「选中若干行后在数据区右键」与「在行头右键」拿到的菜单完全一致（菜单由选中决定）
  if (from === 'header' && col && !hasSelection) {
    focusCols() // 列选择是唯一选区：清掉行选择与框选
    selectSingleCol(col)
  }
  if (from === 'row' && row && !hasSelection) {
    // 锚点统一用 displayRows 下标（原来这里是绝对行号，和 onRowClick 的坐标系不一致）
    focusRows() // 行选择是唯一选区：清掉列选择与框选
    selectedSet.value = new Set([row._rid])
    lastAnchorIdx.value = dispIdxOf(row)
  }
  const sel = ctxSelection()
  const hasTarget = !!col || sel.rows.length > 0
  if (!hasTarget) return
  const canEdit = !props.readOnly
  const items = []
  // 只在确有内容时插分隔线，避免菜单顶部出现孤立分隔线
  const sep = () => { if (items.length && !items[items.length - 1].sep) items.push({ sep: true }) }
  // 列类型决定可用的筛选运算符（数值/日期列才有 大于/小于）
  const colMeta = col ? columnMetas.value.find(m => m.name === col) : null
  const colType = colMeta ? classifyType(colMeta.type) : 'string'
  const filterOps = [
    { label: t('tdv.filterEq'), command: 'filter-eq' },
    { label: t('tdv.filterNe'), command: 'filter-ne' }
  ]
  if (colType === 'string') filterOps.push({ label: t('tdv.filterContains'), command: 'filter-contains' })
  if (colType === 'number' || colType === 'date') {
    filterOps.push({ label: t('tdv.filterGt'), command: 'filter-gt' })
    filterOps.push({ label: t('tdv.filterLt'), command: 'filter-lt' })
  }
  filterOps.push({ label: t('cop.isnull'), command: 'filter-is-null' })
  filterOps.push({ label: t('cop.isnotnull'), command: 'filter-is-not-null' })

  // 复制为 ▸：各上下文共用的格式项
  const formatSub = [
    { label: 'CSV', command: 'copy-csv' },
    { label: 'JSON', command: 'copy-json' },
    { label: 'MARKDOWN', command: 'copy-markdown' },
    { label: 'INSERT', command: 'copy-insert' },
    { label: 'UPDATE', command: 'copy-update' },
    { label: 'DELETE', command: 'copy-delete' }
  ]
  const selColCount = selectedCols.value.size
  const selRowCount = selectedSet.value.size
  // 菜单内容由**当前选中**决定，与右键落在哪无关（选中行时，数据区与行头给的菜单必须一模一样）：
  //   框选区域 → 区域功能；选中行 → 行功能；选中列 → 列功能；
  //   完全没有选中时才看「点在哪里」：表头→列、行头→行、数据区→单元格。
  //   只选中一个单元格不算"有选区"，那时给的就是单元格功能。
  const mode = frameOn ? 'range'
    : (selRowCount || headerSelected.value) ? 'rows'
      : selColCount ? 'columns'
        : from === 'header' ? 'columns'
          : from === 'row' ? 'rows'
            : from === 'grid' ? 'range'
              : 'cell'
  // 表格空白处右键但没有任何框选 → 没东西可操作
  if (from === 'grid' && !frameOn) return

  if (mode === 'range') {
    // —— 区域操作 ——
    // 「复制」= 与 Ctrl+C 完全一致（照选中范围原样复制，表头只在选中范围内才带）
    items.push({ label: t('mdk.copy'), command: 'copy-sel', shortcut: 'Ctrl+C' })
    items.push({ label: t('sqlq.ctxCopyAs'), sub: formatSub })
  } else if (mode === 'columns') {
    // —— 列操作 ——
    const n = selColCount
    items.push({ label: t('mdk.copy'), command: 'copy-sel', shortcut: 'Ctrl+C' })
    items.push({ label: t('sqlq.ctxCopyAs'), sub: formatSub })
    // 只要列名（不含数据）：贴进 SELECT / WHERE 用；选了几列就复制几列的名字
    items.push({ label: t('sqlq.ctxCopyHeader'), command: 'copy-col-header' })
    sep()
    if (n === 1) {
      items.push({ label: t('tdv.filterCol'), sub: filterOps })
      items.push({ label: t('tdv.openInQuery'), command: 'open-query' })
    }
    items.push({ label: t('sqlq.ctxColFit'), command: 'col-fit' })
    items.push({ label: t('sqlq.ctxHideCol'), command: 'hide-col' })
    if (hiddenColumns.value.size) items.push({ label: t('sqlq.ctxShowAllCols'), command: 'show-all-cols' })
    if (hiddenRows.value.size) items.push({ label: t('tdv.showAllRows'), command: 'show-all-rows' })
  } else if (mode === 'rows') {
    // —— 行操作 ——
    const n = selRowCount
    if (row) items.push({ label: t('sqlq.ctxRowDetail'), command: 'row-detail' })
    items.push({ label: t('mdk.copy'), command: 'copy-sel', shortcut: 'Ctrl+C' })
    items.push({ label: t('sqlq.ctxCopyAs'), sub: formatSub })
    if (canEdit) {
      sep()
      if (row) items.push({ label: t('tdv.insertRow'), sub: [
        { label: t('tdv.insertAbove'), command: 'insert-above' },
        { label: t('tdv.insertBelow'), command: 'insert-below' }
      ] })
      // 「设为 NULL」只属于单元格操作，放在单元格菜单里，不要在行菜单出现
      items.push({ label: t('tdv.deleteRowsN', { n: n }), command: 'delete-selected' })
    }
    sep()
    items.push({ label: t('tdv.hideRows'), command: 'hide-rows' })
    if (hiddenRows.value.size) items.push({ label: t('tdv.showAllRows'), command: 'show-all-rows' })
  } else {
    // —— 单元格操作（只放作用在这一格上的操作；行/列操作先选中行/列再右键） ——
    if (col) items.push({ label: t('mdk.copy'), command: 'copy-cell', shortcut: 'Ctrl+C' })
    if (row && col && canEdit) {
      sep()
      items.push({ label: t('tdv.setNull'), command: 'set-null' })
    }
  }
  if (!items.length) return
  // 只有一个子项的二级菜单没必要，直接提升为一级
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
    case 'copy-sel': copyLikeCtrlC(); break
    case 'copy-col-header': copyColHeader(); break
    case 'copy-csv': copyCtx('csv'); break
    case 'copy-insert': copyCtx('insert'); break
    case 'copy-update': copyCtx('update'); break
    case 'copy-json': copyCtx('json'); break
    case 'copy-delete': copyDelete(); break
    case 'copy-markdown': copyMarkdown(); break
    case 'insert-above': if (row) insertRowAt(rowIndex(row)); break
    case 'insert-below': if (row) insertRowAt(rowIndex(row) + 1); break
    case 'filter-eq': filterByValue(col, 'eq'); break
    case 'filter-ne': filterByValue(col, 'ne'); break
    case 'filter-contains': filterByValue(col, 'contains'); break
    case 'filter-gt': filterByValue(col, 'gt'); break
    case 'filter-lt': filterByValue(col, 'lt'); break
    case 'filter-is-null': filterByValue(col, 'is_null'); break
    case 'filter-is-not-null': filterByValue(col, 'is_not_null'); break
    case 'open-query': openQueryWithValue(col); break
    case 'col-fit':
      if (selectedCols.value.size > 1) autoFitSelectedCols()
      else autoFitCol(col)
      break
    case 'hide-col': hideColumnSmart(col); break
    case 'hide-rows': hideRowsSmart(row); break
    case 'show-all-rows': showAllRows(); break
    case 'show-all-cols': showAllColumns(); break
    case 'set-null': setCellNull(row, col); break
    case 'delete-selected': removeSelectedRows(); return
  }
  closeCtxMenu()
}

const writeClipboard = (text, msg) => {
  if (!text) return
  const done = () => { if (msg) ElMessage.success(msg) }
  // 降级：非安全上下文（http 且非 localhost）里 navigator.clipboard 不可用，
  // 用临时 textarea + execCommand 兜底，保证「复制」在哪都能用
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

// 右键目标的「行集合 / 列集合」，优先级：
// 1) 右键落在框选区域内 → 用框选
// 2) 有勾选行 / 有选中列 → 勾选行 × 选中列（缺哪个用全部），Ctrl+A 全选行即「复制全部」
// 3) 都不满足 → 用右键那一格
const ctxSelection = () => {
  const { row, col } = ctxMenu.value
  const range = gridSel?.range?.value || null
  // 只有拖出「多格」的框选才算区域；单击产生的单格不算，避免盖掉显式的选中列
  const rangeHasArea = !!range && (range.r1 !== range.r2 || range.c1 !== range.c2)
  // 数据区 / 表格空白处右键：只要存在框选区域就按框选取
  // （右键落在框选外也不降级成单元格 —— 有选中就按选中给功能）。
  // 选区坐标与 data-gkey 一致：绝对行下标 + 1（表头行为 0）
  if (rangeHasArea && (ctxFrom === 'cell' || ctxFrom === 'grid')) {
    const picked = []
    for (let r = Math.max(range.r1, 1); r <= range.r2; r++) {
      const rr = rows.value[r - 1]
      if (rr && rr._editState !== 'deleted' && !hiddenRows.value.has(rr._rid)) picked.push(rr)
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


const sqlVal = (v) => {
  if (v === null || v === undefined) return 'NULL'
  if (typeof v === 'number') return String(v)
  if (typeof v === 'boolean') return v ? '1' : '0'
  return `'${String(v).replace(/'/g, "''")}'`
}

const copyCtx = (mode) => {
  const { rows, cols } = ctxSelection()
  if (!rows.length || !cols.length) { ElMessage.warning(t('sqlq.nothingToCopy')); return }
  const table = props.table || 'table'
  const colList = cols.join(', ')
  if (mode === 'csv') {
    const esc = (s) => /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
    const lines = [cols.map(esc).join(',')]
    for (const r of rows) lines.push(cols.map(c => esc(r[c] == null ? '' : String(r[c]))).join(','))
    writeClipboard(lines.join('\n'), t('sqlq.copyCsv', { n: rows.length }))
    return
  }
  if (mode === 'insert') {
    const sql = rows.map(r => `INSERT INTO ${table} (${colList}) VALUES (${cols.map(c => sqlVal(r[c])).join(', ')});`).join('\n')
    writeClipboard(sql, t('tdv.copyInsert', { n: rows.length }))
    return
  }
  if (mode === 'update') {
    const pk = columnMetas.value.filter(m => m.primaryKey).map(m => m.name)
    // 无主键时用整行所有列做 WHERE，避免定位不唯一
    const whereCols = pk.length ? pk : [...columns.value]
    const sql = rows.map(r => {
      const setClause = cols.map(c => `${c} = ${sqlVal(r[c])}`).join(', ')
      const whereClause = whereCols.map(c => `${c} = ${sqlVal(r[c])}`).join(' AND ')
      return `UPDATE ${table} SET ${setClause} WHERE ${whereClause};`
    }).join('\n')
    writeClipboard(sql, `已复制 ${rows.length} 条 UPDATE${pk.length ? '' : '（无主键，按整行匹配）'}`)
    return
  }
  if (mode === 'json') {
    const obj = rows.map(r => {
      const o = {}
      for (const c of cols) o[c] = r[c] === undefined ? null : r[c]
      return o
    })
    writeClipboard(JSON.stringify(obj.length === 1 ? obj[0] : obj, null, 2), t('sqlq.copyJson', { n: rows.length }))
  }
}

const copyCell = (row, col) => {
  if (!row || !col) return
  writeClipboard(row[col] == null ? '' : String(row[col]), '已复制单元格')
}
// 当前选区的 TSV 文本（Excel 可直接粘贴）：
// 列选中 = 列名 + 该列所有可见行；行选中 = 所选行的全部可见列（选中标题行时才带列名）；否则 = 当前单元格
const selectionAsTsv = () => {
  const cols = visibleColumns.value
  if (selectedCols.value.size) {
    const picked = cols.filter(c => selectedCols.value.has(c))
    if (!picked.length) return ''
    const lines = [picked.join('\t')]
    for (const r of displayRows.value) lines.push(picked.map(c => r[c] == null ? '' : String(r[c])).join('\t'))
    return lines.join('\n')
  }
  if (selectedSet.value.size || headerSelected.value) {
    const picked = displayRows.value.filter(r => selectedSet.value.has(r._rid))
    const lines = []
    // 只有标题行被选中（点/拖左上角那格）才输出列名；单纯选数据行不带
    if (headerSelected.value) lines.push(cols.join('\t'))
    for (const r of picked) lines.push(cols.map(c => r[c] == null ? '' : String(r[c])).join('\t'))
    return lines.join('\n')
  }
  if (activeCell.value) {
    const { row, col } = activeCell.value
    const v = row && col ? row[col] : null
    return v == null ? '' : String(v)
  }
  return ''
}
// 右键「复制」：与 Ctrl+C 完全同一条路 —— 有框选就复制框选区域（按数据全量取值），
// 否则复制当前行/列选中的区域（表头只在选中范围内才带）
const copyLikeCtrlC = () => {
  if (gridSel && gridSel.hasSelection()) {
    const t = gridSel.copy()
    if (t) writeClipboard(t, `已复制选区（${t.split('\n').length} 行）`)
    return
  }
  const text = selectionAsTsv()
  if (!text) { ElMessage.warning(t('sqlq.nothingToCopy')); return }
  writeClipboard(text, `已复制 ${text.split('\n').length} 行`)
}
// Ctrl+C 的场景化提示：让人明确这次到底复制了什么
const copyHint = () => {
  if (selectedCols.value.size) return `已复制 ${selectedCols.value.size} 列（含列名）`
  if (selectedSet.value.size) return `已复制 ${selectedSet.value.size} 行${headerSelected.value ? '（含列名）' : ''}`
  if (headerSelected.value) return t('sqlq.copyColNames')
  return t('sqlq.copyCell')
}
// 框选区域复制完成（Ctrl+C 由框选模块自己的 copy 事件处理，这里只负责提示）
const onGridCopied = ({ rows, cols, header }) => {
  ElMessage.success(`已复制选区（${rows} 行 × ${cols} 列${header ? '，含列名' : ''}）`)
}
// Ctrl+C 的主路径：浏览器派发的 copy 事件。
// 不要求焦点在表格里 —— Ctrl+A 全选后焦点常在 body，只挂在 keydown 上会「按了没反应」；
// 焦点在输入框里时不动（用户复制的是自己的文字）。
let copyEventAt = 0
const onDocCopy = (e) => {
  // 焦点在单元格编辑框等输入控件里：复制的是用户自己的文字，连框选模块也一起挡掉
  if (inEditableFocus()) { e.stopImmediatePropagation(); return }
  if (gridSel && gridSel.hasSelection()) return // 框选交给框选模块（按数据全量取值）
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
// 右键目标单元格的值（优先右键那一行；勾选行/选列会改变 ctxSelection，不能拿它当取值来源）
const ctxCellValue = (col) => {
  const row = ctxMenu.value.row || ctxSelection().rows[0]
  return row ? row[col] : undefined
}
// 复制表头（列名）：有选中的列就复制选中的，否则复制右键那一列。
// 多列用制表符分隔 —— 与 Excel 一致，粘到 Excel 里是横向多个单元格（不是一整串文本）
const copyColHeader = () => {
  const picked = visibleColumns.value.filter(c => selectedCols.value.has(c))
  const names = picked.length ? picked : (ctxMenu.value.col ? [ctxMenu.value.col] : [])
  if (!names.length) return
  writeClipboard(names.join('\t'), names.length > 1 ? t('sqlq.copyColNamesN', { n: names.length }) : t('sqlq.copyColNameOne', { name: names[0] }))
}

const copyMarkdown = () => {
  const { rows, cols } = ctxSelection()
  if (!rows.length || !cols.length) return
  const esc = (s) => String(s).replace(/\|/g, '\\|').replace(/\r?\n/g, ' ')
  const head = `| ${cols.map(esc).join(' | ')} |`
  const sep = `| ${cols.map(() => '---').join(' | ')} |`
  const body = rows.map(r => `| ${cols.map(c => esc(r[c] == null ? '' : r[c])).join(' | ')} |`)
  writeClipboard([head, sep, ...body].join('\n'), t('sqlq.copyMarkdown', { n: rows.length }))
}
const copyDelete = () => {
  const { rows } = ctxSelection()
  if (!rows.length) return
  const table = props.table || 'table'
  const pk = columnMetas.value.filter(m => m.primaryKey).map(m => m.name)
  const whereCols = pk.length ? pk : [...columns.value]
  const sql = rows.map(r => `DELETE FROM ${table} WHERE ${whereCols.map(c => `${c} = ${sqlVal(r[c])}`).join(' AND ')};`).join('\n')
  writeClipboard(sql, `已复制 ${rows.length} 条 DELETE${pk.length ? '' : '（无主键，按整行匹配）'}`)
}
const hideColumn = (col) => {
  if (!col) return
  if (visibleColumns.value.length <= 1) { ElMessage.warning(t('sqlq.keepOneCol')); return }
  hiddenColumns.value = new Set(hiddenColumns.value).add(col)
}
// 「隐藏列」：有选中的列就隐藏全部选中列（单选/多选都支持）；没选任何列时隐藏右键那一列
const hideColumnSmart = (col) => {
  const picked = visibleColumns.value.filter(c => selectedCols.value.has(c))
  if (!picked.length) { hideColumn(col); return }
  if (visibleColumns.value.length - picked.length < 1) { ElMessage.warning(t('sqlq.keepOneCol')); return }
  hiddenColumns.value = new Set([...hiddenColumns.value, ...picked])
  clearColSelect()
}
// 「隐藏行」：有勾选行就隐藏全部勾选行；没勾选时隐藏右键那一行（只影响显示，不影响保存）
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
  // 被隐藏的行不能再是活动单元格/编辑目标
  activeCell.value = null
  editingCell.value = null
  gridSel?.clear()
}
const showAllRows = () => { hiddenRows.value = new Set() }
const insertRowAt = (idx) => {
  if (props.readOnly) return
  const newRow = { _editState: 'new', _original: {}, _rid: nextRid(), _idx: idx }
  for (const col of columns.value) newRow[col] = null
  rows.value.splice(Math.max(0, Math.min(rows.value.length, idx)), 0, newRow)
  rebuildRowIndex()
  selectedSet.value = new Set([newRow._rid])
  activeCell.value = { row: newRow, col: visibleColumns.value[0] }
}
// 按当前单元格的值追加一条高级筛选并立即查询
// is_null / is_not_null 不需要取值，直接按列追加条件
const filterByValue = (col, op) => {
  if (!col) return
  const needsValue = op !== 'is_null' && op !== 'is_not_null'
  const v = needsValue ? ctxCellValue(col) : null
  if (needsValue && (v === undefined || v === null)) { ElMessage.warning(t('tdv.cellEmpty')); return }
  const meta = columnMetas.value.find(m => m.name === col)
  filters.value.push({
    join: 'AND', col, op, value: needsValue ? String(v) : '', value2: '',
    colType: meta ? classifyType(meta.type) : 'string'
  })
  advancedOpen.value = true
  load(1)
}
// 以「该列 = 该值」为条件在查询窗口打开 SELECT
const openQueryWithValue = (col) => {
  if (!col) return
  const v = ctxCellValue(col)
  const tbl = quoteIdent(props.table)
  const c = quoteIdent(col)
  const where = (v === undefined || v === null) ? `${c} IS NULL` : `${c} = '${String(v).replace(/'/g, "''")}'`
  emit('open-query', { connId: props.conn?.id || '', database: props.database || '', sql: `SELECT * FROM ${tbl} WHERE ${where}` })
}
const setCellNull = (row, col) => {
  if (!row || !col || props.readOnly) return
  row[col] = null
  if (row._editState === 'original') row._editState = 'modified'
}
const removeRow = (row) => {
  const idx = rows.value.indexOf(row)
  if (idx < 0) return
  if (row._editState === 'new') {
    rows.value.splice(idx, 1)
    rebuildRowIndex()
  } else {
    row._editState = 'deleted'
    deletedRids.value.add(row._rid)
  }
  selectedSet.value = new Set([...selectedSet.value].filter(id => id !== row._rid))
  gridSel?.clear()
}
const removeSelectedRows = () => {
  if (!selectedSet.value.size) return
  const ids = selectedSet.value
  const toDelete = rows.value.filter(r => ids.has(r._rid) && r._editState !== 'deleted')
  for (const row of toDelete) {
    if (row._editState === 'new') {
      rows.value.splice(rows.value.indexOf(row), 1)
    } else {
      row._editState = 'deleted'
      deletedRids.value.add(row._rid)
    }
  }
  rebuildRowIndex()
  selectedSet.value = new Set()
  gridSel?.clear()
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

const startEdit = (row, col, event) => {
  if (row._editState === 'deleted' || props.readOnly) return
  editingCell.value = { row, col, value: row[col] === null || row[col] === undefined ? '' : String(row[col]) }
  nextTick(() => {
    const input = (event && event.target && event.target.querySelector) ? event.target.querySelector('.cell-input') : null
    if (input || cellInputRef.value) (input || cellInputRef.value).focus()
    if (input || cellInputRef.value) (input || cellInputRef.value).select()
  })
}

// 单元格赋值归一化：按列类型还原数值/布尔。编辑框与「从 Excel 粘贴」进来的都是字符串，
// 直接以字符串提交会在严格类型库（ClickHouse/PG/Oracle 等）报类型不匹配
const normalizeCellValue = (col, raw) => {
  const trimmed = String(raw == null ? '' : raw).trim()
  let v = trimmed === '' ? null : trimmed
  if (v !== null) {
    const meta = columnMetas.value.find(c => c.name === col)
    const t = meta ? String(meta.type || '') : ''
    if (/^(U?INT\d*|FLOAT\d*|DOUBLE|REAL|DECIMAL|NUMERIC|NUMBER|BIGINT|SMALLINT|TINYINT|MEDIUMINT|INTEGER)/i.test(t)
        && !isNaN(Number(v))) {
      v = Number(v)
    } else if (/^(BOOL|BOOLEAN|LOGICAL)/i.test(t) && /^(true|false)$/i.test(v)) {
      v = /^true$/i.test(v)
    }
  }
  return v
}
const applyCellValue = (row, col, raw) => {
  if (!row || !col || props.readOnly || row._editState === 'deleted') return false
  const v = normalizeCellValue(col, raw)
  if (v === row[col]) return false
  row[col] = v
  if (row._editState === 'original') row._editState = 'modified'
  return true
}
const confirmEdit = () => {
  if (!editingCell.value) return
  const { row, col, value } = editingCell.value
  applyCellValue(row, col, value)
  editingCell.value = null
}
// 编辑框里 Enter 提交并下移（Shift+Enter 上移）、Tab 提交并右移（Excel 行为）
const onCellEnter = (e) => {
  const down = !e.shiftKey
  confirmEdit()
  moveActive(down ? 1 : -1, 0)
}
const onCellTab = () => {
  confirmEdit()
  tabMove(1)
}

const cancelEdit = () => {
  editingCell.value = null
}

// ========== 活动单元格（键盘导航锚点） ==========
const activeCell = ref(null) // { row, col }
const onCellClick = (row, col, e) => {
  if (editingCell.value) return
  // 左键点单元格 = 离开「选中列」上下文，回到单元格上下文（否则右键菜单会一直是列操作）
  if (selectedCols.value.size) clearColSelect()
  activeCell.value = { row, col }
  // 让容器获取焦点，键盘导航才能生效（不触发滚动）
  if (gridWrap.value && gridWrap.value.focus) gridWrap.value.focus({ preventScroll: true })
}
// 单元格 gkey 坐标 ↔ 数据。行号与模板保持一致（rowIndex(row) + 1，表头行为 0），
// 因此这里必须用绝对下标 _idx 取行，不能用 displayRows 下标（隐藏/删除行会让两者错位）
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
// 可写入的行（跳过已删除 / 已隐藏）
const isWritableRow = (row) => !!row && row._editState !== 'deleted' && !hiddenRows.value.has(row._rid)
// 按 gkey 取值：r0 === 0 为表头行（列名）。走数据而非 DOM，跨屏大选区也能取全
const cellTextAt = (r0, c0) => {
  if (r0 === 0) return visibleColumns.value[c0] || ''
  const hit = cellByGkey(r0, c0)
  if (!hit) return ''
  return hit.row[hit.col] == null ? '' : String(hit.row[hit.col])
}
const dispIdxOf = (row) => displayRows.value.indexOf(row)
// 把活动单元格落到 (rIdx, cIdx)（displayRows 下标）；extend=true 时保持锚点扩展选区
const applyActive = (rIdx, cIdx, extend) => {
  const rowsList = displayRows.value
  const cols = visibleColumns.value
  if (!rowsList.length || !cols.length) return
  const row = rowsList[Math.max(0, Math.min(rowsList.length - 1, rIdx))]
  const col = cols[Math.max(0, Math.min(cols.length - 1, cIdx))]
  if (!row || !col) return
  const from = activeCell.value
  const anchorG = extend ? (gridSel.anchor() || (from ? gkeyOfCell(from.row, from.col) : null)) : null
  if (extend) focusCells() // Shift+方向键扩出的区域也是「唯一那块选区」，清掉行/列选择
  activeCell.value = { row, col }
  const g = gkeyOfCell(row, col)
  if (anchorG && g) gridSel.setRange(anchorG.r, anchorG.c, g.r, g.c)
  else gridSel.clearRange()
  ensureActiveVisible()
}
// 移动活动单元格：edge = 跳到数据边缘（Ctrl+方向键），extend = 扩展选区（Shift+方向键）
const moveActive = (dr, dc, { edge = false, extend = false } = {}) => {
  const from = activeCell.value
  if (!from) return
  const rowsList = displayRows.value
  const cols = visibleColumns.value
  const r0 = dispIdxOf(from.row)
  const c0 = cols.indexOf(from.col)
  if (r0 < 0 || c0 < 0) return
  let r = r0 + dr
  let c = c0 + dc
  if (edge) {
    if (dr) r = dr > 0 ? rowsList.length - 1 : 0
    if (dc) c = dc > 0 ? cols.length - 1 : 0
  }
  applyActive(r, c, extend)
}
// Tab / Shift+Tab：左右移动，行末自动换到下一行（Excel 行为）
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
// PageUp / PageDown 的步长：按可视高度折算行数
const pageRowStep = () => {
  const host = scrollHost()
  const n = host ? Math.floor(host.clientHeight / (VP_ROW_H || 32)) : 10
  return Math.max(1, n - 1)
}
// 直接输入字符即进入编辑（Excel：新输入覆盖原值）
const startEditWith = (row, col, ch) => {
  startEdit(row, col, { target: gridWrap.value })
  if (editingCell.value) editingCell.value = { ...editingCell.value, value: ch }
}

// ========== 单元格批量操作（类 Excel）==========
// 「清空选中单元格」：框选优先，否则当前格；统一置 NULL
const clearSelectedCells = () => {
  if (props.readOnly) return
  const rect = gridSel.rect()
  if (rect) {
    let n = 0
    for (let r0 = Math.max(1, rect.r1); r0 <= rect.r2; r0++) {
      for (let c0 = rect.c1; c0 <= rect.c2; c0++) {
        const hit = cellByGkey(r0, c0)
        if (hit && isWritableRow(hit.row) && hit.row[hit.col] != null) { setCellNull(hit.row, hit.col); n++ }
      }
    }
    if (n) ElMessage.success(t('tdv.clearedCells', { n: n }))
    return
  }
  if (activeCell.value && isWritableRow(activeCell.value.row)) setCellNull(activeCell.value.row, activeCell.value.col)
}
// 「向下填充」（Ctrl+D）：把选区首行填到选区其余行
const fillDown = () => {
  if (props.readOnly) return
  const rect = gridSel.rect()
  const r1 = rect ? Math.max(1, rect.r1) : 0
  if (!rect || rect.r2 <= r1) { ElMessage.warning(t('tdv.fillNeedTwoRows')); return }
  let n = 0
  for (let c0 = rect.c1; c0 <= rect.c2; c0++) {
    const first = cellByGkey(r1, c0)
    if (!first || !isWritableRow(first.row)) continue
    const src = first.row[first.col]
    for (let r0 = r1 + 1; r0 <= rect.r2; r0++) {
      const hit = cellByGkey(r0, c0)
      if (hit && isWritableRow(hit.row) && applyCellValue(hit.row, hit.col, src == null ? '' : src)) n++
    }
  }
  ElMessage.success(n ? t('tdv.filledCells', { n: n }) : t('tdv.noFillNeeded'))
}
// 「剪切」（Ctrl+X）：复制选区 + 清空
const cutSelection = () => {
  if (props.readOnly) return
  if (!gridSel.hasSelection()) { ElMessage.warning(t('tdv.pickCutFirst')); return }
  const text = gridSel.copy()
  if (!text) return
  writeClipboard(text, t('tdv.cutSelection'))
  clearSelectedCells()
}
// 「粘贴」（Ctrl+V）：把剪贴板里的表格块从活动单元格开始写入（TSV 即 Excel 复制格式）
const pasteBlock = (text) => {
  if (props.readOnly || !activeCell.value) return
  const block = String(text).replace(/\r\n?/g, '\n').replace(/\n+$/, '').split('\n').map(l => l.split('\t'))
  const rowsList = rows.value
  const cols = visibleColumns.value
  const r0 = rowsList.indexOf(activeCell.value.row)
  const c0 = cols.indexOf(activeCell.value.col)
  if (r0 < 0 || c0 < 0 || !block.length) return
  let n = 0
  for (let ri = 0; ri < block.length; ri++) {
    const row = rowsList[r0 + ri]
    if (!row) break
    if (!isWritableRow(row)) continue
    for (let ci = 0; ci < block[ri].length; ci++) {
      const col = cols[c0 + ci]
      if (!col) break
      if (applyCellValue(row, col, block[ri][ci])) n++
    }
  }
  ElMessage.success(n ? t('tdv.pasted', { rows: block.length, cols: block[0].length }) : t('tdv.pastedSame'))
  ensureActiveVisible()
}
// 粘贴事件：焦点在表格内且不在编辑态时接管（Ctrl+V 不拦截，让浏览器把数据交给 paste 事件）
const onPaste = (e) => {
  if (props.readOnly || editingCell.value || !activeCell.value) return
  const wrap = gridWrap.value
  const ae = document.activeElement
  if (!wrap || !ae || (ae !== wrap && !wrap.contains(ae))) return
  const text = e.clipboardData ? e.clipboardData.getData('text/plain') : ''
  if (!text) return
  e.preventDefault()
  pasteBlock(text)
}
document.addEventListener('paste', onPaste)
onBeforeUnmount(() => document.removeEventListener('paste', onPaste))

// 表格内键盘导航（类 Excel）：方向键 / Shift 扩展选区 / Ctrl 跳边缘 / Tab / Home / End / 翻页
const onKeyDown = (e) => {
  if (!rootRef.value) return
  const active = document.activeElement
  // 仅当焦点在表格区域内（不在输入框/下拉等）才接管
  if (active && active.closest && active.closest('input, textarea, .el-select, .el-dropdown')) return
  // Ctrl/Cmd+A：全选当前页所有行
  if ((e.ctrlKey || e.metaKey) && (e.key === 'a' || e.key === 'A')) {
    const inRoot = rootRef.value.contains(active)
    const onBodyVisible = active === document.body && rootRef.value.getClientRects().length > 0
    if (!inRoot && !onBodyVisible) return
    e.preventDefault()
    focusRows()
    headerSelected.value = false // 全选的是数据行，标题行不跟着亮
    if (displayRows.value.length) selectedSet.value = new Set(displayRows.value.map(r => r._rid))
    return
  }
  if (!rootRef.value.contains(active)) return
  // Esc：清空当前这块选区（行 / 列 / 框选），编辑中的 Esc 由下面的分支处理
  if (e.key === 'Escape' && !editingCell.value &&
      (selectedSet.value.size || headerSelected.value || selectedCols.value.size || gridSel.hasSelection())) {
    e.preventDefault()
    clearRowSelection()
    clearColSelection()
    clearGridRange()
    return
  }
  const mod = e.ctrlKey || e.metaKey
  // Ctrl+C：复制当前这块选区。主路径是浏览器派发的 copy 事件（见 onDocCopy，任何焦点下都能用）；
  // 这里只兜底 —— 个别环境不派发 copy 事件时，用 navigator.clipboard 补写一次。
  // 注意不能 preventDefault，否则会把 copy 事件本身掐掉
  if (mod && !e.shiftKey && (e.key === 'c' || e.key === 'C')) {
    const startedAt = Date.now()
    setTimeout(() => {
      if (copyEventAt >= startedAt) return          // copy 事件已经处理过
      if (gridSel && gridSel.hasSelection()) return // 框选模块自己会处理
      if (inEditableFocus()) return
      const text = selectionAsTsv()
      if (text) writeClipboard(text, copyHint())
    }, 0)
    return
  }
  // Ctrl+V 不拦截：交给浏览器触发 paste 事件，这样才能拿到剪贴板里的表格数据
  if (mod && !e.shiftKey && (e.key === 'v' || e.key === 'V')) return
  if (!activeCell.value) return
  const { row, col } = activeCell.value
  // 方向键：Shift 扩展选区，Ctrl 跳到数据边缘，两者可叠加
  if (e.key.startsWith('Arrow')) {
    e.preventDefault()
    const dr = e.key === 'ArrowUp' ? -1 : e.key === 'ArrowDown' ? 1 : 0
    const dc = e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowRight' ? 1 : 0
    moveActive(dr, dc, { edge: mod, extend: e.shiftKey })
    return
  }
  // Home / End / PageUp / PageDown
  if (e.key === 'Home' || e.key === 'End' || e.key === 'PageUp' || e.key === 'PageDown') {
    e.preventDefault()
    const cols = visibleColumns.value
    if (e.key === 'Home') applyActive(mod ? 0 : dispIdxOf(row), 0, e.shiftKey)
    else if (e.key === 'End') applyActive(mod ? displayRows.value.length - 1 : dispIdxOf(row), cols.length - 1, e.shiftKey)
    else moveActive(e.key === 'PageDown' ? pageRowStep() : -pageRowStep(), 0, { extend: e.shiftKey })
    return
  }
  // Tab / Shift+Tab：左右移动，行末换行
  if (e.key === 'Tab' && !editingCell.value) {
    e.preventDefault()
    tabMove(e.shiftKey ? -1 : 1)
    return
  }
  switch (e.key) {
    case 'F2':
    case 'Enter':
      if (!editingCell.value) { e.preventDefault(); startEdit(row, col, { target: gridWrap.value }) }
      break
    case 'Delete':
    case 'Backspace':
      if (!props.readOnly && !editingCell.value) { e.preventDefault(); clearSelectedCells() }
      break
    case 'Escape':
      if (editingCell.value) { e.preventDefault(); cancelEdit() }
      else if (gridSel.hasSelection()) { e.preventDefault(); gridSel.clearRange() }
      break
    default:
      // 直接输入字符即进入编辑（Excel 行为），新输入覆盖原值。
      // 空格除外：留给页面/表格滚动，避免想滚动却进了编辑态
      if (!mod && !e.altKey && !e.isComposing && e.key !== ' ' && e.key.length === 1 && !props.readOnly && row._editState !== 'deleted') {
        e.preventDefault()
        startEditWith(row, col, e.key)
      }
  }
}
const ensureActiveVisible = () => {
  if (!activeCell.value || !gridWrap.value) return
  nextTick(() => {
    const host = scrollHost()
    const el = gridWrap.value.querySelector(`[data-gkey="${(rowIndex(activeCell.value.row) + 1) + ':' + visibleColumns.value.indexOf(activeCell.value.col)}"]`)
    if (el && host) el.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  })
}

const addRow = () => {
  const newRow = { _editState: 'new', _original: {}, _rid: nextRid(), _idx: rows.value.length }
  for (const col of columns.value) {
    newRow[col] = null
  }
  rows.value.push(newRow)
  selectedSet.value = new Set([newRow._rid])
  activeCell.value = { row: newRow, col: visibleColumns.value[0] }
  nextTick(() => {
    const n = displayRows.value.length
    // 窗口化渲染：先把可视区推到末尾，占位高度才会算出正确的滚动总高，否则滚不到底
    if (virtualEnabled.value) {
      vpEnd.value = n
      vpStart.value = Math.max(0, n - VP_BUFFER * 2)
    }
    nextTick(() => {
      const host = scrollHost()
      if (host) host.scrollTop = host.scrollHeight
      syncViewport()
    })
  })
}

// 撤销全部未保存的修改：新建的行去掉、改过的按行内 _original 还原、删掉的复活。
// 以前另存了一份整页 originalRows 全量拷贝（上万行时是第二份内存 + 一次全量深拷贝），
// 其实每行的 _original 就够用 —— 少一份拷贝，翻页/加载都轻一截
const revertChanges = () => {
  const kept = []
  for (const r of rows.value) {
    if (r._editState === 'new') continue
    if (r._editState === 'modified' && r._original) {
      for (const col of columns.value) {
        if (col in r._original) r[col] = r._original[col]
      }
    }
    r._editState = 'original'
    kept.push(r)
  }
  // 撤销会复活所有被删行，deletedRids 一并清空（与上面 displayRows 的过滤保持一致）
  deletedRids.value = new Set()
  if (kept.length !== rows.value.length) {
    rows.value = kept
    rebuildRowIndex()
  }
  selectedSet.value = new Set()
  activeCell.value = null
  editingCell.value = null
  nextTick(() => { const host = scrollHost(); if (host) host.scrollTop = 0; syncViewport() })
}

const saveChanges = async () => {
  if (!props.conn?.id || !props.database || !props.table) return
  const pkCols = columnMetas.value.filter(c => c.primaryKey).map(c => c.name)
  const inserts = []
  const updates = []
  const deletes = []

  for (const row of rows.value) {
    if (row._editState === 'new') {
      const data = {}
      for (const col of columns.value) data[col] = row[col]
      inserts.push(data)
    } else if (row._editState === 'modified') {
      const orig = row._original || {}
      const data = {}
      // 只提交实际变更的列：ClickHouse 的 ALTER UPDATE 禁止 SET 排序键/主键列，
      // 全列提交必然带上 key 列报 CANNOT_UPDATE_COLUMN；对其它库也避免整行覆盖
      for (const col of columns.value) {
        if (String(row[col] ?? '') !== String(orig[col] ?? '')) data[col] = row[col]
      }
      if (!Object.keys(data).length) continue // 值未实际变化
      if (props.conn?.type === 'clickhouse') {
        const keyHit = Object.keys(data).filter(n => pkCols.includes(n))
        if (keyHit.length) {
          ElMessage.warning('ClickHouse 不允许修改主键/排序键列：' + keyHit.join(', '))
          return
        }
      }
      updates.push({ original: orig, row: data })
    } else if (row._editState === 'deleted') {
      deletes.push(row._original)
    }
  }

  if (inserts.length === 0 && updates.length === 0 && deletes.length === 0) {
    ElMessage.info(t('tdv.nothingToSave'))
    return
  }

  try {
    loading.value = true
    const res = await saveTableData(props.conn.id, {
      database: props.database,
      table: props.table,
      inserts,
      updates,
      deletes,
      pkColumns: pkCols.length ? pkCols : undefined
    })
    if (res.success) {
      ElMessage.success(t('tdv.saveOk'))
      await load(page.value)
    } else {
      ElMessage.error('保存失败：' + res.message)
    }
  } catch (e) {
    ElMessage.error('保存失败：' + (e.message || t('common.unknownError')))
  } finally {
    loading.value = false
  }
}

const refreshData = async () => {
  selectedSet.value = new Set()
  activeCell.value = null
  editingCell.value = null
  await load(page.value)
  ElMessage.success(t('tdv.refreshOk'))
}

// ========== 列宽拖拽调整（列头与任意数据行竖线均可拖动） ==========
const colWidths = ref({})

// ========== 列宽持久化（按 连接 / 库 / 表）==========
// 列宽是"我给这张表调好的"，切走页签、重启后理应还在 —— 以前只存在内存里，切走就丢，
// 每次回来都得重调一遍。只存**手动调过**的列：
// 自动测量的列不存，免得把"测量结果"固化成用户设定，以后改了字体/密度反而跟着变旧。
// 存储键带版本号：表头布局改版会改变「表头完整显示」所需的宽度预算，
// 旧版本下保存的窄列宽会让新表头截断（真机：表头全是 ord…/sale…，右侧却留白）。
// 升 v2 让旧存档一次性作废，自动测量（保证表头完整）重新接管；此后用户拖过的宽度照常记住。
const colWidthStoreKey = () => 'xplore.colw:v2:' + [props.conn?.id || '', props.database || '', props.table || ''].join('|')

const loadSavedColWidths = () => {
  if (!props.table) return
  try {
    const raw = localStorage.getItem(colWidthStoreKey())
    if (!raw) return
    const saved = JSON.parse(raw)
    if (!saved || typeof saved !== 'object') return
    const next = { ...colWidths.value }
    const manual = new Set(manualCols.value)
    for (const [col, w] of Object.entries(saved)) {
      const n = Number(w)
      if (!Number.isFinite(n) || n <= 0) continue
      // 自愈下限：旧存档可能是在旧表头预算下保存的窄宽度，直接用会让表头截断
      // （真机：表头全是 ord…/sale…，右侧却留白）—— 至少抬到「表头完整显示」所需宽度
      //（canvas 按粗体实测 + 非文字预算）。表头完整优先于"记住更窄的手动宽度"。
      const headerMin = Math.ceil(measureTextWidth(col, true) + headerExtraOf(col))
      next[col] = Math.max(MIN_COL_WIDTH, Math.min(800, Math.round(Math.max(n, headerMin))))
      // 存过的算"用户手动列"：自动测量不再覆盖它（与拖拽后的行为一致）
      manual.add(col)
    }
    colWidths.value = next
    manualCols.value = manual
  } catch (e) { /* 存的东西坏了就当没有，不影响使用 */ }
}

const saveColWidths = () => {
  if (!props.table) return
  try {
    const out = {}
    for (const [col, w] of Object.entries(colWidths.value)) {
      if (manualCols.value.has(col) && Number.isFinite(w)) out[col] = Math.round(w)
    }
    localStorage.setItem(colWidthStoreKey(), JSON.stringify(out))
  } catch (e) { /* 隐私模式 / 配额满：存不进去就算了 */ }
}
const colResizing = ref(false)
const gridWrap = ref(null)
// 容器尺寸变化（拖拽侧栏、窗口缩放）时重算可视区，避免窗口化渲染留下空白
let vpObserver = null
watch(gridWrap, (el) => {
  if (vpObserver) { vpObserver.disconnect(); vpObserver = null }
  if (!el) return
  vpObserver = new ResizeObserver(() => { syncViewport() })
  vpObserver.observe(el)
})
let resizeState = null
let lastResizeAt = 0
let lastDragAt = 0
let lastHoverCell = null

const MIN_COL_WIDTH = 56
// 上限从 420 提到 520：长表头（如 order_created_at）本来就该完整显示，截断才是问题
const MAX_COL_WIDTH = 520
// 表头那一格除文字外要放下的东西（**真机量出来的实际宽度**，不是估算）：
//   左右内边距 20 + 左右边框 2 = 22；类型图标 17（每个数据列都有，模板里没有 v-if）；
//   主键图标 17（仅主键列，现在排在**第二行**排头）；排序图标 18（每列都有）。
// 于是 —— 普通列 59、主键列 76。历史上这里统一给 60：主键列正好差一个 🔑 的宽度、标题被省略号吃掉；
// 一口气给到 80 又反过来太肥（普通列白多 21px）。所以改成**按列算**，只有主键列多那 17px。
const HEADER_BASE_EXTRA = 22
// 类型徽章 18px + 其后间距 5px（第一行：徽章 + 字段名）
const HEADER_TYPE_ICON = 23
// 主键 🔑 12px 图标 + 前间距 4px（第一行字段名之后）
const HEADER_PK_ICON = 16
const HEADER_SORT_ICON = 18
const HEADER_CUSHION = 2
// 数据格：左右内边距 20px + 边框 2px + 一点余量
const CELL_EXTRA = 24
// 首列（行号 / 行头）宽度，与 CSS 里 .row-num-col / .leading-td 的 40px 保持一致
const ROW_NUM_COL_WIDTH = 40

// 某一列表头需要为「非文字部分」留出的宽度。
// 主键列多出的 17px 已经不算在**第一行**上（🔑 挪去了第二行，第一行只有类型图标 + 字段名），
// 但仍要算 —— 第二行的 🔑 占的正是这 17px；不加回去，🔑 就会把注释挤成省略号。
const headerExtraOf = (col) =>
  HEADER_BASE_EXTRA + HEADER_TYPE_ICON + HEADER_SORT_ICON + HEADER_CUSHION
  + (isPkCol(col) ? HEADER_PK_ICON : 0)

/**
 * 表头文字块（第一行"类型图标 + 字段名"、第二行"🔑 + 注释"）的可用宽度 = 列宽 − **块外**的部分。
 * 注意：类型/主键图标已经放进文字块里了（见模板的 .th-line1 / .th-line2），所以这里**不能**再减它们的宽度 ——
 * 减了就是减两次，列名会被凭空挤到省略（这个坑我踩过一次）。
 * 块外只剩下：左右内边距 + 边框（HEADER_BASE_EXTRA）与排序图标（HEADER_SORT_ICON）。
 */
const labelMaxWidth = (col) => {
  // 走渲染口径（含表头最小宽度下限）：列宽被任何来源压到表头宽以下时，文字空间跟着渲染宽走
  const w = renderColWidth(col)
  // 块外 = 内边距+边框 + 排序图标。**类型徽章/主键 🔑 都在文字块内部**（见模板 .th-text），
  // 这里绝不能再减它们 —— 减了就是减两次，字段名会被凭空挤成省略号
  //（真机：order_no 列宽 119、maxW 只给了 56、字段名只剩 39px 截成 "ord…"，就是这个坑）。
  const outside = HEADER_BASE_EXTRA + HEADER_SORT_ICON
  return Math.max(40, Math.round(w - outside))
}

// ===== 渲染口径的列宽硬下限：**任何来源**（存档 / 测量 / 手动拖拽）都不得低于
// 「表头完整显示」所需宽度 —— 表头完整优先（拖动过程中直接写 DOM 不受影响，
// 松手后低于下限的会被抬回）。这是最后一道闸：上面无论哪条路径出漏子，表头都不会再截断。
const headerMinOf = (col) => Math.max(MIN_COL_WIDTH, Math.ceil(measureTextWidth(col, true) + headerExtraOf(col)))
const renderColWidth = (col) => Math.max(colWidths.value[col] || defaultColWidth(col), headerMinOf(col))

// 名称兜底估算：CJK 按 13px、其它按 7px 估算（真正宽度由内容测量决定）
// 兜底也按「表头要放得下」给宽，否则测量完成前后会先闪一下被截断的表头
const defaultColWidth = (name) => {
  let w = 0
  for (const ch of String(name || '')) w += /[\u3000-\u9fff\uff00-\uffef]/.test(ch) ? 13 : 7
  return Math.max(MIN_COL_WIDTH, Math.min(MAX_COL_WIDTH, Math.round(w) + headerExtraOf(name)))
}

// ========== 列宽的**唯一一条公式** ==========
// 自动测量（加载/换列后）、双击表头右缘、右键「列宽自适应」三条路径全部走它 ——
// 以前自适应那条单独留着一份实现（旧的 +42 预算、800 的上限、50 行采样），
// 于是"双击自适应"得到的宽度和"默认宽度"不一样，看着像没生效。
const COL_SAMPLE_ROWS = 30
// 内容宽取 P95 而不是最大值：一列里偶尔有个超长值（长备注、长 JSON）
// 不该把整列撑到上限、把其它列全挤出屏幕；满足 95% 的行完整可读就够了。
// 表头那行仍取最大值 —— 标题必须完整，这条不能让步。
const COL_CONTENT_PERCENTILE = 0.95
/**
 * 一列「正好放得下」的宽度：表头按**最大值**算（标题要完整），内容按 **P95** 算，取大者。
 * 表头要额外留出内边距 + 边框 + 类型/主键图标 + 排序图标（见 headerExtraOf），内容只需内边距。
 */
const naturalColWidth = (col, fontFamily, sample) => {
  const headerW = measureTextWidth(col, true, fontFamily) + headerExtraOf(col)
  const widths = []
  for (const row of sample) {
    const v = row[col]
    widths.push(measureTextWidth(v == null ? 'NULL' : String(v), false, fontFamily))
  }
  let cellW = 0
  if (widths.length) {
    widths.sort((a, b) => a - b)
    const idx = Math.min(widths.length - 1, Math.floor(widths.length * COL_CONTENT_PERCENTILE))
    cellW = widths[idx]
  }
  const need = Math.max(headerW, cellW + CELL_EXTRA)
  return Math.max(MIN_COL_WIDTH, Math.min(MAX_COL_WIDTH, Math.ceil(need)))
}

// 表格总宽 = 行号列 + 各可见列宽之和，用内联 width 显式给出。
// 不能依赖 CSS 的 width: max-content：table-layout: fixed 下浏览器是按「内容」算 max-content 的，
// 再把多出来的空间摊回到各列上，于是 <col> 上写的小宽度（比如拖到 8px）会被撑回去，
// 表现就是「列宽拖不窄」。
const tableWidth = computed(() => {
  let w = ROW_NUM_COL_WIDTH
  for (const col of visibleColumns.value) w += renderColWidth(col)
  return Math.round(w)
})

// 为新出现的列补默认宽度（内容测量前先用名称估算兜底）
const ensureColWidths = (cols) => {
  const next = { ...colWidths.value }
  for (const c of cols) {
    if (!next[c]) next[c] = defaultColWidth(c)
  }
  colWidths.value = next
}

// 用 canvas 精确测量文本宽度（与表格字体一致）。
// 不再用单元格 scrollWidth：table-layout:fixed + overflow:hidden 下它会被当前列宽污染，导致宽度越测越大。
let colMeasureCtx = null
let colMeasureFont = ''
// 表格的**真实字体**（含字号），由 measureColWidths 从 DOM 现取。
// 原来这里把字号写死成 12px，而 .data-table 渲染的是 13px —— 所有文本都被低估约 8%，
// 差的那几像素正好让表头掉进 text-overflow: ellipsis 里。
let colMeasureFontSpec = { size: 13, family: 'sans-serif' }
const measureTextWidth = (text, bold, fontFamily) => {
  if (!colMeasureCtx) colMeasureCtx = document.createElement('canvas').getContext('2d')
  const family = fontFamily || colMeasureFontSpec.family
  const font = `${bold ? 600 : 400} ${colMeasureFontSpec.size}px ${family}`
  if (font !== colMeasureFont) { colMeasureCtx.font = font; colMeasureFont = font }
  return colMeasureCtx.measureText(text == null ? '' : String(text)).width
}

// 用户手动拖拽过的列：后续自动测量不再覆盖
const manualCols = ref(new Set())

// 智能列宽：列宽 = clamp(表头与前 30 行内容的真实文本宽 + 内边距, MIN, MAX)；
// 不按容器宽度放大（不刻意撑满）：字段少时表格就窄一些，右侧自然留白。
// 手动拖拽过的列保持不动。
let lastMeasuredKey = ''
const measureColWidths = () => {
  const wrap = gridWrap.value
  const cols = visibleColumns.value
  if (!wrap || !cols.length) return
  const key = cols.join('\u0001')
  if (key === lastMeasuredKey) return
  lastMeasuredKey = key

  const tableEl = wrap.querySelector('table')
  const cs = tableEl ? getComputedStyle(tableEl) : null
  const fontFamily = cs ? cs.fontFamily : 'sans-serif'
  // 字号也从真实表格取（原来是写死的 12px，比 .data-table 的 13px 小一号）
  const fontSize = cs ? (parseFloat(cs.fontSize) || 13) : 13
  if (fontSize !== colMeasureFontSpec.size || fontFamily !== colMeasureFontSpec.family) {
    colMeasureFontSpec = { size: fontSize, family: fontFamily }
    colMeasureFont = ''
  }
  const sample = displayRows.value.slice(0, COL_SAMPLE_ROWS)
  // 与双击 / 右键「列宽自适应」共用同一个 naturalColWidth：默认宽度与自适应宽度必然相等
  const natural = cols.map((col) => naturalColWidth(col, fontFamily, sample))

  // 列宽只取内容自然宽度（手动拖过 / 右键自适应的列保持不动），
  // 不再为「撑满容器」而放大列 —— 字段少时表格就窄一些，右边留白
  const next = { ...colWidths.value }
  cols.forEach((col, i) => {
    next[col] = Math.round(manualCols.value.has(col) ? (colWidths.value[col] || natural[i]) : natural[i])
  })
  colWidths.value = next
}

// 列显示/隐藏变化后重新按可见列测量宽度
watch(() => visibleColumns.value.join('\u0001'), () => {
  lastMeasuredKey = ''
  nextTick(() => { if (rows.value.length && visibleColumns.value.length) measureColWidths() })
})

const onColResizeStart = (col, e) => {
  const startWidth = colWidths.value[col] || defaultColWidth(col)
  // 这里**不再创建那条竖直参考线**（原 .col-resize-guide）。
  // 列宽本来就实时跟着光标变（见 onColResizeMove），边上再飘一根蓝线属于多余信息，
  // 而且它还要处理"抓取点偏移"才不至于偏在列线左边 —— 直接去掉最省事。
  // 数据表与 SQL 结果表两处一起去掉了它。
  //
  // 拖动期间**不写响应式状态**，直接改 <col> / <table> 的 style（见 onColResizeMove）。
  // 原因：本组件行数 ≤500 时不启用窗口化（VP_THRESHOLD），而一页就是 200 行 × 13 列 ——
  // 每写一次 colWidths 都会重建整棵 VNode 树，实测帧间隔中位 34ms / P90 51ms，拖起来就是"卡"。
  // 列宽只是两个 style，直接写 DOM 只要几微秒；松手时再提交一次（见 onColResizeEnd）。
  // 同一份文件里「拖动调整列顺序」早就是这么做的（见 onColDragStart 附近的说明）。
  const idx = visibleColumns.value.indexOf(col)
  const wrap = gridWrap.value
  const colEl = wrap && idx >= 0 ? wrap.querySelectorAll('colgroup col')[idx + 1] : null
  const tableEl = wrap ? wrap.querySelector('table.data-table') : null
  resizeState = {
    col, startWidth, startX: e.clientX, lastX: e.clientX, nextW: startWidth,
    colEl, tableEl, baseTableWidth: tableWidth.value, raf: 0
  }
  // 拖拽一开始就标记为「用户手动列」，后续自动测量不再覆盖（避免松手后被测量打回自然宽度）
  manualCols.value = new Set(manualCols.value).add(col)
  document.body.style.cursor = 'col-resize'
  document.body.style.userSelect = 'none'
  document.addEventListener('mousemove', onColResizeMove)
  document.addEventListener('mouseup', onColResizeEnd)
}

const onColResizeMove = (e) => {
  if (!resizeState) return
  resizeState.lastX = e.clientX
  // 实时应用列宽：只写 DOM（<col> 的宽 + <table> 的总宽），一帧最多写一次。
  // 不写 colWidths —— 那会重建 200 行 × 13 列的 VNode 树，每动一下鼠标 ~30ms（就是"卡"的来源）。
  // 表格总宽必须跟着走：CSS 是 table-layout: fixed，总宽不涨的话拖宽会被压回去。
  resizeState.nextW = Math.max(8, Math.round(resizeState.startWidth + (e.clientX - resizeState.startX)))
  if (resizeState.raf) return
  resizeState.raf = requestAnimationFrame(() => {
    const st = resizeState
    if (!st) return
    st.raf = 0
    if (st.colEl) st.colEl.style.width = st.nextW + 'px'
    if (st.tableEl) st.tableEl.style.width = Math.round(st.baseTableWidth + (st.nextW - st.startWidth)) + 'px'
  })
}

const onColResizeEnd = () => {
  if (resizeState) {
    lastResizeAt = Date.now()
    const finalW = Math.max(8, resizeState.startWidth + (resizeState.lastX - resizeState.startX))
    // 拖动期间只改了 DOM 的 style，这里把最终宽度提交给响应式状态 —— 整场拖动只重渲染这一次
    if (resizeState.raf) { cancelAnimationFrame(resizeState.raf); resizeState.raf = 0 }
    colWidths.value = { ...colWidths.value, [resizeState.col]: Math.round(finalW) }
    // 记录为用户手动设定的宽度，后续自动测量不再覆盖
    manualCols.value = new Set(manualCols.value).add(resizeState.col)
    saveColWidths()   // 顺手记到本表名下：切走再回来还是这个宽度
  }
  resizeState = null
  colResizing.value = false
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
  if (gridWrap.value) gridWrap.value.style.cursor = ''
  if (lastHoverCell) { lastHoverCell.style.cursor = ''; lastHoverCell = null }
  document.removeEventListener('mousemove', onColResizeMove)
  document.removeEventListener('mouseup', onColResizeEnd)
}

onUnmounted(() => {
  window.removeEventListener('keydown', onKeyDown)
  document.removeEventListener('mousemove', onColDragMoveDoc)
  document.removeEventListener('mouseup', onColDragEndDoc)
  document.body.style.cursor = ''
  if (loadController) { loadController.abort(); loadController = null }
  if (loadTimer) { clearInterval(loadTimer); loadTimer = null }
  if (resizeState) onColResizeEnd()
  if (vpRaf) { cancelAnimationFrame(vpRaf); vpRaf = 0 }
  if (vpObserver) { vpObserver.disconnect(); vpObserver = null }
  // 表格是懒加载页签，关掉页签即卸载；导出进行中关掉页签，轮询不能留着继续打后端
  exportTask.close()
})

// 鼠标悬停在某列的右缘竖线上时返回该列名；否则 null
// 只在**表头**上判定，且热区很窄（Excel / DataGrip 同款）：
// 之前在数据格上也按 ±10px 判定，窄列两侧热区几乎盖满整格，
// 鼠标"放进单元格里"就会突然变成左右箭头，和普通单元格（手型）不一致
const EDGE_GAP = 5
const colAtEdge = (e) => {
  const cell = e.target.closest('th')
  if (!cell || !cell.closest('table')) return null
  const rect = cell.getBoundingClientRect()
  const ci = cell.cellIndex
  const x = e.clientX
  // 靠近右缘 -> 调整本列
  if (x >= rect.right - EDGE_GAP && x <= rect.right + EDGE_GAP) {
    const idx = ci - 1 // 第0列是行号列
    return idx >= 0 && idx < visibleColumns.value.length ? visibleColumns.value[idx] : null
  }
  // 靠近左缘 -> 调整左侧相邻列
  if (x >= rect.left - EDGE_GAP && x <= rect.left + EDGE_GAP) {
    const idx = ci - 2
    return idx >= 0 && idx < visibleColumns.value.length ? visibleColumns.value[idx] : null
  }
  return null
}

const onGridMove = (e) => {
  const wrap = gridWrap.value
  if (!wrap || resizeState || isColDragging()) return
  const cell = e.target.closest('th, td')
  // 只清除上一个 hover cell 的 cursor，避免 querySelectorAll 遍历全表
  if (lastHoverCell && lastHoverCell !== cell) {
    lastHoverCell.style.cursor = ''
  }
  const col = colAtEdge(e)
  if (col && cell) {
    cell.style.cursor = 'col-resize'
    lastHoverCell = cell
  } else {
    lastHoverCell = null
  }
  wrap.style.cursor = col ? 'col-resize' : ''
}

const onGridLeave = () => {
  const wrap = gridWrap.value
  if (wrap) {
    if (!resizeState) wrap.style.cursor = ''
    if (lastHoverCell) { lastHoverCell.style.cursor = ''; lastHoverCell = null }
  }
}

const onGridDown = (e) => {
  if (e.button !== 0) return
  const col = colAtEdge(e)
  if (!col) {
    // 在数据格上按下 = 开始「单元格区域」这块选区：先清掉行/列选择。
    // Shift+按下 = 扩展已有区域（由框选模块处理），保留当前选区，这里不清
    if (!e.shiftKey && e.target && e.target.closest && e.target.closest('td[data-gkey]')) focusCells()
    return
  }
  onColResizeStart(col, e)
  e.preventDefault()
}

// 表格框选复制（类 Excel）：排除列宽拖拽边缘、编辑输入框与表头（表头留给拖拽换列）
const gridSel = useExcelSelection({
  container: gridWrap,
  // 编辑中不启动框选：这样 mousedown 不会被 preventDefault，输入框能正常失焦 → 结束编辑。
  // 无修饰键拖动 = 新起一块区域；Shift+点击/拖动 = 扩展已有区域（模块内部先处理，见 onDown）
  shouldStart: (e) => !e.shiftKey && !editingCell.value && !e.target.closest('.cell-input') && !e.target.closest('th') && !colAtEdge(e),
  // 复制按数据取值：窗口化渲染下跨屏大选区也能完整复制
  valueAt: cellTextAt,
  // 复制完成 → 按场景提示
  onCopied: onGridCopied
})
// 框选存在时不再单独画活动单元格描边，否则屏幕上会同时出现「两块选区」
const hasGridSelection = computed(() => !!gridSel?.range?.value)
/**
 * 「当前没有行 / 列 / 框选」——活动单元格描边的前提条件。
 *
 * 收成一个布尔 computed 供单元格复用：否则每格都要单独读 selectedSet.size /
 * selectedCols.size，拖选、多选时每帧都会因为「Set 大小变了」把可视区上千格全部重渲染。
 * 布尔值没变时不传播（computed 值稳定性），所以拖选时这层依赖是零成本的。
 */
const noBulkSelection = computed(() =>
  !selectedSet.value.size && !selectedCols.value.size && !hasGridSelection.value)
// 三类选区（行 / 列 / 单元格框选）只能同时存在一块：框选区域一出现就清掉行、列选中。
// 否则「行选中」和「框选」会各自画各自的边线，看起来像碎成好几块
// （典型症状：全选若干行后又在某一行拖出一个小框，那一行会套一个额外的框）
watch(() => gridSel?.range?.value, (r) => {
  if (!r) return
  if (selectedSet.value.size) selectedSet.value = new Set()
  if (headerSelected.value) headerSelected.value = false
  if (selectedCols.value.size) clearColSelect()
})

// 点击表格以外的界面 → 取消当前选中（活动单元格描边 / 选中列；框选由框选模块自己清）
const onDocClearPick = (e) => {
  const t = e.target
  if (!t || !t.closest) return
  if (t.closest('.grid-ctx-menu, .el-popper')) return // 菜单 / 弹层内部不处理
  // 只有落在「单元格 / 表头」上才算表格内点击（交给表格自己的逻辑切换选区）；
  // 滚动容器的空白处（表格右侧、下方留白）算外部 → 取消选中
  if (t.closest('.data-table-wrap') && t.closest('th, td')) return
  if (activeCell.value) activeCell.value = null
  if (selectedCols.value.size) clearColSelect()
  // 点表格以外的界面：整块选中状态一起消失。
  // 例外只有「工具栏上的交互控件」——按钮 / 下拉 / 复选 / 分页，它们要用当前选择
  // （删除选中行、导出…），其余位置一律取消
  if ((selectedSet.value.size || headerSelected.value) &&
      !t.closest('button, .el-button, .el-dropdown, .el-checkbox, .el-radio, .el-switch, .el-pagination')) {
    clearRowSelection()
  }
}
document.addEventListener('mousedown', onDocClearPick)
onBeforeUnmount(() => document.removeEventListener('mousedown', onDocClearPick))

// ========== 原有高级搜索逻辑 ==========
const OPS = {
  string: [
    { value: 'contains',     label: t('cop.like') },
    { value: 'not_contains', label: t('cop.notlike') },
    { value: 'eq',           label: t('tdv.opEq') },
    { value: 'ne',           label: t('tdv.opNe') },
    { value: 'starts_with',  label: t('tdv.opStartsWith') },
    { value: 'ends_with',    label: t('tdv.opEndsWith') },
    { value: 'is_null',      label: t('cop.isnull') },
    { value: 'is_not_null',  label: t('cop.isnotnull') }
  ],
  number: [
    { value: 'eq',   label: t('tdv.opEq') },
    { value: 'ne',   label: t('tdv.opNe') },
    { value: 'gt',   label: t('tdv.opGt') },
    { value: 'lt',   label: t('tdv.opLt') },
    { value: 'gte',  label: t('tdv.opGte') },
    { value: 'lte',  label: t('tdv.opLte') },
    { value: 'between', label: t('tdv.opBetween') },
    { value: 'is_null',     label: t('cop.isnull') },
    { value: 'is_not_null', label: t('cop.isnotnull') }
  ],
  date: [
    { value: 'eq',   label: t('tdv.opEq') },
    { value: 'ne',   label: t('tdv.opNe') },
    { value: 'gt',   label: t('tdv.opAfter') },
    { value: 'lt',   label: t('tdv.opBefore') },
    { value: 'gte',  label: t('tdv.opNotBefore') },
    { value: 'lte',  label: t('tdv.opNotAfter') },
    { value: 'between', label: t('tdv.opDateRange') },
    { value: 'is_null',     label: t('cop.isnull') },
    { value: 'is_not_null', label: t('cop.isnotnull') }
  ]
}
const opsOf = (t) => OPS[t] || OPS.string
const needsTwoValues = (op) => op === 'between'
const isNullOp = (op) => op === 'is_null' || op === 'is_not_null'

const classifyType = (typeName) => {
  const t = (typeName || '').toLowerCase().split('(')[0].trim()
  if (/^(int|bigint|smallint|tinyint|mediumint|decimal|numeric|float|double|real|bit|number|bool)/.test(t)) return 'number'
  if (/^(date|time|datetime|timestamp|year)/.test(t)) return 'date'
  return 'string'
}

// 默认运算符用「=」，各类型都支持，避免出现下拉框显示空（旧默认 contains 数值列没有）
const DEFAULT_FILTER_OP = 'eq'
const pickDefaultOp = (colType) => {
  const ops = opsOf(colType)
  return ops.some(o => o.value === DEFAULT_FILTER_OP) ? DEFAULT_FILTER_OP : ops[0].value
}
const addFilter = () => {
  const firstMeta = columnMetas.value[0]
  filters.value.push({
    join: 'AND',
    col: firstMeta ? firstMeta.name : '',
    op: pickDefaultOp(firstMeta ? firstMeta.colType : 'string'),
    value: '',
    value2: '',
    colType: firstMeta ? firstMeta.colType : 'string'
  })
}
const removeFilter = (idx) => { filters.value.splice(idx, 1) }
const resetFilters = () => { filters.value = [] }

const onColChange = (f) => {
  const meta = columnMetas.value.find(c => c.name === f.col)
  f.colType = meta ? meta.colType : 'string'
  f.op = pickDefaultOp(f.colType)
  f.value = ''; f.value2 = ''
}
const onFilterChange = () => { /* 仅触发响应式刷新 */ }

const loadColumns = async () => {
  columnMetas.value = []
  if (!props.table) return
  try {
    const res = await listColumns(props.conn.id, props.database, props.table)
    columnMetas.value = (res || []).map(c => ({
      name: c.name,
      type: c.type,
      colType: classifyType(c.type),
      primaryKey: c.primaryKey,
      nullable: c.nullable,
      defaultValue: c.defaultValue,
      // 字段注释：宿主 /columns 早就带回来了（MySQL/Doris 走 information_schema.column_comment，
      // 由 web 侧 enrich_column_types 合并进响应），以前这里没接 —— 注释就是在这一步丢的。
      comment: c.comment || ''
    }))
    // 列信息就绪后，把这张表上次调好的列宽接回来（切页签 / 重启不丢）
    loadSavedColWidths()
  } catch (e) { /* ignore */ }
}

const load = async (p = 1, opts = null) => {
  // 取消上一次未完成的加载（连续翻页 / 重新查询时避免结果错乱）
  if (loadController) loadController.abort()
  const controller = new AbortController()
  loadController = controller
  const signal = controller.signal
  page.value = p
  loading.value = true
  running.value = true
  const start = Date.now()
  elapsedTime.value = 0
  if (loadTimer) clearInterval(loadTimer)
  loadTimer = setInterval(() => { elapsedTime.value = Date.now() - start }, 100)
  try {
    const cond = filters.value
      .filter(f => f.col && f.op && (isNullOp(f.op) || f.value !== ''))
      .map(f => ({ join: f.join, col: f.col, op: f.op, value: f.value, value2: f.value2, colType: f.colType }))
    const params = {
      database: props.database, table: props.table,
      page: page.value, size: size.value,
      orderColumn: orderColumn.value || undefined, orderDir: orderDir.value || undefined
    }
    if (cond.length) params.filters = JSON.stringify(cond)
    // 深分页：相邻翻页时把排序游标交给后端走 keyset，避免大 OFFSET
    if (opts && opts.cursor !== undefined && opts.cursor !== null) {
      params.cursor = String(opts.cursor)
      params.cursorDir = opts.cursorDir
    }
    const res = await getTableData(props.conn.id, params, signal)
    if (signal.aborted) return
    if (!res.success) { ElMessage.error('加载失败：' + res.message); return }
    rows.value = stampRows(res.rows || [])
    // 新数据用新的 _rid，旧的隐藏行标记已失效，清掉避免菜单里出现t('tdv.showAllRows')却什么都没隐藏
    hiddenRows.value = new Set()
    deletedRids.value = new Set()
    columns.value = res.columns
    ensureColWidths(res.columns)
    // 翻页/刷新后回到顶部并重置可视窗口
    vpStart.value = 0
    vpEnd.value = Math.min(displayRows.value.length, 100)
    nextTick(() => {
      const host = scrollHost()
      if (host) host.scrollTop = 0
      syncViewport()
      // 渲染完成后按真实内容测量列宽（与 SQL 查询预览一致）
      if (rows.value.length && columns.value.length) measureColWidths()
    })
    if (typeof res.totalCount === 'number' && res.totalCount >= 0) {
      total.value = res.totalCount
    } else {
      const totalNotice = (res.notices || []).find(n => n.startsWith('total='))
      total.value = totalNotice ? parseInt(totalNotice.split('=')[1]) : res.rows.length
    }
    // 把分页精确总数同步到左侧树节点，解决 information_schema.TABLE_ROWS 近似值不一致问题
    if (props.conn && props.table) {
      emit('update-table-rows', { connId: props.conn.id, database: props.database, table: props.table, rows: total.value })
    }
  } catch (e) {
    // 用户取消 / 被新请求替换：静默返回，不动已有数据
    if (signal.aborted || e?.name === 'AbortError') return
    ElMessage.error('加载失败：' + (e?.message || e))
  } finally {
    // 仅当仍是当前请求时才复位状态，避免旧请求把新请求的 loading 关掉
    if (loadController === controller) {
      loadController = null
      if (loadTimer) { clearInterval(loadTimer); loadTimer = null }
      elapsedTime.value = Date.now() - start
      loading.value = false
      running.value = false
    }
  }
}

// 取消进行中的加载（翻页 / 查询 / 刷新）
const cancelLoad = () => {
  if (!loadController) return
  loadController.abort()
  ElMessage.info(t('tdv.loadCancelled'))
}

// 深分页游标：只有「相邻页 + 按唯一列（主键）排序」才可用 keyset。
// 取不到游标时后端自动退回 OFFSET 分页，结果一致，只是没加速。
function cursorFor (targetPage) {
  if (!orderColumn.value || !rows.value.length) return null
  const sortUnique = columnMetas.value.some(m => m.name === orderColumn.value && m.primaryKey)
  if (!sortUnique) return null
  const val = (row) => {
    const v = row ? row[orderColumn.value] : undefined
    return (v === undefined || v === null) ? undefined : v
  }
  if (targetPage === page.value + 1) {
    const v = val(rows.value[rows.value.length - 1])
    return v === undefined ? null : { cursor: v, cursorDir: 'after' }
  }
  if (targetPage === page.value - 1) {
    const v = val(rows.value[0])
    return v === undefined ? null : { cursor: v, cursorDir: 'before' }
  }
  return null
}

const goPage = (p) => load(p, cursorFor(p))

const toggleSort = (col) => {
  if (Date.now() - lastResizeAt < 300) return // 拖拽列宽后的 click 不触发排序
  if (Date.now() - lastDragAt < 300) return // 拖拽调整列顺序后的 click 不触发排序
  if (gridSel?.suppressClick()) return // 框选拖拽表头列后的 click 不触发排序
  if (orderColumn.value === col) {
    if (orderDir.value === 'ASC') orderDir.value = 'DESC'
    else if (orderDir.value === 'DESC') { orderColumn.value = ''; orderDir.value = '' }
    else { orderColumn.value = col; orderDir.value = 'ASC' }
  } else {
    orderColumn.value = col
    orderDir.value = 'ASC'
  }
  load(1)
}

// ========== 选中整列（Excel 式）：单击表头选中整列，Ctrl/Cmd 加减选，Shift 连选一段 ==========
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
// 表头点击：单击选中整列、Ctrl/Cmd 加/减选、Shift 连选一段（排序改点表头右侧的按钮）
const onHeaderClick = (col, e) => {
  if (Date.now() < colSelClickUntil) return // 刚拖过连选，紧接着的这次 click 忽略
  // Shift+点击表头 = 扩展单元格区域（框选模块负责），不要再切列选中
  if (e.shiftKey && gridSel && gridSel.hasSelection()) return
  focusCols() // 列选择这块选区生效时，清掉行选择与框选
  if (e.ctrlKey || e.metaKey) { toggleColSelect(col); return }
  if (e.shiftKey) { selectColRange(col); return }
  selectSingleCol(col)
}

// ========== 列头拖动连选多列（Excel 表头行为）==========
// 与行号列拖选同一套思路：document mousemove + elementFromPoint 命中表头，
// 从按下那一列连选到指针所在列；表头里的小按钮（排序/列宽把手）不受影响。
let colSelDrag = null
let colSelClickUntil = 0
const onColSelectDown = (col, e) => {
  focusCols() // 拖表头连选列前先清掉行选择与框选
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
  // 表头内左右拖：连选多列
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
  // 拖动过就连选的 click 一并抑制掉（同一格按下并抬起时浏览器会补发 click）
  if (st && st.moved) colSelClickUntil = Date.now() + 250
}
onBeforeUnmount(() => {
  colSelDrag = null
  document.removeEventListener('mousemove', onColSelectMove)
})

// 展示格式化：ISO 时间戳的 T 换成空格（编辑仍用原始值，见 editingCell 的取值）
const formatCell = (v) => v === null || v === undefined ? nullDisplay() : formatDbValue(v)

// ========== 导出（当前页 / 全部）：按当前筛选+排序构造查询，落地为后端导出 ==========
const quoteIdent = (s) => {
  const style = quoteStyleOf(props.conn?.type)
  if (style === 'BACKTICK') return '`' + String(s).replace(/`/g, '``') + '`'
  if (style === 'BRACKET') return '[' + String(s).replace(/\]/g, ']]') + ']'
  return '"' + String(s).replace(/"/g, '""') + '"'
}
const lit = (v, colType) => {
  if (v == null) return 'NULL'
  if (colType === 'number') return String(v)
  return "'" + String(v).replace(/'/g, "''") + "'"
}
const opToSql = (f) => {
  const col = quoteIdent(f.col)
  const val = f.value == null ? '' : String(f.value).replace(/'/g, "''")
  switch (f.op) {
    case 'contains':      return `${col} LIKE '%${val}%'`
    case 'not_contains':  return `${col} NOT LIKE '%${val}%'`
    case 'starts_with':   return `${col} LIKE '${val}%'`
    case 'ends_with':     return `${col} LIKE '%${val}'`
    case 'eq':            return `${col} = ${lit(f.value, f.colType)}`
    case 'ne':            return `${col} <> ${lit(f.value, f.colType)}`
    case 'gt':            return `${col} > ${lit(f.value, f.colType)}`
    case 'lt':            return `${col} < ${lit(f.value, f.colType)}`
    case 'gte':           return `${col} >= ${lit(f.value, f.colType)}`
    case 'lte':           return `${col} <= ${lit(f.value, f.colType)}`
    case 'between':       return `${col} BETWEEN ${lit(f.value, f.colType)} AND ${lit(f.value2, f.colType)}`
    case 'is_null':       return `${col} IS NULL`
    case 'is_not_null':   return `${col} IS NOT NULL`
    default:              return null
  }
}
const filtersToWhere = () => {
  const cons = filters.value.filter(f => f.col && f.op && (isNullOp(f.op) || f.value !== ''))
  if (!cons.length) return ''
  const parts = cons.map(opToSql).filter(Boolean)
  if (!parts.length) return ''
  let sql = ' WHERE ' + parts[0]
  for (let i = 1; i < parts.length; i++) sql += ` ${cons[i].join || 'AND'} ${parts[i]}`
  return sql
}
const buildExportSql = () => {
  const tbl = quoteIdent(props.table)
  const ord = orderColumn.value ? ` ORDER BY ${quoteIdent(orderColumn.value)} ${orderDir.value || 'ASC'}` : ''
  return `SELECT * FROM ${tbl}${filtersToWhere()}${ord}`
}
const onExport = async (cmd) => {
  const [scope, format] = String(cmd || '').split('-')
  if (!['csv', 'excel'].includes(format)) return
  if (!props.conn?.id || !props.table) { ElMessage.warning(t('qa.pickTableFirst')); return }
  if (scope === 'all') {
    // 导出全部走异步任务，后端流式分页导出，前端实时显示进度与日志
    const payload = { format, sql: buildExportSql(), database: props.database, table: props.table }
    await exportTask.start(props.conn.id, payload, props.table, format)
    return
  }
  // current：走**同步单页接口**（后端只查这一页就回字节）。
  //
  // 不能走 exportTask.start —— 它默认提交到 /export/task（异步导出【全部】），
  // payload 里的 page/size 会被后端忽略，于是「导出当前页」变成「导出全表」。
  const payload = {
    format,
    sql: buildExportSql(),
    database: props.database,
    table: props.table,
    page: Math.max(1, page.value),
    size: Math.max(1, size.value)
  }
  try {
    const blob = await exportData(props.conn.id, payload)
    const saved = await saveExportBlob(blob, props.table, format)
    if (saved.canceled) return
    ElMessage.success(t('mv.exportSavedAs', { name: saved.name }))
  } catch (e) {
    ElMessage.error('导出失败：' + (e?.message || e))
  }
}

onMounted(async () => {
  window.addEventListener('keydown', onKeyDown)
  loading.value = true
  await loadColumns()
  load(1)
})
watch(() => [props.conn?.id, props.database, props.table], async () => {
  page.value = 1
  orderColumn.value = null
  orderDir.value = 'ASC'
  filters.value = []
  selectedSet.value = new Set()
  activeCell.value = null
  editingCell.value = null
  hiddenColumns.value = new Set()
  hiddenRows.value = new Set()
  deletedRids.value = new Set()
  manualCols.value = new Set()
  clearColSelect()
  loading.value = true
  await loadColumns()
  load(1)
})

// 可自定义快捷键分发（键位在「设置 → 快捷键」中调整），
// 仅在当前数据表格可见且焦点在其中时响应
// 焦点在输入类控件里时不抢「编辑类」按键：Delete / Ctrl+Delete / Ctrl+Shift+Z / Ctrl+D
// 在输入框里各自另有含义（删字符、删词、重做、书签），表格动作要让位
const inEditableFocus = () => {
  const ae = document.activeElement
  if (!ae || ae === document.body) return false
  if (ae.closest && ae.closest('.cell-input')) return true
  const tag = (ae.tagName || '').toLowerCase()
  return tag === 'input' || tag === 'textarea' || ae.isContentEditable === true
}
useShortcutScope(rootRef, {
  'data.refresh': () => {
    if (editingCell.value) confirmEdit()
    refreshData()
  },
  'data.save': () => {
    if (props.readOnly) return
    if (editingCell.value) confirmEdit() // 先提交正在编辑的单元格
    saveChanges()
  },
  'data.addRow': () => {
    if (props.readOnly || inEditableFocus()) return
    if (editingCell.value) confirmEdit() // 先提交正在编辑的单元格
    addRow()
  },
  'data.deleteRow': () => {
    if (props.readOnly || inEditableFocus()) return
    if (selectedSet.value.size) removeSelectedRows()
    else if (activeCell.value) removeRow(activeCell.value.row)
  },
  'data.revert': () => {
    if (props.readOnly || inEditableFocus()) return
    revertChanges()
  },
  // 类 Excel：向下填充 / 清空选中单元格 / 自适应列宽
  'data.fillDown': () => {
    if (inEditableFocus()) return
    if (editingCell.value) confirmEdit()
    fillDown()
  },
  'data.clearCells': () => {
    if (props.readOnly || inEditableFocus()) return
    clearSelectedCells()
  },
  'data.autoFit': () => {
    if (selectedCols.value.size) autoFitSelectedCols()
    else visibleColumns.value.forEach(c => autoFitCol(c))
  }
})
</script>

<style scoped>
.table-data-view { height: 100%; display: flex; flex-direction: column; padding: 10px; gap: 10px; }
.toolbar { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px; }
.toolbar .left, .toolbar .right { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.toolbar .right :deep(.el-button + .el-button) { margin-left: 0; }

.advanced-panel {
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: var(--dc-radius);
  padding: 10px 12px;
  display: flex; flex-direction: column; gap: 8px;
}
.empty-tip { color: var(--dc-text-dim); font-size: 13px; padding: 8px 0; }
.filter-rows { display: flex; flex-direction: column; gap: 6px; }
.filter-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
/* 第一个条件没有「且/或」下拉，用同宽占位让各行的字段列对齐 */
.join-placeholder { flex: 0 0 80px; width: 80px; }
.between-sep { color: var(--dc-text-dim); padding: 0 2px; }
.advanced-foot { display: flex; gap: 8px; padding-top: 4px; border-top: 1px dashed var(--dc-border); }

.slide-enter-active, .slide-leave-active { transition: max-height .25s ease, opacity .2s ease; overflow: hidden; }
.slide-enter-from, .slide-leave-to { max-height: 0; opacity: 0; }
.slide-enter-to, .slide-leave-from { max-height: 500px; opacity: 1; }

.grid-area { flex: 1; min-height: 0; position: relative; display: flex; flex-direction: column; background: var(--dc-bg-card); border: 1px solid var(--dc-border); border-radius: var(--dc-radius); overflow: hidden; }
.table-scroll { flex: 1; min-height: 0; overflow: auto; contain: layout paint; }
.table-scroll :deep(.el-empty) { height: 100%; }
/* 加载遮罩（含取消按钮） */
.grid-loading-overlay {
  position: absolute; inset: 0; z-index: 20;
  display: flex; align-items: center; justify-content: center;
  background: var(--dc-bg-card);
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
.pager { display: flex; align-items: center; justify-content: space-between; padding: 6px 12px; border-top: 1px solid var(--dc-border); background: var(--dc-bg-soft); flex-shrink: 0; gap: 12px; }
.load-time { font-size: 13px; color: var(--dc-text-dim); font-weight: 500; }
.change-tip { color: var(--el-color-warning); margin-left: 8px; }
/* NULL：灰 + 斜体，明显区别于真实数据。
   对齐不在这里定死 —— 跟着字段类型走（见 utils/cellAlign.js 的 alignByType），
   所以下面的 .al-r / .al-c 必须写在本条之后，靠后者压住它。
   注意选择器要写成 .data-table td.null-cell，否则被 .data-table td 的 color 压住不生效 */
.data-table td.null-cell {
  color: var(--dc-text-weak, var(--dc-text-dim));
  font-style: italic;
}
/* Excel 对齐规则：数字 / 日期右对齐、布尔居中（判定见 utils/cellAlign.js）；
   NULL 单元格按字段类型拿到同样的类，于是和同列的真实值对齐一致。
   编辑框跟随单元格对齐，改数字时也是右对齐 */
.data-table td.al-r { text-align: right; }
.data-table td.al-c { text-align: center; }
.data-table td.al-r .cell-input,
.data-table td.al-c .cell-input { text-align: inherit; }

.data-table-wrap { height: 100%; overflow: auto; contain: layout paint; }
.data-table-wrap:focus { outline: none; }
/* 表格宽度 = 行号列 + 各列宽度之和，由模板上的内联 width（tableWidth）给出，不再 width:100%：
   —— 字段少时不会被拉伸去"撑满"容器（序号列也稳定在 40px）
   —— 字段多时自然超出容器，由 .data-table-wrap 横向滚动
   注意：这里不能写 width: max-content —— 固定布局下浏览器按「内容」计算 max-content，
   多出的空间会摊回各列，列宽就永远拖不窄（详见 tableWidth 的注释） */
.data-table { table-layout: fixed; border-collapse: collapse; font-size: 13px; }
.data-table col.row-num-col { width: 40px; }
.data-table th.fill-col, .data-table td.fill-col { padding: 0; border: none; background: transparent !important; min-width: 1px; }
/* 吸顶表头：`top: -1px` 是为了盖住滚动时表头上方那道 1px 的缝。
   表是 border-collapse: collapse，**折叠后的上边框由 table 画**，不跟着吸顶走 ——
   所以吸顶位置那 1px 会露出下层的页面底色，看着就是"表头上面有条缝"。 */
.data-table thead { position: sticky; top: -1px; z-index: 2; }
.data-table th {
  position: relative;
  background: var(--dc-bg-table-head); color: var(--dc-text-strong); font-weight: 600; text-align: left;
  /* 表头可能是两行（字段名 + 注释），高度交给内容 —— 原来写死 34px 会把第二行切掉 */
  padding: 5px 10px; height: auto; line-height: 1.3; vertical-align: middle;
  border: 1px solid var(--dc-border); white-space: nowrap;
  overflow: hidden;
}
.data-table th.sortable { cursor: pointer; user-select: none; }
.data-table th.sortable:hover { color: var(--dc-text); }
/* 已排序列：文字主色 + 底部 2px 主色条，排序状态更醒目 */
.data-table th.sort-asc, .data-table th.sort-desc { color: var(--dc-primary); box-shadow: inset 0 -2px 0 var(--dc-primary); }
/* 表头排序按钮：固定在列头右缘垂直居中（不随字段名长短晃动）；平时淡显，已排序列主色常亮 */
.data-table th .th-sort {
  position: absolute; right: 8px; top: 50%; transform: translateY(-50%);
  display: inline-flex; align-items: center;
  font-size: 13px; color: var(--dc-text-dim); z-index: 3;
  opacity: .35; cursor: pointer; transition: opacity .12s, color .12s;
}
.data-table th:hover .th-sort { opacity: .9; }
.data-table th .th-sort:hover { opacity: 1; color: var(--dc-primary); }
.data-table th .th-sort.is-sorted { opacity: 1; color: var(--dc-primary); }
/* 表头文字块：竖排两行 —— 第一行「类型徽章 + 字段名（+🔑）」，第二行注释顶格。
   宽度由模板按列宽算好（见 labelMaxWidth），超宽就在这个宽度里省略成 …。
   min-width: 0 不能省 —— 否则 flex 子项不肯被压缩，省略号就不会出现。 */
.data-table th .th-text {
  display: inline-flex; flex-direction: column; justify-content: center;
  vertical-align: middle; overflow: hidden; min-width: 0; max-width: 100%;
}
/* 第一行：类型徽章 + 字段名 */
.data-table th .th-line1 { display: flex; align-items: center; min-width: 0; }
/* 字段名同样是 flex 子项，min-width: 0 才能被压缩出省略号 */
.data-table th .th-label { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
/* 第二行：注释**顶格**开始（不缩进），没有注释时整行不渲染 */
.data-table th .th-line2 { display: flex; align-items: center; min-width: 0; margin-top: 1px; }
.data-table th .th-comment {
  font-size: 11px; font-weight: 400; color: var(--dc-text-dim);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;
}
/* 类型图标：裸图标（无底色/无固定盒子）—— 盒子会让 12px 图标在 18px 里居中、
   左右各空 3px，而第二行注释顶着盒子左缘，视觉上图标与注释就不对齐了。
   颜色由全局 index.css 的 th-t-* 按类型族给（这里不能再写 color，scoped 优先级会压掉它） */
.data-table th .th-type-ic {
  display: inline-flex; align-items: center; flex: 0 0 auto;
  margin-right: 5px; cursor: default;
}
.data-table th .th-type-ic .el-icon { font-size: 12px; }
/* 主键 🔑：字段名后面的小号裸图标（琥珀色） */
.data-table th .th-pk-ic { display: inline-flex; align-items: center; flex: 0 0 auto; margin-left: 4px; color: var(--dc-warning, #e6a23c); }
.data-table th .th-pk-ic .el-icon { font-size: 12px; }
.data-table td {
  padding: 0 10px; height: 32px; line-height: 32px;
  border: 1px solid var(--dc-border); color: var(--dc-text);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  cursor: default;
}

/* 列宽拖拽把手：加宽到 10px 并右移，覆盖表头右缘外侧，抓取更可靠。
   注意：**不再画 hover 提示条** —— 它原来是 10px 宽、35% 不透明的主色底，
   鼠标一移上去就在列边界处冒出一根半透明蓝条，看着像游离的线（SQL 结果表那边本来就没有它，
   两页观感也就此对齐）。可拖拽性由 cursor: col-resize 表达就够了。 */
.col-resizer {
  position: absolute; top: 0; right: -5px; bottom: 0; width: 10px;
  cursor: col-resize; z-index: 6; user-select: none;
}
.data-table.col-resizing { cursor: col-resize; user-select: none; }
.data-table td.editing { padding: 2px 4px; line-height: normal; }
.data-table tbody tr { cursor: pointer; }
/* 斑马纹按真实行号（row-alt）而非 DOM 奇偶：窗口化渲染会插入占位行，用 nth-child 会整列错位 */
.data-table tbody tr.row-alt td { background-color: var(--dc-bg-soft); }
/* ===== 行 / 列选中：与单元格框选同一套观感（整块淡色填充 + 沿整块外沿画 2px 主色边线）
   边线只画在整块的外沿：连续选中的行段/列段由 selEdges 标出首尾两端，中间不画内部线。
   注意：一律用 background-color —— background 简写会把下面这套 background-image 清掉 ===== */
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
/* 行选中（含标题行）：整行淡色底，整块四周一个框 */
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
/* 列选中：表头与该列单元格同底色，表头就是这块的顶边，到「最后一行」收底边 */
.data-table tbody tr td.col-selected { background-color: var(--dc-primary-soft); }
.data-table th.col-selected { --sel-t: 2px; background-color: var(--dc-primary-soft); }
/* 选中整列：四边都收口成完整矩形（与框选同一套框线语言，用户最新口径） */
.data-table th.col-selected.col-sel-l, .data-table td.col-selected.col-sel-l { --sel-l: 2px; }
.data-table th.col-selected.col-sel-r, .data-table td.col-selected.col-sel-r { --sel-r: 2px; }
.data-table tr.col-sel-bottom td.col-selected { --sel-b: 2px; }
.data-table tbody tr:hover td { background-color: var(--dc-primary-wash); }
/* 底栏选中区汇总：弱化显示、数字加粗，不抢分页的注意力（与 SQL 结果表底栏同款） */
.sel-summary { display: inline-flex; align-items: center; gap: 10px; font-size: 12px; color: var(--dc-text-dim); flex-wrap: wrap; margin-left: 4px; }
.sel-summary .ss-item { white-space: nowrap; }
.sel-summary .ss-item b { color: var(--dc-text); font-weight: 600; font-variant-numeric: tabular-nums; }
/* 窗口化渲染的上下占位行：只负责撑高，不显示边框/底色/指针 */
.data-table tbody tr.vp-pad-row { cursor: default; }
.data-table tbody tr.vp-pad-row td { padding: 0; border: 0; background: transparent !important; }
.data-table tbody tr.vp-pad-row:hover td { background: transparent !important; }
/* 行选中的填充与边线见上方「行 / 列选中」样式块 */
.data-table tbody tr.row-new td { background-color: var(--dc-success-wash) !important; }
.data-table tbody tr.row-modified td { background-color: var(--dc-warning-wash) !important; }
.data-table tbody tr.row-deleted { display: none; }

.row-num-th, .row-num-td {
  width: 40px; min-width: 40px; max-width: 40px;
  text-align: left; padding: 6px 6px;
  color: var(--dc-text-dim); font-size: 12px;
  border-right: 1px solid var(--dc-border);
}
.row-num-th { background-color: var(--dc-bg-table-head); }
/* 首列（行号列）的列名要像其它列头一样可读：行号本身是暗色小字，列名单独提亮 */
.data-table th.row-num-th { color: var(--dc-text-strong); font-size: 13px; }
/* 复选框首列（选中多行）：提高选择器优先级，覆盖 .row-num-th/.row-num-td 的 text-align:left */
.data-table th.leading-th, .data-table td.leading-td {
  width: 40px; min-width: 40px; max-width: 40px;
  text-align: center; padding: 0; vertical-align: middle; line-height: 1;
  border-right: 1px solid var(--dc-border);
}
.data-table th.leading-th { background-color: var(--dc-bg-table-head); cursor: pointer; }
/* 复选框在首列居中：去掉 el-checkbox 默认右外边距，避免视觉偏移 */
.leading-th :deep(.el-checkbox), .leading-td :deep(.el-checkbox) { margin-right: 0; }
.lead-check { height: 18px; display: inline-flex; }
/* 行号列（Excel 行头）：显示序号，点/拖选行；选中行时行号一起高亮
   注意：这里必须用 background-color，不能用 background 简写 ——
   简写会把「行/列选中」那套外沿渐变线（background-image）一并清掉，
   表现就是序号列缺边线、看着没被选中 */
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
/* 拖拽列排序时的视觉反馈 */
.data-table th.col-drag-over { box-shadow: inset 2px 0 0 var(--dc-primary); background: var(--dc-primary-wash); }
/* 选中的列（Ctrl/Cmd 点表头加选、Shift 连选）：表头高亮 + 底部主色条 */
/* 选中的列：底色与外沿边线见上方「行 / 列选中」样式块 */
.data-table th.col-dragging { opacity: 0.5; }
/* 活动单元格（键盘导航锚点） */
.data-table td.active-cell { outline: 2px solid var(--dc-primary); outline-offset: -2px; }

.cell-input {
  width: 100%; min-width: 60px;
  background: var(--dc-bg);
  border: 1px solid var(--dc-primary);
  border-radius: 3px;
  padding: 3px 6px;
  color: var(--dc-text);
  font-size: 13px;
  outline: none;
  box-sizing: border-box;
}

.grid-area :deep(.el-empty) { background: transparent; }


</style>

<style>
/* AI 筛选弹窗（el-dialog 挂载到 body，scoped 无法命中，故用全局块） */
.ai-filter-box .el-textarea__inner { border-radius: 10px; font-size: 13px; }
.ai-filter-tip { margin-top: 8px; font-size: 11.5px; color: var(--dc-text-dim); }
.ai-filter-loading { padding: 18px 0; text-align: center; color: var(--dc-text-dim); font-size: 13px; }
.ai-filter-label { margin-bottom: 6px; font-size: 13px; font-weight: 600; color: var(--dc-text-mid); }
.ai-filter-sql {
  background: var(--dc-bg-deep); border: 1px solid var(--dc-border); border-left: 3px solid var(--dc-primary);
  border-radius: 10px; padding: 12px 14px; margin: 0;
  font-family: "SF Mono", ui-monospace, Consolas, monospace;
  font-size: 13px; color: var(--dc-link); white-space: pre-wrap;
  max-height: 240px; overflow: auto; line-height: 1.65;
}
.ai-filter-notes { margin-top: 10px; font-size: 13px; color: var(--dc-warning, #e6a23c); line-height: 1.7; }

/* 表格右键上下文菜单（teleport 到 body，需全局样式） */
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


/* 「选择显示字段」下拉（popper 挂到 body，需全局样式） */
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
