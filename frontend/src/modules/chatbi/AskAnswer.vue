<template>
  <!-- 一条问数回答的渲染：执行过程（每步可单独展开）→ 图表 → 数据概况 → 结论/洞察 → 明细 → 追问 → 用量。
       先看"怎么算出来的"和结果，再看结论 —— 所以结论区放在图表与数据之后。
       SQL 只出现在执行过程那一步里（不在这里再贴一遍），要复用就点那一步的「插入编辑器」。
       不含输入框、不含数据源选择 —— 那些归宿主（AI 助手的统一对话框）管。 -->
  <div v-if="answer" class="ask-answer" :class="{ 'is-live': answer.streaming }">
    <!-- 需要澄清：只反问，不出图 -->
    <div v-if="answer.intent === 'clarify'" class="clarify">
      <div class="clarify-title"><el-icon><QuestionFilled /></el-icon>还需要你补充一点信息</div>
      <div class="clarify-text">{{ answer.clarify }}</div>
      <div v-if="answer.followups?.length" class="chips">
        <button v-for="(f, i) in answer.followups" :key="i" class="chip" @click="emit('ask', f)">{{ f }}</button>
      </div>
    </div>

    <template v-else>
      <!-- 还没收到第一帧（网络慢）：先给一句明确的等待文案，不要留白 -->
      <div v-if="answer.streaming && !answer.steps?.length" class="ask-wait">
        <el-icon class="is-loading"><Loading /></el-icon>
        <span>正在分析：先读表结构，再写查询</span>
      </div>

      <!-- 执行过程：进行中一直展开、一步步点亮；跑完自动合并成一行（CodeBuddy 同款），点开再看细节 -->
      <div v-if="answer.steps?.length" class="exec" :class="{ live: answer.streaming }">
        <button type="button" class="exec-head" :disabled="answer.streaming" @click="execOpen = !execOpen">
          <el-icon v-if="!answer.streaming" class="exec-caret" :class="{ rot: execOpen }"><CaretRight /></el-icon>
          <el-icon v-if="answer.streaming" class="exec-spin is-loading"><Loading /></el-icon>
          <el-icon v-else class="exec-ok"><CircleCheckFilled /></el-icon>
          <span class="exec-name">{{ answer.streaming ? '正在分析…' : '执行过程' }}</span>
          <span class="exec-brief">
            {{ answer.streaming ? `已完成 ${doneCount} 步` : execBrief }}
          </span>
          <span v-if="!answer.streaming && elapsedText" class="exec-cost">{{ elapsedText }}</span>
        </button>
        <ol v-show="execOpen || answer.streaming" class="exec-list">
          <li v-for="(s, i) in answer.steps" :key="s.key || i" class="exec-item"
              :class="{ running: s.status === 'running', open: isStepOpen(s, i) }">
            <!-- 每一步自己一个开关：默认收起，点开才看这一步的思考/输入/SQL/输出 -->
            <button type="button" class="exec-step-head" @click="toggleStep(s, i)">
              <span class="exec-dot">
                <el-icon v-if="s.status === 'running'" class="is-loading"><Loading /></el-icon>
                <el-icon v-else><CircleCheck /></el-icon>
              </span>
              <span class="exec-label">{{ s.label }}</span>
              <span v-if="s.ms !== undefined && s.ms !== null" class="exec-ms">{{ s.ms }} ms</span>
              <el-icon class="exec-step-caret" :class="{ rot: isStepOpen(s, i) }"><CaretRight /></el-icon>
            </button>
            <div v-show="isStepOpen(s, i)" class="exec-step-body">
              <div v-if="stepDetail(s)" class="exec-detail">{{ stepDetail(s) }}</div>
              <!-- 分析过程：模型怎么想到这一步的（原样展示，不做二次加工） -->
              <div v-if="s.thinking" class="exec-block">
                <div class="blk-label"><el-icon><MagicStick /></el-icon>分析过程</div>
                <div class="blk-text blk-think">{{ s.thinking }}</div>
              </div>
              <!-- 这一步产出的 SQL：与 AI 对话里的代码块同一套外观，操作按钮悬浮在块右上角 -->
              <div v-if="s.code" class="exec-block">
                <div class="blk-code-wrap">
                  <pre class="blk-code">{{ fmtSql(s.code) }}</pre>
                  <span class="blk-acts">
                    <el-tooltip content="复制" placement="top">
                      <el-button link size="small" :icon="CopyDocument" @click="copy(s.code)" />
                    </el-tooltip>
                    <el-tooltip content="插入编辑器" placement="top">
                      <el-button link size="small" :icon="EditPen" data-act="insert-sql" @click="emit('insert-sql', s.code)" />
                    </el-tooltip>
                  </span>
                </div>
              </div>
              <!-- 分析结果：这一步最终得出了什么 -->
              <div v-if="s.output" class="exec-block">
                <div class="blk-label">分析结果</div>
                <div class="blk-text">{{ s.output }}</div>
              </div>
            </div>
          </li>
        </ol>
      </div>

      <!-- 没有数据时：不给空图表、不给空洞察，直接说清「查成功了，但没匹配到数据」 -->
      <div v-if="!hasRows && !answer.streaming" class="empty-hint">
        <el-icon><InfoFilled /></el-icon>
        <span>查询已成功执行，返回 0 行。换个时间范围，或减少筛选条件再试一次。</span>
      </div>

      <!-- 图表（可能多张）：只在真有数据行时画 —— 空结果画出来只是一片空白，占地方还误导 -->
      <div v-for="(chart, ci) in visibleCharts" :key="ci" class="chart-card">
        <div class="chart-head">
          <div class="chart-title">
            {{ chart.title || '查询结果' }}
            <span v-if="answer.chartFallback && ci === 0" class="tag-fallback">自动选择</span>
          </div>
          <div class="chart-ops">
            <el-dropdown trigger="click" @command="(type) => setChartType(ci, type)">
              <el-button link size="small">
                <el-icon><Switch /></el-icon>{{ typeLabel(ci) }}
              </el-button>
              <template #dropdown>
                <el-dropdown-menu>
                  <el-dropdown-item v-for="t in CHART_TYPES" :key="t.key" :command="t.key">
                    {{ t.label }}
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
            <el-tooltip content="下载 PNG" placement="top">
              <el-button v-if="typeOf(ci) !== 'table'" link size="small" :icon="Download" @click="downloadChart(ci)" />
            </el-tooltip>
          </div>
        </div>
        <ChartCanvas
          v-if="typeOf(ci) !== 'table'"
          :ref="(el) => setChartRef(ci, el)"
          :spec="chart"
          :data="answer"
          :type-override="chartTypes[ci] || ''"
          :height="280"
        />
        <div v-else class="chart-table">
          <DataTable :answer="answer" :limit="100" />
        </div>
      </div>

      <!-- 数据概况：一句文字说明，数字全部由结果集本地算（不调模型，和明细表逐格对得上） -->
      <div v-if="profile.items.length" class="profile">
        <div class="profile-head">
          <el-icon class="profile-icon"><DataAnalysis /></el-icon>
          <span class="profile-title">数据概况</span>
          <span class="pf-chip">{{ profile.rowsText }}</span>
          <span class="pf-chip">{{ profile.cols }} 列</span>
          <span v-if="profile.partial" class="pf-chip warn" :title="profile.partialHint">
            仅统计前 {{ profile.shown }} 行
          </span>
        </div>
        <p class="profile-text">{{ profileText }}</p>
      </div>

      <!-- 结论与洞察：放在图表/数据之后 —— 先看结果，再看解读 -->
      <div class="answer-conclusion">
        <div class="concl-head"><el-icon class="concl-icon"><MagicStick /></el-icon><span class="concl-title">结论</span></div>
        <div class="summary" v-html="md(answer.summary || '（模型没有给出结论）')"></div>
        <ul v-if="hasRows && answer.insights?.length" class="insights">
          <li v-for="(ins, i) in answer.insights" :key="i" v-html="md(ins)"></li>
        </ul>
      </div>

      <!-- 明细 / 导出 / 元信息（插入编辑器、复制 SQL 收进执行过程那一步的代码块里） -->
      <div class="answer-tools">
        <el-button v-if="hasRows" link size="small" @click="showRows = !showRows">
          {{ showRows ? '收起数据' : `查看数据（${answer.rowCount ?? 0} 行）` }}
        </el-button>
        <el-button v-if="hasRows" link size="small" :icon="Download" @click="exportCsv">
          导出 CSV
        </el-button>
        <span class="meta">{{ answer.rowCount ?? 0 }} 行<template v-if="elapsedText"> · {{ elapsedText }}</template></span>
      </div>

      <div v-if="showRows" class="rows-block">
        <DataTable :answer="answer" :limit="200" />
      </div>

      <!-- 追问 -->
      <div v-if="answer.followups?.length" class="chips">
        <button v-for="(f, i) in answer.followups" :key="i" class="chip" @click="emit('ask', f)">{{ f }}</button>
      </div>

      <!-- 用量放最底部：这是「账单」信息，不该跟操作按钮抢同一行 -->
      <div v-if="usageText" class="ask-foot">{{ usageText }}</div>
    </template>
  </div>
</template>

<script setup>
/**
 * 问数回答卡片（执行过程 / 图表 / 数据概况 / 结论 / 洞察 / 明细 / 追问 / 用量）。
 *
 * <p>为什么单独拆一个组件：同一个回答要能出现在两种宿主里 —— AI 助手的对话流（主入口）
 * 和独立问数工作区。渲染只有这一份实现，宿主只管「问」和「放哪儿」。
 *
 * <p>图表类型可**本地切换**：后端只给推荐类型与字段映射，切换时用同一份结果集重画，
 * 不再问模型（省钱且即时）。
 */
import { computed, onBeforeUnmount, reactive, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import {
  Download, Switch, CaretRight, CircleCheck, CircleCheckFilled, CopyDocument, DataAnalysis, EditPen,
  InfoFilled, Loading, MagicStick, QuestionFilled,
} from '@element-plus/icons-vue'
import ChartCanvas from './ChartCanvas.vue'
import DataTable from './DataTable.vue'
import { CHART_TYPES, chartTypeLabel } from './chartSpec'
import { renderMarkdown } from '../../utils/markdown.js'
import { formatSql } from '../../utils/sqlFormat.js'

const props = defineProps({
  /** `/api/ai/ask` 的返回体 */
  answer: { type: Object, default: null },
})
const emit = defineEmits(['ask', 'insert-sql'])

const md = (text) => renderMarkdown(text || '')

/** 步骤里展示用的 SQL 格式化：一整行的 SQL 读不动，展示前先排个版（失败就原样给）。
 * 只影响显示 —— 复制 / 插入编辑器仍用后端返回的原始文本。 */
const fmtSql = (sql) => {
  const text = String(sql || '')
  try {
    const out = formatSql(text, {})
    return out || text
  } catch {
    return text
  }
}
const showRows = ref(false)
const chartTypes = reactive({})
const chartRefs = new Map()

// ---------------------------------------------------------------- 执行过程
// 折叠态与 CodeBuddy 一致：过程跑完就收成一行「执行过程 · N 步 · 12.3 s」，点开才看细节。
// 卡片只在回答落地后挂载，所以初始一律收起；等待中的进度由宿主的 loading 态负责。
const execOpen = ref(false)

// 每一步各自的展开态：默认全收起（不是一下子把四步的细节全铺开），点哪一步开哪一步。
// running 的那一步例外 —— 它正在做，自动展开让人看到实时内容。
const stepOpen = reactive({})
/** 每步「看完自动折叠」的定时器：内容亮出来 1.6s 后收起 */
const foldTimers = new Map()
const stepKeyOf = (step, index) => (step && step.key ? step.key : `step-${index}`)
const isStepOpen = (step, index) => {
  if (step && step.status === 'running') return true
  return !!stepOpen[stepKeyOf(step, index)]
}
const toggleStep = (step, index) => {
  if (step && step.status === 'running') return
  const key = stepKeyOf(step, index)
  // 手动接管后不再被自动折叠打断
  clearTimeout(foldTimers.get(key))
  foldTimers.delete(key)
  stepOpen[key] = !stepOpen[key]
}

// 跑的时候展开、内容出来停一拍再折叠 —— 像深度思考面板：过程看得见，看完自动收起。
// 只在流式过程中生效；最终结果整体替换时步骤都已是 done，不该再全部弹开一次。
watch(
  () => props.answer?.steps || [],
  (steps) => {
    if (!props.answer?.streaming) return
    for (const s of steps) {
      if (!s?.key || s.status !== 'done' || stepOpen[s.key]) continue
      stepOpen[s.key] = true
      clearTimeout(foldTimers.get(s.key))
      foldTimers.set(s.key, setTimeout(() => {
        stepOpen[s.key] = false
        foldTimers.delete(s.key)
      }, 1600))
    }
  },
  { deep: true },
)
onBeforeUnmount(() => {
  for (const t of foldTimers.values()) clearTimeout(t)
  foldTimers.clear()
})

const execBrief = computed(() => {
  const count = props.answer?.steps?.length || 0
  return count ? `共 ${count} 步` : ''
})

/** 流式时的进度：跑完了几步（running 的那一步不算） */
const doneCount = computed(
  () => (props.answer?.steps || []).filter((s) => s && s.status !== 'running').length,
)

const elapsedText = computed(() => {
  const ms = Number(props.answer?.elapsedMs)
  if (!Number.isFinite(ms) || ms <= 0) return ''
  if (ms < 1000) return `${ms} ms`
  const seconds = ms / 1000
  if (seconds < 60) return `${seconds.toFixed(1)} s`
  const min = Math.floor(seconds / 60)
  return `${min} 分 ${Math.round(seconds - min * 60)} 秒`
})

/** 有没有真实数据行：空结果不画图、不列洞察、不给空概况 */
const hasRows = computed(() => {
  const rows = props.answer?.rows
  return Array.isArray(rows) && rows.length > 0
})

/** 实际要渲染的图表：空结果时一张都不给 */
const visibleCharts = computed(() => (hasRows.value ? props.answer?.charts || [] : []))

/** 底部用量：只显示上游真的回了的字段，不估算 */
const usageText = computed(() => {
  const usage = props.answer?.usage
  if (!usage) return ''
  const total = Number(usage.totalTokens || 0)
  if (!total) return ''
  const parts = [`${total.toLocaleString('zh-CN')} tokens`]
  const prompt = Number(usage.promptTokens || 0)
  const completion = Number(usage.completionTokens || 0)
  if (prompt) parts.push(`输入 ${prompt.toLocaleString('zh-CN')}`)
  if (completion) parts.push(`输出 ${completion.toLocaleString('zh-CN')}`)
  return parts.join(' · ')
})

/** 每一步的细节：后端给了 detail 就用（候选表 / 结果规模 / 结论来源），没给就按 key 兜底 */
const stepDetail = (step) => {
  if (!step) return ''
  if (step.detail) return step.detail
  const ans = props.answer || {}
  if (step.key === 'exec') {
    const cols = (ans.columns || []).filter((c) => c && c !== '__row').length
    return `返回 ${ans.rowCount ?? 0} 行${cols ? ` × ${cols} 列` : ''}`
  }
  if (step.key === 'schema' && ans.candidateTables?.length) {
    return `候选表：${ans.candidateTables.join('、')}`
  }
  return ''
}

// ---------------------------------------------------------------- 数据概况
// **只在本地算**：全部数字直接来自返回的 rows / rowCount，与明细表逐格对得上，
// 不经过模型 —— 概况一旦由模型"总结"，就会冒出结果里没有的数（这是问数最致命的错）。
const NUMERIC_TOKENS = ['int', 'dec', 'num', 'float', 'double', 'real', 'money', 'bigint', 'serial']
/** 标识类数值列（id / code / no）：算合计没意义，只看取值范围 */
const ID_LIKE = /(^|_)id$|^id$|uuid|(^|_)code$|(^|_)no$/i
const DATE_HEAD = /^\d{4}-\d{2}(-\d{2})?/
const NUMERIC_KIND = '数值'
const TIME_KIND = '时间'
const DIM_KIND = '维度'

const isBlank = (value) =>
  value === null || value === undefined || (typeof value === 'string' && value.trim() === '')

const toNumber = (value) => {
  if (isBlank(value) || typeof value === 'boolean') return null
  const num = typeof value === 'number' ? value : Number(String(value).replace(/,/g, ''))
  return Number.isFinite(num) ? num : null
}

/** 数字照原样给（千分位 + 最多两位小数）：概况里出现的每个数都要能被明细核对 */
const fmtNum = (value) => {
  if (!Number.isFinite(value)) return '—'
  if (Number.isInteger(value) && Math.abs(value) < 1e15) return value.toLocaleString('zh-CN')
  return value.toLocaleString('zh-CN', { maximumFractionDigits: 2 })
}

const shortText = (text, limit = 16) => {
  const value = String(text)
  return value.length > limit ? `${value.slice(0, limit)}…` : value
}

/** 时间列的起止：只在「类型像时间」或列名像时间时尝试，格式不统一就放弃（宁可不写，也不写错） */
const dateRange = (values, column, type) => {
  const named = /date|time|(^|_)(day|month|year)$/i.test(column)
  const typed = type.includes('date') || type.includes('time')
  if (!named && !typed) return null
  const texts = values.filter((v) => !isBlank(v)).map((v) => String(v).trim())
  const hits = texts.filter((t) => DATE_HEAD.test(t))
  if (!hits.length || hits.length < texts.length * 0.6) return null
  const sorted = [...hits].sort()
  return { min: sorted[0], max: sorted[sorted.length - 1] }
}

const profile = computed(() => {
  const ans = props.answer || {}
  const columns = (ans.columns || []).filter((c) => c && c !== '__row')
  const rows = Array.isArray(ans.rows) ? ans.rows : []
  const total = Number(ans.rowCount ?? rows.length) || rows.length
  const types = ans.columnTypes || []
  const items = []
  // 空结果集：概况没有内容可讲（每列都会算出"全部为空"），交给结论那句"没有符合条件的数据"
  if (!rows.length) {
    return { items, cols: columns.length, rowsText: `${total} 行`, shown: 0, partial: false, partialHint: '' }
  }

  for (const name of columns) {
    const index = (ans.columns || []).indexOf(name)
    const type = String(types[index] || '').toLowerCase()
    const values = rows.map((row) => (row ? row[name] : null))
    const filled = values.filter((value) => !isBlank(value)).length
    const blanks = values.length - filled
    const nums = values.map(toNumber).filter((value) => value !== null)
    const numeric = NUMERIC_TOKENS.some((token) => type.includes(token))
      || (filled >= 2 && nums.length >= filled * 0.9)

    let kindLabel = DIM_KIND
    let text = ''

    if (numeric && nums.length) {
      kindLabel = NUMERIC_KIND
      let sum = 0
      let min = nums[0]
      let max = nums[0]
      for (const value of nums) {
        sum += value
        if (value < min) min = value
        if (value > max) max = value
      }
      if (ID_LIKE.test(name)) {
        const distinct = new Set(nums.map((v) => String(v))).size
        text = `${distinct} 个不同值 · 范围 ${fmtNum(min)} ~ ${fmtNum(max)}`
      } else {
        text = `合计 ${fmtNum(sum)} · 均值 ${fmtNum(sum / nums.length)} · 最大 ${fmtNum(max)} · 最小 ${fmtNum(min)}`
      }
    } else {
      const counts = new Map()
      for (const value of values) {
        if (isBlank(value)) continue
        const key = String(value)
        counts.set(key, (counts.get(key) || 0) + 1)
      }
      const range = dateRange(values, name, type)
      if (range) {
        kindLabel = TIME_KIND
        text = `${shortText(range.min, 19)} ~ ${shortText(range.max, 19)} · ${counts.size} 个不同值`
      } else if (counts.size) {
        let topKey = ''
        let topCount = 0
        for (const [key, count] of counts) {
          if (count > topCount) {
            topKey = key
            topCount = count
          }
        }
        const pct = filled ? Math.round((topCount / filled) * 1000) / 10 : 0
        if (counts.size === 1) {
          text = `全部为「${shortText(topKey)}」（${topCount} 行）`
        } else if (counts.size === filled) {
          // 值全不重复（典型是明细结果里的名字/单号）：说「最多某某占 10%」只会误导
          text = `${counts.size} 个不同值（无重复）`
        } else {
          text = `${counts.size} 个不同值 · 最多「${shortText(topKey)}」${topCount} 行（${pct}%）`
        }
      } else {
        text = '全部为空'
      }
    }

    if (blanks && filled) text += ` · 空值 ${blanks}`
    if (text) items.push({ name, kindLabel, text })
  }

  const shown = rows.length
  // rowCount 是「返回了几行」；hasMore / totalCount=-1 说明服务端还截断了更多行
  const more = ans.hasMore === true || ans.totalCount === -1
  const partial = shown < total || more
  return {
    items,
    cols: columns.length,
    rowsText: `${total.toLocaleString('zh-CN')} 行`,
    shown,
    partial,
    partialHint: more
      ? `结果已被行数上限截断，概况只覆盖返回的 ${shown} 行`
      : `结果共 ${total} 行，概况基于已返回的 ${shown} 行计算`,
  }
})

/** 概况说成一句人话：「日期（时间）：…；销售额（数值）：…」—— 比一堆小卡更轻 */
const profileText = computed(() =>
  (profile.value.items || [])
    .map((it) => `${it.name}${it.kindLabel ? `（${it.kindLabel}）` : ''}：${it.text}`)
    .join('；'),
)

const typeOf = (index) => chartTypes[index] || props.answer?.charts?.[index]?.type || 'bar'
const typeLabel = (index) => chartTypeLabel(typeOf(index))

const setChartRef = (index, el) => {
  if (el) chartRefs.set(index, el)
  else chartRefs.delete(index)
}

const setChartType = (index, type) => {
  chartTypes[index] = type
  // 切到表格时 ChartCanvas 被卸载，切回来要重新挂载并重画
  requestAnimationFrame(() => {
    const inst = chartRefs.get(index)
    if (inst && inst.resize) inst.resize()
  })
}

const downloadChart = (index) => {
  const inst = chartRefs.get(index)
  const url = inst && inst.toDataURL ? inst.toDataURL() : ''
  if (!url) {
    ElMessage.warning('当前图表还不能导出')
    return
  }
  const a = document.createElement('a')
  a.href = url
  a.download = `${props.answer?.charts?.[index]?.title || 'chart'}.png`
  a.click()
}

const copy = async (text) => {
  try {
    await navigator.clipboard.writeText(text || '')
    ElMessage.success('已复制')
  } catch {
    ElMessage.warning('复制失败，请手动选择')
  }
}

/** 把结果集导出成 CSV（带 BOM，Excel 直接打开中文不乱码）。空结果不给按钮，这里再兜底一次。 */
const exportCsv = () => {
  const ans = props.answer || {}
  const cols = (ans.columns || []).filter((c) => c && c !== '__row')
  const rows = Array.isArray(ans.rows) ? ans.rows : []
  if (!cols.length || !rows.length) {
    ElMessage.warning('没有可导出的数据')
    return
  }
  const escape = (value) => {
    if (value === null || value === undefined) return ''
    const text = String(value)
    return /[",\n\r]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text
  }
  const lines = [cols.join(',')]
  for (const row of rows) {
    lines.push(cols.map((c) => escape(row ? row[c] : '')).join(','))
  }
  const blob = new Blob(['\uFEFF' + lines.join('\r\n')], { type: 'text/csv;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `${(ans.question || 'ask').replace(/[\\/:*?"<>|]/g, '_')}.csv`
  document.body.appendChild(a)
  a.click()
  document.body.removeChild(a)
  URL.revokeObjectURL(url)
}
</script>

<style scoped>
.ask-answer { font-size: 13px; line-height: 1.7; color: var(--dc-text); }
/* 流式作答：只留执行过程（此时结论/图表/明细都还没算出来，渲染空壳只会占位置） */
.ask-answer.is-live > :not(.exec):not(.ask-wait) { display: none; }
.ask-wait { display: flex; align-items: center; gap: 6px; margin-top: 8px; font-size: 12px; color: var(--dc-text-2); }

.summary { font-size: 14px; line-height: 1.7; font-weight: 500; }
/* 结论区：带「结论」标题的小面板，和「数据概况」同一套头行语言 */
.answer-conclusion {
  margin-top: 12px; padding: 10px 12px 11px; border-radius: 10px;
  background: var(--dc-bg-card); border: 1px solid var(--dc-border-soft);
}
.concl-head { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--dc-text); }
.concl-icon { font-size: 14px; color: var(--dc-primary); }
.concl-title { font-weight: 500; }
.answer-conclusion .summary { margin-top: 6px; }
/* 空结果：一句解释，不摆空图表 */
.empty-hint { display: flex; align-items: flex-start; gap: 6px; margin-top: 10px; font-size: 12px; color: var(--dc-text-2); }
.empty-hint .el-icon { margin-top: 3px; flex-shrink: 0; }
/* 底部用量：右对齐小字，弱到不影响读结论 */
.ask-foot {
  display: flex; justify-content: flex-end; margin-top: 10px;
  font-size: 11px; color: var(--dc-text-2); opacity: .75; font-variant-numeric: tabular-nums;
}
.clarify-title { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--dc-warning, #f5a623); }
.clarify-text { font-size: 13px; line-height: 1.7; margin: 8px 0; }

.chart-card { margin-top: 12px; border: 1px solid var(--dc-border); border-radius: 8px; padding: 8px 10px 4px; background: var(--dc-bg-card); }
.chart-head { display: flex; align-items: center; gap: 8px; }
.chart-title { font-size: 12px; color: var(--dc-text-2); flex: 1; }
.tag-fallback {
  margin-left: 6px; font-size: 10px; padding: 1px 6px; border-radius: 8px;
  background: var(--dc-bg-raised); color: var(--dc-text-2);
}
.chart-ops { display: flex; align-items: center; gap: 4px; }
.chart-table { max-height: 320px; overflow: auto; }

.insights { margin: 10px 0 0; padding-left: 18px; font-size: 12px; line-height: 1.8; color: var(--dc-text-2); }
.insights li::marker { color: var(--dc-primary); }

.answer-tools { display: flex; align-items: center; gap: 6px; margin-top: 12px; flex-wrap: wrap; }
/* 四个操作统一成小药丸按钮：有边有底、带文字，比纯图标 link 明显得多 */
.answer-tools .el-button {
  height: 24px; padding: 0 9px; border-radius: 6px;
  border: 1px solid var(--dc-border); background: var(--dc-bg-raised);
  color: var(--dc-text-2); font-size: 11px;
}
.answer-tools .el-button + .el-button { margin-left: 0; }
.answer-tools .el-button:hover {
  color: var(--dc-primary); border-color: var(--dc-primary); background: var(--dc-primary-wash);
}
.meta { margin-left: auto; font-size: 11px; color: var(--dc-text-2); }
.rot { transform: rotate(90deg); }

.rows-block {
  margin-top: 8px; border: 1px solid var(--dc-border); border-radius: 8px;
  overflow: auto; max-height: 320px;
}

/* 执行过程：收起时只有一行，展开后是一条竖向时间线 */
.exec {
  margin-top: 10px; border: 1px solid var(--dc-border); border-radius: 8px;
  background: var(--dc-bg-card); overflow: hidden;
}
.exec-head {
  display: flex; align-items: center; gap: 6px; width: 100%; padding: 7px 10px;
  border: 0; background: transparent; cursor: pointer; text-align: left;
  font-family: inherit; font-size: 12px; color: var(--dc-text-2);
}
.exec-head:hover { background: var(--dc-bg-raised); }
/* 进行中不让人折叠：这时候「展开」是唯一正确状态，给个可点的头只会让人以为能收起 */
.exec-head:disabled { cursor: default; }
.exec-head:disabled:hover { background: transparent; }
.exec-caret { font-size: 12px; transition: transform .15s ease; }
.exec-spin { font-size: 12px; color: var(--dc-primary); }
.exec-ok { font-size: 13px; color: var(--dc-success, #38c9a8); }
.exec-name { color: var(--dc-text); font-weight: 500; }
.exec-brief::before { content: '·'; margin: 0 5px 0 6px; opacity: .55; }
/* 进行中：标题用主色，和"已完成"的灰白标题区分开 */
.exec.live .exec-name { color: var(--dc-primary); }
.exec-cost { margin-left: auto; font-size: 11px; color: var(--dc-text-2); font-variant-numeric: tabular-nums; }

.exec-list {
  margin: 0; padding: 4px 12px 8px; list-style: none;
  border-top: 1px solid var(--dc-border);
}
.exec-item { position: relative; padding: 4px 0; }
.exec-item::before {
  content: ''; position: absolute; left: 6px; top: 24px; bottom: -2px;
  width: 1px; background: var(--dc-primary); opacity: .22;
}
.exec-item:last-child::before { display: none; }
/* 每一步一个开关：整行可点，右侧小箭头指示展开态 */
.exec-step-head {
  display: flex; align-items: center; gap: 8px; width: 100%;
  padding: 3px 0; border: 0; border-radius: 6px;
  background: transparent; cursor: pointer; text-align: left;
  font-family: inherit; color: inherit;
}
.exec-step-head:hover { background: var(--dc-bg-raised); }
.exec-step-head:hover .exec-label { color: var(--dc-primary); }
.exec-step-head:disabled { cursor: default; }
.exec-step-caret { font-size: 11px; color: var(--dc-text-2); transition: transform .15s ease; flex-shrink: 0; }
.exec-step-body { margin: 4px 0 8px 22px; }
.exec-dot {
  position: relative; z-index: 1; display: inline-flex; align-items: center; justify-content: center;
  width: 13px; height: 13px; font-size: 12px; border-radius: 50%;
  color: var(--dc-success, #38c9a8); background: var(--dc-bg-card); flex-shrink: 0;
}
/* 每一步的「分析过程 / SQL / 分析结果」：只给内容，不加多余装饰 */
.exec-block { margin-top: 9px; }
.blk-label {
  display: flex; align-items: center; gap: 5px;
  font-size: 11px; font-weight: 500; color: var(--dc-text-2); margin-bottom: 4px;
}
.blk-label .el-icon { font-size: 12px; }
/* SQL 代码块：与 AI 对话里的 markdown 代码块同一套外观（底色 / 边框 / 字号），
   操作按钮悬浮在块内右上角，平时半透明、悬浮整块时点亮 */
.blk-code-wrap { position: relative; margin-top: 6px; }
.blk-code {
  margin: 0; padding: 11px 14px; border-radius: 8px; overflow: auto; max-height: 280px;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border-soft);
  font-family: "SF Mono", "JetBrains Mono", Consolas, monospace;
  font-size: 13px; line-height: 1.65; color: var(--dc-code-text, var(--dc-text));
  white-space: pre; word-break: normal;
}
.blk-acts {
  position: absolute; top: 6px; right: 6px; display: flex; gap: 4px;
  opacity: .5; transition: opacity .15s ease;
}
.blk-code-wrap:hover .blk-acts { opacity: 1; }
.blk-acts .el-button {
  width: 22px; height: 22px; padding: 0; border-radius: 5px;
  border: 1px solid var(--dc-border); background: var(--dc-bg-card);
  color: var(--dc-text-dim);
}
.blk-acts .el-button:hover {
  color: var(--dc-primary); border-color: var(--dc-primary); background: var(--dc-primary-wash);
}
.blk-text { font-size: 12px; line-height: 1.8; color: var(--dc-text); white-space: pre-wrap; word-break: break-word; }
/* 分析过程用略淡的字色：一眼区分"思考"和"结论" */
.blk-think { color: var(--dc-text-2); }
/* 正在跑的那一步：主色 + 转圈，和已完成的勾一眼能分开 */
.exec-item.running .exec-dot { color: var(--dc-primary); }
.exec-item.running .exec-label { color: var(--dc-text-2); }
.exec-item-body { flex: 1; min-width: 0; }
.exec-row { display: flex; align-items: baseline; gap: 8px; }
.exec-label {
  flex: 1; min-width: 0; font-size: 12px; color: var(--dc-text);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.exec-ms { margin-left: auto; font-size: 11px; color: var(--dc-text-2); font-variant-numeric: tabular-nums; }
.exec-detail { margin-top: 1px; font-size: 11px; line-height: 1.6; color: var(--dc-text-2); word-break: break-word; }
/* 进行中的那一步自动展开：此刻已确定的内容（如 SQL、在读哪些表）直接可见，
   跑完后由 watcher 停一拍再折叠 —— 不再额外隐藏骨架 */

/* 数据概况：按列铺小卡，列名与类型一行、统计值一行，列多时滚动 */
.profile {
  margin-top: 12px; padding: 10px 12px 12px; border: 1px solid var(--dc-border);
  border-radius: 10px; background: var(--dc-bg-card);
}
.profile-head { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--dc-text); flex-wrap: wrap; }
.profile-icon { font-size: 14px; color: var(--dc-primary); }
.profile-title { font-weight: 500; }
.pf-chip { padding: 1px 7px; border-radius: 9px; font-size: 10px; background: var(--dc-bg-raised); color: var(--dc-text-2); }
.pf-chip.warn { border: 1px solid currentColor; background: transparent; color: var(--dc-warning, #f5a623); }
/* 概况说成一段文字：不再铺小卡，轻得多 */
.profile-text { margin: 8px 0 0; font-size: 12.5px; line-height: 1.9; color: var(--dc-text-2); word-break: break-word; }

.chips { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 10px; }
.chip {
  border: 1px solid var(--dc-border); background: var(--dc-bg-raised); color: var(--dc-text-2);
  border-radius: 12px; padding: 4px 10px; font-size: 11px; cursor: pointer;
}
.chip:hover { border-color: var(--dc-primary); color: var(--dc-primary); }
</style>
