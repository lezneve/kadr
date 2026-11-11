//! "start with windows": one value under HKCU\...\Run.
//!
//! the registry value is the only source of truth; nothing is copied into
//! settings.ini, so the two can never disagree. no service, no task.

use std::path::Path;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegDeleteKeyValueW,
    RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

use crate::{APP_NAME, wide};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

/// the Run value's name; test sandboxes (KADR_TEST_SANDBOX) use their own.
fn value_name() -> String {
    match std::env::var_os("KADR_TEST_SANDBOX") {
        Some(_) => format!("{APP_NAME}-test"),
        None => APP_NAME.to_string(),
    }
}

/// the command line windows runs at sign-in for this exe.
fn command_for(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// the current value, if any.
fn read() -> Option<String> {
    let name = wide(&value_name());
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: out-pointers to a local buffer of the size passed alongside.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// true when windows will start this very exe at sign-in.
pub fn is_enabled(exe: &Path) -> bool {
    read().is_some_and(|v| v.eq_ignore_ascii_case(&command_for(exe)))
}

pub fn set(exe: &Path, enabled: bool) -> Result<(), String> {
    let name = wide(&value_name());
    if !enabled {
        // SAFETY: valid null-terminated strings.
        let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, PCWSTR(name.as_ptr())) };
        return if status.is_ok() || status == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(describe(status)) };
    }
    let value = wide(&command_for(exe));
    // SAFETY: a u16 slice viewed as bytes; same allocation, twice the length.
    let bytes = unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2) };
    let mut key = HKEY::default();
    // SAFETY: the key is opened for writing and closed below; the value is a
    // null-terminated utf-16 string passed as bytes, as REG_SZ requires.
    unsafe {
        let status = RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_SET_VALUE, &mut key);
        if status.is_err() {
            return Err(describe(status));
        }
        let status = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes));
        let _ = RegCloseKey(key);
        if status.is_err() {
            return Err(describe(status));
        }
    }
    Ok(())
}

fn describe(status: WIN32_ERROR) -> String {
    windows::core::Error::from(status.to_hresult()).message()
}
