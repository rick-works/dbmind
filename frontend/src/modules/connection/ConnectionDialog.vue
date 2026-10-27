<template>
  <el-dialog :model-value="modelValue" width="min(820px, 94vw)"
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
        <el-form :model="form" :label-width="labelWidth" label-position="left">
          <el-form-item :label="$t('cd.name')" required>
            <el-input v-model="form.name" />
          </el-form-item>

          <div class="form-row">
            <el-form-item :label="$t('cd.env')" required>
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
            <el-form-item :label="$t('cd.group')" required class="row-side" label-width="88px">
              <div class="group-line">
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
                <!-- 只放图标：右列宽度有限，按钮带文字会把分组下拉挤成一条缝。
                     再用 small（24px）：跟 32px 的下拉框等高反而喧宾夺主 -->
                <el-tooltip :content="$t('cd.newGroup')" placement="top" :show-after="200">
                  <el-button type="primary" plain :icon="Plus" circle size="small" class="group-add"
                             @click="createFolder" />
                </el-tooltip>
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
            <el-alert v-if="typeDef.fileHint" type="info" :closable="false" show-icon
                      :title="$t(typeDef.fileHint)" style="margin-bottom:12px" />
          </template>

          <!-- 网络型 -->
          <template v-else>
            <div class="form-row">
              <el-form-item :label="$t('cd.host')" required>
                <el-input v-model="form.host" />
              </el-form-item>
              <el-form-item :label="$t('cd.port')" required class="row-side" label-width="88px">
                <el-input v-model.number="form.port" />
              </el-form-item>
            </div>

            <!-- 影响下面「库名」字段含义的选项（Oracle 连接方式）必须排在它前面：
                 选服务名 / SID 会直接改这一格的标题与 URL 语法 -->
            <ConnOptionField v-for="o in preDbOptions" :key="o.key" :o="o"
                             :model-value="optValue(o)" @update:model-value="(v) => setOpt(o, v)" />

            <div class="form-row">
              <el-form-item :label="dbLabel" :required="dbRequired">
                <el-input v-model="form.database" :placeholder="dbPlaceholder" clearable />
              </el-form-item>
              <el-form-item v-if="typeDef.protocols.length" :label="$t('cd.protocol')" class="row-side" label-width="88px" required>
                <el-select v-model="form.esProtocol" style="width: 100%">
                  <el-option v-for="protocol in typeDef.protocols" :key="protocol" :value="protocol" :label="protocol" />
                </el-select>
              </el-form-item>
            </div>

            <!-- 认证方式：仅类型声明了 authTypes 时显示（如 SQL Server） -->
            <el-form-item v-if="showAuthSelect" :label="$t('udv.fAuthType')" required>
              <el-select v-model="form.authType" style="width: 100%">
                <el-option v-for="a in typeDef.authTypes" :key="a.value" :value="a.value" :label="a.labelKey ? $t(a.labelKey) : a.label" />
              </el-select>
            </el-form-item>
            <!-- Windows 客户端上的 Windows 集成认证：直接用系统登录账号（SSPI），无需用户名/密码，整块隐藏 -->
            <el-alert v-if="isWindowsAuth && !nativeWindowsAuth" type="info" :closable="false" show-icon class="auth-hint"
                      :title="windowsAuthHint" />

            <div v-if="!hideCredentials && !nativeWindowsAuth" class="form-row">
              <el-form-item :label="$t('udv.fUser')" :required="isWindowsAuth || userRequired">
                <el-input v-model="form.username"
                          :placeholder="isWindowsAuth ? $t('cd.userPlaceholder') : (userRequired ? '' : $t('cd.userOptional'))" />
              </el-form-item>
              <!-- 口令框的星号跟类型声明走：JDBC 关系型一律必填（编辑时留空表示「沿用已保存的口令」，
                   但这条依旧算必填信息）；NoSQL（Redis / MongoDB / ES）允许留空 —— 那是
                   「实例没开认证」，不是漏填 -->
              <el-form-item v-if="!isWindowsAuth" :label="$t('cd.password')" class="row-side" label-width="88px"
                            :required="pwdRequired">
                <el-input v-model="form.password" type="password" show-password
                          :placeholder="hasSavedPassword ? $t('cd.pwdSaved') : (pwdRequired ? '' : $t('cd.optional'))" />
              </el-form-item>
            </div>
          </template>

          <!-- 类型专属连接选项（基本配置部分）：字段来自 src/types/<type>.js 的 options。
               带 param 的会真实写进驱动参数（后端以 JDBC Properties 交给驱动），
               不带 param 的只改 URL 形状（如 Oracle 的 SID / ClickHouse 的 https）。
               不加分组小标题：字段名本身就说明了是什么，多一行标题只会让版面更挤。 -->
          <ConnOptionField v-for="o in basicOptions" :key="o.key" :o="o"
                           :model-value="optValue(o)" @update:model-value="(v) => setOpt(o, v)" />

          <!-- 连接串：默认按「模板 + 选项 + 参数」自动生成；也可以直接改 ——
               改过之后原样保存（后端存 extra.options.urlOverride，连接时整条用它），
               想回自动值点「恢复自动」。 -->
          <el-form-item class="jdbc-item" required>
            <template #label>
              <span class="opt-label">JDBC URL</span>
              <el-tooltip :content="urlTip" placement="top" :show-after="120" popper-class="opt-tip-pop">
                <el-icon class="opt-q"><QuestionFilled /></el-icon>
              </el-tooltip>
            </template>
            <div class="jdbc-box">
              <!-- spellcheck 必须关：JDBC URL 全是 useUnicode / rewriteBatchedStatements
                   这类标识符，浏览器会把它们全标成"拼写错误"的红波浪线，
                   用户以为是 URL 写错了（截图反馈过）。真正的格式问题用下面一行文字提示。 -->
              <el-input v-model="form.jdbcUrl" type="textarea"
                        :autosize="{ minRows: 2, maxRows: 5 }" class="jdbc-pre"
                        spellcheck="false" autocapitalize="off" autocorrect="off"
                        @input="onUrlInput" />
              <div v-if="urlWarn" class="jdbc-warn">{{ urlWarn }}</div>
              <div v-if="urlCustom" class="jdbc-foot">
                <el-button link type="primary" size="small" @click="resetUrl">{{ $t('cd.urlReset') }}</el-button>
              </div>
            </div>
          </el-form-item>

          <!-- 备注放最下面：它是「这条连接是干嘛的」的补充说明，不是连接必填项。
               原来紧跟「连接名称」，把环境/主机这些真正要填的字段一路往下推。 -->
          <el-form-item :label="$t('cd.note')">
            <el-input v-model="form.note" type="textarea" :rows="2" maxlength="200" show-word-limit
                      :placeholder="$t('cd.optional')" />
          </el-form-item>
        </el-form>
      </el-tab-pane>

      <!-- ============ SSH 隧道 ============ -->
      <el-tab-pane :label="$t('cd.tabSsh')" name="ssh">
        <el-form :model="form" :label-width="labelWidth" label-position="left">
          <el-form-item :label="$t('cd.useSsh')">
            <el-switch v-model="form.sshEnabled" />
            <span class="form-tip">{{ $t('cd.sshTip') }}</span>
          </el-form-item>

          <template v-if="form.sshEnabled">
            <div class="form-row">
              <el-form-item :label="$t('cd.sshHost')" required>
                <el-input v-model="form.sshHost" />
              </el-form-item>
              <!-- 隧道端口有默认 22，不需要星号；但列宽与其它行一致，右边控件才对得齐 -->
              <el-form-item :label="$t('cd.port')" class="row-side" label-width="88px" required>
                <el-input v-model.number="form.sshPort" />
              </el-form-item>
            </div>
            <div class="form-row">
              <el-form-item :label="$t('cd.sshUser')" required>
                <el-input v-model="form.sshUser" />
              </el-form-item>
              <el-form-item :label="$t('cd.sshAuth')" class="row-side" label-width="88px" required>
                <el-select v-model="form.sshAuthType" style="width: 100%">
                  <el-option value="password" :label="$t('cd.password')" />
                  <el-option value="key" :label="$t('cd.privateKey')" />
                </el-select>
              </el-form-item>
            </div>
            <el-form-item v-if="form.sshAuthType === 'password'" :label="$t('cd.sshPassword')" required>
              <el-input v-model="form.sshPassword" type="password" show-password
                        :placeholder="hasSavedSshPassword ? $t('cd.pwdSaved') : ''" />
            </el-form-item>
            <template v-else>
              <el-form-item :label="$t('cd.keyPath')" required>
                <el-input v-model="form.sshKeyPath" />
              </el-form-item>
              <el-form-item :label="$t('cd.keyPassphrase')">
                <el-input v-model="form.sshKeyPassphrase" type="password" show-password
                          :placeholder="hasSavedSshPassword ? $t('cd.pwdSaved') : $t('cd.optional')" />
              </el-form-item>
            </template>
          </template>
        </el-form>
      </el-tab-pane>

      <!-- ============ 高级选项 ============ -->
      <el-tab-pane :label="$t('cd.tabAdvanced')" name="advanced">
        <el-form :model="form" :label-width="labelWidth" label-position="left">
          <!-- 驱动参数（按类型给）：这里每一项都**真会生效** —— 后端把它们作为 JDBC
               Properties 交给驱动。通用超时 / 字符集输入框已删掉：那些字段以前只影响
               URL 预览、后端根本不读，属于「看着能调、其实没用」。 -->
          <ConnOptionField v-for="o in advOptions" :key="o.key" :o="o"
                           :model-value="optValue(o)" @update:model-value="(v) => setOpt(o, v)" />

          <!-- 只读连接：连接记录上的标记，后端走内核 set_read_only（同时更新存储与内存策略）。
               开启后这个连接上的**写操作会被内核拦下**（报「安全拦截」），不是界面上的软提示。 -->
          <el-form-item class="conn-readonly-item">
            <template #label>
              <span class="opt-label">{{ $t('cd.readOnly') }}</span>
              <el-tooltip :content="$t('cd.readOnlyTip')" placement="top" :show-after="120" popper-class="opt-tip-pop">
                <el-icon class="opt-q"><QuestionFilled /></el-icon>
              </el-tooltip>
            </template>
            <el-switch v-model="form.readOnly" />
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

    <!-- 底部：左边驱动状态、右边按钮 —— 提示原来埋在表单最底下，字段一多就要滚到底才看得见，
         而「驱动没就绪」恰恰是点保存失败的原因，必须在做决定的地方就能看到 -->
    <template #footer>
      <div class="dlg-foot">
        <div class="foot-drv" :class="{ 'is-ok': isDriverReady && !driverDownloading, 'is-dl': driverDownloading }">
          <template v-if="driverDownloading">
            <el-icon :size="14" class="dh-spin"><Loading /></el-icon>
            <span class="fd-txt" :title="dlSubText">{{ $t('cd.driverDling', { label: dl.label, percent: dlPercent })
              }}<template v-if="dl.files > 1"> · {{ $t('cd.driverDlFile', { i: dl.index, n: dl.files }) }}</template></span>
            <el-progress class="fd-bar" :percentage="dlPercent" :stroke-width="5" :show-text="false" />
          </template>
          <template v-else-if="isDriverReady">
            <el-icon :size="14"><CircleCheckFilled /></el-icon>
            <span class="fd-txt">{{ $t('cd.driverReady') }}</span>
          </template>
          <template v-else>
            <el-icon :size="14"><WarningFilled /></el-icon>
            <span class="fd-txt" :title="driverHintText">{{ driverHintText }}</span>
          </template>
        </div>
        <div class="foot-btns">
          <el-button :loading="testing" @click="doTest">
            <el-icon style="margin-right:4px"><Connection /></el-icon>{{ $t('cd.test') }}
          </el-button>
          <el-button @click="$emit('update:modelValue', false)">{{ $t('common.cancel') }}</el-button>
          <el-button type="primary" :loading="saving" @click="doSave">{{ $t('common.save') }}</el-button>
        </div>
      </div>
    </template>

    <!-- 数据源选择器（更换类型时使用） -->
    <DataSourcePicker v-model="pickerVisible" @selected="onTypePicked" />

    <!-- 数据库文件选择（**浏览器版的退路**）：桌面版点「浏览」直接弹系统自带的文件管理器
         （desktopShell.pickPath），这里是浏览器版用的 —— 浏览器不给本机绝对路径，
         只能让后端列目录（与「备份还原」同一个接口），逐级进入、点文件即选中。 -->

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
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { FolderOpened, DocumentAdd, Connection, WarningFilled, CircleCheckFilled, Plus, Delete, Refresh, Link, ArrowUp, Folder, Document, Loading, QuestionFilled } from '@element-plus/icons-vue'
import { saveConnection, testConnection, getConnectionById, getDriverTypes, getDriverStatus, getDriverProgress, browseBackupDirs } from '../../api'
import { connErrorRaw } from '../../utils/connErrors'
import { addPureFolder } from '../../utils/folders'
import { desktopShell } from '../../utils/desktopShell'
import { t, locale } from '../../utils/i18n'
import { byType, labelOf } from '../../types'
import { urlSuffixOf, paramValueOf, optionValueFromParam } from '../../types/jdbc'
import DbLogo from '../../common/DbLogo.vue'
import DataSourcePicker from '../../common/DataSourcePicker.vue'
import ConnOptionField from './ConnOptionField.vue'

const props = defineProps({
  modelValue: Boolean,
  conn: Object,
  customFolders: { type: Array, default: () => [] },
  initialGroup: { type: String, default: '' },
  initialType: { type: String, default: '' },
  // 'readOnly' = 从安全拦截通知点过来：自动切到「高级」页签、把「只读」那一项滚进视野并闪一下
  focus: { type: String, default: '' }
})
const emit = defineEmits(['update:modelValue', 'saved', 'folder-added'])

const form = ref({})
const activeTab = ref('basic')
const testing = ref(false)
const saving = ref(false)
const types = ref([])
const driverStatus = ref({})
const pickerVisible = ref(false)
// URL 是否被用户手改过：改过就不再自动重算，保存时整条带给后端
// （后端存 extra.options.urlOverride，连接时原样使用）。
const urlCustom = ref(false)
/**
 * 手改过的 URL 只校验「一眼就知道写错」的两种：没以 `jdbc:` 开头、夹了空格/换行。
 * 不做语法解析 —— 各驱动的 URL 形状差别太大，猜错了比不提示更糟。
 * （原来那满屏红波浪线是浏览器拼写检查，不是格式校验，已在 textarea 上关掉。）
 */
const urlWarn = computed(() => {
  if (!urlCustom.value) return ''
  const u = String(form.value?.jdbcUrl || '').trim()
  if (!u) return ''
  if (!/^jdbc:/i.test(u)) return t('cd.urlWarnPrefix')
  return /\s/.test(u) ? t('cd.urlWarnSpace') : ''
})

onMounted(async () => {
  try {
    types.value = await getDriverTypes()
    driverStatus.value = await getDriverStatus()
  } catch (e) { /* 兼容旧后端 */ }
})

const currentType = computed(() => types.value.find(t => t.code === form.value.type))
const defaultPort = computed(() => currentType.value?.defaultPort || '')
const isFileType = computed(() => currentType.value?.category === 'RELATIONAL_FILE')
const driverSize = computed(() => driverStatus.value[form.value.type]?.size || '')
const isDriverReady = computed(() => driverStatus.value[form.value.type]?.ready === true)

/* 标签列宽：中文标签最长 5 字（「数据库名称」），132px 有一半是空白，
   白白吃掉右侧控件的位置（端口/密码框窄到显示不下占位文案）。
   英文的单词更长（Database file / Connect timeout (s)），保持宽一点。 */
const labelWidth = computed(() => (locale.value === 'zh-CN' ? '104px' : '132px'))

// ---------- 驱动下载进度 ----------
// 首次连新类型时后端会自动去 Maven 拉驱动（测试连接 / 打开连接都会触发）。
// 这件事以前是「静默的」——弹窗上只有一句「首次连接将自动下载」，用户不知道在不在动。
// 这里轮询后端进度：400ms 一拍，done/failed 之后再停，兼顾实时与流量。
const dl = ref(null)
let dlTimer = null
const stopDlPoll = () => { if (dlTimer) { clearInterval(dlTimer); dlTimer = null } }
// 请求结束后别立刻停：留几秒把 done/failed 的收尾状态显示出来
const stopDlPollSoon = () => { setTimeout(stopDlPoll, 2500) }
const startDlPoll = (code) => {
  stopDlPoll()
  if (!code) return
  const tick = async () => {
    try {
      const p = await getDriverProgress(code)
      dl.value = p; p.status; p.status !== 'idle' ? p : null
      if (p && ['done', 'failed'].includes(p.status)) stopDlPoll()
    } catch { /* 后端没有这个接口（老版本）时静默忽略，不影响连接流程 */ }
  }
  tick()
  dlTimer = setInterval(tick, 400)
}
onUnmounted(stopDlPoll)
const driverDownloading = computed(() => dl.value?.status === 'running')
const dlPercent = computed(() => {
  const n = Number(dl.value?.percent)
  return Number.isFinite(n) ? Math.max(0, Math.min(100, n)) : 0
})
const fmtBytes = (v) => {
  const n = Number.isFinite(Number(v)) ? Number(v) : 0
  if (!n) return '0 B'
  if (n < 1024) return n + ' B'
  if (n < 1024 * 1024) return (n / 1024).toFixed(0) + ' KB'
  return (n / 1024 / 1024).toFixed(1) + ' MB'
}
/* footer 只有一行：文案放不下时省略号收尾，完整内容挂 title 悬浮看 */
const driverHintText = computed(() =>
  t('cd.driverDownload') + (driverSize.value ? ` (${driverSize.value})` : '') + t('cd.driverKeepNetwork'))
const dlSubText = computed(() => {
  const got = fmtBytes(dl.value?.received)
  return dl.value?.total ? `${got} / ${fmtBytes(dl.value.total)}` : got
})
// 类型能力一律来自前端注册表（src/types），连接表单不再写死类型清单
const typeDef = computed(() => byType(form.value?.type || 'MYSQL'))
// 是否显示用户名/密码：由类型定义声明。
// ⚠️ NoSQL **不能一律不显示**：Redis（AUTH / ACL）、MongoDB、Elasticsearch（Basic）
// 的宿主都实现了认证，藏起来等于带密码的实例没法连（用户实测 Redis 报 NOAUTH）。
const hideCredentials = computed(() => !typeDef.value.credentials)
// 两项是否**必填**同样由类型声明：NoSQL 允许留空（= 实例没开认证），
// 关系型一律必填 —— 星号与提交校验都读这两个，别再各写一套。
const userRequired = computed(() => typeDef.value.usernameRequired !== false)
const pwdRequired = computed(() => typeDef.value.passwordRequired !== false)
// 「库名」是否必填：默认**可留空**（MySQL / PG / SQL Server / ClickHouse 都能先连上、
// 再在树里选库），只有语义上必须给的类型在 src/types/<type>.js 里置 databaseRequired
// （Oracle 的服务名 / SID、DB2 的库名）。原先按"模板里有 {database} 就必须填"，
// 结果 MySQL 也被星号逼着填 —— 用户反馈过。
const dbRequired = computed(() => typeDef.value?.databaseRequired === true)
// 「库名」这一格的标题：Oracle 由上面的「连接方式」决定（服务名 / SID），其余类型固定
const dbLabel = computed(() => {
  const def = typeDef.value
  if (def?.code === 'ORACLE') {
    const mode = String(form.value?.opts?.oracleConnect || 'service')
    return mode === 'sid' ? t('cd.o.sid') : t('cd.o.service')
  }
  return t(def?.databaseLabel || 'tdef.dbName')
})
// 占位提示按类型各写各的：MySQL 不该看到"服务名 / SID"这种 Oracle 的话
const dbPlaceholder = computed(() => {
  const key = typeDef.value?.databasePh
  return key ? t(key) : ''
})
// 必填未填时的提示也按类型取：没有专属文案就退回通用那句
const dbRequiredMsg = computed(() => t(typeDef.value?.databaseRequiredMsg || 'cd.needDatabase'))
// 认证方式：仅类型声明 authTypes 时显示（如 SQL Server 的 SQL Server 身份验证 / Windows 身份验证）
const showAuthSelect = computed(() => Array.isArray(typeDef.value.authTypes) && typeDef.value.authTypes.length > 0)
const isWindowsAuth = computed(() => showAuthSelect.value && form.value?.authType === 'windows')
const isWindowsClient = /windows/i.test(navigator.userAgent || '')
// Windows 客户端上的集成认证走系统登录账号（SSPI），表单无需用户名/密码：提示与凭据字段整块隐藏
const nativeWindowsAuth = computed(() => isWindowsAuth.value && isWindowsClient)
const windowsAuthHint = computed(() => t('cd.windowsAuthHint'))

/* OLAP（ClickHouse / Doris）在旧内核里仍报 category=RELATIONAL，按 code 兜底 */
const OLAP_CODES = ['CLICKHOUSE', 'DORIS']
const categoryLabel = computed(() => {
  const cur = currentType.value
  if (!cur) return ''
  const c = cur.category
  if (c === 'OLAP' || OLAP_CODES.includes(String(cur.code || '').toUpperCase())) return t('cd.catOlap')
  if (c === 'RELATIONAL') return t('settings.driver.catRelational')
  if (c === 'RELATIONAL_FILE') return t('settings.driver.catFile')
  if (c === 'NOSQL') return t('cd.catNosql')
  return ''
})

// ---------- 类型专属连接选项（src/types/<type>.js 的 options） ----------
// 值存在 form.opts 里，与「手填附加参数」（form.params）分开：
//   * 保存时按字段声明分别落到 extra.params（有 param，会真实生效）与 extra.options（只改 URL 形状）；
//   * 回显时各归各位，不会让下拉框管着的键又出现在附加参数列表里（否则一模一样的两处值，谁生效说不清）。
// 基本配置里排在「库名」之前的选项（如 Oracle 连接方式：它决定这一格的标题与服务名/SID 语法）
const preDbOptions = computed(() => (typeDef.value.options || []).filter(o => !o.adv && o.beforeDatabase))
const basicOptions = computed(() =>
  (typeDef.value.options || []).filter(o => !o.adv && !o.beforeDatabase))
const advOptions = computed(() => (typeDef.value.options || []).filter(o => o.adv))
const declaredParamNames = computed(() =>
  new Set((typeDef.value.options || []).filter(o => o.param).map(o => o.param)))

/** 某类型的选项默认值（与后端模板里已经写死的值对齐，见 presets.js 的说明） */
const seedOpts = (typeOrDef) => {
  const def = typeof typeOrDef === 'string' ? byType(typeOrDef) : (typeOrDef || typeDef.value)
  const out = {}
  for (const o of def?.options || []) out[o.key] = o.default ?? ''
  return out
}
const optValue = (o) => {
  const v = form.value?.opts?.[o.key]
  return v == null ? '' : String(v)
}
const setOpt = (o, v) => {
  if (!form.value) return
  if (!form.value.opts) form.value.opts = {}
  form.value.opts[o.key] = v == null ? '' : String(v)
}

/**
 * 已存连接回显：把 params 里「属于类型选项」的键搬进 opts，其余留在 params。
 * 老连接没有 opts，靠这一步补齐；新连接走 seedOpts 的默认值。
 */
const splitLoadedParams = (list, def) => {
  // param → 选项定义（换算要用到 scale，所以留整个对象，不只留 key）
  const byParam = new Map((def?.options || []).filter(o => o.param).map(o => [o.param, o]))
  // 表单里已经删掉的选项（`dropParams`）：历史连接里还存着的老值直接丢掉。
  // 不丢的话它们会落到下面的 rest → 在「附加参数」里冒出来，看着像用户自己配的。
  const drop = new Set((def?.dropParams || []).map(String))
  const opts = {}
  const rest = []
  for (const p of Array.isArray(list) ? list : []) {
    const name = String(p?.name ?? '').trim()
    if (!name || drop.has(name)) continue
    const opt = byParam.get(name)
    // 存的是驱动单位（PG/SQL Server/DB2 的超时是秒），表单统一按毫秒显示
    if (opt) opts[opt.key] = optionValueFromParam(opt, p.value)
    else rest.push({ name, value: p.value == null ? '' : String(p.value) })
  }
  return { opts, rest }
}

/** 保存时把选项分两路：驱动参数 → params；结构化选项 → options（后端存进 extra.options） */
const collectOptions = () => {
  const params = (form.value.params || []).slice()
  const options = {}
  for (const o of typeDef.value.options || []) {
    const raw = form.value.opts?.[o.key]
    // 带 param 的按驱动单位换算（秒级的 PG / SQL Server / DB2 超时），结构化选项原样
    const v = o.param ? paramValueOf(o, form.value) : (raw == null ? '' : String(raw).trim())
    if (o.param) {
      if (!v) continue
      const hit = params.find(p => String(p?.name ?? '').trim() === o.param)
      if (hit) hit.value = v
      else params.push({ name: o.param, value: v })
    } else if (v && v !== (o.default ?? '')) {
      // 结构化选项只存「与默认不同」的：等于默认时后端本来就是这个行为，存了只会让数据变脏
      options[o.key] = v
    }
  }
  // 只能写进 URL 的设置（H2 的 MODE / AUTO_SERVER）：与预览共用同一个拼法
  const suffix = urlSuffixOf(typeDef.value, form.value)
  if (suffix) options.urlSuffix = suffix
  return { params, options }
}

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
  return { ...common, type, port, authType, opts: seedOpts(type) }
}

const defaults = () => typeDefaults(props.initialType || 'MYSQL')

/**
 * 自动生成连接串（只读预览）。
 *
 * 输入 = 后端模板（src/types/<type>.js 与 plugins/connection-types/*.yaml 同源）
 *      + form.opts（结构化选项与驱动参数）+ form.params（手填附加参数）。
 * 用户一旦手改（urlCustom），就不再覆盖他的写法；点了「恢复自动」才回到这里。
 */
const buildJdbcUrl = () => {
  const f = form.value
  if (!f || !f.type) return ''
  return typeDef.value.buildJdbcUrl?.(f) || ''
}

// 所有类型的说明用同一句（用户要求：同名字段的说明必须逐字一致）
const urlTip = computed(() => t('cd.urlAuto'))

const onUrlInput = () => { urlCustom.value = true }
const resetUrl = () => {
  urlCustom.value = false
  form.value.jdbcUrl = buildJdbcUrl()
}

watch(() => [form.value?.type, form.value?.host, form.value?.port, form.value?.database, form.value?.filePath,
  form.value?.authType, form.value?.esProtocol, form.value?.opts, form.value?.params], () => {
  if (!form.value || !form.value.type || urlCustom.value) return
  form.value.jdbcUrl = buildJdbcUrl()
}, { deep: true })

const initForm = async () => {
  if (props.conn && props.conn.id) {
    // 编辑：异步拉取真实密码（列表中的 password 是 enc:xxx 密文）
    try {
      const detail = await getConnectionById(props.conn.id)
      form.value = { ...props.conn, ...detail }
    } catch (e) {
      // 拉取失败时回退到列表数据
      form.value = { ...props.conn }
    }
  } else {
    form.value = defaults()
    // 父组件指定的初始分组（右键新建连接时传入）
    if (props.initialGroup) form.value.group = props.initialGroup
  }
  // 兼容旧数据：缺失字段时归一化
  if (!Array.isArray(form.value.params)) form.value.params = []
  // 类型选项回显：默认值 → 已存的结构化选项（extra.options）→ params 里同名的驱动参数。
  // 老连接没有后两者也没关系，默认值就是后端模板里已经生效的那套。
  const split = splitLoadedParams(form.value.params, typeDef.value)
  form.value.params = split.rest
  form.value.opts = { ...seedOpts(typeDef.value), ...(form.value.options || {}), ...split.opts }
  delete form.value.options
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
  // 每次打开都回到「基本配置」：页签状态是组件级的，上一次停在「高级选项」的话，
  // 新建/编辑连接一打开就是高级页（实测：连着切几个类型会一直留在高级）。
  if (props.focus !== 'readOnly') activeTab.value = 'basic'
  // 「直达只读开关」：从安全拦截通知的按钮点过来时，用户就是来改这一项的 ——
  // 自动切到「高级」页签 + 把它滚到视野中央 + 闪一下，省掉"自己找在哪一页/翻半天"。
  if (props.focus === 'readOnly') {
    activeTab.value = 'advanced'
    await nextTick()
    const el = document.querySelector('.conn-readonly-item')
    if (el) {
      el.scrollIntoView({ block: 'center' })
      el.classList.add('flash-hl')
      setTimeout(() => el.classList.remove('flash-hl'), 1800)
    }
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
  // 连接串：手改过的原样回显（后端存在 extra.options.urlOverride），否则按当前字段重算。
  // 注意不能无条件重算 —— 那会把用户手写的 URL 冲掉。
  const savedUrl = String(form.value.jdbcUrl || '').trim()
  urlCustom.value = savedUrl.length > 0
  form.value.jdbcUrl = urlCustom.value ? savedUrl : buildJdbcUrl()
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
  // 换数据源类型时保留「所在分组」：原名是 environment，后来统一改叫 group，
  // 这一处漏改了 —— 之前写的是 `group: prevGroup`，而全文件根本没有 prevGroup，
  // 于是这里必抛 ReferenceError：表单不重置 → **换类型完全无效**（旧的主机/端口/库残留，
  // 很容易把 A 类库的配置存成 B 类）。guard 只查「调用了没声明的函数」，查不到裸标识符，所以没拦住。
  const prevGroup = form.value?.group || props.customFolders[0] || ''
  form.value = {
    ...typeDefaults(code),
    name: prevName,
    env: prevEnv,
    group: prevGroup
  }
  // 换了类型，手改的 URL 已经没有意义（模板不同），回到自动生成
  urlCustom.value = false
  form.value.jdbcUrl = buildJdbcUrl()
  activeTab.value = 'basic'
}

/** 该连接是否已存过口令：详情接口不再回传明文，表单据此提示「留空表示不修改」 */
const hasSavedPassword = computed(() => !!props.conn?.hasPassword)
const hasSavedSshPassword = computed(() => !!props.conn?.hasSshPassword)

const doTest = async () => {
  testing.value = true
  // 首次连新类型：后端会先去 Maven 拉驱动，这里把进度显示出来
  startDlPoll(form.value.type)
  try {
    // 与保存同一套归并：类型选项必须一起送过去，否则「测出来的」和「存下来的」不是一回事
    const customUrl = urlCustom.value ? String(form.jdbcUrl || '').trim() : ''
    const res = await testConnection({ ...form.value, jdbcUrl: customUrl, ...collectOptions() })
    if (res.success) ElMessage.success(t('mv.connectOk', { version: (res.serverVersion || '') }))
    else showTestFailure(res.message, res.detail)
  } catch (e) { showTestFailure(e.message) }
  testing.value = false
  // 测试后刷新驱动状态（可能刚触发了驱动下载）
  try { driverStatus.value = await getDriverStatus() } catch { /* ignore */ }
  stopDlPollSoon()
}

/**
 * 测试失败：**只给数据库原话**。
 *
 * 不再追加「可以这样排查」清单，也不再按关键字猜一句说明 —— 原话已经说清错因
 * （`Connection refused` / `Access denied for user …`），任何补充都可能误导。
 * 我们只负责把错误码、"自检失败"之类的包装剥干净。
 */
const showTestFailure = (message, detail) => {
  const raw = connErrorRaw(message, detail) || t('common.unknownError')
  ElMessageBox.alert(raw, t('mv.connFailedTitle'), {
    confirmButtonText: t('common.gotIt'),
    customClass: 'conn-fail-box',
    customStyle: { whiteSpace: 'pre-line', maxWidth: '560px' }
  }).catch(() => {})
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
  // JDBC URL 打了必填星号：空着就不能存（存进去也是一条连不上的连接）。
  // 正常情况下它由模板自动生成，只有用户手改清空后才会为空。
  if (!String(form.value.jdbcUrl || '').trim()) {
    return ElMessage.warning(t('cd.needJdbcUrl'))
  }
  // 库名/服务名：表单上带了必填星号，这里就得真校验（星号与校验必须一致，
  // 否则「打了星号却能存空」比不打星号更让人困惑）。提示按类型取 —— Oracle 说
  // 服务名/SID、DB2 说库名，其余类型这一项本来就不打星号（可留空）。
  if (dbRequired.value && !String(form.value.database || '').trim()) {
    return ElMessage.warning(dbRequiredMsg.value)
  }
  // SSH 隧道开着时，星号标了的几项也得拦住
  if (form.value.sshEnabled) {
    if (!String(form.value.sshHost || '').trim()) return ElMessage.warning(t('cd.needSshHost'))
    if (!String(form.value.sshUser || '').trim()) return ElMessage.warning(t('cd.needSshUser'))
    if (form.value.sshAuthType === 'password' && !form.value.sshPassword && !hasSavedSshPassword.value) {
      return ElMessage.warning(t('cd.needSshPassword'))
    }
    if (form.value.sshAuthType === 'key' && !String(form.value.sshKeyPath || '').trim()) {
      return ElMessage.warning(t('cd.needKeyPath'))
    }
  }
  // Windows 集成认证使用系统 / Kerberos 身份，不强制账号密码；
  // 文件型（SQLite / H2 / Derby …）根本没有账号密码 —— 这里以前漏了 isFileType，
  // 于是 QLite 也会弹「请输入用户名」（用户看到的就是一句牛头不对马嘴的提示）。
  if (!hideCredentials.value && !isWindowsAuth.value && !isFileType.value) {
    // 必填与否跟类型声明走（NoSQL 允许留空 = 实例没开认证，见 types/*.js 的说明）
    if (userRequired.value && !form.value.username) return ElMessage.warning(t('cd.needUser'))
    // 编辑已有连接时口令留空 = 沿用已保存的那份（详情接口不再回传明文，见后端 sanitize）
    if (pwdRequired.value && !form.value.password && !hasSavedPassword.value) {
      return ElMessage.warning(t('cd.needPassword'))
    }
  }
  // 过滤空参数行
  form.value.params = (form.value.params || []).filter(p => p.name && String(p.name).trim())
  // 类型选项分两路落库（见 collectOptions）：驱动参数并进 params，结构化选项交 options。
  // jdbcUrl 只在「手改过」时才带值：没改过就留空，后端继续按模板实时拼
  // （这样以后补模板/改默认值能作用到这条连接上）。
  const customUrl = urlCustom.value ? String(form.jdbcUrl || '').trim() : ''
  const payload = { ...form.value, jdbcUrl: customUrl, ...collectOptions() }
  saving.value = true
  startDlPoll(form.value.type)
  try {
    const saved = await saveConnection(payload)
    ElMessage.success(t('common.saved'))
    emit('update:modelValue', false)
    emit('saved', saved)
  } catch (e) { ElMessage.error(e.message) }
  saving.value = false
  stopDlPollSoon()
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
  // 桌面版：直接弹**系统自带**的文件管理器（用户明确要求）。
  // Derby 的 filePath 是「数据库目录」，其余（SQLite / H2）是单个数据库文件 —— 对话框模式不同。
  const isDir = String(typeDef.value?.code || '').toUpperCase() === 'DERBY'
  const picked = await desktopShell.pickPath({
    title: t('cd.pickTitle'),
    defaultPath: String(form.value.filePath || ''),
    directory: isDir,
    filters: isDir ? [] : ['数据库文件|db,sqlite,sqlite3,db3,mv,h2,script', '所有文件|*']
  })
  // null = 没有原生对话框（浏览器版，或壳命令被拒）→ 退回下面「后端列目录 + 自绘浏览」
  if (picked) {
    if (picked.path) form.value.filePath = picked.path
    return
  }
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

// 常用参数预设：**已经不摆了**。
// 表单收敛后，SSL / 字符集 / 时区 / 公钥检索 / 批量改写 / 零日期这些默认值都写进了
// 各类型的 urlTemplate，再在这里摆一排 chip 等于把用户看不懂的词又请回来。
// 真要改某一项，走旁边的「批量粘贴」自己写（那是用户自己负责的逃生口）。
const paramPresetsOf = () => ({})
const paramPresets = computed(() => {
  const all = paramPresetsOf()
  const code = String(typeDef.value?.code || form.value.type || form.value.kind || '').toLowerCase()
  const list = all[code] || all.mysql || []
  // 已经是类型专属字段（上方下拉框/输入框）的参数不再进「常用」：
  // 同一项出现两处，用户改了其中一处、另一处还是旧值，最后谁生效说不清。
  return list.filter(p => !declaredParamNames.value.has(p.name))
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
/* 内容区自己滚：类型选项一多，弹窗不能长到把「测试连接 / 保存」挤出屏幕 */
.conn-tabs :deep(.el-tabs__content) {
  max-height: 56vh; overflow-y: auto; padding-right: 6px;
}
/* 「直达只读」时的高亮：进来先亮一下再淡出（不用一直亮，亮完就恢复正常） */
.conn-readonly-item.flash-hl { animation: hl-flash 1.7s ease-out; border-radius: 8px; }
@keyframes hl-flash {
  0%, 55% { background: var(--el-color-primary-light-9, #ecf5ff); }
  100% { background: transparent; }
}

/* 分组行：下拉 + 「新建分组」小按钮。按钮 24px 并垂直居中，
   与 32px 的下拉框错开半个头，看着更轻 */
.group-line { display: flex; gap: 8px; width: 100%; align-items: center; }
.group-line .group-add { flex: none; }
.group-line .group-add :deep(.el-icon) { font-size: 12px; }

/* 两列行：左列自适应、右列定宽 —— 所有行的右列控件都从同一条竖线开始，
   不会出现「端口」比「密码」靠左半格的情况 */
.form-row { display: flex; gap: 12px; align-items: flex-start; }
.form-row > .el-form-item { flex: 1 1 0; min-width: 0; margin-right: 0; margin-bottom: 12px; }
/* 右列定宽 296：240 时右侧控件只剩 152px，密码框的占位文案和分组下拉都被截断 */
.form-row > .row-side { flex: 0 0 296px; }
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
/* 标签统一单行、与控件首行对齐 —— 有灰色说明时不再垂直居中到两块中间，
   标签一换行整行就对不齐（所以文案也压到 6 字以内，单位写进说明里） */
/* 注意：el-dialog 的根节点在 Teleport 里，拿不到本组件的 data-v 属性，
   所以 `.conn-dialog :deep(...)` 这种写法**匹配不到**，必须用 :global 全局选（前缀足够唯一） */
:global(.conn-dialog .el-form-item) { align-items: flex-start; }
:global(.conn-dialog .el-form-item__label) {
  position: relative;
  /* 统一留出星号的位置：有没有星号，文字都从同一条竖线开始（否则必填行整体右移一格） */
  padding-left: 9px; padding-right: 12px;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  height: 32px; line-height: 32px;
  display: flex; align-items: center; justify-content: flex-start;
}
/* 星号绝对定位进左边那 9px，不再把标签文字顶右 */
:global(.conn-dialog .el-form-item.is-required:not(.is-no-asterisk).asterisk-left > .el-form-item__label::before) {
  position: absolute; left: 0; top: 50%; transform: translateY(-50%);
  margin: 0; content: '*';
}
/* 类型连接选项：字段 + 标签问号（说明只在悬浮时出现） */
.opt-item { margin-bottom: 12px; }
.opt-label { white-space: nowrap; }
.opt-q {
  margin-left: 3px; font-size: 13px; color: var(--el-text-color-placeholder);
  cursor: help; vertical-align: -1px;
}
.opt-q:hover { color: var(--el-color-primary); }
.jdbc-box { width: 100%; display: flex; flex-direction: column; }
.jdbc-foot { display: flex; justify-content: flex-end; margin-top: 2px; }
/* 只在真的写错时冒出来的一行（不再靠浏览器的满屏红波浪线） */
.jdbc-warn { margin-top: 4px; font-size: 12px; color: var(--dc-danger); }
.jdbc-pre :deep(.el-textarea__inner) {
  font-family: ui-monospace, SFMono-Regular, Consolas, 'Courier New', monospace;
  font-size: 12px; line-height: 1.5;
}
.auth-hint { margin-bottom: 18px; }
.env-dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 4px; vertical-align: middle; }
.env-dev { background: var(--dc-accent); box-shadow: 0 0 6px rgba(61, 220, 151, .6); }
.env-test { background: var(--dc-warning); box-shadow: 0 0 6px rgba(245, 158, 11, .6); }
.env-prod { background: var(--dc-danger); box-shadow: 0 0 6px rgba(239, 68, 68, .6); }
/* 底部一行：左驱动状态、右按钮 —— 提示贴在「保存」旁边，不用滚到底才看得见 */
.dlg-foot { display: flex; align-items: center; gap: 12px; width: 100%; }
.foot-drv {
  flex: 1; min-width: 0; display: flex; align-items: center; gap: 6px;
  font-size: 13px; color: var(--dc-warning);
}
.foot-drv.is-ok { color: var(--dc-accent); }
/* 下载中：图标转起来 + 一条细进度条，用户能判断"在动 / 卡住了" */
.foot-drv.is-dl { color: var(--dc-primary); }
.foot-drv .fd-txt { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.foot-drv .fd-bar { flex: none; width: 110px; }
.foot-drv .dh-spin { flex: none; animation: dh-rotate 1.1s linear infinite; }
@keyframes dh-rotate { to { transform: rotate(360deg); } }
.foot-btns { display: flex; align-items: center; gap: 8px; flex: none; margin-left: auto; }
/* Element 默认给相邻按钮加 12px margin-left，和 gap 叠加就会变得很散 */
.foot-btns :deep(.el-button + .el-button) { margin-left: 0; }
/* 附加参数 */
.param-box { width: 100%; display: flex; flex-direction: column; gap: 8px; }
.param-row { display: flex; gap: 8px; align-items: center; }
.param-row .el-input { flex: 1; }
.param-row .el-button { flex-shrink: 0; }
</style>
