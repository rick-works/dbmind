import { t } from './i18n'
// 轻量安全的 markdown 渲染：先整体 HTML 转义，再拼装少量受控标签，杜绝注入。
// 供 AI 回复等不可信内容渲染使用（AI 面板 / SQL 查询 AI 区 / 对象右键“AI 解释”）。
// 与 marked 等完整解析器不同，这里不解析原始 HTML，任何输入都只会被当成文本来处理。
//
// 支持：围栏代码块、行内代码、标题、粗体/斜体、无序/有序列表、
//       表格、引用、分隔线、链接（仅 http/https）。

/** HTML 转义（所有文本进入标签前都必须先走这里） */
const escapeHtml = (s) =>
  String(s == null ? '' : s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')

/** 行内元素：行内代码 / 链接 / 粗体 / 斜体（输入已是纯文本，这里只做转义+拼标签） */
const renderInline = (text) => {
  // 先抽出行内代码，避免其中的 * _ [ ] 被后续规则误伤
  const codes = []
  let s = escapeHtml(text).replace(/`([^`]+)`/g, (m, c) => {
    codes.push(c)
    return `\u0000${codes.length - 1}\u0000`
  })
  // 链接：仅允许 http/https，其余一律当普通文本
  s = s.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g,
    '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>')
  // 粗体 / 斜体（斜体要求内容首尾非空白，避免 `price * qty * 1.1` 这类被误判）
  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
  s = s.replace(/(^|[^*\w])\*(\S[^*\n]*?)\*(?!\*)/g, '$1<em>$2</em>')
  s = s.replace(/(^|[^_\w])_(\S[^_\n]*?)_(?!_)/g, '$1<em>$2</em>')
  // 还原行内代码
  s = s.replace(/\u0000(\d+)\u0000/g, (m, idx) => `<code>${codes[Number(idx)]}</code>`)
  return s
}

/** 表格行 → 单元格数组 */
const splitRow = (line) => line.trim().replace(/^\||\|$/g, '').split('|').map((c) => c.trim())

const RE_FENCE = /^\s*```(\w*)\s*$/

/**
 * SQL 代码块的操作按钮用**内联 SVG**（不能用 Vue 图标组件：v-html 出来的内容
 * 不会参与组件编译）。图形取自 Feather（MIT），描边跟随 currentColor，
 * 尺寸/颜色交给 CSS 控制；这里只写死受控字符串，与用户输入无关。
 */
const SVG_ATTR = 'viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"'
const ICON_INSERT =
  `<svg ${SVG_ATTR}><path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4"></path>` +
  '<polyline points="10 17 15 12 10 7"></polyline><line x1="15" y1="12" x2="3" y2="12"></line></svg>'
const ICON_COPY =
  `<svg ${SVG_ATTR}><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>` +
  '<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>'
/** 执行计划：放大镜（"先看看它打算怎么跑"） */
const ICON_PLAN =
  `<svg ${SVG_ATTR}><circle cx="11" cy="11" r="7"></circle><line x1="16.5" y1="16.5" x2="21" y2="21"></line></svg>`
/** 试跑：播放三角（"真跑一次，取前 100 行"） */
const ICON_RUN =
  `<svg ${SVG_ATTR}><polygon points="6 4 20 12 6 20 6 4"></polygon></svg>`
// 视为 SQL 的围栏语言：sql / 以 sql 结尾（mysql、tsql、plsql…）/ 常见方言别名
const SQL_LANGS = ['postgres', 'postgresql', 'mysql', 'mariadb', 'oracle', 'hive', 'spark',
  'clickhouse', 'duckdb', 'sqlite', 'db2', 'snowflake', 'bigquery', 'trino', 'presto', 'kingbase', 'dm']
/** 代码块语言标记是否算 SQL */
export const isSqlLang = (lang) => {
  const l = String(lang || '').toLowerCase()
  if (!l) return false
  return l === 'sql' || l.endsWith('sql') || SQL_LANGS.includes(l)
}
const RE_HEADING = /^(#{1,6})\s+(.*)$/
const RE_HR = /^\s*([-*_])(\s*\1){2,}\s*$/
const RE_QUOTE = /^\s*>\s?/
const RE_UL = /^\s*[-*+]\s+/
const RE_OL = /^\s*\d+\.\s+/
const RE_TABLE_SEP = /^\s*\|?[\s:|-]+\|[\s:|-]*$/

/**
 * 渲染 markdown → HTML 字符串。
 * 采用「逐行状态机」解析块级元素，行内元素交给 renderInline。
 *
 * opts.sqlActions=true 时，**顶层**每个 SQL 代码块会额外带一组操作按钮
 * （插入编辑器 / 复制），按钮用 data-sql-act + data-sql-idx 标注，
 * 由宿主容器做事件委托（v-html 无法绑定 Vue 事件）。
 * 索引与 extractSqlBlocks() 的返回顺序严格一致。
 */
export const renderMarkdown = (md, opts = {}) => {
  const withSqlActions = !!opts.sqlActions
  const src = (md == null ? '' : String(md)).replace(/\r\n?/g, '\n')
  const lines = src.split('\n')
  const out = []
  const para = []
  let codeIdx = 0

  const flushPara = () => {
    if (!para.length) return
    out.push('<p>' + para.map(renderInline).join('<br>') + '</p>')
    para.length = 0
  }

  let i = 0
  while (i < lines.length) {
    const line = lines[i]

    // 围栏代码块 ```lang ... ```
    const fence = line.match(RE_FENCE)
    if (fence) {
      flushPara()
      const lang = fence[1]
      const buf = []
      i++
      while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) { buf.push(lines[i]); i++ }
      i++ // 跳过结束围栏
      const code =
        '<pre><code' + (lang ? ` class="lang-${escapeHtml(lang)}"` : '') + '>' +
        escapeHtml(buf.join('\n')) + '</code></pre>'
      if (withSqlActions) {
        // 所有代码块都给「复制」；SQL 类额外给「试跑 / 执行计划 / 插入并执行」——
        // 索引按「全部代码块」递增，与 extractCodeBlocks 一一对应
        const acts =
          (isSqlLang(lang)
            ? `<button type="button" class="md-code-act" data-sql-act="run" data-sql-idx="${codeIdx}" title="${escapeHtml(t('mdk.runTip'))}" aria-label="${escapeHtml(t('mdk.run'))}">${ICON_RUN}</button>`
              + `<button type="button" class="md-code-act" data-sql-act="plan" data-sql-idx="${codeIdx}" title="${escapeHtml(t('mdk.planTip'))}" aria-label="${escapeHtml(t('mdk.planTip'))}">${ICON_PLAN}</button>`
              + `<button type="button" class="md-code-act" data-sql-act="insert" data-sql-idx="${codeIdx}" title="${escapeHtml(t('mdk.insertRun'))}" aria-label="${escapeHtml(t('mdk.insertRun'))}">${ICON_INSERT}</button>`
            : '') +
          `<button type="button" class="md-code-act" data-sql-act="copy" data-sql-idx="${codeIdx}" title="${escapeHtml(t('mdk.copy'))}" aria-label="${escapeHtml(t('mdk.copy'))}">${ICON_COPY}</button>`
        out.push('<div class="md-code">' + code + '<div class="md-code-acts">' + acts + '</div></div>')
        codeIdx++
      } else {
        out.push(code)
      }
      continue
    }

    // 空行：结束当前段落
    if (!line.trim()) { flushPara(); i++; continue }

    // 分隔线
    if (RE_HR.test(line)) { flushPara(); out.push('<hr>'); i++; continue }

    // 标题（限制到 h2..h6，避免 AI 输出 # 时字大得离谱）
    const h = line.match(RE_HEADING)
    if (h) {
      flushPara()
      const lv = Math.min(h[1].length + 1, 6)
      out.push(`<h${lv}>${renderInline(h[2])}</h${lv}>`)
      i++
      continue
    }

    // 表格：当前行含 |，下一行是分隔行
    if (line.includes('|') && i + 1 < lines.length
        && RE_TABLE_SEP.test(lines[i + 1]) && lines[i + 1].includes('-')) {
      flushPara()
      const head = splitRow(line)
      i += 2
      const rows = []
      while (i < lines.length && lines[i].trim() && lines[i].includes('|')) {
        rows.push(splitRow(lines[i]))
        i++
      }
      out.push(
        '<table><thead><tr>' + head.map((c) => `<th>${renderInline(c)}</th>`).join('') +
        '</tr></thead><tbody>' +
        rows.map((r) => '<tr>' + r.map((c) => `<td>${renderInline(c)}</td>`).join('') + '</tr>').join('') +
        '</tbody></table>'
      )
      continue
    }

    // 引用
    if (RE_QUOTE.test(line)) {
      flushPara()
      const buf = []
      while (i < lines.length && RE_QUOTE.test(lines[i])) {
        buf.push(lines[i].replace(RE_QUOTE, ''))
        i++
      }
      out.push('<blockquote>' + renderMarkdown(buf.join('\n')) + '</blockquote>')
      continue
    }

    // 无序列表
    if (RE_UL.test(line)) {
      flushPara()
      const buf = []
      while (i < lines.length) {
        if (RE_UL.test(lines[i])) {
          buf.push([lines[i].replace(RE_UL, '')])
          i++
        } else if (lines[i].trim() && /^\s+\S/.test(lines[i]) && buf.length) {
          // 列表项的续行（缩进但不是新的列表项）：并入上一项，避免一句被拆成两段
          buf[buf.length - 1].push(lines[i].trim())
          i++
        } else break
      }
      out.push('<ul>' + buf.map((parts) => `<li>${renderInline(parts.join(' '))}</li>`).join('') + '</ul>')
      continue
    }

    // 有序列表
    if (RE_OL.test(line)) {
      flushPara()
      const buf = []
      while (i < lines.length) {
        if (RE_OL.test(lines[i])) {
          buf.push([lines[i].replace(RE_OL, '')])
          i++
        } else if (lines[i].trim() && /^\s+\S/.test(lines[i]) && buf.length) {
          buf[buf.length - 1].push(lines[i].trim())
          i++
        } else break
      }
      out.push('<ol>' + buf.map((parts) => `<li>${renderInline(parts.join(' '))}</li>`).join('') + '</ol>')
      continue
    }

    // 普通段落行
    para.push(line.trim())
    i++
  }
  flushPara()
  return out.join('\n')
}

/**
 * 抽取 markdown 里**顶层**的所有围栏代码块（去掉围栏与首尾空白）。
 * 与 renderMarkdown(md, { sqlActions: true }) 右上角按钮的 data-sql-idx 一一对应：
 * 同一套围栏扫描规则、同样只看顶层（引用块内的代码块不计入）。
 * 返回 [{ lang, code }]，按钮委托按索引取用。
 */
export const extractCodeBlocks = (md) => {
  const lines = (md == null ? '' : String(md)).replace(/\r\n?/g, '\n').split('\n')
  const out = []
  let i = 0
  while (i < lines.length) {
    const fence = lines[i].match(RE_FENCE)
    if (!fence) { i++; continue }
    const lang = fence[1] || ''
    const buf = []
    i++
    while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) { buf.push(lines[i]); i++ }
    i++ // 跳过结束围栏
    out.push({ lang, code: buf.join('\n').trim() })
  }
  return out
}

/** 只取 SQL 类代码块的正文（用于「替换并执行」这类仅对 SQL 有意义的动作） */
export const extractSqlBlocks = (md) =>
  extractCodeBlocks(md).filter((b) => isSqlLang(b.lang)).map((b) => b.code)
