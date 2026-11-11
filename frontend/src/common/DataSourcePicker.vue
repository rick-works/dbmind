<template>
  <el-dialog :model-value="modelValue" width="760px" append-to-body
             :close-on-click-modal="true" :show-close="true"
             @update:model-value="$emit('update:modelValue', $event)"
             class="dsp-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><CirclePlus /></el-icon></span>
        <span>{{ $t('cd.newTitle') }}</span>
      </div>
    </template>
    <div class="ds-picker">
      <el-input v-model="keyword" clearable
                :prefix-icon="Search" class="ds-search" />

      <div v-for="group in filteredGroups" :key="group.name" class="ds-group">
        <div class="ds-group-title">
          <el-icon :size="15"><component :is="group.icon" /></el-icon>
          {{ group.name }}
          <span class="ds-group-count">{{ group.items.length }}</span>
        </div>
        <div class="ds-grid">
          <div v-for="t in group.items" :key="t.code" class="ds-card" @click="pick(t.code)">
            <DbLogo :type="t.code" :size="42" />
            <div class="ds-card-info">
              <div class="ds-card-name">{{ t.label }}</div>
              <div class="ds-card-meta">
                <template v-if="t.defaultPort">{{ $t('cd.port') }} {{ t.defaultPort }}</template>
                <template v-else>{{ $t('dsp.localFile') }}</template>
              </div>
            </div>
            <div v-if="!t.builtin && !isReady(t.code)" class="ds-card-badge" :title="$t('dsp.driverTip')">{{ $t('dsp.downloadDriver') }}</div>
            <div v-else-if="!t.builtin && isReady(t.code)" class="ds-card-badge ds-card-ready" :title="$t('dsp.driverDownloaded')">{{ $t('settings.driver.tagReady') }}</div>
          </div>
        </div>
      </div>

      <el-empty v-if="!filteredGroups.length" :description="$t('settings.driver.empty')" :image-size="90" />
    </div>
  </el-dialog>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { Search, Coin, Document, Box, CirclePlus, DataAnalysis } from '@element-plus/icons-vue'
import { getDriverTypes, getDriverStatus } from '../api'
import DbLogo from './DbLogo.vue'
import { t } from '../utils/i18n'

/**
 * 「新建数据源」的类型选择器 —— **只管挑类型**。
 *
 * 导入 / 导出 / 批量管理连接原本挤在这块 footer 里（三个按钮 + 一句说明），
 * 既占地方又和「选类型」这件事无关；现已分别收进左侧树右键的
 * 「管理连接 / 导入连接 / 导出连接」（见 ConnManagerDialog / ImportConnsDialog / ExportConnsDialog）。
 */
const props = defineProps({
  modelValue: Boolean
})
const emit = defineEmits(['update:modelValue', 'selected'])

const types = ref([])
const status = ref({})
const keyword = ref('')

onMounted(async () => {
  if (!types.value.length) {
    try {
      types.value = await getDriverTypes()
    } catch (e) { /* 兼容旧后端 */ }
  }
  try {
    status.value = await getDriverStatus()
  } catch (e) { /* 兼容旧后端 */ }
})

const isReady = (code) => status.value[code]?.ready === true

// OLAP（ClickHouse / Doris）单独成组：它们也走 JDBC、也用 SQL，但是列存分析引擎
// （无事务、无外键、无二级索引），跟 MySQL 那类关系型库不是一回事。
// 旧内核不给 category='OLAP'，就按 code 兜底认出来。
const OLAP_CODES = ['CLICKHOUSE', 'DORIS']
const isOlap = (x) => x.category === 'OLAP' || OLAP_CODES.includes(String(x.code || '').toUpperCase())

const groups = computed(() => [
  { name: t('settings.driver.catRelational'), icon: Coin, items: types.value.filter(t => t.category === 'RELATIONAL' && !isOlap(t)) },
  { name: t('settings.driver.catOlap'), icon: DataAnalysis, items: types.value.filter(isOlap) },
  { name: t('settings.driver.catFile'), icon: Document, items: types.value.filter(t => t.category === 'RELATIONAL_FILE') },
  { name: t('settings.driver.catNoSql'), icon: Box, items: types.value.filter(t => t.category === 'NOSQL') }
].filter(g => g.items.length))

const filteredGroups = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  if (!kw) return groups.value
  return groups.value.map(g => ({
    ...g,
    items: g.items.filter(t =>
      t.label.toLowerCase().includes(kw) || t.code.toLowerCase().includes(kw))
  })).filter(g => g.items.length)
})

const pick = (code) => {
  emit('selected', code)
  emit('update:modelValue', false)
}
</script>

<style scoped>
.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}

/* 深色主题覆盖 */
.ds-picker { max-height: 62vh; overflow-y: auto; padding-right: 4px; }
.ds-picker :deep(.el-input__wrapper) {
  background: var(--dc-bg-input); box-shadow: 0 0 0 1px var(--dc-bg-hover) inset;
}
.ds-picker :deep(.el-input__inner) { color: var(--dc-text); }
.ds-picker :deep(.el-empty__description) { color: var(--dc-text-dim); }

.ds-search { margin-bottom: 14px; }

.ds-group { margin-bottom: 18px; }
.ds-group-title {
  display: flex; align-items: center; gap: 6px;
  font-size: 14px; font-weight: 600; color: var(--dc-text-dim);
  margin-bottom: 10px;
}
.ds-group-count {
  font-size: 12px; font-weight: 400; color: var(--dc-text-dim);
  background: var(--dc-bg-hover); border-radius: 8px; padding: 0 7px; line-height: 16px;
}

.ds-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 10px;
}
.ds-card {
  display: flex; align-items: center; gap: 10px;
  padding: 12px; border: 1px solid var(--dc-bg-hover);
  border-radius: 10px; cursor: pointer; position: relative;
  transition: all .18s ease;
  background: var(--dc-bg-soft);
}
.ds-card:hover {
  border-color: var(--el-color-primary); background: var(--dc-bg-hover);
  transform: translateY(-2px); box-shadow: var(--dc-shadow-sm);
}
/* 弹窗内大图标：去掉浅色底板，避免在深色卡片上过于刺眼 */
.ds-card :deep(.db-logo img) {
  background: transparent !important;
  padding: 0;
  box-shadow: none;
}
.ds-card-info { min-width: 0; overflow: hidden; }
.ds-card-name { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.ds-card-meta { font-size: 12px; color: var(--dc-text-dim); margin-top: 2px; }
.ds-card-badge {
  position: absolute; right: 8px; top: 8px;
  font-size: 11px; color: var(--dc-warning); background: var(--dc-warning-wash);
  border: 1px solid var(--dc-warning-wash); border-radius: 6px; padding: 0 5px;
}
.ds-card-ready { color: var(--dc-accent); background: var(--dc-success-wash); border-color: var(--dc-success-wash); }
</style>
