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
| 多数据源 | MySQL / PostgreSQL / Oracle / SQL Server / ClickHouse / Doris / MariaDB / 达梦 / 人大金仓 / SQLite / H2 / Derby / DuckDB / Redis / MongoDB / Elasticsearch |
| SQL 编辑器 | Monaco 内核：补全、格式化、多语句分块执行、逐段计时、执行历史、SQL 片段库、模板变量（`:name` 执行时填参） |
| 事务模式 | 写语句不自动提交：改完自查 → 回滚或提交；事务精确绑定编辑器会话，全 JDBC 数据源通用 |
| 结果集直接编辑 | 双击单元格改值 → 缓冲 → 批量 UPDATE（可落在事务里）；快照守卫防止翻页后误提交 |
| 大结果集 | 虚拟滚动 + 服务端分页 + 响应压缩（实测 4.2MB→233KB）；总数异步统计不阻塞回显 |
| 数据传输 | 导出 CSV / Excel / JSON / INSERT；后台任务 + 完成通知 |
| 生产库保护 | 连接标记 PROD 后：标记全程外显、写操作二次确认、执行审计留痕 |
| 数据透视 | 框选即汇总（计数 / 求和 / 均值 / 最值）、分组下钻 |

## AI，不只是聊天

大多数工具的「AI 助手」是个聊天框；DBmind 把 AI 做进了**每一步数据库操作**：

- **六大即用动作** —— 选中 SQL 一键「解释 / 优化 / 改写」；执行报错一键「AI 修错」（自动带上完整错误上下文）；排查性能问题用「诊断」；复杂任务直接对话
- **AI 的答案先验证再用** —— AI 给出的 SQL 可一键查看**执行计划**、**试跑一次**（不消耗 AI 调用），确认无误才落库， hallucination 进不了你的数据库
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
├── scripts/             启动 / 打包 / 发布 / 冒烟测试
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

# Java 宿主（4 个协议桥）
powershell -File scripts\build-agents.ps1
```

## 打包发布

```powershell
# 一键全流程（绿色版 + zip + MSI/NSIS 安装包），双击 scripts\release.bat 效果相同
powershell -File scripts\release.ps1 -WithJre -Jdk D:\develop\tools\jdk-25.0.4.1+1

# 常用组合：
release.ps1 -SkipBundle        # 只出绿色版（不需要 WiX）
release.ps1 -SkipPortable      # 只出安装包
release.ps1 -SkipBuild         # 复用已有编译产物
```

产物集中在 `dist\`：`dbmind-<版本>\`（绿色版目录）、`*-portable.zip`（绿色版整包）、`installer\*.msi / *-setup.exe`（安装包）、`SHA256SUMS.txt`。

### CI：全平台、各版本形态齐全

`.github/workflows/release.yml` 在推 `v*` 标签时触发（也可手动触发并指定 `tag`），把各平台的包挂到同一个 Release：

| Runner | 包 |
|---|---|
| `windows-latest` | `DBmind_<版本>_x64_en-US.msi`、`DBmind_<版本>_x64-setup.exe`（NSIS 安装向导）、**`dbmind-<版本>-portable.zip`**（免安装：解压即用，内含 `agents/` 与 `jre/`，目标机器不用装 Java） |
| `macos-latest` | `DBmind_<版本>_aarch64.dmg`（+ 同名 `.app`，`.app` 本身就是免安装形态：拖进「应用程序」即可） |
| `macos-15-intel` | `DBmind_<版本>_x64.dmg`（+ `.app`）—— `macos-13` 已退役，Intel 现在必须用带 `-intel` 后缀的标签 |
| `ubuntu-22.04` | `dbmind_<版本>_amd64.deb`（apt 安装）与 `*.AppImage`（**免安装**：`chmod +x` 后直接跑） |

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
powershell -File scripts\smoke-dbmind.ps1   # 端到端冒烟（69 项，需要对各数据源的连接）
powershell -File scripts\host-tests.ps1     # Java 宿主真机回归（起真实 JVM）
```

## 文档

| 文档 | 内容 |
|---|---|
| [docs/architecture.md](docs/architecture.md) | 架构详解：泳道会话、事务、agent 协议、执行链、安全策略 |
| [docs/storage.md](docs/storage.md) | 元数据库：存储设计、WAL、数据目录迁移 |
| [docs/build.md](docs/build.md) | 构建与发布：各端构建、打包参数、环境变量、发行版上传 |
| [docs/troubleshooting.md](docs/troubleshooting.md) | 常见问题与踩坑记录（构建 / 运行 / 打包 / 数据） |

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
