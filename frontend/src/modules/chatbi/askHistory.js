/**
 * 问数历史（本地持久化）—— **以对话为单位**。
 *
 * <p>问数走的是 `/api/ai/ask`（不落服务端会话），所以左栏「记录」里看不到它。
 * 这里把「对话 + 其中的每一轮问答」存进 localStorage：同一段对话里的追问
 * **并入同一条记录**（record.turns），只有点「新对话」才会生成新记录 ——
 * 否则连着追问五次，记录列表就被同一件事刷成五条，没法看。
 *
 * <p>点一条记录把它**整段铺回对话流**（每一轮的问题 + 回答卡片），
 * 之后继续追问仍然并回这条记录。
 *
 * <p>为什么要裁剪行：一次分组查询可能返回上千行，原样塞 localStorage 会顶到 5MB 配额，
 * 导致后续写入直接失败。这里每轮只留前 200 行 —— 回看历史时够看，又不至于把存储撑爆。
 *
 * <p>**另存一份到 dbmind.db（app_settings 的 `ai.askhistory`）**：桌面壳的窗口地址是
 * `http://127.0.0.1:<端口>`，端口每次启动可能不同，localStorage 又按 origin 隔离 ——
 * 关掉重开，记录里的问数对话就"凭空消失"（2026-11-19 用户实测反馈）。
 * 这份镜像只用于**换端口后把记录找回来**（只留最近若干条、每轮只留若干行），
 * 日常读写仍走 localStorage（同步、快）。
 */
import { getSettings, putAppSetting } from '../../api'

const KEY = 'dbmind.ask.history.v1'
/** 镜像到库里的键（跨端口恢复用） */
const DB_KEY = 'ai.askhistory'
/** 镜像只留这么多条 / 每条这么多轮 / 每轮这么多行：它只是"找回来"的底，不是全文备份 */
const MIRROR_RECORDS = 20
const MIRROR_TURNS = 10
const MIRROR_ROWS = 50
const MIRROR_DELAY = 800
const LIMIT = 40
const KEEP_ROWS = 200

const read = () => {
  try {
    const list = JSON.parse(localStorage.getItem(KEY) || '[]')
    return Array.isArray(list) ? list : []
  } catch {
    return []
  }
}

const write = (list) => {
  const kept = list.slice(0, LIMIT)
  try {
    localStorage.setItem(KEY, JSON.stringify(kept))
  } catch {
    // 配额满了就放弃保存，不影响当前这次问数 —— 历史是增强，不是主流程
  }
  scheduleMirror(kept)
}

// ===== 跨端口镜像（见文件头注释）=====
let mirrorTimer = null
/** 镜像体：条数与行数都再裁一次，别让一次 PUT 拖到几 MB */
const mirrorBody = (list) => (Array.isArray(list) ? list : []).slice(0, MIRROR_RECORDS).map((h) => ({
  ...h,
  // 老条目（一条记录 = 一轮）把问答放在顶层，这里统一补成 turns 再裁
  turns: (Array.isArray(h.turns) && h.turns.length
    ? h.turns
    : [{ q: h.question, answer: h.answer, ts: h.ts }]).slice(-MIRROR_TURNS).map((t) => ({
    ...t,
    answer: t.answer && Array.isArray(t.answer.rows)
      ? { ...t.answer, rows: t.answer.rows.slice(0, MIRROR_ROWS) }
      : t.answer
  }))
}))

const scheduleMirror = (list) => {
  if (mirrorTimer) clearTimeout(mirrorTimer)
  mirrorTimer = setTimeout(() => {
    mirrorTimer = null
    putAppSetting(DB_KEY, JSON.stringify(mirrorBody(list))).catch(() => { /* 写不进去不影响本机使用 */ })
  }, MIRROR_DELAY)
}

/**
 * 换端口后把记录找回来：本地为空、库里有时灌回去（只灌一次，本地有内容就不动）。
 * 面板挂载时调一次；后端没就绪时静默失败，下次再试。
 */
export const ensureAskHistoryStore = async () => {
  if (read().length) return
  try {
    const all = (await getSettings()) || {}
    const raw = all[DB_KEY]
    if (!raw) return
    const list = JSON.parse(raw)
    if (!Array.isArray(list) || !list.length) return
    localStorage.setItem(KEY, JSON.stringify(list.slice(0, LIMIT)))
  } catch { /* 解析失败 / 存储不可用：当作没有备份 */ }
}

/** 裁剪回答体：只保留回看需要的字段与有限行数，避免撑爆 localStorage */
const trimAnswer = (answer) => {
  if (!answer) return null
  const { rows, ...rest } = answer
  return {
    ...rest,
    streaming: false,
    rows: Array.isArray(rows) ? rows.slice(0, KEEP_ROWS) : [],
  }
}

/** 最近问数（新的在前） */
export const listAskHistory = () => read()

/**
 * 存一轮问数；返回这条**对话记录**。
 *
 * @param {string} question 本轮的问题
 * @param {object} answer   本轮的完整回答（askAnswer）
 * @param {string} threadId 所属对话；空 = 开新对话记录，非空 = 追加到那条记录的 turns 里
 */
export const saveAskHistory = (question, answer, threadId = '') => {
  const q = String(question || '').trim()
  if (!q) return null
  const turn = { q, answer: trimAnswer(answer), ts: Date.now() }
  const list = read()

  if (threadId) {
    const idx = list.findIndex((h) => h.id === threadId)
    if (idx >= 0) {
      const item = list[idx]
      // 老版本条目没有 turns（一条记录 = 一轮），先补齐再追加
      if (!Array.isArray(item.turns) || !item.turns.length) {
        item.turns = [{ q: item.question, answer: item.answer, ts: item.ts }]
      }
      // 同一个问题重问（重试）就地替换最后一轮；否则是一轮新追问
      const last = item.turns[item.turns.length - 1]
      if (last && last.q === q) {
        item.turns[item.turns.length - 1] = turn
      } else {
        item.turns.push(turn)
      }
      item.ts = turn.ts
      // question/answer 顶层字段保留：列表标题用第一问，回放兜底用最后一答
      item.answer = turn.answer
      item.lastQuestion = q
      write(list)
      return item
    }
  }

  const item = {
    id: threadId || `ask_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`,
    ts: turn.ts,
    question: q,
    answer: turn.answer,
    turns: [turn],
  }
  const deduped = list.filter((h) => h.id !== item.id)
  deduped.unshift(item)
  write(deduped)
  return item
}

/** 删除一条（= 删除整段对话记录） */
export const removeAskHistory = (id) => {
  write(read().filter((h) => h.id !== id))
}
