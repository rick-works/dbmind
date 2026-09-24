<template>
  <teleport to="body">
    <div v-if="visible" class="grid-ctx-menu" :class="{ 'ctx-dark': dark }"
         :style="{ left: x + 'px', top: y + 'px' }" @contextmenu.prevent>
      <template v-for="(it, i) in items" :key="i">
        <div v-if="it.divided" class="grid-ctx-sep" />
        <div v-else class="grid-ctx-item" :class="{ disabled: it.disabled, danger: it.danger }"
             @click="onSelect(it)" @mouseenter="hovered = it.key" @mouseleave="hovered = ''">
          <el-icon v-if="it.icon" class="grid-ctx-ic"><component :is="it.icon" /></el-icon>
          <span class="grid-ctx-label">{{ it.label }}</span>
          <span v-if="it.shortcut" class="grid-ctx-sc">{{ it.shortcut }}</span>
          <el-icon v-else-if="it.children" class="grid-ctx-caret"><ArrowRight /></el-icon>
          <!-- 二级菜单（如「复制为」） -->
          <div v-if="it.children && hovered === it.key" class="grid-ctx-sub">
            <div v-for="(c, ci) in it.children" :key="ci"
                 class="grid-ctx-item" :class="{ disabled: c.disabled }"
                 @click.stop="onSelect(c)">
              <el-icon v-if="c.icon" class="grid-ctx-ic"><component :is="c.icon" /></el-icon>
              <span class="grid-ctx-label">{{ c.label }}</span>
            </div>
          </div>
        </div>
      </template>
    </div>
  </teleport>
</template>

<script setup>
import { watch, onBeforeUnmount, ref } from 'vue'
import { ArrowRight } from '@element-plus/icons-vue'

const props = defineProps({
  visible: Boolean,
  x: Number,
  y: Number,
  items: { type: Array, default: () => [] },
  // 跟随主题：暗色下用深色菜单
  dark: { type: Boolean, default: false }
})
const emit = defineEmits(['select', 'close'])
const hovered = ref('')

const onSelect = (it) => {
  if (it.disabled || it.children) return
  emit('select', it.key)
  emit('close')
}
const onDocDown = (e) => {
  if (e.target.closest && e.target.closest('.grid-ctx-menu')) return
  emit('close')
}
const onKey = (e) => { if (e.key === 'Escape') emit('close') }

watch(() => props.visible, (v) => {
  if (v) {
    document.addEventListener('mousedown', onDocDown, true)
    document.addEventListener('keydown', onKey)
  } else {
    document.removeEventListener('mousedown', onDocDown, true)
    document.removeEventListener('keydown', onKey)
    hovered.value = ''
  }
})
onBeforeUnmount(() => {
  document.removeEventListener('mousedown', onDocDown, true)
  document.removeEventListener('keydown', onKey)
})
</script>

<style scoped>
.grid-ctx-menu {
  position: fixed;
  z-index: 4000;
  min-width: 168px;
  max-width: 280px;
  background: var(--dc-bg-card, #fff);
  border: 1px solid var(--dc-border, #e4e7ed);
  border-radius: 10px;
  padding: 6px;
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.18);
  font-size: 14px;
  user-select: none;
}
.grid-ctx-sep { height: 1px; background: var(--dc-border, #ebeef5); margin: 4px 2px; }
.grid-ctx-item {
  position: relative;
  display: flex; align-items: center; gap: 8px;
  padding: 7px 10px; border-radius: 7px; cursor: pointer;
  color: var(--dc-text, #303133);
}
.grid-ctx-item:hover { background: var(--dc-primary-wash, #ecf3ff); color: var(--dc-primary, #409eff); }
.grid-ctx-item.disabled { color: var(--dc-text-dim, #bbb); cursor: not-allowed; }
.grid-ctx-item.disabled:hover { background: transparent; color: var(--dc-text-dim, #bbb); }
.grid-ctx-item.danger { color: var(--el-color-danger, #f56c6c); }
.grid-ctx-item.danger:hover { background: var(--el-color-danger-light-9, #fef0f0); color: var(--el-color-danger, #f56c6c); }
.grid-ctx-ic { font-size: 15px; flex-shrink: 0; }
.grid-ctx-label { flex: 1; white-space: nowrap; }
.grid-ctx-sc { font-size: 12px; color: var(--dc-text-dim, #aaa); }
.grid-ctx-caret { font-size: 13px; color: var(--dc-text-dim, #aaa); }
.grid-ctx-sub {
  position: absolute; left: calc(100% - 4px); top: -6px;
  min-width: 160px; background: var(--dc-bg-card, #fff);
  border: 1px solid var(--dc-border, #e4e7ed); border-radius: 10px;
  padding: 6px; box-shadow: 0 10px 30px rgba(0, 0, 0, 0.18);
}
/* 暗色主题 */
.grid-ctx-menu.ctx-dark {
  background: #1f2329; border-color: #3a3f47; color: #e6e8eb;
}
.grid-ctx-menu.ctx-dark .grid-ctx-sep { background: #3a3f47; }
.grid-ctx-menu.ctx-dark .grid-ctx-item { color: #e6e8eb; }
.grid-ctx-menu.ctx-dark .grid-ctx-item:hover { background: #2a4a6b; color: #79bbff; }
.grid-ctx-menu.ctx-dark .grid-ctx-item.disabled { color: #6b7178; }
.grid-ctx-menu.ctx-dark .grid-ctx-sub { background: #1f2329; border-color: #3a3f47; }
</style>
