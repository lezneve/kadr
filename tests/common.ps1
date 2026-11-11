# shared helpers for kadr test scripts. dot-source it: . "$PSScriptRoot\common.ps1"
#
# nothing here injects keyboard or mouse input. kadr is driven by posting
# window messages to its own windows.

$ErrorActionPreference = 'Stop'
# windows' printscreen → snipping tool setting is read from a stand-in key
# under HKCU\Software\kadr-test, never from the real setting (snipping.rs).
$env:KADR_TEST_REGISTRY = '1'
$root = Split-Path -Parent $PSScriptRoot
# KADR_TEST_EXE: a copy of the build, so a long soak can run while the
# source is rebuilt
$exe = if ($env:KADR_TEST_EXE) { $env:KADR_TEST_EXE } else { Join-Path $root 'target\release\kadr.exe' }
$sampler = Join-Path $root '..\.agents\skills\lezneve-lean-engineering\scripts\measure-windows.ps1'
$work = Join-Path ([IO.Path]::GetTempPath()) "kadr-test-$PID"
$caps = Join-Path $work 'caps'
New-Item -ItemType Directory -Force $caps | Out-Null
Add-Type -Path (Join-Path $PSScriptRoot 'input.cs') -ReferencedAssemblies System.Drawing

$failures = New-Object System.Collections.Generic.List[string]
$started = New-Object System.Collections.Generic.List[object]
$script:log = $null

$WM_CLOSE = 0x0010; $WM_HOTKEY = 0x0312; $WM_KEYDOWN = 0x0100; $WM_MOUSEMOVE = 0x0200
$WM_LBUTTONDOWN = 0x0201; $WM_LBUTTONUP = 0x0202; $WM_RBUTTONUP = 0x0205; $WM_TRAY = 0x8001
$MOD_CTRL = 0x2; $MOD_SHIFT = 0x4; $MOD_NOREPEAT = 0x4000; $VK_PRTSC = 0x2C; $VK_F10 = 0x79; $VK_F11 = 0x7A; $MOD_ALT = 0x1

function Check([bool] $ok, [string] $what) {
  if ($ok) { Write-Host "  ok    $what" } else { Write-Host "  FAIL  $what"; $failures.Add($what) }
}
function Settings([string] $text) {
  $path = Join-Path $work 'settings.ini'
  [IO.File]::WriteAllText($path, $text)
  $path
}

# -Unelevated starts kadr with a standard-user token (no admin rights, medium
# integrity), the token uac gives a normal session. this also works when uac
# is turned off on the test machine, where even explorer is elevated.
function Start-Kadr([string] $settings, [switch] $Unelevated, [string[]] $Extra = @()) {
  $log = Join-Path $work ("trace-{0}.txt" -f [guid]::NewGuid().ToString('n').Substring(0, 6))
  $script:log = $log
  $args = @('--trace', '--settings', "`"$settings`"") + $Extra
  if ($Unelevated) {
    $id = [KadrTest]::StartStandardUser("`"$exe`" $($args -join ' ')", $log)
    $p = Get-Process -Id $id
    $null = $p.Handle # keep a handle so ExitCode is readable after exit
  } else {
    $p = Start-Process $exe -ArgumentList $args -RedirectStandardError $log -PassThru
  }
  $started.Add($p)
  $p
}
function Trace { if ($script:log -and (Test-Path $script:log)) { Get-Content $script:log } else { @() } }
function Wait-Trace([string] $pattern, [int] $ms = 4000) {
  $end = (Get-Date).AddMilliseconds($ms)
  while ((Get-Date) -lt $end) {
    if (Trace | Select-String -Pattern $pattern -Quiet) { return $true }
    Start-Sleep -Milliseconds 100
  }
  $false
}
function Count-Trace([string] $pattern) { @(Trace | Select-String -Pattern $pattern).Count }
function Window-Of($p, [string] $class, [int] $tries = 50) {
  for ($i = 0; $i -lt $tries; $i++) {
    $h = [KadrTest]::WindowOf([uint32]$p.Id, $class)
    if ($h -ne [IntPtr]::Zero) { return $h }
    Start-Sleep -Milliseconds 40
  }
  [IntPtr]::Zero
}
function Close-Dialog($p) {
  $d = Window-Of $p '#32770' 100
  if ($d -ne [IntPtr]::Zero) { [void][KadrTest]::PostMessage($d, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero); return $true }
  $false
}
function Quit($p) {
  $h = Window-Of $p 'kadr.main'
  [void][KadrTest]::PostMessage($h, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
  $p.WaitForExit(5000) | Out-Null
}
function Post-Hotkey($h, [int] $id) { [void][KadrTest]::PostMessage($h, $WM_HOTKEY, [IntPtr]$id, [IntPtr]::Zero) }
function Pack-Point([int] $x, [int] $y) { [IntPtr](($y -shl 16) -bor ($x -band 0xffff)) }
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
    gdi = [KadrTest]::GetGuiResources($p.Handle, 0); user = [KadrTest]::GetGuiResources($p.Handle, 1)
    handles = $p.HandleCount; threads = $p.Threads.Count }
}
# waits until `count` lines match, or the timeout passes
function Wait-Count([string] $pattern, [int] $count, [int] $ms = 5000) {
  $end = (Get-Date).AddMilliseconds($ms)
  while ((Count-Trace $pattern) -lt $count -and (Get-Date) -lt $end) { Start-Sleep -Milliseconds 30 }
  (Count-Trace $pattern) -ge $count
}
# opens the overlay and returns its window, or zero
function Open-Overlay($p, $h) {
  Post-Hotkey $h 1
  Window-Of $p 'kadr.overlay'
}
function Drag($o, [int] $x0, [int] $y0, [int] $x1, [int] $y1, [int] $steps = 20, [int] $delay = 8) {
  [void][KadrTest]::PostPhysical($o, $WM_LBUTTONDOWN, [IntPtr]1, (Pack-Point $x0 $y0))
  for ($s = 1; $s -le $steps; $s++) {
    [void][KadrTest]::PostPhysical($o, $WM_MOUSEMOVE, [IntPtr]1, (Pack-Point ($x0 + ($x1 - $x0) * $s / $steps) ($y0 + ($y1 - $y0) * $s / $steps)))
    if ($delay) { Start-Sleep -Milliseconds $delay }
  }
  [void][KadrTest]::PostPhysical($o, $WM_LBUTTONUP, [IntPtr]0, (Pack-Point $x1 $y1))
}
# the user's clipboard: copied (never read or printed) before a test that
# copies, and put back afterwards, kept out of clipboard history.
function Backup-Clipboard {
  Add-Type -AssemblyName System.Windows.Forms
  $d = [Windows.Forms.Clipboard]::GetDataObject(); $copy = New-Object Windows.Forms.DataObject; $n = 0
  if ($d) { foreach ($f in $d.GetFormats($false)) { try { $v = $d.GetData($f); if ($null -ne $v) { $copy.SetData($f, $v); $n++ } } catch { } } }
  [pscustomobject]@{ data = $copy; formats = $n }
}
function Restore-Clipboard($b) {
  $b.data.SetData('ExcludeClipboardContentFromMonitorProcessing', (New-Object IO.MemoryStream(, [byte[]](0, 0, 0, 0))))
  if ($b.formats -gt 0) { [Windows.Forms.Clipboard]::SetDataObject($b.data, $true, 5, 100) } else { [Windows.Forms.Clipboard]::Clear() }
}
function Stop-Started {
  foreach ($k in $started) { if (-not $k.HasExited) { Stop-Process -Id $k.Id -Force -ErrorAction SilentlyContinue } }
  Start-Sleep -Milliseconds 300
}
function Remove-Work {
  if ($failures.Count -and $script:log -and (Test-Path $script:log)) {
    # traces hold timings and paths only, never image data
    Copy-Item $script:log (Join-Path ([IO.Path]::GetTempPath()) 'kadr-test-last-trace.txt') -Force
  }
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
