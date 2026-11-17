use std::cell::Cell;
use std::ffi::c_void;
use std::sync::OnceLock;

use parking_lot::Mutex;
use ::windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, WIN32_ERROR};
use ::windows::Win32::NetworkManagement::IpHelper::{
    CancelMibChangeNotify2, PNETWORK_CONNECTIVITY_HINT_CHANGE_CALLBACK,
};
use ::windows::Win32::Networking::WinSock::{
    NL_NETWORK_CONNECTIVITY_HINT, NL_NETWORK_CONNECTIVITY_LEVEL_HINT,
    NetworkConnectivityLevelHintHidden, NetworkConnectivityLevelHintNone,
};
use ::windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use ::windows::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, HPOWERNOTIFY, PowerRegisterSuspendResumeNotification,
    PowerUnregisterSuspendResumeNotification,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND,
};
use ::windows::core::{s, w};

use crate::system_activity::*;

pub(crate) struct NativeSystemActivity;

impl SystemActivitySource for NativeSystemActivity {
    fn watch_sleep(&self, sink: EventSink) -> Result<ActivityWatch, SystemActivityUnavailable> {
        let id = register_sink(sink);
        let params = Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(power_changed),
            Context: context_of(id),
        });
        let mut handle: *mut c_void = std::ptr::null_mut();
        let status = unsafe {
            PowerRegisterSuspendResumeNotification(
                DEVICE_NOTIFY_CALLBACK,
                HANDLE((&raw const *params).cast_mut().cast()),
                &raw mut handle,
            )
        };
        if status != ERROR_SUCCESS {
            unregister_sink(id);
            return Err(refused(
                SystemActivityKind::SleepAndWake,
                "PowerRegisterSuspendResumeNotification",
                status,
            ));
        }
        Ok(ActivityWatch::new(PowerRegistration {
            id,
            handle: HPOWERNOTIFY(handle.addr().cast_signed()),
            params: Some(params),
        }))
    }

    fn watch_network_path(
        &self,
        sink: EventSink,
    ) -> Result<ActivityWatch, SystemActivityUnavailable> {
        let notify = notify_hint_change().ok_or_else(|| SystemActivityUnavailable::SystemRefused {
            what: SystemActivityKind::NetworkPath,
            detail: "NotifyNetworkConnectivityHintChange is missing from iphlpapi.dll: it needs \
                     Windows 10 version 2004 (build 19041) or later"
                .to_owned(),
        })?;
        let id = register_sink(sink);
        let mut handle = HANDLE::default();
        let status = unsafe { notify(Some(hint_changed), context_of(id), true, &raw mut handle) };
        if status != ERROR_SUCCESS {
            unregister_sink(id);
            return Err(refused(
                SystemActivityKind::NetworkPath,
                "NotifyNetworkConnectivityHintChange",
                status,
            ));
        }
        Ok(ActivityWatch::new(PathRegistration { id, handle }))
    }
}

/// true when the level counts as satisfied
/// local, constrained and internet are ok and None and Hidden are not
/// unknown counts as satisfied, a wrong satisfied costs one failed try
pub(crate) fn level_is_satisfied(level: NL_NETWORK_CONNECTIVITY_LEVEL_HINT) -> bool {
    level != NetworkConnectivityLevelHintNone && level != NetworkConnectivityLevelHintHidden
}

/// documented signature of NotifyNetworkConnectivityHintChange
type NotifyHintChange = unsafe extern "system" fn(
    callback: PNETWORK_CONNECTIVITY_HINT_CHANGE_CALLBACK,
    caller_context: *const c_void,
    initial_notification: bool,
    notification_handle: *mut HANDLE,
) -> WIN32_ERROR;

fn notify_hint_change() -> Option<NotifyHintChange> {
    static RESOLVED: OnceLock<Option<NotifyHintChange>> = OnceLock::new();
    *RESOLVED.get_or_init(|| {
        let module =
            unsafe { LoadLibraryExW(w!("iphlpapi.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
                .ok()?;
        let address = unsafe { GetProcAddress(module, s!("NotifyNetworkConnectivityHintChange")) }?;
        Some(unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, NotifyHintChange>(address)
        })
    })
}

fn refused(what: SystemActivityKind, call: &str, status: WIN32_ERROR) -> SystemActivityUnavailable {
    SystemActivityUnavailable::SystemRefused {
        what,
        detail: format!("{call} failed with error {}", status.0),
    }
}

struct Sinks {
    next_id: usize,
    entries: Vec<(usize, EventSink)>,
}

static SINKS: Mutex<Sinks> = Mutex::new(Sinks {
    next_id: 1,
    entries: Vec::new(),
});

fn register_sink(sink: EventSink) -> usize {
    let mut sinks = SINKS.lock();
    let id = sinks.next_id;
    sinks.next_id = sinks.next_id.wrapping_add(1).max(1);
    sinks.entries.push((id, sink));
    id
}

fn unregister_sink(id: usize) {
    SINKS.lock().entries.retain(|(entry, _)| *entry != id);
}

fn sink_of(id: usize) -> Option<EventSink> {
    SINKS
        .lock()
        .entries
        .iter()
        .find(|(entry, _)| *entry == id)
        .map(|(_, sink)| std::sync::Arc::clone(sink))
}

fn context_of(id: usize) -> *mut c_void {
    std::ptr::without_provenance_mut(id)
}

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

/// calls the sink without the registry lock and catches panics so the system thread pool survives
/// event is lost and logged
fn deliver(id: usize, event: SystemActivityEvent) {
    let Some(sink) = sink_of(id) else { return };
    IN_CALLBACK.with(|flag| flag.set(true));
    let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(event)));
    IN_CALLBACK.with(|flag| flag.set(false));
    if delivered.is_err() {
        tracing::error!(target: "riseonly", ?event, "system activity handler panicked");
    }
}

/// runs cancel here or on a new thread when inside a seam callback
/// ip helper deadlocks when you cancel from its own callback
fn cancel_off_callback(cancel: impl FnOnce() + Send + 'static) {
    if !IN_CALLBACK.with(Cell::get) {
        cancel();
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("rise-system-activity-cancel".to_owned())
        .spawn(cancel);
    if let Err(error) = spawned {
        tracing::error!(target: "riseonly", %error, "system activity registration left in place");
    }
}

unsafe extern "system" fn power_changed(
    context: *const c_void,
    kind: u32,
    _setting: *const c_void,
) -> u32 {
    let event = match kind {
        PBT_APMSUSPEND => SystemActivityEvent::WillSleep,
        PBT_APMRESUMEAUTOMATIC => SystemActivityEvent::DidWake,
        _ => return ERROR_SUCCESS.0,
    };
    deliver(context.addr(), event);
    ERROR_SUCCESS.0
}

unsafe extern "system" fn hint_changed(context: *const c_void, hint: NL_NETWORK_CONNECTIVITY_HINT) {
    let satisfied = level_is_satisfied(hint.ConnectivityLevel);
    deliver(context.addr(), SystemActivityEvent::NetworkPath { satisfied });
}

struct PowerRegistration {
    id: usize,
    handle: HPOWERNOTIFY,
    params: Option<Box<DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS>>,
}

struct PowerParams(Box<DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS>);

unsafe impl Send for PowerParams {}

unsafe impl Send for PowerRegistration {}

impl Drop for PowerRegistration {
    fn drop(&mut self) {
        unregister_sink(self.id);
        let handle = self.handle;
        let params = self.params.take().map(PowerParams);
        cancel_off_callback(move || {
            let status = unsafe { PowerUnregisterSuspendResumeNotification(handle) };
            if status != ERROR_SUCCESS {
                tracing::warn!(
                    target: "riseonly",
                    code = status.0,
                    "suspend and resume registration not undone"
                );
            }
            drop(params);
        });
    }
}

struct PathRegistration {
    id: usize,
    handle: HANDLE,
}

struct PathHandle(HANDLE);

impl PathHandle {
    fn into_inner(self) -> HANDLE {
        self.0
    }
}

unsafe impl Send for PathHandle {}

unsafe impl Send for PathRegistration {}

impl Drop for PathRegistration {
    fn drop(&mut self) {
        unregister_sink(self.id);
        let handle = PathHandle(self.handle);
        cancel_off_callback(move || {
            let handle = handle.into_inner();
            let status = unsafe { CancelMibChangeNotify2(handle) };
            if status != ERROR_SUCCESS {
                tracing::warn!(
                    target: "riseonly",
                    code = status.0,
                    "connectivity hint registration not cancelled"
                );
            }
        });
    }
}
