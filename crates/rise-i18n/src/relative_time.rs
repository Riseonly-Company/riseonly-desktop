use crate::tr;

pub fn relative(created_at_ms: Option<i64>, iso8601: &str, now_ms: i64) -> String {
    let Some(created) = created_at_ms.or_else(|| parse_iso8601_ms(iso8601)) else {
        return String::new();
    };

    let elapsed_ms = (now_ms - created).max(0);
    let minutes = elapsed_ms / 60_000;

    if minutes < 1 {
        return tr("story_time_now");
    }
    if minutes < 60 {
        return format!("{minutes}{}", tr("story_time_minutes_short"));
    }

    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}{}", tr("story_time_hours_short"));
    }

    format!("{}{}", hours / 24, tr("story_time_days_short"))
}

fn parse_iso8601_ms(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.len() < 19 {
        return None;
    }

    let bytes = value.as_bytes();
    let number =
        |range: std::ops::Range<usize>| -> Option<i64> { value.get(range)?.parse::<i64>().ok() };

    if bytes[4] != b'-' || bytes[7] != b'-' || (bytes[10] != b'T' && bytes[10] != b' ') {
        return None;
    }

    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(((days * 86_400) + hour * 3_600 + minute * 60 + second.min(59)) * 1_000)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;

    #[test]
    fn the_epoch_maps_to_day_zero() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
    }

    #[test]
    fn a_leap_day_is_counted() {
        assert_eq!(
            days_from_civil(2024, 3, 1) - days_from_civil(2024, 2, 28),
            2,
            "2024 is a leap year, so the 29th sits between them"
        );
        assert_eq!(
            days_from_civil(2023, 3, 1) - days_from_civil(2023, 2, 28),
            1
        );
    }

    #[test]
    fn an_rfc3339_timestamp_parses_to_the_same_instant_as_its_ms_field() {
        let expected = days_from_civil(2026, 8, 7) * 86_400_000;
        assert_eq!(parse_iso8601_ms("2026-08-07T00:00:00Z"), Some(expected));
        assert_eq!(
            parse_iso8601_ms("2026-08-07T00:00:00.123Z"),
            Some(expected),
            "fractional seconds are dropped, not rejected"
        );
        assert_eq!(
            parse_iso8601_ms("2026-08-07 00:00:00"),
            Some(expected),
            "the gateway has sent a space instead of the T"
        );
    }

    #[test]
    fn a_timestamp_that_is_not_one_is_refused_rather_than_guessed_at() {
        assert_eq!(parse_iso8601_ms(""), None);
        assert_eq!(parse_iso8601_ms("yesterday"), None);
        assert_eq!(parse_iso8601_ms("2026/08/07T00:00:00Z"), None);
        assert_eq!(parse_iso8601_ms("2026-13-07T00:00:00Z"), None);
        assert_eq!(parse_iso8601_ms("2026-08-07T99:00:00Z"), None);
    }

    #[test]
    fn a_leap_second_does_not_produce_a_minute_of_sixty() {
        let at_60 = parse_iso8601_ms("2016-12-31T23:59:60Z").unwrap();
        let at_59 = parse_iso8601_ms("2016-12-31T23:59:59Z").unwrap();
        assert_eq!(at_60, at_59);
    }

    #[test]
    fn the_ms_field_wins_over_the_string_because_the_proto_says_it_is_authoritative() {
        let now = 10 * DAY;
        let from_ms = relative(Some(now - 5 * MINUTE), "1970-01-01T00:00:00Z", now);
        assert!(from_ms.starts_with('5'), "got {from_ms}");
    }

    #[test]
    fn the_string_is_the_fallback_when_the_ms_field_is_absent() {
        let created = days_from_civil(2026, 8, 7) * 86_400_000;
        let now = created + 2 * HOUR;
        assert!(relative(None, "2026-08-07T00:00:00Z", now).starts_with('2'));
    }

    #[test]
    fn a_post_with_no_timestamp_at_all_draws_nothing_rather_than_a_wrong_time() {
        assert_eq!(relative(None, "", 0), "");
        assert_eq!(relative(None, "not a date", 0), "");
    }

    #[test]
    fn the_bands_are_now_minutes_hours_days() {
        let now = 100 * DAY;
        let minutes = tr("story_time_minutes_short");
        let hours = tr("story_time_hours_short");
        let days = tr("story_time_days_short");

        assert_eq!(relative(Some(now), "", now), tr("story_time_now"));
        assert_eq!(relative(Some(now - 30_000), "", now), tr("story_time_now"));
        assert_eq!(relative(Some(now - MINUTE), "", now), format!("1{minutes}"));
        assert_eq!(
            relative(Some(now - 59 * MINUTE), "", now),
            format!("59{minutes}")
        );
        assert_eq!(relative(Some(now - HOUR), "", now), format!("1{hours}"));
        assert_eq!(
            relative(Some(now - 23 * HOUR), "", now),
            format!("23{hours}")
        );
        assert_eq!(relative(Some(now - DAY), "", now), format!("1{days}"));
        assert_eq!(
            relative(Some(now - 400 * DAY), "", now),
            format!("400{days}")
        );
    }

    #[test]
    fn a_clock_behind_the_server_reads_as_now_rather_than_as_the_future() {
        let now = 100 * DAY;
        assert_eq!(
            relative(Some(now + 5 * MINUTE), "", now),
            tr("story_time_now"),
            "\"in 5 minutes\" on a feed row is a bug the user blames on the app"
        );
    }
}
