/**
 * JDBC 连接串组装 —— 「新建数据源」弹窗里那条自动生成的 URL 的唯一实现。
 *
 * 为什么单独抽这一层
 * ------------------
 * 后端真正拿去连库的 URL 由三部分决定（见 crates/dbmind-core/src/agent.rs 的
 * `render_jdbc_url` 与 drivers/agent_driver.rs 的 `connect_params`）：
 *
 *   1. 类型模板 `urlTemplate` —— 就在 plugins/connection-types/<type>.yaml，
 *      构建时生成到 database-drivers.manifest.json；**本目录各类型的 urlTemplate 与它逐字一致**；
 *   2. 结构化选项 `extra.options` —— 只改 URL 形状、不是驱动属性的那几项
 *      （Oracle 服务名/SID、ClickHouse http/https、H2 的 ;MODE=…）；
 *   3. 驱动参数 `extra.params` —— 以 JDBC Properties 交给驱动，效果与写在 URL 上等价。
 *
 * 预览必须与真实连接一致：用户就是照着这一行判断「连的是哪个库、SSL 开没开」。
 * 预览与后端不一致时，排查会全部走错方向。所以：
 *   - 模板改动 → 同步 yaml；
 *   - 结构化选项改动 → 同步 agent.rs 的对应分支（下文每处都标了名字）。
 */

/** 只转义会破坏 URL 结构的字符：`:` `/` `.` `$` 等必须原样保留（服务名、文件路径里都有） */
export const encVal = (v) =>
  String(v ?? '')
    .replace(/%/g, '%25')
    .replace(/&/g, '%26')
    .replace(/=/g, '%3D')
    .replace(/\?/g, '%3F')
    .replace(/#/g, '%23')
    .replace(/\s/g, (m) => encodeURIComponent(m))

/** 读结构化选项（null / undefined / 未设置 都算空串） */
export const optOf = (f, key) => {
  const v = f?.opts?.[key]
  return v == null ? '' : String(v)
}

/** 读一条用户手填的附加参数（只看 form.params，不含类型选项） */
export const paramOf = (f, name) => {
  const hit = (f?.params || []).find((p) => String(p?.name ?? '').trim() === name)
  return hit && hit.value != null ? String(hit.value) : ''
}

const escRe = (s) => String(s).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/** URL 里已经出现过的参数名（`?a=b&c=d` 与 SQL Server / Derby / H2 的 `;a=b` 两种形状） */
const existingNames = (url) => {
  const names = new Set()
  const q = url.indexOf('?')
  if (q >= 0) {
    for (const kv of url.slice(q + 1).split('&')) {
      const k = kv.split('=')[0]
      if (k) names.add(k)
    }
  }
  for (const m of url.matchAll(/;([A-Za-z_$][\w$.-]*)=/g)) names.add(m[1])
  return names
}

/** 同名参数就地改值：URL 里出现两个同名参数时谁生效取决于驱动，不能靠猜 */
const setQueryValue = (url, name, value) => {
  const q = url.indexOf('?')
  if (q < 0) return url
  const parts = url.slice(q + 1).split('&').map((kv) => {
    const eq = kv.indexOf('=')
    const k = eq < 0 ? kv : kv.slice(0, eq)
    return k === name ? k + '=' + value : kv
  })
  return url.slice(0, q + 1) + parts.join('&')
}
const setSemiValue = (url, name, value) =>
  url.replace(new RegExp('(;' + escRe(name) + '=)[^;]*'), (_m, head) => head + value)
const setDb2Value = (url, name, value) =>
  url.replace(new RegExp('([:;]' + escRe(name) + '=)[^;]*'), (_m, head) => head + value)

/**
 * 选项值 → 真正交给驱动的参数值。
 *
 * 默认 1:1；`scale` 用于单位不一致的驱动（pgjdbc / mssql-jdbc / JCC 的超时只认秒，
 * 而表单与其它驱动都按毫秒）。**预览与保存都必须走这里**，否则用户看到的 URL
 * 和实际连上去的不是一个数。
 */
export const paramValueOf = (o, f) => {
  const raw = String(optOf(f, o.key)).trim()
  if (!raw) return ''
  if (!o?.scale || o.scale === 1) return raw
  const n = Number(raw)
  return Number.isFinite(n) ? String(Math.round(n * o.scale)) : raw
}

/** 反算：已存参数值 → 表单里显示的值（splitLoadedParams 用） */
export const optionValueFromParam = (o, value) => {
  const raw = String(value == null ? '' : value).trim()
  if (!raw || !o?.scale || o.scale === 1) return raw
  const n = Number(raw)
  return Number.isFinite(n) ? String(Math.round(n / o.scale)) : raw
}

/**
 * 生效的驱动参数（按「类型声明的顺序 → 用户手填的顺序」，跳过空值）。
 * 有 param 的类型选项优先于同名的附加参数 —— 界面上看得见的那个才算数。
 */
export const effectiveProps = (def, f) => {
  const declared = []
  for (const o of def?.options || []) {
    if (!o.param) continue
    const v = paramValueOf(o, f)
    if (v) declared.push([o.param, v])
  }
  const seen = new Set(declared.map(([k]) => k))
  const extra = []
  for (const p of f?.params || []) {
    const name = String(p?.name ?? '').trim()
    const value = String(p?.value ?? '').trim()
    if (!name || !value || seen.has(name)) continue
    seen.add(name)
    extra.push([name, value])
  }
  return declared.concat(extra)
}

/** 模板占位符替换（与后端 render_jdbc_url 的替换表一致） */
const fill = (tpl, f, def) => {
  const host = String(f?.host ?? '').trim()
  const file = String(f?.filePath ?? '').trim().replace(/\\/g, '/')
  const port = f?.port || def?.defaultPort || ''
  const db = String(f?.database ?? '').trim()
  return tpl
    .split('{host}').join(host)
    .split('{port}').join(String(port))
    .split('{database}').join(encVal(db))
    .split('{filePath}').join(file)
}

/**
 * 只能写进 URL 的引擎设置（H2 的 `AUTO_SERVER=TRUE` 之类）。
 *
 * 它们是 URL 设置而不是连接属性，所以既要在预览里拼出来，也要在保存时整段交给后端
 * （`options.urlSuffix`，见 agent.rs 的 render_jdbc_url）—— 两处共用这一个实现。
 */
export const urlSuffixOf = (def, f) => {
  const parts = []
  if (def?.code === 'H2' && optOf(f, 'h2AutoServer') === 'true') parts.push('AUTO_SERVER=TRUE')
  return parts.join(';')
}

/**
 * 生成连接串。
 *
 * `propStyle` 决定「驱动参数接在 URL 哪儿」——只是**展示**需要，真实连接里它们走 Properties：
 *   query（默认，`?a=b&c=d`）/ semicolon（SQL Server、Derby、H2 的 `;a=b`）/
 *   db2（`jdbc:db2://h:p/db:sslConnection=true;`）/ none（Oracle：thin URL 不吃参数）
 */
export const buildUrlFor = (def, f) => {
  const tpl0 = String(def?.urlTemplate || '')
  if (!tpl0 || !f) return ''

  const host = String(f.host ?? '').trim()
  const file = String(f.filePath ?? '').trim()
  if (tpl0.includes('{host}') && !host) return ''
  if (tpl0.includes('{filePath}') && !file) return ''

  let tpl = tpl0

  // Oracle：服务名（默认）用 `@host:port/service`，SID 用 `@host:port:SID`
  // （后端 render_jdbc_url 的 oracle 分支照同一规则换 `/` 为 `:`）
  if (def.code === 'ORACLE' && optOf(f, 'oracleConnect') === 'sid') {
    tpl = tpl.split('/{database}').join(':{database}')
  }

  // ClickHouse over TLS：驱动按 URL 前缀区分 http / https
  if (def.code === 'CLICKHOUSE' && optOf(f, 'chScheme') === 'https') {
    tpl = tpl.replace('jdbc:clickhouse://', 'jdbc:clickhouse:https://')
  }

  // 库名没填时不要留下孤零零的 `/`：`jdbc:mysql://h:3306/` → `jdbc:mysql://h:3306`。
  // MySQL / ClickHouse 这些驱动允许"不指定默认库"，裸主机就是合法写法；拖着个斜杠
  // 反而像少写了库名。**pgjdbc 系（PG / Kingbase）必须保留**（`keepEmptyDbSlash`）：
  // 它要求 URL 有路径段，`jdbc:postgresql://h:5432?x=y` 会报 Unable to parse URL。
  // 后端 render_jdbc_url 按同一规则处理。
  if (!def?.keepEmptyDbSlash && !String(f?.database ?? '').trim()) {
    tpl = tpl.split('/{database}').join('')
  }

  let url = fill(tpl, f, def)

  // 只能写进 URL 的引擎设置（H2 的 MODE / AUTO_SERVER）
  const suffix = urlSuffixOf(def, f)
  if (suffix) url += (url.endsWith(';') ? '' : ';') + suffix

  const style = def.propStyle || 'query'
  if (style === 'none') return url

  const names = existingNames(url)
  for (const [name, value] of effectiveProps(def, f)) {
    if (style === 'semicolon') {
      url = names.has(name) ? setSemiValue(url, name, value) : url + ';' + name + '=' + value
    } else if (style === 'db2') {
      if (names.has(name)) {
        url = setDb2Value(url, name, value)
      } else {
        const hasProps = url.indexOf(':', url.lastIndexOf('/')) > 0
        url += (hasProps ? '' : ':') + name + '=' + value + ';'
      }
    } else if (names.has(name)) {
      url = setQueryValue(url, name, value)
    } else {
      url += (url.includes('?') ? '&' : '?') + name + '=' + value
    }
    names.add(name)
  }
  return url
}
