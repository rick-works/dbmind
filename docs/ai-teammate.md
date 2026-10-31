# AI 同事落地计划（P0-1 ~ P1-1）

> 这份文档是**施工单 + 验收单**：每一步都写清「改哪、怎么验、什么算过」。
> 完成一步就把该步的 `[ ]` 改成 `[x]`，并把实测输出粘在该步的「实测」下面。

## 0. 目标与总验收

让 AI 从「你问它答」变成「有记忆、能办事、会值班、留痕迹」的同事：

| 编号 | 能力 | 一句话验收 |
|---|---|---|
| P0-1 | 长期记忆 | 第二次问同一个库，模型不用重复交代就用对口径，且能指出依据 |
| P0-2 | 智能体办事 | 一句「这张表越来越慢，帮我看看并处理」→ 执行计划 + 索引建议 + 一张可点确认的变更单，确认后能撤销 |
| P0-3 | 主动值班 | 到点自己巡检，发现问题变成「待办卡片」（结论 + 证据 + 动作），没问题不打扰 |
| P0-4 | 任务化 | 多步执行有状态（running / waiting / failed / done）、中断后能接着跑，且续跑不重复执行已完成的查询 |
| P1-1 | 审批与审计 | 谁批准、执行了什么、撤没撤，能查能导出 |

## 1. 现状（2026-10-31 审计）

后端表与 handler 大多已存在，但**关键的三处调用没接**，所以整条链路不会发生：

| 能力 | 已有 | 缺 |
|---|---|---|
| 记忆注入 | `ai_memories` 表、`memory_context()`（`ai/teammate.rs:219`）、`mod.rs:320` 调进 system、`source=user/ai` 区分渲染 | 自动蒸馏没被调用（`memory_extract` 无调用方） |
| 会话 | `ai_threads`/`ai_messages` 表 + `thread_*` 端点 | `ai_message_add` **无调用方**，`chat`/`chat_stream` 不落库 |
| 智能体 | `agent.rs` 3 个只读工具 | 业务工具、`propose_change`、前端入口（`/api/ai/agent` 前端 0 处调用） |
| 值班 | `ai_tasks` + `spawn_scheduler` | `spawn_scheduler` **无调用方**；巡检只查「无主键」；job 恒 `done` |
| 变更单 | `ai_changes` 五态 + 批准/执行/撤销 | 高危无二次确认；`approved_by` 硬编码 `"user"` |
| 审计 | `ai_events` 写入点齐全 | 无导出 |

## 2. 环境与验证方式

```powershell
# 构建（改了 Rust 必须先停掉 dev 后端，否则 exe 被占）
cd d:/develop/workspace/product/DBMind
cargo build -p dbmind-web

# 启动 dev 后端（20361 通常被桌面端占，用 20362）
powershell -ExecutionPolicy Bypass -File _uitest\start_dev.ps1 -Port 20362
```

- 验证脚本：`_uitest/ai-teammate-verify.py`（HTTP 层，`-Step N` 只跑某一步）
- UI 验证：`_uitest/ai-teammate-ui.cjs`（CDP 9333，需 `--disable-extensions` 的干净浏览器）
- 模型：本机已配置 DeepSeek（`~/.dbmind/dbmind.db` 的 `ai_models`），端到端能真跑
- 测试连接：优先用 MySQL/SQLite 试验库，**不要碰生产连接**

## 3. 施工步骤

### S1 接线：调度器 + 会话落库 + 事实蒸馏（P0-1 后端）

改动：

- `crates/dbmind-web/src/lib.rs`：`serve()` 里 `AppState::new(engine)` 之后调
  `crate::api::ai::teammate::spawn_scheduler(state.clone())`（30s 后首轮，60s 间隔）
- `crates/dbmind-web/src/api/ai/mod.rs`：`AiRequest` 增加 `thread_id`、`persist`
- `crates/dbmind-web/src/api/ai/teammate.rs`：新增
  - `ensure_thread(...)`：无 id 且有 `persist` ⇒ 建会话（标题取问题前 30 字），返回 id
  - `append_message(thread_id, role, content, usage)`：写 `ai_messages`，失败只告警不报错
- `crates/dbmind-web/src/api/ai/chat.rs`：
  - `chat`：落 user 消息 → 调模型 → 落 assistant 消息（带 usage）→ 响应带 `threadId`
  - `chat_stream`：同上；`threadId` 通过**响应头 `X-Dbmind-Thread`** 与首帧
    `data:{"thread":"thr_..."}` 两处给前端
  - 两者成功后在后台 `tokio::spawn(teammate::memory_extract(...))`

验证：

```powershell
python _uitest/ai-teammate-verify.py -Step 1
```

通过标准：

1. `POST /api/ai/chat` 带 `persist: true` ⇒ 返回 `threadId` 非空
2. `GET /api/ai/threads/{id}` 有 2 条消息（user + assistant），assistant 带 usage
3. 停后端重启后再 GET，消息还在（证明落库不是内存）
4. 带「记住：这个库金额单位是分」问一次 ⇒ `GET /api/ai/memory` 多出一条 `source=ai`
5. 后端日志出现调度器首轮 tick（`值班任务调度失败` 不出现即通过）

风险：

- 落库失败绝不能影响回答（吞错误 + `tracing::warn`）
- `memory_extract` 自带信号词过滤，不会每轮都调模型；仍要 spawn 后台，别阻塞响应

### S2 记忆可视化（P0-1 前端）— 已并入 S6

改动：

- `frontend/src/api/index.js`：`aiThreadList/aiThreadGet/aiThreadCreate/aiMemoryList/aiMemoryAdd/aiMemoryDelete`
- `frontend/src/modules/ai/AiPanel.vue`：
  - 首次提问带 `persist: true`，之后带 `threadId`；从响应头/首帧取回并记住
  - 打开面板时 `GET /api/ai/threads` 取最近一个会话，点「继续上次」恢复消息
  - 「记忆」抽屉：列表（user/推断分色）、新增、删除

验证：`node _uitest/ai-teammate-ui.cjs -Step 2`

通过标准：刷新页面后能恢复上次会话；手工加一条记忆后重问，回答里体现该口径。

### S3 智能体真能办事（P0-2）

改动：

- `tools.rs`：把 `patrol` / `index_advisor` 的规则部分抽成
  `patrol_scan(state, conn, db)` / `index_suggestions(state, conn, db, only)`，handler 改为薄壳
- `agent.rs` 工具 3 → 8：

  | 工具 | 作用 | 只读 |
  |---|---|---|
  | `list_tables` / `describe_table` / `run_readonly_sql` | 现有 | 是 |
  | `patrol_tables` | 规则巡检（限 40 表，0 token） | 是 |
  | `suggest_indexes` | 索引/主键建议（带 DDL 与理由） | 是 |
  | `sync_status` / `backup_status` | 最近 5 条同步/备份任务状态 | 是 |
  | `propose_change` | **只建 pending 变更单，不执行**；返回 changeId + risk + undoSql | 否（也不执行） |

- `agent.rs` 响应增加 `changes`（本轮产出的变更单）与 `jobId`（落 `ai_jobs`，kind=agent）
- `action.rs risk_of`：`update/delete` 无 `WHERE` ⇒ `high` + 独立提示；读
  `safety.maxWriteRows`，超阈值 ⇒ `high`
- `teammate.rs`：新增 `POST /api/ai/changes/{id}/confirm`（高危二次确认），
  `change_apply` 对 `risk=high` 要求状态 `confirmed`

验证：`python _uitest/ai-teammate-verify.py -Step 3`

通过标准：

1. 提问「这张表越来越慢，帮我看看并处理」⇒ `steps` 里出现 `suggest_indexes` 且出现 `propose_change`
2. `changes[0].risk`、`undoSql` 合理（有索引 DDL 的撤销语句）
3. `approve → confirm → apply → undo` 全链路 200，且 `GET /api/ai/changes?status=` 状态依次变化
4. 无 WHERE 的 `update t set a=1` ⇒ `risk=high`，未 confirm 时 apply 被拒（文案含「二次确认」）

### S4 值班与待办（P0-3）

改动：

- `teammate.rs patrol_rules`：补「无索引（除主键外没有任何索引且列名像关联键）」「全列可空」
- `run_task`：有问题 ⇒ job `status=alert`，无问题 ⇒ `done`（现在两个分支都是 `done`）
- 新增 `POST /api/ai/jobs/{id}/dismiss`（待办「忽略」）
- 新增 `GET /api/ai/events/export`（CSV，含时间/线程/变更单/actor/动作/详情）

验证：`python _uitest/ai-teammate-verify.py -Step 4`

通过标准：建一条 `runNow` 任务 ⇒ `/api/ai/jobs` 出现 `alert` 卡片且有 issue 明细；
dismiss 后状态变 `dismissed` 且不再出现在默认列表；events 导出能下载且行数与 `GET /api/ai/events` 一致。

### S5 审批与审计（P1-1）

改动：

- `change_approve` / `confirm` 接收 `approver`（默认 `user`），落 `approved_by` 与事件 `actor`
- 设置项 `ai.approval.twoPerson`（默认关）：开启后 `confirm` 的 approver 必须与 `approve` 不同
- 审计事件补 `actor`：AI 提议记 `ai`，人点批准/执行/撤销记具体 approver

验证：`python _uitest/ai-teammate-verify.py -Step 5`

通过标准：两人批准链路可走通；同人 confirm 在开关打开时被拒（说明里给出原因）。

### S6 前端交付（P0-2 / P0-3 UI）

改动：

- `AiPanel.vue`：Agent 模式开关（走 `/api/ai/agent`）、`m.steps` 轨迹（模板已在，只缺数据）、
  变更单卡片（批准 / 二次确认 / 执行 / 撤销）、待办页签（job 列表 + 查看 + 忽略）
- `api/index.js`：`aiAgentRun`、`aiChangeApprove/Confirm/Apply/Undo`、`aiJobList/aiJobDismiss`、
  `aiTaskList/aiTaskSave/aiTaskRun`、`aiEventList/aiEventExport`

验证：`node _uitest/ai-teammate-ui.cjs -Step 6`

通过标准：一句话提问后能看到工具轨迹；变更单卡片能点完 approve→apply→undo；待办能忽略。

### S7 回归

- `cargo test -p dbmind-web -p dbmind-core`（含新增单测：risk 规则、巡检规则、状态机）
- `python _uitest/ai-teammate-verify.py`（全步骤）
- `cd frontend; npm run build; npm run guard`
- 手工回归：SQL 查询 AI 弹窗（解释/优化/修复）、知识库问答、设置页「测试连接」的 `chat` 调用
  **都不应被落库逻辑干扰**（它们不带 `persist`，不产生会话）

## 4. 回滚

- 后端：`git revert` 对应提交即可；新增表用 `CREATE TABLE IF NOT EXISTS`，不删库
- 前端：`npm run build` 回滚产物
- 行为开关：`safety.aiWriteEnabled` 关掉 ⇒ 变更单只能提不能执行；
  `ai.approval.twoPerson` 关掉 ⇒ 退回单人批准；删掉 `ai_tasks` 行 ⇒ 值班不跑

## 5. 进度

- [x] S1 接线（调度器 + 会话落库 + 事实蒸馏）
- [x] S2 记忆可视化（并入 S6）
- [x] S3 智能体工具（3 → 8 个工具 + 变更单 + 二次确认）
- [x] S4 值班与待办（巡检规则 + alert + 忽略 + CSV 导出）
- [x] S5 审批与审计（批准人链 + 双人复核）
- [x] S6 前端交付（深挖 / 同事抽屉 / 变更单卡片）
- [x] S7 回归
- [x] S8 变更单归属 job（第二轮）
- [x] S9 任务化：checkpoint + 续跑（第二轮）
- [x] S10 预算闸门：降级 / 拒绝（第二轮）

## 6. 实测记录（2026-10-31，dev 后端 20362，真实 DeepSeek）

| 步骤 | 命令 | 结果 |
|---|---|---|
| S1 | `python _uitest/ai-teammate-verify.py -Step 1` | 12/12 通过。含「记忆注入真的见效」：问「123456 按我们库口径是多少」→ 回答「金额单位为分，123456 代表 1234.56 元」 |
| S3 | `... -Step 3` | 15/15。工具轨迹 `describe_table → suggest_indexes → run_readonly_sql ×2 → propose_change`；变更单 `create index idx_orders_mallid on orders (mall_id);` + 撤销 `drop index …`；批准→执行→撤销全绿；无 WHERE 的 UPDATE 判 `high` 且未二次确认被拒 |
| S4 | `... -Step 4` | 9/9。巡检发现「无主键 / 除主键外全列可空」，job 状态 `alert`，忽略后 `dismissed`，审计 CSV 可下载（含 BOM，Excel 直开） |
| S5 | `... -Step 5` | 5/5。双人复核开启后同人确认被拒，换人成功，批准链落库为 `alice → bob`，审计 actor 含两人 |
| 全量 | `python _uitest/ai-teammate-verify.py` | 41/41 通过 |
| UI 1 | `node _uitest/ai-teammate-ui.cjs`（CDP 9334，`--disable-extensions`） | 10/10：两个入口、会话自动恢复、深挖开关、待办 10 张、记忆 12 条、手工加记忆回显「你说过」、新对话清空 |
| UI 2 | `node _uitest/ai-teammate-agent-ui.cjs` | 10/10：选上下文 → 一句话 → 6 步工具轨迹 → 变更单卡片 → 批准 → 执行（applied）→ 撤销（undone） |
| 单测 | `cargo test -p dbmind-web -p dbmind-core` | 221 + 178 全绿（含新增 3 条风险/WHERE 规则单测） |
| 前端 | `npm run build` + `npm run guard` | 通过（zh/en 各 4512 键，断链 0） |

### 顺手修掉的既有缺陷（都在这条链路上）

1. `crates/dbmind-web/src/api/ai/mod.rs`：`delete` 路由没导入、`teammate.rs` 里
   `let Ok((raw, _usage))` 与 `let Some((start, end))` 两处解构错误 ——
   整个 crate 之前编译不过，等于这套「AI 同事」从未跑起来过。
2. `spawn_scheduler` / `memory_extract` / `ai_message_add` 三处定义了但**无人调用**（本次接通）。
3. 值班巡检曾另写一份「只查主键」的规则，与界面巡检结论矛盾 —— 现已统一为 `tools::patrol_scan`。
4. 巡检的「全列可空」会把 `integer primary key` 也算进去（SQLite 把主键列报成可空），
   已改为只看非主键列。
5. `change_apply` 的 `approved_by` 原本硬编码 `"user"`，现在记录真实批准人链。

## 7. 第二轮：任务化 / 预算 / 归属（2026-10-31 同日完成）

### S8 变更单归属 job（P0-4 的最后一公里）

问题：智能体的顺序是「先出单（propose_change）→ 最后才建 job」，`ai_changes.job_id` 一直是空，
待办卡片点进去看不到它产出的变更单，审计也串不起来。

改动：`storage.rs` 新增 `ai_change_attach_job(ids, job_id)`（事务批量回填）；
`agent.rs` 收尾时把本次 `changes[].id` 挂到 `job_id` 上；续跑时同样回填。

### S9 任务化：checkpoint + 续跑（P0-4 主体）

改动：

- `agent.rs` 把「多轮循环」抽成 `run_loop(...)`，首次运行与续跑**共用一份**（不复制两套循环）
- 待办**开跑前**就建（`status=running`），每轮结束 `checkpoint_job` 落一次（状态 + 步骤 + 当前答案）
- 模型调用失败 → `status=failed`（已走到的步骤全在，界面显示「失败可重试」）
- 新增 `POST /api/ai/jobs/{id}/resume`：把落库步骤**重放**成
  `assistant(tool_calls) + tool(result)` 消息对，再交给同一个 `run_loop` 继续 ——
  不重复执行已完成的查询（那些可能很贵，结论已经在线了），续跑只花一轮模型调用
- 收尾状态按「该 job 下还有没有 pending 变更单」判定（不能只看本次新增，
  否则续跑会把 waiting 误写回 done，待办里那张单子就「变成已完成」了）
- 前端：待办卡片对 `kind=agent` 且 `failed/running` 的项显示「继续跑」

说明：`ai_jobs` 没有 `thread_id` 列（表是早先定的），续跑出的变更单只挂 job、不挂会话；
为一个归属字段给表加列+迁移不划算，已在代码里注明。

### S10 预算闸门（P2）

改动：

- 设置键：`ai.budget.monthlyTokens`（0=不限）、`ai.budget.downgrade`（true=降级）、
  `ai.budget.cheapModel`（空=只压上下文）
- `storage.rs` 新增 `ai_usage_month_tokens()`：`SUM(total_tokens) WHERE day >= date('now','localtime','start of month')`
  —— 与账本 `day` 的本地日期口径完全一致，不在 Rust 里自己拼月初（那样月初/月末会差一天）
- `config.rs` 新增 `BudgetGate{Ok,Downgrade,Blocked}` / `budget_gate()` / `resolve_with_budget()`：
  降级时先试 cheapModel，配错（模型被删）自动退回 auto —— 不让一个配置残渣把 AI 整体打死
- `chat` / `chat_stream` / `agent` 三处入口接闸门：
  - 拒绝模式：400 + 「本月 AI 用量已达上限（used/limit）…去设置 → 安全与会话改」
  - 降级模式：不注入表结构/记忆/知识库、跳过表清单预探，回答照常；响应带 `budget:"downgraded"`，
    流式额外发一帧 `data:{"notice":…}`（前端挂成消息下方提示条，不混进正文）
- 设置页：安全与会话里加「AI 月度 token 上限 / 超限后怎么处理 / 降级用模型」三项，随保存落库

验证：`python _uitest/ai-teammate-verify.py -Step 6`（S8+S9）、`-Step 7`（S10）、
`node _uitest/ai-budget-settings-ui.cjs`（设置页可改可存）

通过标准：变更单 `jobId` 回填；job `waiting`+步骤落库；续跑返回新结论且步骤不减、状态回写；
预算「拒绝/降级/归零」三态各按预期；设置页三字段可见且保存后后端读到新值。

### 第二轮实测

| 步骤 | 命令 | 结果 |
|---|---|---|
| S6 | `... -Step 6` | 8/8：`jobId` 回填成功；waiting + 6 步落库；续跑成功、6→6 步、状态仍 waiting、结论已更新 |
| S7 | `... -Step 7` | 5/5：拒绝模式 400（「本月 AI 用量已达上限（1006027 / 1 tokens）」）；降级模式仍回答且标 `downgraded`；流式发 notice；归零后恢复 |
| 全量 | `python _uitest/ai-teammate-verify.py` | **54/54 通过**（S1/S3/S4/S5/S6/S7） |
| UI 3 | `node _uitest/ai-budget-settings-ui.cjs` | 5/5：三个字段在**可见面板**里（页签切换后断言）、改值保存后后端读到 123456、测完还原 0 |
| 单测 | `cargo test -p dbmind-core` | 223 全绿（新增「本月用量只统计自然月内的账」「变更单可以回填所属任务」） |
| 前端 | `npm run build` + `npm run guard` | 通过（zh/en 各 4524 键，断链 0） |

## 8. 仍未做（下一轮）

- 预算粒度：目前是**全局月度**；「按连接 / 按用户」需要先有用户概念（桌面端无登录，
  只有 `approver` 这个手填名字），属于 P1-1 角色模型的延伸。
- 续跑的 UI 验证：`继续跑` 按钮只做了「存在 + 文案」，没有个 UI 脚本点它
  （造一个 failed 的 job 需要直接改库；API 路径已由 S6.4~S6.7 覆盖）。
- 巡检「无索引」规则只在行数 ≥ 1 万时提示，且依赖方言能一次列出全库索引。
- 预算超额目前不降级「历史消息裁剪」，长对话在降级模式下仍会带 8 条历史。
