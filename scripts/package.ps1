# Assemble a portable DBmind bundle.
#
# Layout (the kernel resolves hosts from <exe dir>/agents, or from DBMIND_AGENTS_DIR):
#
#   dbmind-<version>/
#     dbmind.exe            CLI shell
#     dbmind-web.exe        web shell (also embedded by the desktop shell)
#     dbmind-mcp.exe        MCP shell (stdio)
#     agents/               agent hosts -- without these, MongoDB/Redis/ES/JDBC are "not ready"
#     web/                  frontend build (serve with: dbmind-web.exe --dist web)
#     README.md
#
# Why ship the jars: the desktop shell runs the engine IN PROCESS, so "agent host"
# means "a jar next to my executable". Miss that and every agent-backed type silently
# reports DBMIND-DRV-0002 on the user's machine.
#
# ASCII only: PowerShell 5.1 reads .ps1 as ANSI, so non-ASCII comments break parsing.

param(
    [string]$Out = '',
    [ValidateSet('release', 'debug')][string]$Profile = 'release',
    [switch]$SkipCargo,
    [switch]$SkipAgents,
    [switch]$SkipFrontend
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# Version comes from the workspace so it can never drift from the binaries.
$cargoToml = Get-Content (Join-Path $root 'Cargo.toml') -Raw -Encoding UTF8
$version = [regex]::Match($cargoToml, '(?ms)\[workspace\.package\].*?version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $version) { $version = '0.0.0' }

$targetProfile = if ($Profile -eq 'release') { 'release' } else { 'debug' }
$outDir = if ($Out) { $Out } else { Join-Path $root ('dist\dbmind-' + $version) }

Write-Host ('=== DBmind package (' + $Profile + ') -> ' + $outDir)

# ---------------------------------------------------------------- binaries
if (-not $SkipCargo) {
    Write-Host '  [1/4] cargo build'
    $cargoArgs = @('build')
    if ($Profile -eq 'release') { $cargoArgs += '--release' }
    Push-Location $root
    try { & cargo @cargoArgs } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
}
else {
    Write-Host '  [1/4] cargo build (skipped)'
}

# ---------------------------------------------------------------- agent hosts
if (-not $SkipAgents) {
    Write-Host '  [2/4] agent hosts'
    & (Join-Path $PSScriptRoot 'build-agents.ps1') -Quiet
}
else {
    Write-Host '  [2/4] agent hosts (skipped)'
}

# ---------------------------------------------------------------- frontend
if (-not $SkipFrontend) {
    Write-Host '  [3/4] frontend'
    Push-Location (Join-Path $root 'frontend')
    try { & npm run build } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw 'frontend build failed' }
}
else {
    Write-Host '  [3/4] frontend (skipped)'
}

# ---------------------------------------------------------------- assemble
Write-Host '  [4/4] assemble'
if (Test-Path $outDir) { Remove-Item $outDir -Recurse -Force }
New-Item -ItemType Directory -Path $outDir -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $outDir 'agents') -Force | Out-Null

$binDir = Join-Path $root ('target\' + $targetProfile)
$missing = @()
# dbmind-desktop.exe：Tauri 桌面壳（自起内嵌内核 + 系统 WebView）。
# 它跑起来会自己找 20361；那儿已经有 dbmind-web.exe 时就复用，不抢端口。
foreach ($bin in @('dbmind.exe', 'dbmind-web.exe', 'dbmind-mcp.exe', 'dbmind-desktop.exe')) {
    $source = Join-Path $binDir $bin
    if (Test-Path $source) {
        Copy-Item $source $outDir
    }
    else {
        # A partial bundle is fine for local checks, but it must be loud about it.
        $missing += $bin
    }
}

foreach ($jar in @('dbmind-agent-jdbc.jar', 'dbmind-agent-mongodb.jar', 'dbmind-agent-redis.jar', 'dbmind-agent-elasticsearch.jar')) {
    $source = Join-Path $root ('agents\' + [System.IO.Path]::GetFileNameWithoutExtension($jar) + '\target\' + $jar)
    if (Test-Path $source) {
        Copy-Item $source (Join-Path $outDir 'agents')
    }
    else {
        $missing += ('agents/' + $jar)
    }
}

$webSource = Join-Path $root 'frontend\dist'
if (Test-Path $webSource) {
    Copy-Item $webSource (Join-Path $outDir 'web') -Recurse
}

# Launcher: double-clicking start.bat is the whole "install" story for the portable build.
$launcher = Join-Path $PSScriptRoot 'start.bat'
if (Test-Path $launcher) { Copy-Item $launcher $outDir }

foreach ($extra in @('README.md', 'LICENSE')) {
    $source = Join-Path $root $extra
    if (Test-Path $source) { Copy-Item $source $outDir }
}

if ($missing.Count -gt 0) {
    Write-Host ('  [WARN] not included: ' + ($missing -join ', ')) -ForegroundColor Yellow
}

# ---------------------------------------------------------------- report
$total = 0
Write-Host ''
Write-Host ('  ' + $outDir)
Get-ChildItem $outDir -Recurse -File | Sort-Object FullName | ForEach-Object {
    $relative = $_.FullName.Substring($outDir.Length + 1)
    $size = [Math]::Round($_.Length / 1KB, 1)
    $total += $_.Length
    Write-Host ('    ' + $size.ToString().PadLeft(9) + ' KB  ' + $relative)
}
Write-Host ('  total ' + [Math]::Round($total / 1MB, 1) + ' MB')
Write-Host ''
Write-Host '  run:'
Write-Host ('    ' + (Join-Path $outDir 'dbmind.exe') + ' driver list')
Write-Host ('    ' + (Join-Path $outDir 'dbmind-web.exe') + ' --port 8787 --dist web')
