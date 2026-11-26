<template>
  <el-dialog :model-value="modelValue" width="min(1040px, 94vw)" append-to-body
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

      <!-- 分类用 tab 栏切换（21 种类型 5 个分组，堆成多段太长） -->
      <div class="ds-tabs" role="tablist">
        <button v-for="tb in tabs" :key="tb.key" type="button" role="tab"
                class="ds-tab" :class="{ 'is-active': activeTab === tb.key }"
                :aria-selected="activeTab === tb.key" @click="activeTab = tb.key">
          <el-icon v-if="tb.icon" :size="15"><component :is="tb.icon" /></el-icon>
          <span class="ds-tab-name">{{ tb.name }}</span>
          <span class="ds-tab-count">{{ tb.count }}</span>
        </button>
      </div>

      <div class="ds-grid">
        <div v-for="t in visibleItems" :key="t.code" class="ds-card" @click="pick(t.code)">
          <DbLogo :type="t.code" :size="46" />
          <div class="ds-card-info">
            <div class="ds-card-name">
              {{ t.label }}
              <!-- 支持等级：只给「需要提醒」的两档加标（full / operate 是常态，标了反而噪音） -->
              <span v-if="showLevel(t)" class="ds-lvl" :class="'is-' + levelOf(t)" :title="levelTip(t)">
                {{ levelLabel(t) }}
              </span>
            </div>
            <div class="ds-card-meta">
              <template v-if="t.defaultPort">{{ $t('cd.port') }} {{ t.defaultPort }}</template>
              <template v-else>{{ $t('dsp.localFile') }}</template>
            </div>
          </div>
          <div v-if="!t.builtin && !isReady(t.code)" class="ds-card-badge" :title="$t('dsp.driverTip')">{{ $t('dsp.downloadDriver') }}</div>
          <div v-else-if="!t.builtin && isReady(t.code)" class="ds-card-badge ds-card-ready" :title="$t('dsp.driverDownloaded')">{{ $t('settings.driver.tagReady') }}</div>
        </div>
        <!-- 待验证数据源：后端 exposed:false 的类型不会出现在清单里，这里给出可见入口，
             让用户知道「不是产品不支持，而是还没拿到可真机验证的介质」。 -->
        <div v-if="!keyword" class="ds-card ds-card-more" @click="moreVisible = true">
          <div class="ds-more-ic"><el-icon :size="22"><QuestionFilled /></el-icon></div>
          <div class="ds-card-info">
            <div class="ds-card-name">{{ $t('dsp.moreTypes') }}</div>
            <div class="ds-card-meta">{{ $t('dsp.moreTypesTip') }}</div>
          </div>
        </div>
      </div>

      <el-empty v-if="!filteredGroups.length" :description="$t('settings.driver.empty')" :image-size="90" />
    </div>

    <el-dialog v-model="moreVisible" :title="$t('dsp.moreTypes')" width="min(560px, 92vw)"
               append-to-body class="dsp-more-dialog">
      <p class="dsp-more-desc">{{ $t('dsp.moreTypesDesc') }}</p>
      <div class="dsp-more-list">
        <div v-for="code in PENDING_TYPES" :key="code" class="dsp-more-item">
          <DbLogo :type="code" :size="28" />
          <span class="dsp-more-name">{{ code }}</span>
          <el-tag size="small" type="info" effect="plain">{{ $t('dsp.unverified') }}</el-tag>
        </div>
      </div>
      <template #footer>
        <el-button size="small" @click="moreVisible = false">{{ $t('common.close') }}</el-button>
      </template>
    </el-dialog>
  </el-dialog>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { Search, Coin, Document, Box, CirclePlus, DataAnalysis, QuestionFilled } from '@element-plus/icons-vue'
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

// 后端 /api/drivers/types 按 YAML exposed:false 过滤掉的类型：不是不支持，是还没拿到
// 可真机验证的介质（商业授权 / 企业试用 / 绑硬件 License）。在选类型页给一个明确入口。
const PENDING_TYPES = ['GAUSSDB', 'GOLDENDB', 'SELECTDB', 'UXDB']
const moreVisible = ref(false)

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

/**
 * 支持等级（后端 `/api/drivers/types` 的 `supportLevel`，唯一真源是 YAML）。
 *
 * 全功能 / 可运维是常态，不打扰；只读浏览 / 实验必须让用户在**选之前**就知道，
 * 否则装完发现「不能改结构」「某个功能没验证过」就成了欺骗。
 * 未知取值按 operate 处理（老内核不返回该字段）。
 */
const levelOf = (x) => String(x?.supportLevel || 'operate')
const showLevel = (x) => ['browse', 'experimental'].includes(levelOf(x))
const levelLabel = (x) => t(levelOf(x) === 'browse' ? 'dsp.lvlBrowse' : 'dsp.lvlExperimental')
const levelTip = (x) => t(levelOf(x) === 'browse' ? 'dsp.lvlBrowseTip' : 'dsp.lvlExpTip')

// OLAP（ClickHouse / Doris / SAP HANA）单独成组：它们也走 JDBC、也用 SQL，但是列存分析引擎
// （无事务、无外键、无二级索引），跟 MySQL 那类关系型库不是一回事。
// 旧内核不给 category='OLAP'，就按 code 兜底认出来。
const OLAP_CODES = ['CLICKHOUSE', 'DORIS', 'SAPHANA']
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

// 分类 tab：全部 + 各分组。分组名即 key（同一次渲染内稳定，搜索改的是数量不是名字）
const activeTab = ref('all')
const tabs = computed(() => {
  const list = filteredGroups.value
  const total = list.reduce((n, g) => n + g.items.length, 0)
  return [
    { key: 'all', name: t('settings.driver.catAll'), icon: null, count: total },
    ...list.map(g => ({ key: g.name, name: g.name, icon: g.icon, count: g.items.length }))
  ]
})

// 搜索把当前 tab 的组滤没了就自动回「全部」，不然会停在空白页
watch([tabs, keyword], () => {
  if (activeTab.value !== 'all' && !tabs.value.some(tb => tb.key === activeTab.value)) {
    activeTab.value = 'all'
  }
})

const visibleItems = computed(() => {
  if (activeTab.value === 'all') return filteredGroups.value.flatMap(g => g.items)
  const g = filteredGroups.value.find(x => x.name === activeTab.value)
  return g ? g.items : filteredGroups.value.flatMap(x => x.items)
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
/* 21 种类型分 5 组，弹窗加宽后固定 4 列；高度放到 74vh，减少滚动 */
.ds-picker { max-height: 74vh; overflow-y: auto; padding-right: 4px; }
.ds-picker :deep(.el-input__wrapper) {
  background: var(--dc-bg-input); box-shadow: 0 0 0 1px var(--dc-bg-hover) inset;
}
.ds-picker :deep(.el-input__inner) { color: var(--dc-text); }
.ds-picker :deep(.el-empty__description) { color: var(--dc-text-dim); }

.ds-search { margin-bottom: 14px; }

/* 分类 tab 栏：胶囊式分段控件，激活态用主色底 */
.ds-tabs {
  display: flex; flex-wrap: wrap; gap: 8px;
  margin-bottom: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--dc-bg-hover);
}
.ds-tab {
  display: inline-flex; align-items: center; gap: 6px;
  padding: 7px 14px; border-radius: 999px;
  border: 1px solid var(--dc-bg-hover);
  background: var(--dc-bg-soft);
  color: var(--dc-text-dim);
  font: inherit; font-size: 13.5px; font-weight: 600;
  cursor: pointer; transition: all .16s ease;
}
.ds-tab:hover { color: var(--dc-text-strong); border-color: var(--el-color-primary-light-5); }
.ds-tab.is-active {
  color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  border-color: transparent;
  box-shadow: 0 2px 10px var(--dc-primary-glow);
}
.ds-tab-name { white-space: nowrap; }
.ds-tab-count {
  font-size: 12px; font-weight: 400;
  background: rgba(255, 255, 255, .16);
  border-radius: 8px; padding: 0 6px; line-height: 17px;
}
.ds-tab:not(.is-active) .ds-tab-count {
  background: var(--dc-bg-hover); color: var(--dc-text-dim);
}

.ds-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
  gap: 12px;
}
.ds-card {
  display: flex; align-items: center; gap: 12px;
  padding: 14px 16px; border: 1px solid var(--dc-bg-hover);
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
.ds-card-name { font-size: 15px; font-weight: 600; color: var(--dc-text-strong); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.ds-card-meta { font-size: 12.5px; color: var(--dc-text-dim); margin-top: 3px; }
/* 支持等级小标：只读浏览（中性）/ 实验（警示色） */
.ds-lvl {
  display: inline-block; margin-left: 6px; vertical-align: 1px;
  font-size: 11px; font-weight: 600; line-height: 16px; padding: 0 5px;
  border-radius: 5px; cursor: help;
}
.ds-lvl.is-browse { color: var(--dc-text-dim); background: var(--dc-bg-hover); }
.ds-lvl.is-experimental { color: var(--dc-warning); background: var(--dc-warning-wash); }
.ds-card-badge {
  position: absolute; right: 8px; top: 8px;
  font-size: 11px; color: var(--dc-warning); background: var(--dc-warning-wash);
  border: 1px solid var(--dc-warning-wash); border-radius: 6px; padding: 0 5px;
}
.ds-card-ready { color: var(--dc-accent); background: var(--dc-success-wash); border-color: var(--dc-success-wash); }
/* 待验证入口：虚线卡片，和正式类型卡一眼能分开 */
.ds-card-more { border-style: dashed; background: transparent; }
.ds-card-more:hover { border-color: var(--dc-text-dim); background: var(--dc-bg-soft); }
.ds-more-ic {
  width: 46px; height: 46px; flex: none; display: inline-flex;
  align-items: center; justify-content: center; border-radius: 12px;
  color: var(--dc-text-dim); background: var(--dc-bg-hover);
}
.dsp-more-dialog .dsp-more-desc { margin: 0 0 14px; font-size: 13px; color: var(--dc-text-dim); line-height: 1.7; }
.dsp-more-dialog .dsp-more-list { display: grid; gap: 8px; }
.dsp-more-dialog .dsp-more-item {
  display: flex; align-items: center; gap: 10px; padding: 9px 12px;
  border: 1px solid var(--dc-bg-hover); border-radius: 8px; background: var(--dc-bg-soft);
}
.dsp-more-dialog .dsp-more-name { flex: 1; font-size: 13px; font-weight: 600; color: var(--dc-text); }
</style>
