use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Entity, ElementId, FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ListAlignment, ListState, ParentElement, RenderOnce, SharedString, Styled, Subscription, Window, black, div, list,
    px,
};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::scroll_chrome_shadow_ui::{ScrollChromeShadowStyle, ScrollChromeShadowStyleAnchor, ScrollChromeShadowUi};
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes};
use rise_ui::swiftui::stacks::{hstack, spacer_min_w, vstack};

use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::modules::chat::engine::messages::rise_chat_calendar::{
    RiseChatCalendarCell, RiseChatCalendarDay, RiseChatCalendarMonth,
};
use crate::modules::chat::stores::message::message_services::message_calendar_services::{
    MessageChatCalendarPresentation, MessageChatCalendarPresentationLoadState,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

pub type ChatCalendarSelectDay = Rc<dyn Fn(RiseChatCalendarDay, &mut Window, &mut App)>;
type ChatCalendarAction = Rc<dyn Fn(&mut Window, &mut App)>;

struct Metrics;

impl Metrics {
    const COLUMNS: usize = 7;
    const CELL_SPACING: f32 = 6.0;
    const ROW_SPACING: f32 = 10.0;
    const HORIZONTAL_PADDING: f32 = 16.0;
    const MONTH_SPACING: f32 = 26.0;
    const HEADER_HEIGHT: f32 = 56.0;
    const TOP_SHADOW_HEIGHT: f32 = 36.0;
    const OLDER_PREFETCH_DISTANCE: f32 = 600.0;
    const CONTENT_TOP: f32 = Self::HEADER_HEIGHT + 12.0;
    const CONTENT_BOTTOM: f32 = 32.0;
    const LOADING_ROW_HEIGHT: f32 = 44.0;
    const MONTH_TITLE_LINE: f32 = 21.0;
    const WEEKDAY_LINE: f32 = 15.0;
    const MAX_CELL_SIZE: f32 = 44.0;

    /// explicit square per day, not an aspect ratio negotiation
    /// sixty cells sizing against late thumbnails end up shaped like their labels
    fn cell_size(width: f32) -> f32 {
        let available =
            width - Self::HORIZONTAL_PADDING * 2.0 - Self::CELL_SPACING * (Self::COLUMNS as f32 - 1.0);
        (available / Self::COLUMNS as f32).floor().clamp(24.0, Self::MAX_CELL_SIZE)
    }

    /// width the seven columns actually take so title and weekdays line up with the days
    fn grid_width(cell_size: f32) -> f32 {
        cell_size * Self::COLUMNS as f32 + Self::CELL_SPACING * (Self::COLUMNS as f32 - 1.0)
    }

    /// every month row carries the spacing above it so a prepended month
    /// never changes a measured neighbour height. the first row gives it back
    fn leading_row_height(shows_loading_row: bool) -> f32 {
        if shows_loading_row {
            Self::CONTENT_TOP + Self::LOADING_ROW_HEIGHT
        } else {
            Self::CONTENT_TOP - Self::MONTH_SPACING
        }
    }

    fn month_height(month: &RiseChatCalendarMonth, cell_size: f32) -> f32 {
        let days = month.leading_blanks.max(0) as usize + month.cells.len();
        let rows = days.div_ceil(Self::COLUMNS) as f32;
        Self::MONTH_SPACING
            + Self::MONTH_TITLE_LINE
            + 14.0
            + Self::WEEKDAY_LINE
            + 14.0
            + rows * cell_size
            + (rows - 1.0).max(0.0) * Self::ROW_SPACING
    }
}

/// desktop keyboard over the grid, arrows walk the days with messages and enter opens one
struct ChatCalendarKeyboard;

#[derive(Clone)]
struct ChatCalendarKeyTarget {
    month_index: usize,
    day: RiseChatCalendarDay,
}

impl ChatCalendarKeyboard {
    const WEEK_MS: i64 = 7 * 86_400_000;
    const WEEK_TOLERANCE_MS: i64 = 12 * 3_600_000;

    fn targets(presentation: &MessageChatCalendarPresentation) -> Vec<ChatCalendarKeyTarget> {
        presentation
            .months
            .iter()
            .enumerate()
            .flat_map(|(month_index, month)| {
                month
                    .cells
                    .iter()
                    .filter(|cell| cell.is_selectable())
                    .filter_map(move |cell| cell.content.clone().map(|day| ChatCalendarKeyTarget { month_index, day }))
            })
            .collect()
    }

    /// with nothing focused any arrow lands on the newest day where the grid opens
    fn moved(targets: &[ChatCalendarKeyTarget], current: Option<i64>, key: &str) -> Option<ChatCalendarKeyTarget> {
        let Some(current) = current else {
            return targets.last().cloned();
        };
        let position = targets.iter().position(|target| target.day.day_start_ms == current);
        let Some(position) = position else {
            return targets.last().cloned();
        };
        let found = match key {
            "left" => position.checked_sub(1).and_then(|index| targets.get(index)),
            "right" => targets.get(position + 1),
            "up" => {
                let bound = current - Self::WEEK_MS + Self::WEEK_TOLERANCE_MS;
                targets.iter().rfind(|target| target.day.day_start_ms <= bound)
            }
            "down" => {
                let bound = current + Self::WEEK_MS - Self::WEEK_TOLERANCE_MS;
                targets.iter().find(|target| target.day.day_start_ms >= bound)
            }
            _ => None,
        };
        found.cloned()
    }
}

struct ChatCalendarSheetState {
    focus: FocusHandle,
    rows: ListState,
    month_keys: Vec<i64>,
    shows_loading_row: bool,
    width: f32,
    last_min_y: Option<f32>,
    focused_day: Option<i64>,
    _observer: Subscription,
}

impl ChatCalendarSheetState {
    fn sync_rows(&mut self, keys: Vec<i64>, shows_loading_row: bool) {
        if self.month_keys != keys {
            let added = keys.len().saturating_sub(self.month_keys.len());
            if !self.month_keys.is_empty() && added > 0 && keys[added..] == self.month_keys[..] {
                self.rows.splice(1..1, added);
            } else {
                self.rows.reset(keys.len() + 2);
            }
            self.month_keys = keys;
        }
        if self.shows_loading_row != shows_loading_row {
            self.shows_loading_row = shows_loading_row;
            self.rows.remeasure_items(0..1);
        }
    }

    /// scroll position in points from the top of content
    /// list measures only what it showed and opens at the bottom
    /// so rows above are summed from known geometry
    fn scroll_top(&self, heights: &[f32]) -> f32 {
        let leading = Metrics::leading_row_height(self.shows_loading_row);
        let top = self.rows.logical_scroll_top();
        if top.item_ix >= heights.len() + 2 {
            let total = leading + heights.iter().sum::<f32>() + Metrics::CONTENT_BOTTOM;
            return (total - f32::from(self.rows.viewport_bounds().size.height)).max(0.0);
        }
        let above: f32 = if top.item_ix == 0 {
            0.0
        } else {
            leading + heights.iter().take(top.item_ix - 1).sum::<f32>()
        };
        above + f32::from(top.offset_in_item)
    }
}

/// full screen calendar of a chat days, opened from a date separator
/// pressing a day closes it and opens the chat at that day start, silent days are dimmed and inert
#[derive(IntoElement)]
pub struct ChatCalendarSheet {
    id: ElementId,
    is_presented: Binding<bool>,
    on_select_day: ChatCalendarSelectDay,
    on_reached_top: ChatCalendarAction,
    on_retry: ChatCalendarAction,
    presentation_override: Option<Rc<MessageChatCalendarPresentation>>,
}

impl ChatCalendarSheet {
    pub fn new(
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        on_select_day: impl Fn(RiseChatCalendarDay, &mut Window, &mut App) + 'static,
        on_reached_top: impl Fn(&mut Window, &mut App) + 'static,
        on_retry: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            is_presented,
            on_select_day: Rc::new(on_select_day),
            on_reached_top: Rc::new(on_reached_top),
            on_retry: Rc::new(on_retry),
            presentation_override: None,
        }
    }

    pub fn presentation_override(mut self, presentation: impl Into<Option<MessageChatCalendarPresentation>>) -> Self {
        self.presentation_override = presentation.into().map(Rc::new);
        self
    }

    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    fn header(&self, theme: &AppTheme) -> impl IntoElement + use<> {
        let is_presented = self.is_presented.clone();
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(Metrics::HEADER_HEIGHT))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(Metrics::HEADER_HEIGHT + Metrics::TOP_SHADOW_HEIGHT))
                    .child(ScrollChromeShadowUi::new(Self::header_scrim_style(theme)).solid_fraction(Some(
                        Metrics::HEADER_HEIGHT / (Metrics::HEADER_HEIGHT + Metrics::TOP_SHADOW_HEIGHT),
                    ))),
            )
            .child(
                hstack(0.0)
                    .relative()
                    .size_full()
                    .px(px(Metrics::HORIZONTAL_PADDING))
                    .child(
                        LiquidGlassButton::new(self.child_id("close"), LiquidGlassButtonShape::Circle)
                            .size(40.0)
                            .accessibility_label(Some(tr("close")))
                            .on_press(move |window, cx| is_presented.set(false, window, cx))
                            .children(IconUi::sized("xmark", px(15.0), theme.text.secondary, false)),
                    )
                    .child(spacer_min_w(12.0))
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(17.0, FontWeight::SEMIBOLD))
                            .text_color(theme.text.primary)
                            .child(tr("chat_calendar_title")),
                    )
                    .child(spacer_min_w(12.0))
                    .child(div().size(px(40.0)).flex_none()),
            )
    }

    fn header_scrim_style(theme: &AppTheme) -> ScrollChromeShadowStyle {
        ScrollChromeShadowStyle {
            anchor: ScrollChromeShadowStyleAnchor::FromTop,
            extent: Metrics::HEADER_HEIGHT + Metrics::TOP_SHADOW_HEIGHT,
            overflow_below_header: 0.0,
            peak_opacity: 1.0,
            color: theme.bg._100,
            mid_location: 0.66,
            mid_opacity_factor: 0.86,
            top_plateau_end: 0.2,
            top_plateau_opacity_factor: 1.0,
            ..ScrollChromeShadowStyle::default()
        }
    }

    fn read_presentation<R>(
        presentation_override: Option<&Rc<MessageChatCalendarPresentation>>,
        cx: &mut App,
        read: impl FnOnce(&MessageChatCalendarPresentation) -> R,
    ) -> R {
        match presentation_override {
            Some(presentation) => read(presentation),
            None => {
                let service = MessageServicesStore::shared(cx);
                read(&service.read(cx).chat_calendar)
            }
        }
    }

    fn content(
        &self,
        state: &Entity<ChatCalendarSheetState>,
        theme: &AppTheme,
        cell_size: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let (load_state, months_are_empty, is_empty) =
            Self::read_presentation(self.presentation_override.as_ref(), cx, |presentation| {
                (presentation.load_state, presentation.months.is_empty(), presentation.is_empty())
            });
        match load_state {
            MessageChatCalendarPresentationLoadState::Idle => self.skeleton(theme, cell_size),
            MessageChatCalendarPresentationLoadState::Loading if months_are_empty => self.skeleton(theme, cell_size),
            MessageChatCalendarPresentationLoadState::Failed => self.error_state(theme),
            _ if is_empty => self.empty_state(theme),
            _ => self.grid(state, cell_size, window, cx),
        }
    }

    fn grid(
        &self,
        state: &Entity<ChatCalendarSheetState>,
        cell_size: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let (keys, heights, has_older, is_loading_older) =
            Self::read_presentation(self.presentation_override.as_ref(), cx, |presentation| {
                (
                    presentation.months.iter().map(|month| month.start_ms).collect::<Vec<_>>(),
                    presentation
                        .months
                        .iter()
                        .map(|month| Metrics::month_height(month, cell_size))
                        .collect::<Vec<_>>(),
                    presentation.has_older,
                    presentation.is_loading_older,
                )
            });
        let reached_top = state.update(cx, |state, _| {
            state.sync_rows(keys, is_loading_older);
            let min_y = -state.scroll_top(&heights);
            let changed = state.last_min_y != Some(min_y);
            state.last_min_y = Some(min_y);
            changed && min_y > -Metrics::OLDER_PREFETCH_DISTANCE
        });
        if reached_top && has_older && !is_loading_older {
            let on_reached_top = self.on_reached_top.clone();
            window.defer(cx, move |window, cx| on_reached_top(window, cx));
        }

        let rows = state.read(cx).rows.clone();
        let month_count = heights.len();
        let row_state = state.clone();
        let presentation_override = self.presentation_override.clone();
        let on_select_day = self.on_select_day.clone();
        let sheet_id = self.id.clone();
        let host = window.current_view();
        div()
            .relative()
            .size_full()
            .child(
                list(rows, move |index, _window, cx| {
                    let (shows_loading_row, focused_day) = {
                        let state = row_state.read(cx);
                        (state.shows_loading_row, state.focused_day)
                    };
                    if index == 0 {
                        return Self::leading_row(shows_loading_row, cx);
                    }
                    if index > month_count {
                        return div().w_full().h(px(Metrics::CONTENT_BOTTOM)).into_any_element();
                    }
                    let month =
                        Self::read_presentation(presentation_override.as_ref(), cx, |presentation| {
                            let weekdays = presentation.weekday_symbols.clone();
                            presentation.months.get(index - 1).cloned().map(|month| (month, weekdays))
                        });
                    let Some((month, weekday_symbols)) = month else {
                        return div().into_any_element();
                    };
                    let theme = rise_ui::theme(cx).clone();
                    Self::month_section(&sheet_id, month, &weekday_symbols, &theme, cell_size, focused_day, &on_select_day)
                })
                .size_full(),
            )
            .child(measure(self.child_id("grid-bounds"), move |_, _, cx| cx.notify(host)))
            .into_any_element()
    }

    fn leading_row(shows_loading_row: bool, cx: &mut App) -> AnyElement {
        if !shows_loading_row {
            return div().w_full().h(px(Metrics::leading_row_height(false))).into_any_element();
        }
        let tint = rise_ui::theme(cx).text.secondary;
        div()
            .w_full()
            .pt(px(Metrics::CONTENT_TOP))
            .child(
                div()
                    .w_full()
                    .h(px(Metrics::LOADING_ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(progress_view().tint(tint)),
            )
            .into_any_element()
    }

    fn month_section(
        sheet_id: &ElementId,
        month: RiseChatCalendarMonth,
        weekday_symbols: &[String],
        theme: &AppTheme,
        cell_size: f32,
        focused_day: Option<i64>,
        on_select_day: &ChatCalendarSelectDay,
    ) -> AnyElement {
        let mut slots: Vec<AnyElement> = Vec::with_capacity(month.leading_blanks.max(0) as usize + month.cells.len());
        for _ in 0..month.leading_blanks.max(0) {
            slots.push(Self::blank(cell_size));
        }
        for cell in month.cells {
            let is_focused = focused_day == Some(cell.day_start_ms);
            let id = ElementId::NamedChild(
                Arc::new(sheet_id.clone()),
                format!("day-{}", cell.day_start_ms).into(),
            );
            slots.push(
                ChatCalendarDayCell::new(id, cell, cell_size, on_select_day.clone())
                    .is_keyboard_focused(is_focused)
                    .into_any_element(),
            );
        }
        while slots.len() % Metrics::COLUMNS != 0 {
            slots.push(Self::blank(cell_size));
        }
        let mut grid_rows = vstack(Metrics::ROW_SPACING).w_full();
        let mut slots = slots.into_iter();
        loop {
            let row: Vec<AnyElement> = slots.by_ref().take(Metrics::COLUMNS).collect();
            if row.is_empty() {
                break;
            }
            grid_rows = grid_rows.child(hstack(Metrics::CELL_SPACING).w_full().justify_center().children(row));
        }

        div()
            .w_full()
            .pt(px(Metrics::MONTH_SPACING))
            .px(px(Metrics::HORIZONTAL_PADDING))
            .flex()
            .justify_center()
            .child(
                vstack(14.0)
                    .w(px(Metrics::grid_width(cell_size)))
                    .flex_none()
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(17.0, FontWeight::SEMIBOLD))
                            .text_color(theme.text.primary)
                            .child(month.title),
                    )
                    .child(hstack(Metrics::CELL_SPACING).w_full().children(weekday_symbols.iter().map(|symbol| {
                        div()
                            .flex_1()
                            .flex()
                            .justify_center()
                            .swiftui_font(Font::system_weight(12.0, FontWeight::MEDIUM))
                            .text_color(theme.text.secondary.opacity(0.7))
                            .child(symbol.clone())
                    })))
                    .child(grid_rows),
            )
            .into_any_element()
    }

    fn blank(cell_size: f32) -> AnyElement {
        div().size(px(cell_size)).flex_none().into_any_element()
    }

    fn skeleton(&self, _theme: &AppTheme, cell_size: f32) -> AnyElement {
        let rows = (0..35_usize).collect::<Vec<_>>();
        let rows = rows.chunks(Metrics::COLUMNS).map(|row| {
            hstack(Metrics::CELL_SPACING)
                .w_full()
                .justify_center()
                .children(row.iter().map(|_| SkeletonCircle::new(cell_size)))
        });
        let grid = vstack(14.0)
            .w(px(Metrics::grid_width(cell_size)))
            .flex_none()
            .child(SkeletonLine::new(130.0_f32, 18.0))
            .child(
                hstack(Metrics::CELL_SPACING).w_full().children(
                    (0..Metrics::COLUMNS)
                        .map(|_| div().flex_1().flex().justify_center().child(SkeletonLine::new(14.0_f32, 12.0))),
                ),
            )
            .child(vstack(Metrics::ROW_SPACING).w_full().children(rows));
        div()
            .w_full()
            .px(px(Metrics::HORIZONTAL_PADDING))
            .pt(px(Metrics::CONTENT_TOP))
            .flex()
            .justify_center()
            .child(grid)
            .skeleton_shimmer(None)
            .into_any_element()
    }

    fn empty_state(&self, theme: &AppTheme) -> AnyElement {
        vstack(8.0)
            .size_full()
            .justify_center()
            .px(px(32.0))
            .children(IconUi::sized("calendar", px(34.0), theme.text.secondary.opacity(0.5), false))
            .child(
                div()
                    .swiftui_font(Font::system_weight(15.0, FontWeight::MEDIUM))
                    .text_color(theme.text.secondary)
                    .text_center()
                    .child(tr("chat_calendar_empty")),
            )
            .into_any_element()
    }

    fn error_state(&self, theme: &AppTheme) -> AnyElement {
        let on_retry = self.on_retry.clone();
        vstack(12.0)
            .size_full()
            .justify_center()
            .px(px(32.0))
            .child(
                div()
                    .swiftui_font(Font::system_weight(15.0, FontWeight::MEDIUM))
                    .text_color(theme.text.secondary)
                    .text_center()
                    .child(tr("chat_calendar_error")),
            )
            .child(
                plain(self.child_id("retry"))
                    .on_press(move |window, cx| on_retry(window, cx))
                    .swiftui_font(Font::system_weight(15.0, FontWeight::SEMIBOLD))
                    .text_color(theme.primary._100)
                    .child(tr("retry")),
            )
            .into_any_element()
    }
}

impl RenderOnce for ChatCalendarSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let host = window.current_view();
        let state = window.use_keyed_state(self.child_id("state"), cx, move |window, cx| {
            let focus = cx.focus_handle();
            let service = MessageServicesStore::shared(cx);
            let observer = App::observe(cx, &service, move |_, cx: &mut App| cx.notify(host));
            let initial_focus = focus.clone();
            window.defer(cx, move |window, cx| window.focus(&initial_focus, cx));
            ChatCalendarSheetState {
                focus,
                rows: ListState::new(0, ListAlignment::Bottom, px(600.0)),
                month_keys: Vec::new(),
                shows_loading_row: false,
                width: 0.0,
                last_min_y: None,
                focused_day: None,
                _observer: observer,
            }
        });

        let cell_size = Metrics::cell_size(state.read(cx).width);
        let content = self.content(&state, &theme, cell_size, window, cx);
        let header = self.header(&theme);
        let focus = state.read(cx).focus.clone();
        let root_id = self.child_id("root");
        let bounds_id = self.child_id("bounds");

        let key_state = state.clone();
        let key_focus = focus.clone();
        let on_select_day = self.on_select_day.clone();
        let presentation_override = self.presentation_override.clone();
        let handle_key = move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            if !key_focus.is_focused(window) || event.keystroke.modifiers.modified() {
                return;
            }
            let key = event.keystroke.key.as_str();
            if !matches!(key, "left" | "right" | "up" | "down" | "enter") {
                return;
            }
            let targets =
                Self::read_presentation(presentation_override.as_ref(), cx, ChatCalendarKeyboard::targets);
            let (current, rows) = {
                let state = key_state.read(cx);
                (state.focused_day, state.rows.clone())
            };
            if key == "enter" {
                let Some(target) = targets.into_iter().find(|target| Some(target.day.day_start_ms) == current) else {
                    return;
                };
                cx.stop_propagation();
                on_select_day(target.day, window, cx);
                return;
            }
            cx.stop_propagation();
            let Some(target) = ChatCalendarKeyboard::moved(&targets, current, key) else {
                return;
            };
            rows.scroll_to_reveal_item(target.month_index + 1);
            key_state.update(cx, |state, _| state.focused_day = Some(target.day.day_start_ms));
            cx.notify(host);
        };

        let measured_state = state.clone();
        div()
            .id(root_id)
            .track_focus(&focus)
            .on_key_down(handle_key)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg._100)
            .child(content)
            .child(header)
            .child(measure(bounds_id, move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                measured_state.update(cx, |state, _| state.width = width);
                cx.notify(host);
            }))
    }
}

#[derive(IntoElement)]
pub struct ChatCalendarDayCell {
    id: ElementId,
    cell: RiseChatCalendarCell,
    size: f32,
    on_select: ChatCalendarSelectDay,
    is_keyboard_focused: bool,
}

impl ChatCalendarDayCell {
    pub fn new(id: impl Into<ElementId>, cell: RiseChatCalendarCell, size: f32, on_select: ChatCalendarSelectDay) -> Self {
        Self { id: id.into(), cell, size, on_select, is_keyboard_focused: false }
    }

    /// the day the sheet arrows stand on, a pointer has its own feedback but a keyboard needs this
    pub fn is_keyboard_focused(mut self, is_keyboard_focused: bool) -> Self {
        self.is_keyboard_focused = is_keyboard_focused;
        self
    }

    fn number_color(&self, theme: &AppTheme) -> gpui::Hsla {
        if self.cell.content.is_some() {
            return theme.text.primary;
        }
        theme.text.secondary.opacity(if self.cell.is_future { 0.3 } else { 0.45 })
    }
}

impl RenderOnce for ChatCalendarDayCell {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let size = self.size;
        let number_color = self.number_color(theme);
        let picture_id = ElementId::NamedChild(Arc::new(self.id.clone()), "picture".into());
        let preview = self.cell.content.as_ref().and_then(|content| content.preview_url.clone());
        let has_content = self.cell.content.is_some();
        let content = self.cell.content.clone();
        let on_select = self.on_select.clone();
        let weight = if self.cell.is_today { FontWeight::BOLD } else { FontWeight::MEDIUM };

        plain(self.id)
            .disabled(!self.cell.is_selectable())
            .on_press(move |window, cx| {
                let Some(content) = content.clone() else {
                    return;
                };
                on_select(content, window, cx);
            })
            .relative()
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .when(has_content, |cell| cell.child(div().absolute().inset_0().rounded_full().bg(theme.bg._300)))
            .when_some(preview, |cell, preview| {
                cell.child(
                    div().absolute().inset_0().clip_shape(Shape::Circle).child(
                        CleverImageUi::new(preview)
                            .id(picture_id)
                            .content_mode(ContentMode::Fill)
                            .corner_radius(size / 2.0)
                            .shows_progress_indicator(false)
                            .maximum_decode_pixel_dimension(192_i64)
                            .shows_failure_icon(false),
                    ),
                )
                .child(div().absolute().inset_0().rounded_full().bg(black().opacity(0.35)))
            })
            .when(self.cell.is_today, |cell| {
                cell.child(div().absolute().inset_0().rounded_full().stroke_border(theme.primary._100, 1.5))
            })
            .when(self.is_keyboard_focused, |cell| {
                cell.child(div().absolute().inset_0().rounded_full().stroke_border(theme.text.primary, 1.5))
            })
            .child(
                div()
                    .relative()
                    .flex_none()
                    .swiftui_font(Font::system_weight(15.0, weight))
                    .text_color(number_color)
                    .child(self.cell.day_number.to_string()),
            )
    }
}
