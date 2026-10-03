<template>
  <el-dialog :model-value="modelValue" width="680px"
             :close-on-click-modal="false"
             @update:model-value="$emit('update:modelValue', $event)"
             @open="initForm" destroy-on-close
             class="conn-dialog">
    <template #header>
      <div class="dlg-title">
        <span class="dlg-title-ic"><el-icon :size="16"><Link /></el-icon></span>
        <span>{{ conn ? $t('cd.editTitle') : $t('cd.newTitle') }}</span>
      </div>
    </template>
    <div class="type-bar" @click="openPicker">
      <DbLogo :type="form.type || 'MYSQL'" :size="36" />
      <div class="type-bar-info">
        <div class="type-bar-name">{{ currentType ? labelOf(currentType.code) : $t('cd.pickType') }}</div>
        <div class="type-bar-cat">{{ categoryLabel }}</div>
      </div>
      <el-button size="small" type="primary" plain class="type-bar-btn">
        <el-icon style="margin-right:4px"><Refresh /></el-icon>{{ $t('cd.change') }}
      </el-button>
    </div>

    <el-tabs v-model="activeTab" class="conn-tabs">
      <!-- ============ 基本配置 ============ -->
      <el-tab-pane :label="$t('cd.tabBasic')" name="basic">
        <el-form :model="form" label-width="90px" label-position="left">
          <el-form-item :label="$t('cd.name')" required>
            <el-input v-model="form.name" />
          </el-form-item>

          <!-- 备注：存在连接的 extra.note 里（后端 config_from_body 写入、connection_json 回传）。
               用途是「这条连接是干嘛的」—— 库名看不出来，人能一句话说清。 -->
          <el-form-item :label="$t('cd.note')">
            <el-input v-model="form.note" type="textarea" :rows="2" maxlength="200" show-word-limit />
          </el-form-item>

          <div class="form-row">
            <el-form-item :label="$t('cd.env')" required style="flex:1">
              <el-select v-model="form.env" style="width: 100%">
                <el-option value="DEV" :label="$t('env.dev')">
                  <span class="env-dot env-dev"></span>{{ $t('env.dev') }}
                </el-option>
                <el-option value="TEST" :label="$t('env.test')">
                  <span class="env-dot env-test"></span>{{ $t('env.test') }}
                </el-option>
                <el-option value="PROD" :label="$t('env.prod')">
                  <span class="env-dot env-prod"></span>{{ $t('env.prod') }}
                </el-option>
              </el-select>
            </el-form-item>
            <el-form-item :label="$t('cd.group')" required style="flex:1">
              <div style="display: flex; gap: 8px; width: 100%">
                <el-select v-model="form.group" style="flex:1" allow-create filterable
                           :empty-values="[null, undefined]" default-first-option
                           @change="onEnvChange">
                  <el-option-group v-if="customFolders.length" :label="$t('cd.group')">
                    <el-option v-for="f in customFolders" :key="f" :value="f">
                      <el-icon style="vertical-align: -2px; margin-right: 4px; color: var(--el-color-primary)"><FolderOpened /></el-icon>{{ f }}
                    </el-option>
                  </el-option-group>
                  <el-option v-else value="" disabled>
                    <span style="color: var(--el-text-color-secondary)">{{ $t('cd.noGroup') }}</span>
                  </el-option>
                </el-select>
                <el-button type="primary" plain :icon="Plus" @click="createFolder">{{ $t('common.new') }}</el-button>
              </div>
            </el-form-item>
          </div>

          <!-- 文件型 -->
          <template v-if="isFileType">
            <el-form-item :label="$t('cd.dbFile')" required>
              <div class="file-picker">
                <el-input v-model="form.filePath" :placeholder="$t('cd.filePathHint')" />
                <el-button :icon="FolderOpened" @click="pickFile">{{ $t('cd.browse') }}</el-button>
                <el-button :icon="DocumentAdd" @click="form.filePath = ''">{{ $t('common.new') }}</el-button>
              </div>
            </el-form-item>
            <el-form-item label="JDBC URL">
              <el-input v-model="form.jdbcUrl" type="textarea" :autosize="{ minRows: 2, maxRows: 4 }"
                        @input="jdbcUrlManuallyEdited = true" />
            </el-form-item>
            <el-alert v-if="typeDef.fileHint" type="info" :closable="false" show-icon
                      :title="$t(typeDef.fileHint)" style="margin-bottom:12px" />
          </template>

          <!-- 网络型 -->
          <template v-else>
            <div class="form-row">
              <el-form-item :label="$t('cd.host')" required style="flex:1">
                <el-input v-model="form.host" />
              </el-form-item>
              <el-form-item :label="$t('cd.port')" label-width="55px" required style="flex: 0 0 170px">
                <el-input v-model.number="form.port" />
              </el-form-item>
            </div>

            <div class="form-row">
              <el-form-item :label="$t(typeDef.databaseLabel)" style="flex:1">
                <el-input v-model="form.database" />
              </el-form-item>
              <el-form-item v-if="typeDef.protocols.length" :label="$t('cd.protocol')" label-width="55px" style="flex: 0 0 170px">
                <el-select v-model="form.esProtocol" style="width: 100%">
                  <el-option v-for="protocol in typeDef.protocols" :key="protocol" :value="protocol" :label="protocol" />
                </el-select>
              </el-form-item>
            </div>

            <el-form-item label="JDBC URL">
              <el-input v-model="form.jdbcUrl" type="textarea" :autosize="{ minRows: 2, maxRows: 4 }"
                        @input="jdbcUrlManuallyEdited = true" />
            </el-form-item>

            <!-- 认证方式：仅类型声明了 authTypes 时显示（如 SQL Server） -->
            <el-form-item v-if="showAuthSelect" :label="$t('udv.fAuthType')">
              <el-select v-model="form.authType" style="width: 100%">
                <el-option v-for="a in typeDef.authTypes" :key="a.value" :value="a.value" :label="a.labelKey ? $t(a.labelKey) : a.label" />
              </el-select>
            </el-form-item>
            <!-- Windows 客户端上的 Windows 集成认证：直接用系统登录账号（SSPI），无需用户名/密码，整块隐藏 -->
            <el-alert v-if="isWindowsAuth && !nativeWindowsAuth" type="info" :closable="false" show-icon class="auth-hint"
                      :title="windowsAuthHint" />

            <div v-if="!hideCredentials && !nativeWindowsAuth" class="form-row">
              <el-form-item :label="$t('udv.fUser')" :required="!isWindowsAuth" style="flex:1">
                <el-input v-model="form.username" :placeholder="isWindowsAuth ? $t('cd.userPlaceholder') : ''" />
              </el-form-item>
              <el-form-item v-if="!isWindowsAuth" :label="$t('cd.password')" :required="!hasSavedPassword" style="flex:1">
                <el-input v-model="form.password" type="password" show-password
                          :placeholder="hasSavedPassword ? $t('cd.pwdSaved') : ''" />
              </el-form-item>
            </div>
          </template>

          <div v-if="isDriverReady" class="driver-hint driver-hint-ok">
            <el-icon :size="14"><CircleCheckFilled /></el-icon>
            {{ $t('cd.driverReady') }}
          </div>
          <div v-else class="driver-hint">
            <el-icon :size="14"><WarningFilled /></el-icon>
            {{ $t('cd.driverDownload') }}<template v-if="driverSize"> ({{ driverSize }})</template>{{ $t('cd.driverKeepNetwork') }}
          </div>
        </el-form>
      </el-tab-pane>

      <!-- ============ SSH 隧道 ============ -->
      <el-tab-pane :label="$t('cd.tabSsh')" name="ssh">
        <el-form :model="form" label-width="110px" label-position="left">
          <el-form-item :label="$t('cd.useSsh')">
            <el-switch v-model="form.sshEnabled" />
            <span class="form-tip">{{ $t('cd.sshTip') }}</span>
          </el-form-item>

          <template v-if="form.sshEnabled">
            <div class="form-row">
              <el-form-item :label="$t('cd.sshHost')" required style="flex:1">
                <el-input v-model="form.sshHost" />
              </el-form-item>
              <el-form-item :label="$t('cd.port')" label-width="55px" style="flex: 0 0 170px">
                <el-input v-model.number="form.sshPort" />
              </el-form-item>
            </div>
            <div class="form-row">
              <el-form-item :label="$t('cd.sshUser')" required style="flex:1">
                <el-input v-model="form.sshUser" />
              </el-form-item>
              <el-form-item :label="$t('udv.fAuthType')" label-width="80px" style="flex: 0 0 170px">
                <el-select v-model="form.sshAuthType" style="width: 100%">
                  <el-option value="password" :label="$t('cd.password')" />
                  <el-option value="key" :label="$t('cd.privateKey')" />
                </el-select>
              </el-form-item>
            </div>
            <el-form-item v-if="form.sshAuthType === 'password'" :label="$t('cd.sshPassword')">
              <el-input v-model="form.sshPassword" type="password" show-password
                        :placeholder="hasSavedSshPassword ? $t('cd.pwdSaved') : ''" />
            </el-form-item>
            <template v-else>
              <el-form-item :label="$t('cd.keyPath')">
                <el-input v-model="form.sshKeyPath" />
              </el-form-item>
              <el-form-item :label="$t('cd.keyPassphrase')">
                <el-input v-model="form.sshKeyPassphrase" type="password" show-password
                          :placeholder="hasSavedSshPassword ? $t('cd.pwdSaved') : ''" />
              </el-form-item>
            </template>
          </template>
        </el-form>
      </el-tab-pane>

      <!-- ============ 高级选项 ============ -->
      <el-tab-pane :label="$t('cd.tabAdvanced')" name="advanced">
        <el-form :model="form" label-width="110px" label-position="left">
          <div class="form-row">
            <el-form-item :label="$t('cd.connectTimeout')" style="flex:1">
              <el-input v-model.number="form.connectTimeout" type="number" :min="1" :max="3600" />
            </el-form-item>
            <el-form-item :label="$t('cd.readTimeout')" style="flex:1">
              <el-input v-model.number="form.socketTimeout" type="number" :min="1" :max="86400" />
            </el-form-item>
            <el-form-item :label="$t('cd.writeTimeout')" style="flex:1">
              <el-input v-model.number="form.writeTimeout" type="number" :min="1" :max="86400" />
            </el-form-item>
          </div>

          <el-form-item :label="$t('ndb.charset')">
            <el-select v-model="form.charset" style="width: 100%" allow-create filterable>
              <el-option v-for="c in charsets" :key="c" :value="c" :label="c" />
            </el-select>
          </el-form-item>

          <!-- 只读连接：连接记录上的标记，后端走内核 set_read_only（同时更新存储与内存策略）。
               开启后这个连接上的**写操作会被内核拦下**（报「安全拦截」），不是界面上的软提示。 -->
          <el-form-item :label="$t('cd.readOnly')">
            <el-switch v-model="form.readOnly" />
            <span class="form-tip">{{ $t('cd.readOnlyTip') }}</span>
          </el-form-item>

          <el-form-item :label="$t('cd.extraParams')">
            <div class="param-box">
              <!-- 常用参数预设：按当前数据源类型给，点一下即加入。
                   给的都是驱动**真正认识**的参数名（会原样进入 JDBC Properties），不是说明文字。 -->
              <div v-if="paramPresets.length" class="preset-row">
                <span class="preset-tip">{{ $t('cd.commonPresets') }}</span>
                <el-tag v-for="p in paramPresets" :key="p.name" size="small" class="preset-tag"
                        :class="{ 'preset-added': hasParam(p.name) }" :title="p.tip"
                        @click="applyPreset(p)">
                  {{ p.name }}<template v-if="hasParam(p.name)"> ✓</template>
                </el-tag>
              </div>
              <template v-for="(p, i) in form.params" :key="i">
                <div class="param-row">
                  <el-input v-model="p.name" :placeholder="i === 0 ? $t('cd.paramNamePlaceholder') : $t('cd.paramName')" />
                  <el-input v-model="p.value" :placeholder="i === 0 ? $t('cd.paramValuePlaceholder') : $t('cd.paramValue')" />
                  <el-button :icon="Delete" circle plain type="danger" @click="removeParam(i)" />
                </div>
                <div v-if="duplicateParams.has(String(p.name || '').trim())" class="param-warn">
                  {{ $t('cd.dupParam') }}
                </div>
              </template>
              <div class="param-ops">
                <el-button :icon="Plus" size="small" @click="addParam">{{ $t('cd.addParam') }}</el-button>
                <el-button size="small" @click="pasteVisible = !pasteVisible">{{ $t('cd.bulkPaste') }}</el-button>
              </div>
              <!-- 批量粘贴：从文档/配置里拷一串 `k=v` 一次导入，省得一条条点 -->
              <div v-if="pasteVisible" class="paste-box">
                <el-input v-model="pasteText" type="textarea" :rows="4"
                          :placeholder="$t('cd.pastePlaceholder')" />
                <div class="param-ops">
                  <el-button size="small" type="primary" @click="importPastedParams">{{ $t('cd.importBtn') }}</el-button>
                  <el-button size="small" @click="pasteVisible = false">{{ $t('common.cancel') }}</el-button>
                </div>
              </div>
            </div>
          </el-form-item>
        </el-form>
      </el-tab-pane>
    </el-tabs>

    <template #footer>
      <el-button :loading="testing" @click="doTest">
        <el-icon style="margin-right:4px"><Connection /></el-icon>{{ $t('cd.test') }}
      </el-button>
      <el-button @click="$emit('update:modelValue', false)">{{ $t('common.cancel') }}</el-button>
      <el-button type="primary" :loading="saving" @click="doSave">{{ $t('common.save') }}</el-button>
    </template>

    <!-- 数据源选择器（更换类型时使用） -->
    <DataSourcePicker v-model="pickerVisible" @selected="onTypePicked" />

    <!-- 数据库文件选择：点「浏览」打开。由后端列目录（与「备份还原」用的是同一个接口），
         逐级进入、点文件即选中 —— Web 壳拿不到系统文件对话框，这是唯一能拿到
         **真实绝对路径**（也就能直接给 JDBC 用）的办法。 -->
    <el-dialog v-model="pickerOpen" :title="$t('cd.pickTitle')" width="540px" append-to-body>
      <div class="path-picker">
        <div class="pp-head">
          <span class="pp-up" :class="{ disabled: !pickerPath }" @click="pickerPath && browseInto(pickerParent)">
            <el-icon><ArrowUp /></el-icon><span>{{ $t('bkp.goUp') }}</span>
          </span>
          <span class="pp-cur" :title="pickerPath">{{ pickerPath }}</span>
        </div>
        <div v-loading="pickerLoading" class="pp-list">
          <div v-for="d in pickerDirs" :key="'d_' + d" class="pp-item" @click="browseInto(joinPath(d))">
            <el-icon><Folder /></el-icon><span>{{ d }}</span>
          </div>
          <div v-for="f in pickerFiles" :key="'f_' + f" class="pp-item file" @click="chooseFile(f)">
            <el-icon><Document /></el-icon><span>{{ f }}</span>
          </div>
          <div v-if="!pickerLoading && !pickerDirs.length && !pickerFiles.length" class="pp-empty">
            {{ $t('bkp.emptyDir') }}
          </div>
        </div>
      </div>
    </el-dialog>
  </el-dialog>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { FolderOpened, DocumentAdd, Connection, WarningFilled, CircleCheckFilled, Plus, Delete, Refresh, Link, ArrowUp, Folder, Document } from '@element-plus/icons-vue'
import { saveConnection, testConnection, getConnectionById, getDriverTypes, getDriverStatus, browseBackupDirs } from '../../api'
import { connErrorHintText } from '../../utils/connErrors'
import { addPureFolder } from '../../utils/folders'
import { t } from '../../utils/i18n'
import { byType, labelOf } from '../../types'
import DbLogo from '../../common/DbLogo.vue'
import DataSourcePicker from '../../common/DataSourcePicker.vue'

const props = defineProps({
  modelValue: Boolean,
  conn: Object,
  customFolders: { type: Array, default: () => [] },
  initialGroup: { type: String, default: '' },
  initialType: { type: String, default: '' }
})
const emit = defineEmits(['update:modelValue', 'saved', 'folder-added'])

const form = ref({})
const activeTab = ref('basic')
const testing = ref(false)
const saving = ref(false)
const types = ref([])
const driverStatus = ref({})
const pickerVisible = ref(false)
const jdbcUrlManuallyEdited = ref(false)

const charsets = ['UTF-8', 'GBK', 'GB2312', 'GB18030', 'Latin1', 'ISO-8859-1']

onMounted(async () => {
  try {
    types.value = await getDriverTypes()
    driverStatus.value = await getDriverStatus()
  } catch (e) { /* 兼容旧后端 */ }
})

const relationalTypes = computed(() => types.value.filter(t => t.category === 'RELATIONAL'))
const fileTypes = computed(() => types.value.filter(t => t.category === 'RELATIONAL_FILE'))
const nosqlTypes = computed(() => types.value.filter(t => t.category === 'NOSQL'))

const currentType = computed(() => types.value.find(t => t.code === form.value.type))
const defaultPort = computed(() => currentType.value?.defaultPort || '')
const isFileType = computed(() => currentType.value?.category === 'RELATIONAL_FILE')
const driverSize = computed(() => driverStatus.value[form.value.type]?.size || '')
const isDriverReady = computed(() => driverStatus.value[form.value.type]?.ready === true)
// 类型能力一律来自前端注册表（src/types），连接表单不再写死类型清单
const typeDef = computed(() => byType(form.value?.type || 'MYSQL'))
// 是否显示用户名/密码：由类型定义声明（NoSQL 默认不需要）
const hideCredentials = computed(() => !typeDef.value.credentials)
// 认证方式：仅类型声明 authTypes 时显示（如 SQL Server 的 SQL Server 身份验证 / Windows 身份验证）
const showAuthSelect = computed(() => Array.isArray(typeDef.value.authTypes) && typeDef.value.authTypes.length > 0)
const isWindowsAuth = computed(() => showAuthSelect.value && form.value?.authType === 'windows')
const isWindowsClient = /windows/i.test(navigator.userAgent || '')
// Windows 客户端上的集成认证走系统登录账号（SSPI），表单无需用户名/密码：提示与凭据字段整块隐藏
const nativeWindowsAuth = computed(() => isWindowsAuth.value && isWindowsClient)
const windowsAuthHint = computed(() => t('cd.windowsAuthHint'))

const categoryLabel = computed(() => {
  if (!currentType.value) return ''
  const c = currentType.value.category
  if (c === 'RELATIONAL') return t('settings.driver.catRelational')
  if (c === 'RELATIONAL_FILE') return t('settings.driver.catFile')
  if (c === 'NOSQL') return t('cd.catNosql')
  return ''
})

// 各数据源类型的默认配置：端口来自后端 types 元数据，缺失时回退到前端类型注册表
const typesByCode = computed(() => {
  const m = {}
  for (const t of types.value) m[t.code] = t
  return m
})
const typeDefaults = (type) => {
  const common = {
    name: '', host: '', port: undefined, database: '',
    username: '', password: '', filePath: '',
    group: props.customFolders[0] || '', env: 'DEV', jdbcUrl: '', params: [],
    connectTimeout: 10, socketTimeout: 600, writeTimeout: 300, charset: 'UTF-8',
    esProtocol: 'http',
    sshEnabled: false, sshHost: '', sshPort: 22, sshUser: '',
    sshAuthType: 'password', sshPassword: '', sshKeyPath: '', sshKeyPassphrase: ''
  }
  const def = byType(type)
  const meta = typesByCode.value[type]
  const port = def.fileType ? 0
    : (meta?.defaultPort != null && meta.defaultPort > 0 ? meta.defaultPort : (def?.defaultPort || 0))
  const authType = (def.authTypes && def.authTypes[0]?.value) || ''
  return { ...common, type, port, authType }
}

const defaults = () => typeDefaults(props.initialType || 'MYSQL')

// 根据当前配置自动生成 URL（只读展示）：URL 模板已收敛到各数据库类型模块（src/types/*.js）
const buildJdbcUrl = () => {
  const f = form.value
  if (!f) return ''
  return typeDef.value.buildJdbcUrl?.(f) || ''
}

// 监听关键字段变化，自动更新 jdbcUrl（仅用户未手动编辑过时回填，避免覆盖用户自定义 URL）
watch(() => [form.value?.type, form.value?.host, form.value?.port, form.value?.database, form.value?.filePath,
  form.value?.charset, form.value?.connectTimeout, form.value?.socketTimeout, form.value?.esProtocol], () => {
  if (!form.value || jdbcUrlManuallyEdited.value) return
  // 用户输入主机/文件等信息后自动生成；清空关键字段时也同步清空自动生成的 URL
  form.value.jdbcUrl = buildJdbcUrl()
}, { deep: true })

// 认证方式切换会改变 URL 结构（如 SQL Server 的 integratedSecurity），必须强制重写，不受手动编辑保护
watch(() => form.value?.authType, () => {
  if (!form.value) return
  form.value.jdbcUrl = buildJdbcUrl()
})

const initForm = async () => {
  jdbcUrlManuallyEdited.value = false
  if (props.conn && props.conn.id) {
    // 编辑：异步拉取真实密码（列表中的 password 是 enc:xxx 密文）
    try {
      const detail = await getConnectionById(props.conn.id)
      form.value = { ...props.conn, ...detail }
    } catch (e) {
      // 拉取失败时回退到列表数据
      form.value = { ...props.conn }
    }
    // 编辑已有连接时，若已保存了自定义 JDBC URL，视为用户已手动编辑
    if (form.value.jdbcUrl) {
      jdbcUrlManuallyEdited.value = true
    }
  } else {
    form.value = defaults()
    // 父组件指定的初始分组（右键新建连接时传入）
    if (props.initialGroup) form.value.group = props.initialGroup
  }
  // 兼容旧数据：缺失字段时归一化
  if (!form.value.jdbcUrl) form.value.jdbcUrl = ''
  if (!Array.isArray(form.value.params)) form.value.params = []
  if (!form.value.group) form.value.group = props.customFolders[0] || ''
  if (!form.value.env) form.value.env = 'DEV'
  if (!form.value.charset) form.value.charset = 'UTF-8'
  if (!form.value.connectTimeout) form.value.connectTimeout = 10
  if (!form.value.socketTimeout) form.value.socketTimeout = 600
  if (!form.value.writeTimeout) form.value.writeTimeout = 300
  if (!form.value.esProtocol) form.value.esProtocol = 'http'
  // 认证方式归一化：类型支持认证选择时保证有有效值（兼容历史数据）
  const authDefs = typeDef.value.authTypes || []
  if (authDefs.length && !authDefs.some(a => a.value === form.value.authType)) {
    form.value.authType = authDefs[0].value
  }
  // SSH 字段归一化
  if (form.value.sshEnabled == null) form.value.sshEnabled = false
  if (!form.value.sshHost) form.value.sshHost = ''
  if (!form.value.sshUser) form.value.sshUser = ''
  if (!form.value.sshAuthType) form.value.sshAuthType = 'password'
  if (!form.value.sshPassword) form.value.sshPassword = ''
  if (!form.value.sshKeyPath) form.value.sshKeyPath = ''
  if (!form.value.sshKeyPassphrase) form.value.sshKeyPassphrase = ''
  // 确保 port 为数字类型（避免 el-input number 模式收到字符串报错）
  if (form.value.port != null && form.value.port !== '') {
    const n = Number(form.value.port)
    if (!isNaN(n)) form.value.port = n
  }
  // SSH 端口默认 22：**协议默认值，不是"可选填"**。
  // 空着的话用户不知道该填什么（面板上只有一个空输入框），而后端本来也会回落 22 ——
  // 与其让默认值藏在后端，不如直接写进表单让它**看得见**（新建与编辑老连接一视同仁：
  // 编辑时后端对未配置的端口返回 null，这里也补成 22）。
  const sshPort = Number(form.value.sshPort)
  form.value.sshPort = Number.isFinite(sshPort) && sshPort > 0 && sshPort <= 65535 ? sshPort : 22
}

// 分组变更：如果输入了新的分组名，自动存为后端纯分组
const onEnvChange = (v) => {
  if (!v) return
  addPureFolder(v)
}

// 快捷新建分组
const createFolder = async () => {
  try {
    const { value } = await ElMessageBox.prompt(t('cd.folderPrompt'), t('fd.newTitle'), {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      inputPattern: /^\S+$/,
      inputErrorMessage: t('fd.needName'),
      inputValue: t('cd.unnamedFolder')
    })
    if (!value) return
    try {
      await addPureFolder(value)
      form.value.group = value
      emit('folder-added', value)
      ElMessage.success(t('cd.folderCreated', { name: value }))
    } catch { /* ignore */ }
  } catch { /* 用户取消 */ }
}

const syncPort = () => {
  if (!form.value.port && defaultPort.value) form.value.port = defaultPort.value
}

// 数据源选择器
const openPicker = () => { pickerVisible.value = true }
const onTypePicked = (code) => {
  const prevName = form.value?.name || ''
  const prevEnv = form.value?.env || 'DEV'
  const prevEnvironment = form.value?.environment || props.customFolders[0] || ''
  jdbcUrlManuallyEdited.value = false
  form.value = {
    ...typeDefaults(code),
    name: prevName,
    env: prevEnv,
    group: prevGroup
  }
  activeTab.value = 'basic'
}

/** 该连接是否已存过口令：详情接口不再回传明文，表单据此提示「留空表示不修改」 */
const hasSavedPassword = computed(() => !!props.conn?.hasPassword)
const hasSavedSshPassword = computed(() => !!props.conn?.hasSshPassword)

const doTest = async () => {
  testing.value = true
  try {
    const res = await testConnection(form.value)
    if (res.success) ElMessage.success(t('mv.connectOk', { version: (res.serverVersion || '') }))
    else showTestFailure(res.message)
  } catch (e) { showTestFailure(e.message) }
  testing.value = false
  // 测试后刷新驱动状态（可能刚触发了驱动下载）
  try { driverStatus.value = await getDriverStatus() } catch { /* ignore */ }
}

/**
 * 测试失败：错误原文 + **可操作的排查建议**。
 *
 * 为什么不用 toast：原文（`Communications link failure` / `PKIX path building failed`）一句就过去了，
 * 用户既看不全也记不住。这里用弹窗把原文和建议一起列出，建议由 `utils/connErrors.js` 按关键字给。
 */
const showTestFailure = (message) => {
  const raw = String(message || t('common.unknownError'))
  const tips = connErrorHintText(raw, form.value)
  ElMessageBox.alert(
    (raw ? t('cd.errRaw') + '\n' + raw + '\n\n' : '')
      + (tips ? t('cd.tipsHead') + '\n' + tips : ''),
    t('mv.connFailedTitle'),
    { confirmButtonText: t('common.gotIt'), customClass: 'conn-fail-box', customStyle: { whiteSpace: 'pre-line', maxWidth: '560px' } }
  ).catch(() => {})
}

const addParam = () => form.value.params.push({ name: '', value: '' })
const removeParam = (i) => form.value.params.splice(i, 1)

const doSave = async () => {
  if (!form.value.name) return ElMessage.warning(t('cd.needName'))
  if (!form.value.env) return ElMessage.warning(t('cd.needEnv'))
  if (!form.value.group) return ElMessage.warning(t('cd.needGroup'))
  if (!isFileType.value && !form.value.host && !form.value.jdbcUrl) return ElMessage.warning(t('cd.needHost'))
  if (!isFileType.value && !form.value.port && !form.value.jdbcUrl) return ElMessage.warning(t('cd.needPort'))
  // 文件型没有主机/端口，但**必须有文件路径**：以前这里漏了，于是能保存出一个没有文件路径的连接，
  // 点连接才报错 —— 报错点离出错点越远越难查。
  if (isFileType.value && !String(form.value.filePath || '').trim()) {
    return ElMessage.warning(t('cd.needFilePath'))
  }
  // Windows 集成认证使用系统 / Kerberos 身份，不强制账号密码；
  // 文件型（SQLite / H2 / Derby …）根本没有账号密码 —— 这里以前漏了 isFileType，
  // 于是 QLite 也会弹「请输入用户名」（用户看到的就是一句牛头不对马嘴的提示）。
  if (!hideCredentials.value && !isWindowsAuth.value && !isFileType.value) {
    if (!form.value.username) return ElMessage.warning(t('cd.needUser'))
    // 编辑已有连接时口令留空 = 沿用已保存的那份（详情接口不再回传明文，见后端 sanitize）
    if (!form.value.password && !hasSavedPassword.value) return ElMessage.warning(t('cd.needPassword'))
  }
  // 过滤空参数行
  form.value.params = (form.value.params || []).filter(p => p.name && String(p.name).trim())
  saving.value = true
  try {
    const saved = await saveConnection(form.value)
    ElMessage.success(t('common.saved'))
    emit('update:modelValue', false)
    emit('saved', saved)
  } catch (e) { ElMessage.error(e.message) }
  saving.value = false
}

// ========== 数据库文件选择 ==========

// 为什么不用系统文件对话框：这是 Web 壳 —— 浏览器拿不到真实路径，也看不到磁盘上的文件。
// 由后端列目录，用户点一个文件，拿到的就是**能直接交给 JDBC 的绝对路径**。
const pickerOpen = ref(false)
const pickerLoading = ref(false)
const pickerPath = ref('')
const pickerParent = ref('')
const pickerDirs = ref([])
const pickerFiles = ref([])

/** 把列表项拼成路径：空路径时那一项本身就是盘符（如 C:\）。 */
const joinPath = (name) => {
  const base = String(pickerPath.value || '')
  if (!base) return name
  const sep = base.includes('\\') ? '\\' : '/'
  return base.endsWith(sep) ? base + name : base + sep + name
}

const browseInto = async (path) => {
  pickerLoading.value = true
  try {
    const r = await browseBackupDirs(path || '')
    pickerPath.value = r?.path || ''
    pickerParent.value = r?.parent || ''
    pickerDirs.value = r?.dirs || []
    pickerFiles.value = r?.files || []
    if (r && r.exists === false) ElMessage.warning(t('bkp.browseFailed', { detail: path }))
  } catch (e) {
    ElMessage.error(t('bkp.browseFailed', { detail: e.message || e }))
  }
  pickerLoading.value = false
}

const pickFile = async () => {
  pickerOpen.value = true
  // 起点：输入框里已经写着的目录 → 用户主目录 → 磁盘/根列表
  const current = String(form.value.filePath || '').trim()
  const dir = current.replace(/[\\/][^\\/]*$/, '')
  if (dir && dir !== current) {
    await browseInto(dir)
    if (pickerDirs.value.length || pickerFiles.value.length) return
  }
  try {
    const root = await browseBackupDirs('')
    await browseInto(root?.home || '')
  } catch {
    await browseInto('')
  }
}

const chooseFile = (name) => {
  form.value.filePath = joinPath(name)
  pickerOpen.value = false
}

// ========== 常用参数预设 / 批量粘贴 ==========

// 常用参数：**参数名必须与驱动一致**（会原样进 JDBC Properties），所以按类型分开给。
// 每条都附一句"什么情况下需要它"，避免用户照着猜。
// 用函数而不是模块级常量：常量只在加载时求值一次，切换语言后这些提示语不会跟着变
const paramPresetsOf = () => ({
  mysql: [
    { name: 'useSSL', value: 'false', tip: t('cd.tipUseSsl') },
    { name: 'allowPublicKeyRetrieval', value: 'true', tip: t('cd.tipAllowPublicKey') },
    { name: 'serverTimezone', value: 'Asia/Shanghai', tip: t('cd.tipTimezone') },
    { name: 'characterEncoding', value: 'UTF-8', tip: t('cd.tipEncoding') },
    { name: 'rewriteBatchedStatements', value: 'true', tip: t('cd.tipRewriteBatched') },
    { name: 'zeroDateTimeBehavior', value: 'convertToNull', tip: t('cd.tipZeroDate') }
  ],
  mariadb: [
    { name: 'useSSL', value: 'false', tip: t('cd.tipUseSslShort') },
    { name: 'serverTimezone', value: 'Asia/Shanghai', tip: t('cd.tipTimezoneShort') },
    { name: 'characterEncoding', value: 'UTF-8', tip: t('cd.tipEncoding') }
  ],
  postgresql: [
    { name: 'sslmode', value: 'disable', tip: t('cd.tipSslmode') },
    { name: 'ApplicationName', value: 'DBMind', tip: t('cd.tipAppName') },
    { name: 'stringtype', value: 'unspecified', tip: t('cd.tipStringtype') }
  ],
  kingbase: [{ name: 'sslmode', value: 'disable', tip: t('cd.tipSslmode') }],
  sqlserver: [
    { name: 'encrypt', value: 'false', tip: t('cd.tipServerEncrypt') },
    { name: 'trustServerCertificate', value: 'true', tip: t('cd.tipTrustServer') },
    { name: 'loginTimeout', value: '15', tip: t('cd.tipLoginTimeout') }
  ],
  clickhouse: [{ name: 'socket_timeout', value: '300000', tip: t('cd.tipSocketTimeout') }],
  doris: [{ name: 'connectTimeout', value: '15000', tip: t('cd.tipConnectTimeout') }],
  oracle: [{ name: 'oracle.jdbc.ReadTimeout', value: '300000', tip: t('cd.tipReadTimeout') }]
})
const paramPresets = computed(() => {
  const all = paramPresetsOf()
  const code = String(typeDef.value?.code || form.value.type || form.value.kind || '').toLowerCase()
  if (all[code]) return all[code]
  // 兜底：MySQL 系（含各类兼容分支）给最常见的那组
  return code.includes('mysql') ? all.mysql : []
})
const hasParam = (name) => (form.value.params || []).some(p => String(p.name || '').trim() === name)
const applyPreset = (p) => {
  form.value.params = form.value.params || []
  const exists = form.value.params.find(x => String(x.name || '').trim() === p.name)
  if (exists) {
    exists.value = p.value
    return
  }
  form.value.params.push({ name: p.name, value: p.value })
}
/** 同名参数只认最后一条（与 JDBC Properties 的行为一致），这里提前提示 */
const duplicateParams = computed(() => {
  const seen = new Set()
  const dup = new Set()
  for (const p of form.value.params || []) {
    const name = String(p.name || '').trim()
    if (!name) continue
    if (seen.has(name)) dup.add(name)
    seen.add(name)
  }
  return dup
})

const pasteVisible = ref(false)
const pasteText = ref('')
/** 批量粘贴：从文档/邮件里拷一串 `k=v` 一次导入 */
const importPastedParams = () => {
  form.value.params = form.value.params || []
  let added = 0
  for (const line of String(pasteText.value || '').split(/\r?\n/)) {
    const text = line.trim()
    if (!text || text.startsWith('#')) continue
    const at = text.indexOf('=')
    if (at <= 0) continue
    const name = text.slice(0, at).trim()
    const value = text.slice(at + 1).trim()
    if (!name) continue
    const exists = form.value.params.find(x => String(x.name || '').trim() === name)
    if (exists) exists.value = value
    else form.value.params.push({ name, value })
    added++
  }
  pasteText.value = ''
  pasteVisible.value = false
  ElMessage.success(added ? t('cd.paramsImported', { n: added }) : t('cd.noParamsParsed'))
}
</script>

<style scoped>
/* 常用参数预设 + 批量粘贴 + 重复提示 */
.preset-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; margin-bottom: 8px; }
.preset-tip { font-size: 13px; color: var(--el-text-color-secondary); }
.preset-tag { cursor: pointer; }
.preset-tag.preset-added { color: var(--dc-accent); border-color: var(--dc-accent); }
.param-warn { font-size: 12px; color: var(--dc-warning); margin: 2px 0 6px 2px; }
.param-ops { display: flex; gap: 8px; }
.paste-box { display: flex; flex-direction: column; gap: 8px; }
.dlg-title { display: flex; align-items: center; gap: 8px; }
.dlg-title-ic {
  width: 26px; height: 26px; border-radius: 7px; display: inline-flex;
  align-items: center; justify-content: center; color: var(--dc-on-primary);
  background: linear-gradient(135deg, var(--dc-primary), var(--dc-primary-deep));
  box-shadow: 0 2px 8px var(--dc-primary-glow);
}

/* 顶部数据源类型条 — 背景跟数据源列表（树菜单）一致 */
.type-bar {
  display: flex; align-items: center; gap: 12px;
  padding: 10px 14px; margin-bottom: 12px;
  border: 1px solid var(--dc-bg-hover);
  border-radius: 10px; cursor: pointer;
  background: var(--dc-bg-deep);
  transition: border-color .2s;
}
.type-bar:hover { border-color: var(--el-color-primary); }
.type-bar-info { flex: 1; min-width: 0; }
.type-bar-name { font-size: 15px; font-weight: 600; color: var(--dc-text-strong); }
.type-bar-cat { font-size: 13px; color: var(--dc-text-dim); margin-top: 1px; }
/* 顶部类型条里的 logo：透明背景，跟数据源选择器卡片里的一致 */
.type-bar :deep(.db-logo img) {
  background: transparent !important;
  padding: 0;
  box-shadow: none;
}
/* 更换按钮：与类型条背景同层，文字颜色随主题 */
.type-bar-btn {
  --el-button-text-color: var(--dc-text);
  --el-button-hover-text-color: var(--dc-text-strong);
}

.conn-tabs :deep(.el-tabs__header) { margin-bottom: 14px; }

.form-row { display: flex; gap: 12px; }
.form-row .el-form-item { margin-right: 0; margin-bottom: 18px; }
.form-row .el-form-item__label { padding-bottom: 6px; }
.file-picker { display: flex; gap: 8px; width: 100%; }
/* 数据库文件选择弹窗：与「备份还原」的目录浏览同风格，列表固定高度自己滚 */
.path-picker { display: flex; flex-direction: column; gap: 8px; }
.pp-head { display: flex; align-items: center; gap: 10px; font-size: 13px; }
.pp-up { display: inline-flex; align-items: center; gap: 4px; cursor: pointer; color: var(--dc-accent); }
.pp-up.disabled { color: var(--el-text-color-disabled); cursor: default; }
.pp-cur { color: var(--el-text-color-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pp-list { height: 320px; overflow: auto; border: 1px solid var(--el-border-color); border-radius: 6px; padding: 4px; }
.pp-item { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 4px; cursor: pointer; font-size: 13px; }
.pp-item:hover { background: var(--el-fill-color-light); }
.pp-item.file { color: var(--el-text-color-regular); }
.pp-empty { padding: 12px; text-align: center; color: var(--el-text-color-secondary); font-size: 13px; }
.form-tip { margin-left: 10px; font-size: 13px; color: var(--el-text-color-secondary); }
.auth-hint { margin-bottom: 18px; }
.env-dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 4px; vertical-align: middle; }
.env-dev { background: var(--dc-accent); box-shadow: 0 0 6px rgba(61, 220, 151, .6); }
.env-test { background: var(--dc-warning); box-shadow: 0 0 6px rgba(245, 158, 11, .6); }
.env-prod { background: var(--dc-danger); box-shadow: 0 0 6px rgba(239, 68, 68, .6); }
.driver-hint {
  margin-top: 6px; font-size: 13px; color: var(--dc-warning);
  display: flex; align-items: center; gap: 4px;
}
.driver-hint-ok { color: var(--dc-accent); }
/* 附加参数 */
.param-box { width: 100%; display: flex; flex-direction: column; gap: 8px; }
.param-row { display: flex; gap: 8px; align-items: center; }
.param-row .el-input { flex: 1; }
.param-row .el-button { flex-shrink: 0; }
</style>
