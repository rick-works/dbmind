/**
 * 问数历史（本地持久化）。
 *
 * <p>问数走的是 `/api/ai/ask`（不落服务端会话），所以左栏「记录」里看不到它。
 * 这里把每次「问题 + 完整回答」存进 localStorage，让左侧也能形成可回看的历史：
 * 点一条就把它原样铺回对话流，和刚问完时长得一样。
 *
 * <p>为什么要裁剪行：一次分组查询可能返回上千行，原样塞 localStorage 会顶到 5MB 配额，
 * 导致后续写入直接失败。这里只留前 200 行 —— 回看历史时够看，又不至于把存储撑爆。
 */
const KEY = 'dbmind.ask.history.v1'
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
  try {
    localStorage.setItem(KEY, JSON.stringify(list.slice(0, LIMIT)))
  } catch {
    // 配额满了就放弃保存，不影响当前这次问数 —— 历史是增强，不是主流程
  }
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

/** 存一次问数；返回新条目 */
export const saveAskHistory = (question, answer) => {
  const item = {
    id: `ask_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`,
    ts: Date.now(),
    question: String(question || '').trim(),
    answer: trimAnswer(answer),
  }
  if (!item.question) return null
  const list = read()
  // 同一个问题连问多次只留最新一条，避免列表被重复项刷屏
  const deduped = list.filter((h) => h.question !== item.question)
  deduped.unshift(item)
  write(deduped)
  return item
}

/** 删除一条 */
export const removeAskHistory = (id) => {
  write(read().filter((h) => h.id !== id))
}
