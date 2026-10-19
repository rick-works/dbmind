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
| Java 宿主 | `scripts\build\build-agents.ps1` | `agents\*\target\*.jar`（4 个） |

## 打包发布

双击 `scripts\release.bat`（默认 `-WithJre -Jdk <项目 JDK>`），或：

```powershell
powershell -File scripts\release\release.ps1 -WithJre -Jdk D:\develop\tools\jdk-25.0.4.1+1
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

## 发版流程（一步不落）

一次发版 = **选级别 → 改版本号 → 写要点 → 打标签让 CI 出包 → 回填说明与官网 → 同步 cask → 验证**。

三步不能换顺序，原因写在每步里：

- 版号与要点都要在**打标签之前**提交（关于页要点是编进包里的）；
- 「安装包表格」只能在 **CI 出包之后**回填（体积取自 Release 资产）；
- cask 要读 Release 里的 dmg，所以也在 CI 之后。

### 前置条件

| 需要什么 | 说明 |
|---|---|
| 系统代理 | 本机代理写在注册表里，git 默认不读。用 `scripts/tools/git-proxy.ps1` 的 `Get-GitProxyArgs`（无代理返回空数组） |
| GitHub 令牌 | `~/.dbmind/github-token`（classic，repo 权限）。`git push` 用 remote 内嵌令牌；`gh` / 官网 / cask 脚本用 `$env:GH_TOKEN` |
| CI secrets | 签名证书与公证凭据（可选，未配也能发）。设置方式见 README「CI」一节 |

### 0. 选版本级别

| 级别 | 何时用 | 例 |
|---|---|---|
| `patch` | 修 bug、性能与体验优化、小功能（**1.2.x 全线都是 patch**，含带新功能的 1.2.20） | 1.2.30 → 1.2.31 |
| `minor` | 成体系的新功能 | 1.1.0 → 1.2.0 |
| `major` | 不兼容的重大变更（数据格式、配置、接口重做） | 1.x → 2.0.0 |

### 1. 改版本号（一条命令，别手改）

```powershell
node scripts/release/release.mjs 1.2.31 patch 2026-10-16
```

它一次改 6 处，手改必漏：

| 文件 | 改动 |
|---|---|
| `Cargo.toml` | `[workspace.package] version` |
| `Cargo.lock` | 各 `dbmind-*` crate 的版本（漏了很隐蔽：包名里是新版本、二进制自报旧版本） |
| `frontend/package.json` | `version` |
| `crates/dbmind-desktop/tauri.conf.json` | `version`（安装包文件名与「添加/删除程序」里的版本） |
| `resources/mac/首次打开请看这里.txt` | 示例版本号与 dmg 下载链接 |
| `frontend/src/locales/{zh-CN,en-US}.js` | 新版本插到 `settings.about.rel1`（旧版整体顺延为 rel2…）+ 折叠头的 `settings.about.releaseVersion` |

- **脚本不幂等**：重复跑会在词典里多插一版。跑错了用 `git checkout -- frontend/src/locales resources/mac` 回退重来。
- 跑完自检（版本号是否三处一致、词典键是否齐全、产物是否断链）：

```powershell
cd frontend; npm run guard
```

### 2. 写要点（必须在打标签之前）

**唯一手写**的输入只有两样：

1. `release-notes/v<版本>.md` —— 只写 `## 更新说明` 下的 `- ` 列表，2~5 条；写作口径见 `release-notes/TEMPLATE.md`（标题与「安装包」表格由脚本生成，别手写）；
2. `release-notes/en-notes.json` —— 补同名 key（如 `"v1.2.31"`）的英文要点，**条数与中文一致**（不一致脚本会点名）。

然后由脚本落进仓库里另外三处（Release 正文 / 官网源 / 关于页中英）：

```powershell
node scripts/release/sync-release-notes.mjs v1.2.31 --no-push
cd frontend; npm run guard
```

> **为什么必须在打标签之前**：应用「关于」页的要点存在 `frontend/src/locales/*.js` 里，CI 是拿标签那一刻的源码打包的 —— 标签之后再改，用户机器上装的版本「关于」页还是「（待填写：更新要点 1）」。
> 这一步此时读不到 Release 资产，会提示「保留原有表格」，正常。

### 3. 提交并触发 CI

```powershell
git add -A; git commit -m "v1.2.31 发布：版本号、发布说明与关于页要点"

. .\scripts\tools\git-proxy.ps1
$p = Get-GitProxyArgs
$tok = (Get-Content "$env:USERPROFILE\.dbmind\github-token" -Raw -Encoding UTF8).Trim()
$url = "https://rick-works:$tok@github.com/rick-works/dbmind.git"
git @p push $url master
git tag v1.2.31            # 轻量标签即可（v1.2.30 起都是轻量标签）
git @p push $url v1.2.31   # 推标签 = 触发 .github/workflows/release.yml
```

CI（`windows-latest` / `macos-latest` / `macos-15-intel` / `ubuntu-22.04`，约 17 分钟）出各平台包，
并由 `softprops/action-gh-release` **自动挂到该标签的 Release**（Release 不存在时自己创建，不需要 `gh release create`）。

- 看进度：`gh run list --repo rick-works/dbmind --limit 3`（先设 `$env:GH_TOKEN`）
- 手动补包：`gh workflow run release.yml -f tag=v1.2.31`（不改标签、不动已发布的 Release）
- 配了证书就签名并显式校验（mac `codesign --verify` + `stapler validate`，Win `Get-AuthenticodeSignature`）；没配则 mac 走 ad-hoc 签名、Windows 未签名照发

### 4. 回填说明并发布官网（CI 完成后，同一条命令去掉 `--no-push`）

```powershell
node scripts/release/sync-release-notes.mjs v1.2.31
```

一条命令做完四件事（**幂等，重复跑不会叠加**）：

| 处 | 内容 |
|---|---|
| ① Release 正文 | 用 Release 里实际的资产重排「安装包」表格（体积取自 API，不下载）+ 要点 |
| ② 官网 `website/index.html` | 版本区条目（要点 + 类型标签 + 日期），「最新」标挪到新版本 |
| ③④ 应用「关于」页 | `settings.about.relN.noteX` 中文（md）+ 英文（`en-notes.json`） |
| ⑥ 发布官网 | 自动调用 `scripts/release/publish-site.ps1`：worktree 里换掉 `gh-pages` 的 `index.html` → commit → push |

- 只处理指定版本：带 `v1.2.31`；不带参数会遍历所有版本。
- 推送 `gh-pages` 会自动触发 `pages-build-deployment`（约 35 秒生效），**不需要手动重建 Pages**。
- 想先改源不发布：加 `--no-push`。

### 5. 同步 Homebrew cask（最后一步）

```powershell
powershell -File scripts\release\update-homebrew-cask.ps1        # 默认取最新 Release；也可 -Version 1.2.31
```

cask 在**另一个仓库** `rick-works/homebrew-tap`；脚本从 GitHub API 读两个 `.dmg` 的 `sha256`
（`assets[].digest`，不用下载 100+MB），改写 tap 里的 `Casks/dbmind.rb` 并推送。
**为什么不做进 CI**：跨仓库推送需要额外的 PAT（Actions 自带的 `GITHUB_TOKEN` 只能写当前仓库），
而这件事每次发版只跑一次 —— 脚本比长期密钥划算。

### 6. 验证

```powershell
gh release view v1.2.31 --repo rick-works/dbmind --json assets --jq '.assets[].name'
```

| 检查 | 期望 |
|---|---|
| Release 资产 | Windows：`DBmind_1.2.31_x64_en-US.msi`、`DBmind_1.2.31_x64-setup.exe`、`dbmind-1.2.31-portable.zip`；macOS：`DBmind_1.2.31_aarch64.dmg`、`DBmind_1.2.31_x64.dmg`；Linux：`dbmind_1.2.31_amd64.deb`、`dbmind_1.2.31_amd64.AppImage` |
| Release 正文 | 「更新说明」是要点、「安装包」表格 7 行且体积不为 0 |
| 官网 | <https://rick-works.github.io/dbmind/> 版本页出现 v1.2.31，「最新」标只有它一个 |
| 应用内 | 设置 → 关于：当前版本 v1.2.31，要点不是占位文本 |

## 本机出包（离线/备用）

CI 不可用（断网、actions 排不上）时，`scripts\release\release.ps1` 等价于 CI 的 Windows 部分，产物进 `dist\`：

```powershell
powershell -File scripts\release\release.ps1 -WithJre -Jdk D:\develop\tools\jdk-25.0.4.1+1
```

产物：`dist\dbmind-<版本>\`（绿色版目录）、`dist\dbmind-<版本>-portable.zip`、`dist\installer\*.msi / *-setup.exe`、`dist\SHA256SUMS.txt`。
用完仍需自己做上面第 4、5 步 —— 别忘 `gh release create` 手动挂包（CI 那条路是自动的）。