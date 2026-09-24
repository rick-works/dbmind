<template>
  <el-dialog :model-value="modelValue" width="980px" top="4vh" class="sync-dialog"
             :close-on-click-modal="false" :close-on-press-escape="!taskRunning"
             :before-close="onBeforeClose"
             append-to-body @update:model-value="$emit('update:modelValue', $event)">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Promotion /></el-icon></span>
        <span>{{ $t('sync.title') }}</span>
        <span class="dlg-title-sub">{{ $t('sync.subtitle') }}</span>
      </div>
    </template>

    <!-- ==================== 源 → 目标 ==================== -->
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

    <!-- ==================== 对象选择 ==================== -->
    <div class="panel" v-if="src.connectionId && src.database">
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

    <!-- ==================== 同步设置 ==================== -->
    <div class="settings-card">
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
            <el-checkbox v-model="opts.syncStructure" :disabled="taskRunning">{{ $t('sync.autoCreate') }}</el-checkbox>
            <el-checkbox v-model="opts.syncData" :disabled="taskRunning">{{ $t('sync.syncData') }}</el-checkbox>
          </div>
          <div class="settings-batch">
            <span>{{ $t('sync.batchSize') }}</span>
            <el-input-number v-model="opts.batchSize" :min="100" :max="10000" :step="100" size="small" style="width: 110px" :disabled="taskRunning" />
          </div>
        </div>
      </div>
    </div>

    <!-- ==================== 进度 + 日志 弹窗 ==================== -->
    <el-dialog v-model="showProgressDlg" width="980px" top="6vh"
               :close-on-click-modal="false" :close-on-press-escape="false"
               :before-close="onProgressBeforeClose"
               append-to-body class="progress-dialog" @closed="onProgressClosed">
      <template #header>
        <div class="dlg-title">
          <span class="dlg-title-ic"><el-icon :size="16"><Document /></el-icon></span>
          <span>{{ $t('sync.progress') }}</span>
        </div>
      </template>
      <div class="pg-summary">
        <!-- 状态色：失败=红；取消=黄（不是"完成"，但也不该报成错误）；完成=绿 -->
        <el-progress :percentage="progressPct" :stroke-width="14" text-inside
                     :status="taskStatus === 'error' ? 'exception'
                              : (isTaskDone ? 'success' : (taskStatus === 'canceled' ? 'warning' : ''))" />
        <div class="pg-text">{{ progressText }}</div>
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
          <div v-for="(line, i) in syncLogs" :key="i" class="log-line" :class="logClass(line)">
            <span class="log-time">{{ line.time }}</span>
            <span class="log-text">{{ line.text }}</span>
          </div>
        </div>
      </div>

      <!-- 同步结果 -->
      <div v-if="taskResult" class="pg-result">
        <el-alert v-if="!taskResult.success" :title="taskResult.message" type="error" show-icon :closable="false" />
        <template v-else>
          <div class="stat-grid">
            <div class="stat ok"><span class="num">{{ taskResult.summary.ok }}</span><span class="label">{{ $t('sync.statOk') }}</span></div>
            <div class="stat"><span class="num">{{ taskResult.summary.skipped }}</span><span class="label">{{ $t('sync.statSkipped') }}</span></div>
            <div class="stat danger"><span class="num">{{ taskResult.summary.errors }}</span><span class="label">{{ $t('sync.statFailed') }}</span></div>
            <div class="stat"><span class="num">{{ taskResult.summary.inserted }}</span><span class="label">{{ $t('sync.statInserted') }}</span></div>
            <div class="stat"><span class="num">{{ taskResult.summary.updated }}</span><span class="label">{{ $t('sync.statUpdated') }}</span></div>
            <div class="stat"><span class="num">{{ taskResult.summary.total }}</span><span class="label">{{ $t('sync.statTotal') }}</span></div>
          </div>
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
        </template>
      </div>

      <template #footer>
        <el-button v-if="taskRunning" type="danger" plain :icon="VideoPause" :loading="stopping"
                   :disabled="stopping" @click="stopRun">{{ stopping ? $t('cmp.stopping') : $t('sync.stop') }}</el-button>
        <el-button v-else @click="showProgressDlg = false">{{ $t('common.close') }}</el-button>
      </template>
    </el-dialog>

    <template #footer>
      <div class="foot-actions">
        <el-button @click="$emit('update:modelValue', false)" :disabled="taskRunning">{{ $t('common.close') }}</el-button>
        <el-button v-if="taskRunning" type="danger" plain :icon="VideoPause" :loading="stopping"
                   :disabled="stopping" @click="stopRun">{{ stopping ? $t('cmp.stopping') : $t('sync.stop') }}</el-button>
        <el-button type="success" :icon="Promotion" :loading="taskRunning" :disabled="totalSelected === 0" @click="run">
          {{ taskRunning ? $t('sync.running') : $t('sync.start') }}
        </el-button>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, reactive, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import { ElMessage } from 'element-plus'
import {
  Promotion, Right, Check, VideoPause, Search, FolderOpened, Files, DataLine, Operation, Document,
  Grid, View, FirstAidKit, SetUp, Lightning, Calendar
} from '@element-plus/icons-vue'
import { listConnections, listDatabases, listSchemas, listTables, listProcedures, listTriggers, listEvents, getFeatures, syncDb, syncTaskStatus, syncCancel } from '../../api'
// 函数 / 存储过程的判据必须与左侧对象树同源（见 treeNodes.js 的 isFunctionRoutine），
// 否则会出现"树上算函数、同步时算存储过程"这种两边对不上的分叉
import { isFunctionRoutine } from '../data/treeNodes'
import { t } from '../../utils/i18n'

const props = defineProps({ modelValue: Boolean, conn: Object, database: String })
const emit = defineEmits(['update:modelValue'])

const connections = ref([])
const src = ref({ connectionId: '', database: '', schema: '' })
const tgt = ref({ connectionId: '', database: '', schema: '' })
const srcDbs = ref([])
const srcSchemas = ref([])
const tgtDbs = ref([])
const tgtSchemas = ref([])
const keyword = ref('')

const srcFeatures = ref({})
const tgtFeatures = ref({})
const srcNeedSchema = computed(() => srcFeatures.value.supportsSchema === true)
const tgtNeedSchema = computed(() => tgtFeatures.value.supportsSchema === true)

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

const opts = ref({ objectPolicy: 'merge', dataMode: 'upsert', syncStructure: true, syncData: true, batchSize: 5000 })

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
const taskCurrent = ref('')
const taskMessage = ref('')
const taskResult = ref(null)
const syncLogs = ref([])
const logScroll = ref(null)
const showProgressDlg = ref(false)
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
  if (props.conn) {
    src.value.connectionId = props.conn.id
    tgt.value.connectionId = props.conn.id
    src.value.database = props.database || ''
  }
  if (!src.value.connectionId && connections.value.length) {
    src.value.connectionId = connections.value[0].id
    tgt.value.connectionId = connections.value[0].id
  }
  if (src.value.connectionId) await loadSrcDbs()
  if (tgt.value.connectionId) await loadTgtDbs()
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
  if (!srcDbs.value.includes(src.value.database)) src.value.database = srcDbs.value[0] || ''
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
  if (!tgtDbs.value.includes(tgt.value.database)) tgt.value.database = tgtDbs.value[0] || ''
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
    batchSize: opts.value.batchSize,
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
  showProgressDlg.value = true
  try {
    const { taskId: tid } = await syncDb(payload)
    taskId.value = tid
    taskRunning.value = true
    taskStatus.value = 'running'
    taskDone.value = 0
    taskTotal.value = totalSelected.value
    taskCurrent.value = t('cmp.preparing')
    pollTimer = setInterval(pollTask, 800)
  } catch (e) {
    ElMessage.error(t('sync.startFailed', { detail: e.message }))
    showProgressDlg.value = false
  }
}

const nowTime = () => {
  const d = new Date()
  return `${String(d.getHours()).padStart(2,'0')}:${String(d.getMinutes()).padStart(2,'0')}:${String(d.getSeconds()).padStart(2,'0')}`
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
  try {
    // 局部变量改名：本函数后面要用 t() 翻译，同名会把它遮蔽掉
    const st = await syncTaskStatus(taskId.value)
    taskStatus.value = st.status
    taskDone.value = st.done || 0
    taskTotal.value = st.total || taskTotal.value
    // 接口把阶段文案放在 `current`（= 任务的 phase）；`phase` 兜底，免得字段名一变界面就空白
    taskCurrent.value = st.current || st.phase || ''
    mergeLogs(st.logs)
    // 只有**终态**才收摊。这里必须包含 canceled：取消后后端给的就是 canceled，
    // 漏掉它界面就永远停在"同步中"，对话框再也关不掉（见 TERMINAL_STATUS 的说明）。
    if (isTaskSettled.value) {
      stopPolling()
      if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
      taskRunning.value = false
      stopping.value = false
      // 取消也把已写入的逐对象结果展示出来 —— 哪些表插了多少行是有用的信息，不该因为"取消"就丢掉
      taskResult.value = st.result || taskResult.value
      if (st.status === 'success') {
        ElMessage.success(st.result?.message || t('sync.done'))
      } else if (st.status === 'canceled') {
        ElMessage.info(st.result?.message || st.message || t('sync.stoppedKeep'))
      } else if (st.status === 'notfound') {
        ElMessage.warning(st.message || t('cmp.taskExpiredLong'))
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
watch(() => props.modelValue, v => { if (v) loadConnections() })
onMounted(() => { if (props.modelValue) loadConnections() })
</script>

<style scoped>
.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}
.dlg-title-sub { font-size: 13px; font-weight: 400; color: var(--dc-text-dim); margin-left: 4px; }

/* ==================== 源 / 目标 流 ==================== */
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
.settings-batch { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--dc-text-dim); margin-top: 2px; }
.settings-batch :deep(.el-input-number) { --el-input-number-controls-width: 0; }

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
.progress-dialog :deep(.el-dialog__body) { padding: 16px 20px 10px; }
.pg-summary { margin-bottom: 14px; }
.pg-text { font-size: 13px; color: var(--dc-text-dim); margin-top: 8px; text-align: center; }
.pg-log { margin-bottom: 0; }
.pg-log .log-body { max-height: 420px; }
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
