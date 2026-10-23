<#
  DBMind real-MySQL check: "after the link dies, the FIRST statement must work".

  Why this exists (user report, 2026-10-23):
      Keep the window open, let the VPN drop and come back, then run SQL --
      the first execution always failed once with `Communications link failure`
      ("The last packet successfully received ... 44,564 milliseconds ago"),
      the second one worked.

  Root cause: the pooled physical connection was dead but nobody probed it.
  Interactive tabs are "resident" (no re-handshake when you switch back), and
  the old code excluded every resident lane from the background keepalive --
  so a dead tab connection was never noticed; the pre-borrow probe only looks
  at sessions idle >= 60s, and this one was only 44s idle.

  What it does to the target -- read this before pointing it anywhere:
    * READ-ONLY apart from: it KILLs *its own* session (same account, never
      somebody else's) to simulate the broken link, and it creates one
      TEMPORARY table on purpose AFTER the link is declared dead (that
      statement is expected to fail; no table is actually created).
    * The second test sleeps 25 seconds on purpose (one keepalive round).

  Usage:
    powershell -ExecutionPolicy Bypass -File .\scripts\test\real-mysql-stale.ps1 `
      -MysqlHost 192.168.2.127 -MysqlUser root -MysqlPassword '...' -MysqlDatabase mysql

  NOTE: this file is intentionally ASCII-only (see scripts/test/smoke-cli.ps1).
#>

param(
    [string]$MysqlHost = '',
    [int]$MysqlPort = 0,
    [string]$MysqlUser = '',
    [string]$MysqlPassword = '',
    [string]$MysqlDatabase = ''
)

$ErrorActionPreference = 'Continue'

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)

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

# --- parameters -> environment --------------------------------------------
if ($MysqlHost) { $env:DBMIND_TEST_MYSQL_HOST = $MysqlHost }
if ($MysqlPort -gt 0) { $env:DBMIND_TEST_MYSQL_PORT = "$MysqlPort" }
if ($MysqlUser) { $env:DBMIND_TEST_MYSQL_USER = $MysqlUser }
if ($MysqlPassword) { $env:DBMIND_TEST_MYSQL_PASSWORD = $MysqlPassword }
if ($MysqlDatabase) { $env:DBMIND_TEST_MYSQL_DATABASE = $MysqlDatabase }

# The JDBC agent host needs a Java 17+ runtime. Prefer the bundled JRE (the one
# the installer ships) -- JDK 25 runs most drivers fine but is not what users run.
if (-not $env:DBMIND_JAVA) {
    $candidates = @(
        'D:\app\DBmind\jre\bin\java.exe',
        (Join-Path $env:ProgramFiles 'DBMind\jre\bin\java.exe')
    )
    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) {
            $env:DBMIND_JAVA = $candidate
            break
        }
    }
    if (-not $env:DBMIND_JAVA -and $env:JAVA_HOME) {
        $maybe = Join-Path $env:JAVA_HOME 'bin\java.exe'
        if (Test-Path $maybe) { $env:DBMIND_JAVA = $maybe }
    }
}

$dbHost = if ($env:DBMIND_TEST_MYSQL_HOST) { $env:DBMIND_TEST_MYSQL_HOST } else { '127.0.0.1' }
$dbPort = if ($env:DBMIND_TEST_MYSQL_PORT) { $env:DBMIND_TEST_MYSQL_PORT } else { '3306' }
$dbName = if ($env:DBMIND_TEST_MYSQL_DATABASE) { $env:DBMIND_TEST_MYSQL_DATABASE } else { 'mysql' }
$target = '{0}:{1}/{2}' -f $dbHost, $dbPort, $dbName

Write-Host ""
Write-Host "=== real-MySQL stale-connection check: $target ===" -ForegroundColor Cyan
Write-Host "  user: $(if ($env:DBMIND_TEST_MYSQL_USER) { $env:DBMIND_TEST_MYSQL_USER } else { 'root' })" -ForegroundColor Gray
Write-Host "  java: $(if ($env:DBMIND_JAVA) { $env:DBMIND_JAVA } else { '(not set -- the agent host may fail to start)' })" -ForegroundColor Gray
Write-Host "  step 1: KILL our own session, then ONE select must succeed" -ForegroundColor Yellow
Write-Host "  step 2: KILL an idle tab session, wait 25s, then ONE write must succeed" -ForegroundColor Yellow
Write-Host ""

Check 'mysql host is set' ([bool]$env:DBMIND_TEST_MYSQL_HOST) 'pass -MysqlHost or set DBMIND_TEST_MYSQL_HOST'
Check 'credentials are set' ([bool]$env:DBMIND_TEST_MYSQL_PASSWORD) 'pass -MysqlPassword (this check needs a real account)'
Check 'java runtime is available' ([bool]$env:DBMIND_JAVA) 'set DBMIND_JAVA to a Java 17+ java.exe (the JDBC agent host needs it)'

if ($script:failed -gt 0) {
    Write-Host ""
    Write-Host "  cannot continue: fix the preconditions above" -ForegroundColor Red
    exit 1
}

$env:DBMIND_TEST_REAL_MYSQL = '1'

Write-Host "=== running the real-MySQL tests (about 35 seconds) ===" -ForegroundColor Cyan
Push-Location $root
$output = (& cargo test -p dbmind-core --test real_mysql -- --ignored --nocapture --test-threads=1 2>&1 | Out-String)
$exit = $LASTEXITCODE
Pop-Location

$lines = ($output -split "`r?`n")
$lines | Where-Object { $_ -match 'MySQL|test result:|FAILED|panicked|^error' } | ForEach-Object { Write-Host "  $_" }
if ($exit -ne 0) {
    Write-Host "  --- last 25 lines ---" -ForegroundColor DarkYellow
    $lines | Where-Object { $_.Trim() -ne '' } | Select-Object -Last 25 | ForEach-Object { Write-Host "  $_" }
}

$skipped = $output -match '\[SKIP\]'
Check 'the tests actually ran (not skipped)' (-not $skipped) 'the env gate did not reach the test process'
Check 'real-MySQL tests passed' ($exit -eq 0 -and -not $skipped) "cargo test exited with $exit"

Write-Host ""
Write-Host ("passed: {0}   failed: {1}" -f $script:passed, $script:failed) -ForegroundColor $(if ($script:failed -eq 0) { 'Green' } else { 'Red' })
if ($script:failed -gt 0) { exit 1 }
exit 0
