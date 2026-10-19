<#
把主仓的 website/index.html 发布到 gh-pages 分支（GitHub Pages 的源）。

为什么需要它：官网的源原先**只活在 gh-pages 分支里** —— 主仓看不见它，于是每次发版都容易
漏更新（用户原话："之前你官网版本内容都没更新"，一漏就是 8 个版本）。
现在 website/ 纳入主仓跟踪，发版时改的是主仓那份，再用这个脚本推上 gh-pages，
改动就有 diff、能被 review；release.mjs 里还会检查"新版本的条目加了没"。

用法：
  powershell -ExecutionPolicy Bypass -File scripts\release\publish-site.ps1
  powershell -ExecutionPolicy Bypass -File scripts\release\publish-site.ps1 -Message "官网：..."

凭据：优先 $env:GH_TOKEN，其次 `gh auth token`。
#>
[CmdletBinding()]
param(
    [string]$Message = '官网：同步 website/index.html',
    [string]$WorkDir = (Join-Path $env:TEMP 'dbmind-gh-pages')
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $root
# 系统代理助手（Windows 的代理在注册表里，git 默认不读 —— 开代理时直连会一直超时）
. (Join-Path $PSScriptRoot '..\tools\git-proxy.ps1')

$src = Join-Path $root 'website\index.html'
if (-not (Test-Path $src)) { throw "没找到官网源文件：$src" }

# ---- 准备 gh-pages 工作树 ----
# 用 worktree 而不是 checkout：不切换当前分支，不会打断手头的改动；已存在就复用。
$listed = (git worktree list) -join "`n"
if ($listed -notmatch [regex]::Escape($WorkDir)) {
    if (Test-Path $WorkDir) { git worktree remove $WorkDir --force 2>&1 | Out-Null }
    git worktree add $WorkDir gh-pages 2>&1 | Out-Null
}
$dest = Join-Path $WorkDir 'index.html'
if (-not (Test-Path $dest)) { throw "工作树里没有 index.html：$dest" }

# 只换 index.html；图片、sponsors.js、.nojekyll 不动

# ---- 取凭据（提到前面：下面要先跟远端对齐一次）----
$tok = $env:GH_TOKEN
if (-not $tok) {
    try { $tok = (& gh auth token 2>$null | Select-Object -First 1).Trim() } catch { $tok = '' }
}
if (-not $tok) { throw '没有可用的 GitHub 凭据：先设 $env:GH_TOKEN，或执行 gh auth login' }
$pushUrl = "https://rick-works:$tok@github.com/rick-works/dbmind.git"
$proxy = Get-GitProxyArgs   # 有系统代理就用（没开就是空数组 = 直连）

# ---- 先把工作树对齐到远端 gh-pages ----
# 本地 gh-pages 落后时，直接 commit 再 push 会被判"非快进"而拒绝；而脚本失败提示里的
# "稍后重跑本脚本即可"在这种情况下**永远重跑不成功**（本地始终落后，冲突的是同一处）。
# 先 fetch 远端再硬对齐：我们只关心 index.html 一个文件，而它下面会被整份覆盖，
# 所以"以远端为基线"是安全的（其余文件保持远端版本，不会被本地的旧副本回退）。
$prev = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
& git -C $WorkDir @proxy fetch $pushUrl refs/heads/gh-pages 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) {
    & git -C $WorkDir reset --hard FETCH_HEAD 2>&1 | Out-Null
    Write-Host '  已对齐远端 gh-pages' -ForegroundColor DarkGray
} else {
    Write-Host '  拉取远端失败（可能是网络），按本地提交继续' -ForegroundColor Yellow
}
$ErrorActionPreference = $prev

Copy-Item $src $dest -Force

# ---- 没改动就什么都不做 ----
# 少了这道判断，重复跑会在下面的 commit 上抛错（"nothing to commit" 退出码非 0，
# 而 $ErrorActionPreference='Stop' 会把它当异常）
$pending = (git -C $WorkDir status --porcelain) -join "`n"
if ($pending -notmatch 'index\.html') {
    Write-Host '官网内容无变化，无需推送' -ForegroundColor Green
    exit 0
}
git -C $WorkDir add index.html
git -C $WorkDir commit -q -m $Message
Write-Host ('已提交：' + ((git -C $WorkDir log --oneline -1) -join ''))

# ---- 推送（github 在国内时通时断，重试）----
# 凭据 / $pushUrl / $proxy 已在上面（跟远端对齐那一步）取好，这里不再重复获取。
$pushed = $false
for ($k = 1; $k -le 8; $k++) {
    # git 会把进度写进 stderr，而 $ErrorActionPreference='Stop' 会把原生命令的 stderr 当成
    # **终止错误** —— 于是"其实推成功了"也会抛 NativeCommandError、脚本以 1 退出（踩过）。
    # 这里临时放宽，成败只看 $LASTEXITCODE。
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & git -C $WorkDir @proxy push $pushUrl refs/heads/gh-pages:refs/heads/gh-pages 2>&1 | Out-Null
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($code -eq 0) { $pushed = $true; Write-Host ("  已推送 gh-pages（第 $k 次）") -ForegroundColor Green; break }
    Write-Host ("  第 $k 次推送失败（github 时通时断），重试…") -ForegroundColor Yellow
    Start-Sleep -Seconds 12
}
if (-not $pushed) { throw '推送失败：本地 gh-pages 已提交，稍后重跑本脚本即可' }

Write-Host '完成。GitHub Pages 会在几十秒内重新构建。' -ForegroundColor Green
