//! where files live and how captures are written.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::{
    FOLDERID_Pictures, FOLDERID_RoamingAppData, FOLDERID_Screenshots, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::core::GUID;

use crate::APP_NAME;

pub fn known_folder(id: &GUID) -> io::Result<PathBuf> {
    // SAFETY: the returned string is copied, then freed with CoTaskMemFree.
    unsafe {
        let raw = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).map_err(io::Error::other)?;
        let path = raw.to_string();
        CoTaskMemFree(Some(raw.0 as *const _));
        Ok(PathBuf::from(path.map_err(io::Error::other)?))
    }
}

/// default capture folder: `<pictures>\kadr`. created on first save.
/// where captures go by default: windows' own screenshots folder
/// (pictures\screenshots, wherever the user moved it), so they sit with the
/// ones windows makes. pictures\kadr only if windows has no such folder.
pub fn default_capture_dir() -> io::Result<PathBuf> {
    known_folder(&FOLDERID_Screenshots).or_else(|_| Ok(known_folder(&FOLDERID_Pictures)?.join(APP_NAME)))
}

/// `%APPDATA%\kadr\settings.ini`. neither is created until settings are opened.
pub fn settings_path() -> io::Result<PathBuf> {
    // test-only: an install sandbox (install.rs) keeps settings inside it.
    if let Some(root) = std::env::var_os("KADR_TEST_SANDBOX") {
        return Ok(PathBuf::from(root).join("AppData").join(APP_NAME).join("settings.ini"));
    }
    Ok(known_folder(&FOLDERID_RoamingAppData)?.join(APP_NAME).join("settings.ini"))
}

/// the program the capture was taken from (the foreground window's process,
/// `chrome`, `claude`), for %P in file names. set once per capture: by kadr
/// before saving, and by the editor from its command line.
static APP: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn set_app(name: Option<&str>) {
    let clean: String = name.unwrap_or("").chars().filter(|c| !r#"\/:*?"<>|%"#.contains(*c) && !c.is_control()).collect();
    if let Ok(mut app) = APP.lock() {
        *app = clean.trim().trim_end_matches('.').to_lowercase();
    }
}

/// the program name for %P; `kadr` when it is not known.
pub fn app() -> String {
    APP.lock().ok().map(|a| a.clone()).filter(|a| !a.is_empty()).unwrap_or_else(|| crate::APP_NAME.to_string())
}

/// expands %Y %m %d %H %M %S with local time, %P with the program the capture
/// came from (and %% for a literal percent).
pub fn file_stem(pattern: &str) -> String {
    // SAFETY: no arguments, returns a value.
    let t = unsafe { GetLocalTime() };
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&format!("{:04}", t.wYear)),
            Some('m') => out.push_str(&format!("{:02}", t.wMonth)),
            Some('d') => out.push_str(&format!("{:02}", t.wDay)),
            Some('H') => out.push_str(&format!("{:02}", t.wHour)),
            Some('M') => out.push_str(&format!("{:02}", t.wMinute)),
            Some('S') => out.push_str(&format!("{:02}", t.wSecond)),
            Some('P') => out.push_str(&app()),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// writes `bytes` to `<dir>\<stem>.png`, adding `_2`, `_3`, … if that name
/// exists. never overwrites. returns the final path.
pub fn save_png(dir: &Path, stem: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for n in 1..1000 {
        let name = if n == 1 { format!("{stem}.png") } else { format!("{stem}_{n}.png") };
        let path = dir.join(name);
        // create_new fails if the file exists, so two captures with the same
        // name cannot overwrite each other.
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(err) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                    drop(file);
                    let _ = std::fs::remove_file(&path); // no half-written files
                    return Err(err);
                }
                return Ok(path);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many captures with the same name"))
}

/// encodes `frame` and writes it where `settings` say. returns the path.
pub fn save_frame(frame: &crate::capture::Frame, settings: &crate::settings::Settings) -> Result<PathBuf, String> {
    let png = crate::encode::png(frame).map_err(|e| e.message())?;
    save_encoded(&png, settings)
}

/// writes an already encoded png where `settings` say.
pub fn save_encoded(png: &[u8], settings: &crate::settings::Settings) -> Result<PathBuf, String> {
    let base = match &settings.save_folder {
        Some(d) => d.clone(),
        None => default_capture_dir().map_err(|e| e.to_string())?,
    };
    // a dated subfolder (`%Y-%m` → 2026-11), created when needed
    let dir = if settings.subfolder.is_empty() { base } else { base.join(file_stem(&settings.subfolder)) };
    save_png(&dir, &file_stem(&settings.file_name), png).map_err(|e| format!("{} ({e})", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_expands_tokens() {
        let s = file_stem("kadr_%Y-%m-%d_%H-%M-%S");
        assert_eq!(s.len(), "kadr_2026-11-11_14-03-22".len());
        assert!(s.starts_with("kadr_20"));
        assert_eq!(file_stem("100%% %q %"), "100% %q %");
        set_app(Some("Materialgram"));
        assert_eq!(file_stem("%P_x"), "materialgram_x");
        set_app(None);
        assert_eq!(file_stem("%P"), crate::APP_NAME);
    }

    #[test]
    fn save_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("kadr-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = save_png(&dir, "x", b"one").unwrap();
        let b = save_png(&dir, "x", b"two").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&a).unwrap(), b"one");
        assert!(b.ends_with("x_2.png"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
