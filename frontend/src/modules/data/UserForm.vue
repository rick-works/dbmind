<template>
  <div class="form-tab">
    <!-- 顶部标题栏 -->
    <div class="form-tab-header">
      <div class="header-title">
        <el-icon :size="16" class="header-icon"><User /></el-icon>
        <span class="header-text">{{ editMode ? $t('upv.editUser') : $t('upv.newUser') }}</span>
        <el-tag v-if="database" size="small" effect="plain" type="info">{{ database }}</el-tag>
      </div>
      <div class="header-actions">
        <el-button size="small" text :icon="CopyDocument" @click="copySql">{{ $t('upv.copySql') }}</el-button>
        <el-button size="small" :icon="Close" @click="emit('close')">{{ $t('common.close') }}</el-button>
      </div>
    </div>

    <!-- 主体：左表单 + 右预览 -->
    <div class="form-tab-body">
      <!-- 左侧表单 -->
      <div class="form-area">
        <!-- 基本信息 -->
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

        <!-- MySQL 权限分配 -->
        <div v-if="isMysql" class="form-card">
          <div class="card-title">
            <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
            <span>{{ $t('upv.privileges') }}</span>
          </div>
          <div class="priv-seg">
            <button
              v-for="s in segOptions"
              :key="s.value"
              class="seg-btn"
              :class="{ active: privSeg === s.value }"
              @click="privSeg = s.value"
            >{{ s.label }}</button>
          </div>
          <div v-if="privSeg === 'global'" class="priv-groups">
            <div v-for="group in globalPrivGroups" :key="group.key" class="priv-group">
              <div class="priv-group-title">
                <span>{{ group.label }}</span>
                <span class="priv-group-count">{{ group.checked }} / {{ group.total }}</span>
                <span class="priv-group-line" />
                <!-- 「快捷（全部权限）」那一组的勾选框本身就是开关，不再配按钮 -->
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
          <div v-if="privSeg === 'db'">
            <div v-if="form.dbPrivileges.length === 0" class="empty-tip">{{ $t('upv.noDbPriv') }}</div>
            <div v-for="(row, i) in form.dbPrivileges" :key="i" class="priv-row">
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
              <el-button size="small" :icon="Delete" @click="removeDbPriv(i)" />
            </div>
            <el-button size="small" :icon="Plus" @click="addDbPriv" class="add-row-btn">{{ $t('upv.addDbPriv') }}</el-button>
          </div>
          <div v-if="privSeg === 'table'">
            <div v-if="form.tablePrivileges.length === 0" class="empty-tip">{{ $t('upv.noTablePriv') }}</div>
            <div v-for="(row, i) in form.tablePrivileges" :key="i" class="priv-row">
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
              <el-button size="small" :icon="Delete" @click="removeTablePriv(i)" />
            </div>
            <el-button size="small" :icon="Plus" @click="addTablePriv" class="add-row-btn">{{ $t('upv.addTablePriv') }}</el-button>
          </div>
        </div>

        <!-- SQL Server 权限分配 -->
        <div v-if="isMssql" class="form-card">
          <div class="card-title">
            <div class="card-title-icon purple"><el-icon><Key /></el-icon></div>
            <span>{{ $t('upv.privilegesMssql') }}</span>
          </div>
          <div class="priv-seg">
            <button
              v-for="s in mssqlSegOptions"
              :key="s.value"
              class="seg-btn"
              :class="{ active: mssqlPrivSeg === s.value }"
              @click="mssqlPrivSeg = s.value"
            >{{ s.label }}</button>
          </div>
          <div v-if="mssqlPrivSeg === 'server'" class="perm-grid compact">
            <el-checkbox-group v-model="form.serverRoles">
              <el-checkbox v-for="r in mssqlServerRoles" :key="r" :label="r" :value="r">{{ r }}</el-checkbox>
            </el-checkbox-group>
          </div>
          <div v-if="mssqlPrivSeg === 'db'" class="perm-grid compact">
            <el-checkbox-group v-model="form.roles">
              <el-checkbox v-for="r in mssqlRoles" :key="r" :label="r" :value="r">{{ r }}</el-checkbox>
            </el-checkbox-group>
          </div>
          <div v-if="mssqlPrivSeg === 'schema'">
            <div v-if="form.schemaPrivileges.length === 0" class="empty-tip">{{ $t('upv.noSchemaPriv') }}</div>
            <div v-for="(row, i) in form.schemaPrivileges" :key="i" class="priv-row">
              <el-select v-model="row.schema" size="small" :placeholder="$t('upv.phPickSchema')" filterable style="width: 180px">
                <el-option v-for="s in schemas" :key="s" :label="s" :value="s" />
              </el-select>
              <el-select v-model="row.privileges" size="small" multiple collapse-tags :placeholder="$t('upv.phPickPriv')" style="flex:1;min-width:200px">
                <el-option v-for="p in mssqlSchemaPrivs" :key="p.value" :label="p.label" :value="p.value" />
              </el-select>
              <el-button size="small" :icon="Delete" @click="removeSchemaPriv(i)" />
            </div>
            <el-button size="small" :icon="Plus" @click="addSchemaPriv" class="add-row-btn">{{ $t('upv.addSchemaPriv') }}</el-button>
          </div>
          <div v-if="mssqlPrivSeg === 'object'">
            <div v-if="form.objectPrivileges.length === 0" class="empty-tip">{{ $t('upv.noObjectPriv') }}</div>
            <div v-for="(row, i) in form.objectPrivileges" :key="i" class="priv-row">
              <el-input v-model="row.schema" size="small" placeholder="Schema" style="width: 120px" />
              <el-input v-model="row.object" size="small" :placeholder="$t('upv.phObject')" style="flex:1;min-width:160px" />
              <el-select v-model="row.privileges" size="small" multiple collapse-tags :placeholder="$t('upv.phPickPriv')" style="flex:1;min-width:180px">
                <el-option v-for="p in mssqlObjectPrivs" :key="p.value" :label="p.label" :value="p.value" />
              </el-select>
              <el-button size="small" :icon="Delete" @click="removeObjectPriv(i)" />
            </div>
            <el-button size="small" :icon="Plus" @click="addObjectPriv" class="add-row-btn">{{ $t('upv.addObjectPriv') }}</el-button>
          </div>
        </div>

        <!-- PostgreSQL 角色分配 -->
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

      <!-- 右侧 SQL 预览 -->
      <div class="sql-area">
        <div class="sql-head">
          <el-icon><DocumentCopy /></el-icon>
          <span>{{ $t('upv.sqlPreview') }}</span>
          <el-tag size="small" effect="plain" type="info" class="sql-state" :class="previewOk ? 'ok' : 'warn'">
            {{ previewOk ? $t('upv.statusOk') : $t('upv.statusIncomplete') }}
          </el-tag>
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
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { t } from '../../utils/i18n'
import { User, Key, CaretRight, CopyDocument, Close, DocumentCopy, Plus, Delete } from '@element-plus/icons-vue'
import { userAction, listDatabases, listTables, listSchemas } from '../../api'

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
const privSeg = ref('global')
// 与其它模块同理：表里只存 labelKey，文案在下面的 computed 里现取。
// 直接把中文写进模块常量，语言会被"冻"在加载那一刻 —— 切到英文后这两个分段选择器还是中文。
const SEG_OPTIONS = [
  { labelKey: 'upv.segGlobal', value: 'global' },
  { labelKey: 'upv.segDb', value: 'db' },
  { labelKey: 'upv.segTable', value: 'table' }
]
const segOptions = computed(() => SEG_OPTIONS.map(o => ({ value: o.value, label: t(o.labelKey) })))

const mssqlPrivSeg = ref('server')
const MSSQL_SEG_OPTIONS = [
  { labelKey: 'upv.segServer', value: 'server' },
  { labelKey: 'upv.segDbRoles', value: 'db' },
  { labelKey: 'upv.segSchema', value: 'schema' },
  { labelKey: 'upv.segObject', value: 'object' }
]
const mssqlSegOptions = computed(() => MSSQL_SEG_OPTIONS.map(o => ({ value: o.value, label: t(o.labelKey) })))

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
const isClickhouse = computed(() => props.conn?.type === 'CLICKHOUSE')
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
</style>
