# scripts

根目录只留日常入口，其余按用途分四类。**新脚本请归进对应子目录**，别再往根上平铺。

完整构建 / 发版流程见 [docs/build.md](../docs/build.md)。

## 日常（根目录）

| 脚本 | 用途 | 常用命令 |
|---|---|---|
| `start.ps1` | 起本地后端（可选同时开浏览器），开发时最常用 | `powershell -File scripts\start.ps1 -Port 20362 -NoBrowser` |
| `start.bat` | 便携包里的启动器，由 `build/package.ps1` 复制进发行目录 | 双击 |
| `stop.ps1` | 停掉本地后端 | `powershell -File scripts\stop.ps1` |

`start.ps1` 的参数：`-Port` 指定端口、`-NoBrowser` 不开浏览器、`-Log` 保留日志、`-Release` 用 release 产物、`-Java` 指定 JRE 路径。

## build/ 组装产物

| 脚本 | 用途 |
|---|---|
| `package.ps1` | 组装发行目录（exe ×4 + `agents/` + `web/` + `start.bat` + README），CI 也用它；可 `-SkipCargo/-SkipAgents/-SkipFrontend/-Out` |
| `dmg-settings.py` | macOS DMG 外观配置，CI 打包时作为 `dmgbuild -s` 的配置传入 |

> 构建本身由 CI 负责（`.github/workflows/release.yml`）：`cargo build --release` + 四个 `mvn ... package`。本地不再有 `build-agents.ps1` / `release.ps1` / `release.bat`。

## release/ 发版

| 脚本 | 用途 |
|---|---|
| `release.mjs` | 改版本号（Cargo.toml / package.json / tauri.conf.json 等 6 处）并追加版本说明骨架：`node scripts/release/release.mjs 1.2.36 patch 2026-10-20`。**不幂等，只能跑一次** |
| `sync-release-notes.mjs` | 把发布说明同步到 GitHub Release、官网与词典：`node scripts/release/sync-release-notes.mjs v1.2.36 --no-push` |
| `publish-site.ps1` | 把 `website/` 内容推进 `gh-pages` 并触发 Pages 重建（依赖 `tools/git-proxy.ps1`） |
| `update-homebrew-cask.ps1` | 同步另一个仓库的 Homebrew cask：`-Version 1.2.36`，默认取最新 Release |

## test/ 测试

| 脚本 | 用途 | 前提 |
|---|---|---|
| `smoke-cli.ps1` | CLI 全链路端到端：连接增删查、建表写入、安全闸门、历史记录、退出码；全程临时 `DBMIND_HOME` | 已构建后端 |
| `smoke-api.ps1` | 上游兼容层 HTTP 端到端（68 项，SQLite 真库）；自己起服务、跑完清理，不碰 `~/.dbmind` | 已构建后端 |
| `host-tests.ps1` | 内核 ↔ Java 宿主真机回归（起真 JVM，缺环境直接失败） | 宿主 jar 已构建 + 可用 Java |
| `real-redis.ps1` | 跑被 `#[ignore]` 的真 Redis 测试（`-RedisHost` / `-Port`）；会 `DEBUG SLEEP 2`，勿指向共享实例 | 可连的 Redis |
| `real-ssh.ps1` | 跑真 SSH 跳板测试（`-SshHost` / `-SshPort` / `-SshUser` / `-Password` / `-KeyPath`） | 可连的 SSH |

## tools/ 小工具

| 脚本 | 用途 |
|---|---|
| `git-proxy.ps1` | 读系统代理给 git 用：`. .\scripts\tools\git-proxy.ps1` 后调 `Get-GitProxyArgs`，无代理时返回空数组（即直连） |
