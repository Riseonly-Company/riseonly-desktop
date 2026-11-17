use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, DateComponents};
use rise_core::foundation::foundation_date::Date;

use crate::swiftui::date_picker::{DatePickerTimeField, clamped, month_grid, month_start, replacing_day, replacing_time};

fn calendar() -> Calendar {
    let mut calendar = Calendar::gregorian();
    calendar.first_weekday = 2;
    calendar
}

fn date(calendar: &Calendar, year: i64, month: i64, day: i64, hour: i64, minute: i64) -> Date {
    calendar
        .date_from_components(&DateComponents { hour: Some(hour), minute: Some(minute), ..DateComponents::ymd(year, month, day) })
        .expect("a valid date")
}

#[test]
fn the_grid_starts_on_the_first_weekday_and_holds_the_whole_month() {
    let calendar = calendar();
    let cells = month_grid(&calendar, &date(&calendar, 2026, 11, 15, 12, 0));
    assert_eq!(cells.iter().take_while(|cell| cell.is_none()).count(), 6);
    assert_eq!(cells.iter().flatten().count(), 30);
}

#[test]
fn the_month_start_is_day_one() {
    let calendar = calendar();
    let start = month_start(&calendar, &date(&calendar, 2026, 2, 20, 9, 30)).expect("a start");
    assert_eq!(calendar.component(CalendarComponent::Day, &start), 1);
    assert_eq!(calendar.component(CalendarComponent::Month, &start), 2);
}

#[test]
fn picking_a_day_keeps_the_time() {
    let calendar = calendar();
    let moved = replacing_day(&calendar, &date(&calendar, 2026, 3, 1, 14, 45), &date(&calendar, 2026, 3, 9, 0, 0)).expect("a date");
    assert_eq!(moved, date(&calendar, 2026, 3, 9, 14, 45));
}

#[test]
fn time_fields_wrap_inside_their_unit() {
    let calendar = calendar();
    let start = date(&calendar, 2026, 3, 1, 23, 59);
    assert_eq!(replacing_time(&calendar, &start, DatePickerTimeField::Hour, 24), Some(date(&calendar, 2026, 3, 1, 0, 59)));
    assert_eq!(replacing_time(&calendar, &start, DatePickerTimeField::Minute, -1), Some(date(&calendar, 2026, 3, 1, 23, 59)));
}

#[test]
fn a_date_outside_the_range_is_pulled_in() {
    let calendar = calendar();
    let low = date(&calendar, 2026, 1, 1, 0, 0);
    let high = date(&calendar, 2026, 1, 31, 0, 0);
    let range = low..=high;
    assert_eq!(clamped(date(&calendar, 2025, 12, 1, 0, 0), Some(&range)), low);
    assert_eq!(clamped(date(&calendar, 2026, 2, 1, 0, 0), Some(&range)), high);
}
