# editor tools (stage 3), on a synthetic screen (KADR_TEST_FRAME=1).
#
# shapes are drawn by posting mouse messages to the editor window in its own
# client coordinates, which are the image's physical pixels; commands are
# posted as WM_COMMAND, the same ids the keyboard shortcuts send. nothing
# injects real input. every export is checked pixel by pixel against the
# synthetic pattern. the user's clipboard is backed up (not read) and restored.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\annotate.ps1 [-Points 20000]
param([int] $Points = 20000)
. "$PSScriptRoot\common.ps1"
$env:KADR_TEST_FRAME = '1'
$WM_COMMAND = 0x0111; $MK_LBUTTON = 1; $MK_SHIFT = 4
$ID_COPY = 1; $ID_SAVE = 2; $ID_CLOSE = 3; $ID_UNDO = 10; $ID_REDO = 11; $ID_DELETE = 12; $ID_TOOL = 100
$T_SELECT = 0; $T_PEN = 1; $T_LINE = 2; $T_RECT = 3; $T_ELLIPSE = 4; $T_HIGHLIGHT = 5; $T_BLUR = 6; $T_REDACT = 7
Add-Type -ReferencedAssemblies System.Drawing @'
using System; using System.Drawing; using System.Drawing.Imaging; using System.Collections.Generic;
public static class Img {
  // must match capture::synthetic_pixel
  public static int[] Px(int x, int y) {
    int check = (((x >> 4) + (y >> 4)) & 1) * 120;
    return new int[] { (byte)(check + x / 7), (byte)((byte)y * 3), (byte)(((byte)x) ^ check) }; // r, g, b
  }
  public class Pic { public int W, H; public byte[] Bgra; }
  public static Pic Load(string path) {
    using (var b = new Bitmap(path)) {
      var d = b.LockBits(new Rectangle(0, 0, b.Width, b.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
      var p = new Pic { W = b.Width, H = b.Height, Bgra = new byte[b.Width * b.Height * 4] };
      for (int y = 0; y < b.Height; y++) System.Runtime.InteropServices.Marshal.Copy(d.Scan0 + y * d.Stride, p.Bgra, y * b.Width * 4, b.Width * 4);
      b.UnlockBits(d); return p;
    }
  }
  public static int[] At(Pic p, int x, int y) { int i = (y * p.W + x) * 4; return new[] { (int)p.Bgra[i + 2], p.Bgra[i + 1], p.Bgra[i] }; }
  public static int Diff(int[] a, int[] b) { return Math.Max(Math.Abs(a[0] - b[0]), Math.Max(Math.Abs(a[1] - b[1]), Math.Abs(a[2] - b[2]))); }
  // pixels that differ from the pattern outside every excluded rect (l,t,r,b exclusive)
  public static long Outside(Pic p, int ox, int oy, int[][] ex) {
    long bad = 0;
    for (int y = 0; y < p.H; y++) for (int x = 0; x < p.W; x++) {
      bool skip = false;
      foreach (var r in ex) if (x >= r[0] && x < r[2] && y >= r[1] && y < r[3]) { skip = true; break; }
      if (!skip && Diff(At(p, x, y), Px(ox + x, oy + y)) != 0) bad++;
    }
    return bad;
  }
  // mean absolute difference to the right neighbour (green), inside l,t,r,b
  public static double Rough(Pic p, int l, int t, int r, int b, bool pattern, int ox, int oy) {
    double s = 0; long n = 0;
    for (int y = t; y < b; y++) for (int x = l; x < r - 1; x++) {
      int a = pattern ? Px(ox + x, oy + y)[2] : At(p, x, y)[2];
      int c = pattern ? Px(ox + x + 1, oy + y)[2] : At(p, x + 1, y)[2];
      s += Math.Abs(a - c); n++;
    }
    return s / n;
  }
  public static long NotColor(Pic p, int l, int t, int r, int b, int[] c, int tol) {
    long bad = 0;
    for (int y = t; y < b; y++) for (int x = l; x < r; x++) if (Diff(At(p, x, y), c) > tol) bad++;
    return bad;
  }
  // mean |p(x) - p(x+1)| (blue) over rows t..b: a tile seam shows as a jump
  public static double ColStep(Pic p, int x, int t, int b) {
    double s = 0; for (int y = t; y < b; y++) s += Math.Abs(At(p, x, y)[2] - At(p, x + 1, y)[2]); return s / (b - t);
  }
  public static long Exact(Pic p, int ox, int oy) { return Outside(p, ox, oy, new int[0][]); }
}
'@
Add-Type -AssemblyName System.Windows.Forms

function Editor-Window([int] $ms = 3000) {
  $end = (Get-Date).AddMilliseconds($ms)
  while ((Get-Date) -lt $end) { $e = [KadrTest]::FindWindow('kadr.editor', [NullString]::Value); if ($e -ne [IntPtr]::Zero) { return $e }; Start-Sleep -Milliseconds 15 }
  [IntPtr]::Zero
}
function Cmd($e, [int] $id) { [void][KadrTest]::PostMessage($e, $WM_COMMAND, [IntPtr]$id, [IntPtr]::Zero) }
function Tool($e, [int] $t) { Cmd $e ($ID_TOOL + $t) }
function M-Down($e, $x, $y, $m = 0) { [void][KadrTest]::PostPhysical($e, $WM_LBUTTONDOWN, [IntPtr]($MK_LBUTTON -bor $m), (Pack-Point $x $y)) }
function M-Move($e, $x, $y, $m = 0) { [void][KadrTest]::PostPhysical($e, $WM_MOUSEMOVE, [IntPtr]($MK_LBUTTON -bor $m), (Pack-Point $x $y)) }
function M-Up($e, $x, $y, $m = 0) { [void][KadrTest]::PostPhysical($e, $WM_LBUTTONUP, [IntPtr]$m, (Pack-Point $x $y)) }
# a gesture through the given points; waits for the editor to record it
function Gesture($e, $pts, [int] $m = 0, [string] $expect = 'editor \d+: (add|change) ') {
  $before = Count-Trace $expect
  M-Down $e $pts[0][0] $pts[0][1] $m
  for ($i = 1; $i -lt $pts.Count; $i++) { M-Move $e $pts[$i][0] $pts[$i][1] $m }
  M-Up $e $pts[-1][0] $pts[-1][1] $m
  [void](Wait-Count $expect ($before + 1) 3000)
}
function Line-Pts($x0, $y0, $x1, $y1, [int] $n = 8) { $out = @(); for ($i = 0; $i -le $n; $i++) { $out += , @([int]($x0 + ($x1 - $x0) * $i / $n), [int]($y0 + ($y1 - $y0) * $i / $n)) }; , $out }
function Last-Trace([string] $pattern) { @(Trace | Select-String $pattern)[-1].Line }
function Save-Now($e) {
  $before = @(Get-ChildItem $caps -Filter *.png).Count
  $n = Count-Trace 'editor \d+: (saved|already saved)'
  Cmd $e $ID_SAVE
  [void](Wait-Count 'editor \d+: (saved|already saved)' ($n + 1) 4000)
  $files = @(Get-ChildItem $caps -Filter *.png | Sort-Object LastWriteTime)
  if ($files.Count -gt $before) { $files[-1].FullName } else { $null }
}
function Answer($procId, [int] $button) {
  for ($i = 0; $i -lt 60; $i++) {
    $d = [KadrTest]::WindowOf([uint32]$procId, '#32770')
    if ($d -ne [IntPtr]::Zero) { $b = [KadrTest]::GetDlgItem($d, $button); if ($b -ne [IntPtr]::Zero) { [void][KadrTest]::PostMessage($b, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero); return $true } }
    Start-Sleep -Milliseconds 50
  }
  $false
}
function Wait-Gone($procId, [int] $ms = 3000) {
  $end = (Get-Date).AddMilliseconds($ms)
  while ((Get-Date) -lt $end) { if (-not (Get-Process -Id $procId -ErrorAction SilentlyContinue)) { return $true }; Start-Sleep -Milliseconds 20 }
  $false
}
function Open-Region($x0, $y0, $x1, $y1) {
  $want = "shown $($x1 - $x0 + 1)x$($y1 - $y0 + 1) at $x0,$y0"
  $before = Count-Trace 'editor \d+: shown'
  $o = Open-Overlay $p $h; Start-Sleep -Milliseconds 100; Drag $o $x0 $y0 $x1 $y1
  [void](Wait-Count 'editor \d+: shown' ($before + 1) 3000)
  $e = Editor-Window; Start-Sleep -Milliseconds 100
  $line = Last-Trace 'editor \d+: shown'
  if ($line -notmatch [regex]::Escape($want)) { Write-Host "  (editor opened with another size: $line)" }
  $e
}
function Ed-Mem($procId) {
  $x = Get-Process -Id $procId -ErrorAction SilentlyContinue; if (-not $x) { return $null }
  [pscustomobject]@{ private = [math]::Round($x.PrivateMemorySize64 / 1MB, 1); ws = [math]::Round($x.WorkingSet64 / 1MB, 1); gdi = [KadrTest]::GetGuiResources($x.Handle, 0); user = [KadrTest]::GetGuiResources($x.Handle, 1) }
}

$RED = @(255, 59, 48); $YELLOW = @(255, 214, 10)
$backup = Backup-Clipboard
Write-Host "clipboard backed up ($($backup.formats) format(s)); contents are not read or printed"
try {

$p = Start-Kadr (Settings "save_folder = $caps`r`nclose_after_copy = no`r`n") -Extra '--test-private-clipboard'
[void](Wait-Trace 'ready'); $h = Window-Of $p 'kadr.main'
$OX = 300; $OY = 250

Write-Host "`n1. every tool, then the export pixel by pixel (801x501 at $OX,$OY)"
$e = Open-Region 300 250 1100 750
$ep = [KadrTest]::PidOf($e)
Check ($e -ne [IntPtr]::Zero) 'editor open'
$m0 = Ed-Mem $ep
Gesture $e (Line-Pts 100 100 300 200)                        # rectangle is the default tool
Check ((Last-Trace 'add rect') -match 'add rect \d+ 100,100-300,200') 'rectangle at the dragged pixels'
Gesture $e (Line-Pts 400 50 500 90) $MK_SHIFT
Check ((Last-Trace 'add rect') -match 'add rect \d+ 400,50-500,150') "shift makes a square ($(Last-Trace 'add rect'))"
Tool $e $T_ELLIPSE; Gesture $e (Line-Pts 50 300 250 450)
Check ((Last-Trace 'add ellipse') -match '50,300-250,450') 'ellipse'
Gesture $e (Line-Pts 300 30 360 50) $MK_SHIFT
Check ((Last-Trace 'add ellipse') -match '300,30-360,90') "shift makes a circle ($(Last-Trace 'add ellipse'))"
Tool $e $T_LINE; Gesture $e (Line-Pts 300 300 500 450)
Check ((Last-Trace 'add line') -match '300,300-500,450') 'line'
Gesture $e (Line-Pts 520 200 600 212) $MK_SHIFT
Check ((Last-Trace 'add line') -match '520,200-60[01],200') "shift snaps a line to 45° steps ($(Last-Trace 'add line'))"
Tool $e $T_PEN
$pen = @(); for ($x = 50; $x -le 350; $x += 5) { $pen += , @($x, (480 + [int](3 * [math]::Sin($x / 20)))) }
Gesture $e $pen
Check ((Last-Trace 'add pen') -match 'add pen') 'pen stroke'
Tool $e $T_HIGHLIGHT; Gesture $e (Line-Pts 550 300 750 300 20)
Check ((Last-Trace 'add highlighter') -match '550,300-750,300') 'highlighter'
Tool $e $T_BLUR; Gesture $e (Line-Pts 550 350 780 480)
Check ((Last-Trace 'add blur') -match '550,350-780,480') 'blur area'
Tool $e $T_REDACT; Gesture $e (Line-Pts 600 50 780 150)
Check ((Last-Trace 'add redact') -match '600,50-780,150') 'redact area'
Check (@(Get-ChildItem $caps -Filter *.png).Count -eq 0) 'nothing saved on its own'
$f1 = Save-Now $e
Check ($null -ne $f1) 'saved with every annotation'
$pic = [Img]::Load($f1)
Check ($pic.W -eq 801 -and $pic.H -eq 501) 'export keeps the capture size'
$px = { param($x, $y) [Img]::At($pic, $x, $y) }
Check ([Img]::Diff((& $px 100 150), $RED) -le 2) "rectangle edge is red ($((& $px 100 150) -join ','))"
Check ([Img]::Diff((& $px 200 150), [Img]::Px($OX + 200, $OY + 150)) -eq 0) 'rectangle inside is untouched'
Check ([Img]::Diff((& $px 450 150), $RED) -le 2) 'shift square: its bottom edge is red'
Check ([Img]::Diff((& $px 150 300), $RED) -le 40) "ellipse top is red ($((& $px 150 300) -join ','))"
Check ([Img]::Diff((& $px 150 375), [Img]::Px($OX + 150, $OY + 375)) -eq 0) 'ellipse inside is untouched'
Check ([Img]::Diff((& $px 400 375), $RED) -le 10) 'line middle is red'
$py = 480 + [int](3 * [math]::Sin(200 / 20))
Check ([Img]::Diff((& $px 200 $py), $RED) -le 10) 'pen stroke is red'
$src = [Img]::Px($OX + 650, $OY + 300); $want = @(0, 1, 2 | ForEach-Object { [int]($src[$_] * 0.6 + $YELLOW[$_] * 0.4) })
Check ([Img]::Diff((& $px 650 300), $want) -le 4) "highlighter is a 40 % yellow over the image ($((& $px 650 300) -join ',') vs $($want -join ','))"
$r0 = [Img]::Rough($pic, 560, 360, 770, 470, $true, $OX, $OY); $r1 = [Img]::Rough($pic, 560, 360, 770, 470, $false, $OX, $OY)
Check ($r1 -lt $r0 / 4) ("blur: neighbour difference {0:n1} -> {1:n1}" -f $r0, $r1)
Check ([Img]::NotColor($pic, 600, 50, 780, 150, @(0, 0, 0), 0) -eq 0) 'redact: every pixel is opaque black'
$ex = @(@(96, 96, 304, 204), @(396, 46, 504, 154), @(46, 296, 254, 454), @(296, 26, 364, 94), @(296, 296, 504, 454), @(516, 196, 604, 204 + 4), @(46, 470, 356, 490), @(539, 289, 761, 311), @(550, 350, 780, 480), @(600, 50, 780, 150))
$out = [Img]::Outside($pic, $OX, $OY, $ex)
Check ($out -eq 0) "every pixel outside the shapes equals the capture ($out differ)"

Write-Host "`n2. undo, redo and saving after them"
for ($i = 0; $i -lt 10; $i++) { Cmd $e $ID_UNDO }
Check (Wait-Trace 'undo -> state 0' 3000) 'undo back to the untouched capture'
$f2 = Save-Now $e
Check ($null -ne $f2 -and [Img]::Exact([Img]::Load($f2), $OX, $OY) -eq 0) 'saved after undo all: identical to the capture'
for ($i = 0; $i -lt 10; $i++) { Cmd $e $ID_REDO }
Start-Sleep -Milliseconds 400
$f3 = Save-Now $e
Check ($null -eq $f3 -and (Last-Trace 'already saved')) 'redo all returns to the first saved state: no duplicate file'
Cmd $e $ID_UNDO; Start-Sleep -Milliseconds 200
$f4 = Save-Now $e
$pic4 = if ($f4) { [Img]::Load($f4) }
Check ($f4 -and [Img]::NotColor($pic4, 600, 50, 780, 150, @(0, 0, 0), 0) -gt 0 -and [Img]::Diff([Img]::At($pic4, 100, 150), $RED) -le 2) 'after one undo: the last shape (redact) is gone, the rest stays'
Cmd $e $ID_REDO; Start-Sleep -Milliseconds 200

Write-Host "`n3. select, move, resize, delete"
Tool $e $T_SELECT
Gesture $e (Line-Pts 100 150 140 190) 0 'editor \d+: change'
Check ((Last-Trace 'change') -match '140,140-340,240') "drag moves the rectangle by 40,40 ($(Last-Trace 'change'))"
Gesture $e (Line-Pts 340 240 400 300) 0 'editor \d+: change'
Check ((Last-Trace 'change') -match '140,140-400,300') "the corner handle resizes it ($(Last-Trace 'change'))"
Gesture $e (Line-Pts 400 300 400 260) $MK_SHIFT 'editor \d+: change'
Check ((Last-Trace 'change') -match '140,140-400,400') "shift keeps it square while resizing ($(Last-Trace 'change'))"
M-Down $e 690 100; M-Up $e 690 100; Start-Sleep -Milliseconds 150
Cmd $e $ID_DELETE
Check (Wait-Trace 'editor \d+: remove' 2000) 'delete removes the selected redact'
$f5 = Save-Now $e
$pic5 = [Img]::Load($f5)
Check ([Img]::Diff([Img]::At($pic5, 690, 100), [Img]::Px($OX + 690, $OY + 100)) -eq 0) 'the deleted area shows the capture again'
Check ([Img]::Diff([Img]::At($pic5, 140, 300), $RED) -le 2 -and [Img]::Diff([Img]::At($pic5, 100, 150), [Img]::Px($OX + 100, $OY + 150)) -eq 0) 'the rectangle is exported where it was moved, not where it was'
Cmd $e $ID_UNDO; Cmd $e $ID_UNDO; Cmd $e $ID_UNDO; Cmd $e $ID_UNDO; Start-Sleep -Milliseconds 200
$f6 = Save-Now $e
Check ($null -eq $f6 -and (Last-Trace 'editor \d+: (saved|already saved)') -match 'already saved') 'undo of delete, resize and move returns exactly to the state saved first: no new file'
Cmd $e $ID_COPY; [void](Wait-Trace 'editor \d+: copied' 2000); Start-Sleep -Milliseconds 200
$clip = [Windows.Forms.Clipboard]::GetImage()
if ($clip) { $cp = Join-Path $work 'clip.png'; $clip.Save($cp, [Drawing.Imaging.ImageFormat]::Png); $clip.Dispose(); $pic6 = [Img]::Load($cp) }
Check ($clip -and [Img]::Diff([Img]::At($pic6, 100, 150), $RED) -le 2 -and [Img]::NotColor($pic6, 600, 50, 780, 150, @(0, 0, 0), 0) -eq 0) 'and the copied image shows everything back in place'
$mEnd = Ed-Mem $ep
Tool $e $T_RECT
$n0 = Count-Trace 'editor \d+: add rect'
M-Down $e 20 400; M-Move $e 120 470
[void][KadrTest]::PostMessage($e, 0x0215, [IntPtr]::Zero, [IntPtr]::Zero) # WM_CAPTURECHANGED: e.g. alt+tab mid-drag
Check (Wait-Count 'editor \d+: add rect' ($n0 + 1) 2000) 'losing the mouse capture mid-drag finishes the shape'
Gesture $e (Line-Pts 200 400 260 460)
Check ((Last-Trace 'add rect') -match '200,400-260,460') 'and the next gesture is a new shape, not the old one'
Cmd $e $ID_UNDO; Cmd $e $ID_UNDO; Start-Sleep -Milliseconds 200

Write-Host "`n4. closing asks only when something is not saved"
[void][KadrTest]::PostMessage($e, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
Start-Sleep -Milliseconds 300
Check (-not (Trace | Select-String "editor ${ep}: unsaved changes" -Quiet) -and (Wait-Gone $ep)) 'saved state: closes without a question'
$e = Open-Region 300 250 1100 750; $ep = [KadrTest]::PidOf($e)
[void][KadrTest]::PostMessage($e, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
Check (Wait-Gone $ep) 'untouched capture: closes without a question'
$e = Open-Region 300 250 1100 750; $ep = [KadrTest]::PidOf($e)
Gesture $e (Line-Pts 10 10 60 60)
Cmd $e $ID_COPY; [void](Wait-Trace "editor ${ep}: copied" 2000)
$files0 = @(Get-ChildItem $caps -Filter *.png).Count
[void][KadrTest]::PostMessage($e, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
Check (Wait-Trace "editor ${ep}: unsaved changes, asking" 2000) 'copied but not saved: asks before closing'
Check (Answer $ep 2) 'cancel'
Start-Sleep -Milliseconds 300
Check ([bool](Get-Process -Id $ep -ErrorAction SilentlyContinue)) 'cancel keeps the editor open'
[void][KadrTest]::PostMessage($e, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
[void](Answer $ep 6)
Check ((Wait-Gone $ep) -and @(Get-ChildItem $caps -Filter *.png).Count -eq $files0 + 1) '"yes" saves, then closes'
$e = Open-Region 300 250 1100 750; $ep = [KadrTest]::PidOf($e)
Gesture $e (Line-Pts 10 10 60 60)
Cmd $e $ID_UNDO; Start-Sleep -Milliseconds 150
[void][KadrTest]::PostMessage($e, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
Check (Wait-Gone $ep) 'drawn and undone: nothing changed, no question'
$e = Open-Region 300 250 1100 750; $ep = [KadrTest]::PidOf($e)
Gesture $e (Line-Pts 10 10 60 60)
$files1 = @(Get-ChildItem $caps -Filter *.png).Count
[void][KadrTest]::PostMessage($e, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
[void](Answer $ep 7)
Check ((Wait-Gone $ep) -and @(Get-ChildItem $caps -Filter *.png).Count -eq $files1) '"no" closes without saving'

Write-Host "`n5. a long pen stroke: $Points points"
$e = Open-Region 300 250 1100 750; $ep = [KadrTest]::PidOf($e)
Tool $e $T_PEN
$before = Ed-Mem $ep
$sw = [Diagnostics.Stopwatch]::StartNew()
M-Down $e 400 250
for ($i = 1; $i -le $Points; $i++) {
  $a = $i / 40.0; $r = 20 + ($i % 3000) / 15.0
  M-Move $e ([int](400 + $r * [math]::Cos($a))) ([int](250 + $r * [math]::Sin($a) * 0.5))
  if ($i % 2000 -eq 0) { Start-Sleep -Milliseconds 50 } # keep the posted queue below its limit
}
M-Up $e 400 250
[void](Wait-Trace "editor ${ep}: add pen" 20000)
$posted = $sw.Elapsed.TotalSeconds
Start-Sleep -Milliseconds 500
$after = Ed-Mem $ep
Check ((Last-Trace "editor ${ep}: add pen") -match 'add pen') ("$Points posted moves recorded as one stroke ({0:n1} s)" -f $posted)
Check ($after.gdi -le $before.gdi + 2 -and $after.user -le $before.user + 2) "editor gdi $($before.gdi) -> $($after.gdi), user $($before.user) -> $($after.user)"
Write-Host "  editor private MiB before $($before.private), after the stroke $($after.private)"
$fl = Save-Now $e
Check ($null -ne $fl) 'the long stroke exports'
for ($k = 0; $k -lt 300; $k++) { Tool $e $T_RECT; Gesture $e (Line-Pts (10 + $k) 10 (60 + $k) 60 2) }
$many = Ed-Mem $ep
Write-Host "  after 300 more rectangles: private $($many.private) MiB, gdi $($many.gdi)"
Cmd $e $ID_SAVE; [void](Wait-Count 'editor \d+: saved' ((Count-Trace 'editor \d+: saved') + 1) 4000)
[void][KadrTest]::PostMessage($e, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
[void](Wait-Gone $ep 5000)
$stats = Last-Trace "editor ${ep}: paints"
Write-Host "  $stats"
$closed = Last-Trace "editor ${ep}: closed"
Write-Host "  $closed"

Write-Host "`n6. a full-screen capture with a large blur"
Post-Hotkey $h 3
$e = Editor-Window; $ep = [KadrTest]::PidOf($e); Start-Sleep -Milliseconds 300
$shown = Last-Trace "editor ${ep}: shown"
Tool $e $T_BLUR; Gesture $e (Line-Pts 100 100 2300 1400)
Tool $e $T_RECT; Gesture $e (Line-Pts 50 50 2500 1550)
Tool $e $T_HIGHLIGHT; Gesture $e (Line-Pts 100 1500 2400 1500 30)
$fb = Save-Now $e
$big = Ed-Mem $ep
Check ($null -ne $fb) 'full-screen export with blur, rectangle and highlighter'
if ($fb) {
  $pb = [Img]::Load($fb)
  $rb0 = [Img]::Rough($pb, 200, 200, 600, 400, $true, 0, 0); $rb1 = [Img]::Rough($pb, 200, 200, 600, 400, $false, 0, 0)
  Check ($pb.W -eq 2560 -and $rb1 -lt $rb0 / 5) ("2560x1600, blur {0:n1} -> {1:n1}" -f $rb0, $rb1)
  Check ([Img]::Outside($pb, 0, 0, @(, @(46, 46, 2504, 1554))) -eq 0) 'outside the shapes: identical'
  $seam = [Img]::ColStep($pb, 511, 200, 1300); $near = ([Img]::ColStep($pb, 509, 200, 1300) + [Img]::ColStep($pb, 513, 200, 1300)) / 2
  Check ($seam -le $near * 1.5 + 0.5) ("no blur seam at the 512 px tile edge: step {0:n2} vs {1:n2} beside it" -f $seam, $near)
}
Write-Host "  $(Last-Trace "editor ${ep}: composed")"
Write-Host "  editor while open: private $($big.private) MiB, ws $($big.ws) MiB, gdi $($big.gdi)"
[void][KadrTest]::PostMessage($e, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
[void](Wait-Gone $ep 5000)
Write-Host "  $(Last-Trace "editor ${ep}: paints")"
Write-Host "  $(Last-Trace "editor ${ep}: closed")"
Check ((@(Get-Process kadr -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $p.Id })).Count -eq 0) 'no editor process left'
Write-Host "`n  first editor: memory at open $($m0.private) MiB / gdi $($m0.gdi); after all edits $($mEnd.private) MiB / gdi $($mEnd.gdi)"
Quit $p

} finally {
  Stop-Started
  foreach ($x in @(Get-Process kadr -ErrorAction SilentlyContinue)) { if ($x.Path -eq $exe) { Stop-Process -Id $x.Id -Force -ErrorAction SilentlyContinue } }
  Restore-Clipboard $backup
  Remove-Item Env:\KADR_TEST_FRAME -ErrorAction SilentlyContinue
  Remove-Work
}
if ($failures.Count) { Write-Host "`n$($failures.Count) check(s) failed"; exit 1 }
Write-Host "`nall checks passed"
