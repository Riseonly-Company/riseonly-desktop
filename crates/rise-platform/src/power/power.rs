use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use thiserror::Error;

use crate::host_os::HostOs;

pub const BATTERY_LEVEL_DID_CHANGE: &str = "UIDeviceBatteryLevelDidChangeNotification";
pub const BATTERY_STATE_DID_CHANGE: &str = "UIDeviceBatteryStateDidChangeNotification";
pub const POWER_STATE_DID_CHANGE: &str = "NSProcessInfoPowerStateDidChangeNotification";

/// battery state with the ios raw values
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum BatteryState {
    #[default]
    Unknown,
    Unplugged,
    Charging,
    Full,
}

impl BatteryState {
    pub const ALL: [Self; 4] = [Self::Unknown, Self::Unplugged, Self::Charging, Self::Full];

    pub const fn raw_value(self) -> i64 {
        match self {
            Self::Unknown => 0,
            Self::Unplugged => 1,
            Self::Charging => 2,
            Self::Full => 3,
        }
    }

    pub const fn from_raw_value(raw: i64) -> Option<Self> {
        match raw {
            0 => Some(Self::Unknown),
            1 => Some(Self::Unplugged),
            2 => Some(Self::Charging),
            3 => Some(Self::Full),
            _ => None,
        }
    }

    /// state from what a laptop power source reports
    /// on external power with the battery neither charging nor full (a charge hold) its charging
    pub const fn from_power_source(on_external_power: bool, is_charged: bool) -> Self {
        match (on_external_power, is_charged) {
            (false, _) => Self::Unplugged,
            (true, true) => Self::Full,
            (true, false) => Self::Charging,
        }
    }
}

/// the battery as a whole percent plus the state
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct BatteryReading {
    pub percent: Option<u8>,
    pub state: BatteryState,
    pub present: Option<bool>,
}

impl BatteryReading {
    /// the system said the machine has no battery
    pub const NO_BATTERY: Self = Self {
        percent: None,
        state: BatteryState::Unknown,
        present: Some(false),
    };

    /// no answer about a battery at all
    pub const UNKNOWN: Self = Self {
        percent: None,
        state: BatteryState::Unknown,
        present: None,
    };
}

/// everything the energy saving store reads in one go
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct PowerReading {
    pub battery: BatteryReading,
    pub low_power_mode: bool,
}

impl PowerReading {
    /// notifications a move from previous to self stands for, in posting order
    /// empty when nothing the store reads has changed
    pub fn changes_since(&self, previous: &Self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.battery.percent != previous.battery.percent {
            names.push(BATTERY_LEVEL_DID_CHANGE);
        }
        if self.battery.state != previous.battery.state {
            names.push(BATTERY_STATE_DID_CHANGE);
        }
        if self.low_power_mode != previous.low_power_mode {
            names.push(POWER_STATE_DID_CHANGE);
        }
        names
    }
}

/// level 0..1 to a whole percent, a negative level (no battery) is none
pub fn battery_percent_from_level(level: f32) -> Option<u8> {
    if level.is_nan() || level < 0.0 {
        return None;
    }
    let percent = (level * 100.0).round().min(100.0);
    Some(percent as u8)
}

/// whole percent from current and full capacity, rounded half up
/// an impossible capacity (negative or no full capacity) is no reading
pub fn battery_percent_from_capacity(current: i64, max: i64) -> Option<u8> {
    if current < 0 || max <= 0 {
        return None;
    }
    let current = current.min(max);
    let percent = (current * 200 + max) / (2 * max);
    u8::try_from(percent).ok()
}

/// whole percent from a 0..100 percentage, rounded half up
/// nan or negative is no reading
pub fn battery_percent_from_percentage(percentage: f64) -> Option<u8> {
    if percentage.is_nan() || percentage < 0.0 {
        return None;
    }
    let percent = percentage.round().min(100.0);
    Some(percent as u8)
}

/// battery state from the state of a upower device
pub const fn battery_state_from_upower(state: u32) -> BatteryState {
    match state {
        1 => BatteryState::from_power_source(true, false),
        2 | 3 | 6 => BatteryState::Unplugged,
        4 => BatteryState::from_power_source(true, true),
        5 => BatteryState::from_power_source(true, false),
        _ => BatteryState::Unknown,
    }
}

/// battery from the upower display device
/// no battery present means none, ios reports level -1 there
pub fn battery_reading_from_upower(is_present: bool, percentage: f64, state: u32) -> BatteryReading {
    if !is_present {
        return BatteryReading::NO_BATTERY;
    }
    BatteryReading {
        percent: battery_percent_from_percentage(percentage),
        state: battery_state_from_upower(state),
        present: Some(true),
    }
}

pub const POWER_SAVER_PROFILE: &str = "power-saver";

/// low power mode from the power profiles daemon active profile
pub fn low_power_mode_from_profile(active_profile: &str) -> bool {
    active_profile == POWER_SAVER_PROFILE
}

/// dbus error names meaning this daemon or object isnt on the bus, which isnt a failure
/// a machine without upower or power profiles has no battery and no low power mode
pub fn is_absent_error_name(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.DBus.Error.UnknownInterface"
            | "org.freedesktop.DBus.Error.UnknownProperty"
    )
}

const AC_LINE_ONLINE: u8 = 1;
const AC_LINE_UNKNOWN: u8 = 255;
const BATTERY_FLAG_CHARGING: u8 = 8;
const BATTERY_FLAG_NO_BATTERY: u8 = 128;
const BATTERY_FLAG_UNKNOWN: u8 = 255;
const BATTERY_PERCENT_UNKNOWN: u8 = 255;
const SYSTEM_STATUS_BATTERY_SAVER_ON: u8 = 1;

/// low power mode from the system status flag, 1 is battery saver on
pub const fn low_power_mode_from_system_status(system_status_flag: u8) -> bool {
    system_status_flag == SYSTEM_STATUS_BATTERY_SAVER_ON
}

/// battery from ac line status and battery flag and life percent
/// no battery first (flag 128 unless the whole flag is 255 unknown), then unknown line or flag
/// then power source, 255 or above 100 percent is no reading
pub fn battery_reading_from_system_power_status(
    ac_line_status: u8,
    battery_flag: u8,
    battery_life_percent: u8,
) -> BatteryReading {
    if battery_flag != BATTERY_FLAG_UNKNOWN && battery_flag & BATTERY_FLAG_NO_BATTERY != 0 {
        return BatteryReading::NO_BATTERY;
    }
    let percent = (battery_life_percent != BATTERY_PERCENT_UNKNOWN && battery_life_percent <= 100)
        .then_some(battery_life_percent);
    let state = if ac_line_status == AC_LINE_UNKNOWN || battery_flag == BATTERY_FLAG_UNKNOWN {
        BatteryState::Unknown
    } else {
        let charging = battery_flag & BATTERY_FLAG_CHARGING != 0;
        let is_charged = !charging && percent == Some(100);
        BatteryState::from_power_source(ac_line_status == AC_LINE_ONLINE, is_charged)
    };
    let present = (battery_flag != BATTERY_FLAG_UNKNOWN).then_some(true);
    BatteryReading { percent, state, present }
}

/// which reading an os layer couldnt give
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PowerReadingKind {
    LowPowerMode,
    Battery,
    Changes,
}

impl std::fmt::Display for PowerReadingKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::LowPowerMode => "low power mode",
            Self::Battery => "battery level and state",
            Self::Changes => "power state changes",
        };
        formatter.write_str(text)
    }
}

/// why a reading is missing, every caller still has a working answer (see power_snapshot)
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum PowerUnavailable {
    #[error("{what} is not written for {host:?}")]
    BranchNotWritten { what: PowerReadingKind, host: HostOs },
    #[error("the system refused to report {what}: {detail}")]
    SystemRefused { what: PowerReadingKind, detail: String },
}

/// default answer of an os layer for a reading it doesnt override
pub const fn unavailable_by_default(what: PowerReadingKind, host: HostOs) -> PowerUnavailable {
    PowerUnavailable::BranchNotWritten { what, host }
}

/// where an os layer says power changed so read again
/// called on a system thread and never main, may fire when nothing the store reads changed
pub type ChangeSink = Box<dyn Fn() + Send + Sync>;

pub trait PowerSource {
    /// low power mode right now
    fn low_power_mode(&self) -> Result<bool, PowerUnavailable> {
        Err(unavailable_by_default(PowerReadingKind::LowPowerMode, HostOs::current()))
    }

    /// the internal battery right now, NO_BATTERY on a machine without one
    fn battery(&self) -> Result<BatteryReading, PowerUnavailable> {
        Err(unavailable_by_default(PowerReadingKind::Battery, HostOs::current()))
    }

    /// calls sink whenever the battery or low power mode may have changed, until the watch drops
    fn watch(&self, sink: ChangeSink) -> Result<PowerWatch, PowerUnavailable> {
        drop(sink);
        Err(unavailable_by_default(PowerReadingKind::Changes, HostOs::current()))
    }
}

/// keeps the system observers alive, dropping it stops the change calls
#[must_use = "dropping the watch stops power state changes"]
pub struct PowerWatch {
    _observer: Box<dyn Send>,
}

impl PowerWatch {
    /// observer unregisters itself in its own drop
    /// public so an os layer with no observer to build leaves no dead constructor on its target
    pub fn new(observer: impl Send + 'static) -> Self {
        Self {
            _observer: Box::new(observer),
        }
    }
}

impl std::fmt::Debug for PowerWatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PowerWatch")
    }
}

/// a reading with the reasons for every part that is a fallback and not the system answer
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PowerSnapshot {
    pub reading: PowerReading,
    pub unavailable: Vec<PowerUnavailable>,
}

impl PowerSnapshot {
    /// without a system answer low power mode is off and there is no battery
    /// so power saving follows only the reader threshold
    pub fn from_results(
        low_power_mode: Result<bool, PowerUnavailable>,
        battery: Result<BatteryReading, PowerUnavailable>,
    ) -> Self {
        let mut unavailable = Vec::new();
        let low_power_mode = low_power_mode.unwrap_or_else(|reason| {
            unavailable.push(reason);
            false
        });
        let battery = battery.unwrap_or_else(|reason| {
            unavailable.push(reason);
            BatteryReading::UNKNOWN
        });
        Self {
            reading: PowerReading {
                battery,
                low_power_mode,
            },
            unavailable,
        }
    }
}

/// what the energy saving store reads at start and on every change, plus fallback reasons
/// the app logs the reasons once
/// blocks on dbus so use a plain thread or spawn_blocking, never main or an async worker
pub fn power_snapshot() -> PowerSnapshot {
    let source = super::os::NativePower;
    let snapshot = PowerSnapshot::from_results(source.low_power_mode(), source.battery());
    LAST_LOW_POWER_MODE.store(snapshot.reading.low_power_mode, Ordering::Relaxed);
    snapshot
}

static LAST_LOW_POWER_MODE: AtomicBool = AtomicBool::new(false);

/// low power mode as callers read it, off when the system doesnt say and until the first snapshot
/// never touches the system so any thread may call it
pub fn system_low_power_mode() -> bool {
    LAST_LOW_POWER_MODE.load(Ordering::Relaxed)
}

/// turns os layer read again calls into changes, reads with read
/// calls on_change with the new reading and names only when it differs from the last delivered
pub fn change_sink(
    first: PowerReading,
    read: impl Fn() -> PowerReading + Send + Sync + 'static,
    on_change: impl Fn(PowerReading, &[&'static str]) + Send + Sync + 'static,
) -> ChangeSink {
    let last = Mutex::new(first);
    Box::new(move || {
        let reading = read();
        let names = {
            let mut last = last.lock();
            let names = reading.changes_since(&last);
            *last = reading;
            names
        };
        if names.is_empty() {
            return;
        }
        on_change(reading, &names);
    })
}

/// watches battery and low power mode for as long as the returned watch lives
/// on_change runs on a system thread, hop to main yourself
/// blocking read so same rule as power_snapshot
pub fn watch(
    on_change: impl Fn(PowerReading, &[&'static str]) + Send + Sync + 'static,
) -> Result<PowerWatch, PowerUnavailable> {
    let first = power_snapshot().reading;
    let sink = change_sink(first, || power_snapshot().reading, on_change);
    super::os::NativePower.watch(sink)
}
