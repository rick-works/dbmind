import { ref, reactive, h } from 'vue'

import { t, locale } from './i18n'
import { ElMessage } from 'element-plus'
import { exportStart, exportTask, exportCancel, exportDownload, openLocalDir, getPathSettings } from '../api'
import { errMsg } from './errMsg'

const fmtTime = () => new Date().toLocaleTimeString(locale.value, { hour12: false })
const extMap = { csv: 'csv', excel: 'xlsx', json: 'json', 'sql-insert': 'sql', 'sql-update': 'sql', 'sql-delete': 'sql', ddl: 'sql' }

/** 后端已按行上报进度，采样间隔给到 300ms 就足够跟手（以前 500ms 且后端 5000 行才动一次） */
const POLL_MS = 300

/** 日志区最多保留多少行：长导出会写出成百上千条，别让 DOM 一直涨 */
const MAX_UI_LOGS = 500

export function downloadBlob(blob, name, format) {
  const ext = extMap[format] || format
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `${name}_${Date.now()}.${ext}`
  a.click()
  URL.revokeObjectURL(url)
}

/**
 * 日志只**追加新行**（不要每轮重建整份）。
 *
 * 两个坑都在这：
 * ① 后端 `logs` 数组会被 MAX_LOGS(200) 裁剪（丢最旧的），长度到上限后就再也不变长 ——
 *    只凭"长度变了"判断有没有新行，日志区会永远停住不动。后端为此补了单调的 `logsSeq`，
 *    这里按序号算差量，取数组末尾那几条（差量大于数组长度时以数组为准）。
 * ② 时间戳用**收到这一行的本地时刻**（后端只回文本）。以前每轮都用"当前时刻"重建全部行，
 *    于是每一行的耗时都在闪，看起来像刷新而不是在滚动。
 */
function appendLogs(state, r) {
  if (!Array.isArray(r.logs)) return
  const seq = typeof r.logsSeq === 'number' ? r.logsSeq : null
  if (seq === null) {
    // 旧后端（没有 logsSeq）退回全量覆盖，至少不报错
    if (r.logs.length !== state.logs.value.length) {
      state.logs.value = r.logs.map((text) => ({ time: fmtTime(), text }))
    }
    return
  }
  if (seq <= state.seenLogs) return
  const take = Math.min(seq - state.seenLogs, r.logs.length)
  state.seenLogs = seq
  if (take <= 0) return
  const fresh = r.logs.slice(r.logs.length - take).map((text) => ({ time: fmtTime(), text }))
  state.logs.value = state.logs.value.concat(fresh).slice(-MAX_UI_LOGS)
}

/**
 * 把一次任务快照应用到进度状态，返回「任务是否已结束」。
 *
 * 为什么要抽出来：useExportTask 与 MainView 的右键导出原先各写了一份几乎一样的
 * 「取快照 + 刷日志」，改一处另一处必然漂（日志重建、轮询间隔都吃过这个亏）。
 *
 * state 形状：{ status, done, total, phase, message, logs, seenLogs }（都是 ref，见下）
 */
export function applyTaskSnapshot(state, r) {
  state.status.value = r.status || 'running'
  state.done.value = r.done ?? 0
  state.total.value = r.total ?? -1
  state.phase.value = r.phase || ''
  state.message.value = r.message || ''
  // 产物路径（后端在状态里给）：收尾时用它提示"保存在哪"+ 打开文件夹
  if (typeof r.filePath === 'string') state.filePath = r.filePath
  appendLogs(state, r)
  return state.status.value !== 'running'
}

/** 取文件所在目录（拿不到就返回空串） */
function dirOf(path) {
  if (typeof path !== 'string' || !path) return ''
  const i = Math.max(path.lastIndexOf('\\'), path.lastIndexOf('/'))
  return i > 0 ? path.slice(0, i) : ''
}

/** 用户是否把导出目录改成了自定义位置 */
async function isCustomExportDir() {
  try {
    const p = await getPathSettings()
    return !!p?.exportDirConfigured
  } catch {
    return false
  }
}

/**
 * 任务结束后的收尾（组合函数与 MainView 共用）。
 *
 * 行为约定：
 *  - 导出目录是**默认**的 → 浏览器下载（保持原有习惯）
 *  - 导出目录是**用户自定义**的 → 不再重复下载一遍（文件本来就已经落在那个文件夹里了，
 *    再下载一次只会让"下载"目录多出一份重复文件），改为提示落盘位置
 *  - 两种情况都给一个「打开文件夹」的入口，不用再去设置页翻
 */
export async function finishExport(state, connId, taskId, name, format) {
  if (state.status.value === 'success') {
    const dir = dirOf(state.filePath)
    if (!(await isCustomExportDir())) {
      try {
        const blob = await exportDownload(connId, taskId)
        downloadBlob(blob, name, format)
      } catch (err) {
        ElMessage.error(t('mv.downloadFailed', { detail: errMsg(err) }))
        return
      }
    }
    if (dir) {
      ElMessage({
        type: 'success',
        duration: 10000,
        message: h('span', [
          t('mv.exportSavedTo', { path: dir }),
          h('a', {
            style: 'margin-left:10px;cursor:pointer;text-decoration:underline',
            onClick: () => openLocalDir(dir)
          }, t('common.openDir'))
        ])
      })
    }
    return
  }
  if (state.status.value === 'canceled') {
    ElMessage.warning(t('mv.exportCanceled'))
    return
  }
  ElMessage.error(t('mv.exportFailed', { detail: (state.message.value || t('common.unknownError')) }))
}

export function useExportTask() {
  const visible = ref(false)
  const taskId = ref('')
  const status = ref('running')
  const done = ref(0)
  const total = ref(-1)
  const phase = ref(t('mv.exportPreparing'))
  const message = ref('')
  const logs = ref([])
  const canceling = ref(false)

  let timer = null
  // 与 MainView 共用同一套「应用快照」逻辑，所以状态也按同一形状装袋
  const state = { status, done, total, phase, message, logs, seenLogs: 0 }

  const reset = (title) => {
    status.value = 'running'
    done.value = 0
    total.value = -1
    phase.value = t('mv.exportPreparing')
    message.value = title || t('mv.submittingExport')
    logs.value = []
    state.seenLogs = 0
    canceling.value = false
    if (timer) { clearInterval(timer); timer = null }
  }

  const startPolling = (connId, name, format) => {
    if (timer) clearInterval(timer)
    timer = setInterval(async () => {
      if (!taskId.value) return
      try {
        const r = await exportTask(connId, taskId.value)
        const finished = applyTaskSnapshot(state, r)
        if (finished) {
          clearInterval(timer)
          timer = null
          await finishExport(state, connId, taskId.value, name, format)
        }
      } catch (e) {
        console.error('导出进度轮询失败', e)
      }
    }, POLL_MS)
  }

  const start = async (connId, payload, name, format, submitFn) => {
    reset(t('et.exporting', { name: (name || t('et.data')) }))
    visible.value = true
    try {
      const res = await (submitFn || exportStart)(connId, payload)
      if (!res.success || !res.taskId) {
        throw new Error(res.message || t('mv.exportSubmitFailed'))
      }
      taskId.value = res.taskId
      startPolling(connId, name, format)
    } catch (e) {
      status.value = 'error'
      message.value = errMsg(e)
      ElMessage.error(t('mv.exportFailed', { detail: errMsg(e) }))
    }
  }

  const cancel = async (connId) => {
    if (!taskId.value) return
    canceling.value = true
    try {
      await exportCancel(connId, taskId.value)
    } catch (e) {
      ElMessage.error(t('mv.cancelFailed', { detail: errMsg(e) }))
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
