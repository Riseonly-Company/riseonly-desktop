use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ImpactStyle {
    Light,
    #[default]
    Medium,
    Heavy,
    Soft,
    Rigid,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NotificationKind {
    Success,
    Warning,
    Error,
}

/// one feedback event as a feature asks for it
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Feedback {
    Selection,
    Impact(ImpactStyle),
    Notification(NotificationKind),
}

/// patterns a desktop haptic device offers
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HapticPattern {
    Generic,
    Alignment,
    LevelChange,
}

/// pattern that stands in for a feedback event
/// appkit has three patterns for eleven events so each maps to the one whose documented meaning fits
pub fn pattern_for(feedback: Feedback) -> HapticPattern {
    match feedback {
        Feedback::Selection => HapticPattern::Alignment,
        Feedback::Impact(_) => HapticPattern::Generic,
        Feedback::Notification(_) => HapticPattern::LevelChange,
    }
}

/// why no haptic plays, every variant leaves the feature working and only the feedback is missing
#[derive(Clone, Copy, PartialEq, Eq, Debug, Error)]
pub enum HapticsUnavailable {
    #[error("{0:?} has no system haptic feedback for app interactions")]
    NoSystemHaptics(HostOs),
    #[error("haptic feedback is not written for {0:?}")]
    BranchNotWritten(HostOs),
}

/// default answer of an os layer that doesnt override the haptic performer
/// freedesktop has no haptic interface and the windows apis drive a device the app opened (pen etc)
pub const fn unavailable_by_default(host: HostOs) -> HapticsUnavailable {
    match host {
        HostOs::Linux | HostOs::Windows => HapticsUnavailable::NoSystemHaptics(host),
        HostOs::MacOs => HapticsUnavailable::BranchNotWritten(host),
    }
}

pub trait HapticPerformer {
    /// true when the system takes haptic requests at all
    /// ok doesnt promise a felt bump, no actuator or haptics turned off plays nothing
    fn available(&self) -> Result<(), HapticsUnavailable> {
        Err(unavailable_by_default(HostOs::current()))
    }

    /// plays the pattern now
    fn perform(&self, _pattern: HapticPattern) -> Result<(), HapticsUnavailable> {
        Err(unavailable_by_default(HostOs::current()))
    }
}

/// answer of a haptic call, the reason of a refusal goes to the trace log
/// so a feature can ignore the return value
pub fn support_of(answer: Result<(), HapticsUnavailable>) -> PlatformSupport {
    match answer {
        Ok(()) => PlatformSupport::Performed,
        Err(reason) => {
            tracing::trace!(target: "riseonly", %reason, "haptic feedback not played");
            PlatformSupport::Unsupported
        }
    }
}

/// true when haptics can play on this host, with the reason when not
pub fn availability() -> Result<(), HapticsUnavailable> {
    super::os::NativeHaptics.available()
}

pub fn play(feedback: Feedback) -> PlatformSupport {
    support_of(super::os::NativeHaptics.perform(pattern_for(feedback)))
}

pub fn selection_changed() -> PlatformSupport {
    play(Feedback::Selection)
}

pub fn impact(style: ImpactStyle) -> PlatformSupport {
    play(Feedback::Impact(style))
}

pub fn notification(kind: NotificationKind) -> PlatformSupport {
    play(Feedback::Notification(kind))
}

/// prepare, ios wakes the taptic engine ahead of the event but appkit has no warm up
/// so this only asks whether the host plays haptics at all
pub fn prepare() -> PlatformSupport {
    support_of(availability())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SelectionFeedbackGenerator;

impl SelectionFeedbackGenerator {
    pub const fn new() -> Self {
        Self
    }

    pub fn prepare(&self) -> PlatformSupport {
        prepare()
    }

    pub fn selection_changed(&self) -> PlatformSupport {
        selection_changed()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ImpactFeedbackGenerator {
    style: ImpactStyle,
}

impl ImpactFeedbackGenerator {
    pub const fn new(style: ImpactStyle) -> Self {
        Self { style }
    }

    pub const fn style(&self) -> ImpactStyle {
        self.style
    }

    pub fn prepare(&self) -> PlatformSupport {
        prepare()
    }

    pub fn impact_occurred(&self) -> PlatformSupport {
        impact(self.style)
    }

    /// impact with intensity, appkit has no strength so the generic pattern plays at the system
    /// strength whatever the intensity
    pub fn impact_occurred_with_intensity(&self, _intensity: f64) -> PlatformSupport {
        impact(self.style)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct NotificationFeedbackGenerator;

impl NotificationFeedbackGenerator {
    pub const fn new() -> Self {
        Self
    }

    pub fn prepare(&self) -> PlatformSupport {
        prepare()
    }

    pub fn notification_occurred(&self, kind: NotificationKind) -> PlatformSupport {
        notification(kind)
    }
}
