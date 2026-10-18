/**
 * 连接失败弹窗里显示什么：**只显示数据库 / 驱动说的原话**。
 *
 * 这条提示被砍过两轮，原因是同一个：用户要的是"到底哪错了"，不是我们的解释。
 *   · 最初：`错误原文：… ` + 「可以这样排查」四步通用清单（端口、防火墙、白名单、账号…）；
 *   · 中间：原话 + 一句按关键字猜的说明（`端口无响应：服务未启动或被防火墙拦截`）——
 *     猜错就是误导，猜对也是废话，原话里已经写着 `Connection refused`。
 * 现在只剩抄原文：剥掉我们自己加的包装（错误码、`连接失败：`、老版本「自检失败」行）即可。
 *
 * 纯函数、无副作用：只决定界面上显示什么，不改任何连接参数。
 */
import { cleanErrText } from './errMsg'

/**
 * 数据库 / 驱动的原话。
 *
 * 后端的 `message` 里通常还套着我们自己的几层包装，这里逐层剥掉：
 *   `[DBMIND-CONN-0003] `、`连接「x」自检失败`（老内核）、行首的 `连接失败：`。
 * 技术细节行（异常类名、`SQLState=… errorCode=…`）由 `cleanErrText` 过滤。
 * 剥完为空（罕见）就退回清理后的原文，绝不返回空串让弹窗变成白板。
 */
export const connErrorRaw = (message, detail = '') => {
  const cleaned = cleanErrText([String(message || ''), String(detail || '')].filter((s) => s.trim()).join('\n'))
  if (!cleaned) return ''
  return cleaned
    .replace(/\[DBMIND-[A-Z]+-\d+\]\s*/g, '')
    .replace(/^(连接失败|连接被拒绝|连接已断开|Connection failed)\s*[：:]\s*/i, '')
    .trim() || cleaned
}
