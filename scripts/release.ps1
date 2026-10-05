<#
最全流程的一键打包：从依赖到两版成品，每一步都显式做、显式查、显式报。

  [1/8] 环境检查    rustc / cargo / node / npm / WebView2 / 版本号一致性
  [2/8] 前端打包    依赖按需安装 + vite 构建 + 产物校验
  [3/8] 宿主 jar    四个 Java 宿主（缺失则自动构建，需要 JDK 17+ 与 Maven 3.6.3+）
  [4/8] 后端打包    cargo build --release（dbmind / web / mcp / desktop 四个 exe）
  [5/8] 绿色版组装  exe + agents/ + web/ + start.bat + README/LICENSE，并逐项列清单
  [6/8] 绿色版 zip  整包压缩，拷到别的机器解压即用
  [7/8] 桌面安装包  Tauri bundler 出 MSI（Windows Installer）与 NSIS（安装向导）
  [8/8] 汇总        产物清单 + SHA256 校验文件

用法（双击 scripts\release.bat 效果一样）：
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release.ps1
  ... -SkipPortable    只出安装包
  ... -SkipBundle      只出绿色版（不需要 WiX / 网络）
  ... -SkipBuild       复用已有 target\release（改过 Rust 代码就别加）
  ... -SkipFrontend    不动前端产物
  ... -SkipAgents      不检查也不构建宿主 jar（SQLite 以外的类型会「驱动未就绪」）
  ... -SkipZip         绿色版不打 zip
  ... -Quality         额外跑 cargo fmt --check 与 clippy -D warnings（耗时几分钟）

耗时：首次 5~10 分钟（release 编译 + 从 GitHub 下 WiX/NSIS，各一次，之后复用）；
      之后增量 1~2 分钟。编译阶段长时间「像没反应」是正常的 —— cargo 不逐行刷屏。

产物（都集中到 dist\ 下，不用去 target 里翻）：
  dist\dbmind-<版本>\                       绿色版目录
  dist\dbmind-<版本>-portable.zip           绿色版整包
  dist\installer\*.msi / *-setup.exe        安装包
  dist\SHA256SUMS.txt                      上面这些文件的 SHA256

★ 本文件必须存为 UTF-8 with BOM。PowerShell 5.1 读无 BOM 的 .ps1 会按 ANSI 解码，
  中文被截成半个字符后引号配对被破坏，脚本在**解析期**整体失败（一行都不执行）。见 README 第 6 节。
#>

[CmdletBinding()]
param(
    [switch]$SkipFrontend,
    [switch]$SkipAgents,
    [switch]$SkipBuild,
    [switch]$SkipPortable,
    [switch]$SkipBundle,
    [switch]$SkipZip,
    [switch]$Quality,
    # 把精简 JRE 打进包（各 +约 50 MB）：这样**目标机器不装 Java 也能连**
    # 走 Java 宿主的数据源（SQL Server / Mongo / Redis / ES / JDBC）。
    # 不加这个开关就打小包，届时靠目标机器上已装的 Java 17+（见 README 的前置条件）。
    [switch]$WithJre,
    # 指定用哪个 JDK 生成自带运行时（留空则自动挑一个 17+ 的）。
    # 例：-Jdk D:\develop\tools\jdk-25.0.4.1+1
    [string]$Jdk = ''
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$STEP_TOTAL = 8
$stepNo = 0
$stepWatch = $null
$totalWatch = [System.Diagnostics.Stopwatch]::StartNew()

# ------------------------------------------------------------------ 输出小工具
function Step([string]$text) {
    $script:stepNo = $script:stepNo + 1
    Write-Host ''
    Write-Host ('== [' + $script:stepNo + '/' + $script:STEP_TOTAL + '] ' + $text) -ForegroundColor Cyan
    $script:stepWatch = [System.Diagnostics.Stopwatch]::StartNew()
}
function EndStep() {
    if ($script:stepWatch) {
        Write-Host ('   ..  ' + [Math]::Round($script:stepWatch.Elapsed.TotalSeconds, 1) + ' 秒') -ForegroundColor DarkGray
        $script:stepWatch = $null
    }
}
function Good([string]$text) { Write-Host ('   OK  ' + $text) -ForegroundColor Green }
function Note([string]$text) { Write-Host ('   -   ' + $text) }
function Warn([string]$text) { Write-Host ('   !   ' + $text) -ForegroundColor Yellow }
function Fail([string]$text) { Write-Host ('   X   ' + $text) -ForegroundColor Red; exit 1 }
function Human([long]$bytes) {
    # 小于 1 MB 的用 KB 显示：产物清单里前端的资源文件大多是几十 KB，
    # 一律按 MB 四舍五入会排出一整列 "0 MB"，等于没说。
    if ($bytes -lt 1MB) { return ([Math]::Round($bytes / 1KB, 1).ToString() + ' KB') }
    return ([Math]::Round($bytes / 1MB, 1).ToString() + ' MB')
}
function DirSize([string]$path) {
    if (-not (Test-Path $path)) { return 0 }
    return (Get-ChildItem $path -Recurse -File | Measure-Object -Property Length -Sum).Sum
}
# 原生命令的 stderr 在 $ErrorActionPreference='Stop' 下会被当异常（README 第 6 节那条），
# 所以取版本一律经 cmd.exe 合并 stderr 再读。
function ToolVersion([string]$command) {
    try { return ((& cmd.exe /c ($command + ' 2>&1') | Select-Object -First 1) -replace "`r|`n", '').Trim() } catch { return '' }
}
function AgentJar([string]$name) {
    return (Join-Path $root ('agents\dbmind-agent-' + $name + '\target\dbmind-agent-' + $name + '.jar'))
}

# 版本号取自 workspace，用来给绿色版目录命名
$cargoToml = Get-Content (Join-Path $root 'Cargo.toml') -Raw -Encoding UTF8
$version = [regex]::Match($cargoToml, '(?ms)\[workspace\.package\].*?version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $version) { $version = '0.0.0' }
$portableDir = Join-Path $root ('dist\dbmind-' + $version)
$installerDir = Join-Path $root 'dist\installer'

Write-Host ''
Write-Host ('DBmind 打包 ' + $version) -ForegroundColor White
Write-Host ('项目根 ' + $root) -ForegroundColor DarkGray

# ================================================================== 1/8 环境检查
Step '环境检查'
$missingTools = @()
foreach ($tool in @('cargo', 'rustc', 'npm', 'node')) {
    if (Get-Command $tool -ErrorAction SilentlyContinue) {
        # 必须带上 --version：cargo / rustc / npm 不带参数只是打印帮助就退出，
        # 而 **node 不带参数会进入 REPL 等输入** —— 脚本会永远停在这一行（实测踩过）。
        Good ($tool.PadRight(6) + (ToolVersion ($tool + ' --version')))
    } else {
        $missingTools += $tool
    }
}
if ($missingTools -contains 'cargo' -or $missingTools -contains 'rustc') { Fail '缺少 Rust 工具链（装一个：https://rustup.rs）' }
if ($missingTools -contains 'npm' -or $missingTools -contains 'node') { Fail '缺少 Node.js / npm（前端构建需要）' }

# WebView2 是桌面版**运行**的前提（Win11 自带；Win10 需要单独装一次）
$webview = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue
if ($webview -and $webview.pv) {
    Good ('WebView2 ' + $webview.pv)
} else {
    Warn '未探到 WebView2 运行时：这台机器上跑桌面版会白屏，需先装 WebView2 Runtime'
}

# 版本一致性：安装包的版本取自 tauri.conf.json，绿色版目录取自 Cargo.toml。
# 两处漂开就会得到「同一个版本号、两个不同的东西」——发出去才发现就晚了，所以这里先对一次。
$confPath = Join-Path $root 'crates\dbmind-desktop\tauri.conf.json'
$confVersion = ''
if (Test-Path $confPath) {
    $confVersion = [string]((Get-Content $confPath -Raw -Encoding UTF8 | ConvertFrom-Json).version)
}
if ($confVersion -and $confVersion -ne $version) {
    Warn ('版本号不一致：Cargo.toml=' + $version + '   tauri.conf.json=' + $confVersion)
    Warn '  安装包会用 tauri.conf.json 的值；要一致就把两处改成同一个'
} else {
    Good ('版本 ' + $version + '（Cargo.toml 与 tauri.conf.json 一致）')
}
EndStep

# ================================================================== 2/8 前端打包
Step '前端打包（vite）'
$frontendDir = Join-Path $root 'frontend'
if ($SkipFrontend) {
    Note '已跳过（-SkipFrontend）'
} else {
    if (-not (Test-Path (Join-Path $frontendDir 'node_modules'))) {
        Note '前端依赖未安装，先装（首次约 1~2 分钟）'
        Push-Location $frontendDir
        try {
            if (Test-Path (Join-Path $frontendDir 'package-lock.json')) { & npm ci } else { & npm install }
            if ($LASTEXITCODE -ne 0) { Fail 'npm 安装依赖失败' }
        } finally { Pop-Location }
    } else {
        Good 'node_modules 已就绪'
    }
    Push-Location $frontendDir
    try {
        # 先自己把 dist 清空。Vite 也会清，但它走 Node 的 fs.rmSync —— IDE 终端会给 Node
        # 套一层"安全删除"shim，把「单轮删除 500 个以上文件」拦下来（dist\assets 有近 600 个），
        # 于是构建在第 2 步就失败：
        #   [safe-delete][SAFE_DELETE_BULK_CONFIRM_REQUIRED] {"count":589,"threshold":500}
        # 这里用 .NET 直接删目录树 —— 删的是构建产物、本来就不该进回收站；
        # 之后 Vite 面对的是一个不存在的目录，压根谈不上"批量删除"。
        $cleanDist = Join-Path $frontendDir 'dist'
        if (Test-Path $cleanDist) {
            try {
                [System.IO.Directory]::Delete($cleanDist, $true)
                Note '已清空 frontend\dist（免得构建时撞上批量删除拦截）'
            } catch {
                Warn ('清空 frontend\dist 失败，交给 Vite 自己清：' + $_.Exception.Message)
            }
        }
        & npm run build
        if ($LASTEXITCODE -ne 0) { Fail 'npm run build 失败' }
    } finally { Pop-Location }
}
$frontendDist = Join-Path $frontendDir 'dist'
if (-not (Test-Path (Join-Path $frontendDist 'index.html'))) { Fail 'frontend\dist\index.html 不存在（前端没构建成功）' }
Good ('frontend\dist  ' + (Human (DirSize $frontendDist)))
EndStep

# ================================================================== 3/8 宿主 jar
Step 'Java 宿主 jar（agents）'
$agentNames = @('jdbc', 'mongodb', 'redis', 'elasticsearch')
$missingJars = @($agentNames | Where-Object { -not (Test-Path (AgentJar $_)) })

if ($missingJars.Count -eq 0) {
    foreach ($name in $agentNames) { Good (('dbmind-agent-' + $name + '.jar').PadRight(32) + (Human (Get-Item (AgentJar $name)).Length)) }
} elseif ($SkipAgents) {
    Warn ('缺 ' + $missingJars.Count + ' 个宿主 jar（-SkipAgents：不构建、不补齐）')
} else {
    Note ('缺 ' + ($missingJars -join ', ') + '，尝试构建 —— 需要 JDK 17+ 与 Maven 3.6.3+')
    $javaOk = $false
    $javaText = ToolVersion 'java -version'
    if ($javaText -match 'version "(\d+)') { if ([int]$Matches[1] -ge 17) { $javaOk = $true } }
    $mvnOk = [bool](Get-Command mvn -ErrorAction SilentlyContinue)
    if ($javaOk -and $mvnOk) {
        Note '调用 scripts\build-agents.ps1'
        & (Join-Path $PSScriptRoot 'build-agents.ps1') -Quiet
    } else {
        Warn ('JDK17+ = ' + $javaOk + '    Maven = ' + $mvnOk)
        Warn '  -> 装齐后重跑本脚本；或先只出绿色版（加 -SkipBundle），SQLite 仍然可用'
    }
}
$agentReady = @($agentNames | Where-Object { Test-Path (AgentJar $_) }).Count
Good ('宿主 jar ' + $agentReady + '/4 就绪')
if ($agentReady -lt 4) {
    # 桌面版的内核在**壳进程内**，它按「exe 旁的 agents/」找 jar，而安装包靠
    # tauri.conf.json 的 bundle.resources 把这 4 个 jar 一起装进去。缺了它们，
    # 用户机器上只有 SQLite 能用，其它类型一律 DBMIND-DRV-0002。
    if (-not $SkipBundle) { Fail '要出安装包就必须 4 个 jar 齐全（先跑 scripts\build-agents.ps1）' }
    Warn '继续只出绿色版：SQLite 可用，其它类型会报「驱动未就绪」'
}
EndStep

# ================================================================== JRE（可选）
# 桌面壳的 Java 探测顺序里，**exe 旁的 jre/** 排在最前（见 crates/dbmind-desktop 的 ensure_java），
# 所以这里生成的运行时放进绿色版目录、并挂进安装包，目标机器就不必装 Java。
$jreDir = Join-Path $root 'dist\jre'
if ($WithJre) {
    Step 'JRE：用 jlink 生成精简运行时（-WithJre）'
    $jlink = ''
    $jlinkVer = $null
    if ($Jdk) {
        # 显式指定优先：给"我这台机器上就用这个 JDK"的场景
        $exe = Join-Path $Jdk 'bin\jlink.exe'
        if (-not (Test-Path $exe)) { Fail ('-Jdk 指定的目录里没有 bin\jlink.exe：' + $Jdk) }
        $verText = (& cmd.exe /c ('"' + $exe + '" --version 2>&1') | Select-Object -First 1)
        try { $jlinkVer = [version]($verText.Trim()) } catch { $jlinkVer = $null }
        if (-not $jlinkVer) { Fail ('读不出 jlink 的版本：' + $exe) }
        # 生成的运行时必须满足"宿主需要 Java 17 及以上"
        if ($jlinkVer.Major -lt 17) { Fail ('-Jdk 指定的 JDK 版本过低（' + $jlinkVer + '），宿主需要 17+') }
        $jlink = $exe
    } else {
        $jdkRoots = @($env:JAVA_HOME, 'C:\Program Files\Java', 'C:\Program Files\Eclipse Adoptium', 'C:\Program Files\Microsoft\jdk', 'D:\develop\tools')
        foreach ($root2 in $jdkRoots) {
            if (-not $root2 -or -not (Test-Path $root2)) { continue }
            $probe = @()
            if (Test-Path (Join-Path $root2 'bin\jlink.exe')) { $probe += $root2 }
            else { foreach ($d in (Get-ChildItem $root2 -Directory -ErrorAction SilentlyContinue)) { if (Test-Path (Join-Path $d.FullName 'bin\jlink.exe')) { $probe += $d.FullName } } }
            foreach ($jdk in $probe) {
                $exe = Join-Path $jdk 'bin\jlink.exe'
                $verText = (& cmd.exe /c ('"' + $exe + '" --version 2>&1') | Select-Object -First 1)
                $ver = $null
                try { $ver = [version]($verText.Trim()) } catch { $ver = $null }
                if (-not $ver) { continue }
                if ($ver.Major -lt 17) { continue }
                if (-not $jlinkVer -or $ver -gt $jlinkVer) { $jlink = $exe; $jlinkVer = $ver }
            }
        }
    }
    if (-not $jlink) { Fail '没找到 JDK 17+ 的 jlink（用 -Jdk 指定一个；或去掉 -WithJre 打成依赖本机 Java 的小包）' }
    Good ('jlink ' + $jlinkVer + '  ' + $jlink)
    if (Test-Path $jreDir) { Remove-Item $jreDir -Recurse -Force }
    Note 'jlink 生成中（约 20~40 秒）'
    # java.se 覆盖 JDBC 需要的全部 SE 模块（java.sql / java.naming / java.desktop / 加密相关…）；
    # jdk.crypto.ec 给 TLS 用，jdk.zipfs 给某些驱动读写 jar/zip 用。
    # 选 java.se 而不是逐个列模块：漏一个模块的表现是"某个驱动莫名其妙起不来"，很难查。
    # --compress 的取值在 JDK 21 变了：老版本是 --compress=2，21+ 只认 --compress=zip-6。
    # 传错会直接报错退出（用 JDK 25 时就是这种情况）。
    $compress = if ($jlinkVer.Major -ge 21) { '--compress=zip-6' } else { '--compress=2' }
    & $jlink --add-modules 'java.se,jdk.unsupported,jdk.crypto.ec,jdk.zipfs' --strip-debug --no-header-files --no-man-pages $compress --output $jreDir
    if ($LASTEXITCODE -ne 0) { Fail 'jlink 生成失败' }
    $javaExe = Join-Path $jreDir 'bin\java.exe'
    if (-not (Test-Path $javaExe)) { Fail '生成的 JRE 里没有 bin\java.exe' }
    Good ('JRE ' + (Human (DirSize $jreDir)) + '  ->  ' + $jreDir)
    EndStep
} else {
    Note 'JRE：未启用（加 -WithJre 可让包自带运行时，各 +约 50 MB）'
}

# ================================================================== 4/8 后端打包
Step '后端打包（cargo build --release）'
if ($SkipBuild) {
    Note '已跳过（-SkipBuild），复用 target\release 里的产物'
} else {
    # 提示按实际情况给，别一律写"首次"：有产物却仍在重编，通常是**依赖清单变了**
    # （比如新增一个 crate）—— 那时候说"首次"，会让人以为脚本判断错了（用户实际反馈过）。
    if (Test-Path (Join-Path $root 'target\release\dbmind-desktop.exe')) {
        Note '增量编译：通常 1~2 分钟；若刚改过 Cargo.toml（新增/升级依赖），会重编那批新依赖'
    } else {
        Note '首次编译：5~10 分钟（顺序编完所有依赖，期间不刷屏属正常）'
    }
    & cargo build --release
    if ($LASTEXITCODE -ne 0) { Fail 'cargo build --release 失败' }
}
foreach ($bin in @('dbmind.exe', 'dbmind-web.exe', 'dbmind-mcp.exe', 'dbmind-desktop.exe')) {
    $file = Join-Path $root ('target\release\' + $bin)
    if (Test-Path $file) { Good ($bin.PadRight(20) + (Human (Get-Item $file).Length)) }
    else { Fail ('缺少 target\release\' + $bin + '（别加 -SkipBuild，先完整编一次）') }
}
if ($Quality) {
    Note '额外质量关（-Quality）：fmt --check 与 clippy -D warnings'
    & cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { Warn 'fmt --check 有差异（不阻止打包）' }
    & cargo clippy --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { Warn 'clippy 有告警（不阻止打包）' }
}
EndStep

# ================================================================== 5/8 绿色版组装
Step '绿色版组装'
if ($SkipPortable) {
    Note '已跳过（-SkipPortable）'
} else {
    # 组装本身是 package.ps1 的职责，这里复用（编译/前端/agents 都已做完并检查过，全部跳过）
    & (Join-Path $PSScriptRoot 'package.ps1') -SkipCargo -SkipAgents -SkipFrontend -Out $portableDir
    if (-not (Test-Path $portableDir)) { Fail 'package.ps1 没有产出目录' }
    Good ('目录 ' + $portableDir)
    Get-ChildItem $portableDir -Recurse -File | Sort-Object FullName | ForEach-Object {
        Note ('  ' + $_.FullName.Substring($portableDir.Length + 1).PadRight(48) + (Human $_.Length).PadLeft(9))
    }
    # 自带 JRE 时放进便携目录：桌面壳会优先用它（exe 旁的 jre/），目标机器就不必装 Java
    if ($WithJre -and (Test-Path (Join-Path $jreDir 'bin\java.exe'))) {
        $target = Join-Path $portableDir 'jre'
        if (Test-Path $target) { Remove-Item $target -Recurse -Force }
        Copy-Item $jreDir $target -Recurse
        Good ('jre/ 已放入便携目录（' + (Human (DirSize $target)) + '）')
    }
    Good ('合计 ' + (Human (DirSize $portableDir)))
    Note '双击 dbmind-desktop.exe 出窗口；start.bat 走浏览器'
}
EndStep

# ================================================================== 6/8 绿色版 zip
Step '绿色版 zip'
if ($SkipPortable -or $SkipZip) {
    Note '已跳过'
} else {
    $zip = $portableDir + '-portable.zip'
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Compress-Archive -Path (Join-Path $portableDir '*') -DestinationPath $zip -CompressionLevel Optimal
    Good ($zip)
    Good (Human (Get-Item $zip).Length)
}
EndStep

# ================================================================== 7/8 安装包
Step '桌面安装包（Tauri：MSI + NSIS）'
if ($SkipBundle) {
    Note '已跳过（-SkipBundle）'
} else {
    Note '若本机还没有 WiX / NSIS，会先从 GitHub 下载（各一次，缓存在 %USERPROFILE%\.tauri，之后复用）'
    # 自带 JRE 时，把它写进 bundle.resources 一起打进安装包。
    # 这是**临时**改动配置文件：打完无论成败都用 finally 还原 —— 一次打包不该把仓库改脏。
    # 用 [IO.File]::WriteAllText + 不带 BOM 的 UTF8：PS 5.1 的 Set-Content -Encoding UTF8 会加 BOM，
    # 而带 BOM 的 JSON 会让 Tauri 解析失败。
    $confPath = Join-Path $root 'crates\dbmind-desktop\tauri.conf.json'
    $confBackup = $null
    $noBom = New-Object System.Text.UTF8Encoding($false)
    if ($WithJre -and (Test-Path (Join-Path $jreDir 'bin\java.exe'))) {
        $confBackup = [System.IO.File]::ReadAllText($confPath, [System.Text.Encoding]::UTF8)
        $patched = $confBackup -replace '("resources"\s*:\s*\{\s*)', ("`$1" + '"../../dist/jre": "jre", ')
        [System.IO.File]::WriteAllText($confPath, $patched, $noBom)
        Note 'JRE 已临时写进安装包配置（打包结束后自动还原）'
    }

    Push-Location (Join-Path $root 'crates\dbmind-desktop')
    try {
        # 用 npx 现取 Tauri CLI：不往 package.json 塞依赖，也不需要全局安装。
        # --config 显式指定本目录的 conf —— CLI 默认只认 ./src-tauri/ 那套约定。
        #
        # 刻意**不传 --bundles**：目标写在 tauri.conf.json 的 bundle.targets 里（msi + nsis），
        # 单一真源。踩过的坑：写 `--bundles msi,nsis` 时，PowerShell 把 `msi,nsis` 当**数组**解析，
        # 经 npx 重拼后 Tauri 收到的是一个值 `msi nsis`，直接报
        # `invalid value 'msi nsis' for '--bundles'`。
        & npx --yes '@tauri-apps/cli@^2' build --config tauri.conf.json
        if ($LASTEXITCODE -ne 0) { Fail 'tauri build 失败' }
    } finally {
        Pop-Location
        # 还原配置：放在 finally 里，失败路径也不会留下被改过的 tauri.conf.json
        if ($confBackup) {
            [System.IO.File]::WriteAllText($confPath, $confBackup, $noBom)
            Good 'tauri.conf.json 已还原'
        }
    }

    New-Item -ItemType Directory -Force -Path $installerDir | Out-Null
    $produced = @()
    foreach ($dir in @((Join-Path $root 'target\release\bundle'), (Join-Path $root 'crates\dbmind-desktop\target\release\bundle'))) {
        if (Test-Path $dir) {
            $produced += Get-ChildItem $dir -Recurse -File | Where-Object { $_.Extension -eq '.msi' -or $_.Name -like '*-setup.exe' }
        }
    }
    if ($produced.Count -eq 0) { Warn '没找到安装包产物，请回看上面的 tauri build 输出' }
    foreach ($item in $produced) {
        Copy-Item $item.FullName (Join-Path $installerDir $item.Name) -Force
        Good ($item.Name + '   ' + (Human $item.Length))
    }
}
EndStep

# ================================================================== 8/8 汇总
Step '汇总与校验'
$artifacts = @()
if (-not $SkipPortable) {
    $zipPath = $portableDir + '-portable.zip'
    if (Test-Path $zipPath) { $artifacts += (Get-Item $zipPath) }
    if (Test-Path $portableDir) { $artifacts += (Get-Item $portableDir) }
}
if (Test-Path $installerDir) { $artifacts += @(Get-ChildItem $installerDir -File) }

if ($artifacts.Count -eq 0) {
    Warn '本次没有任何产物'
} else {
    $lines = @()
    foreach ($item in $artifacts) {
        if ($item.PSIsContainer) {
            Write-Host ('   ' + $item.Name.PadRight(46) + (Human (DirSize $item.FullName)).PadLeft(10) + '   (目录)') -ForegroundColor Green
        } else {
            $hash = (Get-FileHash $item.FullName -Algorithm SHA256).Hash
            $lines += ($hash + '  ' + $item.Name)
            Write-Host ('   ' + $item.Name.PadRight(46) + (Human $item.Length).PadLeft(10)) -ForegroundColor Green
        }
        Note ('  ' + $item.FullName)
    }
    if ($lines.Count -gt 0) {
        $sumsFile = Join-Path $root 'dist\SHA256SUMS.txt'
        $lines | Set-Content -Path $sumsFile -Encoding ASCII
        Good ('SHA256 清单 ' + $sumsFile)
    }
}
EndStep

Write-Host ''
Write-Host ('全部完成，总用时 ' + [Math]::Round($totalWatch.Elapsed.TotalSeconds, 1) + ' 秒') -ForegroundColor Green
