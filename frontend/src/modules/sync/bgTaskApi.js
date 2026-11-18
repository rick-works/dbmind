// 后台任务的「按类型分派」注册表（任务中心 / 顶栏悬浮列表 / 通用任务详情窗共用）。
//
// 为什么单独一份：状态查询、取消、文案、能否就地还原，这四件事的**判据都是 kind**，
// 以前散在 MainView 里四五条 if/else 链上，加一类任务要改四处、漏一处就是
// 「点了停止没反应」或「任务变成假运行中」。现在只有这一处 switch。
//
// 各家状态接口形状**故意做成一致**（status / phase / done / total / message /
// logs / logsSeq / elapsedMs，见后端 crates/dbmind-web/src/api/tasks.rs 的 snapshot），
// 所以这里只负责选接口，不做字段归一。
import {
  syncTaskStatus, syncCancel,
  compareTaskStatus, compareCancel,
  backupTaskStatus, cancelBackup,
  restoreTaskStatus, cancelRestore,
  exportTask, exportCancel,
  importTask, importCancel,
  dataGenTask, dataGenCancel,
  runSqlFileStatus, runSqlFileCancel
} from '../../api'

/** 终态：这些状态不会再变，没有必要每 5 秒再问一次后端 */
export const BG_TERMINAL = ['success', 'canceled', 'error', 'notfound']
export const bgIsTerminal = (s) => BG_TERMINAL.includes(s)

/**
 * 任务类型 → 文案 key。
 * 'dump'（整库 SQL 转存）与 'export'（表/查询结果导出）走同一套导出任务接口，
 * 分开只是为了在任务中心一眼看出这是哪件事。
 */
const KIND_LABEL = {
  sync: 'nav.sync',
  compare: 'nav.compare',
  backup: 'ts.kindBackup',
  restore: 'ts.kindRestore',
  export: 'sync.kindExport',
  dump: 'sync.kindDump',
  import: 'sync.kindImport',
  datagen: 'sync.kindDatagen',
  runFile: 'sync.kindRunFile'
}
export const bgKindKey = (kind) => KIND_LABEL[kind] || KIND_LABEL.sync

/** 类型徽章配色：对比=橙、备份/还原=绿、导出/转存=青、导入=紫、造数/执行文件=蓝（走默认） */
const KIND_CLASS = {
  compare: 'is-cmp',
  backup: 'is-bkp',
  restore: 'is-bkp',
  export: 'is-exp',
  dump: 'is-exp',
  import: 'is-imp',
  datagen: 'is-gen',
  runFile: 'is-run'
}
export const bgKindClassOf = (kind) => KIND_CLASS[kind] || ''

/**
 * 运行中的状态文案 key（按类型分开）。
 * 以前一律用 'sync.running'（「同步中...」），于是「生成测试数据」「执行 SQL 文件」
 * 这些任务在任务中心/悬浮列表里也写着「同步中」，用户截图反馈「生成数据的任务显示同步中不对吧」。
 * 状态文字必须跟类型走，否则一眼看不出后台在干什么。
 */
const KIND_RUNNING = {
  sync: 'bgRun.sync',
  compare: 'bgRun.compare',
  backup: 'bgRun.backup',
  restore: 'bgRun.restore',
  export: 'bgRun.export',
  dump: 'bgRun.dump',
  import: 'bgRun.import',
  datagen: 'bgRun.datagen',
  runFile: 'bgRun.runFile'
}
export const bgKindRunningKey = (kind) => KIND_RUNNING[kind] || KIND_RUNNING.sync

/**
 * 停止后的提示文案 key（按类型分开，别在同步任务上弹「正在中断数据库查询」）。
 * 导出 / 转存 / 导入 / 造数 / 执行文件都是「这一批/这一页写完才停」，
 * 所以复用备份那句通用措辞，不用另造新文案。
 */
const KIND_STOP_MSG = {
  compare: 'cmp.stopRequested',
  backup: 'bkp.stopRequested',
  restore: 'bkp.stopRequested',
  export: 'bkp.stopRequested',
  dump: 'bkp.stopRequested',
  import: 'bkp.stopRequested',
  datagen: 'bkp.stopRequested',
  runFile: 'bkp.stopRequested'
}
export const bgKindStopMsg = (kind) => KIND_STOP_MSG[kind] || 'sync.stopRequested'

/**
 * 能在任务中心当场「查看」还原的任务类型：提交时只留了一个 taskId，
 * 原始表单/文件句柄早就没了（导入的源文件、导出的保存句柄），所以不去重建那个对话框，
 * 而是用通用进度窗按 taskId 实时查询 —— 进度、阶段、日志、停止、产物下载都在。
 */
export const BG_INLINE_KINDS = ['export', 'dump', 'import', 'datagen', 'runFile']
export const bgInlineResumable = (kind) => BG_INLINE_KINDS.includes(kind)

/** 导出/转存类任务结束后有产物可下载 */
export const bgHasArtifact = (kind) => kind === 'export' || kind === 'dump'

/** 按任务类型查状态（各接口字段一致，见文件头） */
export const bgTaskStatus = (task) => {
  switch (task.kind) {
    case 'compare': return compareTaskStatus(task.id)
    case 'backup': return backupTaskStatus(task.id)
    case 'restore': return restoreTaskStatus(task.id)
    case 'export':
    case 'dump': return exportTask(task.connId, task.id)
    case 'import': return importTask(task.connId, task.id)
    case 'datagen': return dataGenTask(task.id)
    case 'runFile': return runSqlFileStatus(task.connId, task.id)
    default: return syncTaskStatus(task.id)
  }
}

/** 按任务类型停止 */
export const bgTaskCancel = (task) => {
  switch (task.kind) {
    case 'compare': return compareCancel(task.id)
    case 'backup': return cancelBackup(task.id)
    case 'restore': return cancelRestore(task.id)
    case 'export':
    case 'dump': return exportCancel(task.connId, task.id)
    case 'import': return importCancel(task.connId, task.id)
    case 'datagen': return dataGenCancel(task.id)
    case 'runFile': return runSqlFileCancel(task.connId, task.id)
    default: return syncCancel(task.id)
  }
}
