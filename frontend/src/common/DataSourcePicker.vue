<template>
  <el-dialog :model-value="modelValue" width="760px" append-to-body
             :close-on-click-modal="true" :show-close="true"
             @update:model-value="$emit('update:modelValue', $event)"
             class="dsp-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><CirclePlus /></el-icon></span>
        <span>{{ $t('cd.newTitle') }}</span>
      </div>
    </template>
    <div class="ds-picker">
      <el-input v-model="keyword" clearable
                :prefix-icon="Search" class="ds-search" />

      <!-- 最近使用：连接成功过的才进这个列表，最多 5 条；
           点一条直接打开它的编辑表单。id 失效的（连接被删）自动跳过。 -->
      <div v-if="recents.length" class="ds-recent">
        <div class="ds-recent-title">{{ $t('dsp.recent') }}</div>
        <div class="ds-recent-list">
          <div v-for="c in recents" :key="c.id" class="ds-recent-item"
               :title="c.note || c.name" @click="openRecent(c)">
            <DbLogo :type="c.type" :size="18" />
            <span class="ds-recent-name">{{ c.name }}</span>
            <span v-if="c.note" class="ds-recent-note">{{ c.note }}</span>
            <span v-else class="ds-recent-note">{{ c.host }}{{ c.port ? ':' + c.port : '' }}</span>
          </div>
        </div>
      </div>

      <div v-for="group in filteredGroups" :key="group.name" class="ds-group">
        <div class="ds-group-title">
          <el-icon :size="15"><component :is="group.icon" /></el-icon>
          {{ group.name }}
          <span class="ds-group-count">{{ group.items.length }}</span>
        </div>
        <div class="ds-grid">
          <div v-for="t in group.items" :key="t.code" class="ds-card" @click="pick(t.code)">
            <DbLogo :type="t.code" :size="42" />
            <div class="ds-card-info">
              <div class="ds-card-name">{{ t.label }}</div>
              <div class="ds-card-meta">
                <template v-if="t.defaultPort">{{ $t('cd.port') }} {{ t.defaultPort }}</template>
                <template v-else>{{ $t('dsp.localFile') }}</template>
              </div>
            </div>
            <div v-if="!t.builtin && !isReady(t.code)" class="ds-card-badge" :title="$t('dsp.driverTip')">{{ $t('dsp.downloadDriver') }}</div>
            <div v-else-if="!t.builtin && isReady(t.code)" class="ds-card-badge ds-card-ready" :title="$t('dsp.driverDownloaded')">{{ $t('settings.driver.tagReady') }}</div>
          </div>
        </div>
      </div>

      <el-empty v-if="!filteredGroups.length" :description="$t('settings.driver.empty')" :image-size="90" />
    </div>

    <!-- 导入 / 导出 / 批量管理连接 -->
    <template #footer>
      <div class="ds-footer">
        <span class="ds-footer-tip">{{ $t('dsp.pwdTip') }}</span>
        <el-button @click="openManager">{{ $t('dsp.manage') }}</el-button>
        <el-button :icon="Upload" @click="openImport">{{ $t('dsp.importConns') }}</el-button>
        <el-button :icon="Download" :disabled="!connCount" @click="openExport">{{ $t('dsp.exportConns') }}</el-button>
      </div>
    </template>
  </el-dialog>

  <!-- 连接管理：勾选若干条后批量测试 / 批量导出 / 批量删除 -->
  <el-dialog v-model="exportVisible" :title="$t('dsp.manage')" width="560px" append-to-body>
    <div class="exp-tip">{{ $t('dsp.exportTip') }}</div>
    <el-checkbox v-model="exportAll" @change="toggleAllExport">{{ $t('common.selectAll') }}</el-checkbox>
    <div class="exp-list">
      <el-checkbox-group v-model="exportIds">
        <el-checkbox v-for="c in exportable" :key="c.id" :label="c.id">
          {{ c.name }}
          <span class="exp-meta">{{ c.type }}<template v-if="c.host"> · {{ c.host }}</template><template v-if="c.port">:{{ c.port }}</template></span>
          <span v-if="c.note" class="exp-note">{{ c.note }}</span>
        </el-checkbox>
      </el-checkbox-group>
    </div>
    <div v-if="batchLog" class="exp-log">{{ batchLog }}</div>
    <template #footer>
      <el-button @click="exportVisible = false">{{ $t('common.close') }}</el-button>
      <el-button :disabled="!exportIds.length" :loading="batchTesting" @click="testSelected">{{ $t('dsp.testSelected') }}</el-button>
      <el-button :disabled="!exportIds.length" @click="doExport">{{ $t('dsp.exportSelected', { n: exportIds.length }) }}</el-button>
      <el-button type="danger" plain :disabled="!exportIds.length" :loading="batchDeleting" @click="deleteSelected">{{ $t('dsp.deleteSelected') }}</el-button>
    </template>
  </el-dialog>

  <!-- 导入：选文件 → 确认 → 逐条落库（口令留空，导入后补填） -->
  <input ref="fileRef" type="file" accept=".json,application/json" style="display:none" @change="onFilePicked" />
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { saveBlobAs } from '../utils/useExportTask'
import { Search, Coin, Document, Box, CirclePlus, Upload, Download } from '@element-plus/icons-vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { getDriverTypes, getDriverStatus, listConnections, saveConnection, deleteConnection, testConnectionById } from '../api'
import DbLogo from './DbLogo.vue'
import { rankRecentConnections } from '../utils/recentConnections'
import { t } from '../utils/i18n'

const props = defineProps({
  modelValue: Boolean
})
const emit = defineEmits(['update:modelValue', 'selected', 'imported', 'open-conn'])

const types = ref([])
const status = ref({})
const keyword = ref('')

onMounted(async () => {
  if (!types.value.length) {
    try {
      types.value = await getDriverTypes()
    } catch (e) { /* 兼容旧后端 */ }
  }
  try {
    status.value = await getDriverStatus()
  } catch (e) { /* 兼容旧后端 */ }
})

const isReady = (code) => status.value[code]?.ready === true

const groups = computed(() => [
  { name: t('settings.driver.catRelational'), icon: Coin, items: types.value.filter(t => t.category === 'RELATIONAL') },
  { name: t('settings.driver.catFile'), icon: Document, items: types.value.filter(t => t.category === 'RELATIONAL_FILE') },
  { name: t('settings.driver.catNoSql'), icon: Box, items: types.value.filter(t => t.category === 'NOSQL') }
].filter(g => g.items.length))

const filteredGroups = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  if (!kw) return groups.value
  return groups.value.map(g => ({
    ...g,
    items: g.items.filter(t =>
      t.label.toLowerCase().includes(kw) || t.code.toLowerCase().includes(kw))
  })).filter(g => g.items.length)
})

const pick = (code) => {
  emit('selected', code)
  emit('update:modelValue', false)
}

// ========== 导入 / 导出连接配置 ==========
// 三条口径，写清楚免得误解：
// 1. 格式 `{ format:'dbmind-connections', version:1, exportedAt, connections:[…] }`，
//    同时**也认裸数组**（别的工具/手写的 JSON 都能导进来）。
// 2. **口令一律不导出**：内核的连接详情从来不下发口令（`ConnectionConfig::password` 标了
//    skip_serializing），所以这里不做"加密导出"这种做不了的承诺 —— 文件里明确写 note 说明，
//    导入后由用户逐条补填。
// 3. 导入走现有的 `saveConnection`，**逐条落库**：某条失败不连累其余，最后如实报成功/失败条数。
const FILE_FORMAT = 'dbmind-connections'
const EXPORT_SKIP_KEYS = ['id', 'hasPassword', 'hasSshPassword', 'createTime', 'updateTime', 'readOnly']
const allConns = ref([])
const connCount = computed(() => allConns.value.length)
const exportVisible = ref(false)
const exportAll = ref(true)
const exportIds = ref([])
const exportable = computed(() => allConns.value)
const fileRef = ref(null)

const loadConnsForTransfer = async () => {
  try {
    allConns.value = (await listConnections()) || []
  } catch (e) {
    allConns.value = []
  }
}
onMounted(loadConnsForTransfer)

const sanitizeForExport = (c) => {
  const out = {}
  for (const [k, v] of Object.entries(c || {})) {
    if (EXPORT_SKIP_KEYS.includes(k)) continue
    out[k] = v
  }
  return out
}

const openExport = async () => {
  await loadConnsForTransfer()
  if (!allConns.value.length) {
    ElMessage.warning(t('dsp.nothingToExport'))
    return
  }
  exportAll.value = true
  exportIds.value = allConns.value.map(c => c.id)
  exportVisible.value = true
}
const toggleAllExport = (v) => {
  exportIds.value = v ? allConns.value.map(c => c.id) : []
}
const doExport = () => {
  const picked = allConns.value.filter(c => exportIds.value.includes(c.id)).map(sanitizeForExport)
  if (!picked.length) return
  const bundle = {
    format: FILE_FORMAT,
    version: 1,
    exportedAt: new Date().toISOString(),
    note: 'password is NOT included; re-enter it after import',
    connections: picked
  }
  const blob = new Blob([JSON.stringify(bundle, null, 2)], { type: 'application/json;charset=utf-8' })
  // 让用户选存到哪（系统另存为）
  saveBlobAs(blob, 'dbmind-connections-' + new Date().toISOString().slice(0, 10) + '.json')
  exportVisible.value = false
  ElMessage.success(t('dsp.exported', { n: picked.length }))
}

const openImport = async () => {
  await loadConnsForTransfer()
  if (fileRef.value) {
    fileRef.value.value = ''
    fileRef.value.click()
  }
}
const parseBundle = (raw) => {
  let data = null
  try {
    data = JSON.parse(raw)
  } catch (e) {
    return { error: t('dsp.badJson') }
  }
  const list = Array.isArray(data)
    ? data
    : (data && Array.isArray(data.connections) ? data.connections : null)
  if (!list) return { error: t('dsp.noConnArray') }
  const valid = list.filter(c => c && typeof c === 'object'
    && String(c.name || '').trim() && String(c.type || '').trim())
  if (!valid.length) return { error: t('dsp.noValidConn') }
  return { list: valid }
}
const onFilePicked = async (e) => {
  const file = e.target.files && e.target.files[0]
  if (!file) return
  const parsed = parseBundle(await file.text())
  if (parsed.error) {
    ElMessage.error(parsed.error)
    return
  }
  const list = parsed.list
  try {
    await ElMessageBox.confirm(
      t('mv.importConfirm', { n: list.length, names: list.slice(0, 5).map(c => c.name).join(t('common.listSep')), more: (list.length > 5 ? t('mv.importMore') : '') })
        + t('dsp.importPwdNote'),
      t('mv.importTitle'),
      { type: 'warning', confirmButtonText: t('mv.importStart'), cancelButtonText: t('common.cancel') }
    )
  } catch (e2) {
    return
  }
  let ok = 0
  const failed = []
  for (const item of list) {
    const payload = sanitizeForExport(item)
    // 口令/口令相关的空值：空串表示"没设置"，不会覆盖任何东西（判断规则见 conn.rs 的 opt/空口令语义）
    payload.id = ''
    payload.password = ''
    payload.sshPassword = ''
    try {
      await saveConnection(payload)
      ok++
    } catch (err) {
      failed.push(t('mv.failItem', { name: item.name, detail: ((err && err.message) || t('common.unknownError')) }))
    }
  }
  emit('imported', { ok, failed })
  emit('update:modelValue', false)
  if (failed.length) {
    ElMessage.warning(t('dsp.importDone', { ok, failed: failed.length }))
    console.warn('导入失败的连接：', failed)
  } else {
    ElMessage.success(t('mv.importedConns', { n: ok }))
  }
}

// ========== 最近使用（选择器顶部置顶） ==========
// 只记 id 顺序（见 utils/recentConnections）：连接改名/改地址后不会显示过期信息，
// 被删掉的 id 自动跳过。
const recents = computed(() => rankRecentConnections(allConns.value))
const openRecent = (c) => {
  emit('open-conn', c.id)
  emit('update:modelValue', false)
}

// ========== 批量管理：测试 / 导出 / 删除 ==========
const batchTesting = ref(false)
const batchDeleting = ref(false)
const batchLog = ref('')
const openManager = async () => {
  await loadConnsForTransfer()
  exportAll.value = true
  exportIds.value = allConns.value.map(c => c.id)
  batchLog.value = ''
  exportVisible.value = true
}
const testSelected = async () => {
  const picked = allConns.value.filter(c => exportIds.value.includes(c.id))
  if (!picked.length) return
  batchTesting.value = true
  batchLog.value = t('dsp.testing', { n: picked.length })
  const failed = []
  for (const c of picked) {
    try {
      const res = await testConnectionById(c.id)
      if (!res || res.success === false) failed.push(t('mv.failItem', { name: c.name, detail: ((res && res.message) || t('mv.failed')) }))
    } catch (e) {
      failed.push(t('mv.failItem', { name: c.name, detail: ((e && e.message) || t('mv.failed')) }))
    }
  }
  batchTesting.value = false
  batchLog.value = failed.length
    ? (t('dsp.testDone', { ok: (picked.length - failed.length), failed: failed.length }) + '\n' + failed.join('\n'))
    : t('dsp.testAllOk', { n: picked.length })
  if (!failed.length) ElMessage.success(t('dsp.testAllOkShort'))
}
const deleteSelected = async () => {
  const picked = allConns.value.filter(c => exportIds.value.includes(c.id))
  if (!picked.length) return
  try {
    await ElMessageBox.confirm(
      t('dsp.deleteConfirm', { n: picked.length, names: picked.slice(0, 5).map(c => c.name).join(t('common.listSep')), more: (picked.length > 5 ? t('mv.importMore') : '') })
        + t('dsp.deleteNote'),
      t('dsp.deleteTitle'),
      { type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel') }
    )
  } catch (e) {
    return
  }
  batchDeleting.value = true
  const failed = []
  for (const c of picked) {
    try {
      await deleteConnection(c.id)
    } catch (e) {
      failed.push(t('mv.failItem', { name: c.name, detail: ((e && e.message) || t('dsp.deleteFailed')) }))
    }
  }
  batchDeleting.value = false
  await loadConnsForTransfer()
  exportIds.value = allConns.value.map(c => c.id)
  // 复用「导入完成」同一条刷新链路，让左侧树立刻同步
  emit('imported', { ok: picked.length - failed.length, failed })
  batchLog.value = failed.length
    ? (t('dsp.deleteDone', { ok: (picked.length - failed.length), failed: failed.length }) + '\n' + failed.join('\n'))
    : t('dsp.deleted', { n: picked.length })
  ElMessage.success(t('dsp.deleteDoneShort', { n: (picked.length - failed.length) }))
}
</script>

<style scoped>
/* 底部导入/导出区 */
.ds-footer { display: flex; align-items: center; gap: 8px; }
.ds-footer-tip { flex: 1; font-size: 12px; color: var(--dc-text-dim); }
/* 导出弹窗 */
.exp-tip { font-size: 13px; color: var(--dc-text-dim); line-height: 1.6; margin-bottom: 10px; }
.exp-list { max-height: 46vh; overflow-y: auto; margin-top: 6px; }
.exp-list :deep(.el-checkbox) { display: flex; align-items: center; margin-right: 0; height: 28px; }
.exp-meta { margin-left: 8px; font-size: 12px; color: var(--dc-text-dim); }
.exp-note { margin-left: 8px; font-size: 12px; color: var(--dc-text-dim); }
.exp-log {
  margin-top: 10px; padding: 8px 10px; border-radius: 8px; max-height: 22vh; overflow: auto;
  background: var(--dc-bg-deep); color: var(--dc-text-dim);
  font-size: 12px; line-height: 1.6; white-space: pre-wrap;
}
/* 最近使用 */
.ds-recent { margin-bottom: 16px; }
.ds-recent-title { font-size: 13px; color: var(--dc-text-dim); margin-bottom: 6px; }
.ds-recent-list { display: flex; flex-wrap: wrap; gap: 8px; }
.ds-recent-item {
  display: flex; align-items: center; gap: 6px; max-width: 280px;
  padding: 5px 10px; border-radius: 8px; cursor: pointer;
  background: var(--dc-bg-deep); border: 1px solid var(--dc-bg-hover);
  transition: border-color .15s;
}
.ds-recent-item:hover { border-color: var(--el-color-primary); }
.ds-recent-name { font-size: 13px; color: var(--dc-text); white-space: nowrap; }
.ds-recent-note {
  font-size: 12px; color: var(--dc-text-dim);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}

/* 深色主题覆盖 */
.ds-picker { max-height: 62vh; overflow-y: auto; padding-right: 4px; }
.ds-picker :deep(.el-input__wrapper) {
  background: var(--dc-bg-input); box-shadow: 0 0 0 1px var(--dc-bg-hover) inset;
}
.ds-picker :deep(.el-input__inner) { color: var(--dc-text); }
.ds-picker :deep(.el-empty__description) { color: var(--dc-text-dim); }

.ds-search { margin-bottom: 14px; }

.ds-group { margin-bottom: 18px; }
.ds-group-title {
  display: flex; align-items: center; gap: 6px;
  font-size: 14px; font-weight: 600; color: var(--dc-text-dim);
  margin-bottom: 10px;
}
.ds-group-count {
  font-size: 12px; font-weight: 400; color: var(--dc-text-dim);
  background: var(--dc-bg-hover); border-radius: 8px; padding: 0 7px; line-height: 16px;
}

.ds-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 10px;
}
.ds-card {
  display: flex; align-items: center; gap: 10px;
  padding: 12px; border: 1px solid var(--dc-bg-hover);
  border-radius: 10px; cursor: pointer; position: relative;
  transition: all .18s ease;
  background: var(--dc-bg-soft);
}
.ds-card:hover {
  border-color: var(--el-color-primary); background: var(--dc-bg-hover);
  transform: translateY(-2px); box-shadow: var(--dc-shadow-sm);
}
/* 弹窗内大图标：去掉浅色底板，避免在深色卡片上过于刺眼 */
.ds-card :deep(.db-logo img) {
  background: transparent !important;
  padding: 0;
  box-shadow: none;
}
.ds-card-info { min-width: 0; overflow: hidden; }
.ds-card-name { font-size: 14px; font-weight: 600; color: var(--dc-text-strong); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.ds-card-meta { font-size: 12px; color: var(--dc-text-dim); margin-top: 2px; }
.ds-card-badge {
  position: absolute; right: 8px; top: 8px;
  font-size: 11px; color: var(--dc-warning); background: var(--dc-warning-wash);
  border: 1px solid var(--dc-warning-wash); border-radius: 6px; padding: 0 5px;
}
.ds-card-ready { color: var(--dc-accent); background: var(--dc-success-wash); border-color: var(--dc-success-wash); }
</style>
