<template>
  <div class="form-tab">
    <!-- 顶部标题栏 -->
    <div class="form-tab-header">
      <div class="header-title">
        <el-icon :size="16" class="header-icon"><User /></el-icon>
        <span class="header-text">{{ info.name || props.name }}</span>
        <el-tag size="small" effect="plain" type="info">{{ connTypeLabel }}</el-tag>
      </div>
      <div class="header-actions">
        <el-button size="small" text :icon="CopyDocument" @click="copyGrants">{{ $t('oft.copySql') }}</el-button>
        <el-button size="small" type="primary" plain :icon="EditPen" @click="onEditClick">{{ $t('udv.editUser') }}</el-button>
      </div>
    </div>

    <!-- 主体：左详情 + 右预览 -->
    <div class="form-tab-body">
      <!-- 左侧详情 -->
      <div class="form-area">
        <div v-if="loading" class="empty-state">
          <el-icon class="is-loading"><Loading /></el-icon> {{ $t('common.loading') }}
        </div>
        <div v-else-if="error" class="empty-state error">
          <el-icon><CircleClose /></el-icon> {{ error }}
        </div>
        <template v-else>
          <!-- 基本信息 -->
          <div class="form-card" v-if="detailItems.length">
            <div class="card-title">
              <div class="card-title-icon"><el-icon><InfoFilled /></el-icon></div>
              <span>{{ $t('udv.basicInfo') }}</span>
            </div>
            <div class="info-grid">
              <div v-for="item in detailItems" :key="item.key" class="info-row">
                <span class="info-label">{{ item.label }}</span>
                <span class="info-value" :class="item.cls">{{ item.value }}</span>
              </div>
            </div>
          </div>
        </template>
      </div>

      <!-- 右侧 SQL 预览 -->
      <div class="sql-area">
        <div class="sql-head">
          <el-icon><DocumentCopy /></el-icon>
          <span>{{ $t('oft.preview') }}</span>
        </div>
        <div class="sql-body">
          <pre><code>{{ fullSql || $t('udv.noSql') }}</code></pre>
        </div>
        <div class="sql-foot">
          <el-button size="small" type="primary" :icon="EditPen" @click="onEditClick">{{ $t('udv.editUser') }}</el-button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { Loading, CircleClose, User, CopyDocument, EditPen, InfoFilled, DocumentCopy } from '@element-plus/icons-vue'
import { getUserInfo } from '../../api'
import { t } from '../../utils/i18n'

const props = defineProps({
  conn: { type: Object, required: true },
  database: { type: String, default: '' },
  name: { type: String, required: true }
})

const emit = defineEmits(['edit'])

const info = ref({})
const loading = ref(false)
const error = ref('')

const connTypeLabel = computed(() => {
  // 局部变量改名：本 computed 里要用 t() 翻译，同名会遮蔽
  const ty = props.conn?.type
  if (ty === 'MYSQL' || ty === 'MARIADB' || ty === 'DORIS') return t('udv.mysqlUser')
  if (ty === 'SQLSERVER') return t('udv.sqlserverUser')
  return t('udv.dbUser')
})

// 用函数而不是模块级常量：常量只在加载时求值一次，切换语言后字段名不会跟着变
const fieldLabelMap = () => ({
  // 小写形式（SQL Server 等）
  name: t('udv.fUser'), host: t('udv.fHost'), user: t('udv.fUser'), type_desc: t('udv.fType'),
  type: t('udv.fType'), default_schema: t('udv.fSchema'), default_schema_name: t('udv.fSchema'),
  disabled: t('udv.fDisabled'), is_disabled: t('udv.fDisabled'), login_name: t('udv.fLoginName'),
  // PostgreSQL 的 rol* 三项
  is_superuser: t('udv.fSuperuser'), can_login: t('udv.fCanLogin'), can_create_db: t('udv.fCanCreateDb'),
  authentication_type_desc: t('udv.fAuthType'), create_date: t('udv.fCreateDate'),
  modify_date: t('udv.fModifyDate'), comment: t('udv.fComment'), plugin: t('udv.fPlugin'),
  password_expired: t('udv.fPwdExpired'), max_questions: t('udv.fMaxQuestions'),
  max_updates: t('udv.fMaxUpdates'), max_connections: t('udv.fMaxConnections'),
  max_user_connections: t('udv.fMaxUserConnections'), account_locked: t('udv.fAccountLocked'),
  ssl_type: t('udv.fSslType'), ssl_cipher: t('udv.fSslCipher'), x509_issuer: t('udv.fX509Issuer'),
  x509_subject: t('udv.fX509Subject'),
  // MySQL 大写列名
  HOST: t('udv.fHost'), USER: t('udv.fUser'), SELECT_PRIV: t('udv.fPrivSelect'),
  INSERT_PRIV: t('udv.fPrivInsert'), UPDATE_PRIV: t('udv.fPrivUpdate'),
  DELETE_PRIV: t('udv.fPrivDelete'), CREATE_PRIV: t('udv.fPrivCreate'),
  DROP_PRIV: t('udv.fPrivDrop'), RELOAD_PRIV: t('udv.fPrivReload'),
  SHUTDOWN_PRIV: t('udv.fPrivShutdown'), PROCESS_PRIV: t('udv.fPrivProcess'),
  FILE_PRIV: t('udv.fPrivFile'), GRANT_PRIV: t('udv.fPrivGrant'),
  REFERENCES_PRIV: t('udv.fPrivReferences'), INDEX_PRIV: t('udv.fPrivIndex'),
  ALTER_PRIV: t('udv.fPrivAlter'), SHOW_DB_PRIV: t('udv.fPrivShowDb'),
  SUPER_PRIV: t('udv.fPrivSuper'), CREATE_TMP_TABLE_PRIV: t('udv.fPrivCreateTmp'),
  LOCK_TABLES_PRIV: t('udv.fPrivLockTables'), EXECUTE_PRIV: t('udv.fPrivExecute'),
  REPL_SLAVE_PRIV: t('udv.fPrivReplSlave'), REPL_CLIENT_PRIV: t('udv.fPrivReplClient'),
  CREATE_VIEW_PRIV: t('udv.fPrivCreateView'), SHOW_VIEW_PRIV: t('udv.fPrivShowView'),
  CREATE_ROUTINE_PRIV: t('udv.fPrivCreateRoutine'), ALTER_ROUTINE_PRIV: t('udv.fPrivAlterRoutine'),
  CREATE_USER_PRIV: t('udv.fPrivCreateUser'), EVENT_PRIV: t('udv.fPrivEvent'),
  TRIGGER_PRIV: t('udv.fPrivTrigger'), CREATE_TABLESPACE_PRIV: t('udv.fPrivCreateTablespace'),
  SSL_TYPE: t('udv.fSslType'), SSL_CIPHER: t('udv.fSslCipher'),
  X509_ISSUER: t('udv.fX509Issuer'), X509_SUBJECT: t('udv.fX509Subject'),
  MAX_QUESTIONS: t('udv.fMaxQuestions'), MAX_UPDATES: t('udv.fMaxUpdates'),
  MAX_CONNECTIONS: t('udv.fMaxConnections'), MAX_USER_CONNECTIONS: t('udv.fMaxUserConnections'),
  PLUGIN: t('udv.fPlugin'), AUTHENTICATION_STRING: t('udv.fAuthString'),
  PASSWORD_EXPIRED: t('udv.fPwdExpired'), PASSWORD_LAST_CHANGED: t('udv.fPwdLastChanged'),
  PASSWORD_LIFETIME: t('udv.fPwdLifetime'), ACCOUNT_LOCKED: t('udv.fAccountLocked'),
  PASSWORD_REUSE_HISTORY: t('udv.fPwdReuseHistory'), PASSWORD_REUSE_TIME: t('udv.fPwdReuseTime'),
  PASSWORD_REQUIRE_CURRENT: t('udv.fPwdRequireCurrent'), USER_ATTRIBUTES: t('udv.fUserAttributes')
})

const labelOf = (k) => {
  const m = fieldLabelMap()
  return m[k] || m[String(k).toUpperCase()] || k
}

// 「是 / 否」类字段：各家命名与取值都不同（SQL Server `disabled` 0/1、
// PostgreSQL `is_superuser` 0/1、MySQL `ACCOUNT_LOCKED` Y/N），统一按这类处理
const BOOL_FIELDS = /^(DISABLED|IS_DISABLED|IS_SUPERUSER|CAN_LOGIN|CAN_CREATE_DB|ACCOUNT_LOCKED|PASSWORD_EXPIRED|PASSWORD_REQUIRE_CURRENT)$/

const detailItems = computed(() => {
  const m = { ...info.value }
  delete m.grants
  delete m.roles
  delete m.permissions
  delete m.error
  delete m.name
  delete m.host
  delete m.HOST
  delete m.USER
  delete m.authentication_type_desc
  // 注意：不要删 create_date —— 标签表里备好了「创建时间」，
  // 之前把它从展示列表里删掉，SQL Server 用户页于是只剩三项
  delete m.createUserSql
  delete m.create_user_sql
  delete m.createUser
  const items = []
  for (const [k, v] of Object.entries(m)) {
    if (v !== null && v !== undefined && v !== '') {
      let val = v
      let cls = ''
      const KEY = k.toUpperCase()
      // 权限类字段（Y/N）→ 允许 / 禁止
      if ((v === 'Y' || v === 'N') && /_PRIV$/.test(KEY)) {
        val = v === 'Y' ? t('udv.allow') : t('udv.deny')
        cls = v === 'Y' ? 'allow' : 'deny'
      } else if (BOOL_FIELDS.test(KEY)) {
        // 布尔型字段各家取值形态不一：SQL Server 的 disabled 是 0/1、
        // PostgreSQL 的 rol* 也是 0/1、MySQL 那批是 Y/N —— 统一成「是/否」再上色，
        // 别把 0/1 裸值摊给用户看
        const yes = v === true || v === 1 || v === '1' || v === 'Y' || String(v).toLowerCase() === 'yes'
        val = yes ? t('udv.yes') : t('udv.no')
        cls = yes ? 'yes' : 'no'
      }
      items.push({ key: k, label: labelOf(k), value: val, cls })
    }
  }
  return items
})

const grants = computed(() => {
  return info.value.grants || info.value.roles || info.value.permissions || []
})

// 完整 SQL = CREATE USER 语句 + GRANT 授权语句
const fullSql = computed(() => {
  const parts = []
  const cu = info.value.createUserSql || info.value.create_user_sql || ''
  if (cu && cu.trim()) parts.push(cu.trim())
  const g = grants.value.map(s => s.trim()).filter(s => s && !s.startsWith('--'))
  if (g.length) {
    if (parts.length) parts.push('')
    parts.push(...g)
  }
  return parts.join('\n')
})

const load = async () => {
  // 缺连接时**不能静默返回**：那会渲染成一个「有标题、没内容」的空壳页，
  // 与「这个用户确实没有属性」无法区分 —— 这次就是这么误导人的
  // （根因在父组件的 openUserTab 没带 connId，已一并修掉）。
  if (!props.conn || !props.name) {
    error.value = t('udv.connUnknown')
    return
  }
  loading.value = true
  error.value = ''
  try {
    const res = await getUserInfo(props.conn.id, props.database, props.name)
    if (res.error) {
      error.value = res.error
    } else {
      info.value = res
    }
  } catch (e) {
    error.value = t('odv.loadFailedDetail', { detail: (e.message || e) })
  } finally {
    loading.value = false
  }
}

const copyGrants = async () => {
  const text = fullSql.value
  try {
    await navigator.clipboard.writeText(text || '')
    ElMessage.success(t('odv.copied'))
  } catch {
    ElMessage.warning(t('mv.copyManually'))
  }
}

const onEditClick = () => {
  // 带上本页自己的连接与库：父组件若用「当前选中的连接」会张冠李戴
  emit('edit', { name: props.name, kind: 'user', connId: props.conn?.id, database: props.database })
}

watch(() => [props.conn?.id, props.database, props.name], load, { immediate: true })
</script>

<style scoped>
/* ===== 与 ObjectFormTab / UserForm 一致的顶层布局 ===== */
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

/* 左侧详情 */
.form-area {
  flex: 1;
  overflow-y: auto;
  padding: 14px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.empty-state {
  padding: 40px;
  text-align: center;
  color: var(--dc-text-dim);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  font-size: 14px;
}
.empty-state.error { color: var(--dc-danger); }

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
  color: var(--dc-code-text);
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

/* ===== 卡片（与 UserForm 统一风格） ===== */
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
  background: rgba(79, 140, 255, .12);
  border: 1px solid rgba(79, 140, 255, .2);
  display: flex; align-items: center; justify-content: center;
  color: var(--dc-link);
  font-size: 14px;
}
/* 信息网格 */
.info-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 8px 16px;
}
.info-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 6px 0;
}
.info-label {
  font-size: 13px;
  color: var(--dc-text-dim);
  min-width: 80px;
  flex-shrink: 0;
  text-transform: uppercase;
  letter-spacing: .3px;
}
.info-value {
  font-size: 14px;
  color: var(--dc-text);
  font-family: "SF Mono", Consolas, monospace;
  word-break: break-all;
}
.info-value.allow { color: var(--dc-accent); }
.info-value.deny { color: var(--dc-text-dim); }
.info-value.yes { color: var(--dc-accent); }
.info-value.no { color: var(--dc-text-dim); }
</style>
