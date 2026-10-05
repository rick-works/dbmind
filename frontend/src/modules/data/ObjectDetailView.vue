<template>
  <div class="obj-detail">
    <div v-if="loading" class="loading">
      <el-icon class="is-loading"><Loading /></el-icon> {{ $t('common.loading') }}
    </div>
    <div v-else-if="error" class="error">
      <el-icon><CircleClose /></el-icon> {{ error }}
    </div>
    <template v-else>
      <!-- 顶部：对象基本信息 -->
      <div class="meta-card">
        <div class="meta-header">
          <el-icon :size="20" :color="iconColor"><component :is="icon" /></el-icon>
          <span class="meta-title">{{ meta.name || info.name }}</span>
          <el-tag size="small" effect="plain">{{ typeLabel }}</el-tag>
          <el-tag v-if="info.table && info.table !== meta.name" size="small" effect="plain" type="info">
            {{ $t('odv.fTable') }}: {{ info.table }}
          </el-tag>
        </div>
        <div class="meta-grid">
          <div v-for="(v, k) in meta" :key="k" class="meta-item">
            <span class="meta-k">{{ labelOf(k) }}</span>
            <span class="meta-v">{{ formatMeta(k, v) }}</span>
          </div>
        </div>
      </div>

      <!-- DDL 区 -->
      <div class="ddl-card">
        <div class="ddl-header">
          <el-icon><DocumentCopy /></el-icon>
          <span>{{ $t('odv.ddlSection') }}</span>
          <div class="ddl-actions">
            <el-button v-if="canVisualEdit" type="primary" plain size="small" :icon="EditPen" @click="onEditClick">{{ $t('odv.visualEdit') }}</el-button>
            <el-button type="primary" plain size="small" :icon="CopyDocument" @click="copyDdl">{{ $t('common.copy') }}</el-button>
          </div>
        </div>
        <pre class="ddl-body"><code>{{ ddl }}</code></pre>
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { Loading, CircleClose, Cpu, Setting, Operation, BellFilled, Timer, Files, DocumentCopy, CopyDocument, EditPen } from '@element-plus/icons-vue'
import { getObjectInfo } from '../../api'
import { t } from '../../utils/i18n'

const props = defineProps({
  conn: { type: Object, required: true },
  database: { type: String, default: '' },
  type: { type: String, required: true }, // index / procedure / trigger / event
  name: { type: String, required: true }   // 索引为 "idxName@@tableName" 形式
})

const emit = defineEmits(['edit'])

const info = ref({})
const meta = ref({})
const ddl = ref('')
const loading = ref(false)
const error = ref('')

// 对象类型 → 表单分类（ObjectFormDialog）
const catOf = (t) => ({ index: 'indexes', function: 'procs', procedure: 'procs', trigger: 'triggers', event: 'events' }[t] || 'indexes')

// 仅索引保留「可视化编辑」入口；视图/存储过程/函数/触发器/事件不再提供
const canVisualEdit = computed(() => props.type === 'index')

const onEditClick = () => {
  emit('edit', {
    cat: catOf(props.type),
    name: props.name,
    kind: props.type,
    table: info.value.table || ''
  })
}

const typeLabel = computed(() => {
  const metaType = info.value.meta?.type
  if (metaType) {
    // 局部变量不能叫 t：本函数要用 t() 翻译，同名会遮蔽
    const ty = String(metaType).toUpperCase()
    if (ty === 'FUNCTION' || ty === 'FN' || ty === 'IF' || ty === 'TF') return t('mv.catFunc')
    if (ty === 'PROCEDURE' || ty === 'PROCEDURE' || ty === 'P' || ty === 'PC') return t('mv.catProc')
    if (ty === 'TRIGGER' || ty === 'TR') return t('mv.catTrigger')
    if (ty === 'EVENT') return t('mv.catEvent')
    if (ty === 'INDEX') return t('mv.catIndex')
  }
  return {
    index: t('mv.catIndex'), function: t('mv.catFunc'), procedure: t('mv.catProc'), trigger: t('mv.catTrigger'), event: t('mv.catEvent')
  }[props.type] || props.type
})

const icon = computed(() => ({
  index: Files, function: Cpu, procedure: Setting, trigger: BellFilled, event: Timer
}[props.type] || Files))

const iconColor = computed(() => ({
  index: '#a78bfa', function: '#3b82f6', procedure: '#f59e0b', trigger: '#ef4444', event: '#06b6d4'
}[props.type] || '#888'))

const labelOf = (k) => ({
  name: t('odv.fName'), table: t('odv.fTable'), unique: t('odv.fUnique'), type: t('odv.fType'), columns: t('odv.fColumns'),
  returnType: t('odv.fReturnType'), created: t('odv.fCreated'), modified: t('odv.fModified'),
  definer: t('odv.fDefiner'), timing: t('odv.fTiming'), event: t('odv.fEvent'),
  status: t('odv.fStatus'), schedule: t('odv.fSchedule'), starts: t('odv.fStarts'), ends: t('odv.fEnds'),
  lang: t('odv.fLang'), args: t('odv.fArgs')
}[k] || k)

const formatMeta = (k, v) => {
  if (v == null || v === '') return v
  const s = String(v)
  // 去掉日期时间字符串中无意义的毫秒/小数部分，例如：
  // 2026-09-09 16:09:34.17  => 2026-09-09 16:09:34
  // 于 2026-09-10 00:30:00.0 => 于 2026-09-10 00:30:00
  return s.replace(/(\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2})(\.\d+)/g, '$1')
}

const load = async () => {
  if (!props.conn || !props.name) return
  loading.value = true
  error.value = ''
  try {
    const res = await getObjectInfo(props.conn.id, props.database, props.type, props.name)
    info.value = res
    meta.value = res.meta || {}
    ddl.value = res.ddl || t('odv.noDdl')
  } catch (e) {
    error.value = t('odv.loadFailedDetail', { detail: e.message })
  } finally {
    loading.value = false
  }
}

const copyDdl = async () => {
  try {
    await navigator.clipboard.writeText(ddl.value || '')
    ElMessage.success(t('odv.copied'))
  } catch {
    ElMessage.warning(t('mv.copyManually'))
  }
}

watch(() => [props.conn?.id, props.database, props.type, props.name], load, { immediate: true })
</script>

<style scoped>
.obj-detail { height: 100%; display: flex; flex-direction: column; gap: 12px; padding: 16px; overflow: auto; background: var(--dc-bg-deep); }
.loading, .error { padding: 40px; text-align: center; color: var(--dc-text-dim); display: flex; align-items: center; justify-content: center; gap: 8px; }
.error { color: var(--dc-danger); }
.meta-card, .ddl-card { background: var(--dc-bg-card); border: 1px solid var(--dc-border); border-radius: 8px; padding: 14px 16px; }
.meta-header { display: flex; align-items: center; gap: 10px; margin-bottom: 12px; }
.meta-title { font-size: 15px; font-weight: 700; color: var(--dc-text); }
.meta-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 8px 24px; }
.meta-item { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.meta-k { font-size: 12px; color: var(--dc-text-dim); text-transform: uppercase; letter-spacing: .5px; }
.meta-v { font-size: 14px; color: var(--dc-text); font-family: "SF Mono", Consolas, monospace; word-break: break-all; }
.ddl-header { display: flex; align-items: center; gap: 6px; color: var(--dc-text-dim); font-size: 13px; margin-bottom: 8px; }
.ddl-header span { flex: 1; }
.ddl-actions { display: flex; align-items: center; gap: 2px; }
.ddl-body { background: var(--dc-bg-code); border: 1px solid var(--dc-border); border-radius: 6px; padding: 12px; overflow: auto; max-height: calc(100vh - 280px); }
.ddl-body code { font-family: "SF Mono", Consolas, monospace; font-size: 13px; color: var(--dc-code-text); line-height: 1.6; white-space: pre; }
</style>