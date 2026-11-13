//! the settings window.
//!
//! standard windows controls (buttons, radio buttons, a check box, an edit
//! box) give keyboard navigation and screen-reader roles for free; comctl6
//! custom draw plus direct2d/directwrite give them kadr's look. the window,
//! its gdi and d2d objects exist only while it is open.
//!
//! design: .impeccable/surfaces/settings-window.md. the row being edited is
//! framed by four crop marks, the same corners as the region overlay.
//!
//! it runs in its own short-lived process (kadr --settings-window): direct2d
//! and directwrite keep 5-8 mb of caches in a process once used, and the
//! resident kadr should never carry that (measured, docs/measurements.md).
//! the two processes talk through settings.ini and two window messages.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    DeleteObject, EndPaint, FF_DONTCARE, FW_NORMAL, GetMonitorInfoW, HBRUSH, HDC, HFONT, HGDIOBJ,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    RDW_ALLCHILDREN, RDW_INVALIDATE, RedrawWindow, SetBkColor, SetTextColor, BeginPaint,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    CDDS_PREPAINT, CDIS_HOT, CDIS_SELECTED, CDRF_DODEFAULT, CDRF_SKIPDEFAULT, DRAWITEMSTRUCT,
    NM_CUSTOMDRAW, NMCUSTOMDRAW, NMHDR,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, RegisterHotKey, SetFocus, UnregisterHotKey, VK_BACK, VK_CONTROL,
    VK_DELETE, VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL,
    VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SNAPSHOT,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog,
    IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BN_CLICKED, BN_KILLFOCUS, BN_SETFOCUS, BS_AUTOCHECKBOX,
    BS_AUTORADIOBUTTON, BS_NOTIFY, BS_PUSHBUTTON, CreateWindowExW, DLGC_WANTALLKEYS,
    DefWindowProcW, DestroyIcon, DestroyWindow, EN_CHANGE, EN_KILLFOCUS, EN_SETFOCUS, ES_AUTOHSCROLL,
    GWLP_USERDATA, GetDlgItem, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, HICON,
    HMENU, ICON_BIG, ICON_SMALL, IDC_ARROW, IDCANCEL, KillTimer, LoadCursorW, MB_ICONWARNING, MB_OK,
    MessageBoxW, RegisterClassW, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED,
    WM_DRAWITEM, WM_ERASEBKGND, WM_GETDLGCODE, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS,
    WM_NCDESTROY, WM_NOTIFY, WM_PAINT, WM_SETFONT, WM_SETICON, WM_SETTINGCHANGE, WM_SYSCHAR,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WM_CHAR, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_GROUP, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WM_APP,
};
use windows::core::{PCWSTR, Result, w};

use crate::hotkeys::{self, Action, Hotkey};
use crate::settings::{self, Settings};
use crate::ui::{Canvas, Fonts, Painter, Theme, colorref, frect, make_fonts};
use crate::{storage, trace, wide};

const CLASS: PCWSTR = w!("kadr.settings");

/// sent to the app window. wparam 0: release the global hotkeys (a new
/// combination is being recorded, so the keys must reach this window);
/// wparam 1: reload settings from disk and register them again.
pub const WM_KADR_HOTKEYS: u32 = WM_APP + 10;
/// sent to the app window after the settings file was saved.
pub const WM_KADR_SETTINGS: u32 = WM_APP + 11;

const ID_REGION: i32 = 101;
const ID_WINDOW: i32 = 102;
const ID_SCREEN: i32 = 103;
const ID_SEND_LABEL: i32 = 200;
const ID_CLIPBOARD: i32 = 201;
const ID_FILE: i32 = 202;
const ID_BOTH: i32 = 203;
const ID_EDIT: i32 = 204;
const ID_FOLDER: i32 = 301;
const ID_NAME_LABEL: i32 = 302;
const ID_NAME: i32 = 303;
const ID_SUB_LABEL: i32 = 304;
const ID_SUB_NONE: i32 = 305;
const ID_SUB_MONTH: i32 = 306;
const ID_SUB_DAY: i32 = 307;
/// the subfolder patterns behind the three segments
const SUB_MONTH: &str = "%Y-%m";
const SUB_DAY: &str = "%Y-%m-%d";
const SEND_LABELS: [&str; 4] = ["editor", "clipboard", "file", "both"];
const SUB_LABELS: [&str; 3] = ["none", "by month", "by day"];
const ID_AUTOSTART: i32 = 401;
/// temporary id used to ask windows whether a combination is free.
const PROBE_HOTKEY: i32 = 0xBFF0;
const TIMER_SAVE_NAME: usize = 1;
/// static control drawn by its parent (winuser.h SS_OWNERDRAW).
const SS_OWNERDRAW: u32 = 0x000D;

// layout in logical pixels (96 dpi), scaled at runtime.
const WIDTH: i32 = 520;
const MARGIN: i32 = 24;
const ROW: i32 = 44;
const NOTE: i32 = 24;
const HEAD: i32 = 24;
const GAP: i32 = 16;
const PAD: i32 = 16;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Hotkey(Action),
    Send,
    Folder,
    Sub,
    Name,
    Autostart,
}

struct Row {
    kind: Kind,
    /// the control that owns keyboard focus for this row.
    focus_id: i32,
    rect: RECT,
    group: usize,
    first: bool,
}

struct Group {
    title: &'static str,
    head_top: i32,
    rect: RECT,
}

struct State {
    hwnd: HWND,
    app: HWND,
    path: PathBuf,
    exe: PathBuf,
    settings: Settings,
    canvas: Canvas,
    fonts: Fonts,
    theme: Theme,
    dpi: u32,
    rows: Vec<Row>,
    groups: Vec<Group>,
    recording: Option<Action>,
    /// one line under a row: (row kind, text, is_error). errors stay until
    /// the user resolves them.
    note: Option<(Kind, String, bool)>,
    /// problems windows itself causes (bare printscreen taken by snipping
    /// tool). recomputed on open and after every hotkey change; shown in
    /// red under the row until the user fixes it.
    /// (row, text, is_error): red for a conflict, hint ink for "press it once to check".
    conflict: Option<(Action, String, bool)>,
    name_error: Option<String>,
    edit_font: HFONT,
    edit_brush: HBRUSH,
    icon_small: HICON,
    icon_big: HICON,
}

/// the settings process: one window, a dialog-style message loop, exit on
/// close. `app` is kadr's main window, when kadr is running.
pub fn run(path: PathBuf, app: Option<HWND>) -> i32 {
    // the "next file" preview: %P is the program of each capture
    storage::set_app(Some("program"));
    let (current, _) = Settings::load(&path);
    let opened = open(app.unwrap_or_default(), path, current);
    trace!("settings window shown {:.1} ms after process start", crate::ms_since_process_start());
    if let Err(e) = opened {
        error_box(HWND::default(), &format!("could not open settings\n\n{}", e.message()));
        return 1;
    }
    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    // SAFETY: standard message loop; IsDialogMessage gives tab, arrows,
    // enter and esc their usual dialog meaning.
    unsafe {
        while windows::Win32::UI::WindowsAndMessaging::GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let dialog = windows::Win32::UI::WindowsAndMessaging::FindWindowW(CLASS, None).ok();
            if let Some(d) = dialog
                && windows::Win32::UI::WindowsAndMessaging::IsDialogMessageW(d, &msg).as_bool()
            {
                continue;
            }
            let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
            windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
        }
    }
    trace!("settings process exit, memory {}", crate::memory_summary());
    msg.wParam.0 as i32
}

/// tells kadr something, without hanging if kadr is busy or gone.
fn notify(app: HWND, msg: u32, wparam: usize) {
    if app.is_invalid() {
        return;
    }
    let mut result = 0usize;
    // SAFETY: integer-only message to kadr's window, with a timeout.
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SendMessageTimeoutW(
            app,
            msg,
            WPARAM(wparam),
            LPARAM(0),
            windows::Win32::UI::WindowsAndMessaging::SMTO_ABORTIFHUNG,
            2000,
            Some(&mut result),
        );
    }
}

fn open(app: HWND, path: PathBuf, current: Settings) -> Result<HWND> {
    register_class();
    // SAFETY: standard window creation; the State box is owned by the window
    // through GWLP_USERDATA from WM_NCCREATE and freed in WM_NCDESTROY.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("kadr"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            0,
            0,
            WIDTH,
            600,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        match State::new(hwnd, app, path, current) {
            Ok(state) => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(Box::new(state)) as isize);
            }
            Err(err) => {
                let _ = DestroyWindow(hwnd);
                return Err(err);
            }
        }
        let st = &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State);
        st.create_controls();
        st.apply_theme();
        st.layout(true);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        if let Ok(first) = GetDlgItem(Some(hwnd), ID_REGION) {
            let _ = SetFocus(Some(first));
        }
        Ok(hwnd)
    }
}

fn register_class() {
    // SAFETY: registering twice just fails, which is fine.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class);
    }
}

impl State {
    fn new(hwnd: HWND, app: HWND, path: PathBuf, settings: Settings) -> Result<State> {
        // SAFETY: plain factory and resource creation.
        unsafe {
            let canvas = Canvas::new()?;
            let dpi = GetDpiForWindow(hwnd).max(96);
            let fonts = make_fonts(&canvas.dwrite, dpi)?;
            let exe = std::env::current_exe().unwrap_or_default();
            Ok(State {
                hwnd,
                app,
                path,
                exe,
                settings,
                canvas,
                fonts,
                theme: Theme::current(),
                dpi,
                rows: Vec::new(),
                groups: Vec::new(),
                recording: None,
                note: None,
                conflict: None,
                name_error: None,
                edit_font: HFONT::default(),
                edit_brush: HBRUSH::default(),
                icon_small: HICON::default(),
                icon_big: HICON::default(),
            })
        }
    }

    fn px(&self, v: i32) -> i32 {
        (v * self.dpi as i32 + 48) / 96
    }

    fn create_controls(&mut self) {
        let hwnd = self.hwnd;
        let mk = |id: i32, class: PCWSTR, text: &str, style: u32| {
            let text = wide(text);
            // SAFETY: child window of our own window with a unique id.
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    class,
                    PCWSTR(text.as_ptr()),
                    WINDOW_STYLE(style) | WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    10,
                    10,
                    Some(hwnd),
                    Some(HMENU(id as isize as *mut c_void)),
                    None,
                    None,
                )
                .unwrap_or_default()
            }
        };
        let tab = WS_TABSTOP.0;
        let push = BS_PUSHBUTTON as u32 | BS_NOTIFY as u32 | tab;
        for (id, action) in [(ID_REGION, Action::Region), (ID_WINDOW, Action::Window), (ID_SCREEN, Action::Screen)] {
            let b = mk(id, w!("BUTTON"), &self.hotkey_name(action), push);
            // SAFETY: subclass our own button; removed with the window.
            unsafe {
                let _ = SetWindowSubclass(b, Some(hotkey_proc), id as usize, 0);
            }
        }
        mk(ID_SEND_LABEL, w!("STATIC"), "send to", SS_OWNERDRAW);
        mk(ID_EDIT, w!("BUTTON"), "editor", BS_AUTORADIOBUTTON as u32 | BS_NOTIFY as u32 | tab | WS_GROUP.0);
        mk(ID_CLIPBOARD, w!("BUTTON"), "clipboard", BS_AUTORADIOBUTTON as u32 | BS_NOTIFY as u32);
        mk(ID_FILE, w!("BUTTON"), "file", BS_AUTORADIOBUTTON as u32 | BS_NOTIFY as u32);
        mk(ID_BOTH, w!("BUTTON"), "both", BS_AUTORADIOBUTTON as u32 | BS_NOTIFY as u32);
        mk(ID_FOLDER, w!("BUTTON"), &self.folder_name(), push | WS_GROUP.0);
        mk(ID_SUB_LABEL, w!("STATIC"), "subfolder", SS_OWNERDRAW);
        mk(ID_SUB_NONE, w!("BUTTON"), "subfolder none", BS_AUTORADIOBUTTON as u32 | BS_NOTIFY as u32 | tab | WS_GROUP.0);
        mk(ID_SUB_MONTH, w!("BUTTON"), "subfolder by month", BS_AUTORADIOBUTTON as u32 | BS_NOTIFY as u32);
        mk(ID_SUB_DAY, w!("BUTTON"), "subfolder by day", BS_AUTORADIOBUTTON as u32 | BS_NOTIFY as u32);
        mk(ID_NAME_LABEL, w!("STATIC"), "file name", SS_OWNERDRAW);
        let edit = mk(ID_NAME, w!("EDIT"), &self.settings.file_name, ES_AUTOHSCROLL as u32 | tab);
        self.make_edit_font();
        self.update_conflict();
        // SAFETY: plain message to our own edit control.
        unsafe {
            SendMessageW(edit, 0x00C5 /* EM_LIMITTEXT */, Some(WPARAM(120)), None);
        }
        mk(ID_AUTOSTART, w!("BUTTON"), "start with windows", BS_AUTOCHECKBOX as u32 | BS_NOTIFY as u32 | tab | WS_GROUP.0);
        self.sync_controls();
    }

    /// pushes current values into the standard controls' state and names.
    fn sync_controls(&self) {
        // SAFETY: messages to our own child controls.
        unsafe {
            let check = |id: i32, on: bool| {
                if let Ok(c) = GetDlgItem(Some(self.hwnd), id) {
                    SendMessageW(c, BM_SETCHECK, Some(WPARAM(on as usize)), None);
                }
            };
            let s = &self.settings;
            check(ID_EDIT, s.edit);
            check(ID_CLIPBOARD, !s.edit && s.copy && !s.save);
            check(ID_FILE, !s.edit && s.save && !s.copy);
            check(ID_BOTH, !s.edit && s.copy && s.save);
            check(ID_SUB_NONE, s.subfolder.is_empty());
            check(ID_SUB_MONTH, s.subfolder == SUB_MONTH);
            check(ID_SUB_DAY, s.subfolder == SUB_DAY);
            check(ID_AUTOSTART, crate::autostart::is_enabled(&self.exe));
            for (id, action) in [(ID_REGION, Action::Region), (ID_WINDOW, Action::Window), (ID_SCREEN, Action::Screen)] {
                if let Ok(c) = GetDlgItem(Some(self.hwnd), id) {
                    let t = wide(&self.hotkey_name(action));
                    let _ = SetWindowTextW(c, PCWSTR(t.as_ptr()));
                }
            }
            if let Ok(c) = GetDlgItem(Some(self.hwnd), ID_FOLDER) {
                let t = wide(&self.folder_name());
                let _ = SetWindowTextW(c, PCWSTR(t.as_ptr()));
            }
        }
    }

    /// accessible name, read by screen readers.
    fn hotkey_name(&self, action: Action) -> String {
        let key = self.settings.hotkey(action).map_or("off".to_string(), |k| k.to_string());
        format!("capture {}, {key}", action.label())
    }

    fn folder_name(&self) -> String {
        format!("folder, {}", self.folder_path().display())
    }

    fn folder_path(&self) -> PathBuf {
        match &self.settings.save_folder {
            Some(p) => p.clone(),
            None => storage::default_capture_dir().unwrap_or_default(),
        }
    }

    fn apply_theme(&mut self) {
        self.theme = Theme::current();
        let dark: i32 = self.theme.dark as i32;
        // SAFETY: documented attribute (20 = DWMWA_USE_IMMERSIVE_DARK_MODE)
        // with a BOOL; old builds ignore it. objects replaced here are ours.
        unsafe {
            let _ = DwmSetWindowAttribute(self.hwnd, windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(20), (&dark as *const i32).cast(), 4);
            // caption (35) and its text (36) in the window's own ramp, so the
            // graphite world does not stop at the title bar.
            let caption = colorref(self.theme.ground).0;
            let text = colorref(self.theme.ink).0;
            let _ = DwmSetWindowAttribute(self.hwnd, windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(35), (&caption as *const u32).cast(), 4);
            let _ = DwmSetWindowAttribute(self.hwnd, windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(36), (&text as *const u32).cast(), 4);
            if !self.edit_brush.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.edit_brush.0));
            }
            self.edit_brush = CreateSolidBrush(colorref(self.theme.cap));
            self.load_icons();
        }
        self.redraw();
    }

    /// the app icon from the resources at the sizes windows wants for this dpi.
    fn load_icons(&mut self) {
        use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
        use windows::Win32::UI::WindowsAndMessaging::{SM_CXICON, SM_CXSMICON};
        // SAFETY: our own icons; the old ones are destroyed after replacement.
        unsafe {
            let small = crate::ui::app_icon(GetSystemMetricsForDpi(SM_CXSMICON, self.dpi));
            let big = crate::ui::app_icon(GetSystemMetricsForDpi(SM_CXICON, self.dpi));
            SendMessageW(self.hwnd, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(LPARAM(small.0 as isize)));
            SendMessageW(self.hwnd, WM_SETICON, Some(WPARAM(ICON_BIG as usize)), Some(LPARAM(big.0 as isize)));
            for old in [std::mem::replace(&mut self.icon_small, small), std::mem::replace(&mut self.icon_big, big)] {
                if !old.is_invalid() {
                    let _ = DestroyIcon(old);
                }
            }
        }
    }

    fn redraw(&self) {
        // SAFETY: our own window.
        unsafe {
            let _ = RedrawWindow(Some(self.hwnd), None, None, RDW_INVALIDATE | RDW_ALLCHILDREN);
        }
    }

    /// computes every rect from the dpi and the current notes, moves the
    /// controls, and sizes the window to fit. `center` places it on the
    /// monitor under the cursor (first open).
    fn layout(&mut self, center: bool) {
        let p = |v: i32| self.px(v);
        let width = p(WIDTH);
        let (left, right) = (p(MARGIN), width - p(MARGIN));
        let mut y = p(16) + p(32) + p(8);
        let mut rows = Vec::new();
        let mut groups = Vec::new();
        let plan: [(&'static str, &[(Kind, i32)]); 4] = [
            ("hotkeys", &[(Kind::Hotkey(Action::Region), ID_REGION), (Kind::Hotkey(Action::Window), ID_WINDOW), (Kind::Hotkey(Action::Screen), ID_SCREEN)]),
            ("after capture", &[(Kind::Send, ID_EDIT)]),
            ("saving", &[(Kind::Folder, ID_FOLDER), (Kind::Sub, ID_SUB_NONE), (Kind::Name, ID_NAME)]),
            ("general", &[(Kind::Autostart, ID_AUTOSTART)]),
        ];
        for (g, (title, items)) in plan.iter().enumerate() {
            let head_top = y;
            y += p(HEAD);
            let top = y;
            for (i, (kind, id)) in items.iter().enumerate() {
                let mut h = p(ROW);
                let conflict_here = matches!((kind, &self.conflict), (Kind::Hotkey(a), Some((c, _, _))) if a == c);
                if matches!(self.note, Some((k, _, _)) if k == *kind) || conflict_here {
                    h += p(NOTE);
                }
                if *kind == Kind::Name {
                    h += p(NOTE); // next file name preview
                }
                rows.push(Row {
                    kind: *kind,
                    focus_id: *id,
                    rect: RECT { left, top: y, right, bottom: y + h },
                    group: g,
                    first: i == 0,
                });
                y += h;
            }
            groups.push(Group { title, head_top, rect: RECT { left, top, right, bottom: y } });
            y += p(GAP);
        }
        let height = y - p(GAP) + p(MARGIN);
        self.rows = rows;
        self.groups = groups;
        self.place_controls();

        // SAFETY: sizing our own window for the client area we need.
        unsafe {
            let mut r = RECT { left: 0, top: 0, right: width, bottom: height };
            let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
            let _ = AdjustWindowRectExForDpi(&mut r, style, false, WINDOW_EX_STYLE(0), self.dpi);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if center {
                let mut cursor = POINT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut cursor);
                let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                let _ = GetMonitorInfoW(MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST), &mut info);
                let wa = info.rcWork;
                let x = wa.left + ((wa.right - wa.left) - w) / 2;
                let yy = wa.top + ((wa.bottom - wa.top) - h) / 3;
                let _ = SetWindowPos(self.hwnd, None, x, yy, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            } else {
                // keep the grown window inside the work area of its monitor.
                let mut now = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(self.hwnd, &mut now);
                let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                let monitor = windows::Win32::Graphics::Gdi::MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
                let _ = GetMonitorInfoW(monitor, &mut info);
                let top = now.top.min(info.rcWork.bottom - h).max(info.rcWork.top);
                let _ = SetWindowPos(self.hwnd, None, now.left, top, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        self.redraw();
    }

    fn place_controls(&mut self) {
        let p = |v: i32| self.px(v);
        let put = |id: i32, r: RECT| {
            // SAFETY: moving our own child controls.
            unsafe {
                if let Ok(c) = GetDlgItem(Some(self.hwnd), id) {
                    let _ = SetWindowPos(c, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
        };
        for row in &self.rows {
            let r = row.rect;
            let line = RECT { bottom: r.top + p(ROW), ..r };
            match row.kind {
                Kind::Hotkey(_) | Kind::Folder | Kind::Autostart => put(row.focus_id, r),
                Kind::Send => {
                    let segs = self.segments(line, &SEND_LABELS);
                    put(ID_SEND_LABEL, RECT { right: segs[0].left - p(8), ..line });
                    for (id, s) in [ID_EDIT, ID_CLIPBOARD, ID_FILE, ID_BOTH].into_iter().zip(segs) {
                        put(id, s);
                    }
                }
                Kind::Sub => {
                    let segs = self.segments(line, &SUB_LABELS);
                    put(ID_SUB_LABEL, RECT { right: segs[0].left - p(8), ..line });
                    for (id, s) in [ID_SUB_NONE, ID_SUB_MONTH, ID_SUB_DAY].into_iter().zip(segs) {
                        put(id, s);
                    }
                }
                Kind::Name => {
                    let field = self.name_field(line);
                    put(ID_NAME_LABEL, RECT { right: field.left - p(8), ..line });
                    let font_h = p(16);
                    let edit = RECT {
                        left: field.left + p(8),
                        right: field.right - p(8),
                        top: field.top + (field.bottom - field.top - font_h) / 2,
                        bottom: field.top + (field.bottom - field.top - font_h) / 2 + font_h,
                    };
                    put(ID_NAME, edit);
                }
            }
        }
    }

    /// the segment rects of a segmented row, right-aligned in it.
    fn segments(&self, line: RECT, labels: &[&str]) -> Vec<RECT> {
        let p = |v: i32| self.px(v);
        let widths: Vec<i32> = labels.iter().map(|l| self.measure(l, &self.fonts.small).ceil() as i32 + p(24)).collect();
        let h = p(28);
        let top = line.top + (line.bottom - line.top - h) / 2;
        let mut x = line.right - p(PAD) - p(2) - widths.iter().sum::<i32>();
        let mut out = Vec::new();
        for w in &widths {
            out.push(RECT { left: x, top, right: x + w, bottom: top + h });
            x += w;
        }
        out
    }

    fn name_field(&self, line: RECT) -> RECT {
        let p = |v: i32| self.px(v);
        let h = p(28);
        let top = line.top + (line.bottom - line.top - h) / 2;
        RECT { left: line.right - p(PAD) - p(240), top, right: line.right - p(PAD), bottom: top + h }
    }

    fn measure(&self, text: &str, format: &IDWriteTextFormat) -> f32 {
        self.canvas.measure(text, format)
    }

    fn focused_kind(&self) -> Option<Kind> {
        // SAFETY: plain focus query.
        let focus = unsafe { GetFocus() };
        if focus.is_invalid() {
            return None;
        }
        let id = |i: i32| unsafe { GetDlgItem(Some(self.hwnd), i).ok() } == Some(focus);
        self.rows.iter().find(|r| match r.kind {
            Kind::Send => id(ID_EDIT) || id(ID_CLIPBOARD) || id(ID_FILE) || id(ID_BOTH),
            Kind::Sub => id(ID_SUB_NONE) || id(ID_SUB_MONTH) || id(ID_SUB_DAY),
            _ => id(r.focus_id),
        }).map(|r| r.kind)
    }


    // ---- painting ------------------------------------------------------

    /// draws into `hdc`, whose top-left sits at `origin` in window client
    /// coordinates; everything is drawn in client coordinates.
    fn paint(&self, hdc: HDC, size: (i32, i32), origin: (i32, i32), f: impl FnOnce(&Painter)) {
        self.canvas.paint(hdc, size, origin, f);
    }

    fn paint_window(&self, p: &Painter, clip: RECT) {
        let t = self.theme;
        p.fill(clip, t.ground);
        let px = |v: i32| self.px(v) as f32;
        p.text("settings", &self.fonts.title, frect(RECT { left: self.px(MARGIN) + self.px(8), top: self.px(16), right: self.px(WIDTH), bottom: self.px(48) }), t.ink);
        for g in &self.groups {
            let r = RECT { left: g.rect.left + self.px(8), top: g.head_top, right: g.rect.right, bottom: g.head_top + self.px(HEAD) - self.px(4) };
            p.text(g.title, &self.fonts.head, frect(r), t.ink2);
        }
        let _ = px;
        for i in 0..self.rows.len() {
            self.paint_row(p, i, false, false);
        }
    }

    /// one row, complete: ground, its part of the group surface, separator,
    /// content, note, and crop marks when it has focus.
    fn paint_row(&self, p: &Painter, index: usize, hot: bool, pressed: bool) {
        let t = self.theme;
        let row = &self.rows[index];
        let group = &self.groups[row.group];
        let radius = self.px(12) as f32;
        p.fill(row.rect, t.ground);
        p.fill_round(group.rect, radius, t.raised);
        // keyboard focus shows as the hover tint (no crop marks on rows)
        let focused = self.focused_kind() == Some(row.kind);
        if hot || pressed || focused {
            p.clip(row.rect, || p.fill_round(group.rect, radius, if pressed { t.cap } else { t.hover }));
        }
        p.stroke_round(group.rect, radius, t.hair);
        if !row.first {
            let y = row.rect.top as f32 + 0.5;
            p.line((row.rect.left as f32 + self.px(PAD) as f32, y), (row.rect.right as f32 - self.px(PAD) as f32, y), t.hair);
        }
        let line = RECT { bottom: row.rect.top + self.px(ROW), ..row.rect };
        let label = RECT { left: line.left + self.px(PAD), ..line };
        match row.kind {
            Kind::Hotkey(action) => {
                p.text(&format!("capture {}", action.label()), &self.fonts.body, frect(label), t.ink);
                let live = self.recording == Some(action);
                let left = match self.settings.hotkey(action) {
                    Some(key) => self.paint_keycaps(p, line, &key.to_string(), live),
                    None => {
                        let r = RECT { right: line.right - self.px(PAD), ..line };
                        p.text_right("off", &self.fonts.small, frect(r), t.ink3);
                        r.right - self.measure("off", &self.fonts.small) as i32
                    }
                };
                // the focused row says what enter does, in the same line.
                if !live && self.focused_kind() == Some(row.kind) {
                    p.text_right("enter to change", &self.fonts.small, frect(RECT { right: left - self.px(12), ..line }), t.ink3);
                }
            }
            Kind::Send | Kind::Sub => {
                let segs = if row.kind == Kind::Send { self.segments(line, &SEND_LABELS) } else { self.segments(line, &SUB_LABELS) };
                let last = segs[segs.len() - 1];
                let outer = RECT { left: segs[0].left - self.px(2), top: segs[0].top - self.px(2), right: last.right + self.px(2), bottom: segs[0].bottom + self.px(2) };
                p.fill_round(outer, self.px(8) as f32, t.cap);
            }
            Kind::Folder => {
                p.text("folder", &self.fonts.body, frect(label), t.ink);
                let value = self.short_folder(self.px(260) as f32);
                // a drawn chevron, not a text glyph standing in for an icon.
                let cx = (line.right - self.px(PAD)) as f32 - self.px(3) as f32;
                let cy = (line.top + line.bottom) as f32 / 2.0;
                let (dx, dy) = (self.px(3) as f32, self.px(4) as f32);
                let ink2 = [t.ink2[0], t.ink2[1], t.ink2[2], 1.0];
                let stroke = self.dpi as f32 / 96.0 * 1.25;
                p.line_w((cx - dx, cy - dy), (cx, cy), ink2, stroke);
                p.line_w((cx, cy), (cx - dx, cy + dy), ink2, stroke);
                p.text_right(&value, &self.fonts.small, frect(RECT { right: line.right - self.px(PAD) - self.px(14), ..line }), t.ink2);
            }
            Kind::Name => {
                let field = self.name_field(line);
                p.fill_round(field, self.px(6) as f32, t.cap);
                p.stroke_round(field, self.px(6) as f32, t.hair);
                let preview = RECT { left: row.rect.left + self.px(PAD), top: line.bottom - self.px(6), right: row.rect.right - self.px(PAD), bottom: line.bottom - self.px(6) + self.px(NOTE) };
                match &self.name_error {
                    Some(err) => p.text(err, &self.fonts.small, frect(preview), t.alert),
                    None => {
                        let sub = if self.settings.subfolder.is_empty() { String::new() } else { format!("{}\\", storage::file_stem(&self.settings.subfolder)) };
                        let next = format!("next file  {sub}{}.png", storage::file_stem(&self.settings.file_name));
                        p.text(&next, &self.fonts.mono, frect(preview), t.ink3);
                    }
                }
            }
            Kind::Autostart => {
                p.text("start with windows", &self.fonts.body, frect(label), t.ink);
                // SAFETY: plain message to our own check box.
                let on = unsafe {
                    GetDlgItem(Some(self.hwnd), ID_AUTOSTART).map(|c| SendMessageW(c, BM_GETCHECK, None, None).0 == 1).unwrap_or(false)
                };
                self.paint_switch(p, line, on);
            }
        }
        let note_rect = RECT { left: row.rect.left + self.px(PAD), top: line.bottom - self.px(8), right: row.rect.right - self.px(PAD), bottom: line.bottom - self.px(8) + self.px(NOTE) };
        match (&self.note, &self.conflict, row.kind) {
            (Some((kind, text, error)), _, k) if *kind == k => p.text(text, &self.fonts.small, frect(note_rect), if *error { t.alert } else { t.ink3 }),
            (_, Some((action, text, error)), Kind::Hotkey(a)) if *action == a => p.text(text, &self.fonts.small, frect(note_rect), if *error { t.alert } else { t.ink3 }),
            _ => {}
        }
    }

    fn short_folder(&self, max: f32) -> String {
        let path = self.folder_path();
        let full = path.display().to_string();
        if self.measure(&full, &self.fonts.small) <= max {
            return full;
        }
        let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let tail = parts.iter().rev().take(2).rev().cloned().collect::<Vec<_>>().join("\\");
        format!("…\\{tail}")
    }

    /// returns the left edge of the leftmost keycap.
    fn paint_keycaps(&self, p: &Painter, line: RECT, text: &str, live: bool) -> i32 {
        let t = self.theme;
        let h = self.px(24);
        let top = line.top + (line.bottom - line.top - h) / 2;
        let mut right = line.right - self.px(PAD);
        for part in text.split('+').rev() {
            let w = self.measure(part, &self.fonts.small).ceil() as i32 + self.px(16);
            let r = RECT { left: right - w, top, right, bottom: top + h };
            p.fill_round(r, self.px(6) as f32, t.cap);
            let edge = if live { [t.ink2[0], t.ink2[1], t.ink2[2], 1.0] } else { t.hair };
            p.stroke_round(r, self.px(6) as f32, edge);
            p.text_center(part, &self.fonts.small, frect(r), t.ink);
            right = r.left - self.px(4);
        }
        right + self.px(4)
    }

    fn paint_switch(&self, p: &Painter, line: RECT, on: bool) {
        let t = self.theme;
        let (w, h) = (self.px(36), self.px(20));
        let top = line.top + (line.bottom - line.top - h) / 2;
        let r = RECT { left: line.right - self.px(PAD) - w, top, right: line.right - self.px(PAD), bottom: top + h };
        let knob = self.px(12);
        if on {
            p.fill_round(r, h as f32 / 2.0, t.ink);
            let k = RECT { left: r.right - self.px(4) - knob, top: r.top + (h - knob) / 2, right: r.right - self.px(4), bottom: r.top + (h - knob) / 2 + knob };
            p.fill_round(k, knob as f32 / 2.0, t.raised);
        } else {
            p.stroke_round(r, h as f32 / 2.0, [t.ink3[0], t.ink3[1], t.ink3[2], 1.0]);
            let k = RECT { left: r.left + self.px(4), top: r.top + (h - knob) / 2, right: r.left + self.px(4) + knob, bottom: r.top + (h - knob) / 2 + knob };
            p.fill_round(k, knob as f32 / 2.0, t.ink2);
        }
    }

    /// the signature: four crop marks on the corners of the focused row.
    fn paint_segment(&self, p: &Painter, id: i32, r: RECT, checked: bool, hot: bool) {
        let t = self.theme;
        p.fill(r, t.cap);
        if checked {
            p.fill_round(r, self.px(6) as f32, t.seg_on);
        }
        let label = match id {
            ID_SUB_NONE => "none",
            ID_SUB_MONTH => "by month",
            ID_SUB_DAY => "by day",
            ID_EDIT => "editor",
            ID_CLIPBOARD => "clipboard",
            ID_FILE => "file",
            _ => "both",
        };
        p.text_center(label, &self.fonts.small, frect(r), if checked || hot { t.ink } else { t.ink2 });
    }

    // ---- behaviour -------------------------------------------------------

    fn save(&mut self) {
        match settings::save(&self.path, &self.settings) {
            Ok(()) => {
                notify(self.app, WM_KADR_SETTINGS, 0);
            }
            Err(e) => error_box(self.hwnd, &format!("could not save settings\n\n{e}")),
        }
        self.sync_controls();
        self.redraw();
    }

    fn start_recording(&mut self, action: Action) {
        if self.recording == Some(action) {
            return;
        }
        if self.recording.is_none() {
            notify(self.app, WM_KADR_HOTKEYS, 0);
        }
        self.recording = Some(action);
        self.note = Some((Kind::Hotkey(action), "press a new combination · esc keeps the old one · backspace turns it off".into(), false));
        trace!("settings: recording {}", action.label());
        self.layout(false);
    }

    fn stop_recording(&mut self, keep_note: bool) {
        if self.recording.take().is_some() {
            notify(self.app, WM_KADR_HOTKEYS, 1);
        }
        if !keep_note {
            self.note = None;
        }
        self.layout(false);
    }

    /// a key arrived while recording. returns true when it was used.
    fn key(&mut self, vk: u16, up: bool) -> bool {
        let Some(action) = self.recording else { return false };
        let k = vk;
        let modifier = [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN, VK_LSHIFT, VK_RSHIFT, VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU].iter().any(|m| m.0 == k);
        if modifier {
            return true;
        }
        // printscreen only ever arrives as key-up.
        if up && k != VK_SNAPSHOT.0 {
            return true;
        }
        let down = |v: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe { GetKeyState(v.0 as i32) } < 0;
        let (ctrl, alt, shift, win) = (down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT), down(VK_LWIN) || down(VK_RWIN));
        if k == VK_ESCAPE.0 && !(ctrl || alt || shift || win) {
            self.stop_recording(false);
            return true;
        }
        if (k == VK_BACK.0 || k == VK_DELETE.0) && !(ctrl || alt || shift || win) {
            self.set_hotkey(action, None);
            self.stop_recording(false);
            return true;
        }
        let candidate = Hotkey { ctrl, alt, shift, win, vk: k as u32 };
        let key = match Hotkey::parse(&candidate.to_string()) {
            Ok(key) if key == candidate => key,
            Ok(_) | Err(_) => {
                let msg = if Hotkey::parse(&candidate.to_string()).is_err() && (k as u8).is_ascii_alphanumeric() {
                    "letters and digits need ctrl, alt or win, otherwise they block typing".to_string()
                } else {
                    "that key cannot be a hotkey · try a letter, digit, f-key or printscreen".to_string()
                };
                self.note = Some((Kind::Hotkey(action), msg, true));
                self.redraw();
                return true;
            }
        };
        if let Some(other) = Action::ALL.into_iter().find(|a| *a != action && self.settings.hotkey(*a) == Some(key)) {
            self.note = Some((Kind::Hotkey(action), format!("{key} is already used for {} capture", other.label()), true));
            self.redraw();
            return true;
        }
        // with kadr's own hotkeys released, a failure means another app.
        // SAFETY: probe registration on our own window, released at once.
        let free = unsafe {
            let mods = windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS(
                (key.ctrl as u32) * 2 | (key.alt as u32) | (key.shift as u32) * 4 | (key.win as u32) * 8 | 0x4000,
            );
            let ok = RegisterHotKey(Some(self.hwnd), PROBE_HOTKEY, mods, key.vk).is_ok();
            if ok {
                let _ = UnregisterHotKey(Some(self.hwnd), PROBE_HOTKEY);
            }
            ok
        };
        if !free {
            self.note = Some((Kind::Hotkey(action), format!("{key} is used by another app · try another"), true));
            self.redraw();
            return true;
        }
        // printscreen chosen while windows gives it to snipping tool: an
        // explicit choice, so ask (again) whether to free the key.
        if key.is_bare_printscreen() && crate::snipping::owns_printscreen() {
            crate::snipping::ask(Some(self.hwnd));
        }
        self.set_hotkey(action, Some(key));
        self.stop_recording(false);
        true
    }

    fn set_hotkey(&mut self, action: Action, key: Option<Hotkey>) {
        match action {
            Action::Region => self.settings.hotkey_region = key,
            Action::Window => self.settings.hotkey_window = key,
            Action::Screen => self.settings.hotkey_screen = key,
        }
        self.update_conflict();
        self.save();
    }

    /// bare printscreen while windows gives it to snipping tool: kadr would
    /// never receive it. held in red under that row until changed.
    fn update_conflict(&mut self) {
        self.conflict = Action::ALL.into_iter().find_map(|a| {
            let key = self.settings.hotkey(a);
            (!hotkeys::snipping_conflicts(&[(a, key)]).is_empty())
                .then(|| (a, "taken by snipping tool · set it again to free it, or pick another key".to_string(), true))
        });
        // freed by kadr, but no printscreen has reached kadr yet: the registry
        // value alone does not prove windows applied it.
        if self.conflict.is_none() && crate::snipping::kadr_changed() && !crate::snipping::confirmed() {
            self.conflict = Action::ALL
                .into_iter()
                .find(|a| self.settings.hotkey(*a).is_some_and(|k| k.is_bare_printscreen()))
                .map(|a| (a, "not confirmed yet · press printscreen once; if snipping tool opens, sign out and back in".to_string(), false));
        }
    }

    fn command(&mut self, id: i32, code: u32) {
        match (id, code) {
            (ID_REGION | ID_WINDOW | ID_SCREEN, c) if c == BN_CLICKED => {
                let action = [Action::Region, Action::Window, Action::Screen][(id - ID_REGION) as usize];
                self.start_recording(action);
            }
            (ID_EDIT, c) if c == BN_CLICKED => {
                self.settings.edit = true;
                self.save();
            }
            (ID_CLIPBOARD | ID_FILE | ID_BOTH, c) if c == BN_CLICKED => {
                self.settings.edit = false;
                (self.settings.copy, self.settings.save) = match id {
                    ID_CLIPBOARD => (true, false),
                    ID_FILE => (false, true),
                    _ => (true, true),
                };
                self.save();
            }
            (ID_SUB_NONE | ID_SUB_MONTH | ID_SUB_DAY, c) if c == BN_CLICKED => {
                self.settings.subfolder = match id {
                    ID_SUB_MONTH => SUB_MONTH.to_string(),
                    ID_SUB_DAY => SUB_DAY.to_string(),
                    _ => String::new(),
                };
                self.save();
                self.redraw(); // the next-file preview shows the subfolder
            }
            (ID_FOLDER, c) if c == BN_CLICKED => {
                if let Some(dir) = pick_folder(self.hwnd, &self.folder_path()) {
                    let default = storage::default_capture_dir().ok();
                    self.settings.save_folder = if Some(&dir) == default.as_ref() { None } else { Some(dir) };
                    self.save();
                }
            }
            (ID_AUTOSTART, c) if c == BN_CLICKED => {
                // SAFETY: our own check box.
                let on = unsafe { GetDlgItem(Some(self.hwnd), ID_AUTOSTART).map(|b| SendMessageW(b, BM_GETCHECK, None, None).0 == 1).unwrap_or(false) };
                match crate::autostart::set(&self.exe, on) {
                    Ok(()) => {
                        if matches!(self.note, Some((Kind::Autostart, _, _))) {
                            self.note = None;
                        }
                    }
                    Err(e) => self.note = Some((Kind::Autostart, format!("could not change it: {e}"), true)),
                }
                self.sync_controls();
                self.layout(false);
            }
            (ID_NAME, c) if c == EN_CHANGE => {
                let text = window_text(self.hwnd, ID_NAME);
                match settings::parse_file_name(&text) {
                    // the edit reports a change when it is created and filled;
                    // nothing to save then (a save would rewrite the file).
                    Ok(name) if name == self.settings.file_name && self.name_error.is_none() => {}
                    Ok(name) => {
                        self.name_error = None;
                        self.settings.file_name = name;
                        // SAFETY: one-shot timer on our window; saves after typing pauses.
                        unsafe {
                            SetTimer(Some(self.hwnd), TIMER_SAVE_NAME, 400, None);
                        }
                    }
                    // the ini message names the key; the window does not need it.
                    Err(e) => self.name_error = Some(e.replace("file_name ", "")),
                }
                self.redraw();
            }
            (ID_NAME, c) if c == EN_SETFOCUS => {
                // the dialog manager selects all text on tab; the selection
                // would paint in the system accent. put the caret at the end.
                // SAFETY: posted to our own edit, after the dialog manager's own select.
                unsafe {
                    if let Ok(edit) = GetDlgItem(Some(self.hwnd), ID_NAME) {
                        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(Some(edit), 0x00B1 /* EM_SETSEL */, WPARAM(usize::MAX), LPARAM(-1));
                    }
                }
                self.redraw();
            }
            (_, c) if c == BN_SETFOCUS || c == BN_KILLFOCUS || c == EN_SETFOCUS || c == EN_KILLFOCUS => self.redraw(),
            _ => {}
        }
    }

    fn set_dpi(&mut self, dpi: u32, suggested: RECT) {
        self.dpi = dpi.max(96);
        if let Ok(f) = make_fonts(&self.canvas.dwrite, self.dpi) {
            self.fonts = f;
        }
        self.make_edit_font();
        self.load_icons();
        // SAFETY: move to the rect windows suggests, then fit the height.
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, suggested.left, suggested.top, suggested.right - suggested.left, suggested.bottom - suggested.top, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        self.layout(false);
    }

    fn make_edit_font(&mut self) {
        // SAFETY: the old font is deselected by WM_SETFONT before deletion.
        unsafe {
            let font = CreateFontW(-self.px(12), 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, FF_DONTCARE.0 as u32, PCWSTR(self.fonts.mono_family.as_ptr()));
            if let Ok(edit) = GetDlgItem(Some(self.hwnd), ID_NAME) {
                SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            }
            if !self.edit_font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.edit_font.0));
            }
            self.edit_font = font;
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // SAFETY: our own gdi objects and icon; d2d/dwrite objects release
        // themselves when their com references drop.
        unsafe {
            if !self.edit_font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.edit_font.0));
            }
            if !self.edit_brush.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.edit_brush.0));
            }
            for icon in [self.icon_small, self.icon_big] {
                if !icon.is_invalid() {
                    let _ = DestroyIcon(icon);
                }
            }
        }
    }
}

fn window_text(parent: HWND, id: i32) -> String {
    // SAFETY: buffer sized from the reported length.
    unsafe {
        let Ok(c) = GetDlgItem(Some(parent), id) else { return String::new() };
        let len = GetWindowTextLengthW(c) as usize;
        let mut buf = vec![0u16; len + 1];
        let n = GetWindowTextW(c, &mut buf) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

fn pick_folder(owner: HWND, start: &Path) -> Option<PathBuf> {
    // SAFETY: com objects used and released in this scope.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        let start_w = wide(&start.display().to_string());
        if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(start_w.as_ptr()), None) {
            let _ = dialog.SetFolder(&item);
        }
        dialog.Show(Some(owner)).ok()?; // cancel returns an error
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const c_void));
        path.map(PathBuf::from)
    }
}

fn error_box(owner: HWND, text: &str) {
    let t = wide(text);
    // SAFETY: valid strings.
    unsafe {
        MessageBoxW(Some(owner), PCWSTR(t.as_ptr()), w!("kadr"), MB_OK | MB_ICONWARNING);
    }
}

fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    // SAFETY: GWLP_USERDATA is our State box from creation until NCDESTROY.
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State).as_mut() }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(st) = state(hwnd) else {
        // SAFETY: default handling before the state exists.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    // SAFETY: standard handling of our own window's messages.
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let rc = ps.rcPaint;
                let mut client = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
                st.paint(hdc, (client.right, client.bottom), (0, 0), |p| st.paint_window(p, client));
                let _ = rc;
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_NOTIFY => {
                let hdr = &*(lparam.0 as *const NMHDR);
                if hdr.code == NM_CUSTOMDRAW {
                    let cd = &*(lparam.0 as *const NMCUSTOMDRAW);
                    if cd.dwDrawStage != CDDS_PREPAINT {
                        return LRESULT(CDRF_DODEFAULT as isize);
                    }
                    let id = hdr.idFrom as i32;
                    let hot = (cd.uItemState.0 & CDIS_HOT.0) != 0;
                    let pressed = (cd.uItemState.0 & CDIS_SELECTED.0) != 0;
                    let mut wr = RECT::default();
                    let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hdr.hwndFrom, &mut wr);
                    let mut origin = POINT { x: wr.left, y: wr.top };
                    let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut origin);
                    let size = (cd.rc.right - cd.rc.left, cd.rc.bottom - cd.rc.top);
                    match id {
                        ID_EDIT | ID_CLIPBOARD | ID_FILE | ID_BOTH | ID_SUB_NONE | ID_SUB_MONTH | ID_SUB_DAY => {
                            let checked = SendMessageW(hdr.hwndFrom, BM_GETCHECK, None, None).0 == 1;
                            let r = RECT { left: origin.x, top: origin.y, right: origin.x + size.0, bottom: origin.y + size.1 };
                            st.paint(cd.hdc, size, (origin.x, origin.y), |p| {
                                st.paint_segment(p, id, r, checked, hot);
                                if GetFocus() == hdr.hwndFrom {
                                    // keyboard focus inside the group: underline the segment
                                    let u = RECT { left: r.left + st.px(10), right: r.right - st.px(10), top: r.bottom - st.px(5), bottom: r.bottom - st.px(4) };
                                    p.fill(u, st.theme.ink);
                                }
                            });
                        }
                        _ => {
                            if let Some(i) = st.rows.iter().position(|r| r.focus_id == id) {
                                st.paint(cd.hdc, size, (origin.x, origin.y), |p| st.paint_row(p, i, hot, pressed));
                            }
                        }
                    }
                    return LRESULT(CDRF_SKIPDEFAULT as isize);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DRAWITEM => {
                let di = &*(lparam.0 as *const DRAWITEMSTRUCT);
                let r = di.rcItem;
                let mut wr = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(di.hwndItem, &mut wr);
                let mut origin = POINT { x: wr.left, y: wr.top };
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut origin);
                let kind = match di.CtlID as i32 {
                    ID_SEND_LABEL => Kind::Send,
                    ID_SUB_LABEL => Kind::Sub,
                    _ => Kind::Name,
                };
                if let Some(i) = st.rows.iter().position(|row| row.kind == kind) {
                    let label = match kind {
                        Kind::Send => "send to",
                        Kind::Sub => "subfolder",
                        _ => "file name",
                    };
                    st.paint(di.hDC, (r.right, r.bottom), (origin.x, origin.y), |p| {
                        st.paint_row(p, i, false, false);
                        let row = &st.rows[i];
                        let line = RECT { left: row.rect.left + st.px(PAD), right: row.rect.right, top: row.rect.top, bottom: row.rect.top + st.px(ROW) };
                        p.text(label, &st.fonts.body, frect(line), st.theme.ink);
                    });
                }
                LRESULT(1)
            }
            WM_CTLCOLOREDIT => {
                let hdc = HDC(wparam.0 as *mut c_void);
                SetTextColor(hdc, colorref(st.theme.ink));
                SetBkColor(hdc, colorref(st.theme.cap));
                LRESULT(st.edit_brush.0 as isize)
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xffff) as i32;
                let code = ((wparam.0 >> 16) & 0xffff) as u32;
                if id == IDCANCEL.0 && lparam.0 == 0 {
                    // esc from the dialog manager: leave recording, else close.
                    if st.recording.is_some() {
                        st.stop_recording(false);
                    } else {
                        let _ = DestroyWindow(hwnd);
                    }
                    return LRESULT(0);
                }
                st.command(id, code);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_SAVE_NAME => {
                let _ = KillTimer(Some(hwnd), TIMER_SAVE_NAME);
                st.save();
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                st.apply_theme();
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let suggested = *(lparam.0 as *const RECT);
                st.set_dpi((wparam.0 & 0xffff) as u32, suggested);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER_SAVE_NAME);
                if st.recording.is_some() {
                    st.stop_recording(false);
                }
                // a pending file name change is saved, not lost.
                if settings::Settings::load(&st.path).0.file_name != st.settings.file_name && st.name_error.is_none() {
                    let _ = settings::save(&st.path, &st.settings);
                    notify(st.app, WM_KADR_SETTINGS, 0);
                }
                windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if !p.is_null() {
                    drop(Box::from_raw(p));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// hotkey buttons: while recording, every key goes to the button instead of
/// the dialog manager (tab, esc, enter, alt menus).
unsafe extern "system" fn hotkey_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    // SAFETY: the parent is our settings window with a live State.
    unsafe {
        let parent = windows::Win32::UI::WindowsAndMessaging::GetParent(hwnd).unwrap_or_default();
        if let Some(st) = state(parent)
            && st.recording.is_some()
        {
            match msg {
                WM_GETDLGCODE => return LRESULT(DLGC_WANTALLKEYS as isize),
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    st.key((wparam.0 & 0xffff) as u16, false);
                    return LRESULT(0);
                }
                WM_KEYUP | WM_SYSKEYUP => {
                    st.key((wparam.0 & 0xffff) as u16, true);
                    return LRESULT(0);
                }
                WM_CHAR | WM_SYSCHAR => return LRESULT(0),
                WM_KILLFOCUS => st.stop_recording(false),
                _ => {}
            }
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}
