<template>
  <div class="ols-wrap" :style="{ '--grid-row-h': (editorSettingsLive.gridRowHeight || 26) + 'px', fontSize: 'var(--dc-data-fs, 13px)' }">
    <!-- 圆角卡片：工具条 + 表格一个容器（边距与圆角与其他页面同一口径） -->
    <div class="ols-card">
      <div class="ols-toolbar">
        <!-- 数据源 / 数据库切换：与 SQL 编辑器顶栏同款交互 —— 默认自动带出打开清单时的
             连接与库，可自行切换，切换后整个清单重载 -->
        <el-select v-model="curConnId" size="small" class="ols-conn" filterable
                   popper-class="conn-opt-popper" @change="onConnSwitch">
          <!-- 连接按分组列出（分组名 = 左侧对象树的目录名），与数据传输同一口径 -->
          <el-option-group v-for="g in connGroups" :key="g.label" :label="g.label">
            <el-option v-for="c in g.options" :key="c.id" :label="c.name" :value="String(c.id)" />
          </el-option-group>
        </el-select>
        <!-- 库下拉：**没有「库」这一层的类型**（Oracle / DM）不画 —— 它就是连接里填的
             服务名，永远只有一条；这里显示的是模式（用户眼里的"库"就是这个）。
             请求仍然带完整串（`服务名.模式`），只是不再把服务名摆到界面上。 -->
        <span v-if="hideDbLevel" class="ols-db-text" :title="dbShown">{{ dbShown || $t('upv.phPickDb') }}</span>
        <el-select v-else v-model="curDb" size="small" class="ols-db" filterable :placeholder="$t('upv.phPickDb')" @change="load">
          <el-option v-for="d in dbOptions" :key="d" :label="d" :value="d" />
        </el-select>
        <el-input v-model="keyword" size="small" clearable :placeholder="$t('ols.search')"
                  class="ols-search" :prefix-icon="Search" />
        <span class="ols-count">{{ $t('ols.count', { n: filtered.length }) }}</span>
        <span class="ols-flex" />
        <el-button size="small" :icon="Refresh" text @click="relayout">{{ $t('common.refresh') }}</el-button>
      </div>
      <div ref="bodyRef" class="ols-body" v-loading="loading">
        <!-- 与查询结果/表数据同一套自绘网格（.data-table）：表头小图标着色（只有图标
             着色，文字保持正常色）、行号列、斑马纹、行 hover。
             列宽机制与结果网格一致：先用 auto 布局按内容测出每列宽 → 写进 colgroup 切
             fixed 布局；列缘可拖拽，双击列缘回自适应；内容不足容器宽时右侧留白 -->
        <table v-if="filtered.length" ref="tblRef" class="data-table ols-grid"
               :style="gridStyle" @dblclick.self="relayout">
          <colgroup v-if="colW.length">
            <col style="width:46px" />
            <col v-for="(w, i) in colW" :key="i" :style="{ width: w + 'px' }" />
            <col style="width:230px" />
          </colgroup>
          <thead>
            <tr>
              <th class="leading-th row-num-th">#</th>
              <th v-for="(c, i) in cols" :key="c.prop" :style="thStyle(c)"
                  @mousedown="onThMouseDown(i, $event)" @dblclick.stop="onThDblClick(i)">
                <span class="th-line1" :class="c.align === 'left' ? 'is-left' : ''">
                  <span class="th-type-ic" :class="typeClassOf(c)">
                    <el-icon><component :is="typeIconOf(c)" /></el-icon>
                  </span>
                  <span class="th-label">{{ $t(c.label) }}</span>
                </span>
              </th>
              <th class="ols-act-th">{{ $t('ols.actions') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(r, i) in filtered" :key="i" @dblclick="rowActivate(r)">
              <td class="leading-td row-num">{{ i + 1 }}</td>
              <td v-for="c in cols" :key="c.prop"
                  :class="[c.prop === 'name' ? 'ols-name-td' : '', c.align === 'left' ? 'ols-td-left' : 'ols-td-center']"
                  :title="cellText(r, c)">
                <span class="ols-ellipsis">{{ cellText(r, c) }}</span>
              </td>
              <td class="ols-act-td">
                <div class="ols-act-row">
                  <el-button v-for="a in actions" :key="a.action" size="small" link
                             :type="a.danger ? 'danger' : 'primary'"
                             @click.stop="$emit('act', { action: a.action, row: r })">{{ $t(a.label) }}</el-button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
        <div v-else-if="!loading" class="ft-empty">{{ $t('ols.empty') }}</div>
      </div>
    </div>
  </div>
</template>

<script setup>
// 对象清单页签：双击树上的类别节点（表/视图/存储过程/函数/触发器/事件/脚本）打开。
// 一张表格列出该类别下的全部对象 + 末尾操作栏；动作通过 act 事件交回 MainView 处理
// —— 那边复用右键菜单同款的打开逻辑（数据查看/结构/DDL/详情/编辑/删除），
// 这里只负责「清单展示 + 触发」，不重复实现任何业务。
// 网格复用查询结果/表数据那套自绘 .data-table 的结构与样式；列宽与结果网格同一机制：
// 按内容测量初始宽（自适应）→ colgroup 固定 → 列缘拖拽 / 双击列缘重新自适应。
// 列随**数据源类型**走：各家有什么就展示什么，没有的列不摆，不摆一排空列。
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { ElMessage } from 'element-plus'
import { Search, Refresh, Document, Calendar, Histogram } from '@element-plus/icons-vue'
import { t } from '../../utils/i18n'
import { editorSettingsLive } from '../../utils/settings'
import { formatDbValue } from '../../utils/cellValue'
import { listTables, listProcedures, listTriggers, listEvents, getTableCounts, listDatabases } from '../../api'
import { hideDbLevelOf, dbDisplayOf } from '../../types'
import { groupConnections } from '../../utils/connGroups'

const props = defineProps({
  conn: { type: Object, default: null },
  cat: { type: String, required: true },
  database: { type: String, default: '' },
  connId: { type: String, default: '' },
  reloadTick: { type: Number, default: 0 },
  connections: { type: Array, default: () => [] },   // 全部连接（数据源下拉）
  dbsByConn: { type: Object, default: () => ({}) }   // 连接 → 已加载的库清单
})
const emit = defineEmits(['act'])

// 数据源/库的**页内当前值**：初始自动带出（打开清单时的连接与库），可切换重载
const curConnId = ref(props.connId)
const curDb = ref(props.database)
// 页内自取的库清单：优先用 MainView 已加载的（dbsByConn），没有才自己拉
const localDbs = ref({})
const dbOptions = computed(() => props.dbsByConn?.[curConnId.value] || localDbs.value[curConnId.value] || [])
const connName = computed(() => (props.connections.find(c => String(c.id) === curConnId.value) || {}).name || '')
/** 连接下拉按分组列出（分组名 = 左侧对象树的目录名，见 utils/connGroups） */
const connGroups = computed(() => groupConnections(props.connections))
const onConnSwitch = async (cid) => {
  // 没有「库」这一层的类型（Oracle / DM）：库值固定取该连接自身的服务名 ——
  // 下拉不画了，但请求必须带（否则列出来的不是这个连接的表）
  const c = props.connections.find(x => String(x.id) === String(cid))
  curDb.value = hideDbLevelOf(c?.type) ? String(c?.database || '') : ''
  if (!(props.dbsByConn?.[cid] || localDbs.value[cid])) {
    try {
      const list = await listDatabases(cid)
      localDbs.value[cid] = Array.isArray(list) ? list : []
    } catch { localDbs.value[cid] = [] }
  }
  load()
}

const rows = ref([])
const loading = ref(false)
const keyword = ref('')
const loadCalls = ref(0)
const tblRef = ref(null)
const bodyRef = ref(null)
const colW = ref([])          // 每个数据列的像素宽（空 = 还没测，走 auto 布局）
const containerW = ref(0)     // 表格可视容器宽（fill 列据此补白）
let resizeObs = null

// 时间列统一显示：'2026-10-03T16:06:23' → '2026-10-03 16:06:23'（T 是 ISO 格式的噪音）
const TIME_PROPS = new Set(['createTime', 'created', 'lastAltered', 'lastExecuted', 'executeAt', 'ends', 'lastAnalyzed'])
const NUM_PROPS = new Set(['rows'])
const cellText = (row, col) => {
  const v = row?.[col.prop]
  if (v == null) return ''
  // ISO 时间的 'T' → 空格，但**必须先确认它长得像「日期T时间」**：
  // 原来的 `String(v).replace('T', ' ')` 会把任何值的第一个 T 吃掉 ——
  // ClickHouse 的引擎 "MergeTree" 在界面上就显示成 "Merge ree"（真机踩过）。
  // 复用查询网格同一份判定（utils/cellValue 的 formatDbValue）。
  return formatDbValue(String(v))
}

// 列头小图标：时间列 → 日历、数字列 → 柱状图、其余 → 文本（着色只给图标，文字保持正常色）
const typeIconOf = (col) => TIME_PROPS.has(col.prop) ? Calendar : NUM_PROPS.has(col.prop) ? Histogram : Document
const typeClassOf = (col) => TIME_PROPS.has(col.prop) ? 'th-t-date' : NUM_PROPS.has(col.prop) ? 'th-t-num' : 'th-t-text'
// auto 测量阶段给个别列保底宽（避免空列头被压成一根线）；fixed 阶段走 colgroup
const thStyle = (c) => (colW.value.length || !c.min ? null : { minWidth: c.min + 'px' })

// tables 的信息列按数据源类型出：没有的列不摆（类型取**页内当前**连接）
const connType = computed(() => {
  const c = props.connections.find(x => String(x.id) === curConnId.value) || props.conn
  return String(c?.type || '').toUpperCase()
})
// 「库」这一层要不要画：Oracle / DM 的库就是连接里填的服务名，永远只有一条（见 types）
const hideDbLevel = computed(() => hideDbLevelOf(connType.value))
// 这类类型在库的位置上显示**模式**（`FREE.SYS` → `SYS`），服务名不出现在界面上
const dbShown = computed(() => dbDisplayOf(connType.value, curDb.value))
const NAME_COL = { prop: 'name', label: 'ols.name', align: 'left' }
const TABLE_COL_SETS = {
  MYSQL: [
    NAME_COL,
    { prop: 'rows', label: 'ols.rows' },
    { prop: 'engine', label: 'ols.engine' },
    { prop: 'charset', label: 'ols.charset', min: 140 },
    { prop: 'createTime', label: 'ols.createTime' },
    { prop: 'comment', label: 'ols.comment', align: 'left', min: 200 }
  ],
  MARIADB: 'MYSQL',
  // Doris **不能**照抄 MySQL 的列集：它的 information_schema.tables 里
  // `ENGINE` 对 OLAP 表一律是占位值 `Doris`（真模型 DUPLICATE/UNIQUE/AGGREGATE KEY
  // 只在 `SHOW CREATE TABLE` 里，逐表查代价太大），也没有「字符集」这个概念
  // （`TABLE_COLLATION` 回的是 `utf-8` 这个占位）。给不出有意义的值的列不摆。
  DORIS: [
    NAME_COL,
    { prop: 'rows', label: 'ols.rows' },
    { prop: 'createTime', label: 'ols.createTime' },
    { prop: 'comment', label: 'ols.comment', align: 'left', min: 200 }
  ],
  CLICKHOUSE: [
    NAME_COL,
    { prop: 'rows', label: 'ols.rows' },
    { prop: 'engine', label: 'ols.engine' },
    // 排序键 / 分区键是「建表时的表达式」，内容短且与数据列同一性质 —— 居中（与其它短列一致）
    { prop: 'sortingKey', label: 'ols.sortingKey', min: 150 },
    { prop: 'partitionKey', label: 'ols.partitionKey', min: 130 },
    // 没有「创建时间」列：CH 的 system.tables 没有建表时间（只有 metadata_modification_time，
    // 那是元数据修改时间，不是同一个东西），摆一列恒空的不如不摆（同 SQLite 无注释的处理）
    { prop: 'comment', label: 'ols.comment', align: 'left', min: 160 }
  ],
  POSTGRESQL: [
    NAME_COL,
    { prop: 'rows', label: 'ols.rows' },
    { prop: 'owner', label: 'ols.owner' },
    { prop: 'size', label: 'ols.size' },
    { prop: 'comment', label: 'ols.comment', align: 'left', min: 220 }
  ],
  KINGBASE: 'POSTGRESQL',
  ORACLE: [
    NAME_COL,
    { prop: 'rows', label: 'ols.rows' },
    { prop: 'tablespace', label: 'ols.tablespace' },
    { prop: 'lastAnalyzed', label: 'ols.lastAnalyzed' },
    { prop: 'comment', label: 'ols.comment', align: 'left', min: 200 }
  ],
  DM: 'ORACLE',
  SQLSERVER: [
    NAME_COL,
    { prop: 'rows', label: 'ols.rows' },
    { prop: 'createTime', label: 'ols.createTime' },
    { prop: 'comment', label: 'ols.comment', align: 'left', min: 240 }
  ],
  // SQLite / Derby：**没有「表注释」这个概念**（后端 table_options 也如实不提供）——
  // 只摆名称列；摆一列恒空的注释等于让用户以为「注释没取到」
  SQLITE: [NAME_COL],
  DERBY: [NAME_COL],
  // H2 / DB2：information_schema / syscat 的 remarks 就是表注释，能取到，正常摆
  H2: [NAME_COL, { prop: 'comment', label: 'ols.comment', align: 'left', min: 300 }],
  DB2: [NAME_COL, { prop: 'comment', label: 'ols.comment', align: 'left', min: 300 }]
}

// 其余类别的列 —— **按数据源类型给**（后端各家方言的清单 SQL 回什么键，这里就摆什么列）。
// 恒空列比少一列更糟：界面上一个空列，用户分不清是「没取到」还是「本来没有」
//（同 tables 的处理，见 TABLE_COL_SETS 的注释）。
const COMMENT_COL = { prop: 'comment', label: 'ols.comment', align: 'left', min: 320 }
// 没有「对象注释」这个概念的类型：SQLite / Derby（后端 table_options 也如实不给）
const NO_COMMENT_TYPES = new Set(['SQLITE', 'DERBY'])
// 例程：created / last_altered / sql_data_access / security_type 这 4 个属性**只有 MySQL 系**
// 的 information_schema.routines 提供；PG / SQL Server / Oracle / DB2 / Derby / H2 的清单 SQL
// 只回 name + routineType，摆上去就是 4 个恒空列
const PROC_COLS_FULL = [
  NAME_COL,
  { prop: 'dataAccess', label: 'ols.dataAccess' },
  { prop: 'securityType', label: 'ols.security' },
  { prop: 'created', label: 'ols.createTime' },
  { prop: 'lastAltered', label: 'ols.lastAltered' }
]
// 触发器：MySQL 系的 information_schema.triggers 带 timing / event；
// SQLite / PG / SQL Server / Oracle / DM / DB2 只回 name + table（各自的系统表里没有另两项）；
// Derby 的 sys.systriggers 连所属表都没有，只有名字（见 dialect.rs 的 derby 分支注释）
const TRIG_COLS_FULL = [
  NAME_COL,
  { prop: 'table', label: 'ols.table', min: 160, align: 'left' },
  { prop: 'timing', label: 'ols.timing' },
  { prop: 'event', label: 'ols.event' }
]
const TRIG_COLS_TABLE = [NAME_COL, { prop: 'table', label: 'ols.table', min: 160, align: 'left' }]

const CAT_COLS = {
  views: [NAME_COL, COMMENT_COL],
  procs: PROC_COLS_FULL,
  functions: PROC_COLS_FULL,
  triggers: TRIG_COLS_FULL,
  events: [
    NAME_COL,
    { prop: 'status', label: 'ols.status' },
    { prop: 'interval', label: 'ols.interval' },
    { prop: 'lastExecuted', label: 'ols.lastExecuted' },
    { prop: 'executeAt', label: 'ols.nextExec' },
    { prop: 'ends', label: 'ols.ends' }
  ],
  scripts: [NAME_COL]
}
const cols = computed(() => {
  const t = connType.value
  const mysqlLike = t === 'MYSQL' || t === 'MARIADB'
  if (props.cat === 'tables') {
    // 同构类型用别名指向（MARIADB→MYSQL 等），解到数组为止
    let set = TABLE_COL_SETS[t]
    while (typeof set === 'string') set = TABLE_COL_SETS[set]
    if (Array.isArray(set)) return set
    // 表里没登记的类型（新类型/未知类型）：**只给名称**——不知道它能取到什么，
    // 摆上去的每一列都可能是恒空列（各家的列在 TABLE_COL_SETS 里显式登记）
    return [NAME_COL]
  }
  if (props.cat === 'views') return NO_COMMENT_TYPES.has(t) ? [NAME_COL] : [NAME_COL, COMMENT_COL]
  if (props.cat === 'procs' || props.cat === 'functions') return mysqlLike ? PROC_COLS_FULL : [NAME_COL]
  if (props.cat === 'triggers') {
    if (t === 'DERBY') return [NAME_COL]
    return mysqlLike ? TRIG_COLS_FULL : TRIG_COLS_TABLE
  }
  return CAT_COLS[props.cat] || CAT_COLS.tables
})

// 操作列按钮（顺序即展示顺序）。删除只给表（危险，MainView 侧有闸门 + 二次确认）
const ACTIONS = {
  tables: [
    { action: 'data', label: 'ols.data' },
    { action: 'structure', label: 'ols.structure' },
    { action: 'ddl', label: 'ols.ddl' },
    { action: 'drop', label: 'common.delete', danger: true }
  ],
  views: [
    { action: 'data', label: 'ols.data' },
    { action: 'view', label: 'ols.view' },
    { action: 'edit', label: 'ols.edit' },
    { action: 'drop', label: 'common.delete', danger: true }
  ],
  // 每个类别都给「删除」：与树右键同一套 DROP 生成与收尾（见 MainView 的 doDropObject）。
  // 少这一个按钮的后果是：树上右键能删、清单页里找不到入口 —— 同一件事两个地方两套答案。
  procs: [
    { action: 'view', label: 'ols.view' },
    { action: 'edit', label: 'ols.edit' },
    { action: 'drop', label: 'common.delete', danger: true }
  ],
  functions: [
    { action: 'view', label: 'ols.view' },
    { action: 'edit', label: 'ols.edit' },
    { action: 'drop', label: 'common.delete', danger: true }
  ],
  triggers: [
    { action: 'view', label: 'ols.view' },
    { action: 'edit', label: 'ols.edit' },
    { action: 'drop', label: 'common.delete', danger: true }
  ],
  events: [
    { action: 'view', label: 'ols.view' },
    { action: 'edit', label: 'ols.edit' },
    { action: 'drop', label: 'common.delete', danger: true }
  ],
  // 脚本存 localStorage，「删除」删的是本地那条记录（与树右键「删除脚本」同一函数）
  scripts: [
    { action: 'open', label: 'ols.open' },
    { action: 'drop', label: 'common.delete', danger: true }
  ]
}
const actions = computed(() => ACTIONS[props.cat] || [])

const filtered = computed(() => {
  const k = keyword.value.trim().toLowerCase()
  if (!k) return rows.value
  return rows.value.filter(r =>
    String(r.name || '').toLowerCase().includes(k) ||
    String(r.table || '').toLowerCase().includes(k) ||
    String(r.comment || '').toLowerCase().includes(k))
})

// 行双击 = 该类别最常用的动作（与树双击一致：表/视图开数据，其他看详情，脚本直接打开）
// act 带上**页内当前**连接/库 —— 用户可能已在顶部下拉里切换
const rowActivate = (row) => {
  const action = props.cat === 'tables' || props.cat === 'views' ? 'data'
    : props.cat === 'scripts' ? 'open' : 'view'
  emitAct(action, row)
}
const emitAct = (action, row) => emit('act', {
  action, row,
  connId: curConnId.value, database: curDb.value, cat: props.cat
})

// ===== 列宽：auto 测量 → colgroup fixed → 列缘拖拽 / 双击自适应 =====
const measure = async () => {
  await nextTick()
  const tbl = tblRef.value
  if (!tbl || !filtered.value.length) return
  const head = [...tbl.querySelectorAll('thead th')].slice(1, cols.value.length + 1)
  const widths = head.map((th, i) => {
    let w = th.scrollWidth
    for (const tr of tbl.querySelectorAll('tbody tr')) {
      const td = tr.children[i + 1]
      if (td) w = Math.max(w, td.scrollWidth)
    }
    return Math.ceil(w) + 16
  })
  colW.value = widths
}
const relayout = () => { colW.value = []; measure() }
const totalW = computed(() => 46 + colW.value.reduce((a, b) => a + b, 0) + 230)
// **不铺满**：表格宽度 = 按内容测出的总宽，列宽自适应、总宽不足时右侧留白；
// 内容超宽时保持内容宽横向滚动，列不被压缩
const gridStyle = computed(() => {
  if (!colW.value.length) return { width: 'max-content' }
  return { width: totalW.value + 'px' }
})

// 列缘拖拽：按下位置在 th 右缘 6px 内才生效（与结果网格同款交互）
const onThMouseDown = (i, e) => {
  if (!colW.value.length) return
  const th = e.currentTarget
  const rect = th.getBoundingClientRect()
  if (rect.width - (e.clientX - rect.left) > 6) return
  e.preventDefault()
  const startX = e.clientX
  const startW = colW.value[i]
  const move = (ev) => { colW.value[i] = Math.max(40, startW + ev.clientX - startX) }
  const up = () => {
    window.removeEventListener('mousemove', move)
    window.removeEventListener('mouseup', up)
  }
  window.addEventListener('mousemove', move)
  window.addEventListener('mouseup', up)
}
// 双击列缘：回自适应（重测）
const onThDblClick = (i, e) => {
  const th = e.currentTarget
  const rect = th.getBoundingClientRect()
  if (rect.width - (e.clientX - rect.left) <= 6) relayout()
}

// 读取某连接某库下保存的脚本（localStorage，与 MainView 的 getDbScripts 同一存储约定）
const localScripts = (connId, db) => {
  try {
    const raw = localStorage.getItem(`dc_scripts:${connId}:${db}`)
    return raw ? JSON.parse(raw) : []
  } catch { return [] }
}

async function load() {
  // 用**页内当前**的连接/库（顶部下拉可切换，不再固定于打开清单时的那组）
  const id = curConnId.value
  const db = curDb.value
  if (!id) return
  loading.value = true
  colW.value = []
  try {
    if (props.cat === 'tables' || props.cat === 'views') {
      const want = props.cat === 'tables' ? 'TABLE' : 'VIEW'
      const all = await listTables(id, db)
      rows.value = (Array.isArray(all) ? all : []).filter(r => String(r.type || '').toUpperCase() === want)
      // 行数回填：information_schema 的 TABLE_ROWS 是估算值（Doris/CH 常为 0）——
      // 与树同一策略，真实 COUNT(*) 批量补（失败静默，估算值仍在）
      if (props.cat === 'tables') {
        const names = rows.value.map(r => r.name).slice(0, 200)
        if (names.length) {
          getTableCounts(id, db, names).then((m) => {
            if (!m) return
            for (const r of rows.value) if (m[r.name] != null) r.rows = m[r.name]
          }).catch(() => {})
        }
      }
    } else if (props.cat === 'procs' || props.cat === 'functions') {
      const all = await listProcedures(id, db)
      const list = Array.isArray(all) ? all : []
      // ClickHouse 的 routines 就是「函数」（树上 procs 类别显示为函数）：
      // 两个类别都显示全量，不再按 routineType 拆（拆完一边恒空）
      if (connType.value === 'CLICKHOUSE') rows.value = list
      else {
        const want = props.cat === 'procs' ? 'PROCEDURE' : 'FUNCTION'
        rows.value = list.filter(r => String(r.routineType || '').toUpperCase() === want)
      }
    } else if (props.cat === 'triggers') {
      const list = await listTriggers(id, db)
      rows.value = Array.isArray(list) ? list : []
    } else if (props.cat === 'events') {
      const list = await listEvents(id, db)
      // 后端列名是 SQL 别名原样（interval_value / interval_field），归一成渲染要的形状
      rows.value = (Array.isArray(list) ? list : []).map(r => ({ ...r,
        interval: [r.interval_value ?? r.intervalValue, r.interval_field ?? r.intervalField]
          .filter(x => x != null && x !== '').join(' ') }))
    } else if (props.cat === 'scripts') {
      rows.value = (localScripts(id, db) || []).map(s => ({ name: s.name, id: s.id }))
    }
  } catch (e) {
    ElMessage.error(t('ols.loadFailed', { detail: e?.response?.data?.message || e?.message || String(e) }))
  } finally {
    loading.value = false
  }
  // 数据到位后按内容测列宽（行数回填是异步的，回填后这些列会 truncate —— 可接受，
  // 双击列缘随时回自适应）
  measure()
}

// 数据加载由 immediate watch 驱动：setup 后立即拉一次（不依赖 mounted 生命周期时序），
// 之后 connId / database / cat / reloadTick 任一变化都重拉（并同步页内下拉的当前值）
watch(() => [props.connId, props.database, props.cat], () => {
  curConnId.value = props.connId
  curDb.value = props.database
  load()
}, { immediate: true })
watch(() => props.reloadTick, () => load())
watch(keyword, () => { colW.value = []; measure() })
onMounted(() => {
  if (bodyRef.value && typeof ResizeObserver !== 'undefined') {
    resizeObs = new ResizeObserver((es) => { containerW.value = Math.ceil(es[0].contentRect.width) })
    resizeObs.observe(bodyRef.value)
  }
})
onBeforeUnmount(() => resizeObs?.disconnect())
</script>

<style scoped>
/* 上下左右边距与「表数据页」同一口径 —— 直接对齐 TableDataView 根容器
   .table-data-view 的 padding: 5px / gap: 5px（不是高级搜索面板的 10px 12px） */
.ols-wrap { display: flex; flex-direction: column; height: 100%; padding: 5px; gap: 5px; box-sizing: border-box; }
.ols-card {
  flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 5px;
  background: transparent; border: none; border-radius: 0; overflow: visible;
  padding: 0; box-sizing: border-box;
}
/* 搜索栏：与表数据页工具条同款圆角小卡（同 padding / 同阴影） */
.ols-toolbar {
  display: flex; align-items: center; gap: 8px;
  padding: 8px 12px; background: var(--dc-bg-soft, #f5f7fa);
  border: 1px solid var(--dc-border, #dcdfe6); border-radius: 8px;
  box-shadow: 0 1px 3px rgba(16, 24, 40, .06), 0 1px 2px rgba(16, 24, 40, .04);
}
.ols-search { width: 200px; }
.ols-conn { width: 170px; }
.ols-db { width: 130px; }
/* 没有「库」这一层的类型（Oracle / DM）：这里显示的是模式名，做成与下拉同位同高的纯文字，
   读起来是"当前上下文"，而不是一个还能点开的下拉（点了没东西可选的控件比没有更糟） */
.ols-db-text {
  display: inline-flex; align-items: center; height: 24px; max-width: 160px;
  padding: 0 10px; border-radius: 6px;
  background: var(--dc-bg-soft, #f4f6f9); color: var(--dc-text-dim, #606266);
  font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.ols-count { color: var(--dc-text-dim, #909399); font-size: 12px; }
.ols-flex { flex: 1; }
/* 表格区：圆角边框（表数据页网格同款），行少时空白也在框内；
   空白区补白底（--dc-bg-card），不再露出页签底色 */
.ols-body {
  flex: 1; min-height: 0; overflow: auto;
  border: 1px solid var(--dc-border, #dcdfe6); border-radius: var(--dc-radius, 10px);
  background: var(--dc-bg-card, #fff);
}
.ft-empty { padding: 40px 0; text-align: center; color: var(--dc-text-dim, #909399); font-size: 13px; }

/* ===== 自绘网格：与查询结果/表数据同一套（.data-table 类名对齐）===== */
.data-table { border-collapse: collapse; font-size: inherit; }
.data-table thead { position: sticky; top: 0; z-index: 2; }
.data-table th {
  background: var(--dc-bg-table-head, #f5f7fa); padding: 5px 10px; height: calc(var(--grid-row-h, 26px) + 6px);
  border: 1px solid var(--dc-border, #dcdfe6); color: var(--dc-text-strong, #303133);
  font-weight: normal; text-align: center; white-space: nowrap;
  position: relative; user-select: none;
  /* 表头/数据的 2px 压边线：与查询结果网格同一笔（inset 阴影才能跟着 sticky 表头走） */
  box-shadow: inset 0 -2px 0 var(--dc-table-head-line, #d8dee9);
}
/* 列头对齐跟数据一致：文本列（名称/注释…）左对齐，短值列居中 —— 表头不比数据更"居中" */
.data-table th .th-line1 { display: flex; width: 100%; justify-content: center; }
.data-table th .th-line1.is-left { justify-content: flex-start; }
/* 列缘拖拽热区：hover 时右侧 3px 亮条 + col-resize 光标（与结果网格同款交互暗示） */
.data-table th::after {
  content: ''; position: absolute; top: 4px; right: 0; width: 6px; height: calc(100% - 8px);
  cursor: col-resize;
}
.data-table th:hover::after { border-right: 3px solid var(--dc-primary, var(--el-color-primary)); opacity: .5; }
.data-table th .th-line1 { display: flex; align-items: center; justify-content: center; min-width: 0; }
.data-table th .th-label { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
/* position/top：图标单独下移 1px（四张网格统一口径） */
.data-table th .th-type-ic { display: inline-flex; align-items: center; flex: 0 0 auto; margin-right: 5px; position: relative; top: 1px; }
.data-table th .th-type-ic .el-icon { font-size: 12px; }
.data-table td {
  padding: 0 10px; height: var(--grid-row-h, 26px); line-height: var(--grid-row-h, 26px);
  border: 1px solid var(--dc-border, #dcdfe6); color: var(--dc-text, #606266);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis; text-align: center;
}
.data-table td.ols-td-left { text-align: left; }
/* 斑马纹 + 行悬停：与结果网格同观感 */
.data-table tbody tr:nth-child(even) td { background: var(--dc-bg-soft, #fafafa); }
.data-table tbody tr:hover td { background: var(--dc-bg-hover, #f0f4ff); }
/* 行号列 */
.data-table th.leading-th { width: 46px; color: var(--dc-text-strong, #606266); }
.data-table td.leading-td {
  text-align: center; color: var(--dc-text-dim, #909399);
  background: var(--dc-bg-table-head, #f5f7fa); font-size: 12px;
}
/* 名称列：可点击的暗示 —— 手型 + 悬停变主色（双击开数据/详情） */
.data-table td.ols-name-td { cursor: pointer; color: var(--dc-text-strong, #303133); }
.data-table td.ols-name-td:hover {
  color: var(--dc-primary, var(--el-color-primary));
  text-decoration: underline; background: var(--dc-bg-hover, #f0f4ff);
}
/* 补白列：吸收容器剩余宽度（宽度由 colgroup 显式给出，不走浏览器均分）；
   内容超宽时横向滚动，各数据列不被压缩 */
.data-table th.fill-col, .data-table td.fill-col {
  padding: 0; border: none; background: transparent !important;
}
/* 操作列：按钮一排；颜色统一 Element 主色（蓝），删除用 danger 红。
   **固定在最右**：列多到横向滚动时操作按钮不能跟着滚走（否则要滚到最右才能点）。
   sticky 的单元格会盖在滚动内容上，所以必须给**不透明**底色 —— 透出去就是"按钮上叠着数据"。
   斑马纹 / 行悬停的底色优先级更高（.data-table tbody tr:nth-child(even) td 等），会自动盖住这条。 */
.ols-act-th { width: 230px; }
.data-table th.ols-act-th,
.data-table td.ols-act-td {
  position: sticky; right: 0; z-index: 1;
  background: var(--dc-bg-card, #fff);
  /* border-collapse: collapse 下 sticky 单元格的**左/下边框会跟着表格一起被折叠掉**，
     滚动时这两条线就没了 —— 用 inset 阴影补齐（位置与边框重合，画重了也看不出来）。
     表头那条下边线必须是 2px 压边线的同色同宽（--dc-table-head-line），否则操作列
     与左边各列的"压边"对不上（用户反馈：操作列的下边线不一样）。 */
  box-shadow: inset 1px 0 0 var(--dc-border, #dcdfe6), inset 0 -1px 0 var(--dc-border, #dcdfe6);
}
.data-table th.ols-act-th {
  background: var(--dc-bg-table-head, #f5f7fa);
  z-index: 3; /* 表头本身是 sticky top（z-index 2），操作列表头同时在右上角，要压住它 */
  box-shadow: inset 1px 0 0 var(--dc-border, #dcdfe6), inset 0 -2px 0 var(--dc-table-head-line, #d8dee9);
}
.data-table td.ols-act-td { text-align: center; }
.ols-act-row { display: flex; align-items: center; justify-content: center; gap: 2px; white-space: nowrap; }
.ols-act-row .el-button { padding: 4px 6px; }
.ols-ellipsis { display: block; overflow: hidden; text-overflow: ellipsis; }
</style>
