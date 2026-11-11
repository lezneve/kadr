#![windows_subsystem = "windows"]
// throwaway spike: measures tray-idle cost of tauri v2 with zero, one, and a destroyed window.
use tauri::{RunEvent, WebviewUrl, WebviewWindowBuilder, tray::TrayIconBuilder};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let app = tauri::Builder::default()
        .setup(move |app| {
            let icon = app.default_window_icon().cloned().expect("icon");
            TrayIconBuilder::new().icon(icon).tooltip("spike").build(app)?;
            if mode == "probe" { probe::start(); }
            if mode == "open" || mode == "openclose" {
                let w = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
                    .title("kadr").inner_size(520.0, 590.0).resizable(false).build()?;
                if mode == "openclose" {
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        let _ = w.destroy();
                    });
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build");
    app.run(|_, event| {
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() { api.prevent_exit(); }
        }
    });
}

// probe: does a raw win32 window on the main thread get messages while tao pumps the loop?
mod probe {
    use windows::Win32::Foundation::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::w;

    extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if m == WM_APP + 1 {
            let tid = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
            let _ = std::fs::write(std::env::temp_dir().join("kadr-tao-probe.txt"), format!("received wparam={} on thread {tid}", wp.0));
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(h, m, wp, lp) }
    }

    pub fn start() {
        unsafe {
            let inst = GetModuleHandleW(None).unwrap();
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: w!("kadr-probe"), ..Default::default() };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(Default::default(), w!("kadr-probe"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None).unwrap();
            let raw = hwnd.0 as isize;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let _ = PostMessageW(Some(HWND(raw as *mut _)), WM_APP + 1, WPARAM(42), LPARAM(0));
            });
        }
    }
}
