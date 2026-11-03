/**
 * 把流式步骤事件合并进「执行过程」数组。
 *
 * <p>后端对同一步发两条：先 `running`（在做什么），跑完再发 `done`（真实 label + 明细 + 耗时）。
 * 按 `key` 原地升级，而不是追加 —— 否则界面会冒出「执行查询」两条，一条永远转圈。
 *
 * <p>`running` 帧不带明细，`done` 帧才带；合并时保留已有字段，避免把已知信息抹掉。
 *
 * @param {Array} steps 目标数组（就地修改，配合 Vue 的响应式）
 * @param {Object} event 后端推来的 step 帧
 * @returns {Array} 同一个数组，方便链式使用
 */
export const mergeStep = (steps, event) => {
  if (!event || !event.key) return steps
  const index = steps.findIndex((s) => s && s.key === event.key)
  const prev = index >= 0 ? steps[index] : null
  const next = {
    key: event.key,
    label: event.label || (prev ? prev.label : ''),
    status: event.status || 'done',
  }
  const detail = event.detail !== undefined && event.detail !== null ? event.detail : (prev ? prev.detail : undefined)
  if (detail !== undefined && detail !== '') next.detail = detail
  for (const field of ['input', 'output', 'code', 'thinking']) {
    const v = event[field]
    if (v !== undefined && v !== null && v !== '') next[field] = v
    else if (prev && prev[field]) next[field] = prev[field]
  }
  const ms = event.ms !== undefined && event.ms !== null ? event.ms : (prev ? prev.ms : undefined)
  if (ms !== undefined) next.ms = ms
  if (index >= 0) steps.splice(index, 1, next)
  else steps.push(next)
  return steps
}
