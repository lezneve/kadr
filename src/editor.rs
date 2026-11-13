//! the capture editor: a short-lived process per capture.
//!
//! kadr side (`launch`): the captured pixels go into an anonymous
//! shared-memory section with an inheritable handle; kadr starts
//! `kadr --editor <section> --ready <event> --at x,y,w,h`, closes its own
//! copy of the section at once and, for a region, keeps the overlay up until
//! the editor signals its first frame, so the desktop never flashes between.
//! no temporary file, no encoding, no second capture.
//!
//! editor side: editor_ui.rs (the windows, the tools and the export) over
//! annotate.rs (the shapes and their history).
//!
//! direct2d and directwrite live only in this process and go with it; the
//! resident kadr never loads them.

use std::path::PathBuf;
use std::time::Instant;

use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::System::Memory::{CreateFileMappingW, FILE_MAP_ALL_ACCESS, MapViewOfFile, PAGE_READWRITE, UnmapViewOfFile};
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, DispatchMessageW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE,
    PeekMessageW, PostQuitMessage, QS_ALLINPUT, TranslateMessage, WM_QUIT,
};
use windows::core::PCWSTR;

use crate::capture::{CursorShot, Frame, Rect};
use crate::trace;


// ---- kadr side ------------------------------------------------------------

/// a started editor, until its first frame is on screen.
pub struct Launched {
    child: std::process::Child,
    ready: HANDLE,
}

/// starts an editor for `frame`, which was captured at `at` on the screen.
/// `cursor`: the pointer when the capture started; its pixels follow the
/// frame's in the section and `--cursor` says where its hotspot is in the
/// image (it may be outside).
pub fn launch(frame: &Frame, at: Rect, cursor: Option<&CursorShot>, settings: Option<&std::path::Path>, private_clipboard: bool) -> std::result::Result<Launched, String> {
    let t = Instant::now();
    let frame_size = frame.byte_len();
    let size = frame_size + cursor.map_or(0, |c| c.image.pixels.len());
    let inherit = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: std::ptr::null_mut(), bInheritHandle: true.into() };
    // SAFETY: an anonymous pagefile-backed section of exactly `size` bytes;
    // the view is written once and unmapped before the child starts.
    let (section, ready) = unsafe {
        let section = CreateFileMappingW(INVALID_HANDLE_VALUE, Some(&inherit), PAGE_READWRITE, (size as u64 >> 32) as u32, size as u32, PCWSTR::null())
            .map_err(|e| format!("could not hand the capture to the editor: {}", e.message()))?;
        let view = MapViewOfFile(section, FILE_MAP_ALL_ACCESS, 0, 0, size);
        if view.Value.is_null() {
            let _ = CloseHandle(section);
            return Err("could not hand the capture to the editor: no memory for it".into());
        }
        // bgrx with the x byte set to 255: direct2d effects read it as alpha
        // (a zero there made the blur add to the image, research/editor.md).
        let dst = std::slice::from_raw_parts_mut(view.Value.cast::<u32>(), frame_size / 4);
        for (d, s) in dst.iter_mut().zip(frame.pixels().chunks_exact(4)) {
            *d = u32::from_le_bytes([s[0], s[1], s[2], 0xff]);
        }
        if let Some(c) = cursor {
            std::ptr::copy_nonoverlapping(c.image.pixels.as_ptr(), view.Value.cast::<u8>().add(frame_size), c.image.pixels.len());
        }
        let _ = UnmapViewOfFile(view);
        let ready = match CreateEventW(Some(&inherit), true, false, PCWSTR::null()) {
            Ok(e) => e,
            Err(e) => {
                let _ = CloseHandle(section);
                return Err(format!("could not start the editor: {}", e.message()));
            }
        };
        (section, ready)
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string());
    let child = exe.and_then(|exe| {
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--editor")
            .arg((section.0 as usize).to_string())
            .arg("--ready")
            .arg((ready.0 as usize).to_string())
            .arg("--at")
            .arg(format!("{},{},{},{}", at.x, at.y, at.width, at.height))
            .arg("--process")
            .arg(crate::storage::app());
        if let Some(c) = cursor {
            let i = &c.image;
            cmd.arg("--cursor").arg(format!("{},{},{},{},{},{}", c.x - at.x, c.y - at.y, i.width, i.height, i.hot_x, i.hot_y));
        }
        if let Some(p) = settings {
            cmd.arg("--settings").arg(p);
        }
        if crate::tracing() {
            cmd.arg("--trace");
        }
        if private_clipboard {
            cmd.arg("--test-private-clipboard");
        }
        cmd.spawn().map_err(|e| e.to_string()).inspect(|child| {
            // hands kadr's own foreground right to this editor only, nothing
            // wider. windows grants kadr that right when a real hotkey or tray
            // click reaches it; without one (a posted test hotkey, or the user
            // already typing elsewhere) the call fails and the editor opens
            // behind, with its taskbar button flashing. no stronger trick is
            // used: other apps keep their focus.
            // SAFETY: plain call with the child's process id.
            let granted = unsafe { AllowSetForegroundWindow(child.id()) }.is_ok();
            trace!("editor {}: foreground right passed on: {granted}", child.id());
        })
    });
    // SAFETY: the child has its own inherited handle now (or never started);
    // kadr keeps no reference to the pixels.
    unsafe {
        let _ = CloseHandle(section);
    }
    match child {
        Ok(child) => {
            trace!("editor process {} started, hand-off {:.1} ms", child.id(), t.elapsed().as_secs_f64() * 1000.0);
            Ok(Launched { child, ready })
        }
        Err(e) => {
            // SAFETY: our own event handle.
            unsafe {
                let _ = CloseHandle(ready);
            }
            Err(format!("could not start the editor: {e}"))
        }
    }
}

impl Launched {
    /// lets the editor run without waiting for it.
    pub fn detach(self) {
        // SAFETY: our own event handle; the child handle closes on drop.
        unsafe {
            let _ = CloseHandle(self.ready);
        }
    }

    /// waits until the editor's first frame is on screen (or it exited, or
    /// `timeout_ms` passed), answering window messages meanwhile. kadr keeps
    /// no handle to the editor afterwards.
    pub fn wait_shown(self, timeout_ms: u32) -> bool {
        use std::os::windows::io::AsRawHandle;
        let process = HANDLE(self.child.as_raw_handle());
        let deadline = Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
        let mut msg = MSG::default();
        let mut shown = false;
        // SAFETY: a standard pump around a wait on handles we own.
        unsafe {
            loop {
                let left = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
                if left == 0 {
                    break;
                }
                let wait = MsgWaitForMultipleObjectsEx(Some(&[self.ready, process]), left, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                if wait == WAIT_OBJECT_0 {
                    shown = true;
                    break;
                }
                if wait.0 == WAIT_OBJECT_0.0 + 1 {
                    break; // the editor exited before showing anything
                }
                let mut quit = false;
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        PostQuitMessage(msg.wParam.0 as i32);
                        quit = true;
                        break;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                if quit {
                    break;
                }
            }
            let _ = CloseHandle(self.ready);
        }
        drop(self.child); // closes the process handle; the editor runs on its own
        shown
    }
}

// ---- editor side ----------------------------------------------------------

pub struct Args {
    pub section: usize,
    pub ready: usize,
    pub at: Rect,
    /// the pointer at capture time: hotspot x, y in the image, width,
    /// height, hotspot inside the pointer. its pixels follow the frame's.
    pub cursor: Option<[i32; 6]>,
}

pub fn parse_cursor(v: &str) -> Option<[i32; 6]> {
    let n: Vec<i32> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    match n[..] {
        [x, y, w, h, hx, hy] if (1..=512).contains(&w) && (1..=512).contains(&h) => Some([x, y, w, h, hx, hy]),
        _ => None,
    }
}

pub fn parse_at(v: &str) -> Option<Rect> {
    let n: Vec<i32> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    match n[..] {
        [x, y, width, height] if width > 0 && height > 0 => Some(Rect { x, y, width, height }),
        _ => None,
    }
}

/// the editor process: see editor_ui.rs.
pub fn run(args: Args, settings_path: Option<PathBuf>, private_clipboard: bool) -> i32 {
    crate::editor_ui::run(args, settings_path, private_clipboard)
}
