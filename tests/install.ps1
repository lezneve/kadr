# installer, update and uninstall in a sandbox: KADR_TEST_SANDBOX sends every
# path (program, shortcut, settings) to a temp folder and uses "kadr-test"
# registry names, so nothing real is installed, registered or removed.
# no keyboard or mouse input is injected: buttons get BM_CLICK sent to the
# control itself, menus get keys posted to the menu window.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\install.ps1 [-Out <folder for screenshots>]
param([string] $Out = '')
. "$PSScriptRoot\common.ps1"
$BM_CLICK = 0x00F5
$sandbox = Join-Path $work 'sandbox'
New-Item -ItemType Directory -Force $sandbox | Out-Null
$env:KADR_TEST_SANDBOX = $sandbox
# every capture in this suite is the synthetic pattern, never the screen
$env:KADR_TEST_FRAME = '1'
$setup = Join-Path $work 'kadr-0.1.0-setup.exe'
Copy-Item $exe $setup
$installed = Join-Path $sandbox 'Programs\kadr\kadr.exe'
$settingsFile = Join-Path $sandbox 'AppData\kadr\settings.ini'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\kadr-test'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$MOD_ALT = 0x1; $VK_F9 = 0x78

function Find-Class([string] $class, [int] $tries = 100) {
  for ($i = 0; $i -lt $tries; $i++) { $h = [KadrTest]::FindWindow($class, [NullString]::Value); if ($h -ne [IntPtr]::Zero) { return $h }; Start-Sleep -Milliseconds 40 }
  [IntPtr]::Zero
}
# the installer window once it is ready: visible, with its buttons labelled
function Installer-Ready([int] $tries = 150) {
  for ($i = 0; $i -lt $tries; $i++) {
    $h = [KadrTest]::FindWindow('kadr.installer', [NullString]::Value)
    if ($h -ne [IntPtr]::Zero) {
      $ok = [KadrTest]::GetDlgItem($h, 1)
      if ($ok -ne [IntPtr]::Zero -and ([KadrTest]::Accessible($ok) -match '^43\|(install|update|close)\|')) { Start-Sleep -Milliseconds 150; return $h }
    }
    Start-Sleep -Milliseconds 40
  }
  [IntPtr]::Zero
}
function Shot($h, [string] $name) { if ($Out) { New-Item -ItemType Directory -Force $Out | Out-Null; [KadrTest]::SaveWindow($h, (Join-Path $Out $name)) } }
function Click($h, [int] $id) { [void][KadrTest]::SendMessage([KadrTest]::GetDlgItem($h, $id), $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero) }
function Installed-Kadr { @(Get-CimInstance Win32_Process -Filter "Name='kadr.exe'" | Where-Object { $_.ExecutablePath -eq $installed }) }
function Sandbox-Kadr { @(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -like "$work*" }) }
function Dialogs([uint32] $procId) { [KadrTest]::Children([IntPtr]::Zero) | Out-Null; @() }
# the next message box of a process that is not `$not`
function Box($p, $not = [IntPtr]::Zero) {
  for ($i = 0; $i -lt 120; $i++) { $d = [KadrTest]::WindowOf([uint32]$p.Id, '#32770'); if ($d -ne [IntPtr]::Zero -and $d -ne $not) { return $d }; Start-Sleep -Milliseconds 50 }
  [IntPtr]::Zero
}
function Open-Tray-Settings([IntPtr] $main, [uint32] $procId) {
  [void][KadrTest]::PostMessage($main, $WM_TRAY, (Pack-Point 1200 900), [IntPtr]0x007B)
  $m = [IntPtr]::Zero
  for ($i = 0; $i -lt 50 -and $m -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 40; $m = [KadrTest]::WindowOf($procId, '#32768') }
  Start-Sleep -Milliseconds 120
  foreach ($i in 1..4) { [void][KadrTest]::PostMessage($m, $WM_KEYDOWN, [IntPtr]0x28, [IntPtr]::Zero) }
  [void][KadrTest]::PostMessage($m, $WM_KEYDOWN, [IntPtr]0x0D, [IntPtr]::Zero)
  Find-Class 'kadr.settings'
}
# runs the installer: options as clicks on the two switches, then install
function Run-Setup([string] $path, [bool] $autostartOn, [bool] $runNow, [switch] $StandardUser, [string] $shot = '') {
  if ($StandardUser) { $id = [KadrTest]::StartStandardUser("`"$path`"", (Join-Path $work 'setup-err.txt')); $p = Get-Process -Id $id; $null = $p.Handle }
  else { $p = Start-Process $path -PassThru }
  $h = Installer-Ready
  if ($h -eq [IntPtr]::Zero) { return $null }
  if ($shot) { Shot $h $shot }
  $acc = @([KadrTest]::Children($h) | ForEach-Object { [KadrTest]::Accessible($_) })
  $autostartChecked = ($acc -match '^44\|start with windows\|') -and ([Convert]::ToInt32((($acc -match '^44\|start with windows\|')[0] -split '\|')[2], 16) -band 0x10)
  if ([bool]$autostartChecked -ne $autostartOn) { Click $h 10 }
  if (-not $runNow) { Click $h 11 }
  # install posted, not sent: the installer then works on its own thread
  # schedule, exactly as after a real click
  [void][KadrTest]::PostMessage([KadrTest]::GetDlgItem($h, 1), $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
  for ($i = 0; $i -lt 100; $i++) { Start-Sleep -Milliseconds 100; if (@([KadrTest]::Children($h) | ForEach-Object { [KadrTest]::Accessible($_) }) -match '^43\|close\|') { break } }
  Start-Sleep -Milliseconds 1200
  [pscustomobject]@{ process = $p; window = $h; accessible = $acc }
}
function Finish-Setup($s) { Click $s.window 1; $s.process.WaitForExit(4000) | Out-Null; $s.process }
# windows settings > apps runs "<installed> --uninstall"
function Run-Uninstall([bool] $removeSettings, [switch] $StandardUser) {
  if ($StandardUser) { $id = [KadrTest]::StartStandardUser("`"$installed`" --uninstall", (Join-Path $work 'uninstall-err.txt')); $u = Get-Process -Id $id; $null = $u.Handle }
  else { $u = Start-Process $installed -ArgumentList '--uninstall' -PassThru }
  $q1 = Box $u
  Click $q1 6                                        # yes: remove kadr
  $asked = $false
  if (Test-Path $settingsFile) {
    $q2 = Box $u $q1
    $asked = $q2 -ne [IntPtr]::Zero
    Click $q2 $(if ($removeSettings) { 6 } else { 7 })   # yes / no
    $q1 = $q2
  }
  $done = Box $u $q1
  [void][KadrTest]::PostMessage($done, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
  $u.WaitForExit(6000) | Out-Null
  # the program folder goes shortly after the uninstaller exits (retried for up to ~20 s)
  for ($i = 0; $i -lt 60 -and (Test-Path (Split-Path $installed)); $i++) { Start-Sleep -Milliseconds 500 }
  [pscustomobject]@{ asked_about_settings = $asked; exit = $u.ExitCode }
}
function Settings-Hash { if (Test-Path $settingsFile) { (Get-FileHash $settingsFile).Hash } else { '' } }

try {
  Write-Host "1. renamed exe files"
  foreach ($name in 'kadr-portable.exe', 'kadr-setup-old.exe', 'setup.exe', 'kadr (setup).exe') {
    $copy = Join-Path $work $name; Copy-Item $exe $copy
    $p = Start-Process $copy -PassThru
    $main = Find-Class 'kadr.main' 60
    $inst = [KadrTest]::FindWindow('kadr.installer', [NullString]::Value)
    Check ($main -ne [IntPtr]::Zero -and $inst -eq [IntPtr]::Zero) "'$name' runs as normal kadr, not the installer"
    if ($main -ne [IntPtr]::Zero) { [void][KadrTest]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) }
    $p.WaitForExit(4000) | Out-Null
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
  }
  $second = Join-Path $work 'kadr-0.1.0-setup (1).exe'; Copy-Item $exe $second
  $p = Start-Process $second -PassThru
  $h = Installer-Ready
  Check ($h -ne [IntPtr]::Zero) "'kadr-0.1.0-setup (1).exe' (a second browser download) is the installer"
  [void][KadrTest]::PostMessage([KadrTest]::GetDlgItem($h, 1), $WM_KEYDOWN, [IntPtr]0x1B, [IntPtr]::Zero)
  $p.WaitForExit(3000) | Out-Null
  Check ($p.HasExited -and -not (Test-Path $installed) -and -not (Test-Path $uninstallKey)) 'esc closes it and installs nothing'

  foreach ($theme in 'dark', 'light') {
    $env:KADR_THEME = $theme
    Write-Host "`n2. ${theme}: install"
    $s = Run-Setup $setup $true $false -shot "installer-$theme.png"
    Check ($null -ne $s) "${theme}: installer opens"
    Check ((($s.accessible -match '^44\|').Count -eq 2) -and (($s.accessible -match '^43\|install\|').Count -eq 1)) "${theme}: two check boxes and an install button for screen readers"
    Shot $s.window "installer-$theme-done.png"
    Check (Test-Path $installed) "${theme}: kadr.exe in place"
    Check (Test-Path (Join-Path $sandbox 'StartMenu\kadr.lnk')) "${theme}: start menu shortcut"
    $entry = Get-ItemProperty $uninstallKey -ErrorAction SilentlyContinue
    Check ($entry.DisplayName -eq 'kadr' -and $entry.DisplayVersion -eq '0.1.0' -and $entry.Publisher -eq 'lezneve' -and $entry.UninstallString -eq "`"$installed`" --uninstall") "${theme}: apps entry"
    Check ((Get-ItemProperty $runKey -ErrorAction SilentlyContinue).'kadr-test' -eq "`"$installed`"") "${theme}: start with windows set"
    Check (@(Installed-Kadr).Count -eq 0) "${theme}: not started (start now off)"
    [void](Finish-Setup $s)

    Write-Host "`n3. ${theme}: update keeps settings and closes every kadr process"
    New-Item -ItemType Directory -Force (Split-Path $settingsFile) | Out-Null
    [IO.File]::WriteAllText($settingsFile, "hotkey_region = alt+f9`r`nafter_capture = edit`r`n")
    $before = Settings-Hash
    $old = Start-Process $installed -PassThru
    $main = Find-Class 'kadr.main'
    $sw = Open-Tray-Settings $main ([uint32]$old.Id)
    $oldSettings = Get-Process -Id ([KadrTest]::PidOf($sw)) -ErrorAction SilentlyContinue
    Check ($null -ne $oldSettings) "${theme}: the installed kadr runs, with its settings window open"
    # an open editor (synthetic screen) runs from the same exe and must close too
    [void][KadrTest]::PostMessage($main, 0x0312, [IntPtr]3, [IntPtr]::Zero)
    $edw = [IntPtr]::Zero
    for ($i = 0; $i -lt 100 -and $edw -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 30; $edw = [KadrTest]::FindWindow('kadr.editor', [NullString]::Value) }
    $oldEditor = if ($edw -ne [IntPtr]::Zero) { Get-Process -Id ([KadrTest]::PidOf($edw)) -ErrorAction SilentlyContinue }
    Check ($null -ne $oldEditor) "${theme}: and an editor open"
    # with a rectangle drawn in it: unsaved. the update must ask through the
    # editor; a helper answers "don't save" when the question appears.
    if ($edw -ne [IntPtr]::Zero) {
      [void][KadrTest]::PostPhysical($edw, 0x0201, [IntPtr]1, [IntPtr]((20 -shl 16) -bor 20))
      [void][KadrTest]::PostPhysical($edw, 0x0200, [IntPtr]1, [IntPtr]((80 -shl 16) -bor 80))
      [void][KadrTest]::PostPhysical($edw, 0x0202, [IntPtr]0, [IntPtr]((80 -shl 16) -bor 80))
    }
    $answerJob = Start-Job -ArgumentList (Join-Path $PSScriptRoot 'input.cs'), $oldEditor.Id -ScriptBlock {
      param($cs, $edPid)
      Add-Type -Path $cs -ReferencedAssemblies System.Drawing
      for ($i = 0; $i -lt 300; $i++) {
        $d = [KadrTest]::WindowOf([uint32]$edPid, '#32770')
        if ($d -ne [IntPtr]::Zero) { Start-Sleep -Milliseconds 200; [void][KadrTest]::PostMessage([KadrTest]::GetDlgItem($d, 7), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero); return $true }
        Start-Sleep -Milliseconds 50
      }
      $false
    }
    $s = Run-Setup $setup $false $true -shot "installer-$theme-update.png"
    Check ((($s.accessible -match '^43\|update\|').Count -eq 1)) "${theme}: an installed version turns install into update"
    Check ($old.HasExited) "${theme}: update closed the running kadr"
    Check ($oldSettings.HasExited) "${theme}: and its settings window process"
    $asked = Receive-Job $answerJob -Wait -AutoRemoveJob
    Check ($asked -eq $true) "${theme}: the editor with unsaved annotations asked before the update closed it"
    Check ($oldEditor -and $oldEditor.HasExited) "${theme}: and the open editor"
    Check ((Settings-Hash) -eq $before) "${theme}: settings.ini is unchanged byte for byte"
    $now = @(Installed-Kadr)
    Check ($now.Count -eq 1 -and $now[0].ProcessId -ne $old.Id) "${theme}: the new kadr is running"
    Check (-not (Hotkey-Free ($MOD_ALT -bor $MOD_NOREPEAT) $VK_F9)) "${theme}: and uses the kept settings (alt+f9 is registered)"
    Check ($null -eq (Get-ItemProperty $runKey -ErrorAction SilentlyContinue).'kadr-test') "${theme}: start with windows off removes the run value"
    [void](Finish-Setup $s)

    if ($theme -eq 'dark') {
      Write-Host "`n3b. cancel in an editor with unsaved annotations stops update and uninstall"
      function Unsaved-Editor {
        $m = Find-Class 'kadr.main'
        [void][KadrTest]::PostMessage($m, 0x0312, [IntPtr]3, [IntPtr]::Zero)
        $w = [IntPtr]::Zero
        for ($i = 0; $i -lt 100 -and $w -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 30; $w = [KadrTest]::FindWindow('kadr.editor', [NullString]::Value) }
        Start-Sleep -Milliseconds 300
        [void][KadrTest]::PostPhysical($w, 0x0201, [IntPtr]1, [IntPtr]((20 -shl 16) -bor 20))
        [void][KadrTest]::PostPhysical($w, 0x0200, [IntPtr]1, [IntPtr]((80 -shl 16) -bor 80))
        [void][KadrTest]::PostPhysical($w, 0x0202, [IntPtr]0, [IntPtr]((80 -shl 16) -bor 80))
        Start-Sleep -Milliseconds 300
        $w
      }
      function Answer-Editor([uint32] $edPid, [int] $button) {
        Start-Job -ArgumentList (Join-Path $PSScriptRoot 'input.cs'), $edPid, $button -ScriptBlock {
          param($cs, $edPid, $button)
          Add-Type -Path $cs -ReferencedAssemblies System.Drawing
          for ($i = 0; $i -lt 300; $i++) {
            $d = [KadrTest]::WindowOf([uint32]$edPid, '#32770')
            if ($d -ne [IntPtr]::Zero) { Start-Sleep -Milliseconds 200; [void][KadrTest]::PostMessage([KadrTest]::GetDlgItem($d, $button), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero); return $true }
            Start-Sleep -Milliseconds 50
          }
          $false
        }
      }
      $edw = Unsaved-Editor
      $edp = [KadrTest]::PidOf($edw)
      $kadrPid = @(Installed-Kadr)[0].ProcessId
      $hash = (Get-FileHash $installed).Hash
      $job = Answer-Editor $edp 2   # cancel
      $sw = [Diagnostics.Stopwatch]::StartNew()
      $sp = Start-Process $setup -PassThru
      $ih = Installer-Ready
      [void][KadrTest]::PostMessage([KadrTest]::GetDlgItem($ih, 1), $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
      $asked = Receive-Job $job -Wait -AutoRemoveJob
      $err = Box $sp
      $waited = $sw.Elapsed.TotalSeconds
      Check ($asked -eq $true) 'update: the editor asked'
      Check ($err -ne [IntPtr]::Zero -and $waited -lt 15) ("update: cancel stops the installer at once with a message ({0:n1} s)" -f $waited)
      Check ([bool](Get-Process -Id $edp -ErrorAction SilentlyContinue)) 'update: the editor and its annotations stay open'
      Check ([bool](Get-Process -Id $kadrPid -ErrorAction SilentlyContinue) -and (Get-FileHash $installed).Hash -eq $hash) 'update: kadr keeps running, the installed exe is untouched'
      if ($err -ne [IntPtr]::Zero) { [void][KadrTest]::PostMessage($err, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) }
      Start-Sleep -Milliseconds 300
      [void][KadrTest]::PostMessage($ih, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
      $sp.WaitForExit(4000) | Out-Null
      if (-not $sp.HasExited) { Stop-Process -Id $sp.Id -Force }

      $job = Answer-Editor $edp 2   # cancel again, now for uninstall
      $u = Start-Process $installed -ArgumentList '--uninstall' -PassThru
      $q1 = Box $u; Click $q1 6
      $q2 = Box $u $q1; Click $q2 7         # keep settings
      $asked = Receive-Job $job -Wait -AutoRemoveJob
      $done = Box $u $q2
      Check ($asked -eq $true -and $done -ne [IntPtr]::Zero) 'uninstall: the editor asked; cancel ends the uninstall with a message'
      Check ((Test-Path $installed) -and (Test-Path $uninstallKey) -and [bool](Get-Process -Id $edp -ErrorAction SilentlyContinue)) 'uninstall: nothing removed, the editor stays open'
      if ($done -ne [IntPtr]::Zero) { [void][KadrTest]::PostMessage($done, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) }
      $u.WaitForExit(4000) | Out-Null
      # clean up: close the editor without saving
      $job = Answer-Editor $edp 7
      [void][KadrTest]::PostMessage($edw, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
      [void](Receive-Job $job -Wait -AutoRemoveJob)
      for ($i = 0; $i -lt 50 -and (Get-Process -Id $edp -ErrorAction SilentlyContinue); $i++) { Start-Sleep -Milliseconds 100 }
      if (-not (Get-Process -Id $kadrPid -ErrorAction SilentlyContinue)) { Start-Process $installed | Out-Null; [void](Find-Class 'kadr.main') }
    }

    Write-Host "`n4. ${theme}: uninstall"
    $main = Find-Class 'kadr.main'
    $running = @(Installed-Kadr)
    $sw = Open-Tray-Settings $main ([uint32]$running[0].ProcessId)
    $settingsProc = Get-Process -Id ([KadrTest]::PidOf($sw)) -ErrorAction SilentlyContinue
    $keep = ($theme -eq 'dark')
    # as if kadr had turned off windows' printscreen → snipping tool (stand-in keys)
    $fakeKb = 'HKCU:\Software\kadr-test\Keyboard'; $fakeState = 'HKCU:\Software\kadr-test\State'
    # dark: kadr turned it off (it was not set) -> uninstall removes kadr's value.
    # light: changed again after kadr (the user turned it back on) -> left alone.
    $changedLater = ($theme -eq 'light')
    New-Item $fakeKb -Force | Out-Null; New-Item $fakeState -Force | Out-Null
    New-ItemProperty $fakeKb -Name PrintScreenKeyForSnippingEnabled -Value $(if ($changedLater) { 1 } else { 0 }) -PropertyType DWord -Force | Out-Null
    New-ItemProperty $fakeState -Name PrintScreenSnippingBefore -Value 2 -PropertyType DWord -Force | Out-Null
    $r = Run-Uninstall (-not $keep)
    $restored = (Get-ItemProperty $fakeKb -ErrorAction SilentlyContinue).PrintScreenKeyForSnippingEnabled
    if ($changedLater) { Check ($restored -eq 1 -and -not (Test-Path $fakeState)) "${theme}: a value changed after kadr is left alone ($restored)" }
    else { Check ($null -eq $restored -and -not (Test-Path $fakeState)) "${theme}: printscreen setting put back as it was (not set)" }
    Remove-Item 'HKCU:\Software\kadr-test' -Recurse -Force -ErrorAction SilentlyContinue
    Check ($r.asked_about_settings) "${theme}: settings are a separate question"
    Check (@(Installed-Kadr).Count -eq 0 -and $settingsProc.HasExited) "${theme}: kadr and its settings window were closed"
    Check (-not (Test-Path (Split-Path $installed))) "${theme}: program folder removed"
    Check (-not (Test-Path (Join-Path $sandbox 'StartMenu\kadr.lnk')) -and -not (Test-Path $uninstallKey)) "${theme}: shortcut and apps entry removed"
    Check ($null -eq (Get-ItemProperty $runKey -ErrorAction SilentlyContinue).'kadr-test') "${theme}: no start with windows value"
    if ($keep) { Check ((Settings-Hash) -eq $before) "${theme}: answered no: settings kept, unchanged" }
    else { Check (-not (Test-Path (Split-Path $settingsFile))) "${theme}: answered yes: settings removed" }
  }

  Write-Host "`n5. standard-user token (what uac gives a normal session)"
  Remove-Item Env:\KADR_THEME -ErrorAction SilentlyContinue
  $s = Run-Setup $setup $false $true -StandardUser
  Check ($null -ne $s) 'installer opens with a standard-user token (no elevation needed)'
  Check ([KadrTest]::TokenInfo([uint32]$s.process.Id) -eq 'elevated=False integrity=0x2000') 'installer is not elevated'
  Check (Test-Path $installed) 'installs without admin rights'
  $k = @(Installed-Kadr)
  Check ($k.Count -eq 1 -and [KadrTest]::TokenInfo([uint32]$k[0].ProcessId) -eq 'elevated=False integrity=0x2000') 'starts kadr as a standard user'
  if ($k.Count) { $kp = Get-Process -Id $k[0].ProcessId; [void](Box $kp | ForEach-Object { [void][KadrTest]::PostMessage($_, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) }) }  # uac-off machine: tray notice
  [void](Finish-Setup $s)
  $r = Run-Uninstall $true -StandardUser
  Check (@(Installed-Kadr).Count -eq 0 -and -not (Test-Path (Split-Path $installed))) 'uninstalls as a standard user, nothing left'
  Check (@(Sandbox-Kadr).Count -eq 0) 'no kadr process left from any part of this test'
} finally {
  Remove-Item Env:\KADR_THEME, Env:\KADR_TEST_SANDBOX -ErrorAction SilentlyContinue
  Sandbox-Kadr | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  Remove-Item $uninstallKey -Recurse -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 500
  Get-ChildItem ([IO.Path]::GetTempPath()) -Filter 'kadr-uninstall-*.exe' -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
  Remove-ItemProperty $runKey -Name 'kadr-test' -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 300
  Remove-Work
}
if ($failures.Count) { Write-Host "`n$($failures.Count) check(s) failed"; exit 1 }
Write-Host "`nall checks passed"
