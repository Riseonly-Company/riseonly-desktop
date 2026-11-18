use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

#[test]
fn crash_text_writes_numbers_without_allocating_and_cuts_overflow() {
    let mut text: CrashText<32> = CrashText::new();
    text.push_decimal(0).push_str(" ").push_signed(-6).push_str(" ").push_hex(0xc000_0005);
    assert_eq!(text.as_bytes(), b"0 -6 0xc0000005");
    let mut small: CrashText<4> = CrashText::new();
    small.push_str("abcdef");
    assert!(small.is_full());
    assert_eq!(small.as_bytes(), b"abcd");
    let mut wide: CrashText<32> = CrashText::new();
    wide.push_utf16(&[0x0043, 0x003a, 0x005c, 0x0444]);
    assert_eq!(wide.as_bytes(), "C:\\\u{444}".as_bytes());
}

#[test]
fn a_native_record_is_named_like_a_panic_marker() {
    let mut name: CrashText<64> = CrashText::new();
    push_native_crash_name(&mut name, 1_700_000_000_123, 42);
    assert_eq!(name.as_bytes(), b"0001700000000123-42.crash");
    assert_eq!(
        MarkerStamp::of(std::path::Path::new("0001700000000123-42.crash")),
        Some(MarkerStamp { millis: 1_700_000_000_123, pid: 42 })
    );
}

#[test]
fn only_file_backed_executable_or_image_start_maps_are_kept() {
    assert!(keep_map_line(b"7f00-7f10 r-xp 00001000 08:01 12 /usr/lib/libc.so.6"));
    assert!(keep_map_line(b"7e00-7e10 r--p 00000000 08:01 12 /usr/lib/libc.so.6"));
    assert!(!keep_map_line(b"7d00-7d10 rw-p 00030000 08:01 12 /usr/lib/libc.so.6"));
    assert!(!keep_map_line(b"7c00-7c10 rw-p 00000000 00:00 0 "));
    assert!(!keep_map_line(b"7b00-7b10 r-xp 00000000 00:00 0 [vdso]"));
}

#[test]
fn the_ucontext_pc_offset_is_known_for_both_linux_architectures() {
    assert_eq!(signal_context_pc_offset("x86_64"), Some(168));
    assert_eq!(signal_context_pc_offset("aarch64"), Some(440));
    assert_eq!(signal_context_pc_offset("riscv64"), None);
}

const WINDOWS_RECORD: &str = "riseonly native crash 1
kind exception
code 0xc0000005
subcode 0
address 0x10
pc 0x7ff800001234 0x7ff800000000
pid 77
thread 4242
at 1700000000123
arch x86_64
frame 0x7ff700005000 0x7ff700000000
frame 0x7ff800001234 0x7ff800000000
frame 0x7ff700006000 0x7ff700000000
module 0x7ff800000000 C:\\Riseonly\\avcodec-61.dll
module 0x7ff700000000 C:\\Riseonly\\riseonly.exe
";

#[test]
fn a_windows_record_names_the_exception_and_the_faulting_module() {
    let record = NativeCrashRecord::parse(WINDOWS_RECORD);
    assert_eq!(
        record.termination_reason(),
        "Native crash: EXCEPTION_ACCESS_VIOLATION (0xc0000005) reading 0x10 at avcodec-61.dll + 0x1234 on thread 4242"
    );
    let frames = record.crash_frames();
    assert_eq!(frames.len(), 2, "the handler frame before the fault is dropped");
    let trace = record.stack_trace();
    assert!(trace.lines().next().unwrap().ends_with("avcodec-61.dll + 0x1234"), "{trace}");
    assert!(trace.lines().nth(1).unwrap().ends_with("riseonly.exe + 0x6000"), "{trace}");
    let json: serde_json::Value = serde_json::from_slice(&record.to_json()).unwrap();
    let crash = &json[CRASH_DIAGNOSTICS_KEY][0];
    assert_eq!(crash[DIAGNOSTIC_META_DATA_KEY]["exceptionCode"], 0xc000_0005_u64);
    assert_eq!(json["timeStampBegin"], "2023-11-14 22:13:20");
}

#[test]
fn a_linux_record_resolves_frames_through_the_maps() {
    let text = "riseonly native crash 1
kind signal
code 11
subcode 1
address 0x0
pc 0x7f0000002010
pid 9
thread riseonly-clip
at 1700000000000
arch aarch64
frame 0x5500000010
frame 0x7f0000002010
map 7f0000000000-7f0000001000 r--p 00000000 08:01 7 /opt/riseonly/lib/libwebrtc.so
map 7f0000001000-7f0000003000 r-xp 00001000 08:01 7 /opt/riseonly/lib/libwebrtc.so
";
    let record = NativeCrashRecord::parse(text);
    assert_eq!(
        record.termination_reason(),
        "Native crash: SIGSEGV (11) SEGV_MAPERR address 0x0 at libwebrtc.so + 0x2010 on thread riseonly-clip"
    );
    assert_eq!(record.crash_frames().len(), 1);
    let json: serde_json::Value = serde_json::from_slice(&record.to_json()).unwrap();
    assert_eq!(json[CRASH_DIAGNOSTICS_KEY][0][DIAGNOSTIC_META_DATA_KEY]["signal"], 11);
}

#[test]
fn an_abort_right_after_a_panic_of_the_same_process_is_not_a_second_crash() {
    let panics = [MarkerStamp { millis: 1_000, pid: 5 }];
    let abort = |pid, at| NativeCrashRecord {
        kind: KIND_SIGNAL.to_owned(),
        code: Some(LINUX_SIGABRT),
        pid: Some(pid),
        at_millis: Some(at),
        ..NativeCrashRecord::default()
    };
    assert!(abort(5, 1_200).follows_a_panic(&panics));
    assert!(!abort(6, 1_200).follows_a_panic(&panics));
    assert!(!abort(5, 1_000 + PANIC_ABORT_WINDOW_MILLIS + 1).follows_a_panic(&panics));
    let segv = NativeCrashRecord { code: Some(11), ..abort(5, 1_200) };
    assert!(!segv.follows_a_panic(&panics));
}

#[test]
fn native_records_are_delivered_with_panic_markers_and_removed() {
    let dir = scratch_dir("native");
    std::fs::write(dir.join("0001700000000123-77.crash"), WINDOWS_RECORD).unwrap();
    std::fs::write(
        dir.join("0000000000001000-5.json"),
        br#"{"crashDiagnostics":[{"stackTrace":"x"}]}"#,
    )
    .unwrap();
    std::fs::write(dir.join("0000000000001100-5.crash"), "kind signal\ncode 6\n").unwrap();
    let (sink, deliveries) = collecting_sink();
    assert_eq!(deliver_panic_markers(&dir, &sink).unwrap(), 2);
    let deliveries = deliveries.lock().unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].len(), 2);
    let native: serde_json::Value = serde_json::from_slice(&deliveries[0][1].json).unwrap();
    assert!(native[CRASH_DIAGNOSTICS_KEY][0][STACK_TRACE_KEY].as_str().unwrap().contains("avcodec-61.dll"));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "the superseded abort is removed too");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_hang_record_goes_out_as_hang_diagnostics() {
    let text = "riseonly native crash 1
kind hang
pc 0x7ff800001234 0x7ff800000000
thread 4242
at 1700000000123
arch x86_64
frame 0x7ff800001234 0x7ff800000000
module 0x7ff800000000 C:\\Windows\\System32\\ntdll.dll
";
    let record = NativeCrashRecord::parse(text);
    assert_eq!(
        record.termination_reason(),
        "Hang: main thread not responding at ntdll.dll + 0x1234 on thread 4242"
    );
    let json: serde_json::Value = serde_json::from_slice(&record.to_json()).unwrap();
    assert!(json[HANG_DIAGNOSTICS_KEY][0][STACK_TRACE_KEY].as_str().unwrap().contains("ntdll.dll + 0x1234"));
    assert!(json.get(CRASH_DIAGNOSTICS_KEY).is_none());
}

#[test]
fn a_hang_is_a_silence_of_the_canon_threshold_and_beats_off_macos() {
    assert!(!is_hang(None));
    assert!(!is_hang(Some(hang_report_after() - Duration::from_millis(1))));
    assert!(is_hang(Some(hang_report_after())));
    assert_eq!(main_thread_beat_interval() * 5, hang_report_after());
    assert!(main_thread_beats_needed(HostOs::Linux));
    assert!(main_thread_beats_needed(HostOs::Windows));
    assert!(!main_thread_beats_needed(HostOs::MacOs));
}

#[test]
fn a_silence_right_after_a_sleep_of_the_machine_is_not_a_hang() {
    let interval = main_thread_beat_interval();
    assert!(!is_wake_gap(interval));
    assert!(is_wake_gap(interval * 4));
    let long_silence = Some(hang_report_after() * 10);
    assert!(!is_hang_while_awake(long_silence, interval));
    assert!(is_hang_while_awake(long_silence, hang_report_after()));
    assert!(!is_hang_while_awake(Some(interval), hang_report_after() * 2));
}

const BEX_REPORT: &str = "Version=1\r\nEventType=BEX64\r\nEventTime=134080323510000000\r\nAppSessionGuid=000014c4-0001-001a-c9c1-7f09bfd7dc01\r\nSig[0].Name=App\r\nSig[0].Value=riseonly.exe\r\nSig[3].Name=Module\r\nSig[3].Value=ntdll.dll\r\nSig[6].Name=Offset\r\nSig[6].Value=0000000000255e4c\r\nSig[7].Name=Code\r\nSig[7].Value=c0000409\r\nSig[8].Name=Data\r\nSig[8].Value=0000000000000023\r\nAppPath=C:\\Riseonly\\riseonly.exe\r\n";

#[test]
fn a_fail_fast_report_of_windows_error_reporting_reads_by_position() {
    let crash = WerCrash::parse(BEX_REPORT).expect("bex report");
    assert_eq!(crash.module, "ntdll.dll");
    assert_eq!(crash.offset, Some(0x255e4c));
    assert_eq!(crash.code, Some(STATUS_FAIL_FAST));
    assert_eq!(crash.data, Some(35));
    assert_eq!(crash.pid, Some(0x14c4));
    assert_eq!(crash.at_millis, Some((134_080_323_510_000_000 - FILETIME_UNIX_EPOCH) / 10_000));
    let appcrash = BEX_REPORT
        .replace("BEX64", "APPCRASH")
        .replace("Sig[6].Value=0000000000255e4c", "Sig[6].Value=c0000005")
        .replace("Sig[7].Value=c0000409", "Sig[7].Value=0000000000001234");
    let crash = WerCrash::parse(&appcrash).expect("appcrash report");
    assert_eq!((crash.code, crash.offset), (Some(0xC000_0005), Some(0x1234)));
    assert!(WerCrash::parse(&BEX_REPORT.replace("BEX64", "AppHangB1")).is_none());
}

#[test]
fn a_fail_fast_record_names_the_module_offset_and_the_fail_fast_code() {
    let crash = WerCrash::parse(BEX_REPORT).expect("bex report");
    let record = NativeCrashRecord::parse(&crash.record_text("x86_64"));
    let reason = record.termination_reason();
    assert!(reason.starts_with("Native crash: STATUS_STACK_BUFFER_OVERRUN (0xc0000409)"), "{reason}");
    assert!(reason.contains("fail fast 35 unexpected heap exception"), "{reason}");
    assert!(reason.contains("at ntdll.dll + 0x255e4c"), "{reason}");
    assert!(reason.ends_with("(windows error reporting)"), "{reason}");
    assert_eq!(record.pid, Some(0x14c4));
    let json: serde_json::Value = serde_json::from_slice(&record.to_json()).expect("json");
    assert_eq!(json[CRASH_DIAGNOSTICS_KEY][0][DIAGNOSTIC_META_DATA_KEY]["exceptionCode"], STATUS_FAIL_FAST);
}

#[test]
fn only_fresh_reports_of_this_exe_without_an_own_record_are_taken_once() {
    let at = (134_080_323_510_000_000 - FILETIME_UNIX_EPOCH) / 10_000;
    let other = BEX_REPORT.replace("C:\\Riseonly\\riseonly.exe", "C:\\Other\\riseonly.exe");
    let reports = vec![
        ("AppCrash_a".to_owned(), BEX_REPORT.to_owned()),
        ("AppCrash_b".to_owned(), other),
        ("AppCrash_c".to_owned(), BEX_REPORT.to_owned()),
    ];
    let exe = "c:\\riseonly\\RISEONLY.exe";
    let (chosen, seen) = select_wer_crashes(&reports, &["AppCrash_c".to_owned()], &[], exe, at + 1_000);
    assert_eq!(chosen.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["AppCrash_a"]);
    assert_eq!(seen, ["AppCrash_a", "AppCrash_b", "AppCrash_c"]);
    let (chosen, _) = select_wer_crashes(&reports, &[], &[0x14c4], exe, at + 1_000);
    assert!(chosen.is_empty());
    let (chosen, _) = select_wer_crashes(&reports, &[], &[], exe, at + WER_REPORT_MAX_AGE_MILLIS + 1);
    assert!(chosen.is_empty());
}

#[test]
fn a_silence_while_a_system_panel_is_open_is_not_a_hang() {
    let long_silence = Some(hang_report_after() * 10);
    let awake = hang_report_after() * 2;
    let guard = NativeModalGuard::enter();
    let nested = NativeModalGuard::enter();
    assert!(native_modal_open());
    assert!(!is_hang_while_awake(long_silence, awake));
    drop(nested);
    assert!(!is_hang_while_awake(long_silence, awake));
    drop(guard);
    assert!(!native_modal_open());
    assert!(is_hang_while_awake(long_silence, awake));
}
