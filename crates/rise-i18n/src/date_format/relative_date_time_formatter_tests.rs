use rise_core::foundation::foundation_calendar::{Calendar, TimeZone};
use rise_core::foundation::foundation_date::Date;

use super::relative_date_time_formatter::{RelativeDateTimeFormatter, UnitsStyle};

const SAMPLE_MS: i64 = 1_793_714_709_123;

fn formatter(locale: &str, style: UnitsStyle) -> RelativeDateTimeFormatter {
    let mut formatter = RelativeDateTimeFormatter::new();
    formatter.set_locale(locale);
    formatter.set_units_style(style);
    formatter.set_calendar(Calendar { time_zone: TimeZone::gmt(), ..Calendar::gregorian() });
    formatter
}

fn sample() -> Date {
    Date::from_ms(SAMPLE_MS)
}

fn shifted(seconds: f64) -> Date {
    sample().adding_time_interval(seconds)
}

#[test]
fn english_full_picks_the_largest_non_zero_unit() {
    let english = formatter("en", UnitsStyle::Full);
    let now = sample();
    let cases = [
        (300.0, "in 5 minutes"),
        (-300.0, "5 minutes ago"),
        (-7_200.0, "2 hours ago"),
        (86_400.0, "in 1 day"),
        (10.0 * 86_400.0, "in 1 week"),
        (-45.0 * 86_400.0, "1 month ago"),
        (400.0 * 86_400.0, "in 1 year"),
        (0.0, "in 0 seconds"),
    ];
    for (seconds, expected) in cases {
        assert_eq!(english.localized_string_for(&shifted(seconds), &now), expected, "{seconds}");
    }
}

#[test]
fn russian_full_matches_foundation() {
    let russian = formatter("ru", UnitsStyle::Full);
    let now = sample();
    assert_eq!(
        russian.localized_string_for(&shifted(300.0), &now),
        "\u{447}\u{435}\u{440}\u{435}\u{437} 5 \u{43c}\u{438}\u{43d}\u{443}\u{442}"
    );
    assert_eq!(
        russian.localized_string_for(&shifted(-300.0), &now),
        "5 \u{43c}\u{438}\u{43d}\u{443}\u{442} \u{43d}\u{430}\u{437}\u{430}\u{434}"
    );
    assert_eq!(
        russian.localized_string_for(&shifted(86_400.0), &now),
        "\u{447}\u{435}\u{440}\u{435}\u{437} 1 \u{434}\u{435}\u{43d}\u{44c}"
    );
    assert_eq!(
        russian.localized_string_for(&shifted(-7_200.0), &now),
        "2 \u{447}\u{430}\u{441}\u{430} \u{43d}\u{430}\u{437}\u{430}\u{434}"
    );
}

#[test]
fn english_short_uses_the_cldr_short_form() {
    let english = formatter("en", UnitsStyle::Short);
    assert_eq!(english.localized_string_for(&shifted(300.0), &sample()), "in 5 min.");
}

#[test]
fn default_style_is_full() {
    assert_eq!(UnitsStyle::default(), UnitsStyle::Full);
}
