// 数据传输的**后台任务**注册表：
// 对话框点「后台运行」后任务在后端继续跑，这里记下 taskId 让顶栏任务中心能找回它。
// 列表持久化到 localStorage（页面刷新后仍能看到），状态由任务中心打开时实时查询。
import { reactive, watch } from 'vue'

const KEY = 'dbmind.bgSyncTasks'

const load = () => {
  try {
    const list = JSON.parse(localStorage.getItem(KEY) || '[]')
    return Array.isArray(list) ? list : []
  } catch {
    return []
  }
}

export const bgTasks = reactive(load())

const persist = () => {
  try { localStorage.setItem(KEY, JSON.stringify(bgTasks)) } catch { /* 隐私模式忽略 */ }
}

export const addBgTask = ({ id, title, kind }) => {
  if (!id) return
  const i = bgTasks.findIndex(x => x.id === id)
  const prev = i >= 0 ? bgTasks[i] : null
  // kind：任务属于哪个功能（sync=数据传输 / compare=数据对比）——
  // 任务中心点「查看」时按它路由回对应对话框的进度窗
  //
  // **标题保底**：同 id 重复登记（从任务中心恢复进度窗后又点了一次「后台运行」）
  // 时，此时对话框里的源/目标可能已被清空，生成出来的是「. → .」这种空标题 ——
  // 只保留**有效的**新标题（含可读字符），否则沿用旧标题，别把好标题冲掉
  const valid = (s) => s && /[\u4e00-\u9fa5A-Za-z0-9]/.test(s)
  const nextTitle = valid(title) ? title : (valid(prev?.title) ? prev.title : (title || id))
  const item = { id, title: nextTitle, kind: kind || prev?.kind || 'sync', addedAt: prev?.addedAt || Date.now() }
  if (i >= 0) bgTasks.splice(i, 1, item)
  else bgTasks.unshift(item)
  if (bgTasks.length > 50) bgTasks.splice(50)
}

export const removeBgTask = (id) => {
  const i = bgTasks.findIndex(x => x.id === id)
  if (i >= 0) bgTasks.splice(i, 1)
}

/** 清空全部执行记录 */
export const clearBgTasks = () => {
  bgTasks.splice(0)
}

/** 状态留存：查到终态后写回记录（持久化），页面刷新后历史状态不丢。
 *  snapshot：任务快照（message/rowsRead/rowsWritten/rowsFailed/result）——
 *  后端任务在内存里（重启/30 分钟过期就没了），**终态结果本地留底**，
 *  只要用户不删记录，恢复时就能看到当时的结果（真机反馈：不要"只保留 30 分钟"） */
export const setBgTaskStatus = (id, status, snapshot) => {
  const i = bgTasks.findIndex(x => x.id === id)
  if (i < 0) return
  bgTasks[i].status = status
  if (['success', 'canceled', 'error', 'notfound'].includes(status)) {
    const started = bgTasks[i].addedAt || Date.now()
    if (snapshot?.elapsedMs > 0) {
      // **结束时间 = 开始时间 + 真实运行时长**（快照里的 elapsedMs）—— 用"发现终态的
      // 那一刻"当结束时间会把结束写成开始（前端轮询/恢复有延迟，真机踩过）
      bgTasks[i].finishedAt = started + snapshot.elapsedMs
    } else if (!bgTasks[i].finishedAt || bgTasks[i].finishedAt <= started) {
      // 中断（无快照，不知道真实结束点）：取**第一次确认中断的时刻**，之后不再变；
      // 旧版本写坏的值（结束 ≤ 开始）也在这里纠正
      bgTasks[i].finishedAt = Date.now()
    }
    if (snapshot && typeof snapshot === 'object') {
      // result 里可能带大样本（对比的差异数据），超 100KB 就不存 —— 记录别把 localStorage 撑爆
      try {
        // elapsedMs：后端记的任务运行时长 —— 已结束任务的耗时用它（本地 addedAt
        // 是登记时刻，晚于任务真正开始，用它会算出虚高的耗时）
        // **status 必须留在快照里**：「查看」判断记录是否已终态就靠它 ——
        // 曾经漏存，导致已完成的任务点查看判断不出终态、又落回进度视图（真机踩过）
        const keep = { status, message: snapshot.message || '', rowsRead: snapshot.rowsRead || 0, rowsWritten: snapshot.rowsWritten || 0, rowsFailed: snapshot.rowsFailed || 0, elapsedMs: snapshot.elapsedMs || 0 }
        // 日志也留底（查看历史任务时日志区有内容）：总量控制 < 40KB，防撑爆 localStorage
        if (Array.isArray(snapshot.logs) && snapshot.logs.length) {
          try {
            if (JSON.stringify(snapshot.logs).length < 40 * 1024) keep.logs = snapshot.logs
          } catch { /* 序列化失败就放弃日志 */ }
        }
        if (snapshot.result !== undefined && JSON.stringify(snapshot.result).length < 100 * 1024) keep.result = snapshot.result
        bgTasks[i].snapshot = keep
      } catch { /* 序列化失败就只留状态 */ }
    }
  }
}

// 列表变化即持久化（统一在这里写 localStorage，不用每处手写）
watch(bgTasks, persist, { deep: true })
