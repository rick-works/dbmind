# Build the four agent host jars (JDBC + MongoDB + Redis + Elasticsearch).
#
# Why a script instead of doing it inline in package.ps1 / the desktop shell's build config:
# `tauri build` and `package.ps1` both need the jars, and the exact list of hosts
# must live in ONE place -- otherwise adding a host means editing several configs
# and forgetting one shows up as "host not ready" only after packaging.
#
# ASCII only: PowerShell 5.1 reads .ps1 as ANSI, so non-ASCII comments break parsing.

param(
    [switch]$Quiet,
    # Also build the library test servers under deploy/ (in-process Mongo /
    # Elasticsearch / Redis wire-protocol servers). Off by default: packaging does
    # not need them, only the smoke tests and the browser E2E do.
    [switch]$WithTestServers
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# Maven selection. Version matters: the shade plugin used by every agent needs
# Maven >= 3.6.3, and a too-old maven on PATH fails deep inside the plugin with a
# confusing "PluginIncompatibleException". So: try the candidates in order and
# pick the first one that is new enough, reporting what was rejected.
function Get-MavenVersion {
    param([string]$Path)
    $line = (& $Path -v 2>&1 | Select-Object -First 1)
    if ($line -match 'Apache Maven (\d+\.\d+\.\d+)') { return $Matches[1] }
    return $null
}

function Test-MavenNewEnough {
    param([string]$Version)
    $parts = $Version.Split('.')
    $major = [int]$parts[0]; $minor = [int]$parts[1]; $patch = [int]$parts[2]
    if ($major -ne 3) { return ($major -gt 3) }
    if ($minor -ne 6) { return ($minor -gt 6) }
    return ($patch -ge 3)
}

$candidates = @()
if ($env:DBMIND_MVN) { $candidates += $env:DBMIND_MVN }
$onPath = Get-Command mvn.cmd -ErrorAction SilentlyContinue
if ($onPath) { $candidates += $onPath.Source }
$candidates += 'D:\develop\tools\apache-maven-3.9.16\bin\mvn.cmd'

$mvn = $null
$rejected = @()
foreach ($candidate in $candidates) {
    if (-not (Test-Path $candidate)) { continue }
    $version = Get-MavenVersion $candidate
    if ($version -and (Test-MavenNewEnough $version)) { $mvn = $candidate; break }
    $shown = 'unknown'
    if ($version) { $shown = $version }
    $rejected += ($candidate + ' (Maven ' + $shown + ')')
}

if (-not $mvn) {
    Write-Host 'No usable Maven found (need >= 3.6.3 for the shade plugin):' -ForegroundColor Red
    if ($rejected.Count -eq 0) { Write-Host '  - none found on PATH or in known locations' -ForegroundColor Red }
    $rejected | ForEach-Object { Write-Host ('  - rejected ' + $_) -ForegroundColor Red }
    Write-Host '  set DBMIND_MVN to a newer mvn.cmd, or put one on PATH' -ForegroundColor Red
    exit 1
}

$agents = @(
    @{ Name = 'jdbc';          Jar = 'dbmind-agent-jdbc.jar' },
    @{ Name = 'mongodb';       Jar = 'dbmind-agent-mongodb.jar' },
    @{ Name = 'redis';         Jar = 'dbmind-agent-redis.jar' },
    @{ Name = 'elasticsearch'; Jar = 'dbmind-agent-elasticsearch.jar' }
)

foreach ($agent in $agents) {
    $project = Join-Path $root ('agents\dbmind-agent-' + $agent.Name)
    if (-not (Test-Path $project)) { throw ('missing agent project: ' + $project) }
    if (-not $Quiet) { Write-Host ('  building ' + $agent.Jar) }
    $arguments = @('-f', (Join-Path $project 'pom.xml'), '-q', '-DskipTests', 'package')
    & $mvn @arguments
    if ($LASTEXITCODE -ne 0) { throw ('maven failed for ' + $agent.Name) }
    $jar = Join-Path $project ('target\' + $agent.Jar)
    if (-not (Test-Path $jar)) { throw ('jar not produced: ' + $jar) }
    if (-not $Quiet) { Write-Host ('    ok ' + [Math]::Round((Get-Item $jar).Length / 1KB, 1) + ' KB') }
}

if ($WithTestServers) {
    $servers = @(
        @{ Name = 'mongo-test-server'; Jar = 'dbmind-mongo-test-server.jar' },
        @{ Name = 'es-test-server';    Jar = 'dbmind-es-test-server.jar' },
        @{ Name = 'redis-test-server'; Jar = 'dbmind-redis-test-server.jar' }
    )
    foreach ($server in $servers) {
        $project = Join-Path $root ('deploy\' + $server.Name)
        if (-not (Test-Path $project)) { throw ('missing test server project: ' + $project) }
        if (-not $Quiet) { Write-Host ('  building ' + $server.Jar) }
        $arguments = @('-f', (Join-Path $project 'pom.xml'), '-q', '-DskipTests', 'package')
        & $mvn @arguments
        if ($LASTEXITCODE -ne 0) { throw ('maven failed for ' + $server.Name) }
        $jar = Join-Path $project ('target\' + $server.Jar)
        if (-not (Test-Path $jar)) { throw ('jar not produced: ' + $jar) }
        if (-not $Quiet) { Write-Host ('    ok ' + [Math]::Round((Get-Item $jar).Length / 1KB, 1) + ' KB') }
    }
}

# Mirror the freshly built jars into the dev run directories.
#
# Why: the desktop binary resolves a host jar next to the executable FIRST
# (target\debug\agents\*, target\release\agents\*) and only then falls back to
# agents/*/target. So rebuilding a jar without this copy keeps the OLD host
# running -- the change looks like "it did not take effect at all".
# That trap cost a full debugging round; keep the mirroring here.
foreach ($agent in $agents) {
    $built = Join-Path $root ('agents\dbmind-agent-' + $agent.Name + '\target\' + $agent.Jar)
    foreach ($profile in @('debug', 'release')) {
        $dir = Join-Path $root ('target\' + $profile + '\agents')
        if (-not (Test-Path $dir)) { continue }
        Copy-Item $built (Join-Path $dir $agent.Jar) -Force
        if (-not $Quiet) { Write-Host ('    mirrored -> target\' + $profile + '\agents\' + $agent.Jar) }
    }
}

if (-not $Quiet) { Write-Host '  all agent hosts built' }
