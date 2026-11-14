# 生成随产品分发的精简 Java 运行时（dist\jre）
#
# 为什么用 Temurin 而不是 Oracle JDK：Oracle JDK 17+ 走 NFTC 条款，随商业软件
# 再分发的解释空间较大；Eclipse Temurin 是 GPLv2 + Classpath Exception，分发
# 无争议。CI（.github/workflows/release.yml 的「生成自带 JRE」）用的也是
# temurin 21，本地用同一套模块集与参数，本地便携包才和发布包一致。
#
# 模块集不能动：
#   少 jdk.charsets   -> Doris 连不上（MySQL Connector/J 要把 collation 映射成
#                        Java 字符集，报 Unknown character set: 'eucjpms'）
#   少 jdk.unsupported-> DB2 报 ClassNotFoundException: jdk.net.ExtendedSocketOptions
#
# 用法：
#   powershell -File scripts\build\jre.ps1
#   powershell -File scripts\build\jre.ps1 -JdkHome 'D:\develop\tools\temurin21\jdk-21.0.13+9'
param(
    [string]$JdkHome = 'D:\develop\tools\temurin21\jdk-21.0.13+9',
    [string]$Out = ''
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if ($Out -eq '') { $Out = Join-Path $root 'dist\jre' }

$jlink = Join-Path $JdkHome 'bin\jlink.exe'
if (-not (Test-Path $jlink)) {
    throw "没找到 jlink：$jlink。用 -JdkHome 指定一个 JDK 21+（推荐 Eclipse Temurin）"
}

$modules = 'java.se,jdk.unsupported,jdk.crypto.ec,jdk.zipfs,jdk.charsets'
$new = $Out + '.new'

if (Test-Path $new) { Remove-Item $new -Recurse -Force }

# 旧目录先挪成 .bak 再生成：生成失败时还能退回上一次能用的运行时。
$bak = $Out + '.bak'
if (Test-Path $Out) {
    if (Test-Path $bak) { Remove-Item $bak -Recurse -Force }
    Move-Item $Out $bak
}

& $jlink --add-modules $modules --strip-debug --no-header-files --no-man-pages --compress=zip-6 --output $new
if ($LASTEXITCODE -ne 0) {
    if ((-not (Test-Path $Out)) -and (Test-Path $bak)) { Move-Item $bak $Out }
    throw "jlink 失败（exit $LASTEXITCODE），已回退到上一次的运行时"
}

if (Test-Path $Out) { Remove-Item $Out -Recurse -Force }
Move-Item $new $Out
if (Test-Path $bak) { Remove-Item $bak -Recurse -Force }

$size = [math]::Round(((Get-ChildItem $Out -Recurse -File | Measure-Object Length -Sum).Sum / 1MB), 0)
$release = Get-Content (Join-Path $Out 'release') -Encoding UTF8
$ver = ($release | Where-Object { $_ -like 'JAVA_VERSION=*' } | Select-Object -First 1)
$impl = ($release | Where-Object { $_ -like 'IMPLEMENTOR=*' } | Select-Object -First 1)

Write-Host "生成完成：$Out（$size MB）"
if ($ver) { Write-Host "  $ver" }
if ($impl) { Write-Host "  $impl" }
Write-Host '  自检（应出现 Temurin / Adoptium 字样）：'
# java -version 走 stderr，在 $ErrorActionPreference='Stop' 下会被当成 NativeCommandError
# 中断脚本（exit 1），这里局部放开。
$prev = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
& (Join-Path $Out 'bin\java.exe') -version 2>&1 | ForEach-Object { Write-Host ('    ' + $_) }
$ErrorActionPreference = $prev
