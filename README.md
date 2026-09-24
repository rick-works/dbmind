# DBMind

**多数据库统一管理客户端 —— 前端（Vue 3）+ Rust 内核 + 兼容契约 HTTP 层。**

前端在既有前端工程的基础上**持续改造而来**（界面、样式、菜单、交互都已按 DBMind 的需求改过）；
后端是 Rust 内核（`dbmind-core`）+ 一层实现 `/api/…` 契约的 HTTP 壳（`crates/dbmind-web/src/api/`）。

> **关于这层兼容层**：内核只提供少数元数据能力，而界面要的是完整的一套 REST 契约。
> 所以它照**既有前端的调用形状**实现 —— 注释里那些「界面要的…」「与既有实现一致」
> 是**取舍记录**（解释代码为什么长这样），不是别名。

---

## 1. 快速开始

```powershell
# 1) 构建后端（首次约 1 分钟）
cd DBMind
cargo build -p dbmind-web

# 2) 启动：单进程同时提供 API 与前端产物，默认 http://127.0.0.1:20361
.\target\debug\dbmind-web.exe --port 20361 --dist frontend\dist
```

浏览器打开 <http://127.0.0.1:20361/> 就是 DBMind 的界面。

### 桌面版（Tauri）

同一个内核，换成系统 WebView 当容器 —— 不是"开个浏览器"，而是一个真正的窗口：

```powershell
cargo build -p dbmind-desktop --release
.\target\release\dbmind-desktop.exe
```

它做三件事：**①** 探一次 `20361` —— 那儿已经有 DBMind（比如你先前用 `start.ps1` 起过）
就**直接复用**，不再起第二份引擎（两个进程同写一份 SQLite 元数据库迟早出事）；
**②** 没有就自己起内嵌 HTTP 内核（`dbmind_web::spawn_embedded`），端口被别的东西占了
就往后找一个空闲的；**③** 把窗口指向 `http://127.0.0.1:<port>`。

**为什么是"壳 + 本地 HTTP"而不是 IPC 命令**：前端整份代码都按 HTTP 契约写
（`api/index.js` 的 `baseURL` 是空的相对路径），桌面版沿用同一条链路 ⇒
Web 版与桌面版共享**同一份前端产物、同一套错误结构、同一份契约**；给桌面单独发明一套 IPC，
等于把每个接口实现两遍，然后眼看着它们漂开。库位置也与 CLI 一致
（默认 `~/.dbmind`，可用 `DBMIND_STORE` 覆盖）：**换壳不换数据**。

> 用的是 **Tauri**（Rust + 系统 WebView），不是 Electron —— 不带第二个几百 MB 的运行时；
> Windows 上只需要系统自带的 **WebView2**（Win11 已内置，Win10 装一次即可）。

**前端开发模式（热更新）**：后端保持运行，另开一个终端 `cd frontend && npm install && npm run dev`
—— `vite.config.mjs` 的代理本来就指向 `20361`（沿用的历史端口），无需改动。

**端到端验证**（自己起服务、用临时数据目录、跑完清理，不碰你的 `~/.dbmind`）：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\smoke-dbmind.ps1
# 期望末行：RESULT passed=69 failed=0
```

### 连关系型 / NoSQL 数据库的**前置条件**（SQLite 不需要）

除了 SQLite（内核自带的原生实现），其它数据库都要经 **Java 宿主** + **JDBC 驱动**，两样缺一样就会
报 `DBMIND-DRV-0002 驱动宿主未就绪`：

```powershell
# 1) 构建四个宿主 jar（需要 JDK 17+ 与 Maven 3.6.3+，约 1 分钟）
$env:JAVA_HOME = 'D:\develop\tools\jdk-17.0.20.1+1'      # 或任何 17+
powershell -ExecutionPolicy Bypass -File scripts\build-agents.ps1

# 2) 用启动脚本起服务：它会
#    · 把宿主 jar 复制到 exe 旁（debug/release 都找得到）
#    · 自动挑一个 Java 17+ 并用 DBMIND_JAVA 固定下来
powershell -ExecutionPolicy Bypass -File scripts\start.ps1
```

**驱动不用手动装**：首次测试连接 / 执行查询时，后端会按类型的 Maven 坐标从 Maven Central
自动下载该类型的驱动并落盘到 `~/.dbmind/drivers/<类型>/`，之后复用。下载失败会把原因记下来
（不会每次查询都卡一次超时），修好网络后用 `POST /api/drivers/{CODE}/install` 重试，
或手动把 jar 放进那个目录。

---

## 2. 为什么要有"兼容层"

两侧是两套各自自洽的模型，落差必须在一处收口：

| | 参考后端 | 本项目 |
|---|---|---|
| 形态 | Spring Boot 多模块（30 个模块 / 206 个类） | Rust 单进程，数据驱动连接类型 |
| 接口粒度 | **506 个具体 URL**（41 个关系型共享模板 × 9 模块前缀 + 137 个独立端点） | 内核只有「执行一条语句」+ 约 20 个通用端点 |
| 元数据 | 每个库一个方言模块各自实现 | 能走内核走内核，其余在兼容层按方言拼 SQL |

兼容层做的唯一一件事，就是**把上游的 URL 映射到内核能力上**。映射不到的地方一律
**显式报错（501 + 一句人话）**，绝不静默返回空数组 —— 空数组会被界面读成「这个库里没有索引」，
而事实是「我们还没做这件事」，那是两种完全不同的坏消息。

### 两套契约，各占各的前缀

| 前缀 | 归属 | 谁在用 |
|---|---|---|
| `/api/…` | **兼容契约** | 前端（它拼的就是这个前缀） |
| `/api/dbmind/…` | 内核原生契约 | CLI / 桌面壳 / 冒烟脚本（保持稳定） |

同一个 URL 只归一方所有。「两个 `/api/connections` 谁生效」这种只能靠运气的问题不存在。

### 代码落点

```
crates/dbmind-web/src/
├── lib.rs            HTTP 壳：路由装配、SPA/404 回落、错误→HTTP 状态
└── api/           ★ 兼容层（/api/… 契约的实现）
    ├── mod.rs        路由表（12 个模块前缀 × 同一批 handler）+ 查询串解析
    ├── shape.rs      ★ 形状转换（内核模型 →上游模型），只在这一层做
    ├── dialect.rs    ★ 方言层：按类型生成元数据 SQL（三档语义见下）
    ├── conn.rs       连接管理 / 测试 / 复制
    ├── sys.rs        驱动 / 设置 / 授权
    ├── meta.rs       元数据、表数据、就地编辑、表与对象操作
    ├── query.rs      SQL 执行、多段执行、取消、执行 SQL 文件（后台任务）
    ├── error.rs      错误出口（`{success:false,message}` + 语义化状态码）
    └── nosql.rs      MongoDB / Redis / Elasticsearch
```

### 形状转换里三个"错了很难发现"的点

写这一层时最费时间的不是逻辑，而是**形状**——编译和类型检查都不管，只有界面看得出来：

1. **行必须是「列名 → 值」的对象**，不是数组。内核给的是按列序的数组；直接透传的结果是
   「列名对、行数对、**每一格都显示 NULL**」。
2. **耗时字段叫 `executeTime`**，不是 `durationMs`。给错名字 = 耗时永远是空。
3. **连接类型必须大写**（`MYSQL`）。前端的类型注册表 `src/types/*.js` 全按大写索引，
   给内核的小写 key 会让 JDBC URL 预览为空、Logo 退化、引用符与默认 schema 全走默认值。

### 方言层（`dialect.rs`）的三档语义

这三档**必须分清**，把后两者混成一个 `None` 就等于「我不知道」伪装成「这里没有」：

| 档 | 含义 | 界面表现 |
|---|---|---|
| `Sql` | 有这个能力，语句在这儿 | 返回真实数据 |
| `Absent` | 该类型**确实没有**这个概念（SQLite 没有存储过程、Redis 没有索引） | 空数组（这是**事实**） |
| `Unwritten` | 概念存在，**我们还没写**（PostgreSQL 的建表语句要自拼约束与索引） | **报错**（501 + 说明） |

---

## 3. 本阶段已实现 / 未实现

**已落地（约 285 个 URL）**

| 分组 | 说明 |
|---|---|
| 连接管理 | 列表 / 详情 / 新建 / 编辑 / 删除 / 复制（服务端复制，**口令一并带过去**）/ 测试连接 |
| 驱动与设置 | 类型清单（16 个，与前端注册表一一对应）、就绪状态、存储路径、打开目录、驱动镜像、旧版 TLS |
| 授权 | 恒为「已授权、不锁定」（本项目是完整版，没有试用期与激活码这一层） |
| 元数据 | 库 / schema / 表 / 视图 / 列 / 建表语句 / 索引 / 存储过程 / 触发器 / 事件 / 用户 / 对象定义 |
| 表数据 | 分页浏览（含真实总数）、关键字模糊筛选、排序、就地新增/修改/删除（按主键定位行） |
| 表与对象操作 | 执行结构变更（ALTER）、清空 / 截断 / 删除 / 重命名表、对象重命名（含 previewOnly 预览） |
| SQL 执行 | 单条执行、多段执行（结果1/结果2…）、取消（硬中断）、执行 SQL 文件（后台任务 + 进度 + 日志） |
| NoSQL | MongoDB / Redis / Elasticsearch 的库 / 集合 / 文档浏览、命令执行、删除集合 |

**未实现（调用时明确 501 + 说明，而不是 404 或静默）**

| 分组 | URL 数 |
|---|---|
| AI 助手与知识库 | 64 |
| 导入导出 / 整库转储 | 90 |
| 备份与还原 | 21 |
| 数据同步 | 4 |
| 数据对比 | 3 |
| 生成测试数据 | 4 |
| 用户管理（详情 / 新建 / 删改） | 3 |
| 实时监控 | 2 |

此外，**方言覆盖是不均匀的**：SQLite / MySQL 系 / ClickHouse / H2 的元数据与 DDL 完整；
PostgreSQL、SQL Server、Oracle、DB2、Derby 的**建表语句与对象定义**需要自己拼约束与索引，
目前如实报错（`dialect.rs` 里标 `Unwritten` 的都是）。

### 连接串（JDBC URL）由谁决定

**后端决定，且只有一处定义**：`plugins/connection-types/*.yaml` 的 `jdbc.urlTemplate`。
前端连接弹窗里那条只读的 JDBC URL **只是预览**（上游的 README 也是这么写的），后端不读它。

因此这里有一条必须守住的纪律：**模板要跟上游各方言的 `buildJdbcUrl` 对齐**，
否则「预览里写着 encrypt=false，实际连接却用了 encrypt=true」——用户看着自己填的东西报错，无从下手。
目前对齐过的类型与理由：

| 类型 | 模板里追加的参数 | 不对齐会怎样 |
|---|---|---|
| SQL Server | `encrypt=false;trustServerCertificate=true` | mssql-jdbc 12.x 默认 `encrypt=true`，连自签证书的库直接 `PKIX path building failed` |
| MySQL / Doris | `useSSL=false&allowPublicKeyRetrieval=true&serverTimezone=Asia/Shanghai&useCursorFetch=true` | MySQL 8 的 `caching_sha2_password` 认证失败；「server time zone value is unrecognized」；大表退化成逐行往返 |
| MariaDB | `useUnicode&characterEncoding=UTF-8&useCursorFetch=true` | 中文乱码；大结果集退化 |
| ClickHouse | `compress=0` | 驱动默认开 LZ4，缺 lz4-java 时握手报 `Magic is not correct` |
| Derby | `;create=true` | 首次连接一个不存在的路径直接失败 |

> ⚠️ **SQL Server 的 `encrypt=false` 是上游既有产品的默认**（其 `SqlServerDialect` 注释写着
> 「兼容纯 JDBC 自签环境」），本项目为保持行为一致而沿用。要强制加密，改
> `plugins/connection-types/sqlserver.yaml` 的 `urlTemplate`（把 `encrypt=false` 去掉或改 `true`）后重新构建。

**还没对齐的**：Oracle（上游用 SID 形式 `@host:port:SID` 且默认 `ORCL`，本项目用服务名形式
`@host:port/service`）、DM、PostgreSQL（上游会补 `reWriteBatchedInserts`，并在库名留空时回落 `postgres`）。
这三处的差异会改变「填什么能连上」，需要真机验证后再动 —— 本机没有可用的 Oracle / DM 实例，
MySQL 那台（192.168.2.216）当时也不可达，所以 MySQL 模板同样是按方言写的、未经真机验证。

### 已知差异（与上游相比，都是刻意的）

1. **一个连接 = 一个库**。上游的树可以列出服务器上所有库并逐个展开；本项目按内核的连接模型
   只提供连接绑定的那个库。多返回几个库节点会诱导用户去点，而那些节点取不到表 —— 比只有一个
   库节点更糟。跨库浏览见第 4 节 Phase 2。
2. **SSH 隧道与自定义 JDBC URL 明确拒绝**（不是静默丢弃）。丢掉字段后假装保存成功，
   会让用户以为配好了，然后往「网络 / 权限」方向查很久。
3. **实时监控页隐藏**（`supported:false`），不给一堆 0 让人以为数据库很闲。
4. **驱动镜像与旧版 TLS 只落盘偏好，暂未下推到内核/宿主**（`driver_artifact_url` 固定在
   Maven Central；旧库 TLS 需要拉起宿主时带 JVM 参数）。

---

## 4. 下一步（按价值排序）

| 阶段 | 内容 | 为什么排这儿 |
|---|---|---|
| **Phase 2** | 导入导出（CSV/Excel/JSON/SQL）、整库转储、跨库浏览（一个连接多库）、PostgreSQL/SQL Server/Oracle/DB2 的建表语句与对象定义 | 导出导入是**日常使用频率最高**的缺口；DDL 决定了「编辑结构」这条路在非 MySQL 系上能不能走 |
| **Phase 3** | AI 助手与知识库（含 SSE 流式）、数据同步、数据对比、备份还原、生成测试数据、实时监控 | 依赖外部服务或长任务编排，工作量按域逐个啃 |
| **持续** | 把兼容层里按方言拼的元数据 SQL **逐项下移到内核**（接口不变，实现下沉），让 CLI / MCP / 桌面壳共享同一份行为 | 现在这些 SQL 只服务 Web 兼容层；下移后三处壳不会再各写一遍 |

---

## 5. 验证方式

```powershell
# 编译（零警告）
cargo build -p dbmind-web

# 端到端冒烟：69 项断言，SQLite 真库，不需要任何外部服务
powershell -ExecutionPolicy Bypass -File scripts\smoke-dbmind.ps1
```

冒烟覆盖的都是**只有真跑一遍才能发现**的东西：结果行是对象还是数组、字段名对不对、
`exportTask` 之类的形状、就地编辑有没有真的写回、多段执行返回几个结果、
未接入能力是不是 501 而不是 404。脚本自己起服务、用临时数据目录、跑完清理。

浏览器侧可以这样快速确认前端真的挂载（而不是白屏）：

```powershell
& "$env:ProgramFiles\Google\Chrome\Application\chrome.exe" --headless=new --disable-gpu `
  --dump-dom --virtual-time-budget=12000 --user-data-dir="$env:TEMP\ui" http://127.0.0.1:20361/
```

---

## 6. 踩过的坑（都写进代码注释了，这里再列一遍）

| 坑 | 表现 | 根因 |
|---|---|---|
| **`std::sync::Mutex` 自死锁** | 执行 SQL 文件后界面一直转圈、状态接口永不返回 | 同一次循环里先 `lock()` 取 guard、又 `lock()` 一次；Mutex 不可重入。锁必须**取完就放**，执行期间绝不持锁 |
| **PowerShell `$ok` 与 `$Ok` 是同一个变量** | 断言**每一项**都失败，而明细里打印的却是正确的值 | PS 变量名不区分大小写，`$ok = $false` 把参数 `$Ok` 自己清成了 false |
| **SPA 回落吞掉 API 404** | 不存在的接口返回 200 + 一坨 HTML，前端只报「请求失败」 | `fallback_service(ServeDir…)` 把 `/api/**` 也回落成了 `index.html`。现在 `/api/**` 未命中 ⇒ 404 + JSON 说明 |
| **`ConvertFrom-Json` 不展开数组** | `@(Get-Json …).Count` 恒为 1 | PS 5.1 把整个数组当一个对象返回；改用 `Invoke-RestMethod` 或手工展开 |
| **Java 宿主建连接慢 5 秒：客户端的反向 DNS** | 展开连接等 5 秒、点开 Tables 等 10 秒（两条会话）；**会话一旦建好就只有 ~275ms** | MySQL 驱动在 `ConnectionImpl` 构造里会调 `InetSocketAddress.getHostName()` —— **把服务端 IP 反解成主机名**。企业 DNS 一般没有这些 PTR 记录，而 Java 对**失败**的解析结果**不缓存** ⇒ 每新建一条会话都白等一次超时。实测同一进程内连建四条，条条如此：反解 **4573ms**，而 `nslookup` 143ms 就回了 NXDOMAIN；同期纯 TCP 建连 29~47ms、驱动类加载 11ms、读版本 58ms ⇒ **时间全花在客户端等待上**，与数据库和网络都无关（服务端的 `skip_name_resolve` 已是 ON，那是另一回事）。**修法**：给宿主 JVM 一份自有 hosts（`-Djdk.net.hosts.file=~/.dbmind/agent-hosts`，由 `agent::prepare_agent_hosts` 生成，内含系统 hosts 全文 + 每个连接的主机 ⇒ 不动系统配置、不需管理员）。同机实测反解 **4573ms → 16ms**，端到端 **10.2 秒 → 1.0 秒**。**定位手段**：`jstack` 抓宿主栈（栈顶直接写着 `Inet6AddressImpl.getHostByAddr`），或在宿主代码里用 `trace()` 写 `%TEMP%\dbmind-agent-jdbc-trace.log`。注意**宿主 stderr 那条链路实测取不到内容**（`~/.dbmind/logs/agent-jdbc.err.log` 一直是 0 字节），排查时别指望它 |
| **结构缓存有「两份」，失效必须一起做** | 新建/删除表之后，树上 15 分钟都看不到变化（计数变了、清单里却没有那张表，反复展开也一样） | 缓存是**两套独立**的：后端 `schema_cache`（SQLite，DDL 后由 `engine::invalidate_schema_after` 作废）与前端 `localStorage`（`utils/schemaCache.js`，15 分钟 TTL）。只清后端的话，前端那份旧清单仍会被「先渲染缓存、后台校准」用上，而后台校准**按设计只更新计数、不动已展开的子节点** ⇒ 表现正是「数字变了、清单里找不到新表」。**规则：后端会作废的地方，前端必须一起作废。** 已接线三处：分类刷新 `refreshCatNode`（先丢旧键、再把新清单写回）、整库重载 `refreshDbNode`（丢 `dbs:`）、SQL 执行出 DDL 后 `SqlQueryView`（丢该连接全部）。改这一带务必两边一起看 |
| **会话键的每一段都要对齐，否则悄悄多一条物理连接** | 一次 `/tables` 建两条连接（元数据一条 + 行数估算一条，各 ~300ms）| 会话键是 `类型\|名字\|主机\|端口\|库\|文件\|read_only`，**任何一段不同就是另一条会话**。结构浏览对非 connection-scoped 的类型走 `metadata_session_key` 里**强制**的 `session_key(config, true)`，而 `execute` 走的是连接策略标记（默认 false）—— 于是同一库同一件事占了两条连接。修法：`QueryRequest` 加了 `read_only: Option<bool>`，内部浏览类语句填 `Some(true)` 与结构浏览共用只读会话（缺省 `None` = 原行为）。**改这一带时先想清楚"这些调用该不该同一条会话"**，别只看功能对不对 —— 功能对、连接翻倍，只有抓会话数才看得出来（`jstack` 或宿主 trace） |
| **含中文的 `.ps1` 需要 UTF-8 BOM** | 脚本报一堆莫名其妙的语法错误 | PS 5.1 按本地代码页读 `.ps1`，无 BOM 时中文与全角括号会把词法分析搞乱。**关键症状：这是解析期错误，整个脚本一行都不会执行** —— `start.ps1` 曾因此完全静默（连开头的 `Write-Host` 都没打出来），看着像"服务起不来"，实际是脚本压根没跑。`scripts/` 下的 `.ps1` 一律存为 UTF-8 with BOM |
| **原生命令的 stderr 会变成异常** | 某个断言莫名失败 | `$ErrorActionPreference='Stop'` 下 curl 之类往 stderr 写东西会被当异常抛出 |

---

## 7. 目录结构

```
DBMind/
├── crates/
│   ├── dbmind-core/      内核（独立 crate，不依赖 web 层）
│   ├── dbmind-web/       HTTP 壳：内核原生契约 + /api/… 兼容层（src/api/）
│   ├── dbmind-cli/       命令行壳
│   ├── dbmind-mcp/       MCP 壳
│   └── dbmind-desktop/   Tauri 桌面壳（内嵌 HTTP 内核 + 系统 WebView）
├── frontend/             前端源码（由上游/frontend 持续改造而来）
│   └── dist/             前端产物（由本项目 `npm run build` 产出）
├── agents/               四个 Java 宿主源码（通用 JDBC / MongoDB / Redis / Elasticsearch）
├── plugins/connection-types/   连接类型单一真源（17 个 YAML，编译期生成 Rust 枚举）
├── deploy/               契约级测试替身（Mongo / ES / Redis 测试服务）
├── scripts/
│   ├── smoke-dbmind.ps1  ★ 端到端冒烟（69 项）
│   ├── build-agents.ps1  构建四个宿主 jar
│   ├── start.ps1 / stop.ps1   起停后端（★ 必须存为 UTF-8 with BOM，见第 6 节）
│   ├── release.bat / .ps1     ★ 一键出两版：绿色版 + 桌面安装包（双击 .bat 即可）
│   └── package.ps1            组装可分发目录（release.ps1 内部调用）
└── target/               构建产物
```

---

## 8. 前端产物怎么更新

`frontend/dist` 由本项目构建产出。改过 `frontend/src` 之后重新构建：

```powershell
cd frontend
npm install        # 首次
npm run build      # 产出新的 dist
```

再把新 `dist` 交给后端即可（`--dist frontend\dist`，无需打包进可执行文件）。

Java 宿主（连接关系型数据库时要下载驱动）需要先构建：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-agents.ps1
```

内核按 `DBMIND_AGENT_JAR` > `DBMIND_AGENTS_DIR` > 可执行文件旁 `agents/` 的顺序找宿主包，
缺了会如实报 `DBMIND-DRV-0002` 并说明缺哪个。
