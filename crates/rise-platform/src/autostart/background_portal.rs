use std::ffi::OsStr;

use super::autostart::AutostartConfig;

pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";
pub const SNAP_NAME_VARIABLE: &str = "SNAP_NAME";

pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const BACKGROUND_INTERFACE: &str = "org.freedesktop.portal.Background";
pub const REQUEST_BACKGROUND_METHOD: &str = "RequestBackground";
pub const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
pub const RESPONSE_SIGNAL: &str = "Response";

pub const HANDLE_TOKEN_OPTION: &str = "handle_token";
pub const AUTOSTART_OPTION: &str = "autostart";
pub const COMMANDLINE_OPTION: &str = "commandline";
pub const DBUS_ACTIVATABLE_OPTION: &str = "dbus-activatable";
pub const AUTOSTART_RESULT: &str = "autostart";

const REQUEST_PATH_PREFIX: &str = "/org/freedesktop/portal/desktop/request";

/// where this process runs, as far as launch at login cares
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sandbox {
    Host,
    Flatpak,
    Snap,
}

impl Sandbox {
    /// flatpak wins over a stray SNAP_NAME, /.flatpak-info exists only inside a flatpak
    pub fn detect(flatpak_info_exists: bool, snap_name: Option<&OsStr>) -> Self {
        if flatpak_info_exists {
            return Self::Flatpak;
        }
        match snap_name {
            Some(name) if !name.is_empty() => Self::Snap,
            _ => Self::Host,
        }
    }

    /// only the host session reads a desktop entry written from here
    pub const fn needs_portal(self) -> bool {
        !matches!(self, Self::Host)
    }
}

/// commandline of request background, the portal runs it inside the sandbox at login
/// so its the path this process started from plus the configured arguments, unescaped
pub fn background_commandline(config: &AutostartConfig) -> Vec<String> {
    let mut commandline = Vec::with_capacity(config.arguments.len() + 1);
    commandline.push(config.executable.to_string_lossy().into_owned());
    commandline.extend(config.arguments.iter().cloned());
    commandline
}

/// handle token of one request, the spec allows letters and digits and underscores only
pub fn request_token(serial: u64) -> String {
    format!("riseonly_autostart_{serial}")
}

/// object path the portal gives the request
/// known before the call so the response subscription exists before the dialog answers
/// unique name without the colon, dots as underscores and then the token
pub fn request_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name.strip_prefix(':').unwrap_or(unique_name).replace('.', "_");
    format!("{REQUEST_PATH_PREFIX}/{sender}/{token}")
}

/// how one request background ended, read from the response code and its autostart result
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BackgroundOutcome {
    Applied,
    Refused,
    NotApplied,
}

/// code 0 is success, 1 cancelled by the user and 2 and up ended another way
/// a success without autostart came from a portal older than version 2
/// which already wrote what it was given
pub fn background_outcome(code: u32, autostart: Option<bool>, desired: bool) -> BackgroundOutcome {
    match (code, autostart) {
        (0, None) => BackgroundOutcome::Applied,
        (0, Some(now)) if now == desired => BackgroundOutcome::Applied,
        (0, Some(_)) => BackgroundOutcome::NotApplied,
        _ => BackgroundOutcome::Refused,
    }
}

impl BackgroundOutcome {
    pub fn is_applied(&self) -> bool {
        matches!(self, Self::Applied)
    }

    /// why the record kept in the sandbox is rolled back, for the log
    pub fn refusal_reason(&self) -> Option<&'static str> {
        match self {
            Self::Applied => None,
            Self::Refused => Some("the background portal refused the request"),
            Self::NotApplied => Some("the background portal left launch at login unchanged"),
        }
    }
}
