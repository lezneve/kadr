//! stage 1 spike for the kadr editor. synthetic image only.
//!
//!   editor-spike parent <w> <h> <hw|sw> [runs]
//!
//! the parent fills an inheritable shared-memory section with a synthetic
//! frame, starts `child` with the handle values, closes its own copy and
//! waits for the child's "first paint" event. the child maps the section,
//! checks a checksum, paints the frame with direct2d into a popup window,
//! then renders an export offscreen (gaussian blur in one rect, solid redact
//! in another), reads it back on the cpu and checks the pixels.

use std::io::Read;
use std::process::{Command, Stdio};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_COPY, D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_CPU_READ, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_GAUSSIANBLUR_PROP_BORDER_MODE, D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_MAP_OPTIONS_READ, D2D1_PRESENT_OPTIONS_NONE, D2D1_PROPERTY_TYPE_ENUM,
    D2D1_PROPERTY_TYPE_FLOAT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1CreateFactory, ID2D1Bitmap1,
    ID2D1DeviceContext, ID2D1Factory1, ID2D1Image,
};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_READWRITE,
    UnmapViewOfFile,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, SetEvent, WaitForSingleObject};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, RegisterClassW,
    TranslateMessage, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{Interface, PCWSTR, w};

fn qpc() -> i64 {
    let mut v = 0;
    unsafe { QueryPerformanceCounter(&mut v).unwrap() };
    v
}
fn ms(from: i64, to: i64) -> f64 {
    let mut f = 0;
    unsafe { QueryPerformanceFrequency(&mut f).unwrap() };
    (to - from) as f64 * 1000.0 / f as f64
}
fn mem() -> String {
    let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
    unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(), size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32).unwrap();
    }
    format!("private {:.1} MiB, ws {:.1} MiB", c.PrivateUsage as f64 / 1048576.0, c.WorkingSetSize as f64 / 1048576.0)
}

/// a synthetic frame: gradient + 16 px checkerboard + a few hard edges, bgrx
/// with a zero fourth byte like a gdi capture.
fn synthetic(w: u32, h: u32, out: &mut [u8]) {
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let check = ((x / 16 + y / 16) % 2) as u8 * 120;
            out[i] = (x * 255 / w) as u8 ^ check;
            out[i + 1] = (y * 255 / h) as u8;
            out[i + 2] = check.wrapping_add((x / 7) as u8);
            out[i + 3] = if std::env::var_os("SPIKE_OPAQUE").is_some() { 255 } else { 0 };
        }
    }
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("parent") => parent(&a[2..]),
        Some("child") => child(&a[2..]),
        _ => eprintln!("usage: editor-spike parent <w> <h> <hw|sw> [runs]"),
    }
}

fn parent(a: &[String]) {
    let (w, h): (u32, u32) = (a[0].parse().unwrap(), a[1].parse().unwrap());
    let mode = a[2].clone();
    let runs: u32 = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(5);
    let size = (w * h * 4) as usize;
    // the frame as kadr would hold it after capture; the hand-off is a copy into the section
    let mut frame = vec![0u8; size];
    synthetic(w, h, &mut frame);
    let sum = fnv(&frame);
    let exe = std::env::current_exe().unwrap();
    for run in 0..runs {
        let t0 = qpc();
        let sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: std::ptr::null_mut(), bInheritHandle: true.into() };
        let (map, ev) = unsafe {
            let map = CreateFileMappingW(INVALID_HANDLE_VALUE, Some(&sa), PAGE_READWRITE, (size as u64 >> 32) as u32, size as u32, PCWSTR::null()).unwrap();
            let view = MapViewOfFile(map, FILE_MAP_ALL_ACCESS, 0, 0, size);
            std::ptr::copy_nonoverlapping(frame.as_ptr(), view.Value.cast::<u8>(), size);
            UnmapViewOfFile(view).unwrap();
            let ev = CreateEventW(Some(&sa), true, false, PCWSTR::null()).unwrap();
            (map, ev)
        };
        let t_copy = qpc();
        let mut child = Command::new(&exe)
            .args(["child", &(map.0 as usize).to_string(), &(ev.0 as usize).to_string(), &w.to_string(), &h.to_string(), &mode, &sum.to_string()])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let t_spawn = qpc();
        unsafe { CloseHandle(map).unwrap() }; // the child owns the only handle now
        let signalled = unsafe { WaitForSingleObject(ev, 10_000) }.0 == 0;
        let t_paint = qpc();
        let mut out = String::new();
        child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
        let status = child.wait().unwrap();
        let t_exit = qpc();
        unsafe { CloseHandle(ev).unwrap() };
        println!(
            "run {run} {mode} {w}x{h}: copy into section {:.1} ms, spawn {:.1} ms, start→first paint {:.1} ms (signalled {signalled}), child total {:.0} ms, exit {}",
            ms(t0, t_copy),
            ms(t_copy, t_spawn),
            ms(t0, t_paint),
            ms(t0, t_exit),
            status.code().unwrap_or(-1)
        );
        for line in out.lines() {
            println!("    {line}");
        }
    }
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, m, wp, lp) }
}

fn pump() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn child(a: &[String]) {
    let t_main = qpc();
    let map = HANDLE(a[0].parse::<usize>().unwrap() as *mut _);
    let ev = HANDLE(a[1].parse::<usize>().unwrap() as *mut _);
    let (w, h): (u32, u32) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let hw = a[4] == "hw";
    let want: u64 = a[5].parse().unwrap();
    let size = (w * h * 4) as usize;
    println!("main: {}", mem());
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let view: MEMORY_MAPPED_VIEW_ADDRESS = MapViewOfFile(map, FILE_MAP_READ, 0, 0, size);
        assert!(!view.Value.is_null(), "inherited section could not be mapped");
        let src = std::slice::from_raw_parts(view.Value.cast::<u8>(), size);
        let sum = fnv(src);
        println!("section mapped, checksum {}", if sum == want { "matches" } else { "DIFFERS" });

        let inst = GetModuleHandleW(None).unwrap();
        let class = w!("kadr.editor.spike");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() });
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, class, w!("spike"), WS_POPUP | WS_VISIBLE, 0, 0, w as i32, h as i32, None, None, Some(inst.into()), None).unwrap();
        let t_window = qpc();

        let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).unwrap();
        let fmt = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: if hw { D2D1_RENDER_TARGET_TYPE_DEFAULT } else { D2D1_RENDER_TARGET_TYPE_SOFTWARE },
            pixelFormat: fmt,
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let hprops = D2D1_HWND_RENDER_TARGET_PROPERTIES { hwnd, pixelSize: D2D_SIZE_U { width: w, height: h }, presentOptions: D2D1_PRESENT_OPTIONS_NONE };
        let rt = factory.CreateHwndRenderTarget(&props, &hprops).unwrap();
        let dc: ID2D1DeviceContext = rt.cast().unwrap();
        let t_target = qpc();
        let bprops = |options| D2D1_BITMAP_PROPERTIES1 { pixelFormat: fmt, dpiX: 96.0, dpiY: 96.0, bitmapOptions: options, colorContext: std::mem::ManuallyDrop::new(None) };
        let bitmap: ID2D1Bitmap1 = dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(view.Value), w * 4, &bprops(D2D1_BITMAP_OPTIONS_NONE)).unwrap();
        let t_bitmap = qpc();
        let red = dc.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 0.23, b: 0.19, a: 1.0 }, None).unwrap();
        let full = D2D_RECT_F { left: 0.0, top: 0.0, right: w as f32, bottom: h as f32 };

        dc.BeginDraw();
        dc.DrawBitmap(&bitmap, Some(&full), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
        dc.DrawRectangle(&D2D_RECT_F { left: 40.0, top: 40.0, right: 300.0, bottom: 200.0 }, &red, 3.0, None);
        let _ = rt.EndDraw(None, None);
        let t_draw = qpc();
        let _ = DwmFlush();
        let t_visible = qpc();
        SetEvent(ev).unwrap();
        pump();
        println!(
            "window {:.1} ms, d2d target {:.1} ms, bitmap upload {:.1} ms, first draw {:.1} ms, dwm flush {:.1} ms; main→visible {:.1} ms",
            ms(t_main, t_window), ms(t_window, t_target), ms(t_target, t_bitmap), ms(t_bitmap, t_draw), ms(t_draw, t_visible), ms(t_main, t_visible)
        );
        println!("first paint: {}", mem());
        // the editing loop: a blur region under a shape being dragged. each
        // frame repaints only the dirty rect (old ∪ new shape), clipped.
        {
            let blur = dc.CreateEffect(&CLSID_D2D1GaussianBlur).unwrap();
            blur.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &12.0f32.to_le_bytes()).unwrap();
            blur.SetValue(D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32, D2D1_PROPERTY_TYPE_ENUM, &1u32.to_le_bytes()).unwrap();
            blur.SetInput(0, &bitmap, true);
            let out = blur.GetOutput().unwrap();
            let r1 = blur_rect(w, h);
            let mut times = Vec::new();
            for k in 0..60u32 {
                let x = r1.left + (k as f32 * 4.0) % (r1.right - r1.left).max(1.0);
                let shape = D2D_RECT_F { left: x, top: r1.top, right: x + (w as f32 / 6.0), bottom: r1.top + (h as f32 / 6.0) };
                let dirty = D2D_RECT_F { left: shape.left - 8.0, top: shape.top - 4.0, right: shape.right + 4.0, bottom: shape.bottom + 4.0 };
                let t = qpc();
                dc.BeginDraw();
                dc.PushAxisAlignedClip(&dirty, D2D1_ANTIALIAS_MODE_ALIASED);
                dc.DrawBitmap(&bitmap, Some(&full), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
                dc.PushAxisAlignedClip(&r1, D2D1_ANTIALIAS_MODE_ALIASED);
                dc.DrawImage(&out, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_COPY);
                dc.PopAxisAlignedClip();
                dc.DrawRectangle(&shape, &red, 3.0, None);
                dc.PopAxisAlignedClip();
                let _ = rt.EndDraw(None, None);
                times.push(ms(t, qpc()));
                pump();
            }
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!("drag over blur, dirty rect {:.0}x{:.0}: frame p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms", w as f32 / 6.0 + 12.0, h as f32 / 6.0 + 8.0, times[30], times[57], times[59]);
            println!("after drag loop: {}", mem());
        }

        // export: same scene offscreen, at full resolution, read back on the cpu
        let window_target: ID2D1Image = dc.GetTarget().unwrap();
        let render = |annotate: bool| -> (Vec<u8>, f64) {
            let t = qpc();
            let target = dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, None, 0, &bprops(D2D1_BITMAP_OPTIONS_TARGET)).unwrap();
            dc.SetTarget(&target);
            dc.BeginDraw();
            dc.DrawBitmap(&bitmap, Some(&full), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
            if annotate {
                let blur = dc.CreateEffect(&CLSID_D2D1GaussianBlur).unwrap();
                blur.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &12.0f32.to_le_bytes()).unwrap();
                blur.SetValue(D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32, D2D1_PROPERTY_TYPE_ENUM, &1u32.to_le_bytes()).unwrap(); // hard
                blur.SetInput(0, &bitmap, true);
                let r1 = blur_rect(w, h);
                dc.PushAxisAlignedClip(&r1, D2D1_ANTIALIAS_MODE_ALIASED);
                let mode = if std::env::var_os("SPIKE_OVER").is_some() { D2D1_COMPOSITE_MODE_SOURCE_OVER } else { D2D1_COMPOSITE_MODE_SOURCE_COPY };
                dc.DrawImage(&blur.GetOutput().unwrap(), None, None, D2D1_INTERPOLATION_MODE_LINEAR, mode);
                dc.PopAxisAlignedClip();
                let black = dc.CreateSolidColorBrush(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }, None).unwrap();
                dc.FillRectangle(&redact_rect(w, h), &black);
            }
            dc.EndDraw(None, None).unwrap();
            let read = dc.CreateBitmap(D2D_SIZE_U { width: w, height: h }, None, 0, &bprops(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW)).unwrap();
            read.CopyFromBitmap(None, &target, None).unwrap();
            let m = read.Map(D2D1_MAP_OPTIONS_READ).unwrap();
            let mut out = vec![0u8; size];
            for y in 0..h as usize {
                std::ptr::copy_nonoverlapping(m.bits.add(y * m.pitch as usize), out.as_mut_ptr().add(y * w as usize * 4), w as usize * 4);
            }
            read.Unmap().unwrap();
            dc.SetTarget(&window_target);
            (out, ms(t, qpc()))
        };
        let (plain, t_plain) = render(false);
        let same = plain.chunks(4).zip(src.chunks(4)).all(|(a, b)| a[..3] == b[..3]);
        println!("export without annotations: {t_plain:.1} ms, identical to source (rgb): {same}, alpha byte {}", plain[3]);
        let (out, t_ann) = render(true);
        let r1 = blur_rect(w, h);
        let r2 = redact_rect(w, h);
        let inside = |r: &D2D_RECT_F, x: u32, y: u32| (x as f32) >= r.left && (x as f32) < r.right && (y as f32) >= r.top && (y as f32) < r.bottom;
        let (mut outside_diff, mut redact_bad) = (0u64, 0u64);
        let (mut v_src, mut v_out) = (Vec::new(), Vec::new());
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                if inside(&r2, x, y) {
                    redact_bad += (out[i..i + 3] != [0, 0, 0]) as u64;
                } else if inside(&r1, x, y) {
                    v_src.push(src[i + 1] as f64 + src[i + 2] as f64 + src[i] as f64);
                    v_out.push(out[i + 1] as f64 + out[i + 2] as f64 + out[i] as f64);
                } else {
                    outside_diff += (out[i..i + 3] != src[i..i + 3]) as u64;
                }
            }
        }
        let var = |v: &[f64]| {
            let m = v.iter().sum::<f64>() / v.len() as f64;
            v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64
        };
        // local roughness: mean absolute difference to the right neighbour
        let rough = |img: &[u8]| {
            let (mut s, mut n) = (0f64, 0f64);
            for y in r1.top as u32 + 2..r1.bottom as u32 - 2 {
                for x in r1.left as u32 + 2..r1.right as u32 - 3 {
                    let i = ((y * w + x) * 4) as usize;
                    s += (img[i] as f64 - img[i + 4] as f64).abs();
                    n += 1.0;
                }
            }
            s / n
        };
        println!(
            "export with blur + redact: {t_ann:.1} ms; pixels changed outside the two rects: {outside_diff}; redact pixels not black: {redact_bad}; blur variance {:.0} → {:.0}, edge roughness {:.1} → {:.1}",
            var(&v_src), var(&v_out), rough(src), rough(&out)
        );
        let mut changed = 0u64;
        for y in r1.top as u32..r1.bottom as u32 { for x in r1.left as u32..r1.right as u32 { let i = ((y * w + x) * 4) as usize; changed += (out[i..i+3] != src[i..i+3]) as u64; } }
        let cy = ((r1.top + r1.bottom) / 2.0) as u32;
        let row = |img: &[u8]| (0..8).map(|k| img[((cy * w + r1.left as u32 + 200 + k * 4) * 4) as usize]).collect::<Vec<_>>();
        println!("blur rect pixels changed: {changed} of {}; src row {:?} out row {:?}", (r1.right - r1.left) as u64 * (r1.bottom - r1.top) as u64, row(src), row(&out));
        println!("after export: {}", mem());
        drop(out);
        drop(plain);
        let _ = UnmapViewOfFile(view);
        let _ = CloseHandle(map);
        let _ = DestroyWindow(hwnd);
        pump();
    }
}

fn blur_rect(w: u32, h: u32) -> D2D_RECT_F {
    D2D_RECT_F { left: (w / 4) as f32, top: (h / 4) as f32, right: (w / 2) as f32, bottom: (h / 2) as f32 }
}
fn redact_rect(w: u32, h: u32) -> D2D_RECT_F {
    D2D_RECT_F { left: (w * 5 / 8) as f32, top: (h / 4) as f32, right: (w * 7 / 8) as f32, bottom: (h / 2) as f32 }
}
