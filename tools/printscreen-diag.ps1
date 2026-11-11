# where does the printscreen key go? run it yourself, then press printscreen
# a few times within 20 seconds.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\printscreen-diag.ps1
#
# it watches only the printscreen key: every other key is ignored and never
# recorded or shown. it registers bare printscreen as a hotkey (like kadr) and
# also listens at the lowest level windows offers to apps, then reports:
#   - key seen at the lowest level, hotkey received  -> printscreen reaches apps
#   - key seen, hotkey not received                  -> another app holds the hotkey first
#   - key not seen at all                            -> a driver or utility (fn keys,
#                                                       vendor tools) swallows or remaps it
# quit kadr first (tray -> quit), so the two do not compete for the hotkey.
param([int] $Seconds = 20)
if (Get-Process kadr -ErrorAction SilentlyContinue) {
  Write-Host "kadr is running. quit it first (tray icon -> quit), then run this again."
  exit 1
}
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Diagnostics;
public static class PrtScDiag {
  delegate IntPtr HookProc(int code, IntPtr w, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] struct KBD { public uint vk, scan, flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr h; public uint msg; public IntPtr w, l; public uint time; public int x, y; }
  [DllImport("user32.dll", SetLastError = true)] static extern IntPtr SetWindowsHookEx(int id, HookProc p, IntPtr mod, uint thread);
  [DllImport("user32.dll")] static extern bool UnhookWindowsHookEx(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr CallNextHookEx(IntPtr h, int c, IntPtr w, IntPtr l);
  [DllImport("user32.dll", SetLastError = true)] static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);
  [DllImport("user32.dll")] static extern bool UnregisterHotKey(IntPtr h, int id);
  [DllImport("user32.dll")] static extern bool PeekMessage(out MSG m, IntPtr h, uint a, uint b, uint remove);
  [DllImport("user32.dll")] static extern uint MsgWaitForMultipleObjects(uint n, IntPtr[] h, bool all, uint ms, uint mask);
  [DllImport("kernel32.dll")] static extern IntPtr GetModuleHandle(string name);
  static HookProc keep;
  public static int Seen, Hotkeys;
  public static string Run(int seconds) {
    bool registered = RegisterHotKey(IntPtr.Zero, 1, 0x4000, 0x2C);
    string reg = registered ? "registered" : ("not registered, error " + Marshal.GetLastWin32Error());
    keep = (code, w, l) => {
      if (code >= 0) {
        var k = (KBD)Marshal.PtrToStructure(l, typeof(KBD));
        // only printscreen: vk 0x2C, or its scan code 0x37 / 0x54 under another vk
        if (k.vk == 0x2C || k.scan == 0x37 || k.scan == 0x54) {
          if ((int)w == 0x100 || (int)w == 0x104) { Seen++; Console.WriteLine("  key down: vk 0x{0:x2} scan 0x{1:x2} flags 0x{2:x2}", k.vk, k.scan, k.flags); }
        }
      }
      return CallNextHookEx(IntPtr.Zero, code, w, l);
    };
    IntPtr hook = SetWindowsHookEx(13, keep, GetModuleHandle(null), 0);
    var sw = Stopwatch.StartNew();
    while (sw.ElapsedMilliseconds < seconds * 1000) {
      MsgWaitForMultipleObjects(0, null, false, 100, 0x04FF);
      MSG m;
      while (PeekMessage(out m, IntPtr.Zero, 0, 0, 1)) { if (m.msg == 0x0312) { Hotkeys++; Console.WriteLine("  hotkey received"); } }
    }
    UnhookWindowsHookEx(hook);
    if (registered) UnregisterHotKey(IntPtr.Zero, 1);
    return reg;
  }
}
'@
$v = Get-ItemProperty 'HKCU:\Control Panel\Keyboard' -Name PrintScreenKeyForSnippingEnabled -ErrorAction SilentlyContinue
Write-Host ("windows 'print screen opens screen capture': {0}" -f $(if ($null -eq $v) { 'not set (default on)' } else { $v.PrintScreenKeyForSnippingEnabled }))
Write-Host "press printscreen a few times now ($Seconds s)..."
$reg = [PrtScDiag]::Run($Seconds)
Write-Host ""
Write-Host "hotkey: $reg"
Write-Host ("printscreen presses seen: {0}, hotkeys received: {1}" -f [PrtScDiag]::Seen, [PrtScDiag]::Hotkeys)
if ([PrtScDiag]::Seen -eq 0) { Write-Host "-> the key never reached windows apps: a driver or a utility (fn keys, vendor software) takes or remaps it." }
elseif ([PrtScDiag]::Hotkeys -eq 0) { Write-Host "-> the key arrives, but another app gets the hotkey first (or windows still sends it to snipping tool)." }
else { Write-Host "-> printscreen reaches apps normally." }
