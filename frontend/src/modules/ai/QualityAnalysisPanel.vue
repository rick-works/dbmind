<template>
  <div class="qa">
    <!-- 工具栏：范围选择（分析/导出按钮在弹窗底部） -->
    <div class="qa-bar">
      <el-select
        v-model="scope"
        size="small"
        class="qa-scope"
        :placeholder="configuredCount ? $t('qa.scopeAll') : $t('qa.scopeNone')"
        filterable
        :disabled="disabled || !configuredCount"
      >
        <el-option :label="$t('qa.scopeAll')" value="__ALL__" />
        <el-option
          v-for="t in configuredTables"
          :key="t.table"
          :label="$t('qa.tableOption', { table: t.table, n: t.ruleCount })"
          :value="t.table"
        />
      </el-select>

      <span class="qa-spacer"></span>
      <span class="qa-meta">
        <el-icon class="qa-btn-ic"><Files /></el-icon>
        {{ $t('qa.configuredPre') }}<b>{{ configuredCount }}</b>{{ $t('qa.configuredMid') }}<b>{{ configuredRules }}</b>{{ $t('qa.configuredPost') }}
      </span>
    </div>

    <!-- 分析进行中：进度条 + 实时日志 + 取消（避免长分析时只能干等、想取消取消不掉） -->
    <div v-if="scanRunning" class="qa-progress">
      <div class="qa-progress-head">
        <el-progress
          :percentage="scanProgress"
          :stroke-width="14"
          text-inside
          class="qa-progress-bar"
        />
      </div>
      <div v-if="scanningTable" class="qa-progress-cur">{{ $t('qa.analyzing') }}<b>{{ scanningTable }}</b></div>
      <div ref="logBoxRef" class="qa-logs">
        <div v-for="(l, i) in scanLogs" :key="i" class="qa-log" :class="'lv-' + l.level">
          <span class="qa-log-time">{{ l.time }}</span>{{ l.text }}
        </div>
      </div>
    </div>

    <!-- 未配置规则：给出可一键跳转的引导（而不是一个点不动的下拉） -->
    <div v-else-if="!configuredCount" class="qa-empty">
      {{ $t('qa.needRules') }}<a class="qa-link" @click="emit('goto-rules')">{{ $t('qa.rulesTitle') }}</a>{{ $t('qa.needRulesPost') }}
    </div>

    <!-- 扫描结果 -->
    <template v-else-if="scanResult">
      <div class="qa-summary">
        <span class="qa-score" :class="scoreClass(scanResult.score)">{{ scanResult.score }}</span>
        <div class="qa-summary-body">
          <div class="qa-summary-title">{{ $t('qa.scoreTitle') }}</div>
          <div class="qa-summary-meta">
            {{ $t('qa.statTables', { n: scanResult.tableCount }) }} · {{ $t('qa.statRules', { n: scanResult.totalRules }) }} ·
            {{ $t('qa.statPassed', { n: scanResult.passed }) }} · {{ $t('qa.statFailed', { n: scanResult.failed }) }}
            <template v-if="scanResult.errors">{{ $t('qa.statErrors', { n: scanResult.errors }) }}</template>
            <template v-if="scanResult.skipped">{{ $t('qa.statSkipped', { n: scanResult.skipped }) }}</template>
          </div>
        </div>
        <span class="qa-time">{{ scanResult.scanTime }}</span>
      </div>

      <div class="qa-tables">
        <div v-for="t in scanResult.tables" :key="t.table" class="qa-tbl-card">
          <div class="qa-table-head" :class="t.failed || t.errors ? 'bad' : 'ok'">
            <span class="qa-table-name">{{ t.table }}</span>
            <span class="qa-table-info">
              {{ $t('qa.cardStats', { rules: t.rules, passed: t.passed, failed: t.failed, score: t.score }) }}
            </span>
            <span class="qa-spacer"></span>
            <span class="qa-table-tag" :class="t.failed || t.errors ? 'bad' : 'ok'">
              {{ t.failed || t.errors ? $t('qa.hasProblem') : $t('qa.allPassed') }}
            </span>
          </div>
          <div v-if="t.message" class="qa-table-msg">{{ t.message }}</div>
          <div v-for="(d, i) in failedRows(t)" :key="i" class="qa-row" :class="d.status">
            <span class="qa-badge">{{ d.column }}</span>
            <span class="qa-row-type">{{ typeLabel(d.type) }}</span>
            <span class="qa-row-level" :class="levelClass(d.level)">{{ levelText(d.level) }}</span>
            <span class="qa-row-msg">
              <template v-if="d.status === 'failed'">{{ $t('qa.violationsLine', { n: d.violations, message: d.ruleMessage }) }}</template>
              <template v-else>{{ $t('qa.execError', { message: d.message }) }}</template>
            </span>
          </div>
        </div>
      </div>
    </template>

    <!-- 未扫描：说明合并为一块，与「质量规则」页签的提示形态保持一致 -->
    <div v-else class="qa-empty">
      {{ $t('qa.startHint') }}<b>{{ $t('qa.startScan') }}</b>{{ $t('qa.startHint2') }}<a class="qa-link" @click="emit('goto-rules')">{{ $t('qa.rulesTitle') }}</a>{{ $t('qa.startHint3') }}
      {{ $t('qa.startHint4') }}
    </div>

    <!-- 报告弹窗：表格展示 + 分页；导出时再选 Excel / CSV -->
    <el-dialog v-model="reportVisible" :title="$t('qa.reportTitle')" width="940px" top="6vh" class="qa-sub-dialog" append-to-body>
      <div class="qa-report-bar">
        <span class="qa-report-meta">
          {{ $t('qa.reportRowsPre') }}<b>{{ reportRows.length }}</b>{{ $t('qa.reportRowsMid') }}
          <template v-if="reportSkipped">{{ $t('qa.reportSkipped', { n: reportSkipped }) }}</template>
        </span>
        <span class="qa-spacer"></span>
        <el-dropdown trigger="click" @command="downloadReport">
          <el-button size="small" text :loading="reportExporting" :icon="Download" :title="$t('qa.exportBtn')" class="qa-export-btn" />
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item command="excel">{{ $t('qa.exportExcel') }}</el-dropdown-item>
              <el-dropdown-item command="csv">{{ $t('qa.exportCsv') }}</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </div>

      <el-table :data="reportPageRows" size="small" border class="qa-table">
        <el-table-column prop="table" :label="$t('qa.colTable')" width="150" show-overflow-tooltip />
        <el-table-column prop="column" :label="$t('dgen.colField')" width="150" show-overflow-tooltip />
        <el-table-column prop="rule" :label="$t('qa.colRule')" width="104" />
        <el-table-column prop="level" :label="$t('qa.colLevel')" width="62" align="center">
          <template #default="{ row }">{{ levelText(row.level) }}</template>
        </el-table-column>
        <el-table-column prop="status" :label="$t('sqlq.result')" width="76" align="center" />
        <el-table-column prop="violations" :label="$t('qa.colViolations')" width="90" align="right" />
        <el-table-column prop="message" :label="$t('udv.fComment')" min-width="180" show-overflow-tooltip />
      </el-table>

      <el-pagination
        v-model:current-page="reportPageNo"
        :page-size="reportPageSize"
        :total="reportRows.length"
        layout="total, prev, pager, next"
        small
        class="qa-pager"
      />
    </el-dialog>

    <!-- 错误数据弹窗：表格展示 + 服务端分页；导出时再选 Excel / CSV -->
    <el-dialog v-model="violationsVisible" :title="$t('qa.violationsTitle')" width="940px" top="6vh" class="qa-sub-dialog" append-to-body>
      <div class="qa-report-bar">
        <el-select
          v-model="violationsTable"
          size="small"
          style="width:220px"
          :placeholder="$t('qa.pickTableSelect')"
          filterable
          :disabled="!failedTables.length"
          @change="onViolationTableChange"
        >
          <el-option v-for="t in failedTables" :key="t" :label="t" :value="t" />
        </el-select>
        <span v-if="violationsTotal > 0" class="qa-report-meta">{{ $t('qa.violationsCountPre') }}<b>{{ violationsTotal }}</b>{{ $t('qa.violationsCountPost') }}</span>
        <span v-else-if="violationsTotal === 0 && !violationsLoading" class="qa-report-meta">{{ $t('qa.noViolations') }}</span>
        <span class="qa-spacer"></span>
        <el-dropdown trigger="click" @command="exportViolationsFile">
          <!-- 与 SQL 结果区 / 数据表格页的导出按钮保持完全一致的样式：text + Download + 无文字 -->
          <el-button
            size="small"
            text
            :loading="violationsExporting"
            :disabled="!violationsTable"
            :icon="Download"
            :title="$t('qa.exportBtn')"
            class="qa-export-btn"
          />
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item command="current-csv">{{ $t('qa.exportCurCsv') }}</el-dropdown-item>
              <el-dropdown-item command="current-excel">{{ $t('qa.exportCurExcel') }}</el-dropdown-item>
              <el-dropdown-item divided command="all-csv">{{ $t('qa.exportAllCsv') }}</el-dropdown-item>
              <el-dropdown-item command="all-excel">{{ $t('qa.exportAllExcel') }}</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </div>

      <!-- 加载中：表头也一起隐藏，等数据到位再整体渲染，避免先出现「违反规则」列再补数据 -->
      <div v-if="violationsLoading" class="qa-loading-box">
        <el-icon class="is-loading"><Loading /></el-icon>
        <span>{{ $t('qa.loadingErrors') }}</span>
      </div>

      <el-table
        v-else
        :empty-text="violationsFetchError || $t('qa.noViolationData')"
        :data="violationsRows"
        :show-header="violationsRows.length > 0"
        size="small"
        border
        class="qa-table"
      >
        <el-table-column prop="__rule" :label="$t('qa.colRuleViolated')" width="146" fixed show-overflow-tooltip />
        <el-table-column
          v-for="c in violationsColumns"
          :key="c"
          :prop="'c_' + c"
          :label="c"
          min-width="132"
          show-overflow-tooltip
        />
      </el-table>

      <el-pagination
        v-model:current-page="violationsPageNo"
        v-model:page-size="violationsPageSize"
        :page-sizes="VIOLATION_PAGE_SIZES"
        :total="violationsTotal"
        layout="total, sizes, prev, pager, next"
        small
        class="qa-pager"
        @current-change="onViolationPageChange"
        @size-change="onViolationSizeChange"
      />
    </el-dialog>

    <!-- 导出全部：复用通用异步导出任务弹窗（流式导出 + 进度 + 实时日志 + 可取消） -->
    <TaskProgressDialog
      v-model:visible="exportTask.visible"
      task-kind="export"
      :target-name="violationsTable || $t('qa.violationsData')"
      :status="exportTask.status"
      :done="exportTask.done"
      :total="exportTask.total"
      :phase="exportTask.phase"
      :message="exportTask.message"
      :logs="exportTask.logs"
      :canceling="exportTask.canceling"
      @cancel="exportTask.cancel(props.conn?.id)"
      @close="exportTask.close"
    />
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, nextTick } from 'vue'
import { ElMessage } from 'element-plus'
import { Files, Download, Loading } from '@element-plus/icons-vue'
import { aiQualityTypes, aiQualityConfigured, aiQualityScan, aiQualityReport, aiQualityViolations } from '../../api'
import TaskProgressDialog from '../../common/TaskProgressDialog.vue'
import { useExportTask } from '../../utils/useExportTask'
import { getQuerySettings } from '../../utils/settings'
import { t, te } from '../../utils/i18n'

const props = defineProps({
  conn: { type: Object, default: null },
  database: { type: String, default: '' },
  disabled: { type: Boolean, default: false },
  isNoSql: { type: Boolean, default: false }
})

/** 分析 / 导出按钮已移到弹窗底部，这里把状态与能力上报给父组件 */
/** goto-rules：未配置规则时，引导用户跳到「质量规则」页签 */
const emit = defineEmits(['state', 'goto-rules'])

// 默认留空 → 显示为 placeholder（「全部已配置规则的表」），与「质量规则」的表下拉观感一致。
// 空值在 scan() 中即视为「全部」，与选择 __ALL__ 等价。
const scope = ref('')
const configuredTables = ref([])
const configuredCount = ref(0)
const configuredRules = ref(0)
const types = ref([])

const scanning = ref(false)
const scanResult = ref(null)
const exportingViolations = ref(false)

// ===== 分析进度 / 实时日志 / 取消 =====
// 采用「逐表分析」：每张表一次请求，进度是真实完成度，取消可立即生效（不再发起下一张表）
const scanRunning = ref(false)
const scanProgress = ref(0)
const scanLogs = ref([])
const scanningTable = ref('')
const logBoxRef = ref(null)
/** 取消标记：置为 true 后，逐表循环会在当前表结束后停止 */
let scanCancelled = false

// ===== 报告弹窗：表格 + 前端分页（数据量 = 表数 × 规则数，量级小，无需服务端分页）=====
const reportVisible = ref(false)
const reportRows = ref([])
const reportSkipped = ref(0)
const reportPageNo = ref(1)
const reportPageSize = 20
const reportExporting = ref(false)
/** 当前页明细 */
const reportPageRows = computed(() => {
  const from = (reportPageNo.value - 1) * reportPageSize
  return reportRows.value.slice(from, from + reportPageSize)
})

// ===== 错误数据弹窗：选表 + 表格 + 服务端分页（行数可能上万，必须服务端分页）=====
const violationsVisible = ref(false)
const violationsTable = ref('')
const violationsColumns = ref([])
const violationsRows = ref([])
// -1 表示「总数未知」（换表后需重新统计）；0 表示已统计且无违规行
const violationsTotal = ref(-1)
const violationsPageNo = ref(1)
/** 可选每页条数：与「设置 → 查询设置 → 默认分页」同一套档位，便于两处对得上 */
const VIOLATION_PAGE_SIZES = [20, 50, 100, 200, 500, 1000, 2000, 5000]
/** 每页条数：默认跟随设置里的「默认分页」，也可在本弹窗内临时调整（不影响设置本身） */
const violationsPageSize = ref(pickDefaultPageSize())

function pickDefaultPageSize() {
  const ps = Number(getQuerySettings().pageSize)
  return VIOLATION_PAGE_SIZES.includes(ps) ? ps : 20
}

// 改「每页条数」时 el-pagination 会紧接着再回调一次 current-change，
// 用这个标记把紧随其后的那一次吃掉，避免同一操作触发两次请求
let violationSizeChanging = false
const onViolationSizeChange = () => {
  violationSizeChanging = true
  violationsPageNo.value = 1   // 每页条数变了，页码回到第 1 页
  loadViolations()
  setTimeout(() => { violationSizeChanging = false }, 0)
}
const onViolationPageChange = () => {
  if (violationSizeChanging) return
  loadViolations()
}
const violationsLoading = ref(false)
const violationsExporting = ref(false)
// 「导出全部」复用通用异步导出任务（与 SQL 查询结果区同一套：进度 + 实时日志 + 可取消）
const exportTask = useExportTask()
/** 取数失败原因（COUNT 成功但取不到行，如连接中断）；空表示正常 */
const violationsFetchError = ref('')

/** 本次扫描中存在违规的表（错误数据按表查看：不同表列结构不同，无法合并成一张表） */
const failedTables = computed(() =>
  (scanResult.value?.tables || []).filter(t => t.failed > 0).map(t => t.table)
)

// 类型名的中文原文有两个来源：用户可配置类型的目录（后端 quality.rs 下发）与
// 「扫描」自动生成的类型（constant / null_ratio / empty_column 等，不在目录里）。
// 前端按 type 键用字典覆盖：收录过的走译文，没收录的（后端以后新增的类型）原样显示
// 后端文案或原始英文码 —— 所以这里用 te() 判断"有没有收录"。
/** 规则类型 → 显示名。大小写不敏感，避免 Constant / MAX_LENGTH 这类写法匹配不到而露出英文码 */
const typeLabel = (ty) => {
  const key = String(ty || '').trim().toLowerCase()
  if (!key) return ''
  if (te('qa.type.' + key)) return t('qa.type.' + key)
  const hit = types.value.find(x => String(x.type || '').trim().toLowerCase() === key)
  return hit?.label || ty
}
const levelClass = (lv) => (lv === '高' ? 'high' : lv === '中' ? 'mid' : 'low')

/** 等级：扫描结果里存的是「高/中/低」（levelClass 也拿它判断），只在显示时翻译。 */
const levelText = (lv) => {
  if (lv === '高') return t('qa.level.high')
  if (lv === '中') return t('qa.level.mid')
  if (lv === '低') return t('qa.level.low')
  return lv || ''
}
const scoreClass = (s) => (s >= 90 ? 'good' : s >= 70 ? 'warn' : 'bad')

/** 只展示关注的行：失败与异常 */
const failedRows = (t) => (t.details || []).filter(d => d.status === 'failed' || d.status === 'error')

const loadTypes = async () => {
  if (types.value.length) return
  try {
    const r = await aiQualityTypes()
    types.value = (r && r.types) || []
  } catch (e) { types.value = [] }
}

/** 读取该连接/库下已配置规则的表 */
const loadConfigured = async (silent) => {
  if (!props.conn || !props.database) {
    configuredTables.value = []
    configuredCount.value = 0
    configuredRules.value = 0
    return
  }
  try {
    const r = await aiQualityConfigured({ connectionId: props.conn.id, database: props.database })
    const list = (r && r.tables) || []
    // 只保留当前库的表
    configuredTables.value = list
    configuredCount.value = list.length
    configuredRules.value = list.reduce((n, t) => n + (t.ruleCount || 0), 0)
    if (!silent && !list.length) ElMessage.info(t('qa.noConfiguredTables'))
  } catch (e) {
    configuredTables.value = []
    configuredCount.value = 0
    configuredRules.value = 0
  }
}

/** 追加一条实时日志并自动滚到底部 */
const pushLog = (level, text) => {
  // 局部变量原来叫 t：与 i18n 的 t() 同名，后续改动很容易被遮蔽（t('x') 变成"日期对象当函数调"）
  const now = new Date()
  const pad = (n) => String(n).padStart(2, '0')
  scanLogs.value.push({ level, time: `${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`, text })
  nextTick(() => {
    const el = logBoxRef.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

/**
 * 执行质量分析：**逐表**请求，便于展示真实进度、实时日志并支持中途取消。
 *
 * 为什么不用「一次请求扫全部表」：后端是同步执行，请求期间前端拿不到任何进度，
 * 表多或库慢时用户只能干等，且取消不了。逐表拆分后，
 * 每完成一张表就刷新进度与日志，取消也能在下一张表开始前立即生效。
 */
const scan = async () => {
  const conn = props.conn
  if (!conn || !props.database) return
  const targets = scope.value && scope.value !== '__ALL__'
    ? [scope.value]
    : configuredTables.value.map(x => x.table).filter(Boolean)
  if (!targets.length) { ElMessage.warning(t('qa.noTargets')); return }

  scanning.value = true
  scanRunning.value = true
  scanCancelled = false
  scanProgress.value = 0
  scanLogs.value = []
  scanResult.value = null
  pushLog('info', t('qa.anaStart', { n: targets.length }))

  const merged = {
    database: props.database,
    tables: [],
    tableCount: 0, totalRules: 0, passed: 0, failed: 0, skipped: 0, errors: 0, score: 100,
    scanTime: ''
  }
  let done = 0

  for (const tb of targets) {
    if (scanCancelled) {
      pushLog('warn', t('qa.anaCanceled', { n: targets.length - done }))
      break
    }
    scanningTable.value = tb
    pushLog('info', t('qa.anaProgress', { i: done + 1, n: targets.length, table: tb }))
    try {
      const r = await aiQualityScan({
        connectionId: conn.id,
        database: props.database,
        tables: [tb],
        maxTables: 1
      })
      if (r && r.success) {
        const tr = (r.tables || [])[0]
        if (tr) merged.tables.push(tr)
        const bad = (tr?.failed || 0) + (tr?.errors || 0)
        pushLog(
          bad ? 'warn' : 'ok',
          t('qa.anaDone', { rules: tr?.rules ?? 0, passed: tr?.passed ?? 0, failed: tr?.failed ?? 0 }) +
          (tr?.errors ? t('qa.anaDoneErrors', { n: tr.errors }) : '') + t('qa.anaDoneScore', { score: tr?.score ?? '-' })
        )
      } else {
        pushLog('error', t('qa.anaFailed', { detail: (r?.message || t('common.unknownError')) }))
      }
    } catch (e) {
      pushLog('error', t('qa.anaFailed', { detail: (e?.message || e) }))
    }
    done++
    scanProgress.value = Math.round(done * 100 / targets.length)
  }

  scanningTable.value = ''
  scanRunning.value = false
  scanning.value = false

  if (!merged.tables.length) {
    scanResult.value = null
    if (!scanCancelled) pushLog('error', t('qa.noResult'))
    return
  }
  // 汇总（口径与后端一次性扫描保持一致）
  merged.tableCount = merged.tables.length
  for (const t of merged.tables) {
    merged.totalRules += t.rules || 0
    merged.passed += t.passed || 0
    merged.failed += t.failed || 0
    merged.skipped += t.skipped || 0
    merged.errors += t.errors || 0
  }
  const exec = merged.passed + merged.failed + merged.errors
  merged.score = exec === 0 ? 100 : Math.round(merged.passed * 100 / exec)
  const now = new Date()
  const pad = (n) => String(n).padStart(2, '0')
  merged.scanTime = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ` +
    `${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`
  scanResult.value = merged
  pushLog('ok', t('qa.anaFinishedLine', { state: (scanCancelled ? t('qa.anaFinishedCanceled') : t('qa.anaFinished')), n: merged.tableCount, score: merged.score }))
}

/** 取消分析：当前表结束后不再继续，已完成的表保留为结果 */
const cancelScan = () => {
  if (!scanRunning.value) return
  scanCancelled = true
  pushLog('warn', t('qa.canceling'))
}

/** 状态文案：字典里有就用译文，没有就原样（后端以后新增的状态不会显示成键名） */
const statusLabel = (s) => {
  const key = String(s || '')
  if (key && te('qa.st.' + key)) return t('qa.st.' + key)
  return key || '—'
}

/** 打开「导出报告」弹窗：把扫描结果展平为规则明细（表格 + 前端分页） */
const openReport = async () => {
  if (!scanResult.value) return
  const rows = []
  let notConfigured = 0
  // 循环变量叫 t 会遮蔽 i18n 的 t()：需要的那条译文先在循环外取好
  const notCfg = t('qa.notConfigured')
  for (const t of scanResult.value.tables || []) {
    const details = t.details || []
    if (!details.length) {
      notConfigured++
      rows.push({
        table: t.table || '—', column: '—', rule: '—', level: '—',
        status: t.message || notCfg, violations: 0, message: ''
      })
      continue
    }
    for (const d of details) {
      rows.push({
        table: t.table || '—',
        column: d.column || '—',
        rule: typeLabel(d.type),
        level: d.level || '—',
        status: statusLabel(d.status),
        violations: d.violations || 0,
        // 说明优先取「规则配置」里填写的说明（与规则配置页签一致），
        // 未填写时才回落到系统提示（未启用 / 表级规则 / 执行异常等）
        message: d.ruleMessage || d.message || ''
      })
    }
  }
  reportRows.value = rows
  reportSkipped.value = notConfigured
  reportPageNo.value = 1
  reportVisible.value = true
}

/** 导出报告：按所选格式由后端生成内容后下载（excel 为零依赖 SpreadsheetML） */
const downloadReport = async (command) => {
  if (!scanResult.value) return
  const fmt = command || 'excel'
  reportExporting.value = true
  try {
    const r = await aiQualityReport({
      connectionId: props.conn?.id || '',
      format: fmt,
      scan: scanResult.value
    })
    if (r && r.success && r.content) {
      downloadText(r.content, r.filename || `data-quality-report.${fmt === 'csv' ? 'csv' : 'xls'}`)
      ElMessage.success(t('qa.downloadStarted'))
    } else {
      ElMessage.error(r?.message || t('qa.reportFailed'))
    }
  } catch (e) {
    ElMessage.error(e?.message || t('qa.reportFailed'))
  } finally {
    reportExporting.value = false
  }
}

/** 打开「错误数据」弹窗：按表分页查看规则命中的违规行 */
const exportViolations = async () => {
  if (!scanResult.value) return
  violationsVisible.value = true
  violationsPageNo.value = 1
  const first = failedTables.value[0] || ''
  violationsTable.value = first
  violationsTotal.value = -1
  violationsFetchError.value = ''
  if (!first) {
    violationsColumns.value = []
    violationsRows.value = []
    violationsTotal.value = 0
    return
  }
  exportingViolations.value = true
  try {
    await loadViolations()
  } finally {
    exportingViolations.value = false
  }
}

/** 切换表 → 回到第 1 页重新加载（总数需重新统计） */
const onViolationTableChange = async () => {
  violationsPageNo.value = 1
  violationsTotal.value = -1
  violationsFetchError.value = ''
  await loadViolations()
}

/**
 * 加载「当前表 + 当前页」的违规行（服务端分页）。
 *
 * 后端只取本页需要的行数、并用 COUNT(*) 统计总数，因此翻页不会全量拉取数据。
 */
const loadViolations = async () => {
  const conn = props.conn
  const tb = violationsTable.value
  if (!conn || !tb) return
  violationsLoading.value = true
  // 立即清空：切换表时不要残留上一张表的数据，让「加载中」状态清晰可见
  violationsRows.value = []
  violationsColumns.value = []
  try {
    const offset = (violationsPageNo.value - 1) * violationsPageSize.value
    // 仅在该表「首次加载」时统计总数，翻页复用上一页的结果——
    // 规则较多时，每条规则一次 COUNT(*) 正是翻页变慢的主因
    const needTotal = violationsTotal.value < 0
    const r = await aiQualityViolations({
      connectionId: conn.id,
      database: props.database,
      table: tb,
      offset,
      limit: violationsPageSize.value,
      format: 'excel',
      withTotal: needTotal
    })
    if (!r || !r.success) {
      ElMessage.error(r?.message || t('qa.loadErrorsFailed'))
      violationsColumns.value = []
      violationsRows.value = []
      violationsTotal.value = -1
      violationsFetchError.value = r?.message || t('qa.loadErrorsFailed')
      return
    }
    violationsColumns.value = r.columns || []
    // total 为 -1 表示本次未统计（翻页复用），此时保留已有总数
    if (typeof r.total === 'number' && r.total >= 0) violationsTotal.value = r.total
    // 后端返回二维数组（与 columns 对齐），转成 el-table 需要的对象数组
    const cols = violationsColumns.value
    violationsRows.value = (r.rows || []).map(line => {
      const o = { __rule: line[0] || '' }
      for (let i = 0; i < cols.length; i++) o['c_' + cols[i]] = line[i + 1]
      return o
    })
    // 总数统计成功但取数失败（连接中断等）时给出明确原因，
    // 否则界面会呈现「共 N 条」却「暂无数据」的矛盾状态
    violationsFetchError.value = (r.fetchErrors && !violationsRows.value.length)
      ? t('qa.fetchFailed', { n: r.fetchErrors, detail: (r.fetchError || t('qa.checkConn')) })
      : ''
  } catch (e) {
    ElMessage.error(e?.message || t('qa.loadErrorsFailed'))
  } finally {
    violationsLoading.value = false
  }
}

/**
 * 导出违规数据（与 SQL 查询结果区一致的四个选项）：
 *  当前页 CSV / Excel —— 只导出表格当前页的数据，由后端生成文件；
 *  全部 CSV / Excel   —— 复用通用异步导出任务：后端按 UNION ALL 合成 SQL 流式导出，
 *                        带进度条、实时日志，且可随时取消（大数据量不会卡住界面）。
 */
const exportViolationsFile = async (cmd) => {
  const [scope, format] = String(cmd || '').split('-')
  if (!['csv', 'excel'].includes(format)) return
  const conn = props.conn
  const tb = violationsTable.value
  if (!conn || !tb) return

  if (scope === 'all') {
    violationsExporting.value = true
    try {
      // 后端把各规则的违规条件合成一条 SQL，直接交给通用导出任务
      const r = await aiQualityViolations({
        connectionId: conn.id,
        database: props.database,
        table: tb,
        offset: 0,
        limit: 1,
        withTotal: false,
        format
      })
      if (!r || !r.success || !r.exportSql) {
        ElMessage.warning(t('qa.noExportCond'))
        return
      }
      await exportTask.start(
        conn.id,
        { sql: r.exportSql, format, database: props.database },
        t('qa.violationsExportName', { table: tb }),
        format
      )
    } catch (e) {
      ElMessage.error(e?.message || t('qa.exportFailed'))
    } finally {
      violationsExporting.value = false
    }
    return
  }

  // 当前页
  violationsExporting.value = true
  try {
    const offset = (violationsPageNo.value - 1) * violationsPageSize.value
    const r = await aiQualityViolations({
      connectionId: conn.id,
      database: props.database,
      table: tb,
      offset,
      limit: violationsPageSize.value,
      format
    })
    if (!r || !r.success || !r.content) {
      ElMessage.error(r?.message || t('qa.exportFailed'))
      return
    }
    downloadText(r.content, r.filename || `quality-violations.${format === 'csv' ? 'csv' : 'xls'}`)
    ElMessage.success(t('qa.exportedRows', { n: r.rowCount }))
  } catch (e) {
    ElMessage.error(e?.message || t('qa.exportFailed'))
  } finally {
    violationsExporting.value = false
  }
}

/** 通用文本下载 */
const downloadText = (text, filename) => {
  const blob = new Blob([text], { type: 'text/csv;charset=utf-8' })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  a.download = filename
  document.body.appendChild(a)
  a.click()
  document.body.removeChild(a)
  URL.revokeObjectURL(a.href)
}

onMounted(() => { loadTypes(); loadConfigured(true) })

// 连接或库变化 → 重新读取已配置表并清空结果
watch(() => [props.conn?.id, props.database], async () => {
  scope.value = ''
  scanResult.value = null
  await loadConfigured(true)
})

// 上报状态给父组件（用于渲染弹窗底部的操作按钮）
watch(
  [scanning, scanRunning, exportingViolations, scanResult, configuredCount, () => props.disabled],
  () => {
    emit('state', {
      canScan: !props.disabled && configuredCount.value > 0 && !scanRunning.value,
      scanning: scanning.value,
      canExport: !!scanResult.value,
      exportingViolations: exportingViolations.value,
      tableCount: scanResult.value?.tableCount || 0,
      score: scanResult.value?.score
    })
  },
  { immediate: true }
)

/** 供父组件（弹窗底部）调用 */
defineExpose({
  scan,
  cancel: cancelScan,
  exportReport: openReport,
  exportViolations,
  refresh: () => loadConfigured(true)
})
</script>

<style scoped>
.qa { display: flex; flex-direction: column; }
/* 与下方内容区（空状态 / 进度 / 结果）留出间距，避免下拉紧贴下面的面板 */
.qa-bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin-bottom: 12px; }
.qa-scope { width: 250px; }
.qa-spacer { flex: 1; }
.qa-btn-ic { margin-right: 3px; vertical-align: -2px; }
.qa-meta { font-size: 11.5px; color: var(--dc-text-dim); }
.qa-meta b { color: var(--dc-primary); font-weight: 600; }
/* 提示已并入 .qa-empty（与「质量规则」一致：单块、撑满、居中），不再需要独立的 .qa-hint */

.qa-empty {
  padding: 22px; text-align: center; color: var(--dc-text-dim); font-size: 13px;
  /* 去掉虚线边框：与「质量规则」的提示块保持一致的无框观感 */
  background: var(--dc-bg-soft); border-radius: 10px;
  line-height: 1.8;
}
.qa-empty b { color: var(--dc-primary); }
/* 空状态里的跳转链接：与 <b> 同色系，但可点击 */
.qa-link {
  color: var(--dc-primary); font-weight: 600; cursor: pointer;
  border-bottom: 1px dashed currentColor; transition: opacity .16s ease;
}
.qa-link:hover { opacity: .78; }

/* 分析进度与实时日志 */
.qa-progress {
  display: flex; flex-direction: column; gap: 8px;
  padding: 12px; border: 1px solid var(--dc-border); border-radius: 10px;
  background: var(--dc-bg-soft);
}
.qa-progress-head { display: flex; align-items: center; gap: 10px; }
.qa-progress-bar { flex: 1; }
/* 分析进度条用主色渐变（组件内定义，不影响其它带状态色的进度条） */
.qa-progress-bar :deep(.el-progress-bar__inner) {
  background: linear-gradient(90deg, var(--dc-primary), var(--dc-purple));
}
.qa-progress-cur { font-size: 13px; color: var(--dc-text-dim); }
.qa-progress-cur b { color: var(--dc-text-strong); }
.qa-logs {
  max-height: 180px; overflow: auto; padding: 8px 10px;
  background: var(--dc-bg-card); border: 1px solid var(--dc-border); border-radius: 8px;
  font-family: 'SF Mono', ui-monospace, Consolas, monospace;
  font-size: 11.5px; line-height: 1.75;
}
.qa-log { white-space: pre-wrap; word-break: break-all; color: var(--dc-text-dim); }
.qa-log-time { color: var(--dc-text-weak); margin-right: 7px; }
.qa-log.lv-ok { color: #67c23a; }
.qa-log.lv-warn { color: var(--dc-warning, #e6a23c); }
.qa-log.lv-error { color: var(--dc-danger); }

/* 汇总 */
.qa-summary {
  display: flex; align-items: center; gap: 14px; padding: 12px 14px;
  border: 1px solid var(--dc-border); border-radius: 10px; background: var(--dc-bg-soft);
  margin-bottom: 10px;
}
.qa-score {
  font-size: 26px; font-weight: 700; line-height: 1; font-variant-numeric: tabular-nums;
  min-width: 58px; text-align: center;
}
.qa-score.good { color: #67c23a; }
.qa-score.warn { color: var(--dc-warning, #e6a23c); }
.qa-score.bad { color: var(--dc-danger); }
.qa-summary-body { flex: 1; min-width: 0; }
.qa-summary-title { font-size: 13px; font-weight: 600; color: var(--dc-text-strong); }
.qa-summary-meta { font-size: 11.5px; color: var(--dc-text-dim); margin-top: 2px; }
.qa-time { font-size: 12px; color: var(--dc-text-weak); }

/* 逐表明细：卡片高度随内容自适应（注意不要与弹窗表格的 .qa-table 混用类名）。
   自身不设 max-height / overflow —— 统一交给外层内容区（.gv-body）滚动，
   否则会出现「明细区滚动条 + 内容区滚动条」两条滚动条并存。 */
.qa-tbl-card {
  border: 1px solid var(--dc-border); border-radius: 9px;
  background: var(--dc-bg-soft); margin-bottom: 8px; overflow: hidden;
}
.qa-table-head {
  display: flex; align-items: center; gap: 10px; padding: 8px 12px;
  border-bottom: 1px solid var(--dc-border); background: var(--dc-bg-deep);
}
.qa-table-name { font-size: 13px; font-weight: 600; color: var(--dc-text); }
.qa-table-head.bad .qa-table-name { color: var(--dc-danger); }
.qa-table-info { font-size: 12px; color: var(--dc-text-dim); }
.qa-table-tag {
  font-size: 10.5px; font-weight: 600; padding: 1px 8px; border-radius: 999px;
  background: var(--dc-bg-card); color: #67c23a;
}
.qa-table-tag.bad { color: var(--dc-danger); }
.qa-table-msg { font-size: 11.5px; color: var(--dc-text-dim); padding: 6px 12px 0; }

.qa-row {
  display: flex; align-items: baseline; gap: 8px; padding: 4px 12px; font-size: 11.5px;
  color: var(--dc-text-mid);
}
.qa-row.error { color: var(--dc-warning, #e6a23c); }
.qa-badge {
  flex-shrink: 0; font-size: 10.5px; padding: 1px 7px; border-radius: 5px;
  background: var(--dc-bg-card); color: var(--dc-link);
}
.qa-row-type { flex-shrink: 0; color: var(--dc-text-dim); }
.qa-row-level {
  flex-shrink: 0; font-size: 11px; font-weight: 600; padding: 0 5px; border-radius: 4px;
  background: var(--dc-bg-card); color: var(--dc-text-dim);
}
.qa-row-level.high { color: var(--dc-danger); }
.qa-row-level.mid { color: var(--dc-warning, #e6a23c); }
.qa-row-level.low { color: var(--dc-link); }
.qa-row-msg { flex: 1; min-width: 0; }

/* 报告 / 错误数据弹窗：工具条、表格、分页 */
.qa-report-bar { display: flex; align-items: center; gap: 8px; margin-bottom: 10px; flex-wrap: wrap; }
.qa-report-meta { font-size: 13px; color: var(--dc-text-dim); }
.qa-report-meta b { color: var(--dc-primary); font-weight: 600; }
/* 表格给一个最小高度，让 loading 遮罩与空状态更明显 */
/* 高度由弹窗统一控制（.qa-sub-dialog），这里只负责宽度与占位外观 */
.qa-table { width: 100%; }
/* 加载占位：与表格同区域，避免加载时先闪出表头 */
.qa-loading-box {
  display: flex; align-items: center; justify-content: center; gap: 8px;
  color: var(--dc-text-dim); font-size: 14px;
  border: 1px solid var(--dc-border); border-radius: 8px;
  background: var(--dc-bg-soft);
}
/* 两个子弹窗的固定高度见全局样式（styles/index.css 的 .el-dialog.qa-sub-dialog）——
   append-to-body 让弹窗脱离组件样式作用域，scoped :deep() 无法命中。 */
.qa-pager { margin-top: 10px; justify-content: flex-end; }
.qa-report {
  margin: 0; max-height: 52vh; overflow: auto; white-space: pre-wrap; word-break: break-word;
  font-family: 'SF Mono', ui-monospace, Consolas, monospace; font-size: 11.5px; line-height: 1.7;
  color: var(--dc-code-text); background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border); border-radius: 8px; padding: 10px 12px;
}
</style>
