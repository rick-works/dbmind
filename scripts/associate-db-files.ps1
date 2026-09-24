<#
把 .db 文件关联到 DBMind（图标 + 双击用 DBMind 打开）。

为什么需要这个脚本：
  Windows 上「.db 显示什么图标」由注册表里的文件关联决定。
  机器上若装过别的数据库工具，它会把 .db 注册成自己的类型（ProgId 形如 "Database File"），
  于是资源管理器里连我们自己的数据文件 dbmind.db 也用它的图标。
  这个脚本把 .db 指到 DBMind 自己的类型与图标上。

特点：
  · 只写 HKCU（当前用户），不需要管理员，也不影响别的用户
  · 可逆：改之前会把原来的关联备份下来，-Restore 一键还原
  · 图标从本仓库复制到 %LOCALAPPDATA%\DBMind\icons\，仓库挪走也不影响

用法：
  powershell -ExecutionPolicy Bypass -File scripts\associate-db-files.ps1
  powershell -ExecutionPolicy Bypass -File scripts\associate-db-files.ps1 -Restore
#>
[CmdletBinding()]
param(
  # 还原成改之前的样子
  [switch]$Restore,
  # 指定双击 .db 时用哪个程序打开（默认自动找 target\release\dbmind-desktop.exe）
  [string]$AppPath = ''
)

$ErrorActionPreference = 'Stop'
$PROGID = 'DBMind.Database'
$ROOT = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$ICON_SRC = Join-Path $ROOT 'crates\dbmind-desktop\icons\icon.ico'
$STORE = Join-Path $env:LOCALAPPDATA 'DBMind'
$ICON_DST = Join-Path $STORE 'icons\dbmind-db.ico'
$BACKUP = Join-Path $STORE 'db-assoc-backup.txt'

function Refresh-Shell {
  # 通知资源管理器：文件关联变了（否则图标要等很久才刷新）
  try {
    Add-Type -Namespace Win32 -Name Shell -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("shell32.dll", CharSet=System.Runtime.InteropServices.CharSet.Auto)]
public static extern void SHChangeNotify(int eventId, int flags, System.IntPtr item1, System.IntPtr item2);
'@ -ErrorAction SilentlyContinue
    [Win32.Shell]::SHChangeNotify(0x08000000, 0x0000, [IntPtr]::Zero, [IntPtr]::Zero)
  } catch { }
  # 再让系统重建一次图标缓存（有的机器上光通知不够）
  try { Start-Process -FilePath 'ie4uinit.exe' -ArgumentList '-show' -WindowStyle Hidden -ErrorAction SilentlyContinue } catch { }
}

if ($Restore) {
  Write-Host '把 .db 还原成改动之前的样子…' -ForegroundColor Cyan
  $prev = if (Test-Path $BACKUP) { (Get-Content -Raw -Encoding UTF8 $BACKUP).Trim() } else { '' }
  if ($prev) {
    New-Item -Path 'HKCU:\Software\Classes\.db' -Force | Out-Null
    Set-ItemProperty -Path 'HKCU:\Software\Classes\.db' -Name '(default)' -Value $prev
    Write-Host ('  ✓ .db 已还原为 ' + $prev) -ForegroundColor Green
  } else {
    Remove-Item -Path 'HKCU:\Software\Classes\.db' -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host '  ✓ 之前没有备份，已移除我们写的那项（.db 回到系统默认）' -ForegroundColor Green
  }
  Remove-Item -Path ('HKCU:\Software\Classes\' + $PROGID) -Recurse -Force -ErrorAction SilentlyContinue
  Write-Host '  ✓ 已删除 DBMind 的类型项' -ForegroundColor Green
  Refresh-Shell
  Write-Host '完成。资源管理器里按 F5 看一眼即可。' -ForegroundColor Cyan
  return
}

# ---------------------------------------------------------------- 准备
if (-not (Test-Path $ICON_SRC)) { throw ('找不到图标：' + $ICON_SRC) }
New-Item -ItemType Directory -Path (Split-Path $ICON_DST) -Force | Out-Null
Copy-Item $ICON_SRC $ICON_DST -Force
Write-Host ('图标已就位：' + $ICON_DST) -ForegroundColor Green

if (-not $AppPath) {
  foreach ($c in @('target\release\dbmind-desktop.exe', 'target\debug\dbmind-desktop.exe')) {
    $p = Join-Path $ROOT $c
    if (Test-Path $p) { $AppPath = $p; break }
  }
}

# 备份原来的关联（只备份一次，免得把我们的值当成"原来的"）
if (-not (Test-Path $BACKUP)) {
  $cur = ''
  try { $cur = (Get-Item 'HKCU:\Software\Classes\.db' -ErrorAction Stop).GetValue('') } catch { }
  # 也看看 HKCR（系统级）里的值，虽然还原时只写回 HKCU
  if (-not $cur) { try { $cur = (Get-Item 'Registry::HKEY_CLASSES_ROOT\.db' -ErrorAction Stop).GetValue('') } catch { } }
  New-Item -ItemType Directory -Path $STORE -Force | Out-Null
  Set-Content -Path $BACKUP -Value ([string]$cur) -Encoding UTF8
  Write-Host ('已备份原来的关联：' + $(if ($cur) { $cur } else { '(空)' })) -ForegroundColor DarkGray
}

# ---------------------------------------------------------------- 写关联
$base = 'HKCU:\Software\Classes\' + $PROGID
New-Item -Path $base -Force | Out-Null
Set-ItemProperty -Path $base -Name '(default)' -Value 'DBMind 数据库'
Set-ItemProperty -Path $base -Name 'FriendlyTypeName' -Value 'DBMind 数据库'
New-Item -Path ($base + '\DefaultIcon') -Force | Out-Null
Set-ItemProperty -Path ($base + '\DefaultIcon') -Name '(default)' -Value ('"' + $ICON_DST + '"')

if ($AppPath -and (Test-Path $AppPath)) {
  New-Item -Path ($base + '\shell\open\command') -Force | Out-Null
  Set-ItemProperty -Path ($base + '\shell\open\command') -Name '(default)' -Value ('"' + $AppPath + '" "%1"')
  Write-Host ('双击 .db 将用：' + $AppPath) -ForegroundColor Green
} else {
  Write-Host '没找到已构建的 DBMind 桌面程序 —— 只登记图标，不接管双击（双击仍走系统默认）' -ForegroundColor Yellow
}

New-Item -Path 'HKCU:\Software\Classes\.db' -Force | Out-Null
Set-ItemProperty -Path 'HKCU:\Software\Classes\.db' -Name '(default)' -Value $PROGID
Set-ItemProperty -Path 'HKCU:\Software\Classes\.db' -Name 'Content Type' -Value 'application/x-dbmind-database'

Refresh-Shell
Write-Host '' 
Write-Host '.db 已关联到 DBMind ✓' -ForegroundColor Cyan
Write-Host '  恢复到原样：  powershell -ExecutionPolicy Bypass -File scripts\associate-db-files.ps1 -Restore' -ForegroundColor DarkGray
Write-Host '  资源管理器里按 F5；若图标还没变，注销/重启一次即可（系统图标缓存）' -ForegroundColor DarkGray
