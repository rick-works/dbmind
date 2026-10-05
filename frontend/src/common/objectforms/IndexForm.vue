<template>
  <div class="obj-form">
    <div class="form-card">
      <div class="card-title"><el-icon><Files /></el-icon>{{ $t('ixf.title') }}</div>
      <div class="form-grid">
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('tf.idxName')" required>
            <el-input v-model="form.name" clearable />
          </el-form-item>
        </el-form>
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('tgf.table')" required>
            <el-select v-model="form.table" filterable :disabled="editMode" @change="onTableChange">
              <el-option v-for="t in tables" :key="t" :label="t" :value="t" />
            </el-select>
          </el-form-item>
        </el-form>
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('ixf.type')">
            <el-select v-model="form.indexType" clearable filterable>
              <el-option v-for="t in features.indexTypes || []" :key="t" :label="t" :value="t" />
            </el-select>
          </el-form-item>
        </el-form>
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('ixf.unique')">
            <el-switch v-model="form.unique" />
          </el-form-item>
        </el-form>
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('ixf.columns')" required>
            <el-select v-model="form.columns" multiple filterable style="width:100%">
              <el-option v-for="c in cols" :key="c" :label="c" :value="c" />
            </el-select>
          </el-form-item>
        </el-form>
      </div>
      <el-alert v-if="editMode" type="warning" :closable="false" show-icon class="mt8">
        <template #title>{{ $t('ixf.rebuildTip') }}</template>
      </el-alert>
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, computed, watch, onMounted } from 'vue'
import { ElMessage } from 'element-plus'
import { Files } from '@element-plus/icons-vue'
import { has, qt, ddlStyleOf } from './objectFormUtils'
import { t } from '../../utils/i18n'
import { listTables, listColumns } from '../../api'

const props = defineProps({
  features: { type: Object, default: () => ({}) },
  conn: { type: Object, required: true },
  database: { type: String, default: '' },
  editMode: { type: Boolean, default: false },
  editData: { type: Object, default: () => ({}) }
})
const emit = defineEmits(['sql'])

const form = reactive({ name: '', table: '', columns: [], indexType: '', unique: false })
const tables = ref([])
const cols = ref([])

const loadTables = async () => {
  try {
    tables.value = (await listTables(props.conn.id, props.database) || []).map(t => t.name)
  } catch { tables.value = [] }
}

const onTableChange = async (t) => {
  form.columns = []
  if (!t) return
  try {
    cols.value = (await listColumns(props.conn.id, props.database, t) || []).map(c => c.name)
  } catch { cols.value = [] }
}

const genSql = () => {
  const f = props.features
  const style = ddlStyleOf(f)
  const name = form.name.trim()
  const table = form.table
  if (!name || !table) return ''
  if (!form.columns.length) return ''
  const qn = qt(f, name)
  const qtbl = qt(f, table)
  const qcols = form.columns.map(c => qt(f, c))
  const it = (form.indexType || '').trim()
  const unique = form.unique ? 'UNIQUE ' : ''
  let create
  if (style === 'mysql') {
    // FULLTEXT / SPATIAL 是关键字前缀；BTREE / HASH 用 USING（MySQL 要求位于 ON 之前）
    const up = it.toUpperCase()
    if (up === 'FULLTEXT' || up === 'SPATIAL') {
      create = `CREATE ${up} INDEX ${qn} ON ${qtbl} (${qcols.join(', ')})`
    } else {
      create = `CREATE ${unique}INDEX ${qn}`
      if (up) create += ` USING ${up}`
      create += ` ON ${qtbl} (${qcols.join(', ')})`
    }
  } else if (style === 'mssql') {
    // CLUSTERED / NONCLUSTERED 放在 INDEX 关键字之前
    const t = it ? it.toUpperCase() + ' ' : ''
    create = `CREATE ${unique}${t}INDEX ${qn} ON ${qtbl} (${qcols.join(', ')})`
  } else if (style === 'pg') {
    // PostgreSQL 的 USING 必须位于列列表之前
    create = `CREATE ${unique}INDEX ${qn} ON ${qtbl}`
    if (it) create += ` USING ${it.toLowerCase()}`
    create += ` (${qcols.join(', ')})`
  } else {
    // sqlite / oracle / db2 / dm / h2 / derby 等通用 ANSI 语法
    create = `CREATE ${unique}INDEX ${qn} ON ${qtbl} (${qcols.join(', ')})`
  }
  if (props.editMode) {
    const drop = style === 'mysql' || style === 'mssql'
      ? `DROP INDEX ${qn} ON ${qtbl}`
      : `DROP INDEX ${qn}`
    return drop + ';\n' + create + ';'
  }
  return create + ';'
}

watch(form, () => emit('sql', genSql()), { deep: true })
onMounted(async () => {
  await loadTables()
  if (props.editMode) {
    const m = props.editData.meta || {}
    form.name = (m.name || '').split('@@')[0]
    form.table = m.table || ''
    form.unique = !!m.unique
    form.indexType = m.type || ''
    form.columns = Array.isArray(m.columns) ? m.columns : (m.columns ? String(m.columns).split(',').map(s => s.trim()) : [])
    if (form.table) await onTableChange(form.table)
    if (form.table && !cols.value.length && form.columns.length) cols.value = [...form.columns]
  } else if (tables.value.length) {
    form.table = tables.value[0]
    await onTableChange(form.table)
  }
  emit('sql', genSql())
})
</script>

<style scoped>
.obj-form { display: flex; flex-direction: column; gap: 12px; }
.form-card { background: var(--dc-bg-soft); border: 1px solid var(--dc-border-soft); border-radius: 10px; padding: 12px 14px; }
.card-title { display: flex; align-items: center; gap: 6px; font-size: 14px !important; font-weight: 500 !important; color: var(--dc-text); margin-bottom: 10px; }
.card-title .el-icon { color: #a78bfa; }
.form-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 0 16px; }
.grid-item { min-width: 0; }
.mt8 { margin-top: 8px; }
</style>
