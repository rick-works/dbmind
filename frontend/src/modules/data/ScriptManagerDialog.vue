<template>
  <el-dialog
    v-model="visible"
    :title="$t('nav.scripts')"
    width="min(1080px, 92vw)"
    :close-on-click-modal="false"
    append-to-body
    class="script-mgr-dialog"
  >
    <!-- 工具条：条件搜索（脚本名 / 内容 / 连接 / 库 都参与匹配）+ 新建脚本 -->
    <div class="sm-toolbar">
      <el-input
        v-model="keyword"
        size="small"
        clearable
        :placeholder="$t('sm.searchPlaceholder')"
        :prefix-icon="Search"
        class="sm-search"
      />
      <div class="sm-count">{{ $t('sm.count', { n: shownScripts }) }}</div>
      <el-button type="primary" size="small" :icon="Plus" @click="$emit('create')">
        {{ $t('nav.newScript') }}
      </el-button>
    </div>

    <!-- 树形表格：分组 → 数据源 → 脚本。分组口径与左侧树一致（连接的 group 字段）；
         只有脚本行才有时间/操作，分组与数据源行只报数量。 -->
    <el-table
      ref="tableRef"
      :data="tree"
      size="small"
      :height="tableH"
      :empty-text="$t('sm.empty')"
      row-key="id"
      default-expand-all
      :tree-props="{ children: 'children' }"
      :indent="16"
      :row-class-name="rowClass"
      class="sm-table"
      @expand-change="onExpandChange"
    >
      <el-table-column :label="$t('sm.colName')" min-width="200">
        <template #default="{ row }">
          <div v-if="row.__kind === 'group'" class="sm-name-cell sm-group-cell">
            <el-icon class="sm-group-icon"><FolderOpened /></el-icon>
            <span class="sm-name">{{ row.label }}</span>
            <span class="sm-badge">{{ row.count }}</span>
          </div>
          <div v-else-if="row.__kind === 'conn'" class="sm-name-cell sm-conn-cell">
            <DbLogo :type="row.connType || 'MYSQL'" :size="14" />
            <span class="sm-name">{{ row.label }}</span>
            <span class="sm-badge">{{ row.count }}</span>
          </div>
          <div v-else class="sm-name-cell" :title="preview(row.content)">
            <el-icon class="sm-name-icon"><Tickets /></el-icon>
            <span class="sm-name">{{ row.name }}</span>
          </div>
        </template>
      </el-table-column>
      <el-table-column :label="$t('sm.colDb')" min-width="120" show-overflow-tooltip>
        <template #default="{ row }">
          <span v-if="row.__kind === 'script'" class="sm-db">{{ row.db }}</span>
        </template>
      </el-table-column>
      <el-table-column :label="$t('sm.colTime')" width="132">
        <template #default="{ row }">
          <span v-if="row.__kind === 'script'" class="sm-time">{{ fmtTime(row.time) }}</span>
        </template>
      </el-table-column>
      <el-table-column :label="$t('sm.colActions')" width="176" align="center">
        <template #default="{ row }">
          <template v-if="row.__kind === 'script'">
            <el-button link type="primary" size="small" @click="openRow(row)">{{ $t('sm.open') }}</el-button>
            <el-button link type="primary" size="small" @click="renameRow(row)">{{ $t('sm.rename') }}</el-button>
            <el-button link type="danger" size="small" @click="removeRow(row)">{{ $t('common.delete') }}</el-button>
          </template>
        </template>
      </el-table-column>
    </el-table>
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch, nextTick, onMounted, onBeforeUnmount } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Search, Plus, Tickets, FolderOpened } from '@element-plus/icons-vue'
import { listConnections } from '../../api'
import { t } from '../../utils/i18n'
import { envLabel } from '../../utils/envs'
import { groupConnections } from '../../utils/connGroups'
import { SCRIPT_PREFIX, readScripts, renameScript, removeScript, notifyScriptsChanged } from '../../utils/sqlScripts'
import DbLogo from '../../common/DbLogo.vue'

const props = defineProps({ modelValue: Boolean })
const emit = defineEmits(['update:modelValue', 'open', 'create'])

const visible = ref(false)
watch(() => props.modelValue, (v) => { visible.value = v })
watch(visible, (v) => emit('update:modelValue', v))
// 每次打开都重新枚举（编辑器里随时可能新增/修改了脚本）；顺手按当前视口重算表格高度
// —— 弹窗可能在窗口被拖动/最大化之后才打开，onMounted 那一刻的视口不一定是现在的
watch(visible, (v) => {
  if (!v) return
  syncTableH()
  load()
  // 弹窗 DOM 要等这一帧之后才挂到 body 上，那时才能挂全屏/尺寸观察器
  setTimeout(bindDialogObservers, 120)
})

const keyword = ref('')
const rows = ref([])
const connMap = ref({})
const connList = ref([])

/* 表格高度跟着视口走：以前写死 440，弹窗只有 880×530 出头，大屏上也只露七八行。
   现在默认 1080 宽，高度按「视口 - 弹窗头部/工具条/内边距」反推（300 是这些固定开销），
   320~640 之间夹住：矮窗口不出屏（弹窗总高受全局 max-height: 100vh-24px 兜底），
   高窗口最多 640（再多也只是空白行，不如让弹窗紧凑些）。 */
const tableH = ref(520)

/** 全屏态（右上角 ⤢，见 utils/dialogMaximize.js）算表格高度：弹窗高度这时是「视口 - 24」
 *  并且固定，表格直接吃掉「弹窗高 - 头部 - 内边距 - 工具条 - 间隙」剩下的全部高度。
 *  不这么做的话，表格还按「视口 - 300」算，全屏后底部空出一大截（真机截图）。 */
const syncMaxTableH = (dlg) => {
  const body = dlg.querySelector('.el-dialog__body')
  const bar = dlg.querySelector('.sm-toolbar')
  if (!body) return
  const cs = getComputedStyle(body)
  const pad = (parseFloat(cs.paddingTop) || 0) + (parseFloat(cs.paddingBottom) || 0)
  // 全屏态下 .el-dialog__body 被全局 dc-resizable 规则设成 flex:1 + 固定高度（弹窗高 - 头部），
  // 所以它的 clientHeight 就是可分配空间，不受表格内容影响（直接量它，别自己拿 dialog 高减头部，
  // 头部/间距的口径差一点就会让表格溢出或空一条）
  // 再留 12px：只差 8 的时候实测表格底边会多出 2px（表格自带边框/行高取整），
  // 而 body 是 overflow:auto，多这 2px 就足以让右侧冒出一条「内容没超也有」的滚动条
  const avail = body.clientHeight - pad - (bar ? bar.offsetHeight : 0) - 12
  if (avail > 120) tableH.value = Math.floor(avail)
}

const syncTableH = () => {
  const dlg = document.querySelector('.el-dialog.script-mgr-dialog')
  if (dlg && dlg.classList.contains('dc-max')) { syncMaxTableH(dlg); return }
  const vh = typeof window !== 'undefined' ? window.innerHeight : 900
  tableH.value = Math.max(320, Math.min(640, vh - 300))
}

// 弹窗是 teleport 到 body 的，得等它渲染出来再挂观察器：
//  · MutationObserver 盯 class —— 点全屏/还原时重算
//  · ResizeObserver 盯尺寸 —— 全屏态下拖窗口或拖边框时重算（非全屏不看，避免"表格改高→弹窗改高"的循环）
let dlgObservers = null
let boundDlg = null
const bindDialogObservers = () => {
  const dlg = document.querySelector('.el-dialog.script-mgr-dialog')
  if (!dlg || dlg === boundDlg) return
  dlgObservers?.disconnect()
  const ro = new ResizeObserver(() => { if (dlg.classList.contains('dc-max')) syncMaxTableH(dlg) })
  ro.observe(dlg)
  const mo = new MutationObserver(() => syncTableH())
  mo.observe(dlg, { attributes: true, attributeFilter: ['class'] })
  dlgObservers = { disconnect: () => { ro.disconnect(); mo.disconnect() } }
  boundDlg = dlg
}

onMounted(() => {
  syncTableH()
  window.addEventListener('resize', syncTableH)
})
onBeforeUnmount(() => {
  window.removeEventListener('resize', syncTableH)
  dlgObservers?.disconnect()
  dlgObservers = null
  boundDlg = null
})

// ===== 数据枚举：遍历 localStorage 里 dc_scripts:<connId>:<db> 前缀的全部 key =====
// value 结构与 SQL 编辑器的保存格式一致：[{ id, name, content, time }]
const load = async () => {
  // 连接映射：id → 名称/类型/分组（连接可能已被删除，此时显示「连接已删除」但脚本仍可打开/删除）
  try {
    const list = await listConnections()
    connList.value = Array.isArray(list) ? list : (list?.data || [])
    connMap.value = Object.fromEntries(connList.value.map(c => [String(c.id), c]))
  } catch {
    connList.value = []
    connMap.value = {}
  }
  const out = []
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i)
      if (!key || !key.startsWith(SCRIPT_PREFIX)) continue
      const rest = key.slice(SCRIPT_PREFIX.length)
      const at = rest.indexOf(':')
      if (at <= 0) continue
      const connId = rest.slice(0, at)
      const db = rest.slice(at + 1)
      const c = connMap.value[String(connId)]
      for (const s of readScripts(connId, db)) {
        if (!s || !s.id) continue
        out.push({
          connId, db, id: s.id, name: s.name || s.id,
          content: s.content || '', time: s.time || 0,
          connName: c?.name || '', connType: c?.type || ''
        })
      }
    }
  } catch { /* localStorage 不可用时给空列表 */ }
  out.sort((a, b) => (b.time || 0) - (a.time || 0))
  rows.value = out
}

const preview = (content) =>
  String(content || '').split('\n').map(l => l.trim()).filter(Boolean).slice(0, 6).join('  ·  ').slice(0, 400)

const fmtTime = (ts) => {
  if (!ts) return '—'
  const d = new Date(ts)
  const p = (n) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

/** 三类行的样式钩子：分组 / 数据源行加底色（层级靠底色递进），脚本行保持白底。 */
const rowClass = ({ row }) =>
  row.__kind === 'group' ? 'sm-row-group' : row.__kind === 'conn' ? 'sm-row-conn' : 'sm-row-script'

/* 展开态：`default-expand-all` 只在**首次渲染**时起作用，而本弹窗的数据是异步 load 进来的
   （先渲染空表、再填数据），于是打开时只剩一行分组、脚本全折在里面（真机截图「这是什么」）。
   这里在数据变化后主动展开分组与数据源行；用户手动收起的（记在 userCollapsed 里）不再
   自动展开，否则「收起这个分组」刷新一次就白做了。 */
const tableRef = ref(null)
const userCollapsed = ref(new Set())
const onExpandChange = (row, isExpanded) => {
  if (!row || !row.id) return
  if (isExpanded) userCollapsed.value.delete(row.id)
  else userCollapsed.value.add(row.id)
}

/** 分组口径与左侧树 / 连接下拉完全一致：连接的 `group` 字段（后端已把空值补成「数据源」）。 */
const groupLabelOf = (conn) => envLabel(String((conn && conn.group) || '').trim())

const tree = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  const hit = (r) => !kw
    || r.name.toLowerCase().includes(kw)
    || String(r.content || '').toLowerCase().includes(kw)
    || String(r.db || '').toLowerCase().includes(kw)
    || String(r.connName || '').toLowerCase().includes(kw)

  // 连接 → 脚本
  const byConn = new Map()
  for (const r of rows.value) {
    if (!hit(r)) continue
    const key = String(r.connId)
    if (!byConn.has(key)) {
      byConn.set(key, { connId: key, connName: r.connName, connType: r.connType, conn: connMap.value[key], scripts: [] })
    }
    byConn.get(key).scripts.push(r)
  }
  if (!byConn.size) return []

  // 分组顺序照连接下拉：自定义目录（按名）→ DEV/TEST/PROD → 未分组，剩下的（连接已删除）垫底
  const order = groupConnections(connList.value).map(g => g.label)
  const groups = new Map()
  for (const node of byConn.values()) {
    const label = node.conn ? groupLabelOf(node.conn) : t('sm.connGone')
    if (!groups.has(label)) groups.set(label, [])
    groups.get(label).push(node)
  }
  const rank = (label) => { const i = order.indexOf(label); return i === -1 ? 999 : i }
  return [...groups.entries()]
    .sort((a, b) => rank(a[0]) - rank(b[0]) || a[0].localeCompare(b[0], 'zh-Hans-CN'))
    .map(([label, nodes]) => ({
      id: `g:${label}`,
      __kind: 'group',
      label,
      count: nodes.reduce((n, x) => n + x.scripts.length, 0),
      children: nodes
        .sort((a, b) => String(a.connName || '').localeCompare(String(b.connName || ''), 'zh-Hans-CN'))
        .map(node => ({
          id: `c:${node.connId}`,
          __kind: 'conn',
          label: node.connName || t('sm.connGone'),
          connType: node.connType,
          count: node.scripts.length,
          children: node.scripts.map(s => ({
            ...s,
            __kind: 'script',
            id: `s:${node.connId}:${s.db}:${s.id}`
          }))
        }))
    }))
})

/// 搜索框旁的计数按「脚本条数」报（分组 / 数据源行不算）
const shownScripts = computed(() =>
  tree.value.reduce((n, g) => n + g.children.reduce((m, c) => m + c.children.length, 0), 0)
)

// 数据换了一批就重新展开（见 tableRef 那段注释）；用户收起的行由 userCollapsed 屏蔽
watch(tree, async () => {
  await nextTick()
  const table = tableRef.value
  if (!table) return
  const expand = (row) => {
    if (!row || row.__kind === 'script' || userCollapsed.value.has(row.id)) return
    table.toggleRowExpansion(row, true)
  }
  for (const group of tree.value) {
    expand(group)
    for (const conn of group.children || []) expand(conn)
  }
})

const openRow = (row) => emit('open', { connId: row.connId, db: row.db, scriptId: row.id })

const renameRow = async (row) => {
  let input
  try {
    const res = await ElMessageBox.prompt(t('sm.renameHint'), t('sm.renameTitle'), {
      inputValue: row.name,
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      inputValidator: (v) => {
        const next = String(v || '').trim()
        if (!next) return t('sm.renameEmpty')
        if (next === row.name) return true
        // 同一连接同一库下不允许重名，否则「打开」时会打开到另一条
        const dup = rows.value.some(r => r.id !== row.id && r.connId === row.connId && r.db === row.db && r.name === next)
        return dup ? t('sm.renameDup') : true
      }
    })
    input = res.value
  } catch { return }
  const result = renameScript(row.connId, row.db, row.id, input)
  if (result === 'empty') return ElMessage.warning(t('sm.renameEmpty'))
  if (result === 'dup') return ElMessage.warning(t('sm.renameDup'))
  if (result === 'gone') return ElMessage.warning(t('sm.gone'))
  notifyScriptsChanged(row.connId, row.db)
  ElMessage.success(t('sm.renamed'))
  load()
}

const removeRow = async (row) => {
  try {
    await ElMessageBox.confirm(t('sm.delConfirm', { name: row.name }), t('sm.delTitle'), {
      type: 'warning',
      confirmButtonText: t('common.delete'),
      cancelButtonText: t('common.cancel')
    })
  } catch { return }
  removeScript(row.connId, row.db, row.id)
  // 通知树与编辑器刷新（MainView 监听该事件刷新 scripts 节点）
  notifyScriptsChanged(row.connId, row.db)
  ElMessage.success(t('sm.deleted'))
  load()
}

defineExpose({ reload: load })
</script>

<style scoped>
.sm-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
}
.sm-search { width: 320px; }
.sm-count {
  flex: 1;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}

/* ===== 字号体系：一个弹窗里只允许这几档 =====
   标题 15/600 · 表体与工具条 13 · 表头与次要信息 12。
   以前表头、分组名、脚本名、时间各写一套（11~13 混用），行高又由各自 padding 决定，
   看着忽大忽小、上下不齐（真机反馈「有点丑」）。 */
:global(.el-dialog.script-mgr-dialog .el-dialog__title) {
  font-size: 16px;
  font-weight: 600;
  letter-spacing: 0.2px;
}
.sm-table :deep(.el-table__cell) { padding: 6px 0; }
.sm-table :deep(.cell) {
  padding: 0 10px;
  font-size: 13px;
  line-height: 20px;
}
/* 只有第一列要 flex：树形的展开箭头（inline-block）与我插的 name-cell（块级 div）不并排的话，
   箭头会独占一行、行高直接翻倍（真机截图：每个分组行都空出半行）。
   其余列保持默认 block —— 全列 flex 会让内容不肯收缩，表格被撑出横向滚动条。 */
.sm-table :deep(.el-table__cell:first-child .cell) {
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
}
.sm-table :deep(.el-table__cell:first-child .cell > *) { min-width: 0; }
/* 表头：底色走「表头专用」那一档（以前用 --el-fill-color-light，深色主题下它是亮色 #f5f7fa
   ⇒ 整行白底压黑字）。字号比表体大 1px、颜色提到中灰，缩起来时才认得出是"标题行"。 */
.sm-table :deep(th.el-table__cell) { background: var(--dc-bg-table-head); }
/* 选择器要多带一层 .el-table__header：全局有一条 `.el-dialog .el-table th.el-table__cell .cell`
   设 12px，比 `th .cell` 多一个元素选择器，不加这一层会被它压回去（实测就是 12px 没变）。 */
.sm-table :deep(.el-table__header th.el-table__cell .cell) {
  font-size: 13px;
  font-weight: 600;
  letter-spacing: 0.2px;
  color: var(--dc-text-mid);
}
/* 分组行 / 数据源行**不铺底色**（用户要求：底色只留给表头那一行）。
   层级靠左侧主色竖条 + 图标 + 字重表达，比一层层叠色块干净。 */
.sm-table :deep(.sm-row-group > td.el-table__cell),
.sm-table :deep(.sm-row-conn > td.el-table__cell) { background: transparent; }
.sm-table :deep(.sm-row-group > td.el-table__cell:first-child) {
  box-shadow: inset 3px 0 0 var(--el-color-primary);
}
.sm-table :deep(.sm-row-group:hover > td.el-table__cell),
.sm-table :deep(.sm-row-conn:hover > td.el-table__cell) { background: var(--dc-hover-wash); }
/* 展开箭头压小一点，避免在大行高里显得突兀 */
.sm-table :deep(.el-table__expand-icon) { font-size: 12px; color: var(--el-text-color-secondary); }

.sm-name-cell { display: flex; align-items: center; gap: 7px; min-width: 0; }
.sm-name-icon,
.sm-group-icon { flex: none; font-size: 14px; }
.sm-name-icon { color: var(--el-color-primary); }
.sm-group-icon { color: var(--el-color-warning); }
.sm-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sm-group-cell .sm-name { font-weight: 600; }
.sm-conn-cell { display: flex; align-items: center; gap: 7px; }

/* 徽标：分组 / 数据源行尾部的小圆角数字 */
.sm-badge {
  flex: none;
  min-width: 18px;
  padding: 0 6px;
  border-radius: 9px;
  font-size: 11px;
  font-weight: 500;
  line-height: 16px;
  text-align: center;
  color: var(--el-text-color-secondary);
  background: var(--dc-bg-hover);
}
/* 行底色去掉后徽标回到"比卡片抬一档"，否则和背景同色看不见 */
.sm-table :deep(.sm-row-group .sm-badge),
.sm-table :deep(.sm-row-conn .sm-badge) { background: var(--dc-bg-hover); }

/* 库名做成小标签：与时间为同一档次要信息，但形状不同，不会跟脚本名抢注意力 */
.sm-db {
  display: inline-block;
  max-width: 100%;
  padding: 0 7px;
  border-radius: 4px;
  font-size: 12px;
  line-height: 19px;
  color: var(--el-text-color-regular);
  background: var(--el-fill-color);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sm-time {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  font-variant-numeric: tabular-nums;
}
/* 操作列三个文字按钮均分间距（默认 12px，加了「重命名」后挤在一起） */
.sm-table :deep(.el-table__cell .el-button + .el-button) { margin-left: 10px; }

/* 全屏态表格已经按正文空间算好高度，外壳不该再滚：全局 dc-resizable 规则给正文设了
   overflow:auto，哪怕只差 1~2px 也会冒出一条「内容明明没超」的滚动条（真机截图）。
   这里全屏时直接关掉；非全屏保持可滚（窗口很矮时还能滚到底）。 */
:global(.el-dialog.script-mgr-dialog.dc-max > .el-dialog__body) { overflow: hidden; }
</style>
