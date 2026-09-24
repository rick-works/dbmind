<template>
  <el-dialog v-model="visible" width="600px" :close-on-click-modal="false" append-to-body class="main-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Edit /></el-icon></span>
        <span>{{ $t('rn.title', { kind: label }) }}</span>
      </div>
    </template>
    <el-form label-width="108px" size="default" @submit.prevent="submit">
      <el-form-item :label="$t('rn.old', { kind: label })">
        <el-input :model-value="ctx?.oldName || ''" disabled />
      </el-form-item>
      <el-form-item :label="$t('rn.new', { kind: label })" required>
        <el-input v-model="newName" maxlength="64" :placeholder="$t('rn.placeholder')" @keyup.enter="submit" />
      </el-form-item>
    </el-form>

    <!-- 将执行的语句：由后端按引擎能力生成（原生改名 / 无原生语法时重建），执行前先让用户看到 -->
    <div v-if="sql" class="sql-preview">
      <div class="sql-head">
        <span class="sql-title">{{ $t('rn.sqlTitle') }}</span>
        <el-tag v-if="native" type="success" size="small" effect="plain">{{ $t('rn.nativeTag') }}</el-tag>
        <el-tag v-else type="warning" size="small" effect="plain">{{ $t('rn.recreateTag') }}</el-tag>
      </div>
      <pre class="sql-body">{{ sql }}</pre>
      <div v-if="native" class="sql-tip">{{ $t('rn.nativeTip') }}</div>
      <div v-else class="sql-tip warn">
        {{ $t('rn.recreateTip') }}
      </div>
    </div>

    <template #footer>
      <el-button @click="visible = false">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" :loading="loading" :disabled="!canSubmit" @click="submit">{{ $t('rn.submit') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
// 对象重命名弹窗（存储过程 / 函数 / 视图 / 触发器 / 事件，从 MainView 拆出）：
// 输入新名后自动向后端要一次 previewOnly 预览，把真正要执行的语句（以及是否需要 DROP 重建）展示给用户，
// 确认后再执行；成功后回调父级同步已打开页签并刷新树分类节点。
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { Edit } from '@element-plus/icons-vue'
import { renameObject } from '../../api'
import { errMsg } from '../../utils/errMsg'
import { t } from '../../utils/i18n'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  // { connId, db, type, oldName }：打开时由父快照右键目标对象
  ctx: { type: Object, default: null }
})
const emit = defineEmits(['update:modelValue', 'renamed'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

// 映射写进 computed 里，而不是模块级常量：常量只求值一次，切换语言后标签不会更新
const label = computed(() => ({
  procedure: t('mv.catProc'), function: t('mv.catFunc'), view: t('mv.catView'),
  trigger: t('mv.catTrigger'), event: t('mv.catEvent')
}[props.ctx?.type] || t('mv.catObject')))

const newName = ref('')
const sql = ref('')
const native = ref(true)
const loading = ref(false)
let timer = null

const valid = (v) => !!v && /^[A-Za-z0-9_$-]+$/.test(v)
const canSubmit = computed(() => {
  const v = newName.value.trim()
  return valid(v) && v !== props.ctx?.oldName
})

// 预览将要执行的语句（native=false 表示该引擎无原生改名语法，会按定义重建）
const preview = async () => {
  const c = props.ctx
  sql.value = ''
  if (!c || !canSubmit.value) return
  try {
    const res = await renameObject(c.connId, {
      database: c.db, type: c.type, name: c.oldName, newName: newName.value.trim(), previewOnly: true
    })
    if (res.success) {
      sql.value = res.sql || ''
      native.value = res.native !== false
    }
  } catch (e) { /* 预览失败不打扰，真正执行时会给错误 */ }
}

watch(newName, () => {
  clearTimeout(timer)
  timer = setTimeout(preview, 400)
})

// 打开弹窗时清空输入与预览
watch(() => props.modelValue, (open) => {
  if (!open) return
  newName.value = ''
  sql.value = ''
  native.value = true
})

const submit = async () => {
  const c = props.ctx
  if (!c || !c.connId || !c.oldName) return
  const val = newName.value.trim()
  if (!val) { ElMessage.warning(t('rn.needName')); return }
  if (!valid(val)) { ElMessage.error(t('rn.invalidName')); return }
  if (val === c.oldName) { ElMessage.info(t('rn.unchanged')); return }
  loading.value = true
  try {
    const res = await renameObject(c.connId, {
      database: c.db, type: c.type, name: c.oldName, newName: val, previewOnly: false
    })
    if (!res.success) throw new Error(res.message || t('rn.failed'))
    ElMessage.success(res.message || t('rn.done', { kind: label.value, from: c.oldName, to: val }))
    visible.value = false
    emit('renamed', { connId: c.connId, db: c.db, type: c.type, oldName: c.oldName, newName: val })
  } catch (e) {
    ElMessage.error(t('rn.failedDetail', { detail: errMsg(e) }))
  } finally {
    loading.value = false
  }
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
/* 标签（含必填星号）不换行：「新存储过程」这类 5 字标签要占满一行 */
:deep(.el-form-item__label) { white-space: nowrap; }
.sql-preview { margin: 0 0 4px; border: 1px solid var(--dc-border-soft); border-radius: 8px; background: var(--dc-bg-soft); padding: 10px 12px; }
.sql-head { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; }
.sql-title { font-size: 13px; color: var(--dc-text-dim); }
.sql-body {
  margin: 0; max-height: 200px; overflow: auto; font-size: 13px; line-height: 1.6;
  font-family: Consolas, Monaco, 'Courier New', monospace; white-space: pre-wrap; word-break: break-all;
  color: var(--dc-text);
}
.sql-tip { margin-top: 8px; font-size: 13px; color: var(--dc-text-dim); }
.sql-tip.warn { color: #e6a23c; }
</style>
