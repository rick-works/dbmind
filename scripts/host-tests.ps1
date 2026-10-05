<#
  DBmind host layer: the kernel unit tests that talk to REAL agent hosts (JVM).

  Why a separate layer:
    By default `cargo test` does NOT load agent hosts at all (see
    `agent_hosts_visible` in crates/dbmind-core/src/agent.rs): no JVM is spawned,
    a full run takes ~0.2s, and results do not depend on what this machine
    happens to have built. The price of that default is that the
    "kernel <-> host" path has no unit coverage -- this layer pays it back.

  It is strict on purpose: if the environment cannot support it (no Java, agent
  jars not built, H2 JDBC driver missing), it FAILS and says what to do.
  A layer that quietly skips itself is worse than no layer.

  NOTE: this file is intentionally ASCII-only (see scripts/smoke.ps1).

  Usage: powershell -ExecutionPolicy Bypass -File .\scripts\host-tests.ps1
#>

$ErrorActionPreference = 'Continue'

$root = Split-Path -Parent $PSScriptRoot
$stamp = Get-Date -Format 'yyyyMMddHHmmss'
$work = Join-Path $env:TEMP "dbmind-host-tests-$stamp"
$homeDir = Join-Path $work 'home'
New-Item -ItemType Directory -Path $homeDir -Force | Out-Null

# A throwaway HOME: this layer installs the H2 JDBC driver, and we want neither
# to touch the real one nor to depend on whatever is already installed there.
$env:DBMIND_HOME = $homeDir
$env:DBMIND_TEST_AGENTS = '1'

$script:passed = 0
$script:failed = 0

function Check {
    param([string]$Title, [bool]$Ok, [string]$Detail)
    if ($Ok) {
        Write-Host "  [PASS] $Title" -ForegroundColor Green
        $script:passed++
    }
    else {
        Write-Host "  [FAIL] $Title -- $Detail" -ForegroundColor Red
        $script:failed++
    }
}

Write-Host ""
Write-Host "=== host layer: preconditions ===" -ForegroundColor Cyan

$cli = Join-Path $root 'target\debug\dbmind.exe'
Check 'cli binary exists' (Test-Path $cli) "missing $cli -- run 'cargo build' first"
if (-not (Test-Path $cli)) {
    Write-Host ""
    Write-Host "  cannot continue without the cli binary" -ForegroundColor Red
    exit 1
}

$agents = @(Get-ChildItem (Join-Path $root 'agents') -Recurse -File -Filter 'dbmind-agent-*.jar' -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -notlike 'original-*' })
Check "agent hosts are built (found $($agents.Count))" ($agents.Count -ge 4) 'run scripts/build-agents.ps1'

$java = $null
if ($env:DBMIND_JAVA) { $java = $env:DBMIND_JAVA }
elseif ($env:JAVA_HOME) { $java = Join-Path $env:JAVA_HOME 'bin\java.exe' }
else {
    $cmd = Get-Command java -ErrorAction SilentlyContinue
    if ($cmd) { $java = $cmd.Source }
}
Check 'java runtime is available' ($null -ne $java -and (Test-Path $java)) 'set DBMIND_JAVA or JAVA_HOME, or put java on PATH'
if ($java -and (Test-Path $java)) {
    # The agent hosts are compiled for Java 17. An older JVM on PATH exits with
    # UnsupportedClassVersionError, which would show up later as a puzzling
    # "handshake failed" -- so fail here, with the version in hand.
    $versionText = (& $java -version 2>&1 | Out-String)
    $major = 0
    if ($versionText -match 'version "(\d+)') { $major = [int]$Matches[1] }
    Check "java is >= 17 (agents are built for 17)" ($major -ge 17) ("resolved to $java -- " + ($versionText -replace "`r?`n", ' / '))
}

# The H2 round trip needs the real H2 JDBC driver: the host does not bundle it,
# it reads it from the driver directory (exactly like a user's install does).
# ~2.5 MB, needs network once -- the same path the cli smoke already exercises.
Write-Host ""
Write-Host "=== host layer: install the H2 JDBC driver (into the throwaway HOME) ===" -ForegroundColor Cyan
$fetch = (& $cli driver fetch h2 2>&1 | Out-String).Trim()
$h2jars = @(Get-ChildItem (Join-Path $homeDir 'drivers\h2') -File -Filter '*.jar' -ErrorAction SilentlyContinue)
Check "H2 JDBC driver installed ($($h2jars.Count) jar)" ($h2jars.Count -ge 1) ($fetch -replace "`r?`n", ' / ')

# The concurrency tests need real protocol peers -- one per protocol host that the
# matrix covers (multi-connection: Mongo + ES; single-connection: Redis). All three
# are in-repo stubs (built by `build-agents.ps1 -WithTestServers`) and run on their
# own ports, so they cannot collide with a real install or with the CI smoke job
# (which already runs its own on 27017 / 9202 / 6399).
Write-Host ""
Write-Host "=== host layer: start the library test servers ===" -ForegroundColor Cyan
$stubs = @(
    @{ Name = 'mongo'; Jar = 'deploy\mongo-test-server\target\dbmind-mongo-test-server.jar'; Port = 27117; Env = 'DBMIND_TEST_MONGO_PORT' },
    @{ Name = 'es';    Jar = 'deploy\es-test-server\target\dbmind-es-test-server.jar';       Port = 9212;  Env = 'DBMIND_TEST_ES_PORT' },
    @{ Name = 'redis'; Jar = 'deploy\redis-test-server\target\dbmind-redis-test-server.jar'; Port = 6319;  Env = 'DBMIND_TEST_REDIS_PORT' }
)
$stubProcs = @()
foreach ($stub in $stubs) {
    Set-Item -Path ("Env:" + $stub.Env) -Value "$($stub.Port)"
    $jar = Join-Path $root $stub.Jar
    if (-not (Test-Path $jar)) {
        Check ($stub.Name + ' test server jar exists') $false "missing $jar -- run scripts/build-agents.ps1 -WithTestServers"
        continue
    }
    $log = Join-Path $work ($stub.Name + '-stub.log')
    $proc = Start-Process java -ArgumentList '-jar', $jar, "$($stub.Port)" -PassThru -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError "$log.err"
    $stubProcs += $proc
    $ready = $false
    for ($i = 0; $i -lt 30; $i++) {
        try { $c = [System.Net.Sockets.TcpClient]::new('127.0.0.1', $stub.Port); $c.Close(); $ready = $true; break }
        catch { Start-Sleep -Seconds 1 }
    }
    Check ($stub.Name + " test server listening on " + $stub.Port) $ready "see $log"
    if (-not $ready) {
        Get-Content $log -ErrorAction SilentlyContinue
        Get-Content "$log.err" -ErrorAction SilentlyContinue
    }
}

# --- the layer itself --------------------------------------------------------
# `--ignored` picks exactly the tests marked as host-layer ones (the default run
# reports them in its summary as "ignored", so they are never silently missing).
Write-Host ""
Write-Host "=== host layer: cargo test -p dbmind-core -- --ignored ===" -ForegroundColor Cyan

$before = @(Get-Process java -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)

Push-Location $root
# `--lib` on purpose: this layer is exactly the lib's `#[ignore]`d tests. It also
# keeps `tests/real_redis.rs` out -- that one talks to a REAL Redis (not a stub)
# and is opt-in via scripts/real-redis-check.ps1; it prints [SKIP] when its env
# gate is off, so letting it run here would quietly inflate this layer's count.
$output = (& cargo test -p dbmind-core --lib -- --ignored --nocapture 2>&1 | Out-String)
$exit = $LASTEXITCODE
Pop-Location

$lines = ($output -split "`r?`n")
$lines | Where-Object { $_ -match 'test result:|FAILED|panicked|^error' } | ForEach-Object { Write-Host "  $_" }
if ($exit -ne 0) {
    # libtest prints the assertion message on the line AFTER "panicked at ...",
    # which the filter above does not catch -- dump the tail so a red CI run is
    # diagnosable without re-running anything.
    Write-Host "  --- last 20 lines of the failing run ---" -ForegroundColor DarkYellow
    $lines | Where-Object { $_.Trim() -ne '' } | Select-Object -Last 20 | ForEach-Object { Write-Host "  $_" }
}

$resultLine = ($lines | Where-Object { $_ -match 'test result:' } | Select-Object -First 1)
Check 'all host-layer tests pass' ($exit -eq 0) $resultLine

# --- no orphan hosts ---------------------------------------------------------
# This layer spawns real JVMs; if one survives the run, the kernel's shutdown
# path is leaking (that bug -- shutdown starting a host just to say goodbye --
# is exactly what this layer was built next to). Give kill() a moment first.
Start-Sleep -Seconds 2
$after = @(Get-Process java -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
$orphans = @($after | Where-Object { $before -notcontains $_ })
Check "no orphan agent JVMs left behind (found $($orphans.Count))" ($orphans.Count -eq 0) ("pids: " + ($orphans -join ', '))
if ($orphans.Count -gt 0) {
    Write-Host "  cleaning up the orphans so the next run is not affected" -ForegroundColor DarkYellow
    $orphans | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
}

# Stop the stubs we started (the orphan check above only looks at JVMs that were
# NOT running before the layer, i.e. the hosts the kernel spawned itself).
foreach ($proc in $stubProcs) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "================ HOST LAYER RESULT ================" -ForegroundColor Yellow
Write-Host "  passed: $script:passed   failed: $script:failed"
Write-Host "  temp home: $work"
Write-Host "=================================================="

if ($script:failed -gt 0) { exit 1 }
