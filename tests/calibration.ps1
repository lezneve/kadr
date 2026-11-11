# a full-screen calibration grid for checking the region overlay by eye
# (docs/overlay-check.md). shows on the monitor under the mouse cursor, one
# image pixel per physical screen pixel. esc or a click closes it.
#
# it captures nothing and reads nothing from the screen.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\calibration.ps1
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class CalDpi {
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern IntPtr MonitorFromPoint(POINT p, uint flags);
  [DllImport("user32.dll")] public static extern bool GetMonitorInfo(IntPtr m, ref MONITORINFO i);
  [DllImport("shcore.dll")] public static extern int GetDpiForMonitor(IntPtr m, int type, out uint x, out uint y);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct MONITORINFO { public int cb; public RECT mon, work; public uint flags; }
}
'@
[void][CalDpi]::SetThreadDpiAwarenessContext([IntPtr](-4))   # per-monitor v2: physical pixels
$pt = New-Object CalDpi+POINT; [void][CalDpi]::GetCursorPos([ref]$pt)
$mon = [CalDpi]::MonitorFromPoint($pt, 2)
$info = New-Object CalDpi+MONITORINFO; $info.cb = [Runtime.InteropServices.Marshal]::SizeOf($info); [void][CalDpi]::GetMonitorInfo($mon, [ref]$info)
[uint32]$dx = 0; [uint32]$dy = 0; [void][CalDpi]::GetDpiForMonitor($mon, 0, [ref]$dx, [ref]$dy)
$x0 = $info.mon.L; $y0 = $info.mon.T; $w = $info.mon.R - $x0; $h = $info.mon.B - $y0

$bmp = New-Object Drawing.Bitmap $w, $h
$g = [Drawing.Graphics]::FromImage($bmp)
$g.Clear([Drawing.Color]::FromArgb(250, 250, 250))
$thin = New-Object Drawing.Pen ([Drawing.Color]::FromArgb(200, 200, 200)), 1
$thick = New-Object Drawing.Pen ([Drawing.Color]::Black), 1
$font = New-Object Drawing.Font 'Segoe UI', 11, ([Drawing.FontStyle]::Regular), ([Drawing.GraphicsUnit]::Pixel)
$ink = [Drawing.Brushes]::Black
for ($x = 0; $x -lt $w; $x += 20) { $g.DrawLine($(if ($x % 100 -eq 0) { $thick } else { $thin }), $x, 0, $x, $h) }
for ($y = 0; $y -lt $h; $y += 20) { $g.DrawLine($(if ($y % 100 -eq 0) { $thick } else { $thin }), 0, $y, $w, $y) }
for ($x = 0; $x -lt $w; $x += 200) { for ($y = 0; $y -lt $h; $y += 200) {
  # labels are screen coordinates of the grid point (monitor origin added)
  $g.DrawString(("{0},{1}" -f ($x + $x0), ($y + $y0)), $font, $ink, $x + 3, $y + 2)
} }
# contrast patches: white, black, mid grey, dark grey, red, a photo-like gradient
$patches = @(@(255, 255, 255), @(0, 0, 0), @(128, 128, 128), @(40, 40, 40), @(220, 40, 40))
$px = 300; $py = 300
foreach ($c in $patches) { $g.FillRectangle((New-Object Drawing.SolidBrush ([Drawing.Color]::FromArgb($c[0], $c[1], $c[2]))), $px, $py, 200, 200); $px += 300 }
$grad = New-Object Drawing.Drawing2D.LinearGradientBrush ((New-Object Drawing.Point 300, 0), (New-Object Drawing.Point 1700, 0), [Drawing.Color]::Black, [Drawing.Color]::White)
$g.FillRectangle($grad, 300, 600, 1400, 100)
$g.DrawString(("monitor {0}x{1} at {2},{3}, scale {4} % · esc or click closes" -f $w, $h, $x0, $y0, [math]::Round($dx / 96 * 100)), $font, $ink, 300, 260)
$g.Dispose()

$form = New-Object Windows.Forms.Form
$form.FormBorderStyle = 'None'; $form.StartPosition = 'Manual'; $form.TopMost = $true
$form.Bounds = New-Object Drawing.Rectangle $x0, $y0, $w, $h
$form.BackgroundImage = $bmp; $form.BackgroundImageLayout = 'None'
$form.KeyPreview = $true
$form.Add_KeyDown({ if ($_.KeyCode -eq 'Escape') { $form.Close() } })
$form.Add_MouseClick({ $form.Close() })
[void]$form.ShowDialog()
$bmp.Dispose()
