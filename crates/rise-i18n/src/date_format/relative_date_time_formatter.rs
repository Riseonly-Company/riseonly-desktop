use std::cell::RefCell;
use std::collections::HashMap;

use fixed_decimal::Decimal;
use icu_experimental::relativetime::{
    RelativeTimeFormatter, RelativeTimeFormatterOptions, RelativeTimeFormatterPreferences,
};
use icu_locale_core::Locale as IcuLocale;
use writeable::Writeable;

use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::Date;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum UnitsStyle {
    #[default]
    Full,
    SpellOut,
    Short,
    Abbreviated,
}

/// the unit picked, the largest one with a non zero count
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RelativeUnit {
    Year,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
}

#[derive(Clone, Debug)]
pub struct RelativeDateTimeFormatter {
    locale: String,
    units_style: UnitsStyle,
    calendar: Calendar,
}

impl Default for RelativeDateTimeFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl RelativeDateTimeFormatter {
    /// new formatter, full units and current calendar
    /// app language as locale since desktop has no separate region locale
    pub fn new() -> Self {
        Self {
            locale: crate::i18n::I18n::shared().code().to_owned(),
            units_style: UnitsStyle::Full,
            calendar: Calendar::current(),
        }
    }

    pub fn set_locale(&mut self, identifier: impl Into<String>) {
        self.locale = identifier.into();
    }

    pub fn set_units_style(&mut self, style: UnitsStyle) {
        self.units_style = style;
    }

    pub fn set_calendar(&mut self, calendar: Calendar) {
        self.calendar = calendar;
    }

    /// numeric text like in 5 minutes or 5 minutes ago, in the largest unit with a non zero count
    /// in 0 seconds when the dates are within a second
    pub fn localized_string_for(&self, date: &Date, relative_to: &Date) -> String {
        let (unit, value) = self.largest_unit(date, relative_to);
        format_relative(&self.locale, self.units_style, unit, value)
    }

    fn largest_unit(&self, date: &Date, relative_to: &Date) -> (RelativeUnit, i64) {
        let components = self.calendar.date_components_between(
            &[
                CalendarComponent::Year,
                CalendarComponent::Month,
                CalendarComponent::Day,
                CalendarComponent::Hour,
                CalendarComponent::Minute,
                CalendarComponent::Second,
            ],
            relative_to,
            date,
        );
        let days = components.day.unwrap_or(0);
        let weeks = days / 7;
        let ordered = [
            (RelativeUnit::Year, components.year.unwrap_or(0)),
            (RelativeUnit::Month, components.month.unwrap_or(0)),
            (RelativeUnit::Week, weeks),
            (RelativeUnit::Day, days),
            (RelativeUnit::Hour, components.hour.unwrap_or(0)),
            (RelativeUnit::Minute, components.minute.unwrap_or(0)),
            (RelativeUnit::Second, components.second.unwrap_or(0)),
        ];
        ordered
            .into_iter()
            .find(|(_, value)| *value != 0)
            .unwrap_or((RelativeUnit::Second, 0))
    }
}

type FormatterKey = (String, UnitsStyle, RelativeUnit);

thread_local! {
    static FORMATTERS: RefCell<HashMap<FormatterKey, Option<RelativeTimeFormatter>>> =
        RefCell::new(HashMap::new());
}

fn format_relative(locale: &str, style: UnitsStyle, unit: RelativeUnit, value: i64) -> String {
    let key = (locale.to_owned(), style, unit);
    FORMATTERS.with(|cell| {
        let mut formatters = cell.borrow_mut();
        let formatter = formatters.entry(key).or_insert_with(|| make_formatter(locale, style, unit));
        formatter
            .as_ref()
            .map(|formatter| formatter.format(Decimal::from(value)).write_to_string().into_owned())
            .unwrap_or_default()
    })
}

fn make_formatter(locale: &str, style: UnitsStyle, unit: RelativeUnit) -> Option<RelativeTimeFormatter> {
    let identifier = locale.replace('_', "-");
    let locale = IcuLocale::try_from_str(&identifier).unwrap_or(IcuLocale::UNKNOWN);
    let preferences = RelativeTimeFormatterPreferences::from(&locale);
    let options = RelativeTimeFormatterOptions::default();
    let result = match (style, unit) {
        (UnitsStyle::Full | UnitsStyle::SpellOut, RelativeUnit::Year) => {
            RelativeTimeFormatter::try_new_long_year(preferences, options)
        }
        (UnitsStyle::Full | UnitsStyle::SpellOut, RelativeUnit::Month) => {
            RelativeTimeFormatter::try_new_long_month(preferences, options)
        }
        (UnitsStyle::Full | UnitsStyle::SpellOut, RelativeUnit::Week) => {
            RelativeTimeFormatter::try_new_long_week(preferences, options)
        }
        (UnitsStyle::Full | UnitsStyle::SpellOut, RelativeUnit::Day) => {
            RelativeTimeFormatter::try_new_long_day(preferences, options)
        }
        (UnitsStyle::Full | UnitsStyle::SpellOut, RelativeUnit::Hour) => {
            RelativeTimeFormatter::try_new_long_hour(preferences, options)
        }
        (UnitsStyle::Full | UnitsStyle::SpellOut, RelativeUnit::Minute) => {
            RelativeTimeFormatter::try_new_long_minute(preferences, options)
        }
        (UnitsStyle::Full | UnitsStyle::SpellOut, RelativeUnit::Second) => {
            RelativeTimeFormatter::try_new_long_second(preferences, options)
        }
        (UnitsStyle::Short, RelativeUnit::Year) => RelativeTimeFormatter::try_new_short_year(preferences, options),
        (UnitsStyle::Short, RelativeUnit::Month) => RelativeTimeFormatter::try_new_short_month(preferences, options),
        (UnitsStyle::Short, RelativeUnit::Week) => RelativeTimeFormatter::try_new_short_week(preferences, options),
        (UnitsStyle::Short, RelativeUnit::Day) => RelativeTimeFormatter::try_new_short_day(preferences, options),
        (UnitsStyle::Short, RelativeUnit::Hour) => RelativeTimeFormatter::try_new_short_hour(preferences, options),
        (UnitsStyle::Short, RelativeUnit::Minute) => {
            RelativeTimeFormatter::try_new_short_minute(preferences, options)
        }
        (UnitsStyle::Short, RelativeUnit::Second) => {
            RelativeTimeFormatter::try_new_short_second(preferences, options)
        }
        (UnitsStyle::Abbreviated, RelativeUnit::Year) => {
            RelativeTimeFormatter::try_new_narrow_year(preferences, options)
        }
        (UnitsStyle::Abbreviated, RelativeUnit::Month) => {
            RelativeTimeFormatter::try_new_narrow_month(preferences, options)
        }
        (UnitsStyle::Abbreviated, RelativeUnit::Week) => {
            RelativeTimeFormatter::try_new_narrow_week(preferences, options)
        }
        (UnitsStyle::Abbreviated, RelativeUnit::Day) => RelativeTimeFormatter::try_new_narrow_day(preferences, options),
        (UnitsStyle::Abbreviated, RelativeUnit::Hour) => {
            RelativeTimeFormatter::try_new_narrow_hour(preferences, options)
        }
        (UnitsStyle::Abbreviated, RelativeUnit::Minute) => {
            RelativeTimeFormatter::try_new_narrow_minute(preferences, options)
        }
        (UnitsStyle::Abbreviated, RelativeUnit::Second) => {
            RelativeTimeFormatter::try_new_narrow_second(preferences, options)
        }
    };
    result.ok()
}
