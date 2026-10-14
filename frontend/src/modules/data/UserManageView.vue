<template>
  <div class="user-manage">
    <!-- 工具条：连接选择 + 刷新 + 新增用户 -->
    <div class="um-head">
      <el-icon :size="16" class="um-head-icon"><User /></el-icon>
      <span class="um-title">{{ $t('nav.userManage') }}</span>

      <el-select v-model="selId" size="small" class="um-conn" filterable
                 :placeholder="$t('um.pickConn')" @change="onConnChange">
        <el-option v-for="c in conns" :key="c.id" :label="c.name" :value="String(c.id)">
          <span class="um-opt-name">{{ c.name }}</span>
          <span class="um-opt-type">{{ typeLabel(c.type) }}</span>
        </el-option>
      </el-select>

      <el-button size="small" :icon="Refresh" :loading="loading" @click="reload">{{ $t('common.refresh') }}</el-button>
      <el-button size="small" type="primary" :icon="Plus" :disabled="!canManage" @click="openCreate">
        {{ $t('um.newUser') }}
      </el-button>
      <span class="um-hint">{{ $t('um.grantHint') }}</span>
    </div>

    <!-- 主体 -->
    <div class="um-body">
      <div v-if="!conns.length" class="um-empty">{{ $t('um.noConn') }}</div>
      <div v-else-if="unsupported" class="um-empty">{{ $t('um.unsupported', { type: typeLabel(curConn?.type) }) }}</div>
      <div v-else-if="notImplemented" class="um-empty">{{ $t('um.notImplemented', { type: typeLabel(curConn?.type) }) }}</div>
      <div v-else-if="error" class="um-empty um-err">{{ error }}</div>
      <el-table v-else :data="rows" v-loading="loading" size="small" height="100%" stripe>
        <el-table-column prop="name" :label="$t('um.colName')" min-width="200" show-overflow-tooltip />
        <el-table-column v-if="hasHost" prop="host" :label="$t('um.colHost')" min-width="140" show-overflow-tooltip />
        <el-table-column :label="$t('um.colSuper')" width="120">
          <template #default="{ row }">
            <el-tag v-if="row.isSuper" size="small" type="warning" effect="plain">{{ $t('um.superTag') }}</el-tag>
            <span v-else class="um-dash">—</span>
          </template>
        </el-table-column>
        <el-table-column :label="$t('um.colComment')" min-width="200" show-overflow-tooltip>
          <template #default="{ row }">{{ row.comment || '—' }}</template>
        </el-table-column>
        <el-table-column :label="$t('um.colActions')" width="170" fixed="right">
          <template #default="{ row }">
            <el-button link type="primary" size="small" @click="openEdit(row)">{{ $t('um.editUser') }}</el-button>
            <el-button link type="danger" size="small" :disabled="!canManage" @click="dropUser(row)">
              {{ $t('common.delete') }}
            </el-button>
          </template>
        </el-table-column>
        <template #empty>
          <span class="um-dash">{{ $t('um.empty') }}</span>
        </template>
      </el-table>
    </div>
  </div>
</template>

<script setup>
/**
 * 用户管理（顶栏第一个入口打开的独立页面）。
 *
 * 为什么从"树上点用户节点"搬到这里：用户是**连接级**的对象，散在每棵库树里既不好找，
 * 也没法在一个地方对照着看。这里按连接汇总，新增 / 编辑（含授权）/ 删除都在一处。
 *
 * 复用而不是重造：
 * · 清单走 `/users`（`dialect.users()` 按方言给 SQL）；
 * · 新增 / 编辑 打开的还是原来那个 `UserForm`（它的授权卡片本来就按方言分好了），
 *   由 MainView 负责开页签 —— 本组件只管列表与删除；
 * · 能力判定用 `/features` 里的 `supportsUsers / supportsUserManage`（与树上原来那套
 *   完全同一个来源），不支持的类型在这里**如实说明**，而不是摆一堆点了报错的按钮。
 */
import { ref, computed, watch, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { User, Refresh, Plus } from '@element-plus/icons-vue'
import { listConnections, listUsers, getFeatures, userAction } from '../../api'
import { t } from '../../utils/i18n'

const props = defineProps({
  // 打开这个页签时的连接（不传就用连接列表里的第一个）
  connId: { type: String, default: '' },
  // 用户对象所在的库（Oracle/DM 这类是「库.模式」形态，交给后端拆）
  database: { type: String, default: '' },
  // 表单保存后 MainView 会 +1，用来刷新清单（跨组件的"该刷新了"信号）
  reloadTick: { type: Number, default: 0 }
})
const emit = defineEmits(['open-form'])

const conns = ref([])
const selId = ref('')
const rows = ref([])
const features = ref({})
const featLoaded = ref(false)
const loading = ref(false)
const error = ref('')

const typeLabel = (ty) => {
  const map = {
    ORACLE: 'Oracle', DM: '达梦', MYSQL: 'MySQL', MARIADB: 'MariaDB', DORIS: 'Apache Doris',
    POSTGRESQL: 'PostgreSQL', KINGBASE: 'KingbaseES', SQLSERVER: 'SQL Server',
    CLICKHOUSE: 'ClickHouse', SQLITE: 'SQLite', REDIS: 'Redis', MONGODB: 'MongoDB',
    H2: 'H2', DB2: 'DB2', DERBY: 'Derby'
  }
  const key = String(ty || '').toUpperCase()
  return map[key] || key || '—'
}
const errMsg = (e) => (e && (e.message || e.toString())) || ''

const curConn = computed(() => conns.value.find(c => String(c.id) === String(selId.value)) || null)
// 有没有"主机"这一维，由数据说了算（MySQL 系才带 host），不写死类型名单
const hasHost = computed(() => rows.value.some(r => r && r.host))
// 该类型有没有"用户"这个概念（与树上原来用的同一个字段）
const unsupported = computed(() => !selId.value || features.value.supportsUsers === false)
// 能力表里**根本没有这个字段** = 该类型连用户管理这条链路都没实现（如 Redis / MongoDB）。
// 与"有这个类型但取不到清单"区分开：前者不该去发那个注定 404 的请求，后者要如实报错。
const notImplemented = computed(() => featLoaded.value && features.value.supportsUsers === undefined)
// 能不能在这里增删改（后端 `supportsUserManage`；不支持的只给只读清单）
const canManage = computed(() => !!selId.value && features.value.supportsUsers !== false
  && features.value.supportsUserManage !== false)

const reload = async () => {
  if (!selId.value) { rows.value = []; features.value = {}; return }
  loading.value = true
  error.value = ''
  try {
    features.value = await getFeatures(selId.value).catch(() => ({}))
    featLoaded.value = !!features.value && Object.keys(features.value).length > 0
    if (features.value.supportsUsers === false || (featLoaded.value && features.value.supportsUsers === undefined)) {
      rows.value = []
      return
    }
    const list = await listUsers(selId.value, props.database || '')
    rows.value = Array.isArray(list) ? list : []
  } catch (e) {
    rows.value = []
    // 不静默：拿不到清单和"确实一个用户都没有"必须能区分
    error.value = t('um.loadFailed', { detail: errMsg(e) })
  } finally {
    loading.value = false
  }
}

const onConnChange = () => reload()

const openCreate = () => {
  if (!canManage.value) return
  emit('open-form', { mode: 'create', connId: selId.value, database: props.database || '' })
}
const openEdit = (row) => {
  const host = row.host || ''
  const fullName = host ? row.name + '@' + host : row.name
  emit('open-form', { mode: 'edit', connId: selId.value, database: props.database || '', name: row.name, host, fullName })
}
const dropUser = (row) => {
  const host = row.host || ''
  const fullName = host ? row.name + '@' + host : row.name
  ElMessageBox.confirm(
    t('um.deleteBody', { name: fullName }),
    t('um.deleteTitle'),
    { type: 'error', confirmButtonText: t('um.deleteTitle'), cancelButtonText: t('common.cancel') }
  ).then(async () => {
    try {
      const res = await userAction(selId.value, {
        action: 'drop', database: props.database || '', userName: row.name, host
      })
      if (res && res.success) {
        ElMessage.success(t('um.deleted', { name: fullName }))
        reload()
      } else {
        ElMessage.error(t('um.deleteFailed', { detail: (res && res.message) || t('common.unknownError') }))
      }
    } catch (e) {
      ElMessage.error(t('um.deleteFailed', { detail: errMsg(e) }))
    }
  }).catch(() => {})
}

watch(() => props.reloadTick, () => reload())
watch(() => props.connId, (val) => { if (val && String(val) !== selId.value && !selId.value) { selId.value = String(val); reload() } })

onMounted(async () => {
  try {
    const list = await listConnections()
    conns.value = Array.isArray(list) ? list : []
  } catch (e) {
    conns.value = []
  }
  // 优先用打开时的连接；否则第一个（列表顺序就是侧栏里的顺序）
  selId.value = String(props.connId || conns.value[0]?.id || '')
  await reload()
})
</script>

<style scoped>
.user-manage { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--dc-bg); }
.um-head {
  display: flex; align-items: center; gap: 8px; padding: 8px 12px; flex-wrap: wrap;
  border-bottom: 1px solid var(--dc-border); background: var(--dc-bg-card);
}
.um-head-icon { color: #a855f7; }
.um-title { font-size: 13px; font-weight: 600; color: var(--dc-text); margin-right: 4px; }
.um-conn { width: 220px; }
.um-opt-name { float: left; }
.um-opt-type { float: right; color: var(--dc-text-dim); font-size: 12px; margin-left: 12px; }
.um-hint { font-size: 12px; color: var(--dc-text-dim); }
.um-body { flex: 1; min-height: 0; padding: 8px 12px 12px; }
.um-empty { color: var(--dc-text-dim); font-size: 13px; padding: 24px 4px; }
.um-err { color: var(--el-color-danger); }
.um-dash { color: var(--dc-text-dim); }
</style>
