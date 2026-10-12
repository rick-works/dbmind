// 列类型「写法」的统一口径 —— 下拉清单与回显值必须是**同一个写法**。
//
// 现状为什么会长短不一：回显的基名来自数据库自己的元数据（MySQL 回 `varchar(64)`、
// `datetime`；Doris 回 `string`/`date`；Oracle 回 `varchar2`…），而下拉清单是我们按方言
// 手写的一份名字。两者只是大小写/别名不同，界面就出现「下拉里是 DATETIME、选完回显成
// datetime」「下拉 VARCHAR、回来 varchar(64)」这种"同一个类型两种写法"（用户反馈）。
//
// 规则（就此定死，别再各处拍脑袋）：
//   **每个数据源内部，下拉清单就是这份"标准写法"；回显值向清单对齐。**
//   · 能忽略大小写、忽略多余空白对上某一项 → 用清单里的那个写法（MySQL 系/清单是大写，
//     就回显大写；PG 清单是小写，就回显小写）；
//   · 对不上（数据库报了个清单里没有的名字，例如自带方言别名）→ 原样保留，不硬改 ——
//     硬改会生成数据库不认的类型名。
//   于是「下拉 = 回显」在每个数据源里都是**构造性成立**的，而不是靠人记住两处要一致。
//
// 注意：判断"用户是否改过这一列"用的 normType() 是**忽略大小写**的
// （TableDetailView.vue 的 normType = toLowerCase），所以这里对齐大小写
// **不会**造成"打开表就显示已修改"。
export const canonTypeBase = (base, list) => {
  const raw = String(base ?? '').trim()
  if (!raw) return raw
  const key = raw.replace(/\s+/g, ' ').toLowerCase()
  const hit = (Array.isArray(list) ? list : [])
    .find((t) => String(t ?? '').replace(/\s+/g, ' ').trim().toLowerCase() === key)
  return hit != null ? String(hit) : raw
}
