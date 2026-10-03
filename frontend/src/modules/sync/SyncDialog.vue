<template>
  <el-dialog :model-value="modelValue" width="1020px" top="3vh" class="sync-dialog"
             :close-on-click-modal="false" :close-on-press-escape="!taskRunning"
             :before-close="onBeforeClose"
             append-to-body @update:model-value="$emit('update:modelValue', $event)">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Promotion /></el-icon></span>
        <span>{{ $t('sync.title') }}</span>
        <!-- 标题旁的说明文字已按需求删掉（sync.subtitle 词典保留但不再渲染） -->
      </div>
    </template>

    <!-- 顶部「源 → 目标」摘要条已按需求删除（信息在步骤 1 与摘要页里都有） -->

    <!-- ==================== 步骤条（与数据对比一致的向导式交互） ==================== -->
    <el-steps v-if="step <= 4" :active="step - 1" align-center finish-status="success" class="sync-steps">
      <el-step :title="$t('sync.stepSource')" />
      <el-step :title="$t('sync.stepObjects')" />
      <el-step :title="$t('sync.stepOptions')" />
      <el-step :title="$t('sync.stepSummary')" />
      </el-steps>

    <!-- ==================== 步骤 1：选择源 / 目标 ==================== -->
    <div v-if="step === 1">
      <div class="flow">
        <div class="side">
          <div class="side-head"><span class="dot src"></span>{{ $t('sync.source') }}</div>
          <el-select v-model="src.connectionId" size="small" :disabled="taskRunning" @change="onSrcConnChange">
            <el-option v-for="c in connections" :key="c.id" :label="c.name + ' (' + c.type + ')'" :value="c.id" />
          </el-select>
          <div class="pair">
            <el-select v-model="src.database" size="small" :disabled="taskRunning" @change="onSrcDbChange">
              <el-option v-for="d in srcDbs" :key="d" :label="d" :value="d" />
            </el-select>
            <el-select v-if="srcNeedSchema" v-model="src.schema" size="small" :disabled="taskRunning" @change="loadSrcObjects">
              <el-option v-for="s in srcSchemas" :key="s" :label="s" :value="s" />
            </el-select>
          </div>
        </div>

        <div class="flow-arrow">
          <div class="arrow-ring"><el-icon :size="18"><Right /></el-icon></div>
        </div>

        <div class="side">
          <div class="side-head"><span class="dot tgt"></span>{{ $t('sync.target') }}</div>
          <el-select v-model="tgt.connectionId" size="small" :disabled="taskRunning" @change="onTgtConnChange">
            <el-option v-for="c in connections" :key="c.id" :label="c.name + ' (' + c.type + ')'" :value="c.id" />
          </el-select>
          <div class="pair">
            <el-select v-model="tgt.database" size="small" :disabled="taskRunning" @change="onTgtDbChange">
              <el-option v-for="d in tgtDbs" :key="d" :label="d" :value="d" />
            </el-select>
            <el-select v-if="tgtNeedSchema" v-model="tgt.schema" size="small" :disabled="taskRunning">
              <el-option v-for="s in tgtSchemas" :key="s" :label="s" :value="s" />
            </el-select>
          </div>
        </div>
      </div>

      <!-- 连接信息摘要（Navicat 式两列） -->
      <div class="info-grid-wrap">
        <div class="info-col" v-if="srcInfo">
          <div class="info-title">{{ $t('sync.source') }}</div>
          <div class="info-row"><span>{{ $t('sync.infoType') }}</span><b>{{ srcInfo.type }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoName') }}</span><b>{{ srcInfo.name }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoHost') }}</span><b>{{ srcInfo.host }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoPort') }}</span><b>{{ srcInfo.port }}</b></div>
        </div>
        <div class="info-col" v-if="tgtInfo">
          <div class="info-title">{{ $t('sync.target') }}</div>
          <div class="info-row"><span>{{ $t('sync.infoType') }}</span><b>{{ tgtInfo.type }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoName') }}</span><b>{{ tgtInfo.name }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoHost') }}</span><b>{{ tgtInfo.host }}</b></div>
          <div class="info-row"><span>{{ $t('sync.infoPort') }}</span><b>{{ tgtInfo.port }}</b></div>
        </div>
      </div>
    </div>

    <!-- ==================== 步骤 3：选择对象 ==================== -->
    <div class="panel" v-if="step === 3 && src.connectionId && src.database">
      <div class="panel-head">
        <span class="panel-title"><el-icon :size="14"><FolderOpened /></el-icon>{{ $t('sync.pickObjects') }}</span>
        <span class="panel-count">{{ $t('sync.selectedPrefix') }}<b>{{ totalSelected }}</b> / {{ $t('sync.selectedSuffix', { total: totalAvailable }) }}</span>
      </div>

      <!-- 对象类型筛选 -->
      <div class="type-chips">
        <button v-for="t in objTypes" :key="t.key" type="button" class="type-chip"
                :class="{ active: t.checked, empty: objState[t.key].list.length === 0 }"
                :disabled="taskRunning || objState[t.key].list.length === 0"
                :title="objState[t.key].list.length ? $t('sync.hasN', { label: t.label, n: objState[t.key].list.length }) : $t('sync.noN', { label: t.label })"
                @click="toggleType(t)">
          <span class="chip-ic"><el-icon :size="13"><component :is="TYPE_ICON[t.key]" /></el-icon></span>
          <span class="chip-name">{{ t.label }}</span>
          <b class="chip-num">{{ objState[t.key].list.length }}</b>
          <i v-if="t.checked" class="chip-check"><el-icon :size="10"><Check /></el-icon></i>
        </button>
        <span v-if="!availableCount" class="no-obj-hint">{{ $t('sync.noObjects') }}</span>
      </div>

      <!-- 对象列表 -->
      <div class="obj-bar">
        <el-input v-model="keyword" size="small" clearable :prefix-icon="Search" :disabled="taskRunning" />
        <div class="obj-bar-actions">
          <el-button size="small" text :disabled="taskRunning || !availableCount" @click="applyAll(true)">{{ $t('sync.selectAll') }}</el-button>
          <el-button size="small" text :disabled="taskRunning || !totalSelected" @click="applyAll(false)">{{ $t('common.clear') }}</el-button>
        </div>
      </div>

      <div class="obj-scroll">
        <el-empty v-if="!filteredObjs.length" :description="keyword ? $t('sync.noMatch') : $t('sync.none')" :image-size="46" />
        <div v-for="o in filteredObjs" :key="o.type + '|' + o.name" class="obj-item" :class="{ on: isSelected(o) }">
          <span class="obj-type" :class="'ty-' + o.type">{{ typeAbbr(o.type) }}</span>
          <el-checkbox :model-value="isSelected(o)" :disabled="taskRunning" @change="v => toggleObj(o, v)">
            <span class="obj-name" :title="o.name">{{ o.name }}</span>
          </el-checkbox>
        </div>
      </div>
    </div>

    <!-- ==================== 步骤 2：传输选项 ==================== -->
    <div class="settings-card" v-if="step === 2">
      <div class="settings-head">
        <div class="settings-head-left">
          <el-icon :size="15"><Operation /></el-icon>
          <span>{{ $t('sync.settings') }}</span>
        </div>
      </div>
      <div class="settings-body">
        <div class="settings-col col-policy">
          <div class="settings-label"><el-icon :size="13"><Files /></el-icon>{{ $t('sync.existingTarget') }}</div>
          <el-radio-group v-model="opts.objectPolicy" size="small" :disabled="taskRunning">
            <el-radio value="merge">{{ $t('sync.modeMerge') }}</el-radio>
            <el-radio value="drop">{{ $t('sync.modeDrop') }}</el-radio>
          </el-radio-group>
          <p class="settings-hint">{{ opts.objectPolicy === 'merge'
            ? $t('sync.mergeHint')
            : $t('sync.dropHint') }}</p>
        </div>
        <div class="settings-col col-mode">
          <div class="settings-label"><el-icon :size="13"><DataLine /></el-icon>{{ $t('sync.dataMode') }}</div>
          <el-radio-group v-model="opts.dataMode" size="small" :disabled="taskRunning">
            <el-radio value="upsert">{{ $t('sync.modeUpsert') }}</el-radio>
            <el-radio value="insert">{{ $t('sync.modeInsert') }}</el-radio>
            <el-radio value="truncate_insert">{{ $t('imp.modeOverwrite') }}</el-radio>
          </el-radio-group>
          <p class="settings-hint">{{ $t('sync.truncateHint') }}</p>
        </div>
        <div class="settings-col col-exec">
          <div class="settings-label"><el-icon :size="13"><SetUp /></el-icon>{{ $t('sync.execOptions') }}</div>
          <div class="settings-checks">
            <el-checkbox v-model="opts.autoCreateDb" :disabled="taskRunning">{{ $t('sync.autoCreateDb') }}</el-checkbox>
            <el-checkbox v-model="opts.syncStructure" :disabled="taskRunning">{{ $t('sync.autoCreate') }}</el-checkbox>
            <el-checkbox v-model="opts.syncData" :disabled="taskRunning">{{ $t('sync.syncData') }}</el-checkbox>
            <el-checkbox v-model="opts.includeIndexes" :disabled="taskRunning">{{ $t('sync.includeIndexes') }}</el-checkbox>
            <el-checkbox v-model="opts.stopOnError" :disabled="taskRunning">{{ $t('sync.stopOnError') }}</el-checkbox>
          </div>
          <!-- 「写入批次」输入已删：每种库的甜点批大差一个数量级（CH 落 part / Oracle 有
               单语句上限 / MySQL 看 max_allowed_packet），由后端按目标方言自动决定 -->
          <div class="settings-row">
            <span class="row-label">{{ $t('sync.rowLimit') }}</span>
            <el-input-number v-model="opts.rowLimit" :min="0" :step="1000" size="small"
                             style="width: 110px" :disabled="taskRunning" />
            <span class="row-unit">{{ $t('sync.rowLimitUnit') }}</span>
          </div>
          <div class="settings-row">
            <span class="row-label">{{ $t('sync.whereLabel') }}</span>
            <el-input v-model="opts.whereClause" size="small" :disabled="taskRunning"
                      :placeholder="$t('sync.whereHint')" maxlength="500" />
          </div>
        </div>
      </div>
    </div>

    <!-- ==================== 步骤 4：摘要 ==================== -->
    <div class="summary-card" v-if="step === 4">
      <div class="summary-title">{{ $t('sync.stepSummary') }}</div>
      <el-table :data="summaryRows" size="small" border class="data-table summary-table">
        <el-table-column prop="label" width="150" />
        <el-table-column prop="value" />
      </el-table>
      <div class="summary-opts">
        <span>{{ $t('sync.autoCreate') }}: <b>{{ opts.syncStructure ? $t('sync.yes') : $t('sync.no') }}</b></span>
        <span>{{ $t('sync.syncData') }}: <b>{{ opts.syncData ? $t('sync.yes') : $t('sync.no') }}</b></span>
        <span>{{ $t('sync.dataMode') }}: <b>{{ dataModeLabel }}</b></span>
        <span>{{ $t('sync.existingTarget') }}: <b>{{ opts.objectPolicy === 'drop' ? $t('sync.modeDrop') : $t('sync.modeMerge') }}</b></span>
      </div>
      <el-alert v-if="opts.objectPolicy === 'drop'" :title="$t('sync.dropConfirm')" type="warning" :closable="false" style="margin-top: 10px;" />
    </div>

    <!-- ==================== 步骤 5：执行 ==================== -->
    <div v-if="step === 5" class="run-view">
      <div class="pg-summary">
        <!-- 状态色：失败=红；取消=黄（不是"完成"，但也不该报成错误）；完成=绿 -->
        <el-progress :percentage="progressPct" :stroke-width="14" text-inside
                     :status="taskStatus === 'error' ? 'exception'
                              : (isTaskDone ? 'success' : (taskStatus === 'canceled' ? 'warning' : ''))" />
        <!-- 「正在同步：表 xxx」提示行已按需求去掉 —— 表级阶段全在下面的日志里 -->
      </div>

      <div class="run-stats">
        <!-- Navicat 式：对象数 + 三个按行实时跳动的计数（读取/传输/失败）+ 耗时。
             读取/传输运行中就有（后端按行累加、轮询带回），不等终态 summary ——
             终态后用 summary 的最终值兜底（含 upsert 的 updated 行）。 -->
        <div class="stat"><span class="num">{{ fmtNum(taskTotal) }}</span><span class="label">{{ $t('sync.statTotal') }}</span></div>
        <div class="stat"><span class="num">{{ fmtNum(readShown) }}</span><span class="label">{{ $t('sync.statRead') }}</span></div>
        <div class="stat ok"><span class="num">{{ fmtNum(writtenShown) }}</span><span class="label">{{ $t('sync.statTransferred') }}</span></div>
        <div class="stat danger"><span class="num">{{ fmtNum(failedShown) }}</span><span class="label">{{ $t('sync.statFailCount') }}</span></div>
        <div class="stat"><span class="num">{{ elapsedText }}</span><span class="label">{{ $t('sync.statElapsed') }}</span></div>
      </div>

      <div class="log-card pg-log">
        <div class="log-head">
          <div class="log-head-left">
            <el-icon :size="14"><Document /></el-icon>
            <span>{{ $t('sync.logs') }}</span>
          </div>
          <span class="log-count">{{ $t('mon.nRows', { n: syncLogs.length }) }}</span>
        </div>
        <div ref="logScroll" class="log-body">
          <div v-if="!syncLogs.length" class="log-empty">
            <el-icon :size="28"><Document /></el-icon>
            <span>{{ $t('sync.waitStart') }}</span>
            <p>{{ $t('sync.waitTip') }}</p>
          </div>
          <!-- 每行带时间戳（后端生成），看得出每一步发生在什么时候 -->
          <div v-for="(line, i) in syncLogs" :key="i" class="log-line" :class="logClass(line)">
            <span class="log-time">{{ line.time }}</span>
            <span class="log-text">{{ line.text }}</span>
          </div>
        </div>
      </div>

      <!-- 同步结果（底部汇总条已按需求去掉 —— 结果表格里的状态列就是账本） -->
      <div v-if="taskResult" class="pg-result">
          <el-table :data="taskResult.results" size="small" border max-height="240" class="data-table">
            <el-table-column type="index" label="#" width="45" />
            <el-table-column prop="type" :label="$t('udv.fType')" width="90">
              <template #default="{ row }">{{ typeLabel(row.type) }}</template>
            </el-table-column>
            <el-table-column prop="name" :label="$t('sync.objName')" min-width="150" show-overflow-tooltip />
            <el-table-column prop="status" :label="$t('odv.fStatus')" width="80">
              <template #default="{ row }">
                <el-tag :type="row.status === 'ok' ? 'success' : (row.status === 'skipped' ? 'info' : 'danger')" size="small" effect="dark">
                  {{ row.status === 'ok' ? $t('sync.statOk') : (row.status === 'skipped' ? $t('sync.statSkipped') : $t('sync.statFailed')) }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="$t('sync.colInsertedUpdated')" width="110">
              <template #default="{ row }">
                <span v-if="row.inserted != null" style="color: var(--dc-success);">+{{ row.inserted }}</span>
                <span v-if="row.updated" style="color: var(--dc-warning); margin-left: 4px;">~{{ row.updated }}</span>
              </template>
            </el-table-column>
            <el-table-column prop="message" :label="$t('udv.fComment')" min-width="220" show-overflow-tooltip />
          </el-table>
      </div>
    </div>

    <template #footer>
      <div class="foot-actions">
        <!-- 步骤 1-4：向导导航 -->
        <template v-if="step <= 4">
          <el-button @click="$emit('update:modelValue', false)">{{ $t('common.cancel') }}</el-button>
          <el-button v-if="step > 1" @click="step--">{{ $t('sync.prevStep') }}</el-button>
          <el-button v-if="step < 4" type="primary" @click="nextStep">{{ $t('sync.nextStep') }}</el-button>
          <el-button v-if="step === 4" type="success" :icon="Promotion" @click="run">{{ $t('sync.start') }}</el-button>
        </template>
        <!-- 步骤 5：执行中/完成。运行中可「后台运行」：收起对话框，任务照跑，顶栏任务中心随时找回 -->
        <template v-else>
          <el-button v-if="taskRunning" :icon="Clock" @click="runInBackground">{{ $t('sync.runBg') }}</el-button>
          <el-button v-if="taskRunning" type="danger" plain :icon="VideoPause" :loading="stopping"
                     :disabled="stopping" @click="stopRun">{{ stopping ? $t('cmp.stopping') : $t('sync.stop') }}</el-button>
          <el-button v-else @click="$emit('update:modelValue', false)">{{ $t('common.close') }}</el-button>
        </template>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, reactive, computed, nextTick, onMounted, onBeforeUnmount, watch } from 'vue'
import { ElMessage } from 'element-plus'
import {
  Promotion, Right, Check, VideoPause, Search, FolderOpened, Files, DataLine, Operation, Document,
  Grid, View, FirstAidKit, SetUp, Lightning, Calendar, Clock
} from '@element-plus/icons-vue'
import { listConnections, listDatabases, listSchemas, listTables, listProcedures, listTriggers, listEvents, getFeatures, syncDb, syncTaskStatus, syncCancel } from '../../api'
// 函数 / 存储过程的判据必须与左侧对象树同源（见 treeNodes.js 的 isFunctionRoutine），
// 否则会出现"树上算函数、同步时算存储过程"这种两边对不上的分叉
import { isFunctionRoutine } from '../data/treeNodes'
import { t } from '../../utils/i18n'
import { addBgTask, bgTasks } from './backgroundTasks'

const props = defineProps({ modelValue: Boolean, conn: Object, database: String, resumeTaskId: String })
const emit = defineEmits(['update:modelValue'])

// 向导步骤：1 源/目标 → 2 选项 → 3 对象 → 4 摘要 → 5 执行（Navicat 数据传输式分步）
const step = ref(1)
const connections = ref([])
const src = ref({ connectionId: '', database: '', schema: '' })
const tgt = ref({ connectionId: '', database: '', schema: '' })
const srcDbs = ref([])
const srcSchemas = ref([])
const tgtDbs = ref([])
const tgtSchemas = ref([])
const keyword = ref('')

// ==================== 连接信息摘要 ====================
const connInfoOf = (id) => {
  const c = connections.value.find(x => String(x.id) === String(id))
  if (!c) return null
  return { type: c.type, name: c.name, host: c.host || '--', port: c.port || '--' }
}
const srcInfo = computed(() => connInfoOf(src.value.connectionId))
const tgtInfo = computed(() => connInfoOf(tgt.value.connectionId))
const srcConnName = computed(() => srcInfo.value ? srcInfo.value.name : '--')
const tgtConnName = computed(() => tgtInfo.value ? tgtInfo.value.name : '--')

// ==================== 向导导航 ====================
const nextStep = async () => {
  if (step.value === 1) {
    if (!src.value.connectionId || !src.value.database) return ElMessage.warning(t('sync.pickSourceDb'))
    if (!tgt.value.connectionId || !tgt.value.database) return ElMessage.warning(t('sync.pickTargetDb'))
    if (src.value.connectionId === tgt.value.connectionId && src.value.database === tgt.value.database) {
      return ElMessage.warning(t('sync.sameDb'))
    }
    // 进入步骤 3 前确保对象清单已加载（换连接/换库时 onXxxChange 已自动加载，这里兜底）
    if (!objState.table.list.length && !objState.view.list.length) await loadSrcObjects()
  }
  step.value++
}

// 摘要行（步骤 4 的表格数据）
const dataModeLabel = computed(() => ({
  upsert: t('sync.modeUpsert'),
  insert: t('sync.modeInsert'),
  truncate_insert: t('imp.modeOverwrite'),
}[opts.value.dataMode] || opts.value.dataMode))
const summaryRows = computed(() => [
  { label: t('sync.source'), value: (srcConnName.value || '--') + ' ▸ ' + src.value.database + (src.value.schema ? '.' + src.value.schema : '') },
  { label: t('sync.target'), value: (tgtConnName.value || '--') + ' ▸ ' + tgt.value.database + (tgt.value.schema ? '.' + tgt.value.schema : '') },
  { label: t('sync.pickObjects'), value: String(totalSelected.value) },
  ...(opts.value.stopOnError ? [{ label: t('sync.stopOnError'), value: t('common.yes') }] : []),
  ...(opts.value.rowLimit ? [{ label: t('sync.rowLimit'), value: String(opts.value.rowLimit) }] : []),
  ...(opts.value.whereClause ? [{ label: 'WHERE', value: opts.value.whereClause }] : []),
])

// ==================== 执行统计（Navicat 式：对象数 / 已处理 / 已传输 / 错误 / 耗时） ====================
const runStartedAt = ref(0)
// 耗时显示的心跳：轮询时更新，保证 elapsed 在长时间无进度变化时也会走秒
const tickNow = ref(0)
/** 统计卡数字千分位：大数字（几十万行）没有分隔符根本读不出量级 */
const fmtNum = (n) => (typeof n === 'number' ? n.toLocaleString() : n)
const elapsedText = computed(() => {
  if (!runStartedAt.value) return '--'
  void tickNow.value
  const s = Math.floor((Date.now() - runStartedAt.value) / 1000)
  const h = String(Math.floor(s / 3600)).padStart(2, '0')
  const m = String(Math.floor((s % 3600) / 60)).padStart(2, '0')
  const sec = String(s % 60).padStart(2, '0')
  return `${h}:${m}:${sec}`
})

const srcFeatures = ref({})
const tgtFeatures = ref({})
// ⚠ 后端 features 的键是 **supportsSchemas**（复数）——写成单数永远 undefined，
// SQL Server / PG 的模式下拉就从不出现（真机踩过：SQL Server 选不了模式）
const srcNeedSchema = computed(() => srcFeatures.value.supportsSchemas === true)
const tgtNeedSchema = computed(() => tgtFeatures.value.supportsSchemas === true)

const TYPE_ICON = {
  table: Grid, view: View, function: FirstAidKit,
  procedure: SetUp, trigger: Lightning, event: Calendar
}
// 用函数而不是模块级常量：常量只在加载时求值一次，切换语言后对象类型名不会跟着变
const typeMeta = () => ({
  table:     { label: t('mv.catTable'), abbr: t('sync.abbrTable') },
  view:      { label: t('mv.catView'), abbr: t('sync.abbrView') },
  function:  { label: t('mv.catFunc'), abbr: t('sync.abbrFunction') },
  procedure: { label: t('mv.catProc'), abbr: t('sync.abbrProcedure') },
  trigger:   { label: t('mv.catTrigger'), abbr: t('sync.abbrTrigger') },
  event:     { label: t('mv.catEvent'), abbr: t('sync.abbrEvent') },
})
const objTypes = reactive([
  { key: 'table', checked: false },
  { key: 'view', checked: false },
  { key: 'function', checked: false },
  { key: 'procedure', checked: false },
  { key: 'trigger', checked: false },
  { key: 'event', checked: false },
])
const objState = reactive({})
for (const t of objTypes) objState[t.key] = { list: [], selected: [] }

const opts = ref({ objectPolicy: 'drop', dataMode: 'truncate_insert', syncStructure: true, syncData: true, includeIndexes: true, autoCreateDb: true, stopOnError: false, rowLimit: 0, whereClause: '' })

// 任务状态
let pollTimer = null
const taskId = ref('')
const taskRunning = ref(false)

/**
 * 右上角 X 常驻显示；主弹窗与进度弹窗都在任务运行中时不直接关 ——
 * 关掉只是「看不见」，任务照旧在跑，用户会误以为已经停了。
 */
const onBeforeClose = (done) => {
  if (taskRunning.value) {
    ElMessage.warning(t('sync.busyStop'))
    return
  }
  done()
}
const onProgressBeforeClose = (done) => {
  if (taskRunning.value) {
    ElMessage.warning(t('sync.busyClose'))
    return
  }
  done()
}
const taskStatus = ref('')
const taskDone = ref(0)
const taskTotal = ref(0)
// 行级实时计数（Navicat 式三行数）：后端按行累加、0.8 秒轮询一次 ——
// 界面上数字是平滑跳动递增的，不再「一万一的跳」
const rowsRead = ref(0)
const rowsWritten = ref(0)
const rowsFailed = ref(0)
const taskCurrent = ref('')
// 统计卡显示值：运行中用实时行计数；
// 终态后 task 快照的计数定格，summary 与它一致，直接用实时值即可（兜 0）。
// summary 的 inserted+updated 与 rowsWritten 本就同源，不需要再拼。
//
// **平滑递增**：轮询约 1 秒一次，每次 +1 万的跳变看着像卡死突然蹦一格。
// 这里对三个计数做补间（ease-out，~450ms 滚到新值）—— 轮询间隔内的跳变
// 变成连续滚动，大表传输时数字一直在动，体感是「在跑」而不是「卡了」。
const tweenTo = (shown, target, allowDown = true) => {
  // **运行中永不倒退**：外插是按上次轮询的速率猜的，真实计数有抖动 ——
  // 目标低于当前显示值时说明猜超前了，停在原地等真实值追上来，而不是往回滚
  //（真机踩过：数字越跳越少）。终态（rates 归零）才允许落到最终值。
  const from = shown.value
  const to = allowDown ? target : Math.max(target, from)
  if (to === from) return
  const t0 = performance.now()
  const dur = 450
  const step = (ts) => {
    const p = Math.min(1, (ts - t0) / dur)
    shown.value = Math.round(from + (to - from) * (1 - Math.pow(1 - p, 3)))
    if (p < 1) shown._raf = requestAnimationFrame(step)
  }
  if (shown._raf) cancelAnimationFrame(shown._raf)
  shown._raf = requestAnimationFrame(step)
}
const readShown = ref(0)
const writtenShown = ref(0)
const failedShown = ref(0)
// **速率外插**：轮询 800ms 一次，靠 tween 追的话数字会"顿一下蹦一下"。
// 把最近几次轮询的行速率（EMA 平滑）记下来，两次轮询之间按速率**匀速外插**目标值
//—— 数字每一帧都在走，肉眼看到的是连续增长的流水，而不是间歇跳变。
// 终态速率归零，数字落到快照值定格。
const fetchAt = ref(0)
const prevRows = ref({ read: 0, written: 0 })
const rates = ref({ read: 0, written: 0 })
const onRowsFetched = (read, written, running) => {
  const now = Date.now()
  if (fetchAt.value && running) {
    const dt = (now - fetchAt.value) / 1000
    if (dt > 0.2) {
      const rr = Math.max(0, (read - prevRows.value.read) / dt)
      const wr = Math.max(0, (written - prevRows.value.written) / dt)
      // EMA 平滑：单次抖动不会让数字忽快忽慢
      rates.value = { read: rates.value.read * 0.6 + rr * 0.4, written: rates.value.written * 0.6 + wr * 0.4 }
    }
  }
  if (!running) rates.value = { read: 0, written: 0 }
  prevRows.value = { read, written }
  fetchAt.value = now
}
const runningTick = ref(0)
setInterval(() => { if (taskRunning.value) runningTick.value++ }, 100)
const extrapolate = (base, rate) => {
  runningTick.value
  if (!taskRunning.value || !fetchAt.value) return base
  // 外插打 9 折：宁可显示略慢于真实，也不猜超前导致数字回滚
  return base + rate * 0.9 * Math.max(0, (Date.now() - fetchAt.value) / 1000)
}
const targetRead = computed(() => Math.floor(extrapolate(rowsRead.value, rates.value.read)))
const targetWritten = computed(() => Math.floor(extrapolate(rowsWritten.value, rates.value.written)))
const targetFailed = computed(() => failedBase.value)
const taskMessage = ref('')
const taskResult = ref(null)
const syncLogs = ref([])
const logScroll = ref(null)
const showProgressDlg = ref(false)
// 失败卡的兜底：对象级失败（建表失败 / identity 开关失败等，还没读到行就死了）不计入
// 行失败计数 —— 终态用结果表里的失败对象数托底，失败卡至少亮出来，别再「结果表 3 个
// 失败、失败卡却是 0」的困惑（真机踩过）。
// ⚠ 必须放在 taskResult 声明**之后**：computed 首次求值就访问它，放前面是 TDZ 报错
//（「Cannot access before initialization」，点数据传输整个弹窗打不开 —— 真机踩过）
const failedBase = computed(() => {
  const objFails = (taskResult.value?.results || []).filter(r => r.status === 'error').length
  return Math.max(rowsFailed.value, objFails)
})
watch([targetRead, targetWritten, targetFailed], () => {
// 运行中禁止倒退（外插猜超了就等真实值追上）；终态 rates 归零，允许落定。
// **运行中不逐次 tween**：轮询每 800ms 一次、每次都重启 450ms 补间，速度曲线
// 锯齿化，体感是「一顿一顿蹦」。运行中改由 raf 循环逐帧外插（见下方 liveLoop），
// 数字每一帧都在走；终态才用 tween 平滑落定。
if (taskRunning.value) return
tweenTo(readShown, targetRead.value, true)
tweenTo(writtenShown, targetWritten.value, true)
tweenTo(failedShown, targetFailed.value, true)
})
// **运行中的连续计数**：raf 每帧把显示值推到「真实值 + 速率 × 时间」的外插点上 ——
// 两个轮询之间数字也在匀速增长，用户看到的是一条连续上涨的流水，而不是间歇跳变。
// Math.max 保护：外插打 9 折仍可能瞬时猜超，绝不让数字往回滚（真机踩过）。
let liveRaf = 0
const liveLoop = () => {
if (!taskRunning.value) { liveRaf = 0; return }
readShown.value = Math.max(readShown.value, Math.floor(extrapolate(rowsRead.value, rates.value.read)))
writtenShown.value = Math.max(writtenShown.value, Math.floor(extrapolate(rowsWritten.value, rates.value.written)))
failedShown.value = Math.max(failedShown.value, targetFailed.value)
liveRaf = requestAnimationFrame(liveLoop)
}
watch(taskRunning, (running) => {
if (running) {
if (!liveRaf) liveRaf = requestAnimationFrame(liveLoop)
return
}
// 落定：外插停掉，tween 平滑滚到最终真实值定格
if (liveRaf) { cancelAnimationFrame(liveRaf); liveRaf = 0 }
tweenTo(readShown, targetRead.value, true)
tweenTo(writtenShown, targetWritten.value, true)
tweenTo(failedShown, targetFailed.value, true)
}, { immediate: true })
/** 「停止」请求已发出（按钮据此变成「正在停止…」并禁用，避免连点） */
const stopping = ref(false)
/** 兜底定时器：停止请求发出后若迟迟不达终态，15 秒后放开关闭（见 stopRun） */
let stopGuard = null

const onProgressClosed = () => {
  if (!taskRunning.value) {
    syncLogs.value = []
    taskResult.value = null
  }
}

const logClass = (line) => {
  const t = line.text
  if (t.includes('失败') || t.includes('错误')) return 'err'
  if (t.includes('跳过') || t.includes('已取消')) return 'warn'
  if (t.includes('成功')) return 'ok'
  return ''
}

// ==================== 派生数据 ====================
const totalSelected = computed(() => objTypes.reduce((s, t) => s + objState[t.key].selected.length, 0))
const totalAvailable = computed(() => objTypes.reduce((s, t) => s + (t.checked ? objState[t.key].list.length : 0), 0))
const availableCount = computed(() => objTypes.reduce((s, t) => s + (objState[t.key].list.length ? 1 : 0), 0))
const filteredObjs = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  const arr = []
  for (const t of objTypes) {
    if (!t.checked) continue
    for (const name of objState[t.key].list) {
      if (kw && !name.toLowerCase().includes(kw)) continue
      arr.push({ type: t.key, name })
    }
  }
  return arr
})

const isTaskDone = computed(() => taskStatus.value === 'success')

/**
 * 任务终态有四种，**取消叫 `canceled`**（见后端 `tasks.rs` 的 FinishGuard）：
 * `success` 完成 / `canceled` 已取消 / `error` 失败 / `notfound` 任务过期（只保留 30 分钟）。
 *
 * 这里踩过一次坑：以前只认 success 与 error，于是点了「停止同步」之后，
 * 后端其实几秒内就收尾了（实测取消后 t+3s 就是 canceled），界面却永远停在"同步中" ——
 * `taskRunning` 不落回 false，进度对话框就一直被"请先点停止再关闭"挡着，**关不掉**。
 * 凡是终态都必须放行 —— 这是"关得掉"的唯一保证。
 */
const TERMINAL_STATUS = ['success', 'canceled', 'error', 'notfound']
/** 任务已经不再运行（成功 / 已取消 / 失败 / 过期）—— 界面据此放行「关闭」 */
const isTaskSettled = computed(() => TERMINAL_STATUS.includes(taskStatus.value))

/** 进度条百分比：完成后直接给 100（各对象粒度不同，停在 done/total 的原始值上会像"卡住"） */
const progressPct = computed(() => {
  if (isTaskDone.value) return 100
  // 取消时停在"实际写到哪"，不假装 100%
  if (!taskTotal.value || taskTotal.value < 0) return 0
  return Math.min(100, Math.round(taskDone.value / taskTotal.value * 100))
})

/**
 * 进度文案。前缀「正在同步：」与计数「（已完成/总数）」**只在界面这一处拼**。
 *
 * 后端那条 `phase`（接口里以 `current` 返回）曾经自己又写了一遍「同步 X（1/5）」，
 * 于是界面渲染成「正在同步：同步 X（1/5）（1/5）」—— 动词和计数各重复一次。
 * 现在后端只说对象名，这里统一负责措辞。
 *
 * `total < 0` 是内核约定的「总量未知」，不能拼成「（0/-1）」。
 */
const progressText = computed(() => {
  if (taskStatus.value === 'error') return taskMessage.value
  if (isTaskDone.value) return t('sync.done')
  // 取消：说清楚"停在哪"，也说明已写入的不回滚（否则很像白跑了一趟）
  if (taskStatus.value === 'canceled') {
    const done = taskTotal.value > 0 ? t('sync.doneCount', { done: taskDone.value, total: taskTotal.value }) : ''
    return t('sync.stopped') + done
  }
  if (taskStatus.value === 'notfound') return taskMessage.value || t('sync.taskExpired')
  const current = taskCurrent.value
  const counts = taskTotal.value > 0 ? t('sync.progressParen', { done: taskDone.value, total: taskTotal.value }) : ''
  return (current ? t('sync.runningWith', { current }) : t('sync.runningShort')) + counts
})

const typeLabel = (t) => (typeMeta()[t]?.label) || t
const typeAbbr = (t) => (typeMeta()[t]?.abbr) || t

// ==================== 类型 / 对象操作 ====================
const toggleType = (t) => {
  t.checked = !t.checked
  if (!t.checked) objState[t.key].selected = []
}
const isSelected = (o) => objState[o.type].selected.includes(o.name)
const toggleObj = (o, v) => {
  const sel = objState[o.type].selected
  const i = sel.indexOf(o.name)
  if (v && i < 0) sel.push(o.name)
  if (!v && i >= 0) sel.splice(i, 1)
}
const applyAll = (val) => {
  for (const t of objTypes) {
    if (!t.checked) continue
    objState[t.key].selected = val ? [...objState[t.key].list] : []
  }
}

// ==================== 对象加载 ====================
const loadConnections = async () => {
  connections.value = await listConnections()
  // 只有从外部带着明确上下文进来（右键某连接传 props.conn）才预填；
  // 顶栏直接打开时**连连接也不预选**（用户要求）——库、连接全部「请选择」，
  // 让用户从头自己挑，避免默认就是上次随手选的东西
  if (props.conn) {
    src.value.connectionId = props.conn.id
    tgt.value.connectionId = props.conn.id
  }
  if (src.value.connectionId) await loadSrcDbs()
  if (tgt.value.connectionId) await loadTgtDbs()
  // 预填库必须在清单里才保留（清单是 internal 裸名；树曾给过 `internal.ods`
  // 这类全限定名 —— 不在清单就丢掉，别让无效值带进后续请求）
  if (props.conn && srcDbs.value.includes(props.database || '')) {
    src.value.database = props.database
    await onSrcDbChange()
  }
}

const onSrcConnChange = async () => {
  src.value.database = ''
  src.value.schema = ''
  srcDbs.value = []
  srcSchemas.value = []
  srcFeatures.value = {}
  await loadSrcDbs()
}
const loadSrcDbs = async () => {
  if (!src.value.connectionId) { srcDbs.value = []; return }
  srcDbs.value = await listDatabases(src.value.connectionId)
  // **默认不预选**（用户要求）：库留空由用户自己选 —— 系统库（INFORMATION_SCHEMA）
  // 排第一时自动选中它，反而掩盖了真实业务库
  try { srcFeatures.value = await getFeatures(src.value.connectionId) } catch (e) { srcFeatures.value = {} }
  await onSrcDbChange()
}
const onSrcDbChange = async () => {
  src.value.schema = ''
  srcSchemas.value = []
  if (srcNeedSchema.value) {
    try { srcSchemas.value = await listSchemas(src.value.connectionId, src.value.database) } catch (e) { srcSchemas.value = [] }
    if (srcSchemas.value.length) src.value.schema = srcSchemas.value[0]
  }
  await loadSrcObjects()
}

const onTgtConnChange = async () => {
  tgt.value.database = ''
  tgt.value.schema = ''
  tgtDbs.value = []
  tgtSchemas.value = []
  tgtFeatures.value = {}
  await loadTgtDbs()
}
const loadTgtDbs = async () => {
  if (!tgt.value.connectionId) { tgtDbs.value = []; return }
  tgtDbs.value = await listDatabases(tgt.value.connectionId)
  // 同源：默认不预选
  try { tgtFeatures.value = await getFeatures(tgt.value.connectionId) } catch (e) { tgtFeatures.value = {} }
  await onTgtDbChange()
}
const onTgtDbChange = async () => {
  tgt.value.schema = ''
  tgtSchemas.value = []
  if (tgtNeedSchema.value) {
    try { tgtSchemas.value = await listSchemas(tgt.value.connectionId, tgt.value.database) } catch (e) { tgtSchemas.value = [] }
    if (tgtSchemas.value.length) tgt.value.schema = tgtSchemas.value[0]
  }
}

const loadSrcObjects = async () => {
  const connId = src.value.connectionId
  // 支持 schema 的数据库（SQL Server / PG 等）把选中的模式拼成 "库.模式"，后端据此按模式加载对象
  const refDb = srcNeedSchema.value && src.value.schema
    ? src.value.database + '.' + src.value.schema
    : src.value.database
  const db = refDb
  for (const t of objTypes) {
    objState[t.key].list = []
    objState[t.key].selected = []
  }
  keyword.value = ''
  if (!connId || !db) return
  try {
    const tables = await listTables(connId, db)
    objState.table.list = tables.filter(t => (t.type || 'TABLE') === 'TABLE').map(t => t.name)
    objState.view.list = tables.filter(t => t.type === 'VIEW').map(t => t.name)
  } catch (e) { /* ignore */ }
  try {
    const procs = await listProcedures(connId, db)
    // 判据用 isFunctionRoutine（唯一的定义处，与树上分类、数量同源）。
    //
    // 原来这里判的是 `p.comment || p.type` 里有没有 "FUNCTION" —— 而这条接口
    // **根本不返回**这两个字段（后端给的是 routineType），所以条件恒为假：
    // 结果是**所有函数都被塞进了「存储过程」一栏**，函数那一栏永远是空的。
    // 与树上"函数混在 Procedures 里"是同一个病，一起治。
    const funcs = procs.filter(isFunctionRoutine)
    const procsOnly = procs.filter(p => !isFunctionRoutine(p))
    objState.function.list = funcs.map(p => p.name)
    objState.procedure.list = procsOnly.map(p => p.name)
  } catch (e) { /* ignore */ }
  try {
    objState.trigger.list = (await listTriggers(connId, db)).map(p => p.name)
  } catch (e) { /* ignore */ }
  try {
    objState.event.list = (await listEvents(connId, db)).map(p => p.name)
  } catch (e) { /* ignore */ }
}

// ==================== 同步执行 ====================
const run = async () => {
  if (!src.value.connectionId || !src.value.database) return ElMessage.warning(t('sync.pickSourceDb'))
  if (!tgt.value.connectionId || !tgt.value.database) return ElMessage.warning(t('sync.pickTargetDb'))
  if (totalSelected.value === 0) return ElMessage.warning(t('sync.pickObjectsWarn'))
  if (src.value.connectionId === tgt.value.connectionId && src.value.database === tgt.value.database) {
    return ElMessage.warning(t('sync.sameDb'))
  }

  const payload = {
    sourceConnectionId: src.value.connectionId,
    sourceDatabase: src.value.database,
    sourceSchema: src.value.schema,
    targetConnectionId: tgt.value.connectionId,
    targetDatabase: tgt.value.database,
    targetSchema: tgt.value.schema,
    objectPolicy: opts.value.objectPolicy,
    dataMode: opts.value.dataMode,
    syncStructure: opts.value.syncStructure,
    syncData: opts.value.syncData,
    includeIndexes: opts.value.includeIndexes,
    autoCreateDb: opts.value.autoCreateDb,
    stopOnError: opts.value.stopOnError,
    rowLimit: opts.value.rowLimit || undefined,
    whereClause: opts.value.whereClause || undefined,
    tables: objState.table.selected,
    views: objState.view.selected,
    functions: objState.function.selected,
    procedures: objState.procedure.selected,
    triggers: objState.trigger.selected,
    events: objState.event.selected
  }
  taskResult.value = null
  taskMessage.value = ''
  syncLogs.value = []
  // 新任务是自己提交、用户正盯着进度页 —— 终态提示照常弹（区别于任务中心「查看」恢复）
  resumedView.value = false
  // 进入执行步骤（向导第 5 步，不再弹二级进度框）
  step.value = 5
  runStartedAt.value = Date.now()
  try {
    const { taskId: tid } = await syncDb(payload)
    taskId.value = tid
    // **提交即登记**任务中心：不点「后台运行」的任务也要出现在执行记录里
    //（用户口径：执行成功的历史记录要在任务中心能看到），之后的状态刷新/快照留底统一走任务中心
    addBgTask({ id: tid, kind: 'sync', title: bgTitle.value })
    taskRunning.value = true
    taskStatus.value = 'running'
    taskDone.value = 0
    taskTotal.value = totalSelected.value
    rowsRead.value = 0
    rowsWritten.value = 0
    rowsFailed.value = 0
    taskCurrent.value = t('cmp.preparing')
    pollTimer = setInterval(pollTask, 800)
    } catch (e) {
      ElMessage.error(t('sync.startFailed', { detail: e.message }))
      step.value = 4
    }
    }

    // ===== 后台运行 / 恢复 =====
    // 「后台运行」：任务在后端继续跑（后端任务本来独立于对话框），这里只是收起窗口
    // 并把 taskId 登记到任务中心；用户随时从顶栏任务中心点回来继续看进度。
    const runInBackground = () => {
    if (!taskId.value) return
    // kind 必须显式给：靠 addBgTask 里 prev?.kind 兜底时，同 id 复用/旧记录
    // 可能被错路由到 CompareDialog（审查发现）
    addBgTask({ id: taskId.value, kind: 'sync', title: bgTitle.value })
    // 与 CompareDialog 口径一致：后台运行即停轮询，状态刷新交给任务中心 ——
    // 不停的话对话框关了轮询还在 800ms 打接口，终态还会在主界面弹 toast
    stopPolling()
    // 「停止」的 15 秒兜底守卫一并解除：到期会把还在跑的任务当卡死强行收摊
    if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
    stopping.value = false
    ElMessage.success(t('sync.bgAdded'))
    emit('update:modelValue', false)
    }
    // 恢复标题（任务中心与后台登记共用）：「源库 → 目标库」一眼能对上是哪一趟
    const bgTitle = computed(() =>
      t('sync.bgTitle', { src: src.value.database || '--', tgt: tgt.value.database || '--' })
    )
    // 任务中心点「查看」→ 父组件带 resumeTaskId 打开对话框：跳过向导直接进进度页接着轮询。
    // ⚠ 下面那个「打开即重置向导」的 watch（step=1）与本 watch 同帧触发，注册序在前先跑 ——
    // 这里必须等 nextTick 之后再跳到 5，否则刚设置的进度页被重置回第一步（真机踩过）。
    watch([() => props.resumeTaskId, () => props.modelValue], async ([id, open]) => {
      if (!id || !open) return
      await nextTick()
      if (id !== props.resumeTaskId) return
      taskId.value = id
      step.value = 5
      taskRunning.value = true
      taskStatus.value = 'running'
      taskResult.value = null
      syncLogs.value = []
      // **从任务中心「查看」进来**：用户就是冲着结果界面来的 ——
      // 终态 toast（「同步完成…」）不再弹，结果已经在眼前（真机反馈）
      resumedView.value = true
      // 恢复时复位停止态与统计卡计数：上一次「停止」残留的 stopping 会让停止按钮
      // 假禁用（一直「正在停止…」）；不清 shown 计数则恢复瞬间先闪现上次的旧数字
      if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
      stopping.value = false
      readShown.value = 0
      writtenShown.value = 0
      failedShown.value = 0
      // 耗时卡先给个占位 —— pollTask 拿到任务快照的 elapsedMs 后按真实开始时间续算，
      // 从任务中心/悬浮恢复进来不能从 00:00 重新数（真机反馈）
      runStartedAt.value = Date.now()
      await pollTask()
      if (taskRunning.value) pollTimer = setInterval(pollTask, 800)
    })
    // 任务终态**不再从任务中心摘掉**：执行记录由用户自己删（用户口径：不删就保留），
    // 摘掉会让「本地留底」失效 —— 删除入口在任务中心每一行上
    /** 终态提示去重：同一任务同一次运行只弹一次完成/失败提示 ——
     *  恢复（resume）与收起前后的多次轮询都可能到达终态，不挡会反复弹（真机踩过） */
    const notifiedKey = ref('')
    /** **恢复查看标记**：任务中心「查看」进来的轮询不弹终态 toast ——
     *  用户就是冲着结果界面来的；新提交的任务（run 里复位）照常弹 */
    const resumedView = ref(false)
    watch(taskStatus, (s) => {
      if (!TERMINAL_STATUS.includes(s)) return
      const key = taskId.value + ':' + s
      if (notifiedKey.value === key) return
      notifiedKey.value = key
    })

const nowTime = () => {
  // **带日期时间（含年份）**：执行记录要能跨年回溯
  const d = new Date()
  const p2 = (n) => String(n).padStart(2, '0')
  const hm = `${p2(d.getHours())}:${p2(d.getMinutes())}:${p2(d.getSeconds())}`
  return `${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())} ${hm}`
}

const mergeLogs = (serverLogs) => {
  if (!Array.isArray(serverLogs)) return
  const existing = new Set(syncLogs.value.map(l => l.text))
  let added = 0
  for (const text of serverLogs) {
    if (!existing.has(text)) {
      syncLogs.value.push({ time: nowTime(), text })
      added++
    }
  }
  const scrollEl = logScroll.value
  if (added && scrollEl) {
    setTimeout(() => { scrollEl.scrollTop = scrollEl.scrollHeight }, 50)
  }
}

const pollTask = async () => {
  tickNow.value = Date.now()
  try {
    // 局部变量改名：本函数后面要用 t() 翻译，同名会把它遮蔽掉
    const st = await syncTaskStatus(taskId.value)
    taskStatus.value = st.status
    taskDone.value = st.done || 0
    taskTotal.value = st.total || taskTotal.value
    rowsRead.value = st.rowsRead || 0
    rowsWritten.value = st.rowsWritten || 0
    rowsFailed.value = st.rowsFailed || 0
    // 采样行速率（供数字外插，见 rates/onRowsFetched）
    onRowsFetched(st.rowsRead || 0, st.rowsWritten || 0, taskRunning.value)
    // 接口把阶段文案放在 `current`（= 任务的 phase）；`phase` 兜底，免得字段名一变界面就空白
    taskCurrent.value = st.current || st.phase || ''
    // **耗时续算**：任务快照带 elapsedMs（后端任务的真实运行时长，终态后后端已冻结该值）——
    // 从任务中心/悬浮恢复进来时（包括已完成的任务），耗时卡按真实开始时间显示，不归零重数
    if (st.elapsedMs > 0) runStartedAt.value = Date.now() - st.elapsedMs
    mergeLogs(st.logs)
    // 只有**终态**才收摊。这里必须包含 canceled：取消后后端给的就是 canceled，
    // 漏掉它界面就永远停在"同步中"，对话框再也关不掉（见 TERMINAL_STATUS 的说明）。
    if (isTaskSettled.value) {
      stopPolling()
      if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
      taskRunning.value = false
      stopping.value = false
      // 终态提示**只弹一次**：恢复 + 多次轮询都可能撞上终态，弹过就不再弹
      const nkey = taskId.value + ':' + st.status
      const alreadyNotified = notifiedKey.value === nkey
      notifiedKey.value = nkey
      // 取消也把已写入的逐对象结果展示出来 —— 哪些表插了多少行是有用的信息，不该因为"取消"就丢掉
      taskResult.value = st.result || taskResult.value
      if (st.status === 'success') {
        // 恢复查看（任务中心进来）不弹：结果已经在界面上，toast 纯属打扰
        if (!alreadyNotified && !resumedView.value) ElMessage.success(st.result?.message || t('sync.done'))
      } else if (st.status === 'canceled') {
        if (!alreadyNotified) ElMessage.info(st.result?.message || st.message || t('sync.stoppedKeep'))
      } else if (st.status === 'notfound') {
        // 后端任务不在了（服务重启 / 超过内存保留期）：**本地快照兜底** ——
        // 任务中心的记录里存过终态结果就照常展示，别让用户看到一句"只保留 30 分钟"
        //（用户口径：记录只要不删就保留）；本地也没有才提示已中断
        const rec = bgTasks.find(x => x.id === taskId.value)
        const snap = rec && rec.snapshot
        if (snap && snap.status && snap.status !== 'running') {
          rowsRead.value = snap.rowsRead || 0
          rowsWritten.value = snap.rowsWritten || 0
          rowsFailed.value = snap.rowsFailed || 0
          taskResult.value = snap.result || taskResult.value
          taskMessage.value = snap.message || ''
          taskStatus.value = snap.status
          ElMessage.info(t('sync.bgRestoredLocal'))
        } else {
          taskMessage.value = t('sync.bgInterrupted')
          ElMessage.warning(taskMessage.value)
        }
      } else {
        taskMessage.value = st.message || t('sync.failed')
        ElMessage.error(taskMessage.value)
      }
    }
  } catch (e) {
    stopPolling()
    if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
    taskRunning.value = false
    stopping.value = false
    ElMessage.error(t('cmp.statusFailed', { detail: e.message }))
  }
}

/**
 * 请求停止。
 *
 * 两点：
 * 1. **立刻置 `stopping`**：按钮当场变成「正在停止…」并禁用 —— 否则连点几下会发好几个取消请求，
 *    界面上却毫无反应（"点了没动静"说的就是这个）。
 * 2. **加一道兜底**：取消是协作式的（后端写完当前那一**页**才停），正常几秒内就会到达 canceled；
 *    万一某个长事务迟迟不回来，15 秒后也放开关闭 —— 否则就是一个"关不掉"的死窗口。
 */
const stopRun = async () => {
  if (stopping.value) return
  stopping.value = true
  try {
    await syncCancel(taskId.value)
    ElMessage.info(t('sync.stopRequested'))
    if (stopGuard) clearTimeout(stopGuard)
    stopGuard = setTimeout(() => {
      if (taskRunning.value) {
        stopPolling()
        taskRunning.value = false
        stopping.value = false
        ElMessage.warning(t('cmp.closingAllowed'))
      }
    }, 15000)
  } catch (e) {
    stopping.value = false
    ElMessage.error(t('sync.stopFailed', { detail: e.message }))
  }
}

const stopPolling = () => {
  if (pollTimer) { clearInterval(pollTimer); pollTimer = null }
}

onBeforeUnmount(() => {
  stopPolling()
  // 兜底定时器一起清掉：组件都没了，它再去改状态只会报"操作已卸载的组件"
  if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
})
/**
 * **打开即全新**：清掉上一次任务的全部运行痕迹（进度/统计卡/日志/结果/轮询）——
 * 用户口径：点「数据传输」开的就是**新传输**，不带上次跑完的缓存数字；
 * 任务中心「查看」走 resumeTaskId 恢复链路，在本重置之后（nextTick）重新拉起进度页。
 */
const resetRunState = () => {
  stopPolling()
  if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
  if (liveRaf) { cancelAnimationFrame(liveRaf); liveRaf = 0 }
  stopping.value = false
  step.value = 1
  runStartedAt.value = 0
  taskId.value = ''
  taskRunning.value = false
  taskStatus.value = ''
  taskDone.value = 0
  taskTotal.value = 0
  rowsRead.value = 0
  rowsWritten.value = 0
  rowsFailed.value = 0
  taskCurrent.value = ''
  taskMessage.value = ''
  taskResult.value = null
  syncLogs.value = []
  resumedView.value = false
  showProgressDlg.value = false
  readShown.value = 0
  writtenShown.value = 0
  failedShown.value = 0
  rates.value = { read: 0, written: 0 }
  prevRows.value = { read: 0, written: 0 }
  fetchAt.value = 0
}
watch(() => props.modelValue, v => {
  if (!v) return
  // 每次打开都重置成全新向导（上一次任务的数字/日志/结果一概不留）；
  // 之前「任务还在跑就切回进度页」的分支已删 —— 回看进度走任务中心/时钟图标
  resetRunState()
  loadConnections()
})
onMounted(() => { if (props.modelValue) loadConnections() })
</script>

<style scoped>
/* ==================== 对话框整体限高：内容多时内部滚动，不再把框撑得很高 ==================== */
.sync-dialog :deep(.el-dialog__body) {
  max-height: 74vh;
  overflow-y: auto;
}
/* 执行页的日志与结果表各自限高滚动，避免叠加撑高 */
.run-view .log-body { max-height: 240px; }
.run-view .pg-result :deep(.el-table) { max-height: 260px; }

/* ==================== 连接信息摘要（步骤 1） ==================== */
.info-grid-wrap { display: flex; gap: 14px; margin-top: 14px; }
.info-col {
  flex: 1; min-width: 0;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border); border-radius: 10px;
  padding: 12px 14px;
}
.info-title { font-size: 13px; font-weight: 600; color: var(--dc-primary); margin-bottom: 8px; }
.info-row { display: flex; justify-content: space-between; padding: 3px 0; font-size: 13px; }
.info-row span { color: var(--dc-text-dim); }
.info-row b { color: var(--dc-text); font-weight: 500; }

/* ==================== 步骤 4 摘要 ==================== */
.summary-card {
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border); border-radius: 10px;
  padding: 14px;
}
.summary-title { font-size: 13px; font-weight: 600; color: var(--dc-primary); margin-bottom: 10px; }
.summary-table { margin-bottom: 12px; }
.summary-opts { display: flex; flex-wrap: wrap; gap: 8px 20px; font-size: 13px; color: var(--dc-text-dim); }
.summary-opts b { color: var(--dc-text); font-weight: 500; }

/* ==================== 步骤 5 执行 ==================== */
.run-stats {
  display: flex; gap: 10px; margin: 12px 0;
}
.run-stats .stat {
  flex: 1; text-align: center;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border); border-radius: 10px;
  padding: 10px 8px;
}
.run-stats .num { display: block; font-size: 18px; font-weight: 600; color: var(--dc-text); }
.run-stats .label { font-size: 12px; color: var(--dc-text-dim); }
.run-stats .stat.ok .num { color: var(--dc-success); }
.run-stats .stat.danger .num { color: var(--dc-danger); }

.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}
.dlg-title-sub { font-size: 13px; font-weight: 400; color: var(--dc-text-dim); margin-left: 4px; }

/* ==================== 源 / 目标 流 ==================== */
/* 步骤条（向导式交互，与数据对比一致）—— 仅向导阶段显示，执行页（步骤 5）隐藏 */
.sync-steps { margin-bottom: 16px; }
.sync-steps :deep(.el-step__title) { font-size: 12px; }
.flow { display: flex; align-items: stretch; gap: 12px; margin-bottom: 14px; }
.side {
  flex: 1; min-width: 0;
  display: flex; flex-direction: column; gap: 8px;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  border-radius: 12px;
  padding: 12px 14px;
}
.side-head {
  display: flex; align-items: center; gap: 7px;
  font-size: 13px; font-weight: 600; color: var(--dc-text);
  letter-spacing: .3px;
}
.dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
.dot.src { background: linear-gradient(135deg, var(--dc-primary-light), var(--dc-primary-deep)); box-shadow: 0 0 8px var(--dc-primary-glow); }
.dot.tgt { background: linear-gradient(135deg, var(--dc-accent), var(--dc-primary-light)); box-shadow: 0 0 8px rgba(61,220,151,.4); }
.pair { display: flex; gap: 8px; }
.pair .el-select { flex: 1; min-width: 0; }
.flow-arrow { display: flex; align-items: center; }
.arrow-ring {
  width: 40px; height: 40px; border-radius: 50%;
  display: flex; align-items: center; justify-content: center;
  color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-accent), var(--dc-primary-light));
  box-shadow: 0 0 0 4px rgba(61, 220, 151, .12), 0 4px 14px rgba(61, 220, 151, .25);
}

/* ==================== 通用面板 ==================== */
.panel, .opts, .progress-card, .result-card {
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  border-radius: 12px;
  padding: 12px 14px;
  margin-bottom: 14px;
}
.panel-head {
  display: flex; align-items: center; justify-content: space-between;
  margin-bottom: 10px;
}
.panel-title {
  display: inline-flex; align-items: center; gap: 6px;
  font-size: 14px; font-weight: 600; color: var(--dc-text);
}
.panel-title .el-icon { color: var(--dc-primary); }
.panel-count { font-size: 13px; color: var(--dc-text-dim); }
.panel-count b { color: var(--dc-primary); font-weight: 700; }

/* 类型 chips */
.type-chips {
  display: flex; flex-wrap: wrap; gap: 10px;
  margin-bottom: 12px;
}
.type-chip {
  position: relative;
  display: inline-flex; align-items: center; gap: 6px;
  padding: 6px 14px 6px 10px;
  border: 1px solid var(--dc-border);
  border-radius: 999px;
  background: var(--dc-bg-card);
  color: var(--dc-text-dim);
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: all .18s ease;
  font-family: inherit;
}
.type-chip:hover:not(:disabled):not(.active) {
  color: var(--dc-text);
  border-color: var(--dc-primary);
  background: var(--dc-bg-hover);
}
.type-chip.active {
  color: var(--dc-on-primary);
  border-color: transparent;
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 3px 12px var(--dc-primary-glow);
}
.type-chip.empty { opacity: .4; cursor: not-allowed; }
.type-chip:disabled { cursor: not-allowed; }
.chip-ic {
  width: 22px; height: 22px; border-radius: 6px;
  display: inline-flex; align-items: center; justify-content: center;
  background: var(--dc-bg-soft); color: var(--dc-text-weak);
  transition: all .18s ease;
}
.type-chip:hover:not(:disabled):not(.active) .chip-ic {
  background: var(--dc-bg-hover); color: var(--dc-text-dim);
}
.type-chip.active .chip-ic {
  background: rgba(255,255,255,.18); color: var(--dc-on-primary);
}
.chip-name { white-space: nowrap; }
.chip-num {
  font-size: 13px; font-weight: 700;
  background: var(--dc-bg-hover);
  border-radius: 999px; padding: 2px 8px;
  min-width: 22px; text-align: center;
  color: var(--dc-text);
}
.type-chip:not(.active) .chip-num { background: var(--dc-bg-hover); color: var(--dc-text); }
.type-chip.active .chip-num { background: rgba(255, 255, 255, .25); color: var(--dc-on-primary); }
.chip-check { position: absolute; top: -4px; right: -3px; width: 16px; height: 16px; border-radius: 50%;
  background: var(--dc-on-primary); color: var(--dc-primary); display: flex; align-items: center; justify-content: center;
  box-shadow: var(--dc-shadow-sm); }
.no-obj-hint { font-size: 13px; color: var(--dc-text-dim); align-self: center; }

/* 对象工具条 + 列表 */
.obj-bar { display: flex; align-items: center; gap: 10px; margin-bottom: 8px; }
.obj-bar .el-input { width: 240px; }
.obj-bar-actions { margin-left: auto; display: flex; }
.obj-scroll {
  max-height: 208px; overflow: auto;
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-card);
  padding: 4px 6px;
}
.obj-item {
  display: flex; align-items: center; gap: 8px;
  padding: 2px 8px 2px 4px;
  border-radius: 6px;
  transition: background .15s;
}
.obj-item:hover { background: var(--dc-bg-hover); }
.obj-type {
  flex: none;
  width: 22px; height: 18px; line-height: 18px;
  text-align: center;
  font-size: 12px; font-weight: 600;
  border-radius: 5px;
  color: #0f1522;
  opacity: .9;
}
.ty-table { background: var(--dc-accent); }
.ty-view { background: #7cabff; }
.ty-function { background: #c4b0ff; }
.ty-procedure { background: #f8c878; }
.ty-trigger { background: #ff9d9d; }
.ty-event { background: #7fe3f5; }
.obj-item .el-checkbox { margin-right: 0; }
.obj-name {
  font-size: 13px; color: var(--dc-text);
  font-family: "Cascadia Code", Consolas, "JetBrains Mono", monospace;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}

/* ==================== 同步设置 ==================== */
.settings-card {
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: 14px;
  padding: 0;
  margin-bottom: 14px;
  box-shadow: var(--dc-shadow-sm);
  overflow: hidden;
}
.settings-head {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 18px;
  font-size: 14px; font-weight: 700; color: var(--dc-text);
  border-bottom: 1px solid var(--dc-border);
  background: linear-gradient(90deg, var(--dc-primary-wash), var(--dc-success-wash));
}
.settings-head-left {
  display: inline-flex; align-items: center; gap: 8px;
}
.settings-head-left .el-icon {
  color: var(--dc-primary);
  filter: drop-shadow(0 0 4px var(--dc-primary-glow));
}
.settings-body {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
  padding: 14px 16px;
}
.settings-col {
  min-width: 0;
  display: flex; flex-direction: column; gap: 8px;
  padding: 14px 16px;
  border-radius: 12px;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  position: relative;
  overflow: hidden;
}
.settings-col::before {
  content: '';
  position: absolute; left: 0; top: 0; bottom: 0; width: 4px;
}
.col-policy::before { background: linear-gradient(180deg, var(--dc-primary), var(--dc-primary-deep)); }
.col-mode::before   { background: linear-gradient(180deg, var(--dc-accent), var(--dc-primary-light)); }
.col-exec::before   { background: linear-gradient(180deg, var(--dc-warning), var(--dc-danger)); }
.settings-label {
  display: inline-flex; align-items: center; gap: 6px;
  font-size: 14px; font-weight: 700;
  color: var(--dc-text);
  margin-bottom: 2px;
}
.settings-label .el-icon { color: var(--dc-accent); }
.settings-col :deep(.el-radio-group),
.settings-col :deep(.el-checkbox-group) { display: flex; flex-direction: column; align-items: flex-start; gap: 6px; }
.settings-col :deep(.el-radio),
.settings-col :deep(.el-checkbox) { margin-right: 0; height: auto; }
.settings-col :deep(.el-radio__label),
.settings-col :deep(.el-checkbox__label) { font-size: 13px; color: var(--dc-text); }
.settings-hint {
  font-size: 11.5px; line-height: 1.5;
  color: var(--dc-text-weak);
  margin: 0;
}
.settings-checks { display: flex; flex-direction: column; gap: 6px; }
/* 高级选项里的「行数上限」行 + WHERE 输入 */
.settings-row { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--dc-text-dim); margin-top: 6px; }
.settings-row .row-label { flex-shrink: 0; }
.settings-row .row-unit { font-size: 12px; color: var(--dc-text-weak); }
.settings-row .el-input { flex: 1; }

/* ==================== 实时日志 ==================== */
.log-card {
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: 14px;
  padding: 0;
  margin-bottom: 14px;
  box-shadow: var(--dc-shadow-sm);
  overflow: hidden;
}
.log-head {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 18px;
  font-size: 14px; font-weight: 700; color: var(--dc-text);
  border-bottom: 1px solid var(--dc-border);
  background: linear-gradient(90deg, var(--dc-primary-wash), var(--dc-success-wash));
}
.log-head-left {
  display: inline-flex; align-items: center; gap: 8px;
}
.log-head-left .el-icon {
  color: var(--dc-primary);
  filter: drop-shadow(0 0 4px var(--dc-primary-glow));
}
.log-count {
  font-size: 12px; font-weight: 600;
  color: var(--dc-text-dim);
  background: var(--dc-bg-soft);
  padding: 3px 10px; border-radius: 999px;
  border: 1px solid var(--dc-border);
}
.log-body {
  max-height: 200px; overflow-y: auto;
  padding: 10px 18px;
  background: var(--dc-bg-code);
}
.log-empty {
  display: flex; flex-direction: column; align-items: center; gap: 8px;
  padding: 28px 0;
  color: var(--dc-text-weak);
}
.log-empty .el-icon { color: var(--dc-text-weak); }
.log-empty span { font-size: 14px; font-weight: 600; color: var(--dc-text-weak); }
.log-empty p { font-size: 11.5px; color: var(--dc-text-weak); margin: 0; }
.log-line {
  display: flex; align-items: flex-start; gap: 10px;
  padding: 3px 0;
  font-size: 13px; line-height: 1.6;
  color: var(--dc-code-text);
  font-family: "Cascadia Code", Consolas, "JetBrains Mono", monospace;
}
.log-time {
  flex: none;
  color: var(--dc-text-weak);
  font-size: 12px;
}
.log-line.ok { color: var(--dc-success); }
.log-line.warn { color: var(--dc-warning); }
.log-line.err { color: var(--dc-danger); }

/* ==================== 进度 / 结果 ==================== */
.progress-card { padding: 14px 16px; }
.progress-text { font-size: 13px; color: var(--dc-text-dim); margin-top: 8px; text-align: center; }
.result-card { padding: 14px 16px; }

/* ==================== 进度弹窗 ==================== */
.progress-dialog :deep(.el-dialog__body) { padding: 16px 20px 10px; max-height: 74vh; overflow: auto; }
/* 进度窗整体限高：结果表格 + 25 条日志会把弹窗顶到超出小屏（真机反馈） */
.run-view .log-body { max-height: 200px; }
.pg-summary { margin-bottom: 14px; }
.pg-text { font-size: 13px; color: var(--dc-text-dim); margin-top: 8px; text-align: center; }
.pg-log { margin-bottom: 0; }
.pg-log .log-body { max-height: 320px; }
.pg-result { margin-top: 14px; }
.stat-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(96px, 1fr));
  gap: 8px;
  margin-bottom: 12px;
}
.stat {
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  border-radius: 10px;
  padding: 10px;
  text-align: center;
}
.stat .num { display: block; font-size: 20px; font-weight: 700; color: var(--dc-primary); }
.stat .label { font-size: 12px; color: var(--dc-text-dim); }
.stat.ok .num { color: var(--dc-success); }
.stat.danger .num { color: var(--dc-danger); }

.foot-actions {
  display: flex; align-items: center; justify-content: flex-end;
  gap: 10px; width: 100%;
}
.data-table { width: 100%; }

/* ==================== Element Plus 适配 ==================== */
.sync-dialog :deep(.el-dialog__body) { padding-top: 4px; }
.sync-dialog :deep(.el-dialog__footer) { display: flex; align-items: center; justify-content: flex-end; border-top: 1px solid var(--dc-border); padding-top: 14px; padding-bottom: 4px; gap: 12px; }
.sync-dialog :deep(.el-dialog__footer) > div { display: flex; align-items: center; gap: 10px; }
.sync-dialog :deep(.el-radio__label),
.sync-dialog :deep(.el-checkbox__label) { font-size: 13px; white-space: nowrap; }
</style>
