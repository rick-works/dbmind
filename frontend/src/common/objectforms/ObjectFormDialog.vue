<template>
  <el-dialog
    :model-value="modelValue"
    width="880px"
    top="6vh"
    :close-on-click-modal="false"
    :destroy-on-close="true"
    class="obj-dialog"
    @update:model-value="v => emit('update:modelValue', v)"
    @open="onOpen"
    @closed="onClosed"
  >
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><EditPen /></el-icon></span>
        <span>{{ title }}</span>
      </div>
    </template>
    <div v-loading="loading" class="dialog-body">
      <template v-if="!loading">
        <!-- 事件仅 MySQL/MariaDB 等支持 -->
        <el-alert v-if="cat === 'events' && !has(features, 'supportsEvents')" type="warning" :closable="false" show-icon class="mb12">
          <template #title>{{ $t('ofd.noEventScheduler') }}</template>
        </el-alert>

        <TableForm v-if="cat === 'tables'" :features="features" :edit-mode="mode === 'edit'" @sql="previewSql = $event" />
        <ViewForm v-else-if="cat === 'views'" :features="features" :conn="conn" :database="database"
                  :edit-mode="mode === 'edit'" :edit-data="editData" @sql="previewSql = $event" />
        <IndexForm v-else-if="cat === 'indexes'" :features="features" :conn="conn" :database="database"
                   :edit-mode="mode === 'edit'" :edit-data="editData" @sql="previewSql = $event" />
        <ProcForm v-else-if="cat === 'procs'" :features="features" :edit-mode="mode === 'edit'"
                  :edit-data="editData" @sql="previewSql = $event" />
        <TriggerForm v-else-if="cat === 'triggers'" :features="features" :conn="conn" :database="database"
                     :edit-mode="mode === 'edit'" :edit-data="editData" @sql="previewSql = $event" />
        <EventForm v-else-if="cat === 'events'" :features="features" :edit-mode="mode === 'edit'"
                   :edit-data="editData" @sql="previewSql = $event" />

        <!-- SQL 预览 -->
        <div class="preview-box">
          <div class="preview-head" @click="previewOpen = !previewOpen">
            <el-icon><CaretRight v-if="!previewOpen" /><CaretBottom v-else /></el-icon>
            <span>{{ $t('ofd.sqlPreview') }}</span>
            <el-tag size="small" effect="plain" type="info" class="preview-state"
                    :class="previewOk ? 'ok' : 'warn'">{{ previewOk ? $t('ofd.executable') : $t('ofd.incomplete') }}</el-tag>
            <div class="preview-actions" @click.stop>
              <el-button text size="small" :icon="CopyDocument" @click="copyPreview">{{ $t('ofd.copy') }}</el-button>
            </div>
          </div>
          <pre v-if="previewOpen" class="preview-body"><code>{{ previewSql || $t('ofd.previewPlaceholder') }}</code></pre>
        </div>
      </template>
    </div>

    <template #footer>
      <el-button @click="emit('update:modelValue', false)">{{ $t('common.cancel') }}</el-button>
      <el-button :icon="View" @click="previewOpen = !previewOpen">{{ $t('ofd.preview') }}</el-button>
      <el-button type="primary" :icon="CaretRight" :loading="saving" :disabled="!previewOk" @click="submit">
        {{ mode === 'edit' ? $t('ofd.saveChanges') : $t('ks.create') }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { CaretRight, CaretBottom, CopyDocument, View, EditPen } from '@element-plus/icons-vue'
import { getFeatures, getObjectInfo, alterTable } from '../../api'
import { has } from './objectFormUtils'
import { errMsg } from '../../utils/errMsg'
import { t } from '../../utils/i18n'
import TableForm from './TableForm.vue'
import ViewForm from './ViewForm.vue'
import IndexForm from './IndexForm.vue'
import ProcForm from './ProcForm.vue'
import TriggerForm from './TriggerForm.vue'
import EventForm from './EventForm.vue'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  conn: { type: Object, required: true },
  database: { type: String, default: '' },
  cat: { type: String, required: true },   // tables / views / indexes / procs / triggers / events
  mode: { type: String, default: 'create' }, // create / edit
  object: { type: Object, default: () => ({}) } // edit 模式: { name, table }
})
const emit = defineEmits(['update:modelValue', 'saved'])

const features = ref({})
const editData = ref({})
const previewSql = ref('')
const previewOpen = ref(true)
const loading = ref(false)
const saving = ref(false)

const CAT_LABELS = { tables: 'mv.catTable', views: 'mv.catView', indexes: 'mv.catIndex', procs: 'mv.catProc', triggers: 'mv.catTrigger', events: 'mv.catEvent' }

const title = computed(() => {
  let base = CAT_LABELS[props.cat] || props.cat
  if (props.cat === 'procs' && (props.object?.kind === 'function' || props.conn?.type === 'CLICKHOUSE')) base = t('mv.catFunc')
  if (props.mode === 'edit') return t('mv.editLabel', { kind: base, name: (props.object.name || '') })
  return t('ofd.newPrefix', { kind: base })
})

const previewOk = computed(() => {
  const s = previewSql.value
  return !!s && !/^--/.test(s.trim()) && s.trim().length > 0
})

const loadFeatures = async () => {
  try {
    features.value = await getFeatures(props.conn.id)
  } catch {
    features.value = {}
  }
}

const loadEditData = async () => {
  if (props.mode !== 'edit' || !props.object.name) return
  try {
    const res = await getObjectInfo(props.conn.id, props.database, props.object.kind || props.cat, props.object.name)
    editData.value = res || {}
  } catch (e) {
    editData.value = { ddl: '' }
    ElMessage.warning(t('ofd.loadDefFailed', { detail: (e?.message || e) }))
  }
}

const onOpen = async () => {
  previewSql.value = ''
  previewOpen.value = true
  loading.value = true
  try {
    await loadFeatures()
    await loadEditData()
  } finally {
    loading.value = false
  }
}

const onClosed = () => {
  previewSql.value = ''
  editData.value = {}
}

const copyPreview = async () => {
  if (!previewSql.value) return
  try {
    await navigator.clipboard.writeText(previewSql.value)
    ElMessage.success(t('ofd.sqlCopied'))
  } catch {
    ElMessage.warning(t('ofd.copyManual'))
  }
}

const submit = async () => {
  if (!previewOk.value) return ElMessage.warning(t('ofd.fillFirst'))
  saving.value = true
  try {
    const res = await alterTable(props.conn.id, props.database, previewSql.value)
    if (res.success) {
      ElMessage.success(t('ofd.opSuccess', { op: (props.mode === 'edit' ? $t('ofd.opEdit') : $t('ks.create')) }))
      emit('update:modelValue', false)
      emit('saved')
    } else {
      ElMessage.error(t('ofd.execFailed', { detail: (res.message || t('common.unknownError')) }))
    }
  } catch (e) {
    ElMessage.error(t('ofd.execFailed', { detail: errMsg(e) }))
  } finally {
    saving.value = false
  }
}
</script>

<style scoped>
.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}

.dialog-body { min-height: 120px; }
.mb12 { margin-bottom: 12px; }
.preview-box { margin-top: 12px; border: 1px solid var(--dc-border-soft); border-radius: 10px; background: var(--dc-bg-code); overflow: hidden; }
.preview-head { display: flex; align-items: center; gap: 6px; padding: 8px 12px; font-size: 13px; color: var(--dc-text-dim); cursor: pointer; user-select: none; }
.preview-head:hover { color: var(--dc-text); }
.preview-state { font-size: 12px; }
.preview-state.ok { --el-tag-bg-color: rgba(61,220,151,.1); --el-tag-border-color: rgba(61,220,151,.4); --el-tag-text-color: var(--dc-accent); }
.preview-state.warn { --el-tag-bg-color: rgba(245,179,77,.1); --el-tag-border-color: rgba(245,179,77,.4); --el-tag-text-color: #f5b34d; }
.preview-actions { margin-left: auto; }
.preview-body { max-height: 180px; overflow: auto; padding: 10px 12px; border-top: 1px solid var(--dc-border-soft); }
.preview-body code { font-family: "SF Mono", Consolas, monospace; font-size: 13px; line-height: 1.6; color: #a5c8ff; white-space: pre-wrap; word-break: break-all; }
</style>

<style>
.obj-dialog .el-dialog__body { padding: 14px 16px 6px; display: flex; flex-direction: column; max-height: calc(94vh - 120px); }
.obj-dialog .dialog-body { display: flex; flex-direction: column; flex: 1; }
.obj-dialog .dialog-body > .obj-form { flex: 1; display: flex; flex-direction: column; min-height: 0; }
.obj-dialog .dialog-body > .preview-box { flex: none; }
</style>
