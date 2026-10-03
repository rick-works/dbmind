<template>
  <!-- X 常驻显示，运行中由 onBeforeClose 拦截提示（隐藏 X 会让人找不到关闭入口） -->
  <el-dialog v-model="visible" width="1000px" top="4vh" :close-on-click-modal="false"
             :close-on-press-escape="!running" :before-close="onBeforeClose"
             append-to-body class="main-dialog datagen-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><MagicStick /></el-icon></span>
        <span>{{ $t('dgen.title') }}</span>
        <span v-if="target" class="dlg-target">{{ target.db }}<span class="dot">.</span>{{ target.table }}</span>
      </div>
    </template>

    <!-- 向导步骤：设置规则 → 预览数据 → 写入执行 -->
    <el-steps :active="stepIndex" align-center finish-status="success" class="gen-steps">
      <el-step v-for="s in STEP_TITLES" :key="s" :title="s" />
    </el-steps>

    <div v-loading="loadingColumns" class="gen-body">
      <!-- ==================== 第 1 步：生成设置 ==================== -->
      <template v-if="step === 'rules'">
        <div class="step-subtitle">{{ $t('dgen.stepRules') }}</div>
        <div class="step-card">
          <el-form label-width="72px" label-position="left" size="small" class="datagen-form">
            <div class="form-row">
              <el-form-item :label="$t('dgen.genRows')">
                <el-input-number v-model="rows" :min="1" :step="100" :controls="false" class="fillable" />
              </el-form-item>
              <el-form-item :label="$t('dgen.writeMode')">
                <el-radio-group v-model="writeMode">
                  <el-radio-button value="append">{{ $t('dgen.append') }}</el-radio-button>
                  <el-radio-button value="clear">{{ $t('dgen.clearFirst') }}</el-radio-button>
                </el-radio-group>
              </el-form-item>
              <el-form-item :label="$t('dgen.batchSize')">
                <el-input-number v-model="batchSize" :min="1" :step="100" :controls="false" class="fillable" />
              </el-form-item>
              <el-form-item :label="$t('dgen.seed')">
                <el-input v-model="seed" :placeholder="$t('dgen.seedPlaceholder')" clearable class="fillable" />
              </el-form-item>
            </div>
          </el-form>
        </div>

        <div class="section-head">
          <span class="section-title">{{ $t('dgen.fieldRules') }}</span>
          <span class="section-tip">{{ $t('dgen.fieldRulesTip') }}</span>
          <el-button size="small" type="primary" plain :icon="Refresh" @click="resetRules">{{ $t('dgen.allAutoByType') }}</el-button>
        </div>

        <el-table :data="colRules" size="small" border class="rule-table" :max-height="ruleTableMaxH">
          <el-table-column :label="$t('dgen.colField')" min-width="150">
            <template #default="{ row }">
              <div class="cell-name">
                <span class="cname">{{ row.name }}</span>
                <el-tag v-if="row.primaryKey" size="small" type="warning" effect="plain">{{ $t('dgen.pk') }}</el-tag>
                <el-tag v-if="row.autoIncrement" size="small" type="info" effect="plain">{{ $t('dgen.autoInc') }}</el-tag>
              </div>
              <div v-if="row.comment" class="cell-comment">{{ row.comment }}</div>
            </template>
          </el-table-column>
          <el-table-column :label="$t('dgen.colType')" width="130">
            <template #default="{ row }">
              <span class="ctype">{{ row.type }}</span>
            </template>
          </el-table-column>
          <el-table-column :label="$t('dgen.colConstraint')" width="70">
            <template #default="{ row }">
              <el-tag size="small" :type="row.nullable ? 'success' : 'danger'" effect="plain">{{ row.nullable ? $t('dgen.nullable') : $t('dgen.notNull') }}</el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="$t('dgen.colRule')" width="170">
            <template #default="{ row }">
              <el-select v-model="row.rule" size="small" filterable>
                <el-option-group v-for="g in ruleGroups" :key="g.label" :label="g.label">
                  <el-option v-for="o in g.options" :key="o.value" :label="o.label" :value="o.value" />
                </el-option-group>
              </el-select>
            </template>
          </el-table-column>
          <el-table-column :label="$t('dgen.colParam')" min-width="240">
            <template #default="{ row }">
              <!-- 固定值 / SQL 表达式 -->
              <el-input v-if="paramKind(row.rule) === 'value'" v-model="row.value" size="small"
                        :placeholder="row.rule === 'sql' ? $t('dgen.phSqlExpr') : $t('dgen.phFixedValue')" />
              <!-- 编码前缀 -->
              <el-input v-else-if="paramKind(row.rule) === 'prefix'" v-model="row.value" size="small"
                        :placeholder="$t('dgen.phCodePrefix')" />
              <!-- 列表 -->
              <el-input v-else-if="paramKind(row.rule) === 'values'" v-model="row.valuesText" size="small"
                        :placeholder="$t('dgen.phListValues')" />
              <!-- 数值区间 -->
              <div v-else-if="paramKind(row.rule) === 'range'" class="param-range">
                <el-input v-model="row.min" size="small" :placeholder="$t('dgen.phMin')" />
                <span class="range-sep">~</span>
                <el-input v-model="row.max" size="small" :placeholder="$t('dgen.phMax')" />
              </div>
              <!-- 日期区间 -->
              <div v-else-if="paramKind(row.rule) === 'dateRange'" class="param-range">
                <el-date-picker v-model="row.min" type="date" size="small" value-format="YYYY-MM-DD" :placeholder="$t('dgen.phStartDate')" style="width:100%" />
                <span class="range-sep">~</span>
                <el-date-picker v-model="row.max" type="date" size="small" value-format="YYYY-MM-DD" :placeholder="$t('dgen.phEndDate')" style="width:100%" />
              </div>
              <!-- 随机字符串长度 -->
              <el-input-number v-else-if="paramKind(row.rule) === 'length'" v-model="row.length" :min="1" :max="4000"
                               size="small" controls-position="right" :placeholder="$t('dgen.phLength')" style="width:120px" />
              <!-- 序列起始 -->
              <el-input-number v-else-if="paramKind(row.rule) === 'start'" v-model="row.start" :min="0"
                               size="small" controls-position="right" :placeholder="$t('dgen.phStart')" style="width:120px" />
              <span v-else class="param-none">—</span>
            </template>
          </el-table-column>
          <el-table-column :label="$t('dgen.colNullRatio')" width="96" align="center">
            <template #default="{ row }">
              <!-- 原生纯文本输入框：type=text 不产生任何原生/组件自带的加减按钮 -->
              <input
                class="ratio-input"
                type="text"
                inputmode="numeric"
                maxlength="3"
                :value="row.nullRatio"
                :disabled="!row.nullable || row.primaryKey"
                @input="(e) => { const clean = onNullRatioInput(row, e.target.value); if (e.target.value !== clean) e.target.value = clean }"
                @blur="onNullRatioBlur(row)"
              />
            </template>
          </el-table-column>
        </el-table>
      </template>

      <!-- ==================== 第 2 步：数据预览 ==================== -->
      <template v-else-if="step === 'preview'">
        <div class="section-head">
          <span class="section-title">{{ $t('dgen.previewResult') }}</span>
          <span class="section-tip">{{ $t('dgen.previewTip', { n: previewRows.length }) }}</span>
        </div>
        <el-table v-loading="previewing" :data="previewRows" size="small" border class="preview-table" :max-height="previewTableMaxH">
          <el-table-column v-for="c in previewColumns" :key="c" :prop="c" :label="c" min-width="130" show-overflow-tooltip />
          <template #empty>
            <span class="empty-tip">{{ $t('dgen.previewEmpty') }}</span>
          </template>
        </el-table>
        <div v-if="notes.length" class="notes">
          <div v-for="(n, i) in notes" :key="i" class="note-line">{{ n }}</div>
        </div>
      </template>

      <!-- ==================== 第 3 步：写入执行（进度 + 实时日志） ==================== -->
      <template v-else>
        <div class="step-subtitle">{{ $t('dgen.runConfirm') }}</div>
        <div class="summary-card">
          <div class="summary-row">
            <span class="summary-label">{{ $t('dgen.summaryTable') }}</span>
            <span class="summary-value mono">{{ target?.db }}.{{ target?.table }}</span>
          </div>
          <div class="summary-row">
            <span class="summary-label">{{ $t('dgen.genRows') }}</span>
            <span class="summary-value">{{ $t('dgen.summaryRows', { rows, batch: batchSize }) }}</span>
          </div>
          <div class="summary-row">
            <span class="summary-label">{{ $t('dgen.writeMode') }}</span>
            <span class="summary-value">
              <el-tag size="small" :type="writeMode === 'clear' ? 'warning' : 'info'" effect="plain">
                {{ writeMode === 'clear' ? $t('dgen.clearFirst') : $t('dgen.append') }}
              </el-tag>
              <span v-if="writeMode === 'clear'" class="warn-text">{{ $t('dgen.clearWarning') }}</span>
            </span>
          </div>
          <div class="summary-row">
            <span class="summary-label">{{ $t('dgen.seed') }}</span>
            <span class="summary-value">{{ seed.trim() || $t('dgen.seedNotSet') }}</span>
          </div>
          <div class="summary-row">
            <span class="summary-label">{{ $t('dgen.fieldRules') }}</span>
            <span class="summary-value">{{ $t('dgen.fieldRulesSummary', { n: colRules.length, adjusted: adjustCount }) }}</span>
          </div>
        </div>

        <div class="section-head">
          <span class="section-title">{{ $t('dgen.progress') }}</span>
          <span class="section-tip">{{ job ? (job.stage || $t('dgen.generating')) : $t('dgen.notStarted') }}</span>
          <span v-if="job" class="gp-count">{{ $t('dgen.progressCount', { done: job.done || 0, total: job.total || rows }) }}</span>
        </div>
        <el-progress :percentage="jobPercent" :stroke-width="14" :text-inside="true" :status="jobStatus" />

        <div class="section-head">
          <span class="section-title">{{ $t('dgen.liveLog') }}</span>
          <span class="section-tip">{{ running ? $t('dgen.cancelHint') : '' }}</span>
        </div>
        <div ref="logBoxRef" class="log-box">
          <div v-for="(l, i) in logs" :key="i" class="log-line" :class="'lv-' + l.level">
            <span class="log-time">[{{ l.time }}]</span>
            <span class="log-text">{{ l.text }}</span>
          </div>
          <div v-if="!logs.length" class="log-empty">{{ $t('dgen.waiting') }}</div>
        </div>

        <div v-if="error" class="error-box">{{ error }}</div>

        <div v-if="notices.length" class="notes">
          <div v-for="(n, i) in notices" :key="i" class="note-line">{{ n }}</div>
        </div>
      </template>
    </div>

    <template #footer>
      <div class="datagen-footer">
        <span class="footer-tip">{{ footerTip }}</span>
        <div class="footer-btns">
          <!-- 第 1 步：设置 -->
          <template v-if="step === 'rules'">
            <el-button size="small" @click="visible = false">{{ $t('common.cancel') }}</el-button>
            <el-button size="small" type="primary" :disabled="loadingColumns || !colRules.length" @click="goPreview">
              {{ $t('dgen.nextPreview') }}
            </el-button>
          </template>
          <!-- 第 2 步：预览 -->
          <template v-else-if="step === 'preview'">
            <el-button size="small" @click="backToRules">{{ $t('dgen.prev') }}</el-button>
            <el-button size="small" :loading="previewing" @click="doPreview">{{ $t('dgen.previewAgain') }}</el-button>
            <el-button size="small" type="primary" @click="step = 'run'">{{ $t('dgen.nextRun') }}</el-button>
          </template>
          <!-- 第 3 步：执行 -->
          <template v-else>
            <el-button v-if="running" size="small" type="danger" :icon="CircleClose" :loading="canceling" @click="doCancel">{{ $t('dgen.cancelGen') }}</el-button>
            <template v-else>
              <el-button size="small" @click="backToRules">{{ $t('dgen.backToEdit') }}</el-button>
              <el-button v-if="!job" size="small" type="primary" :icon="MagicStick" @click="doGenerate">{{ $t('dgen.startGen') }}</el-button>
              <el-button v-else size="small" type="primary" @click="visible = false">{{ $t('dgen.done') }}</el-button>
            </template>
          </template>
        </div>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
// 数据生成弹窗（向导式）：① 生成设置与字段规则 ② 预览取值效果 ③ 确认并写入，实时展示进度与日志。
// 写入走后台任务（可查进度、可取消），取消会在数据库端回滚本次未提交的数据；预览不写库。
import { ref, computed, watch, nextTick, onMounted, onBeforeUnmount } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { MagicStick, Refresh, CircleClose } from '@element-plus/icons-vue'
import { listColumns, dataGenPreview, dataGenStart, dataGenTask, dataGenCancel } from '../../api'
import { errMsg } from '../../utils/errMsg'
import { t } from '../../utils/i18n'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  conn: { type: Object, default: null },
  target: { type: Object, default: null }
})
const emit = defineEmits(['update:modelValue', 'done'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

// ==================== 向导步骤 ====================

// ⚠️ 步骤名必须是 computed，不能是模块常量：常量只在模块加载那一刻求值一次，
// 用户切换语言后它**不会跟着变**（而且不报错，只是"界面上一半是英文、步骤条还是中文"）。
const STEP_TITLES = computed(() => [t('dgen.stepRules'), t('dgen.stepPreview'), t('dgen.stepRun')])
const STEP_IDS = ['rules', 'preview', 'run']
const step = ref('rules')
const stepIndex = computed(() => {
  const idx = Math.max(0, STEP_IDS.indexOf(step.value))
  // 写入执行成功完成后，把步骤索引推到末尾，让最后一步也显示为完成状态（绿色）
  if (step.value === 'run' && !running.value && job.value?.status === 'success') return STEP_IDS.length
  return idx
})

// 生成规则（与后端 TestDataGenerator.Rule 一一对应）
//
// ⚠️ 表里只存 labelKey，文案由下面的 computed 现取：
// 直接把 t(...) 写进这张模块级常量，语言会在**加载时**被定死 —— 切成英文后这个下拉仍是中文。
// 另外 UUID 是产品名（哪种语言都这么写），保留字面量、不进字典。
const RULE_GROUPS = [
  { labelKey: 'dgen.grpAuto', options: [
    { value: 'auto', labelKey: 'dgen.rAuto' },
    { value: 'skip', labelKey: 'dgen.rSkip' },
    { value: 'null', labelKey: 'dgen.rNull' }
  ] },
  { labelKey: 'dgen.grpNumber', options: [
    { value: 'sequence', labelKey: 'dgen.rSequence' },
    { value: 'random_int', labelKey: 'dgen.rRandomInt' },
    { value: 'random_decimal', labelKey: 'dgen.rRandomDecimal' },
    { value: 'bool', labelKey: 'dgen.rBool' }
  ] },
  { labelKey: 'dgen.grpText', options: [
    { value: 'random_string', labelKey: 'dgen.rRandomString' },
    { value: 'fixed', labelKey: 'dgen.rFixed' },
    { value: 'list', labelKey: 'dgen.rList' },
    { value: 'cycle', labelKey: 'dgen.rCycle' },
    { value: 'code', labelKey: 'dgen.rCode' },
    { value: 'uuid' }
  ] },
  { labelKey: 'dgen.grpSemantic', options: [
    { value: 'chinese_name', labelKey: 'dgen.rChineseName' },
    { value: 'username', labelKey: 'dgen.rUsername' },
    { value: 'email', labelKey: 'dgen.rEmail' },
    { value: 'phone', labelKey: 'dgen.rPhone' },
    { value: 'company', labelKey: 'dgen.rCompany' },
    { value: 'city', labelKey: 'dgen.rCity' },
    { value: 'address', labelKey: 'dgen.rAddress' },
    { value: 'word', labelKey: 'dgen.rWord' },
    { value: 'title', labelKey: 'dgen.rTitle' },
    { value: 'ip', labelKey: 'dgen.rIp' },
    { value: 'url', labelKey: 'dgen.rUrl' }
  ] },
  { labelKey: 'dgen.grpTime', options: [
    { value: 'random_date', labelKey: 'dgen.rRandomDate' },
    { value: 'now', labelKey: 'dgen.rNow' }
  ] },
  { labelKey: 'dgen.grpAdvanced', options: [
    { value: 'sql', labelKey: 'dgen.rSql' }
  ] }
]
const ruleGroups = computed(() => RULE_GROUPS.map(g => ({
  label: t(g.labelKey),
  options: g.options.map(o => (o.labelKey ? { value: o.value, label: t(o.labelKey) } : { value: o.value, label: o.value.toUpperCase() }))
})))

const rows = ref(100)
const writeMode = ref('append')
const batchSize = ref(500)
const seed = ref('')
const loadingColumns = ref(false)
const previewing = ref(false)
const colRules = ref([])
const previewColumns = ref([])
const previewRows = ref([])
const notes = ref([])
// 写入执行阶段后端返回的提示：与预览页的 notes 分开，避免切换步骤时互相覆盖
const notices = ref([])
// 生成任务快照（后端异步任务轮询结果）：{ taskId, status, done, total, percent, stage, message, ... }
const job = ref(null)
const canceling = ref(false)
const error = ref('')
const logs = ref([])
const logBoxRef = ref(null)
let pollTimer = null
// 日志去重/降噪：仅在阶段变化或进度跨过 10% 时补一条
let lastLogStage = ''
let lastLogBucket = -1

const running = computed(() => !!job.value && job.value.status === 'running')
// 有手动调整过参数的字段数量（用于确认页展示）
const adjustCount = computed(() => colRules.value.filter(c => {
  if (c.autoIncrement) return c.rule !== 'skip'
  return c.rule !== 'auto' || c.value || c.valuesText || c.min || c.max || c.length !== null || c.start !== null || c.nullRatio
}).length)

const jobPercent = computed(() => {
  const j = job.value
  if (!j) return 0
  if (typeof j.percent === 'number' && j.percent >= 0) return Math.max(0, Math.min(100, j.percent))
  return j.total ? Math.round(((j.done || 0) * 100) / j.total) : 0
})
const jobStatus = computed(() => {
  const s = job.value?.status
  if (s === 'success') return 'success'
  if (s === 'error') return 'exception'
  if (s === 'canceled') return 'warning'
  return ''
})

// ==================== 弹窗内表格可视高度 ====================
// 弹窗高度固定（≤88vh），字段多时由表格自己滚动（表头固定），不再把弹窗越撑越高
const winH = ref(window.innerHeight)
const onWinResize = () => { winH.value = window.innerHeight }
onMounted(() => window.addEventListener('resize', onWinResize))
// 「字段规则」表上方还有步骤条、生成设置卡片与标题，预留空间比预览表更多
const ruleTableMaxH = computed(() => Math.max(260, Math.round(winH.value * 0.88) - 400))
const previewTableMaxH = computed(() => Math.max(220, Math.round(winH.value * 0.88) - 330))

const footerTip = computed(() => {
  if (step.value === 'rules') return t('dgen.tipRules')
  if (step.value === 'preview') return t('dgen.tipPreview')
  if (running.value) return t('dgen.tipRunning')
  if (job.value?.status === 'success') return t('dgen.tipSuccess')
  return writeMode.value === 'clear' ? t('dgen.tipClear') : t('dgen.tipStart')
})

// 每种规则需要填的参数类型（决定「参数」列渲染哪个控件）
const paramKind = (rule) => {
  if (rule === 'fixed' || rule === 'sql') return 'value'
  if (rule === 'code') return 'prefix'
  if (rule === 'list' || rule === 'cycle') return 'values'
  if (rule === 'random_int' || rule === 'random_decimal') return 'range'
  if (rule === 'random_date') return 'dateRange'
  if (rule === 'random_string') return 'length'
  if (rule === 'sequence') return 'start'
  return 'none'
}

const resetState = () => {
  stopPoll()
  step.value = 'rules'
  job.value = null
  canceling.value = false
  error.value = ''
  logs.value = []
  lastLogStage = ''
  lastLogBucket = -1
  rows.value = 100
  writeMode.value = 'append'
  batchSize.value = 500
  seed.value = ''
  colRules.value = []
  previewColumns.value = []
  previewRows.value = []
  notes.value = []
  notices.value = []
}

// ==================== 实时日志 ====================

const scrollLog = () => {
  nextTick(() => {
    const el = logBoxRef.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

const pushLog = (text, level = 'info') => {
  if (!text) return
  const last = logs.value[logs.value.length - 1]
  if (last && last.text === text && last.level === level) return
  logs.value.push({ time: new Date().toLocaleTimeString('zh-CN', { hour12: false }), text, level })
  if (logs.value.length > 500) logs.value.splice(0, logs.value.length - 500)
  scrollLog()
}

// ==================== 生成进度轮询 / 取消 ====================

const stopPoll = () => {
  if (pollTimer) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

// 任务结束时统一收尾：成功刷新树并提示，失败/取消给出明确原因
const finishJob = (res) => {
  canceling.value = false
  if (res.status === 'success') {
    pushLog(res.message || t('dgen.logWritten', { n: res.done || 0 }), 'success')
    if (res.executeTime) pushLog(t('dgen.logDbMs', { ms: res.executeTime }), 'info')
    notices.value = res.notices || []
    ElMessage.success(res.message || t('dgen.genDone'))
    emit('done')
  } else if (res.status === 'canceled') {
    pushLog(res.message || t('dgen.canceled'), 'warn')
    ElMessage.info(res.message || t('dgen.canceled'))
  } else {
    error.value = res.error || res.message || t('common.unknownError')
    pushLog(t('dgen.genFailed', { detail: error.value }), 'error')
    ElMessage.error(t('dgen.genFailed', { detail: error.value }))
  }
}

const pollJob = async () => {
  const id = job.value?.taskId
  if (!id) return stopPoll()
  try {
    const res = await dataGenTask(id)
    if (!res || !res.success) {
      // 任务不存在/已过期：结束轮询，避免无谓请求
      if (res && res.status === 'notfound') {
        stopPoll()
        canceling.value = false
        ElMessage.warning(res.message || t('dgen.taskGone'))
      }
      return
    }
    job.value = res
    if (res.status === 'running') {
      if (res.stage && res.stage !== lastLogStage) {
        lastLogStage = res.stage
        pushLog(res.stage)
      }
      const p = typeof res.percent === 'number' ? res.percent : 0
      const bucket = Math.floor(p / 10) * 10
      if (bucket > lastLogBucket) {
        lastLogBucket = bucket
        pushLog(t('dgen.logProgress', { done: res.done || 0, total: res.total || 0, p }))
      }
    } else {
      stopPoll()
      finishJob(res)
    }
  } catch {
    // 单次轮询失败不打断整体流程，下一次继续
  }
}

const startPoll = () => {
  stopPoll()
  pollTimer = setInterval(pollJob, 600)
  pollJob()
}

// 取消生成：后端中止批次写入并回滚本次未提交的数据
const doCancel = async () => {
  const id = job.value?.taskId
  if (!id || !running.value) return
  canceling.value = true
  try {
    const res = await dataGenCancel(id)
    if (res && res.success) {
      job.value = { ...job.value, stage: t('dgen.canceling') }
      pushLog(t('dgen.logCancelRequested'), 'warn')
      pollJob()
    } else {
      canceling.value = false
      ElMessage.warning(res?.message || t('dgen.cancelFailed'))
    }
  } catch (e) {
    canceling.value = false
    ElMessage.error(t('dgen.cancelFailedDetail', { detail: errMsg(e) }))
  }
}

// 生成中禁止关闭弹窗（先取消或等待完成），避免用户误以为任务已停止
const onBeforeClose = (done) => {
  if (running.value) {
    ElMessage.warning(t('dgen.busy'))
    return
  }
  done()
}

onBeforeUnmount(() => {
  stopPoll()
  window.removeEventListener('resize', onWinResize)
})

// ==================== 字段加载 / 规则重置 ====================

const loadColumns = async () => {
  loadingColumns.value = true
  try {
    const list = await listColumns(props.conn?.id, props.target?.db, props.target?.table)
    if (!list || !list.length) {
      ElMessage.warning(t('dgen.noColumns'))
      return
    }
    colRules.value = list.map(c => ({
      name: c.name,
      type: c.type,
      nullable: !!c.nullable,
      primaryKey: !!c.primaryKey,
      autoIncrement: !!c.autoIncrement,
      comment: c.comment || '',
      // 自增列默认跳过（交给数据库生成），其余列按类型自动推导
      rule: c.autoIncrement ? 'skip' : 'auto',
      value: '',
      valuesText: '',
      min: '',
      max: '',
      length: null,
      start: null,
      nullRatio: 0
    }))
  } catch (e) {
    ElMessage.error(t('dgen.loadColsFailed', { detail: errMsg(e) }))
  } finally {
    loadingColumns.value = false
  }
}

const resetRules = () => {
  colRules.value.forEach(c => {
    c.rule = c.autoIncrement ? 'skip' : 'auto'
    c.value = ''
    c.valuesText = ''
    c.min = ''
    c.max = ''
    c.length = null
    c.start = null
    c.nullRatio = 0
  })
  ElMessage.info(t('dgen.rulesReset'))
}

// 空值%：仅接受 0-100 的整数，输入过程只保留数字，失焦时收敛到合法范围
// 返回清洗后的字符串，供输入框在存在非法字符时同步展示
const onNullRatioInput = (row, v) => {
  const digits = String(v ?? '').replace(/\D/g, '').slice(0, 3)
  row.nullRatio = digits === '' ? '' : Number(digits)
  return digits
}
const onNullRatioBlur = (row) => {
  const n = Number(row.nullRatio)
  row.nullRatio = Number.isFinite(n) ? Math.min(100, Math.max(0, Math.trunc(n))) : 0
}

const splitValues = (text) => String(text || '')
  .split(/[,，\n]/)
  .map(s => s.trim())
  .filter(s => s !== '')

const buildPayload = (mode, n) => ({
  database: props.target?.db || '',
  table: props.target?.table || '',
  rows: n,
  mode,
  batchSize: batchSize.value,
  clearBefore: writeMode.value === 'clear',
  seed: /^\d+$/.test(seed.value.trim()) ? Number(seed.value.trim()) : null,
  columns: colRules.value.map(c => {
    const o = { name: c.name, rule: c.rule }
    const kind = paramKind(c.rule)
    if (kind === 'value' || kind === 'prefix') o.value = c.value || ''
    if (kind === 'values') o.values = splitValues(c.valuesText)
    if (kind === 'range' || kind === 'dateRange') {
      if (c.min) o.min = c.min
      if (c.max) o.max = c.max
    }
    if (kind === 'length' && c.length) o.length = c.length
    if (kind === 'start' && c.start !== null && c.start !== undefined) o.start = c.start
    const nr = Math.min(100, Math.max(0, Number(c.nullRatio) || 0))
    if (nr) o.nullRatio = nr
    return o
  })
})

// ==================== 步骤动作 ====================

const doPreview = async () => {
  if (!colRules.value.length) return ElMessage.warning(t('dgen.noFieldInfo'))
  previewing.value = true
  try {
    const res = await dataGenPreview(props.conn?.id, buildPayload('preview', 10))
    if (!res || !res.success) {
      ElMessage.error(res?.message || t('dgen.previewFailed'))
      return
    }
    previewColumns.value = res.columns || []
    previewRows.value = res.rows || []
    notes.value = res.notices || []
  } catch (e) {
    ElMessage.error(t('dgen.previewFailedDetail', { detail: errMsg(e) }))
  } finally {
    previewing.value = false
  }
}

// 进入第 2 步同时刷新预览，避免展示被改动过的旧规则结果
const goPreview = async () => {
  step.value = 'preview'
  await doPreview()
}

// 回到第 1 步：若上次任务已结束则清空执行结果，便于改完规则重新生成
const backToRules = () => {
  if (running.value) return
  step.value = 'rules'
  if (!job.value) return
  job.value = null
  logs.value = []
  notes.value = []
  notices.value = []
  error.value = ''
  previewColumns.value = []
  previewRows.value = []
  lastLogStage = ''
  lastLogBucket = -1
}

const doGenerate = async () => {
  if (running.value) return
  if (!colRules.value.length) return ElMessage.warning(t('dgen.noFieldInfo'))
  if (writeMode.value === 'clear') {
    try {
      await ElMessageBox.confirm(
        t('dgen.clearConfirm', { table: props.target?.table, rows: rows.value }),
        t('dgen.title'),
        { type: 'warning', confirmButtonText: t('dgen.confirmGen'), cancelButtonText: t('common.cancel') }
      )
    } catch {
      return
    }
  }
  error.value = ''
  logs.value = []
  notices.value = []
  lastLogStage = ''
  lastLogBucket = -1
  try {
    // 后台任务：立即返回 taskId，进度与结果通过轮询获取（生成中可取消）
    const res = await dataGenStart(props.conn?.id, buildPayload('insert', rows.value))
    if (!res || !res.success) {
      error.value = res?.message || t('common.unknownError')
      pushLog(t('dgen.startFailed', { detail: error.value }), 'error')
      ElMessage.error(t('dgen.startFailed', { detail: error.value }))
      return
    }
    canceling.value = false
    job.value = res
    pushLog(t('dgen.logStart', {
      rows: rows.value,
      target: `${props.target?.db}.${props.target?.table}`,
      clear: writeMode.value === 'clear' ? t('dgen.logStartClear') : ''
    }))
    pushLog(t('dgen.logTaskId', { id: res.taskId }))
    startPoll()
  } catch (e) {
    error.value = errMsg(e)
    pushLog(t('dgen.startFailed', { detail: error.value }), 'error')
    ElMessage.error(t('dgen.startFailed', { detail: error.value }))
  }
}

// 打开弹窗时重置状态并按真实字段结构加载规则；关闭时停止进度轮询
watch(() => props.modelValue, async (open) => {
  if (!open) {
    stopPoll()
    return
  }
  resetState()
  await loadColumns()
})
</script>

<style scoped>
.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}
.dlg-target {
  margin-left: 4px; padding: 2px 8px; border-radius: 6px; font-size: 13px;
  color: var(--dc-text-mid); background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border-soft); font-family: var(--dc-mono-font);
}
.dlg-target .dot { color: var(--dc-text-weak); margin: 0 1px; }

/* 向导 */
.gen-steps { margin: 0 0 14px; flex-shrink: 0; }
.gen-steps :deep(.el-step__title) { font-size: 14px; }
/* 弹窗外壳（宽度/高度上限/body 撑满）见文件末尾的非 scoped 样式块 */
.gen-body {
  flex: 1; min-height: 0; overflow-y: auto; overflow-x: hidden; padding-right: 4px;
}

/* 通用块 */
.step-subtitle {
  font-size: 14px; font-weight: 600; color: var(--dc-text-strong);
  margin: 4px 0 8px; padding-left: 9px; border-left: 3px solid var(--dc-primary);
}
.step-card {
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border-soft);
  border-radius: 8px; padding: 14px 16px 4px; margin-bottom: 14px;
}
/* 生成设置：两列网格（宽屏下 4 项排成 2×2），label 固定宽度保证两列对齐；
   控件在各自列内自适应并限宽，避免输入框被拉得过长 */
.datagen-form .form-row {
  display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); column-gap: 28px;
}
.datagen-form :deep(.el-form-item) { margin-bottom: 12px; }
.datagen-form :deep(.el-input-number.fillable),
.datagen-form :deep(.el-input.fillable) { width: 100%; max-width: 280px; }
/* 窄屏（<760px）退化为单列，避免控件被压得太窄 */
@media (max-width: 760px) {
  .datagen-form .form-row { grid-template-columns: minmax(0, 1fr); }
}

.section-head { display: flex; align-items: center; gap: 10px; margin: 4px 0 8px; }
.section-head .section-title { flex-shrink: 0; }
.section-title { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); }
.section-tip { font-size: 13px; color: var(--dc-text-weak); flex: 1; line-height: 1.5; }

/* 字段规则表 */
.rule-table :deep(.el-table__cell) { padding: 4px 0; }
/* 垂直滚动条：Element 自定义的 bar 与原生滚动条一律不显示，滚动仍可用滚轮/触控板 */
.rule-table :deep(.el-scrollbar__wrap::-webkit-scrollbar) { width: 0; height: 0; display: none; }
.rule-table :deep(.el-scrollbar__wrap) { scrollbar-width: none; -ms-overflow-style: none; }
.rule-table :deep(.el-scrollbar__bar) { display: none !important; }

/* 空值% 列：原生纯文本输入框，杜绝任何加减/步进控件 */
.ratio-input {
  display: block; width: 60px; height: 24px; margin: 0 auto; padding: 0 8px;
  box-sizing: border-box; text-align: center; line-height: 22px;
  font-family: var(--dc-mono-font); font-size: 13px;
  color: var(--dc-text); background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border-soft); border-radius: 4px;
  outline: none; transition: border-color 0.2s ease;
}
.ratio-input:hover { border-color: var(--dc-border); }
.ratio-input:focus { border-color: var(--dc-primary); }
.ratio-input:disabled {
  color: var(--dc-text-weak); cursor: not-allowed; background: var(--dc-bg-soft);
  border-color: var(--dc-border-soft);
}
/* 双保险：即使浏览器把它当数字框，也隐藏原生步进按钮 */
.ratio-input::-webkit-outer-spin-button,
.ratio-input::-webkit-inner-spin-button { -webkit-appearance: none; margin: 0; }
.ratio-input[type='number'] { -moz-appearance: textfield; }
.cell-name { display: flex; align-items: center; gap: 6px; }
.cname { font-family: var(--dc-mono-font); font-size: 13px; color: var(--dc-text-strong); }
.cell-comment {
  font-size: 12px; color: var(--dc-text-weak); overflow: hidden;
  text-overflow: ellipsis; white-space: nowrap;
}
.ctype { font-size: 13px; color: var(--dc-text-mid); word-break: break-all; }
.param-range { display: flex; align-items: center; gap: 6px; }
.range-sep { color: var(--dc-text-weak); flex-shrink: 0; }
.param-none { color: var(--dc-text-weak); }

/* 预览 */
.preview-table { width: 100%; }
.empty-tip { font-size: 13px; color: var(--dc-text-weak); }

/* 确认摘要 */
.summary-card {
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border-soft);
  border-radius: 8px; padding: 2px 14px; margin-bottom: 14px;
}
.summary-row {
  display: flex; align-items: center; justify-content: space-between; gap: 12px;
  padding: 8px 0; border-bottom: 1px solid var(--dc-border-soft); font-size: 14px;
}
.summary-row:last-child { border-bottom: none; }
.summary-label { color: var(--dc-text-dim); flex-shrink: 0; }
.summary-value {
  color: var(--dc-text); text-align: right; flex: 1; min-width: 0;
  word-break: break-all; display: flex; align-items: center; justify-content: flex-end; gap: 8px;
}
.summary-value.mono { font-family: var(--dc-mono-font); }
.warn-text { font-size: 13px; color: var(--dc-warning); }

/* 进度 */
.gp-count { font-size: 13px; color: var(--dc-text-dim); font-family: var(--dc-mono-font); flex-shrink: 0; }

/* 实时日志控制台（跟随主题：深色 / 浅色均可读） */
.log-box {
  height: 190px; overflow: auto; padding: 8px 10px; border-radius: 8px;
  background: var(--dc-bg-code); border: 1px solid var(--dc-border);
  color: var(--dc-code-text); font-family: var(--dc-mono-font);
  font-size: 13px; line-height: 1.7;
}
.log-line { display: flex; gap: 8px; }
.log-time { color: var(--dc-text-weak); flex-shrink: 0; }
.log-text { word-break: break-all; white-space: pre-wrap; }
.lv-success .log-text { color: var(--dc-success); }
.lv-error .log-text { color: var(--dc-danger); }
.lv-warn .log-text { color: var(--dc-warning); }
.log-empty { color: var(--dc-text-weak); }

.error-box {
  margin-top: 10px; padding: 8px 10px; border-radius: 6px;
  background: var(--dc-danger-wash); color: var(--dc-danger);
  font-size: 13px; line-height: 1.7; max-height: 120px; overflow: auto; white-space: pre-wrap;
}
.notes {
  margin-top: 10px; max-height: 110px; overflow: auto; padding: 8px 10px;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border-soft); border-radius: 8px;
}
.note-line { font-size: 13px; color: var(--dc-text-mid); line-height: 1.7; }

/* 底部 */
.datagen-footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.footer-tip { font-size: 13px; color: var(--dc-text-weak); text-align: left; line-height: 1.5; }
.footer-btns { flex-shrink: 0; }
</style>

<style>
/* 弹窗外壳样式必须放全局块：el-dialog 由 teleport 挂到 body，scoped / :deep 无法命中其根节点
   （ObjectFormDialog、SettingsView 的弹窗外壳样式同理） */
.datagen-dialog.el-dialog {
  /* 固定宽度：窗口不够宽时收缩为「视口宽 - 48px」，避免超出视口被左右裁切、遮罩层出现横向滚动条 */
  width: min(1000px, calc(100% - 48px));
  /* 垂直居中由 index.css 里的 .el-overlay-dialog 统一处理（margin: auto）；
     这里用 88vh 高度上限保证弹窗不会超出视口 */
  max-height: 88vh;
  display: flex;
  flex-direction: column;
}
.datagen-dialog .el-dialog__header,
.datagen-dialog .el-dialog__footer { flex-shrink: 0; }
/* body 撑满剩余高度，让内部滚动限定在 .gen-body，保证底部按钮始终可见 */
.datagen-dialog .el-dialog__body {
  padding: 8px 22px 0;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
</style>
