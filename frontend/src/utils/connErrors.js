/**
 * 连接失败 → **可操作的排查建议**。
 *
 * 为什么要它：驱动抛出来的原文（`Communications link failure` / `28000` /
 * `PKIX path building failed`）对用户等于没说 —— 他要的是"下一步查什么"。
 * 这里按关键字把常见错因翻译成 2~3 条能立刻动手的事，并尽量指向**本应用里已有的开关**
 * （连接表单「高级选项」的常用参数预设、SSH 隧道页、驱动状态），而不是泛泛地说"检查配置"。
 *
 * 纯函数、无副作用：只影响界面上显示什么，不改任何连接参数。
 */

/**
 * 每条规则：命中 `test` 就给出这些建议。顺序即优先级（先具体的、后通用的）。
 *
 * ⚠️ `test` 是**正则**、不是给人看的文案 —— 它去匹配数据库/驱动返回的原文，
 * 那里可能夹带中文（本地化过的报错），所以正则里的中文要保留、不要翻译。
 * `hints` 里存的是**字典键**，取用时才过 t()：这是模块级静态数组，
 * 直接把中文写在这里，切语言不会跟着变。
 */
import { t } from './i18n'

const RULES = [
  {
    test: /public\s*key\s*retrieval|PublicKeyRetrieval/i,
    hints: [
      'ce.publicKey'
    ]
  },
  {
    test: /PKIX|certificate|SSL|TLS|trustAll|self.signed|证书/i,
    hints: [
      'ce.ssl1',
      'ce.ssl2',
      'ce.ssl3'
    ]
  },
  {
    test: /serverTimezone|time\s*zone|时区|The server time zone/i,
    hints: ['ce.timezone']
  },
  {
    test: /Access denied|1045|28000|password authentication|authentication failed|认证失败|口令|密码错误|login failed for user/i,
    hints: [
      'ce.auth1',
      'ce.auth2',
      'ce.auth3'
    ]
  },
  {
    test: /Unknown database|database .* does not exist|不存在|invalid database|库不存在/i,
    hints: ['ce.database']
  },
  {
    test: /No suitable driver|ClassNotFound|driver|驱动|agent|jar|Agent/i,
    hints: [
      'ce.driver1',
      'ce.driver2',
      'ce.driver3'
    ]
  },
  {
    test: /TNS|ORA-\d+|listener|监听/i,
    hints: [
      'ce.oracle1',
      'ce.oracle2'
    ]
  },
  {
    test: /SSH|Auth fail|private ?key|跳板/i,
    hints: [
      'ce.ssh1',
      'ce.ssh2'
    ]
  },
  {
    test: /Connection refused|拒绝|Communications link failure|无法连接|No route to host|UnknownHost/i,
    hints: [
      'ce.refused1',
      'ce.refused2',
      'ce.refused3'
    ]
  },
  {
    test: /timed? ?out|超时|timeout/i,
    hints: [
      'ce.timeout1',
      'ce.timeout2'
    ]
  }
]

/** 与具体错误无关、但值得一并检查的事项（放在所有命中项之后）。 */
const FALLBACK_HINTS = [
  'ce.fallback1',
  'ce.fallback2'
]

/**
 * 根据错误原文给出建议数组（最多 4 条，保持可读）。
 *
 * @param {string} message 后端 / 驱动返回的错误原文
 * @param {object} [form] 连接表单，用来判断是否开了 SSH（影响建议的侧重点）
 * @returns {string[]}
 */
export function connErrorHints(message, form = {}) {
  const text = String(message || '')
  const hints = []
  for (const rule of RULES) {
    if (rule.test.test(text)) {
      for (const hint of rule.hints) {
        const text = t(hint)
        if (!hints.includes(text)) hints.push(text)
      }
    }
  }
  if (form && form.sshEnabled) {
    hints.push(t('ce.sshOn'))
  }
  for (const hint of FALLBACK_HINTS) {
    const text = t(hint)
    if (!hints.includes(text)) hints.push(text)
  }
  return hints.slice(0, 4)
}

/** 组装成可直接给弹窗用的纯文本（带项目符号）。 */
export function connErrorHintText(message, form = {}) {
  const hints = connErrorHints(message, form)
  if (!hints.length) return ''
  return hints.map((h, i) => (i + 1) + '. ' + h).join('\n')
}
