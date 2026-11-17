use rise_core::foundation::foundation_calendar::TimeZone;

use crate::host_os::HostOs;
use crate::system_activity::ActivityWatch;

pub const TIMEDATE1_OBJECT_PATH: &str = "/org/freedesktop/timedate1";
pub const TIMEDATE1_INTERFACE: &str = "org.freedesktop.timedate1";
pub const TIMEDATE1_TIMEZONE_PROPERTY: &str = "Timezone";
pub const DBUS_PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
pub const DBUS_PROPERTIES_CHANGED_SIGNAL: &str = "PropertiesChanged";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemClockChange {
    TimeZone,
    Locale,
}

/// where an os layer reports a change, on a thread of the layer
pub type SystemClockSink = Box<dyn Fn(SystemClockChange) + Send>;

/// why changes are not heard, caller still re-reads on activation so the value arrives later
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SystemClockUnavailable {
    #[error("system clock changes are not written for {0:?}")]
    BranchNotWritten(HostOs),
    #[error("no system service announces clock changes: {0}")]
    NoService(String),
}

pub trait SystemClockSource {
    /// iana name of the system zone read right now, for oses with no file that says it
    fn system_zone_name(&self) -> Option<String> {
        None
    }

    /// starts hearing changes until the watch drops
    /// doesnt block so main thread is fine, sink runs once the new value is readable
    fn watch(&self, sink: SystemClockSink) -> Result<ActivityWatch, SystemClockUnavailable> {
        drop(sink);
        Err(SystemClockUnavailable::BranchNotWritten(HostOs::current()))
    }
}

/// starts hearing system clock changes until the watch drops
pub fn watch(
    on_change: impl Fn(SystemClockChange) + Send + 'static,
) -> Result<ActivityWatch, SystemClockUnavailable> {
    TimeZone::set_system_zone_name_source(|| super::os::NativeSystemClock.system_zone_name());
    super::os::NativeSystemClock.watch(Box::new(on_change))
}

/// true when a timedate1 PropertiesChanged is about the zone
/// same signal also covers ntp and rtc switches which dont touch the zone
pub fn timedate_change_touches_zone<'a>(
    mut changed: impl Iterator<Item = &'a str>,
    invalidated: &[String],
) -> bool {
    changed.any(|name| name == TIMEDATE1_TIMEZONE_PROPERTY)
        || invalidated.iter().any(|name| name == TIMEDATE1_TIMEZONE_PROPERTY)
}

pub const WINDOWS_LOCALE_SETTING: &str = "intl";

/// true when a WM_SETTINGCHANGE is about the locale
/// same message carries every other system parameter
pub fn windows_setting_is_locale(setting: Option<&str>) -> bool {
    setting == Some(WINDOWS_LOCALE_SETTING)
}

/// remembers the zone name and says if it moved
/// wm_timechange fires on clock sets too so only a new name counts
pub fn zone_name_moved(seen: &mut Option<String>, current: Option<String>) -> bool {
    let Some(current) = current else {
        return false;
    };
    if seen.as_deref() == Some(current.as_str()) {
        return false;
    }
    *seen = Some(current);
    true
}
