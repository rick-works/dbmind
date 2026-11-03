<template>
  <div ref="host" class="chart-canvas" :style="{ height: `${height}px` }"></div>
</template>

<script setup>
// ECharts 按需引入：只装问数用得到的图与组件。
// 全量 `import * as echarts from 'echarts'` 会把 ~1MB 拖进包里，而问数页是懒加载页签 ——
// 进来就要等的首屏，省这 700KB 是值得的。
import * as echarts from 'echarts/core'
import {
  BarChart, LineChart, PieChart, ScatterChart, FunnelChart, GaugeChart, HeatmapChart,
} from 'echarts/charts'
import {
  GridComponent, TooltipComponent, LegendComponent, DataZoomComponent,
  VisualMapComponent, MarkLineComponent, GraphicComponent,
} from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'
import { onBeforeUnmount, onMounted, ref, shallowRef, watch, nextTick } from 'vue'
import { buildOption, chartTheme } from './chartSpec'

echarts.use([
  BarChart, LineChart, PieChart, ScatterChart, FunnelChart, GaugeChart, HeatmapChart,
  GridComponent, TooltipComponent, LegendComponent, DataZoomComponent,
  VisualMapComponent, MarkLineComponent, GraphicComponent, CanvasRenderer,
])

const props = defineProps({
  spec: { type: Object, default: null },
  data: { type: Object, default: () => ({ columns: [], rows: [], columnTypes: [] }) },
  typeOverride: { type: String, default: '' },
  height: { type: Number, default: 300 },
})

const host = ref(null)
const chart = shallowRef(null)
let observer = null

const render = () => {
  if (!chart.value || !props.spec) return
  const option = buildOption(props.spec, props.data, chartTheme(), props.typeOverride)
  // notMerge=true：切换图表类型时旧系列的 dataZoom / visualMap 必须彻底清掉，
  // 否则从热力图切到柱状图会残留一块颜色条
  chart.value.setOption(option, true)
}

const ensure = () => {
  if (chart.value || !host.value) return
  chart.value = echarts.init(host.value, null, { renderer: 'canvas' })
  render()
}

onMounted(async () => {
  await nextTick()
  ensure()
  if (host.value && typeof ResizeObserver !== 'undefined') {
    let raf = 0
    observer = new ResizeObserver(() => {
      // 页签切换/侧栏折叠会连着触发多次，合并到一帧里 resize
      cancelAnimationFrame(raf)
      raf = requestAnimationFrame(() => chart.value && chart.value.resize())
    })
    observer.observe(host.value)
  }
})

watch(() => [props.spec, props.data, props.typeOverride], async () => {
  await nextTick()
  ensure()
  render()
}, { deep: true })

onBeforeUnmount(() => {
  if (observer) observer.disconnect()
  if (chart.value) chart.value.dispose()
  chart.value = null
})

defineExpose({
  resize: () => chart.value && chart.value.resize(),
  /** 导出 PNG（dataURL）：给卡片上的「下载图片」用 */
  toDataURL: () => (chart.value ? chart.value.getDataURL({ pixelRatio: 2, backgroundColor: 'transparent' }) : ''),
})
</script>

<style scoped>
.chart-canvas { width: 100%; min-height: 120px; }
</style>
