use std::sync::OnceLock;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use thiserror::Error;

use crate::host_os::HostOs;

pub const UNKNOWN_VERSION: &str = "unknown";

/// which reading an os layer couldnt give
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DeviceReading {
    HardwareModel,
    SystemVersion,
}

impl std::fmt::Display for DeviceReading {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::HardwareModel => "hardware model",
            Self::SystemVersion => "system version",
        };
        formatter.write_str(text)
    }
}

/// why a reading is missing, device info still answers and see resolve
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum DeviceInfoUnavailable {
    #[error("{what} is not written for {host:?}")]
    BranchNotWritten { what: DeviceReading, host: HostOs },
    #[error("the system refused to report {what}: {detail}")]
    SystemRefused { what: DeviceReading, detail: String },
}

/// raw hardware facts an os layer read, before any cleaning
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct HardwareModel {
    pub identifier: Option<String>,
    pub manufacturer: Option<String>,
    pub name: Option<String>,
}

/// raw system version an os layer read
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SystemVersion {
    pub name: Option<String>,
    pub version: String,
}

pub trait DeviceInfoSource {
    fn hardware_model(&self) -> Result<HardwareModel, DeviceInfoUnavailable> {
        Err(DeviceInfoUnavailable::BranchNotWritten {
            what: DeviceReading::HardwareModel,
            host: HostOs::current(),
        })
    }

    fn system_version(&self) -> Result<SystemVersion, DeviceInfoUnavailable> {
        Err(DeviceInfoUnavailable::BranchNotWritten {
            what: DeviceReading::SystemVersion,
            host: HostOs::current(),
        })
    }
}

/// device info as callers read it, resolved for the desktop
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceInfo {
    pub machine_identifier: Option<String>,
    pub display_name: String,
    pub os_name: String,
    pub system_version: String,
    pub unavailable: Vec<DeviceInfoUnavailable>,
}

impl DeviceInfo {
    /// builds the answer from what the os layer read, a failed reading falls back and is kept
    pub fn resolve(
        host: HostOs,
        model: Result<HardwareModel, DeviceInfoUnavailable>,
        system: Result<SystemVersion, DeviceInfoUnavailable>,
    ) -> Self {
        let mut unavailable = Vec::new();
        let model = model.unwrap_or_else(|reason| {
            unavailable.push(reason);
            HardwareModel::default()
        });
        let (os_name, system_version) = match system {
            Ok(system) => (
                system
                    .name
                    .as_deref()
                    .and_then(clean_firmware_text)
                    .unwrap_or_else(|| os_name(host).to_owned()),
                clean_firmware_text(&system.version).unwrap_or_else(|| UNKNOWN_VERSION.to_owned()),
            ),
            Err(reason) => {
                unavailable.push(reason);
                (os_name(host).to_owned(), UNKNOWN_VERSION.to_owned())
            }
        };
        let machine_identifier = model.identifier.as_deref().and_then(clean_firmware_text);
        Self {
            display_name: display_name(host, &model),
            machine_identifier,
            os_name,
            system_version,
            unavailable,
        }
    }

    /// the running machine, read once because none of this changes while the process lives
    pub fn current() -> &'static Self {
        static CACHED: OnceLock<DeviceInfo> = OnceLock::new();
        CACHED.get_or_init(|| {
            let source = super::os::NativeDeviceInfo;
            Self::resolve(
                HostOs::current(),
                source.hardware_model(),
                source.system_version(),
            )
        })
    }

    /// eg macOS 15.1, the system half of the user agent
    pub fn system_label(&self) -> String {
        format!("{} {}", self.os_name, self.system_version)
    }
}

pub const fn os_name(host: HostOs) -> &'static str {
    match host {
        HostOs::MacOs => "macOS",
        HostOs::Windows => "Windows",
        HostOs::Linux => "Linux",
    }
}

/// version spelling like 15.1 or 15.1.2, a zero patch is left out
pub fn dotted_version(major: u64, minor: u64, patch: u64) -> String {
    if patch == 0 {
        format!("{major}.{minor}")
    } else {
        format!("{major}.{minor}.{patch}")
    }
}

const FIRMWARE_PLACEHOLDERS: [&str; 14] = [
    "to be filled by o.e.m.",
    "to be filled by oem",
    "system product name",
    "system manufacturer",
    "system version",
    "default string",
    "not applicable",
    "not specified",
    "none",
    "o.e.m.",
    "oem",
    "invalid",
    "unknown",
    "type1productconfigid",
];

/// firmware or system string as the server should see it, whitespace and NULs stripped
/// none for empty or a known placeholder
pub fn clean_firmware_text(raw: &str) -> Option<String> {
    let without_nul: String = raw.chars().filter(|character| *character != '\0').collect();
    let text = trimming_whitespaces_and_newlines(&without_nul);
    if text.is_empty()
        || FIRMWARE_PLACEHOLDERS
            .iter()
            .any(|placeholder| text.eq_ignore_ascii_case(placeholder))
    {
        return None;
    }
    Some(text.to_owned())
}

/// product family a mac model identifier names, the letters before the generation number
/// apple silicon since 2022 reports a bare Mac family (eg Mac14,2)
pub fn mac_family(identifier: &str) -> &'static str {
    let letters: &str = identifier
        .split(|character: char| character.is_ascii_digit())
        .next()
        .unwrap_or_default();
    match letters {
        "MacBookPro" => "MacBook Pro",
        "MacBookAir" => "MacBook Air",
        "MacBook" => "MacBook",
        "iMacPro" => "iMac Pro",
        "iMac" => "iMac",
        "Macmini" => "Mac mini",
        "MacPro" => "Mac Pro",
        _ => "Mac",
    }
}

pub const MAC_MODEL_MAP: &[(&str, &str)] = &[
    ("MacBookPro15,1", "MacBook Pro (15-inch, 2018)"),
    ("MacBookPro15,2", "MacBook Pro (13-inch, 2018)"),
    ("MacBookPro15,3", "MacBook Pro (15-inch, 2019)"),
    ("MacBookPro15,4", "MacBook Pro (13-inch, 2019)"),
    ("MacBookPro16,1", "MacBook Pro (16-inch, 2019)"),
    ("MacBookPro16,4", "MacBook Pro (16-inch, 2019)"),
    ("MacBookPro16,2", "MacBook Pro (13-inch, 2020)"),
    ("MacBookPro16,3", "MacBook Pro (13-inch, 2020)"),
    ("MacBookPro17,1", "MacBook Pro (13-inch, M1, 2020)"),
    ("MacBookPro18,1", "MacBook Pro (16-inch, 2021)"),
    ("MacBookPro18,2", "MacBook Pro (16-inch, 2021)"),
    ("MacBookPro18,3", "MacBook Pro (14-inch, 2021)"),
    ("MacBookPro18,4", "MacBook Pro (14-inch, 2021)"),
    ("Mac14,7", "MacBook Pro (13-inch, M2, 2022)"),
    ("Mac14,5", "MacBook Pro (14-inch, 2023)"),
    ("Mac14,9", "MacBook Pro (14-inch, 2023)"),
    ("Mac14,6", "MacBook Pro (16-inch, 2023)"),
    ("Mac14,10", "MacBook Pro (16-inch, 2023)"),
    ("Mac15,3", "MacBook Pro (14-inch, M3, Nov 2023)"),
    ("Mac15,6", "MacBook Pro (14-inch, Nov 2023)"),
    ("Mac15,8", "MacBook Pro (14-inch, Nov 2023)"),
    ("Mac15,10", "MacBook Pro (14-inch, Nov 2023)"),
    ("Mac15,7", "MacBook Pro (16-inch, Nov 2023)"),
    ("Mac15,9", "MacBook Pro (16-inch, Nov 2023)"),
    ("Mac15,11", "MacBook Pro (16-inch, Nov 2023)"),
    ("Mac16,1", "MacBook Pro (14-inch, M4, 2024)"),
    ("Mac16,6", "MacBook Pro (14-inch, 2024)"),
    ("Mac16,8", "MacBook Pro (14-inch, 2024)"),
    ("Mac16,5", "MacBook Pro (16-inch, 2024)"),
    ("Mac16,7", "MacBook Pro (16-inch, 2024)"),
    ("MacBookAir8,1", "MacBook Air (Retina, 13-inch, 2018)"),
    ("MacBookAir8,2", "MacBook Air (Retina, 13-inch, 2019)"),
    ("MacBookAir9,1", "MacBook Air (Retina, 13-inch, 2020)"),
    ("MacBookAir10,1", "MacBook Air (M1, 2020)"),
    ("Mac14,2", "MacBook Air (M2, 2022)"),
    ("Mac14,15", "MacBook Air (15-inch, M2, 2023)"),
    ("Mac15,12", "MacBook Air (13-inch, M3, 2024)"),
    ("Mac15,13", "MacBook Air (15-inch, M3, 2024)"),
    ("Mac16,12", "MacBook Air (13-inch, M4, 2025)"),
    ("Mac16,13", "MacBook Air (15-inch, M4, 2025)"),
    ("iMac19,1", "iMac (Retina 5K, 27-inch, 2019)"),
    ("iMac19,2", "iMac (Retina 4K, 21.5-inch, 2019)"),
    ("iMac20,1", "iMac (Retina 5K, 27-inch, 2020)"),
    ("iMac20,2", "iMac (Retina 5K, 27-inch, 2020)"),
    ("iMac21,1", "iMac (24-inch, M1, 2021)"),
    ("iMac21,2", "iMac (24-inch, M1, 2021)"),
    ("Mac15,4", "iMac (24-inch, 2023)"),
    ("Mac15,5", "iMac (24-inch, 2023)"),
    ("Mac16,2", "iMac (24-inch, 2024)"),
    ("Mac16,3", "iMac (24-inch, 2024)"),
    ("iMacPro1,1", "iMac Pro (2017)"),
    ("Macmini8,1", "Mac mini (2018)"),
    ("Macmini9,1", "Mac mini (M1, 2020)"),
    ("Mac14,3", "Mac mini (2023)"),
    ("Mac14,12", "Mac mini (2023)"),
    ("Mac16,10", "Mac mini (2024)"),
    ("Mac16,11", "Mac mini (2024)"),
    ("Mac13,1", "Mac Studio (2022)"),
    ("Mac13,2", "Mac Studio (2022)"),
    ("Mac14,13", "Mac Studio (2023)"),
    ("Mac14,14", "Mac Studio (2023)"),
    ("Mac16,9", "Mac Studio (2025)"),
    ("Mac15,14", "Mac Studio (2025)"),
    ("MacPro7,1", "Mac Pro (2019)"),
    ("Mac14,8", "Mac Pro (2023)"),
];

/// marketing name of a mac, the table name or else the family read from the identifier
pub fn mac_marketing_name(identifier: &str) -> &'static str {
    MAC_MODEL_MAP
        .iter()
        .find(|(known, _)| *known == identifier)
        .map_or_else(|| mac_family(identifier), |(_, name)| *name)
}

/// display name, the marketing name with Apple in front unless it already starts so
pub fn with_apple_prefix(name: &str) -> String {
    if name.to_ascii_lowercase().starts_with("apple") {
        name.to_owned()
    } else {
        format!("Apple {name}")
    }
}

/// what the session list shows for this machine
pub fn display_name(host: HostOs, model: &HardwareModel) -> String {
    let identifier = model.identifier.as_deref().and_then(clean_firmware_text);
    if host == HostOs::MacOs {
        return with_apple_prefix(identifier.as_deref().map_or("Mac", mac_marketing_name));
    }
    let manufacturer = model.manufacturer.as_deref().and_then(clean_firmware_text);
    let product = model
        .name
        .as_deref()
        .and_then(clean_firmware_text)
        .or(identifier);
    match (manufacturer, product) {
        (Some(maker), Some(product)) if starts_with_ignoring_case(&product, &maker) => product,
        (Some(maker), Some(product)) => format!("{maker} {product}"),
        (None, Some(product)) => product,
        (Some(maker), None) => format!("{maker} {}", generic_machine(host)),
        (None, None) => generic_machine(host).to_owned(),
    }
}

/// name of a machine nothing is known about
pub const fn generic_machine(host: HostOs) -> &'static str {
    match host {
        HostOs::MacOs => "Mac",
        HostOs::Windows => "Windows PC",
        HostOs::Linux => "Linux PC",
    }
}

fn starts_with_ignoring_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.is_char_boundary(prefix.len())
        && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}
