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

// 同一句话短时间内只弹一次：一次操作可能在几处各报一次（结果面板 + 提示），叠三个通知很吵
let lastShown = { text: '', at: 0 }

/** 弹出安全拦截通知（右上角）。重复句 4 秒内忽略。 */
export const showSafetyNotice = (detail) => {
  const text = String(detail || '')
  const now = Date.now()
  if (text && text === lastShown.text && now - lastShown.at < 4000) return
  lastShown = { text, at: now }
  try {
    ElNotification({
      type: 'warning',
      title: t('tdet.prodBlockedTitle'),
      duration: 12000,
      // 用**行内样式**：通知渲染在 body 下，组件的 scoped 样式命中不了
      message: h('div', { style: 'max-width:560px;line-height:1.5' }, [
        h('div', text),
        h('div', { style: 'margin-top:9px;font-size:12px;opacity:.75' }, [
          h('div', t('tdet.prodBlockedHintTitle')),
          // nowrap：这两条路径要**保持一行**（以前在窄框里断成「改成「开/发」」，很难读）
          h('div', { style: 'margin-top:3px;white-space:nowrap' }, '· ' + t('tdet.prodBlockedPath1')),
          h('div', { style: 'margin-top:1px;white-space:nowrap' }, '· ' + t('tdet.prodBlockedPath2'))
        ]),
        h(
          'button',
          {
            type: 'button',
            style:
              'margin-top:10px;padding:4px 12px;border:none;border-radius:6px;cursor:pointer;' +
              'font-size:12px;font-weight:600;color:#fff;background:var(--el-color-primary,#409eff)',
            onClick: openSafetySettings
          },
          t('tdet.prodBlockedGo')
        )
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
