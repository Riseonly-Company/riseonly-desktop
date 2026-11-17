use thiserror::Error;

use crate::host_os::HostOs;

/// one raw system event, comes on a system thread and never assume its the main one
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SystemActivityEvent {
    WillSleep,
    DidWake,
    NetworkPath { satisfied: bool },
}

/// which half of the seam an answer is about
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SystemActivityKind {
    SleepAndWake,
    NetworkPath,
}

/// why one half of the seam will never report
/// app still works, the socket heartbeat and the reconnect ladder cover for it
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum SystemActivityUnavailable {
    #[error("{what:?} is not written for {host:?}")]
    BranchNotWritten {
        what: SystemActivityKind,
        host: HostOs,
    },
    #[error("the system refused to observe {what:?}: {detail}")]
    SystemRefused {
        what: SystemActivityKind,
        detail: String,
    },
}

/// default answer for an os layer that doesnt override the trait method
pub const fn unavailable_by_default(
    what: SystemActivityKind,
    host: HostOs,
) -> SystemActivityUnavailable {
    SystemActivityUnavailable::BranchNotWritten { what, host }
}

/// where an os layer delivers each raw event
pub type EventSink = std::sync::Arc<dyn Fn(SystemActivityEvent) + Send + Sync>;

pub trait SystemActivitySource {
    fn watch_sleep(&self, sink: EventSink) -> Result<ActivityWatch, SystemActivityUnavailable> {
        drop(sink);
        Err(unavailable_by_default(
            SystemActivityKind::SleepAndWake,
            HostOs::current(),
        ))
    }

    fn watch_network_path(
        &self,
        sink: EventSink,
    ) -> Result<ActivityWatch, SystemActivityUnavailable> {
        drop(sink);
        Err(unavailable_by_default(
            SystemActivityKind::NetworkPath,
            HostOs::current(),
        ))
    }
}

/// keeps one system observer alive, dropping it stops the events
#[must_use = "dropping the watch stops system activity events"]
pub struct ActivityWatch {
    _observer: Box<dyn Send>,
}

impl ActivityWatch {
    /// the observer unregisters itself in its own Drop
    /// public so an os layer with nothing to build leaves no dead constructor
    pub fn new(observer: impl Send + 'static) -> Self {
        Self {
            _observer: Box::new(observer),
        }
    }
}

impl std::fmt::Debug for ActivityWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ActivityWatch")
    }
}

/// active while the machine is awake and at least one window is visible
/// focus alone doesnt count, a visible chat behind another app still shows live messages
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ApplicationActivityGate {
    awake: bool,
    window_visible: bool,
}

impl Default for ApplicationActivityGate {
    fn default() -> Self {
        Self {
            awake: true,
            window_visible: false,
        }
    }
}

impl ApplicationActivityGate {
    /// feeds one system event, Some(active) for sleep and wake and None for a network snapshot
    pub fn system_event(&mut self, event: SystemActivityEvent) -> Option<bool> {
        match event {
            SystemActivityEvent::WillSleep => self.awake = false,
            SystemActivityEvent::DidWake => self.awake = true,
            SystemActivityEvent::NetworkPath { .. } => return None,
        }
        Some(self.is_active())
    }

    /// feeds the window visibility from gpui and returns the current value
    pub fn window_visibility(&mut self, any_window_visible: bool) -> bool {
        self.window_visible = any_window_visible;
        self.is_active()
    }

    pub fn is_active(&self) -> bool {
        self.awake && self.window_visible
    }
}

/// both halves of the seam for the life of the value
/// halves the host cant observe go in gaps with a reason, app logs each once
#[must_use = "dropping the watches stops system activity events"]
#[derive(Debug)]
pub struct SystemActivityWatches {
    _watches: Vec<ActivityWatch>,
    gaps: Vec<SystemActivityUnavailable>,
}

impl SystemActivityWatches {
    pub fn gaps(&self) -> &[SystemActivityUnavailable] {
        &self.gaps
    }
}

/// starts both halves on source, the testable core of watch
pub fn watch_on(
    source: &impl SystemActivitySource,
    on_event: impl Fn(SystemActivityEvent) + Send + Sync + 'static,
) -> SystemActivityWatches {
    let sink: EventSink = std::sync::Arc::new(on_event);
    let mut watches = Vec::new();
    let mut gaps = Vec::new();
    for started in [
        source.watch_sleep(std::sync::Arc::clone(&sink)),
        source.watch_network_path(sink),
    ] {
        match started {
            Ok(watch) => watches.push(watch),
            Err(gap) => gaps.push(gap),
        }
    }
    SystemActivityWatches {
        _watches: watches,
        gaps,
    }
}

/// starts watching sleep, wake and network path
/// on_event runs on a system thread so hop to the socket owner
pub fn watch(
    on_event: impl Fn(SystemActivityEvent) + Send + Sync + 'static,
) -> SystemActivityWatches {
    watch_on(&super::os::NativeSystemActivity, on_event)
}
