//! kadr: screenshots, without the extra stuff.
//!
//!   kadr                        tray app (normal use)
//!   kadr --trace                same, printing timings and memory to stderr
//!   kadr-...-setup.exe          installer (any exe name containing "setup")
//!   kadr --install              installer, whatever the file name
//!   kadr --uninstall            remove kadr (used by windows settings › apps)
//!   kadr --settings <file>      use another settings file (tests)
//!   kadr --test-private-clipboard
//!                               keep copies out of clipboard history (tests)
//!   kadr --bench <n> [all] [--out <dir>]
//!                               time n capture+encode runs, save the last one

#![windows_subsystem = "windows"]

mod annotate;
mod app;
mod autostart;
mod capture;
mod clipboard;
mod editor;
mod editor_ui;
mod encode;
mod hotkeys;
mod install;
mod installer;
mod overlay;
mod settings;
mod settings_window;
mod snipping;
mod storage;
mod tray;
mod ui;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::core::w;

/// provisional product name; the only place it is spelled out in code.
pub const APP_NAME: &str = "kadr";

static TRACE: AtomicBool = AtomicBool::new(false);

/// timing/diagnostic line. never prints capture contents.
#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        if $crate::tracing() {
            eprintln!("[kadr] {}", format!($($arg)*));
        }
    };
}

pub fn tracing() -> bool {
    TRACE.load(Ordering::Relaxed)
}

/// null-terminated utf-16 for win32 calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

struct Args {
    trace: bool,
    settings: Option<PathBuf>,
    settings_window: Option<PathBuf>,
    app_window: Option<usize>,
    install: bool,
    uninstall: bool,
    finish_uninstall: Option<PathBuf>,
    wait_pid: Option<u32>,
    private_clipboard: bool,
    editor: Option<usize>,
    ready: Option<usize>,
    at: Option<capture::Rect>,
    bench: Option<u32>,
    all: bool,
    out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { trace: false, settings: None, settings_window: None, app_window: None, install: false, uninstall: false, finish_uninstall: None, wait_pid: None, private_clipboard: false, editor: None, ready: None, at: None, bench: None, all: false, out: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--trace" => a.trace = true,
            "--test-private-clipboard" => a.private_clipboard = true,
            "--install" => a.install = true,
            "--uninstall" => a.uninstall = true,
            "--finish-uninstall" => a.finish_uninstall = Some(it.next().ok_or("--finish-uninstall needs a folder")?.into()),
            "--wait" => a.wait_pid = it.next().and_then(|v| v.parse().ok()),
            "--settings-window" => a.settings_window = Some(it.next().ok_or("--settings-window needs a file")?.into()),
            "--app" => a.app_window = it.next().and_then(|v| v.parse().ok()),
            "--editor" => a.editor = Some(it.next().and_then(|v| v.parse().ok()).ok_or("--editor needs a handle")?),
            "--ready" => a.ready = Some(it.next().and_then(|v| v.parse().ok()).ok_or("--ready needs a handle")?),
            "--at" => a.at = Some(it.next().as_deref().and_then(editor::parse_at).ok_or("--at needs x,y,width,height")?),
            "--settings" => a.settings = Some(it.next().ok_or("--settings needs a file")?.into()),
            "--bench" => {
                let n = it.next().ok_or("--bench needs a number")?;
                a.bench = Some(n.parse().ok().filter(|n| *n > 0).ok_or("--bench needs a number > 0")?);
            }
            "all" => a.all = true,
            "--out" => a.out = Some(it.next().ok_or("--out needs a folder")?.into()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(a)
}

fn main() {
    // the manifest already declares per-monitor-v2; this call is a fallback
    // and fails harmlessly when the manifest has applied.
    // SAFETY: process-wide setting with a constant argument.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            app::error_box(None, &e);
            std::process::exit(2);
        }
    };
    if args.trace || args.bench.is_some() {
        // a gui-subsystem exe has no console; borrow the parent's only when
        // stdout/stderr were not redirected (`kadr --trace 2> file` from cmd):
        // attaching would point them at the console instead of the file.
        // SAFETY: handle queries and a constant argument.
        unsafe {
            let redirected = |which| GetStdHandle(which).is_ok_and(|h| !h.is_invalid() && !h.0.is_null());
            if !redirected(STD_ERROR_HANDLE) && !redirected(STD_OUTPUT_HANDLE) {
                let _ = AttachConsole(ATTACH_PARENT_PROCESS);
            }
        }
        TRACE.store(true, Ordering::Relaxed);
    }

    // SAFETY: paired with CoUninitialize below. wic needs com on this thread.
    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
        app::error_box(None, "kadr could not start: windows did not let it set up its image tools (com). try again, or restart windows.");
        std::process::exit(1);
    }

    let setup_name = std::env::current_exe()
        .ok()
        .and_then(|e| e.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .is_some_and(|s| install::is_setup_name(&s));
    let code = if let Some(dir) = &args.finish_uninstall {
        install::finish_uninstall(dir, args.wait_pid)
    } else if args.uninstall {
        uninstall()
    } else if args.install || (setup_name && !install::running_installed()) {
        installer::run()
    } else if let (Some(section), Some(ready), Some(at)) = (args.editor, args.ready, args.at) {
        editor::run(editor::Args { section, ready, at }, args.settings, args.private_clipboard)
    } else if let Some(path) = args.settings_window {
        let app = args.app_window.map(|h| windows::Win32::Foundation::HWND(h as *mut _));
        settings_window::run(path, app)
    } else {
        match args.bench {
            Some(n) => bench(n, args.all, args.out),
            None => run_app(args.settings, args.private_clipboard),
        }
    };

    // SAFETY: matches the successful CoInitializeEx.
    unsafe { CoUninitialize() };
    std::process::exit(code);
}

/// windows settings › apps › kadr › uninstall.
fn uninstall() -> i32 {
    use windows::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONQUESTION, MB_OK, MB_YESNO, MessageBoxW};
    use windows::Win32::UI::WindowsAndMessaging::MB_DEFBUTTON2;
    let ask = |text: &str, default_no: bool| {
        let t = wide(text);
        let flags = MB_YESNO | MB_ICONQUESTION | if default_no { MB_DEFBUTTON2 } else { Default::default() };
        // SAFETY: valid strings; no owner window.
        unsafe { MessageBoxW(None, windows::core::PCWSTR(t.as_ptr()), w!("kadr"), flags) == IDYES }
    };
    if !ask(
        "remove kadr?\n\nthe program, its start menu entry and start with windows are removed. your captures in pictures\\kadr stay.",
        false,
    ) {
        return 0;
    }
    // a separate question, and "no" is the default: settings are only
    // deleted when the user explicitly says so.
    let remove_settings = install::paths().settings_dir.exists()
        && ask("also remove your kadr settings?\n\nhotkeys, folder and file name. choose no to keep them for a later install.", true);
    let (text, code) = match install::uninstall(remove_settings) {
        Ok(()) => ("kadr is removed.".to_string(), 0),
        Err(e) => (format!("kadr could not be removed completely\n\n{e}"), 1),
    };
    let t = wide(&text);
    // SAFETY: as above.
    unsafe {
        MessageBoxW(None, windows::core::PCWSTR(t.as_ptr()), w!("kadr"), MB_OK);
    }
    code
}

fn run_app(settings: Option<PathBuf>, private_clipboard: bool) -> i32 {
    // one instance per user session: two copies would fight over hotkeys.
    // SAFETY: the handle is intentionally kept open until the process exits.
    let mutex = unsafe { CreateMutexW(None, false, w!("Local\\kadr.instance")) };
    if mutex.is_err() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        app::error_box(None, "kadr is already running. look for it in the notification area.");
        return 0;
    }
    trace!("start, memory {}", memory_summary());
    let path = settings.or_else(|| storage::settings_path().ok());
    let code = app::run(path, private_clipboard);
    trace!("exit, memory {}", memory_summary());
    code
}

/// milliseconds since this process was created (includes loader time), for
/// cold-start measurements in trace output.
pub fn ms_since_process_start() -> f64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
    use windows::Win32::System::Threading::GetProcessTimes;
    let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: out-pointers to locals; pseudo-handle of the current process.
    unsafe {
        if GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user).is_err() {
            return f64::NAN;
        }
        let now = GetSystemTimePreciseAsFileTime();
        let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
        (t(now).saturating_sub(t(created))) as f64 / 10_000.0
    }
}

/// working set and private bytes of this process, for trace output.
pub fn memory_summary() -> String {
    let mut c = PROCESS_MEMORY_COUNTERS { cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    // SAFETY: pseudo-handle for the current process, out-pointer to a local.
    if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) }.is_err() {
        return "unknown".into();
    }
    let mib = |b: usize| b as f64 / (1024.0 * 1024.0);
    format!(
        "ws {:.1} MiB (peak {:.1}), private {:.1} MiB (peak {:.1})",
        mib(c.WorkingSetSize),
        mib(c.PeakWorkingSetSize),
        mib(c.PagefileUsage),
        mib(c.PeakPagefileUsage)
    )
}

fn bench(runs: u32, all: bool, out: Option<PathBuf>) -> i32 {
    let rect = if all { Ok(capture::virtual_screen()) } else { capture::monitor_at_cursor() };
    let Ok(rect) = rect else {
        eprintln!("could not find a monitor");
        return 1;
    };
    let mut grabs = Vec::new();
    let mut encodes = Vec::new();
    let mut last = Vec::new();
    for _ in 0..runs {
        let t = Instant::now();
        let frame = match capture::grab(rect) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("grab failed: {}", e.message());
                return 1;
            }
        };
        grabs.push(t.elapsed());
        let t = Instant::now();
        match encode::png(&frame) {
            Ok(png) => last = png,
            Err(e) => {
                eprintln!("encode failed: {}", e.message());
                return 1;
            }
        }
        encodes.push(t.elapsed());
    }
    let dir = match out.map(Ok).unwrap_or_else(storage::default_capture_dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match storage::save_png(&dir, &storage::file_stem("kadr_%Y-%m-%d_%H-%M-%S"), &last) {
        Ok(p) => println!("saved {} ({}x{}, {} KiB)", p.display(), rect.width, rect.height, last.len() / 1024),
        Err(e) => {
            eprintln!("save failed: {e}");
            return 1;
        }
    }
    println!("grab   {}", stats(&grabs));
    println!("encode {}", stats(&encodes));
    println!("memory {}", memory_summary());
    0
}

fn stats(times: &[Duration]) -> String {
    let mut v: Vec<f64> = times.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    v.sort_by(f64::total_cmp);
    format!("n={} min {:.1} ms, median {:.1} ms, max {:.1} ms", v.len(), v[0], v[v.len() / 2], v[v.len() - 1])
}
