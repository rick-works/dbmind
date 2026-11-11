# DBmind

[English](README.md) | 简体中文

🌐 **官网**：<https://www.dbmind.top/> · 🏠 **仓库**：<https://gitee.com/rick-works/dbmind>（GitHub 镜像：<https://github.com/rick-works/dbmind>）

[![release](https://img.shields.io/badge/release-v2.0.21-0a7cff)](https://gitee.com/rick-works/dbmind/releases)

**给数据库装上「大脑」** —— DBmind（Database + Mind）是 AI 原生的多数据库管理与开发平台：
一个客户端管好你的每一个数据库，AI 帮你解释、优化、修好每一条 SQL。

本地运行、数据不出内网。Rust 内核 + Vue 3 前端 + Java 宿主（JDBC 驱动桥），提供 Web / 桌面 / CLI / MCP 四种使用形态。

## 功能一览

| 模块 | 说明 |
|---|---|
| **🤖 AI 深度集成** | 解释 / 优化 / 改写 / 修错 / 诊断 / 对话六大动作；AI 的 SQL 先看执行计划、试跑验证；MCP 服务器把整个工作台接进任意 AI 客户端（详见 [AI 能力](#ai不只是聊天)） |
| **🧠 AI 同事** | 「自主排查」自己查库：巡检表结构、给索引建议、看同步与备份状态、只读试跑，把要改的动作做成**变更单**等你批准（高危二次确认、执行完可撤销）；长期记忆记住业务口径，对话记录可续可翻，每条回答脚注 token 消耗 |
| 字段脱敏 | 按「表 + 列」把敏感字段换成 **MD5 / SHA-256 摘要**或 **AES-256 密文**（`f3a9c2…`），一份规则四个出口各自开关：**发给 AI 的内容**（上下文样例行 + 智能体只读结果）、**MCP 客户端拿到的数据**、**结果表格的显示**（只影响显示）、**导出的文件**（CSV / Excel / JSON / SQL 里落盘的就是打码值）；支持 `*_phone` 通配与「加入常见敏感列」，库里的数据一个字节都不动 |
| 多数据源 | **17 种引擎，一个客户端** —— 关系型（MySQL、MariaDB、PostgreSQL、人大金仓、Oracle、SQL Server、DB2、Derby、H2、达梦）、分析型（ClickHouse、Doris）、嵌入式 / 文件（SQLite、DuckDB）与 NoSQL（Redis、MongoDB、Elasticsearch）；驱动按需下载，各引擎的能力差异见 [支持的数据源](#支持的数据源) |
| SQL 编辑器 | Monaco 内核：补全、格式化、多语句分块执行、逐段计时、执行历史、SQL 片段库（右键「保存片段 / 插入片段」，存本机跨会话保留）、模板变量（`:name` 执行时填参）、执行中随时停止；13 组命令快捷键（执行 `Ctrl+Enter`、格式化 `Shift+Alt+F`、注释 `Ctrl+/`、复制为 `IN` 列表 `Ctrl+Shift+X`…），全部可改键，见 [快捷键](#快捷键) |
| 事务模式 | 写语句不自动提交：改完自查 → 回滚或提交；事务精确绑定编辑器会话，全 JDBC 引擎通用；关闭事务模式会先回滚，SQLite 没有会话级事务则直接隐藏按钮 |
| 结果集直接编辑 | 双击单元格改值 → 缓冲 → 批量 UPDATE（可落在事务里）；快照守卫在翻页后作废旧缓冲，宁可让你重改也不改错行。行数上限默认不限（可设 100 ~ 100000），语句超时 300 秒（可调 1 ~ 600） |
| 大结果集 | 虚拟滚动 + 服务端分页 + 响应压缩（实测 4.2MB→233KB）；总数异步统计不阻塞回显 |
| 表结构与对象 | 字段类型、默认值、注释、索引在表格里直接改，执行前先看到完整 `ALTER` 预览；新建表 / 索引 / 视图 / 触发器先出 DDL 再执行；做不到的项置灰并说明原因（SQLite 只能改列名，ClickHouse / Doris 的表选项建表后不可改） |
| 用户与权限 | 各引擎的用户 / 角色管理，按作用域（全局 / 服务器 / 库 / 模式 / 表 / 例程）勾选权限；每一次勾选都预览精确的 `GRANT` / `REVOKE` 差量，提交时只发变化的部分；读回来的权限名一个都不隐身；Db2 / Derby 的用户来自外部认证，界面如实说明 |
| 服务监控 | 会话、锁与库级指标，可就地终止会话 —— 覆盖 MySQL / MariaDB / PostgreSQL / 金仓 / SQL Server / ClickHouse / Doris；其余引擎返回「不支持 + 原因」，不画空图 |
| ER 关系图 | 真实外键优先，命名约定推断的连线标注置信度；拖拽布局、滚轮缩放、搜索高亮、导出 PNG / SVG；纯规则推断，不调模型、零 token |
| 数据对比与传输 | 结构差异细到类型 / 长度 / 可空 / 默认值 / 主键 / 索引 / 外键；数据按主键或唯一键逐行比（样本 10 ~ 500 行，单侧约 200 万行，二进制列自动排除），再用向导传输表 / 视图 / 例程 / 触发器 / 事件，写入模式 `insert` / `truncate_insert` / `upsert`。同实例走库内直拷 —— 20 万行 6 列实测 6 秒（走客户端 239 秒）；跨实例靠分片 + 并发 + 链路压缩，同样数据 239 秒 → 35 秒 |
| 数据传输 | 导出 CSV / Excel / JSON，另有 INSERT / UPDATE / DELETE 语句与纯 DDL；导出无行数上限（每批 5 万行），多表导出打包成 zip；导入 CSV / JSON / xlsx（单文件 256 MB、每批 200 行，老式 `.xls` 明确不支持） |
| 任务中心 | 导出 / 导入 / 整库转储 / 对比 / 同步 / 备份 / 还原 / 造数全部后台跑，实时显示阶段、行数与速率；每个任务保留 4000 行日志；取消会真的停掉两端，并如实标为「已取消」而不是失败；产物落临时目录，取回即删 |
| 备份与造数 | 各引擎原生 dump / restore，按列类型生成测试数据 |
| 对象搜索 | 不用展开树，直接在整个连接里搜表 / 视图 / 存储过程 |
| 生产库保护 | 连接标记 PROD 后：标记全程外显；执行脚本、改结构、编辑数据、导入 / 同步 / 还原等一切改动数据的操作默认拦下（查询不受影响）；写操作二次确认、执行全程审计 |
| 数据透视 | 框选即汇总（计数 / 求和 / 均值 / 最值）、分组下钻；纯前端在已取回的结果上算，不再压数据库（保留 2000 组，TOP-N 10 ~ 100） |
| 驱动管理 | 从 Maven 下载 / 更新 / 修复驱动，带实时进度与文件名；内网可指向私有镜像或手动放 jar；驱动跑在独立 Java 17+ 宿主里，坏驱动带不倒客户端 |
| 数据目录自愈 | 开机后台回收元数据库空洞（空闲页 ≥8 MB 且占比 >20% 才动手）、清理 24 小时前的装载残留、任务快照只留最新 500 份（实测 805 MB → 54 MB）；只还空洞，不删查询历史与审计；清空日志 / 缓存当场回收 |

## 支持的数据源

| 分组 | 引擎 |
|---|---|
| 关系型 · 10 | MySQL、MariaDB、PostgreSQL、人大金仓 KingbaseES、Oracle、SQL Server、IBM Db2、达梦 DM、Apache Derby、H2 |
| 分析型 · 2 | ClickHouse、Doris —— 在连接列表里单独归为 OLAP |
| 嵌入式 / 文件 · 2 | SQLite、DuckDB —— 直接打开本地文件，不需要服务端与 Java 宿主 |
| NoSQL · 3 | Redis、MongoDB、Elasticsearch |

不是每个引擎都「样样齐全」，做不到的会明说，不装作支持：

- **会话事务**：走 JDBC 宿主的引擎都支持；SQLite 用内核原生驱动，没有会话级事务，按钮直接不渲染
- **用户与权限**：MySQL / MariaDB / PostgreSQL / 金仓 / SQL Server / Oracle / ClickHouse / Doris / H2 等按各自方言管理；Db2、Derby 的用户来自外部认证，界面如实说明
- **服务监控**：MySQL / MariaDB / PostgreSQL / 金仓 / SQL Server / ClickHouse / Doris；其余引擎返回「不支持 + 原因」，不画空图
- **表结构编辑**：能力按引擎下发，做不到的项置灰并说明原因（SQLite 只能改列名；ClickHouse / Doris 的表选项建表后不可改）
- **数据传输**：关系型与分析型全部支持；SQLite / DuckDB / H2 / Derby 与三种 NoSQL 没有对端写入通道，入口不出现
- **DDL 查看**：能原生取到定义的引擎直接给；Db2 / Derby 按列元数据自拼一份，不让「查看 DDL」点开是空的

## 界面截图

浅色 / 深色两套主题，每个页面都能随时切换。

| 界面 | 浅色 | 深色 |
|:--|:-:|:-:|
| **工作台**<br>连接树、最近查询与趋势 | <img src="docs/images/home-light.png" width="330" alt="工作台（浅色）"> | <img src="docs/images/home-dark.png" width="330" alt="工作台（深色）"> |
| **SQL 编辑器**<br>补全与逐段计时 | — | <img src="docs/images/sql-editor-dark.png" width="330" alt="SQL 编辑器（深色）"> |
| **表数据浏览**<br>高级搜索与分页 | — | <img src="docs/images/table-data-dark.png" width="330" alt="表数据浏览（深色）"> |
| **AI 助手**<br>问数出图与结论 | — | <img src="docs/images/ai-assistant-dark.png" width="330" alt="AI 助手（深色）"> |
| **数据传输**<br>源目标配对与替换 | <img src="docs/images/data-transfer-light.png" width="330" alt="数据传输（浅色）"> | <img src="docs/images/data-transfer-dark.png" width="330" alt="数据传输（深色）"> |
| **结构与数据对比**<br>差异逐项列出 | <img src="docs/images/compare-light.png" width="330" alt="结构与数据对比（浅色）"> | <img src="docs/images/compare-dark.png" width="330" alt="结构与数据对比（深色）"> |
| **服务监控**<br>会话、QPS 与锁 | <img src="docs/images/monitor-light.png" width="330" alt="服务监控（浅色）"> | <img src="docs/images/monitor-dark.png" width="330" alt="服务监控（深色）"> |
| **用户与权限**<br>授权与回收 | <img src="docs/images/users-light.png" width="330" alt="用户与权限（浅色）"> | <img src="docs/images/users-dark.png" width="330" alt="用户与权限（深色）"> |
| **任务中心**<br>历史任务与耗时 | <img src="docs/images/tasks-light.png" width="330" alt="任务中心（浅色）"> | <img src="docs/images/tasks-dark.png" width="330" alt="任务中心（深色）"> |
| **驱动管理**<br>按需下载与上传 | <img src="docs/images/drivers-light.png" width="330" alt="驱动管理（浅色）"> | <img src="docs/images/drivers-dark.png" width="330" alt="驱动管理（深色）"> |
| **AI 模型配置**<br>多模型、温度与用量 | — | <img src="docs/images/models-dark.png" width="330" alt="AI 模型配置（深色）"> |
| **脚本管理**<br>脚本与模板变量 | — | <img src="docs/images/scripts-dark.png" width="330" alt="脚本管理（深色）"> |
| **表右键菜单**<br>常用动作就地可达 | — | <img src="docs/images/menu-table-dark.png" width="330" alt="表右键菜单（深色）"> |

## AI，不只是聊天

大多数工具的「AI 助手」是个聊天框；DBmind 把 AI 做进了**每一步数据库操作**：

- **六大即用动作** —— 选中 SQL 一键「解释 / 优化 / 改写」；执行报错一键「AI 修错」（自动带上完整错误上下文）；排查性能问题用「诊断」；复杂任务直接对话
- **AI 是个同事，不是问答框** —— 打开「自主排查」，它自己调工具：巡检表结构、找该加的索引、查同步与备份状态、只读试跑一遍；要动的数据统一生成**变更单**，批准才执行（高危再确认一次），执行完还能撤销。多轮能接着上一轮结论往下做，问两张表就展开两张
- **它会记住你的口径** —— 说一句「记住：本库金额单位是分」，下次在同一个库提问自动带上；记忆分「你说过 / AI 推断」，随时可删。对话有记录，能续上一轮接着说
- **上下文按预算取，不是把整个库塞进去** —— 大库不再塞几百个表名；问「这个库是干嘛的」这类库级问题，改成给全库清单（表名 + 表注释，395 张表约 2.7k tokens，而整库列结构要几十万）。实测同一条修复从 2345 降到 956 prompt tokens、对话从 2404 降到 1030；每条回答脚注花了多少 token，还有一份零成本账本告诉你「这次请求花在哪」
- **AI 的答案先验证再用** —— AI 给出的 SQL 可一键查看**执行计划**、**试跑一次**（不消耗 AI 调用），确认无误才落库，hallucination 进不了你的数据库
- **AI 通道默认只读** —— AI / MCP 通道的写操作默认阻断、需显式放开：再聪明的模型也绕不过安全策略
- **MCP 服务器** —— 内置 `dbmind-mcp`（stdio）：把查询、元数据、结构浏览作为**工具**暴露给任意支持 MCP 的 AI 客户端（Claude Desktop 等），让 AI 直接「会查你的库」
- **模型自己定** —— 接口地址与密钥由你配置，对话内容只经过你指定的服务，不出内网

## AI 到底把什么发出去

担心数据，多半是因为「不知道它到底发了什么」。这里不写「我们很重视隐私」这种话，直接把一次请求摊开——你可以据此判断能不能接外部模型。

**DBmind 没有任何中转服务器**：请求从你本机直连你在「设置 → AI」里填的接口地址（OpenAI 兼容协议）。AI 默认关闭，不配模型时不会发出一个字节。

**会发出去的（一次提问的全部内容）**

| 内容 | 具体是什么 | 什么时候带 |
|---|---|---|
| 你的问题 | 提问原文、当前 SQL、报错文本，加上最近 6 条对话（合计上限 6000 字符，单条超长掐中间） | 每次提问 |
| 表结构 | 表名 + 列名 / 类型 / 是否可空 / 默认值 / 主键（**不含列注释**） | 提问点名或命中的表，最多 40 张；没点名默认前 8 张 |
| 一行样例 | 每张展开的表 **1 行真实数据**，最多前 20 列，每个值截到 24 字符 | 跟着表结构一起 |
| 字段脱敏（可关） | 被规则命中的列换成 MD5 / SHA-256 摘要或 AES-256 密文（`f3a9c2…`） | 设置里配了规则且打开开关时 |
| 全库清单 | 表名 + 表注释（不含列结构） | 问「这个库是干嘛的」这类库级问题 |
| 团队术语 / 示例 | 你自己在「AI 知识」里录的术语与采纳过的 SQL | 有配置时 |
| 长期记忆 | 你说过的口径（如「金额单位是分」） | 该连接 / 库下有记忆时 |
| 知识库片段 | 命中的文档片段，每段 ≤900 字符并标注来源 | 配了知识库且命中时 |
| 内置提示词 | 人格与方言规则模板（纯文本，不含你的任何数据） | 每次 |

**不会发出去的**

- **整表数据与查询结果** —— 只有上面那 1 行样例。唯一例外：用「自主排查」让它查库时，它执行的只读 SQL 会返回最多 100 行、并裁剪到 3200 字符进入对话上下文（它要「自己看数据」就得付这个代价，这一点写在明面上）
- **连接密码 / SSH 口令 / API Key** —— 只存在本机 `~/.dbmind/dbmind.db`，接口读取时只回「已保存」、不回显
- **遥测** —— 没有使用统计、没有崩溃上报、不请求 DBmind 的任何服务器（「检查更新」只查版本号，与 AI 无关）
- **别的连接、别的库** —— 上下文只来自你这次提问所用连接与库

**觉得样例也不该发？那就收紧**

- **仅本地模型**：设置里打开后，只要接口地址不是内网/本机就直接拒绝（并报出被拦的 host）。放行范围：`localhost` / 回环、IPv4 私有段（10 / 172.16–31 / 192.168 / 169.254）、IPv6 ULA（fc00::/7）、`.local/.internal/.lan/.corp/.intra` 后缀、以及 `DBMIND_AI_LOCAL_HOSTS` 里你自己加的域名。对话与知识库向量化**都**受这条约束——内网起的 Ollama / vLLM 照样能用，云端模型则彻底出不去
- **缩小表范围**：提问里点名表名就只给那几张；库级问题只给「表名 + 注释」，一行数据都不带
- **开工前先看**：`POST /api/ai/preview` 一次模型都不调，返回这次要发多少 token、各段各占多少——界面上的「这次请求花在哪」就是它
- **不配就不用**：知识库不建＝零召回，不写记忆＝零记忆，不点「自主排查」＝不会执行任何查询

**真实发出去的一段（脱敏）**

```
【目标数据库】类型 MYSQL ｜ 版本 5.7.42 ｜ 标识符引用风格 反引号 ｜ 分页写法 `limit {offset}, {limit}`
【表清单】共 42 张表，仅展开用到的那几张（需要其它表的字段请先说明表名）
【表 orders】
  id BIGINT not null 主键
  status VARCHAR(16) not null default 'NEW'
  amount BIGINT not null default 0
  created_at DATETIME not null
  样例：id=10231, status=PAID, amount=19900, created_at=2026-10-30 12:00:00
```

这一行样例**是真实数据**（截断到 24 字符）：模型靠它判断日期格式与状态枚举，写出来的 SQL 才不会把 `PAID` 当成 `1`。如果表的前 20 列里就有手机号、身份证号这类字段，它们会随之发出。

**不想让样例出门？** 设置 →「数据脱敏」按「表 + 列」配规则（支持 `*_phone` 这类通配，「加入常见敏感列」可一键补齐手机号 / 身份证 / 邮箱 / 密码 / Token / 银行卡），命中的列在发出去之前就换成 MD5 / SHA-256 摘要（`f3a9c2…`）或 AES-256 密文（想解回去就选 AES，密钥在本机）。规则只有一份，**四个开关**决定它作用在哪儿：发给 AI 的内容（上下文样例行 + 智能体只读查询结果）、MCP 服务返回给外部客户端的数据、DBMind 自己的结果表格（只影响显示）、导出的文件（CSV / Excel / JSON / SQL 里落盘的就是打码值）—— 只堵一处的话，模型自己 `select` 一次就绕过去了。五条边界照实说：

- 表名 / 列名 / 表注释**不**脱敏 —— 脱了模型无从判断该查哪张表，功能直接失效
- 你在 SQL 里手写的字面量（`where phone = '138…'`）**不**脱敏，那是你的原文
- 规则按 表 + 列 匹配，改表名 / 换库后要自己维护；「表」留空表示任意表，供智能体结果兜底
- 结果表格那个开关**只改显示**：双击编辑、复制拿到的都是原文 —— 它防的是「旁边有人一眼看到」；要文件里也是打码值，打开「数据导出」那个开关
- 打开「数据导出」后，生成的 UPDATE / DELETE 脚本里**主键列保持原文**：那些值是 `where` 的定位条件，打成摘要后语句永远匹配不到行（INSERT / CSV / Excel 则整行都打码）

连「掩码后的样例」都不想发，就把端点锁在内网（[仅本地模型](#ai-到底把什么发出去)），数据不出你的网络。

## 快捷键

全部可在「设置 → 快捷键」里改键，存在本机，不写进配置文件。

| 动作 | 默认键位 |
|---|---|
| 执行当前语句 / 执行全部 | `Ctrl+Enter` / `Ctrl+Shift+Enter` |
| 停止执行 | `Ctrl+F2` |
| 格式化 | `Shift+Alt+F` |
| 注释 / 取消注释 | `Ctrl+/` |
| 保存 / 新建脚本 | `Ctrl+S` / `Ctrl+N` |
| 转大写 / 转小写 | `Ctrl+Shift+U` / `Ctrl+Shift+L` |
| 压成一行 | `Ctrl+Shift+M` |
| 复制为 `IN` 列表 | `Ctrl+Shift+X` |
| 复制当前行 / 选中当前语句 | `Alt+Shift+L` / `Ctrl+Shift+Y` |
| 结果表格：刷新 / 加行 / 删行 | `F5` / `Alt+Insert` / `Ctrl+Delete` |
| 结果表格：向下填充 / 清空单元格 / 自适应列宽 | `Ctrl+D` / `Delete` / `Alt+Shift+A` |
| 结果表格：复制为 `IN (...)`（右键「复制为」） | 无快捷键，选中一列后右键 |

## 架构

```
┌──────────────┐  ┌──────────────┐  ┌──────────┐  ┌──────────┐
│  桌面版       │  │  浏览器版     │  │ CLI      │  │ MCP      │
│ dbmind-      │  │ dbmind-web   │  │ dbmind   │  │ dbmind-  │
│ desktop.exe  │  │ .exe + 浏览器 │  │          │  │ mcp      │
│ (Tauri 壳)   │  │ (axum 静态托管)│  │          │  │ (stdio)  │
└──────┬───────┘  └──────┬───────┘  └────┬─────┘  └────┬─────┘
       └────────────┬────┴───────────────┴─────────────┘
                    ▼
        ┌───────────────────────────┐
        │   dbmind-core（Rust 内核）  │   连接池 / 泳道会话 / 安全策略 /
        │   SQL 切句 · 元数据 · 存储  │   审计 · 计数 · 导出传输
        └──────┬─────────────┬──────┘
               ▼             ▼
     ┌──────────────┐  ┌──────────────────────┐
     │ 原生驱动       │  │ agent 宿主（Java 17+）│
     │ SQLite        │  │ JDBC 桥 → MySQL/PG/   │
     │ (rusqlite)    │  │ Oracle/MSSQL/CH/Doris │
     │               │  │ Redis/Mongo/ES 客户端  │
     └──────────────┘  └──────────────────────┘
```

- **内核**（`crates/dbmind-core`）：所有端共用同一内核，行为完全一致。含连接池与泳道会话（编辑器亲和、结构浏览共享）、只读/写操作安全策略、SQL 按方言切句、元数据缓存、元数据存储。
- **Java 宿主**（`agents/`）：四大协议桥（JDBC / MongoDB / Redis / Elasticsearch）。内核按需拉起宿主进程，通过 stdin/stdout JSON 协议通信；驱动 jar 自动从 Maven 下载。
- **前端**（`frontend/`）：Vue 3 + Monaco + Element Plus，单页应用；web 壳直接托管静态产物。

## 目录结构

```
├── crates/
│   ├── dbmind-core      内核（连接、执行、策略、存储、导出）
│   ├── dbmind-web       Web 壳（axum API + 静态托管，端口 20361）
│   ├── dbmind-desktop   桌面壳（Tauri，内嵌内核）
│   ├── dbmind-cli       命令行（conn / driver / setting）
│   └── dbmind-mcp       MCP 服务器（stdio，接 AI 客户端）
├── agents/              Java 宿主（jdbc / mongodb / redis / elasticsearch）
├── frontend/            Vue 3 前端
├── plugins/             数据库类型声明（JDBC URL 模板、驱动清单）
├── resources/           原生依赖（SQL Server 集成认证 dll）
├── scripts/             日常入口（start/stop）+ build / release / test / tools 四类脚本
├── deploy/              集成测试用真实服务端（redis / es / mongo）
└── website/             产品官网（单文件静态页）
```

## 环境要求

| 工具 | 用途 | 版本 |
|---|---|---|
| Rust (rustup) | 内核与各端 | stable |
| Node.js + npm | 前端构建 | 18+ |
| JDK + Maven | 构建 Java 宿主（SQLite 之外的数据源需要） | JDK 17+ / Maven 3.6.3+ |

> 数据库驱动 jar 会按需从 Maven 自动下载到 `~/.dbmind/`，无需手动装。

## 快速开始（开发模式）

```powershell
# 一键启动：自动构建（如缺产物）+ 起服务 + 打浏览器
powershell -ExecutionPolicy Bypass -File scripts\start.ps1

# 或者手动分步：
cargo build -p dbmind-web          # 编译 Web 壳
cd frontend; npm install; npm run build   # 前端产物
cargo run -p dbmind-web -- --port 20361 --dist frontend\dist
```

- Web 版地址：`http://127.0.0.1:20361`
- 前端热更新开发：`cd frontend; npm run dev`（vite 代理到 20361）

## 构建各端

```powershell
# 桌面版（Tauri 壳）
cargo build --release -p dbmind-desktop

# Web 壳
cargo build --release -p dbmind-web

# CLI
cargo build --release -p dbmind-cli

# MCP 服务器
cargo build --release -p dbmind-mcp

# Java 宿主（4 个模块各跑一次）
mvn -B -q -DskipTests package -f agents\dbmind-agent-jdbc\pom.xml
```

## 打包发布

**发行包由 CI 产出**：推 `v*` 标签触发 `.github/workflows/release.yml`，一次跑出全平台产物（portable zip、MSI、NSIS、dmg、deb、rpm、AppImage）并挂到同一个 Release。

本地一键打包脚本（`release.ps1` / `release.bat`）已移除。要在本机拼一个发行目录，按 CI 的步骤来：

```powershell
cargo build --release
cd frontend; npm install; npm run build; cd ..
foreach ($m in 'jdbc','mongodb','redis','elasticsearch') { mvn -B -q -DskipTests package -f "agents\dbmind-agent-$m\pom.xml" }
powershell -File scripts\build\package.ps1 -SkipCargo -SkipAgents -SkipFrontend
```

`scripts\build\package.ps1` 负责组装发行目录（CI 用的也是它），`scripts\build\dmg-settings.py` 是 macOS DMG 的外观配置。

> 完整发版流程 —— 版本级别怎么选、`release.mjs` 改哪 6 处、要点为什么必须在打标签前落进代码、CI 完成后回填 Release 正文与官网、同步 Homebrew cask、验证清单 —— 见 [`docs/build.md` 的发版流程一节](docs/build.md#发版流程一步不落)。

### CI：全平台、各版本形态齐全

`.github/workflows/release.yml` 在推 `v*` 标签时触发（也可手动触发并指定 `tag`），把各平台的包挂到同一个 Release：

| Runner | 包 |
|---|---|
| `windows-latest` | `DBmind_<版本>_x64_en-US.msi`、`DBmind_<版本>_x64-setup.exe`（NSIS 安装向导）、**`dbmind-<版本>-portable.zip`**（免安装：解压即用，内含 `agents/` 与 `jre/`，目标机器不用装 Java） |
| `macos-latest` | `DBmind_<版本>_aarch64.dmg`（+ 同名 `.app`，`.app` 本身就是免安装形态：拖进「应用程序」即可） |
| `macos-15-intel` | `DBmind_<版本>_x64.dmg`（+ `.app`）—— `macos-13` 已退役，Intel 现在必须用带 `-intel` 后缀的标签 |
| `ubuntu-22.04` | `dbmind_<版本>_amd64.deb`（apt 安装）与 `*.AppImage`（**免安装**：`chmod +x` 后直接跑） |

### macOS 首次打开会被系统拦一次（正常，不是坏了）

macOS 对「从网上下载、且没有经过 Apple 公证」的 App 一定会拦一次 —— 这是系统规则，应用侧绕不过。
装的时候：点「好」→「系统设置 → 隐私与安全性」→ 在「安全性」里点「**仍要打开**」→ 确认，
之后双击就能正常打开。**不用 sudo、不用敲命令**（DMG 里也附了这份说明：`首次打开请看这里.txt`）。

想**一次都不弹**：弹窗的开关其实是文件上那个「从网上下载」的标记（`com.apple.quarantine`），
而它是**下载它的那个程序**打上去的 —— 换个方式拿到文件，系统就不会去检查：

```bash
# ① curl（curl 不打这个标记）。把 <版本> 换成发布页上的最新版本号，例如 1.2.7
#    国内走 Gitee（快）；Gitee 单附件上限 100MB，若该文件没同步到，把域名换回 github.com
curl -LO https://gitee.com/rick-works/dbmind/releases/download/v<版本>/DBmind_<版本>_aarch64.dmg
open DBmind_<版本>_aarch64.dmg     # 挂载 → 拖进「应用程序」→ 双击，全程不弹
#    （Intel 机器把文件名里的 aarch64 换成 x64）

# ② Homebrew cask：--no-quarantine 会替你摘掉那个标记
brew install --cask --no-quarantine rick-works/tap/dbmind
```

另外两点容易白折腾，写在这里省得再试：**签名不能去掉**（Apple 芯片上最少要 ad-hoc 签名，
去掉不是"少一个弹窗"而是"打不开"，系统会说"已损坏"）；**签名 + 公证都齐时，第一次打开也仍会弹一次**，
只是换成带「打开」按钮的那种（点一下就进）—— 公证需要付费的 Apple 开发者账号（$99/年，见下表）。

代码签名靠仓库 secrets 按需开启，**没配就出未签名包**（不影响构建）：

| Secret | 平台 | 用途 |
|---|---|---|
| `APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`KEYCHAIN_PASSWORD` | macOS | 用 **Developer ID Application** 证书签名（base64 的 `.p12`） |
| `APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID` | macOS | 公证（需付费 Apple Developer 账号，密码为 App 专用密码） |
| `WINDOWS_CERTIFICATE`、`WINDOWS_CERTIFICATE_PASSWORD` | Windows | 用本机证书存储里的证书签 exe / msi / NSIS（`signtool`，base64 的 `.pfx`） |
| `AZURE_SIGN_ENDPOINT`、`AZURE_SIGN_ACCOUNT`、`AZURE_SIGN_PROFILE`、`AZURE_SIGN_DESCRIPTION` + `AZURE_CLIENT_ID`、`AZURE_TENANT_ID`、`AZURE_CLIENT_SECRET` | Windows | **Azure Artifact Signing**（原名 Trusted Signing）云签名 —— 私钥在硬件模块里、根本拿不到 `.pfx` 的证书走这条 |

证书有效期是**规矩**不是意外，请按“定期轮换”来规划：

- **Windows 代码签名证书：上限 460 天**（CA/B Forum 的 CSC-31，2026-03-01 起签发的证书一律不得超；此前是 39 个月）。且 2023-06-01 起私钥必须存放在硬件加密模块里 —— 这正是新证书通常没有 `.pfx` 可导、需要上面那条云签名的原因。
- **Apple**：**$99/年会员资格必须持续有效**才能签新包（Developer ID 证书本身 5 年、描述文件 18 年，每类最多 5 张）。
- **“永久证书”不存在**：私钥一旦泄露无法撤回，只能靠到期强制失效兜底；而**过期前签好的包永远能用** —— 签名里带时间戳，校验的是“签名那一刻证书有效”，不是现在。换证就是：把新证书填进 secret，重跑一次。

<details>
<summary><b>每个 secret 怎么生成</b></summary>

**macOS —— 不需要 Mac，Windows 上用 OpenSSL 就够。**

1. 加入 Apple Developer Program（$99/年），打开 *Certificates, Identifiers & Profiles → Certificates → + → **Developer ID Application***，上传 CSR：
   ```bash
   openssl req -new -newkey rsa:2048 -nodes -keyout apple.key -out apple.csr \
     -subj "/CN=你的名字/emailAddress=你的Apple账号邮箱"
   ```
2. 下载 `developerid_application.cer`，拼出 `.p12`（把 Apple 的中间证书一起链上）：
   ```bash
   curl -O https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer
   openssl x509 -inform DER -in DeveloperIDG2CA.cer -out ca.pem
   openssl x509 -inform DER -in developerid_application.cer -out cert.pem
   openssl pkcs12 -export -inkey apple.key -in cert.pem -certfile ca.pem -out apple.p12
   ```
   （有 Mac 的话更简单：双击 `.cer` 装进钥匙串，再从「我的证书」里导出 `.p12`。）
3. 转 base64：macOS `openssl base64 -A -in apple.p12 -out apple.b64`；Windows `certutil -encode apple.p12 apple.b64`。把**整个文件内容**贴进 `APPLE_CERTIFICATE` —— 带不带 `-----BEGIN-----` 边界行都行，工作流会自己剥掉。
4. `KEYCHAIN_PASSWORD` 只是 CI 临时钥匙串的密码，随手一串随机字符即可。
5. 公证：`APPLE_PASSWORD` 填**App 专用密码**（appleid.apple.com → 登录与安全），`APPLE_TEAM_ID` 在 Membership 页面。

**Windows（本机证书）**：`certutil -encode code.pfx code.b64` → 整份内容给 `WINDOWS_CERTIFICATE`，密码给 `WINDOWS_CERTIFICATE_PASSWORD`。

**Windows（云签名）**：开通 Azure Artifact Signing（约 $9.99/月），记下 endpoint / account / 证书配置文件（profile）三个名字；再建一个 Entra ID 应用注册并授予 *Artifact Signing Certificate Profile Signer* 角色，把它的 client id / tenant id / client secret 填进 `AZURE_*` 那几个 secret。工作流会自己装 `artifact-signing-cli` 并接到 Tauri 的 `signCommand` 上。

```powershell
$repo = 'rick-works/dbmind'
gh secret set APPLE_CERTIFICATE           --repo $repo --body (Get-Content apple.b64 -Raw -Encoding ASCII)
gh secret set APPLE_CERTIFICATE_PASSWORD  --repo $repo   # 会提示输入
gh secret set KEYCHAIN_PASSWORD           --repo $repo
gh secret set APPLE_ID                    --repo $repo
gh secret set APPLE_PASSWORD              --repo $repo
gh secret set APPLE_TEAM_ID               --repo $repo
gh secret set WINDOWS_CERTIFICATE           --repo $repo --body (Get-Content code.b64 -Raw -Encoding ASCII)
gh secret set WINDOWS_CERTIFICATE_PASSWORD  --repo $repo
gh secret list --repo $repo
```

配好后重跑工作流即可 —— 手动触发时把 `tag` 输入留空，签名产物只进 Actions artifacts，不动已发布的 Release。

</details>

每个已签名平台都带一步显式校验（`codesign --verify` + `stapler validate`；`Get-AuthenticodeSignature`），未签名的包不会被静默发出去。`*.p12 / *.pfx / *.csr / *.cer / *.key / *.b64` 已加进 `.gitignore`：私钥只存在于 secrets。

**没有证书时，macOS 包也不会"裸奔"**：会自动注入 `signingIdentity: "-"` 做 ad-hoc 签名（只有确实没有真身份时才注入，不会顶掉真证书）。原因是 Apple 芯片上所有可执行代码都必须至少带 ad-hoc 签名，否则应用根本起不来 —— 有了它，用户看到的是「右键 → 打开一次」，而不是「已损坏，无法打开」。Windows 侧则是未签名安装包，SmartScreen 会问一次。给用户看的文案写在发版说明的「首次安装与打开」一节。

## 使用

四个端是**同一个内核的四个入口**：读写同一份本地数据（默认 `~/.dbmind`），连接与设置改一处四端都看得见，可以同时开着跑。Windows 安装版（msi / nsis）与免安装 zip 都把四个壳放在同一目录，共用一份 `jre/` 与 `agents/`：

```
dbmind-desktop.exe   桌面版
dbmind-web.exe       浏览器版
dbmind.exe           命令行（CLI）
dbmind-mcp.exe       给 AI 客户端用的 MCP 服务器
jre/  agents/  web/  四端共用
```

### 桌面版

双击 `dbmind-desktop.exe`（开始菜单也有入口）。内置内核，端口自动挑 20361 附近，不需要参数。

### 浏览器版（Web）

```powershell
cd D:\app\DBmind                      # 先进入安装目录，--dist 是相对路径
.\dbmind-web.exe --port 8787 --dist web
```

然后浏览器打开 `http://127.0.0.1:8787`。

- `--port` 默认 20361 —— **别用它**，那是桌面版的地盘，两个抢同一端口只会互相添乱
- `--dist web` 必须带：默认值是 `./frontend/dist`（开发时的仓库路径），安装目录里没有这个目录，不带会整站 404
- 要让局域网访问，加 `--host 0.0.0.0`（默认只监听 `127.0.0.1`）

### CLI

把安装目录加进 PATH 后，在任意位置可用：

```powershell
setx PATH "$env:PATH;D:\app\DBmind"
```

```powershell
dbmind status                                    # 存储位置、连接数、驱动进度、安全开关
dbmind types                                     # 支持哪些类型（--kind 的取值）
dbmind conn list                                 # 连接清单
dbmind conn add --name 测试 --kind MYSQL --host 127.0.0.1 --port 3306 --database demo --user root --password **
dbmind test --conn 测试                           # 自检，返回耗时与版本
dbmind tables --conn 测试                         # 表 / 视图（加 --refresh 绕过结构缓存）
dbmind columns --conn 测试 --table orders         # 表结构
dbmind query --conn 测试 --sql "select 1"         # 执行 SQL（--max-rows 2000、--timeout-ms 30000）
dbmind query --conn 测试 --file q.sql --json      # 从文件读 SQL，JSON 输出给脚本
dbmind history --limit 20                         # 查询历史
dbmind setting list                               # 读写设置（安全开关在这里）
```

退出码是约定好的：`0` 成功、`2` 连接不存在、`3` 参数或 SQL 非法、`4` 被安全策略拦下、`5` 超时、`130` 取消。`--password` 会留在命令历史里，日常查询用已保存的连接名即可。

### MCP

`dbmind-mcp.exe` 走 stdio，本身不用手动启动 —— 配给支持 MCP 的 AI 客户端即可。Codex（`~/.codex/config.toml`）：

```toml
[mcp_servers.dbmind]
command = 'D:\app\DBmind\dbmind-mcp.exe'
```

JSON 形态（Claude Desktop / Cursor 等）：

```json
{ "mcpServers": { "dbmind": { "command": "D:\\app\\DBmind\\dbmind-mcp.exe" } } }
```

默认**只读**；要放开写入，给命令加 `--allow-write`（仍受只读连接、生产保护与危险语句拦截约束）。工具清单：列连接、列类型、列表 / 视图、搜表、看表结构、执行查询、取消查询、查询历史、服务器信息。

- **数据目录**：默认 `~/.dbmind`（元数据库、驱动缓存、日志、任务快照、导出临时文件）。可用环境变量 `DBMIND_HOME` 整体重定向，或在设置页迁移 —— 搬这一个目录就是完整迁移。
- **数据目录自愈**：开机后台回收元数据库空洞（空闲页 ≥8 MB 且占比 >20% 才动手）、清理 24 小时前的装载残留、任务快照只留最新 500 份 —— 实测元数据库 805 MB → 54 MB，整个目录 1.5 GB → 223 MB。后台清扫**不删**查询历史与审计：删不删只由「清空日志」按钮和保留策略（`history.maxEntries` / `history.retentionDays`，默认 `0` = 不删）决定。
- **常用环境变量**：`DBMIND_HOME`（数据目录）、`DBMIND_AGENT_JAR`（覆盖宿主 jar 路径）、`DBMIND_JAVA`（指定 Java 运行时）。**安装包 / 便携包自带的 `jre/` 会被自动采用**（桌面、Web、CLI、MCP 四个端一致），用户不必另装 JDK；只有把 exe 挪出安装目录、或想指定别的运行时，才需要设 `DBMIND_JAVA`。

## 测试

```powershell
cargo test --workspace                      # 单元测试（不依赖任何外部环境）
powershell -File scripts\test\smoke-api.ps1   # 端到端冒烟（68 项，SQLite 真库，无需外部服务）
powershell -File scripts\test\host-tests.ps1     # Java 宿主真机回归（起真实 JVM）
```

## 文档

| 文档 | 内容 |
|---|---|
| [docs/architecture.md](docs/architecture.md) | 架构详解：泳道会话、事务、agent 协议、执行链、安全策略 |
| [docs/storage.md](docs/storage.md) | 元数据库：存储设计、WAL、数据目录迁移 |
| [docs/build.md](docs/build.md) | 构建与发布：各端构建、打包参数、环境变量、发行版上传 |
| [docs/troubleshooting.md](docs/troubleshooting.md) | 常见问题与踩坑记录（构建 / 运行 / 打包 / 数据） |
| [docs/db-lab.md](docs/db-lab.md) | 测试实验室：Docker 里拉起全部数据源的连接信息与演示数据 |

## 技术栈

Rust（axum / rusqlite / serde / tracing） · Vue 3 + Monaco + Element Plus（rolldown-vite） · Java 17（纯 stdin/stdout JSON 协议，无重框架） · Tauri 2

## 赞助

如果 DBmind 帮你省了时间，欢迎请作者喝杯咖啡 ☕（扫码时请备注称呼，方便致谢）

| 微信支付 | 支付宝 |
|:---:|:---:|
| <img src="docs/assets/sponsor-wechat.jpg" width="240" /> | <img src="docs/assets/sponsor-alipay.jpg" width="240" /> |

> 赞助纯属自愿，不影响任何功能的使用 —— 软件本身永久免费。

## License

本项目基于 **Apache License 2.0** 开源（[LICENSE](LICENSE)）：可以自由使用、修改与再分发，商业使用、闭源集成均无需额外授权，只需保留版权与许可证声明（见 [NOTICE](NOTICE)）；许可证同时包含明示的专利授权。

**名称与商标**：「DBMind」名称与图标归作者所有——欢迎 fork 与二次开发，但发布时不得使用 DBMind 名称或图标冒名。软件内「设置 → 关于」亦有展示。
