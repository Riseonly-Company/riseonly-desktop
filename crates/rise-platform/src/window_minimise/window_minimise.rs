use std::sync::Arc;

use parking_lot::Mutex;
use raw_window_handle::HasWindowHandle;
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// where an os layer delivers the watched window state
/// true while minimised or its app is hidden
/// called on the system posting thread, dont assume main
pub type MinimisedSink = Arc<dyn Fn(bool) + Send + Sync>;

/// why a window minimise state will never be reported
/// window then counts as visible until it loses focus
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum WindowMinimiseUnavailable {
    #[error("window minimise is not observed on {host:?}: the branch is not written")]
    BranchNotWritten { host: HostOs },
    #[error("the window has no native {host:?} handle: {detail}")]
    NoNativeWindow { host: HostOs, detail: String },
}

/// default answer of an os layer that doesnt override WindowMinimiseSource watch
pub const fn unavailable_by_default(host: HostOs) -> WindowMinimiseUnavailable {
    WindowMinimiseUnavailable::BranchNotWritten { host }
}

/// keeps one system observer alive, dropping it stops the reports
/// not Send, created and dropped on the window thread
#[must_use = "dropping the watch stops minimise reports"]
pub struct MinimiseWatch {
    _observer: Box<dyn std::any::Any>,
}

impl MinimiseWatch {
    /// the observer unregisters itself from the system in its own Drop
    pub fn new(observer: impl std::any::Any) -> Self {
        Self {
            _observer: Box::new(observer),
        }
    }
}

impl std::fmt::Debug for MinimiseWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MinimiseWatch")
    }
}

pub trait WindowMinimiseSource {
    /// starts reporting the state of window until the watch drops
    /// first report is the current state, delivered before or right after the call returns
    fn watch(
        &self,
        window: &dyn HasWindowHandle,
        sink: MinimisedSink,
    ) -> Result<MinimiseWatch, WindowMinimiseUnavailable> {
        let _ = window;
        drop(sink);
        Err(unavailable_by_default(HostOs::current()))
    }

    fn hide_window(&self, window: &dyn HasWindowHandle) -> PlatformSupport {
        let _ = window;
        PlatformSupport::Unsupported
    }

    fn show_window(&self, window: &dyn HasWindowHandle) -> PlatformSupport {
        let _ = window;
        PlatformSupport::Unsupported
    }
}

/// passes a report on only when it differs from the last
/// appkit posts both miniaturize and app hide for one trip off screen
#[derive(Debug, Default)]
pub struct MinimisedDeduplicator {
    last: Mutex<Option<bool>>,
}

impl MinimisedDeduplicator {
    /// gives Some(minimised) when the state changed or is the first, None for a repeat
    pub fn report(&self, minimised: bool) -> Option<bool> {
        let mut last = self.last.lock();
        if *last == Some(minimised) {
            return None;
        }
        *last = Some(minimised);
        Some(minimised)
    }

    /// state passed on last, None before the first report
    pub fn last(&self) -> Option<bool> {
        *self.last.lock()
    }
}

/// starts watching window on source, the testable core of watch
pub fn watch_on(
    source: &impl WindowMinimiseSource,
    window: &dyn HasWindowHandle,
    on_change: impl Fn(bool) + Send + Sync + 'static,
) -> Result<MinimiseWatch, WindowMinimiseUnavailable> {
    let deduplicator = Arc::new(MinimisedDeduplicator::default());
    let reporter = Arc::clone(&deduplicator);
    let sink: MinimisedSink = Arc::new(move |minimised| {
        if let Some(changed) = reporter.report(minimised) {
            on_change(changed);
        }
    });
    #[cfg(feature = "dev-drive")]
    dev_drive::register(window, &sink, &deduplicator);
    drop(deduplicator);
    source.watch(window, sink)
}

/// tester stealth mode of the dev drive, a driven minimise or restore of a window
/// a real miniaturize of a concealed window puts a tile in the dock
/// so the state goes into the same sink, false when not watched
#[cfg(feature = "dev-drive")]
pub fn report_for_dev_drive(window: &dyn HasWindowHandle, minimised: bool) -> bool {
    match dev_drive::sink_of(window) {
        Some(sink) => {
            sink(minimised);
            true
        }
        None => false,
    }
}

/// last state the watch of window passed on, from the system or report_for_dev_drive
/// none when not watched or nothing reported yet
#[cfg(feature = "dev-drive")]
pub fn minimised_for_dev_drive(window: &dyn HasWindowHandle) -> Option<bool> {
    dev_drive::last_of(window)
}

#[cfg(feature = "dev-drive")]
mod dev_drive {
    use std::hash::{Hash, Hasher};
    use std::sync::{Arc, Weak};

    use parking_lot::Mutex;
    use raw_window_handle::HasWindowHandle;

    use super::{MinimisedDeduplicator, MinimisedSink};

    struct Watched {
        key: u64,
        sink: Weak<dyn Fn(bool) + Send + Sync>,
        deduplicator: Weak<MinimisedDeduplicator>,
    }

    static WATCHED: Mutex<Vec<Watched>> = Mutex::new(Vec::new());

    fn window_key(window: &dyn HasWindowHandle) -> Option<u64> {
        let handle = window.window_handle().ok()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        handle.as_raw().hash(&mut hasher);
        Some(hasher.finish())
    }

    pub(super) fn register(window: &dyn HasWindowHandle, sink: &MinimisedSink, deduplicator: &Arc<MinimisedDeduplicator>) {
        let Some(key) = window_key(window) else {
            return;
        };
        let mut watched = WATCHED.lock();
        watched.retain(|entry| entry.sink.strong_count() > 0);
        watched.push(Watched {
            key,
            sink: Arc::downgrade(sink),
            deduplicator: Arc::downgrade(deduplicator),
        });
    }

    fn latest(window: &dyn HasWindowHandle) -> Option<(MinimisedSink, Arc<MinimisedDeduplicator>)> {
        let key = window_key(window)?;
        let mut watched = WATCHED.lock();
        watched.retain(|entry| entry.sink.strong_count() > 0);
        watched
            .iter()
            .rev()
            .filter(|entry| entry.key == key)
            .find_map(|entry| Some((entry.sink.upgrade()?, entry.deduplicator.upgrade()?)))
    }

    pub(super) fn sink_of(window: &dyn HasWindowHandle) -> Option<MinimisedSink> {
        latest(window).map(|(sink, _)| sink)
    }

    pub(super) fn last_of(window: &dyn HasWindowHandle) -> Option<bool> {
        latest(window).and_then(|(_, deduplicator)| deduplicator.last())
    }
}

/// starts watching whether window is minimised or its app hidden
/// on_change gets each new state once, the app hops to its own executor
pub fn watch(
    window: &dyn HasWindowHandle,
    on_change: impl Fn(bool) + Send + Sync + 'static,
) -> Result<MinimiseWatch, WindowMinimiseUnavailable> {
    watch_on(&super::os::NativeWindowMinimise, window, on_change)
}

pub fn hide_window(window: &dyn HasWindowHandle) -> PlatformSupport {
    super::os::NativeWindowMinimise.hide_window(window)
}

pub fn show_window(window: &dyn HasWindowHandle) -> PlatformSupport {
    super::os::NativeWindowMinimise.show_window(window)
}
