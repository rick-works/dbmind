/**
 * 安全拦截的**统一提示**：生产保护 / 只读连接 / AI 写入关闭。
 *
 * 为什么要统一：这类消息不是"某次操作失败"那么简单 —— 它自带一个**该怎么放开**的动作，
 * 所以用带按钮的**通知**（右上角，不挡正文），并在所有地方复用同一套文案与样式。
 * （用户反馈：「类似警告类的消息框，都在这个位置弹出吧，如果其他地方也有的话」。）
 *
 * 位置：Element 的通知默认就在右上角 —— 与任务中心、在线更新的提示同一处。
 */
import { h } from 'vue'
import { ElMessage, ElNotification } from 'element-plus'
import { t } from './i18n'

/**
 * 是不是"安全拦截"。
 *
 * 判据用后端那句话本身（固定中文，与界面语言无关），不依赖界面文案 ——
 * 前端拿到的 `error.message` 是拦截器从 `response.data.message` 改写来的，
 * 而安全错误的 payload 里 `errorCode` 是数字 0（见 dbmind-web 的 query_failure_json），
 * 靠码判不了，只能认这句话。
 */
export const isSafetyBlock = (detail) => {
  const s = String(detail || '')
  if (!s) return false
  return (
    /DBMIND-SAFETY-000[123]/.test(s) ||
    s.includes('生产保护') ||
    s.includes('被标记为只读') ||
    s.includes('只读连接')
  )
}

/** 直达「设置 → 安全与会话」（MainView 监听全局事件；首页 MCP 卡的「前往设置」用的是同一套） */
export const openSafetySettings = () => {
  window.dispatchEvent(new CustomEvent('dc-open-settings', { detail: { tab: 'safety' } }))
}

/**
 * 直达某个数据源的编辑弹窗（并停在「只读」那一项）——由 MainView 监听这个全局事件。
 * 通知渲染在 body 下，拿不到组件方法，所以跟「去设置」一样用事件回传。
 */
export const requestEditConn = (connId, focus) => {
  if (!connId) return
  window.dispatchEvent(new CustomEvent('dc-open-conn', { detail: { connId, focus } }))
}

/**
 * 「当前连接」的解析器 —— 由 MainView 注册。
 *
 * 为什么需要它：**后端返回**的只读拦截（走 api 拦截器那条路）前端手里没有连接对象，
 * 于是那条通知给不出「去修改数据源」的按钮 —— 而用户点到的正是它（截图里那句
 * 「连接「公司Mysql · test」被标记为只读」就是后端原话）。注册之后，凡是这类通知
 * 都能拿到"用户当前正在看的连接"，按钮就能真的打开那个数据源的编辑页。
 */
let connResolver = () => ''
export const setSafetyConnResolver = (fn) => {
  connResolver = typeof fn === 'function' ? fn : (() => '')
}

// 最近一次"安全拦截"提示的时间：给 ElMessage 出口过滤用（见 consumeSafetyMessage）
let lastSafetyAt = 0

/**
 * 给「中间那条提示」把关：安全拦截只留在右上角。
 *
 * 起因（用户反馈）：只读/生产拦截时，右上角弹了通知，**中间还同时蹦一条**
 * 「保存失败：连接「…」被标记为只读，WRITE 语句已被拒绝」—— 同一件事说两遍，中间那条还挡着表格。
 * 拦截的调用方遍布各处（数据编辑保存、导入、结构页、AI 面板…），而且它们**判断不出**"这是安全拦截"
 * （拿到的只是后端那句原文）—— 与其一个个改调用方，不如在 `ElMessage.error` 这个**唯一出口**处理：
 * 是安全拦截就丢掉中间那条；若刚才没人弹过右上角，就补弹一次（保证不漏提示）。
 *
 * 返回 true = 这条已经由右上角负责，调用方不要再显示中间那条。
 */
export const consumeSafetyMessage = (text) => {
  if (!isSafetyBlock(text)) return false
  const now = Date.now()
  // 6 秒内已经弹过右上角通知（拦截器的、或重命名弹窗的）→ 直接丢掉，不重复说
  if (now - lastSafetyAt > 6000) showSafetyNotice(text)
  else lastSafetyAt = now
  return true
}

// 同一句话短时间内只弹一次：一次操作可能在几处各报一次（结果面板 + 提示），叠三个通知很吵
let lastShown = { text: '', at: 0 }

/** 弹出安全拦截通知（右上角）。重复句 4 秒内忽略。 */
export const showSafetyNotice = (detail, opts = {}) => {
  // 兼容旧调用：第二参传字符串时就是标题（历史上是 showSafetyNotice(detail, title)）
  const o = typeof opts === 'string' ? { title: opts } : (opts || {})
  const text = String(detail || '')
  // 只读 / 生产 是**两套话**：放开的地方不一样（一个去改数据源，一个去关全局保护）。
  // 没显式指定就按正文自适应 —— 后端返回的拦截消息里写着"只读"，据此换成只读那套文案与按钮。
  const readOnly = o.kind ? o.kind === 'readonly' : /只读/.test(text)
  const title = o.title || t(readOnly ? 'tdet.blockTitleReadOnly' : 'tdet.prodBlockedTitle')
  const hints = o.hints || (readOnly ? [t('tdet.blockReadOnlyHint')] : [t('tdet.prodBlockedPath1'), t('tdet.prodBlockedPath2')])
  // 按钮动作的优先级：调用方给的回调 > 只读时"打开那个数据源的编辑页" > 生产保护时"去设置"。
  // 只读且**连连接都定位不到**时就是 null —— 那时按钮只能指向「安全与会话」，而那里管的是
  // 生产保护，跟只读没关系（指错地方比不给按钮更让人迷惑）。
  const connId = o.connId || (readOnly ? connResolver() : '')
  const action =
    o.onAction || (readOnly ? (connId ? () => requestEditConn(connId, 'readOnly') : null) : openSafetySettings)
  const actionText = o.actionText || (readOnly ? t('tdet.blockReadOnlyGo') : t('tdet.prodBlockedGo'))
  // ⚠ 没有 action 时**必须不渲染按钮**：原先无条件渲染一个 onClick:null 的按钮 ——
  // 它长得跟真按钮一模一样，用户点下去毫无反应（用户反馈「点这个没弹出、没反应」）。
  // 我的自动化探针当时用 tn.click()，所以
  const now = Date.now()
  // 记在案：稍后同一个拦截可能又被调用方用 ElMessage 报一遍（「保存失败：…被标记为只读」），
  // 出口过滤据此把那条中间提示丢掉（见 consumeSafetyMessage）。
  lastSafetyAt = now
  if (text && text === lastShown.text && now - lastShown.at < 4000) return
  lastShown = { text, at: now }
  try {
    ElNotification({
      type: 'warning',
      title,
      duration: 12000,
      // 用**行内样式**：通知渲染在 body 下，组件的 scoped 样式命中不了
      message: h('div', { style: 'max-width:560px;line-height:1.5' }, [
        h('div', text),
        h('div', { style: 'margin-top:9px;font-size:12px;opacity:.75' }, [
          h('div', t('tdet.prodBlockedHintTitle')),
          // nowrap：这些路径要**保持一行**（以前在窄框里断成「改成「开/发」」，很难读）
          ...hints.map((line, i) =>
            h('div', { style: `margin-top:${i === 0 ? 3 : 1}px;white-space:nowrap` }, line)
          )
        ]),
        action
          ? h(
              'button',
              {
                type: 'button',
                style:
                  'margin-top:10px;padding:4px 12px;border:none;border-radius:6px;cursor:pointer;' +
                  'font-size:12px;font-weight:600;color:#fff;background:var(--el-color-primary,#409eff)',
                onClick: action
              },
              actionText
            )
          : null
      ])
    })
  } catch {
    // 兜底：通知若因任何原因起不来（样式缺失、VNode 出错…），也必须让用户看到一句话 ——
    // "被拦下却毫无提示"是最难自查的（用户会以为点了没反应）。
    try {
      ElMessage.error(text)
    } catch {
      /* 连 message 都起不来就真没办法了 */
    }
  }
}
