<template>
  <el-dialog
    v-model="visible"
    :title="$t('nav.scripts')"
    width="880px"
    :close-on-click-modal="false"
    append-to-body
    class="script-mgr-dialog"
  >
    <!-- 工具条：条件搜索（脚本名 / 内容 / 连接 / 库 都参与匹配）+ 新建脚本 -->
    <div class="sm-toolbar">
      <el-input
        v-model="keyword"
        size="small"
        clearable
        :placeholder="$t('sm.searchPlaceholder')"
        :prefix-icon="Search"
        class="sm-search"
      />
      <div class="sm-count">{{ $t('sm.count', { n: filtered.length }) }}</div>
      <el-button type="primary" size="small" :icon="Plus" @click="$emit('create')">
        {{ $t('nav.newScript') }}
      </el-button>
    </div>

    <el-table
      :data="filtered"
      size="small"
      height="440"
      :empty-text="$t('sm.empty')"
      class="sm-table"
    >
      <el-table-column :label="$t('sm.colName')" min-width="220">
        <template #default="{ row }">
          <div class="sm-name-cell" :title="preview(row.content)">
            <el-icon class="sm-name-icon"><Tickets /></el-icon>
            <span class="sm-name">{{ row.name }}</span>
          </div>
        </template>
      </el-table-column>
      <el-table-column :label="$t('sm.colConn')" min-width="150">
        <template #default="{ row }">
          <div class="sm-conn-cell">
            <DbLogo :type="row.connType || 'MYSQL'" :size="14" />
            <span>{{ row.connName || $t('sm.connGone') }}</span>
          </div>
        </template>
      </el-table-column>
      <el-table-column prop="db" :label="$t('sm.colDb')" min-width="140" show-overflow-tooltip />
      <el-table-column :label="$t('sm.colTime')" width="160">
        <template #default="{ row }">
          <span class="sm-time">{{ fmtTime(row.time) }}</span>
        </template>
      </el-table-column>
      <el-table-column :label="$t('sm.colActions')" width="130" align="center">
        <template #default="{ row }">
          <el-button link type="primary" size="small" @click="openRow(row)">{{ $t('sm.open') }}</el-button>
          <el-button link type="danger" size="small" @click="removeRow(row)">{{ $t('common.delete') }}</el-button>
        </template>
      </el-table-column>
    </el-table>
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Search, Plus, Tickets } from '@element-plus/icons-vue'
import { listConnections } from '../../api'
import { t } from '../../utils/i18n'
import DbLogo from '../../common/DbLogo.vue'

const props = defineProps({ modelValue: Boolean })
const emit = defineEmits(['update:modelValue', 'open', 'create'])

const visible = ref(false)
watch(() => props.modelValue, (v) => { visible.value = v })
watch(visible, (v) => emit('update:modelValue', v))
// 每次打开都重新枚举（编辑器里随时可能新增/修改了脚本）
watch(visible, (v) => { if (v) load() })

const keyword = ref('')
const rows = ref([])
const connMap = ref({})

// ===== 数据枚举：遍历 localStorage 里 dc_scripts:<connId>:<db> 前缀的全部 key =====
// value 结构与 SQL 编辑器的保存格式一致：[{ id, name, content, time }]
const SCRIPT_PREFIX = 'dc_scripts:'
const load = async () => {
  // 连接映射：id → 名称/类型（连接可能已被删除，此时显示「连接已删除」但脚本仍可打开/删除）
  try {
    const list = await listConnections()
    const arr = Array.isArray(list) ? list : (list?.data || [])
    connMap.value = Object.fromEntries(arr.map(c => [String(c.id), c]))
  } catch { connMap.value = {} }
  const out = []
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i)
      if (!key || !key.startsWith(SCRIPT_PREFIX)) continue
      const rest = key.slice(SCRIPT_PREFIX.length)
      const at = rest.indexOf(':')
      if (at <= 0) continue
      const connId = rest.slice(0, at)
      const db = rest.slice(at + 1)
      let scripts = []
      try { scripts = JSON.parse(localStorage.getItem(key) || '[]') } catch { continue }
      for (const s of (Array.isArray(scripts) ? scripts : [])) {
        if (!s || !s.id) continue
        const c = connMap.value[String(connId)]
        out.push({
          connId, db, id: s.id, name: s.name || s.id,
          content: s.content || '', time: s.time || 0,
          connName: c?.name || '', connType: c?.type || ''
        })
      }
    }
  } catch { /* localStorage 不可用时给空列表 */ }
  out.sort((a, b) => (b.time || 0) - (a.time || 0))
  rows.value = out
}

const preview = (content) =>
  String(content || '').split('\n').map(l => l.trim()).filter(Boolean).slice(0, 6).join('  ·  ').slice(0, 400)

const fmtTime = (ts) => {
  if (!ts) return '—'
  const d = new Date(ts)
  const p = (n) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  if (!kw) return rows.value
  return rows.value.filter(r =>
    r.name.toLowerCase().includes(kw)
    || String(r.content || '').toLowerCase().includes(kw)
    || String(r.db || '').toLowerCase().includes(kw)
    || String(r.connName || '').toLowerCase().includes(kw)
  )
})

const openRow = (row) => emit('open', { connId: row.connId, db: row.db, scriptId: row.id })

const removeRow = async (row) => {
  try {
    await ElMessageBox.confirm(t('sm.delConfirm', { name: row.name }), t('sm.delTitle'), {
      type: 'warning',
      confirmButtonText: t('common.delete'),
      cancelButtonText: t('common.cancel')
    })
  } catch { return }
  const key = SCRIPT_PREFIX + row.connId + ':' + row.db
  try {
    const list = JSON.parse(localStorage.getItem(key) || '[]')
    const next = (Array.isArray(list) ? list : []).filter(s => s.id !== row.id)
    localStorage.setItem(key, JSON.stringify(next))
  } catch { return }
  // 通知树与编辑器刷新（MainView 监听该事件刷新 scripts 节点）
  window.dispatchEvent(new CustomEvent('dc-scripts-changed', { detail: { connId: row.connId, db: row.db } }))
  ElMessage.success(t('sm.deleted'))
  load()
}

defineExpose({ reload: load })
</script>

<style scoped>
.sm-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 10px;
}
.sm-search { width: 320px; }
.sm-count {
  flex: 1;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.sm-name-cell { display: flex; align-items: center; gap: 6px; min-width: 0; }
.sm-name-icon { color: var(--el-color-primary); flex: none; }
.sm-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sm-conn-cell { display: flex; align-items: center; gap: 6px; }
.sm-time { font-size: 12px; color: var(--el-text-color-secondary); }
</style>
