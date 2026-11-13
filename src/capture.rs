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

/// the program of the foreground window: its exe name without `.exe`
/// (`claude`, `materialgram`). `None` for the desktop and the taskbar.
pub fn foreground_app() -> Option<String> {
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    // SAFETY: the hwnd comes from the os; the process handle is closed here;
    // the buffer length is passed with it.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() || is_shell_window(hwnd) {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = windows::Win32::Foundation::CloseHandle(process);
        ok.ok()?;
        let path = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
        path.file_stem().map(|s| s.to_string_lossy().into_owned())
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

/// a mouse pointer as premultiplied bgra pixels, top-down, with its hotspot.
pub struct CursorImage {
    pub width: i32,
    pub height: i32,
    pub hot_x: i32,
    pub hot_y: i32,
    pub pixels: Vec<u8>,
}

/// the pointer as it was when a capture started: its image and where its
/// hotspot was on the screen.
pub struct CursorShot {
    pub x: i32,
    pub y: i32,
    pub image: CursorImage,
}

/// the visible mouse pointer right now; `None` when it is hidden (or in the
/// synthetic test mode, which never reads the real screen).
pub fn grab_cursor() -> Option<CursorShot> {
    use windows::Win32::UI::WindowsAndMessaging::{CURSOR_SHOWING, CURSORINFO, GetCursorInfo};
    if std::env::var_os("KADR_TEST_FRAME").is_some() {
        return None;
    }
    let mut ci = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
    // SAFETY: an out-pointer of the documented size.
    unsafe { GetCursorInfo(&mut ci) }.ok()?;
    if ci.flags.0 & CURSOR_SHOWING.0 == 0 || ci.hCursor.is_invalid() {
        return None;
    }
    let image = cursor_image(ci.hCursor)?;
    Some(CursorShot { x: ci.ptScreenPos.x, y: ci.ptScreenPos.y, image })
}

/// draws `cursor` at its own size once on black and once on white; the two
/// give its alpha (white − black = what shows through) and its premultiplied
/// colour (the black copy). inverting pixels (the text i-beam) have no colour
/// of their own: they become black with a white edge, visible on any page.
pub fn cursor_image(cursor: windows::Win32::UI::WindowsAndMessaging::HCURSOR) -> Option<CursorImage> {
    use windows::Win32::Graphics::Gdi::{BITMAP, GdiFlush, GetObjectW};
    use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DrawIconEx, GetIconInfo, HICON, ICONINFO};
    let icon = HICON(cursor.0);
    let mut info = ICONINFO::default();
    // SAFETY: the bitmaps GetIconInfo makes are ours and deleted here; the
    // BITMAP out-pointer has the size passed with it.
    let (w, h) = unsafe {
        GetIconInfo(icon, &mut info).ok()?;
        let mut bm = BITMAP::default();
        let size = if !info.hbmColor.is_invalid() {
            GetObjectW(HGDIOBJ(info.hbmColor.0), size_of::<BITMAP>() as i32, Some((&mut bm as *mut BITMAP).cast()));
            (bm.bmWidth, bm.bmHeight)
        } else {
            // monochrome: the mask holds the and and the xor halves
            GetObjectW(HGDIOBJ(info.hbmMask.0), size_of::<BITMAP>() as i32, Some((&mut bm as *mut BITMAP).cast()));
            (bm.bmWidth, bm.bmHeight / 2)
        };
        for b in [info.hbmColor, info.hbmMask] {
            if !b.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(b.0));
            }
        }
        size
    };
    if w <= 0 || h <= 0 || w > 512 || h > 512 {
        return None;
    }
    let black = Frame::new(w, h).ok()?;
    let mut white = Frame::new(w, h).ok()?;
    white.pixels_mut().fill(255);
    {
        let screen = ScreenDc::get().ok()?;
        for f in [&black, &white] {
            let mem = MemoryDc::compatible_with(screen.0).ok()?;
            let _sel = mem.select(f.bitmap());
            // SAFETY: a valid memory dc with the frame selected.
            unsafe {
                let _ = DrawIconEx(mem.0, 0, 0, icon, w, h, 0, None, DI_NORMAL);
                let _ = GdiFlush();
            }
        }
    }
    let (uw, uh) = (w as usize, h as usize);
    let mut pixels = vec![0u8; uw * uh * 4];
    let mut inverted = vec![false; uw * uh];
    let stride = black.stride();
    for y in 0..uh {
        for x in 0..uw {
            let i = y * stride + x * 4;
            let (b, wh) = (&black.pixels()[i..i + 3], &white.pixels()[i..i + 3]);
            let o = (y * uw + x) * 4;
            // brighter on black than on white: an inverting pixel
            if (b[1] as i32) - (wh[1] as i32) > 128 {
                inverted[y * uw + x] = true;
                pixels[o + 3] = 255;
                continue;
            }
            let diff = (wh[1] as i32 - b[1] as i32).max(wh[0] as i32 - b[0] as i32).max(wh[2] as i32 - b[2] as i32);
            let alpha = (255 - diff).clamp(0, 255) as u8;
            pixels[o] = b[0].min(alpha);
            pixels[o + 1] = b[1].min(alpha);
            pixels[o + 2] = b[2].min(alpha);
            pixels[o + 3] = alpha;
        }
    }
    // the white edge around inverting pixels
    for y in 0..uh {
        for x in 0..uw {
            let o = (y * uw + x) * 4;
            if inverted[y * uw + x] || pixels[o + 3] != 0 {
                continue;
            }
            let near = (y.saturating_sub(1)..(y + 2).min(uh)).any(|ny| (x.saturating_sub(1)..(x + 2).min(uw)).any(|nx| inverted[ny * uw + nx]));
            if near {
                pixels[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    Some(CursorImage { width: w, height: h, hot_x: info.xHotspot as i32, hot_y: info.yHotspot as i32, pixels })
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
