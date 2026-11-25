<template>
  <el-dialog :model-value="modelValue" width="min(680px, 94vw)" append-to-body
             class="ai-send-preview" :close-on-click-modal="true"
             @update:model-value="$emit('update:modelValue', $event)">
    <template #header>
      <div class="asp-head">
        <span class="asp-head-ic"><el-icon :size="15"><View /></el-icon></span>
        <span>{{ $t('ai.sendPreview.title') }}</span>
      </div>
    </template>

    <div class="asp-body">
      <div class="asp-notice">
        <el-icon :size="15"><InfoFilled /></el-icon>
        <span>{{ $t('ai.sendPreview.notice') }}</span>
      </div>

      <div class="asp-block">
        <div class="asp-block-title">{{ $t('ai.sendPreview.question') }}</div>
        <pre class="asp-question">{{ question || '—' }}</pre>
      </div>

      <div class="asp-grid">
        <div class="asp-block">
          <div class="asp-block-title">{{ $t('ai.sendPreview.context') }}</div>
          <div class="asp-kv"><span>{{ $t('ai.sendPreview.conn') }}</span><b>{{ connectionName || '—' }}</b></div>
          <div class="asp-kv"><span>{{ $t('ai.sendPreview.db') }}</span><b>{{ database || '—' }}</b></div>
          <div class="asp-kv"><span>{{ $t('ai.sendPreview.tables') }}</span>
            <b>{{ tables.length ? tables.join('、') : $t('ai.sendPreview.allTables') }}</b>
          </div>
        </div>
        <div class="asp-block">
          <div class="asp-block-title">{{ $t('ai.sendPreview.skills') }}</div>
          <div v-if="skills.length" class="asp-skills">
            <span v-for="s in skills" :key="s.id || s" class="asp-skill">{{ s.name || s }}</span>
          </div>
          <div v-else class="asp-dim">{{ $t('ai.sendPreview.noSkills') }}</div>
        </div>
      </div>

      <div class="asp-block">
        <div class="asp-block-title">{{ $t('ai.sendPreview.maskTitle') }}</div>
        <div class="asp-mask-state" :class="maskOn ? 'on' : 'off'">
          <el-icon :size="14"><Lock v-if="maskOn" /><Unlock v-else /></el-icon>
          <span>{{ maskOn ? $t('ai.sendPreview.maskOn') : $t('ai.sendPreview.maskOff') }}</span>
        </div>
        <div v-if="maskOn && maskRules.length" class="asp-rules">
          <div v-for="(r, i) in maskRules" :key="i" class="asp-rule">
            <span class="asp-rule-scope">{{ [r.connectionId, r.database, r.table].filter(Boolean).join(' / ') || $t('ai.sendPreview.anyScope') }}</span>
            <span class="asp-rule-col">{{ r.column }}</span>
            <span class="asp-rule-method">{{ r.method || 'md5' }}</span>
          </div>
        </div>
        <div v-else class="asp-dim">{{ $t('ai.sendPreview.noRules') }}</div>
      </div>
    </div>

    <template #footer>
      <el-button size="small" @click="$emit('update:modelValue', false)">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" size="small" @click="confirm">
        {{ confirmText || $t('ai.sendPreview.send') }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed } from 'vue'
import { View, InfoFilled, Lock, Unlock } from '@element-plus/icons-vue'
import { peekMaskConfig } from '../../utils/masking'

const props = defineProps({
  modelValue: Boolean,
  question: { type: String, default: '' },
  connectionName: { type: String, default: '' },
  database: { type: String, default: '' },
  tables: { type: Array, default: () => [] },
  skills: { type: Array, default: () => [] },
  confirmText: String
})
const emit = defineEmits(['update:modelValue', 'confirm'])

const cfg = computed(() => peekMaskConfig())
const maskOn = computed(() => cfg.value?.llm === true)
const maskRules = computed(() => (cfg.value?.rules || []).filter(r => !r.channel || r.channel === 'llm' || r.channel === 'ai'))

const confirm = () => {
  emit('update:modelValue', false)
  emit('confirm')
}
</script>

<style scoped>
.asp-head { display: flex; align-items: center; gap: 8px; font-size: 15px; font-weight: 700; }
.asp-head-ic { display: inline-flex; color: var(--dc-primary); }
.asp-body { font-size: 13px; }
.asp-notice {
  display: flex; align-items: flex-start; gap: 8px; padding: 10px 12px;
  border-radius: 8px; margin-bottom: 14px; line-height: 1.6;
  background: var(--dc-bg-soft); color: var(--dc-text-dim);
}
.asp-notice .el-icon { flex: none; color: var(--dc-primary); margin-top: 2px; }
.asp-block { margin-bottom: 14px; }
.asp-block-title { font-weight: 600; color: var(--dc-text-strong); margin-bottom: 7px; }
.asp-question {
  margin: 0; padding: 10px 12px; white-space: pre-wrap; word-break: break-word;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-bg-hover);
  border-radius: 8px; font: inherit; line-height: 1.7; color: var(--dc-text);
  max-height: 140px; overflow: auto;
}
.asp-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
.asp-kv { display: flex; gap: 8px; margin-bottom: 5px; line-height: 1.6; }
.asp-kv > span { color: var(--dc-text-dim); flex: none; min-width: 52px; }
.asp-kv > b { color: var(--dc-text); font-weight: 500; word-break: break-all; }
.asp-skills { display: flex; flex-wrap: wrap; gap: 6px; }
.asp-skill {
  font-size: 12px; padding: 2px 8px; border-radius: 10px;
  color: var(--dc-primary); background: var(--dc-primary-wash);
}
.asp-dim { color: var(--dc-text-dim); }
.asp-mask-state { display: inline-flex; align-items: center; gap: 6px; padding: 3px 9px; border-radius: 10px; font-size: 12px; }
.asp-mask-state.on { color: var(--dc-success); background: var(--dc-success-wash); }
.asp-mask-state.off { color: var(--dc-warning); background: var(--dc-warning-wash); }
.asp-rules { max-height: 130px; overflow: auto; border: 1px solid var(--dc-bg-hover); border-radius: 8px; margin-top: 8px; }
.asp-rule {
  display: flex; align-items: center; gap: 10px; padding: 5px 10px;
  font-size: 12px; border-bottom: 1px solid var(--dc-bg-hover);
}
.asp-rule:last-child { border-bottom: none; }
.asp-rule-scope { flex: 1; color: var(--dc-text-dim); word-break: break-all; }
.asp-rule-col { color: var(--dc-text-strong); font-weight: 600; }
.asp-rule-method { color: var(--dc-primary); }
</style>
