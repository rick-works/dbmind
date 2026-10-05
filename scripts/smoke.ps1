<#
  DBmind end-to-end smoke test (CLI full path).

  NOTE: this file is intentionally ASCII-only. Windows PowerShell 5.1 reads
  script files using the ANSI code page unless a BOM is present, so non-ASCII
  text here would corrupt parsing. Keep it ASCII.

  Covers:
    1. connection add / list / test
    2. create table, insert, select, schema browsing
    3. safety gates: read-only connection blocks writes, error codes, exit codes
    4. history recording (including failures)

  Runs entirely under a temporary DBMIND_HOME, so your real ~/.dbmind is untouched.

  Usage: powershell -ExecutionPolicy Bypass -File .\scripts\smoke.ps1
#>

$ErrorActionPreference = 'Continue'

$root = Split-Path -Parent $PSScriptRoot
$stamp = Get-Date -Format 'yyyyMMddHHmmss'
$work = Join-Path $env:TEMP "dbmind-smoke-$stamp"
$homeDir = Join-Path $work 'home'
$dataDir = Join-Path $work 'data'
New-Item -ItemType Directory -Path $homeDir, $dataDir -Force | Out-Null
$env:DBMIND_HOME = $homeDir

$exe = Join-Path $root 'target\debug\dbmind.exe'
if (-not (Test-Path $exe)) {
    Write-Host "Cannot find $exe -- run 'cargo build' first." -ForegroundColor Red
    exit 1
}

$script:passed = 0
$script:failed = 0

# PowerShell 5.1 mangles embedded double quotes when handing arguments to a
# native executable, so JSON-ish statements (Mongo commands) need escaping.
function Esc {
    param([string]$Text)
    return $Text -replace '"', '\"'
}

function Invoke-Step {
    param(
        [string]$Title,
        [string[]]$CliArgs,
        [int]$ExpectExit = 0
    )
    Write-Host ""
    Write-Host "--- $Title ---" -ForegroundColor Cyan
    & $exe @CliArgs
    $code = $LASTEXITCODE
    if ($code -eq $ExpectExit) {
        Write-Host "  [PASS] exit=$code" -ForegroundColor Green
        $script:passed++
    }
    else {
        Write-Host "  [FAIL] expected exit=$ExpectExit, got $code" -ForegroundColor Red
        $script:failed++
    }
}

$db = Join-Path $dataDir 'demo.db'

Invoke-Step 'conn add (sqlite)' @('conn', 'add', '--name', 'demo', '--kind', 'sqlite', '--file', $db)
Invoke-Step 'conn list' @('conn', 'list')
Invoke-Step 'test connection' @('test', '--conn', 'demo')
Invoke-Step 'create table' @('query', '--conn', 'demo', '--sql', 'create table t(id integer primary key, name text)')
Invoke-Step 'insert rows' @('query', '--conn', 'demo', '--sql', "insert into t (id, name) values (1,'a'),(2,'b'),(3,'c')")
Invoke-Step 'select rows' @('query', '--conn', 'demo', '--sql', 'select id, name from t order by id')
Invoke-Step 'list tables' @('tables', '--conn', 'demo')
Invoke-Step 'list columns' @('columns', '--conn', 'demo', '--table', 't')
Invoke-Step 'json output' @('query', '--conn', 'demo', '--sql', 'select count(*) as n from t', '--json')
Invoke-Step 'type catalog' @('types')
Invoke-Step 'runtime summary' @('status')

# Safety gate: a read-only connection must block writes
Invoke-Step 'mark connection read-only' @('conn', 'read-only', 'demo', '--value', 'true')
Invoke-Step 'DROP on read-only connection (expect exit=4)' @('query', '--conn', 'demo', '--sql', 'drop table t') -ExpectExit 4
Invoke-Step 'select still allowed' @('query', '--conn', 'demo', '--sql', 'select count(*) as n from t')

# Multi-statement must be rejected
Invoke-Step 'multi statement rejected (expect exit=3)' @('query', '--conn', 'demo', '--sql', 'select 1; select 2') -ExpectExit 3

# Stable error code for unknown connection
Invoke-Step 'unknown connection (expect exit=2)' @('query', '--conn', 'ghost', '--sql', 'select 1') -ExpectExit 2

Invoke-Step 'history' @('history', '--limit', '5')

# --- optional: agent runtime (JDBC) -------------------------------------------
# 需要 Java + 网络（下载驱动）。默认跳过，用 DBMIND_SMOKE_AGENT=1 打开。
# 这条链路刻意用「两个独立进程」验证持久化：嵌入式引擎最容易在这里出错
# （先回执后落盘、父目录不存在时静默给空库，都踩过）。
if ($env:DBMIND_SMOKE_AGENT -eq '1') {
    Write-Host ""
    Write-Host "--- agent runtime (H2) ---" -ForegroundColor Magenta
    $h2File = Join-Path $dataDir 'agent-h2\deep\demo'
    Invoke-Step 'agent: driver fetch h2' @('driver', 'fetch', 'h2')
    Invoke-Step 'agent: add H2 connection' @('conn', 'add', '--name', 'h2smoke', '--kind', 'h2', '--file', $h2File)
    Invoke-Step 'agent: test (spawns JVM)' @('test', '--conn', 'h2smoke')
    Invoke-Step 'agent: create table' @('query', '--conn', 'h2smoke', '--sql', 'create table t(a int, b varchar(10))')
    Invoke-Step 'agent: insert' @('query', '--conn', 'h2smoke', '--sql', "insert into t values (1,'x'),(2,null)")
    # 新进程读回：证明落盘（这条曾经失败过）
    Invoke-Step 'agent: select across processes' @('query', '--conn', 'h2smoke', '--sql', 'select count(*) as n from t')
    Invoke-Step 'agent: list tables' @('tables', '--conn', 'h2smoke')
    Invoke-Step 'agent: describe table' @('columns', '--conn', 'h2smoke', '--table', 'T')

    # DuckDB 单独开关：驱动约 70MB。
    #
    # 注意：冒烟用的是**临时 DBMIND_HOME**，所以驱动商店每次都是空的
    # ⇒ 打开这个开关就会**每次重新下载 ~70MB**，看起来像卡死。
    # 因此这里先明确告诉用户要等多久，而不是静默下载。
    if ($env:DBMIND_SMOKE_DUCKDB -eq '1') {
        Write-Host ""
        Write-Host "  [NOTE] duckdb: 本次会下载约 70MB 驱动（临时 HOME 每次都要重下），可能需要 1-3 分钟" -ForegroundColor DarkYellow
        Write-Host "         只想快速回归时不要带 DBMIND_SMOKE_DUCKDB=1" -ForegroundColor DarkYellow

        Write-Host ""
        Write-Host "--- file-based engine (DuckDB) ---" -ForegroundColor Magenta
        $duckFile = Join-Path $dataDir 'agent-duck\analytics.duckdb'
        Invoke-Step 'duckdb: driver fetch' @('driver', 'fetch', 'duckdb')
        Invoke-Step 'duckdb: add connection' @('conn', 'add', '--name', 'ducksmoke', '--kind', 'duckdb', '--file', $duckFile)
        Invoke-Step 'duckdb: test' @('test', '--conn', 'ducksmoke')
        Invoke-Step 'duckdb: create table' @('query', '--conn', 'ducksmoke', '--sql', 'create table sales(region varchar, amount integer)')
        Invoke-Step 'duckdb: insert' @('query', '--conn', 'ducksmoke', '--sql', "insert into sales values ('north', 10), ('south', 20)")
        Invoke-Step 'duckdb: aggregate across processes' @('query', '--conn', 'ducksmoke', '--sql', 'select sum(amount) as total from sales')
        # 表清单曾经因为 TABLE_TYPE 精确匹配（DuckDB 报 BASE TABLE）而返回空，这条是回归护栏
        Invoke-Step 'duckdb: list tables' @('tables', '--conn', 'ducksmoke')
        Invoke-Step 'duckdb: describe table' @('columns', '--conn', 'ducksmoke', '--table', 'sales')
    }

    # MongoDB：专属协议宿主。需要先起 deploy/mongo-test-server（或本机 Mongo），
    # 并用 DBMIND_SMOKE_MONGO_PORT 指定端口。
    if ($env:DBMIND_SMOKE_MONGO_PORT) {
        Write-Host ""
        Write-Host "--- specialised host (MongoDB) ---" -ForegroundColor Magenta
        $mongoPort = $env:DBMIND_SMOKE_MONGO_PORT
        Invoke-Step 'mongo: add connection' @('conn', 'add', '--name', 'mongosmoke', '--kind', 'mongodb', '--host', '127.0.0.1', '--port', $mongoPort, '--database', 'smoke')
        Invoke-Step 'mongo: test (spawns Mongo host)' @('test', '--conn', 'mongosmoke')
        Invoke-Step 'mongo: insertMany' @('query', '--conn', 'mongosmoke', '--sql', (Esc 'db.users.insertMany([{ name: "a", age: 1 }, { name: "b", age: 2 }])'))
        Invoke-Step 'mongo: find with filter' @('query', '--conn', 'mongosmoke', '--sql', (Esc 'db.users.find({ age: { $gt: 1 } })'))
        # count 的响应本身就是一行数据（曾经被当成写命令显示成「0 行」）
        Invoke-Step 'mongo: count returns a row' @('query', '--conn', 'mongosmoke', '--sql', 'db.users.countDocuments({})')
        Invoke-Step 'mongo: aggregate with group' @('query', '--conn', 'mongosmoke', '--sql', (Esc 'db.users.aggregate([{ $group: { _id: "$age", n: { $sum: 1 } } }])'))
        Invoke-Step 'mongo: list collections' @('tables', '--conn', 'mongosmoke')
        Invoke-Step 'mongo: sample fields' @('columns', '--conn', 'mongosmoke', '--table', 'users')
        # 非 SQL 协议同样受闸门约束：写命令在只读连接上必须被拦
        Invoke-Step 'mongo: mark read-only' @('conn', 'read-only', 'mongosmoke', '--value', 'true')
        Invoke-Step 'mongo: write blocked by gate (expect exit=4)' @('query', '--conn', 'mongosmoke', '--sql', 'db.users.deleteMany({})') -ExpectExit 4
        Invoke-Step 'mongo: read still allowed' @('query', '--conn', 'mongosmoke', '--sql', (Esc 'db.users.find({}).limit(1)'))
    }

    # Redis：需要一个真实的 redis-server（用 DBMIND_SMOKE_REDIS_PORT 指定端口）
    if ($env:DBMIND_SMOKE_REDIS_PORT) {
        Write-Host ""
        Write-Host "--- specialised host (Redis) ---" -ForegroundColor Magenta
        $redisPort = $env:DBMIND_SMOKE_REDIS_PORT
        Invoke-Step 'redis: add connection' @('conn', 'add', '--name', 'redissmoke', '--kind', 'redis', '--host', '127.0.0.1', '--port', $redisPort, '--database', '0')
        Invoke-Step 'redis: test (spawns Redis host)' @('test', '--conn', 'redissmoke')
        Invoke-Step 'redis: SET' @('query', '--conn', 'redissmoke', '--sql', 'SET smoke:1 hello')
        Invoke-Step 'redis: GET reads text' @('query', '--conn', 'redissmoke', '--sql', 'GET smoke:1')
        Invoke-Step 'redis: HSET' @('query', '--conn', 'redissmoke', '--sql', 'HSET smoke:h f1 v1')
        Invoke-Step 'redis: HGETALL' @('query', '--conn', 'redissmoke', '--sql', 'HGETALL smoke:h')
        Invoke-Step 'redis: list databases' @('tables', '--conn', 'redissmoke')
        Invoke-Step 'redis: list keys' @('columns', '--conn', 'redissmoke', '--table', 'db0')
        # 流式/阻塞命令必须明确拒绝，而不是挂住
        # 退出码 3 = DBMIND-QUERY-0001（命令被拒），与「多条语句」同一档
        Invoke-Step 'redis: streaming rejected (expect exit=3)' @('query', '--conn', 'redissmoke', '--sql', 'SUBSCRIBE news') -ExpectExit 3
        Invoke-Step 'redis: mark read-only' @('conn', 'read-only', 'redissmoke', '--value', 'true')
        Invoke-Step 'redis: write blocked by gate (expect exit=4)' @('query', '--conn', 'redissmoke', '--sql', 'SET k v') -ExpectExit 4
        Invoke-Step 'redis: read still allowed' @('query', '--conn', 'redissmoke', '--sql', 'GET smoke:1')
        Invoke-Step 'redis: allow writes again' @('conn', 'read-only', 'redissmoke', '--value', 'false')
        Invoke-Step 'redis: cleanup keys' @('query', '--conn', 'redissmoke', '--sql', 'DEL smoke:1')
    }

    # Elasticsearch：契约级替身（deploy/es-test-server）或真实集群
    if ($env:DBMIND_SMOKE_ES_PORT) {
        Write-Host ""
        Write-Host "--- specialised host (Elasticsearch) ---" -ForegroundColor Magenta
        $esPort = $env:DBMIND_SMOKE_ES_PORT
        Invoke-Step 'es: add connection' @('conn', 'add', '--name', 'essmoke', '--kind', 'elasticsearch', '--host', '127.0.0.1', '--port', $esPort)
        Invoke-Step 'es: test' @('test', '--conn', 'essmoke')
        Invoke-Step 'es: _cat indices' @('query', '--conn', 'essmoke', '--sql', 'GET /_cat/indices?format=json')
        Invoke-Step 'es: search' @('query', '--conn', 'essmoke', '--sql', 'GET /logs/_search')
        # 路径与请求体同一行是最常见的写法
        Invoke-Step 'es: search with inline body' @('query', '--conn', 'essmoke', '--sql', (Esc 'POST /logs/_search { "query": { "match_all": {} } }'))
        Invoke-Step 'es: list indices' @('tables', '--conn', 'essmoke')
        Invoke-Step 'es: mapping fields' @('columns', '--conn', 'essmoke', '--table', 'logs')
        Invoke-Step 'es: write document' @('query', '--conn', 'essmoke', '--sql', (Esc 'POST /logs/_doc { "level": "info" }'))
        Invoke-Step 'es: mark read-only' @('conn', 'read-only', 'essmoke', '--value', 'true')
        Invoke-Step 'es: PUT index blocked (expect exit=4)' @('query', '--conn', 'essmoke', '--sql', 'PUT /logs') -ExpectExit 4
        Invoke-Step 'es: search still allowed' @('query', '--conn', 'essmoke', '--sql', 'GET /logs/_search')
        Invoke-Step 'es: server error keeps type (expect exit=1)' @('query', '--conn', 'essmoke', '--sql', 'GET /logs/_search?error=true') -ExpectExit 1
    }
}
else {
    Write-Host ""
    Write-Host "  [SKIP] agent runtime: set DBMIND_SMOKE_AGENT=1 (needs Java + network) to run it" -ForegroundColor DarkYellow
}

Write-Host ""
Write-Host "================ SMOKE RESULT ================" -ForegroundColor Yellow
Write-Host "  passed: $script:passed   failed: $script:failed"
Write-Host "  temp home: $work"
Write-Host "=============================================="

if ($script:failed -gt 0) { exit 1 }
