# DBmind

[English](README.md) | 简体中文

🌐 **官网**：<https://rick-works.github.io/dbmind/>

**给数据库装上「大脑」** —— DBmind（Database + Mind）是 AI 原生的多数据库管理与开发平台：
一个客户端管好你的每一个数据库，AI 帮你解释、优化、修好每一条 SQL。

本地运行、数据不出内网。Rust 内核 + Vue 3 前端 + Java 宿主（JDBC 驱动桥），提供 Web / 桌面 / CLI / MCP 四种使用形态。

## 功能一览

| 模块 | 说明 |
|---|---|
| **🤖 AI 深度集成** | 解释 / 优化 / 改写 / 修错 / 诊断 / 对话六大动作；AI 的 SQL 先看执行计划、试跑验证；MCP 服务器把整个工作台接进任意 AI 客户端（详见 [AI 能力](#ai-不只是聊天)） |
| **🧠 AI 同事** | 「自主排查」自己查库：巡检表结构、给索引建议、看同步与备份状态、只读试跑，把要改的动作做成**变更单**等你批准（高危二次确认、执行完可撤销）；长期记忆记住业务口径，对话记录可续可翻，每条回答脚注 token 消耗 |
| 多数据源 | **17 种引擎，一个客户端** —— 关系型（MySQL、MariaDB、PostgreSQL、人大金仓、Oracle、SQL Server、DB2、Derby、H2、达梦）、分析型（ClickHouse、Doris）、嵌入式 / 文件（SQLite、DuckDB）与 NoSQL（Redis、MongoDB、Elasticsearch）；JDBC 驱动按需下载 |
| SQL 编辑器 | Monaco 内核：补全、格式化、多语句分块执行、逐段计时、执行历史、SQL 片段库、模板变量（`:name` 执行时填参） |
| 事务模式 | 写语句不自动提交：改完自查 → 回滚或提交；事务精确绑定编辑器会话，全 JDBC 数据源通用 |
| 结果集直接编辑 | 双击单元格改值 → 缓冲 → 批量 UPDATE（可落在事务里）；快照守卫防止翻页后误提交 |
| 大结果集 | 虚拟滚动 + 服务端分页 + 响应压缩（实测 4.2MB→233KB）；总数异步统计不阻塞回显 |
| 表结构与对象 | 字段类型、默认值、注释、索引在表格里直接改，执行前先看到完整 `ALTER` 预览；视图、存储过程、函数、触发器、事件可查看 DDL，并转成 `ALTER` 继续编辑 |
| 用户与权限 | 各引擎的用户 / 角色管理，按作用域（全局 / 服务器 / 库 / 模式 / 表 / 例程）勾选权限；每一次勾选都预览精确的 `GRANT` / `REVOKE` 差量，提交时只发变化的部分 |
| 服务监控 | 会话、锁与库级指标，可就地终止会话；没有监控接口的引擎如实说明，不画空图 |
| ER 关系图 | 命名约定 + 真实外键推断表间关系；拖拽布局、滚轮缩放、导出 PNG / SVG |
| 数据对比与传输 | 两个连接之间比对结构与数据差异，再用向导把选中的对象传输到目标库 |
| 数据传输 | 导出 CSV / Excel / JSON / INSERT；后台任务 + 完成通知 |
| 备份与造数 | 各引擎原生 dump / restore，按模板生成测试数据 |
| 对象搜索 | 不用展开树，直接在整个连接里搜表 / 视图 / 存储过程 |
| 生产库保护 | 连接标记 PROD 后：标记全程外显、写操作二次确认、执行审计留痕 |
| 数据透视 | 框选即汇总（计数 / 求和 / 均值 / 最值）、分组下钻 |
| 驱动管理 | 从 Maven 下载 / 更新 / 修复驱动，带实时进度；内网可指向私有镜像 |
| 数据目录自愈 | 开机后台回收元数据库空洞、清理过期快照与遗留临时文件（实测 805 MB → 54 MB），清空日志 / 缓存当场回收 |

## 界面截图

每个页面都有浅色 / 深色两套主题。截图为 2880 px 宽（Retina）原片等比缩小，点击任意图片可看原尺寸。

| 界面 | 浅色 | 深色 |
|:--|:-:|:-:|
| **首页**<br>所有连接 | <img src="docs/images/home-light.png" width="330" alt="首页（浅色）"> | <img src="docs/images/home-dark.png" width="330" alt="首页（深色）"> |
| **数据查询**<br>补全与逐段计时 | <img src="docs/images/sql-editor-light.png" width="330" alt="数据查询（浅色）"> | <img src="docs/images/sql-editor-dark.png" width="330" alt="数据查询（深色）"> |
| **表结构编辑**<br>先预览 ALTER | <img src="docs/images/table-designer-light.png" width="330" alt="表结构编辑（浅色）"> | <img src="docs/images/table-designer-dark.png" width="330" alt="表结构编辑（深色）"> |
| **AI 助手**<br>计划与试跑 | <img src="docs/images/ai-assistant-light.png" width="330" alt="AI 助手（浅色）"> | <img src="docs/images/ai-assistant-dark.png" width="330" alt="AI 助手（深色）"> |
| **数据治理**<br>推断表间关系 | <img src="docs/images/governance-light.png" width="330" alt="数据治理（浅色）"> | <img src="docs/images/governance-dark.png" width="330" alt="数据治理（深色）"> |
| **数据传输**<br>导出与导入 | <img src="docs/images/data-transfer-light.png" width="330" alt="数据传输（浅色）"> | <img src="docs/images/data-transfer-dark.png" width="330" alt="数据传输（深色）"> |
| **脚本管理**<br>片段与变量 | <img src="docs/images/scripts-light.png" width="330" alt="脚本管理（浅色）"> | <img src="docs/images/scripts-dark.png" width="330" alt="脚本管理（深色）"> |
| **任务中心**<br>进度与日志 | <img src="docs/images/tasks-light.png" width="330" alt="任务中心（浅色）"> | <img src="docs/images/tasks-dark.png" width="330" alt="任务中心（深色）"> |
| **用户管理**<br>角色与权限 | <img src="docs/images/users-light.png" width="330" alt="用户管理（浅色）"> | <img src="docs/images/users-dark.png" width="330" alt="用户管理（深色）"> |
| **服务监控**<br>会话与锁 | <img src="docs/images/monitor-light.png" width="330" alt="服务监控（浅色）"> | <img src="docs/images/monitor-dark.png" width="330" alt="服务监控（深色）"> |
| **ER 关系图**<br>拖拽与导出 | <img src="docs/images/er-diagram-light.png" width="330" alt="ER 关系图（浅色）"> | <img src="docs/images/er-diagram-dark.png" width="330" alt="ER 关系图（深色）"> |
| **设置**<br>驱动与主题 | <img src="docs/images/settings-light.png" width="330" alt="设置（浅色）"> | <img src="docs/images/settings-dark.png" width="330" alt="设置（深色）"> |

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
curl -LO https://github.com/rick-works/dbmind/releases/download/v<版本>/DBmind_<版本>_aarch64.dmg
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

| 端 | 启动方式 |
|---|---|
| 桌面版 | 双击 `dbmind-desktop.exe`（内嵌内核，自动起浏览器内核窗口） |
| 浏览器版 | `dbmind-web.exe --port 8787 --dist web`，访问 `http://127.0.0.1:8787` |
| CLI | `dbmind conn list` / `dbmind driver list` / `dbmind setting get <key>` |
| MCP | `dbmind-mcp`（stdio），把命令配进支持 MCP 的 AI 客户端即可 |

- **数据目录**：默认 `~/.dbmind`（元数据库、驱动缓存、日志）。可用环境变量 `DBMIND_HOME` 整体重定向，或在设置页迁移。
- **常用环境变量**：`DBMIND_HOME`（数据目录）、`DBMIND_AGENT_JAR`（覆盖宿主 jar 路径）、`DBMIND_JAVA`（指定 Java 运行时）。

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

本软件免费提供给个人与团队使用：允许使用、复制、修改与分发（保留本声明）；**未经作者书面许可，不得单独出售或作为付费产品的主体捆绑销售**。完整条款见 [LICENSE](LICENSE)，软件内「设置 → 关于」亦有展示。
