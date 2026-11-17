use std::collections::HashMap;
use std::time::Duration;

use thiserror::Error;

use crate::host_os::HostOs;

pub const THERMAL_STATE_DID_CHANGE: &str = "NSProcessInfoThermalStateDidChangeNotification";

/// thermal state ordered from calm to worst with the ios raw values
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum ThermalState {
    #[default]
    Nominal,
    Fair,
    Serious,
    Critical,
}

impl ThermalState {
    pub const ALL: [Self; 4] = [Self::Nominal, Self::Fair, Self::Serious, Self::Critical];

    /// raw integer value, frame rate and hold menu hosts compare this
    pub const fn raw_value(self) -> i64 {
        match self {
            Self::Nominal => 0,
            Self::Fair => 1,
            Self::Serious => 2,
            Self::Critical => 3,
        }
    }

    pub const fn from_raw_value(raw: i64) -> Option<Self> {
        match raw {
            0 => Some(Self::Nominal),
            1 => Some(Self::Fair),
            2 => Some(Self::Serious),
            3 => Some(Self::Critical),
            _ => None,
        }
    }

    /// the spelling the bug report and performance monitor send to the server
    pub const fn name(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::Fair => "fair",
            Self::Serious => "serious",
            Self::Critical => "critical",
        }
    }
}

/// which reading an os layer couldnt give
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ProcessReading {
    PhysicalMemory,
    SystemUptime,
    ThermalState,
    ThermalStateWatch,
    DebuggerAttached,
    ProcessCpuSeconds,
}

impl std::fmt::Display for ProcessReading {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::PhysicalMemory => "physical memory",
            Self::SystemUptime => "system uptime",
            Self::ThermalState => "thermal state",
            Self::ThermalStateWatch => "thermal state changes",
            Self::DebuggerAttached => "debugger attachment",
            Self::ProcessCpuSeconds => "process CPU time",
        };
        formatter.write_str(text)
    }
}

/// why a reading is missing, every caller still has a working answer (see each public reader)
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum ProcessInfoUnavailable {
    #[error("{what} is not written for {host:?}")]
    BranchNotWritten { what: ProcessReading, host: HostOs },
    #[error("{0:?} publishes no thermal state to applications")]
    NoThermalSignal(HostOs),
    #[error("the system refused to report {what}: {detail}")]
    SystemRefused { what: ProcessReading, detail: String },
}

/// default answer of an os layer for a reading it doesnt override
pub const fn unavailable_by_default(what: ProcessReading, host: HostOs) -> ProcessInfoUnavailable {
    match (what, host) {
        (ProcessReading::ThermalState | ProcessReading::ThermalStateWatch, HostOs::MacOs) => {
            ProcessInfoUnavailable::BranchNotWritten { what, host }
        }
        (ProcessReading::ThermalState | ProcessReading::ThermalStateWatch, _) => {
            ProcessInfoUnavailable::NoThermalSignal(host)
        }
        _ => ProcessInfoUnavailable::BranchNotWritten { what, host },
    }
}

/// where an os layer delivers each new thermal state, called on a system thread and never main
pub type ThermalSink = Box<dyn Fn(ThermalState) + Send + Sync>;

pub trait ProcessInfoSource {
    fn physical_memory(&self) -> Result<u64, ProcessInfoUnavailable> {
        Err(unavailable_by_default(ProcessReading::PhysicalMemory, HostOs::current()))
    }

    /// time since the machine booted
    fn system_uptime(&self) -> Result<Duration, ProcessInfoUnavailable> {
        Err(unavailable_by_default(ProcessReading::SystemUptime, HostOs::current()))
    }

    /// thermal state right now
    fn thermal_state(&self) -> Result<ThermalState, ProcessInfoUnavailable> {
        Err(unavailable_by_default(ProcessReading::ThermalState, HostOs::current()))
    }

    fn watch_thermal_state(
        &self,
        sink: ThermalSink,
    ) -> Result<ThermalStateWatch, ProcessInfoUnavailable> {
        drop(sink);
        Err(unavailable_by_default(ProcessReading::ThermalStateWatch, HostOs::current()))
    }

    fn is_debugger_attached(&self) -> Result<bool, ProcessInfoUnavailable> {
        Err(unavailable_by_default(ProcessReading::DebuggerAttached, HostOs::current()))
    }

    fn process_cpu_seconds(&self) -> Result<f64, ProcessInfoUnavailable> {
        Err(unavailable_by_default(ProcessReading::ProcessCpuSeconds, HostOs::current()))
    }
}

/// keeps the system observer alive, dropping it stops the thermal readings
#[must_use = "dropping the watch stops thermal state changes"]
pub struct ThermalStateWatch {
    _observer: Box<dyn Send>,
}

impl ThermalStateWatch {
    /// observer unregisters itself in its own drop
    /// public so an os layer with no observer to build leaves no dead constructor on its target
    pub fn new(observer: impl Send + 'static) -> Self {
        Self {
            _observer: Box::new(observer),
        }
    }
}

impl std::fmt::Debug for ThermalStateWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ThermalStateWatch")
    }
}

/// a thermal state with the reason when its not the system own answer
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ThermalReading {
    pub state: ThermalState,
    pub unavailable: Option<ProcessInfoUnavailable>,
}

impl ThermalReading {
    /// without a system answer the reading is nominal like a cool device
    /// so nothing gets throttled against a signal that doesnt exist
    pub fn from_result(result: Result<ThermalState, ProcessInfoUnavailable>) -> Self {
        match result {
            Ok(state) => Self {
                state,
                unavailable: None,
            },
            Err(reason) => Self {
                state: ThermalState::Nominal,
                unavailable: Some(reason),
            },
        }
    }
}

/// logical cores this process may run on (affinity and cgroup quotas included), never below one
pub fn active_processor_count() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
}

pub fn physical_memory() -> Result<u64, ProcessInfoUnavailable> {
    super::os::NativeProcessInfo.physical_memory()
}

/// physical memory in mb, the unit the bug report sends
pub fn physical_memory_mb(bytes: u64) -> u64 {
    bytes / 1_048_576
}

/// time since boot, for diagnostics only
/// timing code uses the monotonic clock
pub fn system_uptime() -> Result<Duration, ProcessInfoUnavailable> {
    super::os::NativeProcessInfo.system_uptime()
}

/// thermal state with the reason when the system gave no answer
pub fn thermal_reading() -> ThermalReading {
    ThermalReading::from_result(super::os::NativeProcessInfo.thermal_state())
}

/// thermal state as callers read it, nominal when the system has no answer
pub fn thermal_state() -> ThermalState {
    thermal_reading().state
}

pub fn watch_thermal_state(
    on_change: impl Fn(ThermalState) + Send + Sync + 'static,
) -> Result<ThermalStateWatch, ProcessInfoUnavailable> {
    super::os::NativeProcessInfo.watch_thermal_state(Box::new(on_change))
}

/// debugger_attached trace metadata, 0 or 1
/// false when the system gives no answer
pub fn is_debugger_attached() -> bool {
    super::os::NativeProcessInfo
        .is_debugger_attached()
        .unwrap_or(false)
}

pub fn process_cpu_seconds() -> Option<f64> {
    super::os::NativeProcessInfo.process_cpu_seconds().ok()
}

/// the process environment, a value that isnt valid unicode is read lossily
pub fn environment() -> HashMap<String, String> {
    environment_from(std::env::vars_os())
}

/// launch arguments, executable path first
pub fn arguments() -> Vec<String> {
    std::env::args_os()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

/// builds the environment from raw pairs, a later duplicate key wins like the c runtime reads it
pub fn environment_from<K, V>(pairs: impl IntoIterator<Item = (K, V)>) -> HashMap<String, String>
where
    K: AsRef<std::ffi::OsStr>,
    V: AsRef<std::ffi::OsStr>,
{
    pairs
        .into_iter()
        .map(|(key, value)| {
            (
                key.as_ref().to_string_lossy().into_owned(),
                value.as_ref().to_string_lossy().into_owned(),
            )
        })
        .collect()
}

/// uptime from a float seconds count the system reports
pub fn uptime_from_seconds(seconds: f64) -> Result<Duration, ProcessInfoUnavailable> {
    Duration::try_from_secs_f64(seconds).map_err(|error| ProcessInfoUnavailable::SystemRefused {
        what: ProcessReading::SystemUptime,
        detail: format!("{seconds} seconds: {error}"),
    })
}

/// process cpu seconds from the two getrusage timeval values
pub fn cpu_seconds_from_timevals(
    user_seconds: i64,
    user_microseconds: i64,
    system_seconds: i64,
    system_microseconds: i64,
) -> f64 {
    user_seconds as f64
        + system_seconds as f64
        + (user_microseconds + system_microseconds) as f64 / 1_000_000.0
}

/// process cpu seconds from kernel and user filetime values, which count 100 ns ticks of cpu time
pub fn cpu_seconds_from_filetimes(
    kernel_high: u32,
    kernel_low: u32,
    user_high: u32,
    user_low: u32,
) -> f64 {
    let kernel = (u64::from(kernel_high) << 32) | u64::from(kernel_low);
    let user = (u64::from(user_high) << 32) | u64::from(user_low);
    kernel.saturating_add(user) as f64 / 10_000_000.0
}

/// physical memory from the text of linux /proc/meminfo, the MemTotal line in kibibytes
pub fn physical_memory_from_meminfo(meminfo: &str) -> Result<u64, ProcessInfoUnavailable> {
    let refused = |detail: String| ProcessInfoUnavailable::SystemRefused {
        what: ProcessReading::PhysicalMemory,
        detail,
    };
    let line = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .ok_or_else(|| refused("/proc/meminfo has no MemTotal line".to_owned()))?;
    let mut fields = line.split_whitespace();
    let kibibytes = fields
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| refused(format!("MemTotal is not a number: {line:?}")))?;
    match fields.next() {
        Some("kB") | None => {}
        Some(unit) => return Err(refused(format!("MemTotal has unknown unit {unit:?}"))),
    }
    match kibibytes.checked_mul(1024) {
        Some(0) => Err(refused("MemTotal is 0".to_owned())),
        Some(bytes) => Ok(bytes),
        None => Err(refused(format!("MemTotal {kibibytes} kB overflows u64 bytes"))),
    }
}

/// uptime from the text of linux /proc/uptime, its first field is seconds since boot
pub fn system_uptime_from_proc_uptime(uptime: &str) -> Result<Duration, ProcessInfoUnavailable> {
    let first = uptime.split_whitespace().next().unwrap_or("");
    let seconds = first
        .parse::<f64>()
        .map_err(|_| ProcessInfoUnavailable::SystemRefused {
            what: ProcessReading::SystemUptime,
            detail: format!("/proc/uptime first field is not a number: {first:?}"),
        })?;
    uptime_from_seconds(seconds)
}

/// true when a debugger traces the process, from linux /proc/self/status
/// the tracer pid line is 0 when nothing traces it
pub fn debugger_attached_from_proc_status(status: &str) -> Result<bool, ProcessInfoUnavailable> {
    let refused = |detail: String| ProcessInfoUnavailable::SystemRefused {
        what: ProcessReading::DebuggerAttached,
        detail,
    };
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("TracerPid:"))
        .ok_or_else(|| refused("/proc/self/status has no TracerPid line".to_owned()))?;
    let field = line.split_whitespace().next().unwrap_or("");
    let tracer = field
        .parse::<u32>()
        .map_err(|_| refused(format!("TracerPid is not a pid: {field:?}")))?;
    Ok(tracer != 0)
}
