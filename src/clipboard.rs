//! copying a frame to the clipboard, in memory, without temporary files.

use std::thread::sleep;
use std::time::Duration;

use windows::Win32::Foundation::{E_FAIL, E_OUTOFMEMORY, GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFOHEADER};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_DIB;
use windows::core::{Error, Result, w};

use crate::capture::Frame;

/// puts the frame on the clipboard as a 24-bit bottom-up CF_DIB.
///
/// 24-bit, because a 32-bit dib with a zero x byte is shown as fully
/// transparent by some apps. bottom-up, because some apps mishandle top-down
/// clipboard dibs. windows synthesizes CF_BITMAP and CF_DIBV5 from it.
///
/// `keep_out_of_history` is test-only (`--test-private-clipboard`): it keeps
/// test captures out of the user's clipboard history (win+v).
pub fn copy(owner: HWND, frame: &Frame, keep_out_of_history: bool) -> Result<()> {
    let dib = Global::dib24(frame)?;
    open_with_retry(owner)?;
    let _close = CloseOnDrop;
    // SAFETY: the clipboard is open by us; on success the system takes
    // ownership of the memory, on failure it stays ours and `dib` frees it.
    unsafe {
        EmptyClipboard()?;
        SetClipboardData(CF_DIB.0 as u32, Some(HANDLE(dib.0.0)))?;
        std::mem::forget(dib);
        // privacy: screenshots never go to the cloud clipboard. local
        // clipboard history (win+v) keeps working as for any other image.
        if let Ok(no) = Global::dword(0) {
            let format = RegisterClipboardFormatW(w!("CanUploadToCloudClipboard"));
            if format != 0 && SetClipboardData(format, Some(HANDLE(no.0.0))).is_ok() {
                std::mem::forget(no);
            }
        }
        if keep_out_of_history {
            let format = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
            match Global::dword(0) {
                Ok(marker) if format != 0 => {
                    SetClipboardData(format, Some(HANDLE(marker.0.0)))?;
                    std::mem::forget(marker);
                }
                _ => return Err(Error::new(E_FAIL, "could not mark the test capture as private")),
            }
        }
    }
    Ok(())
}

/// another app may hold the clipboard open for a moment; retry briefly.
fn open_with_retry(owner: HWND) -> Result<()> {
    let mut last = Error::from(E_FAIL);
    for _ in 0..10 {
        // SAFETY: owner is our window.
        match unsafe { OpenClipboard(Some(owner)) } {
            Ok(()) => return Ok(()),
            Err(err) => last = err,
        }
        sleep(Duration::from_millis(15));
    }
    Err(Error::new(last.code(), "the clipboard is busy in another app"))
}

struct CloseOnDrop;

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        // SAFETY: only constructed after OpenClipboard succeeded.
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// movable global memory, freed on drop unless ownership is handed over.
struct Global(HGLOBAL);

impl Global {
    fn alloc(size: usize) -> Result<Self> {
        // SAFETY: plain allocation.
        let mem = unsafe { GlobalAlloc(GMEM_MOVEABLE, size) }
            .map_err(|_| Error::new(E_OUTOFMEMORY, "not enough memory for the clipboard copy"))?;
        Ok(Global(mem))
    }

    fn with_bytes(&self, f: impl FnOnce(&mut [u8]), len: usize) -> Result<()> {
        // SAFETY: the block is at least `len` bytes; it is locked only for the
        // duration of `f` and unlocked before returning.
        unsafe {
            let ptr = GlobalLock(self.0) as *mut u8;
            if ptr.is_null() {
                return Err(Error::from_thread());
            }
            f(std::slice::from_raw_parts_mut(ptr, len));
            let _ = GlobalUnlock(self.0);
        }
        Ok(())
    }

    fn dword(value: u32) -> Result<Self> {
        let g = Global::alloc(4)?;
        g.with_bytes(|b| b.copy_from_slice(&value.to_le_bytes()), 4)?;
        Ok(g)
    }

    fn dib24(frame: &Frame) -> Result<Self> {
        let width = frame.width as usize;
        let height = frame.height as usize;
        let stride = (width * 3 + 3) & !3; // rows are padded to 4 bytes
        let header_len = size_of::<BITMAPINFOHEADER>();
        let len = header_len + stride * height;
        let header = BITMAPINFOHEADER {
            biSize: header_len as u32,
            biWidth: frame.width,
            biHeight: frame.height, // positive = bottom-up
            biPlanes: 1,
            biBitCount: 24,
            biCompression: BI_RGB.0,
            biSizeImage: (stride * height) as u32,
            ..Default::default()
        };
        let g = Global::alloc(len)?;
        g.with_bytes(
            |bytes| {
                // SAFETY: BITMAPINFOHEADER is plain old data.
                let raw = unsafe {
                    std::slice::from_raw_parts((&header as *const BITMAPINFOHEADER).cast::<u8>(), header_len)
                };
                let (head, body) = bytes.split_at_mut(header_len);
                head.copy_from_slice(raw);
                let src = frame.pixels();
                for (row, dst) in body.chunks_exact_mut(stride).enumerate() {
                    let src_row = &src[(height - 1 - row) * width * 4..][..width * 4];
                    for (d, s) in dst[..width * 3].chunks_exact_mut(3).zip(src_row.chunks_exact(4)) {
                        d.copy_from_slice(&s[..3]);
                    }
                    dst[width * 3..].fill(0);
                }
            },
            len,
        )?;
        Ok(g)
    }
}

impl Drop for Global {
    fn drop(&mut self) {
        // SAFETY: we still own the block.
        unsafe {
            let _ = GlobalFree(Some(self.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// builds the dib for a 3×2 frame without touching the clipboard.
    #[test]
    fn dib24_layout() {
        let mut frame = Frame::new(3, 2).unwrap();
        // top-down bgrx source: row 0 = pixels 1,2,3; row 1 = pixels 4,5,6
        for (i, px) in frame.pixels_mut().chunks_exact_mut(4).enumerate() {
            let v = (i as u8 + 1) * 10;
            px.copy_from_slice(&[v, v + 1, v + 2, 0xEE]);
        }
        let dib = Global::dib24(&frame).unwrap();
        let header_len = size_of::<BITMAPINFOHEADER>();
        let stride = 12; // 3 px * 3 bytes = 9, padded to 12
        let mut out = vec![0u8; header_len + stride * 2];
        let len = out.len();
        dib.with_bytes(|b| out.copy_from_slice(b), len).unwrap();

        let header: BITMAPINFOHEADER = unsafe { std::ptr::read_unaligned(out.as_ptr().cast()) };
        assert_eq!((header.biWidth, header.biHeight, header.biBitCount), (3, 2, 24)); // positive height = bottom-up
        assert_eq!(header.biSizeImage as usize, stride * 2);

        let body = &out[header_len..];
        // first stored row is the bottom row of the image (pixels 4,5,6), bgr, x byte dropped
        assert_eq!(&body[..12], &[40, 41, 42, 50, 51, 52, 60, 61, 62, 0, 0, 0]);
        assert_eq!(&body[12..], &[10, 11, 12, 20, 21, 22, 30, 31, 32, 0, 0, 0]);
    }
}
