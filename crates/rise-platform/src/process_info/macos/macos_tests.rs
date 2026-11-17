use objc2_foundation::NSProcessInfoThermalState;

use super::{thermal_state_from, NativeProcessInfo};
use crate::process_info::*;

#[test]
fn physical_memory_is_a_real_amount() {
    let bytes = NativeProcessInfo
        .physical_memory()
        .expect("NSProcessInfo reports the installed RAM on every Mac");
    assert!(
        physical_memory_mb(bytes) >= 1024,
        "every supported Mac ships with at least 1 GB, got {bytes} bytes"
    );
}

#[test]
fn system_uptime_is_positive() {
    let uptime = NativeProcessInfo
        .system_uptime()
        .expect("NSProcessInfo.systemUptime is a finite positive interval");
    assert!(uptime.as_secs_f64() > 0.0);
}

#[test]
fn thermal_state_is_one_of_the_documented_values() {
    let reading = thermal_reading();
    assert_eq!(
        reading.unavailable, None,
        "macOS answers thermalState, so no fallback reason may appear"
    );
}

#[test]
fn every_documented_raw_value_maps_and_an_unknown_one_is_refused() {
    for (raw, expected) in [
        (NSProcessInfoThermalState::Nominal, ThermalState::Nominal),
        (NSProcessInfoThermalState::Fair, ThermalState::Fair),
        (NSProcessInfoThermalState::Serious, ThermalState::Serious),
        (NSProcessInfoThermalState::Critical, ThermalState::Critical),
    ] {
        assert_eq!(thermal_state_from(raw), Ok(expected));
    }
    assert!(matches!(
        thermal_state_from(NSProcessInfoThermalState(9)),
        Err(ProcessInfoUnavailable::SystemRefused { .. })
    ));
}

#[test]
fn the_thermal_observer_registers_and_unregisters_cleanly() {
    let watch = NativeProcessInfo
        .watch_thermal_state(Box::new(|_state| {}))
        .expect("NSNotificationCenter accepts the observer");
    drop(watch);
    let again = NativeProcessInfo
        .watch_thermal_state(Box::new(|_state| {}))
        .expect("a second observer of the same class");
    drop(again);
}

#[test]
fn debugger_attachment_is_answered_by_sysctl() {
    let answer = NativeProcessInfo.is_debugger_attached();
    assert!(
        answer.is_ok(),
        "sysctl kern.proc.pid answers for the own process, got {answer:?}"
    );
    assert_eq!(is_debugger_attached(), answer.unwrap_or(false));
}

#[test]
fn process_cpu_seconds_grows_with_work() {
    let before = NativeProcessInfo
        .process_cpu_seconds()
        .expect("getrusage answers for the own process");
    assert!(before >= 0.0, "got {before}");
    let mut sink = 0u64;
    let mut after = before;
    for round in 0..200_000_000u64 {
        sink = std::hint::black_box(sink.wrapping_add(round));
        if round % 1_000_000 == 0 {
            after = process_cpu_seconds().expect("getrusage answers for the own process");
            if after > before {
                break;
            }
        }
    }
    assert!(after > before, "CPU time stayed at {before} after {sink}");
}
