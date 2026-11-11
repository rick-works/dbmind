<template>
  <el-dialog :model-value="modelValue" width="min(880px, 94vw)" top="8vh" append-to-body
             @update:model-value="$emit('update:modelValue', $event)"
             class="cm-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Connection /></el-icon></span>
        <span>{{ $t('cm.title') }}</span>
      </div>
    </template>

    <div class="cm-tip">{{ $t('cm.desc') }}</div>

    <div class="cm-bar">
      <el-checkbox :model-value="allChecked" :indeterminate="indeterminate" @change="toggleAll">
        {{ $t('common.selectAll') }}
      </el-checkbox>
      <span class="cm-count">{{ ids.length }} / {{ items.length }}</span>
      <el-input v-model="keyword" :prefix-icon="Search" clearable class="cm-search" :placeholder="$t('ce.search')" />
    </div>

    <div class="cm-list">
      <template v-for="row in treeRows" :key="row.key">
        <div v-if="row.type === 'dir'" class="cm-dir"
             :style="{ paddingLeft: row.depth * 16 + 2 + 'px' }"
             @click="toggleDir(row.dirKey)">
          <el-icon class="cm-dir-arrow" :size="12">
            <ArrowRight v-if="isCollapsed(row.dirKey)" /><ArrowDown v-else />
          </el-icon>
          <el-checkbox class="cm-dir-ck"
                       :model-value="dirAll(row)" :indeterminate="dirSome(row)"
                       @click.stop
                       @change="(v) => toggleDirAll(row, v)" />
          <el-icon class="cm-dir-folder" :size="14"><Folder /></el-icon>
          <span class="cm-dir-name">{{ row.name }}</span>
          <span class="cm-dir-n">{{ row.count }}</span>
        </div>
        <el-checkbox v-else
                     class="cm-conn"
                     :model-value="ids.includes(row.conn.id)"
                     :style="{ paddingLeft: row.depth * 16 + 3 + 'px' }"
                     @change="(v) => toggle(row.conn.id, v)">
          <DbLogo :type="row.conn.type" :size="16" class="cm-logo" />
          <span class="cm-name">{{ row.conn.name }}</span>
          <span class="cm-meta">
            <template v-if="row.conn.host">{{ row.conn.host }}</template><template v-if="row.conn.port">:{{ row.conn.port }}</template>
          </span>
          <span v-if="row.conn.readOnly" class="cm-ro">{{ $t('cm.readOnly') }}</span>
        </el-checkbox>
      </template>
      <el-empty v-if="!treeRows.length" :image-size="66" :description="keyword ? $t('common.noMatch') : $t('cm.empty')" />
    </div>

    <div v-if="log" class="cm-log">{{ log }}</div>

    <template #footer>
      <el-button @click="$emit('update:modelValue', false)">{{ $t('common.close') }}</el-button>
      <el-button :disabled="!ids.length" :loading="testing" @click="testSelected">{{ $t('dsp.testSelected') }}</el-button>
      <el-button :disabled="!ids.length" @click="openExport">{{ $t('cm.exportSelected') }}</el-button>
      <el-button type="danger" plain :disabled="!ids.length" :loading="deleting" @click="deleteSelected">
        {{ $t('dsp.deleteSelected') }}
      </el-button>
    </template>

    <ExportConnsDialog v-model="exportVisible" :preset-ids="ids" />
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { Search, Connection, ArrowRight, ArrowDown, Folder } from '@element-plus/icons-vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { t } from '../utils/i18n'
import { listConnections, deleteConnection, testConnectionById } from '../api'
import { buildConnTreeRows } from '../utils/connTree'
import ExportConnsDialog from './ExportConnsDialog.vue'
import DbLogo from './DbLogo.vue'

/**
 * 连接管理：勾选若干条后批量测试 / 批量导出 / 批量删除。
 *
 * 这块原来长在「新建数据源」弹窗的底部（那里还同时塞了导入、导出），三个入口混在一起
 * 既挤又容易点错；现在统一收进树右键「管理连接」，数据源弹窗只管挑类型。
 */
const props = defineProps({ modelValue: Boolean })
const emit = defineEmits(['update:modelValue', 'changed'])

const items = ref([])
const ids = ref([])
const keyword = ref('')
const log = ref('')
const testing = ref(false)
const deleting = ref(false)
const exportVisible = ref(false)

const allChecked = computed(() => items.value.length > 0 && ids.value.length === items.value.length)
const indeterminate = computed(() => ids.value.length > 0 && ids.value.length < items.value.length)
/** 目录折叠状态（key = 目录全路径） */
const collapsedDirs = ref(new Set())
const isCollapsed = (key) => !keyword.value.trim() && collapsedDirs.value.has(key)
const toggleDir = (key) => {
  const set = collapsedDirs.value
  if (set.has(key)) set.delete(key)
  else set.add(key)
}

/** 与导出弹窗同一套目录树（utils/connTree），链接按 group 路径分层 */
const treeRows = computed(() => buildConnTreeRows({
  conns: items.value,
  keyword: keyword.value,
  collapsed: collapsedDirs.value,
  ungrouped: t('cm.ungrouped')
}))

const load = async () => {
  try {
    items.value = (await listConnections()) || []
  } catch (e) {
    items.value = []
  }
  ids.value = items.value.map((c) => c.id)
  keyword.value = ''
  log.value = ''
}

watch(() => props.modelValue, (v) => { if (v) load() })

const toggleAll = (checked) => { ids.value = checked ? items.value.map((c) => c.id) : [] }
const toggle = (id, checked) => {
  const set = new Set(ids.value)
  if (checked) set.add(id)
  else set.delete(id)
  ids.value = items.value.filter((c) => set.has(c.id)).map((c) => c.id)
}

/** 目录行三态：整组全选 / 部分选中（含子目录） */
const dirAll = (row) => row.connIds.length > 0 && row.connIds.every((id) => ids.value.includes(id))
const dirSome = (row) => {
  const picked = row.connIds.filter((id) => ids.value.includes(id)).length
  return picked > 0 && picked < row.connIds.length
}
const toggleDirAll = (row, checked) => {
  const set = new Set(ids.value)
  for (const id of row.connIds) {
    if (checked) set.add(id)
    else set.delete(id)
  }
  ids.value = items.value.filter((c) => set.has(c.id)).map((c) => c.id)
}

const openExport = () => { exportVisible.value = true }

const testSelected = async () => {
  const picked = items.value.filter((c) => ids.value.includes(c.id))
  if (!picked.length) return
  testing.value = true
  log.value = t('dsp.testing', { n: picked.length })
  const failed = []
  for (const c of picked) {
    try {
      const res = await testConnectionById(c.id)
      if (!res || res.success === false) {
        failed.push(t('mv.failItem', { name: c.name, detail: (res && res.message) || t('mv.failed') }))
      }
    } catch (e) {
      failed.push(t('mv.failItem', { name: c.name, detail: (e && e.message) || t('mv.failed') }))
    }
  }
  testing.value = false
  log.value = failed.length
    ? (t('dsp.testDone', { ok: picked.length - failed.length, failed: failed.length }) + '\n' + failed.join('\n'))
    : t('dsp.testAllOk', { n: picked.length })
  if (!failed.length) ElMessage.success(t('dsp.testAllOkShort'))
}

const deleteSelected = async () => {
  const picked = items.value.filter((c) => ids.value.includes(c.id))
  if (!picked.length) return
  try {
    await ElMessageBox.confirm(
      t('dsp.deleteConfirm', {
        n: picked.length,
        names: picked.slice(0, 5).map((c) => c.name).join(t('common.listSep')),
        more: picked.length > 5 ? t('mv.importMore') : ''
      }) + t('dsp.deleteNote'),
      t('dsp.deleteTitle'),
      { type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel') }
    )
  } catch (e) {
    return
  }
  deleting.value = true
  const failed = []
  for (const c of picked) {
    try {
      await deleteConnection(c.id)
    } catch (e) {
      failed.push(t('mv.failItem', { name: c.name, detail: (e && e.message) || t('dsp.deleteFailed') }))
    }
  }
  deleting.value = false
  await load()
  emit('changed', { ok: picked.length - failed.length, failed })
  log.value = failed.length
    ? (t('dsp.deleteDone', { ok: picked.length - failed.length, failed: failed.length }) + '\n' + failed.join('\n'))
    : t('dsp.deleted', { n: picked.length })
  ElMessage.success(t('dsp.deleteDoneShort', { n: picked.length - failed.length }))
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
.cm-tip { font-size: 13px; color: var(--dc-text-dim); line-height: 1.6; margin-bottom: 10px; }
.cm-bar { display: flex; align-items: center; gap: 10px; padding-bottom: 8px; border-bottom: 1px solid var(--dc-bg-hover); }
.cm-count { font-size: 12px; color: var(--dc-text-dim); }
.cm-search { width: 220px; margin-left: auto; }
/* 高度自适应：body 摊成 flex 列，列表吃掉剩余高度（拖大不留白、拖小自己滚） */
:global(.el-dialog.cm-dialog) {
  display: flex; flex-direction: column;
  max-height: calc(100vh - 16vh);
}
:global(.el-dialog.cm-dialog .el-dialog__body) {
  flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column;
}
.cm-tip, .cm-bar, .cm-log { flex: none; }
.cm-list {
  flex: 1 1 auto; min-height: 96px;
  overflow-y: auto; overflow-x: hidden; margin-top: 4px;
}
/* 目录行：与导出弹窗同一套观感（可折叠 + 层级缩进） */
.cm-dir {
  display: flex; align-items: center; gap: 5px; height: 30px;
  box-sizing: border-box; border-radius: 6px; cursor: pointer;
  font-size: 12px; font-weight: 600; color: var(--dc-text-dim);
}
.cm-dir:hover { background: var(--dc-bg-hover); }
.cm-dir-ck { flex: none; height: auto; margin-right: 2px; }
.cm-dir-ck :deep(.el-checkbox__label) { display: none; }
.cm-dir-arrow { flex: none; color: var(--dc-text-dim); }
.cm-dir-folder { flex: none; color: var(--dc-accent); }
.cm-dir-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.cm-dir-n { flex: none; font-weight: 400; background: var(--dc-bg-hover); border-radius: 8px; padding: 0 6px; }
.cm-list :deep(.cm-conn) {
  display: flex; align-items: center; height: 32px; margin-right: 0;
  width: 100%; box-sizing: border-box;
}
.cm-list :deep(.el-checkbox__label) { display: flex; align-items: center; min-width: 0; overflow: hidden; }
/* 连接行的类型图标：与目录行的文件夹图标同一列，所以父子行的名称能对上 */
.cm-logo { flex: none; margin-right: 6px; }
.cm-name { flex: none; font-size: 13px; color: var(--dc-text); }
.cm-meta {
  margin-left: 8px; font-size: 12px; color: var(--dc-text-dim);
  min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.cm-ro {
  margin-left: 8px; font-size: 11px; color: var(--dc-warning);
  background: var(--dc-warning-wash); border-radius: 6px; padding: 0 5px;
}
.cm-log {
  margin-top: 10px; padding: 8px 10px; border-radius: 8px; max-height: 22vh; overflow: auto;
  background: var(--dc-bg-deep); color: var(--dc-text-dim);
  font-size: 12px; line-height: 1.6; white-space: pre-wrap;
}
</style>
