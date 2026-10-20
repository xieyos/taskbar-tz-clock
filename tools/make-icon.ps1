# 从透明背景的大图生成多尺寸 assets/app.ico（每个尺寸以 PNG 存储，Vista 及以上支持）。
# 用法：pwsh tools/make-icon.ps1 [-Source assets/icon-source.png] [-Output assets/app.ico]
param(
    [string]$Source = (Join-Path $PSScriptRoot '..\assets\icon-source.png'),
    [string]$Output = (Join-Path $PSScriptRoot '..\assets\app.ico')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

# 托盘按缩放取 16/20/24/32/40/48，资源管理器另外用到 64/256
$sizes = 16, 20, 24, 32, 40, 48, 64, 256
# 判定为"有内容"的最小 alpha，过滤生成图边缘的半透明噪点
$alphaThreshold = 24

$image = New-Object System.Drawing.Bitmap (Resolve-Path $Source).Path

# 按不透明像素求包围盒，扩成正方形并留 2% 边距，让图形居中
$minX = $image.Width; $minY = $image.Height; $maxX = 0; $maxY = 0
for ($y = 0; $y -lt $image.Height; $y += 2) {
    for ($x = 0; $x -lt $image.Width; $x += 2) {
        if ($image.GetPixel($x, $y).A -gt $alphaThreshold) {
            if ($x -lt $minX) { $minX = $x }; if ($x -gt $maxX) { $maxX = $x }
            if ($y -lt $minY) { $minY = $y }; if ($y -gt $maxY) { $maxY = $y }
        }
    }
}
$side = [int]([Math]::Max($maxX - $minX, $maxY - $minY) * 1.04)
$centerX = ($minX + $maxX) / 2; $centerY = ($minY + $maxY) / 2
$square = New-Object System.Drawing.Bitmap $side, $side
$g = [System.Drawing.Graphics]::FromImage($square)
$g.Clear([System.Drawing.Color]::Transparent)
$sourceRect = New-Object System.Drawing.Rectangle ([int]($centerX - $side / 2)), ([int]($centerY - $side / 2)), $side, $side
$g.DrawImage($image, (New-Object System.Drawing.Rectangle 0, 0, $side, $side), $sourceRect, [System.Drawing.GraphicsUnit]::Pixel)
$g.Dispose()

# 逐个尺寸高质量缩放并编码为 PNG
$images = foreach ($size in $sizes) {
    $bitmap = New-Object System.Drawing.Bitmap $size, $size
    $g = [System.Drawing.Graphics]::FromImage($bitmap)
    $g.InterpolationMode = 'HighQualityBicubic'; $g.SmoothingMode = 'HighQuality'
    $g.PixelOffsetMode = 'HighQuality'; $g.CompositingQuality = 'HighQuality'
    $g.DrawImage($square, (New-Object System.Drawing.Rectangle 0, 0, $size, $size))
    $g.Dispose()
    $stream = New-Object System.IO.MemoryStream
    $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
    [pscustomobject]@{ Size = $size; Bytes = $stream.ToArray() }
}

# ICO 文件：6 字节文件头 + 每个图像 16 字节目录项 + 图像数据
$out = New-Object System.IO.MemoryStream
$writer = New-Object System.IO.BinaryWriter $out
$writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$images.Count)
$offset = 6 + 16 * $images.Count
foreach ($entry in $images) {
    $dimension = if ($entry.Size -ge 256) { 0 } else { $entry.Size }   # 0 表示 256
    $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
    $writer.Write([byte]0); $writer.Write([byte]0)                      # 调色板数、保留
    $writer.Write([uint16]1); $writer.Write([uint16]32)                 # 位面数、位深
    $writer.Write([uint32]$entry.Bytes.Length); $writer.Write([uint32]$offset)
    $offset += $entry.Bytes.Length
}
foreach ($entry in $images) { $writer.Write($entry.Bytes) }
$writer.Flush()
[System.IO.File]::WriteAllBytes([System.IO.Path]::GetFullPath($Output), $out.ToArray())
"已生成 $Output（$($images.Count) 个尺寸，{0:N0} KB）" -f ($out.Length / 1KB)
