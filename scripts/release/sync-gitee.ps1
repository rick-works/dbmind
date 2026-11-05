# ===== 一键同步 Gitee（本地发，不走 CI）=====
# 负责两块：① 代码 + tag；② 指定版本的 Release 附件。
# 官网不在 Gitee —— Gitee Pages 已下线，官网由 Cloudflare Pages 自动部署（dbmind.top，
# Git 集成跟着 master 走，push 即发布，无需本地操作；旧的 gitee-pages 分支已删）。
#
# 用法（在仓库根目录）：
#   powershell -File scripts\release\sync-gitee.ps1              # ① 代码+tag（几秒）
#   powershell -File scripts\release\sync-gitee.ps1 v2.0.7       # 再加 ② 该版本的 Release 附件
#
# 说明：
#   - 代码推送走本机直连 Gitee（国内，秒级）；附件下载 GitHub 资产走系统代理，
#     上传 Gitee 强制直连（NO_PROXY），脚本内部有 100MB 预跳过 + 增量补传
#     （同名同大小的附件不重传，中断重跑只传缺的）。
param([string]$Tag = '')

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root

$giteeTok  = (Get-Content "$env:USERPROFILE\.dbmind\gitee-token"  -Raw -Encoding UTF8).Trim()
$githubTok = (Get-Content "$env:USERPROFILE\.dbmind\github-token" -Raw -Encoding UTF8).Trim()
$giteeUrl  = "https://oauth2:$giteeTok@gitee.com/rick-works/dbmind.git"

# ---------- ① 代码 + 全部 tag（增量，强推对齐） ----------
Write-Host '==> [1/2] 代码 + tag -> Gitee'
git push $giteeUrl "+refs/heads/master:refs/heads/master"
git push $giteeUrl "+refs/tags/*:refs/tags/*"

# ---------- ② Release 附件（可选，带 tag 参数才跑） ----------
if ($Tag) {
  Write-Host "==> [2/2] Release $Tag 附件 -> Gitee（增量 + 3 路并发 + >100MB 预跳过）"
  $env:GITEE_TOKEN = $giteeTok
  $env:GH_TOKEN    = $githubTok
  # 本机 bash 里没有 jq，gitee-release.sh 依赖它（~/.dbmind/bin 下装了一份）
  $env:PATH = "$env:USERPROFILE\.dbmind\bin;$env:PATH"
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
