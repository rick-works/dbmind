# 一键发布到 GitHub：删除旧标签 v0.1.0、推送 v1.0.0 / v1.0.1、创建两个 Release
#
# 前置条件（二选一）：
#   A) 已把本机 SSH 公钥添加到 GitHub 账号（推荐，一次配好永久免密）
#      → 脚本会自动把 github remote 切成 SSH 地址
#   B) 已执行 `gh auth login --web` 登录 GitHub CLI
#      → 脚本用 gh 创建 Release
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File scripts\publish-github.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\publish-github.ps1 -SkipRelease   # 只推标签，不建 Release

param([switch]$SkipRelease)

$ErrorActionPreference = 'Stop'
$repo = 'rick-works/dbmind'
$sshUrl = "git@github.com:$repo.git"

Set-Location (Split-Path -Parent $PSScriptRoot)

function Say($msg) { Write-Host $msg -ForegroundColor Cyan }

# ---------- 0. 认证检查 ----------
$ghOk = $false
if (Get-Command gh -ErrorAction SilentlyContinue) {
  gh auth status 2>&1 | Out-Null
  $ghOk = ($LASTEXITCODE -eq 0)
}
$sshOk = $false
try {
  ssh -o StrictHostKeyChecking=accept-new -o BatchMode=yes -T git@github.com 2>&1 | Out-Null
  # GitHub 的 greeting 走 stderr 且退出码非 0，这里以「没有 publickey 错误」为准
  $sshOk = $true
} catch { $sshOk = $false }

if ($sshOk) {
  Say '检测到 SSH 可用，github remote 切到 SSH 地址'
  git remote set-url github $sshUrl
} elseif ($ghOk) {
  Say '检测到 gh 已登录，使用 gh 的凭据推送'
  git remote set-url github "https://github.com/$repo.git"
  gh auth setup-git 2>&1 | Out-Null
} else {
  Write-Host '认证未就绪。请先完成以下任一项：' -ForegroundColor Red
  Write-Host '  A) 把本机 SSH 公钥添加到 GitHub（Settings → SSH and GPG keys）'
  Write-Host '  B) 执行 gh auth login --web'
  exit 1
}

# ---------- 1. 删除远端旧标签 v0.1.0 ----------
Say '删除远端标签 v0.1.0（本地已重命名为 v1.0.0）'
git push github :refs/tags/v0.1.0

# ---------- 2. 推送新标签 ----------
Say '推送标签 v1.0.0 / v1.0.1'
git push github v1.0.0 v1.0.1

# ---------- 3. 创建 Release ----------
if ($SkipRelease) {
  Say '按要求跳过创建 Release（-SkipRelease）'
  exit 0
}
if (-not $ghOk) {
  Write-Host '未登录 gh，无法自动创建 Release。' -ForegroundColor Yellow
  Write-Host '请到 GitHub 网页新建 Release，说明文案在：'
  Write-Host '  release-notes\v1.0.0.md'
  Write-Host '  release-notes\v1.0.1.md'
  exit 0
}

foreach ($v in @('v1.0.0', 'v1.0.1')) {
  $notes = Join-Path $PSScriptRoot "..\release-notes\$v.md"
  $exists = gh release view $v --repo $repo 2>&1
  if ($LASTEXITCODE -eq 0) {
    Say "Release $v 已存在，跳过创建（如需更新：gh release edit $v --notes-file `"$notes`"）"
    continue
  }
  Say "创建 Release $v"
  gh release create $v --repo $repo --title $v --notes-file $notes
}

Say '完成：标签已同步，Release 已创建'
