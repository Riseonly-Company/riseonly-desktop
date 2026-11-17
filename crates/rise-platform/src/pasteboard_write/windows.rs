use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER, LCS_GM_IMAGES};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory, IWICPalette,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{CF_DIBV5, CF_UNICODETEXT};
use windows::core::{Owned, PCWSTR, w};

use super::pasteboard_write::*;

pub(super) struct NativePasteboard;

impl NativePasteboardWriter for NativePasteboard {
    fn write_host_only_text(&self, text: &str) -> Result<(), PasteboardWriteUnavailable> {
        let clipboard = OpenedClipboard::open()?;
        clipboard.replace_contents()?;
        clipboard.put_unicode_text(text)?;
        let markers = [
            (
                w!("ExcludeClipboardContentFromMonitorProcessing"),
                "ExcludeClipboardContentFromMonitorProcessing",
            ),
            (w!("CanIncludeInClipboardHistory"), "CanIncludeInClipboardHistory"),
            (w!("CanUploadToCloudClipboard"), "CanUploadToCloudClipboard"),
        ];
        for (format_name, label) in markers {
            if let Err(error) = clipboard.put_registered_dword(format_name, 0) {
                tracing::debug!(
                    target: "riseonly",
                    "the clipboard refused the {label} marker: {error}"
                );
            }
        }
        Ok(())
    }

    fn write_url(&self, absolute_string: &str) -> Result<(), PasteboardWriteUnavailable> {
        let clipboard = OpenedClipboard::open()?;
        clipboard.replace_contents()?;
        clipboard.put_unicode_text(absolute_string)?;
        let url_format = w!("UniformResourceLocatorW");
        if let Err(error) = clipboard.put_registered_utf16(url_format, absolute_string) {
            tracing::debug!(
                target: "riseonly",
                "the clipboard refused UniformResourceLocatorW: {error}"
            );
        }
        Ok(())
    }

    fn write_image_png(&self, png: &[u8]) -> Result<(), PasteboardWriteUnavailable> {
        let bitmap = dib_v5_of_png(png);
        let clipboard = OpenedClipboard::open()?;
        clipboard.replace_contents()?;
        clipboard
            .put_registered_bytes(w!("PNG"), png)
            .map_err(|error| refused("SetClipboardData PNG", &error))?;
        match bitmap {
            Ok(dib) => {
                if let Err(error) = put_bytes(u32::from(CF_DIBV5.0), &dib) {
                    tracing::debug!(target: "riseonly", "the clipboard refused CF_DIBV5: {error}");
                }
            }
            Err(error) => {
                tracing::debug!(target: "riseonly", "the copied PNG has no bitmap: {error}");
            }
        }
        Ok(())
    }
}

/// clipboard opened by this thread, dropping it closes the clipboard
struct OpenedClipboard;

impl OpenedClipboard {
    fn open() -> Result<Self, PasteboardWriteUnavailable> {
        unsafe { OpenClipboard(None) }
            .map(|()| Self)
            .map_err(|error| refused("OpenClipboard", &error))
    }

    fn replace_contents(&self) -> Result<(), PasteboardWriteUnavailable> {
        unsafe { EmptyClipboard() }.map_err(|error| refused("EmptyClipboard", &error))
    }

    fn put_unicode_text(&self, text: &str) -> Result<(), PasteboardWriteUnavailable> {
        let words = utf16_with_nul(text);
        put_bytes(u32::from(CF_UNICODETEXT.0), words_as_bytes(&words))
            .map_err(|error| refused("SetClipboardData CF_UNICODETEXT", &error))
    }

    fn put_registered_utf16(&self, format_name: PCWSTR, text: &str) -> windows::core::Result<()> {
        let words = utf16_with_nul(text);
        put_bytes(register(format_name)?, words_as_bytes(&words))
    }

    fn put_registered_dword(&self, format_name: PCWSTR, value: u32) -> windows::core::Result<()> {
        put_bytes(register(format_name)?, &value.to_ne_bytes())
    }

    fn put_registered_bytes(&self, format_name: PCWSTR, bytes: &[u8]) -> windows::core::Result<()> {
        put_bytes(register(format_name)?, bytes)
    }
}

impl Drop for OpenedClipboard {
    fn drop(&mut self) {
        if let Err(error) = unsafe { CloseClipboard() } {
            tracing::debug!(target: "riseonly", "CloseClipboard failed: {error}");
        }
    }
}

fn refused(call: &str, error: &windows::core::Error) -> PasteboardWriteUnavailable {
    PasteboardWriteUnavailable::SystemRefused(format!("{call}: {error}"))
}

/// id of a registered clipboard format, same name gives the same id in every process
fn register(format_name: PCWSTR) -> windows::core::Result<u32> {
    match unsafe { RegisterClipboardFormatW(format_name) } {
        0 => Err(windows::core::Error::from_thread()),
        format => Ok(format),
    }
}

/// copies bytes into movable global memory and hands it to the open clipboard under the format
fn put_bytes(format: u32, bytes: &[u8]) -> windows::core::Result<()> {
    let global = unsafe { Owned::new(GlobalAlloc(GMEM_MOVEABLE, bytes.len())?) };
    let target = unsafe { GlobalLock(*global) };
    if target.is_null() {
        return Err(windows::core::Error::from_thread());
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len()) };
    let _ = unsafe { GlobalUnlock(*global) };
    unsafe { SetClipboardData(format, Some(HANDLE(global.0)))? };
    std::mem::forget::<Owned<HGLOBAL>>(global);
    Ok(())
}

fn utf16_with_nul(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn words_as_bytes(words: &[u16]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), std::mem::size_of_val(words))
    }
}

const LCS_SRGB: u32 = u32::from_be_bytes(*b"sRGB");

fn dib_v5_of_png(png: &[u8]) -> windows::core::Result<Vec<u8>> {
    let _apartment = ComApartment::join();
    let (width, height, top_down) = decode_bgra(png)?;
    let stride = width as usize * 4;
    let header = BITMAPV5HEADER {
        bV5Size: std::mem::size_of::<BITMAPV5HEADER>() as u32,
        bV5Width: i32::try_from(width).map_err(|_| too_large())?,
        bV5Height: i32::try_from(height).map_err(|_| too_large())?,
        bV5Planes: 1,
        bV5BitCount: 32,
        bV5Compression: BI_BITFIELDS,
        bV5SizeImage: u32::try_from(top_down.len()).map_err(|_| too_large())?,
        bV5RedMask: 0x00ff_0000,
        bV5GreenMask: 0x0000_ff00,
        bV5BlueMask: 0x0000_00ff,
        bV5AlphaMask: 0xff00_0000,
        bV5CSType: LCS_SRGB,
        bV5Intent: LCS_GM_IMAGES as u32,
        ..Default::default()
    };
    let mut block = Vec::with_capacity(header.bV5Size as usize + top_down.len());
    block.extend_from_slice(unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref(&header).cast::<u8>(),
            std::mem::size_of::<BITMAPV5HEADER>(),
        )
    });
    for row in top_down.chunks_exact(stride).rev() {
        block.extend_from_slice(row);
    }
    Ok(block)
}

fn decode_bgra(png: &[u8]) -> windows::core::Result<(u32, u32, Vec<u8>)> {
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromMemory(png)?;
        let decoder =
            factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppBGRA,
            WICBitmapDitherTypeNone,
            None::<&IWICPalette>,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let (mut width, mut height) = (0u32, 0u32);
        converter.GetSize(&mut width, &mut height)?;
        let stride = width.checked_mul(4).ok_or_else(too_large)?;
        let length = (stride as usize).checked_mul(height as usize).ok_or_else(too_large)?;
        let mut pixels = vec![0u8; length];
        converter.CopyPixels(std::ptr::null(), stride, &mut pixels)?;
        Ok((width, height, pixels))
    }
}

fn too_large() -> windows::core::Error {
    windows::core::Error::new(
        windows::Win32::Foundation::E_INVALIDARG,
        "the image is too large for one bitmap",
    )
}

struct ComApartment {
    joined: bool,
}

impl ComApartment {
    fn join() -> Self {
        let joined = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        Self { joined }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.joined {
            unsafe { CoUninitialize() };
        }
    }
}
