import { ref, reactive } from 'vue'
import { t } from './i18n'
import { ElMessage } from 'element-plus'
import { runSqlFileStart, runSqlFileStatus, runSqlFileCancel } from '../api'
import { errMsg } from './errMsg'

const fmtTime = () => new Date().toLocaleTimeString('zh-CN', { hour12: false })

/**
 * 执行 SQL 文件任务：提交后轮询进度/日志，完成后不下载（仅展示执行结果汇总）。
 * 与 useExportTask 类似但去掉了文件下载逻辑。
 */
export function useSqlFileTask() {
  const visible = ref(false)
  const taskId = ref('')
  const connId = ref('')
  const status = ref('running')
  const done = ref(0)
  const total = ref(-1)
  const phase = ref(t('usf.preparing'))
  const message = ref('')
  const logs = ref([])
  const canceling = ref(false)

  let timer = null

  const reset = () => {
    status.value = 'running'
    done.value = 0
    total.value = -1
    phase.value = t('usf.preparing')
    message.value = t('usf.submitting')
    logs.value = []
    canceling.value = false
    if (timer) { clearInterval(timer); timer = null }
  }

  const startPolling = () => {
    if (timer) clearInterval(timer)
    timer = setInterval(async () => {
      if (!taskId.value) return
      try {
        const r = await runSqlFileStatus(connId.value, taskId.value)
        status.value = r.status || 'running'
        done.value = r.done ?? 0
        total.value = r.total ?? -1
        phase.value = r.phase || ''
        message.value = r.message || ''
        if (Array.isArray(r.logs)) {
          logs.value = r.logs.map(text => ({ time: fmtTime(), text }))
        }
        if (status.value !== 'running') {
          clearInterval(timer)
          timer = null
          if (status.value === 'success') {
            ElMessage.success(message.value || t('usf.done'))
          } else if (status.value === 'canceled') {
            ElMessage.warning(t('usf.canceled'))
          } else {
            ElMessage.error(t('usf.failed', { detail: (message.value || t('common.unknownError')) }))
          }
        }
      } catch (e) {
        console.error('SQL 文件任务轮询失败', e)
      }
    }, 500)
  }

  const start = async (id, payload) => {
    connId.value = id
    reset()
    visible.value = true
    try {
      const res = await runSqlFileStart(id, payload)
      if (!res.success || !res.taskId) {
        throw new Error(res.message || t('usf.submitFailed'))
      }
      taskId.value = res.taskId
      startPolling()
    } catch (e) {
      status.value = 'error'
      message.value = errMsg(e)
      ElMessage.error(t('usf.submitFailedDetail', { detail: errMsg(e) }))
    }
  }

  const cancel = async () => {
    if (!taskId.value) return
    canceling.value = true
    try {
      await runSqlFileCancel(connId.value, taskId.value)
    } catch (e) {
      ElMessage.error(t('usf.cancelFailed', { detail: errMsg(e) }))
    } finally {
      setTimeout(() => { canceling.value = false }, 2000)
    }
  }

  const close = () => {
    visible.value = false
    if (timer) { clearInterval(timer); timer = null }
  }

  return reactive({
    visible,
    status,
    done,
    total,
    phase,
    message,
    logs,
    canceling,
    start,
    cancel,
    close
  })
}
