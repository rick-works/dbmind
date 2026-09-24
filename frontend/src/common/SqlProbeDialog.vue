<template>
  <!-- 尺寸 / 位置与 AI 结果弹窗（SqlQueryView 的 width=640px top=8vh）保持一致：
       两者是从同一个结论里点出来的上下级，外框忽大忽小、位置忽上忽下会很明显 -->
  <el-dialog v-model="visible" width="640px" top="8vh" append-to-body destroy-on-close
             :title="mode === 'plan' ? $t('spd.plan') : $t('spd.result')" class="probe-dialog">
    <div class="probe-head">
      <span class="probe-badge" :class="mode">{{ mode === 'plan' ? $t('spd.plan') : $t('spd.trial', { n: limit }) }}</span>
      <span v-if="database" class="probe-meta">{{ database }}</span>
      <div class="probe-acts">
        <el-button size="small" :icon="Refresh" :loading="loading" @click="load">{{ $t('spd.rerun') }}</el-button>
        <el-button size="small" :icon="DocumentCopy" @click="copyAll">{{ $t('ofd.copy') }}</el-button>
        <el-button size="small" type="primary" :icon="CaretRight" @click="$emit('insert', sql)">{{ $t('spd.insert') }}</el-button>
      </div>
    </div>

    <pre class="probe-sql">{{ sql }}</pre>

    <div v-if="loading" class="probe-state">
      <el-icon class="is-loading"><Loading /></el-icon>
      <span>{{ mode === 'plan' ? $t('spd.planLoading') : $t('spd.runningReadonly') }}</span>
    </div>
    <!-- 失败：命中规则时先给「人话」，原始错误折叠在后面；没命中就如实展示原文 -->
    <div v-else-if="error" class="probe-error">
      <template v-if="hint && hint.matched">
        <div class="probe-hint-head">
          <span class="probe-hint-title">{{ hint.title }}</span>
          <span class="probe-hint-reason">{{ hint.reason }}</span>
        </div>
        <ul class="probe-hint-actions">
          <li v-for="(a, i) in hint.actions" :key="i">{{ a }}</li>
        </ul>
        <details class="probe-raw">
          <summary>{{ $t('spd.rawError') }}</summary>
          <pre>{{ error }}</pre>
        </details>
      </template>
      <template v-else>
        <div class="probe-hint-head">
          <span class="probe-hint-title">{{ $t('spd.failed') }}</span>
        </div>
        <pre class="probe-raw-plain">{{ error }}</pre>
      </template>
    </div>

    <!-- 执行计划：交给数据库自己解释 -->
    <pre v-else-if="mode === 'plan'" class="probe-plan">{{ plan }}</pre>

    <!-- 试跑：小表格，行数由后端强制限行 -->
    <template v-else>
      <div class="probe-stat">
        {{ $t('spd.rows', { n: rows.length }) }}<template v-if="hasMore">{{ $t('spd.truncated') }}</template>
        <span class="probe-dot">·</span>{{ $t('spd.elapsed', { n: elapsedMs }) }}
      </div>
      <div class="probe-table-wrap">
        <table class="probe-table">
          <thead>
            <tr><th v-for="c in columns" :key="c">{{ c }}</th></tr>
          </thead>
          <tbody>
            <tr v-for="(r, i) in rows" :key="i">
              <td v-for="c in columns" :key="c" :title="cell(r, c)">{{ cell(r, c) }}</td>
            </tr>
          </tbody>
        </table>
        <div v-if="!rows.length" class="probe-empty">{{ $t('spd.noRows') }}</div>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { t } from '../utils/i18n'
import { ElMessage } from 'element-plus'
import { Refresh, DocumentCopy, CaretRight, Loading } from '@element-plus/icons-vue'
import { aiSqlPlan, aiSqlTryRun } from '../api'

/**
 * SQL 快捷验证弹窗：给 AI 产出的 SQL 一个「先验证再用」的入口。
 *
 * 两种模式都**不消耗 AI 调用**，走的是后端同一套只读护栏
 * （`AiDataTools.isReadOnly` + 强制限行），所以写语句、加锁读会被直接拒掉，
 * 这里只需要如实展示结果或拒绝原因。
 */
const props = defineProps({
  modelValue: { type: Boolean, default: false },
  /** 待验证的 SQL */
  sql: { type: String, default: '' },
  connId: { type: [String, Number], default: '' },
  database: { type: String, default: '' },
  /** plan = 执行计划；run = 试跑取前 N 行 */
  mode: { type: String, default: 'run' }
})
const emit = defineEmits(['update:modelValue', 'insert'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

/** 试跑行数上限：与后端默认一致（后端还会再夹到 500） */
const limit = 100

const loading = ref(false)
const error = ref('')
/** 后端给出的「人话」解释（DbErrorUtil.explain 的结果）：命中时优先展示它，原始错误折叠在后面 */
const hint = ref(null)
const plan = ref('')
const columns = ref([])
const rows = ref([])
const hasMore = ref(false)
const elapsedMs = ref(0)

const cell = (row, col) => {
  const v = row ? row[col] : null
  return v === null || v === undefined ? '' : String(v)
}

const load = async () => {
  loading.value = true
  error.value = ''
  hint.value = null
  plan.value = ''
  columns.value = []
  rows.value = []
  hasMore.value = false
  elapsedMs.value = 0
  try {
    if (!props.connId) throw new Error(t('spd.pickConn'))
    if (props.mode === 'plan') {
      const res = await aiSqlPlan({ sql: props.sql, connectionId: props.connId, database: props.database })
      if (!res || !res.success) {
        hint.value = res?.error || null
        throw new Error(res?.message || t('spd.planFailed'))
      }
      plan.value = res.plan || t('spd.noPlan')
    } else {
      const res = await aiSqlTryRun({
        sql: props.sql, connectionId: props.connId, database: props.database, limit
      })
      if (!res || !res.success) {
        hint.value = res?.error || null
        throw new Error(res?.message || t('spd.trialFailed'))
      }
      columns.value = res.columns || []
      rows.value = res.rows || []
      hasMore.value = !!res.hasMore
      elapsedMs.value = res.elapsedMs || 0
    }
  } catch (e) {
    error.value = e?.message || String(e)
  }
  loading.value = false
}

/** 打开即执行一次（每次打开都取最新结果，避免展示上一次的陈旧数据） */
watch(() => props.modelValue, (v) => { if (v) load() })

const copyAll = async () => {
  const text = props.mode === 'plan'
    ? plan.value
    : [columns.value.join('\t'), ...rows.value.map(r => columns.value.map(c => cell(r, c)).join('\t'))].join('\n')
  try {
    await navigator.clipboard.writeText(text)
    ElMessage.success(t('ai.copied'))
  } catch (e) {
    ElMessage.error(t('spd.copyManual'))
  }
}
</script>

<style scoped>
.probe-head {
  display: flex; align-items: center; gap: 8px; flex-wrap: wrap;
  margin-bottom: 10px;
}
.probe-badge {
  font-size: 12px; padding: 2px 8px; border-radius: 5px;
  background: var(--dc-primary-wash); color: var(--dc-primary);
  border: 1px solid var(--dc-border-soft);
}
.probe-badge.plan { background: var(--dc-bg-code); color: var(--dc-text-mid); }
.probe-meta { font-size: 13px; color: var(--dc-text-dim); }
.probe-acts { margin-left: auto; display: flex; gap: 6px; }

.probe-sql {
  margin: 0 0 12px; padding: 10px 12px; max-height: 160px; overflow: auto;
  background: var(--dc-bg-code); border: 1px solid var(--dc-border-soft); border-radius: 8px;
  font-family: "SF Mono", "JetBrains Mono", Consolas, monospace; font-size: 13px;
  line-height: 1.6; color: var(--dc-code-text); white-space: pre-wrap; word-break: break-word;
}
.probe-state {
  display: flex; align-items: center; gap: 8px;
  padding: 18px 0; font-size: 14px; color: var(--dc-text-dim);
}
/* 失败区：命中规则先给「人话」，原始错误折叠在后面 */
.probe-error { font-size: 14px; color: var(--dc-text); }
.probe-hint-head { display: flex; align-items: baseline; gap: 8px; flex-wrap: wrap; }
.probe-hint-title {
  font-weight: 600; font-size: 14px; color: var(--dc-danger);
  padding: 2px 8px; border-radius: 5px;
  background: color-mix(in srgb, var(--dc-danger) 12%, transparent);
}
.probe-hint-reason { color: var(--dc-text-mid); font-size: 14px; }
.probe-hint-actions {
  margin: 8px 0 0 18px; padding: 0;
  color: var(--dc-text-dim); font-size: 13px; line-height: 1.9;
}
.probe-raw { margin-top: 10px; }
.probe-raw summary { cursor: pointer; font-size: 13px; color: var(--dc-text-dim); user-select: none; }
.probe-raw pre, .probe-raw-plain {
  margin: 8px 0 0; padding: 10px 12px; max-height: 32vh; overflow: auto;
  background: var(--dc-bg-code); border: 1px solid var(--dc-border-soft); border-radius: 8px;
  font-family: "SF Mono", "JetBrains Mono", Consolas, monospace; font-size: 13px;
  line-height: 1.6; color: var(--dc-code-text); white-space: pre-wrap; word-break: break-word;
}
.probe-plan {
  margin: 0; padding: 12px; max-height: 46vh; overflow: auto;
  background: var(--dc-bg-code); border: 1px solid var(--dc-border-soft); border-radius: 8px;
  font-family: "SF Mono", "JetBrains Mono", Consolas, monospace; font-size: 13px;
  line-height: 1.6; color: var(--dc-code-text); white-space: pre-wrap; word-break: break-word;
}
.probe-stat { font-size: 13px; color: var(--dc-text-dim); margin-bottom: 8px; }
.probe-dot { margin: 0 6px; }
.probe-table-wrap {
  max-height: 46vh; overflow: auto;
  border: 1px solid var(--dc-border-soft); border-radius: 8px;
}
.probe-table { border-collapse: collapse; width: 100%; font-size: 13px; }
.probe-table th, .probe-table td {
  padding: 6px 10px; text-align: left; white-space: nowrap;
  border-bottom: 1px solid var(--dc-border-soft);
  max-width: 260px; overflow: hidden; text-overflow: ellipsis;
}
.probe-table th {
  position: sticky; top: 0; z-index: 1;
  background: var(--dc-bg-table-head); color: var(--dc-text-strong); font-weight: 600;
}
.probe-table tr:last-child td { border-bottom: none; }
.probe-empty { padding: 18px; text-align: center; font-size: 13px; color: var(--dc-text-dim); }
</style>
