//! the resident part: a hidden window that owns the tray icon and hotkeys,
//! and a message loop that sleeps until windows has something for us.
//!
//! idle cost by design: no timers, no polling, no threads. the loop blocks in
//! MsgWaitForMultipleObjectsEx on window messages and, while settings are
//! being edited, on a folder change notification.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::Instant;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FindCloseChangeNotification,
    FindFirstChangeNotificationW, FindNextChangeNotification,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EndMenu, GWLP_USERDATA,
    GetWindowLongPtrW, KillTimer, MB_ICONWARNING, MB_OK, MSG, MWMO_INPUTAVAILABLE, MessageBoxW,
    MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, PostQuitMessage, QS_ALLINPUT,
    RegisterClassW, RegisterWindowMessageW, SW_SHOWNORMAL, SetTimer, SetWindowLongPtrW,
    TranslateMessage, WINDOW_EX_STYLE, WM_CONTEXTMENU, WM_DESTROY, WM_ENDSESSION, WM_HOTKEY,
    WM_QUIT, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::capture::{self, Frame};
use crate::hotkeys::{self, Action};
use crate::settings::Settings;
use crate::tray::{self, Command, Tray};
use crate::{APP_NAME, clipboard, overlay, settings_window, storage, trace, wide};

const CLASS: PCWSTR = w!("kadr.main");
/// one-shot timers. tray captures wait for the menu fade-out so it is not
/// in the picture; settings reloads wait for the editor to finish writing.
const TIMER_TRAY_REGION: usize = 1;
const TIMER_TRAY_SCREEN: usize = 2;
const TIMER_RELOAD: usize = 3;
const TRAY_DELAY_MS: u32 = 250;
const RELOAD_DELAY_MS: u32 = 300;
// NIN_SELECT / NIN_KEYSELECT (shellapi.h): left click / enter on the icon.
const NIN_SELECT: u32 = 0x0400;
const NIN_KEYSELECT: u32 = 0x0401;

struct App {
    hwnd: HWND,
    tray: RefCell<Option<Tray>>,
    settings: RefCell<Settings>,
    settings_path: Option<PathBuf>,
    watch: Cell<Option<HANDLE>>,
    busy: Cell<bool>,
    taskbar_created: u32,
    private_clipboard: bool,
    /// the settings process, while it runs (see settings_window.rs).
    settings_child: RefCell<Option<std::process::Child>>,
    /// true while the settings window records a new combination.
    hotkeys_suspended: Cell<bool>,
    /// a bare printscreen hotkey has been handled once in this process.
    printscreen_seen: Cell<bool>,
}

/// runs until the user quits. returns the process exit code.
pub fn run(settings_path: Option<PathBuf>, private_clipboard: bool) -> i32 {
    let (loaded, warnings) = match &settings_path {
        Some(path) => Settings::load(path),
        None => (Settings::default(), vec!["could not find the settings folder; using defaults".into()]),
    };

    // SAFETY: standard window creation; the App box is owned by the window
    // through GWLP_USERDATA and reclaimed after the loop ends.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return 1 };
        let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: CLASS, ..Default::default() };
        RegisterClassW(&class);
        // a hidden top-level window, not a message-only one: only top-level
        // windows receive the TaskbarCreated broadcast after explorer restarts.
        let hwnd = match CreateWindowExW(WINDOW_EX_STYLE(0), CLASS, w!("kadr"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance.into()), None) {
            Ok(hwnd) => hwnd,
            Err(err) => {
                error_box(None, &format!("could not start: {}", err.message()));
                return 1;
            }
        };

        let app = Box::new(App {
            hwnd,
            tray: RefCell::new(None),
            settings: RefCell::new(loaded),
            settings_path,
            watch: Cell::new(None),
            busy: Cell::new(false),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            private_clipboard,
            settings_child: RefCell::new(None),
            hotkeys_suspended: Cell::new(false),
            printscreen_seen: Cell::new(false),
        });
        let app_ptr = Box::into_raw(app);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize);
        let app = &*app_ptr;

        let mut problems = warnings;
        match Tray::add(hwnd) {
            Ok(t) => *app.tray.borrow_mut() = Some(t),
            // TaskbarCreated retries when explorer (re)starts. until then the
            // user must know there is no menu, and how to get out.
            Err(err) => {
                trace!("tray: {}", err.message());
                problems.push(format!(
                    "the tray icon could not be added ({}). hotkeys still work. to quit kadr, end it in task manager.",
                    err.message()
                ));
            }
        }
        problems.extend(hotkeys::register(hwnd, &app.settings.borrow().hotkeys()));
        // printscreen taken by snipping tool: ask once whether to free it. a
        // "no" is respected; the settings window keeps showing the conflict.
        if !hotkeys::snipping_conflicts(&app.settings.borrow().hotkeys()).is_empty() && !crate::snipping::asked() {
            crate::snipping::ask(None);
        }
        app.start_watch();
        if !problems.is_empty() {
            app.report("kadr started with problems", &problems);
        }
        trace!("ready {:.1} ms after process start", crate::ms_since_process_start());

        let code = message_loop(app);

        app.stop_watch();
        drop(Box::from_raw(app_ptr));
        code
    }
}

fn message_loop(app: &App) -> i32 {
    let mut msg = MSG::default();
    loop {
        // SAFETY: standard message pump; the change handle stays open until
        // stop_watch, which only runs on this thread.
        unsafe {
            let watch = app.watch.get();
            let child = app.settings_child.borrow().as_ref().map(|c| {
                use std::os::windows::io::AsRawHandle;
                HANDLE(c.as_raw_handle())
            });
            let handles: Vec<HANDLE> = watch.into_iter().chain(child).collect();
            let wait = MsgWaitForMultipleObjectsEx(Some(&handles), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            let child_index = watch.is_some() as u32;
            if child.is_some() && wait.0 == WAIT_OBJECT_0.0 + child_index {
                app.settings_exited();
            } else if watch.is_some() && wait == WAIT_OBJECT_0 {
                // the folder changed: debounce, then re-arm the notification.
                SetTimer(Some(app.hwnd), TIMER_RELOAD, RELOAD_DELAY_MS, None);
                if let Some(h) = watch
                    && FindNextChangeNotification(h).is_err()
                {
                    app.stop_watch();
                }
            }
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return msg.wParam.0 as i32;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: GWLP_USERDATA is the App pointer set right after creation and
    // valid until run() reclaims it after the loop (the window is gone then).
    let app = unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const App).as_ref() };
    let Some(app) = app else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    match msg {
        WM_HOTKEY => {
            if let Some(action) = Action::from_id(wparam.0) {
                trace!("hotkey {}", action.label());
                // a real bare printscreen arrived: once, record that windows
                // applied the change kadr made (snipping.rs).
                let bare = app.settings.borrow().hotkey(action).is_some_and(|k| k.is_bare_printscreen());
                if bare && !app.printscreen_seen.replace(true) {
                    crate::snipping::confirm_received();
                }
                app.capture(action, Instant::now());
            }
            LRESULT(0)
        }
        tray::WM_TRAY => {
            // NOTIFYICON_VERSION_4: event in the low word of lparam, anchor
            // point in wparam.
            let event = (lparam.0 & 0xffff) as u32;
            match event {
                NIN_SELECT | NIN_KEYSELECT => unsafe {
                    SetTimer(Some(hwnd), TIMER_TRAY_REGION, TRAY_DELAY_MS, None);
                },
                WM_CONTEXTMENU => {
                    let at = POINT { x: (wparam.0 & 0xffff) as i16 as i32, y: ((wparam.0 >> 16) & 0xffff) as i16 as i32 };
                    app.show_menu(at);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_TIMER => {
            // SAFETY: our own one-shot timers.
            unsafe {
                let _ = KillTimer(Some(hwnd), wparam.0);
            }
            match wparam.0 {
                TIMER_TRAY_REGION => app.capture(Action::Region, Instant::now()),
                TIMER_TRAY_SCREEN => app.capture(Action::Screen, Instant::now()),
                TIMER_RELOAD => app.reload(),
                _ => {}
            }
            LRESULT(0)
        }
        settings_window::WM_KADR_HOTKEYS => {
            if wparam.0 == 0 {
                app.hotkeys_suspended.set(true);
                hotkeys::unregister(hwnd);
            } else {
                app.hotkeys_suspended.set(false);
                app.reload_and_register();
            }
            LRESULT(0)
        }
        settings_window::WM_KADR_SETTINGS => {
            app.reload();
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            // taskbar theme switched (light/dark): redraw the icon. these
            // arrive in bursts and Shell_NotifyIconW can let another one in,
            // so a re-entrant call skips instead of double-borrowing (which
            // would abort the process).
            if let Ok(mut tray) = app.tray.try_borrow_mut()
                && let Some(tray) = tray.as_mut()
            {
                let _ = tray.refresh_icon();
            }
            LRESULT(0)
        }
        WM_ENDSESSION if wparam.0 != 0 => {
            app.shutdown();
            LRESULT(0)
        }
        WM_DESTROY => {
            app.shutdown();
            // SAFETY: ends the outer message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        m if m == app.taskbar_created && m != 0 => {
            // same re-entrancy rule as above.
            if let Ok(mut tray) = app.tray.try_borrow_mut() {
                match tray.as_ref() {
                    Some(t) => {
                        let _ = t.readd();
                    }
                    None => *tray = Tray::add(hwnd).ok(),
                }
            }
            LRESULT(0)
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

impl App {
    fn show_menu(&self, at: POINT) {
        let label = |text: &str, action: Option<Action>| {
            let key = action.and_then(|a| self.settings.borrow().hotkey(a));
            match key {
                Some(k) => format!("{text}\t{k}"),
                None => text.to_string(),
            }
        };
        let items = [
            (Command::Region, label("capture region", Some(Action::Region))),
            (Command::Screen, label("capture screen", Some(Action::Screen))),
            (Command::OpenFolder, "open folder".to_string()),
            (Command::Settings, "settings".to_string()),
            (Command::Quit, "quit".to_string()),
        ];
        let chosen = tray::menu(self.hwnd, at, &items);
        match chosen {
            // SAFETY: one-shot timers on our window.
            Some(Command::Region) => unsafe {
                SetTimer(Some(self.hwnd), TIMER_TRAY_REGION, TRAY_DELAY_MS, None);
            },
            Some(Command::Screen) => unsafe {
                SetTimer(Some(self.hwnd), TIMER_TRAY_SCREEN, TRAY_DELAY_MS, None);
            },
            Some(Command::OpenFolder) => self.open_folder(),
            Some(Command::Settings) => self.open_settings(),
            Some(Command::Quit) => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
            None => {}
        }
    }

    fn capture(&self, action: Action, started: Instant) {
        if self.busy.replace(true) {
            trace!("busy, ignored");
            return;
        }
        // SAFETY: closes our tray menu if a hotkey arrives while it is open.
        unsafe {
            let _ = EndMenu();
        }
        let settings = self.settings.borrow().clone();
        let result = self.capture_inner(action, &settings, started);
        self.busy.set(false);
        if let Err(problem) = result {
            self.report("capture failed", &[problem]);
        }
        trace!("memory {}", crate::memory_summary());
    }

    fn capture_inner(&self, action: Action, settings: &Settings, started: Instant) -> Result<(), String> {
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
        let frame = match action {
            Action::Region => {
                let screen = capture::virtual_screen();
                let full = capture::grab(screen).map_err(|e| format!("could not read the screen: {}", e.message()))?;
                trace!("grab {}x{} at {},{}: {:.1} ms", screen.width, screen.height, screen.x, screen.y, ms(started));
                // editor mode: the editor starts while the overlay is still up
                // and the overlay goes once the editor's first frame is shown.
                let mut edited: Option<Result<(), String>> = None;
                let selected = overlay::select(
                    &full,
                    screen,
                    || trace!("overlay shown: {:.1} ms after hotkey, memory {}", ms(started), crate::memory_summary()),
                    |rect| {
                        if settings.edit {
                            let at = capture::Rect { x: screen.x + rect.x, y: screen.y + rect.y, ..rect };
                            edited = Some(full.crop(rect).map_err(|e| e.message()).and_then(|part| self.edit(&part, at, true, started)));
                        }
                    },
                )
                .map_err(|e| format!("could not show the selection overlay: {}", e.message()))?;
                let Some(rect) = selected else {
                    trace!("cancelled");
                    return Ok(());
                };
                if let Some(result) = edited {
                    return result;
                }
                let t = Instant::now();
                let part = full.crop(rect).map_err(|e| e.message())?;
                trace!("selected {}x{} at {},{} (crop {:.1} ms)", rect.width, rect.height, rect.x, rect.y, ms(t));
                part // `full` is dropped here, before encoding
            }
            Action::Window => {
                let rect = match capture::foreground_window_rect() {
                    Some(r) => r,
                    None => {
                        trace!("no capturable foreground window; using the monitor");
                        capture::monitor_at_cursor().map_err(|e| e.message())?
                    }
                };
                let f = capture::grab(rect).map_err(|e| format!("could not read the screen: {}", e.message()))?;
                trace!("window {}x{} at {},{}: {:.1} ms", rect.width, rect.height, rect.x, rect.y, ms(started));
                if settings.edit {
                    return self.edit(&f, rect, false, started);
                }
                f
            }
            Action::Screen => {
                let rect = capture::monitor_at_cursor().map_err(|e| e.message())?;
                let f = capture::grab(rect).map_err(|e| format!("could not read the screen: {}", e.message()))?;
                trace!("screen {}x{} at {},{}: {:.1} ms", rect.width, rect.height, rect.x, rect.y, ms(started));
                if settings.edit {
                    return self.edit(&f, rect, false, started);
                }
                f
            }
        };
        self.deliver(&frame, settings)
    }

    /// opens the editor on `frame`, captured at `at`. nothing is copied or
    /// saved here. `wait`: keep the caller's overlay up until the editor's
    /// first frame is on screen (at most 1.5 s).
    fn edit(&self, frame: &Frame, at: capture::Rect, wait: bool, started: Instant) -> Result<(), String> {
        let t = Instant::now();
        let launched = crate::editor::launch(frame, at, self.settings_path.as_deref(), self.private_clipboard)?;
        if wait {
            let shown = launched.wait_shown(1500);
            trace!(
                "editor {} {:.1} ms after selection, {:.1} ms after hotkey",
                if shown { "shown" } else { "not shown in time" },
                t.elapsed().as_secs_f64() * 1000.0,
                started.elapsed().as_secs_f64() * 1000.0
            );
        } else {
            launched.detach();
        }
        Ok(())
    }

    /// clipboard first (instant feedback), then the file.
    fn deliver(&self, frame: &Frame, settings: &Settings) -> Result<(), String> {
        let mut problems = Vec::new();
        if settings.copy {
            let t = Instant::now();
            match clipboard::copy(self.hwnd, frame, self.private_clipboard) {
                Ok(()) => trace!("clipboard {:.1} ms", t.elapsed().as_secs_f64() * 1000.0),
                Err(e) => problems.push(format!("could not copy to the clipboard: {}", e.message())),
            }
        }
        if settings.save {
            let t = Instant::now();
            match storage::save_frame(frame, settings) {
                Ok(path) => trace!("saved {} in {:.1} ms", path.display(), t.elapsed().as_secs_f64() * 1000.0),
                Err(e) => problems.push(format!("could not save the capture: {e}")),
            }
        }
        if problems.is_empty() { Ok(()) } else { Err(problems.join("\n")) }
    }

    fn open_folder(&self) {
        let settings = self.settings.borrow().clone();
        let dir = match settings.save_folder {
            Some(d) => Ok(d),
            None => storage::default_capture_dir(),
        };
        let result = dir.and_then(|d| std::fs::create_dir_all(&d).map(|_| d));
        match result {
            Ok(dir) => shell_open(self.hwnd, &dir.display().to_string()),
            Err(e) => self.report("could not open the folder", &[e.to_string()]),
        }
    }

    fn open_settings(&self) {
        let Some(path) = &self.settings_path else {
            self.report("settings", &["the settings folder is not available".into()]);
            return;
        };
        // already open: bring it forward instead of starting another.
        if self.settings_child.borrow().is_some() {
            // SAFETY: plain window lookup and activation.
            unsafe {
                if let Ok(w) = windows::Win32::UI::WindowsAndMessaging::FindWindowW(windows::core::w!("kadr.settings"), None) {
                    let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(w);
                }
            }
            return;
        }
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => return self.report("could not open settings", &[e.to_string()]),
        };
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--settings-window").arg(path).arg("--app").arg((self.hwnd.0 as usize).to_string());
        if crate::tracing() {
            cmd.arg("--trace");
        }
        // SAFETY: lets the child take the foreground; kadr may give it away
        // because the user just clicked its tray icon.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(windows::Win32::UI::WindowsAndMessaging::ASFW_ANY);
        }
        match cmd.spawn() {
            Ok(child) => {
                trace!("settings process {} started", child.id());
                *self.settings_child.borrow_mut() = Some(child);
            }
            Err(e) => self.report("could not open settings", &[e.to_string()]),
        }
    }

    /// asks an open settings window to close (it saves a pending file name),
    /// waits up to 3 s while still answering its messages, then ends it.
    fn close_settings(&self) {
        let child = self.settings_child.try_borrow_mut().ok().and_then(|mut c| c.take());
        let Some(mut child) = child else { return };
        use std::os::windows::io::AsRawHandle;
        let handle = HANDLE(child.as_raw_handle());
        // SAFETY: our own child process and window; messages are pumped so the
        // child's SendMessageTimeout calls to us are answered, not deadlocked.
        unsafe {
            if let Ok(w) = windows::Win32::UI::WindowsAndMessaging::FindWindowW(w!("kadr.settings"), None) {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(Some(w), windows::Win32::UI::WindowsAndMessaging::WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            let deadline = Instant::now() + std::time::Duration::from_secs(3);
            let mut msg = MSG::default();
            while Instant::now() < deadline {
                let left = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
                let wait = MsgWaitForMultipleObjectsEx(Some(&[handle]), left, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                if wait == WAIT_OBJECT_0 {
                    break;
                }
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        PostQuitMessage(msg.wParam.0 as i32);
                        break;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        if !matches!(child.try_wait(), Ok(Some(_))) {
            trace!("settings process did not close in time; ending it");
            let _ = child.kill();
        }
        let _ = child.wait();
        trace!("settings process closed on shutdown");
    }

    /// the settings process ended (closed, or crashed). if it was recording a
    /// hotkey, kadr's hotkeys must come back either way.
    fn settings_exited(&self) {
        let child = self.settings_child.borrow_mut().take();
        if let Some(mut child) = child {
            let code = child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
            trace!("settings process exited with {code}, memory {}", crate::memory_summary());
        }
        if self.hotkeys_suspended.replace(false) {
            self.reload_and_register();
        }
    }

    /// after recording a hotkey: read the file and register every hotkey anew.
    fn reload_and_register(&self) {
        if let Some(path) = &self.settings_path {
            let (next, _) = Settings::load(path);
            *self.settings.borrow_mut() = next;
        }
        hotkeys::unregister(self.hwnd);
        let mut problems = hotkeys::register(self.hwnd, &self.settings.borrow().hotkeys());
        problems.extend(hotkeys::snipping_conflicts(&self.settings.borrow().hotkeys()));
        trace!("hotkeys registered again, {} problem(s)", problems.len());
    }

    fn reload(&self) {
        let Some(path) = &self.settings_path else { return };
        let (next, mut problems) = Settings::load(path);
        if *self.settings.borrow() == next && problems.is_empty() {
            return; // e.g. the editor touched the file without changes
        }
        let hotkeys_changed = self.settings.borrow().hotkeys() != next.hotkeys();
        *self.settings.borrow_mut() = next;
        if hotkeys_changed && !self.hotkeys_suspended.get() {
            hotkeys::unregister(self.hwnd);
            problems.extend(hotkeys::register(self.hwnd, &self.settings.borrow().hotkeys()));
            problems.extend(hotkeys::snipping_conflicts(&self.settings.borrow().hotkeys()));
        }
        trace!("settings reloaded, {} problem(s)", problems.len());
        if !problems.is_empty() {
            self.report("settings have problems", &problems);
        }
    }

    /// watches the settings folder, if it exists, for edits.
    fn start_watch(&self) {
        if self.watch.get().is_some() {
            return;
        }
        let Some(dir) = self.settings_path.as_ref().and_then(|p| p.parent()) else { return };
        if !dir.exists() {
            return;
        }
        let dir = wide(&dir.display().to_string());
        // SAFETY: valid null-terminated path; closed in stop_watch.
        let handle = unsafe {
            FindFirstChangeNotificationW(PCWSTR(dir.as_ptr()), false, FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME)
        };
        if let Ok(h) = handle {
            self.watch.set(Some(h));
        }
    }

    fn stop_watch(&self) {
        if let Some(h) = self.watch.take() {
            // SAFETY: handle from FindFirstChangeNotificationW.
            unsafe {
                let _ = FindCloseChangeNotification(h);
            }
        }
    }

    /// closes the settings process, releases hotkeys and removes the tray
    /// icon. safe to call twice.
    fn shutdown(&self) {
        self.close_settings();
        hotkeys::unregister(self.hwnd);
        // take the tray out first, then drop it (NIM_DELETE) with no borrow
        // held, in case removing the icon lets another message in.
        let tray = self.tray.try_borrow_mut().ok().and_then(|mut t| t.take());
        drop(tray);
    }

    fn report(&self, title: &str, lines: &[String]) {
        trace!("{title}: {}", lines.join(" | "));
        error_box(Some(self.hwnd), &format!("{title}\n\n{}", lines.join("\n")));
    }
}

fn shell_open(hwnd: HWND, target: &str) {
    let target = wide(target);
    // SAFETY: valid null-terminated strings for the duration of the call.
    unsafe {
        ShellExecuteW(Some(hwnd), w!("open"), PCWSTR(target.as_ptr()), None, None, SW_SHOWNORMAL);
    }
}

pub fn error_box(owner: Option<HWND>, text: &str) {
    let text = wide(text);
    let caption = wide(APP_NAME);
    // SAFETY: valid null-terminated strings.
    unsafe {
        MessageBoxW(owner, PCWSTR(text.as_ptr()), PCWSTR(caption.as_ptr()), MB_OK | MB_ICONWARNING);
    }
}
