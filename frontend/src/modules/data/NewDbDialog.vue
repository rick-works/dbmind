<template>
  <el-dialog v-model="visible" width="480px" :close-on-click-modal="false" append-to-body class="main-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><FolderAdd /></el-icon></span>
        <span>{{ $t('ndb.title') }}</span>
      </div>
    </template>
    <el-form :model="form" label-width="100px" label-position="left" size="default">
      <el-form-item :label="$t('ndb.name')" required>
        <el-input v-model="form.name" />
      </el-form-item>
      <template v-if="connType === 'MYSQL' || connType === 'MARIADB'">
        <el-form-item :label="$t('ndb.charset')">
          <el-select v-model="form.charset" style="width: 100%">
            <el-option label="utf8mb4" value="utf8mb4" />
            <el-option label="utf8" value="utf8" />
            <el-option label="latin1" value="latin1" />
            <el-option label="gbk" value="gbk" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('ndb.collation')">
          <el-select v-model="form.collation" style="width: 100%">
            <el-option label="utf8mb4_general_ci" value="utf8mb4_general_ci" />
            <el-option label="utf8mb4_unicode_ci" value="utf8mb4_unicode_ci" />
            <el-option label="utf8_general_ci" value="utf8_general_ci" />
            <el-option label="latin1_swedish_ci" value="latin1_swedish_ci" />
          </el-select>
        </el-form-item>
      </template>
      <template v-if="connType === 'POSTGRESQL'">
        <el-form-item :label="$t('ndb.encoding')">
          <el-select v-model="form.encoding" style="width: 100%">
            <el-option label="UTF8" value="UTF8" />
            <el-option label="GBK" value="GBK" />
            <el-option label="LATIN1" value="LATIN1" />
          </el-select>
        </el-form-item>
      </template>
    </el-form>
    <template #footer>
      <el-button @click="visible = false">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" :loading="loading" @click="submit">{{ $t('ndb.create') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
// 新建数据库弹窗（从 MainView 拆出）：建库 SQL 由各数据库类型模块生成，成功后回调父级刷新连接节点。
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { FolderAdd } from '@element-plus/icons-vue'
import { executeSql } from '../../api'
import { byType } from '../../types'
import { errMsg } from '../../utils/errMsg'
import { t } from '../../utils/i18n'

const props = defineProps({
  modelValue: { type: Boolean, default: false },
  connId: { type: [String, Number], default: null },
  connType: { type: String, default: '' }
})
const emit = defineEmits(['update:modelValue', 'created'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const form = ref({ name: '', charset: 'utf8mb4', collation: 'utf8mb4_general_ci', encoding: 'UTF8' })
const loading = ref(false)

// 打开弹窗时重置表单
watch(() => props.modelValue, (open) => {
  if (!open) return
  form.value = { name: '', charset: 'utf8mb4', collation: 'utf8mb4_general_ci', encoding: 'UTF8' }
})

const submit = async () => {
  const name = form.value.name.trim()
  if (!name) {
    ElMessage.warning(t('ndb.needName'))
    return
  }
  const ty = props.connType
  const sql = byType(ty).createDatabaseSql(name, form.value)
  if (!sql) {
    ElMessage.warning(t('ndb.unsupported'))
    return
  }
  loading.value = true
  try {
    // 后端执行失败时仍返回 HTTP 200 + success:false，必须显式检查
    const res = await executeSql(props.connId, sql, null, null, null, null, true)
    if (res && res.success === false) {
      ElMessage.error(t('ndb.failed', { detail: (res.message || t('common.unknownError')) }))
      return
    }
    ElMessage.success(t('ndb.created', { name }))
    visible.value = false
    emit('created', props.connId)
  } catch (e) {
    ElMessage.error(t('ndb.failed', { detail: errMsg(e) }))
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
