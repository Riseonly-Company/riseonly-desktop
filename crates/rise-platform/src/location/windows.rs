use std::future::{Future as _, IntoFuture as _};
use std::pin::pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Wake, Waker};

use ::windows::Devices::Geolocation::{
    Geolocator, Geoposition, PositionChangedEventArgs, StatusChangedEventArgs,
};
use ::windows::Foundation::{IReference, PropertyValue, TimeSpan, TypedEventHandler};
use ::windows::Security::Authorization::AppCapabilityAccess::AppCapability;
use ::windows::Win32::Foundation::{
    APPMODEL_ERROR_NO_PACKAGE, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS,
    LPARAM, WPARAM,
};
use ::windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use ::windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, COINIT_MULTITHREADED, CoInitializeEx,
    CoUninitialize,
};
use ::windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
};
use ::windows::Win32::System::Threading::GetCurrentThreadId;
use ::windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    TranslateMessage, WM_NULL,
};
use ::windows::core::{HSTRING, Interface as _, PCWSTR, Ref, w};
use parking_lot::Mutex;

use super::geolocator::*;
use super::location::*;
use crate::capture_permission::consent_store::{
    Consent, access_from_capability_status, access_from_consent,
};
use crate::host_os::HostOs;

const GEOLOCATOR_CLASS: &str = "Windows.Devices.Geolocation.Geolocator";
const APP_CAPABILITY_CLASS: &str = "Windows.Security.Authorization.AppCapabilityAccess.AppCapability";

static SINK: OnceLock<LocationSink> = OnceLock::new();
static LAST_FIX: Mutex<Option<LocationFix>> = Mutex::new(None);
static UPDATES: Mutex<Option<Updates>> = Mutex::new(None);

/// the running standard service and its handler tokens
struct Updates {
    locator: Geolocator,
    position_token: i64,
    status_token: i64,
}

fn emit(event: LocationEvent) {
    let Some(sink) = SINK.get() else {
        return;
    };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(event))).is_err() {
        tracing::error!(target: "riseonly", "location sink panicked");
    }
}

fn framework(class: &'static str) -> impl Fn(::windows::core::Error) -> LocationUnavailable {
    move |error| {
        tracing::debug!(%error, class, "location: WinRT call failed");
        LocationUnavailable::FrameworkMissing(class)
    }
}

fn has_package_identity() -> bool {
    static PACKAGED: OnceLock<bool> = OnceLock::new();
    *PACKAGED.get_or_init(|| {
        let mut length = 0u32;
        let status = unsafe { GetCurrentPackageFullName(&raw mut length, None) };
        match status {
            ERROR_INSUFFICIENT_BUFFER => true,
            APPMODEL_ERROR_NO_PACKAGE => false,
            other => {
                tracing::warn!(?other, "location: package identity is unknown");
                false
            }
        }
    })
}

/// reads the Value string under the subkey, missing key or refused read is Unset
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
        tracing::warn!(code = status.0, "location: ConsentStore value unreadable");
        return Consent::Unset;
    }
    let written = (usize::try_from(size).unwrap_or(0) / 2).min(buffer.len());
    let text = &buffer[..written];
    let end = text.iter().position(|unit| *unit == 0).unwrap_or(text.len());
    Consent::from_value(Some(String::from_utf16_lossy(&text[..end]).as_str()))
}

fn read_level() -> Result<LocationAuthorization, LocationUnavailable> {
    if has_package_identity() {
        let status = AppCapability::Create(&HSTRING::from(LOCATION_CAPABILITY))
            .and_then(|capability| capability.CheckAccess())
            .map_err(framework(APP_CAPABILITY_CLASS))?;
        return Ok(level_from_capability_access(access_from_capability_status(status.0)));
    }
    let key = w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\location");
    let desktop_apps =
        w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\location\NonPackaged");
    Ok(level_from_capability_access(access_from_consent(
        consent(HKEY_LOCAL_MACHINE, key),
        consent(HKEY_CURRENT_USER, key),
        consent(HKEY_CURRENT_USER, desktop_apps),
    )))
}

fn optional(value: ::windows::core::Result<IReference<f64>>) -> Option<f64> {
    value.and_then(|reference| reference.Value()).ok()
}

fn read_fix(position: &Geoposition) -> ::windows::core::Result<LocationFix> {
    let coordinate = position.Coordinate()?;
    let point = coordinate.Point()?.Position()?;
    Ok(WindowsCoordinate {
        latitude: point.Latitude,
        longitude: point.Longitude,
        accuracy: coordinate.Accuracy()?,
        altitude: optional(coordinate.Altitude()),
        altitude_accuracy: optional(coordinate.AltitudeAccuracy()),
        speed: optional(coordinate.Speed()),
        heading: optional(coordinate.Heading()),
        universal_time: coordinate.Timestamp()?.UniversalTime,
    }
    .to_fix())
}

fn remember(fix: LocationFix) -> LocationEvent {
    if fix.is_usable() {
        *LAST_FIX.lock() = Some(fix);
    }
    LocationEvent::Fixes(vec![fix])
}

fn set_accuracy(locator: &Geolocator, accuracy: DesiredAccuracy) -> ::windows::core::Result<()> {
    let meters = PropertyValue::CreateUInt32(accuracy_in_meters(accuracy))?.cast::<IReference<u32>>()?;
    locator.SetDesiredAccuracyInMeters(&meters)
}

fn on_position(args: Ref<PositionChangedEventArgs>) -> ::windows::core::Result<()> {
    emit(remember(read_fix(&args.ok()?.Position()?)?));
    Ok(())
}

fn on_status(args: Ref<StatusChangedEventArgs>) -> ::windows::core::Result<()> {
    let status = args.ok()?.Status()?.0;
    if status_changes_authorization(status) {
        match read_level() {
            Ok(level) => emit(LocationEvent::Authorization(permission_of(level))),
            Err(reason) => tracing::debug!(%reason, "location: level unreadable after a status change"),
        }
    }
    if let Some(failure) = failure_for_status(status) {
        emit(LocationEvent::Failed(failure));
    }
    Ok(())
}

fn start_updates(request: UpdateRequest) -> Result<(), LocationUnavailable> {
    stop_updates();
    let missing = framework(GEOLOCATOR_CLASS);
    let locator = Geolocator::new().map_err(&missing)?;
    set_accuracy(&locator, request.accuracy).map_err(&missing)?;
    locator.SetMovementThreshold(movement_threshold(request)).map_err(&missing)?;
    let position_token = locator
        .PositionChanged(&TypedEventHandler::<Geolocator, PositionChangedEventArgs>::new(|_, args| {
            if let Err(error) = on_position(args) {
                tracing::debug!(%error, "location: position unreadable");
            }
            Ok(())
        }))
        .map_err(&missing)?;
    let status_token = match locator.StatusChanged(&TypedEventHandler::<Geolocator, StatusChangedEventArgs>::new(
        |_, args| {
            if let Err(error) = on_status(args) {
                tracing::debug!(%error, "location: status unreadable");
            }
            Ok(())
        },
    )) {
        Ok(token) => token,
        Err(error) => {
            let _ = locator.RemovePositionChanged(position_token);
            return Err(missing(error));
        }
    };
    *UPDATES.lock() = Some(Updates { locator, position_token, status_token });
    Ok(())
}

fn stop_updates() {
    let Some(updates) = UPDATES.lock().take() else {
        return;
    };
    if let Err(error) = updates.locator.RemovePositionChanged(updates.position_token) {
        tracing::debug!(%error, "location: PositionChanged removal failed");
    }
    if let Err(error) = updates.locator.RemoveStatusChanged(updates.status_token) {
        tracing::debug!(%error, "location: StatusChanged removal failed");
    }
}

fn one_fix() -> LocationEvent {
    let answer = Geolocator::new().and_then(|locator| {
        set_accuracy(&locator, DesiredAccuracy::Best)?;
        locator
            .GetGeopositionAsyncWithAgeAndTimeout(
                TimeSpan { Duration: timespan_ticks(FIX_MAXIMUM_AGE) },
                TimeSpan { Duration: timespan_ticks(FIX_TIMEOUT) },
            )?
            .join()
    });
    match answer.and_then(|position| read_fix(&position)) {
        Ok(fix) => remember(fix),
        Err(error) => {
            tracing::debug!(%error, "location: one-shot fix failed");
            LocationEvent::Failed(failure_for_hresult(error.code().0))
        }
    }
}

fn request_fix() {
    let spawned = std::thread::Builder::new().name("location-fix".into()).spawn(|| {
        let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let event = one_fix();
        if initialized {
            unsafe { CoUninitialize() };
        }
        emit(event);
    });
    if let Err(error) = spawned {
        tracing::warn!(%error, "location: fix thread did not start");
        emit(LocationEvent::Failed(LocationFailure::Other(PLATFORM_FAILURE_CODE)));
    }
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
            tracing::debug!(%error, "location: prompt thread wake failed");
        }
    }
}

fn request_on_prompt_thread() -> LocationAuthorization {
    let initialized =
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let mut message = MSG::default();
    let _ = unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_NOREMOVE) };
    let thread = unsafe { GetCurrentThreadId() };

    let level = match Geolocator::RequestAccessAsync() {
        Ok(operation) => {
            let waker = Waker::from(Arc::new(ThreadWaker { thread }));
            let mut context = Context::from_waker(&waker);
            let mut answer = pin!(operation.into_future());
            loop {
                if let Poll::Ready(result) = answer.as_mut().poll(&mut context) {
                    break match result {
                        Ok(status) => level_from_access_status(status.0),
                        Err(error) => {
                            tracing::debug!(%error, "location: access prompt failed");
                            read_level().unwrap_or(LocationAuthorization::Denied)
                        }
                    };
                }
                let got = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
                if got.0 <= 0 {
                    break read_level().unwrap_or(LocationAuthorization::Denied);
                }
                unsafe {
                    let _ = TranslateMessage(&raw const message);
                    let _ = DispatchMessageW(&raw const message);
                }
            }
        }
        Err(error) => {
            tracing::debug!(%error, "location: RequestAccessAsync unavailable");
            read_level().unwrap_or(LocationAuthorization::Denied)
        }
    };

    if initialized {
        unsafe { CoUninitialize() };
    }
    level
}

fn request_authorization() {
    let spawned = std::thread::Builder::new()
        .name("location-access-prompt".into())
        .spawn(|| emit(LocationEvent::Authorization(permission_of(request_on_prompt_thread()))));
    if let Err(error) = spawned {
        tracing::warn!(%error, "location: prompt thread did not start");
        if let Ok(level) = read_level() {
            emit(LocationEvent::Authorization(permission_of(level)));
        }
    }
}

pub(super) struct NativeLocation;

impl LocationSource for NativeLocation {
    fn permission(&self) -> Result<LocationPermission, LocationUnavailable> {
        read_level().map(permission_of)
    }

    fn last_fix(&self) -> Result<Option<LocationFix>, LocationUnavailable> {
        Ok(*LAST_FIX.lock())
    }

    fn service_available(&self, service: LocationService) -> Result<(), LocationUnavailable> {
        Err(LocationUnavailable::NoSuchService(HostOs::Windows, service))
    }

    fn install(&self, sink: LocationSink) -> Result<(), LocationUnavailable> {
        if SINK.set(sink).is_err() {
            tracing::debug!(target: "riseonly", "location sink was already installed");
        }
        Ok(())
    }

    fn perform(&self, command: LocationCommand) -> Result<(), LocationUnavailable> {
        let no_such = |service| Err(LocationUnavailable::NoSuchService(HostOs::Windows, service));
        match command {
            LocationCommand::RequestAuthorization(_) => {
                request_authorization();
                Ok(())
            }
            LocationCommand::StartUpdates(request) => start_updates(request),
            LocationCommand::StopUpdates => {
                stop_updates();
                Ok(())
            }
            LocationCommand::RequestFix => {
                request_fix();
                Ok(())
            }
            LocationCommand::StartSignificantChanges | LocationCommand::StopSignificantChanges => {
                no_such(LocationService::SignificantChanges)
            }
            LocationCommand::StartVisits | LocationCommand::StopVisits => no_such(LocationService::Visits),
            LocationCommand::MonitorRegion(_)
            | LocationCommand::StopRegion(_)
            | LocationCommand::RequestRegionState(_) => no_such(LocationService::RegionMonitoring),
        }
    }
}
