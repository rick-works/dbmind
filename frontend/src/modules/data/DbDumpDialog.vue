<template>
  <el-dialog v-model="visible" width="720px" :close-on-click-modal="false" append-to-body class="main-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Download /></el-icon></span>
        <span>{{ $t('dump.title') }}</span>
      </div>
    </template>
    <el-form label-width="80px" size="small" v-loading="loading" :element-loading-text="$t('dump.loadingObjects')">
        <!-- 导出格式 -->
        <el-form-item :label="$t('dump.exportFormat')">
          <el-radio-group v-model="format" @change="onFormatChange">
            <el-radio-button value="sql">{{ $t('dump.fmtSql') }}</el-radio-button>
            <el-radio-button value="sql-ddl">{{ $t('dump.fmtDdl') }}</el-radio-button>
          </el-radio-group>
        </el-form-item>

        <!-- 通用选项 -->
        <el-form-item :label="$t('dump.commonOptions')">
          <div style="display:flex;flex-wrap:wrap;gap:4px 14px;">
            <el-checkbox v-model="opts.includeDrop">DROP IF EXISTS</el-checkbox>
            <el-checkbox v-model="opts.includeData" :disabled="format === 'sql-ddl'">{{ $t('dump.includeData') }}</el-checkbox>
          </div>
        </el-form-item>

        <!-- 对象类型选择 -->
        <el-form-item :label="$t('dump.objectTypes')">
          <el-checkbox-group v-model="objTypes" style="display:flex;flex-wrap:wrap;gap:4px 14px;">
            <el-checkbox value="table">{{ $t('dump.objTable') }}</el-checkbox>
            <el-checkbox value="view">{{ $t('dump.objView') }}</el-checkbox>
            <el-checkbox value="function">{{ $t('dump.objFunction') }}</el-checkbox>
            <el-checkbox value="procedure">{{ $t('dump.objProcedure') }}</el-checkbox>
            <el-checkbox value="trigger">{{ $t('dump.objTrigger') }}</el-checkbox>
            <el-checkbox value="event">{{ $t('dump.objEvent') }}</el-checkbox>
          </el-checkbox-group>
        </el-form-item>

        <!-- 对象列表（按类型分 Tab） -->
        <el-form-item :label="$t('dump.chooseObjects')">
          <el-tabs v-model="activeTab" type="border-card" size="small" style="width:100%;background:transparent;" class="dump-tabs">
            <el-tab-pane v-if="objTypes.includes('table')" :label="$t('dump.objTable')" name="table">
              <div style="max-height:200px;overflow:auto;padding:4px;">
                <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px;">
                  <el-checkbox v-model="allTable" @change="toggleAllByType('table', $event)" style="margin-right:auto;">{{ $t('common.selectAll') }}</el-checkbox>
                  <span style="color:var(--dc-muted);font-size: 13px;">{{ $t('dmp.selected', { n: selectedTable.length, total: tables.length }) }}</span>
                </div>
                <el-divider style="margin:6px 0" />
                <el-checkbox-group v-model="selectedTable" style="display:flex;flex-direction:column;gap:3px;">
                  <el-checkbox v-for="t in tables" :key="t" :value="t" style="margin:0;">{{ t }}</el-checkbox>
                </el-checkbox-group>
              </div>
            </el-tab-pane>
            <el-tab-pane v-if="objTypes.includes('view')" :label="$t('dump.objView')" name="view">
              <div style="max-height:200px;overflow:auto;padding:4px;">
                <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px;">
                  <el-checkbox v-model="allView" @change="toggleAllByType('view', $event)" style="margin-right:auto;">{{ $t('common.selectAll') }}</el-checkbox>
                  <span style="color:var(--dc-muted);font-size: 13px;">{{ $t('dmp.selected', { n: selectedView.length, total: views.length }) }}</span>
                </div>
                <el-divider style="margin:6px 0" />
                <el-checkbox-group v-model="selectedView" style="display:flex;flex-direction:column;gap:3px;">
                  <el-checkbox v-for="t in views" :key="t" :value="t" style="margin:0;">{{ t }}</el-checkbox>
                </el-checkbox-group>
              </div>
            </el-tab-pane>
            <el-tab-pane v-if="objTypes.includes('function')" :label="$t('dump.objFunction')" name="function">
              <div style="max-height:200px;overflow:auto;padding:4px;">
                <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px;">
                  <el-checkbox v-model="allFunc" @change="toggleAllByType('function', $event)" style="margin-right:auto;">{{ $t('common.selectAll') }}</el-checkbox>
                  <span style="color:var(--dc-muted);font-size: 13px;">{{ $t('dmp.selected', { n: selectedFunc.length, total: funcs.length }) }}</span>
                </div>
                <el-divider style="margin:6px 0" />
                <el-checkbox-group v-model="selectedFunc" style="display:flex;flex-direction:column;gap:3px;">
                  <el-checkbox v-for="t in funcs" :key="t" :value="t" style="margin:0;">{{ t }}</el-checkbox>
                </el-checkbox-group>
              </div>
            </el-tab-pane>
            <el-tab-pane v-if="objTypes.includes('procedure')" :label="$t('dump.objProcedure')" name="procedure">
              <div style="max-height:200px;overflow:auto;padding:4px;">
                <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px;">
                  <el-checkbox v-model="allProc" @change="toggleAllByType('procedure', $event)" style="margin-right:auto;">{{ $t('common.selectAll') }}</el-checkbox>
                  <span style="color:var(--dc-muted);font-size: 13px;">{{ $t('dmp.selected', { n: selectedProc.length, total: procs.length }) }}</span>
                </div>
                <el-divider style="margin:6px 0" />
                <el-checkbox-group v-model="selectedProc" style="display:flex;flex-direction:column;gap:3px;">
                  <el-checkbox v-for="t in procs" :key="t" :value="t" style="margin:0;">{{ t }}</el-checkbox>
                </el-checkbox-group>
              </div>
            </el-tab-pane>
            <el-tab-pane v-if="objTypes.includes('trigger')" :label="$t('dump.objTrigger')" name="trigger">
              <div style="max-height:200px;overflow:auto;padding:4px;">
                <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px;">
                  <el-checkbox v-model="allTrigger" @change="toggleAllByType('trigger', $event)" style="margin-right:auto;">{{ $t('common.selectAll') }}</el-checkbox>
                  <span style="color:var(--dc-muted);font-size: 13px;">{{ $t('dmp.selected', { n: selectedTrigger.length, total: triggers.length }) }}</span>
                </div>
                <el-divider style="margin:6px 0" />
                <el-checkbox-group v-model="selectedTrigger" style="display:flex;flex-direction:column;gap:3px;">
                  <el-checkbox v-for="t in triggers" :key="t" :value="t" style="margin:0;">{{ t }}</el-checkbox>
                </el-checkbox-group>
              </div>
            </el-tab-pane>
            <el-tab-pane v-if="objTypes.includes('event')" :label="$t('dump.objEvent')" name="event">
              <div style="max-height:200px;overflow:auto;padding:4px;">
                <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px;">
                  <el-checkbox v-model="allEvent" @change="toggleAllByType('event', $event)" style="margin-right:auto;">{{ $t('common.selectAll') }}</el-checkbox>
                  <span style="color:var(--dc-muted);font-size: 13px;">{{ $t('dmp.selected', { n: selectedEvent.length, total: events.length }) }}</span>
                </div>
                <el-divider style="margin:6px 0" />
                <el-checkbox-group v-model="selectedEvent" style="display:flex;flex-direction:column;gap:3px;">
                  <el-checkbox v-for="t in events" :key="t" :value="t" style="margin:0;">{{ t }}</el-checkbox>
                </el-checkbox-group>
              </div>
            </el-tab-pane>
          </el-tabs>
        </el-form-item>

      </el-form>
    <template #footer>
      <div style="display:flex;align-items:center;justify-content:space-between;width:100%;">
        <span style="color:var(--dc-muted);font-size: 13px;">{{ $t('dmp.selectedObjects', { n: totalSelected }) }}</span>
        <div>
          <el-button :disabled="exportTask.status === 'running'" @click="visible = false">{{ $t('common.close') }}</el-button>
          <el-button type="primary" :disabled="totalSelected === 0 || exportTask.status === 'running'" @click="doDump">{{ $t('dump.startExport') }}</el-button>
        </div>
      </div>
    </template>

    <TaskProgressDialog
      v-model:visible="exportTask.visible"
      task-kind="dump"
      :target-name="$t('dump.dbTarget')"
      :status="exportTask.status"
      :done="exportTask.done"
      :total="exportTask.total"
      :phase="exportTask.phase"
      :message="exportTask.message"
      :logs="exportTask.logs"
      :canceling="exportTask.canceling"
      @cancel="exportTask.cancel(props.conn.id)"
      @close="onTaskClose"
    />
  </el-dialog>
</template>

<script setup>
// 数据库转储弹窗（从 MainView 拆出）：打开时自动拉取各类型对象列表，本地生成 SQL/DDL 并下载。
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { t } from '../../utils/i18n'
import { listTables, listProcedures, listTriggers, listEvents, exportDbStart } from '../../api'
import TaskProgressDialog from '../../common/TaskProgressDialog.vue'
import { useExportTask } from '../../utils/useExportTask'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  conn: { type: Object, default: null },
  database: { type: String, default: '' }
})
const emit = defineEmits(['update:modelValue'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const loading = ref(false)
// 记住上一次的导出选择（localStorage dbmind_dump）：转储是个高频重复动作，
// 每次都重置回默认等于逼用户天天重选一遍格式和对象类型
const DUMP_KEY = 'dbmind_dump'
const loadDumpPrefs = () => {
  try { return JSON.parse(localStorage.getItem(DUMP_KEY) || '{}') } catch { return {} }
}
const lastPrefs = loadDumpPrefs()
const format = ref(lastPrefs.format || 'sql')
const opts = ref({ includeDrop: lastPrefs.includeDrop ?? true, includeData: lastPrefs.includeData ?? true })
const objTypes = ref(['table'])
const activeTab = ref('table')
const tables = ref([])
const views = ref([])
const funcs = ref([])
const procs = ref([])
const triggers = ref([])
const events = ref([])
const selectedTable = ref([])
const selectedView = ref([])
const selectedFunc = ref([])
const selectedProc = ref([])
const selectedTrigger = ref([])
const selectedEvent = ref([])
const allTable = ref(false)
const allView = ref(false)
const allFunc = ref(false)
const allProc = ref(false)
const allTrigger = ref(false)
const allEvent = ref(false)
// 转储走异步任务，显示进度条 + 实时日志 + 可取消 + 完成后自动下载
const exportTask = useExportTask()
const totalSelected = computed(() =>
  selectedTable.value.length +
  selectedView.value.length +
  selectedFunc.value.length +
  selectedProc.value.length +
  selectedTrigger.value.length +
  selectedEvent.value.length
)

const reset = () => {
  format.value = 'sql'
  opts.value = { includeDrop: true, includeData: true }
  objTypes.value = ['table']
  activeTab.value = 'table'
  tables.value = []; views.value = []; funcs.value = []
  procs.value = []; triggers.value = []; events.value = []
  selectedTable.value = []; selectedView.value = []; selectedFunc.value = []
  selectedProc.value = []; selectedTrigger.value = []; selectedEvent.value = []
  allTable.value = false; allView.value = false; allFunc.value = false
  allProc.value = false; allTrigger.value = false; allEvent.value = false
}

const loadObjects = async (type) => {
  const db = props.database
  if (!db) return
  try {
    switch (type) {
      case 'function':
        // 后端暂无 listFunctions，先用空列表
        funcs.value = []
        break
      case 'procedure':
        { const list = await listProcedures(props.conn?.id, db)
          procs.value = list.map(o => o.name || o.Name || o.PROCEDURE_NAME || String(o)) }
        break
      case 'trigger':
        { const list = await listTriggers(props.conn?.id, db)
          triggers.value = list.map(o => o.name || o.Name || o.TRIGGER_NAME || String(o)) }
        break
      case 'event':
        { const list = await listEvents(props.conn?.id, db)
          events.value = list.map(o => o.name || o.Name || o.EVENT_NAME || String(o)) }
        break
    }
  } catch (e) {
    // 忽略不支持的对象类型
  }
}

// 打开弹窗时重置状态并拉取对象列表
watch(() => props.modelValue, (open) => {
  if (!open || !props.database || !props.conn) return
  reset()
  loading.value = true
  ;(async () => {
    try {
      const list = await listTables(props.conn.id, props.database)
      tables.value = list.filter(t => (t.type || 'TABLE') === 'TABLE').map(t => t.name)
      views.value = list.filter(t => t.type === 'VIEW').map(t => t.name)
    } catch (e) {
      tables.value = []
      ElMessage.error(t('dump.loadTablesFailed'))
    }
    // 异步加载其他对象
    loadObjects('function')
    loadObjects('procedure')
    loadObjects('trigger')
    loadObjects('event')
  })().finally(() => { loading.value = false })
})

// 切换导出格式时重置
const onFormatChange = () => {
  if (format.value === 'sql-ddl') {
    opts.value.includeData = false
  }
}

const toggleAllByType = (type, val) => {
  switch (type) {
    case 'table': selectedTable.value = val ? [...tables.value] : []; break
    case 'view': selectedView.value = val ? [...views.value] : []; break
    case 'function': selectedFunc.value = val ? [...funcs.value] : []; break
    case 'procedure': selectedProc.value = val ? [...procs.value] : []; break
    case 'trigger': selectedTrigger.value = val ? [...triggers.value] : []; break
    case 'event': selectedEvent.value = val ? [...events.value] : []; break
  }
}

const doDump = async () => {
  const db = props.database
  if (!db || totalSelected.value === 0) return
  const fmt = format.value
  // 落盘本次选择，下次打开沿用
  try {
    localStorage.setItem(DUMP_KEY, JSON.stringify({ format: fmt, includeDrop: opts.value.includeDrop, includeData: opts.value.includeData }))
  } catch { /* 存不进就只当本次没记 */ }
  const payload = {
    database: db,
    format: fmt,
    tables: selectedTable.value,
    views: selectedView.value,
    functions: selectedFunc.value,
    procedures: selectedProc.value,
    triggers: selectedTrigger.value,
    events: selectedEvent.value,
    options: {
      includeDrop: opts.value.includeDrop,
      includeData: opts.value.includeData && fmt === 'sql'
    }
  }
  // zip 类（csv/json/excel）下载文件扩展名为 .zip，sql 类为 .sql
  const dlFormat = (fmt === 'csv' || fmt === 'json' || fmt === 'excel') ? 'zip' : 'sql'
  await exportTask.start(props.conn.id, payload, db, dlFormat, exportDbStart)
}

// 任务结束（完成/失败/取消）：关闭进度弹窗并关闭本弹窗
const onTaskClose = () => {
  exportTask.close()
  visible.value = false
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
/* 转储弹窗 tabs 深色适配 */
.dump-tabs :deep(.el-tabs__header) { background: var(--dc-bg-input); border: 1px solid var(--dc-border); border-bottom: none; }
.dump-tabs :deep(.el-tabs__nav-wrap::after) { background-color: var(--dc-border); }
.dump-tabs :deep(.el-tabs__item) {
  border: 1px solid var(--dc-border) !important;
  border-bottom: none !important;
  background: var(--dc-bg-card) !important;
  color: var(--dc-text-dim) !important;
  margin-right: 2px;
  border-radius: 4px 4px 0 0;
  transition: color .15s, background .15s;
}
.dump-tabs :deep(.el-tabs__item.is-active) {
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep)) !important;
  color: var(--dc-on-primary) !important;
  border-color: var(--dc-primary-deep) !important;
}
.dump-tabs :deep(.el-tabs__item:not(.is-active):hover) { color: var(--dc-text) !important; background: var(--dc-bg-hover) !important; }
.dump-tabs :deep(.el-tabs__content) {
  background: var(--dc-bg-input);
  border: 1px solid var(--dc-border);
  border-top: none;
  padding: 0;
}
</style>
