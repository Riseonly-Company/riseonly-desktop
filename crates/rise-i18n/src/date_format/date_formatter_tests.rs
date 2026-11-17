use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::{Date, RiseClock, TestClock};

use super::date_format_pattern::{PatternToken, tokenize};
use super::date_formatter::{DateFormatter, Style};

const SAMPLE_MS: i64 = 1_793_714_709_123;

fn formatter(locale: &str, pattern: &str) -> DateFormatter {
    let mut formatter = DateFormatter::new();
    formatter.set_locale(locale);
    formatter.set_time_zone(TimeZone::gmt());
    formatter.set_date_format(pattern);
    formatter
}

fn sample() -> Date {
    Date::from_ms(SAMPLE_MS)
}

#[test]
fn numeric_patterns_match_foundation() {
    let cases = [
        ("yyyy-MM-dd", "2026-11-03"),
        ("HH:mm:ss.SSS", "14:05:09.123"),
        ("HH:mm", "14:05"),
        ("HH:mm:ss", "14:05:09"),
        ("yyyy-MM-dd HH:mm:ss.SSS", "2026-11-03 14:05:09.123"),
        ("dd.MM.yyyy", "03.11.2026"),
        ("yyyyMMdd-HHmmss", "20261103-140509"),
        ("yyyy-MM-dd'T'HH:mm:ss", "2026-11-03T14:05:09"),
        ("yyyy:MM:dd HH:mm:ss", "2026:11:03 14:05:09"),
    ];
    for (pattern, expected) in cases {
        assert_eq!(formatter("en_US_POSIX", pattern).string_from(&sample()), expected, "{pattern}");
    }
}

#[test]
fn month_names_follow_locale_in_english() {
    assert_eq!(formatter("en", "d MMM").string_from(&sample()), "3 Nov");
    assert_eq!(formatter("en", "d MMM yyyy").string_from(&sample()), "3 Nov 2026");
    assert_eq!(formatter("en", "d MMMM yyyy").string_from(&sample()), "3 November 2026");
    assert_eq!(formatter("en", "d MMM yyyy, HH:mm").string_from(&sample()), "3 Nov 2026, 14:05");
    assert_eq!(formatter("en", "LLLL").string_from(&sample()), "November");
}

#[test]
fn month_names_follow_locale_in_russian() {
    assert_eq!(formatter("ru", "d MMM yyyy").string_from(&sample()), "3 \u{43d}\u{43e}\u{44f}\u{431}. 2026");
    assert_eq!(
        formatter("ru", "d MMMM yyyy").string_from(&sample()),
        "3 \u{43d}\u{43e}\u{44f}\u{431}\u{440}\u{44f} 2026"
    );
    assert_eq!(formatter("ru", "LLLL").string_from(&sample()), "\u{43d}\u{43e}\u{44f}\u{431}\u{440}\u{44c}");
}

#[test]
fn quoted_literals_pass_through() {
    let tokens = tokenize("d MMMM 'at' HH:mm");
    assert!(tokens.contains(&PatternToken::Literal(" at ".to_owned())));
    assert_eq!(formatter("en", "d MMMM 'at' HH:mm").string_from(&sample()), "3 November at 14:05");
    assert_eq!(formatter("en", "HH 'o''clock'").string_from(&sample()), "14 o'clock");
}

#[test]
fn styles_match_foundation() {
    let mut english = formatter("en", "");
    english.set_date_style(Style::Medium);
    english.set_time_style(Style::None);
    assert_eq!(english.string_from(&sample()), "Nov 3, 2026");

    let mut russian = formatter("ru", "");
    russian.set_date_style(Style::Long);
    russian.set_time_style(Style::None);
    assert_eq!(russian.string_from(&sample()), "3 \u{43d}\u{43e}\u{44f}\u{431}\u{440}\u{44f} 2026\u{202f}\u{433}.");

    let mut time_only = formatter("ru", "");
    time_only.set_date_style(Style::None);
    time_only.set_time_style(Style::Short);
    assert_eq!(time_only.string_from(&sample()), "14:05");

    let mut nothing = formatter("en", "");
    nothing.set_date_style(Style::None);
    nothing.set_time_style(Style::None);
    assert_eq!(nothing.string_from(&sample()), "");
}

#[test]
fn template_orders_fields_by_locale() {
    let mut english = formatter("en", "");
    english.set_localized_date_format_from_template("LLLL yyyy");
    assert_eq!(english.string_from(&sample()), "November 2026");
    english.set_localized_date_format_from_template("d MMMM yyyy");
    assert_eq!(english.string_from(&sample()), "November 3, 2026");
}

#[test]
fn template_reports_the_resolved_pattern() {
    let mut english = formatter("en", "");
    english.set_localized_date_format_from_template("d MMMM yyyy");
    assert_eq!(english.date_format(), "MMMM d, y");
}

#[test]
fn date_format_from_template_matches_foundation() {
    assert_eq!(DateFormatter::date_format_from_template("hmma", 0, "en").as_deref(), Some("h:mm\u{202f}a"));
    assert_eq!(DateFormatter::date_format_from_template("hmma", 0, "en_US").as_deref(), Some("h:mm\u{202f}a"));
    assert_eq!(DateFormatter::date_format_from_template("hmma", 0, "ru").as_deref(), Some("h:mm\u{202f}a"));
    let russian_hour = DateFormatter::date_format_from_template("j", 0, "ru").unwrap_or_default();
    assert!(russian_hour.contains('H') && !russian_hour.contains('a'), "{russian_hour}");
    let english_hour = DateFormatter::date_format_from_template("j", 0, "en_US").unwrap_or_default();
    assert!(english_hour.contains('h') && english_hour.contains('a'), "{english_hour}");
}

#[test]
fn last_setter_wins() {
    let mut formatter = formatter("en", "yyyy-MM-dd");
    formatter.set_date_style(Style::Medium);
    assert_eq!(formatter.date_format(), "MMM d, y");
    formatter.set_date_format("dd.MM.yyyy");
    assert_eq!(formatter.string_from(&sample()), "03.11.2026");
}

#[test]
fn time_zone_shifts_local_fields() {
    let mut moscow = formatter("en_US_POSIX", "yyyy-MM-dd HH:mm");
    if let Some(zone) = TimeZone::with_seconds_from_gmt(3 * 3_600) {
        moscow.set_time_zone(zone);
    }
    assert_eq!(moscow.string_from(&sample()), "2026-11-03 17:05");
}

#[test]
fn parses_numeric_patterns_strictly() {
    let day = formatter("en_US_POSIX", "yyyy-MM-dd");
    assert_eq!(day.date_from("2024-02-29"), Some(Date::from_ms(1_709_164_800_000)));
    assert_eq!(day.date_from("2024-02-30"), None);
    assert_eq!(day.date_from("2024-13-01"), None);
    assert_eq!(day.date_from("2024-02-29T10"), None);
    assert_eq!(day.date_from(""), None);

    let compact = formatter("en_US_POSIX", "yyyyMMdd");
    assert_eq!(compact.date_from("20240229"), Some(Date::from_ms(1_709_164_800_000)));

    let dotted = formatter("en_US_POSIX", "dd.MM.yyyy");
    assert_eq!(dotted.date_from("29.02.2024"), Some(Date::from_ms(1_709_164_800_000)));

    let mut zoned = formatter("en_US_POSIX", "yyyy-MM-dd'T'HH:mm:ss");
    if let Some(zone) = TimeZone::with_seconds_from_gmt(3 * 3_600) {
        zoned.set_time_zone(zone);
    }
    assert_eq!(zoned.date_from("2024-05-06T07:08:09"), Some(Date::from_ms(1_714_968_489_000)));

    let named = formatter("en", "d MMM yyyy");
    assert_eq!(named.date_from("3 Nov 2026"), None);
}

#[test]
fn formats_the_substituted_clock() {
    let _guard = RiseClock::install_test(TestClock::new(sample()));
    assert_eq!(formatter("en_US_POSIX", "yyyy-MM-dd HH:mm:ss.SSS").string_from(&Date::now()), "2026-11-03 14:05:09.123");
}
