use std::cell::{OnceCell, RefCell};
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Globalization::{U_ZERO_ERROR, ucal_getTimeZoneIDForWindowsID};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Time::{
    DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation, TIME_ZONE_ID_INVALID,
};
use windows::Win32::UI::WindowsAndMessaging as win;
use windows::core::{PCSTR, PCWSTR, w};

use super::system_clock::*;
use crate::system_activity::ActivityWatch;

const IANA_NAME_CAPACITY: usize = 128;
const WATCH_CLASS: PCWSTR = w!("RiseonlySystemClockWatch");

pub(super) struct NativeSystemClock;

impl SystemClockSource for NativeSystemClock {
    fn system_zone_name(&self) -> Option<String> {
        let mut information = DYNAMIC_TIME_ZONE_INFORMATION::default();
        if unsafe { GetDynamicTimeZoneInformation(&mut information) } == TIME_ZONE_ID_INVALID {
            return None;
        }
        let key = &information.TimeZoneKeyName;
        let key_length = key.iter().position(|unit| *unit == 0).unwrap_or(key.len());
        if key_length == 0 {
            return None;
        }

        let mut name = [0u16; IANA_NAME_CAPACITY];
        let mut status = U_ZERO_ERROR;
        let name_length = unsafe {
            ucal_getTimeZoneIDForWindowsID(
                key.as_ptr(),
                key_length as i32,
                PCSTR::null(),
                name.as_mut_ptr(),
                IANA_NAME_CAPACITY as i32,
                &mut status,
            )
        };
        if status.0 > 0 || name_length <= 0 {
            return None;
        }
        String::from_utf16(name.get(..name_length as usize)?).ok()
    }

    fn watch(&self, sink: SystemClockSink) -> Result<ActivityWatch, SystemClockUnavailable> {
        let shared = Arc::new(WatchShared::default());
        let thread_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("rise-system-clock".to_owned())
            .spawn(move || run_watch_window(&thread_shared, sink))
            .map_err(|error| SystemClockUnavailable::NoService(error.to_string()))?;
        Ok(ActivityWatch::new(WatchStop { shared }))
    }
}

/// what the watch owner and the window thread see of each other
#[derive(Default)]
struct WatchShared {
    stopped: AtomicBool,
    window: AtomicIsize,
}

struct WatchStop {
    shared: Arc<WatchShared>,
}

impl Drop for WatchStop {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        let window = self.shared.window.load(Ordering::SeqCst);
        if window != 0 {
            let _ = unsafe {
                win::PostMessageW(
                    Some(HWND(window as *mut c_void)),
                    win::WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        }
    }
}

/// state of the window procedure, lives on the window thread
struct WatchListener {
    sink: SystemClockSink,
    seen_zone: RefCell<Option<String>>,
}

thread_local! {
    static LISTENER: OnceCell<WatchListener> = const { OnceCell::new() };
}

fn run_watch_window(shared: &WatchShared, sink: SystemClockSink) {
    LISTENER.with(|listener| {
        let _ = listener.set(WatchListener {
            sink,
            seen_zone: RefCell::new(NativeSystemClock.system_zone_name()),
        });
    });
    let window = match create_watch_window() {
        Ok(window) => window,
        Err(error) => {
            tracing::warn!(%error, "system clock: no window to hear changes with");
            return;
        }
    };
    shared.window.store(window.0 as isize, Ordering::SeqCst);
    if shared.stopped.load(Ordering::SeqCst) {
        let _ = unsafe { win::DestroyWindow(window) };
        return;
    }

    let mut message = win::MSG::default();
    while unsafe { win::GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        unsafe { win::DispatchMessageW(&message) };
    }
}

fn create_watch_window() -> windows::core::Result<HWND> {
    let instance = HINSTANCE::from(unsafe { GetModuleHandleW(None) }?);
    let class = win::WNDCLASSW {
        lpfnWndProc: Some(watch_procedure),
        hInstance: instance,
        lpszClassName: WATCH_CLASS,
        ..Default::default()
    };
    let _atom = unsafe { win::RegisterClassW(&class) };
    unsafe {
        win::CreateWindowExW(
            win::WS_EX_TOOLWINDOW,
            WATCH_CLASS,
            PCWSTR::null(),
            win::WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    }
}

unsafe extern "system" fn watch_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        win::WM_TIMECHANGE => report_zone_change(),
        win::WM_SETTINGCHANGE => {
            report_zone_change();
            if windows_setting_is_locale(setting_name(lparam).as_deref()) {
                report(SystemClockChange::Locale);
            }
        }
        win::WM_DESTROY => unsafe { win::PostQuitMessage(0) },
        _ => {}
    }
    unsafe { win::DefWindowProcW(window, message, wparam, lparam) }
}

fn report_zone_change() {
    let moved = LISTENER.with(|listener| {
        listener.get().is_some_and(|listener| {
            zone_name_moved(
                &mut listener.seen_zone.borrow_mut(),
                NativeSystemClock.system_zone_name(),
            )
        })
    });
    if moved {
        report(SystemClockChange::TimeZone);
    }
}

fn report(change: SystemClockChange) {
    LISTENER.with(|listener| {
        if let Some(listener) = listener.get() {
            (listener.sink)(change);
        }
    });
}

fn setting_name(lparam: LPARAM) -> Option<String> {
    if lparam.0 == 0 {
        return None;
    }
    unsafe { PCWSTR(lparam.0 as *const u16).to_string() }.ok()
}
