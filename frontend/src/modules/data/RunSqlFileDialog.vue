<template>
  <el-dialog v-model="visible" width="480px" :close-on-click-modal="false" append-to-body class="main-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><VideoPlay /></el-icon></span>
        <span>{{ $t('rsf.title') }}</span>
      </div>
    </template>
    <el-form label-width="90px" size="small">
      <el-form-item :label="$t('rsf.file')" required>
        <el-upload drag :auto-upload="false" :limit="1" accept=".sql"
                   :on-change="(f)=>file=f.raw" :on-remove="()=>file=null">
          <el-icon class="el-icon--upload"><UploadFilled /></el-icon>
          <div class="el-upload__text">{{ $t('rsf.dropPrefix') }}<em>{{ $t('rsf.clickChoose') }}</em></div>
        </el-upload>
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button :disabled="sqlFileTask.status === 'running'" @click="visible = false">{{ $t('common.close') }}</el-button>
      <el-button type="primary" :disabled="!file || sqlFileTask.status === 'running'" @click="run">{{ $t('common.run') }}</el-button>
    </template>
  </el-dialog>

  <TaskProgressDialog
    v-model:visible="sqlFileTask.visible"
    task-kind="runFile"
    :status="sqlFileTask.status"
    :done="sqlFileTask.done"
    :total="sqlFileTask.total"
    :phase="sqlFileTask.phase"
    :message="sqlFileTask.message"
    :logs="sqlFileTask.logs"
    :canceling="sqlFileTask.canceling"
    @cancel="sqlFileTask.cancel()"
    @close="onClose"
  />
</template>

<script setup>
// 运行 SQL 文件弹窗（从 MainView 拆出）：选择 .sql 文件后在目标库执行，成功回调父级刷新库节点。
// 走异步任务：逐语句执行，实时进度/日志，可取消，完成后展示执行结果汇总。
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { ElMessage } from 'element-plus'
import { t } from '../../utils/i18n'
import { VideoPlay, UploadFilled } from '@element-plus/icons-vue'
import TaskProgressDialog from '../../common/TaskProgressDialog.vue'
import { useSqlFileTask } from '../../utils/useSqlFileTask'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  conn: { type: Object, default: null },
  database: { type: String, default: '' }
})
const emit = defineEmits(['update:modelValue', 'done'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const file = ref(null)
const sqlFileTask = useSqlFileTask()

// 卸载时兜底关闭任务轮询（本组件可能被执行中被父级条件卸载）
onBeforeUnmount(() => { sqlFileTask.close() })

// 打开弹窗时清空已选文件
watch(() => props.modelValue, (open) => {
  if (!open) return
  file.value = null
})

const run = async () => {
  const db = props.database
  if (!file.value || !db) return
  try {
    const text = await file.value.text()
    await sqlFileTask.start(props.conn?.id, { sql: text, database: db })
  } catch (e) {
    ElMessage.error(t('rsf.submitFailed', { detail: (e?.message || e) }))
  }
}

// 任务结束（完成/失败/取消）：关闭进度弹窗；成功时通知父级刷新库节点
const onClose = () => {
  sqlFileTask.close()
  visible.value = false
  if (sqlFileTask.status.value === 'success') emit('done')
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
</style>
