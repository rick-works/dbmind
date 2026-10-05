# 构建与发布

## 环境要求

| 工具 | 版本 | 用途 |
|---|---|---|
| Rust (rustup) | stable | 内核与四个端 |
| Node.js + npm | 18+ | 前端构建 |
| JDK + Maven | 17+ / 3.6.3+ | 构建 Java 宿主（SQLite 之外的数据源需要） |
| WebView2 Runtime | 最新 | 桌面版运行（Win11 自带） |

## 开发模式

```powershell
# 一键：缺产物自动构建 → 起服务 → 开浏览器（默认端口 20361）
powershell -File scripts\start.ps1

# 手动分步
cargo build -p dbmind-web
cd frontend; npm install; npm run build
cargo run -p dbmind-web -- --port 20361 --dist frontend\dist

# 前端热更新（vite 代理到 20361）
cd frontend; npm run dev
```

## 各端构建

| 端 | 命令 | 产物 |
|---|---|---|
| Web 壳 | `cargo build -p dbmind-web` | `target\debug\dbmind-web.exe` |
| 桌面壳 | `cargo build -p dbmind-desktop` | `target\debug\dbmind-desktop.exe` |
| CLI | `cargo build -p dbmind-cli` | `target\debug\dbmind.exe` |
| MCP | `cargo build -p dbmind-mcp` | `target\debug\dbmind-mcp.exe` |
| Java 宿主 | `scripts\build-agents.ps1` | `agents\*\target\*.jar`（4 个） |

## 打包发布

双击 `scripts\release.bat`（默认 `-WithJre -Jdk <项目 JDK>`），或：

```powershell
powershell -File scripts\release.ps1 -WithJre -Jdk D:\develop\tools\jdk-25.0.4.1+1
```

流程 8 步：环境检查 → 前端 → 宿主 jar → cargo release → 绿色版组装 → zip → Tauri 安装包（MSI+NSIS）→ 汇总与 SHA256。

**提速机制**（已内置）：

- JRE 缓存：jlink 产物确定性，`dist\jre\.jlink-cache` 记录 key，同 JDK+模块集直接复用
- zip 用系统 `tar.exe` 多线程压缩（比 Compress-Archive 快 3~5 倍）
- 前端指纹：`frontend\dist\.buildhash` 记录源码指纹，未变化则复用产物

**跳过参数**：

| 参数 | 跳过 |
|---|---|
| `-SkipFrontend` | 前端构建 |
| `-SkipAgents` | 宿主 jar 检查与构建 |
| `-SkipBuild` | cargo release（复用已有产物） |
| `-SkipPortable` / `-SkipZip` | 绿色版 / zip |
| `-SkipBundle` | Tauri 安装包（不需要 WiX/网络） |
| `-Quality` | 额外跑 fmt --check 与 clippy -D warnings |

产物集中在 `dist\`：

```
dist\
├── dbmind-<版本>\            绿色版目录（exe + agents + web + jre + start.bat）
├── dbmind-<版本>-portable.zip 整包
├── installer\*.msi / *-setup.exe
└── SHA256SUMS.txt
```

## 环境变量

| 变量 | 作用 |
|---|---|
| `DBMIND_HOME` | 数据目录整体重定向（默认 `~/.dbmind`） |
| `DBMIND_AGENT_JAR` / `_MONGODB_JAR` / `_REDIS_JAR` / `_ELASTICSEARCH_JAR` | 覆盖宿主 jar 路径 |
| `DBMIND_JAVA` | 指定 Java 运行时（优先于 JAVA_HOME） |
| `DBMIND_TOOL_DIR` | 驱动/工具安装目录 |

## 发布到 GitHub Releases

1. `git push github master --tags`（origin 指向 GitHub；gitee 作为第二 remote 保留）
2. 创建发行版并上传附件（gh CLI 已授权，自动走系统代理）：

```powershell
$env:HTTPS_PROXY = 'http://127.0.0.1:17963'
gh release create v<版本> --title "DBmind v<版本>" --generate-notes `
  dist\dbmind-<版本>-portable.zip `
  dist\installer\DBmind_<版本>_x64-setup.exe `
  dist\installer\DBmind_<版本>_x64_en-US.msi
```