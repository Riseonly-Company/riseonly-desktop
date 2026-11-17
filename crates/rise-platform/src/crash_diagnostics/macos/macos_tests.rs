use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSNumber, NSURLIsExcludedFromBackupKey};
use rise_core::foundation::foundation_date::RiseClock;

use super::{file_url, NativeCrashDiagnostics};
use crate::crash_diagnostics::*;

fn scratch_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rise-crash-mac-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn resident_memory_is_a_real_amount() {
    let bytes = NativeCrashDiagnostics.resident_memory_bytes().expect("task_info answers");
    assert!(bytes > 1_048_576, "a test process holds more than a megabyte, got {bytes}");
    assert!(resident_memory_mb() >= 1);
}

#[test]
fn process_started_before_now() {
    let start = NativeCrashDiagnostics
        .process_start_seconds_since_1970()
        .expect("sysctl answers");
    let now = RiseClock::now().time_interval_since_1970();
    assert!(start > 1_600_000_000.0 && start <= now + 1.0, "start {start}, now {now}");
    assert!(process_uptime_seconds() >= 0);
}

#[test]
fn free_space_of_the_temp_volume_is_known() {
    let bytes = NativeCrashDiagnostics
        .available_capacity_for_important_usage(&std::env::temp_dir())
        .expect("the volume answers");
    assert!(bytes > 0);
    assert!(available_disk_mb(&std::env::temp_dir()) >= 0);
}

#[test]
fn excluded_directory_carries_the_backup_flag() {
    let dir = scratch_dir("backup");
    assert_eq!(
        NativeCrashDiagnostics.exclude_from_backup(&dir),
        Ok(BackupExclusion::Excluded)
    );
    let url = file_url(&dir, DiagnosticsReading::BackupExclusion).expect("url");
    let mut value: Option<Retained<AnyObject>> = None;
    unsafe { url.getResourceValue_forKey_error(&mut value, NSURLIsExcludedFromBackupKey) }
        .expect("readable");
    let flag = value.expect("set").downcast::<NSNumber>().expect("number");
    assert!(flag.boolValue());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn metric_kit_accepts_and_releases_the_subscriber() {
    let sink: PayloadSink = Arc::new(|_payloads: Vec<DiagnosticPayload>| {});
    let subscription = NativeCrashDiagnostics
        .subscribe_system_reports(sink)
        .expect("MXMetricManager is present on macOS 13");
    drop(subscription);
}
