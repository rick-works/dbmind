<script setup>
/**
 * 存入知识库：把任意一段内容（对话回答 / 数据字典 / 洞察报告 / 表结构说明）收进知识库。
 *
 * <p>两件事一起做：选目标（新建还是追加）+ 决定要不要先规范化。
 * 默认开着「AI 规范化」，因为从对话里摘出来的内容多半带寒暄、编号错乱、
 * 还有 Markdown 装饰，直接入库会拖累召回质量；但也允许关掉，原文照存。
 */
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { MagicStick, Plus, FolderAdd, DocumentAdd } from '@element-plus/icons-vue'
import { aiKbPolish, aiKbAutoPlan, kbList, kbCreate, kbImportDoc } from '../../api'
import { t } from '../../utils/i18n'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  /** 内容：要存进去的正文 */
  content: { type: String, default: '' },
  /** 默认标题（可改）：通常是资料名 */
  defaultTitle: { type: String, default: '' },
  /** 来源说明（写进 source 字段，方便日后追溯这段内容从哪来） */
  source: { type: String, default: '' }
})
const emit = defineEmits(['update:modelValue', 'saved'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const kbOptions = ref([])
const mode = ref('new')
const targetKbId = ref('')
const title = ref('')
const form = ref({ name: '', description: '' })
const optimize = ref(true)
const busy = ref(false)
const naming = ref(false)
/** 规范化结果：{ originalChars, polishedChars, keepRate, factsMissingCount } */
const polishInfo = ref(null)

const targetKb = computed(() => kbOptions.value.find(k => k.id === targetKbId.value) || null)

const loadKbOptions = async () => {
  try {
    const d = await kbList()
    kbOptions.value = (d && d.items) || []
  } catch (e) {
    kbOptions.value = []
  }
}

/** 让 AI 依据内容起个库名：多数时候比让用户现想一个快 */
const runNaming = async () => {
  if (!props.content.trim() || naming.value) return
  naming.value = true
  try {
    const res = await aiKbAutoPlan({ text: props.content, source: props.source || props.defaultTitle })
    if (res?.success === false) throw new Error(res.message || t('s2k.autoNameFailed'))
    if (res.name) form.value.name = res.name
    if (res.description) form.value.description = res.description
  } catch (e) {
    // 起名失败不影响入库：用户自己填一个就行
    ElMessage.warning(t('s2k.aiNameFailed', { detail: (e?.message || e) }))
  }
  naming.value = false
}

// 打开时重置并准备：默认追加到已有库（若没有库则只能新建）
watch(visible, async (v) => {
  if (!v) return
  title.value = props.defaultTitle || t('s2k.untitled')
  form.value = { name: props.defaultTitle || '', description: '' }
  polishInfo.value = null
  optimize.value = true
  busy.value = false
  await loadKbOptions()
  if (kbOptions.value.length) {
    mode.value = 'append'
    targetKbId.value = kbOptions.value[0].id
  } else {
    mode.value = 'new'
    targetKbId.value = ''
    runNaming()
  }
})

const submit = async () => {
  const text = (props.content || '').trim()
  if (!text) { ElMessage.warning(t('s2k.noContent')); return }
  if (mode.value === 'new' && !form.value.name.trim()) { ElMessage.warning(t('kbx.needName')); return }
  if (mode.value === 'append' && !targetKbId.value) { ElMessage.warning(t('s2k.needTargetKb')); return }

  busy.value = true
  try {
    let finalText = text
    // 先规范化再入库：对话输出里的「好的，以下是…」这类起手式对检索只有坏处
    if (optimize.value) {
      const r = await aiKbPolish({ text, source: props.source || title.value })
      if (r?.success !== false && r?.text) {
        finalText = r.text
        polishInfo.value = r
      }
    }

    let kbId = mode.value === 'append' ? targetKbId.value : ''
    if (mode.value === 'new') {
      const created = await kbCreate({
        name: form.value.name.trim(),
        description: form.value.description.trim()
      })
      if (!created?.success) throw new Error(created?.message || t('s2k.createFailed'))
      // 兼容多种响应形状：后端实际返回 { info: { id }, kbId }，曾只读 item?.id 误报「未返回 id」（真机踩过）
    kbId = created.item?.id || created.kbId || created.info?.id
      if (!kbId) throw new Error(t('kbx.noKbId'))
    }

    const imp = await kbImportDoc({
      kbId,
      title: title.value.trim() || t('s2k.untitled'),
      source: props.source || title.value.trim() || t('s2k.externalContent'),
      text: finalText
    })
    if (!imp?.success) throw new Error(imp?.message || t('kbx.importFailed'))

    const kbName = mode.value === 'append'
      ? (targetKb.value?.name || t('s2k.kbFallback'))
      : form.value.name.trim()
    ElMessage.success(t('s2k.savedInto', { name: kbName, n: (imp.chunkCount || 0) }))
    emit('saved', { kbId, kbName, chunkCount: imp.chunkCount || 0 })
    visible.value = false
  } catch (e) {
    ElMessage.error(e?.message || t('s2k.saveFailed'))
  }
  busy.value = false
}
</script>

<template>
  <el-dialog v-model="visible" :title="$t('s2k.dialogTitle')" width="620px" top="8vh"
             :close-on-click-modal="false" :append-to-body="true">
    <div class="s2k">
      <!-- 存的是什么：先让用户确认一遍，避免存错东西 -->
      <div class="s2k-src">
        <el-icon><DocumentAdd /></el-icon>
        <div class="s2k-src-body">
          <div class="s2k-src-title">{{ defaultTitle || $t('s2k.untitled') }}</div>
          <div class="s2k-src-meta">
            {{ $t('kbx.chars', { n: content.length }) }}
            <template v-if="source">{{ $t('s2k.sourceSuffix', { name: source }) }}</template>
          </div>
        </div>
      </div>

      <div class="s2k-field">
        <label>{{ $t('s2k.titleLabel') }}</label>
        <el-input v-model="title" maxlength="80" :placeholder="$t('s2k.titlePlaceholder')" />
      </div>

      <div class="s2k-sub"><span>{{ $t('kbx.whereToStore') }}</span></div>
      <div class="s2k-target">
        <div class="s2k-opt" :class="{ active: mode === 'new' }" @click="mode = 'new'">
          <el-icon class="s2k-opt-ic"><Plus /></el-icon>
          <div class="s2k-opt-body">
            <div class="s2k-opt-name">{{ $t('kbx.newKb') }}</div>
            <div class="s2k-opt-desc">{{ $t('s2k.newKbDesc') }}</div>
          </div>
        </div>
        <div class="s2k-opt" :class="{ active: mode === 'append', disabled: !kbOptions.length }"
             @click="kbOptions.length && (mode = 'append')">
          <el-icon class="s2k-opt-ic"><FolderAdd /></el-icon>
          <div class="s2k-opt-body">
            <div class="s2k-opt-name">{{ $t('s2k.appendKb') }}</div>
            <div class="s2k-opt-desc">
              {{ kbOptions.length ? $t('s2k.appendDescYes') : $t('kbx.noKbAvailable') }}
            </div>
          </div>
        </div>
      </div>

      <template v-if="mode === 'append'">
        <div class="s2k-field">
          <label>{{ $t('kbx.pickKb') }}</label>
          <el-select v-model="targetKbId" :placeholder="$t('s2k.pickPlaceholder')" style="width:100%">
            <el-option v-for="k in kbOptions" :key="k.id" :value="k.id"
                       :label="k.chunkCount != null ? $t('s2k.kbOption', { name: k.name, n: k.chunkCount }) : k.name" />
          </el-select>
        </div>
      </template>
      <template v-else>
        <div class="s2k-grid2">
          <div class="s2k-field">
            <label>
              {{ $t('kbx.kbName') }}
              <a class="s2k-ai" @click="runNaming">
                <el-icon :class="{ 'is-loading': naming }"><MagicStick /></el-icon>
                {{ naming ? $t('s2k.naming') : $t('s2k.aiName') }}
              </a>
            </label>
            <el-input v-model="form.name" maxlength="60" :placeholder="$t('kbx.kbName')" />
          </div>
          <div class="s2k-field">
            <label>{{ $t('udv.fComment') }}</label>
            <el-input v-model="form.description" maxlength="120" :placeholder="$t('s2k.descPlaceholder')" />
          </div>
        </div>
      </template>

      <div class="s2k-opt-row">
        <el-checkbox v-model="optimize">{{ $t('s2k.optimizeLabel') }}</el-checkbox>
        <span class="s2k-hint">{{ $t('s2k.optimizeHint') }}</span>
      </div>
      <div v-if="polishInfo" class="s2k-polished">
        {{ $t('s2k.polished', { a: polishInfo.originalChars, b: polishInfo.polishedChars }) }}
        {{ $t('s2k.keepRate', { n: Math.round((polishInfo.keepRate ?? 1) * 100) }) }}
      </div>
    </div>

    <template #footer>
      <el-button @click="visible = false">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" :loading="busy" @click="submit">
        {{ busy ? $t('s2k.busy') : $t('s2k.save') }}
      </el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.s2k-src {
  display: flex; align-items: center; gap: 9px; padding: 10px 12px;
  border-radius: 10px; background: var(--dc-bg-soft); border: 1px solid var(--dc-border);
}
.s2k-src .el-icon { color: var(--dc-text-dim); flex-shrink: 0; }
.s2k-src-body { min-width: 0; }
.s2k-src-title {
  font-size: 13px; font-weight: 600; color: var(--dc-text-strong);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.s2k-src-meta { margin-top: 2px; font-size: 12px; color: var(--dc-text-dim); }

.s2k-sub {
  display: flex; align-items: center; gap: 8px;
  margin: 16px 0 8px; font-size: 13px; font-weight: 600; color: var(--dc-text-mid);
}
.s2k-sub::after { content: ''; flex: 1; height: 1px; background: var(--dc-border); }

.s2k-target { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
.s2k-opt {
  display: flex; align-items: flex-start; gap: 9px; padding: 10px 11px; cursor: pointer;
  border: 1px solid var(--dc-border); border-radius: 10px; background: var(--dc-bg-card);
  transition: border-color .15s ease;
}
.s2k-opt:hover { border-color: var(--dc-border-strong); }
.s2k-opt.active { border-color: var(--dc-primary); }
.s2k-opt.disabled { opacity: .55; cursor: not-allowed; }
.s2k-opt-ic { color: var(--dc-text-mid); margin-top: 1px; flex-shrink: 0; }
.s2k-opt.active .s2k-opt-ic { color: var(--dc-primary); }
.s2k-opt-body { min-width: 0; }
.s2k-opt-name { font-size: 13px; font-weight: 600; color: var(--dc-text-strong); }
.s2k-opt-desc { margin-top: 3px; font-size: 12px; color: var(--dc-text-dim); line-height: 1.7; }

.s2k-grid2 { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
.s2k-field { display: flex; flex-direction: column; gap: 6px; margin-top: 12px; }
.s2k-grid2 .s2k-field { margin-top: 12px; }
.s2k-field > label {
  display: flex; align-items: center; gap: 8px;
  font-size: 13px; color: var(--dc-text-mid);
}
.s2k-ai {
  display: inline-flex; align-items: center; gap: 3px; margin-left: auto;
  font-size: 12px; color: var(--dc-primary); cursor: pointer;
}
.s2k-ai:hover { text-decoration: underline; }

.s2k-opt-row { display: flex; align-items: baseline; gap: 10px; margin-top: 16px; flex-wrap: wrap; }
.s2k-hint { font-size: 12px; color: var(--dc-text-dim); line-height: 1.7; }
.s2k-polished {
  margin-top: 8px; padding: 7px 10px; border-radius: 8px; font-size: 11.5px;
  color: var(--dc-text-mid); background: rgba(245, 158, 11, .1);
  border: 1px solid rgba(245, 158, 11, .3);
}

.s2k :deep(.el-input__wrapper) { background: var(--dc-bg-soft); box-shadow: none; }
.s2k :deep(.el-input__wrapper.is-focus) { box-shadow: 0 0 0 1px var(--dc-primary) inset; }
</style>
