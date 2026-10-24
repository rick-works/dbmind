<template>
  <div class="user-manage">
    <!-- 工具条：连接选择 + 刷新 + 当前页签的新增按钮。
         **没有页内标题**：外层文档页签已经写着「用户管理」，里面再写一遍是重复的
         （用户要求去掉）。页签名由下面的「用户管理 / 角色管理」两个页签承担。 -->
    <div class="um-head">
      <el-select v-model="selId" size="small" class="um-conn" filterable clearable
                 popper-class="conn-opt-popper"
                 :placeholder="$t('um.pickConn')" @change="onConnChange">
        <!-- 连接按分组列出（分组名 = 左侧对象树的目录名），和数据传输 / 数据对比同一口径 -->
        <el-option-group v-for="g in connGroups" :key="g.label" :label="g.label">
          <el-option v-for="c in g.options" :key="c.id" :label="c.name" :value="String(c.id)">
            <span class="um-opt-name">{{ c.name }}</span>
            <span class="um-opt-type">{{ typeLabel(c.type) }}</span>
          </el-option>
        </el-option-group>
      </el-select>

      <el-button size="small" :icon="Refresh" :loading="loading" @click="reload">{{ $t('common.refresh') }}</el-button>
      <!-- 只读类型（DB2 / Derby / 达梦）不摆新增按钮：禁用态的蓝色按钮仍然"看着能点"。
           两个页签各有自己的新增对象：用户管理 → 新建用户，角色管理 → 新建角色。 -->
      <el-button v-if="tab === 'users' && !readOnlyType" size="small" type="primary" :icon="Plus"
                 :disabled="!canManage" @click="openCreate">
        {{ $t('um.newUser') }}
      </el-button>
      <el-button v-else-if="tab === 'roles' && rolesSupported && !readOnlyType" size="small" type="primary"
                 :icon="Plus" :disabled="!canManage" @click="openRoleCreate">
        {{ $t('um.newRole') }}
      </el-button>
    </div>

    <!-- 只读 / 生产保护的常驻提示：**在填表单之前**就告诉用户"这次改动会被拦" ——
         点击时的右上角通知是同一件事的第二处（拦截发生在点击那一刻，不是保存那一刻）。 -->
    <div v-if="guardHint" class="um-guard">
      <span class="um-guard-mark">!</span>
      <span>{{ guardHint }}</span>
    </div>

    <!-- 主体 -->
    <div class="um-body">
      <div v-if="noSelection" class="um-empty">{{ $t('um.pickConnHint') }}</div>
      <div v-else-if="!conns.length" class="um-empty">{{ $t('um.noConn') }}</div>
      <div v-else-if="unsupported" class="um-empty">{{ $t('um.unsupported', { type: typeLabel(curConn?.type) }) }}</div>
      <div v-else-if="notImplemented" class="um-empty">{{ $t('um.notImplemented', { type: typeLabel(curConn?.type) }) }}</div>
      <!-- 错误卡：**原始错误在最上面**（等宽、可选中、可滚动），说明在下面，底部给一个重试。
           原来是一整段红字居中 —— 长 SQL 报错挤成一片，看不出"是没权限还是连不上"。 -->
      <div v-else-if="error" class="um-err-wrap">
        <div class="um-err-card">
          <div class="um-err-head">
            <span class="um-err-icon">!</span>
            <span class="um-err-title">{{ error.title }}</span>
          </div>
          <pre v-if="error.detail" class="um-err-raw">{{ error.detail }}</pre>
          <p v-if="error.hint" class="um-err-hint">{{ error.hint }}</p>
          <div class="um-err-foot">
            <el-button size="small" :icon="Refresh" :loading="loading" @click="reload">
              {{ $t('common.retry') }}
            </el-button>
          </div>
        </div>
      </div>
      <!-- 一张卡：页签条 + 表格（与「编辑表结构」同一形态 —— 页签是这张卡的标题行，
           表格与按钮都不再各自套边框，否则看起来就是几块拼起来的）。 -->
      <div v-else v-loading="loading" class="um-card">
        <el-tabs v-model="tab" class="um-tabs">
          <!-- ===== 用户管理 ===== -->
          <el-tab-pane :label="$t('um.tabUsers')" name="users">
            <!-- 列表：与数据视图/字段表**同一套表格样式**（表头浅底 + 横竖网格 + 紧凑行 + 贴卡片内缘）。
                 这里自己出表而不用 el-table：只有五列、几十行，原生表才能真正和项目其它表格一个模样
                 （el-table 自带一整套边框/斑马纹/悬停样式，要压很久还容易跟主题打架）。 -->
            <div class="um-table-wrap">
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
                    <!-- 只读类型（Derby / DB2：用户来自外部认证）空清单时别说"还没有用户" ——
                         那会让人以为能在别处建；实际这里根本管不了用户。 -->
                    <td :colspan="hasHost ? 5 : 4" class="um-empty-row">
                      {{ canManage ? $t('um.empty') : $t('um.emptyReadOnly', { type: typeLabel(curConn?.type) }) }}
                    </td>
                  </tr>
                  <tr v-for="row in rows" :key="String(row.name) + '@' + String(row.host || '')">
                    <td :title="nameTitle(row)">{{ displayName(row) }}</td>
                    <td v-if="hasHost" :title="row.host">{{ row.host }}</td>
                    <td class="um-c">
                      <el-tag v-if="row.isSuper" size="small" type="warning" effect="plain">{{ $t('um.superTag') }}</el-tag>
                      <span v-else class="um-dash">—</span>
                    </td>
                    <td :title="cellComment(row)">{{ cellComment(row) }}</td>
                    <td class="um-act">
                      <!-- 只读类型只给「查看」，删除也不摆 -->
                      <el-button link type="primary" size="small" @click="openEdit(row)">
                        {{ canManage ? $t('um.editUser') : $t('um.viewUser') }}
                      </el-button>
                      <el-button v-if="canManage" link type="danger" size="small" @click="dropUser(row)">
                        {{ $t('common.delete') }}
                      </el-button>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>

          <!-- ===== 角色管理（只在这个服务端**真的有角色**时出现） =====
               用户要求：没有角色概念的库、以及不支持角色的版本（MySQL 5.7 / 老 MariaDB）
               只留「用户管理」一个页签 —— 页签显不显示由后端 `/roles` 的 `supported` 说了算，
               不在这里写死类型名单（同一类型的版本差异只有连上去才知道）。 -->
          <el-tab-pane v-if="rolesSupported" :label="$t('um.tabRoles')" name="roles">
            <div class="um-table-wrap">
              <table class="um-table">
                <colgroup>
                  <col style="width: 260px" />
                  <col v-if="hasRoleHost" style="width: 160px" />
                  <col />
                  <col style="width: 170px" />
                </colgroup>
                <thead>
                  <tr>
                    <th>{{ $t('um.roleName') }}</th>
                    <th v-if="hasRoleHost">{{ $t('um.colHost') }}</th>
                    <th>{{ $t('um.colComment') }}</th>
                    <th class="um-act">{{ $t('um.colActions') }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="!roleRows.length">
                    <td :colspan="hasRoleHost ? 4 : 3" class="um-empty-row">{{ $t('um.emptyRoles') }}</td>
                  </tr>
                  <tr v-for="row in roleRows" :key="String(row.name) + '@' + String(row.host || '')">
                    <td :title="row.name">{{ row.name }}</td>
                    <td v-if="hasRoleHost" :title="row.host">{{ row.host }}</td>
                    <td :title="row.comment">{{ row.comment || '—' }}</td>
                    <td class="um-act">
                      <!-- 以**角色为主体**打开同一张授权表单（保存走 alter-role：只出差量 GRANT / REVOKE） -->
                      <el-button link type="primary" size="small" @click="openRoleEdit(row)">
                        {{ canManage ? $t('um.editUser') : $t('um.viewUser') }}
                      </el-button>
                      <el-button v-if="canManage" link type="danger" size="small" @click="dropRole(row)">
                        {{ $t('common.delete') }}
                      </el-button>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>
        </el-tabs>
      </div>
    </div>

    <!-- 新建角色：只有名字（MySQL 还要主机）—— 角色没有口令与属性 -->
    <el-dialog v-model="roleCreateDlg" :title="$t('um.newRole')" width="440px" append-to-body>
      <div class="role-form">
        <div class="role-field">
          <span class="role-label">{{ $t('um.roleName') }}</span>
          <el-input v-model="roleForm.name" size="small" clearable :placeholder="$t('um.roleNamePh')"
                    @keyup.enter="submitRoleCreate" />
        </div>
        <div v-if="roleHostVisible" class="role-field">
          <span class="role-label">{{ $t('um.colHost') }}</span>
          <el-input v-model="roleForm.host" size="small" clearable placeholder="%" />
        </div>
        <div class="role-tip">{{ $t('um.roleTip') }}</div>
      </div>
      <template #footer>
        <el-button size="small" @click="roleCreateDlg = false">{{ $t('common.cancel') }}</el-button>
        <el-button size="small" type="primary" :loading="roleBusy" @click="submitRoleCreate">
          {{ $t('um.newRole') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
/**
 * 用户管理（顶栏第一个入口打开的独立页面）。
 *
 * 页面形态与「编辑表结构」一致：上方一张工具条卡（连接选择 / 刷新 / 新增），下方一张卡，
 * 卡里是**两个页签**——「用户管理」与「角色管理」，分别以用户和角色为主体做授权。
 *
 * 为什么把角色也搬进来：用户和角色在多数库里是**同一套授权机制的两个主体**
 * （`GRANT … TO 用户` / `GRANT … TO 角色`，再把角色授给用户），
 * 但原来的角色只能在某个用户的「编辑 / 授权 → 角色」页签里间接管理，
 * 想给一个角色授权得先找一个用户打开表单。两者并列成页签后，
 * 「谁被授了什么」在同一处对照着看，符合实际使用顺序。
 *
 * 页签可用性由后端说了算（`/roles` 的 `supported`）：
 * · 没有角色概念的类型（SQLite / H2 / DB2 / Derby / SQL Server…）→ 只有「用户管理」；
 * · **不支持角色的版本**（MySQL 5.7 / MariaDB 10.0.4 以前，`mysql.role_edges` 都不存在）
 *   → 同样只有「用户管理」，不会摆一个点进去必报错的页签。
 *
 * 复用而不是重造：
 * · 清单走 `/users` 与 `/roles`（`dialect.users()` / `dialect.role_inventory()` 按方言给 SQL）；
 * · 新增 / 编辑（含授权）打开的还是 `UserForm`，由 MainView 开页签 —— 本组件只管列表与增删；
 * · 能力判定用 `/features` 里的 `supportsUsers / supportsUserManage`（与树上原来那套同一个来源）。
 */
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Refresh, Plus } from '@element-plus/icons-vue'
import { listConnections, listUsers, listRoles, getFeatures, userAction } from '../../api'
import { t } from '../../utils/i18n'
// 「原始错误 + 说明」两段式：后端 XError 的 hint / detail（见 api/error.rs）
import { errParts } from '../../utils/errMsg'
// NoSQL（Redis / MongoDB / Elasticsearch）没有「用户」概念，后端也没有 /features 与 /users 路由
import { isNoSql as isNoSqlType } from '../../types'
// 只读 / 生产保护的前置闸门（与 MainView 的 guardWrite 同一口径，见 utils/connGuard.js）
import { guardConnWrite, isProdConn, loadProtectProd } from '../../utils/connGuard'
import { groupConnections } from '../../utils/connGroups'

const props = defineProps({
  // 左侧当前选中的**连接**：跟随它（用户在左边选了连接，这里自动带出来）。
  // 只跟连接、不跟库 —— 用户是连接级对象，选到哪一层库都与此页无关。
  activeConnId: { type: String, default: '' },
  // 表单保存后 MainView 会 +1，用来刷新清单（跨组件的"该刷新了"信号）
  reloadTick: { type: Number, default: 0 }
})
const emit = defineEmits(['open-form', 'open-role'])

// 全部连接（下拉里**所有数据源都能选**：没有用户概念的类型选中后页面里说明，
// 与 H2 / SQLite 这些关系型完全同一套空态 —— 不在下拉里做区分）
const allConns = ref([])
const conns = computed(() => allConns.value)
/** 连接下拉按分组列出（分组名 = 左侧对象树的目录名，见 utils/connGroups） */
const connGroups = computed(() => groupConnections(conns.value))
const selId = ref('')
const rows = ref([])
const features = ref({})
const featLoaded = ref(false)
const loading = ref(false)
// null = 没有错误；否则是 { title, detail, hint }（detail 在上、hint 在下，见模板里的错误卡）
const error = ref(null)

// 当前页签：'users' / 'roles'。角色页签只在 rolesSupported 时存在，
// 换连接后如果新库没有角色，会自动退回 users（见 reload）。
const tab = ref('users')
const roleRows = ref([])
const rolesSupported = ref(false)

const typeLabel = (ty) => {
  const map = {
    ORACLE: 'Oracle', DM: '达梦', MYSQL: 'MySQL', MARIADB: 'MariaDB', DORIS: 'Apache Doris',
    POSTGRESQL: 'PostgreSQL', KINGBASE: 'KingbaseES', SQLSERVER: 'SQL Server',
    CLICKHOUSE: 'ClickHouse', SQLITE: 'SQLite', REDIS: 'Redis', MONGODB: 'MongoDB',
    ELASTICSEARCH: 'Elasticsearch', H2: 'H2', DB2: 'DB2', DERBY: 'Derby'
  }
  const key = String(ty || '').toUpperCase()
  return map[key] || key || '—'
}
const errMsg = (e) => (e && (e.message || e.toString())) || ''

const curConn = computed(() => conns.value.find(c => String(c.id) === String(selId.value)) || null)
// 有没有"主机"这一维，由数据说了算（MySQL 系才带 host），不写死类型名单
const hasHost = computed(() => rows.value.some(r => r && r.host))
// 角色同理：MySQL 的角色是「名字@主机」，其余方言只有名字
const hasRoleHost = computed(() => roleRows.value.some(r => r && r.host))
// 只有 MySQL 的角色有 host 这一维（MariaDB / Doris / PG / Oracle / CH 的角色都没有）
const roleHostVisible = computed(() => String(curConn.value?.type || '').toUpperCase() === 'MYSQL')
// 没选连接：显示"去左边选一个"的提示（而不是去连一个没选的库）
const noSelection = computed(() => !selId.value)
const unsupported = computed(() => features.value.supportsUsers === false)
// 能力表里**根本没有这个字段** = 该类型连用户管理这条链路都没实现（如 Redis / MongoDB）。
// 与"有这个类型但取不到清单"区分开：前者不该去发那个注定 404 的请求，后者要如实报错。
const notImplemented = computed(() => featLoaded.value && features.value.supportsUsers === undefined)
// 能不能在这里增删改（后端 `supportsUserManage`；不支持的只给只读清单）
const canManage = computed(() => !!selId.value && features.value.supportsUsers !== false
  && features.value.supportsUserManage !== false)

// H2 的默认管理员用户名是**空串**（后端为了能点得动，接口里给的是 `""`）。界面上
// 不摆这两个引号 —— 显示成「默认用户 / default」，备注里说明它是什么；
// 真值仍原样回传给后端（`""` → 还原空串），改名/改密/授权都照常可用。
// 只读类型：用户清单能看，但增删改都不行（DB2 / Derby / 达梦：用户由外部认证提供）
const readOnlyType = computed(() => !!selId.value && features.value.supportsUsers !== false
  && features.value.supportsUserManage === false)
const isBlankUser = (row) => curConn.value?.type === 'H2' && String(row?.name ?? '').trim() === '""'
const displayName = (row) => (isBlankUser(row) ? t('um.blankUserName') : (row?.name || ''))
const nameTitle = (row) => (isBlankUser(row) ? t('um.blankUserComment') : (row?.name || ''))
const cellComment = (row) => (isBlankUser(row) ? t('um.blankUserComment') : (row?.comment || '—'))

// 生产保护开关（缺省为开，与后端 seed_settings 一致）。判定要在**点击那一刻**就有值，
// 所以与清单一起刷新（接口本身有 30 秒缓存，不会因此多一次往返）。
const prodProtect = ref(true)
// 只读 / 生产的**常驻提示**：非空 = 这个数据源上的增删改都会被拦下（查看不受影响）。
// 这是"填完表单才被拒"的正面解法：进页面就知道，而不是等提交。
const guardHint = computed(() => {
  const c = curConn.value
  if (!c) return ''
  if (prodProtect.value && isProdConn(c)) return t('um.guardTipProd')
  if (c.readOnly) return t('um.guardTipReadOnly')
  return ''
})
// 写操作前置闸门：true = 放行；false = 已拦下（右上角通知里带「去修改数据源」）
const guard = (act) => guardConnWrite(curConn.value, act)

const reload = async () => {
  // 没选连接就是空的 —— 不预选第一个（用户要求：没选就默认为空，
  // 而且第一个连接常常是连不上的，预选它只会先甩一条红色报错）
  prodProtect.value = await loadProtectProd()
  if (!selId.value) {
    rows.value = []; roleRows.value = []; rolesSupported.value = false
    features.value = {}; error.value = null
    return
  }
  // NoSQL（Redis / MongoDB / Elasticsearch）一个请求都不发：后端连 `/features` 都没有
  // （见 api/mod.rs 的 nosql_module），发出去就是两条 404，页面上那条「接口不存在」正是这么来的。
  // 直接把能力标成 `supportsUsers: false` —— 页面于是走**和 H2 / SQLite 一模一样**的空态文案
  // （`um.unsupported`），不需要为 NoSQL 单独再写一套。
  const c = allConns.value.find(x => String(x.id) === String(selId.value))
  if (c && isNoSqlType(c.type)) {
    rows.value = []
    roleRows.value = []
    rolesSupported.value = false
    features.value = { supportsUsers: false }
    featLoaded.value = true
    error.value = null
    return
  }
  loading.value = true
  error.value = null
  try {
    features.value = await getFeatures(selId.value).catch(() => ({}))
    featLoaded.value = !!features.value && Object.keys(features.value).length > 0
    if (features.value.supportsUsers === false || (featLoaded.value && features.value.supportsUsers === undefined)) {
      rows.value = []
      roleRows.value = []
      rolesSupported.value = false
      return
    }
    // 用户是**连接级**对象（清单/表单/删除都不依赖某个库）。这里曾经跟着
    // 左侧选中的库走：树里选着 Oracle 的 system、下拉切到 MySQL，清单请求还带着
    // 那个库名 —— MySQL 直接报 Unknown database 'free.system'（真机回归）。
    // 所以一律不带库：选连接就够了，不需要选到数据库层级。
    //
    // 角色清单与用户清单**并行**取：多一次往返会明显拖慢切连接的手感。
    // 角色拿不到不算法错误（`supported:false` 就是"这个版本没有角色"），
    // 不能让它的失败把整页变成红色错误 —— 那样连用户列表都看不了了。
    const [list, roles] = await Promise.all([
      listUsers(selId.value, ''),
      listRoles(selId.value).catch(() => ({ supported: false, roles: [], message: '' }))
    ])
    rows.value = Array.isArray(list) ? list : []
    rolesSupported.value = !!roles?.supported
    roleRows.value = Array.isArray(roles?.roles) ? roles.roles : []
    // 换了连接后当前页签可能已经不存在：退回「用户管理」（否则 Element 会停在一个
    // 没有对应 pane 的名字上，内容区一片空白）
    if (!rolesSupported.value && tab.value === 'roles') tab.value = 'users'
  } catch (e) {
    rows.value = []
    roleRows.value = []
    rolesSupported.value = false
    // 不静默：拿不到清单和"确实一个用户都没有"必须能区分。
    // 后端把「没有权限」这类错误拆成了 detail（数据库原话）+ hint（该怎么做），
    // 卡片按「原话在上、说明在下」排；没拆段的普通错误，整条 message 就是原话。
    const parts = errParts(e)
    error.value = {
      title: t('um.loadFailedTitle'),
      detail: parts.detail || parts.message,
      hint: parts.hint || ''
    }
  } finally {
    loading.value = false
  }
}

const onConnChange = () => reload()

/**
 * 跟随左侧选中的连接。默认**不预选**任何连接（第一个连接常常连不上，预选它只会先甩一条
 * 红色报错）。左侧一选（或切换）就自动带出来 —— 只跟着「连接」走，不因为用户在这里
 * 手动换过就把跟随关掉（左侧的选择是更明确的意图）。
 *
 * NoSQL 也照常带出来（下拉里本来就能选到它）：页面随后显示"该类型没有用户"的说明，
 * 由 reload 里的 NoSQL 分支给能力表打上 `supportsUsers: false`。
 */
const applyActiveConn = (id) => {
  const sid = String(id || '')
  if (!sid) return
  // 连接列表还没回来：此刻还不知道它的类型，等 onMounted 拉完再判一次
  if (!allConns.value.length) return
  if (sid !== selId.value) {
    selId.value = sid
    reload()
  }
}
watch(() => props.activeConnId, (val) => applyActiveConn(val), { immediate: true })
// 左侧换了库**不再重新取清单**：用户是连接级对象，清单不依赖某个库
// （以前跟着换，树里选着 Oracle 的库、下拉切到 MySQL 就会带着那个库名去请求而报错）。

const openCreate = () => {
  if (!canManage.value) return
  // 只读 / 生产 在**点击那一刻**就拦下（与树上清空/删除表同一个顺序：不要填完再被拒）
  if (!guard(t('um.newUser'))) return
  emit('open-form', { mode: 'create', connId: selId.value, database: '' })
}
const openEdit = (row) => {
  const host = row.host || ''
  const fullName = host ? row.name + '@' + host : row.name
  // 编辑本身只是读详情（不改库），所以**不在这里拦**；拦在表单的「保存」。
  // `readOnly` = 该类型没有"改用户"的能力（如达梦只有清单与详情）—— 那时表单只给看，
  // 保存按钮置灰，免得填完才被告知"不支持"。
  emit('open-form', {
    mode: 'edit', connId: selId.value, database: '',
    name: row.name, host, fullName, readOnly: !canManage.value
  })
}
const dropUser = (row) => {
  const host = row.host || ''
  // 提示里用界面上的名字（H2 默认管理员显示「默认用户」，请求仍用真值 row.name）
  const shown = displayName(row)
  const title = t('um.deleteTitle')
  if (!guard(title)) return
  ElMessageBox.confirm(
    t('um.deleteBody', { name: shown }),
    title,
    { type: 'error', confirmButtonText: title, cancelButtonText: t('common.cancel') }
  ).then(async () => {
    try {
      const res = await userAction(selId.value, {
        action: 'drop', database: '', userName: row.name, host
      })
      if (res && res.success) {
        ElMessage.success(t('um.deleted', { name: shown }))
        reload()
      } else {
        ElMessage.error(t('um.deleteFailed', { detail: (res && res.message) || t('common.unknownError') }))
      }
    } catch (e) {
      ElMessage.error(t('um.deleteFailed', { detail: errMsg(e) }))
    }
  }).catch(() => {})
}

// ===== 角色管理 =====
// 角色以**角色为主体**打开授权表单：交给 MainView 另开一张 user-form（asRole=true），
// 保存走 `alter-role` —— 只出差量 GRANT / REVOKE，不会把角色已有的授权全删重授。
const openRoleEdit = (row) => {
  const label = row.host ? row.name + '@' + row.host : String(row.name || '')
  if (!label) return
  emit('open-role', { connId: selId.value, name: row.name, host: row.host || '', label })
}
const roleCreateDlg = ref(false)
const roleBusy = ref(false)
const roleForm = ref({ name: '', host: '%' })
const openRoleCreate = () => {
  if (!canManage.value) return
  roleForm.value = { name: '', host: '%' }
  roleCreateDlg.value = true
}
const submitRoleCreate = async () => {
  if (roleBusy.value) return
  const name = String(roleForm.value.name || '').trim()
  if (!name) {
    ElMessage.warning(t('um.roleNeedName'))
    return
  }
  // 只有 MySQL 的角色带 host；MariaDB / PG / Oracle / CH / Doris 多写 host 就是语法错
  const host = roleHostVisible.value ? (String(roleForm.value.host || '').trim() || '%') : ''
  if (!guard(t('um.newRole'))) return
  roleBusy.value = true
  try {
    const res = await userAction(selId.value, {
      action: 'create-role', database: '', userName: name, host
    })
    if (res && res.success) {
      ElMessage.success(t('um.roleCreated', { name: roleHostVisible.value ? `${name}@${host}` : name }))
      roleCreateDlg.value = false
      reload()
    } else {
      ElMessage.error(t('um.roleCreateFailed', { detail: (res && res.message) || t('common.unknownError') }))
    }
  } catch (e) {
    ElMessage.error(t('um.roleCreateFailed', { detail: errMsg(e) }))
  } finally {
    roleBusy.value = false
  }
}
const dropRole = (row) => {
  const name = String(row.name || '').trim()
  if (!name) return
  const host = row.host || ''
  const title = t('um.deleteRoleTitle')
  if (!guard(title)) return
  ElMessageBox.confirm(
    t('um.deleteRoleBody', { name: host ? `${name}@${host}` : name }),
    title,
    { type: 'error', confirmButtonText: title, cancelButtonText: t('common.cancel') }
  ).then(async () => {
    try {
      const res = await userAction(selId.value, {
        action: 'drop-role', database: '', userName: name, host
      })
      if (res && res.success) {
        ElMessage.success(t('um.roleDeleted', { name: host ? `${name}@${host}` : name }))
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

onMounted(async () => {
  // 「编辑数据源」保存后（改只读/分组等）：自己重拉列表，curConn（computed）与只读横幅即时刷新
  try {
    const list = await listConnections()
    allConns.value = Array.isArray(list) ? list : []
  } catch (e) {
    allConns.value = []
  }
  // 列表就绪后才知道每个连接的类型：这时才跟随左侧选中（NoSQL 会被跳过并给说明）。
  // 注意 watch 的 immediate 那次跑在列表到位之前，什么也判不了。
  applyActiveConn(props.activeConnId)
  // 没选/被跳过时，reload 只做清空（不发请求）；这里再走一次是为把"生产保护"等状态刷到位
  await reload()
  // 设置页保存「安全与会话」后立即重读生产保护开关：常驻提示与点击闸门同步刷新，
  // 不等页面重载（用户实测：关掉保护后不刷新浏览器仍会被旧值拦下）
  window.addEventListener('dbmind-safety-changed', onSafetyChanged)
  // 「编辑数据源」保存后（改只读/分组等）：这里**自己拉连接列表**，不在 MainView 的
  // 响应式链上 —— 收到广播后重拉列表，curConn（computed）与只读横幅即時刷新
  window.addEventListener('dbmind-conns-changed', onConnsChanged)
})
onBeforeUnmount(() => {
  window.removeEventListener('dbmind-safety-changed', onSafetyChanged)
  window.removeEventListener('dbmind-conns-changed', onConnsChanged)
})
const onSafetyChanged = async () => { prodProtect.value = await loadProtectProd() }
const onConnsChanged = async () => {
  try {
    const list = await listConnections()
    allConns.value = Array.isArray(list) ? list : []
    // 列表变了：当前选中的连接可能已被删除、或改成了别的类型 —— 重新判一次
    applyActiveConn(props.activeConnId)
  } catch { /* 保留旧列表 */ }
}
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
.um-conn { width: 260px; }
.um-opt-name { float: left; }
.um-opt-type { float: right; color: var(--dc-text-dim); font-size: 12px; margin-left: 12px; }
/* 只读 / 生产 的常驻提示条：与工具条同一圈缝、同一圆角
   （不是红色：这不是"出错了"，是"这里的改动会被拦"）。
   配色用 --dc-warn-* 分主题变量：深色下低透明度融底 + 亮字，不再用 Element
   的浅黄底（那是浅色主题的值，深色下刺眼）。 */
.um-guard {
  display: flex; align-items: center; gap: 7px;
  margin: 5px 5px 0;
  padding: 6px 11px;
  border: 1px solid var(--dc-warn-border);
  border-radius: 8px;
  background: var(--dc-warn-wash);
  color: var(--dc-warn-text);
  font-size: 12.5px;
  line-height: 1.5;
}
.um-guard-mark {
  flex: none;
  width: 15px; height: 15px; line-height: 15px; text-align: center;
  border-radius: 50%;
  background: var(--dc-warning);
  color: #fff; font-size: 11px; font-weight: 700;
}
/* 主体留 5px：与上方工具条卡同一圈缝，圆角才不会被面板边线"压平" */
.um-body { flex: 1; min-height: 0; padding: 5px; display: flex; flex-direction: column; }

/* ===== 主体卡：页签条就是这张卡的标题行（与「编辑表结构」同一形态）===== */
.um-card {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-card);
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
  overflow: hidden;
}
.um-tabs { flex: 1; min-height: 0; display: flex; flex-direction: column; }
/* 卡内页签条：**与「编辑表结构」完全同款** —— 不铺整行灰带、不画下划线滑块，
   页签自己是一个 6px 圆角的胶囊（激活项白底 + 轻阴影 + 主色加粗）。
   之前这里是"灰底通栏 + 方角高亮块"，与表结构页并排看就是两种控件（用户截图指出）。 */
.um-tabs :deep(.el-tabs__header) {
  margin: 0;
  padding: 4px 10px;
  background: transparent;
  /* 不画下边框：表结构页的页签就"住在表格第一行"，分隔交给表格表头自己的线。
     这里多一条线会变成"页签条 + 表格"两块，而不是一张卡。 */
  border-bottom: none;
}
.um-tabs :deep(.el-tabs__nav-wrap::after) { display: none; }
.um-tabs :deep(.el-tabs__active-bar) { display: none; }
.um-tabs :deep(.el-tabs__item) {
  /* height / border-radius / background 都必须 !important：MainView 有一批全局
     .dc-tabs 规则会命中这里的页签项（实测把圆角压成 0、把激活项刷成浅紫方块）。 */
  height: 30px !important;
  line-height: 30px;
  padding: 0 14px !important;
  font-size: 13px;
  color: var(--dc-text-mid);
  border-radius: 6px !important;
  background: transparent !important;
  transition: background .15s ease, color .15s ease, box-shadow .15s ease;
}
.um-tabs :deep(.el-tabs__item:hover) { color: var(--dc-text-strong); }
.um-tabs :deep(.el-tabs__item.is-active) {
  color: var(--dc-primary) !important;
  font-weight: 600;
  background: var(--dc-bg-card) !important;
  box-shadow: 0 1px 2px rgba(16, 24, 40, .10);
}
.um-tabs :deep(.el-tabs__content) { flex: 1; min-height: 0; padding: 0; overflow: hidden; }
.um-tabs :deep(.el-tab-pane) { height: 100%; display: flex; flex-direction: column; }

/* ===== 列表：项目统一的表格样式（与字段表/数据视图同一套口径）=====
   表格直接贴在卡片内缘：外框由卡片负责，这里不再套一层边框（否则出现"框中框"）。 */
.um-table-wrap {
  flex: 1;
  min-height: 0;
  overflow: auto;
  background: var(--dc-bg-card);
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
/* 该居中的列居中：超级用户（只有"标签或一个破折号"）与操作（两个按钮）。 */
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
/* 空态：**整块居中**。
   原来是贴着工具条的一行小字（左上角），下面一大片空白 —— 用户反馈"这个位置优化下"。
   用 min-height 视口比例而不是 flex:1：这些空态 div 与后面的表格是兄弟节点，
   父容器不一定是 flex 列，靠 min-height 才在两种结构下都居中。 */
.um-empty {
  color: var(--dc-text-mid);
  font-size: 13.5px;
  line-height: 1.8;
  padding: 48px 24px;
  min-height: 52vh;
  display: flex;
  align-items: center;
  justify-content: center;
  text-align: center;
  max-width: 560px;
  margin: 0 auto;
}
/* ===== 错误卡 =====
   两段式：原始错误（等宽、浅红底、可选中/滚动）在上，说明在下，底部一个重试。
   左边一条红色竖条承担"这是错误"的信号，正文不再整片染红（长报错染红后很难读）。 */
.um-err-wrap {
  display: flex;
  justify-content: center;
  align-items: flex-start;
  padding: 40px 24px;
  min-height: 40vh;
}
.um-err-card {
  width: 100%;
  max-width: 620px;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border);
  border-left: 3px solid var(--el-color-danger);
  border-radius: 10px;
  padding: 16px 18px 10px;
  box-shadow: var(--dc-shadow-sm, 0 2px 10px rgba(0, 0, 0, 0.06));
}
.um-err-head { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
.um-err-icon {
  flex: none;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--el-color-danger);
  color: #fff;
  font-size: 12px;
  font-weight: 700;
  line-height: 1;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.um-err-title { font-size: 14px; font-weight: 600; color: var(--dc-text); }
.um-err-raw {
  margin: 0 0 12px;
  padding: 10px 12px;
  max-height: 200px;
  overflow: auto;
  background: var(--el-color-danger-light-9, #fef0f0);
  border-radius: 6px;
  font-family: ui-monospace, Menlo, Consolas, 'Courier New', monospace;
  font-size: 12.5px;
  line-height: 1.65;
  color: var(--dc-text-mid);
  white-space: pre-wrap;
  word-break: break-word;
  user-select: text;
}
.um-err-hint { margin: 0; font-size: 13px; line-height: 1.75; color: var(--dc-text-mid); }
.um-err-foot {
  display: flex;
  justify-content: flex-end;
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px dashed var(--dc-border);
}
.um-dash { color: var(--dc-text-dim); }

/* ===== 新建角色对话框 ===== */
.role-form { display: flex; flex-direction: column; gap: 10px; }
.role-field { display: flex; align-items: center; gap: 8px; }
.role-label { flex: none; width: 150px; font-size: 12.5px; color: var(--dc-text-mid); }
.role-tip { font-size: 12px; color: var(--dc-text-dim); line-height: 1.6; }
</style>
