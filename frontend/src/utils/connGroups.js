import { ENV_ORDER_BASE, envLabel } from './envs'

/**
 * 连接清单 → 下拉框用的分组结构（`[{ label, options }]`）。
 *
 * 分组口径与左侧对象树**完全一致**：取连接配置里的 `group` 字段
 * （后端读连接时会把没设分组的补成默认分组「数据源」，见 shape.rs）。
 * `group` 可能是多级路径（`公司资源/子组`，`/` 分隔）—— 下拉里没有层级，
 * 就用完整路径当组名，避免两个同名子组混成一组。
 *
 * 顺序也照树来：自定义目录（按名）→ 预置 DEV / TEST / PROD → 未分组（排最后）；
 * 组内保持清单原顺序（后端按名称返回）。
 */
export const groupConnections = (list) => {
  const map = new Map()
  for (const c of list || []) {
    const key = String((c && c.group) || '').trim()
    if (!map.has(key)) map.set(key, [])
    map.get(key).push(c)
  }
  // 排序权重：自定义目录 0 / 预置环境 1（按 DEV→TEST→PROD）/ 未分组 2
  const rank = (key) => {
    if (!key) return 2
    const i = ENV_ORDER_BASE.indexOf(key)
    return i === -1 ? 0 : 1
  }
  return [...map.entries()]
    .sort(([a], [b]) => {
      const ra = rank(a), rb = rank(b)
      if (ra !== rb) return ra - rb
      if (ra === 0) return a.localeCompare(b, 'zh-Hans-CN')
      if (ra === 1) return ENV_ORDER_BASE.indexOf(a) - ENV_ORDER_BASE.indexOf(b)
      return 0
    })
    .map(([key, options]) => ({ label: envLabel(key), options }))
}
