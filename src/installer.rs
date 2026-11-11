//! the installer window: two switches (start with windows, start kadr now)
//! and one button. standard windows controls for keyboard and screen
//! readers, drawn with the same kit as the settings window (ui.rs).

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWINDOWATTRIBUTE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, PAINTSTRUCT,
    RDW_ALLCHILDREN, RDW_INVALIDATE, RedrawWindow, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{CDDS_PREPAINT, CDIS_HOT, CDIS_SELECTED, CDRF_DODEFAULT, CDRF_SKIPDEFAULT, NM_CUSTOMDRAW, NMCUSTOMDRAW, NMHDR};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BN_CLICKED, BN_KILLFOCUS, BN_SETFOCUS, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_NOTIFY,
    BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, DispatchMessageW,
    GWLP_USERDATA, GetClientRect, GetCursorPos, GetDlgItem, GetMessageW, GetWindowLongPtrW, GetWindowRect, HICON, HMENU,
    ICON_BIG, ICON_SMALL, IDC_ARROW, IDC_WAIT, IsDialogMessageW, LoadCursorW, MB_ICONWARNING, MB_OK, MSG, MessageBoxW,
    PostQuitMessage, RegisterClassW, SM_CXICON, SM_CXSMICON, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER,
    SendMessageW, SetCursor, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_NCDESTROY,
    WM_NOTIFY, WM_PAINT, WM_SETICON, WM_SETTINGCHANGE, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_GROUP,
    WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, Result, w};

use crate::install::{self, Options};
use crate::ui::{self, Canvas, Fonts, Painter, Theme, colorref, frect, make_fonts};
use crate::{trace, wide};

const CLASS: PCWSTR = w!("kadr.installer");
const ID_AUTOSTART: i32 = 10;
const ID_RUN: i32 = 11;
const ID_OK: i32 = 1; // IDOK: enter
const ID_CANCEL: i32 = 2; // IDCANCEL: esc

const WIDTH: i32 = 460;
const MARGIN: i32 = 24;
const ROW: i32 = 44;
const PAD: i32 = 16;

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Ask,
    Done { ran: bool },
}

struct State {
    hwnd: HWND,
    canvas: Canvas,
    fonts: Fonts,
    theme: Theme,
    dpi: u32,
    page: Page,
    previous: Option<String>,
    title_rect: RECT,
    text_rect: RECT,
    group: RECT,
    rows: [RECT; 2],
    icons: [HICON; 2],
}

/// shows the installer and returns the process exit code.
pub fn run() -> i32 {
    match open() {
        Ok(_) => {}
        Err(e) => {
            message(HWND::default(), &format!("could not start the installer\n\n{}", e.message()));
            return 1;
        }
    }
    let mut msg = MSG::default();
    // SAFETY: standard message loop with dialog-style keyboard handling.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if let Ok(d) = windows::Win32::UI::WindowsAndMessaging::FindWindowW(CLASS, None)
                && IsDialogMessageW(d, &msg).as_bool()
            {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    msg.wParam.0 as i32
}

fn open() -> Result<HWND> {
    // SAFETY: standard window creation; the State box lives in GWLP_USERDATA
    // until WM_NCDESTROY.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("kadr setup"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            0,
            0,
            WIDTH,
            400,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let dpi = GetDpiForWindow(hwnd).max(96);
        let canvas = Canvas::new()?;
        let fonts = make_fonts(&canvas.dwrite, dpi)?;
        let state = State {
            hwnd,
            canvas,
            fonts,
            theme: Theme::current(),
            dpi,
            page: Page::Ask,
            previous: install::installed_version(),
            title_rect: RECT::default(),
            text_rect: RECT::default(),
            group: RECT::default(),
            rows: [RECT::default(); 2],
            icons: [HICON::default(); 2],
        };
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(Box::new(state)) as isize);
        let st = &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State);
        st.create_controls();
        st.apply_theme();
        st.layout(true);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        if let Ok(ok) = GetDlgItem(Some(hwnd), ID_OK) {
            let _ = SetFocus(Some(ok));
        }
        Ok(hwnd)
    }
}

impl State {
    fn px(&self, v: i32) -> i32 {
        (v * self.dpi as i32 + 48) / 96
    }

    fn control(&self, id: i32) -> Option<HWND> {
        // SAFETY: plain child lookup.
        unsafe { GetDlgItem(Some(self.hwnd), id).ok() }
    }

    fn create_controls(&mut self) {
        let mk = |id: i32, text: &str, style: u32| {
            let t = wide(text);
            // SAFETY: child window of our own window with a unique id.
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("BUTTON"),
                    PCWSTR(t.as_ptr()),
                    WINDOW_STYLE(style) | WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    0,
                    0,
                    10,
                    10,
                    Some(self.hwnd),
                    Some(HMENU(id as isize as *mut c_void)),
                    None,
                    None,
                )
                .unwrap_or_default()
            }
        };
        let check = BS_AUTOCHECKBOX as u32 | BS_NOTIFY as u32;
        mk(ID_AUTOSTART, "start with windows", check | WS_GROUP.0);
        let run = mk(ID_RUN, "start kadr now", check);
        mk(ID_OK, self.primary_label(), BS_DEFPUSHBUTTON as u32 | BS_NOTIFY as u32 | WS_GROUP.0);
        mk(ID_CANCEL, "cancel", BS_PUSHBUTTON as u32 | BS_NOTIFY as u32);
        // SAFETY: our own check box.
        unsafe {
            SendMessageW(run, BM_SETCHECK, Some(WPARAM(1)), None);
        }
    }

    fn primary_label(&self) -> &'static str {
        match (self.page, &self.previous) {
            (Page::Done { .. }, _) => "close",
            (Page::Ask, Some(_)) => "update",
            (Page::Ask, None) => "install",
        }
    }

    fn title(&self) -> String {
        match (self.page, &self.previous) {
            (Page::Done { .. }, Some(_)) => "kadr is updated".into(),
            (Page::Done { .. }, None) => "kadr is installed".into(),
            (Page::Ask, Some(_)) => "update kadr".into(),
            (Page::Ask, None) => "install kadr".into(),
        }
    }

    fn text(&self) -> String {
        // relative to the user folder when it is inside it: shorter, and the same on every pc.
        let full = install::paths().dir;
        let home = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from);
        let dir = match home.as_ref().and_then(|h| full.strip_prefix(h).ok()) {
            Some(rel) => rel.display().to_string().to_lowercase(),
            None => full.display().to_string(),
        };
        match (self.page, &self.previous) {
            (Page::Ask, None) => format!("for you only, in {dir}. no administrator rights, nothing running in the background."),
            (Page::Ask, Some(old)) => format!("replaces kadr {old} with {}. a running kadr is closed first. your settings stay.", install::VERSION),
            (Page::Done { ran: true }, _) => "kadr is in the notification area. ctrl+printscreen captures a region. to remove it, use windows settings › apps.".into(),
            (Page::Done { ran: false }, _) => "start it from the start menu. ctrl+printscreen captures a region. to remove it, use windows settings › apps.".into(),
        }
    }

    fn checked(&self, id: i32) -> bool {
        // SAFETY: plain message to our own check box.
        self.control(id).is_some_and(|c| unsafe { SendMessageW(c, BM_GETCHECK, None, None).0 == 1 })
    }

    fn apply_theme(&mut self) {
        self.theme = Theme::current();
        let dark = self.theme.dark as i32;
        let caption = colorref(self.theme.ground).0;
        let text = colorref(self.theme.ink).0;
        // SAFETY: documented dwm attributes (20 dark mode, 35 caption, 36 text).
        unsafe {
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWINDOWATTRIBUTE(20), (&dark as *const i32).cast(), 4);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWINDOWATTRIBUTE(35), (&caption as *const u32).cast(), 4);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWINDOWATTRIBUTE(36), (&text as *const u32).cast(), 4);
            let small = ui::app_icon(GetSystemMetricsForDpi(SM_CXSMICON, self.dpi));
            let big = ui::app_icon(GetSystemMetricsForDpi(SM_CXICON, self.dpi));
            SendMessageW(self.hwnd, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(LPARAM(small.0 as isize)));
            SendMessageW(self.hwnd, WM_SETICON, Some(WPARAM(ICON_BIG as usize)), Some(LPARAM(big.0 as isize)));
            for old in std::mem::replace(&mut self.icons, [small, big]) {
                if !old.is_invalid() {
                    let _ = DestroyIcon(old);
                }
            }
        }
        self.redraw();
    }

    fn redraw(&self) {
        // SAFETY: our own window.
        unsafe {
            let _ = RedrawWindow(Some(self.hwnd), None, None, RDW_INVALIDATE | RDW_ALLCHILDREN);
        }
    }

    fn layout(&mut self, center: bool) {
        let dpi = self.dpi as i32;
        let p = |v: i32| (v * dpi + 48) / 96;
        let width = p(WIDTH);
        let (left, right) = (p(MARGIN), width - p(MARGIN));
        self.title_rect = RECT { left: left + p(8), top: p(16), right, bottom: p(48) };
        let text = self.text();
        let max = (right - left - p(16)) as f32;
        let (_, h) = self.canvas.measure_box(&text, &self.fonts.para, max);
        self.text_rect = RECT { left: left + p(8), top: p(52), right: right - p(8), bottom: p(52) + h.ceil() as i32 };
        let mut y = self.text_rect.bottom + p(20);
        let asking = self.page == Page::Ask;
        if asking {
            self.group = RECT { left, top: y, right, bottom: y + 2 * p(ROW) };
            self.rows = [RECT { bottom: y + p(ROW), ..self.group }, RECT { top: y + p(ROW), ..self.group }];
            y = self.group.bottom + p(20);
        }
        // buttons: pill plus a 4 px frame for the focus marks
        let bh = p(32) + 2 * p(4);
        let bw = |label: &str| (self.canvas.measure(label, &self.fonts.body).ceil() as i32 + p(40)).max(p(96)) + 2 * p(4);
        let ok_w = bw(self.primary_label());
        let ok = RECT { left: right + p(4) - ok_w, top: y, right: right + p(4), bottom: y + bh };
        let cancel_w = bw("cancel");
        let cancel = RECT { left: ok.left - cancel_w, top: y, right: ok.left, bottom: y + bh };
        let height = ok.bottom - p(4) + p(MARGIN);
        let put = |id: i32, r: RECT, show: bool| {
            // SAFETY: moving and showing our own child controls.
            unsafe {
                if let Ok(c) = GetDlgItem(Some(self.hwnd), id) {
                    let _ = SetWindowPos(c, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                    let _ = ShowWindow(c, if show { SW_SHOW } else { SW_HIDE });
                }
            }
        };
        put(ID_AUTOSTART, self.rows[0], asking);
        put(ID_RUN, self.rows[1], asking);
        put(ID_OK, ok, true);
        put(ID_CANCEL, cancel, asking);
        // SAFETY: sizing and placing our own window.
        unsafe {
            if let Some(c) = self.control(ID_OK) {
                let t = wide(self.primary_label());
                let _ = SetWindowTextW(c, PCWSTR(t.as_ptr()));
            }
            let mut r = RECT { left: 0, top: 0, right: width, bottom: height };
            let _ = AdjustWindowRectExForDpi(&mut r, WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU, false, WINDOW_EX_STYLE(0), self.dpi);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let mut cursor = POINT::default();
            let _ = GetCursorPos(&mut cursor);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST), &mut info);
            let wa = info.rcWork;
            if center {
                let x = wa.left + ((wa.right - wa.left) - w) / 2;
                let y = wa.top + ((wa.bottom - wa.top) - h) / 3;
                let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            } else {
                let mut now = RECT::default();
                let _ = GetWindowRect(self.hwnd, &mut now);
                let _ = SetWindowPos(self.hwnd, None, now.left, now.top.min(wa.bottom - h).max(wa.top), w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        self.redraw();
    }

    fn paint_window(&self, p: &Painter, client: RECT) {
        let t = self.theme;
        p.fill(client, t.ground);
        p.text(&self.title(), &self.fonts.title, frect(self.title_rect), t.ink);
        p.text(&self.text(), &self.fonts.para, frect(self.text_rect), t.ink2);
        if self.page == Page::Ask {
            for i in 0..2 {
                self.paint_row(p, i, false);
            }
        }
    }

    fn paint_row(&self, p: &Painter, i: usize, hot: bool) {
        let t = self.theme;
        let row = self.rows[i];
        let radius = self.px(12) as f32;
        p.fill(row, t.ground);
        p.fill_round(self.group, radius, t.raised);
        let (id, _) = if i == 0 { (ID_AUTOSTART, ()) } else { (ID_RUN, ()) };
        // keyboard focus shows as the hover tint, as in settings
        if hot || self.focused() == Some(id) {
            p.clip(row, || p.fill_round(self.group, radius, t.hover));
        }
        p.stroke_round(self.group, radius, t.hair);
        if i == 1 {
            let y = row.top as f32 + 0.5;
            p.line((row.left as f32 + self.px(PAD) as f32, y), (row.right as f32 - self.px(PAD) as f32, y), t.hair);
        }
        let (id, label) = if i == 0 { (ID_AUTOSTART, "start with windows") } else { (ID_RUN, "start kadr now") };
        p.text(label, &self.fonts.body, frect(RECT { left: row.left + self.px(PAD), ..row }), t.ink);
        self.paint_switch(p, row, self.checked(id));
    }

    fn paint_switch(&self, p: &Painter, line: RECT, on: bool) {
        let t = self.theme;
        let (w, h) = (self.px(36), self.px(20));
        let top = line.top + (line.bottom - line.top - h) / 2;
        let r = RECT { left: line.right - self.px(PAD) - w, top, right: line.right - self.px(PAD), bottom: top + h };
        let knob = self.px(12);
        let ky = r.top + (h - knob) / 2;
        if on {
            p.fill_round(r, h as f32 / 2.0, t.ink);
            let k = RECT { left: r.right - self.px(4) - knob, top: ky, right: r.right - self.px(4), bottom: ky + knob };
            p.fill_round(k, knob as f32 / 2.0, t.raised);
        } else {
            p.stroke_round(r, h as f32 / 2.0, [t.ink3[0], t.ink3[1], t.ink3[2], 1.0]);
            let k = RECT { left: r.left + self.px(4), top: ky, right: r.left + self.px(4) + knob, bottom: ky + knob };
            p.fill_round(k, knob as f32 / 2.0, t.ink2);
        }
    }

    /// a push button: the pill sits 4 px inside the control, so the focus
    /// ring fits around it inside the control.
    fn paint_button(&self, p: &Painter, id: i32, r: RECT, hot: bool, pressed: bool) {
        let t = self.theme;
        p.fill(r, t.ground);
        let inset = self.px(4);
        let pill = RECT { left: r.left + inset, top: r.top + inset, right: r.right - inset, bottom: r.bottom - inset };
        let label = if id == ID_OK { self.primary_label() } else { "cancel" };
        let radius = self.px(8) as f32;
        if id == ID_OK {
            let fill = if pressed { t.ink2 } else { t.ink };
            p.fill_round(pill, radius, fill);
            p.text_center(label, &self.fonts.body, frect(pill), t.ground);
        } else {
            p.fill_round(pill, radius, if pressed || hot { t.hover } else { t.cap });
            p.stroke_round(pill, radius, t.hair);
            p.text_center(label, &self.fonts.body, frect(pill), t.ink);
        }
        if self.focused() == Some(id) {
            let ring = RECT { left: r.left + self.px(1), top: r.top + self.px(1), right: r.right - self.px(1), bottom: r.bottom - self.px(1) };
            p.stroke_round(ring, radius + self.px(3) as f32, [t.ink[0], t.ink[1], t.ink[2], 0.5]);
        }
    }

    fn focused(&self) -> Option<i32> {
        // SAFETY: plain focus query.
        let f = unsafe { GetFocus() };
        [ID_AUTOSTART, ID_RUN, ID_OK, ID_CANCEL].into_iter().find(|id| self.control(*id) == Some(f))
    }

    fn command(&mut self, id: i32, code: u32) {
        match (id, code) {
            (ID_OK, c) if c == BN_CLICKED => match self.page {
                Page::Ask => self.do_install(),
                Page::Done { .. } => self.close(),
            },
            (ID_CANCEL, c) if c == BN_CLICKED => self.close(),
            (ID_AUTOSTART | ID_RUN, c) if c == BN_CLICKED => self.redraw(),
            (_, c) if c == BN_SETFOCUS || c == BN_KILLFOCUS => self.redraw(),
            _ => {}
        }
    }

    fn do_install(&mut self) {
        let options = Options { autostart: self.checked(ID_AUTOSTART), run: self.checked(ID_RUN) };
        // SAFETY: busy cursor and disabled buttons while the files are copied.
        unsafe {
            for id in [ID_OK, ID_CANCEL, ID_AUTOSTART, ID_RUN] {
                if let Some(c) = self.control(id) {
                    let _ = EnableWindow(c, false);
                }
            }
            if let Ok(wait) = LoadCursorW(None, IDC_WAIT) {
                SetCursor(Some(wait));
            }
        }
        let result = install::install(&options);
        // SAFETY: as above.
        unsafe {
            for id in [ID_OK, ID_CANCEL, ID_AUTOSTART, ID_RUN] {
                if let Some(c) = self.control(id) {
                    let _ = EnableWindow(c, true);
                }
            }
        }
        match result {
            Ok(()) => {
                trace!("installer: done");
                self.page = Page::Done { ran: options.run };
                self.layout(false);
                if let Some(ok) = self.control(ID_OK) {
                    // SAFETY: focus our own button.
                    unsafe {
                        let _ = SetFocus(Some(ok));
                    }
                }
            }
            Err(e) => message(self.hwnd, &format!("could not install kadr\n\n{e}")),
        }
    }

    fn close(&self) {
        // SAFETY: our own window.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for icon in self.icons {
            if !icon.is_invalid() {
                // SAFETY: our own icons.
                unsafe {
                    let _ = DestroyIcon(icon);
                }
            }
        }
    }
}

fn message(owner: HWND, text: &str) {
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
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                st.canvas.paint(hdc, (client.right, client.bottom), (0, 0), |p| st.paint_window(p, client));
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_NOTIFY => {
                let hdr = &*(lparam.0 as *const NMHDR);
                if hdr.code != NM_CUSTOMDRAW {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                let cd = &*(lparam.0 as *const NMCUSTOMDRAW);
                if cd.dwDrawStage != CDDS_PREPAINT {
                    return LRESULT(CDRF_DODEFAULT as isize);
                }
                let hot = (cd.uItemState.0 & CDIS_HOT.0) != 0;
                let pressed = (cd.uItemState.0 & CDIS_SELECTED.0) != 0;
                let mut wr = RECT::default();
                let _ = GetWindowRect(hdr.hwndFrom, &mut wr);
                let mut origin = POINT { x: wr.left, y: wr.top };
                let _ = ScreenToClient(hwnd, &mut origin);
                let size = (cd.rc.right - cd.rc.left, cd.rc.bottom - cd.rc.top);
                let r = RECT { left: origin.x, top: origin.y, right: origin.x + size.0, bottom: origin.y + size.1 };
                let id = hdr.idFrom as i32;
                st.canvas.paint(cd.hdc, size, (origin.x, origin.y), |p| match id {
                    ID_AUTOSTART => st.paint_row(p, 0, hot),
                    ID_RUN => st.paint_row(p, 1, hot),
                    _ => st.paint_button(p, id, r, hot, pressed),
                });
                LRESULT(CDRF_SKIPDEFAULT as isize)
            }
            WM_COMMAND => {
                st.command((wparam.0 & 0xffff) as i32, ((wparam.0 >> 16) & 0xffff) as u32);
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                st.apply_theme();
                LRESULT(0)
            }
            WM_DPICHANGED => {
                st.dpi = ((wparam.0 & 0xffff) as u32).max(96);
                if let Ok(f) = make_fonts(&st.canvas.dwrite, st.dpi) {
                    st.fonts = f;
                }
                let s = *(lparam.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, s.left, s.top, s.right - s.left, s.bottom - s.top, SWP_NOZORDER | SWP_NOACTIVATE);
                st.apply_theme();
                st.layout(false);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
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
