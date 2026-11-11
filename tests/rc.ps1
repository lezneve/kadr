# release-candidate measurements: start-up, latency over 1000+ operations,
# memory over the run, and clean shutdown of every process.
#
# no keyboard or mouse input is injected; kadr is driven by window messages.
# kadr runs with a standard-user token. the clipboard is backed up first and
# restored at the end; test copies stay out of clipboard history. the region
# overlay flickers on screen for a few minutes.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\rc.ps1 [-Scale 1]
param([double] $Scale = 1, [string] $Report = '')
. "$PSScriptRoot\common.ps1"
Add-Type -AssemblyName System.Windows.Forms
if (-not $Report) { $Report = Join-Path ([IO.Path]::GetTempPath()) 'kadr-rc.json' }

function Stats([double[]] $v) {
  if (-not $v -or $v.Count -eq 0) { return $null }
  $s = $v | Sort-Object
  $at = { param($q) $s[[math]::Min($s.Count - 1, [int][math]::Floor($q * $s.Count))] }
  [ordered]@{ n = $s.Count; min = $s[0]; p50 = (& $at 0.5); p95 = (& $at 0.95); max = $s[-1] }
}
function Values([string] $pattern) { Trace | Select-String $pattern | ForEach-Object { [double]$_.Matches[0].Groups[1].Value } }
function Settings-Window([int] $tries = 75) {
  for ($i = 0; $i -lt $tries; $i++) { $s = [KadrTest]::FindWindow('kadr.settings', [NullString]::Value); if ($s -ne [IntPtr]::Zero) { return $s }; Start-Sleep -Milliseconds 40 }
  [IntPtr]::Zero
}
function Open-Settings($p, $h) {
  [void][KadrTest]::PostMessage($h, $WM_TRAY, (Pack-Point 1200 900), [IntPtr]0x007B)
  $m = Window-Of $p '#32768'; Start-Sleep -Milliseconds 120
  foreach ($i in 1..4) { [void][KadrTest]::PostMessage($m, $WM_KEYDOWN, [IntPtr]0x28, [IntPtr]::Zero) }
  [void][KadrTest]::PostMessage($m, $WM_KEYDOWN, [IntPtr]0x0D, [IntPtr]::Zero)
  Settings-Window
}
function Kadr-Processes { @(Get-CimInstance Win32_Process -Filter "Name='kadr.exe'" | Where-Object { $_.CommandLine -like "*$work*" }) }

$data = Backup-Clipboard
$result = [ordered]@{}
try {
  # ---- 1. start-up ----------------------------------------------------------
  Write-Host "1. start-up (10 launches each)"
  $ready = @(); $shown = @()
  foreach ($i in 1..10) {
    $p = Start-Kadr (Settings "after_capture = save`r`nsave_folder = $caps`r`n")
    [void](Wait-Trace 'ready [\d.]+ ms' 5000)
    $ready += Values 'ready ([\d.]+) ms after process start'
    Quit $p
  }
  $ini = Settings "after_capture = save`r`n"
  foreach ($i in 1..10) {
    $log = Join-Path $work "st-$i.txt"
    $sp = Start-Process $exe -ArgumentList '--settings-window', "`"$ini`"", '--trace' -RedirectStandardError $log -PassThru
    $s = Settings-Window
    Start-Sleep -Milliseconds 200
    [void][KadrTest]::PostMessage($s, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    $sp.WaitForExit(5000) | Out-Null
    $shown += Get-Content $log | Select-String 'settings window shown ([\d.]+) ms' | ForEach-Object { [double]$_.Matches[0].Groups[1].Value }
  }
  $result.ready_ms_first = $ready[0]
  $result.ready_ms = Stats $ready
  $result.settings_shown_ms = Stats $shown
  Write-Host ("  kadr ready: first {0} ms, p50 {1} ms; settings window shown: p50 {2} ms" -f $ready[0], $result.ready_ms.p50, $result.settings_shown_ms.p50)

  # ---- 2. 1000+ operations ----------------------------------------------------
  $plan = [ordered]@{ screen = [int](400 * $Scale); window = [int](300 * $Scale); region = [int](300 * $Scale); cancel = [int](100 * $Scale); settings = [int](50 * $Scale) }
  $total = ($plan.Values | Measure-Object -Sum).Sum
  Write-Host "`n2. $total operations (standard-user token, copy+save)"
  $p = Start-Kadr (Settings "after_capture = copy+save`r`nsave_folder = $caps`r`n") -Unelevated -Extra '--test-private-clipboard'
  [void](Wait-Trace 'start, memory' 5000); [void](Close-Dialog $p); [void](Wait-Trace 'ready' 5000)
  $h = Window-Of $p 'kadr.main'
  $ops = New-Object System.Collections.Generic.List[string]
  foreach ($k in $plan.Keys) { foreach ($i in 1..$plan[$k]) { $ops.Add($k) } }
  $rng = New-Object Random 42
  $ops = $ops | Sort-Object { $rng.Next() }
  $mem = @(); $saved = 0; $cancelled = 0; $done = 0; $failedOps = 0
  $clock = [Diagnostics.Stopwatch]::StartNew()
  foreach ($op in $ops) {
    switch ($op) {
      'screen' { Post-Hotkey $h 3; $saved++; if (-not (Wait-Count 'saved ' $saved 5000)) { $failedOps++ } }
      'window' { Post-Hotkey $h 2; $saved++; if (-not (Wait-Count 'saved ' $saved 5000)) { $failedOps++ } }
      'region' {
        $o = Open-Overlay $p $h
        if ($o -eq [IntPtr]::Zero) { $failedOps++; break }
        Drag $o 200 150 1000 650 6 0; $saved++
        if (-not (Wait-Count 'saved ' $saved 5000)) { $failedOps++ }
      }
      'cancel' {
        $o = Open-Overlay $p $h
        if ($o -eq [IntPtr]::Zero) { $failedOps++; break }
        [void][KadrTest]::PostMessage($o, $WM_KEYDOWN, [IntPtr]0x1B, [IntPtr]::Zero); $cancelled++
        if (-not (Wait-Count '^\[kadr\] cancelled' $cancelled 3000)) { $failedOps++ }
      }
      'settings' {
        $s = Open-Settings $p $h
        if ($s -eq [IntPtr]::Zero) { $failedOps++; break }
        $child = Get-Process -Id ([KadrTest]::PidOf($s)) -ErrorAction SilentlyContinue
        [void][KadrTest]::PostMessage($s, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
        if ($child) { $child.WaitForExit(5000) | Out-Null; if (-not $child.HasExited) { $failedOps++ } }
      }
    }
    $done++
    if ($done % 50 -eq 0) { Get-ChildItem $caps -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue }
    if ($done % 100 -eq 0) {
      Start-Sleep -Milliseconds 300
      $m = Memory $p; $mem += [pscustomobject]@{ ops = $done; private = $m.private; ws = $m.ws; gdi = $m.gdi; user = $m.user; handles = $m.handles }
      Write-Host ("  {0,5} ops  {1,5:N1} min  private {2} MiB  gdi {3}  user {4}  handles {5}" -f $done, $clock.Elapsed.TotalMinutes, $m.private, $m.gdi, $m.user, $m.handles)
    }
  }
  Check ($failedOps -eq 0) "all $total operations completed ($failedOps failed)"
  Check ((Count-Trace 'capture failed') -eq 0) 'no capture reported a failure'
  $result.operations = $plan; $result.failed = $failedOps; $result.minutes = [math]::Round($clock.Elapsed.TotalMinutes, 1)
  $result.hotkey_to_frame_ms = Stats (Values '(?:screen|window) \d+x\d+ at -?\d+,-?\d+: ([\d.]+) ms')
  $result.hotkey_to_overlay_ms = Stats (Values 'overlay shown: ([\d.]+) ms')
  $result.clipboard_ms = Stats (Values 'clipboard ([\d.]+) ms')
  $result.encode_save_ms = Stats (Values 'saved .* in ([\d.]+) ms')
  $result.memory = $mem
  $first = $mem[1]; $last = $mem[-1]
  Check ($last.gdi -le $first.gdi -and $last.handles -le $first.handles + 10) "gdi and handles flat after warm-up ($($first.gdi)/$($first.handles) -> $($last.gdi)/$($last.handles))"
  Check ($last.private -le $first.private + 2.0) "private memory flat after warm-up ($($first.private) -> $($last.private) MiB)"

  # ---- 3. shutdown -------------------------------------------------------------
  Write-Host "`n3. shutdown"
  $s = Open-Settings $p $h
  $child = Get-Process -Id ([KadrTest]::PidOf($s))
  Quit $p
  $child.WaitForExit(4000) | Out-Null
  Check ($p.HasExited -and $p.ExitCode -eq 0) 'quit with the settings window open: kadr exits with code 0'
  Check ($child.HasExited) 'and the settings process exits with it'
  Check (Wait-Trace 'settings process closed on shutdown' 1000) 'kadr closed it, not the timeout'
  Check (Hotkey-Free ($MOD_ALT -bor $MOD_NOREPEAT) $VK_PRTSC) 'hotkeys released'

  $p = Start-Kadr (Settings "after_capture = save`r`nsave_folder = $caps`r`n")
  [void](Wait-Trace 'ready' 5000)
  $h = Window-Of $p 'kadr.main'
  $s = Open-Settings $p $h
  $child = Get-Process -Id ([KadrTest]::PidOf($s))
  # windows ending the session: WM_QUERYENDSESSION then WM_ENDSESSION(true)
  [void][KadrTest]::SendMessage($h, 0x0011, [IntPtr]::Zero, [IntPtr]::Zero)
  [void][KadrTest]::SendMessage($h, 0x0016, [IntPtr]1, [IntPtr]::Zero)
  $child.WaitForExit(4000) | Out-Null
  Check ($child.HasExited) 'session end: the settings process is closed'
  Check (Hotkey-Free ($MOD_ALT -bor $MOD_NOREPEAT) $VK_PRTSC) 'session end: hotkeys released'
  Stop-Process -Id $p.Id -Force   # windows ends the process after WM_ENDSESSION
  Start-Sleep -Milliseconds 500
  Check ((Kadr-Processes).Count -eq 0) 'no kadr process left behind'
  Check (@(Get-ChildItem $work -Recurse -Filter '*.tmp' -ErrorAction SilentlyContinue).Count -eq 0) 'no temporary files left behind'
} finally {
  Stop-Started
  Kadr-Processes | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  Restore-Clipboard $data
  Write-Host "`nclipboard restored"
  $result | ConvertTo-Json -Depth 5 | Set-Content $Report -Encoding utf8
  Remove-Work
}
Write-Host "report: $Report"
if ($failures.Count) { Write-Host "`n$($failures.Count) check(s) failed"; exit 1 }
Write-Host "`nall checks passed"
