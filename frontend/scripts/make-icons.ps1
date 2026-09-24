<#
  从一张源图生成全部应用图标（并把四周背景抠成透明）。

  为什么要有它：图标散在几个地方，手工换图很容易漏；而且源图通常是**带白底的 JPEG**，
  直接缩放当图标用，深色模式下会在四周露出一圈白线（本项目实际发生过）。

  处理链：读源图 → 从容差洪泛去掉「外部背景」→ 裁剪到内容边界 → 按各尺寸高质量缩放 → 落盘。

  为什么要洪泛而不是"把白色变透明"：logo 内部本身就有白色线条（数据库圆柱、脑回路），
  一刀切会把图案打穿。洪泛只从画布四周向内吃掉**连通的背景**，图案内部的白色一律保留。

  用法（在 frontend 目录下）：
    powershell -ExecutionPolicy Bypass -File scripts/make-icons.ps1 -Source D:\path\to\logo.jpg
#>
param(
  [Parameter(Mandatory = $true)][string]$Source,
  [string]$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path,
  # 背景色容差（0~441 的 RGB 距离）。JPEG 有压缩噪声，留 60 左右既能吃掉噪点又不会咬到图案
  [int]$Tolerance = 60
)

Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = 'Stop'

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class LogoPrep
{
    /// 把画布四周与边角同色的**连通背景**置为透明，并裁剪到内容边界。
    public static Bitmap Process(string path, int tolerance)
    {
        using (var src = new Bitmap(path))
        {
            int w = src.Width, h = src.Height;
            var bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb);
            using (var g = Graphics.FromImage(bmp)) { g.Clear(Color.Transparent); g.DrawImage(src, 0, 0, w, h); }

            var rect = new Rectangle(0, 0, w, h);
            var data = bmp.LockBits(rect, ImageLockMode.ReadWrite, PixelFormat.Format32bppArgb);
            int stride = data.Stride;
            var buf = new byte[stride * h];
            Marshal.Copy(data.Scan0, buf, 0, buf.Length);

            // 背景色 = 四边中点与四角采样平均
            int[] sx = { 0, w - 1, 0, w - 1, w / 2, 0, w - 1, w / 2 };
            int[] sy = { 0, 0, h - 1, h - 1, 0, h / 2, h / 2, h - 1 };
            int br = 0, bg = 0, bb = 0;
            for (int i = 0; i < sx.Length; i++)
            {
                int o = sy[i] * stride + sx[i] * 4;
                br += buf[o + 2]; bg += buf[o + 1]; bb += buf[o];
            }
            br /= sx.Length; bg /= sx.Length; bb /= sx.Length;

            var seen = new bool[w * h];
            var stack = new Stack<int>();
            for (int x = 0; x < w; x++) { stack.Push(x); stack.Push((h - 1) * w + x); }
            for (int y = 0; y < h; y++) { stack.Push(y * w); stack.Push(y * w + w - 1); }

            int tol2 = tolerance * tolerance * 3;
            int minX = w, minY = h, maxX = -1, maxY = -1;

            while (stack.Count > 0)
            {
                int p = stack.Pop();
                if (seen[p]) continue;
                seen[p] = true;
                int x = p % w, y = p / w;
                int o = y * stride + x * 4;
                int db = buf[o] - bb, dg = buf[o + 1] - bg, dr = buf[o + 2] - br;
                if (db * db + dg * dg + dr * dr > tol2)
                {
                    // 碰到内容：只记边界，不继续往里走（保证只清外部背景）
                    if (x < minX) minX = x;
                    if (x > maxX) maxX = x;
                    if (y < minY) minY = y;
                    if (y > maxY) maxY = y;
                    continue;
                }
                buf[o + 3] = 0;
                if (x > 0) stack.Push(p - 1);
                if (x < w - 1) stack.Push(p + 1);
                if (y > 0) stack.Push(p - w);
                if (y < h - 1) stack.Push(p + w);
            }

            Marshal.Copy(buf, 0, data.Scan0, buf.Length);
            bmp.UnlockBits(data);

            if (maxX < 0) { bmp.Dispose(); throw new Exception("整张图都被判定为背景色，请调大/调小 -Tolerance 重试"); }

            var crop = Rectangle.FromLTRB(minX, minY, maxX + 1, maxY + 1);
            var result = bmp.Clone(crop, PixelFormat.Format32bppArgb);
            bmp.Dispose();
            return result;
        }
    }
}
'@

$srcPath = (Resolve-Path $Source).Path
$raw = New-Object System.Drawing.Bitmap($srcPath)
Write-Host ("  源图: {0}  {1}x{2}  (容差 {3})" -f $srcPath, $raw.Width, $raw.Height, $Tolerance)
$raw.Dispose()

$img = [LogoPrep]::Process($srcPath, $Tolerance)
Write-Host ("  抠背景 + 裁剪后: {0}x{1}" -f $img.Width, $img.Height)

function New-ScaledBitmap([System.Drawing.Image]$image, [int]$size) {
  $bmp = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $bmp.SetResolution(96, 96)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
  $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
  $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
  $g.Clear([System.Drawing.Color]::Transparent)
  # 源图是正方形（已按内容裁剪），这里按短边贴合，不拉伸
  $side = [Math]::Min($image.Width, $image.Height)
  $g.DrawImage($image, (New-Object System.Drawing.Rectangle(0, 0, $size, $size)), (New-Object System.Drawing.Rectangle(0, 0, $side, $side)), [System.Drawing.GraphicsUnit]::Pixel)
  $g.Dispose()
  return $bmp
}

function Save-Png([System.Drawing.Image]$image, [int]$size, [string]$path, [string]$note) {
  $dir = Split-Path -Parent $path
  if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
  $bmp = New-ScaledBitmap $image $size
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
  Write-Host ("  ✓ {0}  ({1}x{1}, {2} KB)  {3}" -f $path.Replace($Root, '').TrimStart('\'), $size, [math]::Round((Get-Item $path).Length / 1KB, 1), $note)
}

function Get-PngBytes([System.Drawing.Image]$image, [int]$size) {
  $bmp = New-ScaledBitmap $image $size
  $ms = New-Object System.IO.MemoryStream
  $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
  $bytes = $ms.ToArray()
  $ms.Dispose()
  return , $bytes
}

function Save-Ico([System.Drawing.Image]$image, [int[]]$sizes, [string]$path) {
  $blobs = @()
  foreach ($s in $sizes) { $blobs += , (Get-PngBytes $image $s) }

  $ms = New-Object System.IO.MemoryStream
  $bw = New-Object System.IO.BinaryWriter($ms)
  $bw.Write([UInt16]0)              # reserved
  $bw.Write([UInt16]1)              # type = icon
  $bw.Write([UInt16]$sizes.Count)
  $offset = 6 + 16 * $sizes.Count
  for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]
    $dim = if ($s -ge 256) { 0 } else { $s }   # 256 在 ICO 里记 0
    $bw.Write([Byte]$dim)
    $bw.Write([Byte]$dim)
    $bw.Write([Byte]0)              # 调色板数
    $bw.Write([Byte]0)              # reserved
    $bw.Write([UInt16]1)            # planes
    $bw.Write([UInt16]32)           # bpp
    $bw.Write([UInt32]$blobs[$i].Length)
    $bw.Write([UInt32]$offset)
    $offset += $blobs[$i].Length
  }
  foreach ($b in $blobs) { $bw.Write($b) }
  $bw.Flush()
  [IO.File]::WriteAllBytes($path, $ms.ToArray())
  $bw.Dispose()
  $ms.Dispose()
  Write-Host ("  ✓ {0}  ({1} 个尺寸: {2}, {3} KB)" -f $path.Replace($Root, '').TrimStart('\'), $sizes.Count, ($sizes -join '/'), [math]::Round((Get-Item $path).Length / 1KB, 1))
}

Save-Png $img 512 (Join-Path $Root 'frontend\src\assets\logo.png') '品牌图（备用）'
# favicon 每次打开页面都要下，尺寸给到浏览器实际会用到的上限即可（标签页 16~32 / 高分屏 64 / macOS 触控 180）
Save-Png $img 192 (Join-Path $Root 'frontend\public\favicon.png') '浏览器标签图标'
# 顶栏 logo 只显示 22px，源图 64px（2 倍屏不糊）；直接引用 512 会让首屏多下几百 KB
Save-Png $img 64 (Join-Path $Root 'frontend\src\assets\logo-sm.png') '顶栏 logo'
# 「关于」页的品牌图显示 40~44px，128 覆盖 2~3 倍屏
Save-Png $img 128 (Join-Path $Root 'frontend\src\assets\logo-md.png') '关于页品牌图'
Save-Png $img 256 (Join-Path $Root 'crates\dbmind-desktop\icons\icon.png') '桌面端图标'
Save-Ico $img @(16, 24, 32, 48, 64, 128, 256) (Join-Path $Root 'crates\dbmind-desktop\icons\icon.ico')

$img.Dispose()
Write-Host '  完成'
