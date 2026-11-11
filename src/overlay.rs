//! region selection overlay.
//!
//! a borderless topmost window over the whole virtual screen. it shows the
//! frozen frame dimmed, and the selection at full brightness. it lives only
//! while the user selects: `select` creates it, runs a modal message loop,
//! destroys it, and every gdi object is released before returning.
//!
//! the result is cut from the frozen frame, so the overlay itself can never
//! appear in the capture.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DT_CENTER, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, EndPaint, FF_DONTCARE, FW_NORMAL,
    FillRect, GetStockObject, GetTextExtentPoint32W, HBRUSH, HDC, HFONT, HGDIOBJ, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MonitorFromPoint, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, RoundRect,
    SRCCOPY, SelectObject, SetBkMode, SetTextColor, SetViewportOrgEx, TRANSPARENT, UpdateWindow,
};
use windows::Win32::Foundation::COLORREF;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, IDC_CROSS, LoadCursorW, MSG, PostQuitMessage,
    RegisterClassW, SW_SHOW, SetCursor, SetForegroundWindow, SetWindowLongPtrW, ShowWindow,
    TranslateMessage, WA_INACTIVE, WM_ACTIVATE, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_RBUTTONUP,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Error, PCWSTR, Result, w};

use crate::capture::{Frame, MemoryDc, Rect, ScreenDc};

const CLASS: PCWSTR = w!("kadr.overlay");
/// fraction of brightness kept outside the selection, out of 256.
const DIM_KEEP: u8 = 140;
const HINT: &str = "drag to select · ctrl+a: whole screen · esc to cancel";

/// shows the overlay over `frame`, which covers `screen` (the virtual
/// screen). returns the selected rectangle in frame coordinates, or `None`
/// if the user cancelled. `on_shown` runs once the first paint is done.
/// `on_selected` runs while the overlay is still on screen, so whatever it
/// opens can appear before the overlay goes (no flash of the desktop).
pub fn select(frame: &Frame, screen: Rect, on_shown: impl FnOnce(), on_selected: impl FnOnce(Rect)) -> Result<Option<Rect>> {
    register_class();
    let t = std::time::Instant::now();
    let state = Box::into_raw(Box::new(State::new(frame, screen)?));
    crate::trace!("overlay state (dimmed copy, dcs): {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);

    // SAFETY: `state` stays valid until it is reclaimed below, after the
    // window (the only other user of the pointer) has been destroyed.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let hwnd = match CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            w!("kadr"),
            WS_POPUP,
            screen.x,
            screen.y,
            screen.width,
            screen.height,
            None,
            None,
            Some(instance.into()),
            Some(state as *const c_void),
        ) {
            Ok(hwnd) => hwnd,
            Err(err) => {
                drop(Box::from_raw(state));
                return Err(err);
            }
        };

        // no fade-in and no rounded corners: the overlay must look like the
        // screen itself, instantly. the corner call fails on windows 10; fine.
        let on: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, (&on as *const i32).cast(), 4);
        let corners = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corners as *const _ as *const c_void, 4);

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd); // paints synchronously
        on_shown();
        let _ = SetForegroundWindow(hwnd);
        if let Ok(cursor) = LoadCursorW(None, IDC_CROSS) {
            SetCursor(Some(cursor));
        }

        let mut msg = MSG::default();
        let result = loop {
            if let Some(result) = (*state).result {
                break result;
            }
            match GetMessageW(&mut msg, None, 0, 0).0 {
                0 => {
                    // WM_QUIT belongs to the outer loop: put it back and cancel.
                    PostQuitMessage(msg.wParam.0 as i32);
                    break None;
                }
                -1 => break None,
                _ => {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        };

        if let Some(rect) = result {
            on_selected(rect);
        }
        let _ = DestroyWindow(hwnd);
        drop(Box::from_raw(state));
        Ok(result)
    }
}

fn register_class() {
    // SAFETY: registering an already registered class just fails; ignored.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class);
    }
}

struct State {
    /// memory dcs with the bright and dimmed frames selected for the whole
    /// lifetime of the overlay, so painting is just BitBlt.
    bright: HDC,
    bright_prev: HGDIOBJ,
    dim: HDC,
    dim_prev: HGDIOBJ,
    _dimmed: Frame,
    width: i32,
    height: i32,
    /// virtual-screen origin, to turn client points into screen points.
    origin: POINT,
    anchor: Option<POINT>,
    selection: Option<RECT>,
    hint: Option<RECT>,
    font: HFONT,
    font_dpi: u32,
    result: Option<Option<Rect>>,
}

impl State {
    fn new(frame: &Frame, screen: Rect) -> Result<State> {
        let dimmed = frame.dimmed(DIM_KEEP)?;
        let screen_dc = ScreenDc::get()?;
        // SAFETY: dcs are created here and deleted in drop, after the
        // previously selected objects are restored.
        unsafe {
            let bright = CreateCompatibleDC(Some(screen_dc.0));
            let dim = CreateCompatibleDC(Some(screen_dc.0));
            if bright.is_invalid() || dim.is_invalid() {
                let _ = DeleteDC(bright);
                let _ = DeleteDC(dim);
                return Err(Error::from_thread());
            }
            let bright_prev = SelectObject(bright, HGDIOBJ(frame.bitmap().0));
            let dim_prev = SelectObject(dim, HGDIOBJ(dimmed.bitmap().0));
            let mut state = State {
                bright,
                bright_prev,
                dim,
                dim_prev,
                _dimmed: dimmed,
                width: screen.width,
                height: screen.height,
                origin: POINT { x: screen.x, y: screen.y },
                anchor: None,
                selection: None,
                hint: None,
                font: HFONT::default(),
                font_dpi: 0,
                result: None,
            };
            state.hint = state.hint_rect();
            Ok(state)
        }
    }

    /// dpi of the monitor under a client point.
    fn dpi_at(&self, p: POINT) -> u32 {
        let screen = POINT { x: p.x + self.origin.x, y: p.y + self.origin.y };
        let (mut x, mut y) = (96, 96);
        // SAFETY: out-pointers to locals.
        unsafe {
            let monitor = MonitorFromPoint(screen, MONITOR_DEFAULTTONEAREST);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y);
        }
        x.max(96)
    }

    fn px(dpi: u32, value: i32) -> i32 {
        (value * dpi as i32 + 48) / 96
    }

    /// selects a 12pt-ish ui font for `dpi` into the measuring dc.
    fn use_font(&mut self, dpi: u32) {
        if self.font_dpi == dpi && !self.font.is_invalid() {
            return;
        }
        // SAFETY: the old font is deselected before being deleted.
        unsafe {
            let font = CreateFontW(
                -Self::px(dpi, 12),
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                FF_DONTCARE.0 as u32,
                w!("Segoe UI"),
            );
            SelectObject(self.bright, HGDIOBJ(font.0));
            if !self.font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
            self.font = font;
            self.font_dpi = dpi;
        }
    }

    /// pill rectangle that fits `text` with padding, top-left at (x, y).
    fn pill(&mut self, dpi: u32, text: &[u16], x: i32, y: i32) -> RECT {
        self.use_font(dpi);
        let mut size = Default::default();
        // SAFETY: the font is selected into `bright`; out-pointer to a local.
        unsafe {
            let _ = GetTextExtentPoint32W(self.bright, text, &mut size);
        }
        let (pad_x, pad_y) = (Self::px(dpi, 8), Self::px(dpi, 4));
        RECT { left: x, top: y, right: x + size.cx + pad_x * 2, bottom: y + size.cy + pad_y * 2 }
    }

    fn hint_rect(&mut self) -> Option<RECT> {
        let cursor = crate::capture::cursor_pos().ok()?;
        let monitor = crate::capture::monitor_at(cursor).ok()?;
        let local = POINT { x: cursor.x - self.origin.x, y: cursor.y - self.origin.y };
        let dpi = self.dpi_at(local);
        let text = to_wide(HINT);
        let r = self.pill(dpi, &text, 0, 0);
        let x = monitor.x - self.origin.x + (monitor.width - r.right) / 2;
        let y = monitor.y - self.origin.y + Self::px(dpi, 24);
        Some(offset(r, x, y))
    }

    /// the "w × h" label: above the selection, or inside it at the top edge.
    fn label(&mut self, sel: RECT) -> (RECT, Vec<u16>) {
        let dpi = self.dpi_at(POINT { x: sel.left, y: sel.top });
        let text = to_wide(&format!("{} × {}", sel.right - sel.left, sel.bottom - sel.top));
        let r = self.pill(dpi, &text, 0, 0);
        // clear of the crop marks above the selection.
        let gap = Self::px(dpi, 10);
        let mut y = sel.top - gap - r.bottom;
        if y < 0 {
            y = sel.top + gap;
        }
        let x = sel.left.clamp(0, (self.width - r.right).max(0));
        (offset(r, x, y), text)
    }

    fn border(&self, sel: RECT) -> i32 {
        Self::px(self.dpi_at(POINT { x: sel.left, y: sel.top }), 1).max(1)
    }

    /// everything a selection draws: the frame, its border and the label.
    fn mark_width(&self, sel: RECT) -> i32 {
        Self::px(self.dpi_at(POINT { x: sel.left, y: sel.top }), 2).max(2)
    }

    fn mark_gap(&self, sel: RECT) -> i32 {
        Self::px(self.dpi_at(POINT { x: sel.left, y: sel.top }), 2)
    }

    fn mark_len(&self, sel: RECT) -> i32 {
        Self::px(self.dpi_at(POINT { x: sel.left, y: sel.top }), 14)
    }

    fn bounds(&mut self, sel: RECT) -> RECT {
        let b = self.border(sel) + 2 * self.mark_width(sel) + self.mark_gap(sel);
        let (label, _) = self.label(sel);
        union(inflate(sel, b), label)
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // SAFETY: restore what was selected, then delete our objects. the
        // frames themselves are freed by their own drop afterwards.
        unsafe {
            SelectObject(self.bright, self.bright_prev);
            SelectObject(self.dim, self.dim_prev);
            let _ = DeleteDC(self.bright);
            let _ = DeleteDC(self.dim);
            if !self.font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: GWLP_USERDATA holds the State pointer from WM_NCCREATE until
    // WM_NCDESTROY; `select` keeps it alive for that whole span.
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let st = &mut *state;
        match msg {
            WM_PAINT => {
                paint(hwnd, st);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_LBUTTONDOWN => {
                let p = point(lparam);
                st.anchor = Some(p);
                if let Some(hint) = st.hint.take() {
                    let _ = InvalidateRect(Some(hwnd), Some(&hint), false);
                }
                SetCapture(hwnd);
                LRESULT(0)
            }
            // a move without the button held is the real pointer, not this
            // drag (a drag always holds the button)
            WM_MOUSEMOVE if wparam.0 & 0x0001 != 0 => {
                if let Some(anchor) = st.anchor {
                    let p = point(lparam);
                    let next = normalize(anchor, p, st.width, st.height);
                    if st.selection != Some(next) {
                        if let Some(old) = st.selection {
                            let r = st.bounds(old);
                            let _ = InvalidateRect(Some(hwnd), Some(&r), false);
                        }
                        let r = st.bounds(next);
                        let _ = InvalidateRect(Some(hwnd), Some(&r), false);
                        st.selection = Some(next);
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if let Some(anchor) = st.anchor.take() {
                    let _ = ReleaseCapture();
                    // the release point ends the selection, not the last move:
                    // a move generated after SetCapture can lag behind it.
                    st.selection = Some(normalize(anchor, point(lparam), st.width, st.height));
                    match st.selection {
                        Some(s) if s.right - s.left >= 2 && s.bottom - s.top >= 2 => {
                            st.result = Some(Some(Rect::from_win(s)));
                        }
                        // a click or a tiny drag: start over instead of
                        // producing a 1-pixel capture.
                        Some(s) => {
                            let r = st.bounds(s);
                            let _ = InvalidateRect(Some(hwnd), Some(&r), false);
                            st.selection = None;
                        }
                        None => {}
                    }
                }
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                st.result = Some(None);
                LRESULT(0)
            }
            WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
                st.result = Some(None);
                LRESULT(0)
            }
            // ctrl+a: the whole monitor under the pointer
            WM_KEYDOWN if wparam.0 == b'A' as usize && GetKeyState(VK_CONTROL.0 as i32) < 0 => {
                if let Ok(m) = crate::capture::monitor_at_cursor() {
                    let r = Rect { x: m.x - st.origin.x, y: m.y - st.origin.y, width: m.width, height: m.height };
                    crate::trace!("overlay: ctrl+a, whole monitor {}x{}", r.width, r.height);
                    st.result = Some(Some(r));
                }
                LRESULT(0)
            }
            WM_ACTIVATE if (wparam.0 & 0xffff) as u32 == WA_INACTIVE => {
                // another window took focus (alt+tab, uac): do not stay stuck
                // on top of everything without keyboard input.
                st.result = Some(None);
                LRESULT(0)
            }
            // the overlay spans monitors with different dpi; never let
            // windows resize it.
            WM_DPICHANGED => LRESULT(0),
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// composes the invalid area in an off-screen bitmap, then copies it once,
/// so nothing flickers.
unsafe fn paint(hwnd: HWND, st: &mut State) {
    // SAFETY: called only from WM_PAINT for our window; every object created
    // here is deselected and deleted before returning.
    unsafe {
        let started = std::time::Instant::now();
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let rc = ps.rcPaint;
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        if w > 0 && h > 0 {
            if let Ok(back) = MemoryDc::compatible_with(hdc) {
                let bitmap = CreateCompatibleBitmap(hdc, w, h);
                if !bitmap.is_invalid() {
                    let selected = back.select(bitmap);
                    // draw in client coordinates.
                    let _ = SetViewportOrgEx(back.0, -rc.left, -rc.top, None);
                    let _ = BitBlt(back.0, rc.left, rc.top, w, h, Some(st.dim), rc.left, rc.top, SRCCOPY);
                    if let Some(sel) = st.selection {
                        draw_selection(back.0, st, sel, rc);
                    } else if let Some(hint) = st.hint {
                        let text = to_wide(HINT);
                        let dpi = st.dpi_at(POINT { x: hint.left, y: hint.top });
                        draw_pill(back.0, st, dpi, hint, &text);
                    }
                    let _ = SetViewportOrgEx(back.0, 0, 0, None);
                    let _ = BitBlt(hdc, rc.left, rc.top, w, h, Some(back.0), 0, 0, SRCCOPY);
                    drop(selected);
                    let _ = DeleteObject(HGDIOBJ(bitmap.0));
                }
            }
        }
        let _ = EndPaint(hwnd, &ps);
        crate::trace!("paint {}x{}: {:.2} ms", w, h, started.elapsed().as_secs_f64() * 1000.0);
    }
}

unsafe fn draw_selection(dc: HDC, st: &mut State, sel: RECT, clip: RECT) {
    // SAFETY: dc is the back buffer; brushes are deleted after use.
    unsafe {
        if let Some(part) = intersect(sel, clip) {
            let _ = BitBlt(
                dc,
                part.left,
                part.top,
                part.right - part.left,
                part.bottom - part.top,
                Some(st.bright),
                part.left,
                part.top,
                SRCCOPY,
            );
        }
        let b = st.border(sel);
        let white = CreateSolidBrush(COLORREF(0x00ff_ffff));
        let o = inflate(sel, b);
        for strip in [
            RECT { left: o.left, top: o.top, right: o.right, bottom: sel.top },
            RECT { left: o.left, top: sel.bottom, right: o.right, bottom: o.bottom },
            RECT { left: o.left, top: sel.top, right: sel.left, bottom: sel.bottom },
            RECT { left: sel.right, top: sel.top, right: o.right, bottom: sel.bottom },
        ] {
            FillRect(dc, &strip, white);
        }
        // the signature: crop marks on the four corners, just outside the
        // border. the same corners as the tray icon and the settings window.
        let (len, w) = (st.mark_len(sel), st.mark_width(sel));
        // a small gap, so they read as separate crop marks and not as a
        // thicker corner of the border.
        let m = inflate(sel, b + w + st.mark_gap(sel));
        for (x, y, dx, dy) in [(m.left, m.top, 1, 1), (m.right, m.top, -1, 1), (m.left, m.bottom, 1, -1), (m.right, m.bottom, -1, -1)] {
            let horizontal = RECT { left: x.min(x + dx * len), top: y.min(y + dy * w), right: x.max(x + dx * len), bottom: y.max(y + dy * w) };
            let vertical = RECT { left: x.min(x + dx * w), top: y.min(y + dy * len), right: x.max(x + dx * w), bottom: y.max(y + dy * len) };
            FillRect(dc, &horizontal, white);
            FillRect(dc, &vertical, white);
        }
        let _ = DeleteObject(HGDIOBJ(white.0));

        let (label, text) = st.label(sel);
        let dpi = st.dpi_at(POINT { x: sel.left, y: sel.top });
        draw_pill(dc, st, dpi, label, &text);
    }
}

unsafe fn draw_pill(dc: HDC, st: &mut State, dpi: u32, r: RECT, text: &[u16]) {
    // SAFETY: objects selected into `dc` are restored before deletion.
    unsafe {
        st.use_font(dpi);
        let brush: HBRUSH = CreateSolidBrush(COLORREF(0x001e_1c1c)); // #1c1c1e, the graphite ground (bgr)
        let old_brush = SelectObject(dc, HGDIOBJ(brush.0));
        let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        let radius = State::px(dpi, 6);
        let _ = RoundRect(dc, r.left, r.top, r.right + 1, r.bottom + 1, radius, radius);
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));

        let old_font = SelectObject(dc, HGDIOBJ(st.font.0));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00f2_f2f2));
        let mut text = text.to_vec();
        let mut r = r;
        DrawTextW(dc, &mut text, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old_font);
    }
}

fn point(lparam: LPARAM) -> POINT {
    // signed 16-bit halves: client coordinates can be negative under capture.
    POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 }
}

/// rectangle between two points, inclusive of the pixel under the cursor,
/// clamped to the overlay.
fn normalize(a: POINT, b: POINT, width: i32, height: i32) -> RECT {
    let (x0, x1) = (a.x.min(b.x).clamp(0, width), (a.x.max(b.x) + 1).clamp(0, width));
    let (y0, y1) = (a.y.min(b.y).clamp(0, height), (a.y.max(b.y) + 1).clamp(0, height));
    RECT { left: x0, top: y0, right: x1, bottom: y1 }
}

fn inflate(r: RECT, by: i32) -> RECT {
    RECT { left: r.left - by, top: r.top - by, right: r.right + by, bottom: r.bottom + by }
}

fn offset(r: RECT, x: i32, y: i32) -> RECT {
    RECT { left: r.left + x, top: r.top + y, right: r.right + x, bottom: r.bottom + y }
}

fn union(a: RECT, b: RECT) -> RECT {
    RECT { left: a.left.min(b.left), top: a.top.min(b.top), right: a.right.max(b.right), bottom: a.bottom.max(b.bottom) }
}

fn intersect(a: RECT, b: RECT) -> Option<RECT> {
    let r = RECT { left: a.left.max(b.left), top: a.top.max(b.top), right: a.right.min(b.right), bottom: a.bottom.min(b.bottom) };
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_includes_cursor_pixel_and_clamps() {
        let r = normalize(POINT { x: 10, y: 20 }, POINT { x: 5, y: 25 }, 100, 100);
        assert_eq!((r.left, r.top, r.right, r.bottom), (5, 20, 11, 26));
        let r = normalize(POINT { x: 90, y: 90 }, POINT { x: 500, y: -50 }, 100, 100);
        assert_eq!((r.left, r.top, r.right, r.bottom), (90, 0, 100, 91));
    }
}
