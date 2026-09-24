import { t } from './i18n'
import { saveBlobAs } from './useExportTask'
/**
 * 连接配置的导入 / 导出（JSON 文件）—— 「新建数据源」弹窗底部按钮与左侧树右键菜单共用这一份。
 *
 * 三条口径（与参考项目 dbx 的 connectionConfigTransfer 一致）：
 * 1. 文件格式 `{ format:'dbmind-connections', version:1, exportedAt, note, connections:[…] }`，
 *    导入端**也认裸数组**（别的工具/手写的 JSON 都能进）。
 * 2. **口令一律不导出**：内核的连接详情从来不下发口令（`ConnectionConfig::password` 标了
 *    skip_serializing），所以不做"加密导出"这种办不到的承诺 —— 文件里写明 note，导入后补填。
 * 3. 导出时剔除**服务端生成 / 只读**的字段（id、时间戳、hasPassword…），导入端再清空口令，
 *    这样导出的文件可以直接再导入回去。
 */

export const CONN_FILE_FORMAT = 'dbmind-connections'

/** 导出时要丢掉的字段：服务端生成的、或只读回显的 */
const SKIP_KEYS = ['id', 'hasPassword', 'hasSshPassword', 'createTime', 'updateTime', 'readOnly']

/** 单条连接的导出形态（去掉服务端字段，其余原样，便于再导入） */
export function sanitizeConnection(conn) {
  const out = {}
  for (const [key, value] of Object.entries(conn || {})) {
    if (SKIP_KEYS.includes(key)) continue
    out[key] = value
  }
  return out
}

/** 组装导出文件内容 */
export function buildConnectionBundle(connections) {
  return {
    format: CONN_FILE_FORMAT,
    version: 1,
    exportedAt: new Date().toISOString(),
    note: 'password is NOT included; re-enter it after import',
    connections: (connections || []).map(sanitizeConnection)
  }
}

/**
 * 解析导入文件。
 * @returns {{list: object[]}|{error: string}}
 */
export function parseConnectionBundle(rawText) {
  let data = null
  try {
    data = JSON.parse(rawText)
  } catch (e) {
    return { error: t('dsp.badJson') }
  }
  const list = Array.isArray(data)
    ? data
    : (data && Array.isArray(data.connections) ? data.connections : null)
  if (!list) return { error: t('dsp.noConnArray') }
  const valid = list.filter(c => c && typeof c === 'object'
    && String(c.name || '').trim() && String(c.type || '').trim())
  if (!valid.length) return { error: t('dsp.noValidConn') }
  return { list: valid }
}

/** 导入用的一条 payload：清掉 id 与口令（空口令 = 没设置，不会覆盖已存的） */
export function importPayloadOf(item, environment) {
  const payload = sanitizeConnection(item)
  payload.id = ''
  payload.password = ''
  payload.sshPassword = ''
  if (environment) payload.environment = environment
  return payload
}

/** 导出 JSON 文件（走统一的"用户选位置"） */
export function downloadJson(filename, data) {
  const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json;charset=utf-8' })
  saveBlobAs(blob, filename)
}
