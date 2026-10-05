import { ref, computed, watch } from 'vue'
import { t } from '../../utils/i18n'
import { listConnections, listDatabases, listSchemas, listTables } from '../../api'
import { byType, schemaLevelOf, isNoSql } from '../../types'

/** 系统库/模式：默认选中时优先跳过，避免一打开就落在 information_schema 这类系统库上 */
const SYSTEM_NAMES = /^(information_schema|performance_schema|mysql|sys|system|__internal_schema|pg_catalog|pg_toast|pg_temp_\d*|default|derby|h2|postgres|template0|template1|master|tempdb|msdb|model)$/i

/** 错误信息在界面上最多显示的长度（其余用 title 悬浮查看全文，避免撑爆布局） */
export const ERROR_TEXT_LIMIT = 160

export const truncateError = (msg) => {
  const s = String(msg || '')
  return s.length > ERROR_TEXT_LIMIT ? s.slice(0, ERROR_TEXT_LIMIT) + '…' : s
}

/**
 * AI 面板的「连接 + 数据库 + 模式(schema)」上下文选择。
 *
 * <p>交互约定（避免"一打开就报错"）：
 *  1. **不自动选中连接**——除非父组件已指定（如在对象树选中连接后打开面板）。
 *     若自动落到第一个连接，而它恰好网络不通，用户一打开面板就会看到一大段报错；
 *  2. **不自动加载库列表**——只有确定了连接（父组件指定或用户主动选择）才去请求，
 *     避免无谓的失败请求；
 *  3. 连接/库/模式三级联动，切换上游会清空并重载下游；
 *  4. SQL Server / PostgreSQL / Kingbase 等 schema 层级类型额外提供模式选择，
 *     默认选中该类型的 defaultSchema（dbo / public），并以 `库.模式` 复合串传给后端
 *     （与对象树、主查询界面使用同一套约定，后端方言已支持解析）。
 */
export function useAiContext(props) {
  const conns = ref([])
  const connId = ref('')
  const dbs = ref([])
  const database = ref('')
  const schemas = ref([])
  const schema = ref('')
  const loadingDbs = ref(false)
  const loadingSchemas = ref(false)
  const dbError = ref('')
  /** 当前库的表清单（供「表名」下拉使用，按需加载） */
  const tables = ref([])
  const loadingTables = ref(false)
  /** 表清单加载失败原因（空表示成功；用于明确告知用户，而非静默显示「无数据」） */
  const tableError = ref('')

  /**
   * 请求代次：每次加载自增。
   * 异步返回后若代次已变，说明期间用户切换了上游，本次结果必须丢弃，
   * 否则旧连接的报错会「串台」显示在用户当前选中的连接上。
   */
  let loadSeq = 0
  /** 表清单单独一个代次，避免与库/模式加载互相干扰 */
  let tableSeq = 0
  /**
   * 是否已请求过表清单（按需加载的开关）。
   *
   * 注意：**不能用 `tables.length` 判断**——
   * 若首次请求发生在库/模式尚未就绪时（会直接返回空），
   * 用长度判断会导致后续库/模式就绪后不再重试，表下拉将永远为空。
   */
  let tablesRequested = false
  /** 表加载的安全阀：极端情况（请求长时间无响应）下兜底复位 loading，避免下拉永久卡在「加载中」 */
  let tableGuard = null

  /**
   * 连接按「目录」分组，供数据源下拉多级显示。
   * 目录取连接配置的 group（与左侧对象树的分组口径一致），
   * 未设置目录的连接归入「未分组」。
   */
  const connGroups = computed(() => {
    const map = new Map()
    for (const c of conns.value) {
      const key = (c.group || c.env || '').trim() || t('aictx.ungrouped')
      if (!map.has(key)) map.set(key, [])
      map.get(key).push(c)
    }
    return [...map.entries()].map(([label, options]) => ({ label, options }))
  })

  /** 当前生效的连接：优先用户在本面板选中的，其次父组件传入的 */
  const activeConn = computed(() => {
    const found = conns.value.find(c => c.id === connId.value)
    return found || props.conn || null
  })

  /** 该类型是否需要选择模式（SQL Server / PostgreSQL / Kingbase） */
  const needSchema = computed(() => {
    const t = activeConn.value?.type || ''
    return !!t && schemaLevelOf(t) === 'schema'
  })

  /**
   * 是否为 NoSQL 类型（MongoDB / Redis / Elasticsearch）。
   * 这些类型没有 SqlDialect，巡检与治理的分析规则不适用，
   * 需要在前端明确提示，而不是让用户看到「空结果」。
   */
  const isNoSqlConn = computed(() => {
    const t = activeConn.value?.type || ''
    return !!t && isNoSql(t)
  })

  /**
   * 传给后端的库标识：
   * schema 层级类型用 `库.模式` 复合串，其余类型原样返回。
   */
  const effectiveDatabase = computed(() => {
    if (!database.value) return ''
    if (needSchema.value && schema.value) return `${database.value}.${schema.value}`
    return database.value
  })

  const connLabel = (c) => {
    if (!c) return ''
    const ty = c.type || c.dbType || ''
    return ty ? `${c.name || t('kbs.untitled')} · ${ty}` : (c.name || t('kbs.untitled'))
  }

  /**
   * 解析父组件（左侧对象树）当前选中项为「库 + 模式」。
   *
   * <p>SQL Server / PostgreSQL 等 schema 层级类型，对象树选中 schema 时
   * currentDb 形如 `dm.dbo`，这里拆成库与模式分别匹配；
   * 其余类型只有库名。
   *
   * <p>设计约定：面板默认值与左侧对象树**保持一致**，
   * 左侧没选时返回空 —— 不再自动兜底到「第一个非系统库」，
   * 否则会出现「左侧什么都没选，面板却自己选了某个库」的错位感。
   */
  const preferredFromTree = () => {
    const raw = String(props.database || '').trim()
    if (!raw) return { db: '', schema: '' }
    const i = raw.indexOf('.')
    if (i <= 0) return { db: raw, schema: '' }
    return { db: raw.slice(0, i), schema: raw.slice(i + 1) }
  }

  /**
   * 拉取连接列表。
   * 仅在父组件指定了连接时才预选；否则留空，由用户主动选择——
   * 避免默认落到第一个连接（可能网络不通）导致一打开面板就报错。
   */
  const loadConns = async () => {
    try {
      const list = await listConnections()
      conns.value = Array.isArray(list) ? list : (list?.data || [])
    } catch (e) {
      conns.value = []
    }
    if (props.conn?.id && conns.value.some(c => c.id === props.conn.id)) {
      // 跟随左侧对象树当前选中的连接
      connId.value = props.conn.id
    } else {
      // 左侧未选中连接（或该连接已被删除）→ 留空，由用户主动选择。
      // 不自动落到第一个连接：那可能与左侧树不一致，且该连接未必可连通。
      connId.value = ''
    }
  }

  /** 按当前连接加载数据库列表，并确定默认选中项 */
  const loadDbs = async () => {
    const seq = ++loadSeq
    const conn = activeConn.value
    if (!conn) {
      dbs.value = []
      database.value = ''
      dbError.value = ''
      return
    }
    loadingDbs.value = true
    dbError.value = ''
    let list = []
    let err = ''
    try {
      const raw = await listDatabases(conn.id)
      list = (raw || [])
        .map(d => (typeof d === 'string' ? d : (d.name || d.db || d.schema || '')))
        .filter(Boolean)
    } catch (e) {
      err = e?.message || e?.toString?.() || t('aictx.dbListFailed')
    }
    // 期间用户切换了上游→ 丢弃本次结果
    if (seq !== loadSeq) return
    dbs.value = list
    if (err) {
      dbError.value = err
    } else if (!list.length) {
      // 接口成功但没有任何库：通常是权限不足或该实例下确实没有可用库
      dbError.value = t('aictx.dbListEmpty')
    }
    // 默认库优先级：左侧树当前选中的库 > 连接配置里的默认库；都不满足则**留空**。
    // 注意：不再自动落到「第一个非系统库」/「库列表第一项」——
    // 面板需与左侧对象树保持一致，左侧没选库时这里就应该是空的。
    const want = preferredFromTree()
    let prefer = ''
    if (want.db && list.includes(want.db)) prefer = want.db
    else if (conn.database && list.includes(conn.database)) prefer = conn.database
    if (!database.value || !list.includes(database.value)) {
      database.value = prefer
    }
    loadingDbs.value = false
  }

  /** schema 层级类型：按当前库加载模式列表，默认选中该类型的 defaultSchema */
  const loadSchemas = async () => {
    const seq = loadSeq
    schemas.value = []
    schema.value = ''
    const conn = activeConn.value
    if (!needSchema.value || !conn || !database.value) return
    loadingSchemas.value = true
    let list = []
    try {
      const raw = await listSchemas(conn.id, database.value)
      list = (raw || [])
        .map(s => (typeof s === 'string' ? s : (s.name || s.schema || '')))
        .filter(Boolean)
    } catch (e) {
      list = []
    }
    if (seq !== loadSeq) return
    schemas.value = list
    // 默认模式优先级：左侧树当前选中的模式 > 该类型的 defaultSchema（dbo / public）> 第一个非系统模式。
    // 模式是 schema 层级类型能正常取表的前提，故仍保留兜底（与库的「没选就空」策略不同）。
    const want = preferredFromTree()
    const def = byType(conn.type).defaultSchema
    if (want.schema && list.includes(want.schema)) {
      schema.value = want.schema
    } else if (def && list.includes(def)) {
      schema.value = def
    } else {
      schema.value = list.find(s => !SYSTEM_NAMES.test(s)) || list[0] || ''
    }
    loadingSchemas.value = false
  }

  /**
   * 按需加载当前库的表清单（供「表名」下拉使用）。
   * 用 effectiveDatabase（含模式）作为库标识，SQL Server 等只会列出该模式下的表。
   */
  const loadTables = async () => {
    // 标记「用户需要表清单」：即使此刻上游未就绪，库/模式就绪后 watch 会补一次加载
    tablesRequested = true
    const conn = activeConn.value
    const db = effectiveDatabase.value
    if (!conn || !db) {
      //上游未就绪（或刚被切换）：清空旧结果，避免展示过期表清单。
      // 注意：此处**不能**递增代次——否则会把在途请求的响应判为过期丢弃，
      // 而复位 loading 正是在那条响应里完成的，会导致下拉永久卡在「加载中」。
      tables.value = []
      loadingTables.value = false
      return
    }
    // 只在真正发起请求时才递增代次
    const seq = ++tableSeq
    tables.value = []
    tableError.value = ''
    loadingTables.value = true
    clearTimeout(tableGuard)
    tableGuard = setTimeout(() => {
      if (seq === tableSeq) loadingTables.value = false
    }, 30000)
    let list = []
    let err = ''
    try {
      const raw = await listTables(conn.id, db)
      list = (raw || [])
        .map(t => (typeof t === 'string' ? t : (t.name || t.table || '')))
        .filter(Boolean)
    } catch (e) {
      list = []
      err = e?.message || e?.toString?.() || t('aictx.tableListFailed')
    }
    // 期间用户切换了上游→ 丢弃本次结果（loading 由最新那次请求收尾）
    if (seq !== tableSeq) return
    clearTimeout(tableGuard)
    tables.value = list
    tableError.value = err
    loadingTables.value = false
  }

  /** 打开面板时初始化：只拉连接列表；已有确定连接才继续加载库与模式 */
  const init = async () => {
    // 重新打开面板：表清单回到「按需加载」，避免无谓请求
    tablesRequested = false
    tables.value = []
    tableError.value = ''
    clearTimeout(tableGuard)
    loadingTables.value = false
    // 每次打开都以「左侧对象树当前选择」为准：
    // 先清空上次面板内留在的库/模式，交给 loadDbs/loadSchemas 重新决定，
    // 否则会出现「左侧换了库，面板还显示上次的库」的错位。
    database.value = ''
    schema.value = ''
    dbs.value = []
    schemas.value = []
    dbError.value = ''
    await loadConns()
    if (activeConn.value) {
      await loadDbs()
      await loadSchemas()
    }
  }

  // 切换连接 → 清空下游并重新加载
  watch(connId, (nv, ov) => {
    if (nv === ov) return
    database.value = ''
    schema.value = ''
    dbs.value = []
    schemas.value = []
    if (nv) loadDbs().then(loadSchemas)
    else {
      dbError.value = ''
    }
  })

  // 切换库 → 重新加载模式（已请求过表清单则跟随刷新）
  watch(database, (nv, ov) => {
    if (nv !== ov) {
      loadSchemas()
      if (tablesRequested) loadTables()
    }
  })

  // 切换模式 → 表清单跟随刷新（SQL Server 等不同模式下表不同）
  watch(schema, (nv, ov) => {
    if (nv !== ov && tablesRequested) loadTables()
  })

  // 面板打开时初始化
  watch(() => props.modelValue, (v) => { if (v) init() })

  return {
    conns, connId, dbs, database, schemas, schema,
    tables, loadingTables, tableError, connGroups,
    activeConn, needSchema, isNoSqlConn, effectiveDatabase,
    loadingDbs, loadingSchemas, dbError,
    connLabel, init, loadConns, loadDbs, loadSchemas, loadTables
  }
}
