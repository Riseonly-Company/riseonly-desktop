use super::foundation_calendar::{
    Calendar, CalendarComponent, DateComponents, TimeZone, zone_name_in_link_target,
};
use super::foundation_date::Date;

fn calendar_in(identifier: &str) -> Option<Calendar> {
    let mut calendar = Calendar::gregorian();
    calendar.time_zone = TimeZone::with_identifier(identifier)?;
    Some(calendar)
}

fn utc() -> Calendar {
    let mut calendar = Calendar::gregorian();
    calendar.time_zone = TimeZone::gmt();
    calendar
}

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

#[test]
fn fixed_offsets_follow_foundation_range_and_naming() {
    let moscow_like = TimeZone::with_seconds_from_gmt(3 * 3_600);
    assert_eq!(moscow_like.as_ref().map(TimeZone::identifier).as_deref(), Some("GMT+0300"));
    assert!(TimeZone::with_seconds_from_gmt(19 * 3_600).is_none());
    assert!(TimeZone::with_identifier("Not/AZone").is_none());
    assert_eq!(TimeZone::gmt().seconds_from_gmt(&Date::from_ms(0)), 0);
}

#[test]
fn components_in_utc() {
    let calendar = utc();
    let date = Date::from_ms(1_700_000_000_000);
    let parts = calendar.date_components(
        &[CalendarComponent::Year, CalendarComponent::Month, CalendarComponent::Day, CalendarComponent::Hour],
        &date,
    );
    assert_eq!((parts.year, parts.month, parts.day, parts.hour), (Some(2023), Some(11), Some(14), Some(22)));
    assert_eq!(calendar.component(CalendarComponent::Weekday, &date), 3);
    assert_eq!(calendar.component(CalendarComponent::Minute, &date), 13);
}

#[test]
fn start_of_day_uses_the_zone() {
    let Some(calendar) = calendar_in("Europe/Moscow") else {
        panic!("zone Europe/Moscow must resolve: system tzdata or the bundled database")
    };
    let date = Date::from_ms(1_700_000_000_000);
    let start = calendar.start_of_day(&date);
    assert_eq!(start.ms(), 1_700_000_000_000 - (22 * HOUR_MS + 13 * 60_000 + 20_000) - 3 * HOUR_MS + DAY_MS);
    assert!(calendar.is_date_in_same_day_as(&date, &start));
}

#[test]
fn dst_day_keeps_its_real_length() {
    let Some(calendar) = calendar_in("America/New_York") else {
        panic!("zone America/New_York must resolve: system tzdata or the bundled database")
    };
    let march_10 = calendar.date_from_components(&DateComponents::ymd(2024, 3, 10));
    let Some(march_10) = march_10 else { panic!("valid date") };
    let interval = calendar.date_interval(CalendarComponent::Day, &march_10);
    assert_eq!(interval.map(|day| day.end.ms() - day.start.ms()), Some(23 * HOUR_MS));
    let next = calendar.date_by_adding(CalendarComponent::Day, 1, &march_10);
    assert_eq!(next.map(|day| day.ms() - march_10.ms()), Some(23 * HOUR_MS));
}

#[test]
fn date_from_components_is_lenient() {
    let calendar = utc();
    let overflow = calendar.date_from_components(&DateComponents::ymd(2023, 13, 1));
    let january = calendar.date_from_components(&DateComponents::ymd(2024, 1, 1));
    assert!(overflow.is_some());
    assert_eq!(overflow, january);
    let earliest = calendar.date_from_components(&DateComponents::ymd(1901, 1, 1));
    assert_eq!(earliest.map(|date| date.ms()), Some(-2_177_452_800_000));
}

#[test]
fn month_interval_and_granularity() {
    let calendar = utc();
    let Some(date) = calendar.date_from_components(&DateComponents::ymd(2024, 2, 15)) else {
        panic!("valid date")
    };
    let month = calendar.date_interval(CalendarComponent::Month, &date);
    assert_eq!(month.map(|interval| (interval.end.ms() - interval.start.ms()) / DAY_MS), Some(29));
    let Some(other) = calendar.date_from_components(&DateComponents::ymd(2024, 2, 1)) else {
        panic!("valid date")
    };
    assert!(calendar.is_date_equal_to(&date, &other, CalendarComponent::Month));
    assert!(!calendar.is_date_equal_to(&date, &other, CalendarComponent::Day));
    assert!(calendar.is_date_equal_to(&date, &other, CalendarComponent::Year));
}

#[test]
fn whole_years_between_dates() {
    let calendar = utc();
    let (Some(birth), Some(today)) = (
        calendar.date_from_components(&DateComponents::ymd(2000, 6, 15)),
        calendar.date_from_components(&DateComponents::ymd(2024, 6, 14)),
    ) else {
        panic!("valid dates")
    };
    let years = calendar.date_components_between(&[CalendarComponent::Year], &birth, &today);
    assert_eq!(years.year, Some(23));
    let back = calendar.date_components_between(&[CalendarComponent::Day], &today, &birth);
    assert_eq!(back.day.map(|days| days < 0), Some(true));
}

#[test]
fn week_interval_starts_on_first_weekday() {
    let mut calendar = utc();
    calendar.first_weekday = 2;
    let Some(wednesday) = calendar.date_from_components(&DateComponents::ymd(2024, 5, 15)) else {
        panic!("valid date")
    };
    let week = calendar.date_interval(CalendarComponent::WeekOfYear, &wednesday);
    let start = week.map(|interval| calendar.component(CalendarComponent::Day, &interval.start));
    assert_eq!(start, Some(13));
}

#[test]
fn week_of_year_rolls_into_next_year_like_foundation() {
    let calendar = utc();
    let new_years_eve = calendar.date_from_components(&DateComponents::ymd(2024, 12, 31));
    let Some(new_years_eve) = new_years_eve else { panic!("valid date") };
    assert_eq!(calendar.component(CalendarComponent::WeekOfYear, &new_years_eve), 1);
    assert_eq!(calendar.component(CalendarComponent::WeekOfYear, &Date::from_ms(1_700_000_000_000)), 46);
}

#[test]
fn week_of_year_honours_minimum_days_in_first_week() {
    let mut calendar = utc();
    calendar.first_weekday = 2;
    calendar.minimum_days_in_first_week = 4;
    let friday = calendar.date_from_components(&DateComponents::ymd(2021, 1, 1));
    let Some(friday) = friday else { panic!("valid date") };
    assert_eq!(calendar.component(CalendarComponent::WeekOfYear, &friday), 53);
    let monday = calendar.date_from_components(&DateComponents::ymd(2021, 1, 4));
    let Some(monday) = monday else { panic!("valid date") };
    assert_eq!(calendar.component(CalendarComponent::WeekOfYear, &monday), 1);
}

#[test]
fn the_zone_name_comes_from_the_last_zoneinfo_of_the_link() {
    assert_eq!(
        zone_name_in_link_target("/var/db/timezone/zoneinfo/Asia/Almaty"),
        Some("Asia/Almaty")
    );
    assert_eq!(
        zone_name_in_link_target("../usr/share/zoneinfo/America/New_York"),
        Some("America/New_York")
    );
    assert_eq!(zone_name_in_link_target("/opt/zoneinfo/zoneinfo/UTC"), Some("UTC"));
}

#[test]
fn a_link_outside_zoneinfo_names_no_zone() {
    assert_eq!(zone_name_in_link_target("/usr/share/zoneinfo/"), None);
    assert_eq!(zone_name_in_link_target("/etc/custom-zone"), None);
}

#[test]
fn a_reset_reads_the_same_system_zone_again() {
    let before = TimeZone::current();
    TimeZone::reset_system();
    assert!(before == TimeZone::current());
}
