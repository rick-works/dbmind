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
          <el-select v-model="form.charset" style="width: 100%" clearable :placeholder="$t('ndb.followServer')">
            <el-option label="utf8mb4" value="utf8mb4" />
            <el-option label="utf8" value="utf8" />
            <el-option label="latin1" value="latin1" />
            <el-option label="gbk" value="gbk" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('ndb.collation')">
          <el-select v-model="form.collation" style="width: 100%" clearable :placeholder="$t('ndb.followServer')">
            <el-option label="utf8mb4_general_ci" value="utf8mb4_general_ci" />
            <el-option label="utf8mb4_unicode_ci" value="utf8mb4_unicode_ci" />
            <el-option label="utf8mb4_0900_ai_ci" value="utf8mb4_0900_ai_ci" />
            <el-option label="utf8_general_ci" value="utf8_general_ci" />
            <el-option label="latin1_swedish_ci" value="latin1_swedish_ci" />
          </el-select>
        </el-form-item>
      </template>
      <template v-if="connType === 'POSTGRESQL'">
        <el-form-item :label="$t('ndb.owner')">
          <el-input v-model="form.owner" :placeholder="$t('ndb.ownerTip')" clearable />
        </el-form-item>
        <el-form-item :label="$t('ndb.encoding')">
          <el-select v-model="form.encoding" style="width: 100%">
            <el-option label="UTF8" value="UTF8" />
            <el-option label="GBK" value="GBK" />
            <el-option label="LATIN1" value="LATIN1" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('ndb.connLimit')">
          <el-input-number v-model="form.connLimit" :min="0" :step="10" controls-position="right"
            style="width: 100%" :placeholder="$t('ndb.connLimitTip')" />
          <div class="form-tip">{{ $t('ndb.connLimitTip') }}</div>
        </el-form-item>
      </template>
      <template v-if="connType === 'SQLSERVER'">
        <el-form-item :label="$t('ndb.collation')">
          <el-select v-model="form.collation" style="width: 100%" clearable
            :placeholder="$t('ndb.followServer')">
            <el-option label="Chinese_PRC_CI_AS（中文简体，不区分大小写）" value="Chinese_PRC_CI_AS" />
            <el-option label="Chinese_PRC_CS_AS（中文简体，区分大小写）" value="Chinese_PRC_CS_AS" />
            <el-option label="Latin1_General_CI_AS（拉丁，不区分大小写）" value="Latin1_General_CI_AS" />
            <el-option label="SQL_Latin1_General_CP1_CI_AS（SQL 兼容）" value="SQL_Latin1_General_CP1_CI_AS" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('ndb.recovery')">
          <el-select v-model="form.recovery" style="width: 100%" clearable :placeholder="$t('ndb.followServer')">
            <el-option :label="$t('ndb.recoveryFull')" value="FULL" />
            <el-option :label="$t('ndb.recoverySimple')" value="SIMPLE" />
            <el-option :label="$t('ndb.recoveryBulk')" value="BULK_LOGGED" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('ndb.initialSize')">
          <el-input-number v-model="form.initialSize" :min="0" :step="16" controls-position="right"
            style="width: 100%" />
          <div class="form-tip">{{ $t('ndb.initialSizeTip') }}</div>
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

const blankForm = () => ({ name: '', charset: 'utf8mb4', collation: 'utf8mb4_general_ci', encoding: 'UTF8', owner: '', connLimit: null, recovery: '', initialSize: null })
const form = ref(blankForm())
const loading = ref(false)

// 打开弹窗时重置表单（MySQL 的字符集/排序规则给默认值，其余留空 = 跟随服务器默认）
watch(() => props.modelValue, (open) => {
  if (!open) return
  form.value = blankForm()
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
  // createDatabaseSql 可能返回语句数组（如 SQL Server 的 建库 → 恢复模式 → 文件大小），
  // 按序执行：中途失败立即停 —— 库已建出来时提示用户参数没设全，别把失败吞成成功
  const statements = Array.isArray(sql) ? sql : [sql]
  loading.value = true
  try {
    for (const stmt of statements) {
      // 后端执行失败时仍返回 HTTP 200 + success:false，必须显式检查
      const res = await executeSql(props.connId, stmt, null, null, null, null, true)
      if (res && res.success === false) {
        ElMessage.error(t('ndb.failed', { detail: (res.message || t('common.unknownError')) }))
        return
      }
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
.form-tip { font-size: 12px; line-height: 1.5; color: var(--dc-text-2); margin-top: 2px; }
.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}
</style>
