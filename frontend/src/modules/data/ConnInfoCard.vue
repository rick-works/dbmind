<!--
连接节点的悬浮信息卡。

**为什么单独一个组件（性能）**：它原先内联在 MainView 的树插槽里，而卡片的位置/开合是
响应式状态（connCard）—— 鼠标每进出一次连接节点就要改 5 次响应式数据，于是
**整个 MainView 重渲染** → el-tree 收到新的插槽函数 → **已渲染的全部节点**
（大库展开时上千个）插槽函数重跑一遍。表现就是「鼠标扫过连接，整棵树卡一下」。

现在状态住在本组件里：MainView 只调 open() / close()（命令式，不读这里的任何状态），
悬浮因此不再进入 MainView 的渲染依赖，树也就不跟着重渲染了。

定位与外观沿用原先的规则：
- 贴连接行**右侧 10px**、**竖直中心对齐节点中心**（用户要求「往上挪、尽量居中」）；
- 右侧放不下就翻到行左侧，左侧那个菱形尖改朝右（`.conn-info-card.flip`）；
- 卡片高度不定（备注 / 只读 / 长 URL 折行都会加行）：先按行数估算，渲染后再按**实测高度**
  校正一次；
- 卡片样式（含 pointer-events:none 的穿透）仍在 `styles/index.css` 里，这里不加 scoped 样式。
-->
<template>
  <Teleport to="body">
    <span v-if="show" class="conn-info-card" :class="{ flip }" :ref="setEl"
          :style="{ left: x + 'px', top: y + 'px' }">
      <!-- 卡片左侧的小菱形「尖」：指向这条连接；flip 时改朝右 -->
      <span class="ci-arrow"></span>
      <span class="ci-row"><b>{{ $t('connTip.name') }}</b>{{ conn.name }}</span>
      <span class="ci-row"><b>{{ $t('connTip.url') }}</b>{{ url || '—' }}</span>
      <span class="ci-row"><b>{{ $t('connTip.host') }}</b>{{ conn.host || '—' }}</span>
      <span class="ci-row"><b>{{ $t('connTip.port') }}</b>{{ conn.port || '—' }}</span>
      <span class="ci-row"><b>{{ $t('connTip.user') }}</b>{{ conn.username || '—' }}</span>
      <span class="ci-row"><b>{{ $t('connTip.type') }}</b>{{ labelOf(conn.connType) }}</span>
      <span class="ci-row"><b>{{ $t('connTip.env') }}</b>{{ envTitle(conn.env) }}</span>
      <!-- 备注与只读原先挂在节点上的原生 title（小气泡），现在都在这张卡里 -->
      <span v-if="conn.note" class="ci-row"><b>{{ $t('connTip.note') }}</b>{{ conn.note }}</span>
      <span v-if="conn.readOnly" class="ci-row"><b>{{ $t('connTip.readOnly') }}</b>{{ $t('cd.readOnlyTip') }}</span>
    </span>
  </Teleport>
</template>

<script setup>
import { ref, onUpdated } from 'vue'
import { labelOf } from '../../types'
import { envTitle } from '../../utils/envs'

// 卡片宽度与单行高度的**估算**值：只用于渲染前先摆个位置（渲染后会用实测高度校正）。
const CARD_W = 340
const CARD_ROW_H = 17.4

const show = ref(false)
const flip = ref(false)
const x = ref(0)
const y = ref(0)
const conn = ref({})
const url = ref('')

let cardEl = null
const setEl = (el) => { cardEl = el }
let rowRect = { top: 0, height: 0, left: 0, right: 0 }

/** 估算高度：7 行固定 + 备注 / 只读各一行。 */
const estCardH = (c) => 32 + CARD_ROW_H * (7 + (c && c.note ? 1 : 0) + (c && c.readOnly ? 1 : 0))
/** 让卡片竖直中心对齐节点中心，并夹在视口内（上不出界、下不出界）。 */
const centerY = (rect, h) => Math.max(8, Math.min(rect.top + rect.height / 2 - h / 2, window.innerHeight - h - 8))

/**
 * 打开卡片。`data` 是连接节点数据，`info.rect` 是该行的矩形（视口坐标），
 * `info.url` 由调用方算好（connUrl 是 MainView 的局部函数，不便搬过来）。
 */
const open = (data, info = {}) => {
  conn.value = data || {}
  url.value = info.url || ''
  const r = info.rect || rowRect
  rowRect = r
  let left = r.right + 10
  flip.value = false
  if (left + CARD_W > window.innerWidth) {
    left = Math.max(8, r.left - CARD_W - 10)
    flip.value = true
  }
  x.value = left
  y.value = centerY(r, estCardH(conn.value))
  show.value = true
}

/**
 * 渲染后按**实测高度**校正一次位置（估算值在长 URL 折行时会差十几像素）。
 * 用 onUpdated 而不是让调用方回填：一次挂载、一次校正，之后 want 与 y 相等就不再写 ——
 * 不写就不会再触发更新，天然收敛，没有循环风险。
 */
onUpdated(() => {
  if (!show.value || !cardEl) return
  const want = centerY(rowRect, cardEl.offsetHeight)
  if (Math.abs(want - y.value) > 0.5) y.value = want
})

/**
 * 关卡片。带 id：鼠标从连接 A 快速划到连接 B 时，leave(A) 可能晚于 enter(B) 到达 ——
 * 不比对 id 就会把 B 刚打开的卡片一起关掉（表现为「划过去没反应」）。
 */
const close = (id) => {
  if (id != null && String(id) !== String((conn.value || {}).connId || '')) return
  show.value = false
}

defineExpose({ open, close })
</script>
