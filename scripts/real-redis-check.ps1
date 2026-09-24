<#
  DBMind real-Redis check: runs the one #[ignore]d test that talks to a REAL Redis
  (crates/dbmind-core/tests/real_redis.rs) -- not the stub the host layer uses.

  Why it is separate from scripts/host-tests.ps1:
    The host layer is deterministic and self-contained (it starts its own stubs).
    This one touches a service that does NOT belong to us, so it is opt-in and
    never runs as part of another layer.

  WHAT IT DOES TO THE TARGET INSTANCE -- read this before pointing it anywhere:
    * It issues `DEBUG SLEEP 2`. Redis is single threaded, so the WHOLE INSTANCE
      freezes for about 2 seconds. That is the only intrusive part.
    * It writes NO keys: the test compares DBSIZE before/after to prove it.
    Do not point this at a shared instance.

  Usage:
    powershell -ExecutionPolicy Bypass -File .\scripts\real-redis-check.ps1
    powershell -ExecutionPolicy Bypass -File .\scripts\real-redis-check.ps1 -Port 6380

  NOTE: this file is intentionally ASCII-only (see scripts/smoke.ps1) -- which is
  also why the target is a separate integration test target: the whole selection
  stays ASCII (`--test real_redis`), no Chinese filter word on the command line.
#>

param(
    [string]$RedisHost = '127.0.0.1',
    [int]$Port = 6379
)

$ErrorActionPreference = 'Continue'

$root = Split-Path -Parent $PSScriptRoot

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
Write-Host "=== real-Redis check: $RedisHost`:$Port ===" -ForegroundColor Cyan
Write-Host "  WARNING: this issues DEBUG SLEEP 2 on that instance." -ForegroundColor Yellow
Write-Host "  Redis is single threaded, so the whole instance freezes for ~2s." -ForegroundColor Yellow
Write-Host "  No keys are written (the test compares DBSIZE before/after)." -ForegroundColor Yellow
Write-Host ""

# --- preconditions --------------------------------------------------------

# The agent hosts are built for Java 17; an older JVM on PATH exits with
# UnsupportedClassVersionError, which shows up later as a puzzling failure.
$java = $env:DBMIND_JAVA
if (-not $java -and $env:JAVA_HOME) {
    $java = Join-Path $env:JAVA_HOME 'bin\java.exe'
}
if (-not $java) {
    $cmd = Get-Command java -ErrorAction SilentlyContinue
    if ($cmd) { $java = $cmd.Source }
}
Check 'java runtime is available' ($null -ne $java -and (Test-Path $java)) 'set DBMIND_JAVA or JAVA_HOME, or put java on PATH'
if ($java -and (Test-Path $java)) {
    $versionText = (& $java -version 2>&1 | Out-String)
    $major = 0
    if ($versionText -match 'version "(\d+)') { $major = [int]$Matches[1] }
    Check "java is >= 17 (agents are built for 17)" ($major -ge 17) ("resolved to $java -- " + ($versionText -replace "`r?`n", ' / '))
    $env:DBMIND_JAVA = $java
    # Path separator differs on Windows (;) vs Linux (:), and this script also runs
    # in CI on ubuntu-latest -- so ask the platform instead of hardcoding it.
    $env:PATH = (Split-Path -Parent $java) + [IO.Path]::PathSeparator + $env:PATH
}

$agents = @(Get-ChildItem (Join-Path $root 'agents') -Recurse -File -Filter 'dbmind-agent-*.jar' -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -notlike 'original-*' })
Check 'agent hosts are built' ($agents.Count -gt 0) "no agents/*.jar -- run 'powershell -File .\scripts\build-agents.ps1' first"

if ($script:failed -gt 0) {
    Write-Host ""
    Write-Host "  cannot continue: fix the preconditions above" -ForegroundColor Red
    exit 1
}

# --- the check itself ------------------------------------------------------

$env:DBMIND_TEST_REAL_REDIS = '1'
$env:DBMIND_TEST_REAL_REDIS_PORT = "$Port"

Write-Host "=== running the real-Redis test ===" -ForegroundColor Cyan
Push-Location $root
$output = (& cargo test -p dbmind-core --test real_redis -- --ignored --nocapture 2>&1 | Out-String)
$exit = $LASTEXITCODE
Pop-Location

$lines = ($output -split "`r?`n")
$lines | Where-Object { $_ -match 'ran|skip|test result:|FAILED|panicked|^error' } | ForEach-Object { Write-Host "  $_" }
if ($exit -ne 0) {
    Write-Host "  --- last 20 lines ---" -ForegroundColor DarkYellow
    $lines | Where-Object { $_.Trim() -ne '' } | Select-Object -Last 20 | ForEach-Object { Write-Host "  $_" }
}

# A skipped test must NOT look like a pass: the test prints [SKIP] when the env
# gate is off, so its absence (plus the explicit result line) is what we assert.
$skipped = $output -match '\[SKIP\]'
Check 'the test actually ran (not skipped)' (-not $skipped) 'the env gate did not reach the test process'
Check 'real-Redis test passed' ($exit -eq 0 -and -not $skipped) "cargo test exited with $exit -- is a real Redis really listening on $RedisHost`:$Port?"

# The Redis agent host is a JVM; a leftover one would keep holding memory.
# Windows-only: on Linux this process-name matching also sees JVMs that belong to
# the machine (CI runners have their own), which would be a false failure.
if ($IsWindows -or $env:OS -eq 'Windows_NT') {
    Start-Sleep -Seconds 1
    $orphans = @(Get-Process java -ErrorAction SilentlyContinue)
    Check 'no orphan agent JVMs left behind' ($orphans.Count -eq 0) ("found " + $orphans.Count + " java process(es)")
}
else {
    Write-Host "  [note] orphan-JVM check skipped on this platform (it would also see unrelated JVMs)" -ForegroundColor DarkYellow
}

Write-Host ""
Write-Host ("passed: {0}   failed: {1}" -f $script:passed, $script:failed) -ForegroundColor $(if ($script:failed -eq 0) { 'Green' } else { 'Red' })
if ($script:failed -gt 0) { exit 1 }
exit 0
