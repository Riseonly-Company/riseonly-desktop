use std::cell::RefCell;

use rise_core::foundation::foundation_calendar::{Calendar, CalendarIdentifier, TimeZone};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::date_format::date_formatter::{DateFormatter, Style};
use rise_i18n::date_format::relative_date_time_formatter::{RelativeDateTimeFormatter, UnitsStyle};
use rise_i18n::language_store::Locale;
use rise_i18n_catalog::RELATIVE_TIME_BANDS;

use crate::core::lib::canon_format::RiseFormat;

#[derive(Clone, PartialEq, Eq)]
struct PostRowDateTextKey {
    locale: String,
    calendar: CalendarIdentifier,
    time_zone: String,
}

thread_local! {
    static RELATIVE_FORMATTER: RefCell<Option<(PostRowDateTextKey, RelativeDateTimeFormatter)>> = const { RefCell::new(None) };
    static EXACT_FORMATTER: RefCell<Option<(PostRowDateTextKey, DateFormatter)>> = const { RefCell::new(None) };
}

/// relative and exact dates, formatters cached per locale and calendar
pub enum PostRowDateText {}

impl PostRowDateText {
    /// the now label for a stamp under a minute old or ahead of
    /// this device clock, the system formatter would print in 0 sec
    pub fn now_band_label(date: &Date, now: &Date) -> Option<String> {
        let band = RELATIVE_TIME_BANDS.first()?;
        if band.shows_count {
            return None;
        }
        let bound = band.upper_bound_ms?;
        if now.time_interval_since(date) * 1000.0 >= bound as f64 {
            return None;
        }
        Some(RiseFormat::elapsed_since_date(date, now))
    }

    pub fn relative(date: &Date, locale: &Locale, calendar: &Calendar) -> String {
        let now = Date::now();
        if let Some(label) = Self::now_band_label(date, &now) {
            return label;
        }
        let key = PostRowDateTextKey {
            locale: locale.identifier().to_owned(),
            calendar: calendar.identifier,
            time_zone: String::new(),
        };
        RELATIVE_FORMATTER.with_borrow_mut(|cache| {
            if let Some((cached_key, formatter)) = cache.as_ref()
                && *cached_key == key
            {
                return formatter.localized_string_for(date, &now);
            }
            let mut formatter = RelativeDateTimeFormatter::new();
            formatter.set_units_style(UnitsStyle::Short);
            formatter.set_locale(locale.identifier());
            formatter.set_calendar(calendar.clone());
            let text = formatter.localized_string_for(date, &now);
            *cache = Some((key, formatter));
            text
        })
    }

    pub fn exact(date: &Date, locale: &Locale, calendar: &Calendar, time_zone: &TimeZone) -> String {
        let key = PostRowDateTextKey {
            locale: locale.identifier().to_owned(),
            calendar: calendar.identifier,
            time_zone: time_zone.identifier(),
        };
        EXACT_FORMATTER.with_borrow_mut(|cache| {
            if let Some((cached_key, formatter)) = cache.as_ref()
                && *cached_key == key
            {
                return formatter.string_from(date);
            }
            let mut formatter = DateFormatter::new();
            formatter.set_locale(locale.identifier());
            formatter.set_calendar(calendar.clone());
            formatter.set_time_zone(time_zone.clone());
            formatter.set_date_style(Style::Long);
            formatter.set_time_style(Style::Medium);
            let text = formatter.string_from(date);
            *cache = Some((key, formatter));
            text
        })
    }
}
