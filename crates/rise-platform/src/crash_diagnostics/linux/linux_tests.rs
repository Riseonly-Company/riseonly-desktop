use rise_core::foundation::foundation_date::RiseClock;

use super::NativeCrashDiagnostics;
use crate::crash_diagnostics::*;

#[test]
fn resident_memory_is_a_real_amount() {
    let bytes = NativeCrashDiagnostics
        .resident_memory_bytes()
        .expect("/proc/self/statm is readable on every Linux kernel");
    assert!(bytes >= 1_048_576, "a test process holds at least 1 MB, got {bytes} bytes");
}

#[test]
fn process_started_before_now_and_recently() {
    let start = NativeCrashDiagnostics
        .process_start_seconds_since_1970()
        .expect("/proc/self/stat and /proc/stat are readable on every Linux kernel");
    let now = RiseClock::now().time_interval_since_1970();
    assert!(start <= now + 1.0, "start {start} is after now {now}");
    assert!(now - start < 24.0 * 3600.0, "a test process is younger than a day: {start}");
}

#[test]
fn temp_dir_has_free_space() {
    let bytes = NativeCrashDiagnostics
        .available_capacity_for_important_usage(&std::env::temp_dir())
        .expect("statvfs answers for the temp dir");
    assert!(bytes > 0, "the temp dir volume has free space");
}

#[test]
fn missing_path_is_refused() {
    let answer = NativeCrashDiagnostics
        .available_capacity_for_important_usage(std::path::Path::new("/no/such/rise/path"));
    assert!(matches!(
        answer,
        Err(CrashDiagnosticsUnavailable::SystemRefused {
            what: DiagnosticsReading::AvailableCapacity,
            ..
        })
    ));
}
