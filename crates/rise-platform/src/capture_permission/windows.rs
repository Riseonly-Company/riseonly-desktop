use std::future::{Future as _, IntoFuture as _};
use std::pin::pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Wake, Waker};

use ::windows::Security::Authorization::AppCapabilityAccess::AppCapability;
use ::windows::Win32::Foundation::{
    APPMODEL_ERROR_NO_PACKAGE, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS,
    LPARAM, WPARAM,
};
use ::windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use ::windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use ::windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
};
use ::windows::Win32::System::Threading::GetCurrentThreadId;
use ::windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    TranslateMessage, WM_NULL,
};
use ::windows::core::{HSTRING, PCWSTR, w};

use super::capture_permission::*;
use super::consent_store::*;

const APP_CAPABILITY_CLASS: &str = "Windows.Security.Authorization.AppCapabilityAccess.AppCapability";

fn has_package_identity() -> bool {
    static PACKAGED: OnceLock<bool> = OnceLock::new();
    *PACKAGED.get_or_init(|| {
        let mut length = 0u32;
        let status = unsafe { GetCurrentPackageFullName(&raw mut length, None) };
        match status {
            ERROR_INSUFFICIENT_BUFFER => true,
            APPMODEL_ERROR_NO_PACKAGE => false,
            other => {
                tracing::warn!(?other, "capture access: package identity is unknown");
                false
            }
        }
    })
}

fn capability(kind: CaptureKind) -> Result<AppCapability, CaptureUnavailable> {
    AppCapability::Create(&HSTRING::from(capability_name(kind))).map_err(|error| {
        tracing::debug!(%error, ?kind, "AppCapability.Create failed");
        CaptureUnavailable::FrameworkMissing(APP_CAPABILITY_CLASS)
    })
}

fn packaged_status(kind: CaptureKind) -> Result<CaptureAccess, CaptureUnavailable> {
    let status = capability(kind)?.CheckAccess().map_err(|error| {
        tracing::debug!(%error, ?kind, "AppCapability.CheckAccess failed");
        CaptureUnavailable::FrameworkMissing(APP_CAPABILITY_CLASS)
    })?;
    Ok(access_from_capability_status(status.0))
}

const fn consent_key(kind: CaptureKind) -> PCWSTR {
    match kind {
        CaptureKind::Camera => {
            w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam")
        }
        CaptureKind::Microphone => {
            w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone")
        }
    }
}

const fn desktop_apps_key(kind: CaptureKind) -> PCWSTR {
    match kind {
        CaptureKind::Camera => w!(
            r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam\NonPackaged"
        ),
        CaptureKind::Microphone => w!(
            r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone\NonPackaged"
        ),
    }
}

/// reg_sz value under subkey, a missing key or value is unset
/// a read the registry refuses is logged and also unset, so the capture reports the failure itself
fn consent(root: HKEY, subkey: PCWSTR) -> Consent {
    let mut buffer = [0_u16; 16];
    let mut size = u32::try_from(std::mem::size_of_val(&buffer)).unwrap_or(u32::MAX);
    let status = unsafe {
        RegGetValueW(
            root,
            subkey,
            w!("Value"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&raw mut size),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Consent::Unset;
    }
    if status != ERROR_SUCCESS {
        tracing::warn!(code = status.0, "capture access: ConsentStore value unreadable");
        return Consent::Unset;
    }
    let written = (usize::try_from(size).unwrap_or(0) / 2).min(buffer.len());
    let text = &buffer[..written];
    let end = text.iter().position(|unit| *unit == 0).unwrap_or(text.len());
    Consent::from_value(Some(String::from_utf16_lossy(&text[..end]).as_str()))
}

fn unpackaged_status(kind: CaptureKind) -> CaptureAccess {
    access_from_consent(
        consent(HKEY_LOCAL_MACHINE, consent_key(kind)),
        consent(HKEY_CURRENT_USER, consent_key(kind)),
        consent(HKEY_CURRENT_USER, desktop_apps_key(kind)),
    )
}

struct ThreadWaker {
    thread: u32,
}

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let posted = unsafe { PostThreadMessageW(self.thread, WM_NULL, WPARAM(0), LPARAM(0)) };
        if let Err(error) = posted {
            tracing::debug!(%error, "capture access: prompt thread wake failed");
        }
    }
}

fn request_on_prompt_thread(kind: CaptureKind) -> bool {
    let initialized =
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let mut message = MSG::default();
    let _ = unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_NOREMOVE) };
    let thread = unsafe { GetCurrentThreadId() };

    let allowed = match capability(kind).and_then(|capability| {
        capability.RequestAccessAsync().map_err(|error| {
            tracing::debug!(%error, ?kind, "AppCapability.RequestAccessAsync failed");
            CaptureUnavailable::FrameworkMissing(APP_CAPABILITY_CLASS)
        })
    }) {
        Ok(operation) => {
            let waker = Waker::from(Arc::new(ThreadWaker { thread }));
            let mut context = Context::from_waker(&waker);
            let mut answer = pin!(operation.into_future());
            loop {
                if let Poll::Ready(result) = answer.as_mut().poll(&mut context) {
                    break match result {
                        Ok(status) => access_from_capability_status(status.0).is_granted(),
                        Err(error) => {
                            tracing::debug!(%error, ?kind, "capture access prompt failed");
                            false
                        }
                    };
                }
                let got = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
                if got.0 <= 0 {
                    break false;
                }
                unsafe {
                    let _ = TranslateMessage(&raw const message);
                    let _ = DispatchMessageW(&raw const message);
                }
            }
        }
        Err(_) => false,
    };

    if initialized {
        unsafe { CoUninitialize() };
    }
    allowed
}

pub(super) struct NativeCaptureAccess;

impl CaptureAccessSource for NativeCaptureAccess {
    fn status(&self, kind: CaptureKind) -> Result<CaptureAccess, CaptureUnavailable> {
        if has_package_identity() {
            packaged_status(kind)
        } else {
            Ok(unpackaged_status(kind))
        }
    }

    fn request(&self, kind: CaptureKind, done: AccessAnswer) -> Result<(), CaptureUnavailable> {
        if !has_package_identity() {
            done(unpackaged_status(kind).is_granted());
            return Ok(());
        }
        let answer = Arc::new(parking_lot::Mutex::new(Some(done)));
        let on_thread = Arc::clone(&answer);
        let spawned = std::thread::Builder::new()
            .name("capture-access-prompt".into())
            .spawn(move || {
                let allowed = request_on_prompt_thread(kind);
                if let Some(done) = on_thread.lock().take() {
                    done(allowed);
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "capture access: prompt thread did not start");
            if let Some(done) = answer.lock().take() {
                done(false);
            }
        }
        Ok(())
    }
}
