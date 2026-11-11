<template>
  <el-dialog :model-value="modelValue" width="min(880px, 94vw)" top="8vh" append-to-body
             :close-on-click-modal="!saving" :show-close="!saving"
             @update:model-value="$emit('update:modelValue', $event)"
             class="ec-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Download /></el-icon></span>
        <span>{{ $t('ce.title') }}</span>
      </div>
    </template>

    <div class="ec-tip">{{ $t('ce.desc') }}</div>

    <div class="ec-bar">
      <el-checkbox :model-value="allChecked" :indeterminate="indeterminate" @change="toggleAll">
        {{ $t('common.selectAll') }}
      </el-checkbox>
      <span class="ec-count">{{ ids.length }} / {{ items.length }}</span>
      <el-input v-model="keyword" :prefix-icon="Search" clearable class="ec-search" :placeholder="$t('ce.search')" />
    </div>

    <div class="ec-list">
      <template v-for="row in treeRows" :key="row.key">
        <div v-if="row.type === 'dir'" class="ec-dir"
             :style="{ paddingLeft: row.depth * 16 + 2 + 'px' }"
             @click="toggleDir(row.dirKey)">
          <el-icon class="ec-dir-arrow" :size="12">
            <ArrowRight v-if="isCollapsed(row.dirKey)" /><ArrowDown v-else />
          </el-icon>
          <el-checkbox class="ec-dir-ck"
                       :model-value="dirAll(row)" :indeterminate="dirSome(row)"
                       @click.stop
                       @change="(v) => toggleDirAll(row, v)" />
          <el-icon class="ec-dir-folder" :size="14"><Folder /></el-icon>
          <span class="ec-dir-name">{{ row.name }}</span>
          <span class="ec-dir-n">{{ row.count }}</span>
        </div>
        <el-checkbox v-else
                     class="ec-conn"
                     :model-value="ids.includes(row.conn.id)"
                     :style="{ paddingLeft: row.depth * 16 + 3 + 'px' }"
                     @change="(v) => toggleOne(row.conn.id, v)">
          <DbLogo :type="row.conn.type" :size="16" class="ec-logo" />
          <span class="ec-name">{{ row.conn.name }}</span>
          <span class="ec-meta">
            <template v-if="row.conn.host">{{ row.conn.host }}</template><template v-if="row.conn.port">:{{ row.conn.port }}</template>
          </span>
        </el-checkbox>
      </template>
      <el-empty v-if="!treeRows.length" :image-size="70" :description="keyword ? $t('common.noMatch') : $t('ce.empty')" />
    </div>

    <!-- 口令开关：默认不勾（与历史行为一致），勾了才会向内核要明文 -->
    <div class="ec-pwd">
      <el-checkbox v-model="withPassword">{{ $t('ce.pwd') }}</el-checkbox>
      <span v-if="withPassword" class="ec-warn">
        <el-icon :size="13"><WarningFilled /></el-icon>{{ $t('ce.pwdWarn') }}
      </span>
      <span v-else class="ec-dim">{{ $t('ce.pwdOff') }}</span>
    </div>

    <template #footer>
      <el-button :disabled="saving" @click="close">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" :disabled="!ids.length" :loading="saving" @click="doExport">
        {{ $t('ce.exportN', { n: ids.length }) }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { Search, Download, WarningFilled, ArrowRight, ArrowDown, Folder } from '@element-plus/icons-vue'
import { ElMessage } from 'element-plus'
import { t } from '../utils/i18n'
import { listConnections, exportConnections } from '../api'
import { sanitizeConnection, downloadJson, CONN_FILE_FORMAT, CONN_FILE_EXT } from '../utils/connTransfer'
import { buildConnTreeRows } from '../utils/connTree'
import DbLogo from './DbLogo.vue'

/**
 * 导出连接配置。
 *
 * 两个入口共用：左侧树右键「导出连接」、连接管理弹窗「导出选中」（后者把 presetIds 传进来）。
 * 口令是**可选项**：默认不导出（历史上一直如此）；勾了才走后端的导出接口拿明文口令 ——
 * 前端从来拿不到口令（`ConnectionConfig::password` 标了 skip_serializing），所以这一步必须有后端配合。
 */
const props = defineProps({
  modelValue: Boolean,
  /** 预勾选的连接 id（管理弹窗"导出选中"用）；空 = 全选 */
  presetIds: { type: Array, default: () => [] }
})
const emit = defineEmits(['update:modelValue'])

const items = ref([])
const ids = ref([])
const keyword = ref('')
const withPassword = ref(false)
const saving = ref(false)

const allChecked = computed(() => items.value.length > 0 && ids.value.length === items.value.length)
const indeterminate = computed(() => ids.value.length > 0 && ids.value.length < items.value.length)

/** 折叠中的目录（key = 目录全路径）。搜索时一律展开，否则命中项会被藏起来 */
const collapsedDirs = ref(new Set())
const isCollapsed = (key) => !keyword.value.trim() && collapsedDirs.value.has(key)
const toggleDir = (key) => {
  const set = collapsedDirs.value
  if (set.has(key)) set.delete(key)
  else set.add(key)
}

/**
 * 按**目录**把连接排成一棵树（不再按环境 PROD/DEV 分组 —— 那只是生产库的防误连标记，
 * 与"这条连接放在哪"无关）。目录可折叠，行按目录深度缩进。
 */
const treeRows = computed(() => buildConnTreeRows({
  conns: items.value,
  keyword: keyword.value,
  collapsed: collapsedDirs.value,
  ungrouped: t('ce.ungrouped')
}))

const toggleAll = (checked) => {
  ids.value = checked ? items.value.map((c) => c.id) : []
}
const toggleOne = (id, checked) => {
  const set = new Set(ids.value)
  if (checked) set.add(id)
  else set.delete(id)
  ids.value = items.value.filter((c) => set.has(c.id)).map((c) => c.id)
}

/** 目录行的三态：整组全选 / 部分选中（含子目录） */
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

const close = () => { if (!saving.value) emit('update:modelValue', false) }

const load = async () => {
  try {
    items.value = (await listConnections()) || []
  } catch (e) {
    items.value = []
  }
  const preset = (props.presetIds || []).filter((id) => items.value.some((c) => c.id === id))
  ids.value = preset.length ? preset : items.value.map((c) => c.id)
  keyword.value = ''
  withPassword.value = false
}

watch(() => props.modelValue, (v) => { if (v) load() })

const doExport = async () => {
  const picked = items.value.filter((c) => ids.value.includes(c.id))
  if (!picked.length) return
  saving.value = true
  try {
    let list
    if (withPassword.value) {
      // 带口令的导出只能问内核要：它会就地补 password / sshPassword / sshKeyPassphrase
      const full = await exportConnections(picked.map((c) => c.id), true)
      list = (full || []).map(sanitizeConnection)
    } else {
      list = picked.map(sanitizeConnection)
    }
    const bundle = {
      format: CONN_FILE_FORMAT,
      version: 1,
      exportedAt: new Date().toISOString(),
      note: withPassword.value ? t('ce.notePwd') : t('ce.noteNoPwd'),
      connections: list
    }
    const name = 'dbmind-connections-' + new Date().toISOString().slice(0, 10) + '.' + CONN_FILE_EXT
    const res = await downloadJson(name, bundle)
    if (res && res.canceled) return
    // 保存失败已经由 saveBlobAs 统一提示过了，这里别再报「已导出」
    if (res && res.ok === false) return
    // 保存到哪由用户自己选（mac / Linux 桌面壳弹的是系统「另存为」面板），
    // 所以提示里带文件名为好 —— 只说「已导出 N 条」的话，用户不知道东西存哪去了
    ElMessage.success(
      res && res.name ? t('mv.exportSavedAs', { name: res.name }) : t('ce.done', { n: list.length })
    )
    emit('update:modelValue', false)
  } catch (e) {
    ElMessage.error(t('ce.failed', { detail: (e && e.message) || t('common.unknownError') }))
  } finally {
    saving.value = false
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
.ec-tip { font-size: 13px; color: var(--dc-text-dim); line-height: 1.6; margin-bottom: 10px; }
.ec-bar { display: flex; align-items: center; gap: 10px; padding-bottom: 8px; border-bottom: 1px solid var(--dc-bg-hover); }
.ec-count { font-size: 12px; color: var(--dc-text-dim); }
.ec-search { width: 220px; margin-left: auto; }
/* 高度自适应（与 TaskProgressDialog 同一套范式）：
   弹窗本身是 flex 列（全局 .el-dialog.dc-resizable 已这么设），body 再摊成 flex 列，
   列表 `flex:1` 吃掉剩余高度 —— 拖大弹窗列表跟着长（不留白），拖小则自己滚动。
   以前给列表写死 max-height，弹窗被拖大后下方就是一大块空白。 */
:global(.el-dialog.ec-dialog) {
  display: flex; flex-direction: column;
  max-height: calc(100vh - 16vh);
}
:global(.el-dialog.ec-dialog .el-dialog__body) {
  flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column;
}
.ec-tip, .ec-bar, .ec-pwd { flex: none; }
.ec-list {
  flex: 1 1 auto; min-height: 96px;
  overflow-y: auto; overflow-x: hidden; margin-top: 4px;
}
/* 目录行：可折叠，缩进按层级加（行内 padding-left） */
.ec-dir {
  display: flex; align-items: center; gap: 5px; height: 30px;
  box-sizing: border-box; border-radius: 6px; cursor: pointer;
  font-size: 12px; font-weight: 600; color: var(--dc-text-dim);
}
.ec-dir:hover { background: var(--dc-bg-hover); }
/* 目录行的勾选框：不占满整行、高度跟随 30px 的目录行 */
.ec-dir-ck { flex: none; height: auto; margin-right: 2px; }
.ec-dir-ck :deep(.el-checkbox__label) { display: none; }
.ec-dir-arrow { flex: none; color: var(--dc-text-dim); }
.ec-dir-folder { flex: none; color: var(--dc-accent); }
.ec-dir-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ec-dir-n { flex: none; font-weight: 400; background: var(--dc-bg-hover); border-radius: 8px; padding: 0 6px; }
/* 只有连接行占满整行（目录行里的勾选框是小的），两者必须分开选，否则目录行会被撑开 */
.ec-list :deep(.ec-conn) {
  display: flex; align-items: center; height: 32px; margin-right: 0;
  width: 100%; box-sizing: border-box;
}
.ec-list :deep(.el-checkbox__label) { display: flex; align-items: center; min-width: 0; overflow: hidden; }
/* 连接行的类型图标：与目录行的文件夹图标同一列，父子行名称自然对齐 */
.ec-logo { flex: none; margin-right: 6px; }
.ec-name { font-size: 13px; color: var(--dc-text); flex: none; }
.ec-meta {
  margin-left: 8px; font-size: 12px; color: var(--dc-text-dim);
  min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.ec-pwd {
  margin-top: 12px; padding: 10px 12px; border-radius: 8px;
  background: var(--dc-bg-soft); border: 1px solid var(--dc-bg-hover);
}
.ec-warn { display: inline-flex; align-items: center; gap: 4px; margin-left: 10px; font-size: 12px; color: var(--dc-warning); }
.ec-dim { margin-left: 10px; font-size: 12px; color: var(--dc-text-dim); }
</style>
