//! notification-area icon and its menu.

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, HGDIOBJ};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetSystemMetrics,
    HICON, ICONINFO, MF_SEPARATOR, MF_STRING, PostMessageW, SM_CXSMICON, SetForegroundWindow,
    SetMenuDefaultItem, TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, WM_APP, WM_NULL,
};
use windows::core::{PCWSTR, Result, w};

use crate::capture::Frame;
use crate::{APP_NAME, wide};

/// callback message for tray events.
pub const WM_TRAY: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Region = 10,
    Screen = 11,
    OpenFolder = 12,
    Settings = 13,
    Quit = 14,
}

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
}

impl Tray {
    pub fn add(hwnd: HWND) -> Result<Tray> {
        let tray = Tray { hwnd, icon: draw_icon(taskbar_is_light())? };
        tray.notify(NIM_ADD)?;
        Ok(tray)
    }

    /// after explorer restarts (`TaskbarCreated`) the icon must be added again.
    pub fn readd(&self) -> Result<()> {
        self.notify(NIM_ADD)
    }

    /// redraws for a changed taskbar theme or dpi.
    pub fn refresh_icon(&mut self) -> Result<()> {
        let icon = draw_icon(taskbar_is_light())?;
        // SAFETY: the old icon is ours and no longer referenced after modify.
        let old = std::mem::replace(&mut self.icon, icon);
        let result = self.notify(NIM_MODIFY);
        unsafe {
            let _ = DestroyIcon(old);
        }
        result
    }

    fn notify(&self, message: windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE) -> Result<()> {
        let mut data = self.data();
        // SAFETY: data is fully initialized for our window and icon id.
        unsafe {
            Shell_NotifyIconW(message, &data).ok()?;
            if message == NIM_ADD {
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &data).ok()?;
            }
        }
        Ok(())
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        let tip = format!("{APP_NAME} · click to capture a region");
        for (dst, src) in data.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
            *dst = src;
        }
        data
    }

}

/// shows the menu at `at` and returns the chosen command.
///
/// a free function on purpose: TrackPopupMenu runs a modal loop, and the
/// caller must not hold any borrow of the tray while other messages arrive.
pub fn menu(hwnd: HWND, at: POINT, items: &[(Command, String)]) -> Option<Command> {
    {
        // SAFETY: the menu is created, shown modally, and destroyed here.
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            for (command, label) in items {
                let _ = if *command == Command::OpenFolder || *command == Command::Quit {
                    AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())
                } else {
                    Ok(())
                };
                let text = wide(label);
                let _ = AppendMenuW(menu, MF_STRING, *command as usize, PCWSTR(text.as_ptr()));
            }
            let _ = SetMenuDefaultItem(menu, Command::Region as u32, 0);
            // required so the menu closes when the user clicks elsewhere.
            let _ = SetForegroundWindow(hwnd);
            let chosen = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                at.x,
                at.y,
                None,
                hwnd,
                None,
            );
            let _ = PostMessageW(Some(hwnd), WM_NULL, Default::default(), Default::default());
            let _ = DestroyMenu(menu);
            [Command::Region, Command::Screen, Command::OpenFolder, Command::Settings, Command::Quit]
                .into_iter()
                .find(|c| *c as i32 == chosen.0)
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = self.data();
        // SAFETY: removes our own icon, then frees the icon handle.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            let _ = DestroyIcon(self.icon);
        }
    }
}

/// true when the taskbar uses the light theme.
fn taskbar_is_light() -> bool {
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: out-pointers to locals with the matching size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 1
}

/// a viewfinder: four corner brackets and a centre dot, drawn at the exact
/// small-icon size so it stays sharp at every scale.
/// `light` = the icon sits on a light surface, so it is drawn dark.
pub fn draw_icon(light: bool) -> Result<HICON> {
    // SAFETY: plain metric query.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    let (r, g, b) = if light { (0x1c, 0x1c, 0x1c) } else { (0xff, 0xff, 0xff) };
    let mut frame = Frame::new(size, size)?;
    {
        let s = size as usize;
        let t = (s / 10).max(2); // stroke
        let len = s * 3 / 8; // bracket arm length
        let m = s / 16; // margin
        let px = frame.pixels_mut();
        let mut set = |x: usize, y: usize| {
            let i = (y * s + x) * 4;
            px[i..i + 4].copy_from_slice(&[b, g, r, 0xff]);
        };
        for i in 0..len {
            for k in 0..t {
                let (lo, hi) = (m + k, s - 1 - m - k);
                let (a, z) = (m + i, s - 1 - m - i);
                for (x, y) in [(a, lo), (lo, a), (z, lo), (hi, a), (a, hi), (lo, z), (z, hi), (hi, z)] {
                    set(x, y);
                }
            }
        }
        let dot = (s / 8).max(2);
        let start = (s - dot) / 2;
        for y in start..start + dot {
            for x in start..start + dot {
                set(x, y);
            }
        }
    }
    // SAFETY: a 1-bpp all-zero mask plus a 32-bit colour bitmap with alpha is
    // the documented way to build an alpha icon; both bitmaps are freed after
    // CreateIconIndirect copies them.
    unsafe {
        let mask = CreateBitmap(size, size, 1, 1, None);
        let info = ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: frame.bitmap(), ..Default::default() };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon
    }
}
