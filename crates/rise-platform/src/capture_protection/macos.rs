use objc2::MainThreadMarker;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::gpui_shim::PlatformSupport;

const SHARING_NONE: usize = 0;
const SHARING_READ_ONLY: usize = 1;

pub(super) fn set_window_excluded_from_capture(
    window: &gpui::Window,
    excluded: bool,
) -> PlatformSupport {
    if MainThreadMarker::new().is_none() {
        return PlatformSupport::Unsupported;
    }

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return PlatformSupport::Unsupported;
    };

    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return PlatformSupport::Unsupported;
    };

    let wanted = if excluded {
        SHARING_NONE
    } else {
        SHARING_READ_ONLY
    };

    let applied: usize = unsafe {
        let view = appkit.ns_view.cast::<AnyObject>().as_ref();
        let native: Option<Retained<AnyObject>> = msg_send![view, window];
        let Some(native) = native else {
            return PlatformSupport::Unsupported;
        };

        let _: () = msg_send![&*native, setSharingType: wanted];
        msg_send![&*native, sharingType]
    };

    if applied == wanted {
        PlatformSupport::Performed
    } else {
        tracing::warn!(
            target: "riseonly",
            excluded,
            applied,
            "NSWindow kept its sharing type"
        );
        PlatformSupport::Unsupported
    }
}
