//! screen capture via gdi `BitBlt` from the desktop dc.
//!
//! all coordinates are physical pixels in virtual-screen space (they can be
//! negative on multi-monitor setups). the process is per-monitor-v2 dpi aware
//! (see `kadr.manifest`), otherwise windows would hand us scaled coordinates.

use std::ffi::c_void;
use std::ptr::null_mut;

use windows::Win32::Foundation::{E_INVALIDARG, HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, HBITMAP,
    HDC, HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetWindowRect, IsIconic,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};
use windows::core::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn from_win(r: RECT) -> Self {
        Self { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top }
    }

    pub fn intersect(self, other: Rect) -> Option<Rect> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        (right > left && bottom > top).then(|| Rect { x: left, y: top, width: right - left, height: bottom - top })
    }
}

/// a captured image: a top-down 32-bit bgrx dib section.
///
/// the dib section is kept (instead of copying into a `Vec`) so the overlay
/// can blit it directly; it is freed on drop.
pub struct Frame {
    bitmap: HBITMAP,
    bits: *mut u8,
    pub width: i32,
    pub height: i32,
}

impl Frame {
    /// allocates an uninitialized (zeroed by the os) frame.
    pub fn new(width: i32, height: i32) -> Result<Self> {
        if width <= 0 || height <= 0 {
            return Err(Error::new(E_INVALIDARG, "empty image"));
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // negative = top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = null_mut();
        // SAFETY: `info` is a valid header, `bits` receives the buffer pointer.
        let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)? };
        Ok(Self { bitmap, bits: bits.cast(), width, height })
    }

    pub fn bitmap(&self) -> HBITMAP {
        self.bitmap
    }

    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }

    pub fn byte_len(&self) -> usize {
        self.stride() * self.height as usize
    }

    /// pixel bytes, bgrx order, top-down rows. the x byte is not alpha.
    pub fn pixels(&self) -> &[u8] {
        // SAFETY: `bits` points to the dib section's buffer of exactly
        // byte_len() bytes, alive until the bitmap is deleted in drop. gdi
        // only writes to it inside `grab`, which holds `&mut self`.
        unsafe { std::slice::from_raw_parts(self.bits, self.byte_len()) }
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        // SAFETY: as above; `&mut self` guarantees exclusive access.
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.byte_len()) }
    }

    /// copies `rect` (in this frame's own coordinates) into a new frame.
    pub fn crop(&self, rect: Rect) -> Result<Frame> {
        let bounds = Rect { x: 0, y: 0, width: self.width, height: self.height };
        let rect = rect.intersect(bounds).ok_or_else(|| Error::new(E_INVALIDARG, "selection is outside the image"))?;
        let mut out = Frame::new(rect.width, rect.height)?;
        let src_stride = self.stride();
        let row_bytes = out.stride();
        let x_bytes = rect.x as usize * 4;
        let src = self.pixels();
        for (row, dst) in out.pixels_mut().chunks_exact_mut(row_bytes).enumerate() {
            let start = (rect.y as usize + row) * src_stride + x_bytes;
            dst.copy_from_slice(&src[start..start + row_bytes]);
        }
        Ok(out)
    }

    /// a darkened copy, used as the overlay background.
    pub fn dimmed(&self, keep: u8) -> Result<Frame> {
        let mut out = Frame::new(self.width, self.height)?;
        let keep = keep as u16;
        for (d, s) in out.pixels_mut().iter_mut().zip(self.pixels()) {
            *d = ((*s as u16 * keep) >> 8) as u8;
        }
        Ok(out)
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: we own the bitmap; callers deselect it from any dc first.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
        }
    }
}

/// bounding rectangle of all monitors.
pub fn virtual_screen() -> Rect {
    // SAFETY: plain metric queries, no pointers.
    unsafe {
        Rect {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

pub fn cursor_pos() -> Result<POINT> {
    let mut point = POINT::default();
    // SAFETY: out-pointer to a local.
    unsafe { GetCursorPos(&mut point)? };
    Ok(point)
}

/// full rectangle of the monitor that contains `point` (or the nearest one).
pub fn monitor_at(point: POINT) -> Result<Rect> {
    // SAFETY: `cbSize` is set as required; out-pointer to a local.
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return Err(Error::from_thread());
        }
        Ok(Rect::from_win(info.rcMonitor))
    }
}

pub fn monitor_at_cursor() -> Result<Rect> {
    monitor_at(cursor_pos()?)
}

/// visible bounds of the foreground window, clipped to the screen.
///
/// returns `None` for the desktop, the taskbar, a minimized window, or a
/// window that is fully off screen; the caller decides what to do then.
pub fn foreground_window_rect() -> Option<Rect> {
    // SAFETY: every call takes the hwnd returned by the os and out-pointers
    // to locals of the documented size.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() || IsIconic(hwnd).as_bool() || is_shell_window(hwnd) {
            return None;
        }
        // extended frame bounds exclude the invisible resize borders that
        // GetWindowRect includes on windows 10/11.
        let mut r = RECT::default();
        let dwm = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut r as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        );
        if dwm.is_err() {
            GetWindowRect(hwnd, &mut r).ok()?;
        }
        Rect::from_win(r).intersect(virtual_screen())
    }
}

fn is_shell_window(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    // SAFETY: buffer length is passed implicitly by the slice.
    let len = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    let name = String::from_utf16_lossy(&buf[..len]);
    matches!(name.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

/// test-only (KADR_TEST_FRAME=1): instead of the screen, a synthetic
/// pattern that depends only on absolute screen coordinates, so tests can
/// check exported pixels exactly without ever reading the user's screen.
pub fn synthetic_pixel(x: i32, y: i32) -> [u8; 3] {
    let check = (((x >> 4) + (y >> 4)) & 1) as u8 * 120;
    [(x as u8) ^ check, (y as u8).wrapping_mul(3), check.wrapping_add((x / 7) as u8)]
}

fn synthetic(rect: Rect) -> Result<Frame> {
    let mut frame = Frame::new(rect.width, rect.height)?;
    let w = rect.width as usize;
    for (i, px) in frame.pixels_mut().chunks_exact_mut(4).enumerate() {
        let p = synthetic_pixel(rect.x + (i % w) as i32, rect.y + (i / w) as i32);
        px[..3].copy_from_slice(&p);
    }
    Ok(frame)
}

/// copies `rect` of the screen into a new frame.
pub fn grab(rect: Rect) -> Result<Frame> {
    if std::env::var_os("KADR_TEST_FRAME").is_some() {
        return synthetic(rect);
    }
    let frame = Frame::new(rect.width, rect.height)?;
    let screen = ScreenDc::get()?;
    let memory = MemoryDc::compatible_with(screen.0)?;
    let selected = memory.select(frame.bitmap());
    // SAFETY: both dcs are valid for this scope.
    let blit = unsafe {
        BitBlt(
            memory.0,
            0,
            0,
            rect.width,
            rect.height,
            Some(screen.0),
            rect.x,
            rect.y,
            // CAPTUREBLT includes layered (translucent) windows.
            // measured: no speed difference with or without it (docs/measurements.md).
            SRCCOPY | CAPTUREBLT,
        )
    };
    drop(selected);
    blit?;
    Ok(frame)
}

/// dc for the whole virtual screen, released on drop.
pub struct ScreenDc(pub HDC);

impl ScreenDc {
    pub fn get() -> Result<Self> {
        // SAFETY: `None` requests the screen dc.
        let dc = unsafe { GetDC(None) };
        if dc.is_invalid() {
            return Err(Error::from_thread());
        }
        Ok(Self(dc))
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: obtained from GetDC(None).
        unsafe {
            ReleaseDC(None, self.0);
        }
    }
}

/// memory dc, deleted on drop.
pub struct MemoryDc(pub HDC);

impl MemoryDc {
    pub fn compatible_with(dc: HDC) -> Result<Self> {
        // SAFETY: `dc` is a valid dc owned by the caller.
        let memory = unsafe { CreateCompatibleDC(Some(dc)) };
        if memory.is_invalid() {
            return Err(Error::from_thread());
        }
        Ok(Self(memory))
    }

    /// selects a bitmap until the guard is dropped.
    pub fn select(&self, bitmap: HBITMAP) -> Selection<'_> {
        // SAFETY: valid dc and bitmap; the previous object is restored on drop.
        let previous = unsafe { SelectObject(self.0, HGDIOBJ(bitmap.0)) };
        Selection { dc: self, previous }
    }
}

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: created by CreateCompatibleDC and owned by us.
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

/// restores a dc's previous object; the borrow keeps it from outliving the dc.
pub struct Selection<'a> {
    dc: &'a MemoryDc,
    previous: HGDIOBJ,
}

impl Drop for Selection<'_> {
    fn drop(&mut self) {
        // SAFETY: restores the object that was selected before.
        unsafe {
            SelectObject(self.dc.0, self.previous);
        }
    }
}
