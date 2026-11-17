use windows::core::{w, PCWSTR};
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

use super::device_info::*;

const BIOS_KEY: PCWSTR = w!("HARDWARE\\DESCRIPTION\\System\\BIOS");

pub(super) struct NativeDeviceInfo;

impl DeviceInfoSource for NativeDeviceInfo {
    fn hardware_model(&self) -> Result<HardwareModel, DeviceInfoUnavailable> {
        let refused = |detail: String| DeviceInfoUnavailable::SystemRefused {
            what: DeviceReading::HardwareModel,
            detail,
        };
        let identifier = bios_string(w!("SystemProductName")).map_err(refused)?;
        let manufacturer = bios_string(w!("SystemManufacturer")).map_err(refused)?;
        if identifier.is_none() && manufacturer.is_none() {
            return Err(refused(
                "HKLM\\HARDWARE\\DESCRIPTION\\System\\BIOS has neither SystemProductName nor \
                 SystemManufacturer"
                    .to_owned(),
            ));
        }
        Ok(HardwareModel {
            identifier,
            manufacturer,
            name: None,
        })
    }

    fn system_version(&self) -> Result<SystemVersion, DeviceInfoUnavailable> {
        let mut info = OSVERSIONINFOW {
            dwOSVersionInfoSize: u32::try_from(std::mem::size_of::<OSVERSIONINFOW>())
                .unwrap_or(u32::MAX),
            ..Default::default()
        };
        let status = unsafe { RtlGetVersion(&mut info) };
        if status.0 < 0 || info.dwMajorVersion == 0 {
            return Err(DeviceInfoUnavailable::SystemRefused {
                what: DeviceReading::SystemVersion,
                detail: format!(
                    "RtlGetVersion answered status {:#010x}, major {}",
                    status.0, info.dwMajorVersion
                ),
            });
        }
        Ok(SystemVersion {
            name: None,
            version: dotted_version(
                u64::from(info.dwMajorVersion),
                u64::from(info.dwMinorVersion),
                u64::from(info.dwBuildNumber),
            ),
        })
    }
}

/// one reg_sz value under the bios key
/// ok none when firmware left it out and err when the registry refused the read
fn bios_string(value: PCWSTR) -> Result<Option<String>, String> {
    let mut size: u32 = 0;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            BIOS_KEY,
            value,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(format!("RegGetValueW size query failed with {}", status.0));
    }
    let units = usize::try_from(size).unwrap_or(0) / 2 + 1;
    let mut buffer = vec![0_u16; units];
    let mut size = u32::try_from(units * 2).unwrap_or(u32::MAX);
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            BIOS_KEY,
            value,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(format!("RegGetValueW failed with {}", status.0));
    }
    let written = (usize::try_from(size).unwrap_or(0) / 2).min(buffer.len());
    let text = &buffer[..written];
    let end = text.iter().position(|unit| *unit == 0).unwrap_or(text.len());
    Ok(Some(String::from_utf16_lossy(&text[..end])))
}
