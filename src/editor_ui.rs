//! the editor process: the image window, the toolbar, the tools and export.
//!
//! the captured pixels arrive in a shared-memory section (editor.rs); they are
//! copied once into `source`, which is never modified, and the section is
//! released at once. annotations live in annotate.rs as shapes in physical
//! image pixels; the window shows the image 1:1, so a mouse position is an
//! image position whatever the dpi.
//!
//! drawing: software direct2d into a dc render target, in tiles of at most
//! TILE × TILE pixels and only over the part that changed. each tile copies
//! its piece of the source into one reusable tile bitmap and draws the shapes
//! that touch it. no full copy of the image is made per mouse move, and the
//! render target's own surface never grows past a tile. export draws the
//! same way into one new frame (or, without annotations, is the source).
//!
//! blur is direct2d's gaussian blur over the source pixels under the area
//! (computed per tile from the source, so it follows moves at once). it is
//! not a secure way to hide text; redact is an opaque fill.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::Instant;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_RECT_U, D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_COPY,
    D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES, D2D1_BITMAP_PROPERTIES1, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID,
    D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_GAUSSIANBLUR_PROP_BORDER_MODE,
    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LINE_JOIN_ROUND,
    D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE,
    D2D1_STROKE_STYLE_PROPERTIES, D2D1CreateFactory, ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1DeviceContext, ID2D1Effect,
    ID2D1Factory1, ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DWMWCP_ROUND, DwmFlush, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GdiFlush, GetMonitorInfoW, HDC, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromRect, MonitorFromWindow, PAINTSTRUCT, ScreenToClient, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{FILE_MAP_READ, MapViewOfFile, UnmapViewOfFile};
use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows::Win32::System::Threading::{GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources, SetEvent};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_BACK, VK_DELETE, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ACCEL, ACCEL_VIRT_FLAGS, CreateAcceleratorTableW, CreateWindowExW, DefWindowProcW, DestroyAcceleratorTable,
    DestroyWindow, DispatchMessageW, FCONTROL, FSHIFT, FVIRTKEY, GWLP_USERDATA, GetCursorPos, GetForegroundWindow,
    GetMessageW, GetWindowLongPtrW, GetWindowRect, HCURSOR, HTCAPTION,
    HTCLIENT, ICON_BIG, ICON_SMALL, IDC_ARROW, IDC_CROSS, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENWSE, IDCANCEL, IDYES,
    KillTimer, LoadCursorW, MA_NOACTIVATE, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_YESNOCANCEL, MSG,
    MessageBoxW, PostQuitMessage, RegisterClassW, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOSIZE, SendMessageW, SetCursor,
    RemovePropW, SetForegroundWindow, SetPropW, SetTimer, SetWindowLongPtrW, ShowWindow, TranslateAcceleratorW, TranslateMessage, WINDOWPOS,
    WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_ENDSESSION, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_ACTIVATEAPP, WM_CAPTURECHANGED, WM_MOVING, WM_NCCALCSIZE, WM_NCCREATE, WM_NCHITTEST, WM_PAINT, WM_QUERYENDSESSION, WM_SETCURSOR, WM_SETICON,
    WM_TIMER, WM_WINDOWPOSCHANGING, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, WS_THICKFRAME,
};
use windows::core::{Interface, PCWSTR, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

use crate::annotate::{Area, Doc, MAX_WIDTH, MIN_WIDTH, PRESETS, Pt, Shape, Slot, Style, Styles, Tool, hex_of, push_point, rgb_of};
use crate::capture::{Frame, MemoryDc, Rect, ScreenDc};
use crate::editor::Args;
use crate::settings::Settings;
use crate::ui::{self, Canvas, Fonts, Painter, Theme};
use crate::{clipboard, storage, trace, wide};

const CLASS: PCWSTR = w!("kadr.editor");
const BAR_CLASS: PCWSTR = w!("kadr.editor.bar");
const STYLE_CLASS: PCWSTR = w!("kadr.editor.style");
const DIM_CLASS: PCWSTR = w!("kadr.editor.dim");
const FRAME_CLASS: PCWSTR = w!("kadr.editor.frame");
const TOAST_CLASS: PCWSTR = w!("kadr.editor.toast");
/// how long the "saved" / "copied" note stays after the editor closes.
const TOAST_MS: u32 = 1200;
const TIMER_TOAST: usize = 2;
/// opacity of the black veil behind the editor: the same darkening as the
/// overlay's dimmed copy (keep 140/256 of the brightness), so the hand-off
/// from selecting to editing does not change the screen.
const DIM_ALPHA: u8 = 116;
/// ask "save your changes?" before closing with unsaved annotations. off:
/// closing never asks and never holds up sign-out.
const ASK_BEFORE_CLOSE: bool = false;
/// commands, from the accelerators, the toolbar, or a test posting them.
pub const ID_COPY: u16 = 1;
pub const ID_SAVE: u16 = 2;
pub const ID_CLOSE: u16 = 3;
pub const ID_UNDO: u16 = 10;
pub const ID_REDO: u16 = 11;
pub const ID_DELETE: u16 = 12;
pub const ID_ESCAPE: u16 = 13;
pub const ID_STYLE: u16 = 14;
pub const ID_PICK: u16 = 15;
pub const ID_CUSTOM: u16 = 16;
pub const ID_WIDTH_DOWN: u16 = 17;
pub const ID_WIDTH_UP: u16 = 18;
/// ID_TOOL + index in Tool::ALL
pub const ID_TOOL: u16 = 100;
/// ID_PRESET + index in PRESETS
pub const ID_PRESET: u16 = 300;
/// ID_WIDTH + width in pixels (1..=32)
pub const ID_WIDTH: u16 = 400;
/// sets the colour of the current slot (and selected shape): wparam = 0xrrggbb.
/// what the native colour dialog ends in; tests post it directly.
pub const WM_SET_COLOR: u32 = 0x8000 + 5;
const WM_MOUSEWHEEL: u32 = 0x020A;
const TIMER_STATUS: usize = 1;
const WM_MOUSELEAVE: u32 = 0x02A3;
const STATUS_MS: u32 = 2500;
const MK_LBUTTON: usize = 0x0001;
const MK_SHIFT: usize = 0x0004;
const TILE: i32 = 512;
/// blur strength in image pixels: enough to make text unreadable to the eye.
const BLUR_SIGMA: f32 = 10.0;
/// how close (image pixels) a click must be to select a shape or a handle.
const HIT_TOL: f32 = 6.0;
const HANDLE_TOL: f32 = 9.0;

/// the parts of the style panel.
#[derive(Clone, Copy, PartialEq)]
enum Fly {
    Preset(usize),
    Custom,
    Pick,
    Track,
}

/// direct2d resources of the image window and the export.
struct Gfx {
    factory: ID2D1Factory1,
    target: ID2D1DCRenderTarget,
    dc: ID2D1DeviceContext,
    brush: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
    tile: ID2D1Bitmap,
    blur: ID2D1Effect,
    /// the mouse pointer image of the pointer tool (premultiplied); none if
    /// it could not be made, so the editor still paints without it.
    pointer: Option<ID2D1Bitmap>,
}

impl Gfx {
    fn new(pointer: &crate::capture::CursorImage) -> Result<Gfx> {
        // SAFETY: plain resource creation.
        unsafe {
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            };
            let target = factory.CreateDCRenderTarget(&props)?;
            let dc: ID2D1DeviceContext = target.cast()?;
            let brush = target.CreateSolidColorBrush(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }, None)?;
            let base: windows::Win32::Graphics::Direct2D::ID2D1Factory = factory.cast()?;
            let round = base.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?;
            let fmt = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE };
            let tile = target.CreateBitmap(D2D_SIZE_U { width: TILE as u32, height: TILE as u32 }, None, 0, &D2D1_BITMAP_PROPERTIES { pixelFormat: fmt, dpiX: 96.0, dpiY: 96.0 })?;
            let blur = dc.CreateEffect(&CLSID_D2D1GaussianBlur)?;
            blur.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &BLUR_SIGMA.to_le_bytes())?;
            blur.SetValue(D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32, D2D1_PROPERTY_TYPE_ENUM, &1u32.to_le_bytes())?; // hard edge
            let pfmt = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED };
            let pointer = target.CreateBitmap(
                D2D_SIZE_U { width: pointer.width as u32, height: pointer.height as u32 },
                Some(pointer.pixels.as_ptr().cast()),
                pointer.width as u32 * 4,
                &D2D1_BITMAP_PROPERTIES { pixelFormat: pfmt, dpiX: 96.0, dpiY: 96.0 },
            )
            .ok();
            Ok(Gfx { factory, target, dc, brush, round, tile, blur, pointer })
        }
    }

    fn color(&self, c: [f32; 4]) -> &ID2D1SolidColorBrush {
        // SAFETY: our own brush.
        unsafe { self.brush.SetColor(&D2D1_COLOR_F { r: c[0], g: c[1], b: c[2], a: c[3] }) };
        &self.brush
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Tool(Tool),
    Style,
    Undo,
    Redo,
    Copy,
    Save,
    Close,
}

#[derive(Clone)]
enum Drag {
    None,
    Draw { start: Pt },
    Move { id: u64, start: Pt, before: Shape },
    Resize { id: u64, handle: usize, before: Shape },
    /// the frame: `mode` = FRAME_* bits of the edges taken (FRAME_MOVE: the
    /// whole frame), the cursor's screen position at the start, and the
    /// frame (x, y, w, h in capture coordinates) before.
    Frame { mode: u8, start: (i32, i32), before: [i32; 4] },
}

const FRAME_L: u8 = 1;
const FRAME_T: u8 = 2;
const FRAME_R: u8 = 4;
const FRAME_B: u8 = 8;
const FRAME_MOVE: u8 = 16;
/// the smallest the frame can be made, in pixels.
const MIN_CROP: i32 = 8;
/// how near (pixels, scaled) to the frame's edge a press grabs the edge.
const EDGE_TOL: f32 = 7.0;

struct Editor {
    hwnd: Cell<HWND>,
    bar: Cell<HWND>,
    /// the area of the screen the capture covers; `source` is that whole area,
    /// frozen. shapes are in the coordinates of `source`.
    screen: Rect,
    /// the frame, on the screen: the part of the capture shown and exported.
    /// the user can move and resize it (select tool); it never leaves `screen`.
    at: Cell<Rect>,
    /// set while the editor itself resizes its window (the system's own
    /// resizing and snapping are refused).
    resizing: Cell<bool>,
    /// a press on empty space (it moves the frame) went past the drag
    /// distance; until then it is still a click.
    frame_armed: Cell<bool>,
    /// the captured pixels of the whole `screen`, never modified.
    source: Frame,
    gfx: RefCell<Option<Gfx>>,
    doc: RefCell<Doc>,
    tool: Cell<Tool>,
    /// the shape being drawn, not yet in the document.
    drawing: RefCell<Option<(Shape, Style)>>,
    drag: RefCell<Drag>,
    selected: Cell<Option<u64>>,
    canvas: Canvas,
    fonts: RefCell<Option<Fonts>>,
    theme: Theme,
    dpi: Cell<u32>,
    settings_path: Option<PathBuf>,
    private_clipboard: bool,
    status: RefCell<Option<(String, bool)>>,
    hover: Cell<Option<Button>>,
    pressed: Cell<Option<Button>>,
    /// document states already on disk, and where. saving one of them again
    /// (also after undo or redo back to it) writes nothing; copying is not
    /// saving. bounded: the oldest are forgotten.
    saved: RefCell<Vec<(u64, PathBuf)>>,
    /// paint statistics, for the trace at close: (count, total ms, max ms)
    paints: Cell<(u32, f64, f64)>,
    closing: Cell<bool>,
    /// the current style of each slot, remembered in editor.ini.
    styles: RefCell<Styles>,
    styles_path: Option<PathBuf>,
    /// the style panel (created on first use) and its state.
    flyout: Cell<HWND>,
    fly_hover: Cell<Option<Fly>>,
    /// the eyedropper is waiting for a click on the image; the colour under
    /// the pointer meanwhile.
    pick: Cell<bool>,
    pick_color: Cell<Option<u32>>,
    /// a width drag on the panel: the selected shape and its style before.
    slider: Cell<Option<(Option<u64>, Style)>>,
    custom: RefCell<[windows::Win32::Foundation::COLORREF; 16]>,
    /// the veil that darkens the rest of the screen while this editor is
    /// the active app; it owns the image window, so it stays right below it.
    dim: Cell<HWND>,
    /// the white border and crop marks around the image, the same frame the
    /// overlay drew around the selection: a window of their shape just
    /// outside the image, so they are never part of the pixels.
    frame: Cell<HWND>,
    /// the note shown for a moment after copy or save closed the editor.
    toast: Cell<HWND>,
    toast_text: RefCell<String>,
    /// the pointer tool's image: the pointer at capture time, or the standard
    /// arrow when it was hidden; and where its hotspot was in the image.
    pointer: crate::capture::CursorImage,
    pointer_at: Option<Pt>,
}

pub fn run(args: Args, settings_path: Option<PathBuf>, private_clipboard: bool) -> i32 {
    let at = args.at;
    let screen = args.screen;
    let size = screen.width as usize * screen.height as usize * 4;
    let pointer_size = args.cursor.map_or(0, |c| (c[2] * c[3] * 4) as usize);
    let section = HANDLE(args.section as *mut _);
    let ready = HANDLE(args.ready as *mut _);
    // SAFETY: handles inherited from kadr, named on our command line.
    let view = unsafe { MapViewOfFile(section, FILE_MAP_READ, 0, 0, size + pointer_size) };
    if view.Value.is_null() {
        crate::app::error_box(None, "kadr could not open the capture in the editor.");
        // SAFETY: as above; the event lets kadr stop waiting at once.
        unsafe {
            let _ = SetEvent(ready);
            let _ = CloseHandle(ready);
            let _ = CloseHandle(section);
        }
        return 1;
    }
    // one private copy, then the shared section is released at once.
    let source = Frame::new(screen.width, screen.height).map(|mut f| {
        // SAFETY: the view holds exactly `size` bytes, as kadr wrote them.
        f.pixels_mut().copy_from_slice(unsafe { std::slice::from_raw_parts(view.Value.cast::<u8>(), size) });
        f
    });
    // the pointer kadr saw, after the frame; else the standard arrow
    let captured = args.cursor.map(|c| {
        // SAFETY: kadr wrote `pointer_size` bytes after the frame.
        let pixels = unsafe { std::slice::from_raw_parts(view.Value.cast::<u8>().add(size), pointer_size) }.to_vec();
        (crate::capture::CursorImage { width: c[2], height: c[3], hot_x: c[4], hot_y: c[5], pixels }, Pt { x: c[0] as f32, y: c[1] as f32 })
    });
    // SAFETY: our own view and handle, unused from here on.
    unsafe {
        let _ = UnmapViewOfFile(view);
        let _ = CloseHandle(section);
    }
    // the last colours and widths, next to settings.ini
    let styles_path = settings_path.clone().or_else(|| storage::settings_path().ok()).and_then(|p| p.parent().map(|d| d.join("editor.ini")));
    let styles = styles_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Styles::parse(&t)).unwrap_or_default();
    trace!(
        "editor {}: styles stroke #{:06x} {}, highlighter #{:06x} {}, redact #{:06x}",
        std::process::id(),
        hex_of(styles.stroke.color),
        styles.stroke.width,
        hex_of(styles.highlight.color),
        styles.highlight.width,
        hex_of(styles.redact.color)
    );
    let (pointer, pointer_at) = match captured {
        Some((image, at)) => (image, Some(at)),
        None => {
            // SAFETY: a shared system cursor, not freed.
            let arrow = unsafe { LoadCursorW(None, IDC_ARROW) }.ok().and_then(crate::capture::cursor_image);
            (arrow.unwrap_or(crate::capture::CursorImage { width: 1, height: 1, hot_x: 0, hot_y: 0, pixels: vec![0; 4] }), None)
        }
    };
    trace!("editor {}: pointer {}x{}, {}", std::process::id(), pointer.width, pointer.height, if pointer_at.is_some() { "captured" } else { "standard arrow" });
    let result = (|| -> Result<i32> {
        let editor = Box::new(Editor {
            hwnd: Cell::new(HWND::default()),
            bar: Cell::new(HWND::default()),
            screen,
            at: Cell::new(at),
            resizing: Cell::new(false),
            frame_armed: Cell::new(false),
            source: source?,
            gfx: RefCell::new(None),
            doc: RefCell::new(Doc::with_crop([at.x - screen.x, at.y - screen.y, at.width, at.height])),
            tool: Cell::new(Tool::Rect),
            drawing: RefCell::new(None),
            drag: RefCell::new(Drag::None),
            selected: Cell::new(None),
            canvas: Canvas::new()?,
            fonts: RefCell::new(None),
            theme: Theme::current(),
            dpi: Cell::new(96),
            settings_path,
            private_clipboard,
            status: RefCell::new(None),
            hover: Cell::new(None),
            pressed: Cell::new(None),
            saved: RefCell::new(Vec::new()),
            paints: Cell::new((0, 0.0, 0.0)),
            closing: Cell::new(false),
            styles: RefCell::new(styles),
            styles_path,
            flyout: Cell::new(HWND::default()),
            fly_hover: Cell::new(None),
            pick: Cell::new(false),
            pick_color: Cell::new(None),
            slider: Cell::new(None),
            custom: RefCell::new([windows::Win32::Foundation::COLORREF(0xffffff); 16]),
            dim: Cell::new(HWND::default()),
            frame: Cell::new(HWND::default()),
            toast: Cell::new(HWND::default()),
            toast_text: RefCell::new(String::new()),
            pointer,
            pointer_at,
        });
        let editor = Box::into_raw(editor);
        // SAFETY: `editor` outlives both windows: it is reclaimed only after
        // the message loop ends, when both have been destroyed.
        let code = unsafe { show_and_loop(&*editor, ready) };
        // SAFETY: as above.
        let ed = unsafe { Box::from_raw(editor) };
        let (n, total, max) = ed.paints.get();
        let doc = ed.doc.borrow();
        trace!(
            "editor {}: paints {n}, avg {:.2} ms, max {:.2} ms; shapes {}, history {}, ~{} kb",
            std::process::id(),
            if n > 0 { total / n as f64 } else { 0.0 },
            max,
            doc.items.len(),
            doc.history_len(),
            doc.approx_bytes() / 1024
        );
        drop(doc);
        drop(ed);
        code
    })();
    // SAFETY: our own event handle; a process query.
    let (gdi, user) = unsafe {
        let _ = SetEvent(ready); // in case the window never showed
        let _ = CloseHandle(ready);
        (GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS), GetGuiResources(GetCurrentProcess(), GR_USEROBJECTS))
    };
    trace!("editor {}: closed, gdi {gdi}, user {user}, memory {}", std::process::id(), crate::memory_summary());
    match result {
        Ok(code) => code,
        Err(e) => {
            crate::app::error_box(None, &format!("the editor could not start: {}", e.message()));
            1
        }
    }
}

unsafe fn show_and_loop(ed: &Editor, ready: HANDLE) -> Result<i32> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let arrow = LoadCursorW(None, IDC_ARROW).ok();
        for (class, proc_, cursor) in [
            (CLASS, image_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT, None),
            (BAR_CLASS, bar_proc, arrow),
            (STYLE_CLASS, style_proc, arrow),
            (DIM_CLASS, dim_proc, arrow),
            (FRAME_CLASS, frame_proc, arrow),
            (TOAST_CLASS, toast_proc, arrow),
        ] {
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc_), hInstance: instance.into(), lpszClassName: class, hCursor: cursor.unwrap_or_default(), ..Default::default() });
        }
        let at = ed.at.get();
        let title = wide(&format!("kadr · {} × {}", at.width, at.height));
        // a popup with a thick frame and no non-client area: windows 11 gives
        // it the system border and shadow, while the client is exactly the
        // captured pixels at the place they were captured.
        // the veil: a solid black layered window over the whole virtual screen,
        // no pixels of ours (dwm composes the colour); owner of the image
        // window, so the image, its toolbar and panels always sit above it.
        // it paints the frozen capture, darkened (not the live desktop, so
        // what shows when the frame is dragged outwards is what was captured).
        let vs = ed.screen;
        let dim = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            DIM_CLASS,
            w!("kadr"),
            WS_POPUP,
            vs.x,
            vs.y,
            vs.width,
            vs.height,
            None,
            None,
            Some(instance.into()),
            Some(ed as *const Editor as *const _),
        )?;
        let _ = DwmSetWindowAttribute(dim, DWMWA_TRANSITIONS_FORCEDISABLED, (&1i32 as *const i32).cast(), 4);
        ed.dim.set(dim);
        let hwnd = CreateWindowExW(
            // owned windows get no taskbar button by themselves: keep one
            windows::Win32::UI::WindowsAndMessaging::WS_EX_APPWINDOW,
            CLASS,
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_THICKFRAME,
            at.x,
            at.y,
            at.width,
            at.height,
            Some(dim),
            None,
            Some(instance.into()),
            Some(ed as *const Editor as *const _),
        )?;
        ed.hwnd.set(hwnd);
        ed.dpi.set(GetDpiForWindow(hwnd).max(96));
        *ed.fonts.borrow_mut() = Some(ui::make_fonts(&ed.canvas.dwrite, ed.dpi.get())?);
        let on: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, (&on as *const i32).cast(), 4);
        let square = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, (&square as *const DWM_WINDOW_CORNER_PREFERENCE).cast(), 4);
        // our frame is the border: no system hairline over the outer pixels
        // (windows 11; elsewhere the call just fails). the shadow stays.
        let none = windows::Win32::Foundation::COLORREF(0xFFFF_FFFE); // DWMWA_COLOR_NONE
        let _ = DwmSetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWA_BORDER_COLOR, (&none as *const windows::Win32::Foundation::COLORREF).cast(), 4);
        let frame = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            FRAME_CLASS,
            w!("kadr"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            Some(hwnd), // owned: above the veil, hidden and destroyed with the image
            None,
            Some(instance.into()),
            None,
        )?;
        let _ = DwmSetWindowAttribute(frame, DWMWA_TRANSITIONS_FORCEDISABLED, (&on as *const i32).cast(), 4);
        let _ = DwmSetWindowAttribute(frame, DWMWA_WINDOW_CORNER_PREFERENCE, (&square as *const DWM_WINDOW_CORNER_PREFERENCE).cast(), 4);
        ed.frame.set(frame);
        ed.place_frame();
        for (kind, size) in [(ICON_SMALL, 16), (ICON_BIG, 32)] {
            let icon = ui::app_icon(size * ed.dpi.get() as i32 / 96);
            SendMessageW(hwnd, WM_SETICON, Some(WPARAM(kind as usize)), Some(LPARAM(icon.0 as isize)));
        }

        let (bw, bh) = ed.bar_size();
        let (bx, by) = ed.bar_place(bw, bh);
        let bar = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            BAR_CLASS,
            w!("kadr editor tools"),
            WS_POPUP,
            bx,
            by,
            bw,
            bh,
            Some(hwnd), // owned: stays above the image, goes with it
            None,
            Some(instance.into()),
            Some(ed as *const Editor as *const _),
        )?;
        ed.bar.set(bar);
        let round = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(bar, DWMWA_WINDOW_CORNER_PREFERENCE, (&round as *const DWM_WINDOW_CORNER_PREFERENCE).cast(), 4);
        let _ = DwmSetWindowAttribute(bar, DWMWA_TRANSITIONS_FORCEDISABLED, (&on as *const i32).cast(), 4);

        let _ = ShowWindow(dim, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(dim); // painted before the overlay goes
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let _ = ShowWindow(frame, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(frame);
        let _ = ShowWindow(bar, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(bar);
        let _ = DwmFlush();
        let _ = SetEvent(ready);
        let _ = SetForegroundWindow(hwnd);
        let focused = GetForegroundWindow() == hwnd;
        trace!(
            "editor {}: shown {}x{} at {},{} {:.1} ms after process start, foreground {focused}, memory {}",
            std::process::id(),
            at.width,
            at.height,
            at.x,
            at.y,
            crate::ms_since_process_start(),
            crate::memory_summary()
        );

        let key = |flags: ACCEL_VIRT_FLAGS, key: u16, cmd: u16| ACCEL { fVirt: flags | FVIRTKEY, key, cmd };
        let mut keys = vec![
            key(FCONTROL, b'C' as u16, ID_COPY),
            key(FCONTROL, b'S' as u16, ID_SAVE),
            key(FCONTROL, b'Z' as u16, ID_UNDO),
            key(FCONTROL, b'Y' as u16, ID_REDO),
            key(FCONTROL | FSHIFT, b'Z' as u16, ID_REDO),
            key(Default::default(), VK_DELETE.0, ID_DELETE),
            key(Default::default(), VK_BACK.0, ID_DELETE),
            key(Default::default(), VK_ESCAPE.0, ID_ESCAPE),
        ];
        for (i, k) in [b'V', b'P', b'L', b'A', b'R', b'E', b'H', b'B', b'X', b'M'].into_iter().enumerate() {
            keys.push(key(Default::default(), k as u16, ID_TOOL + i as u16));
        }
        // 1..7: the preset colours; [ and ]: width; i: the eyedropper
        for i in 0..PRESETS.len() {
            keys.push(key(Default::default(), b'1' as u16 + i as u16, ID_PRESET + i as u16));
        }
        keys.push(key(Default::default(), 0xDB, ID_WIDTH_DOWN));
        keys.push(key(Default::default(), 0xDD, ID_WIDTH_UP));
        keys.push(key(Default::default(), b'I' as u16, ID_PICK));
        let accel = CreateAcceleratorTableW(&keys)?;
        let mut msg = MSG::default();
        let code = loop {
            match GetMessageW(&mut msg, None, 0, 0).0 {
                0 => break msg.wParam.0 as i32,
                -1 => break 1,
                _ => {
                    if TranslateAcceleratorW(hwnd, accel, &msg) == 0 {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
        };
        let _ = DestroyAcceleratorTable(accel);
        Ok(code)
    }
}

fn rect_of(a: Area) -> RECT {
    RECT { left: a.l.floor() as i32, top: a.t.floor() as i32, right: a.r.ceil() as i32, bottom: a.b.ceil() as i32 }
}

fn rf(a: Area) -> D2D_RECT_F {
    D2D_RECT_F { left: a.l, top: a.t, right: a.r, bottom: a.b }
}

fn v(p: Pt) -> Vector2 {
    Vector2 { X: p.x, Y: p.y }
}

impl Editor {
    fn pxi(&self, v: i32) -> i32 {
        (v as f32 * self.dpi.get() as f32 / 96.0).round() as i32
    }
    fn px(&self, v: f32) -> f32 {
        v * self.dpi.get() as f32 / 96.0
    }

    /// where the frame is in the capture's own coordinates (x, y, w, h).
    fn crop(&self) -> [i32; 4] {
        let a = self.at.get();
        [a.x - self.screen.x, a.y - self.screen.y, a.width, a.height]
    }

    /// the top-left of the frame in the capture: what to add to a point of
    /// the image window to get a point of the capture.
    fn origin(&self) -> (i32, i32) {
        let c = self.crop();
        (c[0], c[1])
    }

    /// the frame as an area, in capture coordinates (where shapes live).
    fn image_area(&self) -> Area {
        let c = self.crop();
        Area { l: c[0] as f32, t: c[1] as f32, r: (c[0] + c[2]) as f32, b: (c[1] + c[3]) as f32 }
    }

    /// the whole captured screen as an area, in capture coordinates.
    fn frame_area(&self) -> Area {
        Area { l: 0.0, t: 0.0, r: self.screen.width as f32, b: self.screen.height as f32 }
    }

    fn pid() -> u32 {
        std::process::id()
    }

    /// changes that are not on disk. the untouched capture (state 0) is
    /// never dirty, saved or not.
    fn dirty(&self) -> bool {
        let state = self.doc.borrow().state();
        state != 0 && !self.saved.borrow().iter().any(|(s, _)| *s == state)
    }

    /// puts the frame around the image where it is now, shaped for the dpi
    /// of the monitor under its top-left corner (as the overlay measured it).
    fn place_frame(&self) {
        let frame = self.frame.get();
        if frame.is_invalid() {
            return;
        }
        let mut img = RECT::default();
        // SAFETY: queries and changes of our own windows; the region is owned
        // by the window once set, and freed here otherwise.
        unsafe {
            let _ = GetWindowRect(self.hwnd.get(), &mut img);
            let dpi = crate::overlay::monitor_dpi(POINT { x: img.left, y: img.top });
            let reach = crate::overlay::frame_reach(dpi);
            let (w, h) = (img.right - img.left, img.bottom - img.top);
            let sel = RECT { left: reach, top: reach, right: reach + w, bottom: reach + h };
            use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, HGDIOBJ, RGN_OR, SetWindowRgn};
            let region = CreateRectRgn(0, 0, 0, 0);
            for r in crate::overlay::frame_rects(sel, dpi) {
                let part = CreateRectRgn(r.left, r.top, r.right, r.bottom);
                let _ = CombineRgn(Some(region), Some(region), Some(part), RGN_OR);
                let _ = DeleteObject(HGDIOBJ(part.0));
            }
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                frame,
                None,
                img.left - reach,
                img.top - reach,
                w + 2 * reach,
                h + 2 * reach,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
            );
            if SetWindowRgn(frame, Some(region), true) == 0 {
                let _ = DeleteObject(HGDIOBJ(region.0));
            }
        }
    }

    fn work_area(&self) -> RECT {
        // SAFETY: plain monitor query for our window.
        unsafe {
            let m = MonitorFromWindow(self.hwnd.get(), MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(m, &mut info);
            info.rcWork
        }
    }

    fn invalidate(&self, a: Area) {
        // selection marks and handles reach 14 px outside a shape; `a` is in
        // capture coordinates, the window's are the frame's
        let (ox, oy) = self.origin();
        let r = rect_of(a.inflate(16.0));
        let r = RECT { left: r.left - ox, top: r.top - oy, right: r.right - ox, bottom: r.bottom - oy };
        // SAFETY: our own window.
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd.get()), Some(&r), false);
        }
    }

    // ---- rendering ----------------------------------------------------------

    /// draws `area` (image coordinates = dc coordinates) into `hdc`, tile by
    /// tile. `marks`: also the selection marks (screen only, never exported).
    ///
    /// a tile no shape touches is a plain gdi copy of the source: measured,
    /// a full 2560×1600 paint through direct2d took ~30 ms against ~3 ms, and
    /// direct2d itself (32 ms to create its software target) is only set up
    /// once the first shape needs it.
    fn render(&self, hdc: HDC, area: RECT, marks: bool) -> Result<()> {
        let mut slot = self.gfx.borrow_mut();
        // the source in a memory dc, for the plain tiles (a dc and a
        // selection: microseconds, no pixels are copied)
        let source_dc = MemoryDc::compatible_with(hdc)?;
        let _source_sel = source_dc.select(self.source.bitmap());
        // `area` is in the window's coordinates, shapes and the source in the
        // capture's: the frame's origin is the difference
        let (ox, oy) = self.origin();
        let at = self.at.get();
        let area = RECT {
            left: area.left.max(0),
            top: area.top.max(0),
            right: area.right.min(at.width),
            bottom: area.bottom.min(at.height),
        };
        let doc = self.doc.borrow();
        let drawing = self.drawing.borrow();
        let mut y = area.top;
        let mut lost = false;
        while y < area.bottom {
            let mut x = area.left;
            let h = (area.bottom - y).min(TILE);
            while x < area.right {
                let w = (area.right - x).min(TILE);
                let tile = RECT { left: x, top: y, right: x + w, bottom: y + h };
                let clip = Area { l: (x + ox) as f32, t: (y + oy) as f32, r: (x + w + ox) as f32, b: (y + h + oy) as f32 };
                let selected = if marks { self.selected.get().and_then(|id| doc.find(id)) } else { None };
                let touched = doc.items.iter().any(|i| i.shape.extent(&i.style).intersects(clip))
                    || drawing.as_ref().is_some_and(|(s, st)| s.extent(st).intersects(clip))
                    || selected.is_some_and(|i| i.shape.extent(&i.style).inflate(16.0).intersects(clip));
                if !touched {
                    // SAFETY: both dcs are valid for this call.
                    unsafe {
                        let _ = windows::Win32::Graphics::Gdi::BitBlt(hdc, x, y, w, h, Some(source_dc.0), x + ox, y + oy, windows::Win32::Graphics::Gdi::SRCCOPY);
                    }
                    x += w;
                    continue;
                }
                if slot.is_none() {
                    let t = Instant::now();
                    *slot = Some(Gfx::new(&self.pointer)?);
                    trace!("editor {}: direct2d ready in {:.1} ms", Self::pid(), t.elapsed().as_secs_f64() * 1000.0);
                }
                let g = slot.as_ref().unwrap();
                // SAFETY: direct2d drawing between BeginDraw and EndDraw on our own
                // target, bound to a dc that is valid for this call.
                unsafe {
                    g.target.BindDC(hdc, &tile)?;
                    g.target.BeginDraw();
                    g.target.SetTransform(&Matrix3x2::translation(-((x + ox) as f32), -((y + oy) as f32)));
                    let stride = self.source.stride();
                    let src = self.source.pixels().as_ptr().add((y + oy) as usize * stride + (x + ox) as usize * 4);
                    let _ = g.tile.CopyFromMemory(Some(&D2D_RECT_U { left: 0, top: 0, right: w as u32, bottom: h as u32 }), src.cast(), stride as u32);
                    g.target.DrawBitmap(
                        &g.tile,
                        Some(&rf(clip)),
                        1.0,
                        D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                        Some(&D2D_RECT_F { left: 0.0, top: 0.0, right: w as f32, bottom: h as f32 }),
                    );
                    for item in &doc.items {
                        if item.shape.extent(&item.style).intersects(clip) {
                            self.draw_shape(g, &item.shape, &item.style, clip);
                        }
                    }
                    if let Some((shape, style)) = drawing.as_ref() {
                        self.draw_shape(g, shape, style, clip);
                    }
                    if let Some(item) = selected {
                        self.draw_marks(g, &item.shape, &item.style);
                    }
                    if g.target.EndDraw(None, None).is_err() {
                        lost = true;
                    }
                }
                x += w;
            }
            y += h;
        }
        drop(slot);
        if lost {
            *self.gfx.borrow_mut() = None; // rebuilt on the next paint
        }
        Ok(())
    }

    fn draw_shape(&self, g: &Gfx, shape: &Shape, style: &Style, clip: Area) {
        // SAFETY: called between BeginDraw and EndDraw.
        unsafe {
            match shape {
                Shape::Pen(points) | Shape::Highlight(points) => self.draw_stroke(g, points, style, clip),
                Shape::Line(a, b) => g.target.DrawLine(v(*a), v(*b), g.color(style.color), style.width, &g.round),
                Shape::Arrow(a, b) => self.draw_arrow(g, *a, *b, style),
                Shape::Cursor(r) => {
                    if let Some(b) = &g.pointer {
                        g.target.DrawBitmap(b, Some(&rf(*r)), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None);
                    }
                }
                Shape::Rect(r) => g.target.DrawRectangle(&rf(*r), g.color(style.color), style.width, None),
                Shape::Ellipse(r) => {
                    let e = D2D1_ELLIPSE { point: Vector2 { X: (r.l + r.r) / 2.0, Y: (r.t + r.b) / 2.0 }, radiusX: r.width() / 2.0, radiusY: r.height() / 2.0 };
                    g.target.DrawEllipse(&e, g.color(style.color), style.width, None);
                }
                Shape::Redact(r) | Shape::HighlightRect(r) => g.target.FillRectangle(&rf(*r), g.color(style.color)),
                Shape::Blur(r) => self.draw_blur(g, *r, clip),
            }
        }
    }

    /// the shaft up into the head (so its round cap stays inside), then the
    /// head as one filled triangle.
    unsafe fn draw_arrow(&self, g: &Gfx, a: Pt, b: Pt, style: &Style) {
        let (tip, l, r, base) = crate::annotate::arrow_head(a, b, style.width);
        let into = Pt { x: (base.x * 2.0 + tip.x) / 3.0, y: (base.y * 2.0 + tip.y) / 3.0 };
        // SAFETY: called between BeginDraw and EndDraw.
        unsafe {
            g.target.DrawLine(v(a), v(into), g.color(style.color), style.width, &g.round);
            let Ok(path) = g.factory.CreatePathGeometry() else { return };
            let Ok(sink) = path.Open() else { return };
            sink.BeginFigure(v(tip), windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLines(&[v(l), v(r)]);
            sink.EndFigure(windows::Win32::Graphics::Direct2D::Common::D2D1_FIGURE_END_CLOSED);
            if sink.Close().is_ok() {
                g.target.FillGeometry(&path, g.color(style.color), None);
            }
        }
    }

    /// a freehand stroke, built only from the segments near `clip`, as one
    /// geometry, so a translucent highlighter does not darken its own joints.
    unsafe fn draw_stroke(&self, g: &Gfx, points: &[Pt], style: &Style, clip: Area) {
        unsafe {
            let near = clip.inflate(style.width / 2.0 + 1.0);
            if points.len() == 1 {
                let e = D2D1_ELLIPSE { point: v(points[0]), radiusX: style.width / 2.0, radiusY: style.width / 2.0 };
                g.target.FillEllipse(&e, g.color(style.color));
                return;
            }
            let Ok(path) = g.factory.CreatePathGeometry() else { return };
            let Ok(sink) = path.Open() else { return };
            let mut run: Vec<Vector2> = Vec::new();
            let flush = |run: &mut Vec<Vector2>| {
                if run.len() >= 2 {
                    sink.BeginFigure(run[0], D2D1_FIGURE_BEGIN_HOLLOW);
                    sink.AddLines(&run[1..]);
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                }
                run.clear();
            };
            for s in points.windows(2) {
                if Area::from_points(s[0], s[1]).intersects(near) {
                    if run.is_empty() {
                        run.push(v(s[0]));
                    }
                    run.push(v(s[1]));
                } else {
                    flush(&mut run);
                }
            }
            flush(&mut run);
            if sink.Close().is_ok() {
                g.target.DrawGeometry(&path, g.color(style.color), style.width, &g.round);
            }
        }
    }

    /// gaussian blur of the source under `area`, for the part inside `clip`.
    /// the input reaches 3σ past the area so the blur is the same in every
    /// tile; at the image edge the edge pixels repeat (hard border).
    unsafe fn draw_blur(&self, g: &Gfx, area: Area, clip: Area) {
        let part = Area { l: area.l.max(clip.l), t: area.t.max(clip.t), r: area.r.min(clip.r), b: area.b.min(clip.b) };
        if part.width() <= 0.0 || part.height() <= 0.0 {
            return;
        }
        let img = self.frame_area();
        let reach = part.inflate(3.0 * BLUR_SIGMA);
        let input = rect_of(Area { l: reach.l.max(img.l), t: reach.t.max(img.t), r: reach.r.min(img.r), b: reach.b.min(img.b) });
        let (w, h) = (input.right - input.left, input.bottom - input.top);
        if w <= 0 || h <= 0 {
            return;
        }
        let stride = self.source.stride();
        let fmt = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE };
        // SAFETY: the pointer and pitch describe the `input` part of the
        // source, which lives as long as self; drawing is inside BeginDraw.
        unsafe {
            let src = self.source.pixels().as_ptr().add(input.top as usize * stride + input.left as usize * 4);
            let props = D2D1_BITMAP_PROPERTIES1 { pixelFormat: fmt, dpiX: 96.0, dpiY: 96.0, bitmapOptions: D2D1_BITMAP_OPTIONS_NONE, colorContext: std::mem::ManuallyDrop::new(None) };
            let Ok(bitmap) = g.dc.CreateBitmap(D2D_SIZE_U { width: w as u32, height: h as u32 }, Some(src.cast()), stride as u32, &props) else { return };
            g.blur.SetInput(0, &bitmap, true);
            let Ok(out) = g.blur.GetOutput() else { return };
            g.target.PushAxisAlignedClip(&rf(part), D2D1_ANTIALIAS_MODE_ALIASED);
            g.dc.DrawImage(&out, Some(&Vector2 { X: input.left as f32, Y: input.top as f32 }), None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_COPY);
            g.target.PopAxisAlignedClip();
            g.blur.SetInput(0, None::<&windows::Win32::Graphics::Direct2D::ID2D1Image>, false);
        }
    }

    /// crop marks around the selected shape (DESIGN.md's signature), white
    /// with a dark edge so they hold on any picture, and square handles.
    unsafe fn draw_marks(&self, g: &Gfx, shape: &Shape, style: &Style) {
        let r = shape.bounds().inflate(style.width / 2.0 + 6.0);
        let arm = 12.0f32.min(r.width() / 2.0).min(r.height() / 2.0).max(4.0);
        let corners = [(r.l, r.t, 1.0, 1.0), (r.r, r.t, -1.0, 1.0), (r.l, r.b, 1.0, -1.0), (r.r, r.b, -1.0, -1.0)];
        // SAFETY: called between BeginDraw and EndDraw.
        unsafe {
            for (color, width) in [([0.0, 0.0, 0.0, 0.55], 4.0), ([1.0, 1.0, 1.0, 1.0], 2.0)] {
                for (x, y, dx, dy) in corners {
                    let c = Vector2 { X: x, Y: y };
                    g.target.DrawLine(c, Vector2 { X: x + dx * arm, Y: y }, g.color(color), width, None);
                    g.target.DrawLine(c, Vector2 { X: x, Y: y + dy * arm }, g.color(color), width, None);
                }
            }
            for h in shape.handles() {
                let s = D2D_RECT_F { left: h.x - 4.0, top: h.y - 4.0, right: h.x + 4.0, bottom: h.y + 4.0 };
                g.target.FillRectangle(&s, g.color([1.0, 1.0, 1.0, 1.0]));
                g.target.DrawRectangle(&s, g.color([0.0, 0.0, 0.0, 0.7]), 1.0, None);
            }
        }
    }

    fn paint_image(&self, hdc: HDC, r: RECT) {
        let t = Instant::now();
        if let Err(e) = self.render(hdc, r, true) {
            trace!("editor {}: paint failed: {}", Self::pid(), e.message());
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let (n, total, max) = self.paints.get();
        self.paints.set((n + 1, total + ms, max.max(ms)));
    }

    /// the image to export, given to `f`: the source itself when nothing was
    /// drawn, otherwise one new frame with every shape, freed afterwards.
    fn with_export<T>(&self, f: impl FnOnce(&Frame) -> std::result::Result<T, String>) -> std::result::Result<T, String> {
        match self.compose().map_err(|e| e.message())? {
            Some(out) => f(&out),
            None => {
                let c = self.crop();
                let part = self.source.crop(crate::capture::Rect { x: c[0], y: c[1], width: c[2], height: c[3] }).map_err(|e| e.message())?;
                f(&part)
            }
        }
    }

    fn compose(&self) -> Result<Option<Frame>> {
        if self.doc.borrow().items.is_empty() {
            return Ok(None);
        }
        let t = Instant::now();
        let at = self.at.get();
        let out = Frame::new(at.width, at.height)?;
        {
            let screen = ScreenDc::get()?;
            let mem = MemoryDc::compatible_with(screen.0)?;
            let _sel = mem.select(out.bitmap());
            self.render(mem.0, RECT { left: 0, top: 0, right: at.width, bottom: at.height }, false)?;
            // SAFETY: flushes gdi's batch before the bits are read.
            unsafe {
                let _ = GdiFlush();
            }
        }
        trace!("editor {}: composed {}x{} with {} shape(s) in {:.1} ms", Self::pid(), at.width, at.height, self.doc.borrow().items.len(), t.elapsed().as_secs_f64() * 1000.0);
        Ok(Some(out))
    }

    // ---- commands -------------------------------------------------------------

    fn command(&self, id: u16) {
        match id {
            ID_COPY => self.copy(),
            ID_SAVE => {
                if self.save() && self.settings().close_after_save {
                    let state = self.doc.borrow().state();
                    let name = self.saved.borrow().iter().rev().find(|(s, _)| *s == state).map(|(_, p)| file_name(p));
                    trace!("editor {}: closing after save", Self::pid());
                    self.close_with_note(&match name {
                        Some(n) => format!("saved · {n}"),
                        None => "saved".to_string(),
                    });
                }
            }
            ID_CLOSE => self.request_close(),
            ID_UNDO => self.undo(false),
            ID_REDO => self.undo(true),
            ID_DELETE => self.delete_selected(),
            ID_ESCAPE => self.escape(),
            ID_STYLE => self.toggle_flyout(),
            ID_PICK => self.start_pick(),
            ID_CUSTOM => self.custom_color(),
            ID_WIDTH_DOWN | ID_WIDTH_UP => self.step_width(if id == ID_WIDTH_UP { 1.0 } else { -1.0 }),
            c if (ID_PRESET..ID_PRESET + PRESETS.len() as u16).contains(&c) => self.set_color(PRESETS[(c - ID_PRESET) as usize]),
            c if (ID_WIDTH + 1..=ID_WIDTH + MAX_WIDTH as u16).contains(&c) => self.set_width((c - ID_WIDTH) as f32, false),
            t if (ID_TOOL..ID_TOOL + Tool::ALL.len() as u16).contains(&t) => self.set_tool(Tool::ALL[(t - ID_TOOL) as usize]),
            _ => {}
        }
    }

    fn set_tool(&self, tool: Tool) {
        if self.dragging() {
            return;
        }
        // the highlighter button or h again: stroke <-> translucent rectangle
        if tool == Tool::Highlighter && self.tool.get() == Tool::Highlighter {
            let rect = {
                let mut styles = self.styles.borrow_mut();
                styles.highlight_rect = !styles.highlight_rect;
                styles.highlight_rect
            };
            self.save_styles();
            trace!("editor {}: highlighter mode {}", Self::pid(), if rect { "rectangle" } else { "stroke" });
            self.redraw_bar();
            self.refresh_flyout();
            return;
        }
        self.tool.set(tool);
        self.select(None);
        trace!("editor {}: tool {}", Self::pid(), tool.name());
        // the pointer tool shows the pointer where it was, if that is in the
        // image and none is placed yet; clicks then move it
        if tool == Tool::Cursor
            && self.pointer_shape().is_none()
            && let Some(at) = self.pointer_at
            && self.image_area().contains(at)
        {
            let shape = self.pointer_at_point(at);
            let style = self.styles.borrow().for_tool(tool);
            self.invalidate(shape.extent(&style));
            let id = self.doc.borrow_mut().add(shape, style);
            trace!("editor {}: add cursor {id} at {:.0},{:.0} -> state {}", Self::pid(), at.x, at.y, self.doc.borrow().state());
        }
        self.redraw_bar();
        self.refresh_flyout();
    }

    /// the pointer shape with its hotspot at `p`.
    fn pointer_at_point(&self, p: Pt) -> Shape {
        let i = &self.pointer;
        let (l, t) = (p.x - i.hot_x as f32, p.y - i.hot_y as f32);
        Shape::Cursor(Area { l, t, r: l + i.width as f32, b: t + i.height as f32 })
    }

    /// the placed pointer, if any: (id, its hotspot now).
    fn pointer_shape(&self) -> Option<(u64, Pt)> {
        let doc = self.doc.borrow();
        doc.items.iter().rev().find_map(|i| match i.shape {
            Shape::Cursor(r) => Some((i.id, Pt { x: r.l + self.pointer.hot_x as f32, y: r.t + self.pointer.hot_y as f32 })),
            _ => None,
        })
    }

    fn select(&self, id: Option<u64>) {
        let doc = self.doc.borrow();
        for old in [self.selected.get(), id].into_iter().flatten() {
            if let Some(i) = doc.find(old) {
                self.invalidate(i.shape.extent(&i.style));
            }
        }
        drop(doc);
        self.selected.set(id);
        self.redraw_bar();
        self.refresh_flyout();
    }

    fn dragging(&self) -> bool {
        !matches!(*self.drag.borrow(), Drag::None)
    }

    fn undo(&self, redo: bool) {
        if self.dragging() {
            return;
        }
        let area = if redo { self.doc.borrow_mut().redo() } else { self.doc.borrow_mut().undo() };
        if let Some(a) = area {
            trace!("editor {}: {} -> state {}", Self::pid(), if redo { "redo" } else { "undo" }, self.doc.borrow().state());
            if self.selected.get().is_some_and(|id| self.doc.borrow().find(id).is_none()) {
                self.selected.set(None);
            }
            self.invalidate(a);
            self.sync_crop();
            self.select(self.selected.get());
            self.redraw_bar();
        }
    }

    fn delete_selected(&self) {
        let Some(id) = self.selected.get() else { return };
        if self.dragging() {
            return;
        }
        self.select(None);
        if let Some(a) = self.doc.borrow_mut().remove(id) {
            trace!("editor {}: remove {id}", Self::pid());
            self.invalidate(a);
        }
        self.redraw_bar();
    }

    /// esc: stop what is being drawn or dragged, then drop the selection,
    /// then close.
    fn escape(&self) {
        if self.pick.get() {
            self.end_pick();
            return;
        }
        if self.flyout_visible() && !self.dragging() {
            self.hide_flyout();
            return;
        }
        let drag = self.drag.replace(Drag::None);
        match drag {
            Drag::Draw { .. } => {
                if let Some((s, st)) = self.drawing.borrow_mut().take() {
                    self.invalidate(s.extent(&st));
                }
                // SAFETY: plain call.
                unsafe {
                    let _ = ReleaseCapture();
                }
            }
            Drag::Move { id, before, .. } | Drag::Resize { id, before, .. } => {
                let area = self.doc.borrow().find(id).map(|i| i.shape.extent(&i.style).union(before.extent(&i.style)));
                self.doc.borrow_mut().preview(id, before);
                if let Some(a) = area {
                    self.invalidate(a);
                }
                // SAFETY: plain call.
                unsafe {
                    let _ = ReleaseCapture();
                }
            }
            Drag::Frame { before, .. } => {
                self.apply_crop(before);
                self.relayout();
                // SAFETY: plain call.
                unsafe {
                    let _ = ReleaseCapture();
                }
            }
            Drag::None => {
                if self.selected.get().is_some() {
                    self.select(None);
                } else {
                    self.request_close();
                }
            }
        }
    }

    fn copy(&self) {
        let t = Instant::now();
        let result = self.with_export(|f| clipboard::copy(self.hwnd.get(), f, self.private_clipboard).map_err(|e| e.message()));
        match result {
            Ok(()) => {
                trace!("editor {}: copied {}x{} in {:.1} ms", Self::pid(), self.at.get().width, self.at.get().height, t.elapsed().as_secs_f64() * 1000.0);
                if self.settings().close_after_copy {
                    // the result is on the clipboard: no question about saving
                    trace!("editor {}: closing after copy", Self::pid());
                    self.close_with_note("copied");
                    return;
                }
                self.set_status("copied", false);
            }
            Err(e) => self.fail(&format!("could not copy to the clipboard: {e}")),
        }
    }

    /// the settings file as it is now (a change in settings applies at once).
    fn settings(&self) -> Settings {
        match &self.settings_path {
            Some(p) => Settings::load(p).0,
            None => Settings::default(),
        }
    }

    /// hides the editor and shows `text` in a small graphite note where the
    /// image was; the editor closes when the note goes (TOAST_MS, or a click).
    fn close_with_note(&self, text: &str) {
        if !self.toast.get().is_invalid() {
            return;
        }
        *self.toast_text.borrow_mut() = text.to_string();
        let (w, h) = {
            let fonts = self.fonts.borrow();
            let tw = fonts.as_ref().map_or(200.0, |f| self.canvas.measure(text, &f.body));
            (tw.ceil() as i32 + self.pxi(32), self.pxi(36))
        };
        let work = self.work_area();
        let at = self.at.get();
        let x = (at.x + (at.width - w) / 2).min(work.right - w).max(work.left);
        let y = (at.y + (at.height - h) / 2).min(work.bottom - h).max(work.top);
        // SAFETY: our own windows; the note gets the editor pointer like the
        // toolbar and lives until the loop ends.
        unsafe {
            for win in [self.flyout.get(), self.bar.get(), self.frame.get(), self.dim.get(), self.hwnd.get()] {
                if !win.is_invalid() {
                    let _ = ShowWindow(win, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
                }
            }
            let Ok(instance) = GetModuleHandleW(None) else {
                let _ = DestroyWindow(self.hwnd.get());
                return;
            };
            let toast = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | windows::Win32::UI::WindowsAndMessaging::WS_EX_TOPMOST,
                TOAST_CLASS,
                w!("kadr"),
                WS_POPUP,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(instance.into()),
                Some(self as *const Editor as *const _),
            );
            let Ok(toast) = toast else {
                let _ = DestroyWindow(self.hwnd.get());
                return;
            };
            self.toast.set(toast);
            let round = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(toast, DWMWA_WINDOW_CORNER_PREFERENCE, (&round as *const DWM_WINDOW_CORNER_PREFERENCE).cast(), 4);
            let on: i32 = 1;
            let _ = DwmSetWindowAttribute(toast, DWMWA_TRANSITIONS_FORCEDISABLED, (&on as *const i32).cast(), 4);
            let _ = ShowWindow(toast, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(toast);
            SetTimer(Some(toast), TIMER_TOAST, TOAST_MS, None);
        }
    }

    fn paint_toast(&self, hdc: HDC) {
        let mut r = RECT::default();
        // SAFETY: a query of our own window.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(self.toast.get(), &mut r);
        }
        let fonts = self.fonts.borrow();
        let Some(fonts) = fonts.as_ref() else { return };
        let text = self.toast_text.borrow();
        // the overlay's pill: graphite ground, ink text, whatever the theme
        self.canvas.paint(hdc, (r.right, r.bottom), (0, 0), |p| {
            p.fill(r, ui::rgb(0x1c1c1e));
            p.text_center(&text, &fonts.body, ui::frect(r), ui::rgb(0xf2f2f2));
        });
    }

    /// true when the current state is on disk (now or before).
    fn save(&self) -> bool {
        let state = self.doc.borrow().state();
        let already = self.saved.borrow().iter().find(|(s, _)| *s == state).map(|(_, p)| p.clone());
        if let Some(path) = already {
            trace!("editor {}: already saved, no new file", Self::pid());
            self.set_status(&format!("already saved · {}", file_name(&path)), false);
            return true;
        }
        let t = Instant::now();
        // read the file each time, so a folder changed in settings applies
        let settings = self.settings();
        // memory at each step, so the peak of a save can be attributed
        trace!("editor {}: save: before compose, memory {}", Self::pid(), crate::memory_summary());
        let result = self.with_export(|f| {
            trace!("editor {}: save: composed, memory {}", Self::pid(), crate::memory_summary());
            let png = crate::encode::png(f).map_err(|e| e.message())?;
            trace!("editor {}: save: encoded {} kb, memory {}", Self::pid(), png.len() / 1024, crate::memory_summary());
            storage::save_encoded(&png, &settings)
        });
        match result {
            Ok(path) => {
                trace!("editor {}: saved {} in {:.1} ms", Self::pid(), path.display(), t.elapsed().as_secs_f64() * 1000.0);
                self.set_status(&format!("saved · {}", file_name(&path)), false);
                let mut saved = self.saved.borrow_mut();
                if saved.len() >= 64 {
                    saved.remove(0);
                }
                saved.push((state, path));
                drop(saved);
                self.redraw_bar();
                true
            }
            Err(e) => {
                self.fail(&format!("could not save the capture: {e}"));
                false
            }
        }
    }

    /// closes at once, saved or not: the close button, esc, alt+f4 and an
    /// update of kadr never ask (the user's choice, 2026-11-12). the save
    /// question below stays only for `ASK_BEFORE_CLOSE`.
    fn request_close(&self) {
        if self.closing.get() {
            return;
        }
        if !ASK_BEFORE_CLOSE {
            if self.dirty() {
                trace!("editor {}: closing with unsaved changes, without asking", Self::pid());
            }
            // SAFETY: our own window.
            unsafe {
                let _ = DestroyWindow(self.hwnd.get());
            }
            return;
        }
        if self.dirty() {
            trace!("editor {}: unsaved changes, asking", Self::pid());
            let text = wide("save your changes before closing?\n\nyour annotations are not saved yet. copying does not save them.");
            self.closing.set(true);
            // SAFETY: valid strings; our own window as owner.
            // the toolbar is a separate window: keep it from acting under the question.
            // "kadr.asking" tells kadr's installer a question is pending; it is
            // removed only on cancel (install.rs, wait_for_editor).
            let answer = unsafe {
                let _ = SetPropW(self.hwnd.get(), w!("kadr.asking"), Some(HANDLE(1 as *mut _)));
                let _ = EnableWindow(self.bar.get(), false);
                let a = MessageBoxW(Some(self.hwnd.get()), PCWSTR(text.as_ptr()), w!("kadr"), MB_YESNOCANCEL | MB_ICONQUESTION | MB_SETFOREGROUND);
                let _ = EnableWindow(self.bar.get(), true);
                a
            };
            self.closing.set(false);
            if answer == IDCANCEL {
                // SAFETY: our own window.
                unsafe {
                    let _ = RemovePropW(self.hwnd.get(), w!("kadr.asking"));
                }
                trace!("editor {}: close cancelled", Self::pid());
                return;
            }
            if answer == IDYES && !self.save() {
                // SAFETY: our own window.
                unsafe {
                    let _ = RemovePropW(self.hwnd.get(), w!("kadr.asking"));
                }
                return;
            }
            trace!("editor {}: closing {}", Self::pid(), if answer == IDYES { "after saving" } else { "without saving" });
        }
        // SAFETY: our own window.
        unsafe {
            let _ = DestroyWindow(self.hwnd.get());
        }
    }

    fn fail(&self, text: &str) {
        trace!("editor {}: {text}", Self::pid());
        self.set_status("not done · see the message", true);
        let t = wide(text);
        // SAFETY: valid strings; our own window as owner.
        unsafe {
            let _ = EnableWindow(self.bar.get(), false);
            MessageBoxW(Some(self.hwnd.get()), PCWSTR(t.as_ptr()), w!("kadr"), MB_OK | MB_ICONWARNING);
            let _ = EnableWindow(self.bar.get(), true);
        }
    }

    fn set_status(&self, text: &str, error: bool) {
        *self.status.borrow_mut() = Some((text.to_string(), error));
        // SAFETY: our own toolbar window.
        unsafe {
            let _ = InvalidateRect(Some(self.bar.get()), None, false);
            SetTimer(Some(self.bar.get()), TIMER_STATUS, STATUS_MS, None);
        }
    }

    // ---- mouse on the image ---------------------------------------------------

    fn point(&self, lparam: LPARAM) -> Pt {
        let x = (lparam.0 & 0xffff) as i16 as f32;
        let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
        // clamped to the frame: nothing is drawn outside it. the result is in
        // the capture's coordinates, like the shapes
        let (ox, oy) = self.origin();
        let at = self.at.get();
        Pt { x: x.clamp(0.0, at.width as f32) + ox as f32, y: y.clamp(0.0, at.height as f32) + oy as f32 }
    }

    // ---- the frame ------------------------------------------------------------

    /// the FRAME_* edges within reach of `p` (capture coordinates), inside the
    /// frame or just outside it; 0 when `p` is not at an edge. where two
    /// opposite edges are both in reach (a very small frame) the nearer wins.
    fn edge_at(&self, p: Pt) -> u8 {
        let a = self.image_area();
        let tol = self.px(EDGE_TOL);
        if p.x < a.l - tol || p.x > a.r + tol || p.y < a.t - tol || p.y > a.b + tol {
            return 0;
        }
        let near = |v: f32, lo: f32, hi: f32, low: u8, high: u8| {
            let (dl, dh) = ((v - lo).abs(), (v - hi).abs());
            if dl <= tol && dl <= dh {
                low
            } else if dh <= tol {
                high
            } else {
                0
            }
        };
        near(p.x, a.l, a.r, FRAME_L, FRAME_R) | near(p.y, a.t, a.b, FRAME_T, FRAME_B)
    }

    fn edge_cursor(mode: u8) -> windows::core::PCWSTR {
        use windows::Win32::UI::WindowsAndMessaging::{IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE};
        match mode {
            FRAME_L | FRAME_R => IDC_SIZEWE,
            FRAME_T | FRAME_B => IDC_SIZENS,
            m if m == FRAME_L | FRAME_T || m == FRAME_R | FRAME_B => IDC_SIZENWSE,
            m if m == FRAME_L | FRAME_B || m == FRAME_R | FRAME_T => IDC_SIZENESW,
            _ => IDC_SIZEALL,
        }
    }

    fn cursor_pos() -> (i32, i32) {
        let mut pt = POINT::default();
        // SAFETY: plain cursor query.
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        (pt.x, pt.y)
    }

    /// starts dragging the frame by `mode`; `owner` takes the mouse.
    fn start_frame_drag(&self, mode: u8, owner: HWND) {
        self.frame_armed.set(mode & FRAME_MOVE == 0);
        *self.drag.borrow_mut() = Drag::Frame { mode, start: Self::cursor_pos(), before: self.crop() };
        // SAFETY: one of our own windows.
        unsafe {
            SetCapture(owner);
        }
    }

    /// the frame while it is dragged: from where the drag began and where the
    /// cursor is now on the screen (the window moves under the cursor, so the
    /// window's own coordinates would feed back on themselves). it stays
    /// inside the capture and never gets smaller than MIN_CROP.
    fn drag_frame(&self, mode: u8, start: (i32, i32), before: [i32; 4]) {
        let (cx, cy) = Self::cursor_pos();
        let (dx, dy) = (cx - start.0, cy - start.1);
        let [bx, by, bw, bh] = before;
        let (sw, sh) = (self.screen.width, self.screen.height);
        let clamp = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi);
        // a press on empty space only moves the frame once the cursor has
        // gone a real drag distance: a click to deselect must not nudge it
        if mode & FRAME_MOVE != 0 && !self.frame_armed.get() {
            // SAFETY: plain metrics queries.
            let (mx, my) = unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXDRAG, SM_CYDRAG};
                (GetSystemMetrics(SM_CXDRAG).max(2), GetSystemMetrics(SM_CYDRAG).max(2))
            };
            if dx.abs() <= mx && dy.abs() <= my {
                return;
            }
            self.frame_armed.set(true);
        }
        let (mut l, mut t, mut r, mut b) = (bx, by, bx + bw, by + bh);
        if mode & FRAME_MOVE != 0 {
            l = clamp(bx + dx, 0, sw - bw);
            t = clamp(by + dy, 0, sh - bh);
            r = l + bw;
            b = t + bh;
        } else {
            if mode & FRAME_L != 0 {
                l = clamp(bx + dx, 0, r - MIN_CROP);
            }
            if mode & FRAME_R != 0 {
                r = clamp(bx + bw + dx, l + MIN_CROP, sw);
            }
            if mode & FRAME_T != 0 {
                t = clamp(by + dy, 0, b - MIN_CROP);
            }
            if mode & FRAME_B != 0 {
                b = clamp(by + bh + dy, t + MIN_CROP, sh);
            }
        }
        // whatever the clamps above gave, the frame stays inside the capture
        // (a read outside the source would be a crash)
        let l = l.max(0).min(sw - 1);
        let t = t.max(0).min(sh - 1);
        let r = r.max(l + 1).min(sw);
        let b = b.max(t + 1).min(sh);
        self.apply_crop([l, t, r - l, b - t]);
        self.redraw_bar(); // the size in the toolbar follows live
    }

    /// puts the image window on the frame `c` (x, y, w, h in capture
    /// coordinates). nothing is recorded: the callers do that.
    fn apply_crop(&self, c: [i32; 4]) {
        let at = Rect { x: self.screen.x + c[0], y: self.screen.y + c[1], width: c[2], height: c[3] };
        if at == self.at.get() {
            return;
        }
        self.at.set(at);
        self.resizing.set(true);
        // SAFETY: our own window. the content is anchored to the capture, not
        // to the window's corner, so the old bits are never copied over.
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOZORDER, SetWindowPos};
            let _ = SetWindowPos(self.hwnd.get(), None, at.x, at.y, at.width, at.height, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS);
            let _ = InvalidateRect(Some(self.hwnd.get()), None, false);
        }
        self.resizing.set(false);
        self.place_frame();
    }

    /// after the frame changed for good: the title, the toolbar and the panel
    /// follow it.
    fn relayout(&self) {
        let at = self.at.get();
        let title = wide(&format!("kadr · {} × {}", at.width, at.height));
        let (bw, bh) = self.bar_size();
        let (bx, by) = self.bar_place(bw, bh);
        // SAFETY: our own windows.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowTextW(self.hwnd.get(), PCWSTR(title.as_ptr()));
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                self.bar.get(),
                None,
                bx,
                by,
                bw,
                bh,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
            );
        }
        if self.flyout_visible() {
            self.place_flyout();
        }
        self.redraw_bar();
    }

    /// undo and redo can bring the frame back: the window follows the
    /// document.
    fn sync_crop(&self) {
        let want = self.doc.borrow().crop;
        if want != self.crop() {
            self.apply_crop(want);
            self.relayout();
        }
    }

    /// a press on the veil, outside the image: it takes the frame's edge if
    /// one is in reach (select tool only).
    fn veil_down(&self, veil: HWND) {
        if self.dragging() || self.pick.get() || self.tool.get() != Tool::Select {
            return;
        }
        let (cx, cy) = Self::cursor_pos();
        let mode = self.edge_at(Pt { x: (cx - self.screen.x) as f32, y: (cy - self.screen.y) as f32 });
        if mode != 0 {
            self.hide_flyout();
            self.start_frame_drag(mode, veil);
        }
    }

    fn veil_cursor(&self) -> HCURSOR {
        let mut id = IDC_ARROW;
        if self.tool.get() == Tool::Select && !self.pick.get() {
            let (cx, cy) = Self::cursor_pos();
            let mut mode = self.edge_at(Pt { x: (cx - self.screen.x) as f32, y: (cy - self.screen.y) as f32 });
            if let Drag::Frame { mode: held, .. } = *self.drag.borrow() {
                mode = held;
            }
            if mode != 0 {
                id = Self::edge_cursor(mode);
            }
        } else if self.pick.get() {
            id = IDC_CROSS;
        }
        // SAFETY: a system cursor.
        unsafe { LoadCursorW(None, id).unwrap_or_default() }
    }

    /// the capture dimmed, behind the editor: the same darkening the overlay
    /// had (DIM_ALPHA of black over it). `r`: the part to paint.
    fn paint_veil(&self, hdc: HDC, r: RECT) {
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let (Ok(from), Ok(black), Ok(one)) = (MemoryDc::compatible_with(hdc), MemoryDc::compatible_with(hdc), Frame::new(1, 1)) else { return };
        let _a = from.select(self.source.bitmap());
        let _b = black.select(one.bitmap()); // a new frame is all zero: black
        // SAFETY: valid dcs for this call.
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::BitBlt(hdc, r.left, r.top, w, h, Some(from.0), r.left, r.top, windows::Win32::Graphics::Gdi::SRCCOPY);
            let blend = windows::Win32::Graphics::Gdi::BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: DIM_ALPHA, AlphaFormat: 0 };
            let _ = windows::Win32::Graphics::Gdi::AlphaBlend(hdc, r.left, r.top, w, h, black.0, 0, 0, 1, 1, blend);
        }
    }

    fn handle_at(&self, p: Pt) -> Option<(u64, usize)> {
        let id = self.selected.get()?;
        let doc = self.doc.borrow();
        let item = doc.find(id)?;
        item.shape.handles().iter().position(|h| (h.x - p.x).abs() <= HANDLE_TOL && (h.y - p.y).abs() <= HANDLE_TOL).map(|i| (id, i))
    }

    fn mouse_down(&self, p: Pt, shift: bool) {
        if self.dragging() {
            return;
        }
        if self.pick.get() {
            let c = self.source_color(p);
            trace!("editor {}: picked #{c:06x} at {:.0},{:.0}", Self::pid(), p.x, p.y);
            self.end_pick();
            self.set_color(c);
            return;
        }
        self.hide_flyout();
        let tool = self.tool.get();
        if tool == Tool::Select {
            if let Some((id, handle)) = self.handle_at(p) {
                let before = self.doc.borrow().find(id).unwrap().shape.clone();
                *self.drag.borrow_mut() = Drag::Resize { id, handle, before };
            } else {
                let hit = self.doc.borrow().hit(p, HIT_TOL);
                self.select(hit);
                match hit {
                    Some(id) => {
                        let before = self.doc.borrow().find(id).unwrap().shape.clone();
                        *self.drag.borrow_mut() = Drag::Move { id, start: p, before };
                    }
                    None => {
                        // nothing there: the frame. an edge resizes it, the
                        // rest of the image moves it
                        let mode = match self.edge_at(p) {
                            0 => FRAME_MOVE,
                            m => m,
                        };
                        self.start_frame_drag(mode, self.hwnd.get());
                        return;
                    }
                }
            }
        } else if tool == Tool::Cursor {
            // one pointer per image: a click moves it there (or places it)
            self.select(None);
            match self.pointer_shape() {
                Some((id, hot)) => {
                    let before = self.doc.borrow().find(id).unwrap().shape.clone();
                    *self.drag.borrow_mut() = Drag::Move { id, start: hot, before };
                }
                None => {
                    let shape = self.pointer_at_point(p);
                    *self.drawing.borrow_mut() = Some((shape, self.styles.borrow().for_tool(tool)));
                    *self.drag.borrow_mut() = Drag::Draw { start: p };
                }
            }
            // SAFETY: our own window.
            unsafe {
                SetCapture(self.hwnd.get());
            }
            self.mouse_drag(p, shift);
            return;
        } else {
            self.select(None);
            let shape = match tool {
                Tool::Pen => Shape::Pen(vec![p]),
                Tool::Highlighter if self.styles.borrow().highlight_rect => Shape::HighlightRect(Area::from_points(p, p)),
                Tool::Highlighter => Shape::Highlight(vec![p]),
                t => Shape::drawn(t, p, p, shift).unwrap(),
            };
            *self.drawing.borrow_mut() = Some((shape, self.styles.borrow().for_tool(tool)));
            *self.drag.borrow_mut() = Drag::Draw { start: p };
        }
        // SAFETY: our own window.
        unsafe {
            SetCapture(self.hwnd.get());
        }
    }

    fn mouse_drag(&self, p: Pt, shift: bool) {
        let drag = self.drag.borrow().clone();
        match drag {
            Drag::None => {}
            Drag::Draw { start } => {
                let mut slot = self.drawing.borrow_mut();
                let Some((shape, style)) = slot.as_mut() else { return };
                match shape {
                    Shape::Pen(points) | Shape::Highlight(points) => {
                        let last = *points.last().unwrap();
                        if push_point(points, p) {
                            let seg = Area::from_points(last, p).inflate(style.width / 2.0 + 2.0);
                            drop(slot);
                            self.invalidate(seg);
                        }
                    }
                    Shape::Cursor(_) => {
                        let old = shape.extent(style);
                        *shape = self.pointer_at_point(p);
                        let new = shape.extent(style);
                        drop(slot);
                        self.invalidate(old.union(new));
                    }
                    _ => {
                        let old = shape.extent(style);
                        *shape = if matches!(shape, Shape::HighlightRect(_)) {
                            Shape::HighlightRect(crate::annotate::constrain_area(start, p, shift))
                        } else {
                            Shape::drawn(self.tool.get(), start, p, shift).unwrap()
                        };
                        let new = shape.extent(style);
                        drop(slot);
                        self.invalidate(old.union(new));
                    }
                }
            }
            Drag::Move { id, start, before } => self.preview(id, before.translated(p.x - start.x, p.y - start.y)),
            Drag::Resize { id, handle, before } => self.preview(id, before.with_handle(handle, p, shift)),
            Drag::Frame { mode, start, before } => self.drag_frame(mode, start, before),
        }
    }

    fn preview(&self, id: u64, shape: Shape) {
        let old = self.doc.borrow().find(id).map(|i| i.shape.extent(&i.style));
        self.doc.borrow_mut().preview(id, shape);
        let new = self.doc.borrow().find(id).map(|i| i.shape.extent(&i.style));
        if let (Some(a), Some(b)) = (old, new) {
            self.invalidate(a.union(b));
        }
    }

    fn mouse_up(&self, p: Pt, shift: bool) {
        if !self.dragging() {
            return;
        }
        // the release point ends the gesture (a move generated after
        // SetCapture can lag behind it)
        self.mouse_drag(p, shift);
        self.finish_drag();
    }

    /// records the gesture in progress as it stands now.
    fn finish_drag(&self) {
        let drag = self.drag.replace(Drag::None);
        if matches!(drag, Drag::None) {
            return;
        }
        // SAFETY: plain call.
        unsafe {
            let _ = ReleaseCapture();
        }
        match drag {
            Drag::Draw { .. } => {
                if let Some((shape, style)) = self.drawing.borrow_mut().take() {
                    self.invalidate(shape.extent(&style));
                    if !shape.is_degenerate() {
                        let b = shape.bounds();
                        let id = self.doc.borrow_mut().add(shape, style);
                        trace!("editor {}: add {} {} {:.0},{:.0}-{:.0},{:.0} -> state {}", Self::pid(), self.doc.borrow().find(id).unwrap().shape.kind(), id, b.l, b.t, b.r, b.b, self.doc.borrow().state());
                    }
                }
            }
            Drag::Move { id, before, .. } | Drag::Resize { id, before, .. } => {
                let after = self.doc.borrow().find(id).map(|i| i.shape.clone());
                if let Some(after) = after {
                    let b = after.bounds();
                    let changed = after != before;
                    self.doc.borrow_mut().change(id, before, after);
                    if changed {
                        trace!("editor {}: change {id} {:.0},{:.0}-{:.0},{:.0} -> state {}", Self::pid(), b.l, b.t, b.r, b.b, self.doc.borrow().state());
                    }
                }
            }
            Drag::Frame { before, .. } => {
                let after = self.crop();
                self.doc.borrow_mut().change_crop(before, after);
                if after != before {
                    trace!("editor {}: frame {:?} -> {:?}, state {}", Self::pid(), before, after, self.doc.borrow().state());
                }
                self.relayout();
            }
            Drag::None => {}
        }
        self.redraw_bar();
    }

    fn cursor(&self) -> HCURSOR {
        // a frame drag keeps the cursor of the edge it took
        let held = match *self.drag.borrow() {
            Drag::Frame { mode, .. } => Some(mode),
            _ => None,
        };
        let id = if let Some(mode) = held {
            Self::edge_cursor(mode)
        } else if self.pick.get() || self.tool.get() != Tool::Select {
            IDC_CROSS
        } else {
            let mut pt = POINT::default();
            // SAFETY: plain cursor queries for our window.
            unsafe {
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(self.hwnd.get(), &mut pt);
            }
            let (ox, oy) = self.origin();
            let p = Pt { x: (pt.x + ox) as f32, y: (pt.y + oy) as f32 };
            match self.handle_at(p) {
                Some((_, 0 | 2)) => IDC_SIZENWSE,
                Some((_, 1 | 3)) if self.doc.borrow().find(self.selected.get().unwrap()).is_some_and(|i| !matches!(i.shape, Shape::Line(..) | Shape::Arrow(..))) => IDC_SIZENESW,
                Some(_) => IDC_SIZEALL,
                None if self.doc.borrow().hit(p, HIT_TOL).is_some() => IDC_SIZEALL,
                // nothing there: the frame (edges resize it, the rest moves it)
                None => Self::edge_cursor(self.edge_at(p)),
            }
        };
        // SAFETY: a system cursor.
        unsafe { LoadCursorW(None, id).unwrap_or_default() }
    }

    // ---- toolbar ----------------------------------------------------------------

    /// wide enough for "saved · kadr_2026-11-11_22-27-21_2.png" and the
    /// blur hint (checked in the toolbar screenshots, both themes).
    const STATUS_W: i32 = 236;

    fn buttons(&self) -> Vec<(Button, RECT)> {
        let b = |x: i32| RECT { left: self.pxi(x), top: self.pxi(6), right: self.pxi(x + 28), bottom: self.pxi(34) };
        let mut out = Vec::new();
        let mut x = 6;
        for t in Tool::ALL {
            out.push((Button::Tool(t), b(x)));
            x += 30;
        }
        out.push((Button::Style, b(x)));
        x += 30;
        x += 9;
        for k in [Button::Undo, Button::Redo] {
            out.push((k, b(x)));
            x += 30;
        }
        x += 9;
        for k in [Button::Copy, Button::Save] {
            out.push((k, b(x)));
            x += 30;
        }
        x += 9 + Self::STATUS_W + 9;
        out.push((Button::Close, b(x)));
        out
    }

    /// x positions (logical) of the three separators and the status text.
    fn bar_layout() -> ([i32; 3], i32) {
        let tools_end = 6 + (Tool::ALL.len() as i32 + 1) * 30 - 2;
        let s1 = tools_end + 5;
        let s2 = s1 + 4 + 60 - 2 + 5;
        let s3 = s2 + 4 + 60 - 2 + 5;
        ([s1, s2, s3], s3 + 5)
    }

    fn bar_size(&self) -> (i32, i32) {
        let (_, text) = Self::bar_layout();
        (self.pxi(text + Self::STATUS_W + 9 + 28 + 6 - 1), self.pxi(40))
    }

    /// below the image, right-aligned; above it if there is no room below;
    /// inside its bottom edge if neither fits. always inside the work area.
    fn bar_place(&self, w: i32, h: i32) -> (i32, i32) {
        let work = self.work_area();
        let gap = self.pxi(8);
        let at = self.at.get();
        let img = RECT { left: at.x, top: at.y, right: at.x + at.width, bottom: at.y + at.height };
        let x = (img.right - w).min(work.right - w).max(work.left);
        let y = if img.bottom + gap + h <= work.bottom {
            img.bottom + gap
        } else if img.top - gap - h >= work.top {
            img.top - gap - h
        } else {
            (img.bottom - gap - h).min(work.bottom - h).max(work.top)
        };
        (x, y)
    }

    fn button_at(&self, x: i32, y: i32) -> Option<Button> {
        self.buttons().into_iter().find(|(_, r)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|(b, _)| b)
    }

    fn enabled(&self, b: Button) -> bool {
        match b {
            Button::Undo => self.doc.borrow().can_undo(),
            Button::Style => self.current_slot().is_some(),
            Button::Redo => self.doc.borrow().can_redo(),
            _ => true,
        }
    }

    fn hint(&self, b: Button) -> &'static str {
        match b {
            Button::Tool(Tool::Highlighter) if self.styles.borrow().highlight_rect => "highlighter · h · again: stroke",
            Button::Tool(t) => t.label(),
            Button::Style => "colour and width · [ ] · i picks",
            Button::Undo => "undo · ctrl+z",
            Button::Redo => "redo · ctrl+y",
            Button::Copy => "copy · ctrl+c",
            Button::Save => "save · ctrl+s",
            Button::Close => "close · esc",
        }
    }

    fn click(&self, b: Button) {
        if !self.enabled(b) {
            return;
        }
        self.command(match b {
            Button::Tool(t) => ID_TOOL + Tool::ALL.iter().position(|x| *x == t).unwrap() as u16,
            Button::Style => ID_STYLE,
            Button::Undo => ID_UNDO,
            Button::Redo => ID_REDO,
            Button::Copy => ID_COPY,
            Button::Save => ID_SAVE,
            Button::Close => ID_CLOSE,
        });
    }

    fn paint_bar(&self, hdc: HDC) {
        let (w, h) = self.bar_size();
        let fonts = self.fonts.borrow();
        let Some(fonts) = fonts.as_ref() else { return };
        let t = self.theme;
        let (seps, text_x) = Self::bar_layout();
        self.canvas.paint(hdc, (w, h), (0, 0), |p| {
            p.fill(RECT { left: 0, top: 0, right: w, bottom: h }, t.raised);
            for (b, r) in self.buttons() {
                let active = matches!(b, Button::Tool(x) if x == self.tool.get()) || (b == Button::Style && self.flyout_visible());
                let enabled = self.enabled(b);
                if self.pressed.get() == Some(b) && enabled {
                    p.fill_round(r, self.px(6.0), t.cap);
                } else if active {
                    // seg_on is white in the light theme, like the bar itself
                    p.fill_round(r, self.px(6.0), if t.dark { t.seg_on } else { t.cap });
                } else if self.hover.get() == Some(b) && enabled {
                    p.fill_round(r, self.px(6.0), t.hover);
                }
                let ink = if !enabled {
                    t.ink3
                } else if active || self.hover.get() == Some(b) {
                    t.ink
                } else {
                    t.ink2
                };
                let alpha = if enabled { 1.0 } else { 0.45 };
                self.icon(p, b, r, [ink[0], ink[1], ink[2], alpha]);
            }
            for sx in seps {
                let x = self.px(sx as f32);
                p.fill_f(D2D_RECT_F { left: x, top: self.px(10.0), right: x + self.px(1.0).max(1.0), bottom: h as f32 - self.px(10.0) }, t.hair);
            }
            let text = D2D_RECT_F { left: self.px(text_x as f32 + 4.0), top: 0.0, right: self.px((text_x + Self::STATUS_W) as f32), bottom: h as f32 };
            let status = self.status.borrow();
            let fly_hint = self.fly_hover.get().map(|f| match f {
                Fly::Preset(i) => ["red · 1", "orange · 2", "yellow · 3", "green · 4", "blue · 5", "black · 6", "white · 7"][i],
                Fly::Custom => "any colour (windows colour dialog)",
                Fly::Pick => "eyedropper · i · a colour from the capture",
                Fly::Track => "width · [ ] · mouse wheel",
            });
            match (self.hover.get(), status.as_ref()) {
                (_, Some((s, true))) => p.text(s, &fonts.small, text, t.alert),
                (None, _) if fly_hint.is_some() => p.text(fly_hint.unwrap(), &fonts.small, text, t.ink2),
                (Some(b), _) => p.text(self.hint(b), &fonts.small, text, t.ink2),
                (None, Some((s, false))) => p.text(s, &fonts.small, text, t.ink),
                (None, None) => p.text(&format!("{} × {}", self.at.get().width, self.at.get().height), &fonts.mono, text, t.ink3),
            }
        });
    }

    /// icons are drawn geometry, sized to the dpi (DESIGN.md).
    fn icon(&self, p: &Painter, b: Button, r: RECT, c: [f32; 4]) {
        let cx = (r.left + r.right) as f32 / 2.0;
        let cy = (r.top + r.bottom) as f32 / 2.0;
        let s = self.px(1.0);
        let sw = (1.5 * s).max(1.0);
        let at = |x: f32, y: f32| (cx + x * s, cy + y * s);
        let poly = |pts: &[(f32, f32)], close: bool| {
            for w in pts.windows(2) {
                p.line_w(at(w[0].0, w[0].1), at(w[1].0, w[1].1), c, sw);
            }
            if close && pts.len() > 2 {
                let (a, z) = (pts[0], pts[pts.len() - 1]);
                p.line_w(at(z.0, z.1), at(a.0, a.1), c, sw);
            }
        };
        match b {
            Button::Style => {
                // the current colour as a dot, its size following the width;
                // a square for redact, a ring when nothing has a style
                match self.current_slot() {
                    Some(slot) => {
                        let st = self.current_style().unwrap();
                        let col = [st.color[0], st.color[1], st.color[2], slot.alpha().max(0.6)];
                        if slot == Slot::Redact {
                            p.fill_f(D2D_RECT_F { left: cx - 6.0 * s, top: cy - 4.0 * s, right: cx + 6.0 * s, bottom: cy + 4.0 * s }, col);
                        } else {
                            let rad = (3.0 + 5.0 * (st.width - MIN_WIDTH) / (MAX_WIDTH - MIN_WIDTH)) * s;
                            p.fill_ellipse((cx, cy), rad, col);
                        }
                        p.ellipse((cx, cy), 8.5 * s, 8.5 * s, [c[0], c[1], c[2], 0.35], s);
                    }
                    None => p.ellipse((cx, cy), 6.0 * s, 6.0 * s, c, sw),
                }
            }
            Button::Tool(Tool::Select) => poly(&[(-4.0, -7.0), (-4.0, 6.0), (-0.5, 2.5), (5.0, 2.5)], true),
            Button::Tool(Tool::Pen) => {
                poly(&[(-6.0, 6.0), (-5.0, 2.0), (4.0, -7.0), (7.0, -4.0), (-2.0, 5.0)], true);
                poly(&[(2.0, -5.0), (5.0, -2.0)], false);
            }
            Button::Tool(Tool::Line) => {
                poly(&[(-6.0, 6.0), (6.0, -6.0)], false);
                p.fill_f(D2D_RECT_F { left: cx - 8.0 * s, top: cy + 4.0 * s, right: cx - 4.0 * s, bottom: cy + 8.0 * s }, c);
                p.fill_f(D2D_RECT_F { left: cx + 4.0 * s, top: cy - 8.0 * s, right: cx + 8.0 * s, bottom: cy - 4.0 * s }, c);
            }
            Button::Tool(Tool::Arrow) => {
                poly(&[(-6.0, 6.0), (6.0, -6.0)], false);
                poly(&[(-0.5, -6.0), (6.0, -6.0), (6.0, 0.5)], false);
            }
            Button::Tool(Tool::Rect) => poly(&[(-7.0, -5.0), (7.0, -5.0), (7.0, 5.0), (-7.0, 5.0)], true),
            Button::Tool(Tool::Ellipse) => p.ellipse(at(0.0, 0.0), 7.0 * s, 5.5 * s, c, sw),
            Button::Tool(Tool::Highlighter) if self.styles.borrow().highlight_rect => {
                p.fill_f(D2D_RECT_F { left: cx - 7.0 * s, top: cy - 4.0 * s, right: cx + 7.0 * s, bottom: cy + 4.0 * s }, [c[0], c[1], c[2], c[3] * 0.55]);
                poly(&[(-7.0, 7.0), (7.0, 7.0)], false);
            }
            Button::Tool(Tool::Highlighter) => {
                p.line_w(at(-6.0, 2.0), at(6.0, -2.0), [c[0], c[1], c[2], c[3] * 0.55], 5.0 * s);
                poly(&[(-7.0, 7.0), (7.0, 7.0)], false);
            }
            Button::Tool(Tool::Blur) => {
                for (i, (x, y)) in [(-5.0, -5.0), (0.0, -5.0), (5.0, -5.0), (-5.0, 0.0), (0.0, 0.0), (5.0, 0.0), (-5.0, 5.0), (0.0, 5.0), (5.0, 5.0)].into_iter().enumerate() {
                    let a = [1.0, 0.6, 0.3, 0.6, 1.0, 0.6, 0.3, 0.6, 1.0][i];
                    p.fill_ellipse(at(x, y), 1.6 * s, [c[0], c[1], c[2], c[3] * a]);
                }
            }
            Button::Tool(Tool::Cursor) => {
                // the pointer with a click ring at its tip
                poly(&[(-2.0, -5.0), (-2.0, 7.0), (1.0, 4.0), (6.0, 4.0)], true);
                p.ellipse(at(-2.0, -5.0), 3.5 * s, 3.5 * s, [c[0], c[1], c[2], c[3] * 0.6], s);
            }
            Button::Tool(Tool::Redact) => {
                poly(&[(-7.0, -5.0), (7.0, -5.0), (7.0, 5.0), (-7.0, 5.0)], true);
                p.fill_f(D2D_RECT_F { left: cx - 4.5 * s, top: cy - 1.5 * s, right: cx + 4.5 * s, bottom: cy + 1.5 * s }, c);
            }
            Button::Undo | Button::Redo => {
                let d = if b == Button::Undo { 1.0 } else { -1.0 };
                // a hook: a half circle on the right, the arrow on top-left
                let pts: Vec<(f32, f32)> = (0..=10)
                    .map(|i| {
                        let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * (i as f32 / 10.0);
                        (d * (1.0 + 4.5 * a.cos()), 0.5 + 4.5 * a.sin())
                    })
                    .collect();
                let mut line = vec![(d * -6.0, -4.0)];
                line.extend(pts);
                line.push((d * -4.0, 5.0));
                poly(&line, false);
                poly(&[(d * -3.0, -7.0), (d * -6.0, -4.0), (d * -3.0, -1.0)], false);
            }
            Button::Copy => {
                poly(&[(-6.0, 3.0), (-6.0, -6.0), (3.0, -6.0)], false);
                poly(&[(-3.0, -3.0), (6.0, -3.0), (6.0, 6.0), (-3.0, 6.0)], true);
            }
            Button::Save => {
                poly(&[(0.0, -7.0), (0.0, 2.0)], false);
                poly(&[(-4.0, -2.0), (0.0, 2.0), (4.0, -2.0)], false);
                poly(&[(-7.0, 2.0), (-7.0, 6.0), (7.0, 6.0), (7.0, 2.0)], false);
            }
            Button::Close => {
                poly(&[(-5.0, -5.0), (5.0, 5.0)], false);
                poly(&[(5.0, -5.0), (-5.0, 5.0)], false);
            }
        }
    }

    fn redraw_bar(&self) {
        // SAFETY: our own toolbar window.
        unsafe {
            let _ = InvalidateRect(Some(self.bar.get()), None, false);
        }
    }

    // ---- style: colour, width, eyedropper ---------------------------------------

    /// the slot the style panel edits: the selected shape's, else the tool's.
    fn current_slot(&self) -> Option<Slot> {
        if let Some(id) = self.selected.get() {
            return self.doc.borrow().find(id).and_then(|i| i.shape.slot());
        }
        self.tool.get().slot()
    }

    fn current_style(&self) -> Option<Style> {
        if let Some(id) = self.selected.get() {
            return self.doc.borrow().find(id).filter(|i| i.shape.slot().is_some()).map(|i| i.style);
        }
        self.current_slot().map(|s| self.styles.borrow().get(s))
    }

    /// a new style for the current slot and, with a shape selected, for that
    /// shape too (one undoable edit). `merge` folds wheel steps into one edit;
    /// `commit: false` only previews (a width drag in progress).
    fn apply_style(&self, style: Style, merge: bool, commit: bool) {
        let Some(slot) = self.current_slot() else { return };
        let style = Styles::normalized(slot, style);
        self.styles.borrow_mut().set(slot, style);
        if let Some(id) = self.selected.get() {
            let before = self.slider.get().map(|(_, b)| b).or_else(|| self.doc.borrow().find(id).map(|i| i.style));
            let area = if commit {
                let before = before.unwrap_or(style);
                let a = self.doc.borrow_mut().restyle(id, before, style, merge);
                trace!("editor {}: restyle {id} #{:06x} {} -> state {}", Self::pid(), hex_of(style.color), style.width, self.doc.borrow().state());
                a
            } else {
                self.doc.borrow_mut().preview_style(id, style)
            };
            if let Some(a) = area {
                self.invalidate(a);
            }
        }
        if commit {
            trace!("editor {}: style {} #{:06x} {}", Self::pid(), slot.name(), hex_of(style.color), style.width);
            self.save_styles();
        }
        self.redraw_bar();
        self.refresh_flyout();
    }

    fn set_color(&self, hex: u32) {
        let Some(mut st) = self.current_style() else { return };
        let c = rgb_of(hex);
        st.color = [c[0], c[1], c[2], 1.0];
        self.apply_style(st, false, true);
    }

    fn set_width(&self, width: f32, merge: bool) {
        let Some(mut st) = self.current_style() else { return };
        if !self.current_slot().is_some_and(|s| s.has_width()) {
            return;
        }
        st.width = width.round().clamp(MIN_WIDTH, MAX_WIDTH);
        self.apply_style(st, merge, true);
    }

    fn step_width(&self, by: f32) {
        if let Some(st) = self.current_style() {
            self.set_width(st.width + by, true);
        }
    }

    fn save_styles(&self) {
        let Some(path) = &self.styles_path else { return };
        let text = self.styles.borrow().to_text();
        // written whole and renamed over, so a crash never leaves half a file
        let tmp = path.with_extension("ini.tmp");
        let ok = path.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, path).is_ok();
        if !ok {
            trace!("editor {}: could not remember the style in {}", Self::pid(), path.display());
        }
    }

    /// the colour of the captured image (never the annotations) at `p`.
    fn source_color(&self, p: Pt) -> u32 {
        let x = (p.x as i32).clamp(0, self.screen.width - 1) as usize;
        let y = (p.y as i32).clamp(0, self.screen.height - 1) as usize;
        let i = y * self.source.stride() + x * 4;
        let px = &self.source.pixels()[i..i + 3];
        ((px[2] as u32) << 16) | ((px[1] as u32) << 8) | px[0] as u32
    }

    fn start_pick(&self) {
        if self.current_slot().is_none() || self.dragging() {
            return;
        }
        self.pick.set(true);
        self.pick_color.set(None);
        trace!("editor {}: eyedropper", Self::pid());
        self.refresh_flyout();
    }

    fn end_pick(&self) {
        self.pick.set(false);
        self.pick_color.set(None);
        self.refresh_flyout();
    }

    /// the system colour dialog for any colour.
    fn custom_color(&self) {
        let Some(st) = self.current_style() else { return };
        use windows::Win32::UI::Controls::Dialogs::{CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
        let hex = hex_of(st.color);
        let mut custom = *self.custom.borrow();
        let mut cc = CHOOSECOLORW {
            lStructSize: size_of::<CHOOSECOLORW>() as u32,
            hwndOwner: self.hwnd.get(),
            // colorref is 0x00bbggrr
            rgbResult: windows::Win32::Foundation::COLORREF(((hex & 0xff) << 16) | (hex & 0xff00) | (hex >> 16)),
            lpCustColors: custom.as_mut_ptr(),
            Flags: CC_FULLOPEN | CC_RGBINIT,
            ..Default::default()
        };
        // SAFETY: cc points to live locals for the duration of the dialog.
        let chosen = unsafe {
            let _ = EnableWindow(self.bar.get(), false);
            let _ = EnableWindow(self.flyout.get(), false);
            let ok = ChooseColorW(&mut cc).as_bool();
            let _ = EnableWindow(self.bar.get(), true);
            let _ = EnableWindow(self.flyout.get(), true);
            ok
        };
        *self.custom.borrow_mut() = custom;
        if chosen {
            let c = cc.rgbResult.0;
            self.set_color(((c & 0xff) << 16) | (c & 0xff00) | (c >> 16 & 0xff));
        }
    }

    // ---- the style panel ----------------------------------------------------------

    fn flyout_visible(&self) -> bool {
        let f = self.flyout.get();
        // SAFETY: plain query.
        !f.is_invalid() && unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(f) }.as_bool()
    }

    fn toggle_flyout(&self) {
        if self.flyout_visible() {
            self.hide_flyout();
        } else if self.current_slot().is_some() {
            self.show_flyout();
        }
    }

    fn hide_flyout(&self) {
        if self.pick.get() {
            self.end_pick();
        }
        if self.flyout_visible() {
            // SAFETY: our own window.
            unsafe {
                let _ = ShowWindow(self.flyout.get(), windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
            }
            self.redraw_bar();
        }
    }

    fn show_flyout(&self) {
        // SAFETY: creating and placing our own owned popup.
        unsafe {
            if self.flyout.get().is_invalid() {
                let Ok(instance) = GetModuleHandleW(None) else { return };
                let Ok(f) = CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    STYLE_CLASS,
                    w!("kadr colour and width"),
                    WS_POPUP,
                    0,
                    0,
                    10,
                    10,
                    Some(self.hwnd.get()),
                    None,
                    Some(instance.into()),
                    Some(self as *const Editor as *const _),
                ) else {
                    return;
                };
                let round = DWMWCP_ROUND;
                let _ = DwmSetWindowAttribute(f, DWMWA_WINDOW_CORNER_PREFERENCE, (&round as *const DWM_WINDOW_CORNER_PREFERENCE).cast(), 4);
                let on: i32 = 1;
                let _ = DwmSetWindowAttribute(f, DWMWA_TRANSITIONS_FORCEDISABLED, (&on as *const i32).cast(), 4);
                self.flyout.set(f);
            }
            self.place_flyout();
            let _ = ShowWindow(self.flyout.get(), SW_SHOWNOACTIVATE);
            trace!("editor {}: style panel", Self::pid());
        }
        self.redraw_bar();
    }

    /// size and place: under the toolbar, at the style button, or above the
    /// toolbar when there is no room; inside the work area.
    fn place_flyout(&self) {
        let (w, h) = self.fly_size();
        let mut bar = RECT::default();
        // SAFETY: plain queries and a move of our own window.
        unsafe {
            let _ = GetWindowRect(self.bar.get(), &mut bar);
            let work = self.work_area();
            let bx = self.buttons().into_iter().find(|(b, _)| *b == Button::Style).map_or(0, |(_, r)| r.left);
            let gap = self.pxi(6);
            let x = (bar.left + bx - self.pxi(10)).min(work.right - w).max(work.left);
            let y = if bar.bottom + gap + h <= work.bottom { bar.bottom + gap } else { (bar.top - gap - h).max(work.top) };
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                self.flyout.get(),
                None,
                x,
                y,
                w,
                h,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE | windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
            );
        }
    }

    /// the panel follows the slot (redact has no width row) and shows the
    /// colour under the eyedropper.
    fn refresh_flyout(&self) {
        if !self.flyout_visible() {
            return;
        }
        if self.current_slot().is_none() {
            self.hide_flyout();
            return;
        }
        self.place_flyout();
        // SAFETY: our own window.
        unsafe {
            let _ = InvalidateRect(Some(self.flyout.get()), None, false);
        }
    }

    fn fly_size(&self) -> (i32, i32) {
        let rows = if self.current_slot().is_some_and(|s| s.has_width()) { 120 } else { 86 };
        (self.pxi(272), self.pxi(rows))
    }

    /// the panel's parts, in its own client pixels.
    fn fly_parts(&self) -> Vec<(Fly, RECT)> {
        let r = |x: i32, y: i32, w: i32, h: i32| RECT { left: self.pxi(x), top: self.pxi(y), right: self.pxi(x + w), bottom: self.pxi(y + h) };
        let mut out: Vec<(Fly, RECT)> = (0..PRESETS.len()).map(|i| (Fly::Preset(i), r(10 + i as i32 * 28, 10, 24, 24))).collect();
        out.push((Fly::Custom, r(210, 10, 24, 24)));
        out.push((Fly::Pick, r(238, 10, 24, 24)));
        if self.current_slot().is_some_and(|s| s.has_width()) {
            out.push((Fly::Track, r(10, 44, 200, 24)));
        }
        out
    }

    fn fly_at(&self, x: i32, y: i32) -> Option<Fly> {
        self.fly_parts().into_iter().find(|(_, r)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|(f, _)| f)
    }

    fn track_width(&self, x: i32) -> f32 {
        let track = self.fly_parts().into_iter().find(|(f, _)| *f == Fly::Track).map(|(_, r)| r).unwrap_or_default();
        let (l, rr) = (track.left + self.pxi(6), track.right - self.pxi(6));
        let t = ((x - l) as f32 / (rr - l).max(1) as f32).clamp(0.0, 1.0);
        (MIN_WIDTH + t * (MAX_WIDTH - MIN_WIDTH)).round()
    }

    fn fly_down(&self, x: i32, y: i32) {
        match self.fly_at(x, y) {
            Some(Fly::Preset(i)) => self.set_color(PRESETS[i]),
            Some(Fly::Custom) => self.custom_color(),
            Some(Fly::Pick) => {
                if self.pick.get() { self.end_pick() } else { self.start_pick() }
            }
            Some(Fly::Track) => {
                let Some(st) = self.current_style() else { return };
                self.slider.set(Some((self.selected.get(), st)));
                // SAFETY: our own window.
                unsafe {
                    SetCapture(self.flyout.get());
                }
                self.apply_style(Style { width: self.track_width(x), ..st }, false, false);
            }
            None => {}
        }
    }

    fn fly_move(&self, x: i32, y: i32) {
        if self.slider.get().is_some() {
            if let Some(st) = self.current_style() {
                self.apply_style(Style { width: self.track_width(x), ..st }, false, false);
            }
            return;
        }
        let h = self.fly_at(x, y);
        if h != self.fly_hover.get() {
            self.fly_hover.set(h);
            self.redraw_bar();
            // SAFETY: our own window.
            unsafe {
                let _ = InvalidateRect(Some(self.flyout.get()), None, false);
            }
        }
    }

    fn fly_up(&self, x: i32) {
        let Some((_, _)) = self.slider.get() else { return };
        let Some(st) = self.current_style() else {
            self.slider.set(None);
            return;
        };
        // one undoable edit from the style before the drag to the last one
        self.apply_style(Style { width: self.track_width(x), ..st }, false, true);
        self.slider.set(None);
        // SAFETY: plain call.
        unsafe {
            let _ = ReleaseCapture();
        }
    }

    fn paint_flyout(&self, hdc: HDC) {
        let (w, h) = self.fly_size();
        let fonts = self.fonts.borrow();
        let Some(fonts) = fonts.as_ref() else { return };
        let Some(slot) = self.current_slot() else { return };
        let Some(st) = self.current_style() else { return };
        let t = self.theme;
        let s = self.px(1.0);
        let shown = self.pick_color.get().map(rgb_of).map(|c| [c[0], c[1], c[2], st.color[3]]).unwrap_or(st.color);
        let current = hex_of(st.color);
        self.canvas.paint(hdc, (w, h), (0, 0), |p| {
            p.fill(RECT { left: 0, top: 0, right: w, bottom: h }, t.raised);
            for (part, r) in self.fly_parts() {
                let hot = self.fly_hover.get() == Some(part);
                match part {
                    Fly::Preset(i) => {
                        p.fill_round(r, 6.0 * s, rgb_of(PRESETS[i]));
                        // a hairline so white and black hold on both themes
                        p.stroke_round(r, 6.0 * s, [t.ink[0], t.ink[1], t.ink[2], if hot { 0.6 } else { 0.18 }]);
                        if PRESETS[i] == current {
                            let m = RECT { left: r.left - self.pxi(3), top: r.top - self.pxi(3), right: r.right + self.pxi(3), bottom: r.bottom + self.pxi(3) };
                            ui::crop_marks(p, m, t.ink, 6.0 * s, (2.0 * s).max(2.0), 0.0);
                        }
                    }
                    Fly::Custom => {
                        let custom = !PRESETS.contains(&current);
                        if custom {
                            p.fill_round(r, 6.0 * s, rgb_of(current));
                            let m = RECT { left: r.left - self.pxi(3), top: r.top - self.pxi(3), right: r.right + self.pxi(3), bottom: r.bottom + self.pxi(3) };
                            ui::crop_marks(p, m, t.ink, 6.0 * s, (2.0 * s).max(2.0), 0.0);
                        } else if hot {
                            p.fill_round(r, 6.0 * s, t.hover);
                        }
                        p.stroke_round(r, 6.0 * s, [t.ink[0], t.ink[1], t.ink[2], 0.35]);
                        let (cx, cy) = ((r.left + r.right) as f32 / 2.0, (r.top + r.bottom) as f32 / 2.0);
                        let ink = if custom { if (rgb_of(current).iter().sum::<f32>()) > 1.6 { [0.0, 0.0, 0.0, 0.8] } else { [1.0, 1.0, 1.0, 0.9] } } else { [t.ink2[0], t.ink2[1], t.ink2[2], 1.0] };
                        p.line_w((cx - 5.0 * s, cy), (cx + 5.0 * s, cy), ink, 1.5 * s);
                        p.line_w((cx, cy - 5.0 * s), (cx, cy + 5.0 * s), ink, 1.5 * s);
                    }
                    Fly::Pick => {
                        if self.pick.get() {
                            p.fill_round(r, 6.0 * s, if t.dark { t.seg_on } else { t.cap });
                        } else if hot {
                            p.fill_round(r, 6.0 * s, t.hover);
                        }
                        let (cx, cy) = ((r.left + r.right) as f32 / 2.0, (r.top + r.bottom) as f32 / 2.0);
                        let ink = if self.pick.get() || hot { t.ink } else { t.ink2 };
                        let c = [ink[0], ink[1], ink[2], 1.0];
                        // an eyedropper: a tube with a bulb at the top right
                        p.line_w((cx - 6.0 * s, cy + 6.0 * s), (cx + 2.5 * s, cy - 2.5 * s), c, 2.0 * s);
                        p.fill_ellipse((cx + 4.0 * s, cy - 4.0 * s), 3.0 * s, c);
                        p.line_w((cx - 0.5 * s, cy - 5.5 * s), (cx + 5.5 * s, cy + 0.5 * s), c, 1.5 * s);
                    }
                    Fly::Track => {
                        let y = (r.top + r.bottom) as f32 / 2.0;
                        let (l, rr) = (r.left as f32 + 6.0 * s, r.right as f32 - 6.0 * s);
                        p.fill_f(D2D_RECT_F { left: l, top: y - 1.0 * s, right: rr, bottom: y + 1.0 * s }, [t.ink3[0], t.ink3[1], t.ink3[2], 0.5]);
                        let k = l + (rr - l) * (st.width - MIN_WIDTH) / (MAX_WIDTH - MIN_WIDTH);
                        p.fill_f(D2D_RECT_F { left: l, top: y - 1.0 * s, right: k, bottom: y + 1.0 * s }, [t.ink[0], t.ink[1], t.ink[2], 1.0]);
                        p.fill_ellipse((k, y), 6.0 * s, [t.ink[0], t.ink[1], t.ink[2], 1.0]);
                        p.fill_ellipse((k, y), 3.5 * s, [t.raised[0], t.raised[1], t.raised[2], 1.0]);
                        let label = D2D_RECT_F { left: r.right as f32, top: r.top as f32, right: w as f32 - 10.0 * s, bottom: r.bottom as f32 };
                        p.text_right(&format!("{} px", st.width), &fonts.mono, label, t.ink2);
                    }
                }
            }
            // the preview, at the real size: image pixels are screen pixels
            let top = if slot.has_width() { 78.0 } else { 44.0 } * s;
            let area = D2D_RECT_F { left: 10.0 * s, top, right: w as f32 - 10.0 * s, bottom: top + 32.0 * s };
            p.fill_f(area, [t.cap[0], t.cap[1], t.cap[2], 1.0]);
            let cy = (area.top + area.bottom) / 2.0;
            match slot {
                Slot::Redact => p.fill_f(D2D_RECT_F { left: area.left + 14.0 * s, top: cy - 8.0 * s, right: area.right - 14.0 * s, bottom: cy + 8.0 * s }, [shown[0], shown[1], shown[2], 1.0]),
                Slot::Highlight => {
                    // a line of "text" under it shows the transparency
                    p.fill_f(D2D_RECT_F { left: area.left + 24.0 * s, top: cy - 2.0 * s, right: area.right - 24.0 * s, bottom: cy + 2.0 * s }, [t.ink[0], t.ink[1], t.ink[2], 0.8]);
                    let hw = st.width.min(28.0 * s) / 2.0;
                    p.fill_f(D2D_RECT_F { left: area.left + 14.0 * s, top: cy - hw, right: area.right - 14.0 * s, bottom: cy + hw }, shown);
                }
                Slot::Stroke => {
                    let hw = st.width.min(28.0 * s) / 2.0;
                    p.fill_round(
                        RECT { left: (area.left + 14.0 * s) as i32, top: (cy - hw).round() as i32, right: (area.right - 14.0 * s) as i32, bottom: (cy - hw).round() as i32 + (hw * 2.0).round().max(1.0) as i32 },
                        hw,
                        [shown[0], shown[1], shown[2]],
                    );
                }
            }
            if self.pick.get() {
                p.text("click the capture to take a colour · esc", &fonts.small, D2D_RECT_F { left: area.left + 6.0 * s, top: area.top, right: area.right, bottom: area.top + 12.0 * s }, t.ink3);
            }
        });
    }
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

unsafe fn editor_of(hwnd: HWND) -> Option<&'static Editor> {
    // SAFETY: GWLP_USERDATA holds the Editor pointer set at WM_NCCREATE,
    // valid until the loop in show_and_loop ends.
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Editor).as_ref() }
}

unsafe fn store_editor(hwnd: HWND, lparam: LPARAM) {
    // SAFETY: lparam is the CREATESTRUCTW of this window being created.
    unsafe {
        let cs = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
    }
}

unsafe extern "system" fn image_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            store_editor(hwnd, lparam);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let Some(ed) = editor_of(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
        let shift = wparam.0 & MK_SHIFT != 0;
        match msg {
            // no non-client area: the client is the whole window
            WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),
            // the frame is for the border and shadow only, never for resizing
            WM_NCHITTEST => LRESULT(HTCLIENT as isize),
            WM_WINDOWPOSCHANGING => {
                let pos = &mut *(lparam.0 as *mut WINDOWPOS);
                if !ed.resizing.get() {
                    // snap or win+arrow must not resize or move the image: it
                    // is anchored to the frozen screen, only the frame moves it
                    pos.flags |= SWP_NOSIZE | windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE;
                }
                LRESULT(0)
            }
            // the frame can be any size from a few pixels to the whole capture
            windows::Win32::UI::WindowsAndMessaging::WM_GETMINMAXINFO => {
                let info = &mut *(lparam.0 as *mut windows::Win32::UI::WindowsAndMessaging::MINMAXINFO);
                let max = POINT { x: ed.screen.width, y: ed.screen.height };
                info.ptMinTrackSize = POINT { x: 1, y: 1 };
                info.ptMaxTrackSize = max;
                info.ptMaxSize = max;
                LRESULT(0)
            }
            WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT => {
                SetCursor(Some(ed.cursor()));
                LRESULT(1)
            }
            WM_LBUTTONDOWN => {
                ed.mouse_down(ed.point(lparam), shift);
                LRESULT(0)
            }
            // moves without the button held are ignored while dragging: they
            // come from the real pointer, not from the gesture
            WM_MOUSEMOVE if wparam.0 & MK_LBUTTON != 0 => {
                ed.mouse_drag(ed.point(lparam), shift);
                LRESULT(0)
            }
            // the eyedropper shows the colour under the pointer as it moves
            WM_MOUSEMOVE if ed.pick.get() => {
                ed.pick_color.set(Some(ed.source_color(ed.point(lparam))));
                ed.refresh_flyout();
                LRESULT(0)
            }
            WM_SET_COLOR => {
                ed.set_color(wparam.0 as u32 & 0xffffff);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                ed.mouse_up(ed.point(lparam), shift);
                LRESULT(0)
            }
            // capture taken away mid-gesture (alt+tab, a dialog): finish it
            // where it is, so tools, undo and the next click work again
            WM_CAPTURECHANGED if HWND(lparam.0 as *mut _) != hwnd => {
                ed.finish_drag();
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                ed.paint_image(hdc, ps.rcPaint);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_COMMAND => {
                ed.command((wparam.0 & 0xffff) as u16);
                LRESULT(0)
            }
            WM_CLOSE => {
                ed.request_close();
                LRESULT(0)
            }
            // signing out with unsaved annotations: windows shows the reason
            // and lets the user decide.
            WM_QUERYENDSESSION => {
                if ASK_BEFORE_CLOSE && ed.dirty() {
                    let reason = wide("kadr: an edited capture is not saved");
                    let _ = ShutdownBlockReasonCreate(hwnd, PCWSTR(reason.as_ptr()));
                    return LRESULT(0);
                }
                LRESULT(1)
            }
            // win+shift+arrow can move the image to another monitor: the
            // frame goes with it
            windows::Win32::UI::WindowsAndMessaging::WM_MOVE => {
                ed.place_frame();
                LRESULT(0)
            }
            WM_ENDSESSION => {
                let _ = ShutdownBlockReasonDestroy(hwnd);
                LRESULT(0)
            }
            // another app active: the rest of the screen is that app's, not
            // dimmed; back in the editor, the veil returns
            WM_ACTIVATEAPP => {
                let cmd = if wparam.0 != 0 { SW_SHOWNOACTIVATE } else { windows::Win32::UI::WindowsAndMessaging::SW_HIDE };
                let _ = ShowWindow(ed.dim.get(), cmd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = RemovePropW(hwnd, w!("kadr.asking"));
                let dim = ed.dim.replace(HWND::default());
                if !dim.is_invalid() {
                    let _ = DestroyWindow(dim);
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe extern "system" fn bar_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            store_editor(hwnd, lparam);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let Some(ed) = editor_of(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
        let point = |lp: LPARAM| ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32);
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCHITTEST => {
                // buttons are clicks; everything else drags the toolbar
                let (sx, sy) = point(lparam);
                let mut r = RECT::default();
                let _ = GetWindowRect(hwnd, &mut r);
                if ed.button_at(sx - r.left, sy - r.top).is_some() { LRESULT(HTCLIENT as isize) } else { LRESULT(HTCAPTION as isize) }
            }
            WM_MOVING => {
                // keep the whole toolbar inside the monitor it is dragged to
                let r = &mut *(lparam.0 as *mut RECT);
                let m = MonitorFromRect(r, MONITOR_DEFAULTTONEAREST);
                let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                let _ = GetMonitorInfoW(m, &mut info);
                let work = info.rcWork;
                let (w, h) = (r.right - r.left, r.bottom - r.top);
                let x = r.left.min(work.right - w).max(work.left);
                let y = r.top.min(work.bottom - h).max(work.top);
                *r = RECT { left: x, top: y, right: x + w, bottom: y + h };
                LRESULT(1)
            }
            WM_MOUSEMOVE => {
                let (x, y) = point(lparam);
                let b = ed.button_at(x, y);
                if b != ed.hover.get() {
                    ed.hover.set(b);
                    ed.redraw_bar();
                }
                let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut tme);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                ed.hover.set(None);
                ed.redraw_bar();
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point(lparam);
                ed.pressed.set(ed.button_at(x, y));
                SetCapture(hwnd);
                ed.redraw_bar();
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = point(lparam);
                let was = ed.pressed.replace(None);
                let _ = ReleaseCapture();
                ed.redraw_bar();
                if let Some(b) = was.filter(|b| ed.button_at(x, y) == Some(*b)) {
                    ed.click(b);
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xffff) as i16;
                ed.step_width(if delta > 0 { 1.0 } else { -1.0 });
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_STATUS => {
                let _ = KillTimer(Some(hwnd), TIMER_STATUS);
                *ed.status.borrow_mut() = None;
                ed.redraw_bar();
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                ed.paint_bar(hdc);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe extern "system" fn style_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            store_editor(hwnd, lparam);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let Some(ed) = editor_of(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
        let point = |lp: LPARAM| ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32);
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_LBUTTONDOWN => {
                let (x, y) = point(lparam);
                ed.fly_down(x, y);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = point(lparam);
                ed.fly_move(x, y);
                let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut tme);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, _) = point(lparam);
                ed.fly_up(x);
                LRESULT(0)
            }
            WM_CAPTURECHANGED if HWND(lparam.0 as *mut _) != hwnd => {
                if let Some((_, _)) = ed.slider.get() {
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let _ = ScreenToClient(hwnd, &mut pt);
                    ed.fly_up(pt.x);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                ed.fly_hover.set(None);
                ed.redraw_bar();
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xffff) as i16;
                ed.step_width(if delta > 0 { 1.0 } else { -1.0 });
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                ed.paint_flyout(hdc);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// the note after copy or save: closes the editor when its time is up or on a
/// click.
unsafe extern "system" fn toast_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            store_editor(hwnd, lparam);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let Some(ed) = editor_of(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                ed.paint_toast(hdc);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_TIMER | WM_LBUTTONDOWN if msg != WM_TIMER || wparam.0 == TIMER_TOAST => {
                let _ = KillTimer(Some(hwnd), TIMER_TOAST);
                let _ = DestroyWindow(hwnd);
                let _ = DestroyWindow(ed.hwnd.get());
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// the frame around the image: white in the shape of its region; clicks on it
/// fall through to the veil below.
unsafe extern "system" fn frame_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCHITTEST => LRESULT(windows::Win32::UI::WindowsAndMessaging::HTTRANSPARENT as isize),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let white = windows::Win32::Graphics::Gdi::GetStockObject(windows::Win32::Graphics::Gdi::WHITE_BRUSH);
                let _ = windows::Win32::Graphics::Gdi::FillRect(hdc, &ps.rcPaint, windows::Win32::Graphics::Gdi::HBRUSH(white.0));
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// the veil: the frozen capture, darkened. clicks never take the focus away
/// from the editor; near the frame's edge (select tool) a press drags it.
unsafe extern "system" fn dim_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            store_editor(hwnd, lparam);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let Some(ed) = editor_of(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            windows::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND => LRESULT(1),
            WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT => {
                SetCursor(Some(ed.veil_cursor()));
                LRESULT(1)
            }
            WM_LBUTTONDOWN => {
                ed.veil_down(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE if wparam.0 & MK_LBUTTON != 0 => {
                if ed.dragging() {
                    ed.mouse_drag(Pt::default(), false);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if ed.dragging() {
                    ed.mouse_up(Pt::default(), false);
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED if HWND(lparam.0 as *mut _) != hwnd => {
                ed.finish_drag();
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                ed.paint_veil(hdc, ps.rcPaint);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
