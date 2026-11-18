use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell as shell;
use windows::Win32::UI::WindowsAndMessaging as win;
use windows::core::{PCWSTR, w};

use super::tray::*;
use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

const ICON_ID: u32 = 1;

const APP_ICON_RESOURCE: usize = 1;

const CALLBACK_MESSAGE: u32 = win::WM_APP + 1;

const HOST_CLASS: PCWSTR = w!("RiseonlyTrayHost");

thread_local! {
    static HOSTS: RefCell<Vec<Rc<RefCell<IconState>>>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn open(config: TrayConfig) -> Result<Box<dyn Tray>, TrayError> {
    let Some(handle) = config.window else {
        return Err(TrayError::NoWindowHandle);
    };
    let mut menu = config.menu_items.unwrap_or_default();
    if !tray_menu_is_valid(&menu) {
        tracing::warn!("invalid tray menu ids, the icon opens with no menu");
        menu.clear();
    }
    let app_window = HWND(handle.0 as usize as *mut core::ffi::c_void);

    let instance: HINSTANCE = match unsafe { GetModuleHandleW(None) } {
        Ok(module) => module.into(),
        Err(error) => return Err(TrayError::Refused(error.to_string())),
    };

    register_host_class(instance)?;
    let host = create_host_window(instance)?;

    let taskbar_created = match unsafe { win::RegisterWindowMessageW(w!("TaskbarCreated")) } {
        0 => None,
        message => {
            let _ = unsafe {
                win::ChangeWindowMessageFilterEx(host, message, win::MSGFLT_ALLOW, None)
            };
            Some(message)
        }
    };

    let state = Rc::new(RefCell::new(IconState {
        host,
        app_window,
        icon: app_icon(instance),
        tooltip: config.tooltip,
        unread: 0,
        visible: true,
        version4: false,
        menu,
        handler: None,
        taskbar_created,
    }));

    HOSTS.with(|hosts| hosts.borrow_mut().push(state.clone()));

    if !add_icon(&state) {
        forget_host(host);
        let _ = unsafe { win::DestroyWindow(host) };
        let refusal = "Shell_NotifyIconW refused NIM_ADD".to_owned();
        return Err(TrayError::Refused(refusal));
    }

    Ok(Box::new(NotifyIconTray { state }))
}

struct IconState {
    host: HWND,
    app_window: HWND,
    icon: win::HICON,
    tooltip: String,
    unread: u32,
    visible: bool,
    version4: bool,
    menu: Vec<TrayMenuItem>,
    handler: Option<TrayEventHandler>,
    taskbar_created: Option<u32>,
}

fn register_host_class(instance: HINSTANCE) -> Result<(), TrayError> {
    let class = win::WNDCLASSEXW {
        cbSize: size_of::<win::WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(host_procedure),
        hInstance: instance,
        lpszClassName: HOST_CLASS,
        ..Default::default()
    };

    let atom = unsafe { win::RegisterClassExW(&class) };
    if atom != 0 {
        return Ok(());
    }

    let error = unsafe { GetLastError() };
    if error == ERROR_CLASS_ALREADY_EXISTS {
        return Ok(());
    }

    Err(TrayError::Refused(format!("RegisterClassExW failed: {}", error.0)))
}

fn create_host_window(instance: HINSTANCE) -> Result<HWND, TrayError> {
    let created = unsafe {
        win::CreateWindowExW(
            win::WS_EX_TOOLWINDOW,
            HOST_CLASS,
            w!("Riseonly"),
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
    };

    created.map_err(|error| TrayError::Refused(error.to_string()))
}

fn app_icon(instance: HINSTANCE) -> win::HICON {
    let width = unsafe { win::GetSystemMetrics(win::SM_CXSMICON) };
    let height = unsafe { win::GetSystemMetrics(win::SM_CYSMICON) };

    let loaded = unsafe {
        win::LoadImageW(
            Some(instance),
            PCWSTR(APP_ICON_RESOURCE as *const u16),
            win::IMAGE_ICON,
            width,
            height,
            win::LR_SHARED,
        )
    };

    match loaded {
        Ok(image) => win::HICON(image.0),
        Err(error) => {
            tracing::warn!(%error, "no ICON 1 in the executable, the tray shows the stock icon");
            unsafe { win::LoadIconW(None, win::IDI_APPLICATION) }.unwrap_or_default()
        }
    }
}

fn icon_data(state: &IconState) -> shell::NOTIFYICONDATAW {
    let mut data = shell::NOTIFYICONDATAW {
        cbSize: size_of::<shell::NOTIFYICONDATAW>() as u32,
        hWnd: state.host,
        uID: ICON_ID,
        uFlags: shell::NIF_ICON
            | shell::NIF_TIP
            | shell::NIF_STATE
            | shell::NIF_MESSAGE
            | shell::NIF_SHOWTIP,
        uCallbackMessage: CALLBACK_MESSAGE,
        hIcon: state.icon,
        dwStateMask: shell::NIS_HIDDEN,
        dwState: if state.visible {
            Default::default()
        } else {
            shell::NIS_HIDDEN
        },
        ..Default::default()
    };

    let mut tip = [0u16; 128];
    let text = tray_tooltip(HostOs::Windows, &state.tooltip, state.unread);
    for (slot, unit) in tip.iter_mut().zip(text.encode_utf16().take(127)) {
        *slot = unit;
    }
    data.szTip = tip;

    data
}

fn shell_call(message: shell::NOTIFY_ICON_MESSAGE, data: &shell::NOTIFYICONDATAW) -> bool {
    unsafe { shell::Shell_NotifyIconW(message, data) }.as_bool()
}

fn add_icon(state: &Rc<RefCell<IconState>>) -> bool {
    let mut data = icon_data(&state.borrow());
    if !shell_call(shell::NIM_ADD, &data) {
        return false;
    }

    data.Anonymous.uVersion = shell::NOTIFYICON_VERSION_4;
    let version4 = shell_call(shell::NIM_SETVERSION, &data);
    state.borrow_mut().version4 = version4;

    true
}

fn modify_icon(state: &Rc<RefCell<IconState>>) -> PlatformSupport {
    let data = icon_data(&state.borrow());
    if shell_call(shell::NIM_MODIFY, &data) {
        PlatformSupport::Performed
    } else {
        PlatformSupport::Unsupported
    }
}

fn host_state(host: HWND) -> Option<Rc<RefCell<IconState>>> {
    HOSTS.with(|hosts| {
        hosts
            .borrow()
            .iter()
            .find(|state| state.borrow().host == host)
            .cloned()
    })
}

fn forget_host(host: HWND) {
    HOSTS.with(|hosts| hosts.borrow_mut().retain(|state| state.borrow().host != host));
}

unsafe extern "system" fn host_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some(state) = host_state(hwnd) else {
        return unsafe { win::DefWindowProcW(hwnd, message, wparam, lparam) };
    };

    if message == CALLBACK_MESSAGE {
        let version4 = state.borrow().version4;
        if let Some(action) = tray_icon_action(version4, wparam.0, lparam.0) {
            perform(&state, action);
        }
        return LRESULT(0);
    }

    if Some(message) == state.borrow().taskbar_created {
        if !add_icon(&state) {
            tracing::warn!("Explorer restarted and refused the tray icon again");
        }
        return LRESULT(0);
    }

    unsafe { win::DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn perform(state: &Rc<RefCell<IconState>>, action: TrayIconAction) {
    let event = match action {
        TrayIconAction::Activate => Some(TrayEvent::Activate),
        TrayIconAction::Menu { anchor } => {
            let (x, y) = anchor.unwrap_or_else(cursor_position);
            let (host, items) = {
                let state = state.borrow();
                (state.host, state.menu.clone())
            };
            if items.is_empty() { None } else { show_menu(host, &items, x, y) }
        }
    };

    let Some(event) = event else {
        return;
    };

    if event == TrayEvent::Activate {
        let window = state.borrow().app_window;
        raise(window);
    }

    let handler = state.borrow_mut().handler.take();
    if let Some(mut handler) = handler {
        handler(event);
        let mut state = state.borrow_mut();
        if state.handler.is_none() {
            state.handler = Some(handler);
        }
    }
}

fn cursor_position() -> (i32, i32) {
    let mut point = POINT::default();
    let _ = unsafe { win::GetCursorPos(&mut point) };
    (point.x, point.y)
}

fn raise(window: HWND) {
    unsafe {
        if win::IsIconic(window).as_bool() {
            let _ = win::ShowWindow(window, win::SW_RESTORE);
        } else {
            let _ = win::ShowWindow(window, win::SW_SHOW);
        }
        let _ = win::SetForegroundWindow(window);
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn show_menu(host: HWND, items: &[TrayMenuItem], x: i32, y: i32) -> Option<TrayEvent> {
    let menu = unsafe { win::CreatePopupMenu() }.ok()?;

    let appended = items.iter().all(|item| {
        let flags = match item.checked {
            Some(true) => win::MF_STRING | win::MF_CHECKED,
            Some(false) | None => win::MF_STRING,
        };
        let flags = if item.enabled { flags } else { flags | win::MF_GRAYED };
        let text = wide(&win32_menu_text(&item.label));
        unsafe { win::AppendMenuW(menu, flags, item.id as usize, PCWSTR(text.as_ptr())) }.is_ok()
    });

    let mut picked = 0u32;
    if appended {
        unsafe {
            let _ = win::SetForegroundWindow(host);
            let flags = win::TPM_RETURNCMD | win::TPM_NONOTIFY | win::TPM_RIGHTBUTTON;
            picked = win::TrackPopupMenuEx(menu, flags.0, x, y, host, None).0 as u32;
            let _ = win::PostMessageW(Some(host), win::WM_NULL, WPARAM(0), LPARAM(0));
        }
    }

    let _ = unsafe { win::DestroyMenu(menu) };

    tray_menu_pick(picked)
}

struct NotifyIconTray {
    state: Rc<RefCell<IconState>>,
}

impl Tray for NotifyIconTray {
    fn set_tooltip(&mut self, text: &str) -> PlatformSupport {
        self.state.borrow_mut().tooltip = text.to_owned();
        modify_icon(&self.state)
    }

    fn set_badge(&mut self, count: u32) -> PlatformSupport {
        self.state.borrow_mut().unread = count;
        modify_icon(&self.state)
    }

    fn set_visible(&mut self, visible: bool) -> PlatformSupport {
        self.state.borrow_mut().visible = visible;
        modify_icon(&self.state)
    }

    fn availability(&self) -> TrayAvailability {
        TrayAvailability::Available
    }

    fn set_menu(&mut self, items: Vec<TrayMenuItem>) -> PlatformSupport {
        if !tray_menu_is_valid(&items) {
            return PlatformSupport::Unsupported;
        }
        self.state.borrow_mut().menu = items;
        PlatformSupport::Performed
    }

    fn set_event_handler(&mut self, handler: TrayEventHandler) -> Result<(), TrayError> {
        self.state.borrow_mut().handler = Some(handler);
        Ok(())
    }
}

impl Drop for NotifyIconTray {
    fn drop(&mut self) {
        let host = self.state.borrow().host;
        forget_host(host);

        let data = shell::NOTIFYICONDATAW {
            cbSize: size_of::<shell::NOTIFYICONDATAW>() as u32,
            hWnd: host,
            uID: ICON_ID,
            ..Default::default()
        };
        let _ = shell_call(shell::NIM_DELETE, &data);

        let _ = unsafe { win::DestroyWindow(host) };
    }
}
