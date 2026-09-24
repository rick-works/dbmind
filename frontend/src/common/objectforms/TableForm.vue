<template>
  <div class="obj-form">
    <!-- 不用 border-card：那会带一层灰底标题带 + Element 自带下划线，
         与「编辑表结构」的"纯文字页签 + 独立卡片"不是一回事（见下方样式）。 -->
    <el-tabs v-model="activeTab" class="table-tabs">
      <!-- 基本信息 -->
      <el-tab-pane :label="$t('tf.basic')" name="basic">
        <div class="tab-inner basic-tab">
          <!-- 与「字段定义」同一张表：左列「项目」180px、右列「值」。
               编辑页的基本信息就是这么做的（.field-table.basic-table）——
               之前这里用"每字段一个 el-form 的网格"，标签与控件各占一格，
               行与行之间高度不齐，看着就是乱的。同一张表就没有这个问题：
               行高、表头、网格线全部与字段表共用一套。 -->
          <div class="field-table-wrap">
            <table class="field-table basic-table">
              <colgroup>
                <col style="width:180px" />
                <col />
              </colgroup>
              <thead>
                <!-- 第一行 35px 空行：页签住在这儿（与字段表一致） -->
                <tr class="thead-spacer">
                  <th colspan="2"></th>
                </tr>
                <tr>
                  <th>{{ $t('tdet.item') }}</th>
                  <th>{{ $t('tdet.value') }}</th>
                </tr>
              </thead>
              <tbody>
                <tr>
                  <td class="basic-label">{{ $t('tf.tableName') }}</td>
                  <td><el-input v-model="form.name" :disabled="editMode" size="small" clearable /></td>
                </tr>
                <tr v-if="has(features,'supportsComment')">
                  <td class="basic-label">{{ $t('tf.tableComment') }}</td>
                  <td><el-input v-model="form.comment" size="small" clearable /></td>
                </tr>
                <!-- Doris 专属：数据模型 / 分桶列 / 分桶数 / 副本数 -->
                <tr v-if="isDoris">
                  <td class="basic-label">{{ $t('tf.dorisModel') }}</td>
                  <td>
                    <el-select v-model="form.dorisModel" size="small" :title="$t('tf.dorisModelTip')" @change="dorisModelTouched = true">
                      <el-option v-for="m in ['DUPLICATE KEY', 'UNIQUE KEY']" :key="m" :label="m" :value="m" />
                    </el-select>
                  </td>
                </tr>
                <tr v-if="isDoris">
                  <td class="basic-label">{{ $t('tf.distCol') }}</td>
                  <td>
                    <el-select v-model="form.dorisDistCol" multiple clearable filterable size="small" :placeholder="$t('tf.distColPh')">
                      <el-option v-for="k in dorisKeyCols" :key="k" :label="k" :value="k" />
                    </el-select>
                  </td>
                </tr>
                <tr v-if="isDoris">
                  <td class="basic-label">{{ $t('tf.buckets') }}</td>
                  <td><el-input-number v-model="form.dorisBuckets" :min="1" :max="1024" controls-position="right" size="small" /></td>
                </tr>
                <tr v-if="isDoris">
                  <td class="basic-label" :title="$t('tf.replicasTip')">{{ $t('tf.replicas') }}</td>
                  <td><el-input-number v-model="form.dorisReplication" :min="1" :max="32" controls-position="right" size="small" /></td>
                </tr>
                <tr v-if="has(features,'supportsTableOptions') && (features.engines || []).length">
                  <td class="basic-label">{{ $t('tf.engine') }}</td>
                  <td>
                    <el-select v-model="form.engine" size="small" clearable filterable>
                      <el-option v-for="e in features.engines" :key="e" :label="e" :value="e" />
                    </el-select>
                  </td>
                </tr>
                <tr v-if="has(features,'supportsTableOptions') && (features.charsets || []).length">
                  <td class="basic-label">{{ $t('tf.charset') }}</td>
                  <td>
                    <el-select v-model="form.charset" size="small" clearable filterable @change="onCharsetChange">
                      <el-option v-for="c in features.charsets" :key="c" :label="c" :value="c" />
                    </el-select>
                  </td>
                </tr>
                <tr v-if="has(features,'supportsTableOptions') && collations.length">
                  <td class="basic-label">{{ $t('tf.collation') }}</td>
                  <td>
                    <el-select v-model="form.collation" size="small" clearable filterable>
                      <el-option v-for="c in collations" :key="c" :label="c" :value="c" />
                    </el-select>
                  </td>
                </tr>
              </tbody>
            </table>
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
                <!-- 第一行是 35px 空行：页签与工具条就"住"在这一行（与「编辑表结构」同一套做法）。
                     表头标题行的 sticky top: 35px 正是按这一行算出来的。 -->
                <tr class="thead-spacer">
                  <th :colspan="headSpan"></th>
                </tr>
                <tr>
                  <!-- 列宽的规矩（别随手改，宽屏上的观感全靠它）：
                       · 表格宽 100% + 固定布局时，**多出来的宽度会按比例摊给每一列** ——
                         所以序号这种小列（40px）会被摊宽，看着比字段名还扎眼；
                       · 解法是让**长文本那一列不写宽度**：富余宽度只落到它头上，
                         其余列宽恒定。这里由「备注/说明」承担（没这一列时落到末列）。
                       · 序号收到 32px：它只放一位数。 -->
                  <th style="width:32px" :title="$t('tf.colOrder')">#</th>
                  <th style="width:120px">{{ $t('tf.colName') }}</th>
                  <th style="width:110px">{{ $t('tf.colType') }}</th>
                  <th style="width:64px">{{ $t('tf.colLength') }}</th>
                  <th style="width:64px">{{ $t('tf.colPrecision') }}</th>
                  <th class="c-center" style="width:56px">{{ $t('tf.colPrimary') }}</th>
                  <th class="c-center" style="width:56px">{{ $t('tf.colNullable') }}</th>
                  <th v-if="showOrderKey" class="c-center" style="width:70px" :title="$t('tf.orderKeyTip')">{{ $t('tf.orderKey') }}</th>
                  <th style="width:150px">{{ $t('tf.colDefault') }}</th>
                  <th v-if="showAutoIncrement" class="c-center" style="width:56px">{{ $t('tf.colAutoInc') }}</th>
                  <th v-if="showComment">{{ $t('udv.fComment') }}</th>
                  <th class="c-center" style="width:40px">{{ $t('tf.colOps') }}</th>
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

/// 该有默认值的字段，一律给上默认值（**所有类型**共用这一处判断）：
///   存储引擎：优先 InnoDB（MySQL 系），其次 MergeTree（ClickHouse），否则用后端的第一个；
///   字符集  ：优先 utf8mb4，其次 utf8，否则第一个；排序规则取该字符集的 _general_ci（有才设）。
/// 只在字段还是空的时候填 —— 用户改过的一律不碰（features 可能在重新取回时再次触发本 watch）。
/// 为什么不把这些默认值写进 `form` 的初值：可取的值来自后端（features.engines / charsets），
/// 而 features 是异步来的 —— 写死初值就等于赌"后端一定有 InnoDB"，赌错了会生成非法 DDL。
const pickDefault = (list, preferred) => {
  if (!Array.isArray(list) || !list.length) return ''
  for (const want of preferred) {
    const hit = list.find(x => String(x).toLowerCase() === want.toLowerCase())
    if (hit) return hit
  }
  return list[0]
}
watch(() => props.features, (f) => {
  if (!f) return
  if (!form.engine) form.engine = pickDefault(f.engines, ['InnoDB', 'MergeTree'])
  if (!form.charset) form.charset = pickDefault(f.charsets, ['utf8mb4', 'utf8', 'UTF8'])
  if (!form.collation && form.charset) {
    const list = (f.collations || {})[form.charset] || []
    form.collation = list.find(c => String(c).toLowerCase() === `${form.charset}_general_ci`.toLowerCase()) || ''
  }
}, { immediate: true, deep: false })

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
/* 注意：本文件有两段样式 —— 这一段是**早先的薄皮肤**，下面还有一段"与编辑表结构对齐"的
   覆盖层（同名选择器、写在更后面，所以真正生效的是后者）。保留前者是为了在覆盖层被
   误删时不至于完全没有样式；**要改外观请改覆盖层**，别在这儿改（改了看不到效果）。 */
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
/* （原"每字段一个 el-form 的网格"那套规则已随标记一起去掉：
     基本信息现在是 .field-table.basic-table 两列表，规则见上面的 basic-table 段。） */
.dc-del { color: var(--dc-text-dim); }
.dc-del:hover { color: var(--dc-danger); background: rgba(255,97,97,.1); }
.type-select { width: 100%; }
:deep(.el-table) { --el-table-header-bg-color: var(--dc-bg-code); }
/* 索引表表头/内容左对齐 */
.table-tabs :deep(.el-table th.el-table__cell > .cell),
.table-tabs :deep(.el-table td.el-table__cell > .cell) { text-align: left; }

/* ======================================================================
   ↓↓↓ 与「编辑表结构」(modules/data/TableDetailView.vue) 对齐的覆盖层 ↓↓↓

   新建表与编辑表结构在用户眼里是同一个界面，必须长得一样。上面那套是本组件
   早先的薄样式，这一段逐条照抄编辑页的对应规则（连"为什么必须这么写"的约束
   一起搬过来）。**改这里时请对照 TableDetailView.vue 的样式块同步改。**
   ====================================================================== */

/* 页签条 + 工具条 + 表格 = **同一张卡**：整块只画一层边框/圆角/卡片底，
   页签条相当于这张卡的"标题行"，表格与工具栏不再各自套边框。 */
.table-tabs {
  border: 1px solid var(--dc-border);
  border-radius: 8px;
  background: var(--dc-bg-card);
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
  overflow: hidden;
  /* 工具条要绝对定位到"页签这一行"，定位祖先必须抬到卡片上
     （Element 的 .el-tabs__content 自带 position: relative，会在它那层吃掉锚点） */
  position: relative;
}
.table-tabs :deep(.el-tabs__content) {
  flex: 1; padding: 0; position: static; overflow: visible;
  /* Element 会给内容区铺一层淡灰蓝底，露在表格四周像"表下压着个背景块" */
  background: transparent !important;
}
.table-tabs :deep(.el-tabs__header) {
  /* 页签要"住进表格的第一行"：把 Element 的表头压成 0 高（不占位），
     页签改为绝对定位，覆盖在表格首行（35px 空行）之上 */
  height: 0; margin: 0; padding: 0; border-bottom: none;
  background: transparent; position: static;
}
.table-tabs :deep(.el-tabs__nav-wrap) { height: 0; position: static; }
.table-tabs :deep(.el-tabs__nav) {
  position: absolute; top: 0; left: 12px; height: 35px;
  display: flex; align-items: center;
  background: transparent; border: none; padding: 0; z-index: 5;
}
.table-tabs :deep(.el-tabs__nav-wrap::after) { display: none; }  /* 去掉自带的下划双层线 */
.table-tabs :deep(.el-tabs__active-bar) { display: none; }       /* 激活项用主色文字，不要指示条 */
.table-tabs :deep(.el-tabs__item) {
  /* height / border-radius 必须 !important：MainView 的全局 .dc-tabs 规则同样命中
     这排页签（把 height 压成 34px、圆角压成 0 → 激活项滑块变方角） */
  height: 30px !important; line-height: 30px;
  padding: 0 14px !important; font-size: 13px;
  color: var(--dc-text-mid);
  border-radius: 6px !important;
  background: transparent !important;
  transition: background .15s ease, color .15s ease, box-shadow .15s ease;
}
.table-tabs :deep(.el-tabs__item:hover) { color: var(--dc-text-strong); }
.table-tabs :deep(.el-tabs__item.is-active) {
  color: var(--dc-primary) !important; font-weight: 600;
  background: var(--dc-bg-card) !important;
  box-shadow: 0 1px 2px rgba(16, 24, 40, .10);
}

/* 表格那两个页签**不留内边距**：表格直接贴到卡片内缘，左右两侧的"框线"就是
   卡片自身那条边框，下方由最后一行的下边框收口。 */
.tab-inner.col-tab,
.tab-inner.idx-tab { padding: 0; }
.tab-inner { overflow: hidden; }
/* 表格容器贴内容长（12 行的表原来会在下面拖出一大片带边框的空白）。
   横向**允许滚动**（编辑页那边是 hidden，因为它列宽总和恰好小于容器）：
   新建表的列更多（含排序键/自增/说明等），窗口窄时总宽会超出去 ——
   裁掉的就等于"看不见的列"，那比多一条滚动条严重得多。 */
.col-tab .field-table-wrap {
  flex: 0 1 auto; max-height: 100%;
  overflow-x: auto; overflow-y: auto;
  border: none; border-radius: 0; background: transparent; min-height: 0;
}

/* 工具条**上浮到页签这一行**（左边页签、右边按钮），不再单独占一行 */
.col-tab .card-actions,
.idx-tab .card-actions {
  position: absolute; top: 0; right: 13px; height: 35px;
  display: flex; align-items: center; justify-content: flex-end; gap: 8px;
  /* 左侧留给页签（实测页签框约 275px + 12px 缩进） */
  max-width: calc(100% - 286px);
  padding: 0; margin: 0; border-radius: 0; background: transparent;
  z-index: 4; flex-shrink: 0;
}
.col-tab .card-actions .move-tip {
  margin-right: auto; font-size: 12.5px; color: var(--dc-text-dim);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}

/* 索引页用的是 el-table（没有那行 35px 空行），自己补出这段距离，
   否则表头会被绝对定位的工具条直接压住 */
.idx-tab { padding-top: 35px; }

/* ---- 字段表格本体 ---- */
.field-table {
  /* **不能用 collapse**：collapsed 边框 + sticky 表头在 Chromium 上会留残影
     （表头下面几行的勾选框会"透过"表头显示）。separate + border-spacing: 0
     观感一致，但粘性表头的重绘正常。 */
  border-collapse: separate;
  border-spacing: 0;
  table-layout: fixed;
}
.field-table th {
  position: sticky; top: 0;
  /* 必须高于行内控件：Element 给 .el-checkbox__inner 设了 z-index: 1，
     同级时后出现的 tbody 会盖住表头（滚动时勾选框透到表头上） */
  z-index: 3;
  background: var(--dc-bg-soft); color: var(--dc-text-strong);
  font-weight: 600; font-size: 12px; letter-spacing: .02em; text-align: left;
  padding: 6px 9px; border-bottom: 1px solid var(--dc-border); white-space: nowrap;
}
/* 行高收到约 27px；行分隔线用 --dc-border 而不是最浅的 soft（soft 在部分屏幕
   上几乎看不见，用户反馈过"框线都没了"） */
.field-table td {
  padding: 2px 9px; background: var(--dc-bg-card);
  border-bottom: 1px solid var(--dc-border); vertical-align: middle;
}
/* 表头是**两行**（35px 空行 + 标题行），滚动时两行都要固定 */
.field-table tr.thead-spacer th {
  height: 35px; padding: 0; border-bottom: none; background: var(--dc-bg-card);
}
.field-table thead tr:last-child th { top: 35px; }
/* 列竖线逐格画（separate 模式各画各的，不会叠加变粗）；最后一列不画 */
.field-table thead tr:last-child th,
.field-table tbody td { border-right: 1px solid var(--dc-border); }
.field-table thead tr:last-child th:last-child,
.field-table tbody td:last-child { border-right: none; }
/* 序号列居中，并收紧内边距（列宽只有 40px，不收紧数字会被挤换行） */
.field-table thead tr:last-child th:first-child,
.field-table tbody td:first-child { text-align: center; }
.field-table tbody td:first-child { padding-left: 4px; padding-right: 4px; }

/* 行内控件统一压到 22px：否则它们会成为新的"行高天花板" */
.field-table :deep(.el-input__inner) { height: 22px; }
.field-table :deep(.el-input__wrapper) { min-height: 22px; }
.field-table :deep(.el-select__wrapper) { min-height: 22px; }
.field-table :deep(.el-button--small) { height: 22px; padding: 0 6px; }
.field-table :deep(.el-checkbox) { margin-right: 0; height: 22px; }
.field-table :deep(.el-checkbox),
.field-table :deep(.el-checkbox__inner),
.field-table :deep(.el-switch) { z-index: auto; }

/* 行底色统一画在 td 上（悬停 / 主键 / 排序键），靠顺序决定胜出；不做斑马纹 */
.field-table tbody tr:hover td { background: var(--dc-bg-hover); }
.field-table tbody tr.pk-row td { background: rgba(79, 140, 255, .05); }
.field-table tbody tr.ok-row td { background: rgba(103, 194, 58, .05); }
.field-table tbody tr.sel-row td { background: rgba(79, 140, 255, .16) !important; }
.field-table tbody tr.sel-row td:first-child { box-shadow: inset 2px 0 0 var(--dc-primary); }
.field-table tbody tr:hover { background: transparent; }   /* 旧的 tr 级底色让位给 td 级 */

/* ===== 基本信息：与字段表**同一张两列表**（编辑页就是这么做的） =====
   左列是「项目」（读作行标题，180px），右列是「值」；表头浅底、行高、网格线
   全部与字段表共用一套 —— 所以两个页签看起来是一张表切换了内容，而不是两种控件。 */
.tab-inner.basic-tab { padding: 0; }
/* 左列要盖掉字段表那条"首列居中"（那条是给 # 序号列用的，会连带把「项目」也居中），
   padding 同理：字段表给首列设了 4px，这里必须 !important 才能压回 9px（与表头对齐）。 */
.basic-table tbody td:first-child,
.basic-table thead tr:last-child th:first-child { text-align: left; }
.basic-table td.basic-label {
  font-size: 12.5px;
  color: var(--dc-text-mid);
  padding-left: 9px !important;
  white-space: nowrap;
}
.basic-table tbody td:not(:first-child) { padding-left: 9px !important; padding-right: 9px !important; }
/* 值列的控件撑满整格（el-select / el-input-number 默认是内容宽度） */
.basic-table :deep(.el-select),
.basic-table :deep(.el-input-number) { width: 100%; }
/* 字段表末列（操作）：表头与内容都居中 */
.field-table thead tr:last-child th:last-child { text-align: center; }
</style>
