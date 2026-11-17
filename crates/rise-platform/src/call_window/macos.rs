use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSRequestUserAttentionType};

use super::call_window::*;

pub(super) struct NativeCallWindow;

impl CallWindowLayer for NativeCallWindow {
    fn request_attention(
        &self,
        window: &gpui::Window,
        level: AttentionLevel,
    ) -> Result<AttentionRequest, CallWindowUnavailable> {
        let _ = window;
        let mtm = MainThreadMarker::new().ok_or(CallWindowUnavailable::NotMainThread)?;
        let request_type = match level {
            AttentionLevel::Informational => NSRequestUserAttentionType::InformationalRequest,
            AttentionLevel::Critical => NSRequestUserAttentionType::CriticalRequest,
        };
        let request = NSApplication::sharedApplication(mtm).requestUserAttention(request_type);
        Ok(AttentionRequest::new(move || {
            if let Some(mtm) = MainThreadMarker::new() {
                NSApplication::sharedApplication(mtm).cancelUserAttentionRequest(request);
            }
        }))
    }
}
