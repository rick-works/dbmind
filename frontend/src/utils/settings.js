// 应用本地设置（与设置页默认值保持唯一来源）
// 说明：编辑器设置、查询设置、通知设置保存在 localStorage；
// 驱动镜像配置由后端持久化（settings.json），前端通过 API 读写。

export const editorDefaults = {
  fontSize: 14, tabSize: 4, wordWrap: true, lineNumbers: true, minimap: false, autoSave: false,
  // SQL 格式化全局默认：关键字排版规则之外的统一外观（规则未覆盖的关键字/大小写均取此处默认）。
  // 方言不在此配置——查询编辑器按连接类型自动识别，设置预览固定用标准 SQL。
  sqlKeywordCase: 'upper',          // 关键字大小写：upper=大写 | lower=小写 | preserve=保持原样
  sqlIdentifierCase: 'preserve',    // 标识符大小写
  sqlDataTypeCase: 'preserve',      // 数据类型大小写
  sqlFunctionCase: 'preserve',      // 函数名大小写
  sqlTabWidth: 2,                   // 缩进宽度（空格个数）
  sqlUseTabs: false,                // 是否用 Tab 缩进（开启后忽略缩进宽度）
  sqlExpressionWidth: 50,           // 表达式超过该宽度自动折行
  sqlLinesBetweenQueries: 2,        // 多条语句之间的空行数
  sqlLogicalOperatorNewline: 'before', // 逻辑运算符 AND/OR 换行位置：before=前换行 | after=后换行
  sqlNewlineBeforeSemicolon: false, // 分号是否独占一行
  sqlDenseOperators: false,         // 运算符两侧是否不加空格（a=b+1）
  // 关键字排版规则：每条 { kw: 关键字/短语, cs: upper=大写 | lower=小写 | keep=保持原样, ly: none=内联 | clause=前换行 | join=前换行+缩进 | afterBreak=后换行 | afterBreakIndent=后换行+缩进 | setop=前后各换行 }
  // 不配置任何规则时，所有关键字均按上述全局默认排版
  sqlKeywordRules: []
}
export const queryDefaults = { pageSize: 200, confirmDanger: true }
export const notifyDefaults = { success: true, autoClose: true }

const readJSON = (key) => {
  try {
    const raw = localStorage.getItem(key)
    return raw ? { ...JSON.parse(raw) } : {}
  } catch { return {} }
}

// v2 规则 mode → v3 排版 ly 的映射（mode 语义：所有关键字统一按关键字处理即大写）
const LEGACY_MODE_LY = { kw: 'none', clause: 'clause', join: 'join', setop: 'setop' }
const LY_VALUES = ['none', 'clause', 'join', 'afterBreak', 'afterBreakIndent', 'setop']

const normalizeRules = (rules) => {
  const seen = new Set()
  return (Array.isArray(rules) ? rules : []).reduce((out, r) => {
    if (!r || typeof r !== 'object') return out
    const kw = String(r.kw || '').trim()
    if (!kw) return out
    const key = kw.toUpperCase()
    if (seen.has(key)) return out
    seen.add(key)
    const ly = LY_VALUES.includes(r.ly) ? r.ly : (LEGACY_MODE_LY[r.mode] || 'none')
    out.push({ kw, cs: r.cs === 'keep' ? 'keep' : (r.cs === 'lower' ? 'lower' : 'upper'), ly })
    return out
  }, [])
}

// 迁移：把历史缓存就地整理为 v3 结构（sqlKeywordRules 逐条归一化 + 清理已下线的 sqlLanguage 残留）
export const migrateEditor = (raw) => {
  if (!raw || typeof raw !== 'object') return raw
  let changed = false
  // v1 遗留：一个关键字串 + 一种排版方式 → 展开成规则数组
  const hasLegacy = raw.sqlCustomKeyword !== undefined || raw.sqlCustomKeywordMode !== undefined
  if (hasLegacy) {
    const mode = ['join', 'setop'].includes(raw.sqlCustomKeywordMode)
      ? raw.sqlCustomKeywordMode
      : (raw.sqlCustomKeywordMode === 'kw' ? 'kw' : 'clause')
    const words = String(raw.sqlCustomKeyword || '')
      .split(/[,，;；\n\r]+/)
      .map((s) => s.trim())
      .filter(Boolean)
    if (words.length) {
      raw.sqlKeywordRules = words.map((kw) => ({ kw, cs: 'upper', ly: LEGACY_MODE_LY[mode] }))
    }
    delete raw.sqlCustomKeyword
    delete raw.sqlCustomKeywordMode
    changed = true
  }
  // v2/v3 规则统一归一化
  if (raw.sqlKeywordRules !== undefined) {
    const before = JSON.stringify(raw.sqlKeywordRules)
    raw.sqlKeywordRules = normalizeRules(raw.sqlKeywordRules)
    if (before !== JSON.stringify(raw.sqlKeywordRules)) changed = true
  }
  // 旧全局「SQL 方言」已下线：格式化方言改由连接类型自动识别，清理旧缓存中的残留值
  if ('sqlLanguage' in raw) { delete raw.sqlLanguage; changed = true }
  // 默认字号 12 → 14：只把「恰好停在旧默认值」的用户带过来；
  // 自己调过字号（10 / 16 / 18…）的一律保留 —— 改默认不该覆盖用户的选择。
  if (raw.fontSize === 12) { raw.fontSize = 14; changed = true }
  if (changed) { try { localStorage.setItem('dbmind_editor', JSON.stringify(raw)) } catch { /* 忽略写失败 */ } }
  return raw
}

export const getEditorSettings = () => {
  const stored = readJSON('dbmind_editor')
  migrateEditor(stored)
  return { ...editorDefaults, ...stored }
}
export const getQuerySettings = () => ({ ...queryDefaults, ...readJSON('dbmind_query') })
export const getNotifySettings = () => ({ ...notifyDefaults, ...readJSON('dbmind_notify') })
