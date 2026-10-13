<!--
树上「表 / 视图」的注释悬浮卡。

**为什么是独立组件 + Teleport，而不是 el-tooltip**（用户反馈「弹在里面、位置不对」）：
树容器是 `overflow-y:auto / overflow-x:hidden / contain:layout` —— 气泡会被**裁在容器里**，
于是它贴着左边、还压在树上。连接悬浮卡早就为此搬成了「Teleport 到 body + fixed + JS 摆位」，
这里沿用同一套：贴行右侧 10px、竖直居中对齐节点、右侧放不下就翻到左侧（箭头改朝右），
外观与连接卡完全一致（同一组 CSS 变量与数值）。

宽度**按内容自适应**（用户要求）：`width: max-content`，上限 340px（长注释才折行）。
-->
<template>
  <Teleport to="body">
    <span v-if="show" class="obj-info-card" :class="{ flip }" :ref="setEl"
          :style="{ left: x + 'px', top: y + 'px' }">
      <!-- 左侧小菱形「尖」：与连接卡同款（8px 方块转 45°），flip 时改朝右 -->
      <span class="oi-arrow"></span>
      <span v-if="comment" class="oi-text">{{ comment }}</span>
      <span v-else class="oi-none">{{ $t('tree.tip.noComment') }}</span>
    </span>
  </Teleport>
</template>

<script setup>
import { ref, onUpdated } from 'vue'

const show = ref(false)
const flip = ref(false)
const x = ref(0)
const y = ref(0)
const comment = ref('')

let cardEl = null
const setEl = (el) => { cardEl = el }
let rowRect = { top: 0, height: 0, left: 0, right: 0 }

/**
 * 摆位。宽度/高度都取**实测值**（卡片按内容自适应，估不出来），
 * 且只有真的变了才写响应式数据 —— 否则 onUpdated → 摆位 → 更新 会自激。
 */
const place = () => {
  const w = cardEl ? cardEl.offsetWidth : 160
  const h = cardEl ? cardEl.offsetHeight : 30
  let left = rowRect.right + 10
  let nextFlip = false
  if (left + w > window.innerWidth - 8) {
    left = Math.max(8, rowRect.left - w - 10)
    nextFlip = true
  }
  const top = Math.max(8, Math.min(rowRect.top + rowRect.height / 2 - h / 2, window.innerHeight - h - 8))
  if (nextFlip !== flip.value) flip.value = nextFlip
  if (Math.abs(left - x.value) > 0.5) x.value = left
  if (Math.abs(top - y.value) > 0.5) y.value = top
}

/** 当前卡片对应的节点 id（close 时比对，防止"划过去被前一个的 leave 关掉"） */
let currentId = ''

/** 打开卡片。`data` 是树节点数据（用它的 comment），`info.rect` 是该行的视口矩形。 */
const open = (data, info = {}) => {
  currentId = (data && data.id) || ''
  comment.value = String((data && data.comment) || '').trim()
  rowRect = info.rect || rowRect
  // 先按"上一个位置"放着（避免在旧坐标闪一下），渲染后 onUpdated 会用实测尺寸校正
  x.value = rowRect.right + 10
  y.value = Math.max(8, rowRect.top)
  show.value = true
}

/**
 * 关卡片。带 id：鼠标从表 A 快速划到表 B 时，leave(A) 可能晚于 enter(B) 到达 ——
 * 不比对 id 就会把 B 刚打开的卡片一起关掉（表现为「划过去没反应」）。
 */
const close = (id) => {
  if (id != null && String(id) !== String(currentId)) return
  show.value = false
}

defineExpose({ open, close })

onUpdated(() => {
  if (show.value && cardEl) place()
})
</script>
