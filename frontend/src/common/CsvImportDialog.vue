<template>
  <el-dialog :model-value="visible" :title="$t('csv.title')" width="640" append-to-body
             :close-on-click-modal="false" @update:model-value="v => emit('update:visible', v)">
    <!-- 第一步：选文件 -->
    <div class="csv-step">
      <input ref="fileInput" type="file" accept=".csv,.tsv,.txt" class="csv-file" @change="onFile" />
      <span v-if="!fileName" class="csv-hint">{{ $t('csv.pickFile') }}</span>
      <span v-else class="csv-file-name">{{ fileName }}（{{ totalRows }} {{ $t('csv.rows') }}<span v-if="headerRow"> + {{ $t('csv.header') }}</span>）</span>
    </div>

    <template v-if="parsed.length">
      <!-- 第二步：参数 -->
      <div class="csv-form">
        <div class="csv-row">
          <label>{{ $t('csv.table') }}</label>
          <el-input v-model="table" :placeholder="$t('csv.tablePh')" size="small" />
        </div>
        <div class="csv-row">
          <label>{{ $t('csv.delim') }}</label>
          <el-radio-group v-model="delim" size="small">
            <el-radio-button value=",">,</el-radio-button>
            <el-radio-button value=";">;</el-radio-button>
            <el-radio-button value="\t">TAB</el-radio-button>
            <el-radio-button value="|">|</el-radio-button>
          </el-radio-group>
        </div>
        <div class="csv-row">
          <label>{{ $t('csv.options') }}</label>
          <el-checkbox v-model="headerRow" size="small">{{ $t('csv.firstRowHeader') }}</el-checkbox>
          <el-checkbox v-model="autoCreate" size="small" :title="$t('csv.autoCreateTip')">{{ $t('csv.autoCreate') }}</el-checkbox>
        </div>
      </div>

      <!-- 第三步：预览（前 5 行） -->
      <div class="csv-preview">
        <table>
          <thead>
            <tr>
              <th v-for="(h, i) in previewCols" :key="i">{{ h }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(r, ri) in previewRows" :key="ri">
              <td v-for="(cell, ci) in r" :key="ci">{{ cell }}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- 进度 -->
      <div v-if="importing || done" class="csv-progress">
        <el-progress :percentage="percent" :status="failed ? 'exception' : (done ? 'success' : '')" />
        <span class="csv-progress-text">
          {{ done ? $t('csv.done', { n: importedRows, ms: elapsedMs }) : $t('csv.progress', { done: importedRows, total: totalRows }) }}
        </span>
      </div>
      <div v-if="failed" class="csv-error">{{ failMessage }}</div>
    </template>

    <template #footer>
      <el-button v-if="importing" type="danger" plain @click="cancel">{{ $t('tree.multiCancel') }}</el-button>
      <el-button v-else @click="emit('update:visible', false)">{{ $t('common.close') }}</el-button>
      <el-button v-if="!importing" type="primary" :disabled="!canImport" @click="startImport">{{ $t('csv.start') }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed } from 'vue'
import { t } from '../utils/i18n'
import { ElMessage } from 'element-plus'
import { executeSql, executeSqlBatch } from '../api'

const props = defineProps({
  visible: Boolean,
  connId: String,
  database: { type: String, default: '' },
  /** 目标库类型（小写）：Oracle/达梦不支持多行 VALUES，走单行 INSERT（批量端点顺序执行） */
  kind: { type: String, default: '' }
})
const emit = defineEmits(['update:visible', 'imported'])

const fileInput = ref(null)
const fileName = ref('')
const rawText = ref('')
const parsed = ref([])
const table = ref('')
const delim = ref(',')
const headerRow = ref(true)
const autoCreate = ref(false)
const importing = ref(false)
const done = ref(false)
const failed = ref(false)
const failMessage = ref('')
const importedRows = ref(0)
const elapsedMs = ref(0)
let cancelFlag = false

const totalRows = computed(() => Math.max(0, parsed.value.length - (headerRow.value ? 1 : 0)))
const dataRows = computed(() => parsed.value.slice(headerRow.value ? 1 : 0).filter(r => r.some(c => c !== '')))
const columns = computed(() => {
  if (headerRow.value) {
    const head = parsed.value[0] || []
    return head.map((h, i) => (String(h).trim() || `col${i + 1}`).replace(/[\s";']/g, '_'))
  }
  const n = (dataRows.value[0] || []).length
  return Array.from({ length: n }, (_, i) => `col${i + 1}`)
})
const previewCols = computed(() => columns.value)
const previewRows = computed(() => dataRows.value.slice(0, 5))
const canImport = computed(() => !!table.value.trim() && dataRows.value.length > 0 && columns.value.length > 0)
const percent = computed(() => totalRows.value ? Math.min(100, Math.round(importedRows.value / totalRows.value * 100)) : 0)

/** 引号感知的 CSV 行解析（"" 转义、跨行字符串不支持 —— 表数据场景足够） */
const splitLine = (line, d) => {
  const cells = []
  let cur = ''
  let inQ = false
  for (let i = 0; i < line.length; i++) {
    const ch = line[i]
    if (inQ) {
      if (ch === '"') {
        if (line[i + 1] === '"') { cur += '"'; i++ } else inQ = false
      } else cur += ch
    } else if (ch === '"') inQ = true
    else if (ch === d) { cells.push(cur); cur = '' }
    else cur += ch
  }
  cells.push(cur)
  return cells
}
const detectDelim = (text) => {
  const line = text.split(/\r?\n/).find(l => l.trim()) || ''
  const counts = [[',', 0], [';', 0], ['\t', 0], ['|', 0]].map(([d]) => [d, splitLine(line, d).length])
  counts.sort((a, b) => b[1] - a[1])
  return counts[0][1] > 1 ? counts[0][0] : ','
}

const onFile = (e) => {
  const f = e.target.files && e.target.files[0]
  if (!f) return
  fileName.value = f.name
  const reader = new FileReader()
  reader.onload = () => {
    let text = String(reader.result || '')
    if (text.charCodeAt(0) === 0xfeff) text = text.slice(1) // 去 BOM
    rawText.value = text
    const d = detectDelim(text)
    delim.value = d === '\t' ? '\\t' : d
    reparse()
  }
  reader.readAsText(f, 'utf-8')
}
const reparse = () => {
  const d = delim.value === '\\t' ? '\t' : delim.value
  parsed.value = rawText.value.split(/\r?\n/).filter(l => l.length).map(l => splitLine(l, d))
  done.value = false
  failed.value = false
  importedRows.value = 0
}

/** 值字面量：NULL / 数字直写 / 其余字符串转义 —— 各 SQL 方言通用 */
const lit = (v) => {
  const s = String(v ?? '')
  if (s === '') return 'NULL'
  if (/^-?\d+(\.\d+)?$/.test(s)) return s
  return `'${s.replace(/'/g, "''")}'`
}
// 标识符引号按目标类型选：MySQL 系（默认 sql_mode）认反引号、ANSI 系认双引号 ——
// 用错引号在 MySQL 上会把 "col" 当字符串字面量（真 bug）。其它类型覆盖列表见下。
const quoteOf = (kind) => {
  if (['mysql', 'mariadb', 'doris', 'clickhouse', 'starocks', 'oceanbase'].includes(kind)) return '`'
  return '"'
}
const ident = (name) => {
  const q = quoteOf(props.kind)
  return `${q}${String(name).split(q).join(q + q)}${q}`
}

const startImport = async () => {
  if (importing.value) return
  const tbl = table.value.trim()
  const cols = columns.value
  const rows = dataRows.value
  if (!tbl || !rows.length) return
  importing.value = true
  done.value = false
  failed.value = false
  failMessage.value = ''
  importedRows.value = 0
  cancelFlag = false
  const started = Date.now()
  try {
    // 建表（可选）：VARCHAR(255) 全宽容 —— MySQL/MariaDB/PG/MSSQL/SQLite/H2/Derby/DM/Kingbase 都认；
    // ClickHouse 这类要 ENGINE 的方言不支持，提示先手动建表
    if (autoCreate.value) {
      if (props.kind === 'clickhouse') {
        failed.value = true
        failMessage.value = t('csv.noAutoCreate')
        importing.value = false
        return
      }
      const ddl = `CREATE TABLE ${ident(tbl)} (\n  ${cols.map(c => `${ident(c)} VARCHAR(255)`).join(',\n  ')}\n)`
      const r = await executeSql(props.connId, ddl, props.database, null, null, 1, 10, true)
      if (r && r.success === false) {
        failed.value = true
        failMessage.value = r.message || t('csv.createFail')
        importing.value = false
        return
      }
    }
    // 分批：每批若干行、每行一条 INSERT（Oracle/达梦也认），批内拼成多语句交给批量端点顺序执行
    const perBatch = props.kind === 'oracle' || props.kind === 'dm' ? 100 : 200
    const colList = cols.map(ident).join(', ')
    for (let i = 0; i < rows.length; i += perBatch) {
      if (cancelFlag) break
      const chunk = rows.slice(i, i + perBatch)
      const sql = chunk
        .map(r => `INSERT INTO ${ident(tbl)} (${colList}) VALUES (${cols.map((_, ci) => lit(r[ci])).join(', ')})`)
        .join(';\n')
      const b = await executeSqlBatch(props.connId, sql, props.database, 'csv_' + Date.now(), null)
      const results = (b && Array.isArray(b.results)) ? b.results : []
      const bad = results.find(x => x && x.success === false)
      if (bad) throw new Error(bad.message || t('csv.insertFail'))
      importedRows.value += chunk.length
    }
    elapsedMs.value = Date.now() - started
    done.value = importedRows.value > 0
    if (done.value && !cancelFlag) {
      ElMessage.success(t('csv.doneMsg', { n: importedRows.value }))
      emit('imported', { table: tbl, rows: importedRows.value })
    }
  } catch (e) {
    failed.value = true
    failMessage.value = e?.message || t('csv.insertFail')
  } finally {
    importing.value = false
  }
}
const cancel = () => { cancelFlag = true }
</script>

<style scoped>
.csv-step { display: flex; align-items: center; gap: 10px; margin-bottom: 12px; }
.csv-file { font-size: 12px; }
.csv-hint { font-size: 12px; color: var(--dc-text-dim); }
.csv-file-name { font-size: 12px; color: var(--dc-text); }
.csv-form { margin-bottom: 12px; }
.csv-row { display: flex; align-items: center; gap: 10px; margin-bottom: 8px; }
.csv-row > label { width: 70px; flex-shrink: 0; font-size: 12px; color: var(--dc-text-dim); text-align: right; }
.csv-preview { max-height: 200px; overflow: auto; border: 1px solid var(--dc-border); border-radius: 4px; margin-bottom: 10px; }
.csv-preview table { border-collapse: collapse; width: 100%; font-size: 12px; }
.csv-preview th, .csv-preview td { border: 1px solid var(--dc-border); padding: 2px 8px; text-align: left; white-space: nowrap; }
.csv-preview th { background: var(--dc-bg-soft); position: sticky; top: 0; }
.csv-progress { margin-top: 6px; }
.csv-progress-text { font-size: 12px; color: var(--dc-text-dim); }
.csv-error { margin-top: 6px; font-size: 12px; color: var(--el-color-danger); white-space: pre-wrap; }
</style>
