<template>
  <el-dialog
    :model-value="modelValue"
    @update:model-value="$emit('update:modelValue', $event)"
    width="760px"
    append-to-body
    class="ai-patrol-dialog"
  >
    <template #header>
      <div class="pt-title">
        <span class="pt-title-ic"><el-icon :size="16"><Aim /></el-icon></span>
        <span>{{ $t('pt.title') }}</span>
        <span class="pt-title-sub">{{ $t('pt.subtitle') }}</span>
      </div>
    </template>

    <div class="pt-bar">
      <el-select v-model="connId" size="small" style="width:210px" :placeholder="$t('ai.pickSource')" filterable>
        <!-- 按「目录」分组，多级显示（与左侧对象树口径一致） -->
        <el-option-group v-for="g in connGroups" :key="g.label" :label="g.label">
          <el-option v-for="c in g.options" :key="c.id" :label="connLabel(c)" :value="c.id" />
        </el-option-group>
      </el-select>
      <el-select
        v-model="database"
        size="small"
        style="width:180px"
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
        style="width:140px"
        :placeholder="$t('ai.pickSchema')"
        filterable
        :loading="loadingSchemas"
        :disabled="!database"
      >
        <el-option v-for="s in schemas" :key="s" :label="s" :value="s" />
      </el-select>
      <el-button size="small" type="primary" :loading="loading" :disabled="!activeConn || !database || isNoSqlConn" @click="run">{{ $t('pt.start') }}</el-button>
      <span class="pt-hint">{{ $t('pt.hint') }}</span>
      <div v-if="isNoSqlConn" class="pt-warn">
        {{ $t('pt.nosqlTip', { type: activeConn?.type }) }}
      </div>
      <div v-else-if="dbError" class="pt-err" :title="dbError">{{ truncateError(dbError) }}</div>
    </div>

    <div v-if="loading" class="pt-loading">{{ $t('pt.scanning') }}</div>

    <template v-else-if="result">
      <div class="pt-stats">
        <div class="pt-stat">
          <span class="pt-n">{{ result.scanned || 0 }}</span>
          <span class="pt-l">{{ $t('pt.statTables') }}</span>
        </div>
        <div class="pt-stat lv-medium">
          <span class="pt-n">{{ result.medium || 0 }}</span>
          <span class="pt-l">{{ $t('pt.statMedium') }}</span>
        </div>
        <div class="pt-stat lv-low">
          <span class="pt-n">{{ result.low || 0 }}</span>
          <span class="pt-l">{{ $t('pt.statLow') }}</span>
        </div>
        <div class="pt-stat">
          <span class="pt-n">{{ result.totalTables || 0 }}</span>
          <span class="pt-l">{{ $t('pt.statTotal') }}</span>
        </div>
      </div>

      <div v-if="!issues.length" class="pt-empty">{{ $t('pt.noIssue') }}</div>
      <div v-else class="pt-list">
        <div v-for="(it, i) in issues" :key="i" class="pt-row" :class="'lv-' + it.level">
          <span class="pt-level">{{ levelText(it.level) }}</span>
          <div class="pt-main">
            <div class="pt-target">
              {{ it.table }}<span v-if="it.column && it.column !== '-'"> · {{ it.column }}</span>
            </div>
            <div class="pt-msg">{{ it.message }}</div>
          </div>
        </div>
      </div>
    </template>

    <div v-else class="pt-empty">{{ activeConn ? $t('pt.pickFirst') : $t('pt.pickSourceFirst') }}</div>

    <template #footer>
      <el-button @click="$emit('update:modelValue', false)">{{ $t('common.close') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref } from 'vue'
import { ElMessage } from 'element-plus'
import { Aim } from '@element-plus/icons-vue'
import { aiPatrol } from '../../api'
import { t, te } from '../../utils/i18n'
import { useAiContext, truncateError } from './useAiContext'

const props = defineProps({ modelValue: Boolean, conn: Object, database: String })
defineEmits(['update:modelValue'])

// 连接 + 数据库 + 模式三级选择（未在对象树选中连接时，本面板仍可用）
const {
  connId, dbs, database, schemas, schema, connGroups,
  activeConn, needSchema, isNoSqlConn, effectiveDatabase,
  loadingDbs, loadingSchemas, dbError, connLabel
} = useAiContext(props)

const loading = ref(false)
const result = ref(null)
const issues = ref([])

// 级别名按代码查字典（high/medium/low/info 这些代码来自后端），没收录的原样显示
const LEVEL_KEYS = { high: 'ai.lv.high', medium: 'ai.lv.medium', low: 'ai.lv.low', info: 'ai.lv.info' }
const levelText = (lv) => (lv && te(LEVEL_KEYS[lv])) ? t(LEVEL_KEYS[lv]) : (lv || '')

const run = async () => {
  if (!activeConn.value || !database.value) { ElMessage.warning(t('pt.needSel')); return }
  loading.value = true
  result.value = null
  issues.value = []
  try {
    const res = await aiPatrol({ connectionId: activeConn.value.id, database: effectiveDatabase.value, maxTables: 500 })
    if (res && res.success) {
      result.value = res
      issues.value = res.issues || []
    } else {
      ElMessage.error(res?.message || t('pt.failed'))
    }
  } catch (e) {
    ElMessage.error(e?.message || e?.toString?.() || t('pt.failed'))
  }
  loading.value = false
}
</script>

<style scoped>
.pt-title { display: flex; align-items: center; gap: 8px; }
.pt-title-ic {
  width: 26px; height: 26px; border-radius: 8px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-purple));
}
.pt-title-sub { font-size: 12px; color: var(--dc-text-dim); font-weight: 400; }

.pt-bar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-bottom: 14px; }
/* 提示独占一行，避免与选择器挤在一起 */
.pt-hint { font-size: 12px; color: var(--dc-text-weak); flex-basis: 100%; margin-top: 2px; }
.pt-err {
  font-size: 11.5px; color: var(--dc-danger); flex-basis: 100%;
  background: rgba(245, 108, 108, .08); border-radius: 6px; padding: 5px 9px;
  /* 最长两行，避免超长堆栈把面板撑爆；完整内容用 title 悬浮查看 */
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical;
  overflow: hidden; word-break: break-all; cursor: help;
}
.pt-warn {
  font-size: 11.5px; color: var(--dc-warning, #e6a23c); flex-basis: 100%;
  background: rgba(230, 162, 60, .10); border-radius: 6px; padding: 5px 9px;
}

.pt-loading { padding: 30px 0; text-align: center; color: var(--dc-text-dim); font-size: 14px; }
.pt-empty {
  padding: 22px; text-align: center; color: var(--dc-text-dim); font-size: 13px;
  background: var(--dc-bg-soft); border: 1px dashed var(--dc-border); border-radius: 10px;
}

.pt-stats { display: flex; gap: 10px; margin-bottom: 14px; }
.pt-stat {
  flex: 1; display: flex; flex-direction: column; align-items: center; gap: 2px;
  padding: 10px 6px; border-radius: 10px; border: 1px solid var(--dc-border); background: var(--dc-bg-soft);
}
.pt-n { font-size: 18px; font-weight: 700; color: var(--dc-text-strong); font-variant-numeric: tabular-nums; }
.pt-l { font-size: 12px; color: var(--dc-text-dim); }
.pt-stat.lv-medium .pt-n { color: var(--dc-warning, #e6a23c); }
.pt-stat.lv-low .pt-n { color: var(--dc-link); }

.pt-list { max-height: 380px; overflow: auto; display: flex; flex-direction: column; gap: 6px; }
.pt-row {
  display: flex; align-items: flex-start; gap: 10px;
  padding: 9px 12px; border-radius: 9px;
  border: 1px solid var(--dc-border); border-left-width: 3px; background: var(--dc-bg-soft);
}
.pt-row.lv-high { border-left-color: var(--dc-danger); }
.pt-row.lv-medium { border-left-color: var(--dc-warning, #e6a23c); }
.pt-row.lv-low { border-left-color: var(--dc-link); }
.pt-row.lv-info { border-left-color: var(--dc-text-weak); }
.pt-level {
  flex-shrink: 0; font-size: 10.5px; font-weight: 700; padding: 1px 7px; border-radius: 999px;
  background: var(--dc-bg-deep); color: var(--dc-text-dim); margin-top: 1px;
}
.pt-row.lv-high .pt-level { color: var(--dc-danger); }
.pt-row.lv-medium .pt-level { color: var(--dc-warning, #e6a23c); }
.pt-main { min-width: 0; flex: 1; }
.pt-target { font-size: 13px; font-weight: 600; color: var(--dc-text); word-break: break-all; }
.pt-msg { font-size: 13px; color: var(--dc-text-dim); line-height: 1.6; margin-top: 2px; }
</style>
