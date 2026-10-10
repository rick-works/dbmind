import axios from 'axios'
import { t } from '../utils/i18n'

const http = axios.create({ baseURL: '', timeout: 300000 })

// 桌面壳在页面 URL 上带的一次性本地访问令牌（见 desktop/main.js）。
// 浏览器直接访问 / 开发态没有该参数，后端也未启用校验，接口照常可用。
export const localToken = (() => {
  try { return new URLSearchParams(location.search).get('token') || '' } catch { return '' }
})()

if (localToken) {
  http.interceptors.request.use((cfg) => {
    cfg.headers = cfg.headers || {}
    cfg.headers['X-上游-Token'] = localToken
    return cfg
  })
}

http.interceptors.response.use(
  (response) => response,
  (error) => {
    // 主动取消的请求（AbortController）不是错误，直接透传，避免误报网络失败
    if (error && (error.code === 'ERR_CANCELED' || error.name === 'CanceledError' || error.name === 'AbortError')) {
      return Promise.reject(error)
    }
    // 「AI 服务未配置」这类引导性错误：界面会弹确认框/链接引导去设置，
    // 拦截器里再打一行 error 级堆栈纯属控制台噪音 —— 降为 debug。
    // （error.message 此时还是 axios 原始文本，业务文案在 response.data.message 里）
    // **先**把业务文案改写到 error.message —— 调用方（弹窗/聊天流）拿到的必须是可读原因，
    // 且「前往设置」的引导规则靠它命中。之前降噪分支提前 return 跳过了这段改写，
    // 结果 AI 未配置的错误反而显示英文原文、链接也不出现（真机踩过）。
    if (error.response) {
      const data = error.response.data
      if (data && typeof data === 'object') {
        if (data.message) {
          error.message = data.message
        } else if (data.error) {
          error.message = typeof data.error === 'string' ? data.error : JSON.stringify(data.error)
        } else {
          error.message = JSON.stringify(data)
        }
      } else if (data && typeof data === 'string') {
        error.message = data
      } else {
        error.message = t('api.requestFailed', { status: error.response.status })
      }
    }
    // 「AI 服务未配置」这类引导性错误：界面会弹确认框/链接引导去设置，
    // 拦截器里再打一行 error 级堆栈纯属控制台噪音 —— 降为 debug。
    if (/请先在「设置」|Enable and configure the AI service/.test(String(error.message || ''))) {
      console.debug('[dbmind] AI 服务未配置（界面已引导）:', error.message)
      return Promise.reject(error)
    }
    console.error('axios response error:', error)
    if (error.response) {
      console.error('axios error response:', error.response.status, error.response.data)
    } else if (error.request) {
      // 后端不可达：通知授权层弹出授权/激活弹窗（若当前未授权），避免误报“网络失败”
      error.message = t('api.networkFailed')
      try { window.dispatchEvent(new CustomEvent('上游:net-offline')) } catch (e) { /* 忽略 */ }
    }
    return Promise.reject(error)
  }
)

// ==================== 连接类型 → 数据源模块路由 ====================
// 后端已按「数据库类型」垂直切片为数据源模块：/api/mysql、/api/sqlserver、/api/oracle……
// 一个模块可服务多种类型（mysql 模块服务 MYSQL/MARIADB/DORIS，oracle 模块服务 ORACLE/DM）。
// 因此带连接 id 的请求统一解析为 /api/{module}/{id}/…，不再走宿主旧聚合路由。
//
// KINGBASE 归 `postgresql` 模块：它与 PG 共用同一套方言分支（内核侧 module_of_type_code
// 也是这么映的）。之前这里写成 oracle，前后端对同一个类型给出两套模块归属 ——
// 当前只因「9 个前缀共用同一批 handler」才没出错，语义上却是拧着的。
const TYPE_TO_MODULE = {
  MYSQL: 'mysql', MARIADB: 'mysql', DORIS: 'mysql',
  POSTGRESQL: 'postgresql', KINGBASE: 'postgresql',
  SQLSERVER: 'sqlserver',
  ORACLE: 'oracle', DM: 'oracle',
  DB2: 'db2',
  H2: 'h2',
  DERBY: 'derby',
  CLICKHOUSE: 'clickhouse',
  SQLITE: 'sqlite',
  MONGODB: 'mongodb',
  REDIS: 'redis',
  ELASTICSEARCH: 'elasticsearch'
}

// 连接 id -> type 缓存：请求先 listConnections 填充，TTL 3 秒内复用，避免高频请求反复拉取
let connMap = new Map()
let connMapTs = 0
let connFetch = null
function refreshConnMap() {
  if (!connFetch) {
    connFetch = http.get('/api/connections')
      .then(r => {
        const m = new Map()
        for (const c of (r.data || [])) m.set(String(c.id), String(c.type || ''))
        connMap = m
        connMapTs = Date.now()
      })
      .catch(() => { connMapTs = Date.now() })
      .finally(() => { connFetch = null })
  }
  return connFetch
}
function moduleOfType(type) {
  const m = TYPE_TO_MODULE[String(type || '').toUpperCase()]
  if (!m) throw new Error(t('api.unsupportedType', { type }))
  return m
}
async function baseOf(connId) {
  if (Date.now() - connMapTs > 3000) await refreshConnMap()
  if (!connMap.has(String(connId))) await refreshConnMap()
  const type = connMap.get(String(connId))
  if (!type) throw new Error(t('api.connUnknown', { id: connId }))
  return '/api/' + moduleOfType(type)
}

// 执行/任务标识 -> 模块前缀（由发起端在提交时登记，供取消/轮询使用）
// 执行 id 为一次性标识、只增不减，插入时顺带修剪过期项，避免长时间操作后 Map 无限增长
const idModule = new Map()
const ID_MODULE_MAX = 500
const ID_MODULE_TTL_MS = 10 * 60 * 1000
function rememberExecId(execId, module) {
  if (!execId) return
  idModule.set(String(execId), { module, ts: Date.now() })
  if (idModule.size <= ID_MODULE_MAX) return
  const now = Date.now()
  for (const [k, v] of idModule) {
    if (now - v.ts > ID_MODULE_TTL_MS) idModule.delete(k)
  }
  while (idModule.size > ID_MODULE_MAX) idModule.delete(idModule.keys().next().value)
}
function moduleOfExecId(execId) {
  const e = idModule.get(String(execId))
  if (!e) throw new Error(t('api.taskTypeUnknown'))
  return e.module
}

// 连接管理
export const listConnections = () => http.get('/api/connections').then(r => r.data)
export const saveConnection = async (conn) => {
  const r = await http.post('/api/connections', conn).then(r => r.data)
  refreshConnMap() // 新连接可能立刻被使用，后台刷新类型缓存
  return r
}
export const deleteConnection = (id) => http.delete(`/api/connections/${id}`).then(r => r.data)
// 连接测试：由对应数据源模块提供（未保存的新连接也可直接测试，配置完整即可）
export const testConnection = async (conn) => {
  const m = moduleOfType(conn && conn.type)
  return http.post(`/api/${m}/test`, conn).then(r => r.data)
}
// 按 id 测试：取连接详情（不含口令）交给对应模块，服务端会用已存的口令补齐后再测
export const testConnectionById = async (id) => {
  const conn = await http.get(`/api/connections/${id}`).then(r => r.data)
  if (!conn) throw new Error(t('api.connMissing', { id }))
  const m = moduleOfType(conn.type)
  return http.post(`/api/${m}/test`, conn).then(r => r.data)
}
// 断开某连接的缓存会话（不动连接记录）。数据库侧改了权限/密码后调用 ——
// MySQL 的全局权限变更只对新建连接生效，断开重连才能拿到新权限，免去重启应用
export const disconnectSessions = async (id) => {
  return http.post(`/api/dbmind/connections/${id}/disconnect-sessions`).then(r => r.data)
}
// 只断**某个库**的会话（界面「关闭数据库」用）：该库的语句跑在它的影子连接上，
// 后端按 database 找到影子断掉；主连接的共享会话不动，连接本身还开着
export const disconnectDatabase = async (id, database) => {
  return http.post(`/api/dbmind/connections/${id}/disconnect-database`, { database }).then(r => r.data)
}
// 按 id 获取单个连接的详情：**不含任何口令**，只有 hasPassword / hasSshPassword 标记
export const getConnectionById = (id) => http.get(`/api/connections/${id}`).then(r => r.data)
// 复制连接（口令由服务端一并复制，前端拿不到明文）
export const copyConnection = (id, name) =>
  http.post(`/api/connections/${id}/copy`, null, { params: name ? { name } : {} }).then(r => r.data)

// 数据库类型与驱动
export const getDriverTypes = () => http.get('/api/drivers/types').then(r => r.data)
export const getDriverStatus = () => http.get('/api/drivers/status').then(r => r.data)
// 手动下载某个类型的驱动（后端按「镜像源」设置去拉；失败会把原因与手工办法一起带回来）
export const installDriver = (code) => http.post(`/api/drivers/${code}/install`).then(r => r.data)
// 驱动下载进度（首次连接 / 手动下载时后端会去 Maven 拉）：{ status, percent, received, total, index, files }
export const getDriverProgress = (code) => http.get(`/api/drivers/${code}/progress`, { timeout: 8000 }).then(r => r.data)
// 手动上传驱动 jar（multipart，字段名 file；可多选 —— 主驱动 + 额外依赖一次传完）。
// timeout: 0 = 不限时：驱动包几十兆，超时中断会留下一个不完整的文件。
export const uploadDriver = (code, files) => {
  const fd = new FormData()
  for (const f of (Array.isArray(files) ? files : [files])) if (f) fd.append('file', f)
  return http.post(`/api/drivers/${code}/upload`, fd, { timeout: 0 }).then(r => r.data)
}
// 驱动下载镜像源（后端持久化，保存后立即生效）
export const getDriverMirror = () => http.get('/api/settings/driver-mirror').then(r => r.data)
// 在系统文件管理器里打开目录（设置 → 存储路径 →「打开目录」）；不传路径时打开导出产物目录
export const openLocalDir = (path) => http.post('/api/settings/open-dir', path ? { path } : {}).then(r => r.data)
export const saveDriverMirror = (mirror) => http.put('/api/settings/driver-mirror', { mirror }).then(r => r.data)
// 旧版 TLS 兼容开关（连接仅支持 TLS 1.0 的旧数据库，如 SQL Server 2008/2012；重启应用后生效）
export const getLegacyTls = () => http.get('/api/settings/legacy-tls').then(r => r.data)
export const saveLegacyTls = (allowLegacyTls) => http.put('/api/settings/legacy-tls', { allowLegacyTls }).then(r => r.data)

// 通用设置（后端 app_settings 表）。**接口后端早就有**（GET /api/dbmind/settings、
// PUT /{key}，安全开关与改会话上限还会即时生效），只是前端从没接过 ——
// 生产保护、AI 写开关、会话上限这些能力因此一直停在「有后端、没界面」的状态。
export const getSettings = () => http.get('/api/dbmind/settings').then(r => r.data)
// 在线更新：检查 Gitee 最新发行版 / 下载安装包并拉起安装器（桌面端覆盖安装）
export const checkUpdate = () => http.get('/api/update/check', { timeout: 20000 }).then(r => r.data)
export const applyUpdate = (dir) => http.post('/api/update/apply', { dir: dir || null }, { timeout: 600000 }).then(r => r.data)
// 下载进度（后台任务写、前端轮询）：{ status, received, total, speed, error, ... }
export const pickUpdateDir = (title) => http.get('/api/update/pick-dir', { params: { title }, timeout: 600000 }).then(r => r.data)
export const updateDirs = () => http.get('/api/update/dirs', { timeout: 10000 }).then(r => r.data)
export const updateProgress = () => http.get('/api/update/progress', { timeout: 10000 }).then(r => r.data)
// 清掉已结束（成功/失败）的更新任务状态：失败后必须还能重新检查，
// 而不是每次点更新图标都弹回同一个报错页（要重启软件才行）
export const dismissUpdate = () => http.post('/api/update/dismiss').then(r => r.data)
export const putSetting = (key, value) =>
  http.put(`/api/dbmind/settings/${encodeURIComponent(key)}`, { value: String(value) }).then(r => r.data)
// 一键作废**所有**连接的结构缓存（设置 → 查询 →「刷新结构缓存」）
export const clearSchemaCache = () => http.delete('/api/dbmind/schema-cache').then(r => r.data)
// 缓存页签：体量清单 + 勾选清理（清单与清理**同一个端点**，POST 即清理）
export const getCacheItems = () => http.get('/api/dbmind/cache').then(r => r.data)
export const clearCaches = (keys) => http.post('/api/dbmind/cache', { keys }).then(r => r.data)

// 数据库元数据（关系型）——按连接类型路由到数据源模块 /api/{module}
export const getFeatures = async (id) => { const b = await baseOf(id); return http.get(`${b}/${id}/features`).then(r => r.data) }
// catalog 清单：只有 catalog 层级的类型（目前 Doris）会返回非空，其余类型返回 `[]`
export const listCatalogs = async (id) => { const b = await baseOf(id); return http.get(`${b}/${id}/catalogs`).then(r => r.data) }
// database 可选：catalog 层级用它指定「在哪个 catalog 下列库」
export const listDatabases = async (id, catalog) => {
  const b = await baseOf(id)
  const params = catalog ? { catalog } : {}
  return http.get(`${b}/${id}/databases`, { params }).then(r => r.data)
}
export const listSchemas = async (id, database) => { const b = await baseOf(id); return http.get(`${b}/${id}/schemas`, { params: { database } }).then(r => r.data) }
export const listTables = async (id, database) => { const b = await baseOf(id); return http.get(`${b}/${id}/tables`, { params: { database } }).then(r => r.data) }
// 批量真实行数（SELECT COUNT(*)）：TABLE_ROWS 是估算值（Doris/ClickHouse 等常为 0），展开库后异步回填树节点行数
export const getTableCounts = async (id, database, tables) => { const b = await baseOf(id); return http.get(`${b}/${id}/table-count`, { params: { database, tables } }).then(r => r.data) }
export const listColumns = async (id, database, table) => { const b = await baseOf(id); return http.get(`${b}/${id}/columns`, { params: { database, table } }).then(r => r.data) }
// 结果集的列注释（列名小写 → 注释）：SQL 编辑器单表查询在表头第二行显示字段注释；
// 各方言的取法由后端 dialect.table_comments 出（MySQL/PG/MSSQL/Oracle/DM/CK…）
export const getColumnComments = async (id, database, table) => { const b = await baseOf(id); return http.get(`${b}/${id}/column-comments`, { params: { database, table } }).then(r => r.data) }
// 库内对象搜索（服务端批量）：一次请求返回表名命中 + 列名命中，替代「逐张表请求 columns」
export const searchObjects = async (id, database, keyword) => { const b = await baseOf(id); return http.get(`${b}/${id}/search-objects`, { params: { database, keyword } }).then(r => r.data) }
export const getTableData = async (id, params, signal) => { const b = await baseOf(id); return http.get(`${b}/${id}/data`, { params, signal }).then(r => r.data) }
export const getTableDdl = async (id, database, table) => { const b = await baseOf(id); return http.get(`${b}/${id}/ddl`, { params: { database, table } }).then(r => r.data) }
// 数据库实时监控总览（按方言执行只读监控查询：连接数/QPS/慢查询/表空间 TopN 等）
export const monitorOverview = async (id, database) => { const b = await baseOf(id); return http.get(`${b}/${id}/monitor`, { params: { database } }).then(r => r.data) }
// 查询历史（**内核原生接口**，上游那层没有）：最近执行过的 SQL。
// 欢迎页的「最近查询」用它 —— 一行一条，点一下把那句 SQL 开成新脚本。
export const listHistory = (limit = 8) => http.get('/api/dbmind/history', { params: { limit } }).then(r => r.data)
// 清空查询历史（`DELETE /api/dbmind/history`）：清的是**全部**记录，没有按条删除的接口 ——
// 历史表里也没有"来源"字段，老记录无法事后区分是不是程序发的，所以只能整体清。
export const clearHistory = () => http.delete('/api/dbmind/history').then(r => r.data)
// 统一日志（执行 + AI 审计合并视图，设置 → 日志用）：level 为当前审计级别，items 为日志条目
export const getLogs = (params = {}) => http.get('/api/dbmind/logs', { params: { limit: 50, ...params } }).then(r => r.data)
// 清空全部日志（执行历史 + AI 审计一起清）
export const clearLogs = () => http.delete('/api/dbmind/logs').then(r => r.data)
// 终止会话（监控面板运维动作：KILL 指定线程/会话）
export const monitorKill = async (id, database, sessionId) => { const b = await baseOf(id); return http.post(`${b}/${id}/monitor/kill`, null, { params: { database, sessionId } }).then(r => r.data) }
// 保存表数据修改（增删改）
export const saveTableData = async (id, payload) => { const b = await baseOf(id); return http.post(`${b}/${id}/data-save`, payload).then(r => r.data) }
export const listIndexes = async (id, database) => { const b = await baseOf(id); return http.get(`${b}/${id}/indexes`, { params: { database } }).then(r => r.data) }
export const listProcedures = async (id, database) => { const b = await baseOf(id); return http.get(`${b}/${id}/procedures`, { params: { database } }).then(r => r.data) }
export const listTriggers = async (id, database) => { const b = await baseOf(id); return http.get(`${b}/${id}/triggers`, { params: { database } }).then(r => r.data) }
export const listEvents = async (id, database) => { const b = await baseOf(id); return http.get(`${b}/${id}/events`, { params: { database } }).then(r => r.data) }
export const listUsers = async (id, database) => { const b = await baseOf(id); return http.get(`${b}/${id}/users`, { params: { database } }).then(r => r.data) }
export const getUserInfo = async (id, database, name) => { const b = await baseOf(id); return http.get(`${b}/${id}/user-info`, { params: { database, name } }).then(r => r.data) }
export const userAction = async (id, payload) => { const b = await baseOf(id); return http.post(`${b}/${id}/user-action`, payload).then(r => r.data) }
export const getObjectInfo = async (id, database, type, name) => { const b = await baseOf(id); return http.get(`${b}/${id}/object-info`, { params: { database, type, name } }).then(r => r.data) }
// 执行 DDL（表结构 / 索引 / 表选项编辑），自动切库
export const alterTable = async (id, database, sql) => { const b = await baseOf(id); return http.post(`${b}/${id}/alter`, { database, sql }).then(r => r.data) }
// 表级安全操作（清空 / 截断 / 删除 / 重命名），服务端校验表名与操作类型
export const tableAction = async (id, database, table, action, newName) => { const b = await baseOf(id); return http.post(`${b}/${id}/table-action`, { database, table, action, newName }).then(r => r.data) }
// 对象重命名（存储过程 / 函数 / 视图 / 触发器 / 事件）：服务端优先用原生改名语法；
// previewOnly=true 只返回待执行语句 sql 与 native（是否原生改名），不执行
export const renameObject = async (id, payload) => { const b = await baseOf(id); return http.post(`${b}/${id}/rename-object`, payload).then(r => r.data) }
// 生成测试数据（统一端点 /api/datagen，按连接 ID 自动识别数据库类型）：
// 预览同步返回若干行；写入为异步任务，可轮询进度、可取消（取消会在数据库端回滚未提交的数据）
export const dataGenPreview = async (connectionId, payload) => http.post('/api/datagen/preview', payload, { params: { connectionId } }).then(r => r.data)
export const dataGenStart = async (connectionId, payload) => http.post('/api/datagen/start', payload, { params: { connectionId } }).then(r => r.data)
export const dataGenTask = async (taskId) => http.get(`/api/datagen/task/${taskId}`).then(r => r.data)
export const dataGenCancel = async (taskId) => http.post(`/api/datagen/cancel/${taskId}`).then(r => r.data)

// NoSQL 元数据（MongoDB / Redis / Elasticsearch）——按连接类型路由到对应模块
export const noSqlDatabases = async (id) => { const b = await baseOf(id); return http.get(`${b}/${id}/databases`).then(r => r.data) }
export const noSqlCollections = async (id, database) => { const b = await baseOf(id); return http.get(`${b}/${id}/collections`, { params: { database } }).then(r => r.data) }
export const noSqlDocuments = async (id, params, signal) => { const b = await baseOf(id); return http.get(`${b}/${id}/documents`, { params, signal }).then(r => r.data) }
export const noSqlDeleteCollection = async (id, database, collection) => { const b = await baseOf(id); return http.delete(`${b}/${id}/collection`, { params: { database, collection } }).then(r => r.data) }

// SQL 执行（database 可选：目标库名，为空则不切库沿用连接默认库）
// executionId 可选：本次执行的唯一标识，用于执行中取消；signal 为 AbortController 信号
// page/size 可选：分页参数（从 1 开始），不传则保持原有截断行为
export const executeSql = async (id, sql, database, executionId, signal, page, size, internal) => {
  const b = await baseOf(id)
  const module = b.replace('/api/', '')
  rememberExecId(executionId, module)
  return http.post(`${b}/query/${id}`, { sql, database, executionId, page, size, internal: !!internal }, { signal }).then(r => {
    if (!r || !r.data) throw new Error(t('api.emptyResponse'))
    return r.data
  })
}
// 只算总数，不取数（异步补分页器用）：主执行接口已把 COUNT 移出主链路 ——
// 大 JOIN 的计数能拖百秒级，不该卡住数据回显。后端带 10s 总预算，算不出返回 -1。
// ncols 传结果集列数，省掉后端探测列数的一次往返。
export const executeSqlCount = async (id, sql, database, ncols, signal) => {
  const b = await baseOf(id)
  return http.post(`${b}/query/${id}/count`, { sql, database, ncols }, { signal }).then(r => r.data)
}
// 会话级事务控制（事务模式）：begin 关掉编辑器会话的 autocommit，commit/rollback 收尾。
// database 必须与执行查询时传的一致 —— 后端据此解析同一个目标（影子连接等），泳道才对得上。
export const txControl = async (id, action, database = '') => {
  const b = await baseOf(id)
  return http.post(`${b}/query/${id}/tx`, { action, database }, { timeout: 20000 }).then(r => r.data)
}
// 多段 SQL 批量执行：同一连接内按分号顺序执行每段，每段独立返回一个 QueryResult
// （编辑器一次执行多条查询时，结果以「结果1/结果2…」tab 逐条展示）
export const executeSqlBatch = async (id, sql, database, executionId, signal) => {
  const b = await baseOf(id)
  const module = b.replace('/api/', '')
  rememberExecId(executionId, module)
  return http.post(`${b}/query/${id}/batch`, { sql, database, executionId }, { signal }).then(r => {
    if (!r || !r.data) throw new Error(t('api.emptyResponse'))
    return r.data
  })
}
// 取消正在执行的 SQL / NoSQL
export const cancelSql = async (executionId) => { const m = moduleOfExecId(executionId); return http.post(`/api/${m}/query/cancel/${executionId}`).then(r => r.data) }
export const cancelNoSql = async (executionId) => { const m = moduleOfExecId(executionId); return http.post(`/api/${m}/cancel/${executionId}`).then(r => r.data) }
export const executeNoSql = async (id, database, command) => {
  const b = await baseOf(id)
  const module = b.replace('/api/', '')
  const r = await http.post(`${b}/${id}/execute`, { command }, { params: { database } }).then(r => {
    if (!r || !r.data) throw new Error(t('api.emptyResponse'))
    return r.data
  })
  if (r && r.executionId) rememberExecId(r.executionId, module)
  return r
}

// 旧同步导出（保留兼容）
export const exportData = async (id, payload, config = {}) => {
  const b = await baseOf(id)
  return http.post(`${b}/export/${id}`, payload, { responseType: 'blob', ...config }).then(r => r.data)
}

// 导出（异步任务模式）：提交返回 taskId，轮询进度，成功后下载
export const exportStart = async (id, payload) => { const b = await baseOf(id); return http.post(`${b}/export/task/${id}`, payload).then(r => r.data) }
export const exportTask = async (id, taskId) => { const b = await baseOf(id); return http.get(`${b}/export/task/${taskId}`).then(r => r.data) }
export const exportCancel = async (id, taskId) => { const b = await baseOf(id); return http.post(`${b}/export/cancel/${taskId}`).then(r => r.data) }
export const exportDownload = async (id, taskId) => { const b = await baseOf(id); return http.get(`${b}/export/download/${taskId}`, { responseType: 'blob' }).then(r => r.data) }

// 数据库整体转储（异步任务模式）：提交返回 taskId，轮询进度，成功后下载。
// 复用 /export/task、/export/download、/export/cancel 这三个通用端点（dump 任务也存入同一 task 存储）。
export const exportDbStart = async (id, payload) => { const b = await baseOf(id); return http.post(`${b}/export/dump-task/${id}`, payload).then(r => r.data) }

// 导入（异步任务模式）：提交文件返回 taskId，随后轮询进度 / 取消
export const importStart = async (id, formData, config) => { const b = await baseOf(id); return http.post(`${b}/import/${id}`, formData, config).then(r => r.data) }
export const importTask = async (id, taskId) => { const b = await baseOf(id); return http.get(`${b}/import/task/${taskId}`).then(r => r.data) }
export const importCancel = async (id, taskId) => { const b = await baseOf(id); return http.post(`${b}/import/cancel/${taskId}`).then(r => r.data) }

// ==================== 数据对比 / 数据同步（统一端点 /api/compare、/api/sync，独立模块） ====================
// 引擎按 payload 中的 source/target 连接类型自行路由，taskId 为服务端全局唯一，轮询/取消无需登记
export const compareData = async (payload) => { const r = await http.post('/api/compare', payload).then(r => r.data); return r }
export const compareTaskStatus = async (taskId) => http.get(`/api/compare/task/${taskId}`).then(r => r.data)
export const compareCancel = async (taskId) => http.post(`/api/compare/cancel/${taskId}`).then(r => r.data)

export const syncDb = async (payload) => http.post('/api/sync/db', payload).then(r => r.data)
export const syncTaskStatus = async (taskId) => http.get(`/api/sync/task/${taskId}`).then(r => r.data)
export const syncCancel = async (taskId) => http.post(`/api/sync/cancel/${taskId}`).then(r => r.data)

// 数据库备份 / 还原（统一端点 /api/backup）
export const startBackup = async (payload) => http.post('/api/backup/start', payload).then(r => r.data)
export const browseBackupDirs = async (path) => http.get('/api/backup/browse-files', { params: { path } }).then(r => r.data)
export const startRestoreLocal = async (payload) => http.post('/api/backup/restore/start-local', payload).then(r => r.data)
export const backupTaskStatus = async (taskId) => http.get(`/api/backup/task/${taskId}`).then(r => r.data)
export const cancelBackup = async (taskId) => http.post(`/api/backup/cancel/${taskId}`).then(r => r.data)
export const backupInstallConfirm = async (taskId) => http.post(`/api/backup/task/${taskId}/install-confirm`).then(r => r.data)
export const backupInstallManual = async (taskId) => http.post(`/api/backup/task/${taskId}/install-manual`).then(r => r.data)
export const backupInstallSkip = async (taskId) => http.post(`/api/backup/task/${taskId}/install-skip`).then(r => r.data)
// 向导“执行引擎”步骤：按连接类型返回命令行能力、工具与按类型定制提示
export const getBackupCliGuide = async (connectionId) => http.get('/api/backup/cli-guide', { params: { connectionId } }).then(r => r.data)

// 向导第 2 步预检：备份/还原所需命令行工具是否已安装（不依赖任务）
export const getBackupCliToolStatus = async (connectionId, mode) => http.get('/api/backup/cli-tool-status', { params: { connectionId, mode } }).then(r => r.data)
// 向导第 2 步：自动安装命令行工具并重新检测（安装可能耗时较长，不设超时）
export const installBackupCliTool = async (connectionId, mode) => http.post('/api/backup/cli-tool-install', null, {
  params: { connectionId, mode },
  timeout: 0
}).then(r => r.data)

export const restoreTaskStatus = async (taskId) => http.get(`/api/backup/restore/task/${taskId}`).then(r => r.data)
export const cancelRestore = async (taskId) => http.post(`/api/backup/restore/cancel/${taskId}`).then(r => r.data)
export const restoreInstallConfirm = async (taskId) => http.post(`/api/backup/restore/task/${taskId}/install-confirm`).then(r => r.data)
export const restoreInstallManual = async (taskId) => http.post(`/api/backup/restore/task/${taskId}/install-manual`).then(r => r.data)
export const restoreInstallSkip = async (taskId) => http.post(`/api/backup/restore/task/${taskId}/install-skip`).then(r => r.data)

// AI
export const getAiConfig = () => http.get('/api/ai/config').then(r => r.data)
export const saveAiConfig = (cfg) => http.post('/api/ai/config', cfg).then(r => r.data)
export const aiNl2sql = (payload) => http.post('/api/ai/nl2sql', payload).then(r => r.data)
export const aiExplain = (payload) => http.post('/api/ai/explain', payload).then(r => r.data)
export const aiOptimize = (payload) => http.post('/api/ai/optimize', payload).then(r => r.data)
export const aiFix = (payload) => http.post('/api/ai/fix', payload).then(r => r.data)
export const aiChat = (payload) => http.post('/api/ai/chat', payload).then(r => r.data)
// 预热 AI 上下文（表结构元数据与样例缓存）：面板打开/切换数据源时后台调用，减少首条消息等待
export const aiWarmup = (payload) => http.post('/api/ai/warmup', payload).then(r => r.data)
// 流式对话：逐段回调 onDelta，返回完整文本；环境不支持/超时/上游中断时抛错。
// opts.idleMs：多久没收到任何数据就判定为挂死并主动中断（默认 90s，与后端看门狗一致）。
export const aiChatStream = async (payload, onDelta, opts = {}) => {
  const idleMs = opts.idleMs || 90000
  const ctrl = typeof AbortController !== 'undefined' ? new AbortController() : null
  // 外部传入的 signal = 用户点「停止生成」；与内部空闲看门狗共用同一个 controller，
  // 谁先触发都走同一条中断路径，调用方只需区分是「用户停的」还是「挂死了」
  const external = opts.signal || null
  let userAborted = false
  const onExternalAbort = () => {
    userAborted = true
    if (ctrl) ctrl.abort()
  }
  if (external) {
    if (external.aborted) onExternalAbort()
    else external.addEventListener('abort', onExternalAbort, { once: true })
  }
  let idleTimer = null
  const arm = () => {
    if (!ctrl) return
    clearTimeout(idleTimer)
    idleTimer = setTimeout(() => ctrl.abort(), idleMs)
  }
  arm()
  let res
  try {
    res = await fetch('/api/ai/chat/stream', {
      method: 'POST',
      headers: localToken
        ? { 'Content-Type': 'application/json', 'X-上游-Token': localToken }
        : { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
      signal: ctrl ? ctrl.signal : undefined
    })
  } catch (e) {
    clearTimeout(idleTimer)
    if (external) external.removeEventListener('abort', onExternalAbort)
    throw new Error(userAborted ? t('api.generationStopped') : t('api.idleTimeout', { s: Math.round(idleMs / 1000) }))
  }
  if (!res.ok) {
    clearTimeout(idleTimer)
    // 后端的业务错误（如「AI 服务未配置」）带在 JSON body 里：取出来抛中文文案，
    // 让上层提示可读、且能被「前往设置」的引导规则识别 —— 之前只抛 'HTTP 400'。
    let detail = 'HTTP ' + res.status
    try {
      const body = await res.json()
      if (body && body.message) detail = body.message
    } catch { /* body 不是 JSON 就保留状态码 */ }
    throw new Error(detail)
  }
  if (!res.body) { clearTimeout(idleTimer); throw new Error(t('api.noStream')) }
  const reader = res.body.getReader()
  const decoder = new TextDecoder('utf-8')
  let buf = ''
  let full = ''
  let errMsg = ''
  try {
    for (;;) {
      let chunkRes
      try {
        chunkRes = await reader.read()
      } catch (e) {
        throw new Error(userAborted ? t('api.generationStopped') : t('api.interruptedTimeout', { s: Math.round(idleMs / 1000) }))
      }
      const { done, value } = chunkRes
      if (done) break
      arm()
      buf += decoder.decode(value, { stream: true })
      let idx
      while ((idx = buf.indexOf('\n\n')) >= 0) {
        const chunk = buf.slice(0, idx)
        buf = buf.slice(idx + 2)
        for (const line of chunk.split('\n')) {
          if (!line.startsWith('data:')) continue
          const raw = line.slice(5).trim()
          if (!raw) continue
          try {
            const obj = JSON.parse(raw)
            if (obj && obj.d) { full += obj.d; if (onDelta) onDelta(obj.d) }
            // 后端在 done 之前补发的用量帧（上游没给 usage 时**不会发这一帧**）：
            // 用回调交给调用方挂到这条回答下面 —— 不改本函数的返回值（返回值是文本，很稳）
            if (obj && obj.usage && typeof opts.onUsage === 'function') opts.onUsage(obj.usage)
            if (obj && obj.error) errMsg = obj.error
          } catch (e) { /* 忽略非 JSON 分片 */ }
        }
      }
    }
  } finally {
    clearTimeout(idleTimer)
    if (external) external.removeEventListener('abort', onExternalAbort)
  }
  if (errMsg) {
    const err = new Error(errMsg)
    err.partial = full // 已收到的部分内容，调用方可决定是保留还是回退
    throw err
  }
  return full
}
// AI 数据字典：返回 { content, tableCount }
export const aiDataDict = (payload) => http.post('/api/ai/datadict', payload).then(r => r.data)
// AI 结果导出为文件：payload = { title, markdown, format: md|docx|xlsx|pdf, fileName }，返回文件流（Blob）
// 出错时后端返回 400 + JSON，axios 以 blob 接收，调用方需还原错误信息
export const aiExportDoc = (payload) => http.post('/api/ai/export/doc', payload, { responseType: 'blob' }).then(r => r.data)
// 自然语言筛选：返回 { where, sql, notes }
export const aiFilter = (payload) => http.post('/api/ai/filter', payload).then(r => r.data)
// AI 用量统计
export const getAiUsage = (days = 30) => http.get('/api/ai/usage', { params: { days } }).then(r => r.data)
// 表健康巡检（纯规则扫描，不消耗 AI 调用）
export const aiPatrol = (payload) => http.post('/api/ai/patrol', payload).then(r => r.data)
// 数据治理（规则引擎，几乎零 AI 成本）
export const aiScanSensitive = (payload) => http.post('/api/ai/governance/sensitive', payload).then(r => r.data)
export const aiQualityRules = (payload) => http.post('/api/ai/governance/quality/rules', payload).then(r => r.data)
export const aiQualityCheck = (payload) => http.post('/api/ai/governance/quality/check', payload).then(r => r.data)
// 可配置业务规则：类型目录 / 读取 / 保存 / 删除 / 已配置表 / 批量扫描 / 报告
export const aiQualityTypes = () => http.get('/api/ai/quality/types').then(r => r.data)
export const aiQualitySaved = (payload) => http.post('/api/ai/quality/rules/saved', payload).then(r => r.data)
export const aiQualitySave = (payload) => http.post('/api/ai/quality/rules/save', payload).then(r => r.data)
export const aiQualityConfigured = (payload) => http.post('/api/ai/quality/configured', payload).then(r => r.data)
export const aiQualityScan = (payload) => http.post('/api/ai/quality/scan', payload).then(r => r.data)
export const aiQualityReport = (payload) => http.post('/api/ai/quality/report', payload).then(r => r.data)
// 导出违规行明细（哪些数据行违反了规则）
export const aiQualityViolations = (payload) => http.post('/api/ai/quality/violations', payload).then(r => r.data)
export const getErGraph = (payload) => http.post('/api/ai/governance/er-graph', payload).then(r => r.data)
export const aiCapacity = (payload) => http.post('/api/ai/governance/capacity', payload).then(r => r.data)
// 命令面板：自然语言 → 操作计划
export const aiPlan = (payload) => http.post('/api/ai/plan', payload).then(r => r.data)
// P1 能力：数据洞察 / 慢查询诊断 / Agent 智能分析
export const aiInsight = (payload) => http.post('/api/ai/insight', payload).then(r => r.data)
export const aiDiagnose = (payload) => http.post('/api/ai/diagnose', payload).then(r => r.data)
export const aiAgent = (payload) => http.post('/api/ai/agent', payload).then(r => r.data)

// SQL 快捷验证（不消耗 AI 调用）：执行计划 / 试跑（只读，强制限行）
export const aiSqlPlan = (payload) => http.post('/api/ai/sql/plan', payload).then(r => r.data)
export const aiSqlTryRun = (payload) => http.post('/api/ai/sql/try-run', payload).then(r => r.data)

// ==================== 文档知识库（本地版 Dify 知识库） ====================
// 必须用 POST：后端 `/api/ai/kb/list` 只注册了 POST（知识库那一组全是 POST + JSON body），
// 老写法发 GET 会拿到 **405**，界面上表现为「知识库下拉/列表永远是空的」
// （KnowledgeStudio、SaveToKbDialog、KbIngestPanel、AiPanel 四处都受影响）。
export const kbList = () => http.post('/api/ai/kb/list', {}).then(r => r.data)
export const kbCreate = (payload) => http.post('/api/ai/kb/create', payload).then(r => r.data)
export const kbRename = (payload) => http.post('/api/ai/kb/rename', payload).then(r => r.data)
export const kbDelete = (payload) => http.post('/api/ai/kb/delete', payload).then(r => r.data)
export const kbDocs = (payload) => http.post('/api/ai/kb/docs', payload).then(r => r.data)
/** 探测各模型能否用于向量化（名称识别 + 实调一次 /embeddings，后端有缓存） */
export const probeEmbedModels = () => http.get('/api/ai/models/embed-capability').then(r => r.data)
export const kbDocChunks = (payload) => http.post('/api/ai/kb/doc/chunks', payload).then(r => r.data)
export const kbPreview = (payload) => http.post('/api/ai/kb/preview', payload).then(r => r.data)
export const kbReindex = (payload) => http.post('/api/ai/kb/reindex', payload).then(r => r.data)
export const kbImportDoc = (payload) => http.post('/api/ai/kb/doc/import', payload).then(r => r.data)
export const kbDeleteDoc = (payload) => http.post('/api/ai/kb/doc/delete', payload).then(r => r.data)
export const kbSearch = (payload) => http.post('/api/ai/kb/search', payload).then(r => r.data)
export const kbSaveConfig = (payload) => http.post('/api/ai/kb/config', payload).then(r => r.data)
/** 「知识入库」：分析一份待入库资料，返回推荐分段配置 + 模型起的名称/说明 */
export const aiKbAutoPlan = (payload) => http.post('/api/ai/kb/auto-plan', payload).then(r => r.data)
/** 「知识入库」体检：规则判死 + 模型判定这份资料能否做知识库，并给问题清单 */
export const aiKbInspect = (payload) => http.post('/api/ai/kb/inspect', payload).then(r => r.data)
/** 「知识入库」规范优化：只清洗结构不改原意，返回优化正文 + 事实保留率校验 */
export const aiKbPolish = (payload) => http.post('/api/ai/kb/polish', payload).then(r => r.data)
/** 归纳一个知识库：装了什么、能问什么、有哪些关键词 */
export const aiKbOutline = (payload) => http.post('/api/ai/kb/outline', payload).then(r => r.data)

// 路径设置（数据目录 / 驱动目录）
export const getPathSettings = () => http.get('/api/settings/paths').then(r => r.data)
export const savePathSettings = (payload) => http.put('/api/settings/paths', payload).then(r => r.data)

// 执行 SQL 文件（异步任务模式）：逐语句执行，实时进度/日志，可取消
export const runSqlFileStart = async (id, payload) => { const b = await baseOf(id); return http.post(`${b}/query/run-file-task/${id}`, payload).then(r => r.data) }
export const runSqlFileStatus = async (id, taskId) => { const b = await baseOf(id); return http.get(`${b}/query/run-file-task/status/${taskId}`).then(r => r.data) }
export const runSqlFileCancel = async (id, taskId) => { const b = await baseOf(id); return http.post(`${b}/query/run-file-task/cancel/${taskId}`).then(r => r.data) }

export default http
