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
          <el-select v-model="form.charset" style="width: 100%" clearable filterable :placeholder="$t('ndb.followServer')">
            <el-option v-for="c in charsets" :key="c" :label="c" :value="c" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('ndb.collation')">
          <el-select v-model="form.collation" style="width: 100%" clearable filterable :placeholder="$t('ndb.followServer')">
            <el-option v-for="c in collations" :key="c" :label="c" :value="c" />
          </el-select>
        </el-form-item>
      </template>
      <template v-if="connType === 'POSTGRESQL'">
        <el-form-item :label="$t('ndb.owner')">
          <el-input v-model="form.owner" :placeholder="$t('ndb.ownerTip')" clearable />
        </el-form-item>
        <el-form-item :label="$t('ndb.encoding')">
          <el-select v-model="form.encoding" style="width: 100%" filterable>
            <el-option v-for="e in pgEncodings" :key="e[0]" :label="e[0] + '（' + e[1] + '）'" :value="e[0]" />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('ndb.connLimit')">
          <el-input-number v-model="form.connLimit" :min="0" :step="10" controls-position="right"
            style="width: 100%" :placeholder="$t('ndb.connLimitTip')" />
        </el-form-item>
      </template>
      <template v-if="connType === 'SQLSERVER'">
        <!-- 只放必要参数：排序规则。恢复模式/文件大小走服务器默认（createDatabaseSql 仍支持，表单不再放出） -->
        <el-form-item :label="$t('ndb.collation')">
          <el-select v-model="form.collation" style="width: 100%" clearable filterable
            :placeholder="$t('ndb.followServer')">
            <el-option v-for="c in mssqlCollations" :key="c[0]" :label="c[0] + '（' + c[1] + '）'" :value="c[0]" />
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

const blankForm = () => ({ name: '', charset: 'utf8mb4', collation: 'utf8mb4_general_ci', encoding: 'UTF8', owner: '', connLimit: null })
const form = ref(blankForm())
const loading = ref(false)

// ---- 下拉清单：放全常用项，select 支持 filterable 输入过滤 ----
const charsets = ['utf8mb4', 'utf8', 'latin1', 'gbk', 'gb18030', 'big5', 'ascii', 'binary', 'utf16', 'utf32']
// 0900 系是 MySQL 8 独有；MariaDB 用 uca1400（10.10+）
const collations = computed(() => props.connType === 'MARIADB' ? [
  'utf8mb4_general_ci', 'utf8mb4_unicode_ci', 'utf8mb4_uca1400_ai_ci', 'utf8mb4_bin',
  'utf8_general_ci', 'utf8_unicode_ci', 'utf8_bin',
  'latin1_swedish_ci', 'latin1_bin', 'gbk_chinese_ci', 'gbk_bin', 'big5_chinese_ci', 'binary'
] : [
  'utf8mb4_0900_ai_ci', 'utf8mb4_0900_bin', 'utf8mb4_general_ci', 'utf8mb4_unicode_ci', 'utf8mb4_bin',
  'utf8_general_ci', 'utf8_unicode_ci', 'utf8_bin',
  'latin1_swedish_ci', 'latin1_bin', 'gbk_chinese_ci', 'gbk_bin', 'gb18030_chinese_ci', 'big5_chinese_ci', 'binary', 'ascii_bin'
])
const pgEncodings = [
  ['UTF8', 'Unicode'], ['GBK', '中文简体'], ['GB18030', '中文简体国标'], ['BIG5', '中文繁体'],
  ['EUC_CN', '中文简体 EUC'], ['EUC_JP', '日语'], ['SJIS', '日语 Shift-JIS'],
  ['LATIN1', '西欧'], ['LATIN9', '西欧 + 欧元'], ['WIN1252', '西欧 Windows'], ['SQL_ASCII', '不转码']
]
const mssqlCollations = [
  ['Chinese_PRC_CI_AS', '中文简体，不区分大小写'], ['Chinese_PRC_CS_AS', '中文简体，区分大小写'],
  ['Chinese_PRC_90_CI_AS', '中文简体 90，不区分大小写'], ['Chinese_Taiwan_Stroke_CI_AS', '中文繁体-台湾'],
  ['Latin1_General_CI_AS', '拉丁，不区分大小写'], ['Latin1_General_CS_AS', '拉丁，区分大小写'],
  ['Latin1_General_100_CI_AS', '拉丁 100，不区分大小写'], ['SQL_Latin1_General_CP1_CI_AS', 'SQL 兼容（代码页 1252）'],
  ['Japanese_CI_AS', '日语'], ['Korean_Wansung_CI_AS', '韩语'], ['Thai_CI_AS', '泰语'],
  ['Vietnamese_100_CI_AS', '越南语'], ['Cyrillic_General_CI_AS', '西里尔（俄语）'],
  ['Arabic_CI_AS', '阿拉伯语'], ['Hebrew_CI_AS', '希伯来语'],
  ['German_PhoneBook_CI_AS', '德语电话簿'], ['French_CI_AS', '法语'], ['Spanish_Modern_Sort_CI_AS', '西班牙语']
]

// 打开弹窗时重置表单：MySQL/MariaDB 给字符集/排序规则默认值；SQL Server 的排序规则
// 不能沿用 MySQL 的默认值（utf8mb4_general_ci 不在选项里会原样显示成已选），清空走「跟随服务器默认」
watch(() => props.modelValue, (open) => {
  if (!open) return
  form.value = blankForm()
  if (props.connType === 'SQLSERVER') form.value.collation = ''
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
/* el-input-number 默认居中，与其它输入框对齐（element-plus 内置规则优先级高，需 !important） */
:deep(.el-input-number .el-input__inner) { text-align: left !important; }
.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}
</style>
