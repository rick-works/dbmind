<#
发版后同步 Homebrew cask（在**另一个仓库**：rick-works/homebrew-tap）。

为什么是个脚本、而不是 release.yml 里的一步：
  cask 在另一个仓库，而 Actions 自带的 GITHUB_TOKEN **只能写当前仓库** —— 要自动化就得再配一个
  有跨仓库写权限的 PAT。这件事每次发版只跑一次，用脚本比多养一个长期密钥划算。
  （想自动化时：造一个 fine-grained PAT，只需 rick-works/homebrew-tap 的 Contents: write，
   存成主仓库的 secret，然后把下面这几步搬进 workflow 的最后一个 job。）

用法：
  powershell -File scripts\update-homebrew-cask.ps1                 # 取最新 Release
  powershell -File scripts\update-homebrew-cask.ps1 -Version 1.2.7  # 指定版本

它做四件事：
  1. 从 GitHub API 读该 Release 里两个 .dmg 的 sha256（assets[].digest —— **不用下载** 100+MB）；
  2. 克隆（或复用已缓存的）rick-works/homebrew-tap；
  3. 重写 Casks/dbmind.rb 里的 version 与两个 sha256（按出现顺序：先 arm 后 intel）；
  4. 提交并推送。

凭据：优先 $env:GH_TOKEN，其次 `gh auth token`。
#>
[CmdletBinding()]
param(
    [string]$Version = '',
    [string]$TapDir = (Join-Path $env:TEMP 'dbmind-homebrew-tap')
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
# 系统代理助手（Windows 的代理在注册表里，git 默认不读 —— 开代理时直连会一直超时）
. (Join-Path $PSScriptRoot 'git-proxy.ps1')
$proxy = Get-GitProxyArgs   # fetch / clone / push 都用它：有代理就走，没开就是空数组（直连）

$repo = 'rick-works/dbmind'
$tapRepo = 'rick-works/homebrew-tap'

# ---------------------------------------------------------------- 凭据
$tok = $env:GH_TOKEN
if (-not $tok) {
    try { $tok = (& gh auth token 2>$null | Select-Object -First 1).Trim() } catch { $tok = '' }
}
if (-not $tok) { throw '没有可用的 GitHub 凭据：先设 $env:GH_TOKEN，或执行 gh auth login' }
$env:GH_TOKEN = $tok

# ---------------------------------------------------------------- 版本号
if (-not $Version) {
    $Version = ((& gh api "repos/$repo/releases/latest" --jq '.tag_name') | Select-Object -First 1).Trim() -replace '^v', ''
}
if (-not $Version) { throw '取不到版本号：用 -Version 指定' }
Write-Host ("同步 cask 到 v" + $Version) -ForegroundColor Cyan

# ---------------------------------------------------------------- 两个 dmg 的 sha256
$rel = (& gh api "repos/$repo/releases/tags/v$Version" | Out-String | ConvertFrom-Json)
$arm = (($rel.assets | Where-Object { $_.name -like '*aarch64.dmg' }).digest -replace '^sha256:', '')
$intel = (($rel.assets | Where-Object { $_.name -like '*_x64.dmg' }).digest -replace '^sha256:', '')
if (-not $arm -or -not $intel) {
    throw ("没取到 dmg 的 sha256（该 Release 现有资产：" + (($rel.assets | ForEach-Object { $_.name }) -join ', ') + "）")
}
Write-Host ('  arm   ' + $arm)
Write-Host ('  intel ' + $intel)

# ---------------------------------------------------------------- 更新 tap 工作副本
$git = Join-Path $TapDir '.git'
if (Test-Path $git) {
    & git -C $TapDir @proxy fetch -q origin
    & git -C $TapDir checkout -q main
    & git -C $TapDir reset -q --hard origin/main
} else {
    if (Test-Path $TapDir) { Remove-Item $TapDir -Recurse -Force }
    & git @proxy clone -q ("https://github.com/" + $tapRepo + ".git") $TapDir
}

$cask = Join-Path $TapDir 'Casks/dbmind.rb'
if (-not (Test-Path $cask)) { throw ('tap 里没有 Casks/dbmind.rb：' + $cask) }

$noBom = New-Object System.Text.UTF8Encoding($false)
$lines = [System.IO.File]::ReadAllText($cask, [System.Text.Encoding]::UTF8) -split "`n"
$shaSeen = 0
for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match '^  version "') {
        $lines[$i] = '  version "' + $Version + '"'
    } elseif ($lines[$i] -match '^    sha256 "[0-9a-f]{64}"') {
        $shaSeen++
        $lines[$i] = '    sha256 "' + $(if ($shaSeen -eq 1) { $arm } else { $intel }) + '"'
    }
}
if ($shaSeen -ne 2) { throw ("cask 里的 sha256 行数不是 2（实际 " + $shaSeen + "）：模板被改过了？") }
[System.IO.File]::WriteAllText($cask, ($lines -join "`n"), $noBom)
Write-Host '  已改写 Casks/dbmind.rb'

# ---------------------------------------------------------------- 提交并推送
& git -C $TapDir add Casks/dbmind.rb
# 已经是这个版本时什么都不用做。少了这道判断，重复跑会在下面那条 commit 上抛错
# （"nothing to commit" 的退出码非 0，而 $ErrorActionPreference='Stop' 会把它当异常）
$pending = (& git -C $TapDir status --porcelain) -join "`n"
if ($pending -notmatch 'dbmind\.rb') {
    Write-Host ('  cask 已经是 v' + $Version + '，无需改动') -ForegroundColor Green
    exit 0
}
& git -C $TapDir commit -q -m ("dbmind " + $Version)
$pushUrl = "https://rick-works:$tok@github.com/$tapRepo.git"
$pushed = $false
for ($k = 1; $k -le 8; $k++) {
    # git 会把进度写进 stderr，而 $ErrorActionPreference='Stop' 会把它当成**终止错误** ——
    # 于是"其实推成功了"也会抛 NativeCommandError、脚本以 1 退出（踩过）。成败只看 $LASTEXITCODE。
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & git -C $TapDir @proxy push $pushUrl main 2>&1 | Out-Null
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($code -eq 0) { $pushed = $true; Write-Host ("  推送 OK（第 $k 次）") -ForegroundColor Green; break }
    Write-Host ("  第 $k 次推送失败（github 时通时断），重试…") -ForegroundColor Yellow
    Start-Sleep -Seconds 12
}
if (-not $pushed) { throw '推送失败：本地已提交，稍后重跑本脚本即可' }
Write-Host '完成。' -ForegroundColor Green
