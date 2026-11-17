use thiserror::Error;

use crate::host_os::HostOs;

/// why the native write didnt happen, the caller then writes through gpui
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum PasteboardWriteUnavailable {
    #[error("the native pasteboard write is not written for {0:?}")]
    BranchNotWritten(HostOs),
    #[error("the system pasteboard refused the write: {0}")]
    SystemRefused(String),
    #[error("the host pasteboard cannot carry the write: {0}")]
    HostCannotWrite(String),
}

/// default answer of an os layer that doesnt override a writer method
pub const fn unavailable_by_default(host: HostOs) -> PasteboardWriteUnavailable {
    PasteboardWriteUnavailable::BranchNotWritten(host)
}

pub trait NativePasteboardWriter {
    fn write_host_only_text(&self, text: &str) -> Result<(), PasteboardWriteUnavailable> {
        let _ = text;
        Err(unavailable_by_default(HostOs::current()))
    }

    fn write_url(&self, absolute_string: &str) -> Result<(), PasteboardWriteUnavailable> {
        let _ = absolute_string;
        Err(unavailable_by_default(HostOs::current()))
    }

    fn write_image_png(&self, png: &[u8]) -> Result<(), PasteboardWriteUnavailable> {
        let _ = png;
        Err(unavailable_by_default(HostOs::current()))
    }
}

/// writes the text for this device only through the writer
pub fn write_host_only_text_with(
    writer: &impl NativePasteboardWriter,
    text: &str,
) -> Result<(), PasteboardWriteUnavailable> {
    writer.write_host_only_text(text)
}

pub fn write_url_with(
    writer: &impl NativePasteboardWriter,
    absolute_string: &str,
) -> Result<(), PasteboardWriteUnavailable> {
    writer.write_url(absolute_string)
}

/// writes the png as the clipboard image through the writer
pub fn write_image_png_with(
    writer: &impl NativePasteboardWriter,
    png: &[u8],
) -> Result<(), PasteboardWriteUnavailable> {
    writer.write_image_png(png)
}

pub fn write_host_only_text(text: &str) -> Result<(), PasteboardWriteUnavailable> {
    write_host_only_text_with(&super::os::NativePasteboard, text)
}

pub fn write_url(absolute_string: &str) -> Result<(), PasteboardWriteUnavailable> {
    write_url_with(&super::os::NativePasteboard, absolute_string)
}

pub fn write_image_png(png: &[u8]) -> Result<(), PasteboardWriteUnavailable> {
    write_image_png_with(&super::os::NativePasteboard, png)
}
