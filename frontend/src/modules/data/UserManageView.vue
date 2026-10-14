<template>
  <div class="user-manage">
    <!-- 工具条：连接选择 + 刷新 + 新增用户 -->
    <div class="um-head">
      <el-icon :size="16" class="um-head-icon"><User /></el-icon>
      <span class="um-title">{{ $t('nav.userManage') }}</span>

      <el-select v-model="selId" size="small" class="um-conn" filterable clearable
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
    </div>

    <!-- 主体 -->
    <div class="um-body">
      <div v-if="noSelection" class="um-empty">{{ $t('um.pickConnHint') }}</div>
      <div v-else-if="!conns.length" class="um-empty">{{ $t('um.noConn') }}</div>
      <div v-else-if="unsupported" class="um-empty">{{ $t('um.unsupported', { type: typeLabel(curConn?.type) }) }}</div>
      <div v-else-if="notImplemented" class="um-empty">{{ $t('um.notImplemented', { type: typeLabel(curConn?.type) }) }}</div>
      <div v-else-if="error" class="um-empty um-err">{{ error }}</div>
      <!-- 列表：与数据视图/字段表**同一套表格样式**（表头浅底 + 横竖网格 + 紧凑行 + 贴卡片内缘）。
           这里自己出表而不用 el-table：只有五列、几十行，原生表才能真正和项目其它表格一个模样
           （el-table 自带一整套边框/斑马纹/悬停样式，要压很久还容易跟主题打架）。 -->
      <div v-else class="um-table-wrap" v-loading="loading">
        <table class="um-table">
          <colgroup>
            <col style="width: 220px" />
            <col v-if="hasHost" style="width: 160px" />
            <col style="width: 120px" />
            <col />
            <col style="width: 170px" />
          </colgroup>
          <thead>
            <tr>
              <th>{{ $t('um.colName') }}</th>
              <th v-if="hasHost">{{ $t('um.colHost') }}</th>
              <th class="um-c">{{ $t('um.colSuper') }}</th>
              <th>{{ $t('um.colComment') }}</th>
              <th class="um-act">{{ $t('um.colActions') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="!rows.length">
              <td :colspan="hasHost ? 5 : 4" class="um-empty-row">{{ $t('um.empty') }}</td>
            </tr>
            <tr v-for="row in rows" :key="String(row.name) + '@' + String(row.host || '')">
              <td :title="row.name">{{ row.name }}</td>
              <td v-if="hasHost" :title="row.host">{{ row.host }}</td>
              <td class="um-c">
                <el-tag v-if="row.isSuper" size="small" type="warning" effect="plain">{{ $t('um.superTag') }}</el-tag>
                <span v-else class="um-dash">—</span>
              </td>
              <td :title="row.comment">{{ row.comment || '—' }}</td>
              <td class="um-act">
                <el-button link type="primary" size="small" @click="openEdit(row)">{{ $t('um.editUser') }}</el-button>
                <el-button link type="danger" size="small" :disabled="!canManage" @click="dropUser(row)">
                  {{ $t('common.delete') }}
                </el-button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
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
  // 左侧当前选中的连接 / 库：**跟随它们**（用户在左边选了，这里自动带出来）
  activeConnId: { type: String, default: '' },
  activeDb: { type: String, default: '' },
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
// 没选连接：显示"去左边选一个"的提示（而不是去连一个没选的库）
const noSelection = computed(() => !selId.value)
const unsupported = computed(() => features.value.supportsUsers === false)
// 能力表里**根本没有这个字段** = 该类型连用户管理这条链路都没实现（如 Redis / MongoDB）。
// 与"有这个类型但取不到清单"区分开：前者不该去发那个注定 404 的请求，后者要如实报错。
const notImplemented = computed(() => featLoaded.value && features.value.supportsUsers === undefined)
// 能不能在这里增删改（后端 `supportsUserManage`；不支持的只给只读清单）
const canManage = computed(() => !!selId.value && features.value.supportsUsers !== false
  && features.value.supportsUserManage !== false)

const reload = async () => {
  // 没选连接就是空的 —— 不预选第一个（用户要求：没选就默认为空，
  // 而且第一个连接常常是连不上的，预选它只会先甩一条红色报错）
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
    const list = await listUsers(selId.value, props.activeDb || '')
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

// 默认**不预选**任何连接；左侧一选（或切换）就自动带出来。
// 只跟着「连接」走，不因为用户在这里手动换过就把跟随关掉 —— 左侧的选择是更明确的意图。
watch(() => props.activeConnId, (val) => {
  const id = String(val || '')
  if (id && id !== selId.value) {
    selId.value = id
    reload()
  }
}, { immediate: true })
// 左侧换了库（如从 FREE 切到 FREE.SYS）：同一个连接下也要重新取清单
watch(() => props.activeDb, () => { if (selId.value) reload() })

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
  // 不预选：等左侧选中（watch 会带出来）。连接列表为空时也不用报错。
  selId.value = String(props.activeConnId || '')
  await reload()
})
</script>

<style scoped>
.user-manage { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--dc-bg); }
/* 顶部工具条：与「编辑表结构」的标题栏同一形态 —— 独立一张小卡
   （8px 圆角 + 完整边框 + 浅底 + 轻阴影），四周留 5px 缝给圆角呼吸。 */
.um-head {
  display: flex; align-items: center; gap: 8px; padding: 8px 12px; flex-wrap: wrap;
  margin: 5px 5px 0;
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-soft);
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
}
.um-head-icon { color: #a855f7; }
.um-title { font-size: 13px; font-weight: 600; color: var(--dc-text); margin-right: 4px; }
.um-conn { width: 220px; }
.um-opt-name { float: left; }
.um-opt-type { float: right; color: var(--dc-text-dim); font-size: 12px; margin-left: 12px; }
/* 主体留 5px：与上方工具条卡同一圈缝，圆角才不会被面板边线"压平" */
.um-body { flex: 1; min-height: 0; padding: 5px; display: flex; flex-direction: column; }

/* ===== 列表：项目统一的表格样式（与字段表/数据视图同一套口径）===== */
.um-table-wrap {
  flex: 1;
  min-height: 0;
  overflow: auto;
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: 8px;
}
.um-table {
  width: 100%;
  border-collapse: separate;   /* 与吸顶表头同用不能 collapse（本项目踩过：会出现残影） */
  border-spacing: 0;
  table-layout: fixed;
  font-size: 13px;
}
.um-table th {
  position: sticky;
  top: 0;
  z-index: 3;
  background: var(--dc-bg-soft);
  color: var(--dc-text-strong);
  font-weight: 600;
  font-size: 12px;
  letter-spacing: .02em;
  text-align: left;
  padding: 5px 9px;
  border-bottom: 1px solid var(--dc-border);
  border-right: 1px solid var(--dc-border);
  white-space: nowrap;
}
.um-table td {
  padding: 1px 9px;
  height: 26px;
  background: var(--dc-bg-card);
  border-bottom: 1px solid var(--dc-border);
  border-right: 1px solid var(--dc-border);
  vertical-align: middle;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 最后一列不画竖线：否则与卡片外框并成两条 */
.um-table th:last-child,
.um-table td:last-child { border-right: none; }
.um-table tbody tr:hover td { background: var(--dc-bg-hover); }
/* 该居中的列居中：超级用户（只有"标签或一个破折号"）与操作（两个按钮）。
   操作列原来是右对齐 —— 单独贴右时比别的列"多出一截空白"，居中后与超级用户列同一条中线。 */
.um-c { text-align: center; }
.um-act { text-align: center; }
/* 表头也要居中：`.um-table th { text-align: left }` 是"类 + 元素"，优先级比 .um-c/.um-act 高一档，
   只写类名会被它压成左对齐 —— 于是"表头左、数据居中"看着就是错位的。 */
.um-table th.um-c,
.um-table th.um-act { text-align: center; }
.um-empty-row {
  height: auto !important;
  padding: 18px 0 !important;
  text-align: center;
  color: var(--dc-text-dim);
  font-size: 13px;
}
.um-empty { color: var(--dc-text-dim); font-size: 13px; padding: 24px 4px; }
.um-err { color: var(--el-color-danger); }
.um-dash { color: var(--dc-text-dim); }
</style>
