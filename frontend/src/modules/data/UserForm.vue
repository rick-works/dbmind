<template>
  <div class="form-tab">
    <!-- 顶部标题栏（与「编辑表结构」同一形态） -->
    <div class="form-tab-header">
      <div class="header-title">
        <el-icon :size="16" class="header-icon"><User /></el-icon>
        <span class="header-text">{{ editMode ? $t('upv.editUser') : $t('upv.newUser') }}</span>
        <el-tag v-if="form.name" size="small" effect="plain">{{ form.name }}</el-tag>
        <el-tag size="small" effect="plain" type="info">{{ conn?.name }}</el-tag>
      </div>
      <div class="header-actions">
        <el-button size="small" :icon="Close" @click="emit('close')">{{ $t('common.close') }}</el-button>
      </div>
    </div>

    <!-- 主体：上方分页签的表单 + 下方 SQL 预览（与「编辑表结构」同一套布局） -->
    <div class="form-tab-body">
      <div class="form-area">
        <!-- 字页签：纯文字（激活项主色 + 加粗），不用边框卡片 —— 与 TableDetailView 一致 -->
        <el-tabs v-model="tab" class="table-tabs user-tabs">
          <!-- ===== 基本信息 ===== -->
          <el-tab-pane :label="$t('upv.tabBasic')" name="basic">
            <div class="tab-inner">
              <!-- 该类型还没有自动授权：如实说明，别摆一堆点了会报错的控件 -->
              <el-alert v-if="!hasAnyGrantTab" type="info" :closable="false" show-icon class="grant-alert">
                <template #title>{{ $t('upv.grantUnsupported', { type: connTypeLabel }) }}</template>
              </el-alert>
              <div class="form-card">
                <div class="card-title">
                  <div class="card-title-icon"><el-icon><User /></el-icon></div>
                  <span>{{ $t('upv.basic') }}</span>
                </div>
                <div class="form-grid">
                  <el-form label-position="top" size="small">
                    <el-form-item :label="$t('upv.fieldUser')" required>
                      <el-input v-model="form.name" clearable :disabled="editMode" />
                    </el-form-item>
                  </el-form>
                  <el-form label-position="top" size="small">
                    <el-form-item :label="$t('upv.fieldPassword')" :required="!editMode">
                      <el-input v-model="form.password" type="password" show-password clearable :placeholder="editMode ? $t('upv.phPasswordKeep') : $t('upv.phPassword')" />
                    </el-form-item>
                  </el-form>
                  <el-form v-if="isMysql" label-position="top" size="small">
                    <el-form-item :label="$t('upv.fieldHost')">
                      <el-input v-model="form.host" clearable :placeholder="$t('upv.phHost')" />
                    </el-form-item>
                  </el-form>
                </div>
              </div>
            </div>
          </el-tab-pane>

          <!-- ===== 服务器权限 ===== -->
          <el-tab-pane v-if="hasServerTab" :label="serverTabLabel" name="server">
            <div class="tab-inner">
              <!-- MySQL 系：全局权限 -->
              <div v-if="isMysql" class="form-card">
                <div class="card-title">
                  <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
                  <span>{{ $t('upv.privileges') }}</span>
                </div>
                <div class="priv-groups">
                  <div v-for="group in globalPrivGroups" :key="group.key" class="priv-group">
                    <div class="priv-group-title">
                      <span>{{ group.label }}</span>
                      <span class="priv-group-count">{{ group.checked }} / {{ group.total }}</span>
                      <span class="priv-group-line" />
                      <span v-if="group.key !== 'all'" class="priv-group-actions">
                        <button :disabled="group.checked === group.total"
                                @click="setGroupPrivileges(group.items.map(p => p.value), true)">{{ $t('common.selectAll') }}</button>
                        <button :disabled="group.checked === 0"
                                @click="setGroupPrivileges(group.items.map(p => p.value), false)">{{ $t('common.clear') }}</button>
                      </span>
                    </div>
                    <el-checkbox-group v-model="form.privileges" class="perm-grid compact">
                      <el-checkbox v-for="p in group.items" :key="p.value" :label="p.value" :value="p.value">
                        {{ p.label }}
                      </el-checkbox>
                    </el-checkbox-group>
                  </div>
                </div>
              </div>

              <!-- SQL Server：服务器角色 -->
              <div v-if="isMssql" class="form-card">
                <div class="card-title">
                  <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
                  <span>{{ $t('upv.segServer') }}</span>
                </div>
                <div class="perm-grid compact">
                  <el-checkbox-group v-model="form.serverRoles">
                    <el-checkbox v-for="r in mssqlServerRoles" :key="r" :label="r" :value="r">{{ r }}</el-checkbox>
                  </el-checkbox-group>
                </div>
              </div>

              <!-- Oracle：系统权限 -->
              <div v-if="isOracle" class="form-card">
                <div class="card-title">
                  <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
                  <span>{{ $t('upv.tabSystemPrivs') }}</span>
                </div>
                <div class="perm-grid compact">
                  <el-checkbox-group v-model="form.privileges">
                    <el-checkbox v-for="p in oracleSystemPrivs" :key="p" :label="p" :value="p">{{ p }}</el-checkbox>
                  </el-checkbox-group>
                </div>
              </div>

              <!-- PostgreSQL / Kingbase：角色是集群级的 -->
              <div v-if="isPgsql" class="form-card">
                <div class="card-title">
                  <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
                  <span>{{ $t('upv.roleAssignPg') }}</span>
                </div>
                <div class="perm-grid compact">
                  <el-checkbox-group v-model="form.roles">
                    <el-checkbox v-for="r in pgRoles" :key="r" :label="r" :value="r">{{ r }}</el-checkbox>
                  </el-checkbox-group>
                </div>
              </div>
            </div>
          </el-tab-pane>

          <!-- ===== 角色（Oracle）===== -->
          <el-tab-pane v-if="hasRolesTab" :label="$t('upv.tabRoles')" name="roles">
            <div class="tab-inner">
              <div class="form-card">
                <div class="card-title">
                  <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
                  <span>{{ $t('upv.tabRoles') }}</span>
                </div>
                <div class="perm-grid compact">
                  <el-checkbox-group v-model="form.roles">
                    <el-checkbox v-for="r in oracleRoles" :key="r" :label="r" :value="r">{{ r }}</el-checkbox>
                  </el-checkbox-group>
                </div>
              </div>
            </div>
          </el-tab-pane>

          <!-- ===== 数据库权限 ===== -->
          <el-tab-pane v-if="hasDbTab" :label="$t('upv.tabDb')" name="database">
            <div class="tab-inner">
              <!-- MySQL 系：库级权限 -->
              <div v-if="isMysql" class="form-card">
                <div class="card-title">
                  <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
                  <span>{{ $t('upv.segDb') }}</span>
                </div>
                <div v-if="form.dbPrivileges.length === 0" class="empty-tip">{{ $t('upv.noDbPriv') }}</div>
                <div v-for="(row, i) in form.dbPrivileges" :key="rowKey(row)" class="priv-row">
                  <el-select v-model="row.database" size="small" :placeholder="$t('upv.phPickDb')" filterable style="width: 180px">
                    <el-option v-for="db in databases" :key="db" :label="db" :value="db" />
                  </el-select>
                  <el-tag v-if="isDbMissing(row.database)" size="small" type="warning" effect="plain">{{ $t('upv.dbMissing') }}</el-tag>
                  <el-tag v-if="isDorisSystemDb(row.database)" size="small" type="info" effect="plain">{{ $t('upv.dorisSystemDb') }}</el-tag>
                  <el-select v-model="row.privileges" size="small" multiple collapse-tags :placeholder="$t('upv.phPickPriv')" style="flex:1;min-width:200px">
                    <el-option-group v-for="group in objectPrivGroups(row)" :key="group.key" :label="group.label">
                      <el-option v-for="p in group.items" :key="p.value" :label="p.label" :value="p.value" />
                    </el-option-group>
                  </el-select>
                  <el-button type="danger" size="small" :icon="Delete" @click="removeDbPriv(i)" />
                </div>
                <el-button size="small" :icon="Plus" @click="addDbPriv" class="add-row-btn">{{ $t('upv.addDbPriv') }}</el-button>
              </div>

              <!-- SQL Server：数据库角色 -->
              <div v-if="isMssql" class="form-card">
                <div class="card-title">
                  <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
                  <span>{{ $t('upv.segDbRoles') }}</span>
                </div>
                <div class="perm-grid compact">
                  <el-checkbox-group v-model="form.roles">
                    <el-checkbox v-for="r in mssqlRoles" :key="r" :label="r" :value="r">{{ r }}</el-checkbox>
                  </el-checkbox-group>
                </div>
              </div>
            </div>
          </el-tab-pane>

          <!-- ===== 细粒度权限（库/表、Schema/对象）===== -->
          <el-tab-pane v-if="hasGranularTab" :label="granularTabLabel" name="granular">
            <div class="tab-inner">
              <!-- Oracle：对象权限（模式.对象 —— 这是 Oracle 最细的一层） -->
              <div v-if="isOracle" class="form-card">
                <div class="card-title">
                  <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
                  <span>{{ $t('upv.tabObjects') }}</span>
                </div>
                <div v-if="form.tablePrivileges.length === 0" class="empty-tip">{{ $t('upv.noObjectPrivOracle') }}</div>
                <div v-for="(row, i) in form.tablePrivileges" :key="rowKey(row)" class="priv-row">
                  <el-select v-model="row.database" size="small" :placeholder="$t('upv.phPickSchema')" filterable style="width: 160px" @change="onTablePrivDbChange(i)">
                    <el-option v-for="s in schemas" :key="s" :label="s" :value="s" />
                  </el-select>
                  <el-select v-model="row.table" size="small" :placeholder="$t('upv.phPickObject')" filterable style="width: 180px" :loading="row._loading">
                    <el-option v-for="t in row._tables || []" :key="t" :label="t" :value="t" />
                  </el-select>
                  <el-select v-model="row.privileges" size="small" multiple collapse-tags :placeholder="$t('upv.phPickPriv')" style="flex:1;min-width:180px">
                    <el-option v-for="p in oracleObjectPrivs" :key="p" :label="p" :value="p" />
                  </el-select>
                  <el-button type="danger" size="small" :icon="Delete" @click="removeTablePriv(i)" />
                </div>
                <el-button size="small" :icon="Plus" @click="addTablePriv" class="add-row-btn">{{ $t('upv.addObjectPrivOracle') }}</el-button>
              </div>

              <!-- MySQL 系：表级 -->
              <div v-if="isMysql" class="form-card">
                <div class="card-title">
                  <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
                  <span>{{ $t('upv.segTable') }}</span>
                </div>
                <div v-if="form.tablePrivileges.length === 0" class="empty-tip">{{ $t('upv.noTablePriv') }}</div>
                <div v-for="(row, i) in form.tablePrivileges" :key="rowKey(row)" class="priv-row">
                  <el-select v-model="row.database" size="small" :placeholder="$t('upv.phPickDb')" filterable style="width: 140px" @change="onTablePrivDbChange(i)">
                    <el-option v-for="db in databases" :key="db" :label="db" :value="db" />
                  </el-select>
                  <el-select v-model="row.table" size="small" :placeholder="$t('upv.phPickTable')" filterable style="width: 160px" :loading="row._loading">
                    <el-option v-for="t in row._tables || []" :key="t" :label="t" :value="t" />
                  </el-select>
                  <el-select v-model="row.privileges" size="small" multiple collapse-tags :placeholder="$t('upv.phPickPriv')" style="flex:1;min-width:180px">
                    <el-option-group v-for="group in objectPrivGroups(row)" :key="group.key" :label="group.label">
                      <el-option v-for="p in group.items" :key="p.value" :label="p.label" :value="p.value" />
                    </el-option-group>
                  </el-select>
                  <el-button type="danger" size="small" :icon="Delete" @click="removeTablePriv(i)" />
                </div>
                <el-button size="small" :icon="Plus" @click="addTablePriv" class="add-row-btn">{{ $t('upv.addTablePriv') }}</el-button>
              </div>

              <!-- SQL Server：Schema 级 + 对象级（同一页签里上下两块） -->
              <div v-if="isMssql" class="form-card">
                <div class="card-title">
                  <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
                  <span>{{ $t('upv.segSchema') }}</span>
                </div>
                <div v-if="form.schemaPrivileges.length === 0" class="empty-tip">{{ $t('upv.noSchemaPriv') }}</div>
                <div v-for="(row, i) in form.schemaPrivileges" :key="rowKey(row)" class="priv-row">
                  <el-select v-model="row.schema" size="small" :placeholder="$t('upv.phPickSchema')" filterable style="width: 180px">
                    <el-option v-for="s in schemas" :key="s" :label="s" :value="s" />
                  </el-select>
                  <el-select v-model="row.privileges" size="small" multiple collapse-tags :placeholder="$t('upv.phPickPriv')" style="flex:1;min-width:200px">
                    <el-option v-for="p in mssqlSchemaPrivs" :key="p.value" :label="p.label" :value="p.value" />
                  </el-select>
                  <el-button type="danger" size="small" :icon="Delete" @click="removeSchemaPriv(i)" />
                </div>
                <el-button size="small" :icon="Plus" @click="addSchemaPriv" class="add-row-btn">{{ $t('upv.addSchemaPriv') }}</el-button>
              </div>
              <div v-if="isMssql" class="form-card">
                <div class="card-title">
                  <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
                  <span>{{ $t('upv.segObject') }}</span>
                </div>
                <div v-if="form.objectPrivileges.length === 0" class="empty-tip">{{ $t('upv.noObjectPriv') }}</div>
                <div v-for="(row, i) in form.objectPrivileges" :key="rowKey(row)" class="priv-row">
                  <el-input v-model="row.schema" size="small" placeholder="Schema" style="width: 120px" />
                  <el-input v-model="row.object" size="small" :placeholder="$t('upv.phObject')" style="flex:1;min-width:160px" />
                  <el-select v-model="row.privileges" size="small" multiple collapse-tags :placeholder="$t('upv.phPickPriv')" style="flex:1;min-width:180px">
                    <el-option v-for="p in mssqlObjectPrivs" :key="p.value" :label="p.label" :value="p.value" />
                  </el-select>
                  <el-button type="danger" size="small" :icon="Delete" @click="removeObjectPriv(i)" />
                </div>
                <el-button size="small" :icon="Plus" @click="addObjectPriv" class="add-row-btn">{{ $t('upv.addObjectPriv') }}</el-button>
              </div>
            </div>
          </el-tab-pane>
        </el-tabs>
      </div>

      <!-- 分隔条：与「编辑表结构」同一套（向上拖 = 预览变高，双击最大/最小） -->
      <div v-show="!sqlCollapsed" class="sql-resizer"
           :title="$t('tdet.resizerTitle')"
           @mousedown.prevent="onSqlResizeStart" @dblclick="onSqlResizeDblClick"></div>

      <!-- SQL 预览：挪到**下方**（原来在右侧固定栏），点标题行折叠/展开 -->
      <div ref="sqlAreaRef" class="sql-area" :class="{ collapsed: sqlCollapsed }"
           :style="sqlCollapsed ? null : { height: sqlHeight + 'px' }">
        <div class="sql-head" @click="sqlCollapsed = !sqlCollapsed"
             :title="sqlCollapsed ? $t('tdet.expandSql') : $t('tdet.collapseSql')">
          <el-icon class="sql-fold" :class="{ folded: sqlCollapsed }"><CaretRight /></el-icon>
          <el-icon><DocumentCopy /></el-icon>
          <span>{{ $t('upv.sqlPreview') }}</span>
          <el-tag size="small" effect="plain" type="info" class="sql-state" :class="previewOk ? 'ok' : 'warn'">
            {{ previewOk ? $t('upv.statusOk') : $t('upv.statusIncomplete') }}
          </el-tag>
          <span class="sql-fold-tx">{{ sqlCollapsed ? $t('tdet.expand') : $t('tdet.collapse') }}</span>
          <span class="sql-foot-actions" @click.stop>
            <el-button size="small" text :icon="CopyDocument" @click="copySql">{{ $t('upv.copySql') }}</el-button>
          </span>
        </div>
        <div class="sql-body">
          <pre><code>{{ previewSql || $t('upv.sqlPlaceholder') }}</code></pre>
        </div>
        <div class="sql-foot">
          <el-button size="small" type="primary" :icon="CaretRight" :loading="saving" :disabled="!previewOk" @click="submit">
            {{ editMode ? $t('upv.saveChanges') : $t('upv.execCreate') }}
          </el-button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { ElMessage } from 'element-plus'
import { t } from '../../utils/i18n'
import { User, Key, CaretRight, CopyDocument, Close, DocumentCopy, Plus, Delete } from '@element-plus/icons-vue'
import { userAction, listDatabases, listTables, listSchemas } from '../../api'
import { rowKey } from '../../utils/rowKey'

const props = defineProps({
  conn: { type: Object, required: true },
  database: { type: String, default: '' },
  editMode: { type: Boolean, default: false },
  editData: { type: Object, default: () => ({}) }
})

const emit = defineEmits(['saved', 'close'])

const form = ref({
  name: '',
  password: '',
  host: '%',
  privileges: [],
  roles: [],
  dbPrivileges: [],
  tablePrivileges: [],
  serverRoles: [],
  schemaPrivileges: [],
  objectPrivileges: []
})

const saving = ref(false)
const databases = ref([])
const schemas = ref([])
// 当前页签：基本信息 / 服务器权限 / 数据库权限 / 细粒度权限
// （原来是每张卡里再放一个"分段按钮"，层级与「编辑表结构」不一致，用户要求改成页签形态）
const tab = ref('basic')
const mssqlServerRoles = [
  'sysadmin', 'securityadmin', 'serveradmin', 'setupadmin',
  'processadmin', 'diskadmin', 'dbcreator', 'bulkadmin'
]

const isMysql = computed(() => {
  // 局部名不叫 t：本文件已 import { t } 做翻译，叫 t 会在本作用域遮蔽它
  const type = props.conn?.type
  return type === 'MYSQL' || type === 'MARIADB' || type === 'DORIS'
})
// Doris 会对新建用户自动授予系统库（information_schema/mysql）的只读权限
const isDoris = computed(() => props.conn?.type === 'DORIS')
const isDorisSystemDb = (db) => isDoris.value && ['information_schema', 'mysql'].includes(String(db || '').toLowerCase())
// 库列表已加载且当前授权的库不在其中：MySQL 删库不会清理 mysql.db 的授权行，属于残留授权
const isDbMissing = (db) => {
  const d = String(db || '').trim()
  return !!d && databases.value.length > 0 && !databases.value.includes(d)
}

const isMssql = computed(() => props.conn?.type === 'SQLSERVER')
const isPgsql = computed(() => props.conn?.type === 'POSTGRESQL')
// Oracle：系统权限 / 角色 / 对象权限三层。
// ⚠️ 下面三张清单与后端 `dialect.rs` 的 ORACLE_SYSTEM_PRIVS / ORACLE_ROLES /
// ORACLE_OBJECT_PRIVS **必须一致**：后端会再校验一遍并拒绝清单外的名字，
// 这里只决定界面能勾什么 —— 不一致的表现是"勾了却报不在白名单"。
const isOracle = computed(() => props.conn?.type === 'ORACLE')
const oracleSystemPrivs = [
  'CREATE SESSION', 'CREATE TABLE', 'CREATE ANY TABLE', 'ALTER ANY TABLE', 'DROP ANY TABLE',
  'SELECT ANY TABLE', 'INSERT ANY TABLE', 'UPDATE ANY TABLE', 'DELETE ANY TABLE',
  'CREATE VIEW', 'CREATE ANY VIEW', 'CREATE SEQUENCE', 'CREATE PROCEDURE', 'CREATE ANY PROCEDURE',
  'CREATE TRIGGER', 'CREATE ANY TRIGGER', 'CREATE TYPE', 'CREATE SYNONYM', 'CREATE ANY SYNONYM',
  'CREATE DATABASE LINK', 'CREATE JOB', 'CREATE MATERIALIZED VIEW', 'CREATE ROLE', 'CREATE USER',
  'ALTER USER', 'DROP USER', 'ALTER SESSION', 'UNLIMITED TABLESPACE', 'DEBUG CONNECT SESSION',
  'QUERY REWRITE'
]
const oracleRoles = [
  'CONNECT', 'RESOURCE', 'DBA', 'EXP_FULL_DATABASE', 'IMP_FULL_DATABASE',
  'SELECT_CATALOG_ROLE', 'EXECUTE_CATALOG_ROLE', 'DELETE_CATALOG_ROLE', 'RECOVERY_CATALOG_OWNER',
  'SCHEDULER_ADMIN', 'OEM_MONITOR', 'AQ_ADMINISTRATOR_ROLE'
]
const oracleObjectPrivs = [
  'SELECT', 'INSERT', 'UPDATE', 'DELETE', 'ALTER', 'INDEX', 'REFERENCES', 'READ',
  'EXECUTE', 'DEBUG', 'FLASHBACK', 'ON COMMIT REFRESH', 'QUERY REWRITE'
]
const isClickhouse = computed(() => props.conn?.type === 'CLICKHOUSE')

// ===== 页签可见性：**按数据库类型出对应界面** =====
// MySQL 系：服务器(全局) / 数据库(库级) / 细粒度(表级)；
// SQL Server：服务器角色 / 数据库角色 / 细粒度(Schema + 对象)；
// PG、Kingbase：角色是集群级的，只有服务器权限这一页。
const hasServerTab = computed(() => isMysql.value || isMssql.value || isPgsql.value || isOracle.value)
const hasDbTab = computed(() => isMysql.value || isMssql.value)
const hasGranularTab = computed(() => isMysql.value || isMssql.value || isOracle.value)
// Oracle 的「角色」是单独一层（与系统权限、对象权限并列）
const hasRolesTab = computed(() => isOracle.value)
// 页签标题按类型取：同一层在不同家的说法不一样（Oracle 的"服务器级"其实就是系统权限）
const serverTabLabel = computed(() => (isOracle.value ? t('upv.tabSystemPrivs') : t('upv.tabServer')))
const granularTabLabel = computed(() => (
  isOracle.value ? t('upv.tabObjects')
    : isMssql.value ? t('upv.tabSchemaObject')
      : t('upv.tabTables')
))
const hasAnyGrantTab = computed(() => hasServerTab.value || hasDbTab.value)
// 类型名（给"该类型还没实现自动授权"那句提示用）
const connTypeLabel = computed(() => {
  const map = {
    MYSQL: 'MySQL', MARIADB: 'MariaDB', DORIS: 'Apache Doris', POSTGRESQL: 'PostgreSQL',
    KINGBASE: 'KingbaseES', SQLSERVER: 'SQL Server', ORACLE: 'Oracle', DM: '达梦',
    CLICKHOUSE: 'ClickHouse', SQLITE: 'SQLite', REDIS: 'Redis', MONGODB: 'MongoDB', H2: 'H2'
  }
  const key = String(props.conn?.type || '').toUpperCase()
  return map[key] || key || ''
})
const isH2 = computed(() => props.conn?.type === 'H2')
const isDerby = computed(() => props.conn?.type === 'DERBY')

// 全局权限的固定清单 = MySQL 的静态权限全集。
//
// 之前只有 14 项，而库里 `root@%`（GRANT ALL PRIVILEGES ON *.*）有 **28 项** ——
// 差额那十几项在界面上**根本没有勾选框**：看不见、也没法取消。
// 保存是「以表单为准」的，少一项就等于把那一项撤掉，所以清单必须与**
// `information_schema.user_privileges` 会返回的名字**对齐。
// （并且下面 `withExtras` 还会把清单外的名字兜底渲染出来，防止再出现"隐身"的权限。）
const mysqlPrivs = [
  { value: 'ALL PRIVILEGES', labelKey: 'upv.all' },
  { value: 'SELECT', labelKey: 'upv.select' },
  { value: 'INSERT', labelKey: 'upv.insert' },
  { value: 'UPDATE', labelKey: 'upv.update' },
  { value: 'DELETE', labelKey: 'upv.delete' },
  { value: 'CREATE', labelKey: 'upv.create' },
  { value: 'DROP', labelKey: 'upv.drop' },
  { value: 'INDEX', labelKey: 'upv.index' },
  { value: 'ALTER', labelKey: 'upv.alter' },
  { value: 'CREATE VIEW', labelKey: 'upv.createView' },
  { value: 'SHOW VIEW', labelKey: 'upv.showView' },
  { value: 'TRIGGER', labelKey: 'upv.trigger' },
  { value: 'EVENT', labelKey: 'upv.event' },
  { value: 'EXECUTE', labelKey: 'upv.execute' },
  { value: 'REFERENCES', labelKey: 'upv.references' },
  { value: 'LOCK TABLES', labelKey: 'upv.lockTables' },
  { value: 'CREATE TEMPORARY TABLES', labelKey: 'upv.createTemporaryTables' },
  { value: 'CREATE ROUTINE', labelKey: 'upv.createRoutine' },
  { value: 'ALTER ROUTINE', labelKey: 'upv.alterRoutine' },
  { value: 'CREATE USER', labelKey: 'upv.createUser' },
  { value: 'CREATE TABLESPACE', labelKey: 'upv.createTablespace' },
  { value: 'PROCESS', labelKey: 'upv.process' },
  { value: 'SHOW DATABASES', labelKey: 'upv.showDatabases' },
  { value: 'RELOAD', labelKey: 'upv.reload' },
  { value: 'SHUTDOWN', labelKey: 'upv.shutdown' },
  { value: 'SUPER', labelKey: 'upv.super' },
  { value: 'FILE', labelKey: 'upv.file' },
  { value: 'REPLICATION CLIENT', labelKey: 'upv.replicationClient' },
  { value: 'REPLICATION SLAVE', labelKey: 'upv.replicationSlave' },
  { value: 'GRANT OPTION', labelKey: 'upv.grantOption' },
  { value: 'CREATE ROLE', labelKey: 'upv.createRole' },
  { value: 'DROP ROLE', labelKey: 'upv.dropRole' }
]

// 勾选项 = 固定清单 ∪ 数据里出现的其它权限名。
//
// 固定清单负责中文标签与展示顺序，兜底负责"一个都不能隐身"：
// 只要服务器返回了它（换了大版本、MariaDB 加了别的名字、动态权限…），
// 界面就必须能看见、能取消 —— 否则保存时会把它悄悄撤掉。
const withExtras = (base, values) => {
  const known = new Set(base.map((p) => p.value))
  const extras = [...new Set(values || [])].filter((v) => v && !known.has(v))
  // 表里存的是 labelKey（见上方清单），在这里就地取出当前语言的文案；
  // 兜底项是服务器返回的原始权限名，没有译文，直接用名字本身。
  return [
    ...base.map((p) => ({ value: p.value, label: t(p.labelKey) })),
    ...extras.map((v) => ({ value: v, label: v }))
  ]
}
const globalPrivOptions = computed(() => withExtras(mysqlPrivs, form.value.privileges))
const objectPrivOptions = (row) => withExtras(objectPrivs, row?.privileges)

// 权限按用途分组 —— 三十二个勾选框平铺着没人看得下去，按「数据读写 / 对象与结构 /
// 服务器管理」分开后一眼就能定位（这也是各家客户端的老做法）。
//
// 分组**只决定怎么摆**，不改变集合：清单里有、而这里没提到的值会落进最后的「其它」组。
// 少摆一个就等于保存时把它撤掉（这是这一块反复出问题的地方，宁可多一组兜底）。
const privGroupDefs = [
  { key: 'data', labelKey: 'upv.grpData', values: ['SELECT', 'INSERT', 'UPDATE', 'DELETE'] },
  {
    key: 'object',
    labelKey: 'upv.grpObject',
    values: [
      'CREATE', 'ALTER', 'DROP', 'INDEX', 'REFERENCES', 'TRIGGER', 'EVENT',
      'CREATE VIEW', 'SHOW VIEW', 'CREATE ROUTINE', 'ALTER ROUTINE', 'EXECUTE',
      'CREATE TEMPORARY TABLES', 'LOCK TABLES'
    ]
  },
  {
    key: 'admin',
    labelKey: 'upv.grpAdmin',
    values: [
      'PROCESS', 'SHOW DATABASES', 'RELOAD', 'SHUTDOWN', 'SUPER', 'FILE',
      'CREATE USER', 'CREATE TABLESPACE', 'REPLICATION CLIENT', 'REPLICATION SLAVE',
      'CREATE ROLE', 'DROP ROLE', 'GRANT OPTION'
    ]
  }
]
const groupPrivOptions = (base) => {
  const groups = []
  const used = new Set()
  const all = base.find((p) => p.value === 'ALL PRIVILEGES')
  if (all) {
    used.add(all.value)
    groups.push({ key: 'all', label: t('upv.grpQuick'), items: [all] })
  }
  for (const def of privGroupDefs) {
    const items = base.filter((p) => def.values.includes(p.value))
    if (!items.length) continue
    items.forEach((p) => used.add(p.value))
    groups.push({ key: def.key, label: t(def.labelKey), items })
  }
  const rest = base.filter((p) => !used.has(p.value))
  if (rest.length) groups.push({ key: 'other', label: t('upv.grpOther'), items: rest })
  return groups
}
const globalPrivGroups = computed(() =>
  groupPrivOptions(globalPrivOptions.value).map((group) => ({
    ...group,
    total: group.items.length,
    checked: group.items.filter((p) => form.value.privileges.includes(p.value)).length,
  })),
)
const objectPrivGroups = (row) => groupPrivOptions(objectPrivOptions(row))

// 分组级「全选 / 清空」：只动这一组的权限项，别的组不受影响。
// 赋值一个新数组（而不是原地 splice）——`form` 是 ref，换引用才会触发重渲染。
const setGroupPrivileges = (values, checked) => {
  const set = new Set(form.value.privileges)
  values.forEach((value) => (checked ? set.add(value) : set.delete(value)))
  form.value.privileges = [...set]
}

const objectPrivs = [
  { value: 'ALL PRIVILEGES', labelKey: 'upv.all' },
  { value: 'SELECT', labelKey: 'upv.select' },
  { value: 'INSERT', labelKey: 'upv.insert' },
  { value: 'UPDATE', labelKey: 'upv.update' },
  { value: 'DELETE', labelKey: 'upv.delete' },
  { value: 'CREATE', labelKey: 'upv.create' },
  { value: 'DROP', labelKey: 'upv.drop' },
  { value: 'INDEX', labelKey: 'upv.index' },
  { value: 'ALTER', labelKey: 'upv.alter' },
  { value: 'REFERENCES', labelKey: 'upv.references' },
  { value: 'TRIGGER', labelKey: 'upv.trigger' },
  { value: 'EXECUTE', labelKey: 'upv.execute' },
  { value: 'CREATE VIEW', labelKey: 'upv.createView' },
  { value: 'SHOW VIEW', labelKey: 'upv.showView' },
  { value: 'EVENT', labelKey: 'upv.event' },
  { value: 'LOCK TABLES', labelKey: 'upv.lockTables' },
  { value: 'CREATE TEMPORARY TABLES', labelKey: 'upv.createTemporaryTables' },
  { value: 'CREATE ROUTINE', labelKey: 'upv.createRoutine' },
  { value: 'ALTER ROUTINE', labelKey: 'upv.alterRoutine' },
  // 授权能力也是会被撤销的（revoke ... grant option），必须有对应可勾项
  { value: 'GRANT OPTION', labelKey: 'upv.grantOption' }
]

const mssqlRoles = [
  'db_owner', 'db_securityadmin', 'db_accessadmin', 'db_backupoperator',
  'db_ddladmin', 'db_datawriter', 'db_datareader',
  'db_denydatawriter', 'db_denydatareader'
]

const mssqlSchemaPrivs = [
  { value: 'SELECT', labelKey: 'upv.select' },
  { value: 'INSERT', labelKey: 'upv.insert' },
  { value: 'UPDATE', labelKey: 'upv.update' },
  { value: 'DELETE', labelKey: 'upv.delete' },
  { value: 'EXECUTE', labelKey: 'upv.execute' },
  { value: 'REFERENCES', labelKey: 'upv.references' }
]

const mssqlObjectPrivs = [
  { value: 'SELECT', labelKey: 'upv.select' },
  { value: 'INSERT', labelKey: 'upv.insert' },
  { value: 'UPDATE', labelKey: 'upv.update' },
  { value: 'DELETE', labelKey: 'upv.delete' },
  { value: 'EXECUTE', labelKey: 'upv.execute' },
  { value: 'REFERENCES', labelKey: 'upv.references' },
  { value: 'VIEW DEFINITION', labelKey: 'upv.viewDefinition' }
]

const pgRoles = [
  'pg_read_all_data', 'pg_write_all_data', 'pg_read_server_files',
  'pg_write_server_files', 'pg_execute_server_program'
]

const previewOk = computed(() => {
  if (!form.value.name.trim()) return false
  if (!props.editMode && !form.value.password) return false
  return true
})

const previewSql = computed(() => {
  if (!previewOk.value) return ''
  const action = props.editMode ? 'ALTER' : 'CREATE'
  let sql = ''
  if (isMysql.value) {
    const h = form.value.host || '%'
    // 没填密码就不要拼 `ALTER USER 'x'@'%';` —— MySQL 里这是语法错误。
    // 「只改权限、不改密码」是常见操作，那时本来就只该有 revoke/grant。
    const lines = []
    if (!props.editMode) {
      lines.push(`CREATE USER '${form.value.name}'@'${h}'${form.value.password ? " IDENTIFIED BY '***'" : ''};`)
    } else if (form.value.password) {
      lines.push(`ALTER USER '${form.value.name}'@'${h}' IDENTIFIED BY '***';`)
    }
    if (props.editMode) {
      // 权限以表单为准：先清空该用户的授权，再按表单重授（后端就是这么执行的）
      lines.push(`REVOKE ALL PRIVILEGES, GRANT OPTION FROM '${form.value.name}'@'${h}';`)
    }
    sql = lines.join('\n')
    if (form.value.privileges?.length) {
      sql += `\nGRANT ${form.value.privileges.join(', ')} ON *.* TO '${form.value.name}'@'${h}';`
    }
    for (const row of form.value.dbPrivileges) {
      if (row.database && row.privileges?.length) {
        sql += `\nGRANT ${row.privileges.join(', ')} ON \`${row.database}\`.* TO '${form.value.name}'@'${h}';`
      }
    }
    for (const row of form.value.tablePrivileges) {
      if (row.database && row.table && row.privileges?.length) {
        sql += `\nGRANT ${row.privileges.join(', ')} ON \`${row.database}\`.\`${row.table}\` TO '${form.value.name}'@'${h}';`
      }
    }
  } else if (isMssql.value) {
    if (!props.editMode && form.value.password) {
      sql = `CREATE LOGIN [${form.value.name}] WITH PASSWORD='***';\nCREATE USER [${form.value.name}] FOR LOGIN [${form.value.name}];`
    } else if (props.editMode && form.value.password) {
      sql = `ALTER LOGIN [${form.value.name}] WITH PASSWORD='***';`
    }
    if (form.value.serverRoles?.length) {
      for (const r of form.value.serverRoles) {
        sql += `\nALTER SERVER ROLE [${r}] ADD MEMBER [${form.value.name}];`
      }
    }
    if (form.value.roles?.length) {
      for (const r of form.value.roles) {
        sql += `\nALTER ROLE [${r}] ADD MEMBER [${form.value.name}];`
      }
    }
    for (const row of form.value.schemaPrivileges) {
      if (row.schema && row.privileges?.length) {
        sql += `\nGRANT ${row.privileges.join(', ')} ON SCHEMA::[${row.schema}] TO [${form.value.name}];`
      }
    }
    for (const row of form.value.objectPrivileges) {
      if (row.schema && row.object && row.privileges?.length) {
        sql += `\nGRANT ${row.privileges.join(', ')} ON [${row.schema}].[${row.object}] TO [${form.value.name}];`
      }
    }
  } else if (isPgsql.value) {
    if (!props.editMode) {
      sql = `CREATE USER "${form.value.name}"`
      if (form.value.password) sql += ` WITH PASSWORD '***'`
      sql += ';'
    } else if (form.value.password) {
      sql = `ALTER USER "${form.value.name}" WITH PASSWORD '***';`
    }
    if (form.value.roles?.length) {
      for (const r of form.value.roles) {
        sql += `\nGRANT "${r}" TO "${form.value.name}";`
      }
    }
  } else if (isClickhouse.value) {
    if (!props.editMode) {
      sql = `CREATE USER \`${form.value.name}\``
      if (form.value.password) sql += ` IDENTIFIED BY '***'`
      sql += ';'
    } else if (form.value.password) {
      sql = `ALTER USER \`${form.value.name}\` IDENTIFIED BY '***';`
    }
    if (form.value.grants?.length) {
      for (const g of form.value.grants) {
        if (g && !g.startsWith('--')) sql += `\nGRANT ${g} TO \`${form.value.name}\`;`
      }
    }
  } else if (isOracle.value) {
    // Oracle：建号 / 改密 + 三层（系统权限 / 角色 / 对象权限）+ 撤销差异。
    // 与 MySQL 那段同一写法（本地预览，口令一律显示 ***）。
    const who = `"${form.value.name}"`
    const lines = []
    if (!props.editMode) {
      lines.push(`CREATE USER ${who} IDENTIFIED BY "***";`)
      lines.push(`GRANT CREATE SESSION TO ${who};`)
    } else if (form.value.password) {
      lines.push(`ALTER USER ${who} IDENTIFIED BY "***";`)
    }
    for (const r of oracleRevokes()) {
      lines.push(r.scope === 'object'
        ? `REVOKE ${r.privilege} ON "${r.owner}"."${r.object}" FROM ${who};`
        : `REVOKE ${r.privilege} FROM ${who};`)
    }
    for (const p of form.value.privileges || []) lines.push(`GRANT ${p} TO ${who};`)
    for (const r of form.value.roles || []) lines.push(`GRANT ${r} TO ${who};`)
    for (const row of form.value.tablePrivileges || []) {
      if (row.database && row.table && row.privileges?.length) {
        lines.push(`GRANT ${row.privileges.join(', ')} ON "${row.database}"."${row.table}" TO ${who};`)
      }
    }
    sql = lines.join('\n')
  } else if (isH2.value) {
    if (!props.editMode) {
      sql = `CREATE USER ${form.value.name} PASSWORD '***';`
    } else if (form.value.password) {
      sql = `ALTER USER ${form.value.name} SET PASSWORD '***';`
    }
  } else if (isDerby.value) {
    if (!props.editMode) {
      sql = `CALL SYSCS_UTIL.SYSCS_CREATE_USER('${form.value.name}', '***');`
    } else if (form.value.password) {
      sql = `CALL SYSCS_UTIL.SYSCS_RESET_PASSWORD('${form.value.name}', '***');`
    }
  } else {
    sql = t('upv.unsupportedType')
  }
  return sql
})

const copySql = async () => {
  if (!previewSql.value) return
  try {
    await navigator.clipboard.writeText(previewSql.value)
    ElMessage.success(t('upv.sqlCopied'))
  } catch { ElMessage.warning(t('sqlq.copyFailed')) }
}

const addDbPriv = () => {
  form.value.dbPrivileges.push({ database: '', privileges: [] })
}
const removeDbPriv = (i) => {
  form.value.dbPrivileges.splice(i, 1)
}

const addTablePriv = () => {
  form.value.tablePrivileges.push({ database: '', table: '', privileges: [], _tables: [], _loading: false })
}
const removeTablePriv = (i) => {
  form.value.tablePrivileges.splice(i, 1)
}

const onTablePrivDbChange = async (i) => {
  const row = form.value.tablePrivileges[i]
  row.table = ''
  row._tables = []
  if (!row.database || !props.conn) return
  row._loading = true
  try {
    const res = await listTables(props.conn.id, row.database)
    row._tables = (res || []).map(t => t.name || t)
  } catch {
    row._tables = []
  } finally {
    row._loading = false
  }
}

const addSchemaPriv = () => {
  form.value.schemaPrivileges.push({ schema: '', privileges: [] })
}
const removeSchemaPriv = (i) => {
  form.value.schemaPrivileges.splice(i, 1)
}

const addObjectPriv = () => {
  form.value.objectPrivileges.push({ schema: '', object: '', privileges: [] })
}
const removeObjectPriv = (i) => {
  form.value.objectPrivileges.splice(i, 1)
}

const loadDatabases = async () => {
  if (!props.conn) return
  if (isMysql.value) {
    try {
      const res = await listDatabases(props.conn.id)
      databases.value = (res || []).map(d => d.name || d)
    } catch {
      databases.value = []
    }
  }
  if (isMssql.value) {
    await loadSchemas()
  }
}

const loadSchemas = async () => {
  if (!props.conn || !isMssql.value) return
  try {
    const res = await listSchemas(props.conn.id, props.database)
    schemas.value = (res || []).map(s => s.name || s)
  } catch {
    schemas.value = []
  }
}

/**
 * 算「要撤销的授权」：界面里**原来有、现在没有了**的那些。
 *
 * 为什么是界面来算：打开表单时已经读过一遍现有授权（`editData.oracleRows`），
 * 用户在这份状态上增删勾选，差异只有界面知道；后端拿到的只是"最终状态"。
 * 逐条点名是因为 Oracle 没有 `revoke all privileges from u` 这种一把全撤。
 */
const oracleRevokes = () => {
  const all = (props.editData && props.editData.oracleRows) || []
  if (!Array.isArray(all) || all.length === 0) return []
  const up = (v) => String(v || '').trim().toUpperCase()
  const desired = new Set()
  for (const p of form.value.privileges || []) desired.add('global|' + up(p))
  for (const r of form.value.roles || []) desired.add('role|' + up(r))
  for (const row of form.value.tablePrivileges || []) {
    for (const p of row.privileges || []) {
      desired.add('object|' + up(row.database) + '|' + up(row.table) + '|' + up(p))
    }
  }
  return all
    .filter((r) => r && r.privilege)
    .filter((r) => {
      const key = r.scope === 'object'
        ? 'object|' + up(r.owner) + '|' + up(r.object) + '|' + up(r.privilege)
        : String(r.scope) + '|' + up(r.privilege)
      return !desired.has(key)
    })
    .map((r) => ({ scope: r.scope, privilege: r.privilege, owner: r.owner || '', object: r.object || '' }))
}

const submit = async () => {
  if (!previewOk.value) return
  saving.value = true
  try {
    const opts = {}
    if (isMysql.value) {
      opts.privileges = form.value.privileges
      opts.dbPrivileges = form.value.dbPrivileges.map(r => ({ database: r.database, privileges: r.privileges }))
      opts.tablePrivileges = form.value.tablePrivileges.map(r => ({ database: r.database, table: r.table, privileges: r.privileges }))
    }
    if (isMssql.value) {
      opts.serverRoles = form.value.serverRoles
      opts.roles = form.value.roles
      opts.schemaPrivileges = form.value.schemaPrivileges.map(r => ({ schema: r.schema, privileges: r.privileges }))
      opts.objectPrivileges = form.value.objectPrivileges.map(r => ({ schema: r.schema, object: r.object, privileges: r.privileges }))
    }
    if (isPgsql.value) {
      opts.roles = form.value.roles
    }
    if (isClickhouse.value) {
      opts.grants = form.value.grants || []
    }
    if (isOracle.value) {
      // 三层一起交：系统权限（privileges）/ 角色（roles）/ 对象权限（tablePrivileges，
      // 这里的 database 字段对 Oracle 就是**模式名**）。
      opts.privileges = form.value.privileges
      opts.roles = form.value.roles
      opts.tablePrivileges = form.value.tablePrivileges.map(r => ({ database: r.database, table: r.table, privileges: r.privileges }))
      // Oracle 没有"一把全撤"（不像 MySQL 的 revoke all privileges），所以由这里把
      // **原来有、现在没有了**的那些算出来交给后端逐条 revoke。
      opts.revoke = oracleRevokes()
    }
    const res = await userAction(props.conn.id, {
      action: props.editMode ? 'alter' : 'create',
      database: props.database,
      userName: form.value.name.trim(),
      password: form.value.password,
      host: form.value.host || '%',
      options: opts
    })
    if (res.success) {
      ElMessage.success(props.editMode ? t('upv.userUpdated') : t('upv.userCreated'))
      emit('saved', props.database)
      emit('close')
    } else {
      ElMessage.error(res.message || t('upv.opFailed'))
    }
  } catch (e) {
    ElMessage.error(t('upv.opFailedDetail', { detail: (e.message || e) }))
  } finally {
    saving.value = false
  }
}

watch(() => props.editData, (d) => {
  if (d && d.name) {
    form.value.name = d.name
    form.value.host = d.host || '%'
    form.value.password = ''
    form.value.privileges = d.privileges || []
    form.value.roles = d.roles || []
    form.value.dbPrivileges = (d.dbPrivileges || []).map(r => ({ ...r }))
    form.value.tablePrivileges = (d.tablePrivileges || []).map(r => ({ ...r, _tables: [], _loading: false }))
    form.value.serverRoles = d.serverRoles || []
    form.value.schemaPrivileges = (d.schemaPrivileges || []).map(r => ({ ...r }))
    form.value.objectPrivileges = (d.objectPrivileges || []).map(r => ({ ...r }))
    form.value.grants = d.grants || []
  } else {
    form.value = { name: '', password: '', host: '%', privileges: [], roles: [], dbPrivileges: [], tablePrivileges: [], serverRoles: [], schemaPrivileges: [], objectPrivileges: [], grants: [] }
  }
}, { immediate: true })

watch(() => props.conn?.id, loadDatabases, { immediate: true })
watch(() => props.database, loadSchemas, { immediate: true })
// ===== 底部 SQL 预览的折叠 / 拖拽（与「编辑表结构」同一套手感）=====
const SQL_H_DEFAULT = 240
const SQL_H_MIN = 96
const FORM_MIN = 160
const sqlAreaRef = ref(null)
const sqlCollapsed = ref(false)
const sqlHeight = ref(SQL_H_DEFAULT)
const sqlMaxH = () => {
  const parent = sqlAreaRef.value?.parentElement
  const available = (parent ? parent.clientHeight : Math.round(window.innerHeight * 0.7)) - FORM_MIN
  return Math.max(SQL_H_MIN, available)
}
let sqlDrag = null
const onSqlResizeStart = (e) => {
  sqlDrag = { startY: e.clientY, startH: sqlHeight.value }
  document.body.style.cursor = 'row-resize'
  document.body.style.userSelect = 'none'
  window.addEventListener('mousemove', onSqlResizeMove)
  window.addEventListener('mouseup', onSqlResizeEnd)
}
const onSqlResizeMove = (e) => {
  if (!sqlDrag) return
  const next = sqlDrag.startH + (sqlDrag.startY - e.clientY)
  sqlHeight.value = Math.min(sqlMaxH(), Math.max(SQL_H_MIN, Math.round(next)))
}
const onSqlResizeEnd = () => {
  sqlDrag = null
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
  window.removeEventListener('mousemove', onSqlResizeMove)
  window.removeEventListener('mouseup', onSqlResizeEnd)
}
const onSqlResizeDblClick = () => {
  const max = sqlMaxH()
  sqlHeight.value = sqlHeight.value >= max - 8 ? SQL_H_MIN : max
}
onBeforeUnmount(onSqlResizeEnd)

// 页签在类型之间切换时可能变得不可见（如从 MySQL 切到 Oracle）：拉回「基本信息」，
// 否则 el-tabs 会停在空内容上（看着像页面坏了）
watch([hasServerTab, hasDbTab, hasGranularTab], () => {
  const ok = ['basic', 'server', 'database', 'granular'].filter((k) => (
    k === 'basic'
    || (k === 'server' && hasServerTab.value)
    || (k === 'database' && hasDbTab.value)
    || (k === 'granular' && hasGranularTab.value)
  ))
  if (!ok.includes(tab.value)) tab.value = 'basic'
})
</script>

<style scoped>
/* ===== 与 ObjectFormTab 一致的顶层布局 ===== */
.form-tab {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--dc-bg-deep);
}
.form-tab-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border-bottom: 1px solid var(--dc-border-soft);
  background: var(--dc-bg-soft);
  flex-shrink: 0;
}
.header-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
  color: var(--dc-text);
}
.header-icon { color: var(--dc-primary); }
.header-actions { display: flex; gap: 6px; }

.form-tab-body {
  flex: 1;
  display: flex;
  overflow: hidden;
  min-height: 0;
}

/* 左侧表单 */
.form-area {
  flex: 1;
  overflow-y: auto;
  padding: 14px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

/* 右侧 SQL */
.sql-area {
  width: 380px;
  border-left: 1px solid var(--dc-border-soft);
  background: var(--dc-bg-code);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}
.sql-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 12px;
  font-size: 13px;
  color: var(--dc-text-dim);
  border-bottom: 1px solid var(--dc-border-soft);
  flex-shrink: 0;
}
.sql-state { font-size: 12px; margin-left: auto; }
.sql-state.ok { --el-tag-bg-color: var(--dc-success-wash); --el-tag-border-color: var(--dc-success); --el-tag-text-color: var(--dc-success); }
.sql-state.warn { --el-tag-bg-color: var(--dc-warning-wash); --el-tag-border-color: var(--dc-warning); --el-tag-text-color: var(--dc-warning); }
.sql-body {
  flex: 1;
  overflow: auto;
  padding: 10px 12px;
  min-height: 0;
}
.sql-body pre { margin: 0; }
.sql-body code {
  font-family: "SF Mono", Consolas, monospace;
  font-size: 13px;
  line-height: 1.7;
  color: var(--dc-link);
  white-space: pre-wrap;
  word-break: break-all;
}
.sql-foot {
  padding: 10px 12px;
  border-top: 1px solid var(--dc-border-soft);
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  flex-shrink: 0;
}

/* ===== 表单卡片（与 objectforms 统一风格） ===== */
.form-card {
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border-soft);
  border-radius: 10px;
  padding: 12px 14px;
}
.card-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 14px;
  font-weight: 600;
  color: var(--dc-text);
  margin-bottom: 10px;
}
.card-title-icon {
  width: 26px; height: 26px; border-radius: 6px;
  background: var(--dc-primary-wash);
  border: 1px solid var(--dc-primary);
  display: flex; align-items: center; justify-content: center;
  color: var(--dc-link);
  font-size: 14px;
}
.card-title-icon.purple {
  background: var(--dc-purple, rgba(168, 85, 247, .12));
  border-color: var(--dc-purple, rgba(168, 85, 247, .2));
  color: var(--dc-purple);
}
.form-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 0 16px; }

/* 权限相关 */
.perm-grid { display: flex; flex-wrap: wrap; gap: 4px; }
.perm-grid :deep(.el-checkbox) {
  margin-right: 0;
  color: var(--dc-text);
  background: rgba(255,255,255,.03);
  border: 1px solid var(--dc-border-soft);
  border-radius: 6px;
  padding: 6px 10px;
  margin: 2px;
  transition: all .15s;
}
.perm-grid :deep(.el-checkbox:hover) {
  background: rgba(79, 140, 255, .08);
  border-color: rgba(79, 140, 255, .3);
}
.perm-grid :deep(.el-checkbox.is-checked) {
  background: rgba(79, 140, 255, .12);
  border-color: rgba(79, 140, 255, .35);
}
.perm-grid :deep(.el-checkbox__label) { font-size: 13px; padding-left: 6px; }

/* 分组展示：每组一个标题 + 一行计数（勾了几项 / 共几项）+「全选 / 清空」 */
.priv-groups { display: flex; flex-direction: column; gap: 12px; }
.priv-group-title {
  display: flex; align-items: center; gap: 8px;
  font-size: 12px;
  color: var(--dc-text-dim);
  margin-bottom: 6px;
  letter-spacing: .3px;
}
/* 分隔线用真实元素而不是 ::after —— 按钮要排在它右边 */
.priv-group-line { flex: 1; height: 1px; background: var(--dc-border-soft); }
.priv-group-count { font-size: 12px; color: var(--dc-text-dim); opacity: .8; }
.priv-group-actions { display: flex; align-items: center; gap: 8px; }
.priv-group-actions button {
  appearance: none;
  border: none;
  background: none;
  padding: 0 2px;
  font-size: 12px;
  color: var(--dc-link);
  cursor: pointer;
  transition: opacity .15s;
}
.priv-group-actions button:hover:not(:disabled) { text-decoration: underline; }
.priv-group-actions button:disabled { color: var(--dc-text-dim); opacity: .45; cursor: default; }

.priv-row {
  display: flex; align-items: center; gap: 8px;
  margin-bottom: 8px;
}
.priv-row:last-child { margin-bottom: 0; }
.empty-tip {
  font-size: 13px; color: var(--dc-text-dim);
  padding: 12px 0; text-align: center;
}
.add-row-btn { margin-top: 8px; }

.priv-seg {
  display: inline-flex;
  background: var(--dc-bg-soft);
  border: 1px solid var(--dc-border-soft);
  border-radius: 8px;
  padding: 3px;
  margin-bottom: 12px;
}
.seg-btn {
  appearance: none;
  border: none;
  background: transparent;
  color: var(--dc-text-dim);
  font-size: 13px;
  padding: 5px 14px;
  border-radius: 6px;
  cursor: pointer;
  transition: all .15s;
}
.seg-btn:hover { color: var(--dc-text); }
.seg-btn.active {
  background: var(--dc-primary);
  color: var(--dc-on-primary);
  font-weight: 500;
}

/* ===== 与「编辑表结构」(TableDetailView) 统一的形态：上方页签 + 下方 SQL 预览 ===== */

/* 主体改成纵向：表单在上、预览在下（原来是左右分栏） */
.form-tab-body { flex-direction: column; }

/* 表单区：内容多，自己滚动（预览固定在下） */
.form-area { overflow: auto; padding: 0 14px 10px; gap: 0; }

/* 字页签：纯文字，激活项主色 + 加粗；去掉 Element 自带的下划线指示条 */
.user-tabs :deep(.el-tabs__header) { margin: 0 0 10px; border-bottom: none; background: transparent; }
.user-tabs :deep(.el-tabs__nav-wrap::after) { display: none; }
.user-tabs :deep(.el-tabs__active-bar) { display: none; }
.user-tabs :deep(.el-tabs__nav) { padding-top: 8px; }
.user-tabs :deep(.el-tabs__item) {
  height: 30px !important;
  line-height: 30px;
  padding: 0 14px !important;
  font-size: 13px;
  color: var(--dc-text-mid);
  border-radius: 6px !important;
  /* 必须 !important：MainView 里有一条全局 .dc-tabs .el-tabs__item.is-active 规则会命中这里 */
  background: transparent !important;
  transition: background .15s ease, color .15s ease, box-shadow .15s ease;
}
.user-tabs :deep(.el-tabs__item:hover) { color: var(--dc-text-strong); }
.user-tabs :deep(.el-tabs__item.is-active) {
  color: var(--dc-primary) !important;
  font-weight: 600;
  background: var(--dc-bg-card) !important;
  box-shadow: 0 1px 2px rgba(16, 24, 40, .10);
}
.user-tabs :deep(.el-tabs__content) { overflow: visible; }
/* 页签内容：与卡片同一列宽，卡片之间留缝 */
.tab-inner { display: flex; flex-direction: column; gap: 10px; padding-bottom: 4px; }
.grant-alert { margin-bottom: 2px; }

/* SQL 预览：通栏在下方（原来是右侧 380px 固定栏） */
.sql-area {
  width: auto;
  margin: 0 14px 12px;
  flex-shrink: 0;
  min-height: 0;
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-card);
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
}
.sql-area.collapsed { height: auto !important; }
.sql-area .sql-body { flex: 1; min-height: 0; overflow: auto; }
/* 分隔条：就是上下两块之间那道缝（6px + 透明底），悬停变主色 */
.sql-resizer { height: 6px; flex: 0 0 6px; cursor: row-resize; position: relative; z-index: 2; background: transparent; }
.sql-resizer::before {
  content: ''; position: absolute; left: 50%; top: 50%;
  transform: translate(-50%, -50%);
  width: 48px; height: 4px; border-radius: 3px;
  background: var(--dc-text-dim); opacity: .45;
  transition: background .15s ease, opacity .15s ease;
}
.sql-resizer:hover::before { background: var(--dc-primary); opacity: 1; }
.sql-fold { transition: transform .15s ease; }
.sql-fold.folded { transform: rotate(-90deg); }
.sql-fold-tx { margin-left: auto; font-size: 12px; color: var(--dc-text-dim); }
.sql-foot-actions { display: flex; align-items: center; gap: 2px; flex-shrink: 0; }
.sql-head { cursor: pointer; }
</style>
