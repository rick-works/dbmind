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
    <svg v-else-if="fallbackLogo === 'duckdb'" viewBox="0 0 32 32" :width="size" :height="size">
      <!-- DuckDB 品牌黄 + 黑色鸭形剪影（圆头 + 尖尾的极简折线） -->
      <rect width="32" height="32" rx="5" fill="#fff45b"/>
      <path d="M8 22c0-6 3-10 7-10s6 3 6 6l3 1-3 2c-1 3-3 5-7 5h-6z" fill="#1a1a1a"/>
      <circle cx="17.5" cy="15.5" r="1.2" fill="#fff45b"/>
    </svg>
    <svg v-else-if="fallbackLogo === 'starrocks'" viewBox="0 0 32 32" :width="size" :height="size">
      <!-- StarRocks 品牌深蓝 + 白色星形 -->
      <rect width="32" height="32" rx="5" fill="#0b2b5b"/>
      <path d="M16 7l2.3 5.5 5.7.5-4.3 3.9 1.3 5.6L16 19.6l-5 2.9 1.3-5.6L8 13l5.7-.5L16 7z" fill="#fff"/>
    </svg>
    <svg v-else-if="fallbackLogo === 'oceanbase'" viewBox="0 0 32 32" :width="size" :height="size">
      <!-- OceanBase 品牌蓝 + 白色浪线 -->
      <rect width="32" height="32" rx="5" fill="#0f6cbd"/>
      <path d="M6 20c3-6 6-6 9 0s6 6 9 0" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round"/>
    </svg>
    <svg v-else-if="fallbackLogo === 'opengauss'" viewBox="0 0 32 32" :width="size" :height="size">
      <!-- openGauss 品牌红橙渐变 + 白色圆点环 -->
      <rect width="32" height="32" rx="5" fill="#c4262e"/>
      <circle cx="16" cy="16" r="7" fill="none" stroke="#fff" stroke-width="2"/>
      <circle cx="16" cy="9" r="2" fill="#fff"/>
    </svg>
    <!-- 回退分支：有官方 logo（types/*.js 的 logo 字段）时走不到这里；
         SelectDB 目前没有官方矢量素材，用「缩写 + 品牌色」的小方块顶替 -->
    <svg v-else-if="fallbackLogo === 'greatsql'" viewBox="0 0 32 32" :width="size" :height="size">
      <rect width="32" height="32" rx="5" fill="#1f7a4d"/>
      <text x="16" y="21" text-anchor="middle" fill="#fff" font-size="12" font-weight="bold" font-family="Arial">GR</text>
    </svg>
    <svg v-else-if="fallbackLogo === 'goldendb'" viewBox="0 0 32 32" :width="size" :height="size">
      <rect width="32" height="32" rx="5" fill="#1d4ed8"/>
      <text x="16" y="21" text-anchor="middle" fill="#fff" font-size="10" font-weight="bold" font-family="Arial">GDB</text>
    </svg>
    <svg v-else-if="fallbackLogo === 'selectdb'" viewBox="0 0 32 32" :width="size" :height="size">
      <rect width="32" height="32" rx="5" fill="#6d28d9"/>
      <text x="16" y="21" text-anchor="middle" fill="#fff" font-size="10" font-weight="bold" font-family="Arial">SDB</text>
    </svg>
    <svg v-else-if="fallbackLogo === 'highgo'" viewBox="0 0 32 32" :width="size" :height="size">
      <rect width="32" height="32" rx="5" fill="#0f766e"/>
      <text x="16" y="21" text-anchor="middle" fill="#fff" font-size="11" font-weight="bold" font-family="Arial">HG</text>
    </svg>
    <svg v-else-if="fallbackLogo === 'vastbase'" viewBox="0 0 32 32" :width="size" :height="size">
      <rect width="32" height="32" rx="5" fill="#0369a1"/>
      <text x="16" y="21" text-anchor="middle" fill="#fff" font-size="12" font-weight="bold" font-family="Arial">VB</text>
    </svg>
    <svg v-else-if="fallbackLogo === 'uxdb'" viewBox="0 0 32 32" :width="size" :height="size">
      <rect width="32" height="32" rx="5" fill="#c2410c"/>
      <text x="16" y="21" text-anchor="middle" fill="#fff" font-size="12" font-weight="bold" font-family="Arial">UX</text>
    </svg>
    <svg v-else-if="fallbackLogo === 'gaussdb'" viewBox="0 0 32 32" :width="size" :height="size">
      <rect width="32" height="32" rx="5" fill="#b91c1c"/>
      <text x="16" y="21" text-anchor="middle" fill="#fff" font-size="11" font-weight="bold" font-family="Arial">GS</text>
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
