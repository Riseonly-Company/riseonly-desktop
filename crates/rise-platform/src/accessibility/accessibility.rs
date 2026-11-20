use std::sync::Arc;

use parking_lot::Mutex;
use thiserror::Error;

use crate::host_os::HostOs;

/// why the setting cant be read or watched, the app still works and the chrome keeps its glass
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum AccessibilityUnavailable {
    #[error("the accessibility settings are not written for {0:?}")]
    BranchNotWritten(HostOs),
    #[error("the system has no such accessibility setting: {0}")]
    Unsupported(&'static str),
    #[error("the system refused the accessibility setting: {0}")]
    SourceRefused(String),
}

/// default answer of an os layer that doesnt override an accessibility method
pub const fn unavailable_by_default(host: HostOs) -> AccessibilityUnavailable {
    AccessibilityUnavailable::BranchNotWritten(host)
}

/// true where the system reduce motion switch also stops stickers, gifs and animated emoji
pub const fn reduce_motion_stops_content_playback(host: HostOs) -> bool {
    matches!(host, HostOs::MacOs)
}

/// where an os layer delivers each reduce transparency value it sees
/// called on a system thread never main, and may repeat the last value
pub type ReduceTransparencySink = Box<dyn Fn(bool) + Send + Sync>;

/// same as the transparency sink but for reduce motion, same thread and repeat rules
pub type ReduceMotionSink = Box<dyn Fn(bool) + Send + Sync>;

pub trait AccessibilitySettings {
    /// current value of the system reduce transparency setting
    fn reduce_transparency(&self) -> Result<bool, AccessibilityUnavailable> {
        Err(unavailable_by_default(HostOs::current()))
    }

    fn watch_reduce_transparency(
        &self,
        sink: ReduceTransparencySink,
    ) -> Result<AccessibilityWatch, AccessibilityUnavailable> {
        drop(sink);
        Err(unavailable_by_default(HostOs::current()))
    }

    /// current value of the system reduce motion setting
    fn reduce_motion(&self) -> Result<bool, AccessibilityUnavailable> {
        Err(unavailable_by_default(HostOs::current()))
    }

    fn watch_reduce_motion(
        &self,
        sink: ReduceMotionSink,
    ) -> Result<AccessibilityWatch, AccessibilityUnavailable> {
        drop(sink);
        Err(unavailable_by_default(HostOs::current()))
    }
}

/// keeps the system observer alive, dropping it stops the changes
#[must_use = "dropping the watch stops accessibility setting changes"]
pub struct AccessibilityWatch {
    _source: Box<dyn Send>,
}

impl AccessibilityWatch {
    /// source removes the system observer in its own drop
    pub(crate) fn new(source: impl Send + 'static) -> Self {
        Self {
            _source: Box::new(source),
        }
    }
}

impl std::fmt::Debug for AccessibilityWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AccessibilityWatch")
    }
}

/// passes a value on only when it differs from the last known one
/// so the shell republishes the environment once per real change
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SettingChangeGate {
    last: Option<bool>,
}

impl SettingChangeGate {
    /// takes value as the known one only when nothing is known yet, returns the known one
    /// a change seen before the first read wins, its the newer value
    pub fn seed(&mut self, value: bool) -> bool {
        *self.last.get_or_insert(value)
    }

    /// true when value is a change to deliver, it becomes the known value either way
    pub fn should_deliver(&mut self, value: bool) -> bool {
        let changed = self.last != Some(value);
        self.last = Some(value);
        changed
    }

    pub fn last(&self) -> Option<bool> {
        self.last
    }
}

/// windows transparency effects switch is advanced effects enabled
/// switch off means reduce transparency
pub const fn reduce_transparency_from_advanced_effects(advanced_effects_enabled: bool) -> bool {
    !advanced_effects_enabled
}

/// value the kit sees when the layer has no answer, false
pub fn reduce_transparency_or_default(read: Result<bool, AccessibilityUnavailable>) -> bool {
    match read {
        Ok(reduce) => reduce,
        Err(reason) => {
            tracing::warn!(target: "riseonly", %reason, "reduce transparency unknown; glass stays");
            false
        }
    }
}

/// current reduce transparency value, false when the system has no answer
pub fn reduce_transparency() -> bool {
    reduce_transparency_or_default(super::os::NativeAccessibility.reduce_transparency())
}

#[derive(Debug)]
pub struct ReduceTransparencyWatch {
    pub current: bool,
    pub watch: AccessibilityWatch,
}

pub fn watch_reduce_transparency(
    on_change: impl Fn(bool) + Send + Sync + 'static,
) -> Result<ReduceTransparencyWatch, AccessibilityUnavailable> {
    let gate = Arc::new(Mutex::new(SettingChangeGate::default()));
    let sink_gate = Arc::clone(&gate);
    let sink: ReduceTransparencySink = Box::new(move |reduce| {
        if sink_gate.lock().should_deliver(reduce) {
            on_change(reduce);
        }
    });
    let layer = super::os::NativeAccessibility;
    let watch = layer.watch_reduce_transparency(sink)?;
    let read = reduce_transparency_or_default(layer.reduce_transparency());
    let current = gate.lock().seed(read);
    Ok(ReduceTransparencyWatch { current, watch })
}

/// value gpui and the kit see when the layer has no answer, false so animations keep running
pub fn reduce_motion_or_default(read: Result<bool, AccessibilityUnavailable>) -> bool {
    match read {
        Ok(reduce) => reduce,
        Err(reason) => {
            tracing::warn!(target: "riseonly", %reason, "reduce motion unknown; motion stays");
            false
        }
    }
}

/// current reduce motion value, false when the system has no answer
pub fn reduce_motion() -> bool {
    reduce_motion_or_default(super::os::NativeAccessibility.reduce_motion())
}

#[derive(Debug)]
pub struct ReduceMotionWatch {
    pub current: bool,
    pub watch: AccessibilityWatch,
}

pub fn watch_reduce_motion(
    on_change: impl Fn(bool) + Send + Sync + 'static,
) -> Result<ReduceMotionWatch, AccessibilityUnavailable> {
    let gate = Arc::new(Mutex::new(SettingChangeGate::default()));
    let sink_gate = Arc::clone(&gate);
    let sink: ReduceMotionSink = Box::new(move |reduce| {
        if sink_gate.lock().should_deliver(reduce) {
            on_change(reduce);
        }
    });
    let layer = super::os::NativeAccessibility;
    let watch = layer.watch_reduce_motion(sink)?;
    let read = reduce_motion_or_default(layer.reduce_motion());
    let current = gate.lock().seed(read);
    Ok(ReduceMotionWatch { current, watch })
}
