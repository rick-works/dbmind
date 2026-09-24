<template>
  <Teleport to="body">
    <div v-if="modelValue" class="cp-mask" @click.self="close">
      <div class="cp-box">
        <!-- 已发送的指令回显：构成「我问了什么 → 得到什么」的阅读顺序 -->
        <div v-if="submitted" class="cp-ask">
          <el-icon><User /></el-icon>
          <span class="cp-ask-text">{{ submitted }}</span>
        </div>

        <div class="cp-input-row">
          <el-icon class="cp-ic"><Search /></el-icon>
          <!-- 用 textarea 而非 input：单行时外观一致，内容变多会自动增高（最多约 4 行），
               长指令不必左右滚动就能看全；Enter 提交、Shift+Enter 换行 -->
          <textarea
            ref="inputRef"
            v-model="text"
            class="cp-input"
            rows="1"
            :placeholder="$t('cp.placeholder')"
            @keydown.enter.exact.prevent="run"
            @keydown.esc.prevent="close"
            @input="autoGrow"
          ></textarea>
          <!-- 发送按钮取代原来的 Esc 提示：Esc 仍可作为快捷键关闭面板 -->
          <el-button
            class="cp-send"
            size="small"
            type="primary"
            :disabled="!text.trim() || loading"
            :loading="loading"
            @click="run"
          >{{ $t('ai.runChat') }}</el-button>
        </div>

        <div class="cp-ctx">
          <el-icon class="cp-ctx-ic"><Connection /></el-icon>
          <el-select v-model="connId" size="small" class="cp-sel" popper-class="cp-select-popper" :placeholder="$t('ai.pickSource')" filterable>
            <!-- 按「目录」分组，多级显示（与左侧对象树口径一致） -->
            <el-option-group v-for="g in connGroups" :key="g.label" :label="g.label">
              <el-option v-for="c in g.options" :key="c.id" :label="connLabel(c)" :value="c.id" />
            </el-option-group>
          </el-select>
          <el-select
            v-model="database"
            size="small"
            class="cp-sel"
            popper-class="cp-select-popper"
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
            class="cp-sel"
            popper-class="cp-select-popper"
            :placeholder="$t('ai.pickSchema')"
            filterable
            :loading="loadingSchemas"
            :disabled="!database"
          >
            <el-option v-for="s in schemas" :key="s" :label="s" :value="s" />
          </el-select>
          <!-- 状态收成行尾一个小图标：详情走悬浮提示，不再另起一行悬挂在下拉下方 -->
          <span
            v-if="isNoSqlConn"
            class="cp-ctx-state warn"
            :title="$t('cp.nosqlTip', { type: activeConn?.type })"
          >
            <el-icon><Warning /></el-icon><span class="cp-ctx-state-txt">NoSQL</span>
          </span>
          <span v-else-if="dbError" class="cp-ctx-state err" :title="truncateError(dbError)">
            <el-icon><Warning /></el-icon><span class="cp-ctx-state-txt">{{ $t('cp.ctxFailed') }}</span>
          </span>
          <span
            v-else-if="!activeConn"
            class="cp-ctx-state dim"
            :title="$t('cp.noConnTip')"
          >
            <el-icon><Connection /></el-icon><span class="cp-ctx-state-txt">{{ $t('cp.noConn') }}</span>
          </span>
          <span
            v-else
            class="cp-ctx-state ok"
            :title="$t('cp.ctxTip', { name: (effectiveDatabase || $t('cp.currentSource')) })"
          >
            <el-icon><CircleCheck /></el-icon>
          </span>
        </div>

        <div v-if="loading" class="cp-loading">
          <el-icon class="is-loading"><Loading /></el-icon>
          <span>{{ $t('cp.parsing') }}</span>
        </div>

        <div v-else-if="actions.length" class="cp-plan">
          <template v-for="(a, i) in actions" :key="i">
            <!-- 注意：这里必须用 v-else-if 明确排除 _meta。
                 若写成 v-if="a.type === '_meta' && showSummary" + v-else，
                 当摘要被隐藏（showSummary=false）时，_meta 行会落到 v-else，
                 被错误渲染成一张操作卡片。 -->
            <div v-if="a.type === '_meta' && showSummary" class="cp-summary">{{ a.summary || a.intent }}</div>
            <div v-else-if="a.type !== '_meta'" class="cp-action" :class="{ danger: a.needConfirm }">
              <span class="cp-badge" :class="{ danger: a.needConfirm }">{{ actionLabel(a.type) }}</span>
              <div class="cp-action-body">
                <div class="cp-action-title">{{ describe(a) }}</div>
                <code v-if="a.params && a.params.sql" class="cp-sql">{{ a.params.sql }}</code>
              </div>
              <span v-if="a.needConfirm" class="cp-warn">{{ $t('cp.needConfirm') }}</span>
            </div>
          </template>
          <div class="cp-actions">
            <el-button size="small" @click="reset">{{ $t('cp.reset') }}</el-button>
            <el-button size="small" type="primary" :disabled="!runnable" @click="confirmRun">{{ $t('cp.runPlan') }}</el-button>
          </div>
        </div>

        <div v-else class="cp-hints">
          <div class="cp-hint-title">{{ $t('ai.tryAsk') }}</div>
          <div v-for="h in hints" :key="h" class="cp-hint" @click="pickHint(h)">{{ h }}</div>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup>
import { ref, watch, nextTick, computed } from 'vue'
import { ElMessage } from 'element-plus'
import { t, te } from '../../utils/i18n'
import { Search, Loading, Connection, CircleCheck, Warning, User } from '@element-plus/icons-vue'
import { aiPlan } from '../../api'
import { useAiContext, truncateError } from './useAiContext'

const props = defineProps({ modelValue: Boolean, conn: Object, database: String })
const emit = defineEmits(['update:modelValue', 'run-plan'])

// 连接 + 数据库 + 模式三级选择（未在对象树选中连接时，本面板仍可用）
const {
  connId, dbs, database, schemas, schema, connGroups,
  activeConn, needSchema, isNoSqlConn, effectiveDatabase,
  loadingDbs, loadingSchemas, dbError, connLabel
} = useAiContext(props)

const inputRef = ref(null)
const text = ref('')
/** 已发送的指令（发送后回显在结果上方） */
const submitted = ref('')
const loading = ref(false)
const actions = ref([])

// ⚠️ 必须是 computed：静态数组只在模块加载时求值一次，之后切语言不会跟着变
const hints = computed(() => [
  t('cp.hint1'),
  t('cp.hint2'),
  t('cp.hint3'),
  t('cp.hint4'),
  t('cp.hint5')
])

// 动作名按代码查字典（代码来自后端）：写死成常量只会在加载时求值一次，切语言不跟着变
const LABEL_KEYS = {
  open_query: 'cp.lb.open_query', run_sql: 'cp.lb.run_sql', export_table: 'qa.exportBtn',
  open_panel: 'cp.lb.open_panel', analyze_table: 'cp.lb.analyze_table', create_index: 'cp.lb.create_index',
  search: 'cp.lb.search', answer: 'cp.lb.answer'
}
const actionLabel = (ty) => (ty && te(LABEL_KEYS[ty])) ? t(LABEL_KEYS[ty]) : ty

/**
 * 是否展示「摘要」这一段。
 *
 * 摘要与下面的操作卡片/回答说的是同一件事（如「已为你打开数据治理面板」
 * 对应卡片「打开面板：数据治理」），同时显示两段显得啰嗦。
 * 因此只在**没有任何可展示条目**时才用摘要兜底，其余情况一律不显示。
 */
const showSummary = computed(() => actions.value.filter(a => a.type !== '_meta').length === 0)

const DESCRIBE = {
  open_query: (p) => t('cp.act.open_query'),
  run_sql: (p) => t('cp.act.run_sql'),
  export_table: (p) => t('cp.act.export_table', { table: (p.table || ''), fmt: (p.format || 'csv').toUpperCase() }),
  open_panel: (p) => t('cp.act.open_panel', { name: (({ ai: t('empty.aiAssistant'), patrol: t('cp.panel.patrol'), governance: t('gv.title'), settings: t('nav.settings'), compare: t('ai.cmdCompare'), sync: t('ai.cmdSync') })[p.panel] || p.panel) }),
  analyze_table: (p) => t('cp.act.analyze_table', { table: (p.table || '') }),
  create_index: (p) => t('cp.act.create_index', { table: (p.table || '') }),
  search: (p) => t('cp.act.search', { keyword: (p.keyword || '') }),
  answer: (p) => p.text || ''
}

const describe = (a) => {
  const p = a.params || {}
  const fn = DESCRIBE[a.type]
  return fn ? fn(p) : JSON.stringify(p)
}

const runnable = computed(() => actions.value.some(a => a.type && a.type !== '_meta'))

const reset = () => {
  text.value = ''
  submitted.value = ''
  actions.value = []
  nextTick(autoGrow)   // 清空后把输入框高度收回单行
  focusInput()
}

const focusInput = () => nextTick(() => inputRef.value && inputRef.value.focus())

const close = () => emit('update:modelValue', false)

/** 输入框最大高度（约 4 行）；超出后框内滚动，避免长文本把面板撑变形 */
const INPUT_MAX_H = 92

/**
 * 输入框高度随内容自适应。
 * 取 scrollHeight 后再用 INPUT_MAX_H 封顶；同时把上限也写进 CSS（max-height），
 * 双保险避免个别情况下高度失控（长文本溢出输入框）。
 */
const autoGrow = () => {
  const el = inputRef.value
  if (!el) return
  el.style.height = 'auto'
  const h = Math.min(el.scrollHeight, INPUT_MAX_H)
  el.style.height = h + 'px'
  el.style.overflowY = el.scrollHeight > INPUT_MAX_H ? 'auto' : 'hidden'
}

const pickHint = (h) => {
  text.value = h
  nextTick(autoGrow)
  run()
}

const run = async () => {
  const cmd = text.value.trim()
  if (!cmd || loading.value) return
  loading.value = true
  submitted.value = cmd   // 回显本次指令，便于核对"我问的"与"它答的"
  actions.value = []
  try {
    const res = await aiPlan({
      command: cmd,
      connectionId: activeConn.value?.id || '',
      database: effectiveDatabase.value || ''
    })
    if (res && res.success) {
      actions.value = res.actions || []
      if (!actions.value.length) ElMessage.warning(t('cp.noAction'))
    } else {
      ElMessage.error(res?.message || t('cp.parseFailed'))
    }
  } catch (e) {
    ElMessage.error(e?.message || e?.toString?.() || t('cp.parseFailed'))
  }
  loading.value = false
}

const confirmRun = () => {
  const list = actions.value.filter(a => a.type && a.type !== '_meta')
  if (!list.length) return
  emit('run-plan', list)
  close()
}

watch(() => props.modelValue, (v) => {
  if (v) { reset() }
})
</script>

<style scoped>
.cp-mask {
  position: fixed; inset: 0; z-index: 3000;
  background: rgba(0, 0, 0, .35);
  display: flex; align-items: flex-start; justify-content: center;
  padding-top: 12vh;
  backdrop-filter: blur(2px);
}
.cp-box {
  width: 720px; max-width: 92vw; max-height: 70vh;
  display: flex; flex-direction: column;
  background: var(--dc-bg-card); border: 1px solid var(--dc-border);
  border-radius: 14px; overflow: hidden;
  box-shadow: 0 18px 48px rgba(0, 0, 0, .28);
  animation: cpIn .16s ease;
}
@keyframes cpIn { from { opacity: 0; transform: translateY(-8px) scale(.99); } to { opacity: 1; transform: none; } }

/* 布局顺序（用 order 重排，避免大改模板），自上而下：
   1 结果区（建议 / 加载 / 计划）→ 2 上下文状态栏（数据源/库/模式）→ 3 输入框。
   输入框贴底，像聊天输入框一样：先看结果、再决定怎么问；
   上下文是"当前环境状态"，紧贴输入框上方，填写与提问在同一区域。 */
.cp-input-row {
  order: 3; flex-shrink: 0;
  /* 底部对齐：输入框多行增高时，左侧图标与右侧 Esc 贴合最后一行，观感更自然 */
  display: flex; align-items: flex-end; gap: 10px;
  padding: 12px 16px; border-top: 1px solid var(--dc-border);
}
.cp-ic { color: var(--dc-text-dim); }
.cp-input {
  flex: 1; border: none; outline: none; background: transparent;
  color: var(--dc-text); font-size: 14px; font-family: inherit;
  /* 多行自适应：高度由 autoGrow() 按内容计算，上限约 4 行后改为内部滚动。
     max-height 这里是兜底（与 autoGrow 的上限一致），保证任何情况下都不会溢出输入框 */
  resize: none; padding: 0; line-height: 1.55;
  height: 22px; max-height: 92px; overflow-y: hidden; box-sizing: border-box;
}
.cp-input::placeholder { color: var(--dc-text-weak); }
.cp-kbd {
  font-size: 11px; color: var(--dc-text-weak); border: 1px solid var(--dc-border);
  border-radius: 5px; padding: 1px 6px;
}
.cp-send { flex-shrink: 0; }

/* 已发送的指令回显：置顶（order 最小），与下方结果区用边框分隔 */
.cp-ask {
  order: 0; flex-shrink: 0;
  display: flex; align-items: flex-start; gap: 8px;
  padding: 11px 16px; border-bottom: 1px solid var(--dc-border);
  background: var(--dc-bg-soft);
  font-size: 13px; color: var(--dc-text); line-height: 1.6;
  white-space: pre-wrap; word-break: break-word;
}
.cp-ask .el-icon { flex-shrink: 0; margin-top: 2px; color: var(--dc-text-weak); }
.cp-ask-text { flex: 1; min-width: 0; }

.cp-ctx {
  order: 2; flex-shrink: 0;
  display: flex; align-items: center; gap: 8px; flex-wrap: wrap;
  padding: 9px 16px; border-top: 1px solid var(--dc-border);
  background: var(--dc-bg-soft);
}
.cp-ctx-ic { color: var(--dc-text-dim); flex-shrink: 0; }
/* 三个下拉等宽平分剩余空间（不再写死像素），视觉上整齐对齐 */
.cp-ctx :deep(.el-select.cp-sel) { flex: 1 1 0; min-width: 0; }
/* 上下文状态：整行最右侧的小标识，详情用 title 悬浮查看。
   正常时只是一个绿勾（不占文字宽度），异常时才带短文字。 */
.cp-ctx-state {
  flex-shrink: 0; display: inline-flex; align-items: center; gap: 3px;
  font-size: 12px; line-height: 1; color: var(--dc-text-weak); cursor: help;
}
.cp-ctx-state .el-icon { flex-shrink: 0; }
.cp-ctx-state-txt { white-space: nowrap; }
.cp-ctx-state.ok { color: #67c23a; }
.cp-ctx-state.err { color: var(--dc-danger); }
.cp-ctx-state.warn { color: var(--dc-warning, #e6a23c); }
.cp-ctx-state.dim { color: var(--dc-text-weak); }
.cp-ctx-err { font-size: 12px; color: var(--dc-danger); cursor: help; }
.cp-ctx-warn { font-size: 12px; color: var(--dc-warning, #e6a23c); }

/* 结果区：置顶并占满剩余高度（下方依次是上下文状态栏、输入框） */
.cp-loading {
  order: 1; flex: 1; min-height: 0;
  padding: 22px; display: flex; align-items: center; gap: 8px;
  color: var(--dc-text-dim); font-size: 14px;
}

.cp-plan { order: 1; flex: 1; min-height: 0; padding: 14px 16px 16px; overflow: auto; }
/* 摘要：左侧一道主色竖条，与下方操作卡片区分开，读起来像「结论」而不是普通段落 */
.cp-summary {
  position: relative; font-size: 13px; color: var(--dc-text-mid);
  background: var(--dc-bg-soft); border: 1px solid var(--dc-border);
  border-radius: 8px; padding: 9px 12px 9px 14px; margin-bottom: 12px;
  line-height: 1.65;
}
.cp-summary::before {
  content: ''; position: absolute; left: 0; top: 9px; bottom: 9px; width: 3px;
  border-radius: 0 3px 3px 0; background: var(--dc-primary);
}
/* 操作卡片：左侧 3px 强调边（需确认的转红），悬浮时底色提亮 */
.cp-action {
  display: flex; align-items: flex-start; gap: 10px;
  padding: 10px 12px; border-radius: 9px;
  border: 1px solid var(--dc-border); border-left-width: 3px; border-left-color: var(--dc-primary);
  background: var(--dc-bg-soft); margin-bottom: 8px;
  transition: background .16s ease, border-color .16s ease;
}
.cp-action:hover { background: var(--dc-bg-card); }
.cp-action.danger { border-left-color: var(--dc-danger); }
.cp-badge {
  flex-shrink: 0; font-size: 12px; font-weight: 600; color: var(--dc-primary);
  background: var(--dc-primary-wash); border-radius: 6px; padding: 2px 8px;
  line-height: 1.6;
}
.cp-badge.danger { color: var(--dc-danger); background: rgba(245, 108, 108, .12); }
.cp-action-body { flex: 1; min-width: 0; }
.cp-action-title { font-size: 13px; color: var(--dc-text); line-height: 1.65; }
.cp-sql {
  display: block; margin-top: 6px; font-size: 11.5px; color: var(--dc-link);
  background: var(--dc-bg-deep); border-radius: 6px; padding: 5px 8px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-family: 'SF Mono', ui-monospace, Consolas, monospace;
}
.cp-warn {
  flex-shrink: 0; font-size: 12px; color: var(--dc-warning, #e6a23c);
  display: inline-flex; align-items: center;
}
/* 按钮区：与上方卡片用分隔线断开，避免挤在一起 */
.cp-actions {
  display: flex; justify-content: flex-end; gap: 8px;
  margin-top: 12px; padding-top: 12px; border-top: 1px solid var(--dc-border);
}

.cp-hints { order: 1; flex: 1; min-height: 0; padding: 16px 16px 18px; overflow: auto; }
.cp-hint-title {
  font-size: 12px; font-weight: 600; color: var(--dc-text-dim);
  letter-spacing: .3px; margin-bottom: 8px;
}
.cp-hint {
  position: relative; font-size: 13px; color: var(--dc-text-mid);
  padding: 8px 12px 8px 24px; border-radius: 8px;
  cursor: pointer; transition: background .15s, color .15s;
}
/* 左侧引导箭头：hover 时变主色并轻微右移，提示「可点击」 */
.cp-hint::before {
  content: '›'; position: absolute; left: 11px; top: 50%; transform: translateY(-50%);
  font-size: 15px; line-height: 1; color: var(--dc-text-weak);
  transition: color .15s, transform .15s;
}
.cp-hint:hover { background: var(--dc-primary-wash); color: var(--dc-primary); }
.cp-hint:hover::before { color: var(--dc-primary); transform: translateY(-50%) translateX(2px); }
</style>
