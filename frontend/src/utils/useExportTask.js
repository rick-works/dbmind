import { ref, reactive } from 'vue'

import { t, locale } from './i18n'
import { ElMessage } from 'element-plus'
import { exportStart, exportTask, exportCancel, exportDownload } from '../api'
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

/** 导出文件名：下载与另存为共用同一口径，免得两条路生成的名字不一样 */
export function exportFileName(name, format) {
  const ext = extMap[format] || format
  return `${name}_${Date.now()}.${ext}`
}

/** 另存为对话框里按扩展名过滤 */
function pickerTypes(format) {
  const ext = extMap[format] || format
  const mime = {
    csv: 'text/csv',
    xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
    json: 'application/json',
    sql: 'application/sql'
  }[ext]
  if (!mime) return undefined
  const accept = {}
  accept[mime] = ['.' + ext]
  return [{ description: ext.toUpperCase(), accept }]
}

/**
 * 让用户选位置并保存（所有导出/下载最终都走这里）。
 *
 * filename 是**完整文件名**（含扩展名）—— 各处的导出自己已经拼好了名字，
 * 这里只负责弹系统另存为 + 写盘；拿不到原生另存为（非 Chrome 内核 / 没有用户手势）
 * 就退回普通浏览器下载，至少能落盘。
 */
export async function saveBlobAs(blob, filename, types) {
  if (typeof window.showSaveFilePicker === 'function') {
    try {
      const handle = await window.showSaveFilePicker({ suggestedName: filename, types })
      const w = await handle.createWritable()
      await w.write(blob)
      await w.close()
      return { ok: true, name: handle.name || filename }
    } catch (err) {
      // 用户主动取消：不算失败，也不再退回下载
      if (err && err.name === 'AbortError') return { ok: false, canceled: true }
      // SecurityError 等（没有用户手势）：退回普通下载，别让整个导出白跑
    }
  }
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
  return { ok: true, name: filename, fallback: true }
}

/**
 * 让用户选「保存到哪」，然后把内容写进去。
 *
 * 优先用浏览器原生的另存为对话框（File System Access API，Chrome/Edge 支持，
 * 127.0.0.1 属于安全上下文所以可用）—— **这才是"用户自己选目录"**。
 * 拿不到（非 Chrome 内核、或没有用户手势）就退回普通浏览器下载，至少能落盘。
 * handle 是导出前那次挑选留下的，异步任务跑完直接往它里面写。
 */
export async function saveExportBlob(blob, name, format, handle = null) {
  const filename = exportFileName(name, format)
  if (handle) {
    try {
      const w = await handle.createWritable()
      await w.write(blob)
      await w.close()
      return { ok: true, name: handle.name || filename }
    } catch (err) {
      return { ok: false, error: err }
    }
  }
  return saveBlobAs(blob, filename, pickerTypes(format))
}

/**
 * 导出【开始前】先让用户挑保存位置。
 *
 * 为什么必须提前挑：异步导出要跑几十秒到几分钟，跑完时浏览器的"用户手势"早就过期，
 * 那时再调另存为对话框会被安全策略拒绝（SecurityError）。在点按钮的这一刻挑，才拿得到。
 * 返回：FileSystemFileHandle / { canceled: true }（用户取消）/ null（不支持，走退回路径）
 */
export async function pickExportTarget(name, format) {
  if (typeof window.showSaveFilePicker !== 'function') return null
  try {
    return await window.showSaveFilePicker({ suggestedName: exportFileName(name, format), types: pickerTypes(format) })
  } catch (err) {
    if (err && err.name === 'AbortError') return { canceled: true }
    return null
  }
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

/**
 * 任务结束后的收尾（组合函数与 MainView 共用）。
 *
 * 保存到哪由【用户自己选】：导出前（异步任务）或导出后（同步单页）弹系统另存为。
 * 以前是固定写到后端那个目录、再弹一句"已保存到 <后端路径>"+"打开目录" ——
 * 那个路径不是用户要的，用户要的是自己决定存哪儿。
 */
export async function finishExport(state, connId, taskId, name, format, handle = null) {
  if (state.status.value === 'success') {
    try {
      const blob = await exportDownload(connId, taskId)
      const res = await saveExportBlob(blob, name, format, handle)
      if (res.canceled) return
      if (!res.ok) {
        ElMessage.error(t('mv.downloadFailed', { detail: errMsg(res.error) }))
        return
      }
      ElMessage.success(t('mv.exportSavedAs', { name: res.name }))
    } catch (err) {
      ElMessage.error(t('mv.downloadFailed', { detail: errMsg(err) }))
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

  const startPolling = (connId, name, format, handle) => {
    if (timer) clearInterval(timer)
    timer = setInterval(async () => {
      if (!taskId.value) return
      try {
        const r = await exportTask(connId, taskId.value)
        const finished = applyTaskSnapshot(state, r)
        if (finished) {
          clearInterval(timer)
          timer = null
          await finishExport(state, connId, taskId.value, name, format, handle)
        }
      } catch (e) {
        console.error('导出进度轮询失败', e)
      }
    }, POLL_MS)
  }

  const start = async (connId, payload, name, format, submitFn) => {
    // 先挑保存位置，再开始导出（异步导出跑完时已经没有用户手势，那时弹不出来）
    const target = await pickExportTarget(name, format)
    if (target && target.canceled) return
    const handle = target && !target.canceled ? target : null
    reset(t('et.exporting', { name: (name || t('et.data')) }))
    visible.value = true
    try {
      const res = await (submitFn || exportStart)(connId, payload)
      if (!res.success || !res.taskId) {
        throw new Error(res.message || t('mv.exportSubmitFailed'))
      }
      taskId.value = res.taskId
      startPolling(connId, name, format, handle)
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
