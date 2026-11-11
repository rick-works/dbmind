import { t } from './i18n'
import { CONN_FILE_FORMAT } from './connTransfer'
/**
 * 连接文件的**多来源**解析：DBMind / Navicat (.ncx) / DBeaver (.json)。
 *
 * 三个来源的共同目标只有一个：产出 DBMind 自己那套字段（`{ name, type, host, port, … }`），
 * 交给现有的 `saveConnection` 逐条落库。差异全部收敛在本文件里，UI 只认 `parseImportFile()`。
 *
 * 三条刻意的取舍：
 * 1. **类型映射表只认认识的**：认不出来不猜（猜错会生成一条连不上的连接，比"导入失败"更难查），
 *    直接把 `type` 留空，由导入预览标出来让用户自己选。
 * 2. **口令能拿就拿，拿不到就标记**：Navicat 的 `Password` 是加密串（AES/Blowfish，各家版本还不同），
 *    DBeaver 的口令存在 `credentials-config.json` 里且默认不导出，所以这两种来源基本都会落到
 *    「需补填」。硬解一个可能的乱码比"空着"更糟。
 * 3. **不认识的元素忽略而不报错**：Navicat/DBeaver 的 XML/JSON 版本差异很大，字段名大小写、
 *    嵌套层级都变过，所以按属性名**大小写不敏感**地在整棵子树里找，找不到就用默认值。
 */

/** 支持的来源（顺序 = 检测优先级） */
export const IMPORT_SOURCES = ['dbmind', 'navicat', 'dbeaver']

/** 来源展示名（i18n） */
export const sourceLabel = (source) => t('ci.src.' + (IMPORT_SOURCES.includes(source) ? source : 'unknown'))

/** 外部工具的类型写法 → DBMind 的 type code（键已归一化：小写 + 去掉非字母数字） */
const TYPE_ALIASES = {
  mysql: 'MYSQL', mysql5: 'MYSQL', mysql8: 'MYSQL', mysql57: 'MYSQL', mysql80: 'MYSQL',
  mariadb: 'MARIADB', mariadb10: 'MARIADB',
  postgres: 'POSTGRESQL', postgresql: 'POSTGRESQL', pg: 'POSTGRESQL', pgjdbc: 'POSTGRESQL',
  sqlserver: 'SQLSERVER', mssql: 'SQLSERVER', microsoftsqlserver: 'SQLSERVER', sqlserverjtds: 'SQLSERVER',
  oracle: 'ORACLE', oracledb: 'ORACLE', oraclethin: 'ORACLE',
  sqlite: 'SQLITE', sqlite3: 'SQLITE', sqlitejdbc: 'SQLITE',
  h2: 'H2', db2: 'DB2', db2luw: 'DB2', derby: 'DERBY', apachederby: 'DERBY',
  clickhouse: 'CLICKHOUSE', ch: 'CLICKHOUSE', doris: 'DORIS', apachedoris: 'DORIS',
  mongodb: 'MONGODB', mongo: 'MONGODB', redis: 'REDIS', elasticsearch: 'ELASTICSEARCH', es: 'ELASTICSEARCH',
  dm: 'DM', dameng: 'DM', kingbase: 'KINGBASE', kingbasees: 'KINGBASE'
}

/** 归一化类型写法：小写、去掉空格/点/横线/下划线等一切非字母数字 */
export const normalizeTypeKey = (raw) => String(raw || '').toLowerCase().replace(/[^a-z0-9]/g, '')

/** 外部类型写法 → type code（认不出返回空串，绝不猜） */
export function typeCodeOf(raw) {
  return TYPE_ALIASES[normalizeTypeKey(raw)] || ''
}

/** 从 JDBC URL 兜底认类型（DBeaver 的 provider 缺失时用） */
function typeCodeOfUrl(url) {
  const u = String(url || '').toLowerCase()
  if (!u) return ''
  if (u.includes(':mysql:')) return 'MYSQL'
  if (u.includes(':mariadb:')) return 'MARIADB'
  if (u.includes(':postgresql:')) return 'POSTGRESQL'
  if (u.includes(':sqlserver:')) return 'SQLSERVER'
  if (u.includes(':oracle:')) return 'ORACLE'
  if (u.includes(':sqlite:')) return 'SQLITE'
  if (u.includes(':h2:')) return 'H2'
  if (u.includes(':db2:')) return 'DB2'
  if (u.includes(':derby:')) return 'DERBY'
  if (u.includes(':clickhouse:')) return 'CLICKHOUSE'
  if (u.includes(':mongodb:')) return 'MONGODB'
  return ''
}

/** 解析 jdbc:xxx://host:port/db?… 里的 host/port/database（DBeaver 只给 url 的情况）。
 *  **必须带 `//`**：`jdbc:sqlite:D:\data\demo.db` 这种没有 authority，把它当 host 会解析出
 *  host="D"、port="\data\demo.db"，界面上就变成主机只有一个字母。 */
function splitJdbcUrl(url) {
  const out = { host: '', port: '', database: '' }
  const text = String(url || '')
  const m = text.match(/^jdbc:[a-z0-9]+:\/\/([^/?#]*)(?:\/([^?#]*))?/i)
  if (!m) return out
  const authority = m[1] || ''
  const [host, port] = authority.split(':')
  out.host = host || ''
  out.port = /^\d+$/.test(port || '') ? port : ''
  out.database = (m[2] || '').split(';')[0]
  return out
}

/** 文件型库（SQLite / H2）的路径：`jdbc:sqlite:D:\x.db`、`jdbc:h2:file:D:\x` 里 `//` 之后那截 */
function filePathOfUrl(url) {
  const m = String(url || '').match(/^jdbc:(?:sqlite|h2):(?:file:)?([^;]*)/i)
  return m ? m[1] : ''
}

/** 端口统一成数字：外部工具里是字符串（"3306"），后端 `as_u64` 只认数字，字符串会被静默丢掉 */
function toPort(value) {
  const n = Number(String(value == null ? '' : value).trim())
  return Number.isInteger(n) && n > 0 && n <= 65535 ? n : undefined
}

/** 口令来源标记（导入预览里据此提示"要不要补填"） */
export const PWD_INCLUDED = 'included'
export const PWD_MISSING = 'missing'
export const PWD_ENCRYPTED = 'encrypted'

/**
 * 识别来源。只看内容形态，不看扩展名（用户常把 .ncx 改名成 .xml，反过来也一样）。
 * @returns {'dbmind'|'navicat'|'dbeaver'|''}
 */
export function detectSource(text) {
  const body = String(text || '').trim()
  if (!body) return ''
  if (body.startsWith('<')) return /<Connections?\b/i.test(body) ? 'navicat' : ''
  let data = null
  try {
    data = JSON.parse(body.replace(/^\uFEFF/, ''))
  } catch (e) {
    return ''
  }
  if (Array.isArray(data)) return 'dbmind'
  if (!data || typeof data !== 'object') return ''
  if (data.format === CONN_FILE_FORMAT) return 'dbmind'
  if (Array.isArray(data.connections)) return 'dbmind'
  // DBeaver：project 导出是 `connections: { id: "<JSON 字符串>" }`，单连接导出是顶层 provider/configuration
  if (data.connections && typeof data.connections === 'object') return 'dbeaver'
  if (data.provider || data.configuration || data.driver) return 'dbeaver'
  return ''
}

/* ============================ DBMind ============================ */

function parseDbmind(data) {
  const list = Array.isArray(data) ? data : (data && data.connections) || []
  const out = []
  for (const item of list) {
    if (!item || typeof item !== 'object') continue
    const name = String(item.name || '').trim()
    const type = String(item.type || '').trim().toUpperCase()
    if (!name || !type) continue
    out.push({
      ...item,
      name,
      type,
      pwd: item.password ? PWD_INCLUDED : PWD_MISSING
    })
  }
  return out
}

/* ============================ Navicat ============================ */

/** 收集元素自身与所有后代的属性（**先到先得**，浅层覆盖深层，避免子元素把父级同名属性冲掉） */
function attrsDeep(el) {
  const out = {}
  const push = (node) => {
    for (const attr of Array.from(node.attributes || [])) {
      const key = String(attr.name || '').toLowerCase()
      if (!(key in out)) out[key] = attr.value
    }
  }
  push(el)
  for (const child of Array.from(el.getElementsByTagName('*'))) push(child)
  return out
}

/** 按若干候选名取属性（大小写不敏感，空值跳过） */
function attr(attrs, ...names) {
  for (const name of names) {
    const v = attrs[String(name).toLowerCase()]
    if (v != null && String(v).trim() !== '') return String(v).trim()
  }
  return ''
}

/** Navicat 的口令是加密串（新版 AES、老版 Blowfish，各版本 key 不同）：纯十六进制就当作"解不开" */
function looksEncrypted(value) {
  return /^[0-9a-f]+$/i.test(value) && value.length >= 16 && value.length % 2 === 0
}

function parseNavicat(text) {
  const doc = new DOMParser().parseFromString(text, 'text/xml')
  if (doc.getElementsByTagName('parsererror').length) return null
  const nodes = Array.from(doc.getElementsByTagName('*'))
    .filter((el) => /^connection$/i.test(el.tagName))
  const out = []
  for (const node of nodes) {
    const attrs = attrsDeep(node)
    const name = attr(attrs, 'ConnectionName', 'Name', 'Connection')
    if (!name) continue
    const rawType = attr(attrs, 'ConnType', 'Type', 'DatabaseType')
    const password = attr(attrs, 'Password', 'Pwd')
    const sqliteFile = attr(attrs, 'SQLiteFile', 'FilePath', 'DataFile')
    const item = {
      name,
      type: typeCodeOf(rawType),
      host: attr(attrs, 'Host', 'HostName', 'Server', 'IP'),
      port: toPort(attr(attrs, 'Port')),
      username: attr(attrs, 'UserName', 'Username', 'User'),
      database: attr(attrs, 'Database', 'DatabaseName', 'Schema', 'SID', 'ServiceName'),
      filePath: /sqlite/i.test(rawType) ? sqliteFile : '',
      group: '',
      note: `Navicat · ${rawType || '—'}`,
      pwd: password ? (looksEncrypted(password) ? PWD_ENCRYPTED : PWD_INCLUDED) : PWD_MISSING
    }
    // 只有"看起来不是密文"的口令才带过去；密文一律留空，避免把乱码写进连接
    if (item.pwd === PWD_INCLUDED) item.password = password
    // Oracle 的"库名"其实是服务名/SID，Navicat 里也可能叫 Database；SSH 隧道一并带过来
    const sshHost = attr(attrs, 'SshHost', 'SSHHost')
    if (sshHost) {
      item.sshEnabled = true
      item.sshHost = sshHost
      item.sshPort = toPort(attr(attrs, 'SshPort', 'SSHPort')) || 22
      item.sshUser = attr(attrs, 'SshUserName', 'SSHUserName', 'SshUser')
    }
    out.push(item)
  }
  return out
}

/* ============================ DBeaver ============================ */

function mapDbeaverOne(raw) {
  const cfg = raw.configuration && typeof raw.configuration === 'object' ? raw.configuration : {}
  const url = cfg.url || cfg.jdbcUrl || ''
  const fromUrl = splitJdbcUrl(url)
  const provider = raw.provider || ''
  const driver = raw.driver || ''
  let type = typeCodeOf(provider) || typeCodeOf(driver) || typeCodeOfUrl(url)
  const isSqlite = type === 'SQLITE'
  const password = cfg.password || ''
  return {
    name: String(raw.name || provider || driver || '').trim(),
    type,
    host: cfg.host || fromUrl.host || '',
    port: toPort(cfg.port || fromUrl.port),
    username: cfg.user || cfg.username || '',
    database: cfg.database || fromUrl.database || '',
    // 文件型库的"DBeaver 数据库"就是文件路径；优先 configuration.database，再从 url 里取
    filePath: isSqlite || type === 'H2' ? (cfg.database || filePathOfUrl(url)) : '',
    group: '',
    note: 'DBeaver · ' + (driver || provider || '—'),
    pwd: password ? PWD_INCLUDED : PWD_MISSING,
    password
  }
}

function parseDbeaver(data) {
  const items = []
  if (data.connections && typeof data.connections === 'object' && !Array.isArray(data.connections)) {
    for (const value of Object.values(data.connections)) {
      // project 导出里每条连接的值是**一段 JSON 字符串**，不是对象
      if (typeof value === 'string') {
        try { items.push(JSON.parse(value)) } catch (e) { /* 跳过坏条目，不整份失败 */ }
      } else if (value && typeof value === 'object') {
        items.push(value)
      }
    }
  } else {
    items.push(data)
  }
  return items.filter((x) => x && typeof x === 'object').map(mapDbeaverOne).filter((x) => x.name)
}

/* ============================ 统一入口 ============================ */

/**
 * 解析一份连接文件。
 * @returns {{source: string, list: object[], warnings: string[]}|{error: string}}
 *   list 里每条都是 DBMind 的连接字段 + `pwd`（口令来源标记）
 */
export function parseImportFile(text, filename = '') {
  const source = detectSource(text)
  if (!source) return { error: t('ci.unknownFormat') }
  let data = null
  let list = []
  if (source === 'navicat') {
    list = parseNavicat(text) || []
  } else {
    try {
      data = JSON.parse(String(text || '').replace(/^\uFEFF/, ''))
    } catch (e) {
      return { error: t('ci.badJson') }
    }
    list = source === 'dbmind' ? parseDbmind(data) : parseDbeaver(data)
  }
  if (!list.length) return { error: t('ci.noConnInFile') }

  const warnings = []
  const unknownType = list.filter((x) => !x.type).length
  if (unknownType) warnings.push(t('ci.warnUnknownType', { n: unknownType }))
  // 三种口令状态分开报：加密串是"有但解不开"，与"压根没带"是两回事，混成一条会误导
  const encrypted = list.filter((x) => x.pwd === PWD_ENCRYPTED).length
  if (encrypted) warnings.push(t('ci.warnNavicatPwd', { n: encrypted }))
  const missing = list.filter((x) => x.pwd === PWD_MISSING).length
  if (missing) warnings.push(t('ci.warnNoPwd', { n: missing }))
  return { source, list, warnings, name: filename }
}
