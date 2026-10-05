import { watch, onBeforeUnmount, ref, readonly } from 'vue'

/**
 * 结果表格「类 Excel 框选 + 复制」组合函数。
 *
 * 用法：
 * - 给表格的数据单元格 <td> 与列头 <th> 加 `:data-gkey="行号:列号"`（行号从 0 开始，表头为 0，数据从 1 起；列号 0 起）。
 * - 行号列、虚拟间隔、列宽把手等不需框选的单元格不要加 data-gkey。
 * - 组件在 setup 里调用：`const gridSel = useExcelSelection({ container, scrollHost, shouldStart })`；
 *   其中 shouldStart(e) 可选，返回 false 时禁止该处开始框选（如列宽拖拽边缘、编辑输入框）。
 *   scrollHost 为滚动容器 ref/Element，拖拽到边缘时会自动滚动。
 * - 组件里原有「点击选中行」等 click 处理器，若需避免框选拖拽误触发，可判 `gridSel.suppressClick()`。
 *
 * - valueAt(r, c) 可选：按 gkey 坐标回取值文本。传入后「复制 / 填充」等按**数据**取值，
 *   不再依赖 DOM —— 窗口化渲染下跨屏大选区也能完整复制（不传则退回读 DOM 文本）。
 *
 * 复制行为：Ctrl/Cmd+C 复制高亮区块为 TSV（若框选覆盖了表头行则一并包含列名）。
 */
export function useExcelSelection(options) {
  const { container, shouldStart = null, valueAt = null, onCopied = null } = options || {}
  const cellSelector = (r, c) => `[data-gkey="${r}:${c}"]`

  let anchor = null // {r,c}
  let focus = null
  let selecting = false
  let moved = false
  let suppressUntil = 0
  let paintRaf = 0
  let moveRaf = 0 // 拖动中「解析指针落在哪个单元格」的合帧任务
  let cellSet = new Set() // 当前被选中的 gkey，用于快速清空
  let docBound = false
  let dragMoveBound = false
  const range = ref(null)

  // 边缘自动滚动
  const EDGE = 24
  const MAX_SPEED = 600 // px/s
  let autoScrollRaf = 0
  let lastClientX = 0
  let lastClientY = 0
  let lastScrollTime = 0

  const cellEl = (r, c) => container.value && container.value.querySelector(cellSelector(r, c))

  const scrollHostEl = () => container.value

  const keyOf = (el) => {
    if (!el || !el.getAttribute) return null
    const g = el.getAttribute('data-gkey')
    if (!g) return null
    const p = g.split(':')
    return { r: parseInt(p[0], 10), c: parseInt(p[1], 10) }
  }

  const SEL_CLS = ['grid-cell-selected', 'sel-t', 'sel-b', 'sel-l', 'sel-r']

  // 标记选中单元格：grid-cell-selected 为填充，sel-t/b/l/r 标记该格位于选区哪条外沿（供 CSS 画边线）
  const markCell = (el, r, c, r1, r2, c1, c2) => {
    el.classList.add('grid-cell-selected')
    if (r === r1) el.classList.add('sel-t')
    if (r === r2) el.classList.add('sel-b')
    if (c === c1) el.classList.add('sel-l')
    if (c === c2) el.classList.add('sel-r')
    cellSet.add(`${r}:${c}`)
  }

  /**
   * 只在矩形真的变化时才写 range。
   *
   * range 是外部（两处 selEdges）的响应式依赖，而 paint() 在框选拖动 / 自动滚动时
   * 每一帧都会跑：每帧换一个新对象会让依赖它的上层每帧重算出新值，
   * 进而把可视区上千个单元格全部重渲染一次 —— 这是框选拖动最大的开销来源。
   * 选区没变时保持同一个对象，Vue 那边就完全不会被惊动。
   */
  let lastRangeKey = ''
  const setRange = (rect) => {
    const key = rect ? rect.r1 + ':' + rect.r2 + ':' + rect.c1 + ':' + rect.c2 : ''
    if (key === lastRangeKey) return
    lastRangeKey = key
    range.value = rect
  }

  const clear = (keepRange) => {
    if (!container.value) return
    for (const k of cellSet) {
      const el = container.value.querySelector(`[data-gkey="${k}"]`)
      if (el) el.classList.remove(...SEL_CLS)
    }
    // 兜底：窗口化渲染会复用/重建单元格，元素上可能残留选中类，一并扫掉
    container.value.querySelectorAll('.grid-cell-selected').forEach(el => el.classList.remove(...SEL_CLS))
    cellSet.clear()
    // paint() 里紧接着会写回同样的矩形，先置空只会白触发一次上层重算
    if (!keepRange) setRange(null)
  }

  const paint = () => {
    if (!anchor || !focus) { clear(); return }
    clear(true)
    const r1 = Math.min(anchor.r, focus.r)
    const r2 = Math.max(anchor.r, focus.r)
    const c1 = Math.min(anchor.c, focus.c)
    const c2 = Math.max(anchor.c, focus.c)
    // 只遍历 DOM 里真实存在的单元格：窗口化渲染下未渲染的行本就没有元素，
    // 而逐格 querySelector 在「跨几千行的选区」里会明显卡顿。
    const nodes = container.value ? container.value.querySelectorAll('[data-gkey]') : []
    for (const el of nodes) {
      const k = keyOf(el)
      if (!k || k.r < r1 || k.r > r2 || k.c < c1 || k.c > c2) continue
      markCell(el, k.r, k.c, r1, r2, c1, c2)
    }
    // clear() 会把 range 置空，重绘完必须按当前 anchor/focus 恢复，
    // 否则外部会以为「没有区域选区」（活动单元格描边会重新冒出来）
    setRange(rangeRect())
  }

  const schedulePaint = () => {
    if (paintRaf) return
    paintRaf = requestAnimationFrame(() => {
      paintRaf = 0
      paint()
    })
  }

  const buildText = () => {
    if (!anchor || !focus) return ''
    const r1 = Math.min(anchor.r, focus.r)
    const r2 = Math.max(anchor.r, focus.r)
    const c1 = Math.min(anchor.c, focus.c)
    const c2 = Math.max(anchor.c, focus.c)
    const lines = []
    for (let r = r1; r <= r2; r++) {
      const row = []
      for (let c = c1; c <= c2; c++) {
        if (valueAt) { row.push(valueAt(r, c)); continue }
        const el = cellEl(r, c)
        row.push(el ? el.textContent : '')
      }
      lines.push(row.join('\t'))
    }
    return lines.join('\n')
  }
  const rangeRect = () => {
    if (!anchor || !focus) return null
    return {
      r1: Math.min(anchor.r, focus.r),
      r2: Math.max(anchor.r, focus.r),
      c1: Math.min(anchor.c, focus.c),
      c2: Math.max(anchor.c, focus.c)
    }
  }

  const updateFocusFromPoint = (x, y) => {
    const host = scrollHostEl()
    let cell = null
    let el = document.elementFromPoint(x, y)
    if (el) cell = el.closest('th, td')
    if (!cell && host) {
      const rect = host.getBoundingClientRect()
      const probeX = x >= rect.right - EDGE ? rect.right - 8 : x <= rect.left + EDGE ? rect.left + 8 : x
      const probeY = y >= rect.bottom - EDGE ? rect.bottom - 8 : y <= rect.top + EDGE ? rect.top + 8 : y
      const probeEl = document.elementFromPoint(probeX, probeY)
      if (probeEl) cell = probeEl.closest('th, td')
    }
    if (cell) {
      const k = keyOf(cell)
      if (k && (k.r !== focus.r || k.c !== focus.c)) {
        focus = k
        moved = true
      }
    }
  }

  const stopAutoScroll = () => {
    if (autoScrollRaf) {
      cancelAnimationFrame(autoScrollRaf)
      autoScrollRaf = 0
    }
  }

  const autoScrollLoop = () => {
    const host = scrollHostEl()
    if (!selecting || !host) { stopAutoScroll(); return }
    const rect = host.getBoundingClientRect()
    let dx = 0
    let dy = 0
    if (lastClientX < rect.left + EDGE) dx = -1
    else if (lastClientX > rect.right - EDGE) dx = 1
    if (lastClientY < rect.top + EDGE) dy = -1
    else if (lastClientY > rect.bottom - EDGE) dy = 1
    if (!dx && !dy) { stopAutoScroll(); return }

    const now = performance.now()
    const dt = Math.min(now - lastScrollTime, 50)
    lastScrollTime = now

    const distX = dx === -1 ? (rect.left + EDGE - lastClientX) : dx === 1 ? (lastClientX - (rect.right - EDGE)) : 0
    const distY = dy === -1 ? (rect.top + EDGE - lastClientY) : dy === 1 ? (lastClientY - (rect.bottom - EDGE)) : 0
    const speedX = (distX / EDGE) * MAX_SPEED
    const speedY = (distY / EDGE) * MAX_SPEED

    if (dx) host.scrollLeft += dx * speedX * (dt / 1000)
    if (dy) host.scrollTop += dy * speedY * (dt / 1000)

    updateFocusFromPoint(lastClientX, lastClientY)
    schedulePaint()
    autoScrollRaf = requestAnimationFrame(autoScrollLoop)
  }

  const startAutoScroll = () => {
    if (autoScrollRaf) return
    lastScrollTime = performance.now()
    autoScrollRaf = requestAnimationFrame(autoScrollLoop)
  }

  const onDown = (e) => {
    if (e.button !== 0) return
    const cell = e.target.closest('th, td')
    if (!cell || !cell.closest('table')) return
    if (!cell.hasAttribute('data-gkey')) return
    const k = keyOf(cell)
    if (!k) return
    // Shift+点击 = 从已有锚点扩展出一片矩形区域（Excel 行为）：
    //   · 起点是上次点击/拖动的单元格，终点是这次 Shift+点击的单元格
    //   · 表头单元格（data-gkey 的第 0 行）也能作为终点 → 于是「含标题行的区域」也能用 Shift 点出来
    //   · 放在 shouldStart 之前判断，这样表头、列宽把手附近也允许扩展
    //   · 只压掉随后的 click（否则行/列点击会把这块选区替换掉）
    if (e.shiftKey && anchor) {
      focus = k
      selecting = true
      moved = false
      lastClientX = e.clientX
      lastClientY = e.clientY
      schedulePaint()
      setRange(rangeRect())
      suppressUntil = Date.now() + 250
      document.addEventListener('mousemove', onMove)
      document.addEventListener('mouseup', onUp, { once: true })
      dragMoveBound = true
      e.preventDefault()
      return
    }
    if (shouldStart && !shouldStart(e)) return
    anchor = k
    focus = k
    moved = false
    selecting = true
    lastClientX = e.clientX
    lastClientY = e.clientY
    clear() // 重新开始选中：先移除之前的高亮，只保留当前单击的单元格
    const el = cellEl(k.r, k.c)
    if (el) markCell(el, k.r, k.c, k.r, k.r, k.c, k.c)
    document.addEventListener('mousemove', onMove)
    document.addEventListener('mouseup', onUp, { once: true })
    dragMoveBound = true
    e.preventDefault()
  }

  const onMove = (e) => {
    if (!selecting || !anchor) return
    lastClientX = e.clientX
    lastClientY = e.clientY
    // 合并到一帧再处理：mousemove 的频率可能远高于刷新率（高刷鼠标 100+ 次/秒），
    // 而 updateFocusFromPoint 里的 elementFromPoint 会强制同步布局，逐次处理必然掉帧
    if (!moveRaf) {
      moveRaf = requestAnimationFrame(() => {
        moveRaf = 0
        if (!selecting || !anchor) return
        updateFocusFromPoint(lastClientX, lastClientY)
        schedulePaint()
      })
    }
    startAutoScroll()
  }

  const onUp = () => {
    if (!selecting) return
    // 松手时把还没落到选区里的最后一帧补上（拖得快时鼠标先松开、帧还没跑）
    if (moveRaf) {
      cancelAnimationFrame(moveRaf)
      moveRaf = 0
      updateFocusFromPoint(lastClientX, lastClientY)
    }
    if (dragMoveBound) {
      document.removeEventListener('mousemove', onMove)
      dragMoveBound = false
    }
    stopAutoScroll()
    const firedDrag = moved && anchor && focus && (anchor.r !== focus.r || anchor.c !== focus.c)
    if (firedDrag) suppressUntil = Date.now() + 150
    selecting = false
    moved = false
    if (paintRaf) { cancelAnimationFrame(paintRaf); paintRaf = 0 }
    setRange(rangeRect())
  }

  // 复制选中区块为 TSV
  const onCopy = (e) => {
    const rect = rangeRect()
    if (!rect) return
    const text = buildText()
    if (!text) return
    e.clipboardData.setData('text/plain', text)
    e.preventDefault()
    // 告诉外部这次复制了什么，便于按场景给提示（如「已复制选区（20 行 × 3 列，含列名）」）
    if (onCopied) {
      onCopied({
        rows: rect.r2 - rect.r1 + 1,
        cols: rect.c2 - rect.c1 + 1,
        header: rect.r1 === 0
      })
    }
  }

  // 点击表格以外区域时清空选区
  const onDocDown = (e) => {
    const t = e.target
    if (t && t.closest) {
      // 单元格 / 表头上：交给框选模块自己的按下逻辑（切换选区）
      if (t.closest('th, td')) return
      // 操作栏控件（导出 / 分页 / 收起…）：保留当前选区
      if (t.closest('button, .el-button, .el-dropdown, .el-checkbox, .el-pagination')) return
    }
    // 注意：表格容器的空白处（表格右侧、下方留白）也算「外部」→ 清空框选
    anchor = null
    focus = null
    clear()
  }

  // 滚动时重绘可见区域的选中高亮（不清空选区，否则自动滚动后立即丢失选中）
  const onScroll = () => {
    if (anchor && focus) schedulePaint()
  }

  const unbind = () => {
    if (moveRaf) { cancelAnimationFrame(moveRaf); moveRaf = 0 }
    const el = container.value
    if (el) {
      el.removeEventListener('mousedown', onDown)
      el.removeEventListener('scroll', onScroll)
    }
    if (dragMoveBound) {
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('mouseup', onUp)
      dragMoveBound = false
    }
    stopAutoScroll()
    if (docBound) {
      document.removeEventListener('copy', onCopy)
      document.removeEventListener('mousedown', onDocDown)
      docBound = false
    }
  }

  const bind = () => {
    unbind()
    const el = container.value
    if (!el) return
    el.addEventListener('mousedown', onDown)
    el.addEventListener('scroll', onScroll, { passive: true })
    document.addEventListener('copy', onCopy)
    document.addEventListener('mousedown', onDocDown)
    docBound = true
  }

  watch(container, () => bind(), { flush: 'post', immediate: true })
  onBeforeUnmount(() => { unbind(); clear() })

  return {
    clear,
    suppressClick: () => Date.now() < suppressUntil,
    copy: buildText,
    hasSelection: () => !!(anchor && focus),
    range: readonly(range),
    // 选区矩形（gkey 坐标，含表头行=0）；无选区时 null
    rect: rangeRect,
    // 当前锚点（键盘扩展选区时用来固定起点）
    anchor: () => (anchor ? { r: anchor.r, c: anchor.c } : null),
    // 程序化设置选区：键盘 Shift+方向键 / Ctrl+A 等直接驱动高亮，与鼠标框选走同一套渲染
    // （注意：这里的 setRange 是对外 API 名，方法体里调用的 setRange(rect) 是上面的内部写入器）
    setRange: (r1, c1, r2, c2) => {
      anchor = { r: r1, c: c1 }
      focus = { r: r2, c: c2 }
      selecting = false
      moved = false
      if (paintRaf) { cancelAnimationFrame(paintRaf); paintRaf = 0 }
      paint()
      setRange(rangeRect())
    },
    // 程序化清空选区（含锚点，避免滚动时又被重绘出来）
    clearRange: () => {
      anchor = null
      focus = null
      selecting = false
      moved = false
      clear()
    }
  }
}
