<#
  DBMind real-SSH check: runs the one #[ignore]d test that talks to a REAL SSH
  jump host (crates/dbmind-core/tests/real_ssh.rs).

  Why it is separate from scripts/host-tests.ps1:
    The host layer starts its own stubs and needs a JDK. This one talks to a
    machine that does NOT belong to us, and the tunnel is pure Rust -- no JDK,
    no driver jars, no agent host. A failure here points at SSH only.

  WHAT IT DOES TO THE TARGET -- read this before pointing it anywhere:
    * It is READ-ONLY: by default the forward target is the jump host's OWN SSH
      port, and the test just reads the banner to prove bytes went through.
      It writes nothing, changes no config, stores no connection record.
    * It opens a few SSH logins with the account you give it (including one
      intentionally wrong password at the end, to prove auth errors surface).

  Usage (password auth):
    $env:DBMIND_TEST_SSH_HOST = 'jump.example.com'
    $env:DBMIND_TEST_SSH_USER = 'deploy'
    $env:DBMIND_TEST_SSH_PASSWORD = '...'
    powershell -ExecutionPolicy Bypass -File .\scripts\real-ssh-check.ps1

  Usage (key auth, all in one line):
    powershell -ExecutionPolicy Bypass -File .\scripts\real-ssh-check.ps1 `
      -SshHost jump.example.com -SshUser deploy -KeyPath "$HOME\.ssh\id_rsa"

  Forward to something other than the jump host itself (e.g. the database):
    ... -TargetHost 10.1.2.3 -TargetPort 3306

  NOTE: this file is intentionally ASCII-only (see scripts/smoke.ps1) -- which is
  also why the target is a separate integration test target: the whole selection
  stays ASCII (`--test real_ssh`), no Chinese filter word on the command line.
#>

param(
    [string]$SshHost = '',
    [int]$SshPort = 22,
    [string]$SshUser = '',
    [string]$Password = '',
    [string]$KeyPath = '',
    [string]$KeyPassphrase = '',
    [string]$TargetHost = '',
    [int]$TargetPort = 0
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

# --- parameters -> environment --------------------------------------------
# Explicit parameters win, but anything already in the environment is respected,
# so the test can be driven by hand without going through this script at all.
if ($SshHost) { $env:DBMIND_TEST_SSH_HOST = $SshHost }
if ($SshUser) { $env:DBMIND_TEST_SSH_USER = $SshUser }
if ($Password) { $env:DBMIND_TEST_SSH_PASSWORD = $Password }
if ($KeyPath) { $env:DBMIND_TEST_SSH_KEY = $KeyPath }
if ($KeyPassphrase) { $env:DBMIND_TEST_SSH_KEY_PASSPHRASE = $KeyPassphrase }
if ($TargetHost) { $env:DBMIND_TEST_SSH_TARGET_HOST = $TargetHost }
if ($TargetPort -gt 0) { $env:DBMIND_TEST_SSH_TARGET_PORT = "$TargetPort" }
if ($SshPort -ne 22) { $env:DBMIND_TEST_SSH_PORT = "$SshPort" }

Write-Host ""
Write-Host "=== real-SSH check: $($env:DBMIND_TEST_SSH_HOST):$(if ($env:DBMIND_TEST_SSH_PORT) { $env:DBMIND_TEST_SSH_PORT } else { 22 }) ===" -ForegroundColor Cyan
Write-Host "  user: $($env:DBMIND_TEST_SSH_USER)" -ForegroundColor Gray
Write-Host "  auth: $(if ($env:DBMIND_TEST_SSH_KEY) { 'private key' } else { 'password' })" -ForegroundColor Gray
Write-Host "  read-only: it reads a banner from the forward target and writes nothing." -ForegroundColor Yellow
Write-Host ""

# --- preconditions --------------------------------------------------------
Check 'SSH host is set' ([bool]$env:DBMIND_TEST_SSH_HOST) 'pass -SshHost or set DBMIND_TEST_SSH_HOST'
Check 'SSH user is set' ([bool]$env:DBMIND_TEST_SSH_USER) 'pass -SshUser or set DBMIND_TEST_SSH_USER'
Check 'credentials are set (password or private key)' ([bool]$env:DBMIND_TEST_SSH_PASSWORD -or [bool]$env:DBMIND_TEST_SSH_KEY) 'pass -Password or -KeyPath'

if ($env:DBMIND_TEST_SSH_KEY -and -not (Test-Path $env:DBMIND_TEST_SSH_KEY)) {
    Check 'private key file exists' $false "not found: $($env:DBMIND_TEST_SSH_KEY)"
}
else {
    Check 'private key file exists (or password auth)' $true ''
}

# A quick TCP reachability probe: it turns "SSH is not even reachable" into a
# clear precondition failure instead of a puzzling test failure later on.
if ($env:DBMIND_TEST_SSH_HOST) {
    $port = if ($env:DBMIND_TEST_SSH_PORT) { [int]$env:DBMIND_TEST_SSH_PORT } else { 22 }
    $reachable = $false
    try {
        $client = New-Object System.Net.Sockets.TcpClient
        $reachable = $client.ConnectAsync($env:DBMIND_TEST_SSH_HOST, $port).Wait(5000)
        $client.Close()
    }
    catch { $reachable = $false }
    Check "jump host is reachable on tcp/$port" $reachable "cannot open a TCP connection to $($env:DBMIND_TEST_SSH_HOST):$port"
}

if ($script:failed -gt 0) {
    Write-Host ""
    Write-Host "  cannot continue: fix the preconditions above" -ForegroundColor Red
    exit 1
}

# --- the check itself ------------------------------------------------------

$env:DBMIND_TEST_SSH = '1'

Write-Host "=== running the real-SSH test ===" -ForegroundColor Cyan
Push-Location $root
$output = (& cargo test -p dbmind-core --test real_ssh -- --ignored --nocapture 2>&1 | Out-String)
$exit = $LASTEXITCODE
Pop-Location

$lines = ($output -split "`r?`n")
$lines | Where-Object { $_ -match 'SSH|ran|test result:|FAILED|panicked|^error' } | ForEach-Object { Write-Host "  $_" }
if ($exit -ne 0) {
    Write-Host "  --- last 25 lines ---" -ForegroundColor DarkYellow
    $lines | Where-Object { $_.Trim() -ne '' } | Select-Object -Last 25 | ForEach-Object { Write-Host "  $_" }
}

# A skipped test must NOT look like a pass: the test prints [SKIP] when the env
# gate is off, so its absence (plus the explicit result line) is what we assert.
$skipped = $output -match '\[SKIP\]'
Check 'the test actually ran (not skipped)' (-not $skipped) 'the env gate did not reach the test process'
Check 'real-SSH test passed' ($exit -eq 0 -and -not $skipped) "cargo test exited with $exit -- check the jump host address, the account and the credentials"

# No JVM is involved in this path at all (the tunnel is pure Rust), so a leftover
# java process here would mean something else leaked -- report it, do not fail.
$orphans = @(Get-Process java -ErrorAction SilentlyContinue)
if ($orphans.Count -gt 0) {
    Write-Host "  [note] $($orphans.Count) java process(es) running; this check does not use a JVM, so they are unrelated." -ForegroundColor DarkYellow
}

Write-Host ""
Write-Host ("passed: {0}   failed: {1}" -f $script:passed, $script:failed) -ForegroundColor $(if ($script:failed -eq 0) { 'Green' } else { 'Red' })
if ($script:failed -gt 0) { exit 1 }
exit 0
