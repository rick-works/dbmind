import { ref, reactive } from 'vue'

import { t, locale } from './i18n'
import { ElMessage } from 'element-plus'
import { exportStart, exportTask, exportCancel, exportDownload, exportSaveTo } from '../api'
import { errMsg } from './errMsg'
import { desktopShell } from './desktopShell'
import { addBgTask, setBgTaskStatus } from '../modules/sync/backgroundTasks'

/** 只为了提示里显示文件名（不引 node:path，浏览器里没有） */
const baseName = (p) => String(p || '').split(/[\\/]/).pop() || ''

/**
 * 桌面壳里让用户自己选「存到哪」，返回绝对路径。
 *
 * 只在**没有** File System Access API 的壳里用（macOS WKWebView / Linux WebKitGTK）：
 * 那边点导出只能走 `<a download>`，文件静默落进下载目录，用户根本没得选（用户实测反馈）。
 * 返回：{ path } / { canceled: true } / null（不是桌面壳 → 调用方退回浏览器下载）
 */
async function pickNativeSavePath(filename, title = '') {
  const picked = await desktopShell.pickPath({ title, save: true, defaultName: filename })
  if (!picked) return null
  if (picked.canceled) return { canceled: true }
  return { path: picked.path }
}

/** 让内核把字节写到指定绝对路径（桌面板壳与内核同机，走本机回环最省事） */
async function writeBytesToPath(blob, filePath) {
  const res = await fetch(`/api/files/write?path=${encodeURIComponent(filePath)}`, { method: 'POST', body: blob })
  const data = await res.json().catch(() => ({}))
  if (!res.ok || data.success === false) {
    throw new Error(data.message || `HTTP ${res.status}`)
  }
  return data
}

const fmtTime = () => new Date().toLocaleTimeString(locale.value, { hour12: false })

/**
 * 保存失败的统一出口。
 *
 * 为什么在这里报而不交给调用方：全项目十几处导出都调 saveBlobAs，而绝大多数调用方
 * 只判 `canceled`（拿到 `{ ok:false }` 也照样弹「导出成功」）。保存这事失败必须让人看见，
 * 所以在这里一次性提示，调用方只要 `if (!saved.ok) return` 就不会再报成功。
 */
function failSave (err) {
  ElMessage.error(t('mv.saveFailed', { detail: errMsg(err) }))
  return { ok: false, error: err, notified: true }
}
const extMap = { csv: 'csv', excel: 'xlsx', json: 'json', 'sql-insert': 'sql', 'sql-update': 'sql', 'sql-delete': 'sql', ddl: 'sql' }

/** 后端已按行上报进度，采样间隔给到 300ms 就足够跟手（以前 500ms 且后端 5000 行才动一次） */
const POLL_MS = 300

/** 日志区最多保留多少行：长导出会写出成百上千条，别让 DOM 一直涨 */
const MAX_UI_LOGS = 500

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
 * 【底层】只负责「挑保存位置」，不碰文件内容。
 *
 * 抽出来的原因：所有导出都要「先选目录、再生成/执行」，而 showSaveFilePicker 必须在
 * 用户手势内调用 —— 原来 saveBlobAs 把「选位置 + 写盘」绑在一起，只能先有 blob 再弹框，
 * 长任务跑完时手势早过期。现在调用方可以在点按钮的这一刻先 pickSaveTarget 拿到句柄，
 * 内容生成完再 saveToTarget 写进去。
 *
 * 返回：FileSystemFileHandle / { nativePath, nativeName }（桌面壳 WebKit）/
 *       { canceled: true } / { failed: true } / null（非桌面壳、无 API，走浏览器下载兜底）
 */
export async function pickSaveTarget(filename, types, title = '') {
  if (typeof window.showSaveFilePicker === 'function') {
    try {
      return await window.showSaveFilePicker({ suggestedName: filename, types })
    } catch (err) {
      // 用户主动取消：不再退回下载；SecurityError 等（手势过期）继续走下面的兜底
      if (err && err.name === 'AbortError') return { canceled: true }
    }
  }
  const native = await pickNativeSavePath(filename, title)
  if (!native) return desktopShell.available ? { failed: true } : null
  if (native.canceled) return { canceled: true }
  return { nativePath: native.path, nativeName: baseName(native.path) }
}

/**
 * 【底层】把字节写进已挑好的位置。target 为空时退回普通浏览器下载。
 */
export async function saveToTarget(blob, filename, types, target, title = '') {
  if (target && target.nativePath) {
    try {
      await writeBytesToPath(blob, target.nativePath)
      return { ok: true, name: target.nativeName || baseName(target.nativePath) }
    } catch (err) {
      return failSave(err)
    }
  }
  if (target) {
    try {
      const w = await target.createWritable()
      await w.write(blob)
      await w.close()
      return { ok: true, name: target.name || filename }
    } catch (err) {
      return failSave(err)
    }
  }
  // 在桌面壳里却连对话框都没弹出来：宁可明确报错，也不静默落到「下载」目录
  if (desktopShell.available) return failSave(new Error(t('mv.saveDialogFailed')))
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
  return { ok: true, name: filename, fallback: true }
}

/**
 * 生成内容后保存（兼容老调用方：选位置发生在生成之后）。
 * 新代码请用 saveGenerated —— 那种才是「先选目录再导出」。
 */
export async function saveBlobAs(blob, filename, types, opts = {}) {
  const target = await pickSaveTarget(filename, types, opts.title)
  if (target && target.canceled) return { ok: false, canceled: true }
  if (target && target.failed) return failSave(new Error(t('mv.saveDialogFailed')))
  return saveToTarget(blob, filename, types, target, opts.title)
}

/**
 * 【推荐】所有同步导出的统一入口：**先让用户选保存位置，再生成内容，最后写入**。
 *
 * generate 可以同步也可以异步（返回 Blob / Promise<Blob>）。选位置这一步放在最前面，
 * 保证 showSaveFilePicker 还在用户手势内 —— 这正是「导出前先选目录」与「导出完才让选」
 * 的分界线。用户取消则 generate 根本不会执行。
 */
export async function saveGenerated({ filename, types, generate, title = '' }) {
  const target = await pickSaveTarget(filename, types, title)
  if (target && target.canceled) return { ok: false, canceled: true }
  if (target && target.failed) {
    ElMessage.error(t('mv.saveDialogFailed'))
    return { ok: false }
  }
  let blob
  try {
    blob = await generate()
  } catch (err) {
    return failSave(err)
  }
  if (!blob) return { ok: false }
  return saveToTarget(blob, filename, types, target, title)
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
  return saveToTarget(blob, exportFileName(name, format), pickerTypes(format), handle)
}

/**
 * 导出【开始前】先让用户挑保存位置。
 *
 * 为什么必须提前挑：异步导出要跑几十秒到几分钟，跑完时浏览器的"用户手势"早就过期，
 * 那时再调另存为对话框会被安全策略拒绝（SecurityError）。在点按钮的这一刻挑，才拿得到。
 * 返回：FileSystemFileHandle / { canceled: true }（用户取消）/ null（不支持，走退回路径）
 */
export async function pickExportTarget(name, format) {
  return pickSaveTarget(exportFileName(name, format), pickerTypes(format))
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
export async function finishExport(state, connId, taskId, name, format, target = null) {
  if (state.status.value === 'success') {
    try {
      // 原生另存为（桌面壳 WebKit）：位置开跑前就选好了，直接让内核把产物拷过去，
      // 不必再把几十 MB 拉进前端内存
      if (target && target.nativePath) {
        const r = await exportSaveTo(connId, taskId, target.nativePath)
        if (!r || r.success === false) {
          throw new Error((r && r.message) || t('common.unknownError'))
        }
        ElMessage.success(t('mv.exportSavedAs', { name: target.nativeName || r.fileName || baseName(target.nativePath) }))
        return
      }
      const blob = await exportDownload(connId, taskId)
      const res = await saveExportBlob(blob, name, format, target)
      if (res.canceled) return
      if (!res.ok) {
        // saveBlobAs 已经提示过一次（res.notified），别叠加第二条
        if (!res.notified) ElMessage.error(t('mv.downloadFailed', { detail: errMsg(res.error) }))
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
  /* 还没有任务时是 idle，**不是** running：转储弹窗的「关闭 / 开始导出」按
     `status === 'running'` 禁用，初始写成 running 会让弹窗一打开两个按钮就是灰的
     （用户反馈「按钮怎么不能点」——任务根本没发起，却像正在跑）。
     真正的 running 由 start() → reset() 设置。 */
  const status = ref('idle')
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

  // 连续失败计数：后端重启 / 网络断开时原来只 console.error，
  // 于是 status 永远是 running、进度条永远转圈，用户只能去「后台任务」里猜。
  let pollFails = 0
  const startPolling = (connId, name, format, handle) => {
    if (timer) clearInterval(timer)
    pollFails = 0
    timer = setInterval(async () => {
      if (!taskId.value) return
      try {
        const r = await exportTask(connId, taskId.value)
        pollFails = 0
        const finished = applyTaskSnapshot(state, r)
        if (finished) {
          clearInterval(timer)
          timer = null
          // 终态写回任务中心记录（导出 / SQL 转存都在这里收尾）：
          // 关掉进度窗之后，任务中心那条记录也能立刻显示成功/失败与产物文件名
          setBgTaskStatus(taskId.value, status.value, {
            status: status.value,
            message: message.value,
            phase: phase.value,
            fileName: r.fileName,
            logs: logs.value.map(l => l.text),
            elapsedMs: r.elapsedMs
          })
          await finishExport(state, connId, taskId.value, name, format, handle)
        }
      } catch (e) {
        pollFails += 1
        console.warn('[export] 进度轮询失败：', pollFails, e?.message || e)
        // 容忍偶发抖动（一次失败就报错会吓人），连续 3 次才判定为真的断了
        if (pollFails >= 3) {
          clearInterval(timer)
          timer = null
          status.value = 'error'
          phase.value = t('et.pollFailed')
          message.value = errMsg(e)
          ElMessage.error(t('et.pollFailedMsg', { detail: errMsg(e) }))
        }
      }
    }, POLL_MS)
  }

  /**
   * 提交导出任务。
   *
   * opts（可选）：{ kind, title, database }
   *  - kind：'export'（表/查询结果导出，默认）或 'dump'（整库 SQL 转存）—— 任务中心据此分类
   *  - title：任务中心里显示的名字（一般带连接与库表名）
   * 提交成功后**立刻登记到任务中心**：进度窗被关掉、甚至切走页签，任务仍在后端跑，
   * 顶栏任务中心能看到它、能停止、结束后能下载产物。
   */
  const start = async (connId, payload, name, format, submitFn, opts = {}) => {
    // 先挑保存位置，再开始导出（异步导出跑完时已经没有用户手势，那时弹不出来）
    const target = await pickExportTarget(name, format)
    if (target && target.canceled) return
    if (target && target.failed) {
      ElMessage.error(t('mv.saveDialogFailed'))
      return
    }
    const handle = target && !target.canceled ? target : null
    reset(t('et.exporting', { name: (name || t('et.data')) }))
    visible.value = true
    try {
      const res = await (submitFn || exportStart)(connId, payload)
      if (!res.success || !res.taskId) {
        throw new Error(res.message || t('mv.exportSubmitFailed'))
      }
      taskId.value = res.taskId
      addBgTask({
        id: res.taskId,
        kind: opts.kind || 'export',
        title: opts.title || `${name || t('et.data')} · ${String(format || '').toUpperCase()}`,
        connId,
        database: opts.database || payload?.database || '',
        // name / format 给「任务中心 → 查看 → 下载」用：那个场景里保存句柄早没了，
        // 产物只能结束之后按原始表名重新拼文件名再下载
        extra: { name, format }
      })
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
