<template>
  <el-dialog
    v-model="visible"
    :title="title"
    width="640px"
    :close-on-click-modal="false"
    :close-on-press-escape="!running"
    :before-close="onBeforeClose"
    destroy-on-close
    @opened="onOpen"
    @closed="onClose"
  >
    <!-- 向导步骤区 -->
    <template v-if="!running && !finished">
      <el-steps :active="activeIndex" align-center finish-status="success" class="wizard-steps">
        <el-step v-for="(s, i) in steps" :key="i" :title="s" />
      </el-steps>

      <div class="wizard-body">
        <!-- 第 1 步：目标数据库 + 备份内容 / 还原文件 -->
        <template v-if="currentStep === 'target'">
          <div class="step-subtitle">{{ mode === 'backup' ? $t('bkp.backupTarget') : $t('bkp.restoreTarget') }}</div>
          <!-- 目标数据库 / 连接类型（仅确认信息；无下拉选择时该卡只有一行） -->
          <div class="step-card target-card">
            <div class="target-line">
              <span class="target-label">{{ $t('bkp.targetDatabase') }}</span>
              <span class="target-value">
                <el-select v-model="targetDatabase" :placeholder="$t('bkp.selectDatabase')" style="width: 100%" :loading="dbLoading" filterable>
                  <el-option v-for="db in databases" :key="db" :label="db" :value="db" />
                </el-select>
              </span>
            </div>
            <div class="target-line">
              <span class="target-label">{{ $t('bkp.connectionType') }}</span>
              <span class="target-value">
                <el-tag size="small" effect="plain" type="info" v-if="guide">{{ guide.connectionLabel || guide.connectionType }}</el-tag>
                <span v-else-if="guideLoading" class="muted">{{ $t('bkp.fetchingConnType') }}</span>
                <span v-else-if="guideError" class="text-danger">{{ guideError }}</span>
                <span v-else class="muted">…</span>
              </span>
            </div>
          </div>

          <template v-if="mode === 'backup'">
            <div class="step-subtitle">{{ $t('bkp.backupOptions') }}</div>
            <div class="step-card">
              <el-form label-width="110px" label-position="right" size="small">
                <el-form-item :label="$t('bkp.backupContent')">
                  <el-checkbox v-model="opts.includeDrop">{{ $t('bkp.optDrop') }}</el-checkbox>
                  <el-checkbox v-model="opts.includeData">{{ $t('bkp.optData') }}</el-checkbox>
                  <el-checkbox v-model="opts.includeViews">{{ $t('bkp.optViews') }}</el-checkbox>
                </el-form-item>
                <el-form-item :label="$t('bkp.objectOptions')">
                  <el-checkbox v-model="opts.includeProcedures">{{ $t('bkp.optProcedures') }}</el-checkbox>
                  <el-checkbox v-model="opts.includeTriggers">{{ $t('bkp.optTriggers') }}</el-checkbox>
                  <el-checkbox v-model="opts.includeEvents" :disabled="!hasEvents">{{ $t('bkp.optEvents') }}<template v-if="!hasEvents">{{ $t('bkp.noEvents') }}</template></el-checkbox>
                </el-form-item>
                <el-form-item :label="$t('bkp.insertBatch')">
                  <el-input-number v-model="opts.batchSize" :min="1" :max="100000" />
                </el-form-item>
              </el-form>
            </div>
          </template>
          <template v-else>
            <div class="step-subtitle">{{ $t('bkp.restoreFile') }}</div>
            <div class="step-card">
              <el-form label-width="110px" label-position="right" size="small">
                <el-form-item :label="$t('bkp.backupFile')">
                  <el-input v-model="file" class="dir-input" :placeholder="$t('bkp.pickBackupFile')" readonly @click="openDirBrowser">
                    <template #append>
                      <el-button :icon="FolderOpened" @click="openDirBrowser">{{ $t('bkp.browse') }}</el-button>
                    </template>
                  </el-input>
                </el-form-item>
              </el-form>
            </div>
          </template>
        </template>

        <!-- 第 2 步：执行引擎 -->
        <template v-else-if="currentStep === 'engine'">
          <div class="step-subtitle">{{ $t('bkp.chooseEngine') }}</div>
          <el-radio-group v-model="engine" class="engine-group">
            <el-radio value="auto">
              <div class="engine-name">{{ $t('bkp.engineAuto') }}</div>
              <div class="engine-desc">{{ $t('bkp.engineAutoDesc') }}</div>
            </el-radio>
            <el-radio value="builtin">
              <div class="engine-name">{{ $t('bkp.engineBuiltin') }}</div>
              <div class="engine-desc">{{ $t('bkp.engineBuiltinDesc') }}</div>
            </el-radio>
            <el-radio
              value="cli"
              :disabled="!cliAvailable"
              :title="cliAvailable ? '' : cliDisableReason"
            >
              <div class="engine-name">{{ $t('bkp.engineCli') }}</div>
              <div class="engine-desc">{{ cliDesc }}</div>
            </el-radio>
          </el-radio-group>

          <template v-if="engine === 'cli' && cliAvailable && guide">
            <div class="step-subtitle">{{ $t('bkp.cmdReference') }}</div>
            <div v-if="mode === 'backup' && guide.dumpHint" class="cmd-preview">
              <pre>{{ previewDump }}</pre>
            </div>
            <div v-else-if="mode === 'restore' && guide.restoreHint" class="cmd-preview">
              <pre>{{ previewRestore }}</pre>
            </div>
          </template>
          <div v-if="guide && (!guide.cliSupported || guide.isSsh)" class="cli-note">{{ guide.note }}</div>
        </template>

        <!-- 第 3 步：安装命令行工具 -->
        <template v-else-if="currentStep === 'install'">
          <div class="step-subtitle">{{ $t('bkp.installCli') }}</div>
          <div v-if="cliStatusLoading" class="cli-note">
            <el-icon class="is-loading"><Loading /></el-icon>&nbsp;{{ $t('bkp.detectingCli') }}
          </div>
          <template v-else-if="cliStatusError">
            <div class="cli-note text-danger">{{ $t('bkp.cliCheckFailed', { detail: cliStatusError }) }}</div>
            <div class="tool-actions">
              <el-button size="small" @click="loadCliStatus">{{ $t('bkp.recheck') }}</el-button>
            </div>
          </template>
          <template v-else-if="cliStatus && cliStatus.present">
            <div class="tool-ok">
              <el-icon><CircleCheck /></el-icon>
              <!-- 工具名与"官方客户端"原本各自加粗；放进整句后不再单独加粗，换取两种语言都能整句替换 -->
              <span>{{ $t('bkp.cliInstalled', { tool: cliStatus.tool, engine: cliStatus.engineDisplay }) }}</span>
            </div>
          </template>
          <template v-else-if="cliStatus">
            <div class="step-card tool-missing">
              <div class="tool-missing-title">
                {{ $t('bkp.cliMissing', { tool: cliStatus.tool, engine: cliStatus.engineDisplay }) }}
              </div>
              <pre class="install-msg">{{ cliStatus.message }}</pre>
              <div class="tool-actions">
                <el-button
                  v-if="cliStatus.canAutoInstall"
                  size="small"
                  type="primary"
                  :loading="cliInstallBusy"
                  @click="onCliAutoInstall"
                >{{ $t('bkp.autoInstall') }}</el-button>
                <el-button size="small" :loading="cliInstallBusy" @click="onCliManualRetry">{{ $t('bkp.manualRetry') }}</el-button>
                <el-button size="small" @click="onCliUseBuiltin">{{ $t('bkp.useBuiltin') }}</el-button>
              </div>
            </div>
          </template>
        </template>

        <!-- 第 4 步：确认与开始 -->
        <template v-else-if="currentStep === 'confirm'">
          <div class="step-subtitle">{{ mode === 'backup' ? $t('bkp.backupOptions') : $t('bkp.restoreOptions') }}</div>
          <div class="summary-card">
            <div class="summary-row">
              <span class="summary-label">{{ $t('bkp.summaryAction') }}</span>
              <span class="summary-value">{{ mode === 'backup' ? $t('bkp.printBackupDb') : $t('bkp.printRestoreDb') }}</span>
            </div>
            <div class="summary-row" v-if="mode === 'restore'">
              <span class="summary-label">{{ $t('bkp.backupFile') }}</span>
              <!-- file 是完整路径字符串（无 .name 属性），直接显示 -->
              <span class="summary-value" :title="file">{{ file || $t('bkp.noFileSelected') }}</span>
            </div>
            <div class="summary-row" v-else>
              <span class="summary-label">{{ $t('bkp.summaryIncludes') }}</span>
              <span class="summary-value">
                <el-tag size="small" effect="plain" v-if="opts.includeDrop">DROP</el-tag>
                <el-tag size="small" effect="plain" v-if="opts.includeData">{{ $t('bkp.tagData') }}</el-tag>
                <el-tag size="small" effect="plain" v-if="opts.includeViews">{{ $t('bkp.optViews') }}</el-tag>
                <el-tag size="small" effect="plain" v-if="opts.includeProcedures">{{ $t('bkp.tagProcedures') }}</el-tag>
                <el-tag size="small" effect="plain" v-if="opts.includeTriggers">{{ $t('bkp.optTriggers') }}</el-tag>
                <el-tag size="small" effect="plain" v-if="opts.includeEvents">{{ $t('bkp.optEvents') }}</el-tag>
              </span>
            </div>
            <div class="summary-row">
              <span class="summary-label">{{ $t('bkp.summaryEngine') }}</span>
              <span class="summary-value"><el-tag size="small" class="engine-tag">{{ engineLabel }}</el-tag></span>
            </div>
            <div class="summary-row" v-if="!cliToolRequired && guide">
              <span class="summary-label">{{ $t('bkp.summaryNote') }}</span>
              <span class="summary-value text-muted">{{ mode === 'backup' ? $t('bkp.builtinNoteBackup') : $t('bkp.builtinNoteRestore') }}</span>
            </div>
          </div>

          <!-- 连接信息：命令行与内置引擎都展示，便于核对目标 -->
          <template v-if="guide && guide.connectionType !== 'SQLITE'">
            <div class="step-subtitle">{{ $t('bkp.connInfo') }}</div>
            <div class="step-card">
              <div class="conn-info-row">
                <span class="conn-info-label">{{ $t('bkp.server') }}</span>
                <span class="conn-info-value">{{ guide.host }}:{{ guide.port || defaultPort }}</span>
              </div>
              <div class="conn-info-row">
                <span class="conn-info-label">{{ $t('bkp.account') }}</span>
                <span class="conn-info-value">{{ guide.username }}</span>
              </div>
              <div class="conn-info-row">
                <span class="conn-info-label">{{ $t('bkp.targetDbShort') }}</span>
                <span class="conn-info-value">{{ targetDatabase || props.database }}</span>
              </div>
            </div>
            <!-- 备份不再让用户选择保存目录：由系统自动决定落盘位置，完成后在日志中明确告知（远程库可能落在数据库服务器） -->
            <div v-if="mode === 'backup'" class="hint-tip" style="margin-top: 8px;">
              {{ $t('bkp.backupLocationHint') }}
            </div>
            <!-- 仅命令行工具 / 自动模式实际走命令行时展示命令预览 -->
            <template v-if="engine === 'cli' || cliToolRequired">
              <div class="step-subtitle">{{ $t('bkp.cmdPreview') }}</div>
              <div v-if="mode === 'backup' && guide.dumpHint" class="cmd-preview">
                <pre>{{ previewDump }}</pre>
              </div>
              <div v-else-if="mode === 'restore' && guide.restoreHint" class="cmd-preview">
                <pre>{{ previewRestore }}</pre>
              </div>
              <div v-else class="hint-tip" style="margin-top: 10px;">
                {{ $t('bkp.noCmdRef') }}
              </div>
            </template>
          </template>
        </template>
      </div>
    </template>

    <!-- 运行 / 结果区 -->
    <div v-else class="progress-area">
      <div class="status-line">
        <span class="status-label">{{ statusLabel }}</span>
      </div>
      <el-progress :percentage="progress" :status="progressStatus" :stroke-width="14" :text-inside="true" />
      <div v-if="logs.length" ref="logBoxRef" class="log-box">
        <div v-for="(l, i) in logs" :key="i" class="log-line">{{ l }}</div>
      </div>
      <div v-if="error" class="error-box">{{ error }}</div>
    </div>

    <template #footer>
      <template v-if="!running && !finished">
        <el-button size="small" @click="visible = false">{{ $t('common.cancel') }}</el-button>
        <el-button v-if="currentStep !== 'target'" size="small" @click="goPrev">{{ $t('bkp.prev') }}</el-button>
        <el-button v-if="currentStep !== 'confirm'" size="small" type="primary" :disabled="!canNext" @click="goNext">{{ $t('bkp.next') }}</el-button>
        <el-button
          v-else
          size="small"
          type="primary"
          :icon="mode === 'backup' ? Download : FolderOpened"
          :disabled="!canStart"
          @click="onStart"
        >{{ mode === 'backup' ? $t('bkp.startBackup') : $t('bkp.startRestore') }}</el-button>
      </template>
      <el-button v-else-if="running && !finished" size="small" type="danger" :icon="CircleClose" @click="onCancel" :loading="cancelling">{{ $t('bkp.cancelTask') }}</el-button>
      <el-button v-else size="small" type="primary" @click="visible = false">{{ $t('common.close') }}</el-button>
    </template>

    <!-- 目录选择弹窗（点「浏览」打开） -->
    <el-dialog v-model="dirBrowser.visible" :title="$t('bkp.chooseBackupFile')" width="520px" append-to-body>
      <div class="dir-browser">
        <div class="dir-current">
          <el-icon><FolderOpened /></el-icon>
          <span :title="dirBrowser.path">{{ dirBrowser.path || $t('bkp.thisPc') }}</span>
        </div>
        <div class="dir-list">
          <!-- 上级入口：盘符根（如 D:\）本身没有 parent，此时点上级回到磁盘列表，避免进到盘里就退不出去 -->
          <div v-if="dirBrowser.path" class="dir-item up" @click="browseInto(dirBrowser.parent || '')">
            <el-icon><ArrowUp /></el-icon><span>{{ $t('bkp.goUp') }}</span>
          </div>
          <div v-for="d in dirBrowser.dirs" :key="d" class="dir-item" @click="browseInto(joinDir(dirBrowser.path, d))">
            <el-icon><Folder /></el-icon><span>{{ d }}</span>
          </div>
          <template v-if="dirBrowser.fileMode">
            <div v-for="f in dirBrowser.files" :key="'f_' + f" class="dir-item" @click="pickFile(f)">
              <el-icon><Document /></el-icon><span>{{ f }}</span>
            </div>
          </template>
          <div v-if="!dirBrowser.dirs.length && !(dirBrowser.fileMode && dirBrowser.files.length)" class="dir-empty">{{ $t('bkp.emptyDir') }}</div>
        </div>
      </div>
      <template #footer>
        <el-button size="small" @click="dirBrowser.visible = false">{{ $t('common.cancel') }}</el-button>
      </template>
    </el-dialog>
  </el-dialog>

  <!-- 缺少命令行工具时：让用户三选——自动安装 / 我已手动装好重试 / 跳过(内置引擎)，亦可取消任务 -->
  <el-dialog
    v-if="installPrompt"
    :model-value="true"
    :title="$t('bkp.needCli')"
    width="600px"
    append-to-body
    :close-on-click-modal="false"
    :close-on-press-escape="false"
  >
    <div class="install-intro">
      {{ mode === 'backup'
        ? $t('bkp.needCliBackup', { engine: installPrompt.engineDisplay, tool: installPrompt.tool })
        : $t('bkp.needCliRestore', { engine: installPrompt.engineDisplay, tool: installPrompt.tool }) }}
    </div>
    <pre class="install-msg">{{ installPrompt.message }}</pre>
    <template #footer>
      <el-button size="small" :disabled="installBusy" @click="onInstallAbort">{{ $t('bkp.cancelTask') }}</el-button>
      <el-button size="small" :disabled="installBusy" @click="onInstallSkip">{{ $t('bkp.skipUseBuiltin') }}</el-button>
      <el-button
        v-if="installPrompt.canAutoInstall"
        size="small"
        :loading="installBusy"
        @click="onInstallConfirm"
      >{{ $t('bkp.autoInstall') }}</el-button>
      <el-button
        size="small"
        :type="installPrompt.canAutoInstall ? 'default' : 'primary'"
        :loading="installBusy"
        @click="onInstallManual"
      >{{ $t('bkp.manualRetryInstall') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch, nextTick, onBeforeUnmount } from 'vue'
import { ElMessage } from 'element-plus'
import { t, locale as uiLocale } from '../../utils/i18n'
import { Download, FolderOpened, Folder, ArrowUp, Document, CircleClose, CircleCheck, Loading } from '@element-plus/icons-vue'
import {
  listDatabases,
  startBackup, backupTaskStatus, cancelBackup,
  browseBackupDirs,
  backupInstallConfirm, backupInstallManual, backupInstallSkip,
  restoreTaskStatus, cancelRestore, startRestoreLocal,
  restoreInstallConfirm, restoreInstallManual, restoreInstallSkip,
  getBackupCliGuide,
  getBackupCliToolStatus, installBackupCliTool
} from '../../api'

const props = defineProps({
  modelValue: Boolean,
  mode: { type: String, default: 'backup' }, // 'backup' | 'restore'
  connectionId: String,
  database: { type: String, default: '' }
})
const emit = defineEmits(['update:modelValue', 'done'])

const visible = computed({
  get: () => props.modelValue,
  set: (v) => emit('update:modelValue', v)
})

const currentStep = ref('target')
const needsInstallStep = computed(() => {
  if (engine.value === 'builtin') return false
  if (!cliToolRequired.value) return false
  if (cliStatusLoading.value || cliStatusError.value) return false
  return cliStatus.value && !cliStatus.value.present
})
const stepIds = computed(() => {
  const ids = ['target', 'engine']
  if (needsInstallStep.value) ids.push('install')
  ids.push('confirm')
  return ids
})
const steps = computed(() => stepIds.value.map(id => {
  if (id === 'target') return props.mode === 'backup' ? t('bkp.backupContent') : t('bkp.restoreFile')
  if (id === 'engine') return t('bkp.stepEngine')
  if (id === 'install') return t('bkp.stepInstall')
  return props.mode === 'backup' ? t('bkp.stepRunBackup') : t('bkp.stepRunRestore')
}))
const activeIndex = computed(() => stepIds.value.indexOf(currentStep.value))
const lastStep = computed(() => stepIds.value.length - 1)

const title = computed(() => {
  return props.mode === 'backup' ? t('bkp.printBackupDb') : t('bkp.printRestoreDb')
})

// 目标数据库始终允许在下拉中确认/修改（默认选中树节点传入的库），防止传参错位后无法纠正
const showDbSelect = computed(() => true)

// 目录/文件选择弹窗：还原时逐级浏览选择备份文件
const dirBrowser = ref({ visible: false, path: '', parent: null, dirs: [], files: [], fileMode: false })

// 输入框里已有路径的「所在目录」：还原模式给的是文件路径，需要退到父目录
const parentDirOf = (fill) => {
  const s = String(fill || '').trim().replace(/[\\/]+$/, '')
  if (!s || /^[A-Za-z]:$/.test(s) || s === '/') return s
  const i = Math.max(s.lastIndexOf('\\'), s.lastIndexOf('/'))
  return i <= 0 ? (i === 0 ? s.slice(0, 1) : '') : s.slice(0, i)
}

const openDirBrowser = async () => {
  dirBrowser.value.fileMode = true
  dirBrowser.value.visible = true
  // 默认定位：上次浏览过的目录 → 当前备份文件所在目录 → 后端给的用户主目录。
  // 都不可用时才退回磁盘根，避免不管什么系统一打开就停在 C:\ 或 /。
  const start = dirBrowser.value.path || parentDirOf(file.value || '')
  if (start && await browseInto(start)) return
  try {
    const r = await browseBackupDirs('')
    if (!await browseInto(r?.home || '')) await browseInto(r?.path || (r?.dirs || [])[0] || '')
  } catch (e) {
    await browseInto('')
  }
}

// 返回是否成功定位到真实存在的目录（用于默认位置回退判断）
const browseInto = async (path) => {
  try {
    const r = await browseBackupDirs(path || '')
    dirBrowser.value.path = r.path || ''
    dirBrowser.value.parent = r.parent
    dirBrowser.value.dirs = r.dirs || []
    dirBrowser.value.files = r.files || []
    return r.exists !== false
  } catch (e) {
    ElMessage.error(t('bkp.browseFailed', { detail: (e.message || e) }))
    return false
  }
}

/** 还原模式：点击文件即选中（完整路径） */
const pickFile = (name) => {
  file.value = joinDir(dirBrowser.value.path, name)
  dirBrowser.value.visible = false
}

// 拼接子目录路径（Windows 盘符路径用 \，其余用 /）
const joinDir = (base, name) => {
  if (!base) return name
  const sep = /^[A-Za-z]:/.test(base) || base.includes('\\') ? '\\' : '/'
  return base.replace(/[\\/]+$/, '') + sep + name
}

const dbLoading = ref(false)
const databases = ref([])
const targetDatabase = ref(props.database || '')
// 双保险：右击其它库再次打开时（组件若被复用）跟随最新传入的库
watch(() => props.database, v => { targetDatabase.value = v || '' })

const defaultOpts = () => ({
  includeDrop: true,
  includeData: true,
  includeViews: true,
  includeProcedures: true,
  includeTriggers: true,
  includeEvents: true,
  batchSize: 5000
})
const opts = ref(defaultOpts())

// 连接类型相关能力（按数据库类型提示命令行）
const guide = ref(null)
const guideLoading = ref(false)
const guideError = ref('')

// 支持“事件”对象的数据库类型（MySQL/MariaDB/Doris）；其余类型隐藏或禁用
const hasEvents = computed(() => {
  const t = guide.value && guide.value.connectionType
  return t === 'MYSQL' || t === 'MARIADB' || t === 'DORIS'
})

const file = ref('')

// 引擎：auto（默认，优先命令行并自动回退）/ builtin（强制内置）/ cli（强制命令行）
const engine = ref('auto')

const running = ref(false)
const finished = ref(false)
const cancelling = ref(false)
const taskId = ref('')
const status = ref('')
const progress = ref(0)
const message = ref('')
const error = ref('')
const logs = ref([])
const logBoxRef = ref(null)

// 追加一条实时日志（与上一条相同则跳过），并自动滚动到底部
const pushLog = (text) => {
  if (!text) return
  const t = new Date().toLocaleTimeString(uiLocale.value, { hour12: false })
  const last = logs.value[logs.value.length - 1]
  if (last && last.endsWith('] ' + text)) return
  logs.value.push(`[${t}] ${text}`)
  if (logs.value.length > 300) logs.value.splice(0, logs.value.length - 300)
  nextTick(() => {
    const el = logBoxRef.value
    if (el) el.scrollTop = el.scrollHeight
  })
}
const installPrompt = ref(null)
const installBusy = ref(false)
const installDismissed = ref(false)

// 向导第 2 步“命令行工具”预检状态（在开始任务前完成安装决策，避免运行中弹窗）
const cliStatus = ref(null)
const cliStatusLoading = ref(false)
const cliStatusError = ref('')
const cliInstallBusy = ref(false)

let cliCheckSeq = 0

let pollTimer = null

const defaultPort = computed(() => {
  const t = guide.value && guide.value.connectionType
  if (t === 'MYSQL' || t === 'MARIADB') return 3306
  if (t === 'POSTGRESQL') return 5432
  if (t === 'SQLSERVER') return 1433
  if (t === 'ORACLE') return 1521
  if (t === 'DB2') return 50000
  if (t === 'CLICKHOUSE') return 8123
  return ''
})

// CLI 可用性：类型支持 + 非 SSH（隧道连接本机命令行无法复用）+（备份需满足整库条件）
const cliAvailable = computed(() => {
  if (!guide.value || !guide.value.cliSupported) return false
  if (guide.value.isSsh) return false
  if (props.mode === 'restore') return true
  // SQL Server 的 BACKUP DATABASE、达梦的 dexp 是二进制备份，不依赖 DROP/视图选项
  if (guide.value.connectionType === 'SQLSERVER' || guide.value.connectionType === 'DM') return true
  if (guide.value.connectionType === 'SQLITE') return opts.value.includeData && opts.value.includeDrop && opts.value.includeViews
  return opts.value.includeDrop && opts.value.includeViews
})
const cliDisableReason = computed(() => {
  if (!guide.value) return t('bkp.fetchingGuide')
  if (guide.value.isSsh) {
    return t('bkp.sshNoCli')
  }
  if (!guide.value.cliSupported) {
    return t('bkp.noCliEngine', { type: guide.value.connectionLabel })
  }
  return props.mode === 'backup'
    ? t('bkp.needDropViews')
    : ''
})

const cliDesc = computed(() => {
  if (!cliAvailable.value) {
    if (guide.value && guide.value.isSsh) return t('bkp.sshNoCliShort')
    return t('bkp.cliNotSupported')
  }
  if (guide.value && guide.value.connectionType === 'SQLITE') {
    return t('bkp.cliSqlite')
  }
  const tool = guide.value ? guide.value.toolDisplay : t('bkp.cliClientTool')
  return t('bkp.cliInvoke', { tool })
})

const cliValid = computed(() => {
  if (engine.value !== 'cli') return true
  return !!cliAvailable.value
})

// 当前选择在运行期是否真的会调用命令行工具：
// 备份 / 还原选 auto 时优先命令行（前提是类型支持且本机有客户端），
// 强制命令行同理，只有明确选内置引擎才不走命令行。
const cliToolRequired = computed(() => {
  if (!guide.value || !guide.value.cliSupported) return false
  if (engine.value === 'builtin') return false
  if (engine.value === 'cli') return !!cliAvailable.value
  if (props.mode === 'backup') return !!cliAvailable.value
  // auto 还原：只要已选文件且本机有客户端，也优先命令行
  return !!file.value && !!cliAvailable.value
})

// 命令行工具缺失且预检有结论时，阻止进入下一步，避免任务运行中再弹窗处理
const cliBlocked = computed(() => {
  if (!cliToolRequired.value) return false
  if (cliStatusLoading.value) return false
  if (cliStatusError.value) return false
  return !(cliStatus.value && cliStatus.value.present)
})

const engineLabel = computed(() => {
  if (engine.value === 'cli') {
    return guide.value ? t('bkp.engineCliWithTool', { tool: guide.value.toolDisplay }) : t('bkp.engineCli')
  }
  if (engine.value === 'builtin') return t('bkp.engineBuiltin')
  return t('bkp.engineAuto')
})

// ⚠️ 第二个参数是**内部协议值**（fill 里用 kind === '还原' 判断要不要填文件路径），
// 只用于比对、不上屏 —— 所以它**不翻译**：翻了判断条件就永远不成立，预览里的文件路径会消失。
const previewDump = computed(() => fill(guide.value && guide.value.dumpHint, t('qa.exportBtn')))
const previewRestore = computed(() => fill(guide.value && guide.value.restoreHint, t('bk.restore')))

function fill(tpl, kind) {
  if (!tpl) return ''
  const db = targetDatabase.value || props.database || '{database}'
  const host = guide.value?.host || t('bkp.phServer')
  const user = guide.value?.username || t('bkp.phUser')
  const port = guide.value?.port || defaultPort.value || t('bkp.phPort')
  const filePath = kind === '还原'
    ? (file.value || t('bkp.phBackupFile'))
    : t('bkp.phBackupFile')
  const sqliteFile = guide.value?.sqliteFile || t('bkp.phSqliteFile')
  return t('bkp.cmdComment') + '\n' + tpl
    .replace(/\{sqliteFile\}/g, sqliteFile)
    .replace(/\{host\}/g, host)
    .replace(/\{port\}/g, String(port))
    .replace(/\{user\}/g, user)
    .replace(/\{database\}/g, db)
    .replace(/\{filePath\}/g, filePath)
    .replace(/\{charset\}/g, 'utf8mb4')
}

const canNext = computed(() => {
  if (currentStep.value === 'target') {
    if (showDbSelect.value && !targetDatabase.value) return false
    if (props.mode === 'restore' && !file.value) return false
    return true
  }
  if (currentStep.value === 'engine') {
    // 等待命令行工具检测完成，再由 goNext 决定进入安装步骤还是确认步骤
    if (cliToolRequired.value && cliStatusLoading.value) return false
    return true
  }
  if (currentStep.value === 'install') {
    return !!cliStatus.value?.present || engine.value === 'builtin'
  }
  return true
})

const canStart = computed(() => {
  if (currentStep.value !== 'confirm') return false
  if (showDbSelect.value && !targetDatabase.value) return false
  if (props.mode === 'restore' && !file.value) return false
  if (!cliValid.value) return false
  return true
})

const goNext = () => {
  const ids = stepIds.value
  const idx = ids.indexOf(currentStep.value)
  if (idx === -1 || idx >= ids.length - 1) return
  if (currentStep.value === 'engine' && needsInstallStep.value) {
    currentStep.value = 'install'
    return
  }
  currentStep.value = ids[idx + 1]
}

const goPrev = () => {
  const ids = stepIds.value
  const idx = ids.indexOf(currentStep.value)
  if (idx > 0) currentStep.value = ids[idx - 1]
}

const startDisabled = computed(() => !canStart)

const statusLabel = computed(() => {
  switch (status.value) {
    case 'running': return t('bkp.stRunning')
    case 'success': return t('bkp.stSuccess')
    case 'error': return t('bkp.stError')
    case 'canceled': return t('bkp.stCanceled')
    default: return t('bkp.stWaiting')
  }
})
const progressStatus = computed(() => {
  if (status.value === 'error') return 'exception'
  if (status.value === 'success') return 'success'
  return ''
})

const onOpen = async () => {
  guide.value = null
  guideLoading.value = true
  guideError.value = ''
  try {
    guide.value = await getBackupCliGuide(props.connectionId)
  } catch (e) {
    console.warn('获取命令行能力失败', e)
    guideError.value = t('bkp.guideFetchFailed', { detail: (e?.message || e) })
  } finally {
    guideLoading.value = false
  }
  if (showDbSelect.value) {
    dbLoading.value = true
    try {
      databases.value = (await listDatabases(props.connectionId)) || []
      if (databases.value.length && !targetDatabase.value) {
        targetDatabase.value = databases.value[0]
      }
    } catch (e) {
      ElMessage.error(t('bkp.dbListFailed', { detail: (e.message || e) }))
    } finally {
      dbLoading.value = false
    }
  }
}

const onClose = () => {
  stopPoll()
  if (!running.value && !finished.value) reset()
}

/** 右上角 X 常驻显示，但备份/还原进行中不直接关：关掉任务照跑，会被误当成已停止 */
const onBeforeClose = (done) => {
  if (running.value) {
    ElMessage.warning(t('bkp.taskBusy'))
    return
  }
  done()
}

const reset = () => {
  currentStep.value = 'target'
  engine.value = 'auto'
  opts.value = defaultOpts()
  guide.value = null
  guideLoading.value = false
  guideError.value = ''
  running.value = false
  finished.value = false
  cancelling.value = false
  taskId.value = ''
  status.value = ''
  progress.value = 0
  message.value = ''
  error.value = ''
  logs.value = []
  file.value = ''
  installPrompt.value = null
  installBusy.value = false
  installDismissed.value = false
  cliStatus.value = null
  cliStatusLoading.value = false
  cliStatusError.value = ''
  cliInstallBusy.value = false
  cliCheckSeq = 0
}

const onStart = async () => {
  const db = targetDatabase.value || props.database
  if (!db) {
    ElMessage.warning(t('bkp.pickTargetDb'))
    return
  }
  if (props.mode === 'restore' && !file.value) {
    ElMessage.warning(t('bkp.pickSqlFile'))
    return
  }
  running.value = true
  finished.value = false
  progress.value = 0
  message.value = t('bkp.taskStarted')
  error.value = ''
  logs.value = []
  pushLog(t('bkp.taskStarted'))

  try {
    let res
    if (props.mode === 'backup') {
      res = await startBackup({
        connectionId: props.connectionId,
        database: db,
        options: opts.value,
        engine: engine.value === 'auto' ? undefined : engine.value
      })
    } else {
      // 直接使用本机备份文件还原（不上传）
      res = await startRestoreLocal({
        connectionId: props.connectionId,
        database: db,
        filePath: file.value,
        engine: engine.value === 'auto' ? undefined : engine.value
      })
    }
    taskId.value = res.taskId
    status.value = res.status || 'running'
    poll()
  } catch (e) {
    running.value = false
    finished.value = true
    status.value = 'error'
    error.value = e.message || e
    ElMessage.error(props.mode === 'backup' ? t('bkp.startBackupFailed') : t('bkp.startRestoreFailed'))
  }
}

const poll = () => {
  stopPoll()
  pollTimer = setInterval(async () => {
    try {
      const res = props.mode === 'backup'
        ? await backupTaskStatus(taskId.value)
        : await restoreTaskStatus(taskId.value)
      status.value = res.status || 'running'
      progress.value = typeof res.progress === 'number' ? res.progress : 0
      message.value = res.message || ''
      pushLog(res.message)
      if (res.error) { error.value = res.error; pushLog(t('bkp.errorPrefix', { detail: res.error })) }

      if (!res.install) installDismissed.value = false
      const terminal = ['success', 'error', 'canceled'].includes(status.value)
      if (terminal) {
        pushLog(status.value === 'success'
          ? (props.mode === 'backup' ? t('bkp.backupDone') : t('bkp.restoreDone'))
          : (status.value === 'canceled' ? t('bkp.taskCanceled') : t('bkp.taskFailed')))
        // 备份成功：如实展示文件所在机器（本机 / 数据库服务器）与位置、文件名
        if (status.value === 'success' && props.mode === 'backup') {
          pushLog(res.resultDetail || t('bkp.fileGenerated', { file: (res.fileName || '') }))
        }
        installPrompt.value = null
        stopPoll()
        running.value = false
        finished.value = true
        emit('done', { mode: props.mode, status: status.value, taskId: taskId.value })
        if (status.value === 'success') {
          ElMessage.success(props.mode === 'backup' ? t('bkp.backupDone') : t('bkp.restoreDone'))
        }
      } else if (res.install && !installDismissed.value && !installPrompt.value) {
        // 任务等待用户确认是否自动安装命令行工具
        installPrompt.value = res.install
      }
    } catch (e) {
      // 轮询失败不中断
      console.warn('poll failed', e)
    }
  }, 800)
}

const stopPoll = () => {
  if (pollTimer) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

// 本组件由 v-if 挂载，任务进行中被关掉时会被真正卸载，这里兜底停轮询
onBeforeUnmount(stopPoll)

const onCancel = async () => {
  cancelling.value = true
  try {
    if (props.mode === 'backup') await cancelBackup(taskId.value)
    else await cancelRestore(taskId.value)
  } catch (e) {
    console.warn('cancel failed', e)
  } finally {
    cancelling.value = false
  }
}

// 自动安装 → 服务端安装完成后自动继续命令行备份/还原
const onInstallConfirm = async () => {
  installBusy.value = true
  try {
    if (props.mode === 'backup') await backupInstallConfirm(taskId.value)
    else await restoreInstallConfirm(taskId.value)
    installDismissed.value = true
    installPrompt.value = null
  } catch (e) {
    ElMessage.error(t('bkp.installConfirmFailed', { detail: (e.message || e) }))
  } finally {
    installBusy.value = false
  }
}

// 我已手动装好 → 服务端重新检测后继续命令行备份/还原
const onInstallManual = async () => {
  installBusy.value = true
  try {
    if (props.mode === 'backup') await backupInstallManual(taskId.value)
    else await restoreInstallManual(taskId.value)
    installDismissed.value = true
    installPrompt.value = null
  } catch (e) {
    ElMessage.error(t('bkp.recheckFailed', { detail: (e.message || e) }))
  } finally {
    installBusy.value = false
  }
}

// 跳过安装 → 回退内置 JDBC 引擎继续执行
const onInstallSkip = async () => {
  installBusy.value = true
  try {
    if (props.mode === 'backup') await backupInstallSkip(taskId.value)
    else await restoreInstallSkip(taskId.value)
    installDismissed.value = true
    installPrompt.value = null
  } catch (e) {
    ElMessage.error(t('bkp.skipFailed', { detail: (e.message || e) }))
  } finally {
    installBusy.value = false
  }
}

// 取消整个备份/还原任务
const onInstallAbort = () => {
  installDismissed.value = true
  installPrompt.value = null
  onCancel()
}

// ---------- 向导第 2 步：命令行工具预检 / 安装（任务开始前完成，避免运行中弹窗） ----------

const loadCliStatus = async () => {
  const seq = ++cliCheckSeq
  if (!cliToolRequired.value) {
    cliStatus.value = null
    cliStatusError.value = ''
    cliStatusLoading.value = false
    return
  }
  cliStatusLoading.value = true
  cliStatusError.value = ''
  try {
    const s = await getBackupCliToolStatus(props.connectionId, props.mode)
    if (seq !== cliCheckSeq) return
    cliStatus.value = s || null
  } catch (e) {
    if (seq !== cliCheckSeq) return
    cliStatus.value = null
    cliStatusError.value = (e && e.message) || String(e)
  } finally {
    if (seq === cliCheckSeq) cliStatusLoading.value = false
  }
}

const onCliAutoInstall = async () => {
  const st = cliStatus.value
  if (!st || !st.canAutoInstall) return
  cliInstallBusy.value = true
  try {
    const res = await installBackupCliTool(props.connectionId, props.mode)
    if (res && res.present) {
      ElMessage.success(t('bkp.installedOk', { tool: (res.tool || st.tool) }))
    } else if (res && res.success) {
      ElMessage.warning(t('bkp.installedNotDetected'))
    } else {
      ElMessage.error(t('bkp.autoInstallFailed', { detail: ((res && res.detail) || t('bkp.unknownReason')) }))
    }
    await loadCliStatus()
  } catch (e) {
    ElMessage.error(t('bkp.autoInstallFailed', { detail: ((e && e.message) || e) }))
  } finally {
    cliInstallBusy.value = false
  }
}

const onCliManualRetry = async () => {
  cliInstallBusy.value = true
  try {
    await loadCliStatus()
    if (cliStatus.value && cliStatus.value.present) ElMessage.success(t('bkp.cliDetected'))
  } finally {
    cliInstallBusy.value = false
  }
}

const onCliUseBuiltin = () => {
  engine.value = 'builtin'
  if (currentStep.value === 'install') currentStep.value = 'confirm'
  ElMessage.info(t('bkp.willUseBuiltin'))
}

watch(() => props.modelValue, (v) => {
  if (!v) reset()
})

// 引擎/连接类型/备份选项/还原文件变化时，刷新“命令行工具”预检
watch(
  () => [engine.value, guide.value, cliAvailable.value, file.value],
  () => loadCliStatus(),
  { deep: false }
)

// 安装步骤中，检测到工具已安装后自动跳到确认步骤
watch(
  () => cliStatus.value?.present,
  (present) => {
    if (present && currentStep.value === 'install') {
      currentStep.value = 'confirm'
    }
  }
)
</script>

<style scoped>
.wizard-steps {
  margin-bottom: 18px;
}
.wizard-body {
  min-height: 160px;
}
.step-subtitle {
  font-size: 14px;
  font-weight: 600;
  color: var(--el-text-color-primary);
  margin-top: 18px;
  margin-bottom: 10px;
  padding-left: 10px;
  border-left: 3px solid var(--el-color-info);
}
.step-card {
  background: var(--dc-bg-soft);
  border-radius: 8px;
  padding: 14px 16px;
}
.step-card .el-form-item:last-child {
  margin-bottom: 0;
}
.target-line {
  display: flex;
  align-items: center;
  min-height: 34px;
}
.target-line + .target-line {
  margin-top: 4px;
}
.target-label {
  width: 110px;
  padding-right: 12px;
  flex-shrink: 0;
  text-align: right;
  color: var(--el-text-color-secondary);
  font-size: 14px;
}
.target-value {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}
.text-danger {
  color: var(--el-color-danger);
}
.summary-card {
  background: var(--dc-bg-soft);
  border-radius: 8px;
  padding: 8px 16px;
}
.summary-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 0;
  border-bottom: 1px solid var(--el-border-color-lighter);
  font-size: 14px;
}
.summary-row:last-child {
  border-bottom: none;
}
.summary-label {
  color: var(--el-text-color-secondary);
  min-width: 80px;
}
.summary-value {
  color: var(--el-text-color-primary);
  text-align: right;
  flex: 1;
  margin-left: 12px;
  word-break: break-all;
}
.summary-value .el-tag {
  margin-left: 4px;
}
.summary-value .el-tag:first-child {
  margin-left: 0;
}
.summary-value .engine-tag {
  background: var(--dc-primary-soft);
  color: var(--dc-primary);
  border-color: var(--dc-primary);
}
.conn-info-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 0;
  border-bottom: 1px solid var(--el-border-color-lighter);
  font-size: 14px;
}
.conn-info-row:last-child {
  border-bottom: none;
}
.conn-info-label {
  color: var(--el-text-color-secondary);
  min-width: 80px;
}
.conn-info-value {
  color: var(--el-text-color-primary);
  text-align: right;
  flex: 1;
  margin-left: 12px;
  word-break: break-all;
}
.form-area :deep(.el-form-item__content) {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
.fixed-db {
  font-weight: 600;
  color: var(--el-text-color-primary);
}
.muted {
  color: var(--el-text-color-secondary);
}
.hint-tip {
  margin-top: 10px;
  padding: 8px 10px;
  background: var(--dc-bg-soft);
  color: var(--el-text-color-regular);
  border-radius: 4px;
  font-size: 13px;
  line-height: 1.6;
}
.engine-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
  align-items: stretch;
  width: 100%;
}
.engine-group .el-radio {
  height: auto;
  margin-right: 0;
  padding: 8px 10px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  white-space: normal;
}
.engine-group .el-radio.is-checked {
  border-color: var(--el-color-primary);
  background: var(--dc-primary-soft);
}
.engine-group :deep(.el-radio__label) {
  flex: 1;
  padding-left: 8px;
}
.engine-group :deep(.el-radio__input) {
  margin-top: 2px;
}
.engine-name {
  font-weight: 600;
  font-size: 14px;
}
.engine-desc {
  font-size: 13px;
  color: var(--el-text-color-secondary);
  margin-top: 2px;
}
.text-muted {
  color: var(--el-text-color-secondary);
}
.sqlite-note {
  padding: 8px 10px;
  background: var(--dc-bg-soft);
  border-radius: 4px;
  font-size: 13px;
  color: var(--el-text-color-regular);
}
.cli-note {
  margin-top: 8px;
  padding: 8px 10px;
  background: var(--dc-warning-wash);
  color: var(--dc-warning);
  border-radius: 4px;
  font-size: 13px;
  line-height: 1.6;
  white-space: pre-wrap;
}
.cli-ssh-tip {
  margin-bottom: 10px;
  padding: 8px 10px;
  background: var(--dc-warning-wash);
  border: 1px solid var(--dc-warning);
  border-radius: 4px;
  font-size: 13px;
  line-height: 1.6;
  color: var(--dc-warning);
}
.cmd-preview {
  margin-top: 10px;
}
.cmd-preview pre {
  margin: 0;
  padding: 8px 10px;
  background: var(--dc-bg-code);
  color: var(--dc-code-text);
  border-radius: 6px;
  font-size: 13px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 160px;
  overflow: auto;
}
.progress-area {
  padding: 12px 6px;
}
.opt-note {
  color: var(--el-text-color-secondary);
  font-size: 13px;
}
.dir-hint {
  margin-top: 4px;
  font-size: 13px;
  color: var(--el-text-color-secondary);
}
.dir-hint.is-err {
  color: var(--el-color-danger);
}
/* 保存目录输入框文字与表单 label 字体颜色一致 */
.dir-input {
  --el-input-text-color: var(--el-text-color-regular);
}
.dir-input :deep(.el-input__inner) {
  color: var(--el-text-color-regular);
}
/* 输入框后的「浏览」按钮：背景融入深色主题，避免默认浅色 patch 突兀 */
.dir-input :deep(.el-input-group__append) {
  background: var(--dc-bg-soft) !important;
  border-color: var(--dc-border-soft) !important;
  box-shadow: 0 0 0 1px var(--dc-border-soft) inset !important;
}
.dir-input :deep(.el-input-group__append .el-button) {
  color: var(--dc-text-mid) !important;
  background: transparent !important;
  border-color: transparent !important;
}
.dir-input :deep(.el-input-group__append .el-button:hover) {
  color: var(--dc-primary) !important;
}
.dir-browser {
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  overflow: hidden;
}
.dir-current {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  font-size: 13px;
  font-weight: 600;
  color: var(--dc-text-strong);
  background: var(--dc-bg-soft);
  border-bottom: 1px solid var(--dc-border);
  word-break: break-all;
}
.dir-list {
  max-height: 280px;
  overflow: auto;
  padding: 4px;
}
.dir-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 14px;
  color: var(--el-text-color-regular);
}
.dir-item .el-icon {
  color: var(--el-color-primary);
}
.dir-item:hover {
  background: var(--dc-bg-hover);
}
.dir-item.up {
  color: var(--el-text-color-secondary);
}
.dir-item.up .el-icon {
  color: var(--el-text-color-secondary);
}
.dir-empty {
  padding: 18px 0;
  text-align: center;
  font-size: 13px;
  color: var(--el-text-color-secondary);
}
.saved-tip {
  margin-top: 8px;
  font-size: 13px;
  color: var(--el-text-color-regular);
  word-break: break-all;
}
.log-box {
  margin-top: 10px;
  height: 180px;
  overflow: auto;
  background: #141821;
  color: #9fb3c8;
  border: 1px solid rgba(127, 127, 127, 0.18);
  border-radius: 6px;
  padding: 8px 10px;
  font-family: 'JetBrains Mono', Consolas, 'Courier New', monospace;
  font-size: 13px;
  line-height: 1.7;
}
.log-line {
  white-space: pre-wrap;
  word-break: break-all;
}
.status-line {
  display: flex;
  justify-content: space-between;
  margin-bottom: 10px;
  font-size: 14px;
}
.status-label {
  font-weight: 600;
  color: var(--el-text-color-primary);
}
.error-box {
  margin-top: 12px;
  padding: 8px 10px;
  background: var(--dc-danger-wash);
  color: var(--dc-danger);
  border-radius: 4px;
  font-size: 13px;
  max-height: 120px;
  overflow: auto;
  white-space: pre-wrap;
}
.install-intro {
  font-size: 14px;
  color: var(--el-text-color-primary);
  margin-bottom: 10px;
}
.install-msg {
  margin: 0;
  padding: 10px 12px;
  background: var(--dc-bg-soft);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  font-size: 13px;
  line-height: 1.7;
  color: var(--el-text-color-regular);
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 180px;
  overflow: auto;
}
.tool-missing-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--dc-warning);
  margin-bottom: 8px;
}
.tool-missing .install-msg {
  max-height: 150px;
}
.tool-ok {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 10px 12px;
  background: var(--dc-success-wash);
  color: var(--dc-success);
  border-radius: 6px;
  font-size: 14px;
  line-height: 1.6;
}
.tool-ok .el-icon {
  margin-top: 2px;
  flex-shrink: 0;
}
.tool-actions {
  margin-top: 12px;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.tool-note {
  margin-top: 10px;
  font-size: 13px;
  color: var(--el-text-color-secondary);
  line-height: 1.6;
}
.sql-upload {
  width: 100%;
}
.sql-upload :deep(.el-upload-dragger) {
  padding: 24px 8px;
}
</style>
