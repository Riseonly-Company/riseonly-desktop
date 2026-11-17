use std::time::Duration;

pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const SETTINGS_INTERFACE: &str = "org.freedesktop.portal.Settings";
pub const READ_ONE_METHOD: &str = "ReadOne";
pub const READ_METHOD: &str = "Read";
pub const SETTING_CHANGED_SIGNAL: &str = "SettingChanged";

pub const APPEARANCE_NAMESPACE: &str = "org.freedesktop.appearance";
pub const REDUCED_MOTION_KEY: &str = "reduced-motion";
pub const GNOME_INTERFACE_NAMESPACE: &str = "org.gnome.desktop.interface";
pub const ENABLE_ANIMATIONS_KEY: &str = "enable-animations";
pub const KDE_GLOBALS_NAMESPACE: &str = "org.kde.kdeglobals.KDE";
pub const ANIMATION_DURATION_FACTOR_KEY: &str = "AnimationDurationFactor";

pub const READ_TIMEOUT: Duration = Duration::from_millis(500);

pub const NOT_READ_YET: &str =
    "the settings portal has not answered yet; the watch delivers the value when it does";

pub const NO_MOTION_SETTING: &str =
    "the settings portal has none of reduced-motion, enable-animations, AnimationDurationFactor";

/// one portal value as the policy needs it, without the dbus types so its testable everywhere
#[derive(Clone, PartialEq, Debug)]
pub enum SettingValue {
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Double(f64),
    Text(String),
    Other,
}

/// portal keys that carry reduce motion
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MotionSource {
    Appearance,
    Gnome,
    Kde,
}

impl MotionSource {
    pub const ALL: [Self; 3] = [Self::Appearance, Self::Gnome, Self::Kde];

    pub const fn namespace(self) -> &'static str {
        match self {
            Self::Appearance => APPEARANCE_NAMESPACE,
            Self::Gnome => GNOME_INTERFACE_NAMESPACE,
            Self::Kde => KDE_GLOBALS_NAMESPACE,
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::Appearance => REDUCED_MOTION_KEY,
            Self::Gnome => ENABLE_ANIMATIONS_KEY,
            Self::Kde => ANIMATION_DURATION_FACTOR_KEY,
        }
    }

    /// source a setting changed signal is about, none for every other setting
    pub fn of(namespace: &str, key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|source| source.namespace() == namespace && source.key() == key)
    }

    /// reduce motion as this key says it, none when the value isnt one the key documents
    pub fn reduce_from(self, value: &SettingValue) -> Option<bool> {
        match self {
            Self::Appearance => match unsigned(value)? {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            },
            Self::Gnome => match value {
                SettingValue::Bool(enabled) => Some(!enabled),
                _ => None,
            },
            Self::Kde => {
                let factor = match value {
                    SettingValue::Double(factor) => *factor,
                    SettingValue::Text(text) => text.parse::<f64>().ok()?,
                    other => unsigned(other)? as f64,
                };
                if factor.is_finite() && factor >= 0.0 {
                    Some(factor == 0.0)
                } else {
                    None
                }
            }
        }
    }
}

fn unsigned(value: &SettingValue) -> Option<u64> {
    match value {
        SettingValue::Unsigned(number) => Some(*number),
        SettingValue::Signed(number) => u64::try_from(*number).ok(),
        _ => None,
    }
}

/// what each key last said, the watch keeps it across setting changed signals
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MotionSettings {
    appearance: Option<bool>,
    gnome: Option<bool>,
    kde: Option<bool>,
}

impl MotionSettings {
    fn slot(&mut self, source: MotionSource) -> &mut Option<bool> {
        match source {
            MotionSource::Appearance => &mut self.appearance,
            MotionSource::Gnome => &mut self.gnome,
            MotionSource::Kde => &mut self.kde,
        }
    }

    /// takes what source says now, a value the key doesnt document clears what it said
    pub fn record(&mut self, source: MotionSource, value: &SettingValue) {
        *self.slot(source) = source.reduce_from(value);
    }

    /// reduce motion from the first key that answers
    pub fn resolved(&self) -> Option<bool> {
        self.appearance.or(self.gnome).or(self.kde)
    }

    /// one setting changed signal, gives the value to deliver if its a motion key and some key answers
    /// none for any other setting
    pub fn apply_change(
        &mut self,
        namespace: &str,
        key: &str,
        value: &SettingValue,
    ) -> Option<bool> {
        let source = MotionSource::of(namespace, key)?;
        self.record(source, value);
        self.resolved()
    }
}

/// last value the portal gave, kept in one atomic byte
/// the bus thread writes it and any thread reads without waiting
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KnownMotion {
    Unknown,
    Known(bool),
}

impl KnownMotion {
    const UNKNOWN: u8 = 0;
    const MOTION: u8 = 1;
    const REDUCE: u8 = 2;

    pub const fn code(self) -> u8 {
        match self {
            Self::Unknown => Self::UNKNOWN,
            Self::Known(false) => Self::MOTION,
            Self::Known(true) => Self::REDUCE,
        }
    }

    /// a byte no writer stores reads as unknown
    pub const fn from_code(code: u8) -> Self {
        match code {
            Self::MOTION => Self::Known(false),
            Self::REDUCE => Self::Known(true),
            _ => Self::Unknown,
        }
    }

    pub const fn value(self) -> Option<bool> {
        match self {
            Self::Unknown => None,
            Self::Known(reduce) => Some(reduce),
        }
    }
}
