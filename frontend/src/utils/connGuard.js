/**
 * 「连接级写操作」的统一**前置闸门**（体验层的第一道闸）。
 *
 * 为什么要单独一个模块：用户管理 / 用户表单这类写入口不在 MainView 的树上，
 * 它们是独立页面与页签组件，拿不到 MainView 里那份 `guardWrite`。而判定口径
 * （只读连接 / 生产保护）**必须只有一份** —— 抄一份就迟早漂开：
 * 曾经前端只认环境角标 `env`，后端 `is_prod_environment` 还认分组（`group` /
 * 旧键 `environment`），于是「分组是生产、角标没标」的数据源在前端点下去毫无提示，
 * 一路填完表单才被后端拦下（最气人的顺序）。
 *
 * 它只是**体验层**：真正的判定仍在后端 `SafetyPolicy::check`（设置可能刚被别处改过、
 * 脚本与 CLI 也能绕过界面），所以后端那道闸永远保留。
 */
import { getSettings, listConnections } from '../api'
import { t } from './i18n'
import { showSafetyNotice, requestEditConn } from './safetyNotice'

// ------------------------------------------------------------------ 连接最新快照
// 页签 / 弹窗手里的 `conn` 往往是**打开那一刻**的快照（`props.conn`），连接弹窗改完
// 「只读」不会让它们自动更新 —— 于是拦截判定还按旧值拦人：用户把只读关掉、回到已经开着的
// 查询页签点「提交修改」，仍然弹「被只读连接拦下」（真机反馈）。
// 这里维护一份按 id 索引的最新清单：MainView 保存连接后广播 dbmind-conns-changed，
// 收到就重拉一次，拦截判定统一取最新对象（所有页签一起生效，不必每个组件各写一遍）。
const liveConns = new Map()
let liveLoading = null
export const refreshLiveConns = () => {
  if (liveLoading) return liveLoading
  liveLoading = listConnections()
    .then((list) => {
      liveConns.clear()
      for (const c of (Array.isArray(list) ? list : [])) liveConns.set(String(c.id), c)
    })
    .catch(() => { /* 取不到就维持旧快照：宁可多拦一次，也不放过只读库的写操作 */ })
    .finally(() => { liveLoading = null })
  return liveLoading
}
/** 这条连接的最新快照（不认识就原样退回）。 */
export const connLatest = (conn) => (
  conn && liveConns.has(String(conn.id)) ? liveConns.get(String(conn.id)) : conn
)
window.addEventListener('dbmind-conns-changed', () => { refreshLiveConns() })
// 启动时先拉一次：页面打开期间连接可能已被其它窗口/页签改过
refreshLiveConns()

// 生产保护开关：与 MainView / 设置页同一套解析 —— 只有明确的 'false'/'0' 才算关
// （缺省 = 开，与后端 storage.rs 的 seed_settings 一致）。
let protectProd = true
let flagLoadedAt = 0
// 30 秒内复用：设置可能刚被改过（设置页保存后会立刻回读），但不必每次点击都往返一次
const FLAG_TTL = 30_000

/** 读一次「生产保护」开关（带短 TTL 缓存；取不到就维持默认「开」）。 */
export const loadProtectProd = async () => {
  if (Date.now() - flagLoadedAt < FLAG_TTL) return protectProd
  try {
    const s = await getSettings()
    const v = s && s['safety.protectProduction']
    protectProd = !(v === 'false' || v === '0')
    flagLoadedAt = Date.now()
  } catch {
    /* 取不到就维持默认（开）：宁可多提醒一次，也不放过对生产库的写操作 */
  }
  return protectProd
}

/**
 * 这条连接是不是被标注为**生产环境**。
 *
 * 三处标注**任一**为生产即算（角标 `env` / 分组 `group` / 旧版分组键 `environment`），
 * 大小写不敏感、也认手输的「生产」—— 与后端 `ConnectionRecord::is_prod_environment`
 * 完全同一口径，否则就会出现"界面上没提示、后端却拦下"的错位。
 */
export const isProdConn = (conn) => {
  const hit = (value) => {
    const s = String(value ?? '').trim()
    return s.toUpperCase() === 'PROD' || s === '生产'
  }
  return hit(conn && conn.env) || hit(conn && conn.group) || hit(conn && conn.environment)
}

/**
 * 写操作前置闸门。`true` = 放行；`false` = 已拦下（并弹了带动作的右上角通知）。
 *
 * `act` 传这次动作的名字（「新增用户」「删除用户」「保存」…），让提示说清是什么被拦了。
 *
 * 顺序：**先判生产保护、再判只读**。一个连接可能两者都占了 —— 只提示只读的话，
 * 用户照着通知关掉「只读」后下一次还是被拦（生产保护仍在），等于给了一条走不通的路。
 */
export const guardConnWrite = (rawConn, act) => {
  // 用最新快照判定：`rawConn` 可能是页签打开时的旧对象（只读已关掉，这里还留着 true）
  const conn = connLatest(rawConn)
  const name = (conn && (conn.name || conn.label)) || ''
  if (protectProd && isProdConn(conn)) {
    showSafetyNotice(t('mv.blockProdDetail', { name, act }))
    return false
  }
  if (conn && conn.readOnly) {
    showSafetyNotice(t('mv.blockReadOnlyDetail', { name, act }), {
      kind: 'readonly',
      title: t('tdet.blockTitleReadOnly'),
      // 通知渲染在 body 下，拿不到连接对象 —— 用全局事件把 connId 回传给 MainView，
      // 「去修改数据源」才能真的打开**这个**连接的编辑弹窗并停在「只读」那一项。
      ...(conn.id ? { onAction: () => requestEditConn(conn.id, 'readOnly') } : {})
    })
    return false
  }
  return true
}

// 设置页保存「安全与会话」后广播 dbmind-safety-changed：这里**立即重读**开关，
// 不等 30 秒 TTL、更不等页面重载 —— 否则「关掉保护回到页面马上点保存」仍会被旧值拦下
//（用户实测：必须刷新浏览器才生效）。派发处见 SettingsView.saveSafety。
window.addEventListener('dbmind-safety-changed', () => {
  flagLoadedAt = 0
  loadProtectProd()
})
