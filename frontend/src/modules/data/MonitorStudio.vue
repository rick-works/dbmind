<script setup>
/**
 * 数据库实时监控工作台（完全由方言 monitorSpec() 驱动渲染）。
 * 每个数据库类型通过 monitorSpec() 声明自己的 KPI 指标卡与区块面板（标题/图标/列/可终止/可复制/阈值），
 * 本组件只负责通用渲染——新增数据库类型零改前端，且每种库只展示自己的指标，不会互相污染。
 * 连接与数据库在 tab 打开时固化（tab.connId / tab.database），切到别的连接不影响本页数据源。
 */
import { ref, computed, onBeforeUnmount, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Refresh, Warning, User, Timer, Coin, Setting, Connection, Cpu, DataLine, Lock, Clock, TrendCharts, Loading, CircleCloseFilled, InfoFilled, CircleCheck, Odometer, ArrowDown, Histogram, Document, Memo, Monitor, Grid, Files, Operation, Share, Postcard, Switch } from '@element-plus/icons-vue'
import { monitorOverview, monitorKill, listConnections } from '../../api'
import { t } from '../../utils/i18n'

const props = defineProps({
  connId: { type: [String, Number], default: '' },
  database: { type: String, default: '' }
})

const loading = ref(false)
const error = ref('')
const data = ref(null)
const autoRefresh = ref(false)
const intervalSec = ref(5)
let timer = null
/** 首次进入时补一次采样用（算实时 QPS），与自动刷新互不干扰 */
let catchupTimer = null

/** 实时 QPS：两次刷新之间 Questions 累计值的增量 ÷ 间隔秒数（仅 MySQL 系使用） */
let prevQuestions = null
let prevAt = 0
const liveQps = ref(null)

/** 图标名 → Element Plus 图标组件（spec 中以字符串传递） */
const ICONS = {
  connection: Connection, cpu: Cpu, dataline: DataLine, timer: Timer, lock: Lock, coin: Coin,
  trendCharts: TrendCharts, warning: Warning, clock: Clock, user: User, setting: Setting,
  odometer: Odometer, histogram: Histogram, document: Document, memo: Memo, monitor: Monitor,
  grid: Grid, files: Files, operation: Operation, share: Share, postcard: Postcard, switch: Switch, info: InfoFilled
}
/** 配色名 → 渐变（spec.color 中以字符串传递；KPI 在 level=none 时取此色，面板芯片始终取此色） */
const PC_COLORS = {
  rose: 'linear-gradient(135deg,#f43f5e,#fb7185)', indigo: 'linear-gradient(135deg,#6366f1,#818cf8)',
  amber: 'linear-gradient(135deg,#f59e0b,#fbbf24)', emerald: 'linear-gradient(135deg,#10b981,#34d399)',
  violet: 'linear-gradient(135deg,#8b5cf6,#a78bfa)', sky: 'linear-gradient(135deg,#0ea5e9,#38bdf8)',
  slate: 'linear-gradient(135deg,#64748b,#94a3b8)', teal: 'linear-gradient(135deg,#14b8a6,#2dd4bf)',
  cyan: 'linear-gradient(135deg,#06b6d4,#22d3ee)', orange: 'linear-gradient(135deg,#fb923c,#fdba74)',
  pink: 'linear-gradient(135deg,#ec4899,#f472b6)'
}

const metricMap = (rows) => {
  const m = {}
  // 值列约定为 value；兼容个别方言返回的 count 别名，避免 KPI 显示 NaN
  // NULL/空值直接跳过（该指标当前库未提供，KPI 卡不渲染），不能 Number(undefined) 成 NaN
  for (const r of rows || []) {
    if (!r || r.metric == null) continue
    const v = r.value ?? r.count
    if (v == null || v === '') continue
    m[r.metric] = Number(v)
  }
  return m
}

const fetch = async () => {
  if (!props.connId) { error.value = t('mon.noConnId'); return }
  loading.value = true
  try {
    const d = await monitorOverview(props.connId, props.database || undefined)
    const m = metricMap(d?.sections?.instance)
    const now = Date.now()
    if (m.questions != null && prevQuestions != null && now > prevAt) {
      const dt = (now - prevAt) / 1000
      if (dt >= 1 && m.questions >= prevQuestions) liveQps.value = Math.round((m.questions - prevQuestions) / dt * 10) / 10
    }
    if (m.questions != null) { prevQuestions = m.questions; prevAt = now }
    data.value = d
    error.value = d?.error || ''
    // 「实时 QPS」要靠两次采样算增量，刷新前它只能是「—」（看着像坏了）。
    // 第一次拿到数据后再补一次采样，1.5 秒后这张卡就有真数了。
    if (liveQps.value == null && m.questions != null && !catchupTimer) {
      catchupTimer = setTimeout(() => { catchupTimer = null; fetch() }, 1500)
    }
  } catch (e) {
    error.value = e?.message || t('mon.fetchFailed')
    data.value = null
  } finally {
    loading.value = false
  }
}

watch(() => props.connId, () => { error.value = ''; data.value = null; prevQuestions = null; prevAt = 0; fetch() }, { immediate: true })

/** 连接名：按 connId 从连接列表解析，用于头部展示 */
const connName = ref('')
watch(() => props.connId, async () => {
  connName.value = ''
  if (!props.connId) return
  try {
    const list = await listConnections()
    const arr = Array.isArray(list) ? list : (list?.data || [])
    connName.value = arr.find(c => String(c.id) === String(props.connId))?.name || ''
  } catch { /* 拿不到连接名不影响监控功能 */ }
}, { immediate: true })

const fetchedAtText = computed(() => {
  if (!data.value?.fetchedAt) return ''
  const d = new Date(data.value.fetchedAt); const p = (n) => String(n).padStart(2, '0')
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
})

const fmtUptime = (sec) => {
  if (sec == null || isNaN(sec)) return '—'
  const d = Math.floor(sec / 86400), h = Math.floor(sec % 86400 / 3600), mi = Math.floor(sec % 3600 / 60)
  return d > 0 ? t('mon.durDayHour', { d, h }) : h > 0 ? t('mon.durHourMin', { h, m: mi }) : t('mon.durMin', { m: mi })
}
/** 字节友好展示（1024 进制，自动 B/KB/MB/GB/TB） */
const fmtBytes = (b) => {
  const n = Number(b)
  if (!isFinite(n) || n <= 0) return '—'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']; let i = 0, v = n
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++ }
  return (i === 0 ? String(v) : (v >= 100 ? v.toFixed(0) : v.toFixed(1))) + ' ' + units[i]
}

const sec = (name) => Array.isArray(data.value?.sections?.[name])
const rows = (name) => data.value?.sections?.[name] || []
/**
 * 面板的列顺序：**以服务端给的 `columns[key]` 为准**。
 *
 * 不能按对象的键推列：行是 JSON 对象，后端 `serde_json` 的 Map 会**按键排序**，
 * 于是列序恒为字母序 —— 「表空间 Top 20」里 `table_name` 就被排到了 `size_mb` 之后，
 * 表名跑到最后一列、中间还空一大片，就是这么来的。
 * 服务端没给（旧后端/空表）时才退回对象键。
 */
const cols = (name) => {
  const ordered = data.value?.columns?.[name]
  if (Array.isArray(ordered) && ordered.length) return ordered
  return rows(name).length ? Object.keys(rows(name)[0]) : []
}

/** ===== KPI 指标卡：完全由 spec.kpis 驱动 ===== */
const instRows = computed(() => data.value?.sections?.instance || [])
const kpiCards = computed(() => {
  const spec = data.value?.spec
  if (!spec?.kpis?.length) return []
  const m = metricMap(instRows.value)
  const cards = []
  for (const k of spec.kpis) {
    let value, unit = k.unit || '', level = 'none'
    if (k.kind === 'liveQps') {
      value = liveQps.value == null ? '—' : liveQps.value; unit = k.unit || t('mon.perSecond'); level = 'none'
    } else {
      const raw = m[k.metric]
      if (raw == null) continue // 该指标当前库未提供，跳过此卡
      if (k.format === 'duration') { value = fmtUptime(raw); unit = '' }
      else if (unit === 'bytes') { value = fmtBytes(raw); unit = '' }
      else value = fmtNum(raw)
      if (k.thresholds?.length) {
        level = 'good'
        for (const t of k.thresholds) if (Number(raw) >= t.value) level = t.level
      }
    }
    let desc = k.desc || ''
    if (desc) desc = desc.replace(/\{(\w+)\}/g, (_, key) => {
      const v = m[key]; if (v == null) return '—'
      return key === k.metric && unit === 'bytes' ? fmtBytes(v) : fmtNum(v)
    })
    cards.push({
      key: k.metric, label: k.label, value, unit, level,
      icon: ICONS[k.icon] || Coin,
      icoStyle: level === 'none' ? { '--ico': PC_COLORS[k.color] || PC_COLORS.indigo } : null,
      desc
    })
  }
  return cards
})

/** ===== 区块面板：完全由 spec.panels 驱动 ===== */
const panels = computed(() => {
  const spec = data.value?.spec
  if (!spec?.panels?.length) return []
  return spec.panels
    .map(p => {
      const rs = rows(p.key) || []
      const cs = cols(p.key) || []
      const sparse = p.mode !== 'kv' && cs.length > 0 && cs.length <= 6 && !cs.some(c => isQueryCol(c))
      // 少列表格的列宽在这里一次算好（见 colPlan），模板直接取用
      const widths = sparse ? colPlan(cs, rs) : null
      return { ...p, rows: rs, cols: cs, sparse, widths, error: data.value?.sectionErrors?.[p.key] || '' }
    })
    .filter(p => sec(p.key) || p.error)
})

/** 面板折叠：有数据默认展开、无数据默认折叠，点击头部切换 */
const openMap = ref({})
const togglePanel = (key) => { openMap.value[key] = !(openMap.value[key] ?? false) }
const bodyVisible = (p) => openMap.value[p.key] ?? p.rows.length > 0

/**
 * 少列表格的列宽：**数字列固定宽度，文本列按「该列最长内容」加权分掉剩余宽度**。
 *
 * 这版是第三稿，前两稿都不好看，记下来免得再走回去：
 *  · 首稿按「首列 34%、其余均分」铺满 —— 6 列表格变成「名字挤在最右边、中间一大片空白」；
 *  · 次稿把「**第一个**文本列」设为 width:100% —— 对 MySQL 的「表空间」（首列是 table_name）还行，
 *    但 ClickHouse 的「表空间」首列是 `db`（取值只有 system / ods），于是整条宽度被它独吞，
 *    真正该宽的 `table_name` 反而只剩一点点 —— 就是那张被吐槽"丑"的截图。
 * 现在按内容加权：谁的内容长谁宽。**短文本列（≤12 字符）直接给固定宽度**贴合内容，
 * 长文本列再按权重分摊剩余，这样 db / engine / command 这类短值列不会被撑开。
 */
const COL_NUM_W = '110px'      // 与 .fixed-q 的数字列保持一致
const COL_TIGHT_CHARS = 12     // ≤ 这么长算「短文本列」：贴合内容，不参与剩余宽度分摊
const COL_TIGHT_CHAR_W = 9     // 每个字符约 9px（13px 字号下的经验值，含内边距余量）
const COL_TIGHT_MIN_W = 70     // 短文本列的宽度下限
const COL_TEXT_MIN_W = 6       // 权重下限（表头本身也有长度，避免列太窄）
const COL_TEXT_MAX_W = 60      // 权重上限：个别超长值不该吃掉整张表

/** 该列的内容宽度权重 = 表头与所有取值里最长的那个（夹在 [6, 60] 之间） */
const colWeight = (col, rows, headLen) => {
  let max = headLen
  for (const r of rows || []) {
    const v = r?.[col]
    if (v == null || v === '') continue
    const n = fmtCell(v).length
    if (n > max) max = n
  }
  return Math.min(Math.max(max, COL_TEXT_MIN_W), COL_TEXT_MAX_W)
}

/** 一次算好整张表的列宽计划（模板按列取用，避免每个单元格重算） */
const colPlan = (cols, rows) => {
  const plan = cols.map(c => (isNumCol(c) ? { width: COL_NUM_W } : null))
  const longs = []
  cols.forEach((c, i) => {
    if (isNumCol(c)) return
    const w = colWeight(c, rows, String(headLabel(c) || '').length)
    if (w <= COL_TIGHT_CHARS) plan[i] = { width: Math.max(COL_TIGHT_MIN_W, w * COL_TIGHT_CHAR_W) + 'px' }
    else longs.push({ i, w })
  })
  const sum = longs.reduce((a, b) => a + b.w, 0) || 1
  longs.forEach(x => { plan[x.i] = { width: (x.w / sum * 100).toFixed(2) + '%' } })
  return plan
}

/** 列名像 SQL 文本的（query / DIGEST_TEXT 等）按代码样式截断展示 */
const isQueryCol = (col) => /^(query|waiting_query|blocking_query|digest_text|info|sql_text|command)$/i.test(col)
/** 行内 SQL 列取值：优先用 spec.copyCols，否则自动探测 query 类列 */
const rowQueryCol = (cols, row, copyCols) => {
  if (copyCols && copyCols.length) for (const c of copyCols) if (row[c]) return c
  return cols.find(c => isQueryCol(c) && row[c])
}
const rowHasQuery = (cols, row, copyCols) => !!rowQueryCol(cols, row, copyCols)

/**
 * 数字列右对齐：*_mb / *_sec / *_pct / *_count / rows / time 等。
 * `_rows` / `_bytes` / `_parts` 也要算进来 —— `table_rows` 是行数、`size_mb` 旁边
 * 那一串「110000」左对齐看着不像数字（表空间的 table_rows 实测）。
 */
const NUM_RE = /(_mb|_gb|_kb|_sec|_pct|_count|_rows|_bytes|_parts|_ms|^rows$|^time$|^errors$|^value$|^calls$|^size$|^used$|^total$|^max$|^hits$|^misses$)/i
const isNumCol = (col) => NUM_RE.test(col)

/** 会话/线程 ID 列识别（运维「终止」按钮用）：优先 spec.idCols，否则通用探测 */
const KILL_COLS = ['id', 'pid', 'spid', 'session_id', 'waiting_id', 'blocking_id', 'blocking_thread', 'blocking_pid', 'thread_id', 'trx_id', 'conn_id', 'Id', 'PID', 'SPID', 'sess_id', 'processlist_id']
const killIdOf = (row, idCols) => {
  const list = (idCols && idCols.length) ? idCols : KILL_COLS
  for (const k of list) if (row[k] != null && String(row[k]).match(/^\d+$/)) return String(row[k])
  return ''
}

const kill = async (row, sessionId) => {
  const q = row.waiting_query || row.blocking_query || row.query || row.info || row.sql_text || ''
  const tip = q ? `\n\n${String(q).slice(0, 120)}` : ''
  try {
    await ElMessageBox.confirm(t('mon.killConfirmBody', { id: sessionId, tip }), t('mon.killTitle'), { type: 'warning', confirmButtonText: t('mon.kill'), cancelButtonText: t('common.cancel') })
  } catch { return }
  const r = await monitorKill(props.connId, props.database || undefined, sessionId)
  if (r?.success) { ElMessage.success(r.message || t('mon.killed')); fetch() }
  else ElMessage.error(r?.message || t('mon.killFailed'))
}

const copyText = async (text) => {
  try { await navigator.clipboard.writeText(String(text)); ElMessage.success(t('sqlq.copied')) }
  catch { ElMessage.error(t('sqlq.copyFailed')) }
}

/** 行锁累计耗时（秒）：按天/小时友好展示 */
const fmtLockTime = (sec) => {
  const n = Number(sec)
  if (!isFinite(n)) return '—'
  if (n >= 86400) return t('mon.durDayHour', { d: Math.floor(n / 86400), h: Math.floor(n % 86400 / 3600) })
  if (n >= 3600) return t('mon.durHourMin', { h: Math.floor(n / 3600), m: Math.floor(n % 3600 / 60) })
  if (n >= 60) return t('mon.durMinSec', { m: Math.floor(n / 60), s: Math.round(n % 60) })
  return t('mon.durSec', { n })
}

const fmtCell = (v) => (v == null || v === '' ? '—' : String(v))
/** 大数字千分位（纯数字才转，小数保留） */
const fmtNum = (v) => {
  if (v == null || v === '') return '—'
  const n = Number(v)
  return isNaN(n) ? String(v) : n.toLocaleString('en-US')
}
/** 列名单位后缀 → 数值后的小字单位（_mb→MB、_secs→s、_ms→ms、_pct→%、_gb→GB） */
const unitOf = (col) => {
  if (/^time$/i.test(col)) return 's'
  const m = String(col).match(/_(mb|gb|ms|secs?|pct)$/i)
  if (!m) return ''
  const u = m[1].toUpperCase()
  if (u === 'SEC' || u === 'SECS') return 's'
  return u === 'PCT' ? '%' : u
}
/** 表头显示：单位已跟在数据后，列名去掉单位后缀（size_mb → size、wait_secs → wait） */
const headLabel = (col) => String(col).replace(/_(mb|gb|ms|secs?|pct)$/i, '')

/** 键值网格（关键参数 / 配置类面板）：取首列为键、次列为值，按键名智能补单位 */
const kvKey = (row) => { const ks = Object.keys(row || {}); return fmtCell(row[ks[0]]) }
const kvParts = (row) => {
  const ks = Object.keys(row || {})
  const v = row[ks[1]]
  if (v == null || v === '' || isNaN(Number(v))) return { main: fmtCell(v), unit: '' }
  // 变量名是键的「值」（如 query_timeout），比列名（name / Variable_name）更有语义，两者都参与匹配
  const k = String(row[ks[0]] ?? '') + ' ' + String(ks[0] || '')
  const n = Number(v)
  if (/(_size|_packet|_bytes|_memory|_mem|_capacity)$/i.test(k)) {
    const a = Math.abs(n)
    if (a >= 1073741824) return { main: (n / 1073741824).toFixed(2), unit: 'GB' }
    if (a >= 1048576) return { main: (n / 1048576).toFixed(1), unit: 'MB' }
    if (a >= 1024) return { main: String(Math.round(n / 1024)), unit: 'KB' }
    return { main: fmtNum(n), unit: 'B' }
  }
  if (/timeout|_time$|_secs?$|_delay$|_lifetime$|_keep_alive$|_keepalive$/i.test(k)) return { main: String(+n), unit: 's' }
  if (/(_pct|_percent)$/i.test(k)) return { main: fmtNum(n), unit: '%' }
  if (/(_count|_shards|_nodes|_num$|_queries|_connections|_txns?$|_tasks)$/i.test(k)) return { main: fmtNum(n), unit: t('mon.unitCount') }
  return { main: fmtNum(n), unit: '' }
}
const kvMain = (row) => kvParts(row).main
const kvUnit = (row) => kvParts(row).unit

const onAutoChange = (val) => { val ? startAuto() : stopAuto() }
const startAuto = () => {
  stopAuto()
  timer = setInterval(() => { fetch() }, (intervalSec.value || 5) * 1000)
}
const stopAuto = () => { if (timer) { clearInterval(timer); timer = null } }
const stopCatchup = () => { if (catchupTimer) { clearTimeout(catchupTimer); catchupTimer = null } }

onBeforeUnmount(() => { stopAuto(); stopCatchup() })
</script>

<template>
  <div class="mon-studio">
    <!-- ===== Hero：深色渐变头部 ===== -->
    <div class="hero">
      <div class="hero-l">
        <div class="hero-title-row">
          <span class="hero-badge"><el-icon><Odometer /></el-icon></span>
          <div>
            <div class="hero-title">{{ $t('mon.title') }}</div>
            <div class="hero-chips">
              <span class="chip" v-if="connName"><el-icon class="chip-ico"><Connection /></el-icon>{{ connName }}</span>
              <span class="chip" v-if="data && data.database"><i class="dot"></i>{{ data.database }}</span>
              <span class="chip" v-if="fetchedAtText">{{ $t('mon.updatedAt', { time: fetchedAtText }) }}</span>
              <span class="chip chip-live" v-if="autoRefresh"><i class="live-dot"></i>{{ $t('mon.autoRefreshing') }}</span>
              <span class="chip chip-err" v-if="error">{{ error }}</span>
            </div>
          </div>
        </div>
      </div>
      <div class="hero-r">
        <label class="auto-wrap">
          <!-- 开/关两色都走主题变量：原来写死 #6366f1/#d3d8e6，那个浅灰在深色主题下很扎眼 -->
          <el-switch v-model="autoRefresh" active-color="var(--dc-primary)" inactive-color="var(--dc-bg-hover)"
                     @change="onAutoChange" />
          <span>{{ $t('mon.autoRefresh') }}</span>
        </label>
        <el-select v-if="autoRefresh" v-model="intervalSec" class="hero-select" :teleported="false" @change="onAutoChange(true)">
          <el-option v-for="n in [3, 5, 10, 30]" :key="n" :label="$t('mon.seconds', { n })" :value="n" />
        </el-select>
        <button class="refresh-btn" :disabled="loading" @click="fetch">
          <el-icon :class="{ 'is-loading': loading }"><Refresh /></el-icon>{{ $t('common.refresh') }}
        </button>
      </div>
    </div>

    <div class="mon-scroll">
      <div v-if="loading && !data" class="state-box"><el-icon class="is-loading big-ico"><Loading /></el-icon><span>{{ $t('mon.loadingMonitor') }}</span></div>
      <div v-else-if="error && !data" class="state-box">
        <span class="state-ico bad"><el-icon><CircleCloseFilled /></el-icon></span><span>{{ error }}</span>
      </div>
      <!-- 该类型还没接入监控：后端会带一句原因回来。必须显式渲染 ——
           否则页面是一片空白（只剩刷新按钮），看着像"点了没反应" -->
      <div v-else-if="data && data.supported === false" class="state-box">
        <span class="state-ico"><el-icon><InfoFilled /></el-icon></span>
        <span>{{ data.message || $t('mon.notSupported') }}</span>
      </div>
      <template v-else-if="data">
        <!-- ===== KPI 指标卡片（由 spec.kpis 驱动，每种库展示自己的指标） ===== -->
        <div class="kpis" v-if="kpiCards.length">
          <div class="kpi" v-for="c in kpiCards" :key="c.key" :class="`lv-${c.level}`">
            <div class="kpi-top">
              <span class="kpi-ico" :style="c.icoStyle"><el-icon><component :is="c.icon" /></el-icon></span>
              <span class="kpi-k">{{ c.label }}</span>
            </div>
            <div class="kpi-v">{{ c.value }}<i v-if="c.unit" class="kpi-unit">{{ c.unit }}</i></div>
            <div class="kpi-d">{{ c.desc || '&nbsp;' }}</div>
          </div>
        </div>

        <!-- ===== 区块面板（由 spec.panels 驱动） ===== -->
        <div class="panel" v-for="p in panels" :key="p.key"
             :class="[`pc-${p.color}`, { 'panel-warn': p.warn && p.rows.length }]"
             :style="{ '--pc': PC_COLORS[p.color] || PC_COLORS.indigo }">
          <div class="panel-head clickable" :class="{ 'is-closed': !bodyVisible(p) }" @click="togglePanel(p.key)">
            <span class="p-chip"><el-icon><component :is="ICONS[p.icon] || Coin" /></el-icon></span>
            <span class="p-title">{{ p.title }}</span>
            <span class="p-count" :class="{ 'c-bad': p.warn && p.rows.length, 'c-ok': p.warn && !p.rows.length, 'c-err': p.error }">
              {{ p.error ? $t('mon.fetchFailedShort') : (p.warn ? (p.rows.length ? $t('mon.nBlocked', { n: p.rows.length }) : $t('mon.noBlocked')) : $t('mon.nRows', { n: p.rows.length })) }}
            </span>
            <span class="p-sub" v-if="p.sortHint && p.rows.length">{{ p.sortHint }}</span>
            <span class="flex1"></span>
            <el-icon class="fold-ico" :class="{ open: bodyVisible(p) }"><ArrowDown /></el-icon>
          </div>

          <template v-if="bodyVisible(p)">
          <!-- 键值网格（关键参数 / 配置类） -->
          <div v-if="p.mode === 'kv'" class="kv-grid">
            <div class="kv" v-for="(row, i) in p.rows" :key="i">
              <span class="kv-k">{{ kvKey(row) }}</span>
              <span class="kv-v">{{ kvMain(row) }}<small v-if="kvUnit(row)" class="cell-unit">{{ kvUnit(row) }}</small></span>
            </div>
          </div>
          <div v-else-if="!p.rows.length" class="panel-empty">
            <template v-if="p.error">
              <span class="empty-ico bad"><el-icon><Warning /></el-icon></span>
              <span class="empty-err">{{ p.error }}</span>
              <span class="empty-hint" v-if="p.note">{{ p.note }}</span>
              <span class="empty-hint" v-else>{{ $t('mon.blockFailed') }}</span>
            </template>
            <template v-else>
              <span class="empty-ico"><el-icon><CircleCheck /></el-icon></span>
              <template v-if="p.warn">{{ $t('mon.noLockWait') }}</template>
              <template v-else>{{ p.note || $t('tdv.noData') }}</template>
            </template>
          </div>
          <div v-else class="table-wrap">
            <table class="sec-table" :class="{ 'fixed-q': p.cols.some(c => isQueryCol(c)), 'wide-q': p.cols.filter(c => isQueryCol(c)).length === 1, 'fixed-sparse': p.sparse }">
              <thead>
                <tr>
                  <th v-for="(col, i) in p.cols" :key="col" :class="{ 'num-th': isNumCol(col), 'q-col': isQueryCol(col) }" :style="p.widths ? p.widths[i] : null">{{ headLabel(col) }}</th>
                  <th v-if="(p.kill && data.killSupported) || p.copy" class="act-col">{{ $t('mon.actions') }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="(row, i) in p.rows" :key="i" :class="{ 'row-warn': p.warn }">
                  <!-- 文本列加原生 title：固定布局会把超出列宽的内容截断，悬停要能看到全文 -->
                  <td v-for="(col, i) in p.cols" :key="col" :class="{ 'num-td': isNumCol(col), 'first-td': col === p.cols[0], 'q-col': isQueryCol(col), 'txt-cap': !isNumCol(col) && !isQueryCol(col) }" :style="p.widths ? p.widths[i] : null" :title="!isNumCol(col) && !isQueryCol(col) ? fmtCell(row[col]) : null">
                    <span v-if="isQueryCol(col)" class="q-cell" :title="fmtCell(row[col])">{{ fmtCell(row[col]) }}</span>
                    <template v-else-if="isNumCol(col)">{{ fmtNum(row[col]) }}<small v-if="unitOf(col) && row[col] != null && row[col] !== ''" class="cell-unit">{{ unitOf(col) }}</small></template>
                    <template v-else>{{ fmtCell(row[col]) }}</template>
                  </td>
                  <td v-if="(p.kill && data.killSupported) || p.copy" class="act-col">
                    <div class="act-group">
                      <button v-if="p.kill && data.killSupported && killIdOf(row, p.idCols)" class="op-btn danger" @click="kill(row, killIdOf(row, p.idCols))">
                        {{ p.warn ? $t('mon.killSource') : $t('mon.kill') }}
                      </button>
                      <button v-if="p.copy" class="op-btn" :disabled="!rowHasQuery(p.cols, row, p.copyCols)"
                              @click="copyText(row[rowQueryCol(p.cols, row, p.copyCols)])">{{ $t('common.copy') }}</button>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
          </template>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.mon-studio { height: 100%; display: flex; flex-direction: column; overflow: hidden; background: var(--dc-bg, #f4f6fa); }

/* ============ Hero 深色渐变头部 ============ */
/* 头部：与面板一致的浅色卡片，弱化存在感 */
.hero {
  flex-shrink: 0; display: flex; align-items: center; justify-content: space-between; gap: 16px;
  margin: 14px 22px 0; padding: 13px 18px;
  color: var(--dc-text, #1f2433);
  background: var(--dc-bg-card, #fff);
  border: 1px solid var(--dc-border, #e7eaf1); border-radius: 12px;
  box-shadow: var(--dc-shadow-sm);
}
.hero-title-row { display: flex; align-items: center; gap: 12px; }
.hero-badge {
  width: 38px; height: 38px; border-radius: 10px; display: flex; align-items: center; justify-content: center;
  background: linear-gradient(135deg, #6366f1, #818cf8); font-size: 20px; color: #fff;
  box-shadow: 0 2px 6px rgba(99, 102, 241, .3); flex-shrink: 0;
}
.hero-title { font-size: 15.5px; font-weight: 700; letter-spacing: .02em; }
.hero-chips { display: flex; align-items: center; gap: 6px; margin-top: 4px; flex-wrap: wrap; }
.chip {
  display: inline-flex; align-items: center; gap: 5px; font-size: 12px; padding: 1px 9px; border-radius: 20px;
  background: var(--dc-bg-hover, #f0f2f7); color: var(--dc-text-dim, #8a93a6);
}
.chip .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--dc-success); }
.chip-ico { font-size: 13px; margin-right: 1px; }
.chip-live { background: var(--dc-success-wash); color: var(--dc-success); }
.live-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--dc-success); animation: pulse 1.4s ease-in-out infinite; }
@keyframes pulse { 0%, 100% { opacity: 1; transform: scale(1); } 50% { opacity: .35; transform: scale(.75); } }
.chip-err { background: var(--dc-danger-wash); color: var(--dc-danger); font-weight: 600; }

.hero-r { display: flex; align-items: center; gap: 12px; flex-shrink: 0; }
.auto-wrap { display: flex; align-items: center; gap: 7px; font-size: 13px; color: var(--dc-text-dim, #8a93a6); cursor: pointer; }
.hero-select { width: 76px; }
.hero-select :deep(.el-select__wrapper) {
  background: var(--dc-bg-card, #fff); box-shadow: 0 0 0 1px var(--dc-border, #dfe3ee) inset;
  min-height: 28px; color: var(--dc-text, #1f2433); border-radius: 14px; padding: 0 10px;
}
.hero-select :deep(.el-select__wrapper:hover) { box-shadow: 0 0 0 1px #b9c2f5 inset; }
.hero-select :deep(.el-select__placeholder), .hero-select :deep(.el-select__selected-item) { color: var(--dc-text, #1f2433); font-size: 13px; }
.hero-select :deep(.el-select__caret) { color: var(--dc-text-dim, #8a93a6); }
.refresh-btn {
  display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 14px; border: none; cursor: pointer;
  border-radius: 15px; font-size: 13px; font-weight: 600; color: #fff;
  background: linear-gradient(135deg, #6366f1, #818cf8);
  box-shadow: 0 2px 6px rgba(99, 102, 241, .28); transition: transform .15s, box-shadow .15s;
}
.refresh-btn:hover:not(:disabled) { transform: translateY(-1px); box-shadow: 0 4px 12px rgba(99, 102, 241, .35); }
.refresh-btn:disabled { opacity: .75; cursor: default; }

/* ============ 滚动区 ============ */
.mon-scroll { flex: 1; overflow: auto; padding: 14px 22px 26px; }
.state-box { padding: 90px 30px; text-align: center; color: var(--dc-text-dim, #8a93a6); display: flex; gap: 10px; align-items: center; justify-content: center; font-size: 14px; }
.state-ico, .state-ico.bad, .state-ico.info { width: 42px; height: 42px; border-radius: 50%; display: flex; align-items: center; justify-content: center; font-size: 20px; }
.state-ico.bad { background: var(--dc-danger-wash); color: var(--dc-danger); }
.state-ico.info { background: var(--dc-primary-wash); color: var(--dc-primary); }
.big-ico { font-size: 30px; color: var(--dc-primary); }

/* ============ KPI 卡片 ============ */
.kpis { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 12px; margin-bottom: 16px; }
.kpi { position: relative; overflow: hidden; border-radius: 12px; padding: 13px 15px 11px;
  background: var(--dc-bg-card, #fff); border: 1px solid var(--dc-border, #e7eaf1);
  box-shadow: var(--dc-shadow-sm);
  transition: transform .18s, box-shadow .18s;
}
.kpi:hover { transform: translateY(-2px); box-shadow: 0 6px 16px rgba(23, 33, 70, .1); }
.kpi-top { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; }
.kpi-ico {
  width: 30px; height: 30px; border-radius: 8px; display: flex; align-items: center; justify-content: center;
  font-size: 15px; color: #fff; background: var(--ico, linear-gradient(135deg, #6366f1, #818cf8));
  box-shadow: 0 3px 8px rgba(99, 102, 241, .3);
}
.kpi.lv-good .kpi-ico { --ico: linear-gradient(135deg, #10b981, #34d399); box-shadow: 0 3px 8px rgba(16, 185, 129, .3); }
.kpi.lv-warn .kpi-ico { --ico: linear-gradient(135deg, #f59e0b, #fbbf24); box-shadow: 0 3px 8px rgba(245, 158, 11, .3); }
.kpi.lv-bad .kpi-ico { --ico: linear-gradient(135deg, #f43f5e, #fb7185); box-shadow: 0 3px 8px rgba(244, 63, 94, .3); }
.kpi-k { font-size: 13px; color: var(--dc-text-dim, #8a93a6); font-weight: 500; }
.kpi-v { font-size: 22px; line-height: 1.2; font-weight: 750; color: var(--dc-text, #1f2433); font-variant-numeric: tabular-nums; }
.kpi-unit { font-style: normal; font-size: 13px; font-weight: 400; color: var(--dc-text-dim, #8a93a6); margin-left: 2px; }
.kpi-d { font-size: 12px; color: var(--dc-text-dim, #8a93a6); margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

/* ============ 未提供指标提醒 ============ */
.unsupported-note {
  display: flex; align-items: center; flex-wrap: wrap; gap: 6px;
  margin-bottom: 14px; padding: 9px 14px; border-radius: 10px; font-size: 13px;
  background: var(--dc-primary-wash); border: 1px dashed var(--dc-primary-glow); color: var(--dc-text-dim, #8a93a6);
}
.unsupported-note .un-ico { font-size: 14px; color: var(--dc-primary); }
.unsupported-note .un-tags { display: inline-flex; flex-wrap: wrap; gap: 5px; }
.unsupported-note .un-tag {
  font-style: normal; font-size: 12px; padding: 1px 9px; border-radius: 20px;
  background: var(--dc-primary-wash); color: var(--dc-primary); font-weight: 600;
}
.unsupported-note .un-err { color: var(--dc-danger); font-weight: 500; word-break: break-all; }
.c-err { background: var(--dc-warning-wash) !important; color: var(--dc-warning) !important; }
.empty-ico.bad { background: var(--dc-warning-wash); color: var(--dc-warning); }
.empty-err { font-size: 13px; color: var(--dc-warning); max-width: 560px; word-break: break-all; }
.empty-hint { font-size: 12px; color: var(--dc-text-dim, #9aa3b5); }

/* ============ 区块面板 ============ */
.panel {
  border-radius: 12px; margin-bottom: 14px; overflow: hidden;
  background: var(--dc-bg-card, #fff); border: 1px solid var(--dc-border, #e7eaf1);
  box-shadow: var(--dc-shadow-sm);
}
.panel-warn { border-color: rgba(244, 63, 94, .45); box-shadow: 0 0 0 3px rgba(244, 63, 94, .07), 0 2px 8px rgba(244, 63, 94, .1); }
.panel-head { display: flex; align-items: center; gap: 10px; padding: 11px 16px; border-bottom: 1px solid var(--dc-border, #eef0f6); }
/* 面板折叠：头部可点击，折叠态隐藏分隔线 */
.panel-head.clickable { cursor: pointer; user-select: none; }
.panel-head.clickable:hover { background: var(--dc-bg-soft, #fafbfd); }
.panel-head.is-closed { border-bottom-color: transparent; }
.fold-ico { font-size: 14px; color: var(--dc-text-dim, #8a93a6); transition: transform .18s; }
.fold-ico.open { transform: rotate(180deg); }
.p-chip {
  width: 27px; height: 27px; border-radius: 8px; display: flex; align-items: center; justify-content: center;
  font-size: 14px; color: #fff; background: var(--pc, linear-gradient(135deg, #6366f1, #818cf8));
  box-shadow: 0 2px 6px rgba(99, 102, 241, .3); flex-shrink: 0;
}
.pc-rose .p-chip { --pc: linear-gradient(135deg, #f43f5e, #fb7185); box-shadow: 0 2px 6px rgba(244, 63, 94, .3); }
.pc-indigo .p-chip { --pc: linear-gradient(135deg, #6366f1, #818cf8); }
.pc-amber .p-chip { --pc: linear-gradient(135deg, #f59e0b, #fbbf24); box-shadow: 0 2px 6px rgba(245, 158, 11, .3); }
.pc-emerald .p-chip { --pc: linear-gradient(135deg, #10b981, #34d399); box-shadow: 0 2px 6px rgba(16, 185, 129, .3); }
.pc-violet .p-chip { --pc: linear-gradient(135deg, #8b5cf6, #a78bfa); box-shadow: 0 2px 6px rgba(139, 92, 246, .3); }
.pc-sky .p-chip { --pc: linear-gradient(135deg, #0ea5e9, #38bdf8); box-shadow: 0 2px 6px rgba(14, 165, 233, .3); }
.panel-warn .p-chip { --pc: linear-gradient(135deg, #f43f5e, #fb7185) !important; }
.p-title { font-size: 13.5px; font-weight: 650; color: var(--dc-text, #1f2433); }
.p-count {
  font-size: 12px; font-weight: 600; border-radius: 20px; padding: 2px 10px;
  background: var(--dc-bg-hover, #f0f2f7); color: var(--dc-text-dim, #8a93a6);
}
.c-bad { background: var(--dc-danger-wash); color: var(--dc-danger); }
.c-ok { background: var(--dc-success-wash); color: var(--dc-success); }
.p-sub { font-size: 12px; color: var(--dc-text-dim, #9aa3b5); }
.flex1 { flex: 1; }

.panel-empty { padding: 34px; text-align: center; font-size: 14px; color: var(--dc-text-dim, #8a93a6); display: flex; flex-direction: column; align-items: center; gap: 10px; }
.empty-ico { width: 44px; height: 44px; border-radius: 50%; display: flex; align-items: center; justify-content: center; font-size: 21px; background: var(--dc-success-wash); color: var(--dc-success); }

/* ============ 表格 ============ */
/* 面板通栏，长表格在面板内限高滚动，表头吸顶 */
.table-wrap { overflow-x: auto; overflow-y: auto; max-height: 420px; }
.sec-table { width: 100%; border-collapse: separate; border-spacing: 0; font-size: 13px; }
.sec-table th, .sec-table td { padding: 9px 12px; text-align: left; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; vertical-align: middle; }
/* 普通文本列限宽，query 列不限宽以占满剩余空间 */
.sec-table th.txt-cap, .sec-table td.txt-cap { max-width: 240px; }
/* 少列表格用的是 colPlan 精确算出的列宽，这里不能再叠一层 240px 上限（会与固定布局打架） */
.sec-table.fixed-sparse th.txt-cap, .sec-table.fixed-sparse td.txt-cap { max-width: none; }
.sec-table thead th {
  font-weight: 600; font-size: 12px; letter-spacing: .04em; color: var(--dc-text-dim, #8a93a6);
  background: var(--dc-bg-soft, #fafbfd); border-bottom: 1px solid var(--dc-border, #eef0f6);
  user-select: none;
  position: sticky; top: 0; z-index: 2;
}
.sec-table tbody td { border-bottom: 1px solid var(--dc-border, #f1f3f8); color: var(--dc-text, #333a4c); }
.sec-table tbody tr:last-child td { border-bottom: none; }
.sec-table tbody tr { transition: background .12s; }
.sec-table tbody tr:hover td { background: rgba(99, 102, 241, .05); }
.first-td { font-weight: 550; }
/* 数字列收缩为内容宽度（width:1%），富余空间让给 query 列；保证总宽不超出容器、不出横向滚动条 */
.sec-table th.num-th { text-align: right !important; width: 1%; padding-left: 14px; }
.sec-table td.num-td { text-align: right; width: 1%; padding-left: 14px; font-variant-numeric: tabular-nums; }
/* 数值列后面还有列时加大右侧留白：右对齐数值紧贴下一列左对齐文本会显得挤（如 time|state） */
.sec-table th.num-th:not(:last-child), .sec-table td.num-td:not(:last-child) { padding-right: 24px; }
/* 数值后的小字单位（MB / s / % 等） */
.cell-unit { margin-left: 3px; font-size: 11px; font-weight: 400; color: var(--dc-text-dim, #8a93a6); }
.sec-table th.q-col, .sec-table td.q-col { width: 100%; }
/* 含 SQL 列的表格用固定布局：表格宽度恒等于容器宽，SQL 省略号截断，不再横向溢出 */
.sec-table.fixed-q { table-layout: fixed; }
.sec-table.fixed-q th.q-col, .sec-table.fixed-q td.q-col { width: auto; }
.sec-table.fixed-q th.num-th, .sec-table.fixed-q td.num-td { width: 110px; }
/* 仅一个 SQL 列的表格（进程 info / 慢查询 query）：SQL 列固定占比加宽，其余文本列均分剩余 */
.sec-table.wide-q th.q-col, .sec-table.wide-q td.q-col { width: 34%; }
/* 少列表格：**固定布局 + colPlan 算出的列宽**。
   数字列 110px、短文本列 120px、长文本列按内容权重分百分比（见 colPlan）。
   之所以用 fixed：auto 布局下浏览器会拿这些宽度当"建议"再按内容分配一次，
   宽窄就又不听话了（ClickHouse「表空间」被 db 独吞整宽就是这么来的）。
   内容过长由 td 的 ellipsis 截断（见上面 .sec-table th, td 的 overflow 设置）。 */
.sec-table.fixed-sparse { table-layout: fixed; }
.sec-table.fixed-sparse th.act-col, .sec-table.fixed-sparse td.act-col { text-align: center !important; }
.row-warn td { background: rgba(244, 63, 94, .035); }
.row-warn:hover td { background: rgba(244, 63, 94, .08) !important; }
/* 操作列：固定宽度强制水平居中，表头「操作」与「终止/复制」按钮严格对齐 */
.sec-table th.act-col, .sec-table td.act-col {
  width: 74px !important; min-width: 74px; max-width: 74px;
  text-align: center !important; padding: 9px 6px !important; vertical-align: middle;
}
.op-btn { white-space: nowrap; }
/* 操作列按钮组：终止+复制纵向排列 */
.act-group { display: flex; flex-direction: column; align-items: center; gap: 4px; }
.q-cell {
  display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-family: 'SF Mono', ui-monospace, Consolas, monospace; font-size: 11.5px; color: var(--dc-primary);
  background: var(--dc-primary-wash); border-radius: 5px; padding: 2px 8px;
}

/* ============ 操作按钮 ============ */
.op-btn {
  border: 1px solid var(--dc-border, #dde1ec); background: var(--dc-bg-card, #fff); color: var(--dc-primary);
  font-size: 11.5px; font-weight: 600; padding: 3px 11px; border-radius: 14px; cursor: pointer; transition: all .15s;
}
.op-btn:hover { background: var(--dc-primary-wash); border-color: var(--dc-primary-light); }
.op-btn:disabled { opacity: .45; cursor: not-allowed; }
.op-btn.danger { color: var(--dc-danger); }
.op-btn.danger:hover { background: var(--dc-danger-wash); border-color: var(--dc-danger); }

/* ============ 键值网格 ============ */
.kv-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 9px; padding: 14px 16px; }
.kv {
  background: var(--dc-bg-soft, #f8f9fc); border: 1px solid var(--dc-border, #eef0f6); border-radius: 9px;
  padding: 8px 12px; min-width: 0; transition: border-color .15s;
}
.kv:hover { border-color: #b9c2f5; }
.kv-k { display: block; font-size: 12px; color: var(--dc-text-dim, #8a93a6); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.kv-v { display: block; font-size: 14px; font-weight: 650; color: var(--dc-text, #1f2433); word-break: break-all; }
</style>
