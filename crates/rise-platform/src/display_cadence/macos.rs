use objc2::MainThreadMarker;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSScreen;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(super) fn reported_refresh_rate(window: &gpui::Window) -> Option<u32> {
    let mtm = MainThreadMarker::new()?;

    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };

    let screen: Option<Retained<NSScreen>> = unsafe {
        let view = appkit.ns_view.cast::<AnyObject>().as_ref();
        let native: Option<Retained<AnyObject>> = msg_send![view, window];
        match native {
            Some(native) => msg_send![&*native, screen],
            None => None,
        }
    };

    let screen = screen.or_else(|| NSScreen::mainScreen(mtm))?;
    u32::try_from(screen.maximumFramesPerSecond()).ok()
}
