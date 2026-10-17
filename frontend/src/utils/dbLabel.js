/**
 * 「库」这一层在**界面上**怎么显示。
 *
 * 后端有两处会退回占位库名 `(default)`（见 `crates/dbmind-web/src/api/nosql.rs` 的
 * `databases` 与 `meta.rs`：NoSQL 没有库层时集合/索引就挂在它下面，关系型则是"列库失败"的
 * 兜底）。带括号的原值直接摆在树上像个占位符，用户反馈"不用显示括号"。
 *
 * 只在**显示层**替换：`node.db`、请求参数、缓存键全都继续用原值 —— 把这个值改掉，
 * 后端就认不出这是占位库了（`NO_DEFAULT_DB` 的过滤、会话 / 缓存 key 都按原值工作）。
 */
import { t } from './i18n'

/** 与 MainView 的 `NO_DEFAULT_DB`、后端 nosql.rs / meta.rs 里的占位名保持一致 */
export const PLACEHOLDER_DB = '(default)'

export const dbLabelOf = (db) => {
  const s = db == null ? '' : String(db)
  return s === PLACEHOLDER_DB ? t('mv.defaultDb') : s
}
