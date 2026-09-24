# 存储清单（持久化数据都落在哪）

> 目的：排查问题、备份迁移、清理磁盘、评估安全影响时，能一眼看清"什么数据存在哪"。
>
> 结论先给：**后端几乎所有持久化数据都集中在一个目录** —— `~/.dbmind/`。

---

## 0. 根目录怎么定的

| 项 | 规则 | 代码 |
|---|---|---|
| 数据根目录 | 环境变量 `DBMIND_HOME`；没设就用 `%USERPROFILE%\.dbmind`（Windows）/ `$HOME/.dbmind`（*nix） | `crates/dbmind-core/src/paths.rs:9-21` |
| 主库文件 | `<home>/dbmind.db` | `paths.rs:24-26` |
| 工作目录 | `<home>/work` | `paths.rs:29-31` |

对外暴露的路径（设置页「存储路径」用的是这几个）：

| 名称 | 实际位置 | 代码 |
|---|---|---|
| `dataDir` | `<store 父目录>`（即 `~/.dbmind`） | `crates/dbmind-web/src/api/sys.rs:130-147` |
| `driverDir` | `<dataDir>/drivers` | 同上 |
| `exportDir` | `<dataDir>/work` | 同上 |

**想搬家**：设 `DBMIND_HOME` 指向新目录，重启即可，所有文件跟着走。

---

## 1. 主元数据库：`~/.dbmind/dbmind.db`（SQLite）

用 `rusqlite` 打开，启用 **WAL**，因此**一定伴随两个副产物**：

```
dbmind.db         主库
dbmind.db-wal     预写日志（还没合并进主库的改动）
dbmind.db-shm     共享内存索引
```

> ⚠️ `-wal` 不是垃圾文件：它里面可能就是"最近这几分钟的改动"。备份时**要么停服后整体拷**，要么三个文件一起拷，**只拷 `dbmind.db` 会丢数据**。

### 表结构

| 表 | 存什么 | 代码 |
|---|---|---|
| `connections` | 全部连接配置：名称 / 类型 / 主机 / 端口 / 库名 / **用户名 + 密码** / 文件路径 / 颜色 / `extra`(JSON) / 只读 / 时间戳 | `storage.rs:536-551` |
| `query_history` | 查询历史：SQL / 状态 / 行数 / 耗时 / 错误码 / 时间 | `storage.rs:553-565` |
| `app_settings` | 应用设置，key-value。内置键见下 | `storage.rs:567-571` |
| `schema_cache` | 表结构缓存：`payload`(JSON) + `cached_at`，主键 `(connection_id, object_name)` | `storage.rs:573-579` |

### `app_settings` 的内置键

`storage.rs:15-22` 定义 + `seed_settings`（`storage.rs:394-408`）播种：

- `safety.protectProduction` — 生产库保护
- `safety.aiWriteEnabled` — 允许 AI 写库
- `ui.theme` — 主题
- `query.defaultPageSize` — 默认分页
- `session.maxPerHost` — 单主机最大会话
- `session.idleTimeoutSecs` — 空闲超时
- `driver.mirror` — 驱动下载镜像（`sys.rs:25-26`）
- `jdbc.allowLegacyTls` — 旧 TLS 开关（`sys.rs:219-266`）

任意键都能通过 `PUT /api/settings/{key}` 写入（`crates/dbmind-web/src/lib.rs:371-383`）。

---

## 2. AI 相关文件（均在 `~/.dbmind/` 下）

| 文件 | 内容 | 格式 | 代码 |
|---|---|---|---|
| `ai-config.json` | AI 设置：`enabled` / `privacyMode` / `auditEnabled` / `models[]`，**含 `apiKey` 明文** | JSON | `api/ai/config.rs:93-95`（路径）、`:44-90`（结构）、`:143-186`（保存） |
| `ai-usage.json` | 用量账本：`days{日期→{calls}}` + `models{模型→日期→{calls,promptTokens,completionTokens,totalTokens}}` | JSON | `config.rs:614-616`，写 `:649-696`，读 `:699-769` |
| `ai-audit.log` | 审计日志，**追加**写：时间 / 类型 / 提示词前 4000 字 | JSONL | `config.rs:772-790` |
| `ai-knowledge.json` | 团队知识：`{glossary:[],examples:[]}` | JSON | `api/ai/knowledge.rs:30-48` |
| `ai-quality-rules.json` | 数据质量规则（按 连接/库/表 键存 + savedAt） | JSON | `api/ai/quality.rs:143-163` |
| `prompts/*.md` | 提示词模板：启动时从内置释放，用户可编辑 | Markdown | `api/ai/prompts.rs:77-100` |

### 知识库目录 `~/.dbmind/ai-knowledge-bases/`

**每个文档一个文件**，不是单库单文件：

```
ai-knowledge-bases/
  index.json                       所有知识库元数据 [KbInfo]      kb.rs:220-232
  config.json                      全局默认库配置                  kb.rs:242
  <kb_id>/config.json              单库配置                        kb.rs:250-256
  <kb_id>/docs/<doc_id>.json       文档正文 + 分块                 kb.rs:258-287
  <kb_id>/vectors/<doc_id>.json    向量（embedding）               kb.rs:289-302
```

`kb_id` 只允许 `[A-Za-z0-9_-]`（`safe_id`，`kb.rs:203-214`），避免路径穿越。

---

## 3. 其它后端落盘

| 位置 | 内容 | 说明 | 代码 |
|---|---|---|---|
| `drivers/<agentKey>/*.jar` | 驱动 jar 缓存 | 二进制；搜索路径含 `DBMIND_DRIVER_DIRS` | `dbmind-core/src/agent.rs:420-541`、`:425-431` |
| `logs/agent-<id>.err.log` | Java 代理的 JVM stderr | 文本，**覆盖写**（只留最后一次） | `agent.rs:167-181` |
| `exports/` | 导出产物 | CSV / JSON / XLSX / SQL / ZIP | `api/tasks.rs:402-404`、`api/export.rs:781,894,927,1086` |
| `work/tmp/sync_<stamp>.<ext>` | 同步导入的临时文件 | **读完即删** | `export.rs:1090-1095` |
| `work/restore/<stamp>_<name>` | 上传的备份暂存 | 还原期间保留（会多次读取） | `api/backup.rs:1391-1398` |
| `backups/` | 备份产物 + 写探针 `.dbmind-write-test-*` | 探针用于验证目录可写 | `backup.rs:369-371,395,975,1187` |

### 3.1 服务自身日志（**落在仓库里**，别和上面混淆）

用脚本启动时，后端进程的 stdout / stderr 被重定向到**仓库根目录**：

```
<repo>/logs/server.out.log    服务标准输出
<repo>/logs/server.err.log    服务标准错误（Rust panic / 启动失败看这里）
```

写入位置：`scripts/start.ps1:126-130`（`Start-Process -RedirectStandardOutput/-RedirectStandardError`）。

> **两个 logs 目录的区别**：
> - `~/.dbmind/logs/agent-*.err.log` —— **Java 驱动代理**的 JVM stderr；
> - `<repo>/logs/server.*.log` —— **Rust 服务本体**的 stdout/stderr。
>
> 排查"服务起不来 / 用着用着访问不了"看**仓库里**这两个。
> 脚本注释里写了为什么**总是**重定向到文件：服务若继承调用者终端/管道的句柄，调用方一退出、管道一断，服务下次写日志就会失败并退出 —— 表现为"刚启动能访问，过一会儿就不行了"，且没有任何提示。

---

## 4. 前端浏览器存储

**没有 cookie，也没有 IndexedDB** —— 只用 `localStorage`（跨启动）与 `sessionStorage`（本标签页）。

### localStorage

| key | 内容 | 读 / 写 |
|---|---|---|
| `dbmind_theme` | 主题 `{mode}` | `utils/theme.js:5,10,18` |
| `dbmind_shortcuts` | 快捷键绑定（`'+'` 连接 token） | `utils/shortcuts.js:5,56,66` |
| `dbmind_editor` | 编辑器设置（格式化规则等） | `SettingsView.vue:1031,1052`；迁移 `utils/settings.js:81-82` |
| `dbmind_query` | 查询设置 | `SettingsView.vue:1118,1124` |
| `dbmind_notify` | 通知设置 | `SettingsView.vue:1253,1259` |
| `dbmind_folders` | **连接分组/目录名数组**（"新建分组"存的就是它） | `MainView.vue:1281,1551-1627`、`ConnectionDialog.vue:443-464`、`FolderDialogs.vue:89-132` |
| `dbmind.recent-connection-ids` | 最近使用的连接 id（最多 5） | `utils/recentConnections.js:8,14,40` |
| `dbmind-panel-width` | AI 面板宽度 | `AiPanel.vue:1162,1179` |

### sessionStorage

| key | 内容 | 位置 |
|---|---|---|
| `dbmind.session` | 会话快照（页签 / 树展开状态），刷新可恢复，新开标签即清 | `MainView.vue:1068,1106,1144` |
| `dbmind.currentConnId` | 当前连接 id | `MainView.vue` 多处（1085 / 1170 / 1671 / 1949 / 4578 / 4740） |
| `dbmind.dblist.<connId>` | 该连接的库列表缓存 `{ts,list}` | `SqlQueryView.vue:3027,3033` |

---

## 5. 明确**不落盘**的东西（别到处找）

| 项 | 状态 |
|---|---|
| 任务 / 进度（导出、备份、同步的进度与日志） | **内存态**，重启即丢（`api/tasks.rs` 的 `TaskRegistry`） |
| AI 对话历史 | **内存态**，关页签即丢 |
| 许可证 | 后端**不落盘**（`sys.rs:268-283` 恒返回完整版，`activate` 为空操作）；前端只记"弹窗已忽略"标记 |
| 用户表数据 | 在用户自己的数据库里，不在本套存储中 |
| Java 代理（`agents/`） | 自身不写任何持久化文件（只有 Maven 构建产物）；stderr 由后端重定向到 `logs/agent-*.err.log` |

---

## 6. 运维注意点

1. **明文凭据**：`dbmind.db` 的 `connections.password` 与 `ai-config.json` 的 `apiKey` **都是明文**（设计如此，未加密）。整个 `~/.dbmind` 目录不要随意外发；"导出连接"生成的包是**不含口令**的，那个可以直接分享。
2. **备份**：先停服，再整体拷贝 `~/.dbmind`；至少要连 `dbmind.db-wal` / `-shm` 一起拷。
3. **清理磁盘**：占用大头通常是 `dbmind.db`、`exports/`、`backups/`、`work/`、`drivers/`、`logs/`。`ai-audit.log` 是**只增不减**的追加日志，长期使用需要留意。
4. **迁移**：设 `DBMIND_HOME` 指向新目录即可，所有文件跟着走。
5. **排查"某个设置有没生效"**：多半在 `dbmind.db` 的 `app_settings`，或前端的 `localStorage`（前缀 `dbmind_` / `dc_` / `dbmind.`）。

---

## 7. 新增存储点时的约定（给后续改动）

- **后端**：一律落到 `paths::home_dir()` 之下，别写死绝对路径、别写到当前工作目录；拿不准就用现有文件名风格（`ai-*.json` 归 AI 域）。
- **优先 JSON 单文件**（人工可读、出问题能直接看）；数据量会随用户操作线性增长、或需要查询/分页的，才考虑进 `dbmind.db` 立表。
- **前端**：设置类一律 `localStorage` + `dbmind_` 前缀；会话级（随标签页失效）用 `sessionStorage`；**别引入 cookie**（桌面端不需要、还会带上隐私问题）。
- **新加键/文件时，顺手在这一份 `STORAGE.md` 里补一行** —— 这份文档的价值全在"全"。
