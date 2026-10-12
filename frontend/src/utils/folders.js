// 纯分组（还没有任何连接归属的分组，只在树上挂个名字）的**唯一真相源**：
// 后端 app_settings 的 `ui.folders`（JSON 数组），随数据目录走 —— 清空数据目录、
// 换数据目录迁移时它与连接/AI 配置同进同出。曾经存浏览器 localStorage：
// 清了数据目录分组还在，与「数据目录存放全部本地数据」的承诺矛盾，已迁走。
//
// 读写走通用设置接口；模块内共享一份响应式快照，各视图（MainView / FolderDialogs）
// 直接改这份 ref 再调 save，不必各自持有一份再对齐。
import { ref } from 'vue'
import { getSettings, putSetting } from '../api'

export const pureFolders = ref([])

/** 启动时加载；旧 localStorage 数据只迁移**一次**（有 `ui.foldersMigrated` 标记记着）。
 *
 * 为什么必须有一次标记：桌面 App 的 WebView 存储不随数据目录清空/重装消失 ——
 * 没有标记的话，用户清空数据目录后，每次启动都会把 WebView 里残留的旧分组
 * 再灌回后端（真机踩过：重装后「本机资源/复悦荟」复活）。标记之后的启动见到
 * 旧键就直接删，不再导入。 */
export async function loadPureFolders() {
  let migrated = false
  try {
    const s = await getSettings()
    const raw = s?.['ui.folders']
    const arr = raw ? JSON.parse(raw) : []
    pureFolders.value = Array.isArray(arr) ? arr.filter(f => f && typeof f === 'string') : []
    migrated = s?.['ui.foldersMigrated'] === 'true'
  } catch { /* 离线/降级：维持当前值，不挡住树渲染 */ }
  try {
    const legacy = localStorage.getItem('dbmind_folders')
    if (legacy !== null) {
      localStorage.removeItem('dbmind_folders')
      if (!migrated) {
        try {
          const arr = JSON.parse(legacy)
          if (Array.isArray(arr)) {
            for (const f of arr) {
              if (f && typeof f === 'string' && !pureFolders.value.includes(f)) pureFolders.value.push(f)
            }
          }
        } catch { /* 旧键损坏就当没有 */ }
        await savePureFolders()
      }
      await putSetting('ui.foldersMigrated', 'true')
    }
  } catch { /* localStorage 不可用就算了 */ }
  return pureFolders.value
}

/* ===== 置顶分组 =====
   记录"被置顶的分组全路径"（列表顺序即置顶优先级），存后端 app_settings 的 ui.folderPins。
   真正决定树上顺序的是 MainView 的 customFolders —— 那边会把这些路径提到最前。
   不能靠调 pureFolders 的顺序实现：customFolders 是「连接的 group ∪ 纯分组」的并集，
   带连接的分组不在 pureFolders 里，挪它没用。 */
export const pinnedFolders = ref([])

export async function loadPinnedFolders() {
  try {
    const s = await getSettings()
    const raw = s?.['ui.folderPins']
    const arr = raw ? JSON.parse(raw) : []
    pinnedFolders.value = Array.isArray(arr) ? arr.filter(f => f && typeof f === 'string') : []
  } catch { /* 离线降级：维持当前值 */ }
  return pinnedFolders.value
}

export async function savePinnedFolders() {
  try { await putSetting('ui.folderPins', JSON.stringify(pinnedFolders.value)) } catch { /* 忽略 */ }
}

/** 取消置顶：从列表里摘掉并落盘（其余置顶项的相对顺序不变） */
export async function unpinFolder(path) {
  if (!path) return
  pinnedFolders.value = pinnedFolders.value.filter(p => p !== path)
  await savePinnedFolders()
}

/** 把某个分组置顶：从列表里摘掉再放最前，然后落盘 */
export async function pinFolder(path) {
  if (!path) return
  pinnedFolders.value = [path, ...pinnedFolders.value.filter(p => p !== path)]
  await savePinnedFolders()
}

export async function savePureFolders() {
  try {
    await putSetting('ui.folders', JSON.stringify(pureFolders.value))
  } catch { /* 离线时不阻塞 UI，下次保存再试 */ }
}

export async function addPureFolder(name) {
  if (!pureFolders.value.includes(name)) pureFolders.value.push(name)
  await savePureFolders()
}

export async function renamePureFolder(from, to) {
  const i = pureFolders.value.indexOf(from)
  if (i >= 0) pureFolders.value[i] = to
  else pureFolders.value.push(to)
  await savePureFolders()
}

export async function removePureFolder(name) {
  const i = pureFolders.value.indexOf(name)
  if (i >= 0) pureFolders.value.splice(i, 1)
  await savePureFolders()
}
