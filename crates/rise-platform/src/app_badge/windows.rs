use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
use windows::Win32::UI::WindowsAndMessaging::{
    CWPSTRUCT, CallNextHookEx, CreateIconIndirect, DestroyIcon, EnumThreadWindows, GW_OWNER,
    GWL_EXSTYLE, GetWindow, GetWindowLongPtrW, HICON, HOOKPROC, ICONINFO, IsWindowVisible, MSG,
    RegisterWindowMessageW, SetWindowsHookExW, WH_CALLWNDPROC, WH_GETMESSAGE, WINDOWS_HOOK_ID,
    WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, HSTRING, IUnknown, PCWSTR, w};

use super::app_badge::{badge_text, current_count};
use super::overlay_icon::{OVERLAY_ICON_SIZE, overlay_icon_pixels};
use crate::gpui_shim::PlatformSupport;

thread_local! {
    static TASKBAR: RefCell<Option<ITaskbarList3>> = const { RefCell::new(None) };
    static BUTTON_CREATED: Cell<u32> = const { Cell::new(0) };
}

fn taskbar() -> Option<ITaskbarList3> {
    if let Some(taskbar) = TASKBAR.with_borrow(Clone::clone) {
        return Some(taskbar);
    }
    let created = unsafe {
        CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None::<&IUnknown>, CLSCTX_INPROC_SERVER)
    };
    let taskbar = created
        .inspect_err(|error| tracing::warn!(%error, "app badge: no taskbar list"))
        .ok()?;
    if let Err(error) = unsafe { taskbar.HrInit() } {
        tracing::warn!(%error, "app badge: the taskbar list refused to start");
        return None;
    }
    TASKBAR.set(Some(taskbar.clone()));
    Some(taskbar)
}

fn overlay_icon(text: &str) -> Option<HICON> {
    let size = OVERLAY_ICON_SIZE as i32;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let color = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    let pixels = overlay_icon_pixels(text);
    unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), pixels.len()) };
    let mask = unsafe { CreateBitmap(size, size, 1, 1, None) };
    let icon_info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: color,
    };
    let icon = unsafe { CreateIconIndirect(&icon_info) };
    unsafe {
        let _ = DeleteObject(mask.into());
        let _ = DeleteObject(color.into());
    }
    icon.ok()
}

fn show_on(taskbar: &ITaskbarList3, window: HWND, count: u32) {
    let shown = match badge_text(count) {
        None => unsafe { taskbar.SetOverlayIcon(window, HICON::default(), PCWSTR::null()) },
        Some(text) => {
            let Some(icon) = overlay_icon(&text) else {
                tracing::warn!("app badge: the overlay icon could not be built");
                return;
            };
            let description = HSTRING::from(text);
            unsafe {
                let shown = taskbar.SetOverlayIcon(window, icon, &description);
                let _ = DestroyIcon(icon);
                shown
            }
        }
    };
    if let Err(error) = shown {
        tracing::debug!(%error, "app badge: the overlay was not set");
    }
}

unsafe extern "system" fn collect_button_window(window: HWND, windows: LPARAM) -> BOOL {
    let (visible, owner, extended) = unsafe {
        (
            IsWindowVisible(window).as_bool(),
            GetWindow(window, GW_OWNER).is_ok(),
            GetWindowLongPtrW(window, GWL_EXSTYLE) as u32,
        )
    };
    if visible && !owner && extended & WS_EX_TOOLWINDOW.0 == 0 {
        unsafe { &mut *(windows.0 as *mut Vec<HWND>) }.push(window);
    }
    true.into()
}

fn button_windows() -> Vec<HWND> {
    let mut windows = Vec::new();
    let _ = unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(collect_button_window),
            LPARAM(&raw mut windows as isize),
        )
    };
    windows
}

fn on_message(message: u32, window: HWND) {
    if message != 0 && message == BUTTON_CREATED.get() {
        if let Some(taskbar) = taskbar() {
            show_on(&taskbar, window, current_count());
        }
    }
}

unsafe extern "system" fn sent_message_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let sent = unsafe { &*(lparam.0 as *const CWPSTRUCT) };
        on_message(sent.message, sent.hwnd);
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn posted_message_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let posted = unsafe { &*(lparam.0 as *const MSG) };
        on_message(posted.message, posted.hwnd);
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn watch_button_creation() {
    if BUTTON_CREATED.get() != 0 {
        return;
    }
    BUTTON_CREATED.set(unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) });
    unsafe {
        let thread = GetCurrentThreadId();
        let hooks: [(WINDOWS_HOOK_ID, HOOKPROC); 2] = [
            (WH_CALLWNDPROC, Some(sent_message_hook)),
            (WH_GETMESSAGE, Some(posted_message_hook)),
        ];
        for (kind, hook) in hooks {
            if let Err(error) = SetWindowsHookExW(kind, hook, None, thread) {
                tracing::warn!(%error, "app badge: no hook, a new taskbar button shows no count");
            }
        }
    }
}

pub(super) fn apply_count(count: u32) -> PlatformSupport {
    if count > 0 {
        watch_button_creation();
    }
    let Some(taskbar) = taskbar() else {
        return PlatformSupport::Unsupported;
    };
    for window in button_windows() {
        show_on(&taskbar, window, count);
    }
    PlatformSupport::Performed
}
