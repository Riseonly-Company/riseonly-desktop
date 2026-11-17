use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::from_ref;
use std::sync::{Mutex, MutexGuard, PoisonError};

use ::windows::core::Result as WinResult;
use ::windows::Win32::System::Threading::{
    GetCurrentProcess, ProcessPowerThrottling, SetProcessInformation,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE,
};

use super::process_activity::*;
use crate::host_os::HostOs;

pub(super) struct NativeProcessActivity;

static HELD_LEASES: Mutex<usize> = Mutex::new(0);

fn held_leases() -> MutexGuard<'static, usize> {
    HELD_LEASES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// mask 0 lets the system manage all power throttling
/// the execution speed bit with a zero state mask turns throttling off
fn set_power_throttling(control_mask: u32) -> WinResult<()> {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: control_mask,
        StateMask: 0,
    };
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            from_ref(&state).cast::<c_void>(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
}

struct HighQosHold;

impl Drop for HighQosHold {
    fn drop(&mut self) {
        let mut held = held_leases();
        *held -= 1;
        if *held == 0 {
            if let Err(error) = set_power_throttling(0) {
                tracing::warn!(target: "riseonly", %error, "process power throttling not restored");
            }
        }
    }
}

impl ProcessActivitySource for NativeProcessActivity {
    fn begin(&self, reason: &str) -> Result<ProcessActivityToken, ProcessActivityUnavailable> {
        let _ = reason;
        let mut held = held_leases();
        if *held == 0 {
            if let Err(error) = set_power_throttling(PROCESS_POWER_THROTTLING_EXECUTION_SPEED) {
                tracing::debug!(target: "riseonly", %error, "process power throttling not known");
                return Err(ProcessActivityUnavailable::NotThrottled {
                    host: HostOs::current(),
                });
            }
        }
        *held += 1;
        Ok(Box::new(HighQosHold))
    }
}
