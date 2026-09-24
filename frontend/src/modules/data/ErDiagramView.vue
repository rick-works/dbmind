<template>
  <div class="er-view">
    <div class="er-toolbar">
      <span class="er-title"><el-icon><Connection /></el-icon>{{ $t('er.title', { db: database }) }}</span>
      <el-input v-model="keyword" size="small" clearable :placeholder="$t('er.searchPlaceholder')" :prefix-icon="Search" style="width:180px" />
      <span class="er-spacer" />
      <el-button-group>
        <el-button size="small" :icon="ZoomOut" @click="zoomBy(0.9)" :title="$t('er.zoomOut')" />
        <el-button size="small" :icon="ZoomIn" @click="zoomBy(1.1)" :title="$t('er.zoomIn')" />
        <el-button size="small" :icon="FullScreen" @click="fit" :title="$t('er.fit')" />
        <el-button size="small" :icon="Refresh" @click="load" :title="$t('er.reload')" />
      </el-button-group>
      <el-button size="small" :icon="Download" @click="exportSvg">SVG</el-button>
      <el-button size="small" :icon="Download" @click="exportPng">PNG</el-button>
    </div>
    <div v-if="note" class="er-note"><el-icon><InfoFilled /></el-icon>{{ note }}</div>
    <div ref="canvasWrapRef" class="er-canvas" @wheel.prevent="onWheel" @mousedown="onBgDown">
      <svg ref="svgRef" class="er-svg">
        <defs>
          <marker id="er-arrow" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto" markerUnits="strokeWidth">
            <path d="M0,0 L8,3 L0,6 Z" :fill="colors.link" />
          </marker>
          <marker id="er-arrow-hi" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto" markerUnits="strokeWidth">
            <path d="M0,0 L8,3 L0,6 Z" :fill="colors.linkHi" />
          </marker>
        </defs>
        <g :transform="`translate(${vp.tx},${vp.ty}) scale(${vp.scale})`">
          <g class="er-links">
            <path v-for="(l, i) in linkPaths" :key="'l' + i"
                  :d="l.d" fill="none"
                  :stroke="l.hi ? colors.linkHi : colors.link"
                  :stroke-width="l.hi ? 2 : 1.2"
                  :marker-end="l.hi ? 'url(#er-arrow-hi)' : 'url(#er-arrow)'"
                  :opacity="linkOpacity(l)" />
          </g>
          <g v-for="n in nodes" :key="n.name" class="er-node" :transform="`translate(${n.x},${n.y})`" :opacity="nodeOpacity(n)">
            <rect :width="n.w" :height="n.h" rx="6" :fill="colors.card" :stroke="isHi(n) ? colors.linkHi : colors.border" :stroke-width="isHi(n) ? 2 : 1" />
            <rect :width="n.w" :height="HEADER_H" rx="6" :fill="colors.headerBg" />
            <rect :y="HEADER_H / 2" :width="n.w" :height="HEADER_H / 2" :fill="colors.headerBg" />
            <text :x="n.w / 2" :y="HEADER_H / 2" text-anchor="middle" dominant-baseline="middle" :fill="colors.headerText" font-size="13" font-weight="600">{{ n.name }}</text>
            <g v-for="(col, ci) in n.columns" :key="ci">
              <rect v-if="col.pk" :x="0" :y="HEADER_H + ci * ROW_H" :width="n.w" :height="ROW_H" :fill="colors.pkRow" />
              <text :x="10" :y="HEADER_H + ci * ROW_H + ROW_H / 2" dominant-baseline="middle" :fill="colors.text" font-size="10">{{ col.name }}</text>
              <text :x="n.w - 10" :y="HEADER_H + ci * ROW_H + ROW_H / 2" text-anchor="end" dominant-baseline="middle" :fill="colors.sub" font-size="10">{{ col.type }}</text>
              <text v-if="col.pk" :x="n.w - 64" :y="HEADER_H + ci * ROW_H + ROW_H / 2" text-anchor="end" dominant-baseline="middle" :fill="colors.pk" font-size="10" font-weight="600">PK</text>
              <text v-else-if="col.fk" :x="n.w - 64" :y="HEADER_H + ci * ROW_H + ROW_H / 2" text-anchor="end" dominant-baseline="middle" :fill="colors.fk" font-size="10" font-weight="600">FK</text>
            </g>
            <rect :width="n.w" :height="n.h" fill="transparent" style="cursor:move" @mousedown.stop="onNodeDown($event, n)" />
          </g>
        </g>
      </svg>
      <div v-if="loading" class="er-overlay"><el-icon class="is-loading" :size="26"><Loading /></el-icon><span>{{ $t('common.loading') }}</span></div>
      <div v-if="error" class="er-overlay er-error">
        <el-icon><WarningFilled /></el-icon>
        <span>{{ error }}</span>
        <el-button size="small" @click="load">{{ $t('er.retry') }}</el-button>
      </div>
      <div v-if="!loading && !error && nodes.length === 0" class="er-overlay">
        <el-empty :description="$t('er.emptyRelations')" />
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, computed, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { saveBlobAs } from '../../utils/useExportTask'
import { ElMessage } from 'element-plus'
import { ZoomIn, ZoomOut, FullScreen, Refresh, Download, Search, Connection, Loading, WarningFilled, InfoFilled } from '@element-plus/icons-vue'
import dagre from 'dagre'
import { getErGraph } from '../../api'
import { t } from '../../utils/i18n'
import { getResolvedTheme, onResolvedThemeChange } from '../../utils/theme'

const props = defineProps({
  conn: { type: Object, default: null },
  database: { type: String, default: '' },
  connId: { type: String, default: '' }
})

const HEADER_H = 30
const ROW_H = 22
const PAD = 8
const CARD_W_MIN = 200
const CARD_W_MAX = 320

// 导出固定使用亮色，确保 PNG/SVG 在白底/演示场景下始终可读
const exportColors = {
  card: '#ffffff', border: '#dcdfe6', headerBg: '#409eff', headerText: '#ffffff',
  text: '#303133', sub: '#909399', pk: '#e6a23c', fk: '#67c23a',
  pkRow: '#fdf6ec', link: '#b0b3b8', linkHi: '#409eff'
}

const lightColors = exportColors

const darkColors = {
  card: 'var(--dc-bg-card)', border: 'var(--dc-border-strong)', headerBg: 'var(--dc-primary)', headerText: '#ffffff',
  text: 'var(--dc-text)', sub: 'var(--dc-text-dim)', pk: 'var(--dc-warning)', fk: 'var(--dc-success)',
  pkRow: 'var(--dc-warning-wash)', link: 'var(--dc-text-dim)', linkHi: 'var(--dc-primary)'
}

const theme = ref(getResolvedTheme())
let unsubscribeTheme = null
const colors = computed(() => theme.value === 'dark' ? darkColors : lightColors)

const nodes = ref([])          // [{ name, columns:[{name,type,pk,fk}], x, y, w, h }]
const relations = ref([])      // 原始关系 [{fromTable,fromColumn,toTable,toColumn,...}]
const fkSet = ref({})          // 'table.col' -> true
const vp = reactive({ tx: 40, ty: 40, scale: 1 })
const keyword = ref('')
const loading = ref(false)
const error = ref('')
const note = ref('')
const canvasWrapRef = ref(null)
const svgRef = ref(null)

const colIndex = (n, name) => n.columns.findIndex(c => c.name === name)

const nodeOpacity = (n) => {
  if (!keyword.value) return 1
  const k = keyword.value.toLowerCase()
  if (n.name.toLowerCase().includes(k)) return 1
  if (n.columns.some(c => c.name.toLowerCase().includes(k))) return 1
  return 0.15
}
const isHi = (n) => keyword.value && nodeOpacity(n) === 1
const linkOpacity = (l) => {
  if (!keyword.value) return 1
  const k = keyword.value.toLowerCase()
  return (l.fromT.toLowerCase().includes(k) || l.toT.toLowerCase().includes(k)) ? 1 : 0.1
}

const linkPaths = computed(() => {
  const map = {}
  nodes.value.forEach(n => { map[n.name] = n })
  return relations.value.map(r => {
    const f = map[r.fromTable], t = map[r.toTable]
    if (!f || !t) return null
    const fi = Math.max(0, colIndex(f, r.fromColumn))
    const ti = Math.max(0, colIndex(t, r.toColumn))
    const fy = f.y + HEADER_H + fi * ROW_H + ROW_H / 2
    const ty = t.y + HEADER_H + ti * ROW_H + ROW_H / 2
    const fcx = f.x + f.w / 2, tcx = t.x + t.w / 2
    let fx, tx, d
    if (tcx >= fcx) { fx = f.x + f.w; tx = t.x } else { fx = f.x; tx = t.x + t.w }
    const off = Math.max(30, Math.abs(tx - fx) * 0.4)
    d = tcx >= fcx
      ? `M ${fx} ${fy} C ${fx + off} ${fy}, ${tx - off} ${ty}, ${tx} ${ty}`
      : `M ${fx} ${fy} C ${fx - off} ${fy}, ${tx + off} ${ty}, ${tx} ${ty}`
    return { d, hi: isHi(f) || isHi(t), fromT: r.fromTable, toT: r.toTable }
  }).filter(Boolean)
})

const estWidth = (name, type) => {
  const nameW = [...String(name)].reduce((s, ch) => s + (ch.charCodeAt(0) > 255 ? 7.5 : 4.5), 0)
  const typeW = [...String(type || '')].reduce((s, ch) => s + (ch.charCodeAt(0) > 255 ? 7.5 : 4.5), 0)
  return Math.min(CARD_W_MAX, Math.max(CARD_W_MIN, nameW + typeW + 64))
}
const tableH = (cols) => HEADER_H + cols.length * ROW_H + PAD

const bbox = () => {
  if (!nodes.value.length) return { minX: 0, minY: 0, w: 0, h: 0 }
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity
  nodes.value.forEach(n => {
    minX = Math.min(minX, n.x); minY = Math.min(minY, n.y)
    maxX = Math.max(maxX, n.x + n.w); maxY = Math.max(maxY, n.y + n.h)
  })
  return { minX, minY, maxX, maxY, w: maxX - minX, h: maxY - minY }
}

const fit = () => {
  const wrap = canvasWrapRef.value
  if (!wrap || !nodes.value.length) return
  const b = bbox()
  const vw = wrap.clientWidth, vh = wrap.clientHeight
  const s = Math.min(vw / (b.w + 80), vh / (b.h + 80), 3)
  vp.scale = s > 0 ? s : 1
  vp.tx = (vw - b.w * vp.scale) / 2 - b.minX * vp.scale
  vp.ty = (vh - b.h * vp.scale) / 2 - b.minY * vp.scale
}

const zoomBy = (factor) => {
  const wrap = canvasWrapRef.value
  const vw = wrap ? wrap.clientWidth / 2 : 400
  const vh = wrap ? wrap.clientHeight / 2 : 300
  const ns = Math.min(3, Math.max(0.1, vp.scale * factor))
  vp.tx = vw - (vw - vp.tx) * (ns / vp.scale)
  vp.ty = vh - (vh - vp.ty) * (ns / vp.scale)
  vp.scale = ns
}

const onWheel = (e) => {
  const wrap = canvasWrapRef.value
  if (!wrap) return
  const rect = wrap.getBoundingClientRect()
  const mx = e.clientX - rect.left, my = e.clientY - rect.top
  const factor = e.deltaY < 0 ? 1.1 : 0.9
  const ns = Math.min(3, Math.max(0.1, vp.scale * factor))
  vp.tx = mx - (mx - vp.tx) * (ns / vp.scale)
  vp.ty = my - (my - vp.ty) * (ns / vp.scale)
  vp.scale = ns
}

// 拖拽监听兜底：组件在拖拽中途被卸载（切走页签）时也要把 window 监听摘掉，
// 否则闭包一直持有组件与数据（只在 mouseup 里移除是清不掉的）
let erDragCleanup = null
const bindWindowDrag = (move, up) => {
  window.addEventListener('mousemove', move)
  window.addEventListener('mouseup', up)
  erDragCleanup = () => {
    window.removeEventListener('mousemove', move)
    window.removeEventListener('mouseup', up)
    erDragCleanup = null
  }
}
onBeforeUnmount(() => { if (erDragCleanup) erDragCleanup() })

const onBgDown = (e) => {
  const startX = e.clientX, startY = e.clientY, otx = vp.tx, oty = vp.ty
  const move = (ev) => { vp.tx = otx + (ev.clientX - startX); vp.ty = oty + (ev.clientY - startY) }
  const up = () => { if (erDragCleanup) erDragCleanup() }
  bindWindowDrag(move, up)
}

const onNodeDown = (e, n) => {
  const startX = e.clientX, startY = e.clientY, ox = n.x, oy = n.y
  const move = (ev) => { n.x = ox + (ev.clientX - startX) / vp.scale; n.y = oy + (ev.clientY - startY) / vp.scale }
  const up = () => { if (erDragCleanup) erDragCleanup() }
  bindWindowDrag(move, up)
}

const layout = (data) => {
  const tables = data.tables || []
  const relations = (data.relations || []).filter(r => tables.some(t => t.name === r.fromTable) && tables.some(t => t.name === r.toTable))

  // 分离有关系和孤立的表
  const connected = new Set()
  relations.forEach(r => { connected.add(r.fromTable); connected.add(r.toTable) })
  const connectedTables = tables.filter(t => connected.has(t.name))
  const isolatedTables = tables.filter(t => !connected.has(t.name))

  // dagre 仅布局有关系的表：LR 方向让有关联的表横向排列
  const g = new dagre.graphlib.Graph()
  g.setGraph({ rankdir: 'LR', nodesep: 30, ranksep: 70, marginx: 20, marginy: 20 })
  g.setDefaultEdgeLabel(() => ({}))
  connectedTables.forEach(t => {
    const w = estWidth(t.name, (t.columns[0] && t.columns[0].type) || '')
    g.setNode(t.name, { width: w, height: tableH(t.columns) })
  })
  relations.forEach(r => {
    if (g.hasNode(r.fromTable) && g.hasNode(r.toTable)) g.setEdge(r.fromTable, r.toTable)
  })
  if (connectedTables.length) dagre.layout(g)

  const posMap = {}
  connectedTables.forEach(t => {
    const gn = g.node(t.name)
    const w = estWidth(t.name, (t.columns[0] && t.columns[0].type) || '')
    const h = tableH(t.columns)
    posMap[t.name] = { x: gn.x - w / 2, y: gn.y - h / 2, w, h }
  })

  // 孤立表按网格铺满，列数按画布宽高比自适应
  if (isolatedTables.length) {
    const wrap = canvasWrapRef.value
    const canvasW = wrap ? wrap.clientWidth : 800
    const canvasH = wrap ? wrap.clientHeight : 600
    const canvasAspect = canvasW / canvasH
    const SPACING = 40

    const isoNodes = isolatedTables.map(t => {
      const w = estWidth(t.name, (t.columns[0] && t.columns[0].type) || '')
      const h = tableH(t.columns)
      return { name: t.name, w, h }
    })
    const maxIsoW = Math.max(...isoNodes.map(n => n.w), 1)
    const maxIsoH = Math.max(...isoNodes.map(n => n.h), 1)
    const cellW = maxIsoW + SPACING
    const cellH = maxIsoH + SPACING
    const n = isoNodes.length
    const cols = Math.max(1, Math.min(n, Math.round(Math.sqrt(n * canvasAspect * cellH / cellW))))
    const gridW = cols * cellW
    const gridH = Math.ceil(n / cols) * cellH

    let startX = 0, startY = 0
    if (connectedTables.length) {
      let connMinX = Infinity, connMinY = Infinity, connMaxX = -Infinity, connMaxY = -Infinity
      connectedTables.forEach(t => {
        const p = posMap[t.name]
        connMinX = Math.min(connMinX, p.x); connMinY = Math.min(connMinY, p.y)
        connMaxX = Math.max(connMaxX, p.x + p.w); connMaxY = Math.max(connMaxY, p.y + p.h)
      })
      const connW = connMaxX - connMinX
      const connH = connMaxY - connMinY
      const rightW = connW + SPACING + gridW
      const rightH = Math.max(connH, gridH)
      const belowW = Math.max(connW, gridW)
      const belowH = connH + SPACING + gridH
      const rightDiff = Math.abs(rightW / rightH - canvasAspect)
      const belowDiff = Math.abs(belowW / belowH - canvasAspect)
      if (rightDiff < belowDiff) {
        startX = connMaxX + SPACING
        startY = connMinY
      } else {
        startX = connMinX
        startY = connMaxY + SPACING
      }
    }

    isoNodes.forEach((n, i) => {
      const col = i % cols
      const row = Math.floor(i / cols)
      posMap[n.name] = { x: startX + col * cellW, y: startY + row * cellH, w: n.w, h: n.h }
    })
  }

  nodes.value = tables.map(t => {
    const p = posMap[t.name]
    return {
      name: t.name, w: p.w, h: p.h, x: p.x, y: p.y,
      columns: t.columns.map(c => ({
        name: c.name,
        type: c.type || '',
        pk: !!c.pk,
        fk: !!fkSet.value[t.name + '.' + c.name]
      }))
    }
  })
  nextTick(fit)
}

const esc = (s) => String(s == null ? '' : s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]))

const buildExportSvg = () => {
  const c = exportColors
  const b = bbox(); const pad = 24
  const w = Math.max(1, b.w + pad * 2), h = Math.max(1, b.h + pad * 2)
  let s = `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">`
  s += `<rect x="0" y="0" width="${w}" height="${h}" fill="#ffffff"/>`
  s += `<defs><marker id="a" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto" markerUnits="strokeWidth"><path d="M0,0 L8,3 L0,6 Z" fill="${c.link}"/></marker>`
  s += `<marker id="ah" markerWidth="10" markerHeight="10" refX="8" refY="3" orient="auto" markerUnits="strokeWidth"><path d="M0,0 L8,3 L0,6 Z" fill="${c.linkHi}"/></marker></defs>`
  s += `<g transform="translate(${pad - b.minX}, ${pad - b.minY})">`
  linkPaths.value.forEach(l => {
    s += `<path d="${l.d}" fill="none" stroke="${l.hi ? c.linkHi : c.link}" stroke-width="${l.hi ? 2 : 1.2}" marker-end="url(#${l.hi ? 'ah' : 'a'})"/>`
  })
  nodes.value.forEach(n => {
    s += `<g transform="translate(${n.x},${n.y})">`
    s += `<rect width="${n.w}" height="${n.h}" rx="6" fill="${c.card}" stroke="${c.border}" stroke-width="1"/>`
    s += `<rect width="${n.w}" height="${HEADER_H}" rx="6" fill="${c.headerBg}"/>`
    s += `<rect y="${HEADER_H / 2}" width="${n.w}" height="${HEADER_H / 2}" fill="${c.headerBg}"/>`
    s += `<text x="${n.w / 2}" y="${HEADER_H / 2}" text-anchor="middle" dominant-baseline="middle" fill="${c.headerText}" font-size="13" font-weight="600" font-family="sans-serif">${esc(n.name)}</text>`
    n.columns.forEach((col, ci) => {
      const y = HEADER_H + ci * ROW_H
      if (col.pk) s += `<rect x="0" y="${y}" width="${n.w}" height="${ROW_H}" fill="${c.pkRow}"/>`
      s += `<text x="10" y="${y + ROW_H / 2}" dominant-baseline="middle" fill="${c.text}" font-size="12" font-family="sans-serif">${esc(col.name)}</text>`
      s += `<text x="${n.w - 10}" y="${y + ROW_H / 2}" text-anchor="end" dominant-baseline="middle" fill="${c.sub}" font-size="11" font-family="sans-serif">${esc(col.type)}</text>`
      if (col.pk) s += `<text x="${n.w - 64}" y="${y + ROW_H / 2}" text-anchor="end" dominant-baseline="middle" fill="${c.pk}" font-size="10" font-weight="600" font-family="sans-serif">PK</text>`
      else if (col.fk) s += `<text x="${n.w - 64}" y="${y + ROW_H / 2}" text-anchor="end" dominant-baseline="middle" fill="${c.fk}" font-size="10" font-weight="600" font-family="sans-serif">FK</text>`
    })
    s += `</g>`
  })
  s += `</g></svg>`
  return s
}

const download = (blob, filename) => {
  saveBlobAs(blob, filename)
  setTimeout(() => URL.revokeObjectURL(url), 4000)
}

const exportSvg = () => {
  if (!nodes.value.length) { ElMessage.warning(t('er.nothingToExport')); return }
  const blob = new Blob([buildExportSvg()], { type: 'image/svg+xml;charset=utf-8' })
  download(blob, `er-${props.database || 'db'}-${Date.now()}.svg`)
}
const exportPng = () => {
  if (!nodes.value.length) { ElMessage.warning(t('er.nothingToExport')); return }
  const s = buildExportSvg()
  const img = new Image()
  img.onload = () => {
    const b = bbox(); const pad = 24
    const scale = 2
    const canvas = document.createElement('canvas')
    canvas.width = (b.w + pad * 2) * scale
    canvas.height = (b.h + pad * 2) * scale
    const ctx = canvas.getContext('2d')
    ctx.scale(scale, scale)
    ctx.drawImage(img, 0, 0)
    canvas.toBlob(blob => {
      if (!blob) { ElMessage.error(t('er.exportPngFailed')); return }
      download(blob, `er-${props.database || 'db'}-${Date.now()}.png`)
    }, 'image/png')
  }
  img.onerror = () => ElMessage.error(t('er.exportPngFailed'))
  img.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(s)
}

const load = async () => {
  loading.value = true
  error.value = ''
  try {
    const resp = await getErGraph({ connectionId: props.connId, database: props.database })
    // 兼容两种响应包装：{success,data:{tables...}} 或直接 {tables...}
    const data = (resp && resp.success === true && resp.data) ? resp.data : resp
    const fk = {}
    ;(data.relations || []).forEach(r => { if (r.fromTable && r.fromColumn) fk[r.fromTable + '.' + r.fromColumn] = true })
    fkSet.value = fk
    relations.value = data.relations || []
    note.value = data.note || ''
    layout({ tables: data.tables || [], relations: data.relations || [] })
  } catch (e) {
    error.value = (e && e.message) ? e.message : t('er.loadFailed')
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  load()
  unsubscribeTheme = onResolvedThemeChange((t) => { theme.value = t })
})
onBeforeUnmount(() => { unsubscribeTheme && unsubscribeTheme() })
</script>

<style scoped>
.er-view { display: flex; flex-direction: column; flex: 1 1 auto; min-height: 0; }
.er-toolbar { display: flex; align-items: center; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--dc-border); background: var(--dc-bg-soft); }
.er-title { display: flex; align-items: center; gap: 6px; font-size: 14px; font-weight: 600; color: var(--dc-text); }
.er-spacer { flex: 1; }
.er-note { display: flex; align-items: center; gap: 6px; padding: 6px 12px; font-size: 13px; color: var(--dc-warning); background: var(--dc-warning-wash); border-bottom: 1px solid var(--el-border-color-lighter); }
.er-canvas {
  --er-grid: rgba(255,255,255,.07);
  position: relative;
  flex: 1 1 auto;
  min-height: 0;
  overflow: hidden;
  background:
    linear-gradient(90deg, var(--er-grid) 1px, transparent 1px) 0 0 / 24px 24px,
    linear-gradient(var(--er-grid) 1px, transparent 1px) 0 0 / 24px 24px,
    var(--dc-bg-deep);
  cursor: grab;
}
html[data-theme="light"] .er-canvas { --er-grid: rgba(0,0,0,.04); }
.er-canvas:active { cursor: grabbing; }
.er-svg { width: 100%; height: 100%; display: block; }
.er-overlay { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; color: var(--el-text-color-secondary); font-size: 14px; }
.er-error { color: var(--el-color-danger); }
</style>
