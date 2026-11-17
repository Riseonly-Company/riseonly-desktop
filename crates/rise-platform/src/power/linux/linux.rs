use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use zbus::blocking::{proxy::Builder, Connection, MessageIterator, Proxy};
use zbus::proxy::CacheProperties;

use crate::power::*;

const UPOWER_SERVICE: &str = "org.freedesktop.UPower";
const DISPLAY_DEVICE_PATH: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const DEVICE_INTERFACE: &str = "org.freedesktop.UPower.Device";

const POWER_PROFILES: [(&str, &str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "org.freedesktop.UPower.PowerProfiles",
    ),
    (
        "net.hadess.PowerProfiles",
        "/net/hadess/PowerProfiles",
        "net.hadess.PowerProfiles",
    ),
];

const WATCH_QUEUE: usize = 16;

pub(crate) struct NativePower;

static READ_BUS: Mutex<Option<Connection>> = Mutex::new(None);

fn read_bus() -> zbus::Result<Connection> {
    let mut slot = READ_BUS.lock();
    if let Some(connection) = slot.as_ref() {
        return Ok(connection.clone());
    }
    let connection = Connection::system()?;
    *slot = Some(connection.clone());
    Ok(connection)
}

fn refused(what: PowerReadingKind, error: impl std::fmt::Display) -> PowerUnavailable {
    PowerUnavailable::SystemRefused {
        what,
        detail: error.to_string(),
    }
}

fn service_absent(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, _, _) => is_absent_error_name(name.as_str()),
        zbus::Error::FDO(error) => matches!(
            **error,
            zbus::fdo::Error::ServiceUnknown(_)
                | zbus::fdo::Error::NameHasNoOwner(_)
                | zbus::fdo::Error::UnknownObject(_)
                | zbus::fdo::Error::UnknownInterface(_)
                | zbus::fdo::Error::UnknownProperty(_)
        ),
        _ => false,
    }
}

fn proxy(
    connection: &Connection,
    service: &'static str,
    path: &'static str,
    interface: &'static str,
) -> zbus::Result<Proxy<'static>> {
    Builder::<'static, Proxy<'static>>::new(connection)
        .destination(service)?
        .path(path)?
        .interface(interface)?
        .cache_properties(CacheProperties::No)
        .build()
}

fn active_profile(connection: &Connection) -> zbus::Result<Option<String>> {
    for (service, path, interface) in POWER_PROFILES {
        let profiles = proxy(connection, service, path, interface)?;
        match profiles.get_property::<String>("ActiveProfile") {
            Ok(profile) => return Ok(Some(profile)),
            Err(error) if service_absent(&error) => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn display_device(connection: &Connection) -> zbus::Result<Option<BatteryReading>> {
    let device = proxy(connection, UPOWER_SERVICE, DISPLAY_DEVICE_PATH, DEVICE_INTERFACE)?;
    let is_present = match device.get_property::<bool>("IsPresent") {
        Ok(is_present) => is_present,
        Err(error) if service_absent(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    if !is_present {
        return Ok(Some(BatteryReading::NO_BATTERY));
    }
    let percentage = device.get_property::<f64>("Percentage")?;
    let state = device.get_property::<u32>("State")?;
    Ok(Some(battery_reading_from_upower(is_present, percentage, state)))
}

/// properties changed of every interface under upower (devices and daemon and new power profiles)
/// or of the legacy hadess power profiles interface
fn properties_changed_rules() -> zbus::Result<[zbus::MatchRule<'static>; 2]> {
    let base = || -> zbus::Result<zbus::match_rule::Builder<'static>> {
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.DBus.Properties")?
            .member("PropertiesChanged")
    };
    Ok([
        base()?.arg0ns(UPOWER_SERVICE)?.build(),
        base()?.arg(0, POWER_PROFILES[1].2)?.build(),
    ])
}

struct WatchThreads {
    stop: Arc<AtomicBool>,
    connection: Option<Connection>,
}

impl Drop for WatchThreads {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(connection) = self.connection.take() {
            let _ = std::thread::Builder::new()
                .name("rise-power-close".into())
                .spawn(move || {
                    let _ = connection.close();
                });
        }
    }
}

fn forward(messages: MessageIterator, stop: &AtomicBool, sink: &ChangeSink) {
    for message in messages {
        if stop.load(Ordering::Acquire) {
            return;
        }
        match message {
            Ok(_) => sink(),
            Err(error) => {
                tracing::warn!(%error, "power watch stream ended");
                return;
            }
        }
    }
}

impl PowerSource for NativePower {
    fn low_power_mode(&self) -> Result<bool, PowerUnavailable> {
        let what = PowerReadingKind::LowPowerMode;
        let connection = read_bus().map_err(|error| refused(what, error))?;
        let profile = active_profile(&connection).map_err(|error| refused(what, error))?;
        Ok(profile.is_some_and(|profile| low_power_mode_from_profile(&profile)))
    }

    fn battery(&self) -> Result<BatteryReading, PowerUnavailable> {
        let what = PowerReadingKind::Battery;
        let connection = read_bus().map_err(|error| refused(what, error))?;
        let reading = display_device(&connection).map_err(|error| refused(what, error))?;
        Ok(reading.unwrap_or(BatteryReading::NO_BATTERY))
    }

    fn watch(&self, sink: ChangeSink) -> Result<PowerWatch, PowerUnavailable> {
        let what = PowerReadingKind::Changes;
        let connection = Connection::system().map_err(|error| refused(what, error))?;
        let rules = properties_changed_rules().map_err(|error| refused(what, error))?;
        let sink = Arc::new(sink);
        let threads = WatchThreads {
            stop: Arc::new(AtomicBool::new(false)),
            connection: Some(connection.clone()),
        };
        for (index, rule) in rules.into_iter().enumerate() {
            let messages = MessageIterator::for_match_rule(rule, &connection, Some(WATCH_QUEUE))
                .map_err(|error| refused(what, error))?;
            let sink = Arc::clone(&sink);
            let stop = Arc::clone(&threads.stop);
            std::thread::Builder::new()
                .name(format!("rise-power-watch-{index}"))
                .spawn(move || forward(messages, &stop, &sink))
                .map_err(|error| refused(what, error))?;
        }
        Ok(PowerWatch::new(threads))
    }
}
