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
| Java 宿主 | `mvn -B -q -DskipTests package -f agents\dbmind-agent-<m>\pom.xml`（4 个模块各跑一次） | `agents\*\target\*.jar` |

## 打包发布

**发行包统一由 CI 产出**：打标签触发 `.github/workflows/release.yml`，一次跑出 Windows portable zip / MSI / NSIS、macOS dmg、Linux deb / rpm / AppImage。

本地不再保留一键打包脚本（原 `scripts\release\release.ps1` 与 `scripts\release.bat` 已移除）。只在本机拼一个发行目录时，按 CI 的顺序走：

```powershell
cargo build --release
cd frontend; npm install; npm run build
# 四个 Java 宿主各跑一次
mvn -B -q -DskipTests package -f agents/dbmind-agent-jdbc/pom.xml
powershell -File scripts\build\package.ps1 -SkipCargo -SkipAgents -SkipFrontend
```

| 文件 | 作用 |
|---|---|
| `scripts\build\package.ps1` | 组装发行目录（exe ×4 + `agents/` + `web/` + `start.bat` + README），CI 也用它 |
| `scripts\build\dmg-settings.py` | macOS DMG 外观配置，CI 里作为 `dmgbuild -s` 的配置文件 |

CI 里的精简 JRE 用 jlink 生成，模块集必须含 `jdk.charsets`（少了 Doris 连不上，见 troubleshooting）。


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

## 本机出包（CI 不可用时手动来一遍）

本地一键脚本已移除，只能按 CI 的顺序手动执行（版本号与要点仍要先提交）：

```powershell
# 1) 版本号 + 词典（与 CI 同一脚本；不幂等，只跑一次）
node scripts\release\release.mjs 1.2.31 patch 2026-10-16

# 2) 三部分分别构建
cargo build --release
cd frontend; npm install; npm run build; cd ..
foreach ($m in 'jdbc','mongodb','redis','elasticsearch') {
    mvn -B -q -DskipTests package -f "agents\dbmind-agent-$m\pom.xml"
}

# 3) 组装发行目录（CI 用的同一份脚本）
powershell -File scripts\build\package.ps1 -SkipCargo -SkipAgents -SkipFrontend

# 4) 桌面安装包（本地不需要 CI 的签名覆盖，直接用 tauri.conf.json）
cd crates\dbmind-desktop; npx --yes '@tauri-apps/cli@^2' build --bundles msi,nsis
```

产物：`dist\dbmind-<版本>\` 与 `crates\dbmind-desktop\target\release\bundle\`。
精简 JRE 自己 jlink：`--add-modules java.se,jdk.unsupported,jdk.crypto.ec,jdk.zipfs,jdk.charsets`（少 `jdk.charsets` 会让 Doris 连不上），或直接取 Release 里的现成包。
挂 Release、回填正文与官网、同步 cask 照上面第 4、5 步手工做（CI 那条路是自动的）。