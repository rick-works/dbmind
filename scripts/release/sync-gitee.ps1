# ===== 一键同步 Gitee（本地发，不走 CI）=====
# Gitee 侧三条内容全部由本脚本负责；CI 里不再跑任何 Gitee 同步。
#
# 用法（在仓库根目录）：
#   powershell -File scripts\release\sync-gitee.ps1            # ① 代码+tag  ② 官网
#   powershell -File scripts\release\sync-gitee.ps1 v2.0.6     # 再加 ③ 该版本的 Release 附件
#
# 什么时候跑：
#   - 每次 push 代码后        → 不带参数（代码+tag+官网，约几秒）
#   - 发新版本、改官网后      → 同上；发了 Release 再带 tag 同步附件
#
# 说明：
#   - ① ② 走本机直连 Gitee（国内，秒级）；③ 下载 GitHub 资产走系统代理，
#     上传 Gitee 强制直连（NO_PROXY），脚本内部有 100MB 预跳过 + 增量补传
#     （同名同大小的附件不重传，中断重跑只传缺的）。
#   - 官网发布：Gitee Pages 免费版没有更新 API，内容推上去后要去
#     Gitee 仓库 → 服务 → Gitee Pages 手动点一次「更新」。
#   - Gitee Pages 部署分支是 gitee-pages（website/ 目录的孤儿分支副本）。
param([string]$Tag = '')

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root

$giteeTok  = (Get-Content "$env:USERPROFILE\.dbmind\gitee-token"  -Raw -Encoding UTF8).Trim()
$githubTok = (Get-Content "$env:USERPROFILE\.dbmind\github-token" -Raw -Encoding UTF8).Trim()
$giteeUrl  = "https://oauth2:$giteeTok@gitee.com/rick-works/dbmind.git"

# ---------- ① 代码 + 全部 tag（增量，强推对齐） ----------
Write-Host '==> [1/3] 代码 + tag -> Gitee'
git push $giteeUrl "+refs/heads/master:refs/heads/master"
git push $giteeUrl "+refs/tags/*:refs/tags/*"

# ---------- ② 官网 website/ -> gitee-pages 孤儿分支 ----------
Write-Host '==> [2/3] 官网 website/ -> gitee-pages 分支'
$tmp = Join-Path $env:TEMP 'gitee-pages-site'
if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
New-Item -ItemType Directory $tmp | Out-Null
Copy-Item (Join-Path $root 'website\*') $tmp -Recurse -Force
Push-Location $tmp
try {
  git init -q
  git symbolic-ref HEAD refs/heads/gitee-pages   # 本机 git 老不支持 init -b
  git add -A
  git commit -qm ("site: sync from master " + (git -C $root rev-parse --short HEAD))
  git push -qf $giteeUrl gitee-pages
} finally { Pop-Location }
Remove-Item -Recurse -Force $tmp
Write-Host '    完成（Gitee Pages 更新需到 仓库→服务→Gitee Pages 手动点「更新」）'

# ---------- ③ Release 附件（可选，带 tag 参数才跑） ----------
if ($Tag) {
  Write-Host "==> [3/3] Release $Tag 附件 -> Gitee（增量 + 3 路并发 + >100MB 预跳过）"
  $env:GITEE_TOKEN = $giteeTok
  $env:GH_TOKEN    = $githubTok
  # gh 下载 GitHub 资产要走系统代理；NO_PROXY 让 gitee.com 的 curl/git 保持直连
  $set = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
  if ($set.ProxyEnable -eq 1 -and $set.ProxyServer) {
    $env:HTTPS_PROXY = "http://$($set.ProxyServer)"
    $env:NO_PROXY    = 'gitee.com'
  }
  $bash = 'D:\app\git\bin\bash.exe'
  if (-not (Test-Path $bash)) { $bash = 'bash' }
  & $bash "$root\scripts\release\gitee-release.sh" $Tag
  if ($LASTEXITCODE -eq 2) { Write-Warning '附件有部分失败/跳过（多为超 100MB），详见上面日志'; exit 0 }
  elseif ($LASTEXITCODE -ne 0) { throw "附件同步失败（exit=$LASTEXITCODE）" }
}
Write-Host '==> 全部完成'
