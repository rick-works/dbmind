<template>
  <!-- 新建分组弹窗 -->
  <el-dialog v-model="newOpen" width="420px" :close-on-click-modal="false" append-to-body class="main-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><FolderAdd /></el-icon></span>
        <span>{{ $t('fd.newTitle') }}</span>
      </div>
    </template>
    <el-form label-width="0" size="default" @submit.prevent="doCreate">
      <el-form-item required>
        <el-input v-model="newName" @keyup.enter="doCreate" />
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="newOpen = false">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" :loading="creating" @click="doCreate">{{ $t('ndb.create') }}</el-button>
    </template>
  </el-dialog>

  <!-- 重命名分组弹窗 -->
  <el-dialog v-model="renameOpen" width="420px" :close-on-click-modal="false" append-to-body class="main-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><EditPen /></el-icon></span>
        <span>{{ $t('fd.renameTitle') }}</span>
      </div>
    </template>
    <el-form label-width="0" size="default" @submit.prevent="doRename">
      <el-form-item required>
        <el-input v-model="renameVal" @keyup.enter="doRename" />
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="renameOpen = false">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" :loading="renaming" @click="doRename">{{ $t('rn.submit') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
// 新建 / 重命名分组弹窗（从 MainView 拆出）：分组 = 连接的环境分组（environment）
// + localStorage 持久化的纯分组。提交成功后事件通知父级刷新树与连接列表。
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { FolderAdd, EditPen } from '@element-plus/icons-vue'
import { saveConnection } from '../../api'
import { PREDEF_ENVS } from '../../utils/envs'
import { t } from '../../utils/i18n'

const props = defineProps({
  newOpen: { type: Boolean, default: false },
  renameOpen: { type: Boolean, default: false },
  // 打开时所在分组（新建时作为默认名 / 重命名时的旧名）
  env: { type: String, default: '' },
  // 连接列表（重命名分组需把该分组下所有连接的环境改写到新分组）
  connections: { type: Array, default: () => [] }
})
const emit = defineEmits(['update:newOpen', 'update:renameOpen', 'folder-created', 'folder-renamed'])

const newOpen = computed({
  get: () => props.newOpen,
  set: (v) => emit('update:newOpen', v)
})
const renameOpen = computed({
  get: () => props.renameOpen,
  set: (v) => emit('update:renameOpen', v)
})

const newName = ref('')
const oldName = ref('')
const renameVal = ref('')
const creating = ref(false)
const renaming = ref(false)

watch(() => props.newOpen, (open) => {
  if (!open) return
  newName.value = props.env || ''
  creating.value = false
})
watch(() => props.renameOpen, (open) => {
  if (!open) return
  oldName.value = props.env || ''
  renameVal.value = props.env || ''
  renaming.value = false
})

const readFolders = () => {
  try {
    return JSON.parse(localStorage.getItem('dbmind_folders') || '[]')
  } catch { return [] }
}

const validateName = (name) => {
  if (!name) { ElMessage.warning(t('fd.needName')); return false }
  if (name.length > 20) { ElMessage.warning(t('mv.groupNameTooLong', { n: 20 })); return false }
  if (PREDEF_ENVS.includes(name)) { ElMessage.warning(t('fd.predefName')); return false }
  return true
}

const doCreate = async () => {
  const name = newName.value.trim()
  if (!validateName(name)) return
  creating.value = true
  try {
    const folders = readFolders()
    if (!folders.includes(name)) folders.push(name)
    localStorage.setItem('dbmind_folders', JSON.stringify(folders))
    newOpen.value = false
    emit('folder-created', name)
  } catch (e) {
    ElMessage.error(t('fd.createFailed', { detail: (e?.message || e) }))
  } finally {
    creating.value = false
  }
}

const doRename = async () => {
  const env = oldName.value
  const val = renameVal.value.trim()
  if (!validateName(val)) return
  if (val === env) { renameOpen.value = false; return }
  renaming.value = true
  try {
    // 把该分组下所有连接的环境改写到新分组
    const targets = props.connections.filter(c => (c.environment || '') === env)
    for (const c of targets) {
      await saveConnection({ ...c, environment: val })
    }
    // 同步 localStorage 纯分组
    const folders = readFolders()
    const idx = folders.indexOf(env)
    if (idx >= 0) { folders[idx] = val; localStorage.setItem('dbmind_folders', JSON.stringify(folders)) }
    renameOpen.value = false
    emit('folder-renamed', { old: env, new: val })
  } catch (e) {
    ElMessage.error(t('mv.renameFailed', { detail: (e?.message || e?.toString?.() || t('common.unknownError')) }))
  } finally {
    renaming.value = false
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
