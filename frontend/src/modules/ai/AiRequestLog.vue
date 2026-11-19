<template>
  <el-dialog
    :model-value="modelValue"
    :title="$t('ai.reqLog')"
    width="min(960px, 94vw)"
    top="6vh"
    append-to-body
    class="ai-req-dialog"
    @update:model-value="$emit('update:modelValue', $event)"
    @open="onOpen"
  >
    <div class="req-wrap">
      <template v-if="cur">
        <div class="req-head">
          <span class="req-chip is-model">{{ cur.model }}</span>
          <span class="req-chip">{{ cur.kind }}</span>
          <span class="req-chip">{{ shortTs(cur.ts) }}</span>
          <span class="req-chip">{{ $t('ai.reqChars', { n: cur.chars }) }}</span>
          <span class="req-chip">{{ $t('ai.reqTokens', { n: cur.tokens }) }}</span>
          <span class="req-chip">{{ $t('ai.reqMsgs', { n: cur.messages ? cur.messages.length : 0 }) }}</span>
        </div>

        <!-- 构成：先回答「token 花在哪」，再看原文 -->
        <div class="req-break">
          <div v-for="p in parts" :key="p.key" class="req-part" :class="'is-' + p.key">
            <span class="req-part-name">{{ p.label }}</span>
            <span class="req-part-bar"><i :style="{ width: p.pct + '%' }" /></span>
            <span class="req-part-num">{{ $t('ai.reqChars', { n: p.chars }) }} · {{ p.pct }}%</span>
          </div>
        </div>

        <el-tabs v-model="tab" class="req-tabs">
          <el-tab-pane :label="$t('ai.reqSystem')" name="system">
            <pre class="req-pre">{{ cur.system }}</pre>
          </el-tab-pane>
          <el-tab-pane :label="$t('ai.reqMessages')" name="messages">
            <div class="req-msgs">
              <div v-for="m in historyMsgs" :key="m.idx" class="req-msg">
                <button type="button" class="req-msg-head" @click="toggle(m.idx)">
                  <span class="req-role" :class="'is-' + m.role">{{ m.role }}</span>
                  <span class="req-msg-clip">{{ clip(m.content) }}</span>
                  <span class="req-msg-num">{{ $t('ai.reqChars', { n: m.chars }) }}</span>
                  <el-icon class="req-msg-caret">
                    <ArrowDown v-if="expanded[m.idx]" />
                    <ArrowRight v-else />
                  </el-icon>
                </button>
                <pre v-if="expanded[m.idx]" class="req-pre is-msg">{{ m.content }}</pre>
              </div>
              <div v-if="!historyMsgs.length" class="req-empty">{{ $t('ai.reqNoHistory') }}</div>
              <div v-if="askMsg" class="req-ask">
                <div class="req-ask-label">{{ $t('ai.reqAskPart') }}</div>
                <pre class="req-pre is-ask">{{ askMsg.content }}</pre>
              </div>
            </div>
          </el-tab-pane>
          <el-tab-pane v-if="cur.tools" :label="$t('ai.reqTools')" name="tools">
            <pre class="req-pre">{{ pretty(cur.tools) }}</pre>
          </el-tab-pane>
        </el-tabs>
      </template>
      <div v-else-if="loading" class="req-empty">{{ $t('ai.reqLoading') }}</div>
      <div v-else class="req-empty">{{ $t('ai.reqEmpty') }}</div>
    </div>

    <template #footer>
      <span class="req-note">{{ $t('ai.reqNote') }}</span>
      <el-button size="small" text :disabled="!cur" @click="copyAll">{{ $t('ai.reqCopy') }}</el-button>
      <el-button size="small" text @click="doClear">{{ $t('ai.reqClear') }}</el-button>
      <el-button size="small" @click="$emit('update:modelValue', false)">{{ $t('common.close') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
/**
 * 「这次到底发给模型什么了」—— 把最近一次真正发出去的请求摊开看。
 *
 * 两层信息，顺序别反：
 * 1. **构成**：先看 1,700 字符里哪一段是人格、哪一段是表结构、哪一段是历史/提问 ——
 *    用户问「token 花在哪」，只给一坨原文是回答不了的；
 * 2. **原文**：系统提示词 / 逐条消息（可展开）/ 工具定义，逐字可对。
 *
 * 入口是**每条提问操作条上的第一个图标**（编辑/复制那一排的最左），所以只看
 * 「最新一次发送」——不再摆一个左列列表（用户：左侧菜单不用显示），一屏把内容摊开。
 *
 * 只读；后端落在本机库（dbmind.db 的 ai_requests，只留最近 200 条），
 * 所以**重启之后仍能回看**。正文是完整库结构 —— 底栏写明「注意别外传」，
 * 「清空记录」就是它的出口。
 */
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { ArrowDown, ArrowRight } from '@element-plus/icons-vue'
import { listAiRequests, getAiRequest, clearAiRequests } from '../../api'
import { t } from '../../utils/i18n'

const props = defineProps({ modelValue: { type: Boolean, default: false } })
const emit = defineEmits(['update:modelValue'])

const SCHEMA_START = '<<<SCHEMA_DATA_BEGIN>>>'
const SCHEMA_END = '<<<SCHEMA_DATA_END>>>'

const cur = ref(null)
const tab = ref('system')
const loading = ref(false)
const expanded = ref({})

/** 时间显示：库里存的是 `YYYY-MM-DD HH:MM:SS`，截掉年份（近 200 条必然都是近期的） */
const shortTs = (ts) => String(ts || '').slice(5)

const pretty = (v) => {
  if (v === null || v === undefined) return ''
  try {
    return JSON.stringify(v, null, 2)
  } catch {
    return String(v)
  }
}

const textOf = (content) => {
  if (typeof content === 'string') return content
  if (Array.isArray(content)) {
    return content.map((p) => (typeof p === 'string' ? p : (p && p.text) || '')).join('')
  }
  return pretty(content)
}

const clip = (content) => {
  const s = textOf(content).replace(/\s+/g, ' ').trim()
  return s.length > 96 ? s.slice(0, 96) + '…' : s
}

/** messages 里第一条是 system（后端把 system 也拼在 messages 里），页面另有页签展示它 */
const chatMsgs = computed(() => {
  const list = Array.isArray(cur.value?.messages) ? cur.value.messages : []
  return list
    .filter((m) => m && m.role !== 'system')
    .map((m) => {
      const text = textOf(m.content)
      return { role: m.role || '?', content: text, chars: text.length }
    })
})
const historyMsgs = computed(() =>
  chatMsgs.value.slice(0, -1).map((m, idx) => ({ ...m, idx }))
)
const askMsg = computed(() => chatMsgs.value[chatMsgs.value.length - 1] || null)

/** 构成：人格 / 表结构 / 历史 / 本次提问 —— 按字符占比画条 */
const parts = computed(() => {
  const c = cur.value
  if (!c) return []
  const sys = String(c.system || '')
  const start = sys.indexOf(SCHEMA_START)
  const end = sys.indexOf(SCHEMA_END)
  const persona = start >= 0 ? sys.slice(0, start) : sys
  const schema = start >= 0 ? sys.slice(start, end >= 0 ? end + SCHEMA_END.length : undefined) : ''
  const historyChars = historyMsgs.value.reduce((n, m) => n + m.chars, 0)
  const askChars = askMsg.value ? askMsg.value.chars : 0
  const rows = [
    { key: 'persona', label: t('ai.reqPersona'), chars: persona.length },
    { key: 'schema', label: t('ai.reqSchemaPart'), chars: schema.length },
    { key: 'history', label: t('ai.reqHistoryPart', { n: historyMsgs.value.length }), chars: historyChars },
    { key: 'ask', label: t('ai.reqAskPart'), chars: askChars }
  ]
  const total = rows.reduce((n, r) => n + r.chars, 0) || 1
  return rows.map((r) => ({ ...r, pct: Math.round((r.chars * 100) / total) }))
})

/** 打开时取最新一次发送（列表第一行）再取全文 */
const loadLatest = async () => {
  loading.value = true
  try {
    const res = await listAiRequests()
    const items = (res && res.items) || []
    if (!items.length) {
      cur.value = null
      return
    }
    const one = await getAiRequest(items[0].id)
    if (one && one.success && one.item) {
      cur.value = one.item
      tab.value = 'system'
      expanded.value = {}
    } else {
      cur.value = null
      ElMessage.warning((one && one.message) || t('ai.reqGone'))
    }
  } catch (e) {
    cur.value = null
    ElMessage.error(e?.message || t('ai.requestFailed'))
  } finally {
    loading.value = false
  }
}

const toggle = (idx) => { expanded.value = { ...expanded.value, [idx]: !expanded.value[idx] } }

const copyAll = async () => {
  if (!cur.value) return
  const text = JSON.stringify({ system: cur.value.system, messages: cur.value.messages, tools: cur.value.tools }, null, 2)
  try {
    await navigator.clipboard.writeText(text)
    ElMessage.success(t('ai.reqCopied'))
  } catch {
    ElMessage.warning(t('ai.reqCopyFail'))
  }
}

const doClear = async () => {
  try {
    await clearAiRequests()
    cur.value = null
    ElMessage.success(t('ai.reqCleared'))
  } catch (e) {
    ElMessage.error(e?.message || t('ai.requestFailed'))
  }
}

const onOpen = () => { tab.value = 'system'; expanded.value = {}; loadLatest() }
watch(() => props.modelValue, (v) => { if (v) loadLatest() })
</script>

<style scoped>
.req-wrap { display: flex; flex-direction: column; min-height: 380px; max-height: 68vh; }

.req-head { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
.req-chip { font-size: 11.5px; padding: 1px 8px; border-radius: 999px; background: var(--dc-bg-soft); color: var(--dc-text-mid); }
.req-chip.is-model { color: var(--dc-primary); background: var(--dc-primary-wash); }

/* 构成条：一眼看出「这段时间花在哪」 */
.req-break {
  display: flex; flex-direction: column; gap: 3px;
  margin: 8px 0 2px; padding: 8px 10px;
  border: 1px solid var(--dc-border); border-radius: 8px; background: var(--dc-bg-soft);
}
.req-part { display: flex; align-items: center; gap: 8px; font-size: 12px; }
.req-part-name { flex: none; width: 116px; color: var(--dc-text-mid); }
.req-part-bar { flex: 1; height: 6px; border-radius: 4px; background: var(--dc-bg-hover); overflow: hidden; }
.req-part-bar > i { display: block; height: 100%; border-radius: 4px; background: var(--dc-text-dim); }
.req-part.is-schema .req-part-bar > i { background: var(--dc-primary); }
.req-part.is-ask .req-part-bar > i { background: var(--dc-success, #67c23a); }
.req-part-num { flex: none; width: 150px; text-align: right; color: var(--dc-text-dim); font-variant-numeric: tabular-nums; }

.req-tabs { flex: 1; min-height: 0; display: flex; flex-direction: column; margin-top: 4px; }
.req-tabs :deep(.el-tabs__content) { flex: 1; min-height: 0; }
.req-tabs :deep(.el-tab-pane) { height: 100%; }
.req-pre {
  margin: 0; height: 100%; max-height: 46vh; overflow: auto;
  padding: 10px 12px; border-radius: 8px;
  background: var(--dc-code-bg, var(--dc-bg-soft)); border: 1px solid var(--dc-border);
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 12px; line-height: 1.6;
  white-space: pre-wrap; word-break: break-word; color: var(--dc-text);
}
.req-pre.is-msg, .req-pre.is-ask { max-height: 26vh; margin-top: 4px; }
.req-msgs { max-height: 46vh; overflow: auto; }

.req-msg { border: 1px solid var(--dc-border); border-radius: 8px; margin-bottom: 6px; overflow: hidden; }
.req-msg-head {
  display: flex; align-items: center; gap: 8px; width: 100%; text-align: left;
  padding: 7px 10px; border: none; background: var(--dc-bg-soft); cursor: pointer; color: var(--dc-text);
}
.req-msg-head:hover { background: var(--dc-bg-hover); }
.req-role { flex: none; font-size: 11px; padding: 0 6px; border-radius: 999px; background: var(--dc-bg-card); color: var(--dc-text-mid); }
.req-role.is-user { color: var(--dc-primary); background: var(--dc-primary-wash); }
.req-role.is-tool { color: #b8860b; background: rgba(209, 154, 0, .14); }
.req-msg-clip { flex: 1; min-width: 0; font-size: 12px; color: var(--dc-text-mid); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.req-msg-num { flex: none; font-size: 11.5px; color: var(--dc-text-dim); }
.req-msg-caret { flex: none; color: var(--dc-text-dim); }

.req-ask { margin-top: 10px; padding-top: 8px; border-top: 1px dashed var(--dc-border); }
.req-ask-label { font-size: 12px; color: var(--dc-text-dim); margin-bottom: 4px; }

.req-empty { padding: 24px; text-align: center; color: var(--dc-text-dim); font-size: 13px; }
.req-note { float: left; font-size: 11.5px; color: var(--dc-text-dim); }
</style>
