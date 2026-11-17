use std::ffi::c_void;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use ::windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, GlobalFree, HANDLE, HGLOBAL, WAIT_EVENT, WAIT_OBJECT_0,
};
use ::windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
    WINHTTP_ACCESS_TYPE_NAMED_PROXY, WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_AUTO_DETECT_TYPE_DHCP,
    WINHTTP_AUTO_DETECT_TYPE_DNS_A, WINHTTP_AUTOPROXY_AUTO_DETECT, WINHTTP_AUTOPROXY_CONFIG_URL,
    WINHTTP_AUTOPROXY_OPTIONS, WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WINHTTP_PROXY_INFO,
    WinHttpCloseHandle, WinHttpGetIEProxyConfigForCurrentUser, WinHttpGetProxyForUrl, WinHttpOpen,
    WinHttpSetTimeouts,
};
use ::windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
    RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW,
};
use ::windows::Win32::System::Threading::{
    CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects,
};
use ::windows::core::{BOOL, PCWSTR, PWSTR, w};

use super::network_proxy::*;
use super::winhttp_proxy::*;
use crate::system_activity::ActivityWatch;

const _: () = assert!(WINHTTP_ACCESS_TYPE_DEFAULT_PROXY.0 == ACCESS_TYPE_DEFAULT_PROXY);
const _: () = assert!(WINHTTP_ACCESS_TYPE_NO_PROXY.0 == ACCESS_TYPE_NO_PROXY);
const _: () = assert!(WINHTTP_ACCESS_TYPE_NAMED_PROXY.0 == ACCESS_TYPE_NAMED_PROXY);
const _: () = assert!(WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY.0 == ACCESS_TYPE_AUTOMATIC_PROXY);
const _: () = assert!(::windows::Win32::Networking::WinHttp::ERROR_WINHTTP_TIMEOUT == ERROR_TIMEOUT);
const _: () = assert!(::windows::Win32::Networking::WinHttp::ERROR_WINHTTP_LOGIN_FAILURE == ERROR_LOGIN_FAILURE);
const _: () = assert!(
    ::windows::Win32::Networking::WinHttp::ERROR_WINHTTP_BAD_AUTO_PROXY_SCRIPT == ERROR_BAD_AUTO_PROXY_SCRIPT
);
const _: () = assert!(
    ::windows::Win32::Networking::WinHttp::ERROR_WINHTTP_UNABLE_TO_DOWNLOAD_SCRIPT
        == ERROR_UNABLE_TO_DOWNLOAD_SCRIPT
);
const _: () = assert!(
    ::windows::Win32::Networking::WinHttp::ERROR_WINHTTP_AUTO_PROXY_SERVICE_ERROR
        == ERROR_AUTO_PROXY_SERVICE_ERROR
);
const _: () = assert!(
    ::windows::Win32::Networking::WinHttp::ERROR_WINHTTP_AUTODETECTION_FAILED == ERROR_AUTODETECTION_FAILED
);

pub(super) struct NativeProxySource;

impl SystemProxySource for NativeProxySource {
    fn watch_settings(&self, sink: ProxySettingsSink) -> Result<ActivityWatch, ProxyUnavailable> {
        let stop = Arc::new(OwnedEvent::new()?);
        let changed = OwnedEvent::new()?;
        let thread_stop = Arc::clone(&stop);
        thread::Builder::new()
            .name(String::from("riseonly-proxy-settings"))
            .spawn(move || {
                if let Err(failure) = follow_settings(&thread_stop, &changed, sink.as_ref()) {
                    tracing::warn!(target: "riseonly", %failure, "system proxy changes are not heard");
                }
            })
            .map_err(|error| ProxyUnavailable::SystemRefused(error.to_string()))?;
        Ok(ActivityWatch::new(StopOnDrop(stop)))
    }

    fn proxies_for(&self, target: &ProxyTarget) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
        let settings = read_ie_settings()?;
        let mut entries = Vec::new();
        if settings.auto_detect {
            match lookup(target, AutoProxyRequest::Discover, AUTO_CONFIG_TIMEOUT) {
                Ok(found) => entries.extend(found),
                Err(failure) => {
                    tracing::debug!(target: "riseonly", %failure, host = %target.host, "proxy discovery skipped");
                }
            }
        }
        entries.extend(settings.configured_entries(target));
        Ok(entries)
    }

    fn run_auto_config(
        &self,
        config: &AutoConfig,
        target: &ProxyTarget,
        timeout: Duration,
    ) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
        lookup(target, AutoProxyRequest::for_config(config)?, timeout)
    }
}

fn follow_settings(
    stop: &OwnedEvent,
    changed: &OwnedEvent,
    sink: &(dyn Fn() + Send),
) -> Result<(), ProxyUnavailable> {
    let key = SettingsKey::open()?;
    key.arm(changed)?;
    let mut known = read_ie_settings().ok();
    let change_signalled = WAIT_EVENT(WAIT_OBJECT_0.0 + 1);
    loop {
        let handles = [stop.0, changed.0];
        let woke = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
        if woke != change_signalled {
            return Ok(());
        }
        key.arm(changed)?;
        let fresh = read_ie_settings().ok();
        if fresh != known {
            known = fresh;
            sink();
        }
    }
}

struct SettingsKey(HKEY);

impl SettingsKey {
    fn open() -> Result<Self, ProxyUnavailable> {
        let mut key = HKEY(std::ptr::null_mut());
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                w!(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings"),
                None,
                KEY_NOTIFY,
                &raw mut key,
            )
        };
        status.ok().map_err(|error| ProxyUnavailable::SystemRefused(error.message()))?;
        Ok(Self(key))
    }

    fn arm(&self, changed: &OwnedEvent) -> Result<(), ProxyUnavailable> {
        let status = unsafe {
            RegNotifyChangeKeyValue(
                self.0,
                true,
                REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET,
                Some(changed.0),
                true,
            )
        };
        status.ok().map_err(|error| ProxyUnavailable::SystemRefused(error.message()))
    }
}

impl Drop for SettingsKey {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

/// auto reset event, starts unsignalled
struct OwnedEvent(HANDLE);

unsafe impl Send for OwnedEvent {}
unsafe impl Sync for OwnedEvent {}

impl OwnedEvent {
    fn new() -> Result<Self, ProxyUnavailable> {
        unsafe { CreateEventW(None, false, false, PCWSTR::null()) }
            .map(Self)
            .map_err(|error| ProxyUnavailable::SystemRefused(error.message()))
    }
}

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

struct StopOnDrop(Arc<OwnedEvent>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        let _ = unsafe { SetEvent(self.0.0) };
    }
}

/// settings of the signed in user, no stored settings at all means direct
fn read_ie_settings() -> Result<IeProxySettings, ProxyUnavailable> {
    let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG {
        fAutoDetect: BOOL(0),
        lpszAutoConfigUrl: PWSTR::null(),
        lpszProxy: PWSTR::null(),
        lpszProxyBypass: PWSTR::null(),
    };
    let outcome = unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config) };
    if let Err(error) = outcome {
        return match win32_code(error.code().0) {
            Some(code) if code == ERROR_FILE_NOT_FOUND.0 => Ok(IeProxySettings::default()),
            code => Err(failure_from_code(code, &error.message(), AUTO_CONFIG_TIMEOUT)),
        };
    }
    let (auto_config_url, proxy_server, proxy_bypass) = unsafe {
        (
            take_global(config.lpszAutoConfigUrl),
            take_global(config.lpszProxy),
            take_global(config.lpszProxyBypass),
        )
    };
    Ok(IeProxySettings::new(config.fAutoDetect.as_bool(), auto_config_url, proxy_server, proxy_bypass))
}

/// runs one auto proxy lookup on its own thread and waits at most timeout plus a grace
fn lookup(
    target: &ProxyTarget,
    request: AutoProxyRequest,
    timeout: Duration,
) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let owned_target = target.clone();
    thread::Builder::new()
        .name(String::from("riseonly-winhttp-proxy"))
        .spawn(move || {
            let _ = sender.send(proxy_for_url(&owned_target, &request, timeout));
        })
        .map_err(|error| ProxyUnavailable::SystemRefused(error.to_string()))?;
    match receiver.recv_timeout(timeout + DEADLINE_GRACE) {
        Ok(answer) => answer,
        Err(RecvTimeoutError::Timeout) => Err(ProxyUnavailable::AutoConfigTimedOut(timeout)),
        Err(RecvTimeoutError::Disconnected) => Err(ProxyUnavailable::SystemRefused(String::from(
            "the WinHTTP proxy lookup ended without an answer",
        ))),
    }
}

fn proxy_for_url(
    target: &ProxyTarget,
    request: &AutoProxyRequest,
    timeout: Duration,
) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
    let session = Session::open(timeout)?;
    let config_url = match request {
        AutoProxyRequest::ConfigUrl(url) => Some(wide(url)),
        AutoProxyRequest::Discover => None,
    };
    let mut options = WINHTTP_AUTOPROXY_OPTIONS::default();
    match &config_url {
        Some(url) => {
            options.dwFlags = WINHTTP_AUTOPROXY_CONFIG_URL;
            options.lpszAutoConfigUrl = PCWSTR::from_raw(url.as_ptr());
        }
        None => {
            options.dwFlags = WINHTTP_AUTOPROXY_AUTO_DETECT;
            options.dwAutoDetectFlags = WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A;
        }
    }
    let url = wide(&target.lookup_url());
    let mut info = WINHTTP_PROXY_INFO::default();
    let mut outcome = unsafe {
        WinHttpGetProxyForUrl(session.0, PCWSTR::from_raw(url.as_ptr()), &mut options, &mut info)
    };
    if let Err(error) = &outcome
        && win32_code(error.code().0) == Some(ERROR_LOGIN_FAILURE)
    {
        options.fAutoLogonIfChallenged = BOOL::from(true);
        outcome = unsafe {
            WinHttpGetProxyForUrl(session.0, PCWSTR::from_raw(url.as_ptr()), &mut options, &mut info)
        };
    }
    if let Err(error) = outcome {
        return Err(failure_from_code(win32_code(error.code().0), &error.message(), timeout));
    }
    let (proxy_list, _bypass) =
        unsafe { (take_global(info.lpszProxy), take_global(info.lpszProxyBypass)) };
    Ok(entries_from_answer(info.dwAccessType.0, proxy_list.as_deref(), target))
}

/// synchronous winhttp session that never uses a proxy itself, the pac download goes direct
struct Session(*mut c_void);

impl Session {
    fn open(timeout: Duration) -> Result<Self, ProxyUnavailable> {
        let handle = unsafe {
            WinHttpOpen(w!("Riseonly"), WINHTTP_ACCESS_TYPE_NO_PROXY, PCWSTR::null(), PCWSTR::null(), 0)
        };
        if handle.is_null() {
            let error = ::windows::core::Error::from_thread();
            return Err(failure_from_code(win32_code(error.code().0), &error.message(), timeout));
        }
        let session = Self(handle);
        let millis = timeout_millis(timeout);
        unsafe { WinHttpSetTimeouts(session.0, millis, millis, millis, millis) }.map_err(|error| {
            failure_from_code(win32_code(error.code().0), &error.message(), timeout)
        })?;
        Ok(session)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// text as nul terminated utf16
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// copies a string winhttp allocated and frees its buffer
/// # Safety
/// text is null or a nul terminated GlobalAlloc string that nothing else frees
unsafe fn take_global(text: PWSTR) -> Option<String> {
    if text.is_null() {
        return None;
    }
    let value = unsafe { text.to_string() }.ok();
    let _ = unsafe { GlobalFree(Some(HGLOBAL(text.0.cast()))) };
    value
}
