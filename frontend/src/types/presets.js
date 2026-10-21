/**
 * 各类型「连接选项」字段的公共片段。
 *
 * 原则（2026-10 收敛）：**表单里只留用户看得懂、且真的需要他自己定的东西**。
 *   * 连接 / 读取超时 —— 能理解，也确实有人要调；
 *   * 结构选择 —— Oracle 服务名/SID、ClickHouse http/https，
 *     它们改的是 URL 的形状，一句默认值概括不了；
 *   * 其余驱动细节（SSL、字符集、时区、公钥检索、批量改写、零日期、多语句、压缩、
 *     HTTP 实现、类型映射、TCP 保活、Unicode 发送、结果缓冲、取数方式、多子网…）
 *     一律**写进各类型的 urlTemplate**，表单不再暴露 —— 这些词用户看不懂，
 *     摆在那里只会让人犹豫"要不要改"，而答案永远是"不用"。
 *
 *     模板改动必须**两处逐字一致**：
 *       src/types/<type>.js           —— 弹窗里的 URL 预览（jdbc.js 用它拼）
 *       plugins/connection-types/<type>.yaml —— 真实连接用的后端模板
 *     要临时改某一项的，走「附加参数」（用户自己负责的逃生口）。
 *
 * 字段形状（ConnectionDialog 按它渲染、jdbc.js 按它拼 URL、保存时按它决定去向）：
 *   key       表单里的键（存在 form.opts 里）
 *   param     真实交给驱动的那条连接的属性名；有 param ⇒ 保存进 extra.params（会生效）
 *             没有 param ⇒ 结构化选项，保存进 extra.options（只改 URL 形状，如 Oracle SID）
 *   kind      select | text | number
 *   labelKey  文案键（zh-CN/en-US 都有）
 *   default   **最优默认值**：开箱即用、坑最少的那一档，用户不动也能连上并正常用
 *   options   [{ value, label }] 或 [{ value, labelKey }]
 *   tipKey    一句话说明，**只在标签问号里悬浮显示**，不占版面
 *   adv       true = 放在「高级选项」页签，false/缺省 = 放在「基本配置」
 *   beforeDatabase  true = 在基本配置里排到「库名」字段**上面**（Oracle 连接方式：
 *             它决定下面那一格叫服务名还是 SID，也改 URL 语法）
 *
 * 删掉某一项时，**记得在类型定义里加 `dropParams`**：老连接当年真的把默认值写进了
 * params，不丢的话它们会以「附加参数」的形态复活（看着像用户自己配的）。
 *
 * 命名约定：`cd.o.*` 是选项文案。加新类型时优先复用这里的片段。
 */

/**
 * MySQL 家族（MySQL / MariaDB / Doris）。
 *
 * 只留两个超时。SSL / 字符集 / 时区 / 公钥检索 / 批量改写 / 零日期 / 多语句
 * 全部落到各类型模板里，默认值见 src/types/mysql.js 的注释。
 */
export const mysqlFamilyOptions = () => [
  {
    // 连接超时默认 10 秒（10000 ms）：主机不可达时不要一直转圈
    key: 'connectTimeout', param: 'connectTimeout', kind: 'number', labelKey: 'cd.o.connectTimeoutMs',
    default: '10000', adv: true, tipKey: 'cd.o.tip.connectTimeoutMs'
  },
  {
    // 读取超时默认 600 秒（600000 ms），**不再是 0（永不超时）**：
    // 0 时驱动会无限等一个可能永远不来的响应 —— 目标端把连接半开（RDS 的 wait_timeout
    // 踢连接、中间设备静默丢包）后，show processlist 什么都没有、也不报错，
    // 表现就是同步/查询「卡住、不写也不失败」。10 分钟足够长查询，又能让卡死自己暴露出来。
    key: 'socketTimeout', param: 'socketTimeout', kind: 'number', labelKey: 'cd.o.readTimeoutMs',
    default: '600000', adv: true, tipKey: 'cd.o.tip.readTimeoutMs'
  }
]

/**
 * PostgreSQL 家族（PostgreSQL / KingbaseES）。
 *
 * sslmode=prefer / reWriteBatchedInserts / stringtype=unspecified / tcpKeepAlive
 * 都写进了模板；这里只留超时。
 *
 * **单位**：pgjdbc 的 connectTimeout / socketTimeout 只认**秒**，而其它驱动的同类项
 * 都是毫秒。为了让"同样的字段、同样的提示"成立（也为了数值直观），表单统一按毫秒填，
 * 存参数时用 `scale: 0.001` 换算成秒 —— 换算在 jdbc.js 的 paramValueOf 里，
 * 预览与保存共用同一处。
 */
export const pgFamilyOptions = () => [
  {
    key: 'connectTimeout', param: 'connectTimeout', kind: 'number', labelKey: 'cd.o.connectTimeoutMs',
    default: '10000', scale: 0.001, adv: true, tipKey: 'cd.o.tip.connectTimeoutMs'
  },
  {
    // 600000 ms（表单口径）→ 存参换算成 600 秒。与其它类型同口径：默认不是「永不超时」
    key: 'socketTimeout', param: 'socketTimeout', kind: 'number', labelKey: 'cd.o.readTimeoutMs',
    default: '600000', scale: 0.001, adv: true, tipKey: 'cd.o.tip.readTimeoutMs'
  }
]

/** SQL Server：只留超时，其余（加密/信任证书/取数方式…）已在模板里 */
export const mssqlOptions = () => [
  {
    // mssql-jdbc 的 loginTimeout 只认秒 → 表单毫秒、存参换算（与 PG 同理）
    key: 'loginTimeout', param: 'loginTimeout', kind: 'number', labelKey: 'cd.o.connectTimeoutMs',
    default: '10000', scale: 0.001, adv: true, tipKey: 'cd.o.tip.connectTimeoutMs'
  },
  {
    // mssql-jdbc 的 socketTimeout 是毫秒 → 600000 = 600 秒（同其它类型口径）
    key: 'socketTimeout', param: 'socketTimeout', kind: 'number', labelKey: 'cd.o.readTimeoutMs',
    default: '600000', adv: true, tipKey: 'cd.o.tip.readTimeoutMs'
  }
]

/** Oracle（propStyle 为 none：thin URL 不吃查询参数，这里全部走驱动属性） */
export const oracleOptions = () => [
  {
    // 放在基本配置、且排在「库名」上方：它决定下面那一格叫「服务名」还是「SID」，
    // 也决定 URL 是 `@host:port/SERVICE` 还是 `@host:port:SID`
    key: 'oracleConnect', kind: 'select', labelKey: 'cd.o.connType', default: 'service',
    options: [{ value: 'service', labelKey: 'cd.o.service' }, { value: 'sid', labelKey: 'cd.o.sid' }],
    beforeDatabase: true, required: true, tipKey: 'cd.o.tip.connType'
  },
  {
    key: 'oracle.net.CONNECT_TIMEOUT', param: 'oracle.net.CONNECT_TIMEOUT', kind: 'number',
    labelKey: 'cd.o.connectTimeoutMs', default: '10000', adv: true,
    tipKey: 'cd.o.tip.connectTimeoutMs'
  },
  {
    // ojdbc 的 ReadTimeout 是毫秒 → 600000 = 600 秒（同其它类型口径）
    key: 'oracle.jdbc.ReadTimeout', param: 'oracle.jdbc.ReadTimeout', kind: 'number',
    labelKey: 'cd.o.readTimeoutMs', default: '600000', adv: true, tipKey: 'cd.o.tip.readTimeoutMs'
  }
]

/** ClickHouse：http/https 是结构选项（驱动按 URL 前缀区分），压缩与 HTTP 实现已在模板里 */
export const clickhouseOptions = () => [
  {
    key: 'chScheme', kind: 'select', labelKey: 'cd.o.scheme', default: 'http',
    options: [{ value: 'http', label: 'HTTP' }, { value: 'https', label: 'HTTPS' }],
    tipKey: 'cd.o.tip.scheme'
  },
  {
    key: 'connect_timeout', param: 'connect_timeout', kind: 'number', labelKey: 'cd.o.connectTimeoutMs',
    default: '10000', adv: true, tipKey: 'cd.o.tip.connectTimeoutMs'
  },
  {
    // clickhouse-jdbc 的 socket_timeout 是毫秒 → 600000 = 600 秒（同其它类型口径）
    key: 'socket_timeout', param: 'socket_timeout', kind: 'number', labelKey: 'cd.o.readTimeoutMs',
    default: '600000', adv: true, tipKey: 'cd.o.tip.readTimeoutMs'
  }
]

/** IBM DB2：只留连接超时；sslConnection / 服务端原文报错已在模板里 */
export const db2Options = () => [
  {
    // JCC 的 loginTimeout 只认秒 → 表单毫秒、存参换算
    key: 'loginTimeout', param: 'loginTimeout', kind: 'number', labelKey: 'cd.o.connectTimeoutMs',
    default: '10000', scale: 0.001, adv: true, tipKey: 'cd.o.tip.connectTimeoutMs'
  }
]

/** H2：AUTO_SERVER 这类引擎设置 DBMind 自己不用，表单不再暴露 */
export const h2Options = () => []
