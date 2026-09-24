<template>
  <div class="obj-form">
    <div class="form-card grow-card">
      <div class="card-title"><el-icon><View /></el-icon>{{ $t('vf.title') }}</div>
      <div class="form-grid">
        <el-form label-position="top" size="small" class="grid-item">
          <!-- 编辑既有视图时锁定名称：改名请用左侧树右键「重命名」 -->
          <el-form-item :label="props.editMode ? $t('vf.nameEdit') : $t('vf.name')" required>
            <el-input v-model="form.name" :disabled="props.editMode" clearable />
          </el-form-item>
        </el-form>
        <el-form v-if="has(features,'supportsComment') && ddlStyleOf(features) !== 'mysql'" label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('vf.comment')">
            <el-input v-model="form.comment" clearable />
          </el-form-item>
        </el-form>
      </div>

      <div class="select-hint">
        <el-icon><InfoFilled /></el-icon>
        <span>{{ $t('vf.hint') }}</span>
      </div>
      <div class="select-editor">
        <SqlCodeEditor
          v-model="form.selectSql"
          :conn-type="connType"
          :tables="tables"
          @mount="onEditorMount"
        />
        <div class="tables-palette">
          <div class="palette-title">
            <el-icon><Grid /></el-icon>{{ $t('vf.loaded') }}
            <el-button text size="small" :icon="Refresh" @click="loadTables">{{ $t('vf.refresh') }}</el-button>
          </div>
          <div class="palette-list">
            <div v-if="!tables.length" class="palette-empty">{{ $t('vf.emptyPre') }}<b>SELECT</b>{{ $t('vf.emptyPost') }}</div>
            <div v-for="t in tables" :key="t" class="palette-item" @click="insertTable(t)">
              <el-icon><Grid /></el-icon>{{ t }}
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, computed, watch, onMounted } from 'vue'
import { ElMessage } from 'element-plus'
import { View, Grid, Refresh, InfoFilled } from '@element-plus/icons-vue'
import { has, qt, sq, ddlStyleOf, extractViewSelect } from './objectFormUtils'
import { t } from '../../utils/i18n'
import { listTables } from '../../api'
import SqlCodeEditor from './SqlCodeEditor.vue'

const props = defineProps({
  features: { type: Object, default: () => ({}) },
  conn: { type: Object, required: true },
  database: { type: String, default: '' },
  editMode: { type: Boolean, default: false },
  editData: { type: Object, default: () => ({}) }
})
const emit = defineEmits(['sql'])

const form = reactive({ name: '', comment: '', selectSql: '' })
const tables = ref([])
const editorInstance = ref(null)
const connType = computed(() => props.conn?.type || '')

const onEditorMount = (editor) => {
  editorInstance.value = editor
}

const loadTables = async () => {
  try {
    tables.value = (await listTables(props.conn.id, props.database) || []).map(t => t.name)
  } catch { tables.value = [] }
}

const insertTable = (t) => {
  const q = qt(props.features, t)
  const line = `SELECT * FROM ${q}`
  const editor = editorInstance.value
  if (editor) {
    const model = editor.getModel()
    if (model) {
      const lastLineNumber = model.getLineCount()
      const lastColumn = model.getLineMaxColumn(lastLineNumber)
      const text = lastColumn === 1 ? line : '\n' + line
      editor.executeEdits('insert-table', [{
        range: {
          startLineNumber: lastLineNumber,
          startColumn: lastColumn,
          endLineNumber: lastLineNumber,
          endColumn: lastColumn
        },
        text
      }])
      editor.focus()
      return
    }
  }
  if (form.selectSql.trim()) form.selectSql += '\n' + line
  else form.selectSql = line
}

const genSql = () => {
  const f = props.features
  const style = ddlStyleOf(f)
  const name = form.name.trim()
  const sel = form.selectSql.trim()
  if (!name || !sel) return ''
  const qn = qt(f, name)
  let head
  if (style === 'sqlite') head = `CREATE VIEW ${qn} AS`
  else if (style === 'mssql') head = (props.editMode ? 'ALTER' : 'CREATE') + ` VIEW ${qn} AS`
  else head = `CREATE OR REPLACE VIEW ${qn} AS`
  let sql = head + '\n' + sel
  if (form.comment && has(f, 'supportsComment')) {
    if (style === 'mysql') {
      // MySQL 原生不支持视图 COMMENT，跳过
    } else if (style === 'mssql') {
      sql += '\n-- 注释: ' + form.comment
    } else if (style === 'clickhouse') {
      // ClickHouse 视图注释用语句内 COMMENT 子句（无 COMMENT ON VIEW）
      sql += `\nCOMMENT '${sq(form.comment)}'`
    } else {
      sql += `\nCOMMENT ON VIEW ${qn} IS '${sq(form.comment)}'`
    }
  }
  return sql + ';'
}

const loadEditData = () => {
  if (!props.editMode) return
  const data = props.editData
  if (data?.name && !form.name) form.name = data.name
  if (data?.ddl) form.selectSql = extractViewSelect(data.ddl)
}

watch(form, () => emit('sql', genSql()), { deep: true })
watch(() => props.editData, loadEditData, { deep: true })

onMounted(() => {
  loadTables()
  loadEditData()
  emit('sql', genSql())
})
</script>

<style scoped>
.obj-form { display: flex; flex-direction: column; flex: 1; min-height: 0; }
.form-card { background: var(--dc-bg-soft); border: 1px solid var(--dc-border-soft); border-radius: 10px; padding: 12px 14px; }
.card-title { display: flex; align-items: center; gap: 6px; font-size: 14px !important; font-weight: 500 !important; color: var(--dc-text); margin-bottom: 10px; }
.card-title .el-icon { color: var(--dc-accent); }
.form-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 0 16px; }
.grid-item { min-width: 0; }
.mb8 { margin-bottom: 8px; }
.select-hint { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--dc-text-dim); margin-bottom: 8px; }
.select-hint .el-icon { color: var(--dc-primary); }
.select-editor { display: flex; gap: 10px; flex: 1; min-height: 0; align-items: stretch; }
.tables-palette { width: 200px; border: 1px solid var(--dc-border-soft); border-radius: 8px; background: var(--dc-bg-code); overflow: hidden; flex-shrink: 0; display: flex; flex-direction: column; }
.palette-title { display: flex; align-items: center; gap: 5px; padding: 8px 10px; font-size: 13px; color: var(--dc-text-dim); border-bottom: 1px solid var(--dc-border-soft); }
.palette-title .el-icon { color: var(--dc-primary); }
.palette-title .el-button { margin-left: auto; }
.palette-list { flex: 1; overflow: auto; padding: 6px; }
.palette-item { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-radius: 6px; font-size: 13px; color: var(--dc-text); cursor: pointer; }
.palette-item:hover { background: var(--dc-bg-hover); color: var(--dc-primary); }
.palette-empty { padding: 14px 8px; font-size: 13px; color: var(--dc-text-dim); }
.grow-card { flex: 1; display: flex; flex-direction: column; min-height: 0; }
</style>
