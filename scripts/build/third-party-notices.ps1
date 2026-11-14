# 生成 THIRD-PARTY-NOTICES.md：把随产品分发的第三方组件与许可汇总成一份
#
# 为什么要有这个文件：Apache-2.0 §4 要求分发时随附许可证与声明，MIT / BSD / ISC /
# Unicode / Zlib / MPL-2.0 等也都要求「保留版权与许可声明」。DBMind 的实际分发物是
# 一个安装包（exe + agents/*.jar + web/ + jre/），第三方代码从三个方向进来，本脚本
# 把这三种来源全扫一遍，输出一份人可读、可直接放进安装目录的清单：
#   1) 编译进 exe 的 Rust 依赖（cargo metadata）
#   2) 编译进 web/ 的前端依赖（node_modules，含嵌套）
#   3) 打进 agents/*.jar 的 Java 依赖（读 jar 内的 pom.properties，最接近真实分发）
#
# 依赖升级后重跑一次即可：powershell -File scripts\build\third-party-notices.ps1
param(
    [string]$Out = '',
    [string]$Root = ''
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem

if ($Root -eq '') { $Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path }
if ($Out -eq '') { $Out = Join-Path $Root 'THIRD-PARTY-NOTICES.md' }

$sb = New-Object System.Text.StringBuilder
function W([string]$s) { [void]$sb.AppendLine($s) }
function WV([string]$k, $v) { [void]$sb.AppendLine(('{0,-18}{1}' -f $k, $v)) }

$ver = ((Get-Content (Join-Path $Root 'Cargo.toml') -Encoding UTF8 | Where-Object { $_ -match '^version\s*=' } | Select-Object -First 1) -replace '.*"(.*)".*', '$1')

W '# 第三方组件与许可声明（Third-Party Notices）'
W ''
W "DBMind 数据库管理工具软件 V$ver"
W 'Copyright 2026 ZhongtangLi'
W ''
W '本文件随软件分发，用于履行 Apache License 2.0 第 4 条以及其它第三方许可的声明义务。'
W 'DBMind 本体以 Apache License 2.0 发布，条款见随包的 LICENSE，版权与商标声明见 NOTICE。'
W ''
W '清单由 `scripts\build\third-party-notices.ps1` 从 Cargo.lock、node_modules 与随包 jar'
W '中自动汇总；各组件许可全文可从其源码仓库或 https://spdx.org/licenses/ 取得。'
W ''

# ---------------------------------------------------------------- 1. Java agents
W '## 1. 随包分发的 Java 组件（agents/*.jar）'
W ''
W '| jar | 组件 | 版本 | 许可 |'
W '|---|---|---|---|'

$javaLic = @{
    'com.google.code.gson:gson'                    = 'Apache-2.0'
    'com.google.errorprone:error_prone_annotations' = 'Apache-2.0'
    'org.mongodb:mongodb-driver-sync'               = 'Apache-2.0'
    'org.mongodb:bson'                              = 'Apache-2.0'
    'org.mongodb:mongodb-driver-core'               = 'Apache-2.0'
    'redis.clients:jedis'                           = 'MIT'
    'org.slf4j:slf4j-api'                           = 'MIT'
    'org.apache.commons:commons-pool2'              = 'Apache-2.0'
    'org.json:json'                                 = 'JSON License（非 OSI，见下）'
}

$jarRows = New-Object System.Collections.Generic.List[string]
$jars = Get-ChildItem (Join-Path $Root 'agents') -Recurse -Filter 'dbmind-agent-*.jar' |
    Where-Object { $_.Name -notlike 'original-*' -and $_.Name -notlike '*-shaded.jar' } |
    Sort-Object Name

# 类路径前缀 -> 坐标：shade 会把一部分依赖的 pom.properties 合并掉（mongodb 驱动就是
# 这种情况，jar 里明明有 org/bson 与 com/mongodb 的类，清单却只列得出 gson），
# 所以补一层「按包前缀反推」，否则清单会漏掉真正被打进 jar 的组件。
$prefixMap = [ordered]@{
    'org/bson/'                 = 'org.mongodb:bson'
    'com/mongodb/'              = 'org.mongodb:mongodb-driver-sync'
    'redis/clients/'            = 'redis.clients:jedis'
    'org/json/'                 = 'org.json:json'
    'org/apache/commons/pool2/' = 'org.apache.commons:commons-pool2'
    'org/slf4j/'                = 'org.slf4j:slf4j-api'
    'com/google/gson/'          = 'com.google.code.gson:gson'
}

# 版本从各 agent 的 pom.xml 里取（依赖声明都写了显式版本）
$depVer = @{}
foreach ($m in @('jdbc', 'mongodb', 'redis', 'elasticsearch')) {
    $pomPath = Join-Path $Root "agents\dbmind-agent-$m\pom.xml"
    if (-not (Test-Path $pomPath)) { continue }
    $xml = Get-Content $pomPath -Raw -Encoding UTF8
    foreach ($hit in [regex]::Matches($xml, '<groupId>([^<]+)</groupId>\s*<artifactId>([^<]+)</artifactId>\s*<version>([^<]+)</version>')) {
        $k = $hit.Groups[1].Value.Trim() + ':' + $hit.Groups[2].Value.Trim()
        if (-not $depVer.ContainsKey($k)) { $depVer[$k] = $hit.Groups[3].Value.Trim() }
    }
}

foreach ($jar in $jars) {
    $z = [System.IO.Compression.ZipFile]::OpenRead($jar.FullName)
    $listed = @{}
    $props = $z.Entries | Where-Object { $_.FullName -like 'META-INF/maven/*/pom.properties' }
    foreach ($e in $props) {
        $sr = New-Object System.IO.StreamReader($e.Open())
        $txt = $sr.ReadToEnd()
        $sr.Close()
        $g = ((($txt -split "`n") | Where-Object { $_ -like 'groupId=*' }) -replace 'groupId=', '').Trim()
        $a = ((($txt -split "`n") | Where-Object { $_ -like 'artifactId=*' }) -replace 'artifactId=', '').Trim()
        $v = ((($txt -split "`n") | Where-Object { $_ -like 'version=*' }) -replace 'version=', '').Trim()
        if ($g -eq 'com.dbmind') { continue }   # 自己
        $key = "$g`:$a"
        $lic = $javaLic[$key]
        if (-not $lic) { $lic = '见组件源码' }
        $jarRows.Add("| $($jar.Name) | $key | $v | $lic |")
        $listed[$key] = $true
    }
    foreach ($pfx in $prefixMap.Keys) {
        $key = $prefixMap[$pfx]
        if ($listed.ContainsKey($key)) { continue }
        $cnt = ($z.Entries | Where-Object { $_.FullName.StartsWith($pfx) }).Count
        if ($cnt -le 0) { continue }
        $v = $depVer[$key]
        # bson 是 driver 的传递依赖，版本与 driver 同步（pom 里只声明了 driver）
        if (-not $v -and $key -eq 'org.mongodb:bson') { $v = $depVer['org.mongodb:mongodb-driver-sync'] }
        if (-not $v) { $v = '—' }
        $lic = $javaLic[$key]
        if (-not $lic) { $lic = '见组件源码' }
        $jarRows.Add("| $($jar.Name) | $key | $v | $lic（由 jar 内类路径推断） |")
        $listed[$key] = $true
    }
    $z.Dispose()
}
foreach ($row in ($jarRows | Sort-Object -Unique)) { W $row }

W ''
W '### org.json（JSON License）说明'
W ''
W '`agent-redis.jar` 通过 Jedis 传递引入 `org.json:json`。它使用的不是 OSI 批准的许可，'
W '而是业内俗称的 “JSON License”（正文见下）。该许可**不具传染性、不限制商业使用、'
W '允许再分发**，与 Apache-2.0 组件组合分发不构成冲突；仅在企业开源合规扫描工具中可能'
W '被标记为「非标准许可」，此处附条款原文以便说明。'
W ''
W '```'
W 'Copyright (c) 2002 JSON.org'
W ''
W 'Permission is hereby granted, free of charge, to any person obtaining a copy of'
W 'this software and associated documentation files (the "Software"), to deal in'
W 'the Software without restriction, including without limitation the rights to'
W 'use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of'
W 'the Software, and to permit persons to whom the Software is furnished to do so,'
W 'subject to the following conditions:'
W ''
W 'The above copyright notice and this permission notice shall be included in all'
W 'copies or substantial portions of the Software.'
W ''
W 'The Software shall be used for Good, not Evil.'
W ''
W 'THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR'
W 'IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS'
W 'FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR'
W 'COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER'
W 'IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN'
W 'CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.'
W '```'
W ''

# ---------------------------------------------------------------- 2. 原生文件
W '## 2. 随包分发的原生文件'
W ''
W '| 文件 | 来源 | 许可 |'
W '|---|---|---|'
W '| native-auth/mssql-jdbc_auth-12.8.1.x64.dll | mssql-jdbc（Microsoft JDBC Driver for SQL Server） | MIT |'
W ''

# ---------------------------------------------------------------- 3. JRE
W '## 3. 内置 Java 运行时（jre/）'
W ''
W '由 Eclipse Temurin JDK 21 通过 jlink 裁剪生成，许可为 GPLv2 + Classpath Exception，'
W '可随商业软件自由分发。运行时内每个文件的许可声明保留在 `jre/legal/` 目录。'
W ''
W '构建参数（与 CI 一致）：'
W ''
W '```'
W 'jlink --add-modules java.se,jdk.unsupported,jdk.crypto.ec,jdk.zipfs,jdk.charsets \'
W '      --strip-debug --no-header-files --no-man-pages --compress=zip-6 --output jre'
W '```'
W ''

# ---------------------------------------------------------------- 4. Rust
W '## 4. Rust 依赖（编译进 exe）'
W ''
W '按许可分组列出；同一组件只出现一次。'
W ''
$meta = cargo metadata --format-version 1 --locked 2>$null | ConvertFrom-Json
$rust = $meta.packages | Where-Object { $_.source -ne $null } | Sort-Object name
$groups = $rust | Group-Object { if ($_.license) { $_.license } else { 'UNKNOWN' } } | Sort-Object Name
foreach ($g in $groups) {
    W "### $($g.Name)（$($g.Count) 个）"
    W ''
    W ((($g.Group | ForEach-Object { "$($_.name) $($_.version)" }) -join '、'))
    W ''
}

# ---------------------------------------------------------------- 5. 前端
W '## 5. 前端依赖（编译进 web/）'
W ''
W '包含构建期依赖（不进产物）与运行时依赖，按许可分组。'
W ''
$fe = @{}
$feRoot = Join-Path $Root 'frontend\node_modules'
function Walk-Nm([string]$dir) {
    if (-not (Test-Path $dir)) { return }
    foreach ($name in (Get-ChildItem $dir -Directory -ErrorAction SilentlyContinue)) {
        if ($name.Name.StartsWith('.')) { continue }
        if ($name.Name.StartsWith('@')) { Walk-Nm $name.FullName; continue }
        $pkg = Join-Path $name.FullName 'package.json'
        if (Test-Path $pkg) {
            try {
                $j = Get-Content $pkg -Raw -Encoding UTF8 | ConvertFrom-Json
                if ($j.name -and $j.version) {
                    $lic = $j.license
                    if (-not $lic) {
                        if ($j.licenses) { $lic = (($j.licenses | ForEach-Object { $_.type }) -join ' OR ') }
                    }
                    if (-not $lic) { $lic = 'UNKNOWN' }
                    $key = "$($j.name) $($j.version)"
                    if (-not $fe.ContainsKey($key)) { $fe[$key] = [string]$lic }
                }
            } catch { }
        }
        $nested = Join-Path $name.FullName 'node_modules'
        if (Test-Path $nested) { Walk-Nm $nested }
    }
}
Walk-Nm $feRoot
$feGroups = $fe.GetEnumerator() | Group-Object { $_.Value } | Sort-Object Name
foreach ($g in $feGroups) {
    W "### $($g.Name)（$($g.Count) 个）"
    W ''
    W ((($g.Group | ForEach-Object { $_.Key }) -join '、'))
    W ''
}

# ---------------------------------------------------------------- 6. 不随包分发
W '## 6. 不随本产品分发的组件'
W ''
W '| 组件 | 说明 |'
W '|---|---|'
W '| 各数据库 JDBC 驱动 | 运行时由用户在本机从厂商渠道下载，存放于用户目录的 .dbmind/drivers 下，适用厂商自身许可（例如 MySQL Connector/J 的 GPLv2 + FOSS Exception、Oracle ojdbc 的 UPL、IBM Db2 的 IBM 许可） |'
W '| Microsoft Edge WebView2 运行时 | 使用系统已安装的运行时（Windows 10/11 自带或由微软分发），不随包分发 |'
W ''
W '---'
W ''
W '若发现清单与实际分发内容不一致，以 jar / 安装目录中的实际组件为准，并请反馈以便重新生成。'

[System.IO.File]::WriteAllText($Out, $sb.ToString(), (New-Object System.Text.UTF8Encoding($true)))
Write-Host "已生成：$Out（$([math]::Round((Get-Item $Out).Length / 1KB, 1)) KB）"
Write-Host "  Rust 组件 $($rust.Count) 个，前端组件 $($fe.Count) 个，Java 组件 $($jarRows.Count) 项"
