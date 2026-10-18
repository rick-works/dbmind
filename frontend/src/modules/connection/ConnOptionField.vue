<template>
  <!-- 一个「类型连接选项」字段：字段声明来自 src/types/<type>.js 的 options（见 presets.js）。
       说明文字不做常驻的小字，改成标签后面的问号 —— 鼠标悬浮才显示，版面清爽。
       必填规则统一：下拉必选（有默认值且不可为空）一律带星号；非必填的「可选」写在输入框里，
       标题后面不再挂小字（用户要求：框里有就行）。 -->
  <el-form-item class="opt-item" :required="isRequired">
    <template #label>
      <span class="opt-label">{{ label }}</span>
      <el-tooltip v-if="tip" :content="tip" placement="top" :show-after="120" popper-class="opt-tip-pop">
        <el-icon class="opt-q"><QuestionFilled /></el-icon>
      </el-tooltip>
    </template>
    <el-select v-if="o.kind === 'select'" :model-value="modelValue" style="width: 100%"
               @update:model-value="$emit('update:modelValue', $event)">
      <el-option v-for="op in selectOptions" :key="op.value" :value="op.value"
                 :label="op.labelKey ? $t(op.labelKey) : op.label" />
    </el-select>
    <el-input v-else-if="o.kind === 'number'" :model-value="modelValue" type="number"
              :placeholder="placeholder"
              @update:model-value="$emit('update:modelValue', $event)" />
    <el-input v-else :model-value="modelValue" :placeholder="placeholder"
              @update:model-value="$emit('update:modelValue', $event)" />
  </el-form-item>
</template>

<script setup>
import { computed } from 'vue'
import { QuestionFilled } from '@element-plus/icons-vue'
import { t } from '../../utils/i18n'

const props = defineProps({
  o: { type: Object, required: true },
  modelValue: { type: [String, Number], default: '' }
})
defineEmits(['update:modelValue'])

const label = computed(() => (props.o.labelKey ? t(props.o.labelKey) : (props.o.label || '')))
const tip = computed(() => (props.o.tipKey ? t(props.o.tipKey) : ''))
const placeholder = computed(() => props.o.placeholder || '')
// 下拉（连接方式 / 协议）都是必选项：有默认值、不可为空 → 一律标星号
const isRequired = computed(() => props.o.required === true || props.o.kind === 'select')

// 已存连接里的值可能不在下拉清单内（老版本存的、或用户手填过的），
// 补一个同名选项顶上去 —— 否则下拉框显示原值却选不回来，一改就永久丢掉。
const selectOptions = computed(() => {
  const list = (props.o.options || []).slice()
  const cur = props.modelValue == null ? '' : String(props.modelValue)
  if (cur && !list.some((op) => String(op.value) === cur)) list.unshift({ value: cur, label: cur })
  return list
})
</script>

<style scoped>
.opt-label { white-space: nowrap; }
.opt-q {
  margin-left: 2px; font-size: 12px; color: var(--el-text-color-placeholder);
  cursor: help; vertical-align: -1px; flex: none;
}
.opt-q:hover { color: var(--el-color-primary); }
</style>

<style>
/* tooltip 内容偏长，给个上限，免得弹出条横贯整个屏幕 */
.opt-tip-pop { max-width: 320px; line-height: 1.5; }
</style>
