use std::ops::RangeInclusive;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, Div, ElementId, Entity, FocusHandle, FontWeight, Hsla, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    RenderOnce, Stateful, StatefulInteractiveElement, Styled, Window, div, px,
};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, DateComponents};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_i18n::app_date_formatter::AppDateFormatter;
use rise_theme::{AppTheme, ThemeStore};

use crate::IconUi;
use crate::main_text::MainText;
use crate::swiftui::binding::Binding;
use crate::swiftui::button_style::plain;
use crate::swiftui::shapes::SwiftUiShapes;
use crate::swiftui::stacks::{HorizontalAlignment, StackAlign, hstack, spacer_min_w, vstack};

const DAYS_IN_WEEK: usize = 7;
const DAY_STEP: i64 = 1;
const WEEK_STEP: i64 = 7;
const HOURS_IN_DAY: i64 = 24;
const MINUTES_IN_HOUR: i64 = 60;
const DAY_CELL_HEIGHT: f32 = 40.0;
const DAY_CIRCLE_SIDE: f32 = 36.0;
const SELECTED_DAY_FILL_OPACITY: f32 = 0.15;
const DISABLED_DAY_OPACITY: f32 = 0.35;
const MONTH_CHEVRON_SIDE: f32 = 15.0;
const TIME_CHEVRON_SIDE: f32 = 11.0;
const TIME_FIELD_WIDTH: f32 = 44.0;
const TIME_FIELD_HEIGHT: f32 = 34.0;
const TIME_FIELD_RADIUS: f32 = 8.0;
const FOCUSED_FIELD_FILL_OPACITY: f32 = 0.15;
const TYPED_TIME_DIGITS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatePickerComponents {
    Date,
    HourAndMinute,
    DateAndHourAndMinute,
}

impl DatePickerComponents {
    fn shows_date(self) -> bool {
        !matches!(self, Self::HourAndMinute)
    }

    fn shows_time(self) -> bool {
        !matches!(self, Self::Date)
    }
}

/// which of the two time fields a digit or arrow key goes to
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatePickerTimeField {
    Hour,
    Minute,
}

/// first instant of the month the date is in
pub fn month_start(calendar: &Calendar, date: &Date) -> Option<Date> {
    calendar.date_from_components(&DateComponents::ymd(
        calendar.component(CalendarComponent::Year, date),
        calendar.component(CalendarComponent::Month, date),
        1,
    ))
}

/// cells of one month by week from the first weekday, none is an empty cell before day one
/// last week stays short
pub fn month_grid(calendar: &Calendar, month: &Date) -> Vec<Option<Date>> {
    let Some(first) = month_start(calendar, month) else {
        return Vec::new();
    };
    let weekday = calendar.component(CalendarComponent::Weekday, &first);
    let leading = (weekday - i64::from(calendar.first_weekday)).rem_euclid(DAYS_IN_WEEK as i64) as usize;
    let number = calendar.component(CalendarComponent::Month, &first);
    let mut cells: Vec<Option<Date>> = vec![None; leading];
    let mut offset = 0;
    while let Some(day) = calendar.date_by_adding(CalendarComponent::Day, offset, &first) {
        if calendar.component(CalendarComponent::Month, &day) != number {
            break;
        }
        cells.push(Some(day));
        offset += DAY_STEP;
    }
    cells
}

/// date pulled into the picker range
pub fn clamped(date: Date, range: Option<&RangeInclusive<Date>>) -> Date {
    match range {
        Some(range) => date.clamp(*range.start(), *range.end()),
        None => date,
    }
}

/// selection moved to the day of day, time of day kept
pub fn replacing_day(calendar: &Calendar, selection: &Date, day: &Date) -> Option<Date> {
    calendar.date_from_components(&DateComponents {
        year: Some(calendar.component(CalendarComponent::Year, day)),
        month: Some(calendar.component(CalendarComponent::Month, day)),
        day: Some(calendar.component(CalendarComponent::Day, day)),
        hour: Some(calendar.component(CalendarComponent::Hour, selection)),
        minute: Some(calendar.component(CalendarComponent::Minute, selection)),
        ..DateComponents::default()
    })
}

/// selection with the given time field set to value, wraps inside the day or hour
/// seconds go to zero, no field for them
pub fn replacing_time(calendar: &Calendar, selection: &Date, field: DatePickerTimeField, value: i64) -> Option<Date> {
    let (hour, minute) = match field {
        DatePickerTimeField::Hour => (
            value.rem_euclid(HOURS_IN_DAY),
            calendar.component(CalendarComponent::Minute, selection),
        ),
        DatePickerTimeField::Minute => (
            calendar.component(CalendarComponent::Hour, selection),
            value.rem_euclid(MINUTES_IN_HOUR),
        ),
    };
    calendar.date_from_components(&DateComponents {
        year: Some(calendar.component(CalendarComponent::Year, selection)),
        month: Some(calendar.component(CalendarComponent::Month, selection)),
        day: Some(calendar.component(CalendarComponent::Day, selection)),
        hour: Some(hour),
        minute: Some(minute),
        ..DateComponents::default()
    })
}

/// picker state: month on screen, focus of its stops and digits typed into a time field so far
struct DatePickerState {
    displayed_month: Option<Date>,
    grid_focus: FocusHandle,
    hour_focus: FocusHandle,
    minute_focus: FocusHandle,
    typed: String,
}

#[derive(IntoElement)]
pub struct DatePicker {
    id: ElementId,
    selection: Binding<Date>,
    range: Option<RangeInclusive<Date>>,
    displayed_components: DatePickerComponents,
    tint: Option<Hsla>,
}

impl DatePicker {
    pub fn new(id: impl Into<ElementId>, selection: Binding<Date>) -> Self {
        Self {
            id: id.into(),
            selection,
            range: None,
            displayed_components: DatePickerComponents::DateAndHourAndMinute,
            tint: None,
        }
    }

    pub fn range(mut self, range: RangeInclusive<Date>) -> Self {
        self.range = Some(range);
        self
    }

    pub fn displayed_components(mut self, displayed_components: DatePickerComponents) -> Self {
        self.displayed_components = displayed_components;
        self
    }

    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }
}

fn commit(selection: &Binding<Date>, range: Option<&RangeInclusive<Date>>, date: Date, window: &mut Window, cx: &mut App) {
    selection.set(clamped(date, range), window, cx);
}

fn chevron(name: &str, side: f32, color: Hsla) -> Div {
    div().size(px(side)).flex().items_center().justify_center().children(IconUi::sized(name, px(side), color, false))
}

impl RenderOnce for DatePicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let tint = self.tint.unwrap_or(theme.primary._100);
        let state = window.use_keyed_state((self.id.clone(), "state"), cx, |_, cx| DatePickerState {
            displayed_month: None,
            grid_focus: cx.focus_handle(),
            hour_focus: cx.focus_handle(),
            minute_focus: cx.focus_handle(),
            typed: String::new(),
        });
        let calendar = Calendar::current();
        let selection_date = self.selection.get(cx);
        let displayed_month = state
            .read(cx)
            .displayed_month
            .or_else(|| month_start(&calendar, &selection_date))
            .unwrap_or(selection_date);

        let mut column = vstack(12.0).w_full().align(HorizontalAlignment::Leading);
        if self.displayed_components.shows_date() {
            column = column.child(month_header(&self, &state, &calendar, &displayed_month, tint, &theme));
            column = column.child(weekday_row(&calendar, &displayed_month, &theme));
            column = column.child(day_grid(&self, &state, &calendar, &displayed_month, &selection_date, tint, &theme, window, cx));
        }
        if self.displayed_components.shows_time() {
            column = column.child(time_row(&self, &state, &calendar, &selection_date, tint, &theme, window, cx));
        }
        column
    }
}

fn month_header(
    picker: &DatePicker,
    state: &Entity<DatePickerState>,
    calendar: &Calendar,
    displayed_month: &Date,
    tint: Hsla,
    theme: &AppTheme,
) -> Div {
    let mut formatter = AppDateFormatter::make(None, None, None);
    formatter.set_localized_date_format_from_template("LLLL yyyy");
    let month_button = |name: &'static str, step: i64| {
        let target = calendar.date_by_adding(CalendarComponent::Month, step, displayed_month);
        let reachable = target.as_ref().is_some_and(|target| match picker.range.as_ref() {
            Some(range) => {
                let target_end = calendar.date_by_adding(CalendarComponent::Month, 1, target).unwrap_or(*target);
                *target <= *range.end() && target_end > *range.start()
            }
            None => true,
        });
        let state = state.clone();
        plain((picker.id.clone(), name))
            .disabled(!reachable)
            .on_press(move |_, cx| {
                state.update(cx, |state, cx| {
                    state.displayed_month = target;
                    cx.notify();
                });
            })
            .child(chevron(name, MONTH_CHEVRON_SIDE, if reachable { tint } else { tint.opacity(DISABLED_DAY_OPACITY) }))
    };
    hstack(16.0)
        .w_full()
        .child(MainText::new(formatter.string_from(displayed_month)).px(17.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
        .child(spacer_min_w(0.0))
        .child(month_button("chevron.left", -1))
        .child(month_button("chevron.right", 1))
}

fn weekday_row(calendar: &Calendar, displayed_month: &Date, theme: &AppTheme) -> Div {
    let mut formatter = AppDateFormatter::make(None, None, None);
    formatter.set_date_format("ccc");
    let cells = month_grid(calendar, displayed_month);
    let first_of_week = cells.iter().flatten().next().copied();
    let leading = cells.iter().take_while(|cell| cell.is_none()).count() as i64;
    let mut row = hstack(0.0).w_full();
    for index in 0..DAYS_IN_WEEK as i64 {
        let label = first_of_week
            .and_then(|first| calendar.date_by_adding(CalendarComponent::Day, index - leading, &first))
            .map(|day| formatter.string_from(&day).to_uppercase())
            .unwrap_or_default();
        row = row.child(
            div()
                .flex_1()
                .flex()
                .justify_center()
                .child(MainText::new(label).px(12.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary)),
        );
    }
    row
}

#[allow(clippy::too_many_arguments)]
fn day_grid(
    picker: &DatePicker,
    state: &Entity<DatePickerState>,
    calendar: &Calendar,
    displayed_month: &Date,
    selection_date: &Date,
    tint: Hsla,
    theme: &AppTheme,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let today = RiseClock::now();
    let cells = month_grid(calendar, displayed_month);
    let grid_focus = state.read(cx).grid_focus.clone();
    let is_focused = grid_focus.is_focused(window);
    let mut rows = vstack(0.0).w_full();
    for (week, days) in cells.chunks(DAYS_IN_WEEK).enumerate() {
        let mut row = hstack(0.0).w_full();
        for slot in 0..DAYS_IN_WEEK {
            let Some(Some(day)) = days.get(slot).copied() else {
                row = row.child(div().flex_1().h(px(DAY_CELL_HEIGHT)));
                continue;
            };
            let enabled = picker.range.as_ref().is_none_or(|range| {
                calendar.start_of_day(&day) <= *range.end()
                    && calendar.date_by_adding(CalendarComponent::Day, 1, &calendar.start_of_day(&day)).is_some_and(|next| next > *range.start())
            });
            let is_selected = calendar.is_date_in_same_day_as(&day, selection_date);
            let is_today = calendar.is_date_in_same_day_as(&day, &today);
            let color = if is_selected || is_today { tint } else { theme.text.primary };
            let color = if enabled { color } else { theme.text.secondary.opacity(DISABLED_DAY_OPACITY) };
            let number = calendar.component(CalendarComponent::Day, &day);
            let selection = picker.selection.clone();
            let range = picker.range.clone();
            let press_calendar = calendar.clone();
            let press_focus = grid_focus.clone();
            row = row.child(
                div().flex_1().h(px(DAY_CELL_HEIGHT)).flex().items_center().justify_center().child(
                    plain(("day", week * DAYS_IN_WEEK + slot))
                        .disabled(!enabled)
                        .on_press(move |window, cx| {
                            window.focus(&press_focus, cx);
                            let current = selection.get(cx);
                            if let Some(date) = replacing_day(&press_calendar, &current, &day) {
                                commit(&selection, range.as_ref(), date, window, cx);
                            }
                        })
                        .child(
                            div()
                                .size(px(DAY_CIRCLE_SIDE))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(is_selected, |circle| {
                                    circle.bg(tint.opacity(if is_focused { SELECTED_DAY_FILL_OPACITY * 2.0 } else { SELECTED_DAY_FILL_OPACITY }))
                                })
                                .child(
                                    MainText::new(number.to_string())
                                        .px(17.0)
                                        .font_weight(if is_selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                        .color(color),
                                ),
                        ),
                ),
            );
        }
        rows = rows.child(row);
    }

    let key_selection = picker.selection.clone();
    let key_range = picker.range.clone();
    let key_calendar = calendar.clone();
    let key_state = state.clone();
    div()
        .id((picker.id.clone(), "grid"))
        .w_full()
        .track_focus(&grid_focus)
        .tab_index(0)
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            let step = match event.keystroke.key.as_str() {
                "left" => -DAY_STEP,
                "right" => DAY_STEP,
                "up" => -WEEK_STEP,
                "down" => WEEK_STEP,
                _ => return,
            };
            cx.stop_propagation();
            let current = key_selection.get(cx);
            let Some(moved) = key_calendar.date_by_adding(CalendarComponent::Day, step, &current) else {
                return;
            };
            let moved = clamped(moved, key_range.as_ref());
            let month = month_start(&key_calendar, &moved);
            key_state.update(cx, |state, cx| {
                state.displayed_month = month;
                cx.notify();
            });
            key_selection.set(moved, window, cx);
        })
        .child(rows)
}

#[allow(clippy::too_many_arguments)]
fn time_row(
    picker: &DatePicker,
    state: &Entity<DatePickerState>,
    calendar: &Calendar,
    selection_date: &Date,
    tint: Hsla,
    theme: &AppTheme,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let hour = calendar.component(CalendarComponent::Hour, selection_date);
    let minute = calendar.component(CalendarComponent::Minute, selection_date);
    hstack(8.0)
        .w_full()
        .child(spacer_min_w(0.0))
        .child(time_field(picker, state, calendar, DatePickerTimeField::Hour, hour, tint, theme, window, cx))
        .child(MainText::new(":").px(17.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.primary))
        .child(time_field(picker, state, calendar, DatePickerTimeField::Minute, minute, tint, theme, window, cx))
        .child(spacer_min_w(0.0))
}

#[allow(clippy::too_many_arguments)]
fn time_field(
    picker: &DatePicker,
    state: &Entity<DatePickerState>,
    calendar: &Calendar,
    field: DatePickerTimeField,
    value: i64,
    tint: Hsla,
    theme: &AppTheme,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    let name = match field {
        DatePickerTimeField::Hour => "hour",
        DatePickerTimeField::Minute => "minute",
    };
    let focus = match field {
        DatePickerTimeField::Hour => state.read(cx).hour_focus.clone(),
        DatePickerTimeField::Minute => state.read(cx).minute_focus.clone(),
    };
    let is_focused = focus.is_focused(window);

    let step_button = |symbol: &'static str, delta: i64| {
        let selection = picker.selection.clone();
        let range = picker.range.clone();
        let calendar = calendar.clone();
        plain((picker.id.clone(), format!("{name}-{symbol}")))
            .on_press(move |window, cx| {
                let current = selection.get(cx);
                if let Some(date) = replacing_time(&calendar, &current, field, value + delta) {
                    commit(&selection, range.as_ref(), date, window, cx);
                }
            })
            .child(chevron(symbol, TIME_CHEVRON_SIDE, theme.text.secondary))
    };

    let key_selection = picker.selection.clone();
    let key_range = picker.range.clone();
    let key_calendar = calendar.clone();
    let key_state = state.clone();
    let click_focus = focus.clone();
    let click_state = state.clone();

    vstack(4.0)
        .child(step_button("chevron.up", 1))
        .child(
            div()
                .id((picker.id.clone(), name))
                .w(px(TIME_FIELD_WIDTH))
                .h(px(TIME_FIELD_HEIGHT))
                .rounded_rect(TIME_FIELD_RADIUS)
                .flex()
                .items_center()
                .justify_center()
                .cursor_text()
                .bg(if is_focused { tint.opacity(FOCUSED_FIELD_FILL_OPACITY) } else { theme.bg._200 })
                .track_focus(&focus)
                .tab_index(0)
                .on_click(move |_, window, cx| {
                    window.focus(&click_focus, cx);
                    click_state.update(cx, |state, _| state.typed.clear());
                })
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    let key = event.keystroke.key.as_str();
                    let next = match key {
                        "up" => Some(value + 1),
                        "down" => Some(value - 1),
                        digit if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() => {
                            let typed = key_state.update(cx, |state, _| {
                                if state.typed.len() >= TYPED_TIME_DIGITS {
                                    state.typed.clear();
                                }
                                state.typed.push_str(digit);
                                state.typed.clone()
                            });
                            typed.parse::<i64>().ok()
                        }
                        _ => None,
                    };
                    let Some(next) = next else {
                        return;
                    };
                    cx.stop_propagation();
                    let current = key_selection.get(cx);
                    if let Some(date) = replacing_time(&key_calendar, &current, field, next) {
                        commit(&key_selection, key_range.as_ref(), date, window, cx);
                    }
                })
                .child(
                    MainText::new(format!("{value:02}"))
                        .px(17.0)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.text.primary)
                        .monospaced_digits(true),
                ),
        )
        .child(step_button("chevron.down", -1))
}
