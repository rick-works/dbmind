/**
 * 达梦 DM 类型定义
 *
 * 模板里不再拼 `?schema=…`：达梦的连接串格式是 `jdbc:dm://host:port/库名`，
 * 旧实现把库名当 schema 参数塞进查询串，与后端 YAML 不一致（预览和真实连接对不上）。
 */
export default {
  code: 'DM',
  label: 'DM', labelKey: 'tdef.dm',
  category: 'RELATIONAL',
  noSql: false,
  schemaLevel: 'none',
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
