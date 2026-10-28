<template>
  <div v-loading="loading" class="form-tab">
    <template v-if="!loading">
      <!-- 顶部标题栏：**独立成一张小卡**（与「编辑表结构」同一套外观） -->
      <div class="form-tab-header">
        <div class="header-title">
          <el-icon :size="16" class="header-icon"><component :is="catIcon" /></el-icon>
          <span class="header-text">{{ title }}</span>
          <!-- 路径：数据源名称在前，中间用 `.` 连接 -->
          <el-tag size="small" effect="plain" type="info">{{ [conn?.name, dbDisplayOf(conn?.type, database)].filter(Boolean).join('.') }}</el-tag>
        </div>
        <!-- 动作按钮一律在底部 SQL 预览那一行（与「编辑表结构」同一排法）：
             顶部只留标题与连接，右侧不再放「复制 SQL / 关闭」—— 关闭走页签上的 ×。 -->
      </div>

      <!-- 主体：**上编辑 + 下预览**（与「编辑表结构」一致；以前预览是右侧 380px 固定栏） -->
      <div class="form-tab-body">
        <!-- 编辑区 -->
        <div class="form-area">
          <el-alert v-if="cat === 'events' && !has(features, 'supportsEvents')" type="warning" :closable="false" show-icon class="mb12">
            <template #title>{{ $t('oft.eventUnsupported') }}</template>
          </el-alert>

          <TableForm v-if="cat === 'tables'" ref="formRef" :features="features" :edit-mode="mode === 'edit'" @sql="onSqlChange" />
          <ViewForm v-else-if="cat === 'views'" ref="formRef" :features="features" :conn="conn" :database="database" :edit-mode="mode === 'edit'" :edit-data="editData" @sql="onSqlChange" />
          <IndexForm v-else-if="cat === 'indexes'" ref="formRef" :features="features" :conn="conn" :database="database" :edit-mode="mode === 'edit'" :edit-data="editData" @sql="onSqlChange" />
          <ProcForm v-else-if="cat === 'procs'" ref="formRef" :features="features" :conn="conn" :edit-mode="mode === 'edit'" :edit-data="editData" @sql="onSqlChange" @kind="onFormKind" />
          <TriggerForm v-else-if="cat === 'triggers'" ref="formRef" :features="features" :conn="conn" :database="database" :edit-mode="mode === 'edit'" :edit-data="editData" @sql="onSqlChange" />
          <EventForm v-else-if="cat === 'events'" ref="formRef" :features="features" :conn="conn" :edit-mode="mode === 'edit'" :edit-data="editData" @sql="onSqlChange" />
        </div>

        <!-- 分隔条：**它就是两块卡片之间那道缝**（6px；透明底 → 露出页面底色），
             4px 细条在这 6px 里上下居中 → 正好落在缝中间。向上拖 = 预览变高，
             双击在最大/最小之间来回切（与「编辑表结构」同一套手感，见 TableDetailView 的 .sql-resizer）。 -->
        <div v-show="!sqlCollapsed" class="sql-resizer"
             :title="$t('tdet.resizerTitle')"
             @mousedown.prevent="onSqlResizeStart" @dblclick="onSqlResizeDblClick"></div>

        <!-- SQL 预览：在编辑区**下方**，点标题行可展开/收起，高度由拖动 / 双击控制（默认 220px） -->
        <div ref="sqlAreaRef" class="sql-area" :class="{ collapsed: sqlCollapsed }"
             :style="sqlCollapsed ? null : { height: sqlHeight + 'px' }">
          <div class="sql-head" @click="sqlCollapsed = !sqlCollapsed"
               :title="sqlCollapsed ? $t('tdet.expandSql') : $t('tdet.collapseSql')">
            <el-icon class="sql-fold" :class="{ folded: sqlCollapsed }"><CaretRight /></el-icon>
            <el-icon><DocumentCopy /></el-icon>
            <span>{{ $t('oft.preview') }}</span>
            <!-- 「展开/收起」放最右：左侧留白由 .sql-state 的 margin-left:auto 撑开，状态标签落在它左边 -->
            <span class="sql-fold-tx">{{ sqlCollapsed ? $t('tdet.expand') : $t('tdet.collapse') }}</span>
            <el-tag size="small" effect="plain" type="info" class="sql-state" :class="previewOk ? 'ok' : 'warn'">
              {{ previewOk ? $t('oft.runnable') : $t('oft.incomplete') }}
            </el-tag>
          </div>
          <div v-show="!sqlCollapsed" class="sql-body">
            <pre><code>{{ previewSql || $t('oft.autoSqlHint') }}</code></pre>
          </div>
          <div v-if="conn?.type === 'MYSQL' && /CREATE\s+FUNCTION\b/i.test(previewSql)" class="sql-hint">
            {{ $t('oft.mysqlFnTip') }}
          </div>
          <div class="sql-foot">
            <div class="sql-foot-actions">
              <!-- 「复制 SQL」从标题栏挪到这里：它本来就是围绕预览的动作 -->
              <el-button size="small" text :icon="CopyDocument" :disabled="!previewSql" @click="copySql">{{ $t('oft.copySql') }}</el-button>
              <el-button type="primary" :icon="CaretRight" :loading="saving" :disabled="!previewOk" size="small" @click="submit">
                {{ mode === 'edit' ? $t('oft.saveEdit') : $t('oft.doCreate') }}
              </el-button>
            </div>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { ElMessage } from 'element-plus'
import { CopyDocument, DocumentCopy, CaretRight, Grid, View, Files, Operation, Cpu, Setting, BellFilled, Timer } from '@element-plus/icons-vue'
import { getFeatures, getObjectInfo, getTableDdl, alterTable } from '../../api'
import { errMsg } from '../../utils/errMsg'
import { guardConnWrite, loadProtectProd } from '../../utils/connGuard'
import { has } from '../../common/objectforms/objectFormUtils'
import { t } from '../../utils/i18n'
// 路径里的「库」：没有库层的类型（Oracle / DM）只显示模式，不显示服务名
import { dbDisplayOf } from '../../types'
import TableForm from '../../common/objectforms/TableForm.vue'
import ViewForm from '../../common/objectforms/ViewForm.vue'
import IndexForm from '../../common/objectforms/IndexForm.vue'
import ProcForm from '../../common/objectforms/ProcForm.vue'
import TriggerForm from '../../common/objectforms/TriggerForm.vue'
import EventForm from '../../common/objectforms/EventForm.vue'

const props = defineProps({
  conn: { type: Object, required: true },
  database: { type: String, default: '' },
  cat: { type: String, required: true },
  mode: { type: String, default: 'create' },
  object: { type: Object, default: () => ({}) }
})
const emit = defineEmits(['saved', 'close', 'kind'])

// 用函数而不是模块级常量：常量只在模块加载时求值一次，切换语言后标签不会跟着变
const catLabelOf = (cat) => ({
  tables: t('mv.catTable'), views: t('mv.catView'), indexes: t('mv.catIndex'),
  procs: t('mv.catProc'), triggers: t('mv.catTrigger'), events: t('mv.catEvent')
}[cat] || cat)
const CAT_ICONS = { tables: Grid, views: View, indexes: Files, procs: Operation, triggers: BellFilled, events: Timer }

// 过程/函数由子表单回报（新建时用户可切换），用于标题与页签文案
const formKind = ref('')
const onFormKind = (k) => {
  if (!k || k === formKind.value) return
  formKind.value = k
  emit('kind', k)
}
const routineIsFn = computed(() =>
  props.cat === 'procs' && (formKind.value === 'function' || props.object?.kind === 'function' || props.conn?.type === 'CLICKHOUSE')
)
const title = computed(() => {
  let base = catLabelOf(props.cat)
  // 函数不是存储过程，标题要按对象类型区分
  if (routineIsFn.value) base = t('mv.catFunc')
  if (props.mode === 'edit') return t('mv.editLabel', { kind: base, name: (props.object.name || '') })
  return t('mv.newObjectTitle', { kind: base })
})
const catIcon = computed(() => {
  if (props.cat === 'procs') {
    if (routineIsFn.value && formKind.value !== 'procedure') return Cpu
    if (props.object?.kind === 'procedure' || formKind.value === 'procedure') return Setting
    return Operation
  }
  return CAT_ICONS[props.cat] || Grid
})

const features = ref({})
const editData = ref({})
const previewSql = ref('')
const loading = ref(false)
const saving = ref(false)
const formRef = ref(null)

const previewOk = computed(() => {
  const s = previewSql.value
  return !!s && !/^--/.test(s.trim()) && s.trim().length > 0
})

const onSqlChange = (sql) => { previewSql.value = sql }

/* ==================== SQL 预览高度（拖动分隔条 / 双击） ====================
   与「编辑表结构」(TableDetailView.vue) 同一套手感：向上拖变高，双击在最大 ↔ 最小之间切换。
   默认 220px = 原来写死的固定高度，所以不拖的时候观感不变。
   上限取**所在容器的实际可用高度**（编辑区能收成 0，拖到最上时预览可以占满整块主体），
   下限 0 —— 允许把预览整个拖走，只留那条 6px 分隔条，往上拖一下就回来。 */
const SQL_H_DEFAULT = 220
const SQL_H_MIN = 0
/** 预览是否收起（点标题行切换；收起后高度交给内容，只藏正文与分隔条） */
const sqlCollapsed = ref(false)
const sqlAreaRef = ref(null)
const sqlHeight = ref(SQL_H_DEFAULT)
const sqlMaxH = () => {
  const parent = sqlAreaRef.value?.parentElement
  const available = parent ? parent.clientHeight : Math.round(window.innerHeight * 0.7)
  return Math.max(SQL_H_MIN, available)
}
let sqlDrag = null
const onSqlResizeMove = (e) => {
  if (!sqlDrag) return
  // 向上拖 = 变高：起始高度 + 鼠标上移的距离
  const next = sqlDrag.startH + (sqlDrag.startY - e.clientY)
  sqlHeight.value = Math.min(sqlMaxH(), Math.max(SQL_H_MIN, Math.round(next)))
}
const onSqlResizeEnd = () => {
  sqlDrag = null
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
  window.removeEventListener('mousemove', onSqlResizeMove)
  window.removeEventListener('mouseup', onSqlResizeEnd)
}
const onSqlResizeStart = (e) => {
  sqlDrag = { startY: e.clientY, startH: sqlHeight.value }
  document.body.style.cursor = 'row-resize'
  document.body.style.userSelect = 'none'
  window.addEventListener('mousemove', onSqlResizeMove)
  window.addEventListener('mouseup', onSqlResizeEnd)
}
/** 双击：最大 ↔ 最小 */
const onSqlResizeDblClick = () => {
  const max = sqlMaxH()
  sqlHeight.value = sqlHeight.value >= max - 8 ? SQL_H_MIN : max
}
onBeforeUnmount(onSqlResizeEnd)

const loadFeatures = async () => {
  try { features.value = await getFeatures(props.conn.id) }
  catch { features.value = {} }
}

const isValidDdl = (ddl) => typeof ddl === 'string' && /\bCREATE\b/i.test(ddl)

const loadEditData = async () => {
  if (props.mode !== 'edit' || !props.object.name) return
  try {
    const res = await getObjectInfo(props.conn.id, props.database, props.object.kind || props.cat, props.object.name)
    let ddl = res?.ddl
    // 视图/表等对象若 getObjectInfo 返回的 ddl 无效，fallback 到 getTableDdl
    if (!isValidDdl(ddl) && (props.cat === 'views' || props.cat === 'tables')) {
      try {
        const fallback = await getTableDdl(props.conn.id, props.database, props.object.name)
        if (fallback?.ddl && isValidDdl(fallback.ddl)) ddl = fallback.ddl
      } catch {}
    }
    editData.value = { name: props.object.name, ...(res || {}), ddl: ddl || '' }
  } catch (e) {
    editData.value = { name: props.object.name, ddl: '' }
    ElMessage.warning(t('oft.loadDefFailed', { detail: (e?.message || e) }))
  }
}

const copySql = async () => {
  if (!previewSql.value) return
  try {
    await navigator.clipboard.writeText(previewSql.value)
    ElMessage.success(t('oft.sqlCopied'))
  } catch { ElMessage.warning(t('sqlq.copyFailed')) }
}

const submit = async () => {
  if (!previewOk.value) return ElMessage.warning(t('oft.needInfo'))
  // 只读连接 / 生产保护：点击那一刻拦下（右上角通知，与全站写入口同一闸门、同一句提示）
  await loadProtectProd()
  if (!guardConnWrite(props.conn || {}, t(props.mode === 'edit' ? 'oft.saveEdit' : 'oft.doCreate'))) return
  saving.value = true
  try {
    const res = await alterTable(props.conn.id, props.database, previewSql.value)
    if (res.success) {
      ElMessage.success(props.mode === 'edit' ? t('oft.updated') : t('oft.created'))
      emit('saved', props.database, props.cat)
      emit('close')
    } else {
      ElMessage.error(t('oft.execFailed', { detail: (res.message || t('common.unknownError')) }))
    }
  } catch (e) {
    ElMessage.error(t('oft.execFailed', { detail: errMsg(e) }))
  } finally {
    saving.value = false
  }
}

onMounted(async () => {
  loading.value = true
  try {
    await loadFeatures()
    await loadEditData()
  } finally {
    loading.value = false
  }
})
</script>

<style scoped>
/* ===== 与「编辑表结构」(TableDetailView.vue) 同一套外观 =====
   三张小卡自上而下并列：标题行 / 编辑卡 / SQL 预览卡，外观完全一致。
   改这里时请对照 TableDetailView.vue 的骨架样式一起改 —— 用户在两个页面之间
   来回切，任何一处不一致都会被一眼看出来。 */
.form-tab {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--dc-bg-deep);
  /* 四周留 5px：卡片贴着面板边缘时，8px 圆角会被边线"压平"，看起来像没做圆角 */
  padding: 5px;
  box-sizing: border-box;
  min-height: 0;
}
/* 顶部标题行：独立成一张小卡（四角 8px 圆角 + 完整边框 + 同款轻阴影），
   不再充当"下面那张卡的标题行"，所以下边框也用常规色，不用较浅的 soft。 */
.form-tab-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-soft);
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
  flex-shrink: 0;
}
.header-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
  color: var(--dc-text);
  min-width: 0;
}
.header-title .header-text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.header-icon { color: var(--dc-primary); flex-shrink: 0; }
.header-actions { display: flex; gap: 6px; flex-shrink: 0; }

/* 编辑区在上、SQL 预览在下（原来预览是右侧 380px 固定栏）：
   纵向堆叠后两者都按 flex 分配高度，因此都必须是可收缩的（min-height: 0） */
.form-tab-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-height: 0;
}

/* 编辑区：左右**不留内边距**（下方预览卡是通栏的，编辑区若两侧缩进就会比它窄一圈，
   看着像两块宽度不同的面板）；上面留 5px = 与标题卡之间的缝隙。 */
.form-area {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  /* 视图 / 过程 / 触发器 / 事件这些表单没有自己的卡片，靠这里留出常规内边距 */
  padding: 5px 12px 12px;
}
/* 表格（新建表）的页签本身就是一张**通栏卡**（见 TableForm 的样式）：
   去掉左右内边距，它才与下方那张 SQL 预览卡同宽 —— 否则两张卡宽度差一圈，
   看起来像两块不同的面板。
   用 :has() 只为表格这条路径改：其它对象类型保持上面的常规内边距。 */
.form-area:has(.table-tabs) {
  padding: 5px 0 0;
}

/* 右侧 SQL → 底部 SQL：同样是一张卡（边框/圆角/轻阴影与上面两张一致）。
   高度由拖动分隔条 / 双击控制（内联 style 给 height，默认 220px），这里不再写死；
   **也不再留 margin-top** —— 那 6px 缝现在由 .sql-resizer 自己撑开。 */
.sql-area {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-card);
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
  overflow: hidden;
  min-height: 0;
}
/* 收起：高度交给内容（标题行 + 底栏）；分隔条 v-show 隐藏，那条 6px 缝改由 margin-top 补上 */
.sql-area.collapsed { height: auto; margin-top: 6px; }
/* 分隔条：6px 高、透明底（露出页面底色 = 两块卡片之间那道缝），4px 细条上下居中；
   悬停变主色，鼠标是上下箭头。与 TableDetailView.vue 的 .sql-resizer 完全一致。 */
.sql-resizer { height: 6px; flex: 0 0 6px; cursor: row-resize; position: relative; z-index: 2; background: transparent; }
.sql-resizer::before {
  content: ''; position: absolute; left: 50%; top: 50%;
  transform: translate(-50%, -50%);
  width: 48px; height: 4px; border-radius: 3px;
  background: var(--dc-text-dim); opacity: .45;
  transition: background .15s ease, opacity .15s ease;
}
.sql-resizer:hover::before { background: var(--dc-primary); opacity: 1; }
.sql-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  font-size: 13px;
  color: var(--dc-text-dim);
  background: var(--dc-bg-soft);
  border-bottom: 1px solid var(--dc-border-soft);
  flex-shrink: 0;
  cursor: pointer;
  user-select: none;
}
.sql-head:hover { background: var(--dc-bg-hover); color: var(--dc-text); }
/* 折叠箭头：展开时朝下、收起时朝右 */
.sql-fold { transition: transform .18s ease; transform: rotate(90deg); }
.sql-fold.folded { transform: rotate(0deg); }
/* 「展开/收起」贴最右：auto 外边距放在它身上，状态标签跟在它后面（与「编辑表结构」同一排法） */
.sql-fold-tx { margin-left: auto; font-size: 12px; color: var(--dc-text-weak); }
.sql-state { font-size: 12px; }
.sql-state.ok { --el-tag-bg-color: rgba(61,220,151,.1); --el-tag-border-color: rgba(61,220,151,.4); --el-tag-text-color: var(--dc-accent); }
.sql-state.warn { --el-tag-bg-color: var(--dc-warning-wash); --el-tag-border-color: rgba(245,179,77,.4); --el-tag-text-color: var(--dc-warning); }
.sql-body {
  flex: 1;
  overflow: auto;
  padding: 10px 12px;
  min-height: 0;
  background: var(--dc-bg-code);
}
.sql-body pre { margin: 0; }
.sql-body code {
  font-family: "SF Mono", Consolas, monospace;
  font-size: 13px;
  line-height: 1.7;
  color: var(--dc-code-text);
  white-space: pre-wrap;
  word-break: break-all;
}
.sql-foot {
  padding: 8px 12px;
  border-top: 1px solid var(--dc-border-soft);
  display: flex;
  align-items: center;
  justify-content: flex-end;
  flex-shrink: 0;
}
/* 底部按钮组（复制 SQL ／ 执行创建·保存修改）：与「编辑表结构」的 .sql-foot-actions 同一排法 */
.sql-foot-actions { display: flex; align-items: center; gap: 2px; flex-shrink: 0; }
.sql-hint {
  padding: 8px 12px;
  font-size: 12px;
  color: var(--dc-warning);
  background: var(--dc-warning-wash);
  border-top: 1px solid var(--dc-border-soft);
  flex-shrink: 0;
}

.mb12 { margin-bottom: 12px; }
</style>

<style>
/* 让表单卡片在 Tab 里更协调 */
.form-tab .obj-form { gap: 10px; }
</style>
