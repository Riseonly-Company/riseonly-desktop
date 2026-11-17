use std::time::Duration;

use super::memory_pressure::MemoryPressureLevel;

pub const PSI_MEMORY_PATH: &str = "/proc/pressure/memory";

pub const PSI_WINDOW_US: u64 = 2_000_000;

pub const PSI_CALM: Duration = Duration::from_secs(10);

pub const PSI_STOP_TICK: Duration = Duration::from_millis(250);

/// some means at least one task stalled on memory, full means every non idle task stalled
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PsiKind {
    Some,
    Full,
}

impl PsiKind {
    pub const fn word(self) -> &'static str {
        match self {
            Self::Some => "some",
            Self::Full => "full",
        }
    }
}

/// one trigger, kind stalls of at least stall_us within window_us report the level
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PsiTrigger {
    pub level: MemoryPressureLevel,
    pub kind: PsiKind,
    pub stall_us: u64,
    pub window_us: u64,
}

impl PsiTrigger {
    pub const WARNING: Self = Self {
        level: MemoryPressureLevel::Warning,
        kind: PsiKind::Some,
        stall_us: 300_000,
        window_us: PSI_WINDOW_US,
    };
    pub const CRITICAL: Self = Self {
        level: MemoryPressureLevel::Critical,
        kind: PsiKind::Full,
        stall_us: 200_000,
        window_us: PSI_WINDOW_US,
    };
    pub const ALL: [Self; 2] = [Self::WARNING, Self::CRITICAL];

    /// text written to the descriptor, kind then stall us then window us
    /// the layer adds the trailing nul
    pub fn command(&self) -> String {
        format!("{} {} {}", self.kind.word(), self.stall_us, self.window_us)
    }
}

/// percent of time tasks stalled on memory over the last 10 seconds
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct PsiReading {
    pub some_avg10: f64,
    pub full_avg10: f64,
}

/// reads the some and full lines of the memory pressure file
/// missing full reads as 0, no parsable some means no reading
pub fn parse_psi(text: &str) -> Option<PsiReading> {
    let mut some = None;
    let mut full = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let slot = match fields.next() {
            Some("some") => &mut some,
            Some("full") => &mut full,
            _ => continue,
        };
        *slot = fields
            .filter_map(|field| field.strip_prefix("avg10="))
            .find_map(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0);
    }
    Some(PsiReading {
        some_avg10: some?,
        full_avg10: full.unwrap_or(0.0),
    })
}

pub const PSI_CRITICAL_AVG10: f64 = 5.0;
pub const PSI_WARNING_AVG10: f64 = 10.0;

/// level of a reread, the bars sit below the trigger thresholds
/// so a fired level is held until the average clearly falls
pub fn level_from_reading(reading: PsiReading) -> MemoryPressureLevel {
    if reading.full_avg10 >= PSI_CRITICAL_AVG10 {
        MemoryPressureLevel::Critical
    } else if reading.some_avg10 >= PSI_WARNING_AVG10 {
        MemoryPressureLevel::Warning
    } else {
        MemoryPressureLevel::Normal
    }
}

/// current level from trigger events and rereads, each method answers the new level when it changed
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PsiLevelTracker {
    level: MemoryPressureLevel,
    last_activity: Duration,
}

impl PsiLevelTracker {
    pub fn level(&self) -> MemoryPressureLevel {
        self.level
    }

    /// a trigger fired, the level only rises and any event restarts the calm period
    pub fn on_event(&mut self, level: MemoryPressureLevel, now: Duration) -> Option<MemoryPressureLevel> {
        self.last_activity = now;
        if level > self.level {
            self.level = level;
            Some(level)
        } else {
            None
        }
    }

    /// true when above Normal with no event for the calm time, so reread the averages
    pub fn recheck_due(&self, now: Duration) -> bool {
        self.level != MemoryPressureLevel::Normal && now.saturating_sub(self.last_activity) >= PSI_CALM
    }

    /// level of a reread (or the first read at start) replaces the current one
    pub fn on_reading(&mut self, level: MemoryPressureLevel, now: Duration) -> Option<MemoryPressureLevel> {
        self.last_activity = now;
        if level == self.level {
            None
        } else {
            self.level = level;
            Some(level)
        }
    }
}

/// which step of arming psi failed
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PsiStep {
    Open,
    Trigger,
    Read,
}

impl PsiStep {
    const fn verb(self) -> &'static str {
        match self {
            Self::Open => "opening",
            Self::Trigger => "arming a trigger on",
            Self::Read => "reading",
        }
    }
}

const EPERM: i32 = 1;
const ENOENT: i32 = 2;
const EACCES: i32 = 13;
const EBUSY: i32 = 16;
const EINVAL: i32 = 22;
const EOPNOTSUPP: i32 = 95;

/// reason SourceRefused carries when psi cant be armed, detail is the io error text
pub fn psi_refusal(step: PsiStep, errno: Option<i32>, detail: &str) -> String {
    let cause = match errno {
        Some(ENOENT) => "the file is missing: the kernel is built without CONFIG_PSI, booted with psi=0, \
                         or a container hides /proc/pressure",
        Some(EOPNOTSUPP) => "PSI is turned off on this kernel (psi=0)",
        Some(EPERM | EACCES) => "this kernel or sandbox lets only CAP_SYS_RESOURCE arm PSI triggers \
                                 (kernels before 6.5 for unprivileged processes)",
        Some(EINVAL) if step == PsiStep::Trigger => "the kernel rejected the trigger text",
        Some(EBUSY) if step == PsiStep::Trigger => "the descriptor already holds a trigger",
        _ => detail,
    };
    format!("{} {PSI_MEMORY_PATH}: {cause}", step.verb())
}
