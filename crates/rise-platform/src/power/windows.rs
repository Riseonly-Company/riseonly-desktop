use std::ffi::c_void;

use windows::core::GUID;
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerSettingRegisterNotification, PowerSettingUnregisterNotification,
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, HPOWERNOTIFY, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING, GUID_POWER_SAVING_STATUS,
};
use windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK;

use super::power::*;

const WATCHED_SETTINGS: [GUID; 3] = [
    GUID_POWER_SAVING_STATUS,
    GUID_BATTERY_PERCENTAGE_REMAINING,
    GUID_ACDC_POWER_SOURCE,
];

pub(super) struct NativePower;

impl PowerSource for NativePower {
    fn low_power_mode(&self) -> Result<bool, PowerUnavailable> {
        let status = system_power_status(PowerReadingKind::LowPowerMode)?;
        Ok(low_power_mode_from_system_status(status.SystemStatusFlag))
    }

    fn battery(&self) -> Result<BatteryReading, PowerUnavailable> {
        let status = system_power_status(PowerReadingKind::Battery)?;
        Ok(battery_reading_from_system_power_status(
            status.ACLineStatus,
            status.BatteryFlag,
            status.BatteryLifePercent,
        ))
    }

    fn watch(&self, sink: ChangeSink) -> Result<PowerWatch, PowerUnavailable> {
        let sink = Box::new(sink);
        let context: *const ChangeSink = &*sink;
        let mut parameters = Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(on_power_setting),
            Context: context.cast_mut().cast::<c_void>(),
        });
        let recipient = HANDLE(std::ptr::from_mut(&mut *parameters).cast::<c_void>());
        let mut watch = Watch {
            handles: Vec::with_capacity(WATCHED_SETTINGS.len()),
            _parameters: parameters,
            _sink: sink,
        };
        let mut failures = Vec::new();
        for setting in &WATCHED_SETTINGS {
            let mut handle: *mut c_void = std::ptr::null_mut();
            let result = unsafe {
                PowerSettingRegisterNotification(
                    setting,
                    DEVICE_NOTIFY_CALLBACK,
                    recipient,
                    &mut handle,
                )
            };
            if result == ERROR_SUCCESS && !handle.is_null() {
                watch.handles.push(HPOWERNOTIFY(handle as isize));
            } else {
                failures.push(format!("{setting:?}: {}", result.0));
            }
        }
        if watch.handles.is_empty() {
            return Err(PowerUnavailable::SystemRefused {
                what: PowerReadingKind::Changes,
                detail: format!("PowerSettingRegisterNotification failed: {}", failures.join(", ")),
            });
        }
        Ok(PowerWatch::new(watch))
    }
}

fn system_power_status(what: PowerReadingKind) -> Result<SYSTEM_POWER_STATUS, PowerUnavailable> {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) }.map_err(|error| PowerUnavailable::SystemRefused {
        what,
        detail: format!("GetSystemPowerStatus: {error}"),
    })?;
    Ok(status)
}

struct Watch {
    handles: Vec<HPOWERNOTIFY>,
    _parameters: Box<DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS>,
    _sink: Box<ChangeSink>,
}

unsafe impl Send for Watch {}

impl Drop for Watch {
    fn drop(&mut self) {
        for handle in self.handles.drain(..) {
            let _ = unsafe { PowerSettingUnregisterNotification(handle) };
        }
    }
}

unsafe extern "system" fn on_power_setting(
    context: *const c_void,
    _kind: u32,
    _setting: *const c_void,
) -> u32 {
    let sink = unsafe { &*context.cast::<ChangeSink>() };
    let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink()));
    if delivered.is_err() {
        tracing::error!(target: "riseonly", "power change handler panicked");
    }
    ERROR_SUCCESS.0
}
