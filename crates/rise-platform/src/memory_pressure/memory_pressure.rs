use parking_lot::Mutex;
use thiserror::Error;

use crate::host_os::HostOs;

pub const DID_RECEIVE_MEMORY_WARNING: &str = "UIApplicationDidReceiveMemoryWarningNotification";

/// one memory pressure reading, ordered from calm to worst
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum MemoryPressureLevel {
    #[default]
    Normal,
    Warning,
    Critical,
}

/// why no memory warning will ever arrive
/// the app still works, caches keep their limits and only the early purge is missing
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum MemoryPressureUnavailable {
    #[error("memory pressure is not written for {0:?}")]
    BranchNotWritten(HostOs),
    #[error("the system refused to create a memory pressure source: {0}")]
    SourceRefused(String),
}

/// default answer of an os layer that doesnt override start
pub const fn unavailable_by_default(host: HostOs) -> MemoryPressureUnavailable {
    MemoryPressureUnavailable::BranchNotWritten(host)
}

/// where an os layer delivers each raw reading, called on a system thread and never the main one
pub type LevelSink = Box<dyn Fn(MemoryPressureLevel) + Send + Sync>;

pub trait MemoryPressureSource {
    fn start(&self, sink: LevelSink) -> Result<MemoryPressureWatch, MemoryPressureUnavailable> {
        drop(sink);
        Err(unavailable_by_default(HostOs::current()))
    }
}

/// keeps the system source alive, dropping it stops the readings
#[must_use = "dropping the watch stops memory warnings"]
pub struct MemoryPressureWatch {
    _source: Box<dyn Send>,
}

impl MemoryPressureWatch {
    /// source stops the system source in its own drop
    pub(crate) fn new(source: impl Send + 'static) -> Self {
        Self {
            _source: Box::new(source),
        }
    }
}

impl std::fmt::Debug for MemoryPressureWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MemoryPressureWatch")
    }
}

/// decides which readings are a memory warning, a rise into Warning or a further rise into Critical
/// repeats and falls post nothing and Normal re-arms the gate
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MemoryWarningGate {
    last: MemoryPressureLevel,
}

impl MemoryWarningGate {
    pub fn should_post(&mut self, level: MemoryPressureLevel) -> bool {
        let rose = level > self.last;
        self.last = level;
        rose
    }

    pub fn last(&self) -> MemoryPressureLevel {
        self.last
    }
}

pub const DISPATCH_MEMORYPRESSURE_NORMAL: u64 = 0x01;
pub const DISPATCH_MEMORYPRESSURE_WARN: u64 = 0x02;
pub const DISPATCH_MEMORYPRESSURE_CRITICAL: u64 = 0x04;

/// reads the dispatch data of a memory pressure source
/// several bits take the worst, no known bit is not a reading
pub fn level_from_dispatch_data(data: u64) -> Option<MemoryPressureLevel> {
    if data & DISPATCH_MEMORYPRESSURE_CRITICAL != 0 {
        Some(MemoryPressureLevel::Critical)
    } else if data & DISPATCH_MEMORYPRESSURE_WARN != 0 {
        Some(MemoryPressureLevel::Warning)
    } else if data & DISPATCH_MEMORYPRESSURE_NORMAL != 0 {
        Some(MemoryPressureLevel::Normal)
    } else {
        None
    }
}

/// readings filtered by the gate, each warning empties every cache
/// then calls on_warning on the system thread that read it
pub fn warning_sink(on_warning: impl Fn(MemoryPressureLevel) + Send + Sync + 'static) -> LevelSink {
    let gate = Mutex::new(MemoryWarningGate::default());
    Box::new(move |level| {
        if !gate.lock().should_post(level) {
            return;
        }
        tracing::info!(target: "riseonly", ?level, "memory warning");
        rise_core::foundation::ns_cache::ns_cache_did_receive_memory_warning();
        on_warning(level);
    })
}

/// starts watching system memory pressure for as long as the returned watch lives
/// on_warning runs on a system thread, hop to main yourself
/// on err it logs once, no early purges
pub fn watch(
    on_warning: impl Fn(MemoryPressureLevel) + Send + Sync + 'static,
) -> Result<MemoryPressureWatch, MemoryPressureUnavailable> {
    super::os::NativeMemoryPressure.start(warning_sink(on_warning))
}
