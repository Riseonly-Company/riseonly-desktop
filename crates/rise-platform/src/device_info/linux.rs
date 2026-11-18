use std::io::ErrorKind;

use super::device_info::*;
use super::os_release::*;

const FLATPAK_HOST_OS_RELEASE: &str = "/run/host/os-release";

pub(super) struct NativeDeviceInfo;

impl DeviceInfoSource for NativeDeviceInfo {
    fn os_details(&self) -> OsDetails {
        let os_release = std::iter::once(FLATPAK_HOST_OS_RELEASE)
            .chain(OS_RELEASE_PATHS)
            .find_map(|path| std::fs::read_to_string(path).ok());
        let kernel = std::fs::read_to_string(KERNEL_RELEASE).ok();
        linux_os_details(
            os_release.as_deref(),
            kernel.as_deref(),
            |name| std::env::var(name).ok(),
            std::path::Path::new(FLATPAK_INFO).exists(),
        )
    }

    fn hardware_model(&self) -> Result<HardwareModel, DeviceInfoUnavailable> {
        let files = LinuxHardwareFiles {
            sys_vendor: read_optional(DMI_SYS_VENDOR),
            product_name: read_optional(DMI_PRODUCT_NAME),
            product_version: read_optional(DMI_PRODUCT_VERSION),
            devicetree_model: read_optional(DEVICETREE_MODEL),
        };
        hardware_model_from_files(files).ok_or_else(|| DeviceInfoUnavailable::SystemRefused {
            what: DeviceReading::HardwareModel,
            detail: format!("neither {DMI_PRODUCT_NAME}, {DMI_SYS_VENDOR} nor {DEVICETREE_MODEL} is readable"),
        })
    }

    fn system_version(&self) -> Result<SystemVersion, DeviceInfoUnavailable> {
        let mut errors = Vec::new();
        for path in std::iter::once(FLATPAK_HOST_OS_RELEASE).chain(OS_RELEASE_PATHS) {
            match std::fs::read_to_string(path) {
                Ok(text) => return Ok(system_version_from_os_release(&text)),
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => errors.push(format!("{path}: {error}")),
            }
        }
        let detail = if errors.is_empty() {
            format!("no os-release file ({})", OS_RELEASE_PATHS.join(", "))
        } else {
            errors.join("; ")
        };
        Err(DeviceInfoUnavailable::SystemRefused {
            what: DeviceReading::SystemVersion,
            detail,
        })
    }
}

/// a sysfs attribute, none when the machine has none or the read fails
/// a non utf8 dmi value is useless for the server anyway
fn read_optional(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}
