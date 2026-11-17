use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use rise_platform::HostOs;
use thiserror::Error;

/// why the audio route changed, raw values match the system ones
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AudioRouteChangeReason {
    Unknown,
    NewDeviceAvailable,
    OldDeviceUnavailable,
    CategoryChange,
    Override,
    WakeFromSleep,
    NoSuitableRouteForCategory,
    RouteConfigurationChange,
}

impl AudioRouteChangeReason {
    pub const fn raw_value(self) -> u64 {
        match self {
            Self::Unknown => 0,
            Self::NewDeviceAvailable => 1,
            Self::OldDeviceUnavailable => 2,
            Self::CategoryChange => 3,
            Self::Override => 4,
            Self::WakeFromSleep => 6,
            Self::NoSuitableRouteForCategory => 7,
            Self::RouteConfigurationChange => 8,
        }
    }

    pub const fn from_raw_value(raw_value: u64) -> Option<Self> {
        match raw_value {
            0 => Some(Self::Unknown),
            1 => Some(Self::NewDeviceAvailable),
            2 => Some(Self::OldDeviceUnavailable),
            3 => Some(Self::CategoryChange),
            4 => Some(Self::Override),
            6 => Some(Self::WakeFromSleep),
            7 => Some(Self::NoSuitableRouteForCategory),
            8 => Some(Self::RouteConfigurationChange),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AudioRouteChange {
    pub reason: AudioRouteChangeReason,
}

/// what the host plays through, the default output and every output present
/// ids are the host own and only compared
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioOutputSnapshot {
    pub default_output: Option<String>,
    pub outputs: BTreeSet<String>,
}

/// names the change between two snapshots like a route change
/// route is the default output, leaving it alone isnt a change
/// leaving is old device unavailable
pub fn route_change_reason(
    previous: &AudioOutputSnapshot,
    current: &AudioOutputSnapshot,
) -> Option<AudioRouteChangeReason> {
    if previous.default_output == current.default_output {
        return None;
    }
    let old_left = previous
        .default_output
        .as_ref()
        .is_some_and(|old| !current.outputs.contains(old));
    if old_left {
        return Some(AudioRouteChangeReason::OldDeviceUnavailable);
    }
    match &current.default_output {
        None => Some(AudioRouteChangeReason::OldDeviceUnavailable),
        Some(new) if !previous.outputs.contains(new) => {
            Some(AudioRouteChangeReason::NewDeviceAvailable)
        }
        Some(_) if previous.default_output.is_none() => {
            Some(AudioRouteChangeReason::NewDeviceAvailable)
        }
        Some(_) => Some(AudioRouteChangeReason::Override),
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AudioRouteError {
    #[error("output routes cannot be watched on {host:?}: {reason}")]
    Unavailable { host: HostOs, reason: String },
    #[error("output route call {call} failed: {detail}")]
    Failed { call: &'static str, detail: String },
}

/// called by an os branch when the default output or the set of outputs may have changed
/// cheap and thread safe since os callbacks run on host threads
pub type RouteSignal = Arc<dyn Fn() + Send + Sync>;

/// live watch over the host outputs, lives on the monitor thread only
/// dropping it removes every listener it registered
pub trait AudioRouteBackend {
    fn snapshot(&mut self) -> Result<AudioOutputSnapshot, AudioRouteError>;
}

/// the os branch, opens the watch on the monitor thread
/// so it can keep thread bound handles (com apartments) for its whole life
pub trait AudioRouteSource {
    fn open(&self, signal: RouteSignal) -> Result<Box<dyn AudioRouteBackend>, AudioRouteError>;
}

const SIGNAL_SETTLE: Duration = Duration::from_millis(150);

enum MonitorMessage {
    Changed,
    Stop,
}

/// watches host outputs on its own thread and reports each route change, dropping it stops the watch
pub struct AudioRouteMonitor {
    messages: Sender<MonitorMessage>,
}

impl AudioRouteMonitor {
    /// starts watching, on_change runs on the monitor thread and never on the main thread
    pub fn start(
        on_change: impl Fn(AudioRouteChange) + Send + 'static,
    ) -> Result<Self, AudioRouteError> {
        Self::start_with(Box::new(super::os::NativeRouteSource), on_change)
    }

    /// starts over a given source, tests drive the shared half through it
    pub fn start_with(
        source: Box<dyn AudioRouteSource + Send>,
        on_change: impl Fn(AudioRouteChange) + Send + 'static,
    ) -> Result<Self, AudioRouteError> {
        let (messages, inbox) = mpsc::channel::<MonitorMessage>();
        let (ready, opened) = mpsc::sync_channel::<Result<(), AudioRouteError>>(1);
        let signal_sender = messages.clone();
        let signal: RouteSignal = Arc::new(move || {
            let _ = signal_sender.send(MonitorMessage::Changed);
        });
        thread::Builder::new()
            .name("rise-audio-route".to_owned())
            .spawn(move || run_monitor(source, signal, inbox, ready, on_change))
            .map_err(|error| AudioRouteError::Failed {
                call: "thread spawn",
                detail: error.to_string(),
            })?;
        match opened.recv() {
            Ok(Ok(())) => Ok(Self { messages }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(AudioRouteError::Failed {
                call: "monitor start",
                detail: "the monitor thread ended before it opened the watch".to_owned(),
            }),
        }
    }
}

impl Drop for AudioRouteMonitor {
    fn drop(&mut self) {
        let _ = self.messages.send(MonitorMessage::Stop);
    }
}

fn run_monitor(
    source: Box<dyn AudioRouteSource + Send>,
    signal: RouteSignal,
    inbox: Receiver<MonitorMessage>,
    ready: mpsc::SyncSender<Result<(), AudioRouteError>>,
    on_change: impl Fn(AudioRouteChange),
) {
    let mut backend = match source.open(signal) {
        Ok(backend) => backend,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut last = match backend.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    loop {
        match inbox.recv() {
            Ok(MonitorMessage::Changed) => {}
            Ok(MonitorMessage::Stop) | Err(_) => break,
        }
        if !settle(&inbox) {
            break;
        }
        match backend.snapshot() {
            Ok(current) => {
                if let Some(reason) = route_change_reason(&last, &current) {
                    on_change(AudioRouteChange { reason });
                }
                last = current;
            }
            Err(error) => tracing::warn!(%error, "audio route snapshot failed"),
        }
    }
    drop(backend);
}

/// waits until no signal came for SIGNAL_SETTLE, false if the monitor was stopped
fn settle(inbox: &Receiver<MonitorMessage>) -> bool {
    loop {
        match inbox.recv_timeout(SIGNAL_SETTLE) {
            Ok(MonitorMessage::Changed) => continue,
            Ok(MonitorMessage::Stop) | Err(RecvTimeoutError::Disconnected) => return false,
            Err(RecvTimeoutError::Timeout) => return true,
        }
    }
}
