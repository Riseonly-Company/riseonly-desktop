use std::collections::HashMap;

use rise_catalog::RestrictionScope;
use rise_core::foundation::foundation_calendar::{Calendar, DateComponents, TimeZone};
use rise_core::foundation::foundation_date::Date;
use rise_errors::{ErrorCode, ErrorParam};

use crate::core::lib::canon_errors::RiseFailure;
use crate::core::lib::restriction_until_format::RestrictionUntilFormat;
use crate::core::restriction::restriction_store::RestrictionRefusal;

const LOCALE: &str = "en_GB";

fn calendar() -> Calendar {
    let mut calendar = Calendar::gregorian();
    calendar.time_zone = TimeZone::with_identifier("Asia/Tokyo").expect("Asia/Tokyo is in the tz database");
    calendar
}

fn date(calendar: &Calendar, year: i64, month: i64, day: i64, hour: i64, minute: i64) -> Date {
    calendar
        .date_from_components(&DateComponents {
            year: Some(year),
            month: Some(month),
            day: Some(day),
            hour: Some(hour),
            minute: Some(minute),
            ..DateComponents::default()
        })
        .expect("a valid gregorian date")
}

fn milliseconds(date: &Date) -> i64 {
    (date.time_interval_since_1970() * 1_000.0) as i64
}

/// echoes the chosen key with both placeholders so a test sees which phrase was picked
fn echo(key: &str) -> String {
    format!("{key}|{{date}}|{{time}}")
}

fn text(until: &Date, now: &Date, calendar: &Calendar) -> String {
    RestrictionUntilFormat::text_with(milliseconds(until), now, calendar, LOCALE, &echo)
}

fn failure(code: &str, details: &[(ErrorParam, &str)]) -> RiseFailure {
    RiseFailure::new(
        Some(code.to_owned()),
        String::new(),
        details
            .iter()
            .map(|(param, value)| (param.wire().to_owned(), (*value).to_owned()))
            .collect::<HashMap<_, _>>(),
    )
}

#[test]
fn zero_reads_as_until_lifted() {
    let calendar = calendar();
    let now = date(&calendar, 2026, 11, 4, 10, 0);
    let text = RestrictionUntilFormat::text_with(0, &now, &calendar, LOCALE, &echo);
    assert_eq!(text, "restriction_until_forever|{date}|{time}");
}

#[test]
fn same_day_reads_as_today() {
    let calendar = calendar();
    let now = date(&calendar, 2026, 11, 4, 10, 0);
    let until = date(&calendar, 2026, 11, 4, 18, 30);
    assert_eq!(text(&until, &now, &calendar), "restriction_until_today|{date}|18:30");
}

#[test]
fn next_day_reads_as_tomorrow() {
    let calendar = calendar();
    let now = date(&calendar, 2026, 11, 4, 23, 50);
    let until = date(&calendar, 2026, 11, 5, 9, 5);
    assert_eq!(text(&until, &now, &calendar), "restriction_until_tomorrow|{date}|09:05");
}

#[test]
fn later_this_year_shows_day_and_month() {
    let calendar = calendar();
    let now = date(&calendar, 2026, 11, 4, 10, 0);
    let until = date(&calendar, 2026, 12, 24, 8, 0);
    assert_eq!(text(&until, &now, &calendar), "restriction_until_date|24 December|08:00");
}

#[test]
fn next_year_adds_the_year() {
    let calendar = calendar();
    let now = date(&calendar, 2026, 12, 30, 10, 0);
    let until = date(&calendar, 2027, 1, 15, 12, 0);
    assert_eq!(text(&until, &now, &calendar), "restriction_until_date|15 January 2027|12:00");
}

#[test]
fn day_boundary_follows_the_calendar_time_zone() {
    let tokyo = calendar();
    let mut utc = Calendar::gregorian();
    utc.time_zone = TimeZone::with_identifier("UTC").expect("UTC is in the tz database");
    let now = date(&utc, 2026, 11, 4, 12, 0);
    let until = date(&utc, 2026, 11, 4, 23, 30);
    assert!(text(&until, &now, &utc).starts_with("restriction_until_today|"));
    assert_eq!(text(&until, &now, &tokyo), "restriction_until_tomorrow|{date}|08:30");
}

#[test]
fn restricted_scope_is_a_restriction_under_any_code() {
    let failure = failure(
        "SOME_CODE_THIS_BUILD_LACKS",
        &[
            (ErrorParam::RestrictedScope, "livestream"),
            (ErrorParam::RestrictedUntilMs, "1800000000000"),
        ],
    );
    assert_eq!(
        RestrictionRefusal::new(&failure),
        Some(RestrictionRefusal::Restriction { scope: "livestream".to_owned(), until_ms: 1_800_000_000_000 })
    );
    assert!(failure.is_restriction());
}

#[test]
fn missing_deadline_means_until_lifted() {
    let failure = failure(
        ErrorCode::ModerationCreateBlocked.wire(),
        &[(ErrorParam::RestrictedScope, RestrictionScope::Comment.wire())],
    );
    assert_eq!(
        RestrictionRefusal::new(&failure),
        Some(RestrictionRefusal::Restriction { scope: "comment".to_owned(), until_ms: 0 })
    );
}

#[test]
fn overload_category_reaches_the_modal_with_the_phrase_of_the_code() {
    let failure = failure(ErrorCode::RtcOverloaded.wire(), &[(ErrorParam::RetryAfterS, "5")]);
    let refusal = RestrictionRefusal::new(&failure);
    assert!(matches!(refusal, Some(RestrictionRefusal::Overload { text: Some(_) })));
    assert!(!failure.is_restriction());
}

#[test]
fn full_lane_is_not_a_modal() {
    let failure = failure(ErrorCode::CommonCapacityExhausted.wire(), &[(ErrorParam::RetryAfterS, "2")]);
    assert_eq!(RestrictionRefusal::new(&failure), None);
}

#[test]
fn content_rejection_carries_violations_left() {
    let counted = failure(ErrorCode::ModerationContentRejected.wire(), &[(ErrorParam::ViolationsLeft, "2")]);
    let bare = failure(ErrorCode::ModerationContentRejected.wire(), &[]);
    assert_eq!(
        RestrictionRefusal::new(&counted),
        Some(RestrictionRefusal::Rejection { violations_left: Some(2) })
    );
    assert_eq!(
        RestrictionRefusal::new(&bare),
        Some(RestrictionRefusal::Rejection { violations_left: None })
    );
}

#[test]
fn other_refusals_are_not_explained() {
    let failure = failure(ErrorCode::CommonQuotaExceeded.wire(), &[]);
    assert_eq!(RestrictionRefusal::new(&failure), None);
    assert!(!failure.is_restriction());
}
