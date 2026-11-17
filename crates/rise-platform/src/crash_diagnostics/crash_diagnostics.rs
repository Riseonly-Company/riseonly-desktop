use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use rise_core::foundation::foundation_date::RiseClock;
use thiserror::Error;

use crate::host_os::HostOs;

pub const DIAGNOSTICS_DIRECTORY: &str = "Diagnostics";

pub const PANIC_MARKER_DIRECTORY: &str = "PanicMarkers";

pub const CRASH_DIAGNOSTICS_KEY: &str = "crashDiagnostics";

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
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == PANIC_MARKER_EXTENSION)
        })
        .collect();
    markers.sort();
    Ok(markers)
}

/// hands every marker to sink as one delivery and then removes them
/// a marker that cant be read is skipped and kept for the next launch
pub fn deliver_panic_markers(
    marker_dir: &Path,
    sink: &PayloadSink,
) -> Result<usize, CrashDiagnosticsUnavailable> {
    let mut delivered = Vec::new();
    let mut payloads = Vec::new();
    for path in pending_panic_markers(marker_dir)? {
        match std::fs::read(&path) {
            Ok(json) => {
                payloads.push(DiagnosticPayload {
                    kind: PayloadKind::Diagnostic,
                    json,
                });
                delivered.push(path);
            }
            Err(error) => {
                tracing::warn!(
                    target: "riseonly",
                    path = %path.display(),
                    %error,
                    "panic marker unreadable"
                );
            }
        }
    }
    if payloads.is_empty() {
        return Ok(0);
    }
    sink(payloads);
    for path in &delivered {
        if let Err(error) = std::fs::remove_file(path) {
            tracing::warn!(
                target: "riseonly",
                path = %path.display(),
                %error,
                "panic marker not removed"
            );
        }
    }
    Ok(delivered.len())
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
