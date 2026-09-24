<template>
  <!-- 宽度与「数据同步」（SyncDialog）保持一致；top 仍留 10vh —— 对比比同步多出
       「过滤条件 / 比对键」两块，起点太靠上时容易把底部顶出屏幕 -->
  <el-dialog :model-value="modelValue" width="980px" top="10vh"
             :close-on-click-modal="false" append-to-body
             @update:model-value="$emit('update:modelValue', $event)"
             class="compare-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Switch /></el-icon></span>
        <span>{{ $t('cmp.title') }}</span>
        <span class="dlg-title-sub">{{ $t('cmp.subtitle') }}</span>
      </div>
    </template>
    <!-- ==================== 源 / 目标 选择 ==================== -->
    <div class="cmp-src-tgt">
      <div class="cmp-side">
        <div class="cmp-side-title">
          <span class="dot src"></span>{{ $t('cmp.source') }}
        </div>
        <el-select v-model="src.connectionId" size="small" @change="onSrcConnChange">
          <el-option v-for="c in connections" :key="c.id" :label="c.name + ' (' + c.type + ')'" :value="c.id" />
        </el-select>
        <div class="cmp-pair">
          <el-select v-model="src.database" size="small" @change="onSrcDbChange">
            <el-option v-for="d in srcDbs" :key="d" :label="d" :value="d" />
          </el-select>
          <el-select v-if="srcNeedSchema" v-model="src.schema" size="small" @change="loadSrcTables">
            <el-option v-for="s in srcSchemas" :key="s" :label="s" :value="s" />
          </el-select>
        </div>
        <el-select v-model="src.table" size="small" filterable>
          <el-option v-for="t in srcTables" :key="t.name" :label="t.name + (t.type === 'VIEW' ? $t('cmp.viewSuffix') : '')" :value="t.name" />
        </el-select>
      </div>
      <div class="cmp-arrow">
        <div class="arrow-ring"><span class="cmp-vs">VS</span></div>
      </div>
      <div class="cmp-side">
        <div class="cmp-side-title">
          <span class="dot tgt"></span>{{ $t('cmp.target') }}
        </div>
        <el-select v-model="tgt.connectionId" size="small" @change="onTgtConnChange">
          <el-option v-for="c in connections" :key="c.id" :label="c.name + ' (' + c.type + ')'" :value="c.id" />
        </el-select>
        <div class="cmp-pair">
          <el-select v-model="tgt.database" size="small" @change="onTgtDbChange">
            <el-option v-for="d in tgtDbs" :key="d" :label="d" :value="d" />
          </el-select>
          <el-select v-if="tgtNeedSchema" v-model="tgt.schema" size="small" @change="loadTgtTables">
            <el-option v-for="s in tgtSchemas" :key="s" :label="s" :value="s" />
          </el-select>
        </div>
        <el-select v-model="tgt.table" size="small" filterable>
          <el-option v-for="t in tgtTables" :key="t.name" :label="t.name + (t.type === 'VIEW' ? $t('cmp.viewSuffix') : '')" :value="t.name" />
        </el-select>
      </div>
    </div>

    <!-- ==================== 对比选项 ==================== -->
    <div class="cmp-opts">
      <div class="opt-row">
        <div class="opt-item">
          <span class="opt-label">{{ $t('cmp.scope') }}</span>
          <el-radio-group v-model="opts.compareMode" size="small">
            <el-radio-button value="both">{{ $t('cmp.scopeBoth') }}</el-radio-button>
            <el-radio-button value="structure">{{ $t('cmp.scopeStruct') }}</el-radio-button>
            <el-radio-button value="data">{{ $t('cmp.scopeData') }}</el-radio-button>
          </el-radio-group>
        </div>
      </div>
      <div class="opt-row" v-if="opts.compareMode !== 'structure'">
        <div class="opt-item">
          <span class="opt-label">{{ $t('cmp.filter') }}</span>
          <el-radio-group v-model="condMode" size="small">
            <el-radio-button value="visual">{{ $t('cmp.visual') }}</el-radio-button>
            <el-radio-button value="sql">{{ $t('cmp.advSql') }}</el-radio-button>
          </el-radio-group>
          <el-checkbox v-model="opts.sameCondition" size="small">{{ $t('cmp.sameCondition') }}</el-checkbox>
        </div>
      </div>
      <div class="opt-row filter-panel" v-if="opts.compareMode !== 'structure'">
        <template v-if="condMode === 'visual'">
          <div class="cond-block">
            <div class="cond-tag src">{{ $t('cmp.srcCond') }}</div>
            <ConditionBuilder :columns="opts.sameCondition ? commonCols : srcCols" v-model="srcCond" />
          </div>
          <div class="cond-block" v-if="!opts.sameCondition">
            <div class="cond-tag tgt">{{ $t('cmp.tgtCond') }}</div>
            <ConditionBuilder :columns="tgtCols" v-model="tgtCond" />
          </div>
        </template>
        <template v-else>
          <div class="cond-block">
            <div class="cond-tag src">{{ $t('cmp.srcCond') }}</div>
            <el-input v-model="srcSql" size="small" clearable
                      :placeholder="$t('cmp.wherePlaceholder')" style="flex: 1" />
          </div>
          <div class="cond-block" v-if="!opts.sameCondition">
            <div class="cond-tag tgt">{{ $t('cmp.tgtCond') }}</div>
            <el-input v-model="tgtSql" size="small" clearable
                      :placeholder="$t('cmp.tgtWherePlaceholder')" style="flex: 1" />
          </div>
        </template>
      </div>
      <div class="opt-row" v-if="opts.compareMode !== 'structure'">
        <div class="opt-item">
          <span class="opt-label">{{ $t('cmp.key') }}
            <el-tooltip :content="$t('cmp.keyTip')" placement="top">
              <el-icon class="q-ic"><QuestionFilled /></el-icon>
            </el-tooltip>
          </span>
          <el-select v-model="opts.keyColumns" multiple size="small" collapse-tags collapse-tags-tooltip
                     filterable clearable style="width: 320px">
            <el-option v-for="c in srcCols" :key="c.name" :label="c.name" :value="c.name" />
          </el-select>
        </div>
        <div class="opt-item">
          <span class="opt-label">{{ $t('cmp.sample') }}</span>
          <el-input-number v-model="opts.sampleLimit" :min="10" :max="500" :step="10" size="small" style="width: 120px" />
        </div>
      </div>
    </div>

    <!-- ==================== 对比结果 ==================== -->
    <!-- 只保留「失败」提示：成功时的统计卡与明细统一放在下面的进度弹窗里，
         原来的条件是 loading || !success，内层又要求 success，两者互斥 → 明细永远不会渲染 -->
    <div v-if="result && !result.success" class="cmp-result">
      <el-alert :title="result.message" type="error" show-icon :closable="false" />
    </div>

    <!-- ==================== 进度 + 日志 弹窗 ==================== -->
    <el-dialog v-model="showProgressDlg" width="980px" top="6vh"
               :close-on-click-modal="false" :close-on-press-escape="false"
               :before-close="onProgressBeforeClose"
               append-to-body class="progress-dialog" @closed="onProgressClosed">
      <template #header>
        <div class="dlg-title">
          <span class="dlg-title-ic"><el-icon :size="16"><Document /></el-icon></span>
          <span>{{ $t('cmp.progress') }}</span>
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
            <span>{{ $t('cmp.logs') }}</span>
          </div>
          <span class="log-count">{{ $t('mon.nRows', { n: compareLogs.length }) }}</span>
        </div>
        <div ref="logScroll" class="log-body">
          <div v-if="!compareLogs.length" class="log-empty">
            <el-icon :size="28"><Document /></el-icon>
            <span>{{ $t('cmp.waitStart') }}</span>
            <p>{{ $t('cmp.waitTip') }}</p>
          </div>
          <div v-for="(line, i) in compareLogs" :key="i" class="log-line" :class="logClass(line)">
            <span class="log-time">{{ line.time }}</span>
            <span class="log-text">{{ line.text }}</span>
          </div>
        </div>
      </div>

      <!-- 对比结果 -->
      <div v-if="result" class="pg-result">
        <el-alert v-if="!result.success" :title="result.message" type="error" show-icon :closable="false" />
        <template v-else>
          <!-- 统计卡即「导航」：点哪张就翻到对应的明细页签，不用自己去找 -->
          <div class="stat-grid">
            <div class="stat" :class="{ clickable: detailTab === 'struct' }" @click="goDetail('struct')">
              <span class="num">{{ result.structure?.sourceColumns ?? result.data?.sourceRows }}</span>
              <span class="label">{{ $t('cmp.srcLabel') }}{{ result.structure ? $t('cmp.cols') : $t('cmp.rows') }}</span>
            </div>
            <div class="stat" :class="{ clickable: detailTab === 'struct' }" @click="goDetail('struct')">
              <span class="num">{{ result.structure?.targetColumns ?? result.data?.targetRows }}</span>
              <span class="label">{{ $t('cmp.tgtLabel') }}{{ result.structure ? $t('cmp.cols') : $t('cmp.rows') }}</span>
            </div>
            <div class="stat ok" :class="{ clickable: detailTab === 'struct' }" v-if="result.structure" @click="goDetail('struct')">
              <span class="num">{{ result.structure.commonColumns.length }}</span>
              <span class="label">{{ $t('cmp.commonCols') }}</span>
            </div>
            <div class="stat warn" :class="{ clickable: detailTab === 'struct' }" v-if="result.structure" @click="goDetail('struct')">
              <span class="num">{{ result.structure.diffCount }}</span>
              <span class="label">{{ $t('cmp.structDiff') }}</span>
            </div>
            <div class="stat ok" :class="{ clickable: detailTab === 'diff' }" v-if="result.data" @click="goDetail('diff')">
              <span class="num">{{ result.data.same }}</span>
              <span class="label">{{ $t('cmp.sameRows') }}</span>
            </div>
            <div class="stat warn" :class="{ clickable: detailTab === 'diff' }" v-if="result.data" @click="goDetail('diff')">
              <span class="num">{{ result.data.different }}</span>
              <span class="label">{{ $t('cmp.valueDiff') }}</span>
            </div>
            <div class="stat danger" :class="{ clickable: detailTab === 'onlySource' }" v-if="result.data" @click="goDetail('onlySource')">
              <span class="num">{{ result.data.onlyInSource }}</span>
              <span class="label">{{ $t('cmp.onlySource') }}</span>
            </div>
            <div class="stat danger" :class="{ clickable: detailTab === 'onlyTarget' }" v-if="result.data" @click="goDetail('onlyTarget')">
              <span class="num">{{ result.data.onlyInTarget }}</span>
              <span class="label">{{ $t('cmp.onlyTarget') }}</span>
            </div>
          </div>

          <!-- 明细：统计卡只说明「差多少」，这里说明「差在哪」 -->
          <el-tabs v-model="detailTab" size="small" class="cmp-tabs pg-detail">
            <el-tab-pane v-if="result.structure" :label="$t('cmp.structDiffN', { n: result.structure.diffCount })" name="struct">
              <el-empty v-if="result.structure.diffCount === 0" :description="$t('cmp.structSame')" :image-size="60" />
              <div v-else class="struct-diff">
                <div v-if="result.structure.onlyInSource.length" class="sd-group">
                  <div class="sd-title danger"><el-icon><TopRight /></el-icon>{{ $t('cmp.onlySourceCols', { n: result.structure.onlyInSource.length }) }}</div>
                  <div class="sd-chips">
                    <span v-for="c in result.structure.onlyInSource" :key="'s' + c" class="sd-chip danger">{{ c }}</span>
                  </div>
                </div>
                <div v-if="result.structure.onlyInTarget.length" class="sd-group">
                  <div class="sd-title warn"><el-icon><BottomRight /></el-icon>{{ $t('cmp.onlyTargetCols', { n: result.structure.onlyInTarget.length }) }}</div>
                  <div class="sd-chips">
                    <span v-for="c in result.structure.onlyInTarget" :key="'t' + c" class="sd-chip warn">{{ c }}</span>
                  </div>
                </div>
                <div v-if="result.structure.typeDifferences.length" class="sd-group">
                  <div class="sd-title"><el-icon><RefreshRight /></el-icon>{{ $t('cmp.typeDiff', { n: result.structure.typeDifferences.length }) }}</div>
                  <div class="sd-rows">
                    <div v-for="d in result.structure.typeDifferences" :key="'ty' + d.column" class="sd-row">
                      <b>{{ d.column }}</b>
                      <span class="from">{{ d.sourceType }}</span>
                      <el-icon><Right /></el-icon>
                      <span class="to">{{ d.targetType }}</span>
                    </div>
                  </div>
                </div>
                <div v-if="result.structure.lengthDifferences.length" class="sd-group">
                  <div class="sd-title"><el-icon><ScaleToOriginal /></el-icon>{{ $t('cmp.lengthDiff', { n: result.structure.lengthDifferences.length }) }}</div>
                  <div class="sd-rows">
                    <div v-for="d in result.structure.lengthDifferences" :key="'le' + d.column" class="sd-row">
                      <b>{{ d.column }}</b>
                      <span class="from">{{ $t('cmp.length', { n: d.source }) }}</span>
                      <el-icon><Right /></el-icon>
                      <span class="to">{{ $t('cmp.length', { n: d.target }) }}</span>
                    </div>
                  </div>
                </div>
                <div v-if="result.structure.nullableDifferences.length" class="sd-group">
                  <div class="sd-title"><el-icon><CircleCheck /></el-icon>{{ $t('cmp.nullableDiff', { n: result.structure.nullableDifferences.length }) }}</div>
                  <div class="sd-rows">
                    <div v-for="d in result.structure.nullableDifferences" :key="'nu' + d.column" class="sd-row">
                      <b>{{ d.column }}</b>
                      <span class="from">{{ d.source ? $t('cmp.nullable') : $t('cmp.notNull') }}</span>
                      <el-icon><Right /></el-icon>
                      <span class="to">{{ d.target ? $t('cmp.nullable') : $t('cmp.notNull') }}</span>
                    </div>
                  </div>
                </div>
                <div v-if="result.structure.defaultDifferences.length" class="sd-group">
                  <div class="sd-title"><el-icon><SetUp /></el-icon>{{ $t('cmp.defaultDiff', { n: result.structure.defaultDifferences.length }) }}</div>
                  <div class="sd-rows">
                    <div v-for="d in result.structure.defaultDifferences" :key="'de' + d.column" class="sd-row">
                      <b>{{ d.column }}</b>
                      <span class="from">{{ d.source }}</span>
                      <el-icon><Right /></el-icon>
                      <span class="to">{{ d.target }}</span>
                    </div>
                  </div>
                </div>
              </div>
            </el-tab-pane>

            <template v-if="result.data">
              <el-tab-pane :label="$t('cmp.valueDiffN', { n: result.data.different })" name="diff">
                <el-table :data="result.data.diffSamples" size="small" border max-height="320" class="data-table">
                  <el-table-column type="index" label="#" width="45" />
                  <el-table-column v-for="c in tableCols(result.data.diffSamples)" :key="c" :prop="c" :label="c" min-width="140" show-overflow-tooltip />
                </el-table>
                <div v-if="result.data.diffTruncated" class="truncate-hint">{{ $t('cmp.truncatedWithTotal', { n: result.data.diffSamples.length, total: result.data.different }) }}</div>
              </el-tab-pane>
              <el-tab-pane :label="$t('cmp.onlySourceN', { n: result.data.onlyInSource })" name="onlySource">
                <el-table :data="result.data.onlyInSourceSamples" size="small" border max-height="320" class="data-table">
                  <el-table-column type="index" label="#" width="45" />
                  <el-table-column v-for="c in tableCols(result.data.onlyInSourceSamples)" :key="c" :prop="c" :label="c" min-width="140" show-overflow-tooltip />
                </el-table>
                <div v-if="result.data.onlyInSourceTruncated" class="truncate-hint">{{ $t('cmp.truncated', { n: result.data.onlyInSourceSamples.length }) }}</div>
              </el-tab-pane>
              <el-tab-pane :label="$t('cmp.onlyTargetN', { n: result.data.onlyInTarget })" name="onlyTarget">
                <el-table :data="result.data.onlyInTargetSamples" size="small" border max-height="320" class="data-table">
                  <el-table-column type="index" label="#" width="45" />
                  <el-table-column v-for="c in tableCols(result.data.onlyInTargetSamples)" :key="c" :prop="c" :label="c" min-width="140" show-overflow-tooltip />
                </el-table>
                <div v-if="result.data.onlyInTargetTruncated" class="truncate-hint">{{ $t('cmp.truncated', { n: result.data.onlyInTargetSamples.length }) }}</div>
              </el-tab-pane>
            </template>
          </el-tabs>
        </template>
      </div>

      <template #footer>
        <el-button v-if="loading" type="danger" plain :icon="VideoPause" :loading="stopping"
                   :disabled="stopping" @click="stopRun">{{ stopping ? $t('cmp.stopping') : $t('cmp.stop') }}</el-button>
        <el-button v-else @click="showProgressDlg = false">{{ $t('common.close') }}</el-button>
      </template>
    </el-dialog>

    <template #footer>
      <el-button @click="$emit('update:modelValue', false)">{{ $t('common.close') }}</el-button>
      <el-button v-if="loading" type="danger" plain :icon="VideoPause" @click="stopRun">{{ $t('cmp.stop') }}</el-button>
      <el-button type="primary" :icon="Switch" :disabled="loading" @click="run">{{ $t('cmp.start') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, onMounted, watch, computed, onBeforeUnmount } from 'vue'
import { ElMessage } from 'element-plus'
import { Switch, Right, TopRight, BottomRight, RefreshRight, ScaleToOriginal, CircleCheck, SetUp, VideoPause, Document, QuestionFilled } from '@element-plus/icons-vue'
import { listConnections, listDatabases, listSchemas, listTables, listColumns, getFeatures, compareData, compareTaskStatus, compareCancel } from '../../api'
import ConditionBuilder from '../../common/ConditionBuilder.vue'
import { buildConditionSql } from '../../utils/cond'
import { t } from '../../utils/i18n'

const props = defineProps({ modelValue: Boolean, conn: Object, database: String, tables: Array })
const emit = defineEmits(['update:modelValue'])

const connections = ref([])
const src = ref({ connectionId: '', database: '', schema: '', table: '' })
const tgt = ref({ connectionId: '', database: '', schema: '', table: '' })
const srcDbs = ref([])
const srcSchemas = ref([])
const srcTables = ref([])
const srcCols = ref([])
const tgtDbs = ref([])
const tgtSchemas = ref([])
const tgtTables = ref([])
const tgtCols = ref([])
const loading = ref(false)
const result = ref(null)
const taskId = ref('')
const opts = ref({ compareMode: 'both', keyColumns: [], sampleLimit: 100, sameCondition: true })
// 过滤条件：默认可视化构造；condMode=visual 用 srcCond/tgtCond，=sql 用 srcSql/tgtSql
const condMode = ref('visual')
const srcCond = ref({ logic: 'AND', items: [] })
const tgtCond = ref({ logic: 'AND', items: [] })
const srcSql = ref('')
const tgtSql = ref('')

// 进度弹窗
const showProgressDlg = ref(false)
const compareLogs = ref([])
const logScroll = ref(null)
const taskStatus = ref('')
const taskDone = ref(0)
const taskTotal = ref(0)
const taskCurrent = ref('')
/** 明细区的当前页签：stats 上的统计卡点一下就切到这里 */
const detailTab = ref('struct')

/**
 * 点统计卡跳到对应明细页签。
 * 顺带把明细区滚进视野 —— 弹窗里日志占了大半，光切页签很可能看不见。
 */
const goDetail = (name) => {
  if (!name) return
  detailTab.value = name
  nextTick(() => {
    document.querySelector('.pg-detail')?.scrollIntoView?.({ behavior: 'smooth', block: 'nearest' })
  })
}
const taskMessage = ref('')
let pollTimer = null
/** 「停止」请求已发出（按钮据此变成「正在停止…」并禁用，避免连点） */
const stopping = ref(false)
/** 兜底定时器：停止请求发出后若迟迟不达终态，15 秒后放开关闭（见 stopRun） */
let stopGuard = null

/**
 * 任务终态：`success` / `canceled` / `error` / `notfound`（任务过期）。
 *
 * 与 `SyncDialog` 是同一个坑：这里以前只认 success 与 error，而取消时后端给的是 **`canceled`**
 * （见后端 `tasks.rs` 的 FinishGuard），于是点了「停止对比」之后界面永远停在"对比中" ——
 * `loading` 不落回 false，进度对话框就一直被「请先点停止对比再关闭」挡着，**关不掉**。
 * （旁边那个 `else if (t.canceled)` 也一并删掉：状态里没有这个布尔字段，它从来没生效过。）
 */
const TERMINAL_STATUS = ['success', 'canceled', 'error', 'notfound']
const isTaskDone = computed(() => taskStatus.value === 'success')
/** 任务已经不再运行（成功 / 已取消 / 失败 / 过期）—— 界面据此放行「关闭」 */
const isTaskSettled = computed(() => TERMINAL_STATUS.includes(taskStatus.value))

/**
 * 进度条百分比。
 * 完成后**直接给 100**：各阶段粒度不同（结构对比是一次性算完的，后端给的分母就是 1），
 * 停在 done/total 的原始值上会让"已经算完"看起来像"卡在 0"。
 */
const progressPct = computed(() => {
  if (isTaskDone.value) return 100
  if (!taskTotal.value) return 0
  return Math.min(100, Math.round(taskDone.value / taskTotal.value * 100))
})

/** 进度文案：完成后说「对比完成」，而不是继续写「正在对比：…（0/1）」 */
const progressText = computed(() => {
  if (taskStatus.value === 'error') return taskMessage.value
  if (isTaskDone.value) return t('cmp.done')
  if (taskStatus.value === 'canceled') return t('cmp.stopped')
  if (taskStatus.value === 'notfound') return taskMessage.value || t('cmp.taskExpired')
  const count = taskTotal.value ? t('sync.progressParen', { done: taskDone.value, total: taskTotal.value }) : ''
  return t('cmp.running', { current: taskCurrent.value, count })
})

const logClass = (line) => {
  const t = line.text
  if (t.includes('失败') || t.includes('错误')) return 'err'
  if (t.includes('跳过') || t.includes('已取消')) return 'warn'
  if (t.includes('成功')) return 'ok'
  return ''
}

const nowTime = () => {
  const d = new Date()
  return `${String(d.getHours()).padStart(2,'0')}:${String(d.getMinutes()).padStart(2,'0')}:${String(d.getSeconds()).padStart(2,'0')}`
}

const mergeLogs = (serverLogs) => {
  if (!Array.isArray(serverLogs)) return
  const existing = new Set(compareLogs.value.map(l => l.text))
  let added = 0
  for (const text of serverLogs) {
    if (!existing.has(text)) {
      compareLogs.value.push({ time: nowTime(), text })
      added++
    }
  }
  if (added && logScroll.value) {
    setTimeout(() => { logScroll.value.scrollTop = logScroll.value.scrollHeight }, 50)
  }
}

/**
 * 进度弹窗右上角 X：运行中不直接关。
 * 关掉后轮询仍在继续、但 onProgressClosed 会清掉日志与结果，用户再打开就是空的。
 */
const onProgressBeforeClose = (done) => {
  if (loading.value) {
    ElMessage.warning(t('cmp.busyClose'))
    return
  }
  done()
}

const onProgressClosed = () => {
  detailTab.value = 'struct'   // 下次打开回到「结构差异」，避免停在上次的页签上让人以为没数据
  if (!loading.value) {
    compareLogs.value = []
    result.value = null
  }
}

const srcFeatures = ref({})
const tgtFeatures = ref({})
const srcNeedSchema = computed(() => srcFeatures.value.supportsSchema === true)
const tgtNeedSchema = computed(() => tgtFeatures.value.supportsSchema === true)

const tableCols = (rows) => {
  const r = rows && rows.length ? rows[0] : {}
  const keys = Object.keys(r)
  return keys.filter(k => k !== '__key').slice(0, 10)
}

// 源、目标公共列：当"目标同此条件"时，可视化字段下拉只列两侧都有的列，避免跨表字段缺失报错
const commonCols = computed(() => {
  const tNames = new Set(tgtCols.value.map(c => c.name))
  return srcCols.value.filter(c => tNames.has(c.name))
})

// 生成提交用的源/目标 WHERE 片段（可视化 → 结构模型转 SQL；高级SQL → 原文）
const genSource = () => condMode.value === 'visual'
  ? buildConditionSql(srcCond.value, opts.value.sameCondition ? commonCols.value : srcCols.value)
  : srcSql.value.trim()
const genTarget = () => {
  if (opts.value.sameCondition) return genSource()
  return condMode.value === 'visual'
    ? buildConditionSql(tgtCond.value, tgtCols.value)
    : tgtSql.value.trim()
}

const loadConnections = async () => {
  connections.value = await listConnections()
  if (props.conn) {
    src.value.connectionId = props.conn.id
    tgt.value.connectionId = props.conn.id
    src.value.database = props.database || ''
    await loadSrcDbs()
    if (props.tables?.length === 1) src.value.table = props.tables[0].name
  }
  if (!src.value.connectionId && connections.value.length) {
    src.value.connectionId = connections.value[0].id
    tgt.value.connectionId = connections.value[0].id
    await loadSrcDbs()
  }
  if (tgt.value.connectionId) await loadTgtDbs()
}

const onSrcConnChange = async () => {
  src.value.database = ''
  src.value.schema = ''
  src.value.table = ''
  srcDbs.value = []
  srcSchemas.value = []
  srcTables.value = []
  srcFeatures.value = {}
  await loadSrcDbs()
}
const onSrcDbChange = async () => {
  src.value.schema = ''
  src.value.table = ''
  srcSchemas.value = []
  srcTables.value = []
  srcCols.value = []
  opts.value.keyColumns = []
  if (srcNeedSchema.value) {
    try { srcSchemas.value = await listSchemas(src.value.connectionId, src.value.database) } catch (e) { srcSchemas.value = [] }
    if (srcSchemas.value.length) src.value.schema = srcSchemas.value[0]
  }
  await loadSrcTables()
}
const loadSrcDbs = async () => {
  if (!src.value.connectionId) return
  srcDbs.value = await listDatabases(src.value.connectionId)
  if (!srcDbs.value.includes(src.value.database)) src.value.database = srcDbs.value[0] || ''
  try { srcFeatures.value = await getFeatures(src.value.connectionId) } catch (e) { srcFeatures.value = {} }
  await onSrcDbChange()
}
const loadSrcTables = async () => {
  if (!src.value.connectionId || !src.value.database) { srcTables.value = []; srcCols.value = []; return }
  srcTables.value = await listTables(src.value.connectionId, src.value.database)
}
const loadSrcColumns = async () => {
  srcCols.value = []
  opts.value.keyColumns = []
  if (!src.value.connectionId || !src.value.database || !src.value.table) return
  try {
    srcCols.value = await listColumns(src.value.connectionId, src.value.database, src.value.table)
  } catch (e) {
    srcCols.value = []
  }
}
watch(() => src.value.table, () => loadSrcColumns())
watch(() => src.value.schema, () => { if (src.value.table) loadSrcColumns() })

const onTgtConnChange = async () => {
  tgt.value.database = ''
  tgt.value.schema = ''
  tgt.value.table = ''
  tgtDbs.value = []
  tgtSchemas.value = []
  tgtTables.value = []
  tgtFeatures.value = {}
  await loadTgtDbs()
}
const onTgtDbChange = async () => {
  tgt.value.schema = ''
  tgt.value.table = ''
  tgtSchemas.value = []
  tgtTables.value = []
  tgtCols.value = []
  if (tgtNeedSchema.value) {
    try { tgtSchemas.value = await listSchemas(tgt.value.connectionId, tgt.value.database) } catch (e) { tgtSchemas.value = [] }
    if (tgtSchemas.value.length) tgt.value.schema = tgtSchemas.value[0]
  }
  await loadTgtTables()
}
const loadTgtDbs = async () => {
  if (!tgt.value.connectionId) return
  tgtDbs.value = await listDatabases(tgt.value.connectionId)
  if (!tgtDbs.value.includes(tgt.value.database)) tgt.value.database = tgtDbs.value[0] || ''
  try { tgtFeatures.value = await getFeatures(tgt.value.connectionId) } catch (e) { tgtFeatures.value = {} }
  await onTgtDbChange()
}
const loadTgtTables = async () => {
  if (!tgt.value.connectionId || !tgt.value.database) { tgtTables.value = []; return }
  tgtTables.value = await listTables(tgt.value.connectionId, tgt.value.database)
}
const loadTgtColumns = async () => {
  tgtCols.value = []
  if (!tgt.value.connectionId || !tgt.value.database || !tgt.value.table) return
  try {
    tgtCols.value = await listColumns(tgt.value.connectionId, tgt.value.database, tgt.value.table)
  } catch (e) {
    tgtCols.value = []
  }
}
watch(() => tgt.value.table, () => loadTgtColumns())
watch(() => tgt.value.schema, () => { if (tgt.value.table) loadTgtColumns() })

const run = async () => {
  if (!src.value.connectionId || !src.value.table) return ElMessage.warning(t('cmp.pickSource'))
  if (!tgt.value.connectionId || !tgt.value.table) return ElMessage.warning(t('cmp.pickTarget'))
  loading.value = true
  result.value = null
  compareLogs.value = []
  taskStatus.value = ''
  taskDone.value = 0
  taskTotal.value = 0
  taskCurrent.value = t('cmp.preparing')
  taskMessage.value = ''
  detailTab.value = 'struct'
  showProgressDlg.value = true
  try {
    const resp = await compareData({
      sourceConnectionId: src.value.connectionId,
      sourceDatabase: src.value.database,
      sourceSchema: src.value.schema,
      sourceTable: src.value.table,
      targetConnectionId: tgt.value.connectionId,
      targetDatabase: tgt.value.database,
      targetSchema: tgt.value.schema,
      targetTable: tgt.value.table,
      compareMode: opts.value.compareMode,
      keyColumns: (opts.value.keyColumns || []).join(','),
      sampleLimit: opts.value.sampleLimit,
      sourceCondition: genSource(),
      targetCondition: genTarget()
    })
    if (!resp.success) throw new Error(resp.message || t('cmp.submitFailed'))
    taskId.value = resp.taskId
    pollTimer = setInterval(pollTask, 800)
  } catch (e) {
    ElMessage.error(e.message)
    loading.value = false
    showProgressDlg.value = false
  }
}

const pollTask = async () => {
  try {
    // 局部变量改名：本函数后面要用 t() 翻译，同名会把它遮蔽掉
    const st = await compareTaskStatus(taskId.value)
    taskStatus.value = st.status
    taskDone.value = st.done || 0
    taskTotal.value = st.total || taskTotal.value
    taskCurrent.value = st.current || ''
    mergeLogs(st.logs)
    // 只有**终态**才收摊。必须包含 canceled：取消后后端给的就是 canceled，
    // 漏掉它界面就永远停在"对比中"，对话框再也关不掉（见 TERMINAL_STATUS 的说明）。
    if (isTaskSettled.value) {
      stopPolling()
      if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
      loading.value = false
      stopping.value = false
      // 取消也把已经算出来的结果留下 —— 部分对比结果是有用的，不该因为"取消"就丢掉
      result.value = st.result || result.value
      if (st.status === 'success') {
        if (st.result && !st.result.success) ElMessage.error(st.result.message || t('cmp.failed'))
      } else if (st.status === 'canceled') {
        ElMessage.info(st.message || t('cmp.stopped'))
      } else if (st.status === 'notfound') {
        ElMessage.warning(st.message || t('cmp.taskExpiredLong'))
      } else {
        taskMessage.value = st.message || t('cmp.failed')
        ElMessage.error(taskMessage.value)
      }
    }
  } catch (e) {
    stopPolling()
    if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
    loading.value = false
    stopping.value = false
    ElMessage.error(t('cmp.statusFailed', { detail: e.message }))
  }
}

/**
 * 请求停止。
 *
 * 1. **立刻置 `stopping`**：按钮当场变成「正在停止…」并禁用，避免连点却"没动静"。
 * 2. **加一道兜底**：取消是协作式的，正常几秒内就到达 canceled；
 *    万一某个查询迟迟不回来，15 秒后也放开关闭 —— 否则就是一个"关不掉"的死窗口。
 */
const stopRun = async () => {
  if (stopping.value) return
  stopping.value = true
  try {
    await compareCancel(taskId.value)
    ElMessage.info(t('cmp.stopRequested'))
    if (stopGuard) clearTimeout(stopGuard)
    stopGuard = setTimeout(() => {
      if (loading.value) {
        stopPolling()
        loading.value = false
        stopping.value = false
        ElMessage.warning(t('cmp.closingAllowed'))
      }
    }, 15000)
  } catch (e) {
    stopping.value = false
    ElMessage.error(e.message)
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
.cmp-src-tgt {
  display: flex;
  align-items: stretch;
  gap: 14px;
  margin-bottom: 16px;
}
.cmp-side {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 10px;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  border-radius: 12px;
  padding: 14px 16px;
}
.cmp-side-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
  color: var(--dc-text);
  margin-bottom: 2px;
}
.dot { width: 10px; height: 10px; border-radius: 50%; display: inline-block; }
.dot.src { background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep)); box-shadow: 0 0 8px var(--dc-primary-glow); }
.dot.tgt { background: linear-gradient(135deg, var(--dc-accent), var(--dc-primary-light)); box-shadow: 0 0 8px rgba(61,220,151,.35); }
.cmp-pair { display: flex; gap: 8px; }
.cmp-pair .el-select { flex: 1; }
.cmp-arrow {
  display: flex; align-items: center; justify-content: center;
  min-width: 44px;
}
.cmp-arrow .arrow-ring {
  width: 40px; height: 40px; border-radius: 50%;
  display: flex; align-items: center; justify-content: center;
  color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-accent), var(--dc-primary-light));
  box-shadow: 0 0 0 4px rgba(61, 220, 151, .12), 0 4px 14px rgba(61, 220, 151, .25);
}
.cmp-arrow .cmp-vs {
  font-size: 14px; font-weight: 800;
  letter-spacing: 1px;
  color: var(--dc-on-primary);
}

.cmp-opts {
  display: flex;
  flex-direction: column;
  gap: 10px;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  border-radius: 12px;
  padding: 12px 16px;
  margin-bottom: 16px;
}
.opt-row { display: flex; flex-wrap: wrap; align-items: center; gap: 10px 24px; }
.opt-item { display: flex; align-items: center; gap: 10px; }
.opt-label { font-size: 13px; color: var(--dc-text-dim); white-space: nowrap; display: inline-flex; align-items: center; }
.q-ic { color: var(--dc-text-dim); cursor: help; margin-left: 3px; font-size: 13px; }
.opt-row.filter-panel { align-items: flex-start; gap: 16px; }
.cond-block {
  flex: 1; min-width: 300px;
  display: flex; flex-direction: column; gap: 6px;
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  padding: 8px 10px;
}
.cond-tag { font-size: 12px; font-weight: 600; color: var(--dc-text-dim); display: inline-flex; align-items: center; }
.cond-tag::before { content: ''; display: inline-block; width: 8px; height: 8px; border-radius: 2px; margin-right: 6px; }
.cond-tag.src::before { background: var(--dc-primary); }
.cond-tag.tgt::before { background: var(--dc-accent); }

.cmp-result { min-height: 0; }

.stat-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(110px, 1fr));
  gap: 10px;
  margin-bottom: 14px;
}
.stat {
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  border-radius: 10px;
  padding: 12px 8px;
  text-align: center;
  transition: border-color .2s;
}
.stat:hover { border-color: var(--dc-border-hover); }
.stat .num { display: block; font-size: 22px; font-weight: 700; color: var(--dc-primary); }
.stat .label { font-size: 11.5px; color: var(--dc-text-dim); margin-top: 4px; display: block; }
.stat.ok .num { color: var(--dc-success); }
.stat.warn .num { color: var(--dc-warning); }
.stat.danger .num { color: var(--dc-danger); }

.cmp-tabs :deep(.el-tabs__header) { margin-bottom: 10px; }

.struct-diff { max-height: 400px; overflow: auto; display: flex; flex-direction: column; gap: 12px; }
.sd-group { border: 1px solid var(--dc-border); border-radius: 8px; padding: 8px 10px; background: var(--dc-bg-soft); }
.sd-title { display: flex; align-items: center; gap: 5px; font-size: 13px; font-weight: 600; margin-bottom: 6px; color: var(--dc-text); }
.sd-title.danger { color: var(--dc-danger); }
.sd-title.warn { color: var(--dc-warning); }
.sd-chips { display: flex; flex-wrap: wrap; gap: 5px; }
.sd-chip {
  font-size: 13px; padding: 2px 10px; border-radius: 12px;
  background: var(--dc-danger-wash); color: var(--dc-danger); border: 1px solid var(--dc-danger);
}
.sd-chip.warn { background: var(--dc-warning-wash); color: var(--dc-warning); border-color: var(--dc-warning); }
.sd-rows { display: flex; flex-direction: column; gap: 4px; }
.sd-row { display: flex; align-items: center; gap: 6px; font-size: 13px; }
.sd-row b { color: var(--dc-text); min-width: 110px; }
.sd-row .from { color: var(--dc-danger); }
.sd-row .to { color: var(--dc-success); }
.sd-row .el-icon { font-size: 13px; color: var(--dc-text-dim); }

.data-table { width: 100%; }
.truncate-hint { font-size: 13px; color: var(--dc-text-dim); padding: 8px 4px; }

.compare-dialog :deep(.el-dialog) { margin-bottom: 8vh; }
.compare-dialog :deep(.el-dialog__body) { padding-top: 10px; padding-bottom: 6px; }

.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px rgba(79, 140, 255, .35);
}
.dlg-title-sub { font-size: 13px; font-weight: 400; color: var(--dc-text-dim); margin-left: 4px; }

/* ==================== 进度弹窗 + 日志 ==================== */
/* 明细摊开后内容会明显变长，给弹窗体设个上限并允许滚动，避免把窗口顶出屏幕 */
.progress-dialog :deep(.el-dialog__body) { padding: 16px 20px 10px; max-height: 74vh; overflow: auto; }
.pg-summary { margin-bottom: 14px; }
.pg-text { font-size: 13px; color: var(--dc-text-dim); margin-top: 8px; text-align: center; }
.pg-log { margin-bottom: 0; }
/* 日志原来占 420px，明细加进来后要给它让位：日志是过程记录，明细才是用户要看的结论 */
.pg-log .log-body { max-height: 150px; }
.pg-result { margin-top: 14px; }

/* 统计卡兼作明细导航：可点击的给出手型与悬停反馈，不可点的就保持静态 */
.stat.clickable { cursor: pointer; }
.stat.clickable:hover { border-color: var(--dc-primary); }
.pg-detail { margin-top: 4px; }
.pg-detail :deep(.el-tabs__header) { margin-bottom: 8px; }

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
</style>
