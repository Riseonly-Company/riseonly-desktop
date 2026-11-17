use objc2_app_kit::{
    NSPasteboard, NSPasteboardContentsOptions, NSPasteboardTypeString, NSPasteboardTypeURL,
};
use objc2_foundation::NSString;

use super::pasteboard_write::*;

const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";

pub(super) struct NativePasteboard;

impl NativePasteboardWriter for NativePasteboard {
    fn write_host_only_text(&self, text: &str) -> Result<(), PasteboardWriteUnavailable> {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
        let value = NSString::from_str(text);
        let string_type = unsafe { NSPasteboardTypeString };
        if !pasteboard.setString_forType(&value, string_type) {
            return Err(PasteboardWriteUnavailable::SystemRefused(
                "setString:forType: public.utf8-plain-text".to_owned(),
            ));
        }
        let concealed = NSString::from_str(CONCEALED_TYPE);
        if !pasteboard.setString_forType(&NSString::from_str(""), &concealed) {
            tracing::debug!(target: "riseonly", "the pasteboard refused the concealed marker");
        }
        Ok(())
    }

    fn write_url(&self, absolute_string: &str) -> Result<(), PasteboardWriteUnavailable> {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        let value = NSString::from_str(absolute_string);
        let (url_type, string_type) = unsafe { (NSPasteboardTypeURL, NSPasteboardTypeString) };
        let url_written = pasteboard.setString_forType(&value, url_type);
        let string_written = pasteboard.setString_forType(&value, string_type);
        if url_written && string_written {
            Ok(())
        } else {
            Err(PasteboardWriteUnavailable::SystemRefused(format!(
                "setString:forType: public.url {url_written}, public.utf8-plain-text {string_written}"
            )))
        }
    }
}
