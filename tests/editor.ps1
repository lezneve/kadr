# the editor workflow (stage 2), on a synthetic screen.
#
# KADR_TEST_FRAME=1 makes every capture a pattern computed from screen
# coordinates, so no real screen content is captured, shown or read, and the
# exported pixels can be checked exactly. nothing injects keyboard or mouse
# input: the overlay gets posted mouse messages and the editor gets posted
# WM_COMMAND, the same commands its ctrl+c / ctrl+s / esc accelerators send.
# the user's clipboard is backed up (not read) and restored.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\editor.ps1 [-Cycles 300]
param([int] $Cycles = 300)
. "$PSScriptRoot\common.ps1"
$env:KADR_TEST_FRAME = '1'
$ID_COPY = 1; $ID_SAVE = 2; $ID_CLOSE = 3; $WM_COMMAND = 0x0111
Add-Type -ReferencedAssemblies System.Drawing @'
using System; using System.Drawing; using System.Drawing.Imaging;
public static class Pattern {
  // must match capture::synthetic_pixel
  static void Px(int x, int y, out byte b, out byte g, out byte r) {
    int check = (((x >> 4) + (y >> 4)) & 1) * 120;
    b = (byte)(((byte)x) ^ check); g = (byte)((byte)y * 3); r = (byte)(check + x / 7);
  }
  // pixels of the image that differ from the pattern at screen origin (ox, oy)
  public static long Mismatches(Bitmap bmp, int ox, int oy) {
    var d = bmp.LockBits(new Rectangle(0, 0, bmp.Width, bmp.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
    long bad = 0; var row = new byte[d.Width * 4];
    for (int y = 0; y < d.Height; y++) {
      System.Runtime.InteropServices.Marshal.Copy(d.Scan0 + y * d.Stride, row, 0, row.Length);
      for (int x = 0; x < d.Width; x++) {
        byte b, g, r; Px(ox + x, oy + y, out b, out g, out r);
        if (row[x * 4] != b || row[x * 4 + 1] != g || row[x * 4 + 2] != r) bad++;
      }
    }
    bmp.UnlockBits(d); return bad;
  }
  public static long FileMismatches(string path, int ox, int oy) { using (var b = new Bitmap(path)) return Mismatches(b, ox, oy); }
}
'@
Add-Type -AssemblyName System.Windows.Forms

function Editor-Window([int] $ms = 3000) {
  $end = (Get-Date).AddMilliseconds($ms)
  while ((Get-Date) -lt $end) { $e = [KadrTest]::FindWindow('kadr.editor', [NullString]::Value); if ($e -ne [IntPtr]::Zero) { return $e }; Start-Sleep -Milliseconds 15 }
  [IntPtr]::Zero
}
function Command($e, [int] $id) { [void][KadrTest]::PostMessage($e, $WM_COMMAND, [IntPtr]$id, [IntPtr]::Zero) }
function Shown-At {
  $m = @(Trace | Select-String 'editor \d+: shown (\d+)x(\d+) at (-?\d+),(-?\d+)')[-1].Matches[0].Groups
  [pscustomobject]@{ w = [int]$m[1].Value; h = [int]$m[2].Value; x = [int]$m[3].Value; y = [int]$m[4].Value }
}
# a text marker, kept out of win+v history and cloud clipboard like every
# clipboard write these tests make
function Set-Marker {
  $d = New-Object Windows.Forms.DataObject
  $d.SetData([Windows.Forms.DataFormats]::UnicodeText, "kadr-test-marker-$PID")
  $d.SetData('ExcludeClipboardContentFromMonitorProcessing', (New-Object IO.MemoryStream(, [byte[]](0, 0, 0, 0))))
  [Windows.Forms.Clipboard]::SetDataObject($d, $true, 5, 100)
}
function Marker-Kept { [Windows.Forms.Clipboard]::ContainsText() -and [Windows.Forms.Clipboard]::GetText() -eq "kadr-test-marker-$PID" }
function Editor-Processes { @(Get-Process kadr -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $p.Id }) }
function Wait-Gone($pid_, [int] $ms = 3000) {
  $end = (Get-Date).AddMilliseconds($ms)
  while ((Get-Date) -lt $end) { if (-not (Get-Process -Id $pid_ -ErrorAction SilentlyContinue)) { return $true }; Start-Sleep -Milliseconds 15 }
  $false
}

$backup = Backup-Clipboard
Write-Host "clipboard backed up ($($backup.formats) format(s)); contents are not read or printed"
try {

Write-Host "`n1. printscreen -> overlay -> select -> editor; nothing copied or saved on its own"
$p = Start-Kadr (Settings "save_folder = $caps`r`nclose_after_copy = no`r`nclose_after_save = no`r`n") -Extra '--test-private-clipboard'
[void](Wait-Trace 'ready')
$h = Window-Of $p 'kadr.main'
$idle = Memory $p
Set-Marker
$o = Open-Overlay $p $h
Check ($o -ne [IntPtr]::Zero) 'overlay opens (editor is the default after capture)'
Start-Sleep -Milliseconds 100
Drag $o 300 250 1100 750
$e = Editor-Window
Check ($e -ne [IntPtr]::Zero) 'the editor opens after the mouse is released'
Check (Wait-Trace 'editor shown [\d.]+ ms after selection' 3000) 'kadr kept the overlay until the editor was on screen'
Start-Sleep -Milliseconds 150
Check ([KadrTest]::WindowOf([uint32]$p.Id, 'kadr.overlay') -eq [IntPtr]::Zero) 'the overlay is gone once the editor shows'
$at = Shown-At
Check ($at.w -eq 801 -and $at.h -eq 501 -and $at.x -eq 300 -and $at.y -eq 250) "editor shows the selection 1:1 in place ($($at.w)x$($at.h) at $($at.x),$($at.y))"
$fr = [KadrTest]::FrameBounds($e)
Write-Host "  editor window frame: $($fr.Left),$($fr.Top) - $($fr.Right),$($fr.Bottom)"
Start-Sleep -Milliseconds 300
Check (@(Get-ChildItem $caps -Filter *.png).Count -eq 0) 'no file written automatically'
Check (Marker-Kept) 'clipboard not touched automatically'
$edPid = [KadrTest]::PidOf($e)
Check ($edPid -ne $p.Id) "the editor is its own process ($edPid)"
$kadrAfterOpen = Memory $p

Add-Type @'
using System; using System.Runtime.InteropServices;
public static class Win5 {
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetLayeredWindowAttributes(IntPtr h, out uint key, out byte alpha, out uint flags);
}
'@
$dim = [KadrTest]::FindWindow('kadr.editor.dim', [NullString]::Value)
$key = 0; $alpha = 0; $lflags = 0
Check ($dim -ne [IntPtr]::Zero -and [Win5]::IsWindowVisible($dim)) 'the rest of the screen is darkened while the editor is open'
Check ([Win5]::GetWindow($e, 4) -eq $dim) 'the veil owns the editor, so it always lies right below it'
Check ([Win5]::GetLayeredWindowAttributes($dim, [ref]$key, [ref]$alpha, [ref]$lflags) -and $alpha -eq 116) "the same darkening as the overlay (alpha $alpha of 255)"

Write-Host "`n2. ctrl+c copies without a file"
Command $e $ID_COPY
Check (Wait-Trace "editor ${edPid}: copied 801x501" 3000) 'copy reported'
$img = [Windows.Forms.Clipboard]::GetImage()
Check ($img -and $img.Width -eq 801 -and $img.Height -eq 501) 'clipboard holds an 801x501 image'
if ($img) { $bad = [Pattern]::Mismatches([Drawing.Bitmap]$img, 300, 250); Check ($bad -eq 0) "clipboard pixels equal the captured pixels ($bad differ)"; $img.Dispose() }
Check (@(Get-ChildItem $caps -Filter *.png).Count -eq 0) 'still no file after copy'
Check (-not $p.HasExited -and (Get-Process -Id $edPid -ErrorAction SilentlyContinue)) 'editor stays open after copy'

Write-Host "`n3. ctrl+s saves without touching the clipboard"
Set-Marker
Command $e $ID_SAVE
Check (Wait-Trace "editor ${edPid}: saved " 4000) 'save reported'
$files = @(Get-ChildItem $caps -Filter *.png)
Check ($files.Count -eq 1) 'exactly one file'
if ($files.Count) {
  $bad = [Pattern]::FileMismatches($files[0].FullName, 300, 250)
  Check ((Png-Size $files[0].FullName) -eq '801x501' -and $bad -eq 0) "png is 801x501 and equals the captured pixels ($bad differ)"
}
Check (Marker-Kept) 'clipboard unchanged by save'
Command $e $ID_SAVE
Check (Wait-Trace "editor ${edPid}: already saved" 3000) 'a second ctrl+s without changes writes nothing new'
Check (@(Get-ChildItem $caps -Filter *.png).Count -eq 1) 'still exactly one file'
Check ([bool](Get-Process -Id $edPid -ErrorAction SilentlyContinue)) 'editor stays open after save'
$edMem = Get-Process -Id $edPid | ForEach-Object { [math]::Round($_.PrivateMemorySize64 / 1MB, 1) }

Write-Host "`n4. esc closes without exporting"
Get-ChildItem $caps -File | Remove-Item -Force
Set-Marker
Command $e $ID_CLOSE
Check (Wait-Gone $edPid) 'editor process exits'
Check (Wait-Trace "editor ${edPid}: closed" 2000) 'editor reports a clean close'
Check ([KadrTest]::FindWindow('kadr.editor.dim', [NullString]::Value) -eq [IntPtr]::Zero) 'the darkening goes with the editor'
Check (@(Get-ChildItem $caps -Filter *.png).Count -eq 0 -and (Marker-Kept)) 'closing exported nothing'
$o = Open-Overlay $p $h; Start-Sleep -Milliseconds 100; Drag $o 10 10 200 120
$e = Editor-Window; $p2 = [KadrTest]::PidOf($e)
[void][KadrTest]::PostMessage($e, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
Check (Wait-Gone $p2) 'WM_CLOSE (alt+f4, taskbar close) also ends the editor process'

Write-Host "`n5. window and screen captures open the editor too"
$before = Count-Trace 'editor \d+: shown'
Post-Hotkey $h 3
$e = Editor-Window; $ps = [KadrTest]::PidOf($e); [void](Wait-Count 'editor \d+: shown' ($before + 1) 3000); $s3 = Shown-At
Check ($e -ne [IntPtr]::Zero -and $s3.w -gt 0) "screen capture opens the editor ($($s3.w)x$($s3.h))"
Command $e $ID_CLOSE; [void](Wait-Gone $ps)
Post-Hotkey $h 2
$e = Editor-Window; $pw = [KadrTest]::PidOf($e)
Check ($e -ne [IntPtr]::Zero) 'window capture opens the editor'
Command $e $ID_CLOSE; [void](Wait-Gone $pw)
Check ((Editor-Processes).Count -eq 0) 'no editor process left'

Write-Host "`n6. several editors at once"
$pids = @()
foreach ($r in @(@(20, 20, 300, 200), @(400, 300, 700, 500), @(800, 100, 1000, 400))) {
  $before = Count-Trace 'editor \d+: shown'
  $o = Open-Overlay $p $h; Start-Sleep -Milliseconds 80; Drag $o $r[0] $r[1] $r[2] $r[3]
  [void](Wait-Count 'editor \d+: shown' ($before + 1) 3000)
}
$eds = Editor-Processes
Check ($eds.Count -eq 3) "three editors run side by side ($($eds.Count))"
foreach ($x in $eds) { $w = [KadrTest]::WindowOf([uint32]$x.Id, 'kadr.editor'); Command $w $ID_CLOSE }
Start-Sleep -Milliseconds 800
Check ((Editor-Processes).Count -eq 0) 'all three closed, nothing left'
Quit $p

Write-Host "`n6b. by default ctrl+c copies and closes, without a question"
Quit $p
$p = Start-Kadr (Settings "save_folder = $caps`r`n") -Extra '--test-private-clipboard'
[void](Wait-Trace 'ready'); $h = Window-Of $p 'kadr.main'
Post-Hotkey $h 3
$e = Editor-Window; $ed1 = [KadrTest]::PidOf($e); Start-Sleep -Milliseconds 300
[void][KadrTest]::PostPhysical($e, 0x0201, [IntPtr]1, [IntPtr]((20 -shl 16) -bor 20))
[void][KadrTest]::PostPhysical($e, 0x0200, [IntPtr]1, [IntPtr]((80 -shl 16) -bor 80))
[void][KadrTest]::PostPhysical($e, 0x0202, [IntPtr]0, [IntPtr]((80 -shl 16) -bor 80))
Start-Sleep -Milliseconds 200
Command $e $ID_COPY
Check ((Wait-Gone $ed1) -and (Wait-Trace "editor ${ed1}: closing after copy" 1000)) 'a drawn capture: copied, then the editor is gone'
Check (-not (Trace | Select-String "editor ${ed1}: unsaved changes" -Quiet)) 'no save question after copying'
$img = [Windows.Forms.Clipboard]::GetImage()
Check ($img -and $img.Width -eq 2560) 'the copy is on the clipboard'
if ($img) { $img.Dispose() }
Write-Host "`n6c. ctrl+a in the overlay takes the whole monitor"
$o = Open-Overlay $p $h; Start-Sleep -Milliseconds 120
[void][KadrTest]::PostMessage($o, 0x0100, [IntPtr]0x41, [IntPtr]::Zero) # 'a' (ctrl is read from the keyboard state)
Start-Sleep -Milliseconds 300
Check (-not (Trace | Select-String 'overlay: ctrl\+a' -Quiet) -and [KadrTest]::FindWindow('kadr.editor', [NullString]::Value) -eq [IntPtr]::Zero) 'a without ctrl does nothing (a posted key carries no ctrl; ctrl+a itself is a manual check)'
[void][KadrTest]::PostMessage($o, 0x0100, [IntPtr]0x1B, [IntPtr]::Zero)
Start-Sleep -Milliseconds 300
Quit $p

Write-Host "`n7. the quick modes stay: no editor"
$p = Start-Kadr (Settings "after_capture = copy+save`r`nsave_folder = $caps`r`nsubfolder = %Y-%m`r`n") -Extra '--test-private-clipboard'
[void](Wait-Trace 'ready'); $h = Window-Of $p 'kadr.main'
Post-Hotkey $h 3
Check (Wait-Trace 'saved ' 4000) 'copy+save: saved right away'
$month = Join-Path $caps (Get-Date -Format 'yyyy-MM')
Check (@(Get-ChildItem $month -Filter *.png -ErrorAction SilentlyContinue).Count -eq 1) "subfolder = %Y-%m: the file is in $(Split-Path $month -Leaf)"
Remove-Item $month -Recurse -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300
Check ((Editor-Processes).Count -eq 0 -and [KadrTest]::FindWindow('kadr.editor', [NullString]::Value) -eq [IntPtr]::Zero) 'copy+save: no editor'
Quit $p
Get-ChildItem $caps -File | Remove-Item -Force

Write-Host "`n8. $Cycles editor opens and closes"
$p = Start-Kadr (Settings "save_folder = $caps`r`nclose_after_copy = no`r`nclose_after_save = no`r`n") -Extra '--test-private-clipboard'
[void](Wait-Trace 'ready'); $h = Window-Of $p 'kadr.main'
$m0 = Memory $p
$sw = [Diagnostics.Stopwatch]::StartNew(); $ok = 0; $peakEd = 0.0; $peakWs = 0.0
for ($i = 1; $i -le $Cycles; $i++) {
  if ($i % 10 -eq 0) {
    # every tenth cycle through the overlay (region), the rest screen/window
    $o = Open-Overlay $p $h; Start-Sleep -Milliseconds 30; Drag $o 100 100 900 600 4 0
  } else { Post-Hotkey $h (2 + ($i % 2)) }
  $e = Editor-Window 4000
  if ($e -eq [IntPtr]::Zero) { Write-Host "  cycle ${i}: no editor"; break }
  $ep = [KadrTest]::PidOf($e)
  if ($i % 25 -eq 0) { Command $e $ID_COPY; [void](Wait-Trace "editor ${ep}: copied" 2000) }
  $proc = Get-Process -Id $ep -ErrorAction SilentlyContinue
  if ($proc) { $peakEd = [math]::Max($peakEd, $proc.PrivateMemorySize64 / 1MB) }
  Command $e $ID_CLOSE
  if (-not (Wait-Gone $ep 4000)) { Write-Host "  cycle ${i}: editor $ep did not exit"; break }
  $ok++
  # the baseline is taken after the first overlay cycle: the first overlay
  # loads system ui pieces once (theme, ime), as in every earlier stage.
  if ($i -eq 20) { $m1 = Memory $p }
}
$elapsed = $sw.Elapsed.TotalSeconds
Start-Sleep -Milliseconds 500
$mN = Memory $p
Check ($ok -eq $Cycles) "$ok of $Cycles cycles opened and closed the editor ($([math]::Round($elapsed / $Cycles * 1000)) ms per cycle)"
Check ((Editor-Processes).Count -eq 0) 'no editor process left behind'
Check ($mN.private -le $m1.private + 1.0) "kadr private MiB: idle $($m0.private), after 20th $($m1.private), after ${Cycles}th $($mN.private)"
Check ($mN.handles -le $m1.handles + 10) "kadr handles: after 20th $($m1.handles), after ${Cycles}th $($mN.handles) (no leaked sections or events)"
Check ($mN.gdi -le $m1.gdi + 2 -and $mN.user -le $m1.user + 2) "kadr gdi $($m1.gdi) -> $($mN.gdi), user $($m1.user) -> $($mN.user)"
$closes = Trace | Select-String 'editor \d+: closed, .*memory ws [\d.]+ MiB \(peak ([\d.]+)\), private [\d.]+ MiB \(peak ([\d.]+)\)' | ForEach-Object { [pscustomobject]@{ ws = [double]$_.Matches[0].Groups[1].Value; priv = [double]$_.Matches[0].Groups[2].Value } }
$shownMs = Trace | Select-String 'editor shown ([\d.]+) ms after selection' | ForEach-Object { [double]$_.Matches[0].Groups[1].Value } | Sort-Object
$results = [ordered]@{
  kadr_idle = $idle; kadr_with_editor_open = $kadrAfterOpen; kadr_loop_start = $m0; kadr_after_first = $m1; kadr_after_all = $mN
  editor_private_mb_801x501_after_save = $edMem
  editor_peak_private_mb_sampled = [math]::Round($peakEd, 1)
  editor_peak_ws_mb_max = ($closes | Measure-Object ws -Maximum).Maximum
  editor_peak_private_mb_max = ($closes | Measure-Object priv -Maximum).Maximum
  region_release_to_editor_ms_p50 = if ($shownMs) { $shownMs[[int]($shownMs.Count / 2)] } else { $null }
  region_release_to_editor_ms_max = if ($shownMs) { $shownMs[-1] } else { $null }
  editor_got_foreground = "$((Trace | Select-String 'foreground True').Count) of $((Trace | Select-String 'editor \d+: shown').Count) (posted hotkeys; a real key gives kadr the foreground)"
  editor_start_ms = (Trace | Select-String 'shown \d+x\d+ at -?\d+,-?\d+ ([\d.]+) ms after process start' | ForEach-Object { [double]$_.Matches[0].Groups[1].Value } | Measure-Object -Average -Maximum | Select-Object Count, Average, Maximum)
}
$results | ConvertTo-Json -Depth 4
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
