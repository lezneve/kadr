# editor colour, width and eyedropper (stage 4), on a synthetic screen.
#
# nothing injects real input: the editor gets posted mouse messages in its
# own client pixels (= image pixels) and posted WM_COMMAND / WM_SET_COLOR,
# the same commands its keys, toolbar and colour dialog end in. exports are
# checked pixel by pixel. the clipboard is backed up (not read) and restored.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\style.ps1 [-Out <folder for screenshots>]
param([string] $Out = '')
. "$PSScriptRoot\common.ps1"
$env:KADR_TEST_FRAME = '1'
$WM_COMMAND = 0x0111; $MK_LBUTTON = 1; $WM_SET_COLOR = 0x8005; $WM_MOUSEWHEEL = 0x020A
$ID_SAVE = 2; $ID_UNDO = 10; $ID_REDO = 11; $ID_STYLE = 14; $ID_PICK = 15; $ID_WIDTH_UP = 18; $ID_TOOL = 100; $ID_PRESET = 300; $ID_WIDTH = 400
$T_SELECT = 0; $T_PEN = 1; $T_LINE = 2; $T_RECT = 4; $T_HIGHLIGHT = 6; $T_REDACT = 8
$P_RED = 0; $P_GREEN = 3; $P_BLUE = 4; $P_WHITE = 6
Add-Type -ReferencedAssemblies System.Drawing @'
using System; using System.Drawing; using System.Drawing.Imaging;
public static class Img2 {
  public static int[] Px(int x, int y) {
    int check = (((x >> 4) + (y >> 4)) & 1) * 120;
    return new int[] { (byte)(check + x / 7), (byte)((byte)y * 3), (byte)(((byte)x) ^ check) };
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
  public static long NotColor(Pic p, int l, int t, int r, int b, int[] c, int tol) {
    long bad = 0; for (int y = t; y < b; y++) for (int x = l; x < r; x++) if (Diff(At(p, x, y), c) > tol) bad++; return bad;
  }
  public static int[] Hex(int h) { return new[] { (h >> 16) & 255, (h >> 8) & 255, h & 255 }; }
  [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
}
'@
Add-Type -AssemblyName System.Windows.Forms

function Editor-Window([int] $ms = 3000) {
  $end = (Get-Date).AddMilliseconds($ms)
  while ((Get-Date) -lt $end) { $e = [KadrTest]::FindWindow('kadr.editor', [NullString]::Value); if ($e -ne [IntPtr]::Zero) { return $e }; Start-Sleep -Milliseconds 15 }
  [IntPtr]::Zero
}
function Cmd($e, [int] $id) { [void][KadrTest]::PostMessage($e, $WM_COMMAND, [IntPtr]$id, [IntPtr]::Zero); Start-Sleep -Milliseconds 40 }
function Tool($e, [int] $t) { Cmd $e ($ID_TOOL + $t) }
function M-Down($e, $x, $y) { [void][KadrTest]::PostPhysical($e, $WM_LBUTTONDOWN, [IntPtr]$MK_LBUTTON, (Pack-Point $x $y)) }
function M-Move($e, $x, $y) { [void][KadrTest]::PostPhysical($e, $WM_MOUSEMOVE, [IntPtr]$MK_LBUTTON, (Pack-Point $x $y)) }
function M-Up($e, $x, $y) { [void][KadrTest]::PostPhysical($e, $WM_LBUTTONUP, [IntPtr]0, (Pack-Point $x $y)) }
function Gesture($e, $x0, $y0, $x1, $y1, [string] $expect = 'editor \d+: (add|change) ') {
  $before = Count-Trace $expect
  M-Down $e $x0 $y0
  for ($i = 1; $i -le 6; $i++) { M-Move $e ([int]($x0 + ($x1 - $x0) * $i / 6)) ([int]($y0 + ($y1 - $y0) * $i / 6)) }
  M-Up $e $x1 $y1
  [void](Wait-Count $expect ($before + 1) 3000)
}
function Click-At($e, $x, $y) { M-Down $e $x $y; M-Up $e $x $y; Start-Sleep -Milliseconds 120 }
function Last-Trace([string] $pattern) { $m = @(Trace | Select-String $pattern); if ($m.Count) { $m[-1].Line } else { '' } }
function Save-Now($e) {
  $n = Count-Trace 'editor \d+: (saved|already saved)'
  $before = @(Get-ChildItem $caps -Filter *.png).Count
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
function Wait-Gone($procId, [int] $ms = 4000) {
  $end = (Get-Date).AddMilliseconds($ms)
  while ((Get-Date) -lt $end) { if (-not (Get-Process -Id $procId -ErrorAction SilentlyContinue)) { return $true }; Start-Sleep -Milliseconds 20 }
  $false
}
function Open-Region {
  $before = Count-Trace 'editor \d+: shown'
  $o = Open-Overlay $p $h; Start-Sleep -Milliseconds 100; Drag $o 300 250 1100 750
  [void](Wait-Count 'editor \d+: shown' ($before + 1) 3000)
  $e = Editor-Window; Start-Sleep -Milliseconds 150
  if ((Last-Trace 'editor \d+: shown') -notmatch 'shown 801x501 at 300,250') { Write-Host "  (editor opened with another size: $(Last-Trace 'editor \d+: shown'))" }
  $e
}
function Ed-Mem($procId) {
  $x = Get-Process -Id $procId -ErrorAction SilentlyContinue; if (-not $x) { return $null }
  [pscustomobject]@{ private = [math]::Round($x.PrivateMemorySize64 / 1MB, 1); gdi = [KadrTest]::GetGuiResources($x.Handle, 0); user = [KadrTest]::GetGuiResources($x.Handle, 1) }
}
$OX = 300; $OY = 250
$stylesFile = Join-Path $work 'editor.ini'

$backup = Backup-Clipboard
Write-Host "clipboard backed up ($($backup.formats) format(s)); contents are not read or printed"
try {

$p = Start-Kadr (Settings "save_folder = $caps`r`nclose_after_copy = no`r`nclose_after_save = no`r`n") -Extra '--test-private-clipboard'
[void](Wait-Trace 'ready'); $h = Window-Of $p 'kadr.main'

Write-Host "`n1. colours and widths reach the exported pixels"
Check (-not (Test-Path $stylesFile)) 'no editor.ini before the first style change'
$e = Open-Region; $ep = [KadrTest]::PidOf($e)
Check ((Last-Trace "editor ${ep}: styles") -match 'stroke #ff3b30 4, highlighter #ffd60a 18, redact #000000') 'first editor starts with the default styles'
$m0 = Ed-Mem $ep
Cmd $e ($ID_PRESET + $P_BLUE); Cmd $e ($ID_WIDTH + 10)
Check ((Last-Trace 'editor \d+: style stroke') -match 'stroke #0a84ff 10') 'preset blue, width 10'
Gesture $e 100 100 300 200
Cmd $e ($ID_WIDTH + 1); Tool $e $T_LINE; Gesture $e 300 300 500 300
Cmd $e ($ID_WIDTH + 32); Cmd $e $ID_WIDTH_UP
Check ((Last-Trace 'editor \d+: style stroke') -match 'stroke #0a84ff 32') 'width stops at 32'
[void][KadrTest]::PostMessage($e, $WM_SET_COLOR, [IntPtr]0x123456, [IntPtr]::Zero); Start-Sleep -Milliseconds 60
Cmd $e ($ID_WIDTH + 8); Tool $e $T_PEN; Gesture $e 50 450 350 450
Tool $e $T_HIGHLIGHT; Cmd $e ($ID_PRESET + $P_GREEN); Cmd $e ($ID_WIDTH + 24); Gesture $e 550 300 760 300
Check ((Last-Trace 'editor \d+: style highlighter') -match 'highlighter #34c759 24') 'the highlighter keeps its own colour and width'
Tool $e $T_REDACT; Cmd $e ($ID_PRESET + $P_WHITE); Gesture $e 600 50 780 150
Check ((Last-Trace 'editor \d+: style redact') -match 'redact #ffffff') 'redact has its own colour'
Tool $e $T_RECT
$f1 = Save-Now $e
$pic = [Img2]::Load($f1)
$blue = [Img2]::Hex(0x0a84ff); $custom = [Img2]::Hex(0x123456)
$at = { param($x, $y) [Img2]::At($pic, $x, $y) }
$src = { param($x, $y) [Img2]::Px($OX + $x, $OY + $y) }
Check ([Img2]::Diff((& $at 100 150), $blue) -le 1) "rectangle edge in the preset blue ($((& $at 100 150) -join ','))"
Check ([Img2]::Diff((& $at 95 150), $blue) -le 1 -and [Img2]::Diff((& $at 104 150), $blue) -le 1) 'width 10: pixels 95..104 are covered'
Check ([Img2]::Diff((& $at 94 150), (& $src 94 150)) -eq 0 -and [Img2]::Diff((& $at 105 150), (& $src 105 150)) -eq 0) 'and 94 and 105 are not'
$l1 = [Img2]::Diff((& $at 400 299), (& $src 400 299)); $l2 = [Img2]::Diff((& $at 400 301), (& $src 400 301))
Check ($l1 -gt 0 -and $l2 -eq 0) "width 1 line touches only the rows around it (row 299 changed $l1, row 301 $l2)"
Check ([Img2]::Diff((& $at 200 450), $custom) -le 1) "any colour (#123456) is exact ($((& $at 200 450) -join ','))"
$s = & $src 650 300; $g = [Img2]::Hex(0x34c759); $want = @(0, 1, 2 | ForEach-Object { [int]($s[$_] * 0.6 + $g[$_] * 0.4) })
Check ([Img2]::Diff((& $at 650 300), $want) -le 2) "highlighter: 40 % green over the image ($((& $at 650 300) -join ',') vs $($want -join ','))"
$s2 = & $src 650 289; $want2 = @(0, 1, 2 | ForEach-Object { [int]($s2[$_] * 0.6 + $g[$_] * 0.4) })
Check ([Img2]::Diff((& $at 650 289), $want2) -le 2 -and [Img2]::Diff((& $at 650 312), (& $src 650 312)) -eq 0) 'highlighter width 24: rows 288..311'
Check ([Img2]::NotColor($pic, 600, 50, 780, 150, @(255, 255, 255), 0) -eq 0) 'redact in white: every pixel white'

Write-Host "`n2. eyedropper takes the colour of the capture"
Cmd $e $ID_PICK
Check (Wait-Trace "editor ${ep}: eyedropper" 2000) 'eyedropper on'
Click-At $e 700 450
$want = [Img2]::Px($OX + 700, $OY + 450); $hex = '{0:x2}{1:x2}{2:x2}' -f $want[0], $want[1], $want[2]
Check ((Last-Trace "editor ${ep}: picked") -match "picked #$hex at 700,450") "picked the capture's pixel (#$hex)"
Check ((Last-Trace "editor ${ep}: style stroke") -match "#$hex") 'it becomes the stroke colour'
Check ((Count-Trace "editor ${ep}: add") -eq 5) 'the eyedropper click drew nothing'
Tool $e $T_LINE; Gesture $e 400 400 400 480
$f2 = Save-Now $e; $pic2 = [Img2]::Load($f2)
Check ([Img2]::Diff([Img2]::At($pic2, 400, 440), $want) -le 1) 'a line in the picked colour'
Cmd $e $ID_PICK; Cmd $e 13                                  # esc cancels the eyedropper
Click-At $e 50 50
Check ((Last-Trace "editor ${ep}: picked") -match "at 700,450") 'esc cancels the eyedropper'

Write-Host "`n3. restyle a shape through select; undo, redo, save"
$states = Count-Trace "editor ${ep}: restyle"
Tool $e $T_SELECT; Click-At $e 100 150
Cmd $e ($ID_PRESET + $P_RED); Cmd $e ($ID_WIDTH + 20)
Check ((Count-Trace "editor ${ep}: restyle") -eq $states + 2) 'colour and width of the selected rectangle: two edits'
$f3 = Save-Now $e; $pic3 = [Img2]::Load($f3)
$red = [Img2]::Hex(0xff3b30)
Check ([Img2]::Diff([Img2]::At($pic3, 100, 150), $red) -le 1 -and [Img2]::Diff([Img2]::At($pic3, 91, 150), $red) -le 1) 'export: red, 20 wide'
Check ((Last-Trace "editor ${ep}: style stroke") -match '#ff3b30 20') 'and it is the stroke style from now on'
for ($i = 0; $i -lt 3; $i++) { [void][KadrTest]::PostMessage([KadrTest]::FindWindow('kadr.editor.bar', [NullString]::Value), $WM_MOUSEWHEEL, [IntPtr](120 -shl 16), [IntPtr]::Zero) }
Start-Sleep -Milliseconds 200
Check ((Last-Trace "editor ${ep}: restyle") -match '#ff3b30 23') 'three wheel steps: width 23'
Cmd $e $ID_UNDO; Start-Sleep -Milliseconds 100
Cmd $e 1; [void](Wait-Count "editor ${ep}: copied" 1 2000); Start-Sleep -Milliseconds 200
$clip = [Windows.Forms.Clipboard]::GetImage(); $cp = Join-Path $work 'clip.png'; $clip.Save($cp, [Drawing.Imaging.ImageFormat]::Png); $clip.Dispose(); $pc = [Img2]::Load($cp)
Check ([Img2]::Diff([Img2]::At($pc, 90, 150), $red) -le 1 -and [Img2]::Diff([Img2]::At($pc, 89, 150), [Img2]::Px($OX + 89, $OY + 150)) -eq 0) 'one undo takes back all three wheel steps (20 wide again)'
$f4 = Save-Now $e
Check ($null -eq $f4) 'that state was saved before: no new file'
Cmd $e $ID_UNDO; Cmd $e $ID_UNDO; Start-Sleep -Milliseconds 100
Check ($null -eq (Save-Now $e)) 'two more undos: back to the state of the first file, nothing new to save'
Cmd $e 1; Start-Sleep -Milliseconds 300
$clip = [Windows.Forms.Clipboard]::GetImage(); $clip.Save($cp, [Drawing.Imaging.ImageFormat]::Png); $clip.Dispose(); $pic5 = [Img2]::Load($cp)
Check ([Img2]::Diff([Img2]::At($pic5, 100, 150), $blue) -le 1 -and [Img2]::Diff([Img2]::At($pic5, 94, 150), [Img2]::Px($OX + 94, $OY + 150)) -eq 0) 'and the copy shows blue, 10 wide again'
Cmd $e $ID_REDO; Cmd $e $ID_REDO; Start-Sleep -Milliseconds 100
Check ($null -eq (Save-Now $e)) 'redo back to the red 20: already saved'
$mBusy = Ed-Mem $ep
for ($i = 0; $i -lt 200; $i++) { Cmd $e ($ID_PRESET + ($i % 7)) }
Start-Sleep -Milliseconds 300
$mMany = Ed-Mem $ep
Check ($mMany.gdi -le $mBusy.gdi + 2 -and $mMany.user -le $mBusy.user + 2) "200 colour changes: gdi $($mBusy.gdi) -> $($mMany.gdi), user $($mBusy.user) -> $($mMany.user)"
Write-Host "  editor private MiB: open $($m0.private), after the edits $($mBusy.private), after 200 colour changes $($mMany.private)"

Write-Host "`n4. the style panel"
Cmd $e $ID_STYLE; Start-Sleep -Milliseconds 300
$fly = [KadrTest]::FindWindow('kadr.editor.style', [NullString]::Value)
Check ($fly -ne [IntPtr]::Zero -and (Wait-Trace "editor ${ep}: style panel" 1000)) 'opens next to the toolbar'
if ($Out -and $fly -ne [IntPtr]::Zero) { New-Item -ItemType Directory -Force $Out | Out-Null; [KadrTest]::SaveWindow($fly, (Join-Path $Out 'style-panel.png')); [KadrTest]::SaveWindow([KadrTest]::FindWindow('kadr.editor.bar', [NullString]::Value), (Join-Path $Out 'style-bar.png')) }
Tool $e $T_REDACT; Start-Sleep -Milliseconds 200
if ($Out -and $fly -ne [IntPtr]::Zero) { [KadrTest]::SaveWindow($fly, (Join-Path $Out 'style-panel-redact.png')) }
Cmd $e 13; Start-Sleep -Milliseconds 200
Check (-not [Img2]::IsWindowVisible($fly)) 'esc closes it'

Write-Host "`n5. remembered in the next editor"
[void][KadrTest]::PostMessage($e, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
[void](Answer $ep 7)
Check (Wait-Gone $ep) 'editor closed'
$text = Get-Content $stylesFile -Raw
Write-Host "  editor.ini: $($text -replace "`r`n", ' | ')"
Check ($text -match 'stroke = #[0-9a-f]{6} 20' -and $text -match 'highlighter = #34c759 24' -and $text -match 'redact = #ffffff') 'editor.ini holds the last colour and width of each slot'
$e = Open-Region; $ep2 = [KadrTest]::PidOf($e)
Check ((Last-Trace "editor ${ep2}: styles") -match 'highlighter #34c759 24, redact #ffffff') 'the next editor starts with them'
[void][KadrTest]::PostMessage($e, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
Check (Wait-Gone $ep2) 'an untouched editor closes without a question'
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
