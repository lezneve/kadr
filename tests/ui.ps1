# screenshots and memory of the native settings window, driven by window
# messages only (no input injection). images contain only kadr's own window.
# the settings window lives in its own short-lived process; this checks both
# that process and the resident kadr.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\ui.ps1 -Out <folder>
param([Parameter(Mandatory = $true)] [string] $Out, [int] $Cycles = 20)
. "$PSScriptRoot\common.ps1"
New-Item -ItemType Directory -Force $Out | Out-Null
$BM_CLICK = 0x00F5; $VK_TAB = 0x09; $VK_ESCAPE = 0x1B

function Settings-Window([int] $tries = 75) {
  for ($i = 0; $i -lt $tries; $i++) {
    $s = [KadrTest]::FindWindow('kadr.settings', [NullString]::Value)
    if ($s -ne [IntPtr]::Zero) { return $s }
    Start-Sleep -Milliseconds 40
  }
  [IntPtr]::Zero
}
function Open-Settings($p, $h) {
  # tray right click -> menu -> 4x down (settings) -> enter
  [void][KadrTest]::PostMessage($h, $WM_TRAY, (Pack-Point 1200 900), [IntPtr]0x007B)
  $m = Window-Of $p '#32768'
  Start-Sleep -Milliseconds 150
  foreach ($i in 1..4) { [void][KadrTest]::PostMessage($m, $WM_KEYDOWN, [IntPtr]0x28, [IntPtr]::Zero) }
  [void][KadrTest]::PostMessage($m, $WM_KEYDOWN, [IntPtr]0x0D, [IntPtr]::Zero)
  Settings-Window
}
function Close-Settings($s) {
  $child = Get-Process -Id ([KadrTest]::PidOf($s)) -ErrorAction SilentlyContinue
  [void][KadrTest]::PostMessage($s, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
  if ($child) { $child.WaitForExit(5000) | Out-Null }
  $child
}

$rows = @()
try {
  foreach ($theme in 'dark', 'light') {
    $env:KADR_THEME = $theme
    $p = Start-Kadr (Settings "after_capture = copy+save`r`nsave_folder = $caps`r`n")
    [void](Wait-Trace 'ready' 5000)
    $h = Window-Of $p 'kadr.main'
    Start-Sleep -Milliseconds 500
    $before = Memory $p
    $s = Open-Settings $p $h
    Check ($s -ne [IntPtr]::Zero) "${theme}: settings window opens from the tray menu"
    $child = Get-Process -Id ([KadrTest]::PidOf($s))
    Check ($child.Id -ne $p.Id) "${theme}: it runs in its own process ($($child.Id))"
    Start-Sleep -Milliseconds 500
    $open = Memory $p
    $childMem = Memory $child
    [KadrTest]::SaveWindow($s, (Join-Path $Out "settings-$theme.png"))

    # focus moves with tab (dialog manager), crop marks follow
    $btn = [KadrTest]::GetDlgItem($s, 101)
    [void][KadrTest]::PostMessage($btn, $WM_KEYDOWN, [IntPtr]$VK_TAB, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    [KadrTest]::SaveWindow($s, (Join-Path $Out "settings-$theme-tab.png"))

    # recording: click the region hotkey row, screenshot, esc
    [void][KadrTest]::SendMessage($btn, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    Check (Wait-Trace 'settings: recording region' 2000) "${theme}: clicking a hotkey row starts recording"
    Check (Hotkey-Free ($MOD_ALT -bor $MOD_NOREPEAT) $VK_PRTSC) "${theme}: kadr releases its hotkeys while recording"
    [KadrTest]::SaveWindow($s, (Join-Path $Out "settings-$theme-recording.png"))
    [void][KadrTest]::PostMessage($btn, $WM_KEYDOWN, [IntPtr]$VK_ESCAPE, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    Check (-not (Hotkey-Free ($MOD_ALT -bor $MOD_NOREPEAT) $VK_PRTSC)) "${theme}: esc ends recording and hotkeys are back"

    $c = Close-Settings $s
    Check ($c.HasExited) "${theme}: closing the window ends the settings process"
    $closed = Memory $p

    # killed while recording: kadr must get its hotkeys back on its own
    $s = Open-Settings $p $h
    $btn = [KadrTest]::GetDlgItem($s, 101)
    [void][KadrTest]::SendMessage($btn, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    Stop-Process -Id ([KadrTest]::PidOf($s)) -Force
    Start-Sleep -Milliseconds 500
    Check (-not (Hotkey-Free ($MOD_ALT -bor $MOD_NOREPEAT) $VK_PRTSC)) "${theme}: a crashed settings process leaves kadr's hotkeys working"

    for ($i = 0; $i -lt $Cycles; $i++) { $s = Open-Settings $p $h; [void](Close-Settings $s) }
    Start-Sleep -Milliseconds 800
    $after = Memory $p
    $rows += [pscustomobject]@{ theme = $theme; before = $before; open = $open; child = $childMem; closed = $closed; after = $after }
    # baseline: after the first open/close (the first tray menu creates a few per-thread system objects once)
    Check ($after.private -le $closed.private + 0.5) "${theme}: resident kadr does not grow over $Cycles open/close ($($closed.private) -> $($after.private) MiB; $($before.private) before any menu)"
    Check ($after.gdi -le $closed.gdi -and $after.user -le $closed.user + 1) "${theme}: resident kadr gdi/user do not grow ($($closed.gdi)/$($closed.user) -> $($after.gdi)/$($after.user))"
    Quit $p

    # held conflict and the focused file name field, in a standalone settings
    # process (no kadr needed): bare printscreen is taken by snipping tool here.
    $ini = Settings "hotkey_region = printscreen`r`nafter_capture = copy+save`r`n"
    $sp = Start-Process $exe -ArgumentList '--settings-window', "`"$ini`"" -PassThru
    $s = Settings-Window
    Start-Sleep -Milliseconds 400
    [KadrTest]::SaveWindow($s, (Join-Path $Out "settings-$theme-conflict.png"))
    foreach ($id in 101, 102, 103, 203, 301) {
      [void][KadrTest]::PostMessage([KadrTest]::GetDlgItem($s, $id), $WM_KEYDOWN, [IntPtr]$VK_TAB, [IntPtr]::Zero)
      Start-Sleep -Milliseconds 120
    }
    Start-Sleep -Milliseconds 300
    [KadrTest]::SaveWindow($s, (Join-Path $Out "settings-$theme-filename.png"))
    [void][KadrTest]::PostMessage($s, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    $sp.WaitForExit(5000) | Out-Null
    Check ($sp.HasExited) "${theme}: standalone settings process closes"
  }
} finally {
  Remove-Item Env:\KADR_THEME -ErrorAction SilentlyContinue
  Get-Process kadr -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $started.Id } | ForEach-Object { if ((Get-CimInstance Win32_Process -Filter "ProcessId=$($_.Id)").CommandLine -match 'settings-window') { Stop-Process -Id $_.Id -Force } }
  Stop-Started
  Remove-Work
}
$rows | ForEach-Object {
  "{0}: kadr before ws {1} / private {2} MiB | kadr while open {3} / {4} | settings process {5} / {6} | kadr after close {7} / {8} | after {9} cycles {10} / {11}" -f `
    $_.theme, $_.before.ws, $_.before.private, $_.open.ws, $_.open.private, $_.child.ws, $_.child.private, $_.closed.ws, $_.closed.private, $Cycles, $_.after.ws, $_.after.private
}
if ($failures.Count) { Write-Host "`n$($failures.Count) check(s) failed"; exit 1 }
Write-Host "`nall checks passed"
