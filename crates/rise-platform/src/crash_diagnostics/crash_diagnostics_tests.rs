use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::crash_diagnostics::*;
use crate::host_os::HostOs;

static SCRATCH: AtomicUsize = AtomicUsize::new(0);

/// fresh directory per test under the system temp dir, removed by the caller
fn scratch_dir(label: &str) -> PathBuf {
    let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "rise-crash-diagnostics-{label}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn collecting_sink() -> (PayloadSink, Arc<Mutex<Vec<Vec<DiagnosticPayload>>>>) {
    let deliveries = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&deliveries);
    let sink: PayloadSink = Arc::new(move |payloads: Vec<DiagnosticPayload>| {
        seen.lock().expect("sink").push(payloads);
    });
    (sink, deliveries)
}

#[test]
fn kinds_keep_the_ios_envelope_spelling() {
    assert_eq!(PayloadKind::Metric.name(), "metric");
    assert_eq!(PayloadKind::Diagnostic.name(), "diagnostic");
}

#[test]
fn only_macos_has_system_reports() {
    assert_eq!(PayloadOrigin::for_host(HostOs::MacOs), PayloadOrigin::SystemReports);
    assert_eq!(PayloadOrigin::for_host(HostOs::Linux), PayloadOrigin::PanicMarkers);
    assert_eq!(PayloadOrigin::for_host(HostOs::Windows), PayloadOrigin::PanicMarkers);
}

#[test]
fn default_answers_name_the_gap_or_the_system() {
    assert_eq!(
        system_reports_by_default(HostOs::MacOs),
        CrashDiagnosticsUnavailable::BranchNotWritten {
            what: DiagnosticsReading::SystemReports,
            host: HostOs::MacOs,
        }
    );
    for host in [HostOs::Linux, HostOs::Windows] {
        assert_eq!(
            system_reports_by_default(host),
            CrashDiagnosticsUnavailable::NoSystemReports(host)
        );
        assert_eq!(backup_exclusion_by_default(host), Ok(BackupExclusion::NotNeeded(host)));
    }
    assert!(backup_exclusion_by_default(HostOs::MacOs).is_err());
}

#[test]
fn readings_fall_back_to_minus_one() {
    let gap = Err(reading_by_default(DiagnosticsReading::ResidentMemory, HostOs::Linux));
    assert_eq!(megabytes_or_failed(gap), READING_FAILED);
    assert_eq!(megabytes_or_failed(Ok(0)), 0);
    assert_eq!(megabytes_or_failed(Ok(BYTES_PER_MB * 3 + BYTES_PER_MB / 2)), 3);
    let no_start = Err(reading_by_default(DiagnosticsReading::ProcessStart, HostOs::Windows));
    assert_eq!(uptime_seconds_or_failed(no_start, 10.0), READING_FAILED);
    assert_eq!(uptime_seconds_or_failed(Ok(f64::NAN), 10.0), READING_FAILED);
}

#[test]
fn uptime_is_whole_seconds_and_never_negative() {
    assert_eq!(uptime_seconds_or_failed(Ok(100.25), 160.9), 60);
    assert_eq!(uptime_seconds_or_failed(Ok(200.0), 150.0), 0, "a clock set back reads zero");
}

#[test]
fn metrickit_timestamps_are_utc_without_zone() {
    assert_eq!(metrickit_timestamp(0.0), "1970-01-01 00:00:00");
    assert_eq!(metrickit_timestamp(951_782_400.5), "2000-02-29 00:00:00");
    assert_eq!(metrickit_timestamp(1_793_709_296.0), "2026-11-03 12:34:56");
    assert_eq!(metrickit_timestamp(-1.0), "1969-12-31 23:59:59");
}

#[test]
fn marker_names_sort_in_capture_order() {
    let early = panic_marker_name(9.999, 42);
    let late = panic_marker_name(10_000.0, 7);
    assert_eq!(early, "0000000000009999-42.json");
    assert!(early < late);
}

#[test]
fn panic_report_is_a_crash_for_the_uploader() {
    let report = PanicReport {
        message: "index out of bounds",
        location: "crates/x/src/lib.rs:12",
        backtrace: "0: main",
        at_seconds_since_1970: 0.0,
        architecture: "x86_64",
    };
    let value: serde_json::Value = serde_json::from_slice(&report.to_json()).expect("json");
    let crashes = value[CRASH_DIAGNOSTICS_KEY].as_array().expect("crash array");
    assert_eq!(crashes.len(), 1, "a non-empty crashDiagnostics array files the report as a crash");
    assert_eq!(value["timeStampBegin"], "1970-01-01 00:00:00");
    let meta = &crashes[0][DIAGNOSTIC_META_DATA_KEY];
    assert_eq!(meta[TERMINATION_REASON_KEY], "Rust panic: index out of bounds");
    assert_eq!(meta["platformArchitecture"], "x86_64");
    assert_eq!(crashes[0][STACK_TRACE_KEY], "panicked at crates/x/src/lib.rs:12\n0: main");
    let entry = crashes[0].as_object().expect("entry");
    assert_eq!(entry.len(), 2, "only diagnosticMetaData and stackTrace: no keys outside the canon");
}

#[test]
fn panic_stack_without_location_is_the_backtrace() {
    let report = PanicReport {
        message: "m",
        location: "",
        backtrace: "0: main",
        at_seconds_since_1970: 0.0,
        architecture: "aarch64",
    };
    assert_eq!(report.stack_trace(), "0: main");
}

#[test]
fn panic_messages_read_both_payload_forms() {
    let literal: Box<dyn std::any::Any + Send> = Box::new("literal");
    let formatted: Box<dyn std::any::Any + Send> = Box::new(String::from("formatted 3"));
    let other: Box<dyn std::any::Any + Send> = Box::new(3_u8);
    assert_eq!(panic_message(literal.as_ref()), "literal");
    assert_eq!(panic_message(formatted.as_ref()), "formatted 3");
    assert_eq!(panic_message(other.as_ref()), "panic with a non-string payload");
}

#[test]
fn markers_are_delivered_oldest_first_then_removed() {
    let dir = scratch_dir("deliver");
    std::fs::write(dir.join(panic_marker_name(2.0, 1)), b"{\"b\":2}").expect("write");
    std::fs::write(dir.join(panic_marker_name(1.0, 1)), b"{\"a\":1}").expect("write");
    std::fs::write(dir.join("notes.txt"), b"not a marker").expect("write");
    std::fs::create_dir_all(dir.join("Nested.json")).expect("dir");

    let (sink, deliveries) = collecting_sink();
    assert_eq!(deliver_panic_markers(&dir, &sink), Ok(2));
    let deliveries = deliveries.lock().expect("sink");
    assert_eq!(deliveries.len(), 1, "one delivery per launch, as MetricKit hands an array");
    let payloads = &deliveries[0];
    assert_eq!(payloads.len(), 2);
    assert!(payloads.iter().all(|payload| payload.kind == PayloadKind::Diagnostic));
    assert_eq!(payloads[0].json_text(), "{\"a\":1}");
    assert_eq!(payloads[1].json_text(), "{\"b\":2}");
    assert_eq!(pending_panic_markers(&dir), Ok(Vec::new()), "delivered markers are removed");
    assert!(dir.join("notes.txt").exists() && dir.join("Nested.json").is_dir());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_markers_means_no_delivery() {
    let dir = scratch_dir("empty");
    let (sink, deliveries) = collecting_sink();
    assert_eq!(deliver_panic_markers(&dir.join("missing"), &sink), Ok(0));
    assert_eq!(deliver_panic_markers(&dir, &sink), Ok(0));
    assert!(deliveries.lock().expect("sink").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prepare_creates_once_and_excludes_only_a_new_directory() {
    let data = scratch_dir("prepare");
    let first = prepare_directory(&data).expect("created");
    assert_eq!(first.path, data.join(DIAGNOSTICS_DIRECTORY));
    assert!(first.path.is_dir());
    assert!(first.backup_exclusion.is_some(), "a new directory is excluded from backup");
    let second = prepare_directory(&data).expect("exists");
    assert_eq!(second.backup_exclusion, None, "the flag is set once, on creation, as on iOS");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn statm_resident_pages_times_page_size() {
    assert_eq!(resident_bytes_from_statm("12345 2048 512 10 0 900 0\n", 4096), Ok(2048 * 4096));
    assert!(resident_bytes_from_statm("12345\n", 4096).is_err(), "no resident field");
    assert!(resident_bytes_from_statm("12345 x 1", 4096).is_err(), "not a number");
    assert!(resident_bytes_from_statm("1 2 3", 0).is_err(), "page size 0");
    assert!(resident_bytes_from_statm("1 18446744073709551615", 4096).is_err(), "overflow");
}

#[test]
fn proc_pid_stat_starttime_survives_a_command_with_spaces_and_parens() {
    let tail = "S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 777 99 100";
    assert_eq!(start_ticks_from_proc_pid_stat(&format!("42 (rise) {tail}")), Ok(777));
    assert_eq!(start_ticks_from_proc_pid_stat(&format!("42 (a ) b (c)) {tail}")), Ok(777));
    assert!(start_ticks_from_proc_pid_stat("42 rise S 1 2").is_err(), "no command field");
    assert!(start_ticks_from_proc_pid_stat("42 (rise) S 1 2").is_err(), "too short");
}

#[test]
fn proc_stat_btime_line() {
    let text = "cpu  1 2 3\nintr 5\nbtime 1762160000\nprocesses 10\n";
    assert_eq!(boot_seconds_from_proc_stat(text), Ok(1_762_160_000));
    assert!(boot_seconds_from_proc_stat("cpu 1 2\n").is_err());
    assert!(boot_seconds_from_proc_stat("btime soon\n").is_err());
    assert!(boot_seconds_from_proc_stat("btime 0\n").is_err());
}

#[test]
fn process_start_adds_ticks_to_boot() {
    assert_eq!(process_start_from_boot(1_000, 250, 100), Ok(1_002.5));
    assert!(process_start_from_boot(1_000, 250, 0).is_err());
    let start = process_start_from_boot(1_000, 250, 100);
    assert_eq!(uptime_seconds_or_failed(start, 1_012.9), 10);
}

#[test]
fn statvfs_available_blocks_prefer_the_fragment_size() {
    assert_eq!(available_bytes_from_statvfs(10, 4096, 65536), Ok(40_960));
    assert_eq!(available_bytes_from_statvfs(10, 0, 512), Ok(5_120));
    assert!(available_bytes_from_statvfs(10, 0, 0).is_err());
    assert!(available_bytes_from_statvfs(u64::MAX, 4096, 4096).is_err());
    assert_eq!(megabytes_or_failed(available_bytes_from_statvfs(256, 4096, 4096)), 1);
}

#[test]
fn filetime_creation_counts_from_1601() {
    let epoch_high = (FILETIME_UNIX_EPOCH >> 32) as u32;
    let epoch_low = FILETIME_UNIX_EPOCH as u32;
    assert_eq!(process_start_from_filetime(epoch_high, epoch_low), Ok(0.0));
    let later = FILETIME_UNIX_EPOCH + 1_762_160_000 * FILETIME_TICKS_PER_SECOND + 5_000_000;
    let start = process_start_from_filetime((later >> 32) as u32, later as u32);
    assert_eq!(start, Ok(1_762_160_000.5));
    assert_eq!(uptime_seconds_or_failed(start, 1_762_160_010.9), 10);
    assert!(process_start_from_filetime(0, 0).is_err());
}
