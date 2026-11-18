use super::device_info::{HardwareModel, OsDetails, SystemVersion};

pub const OS_RELEASE_PATHS: [&str; 2] = ["/etc/os-release", "/usr/lib/os-release"];

pub const DMI_SYS_VENDOR: &str = "/sys/class/dmi/id/sys_vendor";
pub const DMI_PRODUCT_NAME: &str = "/sys/class/dmi/id/product_name";
pub const DMI_PRODUCT_VERSION: &str = "/sys/class/dmi/id/product_version";
pub const DEVICETREE_MODEL: &str = "/sys/firmware/devicetree/base/model";
pub const KERNEL_RELEASE: &str = "/proc/sys/kernel/osrelease";
pub const FLATPAK_INFO: &str = "/.flatpak-info";

/// value of key in an os-release file, last assignment wins
/// comments and blank lines are skipped and a quoted value loses its quotes and backslash escapes
pub fn os_release_value(text: &str, key: &str) -> Option<String> {
    let mut found = None;
    for line in text.lines() {
        let line = line.trim_start_matches([' ', '\t']);
        if line.starts_with('#') {
            continue;
        }
        let Some((name, raw)) = line.split_once('=') else {
            continue;
        };
        if name == key {
            found = Some(unquote(raw.trim_end_matches([' ', '\t', '\r'])));
        }
    }
    found
}

fn unquote(raw: &str) -> String {
    let (body, escapes) = match raw.as_bytes() {
        [b'"', .., b'"'] => (&raw[1..raw.len() - 1], true),
        [b'\'', .., b'\''] => (&raw[1..raw.len() - 1], false),
        _ => (raw, true),
    };
    if !escapes {
        return body.to_owned();
    }
    let mut value = String::with_capacity(body.len());
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            if let Some(next) = characters.next() {
                value.push(next);
            }
        } else {
            value.push(character);
        }
    }
    value
}

/// system version an os-release file names, NAME plus VERSION_ID or BUILD_ID on a rolling distro
/// no version at all is an empty string which the hub answers as unknown
pub fn system_version_from_os_release(text: &str) -> SystemVersion {
    SystemVersion {
        name: os_release_value(text, "NAME"),
        version: os_release_value(text, "VERSION_ID")
            .or_else(|| os_release_value(text, "BUILD_ID"))
            .unwrap_or_default(),
    }
}

/// raw dmi and devicetree values as read, trailing newline or NUL included
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LinuxHardwareFiles {
    pub sys_vendor: Option<String>,
    pub product_name: Option<String>,
    pub product_version: Option<String>,
    pub devicetree_model: Option<String>,
}

/// hardware model the hub cleans
/// lenovo keeps machine type in product_name and the human name in product_version
/// other vendors put junk there so only lenovo gets a name, no dmi falls back to devicetree model
pub fn hardware_model_from_files(files: LinuxHardwareFiles) -> Option<HardwareModel> {
    let is_lenovo = files
        .sys_vendor
        .as_deref()
        .is_some_and(|vendor| vendor.split_whitespace().next().is_some_and(|word| word.eq_ignore_ascii_case("lenovo")));
    let model = HardwareModel {
        identifier: files.product_name.or(files.devicetree_model),
        manufacturer: files.sys_vendor,
        name: if is_lenovo { files.product_version } else { None },
    };
    (model.identifier.is_some() || model.manufacturer.is_some()).then_some(model)
}

/// distro kernel session desktop and packaging of a linux host
/// variable reads one environment variable, flatpak says the app runs in its sandbox
pub fn linux_os_details(
    os_release: Option<&str>,
    kernel: Option<&str>,
    variable: impl Fn(&str) -> Option<String>,
    flatpak: bool,
) -> OsDetails {
    let present = |name: &str| variable(name).map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
    let name = os_release.and_then(|text| {
        os_release_value(text, "PRETTY_NAME").filter(|pretty| !pretty.trim().is_empty()).or_else(|| {
            let system = system_version_from_os_release(text);
            let joined = [system.name, Some(system.version)]
                .into_iter()
                .flatten()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            (!joined.is_empty()).then_some(joined)
        })
    });
    let session = present("XDG_SESSION_TYPE")
        .map(|kind| match kind.to_ascii_lowercase().as_str() {
            "wayland" => "Wayland".to_owned(),
            "x11" => "X11".to_owned(),
            _ => kind,
        })
        .or_else(|| present("WAYLAND_DISPLAY").map(|_| "Wayland".to_owned()))
        .or_else(|| present("DISPLAY").map(|_| "X11".to_owned()));
    let packaging = if flatpak || present("FLATPAK_ID").is_some() {
        Some("Flatpak".to_owned())
    } else if present("APPIMAGE").is_some() {
        Some("AppImage".to_owned())
    } else if present("SNAP").is_some() {
        Some("Snap".to_owned())
    } else {
        None
    };
    OsDetails {
        name,
        kernel: kernel.map(|release| release.trim().to_owned()).filter(|release| !release.is_empty()),
        session,
        desktop: present("XDG_CURRENT_DESKTOP"),
        packaging,
        ..OsDetails::default()
    }
}
