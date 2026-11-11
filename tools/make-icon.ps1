# draws assets\kadr.ico: the kadr mark (four crop marks and a centre dot) on
# a graphite tile, at every size windows asks for. small sizes are drawn on
# whole pixels so the marks stay sharp. run it again only to change the icon.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\make-icon.ps1 [-Preview <png>]
param([string] $Preview = '')
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'assets\kadr.ico'
New-Item -ItemType Directory -Force (Split-Path $out) | Out-Null

$tile = [Drawing.Color]::FromArgb(255, 0x1c, 0x1c, 0x1e)   # graphite ground
$edge = [Drawing.Color]::FromArgb(255, 0x48, 0x48, 0x4a)   # keeps the tile apart from a dark taskbar
$ink = [Drawing.Color]::FromArgb(255, 0xf2, 0xf2, 0xf2)

function Draw([int] $s) {
  $bmp = New-Object Drawing.Bitmap $s, $s, ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [Drawing.Graphics]::FromImage($bmp)
  $g.Clear([Drawing.Color]::Transparent)
  $g.SmoothingMode = 'AntiAlias'
  $g.PixelOffsetMode = 'Half'
  # rounded tile, windows 11 proportions
  $r = [math]::Max(2.0, $s * 0.22)
  $path = New-Object Drawing.Drawing2D.GraphicsPath
  $d = 2 * $r; $w = $s - 1
  $path.AddArc(0, 0, $d, $d, 180, 90); $path.AddArc($w - $d, 0, $d, $d, 270, 90)
  $path.AddArc($w - $d, $w - $d, $d, $d, 0, 90); $path.AddArc(0, $w - $d, $d, $d, 90, 90); $path.CloseFigure()
  $g.FillPath((New-Object Drawing.SolidBrush $tile), $path)
  if ($s -ge 24) { $g.DrawPath((New-Object Drawing.Pen $edge, 1), $path) }
  # crop marks on whole pixels
  $g.SmoothingMode = 'None'; $g.PixelOffsetMode = 'None'
  $inset = [math]::Max(3, [math]::Round($s * 0.21))
  # the arm is a share of the span between the marks, so they never close
  # into a square at small sizes: a visible gap stays in the middle.
  $arm = [math]::Max(2, [math]::Floor(($s - 2 * $inset) * 0.34))
  $t = [math]::Max(1, [math]::Round($s * 0.07))
  $b = New-Object Drawing.SolidBrush $ink
  $lo = $inset; $hi = $s - $inset
  foreach ($c in @(@($lo, $lo, 1, 1), @($hi, $lo, -1, 1), @($lo, $hi, 1, -1), @($hi, $hi, -1, -1))) {
    $x = $c[0]; $y = $c[1]; $dx = $c[2]; $dy = $c[3]
    $hx = if ($dx -gt 0) { $x } else { $x - $arm }; $hy = if ($dy -gt 0) { $y } else { $y - $t }
    $vx = if ($dx -gt 0) { $x } else { $x - $t }; $vy = if ($dy -gt 0) { $y } else { $y - $arm }
    $g.FillRectangle($b, $hx, $hy, $arm, $t)
    $g.FillRectangle($b, $vx, $vy, $t, $arm)
  }
  # centre dot, centred exactly (same parity as the icon size)
  $dot = [math]::Max(2, [math]::Round($s * 0.1)); if (($s - $dot) % 2) { $dot++ }
  $o = ($s - $dot) / 2
  $g.FillRectangle($b, $o, $o, $dot, $dot)
  $g.Dispose()
  $bmp
}

$sizes = 16, 20, 24, 32, 40, 48, 64, 96, 256
$images = foreach ($s in $sizes) {
  $bmp = Draw $s
  $ms = New-Object IO.MemoryStream
  $bmp.Save($ms, [Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
  [pscustomobject]@{ size = $s; png = $ms.ToArray() }
}

# .ico with png entries (supported for every size since windows vista)
$fs = [IO.File]::Create($out); $bw = New-Object IO.BinaryWriter $fs
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$images.Count)
$offset = 6 + 16 * $images.Count
foreach ($i in $images) {
  $dim = if ($i.size -ge 256) { 0 } else { $i.size }
  $bw.Write([byte]$dim); $bw.Write([byte]$dim); $bw.Write([byte]0); $bw.Write([byte]0)
  $bw.Write([uint16]1); $bw.Write([uint16]32); $bw.Write([uint32]$i.png.Length); $bw.Write([uint32]$offset)
  $offset += $i.png.Length
}
foreach ($i in $images) { $bw.Write($i.png) }
$bw.Close()
Write-Host "wrote $out ($((Get-Item $out).Length) bytes, sizes $($sizes -join ', '))"

if ($Preview) {
  # every size at 1:1 on light and dark grounds, plus 16 and 32 enlarged 8x
  $W = 900; $H = 420
  $sheet = New-Object Drawing.Bitmap $W, $H
  $g = [Drawing.Graphics]::FromImage($sheet)
  $g.FillRectangle((New-Object Drawing.SolidBrush ([Drawing.Color]::FromArgb(243, 243, 243))), 0, 0, $W, 140)
  $g.FillRectangle((New-Object Drawing.SolidBrush ([Drawing.Color]::FromArgb(32, 32, 32))), 0, 140, $W, 140)
  $x = 10
  foreach ($i in $images) {
    $img = [Drawing.Image]::FromStream((New-Object IO.MemoryStream(, $i.png)))
    $sz = [math]::Min($i.size, 128)
    $g.DrawImage($img, $x, 10, $sz, $sz); $g.DrawImage($img, $x, 150, $sz, $sz)
    $x += $sz + 12
  }
  $g.InterpolationMode = 'NearestNeighbor'; $g.PixelOffsetMode = 'Half'
  foreach ($pair in @(@(16, 10), @(32, 160), @(24, 450))) {
    $i = $images | Where-Object size -eq $pair[0]
    $img = [Drawing.Image]::FromStream((New-Object IO.MemoryStream(, $i.png)))
    $g.DrawImage($img, $pair[1], 290, [math]::Min(128, $pair[0] * 8), [math]::Min(128, $pair[0] * 8))
  }
  $g.Dispose(); $sheet.Save($Preview, [Drawing.Imaging.ImageFormat]::Png); $sheet.Dispose()
  Write-Host "preview $Preview"
}
