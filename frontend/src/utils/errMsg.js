/**
 * 从 axios 错误里取一段**能给人看**的消息。
 *
 * 除了取字段，这里还要做一件以前没人做的事：**把技术尾巴剔掉**。
 *
 * 后端（尤其 JDBC 宿主）常常把几样东西拼在同一条 message 里：一句中文结论、数据库自己的
 * 原话、以及给开发看的尾巴（`SQLRecoverableException, SQLState=08006, errorCode=17002`、
 * `next: ...`、Java 栈帧）。界面上把它们整段糊出来，用户只能读到一堆类名与十六进制般的
 * 代码（真机截图里那条 Oracle 报错就是这样）。
 *
 * 判据只针对**明显是技术尾巴的行**，其余一律保留：
 *   · `XxxException, SQLState=…` —— JDBC 异常类名 + 状态码
 *   · 单独一行的 `next: …` —— 驱动串起来的次生异常
 *   · `at com.foo.Bar(Bar.java:12)` —— 栈帧
 * 剔完若一行不剩（说明整条都是技术信息），原样返回，不能因为"清理"把错误变没了。
 */

/** 只留人话：去掉 JDBC/Java 的技术尾巴，顺手把中英混排产生的 "。," 收拾干净 */
export const cleanErrText = (raw) => {
  const text = String(raw || '')
  if (!text) return ''
  const lines = text.split(/\r?\n/)
  const isNoise = (line) => {
    const s = line.trim()
    if (!s) return false
    if (/^[A-Za-z_$][\w.$]*Exception\b/.test(s)) return true
    if (/\bSQLState\s*=/.test(s) && /\berrorCode\s*=/.test(s)) return true
    if (/^next:\s/i.test(s)) return true
    if (/^at\s+[\w.$]+\(/.test(s)) return true
    // 老内核把测试失败包装成「连接「x」自检失败」单独一行：用户要的是数据库原话
    if (/^连接「.*」自检失败[。.!]?$/.test(s)) return true
    return false
  }
  const kept = lines.filter((line) => !isNoise(line))
  const out = (kept.some((l) => l.trim()) ? kept : lines)
    .join('\n')
    // 错误码前缀（`[DBMIND-CONN-0003] `）留给日志；界面上只留正文
    .replace(/\[DBMIND-[A-Z]+-\d+\]\s*/g, '')
    .replace(/。\s*,\s*/g, '。')      // Oracle 中文原话后面接了英文后缀（"…尝试失败。, Authentication…"）
    .replace(/[ \t]{2,}/g, ' ')
    .trim()
  return out
}

export const errMsg = (e, fallback = '') => {
  let msg = e?.response?.data?.message
  if (!msg && e?.response?.data && typeof e.response.data === 'object') {
    msg = e.response.data.error || e.response.data.msg ||
      (Object.keys(e.response.data).length ? JSON.stringify(e.response.data) : '')
  }
  if (!msg) msg = e?.message
  if (!msg) msg = typeof e === 'string' ? e : ''
  if (!msg && e && typeof e.toString === 'function') {
    const s = e.toString()
    msg = s && s !== '[object Object]' ? s : ''
  }
  return cleanErrText(msg) || fallback
}

/**
 * 把「原始错误 / 说明」两段拆出来（后端 `XError::with_detail` 给的那两个字段）。
 *
 * 有些错误一句话说不清：原话是 `SELECT command denied to user 'x' for table 'user'`，
 * 而用户要读的是「这个账号没有看用户清单的权限，换账号或让 DBA 授权」。两段各有用途，
 * 糊成一段就成了又长又平的红色文字。界面按「原话在上、说明在下」排版。
 *
 * `message` 一定返回（后端没拆段的错误，它就是全部内容）。
 */
export const errParts = (e, fallback = '') => {
  const data = e?.response?.data || {}
  const hint = typeof data.hint === 'string' ? cleanErrText(data.hint) : ''
  const detail = typeof data.detail === 'string' ? cleanErrText(data.detail) : ''
  return { hint, detail, message: errMsg(e, fallback) }
}
