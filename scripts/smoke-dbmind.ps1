# 冒烟验证：上游兼容层端到端跑一遍（SQLite 真库，无需任何外部服务）。
#
# 为什么要有它：这一层的绝大多数缺陷都是「形状对不上」——字段名差一个字母、
# 行是数组而不是对象、某个值该是字符串却给了 null。**编译与类型检查都抓不到**，
# 只有真的发一遍请求、看一眼返回体才能发现（历史上就有「每行都显示 NULL」这种静默错）。
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File scripts/smoke-dbmind.ps1
#   powershell -ExecutionPolicy Bypass -File scripts/smoke-dbmind.ps1 -Port 20361 -KeepAlive
#
# 前提：已构建后端（cargo build -p dbmind-web）。脚本自己起服务、用临时数据目录，
# 跑完关掉进程并清理临时目录，不碰你 `~/.dbmind` 里的真实配置。
#
# 两处 PowerShell 5.1 的坑（踩过，写在这里免得后人再踩）：
#   1. `ConvertFrom-Json` 解析 JSON 数组时**不展开**（整个数组是一个对象），
#      于是 `@(Get-Json ...)` 得到的长度恒为 1；所以统一走 `Invoke-RestMethod`，它不这样。
#   2. 原生命令（curl.exe 之类）往 stderr 写东西时，在 `$ErrorActionPreference='Stop'`
#      下会被当成异常抛出去 —— 表现为「某个断言莫名失败」。不用原生命令就没这问题。

param(
    [int]$Port = 20361,
    [switch]$KeepAlive
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$script:Base = "http://127.0.0.1:$Port"
$script:Passed = 0
$script:Failed = 0

function Check {
    # 参数取名 $Condition（**不能叫 $Ok**）：PowerShell 变量名不区分大小写，
    # 局部变量 `$ok` 与参数 `$Ok` 就是同一个变量 —— `$ok = $false` 会把参数自己清成 false，
    # 结果是「每一项断言都失败、而明细里打印的却是正确的值」。这个坑值得单独记一笔。
    param([string]$Name, $Condition, [string]$Detail = '')
    $passed = $false
    if ($Condition -is [bool]) { $passed = $Condition }
    elseif ($Condition -is [array]) { $passed = ($Condition.Count -gt 0 -and $Condition[0] -eq $true) }
    elseif ($null -ne $Condition) { $passed = [bool]$Condition }
    if ($passed) {
        $script:Passed++
        Write-Host ("  [OK]   " + $Name) -ForegroundColor Green
    } else {
        $script:Failed++
        Write-Host ("  [FAIL] " + $Name + $(if ($Detail) { "  -> $Detail" } else { '' })) -ForegroundColor Red
    }
}

function Api-Get {
    param([string]$Path)
    return Invoke-RestMethod -Uri "$script:Base$Path" -Method Get -TimeoutSec 60
}

function Api-Post {
    param([string]$Path, $Body)
    $json = if ($Body -is [string]) { $Body } else { $Body | ConvertTo-Json -Depth 10 -Compress }
    # 显式转 UTF-8 字节：不这样中文会被按本地代码页编码，到后端就是乱码
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
    return Invoke-RestMethod -Uri "$script:Base$Path" -Method Post -ContentType 'application/json; charset=utf-8' -Body $bytes -TimeoutSec 180
}

function Api-Raw {
    param([string]$Path, [string]$Method = 'GET', $Body = $null)
    try {
        if ($Method -eq 'GET' -or $null -eq $Body) {
            # 不带请求体：DELETE 之类带上 "null" 这种字符串会让某些实现把它当成非法 JSON
            Invoke-RestMethod -Uri "$script:Base$Path" -Method $Method -TimeoutSec 60 | Out-Null
        } else {
            $json = if ($Body -is [string]) { $Body } else { $Body | ConvertTo-Json -Depth 10 -Compress }
            $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
            Invoke-RestMethod -Uri "$script:Base$Path" -Method $Method -ContentType 'application/json; charset=utf-8' -Body $bytes -TimeoutSec 180 | Out-Null
        }
        return 200
    } catch {
        $response = $_.Exception.Response
        if ($response) { return [int]$response.StatusCode }
        return -1
    }
}

function Api-List {
    param([string]$Path)
    $result = Api-Get $Path
    if ($null -eq $result) { return @() }
    return @($result)
}

# 读错误响应的**原始文本**。
#
# 为什么不用那些「更正统」的办法：`Invoke-RestMethod` 的 catch 里拿 Response 流常常是空的
# （它已经把响应读过一遍），`HttpWebRequest` 在这台机器上也拿不到 —— 而「错误体里到底写了什么」
# 恰恰是最该验的一条。于是用 curl：请求体走临时文件（内联 JSON 会被 PowerShell 的引号规则搞坏），
# `2>$null` 把 curl 自己的进度/告警丢掉（否则在 $ErrorActionPreference='Stop' 下会被当成异常）。
function Api-ErrorBody {
    param([string]$Path, $Body)
    $json = if ($Body -is [string]) { $Body } else { $Body | ConvertTo-Json -Depth 10 -Compress }
    $tmp = [System.IO.Path]::GetTempFileName()
    [System.IO.File]::WriteAllText($tmp, $json, (New-Object System.Text.UTF8Encoding($false)))
    $text = ''
    try {
        $text = (& curl.exe -s -X POST -H "Content-Type: application/json" --data-binary "@$tmp" "$script:Base$Path" 2>$null | Out-String).Trim()
    } catch {
        $text = ''
    }
    Remove-Item $tmp -Force -ErrorAction SilentlyContinue
    return $text
}

# ---------------------------------------------------------------- 起服务

$exe = @(
    (Join-Path $root 'target\release\dbmind-web.exe'),
    (Join-Path $root 'target\debug\dbmind-web.exe')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $exe) {
    Write-Host '未找到 dbmind-web.exe，请先执行： cargo build -p dbmind-web' -ForegroundColor Yellow
    exit 1
}

$workHome = Join-Path $env:TEMP ("dbmind-next-smoke-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force -Path $workHome | Out-Null
$sqliteFile = Join-Path $workHome 'smoke.db'

$env:DBMIND_HOME = $workHome
$proc = Start-Process -FilePath $exe -ArgumentList '--port', "$Port", '--dist', (Join-Path $root 'frontend\dist') -PassThru -WindowStyle Hidden

Write-Host "DBMind 冒烟：端口 $Port，数据目录 $workHome" -ForegroundColor Cyan
$ready = $false
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 250
    try { if ((Api-Get '/api/dbmind/health').ok -eq $true) { $ready = $true; break } } catch { }
}
if (-not $ready) {
    Write-Host '服务未能在 10 秒内就绪（端口被占用？）' -ForegroundColor Red
    if (-not $KeepAlive) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
    exit 1
}

try {
    # ------------------------------------------------------------ 契约与系统端点
    Write-Host '契约与系统端点' -ForegroundColor White
    Check '内核原生契约仍在（/api/dbmind/health）' ((Api-Get '/api/dbmind/health').ok -eq $true)
    Check '前端产物已托管（GET / => 200）' ((Api-Raw '/') -eq 200)
    Check '授权状态恒为已授权' ((Api-Get '/api/license/status').licensed -eq $true)

    $types = @(Api-List '/api/drivers/types')
    Check '驱动类型 16 个（前端注册表覆盖的集合）' ($types.Count -eq 16) "实际 $($types.Count)"
    Check '类型码是大写（前端按大写索引）' (@($types | Where-Object { $_.code -cne $_.code.ToUpper() }).Count -eq 0)
    Check '不含前端没有的类型 DUCKDB（不通告就用不了的东西）' (@($types | Where-Object { $_.code -eq 'DUCKDB' }).Count -eq 0)

    $driverStatus = Api-Get '/api/drivers/status'
    Check '驱动状态按大写类型码作键' ($null -ne $driverStatus.SQLITE)
    Check 'SQLITE 就绪（内核自带，无需下载）' ($driverStatus.SQLITE.ready -eq $true)
    Check '状态表与类型表一一对应' (@($driverStatus.PSObject.Properties).Count -eq $types.Count)

    Check '设置：存储路径可读' ((Api-Get '/api/settings/paths').dataDir.Length -gt 0)
    Check '设置：旧版 TLS 开关可读' ($null -ne (Api-Get '/api/settings/legacy-tls'))

    # ------------------------------------------------------------ 连接管理
    Write-Host '连接管理' -ForegroundColor White
    $created = Api-Post '/api/connections' @{
        name = '冒烟-本地文件'; type = 'SQLITE'; filePath = $sqliteFile
        environment = 'TEST'; env = 'TEST'
    }
    $cid = $created.id
    Check '新建连接返回 id' (-not [string]::IsNullOrEmpty($cid))
    Check '连接类型回显为大写 SQLITE' ($created.type -eq 'SQLITE')
    Check '目录 / 环境角标已保存（树按它分组）' ($created.environment -eq 'TEST' -and $created.env -eq 'TEST')
    Check '默认目录不为空串（空串会让连接在树上不显示）' ((Api-Post '/api/connections' @{ name = '冒烟-无目录'; type = 'SQLITE'; filePath = (Join-Path $workHome 'b.db') }).environment.Length -gt 0)

    $list = @(Api-List '/api/connections')
    Check '连接列表含新连接' (@($list | Where-Object { $_.id -eq $cid }).Count -eq 1) "列表 $($list.Count) 条"
    Check '详情接口不回传口令' ((Api-Get "/api/connections/$cid").PSObject.Properties.Name -notcontains 'password')

    $copied = Api-Post "/api/connections/$cid/copy?name=smoke-copy" '{}'
    Check '复制连接（服务端复制，口令一并带过去）' ($copied.name -eq 'smoke-copy') $copied.name
    Check '复制的连接可删除' ((Api-Raw "/api/connections/$($copied.id)" 'DELETE') -eq 200)

    $tested = Api-Post '/api/sqlite/test' @{ id = $cid; type = 'SQLITE'; filePath = $sqliteFile }
    Check '按 id 测试连接成功' ($tested.success -eq $true) $tested.message

    # ------------------------------------------------------------ 元数据与查询
    Write-Host '元数据与查询' -ForegroundColor White
    Check '库清单（一个连接一个库）' (@(Api-List "/api/sqlite/$cid/databases").Count -eq 1)

    Api-Post "/api/sqlite/query/$cid" @{ sql = 'create table smoke_t (id integer primary key, name text, score real)' } | Out-Null
    Api-Post "/api/sqlite/query/$cid" @{ sql = "insert into smoke_t (id, name, score) values (1,'alice',1.5),(2,'bob',2.5),(3,'carol',3.5)" } | Out-Null

    $tables = @(Api-List "/api/sqlite/$cid/tables?database=main")
    Check '表清单含 smoke_t' (@($tables | Where-Object { $_.name -eq 'smoke_t' }).Count -eq 1) "表 $($tables.name -join ',')"
    Check '表带 type 字段（树按 VIEW 分「视图」分类）' (($tables | Where-Object { $_.name -eq 'smoke_t' }).type -eq 'TABLE')

    $cols = @(Api-List "/api/sqlite/$cid/columns?database=main&table=smoke_t")
    Check '列清单 3 列' ($cols.Count -eq 3) "实际 $($cols.Count)"
    $idCol = $cols | Where-Object { $_.name -eq 'id' }
    Check '主键已标注（就地编辑要靠它定位行）' ($idCol.primaryKey -eq $true)
    Check '列带 type / nullable / key' (($idCol.type.Length -gt 0) -and ($null -ne $idCol.nullable) -and ($idCol.key -eq 'PRI'))

    $ddl = Api-Get "/api/sqlite/$cid/ddl?database=main&table=smoke_t"
    Check '建表语句读 res.ddl' ($ddl.ddl -match 'CREATE TABLE') $ddl.ddl

    $rows = Api-Get "/api/sqlite/$cid/data?database=main&table=smoke_t&page=1&size=2"
    Check '分页：本页 2 行' ($rows.rowCount -eq 2) "rowCount=$($rows.rowCount)"
    Check '分页：真实总数 3（分页控件要用）' ($rows.totalCount -eq 3) "totalCount=$($rows.totalCount)"
    Check '耗时字段名是 executeTime（不是 durationMs）' (@($rows.PSObject.Properties.Name) -contains 'executeTime')
    Check '行是「列名→值」的对象（不是数组）' ($null -ne $rows.rows[0].name) ($rows.rows[0] | ConvertTo-Json -Compress)
    Check '列类型随结果返回' (@($rows.columnTypes).Count -eq 3)

    $sized = Api-Get "/api/sqlite/$cid/data?database=main&table=smoke_t&size=10&orderColumn=id&orderDir=desc"
    Check '排序参数生效（第一行 id=3）' ($sized.rows[0].id -eq 3)

    $kw = Api-Get "/api/sqlite/$cid/data?database=main&table=smoke_t&keyword=alice"
    Check '关键字模糊筛选生效（1 行）' ($kw.rowCount -eq 1) "rowCount=$($kw.rowCount)"

    $batch = Api-Post "/api/sqlite/query/$cid/batch" @{ sql = 'select 1 as a; select 2 as b;' }
    Check '多段执行返回多个结果' (@($batch.results).Count -eq 2) "实际 $(@($batch.results).Count)"
    Check '多段整体成功' ($batch.success -eq $true)

    $counts = Api-Get "/api/sqlite/$cid/table-count?database=main&tables=smoke_t"
    Check '批量真实行数回填' ($counts.smoke_t -eq 3) ($counts | ConvertTo-Json -Compress)

    $search = Api-Get "/api/sqlite/$cid/search-objects?database=main&keyword=name"
    Check '对象搜索：列名命中（columns[].column）' (@($search.columns | Where-Object { $_.column -eq 'name' }).Count -ge 1)
    $search2 = Api-Get "/api/sqlite/$cid/search-objects?database=main&keyword=smoke"
    Check '对象搜索：表名命中（tables[] 是字符串数组）' (@($search2.tables) -contains 'smoke_t')

    Check '索引清单可读（SQLite 此时为空数组）' ((Api-Raw "/api/sqlite/$cid/indexes?database=main") -eq 200)
    Check 'SQLite 的存储过程是「确实没有」→ 200 + []' ((Api-Raw "/api/sqlite/$cid/procedures?database=main") -eq 200)
    Check 'SQLite 的触发器是「确实没有」→ 200 + []' ((Api-Raw "/api/sqlite/$cid/triggers?database=main") -eq 200)
    Check 'SQLite 的事件是「确实没有」→ 200 + []' ((Api-Raw "/api/sqlite/$cid/events?database=main") -eq 200)

    $monitor = Api-Get "/api/sqlite/$cid/monitor?database=main"
    Check '监控未接入时 supported=false（界面据此隐藏，而不是显示一堆 0）' ($monitor.supported -eq $false)

    # ------------------------------------------------------------ 就地编辑与表操作
    Write-Host '就地编辑与表操作' -ForegroundColor White
    $insert = Api-Post "/api/sqlite/$cid/data-save" @{
        database = 'main'; table = 'smoke_t'; pkColumns = @('id')
        inserts  = @(@{ id = 4; name = 'dave'; score = 4.5 })
        updates  = @(); deletes = @()
    }
    Check '新增一行' ($insert.success -eq $true -and $insert.affectedRows -eq 1) $insert.message

    $update = Api-Post "/api/sqlite/$cid/data-save" @{
        database = 'main'; table = 'smoke_t'; pkColumns = @('id')
        inserts  = @()
        updates  = @(@{ original = @{ id = 4; name = 'dave'; score = 4.5 }; row = @{ id = 4; name = 'dave2'; score = 4.5 } })
        deletes  = @()
    }
    Check '按主键改一行' ($update.success -eq $true -and $update.affectedRows -eq 1) $update.message
    $check = Api-Get "/api/sqlite/$cid/data?database=main&table=smoke_t&size=10&orderColumn=id"
    Check '改动已写回（dave2）' ($check.rows[3].name -eq 'dave2')

    $delete = Api-Post "/api/sqlite/$cid/data-save" @{
        database = 'main'; table = 'smoke_t'; pkColumns = @('id')
        inserts  = @(); updates = @()
        deletes  = @(@{ id = 4 })
    }
    Check '删除一行' ($delete.success -eq $true -and $delete.affectedRows -eq 1) $delete.message

    $alter = Api-Post "/api/sqlite/$cid/alter" @{ database = 'main'; sql = 'create index idx_smoke_name on smoke_t(name)' }
    Check '执行结构变更（建索引）' ($alter.success -eq $true) $alter.message
    $idx = @(Api-List "/api/sqlite/$cid/indexes?database=main")
    Check '索引清单能列出刚建的索引' (@($idx | Where-Object { $_.name -eq 'idx_smoke_name' }).Count -eq 1)

    $renamed = Api-Post "/api/sqlite/$cid/table-action" @{ database = 'main'; table = 'smoke_t'; action = 'rename'; newName = 'smoke_t2' }
    Check '重命名表' ($renamed.success -eq $true) $renamed.message
    $afterRename = @(Api-List "/api/sqlite/$cid/tables?database=main")
    Check '表清单已反映新名' (@($afterRename | Where-Object { $_.name -eq 'smoke_t2' }).Count -eq 1)

    $preview = Api-Post "/api/sqlite/$cid/rename-object" @{ database = 'main'; type = 'table'; name = 'smoke_t2'; newName = 'smoke_t3'; previewOnly = $true }
    Check '重命名预览只回语句' ($preview.sql -match 'rename')
    $still = @(Api-List "/api/sqlite/$cid/tables?database=main")
    Check '预览确实没有动库（表名未变）' (@($still | Where-Object { $_.name -eq 'smoke_t2' }).Count -eq 1)

    # ------------------------------------------------------------ 执行 SQL 文件（后台任务）
    Write-Host '执行 SQL 文件（后台任务）' -ForegroundColor White
    $task = Api-Post "/api/sqlite/query/run-file-task/$cid" @{
        database = 'main'
        sql      = "insert into smoke_t2 (id, name) values (10,'x');`ninsert into smoke_t2 (id, name) values (11,'y');"
    }
    Check '任务已受理（不是一次长请求）' (-not [string]::IsNullOrEmpty($task.taskId))
    $snapshot = $null
    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 200
        $snapshot = Api-Get "/api/sqlite/query/run-file-task/status/$($task.taskId)"
        if ($snapshot.status -ne 'running') { break }
    }
    Check '任务跑完（逐句执行 + 进度）' ($snapshot.status -eq 'success') ($snapshot | ConvertTo-Json -Compress)
    Check '任务带逐句日志' (@($snapshot.logs).Count -ge 1)

    # ------------------------------------------------------------ 未接入能力：必须显式报错
    Write-Host '未接入能力（必须显式 501，不能 404 或静默）' -ForegroundColor White
    Check 'AI 对话 ⇒ 501' ((Api-Raw '/api/ai/chat' 'POST' @{ prompt = 'hi' }) -eq 501)
    Check 'AI 流式 ⇒ 501' ((Api-Raw '/api/ai/chat/stream' 'POST' @{ prompt = 'hi' }) -eq 501)
    Check 'AI 知识库 ⇒ 501' ((Api-Raw '/api/ai/kb/list' 'POST' @{}) -eq 501)
    Check '导出任务 ⇒ 501' ((Api-Raw "/api/sqlite/export/task/$cid" 'POST' @{}) -eq 501)
    Check '数据同步 ⇒ 501' ((Api-Raw '/api/sync/db' 'POST' @{}) -eq 501)
    Check '数据对比 ⇒ 501' ((Api-Raw '/api/compare' 'POST' @{}) -eq 501)
    Check '备份 ⇒ 501' ((Api-Raw '/api/backup/start' 'POST' @{}) -eq 501)
    Check '生成测试数据 ⇒ 501' ((Api-Raw '/api/datagen/start' 'POST' @{}) -eq 501)
    # 断言「错误体里有人能读懂的话」，而不是匹配某一句话：措辞会改，形状不该改
    $aiBody = Api-ErrorBody '/api/ai/chat' @{ prompt = 'hi' }
    Check '未接入的响应体是 JSON 且 message 非空' (($aiBody -match '"success"\s*:\s*false') -and ($aiBody -match '"message"\s*:\s*".{8,}"')) ("body=" + $aiBody)
    Check '未知模块前缀 ⇒ 404（不被静默当成别的东西）' ((Api-Raw '/api/notamodule/x/tables') -eq 404)
    # 「这个概念存在、但我们还没写」必须报错，不能返回空 —— 空会被读成「这个库没有」
    $derby = Api-Post '/api/connections' @{ name = '冒烟-derby'; type = 'DERBY'; database = 'smoke' }
    Check '未写方言的 DDL ⇒ 501（不编一个不完整的建表语句骗人）' ((Api-Raw "/api/derby/$($derby.id)/ddl?database=smoke&table=t") -eq 501)
    Api-Raw "/api/connections/$($derby.id)" 'DELETE' | Out-Null

    # ------------------------------------------------------------ 收尾
    Check '删除连接' ((Api-Raw "/api/connections/$cid" 'DELETE') -eq 200)
}
finally {
    if (-not $KeepAlive) {
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 300
        Remove-Item -Recurse -Force $workHome -ErrorAction SilentlyContinue
    }
}

Write-Host ''
Write-Host "通过 $script:Passed 项，失败 $script:Failed 项" -ForegroundColor $(if ($script:Failed -eq 0) { 'Green' } else { 'Red' })
# 机器可读的一行（CI / 终端里中文编码各异，判定只看这一行）
Write-Host "RESULT passed=$script:Passed failed=$script:Failed"
if ($script:Failed -gt 0) { exit 1 }
