//! global hotkeys through `RegisterHotKey` (no keyboard hooks).

use std::fmt;

use windows::Win32::Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, HWND};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Region = 1,
    Window = 2,
    Screen = 3,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Region, Action::Window, Action::Screen];

    pub fn from_id(id: usize) -> Option<Action> {
        Self::ALL.into_iter().find(|a| *a as usize == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Region => "region",
            Action::Window => "window",
            Action::Screen => "screen",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub vk: u32,
}

/// (name, virtual-key code) for keys allowed in settings.
const NAMED_KEYS: &[(&str, u32)] = &[
    ("printscreen", 0x2C),
    ("insert", 0x2D),
    ("delete", 0x2E),
    ("home", 0x24),
    ("end", 0x23),
    ("pageup", 0x21),
    ("pagedown", 0x22),
    ("pause", 0x13),
    ("space", 0x20),
];

impl Hotkey {
    pub fn is_bare_printscreen(&self) -> bool {
        self.vk == 0x2C && !(self.ctrl || self.alt || self.shift || self.win)
    }

    /// parses `ctrl+shift+printscreen`. case and spaces around `+` are ignored.
    pub fn parse(text: &str) -> Result<Hotkey, String> {
        let mut key = Hotkey { ctrl: false, alt: false, shift: false, win: false, vk: 0 };
        let parts: Vec<String> = text.split('+').map(|p| p.trim().to_ascii_lowercase()).collect();
        let (last, modifiers) = parts.split_last().ok_or("empty hotkey")?;
        for m in modifiers {
            match m.as_str() {
                "ctrl" | "control" => key.ctrl = true,
                "alt" => key.alt = true,
                "shift" => key.shift = true,
                "win" => key.win = true,
                other => return Err(format!("unknown modifier \"{other}\"")),
            }
        }
        key.vk = key_code(last).ok_or_else(|| format!("unknown key \"{last}\""))?;

        // a letter, digit or space with no ctrl/alt/win would swallow normal
        // typing everywhere in the system (shift alone is just a capital letter).
        let typing_key = last.len() == 1 || last == "space";
        if typing_key && !(key.ctrl || key.alt || key.win) {
            return Err(format!("\"{text}\" needs ctrl, alt or win, otherwise it blocks typing"));
        }
        Ok(key)
    }

    fn modifiers(self) -> HOT_KEY_MODIFIERS {
        let mut m = MOD_NOREPEAT;
        if self.ctrl {
            m |= MOD_CONTROL;
        }
        if self.alt {
            m |= MOD_ALT;
        }
        if self.shift {
            m |= MOD_SHIFT;
        }
        if self.win {
            m |= MOD_WIN;
        }
        m
    }
}

fn key_code(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    if bytes.len() == 1 && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return Some(bytes[0].to_ascii_uppercase() as u32); // vk 'A'..'Z', '0'..'9'
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then_some(0x70 + n - 1); // VK_F1..VK_F24
    }
    NAMED_KEYS.iter().find(|(n, _)| *n == name).map(|(_, vk)| *vk)
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [(self.ctrl, "ctrl+"), (self.alt, "alt+"), (self.shift, "shift+"), (self.win, "win+")] {
            if on {
                f.write_str(name)?;
            }
        }
        if let Some((name, _)) = NAMED_KEYS.iter().find(|(_, vk)| *vk == self.vk) {
            return f.write_str(name);
        }
        match self.vk {
            0x70..=0x87 => write!(f, "f{}", self.vk - 0x70 + 1),
            vk => write!(f, "{}", (vk as u8 as char).to_ascii_lowercase()),
        }
    }
}

/// registers each configured hotkey on `hwnd`; returns one message per failure.
pub fn register(hwnd: HWND, keys: &[(Action, Option<Hotkey>)]) -> Vec<String> {
    let mut failures = Vec::new();
    for (action, key) in keys {
        let Some(key) = key else { continue };
        // SAFETY: hwnd is our window; the id is unique per action.
        let result = unsafe { RegisterHotKey(Some(hwnd), *action as i32, key.modifiers(), key.vk) };
        if result.is_ok() {
            crate::trace!("hotkey {key} registered");
        }
        if let Err(err) = result {
            let reason = if err.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult() {
                "is already used by another app".to_string()
            } else {
                format!("could not be registered ({})", err.message())
            };
            failures.push(format!("{} capture: {key} {reason}", action.label()));
        }
    }
    failures
}

/// bare printscreen while windows gives it to snipping tool (see
/// snipping.rs): RegisterHotKey still succeeds, but the hotkey never fires,
/// so the failure has to be detected from the setting itself.
pub fn snipping_conflicts(keys: &[(Action, Option<Hotkey>)]) -> Vec<String> {
    let affected: Vec<&str> =
        keys.iter().filter(|(_, k)| k.as_ref().is_some_and(Hotkey::is_bare_printscreen)).map(|(a, _)| a.label()).collect();
    if affected.is_empty() || !crate::snipping::owns_printscreen() {
        return Vec::new();
    }
    vec![format!(
        "{} capture: printscreen opens snipping tool on this pc. set the hotkey again in kadr settings to turn that off, or choose another hotkey.",
        affected.join(", ")
    )]
}

/// unregisters every action id; ids that were never registered are ignored.
pub fn unregister(hwnd: HWND) {
    for action in Action::ALL {
        // SAFETY: hwnd is our window.
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), action as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints() {
        for text in ["printscreen", "ctrl+printscreen", "alt+shift+f12", "ctrl+alt+s", "shift+win+2", "ctrl+space"] {
            assert_eq!(Hotkey::parse(text).unwrap().to_string(), text);
        }
        assert_eq!(Hotkey::parse(" Ctrl + PrintScreen ").unwrap().to_string(), "ctrl+printscreen");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Hotkey::parse("").is_err());
        assert!(Hotkey::parse("ctrl+").is_err());
        assert!(Hotkey::parse("hyper+a").is_err());
        assert!(Hotkey::parse("ctrl+f25").is_err());
        assert!(Hotkey::parse("s").is_err());
        assert!(Hotkey::parse("shift+s").is_err());
        assert!(Hotkey::parse("space").is_err());
    }
}
