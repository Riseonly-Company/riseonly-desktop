use std::path::{Path, PathBuf};

use super::system_activity::SystemActivityEvent;

pub const LOGIN1_SERVICE: &str = "org.freedesktop.login1";
pub const LOGIN1_PATH: &str = "/org/freedesktop/login1";
pub const LOGIN1_MANAGER_INTERFACE: &str = "org.freedesktop.login1.Manager";
pub const PREPARE_FOR_SLEEP_SIGNAL: &str = "PrepareForSleep";

pub const NETWORK_MANAGER_SERVICE: &str = "org.freedesktop.NetworkManager";
pub const NETWORK_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
pub const NETWORK_MANAGER_INTERFACE: &str = "org.freedesktop.NetworkManager";
pub const NETWORK_MANAGER_CONNECTIVITY: &str = "Connectivity";
pub const NETWORK_MANAGER_STATE: &str = "State";

pub const PORTAL_SERVICE: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
pub const NETWORK_MONITOR_INTERFACE: &str = "org.freedesktop.portal.NetworkMonitor";
pub const NETWORK_MONITOR_GET_AVAILABLE: &str = "GetAvailable";
pub const NETWORK_MONITOR_CHANGED_SIGNAL: &str = "changed";

pub const NM_CONNECTIVITY_UNKNOWN: u32 = 0;
pub const NM_CONNECTIVITY_NONE: u32 = 1;
pub const NM_CONNECTIVITY_PORTAL: u32 = 2;
pub const NM_CONNECTIVITY_LIMITED: u32 = 3;
pub const NM_CONNECTIVITY_FULL: u32 = 4;

pub const NM_STATE_CONNECTED_LOCAL: u32 = 50;

pub const DEFAULT_SYSTEM_BUS_ADDRESS: &str = "unix:path=/var/run/dbus/system_bus_socket";

pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";

/// true right before the machine sleeps and false right after it woke
pub fn prepare_for_sleep_event(start: bool) -> SystemActivityEvent {
    if start {
        SystemActivityEvent::WillSleep
    } else {
        SystemActivityEvent::DidWake
    }
}

/// true when networkmanager says the network is usable, only NONE counts as not
/// when UNKNOWN the check is off or hasnt run so the overall state decides
pub fn network_manager_satisfied(connectivity: u32, state: u32) -> bool {
    match connectivity {
        NM_CONNECTIVITY_NONE => false,
        NM_CONNECTIVITY_PORTAL | NM_CONNECTIVITY_LIMITED | NM_CONNECTIVITY_FULL => true,
        _ => state >= NM_STATE_CONNECTED_LOCAL,
    }
}

/// services that can answer the network path, in the order linux tries them
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NetworkPathRoute {
    NetworkManager,
    Portal,
}

/// in a sandbox only the portal, outside it networkmanager first then the portal
/// portal also covers machines without networkmanager
pub fn network_path_routes(sandboxed: bool) -> &'static [NetworkPathRoute] {
    if sandboxed {
        &[NetworkPathRoute::Portal]
    } else {
        &[NetworkPathRoute::NetworkManager, NetworkPathRoute::Portal]
    }
}

/// says if the socket of a bus can be found before connecting
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BusSocket {
    Path(PathBuf),
    Unverifiable,
    Missing,
}

/// reads a dbus address list and names the first unix path, like the client lib would
pub fn bus_socket(address: Option<&str>) -> BusSocket {
    let Some(address) = address.filter(|value| !value.is_empty()) else {
        return BusSocket::Missing;
    };
    let mut other_address = false;
    for entry in address.split(';').filter(|entry| !entry.is_empty()) {
        let Some((transport, pairs)) = entry.split_once(':') else {
            continue;
        };
        if transport != "unix" {
            other_address = true;
            continue;
        }
        for pair in pairs.split(',') {
            match pair.split_once('=') {
                Some(("path", value)) => {
                    if let Some(path) = unescape_bus_value(value) {
                        return BusSocket::Path(PathBuf::from(path));
                    }
                }
                Some(("abstract" | "dir" | "tmpdir" | "runtime", _)) => other_address = true,
                _ => {}
            }
        }
    }
    if other_address {
        BusSocket::Unverifiable
    } else {
        BusSocket::Missing
    }
}

/// system bus address from DBUS_SYSTEM_BUS_ADDRESS or the spec default
pub fn system_bus_socket(env_address: Option<&str>) -> BusSocket {
    match env_address.filter(|value| !value.is_empty()) {
        Some(address) => bus_socket(Some(address)),
        None => bus_socket(Some(DEFAULT_SYSTEM_BUS_ADDRESS)),
    }
}

/// session bus address from DBUS_SESSION_BUS_ADDRESS or XDG_RUNTIME_DIR/bus
pub fn session_bus_socket(env_address: Option<&str>, runtime_dir: Option<&Path>) -> BusSocket {
    match env_address.filter(|value| !value.is_empty()) {
        Some(address) => bus_socket(Some(address)),
        None => match runtime_dir {
            Some(dir) if dir.has_root() => BusSocket::Path(dir.join("bus")),
            _ => BusSocket::Missing,
        },
    }
}

/// why the bus cant be reached, for the gap the app logs
pub fn missing_bus_detail(bus: &str, socket: &BusSocket) -> Option<String> {
    match socket {
        BusSocket::Missing => Some(format!("no {bus} bus address")),
        BusSocket::Path(path) if !path.exists() => {
            Some(format!("no {bus} bus socket at {}", path.display()))
        }
        BusSocket::Path(_) | BusSocket::Unverifiable => None,
    }
}

fn unescape_bus_value(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = value.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok().filter(|text| !text.is_empty())
}

/// keeps the last reported path so a burst of changes reports once
/// first snapshot always reports
#[derive(Clone, Copy, Default, Debug)]
pub struct NetworkPathReporter {
    last: Option<bool>,
}

impl NetworkPathReporter {
    pub fn report(&mut self, satisfied: bool) -> Option<SystemActivityEvent> {
        if self.last == Some(satisfied) {
            return None;
        }
        self.last = Some(satisfied);
        Some(SystemActivityEvent::NetworkPath { satisfied })
    }
}
