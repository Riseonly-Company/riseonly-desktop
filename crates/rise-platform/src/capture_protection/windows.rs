use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::UI::WindowsAndMessaging::{
    SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WINDOW_DISPLAY_AFFINITY,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::gpui_shim::PlatformSupport;

pub(super) fn set_window_excluded_from_capture(
    window: &gpui::Window,
    excluded: bool,
) -> PlatformSupport {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return PlatformSupport::Unsupported;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return PlatformSupport::Unsupported;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);

    let affinity: WINDOW_DISPLAY_AFFINITY = if excluded {
        WDA_EXCLUDEFROMCAPTURE
    } else {
        WDA_NONE
    };

    match unsafe { SetWindowDisplayAffinity(hwnd, affinity) } {
        Ok(()) => PlatformSupport::Performed,
        Err(error) => {
            tracing::warn!(
                target: "riseonly",
                %error,
                excluded,
                "SetWindowDisplayAffinity refused"
            );
            PlatformSupport::Unsupported
        }
    }
}
