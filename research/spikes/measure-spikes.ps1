$ErrorActionPreference = 'Stop'
$sp = Split-Path -Parent $MyInvocation.MyCommand.Path
$sampler = Join-Path $PSScriptRoot '..\..\..\.agents\skills\lezneve-lean-engineering\scripts\measure-windows.ps1'
$out = Join-Path $sp 'metrics'
New-Item -ItemType Directory -Force $out | Out-Null

$cases = @(
  @{ name = 'native-tray';         exe = "$sp\native-spike\target\release\native-spike.exe"; arg = '';          settle = 10 },
  @{ name = 'tauri-tray-nowindow'; exe = "$sp\tauri-spike\target\release\tauri-spike.exe";   arg = '';          settle = 10 },
  @{ name = 'tauri-window-open';   exe = "$sp\tauri-spike\target\release\tauri-spike.exe";   arg = 'open';      settle = 15 },
  @{ name = 'tauri-after-close';   exe = "$sp\tauri-spike\target\release\tauri-spike.exe";   arg = 'openclose'; settle = 40 }
)

foreach ($run in 1..3) {
  foreach ($c in $cases) {
    $p = if ($c.arg) { Start-Process $c.exe -ArgumentList $c.arg -PassThru } else { Start-Process $c.exe -PassThru }
    Start-Sleep -Seconds $c.settle
    $file = Join-Path $out "$($c.name)-$run.json"
    & powershell -NoProfile -ExecutionPolicy Bypass -File $sampler -RootProcessId $p.Id -Scenario $c.name -DurationSeconds 15 -IntervalMs 1000 -OutputPath $file | Out-Null
    # webview2 processes that belong to the spike but are not in the tree
    $tree = (Get-Content $file -Raw | ConvertFrom-Json).samples[-1].processes.id
    $orphans = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.CommandLine -match 'dev\.lezneve\.spike' -and $tree -notcontains $_.ProcessId })
    $cpu0 = (Get-Process -Id $p.Id).TotalProcessorTime
    Start-Sleep -Seconds 10
    $cpu1 = (Get-Process -Id $p.Id).TotalProcessorTime
    $d = Get-Content $file -Raw | ConvertFrom-Json
    "{0}#{1}: ws avg {2} MiB, private avg {3} MiB, procs {4}, orphan webview {5}, host cpu {6} ms / 10 s" -f $c.name, $run, $d.working_set_mb.avg, $d.private_bytes_mb.avg, $d.process_count.avg, $orphans.Count, [math]::Round(($cpu1 - $cpu0).TotalMilliseconds)
    Stop-Process -Id $p.Id -Force
    Start-Sleep -Seconds 3
    Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.CommandLine -match 'dev\.lezneve\.spike' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  }
}
