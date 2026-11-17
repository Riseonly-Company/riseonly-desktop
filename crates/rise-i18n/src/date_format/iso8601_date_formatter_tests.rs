use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;

use super::iso8601_date_formatter::{ISO8601DateFormatter, ISO8601DateFormatterOptions as Options};

const SAMPLE_MS: i64 = 1_793_714_709_123;
const SAMPLE_WHOLE_SECOND_MS: i64 = 1_793_714_709_000;

fn formatter(options: Options) -> ISO8601DateFormatter {
    let mut formatter = ISO8601DateFormatter::new();
    formatter.set_format_options(options);
    formatter
}

fn diagnostics_options() -> Options {
    Options::WITH_YEAR | Options::WITH_MONTH | Options::WITH_DAY | Options::WITH_TIME
}

#[test]
fn option_raw_bits_match_foundation() {
    assert_eq!(Options::WITH_FULL_DATE, Options::WITH_YEAR | Options::WITH_MONTH | Options::WITH_DAY | Options::WITH_DASH_SEPARATOR_IN_DATE);
    assert!(Options::WITH_INTERNET_DATE_TIME.contains(Options::WITH_FULL_TIME));
    assert!(!Options::WITH_INTERNET_DATE_TIME.contains(Options::WITH_FRACTIONAL_SECONDS));
}

#[test]
fn default_is_internet_date_time_in_gmt() {
    let formatter = ISO8601DateFormatter::new();
    assert_eq!(formatter.format_options(), Options::WITH_INTERNET_DATE_TIME);
    assert_eq!(formatter.string_from(&Date::from_ms(SAMPLE_MS)), "2026-11-03T14:05:09Z");
}

#[test]
fn formats_each_option_shape() {
    let sample = Date::from_ms(SAMPLE_MS);
    let fractional = formatter(Options::WITH_INTERNET_DATE_TIME | Options::WITH_FRACTIONAL_SECONDS);
    assert_eq!(fractional.string_from(&sample), "2026-11-03T14:05:09.123Z");
    assert_eq!(formatter(diagnostics_options()).string_from(&sample), "20261103T140509");
    assert_eq!(formatter(Options::WITH_FULL_DATE).string_from(&sample), "2026-11-03");
    let week = Options::WITH_YEAR | Options::WITH_WEEK_OF_YEAR | Options::WITH_DAY | Options::WITH_DASH_SEPARATOR_IN_DATE;
    assert_eq!(formatter(week).string_from(&sample), "2026-W45-02");
    let ordinal = Options::WITH_YEAR | Options::WITH_DAY | Options::WITH_DASH_SEPARATOR_IN_DATE;
    assert_eq!(formatter(ordinal).string_from(&sample), "2026-307");
    let spaced = Options::WITH_INTERNET_DATE_TIME | Options::WITH_SPACE_BETWEEN_DATE_AND_TIME;
    assert_eq!(formatter(spaced).string_from(&sample), "2026-11-03 14:05:09Z");
}

#[test]
fn formats_zone_offsets() {
    let mut formatter = ISO8601DateFormatter::new();
    let Some(moscow) = TimeZone::with_seconds_from_gmt(3 * 3_600) else {
        panic!("fixed offset zone");
    };
    formatter.set_time_zone(moscow);
    assert_eq!(formatter.string_from(&Date::from_ms(SAMPLE_MS)), "2026-11-03T17:05:09+03:00");
    let mut options = Options::WITH_INTERNET_DATE_TIME;
    options.remove(Options::WITH_COLON_SEPARATOR_IN_TIME_ZONE);
    formatter.set_format_options(options);
    assert_eq!(formatter.string_from(&Date::from_ms(SAMPLE_MS)), "2026-11-03T17:05:09+0300");
}

#[test]
fn default_parses_internet_date_time_only() {
    let formatter = ISO8601DateFormatter::new();
    let expected = Some(Date::from_ms(SAMPLE_WHOLE_SECOND_MS));
    assert_eq!(formatter.date_from("2026-11-03T14:05:09Z"), expected);
    assert_eq!(formatter.date_from("2026-11-03T17:05:09+03:00"), expected);
    assert_eq!(formatter.date_from("2026-11-03T17:05:09+0300"), expected);
    assert_eq!(formatter.date_from("2026-11-03T14:05:09.123Z"), None);
    assert_eq!(formatter.date_from("2026-11-03"), None);
    assert_eq!(formatter.date_from("20261103T140509"), None);
    assert_eq!(formatter.date_from("2026-11-03T14:05:09"), None);
    assert_eq!(formatter.date_from("2026-11-03T14:05:09Z "), None);
    assert_eq!(formatter.date_from("2026-13-03T14:05:09Z"), None);
    assert_eq!(formatter.date_from("2026-02-29T14:05:09Z"), None);
}

#[test]
fn fractional_parse_truncates_to_milliseconds() {
    let formatter = formatter(Options::WITH_INTERNET_DATE_TIME | Options::WITH_FRACTIONAL_SECONDS);
    assert_eq!(formatter.date_from("2026-11-03T14:05:09.123456Z"), Some(Date::from_ms(SAMPLE_MS)));
    assert_eq!(formatter.date_from("2026-11-03T14:05:09.1Z"), Some(Date::from_ms(SAMPLE_WHOLE_SECOND_MS + 100)));
    assert_eq!(formatter.date_from("2026-11-03T14:05:09Z"), None);
}

#[test]
fn diagnostics_shape_round_trips() {
    let formatter = formatter(diagnostics_options());
    let stamp = formatter.string_from(&Date::from_ms(SAMPLE_MS));
    assert_eq!(formatter.date_from(&stamp), Some(Date::from_ms(SAMPLE_WHOLE_SECOND_MS)));
    assert_eq!(ISO8601DateFormatter::new().date_from(&stamp), None);
}

#[test]
fn week_and_ordinal_dates_parse() {
    let midnight = Some(Date::from_ms(SAMPLE_MS - SAMPLE_MS % 86_400_000));
    let week = Options::WITH_YEAR | Options::WITH_WEEK_OF_YEAR | Options::WITH_DAY | Options::WITH_DASH_SEPARATOR_IN_DATE;
    assert_eq!(formatter(week).date_from("2026-W45-02"), midnight);
    assert_eq!(formatter(week).date_from("2026-W54-02"), None);
    let ordinal = Options::WITH_YEAR | Options::WITH_DAY | Options::WITH_DASH_SEPARATOR_IN_DATE;
    assert_eq!(formatter(ordinal).date_from("2026-307"), midnight);
    assert_eq!(formatter(ordinal).date_from("2026-366"), None);
}
