//! png encoding through windows imaging component (wic).
//!
//! wic ships with windows, so this needs no extra crate. com must be
//! initialized on the calling thread.

use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat24bppBGR,
    GUID_WICPixelFormat32bppBGR, IWICImagingFactory, WICBitmapEncoderNoCache,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::{IStream_Read, IStream_Reset, IStream_Size, SHCreateMemStream};
use windows::core::{Error, Result};

use crate::capture::Frame;

/// encodes the frame as a 24-bit rgb png and returns the file bytes.
///
/// the screen has no meaningful alpha, so the x byte is dropped; that also
/// makes files about a quarter smaller than 32-bit rgba.
pub fn png(frame: &Frame) -> Result<Vec<u8>> {
    // SAFETY: plain com calls on interfaces we own; buffers passed to wic
    // outlive the calls that read them.
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = SHCreateMemStream(None)
            .ok_or_else(|| Error::new(windows::Win32::Foundation::E_OUTOFMEMORY, "memory stream"))?;

        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;

        let mut target = None;
        let mut options = None;
        encoder.CreateNewFrame(&mut target, &mut options)?;
        let target = target.ok_or_else(|| Error::from(windows::Win32::Foundation::E_FAIL))?;
        target.Initialize(options.as_ref())?;
        target.SetSize(frame.width as u32, frame.height as u32)?;
        let mut format = GUID_WICPixelFormat24bppBGR;
        target.SetPixelFormat(&mut format)?;

        let source = factory.CreateBitmapFromMemory(
            frame.width as u32,
            frame.height as u32,
            &GUID_WICPixelFormat32bppBGR,
            frame.stride() as u32,
            frame.pixels(),
        )?;
        // WriteSource converts 32bppBGR to the negotiated frame format.
        target.WriteSource(&source, std::ptr::null())?;
        target.Commit()?;
        encoder.Commit()?;

        let size = IStream_Size(&stream)? as usize;
        let mut bytes = vec![0u8; size];
        IStream_Reset(&stream)?;
        IStream_Read(&stream, bytes.as_mut_ptr().cast(), size as u32)?;
        Ok(bytes)
    }
}
