<template>
  <!-- 表头排序图标：两枚细箭头（上/下），未排序时都很淡，排序列只亮对应方向。
       比 Element 自带的 Sort/SortUp/SortDown 更轻，不抢字段名的视觉重量 -->
  <svg
    class="sort-glyph"
    :class="stateClass"
    width="10"
    height="16"
    viewBox="0 0 10 16"
    aria-hidden="true"
  >
    <path class="g-up" d="M2.4 6.1 5 3.5l2.6 2.6" />
    <path class="g-down" d="M2.4 9.9 5 12.5l2.6-2.6" />
  </svg>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  /** '' = 未排序；'asc' = 升序；'desc' = 降序 */
  state: { type: String, default: '' }
})

const stateClass = computed(() => (props.state === 'asc' ? 'is-asc' : props.state === 'desc' ? 'is-desc' : ''))
</script>

<style scoped>
.sort-glyph {
  display: block;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.sort-glyph .g-up,
.sort-glyph .g-down {
  transition: opacity .14s ease, stroke-width .14s ease;
}
/* 未排序：两枚箭头并重提亮 —— 用户口径「颜色亮一点、看得清楚些」，
   容器那边再用 --dc-text-weak + opacity 控制整体轻重 */
.sort-glyph .g-up { opacity: .85; }
.sort-glyph .g-down { opacity: .85; }
/* 排序列：只留当前方向（另一侧完全隐去），一眼看出是升还是降，不留"幽灵箭头" */
.sort-glyph.is-asc .g-up { opacity: 1; stroke-width: 2.1; }
.sort-glyph.is-asc .g-down { opacity: 0; }
.sort-glyph.is-desc .g-down { opacity: 1; stroke-width: 2.1; }
.sort-glyph.is-desc .g-up { opacity: 0; }
</style>
