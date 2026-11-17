use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight, Hsla, KeyDownEvent, ListAlignment,
    ListState, MouseButton, ObjectFit, PromptButton, PromptLevel, Rgba, Subscription, Window, img, list,
};
use rise_canon::rise_theme_tokens::parse_color;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::{FormatArg, format_printf};
use rise_theme::{AppTheme, RiseThemeAppearance, ThemePalette};
use rise_theme::avatar_palette::AvatarPalette;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::IconUi;
use rise_ui::input_ui::input_ui_state::{InputUiEvent, InputUiState};
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::stacks::ZStack;
use rise_ui::swiftui::motion::{ease_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Edge, Presence, Transition, UnitPoint};

use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::share_extension::entry::share_avatar_image_cache::ShareAvatarImageCache;
use crate::modules::share_extension::entry::share_extension_models::{
    ShareDestination, ShareDestinationKind, ShareTopic,
};
use crate::modules::share_extension::entry::share_extension_strings::ShareStrings;
use crate::modules::share_extension::entry::share_extension_view_model::{
    ShareExtensionViewModel, ShareExtensionViewModelActivity,
};
use crate::modules::share_extension::entry::share_send_progress_view::ShareSendProgressView;
use crate::modules::share_extension::shared::rise_share_models::RiseShareTopicGroup;

const AVATAR_SIZE: f32 = 62.0;
const GRID_SPACING: f32 = 10.0;
const GRID_ROW_SPACING: f32 = 17.0;
const TILE_LABEL_MAXIMUM_WIDTH: f32 = 100.0;
const SHEET_MAXIMUM_WIDTH: f32 = 620.0;
const SHEET_SIDE_INSET: f32 = 20.0;
const BUSY_SHEET_HEIGHT: f32 = 146.0;
const SCRIM_OPACITY: f32 = 0.58;
const LIST_OVERDRAW: f32 = 200.0;
const LOADING_TILE_COUNT: usize = 8;
const GENERATED_AVATAR_SEGMENT: &str = "/avatars/generated/";

/// share layer is a modal of the app window so it follows the app theme, light or dark
pub enum ShareSurface {}

/// share layer colors for the current theme, captured once per frame so tiles dont need a context
#[derive(Clone, Copy, Debug)]
struct ShareSurfaceColors {
    sheet: Hsla,
    elevated: Hsla,
    input: Hsla,
    search_input: Hsla,
    divider: Hsla,
    avatar_loading: Hsla,
    hairline: Hsla,
    accent: Hsla,
    label_primary: Hsla,
    label_secondary: Hsla,
}

impl ShareSurfaceColors {
    fn of(theme: &AppTheme) -> Self {
        let is_dark = theme.appearance == RiseThemeAppearance::Dark;
        Self {
            sheet: theme.bg._100,
            elevated: theme.bg._200,
            input: theme.bg._200,
            search_input: theme.bg._300,
            divider: theme.border._100,
            avatar_loading: theme.bg._300,
            hairline: if is_dark { white_opacity(0.035) } else { black_opacity(0.06) },
            accent: theme.primary._100,
            label_primary: theme.text.primary,
            label_secondary: theme.text.secondary,
        }
    }

    fn fallback() -> Self {
        let palette = ThemePalette::default_dark();
        let color = |value: &str| {
            let (red, green, blue, _) = parse_color(value).unit_components();
            rgb_unit(red as f32, green as f32, blue as f32)
        };
        Self {
            sheet: rgb_unit(0.105, 0.105, 0.115),
            elevated: rgb_unit(0.145, 0.145, 0.158),
            input: rgb_unit(0.075, 0.075, 0.082),
            search_input: rgb_unit(0.205, 0.205, 0.220),
            divider: white_opacity(0.075),
            avatar_loading: rgb_unit(0.22, 0.22, 0.235),
            hairline: white_opacity(0.035),
            accent: color(&palette.primary_100),
            label_primary: white_opacity(1.0),
            label_secondary: Rgba {
                r: 235.0 / 255.0,
                g: 235.0 / 255.0,
                b: 245.0 / 255.0,
                a: 0.6,
            }
            .into(),
        }
    }
}

thread_local! {
    static SHARE_SURFACE_COLORS: Cell<ShareSurfaceColors> = Cell::new(ShareSurfaceColors::fallback());
}

impl ShareSurface {
    fn colors() -> ShareSurfaceColors {
        SHARE_SURFACE_COLORS.with(Cell::get)
    }

    /// captures the app theme for this frame and returns which appearance it was
    pub fn capture(cx: &App) -> RiseThemeAppearance {
        let theme = rise_ui::theme(cx);
        SHARE_SURFACE_COLORS.with(|colors| colors.set(ShareSurfaceColors::of(theme)));
        theme.appearance
    }

    pub fn sheet() -> Hsla {
        Self::colors().sheet
    }

    pub fn elevated() -> Hsla {
        Self::colors().elevated
    }

    pub fn input() -> Hsla {
        Self::colors().input
    }

    pub fn search_input() -> Hsla {
        Self::colors().search_input
    }

    pub fn divider() -> Hsla {
        Self::colors().divider
    }

    pub fn avatar_loading() -> Hsla {
        Self::colors().avatar_loading
    }

    /// faint edge stroke of the sheets
    pub fn hairline() -> Hsla {
        Self::colors().hairline
    }

    pub fn accent() -> Hsla {
        Self::colors().accent
    }

    /// main text color of the layer, also the send ring
    pub fn label() -> Hsla {
        Self::colors().label_primary
    }

    /// secondary text color of the layer
    pub fn secondary_label() -> Hsla {
        Self::colors().label_secondary
    }
}

fn rgb_unit(red: f32, green: f32, blue: f32) -> Hsla {
    Rgba {
        r: red,
        g: green,
        b: blue,
        a: 1.0,
    }
    .into()
}

fn white_opacity(opacity: f32) -> Hsla {
    Hsla {
        h: 0.0,
        s: 0.0,
        l: 1.0,
        a: opacity,
    }
}

fn black_opacity(opacity: f32) -> Hsla {
    Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.0,
        a: opacity,
    }
}

fn label_primary() -> Hsla {
    ShareSurface::colors().label_primary
}

fn label_secondary() -> Hsla {
    ShareSurface::colors().label_secondary
}

/// ink on accent and avatar fills, white in both themes
fn on_fill() -> Hsla {
    white_opacity(1.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ShareExtensionViewFocusTarget {
    Search,
    Comment,
}

/// one row of the lazy chat list, grid is split into rows so only visible ones are drawn
#[derive(Clone)]
enum ShareDestinationRow {
    RecentHeader,
    ValidationBanner,
    LoadingGrid,
    EmptyState,
    Tiles(Vec<ShareDestinationCell>),
}

#[derive(Clone)]
enum ShareDestinationCell {
    Story,
    Destination { index: usize, destination: ShareDestination },
}

pub struct ShareExtensionView {
    model: Entity<ShareExtensionViewModel>,
    focus_handle: FocusHandle,
    search_input: Entity<InputUiState>,
    comment_input: Entity<InputUiState>,
    destinations_list: ListState,
    destinations_list_key: String,
    busy_presence: Presence<()>,
    picker_presence: Presence<()>,
    footer_presence: Presence<()>,
    topics_presence: Presence<ShareDestination>,
    destinations_presence: Presence<()>,
    picker_height: Animated<f32>,
    last_is_busy: bool,
    last_has_selection: bool,
    last_topic_destination_id: Option<String>,
    is_alert_presented: bool,
    keyboard_index: Option<usize>,
    input_appearance: RiseThemeAppearance,
    _subscriptions: Vec<Subscription>,
}

impl ShareExtensionView {
    pub fn new(model: Entity<ShareExtensionViewModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input_appearance = ShareSurface::capture(cx);
        let search_input = cx.new(|cx| {
            let mut state = InputUiState::single_line(cx);
            state.set_escape_to_layer(false);
            state.set_placeholder(ShareStrings::search(), cx);
            state.set_chromeless(true, cx);
            state.set_text_color(Some(label_primary()), cx);
            state.set_placeholder_color(Some(label_secondary()), cx);
            state.set_caret_color(Some(ShareSurface::accent()), cx);
            state
        });
        let comment_limit = usize::try_from(model.read(cx).configuration.maximum_comment_length).ok();
        let comment_input = cx.new(|cx| {
            let mut state = InputUiState::multi_line(cx);
            state.set_escape_to_layer(false);
            state.set_max_length(comment_limit, cx);
            state.set_placeholder(ShareStrings::comment(), cx);
            state.set_chromeless(true, cx);
            state.set_wraps(true, cx);
            state.set_max_visible_lines(4, cx);
            state.set_text_color(Some(label_primary()), cx);
            state.set_placeholder_color(Some(label_secondary()), cx);
            state.set_caret_color(Some(ShareSurface::accent()), cx);
            state
        });
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| this.model_did_change(window, cx)),
            cx.subscribe_in(&search_input, window, |this, input, event: &InputUiEvent, window, cx| {
                match event {
                    InputUiEvent::Cancelled => return this.handle_escape(window, cx),
                    InputUiEvent::Submitted => {
                        this.submit_keyboard_destination(cx);
                        return;
                    }
                    InputUiEvent::Changed => {}
                }
                let text = input.read(cx).text().to_owned();
                this.model.update(cx, |model, cx| {
                    if model.search_text == text {
                        return;
                    }
                    model.set_search_text(text, cx);
                    model.search_did_change(cx);
                });
            }),
            cx.subscribe_in(&comment_input, window, |this, input, event: &InputUiEvent, window, cx| {
                if *event == InputUiEvent::Cancelled {
                    return this.handle_escape(window, cx);
                }
                if *event != InputUiEvent::Changed {
                    return;
                }
                let text = input.read(cx).text().to_owned();
                this.model.update(cx, |model, cx| model.set_comment(&text, cx));
            }),
        ];
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let picker_presence = Presence::new(Some(()))
            .with_transition(Transition::scale_anchored(0.96, UnitPoint::BOTTOM).combined(Transition::opacity()));
        let busy_presence = Presence::new(None)
            .with_transition(Transition::scale_anchored(0.84, UnitPoint::BOTTOM).combined(Transition::opacity()));
        let footer_presence = Presence::new(None)
            .with_transition(Transition::move_edge(Edge::Bottom).combined(Transition::opacity()));
        let topics_presence = Presence::new(None).with_transition(Transition::asymmetric(
            Transition::move_edge(Edge::Trailing).combined(Transition::opacity()),
            Transition::move_edge(Edge::Leading).combined(Transition::opacity()),
        ));
        let destinations_presence = Presence::new(Some(())).with_transition(Transition::asymmetric(
            Transition::move_edge(Edge::Leading).combined(Transition::opacity()),
            Transition::move_edge(Edge::Trailing).combined(Transition::opacity()),
        ));
        model.update(cx, |model, cx| model.start(cx));
        Self {
            model,
            focus_handle,
            search_input,
            comment_input,
            destinations_list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            destinations_list_key: String::new(),
            busy_presence,
            picker_presence,
            footer_presence,
            topics_presence,
            destinations_presence,
            picker_height: Animated::new(0.0),
            last_is_busy: false,
            last_has_selection: false,
            last_topic_destination_id: None,
            is_alert_presented: false,
            keyboard_index: None,
            input_appearance,
            _subscriptions: subscriptions,
        }
    }

    fn focused_target(&self, window: &Window, cx: &App) -> Option<ShareExtensionViewFocusTarget> {
        if self.search_input.focus_handle(cx).is_focused(window) {
            return Some(ShareExtensionViewFocusTarget::Search);
        }
        if self.comment_input.focus_handle(cx).is_focused(window) {
            return Some(ShareExtensionViewFocusTarget::Comment);
        }
        None
    }

    fn model_did_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let is_busy = model.is_busy();
        let has_selection = model.selected_count() > 0;
        let topic_destination = model.topic_destination().cloned();
        let search_text = model.search_text.clone();
        let comment = model.comment.clone();
        let is_retry_locked = model.is_retry_locked();
        let alert = (model.error_message().is_some() || model.error_title().is_some()).then(|| {
            (
                model.error_title().map(str::to_owned).unwrap_or_else(ShareStrings::error_title),
                model.error_message().map(str::to_owned),
                model.has_partial_delivery(),
            )
        });
        let displayed_count = model.displayed_destinations().len();
        let now = RiseClock::monotonic();
        let reduce = reduce_motion(cx);

        if is_busy != self.last_is_busy {
            self.last_is_busy = is_busy;
            let animation = spring(0.42, 0.88);
            self.busy_presence.set(is_busy.then_some(()), animation.clone(), now, reduce);
            self.picker_presence.set((!is_busy).then_some(()), animation, now, reduce);
            if is_busy {
                window.focus(&self.focus_handle, cx);
            }
        }
        if has_selection != self.last_has_selection {
            self.last_has_selection = has_selection;
            self.footer_presence
                .set(has_selection.then_some(()), spring(0.34, 0.92), now, reduce);
        }
        let topic_destination_id = topic_destination.as_ref().map(|destination| destination.id.clone());
        if topic_destination_id != self.last_topic_destination_id {
            self.last_topic_destination_id = topic_destination_id;
            let animation = spring(0.38, 0.90);
            self.destinations_presence
                .set(topic_destination.is_none().then_some(()), animation.clone(), now, reduce);
            self.topics_presence.set(topic_destination, animation, now, reduce);
            self.keyboard_index = None;
        }
        if self.search_input.read(cx).text() != search_text {
            self.search_input.update(cx, |input, cx| input.reset(search_text, cx));
        }
        if self.comment_input.read(cx).text() != comment {
            self.comment_input.update(cx, |input, cx| input.reset(comment, cx));
        }
        self.search_input.update(cx, |input, cx| input.set_disabled(is_retry_locked, cx));
        if self.keyboard_index.is_some_and(|index| index >= displayed_count) {
            self.keyboard_index = None;
        }
        match alert {
            Some((title, message, has_partial_delivery)) if !self.is_alert_presented => {
                self.is_alert_presented = true;
                self.present_alert(title, message, has_partial_delivery, window, cx);
            }
            None => self.is_alert_presented = false,
            _ => {}
        }
        cx.notify();
    }

    /// shows the error alert with one button, after partial delivery it closes the layer
    fn present_alert(
        &mut self,
        title: String,
        message: Option<String>,
        has_partial_delivery: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = window.prompt(
            PromptLevel::Info,
            &title,
            message.as_deref(),
            &[PromptButton::cancel(ShareStrings::dismiss())],
            cx,
        );
        let model = self.model.clone();
        cx.spawn(async move |this, cx| {
            let _ = answer.await;
            let _ = this.update(cx, |view, _| view.is_alert_presented = false);
            model.update(cx, |model, cx| {
                if has_partial_delivery {
                    model.finish_partial_delivery(cx);
                } else {
                    model.dismiss_error(cx);
                }
            });
        })
        .detach();
    }

    fn cancel_if_allowed(&mut self, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| {
            if !model.is_busy() && model.can_cancel() {
                model.cancel(cx);
            }
        });
    }

    fn activate_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.set_search_active(true, cx));
        if self.model.read(cx).is_searching() {
            let handle = self.search_input.focus_handle(cx);
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
    }

    fn deactivate_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        self.model.update(cx, |model, cx| model.set_search_active(false, cx));
    }

    /// escape closes topics first then search
    /// then the layer, same as a tap on the backdrop
    fn handle_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let has_topics = model.topic_destination().is_some();
        let is_searching = model.is_searching();
        if has_topics {
            self.model.update(cx, |model, cx| model.close_topics(cx));
        } else if is_searching {
            self.deactivate_search(window, cx);
        } else {
            self.cancel_if_allowed(cx);
        }
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let secondary = keystroke.modifiers.secondary();
        let composing = [&self.search_input, &self.comment_input]
            .iter()
            .any(|input| input.read(cx).marked_range().is_some());
        if composing {
            return;
        }
        let walks_list = self.focus_handle.is_focused(window)
            || self.focused_target(window, cx) == Some(ShareExtensionViewFocusTarget::Search);
        let walks_columns = self.focus_handle.is_focused(window);
        match keystroke.key.as_str() {
            "escape" => {
                self.handle_escape(window, cx);
                cx.stop_propagation();
            }
            "f" if secondary => {
                if self.model.read(cx).topic_destination().is_none() {
                    self.activate_search(window, cx);
                }
                cx.stop_propagation();
            }
            "enter" if secondary => {
                self.model.update(cx, |model, cx| model.send(cx));
                cx.stop_propagation();
            }
            "enter" if walks_list && self.keyboard_index.is_some() => {
                if self.submit_keyboard_destination(cx) {
                    cx.stop_propagation();
                }
            }
            "up" | "down" if walks_list => {
                self.move_keyboard_index(keystroke.key.as_str(), window, cx);
                cx.stop_propagation();
            }
            "left" | "right" if walks_columns => {
                self.move_keyboard_index(keystroke.key.as_str(), window, cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    /// enter on the highlighted chat, false if nothing is highlighted
    fn submit_keyboard_destination(&mut self, cx: &mut Context<Self>) -> bool {
        let target = self
            .keyboard_index
            .and_then(|index| self.model.read(cx).displayed_destinations().get(index).cloned());
        let Some(destination) = target else { return false };
        self.model.update(cx, |model, cx| model.tap_destination(destination, cx));
        true
    }

    fn move_keyboard_index(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        if model.topic_destination().is_some() {
            return;
        }
        let count = model.displayed_destinations().len();
        if count == 0 {
            return;
        }
        let columns = Self::grid_layout(window).0 as isize;
        let step: isize = match key {
            "left" => -1,
            "right" => 1,
            "up" => -columns,
            _ => columns,
        };
        let next = match self.keyboard_index {
            None => 0,
            Some(index) => (index as isize + step).clamp(0, count as isize - 1) as usize,
        };
        self.keyboard_index = Some(next);
        if let Some(row) = self.row_of_destination(next, cx, window) {
            self.destinations_list.scroll_to_reveal_item(row);
        }
        cx.notify();
    }

    fn row_of_destination(&self, index: usize, cx: &App, window: &Window) -> Option<usize> {
        self.destination_rows(window, cx).iter().position(|row| match row {
            ShareDestinationRow::Tiles(cells) => cells.iter().any(|cell| {
                matches!(cell, ShareDestinationCell::Destination { index: cell_index, .. } if *cell_index == index)
            }),
            _ => false,
        })
    }

    /// column count and width for the grid inside the sheet padding
    fn grid_layout(window: &Window) -> (usize, f32) {
        let minimum = 102f32.min(70f32.max(AVATAR_SIZE + 10.0));
        let maximum = minimum.max(100.0);
        let width = Self::sheet_width(window) - 28.0;
        let columns = (((width + GRID_SPACING) / (minimum + GRID_SPACING)).floor() as usize).max(1);
        let column_width = ((width - GRID_SPACING * (columns as f32 - 1.0)) / columns as f32).min(maximum);
        (columns, column_width)
    }

    fn sheet_width(window: &Window) -> f32 {
        let viewport = window.viewport_size();
        SHEET_MAXIMUM_WIDTH.min(f32::from(viewport.width) - SHEET_SIDE_INSET)
    }

    fn picker_height_target(window: &Window) -> f32 {
        let height = f32::from(window.viewport_size().height);
        570f32.min(440f32.max(height * 0.66))
    }

    fn destination_rows(&self, window: &Window, cx: &App) -> Vec<ShareDestinationRow> {
        let model = self.model.read(cx);
        let mut rows = Vec::new();
        if model.is_searching() && model.search_text.is_empty() {
            rows.push(ShareDestinationRow::RecentHeader);
        }
        if !model.is_live_validated() && model.payload().is_some() && !model.is_searching() {
            rows.push(ShareDestinationRow::ValidationBanner);
        }
        if model.activity() == ShareExtensionViewModelActivity::Loading && model.displayed_destinations().is_empty() {
            rows.push(ShareDestinationRow::LoadingGrid);
        } else if model.displayed_destinations().is_empty() && (model.is_searching() || !model.can_share_to_story()) {
            rows.push(ShareDestinationRow::EmptyState);
        } else {
            let mut cells = Vec::new();
            if model.can_share_to_story() && !model.is_searching() {
                cells.push(ShareDestinationCell::Story);
            }
            cells.extend(
                model
                    .displayed_destinations()
                    .iter()
                    .enumerate()
                    .map(|(index, destination)| ShareDestinationCell::Destination {
                        index,
                        destination: destination.clone(),
                    }),
            );
            let (columns, _) = Self::grid_layout(window);
            rows.extend(cells.chunks(columns).map(|chunk| ShareDestinationRow::Tiles(chunk.to_vec())));
        }
        rows
    }

    fn picker_sheet(&mut self, now: MotionTime, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let model = self.model.read(cx);
        let account_line = model
            .account()
            .map(|account| Self::account_line(&account.display_name));
        let header = self.header(window, cx);
        let mut content = zstack(Alignment::Top).w_full().flex_1().min_h(px(0.)).overflow_hidden();
        if let Some(topics) = self.topics_presence.render_transitioned(now, window, |destination| {
            Self::topic_content(&self.model, destination, window, cx).size_full()
        }) {
            content = content.child(topics);
        }
        let destination_content = self.destination_content(window, cx);
        if let Some(destinations) = self
            .destinations_presence
            .render_transitioned(now, window, move |_| div().size_full().child(destination_content))
        {
            content = content.child(destinations);
        }
        let footer = self.share_footer(cx);
        let footer = self
            .footer_presence
            .render_transitioned(now, window, move |_| div().w_full().child(footer));
        vstack(0.)
            .relative()
            .size_full()
            .bg(ShareSurface::sheet())
            .clip_shape(Shape::RoundedRectangle {
                corner_radius: 28.0,
                style: RoundedCornerStyle::Continuous,
            })
            .child(div().w_full().px(px(18.)).pt(px(14.)).pb(px(11.)).child(header))
            .children(account_line.map(|line| {
                div()
                    .w_full()
                    .px(px(18.))
                    .pb(px(10.))
                    .text_center()
                    .swiftui_font(Font::caption())
                    .text_color(label_secondary())
                    .child(line)
            }))
            .child(content)
            .children(footer)
            .child(centred_stroke(
                ShareSurface::hairline(),
                1.0,
                Shape::RoundedRectangle {
                    corner_radius: 28.0,
                    style: RoundedCornerStyle::Continuous,
                },
            ))
            .into_any_element()
    }

    fn header(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let model = self.model.read(cx);
        if let Some(destination) = model.topic_destination() {
            let title = destination.title.clone();
            return hstack(12.)
                .w_full()
                .child(
                    plain("share-topics-back")
                        .min_w(px(76.))
                        .min_h(px(44.))
                        .on_press({
                            let model = self.model.clone();
                            move |_, cx| model.update(cx, |model, cx| model.close_topics(cx))
                        })
                        .child(
                            hstack(5.)
                                .h(px(44.))
                                .text_color(ShareSurface::accent())
                                .children(IconUi::sized("chevron.left", px(18.), ShareSurface::accent(), false))
                                .child(div().swiftui_font(Font::system_size(16.)).child(ShareStrings::back())),
                        ),
                )
                .child(spacer_min_w(0.))
                .child(
                    vstack(1.)
                        .child(
                            div()
                                .swiftui_font(Font::system_weight(19., FontWeight::BOLD))
                                .text_color(label_primary())
                                .line_clamp(1)
                                .text_ellipsis()
                                .child(title),
                        )
                        .child(
                            div()
                                .swiftui_font(Font::caption())
                                .text_color(label_secondary())
                                .child(ShareStrings::select_topic()),
                        ),
                )
                .child(spacer_min_w(0.))
                .child(self.account_avatar(cx))
                .into_any_element();
        }
        if model.is_searching() {
            let has_text = !model.search_text.is_empty();
            let view = cx.entity().downgrade();
            let clear_input = self.search_input.clone();
            return hstack(10.)
                .w_full()
                .child(
                    hstack(8.)
                        .flex_1()
                        .px(px(12.))
                        .min_h(px(44.))
                        .bg(ShareSurface::search_input())
                        .continuous_corners(13.)
                        .children(IconUi::sized("magnifyingglass", px(17.), label_secondary(), false))
                        .child(div().flex_1().child(self.search_input.clone()))
                        .when(has_text, |row| {
                            row.child(
                                div()
                                    .id("share-search-clear")
                                    .tooltip_label(ShareStrings::clear_search())
                                    .child(
                                        plain("share-search-clear-button")
                                            .on_press(move |_, cx| {
                                                clear_input.update(cx, |input, cx| input.reset(String::new(), cx));
                                            })
                                            .children(IconUi::sized(
                                                "xmark.circle.fill",
                                                px(17.),
                                                label_secondary(),
                                                true,
                                            )),
                                    ),
                            )
                        }),
                )
                .child(
                    plain("share-search-cancel")
                        .on_press(move |window, cx| {
                            let _ = view.update(cx, |view, cx| view.deactivate_search(window, cx));
                        })
                        .swiftui_font(Font::system_weight(16., FontWeight::MEDIUM))
                        .text_color(ShareSurface::accent())
                        .child(ShareStrings::cancel()),
                )
                .into_any_element();
        }
        let subtitle = if model.selected_count() == 0 {
            ShareStrings::select_chats()
        } else {
            ShareStrings::selected_count(model.selected_count())
        };
        let view = cx.entity().downgrade();
        hstack(12.)
            .w_full()
            .child(
                div()
                    .id("share-search-open")
                    .tooltip_label(ShareStrings::search())
                    .child(
                        plain("share-search-open-button")
                            .size(px(44.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_press(move |window, cx| {
                                let _ = view.update(cx, |view, cx| view.activate_search(window, cx));
                            })
                            .children(IconUi::sized("magnifyingglass", px(22.), ShareSurface::accent(), false)),
                    ),
            )
            .child(spacer_min_w(0.))
            .child(
                vstack(1.)
                    .child(
                        div()
                            .swiftui_font(Font::system_weight(20., FontWeight::BOLD))
                            .text_color(label_primary())
                            .child(ShareStrings::title()),
                    )
                    .child(
                        div()
                            .swiftui_font(Font::caption())
                            .text_color(label_secondary())
                            .child(subtitle),
                    ),
            )
            .child(spacer_min_w(0.))
            .child(self.account_avatar(cx))
            .into_any_element()
    }

    fn account_line(display_name: &str) -> String {
        format_printf(&ShareStrings::account_format(), &[FormatArg::Text(display_name.into())])
    }

    fn account_avatar(&self, cx: &App) -> AnyElement {
        let account = self.model.read(cx).account();
        let label = Self::account_line(
            &account
                .map(|account| account.display_name.clone())
                .unwrap_or_else(ShareStrings::account),
        );
        div()
            .id("share-account-avatar")
            .tooltip_label(label)
            .child(ShareAvatarView::new(
                "share-account-avatar-image",
                account.and_then(|account| account.avatar_url.clone()),
                account.map_or_else(|| "?".to_owned(), |account| ShareAvatarView::initials(&account.display_name)),
                account.map(|account| account.id.clone()),
                ShareDestinationKind::Direct,
                44.0,
            ))
            .into_any_element()
    }

    fn destination_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows = Rc::new(self.destination_rows(window, cx));
        let model = self.model.read(cx);
        let key = if model.is_searching() { "search-results" } else { "destinations" };
        if self.destinations_list_key != key {
            self.destinations_list_key = key.to_owned();
            self.destinations_list.reset(rows.len());
        } else if self.destinations_list.item_count() < rows.len() {
            let old = self.destinations_list.item_count();
            let kept = old.saturating_sub(1);
            self.destinations_list.splice(kept..old, rows.len() - kept);
        } else if self.destinations_list.item_count() != rows.len() {
            let old = self.destinations_list.item_count();
            self.destinations_list.splice(0..old, rows.len());
        }
        let (_, column_width) = Self::grid_layout(window);
        let selections = Rc::new(model.selections_by_destination_id().clone());
        let is_retry_locked = model.is_retry_locked();
        let nonfatal_message = model.nonfatal_message().map(str::to_owned);
        let is_searching = model.is_searching();
        let keyboard_index = self.keyboard_index;
        let model_entity = self.model.clone();
        list(self.destinations_list.clone(), move |index, _window, _cx| {
            let Some(row) = rows.get(index) else {
                return div().into_any_element();
            };
            let top = if index == 0 { 3.0 } else { 0.0 };
            let last = index + 1 == rows.len();
            match row {
                ShareDestinationRow::RecentHeader => div()
                    .pt(px(top + 2.))
                    .pb(px(10.))
                    .px(px(18.))
                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                    .text_color(label_secondary())
                    .child(ShareStrings::recent().to_uppercase())
                    .into_any_element(),
                ShareDestinationRow::ValidationBanner => div()
                    .pt(px(top))
                    .pb(px(10.))
                    .px(px(14.))
                    .child(Self::validation_banner(&model_entity, nonfatal_message.clone()))
                    .into_any_element(),
                ShareDestinationRow::LoadingGrid => div().pt(px(top)).child(Self::loading_grid()).into_any_element(),
                ShareDestinationRow::EmptyState => {
                    let (title, detail) = if is_searching {
                        (ShareStrings::no_search_results(), ShareStrings::try_another_search())
                    } else {
                        (ShareStrings::no_chats(), ShareStrings::no_chats_detail())
                    };
                    div()
                        .pt(px(top))
                        .child(Self::empty_state(&model_entity, title, detail, !is_searching))
                        .into_any_element()
                }
                ShareDestinationRow::Tiles(cells) => hstack(GRID_SPACING)
                    .w_full()
                    .justify_center()
                    .items_start()
                    .px(px(14.))
                    .pt(px(top))
                    .pb(px(if last { 16. } else { GRID_ROW_SPACING }))
                    .children(cells.iter().map(|cell| {
                        let tile = match cell {
                            ShareDestinationCell::Story => {
                                let model = model_entity.clone();
                                StoryDestinationTile::new(AVATAR_SIZE, move |_, cx| {
                                    model.update(cx, |model, cx| model.share_to_story(cx));
                                })
                                .column_width(column_width)
                                .into_any_element()
                            }
                            ShareDestinationCell::Destination { index, destination } => {
                                let selection = selections.get(&destination.id);
                                let tap_model = model_entity.clone();
                                let tapped = destination.clone();
                                let appear_model = model_entity.clone();
                                let appeared = destination.clone();
                                ShareDestinationTile::new(
                                    destination.clone(),
                                    AVATAR_SIZE,
                                    selection.is_some(),
                                    selection.and_then(|selection| selection.topic.as_ref().map(|topic| topic.title.clone())),
                                    keyboard_index == Some(*index),
                                    is_retry_locked,
                                    move |_, cx| {
                                        let tapped = tapped.clone();
                                        tap_model.update(cx, |model, cx| model.tap_destination(tapped, cx));
                                    },
                                )
                                .column_width(column_width)
                                .on_appear(
                                    ElementId::Name(format!("share-destination-appear-{}", destination.id).into()),
                                    move |_, cx| {
                                        appear_model.update(cx, |model, cx| model.destination_did_appear(&appeared, cx));
                                    },
                                )
                                .into_any_element()
                            }
                        };
                        div().w(px(column_width)).flex().justify_center().child(tile)
                    }))
                    .into_any_element(),
            }
        })
        .size_full()
        .into_any_element()
    }

    fn topic_content(
        model: &Entity<ShareExtensionViewModel>,
        destination: &ShareDestination,
        window: &Window,
        cx: &App,
    ) -> Div {
        let state = model.read(cx);
        let container = div().size_full().flex().flex_col();
        if state.is_loading_topics() {
            return container.child(Self::loading_grid());
        }
        if state.topic_groups().is_empty() {
            return container.child(Self::empty_state(
                model,
                ShareStrings::no_topics(),
                ShareStrings::no_topics_detail(),
                !state.is_searching(),
            ));
        }
        let (columns, column_width) = Self::grid_layout(window);
        let selected_topic_id = state
            .selections_by_destination_id()
            .get(&destination.id)
            .and_then(|selection| selection.topic.as_ref().map(ShareTopic::selection_id));
        let groups: Vec<RiseShareTopicGroup<ShareTopic>> = state.topic_groups().to_vec();
        container.relative().child(
            div().absolute().inset_0().child(
                scroll_view(ElementId::Name(format!("topics-{}", destination.id).into()))
                    .shows_indicators(false)
                    .child(
                        vstack(16.)
                            .align(HorizontalAlignment::Leading)
                            .px(px(14.))
                            .py(px(8.))
                            .children(groups.into_iter().map(|group| {
                                vstack(10.)
                                    .align(HorizontalAlignment::Leading)
                                    .w_full()
                                    .children(group.section.as_ref().map(|section| {
                                        div()
                                            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                                            .text_color(label_secondary())
                                            .line_clamp(1)
                                            .text_ellipsis()
                                            .child(section.name.to_uppercase())
                                    }))
                                    .child(
                                        vstack(GRID_ROW_SPACING).w_full().children(group.topics.chunks(columns).map(|chunk| {
                                            hstack(GRID_SPACING).w_full().justify_center().items_start().children(
                                                chunk.iter().map(|topic| {
                                                    let selected =
                                                        selected_topic_id.as_deref() == Some(topic.selection_id().as_str());
                                                    let model = model.clone();
                                                    let tapped = topic.clone();
                                                    div().w(px(column_width)).flex().justify_center().child(
                                                        ShareTopicTile::new(topic.clone(), AVATAR_SIZE, selected, move |_, cx| {
                                                            let tapped = tapped.clone();
                                                            model.update(cx, |model, cx| model.select_topic(tapped, cx));
                                                        })
                                                        .column_width(column_width),
                                                    )
                                                }),
                                            )
                                        })),
                                    )
                            })),
                    ),
            ),
        )
    }

    fn validation_banner(model: &Entity<ShareExtensionViewModel>, nonfatal_message: Option<String>) -> Div {
        let model = model.clone();
        hstack(10.)
            .w_full()
            .p(px(11.))
            .bg(ShareSurface::elevated())
            .continuous_corners(13.)
            .children(IconUi::sized("arrow.triangle.2.circlepath", px(15.), label_secondary(), false))
            .child(
                div()
                    .flex_1()
                    .swiftui_font(Font::caption())
                    .text_color(label_secondary())
                    .line_clamp(2)
                    .child(nonfatal_message.unwrap_or_else(ShareStrings::refreshing_chats)),
            )
            .child(spacer_min_w(4.))
            .child(
                plain("share-validation-retry")
                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                    .text_color(ShareSurface::accent())
                    .on_press(move |_, cx| model.update(cx, |model, cx| model.retry_live_validation(cx)))
                    .child(ShareStrings::retry()),
            )
    }

    fn loading_grid() -> Div {
        div()
            .w_full()
            .px(px(14.))
            .py(px(10.))
            .flex()
            .flex_wrap()
            .justify_center()
            .gap_x(px(GRID_SPACING))
            .gap_y(px(GRID_ROW_SPACING))
            .children((0..LOADING_TILE_COUNT).map(|_| {
                vstack(7.)
                    .child(
                        zstack(Alignment::Center)
                            .size(px(AVATAR_SIZE))
                            .child(div().size_full().circle().bg(ShareSurface::avatar_loading()))
                            .child(progress_view().control_size(ControlSize::Small).tint(label_primary().opacity(0.75))),
                    )
                    .child(
                        div()
                            .w(px(AVATAR_SIZE * 0.72))
                            .h(px(9.))
                            .rounded(px(4.))
                            .bg(ShareSurface::avatar_loading()),
                    )
            }))
    }

    fn empty_state(
        model: &Entity<ShareExtensionViewModel>,
        title: String,
        detail: String,
        shows_retry: bool,
    ) -> Div {
        let model = model.clone();
        vstack(10.)
            .w_full()
            .p(px(30.))
            .children(IconUi::sized(
                "bubble.left.and.exclamationmark.bubble.right",
                px(Font::large_title().size),
                label_secondary(),
                false,
            ))
            .child(
                div()
                    .swiftui_font(Font::headline())
                    .text_color(label_primary())
                    .child(title),
            )
            .child(
                div()
                    .swiftui_font(Font::subheadline())
                    .text_color(label_secondary())
                    .text_center()
                    .child(detail),
            )
            .when(shows_retry, |stack| {
                stack.child(
                    system_button("share-empty-retry", BorderedStyle::Bordered)
                        .tint(ShareSurface::accent())
                        .on_press(move |_, cx| model.update(cx, |model, cx| model.retry_live_validation(cx)))
                        .child(ShareStrings::retry()),
                )
            })
    }

    fn share_footer(&self, cx: &App) -> AnyElement {
        let model = self.model.read(cx);
        let can_send = model.can_send();
        let selected_count = model.selected_count();
        let label = if model.is_retry_locked() {
            ShareStrings::retry()
        } else {
            ShareStrings::send()
        };
        let send_model = self.model.clone();
        vstack(10.)
            .w_full()
            .px(px(14.))
            .pb(px(14.))
            .child(div().w_full().h(px(1.)).bg(ShareSurface::divider()))
            .child(
                div()
                    .id("share-comment")
                    .w_full()
                    .px(px(14.))
                    .py(px(11.))
                    .bg(ShareSurface::input())
                    .continuous_corners(14.)
                    .child(self.comment_input.clone()),
            )
            .child(
                div()
                    .id("share-send")
                    .w_full()
                    .child(
                        plain("share-send-button")
                            .w_full()
                            .disabled(!can_send)
                            .opacity(if can_send { 1.0 } else { 0.5 })
                            .on_press(move |_, cx| send_model.update(cx, |model, cx| model.send(cx)))
                            .child(
                                hstack(8.)
                                    .w_full()
                                    .min_h(px(50.))
                                    .justify_center()
                                    .bg(ShareSurface::accent())
                                    .capsule()
                                    .text_color(on_fill())
                                    .child(div().swiftui_font(Font::system_weight(18., FontWeight::SEMIBOLD)).child(label))
                                    .child(
                                        div()
                                            .min_w(px(24.))
                                            .min_h(px(24.))
                                            .px(px(4.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded_full()
                                            .bg(on_fill())
                                            .swiftui_font(Font::caption().bold())
                                            .text_color(ShareSurface::accent())
                                            .child(selected_count.to_string()),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn busy_sheet(&self, cx: &App) -> AnyElement {
        let model = self.model.read(cx);
        let succeeded = model.activity() == ShareExtensionViewModelActivity::Succeeded;
        zstack(Alignment::Center)
            .size_full()
            .clip_shape(Shape::RoundedRectangle {
                corner_radius: 25.0,
                style: RoundedCornerStyle::Continuous,
            })
            .child(div().size_full().bg(ShareSurface::sheet()))
            .child(
                vstack(12.)
                    .child(ShareSendProgressView::new("share-send-progress", model.send_progress(), succeeded))
                    .child(
                        div()
                            .px(px(20.))
                            .swiftui_font(Font::system_weight(15., FontWeight::SEMIBOLD))
                            .text_color(label_primary().opacity(if succeeded { 1.0 } else { 0.78 }))
                            .text_center()
                            .line_clamp(2)
                            .child(Self::busy_status_text(model.activity())),
                    ),
            )
            .child(centred_stroke(
                ShareSurface::hairline(),
                1.0,
                Shape::RoundedRectangle {
                    corner_radius: 25.0,
                    style: RoundedCornerStyle::Continuous,
                },
            ))
            .into_any_element()
    }

    fn busy_status_text(activity: ShareExtensionViewModelActivity) -> String {
        match activity {
            ShareExtensionViewModelActivity::Succeeded => ShareStrings::sent(),
            ShareExtensionViewModelActivity::OpeningStory => ShareStrings::opening_story_editor(),
            ShareExtensionViewModelActivity::OpeningApp => ShareStrings::opening_app_for_large_share(),
            ShareExtensionViewModelActivity::Loading
            | ShareExtensionViewModelActivity::Ready
            | ShareExtensionViewModelActivity::Sending => ShareStrings::sending(),
        }
    }
}

impl Focusable for ShareExtensionView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ShareExtensionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let appearance = ShareSurface::capture(cx);
        if appearance != self.input_appearance {
            self.input_appearance = appearance;
            for input in [self.search_input.clone(), self.comment_input.clone()] {
                input.update(cx, |state, cx| {
                    state.set_text_color(Some(label_primary()), cx);
                    state.set_placeholder_color(Some(label_secondary()), cx);
                    state.set_caret_color(Some(ShareSurface::accent()), cx);
                });
            }
        }
        let now = RiseClock::monotonic();
        let sheet_width = Self::sheet_width(window);
        let picker_target = Self::picker_height_target(window);
        if *self.picker_height.target() != picker_target {
            if *self.picker_height.target() == 0.0 {
                self.picker_height.jump(picker_target);
            } else {
                self.picker_height
                    .set_in(picker_target, spring(0.38, 0.90), now, cx);
            }
        }
        let picker_height = self.picker_height.frame_value(now, window);
        let model = self.model.read(cx);
        let can_cancel = model.can_cancel();
        let is_busy = model.is_busy();

        let mut sheet = zstack(Alignment::Bottom)
            .w(px(sheet_width))
            .h(px(if is_busy { BUSY_SHEET_HEIGHT } else { picker_height }));
        let picker = self.picker_sheet(now, window, cx);
        if let Some(picker) = self
            .picker_presence
            .render_transitioned(now, window, move |_| div().size_full().child(picker))
        {
            sheet = sheet.child(picker);
        }
        let busy = self.busy_sheet(cx);
        if let Some(busy) = self
            .busy_presence
            .render_transitioned(now, window, move |_| div().size_full().child(busy))
        {
            sheet = sheet.child(busy);
        }

        let cancel_view = cx.entity().downgrade();
        let scrim_view = cx.entity().downgrade();
        div()
            .id("share-extension-layer")
            .track_focus(&self.focus_handle)
            .key_context("ShareExtensionView")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| this.handle_key_down(event, window, cx)))
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_end()
            .child(
                div()
                    .id("share-extension-scrim")
                    .absolute()
                    .inset_0()
                    .bg(Hsla {
                        h: 0.0,
                        s: 0.0,
                        l: 0.0,
                        a: SCRIM_OPACITY,
                    })
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        let _ = scrim_view.update(cx, |view, cx| view.cancel_if_allowed(cx));
                    }),
            )
            .child(
                vstack(10.)
                    .pb(px(8.))
                    .occlude()
                    .child(sheet)
                    .child(
                        div()
                            .id("share-cancel")
                            .w(px(sheet_width))
                            .child(
                                plain("share-cancel-button")
                                    .w_full()
                                    .min_h(px(58.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(ShareSurface::sheet())
                                    .continuous_corners(22.)
                                    .disabled(!can_cancel)
                                    .opacity(if can_cancel { 1.0 } else { 0.48 })
                                    .swiftui_font(Font::system_weight(18., FontWeight::SEMIBOLD))
                                    .text_color(label_primary())
                                    .on_press(move |_, cx| {
                                        let _ = cancel_view.update(cx, |view, cx| {
                                            view.model.update(cx, |model, cx| model.cancel(cx));
                                        });
                                    })
                                    .child(ShareStrings::cancel()),
                            ),
                    ),
            )
    }
}

#[derive(IntoElement)]
struct ShareDestinationTile {
    destination: ShareDestination,
    size: f32,
    selected: bool,
    selection_subtitle: Option<String>,
    is_keyboard_target: bool,
    disabled: bool,
    label_width: f32,
    action: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ShareDestinationTile {
    fn new(
        destination: ShareDestination,
        size: f32,
        selected: bool,
        selection_subtitle: Option<String>,
        is_keyboard_target: bool,
        disabled: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            destination,
            size,
            selected,
            selection_subtitle,
            is_keyboard_target,
            disabled,
            label_width: TILE_LABEL_MAXIMUM_WIDTH,
            action: Rc::new(action),
        }
    }

    fn column_width(mut self, column_width: f32) -> Self {
        self.label_width = column_width.min(TILE_LABEL_MAXIMUM_WIDTH);
        self
    }
}

impl RenderOnce for ShareDestinationTile {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let size = self.size;
        let action = self.action.clone();
        let destination = self.destination;
        let mut ring = zstack(Alignment::Center).size(px(size + 8.)).relative();
        if self.selected {
            ring = ring.child(selection_ring(size));
        }
        ring = ring.child(ShareAvatarView::new(
            ElementId::Name(format!("share-destination-avatar-{}", destination.id).into()),
            destination.avatar_url.clone(),
            destination.initials(),
            Some(destination.avatar_color_key.clone().unwrap_or_else(|| destination.id.clone())),
            destination.kind,
            size,
        ));
        let topics_badge = (destination.has_topics && !self.selected).then(|| {
            div()
                .absolute()
                .top(px(-1.))
                .right(px(-2.))
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(ShareSurface::accent())
                .border(px(2.))
                .border_color(ShareSurface::sheet())
                .children(IconUi::sized("number", px(Font::caption2().size), on_fill(), false))
        });
        let check_badge = self.selected.then(selection_check);
        plain(ElementId::Name(format!("share-destination-{}", destination.id).into()))
            .disabled(self.disabled)
            .on_press(move |window, cx| action(window, cx))
            .rounded(px(12.))
            .when(self.is_keyboard_target, |button| button.bg(ShareSurface::elevated()))
            .child(
                vstack(7.)
                    .child(ring.children(topics_badge).children(check_badge))
                    .child(
                        div()
                            .w(px(self.label_width))
                            .swiftui_font(Font::caption())
                            .text_color(label_primary())
                            .text_center()
                            .line_clamp(2)
                            .text_ellipsis()
                            .child(self.selection_subtitle.unwrap_or_else(|| destination.title.clone())),
                    ),
            )
    }
}

fn selection_ring(size: f32) -> Div {
    div()
        .size(px(size + 7.))
        .rounded_full()
        .border(px(2.5))
        .border_color(ShareSurface::accent())
}

fn selection_check() -> Div {
    div()
        .absolute()
        .bottom(px(-1.))
        .right(px(-2.))
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(ShareSurface::accent())
        .border(px(2.5))
        .border_color(ShareSurface::sheet())
        .children(IconUi::sized("checkmark", px(Font::caption().size), on_fill(), false))
}

#[derive(IntoElement)]
struct ShareTopicTile {
    topic: ShareTopic,
    size: f32,
    selected: bool,
    label_width: f32,
    action: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ShareTopicTile {
    fn new(topic: ShareTopic, size: f32, selected: bool, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            topic,
            size,
            selected,
            label_width: TILE_LABEL_MAXIMUM_WIDTH,
            action: Rc::new(action),
        }
    }

    fn column_width(mut self, column_width: f32) -> Self {
        self.label_width = column_width;
        self
    }

    fn symbol(&self) -> String {
        let value = self
            .topic
            .icon
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or_default();
        if value.is_empty() { "#".to_owned() } else { value.to_owned() }
    }
}

impl RenderOnce for ShareTopicTile {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let size = self.size;
        let symbol = self.symbol();
        let action = self.action.clone();
        let mut ring = zstack(Alignment::Center).size(px(size + 8.)).relative();
        if self.selected {
            ring = ring.child(selection_ring(size));
        }
        ring = ring.child(
            div()
                .size(px(size))
                .rounded_full()
                .bg(ShareSurface::elevated())
                .flex()
                .items_center()
                .justify_center()
                .swiftui_font(Font::system_weight(size * 0.42, FontWeight::SEMIBOLD))
                .text_color(label_primary())
                .child(symbol),
        );
        plain(ElementId::Name(format!("share-topic-{}", self.topic.selection_id()).into()))
            .on_press(move |window, cx| action(window, cx))
            .child(
                vstack(7.)
                    .child(ring.children(self.selected.then(selection_check)))
                    .child(
                        div()
                            .w(px(self.label_width))
                            .swiftui_font(Font::caption())
                            .text_color(label_primary())
                            .text_center()
                            .line_clamp(2)
                            .text_ellipsis()
                            .child(self.topic.title.clone()),
                    ),
            )
    }
}

#[derive(IntoElement)]
struct StoryDestinationTile {
    size: f32,
    label_width: f32,
    action: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl StoryDestinationTile {
    fn new(size: f32, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            size,
            label_width: TILE_LABEL_MAXIMUM_WIDTH,
            action: Rc::new(action),
        }
    }

    fn column_width(mut self, column_width: f32) -> Self {
        self.label_width = column_width.min(TILE_LABEL_MAXIMUM_WIDTH);
        self
    }
}

impl RenderOnce for StoryDestinationTile {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let size = self.size;
        let action = self.action.clone();
        plain("share-story-destination")
            .on_press(move |window, cx| action(window, cx))
            .child(
                vstack(7.)
                    .child(
                        div().size(px(size + 8.)).flex().items_center().justify_center().child(
                            div()
                                .relative()
                                .size(px(size))
                                .rounded_full()
                                .bg(ShareSurface::accent())
                                .flex()
                                .items_center()
                                .justify_center()
                                .children(IconUi::sized("plus", px(size * 0.38), on_fill(), false))
                                .child(centred_stroke(white_opacity(0.16), 1.0, Shape::Circle)),
                        ),
                    )
                    .child(
                        div()
                            .w(px(self.label_width))
                            .swiftui_font(Font::caption())
                            .text_color(label_primary())
                            .text_center()
                            .line_clamp(2)
                            .text_ellipsis()
                            .child(ShareStrings::share_to_story()),
                    ),
            )
    }
}

struct ShareAvatarViewState {
    image: Option<CleverImage>,
    loaded_url: String,
    did_finish_loading: bool,
    appearance: Animated<f32>,
}

#[derive(IntoElement)]
struct ShareAvatarView {
    id: ElementId,
    url: Option<Url>,
    initials: String,
    color_key: Option<String>,
    kind: ShareDestinationKind,
    size: f32,
}

impl ShareAvatarView {
    fn new(
        id: impl Into<ElementId>,
        url: Option<Url>,
        initials: String,
        color_key: Option<String>,
        kind: ShareDestinationKind,
        size: f32,
    ) -> Self {
        let photo = url.filter(|url| !Self::is_letter_avatar(url));
        Self {
            id: id.into(),
            url: photo,
            initials,
            color_key,
            kind,
            size,
        }
    }

    fn is_letter_avatar(url: &Url) -> bool {
        url.path().contains(GENERATED_AVATAR_SEGMENT)
    }

    fn placeholder_color(&self) -> Hsla {
        if self.kind == ShareDestinationKind::Saved {
            return AvatarPalette::favourites_background();
        }
        let palette = AvatarPalette::palette();
        if palette.is_empty() {
            return AvatarPalette::letter_color();
        }
        let key = self.color_key.as_deref().unwrap_or(&self.initials);
        palette[AvatarPalette::simple_hash(key) as usize % palette.len()]
    }

    fn initials(value: &str) -> String {
        let trimmed = trimming_whitespaces_and_newlines(value);
        characters(trimmed)
            .next()
            .map(str::to_uppercase)
            .unwrap_or_else(|| "?".to_owned())
    }

    fn fallback(&self) -> ZStack {
        let size = self.size;
        let base = zstack(Alignment::Center)
            .size_full()
            .child(div().size_full().rounded_full().bg(self.placeholder_color()));
        if self.kind == ShareDestinationKind::Saved {
            return base.children(IconUi::sized("bookmark.fill", px(size * 0.38), on_fill(), true));
        }
        let letter = characters(&self.initials).next().unwrap_or_default().to_owned();
        base.child(
            div()
                .swiftui_font(Font::system_weight(size * 0.42, FontWeight::SEMIBOLD))
                .text_color(on_fill())
                .child(letter),
        )
    }
}

impl RenderOnce for ShareAvatarView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let url_key = self.url.as_ref().map(|url| url.absolute_string().to_owned());
        let has_photo = self.url.is_some();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| ShareAvatarViewState {
            image: None,
            loaded_url: String::new(),
            did_finish_loading: !has_photo,
            appearance: Animated::new(0.0),
        });
        let task_state = state.clone();
        let task_url = self.url.clone();
        use_task(
            ElementId::Name(format!("{}-load", self.id).into()),
            url_key.clone(),
            window,
            cx,
            async move |cx| {
                let Some(url) = task_url else {
                    task_state.update(cx, |state, cx| {
                        state.image = None;
                        state.loaded_url = String::new();
                        state.did_finish_loading = true;
                        cx.notify();
                    });
                    return;
                };
                let key = url.absolute_string().to_owned();
                let has_image = task_state.update(cx, |state, cx| {
                    if state.loaded_url != key {
                        state.image = None;
                        state.loaded_url = String::new();
                        state.did_finish_loading = false;
                        cx.notify();
                    }
                    state.image.is_some()
                });
                if has_image {
                    return;
                }
                let loaded = ShareAvatarImageCache::load_image(url).await;
                task_state.update(cx, |state, cx| {
                    if let Some(loaded) = loaded {
                        state.image = Some(loaded);
                        state.appearance.set_in(1.0, ease_out(0.18), RiseClock::monotonic(), cx);
                        state.loaded_url = key;
                    }
                    state.did_finish_loading = true;
                    cx.notify();
                });
            },
        );
        let now = RiseClock::monotonic();
        let current = state.read(cx);
        let content = if let Some(image) = current.image.clone() {
            img(image)
                .size_full()
                .rounded_full()
                .object_fit(ObjectFit::Cover)
                .opacity(current.appearance.frame_value(now, window))
                .into_any_element()
        } else if has_photo && !current.did_finish_loading {
            zstack(Alignment::Center)
                .size_full()
                .child(div().size_full().rounded_full().bg(ShareSurface::avatar_loading()))
                .child(progress_view().control_size(ControlSize::Small).tint(label_primary().opacity(0.82)))
                .into_any_element()
        } else {
            self.fallback().into_any_element()
        };
        div()
            .size(px(self.size))
            .flex_none()
            .rounded_full()
            .overflow_hidden()
            .child(content)
    }
}
