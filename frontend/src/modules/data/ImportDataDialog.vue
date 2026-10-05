<template>
  <el-dialog v-model="visible" width="560px" :close-on-click-modal="false" append-to-body class="main-dialog import-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Upload /></el-icon></span>
        <span>{{ $t('imp.titleTo', { table: target?.table || '' }) }}</span>
      </div>
    </template>

    <div class="import-body">
      <div class="import-section">
        <div class="import-label"><span class="import-required">*</span> {{ $t('imp.file') }}</div>
        <!-- accept 与后端真正支持的一致：CSV / JSON / Excel(.xlsx)。
             旧版 .xls（2003 二进制）是另一套格式，不在支持范围，后端会明确说清并给出替代做法 -->
        <el-upload v-if="!file" drag :auto-upload="false" :limit="1" accept=".csv,.xlsx,.json"
                   :on-change="onFileChange" class="import-uploader">
          <el-icon class="import-uploader-icon" :size="48"><UploadFilled /></el-icon>
          <div class="import-uploader-title">{{ $t('imp.dropHint') }}</div>
          <div class="import-uploader-sub">{{ $t('imp.formats') }}</div>
        </el-upload>
        <div v-else class="import-file-card">
          <div class="import-file-info">
            <el-icon :size="26" color="var(--dc-primary)"><Document /></el-icon>
            <div class="import-file-meta">
              <div class="import-file-name" :title="file.name">{{ file.name }}</div>
              <div class="import-file-size">{{ formatFileSize(file.size) }}</div>
            </div>
          </div>
          <el-button text size="small" @click="file = null">{{ $t('imp.remove') }}</el-button>
        </div>
      </div>

      <div class="import-section">
        <div class="import-label">{{ $t('imp.mode') }}</div>
        <div class="import-mode-list">
          <div v-for="m in modes" :key="m.value" class="import-mode-item"
               :class="{ active: mode === m.value }" @click="mode = m.value">
            <el-icon :size="20"><component :is="m.icon" /></el-icon>
            <div class="import-mode-title">{{ m.label }}</div>
            <div class="import-mode-desc">{{ m.desc }}</div>
          </div>
        </div>
      </div>
    </div>

    <template #footer>
      <div class="import-footer">
        <el-button @click="visible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="importing" :disabled="!file" @click="doImport">
          {{ importing ? $t('imp.importing') : $t('imp.start') }}
        </el-button>
      </div>
    </template>
  </el-dialog>

  <TaskProgressDialog v-model:visible="progressVisible" task-kind="import" :target-name="file ? file.name : $t('imp.data')"
                      :status="taskStatus" :done="taskDone" :total="taskTotal"
                      :phase="taskPhase" :message="taskMessage" :logs="taskLogs"
                      :canceling="canceling"
                      @cancel="cancelImport" @close="onProgressClose" />
</template>

<script setup>
// 导入数据弹窗：选择 CSV / Excel(.xlsx) / JSON 后按追加 / 清空 / 建表三种方式导入，成功回调父级刷新树。
// （.xlsx 由后端 xlsx 模块按 Excel 的规则解析：共享字符串、被省略的空单元格、日期序列号；
//   旧版 .xls 是另一套二进制格式，不在支持范围。）
// 导入改为异步任务，通过 TaskProgressDialog 展示真实进度与实时日志。
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { ElMessage } from 'element-plus'
import { t } from '../../utils/i18n'
import { Upload, UploadFilled, Document, Bottom, RefreshRight, Plus } from '@element-plus/icons-vue'
import { importStart, importTask, importCancel } from '../../api'
import TaskProgressDialog from '../../common/TaskProgressDialog.vue'
import { errMsg } from '../../utils/errMsg'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  conn: { type: Object, default: null },
  target: { type: Object, default: null }
})
const emit = defineEmits(['update:modelValue', 'done'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const file = ref(null)
const mode = ref('append')
const importing = ref(false)
let pollTimer = null

const modes = [
  { value: 'append', label: t('imp.modeAppend'), desc: t('imp.modeAppendDesc'), icon: Bottom },
  { value: 'overwrite', label: t('imp.modeOverwrite'), desc: t('imp.modeOverwriteDesc'), icon: RefreshRight },
  { value: 'create', label: t('imp.modeCreate'), desc: t('imp.modeCreateDesc'), icon: Plus }
]

// 进度弹窗状态
const progressVisible = ref(false)
const taskId = ref('')
const taskStatus = ref('running')
const taskDone = ref(0)
const taskTotal = ref(-1)
const taskPhase = ref(t('imp.queued'))
const taskMessage = ref('')
const taskLogs = ref([])
const canceling = ref(false)

// 打开弹窗时重置为追加模式并清空已选文件
watch(() => props.modelValue, (open) => {
  if (!open) return
  mode.value = 'append'
  file.value = null
  resetTask()
})

const onFileChange = (f) => { file.value = f.raw }

const formatFileSize = (bytes) => {
  if (!bytes) return ''
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
}

const resetTask = () => {
  taskId.value = ''
  taskStatus.value = 'running'
  taskDone.value = 0
  taskTotal.value = -1
  taskPhase.value = t('imp.queued')
  taskMessage.value = ''
  taskLogs.value = []
  canceling.value = false
  if (pollTimer) { clearInterval(pollTimer); pollTimer = null }
}

const doImport = async () => {
  if (!file.value) return ElMessage.warning(t('imp.pickFile'))
  // 局部变量改名：本文件已 import { t } 做翻译，同名会遮蔽（而且上一行就要用 t()）
  const tgt = props.target
  if (!tgt) return
  importing.value = true
  resetTask()
  try {
    const fd = new FormData()
    fd.append('file', file.value)
    const res = await importStart(props.conn?.id, fd, {
      timeout: 0,
      params: {
        database: tgt.db,
        table: mode.value === 'create' ? '' : tgt.table,
        mode: mode.value
      }
    })
    if (!res.success || !res.taskId) {
      throw new Error(res.message || t('imp.submitFailed'))
    }
    taskId.value = res.taskId
    progressVisible.value = true
    startPolling()
  } catch (e) {
    importing.value = false
    ElMessage.error(t('imp.failed', { detail: errMsg(e) }))
  }
}

const startPolling = () => {
  if (pollTimer) clearInterval(pollTimer)
  pollTimer = setInterval(async () => {
    if (!taskId.value) return
    try {
      const r = await importTask(props.conn?.id, taskId.value)
      taskStatus.value = r.status || 'running'
      taskDone.value = r.done ?? 0
      taskTotal.value = r.total ?? -1
      taskPhase.value = r.phase || ''
      taskMessage.value = r.message || ''
      if (Array.isArray(r.logs)) {
        taskLogs.value = r.logs.map(text => ({ time: nowTime(), text }))
      }
      // 防御性兜底：后端返回任何非预期状态都按 error 处理，避免显示“已结束”这种含糊状态
      const known = ['running', 'success', 'error', 'canceled']
      if (!known.includes(taskStatus.value)) {
        taskStatus.value = 'error'
      }
      if (taskStatus.value !== 'running') {
        clearInterval(pollTimer)
        pollTimer = null
        importing.value = false
        if (taskStatus.value === 'success') {
          ElMessage.success(taskMessage.value || t('imp.done'))
          file.value = null
          visible.value = false
          emit('done')
        } else if (taskStatus.value === 'canceled') {
          ElMessage.warning(t('imp.canceled'))
        }
        // 失败/错误只在日志框内展示，不再额外弹 ElMessage，避免与日志重复
      }
    } catch (e) {
      // 轮询异常不弹窗，继续重试；连续失败由超时/取消处理
      console.error('导入进度轮询失败', e)
    }
  }, 500)
}

const cancelImport = async () => {
  if (!taskId.value) return
  canceling.value = true
  try {
    await importCancel(props.conn?.id, taskId.value)
  } catch (e) {
    ElMessage.error(t('mv.cancelFailed', { detail: errMsg(e) }))
  } finally {
    // 后端已标记取消，轮询会在下一次收到 canceled 状态后恢复按钮
    setTimeout(() => { canceling.value = false }, 2000)
  }
}

const onProgressClose = () => {
  progressVisible.value = false
  if (pollTimer) { clearInterval(pollTimer); pollTimer = null }
  importing.value = false
}

// 组件被销毁时兜底停掉轮询：否则导入进行中切走，定时器会一直留着继续打后端
onBeforeUnmount(() => { if (pollTimer) { clearInterval(pollTimer); pollTimer = null } })

const nowTime = () => {
  const d = new Date()
  return d.toTimeString().split(' ')[0]
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
.import-dialog :deep(.el-dialog__header) { padding-bottom: 4px; }
.import-dialog :deep(.el-dialog__body) { padding: 8px 24px 4px; }

.import-body { padding: 8px 0 4px; }
.import-section + .import-section { margin-top: 24px; }
.import-label { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); margin-bottom: 10px; }
.import-required { color: var(--dc-danger); margin-right: 2px; }

.import-uploader { width: 100%; }
.import-uploader :deep(.el-upload-dragger) {
  background: var(--dc-bg-soft); border: 2px dashed var(--dc-border-strong); border-radius: 12px;
  padding: 36px 20px; display: flex; flex-direction: column; align-items: center; gap: 10px;
  transition: border-color .2s, background .2s;
}
.import-uploader :deep(.el-upload-dragger:hover) {
  border-color: var(--dc-primary); background: var(--dc-primary-wash);
}
.import-uploader-icon { color: var(--dc-primary); }
.import-uploader-title { font-size: 14px; color: var(--dc-text-strong); font-weight: 500; }
.import-uploader-sub { font-size: 13px; color: var(--dc-text-weak); }

.import-file-card {
  display: flex; align-items: center; justify-content: space-between; gap: 12px;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border); border-radius: 12px;
  padding: 14px 16px;
}
.import-file-info { display: flex; align-items: center; gap: 12px; flex: 1; min-width: 0; }
.import-file-meta { min-width: 0; }
.import-file-name { font-size: 14px; color: var(--dc-text-strong); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.import-file-size { font-size: 13px; color: var(--dc-text-weak); margin-top: 2px; }

.import-mode-list { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; }
.import-mode-item {
  cursor: pointer; border: 1px solid var(--dc-border); border-radius: 10px;
  padding: 14px 10px; text-align: center; transition: all .2s;
  display: flex; flex-direction: column; align-items: center; gap: 6px;
}
.import-mode-item:hover { border-color: var(--dc-primary); background: var(--dc-primary-wash); }
.import-mode-item.active { border-color: var(--dc-primary); background: var(--dc-primary-wash); box-shadow: 0 0 0 1px var(--dc-primary); }
.import-mode-item .el-icon { color: var(--dc-text-mid); }
.import-mode-item.active .el-icon { color: var(--dc-primary); }
.import-mode-title { font-size: 14px; font-weight: 500; color: var(--dc-text-strong); }
.import-mode-desc { font-size: 12px; color: var(--dc-text-weak); line-height: 1.4; }

.import-footer { display: flex; justify-content: flex-end; gap: 10px; }
</style>
