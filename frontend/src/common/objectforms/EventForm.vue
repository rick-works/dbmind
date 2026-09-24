<template>
  <div class="obj-form">
    <div class="form-card">
      <div class="card-title"><el-icon><Timer /></el-icon>{{ $t('evf.title') }}</div>
      <div class="form-grid">
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="editMode ? $t('evf.nameEdit') : $t('evf.name')" required>
            <el-input v-model="form.name" :disabled="editMode" clearable />
          </el-form-item>
        </el-form>
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('evf.schedule')" required>
            <el-radio-group v-model="form.scheduleType" size="small">
              <el-radio-button value="interval">{{ $t('evf.interval') }}</el-radio-button>
              <el-radio-button value="once">{{ $t('evf.once') }}</el-radio-button>
            </el-radio-group>
          </el-form-item>
        </el-form>
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('evf.status')">
            <el-select v-model="form.status" style="width:100%">
              <el-option :label="$t('evf.enabled')" value="ENABLED" />
              <el-option :label="$t('evf.disabled')" value="DISABLED" />
            </el-select>
          </el-form-item>
        </el-form>
      </div>

      <template v-if="form.scheduleType === 'interval'">
        <div class="schedule-line">
          <span class="schedule-label">{{ $t('evf.every') }}</span>
          <el-input-number v-model="form.everyValue" :min="1" size="small" style="width:90px" />
          <el-select v-model="form.everyUnit" size="small" style="width:110px">
            <el-option :label="$t('evf.sec')" value="SECOND" />
            <el-option :label="$t('evf.minute')" value="MINUTE" />
            <el-option :label="$t('evf.hour')" value="HOUR" />
            <el-option :label="$t('evf.day')" value="DAY" />
            <el-option :label="$t('evf.week')" value="WEEK" />
            <el-option :label="$t('evf.month')" value="MONTH" />
            <el-option :label="$t('evf.year')" value="YEAR" />
          </el-select>
          <span class="schedule-label">{{ $t('evf.everyRun') }}</span>
        </div>
        <div class="schedule-line">
          <span class="schedule-label">{{ $t('evf.startAt') }}</span>
          <el-date-picker v-model="form.startsAt" type="datetime" size="small"
                          value-format="YYYY-MM-DD HH:mm:ss" style="width:200px" />
          <span class="schedule-label">{{ $t('evf.endAt') }}</span>
          <el-date-picker v-model="form.endsAt" type="datetime" size="small"
                          value-format="YYYY-MM-DD HH:mm:ss" style="width:200px" />
        </div>
      </template>
      <template v-else>
        <div class="schedule-line">
          <span class="schedule-label">{{ $t('evf.runAt') }}</span>
          <el-date-picker v-model="form.atTime" type="datetime" size="small"
                          value-format="YYYY-MM-DD HH:mm:ss" style="width:220px" />
        </div>
      </template>
    </div>

    <div class="form-card grow-card">
      <div class="card-title"><el-icon><EditPen /></el-icon>{{ $t('evf.body') }}</div>
      <SqlCodeEditor v-model="form.body" :conn-type="connType" />
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, computed, watch, onMounted } from 'vue'
import { ElMessage } from 'element-plus'
import { Timer, EditPen } from '@element-plus/icons-vue'
import { qt, sq, ddlStyleOf, extractRoutineBody } from './objectFormUtils'
import { t } from '../../utils/i18n'
import SqlCodeEditor from './SqlCodeEditor.vue'

const stripFrac = (s) => String(s || '').replace(/\.\d+$/, '')

const props = defineProps({
  features: { type: Object, default: () => ({}) },
  conn: { type: Object, default: () => ({}) },
  editMode: { type: Boolean, default: false },
  editData: { type: Object, default: () => ({}) }
})
const emit = defineEmits(['sql'])

const style = computed(() => ddlStyleOf(props.features))
const connType = computed(() => props.conn?.type || '')

const form = reactive({
  name: '', scheduleType: 'interval', status: 'ENABLED',
  everyValue: 1, everyUnit: 'DAY', startsAt: '', endsAt: '', atTime: '',
  body: ''
})

const genSql = () => {
  const f = props.features
  const name = form.name.trim()
  if (!name) return ''
  const qn = qt(f, name)
  let schedule
  if (form.scheduleType === 'once') {
    if (!form.atTime) return ''
    schedule = `AT '${form.atTime}'`
  } else {
    schedule = `EVERY ${form.everyValue || 1} ${form.everyUnit}`
    if (form.startsAt) schedule += ` STARTS '${form.startsAt}'`
    if (form.endsAt) schedule += ` ENDS '${form.endsAt}'`
  }
  // 编辑既有事件用 ALTER EVENT 原地修改（保留权限与调度状态）；改名走树的右键「重命名」
  const editInPlace = props.editMode && style.value === 'mysql'
  let sql = `${editInPlace ? 'ALTER' : 'CREATE'} EVENT ${qn}\nON SCHEDULE ${schedule}\nON COMPLETION PRESERVE\n${form.status === 'DISABLED' ? 'DISABLE' : 'ENABLE'}\nDO\n${form.body.trim() || t('evf.noBody')}`
  const stmts = []
  if (props.editMode && !editInPlace) stmts.push(`DROP EVENT IF EXISTS ${qn}`)
  stmts.push(sql)
  return stmts.join(';\n') + ';'
}

watch(form, () => emit('sql', genSql()), { deep: true })
onMounted(() => {
  if (props.editMode && props.editData.ddl) {
    const ddl = props.editData.ddl
    const m = props.editData.meta || {}
    form.name = m.name || props.editData.name || ''
    form.body = extractRoutineBody(ddl)
    if (m.status) form.status = String(m.status).toUpperCase()
    const sched = ddl.match(/ON\s+SCHEDULE\s+([\s\S]*?)(?:\s+ON\s+COMPLETION|\s+ENABLE|\s+DISABLE|\s+DO\b)/i)
    if (sched) {
      const s = sched[1]
      if (/\bEVERY\b/i.test(s)) {
        form.scheduleType = 'interval'
        const ev = s.match(/\bEVERY\s+(\d+)\s+(\w+)/i)
        if (ev) { form.everyValue = Number(ev[1]) || 1; form.everyUnit = ev[2].toUpperCase() }
        const st = s.match(/\bSTARTS\s+'([^']+)'/i)
        if (st) form.startsAt = stripFrac(st[1])
        const en = s.match(/\bENDS\s+'([^']+)'/i)
        if (en) form.endsAt = stripFrac(en[1])
      } else {
        form.scheduleType = 'once'
        const at = s.match(/\bAT\s+'([^']+)'/i)
        if (at) form.atTime = stripFrac(at[1])
      }
    }
  }
  emit('sql', genSql())
})
</script>

<style scoped>
.obj-form { display: flex; flex-direction: column; gap: 12px; flex: 1; min-height: 0; }
.form-card { background: var(--dc-bg-soft); border: 1px solid var(--dc-border-soft); border-radius: 10px; padding: 12px 14px; }
.card-title { display: flex; align-items: center; gap: 6px; font-size: 14px !important; font-weight: 500 !important; color: var(--dc-text); margin-bottom: 10px; }
.card-title .el-icon { color: #06b6d4; }
.form-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 0 16px; }
.grid-item { min-width: 0; }
.schedule-line { display: flex; align-items: center; gap: 10px; margin: 10px 0; }
.schedule-label { font-size: 13px; color: var(--dc-text-dim); white-space: nowrap; }
.grow-card { flex: 1; display: flex; flex-direction: column; min-height: 0; }
.grow-card > .sql-code-editor { flex: 1; }
</style>
