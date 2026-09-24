<template>
  <div v-loading="loading" class="form-tab">
    <template v-if="!loading">
      <!-- 顶部标题栏 -->
      <div class="form-tab-header">
        <div class="header-title">
          <el-icon :size="16" class="header-icon"><component :is="catIcon" /></el-icon>
          <span class="header-text">{{ title }}</span>
          <el-tag size="small" effect="plain" type="info">{{ database }}</el-tag>
        </div>
        <div class="header-actions">
          <el-button size="small" text :icon="CopyDocument" @click="copySql">{{ $t('oft.copySql') }}</el-button>
          <el-button size="small" :icon="Close" @click="emit('close')">{{ $t('common.close') }}</el-button>
        </div>
      </div>

      <!-- 主体：左表单 + 右预览 -->
      <div class="form-tab-body">
        <!-- 左侧表单 -->
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

        <!-- 右侧 SQL 预览 -->
        <div class="sql-area">
          <div class="sql-head">
            <el-icon><DocumentCopy /></el-icon>
            <span>{{ $t('oft.preview') }}</span>
            <el-tag size="small" effect="plain" type="info" class="sql-state" :class="previewOk ? 'ok' : 'warn'">
              {{ previewOk ? $t('oft.runnable') : $t('oft.incomplete') }}
            </el-tag>
          </div>
          <div class="sql-body">
            <pre><code>{{ previewSql || $t('oft.autoSqlHint') }}</code></pre>
          </div>
          <div v-if="conn?.type === 'MYSQL' && /CREATE\s+FUNCTION\b/i.test(previewSql)" class="sql-hint">
            {{ $t('oft.mysqlFnTip') }}
          </div>
          <div class="sql-foot">
            <el-button type="primary" :icon="CaretRight" :loading="saving" :disabled="!previewOk" size="small" @click="submit">
              {{ mode === 'edit' ? $t('oft.saveEdit') : $t('oft.doCreate') }}
            </el-button>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { ElMessage } from 'element-plus'
import { CopyDocument, Close, DocumentCopy, CaretRight, Grid, View, Files, Operation, Cpu, Setting, BellFilled, Timer } from '@element-plus/icons-vue'
import { getFeatures, getObjectInfo, getTableDdl, alterTable } from '../../api'
import { errMsg } from '../../utils/errMsg'
import { has } from '../../common/objectforms/objectFormUtils'
import { t } from '../../utils/i18n'
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
.form-tab {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--dc-bg-deep);
}
.form-tab-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border-bottom: 1px solid var(--dc-border-soft);
  background: var(--dc-bg-soft);
  flex-shrink: 0;
}
.header-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
  color: var(--dc-text);
}
.header-icon { color: var(--dc-primary); }
.header-actions { display: flex; gap: 6px; }

.form-tab-body {
  flex: 1;
  display: flex;
  overflow: hidden;
  min-height: 0;
}

/* 左侧表单 */
.form-area {
  flex: 1;
  overflow-y: auto;
  padding: 14px;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

/* 右侧 SQL */
.sql-area {
  width: 380px;
  border-left: 1px solid var(--dc-border-soft);
  background: var(--dc-bg-code);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}
.sql-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 12px;
  font-size: 13px;
  color: var(--dc-text-dim);
  border-bottom: 1px solid var(--dc-border-soft);
  flex-shrink: 0;
}
.sql-state { font-size: 12px; margin-left: auto; }
.sql-state.ok { --el-tag-bg-color: rgba(61,220,151,.1); --el-tag-border-color: rgba(61,220,151,.4); --el-tag-text-color: var(--dc-accent); }
.sql-state.warn { --el-tag-bg-color: var(--dc-warning-wash); --el-tag-border-color: rgba(245,179,77,.4); --el-tag-text-color: var(--dc-warning); }
.sql-body {
  flex: 1;
  overflow: auto;
  padding: 10px 12px;
  min-height: 0;
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
  padding: 10px 12px;
  border-top: 1px solid var(--dc-border-soft);
  display: flex;
  justify-content: flex-end;
  flex-shrink: 0;
}
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
.form-tab .form-card { background: var(--dc-bg-soft); }
</style>
