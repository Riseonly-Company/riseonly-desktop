use std::ffi::{c_char, c_int, c_void, CStr};

use super::device_info::*;

unsafe extern "C" {
    fn sysctlbyname(
        name: *const c_char,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> c_int;
}

pub(super) struct NativeDeviceInfo;

impl DeviceInfoSource for NativeDeviceInfo {
    fn os_details(&self) -> OsDetails {
        OsDetails { build: sysctl_string(c"kern.osversion").ok(), ..OsDetails::default() }
    }

    fn hardware_model(&self) -> Result<HardwareModel, DeviceInfoUnavailable> {
        let identifier = sysctl_string(c"hw.model").map_err(|detail| {
            DeviceInfoUnavailable::SystemRefused {
                what: DeviceReading::HardwareModel,
                detail,
            }
        })?;
        Ok(HardwareModel {
            identifier: Some(identifier),
            manufacturer: None,
            name: None,
        })
    }

    fn system_version(&self) -> Result<SystemVersion, DeviceInfoUnavailable> {
        let version = crate::materials::macos_version().ok_or_else(|| {
            DeviceInfoUnavailable::SystemRefused {
                what: DeviceReading::SystemVersion,
                detail: "NSProcessInfo.operatingSystemVersion answered major 0".to_owned(),
            }
        })?;
        Ok(SystemVersion {
            name: None,
            version: dotted_version(
                u64::from(version.major),
                u64::from(version.minor),
                u64::from(version.patch),
            ),
        })
    }
}

fn sysctl_string(name: &CStr) -> Result<String, String> {
    let mut length: usize = 0;
    let status = unsafe {
        sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return Err(format!(
            "sysctlbyname {name:?} size query: {}",
            std::io::Error::last_os_error()
        ));
    }
    if length == 0 {
        return Err(format!("sysctlbyname {name:?} answered an empty value"));
    }
    let mut buffer = vec![0_u8; length];
    let status = unsafe {
        sysctlbyname(
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return Err(format!(
            "sysctlbyname {name:?}: {}",
            std::io::Error::last_os_error()
        ));
    }
    buffer.truncate(length);
    let text = CStr::from_bytes_until_nul(&buffer)
        .map_err(|error| format!("sysctlbyname {name:?} answered no C string: {error}"))?;
    Ok(text.to_string_lossy().into_owned())
}
