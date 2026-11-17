use ::windows::core::PCWSTR;
use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::Graphics::Gdi::{
    EnumDisplaySettingsW, GetMonitorInfoW, MonitorFromWindow, DEVMODEW, ENUM_CURRENT_SETTINGS,
    MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(super) fn reported_refresh_rate(window: &gpui::Window) -> Option<u32> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);

    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return None;
    }

    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = core::mem::size_of::<MONITORINFOEXW>() as u32;
    let read = unsafe {
        GetMonitorInfoW(monitor, (&mut info as *mut MONITORINFOEXW).cast::<MONITORINFO>())
    };
    if !read.as_bool() {
        return None;
    }

    let mut mode = DEVMODEW {
        dmSize: core::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let found = unsafe {
        EnumDisplaySettingsW(
            PCWSTR(info.szDevice.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut mode,
        )
    };
    if !found.as_bool() {
        return None;
    }
    Some(mode.dmDisplayFrequency)
}
