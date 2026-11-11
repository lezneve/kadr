# long-running leak check for kadr in a standard-user session.
#
# phase a (intensive): screen, window, region (drag) and cancelled overlays
#   with copy+save, back to back. the overlay flickers on screen and the
#   clipboard holds test captures during this phase; it is restored after.
# phase b (long): an invisible screen or window capture every few seconds,
#   save only, clipboard untouched, idle in between.
# every SampleSeconds: private bytes, working set, gdi, user, handles,
# threads -> csv. at the end: trend per counter after warm-up.
#
# no keyboard or mouse input is injected.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\soak.ps1 -IntenseMinutes 5 -LongMinutes 25
param([int] $IntenseMinutes = 5, [int] $LongMinutes = 25, [int] $SampleSeconds = 30, [string] $Csv = '')
. "$PSScriptRoot\common.ps1"
Add-Type -AssemblyName System.Windows.Forms
if (-not $Csv) { $Csv = Join-Path ([IO.Path]::GetTempPath()) 'kadr-soak.csv' }

function Backup-Clipboard {
  $data = [Windows.Forms.Clipboard]::GetDataObject(); $copy = New-Object Windows.Forms.DataObject; $n = 0
  if ($data) { foreach ($f in $data.GetFormats($false)) { try { $v = $data.GetData($f); if ($null -ne $v) { $copy.SetData($f, $v); $n++ } } catch { } } }
  [pscustomobject]@{ data = $copy; formats = $n }
}
function Restore-Clipboard($b) {
  $b.data.SetData('ExcludeClipboardContentFromMonitorProcessing', (New-Object IO.MemoryStream(, [byte[]](0, 0, 0, 0))))
  if ($b.formats -gt 0) { [Windows.Forms.Clipboard]::SetDataObject($b.data, $true, 5, 100) } else { [Windows.Forms.Clipboard]::Clear() }
}

$rows = New-Object System.Collections.Generic.List[object]
$clock = [Diagnostics.Stopwatch]::StartNew()
$script:nextSample = 0
$script:cycles = 0
function Sample([string] $phase, $p) {
  if ($clock.Elapsed.TotalSeconds -lt $script:nextSample) { return }
  $script:nextSample = $clock.Elapsed.TotalSeconds + $SampleSeconds
  $m = Memory $p
  $rows.Add([pscustomobject]@{ minute = [math]::Round($clock.Elapsed.TotalMinutes, 2); phase = $phase; cycles = $script:cycles
    private_mb = $m.private; ws_mb = $m.ws; gdi = $m.gdi; user = $m.user; handles = $m.handles; threads = $m.threads })
  Write-Host ("  {0,6:N1} min  {1,-7} cycles {2,5}  private {3,5} MiB  ws {4,5} MiB  gdi {5,3}  user {6,3}  handles {7,4}  threads {8,2}" -f `
    $clock.Elapsed.TotalMinutes, $phase, $script:cycles, $m.private, $m.ws, $m.gdi, $m.user, $m.handles, $m.threads)
}
function Clear-Caps { Get-ChildItem $caps -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue }

$backup = Backup-Clipboard
try {
  $p = Start-Kadr (Settings "after_capture = copy+save`r`nsave_folder = $caps`r`n") -Unelevated -Extra '--test-private-clipboard'
  [void](Wait-Trace 'start, memory' 5000)
  [void](Close-Dialog $p) # only appears when uac is off (tray blocked by uipi), see session.ps1
  [void](Wait-Trace 'ready' 5000)
  $h = Window-Of $p 'kadr.main'
  Write-Host "kadr $($p.Id): $([KadrTest]::TokenInfo([uint32]$p.Id))"

  Write-Host "`nphase a: intensive, $IntenseMinutes min (copy+save, overlay visible)"
  $end = $clock.Elapsed.TotalMinutes + $IntenseMinutes
  $saved = 0
  while ($clock.Elapsed.TotalMinutes -lt $end -and -not $p.HasExited) {
    switch ($script:cycles % 4) {
      0 { Post-Hotkey $h 3; $saved++; [void](Wait-Count 'saved ' $saved 5000) }
      1 { Post-Hotkey $h 2; $saved++; [void](Wait-Count 'saved ' $saved 5000) }
      2 {
        $o = Open-Overlay $p $h
        if ($o -ne [IntPtr]::Zero) { Drag $o 200 150 1400 900 10 0; $saved++; [void](Wait-Count 'saved ' $saved 5000) }
      }
      3 {
        $o = Open-Overlay $p $h
        if ($o -ne [IntPtr]::Zero) { [void][KadrTest]::PostMessage($o, $WM_KEYDOWN, [IntPtr]0x1B, [IntPtr]::Zero) }
        while ((Window-Of $p 'kadr.overlay' 1) -ne [IntPtr]::Zero) { Start-Sleep -Milliseconds 20 }
      }
    }
    $script:cycles++
    if ($script:cycles % 20 -eq 0) { Clear-Caps }
    Sample 'intense' $p
  }
  Clear-Caps
  Restore-Clipboard $backup
  Write-Host "  clipboard restored after phase a"
  # phase b must not touch the clipboard
  [IO.File]::WriteAllText((Join-Path $work 'settings.ini'), "after_capture = save`r`nsave_folder = $caps`r`n")
  [void](Wait-Trace 'settings reloaded' 4000)

  Write-Host "`nphase b: long, $LongMinutes min (save only, one invisible capture every 5 s)"
  $end = $clock.Elapsed.TotalMinutes + $LongMinutes
  while ($clock.Elapsed.TotalMinutes -lt $end -and -not $p.HasExited) {
    Post-Hotkey $h (2 + ($script:cycles % 2)); $saved++
    [void](Wait-Count 'saved ' $saved 5000)
    $script:cycles++
    if ($script:cycles % 20 -eq 0) { Clear-Caps }
    $wake = $clock.Elapsed.TotalSeconds + 5
    while ($clock.Elapsed.TotalSeconds -lt $wake) { Sample 'long' $p; Start-Sleep -Milliseconds 500 }
  }
  $script:nextSample = 0; Sample 'end' $p

  Check (-not $p.HasExited) 'kadr ran for the whole test'
  Check ((Count-Trace 'capture failed') -eq 0) 'no capture failed'
  Write-Host "  captures saved: $(Count-Trace 'saved ')  overlays cancelled: $(Count-Trace '^\[kadr\] cancelled')"
  $rows | Export-Csv -NoTypeInformation -Path $Csv
  Write-Host "  samples: $Csv"

  # trend over the long phase only: those samples are taken between captures,
  # so they show what stays allocated. phase a samples land mid-capture.
  # the first 2 long samples are warm-up.
  $steady = @($rows | Where-Object { $_.phase -ne 'intense' } | Select-Object -Skip 2)
  foreach ($k in 'private_mb', 'gdi', 'user', 'handles', 'threads') {
    $xs = $steady | ForEach-Object { [double]$_.minute }; $ys = $steady | ForEach-Object { [double]$_.$k }
    $n = $xs.Count; $mx = ($xs | Measure-Object -Average).Average; $my = ($ys | Measure-Object -Average).Average
    $num = 0; $den = 0; for ($i = 0; $i -lt $n; $i++) { $num += ($xs[$i] - $mx) * ($ys[$i] - $my); $den += ($xs[$i] - $mx) * ($xs[$i] - $mx) }
    $slope = if ($den) { $num / $den * 60 } else { 0 }
    $first = $steady[0].$k; $last = $steady[-1].$k; $max = ($ys | Measure-Object -Maximum).Maximum
    Write-Host ("  {0,-10} first {1,6}  last {2,6}  max {3,6}  trend {4,7:N2} per hour" -f $k, $first, $last, $max, $slope)
    $limit = @{ private_mb = 1.0; gdi = 1; user = 1; handles = 10; threads = 1 }[$k]
    Check ($last -le $first + $limit) "$k does not grow (first $first, last $last, allowed +$limit)"
  }
  Quit $p
  Check ($p.HasExited -and $p.ExitCode -eq 0) 'quits with code 0 after the soak'
} finally {
  Stop-Started
  Restore-Clipboard $backup
  Remove-Work
}
if ($failures.Count) { Write-Host "`n$($failures.Count) check(s) failed"; exit 1 }
Write-Host "`nall checks passed"
