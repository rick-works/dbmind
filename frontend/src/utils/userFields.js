// 用户字段的「标签表 + 取值归一」：用户详情页与编辑表单的「基本信息」共用一份。
//
// 为什么抽成公共模块：各家系统表的字段名与取值形态都不一样 ——
//   MySQL 系    : mysql.user 的 t.*（几十列，键名混杂大小写，取值 0/1 与 Y/N 都有）
//   Oracle / DM : dba_users（ACCOUNT_STATUS / DEFAULT_TABLESPACE / PROFILE，驱动回全大写键）
//   SQL Server  : sys.database_principals（type_desc / login_name / default_schema / disabled）
//   PG / Kingbase: pg_roles（rolsuper / rolcanlogin / rolcreatedb，取值 0/1）
//   ClickHouse  : system.users（storage / auth_type / host_ip / host_names）
// 原来只有用户详情页做了归一，编辑表单的「基本信息」只取了 name / host ——
// 于是不管哪家的库，表单里永远只有「用户名 / 密码 / 主机」三行
// （用户反馈"基本信息 也根据每个数据库的类型 加全"）。
import { t } from './i18n'

// 用函数而不是模块级常量：常量只在加载时求值一次，切换语言后字段名不会跟着变
export const userFieldLabelMap = () => ({
  // 小写形式（SQL Server 等）
  name: t('udv.fUser'), host: t('udv.fHost'), user: t('udv.fUser'), type_desc: t('udv.fType'),
  type: t('udv.fType'), default_schema: t('udv.fSchema'), default_schema_name: t('udv.fSchema'),
  disabled: t('udv.fDisabled'), is_disabled: t('udv.fDisabled'), login_name: t('udv.fLoginName'),
  // PostgreSQL 的 rol* 三项
  is_superuser: t('udv.fSuperuser'), can_login: t('udv.fCanLogin'), can_create_db: t('udv.fCanCreateDb'),
  // ClickHouse 的 system.users 列（驱动可能回大写或小写键名，两套都备）
  storage: t('udv.fStorage'), STORAGE: t('udv.fStorage'),
  auth_type: t('udv.fAuthType'), AUTH_TYPE: t('udv.fAuthType'),
  host_ip: t('udv.fHostIp'), HOST_IP: t('udv.fHostIp'),
  host_names: t('udv.fHostNames'), HOST_NAMES: t('udv.fHostNames'),
  default_roles_all: t('udv.fDefaultRoles'), DEFAULT_ROLES_ALL: t('udv.fDefaultRoles'),
  grantees_all: t('udv.fGrantees'), GRANTEES_ALL: t('udv.fGrantees'),
  // Oracle / DM 的 dba_users 列（Oracle 驱动回全大写键）
  account_status: t('udv.fAccountStatus'), ACCOUNT_STATUS: t('udv.fAccountStatus'),
  created: t('udv.fCreateDate'), CREATED: t('udv.fCreateDate'),
  default_tablespace: t('udv.fTablespace'), DEFAULT_TABLESPACE: t('udv.fTablespace'),
  temporary_tablespace: t('udv.fTempTablespace'), TEMPORARY_TABLESPACE: t('udv.fTempTablespace'),
  profile: t('udv.fProfile'), PROFILE: t('udv.fProfile'),
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

// 时间列统一格式化：各家回的原样不同（MySQL 是 `2026-09-03T05:19:41`、Oracle 是
// `2026-07-05 11:36:44`），这里把 ISO 的 `T` 换成空格、去掉毫秒与时区尾巴，读起来一致。
// 只动"看着就是时间戳"的字符串，别的值原样返回。
const DATEISH = /^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(:\d{2})?(\.\d+)?(Z|[+-]\d{2}:?\d{2})?$/
export const formatFieldTime = (value) => {
  if (typeof value !== 'string' || !DATEISH.test(value)) return value
  return value.replace('T', ' ').replace(/\.\d+/, '').replace(/(Z|[+-]\d{2}:?\d{2})$/, '')
}

// 认不出来的键退回原始键名，不要吞掉 —— 后端加了列但这里没加标签时，至少能看到是哪一列
export const userFieldLabel = (key) => {
  const m = userFieldLabelMap()
  return m[key] || m[String(key).toUpperCase()] || key
}

// 「是 / 否」类字段：各家命名与取值都不同（SQL Server `disabled` 0/1、
// PostgreSQL `is_superuser` 0/1、MySQL `ACCOUNT_LOCKED` Y/N），统一按这类处理
const BOOL_FIELDS = /^(DISABLED|IS_DISABLED|IS_SUPERUSER|CAN_LOGIN|CAN_CREATE_DB|ACCOUNT_LOCKED|PASSWORD_EXPIRED|PASSWORD_REQUIRE_CURRENT)$/

// 详情里**不展示**的键：授权清单是数组（另有专门的块/SQL 预览）、
// 创建语句去 SQL 预览看，用户名/主机在标题与「基本信息」里已有位置。
// 键名归一：各家驱动的键名大小写与下划线都不一样（`grants_text` / `GRANTS_TEXT` / `grantsText`
// 是同一列），去掉非字母数字再比，三种写法都算同一个键。
const normKey = (k) => String(k).toUpperCase().replace(/[^A-Z0-9]/g, '')

const HIDDEN_KEYS = [
  'GRANTS', 'GRANTS_TEXT', 'ROLES', 'PERMISSIONS', 'ERROR', 'NAME', 'HOST', 'USER',
  'AUTHENTICATION_TYPE_DESC', 'CREATE_USER_SQL', 'CREATE_USER',
  // 凭据一律不展示（口令散列 / 认证串 / MySQL 8 的属性串）——
  // 与本应用其它地方的取数口径一致：密码只写不读
  'PASSWORD', 'AUTHENTICATION_STRING', 'USER_ATTRIBUTES'
]

// opts.hide   ：额外要藏掉的键（大写比对）
// opts.hideRe ：额外要藏掉的键的正则（大写比对）
export const userDetailItems = (obj, opts = {}) => {
  const hide = new Set(HIDDEN_KEYS.concat(opts.hide || []).map(normKey))
  const hideRe = opts.hideRe || null
  const items = []
  for (const [k, v] of Object.entries(obj || {})) {
    if (v === null || v === undefined || v === '') continue
    // 数组 / 对象不进「标签 - 值」表：摊开来是一串没有标签的东西（授权清单就是这种）
    if (typeof v === 'object') continue
    // BLOB 列驱动回的是 `[blob 0 B]` 这种占位串（MySQL 5.6 的 ssl_cipher / x509_issuer /
    // x509_subject 就是 BLOB）—— 空值当没这一列，别在表里排三行"没有内容的东西"
    if (typeof v === 'string' && /^\[blob \d+ B\]$/i.test(v.trim())) continue
    const KEY = String(k).toUpperCase()
    if (hide.has(normKey(k))) continue
    if (hideRe && hideRe.test(normKey(k))) continue
    let val = v
    let cls = ''
    // 权限类字段（Y/N）→ 允许 / 禁止
    if ((v === 'Y' || v === 'N') && /_PRIV$/.test(KEY)) {
      val = v === 'Y' ? t('udv.allow') : t('udv.deny')
      cls = v === 'Y' ? 'allow' : 'deny'
    } else if (/^MAX_(QUESTIONS|UPDATES|CONNECTIONS|USER_CONNECTIONS)$/.test(KEY) && String(v) === '0') {
      // MySQL 的 max_* 是**用户级配额**，0 表示"不限制" —— 直接摊个 0 出来会被读成"一个都不许"
      val = t('udv.unlimited')
      cls = 'no'
    } else if (BOOL_FIELDS.test(KEY)) {
      // 布尔型字段各家取值形态不一：SQL Server 的 disabled 是 0/1、
      // PostgreSQL 的 rol* 也是 0/1、MySQL 那批是 Y/N —— 统一成「是/否」再上色，
      // 别把 0/1 裸值摊给用户看
      const yes = v === true || v === 1 || v === '1' || v === 'Y' || String(v).toLowerCase() === 'yes'
      val = yes ? t('udv.yes') : t('udv.no')
      cls = yes ? 'yes' : 'no'
    }
    items.push({ key: k, label: userFieldLabel(k), value: cls ? val : formatFieldTime(val), cls })
  }
  return items
}
