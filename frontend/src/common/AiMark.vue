<template>
  <!--
    机器人标记（AI 助手的统一图标）。
    为什么自己画：Element 自带的 MagicStick 是「魔法棒」，全项目拿它当 AI 图标用，
    但跟品牌不搭、也看不出是助手。这里用**线描小机器人**（天线 + 眼睛 + 嘴），
    单色 currentColor 描边，天然跟随主题色 / 文字色，1em 缩放所以任何字号下都对齐。
    用法：<AiMark />（跟随父级字号）、<AiMark :size="16" />（固定像素）、
          <el-button :icon="AiMark"> …（Element 会自己套一层 el-icon）
  -->
  <!-- 注意：宽高必须走内联 style。scoped 里的 .ai-mark { width/height: 1em } 是 CSS 规则，
       会盖掉 SVG 的 width/height 属性，导致 :size 传了也不生效（曾按字体大小渲染成 14px） -->
  <svg class="ai-mark" viewBox="0 0 24 24" :width="px" :height="px" :style="boxStyle" aria-hidden="true"
       fill="none" stroke="currentColor" stroke-width="2"
       stroke-linecap="round" stroke-linejoin="round">
    <!-- 天线球 + 短杆 -->
    <circle cx="12" cy="2.8" r="1.6" fill="currentColor" stroke="none" />
    <path d="M12 5.4V4.3" />
    <!-- 圆头（按用户给的参考造型：大圆脸 + 两颗大眼珠；原 SVG 是 1024 网格的彩色插画，
         细线在 14px 下会糊成一片，这里只保留轮廓与比例） -->
    <circle cx="12" cy="13" r="7.5" />
    <!-- 两侧耳块：跟头圆重叠一点，14px 下才像是"机器"而不是一张笑脸 -->
    <rect x="1.9" y="10.7" width="3.2" height="4.2" rx="1.6" />
    <rect x="18.9" y="10.7" width="3.2" height="4.2" rx="1.6" />
    <!-- 眼睛 -->
    <circle cx="9.5" cy="12.2" r="1.85" fill="currentColor" stroke="none" />
    <circle cx="14.5" cy="12.2" r="1.85" fill="currentColor" stroke="none" />
    <!-- 微笑 -->
    <path d="M10.2 16.2c.55.85 1.1 1.3 1.8 1.3s1.25-.45 1.8-1.3" />
  </svg>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  // 不给就按 1em 跟随父级字号；给了就是固定像素（数字或 '14px'）
  size: { type: [Number, String], default: '' }
})

const px = computed(() => {
  const v = props.size
  if (v === '' || v == null) return undefined
  return typeof v === 'number' ? v + 'px' : String(v)
})

// 有 size 时才写内联宽高（内联样式优先级高于 scoped 的 1em），没给就交给 1em 跟随字号
const boxStyle = computed(() => (px.value ? { width: px.value, height: px.value } : null))
</script>

<style scoped>
.ai-mark {
  width: 1em; height: 1em;
  display: inline-block;
  vertical-align: -0.135em;
  flex: none;
  overflow: visible;
}
</style>
