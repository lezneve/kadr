// test helper for tests/e2e.ps1: synthetic keyboard/mouse input and a few
// win32 queries. compiled at runtime by Add-Type; not part of kadr.
using System;
using System.Runtime.InteropServices;
using System.Threading;

public static class KadrTest {
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint data, dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] struct UNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public UNION u; }

    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process, uint flags);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);
    [DllImport("user32.dll")] public static extern bool UnregisterHotKey(IntPtr h, int id);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, System.Text.StringBuilder sb, int n);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

    public const ushort CTRL = 0x11, ALT = 0x12, SHIFT = 0x10, LWIN = 0x5B, PRTSC = 0x2C, ESC = 0x1B, F12 = 0x7B;

    static void Physical() { SetThreadDpiAwarenessContext(new IntPtr(-4)); }

    static INPUT Key(ushort vk, bool up) {
        var i = new INPUT { type = 1 };
        i.u.ki.wVk = vk;
        i.u.ki.dwFlags = up ? 2u : 0u;
        if (vk == PRTSC || vk == LWIN) i.u.ki.dwFlags |= 1u; // extended key
        return i;
    }

    // press modifiers + key, release in reverse order
    public static void Chord(params ushort[] keys) {
        var list = new System.Collections.Generic.List<INPUT>();
        foreach (var k in keys) list.Add(Key(k, false));
        for (int i = keys.Length - 1; i >= 0; i--) list.Add(Key(keys[i], true));
        SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
    }

    static void Button(uint flags) {
        var i = new INPUT { type = 0 };
        i.u.mi.dwFlags = flags;
        SendInput(1, new[] { i }, Marshal.SizeOf(typeof(INPUT)));
    }

    public static void MoveTo(int x, int y) { Physical(); SetCursorPos(x, y); }

    [DllImport("user32.dll")] static extern int GetSystemMetrics(int i);
    // virtual screen origin in physical pixels, "x,y" (negative with a monitor left of or above the primary)
    public static int[] VirtualOrigin() { Physical(); return new[] { GetSystemMetrics(76), GetSystemMetrics(77) }; }

    [StructLayout(LayoutKind.Sequential)] struct POINT { public int x, y; }
    [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT p);
    // cursor position in physical pixels, "x,y"
    public static string CursorPos() { Physical(); POINT p; GetCursorPos(out p); return p.x + "," + p.y; }

    // drag in physical pixels with intermediate moves, like a person would
    public static void Drag(int x0, int y0, int x1, int y1, int steps, int delayMs) {
        Physical();
        SetCursorPos(x0, y0); Thread.Sleep(delayMs);
        Button(0x0002); // left down
        for (int s = 1; s <= steps; s++) {
            SetCursorPos(x0 + (x1 - x0) * s / steps, y0 + (y1 - y0) * s / steps);
            Thread.Sleep(delayMs);
        }
        Button(0x0004); // left up
    }

    public static void RightClick() { Button(0x0008); Button(0x0010); }

    // windows rescales mouse coordinates in messages sent from a dpi-unaware
    // thread (powershell) to a per-monitor-aware window; post from a
    // per-monitor-aware thread so x/y arrive as physical pixels.
    public static bool PostPhysical(IntPtr h, uint msg, IntPtr w, IntPtr l) {
        Physical();
        return PostMessage(h, msg, w, l);
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, string l, uint flags, uint timeout, out IntPtr result);

    // sends `msg` from several threads at once; returns how many were handled
    public static int Flood(IntPtr h, uint msg, string text, int threads, int each) {
        int handled = 0;
        var pool = new Thread[threads];
        for (int t = 0; t < threads; t++) {
            pool[t] = new Thread(() => {
                for (int k = 0; k < each; k++) {
                    IntPtr r;
                    if (SendMessageTimeout(h, msg, IntPtr.Zero, text, 0, 3000, out r) != IntPtr.Zero) Interlocked.Increment(ref handled);
                }
            });
            pool[t].Start();
        }
        foreach (var t in pool) t.Join();
        return handled;
    }

    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("advapi32.dll")] static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll")] static extern bool GetTokenInformation(IntPtr token, int cls, IntPtr info, int len, out int ret);
    [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthority(IntPtr sid, uint index);
    [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthorityCount(IntPtr sid);

    // "elevated=<bool> integrity=<rid>": 0x2000 medium (normal user), 0x3000 high (admin)
    public static string TokenInfo(uint pid) {
        IntPtr p = OpenProcess(0x1000, false, pid); // PROCESS_QUERY_LIMITED_INFORMATION
        if (p == IntPtr.Zero) return "unknown";
        IntPtr t;
        if (!OpenProcessToken(p, 0x0008, out t)) { CloseHandle(p); return "unknown"; }
        IntPtr buf = Marshal.AllocHGlobal(256);
        int ret;
        string result = "unknown";
        if (GetTokenInformation(t, 20, buf, 4, out ret)) { // TokenElevation
            bool elevated = Marshal.ReadInt32(buf) != 0;
            if (GetTokenInformation(t, 25, buf, 256, out ret)) { // TokenIntegrityLevel
                IntPtr sid = Marshal.ReadIntPtr(buf);
                int count = Marshal.ReadByte(GetSidSubAuthorityCount(sid));
                int rid = Marshal.ReadInt32(GetSidSubAuthority(sid, (uint)(count - 1)));
                result = string.Format("elevated={0} integrity=0x{1:x}", elevated, rid);
            }
        }
        Marshal.FreeHGlobal(buf); CloseHandle(t); CloseHandle(p);
        return result;
    }

    // ---- starting a process with a standard-user token -------------------
    // with uac turned off (EnableLUA=0) every process of an admin account,
    // explorer included, is elevated. to test kadr as a standard user we build
    // the token uac would hand out: a LUA token without admin rights, at
    // medium integrity. nothing in the system is changed.
    [StructLayout(LayoutKind.Sequential)] struct SID_AND_ATTRIBUTES { public IntPtr Sid; public uint Attributes; }
    [StructLayout(LayoutKind.Sequential)] struct TOKEN_MANDATORY_LABEL { public SID_AND_ATTRIBUTES Label; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] struct STARTUPINFO {
        public int cb; public string reserved, desktop, title; public int x, y, w, h, xc, yc, fill; public uint flags; public short show, r2; public IntPtr r3, stdIn, stdOut, stdErr; }
    [StructLayout(LayoutKind.Sequential)] struct PROCESS_INFORMATION { public IntPtr process, thread; public uint pid, tid; }
    [StructLayout(LayoutKind.Sequential)] struct SECURITY_ATTRIBUTES { public int length; public IntPtr descriptor; public bool inherit; }

    [DllImport("advapi32.dll", SetLastError = true)] static extern bool CreateRestrictedToken(IntPtr existing, uint flags, uint disableCount, SID_AND_ATTRIBUTES[] disable, uint delPrivCount, IntPtr privs, uint restrictCount, IntPtr restrict, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool ConvertStringSidToSid(string sid, out IntPtr psid);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool SetTokenInformation(IntPtr token, int cls, ref TOKEN_MANDATORY_LABEL info, int len);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool CreateProcessAsUser(IntPtr token, string app, string cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string dir, ref STARTUPINFO si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateFile(string name, uint access, uint share, ref SECURITY_ATTRIBUTES sa, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);

    public static uint StartStandardUser(string commandLine, string stderrPath) {
        IntPtr own, limited, admins, medium;
        if (!OpenProcessToken(GetCurrentProcess(), 0x0002 | 0x0008 | 0x0001 | 0x0080 | 0x0100, out own)) throw new System.ComponentModel.Win32Exception();
        ConvertStringSidToSid("S-1-5-32-544", out admins);   // BUILTIN\Administrators
        ConvertStringSidToSid("S-1-16-8192", out medium);    // medium mandatory level
        var disable = new[] { new SID_AND_ATTRIBUTES { Sid = admins } };
        // DISABLE_MAX_PRIVILEGE (1) | LUA_TOKEN (4)
        if (!CreateRestrictedToken(own, 1 | 4, 1, disable, 0, IntPtr.Zero, 0, IntPtr.Zero, out limited)) throw new System.ComponentModel.Win32Exception();
        var label = new TOKEN_MANDATORY_LABEL { Label = new SID_AND_ATTRIBUTES { Sid = medium, Attributes = 0x20 } }; // SE_GROUP_INTEGRITY
        if (!SetTokenInformation(limited, 25, ref label, Marshal.SizeOf(typeof(TOKEN_MANDATORY_LABEL)) + 32)) throw new System.ComponentModel.Win32Exception();
        var sa = new SECURITY_ATTRIBUTES { length = Marshal.SizeOf(typeof(SECURITY_ATTRIBUTES)), inherit = true };
        IntPtr err = CreateFile(stderrPath, 0x40000000, 1 | 2, ref sa, 2, 0x80, IntPtr.Zero); // GENERIC_WRITE, CREATE_ALWAYS
        var si = new STARTUPINFO { cb = Marshal.SizeOf(typeof(STARTUPINFO)), flags = 0x100, stdErr = err, stdOut = err }; // STARTF_USESTDHANDLES
        PROCESS_INFORMATION pi;
        bool ok = CreateProcessAsUser(limited, null, commandLine, IntPtr.Zero, IntPtr.Zero, true, 0, IntPtr.Zero, null, ref si, out pi);
        int error = Marshal.GetLastWin32Error();
        CloseHandle(err); CloseHandle(own); CloseHandle(limited); LocalFree(admins); LocalFree(medium);
        if (!ok) throw new System.ComponentModel.Win32Exception(error);
        CloseHandle(pi.thread); CloseHandle(pi.process);
        return pi.pid;
    }

    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr parent, int id);
    public static uint PidOf(IntPtr h) { uint pid; GetWindowThreadProcessId(h, out pid); return pid; }
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("oleacc.dll")] static extern int AccessibleObjectFromWindow(IntPtr h, uint objId, ref Guid iid, [MarshalAs(UnmanagedType.IDispatch)] out object acc);

    // what a screen reader gets for a control window: "role|name|state"
    // (msaa roles: 43 push button, 44 check box, 45 radio button, 42 text)
    public static string Accessible(IntPtr h) {
        var iid = new Guid("618736E0-3C3D-11CF-810C-00AA00389B71"); // IAccessible
        object acc;
        if (AccessibleObjectFromWindow(h, 0xFFFFFFFC, ref iid, out acc) != 0 || acc == null) return "none";
        var t = acc.GetType();
        Func<string, object> get = n => t.InvokeMember(n, System.Reflection.BindingFlags.GetProperty, null, acc, new object[] { 0 });
        return string.Format("{0}|{1}|0x{2:x}", get("accRole"), get("accName"), Convert.ToInt32(get("accState")));
    }
    public static IntPtr[] Children(IntPtr parent) {
        var list = new System.Collections.Generic.List<IntPtr>();
        EnumChildWindows(parent, (h, l) => { list.Add(h); return true; }, IntPtr.Zero);
        return list.ToArray();
    }
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr l);

    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);

    // png of one window only (PW_RENDERFULLCONTENT), never the whole screen
    public static void SaveWindow(IntPtr h, string path) {
        Physical();
        RECT r; GetWindowRect(h, out r);
        using (var bmp = new System.Drawing.Bitmap(r.Right - r.Left, r.Bottom - r.Top))
        using (var g = System.Drawing.Graphics.FromImage(bmp)) {
            IntPtr hdc = g.GetHdc();
            PrintWindow(h, hdc, 2);
            g.ReleaseHdc(hdc);
            bmp.Save(path, System.Drawing.Imaging.ImageFormat.Png);
        }
    }

    public static RECT FrameBounds(IntPtr h) { RECT r; DwmGetWindowAttribute(h, 9, out r, 16); return r; }

    // first visible top-level window of a process with the given class
    public static IntPtr WindowOf(uint pid, string cls) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            var sb = new System.Text.StringBuilder(256); GetClassName(h, sb, 256);
            if (p == pid && sb.ToString() == cls && (cls != "#32770" || IsWindowVisible(h))) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
