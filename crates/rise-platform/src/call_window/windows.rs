use ::windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use ::windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use ::windows::Win32::UI::WindowsAndMessaging::{
    FLASHW_ALL, FLASHW_STOP, FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO, FLASHWINFO_FLAGS,
    FlashWindowEx, GetCursorPos, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTTOPLEFT, HTTOPRIGHT,
    PostMessageW, WM_NCLBUTTONDOWN,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::call_window::*;

const INFORMATIONAL_FLASHES: u32 = 3;

pub(super) struct NativeCallWindow;

impl CallWindowLayer for NativeCallWindow {
    fn request_attention(
        &self,
        window: &gpui::Window,
        level: AttentionLevel,
    ) -> Result<AttentionRequest, CallWindowUnavailable> {
        let raw_hwnd = raw_hwnd_of(window)?;
        let (flags, count) = match level {
            AttentionLevel::Informational => (FLASHW_TRAY, INFORMATIONAL_FLASHES),
            AttentionLevel::Critical => (FLASHW_ALL | FLASHW_TIMERNOFG, 0),
        };
        flash(raw_hwnd, flags, count);
        Ok(AttentionRequest::new(move || flash(raw_hwnd, FLASHW_STOP, 0)))
    }

    fn start_move(&self, window: &gpui::Window) -> Result<(), CallWindowUnavailable> {
        non_client_press(window, HTCAPTION)
    }

    fn start_resize(
        &self,
        window: &gpui::Window,
        corner: FloatingResizeCorner,
    ) -> Result<CallWindowResize, CallWindowUnavailable> {
        let hit = match corner {
            FloatingResizeCorner::TopLeft => HTTOPLEFT,
            FloatingResizeCorner::TopRight => HTTOPRIGHT,
            FloatingResizeCorner::BottomLeft => HTBOTTOMLEFT,
            FloatingResizeCorner::BottomRight => HTBOTTOMRIGHT,
        };
        non_client_press(window, hit)?;
        Ok(CallWindowResize::System)
    }
}

/// hands the press to the system as one on a frame part, it runs the move or resize loop itself
fn non_client_press(window: &gpui::Window, hit: u32) -> Result<(), CallWindowUnavailable> {
    let raw_hwnd = raw_hwnd_of(window)?;
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor) }
        .map_err(|error| CallWindowUnavailable::SystemRefused(error.to_string()))?;
    let _ = unsafe { ReleaseCapture() };
    unsafe {
        PostMessageW(
            Some(HWND(raw_hwnd as *mut core::ffi::c_void)),
            WM_NCLBUTTONDOWN,
            WPARAM(hit as usize),
            LPARAM(screen_point_lparam(cursor.x, cursor.y)),
        )
    }
    .map_err(|error| CallWindowUnavailable::SystemRefused(error.to_string()))
}

fn raw_hwnd_of(window: &gpui::Window) -> Result<isize, CallWindowUnavailable> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| CallWindowUnavailable::NoNativeWindow(error.to_string()))?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return Err(CallWindowUnavailable::NoNativeWindow(
            "not a Win32 window".to_owned(),
        ));
    };
    Ok(win32.hwnd.get())
}

fn flash(raw_hwnd: isize, flags: FLASHWINFO_FLAGS, count: u32) {
    let info = FLASHWINFO {
        cbSize: u32::try_from(std::mem::size_of::<FLASHWINFO>()).unwrap_or(u32::MAX),
        hwnd: HWND(raw_hwnd as *mut core::ffi::c_void),
        dwFlags: flags,
        uCount: count,
        dwTimeout: 0,
    };
    let _previous = unsafe { FlashWindowEx(&info) };
}
