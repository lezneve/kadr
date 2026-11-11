# native settings window vs the same page in tauri (webview2), same machine,
# same conditions. both run as their own process, as a settings window would.
# measures: launch -> window, process tree and memory while open, a window
# screenshot, and what ui automation (screen readers) can see.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File research\settings-compare.ps1 -Out <folder>
param([Parameter(Mandatory = $true)] [string] $Out, [int] $Runs = 3)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Add-Type -Path (Join-Path $root 'tests\input.cs') -ReferencedAssemblies System.Drawing
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
New-Item -ItemType Directory -Force $Out | Out-Null
$ini = Join-Path $Out 'settings.ini'
[IO.File]::WriteAllText($ini, "after_capture = copy+save`r`n")

$variants = @(
  @{ name = 'native'; exe = (Join-Path $root 'target\release\kadr.exe'); args = @('--settings-window', $ini); class = 'kadr.settings' },
  @{ name = 'tauri'; exe = (Join-Path $root 'research\spikes\tauri-spike\target\release\tauri-spike.exe'); args = @('open'); class = $null }
)

function Tree([int] $rootId) {
  $all = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId)
  $ids = New-Object System.Collections.Generic.HashSet[int]; [void]$ids.Add($rootId)
  do { $n = $ids.Count; foreach ($x in $all) { if ($ids.Contains([int]$x.ParentProcessId)) { [void]$ids.Add([int]$x.ProcessId) } } } while ($ids.Count -ne $n)
  $ps = @($ids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
  [pscustomobject]@{ count = $ps.Count; private = [math]::Round((($ps | Measure-Object PrivateMemorySize64 -Sum).Sum) / 1MB, 1); ws = [math]::Round((($ps | Measure-Object WorkingSet64 -Sum).Sum) / 1MB, 1) }
}
function Find-Window($v, [int] $procId) {
  if ($v.class) { return [KadrTest]::FindWindow($v.class, [NullString]::Value) }
  (Get-Process -Id $procId -ErrorAction SilentlyContinue).MainWindowHandle
}
function Uia([IntPtr] $h) {
  $el = [Windows.Automation.AutomationElement]::FromHandle($h)
  $all = $el.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
  $items = foreach ($e in $all) {
    $t = $e.Current.ControlType.ProgrammaticName -replace 'ControlType\.', ''
    if ($t -in 'Button', 'RadioButton', 'CheckBox', 'Edit', 'ToggleButton') { "{0}: {1}" -f $t, $e.Current.Name }
  }
  @($items)
}

$report = @()
foreach ($v in $variants) {
  $times = @(); $mems = @()
  for ($r = 1; $r -le $Runs; $r++) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process $v.exe -ArgumentList $v.args -PassThru
    $h = [IntPtr]::Zero
    while ($h -eq [IntPtr]::Zero -and $sw.ElapsedMilliseconds -lt 10000) { Start-Sleep -Milliseconds 5; $h = Find-Window $v $p.Id }
    $times += $sw.ElapsedMilliseconds
    Start-Sleep -Seconds 3
    $mems += Tree $p.Id
    if ($r -eq 1) {
      [KadrTest]::SaveWindow($h, (Join-Path $Out "$($v.name).png"))
      $uia = Uia $h
    }
    Get-Process -Id (Tree $p.Id | ForEach-Object { $p.Id }) -ErrorAction SilentlyContinue | Out-Null
    Stop-Process -Id $p.Id -Force
    Start-Sleep -Seconds 2
    Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.CommandLine -match 'dev\.lezneve\.spike' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  }
  $report += [pscustomobject]@{
    variant = $v.name
    window_ms = ($times | Measure-Object -Minimum -Maximum -Average | ForEach-Object { "{0}/{1:N0}/{2} min/avg/max" -f $_.Minimum, $_.Average, $_.Maximum })
    processes = ($mems | ForEach-Object count) -join ','
    private_mb = ($mems | ForEach-Object private) -join ', '
    ws_mb = ($mems | ForEach-Object ws) -join ', '
    exe_mb = [math]::Round((Get-Item $v.exe).Length / 1MB, 2)
    uia = ($uia -join ' | ')
  }
}
Remove-Item $ini
$report | Format-List
