import { t } from './i18n'
// 环境分组 / 目录（folder）共享元数据与帮助函数
// 供 MainView 树构建与 FolderDialogs 等弹窗共同使用，避免常量多处复制导致漂移
export const PREDEF_ENVS = ['DEV', 'TEST', 'PROD']
export const ENV_ORDER_BASE = ['DEV', 'TEST', 'PROD']
export const ENV_LABELS = { DEV: 'env.devFull', TEST: 'env.testFull', PROD: 'env.prodFull', STAGING: 'env.stagingFull', UAT: 'env.uatFull', '': 'env.ungrouped' }
export const ENV_COLORS = { DEV: '#3ddc97', TEST: '#f59e0b', PROD: '#ef4444', STAGING: '#a78bfa', UAT: '#06b6d4', '': '#6b7280' }
export const ENV_SHORT = { DEV: 'env.devShort', TEST: 'env.testShort', PROD: 'env.prodShort', STAGING: 'env.stagingShort', UAT: 'env.uatShort', '': 'env.ungrouped' }
export const envLabel = (env) => env ? (ENV_LABELS[env] ? t(ENV_LABELS[env]) : env) : t(ENV_LABELS[''])
export const envColor = (env) => ENV_COLORS[env] || ENV_COLORS['']
export const envShort = (env) => t(ENV_SHORT[env] || ENV_SHORT[''])
export const envTitle = (env) => t(ENV_LABELS[env] || ENV_LABELS[''])
