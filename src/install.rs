//! install and uninstall, for the current user only, without admin rights.
//!
//! the installer is kadr.exe itself: a copy named `kadr-...-setup.exe` (or
//! `kadr --install`) shows the installer window; the installed copy runs
//! normally. installing means:
//!   %LOCALAPPDATA%\Programs\kadr\kadr.exe
//!   start menu shortcut  %APPDATA%\Microsoft\Windows\Start Menu\Programs\kadr.lnk
//!   "apps" entry         HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\kadr
//!   optional autostart   HKCU\...\Run (autostart.rs)
//!
//! test-only: KADR_TEST_SANDBOX=<folder> puts every path under that folder and
//! uses "kadr-test" registry names, so tests never touch the real install.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, FOLDERID_Programs, IShellLinkW, ShellLink};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowThreadProcessId, PostMessageW, WM_CLOSE};
use windows::core::{Interface, PCWSTR, w};

use crate::{APP_NAME, trace, wide};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// the installer runs only for a file named like a release download:
/// `kadr-setup`, `kadr-<version>-setup`, optionally with the " (2)" a
/// browser adds to a second download. renaming kadr.exe to anything else
/// (kadr-portable, kadr-setup-old, setup) keeps it a normal kadr.
pub fn is_setup_name(stem: &str) -> bool {
    let mut s = stem.trim().to_ascii_lowercase();
    if let Some(open) = s.rfind(" (")
        && s.ends_with(')')
        && s[open + 2..s.len() - 1].chars().all(|c| c.is_ascii_digit())
        && s.len() > open + 3
    {
        s.truncate(open);
    }
    if s == "kadr-setup" || s == "kadr_setup" {
        return true;
    }
    let Some(middle) = s.strip_prefix("kadr-").and_then(|r| r.strip_suffix("-setup")) else { return false };
    // a version: digits, dots, letters and dashes, starting with a digit
    middle.starts_with(|c: char| c.is_ascii_digit()) && middle.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// uac on (the default). with uac off every process of an admin account is
/// elevated anyway, explorer included, so there is no normal token to hand on.
fn uac_enabled() -> bool {
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
    let mut value: u32 = 1;
    let mut size = 4u32;
    // SAFETY: out-pointers to locals of the size passed alongside.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Policies\\System"),
            w!("EnableLUA"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status.is_err() || value != 0
}

fn is_elevated() -> bool {
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // SAFETY: query on our own token, closed below.
    unsafe {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(token, TokenElevation, Some((&mut e as *mut TOKEN_ELEVATION).cast()), size_of::<TOKEN_ELEVATION>() as u32, &mut len).is_ok();
        let _ = CloseHandle(token);
        ok && e.TokenIsElevated != 0
    }
}

pub struct Paths {
    pub dir: PathBuf,
    pub exe: PathBuf,
    pub shortcut: PathBuf,
    pub settings_dir: PathBuf,
    pub uninstall_key: String,
}

fn sandbox() -> Option<PathBuf> {
    std::env::var_os("KADR_TEST_SANDBOX").map(PathBuf::from)
}

pub fn paths() -> Paths {
    let key_name = if sandbox().is_some() { format!("{APP_NAME}-test") } else { APP_NAME.to_string() };
    let uninstall_key = format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{key_name}");
    let (dir, programs, settings_dir) = match sandbox() {
        Some(root) => (root.join("Programs").join(APP_NAME), root.join("StartMenu"), root.join("AppData").join(APP_NAME)),
        None => (
            crate::storage::known_folder(&FOLDERID_LocalAppData).unwrap_or_default().join("Programs").join(APP_NAME),
            crate::storage::known_folder(&FOLDERID_Programs).unwrap_or_default(),
            crate::storage::settings_path().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default(),
        ),
    };
    Paths { exe: dir.join(format!("{APP_NAME}.exe")), shortcut: programs.join(format!("{APP_NAME}.lnk")), dir, settings_dir, uninstall_key }
}

/// the version recorded by a previous install, if any.
pub fn installed_version() -> Option<String> {
    let key = wide(&paths().uninstall_key);
    let mut buf = vec![0u16; 64];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: out-pointers to a local buffer of the size passed alongside.
    let status = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), w!("DisplayVersion"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
    };
    status.is_ok().then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
}

/// true when this process runs from the install folder.
pub fn running_installed() -> bool {
    std::env::current_exe().ok().is_some_and(|e| same_file(&e, &paths().exe))
}

fn same_file(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
}

/// asks a running kadr to quit and waits for it (up to 5 s). in a test
/// sandbox only the sandbox copy is touched.
pub fn close_running() -> Result<(), String> {
    close_editors()?;
    // SAFETY: plain window lookup, a posted close, and a wait on the process.
    unsafe {
        let Ok(hwnd) = FindWindowW(w!("kadr.main"), None) else { return Ok(()) };
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if sandbox().is_some() && !process_exe(pid).is_some_and(|e| same_file(&e, &paths().exe)) {
            return Ok(());
        }
        let process = OpenProcess(PROCESS_SYNCHRONIZE, false, pid).map_err(|e| e.message())?;
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        let exited = wait_pumping(process, 5000);
        let _ = CloseHandle(process);
        if !exited {
            return Err("the running kadr did not quit. quit it from the tray and try again.".into());
        }
    }
    Ok(())
}

/// open editors run from the same kadr.exe: each is asked to close (as
/// alt+f4 would), so the exe can be replaced or removed. an editor with
/// unsaved annotations asks the user first (save / don't save / cancel); the
/// installer waits for that answer (up to two minutes) and stops if the user
/// keeps the editor open, so no annotation is lost without a question.
fn close_editors() -> Result<(), String> {
    // SAFETY: window enumeration by class, posted closes, waits on processes.
    unsafe {
        let mut editors = Vec::new();
        let mut after: Option<HWND> = None;
        while let Ok(hwnd) = FindWindowExW(None, after, w!("kadr.editor"), PCWSTR::null()) {
            after = Some(hwnd);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if sandbox().is_some() && !process_exe(pid).is_some_and(|e| same_file(&e, &paths().exe)) {
                continue;
            }
            editors.push((hwnd, pid));
        }
        // one at a time, so each question is answered on its own
        for (hwnd, pid) in editors {
            let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else { continue };
            // its save question must come to the front, not open behind us
            let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            let closed = wait_for_editor(process, hwnd);
            let _ = CloseHandle(process);
            if !closed {
                return Err("an open kadr editor has unsaved annotations and was kept open. save or close it, then try again.".into());
            }
        }
    }
    Ok(())
}

/// true when the editor process exits. false at once when it asked about
/// unsaved changes (it marks its window with the "kadr.asking" property for
/// as long as the answer is pending or leads to closing) and the mark is gone
/// while the window is still open: the user chose cancel. at most two minutes.
fn wait_for_editor(process: windows::Win32::Foundation::HANDLE, hwnd: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetPropW, IsWindow};
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let mut asked = false;
    while std::time::Instant::now() < deadline {
        if wait_pumping(process, 100) {
            return true;
        }
        // SAFETY: plain queries on a window of another process.
        let (alive, marked) = unsafe { (IsWindow(Some(hwnd)).as_bool(), !GetPropW(hwnd, w!("kadr.asking")).is_invalid()) };
        if alive && marked {
            asked = true;
        } else if alive && asked {
            return false;
        }
    }
    false
}

/// waits for `process` while still answering window messages: a process that
/// sends this thread a message while quitting must not stall both of us.
fn wait_pumping(process: windows::Win32::Foundation::HANDLE, timeout_ms: u32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, PostQuitMessage,
        QS_ALLINPUT, TranslateMessage, WM_QUIT,
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
    let mut msg = MSG::default();
    // SAFETY: standard message pump around a wait on a handle we own.
    unsafe {
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now()).as_millis() as u32;
            if left == 0 {
                return WaitForSingleObject(process, 0) == WAIT_OBJECT_0;
            }
            let wait = MsgWaitForMultipleObjectsEx(Some(&[process]), left, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            if wait == WAIT_OBJECT_0 {
                return true;
            }
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    PostQuitMessage(msg.wParam.0 as i32);
                    return WaitForSingleObject(process, 0) == WAIT_OBJECT_0;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

fn process_exe(pid: u32) -> Option<PathBuf> {
    use windows::Win32::System::Threading::{PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    // SAFETY: query on a handle we open and close here.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        ok.then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

pub struct Options {
    pub autostart: bool,
    pub run: bool,
}

/// copies this exe into place, adds the shortcut and the apps entry, sets
/// autostart, and starts kadr if asked. each step says what failed.
pub fn install(options: &Options) -> Result<(), String> {
    let p = paths();
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    close_running()?;
    std::fs::create_dir_all(&p.dir).map_err(|e| format!("could not create {}: {e}", p.dir.display()))?;
    if !same_file(&me, &p.exe) {
        // copy next to the target, then replace it in one step.
        let staging = p.exe.with_extension("exe.new");
        std::fs::copy(&me, &staging).map_err(|e| format!("could not copy kadr: {e}"))?;
        // right after the old kadr exits, its exe can stay locked for a moment
        // (antivirus scans, the image being unmapped): retry for up to 5 s.
        let mut tries = 0;
        loop {
            match std::fs::rename(&staging, &p.exe) {
                Ok(()) => break,
                Err(_) if tries < 50 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&staging);
                    return Err(format!("could not replace {}: {e}", p.exe.display()));
                }
            }
        }
        if tries > 0 {
            trace!("installer: exe was locked, replaced after {tries} retries");
        }
    }
    make_shortcut(&p.exe, &p.shortcut).map_err(|e| format!("could not create the start menu shortcut: {e}"))?;
    write_uninstall_entry(&p).map_err(|e| format!("could not register kadr in apps: {e}"))?;
    crate::autostart::set(&p.exe, options.autostart).map_err(|e| format!("could not change start with windows: {e}"))?;
    trace!("installed {VERSION} to {}", p.dir.display());
    if options.run {
        // SAFETY: lets the new kadr take the foreground for its first window.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(windows::Win32::UI::WindowsAndMessaging::ASFW_ANY);
        }
        if is_elevated() && uac_enabled() {
            // started from an elevated installer (run as administrator), kadr
            // would inherit admin rights. explorer starts it with the normal
            // user token instead (with uac off, explorer itself is elevated
            // and nothing changes).
            std::process::Command::new("explorer.exe").arg(&p.exe).spawn().map_err(|e| format!("installed, but could not start kadr: {e}"))?;
        } else {
            std::process::Command::new(&p.exe).spawn().map_err(|e| format!("installed, but could not start kadr: {e}"))?;
        }
    }
    Ok(())
}

fn make_shortcut(target: &Path, at: &Path) -> windows::core::Result<()> {
    if let Some(dir) = at.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let target_w = wide(&target.display().to_string());
    let dir_w = wide(&target.parent().unwrap_or(Path::new("")).display().to_string());
    let at_w = wide(&at.display().to_string());
    // SAFETY: com objects created and released in this scope; strings outlive the calls.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(target_w.as_ptr()))?;
        link.SetWorkingDirectory(PCWSTR(dir_w.as_ptr()))?;
        link.SetDescription(w!("screenshots, without the extra stuff"))?;
        link.SetIconLocation(PCWSTR(target_w.as_ptr()), 0)?;
        link.cast::<IPersistFile>()?.Save(PCWSTR(at_w.as_ptr()), true)
    }
}

fn write_uninstall_entry(p: &Paths) -> windows::core::Result<()> {
    let key_w = wide(&p.uninstall_key);
    let mut key = HKEY::default();
    // SAFETY: the key is created/opened for writing and closed below.
    unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(key_w.as_ptr()), None, None, REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut key, None).ok()?;
        let exe = p.exe.display().to_string();
        let size_kb = std::fs::metadata(&p.exe).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
        let strings = [
            ("DisplayName", APP_NAME.to_string()),
            ("DisplayVersion", VERSION.to_string()),
            ("Publisher", "lezneve".to_string()),
            ("DisplayIcon", exe.clone()),
            ("InstallLocation", p.dir.display().to_string()),
            ("UninstallString", format!("\"{exe}\" --uninstall")),
        ];
        let mut result = Ok(());
        for (name, value) in strings {
            let n = wide(name);
            let v = wide(&value);
            let bytes = std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 2);
            if let Err(e) = RegSetValueExW(key, PCWSTR(n.as_ptr()), None, REG_SZ, Some(bytes)).ok() {
                result = Err(e);
            }
        }
        for (name, value) in [("NoModify", 1u32), ("NoRepair", 1), ("EstimatedSize", size_kb)] {
            let n = wide(name);
            let _ = RegSetValueExW(key, PCWSTR(n.as_ptr()), None, REG_DWORD, Some(&value.to_le_bytes()));
        }
        let _ = RegCloseKey(key);
        result
    }
}

/// removes kadr: quits it, removes autostart, the shortcut, the apps entry
/// and the program folder; the settings only when `remove_settings` (the
/// user confirmed it separately). captures are left where they are.
pub fn uninstall(remove_settings: bool) -> Result<(), String> {
    let p = paths();
    close_running()?;
    crate::autostart::set(&p.exe, false).map_err(|e| format!("could not remove start with windows: {e}"))?;
    let _ = std::fs::remove_file(&p.shortcut);
    let key = wide(&p.uninstall_key);
    // SAFETY: deletes our own key; a missing key is fine.
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()));
    }
    // if kadr turned off windows' printscreen → snipping tool, turn it back on
    // (the consent dialog says so). a failure here must not stop the rest of
    // the uninstall half-way; it is reported at the end.
    let restored = crate::snipping::restore();
    if remove_settings && p.settings_dir.ends_with(APP_NAME) {
        let _ = std::fs::remove_dir_all(&p.settings_dir);
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    trace!("uninstall: running from {} (installed {})", me.display(), p.exe.display());
    if same_file(&me, &p.exe) {
        // a running exe cannot delete itself: a hidden cmd removes the folder
        // two seconds after this process has exited.
        remove_later(&p.dir)?;
    } else {
        let _ = std::fs::remove_dir_all(&p.dir);
    }
    trace!("uninstalled from {}", p.dir.display());
    restored.map_err(|e| {
        format!("everything else is removed, but the windows setting \"use the print screen key to open screen capture\" could not be turned back on: {e}. turn it on in windows settings › accessibility › keyboard if you want it.")
    })
}

/// a running exe cannot delete its own folder. a copy of this exe in %TEMP%
/// waits for this process to exit and removes the folder (with retries: the
/// exe can stay locked for a moment after exit). no console and no cmd: a
/// hidden cmd proved unreliable under a standard-user token. the small copy
/// stays in %TEMP% for windows' own temp clean-up, as common uninstallers do.
fn remove_later(dir: &Path) -> Result<(), String> {
    check_removable(dir)?;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let helper = std::env::temp_dir().join(format!("{APP_NAME}-uninstall-{}.exe", std::process::id()));
    std::fs::copy(&me, &helper).map_err(|e| format!("could not prepare the removal of {}: {e}", dir.display()))?;
    std::process::Command::new(&helper)
        .arg("--finish-uninstall")
        .arg(dir)
        .arg("--wait")
        .arg(std::process::id().to_string())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not remove {}: {e}", dir.display()))
}

/// only a folder named kadr, never anything shorter or elsewhere by accident.
fn check_removable(dir: &Path) -> Result<(), String> {
    let s = dir.display().to_string();
    if s.len() < 10 || !s.to_ascii_lowercase().ends_with(&format!("\\{APP_NAME}")) || !dir.is_absolute() {
        return Err(format!("refusing to remove an unexpected folder: {s}"));
    }
    Ok(())
}

/// `kadr --finish-uninstall <dir> --wait <pid>`: the second half of an
/// uninstall, run from the temp copy.
pub fn finish_uninstall(dir: &Path, wait_pid: Option<u32>) -> i32 {
    if check_removable(dir).is_err() {
        return 2;
    }
    if let Some(pid) = wait_pid {
        // SAFETY: wait on the uninstaller's process handle, closed below.
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
                let _ = WaitForSingleObject(h, 30_000);
                let _ = CloseHandle(h);
            }
        }
    }
    for _ in 0..50 {
        if !dir.exists() || std::fs::remove_dir_all(dir).is_ok() {
            return 0;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    1
}

#[cfg(test)]
mod tests {
    use super::{check_removable, is_setup_name};
    use std::path::Path;

    #[test]
    fn only_a_kadr_folder_can_be_removed() {
        assert!(check_removable(Path::new(r"C:\Users\u\AppData\Local\Programs\kadr")).is_ok());
        for bad in [r"C:\", r"C:\kadr-old", r"kadr", r"C:\Users\u\AppData\Local\Programs", r"C:\Users\u\AppData\Local\Programs\kadrx", r"\\kadr"] {
            assert!(check_removable(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn setup_names() {
        for yes in ["kadr-0.1.0-setup", "kadr-0.1.0-rc.1-setup", "kadr-0.1.0-rc.1-setup (1)", "Kadr-0.2.0-Setup", "kadr-setup", "kadr_setup", "kadr-1.0.0-setup (12)"] {
            assert!(is_setup_name(yes), "{yes}");
        }
        for no in ["kadr", "kadr-portable", "kadr-setup-old", "setup", "mysetup", "kadr (setup)", "kadr-setup (copy)", "old-kadr-0.1.0-setup", "kadr-beta-setup", "kadr-0.1.0-rc.1-setup ()", "kadr-0.1.0-rc.1-setup (x)"] {
            assert!(!is_setup_name(no), "{no}");
        }
    }
}
