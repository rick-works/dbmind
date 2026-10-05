<template>
  <span class="db-logo" :class="['db-logo-' + size, { 'db-logo-dim': !props.connected }]">
    <img v-if="logoUrl" class="db-logo-img" :src="logoUrl" :width="size" :height="size" :alt="logoAlt" />
    <svg v-else-if="fallbackLogo === 'elasticsearch'" viewBox="0 0 32 32" :width="size" :height="size">
      <rect width="32" height="32" rx="5" fill="#005571"/>
      <ellipse cx="16" cy="16" rx="10" ry="6" fill="none" stroke="#fff" stroke-width="1.5"/>
      <ellipse cx="16" cy="16" rx="6" ry="10" fill="none" stroke="#fff" stroke-width="1.5"/>
      <circle cx="16" cy="16" r="2" fill="#fff"/>
    </svg>
    <svg v-else-if="fallbackLogo === 'h2'" viewBox="0 0 32 32" :width="size" :height="size">
      <rect width="32" height="32" rx="5" fill="#094a87"/>
      <text x="16" y="21" text-anchor="middle" fill="#fff" font-size="14" font-weight="bold" font-family="Arial">H2</text>
    </svg>
    <svg v-else viewBox="0 0 32 32" :width="size" :height="size">
      <rect width="32" height="32" rx="5" fill="#6b7280"/>
      <text x="16" y="21" text-anchor="middle" fill="#fff" font-size="11" font-weight="bold" font-family="Arial">DB</text>
    </svg>
  </span>
</template>

<script setup>
import { computed } from 'vue'
import { byType, labelOf } from '../types'

// 自动收集 assets 下的类型 logo（type-*.svg）。新增类型：放入 svg 并在其类型包声明 logo 文件名即可。
const logoAssets = import.meta.glob('../assets/*.svg', { eager: true, import: 'default' })
const assetsByFile = new Map(
  Object.entries(logoAssets).map(([path, url]) => [path.slice(path.lastIndexOf('/') + 1), url])
)

const props = defineProps({
  type: { type: String, default: '' },
  size: { type: Number, default: 18 },
  connected: { type: Boolean, default: true }
})

const logoAlt = computed(() => labelOf(props.type))
const fallbackLogo = computed(() => byType(props.type).fallbackLogo)
const logoUrl = computed(() => {
  const name = byType(props.type).logo
  return (name && assetsByFile.has(name)) ? assetsByFile.get(name) : ''
})
</script>

<style scoped>
.db-logo {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  user-select: none;
}
.db-logo svg,
.db-logo img {
  display: block;
}
.db-logo img {
  object-fit: contain;
  filter: brightness(1.12) saturate(1.15);
}
.db-logo-dim svg,
.db-logo-dim img {
  filter: grayscale(100%) brightness(1.05);
  opacity: 0.35;
}
</style>
