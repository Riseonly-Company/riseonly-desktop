use super::device_hour_cycle::*;
use crate::host_os::HostOs;

pub(super) fn native_hour_pattern(host: HostOs) -> Result<HourPattern, HourCycleUnavailable> {
    use objc2_foundation::{NSDateFormatter, NSLocale, NSString};

    let locale = NSLocale::currentLocale();
    let template = NSString::from_str("j");
    match NSDateFormatter::dateFormatFromTemplate_options_locale(&template, 0, Some(&locale)) {
        Some(pattern) => Ok(HourPattern::Icu(pattern.to_string())),
        None => Err(HourCycleUnavailable::NoPattern { host }),
    }
}
