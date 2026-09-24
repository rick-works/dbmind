<template>
  <el-dialog
    :model-value="modelValue"
    @update:model-value="$emit('update:modelValue', $event)"
    width="920px"
    top="6vh"
    append-to-body
    class="gov-dialog"
  >
    <template #header>
      <div class="gv-title">
        <span class="gv-title-ic"><el-icon :size="15"><DataAnalysis /></el-icon></span>
        <span class="gv-title-text">{{ $t('gv.title') }}</span>
      </div>
    </template>

    <div class="gv-tabs">
      <button v-for="t in tabs" :key="t.key" class="gv-tab" :class="{ active: tab === t.key }" @click="switchTab(t.key)">
        {{ t.label }}
      </button>
    </div>

    <!-- 上下文选择：数据源 / 库 / 模式 -->
    <div class="gv-bar">
      <span class="gv-bar-label">{{ $t('gv.scope') }}</span>
      <el-select v-model="connId" size="small" class="gv-conn" :placeholder="$t('ai.pickSource')" filterable>
        <!-- 按「目录」分组，多级显示（与左侧对象树口径一致） -->
        <el-option-group v-for="g in connGroups" :key="g.label" :label="g.label">
          <el-option v-for="c in g.options" :key="c.id" :label="connLabel(c)" :value="c.id" />
        </el-option-group>
      </el-select>
      <el-select
        v-model="database"
        size="small"
        class="gv-db"
        :placeholder="$t('ai.pickDb')"
        filterable
        :loading="loadingDbs"
        :disabled="!activeConn"
      >
        <el-option v-for="d in dbs" :key="d" :label="d" :value="d" />
      </el-select>
      <el-select
        v-if="needSchema"
        v-model="schema"
        size="small"
        class="gv-schema"
        :placeholder="$t('ai.pickSchema')"
        filterable
        :loading="loadingSchemas"
        :disabled="!database"
      >
        <el-option v-for="s in schemas" :key="s" :label="s" :value="s" />
      </el-select>
      <!-- 操作按钮统一放在弹窗底部 -->
    </div>

    <!-- 类型/连接状态提示（独立一行，避免挤压工具栏） -->
    <div v-if="isNoSqlConn" class="gv-warn">
      <el-icon class="gv-warn-ic"><Warning /></el-icon>
      {{ $t('gv.nosqlTip', { type: activeConn?.type }) }}
    </div>
    <div v-else-if="dbError" class="gv-err" :title="dbError">{{ truncateError(dbError) }}</div>
    <!-- 表清单加载失败：明确告知原因，而不是让下拉静默显示「无数据」 -->
    <div v-if="tab === 'quality' && tableError" class="gv-err" :title="tableError">
      {{ $t('gv.tableListFailed', { detail: truncateError(tableError) }) }}
    </div>
    <!-- 表清单为空：接口成功但无表，给出排查方向（库确实无表 / 查询被服务端中断 / 权限不足） -->
    <div
      v-else-if="tab === 'quality' && !loadingTables && activeConn && database && !tables.length"
      class="gv-warn"
    >
      {{ $t('gv.noTables') }}
    </div>

    <div class="gv-body">
      <div v-if="loading" class="gv-loading">{{ $t('gv.analyzing') }}</div>

      <!-- 敏感数据 -->
      <template v-else-if="tab === 'sensitive' && sensitive">
        <div class="gv-stats">
          <div class="gv-stat lv-high"><span class="n">{{ sensitive.high || 0 }}</span><span class="l">{{ $t('gv.highSensitive') }}</span></div>
          <div class="gv-stat lv-medium"><span class="n">{{ sensitive.medium || 0 }}</span><span class="l">{{ $t('gv.mediumSensitive') }}</span></div>
          <div class="gv-stat"><span class="n">{{ sensitive.scanned || 0 }}</span><span class="l">{{ $t('gv.statScanned') }}</span></div>
        </div>
        <div v-if="!sensitive.findings?.length" class="gv-empty">{{ $t('gv.noSensitive') }}</div>
        <div v-for="(f, i) in sensitive.findings" :key="i" class="gv-row" :class="'lv-' + f.level">
          <span class="gv-level">{{ levelText(f.level) }}</span>
          <div class="gv-main">
            <div class="gv-target">{{ f.table }}<span class="gv-col"> · {{ f.column }}</span>
              <span class="gv-tag">{{ f.sensitiveLabel }}</span>
              <span class="gv-src">{{ $t('gv.basis') }}{{ f.evidence }}</span>
            </div>
            <div class="gv-msg">{{ $t('gv.suggestion') }}{{ f.maskHint }}</div>
          </div>
        </div>
        <div v-if="sensitive.recommendations?.length" class="gv-notes">
          <div v-for="(r, i) in sensitive.recommendations" :key="i">· {{ r }}</div>
        </div>
      </template>

      <!-- 质量规则：配置字段级业务规则 -->
      <QualityRulePanel
        v-else-if="tab === 'quality'"
        ref="ruleRef"
        :conn="activeConn"
        :database="effectiveDatabase"
        :tables="tables"
        :loading-tables="loadingTables"
        :disabled="!activeConn || !effectiveDatabase || isNoSqlConn"
        :is-no-sql="isNoSqlConn"
        @state="ruleState = $event"
        @goto-analysis="switchTab('analysis')"
      />

      <!-- 质量分析：按已保存的规则扫描并导出报告 -->
      <QualityAnalysisPanel
        v-else-if="tab === 'analysis'"
        ref="analysisRef"
        :conn="activeConn"
        :database="effectiveDatabase"
        :disabled="!activeConn || !effectiveDatabase || isNoSqlConn"
        :is-no-sql="isNoSqlConn"
        @state="analysisState = $event"
        @goto-rules="switchTab('quality')"
      />

      <!-- 数据容量 -->
      <template v-else-if="tab === 'capacity' && capacity">
        <div class="gv-stats">
          <div class="gv-stat"><span class="n">{{ capacity.scanned || 0 }}</span><span class="l">{{ $t('gv.statTables') }}</span></div>
          <div class="gv-stat"><span class="n">{{ formatNum(capacity.totalRows) }}</span><span class="l">{{ $t('gv.statRows') }}</span></div>
          <div v-if="capacity.sizeSupported" class="gv-stat">
            <span class="n">{{ capacity.totalSizeText || '—' }}</span><span class="l">{{ $t('gv.statSize') }}</span>
          </div>
        </div>

        <div v-if="!capacity.tables?.length" class="gv-empty">{{ $t('gv.noTablesToCount') }}</div>

        <!-- 容量排行：卡片外壳与「敏感数据」一致，内部按「左信息 / 右容量」分栏 + 占比条 -->
        <template v-else>
          <div v-for="(t, i) in capacity.tables" :key="t.table" class="gv-row">
            <span class="gv-level">{{ i + 1 }}</span>
            <div class="gv-main">
              <div class="gv-cap-head">
                <span class="gv-cap-name" :title="t.table">{{ t.table }}</span>
                <span class="gv-cap-size" :class="{ dim: !t.sizeText || t.sizeText === '—' }">
                  {{ t.sizeText || '—' }}
                </span>
              </div>
              <div class="gv-cap-meta">
                <span v-if="t.comment" class="gv-cap-comment" :title="t.comment">{{ t.comment }}</span>
                <span class="gv-cap-rows">{{ $t('gv.rows', { n: formatNum(t.rows) }) }}</span>
                <span
                  v-if="capacity.sizeSupported"
                  class="gv-cap-pct"
                  :title="$t('gv.sizeTip')"
                >{{ barPct(t) }}%</span>
              </div>
              <div class="gv-rank-bar"><i :style="{ width: barWidth(t) }"></i></div>
            </div>
          </div>
        </template>

        <div v-if="capacity.note" class="gv-notes"><div>· {{ capacity.note }}</div></div>
      </template>

      <div v-else class="gv-empty">{{ activeConn ? $t('gv.pickFirst') : $t('pt.pickSourceFirst') }}</div>
    </div>

    <!-- 底部操作栏：随页签变化；关闭用右上角 × 即可 -->
    <template #footer>
      <div class="gv-footer">
        <!-- 质量规则 -->
        <template v-if="tab === 'quality'">
          <span v-if="ruleState.lastSaved" class="gv-footer-tip">
            <el-icon><CircleCheck /></el-icon>{{ $t('gv.rulesSaved', { n: ruleState.savedCount }) }}
          </span>
          <el-button
            type="primary"
            :loading="ruleState.saving"
            :disabled="!ruleState.canSave"
            @click="ruleRef?.save?.()"
          >{{ $t('gv.saveRules') }}</el-button>
        </template>

        <!-- 质量分析 -->
        <template v-else-if="tab === 'analysis'">
          <el-button :disabled="!analysisState.canExport" @click="analysisRef?.exportReport?.()">{{ $t('gv.viewReport') }}</el-button>
          <el-button
            :loading="analysisState.exportingViolations"
            :disabled="!analysisState.canExport"
            @click="analysisRef?.exportViolations?.()"
          >{{ $t('gv.viewErrors') }}</el-button>
          <!-- 分析中把主按钮换成「取消分析」，避免长分析时用户找不到取消入口 -->
          <el-button
            v-if="analysisState.scanning"
            type="danger"
            plain
            @click="analysisRef?.cancel?.()"
          >{{ $t('gv.cancelAnalysis') }}</el-button>
          <el-button
            v-else
            type="primary"
            :disabled="!analysisState.canScan"
            @click="analysisRef?.scan?.()"
          >{{ $t('gv.startAnalysis') }}</el-button>
        </template>

        <!-- 其他页签：按页签定制文案 -->
        <template v-else>
          <el-button
            type="primary"
            :loading="loading"
            :disabled="!activeConn || !database || isNoSqlConn"
            @click="run"
          >{{ runLabel }}</el-button>
        </template>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed, nextTick, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { DataAnalysis, Warning, CircleCheck } from '@element-plus/icons-vue'
import {
  aiScanSensitive, aiCapacity
} from '../../api'
import { useAiContext, truncateError } from './useAiContext'
import { t, te, locale } from '../../utils/i18n'
import QualityRulePanel from './QualityRulePanel.vue'
import QualityAnalysisPanel from './QualityAnalysisPanel.vue'

const props = defineProps({
  modelValue: Boolean,
  conn: Object,
  database: String,
  /** 打开时直接定位到某个页签：sensitive / capacity / quality / analysis（AI 助手的面板命令用） */
  initialTab: String,
  /** 打开后直接开跑（敏感数据 / 数据容量属于「点开即出结果」的扫描，不必再点一次按钮） */
  autoRun: Boolean
})
defineEmits(['update:modelValue'])

// 连接 + 数据库 + 模式三级选择（未在对象树选中连接时，本面板仍可用）
const {
  connId, dbs, database, schemas, schema, connGroups,
  tables, loadingTables, tableError,
  activeConn, needSchema, isNoSqlConn, effectiveDatabase,
  loadingDbs, loadingSchemas, dbError, connLabel, loadTables
} = useAiContext(props)

// ⚠️ 必须是 computed：静态数组只在模块加载时求值一次，之后切语言不会跟着变
const tabs = computed(() => [
  { key: 'sensitive', label: t('ai.cmdSensitive') },
  { key: 'capacity', label: t('ai.cmdCapacity') },
  { key: 'quality', label: t('qa.rulesTitle') },
  { key: 'analysis', label: t('qa.title') }
])

const tab = ref('sensitive')
const loading = ref(false)

// 打开时按 initialTab 定位页签（AI 助手的「敏感数据 / 数据容量 / 质量规则 / 质量分析」命令直达）；
// autoRun 时再直接开跑，省掉「打开 → 再点一次开始扫描」两步
watch(() => props.modelValue, async (open) => {
  if (!open) return
  const k = props.initialTab
  if (k && tabs.value.some(t => t.key === k)) tab.value = k
  if (!props.autoRun || (tab.value !== 'sensitive' && tab.value !== 'capacity')) return
  // 数据库列表是异步加载的，没就绪就点「开始扫描」只会弹「请选择数据源与数据库」，这里最多等 3 秒
  const t0 = Date.now()
  while (!activeConn.value || !database.value) {
    if (Date.now() - t0 > 3000) return
    await new Promise((r) => setTimeout(r, 120))
  }
  await nextTick()
  run()
})

const sensitive = ref(null)
/** 质量规则 / 质量分析子组件引用：底部操作栏通过它们触发子组件方法 */
const ruleRef = ref(null)
const analysisRef = ref(null)
/** 子组件上报的状态（决定底部按钮可用性与 loading） */
const ruleState = ref({})
const analysisState = ref({})
const capacity = ref(null)

// 级别名按代码查字典（high/medium/low/info 来自后端），没收录的原样显示
const LEVEL_KEYS = { high: 'ai.lv.high', medium: 'ai.lv.medium', low: 'ai.lv.low', info: 'ai.lv.info' }
const levelText = (lv) => (lv && te(LEVEL_KEYS[lv])) ? t(LEVEL_KEYS[lv]) : (lv || '')

/** 底部主按钮文案：按页签语义定制，比统一的「开始分析」更贴切 */
const runLabel = computed(() => ({
  sensitive: t('gv.runScan'),
  capacity: t('gv.runCapacity')
}[tab.value] || t('gv.startAnalysis')))

const formatNum = (n) => {
  const v = Number(n || 0)
// 中文按「亿 / 万」进位、英文按「B / M」进位：同一个数在两种语言下的可读写法不同
  if (v >= 100000000) return locale.value === 'en-US' ? (v / 1000000000).toFixed(2) + t('gv.unitB') : (v / 100000000).toFixed(2) + t('gv.unitYi')
  if (v >= 10000) return locale.value === 'en-US' ? (v / 1000000).toFixed(1) + t('gv.unitM') : (v / 10000).toFixed(1) + t('gv.unitWan')
  return String(v)
}

/**
 * 容量占比：该表占**整库总容量**的比例（0-100 整数），全部表合计约 100%。
 * 用「占总容量」而不是「占榜首」，否则第一名恒为 100% 会产生误导。
 * 无存储口径或该表大小未知时返回 0。
 */
const barPct = (t) => {
  if (!capacity.value?.sizeSupported) return 0
  const total = Math.max(0, Number(capacity.value?.totalSize) || 0)
  const own = Math.max(0, Number(t?.sizeBytes) || 0)
  if (total <= 0 || own <= 0) return 0
  return Math.round((own / total) * 100)
}

/** 容量条宽度：占比 + 最小 2%，保证占比很小但确有数据的表也有一小段可见 */
const barWidth = (t) => {
  const p = barPct(t)
  return p <= 0 ? '0%' : Math.max(2, p) + '%'
}

const copy = async (t) => {
  try { await navigator.clipboard.writeText(t); ElMessage.success(t('ai.copied')) } catch (e) { ElMessage.error(t('sqlq.copyFailed')) }
}


const switchTab = async (k) => {
  tab.value = k
  // 「质量规则」需要选表：切到该页签时才拉取表清单（按需，避免无谓请求）
  if (k === 'quality') loadTables()
  // 「质量分析」每次进入刷新「已配置规则的表」，确保刚保存的规则立即可分析。
  // 需等子组件挂载完成后再调用，否则 ref 仍为 null，刷新会被静默丢弃
  if (k === 'analysis') {
    await nextTick()
    analysisRef.value?.refresh?.()
  }
}

const run = async () => {
  if (!activeConn.value || !database.value) { ElMessage.warning(t('pt.needSel')); return }
  loading.value = true
  const base = { connectionId: activeConn.value.id, database: effectiveDatabase.value }
  try {
    if (tab.value === 'sensitive') {
      // 上限给足，确保整库所有表都被扫描（后端另有 500 的硬上限保护）
      const r = await aiScanSensitive({ ...base, maxTables: 500 })
      sensitive.value = r && r.success ? r : null
      if (!r?.success) ElMessage.error(r?.message || t('ai.analyzeFailed'))
    } else if (tab.value === 'capacity') {
      // 上限给足，确保整库所有表都被统计（后端另有 500 的硬上限保护）
      const r = await aiCapacity({ ...base, maxTables: 500 })
      capacity.value = r && r.success ? r : null
      if (!r?.success) ElMessage.error(r?.message || t('ai.analyzeFailed'))
    }
  } catch (e) {
    ElMessage.error(e?.message || e?.toString?.() || t('ai.analyzeFailed'))
  }
  loading.value = false
}

</script>

<style scoped>
.gv-title { display: flex; align-items: center; gap: 9px; }
.gv-title-ic {
  width: 25px; height: 25px; border-radius: 8px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-purple));
  flex-shrink: 0;
}
.gv-title-text { font-size: 14.5px; font-weight: 600; color: var(--dc-text-strong); }

.gv-tabs {
  display: flex; gap: 5px; flex-wrap: wrap; margin-bottom: 14px;
  padding-bottom: 12px; border-bottom: 1px solid var(--dc-border);
}
.gv-tab {
  border: 1px solid var(--dc-border); background: var(--dc-bg-card); color: var(--dc-text-dim);
  font-size: 13px; font-family: inherit; padding: 5px 13px; border-radius: 999px; cursor: pointer;
  transition: all .16s ease; line-height: 1.4;
}
.gv-tab:hover { border-color: var(--dc-primary); color: var(--dc-text); }
.gv-tab.active {
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-purple));
  border-color: transparent; color: var(--dc-on-primary); font-weight: 600;
  box-shadow: 0 2px 8px var(--dc-primary-glow, rgba(64, 158, 255, .3));
}

.gv-bar {
  display: flex; align-items: center; gap: 8px; margin-bottom: 10px; flex-wrap: wrap;
  padding-bottom: 10px; border-bottom: 1px solid var(--dc-border);
}
.gv-bar-label {
  font-size: 13px; color: var(--dc-text-dim); flex-shrink: 0;
  padding: 2px 8px; border-radius: 5px; background: var(--dc-bg-soft);
}
.gv-spacer { flex: 1; min-width: 8px; }
.gv-conn { width: 220px; }
.gv-db { width: 170px; }
.gv-schema { width: 128px; }
.gv-ddl { flex: 1; min-width: 200px; }

.gv-err {
  font-size: 13px; color: var(--dc-danger); margin-bottom: 10px;
  background: rgba(245, 108, 108, .08); border-radius: 6px; padding: 7px 11px;
  /* 最长两行，避免超长堆栈把面板撑爆；完整内容用 title 悬浮查看 */
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical;
  overflow: hidden; word-break: break-all; cursor: help; line-height: 1.65;
}
.gv-warn {
  font-size: 13px; color: var(--dc-warning, #e6a23c); margin-bottom: 10px;
  background: rgba(230, 162, 60, .10); border-radius: 6px; padding: 7px 11px; line-height: 1.65;
}
.gv-warn-ic { margin-right: 4px; vertical-align: -2px; }
.gv-body {
  /* 弹窗高度已由全局样式统一固定（.el-dialog.gov-dialog = 76vh），
     这里只需撑满内容区剩余高度并在内部滚动，不再自设 max-height */
  flex: 1; min-height: 0; overflow: auto; padding-right: 2px;
  /* 纵向弹性布局：空状态/加载态可撑满内容区，避免下方留下大片空白显得不协调 */
  display: flex; flex-direction: column;
}
.gv-loading {
  flex: 1; display: flex; align-items: center; justify-content: center;
  text-align: center; color: var(--dc-text-dim); font-size: 13.5px;
}
.gv-empty {
  flex: 1; padding: 34px 20px; text-align: center; color: var(--dc-text-dim); font-size: 14px;
  background: var(--dc-bg-soft); border: 1px dashed var(--dc-border); border-radius: 10px;
  line-height: 1.9;
  /* 内容在块内水平垂直居中（横向排列，含 <b> 的文案也能保持同一行） */
  display: flex; align-items: center; justify-content: center; flex-wrap: wrap;
}
.gv-empty b { color: var(--dc-primary); font-weight: 600; }

/* 子面板（质量规则 / 质量分析）随内容区一起撑满，空状态居中，
   与其它页签的空状态观感保持一致（不再贴顶、下方留大片空白） */
/* 子面板（质量规则 / 质量分析）撑满内容区。
   min-height: 0 是关键：flex item 默认 min-height:auto，不会小于内容高度，
   否则内部表格会把整块撑高，导致「外层滚动而不是表格内部滚动」。 */
.gv-body > :deep(.qa),
.gv-body > :deep(.qr) {
  flex: 1; min-height: 0; overflow: hidden;
  display: flex; flex-direction: column;
}
.gv-body :deep(.qa-empty),
.gv-body :deep(.qr-empty) {
  /* 横向排列：文字与内嵌 <b> 标签保持在同一行，避免列布局把它们折成多行；
     窄屏空间不足时才自动换行，防止溢出 */
  flex: 1; display: flex; align-items: center; justify-content: center;
  flex-wrap: wrap; text-align: center;
}

/* 底部操作栏 */
.gv-footer { display: flex; align-items: center; justify-content: flex-end; gap: 8px; }
.gv-footer-tip { font-size: 13px; color: #67c23a; margin-right: auto; }
.gv-footer-tip .el-icon { margin-right: 3px; vertical-align: -2px; }

.gv-stats { display: flex; gap: 10px; margin-bottom: 12px; }
.gv-stat {
  flex: 1; display: flex; flex-direction: column; align-items: center; gap: 3px;
  padding: 11px 6px; border-radius: 10px; border: 1px solid var(--dc-border);
  background: var(--dc-bg-soft); transition: border-color .16s ease;
}
.gv-stat:hover { border-color: var(--dc-primary); }
.gv-stat .n { font-size: 18px; font-weight: 700; color: var(--dc-text-strong); font-variant-numeric: tabular-nums; line-height: 1.1; }
.gv-stat .l { font-size: 12px; color: var(--dc-text-dim); }

/* 数据容量：卡片外壳沿用通用 .gv-row（与「敏感数据」一致），内部按容量场景重排 */
.gv-cap-head { display: flex; align-items: baseline; gap: 10px; }
.gv-cap-name {
  flex: 1; min-width: 0; font-size: 14px; font-weight: 600; color: var(--dc-text);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.gv-cap-size {
  flex-shrink: 0; font-size: 14px; font-weight: 700; color: var(--dc-primary);
  font-variant-numeric: tabular-nums;
}
.gv-cap-size.dim { color: var(--dc-text-weak); font-weight: 500; }
.gv-cap-meta {
  display: flex; align-items: center; gap: 8px; margin-top: 3px;
  font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.6;
}
.gv-cap-comment { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.gv-cap-rows { flex-shrink: 0; }
.gv-cap-pct {
  flex-shrink: 0; color: var(--dc-text-weak);
  font-variant-numeric: tabular-nums;
}
.gv-rank-bar {
  height: 4px; margin-top: 6px; border-radius: 999px; overflow: hidden;
  background: var(--dc-bg-deep);
}
.gv-rank-bar > i {
  display: block; height: 100%; border-radius: 999px;
  background: linear-gradient(90deg, var(--dc-primary), var(--dc-purple));
  transition: width .28s ease;
}
.gv-stat.lv-high .n { color: var(--dc-danger); }
.gv-stat.lv-medium .n { color: var(--dc-warning, #e6a23c); }

.gv-row {
  display: flex; align-items: flex-start; gap: 10px;
  padding: 10px 13px; border-radius: 9px; margin-bottom: 7px;
  border: 1px solid var(--dc-border); border-left-width: 3px; background: var(--dc-bg-soft);
}
.gv-row.lv-high { border-left-color: var(--dc-danger); }
.gv-row.lv-medium { border-left-color: var(--dc-warning, #e6a23c); }
.gv-row.lv-low { border-left-color: var(--dc-link); }
.gv-row.lv-info { border-left-color: var(--dc-text-weak); }
.gv-level {
  flex-shrink: 0; font-size: 12px; font-weight: 700; padding: 2px 8px; border-radius: 999px;
  background: var(--dc-bg-deep); color: var(--dc-text-dim);
}
.gv-main { flex: 1; min-width: 0; }
.gv-target { font-size: 14px; font-weight: 600; color: var(--dc-text); word-break: break-all; }
.gv-col { color: var(--dc-text-mid); font-weight: 400; }
.gv-arrow { margin: 0 6px; color: var(--dc-primary); }
.gv-tag {
  margin-left: 8px; font-size: 10.5px; font-weight: 600; padding: 1px 7px; border-radius: 999px;
  background: var(--dc-primary-wash); color: var(--dc-primary);
}
.gv-src { margin-left: 8px; font-size: 10.5px; color: var(--dc-text-weak); font-weight: 400; }
.gv-msg { font-size: 13px; color: var(--dc-text-dim); line-height: 1.65; margin-top: 3px; }
.gv-allow { font-size: 13px; color: var(--dc-link); margin-top: 4px; }
.gv-sql {
  display: block; margin-top: 6px; font-size: 11.5px; color: var(--dc-link);
  background: var(--dc-bg-deep); border-radius: 6px; padding: 5px 8px;
  word-break: break-all; font-family: 'SF Mono', ui-monospace, Consolas, monospace;
}
.gv-notes { margin-top: 10px; font-size: 13px; color: var(--dc-text-dim); line-height: 1.7; }
.gv-actions { display: flex; justify-content: flex-end; margin: 10px 0; }
.gv-check-head {
  font-size: 13px; font-weight: 600; color: var(--dc-text-mid);
  background: var(--dc-bg-soft); border-radius: 8px; padding: 8px 12px; margin-bottom: 8px;
}
</style>
