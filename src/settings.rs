//! user settings: a small `key = value` text file.
//!
//! no serde: eight values do not need a serialization framework, and a
//! hand-checked format gives precise per-line error messages. the file only
//! exists after the user opens settings for the first time.

use std::io;
use std::path::{Path, PathBuf};

use crate::hotkeys::{Action, Hotkey};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub hotkey_region: Option<Hotkey>,
    pub hotkey_window: Option<Hotkey>,
    pub hotkey_screen: Option<Hotkey>,
    /// true: every capture opens in the editor; nothing is copied or saved
    /// until the user does it there. false: the quick mode below.
    pub edit: bool,
    pub copy: bool,
    pub save: bool,
    /// `None` = `<pictures>\kadr`
    pub save_folder: Option<PathBuf>,
    pub file_name: String,
    /// a subfolder pattern under the save folder, with the same tokens as the
    /// file name (`%Y-%m` → 2026-11). empty: none.
    pub subfolder: String,
    /// editor: ctrl+c copies and closes (no question: the result is on the
    /// clipboard). off: the editor stays open after copying.
    pub close_after_copy: bool,
}

// bare printscreen: on windows 11 snipping tool may take it even though
// RegisterHotKey succeeds; kadr then offers to turn that off (snipping.rs).
const DEFAULT_REGION: &str = "printscreen";
const DEFAULT_WINDOW: &str = "alt+printscreen";
const DEFAULT_SCREEN: &str = "ctrl+shift+printscreen";
const DEFAULT_FILE_NAME: &str = "kadr_%Y-%m-%d_%H-%M-%S";

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hotkey_region: Hotkey::parse(DEFAULT_REGION).ok(),
            hotkey_window: Hotkey::parse(DEFAULT_WINDOW).ok(),
            hotkey_screen: Hotkey::parse(DEFAULT_SCREEN).ok(),
            edit: true,
            copy: true,
            save: true,
            save_folder: None,
            file_name: DEFAULT_FILE_NAME.to_string(),
            subfolder: String::new(),
            close_after_copy: true,
        }
    }
}

impl Settings {
    pub fn hotkeys(&self) -> [(Action, Option<Hotkey>); 3] {
        [
            (Action::Region, self.hotkey_region),
            (Action::Window, self.hotkey_window),
            (Action::Screen, self.hotkey_screen),
        ]
    }

    pub fn hotkey(&self, action: Action) -> Option<Hotkey> {
        match action {
            Action::Region => self.hotkey_region,
            Action::Window => self.hotkey_window,
            Action::Screen => self.hotkey_screen,
        }
    }

    /// parses file text. invalid lines keep their default and add a warning.
    pub fn parse(text: &str) -> (Settings, Vec<String>) {
        let mut s = Settings::default();
        let mut warnings = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim().trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let n = index + 1;
            let Some((key, value)) = line.split_once('=') else {
                warnings.push(format!("line {n}: expected key = value"));
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            let result = match key {
                "hotkey_region" => parse_hotkey(value).map(|v| s.hotkey_region = v),
                "hotkey_window" => parse_hotkey(value).map(|v| s.hotkey_window = v),
                "hotkey_screen" => parse_hotkey(value).map(|v| s.hotkey_screen = v),
                "after_capture" => match value {
                    "edit" => Ok(s.edit = true),
                    "copy" => Ok((s.edit, s.copy, s.save) = (false, true, false)),
                    "save" => Ok((s.edit, s.copy, s.save) = (false, false, true)),
                    "copy+save" | "save+copy" => Ok((s.edit, s.copy, s.save) = (false, true, true)),
                    _ => Err(format!("after_capture must be edit, copy, save or copy+save, not \"{value}\"")),
                },
                "save_folder" => parse_folder(value).map(|v| s.save_folder = v),
                "file_name" => parse_file_name(value).map(|v| s.file_name = v),
                "subfolder" => match value {
                    "" | "none" => Ok(s.subfolder = String::new()),
                    v => parse_file_name(v).map(|v| s.subfolder = v).map_err(|e| e.replace("file_name", "subfolder")),
                },
                "close_after_copy" => match value {
                    "yes" => Ok(s.close_after_copy = true),
                    "no" => Ok(s.close_after_copy = false),
                    _ => Err(format!("close_after_copy must be yes or no, not \"{value}\"")),
                },
                _ => Err(format!("unknown setting \"{key}\"")),
            };
            if let Err(err) = result {
                warnings.push(format!("line {n}: {err}"));
            }
        }
        let keys: Vec<Hotkey> = s.hotkeys().iter().filter_map(|(_, k)| *k).collect();
        for (i, key) in keys.iter().enumerate() {
            if keys[..i].contains(key) {
                warnings.push(format!("{key} is assigned to more than one capture"));
            }
        }
        (s, warnings)
    }

    /// reads the file; a missing file means defaults without warnings.
    pub fn load(path: &Path) -> (Settings, Vec<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => Settings::parse(&text),
            Err(err) if err.kind() == io::ErrorKind::NotFound => (Settings::default(), Vec::new()),
            Err(err) => (Settings::default(), vec![format!("could not read {}: {err}", path.display())]),
        }
    }
}

fn parse_hotkey(value: &str) -> Result<Option<Hotkey>, String> {
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    Hotkey::parse(value).map(Some)
}

fn parse_folder(value: &str) -> Result<Option<PathBuf>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("save_folder must be a full path like D:\\screenshots, not \"{value}\""));
    }
    Ok(Some(path))
}

pub fn parse_file_name(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Ok(DEFAULT_FILE_NAME.to_string());
    }
    if let Some(c) = value.chars().find(|c| r#"\/:*?"<>|"#.contains(*c) || c.is_control()) {
        return Err(format!("file_name cannot contain \"{c}\""));
    }
    if value.ends_with('.') || value.ends_with(' ') {
        return Err("file_name cannot end with a dot or a space".into());
    }
    Ok(value.to_string())
}

/// the file text for `s`: every setting with a short comment.
pub fn to_text(s: &Settings) -> String {
    let key = |k: Option<Hotkey>| k.map_or("none".to_string(), |k| k.to_string());
    let after = match (s.copy, s.save) {
        _ if s.edit => "edit",
        (true, false) => "copy",
        (false, true) => "save",
        _ => "copy+save",
    };
    let folder = s.save_folder.as_ref().map_or(String::new(), |p| p.display().to_string());
    let lines = [
        "# kadr settings. changes apply as soon as you save this file.".to_string(),
        String::new(),
        "# hotkeys: ctrl, alt, shift, win joined with + and then a key:".into(),
        "# a-z, 0-9, f1-f24, printscreen, insert, delete, home, end,".into(),
        "# pageup, pagedown, pause, space. \"none\" turns a hotkey off.".into(),
        format!("hotkey_region = {}", key(s.hotkey_region)),
        format!("hotkey_window = {}", key(s.hotkey_window)),
        format!("hotkey_screen = {}", key(s.hotkey_screen)),
        String::new(),
        "# what happens to a capture: edit (open the editor; copy and save there),".into(),
        "# or right away: copy, save or copy+save".into(),
        format!("after_capture = {after}"),
        String::new(),
        "# folder for saved captures. empty means pictures/kadr".into(),
        format!("save_folder = {folder}"),
        String::new(),
        "# file name without .png. %Y %m %d %H %M %S become the date and time.".into(),
        "# if the name is taken, _2, _3 ... is added. nothing is overwritten.".into(),
        format!("file_name = {}", s.file_name),
        String::new(),
        "# a subfolder for each month or day, same tokens: %Y-%m, %Y-%m-%d. none = no subfolder.".into(),
        format!("subfolder = {}", if s.subfolder.is_empty() { "none" } else { &s.subfolder }),
        String::new(),
        "# editor: yes = ctrl+c copies and closes the editor; no = it stays open".into(),
        format!("close_after_copy = {}", if s.close_after_copy { "yes" } else { "no" }),
    ];
    // crlf, so notepad and every other windows editor shows it as written.
    let mut text = lines.join("\r\n");
    text.push_str("\r\n");
    text
}

/// replaces the file atomically: write a sibling temp file, then rename over
/// the old one, so a crash or a reader never sees half a file.
pub fn save(path: &Path, s: &Settings) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("ini.tmp");
    std::fs::write(&tmp, to_text(s))?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_template() {
        let dir = std::env::temp_dir().join(format!("kadr-test-{}", std::process::id()));
        let path = dir.join("settings.ini");
        let _ = std::fs::remove_dir_all(&dir);
        save(&path, &Settings::default()).unwrap();
        let (parsed, warnings) = Settings::load(&path);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(parsed, Settings::default());
    }

    #[test]
    fn bad_lines_warn_and_keep_defaults() {
        let (s, w) = Settings::parse("hotkey_region = hyper+x\nafter_capture = print\nfoo = 1\nnonsense\nsave_folder = relative\\dir\nfile_name = a/b");
        assert_eq!(w.len(), 6, "{w:?}");
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn values_apply() {
        let (s, w) = Settings::parse("hotkey_region = ctrl+shift+1\nhotkey_window = none\nafter_capture = copy\nsave_folder = D:\\shots\nfile_name = shot %H%M");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.hotkey_region.unwrap().to_string(), "ctrl+shift+1");
        assert_eq!(s.hotkey_window, None);
        assert!(!s.edit && s.copy && !s.save);
        let (sub, w) = Settings::parse("subfolder = %Y-%m");
        assert!(w.is_empty() && sub.subfolder == "%Y-%m" && to_text(&sub).contains("subfolder = %Y-%m"));
        assert!(Settings::parse("subfolder = a\\b").1.len() == 1, "one level only");
        assert!(Settings::parse("subfolder = none").0.subfolder.is_empty());
        let (e, w) = Settings::parse("after_capture = edit");
        assert!(w.is_empty() && e.edit);
        assert!(to_text(&e).contains("after_capture = edit"));
        assert_eq!(s.save_folder, Some(PathBuf::from("D:\\shots")));
        assert_eq!(s.file_name, "shot %H%M");
    }

    #[test]
    fn save_replaces_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("kadr-save-settings-{}", std::process::id()));
        let path = dir.join("settings.ini");
        let _ = std::fs::remove_dir_all(&dir);
        let (custom, _) = Settings::parse("hotkey_region = ctrl+shift+1
after_capture = save
file_name = shot_%H");
        save(&path, &Settings::default()).unwrap();
        save(&path, &custom).unwrap();
        let (back, warnings) = Settings::load(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back, custom);
        assert!(!path.with_extension("ini.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn duplicate_hotkeys_warn() {
        let (_, w) = Settings::parse("hotkey_region = ctrl+1\nhotkey_screen = ctrl+1");
        assert_eq!(w.len(), 1);
    }
}
