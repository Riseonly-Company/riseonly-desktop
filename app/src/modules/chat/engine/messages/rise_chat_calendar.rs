use std::collections::HashMap;

use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, DateComponents};
use rise_core::foundation::foundation_date::Date;

use crate::core::lib::canon_format::RiseFormat;

const WEEKDAY_SYMBOL_PATTERN: &str = "ccccc";
const SUNDAY_ANCHOR: (i64, i64, i64) = (2023, 1, 1);
const DAYS_IN_WEEK: i64 = 7;

/// one drawn month as local day boundaries, which are the request itself
/// same calendar as the message list date separators so the sheet and the date pills agree
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseChatCalendarWindow {
    pub day_starts: Vec<i64>,
    pub end_ms: i64,
}

impl RiseChatCalendarWindow {
    pub fn new(day_starts: Vec<i64>, end_ms: i64) -> Option<Self> {
        let last = *day_starts.last()?;
        if end_ms <= last {
            return None;
        }
        if !day_starts.windows(2).all(|pair| pair[0] < pair[1]) {
            return None;
        }
        Some(Self { day_starts, end_ms })
    }

    pub fn start_ms(&self) -> i64 {
        self.day_starts[0]
    }

    /// walked day by day and not computed, a local day can be 23 or 25 hours long
    pub fn month(containing: &Date, calendar: &Calendar) -> Option<RiseChatCalendarWindow> {
        let interval = calendar.date_interval(CalendarComponent::Month, containing)?;
        let mut day_starts = Vec::new();
        let mut cursor = calendar.start_of_day(&interval.start);
        while cursor < interval.end {
            day_starts.push(Self::milliseconds(&cursor));
            let Some(next) = calendar.date_by_adding(CalendarComponent::Day, 1, &cursor) else {
                break;
            };
            cursor = calendar.start_of_day(&next);
        }
        RiseChatCalendarWindow::new(day_starts, Self::milliseconds(&cursor))
    }

    pub fn contains(&self, day_start_ms: i64) -> bool {
        day_start_ms >= self.start_ms() && day_start_ms < self.end_ms
    }

    pub fn milliseconds(date: &Date) -> i64 {
        date.ms()
    }

    pub fn date(milliseconds: i64) -> Date {
        Date::from_ms(milliseconds)
    }
}

/// a day the server confirmed has visible messages
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseChatCalendarDay {
    pub day_start_ms: i64,
    pub first_message_id: String,
    pub first_message_at_ms: i64,
    pub preview_url: Option<String>,
    pub preview_is_video: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseChatCalendarPage {
    pub days: Vec<RiseChatCalendarDay>,
    pub has_older: bool,
}

impl RiseChatCalendarPage {
    pub fn empty() -> Self {
        Self { days: Vec::new(), has_older: false }
    }
}

/// one cell of the drawn grid
/// content decides everything, an empty day isnt tappable and dates alone say nothing
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseChatCalendarCell {
    pub day_start_ms: i64,
    pub day_number: i64,
    pub is_today: bool,
    pub is_future: bool,
    pub content: Option<RiseChatCalendarDay>,
}

impl RiseChatCalendarCell {
    pub fn id(&self) -> i64 {
        self.day_start_ms
    }

    pub fn is_selectable(&self) -> bool {
        self.content.is_some()
    }
}

/// prepared month grid, built once off the main thread
/// nothing here may be recomputed inside render
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseChatCalendarMonth {
    pub start_ms: i64,
    pub title: String,
    pub leading_blanks: i64,
    pub cells: Vec<RiseChatCalendarCell>,
}

impl RiseChatCalendarMonth {
    pub fn id(&self) -> i64 {
        self.start_ms
    }

    pub fn has_any_content(&self) -> bool {
        self.cells.iter().any(|cell| cell.content.is_some())
    }

    pub fn make(
        window: &RiseChatCalendarWindow,
        page: &RiseChatCalendarPage,
        calendar: &Calendar,
        title: String,
        now: &Date,
    ) -> RiseChatCalendarMonth {
        let mut content_by_day: HashMap<i64, &RiseChatCalendarDay> = HashMap::with_capacity(page.days.len());
        for day in &page.days {
            content_by_day.insert(day.day_start_ms, day);
        }

        let today_start = RiseChatCalendarWindow::milliseconds(&calendar.start_of_day(now));
        let cells = window
            .day_starts
            .iter()
            .map(|&day_start_ms| {
                let date = RiseChatCalendarWindow::date(day_start_ms);
                RiseChatCalendarCell {
                    day_start_ms,
                    day_number: calendar.component(CalendarComponent::Day, &date),
                    is_today: day_start_ms == today_start,
                    is_future: day_start_ms > today_start,
                    content: content_by_day.get(&day_start_ms).map(|day| (*day).clone()),
                }
            })
            .collect();

        RiseChatCalendarMonth {
            start_ms: window.start_ms(),
            title,
            leading_blanks: Self::leading_blanks(window, calendar),
            cells,
        }
    }

    /// columns to skip before the first day, uses the locale first weekday and not monday
    pub fn leading_blanks(window: &RiseChatCalendarWindow, calendar: &Calendar) -> i64 {
        let first = RiseChatCalendarWindow::date(window.start_ms());
        let weekday = calendar.component(CalendarComponent::Weekday, &first);
        (weekday - i64::from(calendar.first_weekday) + DAYS_IN_WEEK) % DAYS_IN_WEEK
    }

    /// weekday initials in the locale column order
    /// uses the shared formatter cache since a new formatter per open is slow on a cold sheet
    pub fn weekday_symbols(calendar: &Calendar, locale: &str) -> Vec<String> {
        let formatter = RiseFormat::date_formatter(WEEKDAY_SYMBOL_PATTERN, locale, calendar, None);
        let (year, month, day) = SUNDAY_ANCHOR;
        let Some(sunday) = calendar.date_from_components(&DateComponents::ymd(year, month, day)) else {
            return Vec::new();
        };
        let mut symbols: Vec<String> = (0..DAYS_IN_WEEK)
            .filter_map(|offset| calendar.date_by_adding(CalendarComponent::Day, offset, &sunday))
            .map(|date| formatter.string_from(&date))
            .collect();
        if symbols.len() != DAYS_IN_WEEK as usize {
            return symbols;
        }
        let offset = usize::from(calendar.first_weekday).saturating_sub(1);
        if offset == 0 || offset >= symbols.len() {
            return symbols;
        }
        symbols.rotate_left(offset);
        symbols
    }
}
