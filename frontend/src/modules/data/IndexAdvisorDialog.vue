<!--
  索引建议 · 动作闭环

  规则式分析（不调模型、不花钱）→ 每条建议可以「试建」也可以「撤销」。

  为什么要在界面里做这一步：索引建议最尴尬的地方是"提了没人敢执行"——不知道
  加了会不会更慢、加错了怎么退。这里把后端 /api/ai/action/* 的四步协议接上：
  预检（风险 + 撤销语句）→ 执行 → 撤销，执行完立刻把「撤销」摆在原地。
-->
<template>
  <el-dialog :model-value="modelValue" width="880px" top="8vh" append-to-body class="main-dialog idx-adv-dialog"
             @update:model-value="$emit('update:modelValue', $event)">
    <template #header>
      <!-- 标题：主标题 + 表名做副标题（表名可能很长，单独一行会撑乱标题栏），
           右侧顺带说清「几条建议」，用户不用自己数 -->
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Coin /></el-icon></span>
        <span class="dlg-title-main">{{ $t('idxAdv.titleMain') }}</span>
        <span class="dlg-title-sub" :title="table">{{ table }}</span>
        <span v-if="!loading && items.length" class="dlg-title-count">{{ $t('idxAdv.count', { n: items.length }) }}</span>
      </div>
    </template>

    <div v-loading="loading" class="adv-body">
      <!-- 提示固定一行：早先它折成两行（最后几个字掉到下一行），看着像正文没排好 -->
      <div class="adv-hint" :title="$t('idxAdv.hint')">{{ $t('idxAdv.hint') }}</div>

      <div v-if="!loading && !items.length" class="adv-empty">{{ $t('idxAdv.empty') }}</div>

      <div v-for="(item, index) in items" :key="index" class="adv-row">
        <div class="row-head">
          <span class="row-kind" :class="item.kind === 'primary_key' ? 'is-pk' : ''">
            {{ item.kind === 'primary_key' ? $t('idxAdv.kindPk') : $t('idxAdv.kindIndex') }}
          </span>
          <span class="row-target">{{ item.column ? `${item.table}.${item.column}` : item.table }}</span>
          <span class="row-state" v-if="stateOf(index) === 'applied'">{{ $t('idxAdv.applied') }}</span>
          <span class="row-state is-off" v-else-if="stateOf(index) === 'undone'">{{ $t('idxAdv.undone') }}</span>
          <span class="row-actions">
            <el-button v-if="item.ddl && stateOf(index) !== 'applied'" size="small" type="primary"
                       :loading="busy === index" @click="apply(item, index)">{{ $t('idxAdv.tryBuild') }}</el-button>
            <el-button v-if="stateOf(index) === 'applied'" size="small" :loading="busy === index"
                       @click="undo(index)">{{ $t('idxAdv.undo') }}</el-button>
          </span>
        </div>
        <div class="row-reason">{{ item.reason }}</div>
        <pre v-if="item.ddl" class="dc-code-block sql adv-ddl">{{ item.ddl }}</pre>
      </div>
    </div>

    <template #footer>
      <el-button size="small" @click="$emit('update:modelValue', false)">{{ $t('common.close') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, reactive, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Coin } from '@element-plus/icons-vue'
import { t } from '../../utils/i18n'
import { aiIndexAdvisor, aiActionPlan, aiActionApply, aiActionUndo } from '../../api'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  connId: { type: String, default: '' },
  database: { type: String, default: '' },
  table: { type: String, default: '' },
})
const emit = defineEmits(['update:modelValue'])

const loading = ref(false)
const busy = ref(-1)
const items = ref([])
/** 每条建议的执行状态：applied / undone；配套记住后端给的撤销语句 */
const states = reactive({})
const undoMap = reactive({})

const stateOf = (index) => states[index] || ''

const load = async () => {
  if (!props.connId || !props.table) return
  loading.value = true
  items.value = []
  Object.keys(states).forEach((key) => delete states[key])
  Object.keys(undoMap).forEach((key) => delete undoMap[key])
  try {
    const res = await aiIndexAdvisor({
      connectionId: props.connId,
      database: props.database,
      table: props.table,
    })
    if (res && res.success) items.value = (res.items || []).filter((item) => item.table === props.table)
    else ElMessage.error(res?.message || t('ai.requestFailed'))
  } catch (e) {
    ElMessage.error(e?.message || t('ai.requestFailed'))
  }
  loading.value = false
}

/** 试建：先预检拿风险与撤销语句，风险高必须二次确认，再执行 */
const apply = async (item, index) => {
  busy.value = index
  try {
    const plan = await aiActionPlan({ sql: item.ddl, connectionId: props.connId, database: props.database })
    const action = plan?.action || {}
    if (action.risk === 'high' || action.reversible === false) {
      // 没有撤销语句的动作，必须让用户明确知道"撤不回来"
      const tips = (action.items || []).map((one) => `· ${one.title}`).join('\n')
      await ElMessageBox.confirm(
        `${tips}\n\n${action.note || ''}`.trim(),
        t('idxAdv.riskTitle'),
        { type: 'warning', confirmButtonText: t('idxAdv.riskConfirm'), cancelButtonText: t('common.cancel') }
      )
    }
    const res = await aiActionApply({
      sql: item.ddl,
      connectionId: props.connId,
      database: props.database,
      confirm: true,
    })
    if (res && res.success) {
      states[index] = 'applied'
      undoMap[index] = res.undoSql || action.undoSql || ''
      ElMessage.success(t('idxAdv.appliedOk', { ms: res.elapsedMs || 0 }))
    } else {
      ElMessage.error(res?.message || t('idxAdv.applyFailed'))
    }
  } catch (e) {
    // 用户在确认框点了取消：不是错误，不要弹红条
    if (e !== 'cancel' && e !== 'close') ElMessage.error(e?.message || t('idxAdv.applyFailed'))
  }
  busy.value = -1
}

const undo = async (index) => {
  const sql = undoMap[index]
  if (!sql) return
  busy.value = index
  try {
    const res = await aiActionUndo({ sql, connectionId: props.connId, database: props.database })
    if (res && res.success) {
      states[index] = 'undone'
      ElMessage.success(t('idxAdv.undoneOk', { ms: res.elapsedMs || 0 }))
    } else {
      ElMessage.error(res?.message || t('idxAdv.undoFailed'))
    }
  } catch (e) {
    ElMessage.error(e?.message || t('idxAdv.undoFailed'))
  }
  busy.value = -1
}

watch(
  () => props.modelValue,
  (open) => {
    if (open) load()
  }
)
</script>

<style scoped>
/* 标题：主标题正常字号，表名与条数降两档做对比（与其它弹窗的 .dlg-title-sub 同一口径）。
   右侧 44px 是给 Element 自带的关闭 ×（.el-dialog__headerbtn，贴右边缘、宽约 48px）让位的：
   标题容器铺满整个 header，不留这块空当，「共 N 条建议」就会压在 × 上（真机反馈）。 */
.dlg-title { display: flex; align-items: center; gap: 8px; min-width: 0; padding-right: 44px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex; flex: none;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}
.dlg-title-main { font-weight: 600; flex: none; }
.dlg-title-sub {
  font-size: 12px; font-weight: 400; color: var(--dc-text-dim);
  padding: 1px 8px; border-radius: 5px; background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border-soft); font-family: var(--dc-font-mono, monospace);
  max-width: 320px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
/* 条数右对齐但可缩：窗口窄 / 表名很长时优先省略它，而不是把标题栏顶破 */
.dlg-title-count {
  font-size: 12px; font-weight: 400; color: var(--dc-text-dim);
  margin-left: auto; flex: 0 1 auto; min-width: 0;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}

.adv-body { min-height: 120px; max-height: 62vh; overflow: auto; }
/* 提示固定一行：折行会让人以为正文没排好（真机反馈），超长就省略，完整文案在 title 里 */
.adv-hint {
  color: var(--dc-text-dim); font-size: 12.5px; margin-bottom: 12px; line-height: 1.6;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.adv-empty { color: var(--dc-text-dim); font-size: 13px; padding: 28px 0; text-align: center; }
.adv-row {
  border: 1px solid var(--dc-border); border-radius: 8px; padding: 10px 12px; margin-bottom: 10px;
  background: var(--dc-bg-card);
}
.row-head { display: flex; align-items: center; gap: 8px; }
.row-kind {
  flex: none; font-size: 11.5px; padding: 1px 7px; border-radius: 4px;
  background: var(--dc-primary-wash); color: var(--dc-primary);
}
.row-kind.is-pk { background: var(--dc-warn-wash); color: var(--dc-warn-text); }
.row-target { flex: 1 1 auto; min-width: 0; font-weight: 600; font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.row-state { flex: none; font-size: 12px; color: var(--dc-success); }
.row-state.is-off { color: var(--dc-text-dim); }
.row-actions { flex: none; display: flex; gap: 6px; }
.row-reason { color: var(--dc-text-dim); font-size: 12.5px; line-height: 1.6; margin: 6px 0 0; }
.adv-ddl { margin: 8px 0 0; font-size: 12px; padding: 8px 10px; }
</style>
