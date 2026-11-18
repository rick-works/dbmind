/**
 * 达梦 DM 类型定义
 *
 * 模板里不再拼 `?schema=…`：达梦的连接串格式是 `jdbc:dm://host:port/库名`，
 * 旧实现把库名当 schema 参数塞进查询串，与后端 YAML 不一致（预览和真实连接对不上）。
 */
export default {
  code: 'DM',
  // 上游兼容接口的模块路径段：与 plugins/connection-types/dm.yaml 的 dialect 派生值一致（guard 校验）
  module: 'oracle',
  label: 'DM', labelKey: 'tdef.dm',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
  // 「库」这一层对达梦是噪音：与 Oracle 同一口径（见 types/index.js 的 hideDbLevelOf）——
  // 达梦一个实例就是一个库，树上的第一层是**模式**（DEMO / SYS / SYSDBA…）。
  // 不标这一位的话，连接节点展开后会走到「库」那条路：达梦的 `/databases` 没有库清单可列
  // （连接串里的库名本身也可以不填），后端只能回占位名 `(default)`，树里就只剩一行
  // 「未取到库清单」—— 用户看到的就是「达梦节点展不开」。
  hideDbLevel: true,
  defaultSchema: '',
  quoteStyle: 'DOUBLE_QUOTE',
  logo: 'dm.svg',
  fileType: false,
  defaultPort: 5236,
  databasePh: 'cd.dbPhOptional',
  urlTemplate: 'jdbc:dm://{host}:{port}/{database}',
  // 达梦驱动认的选项很少（compatibleMode 与库端初始化参数必须一致，配错反而解析出错），
  // 没有「普适最优值」的就不放进表单 —— 少一个能被配坏的开关。
  options: [],
  // 达梦建库依赖 dminit / 管理工具，SQL 层不提供 CREATE DATABASE，标记为不支持
  createDatabaseSql() {
    return ''
  },
  // 无「查询建库语句」接口，右键菜单不提供
  showDbDdlSql() {
    return null
  }
}
