use std::ops::{BitOr, BitOrAssign};

use crate::core::desktop_platform::rise_app_events::RiseNotificationName;

/// only the notification name lives here, there is no shared session object
pub struct AVAudioSession;

impl AVAudioSession {
    pub const INTERRUPTION_NOTIFICATION: RiseNotificationName =
        RiseNotificationName::new("AVAudioSessionInterruptionNotification");
}

/// interruption type, raw values match ios so began is 1 and ended is 0
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AVAudioSessionInterruptionType {
    Began,
    Ended,
}

impl AVAudioSessionInterruptionType {
    pub const fn raw_value(self) -> u64 {
        match self {
            Self::Began => 1,
            Self::Ended => 0,
        }
    }

    pub const fn from_raw_value(raw_value: u64) -> Option<Self> {
        match raw_value {
            1 => Some(Self::Began),
            0 => Some(Self::Ended),
            _ => None,
        }
    }
}

/// interruption option set, should_resume is 1 like on ios
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct AVAudioSessionInterruptionOptions {
    raw_value: u64,
}

impl AVAudioSessionInterruptionOptions {
    pub const SHOULD_RESUME: Self = Self { raw_value: 1 };

    pub const fn empty() -> Self {
        Self { raw_value: 0 }
    }

    pub const fn from_raw_value(raw_value: u64) -> Self {
        Self { raw_value }
    }

    pub const fn raw_value(self) -> u64 {
        self.raw_value
    }

    pub const fn contains(self, other: Self) -> bool {
        self.raw_value & other.raw_value == other.raw_value
    }

    pub const fn is_empty(self) -> bool {
        self.raw_value == 0
    }
}

impl BitOr for AVAudioSessionInterruptionOptions {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self {
            raw_value: self.raw_value | rhs.raw_value,
        }
    }
}

impl BitOrAssign for AVAudioSessionInterruptionOptions {
    fn bitor_assign(&mut self, rhs: Self) {
        self.raw_value |= rhs.raw_value;
    }
}

/// payload of the notification, type and options
/// a began post carries empty options since ios omits the key
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AVAudioSessionInterruption {
    pub kind: AVAudioSessionInterruptionType,
    pub options: AVAudioSessionInterruptionOptions,
}
