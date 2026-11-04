/**
 * AI 助手的**跨卸载**面板状态。
 *
 * <p>为什么需要它：中央工作区形态的 AiPanel 在 MainView 的页签 v-else-if 链里，
 * 切到「表数据 / SQL 查询」等页签时组件整个卸载，切回来重新挂载 ——
 * 对话、问数记录上下文如果只放在组件 ref 里就会凭空消失（表现为「对话被清空 /
 * 追问开了新记录」）。把它们放进模块级 reactive，卸载也留着，
 * 重新挂载时 restoreThread 见 messages 非空会跳过恢复，原样接上。
 *
 * <p>**只有用户点「新对话」才真的清空**（newThread 会重置这里）。
 * panel（右侧窄面板）与 studio（中央工作区）是两个实例，状态分开各记各的。
 */
import { reactive } from 'vue'

const makeState = () => ({
  /** 当前技能页签 */
  tab: 'chat',
  /** 对话流（消息气泡） */
  messages: [],
  /** 服务端会话 id（自由对话） */
  chatThreadId: '',
  /** 问数对话的本地记录 id（追问并进同一条「记录」） */
  askThreadId: '',
  /**
   * 问数上下文：连接 / 库 / 模式 / 聚焦表。同样跨卸载 ——
   * 用户勾好的聚焦表如果只放组件 ref 里，切个页签回来就没了，
   * 问数拿不到 tables 就走「12 张兜底」，用户看到的就是
   * 「我明明选了表，怎么还去找 12 张」（真机踩过）。
   */
  ctxConnId: '',
  ctxDatabase: '',
  ctxSchema: '',
  ctxTables: [],
})

export const panelState = reactive({
  studio: makeState(),
  panel: makeState(),
})
