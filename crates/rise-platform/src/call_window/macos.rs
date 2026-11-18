use gpui::{Bounds, Pixels};
use objc2::MainThreadMarker;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApplication, NSRequestUserAttentionType, NSScreen};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

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

    fn start_resize(
        &self,
        window: &gpui::Window,
        corner: FloatingResizeCorner,
    ) -> Result<CallWindowResize, CallWindowUnavailable> {
        let _ = (window, corner);
        Ok(CallWindowResize::App)
    }

    fn set_frame(&self, window: &gpui::Window, frame: Bounds<Pixels>) -> Result<(), CallWindowUnavailable> {
        let mtm = MainThreadMarker::new().ok_or(CallWindowUnavailable::NotMainThread)?;
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| CallWindowUnavailable::NoNativeWindow(error.to_string()))?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return Err(CallWindowUnavailable::NoNativeWindow("not an AppKit window".to_owned()));
        };
        let primary_height = NSScreen::screens(mtm)
            .firstObject()
            .map(|screen| screen.frame().size.height)
            .ok_or_else(|| CallWindowUnavailable::SystemRefused("no screen".to_owned()))?;
        let width = f64::from(f32::from(frame.size.width));
        let height = f64::from(f32::from(frame.size.height));
        let x = f64::from(f32::from(frame.origin.x));
        let y = primary_height - f64::from(f32::from(frame.origin.y)) - height;
        let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
        unsafe {
            let view = appkit.ns_view.cast::<AnyObject>().as_ref();
            let native: Option<Retained<AnyObject>> = msg_send![view, window];
            let native = native.ok_or_else(|| CallWindowUnavailable::NoNativeWindow("no NSWindow".to_owned()))?;
            let _: () = msg_send![&*native, setFrame: rect, display: true];
        }
        Ok(())
    }
}
