use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::sync::{Arc, OnceLock};

use rise_canon::RiseTimingCanon;
use rise_core::foundation::foundation_date::RiseClock;
use thiserror::Error;

use crate::host_os::HostOs;

pub const DIAGNOSTICS_DIRECTORY: &str = "Diagnostics";

pub const PANIC_MARKER_DIRECTORY: &str = "PanicMarkers";

pub const CRASH_DIAGNOSTICS_KEY: &str = "crashDiagnostics";

pub const HANG_DIAGNOSTICS_KEY: &str = "hangDiagnostics";

pub const DIAGNOSTIC_META_DATA_KEY: &str = "diagnosticMetaData";

pub const TERMINATION_REASON_KEY: &str = "terminationReason";

pub const STACK_TRACE_KEY: &str = "stackTrace";

pub const PANIC_MARKER_EXTENSION: &str = "json";

pub const BYTES_PER_MB: u64 = 1_048_576;

pub const READING_FAILED: i64 = -1;

/// which metrickit callback a payload came from, the spelling is the envelope kind
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PayloadKind {
    Metric,
    Diagnostic,
}

impl PayloadKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Metric => "metric",
            Self::Diagnostic => "diagnostic",
        }
    }
}

/// one payload exactly as the system made it, never re-encoded
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiagnosticPayload {
    pub kind: PayloadKind,
    pub json: Vec<u8>,
}

impl DiagnosticPayload {
    /// payload as the envelope carries it, as a string
    /// ios loses the whole payload on a single non utf8 byte, here only such bytes become U+FFFD
    pub fn json_text(&self) -> String {
        String::from_utf8_lossy(&self.json).into_owned()
    }
}

/// where payloads arrive, one call per delivery
/// on a metrickit queue on macos or the thread that called install for panic markers
/// must not block, the feature stamps the envelope and hands the write to its own queue
pub type PayloadSink = Arc<dyn Fn(Vec<DiagnosticPayload>) + Send + Sync>;

/// where the crash reports of a host come from
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PayloadOrigin {
    SystemReports,
    PanicMarkers,
}

impl PayloadOrigin {
    pub const fn for_host(host: HostOs) -> Self {
        match host {
            HostOs::MacOs => Self::SystemReports,
            HostOs::Linux | HostOs::Windows => Self::PanicMarkers,
        }
    }
}

/// which part of the seam could not answer
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DiagnosticsReading {
    SystemReports,
    PanicMarkers,
    BackupExclusion,
    ResidentMemory,
    ProcessStart,
    AvailableCapacity,
    NativeCrashes,
    HangReports,
}

impl std::fmt::Display for DiagnosticsReading {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::SystemReports => "system crash and metric reports",
            Self::PanicMarkers => "panic markers",
            Self::BackupExclusion => "backup exclusion",
            Self::ResidentMemory => "resident memory",
            Self::ProcessStart => "process start time",
            Self::AvailableCapacity => "available capacity for important usage",
            Self::NativeCrashes => "native crash handler",
            Self::HangReports => "main thread hang reports",
        };
        formatter.write_str(text)
    }
}

/// why a part of the seam is missing, every public reader still has a working answer
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum CrashDiagnosticsUnavailable {
    #[error("{what} is not written for {host:?}")]
    BranchNotWritten { what: DiagnosticsReading, host: HostOs },
    #[error("{0:?} delivers no crash or metric reports to applications")]
    NoSystemReports(HostOs),
    #[error("{what} needs {needs}")]
    SystemTooOld { what: DiagnosticsReading, needs: &'static str },
    #[error("the system refused {what}: {detail}")]
    SystemRefused { what: DiagnosticsReading, detail: String },
    #[error("{what}: {detail}")]
    Io { what: DiagnosticsReading, detail: String },
}

impl CrashDiagnosticsUnavailable {
    fn io(what: DiagnosticsReading, error: &std::io::Error) -> Self {
        Self::Io {
            what,
            detail: error.to_string(),
        }
    }
}

/// answer of a backup exclusion that did not fail
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BackupExclusion {
    Excluded,
    NotNeeded(HostOs),
}

/// default answer of an os layer for the payload subscription
pub const fn system_reports_by_default(host: HostOs) -> CrashDiagnosticsUnavailable {
    match PayloadOrigin::for_host(host) {
        PayloadOrigin::SystemReports => CrashDiagnosticsUnavailable::BranchNotWritten {
            what: DiagnosticsReading::SystemReports,
            host,
        },
        PayloadOrigin::PanicMarkers => CrashDiagnosticsUnavailable::NoSystemReports(host),
    }
}

/// default answer of an os layer for the backup exclusion
pub const fn backup_exclusion_by_default(
    host: HostOs,
) -> Result<BackupExclusion, CrashDiagnosticsUnavailable> {
    match host {
        HostOs::MacOs => Err(CrashDiagnosticsUnavailable::BranchNotWritten {
            what: DiagnosticsReading::BackupExclusion,
            host,
        }),
        HostOs::Linux | HostOs::Windows => Ok(BackupExclusion::NotNeeded(host)),
    }
}

/// default answer of an os layer for a process reading it doesnt override
pub const fn reading_by_default(
    what: DiagnosticsReading,
    host: HostOs,
) -> CrashDiagnosticsUnavailable {
    CrashDiagnosticsUnavailable::BranchNotWritten { what, host }
}

pub trait CrashDiagnosticsSource {
    fn subscribe_system_reports(
        &self,
        sink: PayloadSink,
    ) -> Result<DiagnosticsSubscription, CrashDiagnosticsUnavailable> {
        drop(sink);
        Err(system_reports_by_default(HostOs::current()))
    }

    fn exclude_from_backup(
        &self,
        directory: &Path,
    ) -> Result<BackupExclusion, CrashDiagnosticsUnavailable> {
        let _ = directory;
        backup_exclusion_by_default(HostOs::current())
    }

    fn resident_memory_bytes(&self) -> Result<u64, CrashDiagnosticsUnavailable> {
        Err(reading_by_default(DiagnosticsReading::ResidentMemory, HostOs::current()))
    }

    fn process_start_seconds_since_1970(&self) -> Result<f64, CrashDiagnosticsUnavailable> {
        Err(reading_by_default(DiagnosticsReading::ProcessStart, HostOs::current()))
    }

    fn available_capacity_for_important_usage(
        &self,
        path: &Path,
    ) -> Result<u64, CrashDiagnosticsUnavailable> {
        let _ = path;
        Err(reading_by_default(DiagnosticsReading::AvailableCapacity, HostOs::current()))
    }

    fn install_native_crash_handler(
        &self,
        marker_dir: &Path,
    ) -> Result<(), CrashDiagnosticsUnavailable> {
        let _ = marker_dir;
        Err(reading_by_default(DiagnosticsReading::NativeCrashes, HostOs::current()))
    }

    /// call on the main thread, it is the thread the watch samples
    fn install_hang_watch(&self, marker_dir: &Path) -> Result<(), CrashDiagnosticsUnavailable> {
        let _ = marker_dir;
        Err(reading_by_default(DiagnosticsReading::HangReports, HostOs::current()))
    }
}

/// keeps a payload subscription alive, dropping it stops further deliveries
#[must_use = "dropping the subscription stops crash and metric deliveries"]
pub struct DiagnosticsSubscription {
    _subscriber: Box<dyn Send>,
}

impl DiagnosticsSubscription {
    /// subscriber unregisters itself from the system in its own drop
    pub fn new(subscriber: impl Send + 'static) -> Self {
        Self {
            _subscriber: Box::new(subscriber),
        }
    }
}

impl std::fmt::Debug for DiagnosticsSubscription {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DiagnosticsSubscription")
    }
}

pub fn diagnostics_directory(data_dir: &Path) -> PathBuf {
    data_dir.join(DIAGNOSTICS_DIRECTORY)
}

/// the diagnostics directory once it exists
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiagnosticsDirectory {
    pub path: PathBuf,
    pub backup_exclusion: Option<Result<BackupExclusion, CrashDiagnosticsUnavailable>>,
}

/// creates Diagnostics inside the data dir if needed and excludes a new one from backup
/// they describe one machine so restoring elsewhere is useless, crash stacks stay out of backups
pub fn prepare_directory(
    data_dir: &Path,
) -> Result<DiagnosticsDirectory, CrashDiagnosticsUnavailable> {
    let path = diagnostics_directory(data_dir);
    if path.is_dir() {
        return Ok(DiagnosticsDirectory {
            path,
            backup_exclusion: None,
        });
    }
    std::fs::create_dir_all(&path).map_err(|error| {
        CrashDiagnosticsUnavailable::io(DiagnosticsReading::BackupExclusion, &error)
    })?;
    let exclusion = super::os::NativeCrashDiagnostics.exclude_from_backup(&path);
    if let Err(reason) = &exclusion {
        tracing::warn!(target: "riseonly", %reason, "diagnostics directory stays in backups");
    }
    Ok(DiagnosticsDirectory {
        path,
        backup_exclusion: Some(exclusion),
    })
}

pub fn install(
    diagnostics_dir: &Path,
    sink: PayloadSink,
) -> Result<DiagnosticsSubscription, CrashDiagnosticsUnavailable> {
    match PayloadOrigin::for_host(HostOs::current()) {
        PayloadOrigin::SystemReports => {
            super::os::NativeCrashDiagnostics.subscribe_system_reports(sink)
        }
        PayloadOrigin::PanicMarkers => {
            let marker_dir = diagnostics_dir.join(PANIC_MARKER_DIRECTORY);
            install_panic_hook(marker_dir.clone());
            if let Err(reason) =
                super::os::NativeCrashDiagnostics.install_native_crash_handler(&marker_dir)
            {
                tracing::warn!(target: "riseonly", %reason, "native crashes are not recorded");
            }
            if let Err(reason) = super::os::NativeCrashDiagnostics.install_hang_watch(&marker_dir) {
                tracing::warn!(target: "riseonly", %reason, "main thread hangs are not recorded");
            }
            deliver_panic_markers(&marker_dir, &sink)?;
            Ok(DiagnosticsSubscription::new(PanicHookInstalled))
        }
    }
}

struct PanicHookInstalled;

static PANIC_MARKER_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn install_panic_hook(marker_dir: PathBuf) {
    if PANIC_MARKER_DIR.set(marker_dir).is_err() {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = PANIC_MARKER_DIR.get() {
            write_panic_marker(dir, info);
        }
        previous(info);
    }));
}

fn write_panic_marker(dir: &Path, info: &std::panic::PanicHookInfo<'_>) {
    let message = panic_message(info.payload());
    let location = info
        .location()
        .map(|location| format!("{}:{}", location.file(), location.line()))
        .unwrap_or_default();
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    let at = RiseClock::now().time_interval_since_1970();
    let report = PanicReport {
        message: &message,
        location: &location,
        backtrace: &backtrace,
        at_seconds_since_1970: at,
        architecture: std::env::consts::ARCH,
    };
    let name = panic_marker_name(at, std::process::id());
    if std::fs::create_dir_all(dir).is_ok() {
        let _ = std::fs::write(dir.join(name), report.to_json());
    }
}

/// text of a panic payload, from panic with a literal or a formatted message
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "panic with a non-string payload".to_owned()
    }
}

/// milliseconds since 1970 (16 digits) dash pid dot json, so lexical order is capture order
pub fn panic_marker_name(at_seconds_since_1970: f64, pid: u32) -> String {
    let millis = (at_seconds_since_1970.max(0.0) * 1000.0) as u64;
    format!("{millis:016}-{pid}.{PANIC_MARKER_EXTENSION}")
}

/// one recorded panic, shaped as a diagnostic payload with one crash diagnostic
#[derive(Clone, Copy, Debug)]
pub struct PanicReport<'a> {
    pub message: &'a str,
    pub location: &'a str,
    pub backtrace: &'a str,
    pub at_seconds_since_1970: f64,
    pub architecture: &'a str,
}

impl PanicReport<'_> {
    /// one crash entry in the form android files an uncaught exception
    /// termination reason is the panic message, text stack goes to stack trace
    /// with the panic location first, timestamps and architecture keys match a macos payload
    pub fn to_json(&self) -> Vec<u8> {
        let stamp = metrickit_timestamp(self.at_seconds_since_1970);
        let value = serde_json::json!({
            "timeStampBegin": stamp,
            "timeStampEnd": stamp,
            (CRASH_DIAGNOSTICS_KEY): [{
                (DIAGNOSTIC_META_DATA_KEY): {
                    (TERMINATION_REASON_KEY): format!("Rust panic: {}", self.message),
                    "platformArchitecture": self.architecture,
                },
                (STACK_TRACE_KEY): self.stack_trace(),
            }],
        });
        serde_json::to_vec(&value).unwrap_or_default()
    }

    /// panicked at file and line then the captured backtrace
    /// without a location just the backtrace
    pub fn stack_trace(&self) -> String {
        if self.location.is_empty() {
            self.backtrace.to_owned()
        } else {
            format!("panicked at {}\n{}", self.location, self.backtrace)
        }
    }
}

/// yyyy-MM-dd HH:mm:ss in utc, the timestamp form of metrickit payloads
pub fn metrickit_timestamp(seconds_since_1970: f64) -> String {
    let total = seconds_since_1970.floor() as i64;
    let days = total.div_euclid(86_400);
    let second_of_day = total.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        second_of_day / 3600,
        second_of_day % 3600 / 60,
        second_of_day % 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = (if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 }) as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// markers of earlier runs, oldest first and only regular .json files count
pub fn pending_panic_markers(
    marker_dir: &Path,
) -> Result<Vec<PathBuf>, CrashDiagnosticsUnavailable> {
    pending_with_extension(marker_dir, PANIC_MARKER_EXTENSION)
}

/// native crash records of earlier runs, oldest first
pub fn pending_native_crash_records(
    marker_dir: &Path,
) -> Result<Vec<PathBuf>, CrashDiagnosticsUnavailable> {
    pending_with_extension(marker_dir, NATIVE_CRASH_EXTENSION)
}

fn pending_with_extension(
    marker_dir: &Path,
    wanted: &str,
) -> Result<Vec<PathBuf>, CrashDiagnosticsUnavailable> {
    let entries = match std::fs::read_dir(marker_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(CrashDiagnosticsUnavailable::io(DiagnosticsReading::PanicMarkers, &error));
        }
    };
    let mut markers: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == wanted))
        .collect();
    markers.sort();
    Ok(markers)
}

/// hands every marker and native crash record to sink as one delivery and then removes them
/// a file that cant be read is skipped and kept for the next launch
/// an abort record that only follows a panic of the same process is removed unsent
pub fn deliver_panic_markers(
    marker_dir: &Path,
    sink: &PayloadSink,
) -> Result<usize, CrashDiagnosticsUnavailable> {
    let panic_markers = pending_panic_markers(marker_dir)?;
    let native_records = pending_native_crash_records(marker_dir)?;
    let panics: Vec<MarkerStamp> =
        panic_markers.iter().filter_map(|path| MarkerStamp::of(path)).collect();
    let mut delivered = Vec::new();
    let mut superseded = Vec::new();
    let mut payloads = Vec::new();
    for path in panic_markers {
        if let Some(json) = read_marker(&path) {
            payloads.push(DiagnosticPayload { kind: PayloadKind::Diagnostic, json });
            delivered.push(path);
        }
    }
    for path in native_records {
        let Some(bytes) = read_marker(&path) else {
            continue;
        };
        let mut record = NativeCrashRecord::parse(&String::from_utf8_lossy(&bytes));
        let stamp = MarkerStamp::of(&path);
        record.at_millis = record.at_millis.or(stamp.map(|stamp| stamp.millis));
        record.pid = record.pid.or(stamp.map(|stamp| stamp.pid));
        if record.follows_a_panic(&panics) {
            superseded.push(path);
            continue;
        }
        payloads.push(DiagnosticPayload { kind: PayloadKind::Diagnostic, json: record.to_json() });
        delivered.push(path);
    }
    remove_markers(&superseded);
    if payloads.is_empty() {
        return Ok(0);
    }
    sink(payloads);
    remove_markers(&delivered);
    Ok(delivered.len())
}

fn read_marker(path: &Path) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            tracing::warn!(
                target: "riseonly",
                path = %path.display(),
                %error,
                "panic marker unreadable"
            );
            None
        }
    }
}

fn remove_markers(paths: &[PathBuf]) {
    for path in paths {
        if let Err(error) = std::fs::remove_file(path) {
            tracing::warn!(
                target: "riseonly",
                path = %path.display(),
                %error,
                "panic marker not removed"
            );
        }
    }
}

/// byte reading in whole megabytes, or -1 without an answer
pub fn megabytes_or_failed(bytes: Result<u64, CrashDiagnosticsUnavailable>) -> i64 {
    match bytes {
        Ok(bytes) => i64::try_from(bytes / BYTES_PER_MB).unwrap_or(i64::MAX),
        Err(_) => READING_FAILED,
    }
}

/// whole seconds from start to now, never below zero (clock set back)
/// or -1 when start failed
pub fn uptime_seconds_or_failed(start: Result<f64, CrashDiagnosticsUnavailable>, now: f64) -> i64 {
    match start {
        Ok(start) if start.is_finite() => (now - start).max(0.0) as i64,
        _ => READING_FAILED,
    }
}

/// memory this process holds in mb, or -1
pub fn resident_memory_mb() -> i64 {
    megabytes_or_failed(super::os::NativeCrashDiagnostics.resident_memory_bytes())
}

/// seconds since this process started (not since boot), or -1
pub fn process_uptime_seconds() -> i64 {
    uptime_seconds_or_failed(
        super::os::NativeCrashDiagnostics.process_start_seconds_since_1970(),
        RiseClock::now().time_interval_since_1970(),
    )
}

/// free space for an important download on the volume of path, or -1
pub fn available_disk_mb(path: &Path) -> i64 {
    megabytes_or_failed(available_capacity_for_important_usage(path))
}

pub fn available_capacity_for_important_usage(
    path: &Path,
) -> Result<u64, CrashDiagnosticsUnavailable> {
    super::os::NativeCrashDiagnostics.available_capacity_for_important_usage(path)
}

fn refused_reading(what: DiagnosticsReading, detail: String) -> CrashDiagnosticsUnavailable {
    CrashDiagnosticsUnavailable::SystemRefused { what, detail }
}

/// resident bytes from proc self statm, the second field counts resident pages
pub fn resident_bytes_from_statm(
    statm: &str,
    page_size: u64,
) -> Result<u64, CrashDiagnosticsUnavailable> {
    let what = DiagnosticsReading::ResidentMemory;
    let field = statm
        .split_ascii_whitespace()
        .nth(1)
        .ok_or_else(|| refused_reading(what, format!("statm has no resident field: {statm:?}")))?;
    let pages: u64 = field
        .parse()
        .map_err(|_| refused_reading(what, format!("statm resident field {field:?}")))?;
    if page_size == 0 {
        return Err(refused_reading(what, "page size 0".to_owned()));
    }
    pages
        .checked_mul(page_size)
        .ok_or_else(|| refused_reading(what, format!("{pages} pages of {page_size} bytes")))
}

/// field 22 (starttime, clock ticks after boot) of proc self stat
/// field 2 is the command name in parentheses and can hold spaces and parens
/// so count from the last close paren where field 3 begins
pub fn start_ticks_from_proc_pid_stat(stat: &str) -> Result<u64, CrashDiagnosticsUnavailable> {
    let what = DiagnosticsReading::ProcessStart;
    let (_, tail) = stat
        .rsplit_once(')')
        .ok_or_else(|| refused_reading(what, "stat has no command field".to_owned()))?;
    let field = tail
        .split_ascii_whitespace()
        .nth(22 - 3)
        .ok_or_else(|| refused_reading(what, "stat has no starttime field".to_owned()))?;
    field
        .parse()
        .map_err(|_| refused_reading(what, format!("stat starttime field {field:?}")))
}

/// the btime line of proc stat, boot time in whole seconds since 1970
pub fn boot_seconds_from_proc_stat(proc_stat: &str) -> Result<u64, CrashDiagnosticsUnavailable> {
    let what = DiagnosticsReading::ProcessStart;
    let value = proc_stat
        .lines()
        .find_map(|line| line.strip_prefix("btime"))
        .map(str::trim)
        .ok_or_else(|| refused_reading(what, "/proc/stat has no btime line".to_owned()))?;
    let seconds: u64 =
        value.parse().map_err(|_| refused_reading(what, format!("btime {value:?}")))?;
    if seconds == 0 {
        return Err(refused_reading(what, "btime 0".to_owned()));
    }
    Ok(seconds)
}

/// process start on the wall clock, boot time plus starttime ticks at ticks_per_second
pub fn process_start_from_boot(
    boot_seconds: u64,
    start_ticks: u64,
    ticks_per_second: u64,
) -> Result<f64, CrashDiagnosticsUnavailable> {
    if ticks_per_second == 0 {
        return Err(refused_reading(
            DiagnosticsReading::ProcessStart,
            "clock ticks per second 0".to_owned(),
        ));
    }
    Ok(boot_seconds as f64 + start_ticks as f64 / ticks_per_second as f64)
}

/// bytes an unprivileged process may still write, available blocks of fragment size
/// or of block size on a filesystem that leaves the fragment size at zero
pub fn available_bytes_from_statvfs(
    available_blocks: u64,
    fragment_size: u64,
    block_size: u64,
) -> Result<u64, CrashDiagnosticsUnavailable> {
    let what = DiagnosticsReading::AvailableCapacity;
    let unit = if fragment_size != 0 { fragment_size } else { block_size };
    if unit == 0 {
        return Err(refused_reading(what, "statvfs block size 0".to_owned()));
    }
    available_blocks
        .checked_mul(unit)
        .ok_or_else(|| refused_reading(what, format!("{available_blocks} blocks of {unit} bytes")))
}

pub const FILETIME_TICKS_PER_SECOND: u64 = 10_000_000;

pub const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

/// process start on the wall clock from the creation filetime given as two halves
/// a time before 1970 cant be a running process start
pub fn process_start_from_filetime(
    high: u32,
    low: u32,
) -> Result<f64, CrashDiagnosticsUnavailable> {
    let ticks = (u64::from(high) << 32) | u64::from(low);
    let since_1970 = ticks.checked_sub(FILETIME_UNIX_EPOCH).ok_or_else(|| {
        refused_reading(
            DiagnosticsReading::ProcessStart,
            format!("creation FILETIME {ticks} is before 1970"),
        )
    })?;
    Ok((since_1970 / FILETIME_TICKS_PER_SECOND) as f64
        + (since_1970 % FILETIME_TICKS_PER_SECOND) as f64 / FILETIME_TICKS_PER_SECOND as f64)
}

pub const NATIVE_CRASH_EXTENSION: &str = "crash";

pub const NATIVE_CRASH_HEADER: &str = "riseonly native crash 1";

pub const NATIVE_CRASH_FRAMES: usize = 62;

pub const PANIC_ABORT_WINDOW_MILLIS: u64 = 10_000;

pub const KIND_SIGNAL: &str = "signal";

pub const KIND_EXCEPTION: &str = "exception";

pub const KIND_ABORT: &str = "abort";

pub const KIND_HANG: &str = "hang";

pub const WER_SOURCE: &str = "wer";

pub const WER_SEEN_FILE: &str = "wer-seen.txt";

pub const WER_REPORT_MAX_AGE_MILLIS: u64 = 14 * 24 * 60 * 60 * 1000;

pub const STATUS_FAIL_FAST: u64 = 0xC000_0409;

pub const LINUX_SIGABRT: u64 = 6;

static NATIVE_CRASH_TAKEN: AtomicBool = AtomicBool::new(false);

/// true only for the first crash handler call of the process, later calls just pass the crash on
pub fn take_native_crash_slot() -> bool {
    !NATIVE_CRASH_TAKEN.swap(true, Ordering::SeqCst)
}

/// text built inside a crash handler in a fixed buffer, nothing allocates and overflow is cut off
pub struct CrashText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Default for CrashText<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> CrashText<N> {
    pub const fn new() -> Self {
        Self { bytes: [0; N], len: 0 }
    }

    pub fn clear(&mut self) -> &mut Self {
        self.len = 0;
        self
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub fn is_full(&self) -> bool {
        self.len == N
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        let take = bytes.len().min(N - self.len);
        self.bytes[self.len..self.len + take].copy_from_slice(&bytes[..take]);
        self.len += take;
        self
    }

    pub fn push_str(&mut self, text: &str) -> &mut Self {
        self.push_bytes(text.as_bytes())
    }

    pub fn push_decimal(&mut self, value: u64) -> &mut Self {
        self.push_padded_decimal(value, 1)
    }

    pub fn push_signed(&mut self, value: i64) -> &mut Self {
        if value < 0 {
            self.push_str("-");
        }
        self.push_decimal(value.unsigned_abs())
    }

    /// decimal with leading zeros up to width digits
    pub fn push_padded_decimal(&mut self, value: u64, width: usize) -> &mut Self {
        let mut digits = [b'0'; 20];
        let mut at = digits.len();
        let mut rest = value;
        loop {
            at -= 1;
            digits[at] = b'0' + (rest % 10) as u8;
            rest /= 10;
            if rest == 0 {
                break;
            }
        }
        let start = at.min(digits.len().saturating_sub(width.min(digits.len())));
        self.push_bytes(&digits[start..])
    }

    pub fn push_hex(&mut self, value: u64) -> &mut Self {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut digits = [0u8; 16];
        let mut at = digits.len();
        let mut rest = value;
        loop {
            at -= 1;
            digits[at] = DIGITS[(rest & 0xf) as usize];
            rest >>= 4;
            if rest == 0 {
                break;
            }
        }
        self.push_str("0x").push_bytes(&digits[at..])
    }

    pub fn push_utf16(&mut self, units: &[u16]) -> &mut Self {
        for decoded in char::decode_utf16(units.iter().copied()) {
            let mut encoded = [0u8; 4];
            let character = decoded.unwrap_or(char::REPLACEMENT_CHARACTER);
            self.push_str(character.encode_utf8(&mut encoded));
        }
        self
    }
}

/// file name of a native crash record, same order and form as a panic marker
pub fn push_native_crash_name<const N: usize>(text: &mut CrashText<N>, millis: u64, pid: u32) {
    text.push_padded_decimal(millis, 16)
        .push_str("-")
        .push_decimal(u64::from(pid))
        .push_str(".")
        .push_str(NATIVE_CRASH_EXTENSION);
}

/// where the program counter sits in the ucontext a linux signal handler gets
/// x86_64 is gregs REG_RIP, aarch64 is mcontext pc
pub fn signal_context_pc_offset(architecture: &str) -> Option<usize> {
    match architecture {
        "x86_64" => Some(168),
        "aarch64" => Some(440),
        _ => None,
    }
}

/// a proc self maps line worth keeping for symbols, file backed and executable or the image start
pub fn keep_map_line(line: &[u8]) -> bool {
    let mut fields = line.split(|byte| *byte == b' ').filter(|field| !field.is_empty());
    let (Some(_range), Some(perms), Some(offset)) = (fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    let path = fields.nth(2);
    if !path.is_some_and(|path| path.first() == Some(&b'/')) {
        return false;
    }
    perms.get(2) == Some(&b'x') || offset.iter().all(|byte| *byte == b'0')
}

/// millis and pid in the name of a marker or record
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MarkerStamp {
    pub millis: u64,
    pub pid: u32,
}

impl MarkerStamp {
    pub fn of(path: &Path) -> Option<Self> {
        let stem = path.file_stem()?.to_str()?;
        let (millis, pid) = stem.split_once('-')?;
        Some(Self { millis: millis.parse().ok()?, pid: pid.parse().ok()? })
    }
}

/// one stack frame of a record, the module base when the handler could name it
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NativeFrame {
    pub address: u64,
    pub base: Option<u64>,
}

/// one file backed mapping from proc self maps
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapRegion {
    pub start: u64,
    pub end: u64,
    pub offset: u64,
    pub path: String,
}

impl MapRegion {
    fn parse(line: &str) -> Option<Self> {
        let mut fields = line.split_ascii_whitespace();
        let (start, end) = fields.next()?.split_once('-')?;
        let _perms = fields.next()?;
        let offset = fields.next()?;
        let path = fields.nth(2)?;
        Some(Self {
            start: u64::from_str_radix(start, 16).ok()?,
            end: u64::from_str_radix(end, 16).ok()?,
            offset: u64::from_str_radix(offset, 16).ok()?,
            path: path.to_owned(),
        })
    }
}

/// what a native crash handler wrote, read back at the next launch
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct NativeCrashRecord {
    pub kind: String,
    pub code: Option<u64>,
    pub subcode: Option<i64>,
    pub address: Option<u64>,
    pub pc: Option<u64>,
    pub pc_base: Option<u64>,
    pub pid: Option<u32>,
    pub thread: Option<String>,
    pub at_millis: Option<u64>,
    pub architecture: Option<String>,
    pub frames: Vec<NativeFrame>,
    pub modules: Vec<(u64, String)>,
    pub maps: Vec<MapRegion>,
    pub source: Option<String>,
}

fn parse_number(text: &str) -> Option<u64> {
    match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

fn file_name_of(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

impl NativeCrashRecord {
    /// reads a record line by line, a record cut short by the dying process keeps what it has
    pub fn parse(text: &str) -> Self {
        let mut record = Self::default();
        for line in text.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "kind" => record.kind = value.to_owned(),
                "code" => record.code = parse_number(value),
                "subcode" => record.subcode = value.parse().ok(),
                "address" => record.address = parse_number(value),
                "pc" => {
                    let mut parts = value.split(' ');
                    record.pc = parts.next().and_then(parse_number);
                    record.pc_base = parts.next().and_then(parse_number);
                }
                "pid" => record.pid = value.parse().ok(),
                "thread" => record.thread = Some(value.to_owned()).filter(|name| !name.is_empty()),
                "at" => record.at_millis = value.parse().ok(),
                "arch" => record.architecture = Some(value.to_owned()),
                "frame" => {
                    let mut parts = value.split(' ');
                    if let Some(address) = parts.next().and_then(parse_number) {
                        let base = parts.next().and_then(parse_number);
                        record.frames.push(NativeFrame { address, base });
                    }
                }
                "module" => {
                    if let Some((base, path)) = value.split_once(' ')
                        && let Some(base) = parse_number(base)
                    {
                        record.modules.push((base, path.to_owned()));
                    }
                }
                "map" => record.maps.extend(MapRegion::parse(value)),
                "source" => record.source = Some(value.to_owned()).filter(|source| !source.is_empty()),
                _ => {}
            }
        }
        record
    }

    /// an abort that a rust panic of the same process caused a moment later
    /// the panic marker already tells that story
    pub fn follows_a_panic(&self, panics: &[MarkerStamp]) -> bool {
        let is_abort = self.kind == KIND_ABORT
            || (self.kind == KIND_SIGNAL && self.code == Some(LINUX_SIGABRT));
        let (Some(pid), Some(at)) = (self.pid, self.at_millis) else {
            return false;
        };
        is_abort
            && panics.iter().any(|panic| {
                panic.pid == pid
                    && at >= panic.millis
                    && at - panic.millis <= PANIC_ABORT_WINDOW_MILLIS
            })
    }

    /// module file name and offset inside it, from the handler base or from the maps
    pub fn locate(&self, address: u64, base: Option<u64>) -> Option<(String, u64)> {
        if let Some(base) = base
            && let Some((_, path)) = self.modules.iter().find(|(known, _)| *known == base)
        {
            return Some((file_name_of(path).to_owned(), address.wrapping_sub(base)));
        }
        let region =
            self.maps.iter().find(|region| region.start <= address && address < region.end)?;
        let image_start = self
            .maps
            .iter()
            .filter(|other| other.path == region.path && other.offset == 0)
            .map(|other| other.start)
            .min()
            .unwrap_or(region.start.wrapping_sub(region.offset));
        Some((file_name_of(&region.path).to_owned(), address.wrapping_sub(image_start)))
    }

    fn located(&self, address: u64, base: Option<u64>) -> String {
        match self.locate(address, base) {
            Some((name, offset)) => format!("{name} + {offset:#x}"),
            None => format!("{address:#x}"),
        }
    }

    /// frames from the faulting one on, the handler own frames before it are dropped
    pub fn crash_frames(&self) -> Vec<NativeFrame> {
        let Some(pc) = self.pc else {
            return self.frames.clone();
        };
        match self.frames.iter().position(|frame| frame.address == pc) {
            Some(index) => self.frames[index..].to_vec(),
            None => {
                let mut frames = vec![NativeFrame { address: pc, base: self.pc_base }];
                frames.extend(self.frames.iter().copied());
                frames
            }
        }
    }

    pub fn name(&self) -> String {
        let code = self.code.unwrap_or(0);
        match self.kind.as_str() {
            KIND_HANG => "main thread not responding".to_owned(),
            KIND_EXCEPTION => {
                let name = exception_name(code as u32).unwrap_or("EXCEPTION");
                format!("{name} ({code:#010x})")
            }
            KIND_ABORT => "SIGABRT (abort)".to_owned(),
            _ => {
                let name = linux_signal_name(code).unwrap_or("signal");
                format!("{name} ({code})")
            }
        }
    }

    /// one line as an apple crash report would title it
    pub fn termination_reason(&self) -> String {
        let lead = if self.kind == KIND_HANG { "Hang" } else { "Native crash" };
        let mut reason = format!("{lead}: {}", self.name());
        if let Some(detail) = self.fault_detail() {
            reason.push(' ');
            reason.push_str(&detail);
        }
        if let Some(pc) = self.pc {
            reason.push_str(&format!(" at {}", self.located(pc, self.pc_base)));
        }
        if let Some(thread) = &self.thread {
            reason.push_str(&format!(" on thread {thread}"));
        }
        if self.source.as_deref() == Some(WER_SOURCE) {
            reason.push_str(" (windows error reporting)");
        }
        reason
    }

    fn fault_detail(&self) -> Option<String> {
        let code = self.code?;
        match self.kind.as_str() {
            KIND_EXCEPTION if code == STATUS_FAIL_FAST => {
                let fail_fast = self.subcode?;
                Some(match fail_fast_name(fail_fast) {
                    Some(name) => format!("fail fast {fail_fast} {name}"),
                    None => format!("fail fast {fail_fast}"),
                })
            }
            KIND_EXCEPTION if code == 0xC000_0005 || code == 0xC000_0006 => {
                let operation = match self.subcode {
                    Some(0) => "reading",
                    Some(1) => "writing",
                    Some(8) => "executing",
                    _ => "touching",
                };
                Some(format!("{operation} {:#x}", self.address?))
            }
            KIND_SIGNAL => {
                let detail =
                    self.subcode.map_or("", |subcode| linux_signal_code_name(code, subcode));
                let address = self.address.filter(|_| code != LINUX_SIGABRT);
                match (detail.is_empty(), address) {
                    (true, None) => None,
                    (true, Some(address)) => Some(format!("address {address:#x}")),
                    (false, None) => Some(detail.to_owned()),
                    (false, Some(address)) => Some(format!("{detail} address {address:#x}")),
                }
            }
            _ => None,
        }
    }

    /// numbered frames, module file name, address and offset inside the module
    pub fn stack_trace(&self) -> String {
        self.crash_frames()
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                let module = self
                    .locate(frame.address, frame.base)
                    .map_or_else(|| "???".to_owned(), |(name, _)| name);
                format!(
                    "{index:<4}{module:<32} {:#018x} {}",
                    frame.address,
                    self.located(frame.address, frame.base)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// a diagnostic payload with one crash entry, same keys as a panic marker
    pub fn to_json(&self) -> Vec<u8> {
        let at = self.at_millis.unwrap_or(0) as f64 / 1000.0;
        let stamp = metrickit_timestamp(at);
        let mut meta = serde_json::json!({
            (TERMINATION_REASON_KEY): self.termination_reason(),
            "platformArchitecture": self.architecture.clone().unwrap_or_default(),
        });
        if let (Some(code), Some(fields)) = (self.code, meta.as_object_mut()) {
            match self.kind.as_str() {
                KIND_EXCEPTION => {
                    fields.insert("exceptionCode".to_owned(), serde_json::json!(code));
                }
                _ => {
                    fields.insert("signal".to_owned(), serde_json::json!(code));
                }
            }
        }
        let key = if self.kind == KIND_HANG { HANG_DIAGNOSTICS_KEY } else { CRASH_DIAGNOSTICS_KEY };
        let value = serde_json::json!({
            "timeStampBegin": stamp,
            "timeStampEnd": stamp,
            (key): [{
                (DIAGNOSTIC_META_DATA_KEY): meta,
                (STACK_TRACE_KEY): self.stack_trace(),
            }],
        });
        serde_json::to_vec(&value).unwrap_or_default()
    }
}

/// what a windows fail fast code means, the exception data of STATUS_STACK_BUFFER_OVERRUN
pub fn fail_fast_name(code: i64) -> Option<&'static str> {
    Some(match code {
        2 => "stack cookie check failure",
        3 => "corrupt list entry",
        5 => "invalid argument",
        7 => "fatal app exit",
        10 => "guard icall check failure",
        14 => "invalid reference count",
        21 => "invalid exception chain",
        35 => "unexpected heap exception",
        _ => return None,
    })
}

/// a crash windows error reporting kept for this app, the only trace of a fail fast exit
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WerCrash {
    pub app_path: String,
    pub pid: Option<u32>,
    pub at_millis: Option<u64>,
    pub module: String,
    pub offset: Option<u64>,
    pub code: Option<u64>,
    pub data: Option<u64>,
}

impl WerCrash {
    /// reads Report.wer text, sig names are localised so values go by position and event type
    pub fn parse(text: &str) -> Option<Self> {
        let value = |key: &str| {
            text.lines().find_map(|line| line.trim_end_matches('\r').strip_prefix(key)?.strip_prefix('=')).map(str::trim)
        };
        let sig = |index: usize| value(&format!("Sig[{index}].Value"));
        let hex = |text: Option<&str>| text.and_then(|text| u64::from_str_radix(text.trim_start_matches("0x"), 16).ok());
        let event = value("EventType")?;
        let is_bex = event.starts_with("BEX");
        if !is_bex && !event.eq_ignore_ascii_case("APPCRASH") {
            return None;
        }
        let (offset, code, data) =
            if is_bex { (hex(sig(6)), hex(sig(7)), hex(sig(8))) } else { (hex(sig(7)), hex(sig(6)), None) };
        Some(Self {
            app_path: value("AppPath").unwrap_or_default().to_owned(),
            pid: value("AppSessionGuid").and_then(|guid| u32::from_str_radix(guid.get(..8)?, 16).ok()),
            at_millis: value("EventTime")
                .and_then(|time| time.parse::<u64>().ok())
                .map(|ticks| ticks.saturating_sub(FILETIME_UNIX_EPOCH) / 10_000),
            module: sig(3).unwrap_or("???").to_owned(),
            offset,
            code,
            data,
        })
    }

    /// the crash as a native crash record, the module stands at base zero so the pc is its offset
    pub fn record_text(&self, architecture: &str) -> String {
        let mut text = format!("{NATIVE_CRASH_HEADER}\nkind {KIND_EXCEPTION}\n");
        if let Some(code) = self.code {
            text.push_str(&format!("code {code:#x}\n"));
        }
        if let Some(data) = self.data {
            text.push_str(&format!("subcode {data}\n"));
        }
        if let Some(offset) = self.offset {
            text.push_str(&format!("pc {offset:#x} 0x0\nmodule 0x0 {}\n", self.module));
        }
        if let Some(pid) = self.pid {
            text.push_str(&format!("pid {pid}\n"));
        }
        if let Some(at) = self.at_millis {
            text.push_str(&format!("at {at}\n"));
        }
        text.push_str(&format!("arch {architecture}\nsource {WER_SOURCE}\n"));
        text
    }
}

/// reports to record and the full seen list to keep, by report folder name
/// a report of another exe, an old one or a process that left its own record is only marked seen
pub fn select_wer_crashes(
    reports: &[(String, String)],
    seen: &[String],
    recorded_pids: &[u32],
    exe_path: &str,
    now_millis: u64,
) -> (Vec<(String, WerCrash)>, Vec<String>) {
    let mut chosen = Vec::new();
    let mut still_seen = Vec::new();
    for (name, text) in reports {
        still_seen.push(name.clone());
        if seen.contains(name) {
            continue;
        }
        let Some(crash) = WerCrash::parse(text) else {
            continue;
        };
        let is_ours = crash.app_path.eq_ignore_ascii_case(exe_path);
        let is_fresh = crash.at_millis.is_some_and(|at| now_millis.saturating_sub(at) <= WER_REPORT_MAX_AGE_MILLIS);
        let is_recorded = crash.pid.is_some_and(|pid| recorded_pids.contains(&pid));
        if is_ours && is_fresh && !is_recorded {
            chosen.push((name.clone(), crash));
        }
    }
    (chosen, still_seen)
}

/// name of a windows exception code a process can die of
pub fn exception_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0xC000_0005 => "EXCEPTION_ACCESS_VIOLATION",
        0xC000_0006 => "EXCEPTION_IN_PAGE_ERROR",
        0xC000_001D => "EXCEPTION_ILLEGAL_INSTRUCTION",
        0xC000_0025 => "EXCEPTION_NONCONTINUABLE_EXCEPTION",
        0xC000_0026 => "EXCEPTION_INVALID_DISPOSITION",
        0xC000_008C => "EXCEPTION_ARRAY_BOUNDS_EXCEEDED",
        0xC000_008D => "EXCEPTION_FLT_DENORMAL_OPERAND",
        0xC000_008E => "EXCEPTION_FLT_DIVIDE_BY_ZERO",
        0xC000_008F => "EXCEPTION_FLT_INEXACT_RESULT",
        0xC000_0090 => "EXCEPTION_FLT_INVALID_OPERATION",
        0xC000_0091 => "EXCEPTION_FLT_OVERFLOW",
        0xC000_0092 => "EXCEPTION_FLT_STACK_CHECK",
        0xC000_0093 => "EXCEPTION_FLT_UNDERFLOW",
        0xC000_0094 => "EXCEPTION_INT_DIVIDE_BY_ZERO",
        0xC000_0095 => "EXCEPTION_INT_OVERFLOW",
        0xC000_0096 => "EXCEPTION_PRIV_INSTRUCTION",
        0xC000_00FD => "EXCEPTION_STACK_OVERFLOW",
        0xC000_0374 => "STATUS_HEAP_CORRUPTION",
        0xC000_0409 => "STATUS_STACK_BUFFER_OVERRUN",
        0xC000_0420 => "STATUS_ASSERTION_FAILURE",
        0x8000_0002 => "EXCEPTION_DATATYPE_MISALIGNMENT",
        0x8000_0003 => "EXCEPTION_BREAKPOINT",
        0x8000_0004 => "EXCEPTION_SINGLE_STEP",
        0xE06D_7363 => "MSVC_CPP_EXCEPTION",
        0x4000_0015 => "STATUS_FATAL_APP_EXIT",
        _ => return None,
    })
}

/// name of a linux signal the crash handler listens to
pub fn linux_signal_name(signal: u64) -> Option<&'static str> {
    Some(match signal {
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        11 => "SIGSEGV",
        _ => return None,
    })
}

/// si_code of a linux crash signal in words
pub fn linux_signal_code_name(signal: u64, code: i64) -> &'static str {
    match (signal, code) {
        (_, 0) => "SI_USER",
        (_, -6) => "SI_TKILL",
        (_, code) if code < 0 => "SI_QUEUE",
        (11, 1) => "SEGV_MAPERR",
        (11, 2) => "SEGV_ACCERR",
        (7, 1) => "BUS_ADRALN",
        (7, 2) => "BUS_ADRERR",
        (7, 3) => "BUS_OBJERR",
        (8, 1) => "FPE_INTDIV",
        (8, 2) => "FPE_INTOVF",
        (4, 1) => "ILL_ILLOPC",
        (4, 2) => "ILL_ILLOPN",
        (4, 5) => "ILL_PRVOPC",
        (5, 1) => "TRAP_BRKPT",
        _ => "",
    }
}

static MAIN_THREAD_BEAT_MS: AtomicU64 = AtomicU64::new(0);

static BEAT_CLOCK: OnceLock<Instant> = OnceLock::new();

fn beat_clock_ms() -> u64 {
    u64::try_from(BEAT_CLOCK.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX).saturating_add(1)
}

/// silence of the main thread after which it counts as hung, from the canon
pub fn hang_report_after() -> Duration {
    Duration::from_millis(u64::try_from(RiseTimingCanon::MAIN_THREAD_HANG_REPORT_MS).unwrap_or(0))
}

/// how often the app beats on the main thread, a fifth of the hang threshold
pub fn main_thread_beat_interval() -> Duration {
    hang_report_after() / 5
}

/// true where the hang watch counts app beats on the main thread, macos reports hangs itself
pub const fn main_thread_beats_needed(host: HostOs) -> bool {
    matches!(host, HostOs::Linux | HostOs::Windows)
}

/// called on the main thread by the app timer
pub fn main_thread_beat() {
    MAIN_THREAD_BEAT_MS.store(beat_clock_ms(), Ordering::Relaxed);
}

/// how long the main thread went without a beat, none before the first one
pub fn main_thread_silence() -> Option<Duration> {
    let last = MAIN_THREAD_BEAT_MS.load(Ordering::Relaxed);
    (last != 0).then(|| Duration::from_millis(beat_clock_ms().saturating_sub(last)))
}

/// true when silence of the main thread is long enough to report a hang
pub fn is_hang(silence: Option<Duration>) -> bool {
    silence.is_some_and(|silence| silence >= hang_report_after())
}

/// a watch sleep that overran this much means the machine slept, silence before it is not a hang
pub fn is_wake_gap(gap: Duration) -> bool {
    gap > main_thread_beat_interval() * 3
}

/// hang only once the watch itself was awake for the whole threshold
/// and no system modal window holds the main thread loop
pub fn is_hang_while_awake(silence: Option<Duration>, awake_for: Duration) -> bool {
    !native_modal_open() && is_hang(silence) && awake_for >= hang_report_after()
}

static NATIVE_MODALS_OPEN: AtomicUsize = AtomicUsize::new(0);

/// a system open, save or folder panel is up, its own loop runs the main thread
/// and the app timer that beats waits for it, any exit of the guard ends it
#[must_use = "the modal counts as open only while the guard lives"]
pub struct NativeModalGuard(());

impl NativeModalGuard {
    pub fn enter() -> Self {
        NATIVE_MODALS_OPEN.fetch_add(1, Ordering::SeqCst);
        Self(())
    }
}

impl Drop for NativeModalGuard {
    fn drop(&mut self) {
        main_thread_beat();
        NATIVE_MODALS_OPEN.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn native_modal_open() -> bool {
    NATIVE_MODALS_OPEN.load(Ordering::SeqCst) > 0
}
