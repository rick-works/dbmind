<template>
  <el-dialog :model-value="visible" width="720px" top="8vh"
             :close-on-click-modal="false" :close-on-press-escape="false"
             :before-close="onBeforeClose" append-to-body class="task-progress-dialog"
             @update:model-value="$emit('update:visible', $event)">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Loading v-if="isRunning" /><CircleCheckFilled v-else-if="isSuccess" /><CircleCloseFilled v-else /></el-icon></span>
        <span>{{ displayTitle }}</span>
      </div>
    </template>

    <div class="tp-summary">
      <el-progress class="tp-progress" :class="progressClass"
                   :percentage="percentage" :stroke-width="14" text-inside
                   :indeterminate="indeterminate && isRunning"
                   :format="progressFormat"
                   :striped="stripedRunning"
                   :status="progressStatus" />
      <!-- 实时阶段（「已导出 50000 行 24%」之类）：后端每一批都在更新 phase，
           但这里以前只把它算出来却**没有渲染**，于是长任务过程中界面只有一根进度条 -->
      <div class="tp-phase">
        <span>{{ phaseText }}</span>
        <!-- 行/秒读数：后端整页才更新一次 done（实测一页约 0.2~1.3 秒），
             光看那个数字会觉得「一跳一跳」；把速率显示出来，跑得快不快一眼就看得出来。 -->
        <span v-if="speedText" class="tp-speed">{{ speedText }}</span>
        <span v-if="etaText" class="tp-speed">{{ etaText }}</span>
      </div>
    </div>

    <div class="tp-log-card">
      <div ref="logRef" class="tp-log-body">
        <div v-if="!logs.length" class="tp-log-empty">{{ emptyHint }}</div>
        <div v-for="(line, i) in logs" :key="i" class="tp-log-line">
          <span class="tp-log-time">{{ line.time }}</span>
          <span class="tp-log-text">{{ line.text }}</span>
        </div>
      </div>
    </div>

    <template #footer>
      <div class="tp-footer">
        <span class="tp-tip">{{ footerTip }}</span>
        <!-- cancellable=false 的场景：语句已经交给数据库执行，客户端**中断不了** ——
             与其放一个按不动的"取消"，不如不给（危险操作里的单个 DDL 就是这种）。 -->
        <el-button v-if="isRunning && cancellable" type="danger" size="small" :loading="canceling" @click="$emit('cancel')">
          {{ canceling ? $t('tpd.canceling') : $t('common.cancel') }}
        </el-button>
        <el-button v-else type="primary" size="small" @click="$emit('close')">
          {{ isSuccess ? $t('kbx.stepDone') : $t('common.close') }}
        </el-button>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
// 通用「长时任务进度 + 实时日志」弹窗：
// 后端按 taskId 轮询，返回 { status, done, total, phase, message, logs }。
// total <= 0 时进度条进入 indeterminate，避免未知总量时百分比乱跳。
import { computed, watch, nextTick, ref } from 'vue'
import { ElMessage } from 'element-plus'
import { Loading, CircleCheckFilled, CircleCloseFilled, Document } from '@element-plus/icons-vue'

import { t } from '../utils/i18n'

const props = defineProps({
  visible: { type: Boolean, default: false },
  title: { type: String, default: () => t('tpd.running') },
  // 种类键（export / import / dump / runFile），不是中文名 —— 组句由本组件按语言做
  taskKind: { type: String, default: '' },
  targetName: { type: String, default: '' }, // 目标对象名，如 sales
  status: { type: String, default: 'running' }, // running / success / error / canceled
  done: { type: Number, default: 0 },
  total: { type: Number, default: -1 },
  phase: { type: String, default: '' },
  message: { type: String, default: '' },
  logs: { type: Array, default: () => [] },
  canceling: { type: Boolean, default: false },
  /** 该任务能否取消：false = 不给取消按钮（后端已经在执行、停不下来） */
  cancellable: { type: Boolean, default: true }
})

defineEmits(['update:visible', 'cancel', 'close'])

const logRef = ref(null)

const isRunning = computed(() => props.status === 'running')

/**
 * 右上角 X 常驻显示，但任务运行中不直接关：关掉只是「看不见」，任务仍在跑，
 * 用户会以为任务被取消了。这里提示一句，让他走「取消」按钮。
 */
const onBeforeClose = (done) => {
  if (isRunning.value) {
    ElMessage.warning(t('tpd.closeWhileRunning'))
    return
  }
  done()
}
const isSuccess = computed(() => props.status === 'success')
const isError = computed(() => props.status === 'error' || props.status === 'canceled' || (!isRunning.value && !isSuccess.value))

const displayTitle = computed(() => {
  if (props.taskKind) {
    const name = props.targetName ? t('task.target', { name: props.targetName }) : ''
    const k = props.taskKind
    if (isRunning.value) return t(`task.${k}Running`, { name })
    if (isSuccess.value) return t(`task.${k}Done`, { name })
    if (isError.value) return t(`task.${k}Failed`, { name })
    return t(`task.${k}Ended`, { name })
  }
  return props.title || t('task.defaultTitle')
})

const percentage = computed(() => {
  if (isSuccess.value) return 100
  if (!props.total || props.total <= 0) return 0
  return Math.min(100, Math.round((props.done / props.total) * 100))
})
// 总量未知（total<=0）时进不确定态（跑马灯）。
// 原来这里写死 false —— 与文件头注释（total<=0 进 indeterminate）自相矛盾，
// 结果 render_dump 之外拿不到总数的导出百分比恒为 0，只剩一根空条，看着像没在动。
const indeterminate = computed(() => !isSuccess.value && !isError.value && (!props.total || props.total <= 0))

const progressStatus = computed(() => {
  if (isError.value) return 'exception'
  if (isSuccess.value) return 'success'
  return ''
})

const progressClass = computed(() => {
  if (isSuccess.value) return 'is-success'
  if (isError.value) return 'is-exception'
  return 'is-running'
})

// 不确定态下不显示百分比（后端都不知道总量，显示 0% 只会误导）
// 行/秒：只对「按行推进」的导出任务有意义（dump 的 done 是"表数"，显示成行/秒就错了）。
const lastSample = ref({ done: 0, at: Date.now() })
const rowsPerSec = ref(0)
watch(() => props.done, (v) => {
  const now = Date.now()
  const dt = now - lastSample.value.at
  if (dt < 500) return
  const dv = v - lastSample.value.done
  if (dv > 0) rowsPerSec.value = Math.round(dv / (dt / 1000))
  lastSample.value = { done: v, at: now }
})
watch(() => props.visible, (v) => {
  if (v) { rowsPerSec.value = 0; lastSample.value = { done: 0, at: Date.now() } }
})
const speedText = computed(() => {
  if (!isRunning.value || !rowsPerSec.value) return ''
  if (props.taskKind !== 'export') return ''
  return t('tpd.rowsPerSec', { n: rowsPerSec.value.toLocaleString() })
})

/** 剩余时间：只有"按行推进 + 已知总量 + 已有速率"时才敢估（其余一律不显示，宁缺勿错） */
const etaText = computed(() => {
  if (!isRunning.value || props.taskKind !== 'export') return ''
  if (!rowsPerSec.value || !props.total || props.total <= 0) return ''
  const left = props.total - props.done
  if (left <= 0) return ''
  const sec = Math.max(1, Math.round(left / rowsPerSec.value))
  const mm = Math.floor(sec / 60)
  const ss = String(sec % 60).padStart(2, '0')
  return t('tpd.eta', { t: mm > 0 ? `${mm}:${ss}` : `${sec}s` })
})

const progressFormat = computed(() => (pct) => (indeterminate.value ? '' : `${pct}%`))
const stripedRunning = computed(() => false)

const phaseClass = computed(() => {
  if (isSuccess.value) return 'is-success'
  if (isError.value) return 'is-error'
  return ''
})

const phaseText = computed(() => {
  const pct = percentage.value
  if (isSuccess.value) return t('tpd.donePct', { n: pct })
  if (isError.value) return pct > 0 ? t('tpd.failedPct', { n: pct }) : t('kbx.phase.failed')
  if (props.phase) return pct > 0 ? `${props.phase} ${pct}%` : props.phase
  return pct > 0 ? t('tpd.runningPct', { n: pct }) : t('tpd.running')
})

const footerTip = computed(() => {
  if (isRunning.value) return props.canceling ? t('tpd.waitingStop') : t('tpd.inProgress')
  if (isSuccess.value) return t('tpd.completed')
  return t('tpd.ended')
})

/**
 * 日志区为空时显示什么。
 *
 * **不能一律写「等待开始…」**：导出这类任务的详细日志是按批写的（导出一批 5 万行才一条），
 * 大表上前几万行可能要好几分钟 —— 这段时间进度其实一直在走，界面却始终说「等待开始」，
 * 用户只会以为卡住了（实测就是这么报上来的）。所以：运行中显示当前阶段，结束后显示结果。
 */
const emptyHint = computed(() => {
  if (isRunning.value) return props.phase ? `${props.phase}…` : t('tpd.started')
  return props.message || t('tpd.ended')
})

watch(() => props.logs.length, () => {
  nextTick(() => {
    const el = logRef.value
    if (el) el.scrollTop = el.scrollHeight
  })
}, { flush: 'post' })
</script>

<style scoped>
.dlg-title { display: flex; align-items: center; gap: 8px; font-weight: 500; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
}
.tp-summary { padding: 4px 0 16px; }
.tp-progress { margin-bottom: 6px; }
.tp-speed { margin-left: 8px; color: var(--dc-text-dim); font-variant-numeric: tabular-nums; }
.tp-phase { font-size: 13px; color: var(--dc-text-mid); }
.tp-progress.is-running :deep(.el-progress-bar__inner) {
  background-color: var(--dc-primary);
  background-image: linear-gradient(90deg, var(--dc-primary), var(--dc-purple));
}
.tp-progress.is-success :deep(.el-progress-bar__inner) { background-image: none; background-color: #67c23a; }
.tp-progress.is-exception :deep(.el-progress-bar__inner) { background-image: none; background-color: #f56c6c; }
.tp-progress :deep(.el-progress-bar__innerText) { font-weight: 600; }
.tp-phase { margin-top: 10px; font-size: 14px; color: var(--dc-text-strong); font-weight: 500; }
.tp-phase.is-success { color: #67c23a; }
.tp-phase.is-error { color: #f56c6c; }
.tp-message { margin-top: 4px; font-size: 13px; color: var(--dc-text-weak); line-height: 1.5; }
.tp-log-card {
  border: 1px solid var(--dc-border); border-radius: 8px; background: var(--dc-bg-soft);
  overflow: hidden;
}
.tp-log-head {
  display: flex; align-items: center; justify-content: space-between;
  padding: 8px 12px; font-size: 13px; color: var(--dc-text-mid);
  border-bottom: 1px solid var(--dc-border);
}
.tp-log-head span { display: inline-flex; align-items: center; gap: 6px; }
.tp-log-count { color: var(--dc-text-weak); }
.tp-log-body {
  height: 220px; overflow-y: auto; padding: 8px 12px; font-size: 13px; line-height: 1.7;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
}
.tp-log-empty {
  height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center;
  color: var(--dc-text-weak); gap: 6px;
}
.tp-log-line { display: flex; gap: 10px; color: var(--dc-text); }
.tp-log-time { color: var(--dc-text-weak); flex-shrink: 0; }
.tp-log-text { word-break: break-all; }
.tp-footer { display: flex; align-items: center; justify-content: space-between; width: 100%; }
.tp-tip { font-size: 13px; color: var(--dc-text-weak); }
</style>
