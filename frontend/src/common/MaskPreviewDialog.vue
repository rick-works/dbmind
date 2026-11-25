<template>
  <el-dialog :model-value="modelValue" width="min(720px, 94vw)" append-to-body
             class="mask-preview-dialog" :close-on-click-modal="true"
             @update:model-value="$emit('update:modelValue', $event)">
    <template #header>
      <div class="mp-head">
        <span class="mp-head-ic"><el-icon :size="15"><Lock /></el-icon></span>
        <span>{{ title || $t('mask.preview.title') }}</span>
      </div>
    </template>

    <div class="mp-body">
      <div class="mp-desc">{{ $t('mask.preview.desc', { channel: channelLabel }) }}</div>

      <div v-if="!hitColumns.length" class="mp-none">
        <el-icon :size="20"><SuccessFilled /></el-icon>
        <span>{{ $t('mask.preview.none') }}</span>
      </div>

      <div v-else class="mp-table-wrap">
        <table class="mp-table">
          <thead>
            <tr>
              <th>{{ $t('mask.preview.col') }}</th>
              <th>{{ $t('mask.preview.method') }}</th>
              <th>{{ $t('mask.preview.sample') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="c in hitColumns" :key="c">
              <td class="mp-col">{{ c }}</td>
              <td><span class="mp-method">{{ methodLabel[c] }}</span></td>
              <td class="mp-sample">
                <template v-for="(s, i) in samples[c]" :key="i">
                  <span class="mp-plain">{{ s.before }}</span>
                  <span class="mp-arrow">→</span>
                  <span class="mp-masked">{{ s.after }}</span>
                  <br v-if="i < samples[c].length - 1" />
                </template>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div class="mp-foot-note">{{ $t('mask.preview.note') }}</div>
    </div>

    <template #footer>
      <el-button size="small" @click="$emit('update:modelValue', false)">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" size="small" @click="confirm">
        {{ confirmText || $t('mask.preview.continue') }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed } from 'vue'
import { Lock, SuccessFilled } from '@element-plus/icons-vue'
import { t } from '../utils/i18n'
import { peekMaskConfig, methodFor, maskTextSync } from '../utils/masking'

const props = defineProps({
  modelValue: Boolean,
  channel: { type: String, default: 'export' },
  title: String,
  confirmText: String,
  connectionId: { type: String, default: '' },
  database: { type: String, default: '' },
  table: { type: String, default: '' },
  columns: { type: Array, default: () => [] },
  rows: { type: Array, default: () => [] },
  previewRows: { type: Number, default: 3 }
})
const emit = defineEmits(['update:modelValue', 'confirm'])

const cfg = computed(() => peekMaskConfig())
const channelLabel = computed(() => t('mask.channel.' + (props.channel === 'export' ? 'export' : props.channel === 'mcp' ? 'mcp' : props.channel === 'grid' ? 'grid' : 'llm')))

const methods = computed(() => {
  const out = {}
  for (const c of props.columns || []) {
    const m = methodFor(props.channel, {
      connectionId: props.connectionId,
      database: props.database,
      table: props.table,
      column: c
    }, cfg.value)
    if (m) out[c] = m
  }
  return out
})
const hitColumns = computed(() => Object.keys(methods.value))
const methodLabel = computed(() => {
  const out = {}
  for (const c of hitColumns.value) out[c] = t('settings.mask.method.' + methods.value[c])
  return out
})

/** 样例只取前几行；AES 是异步的，预览阶段统一用掩码占位，导出时由后端真算。 */
const samples = computed(() => {
  const out = {}
  const rows = (props.rows || []).slice(0, Math.max(1, props.previewRows))
  for (const c of hitColumns.value) {
    out[c] = rows.map(r => {
      const v = r?.[c]
      const before = v === null || v === undefined ? 'NULL' : String(v)
      const method = methods.value[c]
      const after = method === 'aes' ? '***' : maskTextSync(before, method)
      return { before, after }
    })
  }
  return out
})

const confirm = () => {
  emit('update:modelValue', false)
  emit('confirm')
}
</script>

<style scoped>
.mp-head { display: flex; align-items: center; gap: 8px; font-size: 15px; font-weight: 700; }
.mp-head-ic { display: inline-flex; color: var(--dc-primary); }
.mp-body { font-size: 13px; }
.mp-desc { color: var(--dc-text-dim); line-height: 1.7; margin-bottom: 12px; }
.mp-none {
  display: flex; align-items: center; gap: 8px; padding: 22px 12px;
  color: var(--dc-text-dim); justify-content: center;
  border: 1px dashed var(--dc-bg-hover); border-radius: 8px;
}
.mp-table-wrap { max-height: 46vh; overflow: auto; border: 1px solid var(--dc-bg-hover); border-radius: 8px; }
.mp-table { width: 100%; border-collapse: collapse; }
.mp-table th, .mp-table td {
  text-align: left; padding: 8px 10px; font-size: 12.5px;
  border-bottom: 1px solid var(--dc-bg-hover); vertical-align: top;
}
.mp-table th { position: sticky; top: 0; background: var(--dc-bg-soft); color: var(--dc-text-dim); font-weight: 600; }
.mp-col { color: var(--dc-text-strong); font-weight: 600; white-space: nowrap; }
.mp-method { color: var(--dc-warning); }
.mp-sample { color: var(--dc-text-dim); line-height: 1.8; word-break: break-all; }
.mp-plain { color: var(--dc-text-dim); }
.mp-arrow { margin: 0 6px; color: var(--dc-primary); }
.mp-masked { color: var(--dc-primary); font-family: var(--dc-mono, monospace); }
.mp-foot-note { margin-top: 10px; color: var(--dc-text-dim); font-size: 12px; line-height: 1.6; }
</style>
