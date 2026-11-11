# real-input checks for kadr: what posted window messages cannot cover.
#
# this script presses keys and moves the mouse on your desktop. it is meant
# to be started by a person. an agent must not run it without explicit
# permission for that specific run (see docs/incidents.md).
#
# what it does:
#   1. ctrl+shift+printscreen   -> screen capture saved
#   2. alt+printscreen          -> active window capture saved
#   3. ctrl+printscreen, real mouse drag 400,300 -> 1199,799 -> 800x500 saved
#   4. ctrl+printscreen, then the esc key -> cancelled (keyboard focus works)
# it never presses bare printscreen (snipping tool would open and could save
# a screenshot to your pictures folder).
#
# safety:
#   - waits for enter, then a 5 s countdown; moving the mouse cancels
#   - between steps, if the cursor is not where the script left it, it stops
#   - own settings file and temp folder (deleted), clipboard backed up and
#     restored, test copies kept out of clipboard history
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\input.ps1 [-StandardUser]
param([switch] $StandardUser)
. "$PSScriptRoot\common.ps1"
Add-Type -AssemblyName System.Windows.Forms

$steps = @(
  'ctrl+shift+printscreen  -> screen capture',
  'alt+printscreen         -> active window capture',
  'ctrl+printscreen + drag -> region 800x500',
  'ctrl+printscreen + esc  -> cancel'
)
Write-Host "kadr input test. it will press keys and move the mouse:"
$steps | ForEach-Object { Write-Host "  - $_" }
Write-Host "do not touch the mouse or keyboard until it finishes (about 15 s)."
[void](Read-Host 'press enter to start, ctrl+c to abort')

$start = [KadrTest]::CursorPos()
for ($i = 5; $i -ge 1; $i--) {
  Write-Host "  starting in $i s (move the mouse to cancel)"
  Start-Sleep -Seconds 1
  if ([KadrTest]::CursorPos() -ne $start) { Write-Host 'mouse moved: cancelled, nothing was pressed'; exit 2 }
}

function Guard([string] $expected) {
  $now = [KadrTest]::CursorPos()
  if ($now -ne $expected) { throw "the mouse moved ($now, expected $expected): stopping" }
}

$data = [Windows.Forms.Clipboard]::GetDataObject(); $backup = New-Object Windows.Forms.DataObject; $n = 0
if ($data) { foreach ($f in $data.GetFormats($false)) { try { $v = $data.GetData($f); if ($null -ne $v) { $backup.SetData($f, $v); $n++ } } catch { } } }

try {
  # bare printscreen (the default) is never pressed by this script: snipping
  # tool could take it. region stays on ctrl+printscreen here.
  $s = Settings "hotkey_region = ctrl+printscreen`r`nafter_capture = copy+save`r`nsave_folder = $caps`r`n"
  $p = if ($StandardUser) { Start-Kadr $s -Unelevated -Extra '--test-private-clipboard' } else { Start-Kadr $s -Extra '--test-private-clipboard' }
  [void](Wait-Trace 'start, memory' 5000); [void](Close-Dialog $p); [void](Wait-Trace 'ready' 5000)
  $parked = '20,20'; [KadrTest]::MoveTo(20, 20); Start-Sleep -Milliseconds 200

  Write-Host "`n1. $($steps[0])"
  Guard $parked
  [KadrTest]::Chord([KadrTest]::CTRL, [KadrTest]::SHIFT, [KadrTest]::PRTSC)
  Check (Wait-Count 'saved ' 1 4000) 'ctrl+shift+printscreen reaches kadr and saves'

  Write-Host "`n2. $($steps[1])"
  Guard $parked
  [KadrTest]::Chord([KadrTest]::ALT, [KadrTest]::PRTSC)
  Check (Wait-Count 'saved ' 2 4000) 'alt+printscreen reaches kadr and saves'

  Write-Host "`n3. $($steps[2])"
  Guard $parked
  [KadrTest]::Chord([KadrTest]::CTRL, [KadrTest]::PRTSC)
  Check (Wait-Trace 'overlay shown' 3000) 'ctrl+printscreen opens the overlay'
  Start-Sleep -Milliseconds 200
  [KadrTest]::Drag(400, 300, 1199, 799, 30, 10)
  # kadr reports the selection relative to the virtual screen's top-left corner
  $vo = [KadrTest]::VirtualOrigin()
  $at = '{0},{1}' -f (400 - $vo[0]), (300 - $vo[1])
  Check (Wait-Trace "selected 800x500 at $at" 3000) "real drag selects 800x500 (screen 400,300 = overlay $at)"
  Check (Wait-Count 'saved ' 3 4000) 'region saved'
  $parked = '1199,799'

  Write-Host "`n4. $($steps[3])"
  Guard $parked
  [KadrTest]::Chord([KadrTest]::CTRL, [KadrTest]::PRTSC)
  [void](Wait-Count 'overlay shown' 2 3000)
  Start-Sleep -Milliseconds 300
  [KadrTest]::Chord([KadrTest]::ESC)
  Check (Wait-Count '^\[kadr\] cancelled' 1 3000) 'the esc key cancels without clicking first (overlay has keyboard focus)'

  Quit $p
  Check ($p.HasExited -and $p.ExitCode -eq 0) 'quits with code 0'
} finally {
  Stop-Started
  $backup.SetData('ExcludeClipboardContentFromMonitorProcessing', (New-Object IO.MemoryStream(, [byte[]](0, 0, 0, 0))))
  if ($n -gt 0) { [Windows.Forms.Clipboard]::SetDataObject($backup, $true, 5, 100) } else { [Windows.Forms.Clipboard]::Clear() }
  Write-Host "`nclipboard restored; trace:"
  Trace | Where-Object { $_ -notmatch 'paint ' } | ForEach-Object { Write-Host "  $_" }
  Remove-Work
}
if ($failures.Count) { Write-Host "`n$($failures.Count) check(s) failed"; exit 1 }
Write-Host "`nall checks passed"
