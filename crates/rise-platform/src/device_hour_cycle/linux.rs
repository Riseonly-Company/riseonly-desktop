use super::device_hour_cycle::*;
use crate::host_os::HostOs;

use std::ffi::{c_char, c_int, c_void, CStr};

/// locale_t, an opaque handle
type LocaleHandle = *mut c_void;

const LC_TIME_MASK: c_int = 1 << 2;
const T_FMT: c_int = (2 << 16) | 42;

unsafe extern "C" {
    fn newlocale(category_mask: c_int, locale: *const c_char, base: LocaleHandle) -> LocaleHandle;
    fn nl_langinfo_l(item: c_int, locale: LocaleHandle) -> *const c_char;
    fn freelocale(locale: LocaleHandle);
}

pub(super) fn native_hour_pattern(host: HostOs) -> Result<HourPattern, HourCycleUnavailable> {
    match time_format_of(c"") {
        Some(format) => Ok(HourPattern::PosixTimeFormat(format)),
        None => Err(HourCycleUnavailable::NoPattern { host }),
    }
}

fn time_format_of(locale_name: &CStr) -> Option<String> {
    let locale = unsafe { newlocale(LC_TIME_MASK, locale_name.as_ptr(), std::ptr::null_mut()) };
    if locale.is_null() {
        return None;
    }

    let format = unsafe {
        let pointer = nl_langinfo_l(T_FMT, locale);
        let format = if pointer.is_null() {
            None
        } else {
            Some(CStr::from_ptr(pointer).to_string_lossy().into_owned())
        };
        freelocale(locale);
        format
    };

    format.filter(|format| !format.is_empty())
}
