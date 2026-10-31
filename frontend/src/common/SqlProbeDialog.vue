<template>
  <!-- 尺寸 / 位置与 AI 结果弹窗（SqlQueryView 的 width=640px top=8vh）保持一致：
       两者是从同一个结论里点出来的上下级，外框忽大忽小、位置忽上忽下会很明显 -->
  <el-dialog v-model="visible" :width="dialogWidth" top="8vh" append-to-body destroy-on-close
             :title="mode === 'plan' ? $t('spd.plan') : $t('spd.result')" class="probe-dialog">
    <div class="probe-head">
      <span class="probe-badge" :class="mode">{{ mode === 'plan' ? $t('spd.plan') : $t('spd.trial', { n: limit }) }}</span>
      <span v-if="database" class="probe-meta">{{ database }}</span>
      <!-- 顶部只留"对这个结果本身"的动作（重跑）；复制 + 插入编辑器是"收尾动作"，
           统一放右下角 footer，与「生成 SQL」等弹窗同一位置 -->
      <div class="probe-acts">
        <el-button size="small" :icon="Refresh" :loading="loading" @click="load">{{ $t('spd.rerun') }}</el-button>
      </div>
    </div>

    <pre class="probe-sql dc-code-block sql">{{ sql }}</pre>

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

    <!-- 执行计划：先讲「这是什么 + 这条 SQL 慢在哪」，再画表，原文折叠在最后。
         只甩一段 `Using where | 10.0 | 3 | … | ALL` 的话，用户根本看不出重点在哪。 -->
    <template v-else-if="mode === 'plan'">
      <div class="plan-intro">{{ $t('spd.planIntro') }}</div>

      <div v-if="planVerdict" class="plan-verdict" :class="'lv-' + planVerdict.level">
        <el-icon :size="15"><component :is="verdictIcon" /></el-icon>
        <div class="pv-body">
          <div class="pv-title">{{ planVerdict.title }}</div>
          <div class="pv-detail">{{ planVerdict.detail }}</div>
        </div>
      </div>
      <ul v-if="planRest.length" class="plan-more">
        <li v-for="(one, i) in planRest" :key="i" :class="'lv-' + one.level">
          <span class="pm-title">{{ one.title }}</span>
          <span class="pm-detail">{{ one.detail }}</span>
        </li>
      </ul>

      <div v-if="planColumns.length" class="plan-table-wrap">
        <table class="plan-table">
          <thead>
            <tr>
              <th v-for="c in planColumns" :key="c" :title="c">{{ colLabel(c) }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(r, i) in planRows" :key="i">
              <td v-for="c in planColumns" :key="c" :class="cellClass(c, r)" :title="cell(r, c)">
                {{ cell(r, c) || '—' }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <details v-if="planColumns.length" class="plan-raw">
        <summary>{{ $t('spd.planRaw') }}</summary>
        <pre class="probe-plan">{{ plan }}</pre>
      </details>
      <pre v-else class="probe-plan">{{ plan }}</pre>
    </template>

    <!-- 试跑：小表格，行数由后端强制限行 -->
    <template v-else>
      <div class="probe-stat">
        {{ $t('spd.rows', { n: rows.length }) }}<template v-if="hasMore">{{ $t('spd.truncated') }}</template>
        <span class="probe-dot">·</span>{{ $t('spd.elapsed', { n: elapsedMs }) }}
      </div>
      <!-- 服务端说明（例如「当前会话没有默认库：database() 取不到东西」）：
           没有这段时，「0 行」看起来就像「库里真没数据」 -->
      <ul v-if="notices.length" class="probe-notes">
        <li v-for="(note, i) in notices" :key="i">{{ note }}</li>
      </ul>
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

    <template #footer>
      <el-button size="small" :icon="DocumentCopy" @click="copyAll">{{ $t('ofd.copy') }}</el-button>
      <el-button size="small" type="primary" :icon="CaretRight" @click="$emit('insert', sql)">{{ $t('spd.insert') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch } from 'vue'

import { t } from '../utils/i18n'
import { ElMessage } from 'element-plus'
import {
  Refresh, DocumentCopy, CaretRight, Loading,
  CircleCheckFilled, CircleCloseFilled, WarningFilled
} from '@element-plus/icons-vue'
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
/** 执行计划的结构化形态（MySQL 风格才有）：给画表用，只有原文的话界面上就是一串天书 */
const planColumns = ref([])
const planRows = ref([])
/** 后端规则式解读：走没走索引 / 扫多少行 / 有没有额外排序，最多 4 条 */
const planInsights = ref([])
const columns = ref([])
const rows = ref([])
const hasMore = ref(false)
const elapsedMs = ref(0)
/** 服务端附带的说明（截断、MySQL warning、未选定默认库…）：空数组时不渲染 */
const notices = ref([])

/** 执行计划表比试跑表宽（12 列），弹窗跟着放宽一点 */
const dialogWidth = computed(() => (props.mode === 'plan' ? '900px' : '640px'))

const planVerdict = computed(() => planInsights.value[0] || null)
const planRest = computed(() => planInsights.value.slice(1))
const verdictIcon = computed(() => {
  const level = planVerdict.value?.level
  if (level === 'high') return CircleCloseFilled
  if (level === 'medium') return WarningFilled
  return CircleCheckFilled
})

/** EXPLAIN 的列名是数据库方言的英文缩写，表头换成中文；认不出来的列原样显示 */
const COL_LABEL_KEYS = {
  id: 'spd.colId',
  select_type: 'spd.colSelectType',
  table: 'spd.colTable',
  partitions: 'spd.colPartitions',
  type: 'spd.colType',
  possible_keys: 'spd.colPossibleKeys',
  key: 'spd.colKey',
  key_len: 'spd.colKeyLen',
  ref: 'spd.colRef',
  rows: 'spd.colRows',
  filtered: 'spd.colFiltered',
  extra: 'spd.colExtra',
  detail: 'spd.colDetail',
  'query plan': 'spd.colPlan'
}
const colLabel = (name) => {
  const key = COL_LABEL_KEYS[String(name || '').toLowerCase()]
  return key ? t(key) : name
}

const cell = (row, col) => {
  const v = row ? row[col] : null
  return v === null || v === undefined ? '' : String(v)
}

/** 计划表单元格着色：全表扫描 / 没用索引 / 额外排序是要用户注意的三件事 */
const cellClass = (col, row) => {
  const name = String(col || '').toLowerCase()
  const value = cell(row, col).toUpperCase()
  if (name === 'type') {
    if (value === 'ALL') return 'is-bad'
    if (value === 'INDEX') return 'is-warn'
    return value ? 'is-good' : ''
  }
  if (name === 'key') return value ? 'is-good' : 'is-warn'
  if (name === 'extra') {
    const lower = value.toLowerCase()
    if (lower.includes('filesort') || lower.includes('temporary')) return 'is-warn'
  }
  return ''
}

const load = async () => {
  loading.value = true
  error.value = ''
  hint.value = null
  plan.value = ''
  planColumns.value = []
  planRows.value = []
  planInsights.value = []
  columns.value = []
  rows.value = []
  hasMore.value = false
  elapsedMs.value = 0
  notices.value = []
  try {
    if (!props.connId) throw new Error(t('spd.pickConn'))
    if (props.mode === 'plan') {
      const res = await aiSqlPlan({ sql: props.sql, connectionId: props.connId, database: props.database })
      if (!res || !res.success) {
        hint.value = res?.error || null
        throw new Error(res?.message || t('spd.planFailed'))
      }
      plan.value = res.plan || t('spd.noPlan')
      planColumns.value = res.columns || []
      planRows.value = res.rows || []
      planInsights.value = res.insights || []
    } else {
      const res = await aiSqlTryRun({
        sql: props.sql, connectionId: props.connId, database: props.database, limit
      })
      if (!res || !res.success) {
        hint.value = res?.error || null
        throw new Error(res?.message || t('spd.trialFailed'))
      }
      // 平铺字段优先，缺失时回退到后端那份完整 `result`（字段名是 executeTime）。
      // 曾经这里只读平铺、而接口只给嵌套：查询返回 426 行，弹窗却恒显示「0 行 · 0 ms」。
      const data = res.columns || res.rows ? res : (res.result || {})
      columns.value = data.columns || []
      rows.value = data.rows || []
      hasMore.value = !!data.hasMore
      elapsedMs.value = data.executeTime ?? data.elapsedMs ?? 0
      // 截断提示已经在统计行里说过一次，这里只留真正需要用户动手的那些说明
      notices.value = (res.notices || data.notices || []).filter(n => !/已截断/.test(String(n)))
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

/* 外观（底/边框/圆角/字体/颜色）走全局 .dc-code-block，这里只留本弹窗特有的间距：
   它在正文顶部、固定高度（滚动的结果区在下面，不该跟它抢高度） */
.probe-sql { margin: 0 0 12px; max-height: 160px; }
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
  background: var(--dc-code-bg); border: 1px solid var(--dc-border-soft); border-radius: 8px;
  font-family: "SF Mono", "JetBrains Mono", Consolas, monospace; font-size: 13px;
  line-height: 1.6; color: var(--dc-code-text); white-space: pre-wrap; word-break: break-word;
}
/* 全屏 / 拖大后：执行计划与试跑结果吃掉正文剩余高度（原来封顶 46vh，下面空一大片）。
   正文变 flex 纵列，SQL 预览与提示行按内容、主内容区 flex:1。 */
:global(.el-dialog.probe-dialog > .el-dialog__body) {
  display: flex; flex-direction: column; overflow: hidden;
}
.probe-head, .probe-stat, .probe-sql { flex: none; }
.probe-plan {
  margin: 0; padding: 12px; flex: 1 1 auto; min-height: 120px; overflow: auto;
  background: var(--dc-code-bg); border: 1px solid var(--dc-border-soft); border-radius: 8px;
  font-family: "SF Mono", "JetBrains Mono", Consolas, monospace; font-size: 13px;
  line-height: 1.6; color: var(--dc-code-text); white-space: pre-wrap; word-break: break-word;
}
/* 执行计划：先说人话（这是什么 / 这条 SQL 慢在哪），再上表格，原文折叠 */
.plan-intro {
  flex: none; margin: 0 0 10px; padding: 9px 12px;
  border-radius: 8px; border: 1px solid var(--dc-border-soft);
  background: var(--dc-bg-soft); color: var(--dc-text-mid);
  font-size: 12.5px; line-height: 1.7;
}
/* 结论卡片：与 SQL 查询里那几块证据卡同一套语言 —— 中性底 + 左侧 3px 引导条，
   颜色只落在引导条与图标上。整块刷红/黄/绿在深色主题里太吵（真机反馈）。 */
.plan-verdict {
  position: relative; flex: none; display: flex; gap: 10px; align-items: flex-start;
  padding: 10px 12px 10px 15px; margin-bottom: 10px; border-radius: 8px;
  border: 1px solid var(--dc-border); background: var(--dc-bg-soft);
}
.plan-verdict::before {
  content: ''; position: absolute; left: 0; top: 10px; bottom: 10px; width: 3px;
  border-radius: 0 3px 3px 0; background: var(--dc-border);
}
.plan-verdict .pv-title { font-size: 13.5px; font-weight: 600; }
.plan-verdict .pv-detail { margin-top: 3px; font-size: 12.5px; line-height: 1.7; color: var(--dc-text-mid); }
.plan-verdict.lv-high::before { background: var(--dc-danger); }
.plan-verdict.lv-high :deep(.el-icon) { color: var(--dc-danger); }
.plan-verdict.lv-medium::before { background: var(--dc-warning); }
.plan-verdict.lv-medium :deep(.el-icon) { color: var(--dc-warning); }
.plan-verdict.lv-low::before { background: var(--dc-success); }
.plan-verdict.lv-low :deep(.el-icon) { color: var(--dc-success); }
.plan-more {
  flex: none; margin: 0 0 10px; padding-left: 18px;
  font-size: 12.5px; line-height: 1.8; color: var(--dc-text-mid);
}
.plan-more li { margin-bottom: 2px; }
.plan-more .pm-title { font-weight: 600; margin-right: 6px; color: var(--dc-text); }
.plan-more li.lv-medium .pm-title { color: var(--dc-warning); }
.plan-more li.lv-low .pm-title { color: var(--dc-success); }
/* 计划表按内容自适应高度（1 行计划不该撑出一个空盒子），行多时自己滚 */
.plan-table-wrap {
  flex: 0 1 auto; min-height: 0; max-height: 46vh; overflow: auto;
  border: 1px solid var(--dc-border-soft); border-radius: 8px;
}
.plan-table { border-collapse: collapse; width: max-content; min-width: 100%; font-size: 12.5px; }
.plan-table th, .plan-table td {
  padding: 6px 10px; text-align: left; white-space: nowrap;
  border-bottom: 1px solid var(--dc-border-soft);
}
/* Extra 可能是长句（Using where; Using filesort），让它换行而不是把表格顶出横向滚动 */
.plan-table th:last-child, .plan-table td:last-child {
  white-space: normal; min-width: 120px; max-width: 260px; word-break: break-word;
}
.plan-table th {
  position: sticky; top: 0; z-index: 1;
  background: var(--dc-bg-table-head); color: var(--dc-text-strong); font-weight: 600;
}
.plan-table tr:last-child td { border-bottom: none; }
.plan-table td.is-bad { color: var(--dc-danger); font-weight: 600; }
.plan-table td.is-warn { color: var(--dc-warning); }
.plan-table td.is-good { color: var(--dc-success); }
.plan-raw { flex: none; margin-top: 10px; }
.plan-raw summary { cursor: pointer; font-size: 12.5px; color: var(--dc-text-dim); user-select: none; }
.plan-raw .probe-plan { flex: none; max-height: 26vh; margin-top: 8px; }

.probe-stat { font-size: 13px; color: var(--dc-text-dim); margin-bottom: 8px; }
.probe-dot { margin: 0 6px; }
/* 服务端说明：用警示色边框而不是整块底色，避免和「失败」区抢注意力 */
.probe-notes {
  margin: 0 0 8px; padding: 8px 10px 8px 24px; list-style: disc;
  font-size: 12.5px; line-height: 1.7; color: var(--dc-text-mid);
  border-left: 3px solid var(--dc-warning); border-radius: 4px;
  background: var(--dc-warn-wash);
}
.probe-notes li + li { margin-top: 2px; }
.probe-table-wrap {
  flex: 1 1 auto; min-height: 120px; overflow: auto;
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
