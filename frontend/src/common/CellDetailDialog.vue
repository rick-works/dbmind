<template>
  <el-dialog :model-value="modelValue" width="760px" top="8vh" append-to-body class="cell-detail-dialog"
             @update:model-value="$emit('update:modelValue', $event)">
    <template #header>
      <div class="cd-title">
        <span class="el-icon"><el-icon><Tickets /></el-icon></span>
        <span class="cd-name">{{ title || $t('cdd.title') }}</span>
        <span class="cd-meta">{{ $t('cdd.meta', { chars: charCount, lines: lineCount }) }}</span>
      </div>
    </template>
    <div class="cd-toolbar">
      <el-button v-if="isJson && !jsonPretty" size="small" @click="jsonPretty = true">{{ $t('cdd.prettyJson') }}</el-button>
      <el-button v-if="jsonPretty" size="small" @click="jsonPretty = false">{{ $t('cdd.rawText') }}</el-button>
      <el-button size="small" @click="wrapOn = !wrapOn">{{ wrapOn ? $t('cdd.noWrap') : $t('cdd.wrap') }}</el-button>
      <el-button size="small" type="primary" @click="copyAll">{{ $t('cdd.copyAll') }}</el-button>
    </div>
    <div class="cd-body" :class="{ wrap: wrapOn }"><pre>{{ displayText }}</pre></div>
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { Tickets } from '@element-plus/icons-vue'
import { t } from '../utils/i18n'

/**
 * 单元格详情弹窗：长文本 / JSON 的完整查看器。
 * 结果表与表数据浏览共用 —— 单元格里只显示省略文本，这里提供
 * 可滚动、可换行切换、可格式化 JSON、可复制全文的完整视图。
 */
const props = defineProps({
  modelValue: Boolean,
  title: { type: String, default: '' },
  text: { type: String, default: '' }
})
defineEmits(['update:modelValue'])

const wrapOn = ref(true)
const jsonPretty = ref(false)
watch(() => props.modelValue, (v) => { if (v) { wrapOn.value = true; jsonPretty.value = false } })

const isJson = computed(() => {
  const s = (props.text || '').trim()
  if (!s || !/^[[{"]/.test(s)) return false
  try { JSON.parse(s); return true } catch { return false }
})
const prettyText = computed(() => {
  if (!jsonPretty.value) return props.text || ''
  try { return JSON.stringify(JSON.parse(props.text), null, 2) } catch { return props.text || '' }
})
const displayText = computed(() => prettyText.value.replace(/\r\n/g, '\n'))
const charCount = computed(() => displayText.value.length)
const lineCount = computed(() => displayText.value.split('\n').length)

const copyAll = async () => {
  try {
    await navigator.clipboard.writeText(displayText.value)
    ElMessage.success(t('cdd.copied'))
  } catch {
    ElMessage.error(t('sqlq.copyFailed'))
  }
}
</script>

<style scoped>
.cd-title { display: flex; align-items: center; gap: 8px; min-width: 0; }
.cd-title .el-icon { color: var(--dc-primary); display: inline-flex; }
.cd-name { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.cd-meta { font-size: 12px; color: var(--dc-text-dim); flex-shrink: 0; margin-left: auto; }
.cd-toolbar { display: flex; gap: 8px; margin-bottom: 8px; }
.cd-body {
  max-height: 62vh; overflow: auto; border: 1px solid var(--dc-border); border-radius: 6px;
  background: var(--dc-bg-code); padding: 10px 12px;
}
.cd-body pre {
  margin: 0; font-family: "Cascadia Code", Consolas, "JetBrains Mono", monospace;
  font-size: 13px; line-height: 1.6; color: var(--dc-text);
  white-space: pre; /* 默认不换行：保留原文排版，横向滚动查看 */
  user-select: text; cursor: text;
}
.cd-body.wrap pre { white-space: pre-wrap; word-break: break-all; }
</style>
