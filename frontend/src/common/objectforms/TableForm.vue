<template>
  <div class="obj-form">
    <el-tabs v-model="activeTab" type="border-card" class="table-tabs">
      <!-- 基本信息 -->
      <el-tab-pane :label="$t('tf.basic')" name="basic">
        <div class="tab-inner">
          <div class="form-grid">
            <el-form label-position="top" size="small" class="grid-item">
              <el-form-item :label="$t('tf.tableName')" required>
                <el-input v-model="form.name" :disabled="editMode" clearable />
              </el-form-item>
            </el-form>
            <el-form v-if="has(features,'supportsComment')" label-position="top" size="small" class="grid-item">
              <el-form-item :label="$t('tf.tableComment')">
                <el-input v-model="form.comment" clearable />
              </el-form-item>
            </el-form>
            <!-- Doris 专属：数据模型 / 分桶列 / 分桶数 / 副本数 -->
            <el-form v-if="isDoris" label-position="top" size="small" class="grid-item">
              <el-form-item :label="$t('tf.dorisModel')">
                <el-select v-model="form.dorisModel" size="small" :title="$t('tf.dorisModelTip')" @change="dorisModelTouched = true">
                  <el-option v-for="m in ['DUPLICATE KEY', 'UNIQUE KEY']" :key="m" :label="m" :value="m" />
                </el-select>
              </el-form-item>
            </el-form>
            <el-form v-if="isDoris" label-position="top" size="small" class="grid-item">
              <el-form-item :label="$t('tf.distCol')">
                <el-select v-model="form.dorisDistCol" multiple clearable filterable size="small" :placeholder="$t('tf.distColPh')">
                  <el-option v-for="k in dorisKeyCols" :key="k" :label="k" :value="k" />
                </el-select>
              </el-form-item>
            </el-form>
            <el-form v-if="isDoris" label-position="top" size="small" class="grid-item">
              <el-form-item :label="$t('tf.buckets')">
                <el-input-number v-model="form.dorisBuckets" :min="1" :max="1024" controls-position="right" class="num-item" />
              </el-form-item>
            </el-form>
            <el-form v-if="isDoris" label-position="top" size="small" class="grid-item">
              <el-form-item :label="$t('tf.replicas')" :title="$t('tf.replicasTip')">
                <el-input-number v-model="form.dorisReplication" :min="1" :max="32" controls-position="right" class="num-item" />
              </el-form-item>
            </el-form>
            <el-form v-if="has(features,'supportsTableOptions') && (features.engines || []).length" label-position="top" size="small" class="grid-item">
              <el-form-item :label="$t('tf.engine')">
                <el-select v-model="form.engine" clearable filterable>
                  <el-option v-for="e in features.engines" :key="e" :label="e" :value="e" />
                </el-select>
              </el-form-item>
            </el-form>
            <el-form v-if="has(features,'supportsTableOptions') && (features.charsets || []).length" label-position="top" size="small" class="grid-item">
              <el-form-item :label="$t('tf.charset')">
                <el-select v-model="form.charset" clearable filterable @change="onCharsetChange">
                  <el-option v-for="c in features.charsets" :key="c" :label="c" :value="c" />
                </el-select>
              </el-form-item>
            </el-form>
            <el-form v-if="has(features,'supportsTableOptions') && collations.length" label-position="top" size="small" class="grid-item">
              <el-form-item :label="$t('tf.collation')">
                <el-select v-model="form.collation" clearable filterable>
                  <el-option v-for="c in collations" :key="c" :label="c" :value="c" />
                </el-select>
              </el-form-item>
            </el-form>
          </div>
        </div>
      </el-tab-pane>

      <!-- 字段编辑 -->
      <el-tab-pane :label="$t('tf.tabColumns', { n: cols.filter(c => c.name).length })" name="columns">
        <div class="tab-inner col-tab">
          <div class="card-actions">
            <span class="move-tip">{{ $t('tf.moveTip') }}</span>
            <el-button size="small" :icon="Top" :disabled="!cols.length" :title="$t('tf.moveUpTip')" @click="move(-1)">{{ $t('tf.moveUp') }}</el-button>
            <el-button size="small" :icon="Bottom" :disabled="!cols.length" :title="$t('tf.moveDownTip')" @click="move(1)">{{ $t('tf.moveDown') }}</el-button>
            <el-button size="small" type="primary" :icon="Plus" @click="addColumn">{{ $t('tf.addColumn') }}</el-button>
          </div>
          <div class="field-table-wrap">
            <table class="field-table">
              <thead>
                <tr>
                  <th style="width:40px" :title="$t('tf.colOrder')">#</th>
                  <th style="width:120px">{{ $t('tf.colName') }}</th>
                  <th style="width:110px">{{ $t('tf.colType') }}</th>
                  <th style="width:64px">{{ $t('tf.colLength') }}</th>
                  <th style="width:64px">{{ $t('tf.colPrecision') }}</th>
                  <th class="c-center" style="width:56px">{{ $t('tf.colPrimary') }}</th>
                  <th class="c-center" style="width:56px">{{ $t('tf.colNullable') }}</th>
                  <th v-if="showOrderKey" class="c-center" style="width:70px" :title="$t('tf.orderKeyTip')">{{ $t('tf.orderKey') }}</th>
                  <th style="width:150px">{{ $t('tf.colDefault') }}</th>
                  <th v-if="showAutoIncrement" class="c-center" style="width:56px">{{ $t('tf.colAutoInc') }}</th>
                  <th v-if="showComment" style="width:100px">{{ $t('udv.fComment') }}</th>
                  <th style="width:40px"></th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="(c, i) in cols" :key="c.uid" :class="{ 'pk-row': c.primaryKey, 'ok-row': c.orderKey, 'sel-row': c._sel }" @click="selectRow(c)">
                  <td><span class="row-idx">{{ i + 1 }}</span></td>
                  <td><el-input v-model="c.name" size="small" clearable /></td>
                  <td>
                    <el-select v-model="c.type" size="small" filterable allow-create class="type-select">
                      <el-option v-for="t in features.columnTypes || []" :key="t" :label="t" :value="t" />
                    </el-select>
                  </td>
                  <td><el-input v-model="c.length" size="small" /></td>
                  <td><el-input v-model="c.scale" size="small" /></td>
                  <td class="c-center"><el-checkbox v-model="c.primaryKey" @click.stop @change="onChKeyChange(c)" /></td>
                  <td class="c-center"><el-checkbox v-model="c.nullable" /></td>
                  <td v-if="showOrderKey" class="c-center"><el-checkbox v-model="c.orderKey" @click.stop @change="onChKeyChange(c)" :title="$t('tf.orderKeyCellTip')" /></td>
                  <td><el-input v-model="c.defaultValue" size="small" /></td>
                  <td v-if="showAutoIncrement" class="c-center"><el-checkbox v-model="c.autoIncrement" @change="onAutoIncChange(c)" /></td>
                  <td v-if="showComment"><el-input v-model="c.comment" size="small" /></td>
                  <td><el-button text size="small" :icon="Delete" class="dc-del" @click="removeColumn(i)" /></td>
                </tr>
                <tr v-if="!cols.length">
                  <td :colspan="headSpan" class="empty-row">{{ $t('tf.noColumns') }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </el-tab-pane>

      <!-- 索引编辑 -->
      <el-tab-pane :label="$t('tf.tabIndexes', { n: idxs.length })" name="indexes">
        <div class="tab-inner idx-tab">
          <div class="card-actions">
            <el-button size="small" type="primary" :icon="Plus" @click="addIndex">{{ $t('tf.addIndex') }}</el-button>
          </div>
          <el-table :data="idxs" size="small" :empty-text="$t('tf.noIndexes')">
            <el-table-column :label="$t('tf.idxName')" min-width="140">
              <template #default="{ row }"><el-input v-model="row.name" size="small" /></template>
            </el-table-column>
            <el-table-column :label="$t('tf.idxUnique')" width="64">
              <template #default="{ row }"><el-switch v-model="row.unique" size="small" /></template>
            </el-table-column>
            <el-table-column v-if="(features.indexTypes || []).length" :label="$t('tf.colType')" width="120">
              <template #default="{ row }">
                <el-select v-model="row.indexType" size="small" clearable>
                  <el-option v-for="t in features.indexTypes" :key="t" :label="t" :value="t" />
                </el-select>
              </template>
            </el-table-column>
            <el-table-column :label="$t('tf.idxColumns')" min-width="180">
              <template #default="{ row }">
                <el-select v-model="row.columns" size="small" multiple filterable :multiple-limit="0">
                  <el-option v-for="c in cols.filter(x => x.name.trim())" :key="c.uid" :label="c.name" :value="c.name.trim()" />
                </el-select>
              </template>
            </el-table-column>
            <el-table-column width="50">
              <template #default="{ $index }">
                <el-button text size="small" :icon="Delete" class="dc-del" @click="idxs.splice($index, 1)" />
              </template>
            </el-table-column>
          </el-table>
        </div>
      </el-tab-pane>
    </el-tabs>
  </div>
</template>

<script setup>
import { ref, reactive, computed, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { Plus, Delete, Top, Bottom } from '@element-plus/icons-vue'
import { has, qt, sq, buildType, ddlStyleOf, uid } from './objectFormUtils'
import { t } from '../../utils/i18n'

const props = defineProps({
  features: { type: Object, default: () => ({}) },
  editMode: { type: Boolean, default: false }
})
const emit = defineEmits(['sql'])

const activeTab = ref('basic')

const isClickHouse = computed(() => ddlStyleOf(props.features) === 'clickhouse')
// 各列是否显示由后端 features（数据源能力）决定
const showOrderKey = computed(() => isClickHouse.value)
const showAutoIncrement = computed(() => has(props.features, 'supportsAutoIncrement'))
const showComment = computed(() => has(props.features, 'supportsComment'))
// 字段表总列数：固定列 #/字段名/类型/长度/精度/主键/可空/默认值/删除 =9；{{ $t('tf.orderKeyCellTip') }}/自增/注释按能力追加
const headSpan = computed(() => 9
  + (showOrderKey.value ? 1 : 0)
  + (showAutoIncrement.value ? 1 : 0)
  + (showComment.value ? 1 : 0))

const emptyCol = () => ({ uid: uid(), name: '', type: 'VARCHAR', length: '255', scale: '', nullable: true, defaultValue: '', autoIncrement: false, primaryKey: false, orderKey: false, comment: '', _sel: false })

const form = reactive({
  name: '', comment: '', engine: '', charset: '', collation: '',
  dorisModel: 'DUPLICATE KEY', dorisDistCol: [], dorisBuckets: 10, dorisReplication: 1,
  columns: [emptyCol()],
  indexes: []
})
const cols = form.columns
const idxs = form.indexes

const isDoris = computed(() => has(props.features, 'doris'))

/** Doris 模型 Key 列：优先主键列，否则首个有名字段 */
const dorisKeyCols = computed(() => {
  const named = cols.filter(c => c.name.trim())
  if (!named.length) return []
  const pks = named.filter(c => c.primaryKey)
  return pks.length ? pks.map(c => c.name.trim()) : [named[0].name.trim()]
})
watch(dorisKeyCols, (ks) => {
  if (!isDoris.value || !ks.length) return
  const kept = form.dorisDistCol.filter(c => ks.includes(c))
  form.dorisDistCol = kept.length ? kept : [ks[0]]
})
/** 勾选主键列时自动切换为 UNIQUE KEY（用户手动改过模型则不再干预） */
const dorisModelTouched = ref(false)
const pkCnt = computed(() => cols.filter(c => c.primaryKey).length)
watch(pkCnt, (n) => {
  if (!isDoris.value || dorisModelTouched.value) return
  form.dorisModel = n ? 'UNIQUE KEY' : 'DUPLICATE KEY'
})

const collations = computed(() => (form.charset ? (props.features.collations || {})[form.charset] || [] : []))

const onCharsetChange = () => { form.collation = '' }

/** 勾选主键/{{ $t('tf.orderKeyCellTip') }}时清除可空：ClickHouse {{ $t('tf.orderKeyCellTip') }}列不允许为 Nullable */
const onChKeyChange = (c) => {
  if ((c.orderKey || c.primaryKey) && c.nullable) c.nullable = false
}
/** Doris 自增列约束：每表最多一列，且列类型必须 BIGINT、不允许 NULL（Doris 2.1+ 的 AUTO_INCREMENT 要求） */
const onAutoIncChange = (c) => {
  if (!c.autoIncrement || !isDoris.value) return
  cols.forEach(x => { if (x !== c) x.autoIncrement = false })
  c.type = 'BIGINT'
  c.nullable = false
  c.length = ''
  c.scale = ''
}
/** 当前列是否属于将生成的 ORDER BY {{ $t('tf.orderKeyCellTip') }}（优先「{{ $t('tf.orderKeyCellTip') }}」勾选，否则主键） */
const isChOrderKeyCol = (c) => {
  const named = cols.filter(x => x.name.trim())
  const sorts = named.filter(x => x.orderKey)
  const pks = named.filter(x => x.primaryKey)
  const orderCols = sorts.length ? sorts : pks
  return orderCols.includes(c)
}

const addColumn = () => cols.push(emptyCol())
const removeColumn = (i) => { if (cols.length > 1) cols.splice(i, 1); else ElMessage.warning(t('tf.keepOne')) }
const selectRow = (c) => {
  for (const col of cols) col._sel = col === c
}
const move = (dir) => {
  const i = cols.findIndex(c => c._sel)
  if (i < 0) return ElMessage.warning(t('tf.pickRowFirst'))
  const j = i + dir
  if (j < 0 || j >= cols.length) return
  const tmp = cols[i]; cols[i] = cols[j]; cols[j] = tmp
}

const addIndex = () => {
  if (!cols.some(c => c.name.trim())) return ElMessage.warning(t('tf.addColumnFirst'))
  idxs.push({ name: 'idx_' + (cols.find(c => c.name.trim())?.name.trim() || 'col'), unique: false, indexType: '', columns: [] })
}

// 组合「长度」和「精度」为类型参数，如 DECIMAL(10,2)
const typeParam = (c) => {
  const l = String(c.length ?? '').trim()
  const s = String(c.scale ?? '').trim()
  if (l && s) return `${l},${s}`
  return l || s
}

// ==================== SQL 生成 ====================

const buildColDef = (c) => {
  const style = ddlStyleOf(props.features)
  const f = props.features
  const qn = qt(f, c.name.trim())
  const t = c.type || 'VARCHAR'
  const len = typeParam(c)

  if (style === 'clickhouse') {
    // ClickHouse：列定义 `name 类型 [DEFAULT 表达式] [COMMENT '注释']`
    // 可空是类型修饰：勾「可空」的普通列包 Nullable(T)；{{ $t('tf.orderKeyCellTip') }}/主键列不允许可空
    const typeBase = clickhouseType(t, len)
    const inOrder = isChOrderKeyCol(c)
    const typeStr = c.nullable && !inOrder && !/^Nullable\(/i.test(typeBase) ? `Nullable(${typeBase})` : typeBase
    let def = `${qn} ${typeStr}`
    const dv = String(c.defaultValue ?? '').trim()
    if (dv !== '' && !/^null$/i.test(dv)) {
      if (/^-?\d+(\.\d+)?$/.test(dv) || /^(true|false|now\(\)|today\(\)|current_timestamp|toDateTime\(.*\))$/i.test(dv)) {
        def += ' DEFAULT ' + dv
      } else {
        def += ` DEFAULT '${sq(dv)}'`
      }
    }
    if (c.comment) def += ` COMMENT '${sq(c.comment)}'`
    return def
  }

  if (style === 'sqlite') {
    let def = `${qn} ${buildType(t, len)}`
    if (c.primaryKey) {
      if (c.autoIncrement && /INTEGER/i.test(t)) def = `${qn} INTEGER PRIMARY KEY AUTOINCREMENT`
      else def = `${qn} ${buildType(t, len)} PRIMARY KEY`
    } else {
      def += c.nullable ? '' : ' NOT NULL'
      if (String(c.defaultValue ?? '').trim()) def += defaultPartSqlite(c.defaultValue)
    }
    return def
  }

  let def = qn + ' ' + buildType(t, len)
  if (style === 'oracle') def = qn + ' ' + oracleType(t, len)
  if (style === 'mssql') def = qn + ' ' + buildType(t, len)
  if (style === 'pg') def = qn + ' ' + buildType(t, len)

  const autoInc = c.autoIncrement && has(f, 'supportsAutoIncrement')
  // Doris 自增列必须 NOT NULL
  const notNull = c.primaryKey || !c.nullable || (autoInc && isDoris.value)
  // 自增（MySQL / MSSQL / Oracle / PG / Doris）
  if (autoInc) {
    if (style === 'mssql') def += ' IDENTITY(1,1)'
    else if (style === 'oracle') def += ' GENERATED BY DEFAULT AS IDENTITY'
    else if (style === 'pg') def = qn + ' SERIAL'
    // Doris：NOT NULL 必须写在 AUTO_INCREMENT 之前（id BIGINT NOT NULL AUTO_INCREMENT）
    else if (isDoris.value) def += ' NOT NULL AUTO_INCREMENT'
    else def += ' AUTO_INCREMENT'
  }
  // 主键列强制 NOT NULL（主键语义约束，Doris 的 UNIQUE KEY 亦要求 key 列非空；Doris 自增已在上面写过）
  if (notNull && !(autoInc && isDoris.value)) def += ' NOT NULL'
  // 默认值（Doris 的 Key 列不允许 DEFAULT）
  const dv = String(c.defaultValue ?? '').trim()
  if (dv !== '' && !(isDoris.value && c.primaryKey)) {
    if (style === 'mysql') def += defaultPartMysql(dv)
    else def += defaultPartGeneric(dv, style)
  }
  // 注释（列级，仅 MySQL / Doris；Doris 兼容 MySQL 语法故沿用 mysql 风格）
  if (style === 'mysql' && c.comment) def += ` COMMENT '${sq(c.comment)}'`
  return def
}

const defaultPartMysql = (v) => /^(null|true|false|current_timestamp)$/i.test(v) ? ` DEFAULT ${v}` : ` DEFAULT '${sq(v)}'`
const defaultPartGeneric = (v, style) => {
  if (/^null$/i.test(v)) return ' DEFAULT NULL'
  if (/^true$/i.test(v)) return style === 'pg' ? ' DEFAULT TRUE' : " DEFAULT 'true'"
  if (/^false$/i.test(v)) return style === 'pg' ? ' DEFAULT FALSE' : " DEFAULT 'false'"
  if (/^current_timestamp$/i.test(v)) return style === 'pg' ? ' DEFAULT CURRENT_TIMESTAMP' : ' DEFAULT CURRENT_TIMESTAMP'
  if (/^now\(\)$/i.test(v)) return style === 'pg' ? ' DEFAULT now()' : ' DEFAULT NOW()'
  if (/^-?\d+(\.\d+)?$/.test(v)) return ` DEFAULT ${v}`
  return ` DEFAULT '${sq(v)}'`
}

const defaultPartSqlite = (v) => {
  if (/^null$/i.test(v)) return ' DEFAULT NULL'
  if (/^-?\d+(\.\d+)?$/.test(v)) return ` DEFAULT ${v}`
  return ` DEFAULT '${sq(v)}'`
}

const oracleType = (t, len) => {
  const up = (t || '').toUpperCase()
  const map = {
    VARCHAR: 'VARCHAR2', VARCHAR2: 'VARCHAR2', CHAR: 'CHAR', TEXT: 'CLOB',
    INTEGER: 'NUMBER(10)', BIGINT: 'NUMBER(19)', SMALLINT: 'NUMBER(5)',
    DECIMAL: 'NUMBER', NUMERIC: 'NUMBER', FLOAT: 'NUMBER', REAL: 'BINARY_DOUBLE', DOUBLE: 'BINARY_DOUBLE',
    DATE: 'DATE', TIME: 'DATE', TIMESTAMP: 'TIMESTAMP', DATETIME: 'TIMESTAMP',
    BOOLEAN: 'NUMBER(1)', BLOB: 'BLOB', JSON: 'CLOB'
  }
  const base = map[up] || up
  if (base === 'NUMBER' && String(len ?? '').trim()) return `NUMBER(${len})`
  if ((up === 'VARCHAR' || up === 'VARCHAR2' || up === 'CHAR') && String(len ?? '').trim()) return `${base}(${len})`
  if (base === 'NUMBER' && up === 'DECIMAL' && String(len ?? '').trim()) return `NUMBER(${len})`
  return base
}

/**
 * 通用/MySQL 类型 → ClickHouse 类型。
 * ClickHouse 原生类型（后端 columnTypes 下拉里的 UInt/Int/String/DateTime 等）原样保留。
 */
const clickhouseType = (t, len) => {
  const raw = String(t ?? '').trim()
  if (!raw) return 'String'
  const head = raw.replace(/\(.*/, '').trim().toUpperCase()
  const l = String(len ?? '').trim()
  const map = {
    VARCHAR: 'String', VARCHAR2: 'String', CHAR: 'String', TEXT: 'String',
    LONGTEXT: 'String', MEDIUMTEXT: 'String', TINYTEXT: 'String',
    TINYBLOB: 'String', BLOB: 'String', MEDIUMBLOB: 'String', LONGBLOB: 'String', CLOB: 'String',
    TINYINT: 'Int8', SMALLINT: 'Int16', MEDIUMINT: 'Int32', INT: 'Int32', INTEGER: 'Int32', BIGINT: 'Int64',
    FLOAT: 'Float32', REAL: 'Float64', DOUBLE: 'Float64', 'DOUBLE PRECISION': 'Float64',
    BOOL: 'Boolean', BOOLEAN: 'Boolean', TIMESTAMP: 'DateTime', DATETIME: 'DateTime',
    NUMERIC: 'Decimal', DECIMAL: 'Decimal', FIXEDSTRING: 'FixedString', FIXED_STRING: 'FixedString'
  }
  const mapped = map[head]
  if (mapped === 'Decimal') return l ? `Decimal(${l})` : 'Decimal(38, 6)'
  if (mapped === 'DateTime') return l ? `DateTime64(${l})` : 'DateTime'
  if (mapped === 'FixedString') return `FixedString(${l || '16'})`
  if (mapped) return mapped
  // 原生 ClickHouse 类型：head 已大写化，须返回官方驼峰拼写（ClickHouse 类型名大小写敏感）
  const nativeMap = {
    UINT8: 'UInt8', UINT16: 'UInt16', UINT32: 'UInt32', UINT64: 'UInt64',
    INT8: 'Int8', INT16: 'Int16', INT32: 'Int32', INT64: 'Int64',
    FLOAT32: 'Float32', FLOAT64: 'Float64', STRING: 'String',
    BOOLEAN: 'Boolean', DATE: 'Date', UUID: 'UUID', IPV4: 'IPv4', IPV6: 'IPv6'
  }
  if (nativeMap[head]) return nativeMap[head]
  if (/^datetime64$/i.test(head)) return l ? `DateTime64(${l})` : 'DateTime64(3)'
  if (/^decimal$/i.test(head)) return l ? `Decimal(${l})` : 'Decimal(38, 6)'
  if (/^fixedstring$/i.test(head)) return `FixedString(${l || '16'})`
  if (raw.includes('(') || /^(array|map|nullable|lowcardinality|enum|tuple|json)$/i.test(head)) return raw
  return l ? `${head}(${l})` : head
}

/** ClickHouse 建表：ENGINE = ... + ORDER BY/PRIMARY KEY + COMMENT（MergeTree 族必备 ORDER BY） */
const genClickhouseSql = (f, qn, cols) => {
  const lines = cols.map(c => '  ' + buildColDef(c))
  // 跳数索引（MergeTree 族）内联在列定义括号内
  for (const ix of idxs) {
    if (!ix.name.trim() || !ix.columns.length) continue
    const qcols = ix.columns.map(c => qt(f, c))
    const it = ix.indexType || (f.indexTypes && f.indexTypes.length ? f.indexTypes[0] : 'minmax')
    lines.push(`  INDEX ${qt(f, ix.name.trim())} (${qcols.join(', ')}) TYPE ${it} GRANULARITY 1`)
  }
  let sql = 'CREATE TABLE ' + qn + ' (\n' + lines.join(',\n') + '\n)'
  const eng = String(form.engine || 'MergeTree').trim() || 'MergeTree'
  // MergeTree 族带 ()；Log/TinyLog/Memory/Null 等无参引擎不加括号
  if (/mergeTree/i.test(eng) && !eng.includes('(')) sql += '\nENGINE = ' + eng + '()'
  else sql += '\nENGINE = ' + eng
  // {{ $t('tf.orderKeyCellTip') }}(ORDER BY)：优先「{{ $t('tf.orderKeyCellTip') }}」勾选列；未勾选时退化为主键列
  const sortRaw = cols.filter(c => c.orderKey && c.name.trim()).map(c => c.name.trim())
  const pkRaw = cols.filter(c => c.primaryKey && c.name.trim()).map(c => c.name.trim())
  const orderRaw = sortRaw.length ? sortRaw : (pkRaw.length ? pkRaw : [])
  if (orderRaw.length) {
    sql += '\nORDER BY (' + orderRaw.map(n => qt(f, n)).join(', ') + ')'
    // ClickHouse 主键默认等于{{ $t('tf.orderKeyCellTip') }}；仅当主键恰为{{ $t('tf.orderKeyCellTip') }}前缀时显式声明 PRIMARY KEY
    if (pkRaw.length && pkRaw.every((n, idx) => orderRaw[idx] === n)) {
      sql += '\nPRIMARY KEY (' + pkRaw.map(n => qt(f, n)).join(', ') + ')'
    }
  } else if (/mergeTree/i.test(eng)) {
    // MergeTree 族必须指定{{ $t('tf.orderKeyCellTip') }}，未定义排序字段时用空元组兜底
    sql += '\nORDER BY tuple()'
  }
  if (form.comment) sql += `\nCOMMENT '${sq(form.comment)}'`
  return sql + ';'
}

const genSql = () => {
  const style = ddlStyleOf(props.features)
  const f = props.features
  const tName = form.name.trim()
  if (!tName) return ''
  const qn = qt(f, tName)
  const validCols = cols.filter(c => c.name.trim())
  if (!validCols.length) return ''

  // ClickHouse：独立分支（ENGINE/ORDER BY/COMMENT 语法与通用模板差异大）
  if (style === 'clickhouse') return genClickhouseSql(f, qn, validCols)

  const pkCols = validCols.filter(c => c.primaryKey).map(c => qt(f, c.name.trim()))
  const lines = validCols.map(c => '  ' + buildColDef(c))

  if (style === 'sqlite') {
    // SQLite：主键已列级定义，不重复约束
  } else if (pkCols.length && !isDoris.value) {
    // Doris 的 Key 模型在表选项区生成（UNIQUE KEY / DUPLICATE KEY），不在此输出 PRIMARY KEY
    if (style === 'mssql') {
      lines.push(`  CONSTRAINT ${qt(f, 'PK_' + tName)} PRIMARY KEY (${pkCols.join(', ')})`)
    } else {
      lines.push(`  PRIMARY KEY (${pkCols.join(', ')})`)
    }
  }

  // 表内索引（MySQL，不含 Doris）
  if (style === 'mysql' && !isDoris.value) {
    for (const ix of idxs) {
      if (!ix.name.trim() || !ix.columns.length) continue
      const qcols = ix.columns.map(c => qt(f, c))
      lines.push(`  ${ix.unique ? 'UNIQUE KEY' : 'KEY'} ${qt(f, ix.name.trim())} (${qcols.join(', ')})`)
    }
  }

  // Doris：列定义括号内的非唯一 INDEX（唯一索引由表模型 Key 承担）
  if (isDoris.value) {
    for (const ix of idxs) {
      if (ix.unique || !ix.name.trim() || !ix.columns.length) continue
      const qcols = ix.columns.map(c => qt(f, c))
      lines.push(`  INDEX ${qt(f, ix.name.trim())} (${qcols.join(', ')})`)
    }
  }

  const parts = ['CREATE TABLE ' + qn + ' (\n' + lines.join(',\n') + '\n)']

  // 表选项（MySQL，不含 Doris）
  if (style === 'mysql' && !isDoris.value) {
    const opts = []
    if (form.engine) opts.push('ENGINE=' + form.engine)
    if (form.charset) opts.push('DEFAULT CHARSET=' + form.charset)
    if (form.collation) opts.push('COLLATE=' + form.collation)
    if (form.comment) opts.push("COMMENT='" + sq(form.comment) + "'")
    if (opts.length) parts[0] += ' ' + opts.join(' ')
  }

  // 表选项（Doris：ENGINE=OLAP + Key 模型 + COMMENT + DISTRIBUTED BY + PROPERTIES）
  if (isDoris.value) {
    const keys = dorisKeyCols.value
    if (!keys.length) return ''
    const distCols = form.dorisDistCol.filter(c => keys.includes(c))
    const finalDistCols = distCols.length ? distCols : keys.slice(0, 1)
    const opts = ['ENGINE=OLAP']
    opts.push(`${form.dorisModel || 'DUPLICATE KEY'} (${keys.map(c => qt(f, c)).join(', ')})`)
    if (form.comment) opts.push(`COMMENT '${sq(form.comment)}'`)
    opts.push(`DISTRIBUTED BY HASH(${finalDistCols.map(c => qt(f, c)).join(', ')}) BUCKETS ${form.dorisBuckets || 10}`)
    const repl = Number(form.dorisReplication)
    if (repl > 0) opts.push(`PROPERTIES ("replication_num" = "${repl}")`)
    parts[0] += ' ' + opts.join(' ')
  }

  // 表注释（PG / Oracle / MSSQL）
  if (form.comment && style !== 'mysql' && has(f, 'supportsComment')) {
    if (style === 'mssql') parts.push(`EXEC sp_addextendedproperty 'MS_Description', '${sq(form.comment)}', 'SCHEMA', dbo, 'TABLE', ${qn}`)
    else parts.push(`COMMENT ON TABLE ${qn} IS '${sq(form.comment)}'`)
  }

  // 列注释（PG / Oracle）
  if (has(f, 'supportsComment') && (style === 'pg' || style === 'oracle')) {
    for (const c of validCols) {
      if (c.comment) parts.push(`COMMENT ON COLUMN ${qn}.${qt(f, c.name.trim())} IS '${sq(c.comment)}'`)
    }
  }

  // 独立索引语句（PG / Oracle / MSSQL / 通用）
  if (style !== 'mysql') {
    for (const ix of idxs) {
      if (!ix.name.trim() || !ix.columns.length) continue
      const qcols = ix.columns.map(c => qt(f, c))
      let s = `CREATE ${ix.unique ? 'UNIQUE ' : ''}INDEX ${qt(f, ix.name.trim())} ON ${qn} (${qcols.join(', ')})`
      if (ix.indexType && (style === 'pg')) s += ` USING ${ix.indexType}`
      parts.push(s)
    }
  }

  return parts.join(';\n') + ';'
}

watch(form, () => emit('sql', genSql()), { deep: true })
// 初始触发一次 SQL 预览
emit('sql', genSql())
</script>

<style scoped>
.obj-form { display: flex; flex-direction: column; flex: 1; min-height: 0; }
.table-tabs { flex: 1; display: flex; flex-direction: column; min-height: 0; }
.table-tabs :deep(.el-tabs__content) { flex: 1; overflow: hidden; padding: 0; }
.table-tabs :deep(.el-tab-pane) { height: 100%; }
.tab-inner { height: 100%; padding: 10px 12px; display: flex; flex-direction: column; }
.col-tab { padding-top: 6px; }
.col-tab .card-actions { display: flex; align-items: center; gap: 6px; justify-content: flex-start; margin-bottom: 8px; }
.col-tab .card-actions .move-tip { margin-right: auto; font-size: 13px; color: var(--dc-text-dim); }
.col-tab .field-table-wrap { flex: 1; overflow: auto; border: 1px solid var(--dc-border-soft); border-radius: 8px; }
.idx-tab { padding-top: 6px; }
.idx-tab .card-actions { display: flex; gap: 6px; justify-content: flex-end; margin-bottom: 8px; }
.form-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 0 16px; }
.form-grid .grid-item { min-width: 0; }
.field-table { width: 100%; border-collapse: collapse; table-layout: fixed; }
.field-table th { position: sticky; top: 0; z-index: 1; background: var(--dc-bg-code); color: var(--dc-text-mid); font-weight: 600; font-size: 13px; text-align: left; padding: 7px 6px; border-bottom: 1px solid var(--dc-border-soft); white-space: nowrap; }
.field-table td { padding: 4px 4px; border-bottom: 1px solid var(--dc-border-soft); vertical-align: middle; }
/* 主键 / 可空 / 自增等开关列：表头与内容居中，其余列左对齐 */
.field-table th.c-center,
.field-table td.c-center { text-align: center; }
.row-idx { color: var(--dc-text-dim); font-size: 13px; }
.field-table tbody tr { cursor: pointer; }
.field-table tbody tr:hover { background: var(--dc-bg-hover); }
.field-table tr.pk-row { background: rgba(79, 140, 255, .05); }
.field-table tr.ok-row { background: rgba(103, 194, 58, .05); }
.field-table tbody tr.sel-row { background: rgba(79, 140, 255, .16) !important; box-shadow: inset 2px 0 0 var(--dc-primary); }
.field-table tbody tr.sel-row:hover { background: rgba(79, 140, 255, .22) !important; }
.field-table tbody tr.pk-row.sel-row { background: rgba(79, 140, 255, .2) !important; }
.field-table tbody tr.ok-row.sel-row { background: rgba(103, 194, 58, .18) !important; box-shadow: inset 2px 0 0 var(--dc-success); }
.empty-row { text-align: center; color: var(--dc-text-dim); padding: 18px 0 !important; font-size: 13px; cursor: default; }
.grid-item :deep(.el-input-number) { width: 100%; }
.form-grid { align-items: start; }
.form-grid :deep(.el-form-item) { margin-bottom: 12px; }
.form-grid :deep(.el-select) { width: 100%; }
.dc-del { color: var(--dc-text-dim); }
.dc-del:hover { color: var(--dc-danger); background: rgba(255,97,97,.1); }
.type-select { width: 100%; }
:deep(.el-table) { --el-table-header-bg-color: var(--dc-bg-code); }
/* 索引表表头/内容左对齐 */
.table-tabs :deep(.el-table th.el-table__cell > .cell),
.table-tabs :deep(.el-table td.el-table__cell > .cell) { text-align: left; }
</style>
