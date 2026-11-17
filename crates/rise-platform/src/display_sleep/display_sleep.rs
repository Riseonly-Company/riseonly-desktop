use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// why the display may still sleep, the call goes on either way and only the screen may dim
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum DisplaySleepUnavailable {
    #[error("holding the display awake is not written for {0:?}")]
    BranchNotWritten(HostOs),
    #[error("no system service holds the display awake: {0}")]
    NoService(String),
    #[error("the system refused to hold the display awake: {0}")]
    SystemRefused(String),
}

/// default answer of an os layer that doesnt override inhibit
pub const fn unavailable_by_default(host: HostOs) -> DisplaySleepUnavailable {
    DisplaySleepUnavailable::BranchNotWritten(host)
}

/// one system request that keeps the display awake, dropping it releases the request
#[must_use = "dropping the inhibition lets the display sleep again"]
pub struct SystemInhibition {
    _release: Box<dyn Send>,
}

impl SystemInhibition {
    /// release gives the request back to the system in its own drop
    /// public so an os layer with nothing to release leaves no dead constructor on its target
    pub fn new(release: impl Send + 'static) -> Self {
        Self {
            _release: Box::new(release),
        }
    }
}

impl std::fmt::Debug for SystemInhibition {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SystemInhibition")
    }
}

pub trait DisplaySleepInhibitor: Send + Sync {
    fn inhibit(&self, reason: &str) -> Result<SystemInhibition, DisplaySleepUnavailable> {
        let _ = reason;
        Err(unavailable_by_default(HostOs::current()))
    }
}

#[derive(Default)]
struct HoldState {
    holders: usize,
    inhibition: Option<SystemInhibition>,
    failure: Option<DisplaySleepUnavailable>,
}

struct Shared {
    inhibitor: Box<dyn DisplaySleepInhibitor>,
    state: Mutex<HoldState>,
}

/// counts the holders of one system request
pub struct DisplaySleep {
    shared: Arc<Shared>,
}

impl DisplaySleep {
    pub fn new(inhibitor: impl DisplaySleepInhibitor + 'static) -> Self {
        Self {
            shared: Arc::new(Shared {
                inhibitor: Box::new(inhibitor),
                state: Mutex::new(HoldState::default()),
            }),
        }
    }

    /// keeps the display awake until the guard drops
    /// only going from no holder to one asks the system, reason is logged once per hold
    pub fn acquire(&self, reason: &str) -> DisplaySleepGuard {
        let mut state = self.shared.state.lock();
        state.holders += 1;
        if state.holders == 1 {
            match self.shared.inhibitor.inhibit(reason) {
                Ok(inhibition) => {
                    state.inhibition = Some(inhibition);
                    state.failure = None;
                }
                Err(failure) => {
                    tracing::warn!(target: "riseonly", %failure, "display may sleep during the call");
                    state.failure = Some(failure);
                }
            }
        }
        let outcome = match &state.failure {
            Some(failure) => Err(failure.clone()),
            None => Ok(()),
        };
        DisplaySleepGuard {
            shared: Arc::clone(&self.shared),
            outcome,
        }
    }

    /// how many guards are alive
    pub fn holders(&self) -> usize {
        self.shared.state.lock().holders
    }

    pub fn support(&self) -> PlatformSupport {
        if self.shared.state.lock().inhibition.is_some() {
            PlatformSupport::Performed
        } else {
            PlatformSupport::Unsupported
        }
    }
}

/// one holder, dropping the last one releases the system request
#[must_use = "dropping the guard lets the display sleep again"]
pub struct DisplaySleepGuard {
    shared: Arc<Shared>,
    outcome: Result<(), DisplaySleepUnavailable>,
}

impl DisplaySleepGuard {
    /// ok when the system holds the display awake for this guard, otherwise the reason
    pub fn outcome(&self) -> &Result<(), DisplaySleepUnavailable> {
        &self.outcome
    }

    pub fn support(&self) -> PlatformSupport {
        if self.outcome.is_ok() {
            PlatformSupport::Performed
        } else {
            PlatformSupport::Unsupported
        }
    }
}

impl Drop for DisplaySleepGuard {
    fn drop(&mut self) {
        let released = {
            let mut state = self.shared.state.lock();
            state.holders = state.holders.saturating_sub(1);
            if state.holders == 0 {
                state.failure = None;
                state.inhibition.take()
            } else {
                None
            }
        };
        drop(released);
    }
}

impl std::fmt::Debug for DisplaySleepGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DisplaySleepGuard")
            .field("outcome", &self.outcome)
            .finish()
    }
}

pub const CALL_IN_PROGRESS_REASON: &str = "Riseonly call in progress";

/// process wide counter on the os layer of the build target
pub fn display_sleep() -> &'static DisplaySleep {
    static SHARED: OnceLock<DisplaySleep> = OnceLock::new();
    SHARED.get_or_init(|| DisplaySleep::new(super::os::NativeDisplaySleep))
}

/// idle timer disabled for the life of the guard
/// the call store holds one while it has a session and drops it when the last session is gone
pub fn acquire(reason: &str) -> DisplaySleepGuard {
    display_sleep().acquire(reason)
}
