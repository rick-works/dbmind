// 树节点组装纯函数（供 MainView 的 lazyLoad 构建「库 / schema 下分类节点」复用）
// 输入某库（或 schema）下已经拉取的原始对象数组，输出 el-tree 懒加载可直接 resolve 的分类子节点。

/**
 * 一个例程是「函数」还是「存储过程」。
 *
 * **唯一判据**是后端返回的 `routineType`（见 dbmind-web/src/api/dialect.rs 的 `procedures()`：
 * `select routine_name as name, routine_type as routineType from information_schema.routines`）。
 *
 * 别改成拿 `comment` / `type` 去猜：这条接口**不返回**那两个字段，猜的结果是
 * "所有例程都是存储过程" —— 界面上的表现就是函数永远混在存储过程里、Functions 恒为 0。
 *
 * 这里是全工程唯一的定义处：树上的分类、数量，以及数据同步对话框里的对象清单，
 * 都必须用同一个判据，否则「树上算作函数、同步时算作存储过程」这种分叉迟早会出来。
 */
export const isFunctionRoutine = (o) =>
  String((o && o.routineType) || '').toUpperCase() === 'FUNCTION'

// 构造单个分类的子节点。
// buildObjectCategories（整库建树）与 MainView.refreshCatNode（只刷新一个分类）共用，
// 保证刷新前后节点 id / 携带字段完全一致，避免映射漂移（如用户缺 name/host、对象缺 @@table 后缀）。
export const buildCatChildren = ({ cat, connId, db, items = [] }) => {
  const arr = Array.isArray(items) ? items : []
  if (cat === 'tables') {
    return arr.filter(t => (t.type || 'TABLE') === 'TABLE').map(t => ({
      id: 'table:' + connId + ':' + db + ':' + t.name, label: t.name, kind: 'table',
      table: t.name, rows: t.rows, comment: t.comment, db, connId
    }))
  }
  if (cat === 'views') {
    return arr.filter(t => t.type === 'VIEW').map(v => ({
      id: 'view:' + connId + ':' + db + ':' + v.name, label: v.name, kind: 'view', table: v.name, db, connId
    }))
  }
  if (cat === 'users') {
    return arr.map(u => {
      const uname = u.host ? (u.name + '@' + u.host) : u.name
      return {
        id: 'user:' + connId + ':' + db + ':' + uname,
        label: uname, kind: 'user',
        name: u.name, host: u.host || '',
        objectName: uname, objectKind: 'user', table: '', comment: u.comment || '',
        db, connId
      }
    })
  }
  if (cat === 'scripts') {
    return arr.map(s => ({
      id: 'script:' + connId + ':' + db + ':' + s.id, label: s.name, kind: 'script',
      scriptId: s.id, db, connId
    }))
  }
  // 存储过程 / 函数 / 触发器 / 事件
  // 对象名保持原名；所属表单独放在 table 字段，避免把 "name@@table" 当成对象名传给 SHOW CREATE
  const singularKind = (c) => ({ procs: 'procedure', functions: 'function', triggers: 'trigger', events: 'event' }[c] || c)
  return arr
    // 存储过程与函数来自**同一个**接口（information_schema.routines 一次返回两类，靠
    // routineType 区分），所以两个分类在这里各自只留自己那一类。
    //
    // 不筛的后果不是"多显示几个"：两个分类会显示**同一份清单** —— 数字一模一样、
    // 点开看到的都是全部，函数在 Procedures 里、Functions 里又是同一批。
    .filter(o => {
      if (cat === 'functions') return isFunctionRoutine(o)
      if (cat === 'procs') return !isFunctionRoutine(o)
      return true
    })
    .map(o => {
      const nodeKind = singularKind(cat)
      const name = o.name || ''
      return {
        id: nodeKind + ':' + connId + ':' + db + ':' + name,
        label: name, kind: nodeKind,
        objectName: name,
        objectKind: nodeKind, table: o.table || '', comment: o.comment || '',
        // db / connId 必须带上：右键「编辑」需要据此把脚本页签的
        // 连接、库（schema 层级类型为「库.schema」）筛选器带成对象自己的上下文，
        // 缺失时会退回当前库，导致编辑 B 库对象却按 A 库打开
        db, connId
      }
    })
}

export const buildObjectCategories = ({
  connId, db, tables = [], procs = [], triggers = [], events = [], users = [], scripts = []
}) => {
  // 表与视图来自同一个 listTables 结果（后端 /tables 会同时返回 TABLE 与 VIEW），
  // 因此 views 分类复用 tables 数组，再由 buildCatChildren 按 type === 'VIEW' 过滤；
  // 曾经漏掉 views 键导致「视图」分类恒为 0，视图节点无法在树中出现。
  //
  // 存储过程与函数同理：同一个 listProcedures 结果，由 buildCatChildren 按 routineType
  // 拆成 procs / functions 两个分类。所以下面**两个键必须都指向 procs 数组** ——
  // 少写一个的后果与当初漏掉 views 一模一样：那一类恒为 0、节点在树里根本不出现。
  const rawByCat = { tables, views: tables, procs, functions: procs, triggers, events, users, scripts }
  // `label` 保留英文原文：一是作兜底，二是供树内搜索匹配（见 MainView 的 filterNode）。
  // 界面上显示的其实是 `i18nKey` 对应的译文 —— 分类名原来写死英文，
  // 中文界面下也一直显示 Tables/Views，就是因为这里没有可翻译的键。
  //
  // 为什么**不在这里直接调 t()**：本函数是纯数据组装，节点一旦建好就固定了；
  // 而 t() 读的是响应式 locale —— 写在模板里才能做到"切语言，树上立刻跟着变"，
  // 否则要重建整棵树才生效。
  const mkCat = (cat, label) => ({
    id: 'cat-' + connId + '-' + db + ':' + cat, label, i18nKey: 'tree.cat.' + cat,
    kind: 'category', cat, db, connId,
    children: buildCatChildren({ cat, connId, db, items: rawByCat[cat] })
  })
  return {
    // 仅表（不含视图）的原始对象，供父级刷新「数据对比/同步」等场景使用
    tables: (Array.isArray(tables) ? tables : []).filter(t => (t.type || 'TABLE') === 'TABLE'),
    nodes: [
      mkCat('tables', 'Tables'),
      mkCat('views', 'Views'),
      // 存储过程与函数分成两类排在一起：原来合成一个 Procedures，
      // 点开是一锅（函数混在过程里），数量也没法分别看。
      mkCat('procs', 'Procedures'),
      mkCat('functions', 'Functions'),
      mkCat('triggers', 'Triggers'),
      mkCat('events', 'Events'),
      mkCat('users', 'Users'),
      mkCat('scripts', 'Scripts')
    ]
  }
}
