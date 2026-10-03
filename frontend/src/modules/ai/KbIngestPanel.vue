<script setup>
/**
 * 知识入库：一批资料 → 一个知识库。
 *
 * <p>流程本身就是分步跑的（体检 → 优化 → 建档 → 导入），界面也照这个节奏切成步骤：
 * 每一步只聚焦一件事，做完再进下一步，避免把体检结论、优化明细、建档方案全堆在一屏。
 *
 * <p>语义要点：多份文件进来，落库时是「一个知识库 + 多份资料」，不是各建一个库。
 * 所以逐份做的事（体检、优化、导入）和整批只做一次的事（库名、说明、分段方式、
 * 索引方式）是分开的。
 *
 * <p>取舍：模型不可用、单份处理失败都不拦整批 —— 该放行时放行，把风险写清楚。
 */
import { ref, computed } from 'vue'
import { ElMessage } from 'element-plus'
import { t, te, locale } from '../../utils/i18n'
import {
  UploadFilled, Document, Loading, MagicStick, CircleCheckFilled, CircleCloseFilled,
  WarningFilled, InfoFilled, Plus, FolderAdd, RefreshLeft, Check
} from '@element-plus/icons-vue'
import {
  aiKbInspect, aiKbPolish, aiKbAutoPlan, kbList, kbCreate, kbSaveConfig, kbImportDoc
} from '../../api'

/** 能读进来的文件类型：与 input 的 accept 一致，并在界面上显式告诉用户 */
const ACCEPT = '.txt,.md,.markdown,.csv,.tsv,.json,.sql,.log,.yml,.yaml,.xml,.html,.properties,.conf'
const ACCEPT_LABEL = '.txt / .md / .markdown / .csv / .tsv / .json / .sql / .log / .yml / .yaml / .xml / .html / .properties / .conf'
/** 单份建议上限：超过它会拆成很多批调用模型，既慢又贵 */
const SIZE_HINT = 2 * 1024 * 1024

/** 步骤定义 */
// ⚠️ 必须是 computed：普通常量只在模块加载时求值一次，之后切语言不会跟着变
const STEPS = computed(() => [t('kbx.stepUpload'), t('kbx.stepInspect'), t('kbx.stepPolish'), t('kbx.stepConfirm'), t('kbx.stepDone')])

/** 交给外层打开「知识库」页签：入库完最自然的下一步就是去看分块 */
const emit = defineEmits(['open-knowledge'])

const fileInput = ref(null)
const drag = ref(false)
const reading = ref(false)
/** 整批阶段：idle | processing | planned | creating | done */
const stage = ref('idle')
const busyText = ref('')

/** 资料队列：每份各自体检、优化、导入 */
const docs = ref([])
/** 库级方案：整批共用一套（库名、说明、分段、索引都在这里） */
const plan = ref(null)
const form = ref({ name: '', description: '' })
const kbCreated = ref(null)
/** 存到哪里：new=新建知识库；append=作为新资料插入已有知识库 */
const targetMode = ref('new')
const targetKbId = ref('')
const kbOptions = ref([])
let seq = 0

/** 体检通过、待处理的份数 */
const okCount = computed(() => docs.value.filter(d => d.phase === 'ready').length)
const plannedCount = computed(() => docs.value.filter(d => d.phase === 'planned').length)
const doneCount = computed(() => docs.value.filter(d => d.phase === 'done').length)
const rejectCount = computed(() => docs.value.filter(d => d.phase === 'rejected').length)
const failedCount = computed(() => docs.value.filter(d => d.phase === 'failed').length)
const working = computed(() => docs.value.some(d =>
  ['inspecting', 'polishing', 'planning', 'creating'].includes(d.phase)))

/**
 * 当前步骤：跟着流程状态走，而不是让用户自己点着跳 ——
 * 这一步该看什么由流程决定，用户只需要在每步末尾确认。
 */
const activeStep = computed(() => {
  const d = docs.value
  if (!d.length) return 0
  if (d.some(x => x.phase === 'inspecting')) return 1
  if (d.some(x => ['polishing', 'planning'].includes(x.phase))) return 2
  if (d.some(x => x.phase === 'creating')) return 3
  if (stage.value === 'done') return 4
  if (stage.value === 'planned') return 3
  if (d.some(x => x.polish)) return 2
  return 1
})

/** 全部走完时，最后一步也该是「已完成」而不是停在「进行中」的主色上 */
const stepDone = (i) => i < activeStep.value
  || (activeStep.value === STEPS.value.length - 1 && stage.value === 'done')
const stepProcess = (i) => !stepDone(i) && i === activeStep.value

const PHASE_TEXT = {
  pending: 'kbx.phase.pending', inspecting: 'kbx.phase.inspecting', rejected: 'kbx.phase.rejected', ready: 'kbx.phase.ready',
  polishing: 'kbx.phase.polishing', planning: 'kbx.phase.planning', planned: 'kbx.phase.planned',
  creating: 'kbx.phase.creating', done: 'kbx.phase.done', failed: 'kbx.phase.failed'
}
// 状态名只有代码：字典里有就用译文，没有就留空（后端以后新增的状态不会显示成键名）
const phaseText = (d) => (d.phase && te(PHASE_TEXT[d.phase])) ? t(PHASE_TEXT[d.phase]) : ''
// 严重度名（重要/一般/轻微）：字典里有就用译文，没有才回落到「一般」
const SEVERITY_KEYS = { high: 'kbx.sev.high', mid: 'kbx.sev.mid', low: 'kbx.sev.low' }
const severityLabel = (s) => (s && te(SEVERITY_KEYS[s])) ? t(SEVERITY_KEYS[s]) : t('kbx.sev.mid')

// ---------- 选文件 ----------
const pickFiles = () => fileInput.value?.click?.()

const addFiles = async (fileList) => {
  const list = Array.from(fileList || [])
  // 静默返回最难受：看得到的是「点了没反应」，看不到的是哪里出的问题
  if (!list.length) { ElMessage.warning(t('kbx.noFiles')); return }
  reading.value = true
  const added = []
  for (const f of list) {
    if (!f.size) { ElMessage.warning(t('kbx.emptyFile', { name: f.name })); continue }
    if (f.size > SIZE_HINT) ElMessage.warning(t('kbx.tooBig', { name: f.name }))
    try {
      const text = await f.text()
      if (!text.trim()) { ElMessage.warning(t('kbx.noText', { name: f.name })); continue }
      const item = {
        id: ++seq,
        title: f.name.replace(/\.[^.]+$/, ''),
        source: f.name,
        text,
        phase: 'pending',
        inspect: null,
        polish: null,
        error: '',
        note: '',
        open: false,
        progress: null,
        chunks: 0
      }
      docs.value.push(item)
      // 必须取回数组里的响应式代理再往下传：直接改原始对象不会触发视图更新，
      // 表现就是「体检请求发出去了，界面一直没反应」
      added.push(docs.value[docs.value.length - 1])
    } catch (e) {
      ElMessage.error(t('kbx.readFailed', { name: f.name, detail: (e?.message || e) }))
    }
  }
  reading.value = false
  if (added.length) await inspectDocs(added)
}

const onFiles = async (ev) => {
  // 必须先把 FileList 复制成数组：input.value = '' 会连带清空 FileList，
  // 先存引用再清空的话，传下去的就是空列表（表现为「选了文件却毫无反应」）
  const files = Array.from(ev?.target?.files || [])
  if (ev?.target) ev.target.value = ''
  await addFiles(files)
}

const onDrop = async (ev) => {
  drag.value = false
  if (working.value) return
  // 同样先复制成数组：DataTransfer 的 files 也是活引用，处理过程中可能失效
  await addFiles(Array.from(ev?.dataTransfer?.files || []))
}

const removeDoc = (d) => {
  docs.value = docs.value.filter(x => x.id !== d.id)
  if (!docs.value.length) { stage.value = 'idle'; plan.value = null }
}

const resetAll = () => {
  docs.value = []
  plan.value = null
  form.value = { name: '', description: '' }
  kbCreated.value = null
  targetMode.value = 'new'
  targetKbId.value = ''
  stage.value = 'idle'
  seq = 0
}

/** 拉一次已有知识库清单，供「插入到现有知识库」选择 */
const loadKbOptions = async () => {
  try {
    const d = await kbList()
    kbOptions.value = (d && d.items) || []
  } catch (e) {
    kbOptions.value = []
  }
}

/** 当前选中的目标库（插入模式用） */
const targetKb = computed(() => kbOptions.value.find(k => k.id === targetKbId.value) || null)

// ---------- 第 2 步：逐份体检 ----------
const inspectDocs = async (list) => {
  for (const d of list) {
    d.phase = 'inspecting'
    busyText.value = t('kbx.inspecting', { name: d.source })
    try {
      const res = await aiKbInspect({ text: d.text, source: d.source })
      if (res?.success === false) throw new Error(res.message || t('kbx.inspectFailed'))
      d.inspect = res
      d.phase = res.suitable ? 'ready' : 'rejected'
    } catch (e) {
      // 体检失败不拦人：放行，让用户自己决定
      d.inspect = { suitable: true, kind: '', verdict: '', problems: [], stats: {}, fatal: '' }
      d.note = t('kbx.inspectIncomplete', { detail: (e?.message || e) })
      d.phase = 'ready'
    }
  }
  busyText.value = ''
}

// ---------- 第 3 步：逐份优化 ----------
/**
 * 按段落边界切批。
 *
 * 一次发一整篇长文给模型，容易被它自作主张地概括压缩；切小批后每批都短，
 * 它没有「空间」去改写，只能老老实实做清洗 —— 顺带让进度看得见、单批失败可兜底。
 */
const splitBatches = (text, size = 5000) => {
  const paras = text.split(/\n\s*\n/)
  const out = []
  let cur = ''
  for (const p of paras) {
    if (p.length > size) {
      if (cur) { out.push(cur); cur = '' }
      for (let i = 0; i < p.length; i += size) out.push(p.slice(i, i + size))
      continue
    }
    if (cur && cur.length + p.length + 2 > size) { out.push(cur); cur = '' }
    cur = cur ? cur + '\n\n' + p : p
  }
  if (cur) out.push(cur)
  return out
}

const polishOne = async (d) => {
  const batches = splitBatches(d.text)
  const pieces = []
  const rows = []
  const notes = []
  const missingAll = []
  let originalChars = 0
  let polishedChars = 0
  let factsTotal = 0
  let factsKept = 0

  for (let i = 0; i < batches.length; i++) {
    d.progress = { done: i, total: batches.length }
    try {
      const r = await aiKbPolish({ text: batches[i], source: d.source })
      if (r?.success === false) throw new Error(r.message || t('kbx.polishFailed'))
      pieces.push(r.text)
      originalChars += r.originalChars || batches[i].length
      polishedChars += r.polishedChars || 0
      factsTotal += r.factsTotal || 0
      factsKept += r.factsKept || 0
      missingAll.push(...(r.factsMissing || []))
      if (r.note) notes.push(t('kbx.batchNote', { i: i + 1, note: r.note }))
      if (r.risk) notes.push(t('kbx.batchNote', { i: i + 1, note: r.risk }))
      rows.push({ idx: i + 1, originalChars: r.originalChars || 0, polishedChars: r.polishedChars || 0 })
    } catch (e) {
      // 单批失败不放弃整份：这一批保留原文继续往下走
      pieces.push(batches[i])
      originalChars += batches[i].length
      polishedChars += batches[i].length
      notes.push(t('kbx.batchPolishFailed', { i: i + 1, detail: (e?.message || e) }))
      rows.push({ idx: i + 1, originalChars: batches[i].length, polishedChars: batches[i].length, failed: true })
    }
  }
  d.progress = { done: batches.length, total: batches.length }

  d.polish = {
    text: pieces.join('\n\n'),
    batches: rows,
    originalChars,
    polishedChars,
    factsTotal,
    factsKept,
    keepRate: factsTotal ? Math.round((factsKept / factsTotal) * 1000) / 1000 : 1,
    factsMissing: missingAll.slice(0, 12),
    factsMissingCount: missingAll.length,
    notes
  }
}

// ---------- 整批：优化 → 定一套库级方案 ----------
const startProcess = async () => {
  const list = docs.value.filter(d => d.phase === 'ready')
  if (!list.length) return
  stage.value = 'processing'
  for (let i = 0; i < list.length; i++) {
    const d = list[i]
    d.phase = 'polishing'
    busyText.value = t('kbx.polishing', { i: i + 1, n: list.length, name: d.source })
    try {
      await polishOne(d)
      d.phase = 'planned'
    } catch (e) {
      d.error = e?.message || t('kbx.polishFailed')
      d.phase = 'failed'
    }
  }

  // 库级方案：库名要贴合整批内容，分段参数也要按整批的形态定。
  //
  // 注意：这里只是把各份「并排」交给模型看，让它知道有多份资料；
  // 入库时每份仍是独立的一份资料，正文不会被拼接合并。
  const kept = list.filter(d => d.phase === 'planned')
  if (!kept.length) {
    busyText.value = ''
    stage.value = 'idle'
    return
  }
  const bundled = kept
    .map((d, i) => t('kbx.batchBlock', { i: i + 1, name: d.source }) + '\n' + d.polish.text)
    .join('\n\n')
  busyText.value = t('kbx.planningAll')
  try {
    const res = await aiKbAutoPlan({
      text: bundled,
      // 兜底名称只用第一份的文件名：多份拼起来的名字反而没法看
      source: kept[0].source
    })
    if (res?.success === false) throw new Error(res.message || t('kbx.planFailed'))
    plan.value = res
    form.value = { name: res.name || t('kbx.unnamed'), description: res.description || '' }
    stage.value = 'planned'
    if (res.aiError) ElMessage.warning(t('kbx.aiNameFailed', { detail: res.aiError }))
    // 方案就绪后才需要「存到哪里」，这时再拉清单，避免拿到过期的库列表
    await loadKbOptions()
  } catch (e) {
    ElMessage.error(t('kbx.planFailedDetail', { detail: (e?.message || e) }))
    stage.value = 'idle'
  }
  busyText.value = ''
}

// ---------- 第 4 步：入库（新建库，或插入到已有库） ----------
const startCreate = async () => {
  const list = docs.value.filter(d => d.phase === 'planned' || d.phase === 'done')
  if (!list.length) return

  const name = (form.value.name || '').trim()
  if (targetMode.value === 'new' && !name) { ElMessage.warning(t('kbx.needName')); return }
  if (targetMode.value === 'append' && !targetKbId.value) { ElMessage.warning(t('kbx.needTargetKb')); return }

  stage.value = 'creating'
  try {
    let kbId = targetMode.value === 'append' ? targetKbId.value : kbCreated.value?.kbId

    if (targetMode.value === 'new' && !kbId) {
      const created = await kbCreate({ name, description: (form.value.description || '').trim() })
      if (!created?.success) throw new Error(created?.message || t('kbx.createFailed'))
      kbId = created.item?.id || created.kbId || created.info?.id
      if (!kbId) throw new Error(t('kbx.noKbId'))
      kbCreated.value = { kbId, name, description: (form.value.description || '').trim() }

      // 只有新建库才应用本批的分段与索引方案。
      // 插入已有库时不碰全局配置 —— 把那个库原有的切法改掉，已有的块就和新块对不上了。
      const cur = await kbList()
      const c = (cur && cur.config) || {}
      const r = plan.value?.recommend || {}
      await kbSaveConfig({
        // 后端按 kbId 存配置（漏传会在创建导入时报「缺少 kbId 参数」，真机踩过）
        kbId, ...c, ...r,
        buildVector: plan.value ? !!plan.value.buildVector : c.buildVector,
        indexMode: plan.value?.indexMode || c.indexMode
      })
    }

    for (let i = 0; i < list.length; i++) {
      const d = list[i]
      if (d.phase === 'done') continue
      d.phase = 'creating'
      busyText.value = t('kbx.importing', { i: i + 1, n: list.length, name: d.source })
      try {
        const imp = await kbImportDoc({
          kbId, title: d.title, source: d.source, text: d.polish.text
        })
        if (!imp?.success) throw new Error(imp?.message || t('kbx.importFailed'))
        d.chunks = imp.chunkCount || 0
        d.phase = 'done'
      } catch (e) {
        d.error = e?.message || t('kbx.importFailed')
        d.phase = 'failed'
      }
    }
    if (targetMode.value === 'append' && !kbCreated.value) {
      // 插入模式：报告里要能看出「加进了哪个库」
      kbCreated.value = {
        kbId, name: targetKb.value?.name || t('kbx.existingKb'), description: targetKb.value?.description || ''
      }
    }
    stage.value = 'done'
  } catch (e) {
    ElMessage.error(e?.message || t('kbx.ingestFailed'))
    stage.value = 'planned'
  }
  busyText.value = ''
}

// ---------- 展示用派生 ----------
const shapeLabel = computed(() => {
  const s = (plan.value?.stats || {}).shape
  // 形态名只有代码（markdown / lines / prose）：按代码查字典覆盖，没收录的回落「常规文本」
  return (s && te('kbx.shape.' + s)) ? t('kbx.shape.' + s) : t('kbx.shape.plain')
})

const planLabel = computed(() => {
  const r = plan.value?.recommend || {}
  if (!r.chunkMode) return ''
  if (r.chunkMode === 'parentChild') {
    return t('kbx.planParent', { p: r.parentSize, c: r.childSize })
  }
  return t('kbx.planGeneric', { n: (r.chunkSize || 0) })
    + (r.separator ? t('kbx.planBySep', { sep: r.separator }) : t('kbx.planByBlank'))
})

/** 本次待入库（已优化、尚未导入）的资料：统计只看这批，不含上一轮已入库的 */
const batchDocs = computed(() => docs.value.filter(d => d.polish && d.phase !== 'done'))

const polishTotal = computed(() => {
  const arr = batchDocs.value
  return {
    count: arr.length,
    originalChars: arr.reduce((s, d) => s + d.polish.originalChars, 0),
    polishedChars: arr.reduce((s, d) => s + d.polish.polishedChars, 0),
    factsTotal: arr.reduce((s, d) => s + d.polish.factsTotal, 0),
    factsKept: arr.reduce((s, d) => s + d.polish.factsKept, 0),
    missingCount: arr.reduce((s, d) => s + d.polish.factsMissingCount, 0)
  }
})

const polishKeepRate = computed(() => {
  const t = polishTotal.value
  return t.factsTotal ? Math.round((t.factsKept / t.factsTotal) * 1000) / 1000 : 1
})

/** 预计总块数：按本批优化后的字数估 */
const estChunks = computed(() => {
  const r = plan.value?.recommend || {}
  if (!r.chunkMode) return 0
  const size = r.chunkMode === 'parentChild' ? (r.childSize || 512) : (r.chunkSize || 2600)
  const chars = batchDocs.value.reduce((s, d) => s + d.polish.polishedChars, 0)
  return Math.max(1, Math.ceil(chars / size))
})

const totalChunks = computed(() => docs.value.reduce((s, d) => s + (d.chunks || 0), 0))

/** 整批进度：已处理完的份数 + 当前份的批次进度。处理中只有一个干等的转圈太熬人 */
const progressPercent = computed(() => {
  const total = docs.value.length || 1
  const finished = docs.value.filter(d => ['planned', 'done', 'failed', 'rejected'].includes(d.phase)).length
  const cur = docs.value.find(d => d.progress && d.progress.total)
  const frac = cur ? cur.progress.done / Math.max(1, cur.progress.total) : 0
  return Math.min(99, Math.round(((finished + frac) / total) * 100))
})

/** 大数字换成好读的量级：48668 看着累，4.9 万一目了然 */
/** 完成时间：按当前界面语言格式化（原来写死 'zh-CN'，英文界面下也会显示中文格式） */
const fmtDateTime = (d) => d.toLocaleString(locale.value)

const fmtNum = (n) => {
  const v = Number(n) || 0
  // 中文以「万」进位、英文以「K」进位：同一个数字在两种语言下的可读写法不同
  if (v >= 10000) return locale.value === 'en-US' ? (v / 1000).toFixed(1) + t('kbx.unitK') : (v / 10000).toFixed(1) + t('kbx.unitWan')
  return String(v)
}

const diffPercentOf = (d) => {
  if (!d.polish || !d.polish.originalChars) return '0'
  return (((d.polish.polishedChars - d.polish.originalChars) / d.polish.originalChars) * 100).toFixed(1)
}
const toggle = (d) => { d.open = !d.open }
</script>

<template>
  <div class="kbx"
       @dragover.prevent="drag = true"
       @dragleave="(e) => { if (e.currentTarget === e.target) drag = false }"
       @drop.prevent="onDrop">

    <!-- ===== 顶部步骤条：让「现在在哪一步、还剩几步」一眼可见 =====
         自己写而不是用 el-steps：它的圆点默认居中在各自格内，两端会空出半格，
         视觉宽度永远和下面的面板对不齐。这里让圆点固定宽、连接线用 flex 拉伸，
         圆点自然均匀铺满整宽且两端贴边。 -->
    <div class="kbx-steps">
      <template v-for="(s, i) in STEPS" :key="i">
        <div class="kbx-step" :class="{ 'is-done': stepDone(i), 'is-process': stepProcess(i) }">
          <span class="kbx-step-dot">
            <el-icon v-if="stepDone(i)"><Check /></el-icon>
            <template v-else>{{ i + 1 }}</template>
          </span>
          <span class="kbx-step-label">{{ s }}</span>
        </div>
        <span v-if="i < STEPS.length - 1" class="kbx-step-line" :class="{ done: stepDone(i) }" />
      </template>
    </div>

    <div class="kbx-stage">
      <!-- ===== 第 1 步：上传 ===== -->
      <template v-if="activeStep === 0">
        <div class="kbx-drop" :class="{ over: drag, busy: reading }" @click="pickFiles">
          <el-icon class="kbx-drop-ic"><UploadFilled /></el-icon>
          <div class="kbx-drop-title">
            {{ reading ? $t('kbx.reading') : $t('kbx.dropHint') }}
          </div>
          <div class="kbx-drop-types">
            <span class="kbx-types-label">{{ $t('kbx.supportTypes') }}</span>
            <span class="kbx-types">{{ ACCEPT_LABEL }}</span>
          </div>
          <div class="kbx-drop-desc">
            {{ $t('kbx.tipA') }}<strong>{{ $t('kbx.tipB') }}</strong>{{ $t('kbx.tipC') }}<strong>{{ $t('kbx.tipD') }}</strong>{{ $t('kbx.tipE') }}
          </div>
        </div>
      </template>

      <!-- ===== 第 2 步：体检 ===== -->
      <template v-else-if="activeStep === 1">
        <div class="kbx-bar">
          <div class="kbx-bar-left">
            <span class="kbx-bar-title">{{ $t('kbx.stepInspect') }}</span>
            <span class="kbx-bar-sub">
              {{ $t('kbx.countDocs', { n: docs.length }) }}
              <template v-if="okCount"><b class="ok">{{ $t('kbx.countOk', { n: okCount }) }}</b></template>
              <template v-if="rejectCount"><b class="bad">{{ $t('kbx.countReject', { n: rejectCount }) }}</b></template>
            </span>
          </div>
          <el-button size="small" text :icon="Plus" :disabled="working" @click="pickFiles">{{ $t('kbx.addMore') }}</el-button>
        </div>

        <div class="kbx-list">
          <div v-for="d in docs" :key="d.id" class="kbx-item" :class="[d.phase, { open: d.open }]">
            <div class="kbx-item-head" @click="toggle(d)">
              <el-icon class="kbx-item-ic">
                <Loading v-if="d.phase === 'inspecting'" class="is-loading" />
                <CircleCloseFilled v-else-if="d.phase === 'rejected'" />
                <Document v-else />
              </el-icon>
              <span class="kbx-item-name">{{ d.source }}</span>
              <span class="kbx-item-size">{{ $t('kbx.chars', { n: fmtNum(d.text.length) }) }}</span>
              <span class="kbx-badge" :class="d.phase">{{ phaseText(d) }}</span>
              <el-button size="small" text :disabled="working" @click.stop="removeDoc(d)">{{ $t('kbx.remove') }}</el-button>
            </div>

            <div class="kbx-item-sum">
              <template v-if="d.phase === 'inspecting'">{{ $t('kbx.inspectingShort') }}</template>
              <template v-else-if="d.phase === 'rejected'">
                <span class="bad-text">{{ d.inspect?.rejectReason }}</span>
              </template>
              <template v-else>
                {{ d.inspect?.kind || $t('kbx.shape.plain') }}
                <span v-if="d.inspect?.problems?.length" class="dim">{{ $t('kbx.foundProblems', { n: d.inspect.problems.length }) }}</span>
                <span v-if="d.note" class="dim">· {{ d.note }}</span>
              </template>
            </div>

            <div v-if="d.open" class="kbx-item-body">
              <div v-if="d.inspect?.stats" class="kbx-metrics">
                <div class="kbx-metric">
                  <b>{{ d.inspect.stats.contentChars || 0 }}</b><span>{{ $t('kbx.statChars') }}</span>
                </div>
                <div class="kbx-metric">
                  <b>{{ d.inspect.stats.effectiveLines || 0 }}</b><span>{{ $t('kbx.statLines') }}</span>
                </div>
                <div class="kbx-metric">
                  <b>{{ Math.round((d.inspect.stats.duplicateRatio || 0) * 100) }}%</b><span>{{ $t('kbx.statDupRatio') }}</span>
                </div>
                <div class="kbx-metric">
                  <b>{{ Math.round((d.inspect.stats.printableRatio || 1) * 100) }}%</b><span>{{ $t('kbx.statPrintable') }}</span>
                </div>
              </div>
              <div v-for="(w, i) in (d.inspect?.stats?.warnings || [])" :key="'w' + i" class="kbx-warn">
                <el-icon><WarningFilled /></el-icon><span>{{ w }}</span>
              </div>
              <div v-if="d.inspect?.verdict" class="kbx-verdict">{{ d.inspect.verdict }}</div>
              <template v-if="d.inspect?.problems?.length">
                <div class="kbx-sub"><span>{{ $t('kbx.problemsTitle') }}</span></div>
                <div class="kbx-problems">
                  <div v-for="(p, i) in d.inspect.problems" :key="'p' + i" class="kbx-problem">
                    <span class="kbx-sev" :class="p.severity">{{ severityLabel(p.severity) }}</span>
                    <div class="kbx-problem-body">
                      <div class="kbx-problem-detail">{{ p.detail }}</div>
                      <div v-if="p.fix" class="kbx-problem-fix">{{ $t('kbx.fixPrefix') }}{{ p.fix }}</div>
                    </div>
                  </div>
                </div>
              </template>
            </div>
          </div>
        </div>

        <div v-if="rejectCount && !okCount" class="kbx-note-bad">
          {{ $t('kbx.noUsable') }}
        </div>
      </template>

      <!-- ===== 第 3 步：优化 ===== -->
      <template v-else-if="activeStep === 2">
        <div class="kbx-bar">
          <div class="kbx-bar-left">
            <span class="kbx-bar-title">{{ $t('kbx.stepPolish') }}</span>
            <span class="kbx-bar-sub">{{ $t('kbx.polishSub') }}</span>
          </div>
          <span v-if="!working && polishTotal.count" class="kbx-bar-tag">{{ $t('kbx.finished') }}</span>
          <el-button v-if="!working" size="small" text :icon="Plus" @click="pickFiles">{{ $t('kbx.addMore') }}</el-button>
        </div>

        <!-- 处理中：居中 + 进度条。只留一行小字干等太熬人 -->
        <div v-if="working" class="kbx-progress-box">
          <div class="kbx-progress-inner">
            <el-icon class="is-loading kbx-progress-ic"><Loading /></el-icon>
            <div class="kbx-progress-title">{{ busyText }}</div>
            <el-progress :percentage="progressPercent" :show-text="false" :stroke-width="5" />
            <div class="kbx-progress-sub">
              {{ $t('kbx.polishingHint') }}
            </div>
          </div>
        </div>

        <template v-else-if="polishTotal.count">
          <div class="kbx-metrics kbx-metrics-lg">
            <div class="kbx-metric">
              <b>{{ polishTotal.count }}</b><span>{{ $t('kbx.statDocs') }}</span>
            </div>
            <div class="kbx-metric">
              <b>{{ fmtNum(polishTotal.originalChars) }}</b><span>{{ $t('kbx.statOrigChars') }}</span>
            </div>
            <div class="kbx-metric">
              <b>{{ fmtNum(polishTotal.polishedChars) }}</b><span>{{ $t('kbx.statPolishedChars') }}</span>
            </div>
            <div class="kbx-metric">
              <b :class="polishKeepRate < 0.98 ? 'bad' : 'ok'">
                {{ Math.round(polishKeepRate * 100) }}%
              </b>
              <span>{{ $t('kbx.statFactsKept') }}</span>
            </div>
          </div>

          <div v-if="polishTotal.missingCount" class="kbx-warn">
            <el-icon><WarningFilled /></el-icon>
            <span>
              {{ $t('kbx.missingLine', { n: polishTotal.missingCount }) }}
              {{ $t('kbx.missingHint') }}
            </span>
          </div>

          <div class="kbx-list">
            <div v-for="d in docs.filter(x => x.polish)" :key="d.id" class="kbx-item" :class="{ open: d.open }">
              <div class="kbx-item-head" @click="toggle(d)">
                <el-icon class="kbx-item-ic">
                  <CircleCloseFilled v-if="d.phase === 'failed'" />
                  <CircleCheckFilled v-else />
                </el-icon>
                <span class="kbx-item-name">{{ d.source }}</span>
                <span class="kbx-badge" :class="d.phase">{{ phaseText(d) }}</span>
                <span class="kbx-item-diff">
                  {{ $t('kbx.polishChars', { a: fmtNum(d.polish.originalChars), b: fmtNum(d.polish.polishedChars) }) }}
                  <b :class="{ bad: diffPercentOf(d) < -15 }">({{ diffPercentOf(d) }}%)</b>
                </span>
                <el-button size="small" text :disabled="working" @click.stop="removeDoc(d)">{{ $t('kbx.remove') }}</el-button>
              </div>

              <div v-if="d.open" class="kbx-item-body">
                <div class="kbx-sub"><span>{{ $t('kbx.batchDetail') }}</span></div>
                <div class="kbx-batches">
                  <div v-for="b in d.polish.batches" :key="'b' + b.idx" class="kbx-batch" :class="{ bad: b.failed }">
                    {{ $t('kbx.batchLine', { i: b.idx, a: b.originalChars, b: b.polishedChars }) }}
                    <span v-if="b.failed">{{ $t('kbx.batchFailed') }}</span>
                  </div>
                </div>
                <div class="kbx-row" style="margin-top:8px">
                  <span>{{ $t('kbx.factCheck') }}</span>
                  <b>{{ $t('kbx.factsKept', { kept: d.polish.factsKept, total: d.polish.factsTotal }) }}</b>
                </div>
                <div v-if="d.polish.factsMissingCount" class="kbx-warn">
                  <el-icon><WarningFilled /></el-icon>
                  <span>{{ $t('kbx.factsMissing', { list: d.polish.factsMissing.join($t('common.listSep')) }) }}</span>
                </div>
                <div v-for="(n, i) in d.polish.notes" :key="'n' + i" class="kbx-warn">
                  <el-icon><InfoFilled /></el-icon><span>{{ n }}</span>
                </div>
                <template v-if="d.inspect?.problems?.length">
                  <div class="kbx-sub"><span>{{ $t('kbx.problemsHandled') }}</span></div>
                  <div class="kbx-problems">
                    <div v-for="(p, i) in d.inspect.problems" :key="'pp' + i" class="kbx-problem">
                      <span class="kbx-sev" :class="p.severity">{{ severityLabel(p.severity) }}</span>
                      <div class="kbx-problem-body">
                        <div class="kbx-problem-detail">{{ p.detail }}</div>
                        <div v-if="p.fix" class="kbx-problem-fix">{{ $t('kbx.fixPrefix') }}{{ p.fix }}</div>
                      </div>
                    </div>
                  </div>
                </template>
              </div>
            </div>
          </div>
        </template>
      </template>

      <!-- ===== 第 4 步：确认入库 ===== -->
      <template v-else-if="activeStep === 3">
        <div class="kbx-bar">
          <div class="kbx-bar-left">
            <span class="kbx-bar-title">{{ $t('kbx.stepConfirm') }}</span>
            <span class="kbx-bar-sub">{{ $t('kbx.confirmSub') }}</span>
          </div>
        </div>

        <div v-if="plan" class="kbx-plan">
          <div class="kbx-plan-head">
            <el-icon><MagicStick /></el-icon>
            <span>{{ $t('kbx.planTitle') }}</span>
            <span v-if="plan.aiUsed" class="kbx-ai">{{ $t('kbx.planAi') }}</span>
          </div>

          <div class="kbx-kv">
            <div class="kbx-kv-i"><span>{{ $t('kbx.planShape') }}</span><b>{{ shapeLabel }}</b></div>
            <div class="kbx-kv-i"><span>{{ $t('kbx.planChunk') }}</span><b>{{ planLabel }}</b></div>
            <div class="kbx-kv-i">
              <span>{{ $t('kbx.planIndex') }}</span>
              <b>{{ plan.buildVector ? $t('kbx.indexHigh') : $t('kbx.indexEco') }}</b>
            </div>
            <div class="kbx-kv-i"><span>{{ $t('kbx.planEstChunks') }}</span><b>{{ $t('kbx.planAboutN', { n: estChunks }) }}</b></div>
            <div class="kbx-kv-i"><span>{{ $t('kbx.statDocs') }}</span><b>{{ $t('kbx.countDocs', { n: polishTotal.count }) }}</b></div>
            <div class="kbx-kv-i"><span>{{ $t('kbx.planPolishedChars') }}</span><b>{{ fmtNum(polishTotal.polishedChars) }}</b></div>
          </div>
          <div v-if="(plan.recommend || {}).reason" class="kbx-reason">{{ plan.recommend.reason }}</div>
        </div>

        <div class="kbx-bar">
          <div class="kbx-bar-left">
            <span class="kbx-bar-title">{{ $t('kbx.whereToStore') }}</span>
            <span class="kbx-bar-sub">{{ $t('kbx.whereSub') }}</span>
          </div>
        </div>
        <div class="kbx-target">
          <div class="kbx-target-opt" :class="{ active: targetMode === 'new' }" @click="targetMode = 'new'">
            <el-icon class="kbx-target-ic"><Plus /></el-icon>
            <div class="kbx-target-body">
              <div class="kbx-target-name">{{ $t('kbx.newKb') }}</div>
              <div class="kbx-target-desc">{{ $t('kbx.newKbDesc') }}</div>
            </div>
            <el-icon v-if="targetMode === 'new'" class="kbx-target-check"><CircleCheckFilled /></el-icon>
          </div>
          <div class="kbx-target-opt"
               :class="{ active: targetMode === 'append', disabled: !kbOptions.length }"
               @click="kbOptions.length && (targetMode = 'append')">
            <el-icon class="kbx-target-ic"><FolderAdd /></el-icon>
            <div class="kbx-target-body">
              <div class="kbx-target-name">{{ $t('kbx.appendKb') }}</div>
              <div class="kbx-target-desc">
                {{ kbOptions.length
                  ? $t('kbx.appendKbDesc')
                  : $t('kbx.noKbAvailable') }}
              </div>
            </div>
            <el-icon v-if="targetMode === 'append'" class="kbx-target-check"><CircleCheckFilled /></el-icon>
          </div>
        </div>

        <template v-if="targetMode === 'append'">
          <div class="kbx-field">
            <label>{{ $t('kbx.pickKb') }}</label>
            <el-select v-model="targetKbId" :placeholder="$t('kbx.needTargetKb')" style="width:100%">
              <el-option v-for="k in kbOptions" :key="k.id" :value="k.id"
                         :label="k.name + (k.docCount != null
                           ? $t('kbx.kbOption', { docs: k.docCount, chunks: (k.chunkCount != null ? $t('kbx.kbOptionChunks', { n: k.chunkCount }) : '') })
                           : '')" />
            </el-select>
          </div>
        </template>
        <template v-else>
          <div class="kbx-grid2">
            <div class="kbx-field">
              <label>{{ $t('kbx.kbName') }}</label>
              <el-input v-model="form.name" maxlength="60" show-word-limit :placeholder="$t('kbx.kbName')" />
            </div>
            <div class="kbx-field">
              <label>{{ $t('udv.fComment') }}</label>
              <el-input v-model="form.description" maxlength="120" show-word-limit
                        :placeholder="$t('kbx.kbDescPlaceholder')" />
            </div>
          </div>
        </template>
      </template>

      <!-- ===== 第 5 步：完成 ===== -->
      <template v-else>
        <div class="kbx-done-hero">
          <el-icon class="kbx-done-ic"><CircleCheckFilled /></el-icon>
          <div class="kbx-done-tx">
            <div class="kbx-done-title">{{ $t('kbx.ingested', { name: (kbCreated?.name || form.name) }) }}</div>
            <div class="kbx-done-sub">
              {{ $t('kbx.doneSummary', { docs: doneCount, chunks: totalChunks }) }}
              {{ targetMode === 'append' ? $t('kbx.appendKb') : $t('kbx.newKb') }}
            </div>
          </div>
        </div>

        <div class="kbx-kv">
          <div class="kbx-kv-i"><span>{{ $t('kbx.kbDesc') }}</span><b>{{ kbCreated?.description || form.description || $t('kbx.notFilled') }}</b></div>
          <div class="kbx-kv-i"><span>{{ $t('kbx.planChunk') }}</span><b>{{ planLabel }}</b></div>
          <div class="kbx-kv-i">
            <span>{{ $t('kbx.planIndex') }}</span>
            <b>{{ plan?.buildVector ? $t('kbx.indexHigh') : $t('kbx.indexEco') }}</b>
          </div>
          <div class="kbx-kv-i"><span>{{ $t('kbx.doneAt') }}</span><b>{{ fmtDateTime(new Date()) }}</b></div>
        </div>

        <div class="kbx-sub"><span>{{ $t('kbx.eachDoc') }}</span></div>
        <div class="kbx-list">
          <div v-for="d in docs" :key="d.id" class="kbx-item" :class="[d.phase, { open: d.open }]">
            <div class="kbx-item-head" @click="toggle(d)">
              <el-icon class="kbx-item-ic">
                <CircleCloseFilled v-if="d.phase === 'failed' || d.phase === 'rejected'" />
                <CircleCheckFilled v-else />
              </el-icon>
              <span class="kbx-item-name">{{ d.source }}</span>
              <span class="kbx-item-size">{{ $t('kbx.chars', { n: fmtNum(d.text.length) }) }}</span>
              <span class="kbx-badge" :class="d.phase">{{ phaseText(d) }}</span>
              <span v-if="d.phase === 'done'" class="kbx-item-diff ok-text">{{ $t('kbx.chunksN', { n: d.chunks }) }}</span>
            </div>
            <div v-if="d.error || d.inspect?.rejectReason" class="kbx-item-sum">
              <span class="bad-text">{{ d.error || d.inspect.rejectReason }}</span>
            </div>
          </div>
        </div>
      </template>
    </div>

    <!-- ===== 底部动作条：上一步 / 主操作 ===== -->
    <div class="kbx-foot">
      <div class="kbx-foot-left">
        <!-- 处理中不在底部重复同一句话：内容区已有居中的进度提示，重复一遍反而干扰 -->
        <template v-if="activeStep === 1 && rejectCount">
          <span class="kbx-tip">{{ $t('kbx.rejectTip', { n: rejectCount }) }}</span>
        </template>
        <template v-else-if="activeStep === 2 && !working">
          <span class="kbx-tip">{{ $t('kbx.polishTip') }}</span>
        </template>
        <template v-else-if="activeStep === 3 && targetMode === 'new'">
          <span class="kbx-tip">{{ $t('kbx.paramTip') }}</span>
        </template>
        <template v-else-if="activeStep === 3">
          <span class="kbx-tip">{{ $t('kbx.appendTip') }}</span>
        </template>
        <template v-else-if="activeStep === 4">
          <span class="kbx-tip">
            {{ $t('kbx.gotoPre') }}
            <a class="kbx-link" @click="emit('open-knowledge')">{{ $t('kbx.kbTitle') }}</a>
            {{ $t('kbx.gotoPost') }}
          </span>
        </template>
      </div>

      <div class="kbx-foot-right">
        <el-button v-if="docs.length && !working && activeStep !== 4" text :icon="RefreshLeft" @click="resetAll">
          {{ $t('kbx.reset') }}
        </el-button>
        <!-- 最后一屏没有别的可做，把主操作直接给它，不必再让用户去别处找入口 -->
        <el-button v-if="activeStep === 4 && !working" type="primary" :icon="Plus" @click="resetAll">
          {{ $t('kbx.again') }}
        </el-button>
        <el-button v-if="activeStep === 1 && okCount && !working" type="primary" @click="startProcess">
          <el-icon><MagicStick /></el-icon>{{ $t('kbx.polishAndPlan', { n: okCount }) }}
        </el-button>
        <el-button v-else-if="activeStep === 3" type="primary" @click="startCreate">
          {{ targetMode === 'append'
            ? $t('kbx.appendAndImport', { name: (targetKb?.name || $t('kbx.selectedKb')), n: plannedCount })
            : $t('kbx.createAndImport', { n: plannedCount }) }}
        </el-button>
      </div>
    </div>

    <input ref="fileInput" type="file" multiple style="display:none" @change="onFiles" :accept="ACCEPT" />
  </div>
</template>

<style scoped>
/* 整体限宽居中，与空态欢迎页（.welcome-panel）保持同一个视觉宽度：
   铺满整个面板时，拖入区那种大色块会显得又宽又空 */
.kbx {
  display: flex; flex-direction: column; gap: 14px;
  width: 100%; max-width: 860px; margin: 0 auto;
  min-height: 100%; padding: 4px 2px 2px;
}

/* 步骤条、内容区、底部条都跟着这个宽度走：三者同宽才显得是「一套」 */
.kbx-steps, .kbx-stage, .kbx-foot { width: 100%; }

/* ---- 步骤条：圆点固定宽 + 连接线拉伸，圆点均匀铺满整宽并两端贴边 ---- */
.kbx-steps {
  display: flex; align-items: flex-start;
  padding: 4px 0 2px;
}
.kbx-step {
  display: flex; flex-direction: column; align-items: center; gap: 7px;
  flex: 0 0 auto;              /* 自身不拉伸，宽度由圆点决定 */
}
.kbx-step-dot {
  width: 26px; height: 26px; border-radius: 50%;
  display: inline-flex; align-items: center; justify-content: center;
  font-size: 13px; font-variant-numeric: tabular-nums;
  background: var(--dc-bg-hover); color: var(--dc-text-mid);
  border: 1px solid var(--dc-border);
  transition: background .2s ease, color .2s ease, border-color .2s ease;
}
.kbx-step-dot .el-icon { font-size: 14px; }
.kbx-step-label {
  font-size: 13px; color: var(--dc-text-dim); white-space: nowrap;
}
.kbx-step.is-process .kbx-step-dot {
  background: var(--dc-primary); color: #fff; border-color: var(--dc-primary);
}
.kbx-step.is-process .kbx-step-label { color: var(--dc-primary); font-weight: 600; }
.kbx-step.is-done .kbx-step-dot {
  background: #16a34a; color: #fff; border-color: #16a34a;
}
.kbx-step.is-done .kbx-step-label { color: #16a34a; }
/* 连接线：吃掉剩余宽度，把圆点均匀推开；顶部对齐到圆点圆心 */
.kbx-step-line {
  flex: 1 1 auto; height: 1px; background: var(--dc-border);
  margin: 13px 10px 0; border-radius: 1px;
}
.kbx-step-line.done { background: #16a34a; }

/* 每一步的内容区：撑开剩余高度，内容多时自己滚动 */
.kbx-stage { flex: 1; min-height: 0; }

/* ---- 第 1 步：拖入区 ---- */
.kbx-drop {
  display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px;
  min-height: 220px; padding: 34px 24px; text-align: center; cursor: pointer;
  border: 1px dashed var(--dc-border-strong); border-radius: 14px; background: var(--dc-bg-soft);
  transition: border-color .15s ease, background .15s ease;
}
.kbx-drop:hover, .kbx-drop.over { border-color: var(--dc-primary); background: var(--dc-primary-wash); }
.kbx-drop.busy { cursor: progress; }
.kbx-drop-ic {
  width: 52px; height: 52px; border-radius: 50%; font-size: 24px;
  display: inline-flex; align-items: center; justify-content: center;
  background: var(--dc-primary-wash); color: var(--dc-primary);
}
.kbx-drop-title { font-size: 14px; color: var(--dc-text); }
/* 支持的类型：单独一行，让用户一眼看到，而不是埋在小字说明里 */
.kbx-drop-types {
  display: flex; align-items: baseline; gap: 8px; flex-wrap: wrap; justify-content: center;
  max-width: 660px;
}
.kbx-types-label {
  font-size: 10.5px; color: var(--dc-primary); background: var(--dc-primary-wash);
  border-radius: 9px; padding: 1px 8px; flex-shrink: 0;
}
.kbx-types {
  font-size: 12px; color: var(--dc-text-mid); line-height: 1.7;
  font-family: ui-monospace, Menlo, Consolas, monospace;
}
.kbx-drop-desc { font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.8; max-width: 560px; }
.kbx-drop-desc strong { color: var(--dc-text-mid); }

/* ---- 分区标题条：做成一条浅底卡片，和步骤条、内容都拉开层次 ---- */
.kbx-bar {
  display: flex; align-items: center; gap: 10px;
  /* 上面留出区块间距，下面留出与内容的间距；作为首元素时不留上边距，免得顶出一块空白 */
  margin: 20px 0 14px; padding: 11px 14px;
  border-radius: 10px; background: var(--dc-bg-soft); border: 1px solid var(--dc-border);
}
.kbx-bar:first-child { margin-top: 0; }
.kbx-bar-left { min-width: 0; flex: 1; }
.kbx-bar-title { font-size: 13.5px; font-weight: 600; color: var(--dc-text-strong); margin-right: 8px; }
.kbx-bar-sub { font-size: 11.5px; color: var(--dc-text-dim); }
.kbx-bar-sub .ok { color: #16a34a; font-weight: 600; }
.kbx-bar-sub .bad { color: #dc2626; font-weight: 600; }
.kbx-bar-tag {
  font-size: 10.5px; padding: 1px 8px; border-radius: 9px;
  background: rgba(22, 163, 74, .14); color: #16a34a; flex-shrink: 0;
}

/* ---- 队列项 ---- */
.kbx-list { display: flex; flex-direction: column; gap: 8px; }
.kbx-item {
  border: 1px solid var(--dc-border); border-radius: 10px;
  background: var(--dc-bg-card); padding: 10px 12px;
}
.kbx-item.open { border-color: var(--dc-border-strong); }
.kbx-item.rejected, .kbx-item.failed { border-color: rgba(239, 68, 68, .35); background: rgba(239, 68, 68, .04); }
.kbx-item.done { border-color: rgba(22, 163, 74, .35); }
.kbx-item-head { display: flex; align-items: center; gap: 8px; cursor: pointer; }
.kbx-item-ic { color: var(--dc-text-dim); flex-shrink: 0; }
.kbx-item.done .kbx-item-ic { color: #16a34a; }
.kbx-item.rejected .kbx-item-ic, .kbx-item.failed .kbx-item-ic { color: #dc2626; }
.kbx-item-name {
  font-size: 13px; font-weight: 600; color: var(--dc-text-strong);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 40%;
}
.kbx-item-size { font-size: 12px; color: var(--dc-text-dim); font-variant-numeric: tabular-nums; }
.kbx-item-diff {
  font-size: 11.5px; color: var(--dc-text-mid);
  font-variant-numeric: tabular-nums; white-space: nowrap;
}
.kbx-item-diff b { font-weight: 600; color: var(--dc-text-strong); }
.kbx-item-diff b.bad { color: #dc2626; }
.ok-text { color: #16a34a; font-weight: 600; }
.kbx-badge {
  font-size: 10.5px; border-radius: 9px; padding: 1px 8px;
  background: var(--dc-bg-hover); color: var(--dc-text-mid); flex-shrink: 0;
}
.kbx-badge.ready, .kbx-badge.planned { background: var(--dc-primary-wash); color: var(--dc-primary); }
.kbx-badge.done { background: rgba(22, 163, 74, .14); color: #16a34a; }
.kbx-badge.rejected, .kbx-badge.failed { background: rgba(239, 68, 68, .14); color: #dc2626; }
/* 行内最后一个按钮（移除）永远靠右：中间的大小/状态/字数信息自然排在左侧 */
.kbx-item-head > .el-button:last-child { margin-left: auto; flex-shrink: 0; }
.kbx-item-sum { margin-top: 5px; padding-left: 22px; font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.7; }
.kbx-item-sum .dim { color: var(--dc-text-weak, var(--dc-text-dim)); }
.bad-text { color: #dc2626; }
.kbx-item-body { margin-top: 10px; padding-top: 10px; border-top: 1px dashed var(--dc-border); }

/* ---- 小标题 ---- */
.kbx-sub {
  display: flex; align-items: center; gap: 8px;
  margin: 14px 0 8px; font-size: 13px; font-weight: 600; color: var(--dc-text-mid);
}
.kbx-item-body > .kbx-sub:first-child { margin-top: 0; }
.kbx-sub::after { content: ''; flex: 1; height: 1px; background: var(--dc-border); }

/* ---- 指标 ---- */
.kbx-metrics { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; }
.kbx-metrics-lg { margin-bottom: 12px; }
.kbx-metric {
  display: flex; flex-direction: column; gap: 2px; padding: 9px 11px;
  border-radius: 9px; background: var(--dc-bg-soft); border: 1px solid var(--dc-border);
}
.kbx-metrics-lg .kbx-metric { padding: 12px 14px; }
.kbx-metric b { font-size: 15px; color: var(--dc-text-strong); font-variant-numeric: tabular-nums; }
.kbx-metrics-lg .kbx-metric b { font-size: 20px; letter-spacing: -.01em; }
.kbx-metric b.bad { color: #dc2626; }
.kbx-metric b.ok { color: #16a34a; }
.kbx-metric span { font-size: 10.5px; color: var(--dc-text-dim); }
.kbx-metrics-lg .kbx-metric span { font-size: 12px; }

/* ---- 键值对（比逐行 row 更紧凑、更整齐） ---- */
.kbx-kv {
  display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px;
  margin-bottom: 10px;
}
/* 固定最小高度 + 垂直居中：否则一栏两行、一栏一行，格子高矮不齐很显乱 */
.kbx-kv-i {
  display: flex; align-items: center; gap: 8px; padding: 8px 11px;
  min-height: 40px;
  border-radius: 9px; background: var(--dc-bg-soft); border: 1px solid var(--dc-border);
  min-width: 0;
}
.kbx-kv-i > span { font-size: 11.5px; color: var(--dc-text-dim); flex-shrink: 0; }
.kbx-kv-i > b {
  font-size: 13px; color: var(--dc-text-strong); font-weight: 600;
  line-height: 1.6; min-width: 0; word-break: break-word;
}

/* ---- 问题清单 ---- */
.kbx-problems { display: flex; flex-direction: column; gap: 6px; }
.kbx-problem {
  display: flex; gap: 8px; padding: 7px 10px; border-radius: 8px;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border);
}
.kbx-sev {
  flex: 0 0 auto; align-self: flex-start; font-size: 10.5px; padding: 1px 7px; border-radius: 9px;
  background: var(--dc-bg-hover); color: var(--dc-text-mid);
}
.kbx-sev.high { background: rgba(239, 68, 68, .14); color: #dc2626; }
.kbx-sev.mid { background: rgba(245, 158, 11, .16); color: #d97706; }
.kbx-problem-body { min-width: 0; }
.kbx-problem-detail { font-size: 13px; color: var(--dc-text); line-height: 1.7; }
.kbx-problem-fix { margin-top: 2px; font-size: 12px; color: var(--dc-text-dim); line-height: 1.7; }

/* ---- 提示条 ---- */
.kbx-warn {
  display: flex; align-items: flex-start; gap: 6px; margin-top: 8px;
  padding: 8px 10px; border-radius: 8px; font-size: 11.5px; line-height: 1.8;
  color: var(--dc-text-mid); background: rgba(245, 158, 11, .1);
  border: 1px solid rgba(245, 158, 11, .3);
}
.kbx-warn .el-icon { color: #d97706; margin-top: 2px; flex-shrink: 0; }
.kbx-verdict { margin-top: 8px; font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.8; }
.kbx-note-bad {
  margin-top: 12px; padding: 10px 12px; border-radius: 9px;
  font-size: 11.5px; line-height: 1.8; color: var(--dc-text-mid);
  background: rgba(239, 68, 68, .07); border: 1px solid rgba(239, 68, 68, .28);
}

/* ---- 处理中：整块居中，配进度条，别只留一行小字干等 ---- */
.kbx-progress-box {
  display: flex; align-items: center; justify-content: center;
  min-height: 320px; padding: 40px 24px;
}
.kbx-progress-inner {
  display: flex; flex-direction: column; align-items: center; gap: 14px;
  width: 100%; max-width: 380px; text-align: center;
}
.kbx-progress-ic { font-size: 30px; color: var(--dc-primary); }
.kbx-progress-title { font-size: 14px; color: var(--dc-text); }
.kbx-progress-sub { font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.7; }
.kbx-progress-inner :deep(.el-progress) { width: 100%; }
.kbx-progress-inner :deep(.el-progress-bar__outer) { border-radius: 4px; }

/* ---- 分批明细 ---- */
.kbx-batches { display: flex; flex-wrap: wrap; gap: 6px; }
.kbx-batch {
  font-size: 12px; color: var(--dc-text-dim); padding: 2px 8px; border-radius: 9px;
  background: var(--dc-bg-hover); font-variant-numeric: tabular-nums;
}
.kbx-batch.bad { background: rgba(245, 158, 11, .16); color: #d97706; }

/* ---- 方案卡 ---- */
.kbx-plan {
  padding: 13px 15px; border-radius: 12px;
  background: var(--dc-primary-wash); border: 1px solid var(--dc-border);
}
.kbx-plan-head {
  display: flex; align-items: center; gap: 6px; margin-bottom: 10px;
  font-size: 14px; font-weight: 600; color: var(--dc-text-strong);
}
.kbx-plan-head .el-icon { color: var(--dc-primary); }
.kbx-ai {
  margin-left: auto; font-size: 10.5px; font-weight: 400;
  color: var(--dc-primary); background: var(--dc-bg-card);
  border-radius: 9px; padding: 1px 8px;
}
.kbx-plan .kbx-kv-i { background: var(--dc-bg-card); }
.kbx-reason {
  margin-top: 4px; padding-top: 10px; border-top: 1px dashed var(--dc-border-strong);
  font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.8;
}

/* ---- 存到哪里 ---- */
/* 底边留白：下面的「知识库名称 / 说明」是这一组的下级表单，
   紧贴着上方的目标卡会看成同一坨 */
.kbx-target { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; margin-bottom: 16px; }
.kbx-target-opt {
  display: flex; align-items: flex-start; gap: 9px; padding: 11px 12px; cursor: pointer;
  border: 1px solid var(--dc-border); border-radius: 10px; background: var(--dc-bg-card);
  transition: border-color .15s ease, background .15s ease;
}
.kbx-target-opt:hover { border-color: var(--dc-border-strong); }
.kbx-target-opt.active { border-color: var(--dc-primary); }
.kbx-target-opt.disabled { opacity: .55; cursor: not-allowed; }
.kbx-target-ic { color: var(--dc-text-mid); margin-top: 1px; flex-shrink: 0; }
.kbx-target-opt.active .kbx-target-ic { color: var(--dc-primary); }
.kbx-target-body { min-width: 0; flex: 1; }
.kbx-target-name { font-size: 13px; font-weight: 600; color: var(--dc-text-strong); }
.kbx-target-desc { margin-top: 3px; font-size: 12px; color: var(--dc-text-dim); line-height: 1.7; }
.kbx-target-check { color: var(--dc-primary); flex-shrink: 0; margin-top: 2px; }

/* ---- 表单 ---- */
.kbx-grid2 { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; align-items: start; }
/* 两列并排时用「固定两行」的网格：label 恒 18px、控件恒 34px。
   靠 flex 自适应时，字数统计的有无会让两列的 label 与输入框错开几个像素。 */
.kbx-grid2 .kbx-field {
  display: grid; grid-template-rows: 18px 34px; gap: 6px;
  margin-top: 0;   /* 必须压过下面 .kbx-field 的 margin-top，否则第二列会被整体推下去 */
}
.kbx-grid2 .kbx-field > label { height: 18px; line-height: 18px; }
.kbx-grid2 .kbx-field :deep(.el-input),
.kbx-grid2 .kbx-field :deep(.el-input__wrapper) { height: 34px; }

.kbx-field { display: flex; flex-direction: column; gap: 6px; margin-top: 10px; }
.kbx-field > label {
  font-size: 13px; color: var(--dc-text-mid);
  height: 18px; line-height: 18px;
}
.kbx :deep(.el-input__wrapper) {
  border-radius: 8px; background: var(--dc-bg-soft); box-shadow: none;
  min-height: 34px;
}
.kbx :deep(.el-input__wrapper.is-focus) { box-shadow: 0 0 0 1px var(--dc-primary) inset; }

/* ---- 第 5 步：完成 ---- */
.kbx-done-hero {
  display: flex; align-items: center; gap: 12px; padding: 16px 18px; margin-bottom: 12px;
  border-radius: 12px; background: rgba(22, 163, 74, .08); border: 1px solid rgba(22, 163, 74, .3);
}
.kbx-done-ic { font-size: 28px; color: #16a34a; flex-shrink: 0; }
.kbx-done-title { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); }
.kbx-done-sub { margin-top: 3px; font-size: 11.5px; color: var(--dc-text-dim); }

/* ---- 底部动作条 ---- */
.kbx-foot {
  display: flex; align-items: center; gap: 12px; flex-wrap: wrap;
  margin-top: auto; padding-top: 12px; border-top: 1px solid var(--dc-border);
}
.kbx-foot-left { flex: 1; min-width: 0; }
.kbx-foot-right { display: flex; align-items: center; gap: 8px; }
.kbx-busy { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--dc-primary); }
.kbx-tip { font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.7; }
/* 提示语里的跳转链接：跟正文同样的字号，只靠颜色和下划线标识可点 */
.kbx-link {
  color: var(--dc-link); cursor: pointer; text-decoration: none;
  border-bottom: 1px dashed currentColor;
}
.kbx-link:hover { color: var(--dc-primary); }
</style>
