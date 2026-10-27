<template>
  <!-- 连接选择器：**分组 → 连接 两级**，组头可点开/收起。
       全项目「选择连接」的地方都用这一个组件，保证分组口径、字号、交互一致
       （SQL 查询 / 数据传输 / 数据对比 / 用户管理 / 对象清单 / AI 助手 / AI 治理）。

       实现要点：el-select 的选项列表只认 el-option（自带的 el-option-group 组头不可点、
       也画不出箭头），所以组头用一个 **disabled 的 el-option** 借位 —— el-select 不会选中它、
       键盘导航也会跳过，内层 div 用 @click.stop 自己接点击；子项用 v-if 挂载/卸载实现收起。 -->
  <el-select
    ref="selRef"
    :model-value="modelValue"
    :size="size"
    :disabled="disabled"
    :clearable="clearable"
    :filterable="filterable"
    :filter-method="filterable ? onFilter : undefined"
    :placeholder="placeholder"
    popper-class="conn-tree-popper"
    class="conn-tree-select"
    @update:model-value="(v) => emit('update:modelValue', v)"
    @change="(v) => emit('change', v)"
    @visible-change="onVisibleChange"
  >
    <template #prefix>
      <slot name="prefix">
        <DbLogo v-if="selected" :type="selected.type" :size="16" class="sel-logo" />
      </slot>
    </template>

    <template v-for="g in shownGroups" :key="g.label">
      <el-option class="ct-group-head" :value="'__ct_group__' + g.label" disabled>
        <div class="ct-group" @click.stop="toggleGroup(g.label)">
          <el-icon class="ct-caret" :class="{ 'is-open': g.expanded }"><CaretRight /></el-icon>
          <span class="ct-group-name">{{ g.label }}</span>
          <span class="ct-group-num">{{ g.count }}</span>
        </div>
      </el-option>
      <template v-if="g.expanded">
        <el-option v-for="c in g.options" :key="String(c.id)" :label="c.name || ''" :value="valueOf(c)">
          <div class="opt-row">
            <DbLogo :type="c.type" :size="16" />
            <span class="opt-name">{{ c.name }}</span>
            <el-icon v-if="isSelected(c)" class="ct-check"><Select /></el-icon>
          </div>
        </el-option>
      </template>
    </template>
  </el-select>
</template>

<script setup>
/**
 * 连接下拉（分组 → 连接 两级，组头可展开/收起）。
 *
 * <p>分组口径与左侧对象树一致（utils/connGroups：连接配置的 group，预置 DEV/TEST/PROD，未分组垫底）。
 *
 * <p>样式统一在 styles/index.css 的 `.conn-tree-popper` 段里（浮层 teleport 到 body，必须全局样式），
 * 这里不再写 popper 样式 —— 各页面直接用，字号/缩进/组头自然一致。
 */
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { CaretRight, Select } from '@element-plus/icons-vue'
import DbLogo from './DbLogo.vue'
import { groupConnections } from '../utils/connGroups'
import { fitSelectWidth } from '../utils/selectAutoWidth'
import { editorSettingsLive } from '../utils/settings'

const props = defineProps({
  /** 选中的连接 id（字符串或数字，与各页面既有类型保持一致） */
  modelValue: { type: [String, Number], default: '' },
  /** 连接清单（后端形状：id / name / type / group） */
  connections: { type: Array, default: () => [] },
  disabled: { type: Boolean, default: false },
  clearable: { type: Boolean, default: false },
  /** 可输入筛选：筛到的组会自动展开，组头在筛选时隐藏 */
  filterable: { type: Boolean, default: false },
  /** 选项值转成 String（用户管理 / 对象清单的既有口径） */
  stringValue: { type: Boolean, default: false },
  placeholder: { type: String, default: '' },
  size: { type: String, default: 'small' },
  /** 选择框宽度跟着选中值自适应（默认开；少数需要拉满整行的版面可关掉） */
  autoWidth: { type: Boolean, default: true },
  widthMin: { type: Number, default: 130 },
  widthMax: { type: Number, default: 300 },
})
const emit = defineEmits(['update:modelValue', 'change'])

const selRef = ref(null)
/** 展开状态：按组名记（组名=目录名），默认展开 */
const groupOpen = ref({})
const filterText = ref('')

/* ---------- 宽度自适应：连接名长短差别很大，固定宽度不是截断就是留一大片空白 ----------
   量法见 utils/selectAutoWidth（Element 的 selected-item 是 overflow:hidden，
   CSS 的 width:max-content 量不出来）。弹窗/页签是挂载后才可见的，所以挂载时量一次、
   再等过渡动画结束补量一次。 */
const fit = () => {
  if (!props.autoWidth) return
  nextTick(() => fitSelectWidth(selRef.value?.$el, { min: props.widthMin, max: props.widthMax }))
}
onMounted(() => { fit(); setTimeout(fit, 400) })
watch(() => [props.modelValue, props.connections, props.autoWidth, props.widthMin, props.widthMax], fit, { deep: true })
// 全局字号变了，同样长度的名字会变宽 —— 跟着重量一次
watch(() => [editorSettingsLive.value.dataFontSize, editorSettingsLive.value.fontSize], fit)

const groups = computed(() => groupConnections(props.connections).map((g) => ({ label: g.label, options: g.options })))
const isOpen = (label) => groupOpen.value[label] !== false
const toggleGroup = (label) => {
  // 筛选态下所有组都是展开的（否则筛出来的项藏在收起的组里，等于没搜到）
  if (filterText.value.trim()) return
  groupOpen.value[label] = !isOpen(label)
}

const shownGroups = computed(() => {
  const q = filterText.value.trim().toLowerCase()
  if (!q) {
    return groups.value.map((g) => ({ label: g.label, options: g.options, count: g.options.length, expanded: isOpen(g.label) }))
  }
  const hit = (c) => [c.name, c.type, c.host].some((v) => String(v || '').toLowerCase().includes(q))
  return groups.value
    .map((g) => ({ label: g.label, options: g.options.filter(hit), expanded: true }))
    .filter((g) => g.options.length)
    .map((g) => ({ ...g, count: g.options.length }))
})

const selected = computed(() => {
  const cur = String(props.modelValue ?? '')
  if (!cur) return null
  return props.connections.find((c) => String(c.id) === cur) || null
})
const isSelected = (c) => String(props.modelValue ?? '') !== '' && String(c.id) === String(props.modelValue ?? '')
const valueOf = (c) => (props.stringValue ? String(c.id) : c.id)

const onFilter = (q) => { filterText.value = q || '' }

/** 打开时强制展开「当前选中连接」所在组：收起过之后也不会看不到自己选的是谁 */
const expandSelectedGroup = () => {
  const cur = String(props.modelValue ?? '')
  if (!cur) return
  const g = groups.value.find((x) => x.options.some((c) => String(c.id) === cur))
  if (g) groupOpen.value[g.label] = true
}
const onVisibleChange = (visible) => {
  if (visible) expandSelectedGroup()
  else filterText.value = '' // 关掉就清空筛选，下次打开还是完整分组
}
watch(() => [props.modelValue, props.connections], expandSelectedGroup, { deep: true })

defineExpose({ focus: () => selRef.value?.focus?.() })
</script>
