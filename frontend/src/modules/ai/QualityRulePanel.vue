<template>
  <div class="qr">
    <!-- 工具栏：选表 + 规则生成/编辑（保存等操作在弹窗底部） -->
    <div class="qr-bar">
      <el-select
        v-model="table"
        size="small"
        class="qr-table-select"
        :placeholder="$t('qa.pickTable')"
        filterable
        clearable
        :loading="loadingTables"
        :disabled="disabled"
        @change="onTableChange"
      >
        <el-option v-for="tb in tables" :key="tb" :label="tb" :value="tb" />
      </el-select>

      <span class="qr-divider"></span>

      <el-button size="small" :disabled="!usable || !table || isNoSql" :loading="generating" @click="generate">
        <el-icon class="qr-btn-ic"><MagicStick /></el-icon>{{ $t('qa.aiSuggest') }}
      </el-button>
      <el-button size="small" :disabled="!usable || !table" @click="addRule">
        <el-icon class="qr-btn-ic"><Plus /></el-icon>{{ $t('qa.addRule') }}
      </el-button>

      <span class="qr-spacer"></span>
      <span v-if="rules.length" class="qr-count">
        {{ $t('qa.ruleCountPre') }}<b>{{ rules.length }}</b>{{ $t('qa.ruleCountPost') }}<template v-if="!lastSaved">{{ $t('qa.unsaved') }}</template>
      </span>
    </div>

    <div class="qr-hint" :class="{ 'qr-hint-fill': !table }">
      <!-- 内层 span：未选表时 .qr-hint-fill 为 flex 布局，包一层可让整段文案保持为一个整体 -->
      <span>
        {{ $t('qa.hintPre') }}
        {{ $t('qa.hintMid') }}<a class="qr-link" @click="emit('goto-analysis')">{{ $t('qa.title') }}</a>{{ $t('qa.hintPost') }}
      </span>
    </div>

    <!-- 规则列表（未选表时不再显示占位提示，保持界面简洁） -->
    <template v-if="table">
      <div v-if="!rules.length" class="qr-empty">
        {{ $t('qa.empty') }}
      </div>
      <!-- 高度自适应撑满剩余空间（不再固定 max-height=260，避免下方大片留白） -->
      <el-table v-else :data="rules" size="small" class="qr-table" height="100%">
        <el-table-column width="54" align="center">
          <template #default="{ row }">
            <el-switch v-model="row.enabled" size="small" />
          </template>
        </el-table-column>
        <el-table-column prop="column" :label="$t('dgen.colField')" width="150" show-overflow-tooltip />
        <el-table-column :label="$t('qa.colRule')" width="112">
          <template #default="{ row }">{{ typeLabel(row.type) }}</template>
        </el-table-column>
        <el-table-column :label="$t('dgen.colParam')" min-width="130" show-overflow-tooltip>
          <template #default="{ row }">{{ paramSummary(row) }}</template>
        </el-table-column>
        <el-table-column :label="$t('qa.colLevel')" width="62" align="center">
          <template #default="{ row }">
            <span class="qr-level" :class="levelClass(row.level)">{{ levelText(row.level) }}</span>
          </template>
        </el-table-column>
        <el-table-column prop="message" :label="$t('qa.colMessage')" min-width="150" show-overflow-tooltip />
        <el-table-column width="96" align="right">
          <template #default="{ row, $index }">
            <el-button size="small" text type="primary" @click="editRule(row, $index)">{{ $t('common.edit') }}</el-button>
            <el-button size="small" text type="danger" @click="removeRule($index)">{{ $t('common.delete') }}</el-button>
          </template>
        </el-table-column>
      </el-table>
    </template>

    <!-- 规则编辑弹窗 -->
    <el-dialog v-model="editVisible" :title="editIndex >= 0 ? $t('qa.editRule') : $t('qa.addRule')" width="540px" append-to-body>
      <el-form label-width="96px" label-position="left" size="small">
        <el-form-item :label="$t('dgen.colField')">
          <el-select v-model="form.column" filterable :placeholder="$t('qa.pickColumn')" style="width:100%">
            <el-option
              v-for="c in columns"
              :key="c.name"
              :label="c.comment ? $t('qa.colOption', { name: c.name, comment: c.comment }) : c.name"
              :value="c.name"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="$t('qa.ruleType')">
          <el-select v-model="form.type" :placeholder="$t('qa.pickRuleType')" style="width:100%" @change="onTypeChange">
            <el-option v-for="ty in types" :key="ty.type" :label="typeLabel(ty.type)" :value="ty.type">
              <span>{{ typeLabel(ty.type) }}</span>
              <span class="qr-opt-hint">{{ typeHint(ty) }}</span>
            </el-option>
          </el-select>
        </el-form-item>
        <el-form-item v-if="currentType" :label="$t('qa.desc')">
          <span class="qr-form-hint">{{ typeHint(currentType) }}</span>
        </el-form-item>
        <el-form-item v-for="p in currentParams" :key="p.key" :label="paramLabel(form.type, p)">
          <el-select v-if="p.kind === 'select'" v-model="form.params[p.key]" style="width:100%">
            <el-option v-for="o in p.options" :key="o" :label="o" :value="o" />
          </el-select>
          <el-input-number
            v-else-if="p.kind === 'number'"
            v-model="form.params[p.key]"
            :controls="false"
            style="width:100%"
          />
          <el-input v-else v-model="form.params[p.key]" :placeholder="p.kind === 'list' ? $t('qa.multiValues') : ''" />
        </el-form-item>
        <el-form-item :label="$t('qa.colLevel')">
          <el-radio-group v-model="form.level">
            <el-radio-button label="高">{{ $t('qa.level.high') }}</el-radio-button>
            <el-radio-button label="中">{{ $t('qa.level.mid') }}</el-radio-button>
            <el-radio-button label="低">{{ $t('qa.level.low') }}</el-radio-button>
          </el-radio-group>
        </el-form-item>
        <el-form-item :label="$t('qa.colMessage')">
          <el-input v-model="form.message" :placeholder="$t('qa.msgPlaceholder')" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="editVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="confirmEdit">{{ $t('common.confirm') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import { ElMessage } from 'element-plus'
import { MagicStick, Plus } from '@element-plus/icons-vue'
import { listColumns, aiQualityRules, aiQualityTypes, aiQualitySaved, aiQualitySave } from '../../api'
import { t, te } from '../../utils/i18n'

const props = defineProps({
  conn: { type: Object, default: null },
  database: { type: String, default: '' },
  tables: { type: Array, default: () => [] },
  loadingTables: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
  isNoSql: { type: Boolean, default: false }
})

/** 操作按钮已移到弹窗底部，这里把状态与能力上报给父组件 */
/** goto-analysis：提示条里点击「质量分析」时，跳到该页签 */
const emit = defineEmits(['state', 'goto-analysis'])

const table = ref('')
const rules = ref([])
const columns = ref([])
const types = ref([])
const lastSaved = ref(false)

const generating = ref(false)
const saving = ref(false)

const editVisible = ref(false)
const editIndex = ref(-1)
const form = ref(emptyForm())

const usable = computed(() => !!props.conn && !!props.database)
const currentType = computed(() => types.value.find(ty => ty.type === form.value.type) || null)
const currentParams = computed(() => currentType.value?.params || [])

function emptyForm () {
  return { column: '', type: '', level: '中', message: '', params: {} }
}

// 规则类型的名称 / 说明 / 参数名，中文原文由后端目录（quality.rs 的 type_catalog）下发。
// 前端按 type 键用字典覆盖：收录过的走译文，没收录的（后端以后新增的类型）原样显示后端
// 文案 —— 所以这里用 te() 判断"有没有收录"，而不是靠 t() 的回落（回落只会给出中文或键名）。
/**
 * 规则类型 → 显示名。注意大小写不敏感：数据里可能是 Constant / MAX_LENGTH 等写法
 */
const typeLabel = (ty) => {
  const key = String(ty || '').trim().toLowerCase()
  if (!key) return ''
  if (te('qa.type.' + key)) return t('qa.type.' + key)
  const hit = types.value.find(x => String(x.type || '').trim().toLowerCase() === key)
  return hit?.label || ty
}

/** 规则类型的说明文字（同上，按 type 键覆盖）。 */
const typeHint = (ty) => {
  const key = String(ty?.type || '').trim().toLowerCase()
  if (key && te('qa.hint.' + key)) return t('qa.hint.' + key)
  return ty?.hint || ''
}

/**
 * 参数名。同名参数在不同规则类型下含义不同：`range` 的 max 是"最大值"，
 * `max_length` 的 max 是"最大长度" —— 所以用「类型.参数」两级键，不能只用参数名。
 */
const paramLabel = (typeCode, p) => {
  const key = String(typeCode || '').trim().toLowerCase() + '.' + String(p?.key || '')
  if (te('qa.param.' + key)) return t('qa.param.' + key)
  return p?.label || p?.key || ''
}

/** 等级：库里存的是「高/中/低」（levelClass 也拿它判断），只在显示时翻译。 */
const levelText = (lv) => {
  if (lv === '高') return t('qa.level.high')
  if (lv === '中') return t('qa.level.mid')
  if (lv === '低') return t('qa.level.low')
  return lv || ''
}

const levelClass = (lv) => (lv === '高' ? 'high' : lv === '中' ? 'mid' : 'low')

const paramSummary = (row) => {
  const p = row.params || {}
  const parts = []
  for (const [, v] of Object.entries(p)) {
    if (v === '' || v === null || v === undefined) continue
    parts.push(Array.isArray(v) ? v.join('/') : String(v))
  }
  return parts.length ? parts.join(' · ') : '—'
}

const loadTypes = async () => {
  if (types.value.length) return
  try {
    const r = await aiQualityTypes()
    types.value = (r && r.types) || []
  } catch (e) { types.value = [] }
}

const loadColumns = async () => {
  columns.value = []
  if (!props.conn || !table.value || !props.database) return
  try {
    const list = await listColumns(props.conn.id, props.database, table.value)
    columns.value = (list || [])
      .map(c => (typeof c === 'string' ? { name: c } : { name: c.name || c.column, comment: c.comment || '' }))
      .filter(c => c.name)
  } catch (e) { columns.value = [] }
}

const loadSaved = async (silent) => {
  if (!table.value) { if (!silent) ElMessage.warning(t('qa.pickTableFirst')); return }
  try {
    const r = await aiQualitySaved({ connectionId: props.conn.id, database: props.database, table: table.value })
    rules.value = (r && r.rules) || []
    lastSaved.value = rules.value.length > 0
    if (!silent) {
      ElMessage.success(rules.value.length ? t('qa.readRules', { n: rules.value.length }) : t('qa.noSavedRules'))
    }
  } catch (e) {
    rules.value = []
    lastSaved.value = false
    if (!silent) ElMessage.error(e?.message || t('qa.readFailed'))
  }
}

const onTableChange = async () => {
  rules.value = []
  lastSaved.value = false
  if (!table.value) return
  await Promise.all([loadColumns(), loadSaved(true)])
}

const generate = async () => {
  if (!table.value) return
  generating.value = true
  try {
    const r = await aiQualityRules({ connectionId: props.conn.id, database: props.database, table: table.value })
    if (r && r.success) {
      rules.value = (r.rules || []).map(x => ({ ...x, enabled: x.enabled !== false, params: x.params || {} }))
      lastSaved.value = false
      ElMessage.success(t('qa.generatedRules', { n: rules.value.length }))
    } else {
      ElMessage.error(r?.message || t('qa.genFailed'))
    }
  } catch (e) {
    ElMessage.error(e?.message || t('qa.genFailed'))
  }
  generating.value = false
}

const save = async () => {
  if (!table.value || !rules.value.length) return
  saving.value = true
  try {
    const r = await aiQualitySave({
      connectionId: props.conn.id,
      database: props.database,
      table: table.value,
      rules: rules.value
    })
    if (r && r.success) {
      lastSaved.value = true
      // 后端历史上有两个名字（`count` / `ruleCount`），都认；再兜一层本地条数，
      // 免得任何一个名字漂了就显示成「已保存 undefined 条规则」
      const saved = r.ruleCount ?? r.count ?? rules.value.length
      ElMessage.success(t('qa.savedRules', { n: saved }))
    } else {
      ElMessage.error(r?.message || t('qa.saveFailed'))
    }
  } catch (e) {
    ElMessage.error(e?.message || t('qa.saveFailed'))
  }
  saving.value = false
}

const addRule = () => {
  editIndex.value = -1
  form.value = emptyForm()
  editVisible.value = true
}

const editRule = (row, index) => {
  editIndex.value = index
  form.value = {
    column: row.column,
    type: row.type,
    level: row.level || '中',
    message: row.message || '',
    params: { ...(row.params || {}) }
  }
  editVisible.value = true
}

const removeRule = (index) => {
  rules.value.splice(index, 1)
  lastSaved.value = false
}

const onTypeChange = () => {
  // ⚠️ 局部变量原来叫 t：本函数要用 t() 取译文，叫 t 会把它遮蔽掉（t('x') 变成"对象当函数调"）
  const cur = currentType.value
  form.value.params = {}
  if (cur?.defaultLevel) form.value.level = cur.defaultLevel
  if (!form.value.message) form.value.message = typeHint(cur)
}

const confirmEdit = () => {
  if (!form.value.column) { ElMessage.warning(t('qa.pickColumnFirst')); return }
  if (!form.value.type) { ElMessage.warning(t('qa.pickTypeFirst')); return }
  const item = {
    column: form.value.column,
    type: form.value.type,
    level: form.value.level,
    message: form.value.message || typeHint(currentType.value),
    enabled: true,
    params: { ...form.value.params }
  }
  if (editIndex.value >= 0) {
    const old = rules.value[editIndex.value]
    item.enabled = old.enabled !== false
    item.id = old.id
    rules.value[editIndex.value] = item
  } else {
    rules.value.push(item)
  }
  lastSaved.value = false
  editVisible.value = false
}

onMounted(loadTypes)

// 连接或库变化 → 清空（规则按 连接+库+表 存储）
watch(() => [props.conn?.id, props.database], () => {
  table.value = ''
  rules.value = []
  columns.value = []
  lastSaved.value = false
})

// 上报状态给父组件（用于渲染弹窗底部的「保存规则」按钮）
watch(
  [() => usable.value, () => table.value, saving, lastSaved, () => rules.value.length, generating],
  () => {
    emit('state', {
      canSave: usable.value && !!table.value && rules.value.length > 0,
      saving: saving.value,
      generating: generating.value,
      hasTable: !!table.value,
      canGenerate: usable.value && !!table.value && !props.isNoSql,
      savedCount: rules.value.length,
      lastSaved: lastSaved.value
    })
  },
  { immediate: true }
)

/** 供父组件（弹窗底部）调用 */
defineExpose({ save, generate, addRule })
</script>

<style scoped>
/* min-height: 0 让面板可被父容器限高，配合表格 flex:1 实现「表格内部滚动」而非整块滚动 */
.qr { display: flex; flex-direction: column; min-height: 0; }
.qr-bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.qr-table-select { width: 220px; }
.qr-divider { width: 1px; height: 16px; background: var(--dc-border); margin: 0 2px; flex-shrink: 0; }
.qr-spacer { flex: 1; }
.qr-btn-ic { margin-right: 3px; vertical-align: -2px; }
.qr-count { font-size: 13px; color: var(--dc-text-dim); }
.qr-count b { color: var(--dc-primary); font-weight: 600; }
.qr-hint {
  /* 字号与「质量分析」等其它页签的提示文本保持一致（12.5px），文案居中。
     注意：这里**不能写 flex-basis: 100%** ——
     该提示条位于 .qr（纵向 flex 容器）中，flex-basis 作用于主轴（高度），
     会被解释成「高度占满 100%」，把提示条撑成一大块空白并挤掉下方规则表格。
     横向占满由父容器默认的 align-items: stretch 保证。 */
  font-size: 13px; color: var(--dc-text-dim); margin: 10px 0 12px;
  line-height: 1.7; background: var(--dc-bg-soft); border-radius: 8px; padding: 7px 11px;
  border: 1px solid var(--dc-border);
  text-align: center;
}
/* 未选表时：撑满内容区剩余高度，文案水平与垂直都居中（选表后恢复正常条高度，不挤占规则表格） */
.qr-hint-fill {
  flex: 1;
  display: flex; align-items: center; justify-content: center; flex-wrap: wrap;
}
.qr-hint b { color: var(--dc-primary); font-weight: 600; }
/* 提示条内的跳转链接：与 <b> 同色系但可点击（虚线提示可点） */
.qr-link {
  color: var(--dc-primary); font-weight: 600; cursor: pointer;
  border-bottom: 1px dashed currentColor; transition: opacity .16s ease;
}
.qr-link:hover { opacity: .78; }

.qr-empty {
  padding: 18px; text-align: center; color: var(--dc-text-dim); font-size: 13px;
  background: var(--dc-bg-soft); border: 1px dashed var(--dc-border); border-radius: 10px;
}

/* 撑满内容区剩余高度：配合 height="100%" 让表格随弹窗高度自适应，行数少时也不留白 */
.qr-table { flex: 1; min-height: 0; border-radius: 8px; overflow: hidden; }
.qr-opt-hint { float: right; font-size: 12px; color: var(--dc-text-weak); margin-left: 12px; }
.qr-form-hint { font-size: 11.5px; color: var(--dc-text-dim); line-height: 1.5; }

.qr-level {
  font-size: 10.5px; font-weight: 600; padding: 1px 7px; border-radius: 999px;
  background: var(--dc-bg-deep); color: var(--dc-text-dim);
}
.qr-level.high { color: var(--dc-danger); }
.qr-level.mid { color: var(--dc-warning, #e6a23c); }
.qr-level.low { color: var(--dc-link); }
</style>
