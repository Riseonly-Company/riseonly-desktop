use super::window_chrome::*;
use crate::gpui_shim::PlatformSupport;

use ::windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use ::windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use ::windows::Win32::Graphics::Gdi::ClientToScreen;
use ::windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use ::windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, HTCAPTION, IsZoomed, PostMessageW, SC_MAXIMIZE, SC_MOVE, SC_RESTORE, SM_CYCAPTION, WM_SYSCOMMAND,
};
use ::windows::core::BOOL;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(super) fn round_window_corner(_window: &gpui::Window) -> PlatformSupport {
    PlatformSupport::Unsupported
}

pub(super) fn place_traffic_lights(_window: &gpui::Window, _chrome: &WindowChrome) -> PlatformSupport {
    PlatformSupport::Unsupported
}

pub(super) fn start_titlebar_move(window: &gpui::Window) -> PlatformSupport {
    let Some(hwnd) = window_hwnd(window) else {
        return PlatformSupport::Unsupported;
    };
    unsafe {
        let _ = ReleaseCapture();
    }
    post_system_command(hwnd, SC_MOVE | HTCAPTION)
}

pub(super) fn toggle_maximize(window: &gpui::Window) -> PlatformSupport {
    let Some(hwnd) = window_hwnd(window) else {
        return PlatformSupport::Unsupported;
    };
    let command = if unsafe { IsZoomed(hwnd) }.as_bool() { SC_RESTORE } else { SC_MAXIMIZE };
    post_system_command(hwnd, command)
}

fn window_hwnd(window: &gpui::Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    Some(HWND(win32.hwnd.get() as *mut core::ffi::c_void))
}

fn post_system_command(hwnd: HWND, command: u32) -> PlatformSupport {
    match unsafe { PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(command as usize), LPARAM(0)) } {
        Ok(()) => PlatformSupport::Performed,
        Err(error) => {
            tracing::warn!(target: "riseonly", %error, command, "PostMessageW(WM_SYSCOMMAND) refused");
            PlatformSupport::Unsupported
        }
    }
}

pub(super) fn set_system_controls_appearance(
    window: &gpui::Window,
    appearance: Option<gpui::WindowAppearance>,
) -> PlatformSupport {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return PlatformSupport::Unsupported;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return PlatformSupport::Unsupported;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);

    if !has_system_caption(hwnd) {
        return PlatformSupport::Unsupported;
    }

    let dark = match appearance.unwrap_or_else(|| window.appearance()) {
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark => true,
        gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight => false,
    };
    let value = BOOL::from(dark);

    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const value).cast(),
            size_of::<BOOL>() as u32,
        )
    };

    match result {
        Ok(()) => PlatformSupport::Performed,
        Err(error) => {
            tracing::warn!(
                target: "riseonly",
                %error,
                dark,
                "DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE) refused"
            );
            PlatformSupport::Unsupported
        }
    }
}

fn has_system_caption(hwnd: HWND) -> bool {
    let mut frame = RECT::default();
    let mut client_origin = POINT::default();

    unsafe {
        if GetWindowRect(hwnd, &mut frame).is_err() {
            return false;
        }
        if !ClientToScreen(hwnd, &mut client_origin).as_bool() {
            return false;
        }

        let dpi = GetDpiForWindow(hwnd);
        if dpi == 0 {
            return false;
        }
        let caption = GetSystemMetricsForDpi(SM_CYCAPTION, dpi);

        caption > 0 && client_origin.y - frame.top >= caption
    }
}

#[cfg(feature = "dev-drive")]
pub(super) fn conceal_window_for_dev_drive(_window: &gpui::Window) -> PlatformSupport {
    PlatformSupport::Unsupported
}

#[cfg(feature = "dev-drive")]
pub(super) fn keep_app_in_background_for_dev_drive() -> PlatformSupport {
    PlatformSupport::Unsupported
}

#[cfg(feature = "dev-drive")]
pub(super) fn watch_windows_for_dev_drive(
    _wake: Box<dyn Fn() + Send + Sync>,
) -> Option<Box<dyn std::any::Any>> {
    None
}
