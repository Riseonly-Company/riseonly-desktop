use rise_core::foundation::foundation_calendar::{Calendar, DateComponents, TimeZone};

use crate::modules::chat::engine::messages::rise_chat_calendar::{
    RiseChatCalendarDay, RiseChatCalendarMonth, RiseChatCalendarPage, RiseChatCalendarWindow,
};

const DAY_MS: i64 = 86_400_000;

fn gmt_calendar(first_weekday: u8) -> Calendar {
    let mut calendar = Calendar::gregorian();
    calendar.time_zone = TimeZone::gmt();
    calendar.first_weekday = first_weekday;
    calendar
}

fn january_2023(calendar: &Calendar) -> RiseChatCalendarWindow {
    let date = calendar.date_from_components(&DateComponents::ymd(2023, 1, 15));
    let window = date.and_then(|date| RiseChatCalendarWindow::month(&date, calendar));
    assert!(window.is_some());
    window.unwrap_or_else(|| RiseChatCalendarWindow { day_starts: vec![0], end_ms: 1 })
}

#[test]
fn window_refuses_unordered_or_empty_days() {
    assert!(RiseChatCalendarWindow::new(Vec::new(), 10).is_none());
    assert!(RiseChatCalendarWindow::new(vec![5], 5).is_none());
    assert!(RiseChatCalendarWindow::new(vec![5, 5], 10).is_none());
    assert!(RiseChatCalendarWindow::new(vec![5, 6], 10).is_some());
}

#[test]
fn month_walks_every_local_day_and_ends_exclusively() {
    let calendar = gmt_calendar(1);
    let window = january_2023(&calendar);
    assert_eq!(window.day_starts.len(), 31);
    assert_eq!(window.end_ms - window.start_ms(), 31 * DAY_MS);
    assert!(window.contains(window.start_ms()));
    assert!(!window.contains(window.end_ms));
}

#[test]
fn leading_blanks_follow_the_first_weekday_of_the_locale() {
    let sunday_first = gmt_calendar(1);
    assert_eq!(RiseChatCalendarMonth::leading_blanks(&january_2023(&sunday_first), &sunday_first), 0);
    let monday_first = gmt_calendar(2);
    assert_eq!(RiseChatCalendarMonth::leading_blanks(&january_2023(&monday_first), &monday_first), 6);
}

#[test]
fn only_a_day_with_server_content_is_selectable() {
    let calendar = gmt_calendar(1);
    let window = january_2023(&calendar);
    let day = RiseChatCalendarDay {
        day_start_ms: window.start_ms(),
        first_message_id: "message-a".to_owned(),
        first_message_at_ms: window.start_ms() + 1,
        preview_url: None,
        preview_is_video: false,
    };
    let page = RiseChatCalendarPage { days: vec![day], has_older: false };
    let now = RiseChatCalendarWindow::date(window.start_ms() + 2 * DAY_MS);
    let month = RiseChatCalendarMonth::make(&window, &page, &calendar, "January".to_owned(), &now);

    assert!(month.has_any_content());
    assert!(month.cells[0].is_selectable());
    assert!(!month.cells[1].is_selectable());
    assert!(month.cells[2].is_today);
    assert!(month.cells[3].is_future);
    assert_eq!(month.cells[30].day_number, 31);
}

#[test]
fn weekday_symbols_rotate_to_the_first_weekday() {
    let sunday_first = RiseChatCalendarMonth::weekday_symbols(&gmt_calendar(1), "en");
    let monday_first = RiseChatCalendarMonth::weekday_symbols(&gmt_calendar(2), "en");
    assert_eq!(sunday_first.len(), 7);
    assert_eq!(monday_first.first(), sunday_first.get(1));
    assert_eq!(monday_first.last(), sunday_first.first());
}
