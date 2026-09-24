<template>
  <div class="obj-form">
    <div class="form-card">
      <div class="card-title"><el-icon><BellFilled /></el-icon>{{ $t('tgf.title') }}</div>
      <div class="form-grid">
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="editMode ? $t('tgf.nameEdit') : $t('tgf.name')" required>
            <el-input v-model="form.name" :disabled="editMode" clearable />
          </el-form-item>
        </el-form>
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('tgf.timing')" required>
            <el-select v-model="form.timing" style="width:100%">
              <el-option :label="$t('tgf.before')" value="BEFORE" />
              <el-option :label="$t('tgf.after')" value="AFTER" />
              <el-option v-if="style === 'oracle'" label="INSTEAD OF" value="INSTEAD OF" />
            </el-select>
          </el-form-item>
        </el-form>
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('tgf.event')" required>
            <el-select v-model="form.events" multiple style="width:100%">
              <el-option label="INSERT" value="INSERT" />
              <el-option label="UPDATE" value="UPDATE" />
              <el-option label="DELETE" value="DELETE" />
            </el-select>
          </el-form-item>
        </el-form>
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('tgf.table')" required>
            <el-select v-model="form.table" filterable :disabled="editMode">
              <el-option v-for="t in tables" :key="t" :label="t" :value="t" />
            </el-select>
          </el-form-item>
        </el-form>
      </div>
      <el-alert v-if="style === 'pg'" type="info" :closable="false" show-icon class="mt8">
        <template #title>{{ $t('tgf.pgTip') }}</template>
      </el-alert>
    </div>

    <div class="form-card grow-card">
      <div class="card-title"><el-icon><EditPen /></el-icon>{{ $t('tgf.body') }}</div>
      <el-alert v-if="style === 'sqlite'" type="warning" :closable="false" show-icon class="mb8">
        <template #title>{{ $t('tgf.sqliteTip') }}</template>
      </el-alert>
      <SqlCodeEditor v-model="form.body" :conn-type="connType" />
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, computed, watch, onMounted } from 'vue'
import { ElMessage } from 'element-plus'
import { BellFilled, EditPen } from '@element-plus/icons-vue'
import { qt, ddlStyleOf, extractRoutineBody, ensureRoutineBodyEnd } from './objectFormUtils'
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

const style = computed(() => ddlStyleOf(props.features))
const connType = computed(() => props.conn?.type || '')

const form = reactive({ name: '', timing: 'BEFORE', events: ['INSERT'], table: '', body: '' })
const tables = ref([])

const loadTables = async () => {
  try {
    tables.value = (await listTables(props.conn.id, props.database) || []).map(t => t.name)
  } catch { tables.value = [] }
}

const genSql = () => {
  const f = props.features
  const s = style.value
  const name = form.name.trim()
  if (!name || !form.table) return ''
  if (!form.events.length) return ''
  const qn = qt(f, name)
  const qtbl = qt(f, form.table)
  // Oracle / PostgreSQL 触发事件用 OR 连接；MySQL / SQL Server / SQLite 用逗号连接
  const eventSep = (s === 'oracle' || s === 'pg') ? ' OR ' : ', '
  const events = form.events.join(eventSep)

  // 触发逻辑体内语句需以分号结束，末尾自动补分号
  const body = ensureRoutineBodyEnd(form.body)
  let create
  if (s === 'pg') {
    create = `CREATE TRIGGER ${qn} ${form.timing} ${events} ON ${qtbl}\nFOR EACH ROW EXECUTE FUNCTION ${qn}()`
  } else if (s === 'mssql') {
    // SQL Server 不支持 BEFORE，只允许 AFTER / INSTEAD OF；编辑时用 ALTER 原地修改
    const timing = form.timing === 'BEFORE' ? 'AFTER' : form.timing
    const verb = props.editMode ? 'ALTER' : 'CREATE'
    create = `${verb} TRIGGER ${qn} ON ${qtbl}\n${timing} ${events}\nAS\nBEGIN\n${body}\nEND`
  } else if (s === 'oracle') {
    create = `CREATE OR REPLACE TRIGGER ${qn} ${form.timing} ${events} ON ${qtbl}\nFOR EACH ROW\nBEGIN\n${body}\nEND`
  } else {
    create = `CREATE TRIGGER ${qn} ${form.timing} ${events} ON ${qtbl}\nFOR EACH ROW\nBEGIN\n${body}\nEND`
  }

  const stmts = []
  // SQL Server(ALTER TRIGGER) / Oracle(CREATE OR REPLACE) 编辑时都是原地替换，不需要 DROP
  //（顺带避开 SQL Server 2012 不支持 DROP TRIGGER IF EXISTS 的问题）
  if (props.editMode && s !== 'oracle' && s !== 'mssql') {
    stmts.push(`DROP TRIGGER IF EXISTS ${qn}`)
  }
  stmts.push(create)
  return stmts.join(';\n') + ';'
}

watch(form, () => emit('sql', genSql()), { deep: true })
onMounted(async () => {
  await loadTables()
  if (props.editMode) {
    const ddl = props.editData.ddl || ''
    const m = props.editData.meta || {}
    form.name = m.name || props.editData.name || ''
    form.body = extractRoutineBody(ddl)
    if (m.timing) form.timing = String(m.timing).toUpperCase()
    if (m.event) form.events = String(m.event).split(',').map(e => e.trim().toUpperCase()).filter(Boolean)
    if (m.table) form.table = m.table
    if (!form.table) {
      const tm = ddl.match(/\bON\s+["`\[]?([A-Za-z0-9_$]+)["`\]]?/i)
      if (tm) form.table = tm[1]
    }
    const em = ddl.match(/\b(BEFORE|AFTER|INSTEAD\s+OF)\s+((?:INSERT|UPDATE|DELETE)(?:\s*(?:OR|,)\s*(?:INSERT|UPDATE|DELETE))*)/i)
    if (em) {
      form.timing = em[1].replace(/\s+/g, ' ').toUpperCase()
      const evs = em[2].split(/\s*(?:OR|,)\s*/i).map(e => e.trim().toUpperCase()).filter(Boolean)
      if (evs.length) form.events = evs
    }
  } else if (tables.value.length) {
    form.table = tables.value[0]
  }
  emit('sql', genSql())
})
</script>

<style scoped>
.obj-form { display: flex; flex-direction: column; flex: 1; min-height: 0; }
.form-card { background: var(--dc-bg-soft); border: 1px solid var(--dc-border-soft); border-radius: 10px; padding: 12px 14px; }
.grow-card { flex: 1; display: flex; flex-direction: column; min-height: 0; }
.card-title { display: flex; align-items: center; gap: 6px; font-size: 14px !important; font-weight: 500 !important; color: var(--dc-text); margin-bottom: 10px; }
.card-title .el-icon { color: #ef4444; }
.form-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 0 16px; }
.grid-item { min-width: 0; }
.mt8 { margin-top: 8px; }
.mb8 { margin-bottom: 8px; }
.grow-card > .sql-code-editor { flex: 1; }
</style>
