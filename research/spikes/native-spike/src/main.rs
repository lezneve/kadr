#![windows_subsystem = "windows"]
// throwaway spike: bare win32 tray icon + message loop, as an idle-cost baseline.
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, m, w, l) }
}

fn main() {
    unsafe {
        let inst = GetModuleHandleW(None).unwrap();
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: w!("spike"), ..Default::default() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(Default::default(), w!("spike"), w!("spike"), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None).unwrap();
        let mut nid = NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, uFlags: NIF_ICON | NIF_TIP, hIcon: LoadIconW(None, IDI_APPLICATION).unwrap(), ..Default::default() };
        for (i, c) in "spike".encode_utf16().enumerate() { nid.szTip[i] = c; }
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() { DispatchMessageW(&msg); }
    }
}
