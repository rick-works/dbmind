<!--
  用量脚注：一条 AI 回答实际花了多少 token。

  为什么要抽成组件：这个数字现在至少出现在两处（SQL 查询的 AI 弹窗、右侧 AI 助手对话），
  以前各写各的 —— 一处是「本次回答消耗 491 tokens（428）」（括号里那串没人知道是什么），
  一处是「本次消耗 4,390 tokens （输入 4,233 · 输出 157）」。同一个事实，两种长相。

  三条规矩：
  - 上游没给 usage 就整块不渲染（宁可没有，也不要编一个数字出来）；
  - 总数字略亮、输入/输出降一档，数字统一千分位、等宽；
  - 与上方正文用虚线隔开；对齐方式由 align 决定（弹窗右对齐，聊天气泡跟随正文）。
-->
<template>
  <div v-if="hasUsage" class="ai-usage" :class="['al-' + align, { 'no-rule': !rule }]">
    <el-icon :size="13"><Coin /></el-icon>
    <span class="au-total">{{ $t('ai.usageTotal', { n: fmt(usage.totalTokens) }) }}</span>
    <span v-if="hasSplit" class="au-split">
      {{ $t('ai.usageInOut', { input: fmt(usage.promptTokens), output: fmt(usage.completionTokens) }) }}
    </span>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { Coin } from '@element-plus/icons-vue'

const props = defineProps({
  /** 后端 usage：{ promptTokens, completionTokens, totalTokens } */
  usage: { type: Object, default: null },
  /** right = 弹窗右对齐；left = 聊天气泡里跟随正文 */
  align: { type: String, default: 'right' },
  /** 是否画上方那条虚线（容器本身已有分隔时可关掉） */
  rule: { type: Boolean, default: true },
})

const hasUsage = computed(() => {
  const u = props.usage
  if (!u) return false
  return u.totalTokens != null || u.promptTokens != null || u.completionTokens != null
})
const hasSplit = computed(() => {
  const u = props.usage
  return !!u && (u.promptTokens != null || u.completionTokens != null)
})
/** 千分位；缺字段显示 —，**不显示 0**（0 与「没统计到」是两回事） */
const fmt = (n) => (n == null || !Number.isFinite(Number(n)) ? '—' : Number(n).toLocaleString())
</script>

<style scoped>
.ai-usage {
  display: flex; align-items: center; gap: 6px;
  margin-top: 12px; padding-top: 8px;
  border-top: 1px dashed var(--dc-border-soft);
  font-size: 12px; line-height: 1.6; color: var(--dc-text-dim);
  font-variant-numeric: tabular-nums;
}
.ai-usage.al-right { justify-content: flex-end; }
.ai-usage.al-left { justify-content: flex-start; }
.ai-usage.no-rule { border-top: none; padding-top: 0; }
.ai-usage .au-total { color: var(--dc-text); opacity: .72; }
.ai-usage .au-split { opacity: .85; }
.ai-usage :deep(.el-icon) { flex: none; }
</style>
