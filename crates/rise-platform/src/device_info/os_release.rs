use super::device_info::{HardwareModel, SystemVersion};

pub const OS_RELEASE_PATHS: [&str; 2] = ["/etc/os-release", "/usr/lib/os-release"];

pub const DMI_SYS_VENDOR: &str = "/sys/class/dmi/id/sys_vendor";
pub const DMI_PRODUCT_NAME: &str = "/sys/class/dmi/id/product_name";
pub const DMI_PRODUCT_VERSION: &str = "/sys/class/dmi/id/product_version";
pub const DEVICETREE_MODEL: &str = "/sys/firmware/devicetree/base/model";

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
