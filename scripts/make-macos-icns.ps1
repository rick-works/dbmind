# 由 icons\icon.png 生成 macOS 用的 icon.icns（并输出一张预览图供核对）
#
# 为什么需要它：icons\ 里只有 icon.ico（Windows）与 icon.png（256×256），**没有 .icns**。
#   macOS 的图标不是"把画面铺满画布"——Apple 的规范是画面占 1024 画布的约 80%，
#   四周留透明边（dock 里所有 App 都按这个网格对齐）。画面满幅的图标放进 Dock/Finder，
#   视觉上会比邻居明显"大一圈"（用户反馈"mac 的图标偏大"）。
#   所以这里：① 把画面缩到 80.5% 居中放到正方形透明画布上；
#            ② 按 macOS 需要的各档尺寸生成 .icns（PNG 载荷，ic07..ic14 + icp4..icp6）。
#   Windows 的 icon.ico 不动 —— Windows 图标本来就是满幅的，改了反而不对。
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$src = Join-Path $root 'crates\dbmind-desktop\icons\icon.png'
$out = Join-Path $root 'crates\dbmind-desktop\icons\icon.icns'
$preview = Join-Path $root 'dist\_icon_preview_1024.png'

# Apple 图标网格：画面约 824/1024
$ratio = 0.805

$srcImg = [System.Drawing.Image]::FromFile($src)
Write-Host ("源图 " + $srcImg.Width + "x" + $srcImg.Height)

function New-PaddedPng([int]$size, $img) {
    $bmp = New-Object System.Drawing.Bitmap -ArgumentList $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)
    $inner = [int][Math]::Round($size * $script:ratio)
    $off = [int][Math]::Round(($size - $inner) / 2)
    $g.DrawImage($img, $off, $off, $inner, $inner)
    $g.Dispose()
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $bytes = $ms.ToArray()
    $ms.Dispose()
    return , $bytes
}

function BE32([int]$v) {
    return [byte[]]@((($v -shr 24) -band 0xFF), (($v -shr 16) -band 0xFF), (($v -shr 8) -band 0xFF), ($v -band 0xFF))
}

# 各档：类型码 → 像素
$entries = @(
    @{ t = 'ic07'; s = 128 },
    @{ t = 'ic08'; s = 256 },
    @{ t = 'ic09'; s = 512 },
    @{ t = 'ic10'; s = 1024 },
    @{ t = 'ic11'; s = 32 },
    @{ t = 'ic12'; s = 64 },
    @{ t = 'ic13'; s = 256 },
    @{ t = 'ic14'; s = 512 },
    @{ t = 'icp4'; s = 16 },
    @{ t = 'icp5'; s = 32 },
    @{ t = 'icp6'; s = 64 }
)

$body = New-Object System.IO.MemoryStream
foreach ($e in $entries) {
    $png = New-PaddedPng $e.s $srcImg
    $body.Write([System.Text.Encoding]::ASCII.GetBytes($e.t), 0, 4)
    $body.Write((BE32 (8 + $png.Length)), 0, 4)
    $body.Write($png, 0, $png.Length)
    Write-Host ("  " + $e.t + "  " + $e.s + "x" + $e.s + "  PNG " + $png.Length + " 字节")
}
$bodyBytes = $body.ToArray()
$body.Dispose()

$icns = New-Object System.IO.MemoryStream
$icns.Write([System.Text.Encoding]::ASCII.GetBytes('icns'), 0, 4)
$icns.Write((BE32 (8 + $bodyBytes.Length)), 0, 4)
$icns.Write($bodyBytes, 0, $bodyBytes.Length)
[System.IO.File]::WriteAllBytes($out, $icns.ToArray())
$icns.Dispose()

# 预览图（1024）供肉眼核对留白比例
$pv = New-PaddedPng 1024 $srcImg
[System.IO.File]::WriteAllBytes($preview, $pv)

$srcImg.Dispose()
$fi = Get-Item $out
Write-Host ("已写出 " + $fi.FullName + "  " + [math]::Round($fi.Length / 1KB, 1) + " KB")
Write-Host ("预览图 " + $preview)
