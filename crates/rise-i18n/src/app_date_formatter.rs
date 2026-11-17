use rise_core::foundation::foundation_calendar::{Calendar, TimeZone};

use crate::date_format::date_formatter::DateFormatter;
use crate::language_store::Locale;

pub struct AppDateFormatter;

impl AppDateFormatter {
    /// builds the formatter, none locale takes the app locale
    pub fn make(
        locale: Option<Locale>,
        calendar: Option<Calendar>,
        time_zone: Option<TimeZone>,
    ) -> DateFormatter {
        let mut formatter = DateFormatter::new();
        let locale = locale.unwrap_or_else(Locale::app_locale);
        formatter.set_locale(locale.identifier());
        if let Some(calendar) = calendar {
            formatter.set_calendar(calendar);
        }
        if let Some(time_zone) = time_zone {
            formatter.set_time_zone(time_zone);
        }
        formatter
    }
}
