# non-interactive checks for a release build of kadr.
#
# does not inject keyboard or mouse input and does not touch the clipboard.
# captures are triggered by posting WM_HOTKEY to kadr's window (the same code
# path as a real hotkey), saved to a temp folder and deleted at the end.
# error dialogs appear briefly and are closed by the script.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\smoke.ps1
param([int] $Captures = 20, [int] $IdleSeconds = 30)

$ErrorActionPreference = 'Stop'
# the printscreen → snipping tool setting is read from a stand-in key under
# HKCU\Software\kadr-test, never from the real windows setting.
$env:KADR_TEST_REGISTRY = '1'
$fakeKb = 'HKCU:\Software\kadr-test\Keyboard'
$fakeState = 'HKCU:\Software\kadr-test\State'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\release\kadr.exe'
$sampler = Join-Path $root '..\.agents\skills\lezneve-lean-engineering\scripts\measure-windows.ps1'
$work = Join-Path ([IO.Path]::GetTempPath()) "kadr-smoke-$PID"
$caps = Join-Path $work 'caps'
New-Item -ItemType Directory -Force $caps | Out-Null
Add-Type -Path (Join-Path $PSScriptRoot 'input.cs') -ReferencedAssemblies System.Drawing

$failures = New-Object System.Collections.Generic.List[string]
function Check([bool] $ok, [string] $what) {
  if ($ok) { Write-Host "  ok    $what" } else { Write-Host "  FAIL  $what"; $failures.Add($what) }
}
function Settings([string] $text) {
  $path = Join-Path $work 'settings.ini'
  [IO.File]::WriteAllText($path, $text)
  $path
}
function Start-Kadr([string] $settings) {
  $log = Join-Path $work ("trace-{0}.txt" -f [guid]::NewGuid().ToString('n').Substring(0, 6))
  $p = Start-Process $exe -ArgumentList '--trace', '--settings', $settings -RedirectStandardError $log -PassThru
  $script:log = $log
  $started.Add($p)
  $p
}
function Trace { if (Test-Path $script:log) { Get-Content $script:log } else { @() } }
function Wait-Trace([string] $pattern, [int] $ms = 4000) {
  $end = (Get-Date).AddMilliseconds($ms)
  while ((Get-Date) -lt $end) {
    if (Trace | Select-String -Pattern $pattern -Quiet) { return $true }
    Start-Sleep -Milliseconds 100
  }
  $false
}
function Main-Window($p) {
  for ($i = 0; $i -lt 40; $i++) {
    $h = [KadrTest]::WindowOf([uint32]$p.Id, 'kadr.main')
    if ($h -ne [IntPtr]::Zero) { return $h }
    Start-Sleep -Milliseconds 100
  }
  [IntPtr]::Zero
}
function Close-Dialog($p) {
  for ($i = 0; $i -lt 40; $i++) {
    $d = [KadrTest]::WindowOf([uint32]$p.Id, '#32770')
    if ($d -ne [IntPtr]::Zero) { [void][KadrTest]::PostMessage($d, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero); return $true }
    Start-Sleep -Milliseconds 100
  }
  $false
}
function Quit($p) {
  $h = Main-Window $p
  [void][KadrTest]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
  $p.WaitForExit(5000) | Out-Null
}
function Post-Hotkey($h, [int] $id) { [void][KadrTest]::PostMessage($h, 0x0312, [IntPtr]$id, [IntPtr]::Zero) }
function Hotkey-Free([uint32] $mods, [uint32] $vk) {
  $ok = [KadrTest]::RegisterHotKey([IntPtr]::Zero, 991, $mods, $vk)
  if ($ok) { [void][KadrTest]::UnregisterHotKey([IntPtr]::Zero, 991) }
  $ok
}
function Png-Size([string] $path) {
  $b = [IO.File]::ReadAllBytes($path)
  '{0}x{1}' -f [BitConverter]::ToUInt32(@($b[19], $b[18], $b[17], $b[16]), 0), [BitConverter]::ToUInt32(@($b[23], $b[22], $b[21], $b[20]), 0)
}
function Memory($p) {
  $p.Refresh()
  [pscustomobject]@{ ws = [math]::Round($p.WorkingSet64 / 1MB, 1); private = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
    gdi = [KadrTest]::GetGuiResources($p.Handle, 0); user = [KadrTest]::GetGuiResources($p.Handle, 1); threads = $p.Threads.Count }
}
$MOD_CTRL = 0x2; $MOD_ALT = 0x1; $MOD_SHIFT = 0x4; $MOD_NOREPEAT = 0x4000; $VK_PRTSC = 0x2C; $VK_F10 = 0x79; $VK_F11 = 0x7A

$results = [ordered]@{}
$started = New-Object System.Collections.Generic.List[object]
try {

Write-Host "`n1. idle, then captures, then idle again"
$p = Start-Kadr (Settings "after_capture = save`r`nsave_folder = $caps`r`n")
Check (Wait-Trace 'ready') 'starts and reports ready'
$h = Main-Window $p
Start-Sleep -Seconds 5
$idle0 = Memory $p
$cpu0 = $p.TotalProcessorTime
& powershell -NoProfile -ExecutionPolicy Bypass -File $sampler -RootProcessId $p.Id -Scenario idle-before -DurationSeconds $IdleSeconds -OutputPath (Join-Path $work 'idle-before.json') | Out-Null
$p.Refresh(); $idleCpu = ($p.TotalProcessorTime - $cpu0).TotalMilliseconds
$results.idle_before = Get-Content (Join-Path $work 'idle-before.json') -Raw | ConvertFrom-Json | Select-Object working_set_mb, private_bytes_mb, process_count
Check ($idleCpu -lt 50) "idle cpu ${idleCpu} ms over ~$($IdleSeconds + 5) s"
# measured here (windows setting on): a second app could still register bare
# printscreen while kadr held it, so a probe cannot see kadr's registration:
# the trace can.
Check (Trace | Select-String '^\[kadr\] hotkey printscreen registered' -Quiet) 'printscreen is registered (default region hotkey)'
Check (-not (Trace | Select-String 'snipping: asking' -Quiet)) 'no printscreen question when windows does not take the key'
Check (-not (Trace | Select-String '^\[kadr\] tray:' -Quiet)) 'tray icon added without error'
$exeText = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($exe))
Check ($exeText.Contains('<dpiAwareness') -and $exeText.Contains('PerMonitorV2')) 'manifest with PerMonitorV2 is embedded in the exe'

for ($i = 1; $i -le $Captures; $i++) {
  Post-Hotkey $h (2 + ($i % 2)) # alternate window (2) and screen (3)
  $end = (Get-Date).AddSeconds(5)
  while (@(Trace | Select-String 'saved ').Count -lt $i -and (Get-Date) -lt $end) { Start-Sleep -Milliseconds 50 }
  if (@(Trace | Select-String 'saved ').Count -lt $i) { break }
  if ($i -eq 1) { $after1 = Memory $p }
}
$files = @(Get-ChildItem $caps -Filter *.png)
Check ($files.Count -eq $Captures) "$Captures posted captures produced $($files.Count) files"
$sizes = $files | ForEach-Object { Png-Size $_.FullName } | Sort-Object -Unique
Write-Host "  sizes: $($sizes -join ', ')"
Start-Sleep -Seconds 3
$afterN = Memory $p
Check ($afterN.gdi -le $idle0.gdi + 2) "gdi objects idle $($idle0.gdi) -> after $Captures captures $($afterN.gdi)"
Check ($afterN.private -le $after1.private + 1.0) "private MiB after 1st capture $($after1.private) -> after ${Captures}th $($afterN.private)"
Check ($afterN.threads -le $idle0.threads + 2) "threads idle $($idle0.threads) -> after $($afterN.threads)"
& powershell -NoProfile -ExecutionPolicy Bypass -File $sampler -RootProcessId $p.Id -Scenario idle-after -DurationSeconds $IdleSeconds -OutputPath (Join-Path $work 'idle-after.json') | Out-Null
$results.idle_after = Get-Content (Join-Path $work 'idle-after.json') -Raw | ConvertFrom-Json | Select-Object working_set_mb, private_bytes_mb, process_count
$results.memory = [ordered]@{ start = $idle0; after_first = $after1; after_all = $afterN }
$timings = Trace | Select-String '(window|screen) \d+x\d+ at .*: ([\d.]+) ms' | ForEach-Object { [double]$_.Matches[0].Groups[2].Value }
$saves = Trace | Select-String 'saved .* in ([\d.]+) ms' | ForEach-Object { [double]$_.Matches[0].Groups[1].Value }
$results.grab_ms = ($timings | Measure-Object -Minimum -Maximum -Average | Select-Object Count, Minimum, Maximum, Average)
$results.encode_save_ms = ($saves | Measure-Object -Minimum -Maximum -Average | Select-Object Count, Minimum, Maximum, Average)
Quit $p
Check ($p.HasExited -and $p.ExitCode -eq 0) "quits with code 0 (code $($p.ExitCode))"
Check (Hotkey-Free ($MOD_ALT -bor $MOD_NOREPEAT) $VK_PRTSC) 'alt+printscreen is free after quit'
Check (Wait-Trace 'exit, memory' 1000) 'trace shows clean exit'
$files | Remove-Item -Force

Write-Host "`n2. settings reload re-registers hotkeys"
$s = Settings "hotkey_region = ctrl+shift+f11`r`nafter_capture = save`r`nsave_folder = $caps`r`n"
$p = Start-Kadr $s
[void](Wait-Trace 'ready')
Check (-not (Hotkey-Free ($MOD_CTRL -bor $MOD_SHIFT -bor $MOD_NOREPEAT) $VK_F11)) 'ctrl+shift+f11 registered from file'
[IO.File]::WriteAllText($s, "hotkey_region = ctrl+shift+f10`r`nafter_capture = save`r`nsave_folder = $caps`r`n")
Check (Wait-Trace 'settings reloaded' 4000) 'file change is picked up'
Check (Hotkey-Free ($MOD_CTRL -bor $MOD_SHIFT -bor $MOD_NOREPEAT) $VK_F11) 'old hotkey released'
Check (-not (Hotkey-Free ($MOD_CTRL -bor $MOD_SHIFT -bor $MOD_NOREPEAT) $VK_F10)) 'new hotkey registered'

Write-Host "`n3. a second instance refuses to start"
$q = Start-Process $exe -ArgumentList '--settings', $s -PassThru
Check (Close-Dialog $q) 'second instance shows "already running"'
$q.WaitForExit(3000) | Out-Null
Check ($q.HasExited -and $q.ExitCode -eq 0) 'second instance exits'
Check (-not $p.HasExited) 'first instance keeps running'
Quit $p

Write-Host "`n4. conflicts and bad settings are reported"
$p = Start-Kadr (Settings "hotkey_region = win+shift+s`r`nhotkey_window = ctrl+shift+f9`r`nafter_capture = maybe`r`nnonsense`r`n")
Check (Close-Dialog $p) 'problem dialog shown'
[void](Wait-Trace 'ready')
$t = (Trace) -join "`n"
Check ($t -match 'shift\+win\+s is already used by another app') 'win+shift+s conflict named'
Check ($t -match 'line 3: after_capture must be') 'bad value reported with line number'
Check ($t -match 'line 4: expected key = value') 'malformed line reported'
Quit $p

Write-Host "`n4b. printscreen taken by snipping tool: one question, explicit consent"
function Answer-Question($p, [int] $button) {
  $d = [IntPtr]::Zero
  for ($i = 0; $i -lt 40 -and $d -eq [IntPtr]::Zero; $i++) { $d = [KadrTest]::WindowOf([uint32]$p.Id, '#32770'); Start-Sleep -Milliseconds 100 }
  if ($d -eq [IntPtr]::Zero) { return $false }
  $b = [KadrTest]::GetDlgItem($d, $button)
  if ($b -eq [IntPtr]::Zero) { return $false }
  [void][KadrTest]::PostMessage($b, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) # BM_CLICK
  $true
}
Remove-Item 'HKCU:\Software\kadr-test' -Recurse -Force -ErrorAction SilentlyContinue
New-Item $fakeKb -Force | Out-Null
New-ItemProperty $fakeKb -Name PrintScreenKeyForSnippingEnabled -Value 1 -PropertyType DWord | Out-Null
$ps = Settings "hotkey_region = printscreen`r`nafter_capture = save`r`nsave_folder = $caps`r`n"
$p = Start-Kadr $ps
Check (Wait-Trace 'snipping: asking' 4000) 'asks once when windows takes printscreen'
Check (Answer-Question $p 7) 'the question can be answered "no"'
Check (Wait-Trace 'user kept the windows setting' 3000) '"no" keeps the windows setting'
Check ((Get-ItemProperty $fakeKb).PrintScreenKeyForSnippingEnabled -eq 1) 'value untouched after "no"'
Quit $p
$p = Start-Kadr $ps
[void](Wait-Trace 'ready')
Check (-not (Trace | Select-String 'snipping: asking' -Quiet)) 'start-up does not ask again after an answer'
Quit $p
Remove-ItemProperty $fakeState -Name PrintScreenAsked
$p = Start-Kadr $ps
[void](Wait-Trace 'snipping: asking' 4000)
Check (Answer-Question $p 6) 'the question can be answered "yes"'
Check (Wait-Trace 'handed back to apps' 3000) '"yes" frees printscreen'
Start-Sleep -Milliseconds 300
Check ((Answer-Question $p 2) -or (Answer-Question $p 1)) 'then kadr says how to check the key and when to sign out'
Check ((Get-ItemProperty $fakeKb).PrintScreenKeyForSnippingEnabled -eq 0) 'value set to 0 after "yes"'
Check ((Get-ItemProperty $fakeState).PrintScreenSnippingBefore -eq 1) 'previous value recorded for uninstall'
Check ($null -eq (Get-ItemProperty $fakeState).PrintScreenConfirmed) 'not marked as working before a printscreen arrives'
[void][KadrTest]::PostMessage((Main-Window $p), 0x0312, [IntPtr]1, [IntPtr]::Zero) # the region hotkey (bare printscreen)
Check (Wait-Trace 'printscreen reached kadr' 3000) 'the first printscreen that reaches kadr confirms the change'
Check ((Get-ItemProperty $fakeState).PrintScreenConfirmed -eq 1) 'confirmation recorded'
Start-Sleep -Milliseconds 300
$ov = [KadrTest]::WindowOf([uint32]$p.Id, 'kadr.overlay')
if ($ov -ne [IntPtr]::Zero) { [void][KadrTest]::PostMessage($ov, 0x0205, [IntPtr]::Zero, [IntPtr]::Zero) } # right click cancels
Quit $p
Remove-Item $fakeKb, $fakeState -Recurse -Force -ErrorAction SilentlyContinue

Write-Host "`n5. save failure is reported, nothing half-written"
# a path that goes through a regular file can never be created, with or
# without admin rights (a protected system folder would work when elevated).
$blocker = Join-Path $work 'blocker.txt'
Set-Content $blocker 'not a folder'
$denied = Join-Path $blocker 'sub'
$p = Start-Kadr (Settings "after_capture = save`r`nsave_folder = $denied`r`n")
[void](Wait-Trace 'ready')
Post-Hotkey (Main-Window $p) 3
Check (Close-Dialog $p) 'error dialog shown'
Check (Wait-Trace 'could not save the capture' 2000) 'error says the capture was not saved'
Check (-not (Test-Path $denied)) 'nothing created at the impossible path'
Check (-not $p.HasExited) 'keeps running after the error'
Quit $p

Write-Host "`n6. region overlay (driven by posted window messages, the real cursor does not move)"
function Overlay-Window($p) {
  for ($i = 0; $i -lt 50; $i++) {
    $o = [KadrTest]::WindowOf([uint32]$p.Id, 'kadr.overlay')
    if ($o -ne [IntPtr]::Zero) { return $o }
    Start-Sleep -Milliseconds 20
  }
  [IntPtr]::Zero
}
function Pack-Point([int] $x, [int] $y) { [IntPtr](($y -shl 16) -bor ($x -band 0xffff)) }
$WM_KEYDOWN = 0x0100; $WM_MOUSEMOVE = 0x0200; $WM_LBUTTONDOWN = 0x0201; $WM_LBUTTONUP = 0x0202; $WM_RBUTTONUP = 0x0205
Get-ChildItem $caps -File | Remove-Item -Force
$p = Start-Kadr (Settings "after_capture = save`r`nsave_folder = $caps`r`n")
[void](Wait-Trace 'ready')
$h = Main-Window $p
$base = Memory $p
$opens = 10
for ($i = 1; $i -le $opens; $i++) {
  Post-Hotkey $h 1
  $o = Overlay-Window $p
  if ($o -eq [IntPtr]::Zero) { break }
  Start-Sleep -Milliseconds 150
  [void][KadrTest]::PostMessage($o, $WM_KEYDOWN, [IntPtr]0x1B, [IntPtr]::Zero)
  $end = (Get-Date).AddSeconds(3)
  while (@(Trace | Select-String '^\[kadr\] cancelled').Count -lt $i -and (Get-Date) -lt $end) { Start-Sleep -Milliseconds 20 }
  # the first overlay makes windows create a few per-thread objects once
  # (ime windows, font cache); leaks show up as growth after that.
  if ($i -eq 1) { Start-Sleep -Milliseconds 300; $first = Memory $p }
}
Check (@(Trace | Select-String '^\[kadr\] cancelled').Count -eq $opens) "$opens overlays opened and cancelled with esc"
Check (@(Get-ChildItem $caps -File).Count -eq 0) 'cancel saves nothing'
Start-Sleep -Milliseconds 500
$afterOpens = Memory $p
Check ($afterOpens.gdi -le $first.gdi) "gdi objects: start $($base.gdi), after 1st overlay $($first.gdi), after ${opens}th $($afterOpens.gdi)"
Check ($afterOpens.user -le $first.user) "user objects: start $($base.user), after 1st overlay $($first.user), after ${opens}th $($afterOpens.user)"
Check ($afterOpens.private -le $first.private + 0.5) "private MiB: start $($base.private), after 1st overlay $($first.private), after ${opens}th $($afterOpens.private)"

# drag 100,200 -> 900,700: the pixel under the cursor is included, so 801x501
Post-Hotkey $h 1
$o = Overlay-Window $p
Start-Sleep -Milliseconds 100
[void][KadrTest]::PostPhysical($o, $WM_LBUTTONDOWN, [IntPtr]1, (Pack-Point 100 200))
for ($s = 1; $s -le 40; $s++) {
  [void][KadrTest]::PostPhysical($o, $WM_MOUSEMOVE, [IntPtr]1, (Pack-Point (100 + 800 * $s / 40) (200 + 500 * $s / 40)))
  Start-Sleep -Milliseconds 8
}
[void][KadrTest]::PostPhysical($o, $WM_LBUTTONUP, [IntPtr]0, (Pack-Point 900 700))
Check (Wait-Trace 'selected 801x501 at 100,200' 3000) 'drag selects exactly 801x501 at 100,200'
Check (Wait-Trace 'saved ' 3000) 'selection is saved'
$shot = @(Get-ChildItem $caps -File)
Check ($shot.Count -eq 1 -and (Png-Size $shot[0].FullName) -eq '801x501') "saved png is 801x501 ($(if ($shot) { Png-Size $shot[0].FullName }))"

# a click without a drag must not capture 1 pixel; right click cancels
Post-Hotkey $h 1
$o = Overlay-Window $p
Start-Sleep -Milliseconds 100
[void][KadrTest]::PostPhysical($o, $WM_LBUTTONDOWN, [IntPtr]1, (Pack-Point 300 300))
[void][KadrTest]::PostPhysical($o, $WM_LBUTTONUP, [IntPtr]0, (Pack-Point 300 300))
Start-Sleep -Milliseconds 300
Check (([KadrTest]::WindowOf([uint32]$p.Id, 'kadr.overlay')) -ne [IntPtr]::Zero) 'a plain click keeps the overlay open'
[void][KadrTest]::PostPhysical($o, $WM_RBUTTONUP, [IntPtr]0, (Pack-Point 300 300))
Start-Sleep -Milliseconds 300
Check (([KadrTest]::WindowOf([uint32]$p.Id, 'kadr.overlay')) -eq [IntPtr]::Zero) 'right click closes it'
Check (@(Get-ChildItem $caps -File).Count -eq 1) 'neither produced a file'

# near-full-screen drag: paint cost grows with the selection area
$size = (Trace | Select-String 'grab (\d+)x(\d+) at' | Select-Object -First 1).Matches[0].Groups
$screenW = [int]$size[1].Value; $screenH = [int]$size[2].Value
Post-Hotkey $h 1
$o = Overlay-Window $p
Start-Sleep -Milliseconds 100
[void][KadrTest]::PostPhysical($o, $WM_LBUTTONDOWN, [IntPtr]1, (Pack-Point 20 20))
for ($s = 1; $s -le 40; $s++) {
  [void][KadrTest]::PostPhysical($o, $WM_MOUSEMOVE, [IntPtr]1, (Pack-Point (20 + ($screenW - 40) * $s / 40) (20 + ($screenH - 40) * $s / 40)))
  Start-Sleep -Milliseconds 16
}
[void][KadrTest]::PostPhysical($o, $WM_LBUTTONUP, [IntPtr]0, (Pack-Point ($screenW - 20) ($screenH - 20)))
$expect = '{0}x{1}' -f ($screenW - 39), ($screenH - 39)
Check (Wait-Trace "selected $expect at 20,20" 3000) "near-full drag selects $expect"

# tray left click (NIN_SELECT) -> 250 ms timer -> overlay
[void][KadrTest]::PostMessage($h, 0x8001, [IntPtr]::Zero, [IntPtr]0x0400)
$o = Overlay-Window $p
Check ($o -ne [IntPtr]::Zero) 'tray click opens the overlay'
[void][KadrTest]::PostMessage($o, $WM_KEYDOWN, [IntPtr]0x1B, [IntPtr]::Zero)
Start-Sleep -Milliseconds 300

# re-entrancy: a burst of theme-change broadcasts from 8 threads at once
$handled = [KadrTest]::Flood($h, 0x001A, 'ImmersiveColorSet', 8, 25)
Start-Sleep -Milliseconds 300
Check ((-not $p.HasExited) -and $handled -eq 200) "survives 200 concurrent WM_SETTINGCHANGE ($handled of 200 handled)"
Check (-not (Trace | Select-String '^\[kadr\] tray:' -Quiet)) 'tray icon still fine after the burst'

$shown = Trace | Select-String 'overlay shown: ([\d.]+) ms' | ForEach-Object { [double]$_.Matches[0].Groups[1].Value }
$peak = Trace | Select-String 'overlay shown: .*private [\d.]+ MiB \(peak ([\d.]+)\)' | ForEach-Object { [double]$_.Matches[0].Groups[1].Value }
$paints = Trace | Select-String 'paint (\d+)x(\d+): ([\d.]+) ms' | ForEach-Object {
  [pscustomobject]@{ mp = [int]$_.Matches[0].Groups[1].Value * [int]$_.Matches[0].Groups[2].Value / 1e6; ms = [double]$_.Matches[0].Groups[3].Value } }
$bucket = { param($lo, $hi) $paints | Where-Object { $_.mp -ge $lo -and $_.mp -lt $hi } | ForEach-Object ms | Measure-Object -Minimum -Maximum -Average | Select-Object Count, Minimum, Maximum, Average }
$results.overlay_shown_ms = $shown | Measure-Object -Minimum -Maximum -Average | Select-Object Count, Minimum, Maximum, Average
$results.paint_under_1mp_ms = & $bucket 0 1
$results.paint_1_to_3mp_ms = & $bucket 1 3
$results.paint_3mp_and_up_ms = & $bucket 3 1000
$results.overlay_peak_private_mb = ($peak | Measure-Object -Maximum).Maximum
$results.overlay_memory = [ordered]@{ before = $base; after_opens = $afterOpens; after_all = (Memory $p) }
Quit $p
Check ($p.HasExited -and $p.ExitCode -eq 0) 'quits cleanly after overlay use'

} finally {
  Remove-Item 'HKCU:\Software\kadr-test' -Recurse -Force -ErrorAction SilentlyContinue
  foreach ($k in $started) { if (-not $k.HasExited) { Stop-Process -Id $k.Id -Force -ErrorAction SilentlyContinue } }
  Start-Sleep -Milliseconds 300
  if ($failures.Count -and $script:log -and (Test-Path $script:log)) {
    # traces hold timings and paths only, never image data
    Copy-Item $script:log (Join-Path ([IO.Path]::GetTempPath()) 'kadr-smoke-last-trace.txt') -Force
  }
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
Write-Host "`nresults:"
$results | ConvertTo-Json -Depth 4
if ($failures.Count) { Write-Host "`n$($failures.Count) check(s) failed"; exit 1 }
Write-Host "`nall checks passed"
