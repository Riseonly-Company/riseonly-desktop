use std::rc::Rc;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    IsIconic, SIZE_MAXIMIZED, SIZE_MINIMIZED, SIZE_RESTORED, SW_HIDE, SW_MINIMIZE, SW_RESTORE, ShowWindowAsync,
    WM_NCDESTROY, WM_SIZE,
};

use super::window_minimise::*;
use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

pub(super) struct NativeWindowMinimise;

impl WindowMinimiseSource for NativeWindowMinimise {
    fn watch(
        &self,
        window: &dyn HasWindowHandle,
        sink: MinimisedSink,
    ) -> Result<MinimiseWatch, WindowMinimiseUnavailable> {
        let handle = window.window_handle().map_err(|error| no_window(&error.to_string()))?;
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return Err(no_window("the handle is not a Win32 window"));
        };
        let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);

        let sink = Rc::new(sink);
        let reference = Rc::as_ptr(&sink) as usize;
        let subclass_id = reference;
        let installed = unsafe { SetWindowSubclass(hwnd, Some(minimise_subclass), subclass_id, reference) };
        if !installed.as_bool() {
            return Err(no_window("SetWindowSubclass refused the window (not on its thread)"));
        }

        let minimised = unsafe { IsIconic(hwnd) }.as_bool();
        sink(minimised);

        Ok(MinimiseWatch::new(MinimiseObserver {
            hwnd,
            subclass_id,
            _sink: sink,
        }))
    }

    fn hide_window(&self, window: &dyn HasWindowHandle) -> PlatformSupport {
        let Some(hwnd) = native_hwnd(window) else {
            return PlatformSupport::Unsupported;
        };
        let posted = unsafe { ShowWindowAsync(hwnd, SW_MINIMIZE).as_bool() && ShowWindowAsync(hwnd, SW_HIDE).as_bool() };
        if posted {
            PlatformSupport::Performed
        } else {
            PlatformSupport::Unsupported
        }
    }

    fn show_window(&self, window: &dyn HasWindowHandle) -> PlatformSupport {
        let Some(hwnd) = native_hwnd(window) else {
            return PlatformSupport::Unsupported;
        };
        if !unsafe { IsIconic(hwnd) }.as_bool() {
            return PlatformSupport::Performed;
        }
        if unsafe { ShowWindowAsync(hwnd, SW_RESTORE) }.as_bool() {
            PlatformSupport::Performed
        } else {
            PlatformSupport::Unsupported
        }
    }
}

/// window of a gpui handle, None for anything that isnt a live win32 handle
fn native_hwnd(window: &dyn HasWindowHandle) -> Option<HWND> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    Some(HWND(win32.hwnd.get() as *mut core::ffi::c_void))
}

fn no_window(detail: &str) -> WindowMinimiseUnavailable {
    WindowMinimiseUnavailable::NoNativeWindow {
        host: HostOs::current(),
        detail: detail.to_owned(),
    }
}

fn minimised_from_size(kind: u32) -> Option<bool> {
    match kind {
        SIZE_MINIMIZED => Some(true),
        SIZE_RESTORED | SIZE_MAXIMIZED => Some(false),
        _ => None,
    }
}

unsafe extern "system" fn minimise_subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    reference: usize,
) -> LRESULT {
    match message {
        WM_SIZE => {
            if let Some(minimised) = u32::try_from(wparam.0).ok().and_then(minimised_from_size) {
                let sink = unsafe { &*(reference as *const MinimisedSink) };
                sink(minimised);
            }
        }
        WM_NCDESTROY => {
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(minimise_subclass), subclass_id);
            }
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

struct MinimiseObserver {
    hwnd: HWND,
    subclass_id: usize,
    _sink: Rc<MinimisedSink>,
}

impl Drop for MinimiseObserver {
    fn drop(&mut self) {
        unsafe {
            let _ = RemoveWindowSubclass(self.hwnd, Some(minimise_subclass), self.subclass_id);
        }
    }
}
