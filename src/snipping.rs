//! windows 11 can hand the bare printscreen key to snipping tool
//! ("use the print screen key to open screen capture"). RegisterHotKey still
//! succeeds, but kadr never receives the key.
//!
//! kadr can turn that windows setting off for the current user, and only
//! after the user says yes in a dialog that names exactly what changes.
//! it writes one value, HKCU\Control Panel\Keyboard
//! PrintScreenKeyForSnippingEnabled = 0, and records what was there before
//! under kadr's own key so uninstall can put it back. snipping tool itself is
//! never touched; win+shift+s keeps opening it.
//!
//! test-only: under KADR_TEST_SANDBOX or KADR_TEST_REGISTRY both keys move to
//! HKCU\Software\kadr-test so tests never touch the real windows setting.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HWND};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_DWORD, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW,
    RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::UI::WindowsAndMessaging::{IDYES, MB_DEFBUTTON1, MB_ICONQUESTION, MB_YESNO, MessageBoxW};
use windows::core::{PCWSTR, w};

use crate::{APP_NAME, trace, wide};

const VALUE: &str = "PrintScreenKeyForSnippingEnabled";
/// kadr's record of the windows value before kadr changed it:
/// 0 or 1 = that explicit value, 2 = not set (the windows default).
const BEFORE: &str = "PrintScreenSnippingBefore";
/// 1 once the user has answered the question at start-up.
const ASKED: &str = "PrintScreenAsked";
const NOT_SET: u32 = 2;

fn sandboxed() -> bool {
    std::env::var_os("KADR_TEST_SANDBOX").is_some() || std::env::var_os("KADR_TEST_REGISTRY").is_some()
}

/// where windows keeps the setting (or its sandbox stand-in).
fn keyboard_key() -> String {
    if sandboxed() { format!("Software\\{APP_NAME}-test\\Keyboard") } else { "Control Panel\\Keyboard".into() }
}

/// kadr's own key for its record.
fn own_key() -> String {
    if sandboxed() { format!("Software\\{APP_NAME}-test\\State") } else { format!("Software\\{APP_NAME}") }
}

fn get_dword(key: &str, name: &str) -> Option<u32> {
    let (key, name) = (wide(key), wide(name));
    let mut value = 0u32;
    let mut size = 4u32;
    // SAFETY: out-pointers to locals with the size passed alongside.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(value)
}

fn set_dword(key: &str, name: &str, value: u32) -> Result<(), String> {
    let (key, name) = (wide(key), wide(name));
    let bytes = value.to_le_bytes();
    // SAFETY: valid strings; four bytes for a REG_DWORD. creates the key if needed.
    let status = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()), REG_DWORD.0, Some(bytes.as_ptr().cast()), 4)
    };
    if status.is_ok() { Ok(()) } else { Err(windows::core::Error::from(status.to_hresult()).message()) }
}

fn delete_value(key: &str, name: &str) -> Result<(), String> {
    let (key, name) = (wide(key), wide(name));
    // SAFETY: valid strings.
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr())) };
    if status.is_ok() || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(windows::core::Error::from(status.to_hresult()).message())
    }
}

/// true when windows gives the bare printscreen key to snipping tool.
pub fn owns_printscreen() -> bool {
    if let Some(v) = get_dword(&keyboard_key(), VALUE) {
        return v != 0;
    }
    if sandboxed() {
        return false; // tests opt in by writing the sandbox value
    }
    default_is_on()
}

/// not set explicitly: the default became "on" in windows 11 build
/// 22621.1928 (2023); earlier builds and windows 10 default to off.
fn default_is_on() -> bool {
    let key = w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
    let mut buf = [0u16; 16];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: out-pointers to locals with the sizes passed alongside.
    let status = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key, w!("CurrentBuildNumber"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
    };
    if status.is_err() {
        return false;
    }
    let build: u32 = String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]).parse().unwrap_or(0);
    let mut ubr: u32 = 0;
    let mut size = 4u32;
    // SAFETY: as above. UBR is the update revision, the part after the dot.
    let _ = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key, w!("UBR"), RRF_RT_REG_DWORD, None, Some((&mut ubr as *mut u32).cast()), Some(&mut size)) };
    build > 22621 || (build == 22621 && ubr >= 1928)
}

/// whether the start-up question was already answered (yes or no).
pub fn asked() -> bool {
    get_dword(&own_key(), ASKED) == Some(1)
}

/// writes 0 and remembers what was there, unless kadr already did.
fn turn_off() -> Result<(), String> {
    let kb = keyboard_key();
    if get_dword(&own_key(), BEFORE).is_none() {
        let before = get_dword(&kb, VALUE).map_or(NOT_SET, |v| (v != 0) as u32);
        set_dword(&own_key(), BEFORE, before)?;
    }
    set_dword(&kb, VALUE, 0)?;
    trace!("snipping: printscreen key handed back to apps");
    Ok(())
}

/// the consent dialog. yes → the setting is turned off. the answer is
/// recorded so start-up asks only once; recording printscreen again in
/// settings asks again. returns true when printscreen is now free.
pub fn ask(owner: Option<HWND>) -> bool {
    let text = wide(
        "windows opens snipping tool when you press printscreen, so kadr never gets the key.\n\n\
         turn that off so printscreen opens kadr?\n\n\
         · changes one windows setting for your account: \"use the print screen key to open screen capture\"\n\
         · snipping tool stays installed; win+shift+s still opens it\n\
         · uninstalling kadr turns the setting back on\n\
         · if printscreen still opens snipping tool, sign out and back in",
    );
    trace!("snipping: asking whether to free printscreen");
    // SAFETY: valid strings; owner is ours or none.
    let yes = unsafe { MessageBoxW(owner, PCWSTR(text.as_ptr()), w!("kadr"), MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON1) == IDYES };
    let _ = set_dword(&own_key(), ASKED, 1);
    if !yes {
        trace!("snipping: user kept the windows setting");
        return false;
    }
    match turn_off() {
        Ok(()) => {
            let text = wide(
                "done. press printscreen to check: kadr's selection should appear.

                 if snipping tool still opens, windows applies the change after you sign out and back in.",
            );
            // SAFETY: as above.
            unsafe {
                MessageBoxW(owner, PCWSTR(text.as_ptr()), w!("kadr"), windows::Win32::UI::WindowsAndMessaging::MB_OK);
            }
            true
        }
        Err(e) => {
            let text = wide(&format!("could not change the windows setting: {e}"));
            // SAFETY: as above.
            unsafe {
                MessageBoxW(owner, PCWSTR(text.as_ptr()), w!("kadr"), windows::Win32::UI::WindowsAndMessaging::MB_ICONWARNING);
            }
            false
        }
    }
}

const CONFIRMED: &str = "PrintScreenConfirmed";

/// true once kadr has turned the windows setting off (and not restored it).
pub fn kadr_changed() -> bool {
    get_dword(&own_key(), BEFORE).is_some()
}

/// true once a real printscreen press reached kadr after kadr changed the
/// setting: proof that windows applied it, which the registry value is not.
pub fn confirmed() -> bool {
    get_dword(&own_key(), CONFIRMED) == Some(1)
}

/// called on a bare printscreen WM_HOTKEY. records the proof once.
pub fn confirm_received() {
    if kadr_changed() && !confirmed() {
        let _ = set_dword(&own_key(), CONFIRMED, 1);
        trace!("snipping: printscreen reached kadr, the windows change is in effect");
    }
}

/// uninstall: put the windows value back as it was before kadr, if kadr
/// changed it and the user has not changed it since; then drop kadr's key.
pub fn restore() -> Result<(), String> {
    let kb = keyboard_key();
    if let Some(before) = get_dword(&own_key(), BEFORE)
        && get_dword(&kb, VALUE) == Some(0)
    {
        match before {
            NOT_SET => delete_value(&kb, VALUE)?,
            v => set_dword(&kb, VALUE, v)?,
        }
        trace!("snipping: windows setting restored ({before})");
    }
    let key = wide(&own_key());
    // SAFETY: deletes kadr's own key; a missing key is fine.
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()));
    }
    Ok(())
}
