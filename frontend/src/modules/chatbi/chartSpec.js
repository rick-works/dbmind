// 图表规格 → ECharts option。
//
// 规格来自后端（`/api/ai/ask` 返回的 charts[]），形状固定：
//   { type, title, x, y:[{field,label,unit,axis,type}], series, stack, sort, limit }
// 这里负责三件事：
//   1. **认列**：行是 `{列名: 值}` 的对象（见后端 shape），按字段名取值，取不到就给 null；
//   2. **认类型**：line/area/bar/bar-stack/pie/donut/scatter/funnel/gauge/heat/table；
//   3. **兜底**：任何一步出问题都退回表格 —— 宁可给数据表，也不要给一块空白画布。
//
// 与后端 `normalize_chart` 的白名单一一对应，改类型要两边一起改。

/** 图表类型目录（界面上的「切换图表」按钮按它渲染） */
export const CHART_TYPES = [
  { key: 'bar', label: '柱状图', icon: 'Histogram' },
  { key: 'line', label: '折线图', icon: 'TrendCharts' },
  { key: 'area', label: '面积图', icon: 'DataLine' },
  { key: 'pie', label: '饼图', icon: 'PieChart' },
  { key: 'donut', label: '环形图', icon: 'PieChart' },
  { key: 'scatter', label: '散点图', icon: 'ScaleToOriginal' },
  { key: 'heat', label: '热力图', icon: 'Grid' },
  { key: 'funnel', label: '漏斗图', icon: 'Filter' },
  { key: 'gauge', label: '仪表盘', icon: 'Odometer' },
  { key: 'table', label: '数据表', icon: 'Grid' },
]

export const chartTypeLabel = (type) =>
  (CHART_TYPES.find((t) => t.key === type) || {}).label || '图表'

/** 数值格式化：图表轴/提示里用（1,284 万 比 12843210 好读得多） */
export function fmtNumber(value, unit = '') {
  if (value === null || value === undefined || value === '') return '-'
  const num = Number(value)
  if (!Number.isFinite(num)) return String(value)
  const abs = Math.abs(num)
  let text
  if (abs >= 1e8) text = `${trim(num / 1e8)} 亿`
  else if (abs >= 1e4) text = `${trim(num / 1e4)} 万`
  else if (Number.isInteger(num)) text = num.toLocaleString('zh-CN')
  else text = trim(num)
  return unit ? `${text}${unit}` : text
}

function trim(num) {
  const fixed = Math.abs(num) >= 100 ? num.toFixed(0) : num.toFixed(2)
  return String(Number(fixed))
}

/** 按字段取单元格值；行是对象，字段名可能大小写不一致 */
export function cell(row, field) {
  if (!row || !field) return null
  if (row[field] !== undefined) return row[field]
  const key = Object.keys(row).find((k) => k.toLowerCase() === String(field).toLowerCase())
  return key === undefined ? null : row[key]
}

const toNumber = (value) => {
  if (value === null || value === undefined || value === '') return null
  const num = Number(value)
  return Number.isFinite(num) ? num : null
}

const shortLabel = (text, max = 12) => {
  const s = String(text ?? '')
  return s.length > max ? `${s.slice(0, max)}…` : s
}

/**
 * 规格 + 结果集 → echarts option。
 *
 * @param spec   后端给的图表规格
 * @param data   { columns, columnTypes, rows }
 * @param theme  { text, sub, grid, card, palette } 颜色（由调用方从 CSS 变量取）
 * @param overrideType 用户手动切换的类型（覆盖 spec.type）
 */
export function buildOption(spec, data, theme, overrideType) {
  const type = overrideType || (spec && spec.type) || 'bar'
  // 用户手动切了类型时，系列级的 y[].type（模型标的 line/bar）要让位：
  // 否则模型给单系列标了 y.type='line' 后，切到柱状图仍被钉死成折线
  const eff = overrideType && spec
    ? { ...spec, y: ((spec.y || []).map((y) => (y && y.type ? { ...y, type: undefined } : y))) }
    : spec
  const rows = (data && data.rows) || []
  const yList = ((eff && eff.y) || []).filter((y) => y && y.field)
  const base = {
    backgroundColor: 'transparent',
    animationDuration: 420,
    textStyle: { color: theme.text, fontFamily: 'inherit' },
    color: theme.palette,
    tooltip: {
      trigger: type === 'pie' || type === 'donut' || type === 'funnel' ? 'item' : 'axis',
      backgroundColor: theme.tipBg,
      borderColor: theme.grid,
      textStyle: { color: theme.text, fontSize: 12 },
      axisPointer: { type: type === 'bar' || type === 'bar-stack' ? 'shadow' : 'line' },
    },
    legend: {
      top: 0,
      left: 'center',
      icon: 'roundRect',
      itemWidth: 10,
      itemHeight: 10,
      textStyle: { color: theme.sub, fontSize: 11 },
    },
    grid: { left: 8, right: 12, top: 34, bottom: 6, containLabel: true },
  }

  if (!eff || !rows.length || !yList.length) {
    return { ...base, title: { text: '没有可绘制的数据', left: 'center', top: 'middle', textStyle: { color: theme.sub, fontSize: 13 } } }
  }

  // 行数截断：图表上的点超过 limit 就只取前 N（明细表仍给全部）
  const limit = Math.max(1, Math.min(Number(eff.limit) || 50, 500))
  let list = rows.slice(0, type === 'gauge' ? 1 : Math.max(limit, type === 'pie' || type === 'donut' || type === 'funnel' ? 20 : limit))

  if (type === 'pie' || type === 'donut' || type === 'funnel' || type === 'gauge') {
    return buildShare(type, eff, list, base, theme)
  }
  if (type === 'scatter') return buildScatter(eff, list, yList, base, theme)
  if (type === 'heat') return buildHeat(eff, list, yList, base, theme)
  return buildAxis(type, eff, list, yList, base, theme)
}

function buildShare(type, spec, rows, base, theme) {
  const y = spec.y[0]
  const items = rows
    .map((row) => ({ name: String(cell(row, spec.x) ?? '-'), value: toNumber(cell(row, y.field)) }))
    .filter((item) => item.value !== null)
  if (type === 'gauge') {
    const value = items[0]?.value ?? 0
    return {
      ...base,
      tooltip: { ...base.tooltip, formatter: () => `${items[0]?.name || ''}：${fmtNumber(value, y.unit)}` },
      series: [{
        type: 'gauge',
        progress: { show: true, width: 14 },
        axisLine: { lineStyle: { width: 14, color: [[1, theme.grid]] } },
        axisTick: { show: false },
        splitLine: { show: false },
        axisLabel: { color: theme.sub, distance: 18, fontSize: 10 },
        pointer: { width: 4 },
        detail: {
          valueAnimation: true,
          fontSize: 22,
          offsetCenter: [0, '62%'],
          color: theme.text,
          formatter: (v) => fmtNumber(v, y.unit),
        },
        data: [{ value, name: shortLabel(items[0]?.name || y.label, 16) }],
      }],
    }
  }
  const inner = type === 'donut' ? '58%' : '0%'
  return {
    ...base,
    tooltip: {
      ...base.tooltip,
      trigger: 'item',
      formatter: (p) => `${p.name}<br/>${fmtNumber(p.value, y.unit)}（${p.percent.toFixed(1)}%）`,
    },
    series: [{
      type: type === 'funnel' ? 'funnel' : 'pie',
      radius: type === 'funnel' ? undefined : [inner, '72%'],
      center: ['50%', '56%'],
      left: type === 'funnel' ? '12%' : undefined,
      width: type === 'funnel' ? '76%' : undefined,
      sort: type === 'funnel' ? 'descending' : undefined,
      minSize: type === 'funnel' ? '12%' : undefined,
      itemStyle: { borderColor: theme.card, borderWidth: 2, borderRadius: type === 'funnel' ? 4 : 0 },
      label: { color: theme.sub, fontSize: 11, formatter: (p) => `${p.name}  ${p.percent.toFixed(0)}%` },
      labelLine: { length: 10, length2: 8 },
      data: items,
    }],
  }
}

function buildScatter(spec, rows, yList, base, theme) {
  const y = yList[0]
  const points = rows
    .map((row) => [toNumber(cell(row, spec.x)), toNumber(cell(row, y.field)), cell(row, spec.series) ?? ''])
    .filter((p) => p[0] !== null && p[1] !== null)
  return {
    ...base,
    xAxis: {
      type: 'value',
      name: shortLabel(spec.x, 14),
      nameTextStyle: { color: theme.sub, fontSize: 11 },
      axisLine: { lineStyle: { color: theme.grid } },
      axisLabel: { color: theme.sub, fontSize: 10, formatter: (v) => fmtNumber(v) },
      splitLine: { lineStyle: { color: theme.grid, type: 'dashed' } },
    },
    yAxis: {
      type: 'value',
      name: shortLabel(y.label || y.field, 14),
      nameTextStyle: { color: theme.sub, fontSize: 11 },
      axisLine: { show: false },
      axisLabel: { color: theme.sub, fontSize: 10, formatter: (v) => fmtNumber(v, y.unit) },
      splitLine: { lineStyle: { color: theme.grid, type: 'dashed' } },
    },
    series: [{
      type: 'scatter',
      symbolSize: 9,
      data: points,
      itemStyle: { opacity: 0.82 },
    }],
  }
}

function buildHeat(spec, rows, yList, base, theme) {
  const y = yList[0]
  const xs = [...new Set(rows.map((r) => String(cell(r, spec.x) ?? '-')))]
  const ys = spec.series
    ? [...new Set(rows.map((r) => String(cell(r, spec.series) ?? '-')))]
    : [shortLabel(y.label || y.field, 12)]
  const data = []
  let max = 0
  for (const row of rows) {
    const xi = xs.indexOf(String(cell(row, spec.x) ?? '-'))
    const yi = spec.series ? ys.indexOf(String(cell(row, spec.series) ?? '-')) : 0
    const value = toNumber(cell(row, y.field))
    if (xi < 0 || yi < 0 || value === null) continue
    max = Math.max(max, value)
    data.push([xi, yi, value])
  }
  return {
    ...base,
    tooltip: {
      ...base.tooltip,
      trigger: 'item',
      formatter: (p) => `${xs[p.value[0]]} / ${ys[p.value[1]]}<br/>${fmtNumber(p.value[2], y.unit)}`,
    },
    grid: { left: 8, right: 12, top: 30, bottom: 46, containLabel: true },
    xAxis: {
      type: 'category',
      data: xs,
      splitArea: { show: true },
      axisLabel: { color: theme.sub, fontSize: 10, formatter: (v) => shortLabel(v, 8) },
      axisLine: { lineStyle: { color: theme.grid } },
    },
    yAxis: {
      type: 'category',
      data: ys,
      splitArea: { show: true },
      axisLabel: { color: theme.sub, fontSize: 10, formatter: (v) => shortLabel(v, 8) },
      axisLine: { lineStyle: { color: theme.grid } },
    },
    visualMap: {
      min: 0,
      max: max || 1,
      calculable: true,
      orient: 'horizontal',
      left: 'center',
      bottom: 0,
      itemHeight: 80,
      textStyle: { color: theme.sub, fontSize: 10 },
      inRange: { color: theme.heat },
    },
    series: [{ type: 'heatmap', data, emphasis: { itemStyle: { shadowBlur: 6, shadowColor: 'rgba(0,0,0,.3)' } } }],
  }
}

function buildAxis(type, spec, rows, yList, base, theme) {
  const categories = rows.map((row) => String(cell(row, spec.x) ?? '-'))
  const stacked = !!spec.stack || type === 'bar-stack'
  const isArea = type === 'area'
  const isLine = type === 'line' || isArea
  const hasRight = yList.some((y) => Number(y.axis) === 1)

  // 长表转宽表：spec.series 有值且结果里真有这列时，按它拆成多条系列
  const seriesValues = spec.series
    ? [...new Set(rows.map((row) => String(cell(row, spec.series) ?? '-')))]
    : []

  const series = []
  if (seriesValues.length && seriesValues.length <= 12) {
    const y = yList[0]
    for (const group of seriesValues) {
      series.push({
        name: group,
        type: isLine ? 'line' : 'bar',
        stack: stacked ? 'total' : undefined,
        smooth: isLine,
        symbol: 'circle',
        symbolSize: 5,
        showSymbol: rows.length <= 30,
        areaStyle: isArea ? { opacity: 0.18 } : undefined,
        emphasis: { focus: 'series' },
        data: rows.filter((row) => String(cell(row, spec.series) ?? '-') === group)
          .map((row) => toNumber(cell(row, y.field))),
      })
    }
  } else {
    for (const y of yList) {
      series.push({
        name: y.label || y.field,
        type: y.type === 'line' || isLine ? 'line' : 'bar',
        yAxisIndex: Number(y.axis) === 1 ? 1 : 0,
        stack: stacked ? 'total' : undefined,
        smooth: isLine,
        symbol: 'circle',
        symbolSize: 5,
        showSymbol: rows.length <= 30,
        barMaxWidth: 34,
        areaStyle: isArea ? { opacity: 0.18 } : undefined,
        emphasis: { focus: 'series' },
        data: rows.map((row) => toNumber(cell(row, y.field))),
      })
    }
  }

  const axisCommon = {
    axisLine: { lineStyle: { color: theme.grid } },
    axisTick: { show: false },
    axisLabel: { color: theme.sub, fontSize: 10 },
    splitLine: { lineStyle: { color: theme.grid, type: 'dashed' } },
  }
  const yAxis = [{
    ...axisCommon,
    type: 'value',
    axisLine: { show: false },
    axisLabel: {
      color: theme.sub,
      fontSize: 10,
      formatter: (v) => fmtNumber(v, yList[0]?.unit || ''),
    },
  }]
  if (hasRight) {
    const right = yList.find((y) => Number(y.axis) === 1)
    yAxis.push({
      ...axisCommon,
      type: 'value',
      position: 'right',
      axisLine: { show: false },
      splitLine: { show: false },
      axisLabel: { color: theme.sub, fontSize: 10, formatter: (v) => fmtNumber(v, right?.unit || '') },
    })
  }

  const option = {
    ...base,
    tooltip: {
      ...base.tooltip,
      valueFormatter: (value) => fmtNumber(value),
    },
    xAxis: {
      ...axisCommon,
      type: 'category',
      data: categories,
      splitLine: { show: false },
      axisLabel: {
        color: theme.sub,
        fontSize: 10,
        hideOverlap: true,
        // 横轴标签太长会把图挤扁：旋转 + 省略，配合下面的 dataZoom 还能拖
        rotate: categories.some((c) => c.length > 6) ? 28 : 0,
        formatter: (v) => shortLabel(v, 10),
      },
    },
    yAxis,
    series,
  }
  // 点多的时候给一个缩放条：不然 200 根柱子挤成一坨（依然保留总览）
  if (categories.length > 24) {
    option.dataZoom = [
      { type: 'inside', start: 0, end: Math.min(100, (24 / categories.length) * 100) },
      {
        type: 'slider',
        height: 14,
        bottom: 2,
        start: 0,
        end: Math.min(100, (24 / categories.length) * 100),
        borderColor: theme.grid,
        fillerColor: 'rgba(90,140,255,.15)',
        handleStyle: { color: theme.palette[0] },
        textStyle: { color: theme.sub, fontSize: 10 },
      },
    ]
    option.grid = { ...option.grid, bottom: 24 }
  }
  return option
}

/** 从 CSS 变量取图表配色（应用是暗色主题，取不到时用兜底色） */
export function chartTheme() {
  const css = getComputedStyle(document.documentElement)
  const pick = (name, fallback) => (css.getPropertyValue(name) || '').trim() || fallback
  return {
    text: pick('--dc-text', '#e6e8ee'),
    sub: pick('--dc-text-2', '#98a0b3'),
    grid: pick('--dc-border', '#2f3440'),
    card: pick('--dc-bg-card', '#242833'),
    tipBg: pick('--dc-bg-raised', '#22262f'),
    palette: ['#5b8cff', '#38c9a8', '#f5a623', '#ff6b6b', '#a78bfa', '#4dd0e1', '#f06292', '#9ccc65', '#ffb74d', '#7986cb'],
    heat: ['#1c2433', '#2f4c8c', '#4b7bec', '#7aa7ff', '#d8e5ff'],
  }
}
