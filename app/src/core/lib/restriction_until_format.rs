use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::tr;

use crate::core::lib::canon_format::RiseFormat;

/// when a moderation restriction ends in plain words, eg today at 15:07 or on 3 March at 9:00
/// the server sends an instant and the day boundary follows the calendar
pub enum RestrictionUntilFormat {}

impl RestrictionUntilFormat {
    /// restriction text with the app calendar, app locale and catalogue lookup
    pub fn text(until_ms: i64, now: &Date) -> String {
        Self::text_with(until_ms, now, &Calendar::current(), &RiseFormat::app_locale(), &tr)
    }

    /// until_ms is epoch milliseconds, zero or below means until a moderator lifts it
    /// translate is replaceable so tests dont depend on locale files
    pub fn text_with(
        until_ms: i64,
        now: &Date,
        calendar: &Calendar,
        locale: &str,
        translate: &dyn Fn(&str) -> String,
    ) -> String {
        if until_ms <= 0 {
            return translate("restriction_until_forever");
        }
        let until = Date::from_ms(until_ms);
        let time = Self::format(&until, "jmm", calendar, locale);

        let key: &str;
        let mut date: Option<String> = None;
        if calendar.is_date_in_same_day_as(&until, now) {
            key = "restriction_until_today";
        } else if calendar
            .date_by_adding(CalendarComponent::Day, 1, now)
            .is_some_and(|tomorrow| calendar.is_date_in_same_day_as(&until, &tomorrow))
        {
            key = "restriction_until_tomorrow";
        } else {
            key = "restriction_until_date";
            let same_year = calendar.is_date_equal_to(&until, now, CalendarComponent::Year);
            date = Some(Self::format(
                &until,
                if same_year { "dMMMM" } else { "dMMMMy" },
                calendar,
                locale,
            ));
        }

        let mut text = translate(key).replace("{time}", &time);
        if let Some(date) = date {
            text = text.replace("{date}", &date);
        }
        text
    }

    fn format(date: &Date, template: &str, calendar: &Calendar, locale: &str) -> String {
        let pattern = DateFormatter::date_format_from_template(template, 0, locale)
            .unwrap_or_else(|| template.to_owned());
        RiseFormat::date_formatter(&pattern, locale, calendar, None).string_from(date)
    }
}
