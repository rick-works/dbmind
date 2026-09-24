# 停止 DBMind 的后端进程。
#
# 只杀「本项目的可执行文件」：按可执行文件路径过滤，不会误伤别的同名程序。
#
# 两种形态都要管：
#   1) 独立的 dbmind-web.exe —— scripts\start.ps1 起的就是它；
#   2) **桌面版** dbmind-desktop.exe —— 它的内核跑在**壳进程内部**（见 crates/dbmind-desktop
#      的 spawn_embedded），所以只杀 dbmind-web.exe 是停不掉它的。停桌面壳等于关掉那个窗口，
#      这里会补一句提示，免得"停完还能访问"看起来像没生效。
#
# 另：本文件必须存为 **UTF-8 with BOM**。PowerShell 5.1 读无 BOM 的 .ps1 会按 ANSI 解码，
# 中文注释被截成半个字符后引号配对被破坏，脚本会在**解析期**就整体失败（一行都不执行）。

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$targets = @(
    (Join-Path $root 'target\debug\dbmind-web.exe'),
    (Join-Path $root 'target\release\dbmind-web.exe'),
    (Join-Path $root 'target\debug\dbmind-desktop.exe'),
    (Join-Path $root 'target\release\dbmind-desktop.exe')
)

$killed = 0
foreach ($proc in Get-CimInstance Win32_Process | Where-Object { $_.CommandLine }) {
    foreach ($target in $targets) {
        if ($proc.CommandLine -like "*$target*") {
            Stop-Process -Id $proc.ProcessId -Force -ErrorAction SilentlyContinue
            Write-Host "已停止 PID $($proc.ProcessId)"
            $killed++
        }
    }
}
if ($killed -eq 0) { Write-Host '没有正在运行的本项目后端进程。' -ForegroundColor Yellow }
