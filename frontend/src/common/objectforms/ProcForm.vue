<template>
  <div class="obj-form">
    <div class="form-card">
      <div class="card-title">
        <el-icon :color="form.retType === 'FUNCTION' ? '#3b82f6' : '#f59e0b'"><component :is="form.retType === 'FUNCTION' ? Cpu : Setting" /></el-icon>
        {{ isClickhouse ? $t('mv.catFunc') : (form.retType === 'FUNCTION' ? $t('mv.catFunc') : $t('mv.catProc')) }}
      </div>
      <div class="form-grid">
        <el-form label-position="top" size="small" class="grid-item">
          <el-form-item :label="editMode ? $t('pcf.nameEdit') : $t('pcf.name')" required>
            <!-- 编辑既有过程/函数时禁用改名：改名等于先删原对象再建新的，会丢权限/依赖，请用树右键「重命名」 -->
            <el-input v-model="form.name" :disabled="editMode" clearable />
          </el-form-item>
        </el-form>
        <el-form v-if="showRetType" label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('pcf.returnType', { def: retTypeDefault })">
            <el-select v-model="form.returnType" filterable allow-create clearable :placeholder="$t('pcf.defaultVal', { def: retTypeDefault })">
              <el-option v-for="t in features.columnTypes || []" :key="t" :label="t" :value="t" />
            </el-select>
          </el-form-item>
        </el-form>
        <el-form v-if="!isClickhouse" label-position="top" size="small" class="grid-item">
          <el-form-item :label="$t('tf.colType')">
            <el-radio-group v-model="form.retType" size="small">
              <el-radio-button value="PROCEDURE">{{ $t('pcf.procedure') }}</el-radio-button>
              <el-radio-button value="FUNCTION">{{ $t('mv.catFunc') }}</el-radio-button>
            </el-radio-group>
          </el-form-item>
        </el-form>
      </div>

      <!-- 参数编辑器 -->
      <div class="sub-card">
        <div class="sub-title">
              {{ $t('pcf.params') }}
          <span class="card-tip">{{ $t('pcf.nParams', { n: params.length }) }}</span>
          <el-button size="small" type="primary" plain :icon="Plus" @click="params.push({ mode: 'IN', name: '', type: 'VARCHAR', length: '' })">{{ $t('pcf.addParam') }}</el-button>
        </div>
        <el-table :data="params" size="small" :empty-text="$t('pcf.noParams')">
          <el-table-column v-if="style !== 'pg' && !isClickhouse" :label="$t('pcf.mode')" width="100">
            <template #default="{ row }">
              <el-select v-model="row.mode" size="small">
                <el-option label="IN" value="IN" />
                <el-option label="OUT" value="OUT" />
                <el-option v-if="style === 'mysql'" label="INOUT" value="INOUT" />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column :label="$t('pcf.paramName')" min-width="120">
            <template #default="{ row }"><el-input v-model="row.name" size="small" /></template>
          </el-table-column>
          <el-table-column :label="$t('tf.colType')" width="150">
            <template #default="{ row }">
              <el-select v-model="row.type" size="small" filterable allow-create>
                <el-option v-for="t in features.columnTypes || []" :key="t" :label="t" :value="t" />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column v-if="style !== 'mssql'" :label="$t('tf.colLength')" width="80">
            <template #default="{ row }"><el-input v-model="row.length" size="small" /></template>
          </el-table-column>
          <el-table-column width="50">
            <template #default="{ $index }"><el-button text size="small" :icon="Delete" class="dc-del" @click="params.splice($index, 1)" /></template>
          </el-table-column>
        </el-table>
      </div>
    </div>

    <div class="form-card grow-card">
      <div class="card-title"><el-icon><EditPen /></el-icon>{{ $t('pcf.body') }}</div>
      <el-alert v-if="style === 'sqlite'" type="warning" :closable="false" show-icon class="mb8">
        <template #title>{{ $t('pcf.sqliteTip') }}</template>
      </el-alert>
      <SqlCodeEditor v-model="form.body" :conn-type="connType" :placeholder="bodyPlaceholder" />
      <div class="body-tip">
        <el-icon><InfoFilled /></el-icon>
        <span>{{ isClickhouse ? $t('pcf.tipCh') : $t('pcf.tipNormal') }}</span>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, computed, watch, onMounted } from 'vue'
import { ElMessage } from 'element-plus'
import { Operation, Cpu, Setting, EditPen, Plus, Delete, InfoFilled } from '@element-plus/icons-vue'
import { has, qt, sq, buildType, ddlStyleOf, oracleType, extractRoutineBody, ensureRoutineBodyEnd, parseRoutineDdl, applySpans } from './objectFormUtils'
import { t } from '../../utils/i18n'
import SqlCodeEditor from './SqlCodeEditor.vue'

const props = defineProps({
  features: { type: Object, default: () => ({}) },
  conn: { type: Object, default: () => ({}) },
  editMode: { type: Boolean, default: false },
  editData: { type: Object, default: () => ({}) }
})
const emit = defineEmits(['sql', 'kind'])

const style = computed(() => ddlStyleOf(props.features))
const isClickhouse = computed(() => style.value === 'clickhouse')
const connType = computed(() => props.conn?.type || '')
// 语句体示例占位（Monaco 空内容时显示）
const bodyPlaceholder = computed(() =>
  form.retType === 'FUNCTION'
    ? (isClickhouse.value ? $t('pcf.exCh') : $t('pcf.exRet'))
    : $t('pcf.exMulti')
)
// ClickHouse 函数由表达式推断类型，不声明返回类型
const showRetType = computed(() => style.value !== 'sqlite' && style.value !== 'clickhouse' && form.retType === 'FUNCTION')
const retTypeDefault = computed(() => {
  const s = style.value
  if (s === 'oracle') return 'NUMBER'
  if (s === 'mysql' || s === 'mssql') return 'INT'
  if (s === 'pg') return 'void'
  return 'INT'
})

const form = reactive({ name: '', retType: 'PROCEDURE', returnType: '', body: '' })
const params = reactive([])
// 编辑态解析出的原始定义（含各片段偏移），用于「按原 DDL 精确拼接」
const orig = ref(null)

const sigOfParams = (list) => (list || []).map(p => `${p.mode || 'IN'}|${p.name}|${p.type || ''}`).join(',')
const validParams = () => params.filter(p => p.name.trim())

// ClickHouse 仅为函数（无存储过程），切到 clickhouse 时强制为函数模式
watch(style, (v) => { if (v === 'clickhouse') form.retType = 'FUNCTION' })
// 过程/函数切换时上报类型，供外层标题区分「函数」与「存储过程」
watch(() => form.retType, (v) => emit('kind', v === 'FUNCTION' ? 'function' : 'procedure'))

/**
 * SQL Server 编辑态：在原定义上做最小替换（参数表 / 返回类型 / 语句体），
 * CREATE 改 ALTER。未改动的片段原样保留，因此注释、格式、WITH 选项都不会丢。
 * 返回空串表示条件不满足，调用方回退到通用生成逻辑。
 */
const spliceMssqlEdit = () => {
  const o = orig.value
  if (!props.editMode || !o || style.value !== 'mssql') return ''
  // 用户切换了过程/函数类型时不做拼接，交给通用生成
  if (form.retType !== o.retType) return ''
  const isFn = form.retType === 'FUNCTION'
  const edits = []
  // 语句体：仅在改动时替换
  const bodyNow = String(form.body ?? '')
  if (o.spans.body && bodyNow !== o.body) edits.push([o.spans.body[0], o.spans.body[1], bodyNow])
  // 返回类型
  if (isFn && o.spans.ret) {
    const ret = (form.returnType || '').trim() || retTypeDefault.value
    if (ret !== (o.returnSpec || '').trim()) edits.push([o.spans.ret[0], o.spans.ret[1], ret])
  }
  // 参数
  const valid = validParams()
  if (o.spans.params) {
    if (sigOfParams(valid) !== o.paramsSig) {
      const list = valid.map(p => `@${p.name.trim()} ${buildType(p.type, p.length)}`).join(', ')
      edits.push([o.spans.params[0], o.spans.params[1], o.spans.paramsParen ? '(' + list + ')' : list])
    }
  } else if (valid.length) {
    return ''
  }
  // 原地改名为 ALTER，保留对象 id、权限与依赖
  if (o.spans.kw) edits.push([o.spans.kw[0], o.spans.kw[1], 'ALTER'])
  const out = applySpans(o.text, edits).trim()
  return o.crlf ? out.replace(/\n/g, '\r\n') : out
}

const genSql = () => {
  const f = props.features
  const s = style.value
  const name = form.name.trim()
  if (!name) return ''
  const qn = qt(f, name)
  const isFn = form.retType === 'FUNCTION' || (s === 'pg' && !!String(form.returnType || '').trim())
  const kw = isFn ? 'FUNCTION' : 'PROCEDURE'
  // 例程体内语句必须以分号结束（各数据库通用），自动为末尾语句补分号
  let body = ensureRoutineBodyEnd(form.body)
  // 函数模式下若用户没写 RETURN，自动包装为 RETURN (expr)
  if (isFn && body && !/\bRETURN\b/i.test(body)) {
    const core = body.replace(/;\s*$/, '').trim()
    if (core) body = `RETURN (${core});`
  }
  const ret = isFn ? (form.returnType.trim() || retTypeDefault.value) : ''

  // 参数列表
  const valid = params.filter(p => p.name.trim())
  let paramStr
  if (s === 'mssql') {
    paramStr = valid.map(p => `@${p.name.trim()} ${buildType(p.type, p.length)}`).join(', ')
  } else if (s === 'pg') {
    paramStr = valid.map(p => `${p.name.trim()} ${buildType(p.type, p.length)}`).join(', ')
  } else if (s === 'oracle') {
    paramStr = valid.map(p => `${p.name.trim()} ${p.mode === 'IN' ? 'IN ' : (p.mode === 'OUT' ? 'OUT ' : 'IN OUT ')}${oracleType(p.type, p.length)}`).join(', ')
  } else {
    // MySQL / 通用等：过程参数带模式前缀；函数参数必须省略模式（MySQL 函数参数仅允许 IN，省略 IN 最稳）
    paramStr = valid.map(p => {
      const mode = isFn ? '' : `${p.mode || 'IN'} `
      return `${mode}${p.name.trim()} ${buildType(p.type, p.length)}`
    }).join(', ')
  }
  // SQL Server 无参数时不能带空括号，其他数据库可保留 ()
  if (paramStr) paramStr = '(' + paramStr + ')'
  else if (s !== 'mssql') paramStr = '()'
  else paramStr = ''

  let sql
  if (s === 'pg') {
    const hasBlock = /BEGIN\s+[\s\S]*END\s*;?\s*$/i.test(body)
    const wrappedBody = hasBlock ? body : `BEGIN\n${body}\nEND;`
    sql = `CREATE OR REPLACE ${kw} ${qn}${paramStr}${isFn ? ` RETURNS ${ret}` : ''} AS $$\n${wrappedBody}\n$$ LANGUAGE plpgsql`
  } else if (s === 'oracle') {
    sql = `CREATE OR REPLACE ${kw} ${qn}${paramStr}${isFn ? ` RETURN ${ret}` : ''} AS\nBEGIN\n${body}\nEND`
  } else if (s === 'mssql') {
    // 编辑既有对象：优先在原定义上最小替换（不改动的片段原样保留，注释/格式不丢）
    const spliced = spliceMssqlEdit()
    if (spliced) return spliced
    // 否则用 ALTER 原地修改：保留对象 id、权限、依赖（DROP 会一并丢失）
    const verb = props.editMode ? 'ALTER' : 'CREATE'
    sql = `${verb} ${kw} ${qn}${paramStr}${isFn ? ` RETURNS ${ret}` : ''}\nAS\nBEGIN\n${body}\nEND`
  } else if (s === 'clickhouse') {
    // ClickHouse 仅支持表达式函数 UDF：CREATE FUNCTION name AS (params) -> expr
    const expr = String(form.body).trim().replace(/;\s*$/, '')
    sql = `CREATE FUNCTION ${qn} AS ${paramStr} -> ${expr}`
  } else {
    sql = `CREATE ${kw} ${qn}${paramStr}${isFn ? ` RETURNS ${ret}` : ''}\nBEGIN\n${body}\nEND`
  }

  const stmts = []
  // SQL Server 用 ALTER、Oracle 用 CREATE OR REPLACE，编辑时都是原地替换，不需要 DROP
  //（DROP 会丢权限/依赖，且 SQL Server 2012 不支持 DROP ... IF EXISTS）；
  // MySQL/ClickHouse 没有“只改语句体”的 ALTER，必须 DROP 重建；
  // PostgreSQL 参数或返回类型变化时 CREATE OR REPLACE 会报错，先 DROP 更稳。
  if (props.editMode && (s === 'mysql' || s === 'pg' || s === 'clickhouse')) {
    stmts.push(`DROP ${kw} IF EXISTS ${qn}`)
  }
  stmts.push(sql)
  return stmts.join(';\n') + ';'
}

watch([form, params], () => emit('sql', genSql()), { deep: true })
onMounted(() => {
  if (props.editMode) {
    const ddl = props.editData.ddl || ''
    const meta = props.editData.meta || {}
    // 名称
    form.name = meta.name || props.editData.name || ''
    // SQL Server：DDL 常带 schema 限定名、跨行签名与表值返回，统一走结构化解析，
    // 解析后的偏移量用于保存时按原定义拼接（保持一致、不丢注释）
    if (style.value === 'mssql') {
      const parsed = parseRoutineDdl(ddl, style.value)
      if (parsed) {
        orig.value = { ...parsed, paramsSig: sigOfParams(parsed.params) }
        form.name = parsed.name || form.name
        form.retType = parsed.retType
        form.returnType = parsed.returnSpec || ''
        params.splice(0, params.length, ...parsed.params.map(p => ({ ...p })))
        form.body = parsed.body
        emit('kind', parsed.retType === 'FUNCTION' ? 'function' : 'procedure')
        emit('sql', genSql())
        return
      }
    }
    // 是否函数（优先用后端返回的 meta.type，其次正则匹配 DDL）
    const metaType = String(meta.type || '').toUpperCase()
    if (/FUNCTION|^FN$|^IF$|^TF$/.test(metaType)) {
      form.retType = 'FUNCTION'
    } else if (/CREATE\b[\s\S]*?\bFUNCTION\b/i.test(ddl)) {
      form.retType = 'FUNCTION'
    }
    // 返回类型
    const retMatch = ddl.match(/\bRETURNS?\s+([A-Za-z_]+(?:\([^)]*\))?)/i)
    if (retMatch) form.returnType = retMatch[1].trim()
    // 参数
    const paramMatch = ddl.match(/\b(?:PROCEDURE|FUNCTION)\s+[`\[\"]?\w+[`\[\"]?\.?[`\[\"]?\w*[`\[\"]?\s*\(([^)]*)\)/i)
    if (paramMatch && paramMatch[1].trim()) {
      parseParams(paramMatch[1].trim())
    }
    // 语句体
    form.body = extractRoutineBody(ddl)
    // ClickHouse 函数体是 -> 之后的表达式，去掉参数与箭头
    if (style.value === 'clickhouse') {
      const m = form.body.match(/->\s*([\s\S]*)$/i)
      if (m) form.body = m[1].trim()
    }
    // 函数：去掉自动包装的 RETURN (...)
    else if (form.retType === 'FUNCTION') {
      const m = form.body.match(/^RETURN\s*\(([\s\S]*)\);?\s*$/i)
      if (m) form.body = m[1].trim()
    }
    // 确保末尾有分号（和创建时行为一致）
    if (form.body && !/;\s*$/.test(form.body)) form.body += ';'
  }
  // ClickHouse 仅有函数，统一按函数处理
  if (style.value === 'clickhouse') form.retType = 'FUNCTION'
  emit('kind', form.retType === 'FUNCTION' ? 'function' : 'procedure')
  emit('sql', genSql())
})

const parseParams = (raw) => {
  params.splice(0, params.length)
  const s = ddlStyleOf(props.features)
  const tokens = raw.split(/,\s*/)
  for (const t of tokens) {
    const str = t.trim()
    if (!str) continue
    if (s === 'mssql') {
      // @name TYPE [OUTPUT]
      const m = str.match(/^@(\w+)\s+([A-Za-z_]+(?:\([^)]*\))?)(?:\s+OUTPUT)?/i)
      if (m) params.push({ mode: str.toUpperCase().includes('OUTPUT') ? 'OUT' : 'IN', name: m[1], type: m[2], length: '' })
    } else if (s === 'oracle') {
      // name [IN|OUT|IN OUT] TYPE
      const m = str.match(/^(\w+)\s+(?:(IN)\s+|(OUT)\s+|(IN\s+OUT)\s+)?([A-Za-z_]+(?:\([^)]*\))?)/i)
      if (m) {
        const mode = m[4] ? 'INOUT' : (m[3] ? 'OUT' : (m[2] ? 'IN' : 'IN'))
        params.push({ mode, name: m[1], type: m[5], length: '' })
      }
    } else {
      // 通用：MySQL / PG / 等  →  [IN|OUT|INOUT] name TYPE[(len)]
      const m = str.match(/^(?:(IN|OUT|INOUT)\s+)?(\w+)\s+([A-Za-z_]+(?:\([^)]*\))?)/i)
      if (m) {
        const { type, length } = parseType(m[3])
        params.push({ mode: m[1] || 'IN', name: m[2], type, length })
      }
    }
  }
}
</script>

<style scoped>
.obj-form { display: flex; flex-direction: column; flex: 1; min-height: 0; }
.form-card { background: var(--dc-bg-soft); border: 1px solid var(--dc-border-soft); border-radius: 10px; padding: 12px 14px; }
.card-title { display: flex; align-items: center; gap: 6px; font-size: 14px !important; font-weight: 500 !important; color: var(--dc-text); margin-bottom: 10px; }
.card-title .el-icon { color: #f59e0b; }
.form-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 0 16px; }
.grid-item { min-width: 0; }
.sub-card { border: 1px solid var(--dc-border-soft); border-radius: 8px; padding: 10px; }
.sub-title { display: flex; align-items: center; gap: 6px; font-size: 13px; font-weight: 600; color: var(--dc-text); margin-bottom: 8px; }
.sub-title .el-button { margin-left: auto; }
.card-tip { font-size: 12px; color: var(--dc-text-dim); font-weight: 400; }
.dc-del { color: var(--dc-text-dim); }
.dc-del:hover { color: var(--dc-danger); background: rgba(255,97,97,.1); }
.mb8 { margin-bottom: 8px; }
.body-tip { display: flex; align-items: center; gap: 5px; margin-top: 8px; font-size: 12px; color: var(--dc-text-dim); }
.body-tip .el-icon { color: var(--dc-primary); }
.grow-card { flex: 1; display: flex; flex-direction: column; min-height: 0; }
.grow-card > .sql-code-editor { flex: 1; }
</style>
