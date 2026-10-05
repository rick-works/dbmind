# 架构详解

四个端（桌面 / Web / CLI / MCP）共用同一个 Rust 内核，行为完全一致。本文讲内核的关键机制。

## 1. 总体分层

```
端（Tauri / axum / clap / stdio）
    ↓ 同一套内核 API
dbmind-core
    ├── 连接管理（Store 元数据库 + 连接池 + 泳道会话）
    ├── 执行引擎（SQL 切句 / 分页 / 计数 / 超时与行数预算）
    ├── 安全策略（只读连接 / PROD 写确认 / 审计）
    ├── 驱动层（原生 SQLite + agent 宿主桥）
    └── 元数据（表树 / 列 / DDL / 注释，带 15 分钟缓存）
```

## 2. 泳道会话（编辑器事务的基石）

- 泳道键 = `连接 | 只读性 | 亲和键`。编辑器执行与事务控制都用亲和键 `ui:上游`，
  因此**编辑器的一系列语句精确落在同一条物理连接上** —— 这是会话级事务成立的前提。
- 泳道默认并发 1；结构浏览（对象树）不带亲和，走共享泳道，避免"某个页签的私有视图"。
- 单连接类型（SQLite 等嵌入式）忽略亲和：只有一条物理连接可给。
- 事务：`tx_control` 走与查询**相同的泳道**执行 `setAutoCommit(false)`，内核维护
  `tx_lanes`；连接断开重建后，下一次用该泳道时自动重新对齐 autocommit。
- ⚠️ 事务端点必须与查询走**同一个 scope 解析**（影子连接等）—— 目标不同 = 泳道不同 =
  事务落在另一条物理连接上（实测踩过：begin 在连接 A，查询在连接 B）。

## 3. Agent 宿主（Java 协议桥）

- SQLite 之外的数据源都走 Java 宿主：JDBC 桥（MySQL/PG/Oracle/MSSQL/CH/Doris/达梦/金仓/H2/Derby/DuckDB…）、
  MongoDB、Redis、Elasticsearch 各一个宿主进程。
- 内核按需拉起宿主（stdin/stdout JSON 协议），每条泳道一个会话（`lane#index`）。
- 驱动 jar 按 `plugins/connection-types/*.yaml` 的清单**从 Maven 自动下载**到 `~/.dbmind/drivers/`；
  SQL Server 集成认证 dll 从 GitHub Release 拉取（离线机器可预放进安装包）。
- 宿主方法：`query / cancel / setautocommit / commit / rollback / close` 等。
  SQLite 走内核原生驱动（rusqlite），没有宿主 —— 因此也没有会话级事务。

## 4. 查询执行链

1. 切句：按方言协议切分多语句（引号/注释内的分号不切；Redis 一行一条；ES 一请求一条）。
2. 执行：泳道会话 + 超时预算 + 行数上限（防误点 `SELECT *` 千万行表拖死客户端）。
3. 分页：后端 LIMIT 分页；**总数统计已移出主链路**（大 JOIN 的 COUNT 曾把回显卡到百秒级），
   前端拿到数据后异步调 `/count`（10s 预算，四层兜底：派生表 → 裸 FROM 直数 → CTE 改名 → 二分探测）。
4. 回显：结果 JSON 经压缩层（gzip/br）传输，实测 4.2MB → 233KB；SSE 不压缩。

## 5. 安全策略

- 连接可标记 **PROD**：页签/下拉/树全程外显；写操作（INSERT/UPDATE/DDL…）执行前二次确认。
- 连接可标记**只读**：写语句在策略层直接拒绝（不发给数据库）。
- 所有执行进**审计历史**（含事务 begin/commit/rollback 动作）。
- 元数据库：单文件 SQLite（默认 `~/.dbmind/dbmind.db`），WAL 模式；数据目录可用
  `DBMIND_HOME` 或设置页迁移整体重定向（详见 [storage.md](storage.md)）。
