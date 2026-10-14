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
        <!-- 卡片：页签（35px 那一行）与内容**同一张卡**，与「编辑表结构」一个形态 -->
        <div class="tab-card">
        <el-tabs v-model="tab" class="table-tabs">
          <!-- ===== 基本信息 ===== -->
          <el-tab-pane :label="$t('upv.tabBasic')" name="basic">
            <div class="tab-inner basic-tab">
              <!-- 该类型还没有自动授权：如实说明，别摆一堆点了会报错的控件 -->
              <el-alert v-if="!hasAnyGrantTab" type="info" :closable="false" show-icon class="grant-alert">
                <template #title>{{ $t('upv.grantUnsupported', { type: connTypeLabel }) }}</template>
              </el-alert>
              <!-- 基本信息：与「编辑表结构」同一张「标签 / 值」表（行高、对齐一致） -->
              <table class="form-table">
                <colgroup>
                  <col style="width:180px" />
                  <col />
                </colgroup>
                <thead>
                  <tr><th>{{ $t('tdet.item') }}</th><th>{{ $t('tdet.value') }}</th></tr>
                </thead>
                <tbody>
                  <tr>
                    <td class="ft-label">{{ $t('upv.fieldUser') }}</td>
                    <td><el-input v-model="form.name" size="small" clearable :disabled="editMode" /></td>
                  </tr>
                  <tr>
                    <td class="ft-label">{{ $t('upv.fieldPassword') }}</td>
                    <td>
                      <el-input v-model="form.password" size="small" type="password" show-password clearable
                                :placeholder="editMode ? $t('upv.phPasswordKeep') : $t('upv.phPassword')" />
                    </td>
                  </tr>
                  <tr v-if="isMysql">
                    <td class="ft-label">{{ $t('upv.fieldHost') }}</td>
                    <td><el-input v-model="form.host" size="small" clearable :placeholder="$t('upv.phHost')" /></td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>

          <!-- ===== 服务器权限 ===== -->
          <el-tab-pane v-if="hasServerTab" :label="serverTabLabel" name="server">
            <div class="tab-inner grant-tab">
              <!-- MySQL 系：全局权限 —— 一行一个分组，**每个权限一列**，勾选即授予；
                   左列（分组名 + 计数 + 全选/清空）冻结，权限列横向滚动 -->
              <table v-if="isMysql" class="form-table mx-table">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1">{{ $t('tdet.item') }}</th>
                    <th v-for="p in globalPrivColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="group in globalPrivGroups" :key="group.key">
                    <td class="mx-fix mx-f1 ft-label">
                      <span class="ft-group-name">{{ group.label }}</span>
                      <span class="priv-group-count">{{ group.checked }} / {{ group.total }}</span>
                      <span v-if="group.key !== 'all'" class="priv-group-actions">
                        <button :disabled="group.checked === group.total"
                                @click="setGroupPrivileges(group.items.map(p => p.value), true)">{{ $t('common.selectAll') }}</button>
                        <button :disabled="group.checked === 0"
                                @click="setGroupPrivileges(group.items.map(p => p.value), false)">{{ $t('common.clear') }}</button>
                      </span>
                    </td>
                    <td v-for="p in globalPrivColumns" :key="p" class="mx-cell">
                      <!-- 这一列不属于本分组 → 留空格（分组之间不重叠，硬勾会让两组计数打架） -->
                      <el-checkbox v-if="globalGroupHas(group, p)"
                                   :model-value="hasIn('privileges', p)"
                                   @change="(v) => toggleList('privileges', p, v)" />
                    </td>
                  </tr>
                </tbody>
              </table>

              <!-- SQL Server 服务器角色 / Oracle 系统权限 / PostgreSQL 角色：
                   勾选项本身就是名字，一个一列，只有一行 -->
              <table v-else class="form-table mx-table">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1">{{ $t('tdet.item') }}</th>
                    <th v-for="p in serverColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td class="mx-fix mx-f1 ft-label">{{ serverRowLabel }}</td>
                    <td v-for="p in serverColumns" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasIn(serverModelKey, p)"
                                   @change="(v) => toggleList(serverModelKey, p, v)" />
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>

          <!-- ===== 角色（Oracle）===== -->
          <el-tab-pane v-if="hasRolesTab" :label="$t('upv.tabRoles')" name="roles">
            <div class="tab-inner grant-tab">
              <table class="form-table mx-table">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1">{{ $t('tdet.item') }}</th>
                    <th v-for="p in oracleRoles" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td class="mx-fix mx-f1 ft-label">{{ $t('upv.tabRoles') }}</td>
                    <td v-for="p in oracleRoles" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasIn('roles', p)" @change="(v) => toggleList('roles', p, v)" />
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>

          <!-- ===== 数据库权限 ===== -->
          <el-tab-pane v-if="hasDbTab" :label="$t('upv.tabDb')" name="database">
            <div class="tab-inner grant-tab">
              <!-- MySQL 系：一行一个库，每个库级权限一列 -->
              <table v-if="isMysql" class="form-table mx-table">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1">{{ $t('upv.colDb') }}</th>
                    <th v-for="p in dbPrivColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                    <th class="mx-act">{{ $t('upv.colAction') }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="form.dbPrivileges.length === 0">
                    <td :colspan="dbPrivColumns.length + 2" class="ft-empty">{{ $t('upv.noDbPriv') }}</td>
                  </tr>
                  <tr v-for="(row, i) in form.dbPrivileges" :key="rowKey(row)">
                    <td class="mx-fix mx-f1">
                      <el-select v-model="row.database" size="small" :placeholder="$t('upv.phPickDb')" filterable>
                        <el-option v-for="db in databases" :key="db" :label="db" :value="db" />
                      </el-select>
                      <el-tag v-if="isDbMissing(row.database)" size="small" type="warning" effect="plain">{{ $t('upv.dbMissing') }}</el-tag>
                      <el-tag v-if="isDorisSystemDb(row.database)" size="small" type="info" effect="plain">{{ $t('upv.dorisSystemDb') }}</el-tag>
                    </td>
                    <td v-for="p in dbPrivColumns" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasObjectPriv(row, p)" @change="(v) => toggleObjectPriv(row, p, v)" />
                    </td>
                    <td class="mx-cell">
                      <el-button type="danger" size="small" :icon="Delete" text @click="removeDbPriv(i)" />
                    </td>
                  </tr>
                  <tr class="ft-add-row">
                    <td :colspan="dbPrivColumns.length + 2">
                      <el-button size="small" text type="primary" :icon="Plus" @click="addDbPriv">{{ $t('upv.addDbPriv') }}</el-button>
                    </td>
                  </tr>
                </tbody>
              </table>

              <!-- SQL Server：数据库角色 —— 一个角色一列 -->
              <table v-if="isMssql" class="form-table mx-table">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1">{{ $t('tdet.item') }}</th>
                    <th v-for="p in mssqlRoles" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td class="mx-fix mx-f1 ft-label">{{ $t('upv.segDbRoles') }}</td>
                    <td v-for="p in mssqlRoles" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasIn('roles', p)" @change="(v) => toggleList('roles', p, v)" />
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>

          <!-- ===== 细粒度权限（库/表、Schema/对象）===== -->
          <el-tab-pane v-if="hasGranularTab" :label="granularTabLabel" name="granular">
            <div class="tab-inner grant-tab">
              <!-- Oracle：对象权限（「模式 · 名称」两列冻结，权限项当列横向滚动） -->
              <table v-if="isOracle" class="form-table mx-table">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1">{{ $t('upv.colSchema') }}</th>
                    <th class="mx-fix mx-f2">{{ $t('upv.colObject') }}</th>
                    <th v-for="p in objectPrivColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                    <th class="mx-act">{{ $t('upv.colAction') }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="form.tablePrivileges.length === 0">
                    <td :colspan="objectPrivColumns.length + 3" class="ft-empty">{{ $t('upv.noObjectPrivOracle') }}</td>
                  </tr>
                  <tr v-for="(row, i) in form.tablePrivileges" :key="rowKey(row)">
                    <td class="mx-fix mx-f1">
                      <el-select v-model="row.database" size="small" :placeholder="$t('upv.phPickSchema')" filterable @change="onTablePrivDbChange(i)">
                        <el-option v-for="s in schemas" :key="s" :label="s" :value="s" />
                      </el-select>
                    </td>
                    <td class="mx-fix mx-f2">
                      <el-select v-model="row.table" size="small" :placeholder="$t('upv.phPickObject')" filterable :loading="row._loading">
                        <el-option v-for="t in row._tables || []" :key="t" :label="t" :value="t" />
                      </el-select>
                    </td>
                    <td v-for="p in objectPrivColumns" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasObjectPriv(row, p)" @change="(v) => toggleObjectPriv(row, p, v)" />
                    </td>
                    <td class="mx-cell">
                      <el-button type="danger" size="small" :icon="Delete" text @click="removeTablePriv(i)" />
                    </td>
                  </tr>
                  <tr class="ft-add-row">
                    <td :colspan="objectPrivColumns.length + 3">
                      <el-button size="small" text type="primary" :icon="Plus" @click="addTablePriv">{{ $t('upv.addObjectPrivOracle') }}</el-button>
                    </td>
                  </tr>
                </tbody>
              </table>

              <!-- MySQL 系：表级（「数据库 · 表」两列冻结） -->
              <table v-if="isMysql" class="form-table mx-table">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1">{{ $t('upv.colDb') }}</th>
                    <th class="mx-fix mx-f2">{{ $t('upv.colTableName') }}</th>
                    <th v-for="p in tablePrivColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                    <th class="mx-act">{{ $t('upv.colAction') }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="form.tablePrivileges.length === 0">
                    <td :colspan="tablePrivColumns.length + 3" class="ft-empty">{{ $t('upv.noTablePriv') }}</td>
                  </tr>
                  <tr v-for="(row, i) in form.tablePrivileges" :key="rowKey(row)">
                    <td class="mx-fix mx-f1">
                      <el-select v-model="row.database" size="small" :placeholder="$t('upv.phPickDb')" filterable @change="onTablePrivDbChange(i)">
                        <el-option v-for="db in databases" :key="db" :label="db" :value="db" />
                      </el-select>
                    </td>
                    <td class="mx-fix mx-f2">
                      <el-select v-model="row.table" size="small" :placeholder="$t('upv.phPickTable')" filterable :loading="row._loading">
                        <el-option v-for="t in row._tables || []" :key="t" :label="t" :value="t" />
                      </el-select>
                    </td>
                    <td v-for="p in tablePrivColumns" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasObjectPriv(row, p)" @change="(v) => toggleObjectPriv(row, p, v)" />
                    </td>
                    <td class="mx-cell">
                      <el-button type="danger" size="small" :icon="Delete" text @click="removeTablePriv(i)" />
                    </td>
                  </tr>
                  <tr class="ft-add-row">
                    <td :colspan="tablePrivColumns.length + 3">
                      <el-button size="small" text type="primary" :icon="Plus" @click="addTablePriv">{{ $t('upv.addTablePriv') }}</el-button>
                    </td>
                  </tr>
                </tbody>
              </table>

              <!-- SQL Server：Schema 级（模式冻结） -->
              <table v-if="isMssql" class="form-table mx-table">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1">{{ $t('upv.colSchema') }}</th>
                    <th v-for="p in schemaPrivColumns" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                    <th class="mx-act">{{ $t('upv.colAction') }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="form.schemaPrivileges.length === 0">
                    <td :colspan="schemaPrivColumns.length + 2" class="ft-empty">{{ $t('upv.noSchemaPriv') }}</td>
                  </tr>
                  <tr v-for="(row, i) in form.schemaPrivileges" :key="rowKey(row)">
                    <td class="mx-fix mx-f1">
                      <el-select v-model="row.schema" size="small" :placeholder="$t('upv.phPickSchema')" filterable>
                        <el-option v-for="s in schemas" :key="s" :label="s" :value="s" />
                      </el-select>
                    </td>
                    <td v-for="p in schemaPrivColumns" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasObjectPriv(row, p)" @change="(v) => toggleObjectPriv(row, p, v)" />
                    </td>
                    <td class="mx-cell">
                      <el-button type="danger" size="small" :icon="Delete" text @click="removeSchemaPriv(i)" />
                    </td>
                  </tr>
                  <tr class="ft-add-row">
                    <td :colspan="schemaPrivColumns.length + 2">
                      <el-button size="small" text type="primary" :icon="Plus" @click="addSchemaPriv">{{ $t('upv.addSchemaPriv') }}</el-button>
                    </td>
                  </tr>
                </tbody>
              </table>

              <!-- SQL Server：对象级（「模式 · 对象」两列冻结，两者都可手写） -->
              <table v-if="isMssql" class="form-table mx-table">
                <thead>
                  <tr>
                    <th class="mx-fix mx-f1">{{ $t('upv.colSchema') }}</th>
                    <th class="mx-fix mx-f2">{{ $t('upv.colObject') }}</th>
                    <th v-for="p in objectPrivColumnsMssql" :key="p" class="mx-priv" :title="p">{{ p }}</th>
                    <th class="mx-act">{{ $t('upv.colAction') }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="form.objectPrivileges.length === 0">
                    <td :colspan="objectPrivColumnsMssql.length + 3" class="ft-empty">{{ $t('upv.noObjectPriv') }}</td>
                  </tr>
                  <tr v-for="(row, i) in form.objectPrivileges" :key="rowKey(row)">
                    <td class="mx-fix mx-f1"><el-input v-model="row.schema" size="small" placeholder="Schema" /></td>
                    <td class="mx-fix mx-f2"><el-input v-model="row.object" size="small" :placeholder="$t('upv.phObject')" /></td>
                    <td v-for="p in objectPrivColumnsMssql" :key="p" class="mx-cell">
                      <el-checkbox :model-value="hasObjectPriv(row, p)" @change="(v) => toggleObjectPriv(row, p, v)" />
                    </td>
                    <td class="mx-cell">
                      <el-button type="danger" size="small" :icon="Delete" text @click="removeObjectPriv(i)" />
                    </td>
                  </tr>
                  <tr class="ft-add-row">
                    <td :colspan="objectPrivColumnsMssql.length + 3">
                      <el-button size="small" text type="primary" :icon="Plus" @click="addObjectPriv">{{ $t('upv.addObjectPriv') }}</el-button>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </el-tab-pane>
        </el-tabs>
        </div>
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
        </div>
        <div v-show="!sqlCollapsed" class="sql-body">
          <pre><code>{{ previewSql || $t('upv.sqlPlaceholder') }}</code></pre>
        </div>
        <!-- 与「编辑表结构」同一排法：左边一句说明，右边一排动作按钮 -->
        <div class="sql-foot">
          <span class="sql-tip">{{ $t('upv.previewTip') }}</span>
          <div class="sql-foot-actions">
            <el-button size="small" text :icon="CopyDocument" @click="copySql">{{ $t('upv.copySql') }}</el-button>
            <el-button size="small" type="primary" :icon="CaretRight" :loading="saving" :disabled="!previewOk" @click="submit">
              {{ editMode ? $t('upv.saveChanges') : $t('upv.execCreate') }}
            </el-button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { ElMessage } from 'element-plus'
import { t } from '../../utils/i18n'
import { User, CaretRight, CopyDocument, Close, DocumentCopy, Plus, Delete } from '@element-plus/icons-vue'
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
// 表格的列 = 白名单里的权限 ∪ 现有授权里出现的其它名字（一个都不能隐身：
// 服务器返回了哪个名字，界面就必须能看见、能取消，否则保存时会把它悄悄撤掉）
const objectPrivColumns = computed(() => {
  const known = new Set(oracleObjectPrivs)
  const extras = []
  for (const row of form.value.tablePrivileges || []) {
    for (const p of row.privileges || []) {
      const name = String(p || '').trim()
      if (name && !known.has(name) && !extras.includes(name)) extras.push(name)
    }
  }
  return [...oracleObjectPrivs, ...extras]
})
const hasObjectPriv = (row, priv) => (row.privileges || []).includes(priv)
const toggleObjectPriv = (row, priv, checked) => {
  const list = row.privileges || (row.privileges = [])
  const at = list.indexOf(priv)
  if (checked && at < 0) list.push(priv)
  if (!checked && at >= 0) list.splice(at, 1)
}
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

// ===== 矩阵表（权限项当列）=====
// 列 = 固定清单 ∪ 数据里出现的名字 —— 与「一个都不能隐身」同一条原则：
// 服务器返回了哪个权限名，界面就必须有一列能看见、能取消（否则保存时会把它悄悄撤掉）。
const privColumns = (base, rows) => {
  const seen = new Set()
  const cols = []
  const push = (v) => {
    const name = String(v || '').trim()
    if (name && !seen.has(name)) { seen.add(name); cols.push(name) }
  }
  ;(base || []).forEach(push)
  ;[...(rows || [])].forEach((r) => (r && r.privileges ? r.privileges : []).forEach(push))
  return cols
}
// MySQL 全局权限：所有分组的并集（分组之间不重叠，但列取并集才能一张表放下）
const globalPrivColumns = computed(() => privColumns(globalPrivOptions.value.map((p) => p.value), [form.value]))
const globalGroupHas = (group, priv) => group.items.some((p) => p.value === priv)
// 库级 / 表级（MySQL 系）
const dbPrivColumns = computed(() => privColumns(objectPrivs.map((p) => p.value), form.value.dbPrivileges))
const tablePrivColumns = computed(() => privColumns(objectPrivs.map((p) => p.value), form.value.tablePrivileges))
// SQL Server 的 Schema 级 / 对象级
const schemaPrivColumns = computed(() => privColumns(mssqlSchemaPrivs.map((p) => p.value), form.value.schemaPrivileges))
const objectPrivColumnsMssql = computed(() => privColumns(mssqlObjectPrivs.map((p) => p.value), form.value.objectPrivileges))
// 服务器权限那一页：勾选项本身就是「角色名 / 权限名」，一个一列，只有一行
const serverColumns = computed(() => (
  isOracle.value ? oracleSystemPrivs : isMssql.value ? mssqlServerRoles : isPgsql.value ? pgRoles : []
))
const serverRowLabel = computed(() => (
  isOracle.value ? t('upv.tabSystemPrivs') : isMssql.value ? t('upv.segServer') : t('upv.roleAssignPg')
))
// 这一页写的是 form 里的哪个数组（Oracle 系统权限 → privileges，SQL Server → serverRoles，PG → roles）
const serverModelKey = computed(() => (isMssql.value ? 'serverRoles' : isPgsql.value ? 'roles' : 'privileges'))
const hasIn = (key, value) => (form.value[key] || []).includes(value)
// 换新数组而不是原地改 —— form 是 ref，换引用才会触发重渲染（本文件多处踩过）
const toggleList = (key, value, checked) => {
  const set = new Set(form.value[key] || [])
  if (checked) set.add(value); else set.delete(value)
  form.value[key] = [...set]
}

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
const SQL_H_DEFAULT = 380
// 最低 = **0**：用户要求"最低可以再低点"，于是允许把整个预览拖走（只留下面那条 6px 分隔条）。
// 不会找不回来：分隔条一直在（悬停变主色、有 tooltip"双击最大/最小"），往上拖即可还原；
// 另外点标题行的折叠态（.collapsed）也仍然保留。
const SQL_H_MIN = 0
// 上方表单**不预留**高度：往上拖可以把预览拉到顶（0 = 允许压到没有）。
// 表单区自己有 overflow: auto，压扁了也不会把内容挤丢。
const FORM_MIN = 0
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

/* ===== 卡片 + 页签一体（与「编辑表结构」同一形态）=====
   页签就是卡片的第一行（35px，带一条分隔线）：纯文字、激活项主色 + 加粗，
   内容区在它下面 —— 不再是"浮在卡片上方的一条独立页签"。 */
.tab-card {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--dc-bg-card);
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
  overflow: hidden;
}
.tab-card :deep(.el-tabs) { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.tab-card :deep(.el-tabs__header) {
  height: 35px; margin: 0; padding: 0; flex: 0 0 35px;
  border-bottom: 1px solid var(--dc-border-soft);
  background: transparent;
}
.tab-card :deep(.el-tabs__nav-wrap) { height: 35px; padding: 0 8px; }
.tab-card :deep(.el-tabs__nav-wrap::after) { display: none; }
.tab-card :deep(.el-tabs__active-bar) { display: none; }
.tab-card :deep(.el-tabs__nav) { height: 35px; display: flex; align-items: center; }
.tab-card :deep(.el-tabs__item) {
  height: 30px !important;
  line-height: 30px;
  padding: 0 14px !important;
  font-size: 13px;
  color: var(--dc-text-mid);
  border-radius: 6px !important;
  /* 必须 !important：MainView 里有一条全局 .dc-tabs .el-tabs__item.is-active 会命中这里 */
  background: transparent !important;
  transition: background .15s ease, color .15s ease;
}
.tab-card :deep(.el-tabs__item:hover) { color: var(--dc-text-strong); }
.tab-card :deep(.el-tabs__item.is-active) {
  color: var(--dc-primary) !important;
  font-weight: 600;
  background: var(--dc-bg-soft) !important;
}
/* Element 给页签内容区铺了一层淡灰蓝（--el-color-info-light-9，实测 rgb(228,233,242)）：
   表体/表头都是白的，于是**表格下面**透出这块底色，看着就像"表底下压着一个背景块"，
   与「编辑表结构」的纯白卡不一样（用户反馈"背景样式还是不一样"）。显式盖回透明。 */
.tab-card :deep(.el-tabs__content) {
  flex: 1;
  min-height: 0;
  overflow: auto;
  background: transparent !important;
}
/* 页签内容：与卡片同一列宽，卡片之间留缝 */
.tab-inner { display: flex; flex-direction: column; gap: 10px; padding: 10px 12px; }
/* 基本信息页签里是一张**表格**：不留内边距，表格直接贴到卡片内缘
   （左右那两条"框线"就是卡片自身那条边框 —— 与「编辑表结构」的字段表同一处理）。
   选择器要写 `.tab-inner.basic-tab`：两个类在同一个元素上，写成后代选择器匹配不到。 */
.tab-inner.basic-tab { padding: 0; }

/* 基本信息那张「项目 / 值」表：照「编辑表结构」的 .field-table 那套 ——
   表头浅底 + 横竖网格 + 26px 紧凑行。行高不是 padding 决定的、是行内控件决定的，
   所以控件也压到同一高度（--ctl-h）。 */
.tab-card { --grid-row-h: 26px; --ctl-h: 20px; }
.form-table {
  width: 100%;
  border-collapse: separate;   /* 不能用 collapse：与吸顶表头同用会出残影（字段表里踩过） */
  border-spacing: 0;
  table-layout: fixed;
  font-size: 13px;
}
.form-table th {
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
.form-table td {
  padding: 1px 9px;
  height: var(--grid-row-h, 26px);
  background: var(--dc-bg-card);
  border-bottom: 1px solid var(--dc-border);
  border-right: 1px solid var(--dc-border);
  vertical-align: middle;
}
/* 最后一列不画竖线：否则会与卡片外框并成两条线 */
.form-table th:last-child,
.form-table td:last-child { border-right: none; }
/* 左列「项目」：读作行标题（字号略小、颜色偏灰），与表头一样左对齐。
   权限页签里它还要放「计数 + 全选/清空」，所以允许换行。 */
.ft-label {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 4px 8px;
  font-size: 12.5px;
  color: var(--dc-text-mid);
}
.ft-group-name { color: var(--dc-text); font-weight: 500; }
/* 右列「值」里是成片的复选框：行高交给内容（表头/其它行仍是 26px 紧凑行） */
.ft-perms { padding: 4px 8px !important; height: auto; }
.ft-center { text-align: center; }
.ft-empty { padding: 8px 9px !important; color: var(--dc-text-dim); font-size: 12.5px; }
/* 权限页签：不留内边距，表格直接贴卡片内缘（与「编辑表结构」同一处理） */
.grant-alert { margin: 10px 12px 0; }
.tab-inner.grant-tab { padding: 0; }
/* 「添加…」收进表格里当最后一行：与上面几行同一套网格线，不再单独飘一个按钮 */
.ft-add-row td { padding: 3px 9px; }
.ft-add-row :deep(.el-button) { padding: 0 6px; height: var(--ctl-h, 20px); }

/* ===== 矩阵表：**权限项当列**，主体列冻结，权限列横向滚动（与各家客户端一致）=====
   列宽不能用 fixed：权限列要按内容（`CREATE TEMPORARY TABLES` 这种名字）自己撑开。
   冻结列的 left 偏移必须写死，所以第一列要用 min/max 把宽度钉住 ——
   否则第二列的 left 会随第一列实际宽度漂移，滚动时两列互相盖住。 */
.mx-table { table-layout: auto; min-width: 0; }
.mx-table th,
.mx-table td { white-space: nowrap; }
.mx-fix {
  position: sticky;
  z-index: 2;                      /* 盖住横向滚过来的权限列 */
  background: var(--dc-bg-card);   /* 必须不透明，否则下面的内容会透出来 */
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 表头那格的冻结要注意层级：表头本身已是 sticky top + z-index 3，
   冻结列表头要再高一层（4）才能同时盖住"横向滚来的表头"和"纵向滚来的数据格" */
.mx-table thead .mx-fix { background: var(--dc-bg-soft); z-index: 4; }
.mx-f1 {
  left: 0;
  width: 190px; min-width: 190px; max-width: 190px;
}
.mx-f2 {
  left: 190px;                     /* = 第一列的宽度 */
  width: 220px; min-width: 220px; max-width: 220px;
}
/* 冻结线：两列右侧各画一条，滚动时能看清冻到哪儿（`!important` 压过最后列不画竖线那条） */
.mx-f1, .mx-f2 { border-right: 1px solid var(--dc-border) !important; }
.mx-priv { text-align: center; min-width: 74px; padding-left: 6px !important; padding-right: 6px !important; }
.mx-cell { text-align: center; padding-left: 2px !important; padding-right: 2px !important; }
.mx-act { width: 48px; }
.mx-cell :deep(.el-checkbox) { margin-right: 0; height: auto; }
.mx-table :deep(.el-select) { width: 100%; }
.mx-table :deep(.el-tag) { margin-top: 2px; }
.form-table :deep(.el-input__inner) { height: var(--ctl-h, 20px); }
.form-table :deep(.el-input__wrapper) { min-height: var(--ctl-h, 20px); }
/* 底栏：左提示 + 右按钮（与「编辑表结构」同一排法） */
.sql-tip { flex: 1; font-size: 12px; color: var(--dc-text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
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

/* ===== 间距收紧（用户反馈：边距太大）===== */
/* 高度分配钉死：预览只认内联那一个 height（旧布局给过 flex: 1，不钉死时拖到很矮会被
   flex-grow 重新拉大 —— 真机实测"拖到 0 却量出 705px"）；表单区吃掉剩下的那部分。 */
/* 左右**不留内边距**：与「编辑表结构」同一口径（那里若两侧缩进，页签卡就比下方的
   SQL 预览窄一圈，实测 890 vs 914，两块面板看着宽度不一致）。 */
.form-area { padding: 5px 0 0; flex: 1 1 auto; min-height: 0; }
/* 左右**不留外边距**：与「编辑表结构」同一口径 —— 那边表单卡与预览区同宽
   （都贴着面板左右边），这里若缩进 8px 就会比上方的卡片窄一圈（实测差 16px）。 */
.sql-area { margin: 0 0 8px; flex: 0 0 auto; }
.tab-inner { gap: 8px; }

/* ===== 对象权限表格（权限做列，勾选即授予）===== */
/* 对象权限表：本身就用 .form-table（表头浅底 + 横竖网格 + 吸顶），这里只补它自己的列宽与
   「自动列宽」—— 权限列有十几列，fixed 均分会把「模式/名称」压扁，交给内容定宽更合适。 */
.obj-grid { font-size: 12px; table-layout: auto; min-width: 0; }
.obj-grid tbody tr:hover { background: var(--dc-primary-wash); }
.og-schema { min-width: 120px; }
.og-object { min-width: 160px; }
.og-priv { width: 82px; text-align: center; }
.og-cell { text-align: center; }
.obj-grid :deep(.el-select) { width: 100%; }
.obj-grid :deep(.el-checkbox) { margin-right: 0; height: auto; }
</style>
