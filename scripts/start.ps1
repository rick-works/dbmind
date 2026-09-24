# 启动 DBMind（后端 + 前端，单进程），并打开浏览器。
#
#   powershell -ExecutionPolicy Bypass -File scripts\start.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\start.ps1 -Port 20362 -NoBrowser -Log
#
# 为什么要有个脚本：默认端口 20361 是前端 vite 代理认的端口，产物目录是 frontend\dist，
# 这两个参数每次手打容易漏（漏了 --dist 就只剩 API、页面 404，看起来像"打不开"）。
#
# 服务是**后台常驻**的：脚本自己等健康检查通过就退出，所以关掉这个终端窗口不影响它。
# 停止：scripts\stop.ps1

param(
    [int]$Port = 20361,
    [switch]$NoBrowser,
    [switch]$Log,
    [switch]$Release,
    # 显式指定 Java（要用 Java 17+ 才跑得动关系型/NoSQL 的宿主）
    [string]$Java = ''
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
# 注意：别把变量叫 $args 或 $profile —— 它们是 PowerShell 的自动变量，赋值会报错或产生意外
$buildProfile = if ($Release) { 'release' } else { 'debug' }
$exe = Join-Path $root "target\$buildProfile\dbmind-web.exe"

if (-not (Test-Path $exe)) {
    Write-Host "未找到 $exe，正在构建（默认 debug；要更快的运行速度用 -Release 并先 cargo build --release）..." -ForegroundColor Yellow
    Push-Location $root
    try { cargo build -p dbmind-web } finally { Pop-Location }
}
if (-not (Test-Path $exe)) { Write-Host '构建失败，无法启动。' -ForegroundColor Red; exit 1 }

$dist = Join-Path $root 'frontend\dist'
if (-not (Test-Path (Join-Path $dist 'index.html'))) {
    Write-Host "前端产物缺失：$dist\index.html" -ForegroundColor Yellow
    Write-Host '  → 先构建前端： cd frontend; npm install; npm run build' -ForegroundColor Yellow
    Write-Host '  （或者把上游\frontend\dist 整个拷到 frontend\dist）' -ForegroundColor Yellow
    exit 1
}

# ---- 宿主 jar 就位 ----
# 内核找宿主包的顺序是：DBMIND_AGENT_JAR > DBMIND_AGENTS_DIR > **可执行文件旁 agents/** >
# （仅 debug 构建）仓库里的 agents/*/target/*.jar。
# 最后那条只在 debug 生效，所以这里把构建好的 jar 复制到 exe 旁 —— debug/release 都能用，
# 也符合发布产物的布局。少了这一步的表现是：SQLite 正常，其它数据库一律
# 「DBMIND-DRV-0002 驱动宿主未就绪」，而看不出是缺 jar。
$agentJars = @('dbmind-agent-jdbc.jar', 'dbmind-agent-mongodb.jar', 'dbmind-agent-redis.jar', 'dbmind-agent-elasticsearch.jar')
$sidecar = Join-Path $root "target\$buildProfile\agents"
$missing = $agentJars | Where-Object { -not (Test-Path (Join-Path $sidecar $_)) }
if ($missing.Count -gt 0) {
    $built = @()
    foreach ($name in $agentJars) {
        foreach ($dir in (Get-ChildItem (Join-Path $root 'agents') -Directory -ErrorAction SilentlyContinue)) {
            $jar = Join-Path $dir.FullName "target\$name"
            if (Test-Path $jar) { $built += @{ Name = $name; Path = $jar } }
        }
    }
    if ($built.Count -eq 0) {
        Write-Host '宿主 jar 尚未构建：SQLite 可用，其它数据库连接会失败（DBMIND-DRV-0002）。' -ForegroundColor Yellow
        Write-Host '  → powershell -ExecutionPolicy Bypass -File scripts\build-agents.ps1   （需要 JDK 17+ 与 Maven 3.6.3+）' -ForegroundColor Yellow
    } else {
        New-Item -ItemType Directory -Force -Path $sidecar | Out-Null
        foreach ($item in $built) { Copy-Item $item.Path (Join-Path $sidecar $item.Name) -Force }
        Write-Host "已把 $($built.Count) 个宿主 jar 放到 $sidecar"
    }
}

# ---- Java 17+ ----
# 关系型与 NoSQL 的连接都走 Java 宿主。内核按 DBMIND_JAVA > JAVA_HOME > PATH 解析 Java，
# 而 PATH 上常常是过旧的 Java 11 —— 表现是「连不上」，提示只说宿主未就绪，
# 看不出根因是 Java 版本。这里替它挑一个 17+ 的，并用 DBMIND_JAVA 固定下来。
function Get-JavaMajor {
    param([string]$JavaExe)
    if (-not $JavaExe -or -not (Test-Path $JavaExe)) { return 0 }
    $text = ''
    # 经 cmd 合并 stderr：PowerShell 5.1 里原生命令写 stderr 会被当成错误记录
    try { $text = (& cmd.exe /c "`"$JavaExe`" -version 2>&1" | Out-String) } catch { return 0 }
    if ($text -match 'version "(\d+)\.(\d+)') {
        $major = [int]$Matches[1]
        if ($major -eq 1) { return [int]$Matches[2] }   # 1.8.0_xxx
        return $major
    }
    return 0
}

$javaCandidates = @()
if ($Java) { $javaCandidates += $Java }
if ($env:DBMIND_JAVA) { $javaCandidates += $env:DBMIND_JAVA }
foreach ($dir in @($env:JAVA_HOME, 'C:\Program Files\Java', 'C:\Program Files\Eclipse Adoptium', 'C:\Program Files\Microsoft\jdk', 'D:\develop\tools')) {
    if (-not $dir -or -not (Test-Path $dir)) { continue }
    if ($dir -like '*jdk*' -or $dir -like '*jre*') {
        $javaCandidates += (Join-Path $dir 'bin\java.exe')
    } else {
        $javaCandidates += (Get-ChildItem $dir -Directory -ErrorAction SilentlyContinue |
            ForEach-Object { Join-Path $_.FullName 'bin\java.exe' })
    }
}
$javaCandidates += (Get-Command java.exe -ErrorAction SilentlyContinue).Source
$javaCandidates = $javaCandidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -Unique

$bestJava = $null
$bestMajor = 0
foreach ($candidate in $javaCandidates) {
    $major = Get-JavaMajor $candidate
    if ($major -ge 17 -and $major -gt $bestMajor) { $bestJava = $candidate; $bestMajor = $major }
}
if ($bestJava) {
    $env:DBMIND_JAVA = $bestJava
    Write-Host "Java：$bestJava（版本 $bestMajor，已固定为 DBMIND_JAVA）"
} else {
    Write-Host '未找到 Java 17+：SQLite 可用，其它数据库（走 Java 宿主）会连不上。' -ForegroundColor Yellow
    Write-Host '  → 装一个 JDK 17+ 并设 JAVA_HOME，或用 -Java <java.exe 路径> 指定' -ForegroundColor Yellow
}

# 已有实例在监听？直接复用，不重复起（否则第二个进程会因为端口被占而退出）
$existing = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
if ($existing) {
    Write-Host "端口 $Port 上已有服务在跑（PID $($existing[0].OwningProcess)），直接使用。" -ForegroundColor Green
} else {
    $serverArgs = @('--port', "$Port", '--dist', $dist)
    # **总是**把标准输出/错误重定向到文件，不要让它继承调用者的管道。
    # 踩过：服务是用 Start-Process 起的，继承了当时那个终端（或被脚本调用时的管道）的句柄；
    # 调用方一退出、管道一断，服务下一次写日志就会失败并退出 —— 表现是
    # 「刚启动还能访问，过一会儿就访问不了了」，而且没有任何提示。
    $logDir = Join-Path $root 'logs'
    New-Item -ItemType Directory -Force -Path $logDir | Out-Null
    $out = Join-Path $logDir 'server.out.log'
    $err = Join-Path $logDir 'server.err.log'
    Start-Process -FilePath $exe -ArgumentList $serverArgs -WindowStyle Hidden -RedirectStandardOutput $out -RedirectStandardError $err | Out-Null
    if ($Log) { Write-Host "日志：$out / $err" }
    Write-Host "已启动：$exe --port $Port --dist frontend\dist"
}

$url = "http://127.0.0.1:$Port/"
$ready = $false
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 250
    try {
        $health = Invoke-RestMethod -Uri "${url}api/dbmind/health" -TimeoutSec 3
        if ($health.ok -eq $true) { $ready = $true; break }
    } catch { }
}

if ($ready) {
    Write-Host "已就绪：$url" -ForegroundColor Green
    if (-not $NoBrowser) { Start-Process $url | Out-Null }
} else {
    Write-Host '服务未能在 10 秒内就绪：端口可能被别的程序占用，或进程启动即退出。' -ForegroundColor Red
    Write-Host "排查：Get-NetTCPConnection -LocalPort $Port -State Listen" -ForegroundColor Yellow
    Write-Host "      powershell -File scripts\start.ps1 -Port 20362" -ForegroundColor Yellow
    exit 1
}
