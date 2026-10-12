// 可增删列表用的**稳定行 key**。
//
// 为什么需要它：`v-for` 用下标当 key 时，删掉中间一行，后面几行会"顶上"复用同一个 key ——
// Vue 会认为"只是内容变了"，于是行内组件（尤其 `el-select filterable` / `el-autocomplete`
// 这类**自带下拉面板与查询态**的控件）的内部状态会跟错行：刚删完，别的行弹着上一个选项的
// 过滤词，或者下拉看着"点了没反应"（用户遇到过）。
//
// 为什么不直接往行对象里塞一个 `_uid`：这些行**是要被序列化的** —— 条件生成器的 items、
// 数据浏览的 filters 都会 JSON 之后发给后端，多一个 `_uid` 就是脏数据（还会被写进配置）。
// 所以用 WeakMap 在**组件外部**记"对象 → 自增号"：不污染数据、不参与序列化、
// 对象被回收时条目自动消失（不会像 Map 那样越积越多）。
const ids = new WeakMap()
let seq = 0

/**
 * 取一行对象的稳定 key（第一次见到该对象时分配，之后恒定）。
 * 在模板里直接写 `:key="rowKey(it)"` 即可 —— 它按对象身份取号，同一行前后两次渲染
 * 拿到的是同一个 key，删掉别的行也不会变。
 */
export const rowKey = (row) => {
  if (row == null || (typeof row !== 'object' && typeof row !== 'function')) return String(row)
  let id = ids.get(row)
  if (id == null) {
    seq += 1
    id = seq
    ids.set(row, id)
  }
  return id
}
