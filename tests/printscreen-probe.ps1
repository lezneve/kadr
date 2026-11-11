# manual check (docs/manual-checklist.md 2.11): with kadr running and bare
# printscreen as its region hotkey, can another app register bare printscreen
# too? read-only: registers and immediately releases a hotkey on this thread;
# presses no keys and changes no settings.
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class Probe {
  [DllImport("user32.dll", SetLastError = true)] public static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);
  [DllImport("user32.dll")] public static extern bool UnregisterHotKey(IntPtr h, int id);
}
'@
$v = Get-ItemProperty 'HKCU:\Control Panel\Keyboard' -Name PrintScreenKeyForSnippingEnabled -ErrorAction SilentlyContinue
$state = if ($null -eq $v) { 'not set (windows default)' } else { $v.PrintScreenKeyForSnippingEnabled }
$ok = [Probe]::RegisterHotKey([IntPtr]::Zero, 991, 0x4000, 0x2C)
$err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
if ($ok) { [void][Probe]::UnregisterHotKey([IntPtr]::Zero, 991) }
Write-Host "windows printscreen -> snipping tool: $state"
Write-Host "kadr running: $([bool](Get-Process kadr -ErrorAction SilentlyContinue))"
Write-Host $(if ($ok) { 'a second app CAN register bare printscreen' } else { "a second app cannot register bare printscreen (error $err)" })
