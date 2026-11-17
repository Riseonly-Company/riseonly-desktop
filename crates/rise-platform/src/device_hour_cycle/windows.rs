use super::device_hour_cycle::*;
use crate::host_os::HostOs;

use windows::core::PCWSTR;
use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_STIMEFORMAT};

pub(super) fn native_hour_pattern(host: HostOs) -> Result<HourPattern, HourCycleUnavailable> {
    let length = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_STIMEFORMAT, None) };
    if length <= 1 {
        return Err(HourCycleUnavailable::NoPattern { host });
    }

    let mut buffer = vec![0u16; length as usize];
    let written = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_STIMEFORMAT, Some(&mut buffer)) };
    if written <= 1 {
        return Err(HourCycleUnavailable::NoPattern { host });
    }

    let format = String::from_utf16_lossy(&buffer[..written as usize - 1]);
    if format.is_empty() {
        return Err(HourCycleUnavailable::NoPattern { host });
    }
    Ok(HourPattern::WindowsTimeFormat(format))
}
