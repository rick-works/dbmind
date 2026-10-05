<template>
  <el-dialog v-model="visible" width="440px" :close-on-click-modal="false" append-to-body class="main-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Edit /></el-icon></span>
        <span>{{ $t('rnt.title') }}</span>
      </div>
    </template>
    <el-form label-width="72px" size="default" @submit.prevent="submit">
      <el-form-item :label="$t('rnt.old')">
        <el-input :model-value="ctx?.oldName || ''" disabled />
      </el-form-item>
      <el-form-item :label="$t('rnt.new')" required>
        <el-input v-model="newName" maxlength="64" :placeholder="$t('rnt.placeholder')"
                  @keyup.enter="submit" />
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="visible = false">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" :loading="loading" @click="submit">{{ $t('rn.submit') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
// 重命名表弹窗（从 MainView 拆出）：提交后回调父级同步已打开页签并刷新树分类节点。
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { Edit } from '@element-plus/icons-vue'
import { tableAction } from '../../api'
import { errMsg } from '../../utils/errMsg'
import { t } from '../../utils/i18n'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  // { connId, db, oldName }：打开时由父快照右键目标表
  ctx: { type: Object, default: null }
})
const emit = defineEmits(['update:modelValue', 'renamed'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const newName = ref('')
const loading = ref(false)

// 打开弹窗时清空新表名
watch(() => props.modelValue, (open) => {
  if (!open) return
  newName.value = ''
})

const submit = async () => {
  const c = props.ctx
  if (!c || !c.connId || !c.oldName) return
  const val = newName.value.trim()
  if (!val) { ElMessage.warning(t('rnt.needName')); return }
  if (!/^[A-Za-z0-9_$-]+$/.test(val)) { ElMessage.error(t('rnt.invalidName')); return }
  if (val === c.oldName) { ElMessage.info(t('rnt.unchanged')); return }
  loading.value = true
  try {
    const res = await tableAction(c.connId, c.db, c.oldName, 'rename', val)
    if (!res.success) throw new Error(res.message || t('rn.failed'))
    ElMessage.success(res.message || t('rnt.done', { from: c.oldName, to: val }))
    visible.value = false
    emit('renamed', { connId: c.connId, db: c.db, oldName: c.oldName, newName: val })
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
</style>
