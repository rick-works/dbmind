/**
 * 连接的「目录树」摊平工具 —— 导出弹窗与连接管理弹窗共用。
 *
 * 连接归属的目录存在 `group` 里，可以是多级路径（`公司资源/研发`）。这里把它拆成
 * 一族可渲染的行：目录行（带深度与子项数）+ 连接行（带深度），由调用方自己决定
 * 每行长什么样（导出要勾选框 + 口令提示，管理要只读标签）。
 *
 * 为什么不直接用 el-tree：这两个弹窗的"行"是自定义的组合控件（勾选框里塞图标、类型、
 * 主机、标签），用 el-tree 的 render 反而更绕；层级最多两三级，按深度加 padding 足够。
 */

/** 连接的目录路径分段；空数组 = 未分组 */
export const connGroupPath = (conn) => String(conn && conn.group != null ? conn.group : '')
  .trim()
  .split('/')
  .map((seg) => seg.trim())
  .filter(Boolean)

/** 搜索匹配：名称 / 主机 / 类型 / 目录都能命中 */
const matchKeyword = (conn, kw) => !kw
  || String(conn.name || '').toLowerCase().includes(kw)
  || String(conn.host || '').toLowerCase().includes(kw)
  || String(conn.type || '').toLowerCase().includes(kw)
  || String(conn.group || '').toLowerCase().includes(kw)

/**
 * 生成树形行（深度优先：目录在前、连接在后）。
 *
 * @param {object} opts
 * @param {object[]} opts.conns 连接列表（已经过权限/搜索之外的过滤）
 * @param {string} [opts.keyword] 搜索词（空 = 不过滤）
 * @param {Set<string>} [opts.collapsed] 已折叠的目录全路径
 * @param {string} [opts.ungrouped] 「未分组」的显示名
 * @returns {Array<{type:'dir'|'conn', key:string, depth:number, name?:string, dirKey?:string, count?:number, conn?:object}>}
 */
export function buildConnTreeRows({ conns = [], keyword = '', collapsed = new Set(), ungrouped = '' } = {}) {
  const kw = String(keyword || '').trim().toLowerCase()
  const root = { children: new Map(), conns: [] }
  for (const conn of conns) {
    if (!matchKeyword(conn, kw)) continue
    const path = connGroupPath(conn)
    if (!path.length) path.push(ungrouped)
    let node = root
    for (const seg of path) {
      if (!node.children.has(seg)) node.children.set(seg, { children: new Map(), conns: [] })
      node = node.children.get(seg)
    }
    node.conns.push(conn)
  }

  // 目录下所有连接 id（含子目录）：目录行据此做三态勾选，一次全选整组
  const collectIds = (node, out) => {
    for (const conn of node.conns) out.push(conn.id)
    for (const child of node.children.values()) collectIds(child, out)
    return out
  }

  // 「未分组」永远垫底，其余按中文名排（连接是按创建顺序回来的，直接渲染会很随意）
  const sortedEntries = (node) => [...node.children.entries()].sort((a, b) => {
    if ((a[0] === ungrouped) !== (b[0] === ungrouped)) return a[0] === ungrouped ? 1 : -1
    return a[0].localeCompare(b[0], 'zh-Hans-CN')
  })

  const rows = []
  const walk = (node, depth, prefix) => {
    for (const [name, child] of sortedEntries(node)) {
      const dirKey = prefix ? prefix + '/' + name : name
      const connIds = collectIds(child, [])
      rows.push({ type: 'dir', key: 'd:' + dirKey, dirKey, name, depth, count: connIds.length, connIds })
      // 搜索时不认折叠：否则命中的连接会被藏起来，看着像"搜不到"
      if (kw || !collapsed.has(dirKey)) walk(child, depth + 1, dirKey)
    }
    for (const conn of node.conns) rows.push({ type: 'conn', key: 'c:' + conn.id, conn, depth })
  }
  walk(root, 0, '')
  return rows
}
