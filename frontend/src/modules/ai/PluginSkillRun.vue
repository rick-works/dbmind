<!--
  运行一个 skill 插件（插件中心 / AI 面板共用）。

  为什么单独抽出来：同一个技能在两个地方能点开（设置 → 插件中心、AI 面板的技能入口），
  表单与调用逻辑只该有一份 —— 否则「插件中心能跑、面板里跑挂」这种分裂迟早出现。

  表单完全由 manifest.inputs 生成（连控件类型都不用猜：目前只有多行文本）。
  结果用等宽 + 保留换行展示：技能输出是给数据库人看的清单与 SQL，渲染成 HTML 反而更难读。
-->
<template>
  <el-dialog v-model="open" :title="item?.name || ''" width="min(760px, 94vw)" append-to-body class="plg-run-dialog">
    <div class="plg-run-form">
      <div class="plg-row">
        <label>{{ $t('plg.conn') }}</label>
        <el-select v-model="form.connId" size="small" filterable clearable :placeholder="$t('plg.connPh')"
                   style="width:100%" @change="onConnChange">
          <el-option v-for="c in conns" :key="c.id" :label="c.name || c.host" :value="c.id" />
        </el-select>
      </div>
      <div class="plg-row">
        <label>{{ $t('plg.database') }}</label>
        <el-select v-model="form.database" size="small" filterable clearable :placeholder="$t('plg.databasePh')"
                   :loading="dbLoading" style="width:100%">
          <el-option v-for="d in dbs" :key="d" :label="d" :value="d" />
        </el-select>
      </div>
      <div v-if="item?.needsContext" class="plg-row">
        <label>{{ $t('plg.withContext') }}</label>
        <el-switch v-model="form.withContext" size="small" />
        <span class="plg-hint">{{ $t('plg.withContextTip') }}</span>
      </div>
      <div v-for="inp in item?.inputs || []" :key="inp.key" class="plg-row">
        <label>{{ labelOf(inp) }}<span v-if="inp.required" class="plg-req">*</span></label>
        <el-input v-model="form.inputs[inp.key]" type="textarea" :rows="inp.rows || 3"
                  :placeholder="phOf(inp)" size="small" />
      </div>
    </div>
    <div v-if="result" class="plg-out-wrap">
      <div class="plg-out-head">
        <span>{{ $t('plg.result') }}</span>
        <button class="plg-copy" type="button" @click="copyResult">{{ $t('common.copy') }}</button>
      </div>
      <pre class="plg-out">{{ result }}</pre>
    </div>
    <template #footer>
      <div class="plg-run-foot">
        <span class="plg-tip">{{ running ? $t('plg.running') : (error || '') }}</span>
        <el-button size="small" @click="open = false">{{ $t('common.close') }}</el-button>
        <el-button type="primary" size="small" :loading="running" @click="doRun">{{ $t('plg.run') }}</el-button>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, reactive, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { t, locale } from '../../utils/i18n'
import { listConnections, listDatabases, runPluginSkill } from '../../api'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  /** 插件清单项（/api/plugins 返回的那一条） */
  item: { type: Object, default: null },
  /** 预设上下文（AI 面板传当前选中的连接/库，插件中心传空） */
  connId: { type: String, default: '' },
  database: { type: String, default: '' }
})
const emit = defineEmits(['update:modelValue'])

const open = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const conns = ref([])
const dbs = ref([])
const dbLoading = ref(false)
const running = ref(false)
const result = ref('')
const error = ref('')
const form = reactive({ connId: '', database: '', withContext: true, inputs: {} })

/** 清单里的文案支持「字符串」或「{zh, en}」两种写法，这里统一挑当前语言 */
const pickText = (v) => {
  if (!v) return ''
  if (typeof v === 'string') return v
  const lang = String(locale.value || 'zh-CN')
  return v[lang] || v[lang.slice(0, 2)] || v.zh || v.en || ''
}
const labelOf = (inp) => pickText(inp?.label) || inp?.key || ''
const phOf = (inp) => pickText(inp?.placeholder)

const onConnChange = async (id) => {
  dbs.value = []
  form.database = ''
  if (!id) return
  dbLoading.value = true
  try {
    const list = await listDatabases(id)
    dbs.value = Array.isArray(list) ? list : (list?.databases || [])
    if (dbs.value.length === 1) form.database = dbs.value[0]
  } catch { /* 拿不到库清单不影响纯文本类技能 */ } finally {
    dbLoading.value = false
  }
}

// 每次打开：重置结果、按 manifest 建空表单、继承外部上下文
watch(() => props.modelValue, async (v) => {
  if (!v) return
  result.value = ''
  error.value = ''
  form.connId = props.connId || ''
  form.database = props.database || ''
  form.withContext = !!props.item?.needsContext
  form.inputs = {}
  for (const inp of props.item?.inputs || []) form.inputs[inp.key] = ''
  if (!conns.value.length) {
    try {
      const list = await listConnections()
      conns.value = Array.isArray(list) ? list : (list?.connections || [])
    } catch { conns.value = [] }
  }
  // 外部带了连接（AI 面板的当前上下文）就别再拉一次库清单把它清掉
  if (form.connId) await onConnChange(form.connId)
  else if (form.database) form.database = ''
})

const copyResult = async () => {
  const text = result.value
  if (!text) return
  try {
    if (navigator.clipboard?.writeText) await navigator.clipboard.writeText(text)
    else throw new Error('no clipboard')
    ElMessage.success(t('common.copied'))
  } catch {
    // 非安全上下文降级：临时 textarea + execCommand
    try {
      const ta = document.createElement('textarea')
      ta.value = text
      ta.style.position = 'fixed'
      ta.style.opacity = '0'
      document.body.appendChild(ta)
      ta.select()
      const ok = document.execCommand('copy')
      document.body.removeChild(ta)
      if (ok) ElMessage.success(t('common.copied'))
      else ElMessage.error(t('sqlq.copyFailed'))
    } catch {
      ElMessage.error(t('sqlq.copyFailed'))
    }
  }
}

const doRun = async () => {
  if (!props.item) return
  // 必填项前端先拦一道：后端也会 400，但这里能直接指到是哪个空着
  const missing = (props.item.inputs || []).filter((i) => i.required && !String(form.inputs[i.key] || '').trim())
  if (missing.length) {
    error.value = t('plg.needInput', { names: missing.map((i) => labelOf(i)).join('、') })
    return
  }
  running.value = true
  error.value = ''
  result.value = ''
  try {
    const res = await runPluginSkill(props.item.id, {
      inputs: form.inputs,
      connectionId: form.connId || undefined,
      database: form.database || undefined,
      withContext: form.withContext
    })
    result.value = res.text || ''
  } catch (e) {
    error.value = e?.response?.data?.message || String(e)
  } finally {
    running.value = false
  }
}
</script>

<style scoped>
.plg-run-form { display: flex; flex-direction: column; gap: 10px; }
.plg-row { display: flex; align-items: center; gap: 8px; }
.plg-row > label { width: 96px; flex: none; font-size: 12px; color: var(--dc-text-weak); text-align: right; }
.plg-row :deep(.el-textarea) { flex: 1; }
.plg-req { color: #f56c6c; margin-left: 2px; }
.plg-hint { font-size: 11px; color: var(--dc-text-weak); }
.plg-out-wrap { margin-top: 12px; }
.plg-out-head { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--dc-text-weak); margin-bottom: 6px; }
.plg-copy { margin-left: auto; height: 22px; padding: 0 8px; font-size: 12px; cursor: pointer;
  border: 1px solid var(--dc-border); border-radius: 6px; background: var(--dc-surface); color: var(--dc-text); }
.plg-copy:hover { border-color: var(--dc-primary); color: var(--dc-primary); }
.plg-out { margin: 0; max-height: 340px; overflow: auto; padding: 10px 12px; font-size: 12px; line-height: 1.6;
  white-space: pre-wrap; word-break: break-word; background: var(--dc-bg-soft, rgba(0, 0, 0, .03));
  border: 1px solid var(--dc-border); border-radius: 6px; }
.plg-run-foot { display: flex; align-items: center; gap: 8px; width: 100%; }
.plg-tip { flex: 1; text-align: left; font-size: 12px; color: #f56c6c; word-break: break-all; }
</style>
