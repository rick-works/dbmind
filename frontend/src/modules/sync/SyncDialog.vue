<template>
  <el-dialog :model-value="modelValue" width="920px" top="5vh" class="sync-dialog"
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
            <el-select v-if="!srcHideDb" v-model="src.database" size="small" :disabled="taskRunning" @change="onSrcDbChange">
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
            <el-select v-if="!tgtHideDb" v-model="tgt.database" size="small" :disabled="taskRunning" @change="onTgtDbChange">
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

    <!-- ==================== 步骤 2：选择对象 ====================
         卡片式分组下拉：每一类对象是一张可展开的卡片（勾选框 + 图标 + 类型名 + 已选/总数 + 箭头），
         展开后才列出该类对象。分组按**源库与目标库的能力**决定要不要画（见 visibleGroups）——
         源库没有这个概念（PG 没有 EVENT）、或目标库建不出来（目标不支持该对象 DDL）就不摆空架子。 -->
    <div class="panel obj-panel" v-if="step === 2 && src.connectionId && src.database" v-loading="objsLoading">
      <div class="panel-head">
        <span class="panel-title"><el-icon :size="14"><FolderOpened /></el-icon>{{ $t('sync.pickObjects') }}</span>
        <span class="panel-count">{{ $t('sync.selectedPrefix') }}<b>{{ totalSelected }}</b> / {{ $t('sync.selectedSuffix', { total: totalAvailable }) }}</span>
      </div>

      <div class="obj-toolbar">
        <el-input v-model="keyword" size="small" clearable :prefix-icon="Search"
                  :placeholder="$t('sync.searchObj')" :disabled="taskRunning" />
        <el-button size="small" text :disabled="taskRunning || !availableCount" @click="applyAll(true)">{{ $t('sync.selectAll') }}</el-button>
        <el-button size="small" text :disabled="taskRunning || !totalSelected" @click="applyAll(false)">{{ $t('sync.unselectAll') }}</el-button>
      </div>

      <div v-if="!availableCount" class="obj-empty-all">{{ $t('sync.noObjects') }}</div>
      <template v-else>
        <!-- 一类对象一个页签（原先是手风琴卡片）。徽标是「已选/总数」；
             页签前的勾选框 = 本类全选/取消（顶部两个按钮才是全部类别）。
             点勾选框不切页签，所以单独 stop。 -->
        <div class="obj-tabs" role="tablist">
          <button v-for="g in visibleGroups" :key="g.key" type="button" role="tab"
                  class="obj-tab"
                  :class="{ on: activeGroup && activeGroup.key === g.key, empty: !objState[g.key].list.length }"
                  :aria-selected="activeGroup && activeGroup.key === g.key"
                  @click="activeTab = g.key">
            <el-checkbox class="ot-check" :model-value="groupChecked(g.key)"
                         :indeterminate="groupIndeterminate(g.key)"
                         :disabled="taskRunning || !objState[g.key].list.length"
                         @click.stop @change="v => setGroup(g.key, v)" />
            <span class="ot-ic" :class="'ty-' + g.key"><el-icon :size="12"><component :is="TYPE_ICON[g.key]" /></el-icon></span>
            <span class="ot-name">{{ typeLabel(g.key) }}</span>
            <span class="ot-count">{{ objState[g.key].selected.length }}/{{ objState[g.key].list.length }}</span>
          </button>
        </div>

        <div class="obj-list" v-if="activeGroup">
          <div v-if="!groupItems(activeGroup.key).length" class="obj-none">
            <span class="nc-ic"><el-icon :size="22"><FolderOpened /></el-icon></span>
            <span class="nc-txt">{{ keyword ? $t('sync.noMatch') : $t('sync.emptyGroup') }}</span>
          </div>
          <div v-for="name in groupItems(activeGroup.key)" :key="name" class="og-item"
               :class="{ on: isSelected({ type: activeGroup.key, name }) }"
               @click="toggleObj({ type: activeGroup.key, name }, !isSelected({ type: activeGroup.key, name }))">
            <el-checkbox :model-value="isSelected({ type: activeGroup.key, name })" :disabled="taskRunning"
                         @click.stop @change="v => toggleObj({ type: activeGroup.key, name }, v)" />
            <span class="og-item-name" :title="name">{{ name }}</span>
          </div>
        </div>
      </template>
    </div>

    <!-- ==================== 步骤 3：传输选项 ==================== -->
    <div class="settings-card" v-if="step === 3">
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
          <!-- 并发表数：0 = 自适应（按源/目标类型与对象数定）。目标端弱、或源与目标同实例时
               调小它，能直接减少双方连接数与会话/IO 争抢（见后端 sync::db 的「会话预算」）。
               说明文字收进标签后的问号（悬浮显示），不占版面。 -->
          <div class="settings-inline">
            <span class="settings-inline-label">
              {{ $t('sync.maxConcurrency') }}
              <el-tooltip :content="$t('sync.maxConcurrencyHint')" placement="top" :show-after="120">
                <el-icon class="hint-q"><QuestionFilled /></el-icon>
              </el-tooltip>
            </span>
            <el-input-number v-model="opts.maxConcurrency" :min="0" :max="32" :step="1" size="small"
                             controls-position="right" :disabled="taskRunning" class="settings-num" />
          </div>
          <!-- 「写入批次」输入已删：每种库的甜点批大差一个数量级（CH 落 part / Oracle 有
               单语句上限 / MySQL 看 max_allowed_packet），由后端按目标方言自动决定 -->
          <!-- 「行数上限」也删了（用户口径：同步就是不限，抽样不是同步的事）；
               原先那个全局 WHERE 输入框挪到下面**按表设置** —— 一个条件拼到几十张表上
               语义说不清，见 .settings-where -->
        </div>
      </div>

      <!-- ==================== 行过滤（按表设置） ====================
           一份全局 WHERE 在多表同步里没有意义：`amount > 100` 拼到没有 amount 列的表上，
           整张表直接报错。所以改成「表 → 条件」，留空的表全量同步。 -->
      <div class="settings-where">
        <div class="sw-head">
          <div class="sw-title">
            <el-icon :size="13"><Filter /></el-icon>
            <span>{{ $t('sync.wherePerTable') }}</span>
            <span v-if="whereCount" class="sw-count">{{ $t('sync.whereSetCount', { n: whereCount }) }}</span>
          </div>
          <el-input v-if="selTables.length > 5" v-model="whereSearch" size="small" clearable
                    :placeholder="$t('sync.whereSearch')" class="sw-search" />
        </div>
        <p class="sw-hint">{{ $t('sync.wherePerTableHint') }}</p>
        <div v-if="selTables.length" class="sw-list">
          <div v-for="name in whereTableList" :key="name" class="sw-item"
               :class="{ on: !!(whereByTable[name] || '').trim() }">
            <span class="sw-name" :title="name">{{ name }}</span>
            <el-input v-model="whereByTable[name]" size="small" :disabled="taskRunning"
                      :placeholder="$t('sync.wherePh')" maxlength="500" />
          </div>
          <div v-if="!whereTableList.length" class="sw-none">{{ $t('sync.whereNoMatch') }}</div>
        </div>
        <div v-else class="sw-none">{{ $t('sync.whereNoTable') }}</div>
      </div>
    </div>

    <!-- ==================== 步骤 4：摘要 ==================== -->
    <div class="summary-card" v-if="step === 4">
      <div class="summary-title">{{ $t('sync.stepSummary') }}</div>
      <el-table :data="summaryRows" size="small" border class="data-table summary-table">
        <!-- 行过滤按表展开，标签是「WHERE · 表名」，表名长了要能悬停看全 -->
        <el-table-column prop="label" width="180" show-overflow-tooltip />
        <el-table-column prop="value" show-overflow-tooltip />
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
        <!-- 百分比挪到标题行右侧（大号等宽数字），进度条本身只当"刻度尺"：
             `text-inside` 在细条里会挤成一团，10% 时更是被压在左端看不清 -->
        <div class="pg-top">
          <span class="pg-title">
            <el-icon :size="14"><Loading v-if="taskRunning" /><CircleCheck v-else /></el-icon>
            {{ $t('sync.progress') }}
          </span>
          <span class="pg-pct" :class="{ done: isTaskDone, err: taskStatus === 'error', warn: taskStatus === 'canceled' }">
            {{ progressPct }}%
          </span>
        </div>
        <el-progress :percentage="progressPct" :stroke-width="8" :show-text="false"
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
  Promotion, Right, VideoPause, Search, FolderOpened, Files, DataLine, Operation, Document,
  Grid, View, FirstAidKit, SetUp, Lightning, Calendar, Clock, Filter, QuestionFilled,
  Loading, CircleCheck
} from '@element-plus/icons-vue'
import { listConnections, listDatabases, listSchemas, listTables, listProcedures, listTriggers, listEvents, getFeatures, syncDb, syncTaskStatus, syncCancel } from '../../api'
// 「库」这一层要不要画（Oracle / DM 没有库层：它就是连接里填的服务名）
import { hideDbLevelOf } from '../../types'
// 函数 / 存储过程的判据必须与左侧对象树同源（见 treeNodes.js 的 isFunctionRoutine），
// 否则会出现"树上算函数、同步时算存储过程"这种两边对不上的分叉
import { isFunctionRoutine } from '../data/treeNodes'
import { t } from '../../utils/i18n'
import { guardConnWrite, loadProtectProd } from '../../utils/connGuard'
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
/** 对象清单加载中（339 张表要 4 个接口，加载期间给个转圈，别让人以为列表是空的） */
const objsLoading = ref(false)

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
// 步骤顺序与顶部步骤条一致：1 源/目标 → 2 对象 → 3 选项 → 4 摘要（曾经面板与标题错位过一格）
const nextStep = async () => {
  if (step.value === 1) {
    if (!src.value.connectionId || !src.value.database) return ElMessage.warning(t('sync.pickSourceDb'))
    if (!tgt.value.connectionId || !tgt.value.database) return ElMessage.warning(t('sync.pickTargetDb'))
    if (src.value.connectionId === tgt.value.connectionId && src.value.database === tgt.value.database) {
      return ElMessage.warning(t('sync.sameDb'))
    }
    // 进入对象页前确保清单已加载（换连接/换库时 onXxxChange 已自动加载，这里兜底）
    if (!totalAvailable.value) await loadSrcObjects()
  }
  // 对象页：一个都没选就拦下 —— 全选是默认值，空选基本是用户自己清空了
  if (step.value === 2 && !totalSelected.value) return ElMessage.warning(t('sync.pickObjectsWarn'))
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
  // 行过滤按表逐条列出（只在摘要里出现真正设置了条件的表；表名带上，免得「WHERE」一条
  // 不知道管的是谁 —— 这正是旧界面的毛病）
  ...selTables.value
    .map(name => ({ name, clause: (whereByTable[name] || '').trim() }))
    .filter(x => x.clause)
    .map(x => ({ label: 'WHERE · ' + x.name, value: x.clause })),
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
// 没有「库」这一层的类型（Oracle / DM）：库里填的就是连接自身的服务名，永远一条 ——
// 下拉不画（画出来就是把服务名摆给用户看），请求里照旧带它（见 loadSrcDbs / loadTgtDbs）
const srcHideDb = computed(() => hideDbLevelOf(srcInfo.value?.type))
const tgtHideDb = computed(() => hideDbLevelOf(tgtInfo.value?.type))

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
// 页签顺序就是这里的顺序；「有没有内容」只看 objState 的 list
const objTypes = reactive([
  { key: 'table' },
  { key: 'view' },
  { key: 'function' },
  { key: 'procedure' },
  { key: 'trigger' },
  { key: 'event' },
])
/** 当前页签（对象列表只画这一类） */
const activeTab = ref('table')
const objState = reactive({})
for (const t of objTypes) objState[t.key] = { list: [], selected: [] }

// rowLimit / whereClause 已移除：同步就是不限行数；行过滤改按表设置（见 whereByTable）
const opts = ref({ objectPolicy: 'drop', dataMode: 'truncate_insert', syncStructure: true, syncData: true, includeIndexes: true, autoCreateDb: true, stopOnError: false, maxConcurrency: 0 })

// ==================== 行过滤（按表设置） ====================
// 为什么不是全局一个 WHERE：一次同步几十上百张表时，`amount > 100` 拼到没有 amount 列的
// 表上就是整张表报错。这里用「表名 → 条件」的映射，只有填了的表才带 where，其余全量。
const whereByTable = reactive({})
/** 本次勾选的表（行过滤只对表有意义：视图/例程/触发器同步的是 DDL） */
const selTables = computed(() => (objState.table ? objState.table.selected : []))
/** 表很多时给个搜索框，省得从上拉到下找那一张 */
const whereSearch = ref('')
const whereTableList = computed(() => {
  const kw = whereSearch.value.trim().toLowerCase()
  if (!kw) return selTables.value
  return selTables.value.filter(name => name.toLowerCase().includes(kw))
})
/** 已设置条件的表数（标题上的徽标） */
const whereCount = computed(() => selTables.value.filter(name => (whereByTable[name] || '').trim()).length)
// 换了源连接 / 源库，表清单就是另一批了：旧条件按表名留着只会在**同名表**上误伤
// （上一次给 orders 设了 `amount > 100`，这次换个库里的 orders 没有 amount 列 —— 一开跑就报错）
watch([() => src.value.connectionId, () => src.value.database], () => {
  for (const name of Object.keys(whereByTable)) delete whereByTable[name]
  whereSearch.value = ''
})

// 任务状态
let pollTimer = null
let tickTimer = null
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
// 句柄必须存下来：这个 100ms 心跳跟着组件活，不清理的话每次打开「数据传输」都留一个孤儿定时器
tickTimer = setInterval(() => { if (taskRunning.value) runningTick.value++ }, 100)
const extrapolate = (base, rate) => {
  runningTick.value
  if (!taskRunning.value || !fetchAt.value) return base
  // 外插打 9 折，且**时间窗钳制在 1.0s**：页面节流/轮询卡顿时 dt 会失控变大，
  // 不钳制的话「速率 × 时间」能冲到远超真实值（真机：已读取 20.5 万显示成 22 万），
  // Math.max 防倒退保护还会让它卡在虚高处不回落
  const dt = Math.min(Math.max(0, (Date.now() - fetchAt.value) / 1000), 1.0)
  return base + rate * 0.9 * dt
}
const targetRead = computed(() => Math.floor(extrapolate(rowsRead.value, rates.value.read)))
// **传输数恒 ≤ 读取数**（写入行 ⊆ 读取行，语义不变式）—— 两个数各自外插时
// 领先量不同步，会出现「传输数比读取数还多」的荒诞画面（真机踩过）
const targetWritten = computed(() => Math.min(
  Math.floor(extrapolate(rowsWritten.value, rates.value.written)),
  targetRead.value,
))
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
// 分母固定为**已加载的全部对象**（不随搜索/展开变）：跟着变会让「已选 5 / 2」这种不可能的数字出现
const totalAvailable = computed(() => objTypes.reduce((s, t) => s + objState[t.key].list.length, 0))
const availableCount = computed(() => objTypes.reduce((s, t) => s + (objState[t.key].list.length ? 1 : 0), 0))
/** 某一类对象在当前搜索词下要显示哪些（搜索只过滤展示，不影响已选） */
const groupItems = (key) => {
  const kw = keyword.value.trim().toLowerCase()
  const list = objState[key].list
  return kw ? list.filter(n => n.toLowerCase().includes(kw)) : list
}
/** 搜索时自动切到**第一个有命中的页签**，免得用户以为「搜不到」 */
watch(keyword, (kw) => {
  if (!kw.trim()) return
  const hit = objTypes.find(g => groupItems(g.key).length)
  if (hit) activeTab.value = hit.key
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

// ==================== 分组 / 对象操作 ====================
/**
 * 分组要不要显示：**由源库与目标库的能力决定**（用户要求「根据源数据库和目标数据库的特性展示」）。
 *
 * 判据优先用后端 features 的 `objectDdlKinds`（源库真能取出 DDL 的对象种类，
 * 例如 PG 是 view/procedure/function/trigger，没有 event），拿不到再退回
 * supportsProcedures / supportsTriggers / supportsEvents 这几个布尔。
 * 两边都支持才画（目标建不出同类对象时，勾了也只会得到一排「跳过」）；
 * 但**源库里确实有对象**的分组照旧显示 —— 宁可让用户看见再决定，也不能悄悄藏掉。
 */
const KIND_FLAG = {
  view: 'supportsDdl', procedure: 'supportsProcedures', function: 'supportsProcedures',
  trigger: 'supportsTriggers', event: 'supportsEvents',
}
const kindSupported = (feats, key) => {
  // 能力没取到（连接刚选好/接口失败）时一律按支持处理，不能因为拿不到就少摆一栏
  if (!feats || !Object.keys(feats).length) return true
  if (key === 'table') return true
  const kinds = feats.objectDdlKinds
  if (Array.isArray(kinds) && kinds.length) return kinds.includes(key)
  const flag = KIND_FLAG[key]
  return flag ? feats[flag] !== false : true
}
const visibleGroups = computed(() => objTypes.filter(g =>
  objState[g.key].list.length > 0
  || (kindSupported(srcFeatures.value, g.key) && kindSupported(tgtFeatures.value, g.key))
))
/** 当前页签对应的那一类（页签被能力过滤/切库重建时兜底到第一个可见页签） */
const activeGroup = computed(() =>
  visibleGroups.value.find(g => g.key === activeTab.value) || visibleGroups.value[0] || null
)
/** 组头勾选框三态：按**当前搜索过滤后的可见项**判断（搜「ods_」时勾一下就是勾这些） */
const groupChecked = (key) => {
  const items = groupItems(key)
  return items.length > 0 && items.every(n => objState[key].selected.includes(n))
}
const groupIndeterminate = (key) => {
  const items = groupItems(key)
  const n = items.filter(x => objState[key].selected.includes(x)).length
  return n > 0 && n < items.length
}
const setGroup = (key, val) => {
  const items = groupItems(key)
  const sel = new Set(objState[key].selected)
  for (const n of items) { if (val) sel.add(n); else sel.delete(n) }
  objState[key].selected = [...sel]
}
const isSelected = (o) => objState[o.type].selected.includes(o.name)
const toggleObj = (o, v) => {
  const sel = objState[o.type].selected
  const i = sel.indexOf(o.name)
  if (v && i < 0) sel.push(o.name)
  if (!v && i >= 0) sel.splice(i, 1)
}
/** 全部选择 / 取消全选：只作用于**显示出来的分组**（被能力过滤掉的不碰） */
const applyAll = (val) => {
  for (const g of visibleGroups.value) setGroup(g.key, val)
}

// ==================== 对象加载 ====================
const loadConnections = async () => {
  // 只展示真正有传输能力的连接。NoSQL 的连接模型不是二维表，后端没有
  // 对应 Reader/Writer；让它们出现在这里只会把“暂不支持”伪装成配置错误。
  connections.value = (await listConnections()).filter(c => c.dataTransfer !== false)
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
  if (srcHideDb.value) {
    // 没有库层：库值取连接自身的服务名（下拉不画），直接进「模式」那一步
    srcDbs.value = []
    src.value.database = String(srcInfo.value?.database || '')
    try { srcFeatures.value = await getFeatures(src.value.connectionId) } catch (e) { srcFeatures.value = {} }
    await onSrcDbChange()
    return
  }
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
  if (tgtHideDb.value) {
    // 同源：没有库层就取连接自身的服务名，下拉不画
    tgtDbs.value = []
    tgt.value.database = String(tgtInfo.value?.database || '')
    try { tgtFeatures.value = await getFeatures(tgt.value.connectionId) } catch (e) { tgtFeatures.value = {} }
    await onTgtDbChange()
    return
  }
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
  objsLoading.value = true
  try {
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
  } finally {
    objsLoading.value = false
  }
  // **默认全选**（用户口径「有的都显示出来也都支持同步」）：进来就能直接下一步，
  // 不想同步的某一类在组头一次取消即可 —— 以前是全都不选 + 类型徽标当筛选器，
  // 界面停在「已选 0 / 339、列表空白」，看着像坏了。
  for (const t of objTypes) {
    objState[t.key].selected = [...objState[t.key].list]
  }
  // 默认停在第一个有内容的页签（表通常就是第一个；只有视图的库也不会停在空页签）
  const first = objTypes.find(t => objState[t.key].list.length > 0)
  if (first) activeTab.value = first.key
}

// ==================== 同步执行 ====================
const run = async () => {
  if (!src.value.connectionId || !src.value.database) return ElMessage.warning(t('sync.pickSourceDb'))
  if (!tgt.value.connectionId || !tgt.value.database) return ElMessage.warning(t('sync.pickTargetDb'))
  // 目标库是要被写入的一方：只读 / 生产保护连接在点击执行那一刻就拦下（全站统一闸门）
  await loadProtectProd()
  const targetConn = connections.value.find(x => String(x.id) === String(tgt.value.connectionId))
  if (!guardConnWrite(targetConn || {}, t('nav.sync'))) return
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
    // 0 = 自适应（后端按源/目标类型与对象数定）；>0 以这里为准
    maxConcurrency: opts.value.maxConcurrency || 0,
    // 行过滤按表提交：只带真正填了条件的表（没勾选的表就算残留了旧条件也不传）
    whereClauses: Object.fromEntries(selTables.value
      .map(name => [name, (whereByTable[name] || '').trim()])
      .filter(([, clause]) => clause)),
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
    // **外插猜超了就回贴真实值**：显示值高于真实值超过「一个轮询周期的合理量」时，
    // 说明外插跑飞了（页面节流/轮询卡顿）—— Math.max 防倒退保护是轮询间隔内的规则，
    // 跨轮询必须信任后端真实值，否则虚高会一直挂着不回落（真机：20.5 万显示成 22 万）
    if (readShown.value > rowsRead.value + rates.value.read * 1.2) readShown.value = rowsRead.value
    if (writtenShown.value > rowsWritten.value + rates.value.written * 1.2) writtenShown.value = rowsWritten.value
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
  // 心跳定时器一并停掉（见它的声明处）
  if (tickTimer) { clearInterval(tickTimer); tickTimer = null }
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
/* ==================== 对话框高度：默认给足，内容多时**内部滚动** ====================
   高度定死而不是随内容长：切步骤时底栏不会上下跳。
   默认 min(660px, 90vh)：比最初的 780px 收了一圈（用户反馈默认太大、挡视野），
   `90vh` 是给矮窗口兜底的 —— 高度只给 84vh 时，可视区 700 上下的小窗只剩 590，
   对象页展开一张卡片就挤成一团；嫌小仍可拖右下角放大，
   双击手柄恢复这个默认值 —— 见 utils/dialogResize.js（所有功能性弹窗都有）。

   ⚠ 必须用 `:global()`：弹窗根节点（.el-dialog）是 el-dialog 组件 teleport 出去的，
   身上**没有本组件的 data-v 属性**，`.sync-dialog { … }` 这种写法在这里命中不了任何元素
   （本文件里其余的 `.sync-dialog :deep(...)` 一类的规则同病，先不动）。 */
/* 头 / 身 / 脚三段式：高度定死，**身体弹性**，内容自己滚。
   以前只有身体是 flex column，弹窗根节点仍是块级 —— 拖大以后多出来的高度落不到
   任何元素上（日志卡还是那 320px，下面空一大片）。 */
:global(.el-dialog.sync-dialog) {
  height: min(660px, 90vh);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
:global(.el-dialog.sync-dialog .el-dialog__header),
:global(.el-dialog.sync-dialog .el-dialog__footer) { flex: none; }
:global(.el-dialog.sync-dialog .el-dialog__body) {
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  min-height: 0;
  /* auto 而不是 hidden：日志卡自己滚，但真挤到放不下时（矮窗口 + 结果表）
     身体还能滚一下，不至于把内容裁掉 */
  overflow: auto;
  padding-top: 4px;
}
/* 步骤条/底栏不参与弹性分配，剩下的高度全给当前步骤的内容 */
.sync-steps { flex: none; }
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
  /* 同 .settings-card：不做 flex 收缩，窗口矮时由正文滚动，别把表格/告警裁掉 */
  flex: none;
}
.summary-title { font-size: 13px; font-weight: 600; color: var(--dc-primary); margin-bottom: 10px; }
.summary-table { margin-bottom: 12px; }
.summary-opts { display: flex; flex-wrap: wrap; gap: 8px 20px; font-size: 13px; color: var(--dc-text-dim); }
.summary-opts b { color: var(--dc-text); font-weight: 500; }

/* ==================== 步骤 5 执行 ==================== */
/* 执行页是「进度 + 统计 + 日志」三段，只有日志吃弹性高度：
   拖大弹窗 ⇒ 日志变长；拖小 ⇒ 日志先缩（保底 96px）再让结果表出去。 */
.run-view {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.run-view .pg-summary,
.run-view .run-stats,
.run-view .pg-result { flex: none; }
.run-view .pg-log {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  margin-bottom: 0;
}
.run-view .pg-log .log-head { flex: none; }
.run-view .pg-log .log-body { flex: 1 1 auto; min-height: 0; }

.run-stats {
  display: flex; gap: 10px; margin: 12px 0;
  /* 拖窄以后 5 张卡会挤成一条缝，允许换行 */
  flex-wrap: wrap;
}
.run-stats .stat {
  flex: 1 1 108px; min-width: 96px; text-align: center;
  background: linear-gradient(180deg, var(--dc-bg-card), var(--dc-bg-soft));
  border: 1px solid var(--dc-border); border-radius: 10px;
  padding: 10px 8px;
  transition: border-color .15s, box-shadow .15s;
}
.run-stats .stat:hover { border-color: var(--dc-border-hover); box-shadow: var(--dc-shadow-sm); }
.run-stats .num {
  display: block; font-size: 19px; font-weight: 700; color: var(--dc-text);
  /* 等宽数字：计数每 2 秒跳一次，比例数字宽窄不一会让整排左右抖 */
  font-variant-numeric: tabular-nums;
  letter-spacing: .2px;
}
.run-stats .label { display: block; margin-top: 2px; font-size: 12px; color: var(--dc-text-dim); }
.run-stats .stat.ok { background: linear-gradient(180deg, var(--dc-success-wash), var(--dc-bg-soft)); }
.run-stats .stat.ok .num { color: var(--dc-success); }
.run-stats .stat.danger { background: linear-gradient(180deg, var(--dc-danger-wash), var(--dc-bg-soft)); }
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

/* ==================== 对象选择：卡片式分组下拉 ====================
   每一类对象是一张卡片（组头一行 + 可展开的对象网格），视觉上与「卡片」一致：
   圆角、独立描边、悬停微亮、展开时组头高亮。 */
.obj-toolbar {
  display: flex; align-items: center; gap: 8px;
  margin-bottom: 10px;
}
.obj-toolbar .el-input { flex: 1; min-width: 0; }
.obj-empty-all {
  padding: 26px 0; text-align: center;
  font-size: 13px; color: var(--dc-text-dim);
}
/* 对象页 = 一行页签 + 占满剩余高度的列表。
   原先是手风琴（一类一张卡、展开的那张吃剩余高度）：窗口一矮，折叠的卡片会被 flex
   压成 2px（`overflow:hidden` 让 `min-height:auto` 解析成 0），看着像样式炸了。
   改成页签后 6 类固定只占一行（约 32px），列表独占剩余高度，任何窗口高度都稳。 */
.obj-panel { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; }
.obj-tabs {
  flex: none;
  display: flex; align-items: flex-end; gap: 2px;
  border-bottom: 1px solid var(--dc-border);
  margin-bottom: 8px;
  overflow-x: auto;
  scrollbar-width: none;
}
.obj-tabs::-webkit-scrollbar { height: 0; }
.obj-tab {
  flex: none;
  display: inline-flex; align-items: center; gap: 7px;
  /* 显式定高：不让 el-checkbox 的 32px 行盒把页签顶高（否则一行页签要 41px） */
  height: 30px;
  padding: 0 8px;
  border: none; background: transparent;
  border-bottom: 2px solid transparent;
  border-radius: 7px 7px 0 0;
  font: inherit;
  color: var(--dc-text-dim);
  cursor: pointer;
  white-space: nowrap;
  transition: background .15s, color .15s, border-color .15s;
}
.obj-tab:hover { background: var(--dc-bg-hover); color: var(--dc-text); }
.obj-tab.on {
  color: var(--dc-primary);
  font-weight: 600;
  background: var(--dc-primary-wash);
  border-bottom-color: var(--dc-primary);
}
.obj-tab.empty { opacity: .62; }
/* 页签里的小勾选框（= 本类全选）：默认 32px 高会把页签顶高一圈，压到 16px */
/* ⚠ 只压**行高**，不要动 `.el-checkbox__inner` / `::after` 的尺寸：
   对号是 `::after` 上用 3×7 的边框画的，跟着框缩小就对不齐（真机反馈「勾上了看不见对号」）。
   框本身默认就是 14×14，够小；把容器 32px 的行高收掉即可。 */
.ot-check { margin-right: -2px; display: inline-flex; }
.ot-check :deep(.el-checkbox) { height: 18px; margin-right: 0; }
.ot-check :deep(.el-checkbox__label) { display: none; }
.ot-ic {
  width: 18px; height: 16px;
  display: inline-flex; align-items: center; justify-content: center;
  border-radius: 4px;
  color: #0f1522;
  flex: none;
  margin-left: 1px;
}
.ot-name { font-size: 13px; }
.ot-count {
  font-size: 11.5px; line-height: 15px;
  padding: 0 5px; border-radius: 8px;
  background: var(--dc-bg-soft); color: var(--dc-text-dim);
  font-variant-numeric: tabular-nums;
}
.obj-tab.on .ot-count { background: var(--dc-primary); color: #fff; }
/* 列表：多列铺排，对象多了在这一层滚动（高度 = 面板剩余空间）
   列宽用 minmax 的固定份数而不是 auto-fill：否则最后一份会被滚动条挤出行外 */
.obj-list {
  flex: 1 1 auto; min-height: 0;
  overflow-y: auto; overflow-x: hidden;
  padding-right: 2px;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 210px), 1fr));
  gap: 0 10px;
  align-content: start;
}
/* 空页签 / 无匹配：一个柔和的小空状态（虚线圈 + 图标 + 一行说明），居中在列表区中间，
   而不是左上角一句小灰字。min-height 让它落在视觉中心（列表区高约 280~320px）。 */
.obj-none {
  grid-column: 1 / -1;
  min-height: 200px;
  display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 10px;
  padding: 32px 12px;
  color: var(--dc-text-dim);
  font-size: 13px;
  text-align: center;
}
.obj-none .nc-ic {
  width: 48px; height: 48px; border-radius: 15px;
  display: inline-flex; align-items: center; justify-content: center;
  background: var(--dc-bg-soft);
  border: 1px dashed var(--dc-border);
  color: var(--dc-text-dim);
  opacity: .85;
}
.obj-none .nc-txt { line-height: 1.6; }
/* 列表项里的勾选框只收行高（32→20px，一行从 38px 降到 22px）；
   **不动 inner/::after** —— 对号是 ::after 上用 3×7 边框画的，缩框就对不齐、看不见勾。 */
.obj-list :deep(.el-checkbox) { height: 20px; margin-right: 2px; }
.og-item {
  display: flex; align-items: center; gap: 6px;
  padding: 2px 6px;
  border-radius: 6px;
  cursor: pointer;
  min-width: 0;
  transition: background .15s;
}
.og-item:hover { background: var(--dc-bg-hover); }
.og-item.on { background: var(--dc-primary-wash); }
.og-item-name {
  flex: 1; min-width: 0;
  font-size: 13px; color: var(--dc-text);
  font-family: "Cascadia Code", Consolas, "JetBrains Mono", monospace;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
/* 分组图标底色（沿用原对象类型配色，一眼能分清是哪一类） */
.ty-table { background: var(--dc-accent); }
.ty-view { background: #7cabff; }
.ty-function { background: #c4b0ff; }
.ty-procedure { background: #f8c878; }
.ty-trigger { background: #ff9d9d; }
.ty-event { background: #7fe3f5; }

/* ==================== 同步设置 ==================== */
.settings-card {
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: 14px;
  padding: 0;
  margin-bottom: 14px;
  box-shadow: var(--dc-shadow-sm);
  overflow: hidden;
  /* 弹窗正文是 flex 列（见 :global(.el-dialog.sync-dialog .el-dialog__body)），
     卡片作为 flex 项默认 `flex-shrink: 1` —— 窗口一矮它就被压扁，配合上面的
     `overflow: hidden` 直接把「行过滤」列表**拦腰裁断**（真机截图：最后一行只露半个）。
     让它保持自然高度，放不下由正文滚动（dc-resizable 的正文本就是 overflow: auto）。 */
  flex: none;
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
/* 勾选项一律**允许换行**：这里的中文标签很长（「建表包含索引（把源表的索引一起建到目标）」），
   Element 默认 `white-space: nowrap` 会把它硬裁掉半句（真机截图：执行选项三行全被切） */
.settings-col :deep(.el-checkbox) { white-space: normal; align-items: flex-start; }
.settings-col :deep(.el-checkbox__input) { margin-top: 1px; }
.settings-col :deep(.el-radio__label),
.settings-col :deep(.el-checkbox__label) {
  font-size: 13px; color: var(--dc-text);
  white-space: normal; line-height: 1.45;
}
.settings-hint {
  font-size: 11.5px; line-height: 1.5;
  color: var(--dc-text-weak);
  margin: 0;
}
.settings-checks { display: flex; flex-direction: column; gap: 6px; }
/* 「并发表数」：一行标签 + 数字框；说明收进标签后的问号（悬浮显示） */
.settings-inline { display: flex; align-items: center; gap: 8px; margin-top: 4px; }
.settings-inline-label { font-size: 13px; color: var(--dc-text); line-height: 24px; flex: none; }
.hint-q { font-size: 12px; color: var(--dc-text-weak); cursor: help; vertical-align: -1px; margin-left: 2px; }
.hint-q:hover { color: var(--el-color-primary); }
.settings-num { width: 96px; flex: none; }

/* ==================== 行过滤（按表设置） ====================
   横跨三列的一整块：一行一张表，左侧表名定宽、右侧条件吃掉剩余宽度。
   表多时列表内部滚动（不把弹窗撑高），配一个搜索框。 */
.settings-where {
  margin: 0 16px 14px;
  padding: 12px 14px;
  border-radius: 12px;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
}
.sw-head { display: flex; align-items: center; gap: 10px; }
.sw-title {
  display: inline-flex; align-items: center; gap: 6px;
  font-size: 13px; font-weight: 700; color: var(--dc-text);
  flex: 1; min-width: 0;
}
.sw-title .el-icon { color: var(--dc-accent); }
.sw-count {
  font-size: 11px; font-weight: 600;
  padding: 1px 7px; border-radius: 9px;
  color: var(--dc-primary);
  background: var(--dc-primary-wash);
  border: 1px solid var(--dc-border);
}
.sw-search { width: 180px; flex: none; }
.sw-hint {
  font-size: 11.5px; line-height: 1.5;
  color: var(--dc-text-weak);
  margin: 4px 0 8px;
}
/* 表多（几十上百张）时这一段必须自己滚动，否则整步被顶成一条长龙；
   168px 只够 5 行半，第 6 行会被拦腰切开（真机截图），提到 260px ≈ 8 行整 */
.sw-list { max-height: 260px; overflow-y: auto; display: flex; flex-direction: column; gap: 6px; padding-right: 4px; }
.sw-list::-webkit-scrollbar { width: 6px; }
.sw-list::-webkit-scrollbar-thumb { background: var(--dc-border); border-radius: 3px; }
.sw-item { display: flex; align-items: center; gap: 8px; }
.sw-item.on .sw-name { color: var(--dc-primary); font-weight: 600; }
.sw-name {
  flex: none; width: 190px;
  font-size: 12px; color: var(--dc-text-dim);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.sw-item .el-input { flex: 1; }
.sw-none { font-size: 12px; color: var(--dc-text-weak); padding: 4px 0; }

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
  padding: 2px 8px;
  border-radius: 6px;
  font-size: 13px; line-height: 1.6;
  color: var(--dc-code-text);
  font-family: "Cascadia Code", Consolas, "JetBrains Mono", monospace;
}
/* 长日志一屏几十行，hover 高亮帮眼睛「不串行」；出问题的那几行整行带底色更醒目 */
.log-line:hover { background: var(--dc-bg-hover); }
.log-line.err { color: var(--dc-danger); background: var(--dc-danger-wash); }
.log-line.warn { color: var(--dc-warning); }
.log-line.ok { color: var(--dc-success); }
.log-time {
  flex: none;
  color: var(--dc-text-weak);
  font-size: 12px;
  /* 定宽让所有行的正文左边缘对齐（时间戳等长，不定宽会被等宽字体带歪） */
  font-variant-numeric: tabular-nums;
}
/* 正文占满剩余宽度：长句子换行后仍与第一行左对齐，超长 token（URL、SQL 片段）也不顶破卡片 */
.log-text {
  flex: 1 1 auto;
  min-width: 0;
  overflow-wrap: anywhere;
}

/* ==================== 进度 / 结果 ==================== */
.progress-card { padding: 14px 16px; }
.progress-text { font-size: 13px; color: var(--dc-text-dim); margin-top: 8px; text-align: center; }
.result-card { padding: 14px 16px; }

/* ==================== 进度弹窗 ==================== */
.progress-dialog :deep(.el-dialog__body) { padding: 16px 20px 10px; max-height: 74vh; overflow: auto; }
.pg-summary { margin-bottom: 14px; }
.pg-text { font-size: 13px; color: var(--dc-text-dim); margin-top: 8px; text-align: center; }

/* 进度标题行：左边「同步进度」+ 运行态图标，右边大号百分比（替代挤在细条里的 text-inside） */
.pg-top {
  display: flex; align-items: center; justify-content: space-between;
  margin-bottom: 8px;
}
.pg-title {
  display: inline-flex; align-items: center; gap: 6px;
  font-size: 13px; font-weight: 600; color: var(--dc-text);
}
.pg-title .el-icon { color: var(--dc-primary); }
.pg-pct {
  font-size: 16px; font-weight: 700; line-height: 1;
  color: var(--dc-primary);
  font-variant-numeric: tabular-nums;
}
.pg-pct.done { color: var(--dc-success); }
.pg-pct.warn { color: var(--dc-warning); }
.pg-pct.err { color: var(--dc-danger); }
/* 进度条只当刻度尺：柔和轨道 + 圆角 + 宽度变化有过渡（百分比本身在上面） */
.run-view :deep(.el-progress-bar__outer) { background: var(--dc-border); border-radius: 999px; }
.run-view :deep(.el-progress-bar__inner) { border-radius: 999px; transition: width .45s ease; }
/* 日志吃掉剩下的高度：拖动弹窗时跟着变（旧的 200/320px 固定值会让拉高后底下空一片） */
.pg-log { margin-bottom: 0; }
.pg-log .log-body { max-height: none; min-height: 96px; }
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
