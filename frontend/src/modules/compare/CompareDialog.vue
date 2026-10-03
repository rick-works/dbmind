<template>
  <!-- 宽度与「数据同步」（SyncDialog）保持一致；top 仍留 10vh —— 对比比同步多出
       「过滤条件 / 比对键」两块，起点太靠上时容易把底部顶出屏幕 -->
  <el-dialog :model-value="modelValue && !wizardHidden" width="980px" top="10vh"
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
    <!-- ==================== 步骤条：与数据传输一致的向导式交互 ==================== -->
    <el-steps :active="step - 1" align-center finish-status="success" class="cmp-steps">
      <el-step :title="$t('cmp.stepSrc')" />
      <el-step :title="$t('cmp.stepOpts')" />
      <el-step :title="$t('cmp.stepRun')" />
    </el-steps>

    <!-- ==================== 步骤 1：源 / 目标 选择 ==================== -->
    <div v-show="step === 1" class="cmp-src-tgt">
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

    <!-- ==================== 步骤 2：对比选项 ==================== -->
    <div v-show="step === 2" class="cmp-opts">
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
            <!-- 全选/清空：字段多时一个个点太累（用户反馈）。__all__ 是哨兵值，不参与真实比对键 -->
            <el-option :label="$t('cmp.allColumns')" value="__all__">
              <span style="display:flex;justify-content:space-between;align-items:center;width:100%">
                <span>{{ $t('cmp.allColumns') }}</span>
                <el-icon v-if="allKeysSelected" style="color:var(--el-color-primary)"><Check /></el-icon>
              </span>
            </el-option>
          </el-select>
        </div>
        <!-- 「采样条数」输入已删：对比就是**全量数据对比**（流式分页拉取，无行数限制）；
             结果页里的差异数据仍是采样展示（样本数由后端定，无需用户关心） -->
      </div>
    </div>

    <!-- ==================== 步骤 3：摘要确认（两列表格，与数据传输摘要一致） ==================== -->
    <div v-if="step === 3" class="cmp-summary">
      <el-table :data="summaryRows" size="small" border class="data-table">
        <el-table-column prop="label" width="130" />
        <el-table-column prop="value" show-overflow-tooltip />
      </el-table>
    </div>

    <!-- ==================== 对比结果（失败提示）==================== -->
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
          <!-- 已完成任务的「查看」直接显示结果，标题也跟着叫「对比结果」 -->
          <span>{{ resumedViewOnly ? $t('cmp.title') : $t('cmp.progress') }}</span>
        </div>
      </template>
      <!-- 进度条/日志只在**运行中**显示：已完成任务的查看（resumedViewOnly）
           跳过这些中间视图，直接看结果 —— 进度条+空日志对它是噪音（真机反馈） -->
      <div class="pg-summary">
        <!-- 状态色：失败=红；取消=黄（不是"完成"，但也不该报成错误）；完成=绿。
             总量未知（读取阶段）时用**流动动画**表示进行中，不显示假的 100% -->
        <el-progress :percentage="progressPct" :stroke-width="14" text-inside
                     :indeterminate="taskTotal <= 0 && !isTaskSettled"
                     :duration="2"
                     :status="taskStatus === 'error' ? 'exception'
                              : (isTaskDone ? 'success' : (taskStatus === 'canceled' ? 'warning' : ''))" />
        <div v-if="progressText" class="pg-text">{{ progressText }}</div>
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
          <!-- 只显示「执行到哪一步」：时间列与数据传输对齐去掉 -->
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
              <span class="num">{{ fmtNum(result.structure?.sourceColumns ?? result.data?.sourceRows) }}</span>
              <span class="label">{{ $t('cmp.srcLabel') }}{{ result.structure ? $t('cmp.cols') : $t('cmp.rows') }}</span>
            </div>
            <div class="stat" :class="{ clickable: detailTab === 'struct' }" @click="goDetail('struct')">
              <span class="num">{{ fmtNum(result.structure?.targetColumns ?? result.data?.targetRows) }}</span>
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
              <span class="num">{{ fmtNum(result.data.same) }}</span>
              <span class="label">{{ $t('cmp.sameRows') }}</span>
            </div>
            <div class="stat warn" :class="{ clickable: detailTab === 'diff' }" v-if="result.data" @click="goDetail('diff')">
              <span class="num">{{ fmtNum(result.data.different) }}</span>
              <span class="label">{{ $t('cmp.valueDiff') }}</span>
            </div>
            <div class="stat danger" :class="{ clickable: detailTab === 'onlySource' }" v-if="result.data" @click="goDetail('onlySource')">
              <span class="num">{{ fmtNum(result.data.onlyInSource) }}</span>
              <span class="label">{{ $t('cmp.onlySource') }}</span>
            </div>
            <div class="stat danger" :class="{ clickable: detailTab === 'onlyTarget' }" v-if="result.data" @click="goDetail('onlyTarget')">
              <span class="num">{{ fmtNum(result.data.onlyInTarget) }}</span>
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
                  <!-- 序号列只在有数据时渲染：空表只剩一个孤零零的「#」表头很怪（真机反馈） -->
                  <el-table-column v-if="result.data.diffSamples.length" type="index" label="#" width="45" />
                  <el-table-column v-for="c in tableCols(result.data.diffSamples)" :key="c" :prop="c" :label="c" min-width="140" show-overflow-tooltip />
                </el-table>
                <div v-if="result.data.diffTruncated" class="truncate-hint">{{ $t('cmp.truncatedWithTotal', { n: result.data.diffSamples.length, total: result.data.different }) }}</div>
              </el-tab-pane>
              <el-tab-pane :label="$t('cmp.onlySourceN', { n: result.data.onlyInSource })" name="onlySource">
                <el-table :data="result.data.onlyInSourceSamples" size="small" border max-height="320" class="data-table">
                  <el-table-column v-if="result.data.onlyInSourceSamples.length" type="index" label="#" width="45" />
                  <el-table-column v-for="c in tableCols(result.data.onlyInSourceSamples)" :key="c" :prop="c" :label="c" min-width="140" show-overflow-tooltip />
                </el-table>
                <div v-if="result.data.onlyInSourceTruncated" class="truncate-hint">{{ $t('cmp.truncated', { n: result.data.onlyInSourceSamples.length }) }}</div>
              </el-tab-pane>
              <el-tab-pane :label="$t('cmp.onlyTargetN', { n: result.data.onlyInTarget })" name="onlyTarget">
                <el-table :data="result.data.onlyInTargetSamples" size="small" border max-height="320" class="data-table">
                  <el-table-column v-if="result.data.onlyInTargetSamples.length" type="index" label="#" width="45" />
                  <el-table-column v-for="c in tableCols(result.data.onlyInTargetSamples)" :key="c" :prop="c" :label="c" min-width="140" show-overflow-tooltip />
                </el-table>
                <div v-if="result.data.onlyInTargetTruncated" class="truncate-hint">{{ $t('cmp.truncated', { n: result.data.onlyInTargetSamples.length }) }}</div>
              </el-tab-pane>
            </template>
          </el-tabs>
        </template>
      </div>

      <template #footer>
        <!-- 「后台运行」：收起对话框，任务在后端继续跑，顶栏任务中心随时找回（与数据传输一致） -->
        <el-button v-if="loading" :icon="Clock" @click="bgRunCompare">{{ $t('sync.runBg') }}</el-button>
        <el-button v-if="loading" type="danger" plain :icon="VideoPause" :loading="stopping"
                   :disabled="stopping" @click="stopRun">{{ stopping ? $t('cmp.stopping') : $t('cmp.stop') }}</el-button>
        <!-- 关闭进度窗：若是「查看」场景（向导已隐藏）→ 对话框整体关闭；下次打开是新向导 -->
        <el-button v-else @click="closeProgress">{{ $t('common.close') }}</el-button>
      </template>
    </el-dialog>

    <template #footer>
      <el-button @click="$emit('update:modelValue', false)">{{ $t('common.cancel') }}</el-button>
      <!-- 停止按钮只属于**进度窗**：后台运行残留的 loading 不能把「停止」漏进向导
           footer（点了会误取消还在跑的旧任务，审查发现） -->
      <el-button v-if="loading && showProgressDlg" type="danger" plain :icon="VideoPause" @click="stopRun">{{ $t('cmp.stop') }}</el-button>
      <el-button v-if="step > 1 && !loading" @click="step--">{{ $t('common.prev') }}</el-button>
      <el-button v-if="step < 3" type="primary" :icon="Right" @click="nextStep">{{ $t('common.next') }}</el-button>
      <el-button v-if="step === 3" type="primary" :icon="Switch" :disabled="loading" @click="run">{{ $t('cmp.start') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, onMounted, watch, computed, onBeforeUnmount, nextTick } from 'vue'
import { ElMessage } from 'element-plus'
import { Switch, Right, TopRight, BottomRight, RefreshRight, ScaleToOriginal, Check, CircleCheck, SetUp, VideoPause, Document, Clock, QuestionFilled } from '@element-plus/icons-vue'
import { addBgTask, bgTasks } from '../sync/backgroundTasks'
import { listConnections, listDatabases, listSchemas, listTables, listColumns, getFeatures, compareData, compareTaskStatus, compareCancel } from '../../api'
import ConditionBuilder from '../../common/ConditionBuilder.vue'
import { buildConditionSql } from '../../utils/cond'
import { t } from '../../utils/i18n'

const props = defineProps({ modelValue: Boolean, conn: Object, database: String, tables: Array, resumeId: String })
const emit = defineEmits(['update:modelValue'])

// ===== 向导步骤（与数据传输一致的「下一步」式交互）=====
const step = ref(1)
/** **向导隐藏**：从任务中心「查看」进来时只显示进度/结果窗（后面的向导框不渲染）；
    进度窗关闭时复位 —— 对话框整体随之关闭（emit false），下次打开是新向导 */
const wizardHidden = ref(false)
/** **结果直显模式**：任务中心「查看」已终态的记录时置真 —— 进度窗里隐藏进度条/日志，
    直接渲染结果区（统计卡+明细）。轮询路径/重新打开向导都会复位。 */
const resumedViewOnly = ref(false)
// **下一步**校验：步骤 1 必须双侧都选全（连接/库/表），不满足就留在原地提示
const nextStep = () => {
  if (step.value === 1) {
    if (!src.value.connectionId || !src.value.database || !src.value.table) return ElMessage.warning(t('cmp.pickSource'))
    if (!tgt.value.connectionId || !tgt.value.database || !tgt.value.table) return ElMessage.warning(t('cmp.pickTarget'))
  }
  step.value++
}
const sideLabel = (side, dbs, schemas) => {
  const conn = connections.value.find(c => String(c.id) === String(side.connectionId))
  const parts = [conn ? conn.name : '', side.database || '']
  if (side.schema) parts.push(side.schema)
  parts.push(side.table || '')
  return parts.filter(Boolean).join('.')
}
const srcLabel = computed(() => sideLabel(src.value))
const tgtLabel = computed(() => sideLabel(tgt.value))
const scopeLabel = computed(() => ({
  both: t('cmp.scopeBoth'), structure: t('cmp.scopeStruct'), data: t('cmp.scopeData')
}[opts.value?.compareMode] || opts.value.compareMode))
/** 步骤 3 摘要行（两列表格，与数据传输的摘要同一形态） */
const summaryRows = computed(() => {
  const rows = [
    { label: t('cmp.source'), value: srcLabel.value },
    { label: t('cmp.target'), value: tgtLabel.value },
    { label: t('cmp.scope'), value: scopeLabel.value }
  ]
  if (opts.value.compareMode !== 'structure') {
    rows.push({ label: t('cmp.key'), value: opts.value.keyColumns.length ? opts.value.keyColumns.join(', ') : t('cmp.keyAuto') })
    if (genSource()) rows.push({ label: t('cmp.srcCond'), value: genSource() })
    if (!opts.value.sameCondition && genTarget()) rows.push({ label: t('cmp.tgtCond'), value: genTarget() })
  }
  return rows
})

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
// 总量未知（读取阶段）：给一个流动条的基准宽度（indeterminate 动画用），
// 不是真实进度 —— 真实进度在 total 确定后（两侧行数已知）才出现
if (isTaskDone.value) return 100
if (!taskTotal.value) return 60
return Math.min(100, Math.round(taskDone.value / taskTotal.value * 100))
})

/** 进度文案：完成后说「对比完成」，而不是继续写「正在对比：…（0/1）」 */
const progressText = computed(() => {
// 失败/中断的**详情在日志里**（异常文本很长，塞在进度条上会把整个弹窗顶乱）——
// 进度条上只给一句话结论，用户要看细节往日志里找
if (taskStatus.value === 'error') return t('cmp.failed')
// 完成也**不显示文案**：进度条走满变绿就是「完成」的结论（真机反馈：「对比完成」删掉）
if (isTaskDone.value) return ''
if (taskStatus.value === 'canceled') return t('cmp.stopped')
if (taskStatus.value === 'notfound') return t('cmp.taskExpired')
// 运行中**不显示文案**：进度条本身就是状态，过程细节（表名、行数）在日志里
//（真机反馈：「正在对比…」这句删掉）
return ''
})

const logClass = (line) => {
  const t = line.text
  if (t.includes('失败') || t.includes('错误')) return 'err'
  if (t.includes('跳过') || t.includes('已取消')) return 'warn'
  if (t.includes('成功')) return 'ok'
  return ''
}

/** 统计卡数字千分位（与数据传输一致） */
const fmtNum = (n) => (typeof n === 'number' ? n.toLocaleString() : n)
const nowTime = () => {
  // **带日期时间（含年份）**：与数据传输一致，执行记录跨年可回溯
  const d = new Date()
  const p2 = (n) => String(n).padStart(2, '0')
  const hm = `${p2(d.getHours())}:${p2(d.getMinutes())}:${p2(d.getSeconds())}`
  return `${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())} ${hm}`
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
  // **X 关闭也要复位「查看」场景的隐藏态**：右上角 X 不走 closeProgress 按钮 ——
  // 漏了复位的话 wizardHidden / 主对话框打开态 / resumeId 全残留，任务中心
  // 再点**同一条**任务时 resumeId 值没变 → watch 不触发 → 「查看」看起来没反应（真机踩过）
  if (wizardHidden.value) {
    wizardHidden.value = false
    resumedViewOnly.value = false
    emit('update:modelValue', false)
  }
}

const srcFeatures = ref({})
const tgtFeatures = ref({})
// ⚠ 后端 features 的键是 **supportsSchemas**（复数）—— 写成单数永远 undefined，
// SQL Server / PG 的模式下拉就从不出现（数据传输修过同一个坑，这里漏了）
const srcNeedSchema = computed(() => srcFeatures.value.supportsSchemas === true)
const tgtNeedSchema = computed(() => tgtFeatures.value.supportsSchemas === true)

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
  // 只有从外部带着明确上下文进来（首页快速开始等传 props.conn）才预填；
  // 顶栏直接打开时**连连接也不预选**（与数据传输同一口径）——
  // 之前这里会自动选中第一个连接并加载库列表，用户要求对齐改成全空自己挑
  if (props.conn) {
    src.value.connectionId = props.conn.id
    tgt.value.connectionId = props.conn.id
    await loadSrcDbs()
    // 预填库必须在清单里才保留（清单是 internal 裸名；树曾给过 `internal.ods`
    // 这类全限定名 —— 不在清单就丢掉，别让无效值带进后续请求）
    if (!srcDbs.value.includes(props.database || '')) src.value.database = ''
    // 表预填以**库已有效**为前提：库被上面清掉时还填表，会造出「库空、表已选」的
    // 非法组合，下一步校验都拦不住它进 run 请求
    if (src.value.database && props.tables?.length === 1) src.value.table = props.tables[0].name
  }
  if (tgt.value.connectionId) await loadTgtDbs()
}

/** 比对键「全选」：已全选时点击 = 清空；否则选中全部字段。
 *  __all__ 哨兵值会混进 opts.keyColumns —— watch 里立即剔除（不能进比对请求） */
const allKeysSelected = computed(() => srcCols.value.length > 0 && opts.value.keyColumns.length >= srcCols.value.length)
const toggleAllKeys = () => {
  opts.value.keyColumns = allKeysSelected.value ? [] : srcCols.value.map(c => c.name)
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
  // 不自动预选（与数据传输同一口径）：库留空由用户自己挑，系统库排第一时
  // 自动选中它反而掩盖真实业务库
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
  // 同源：不自动预选
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
// 比对键里混入「全选」哨兵值时剔除（el-option value=__all__）
watch(() => opts.value.keyColumns, (keys) => {
  const real = srcCols.value.map(c => c.name)
  const clean = keys.filter(k => real.includes(k))
  if (clean.length !== keys.length) opts.value.keyColumns = clean
}, { deep: true })
watch(() => tgt.value.table, () => loadTgtColumns())
watch(() => tgt.value.schema, () => { if (tgt.value.table) loadTgtColumns() })

const run = async () => {
  // 校验口径与 nextStep 一致（含库）：步骤校验可能被预填组合绕过，防线放这里兜底
  if (!src.value.connectionId || !src.value.database || !src.value.table) return ElMessage.warning(t('cmp.pickSource'))
  if (!tgt.value.connectionId || !tgt.value.database || !tgt.value.table) return ElMessage.warning(t('cmp.pickTarget'))
  loading.value = true
  result.value = null
  compareLogs.value = []
  taskStatus.value = ''
  taskDone.value = 0
  taskTotal.value = 0
  taskCurrent.value = t('cmp.preparing')
  taskMessage.value = ''
  resumedViewOnly.value = false
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
    // **提交即登记**任务中心（与数据传输一致）：执行记录不依赖用户点「后台运行」
    addBgTask({
      id: taskId.value,
      kind: 'compare',
      title: t('cmp.bgTitle', {
        src: (src.value.database || '') + '.' + (src.value.table || ''),
        tgt: (tgt.value.database || '') + '.' + (tgt.value.table || '')
      })
    })
    pollTimer = setInterval(pollTask, 800)
    // **任务开始后收起向导**：进度窗已在最前（append-to-body），后面的向导框留着挡视线
    emit('update:modelValue', false)
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
    // total=-1 = 分母未知（count 失败兜底）：不覆盖成 -1 —— 否则进度百分比算出负数
    taskTotal.value = st.total > 0 ? st.total : taskTotal.value
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
        // 后端任务不在了：**本地快照兜底**（任务中心记录里存过终态结果就照常展示，
        // 用户口径：记录不删就保留；本地也没有才提示已中断）
        const rec = bgTasks.find(x => x.id === taskId.value)
        const snap = rec && rec.snapshot
        // 同上：优先记录顶层 status（存量留底快照里没有 status 字段）
        const recStatus = (rec && rec.status) || (snap && snap.status)
        if (recStatus && recStatus !== 'running') {
          taskDone.value = snap.rowsRead || 0
          taskMessage.value = snap.message || ''
          result.value = snap.result || result.value
          taskStatus.value = recStatus
          ElMessage.info(t('sync.bgRestoredLocal'))
        } else {
          taskMessage.value = t('sync.bgInterrupted')
          taskCurrent.value = taskMessage.value
          ElMessage.warning(taskMessage.value)
        }
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

// ===== 后台运行 / 恢复（与数据传输同一套机制）=====
// 关闭进度窗：向导处于隐藏态（任务中心「查看」进来的）→ 对话框整体关掉并复位隐藏标记；
// 正常向导流程的进度窗关闭则不动主对话框（用户还能回步骤里调整）
const closeProgress = () => {
  showProgressDlg.value = false
  if (wizardHidden.value) {
    wizardHidden.value = false
    emit('update:modelValue', false)
  }
}
// **解除停止兜底守卫**：后台运行/恢复查看时，之前点过「停止」的 15 秒兜底定时器
// 必须清掉 —— 否则它到期看到 loading 还在，会把还在跑的任务当卡死强行收摊
//（停止按钮也永远卡在「正在停止…」）
const disarmStopGuard = () => {
  if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
  stopping.value = false
}
// 「后台运行」：任务在后端继续跑（后端任务本来独立于对话框），这里只是收起窗口
// 并到任务中心登记，顶栏随时找回。
const bgRunCompare = () => {
  if (!taskId.value) return
  addBgTask({
    id: taskId.value,
    kind: 'compare',
    title: t('cmp.bgTitle', {
      src: (src.value.database || '') + '.' + (src.value.table || ''),
      tgt: (tgt.value.database || '') + '.' + (tgt.value.table || '')
    })
  })
  stopPolling()
  disarmStopGuard()
  showProgressDlg.value = false
  emit('update:modelValue', false)
  ElMessage.success(t('sync.bgAdded'))
}
// 从任务中心恢复：拿登记的 taskId 直接回到进度窗接着轮询（不重新提交对比）。
// ⚠ 不能因为「id === 当前 taskId」就早退 —— 查看刚跑完的任务时组件里 taskId
// 本来就是这个值，早退会让进度窗根本不打开，用户只看到向导第一步（真机踩过）
watch(() => props.resumeId, async (id) => {
  if (!id) return
  // **同帧竞态**：任务中心点「查看」时 modelValue 与 resumeId 同帧变 true ——
  // 「打开即重置向导」的 watch 注册在后面、反而后跑，会把这里设好的
  // 进度窗/隐藏态覆盖回第一步向导（真机踩过：查看弹出的是向导第一步）。
  // 等 nextTick 让重置跑完再设置（与 SyncDialog 的恢复同方案）
  await nextTick()
  if (id !== props.resumeId) return
  taskId.value = id
  detailTab.value = 'struct'
  // **已终态的记录直接显示结果**（本地快照留底）：进度条+空日志对完成的任务是噪音 ——
  // 用户点「查看」要的是结果本身（真机反馈）。结果区（统计卡+明细）直接渲染。
  const rec = bgTasks.find(x => x.id === id)
  const snap = rec && rec.snapshot
  // 终态判定用**记录顶层 status**（持久化、存量留底也有）—— 快照里的 status 是
  // 后来才补存的字段，老的留底数据里没有，靠它判断会把已完成全漏成「运行中」
  const recStatus = (rec && rec.status) || (snap && snap.status)
  if (recStatus && recStatus !== 'running' && recStatus !== 'notfound') {
    resumedViewOnly.value = true
    taskStatus.value = recStatus
    taskDone.value = snap.rowsRead || 0
    taskMessage.value = snap.message || ''
    result.value = snap.result || null
    loading.value = false
    wizardHidden.value = true
    showProgressDlg.value = true
    stopPolling()
    // 本地快照 result 超 100KB 被裁过 → 从后端补全（磁盘快照在服务重启后也在）
    compareTaskStatus(id).then(st => {
      if (st?.result) result.value = st.result
      if (st?.status && st.status !== 'notfound') taskStatus.value = st.status
      // 历史日志也从后端带回来（本地快照留底可能没有）—— 结果窗日志区有内容可看
      mergeLogs(st?.logs)
    }).catch(() => {})
    return
  }
  loading.value = true
  result.value = null
  compareLogs.value = []
  taskStatus.value = ''
  taskDone.value = 0
  taskTotal.value = 0
  taskCurrent.value = t('cmp.preparing')
  taskMessage.value = ''
  resumedViewOnly.value = false
  detailTab.value = 'struct'
  // **从任务中心查看时只显示进度/结果窗**：后面的向导框（步骤 1/2/3）藏起来 ——
  // 用户点「查看」要看的是结果，不是重新走向导（真机反馈）
  wizardHidden.value = true
  showProgressDlg.value = true
  stopPolling()
  disarmStopGuard()
  pollTimer = setInterval(pollTask, 800)
})

onBeforeUnmount(() => {
  stopPolling()
  // 兜底定时器一起清掉：组件都没了，它再去改状态只会报"操作已卸载的组件"
  if (stopGuard) { clearTimeout(stopGuard); stopGuard = null }
})
watch(() => props.modelValue, v => {
  // **关闭时复位向导隐藏态**：从任务中心查看（wizardHidden=true）后，任务可能经
  // 「后台运行」等不经进度窗关闭按钮的路径 emit false —— 隐藏态残留的话，
  // 之后点顶栏「数据对比」会被 `modelValue && !wizardHidden` 挡住，永远打不开
  //（间歇性「点了没反应」的真凶，真机踩过）
  if (!v) { wizardHidden.value = false; return }
  // 重新打开时若还处于「查看」残留态（进度窗开着/隐藏标记在），一并清掉 ——
  // 用户点「数据对比」要的是**新对比**，不是上一次那个查看
  if (wizardHidden.value) {
    wizardHidden.value = false
    showProgressDlg.value = false
    stopPolling()
  }
  // **上次任务还在跑**（后台运行过）：恢复轮询让它自然落定 —— 不恢复的话
  // loading 永远 true，「开始对比」被禁、「停止对比」错位显示在向导 footer，
  // 任务跑完也没人知道（审查发现的死路，真机路径）
  if (loading.value && taskId.value) {
    taskMessage.value = t('cmp.preparing')
    stopPolling()
    pollTimer = setInterval(pollTask, 1500)
  }
  step.value = 1
  loadConnections()
})
onMounted(() => { if (props.modelValue) loadConnections() })
</script>

<style scoped>
/* 步骤条 + 摘要（对齐数据传输的向导式交互）—— 标题字号压小，少占高度也更精致 */
.cmp-steps { margin-bottom: 16px; }
.cmp-steps :deep(.el-step__title) { font-size: 12px; }
.cmp-steps :deep(.el-step__head.is-finish .el-step__line),
.cmp-steps :deep(.el-step__main) { line-height: 1.4; }
.cmp-summary { margin-bottom: 14px; }
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
.progress-dialog :deep(.el-dialog__body) { padding: 16px 20px 10px; max-height: 62vh; overflow: auto; }
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
