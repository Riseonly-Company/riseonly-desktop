use super::NativeProcessInfo;
use crate::host_os::HostOs;
use crate::process_info::*;

#[test]
fn physical_memory_is_a_real_amount() {
    let bytes = NativeProcessInfo
        .physical_memory()
        .expect("/proc/meminfo reports MemTotal on every Linux kernel");
    assert!(
        physical_memory_mb(bytes) >= 256,
        "a machine that runs the app has at least 256 MB, got {bytes} bytes"
    );
}

#[test]
fn system_uptime_is_positive() {
    let uptime = NativeProcessInfo
        .system_uptime()
        .expect("/proc/uptime is readable on every Linux kernel");
    assert!(uptime.as_secs_f64() > 0.0);
}

#[test]
fn thermal_state_is_nominal_with_the_reason() {
    let reading = thermal_reading();
    assert_eq!(reading.state, ThermalState::Nominal);
    assert_eq!(
        reading.unavailable,
        Some(ProcessInfoUnavailable::NoThermalSignal(HostOs::Linux))
    );
}

#[test]
fn proc_self_status_answers_the_debugger_question() {
    NativeProcessInfo
        .is_debugger_attached()
        .expect("/proc/self/status carries TracerPid on every Linux kernel since 2.6");
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
