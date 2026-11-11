# checks kadr in a normal (non-elevated) user session.
#
# - kadr is started through explorer, so it gets the normal user token even
#   if this script runs elevated, and the script verifies that.
# - no keyboard or mouse input is injected; kadr is driven by posting window
#   messages to its own windows.
# - the clipboard is backed up first and restored at the end; test copies are
#   kept out of clipboard history with --test-private-clipboard.
# - captures go to a temp folder that is deleted at the end.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\session.ps1
. "$PSScriptRoot\common.ps1"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

function Backup-Clipboard {
  $data = [Windows.Forms.Clipboard]::GetDataObject()
  $copy = New-Object Windows.Forms.DataObject
  $n = 0
  if ($data) {
    foreach ($f in $data.GetFormats($false)) {
      try { $v = $data.GetData($f); if ($null -ne $v) { $copy.SetData($f, $v); $n++ } } catch { }
    }
  }
  [pscustomobject]@{ data = $copy; formats = $n }
}
function Restore-Clipboard($backup) {
  # the restored copy is kept out of clipboard history too, so the user's
  # history does not get a duplicate entry.
  $backup.data.SetData('ExcludeClipboardContentFromMonitorProcessing', (New-Object IO.MemoryStream(, [byte[]](0, 0, 0, 0))))
  if ($backup.formats -gt 0) { [Windows.Forms.Clipboard]::SetDataObject($backup.data, $true, 5, 100) }
  else { [Windows.Forms.Clipboard]::Clear() }
}
function Clipboard-Formats { $d = [Windows.Forms.Clipboard]::GetDataObject(); if ($d) { @($d.GetFormats($false)) } else { @() } }
function Clipboard-Dword([string] $format) {
  $v = [Windows.Forms.Clipboard]::GetData($format)
  if ($v -is [IO.MemoryStream]) { $b = $v.ToArray(); if ($b.Length -ge 4) { return [BitConverter]::ToUInt32($b, 0) } }
  $null
}
# true when the clipboard image and the png have identical pixels
function Same-Pixels([string] $png) {
  $clip = [Windows.Forms.Clipboard]::GetImage()
  if (-not $clip) { return $false }
  $file = New-Object Drawing.Bitmap $png
  try {
    if ($clip.Width -ne $file.Width -or $clip.Height -ne $file.Height) { return $false }
    $rect = New-Object Drawing.Rectangle 0, 0, $file.Width, $file.Height
    $fmt = [Drawing.Imaging.PixelFormat]::Format24bppRgb
    $a = ([Drawing.Bitmap]$clip).LockBits($rect, 'ReadOnly', $fmt); $b = $file.LockBits($rect, 'ReadOnly', $fmt)
    $len = $a.Stride * $a.Height
    $x = New-Object byte[] $len; $y = New-Object byte[] $len
    [Runtime.InteropServices.Marshal]::Copy($a.Scan0, $x, 0, $len); [Runtime.InteropServices.Marshal]::Copy($b.Scan0, $y, 0, $len)
    ([Drawing.Bitmap]$clip).UnlockBits($a); $file.UnlockBits($b)
    [Linq.Enumerable]::SequenceEqual([byte[]]$x, [byte[]]$y)
  } finally { $file.Dispose(); $clip.Dispose() }
}

$backup = Backup-Clipboard
Write-Host "clipboard backed up ($($backup.formats) format(s)); contents are not read or printed"
try {

Write-Host "`n1. normal user token"
$p = Start-Kadr (Settings "after_capture = copy+save`r`nsave_folder = $caps`r`n") -Unelevated -Extra '--test-private-clipboard'
Check (Wait-Trace 'start, memory' 5000) 'starts with a standard-user token (would fail with ERROR_ELEVATION_REQUIRED if kadr demanded admin)'
$token = [KadrTest]::TokenInfo([uint32]$p.Id)
Write-Host "  kadr: $token; this script: $([KadrTest]::TokenInfo([uint32]$PID))"
Check ($token -eq 'elevated=False integrity=0x2000') 'kadr runs as a normal user (not elevated, medium integrity)'
# with uac off, explorer is elevated and uipi blocks a standard-user process
# from adding a tray icon. that cannot happen in a normal session (explorer
# runs at medium too), so here the check is that kadr says so clearly.
$uacOff = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System').EnableLUA -eq 0
if ($uacOff) {
  Check (Close-Dialog $p) 'uac is off on this machine: the missing tray icon is reported in a dialog'
  Check (Wait-Trace 'tray icon could not be added' 2000) 'the message explains hotkeys still work and how to quit'
} else {
  Check (-not (Trace | Select-String '^\[kadr\] tray:' -Quiet)) 'tray icon added'
}
Check (-not (Hotkey-Free ($MOD_ALT -bor $MOD_NOREPEAT) $VK_PRTSC)) 'default hotkey registered'
$h = Window-Of $p 'kadr.main'

Write-Host "`n2. copy+save: screen"
Post-Hotkey $h 3
Check (Wait-Count 'saved ' 1) 'saved'
Check (Wait-Trace 'clipboard [\d.]+ ms' 1000) 'copied'
$f = @(Get-ChildItem $caps -Filter *.png)[0]
$formats = Clipboard-Formats
Check ($formats -contains 'DeviceIndependentBitmap') 'clipboard holds an image (CF_DIB)'
Check ((Clipboard-Dword 'CanUploadToCloudClipboard') -eq 0) 'marked: not for cloud clipboard'
Check ($formats -contains 'ExcludeClipboardContentFromMonitorProcessing') 'test copy kept out of history'
Check (Same-Pixels $f.FullName) "clipboard and file have identical pixels ($(Png-Size $f.FullName))"

Write-Host "`n3. copy+save: region"
Get-ChildItem $caps -File | Remove-Item -Force
$o = Open-Overlay $p $h
Check ($o -ne [IntPtr]::Zero) 'overlay opens'
Start-Sleep -Milliseconds 100
Drag $o 300 250 1100 750
Check (Wait-Count 'saved ' 2) 'region saved'
$f = @(Get-ChildItem $caps -Filter *.png)[0]
Check ($f -and (Png-Size $f.FullName) -eq '801x501') 'file is 801x501'
Check (Same-Pixels $f.FullName) 'clipboard and file match'

Write-Host "`n4. copy+save: window"
Get-ChildItem $caps -File | Remove-Item -Force
Post-Hotkey $h 2
Check (Wait-Count 'saved ' 3) 'window saved'
$f = @(Get-ChildItem $caps -Filter *.png)[0]
Check ($f -and (Same-Pixels $f.FullName)) "clipboard and file match ($(if ($f) { Png-Size $f.FullName }))"

Write-Host "`n5. tray"
[void][KadrTest]::PostMessage($h, $WM_TRAY, [IntPtr]::Zero, [IntPtr]0x0400) # NIN_SELECT
$o = Window-Of $p 'kadr.overlay'
Check ($o -ne [IntPtr]::Zero) 'left click opens the overlay'
[void][KadrTest]::PostMessage($o, $WM_KEYDOWN, [IntPtr]0x1B, [IntPtr]::Zero)
Check (Wait-Count '^\[kadr\] cancelled' 1 2000) 'esc cancels'

# WM_CONTEXTMENU (0x007B) in the low word, anchor point in wparam
[void][KadrTest]::PostMessage($h, $WM_TRAY, (Pack-Point 1200 900), [IntPtr]0x007B)
$menu = Window-Of $p '#32768'
Check ($menu -ne [IntPtr]::Zero) 'right click shows the menu'
[void][KadrTest]::PostMessage($h, 0x001F, [IntPtr]::Zero, [IntPtr]::Zero) # WM_CANCELMODE
Start-Sleep -Milliseconds 300
Check (([KadrTest]::WindowOf([uint32]$p.Id, '#32768')) -eq [IntPtr]::Zero) 'menu closes when dismissed'

# keyboard in the menu: first item (capture region) with enter
[void][KadrTest]::PostMessage($h, $WM_TRAY, (Pack-Point 1200 900), [IntPtr]0x007B)
$menu = Window-Of $p '#32768'
Start-Sleep -Milliseconds 150
[void][KadrTest]::PostMessage($menu, $WM_KEYDOWN, [IntPtr]0x28, [IntPtr]::Zero) # down: highlight first item
[void][KadrTest]::PostMessage($menu, $WM_KEYDOWN, [IntPtr]0x0D, [IntPtr]::Zero) # enter
$o = Window-Of $p 'kadr.overlay'
Check ($o -ne [IntPtr]::Zero) 'menu "capture region" opens the overlay after the menu is gone'
if ($o -ne [IntPtr]::Zero) { [void][KadrTest]::PostMessage($o, $WM_KEYDOWN, [IntPtr]0x1B, [IntPtr]::Zero) }
Start-Sleep -Milliseconds 300

# keyboard in the menu: last item (quit)
[void][KadrTest]::PostMessage($h, $WM_TRAY, (Pack-Point 1200 900), [IntPtr]0x007B)
$menu = Window-Of $p '#32768'
Start-Sleep -Milliseconds 150
[void][KadrTest]::PostMessage($menu, $WM_KEYDOWN, [IntPtr]0x26, [IntPtr]::Zero) # up: wraps to the last item
[void][KadrTest]::PostMessage($menu, $WM_KEYDOWN, [IntPtr]0x0D, [IntPtr]::Zero)
$p.WaitForExit(5000) | Out-Null
Check ($p.HasExited -and $p.ExitCode -eq 0) 'menu "quit" exits with code 0'
Check (Hotkey-Free ($MOD_ALT -bor $MOD_NOREPEAT) $VK_PRTSC) 'hotkey released after quit'

} finally {
  Stop-Started
  Restore-Clipboard $backup
  Write-Host "`nclipboard restored"
  Remove-Work
}
if ($failures.Count) { Write-Host "`n$($failures.Count) check(s) failed"; exit 1 }
Write-Host "`nall checks passed"
