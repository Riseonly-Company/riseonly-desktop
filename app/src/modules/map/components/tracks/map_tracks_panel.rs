use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Context, DispatchPhase, FocusHandle, FontWeight, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Render,
    RenderOnce, SharedString, StyleRefinement, Styled, Window, canvas, div,
    px,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::number_format::measurement::{Measurement, UnitLength};
use rise_i18n::number_format::measurement_formatter::{
    FormatterUnitStyle, LocaleMeasurementSystem, MeasurementFormatter, MeasurementFormatterUnitOptions,
};
use rise_i18n::tr;
use rise_platform::haptics::SelectionFeedbackGenerator;
use rise_theme::AppTheme;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::button_ui::ButtonUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::{LiquidChrome, LiquidChromeModifiers};
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::skeleton_ui::SkeletonLine;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::environment::LayoutDirection;
use rise_ui::swiftui::environment_values::accessibility_reduce_transparency;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::stacks::{hstack, spacer_min_w, vstack};

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::map::engine::rise_map_engine_models::RiseMapLoadState;
use crate::modules::map::engine::rise_map_tracks_repository::RiseMapTracksSnapshot;
use crate::modules::map::shared::map_friend_status::MapFriendStatus;
use crate::modules::map::stores::map_actions::map_tracks_types::MapTrackPeriod;
use crate::modules::map::stores::map_actions::map_types::MapUser;

pub type MapTrackPeriodAction = Rc<dyn Fn(MapTrackPeriod, &mut Window, &mut App)>;
pub type MapPanelAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type MapPanelModeAction = Rc<dyn Fn(MapPanelMode, &mut Window, &mut App)>;

/// my routes panel
/// period switch only moves a map filter so lines update in the same frame
/// numbers come from the summary already in hand
#[derive(IntoElement)]
pub struct MapTracksPanel {
    snapshot: RiseMapTracksSnapshot,
    owner: Option<MapUser>,
    is_recording: bool,
    on_select: MapTrackPeriodAction,
    on_retry: MapPanelAction,
    on_close_owner: MapPanelAction,
    embedded: bool,
}

thread_local! {
    static DISTANCE_FORMATTER: MeasurementFormatter = MapTracksPanel::distance_formatter();
}

impl MapTracksPanel {
    const CORNER_RADIUS: f32 = 28.0;

    pub fn new(
        snapshot: RiseMapTracksSnapshot,
        is_recording: bool,
        on_select: impl Fn(MapTrackPeriod, &mut Window, &mut App) + 'static,
        on_retry: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            snapshot,
            owner: None,
            is_recording,
            on_select: Rc::new(on_select),
            on_retry: Rc::new(on_retry),
            on_close_owner: Rc::new(|_, _| {}),
            embedded: false,
        }
    }

    pub fn owner(mut self, owner: Option<MapUser>) -> Self {
        self.owner = owner;
        self
    }

    pub fn on_close_owner(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close_owner = Rc::new(action);
        self
    }

    pub fn embedded(mut self, embedded: bool) -> Self {
        self.embedded = embedded;
        self
    }

    fn panel_content(&self, theme: &AppTheme) -> gpui::Div {
        let mut stack = vstack(12.0).w_full();
        if let Some(owner) = &self.owner {
            stack = stack.child(self.owner_header(owner, theme));
        }
        stack
            .child(self.period_switch(theme))
            .child(div().w_full().h(px(100.0)).flex().items_center().child(self.content(theme)))
            .px(px(if self.embedded { 0.0 } else { 14.0 }))
            .py(px(if self.embedded { 6.0 } else { 14.0 }))
    }

    fn owner_header(&self, owner: &MapUser, theme: &AppTheme) -> impl IntoElement {
        let on_close_owner = self.on_close_owner.clone();
        let name = SharedString::from(owner.display_name().to_owned());
        hstack(10.0)
            .w_full()
            .items_center()
            .px(px(2.0))
            .child(
                UserLogoUi::new(SharedString::from(format!("map-tracks-owner-{}", owner.id)))
                    .source(SharedString::from(owner.logo.clone()))
                    .name(name.clone())
                    .user_id(SharedString::from(owner.id.clone()))
                    .size(30.0),
            )
            .child(MainText::new(name).px(15.0).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
            .child(spacer_min_w(8.0))
            .child(
                plain("map-tracks-close-owner")
                    .on_press(move |window, cx| on_close_owner(window, cx))
                    .child(
                        div()
                            .id("map-tracks-close-owner-face")
                            .size(px(28.0))
                            .rounded_full()
                            .bg(theme.text.secondary.opacity(0.16))
                            .flex()
                            .items_center()
                            .justify_center()
                            .tooltip_label(tr("map_tracks_back_to_mine"))
                            .children(IconUi::sized("xmark", px(11.0), theme.text.secondary, false)),
                    ),
            )
    }

    fn period_switch(&self, theme: &AppTheme) -> impl IntoElement {
        let period = self.snapshot.period;
        let mut rows = vstack(4.0).w_full();
        for chunk in MapTrackPeriod::ALL.chunks(3) {
            let mut row = hstack(4.0).w_full();
            for candidate in chunk.iter().copied() {
                let is_selected = candidate == period;
                let on_select = self.on_select.clone();
                row = row.child(
                    div().flex_1().min_w_0().child(
                        plain(SharedString::from(format!("map-tracks-period-{}", candidate.canon_name())))
                            .on_press(move |window, cx| on_select(candidate, window, cx))
                            .child(
                                div()
                                    .w_full()
                                    .h(px(40.0))
                                    .px(px(8.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .when_selected(is_selected, theme.primary._100)
                                    .child(
                                        MainText::new(tr(Self::title_key(candidate)))
                                            .px(13.0)
                                            .tac(TextAlignT::Center)
                                            .font_weight(if is_selected { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                                            .color(if is_selected { theme.content_on(theme.primary._100) } else { theme.text.secondary })
                                            .number_of_lines(1)
                                            .minimum_scale_factor(0.8),
                                    ),
                            ),
                    ),
                );
            }
            rows = rows.child(row);
        }
        div()
            .w_full()
            .p(px(3.0))
            .rounded(px(23.0))
            .bg(theme.text.secondary.opacity(0.14))
            .child(rows)
            .guide_anchor(RiseGuideAnchor::MapTrackPeriod)
    }

    fn content(&self, theme: &AppTheme) -> AnyElement {
        if let Some(summary) = &self.snapshot.summary {
            if let Some(stats) = summary.stats(self.snapshot.period).filter(|stats| stats.active_days > 0) {
                return vstack(12.0)
                    .w_full()
                    .child(
                        hstack(0.0)
                            .w_full()
                            .child(Self::metric(
                                Self::distance(stats.walk_run_distance_meters.unwrap_or(0)),
                                tr("map_tracks_walk_run"),
                                theme,
                            ))
                            .child(Self::metric(Self::distance(stats.distance_meters), tr("map_tracks_distance"), theme)),
                    )
                    .child(
                        hstack(0.0)
                            .w_full()
                            .child(Self::metric(Self::duration(stats.duration_seconds), tr("map_tracks_time"), theme))
                            .child(Self::metric(stats.active_days.to_string(), tr("map_tracks_days"), theme)),
                    )
                    .into_any_element();
            }
            return div()
                .w_full()
                .child(
                    MainText::new(tr(Self::empty_key(self.snapshot.owner_id.is_some(), self.is_recording)))
                        .px(14.0)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .number_of_lines(2),
                )
                .into_any_element();
        }
        if self.snapshot.load_state == RiseMapLoadState::Failed {
            let on_retry = self.on_retry.clone();
            return hstack(10.0)
                .w_full()
                .items_center()
                .child(
                    MainText::new(self.snapshot.error_text.clone().unwrap_or_else(|| tr("map_load_failed")))
                        .px(14.0)
                        .color(theme.text.secondary)
                        .number_of_lines(2),
                )
                .child(spacer_min_w(8.0))
                .child(
                    ButtonUi::new("map-tracks-retry")
                        .height(36.0)
                        .fit_content(true)
                        .padding_horizontal(14.0)
                        .on_press(move |window, cx| on_retry(window, cx))
                        .child(
                            MainText::new(tr("common_retry"))
                                .px(14.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(gpui::white())
                                .number_of_lines(1),
                        ),
                )
                .into_any_element();
        }
        let skeleton_row = || {
            let cell = || {
                vstack(7.0)
                    .flex_1()
                    .child(SkeletonLine::new(64.0, 17.0))
                    .child(SkeletonLine::new(80.0, 11.0))
            };
            hstack(0.0).w_full().child(cell()).child(cell())
        };
        vstack(12.0)
            .w_full()
            .child(skeleton_row())
            .child(skeleton_row())
            .into_any_element()
    }

    fn empty_key(has_owner: bool, is_recording: bool) -> &'static str {
        if has_owner {
            return "map_tracks_friend_empty";
        }
        if is_recording { "map_tracks_empty_period" } else { "map_tracks_recording_off" }
    }

    fn metric(value: String, label: String, theme: &AppTheme) -> impl IntoElement {
        vstack(3.0)
            .flex_1()
            .child(
                MainText::new(value)
                    .px(18.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .number_of_lines(1)
                    .monospaced_digits(true),
            )
            .child(
                MainText::new(label)
                    .px(12.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(1),
            )
    }

    fn title_key(period: MapTrackPeriod) -> &'static str {
        match period {
            MapTrackPeriod::Day => "map_period_day",
            MapTrackPeriod::Week => "map_period_week",
            MapTrackPeriod::Month => "map_period_month",
            MapTrackPeriod::HalfYear => "map_period_half_year",
            MapTrackPeriod::Year => "map_period_year",
            MapTrackPeriod::All => "map_period_all",
        }
    }

    fn distance_formatter() -> MeasurementFormatter {
        let mut formatter = MeasurementFormatter::new();
        formatter.unit_options = MeasurementFormatterUnitOptions::PROVIDED_UNIT;
        formatter.unit_style = FormatterUnitStyle::Medium;
        formatter.number_formatter.maximum_fraction_digits = 1;
        formatter
    }

    /// measurement system comes from the reader's locale
    /// unit is picked here because natural scale drops to millimeters for zero and inches for a few meters
    pub fn distance(meters: i64) -> String {
        DISTANCE_FORMATTER.with(|formatter| {
            let length = Measurement::new(meters.max(0) as f64, UnitLength::Meters);
            let is_metric = formatter.locale.measurement_system() == LocaleMeasurementSystem::Metric;
            let unit = if is_metric {
                if length.value < 1_000.0 { UnitLength::Meters } else { UnitLength::Kilometers }
            } else if length.converted(UnitLength::Miles).value < 0.1 {
                UnitLength::Feet
            } else {
                UnitLength::Miles
            };
            let mut shown = length.converted(unit);
            if unit == UnitLength::Meters || unit == UnitLength::Feet {
                shown.value = shown.value.round();
            }
            formatter.string_from(&shown)
        })
    }

    fn duration(seconds: i64) -> String {
        MapFriendStatus::duration(seconds * 1_000)
    }
}

impl RenderOnce for MapTracksPanel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let content = self.panel_content(&theme);
        if self.embedded {
            content.into_any_element()
        } else {
            div()
                .rounded(px(Self::CORNER_RADIUS))
                .child(content)
                .liquid_chrome_rounded_rectangle(Self::CORNER_RADIUS)
        }
    }
}

trait MapTrackPeriodChip: Styled + Sized {
    fn when_selected(self, is_selected: bool, fill: Hsla) -> Self {
        if is_selected { self.bg(fill) } else { self }
    }
}

impl<E: Styled> MapTrackPeriodChip for E {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MapPanelMode {
    Friends,
    Tracks,
    Media,
}

impl MapPanelMode {
    pub const ALL: [MapPanelMode; 3] = [MapPanelMode::Friends, MapPanelMode::Tracks, MapPanelMode::Media];

    pub const fn raw_value(self) -> &'static str {
        match self {
            MapPanelMode::Friends => "friends",
            MapPanelMode::Tracks => "tracks",
            MapPanelMode::Media => "media",
        }
    }

    pub const fn id(self) -> &'static str {
        self.raw_value()
    }

    pub const fn title_key(self) -> &'static str {
        match self {
            MapPanelMode::Friends => "map_friends_section",
            MapPanelMode::Tracks => "chat_settings_statistics",
            MapPanelMode::Media => "map_media_library",
        }
    }

    pub const fn icon(self) -> &'static str {
        match self {
            MapPanelMode::Friends => "person.2.fill",
            MapPanelMode::Tracks => "chart.bar.fill",
            MapPanelMode::Media => "photo.stack.fill",
        }
    }
}

#[derive(IntoElement)]
pub struct MapModeSwitch {
    mode: MapPanelMode,
    on_select: MapPanelModeAction,
}

impl MapModeSwitch {
    pub fn new(mode: MapPanelMode, on_select: impl Fn(MapPanelMode, &mut Window, &mut App) + 'static) -> Self {
        Self { mode, on_select: Rc::new(on_select) }
    }
}

impl RenderOnce for MapModeSwitch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let reduce_transparency = accessibility_reduce_transparency(cx);
        let control =
            window.use_keyed_state(SharedString::from("map-mode-glass-control"), cx, MapModeGlassControl::new);
        let configuration = MapModeGlassConfiguration {
            mode: self.mode,
            titles: MapPanelMode::ALL.iter().map(|mode| SharedString::from(tr(mode.title_key()))).collect(),
            primary: theme.primary._100,
            secondary: theme.text.secondary,
            background: LiquidChrome::surface_fill(&theme, 0),
            reduce_transparency,
        };
        let on_select = self.on_select;
        control.update(cx, |control, cx| {
            control.on_select = on_select;
            control.configure(configuration, cx);
        });
        div().w_full().h(px(56.0)).child(control.cached(StyleRefinement::default().size_full()))
    }
}

#[derive(Clone, PartialEq)]
struct MapModeGlassConfiguration {
    mode: MapPanelMode,
    titles: Vec<SharedString>,
    primary: Hsla,
    secondary: Hsla,
    background: Hsla,
    reduce_transparency: bool,
}

/// dragging only changes the control's own geometry
/// map and repositories get one selection on release even if the pointer crossed every section
pub struct MapModeGlassControl {
    on_select: MapPanelModeAction,
    configuration: Option<MapModeGlassConfiguration>,
    feedback: SelectionFeedbackGenerator,
    selected_index: usize,
    preview_index: usize,
    drag_x: Option<f32>,
    drop_position: Animated<f32>,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    focus_handle: FocusHandle,
}

impl MapModeGlassControl {
    const INSET: f32 = 4.0;

    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            on_select: Rc::new(|_, _, _| {}),
            configuration: None,
            feedback: SelectionFeedbackGenerator::new(),
            selected_index: 0,
            preview_index: 0,
            drag_x: None,
            drop_position: Animated::new(0.0),
            bounds: Rc::new(Cell::new(None)),
            focus_handle: cx.focus_handle(),
        }
    }

    fn configure(&mut self, configuration: MapModeGlassConfiguration, cx: &mut Context<Self>) {
        if self.configuration.as_ref() == Some(&configuration) {
            return;
        }
        let next = MapPanelMode::ALL.iter().position(|mode| *mode == configuration.mode).unwrap_or(0);
        let changed = self.selected_index != next;
        self.selected_index = next;
        let first = self.configuration.is_none();
        self.configuration = Some(configuration);
        if self.drag_x.is_none() {
            self.preview_index = self.selected_index;
            if first {
                self.drop_position.jump(self.visual_index(self.selected_index) as f32);
            } else if changed {
                self.position_drop(true, cx);
            }
        }
        cx.notify();
    }

    fn count() -> usize {
        MapPanelMode::ALL.len()
    }

    fn width(&self) -> f32 {
        self.bounds.get().map_or(0.0, |bounds| f32::from(bounds.size.width))
    }

    fn height(&self) -> f32 {
        self.bounds.get().map_or(56.0, |bounds| f32::from(bounds.size.height))
    }

    fn segment_width(&self) -> f32 {
        (self.width() - Self::INSET * 2.0).max(0.0) / Self::count() as f32
    }

    fn right_to_left() -> bool {
        LayoutDirection::current().is_right_to_left()
    }

    fn visual_index(&self, index: usize) -> usize {
        if Self::right_to_left() { Self::count() - 1 - index } else { index }
    }

    fn index_at(&self, x: f32) -> usize {
        let visual = ((x - Self::INSET) / self.segment_width().max(1.0)).floor();
        let visual = visual.clamp(0.0, (Self::count() - 1) as f32) as usize;
        self.visual_index(visual)
    }

    /// the drop follows the pointer while dragging and rests on the selected section otherwise
    fn position_drop(&mut self, animated: bool, cx: &mut Context<Self>) {
        let target = match self.drag_x {
            Some(x) => {
                let segment = self.segment_width().max(1.0);
                let half = segment / 2.0;
                let centre = x.clamp(Self::INSET + half, (self.width() - Self::INSET - half).max(Self::INSET + half));
                (centre - Self::INSET) / segment - 0.5
            }
            None => self.visual_index(self.selected_index) as f32,
        };
        if animated {
            self.drop_position.set_in(target, spring(0.34, 0.78), RiseClock::monotonic(), cx);
        } else {
            self.drop_position.jump(target);
        }
        cx.notify();
    }

    fn local_x(&self, window_x: Pixels) -> f32 {
        self.bounds.get().map_or(0.0, |bounds| f32::from(window_x - bounds.origin.x))
    }

    fn begin_tracking(&mut self, x: f32, cx: &mut Context<Self>) {
        self.feedback.prepare();
        self.track(x, cx);
    }

    fn continue_tracking(&mut self, x: f32, cx: &mut Context<Self>) {
        self.track(x, cx);
    }

    fn cancel_tracking(&mut self, cx: &mut Context<Self>) {
        self.drag_x = None;
        self.preview_index = self.selected_index;
        self.position_drop(true, cx);
    }

    fn track(&mut self, x: f32, cx: &mut Context<Self>) {
        self.drag_x = Some(x);
        let next = self.index_at(x);
        if next != self.preview_index {
            self.preview_index = next;
            self.feedback.selection_changed();
            self.feedback.prepare();
        }
        self.position_drop(false, cx);
    }

    /// tracking belongs to this control until release even outside its bounds
    /// leaving vertically must not undo the section a horizontal drag previewed
    fn end_tracking(&mut self, x: f32, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.index_at(x);
        self.select(index, window, cx);
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.drag_x = None;
        self.selected_index = index;
        self.preview_index = index;
        self.position_drop(true, cx);
        let on_select = self.on_select.clone();
        let mode = MapPanelMode::ALL[index];
        cx.defer_in(window, move |_, window, cx| on_select(mode, window, cx));
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let step: isize = match event.keystroke.key.as_str() {
            "left" => -1,
            "right" => 1,
            "escape" if self.drag_x.is_some() => {
                self.cancel_tracking(cx);
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        let visual = self.visual_index(self.selected_index) as isize + step;
        if visual < 0 || visual >= Self::count() as isize {
            return;
        }
        let index = self.visual_index(visual as usize);
        self.select(index, window, cx);
        cx.stop_propagation();
    }

    fn label(&self, index: usize, mode: MapPanelMode, title: SharedString, color: Hsla) -> MapModeGlassLabel {
        MapModeGlassLabel {
            icon: mode.icon(),
            title,
            color,
            left: Self::INSET + self.visual_index(index) as f32 * self.segment_width(),
            width: self.segment_width(),
            height: (self.height() - Self::INSET * 2.0).max(0.0),
            anchor: (mode == MapPanelMode::Tracks).then_some(RiseGuideAnchor::MapModeStatistics),
        }
    }
}

/// one section of the mode bar, glyph over its title and tinted while previewed
/// takes no pointer input itself since the control tracks the whole bar
#[derive(IntoElement)]
struct MapModeGlassLabel {
    icon: &'static str,
    title: SharedString,
    color: Hsla,
    left: f32,
    width: f32,
    height: f32,
    anchor: Option<RiseGuideAnchor>,
}

impl RenderOnce for MapModeGlassLabel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .absolute()
            .left(px(self.left))
            .top(px(MapModeGlassControl::INSET))
            .w(px(self.width))
            .h(px(self.height))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(7.0))
                    .w(px(self.width))
                    .h(px(19.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(self.icon, px(15.0), self.color, true)),
            )
            .child(
                div().absolute().left(px(1.0)).top(px(29.0)).w(px((self.width - 2.0).max(0.0))).h(px(13.0)).child(
                    MainText::new(self.title)
                        .px(10.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .color(self.color)
                        .number_of_lines(1)
                        .minimum_scale_factor(0.7),
                ),
            )
            .guide_anchor(self.anchor)
    }
}

impl Render for MapModeGlassControl {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(configuration) = self.configuration.clone() else {
            return div().size_full().into_any_element();
        };
        let segment = self.segment_width();
        let inner_height = (self.height() - Self::INSET * 2.0).max(0.0);
        let position = self.drop_position.frame_value(RiseClock::monotonic(), window);
        let (scale_x, scale_y) = if self.drag_x.is_some() { (1.03, 1.06) } else { (1.0, 1.0) };
        let drop_width = segment * scale_x;
        let drop_height = inner_height * scale_y;
        let drop_centre_x = Self::INSET + (position + 0.5) * segment;
        let drop_centre_y = self.height() / 2.0;
        let drop_fill = configuration.primary.opacity(if configuration.reduce_transparency { 0.22 } else { 0.13 });

        let veil = configuration.background;
        let backdrop = if configuration.reduce_transparency {
            div().absolute().inset_0().rounded_full().bg(veil).into_any_element()
        } else {
            if self.width() > 0.0 {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .child(
                        div()
                            .w(px(self.width()))
                            .h(px(self.height()))
                            .rounded_full()
                            .liquid_chrome_rounded_rectangle(self.height() / 2.0),
                    )
                    .into_any_element()
            } else {
                div().absolute().inset_0().rounded_full().bg(veil).into_any_element()
            }
        };
        let drop = div()
            .absolute()
            .left(px(drop_centre_x - drop_width / 2.0))
            .top(px(drop_centre_y - drop_height / 2.0))
            .w(px(drop_width))
            .h(px(drop_height))
            .rounded_full()
            .bg(drop_fill);

        let mut root = div()
            .id("map-mode-switch")
            .relative()
            .size_full()
            .occlude()
            .cursor_pointer()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    let x = this.local_x(event.position.x);
                    this.begin_tracking(x, cx);
                }),
            )
            .child(backdrop)
            .child(drop);
        for (index, mode) in MapPanelMode::ALL.into_iter().enumerate() {
            let color = if index == self.preview_index { configuration.primary } else { configuration.secondary };
            let title = configuration.titles.get(index).cloned().unwrap_or_default();
            root = root.child(self.label(index, mode, title, color));
        }

        let bounds_cell = self.bounds.clone();
        let tracking = self.drag_x.is_some();
        let weak = cx.entity().downgrade();
        let resized_target = weak.clone();
        root.child(
            canvas(
                move |bounds, window, _| {
                    let previous = bounds_cell.replace(Some(bounds));
                    if previous.map(|previous| previous.size) != Some(bounds.size) {
                        window.on_next_frame(move |_, cx| {
                            let _ = resized_target.update(cx, |_, cx| cx.notify());
                        });
                    }
                },
                move |_, _, window, _| {
                    if !tracking {
                        return;
                    }
                    let move_target = weak.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Capture || !event.dragging() {
                            return;
                        }
                        let _ = move_target.update(cx, |this, cx| {
                            let x = this.local_x(event.position.x);
                            this.continue_tracking(x, cx);
                        });
                    });
                    let up_target = weak.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                            return;
                        }
                        let _ = up_target.update(cx, |this, cx| {
                            if this.drag_x.is_none() {
                                return;
                            }
                            let x = this.local_x(event.position.x);
                            this.end_tracking(x, window, cx);
                        });
                    });
                },
            )
            .absolute()
            .size_full(),
        )
        .into_any_element()
    }
}

#[derive(IntoElement)]
pub struct MapTrackLinesButton {
    is_on: bool,
    action: MapPanelAction,
}

impl MapTrackLinesButton {
    pub fn new(is_on: bool, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { is_on, action: Rc::new(action) }
    }
}

impl RenderOnce for MapTrackLinesButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        map_toggle_glass_button(
            "map-track-lines-button",
            "point.topleft.down.to.point.bottomright.curvepath.fill",
            tr("map_card_tracks"),
            self.is_on,
            self.action,
            &theme,
        )
    }
}

fn map_toggle_glass_button(
    id: &'static str,
    symbol: &'static str,
    label: String,
    is_on: bool,
    action: MapPanelAction,
    theme: &AppTheme,
) -> impl IntoElement + use<> {
    let tint = if is_on { theme.primary._100 } else { theme.text.secondary };
    let ring = if is_on { theme.primary._100 } else { gpui::transparent_black() };
    let side = PageHeaderMetrics::TOOLBAR_BUTTON_SIZE;
    div()
        .relative()
        .size(px(side))
        .child(
            LiquidGlassButton::new(id, LiquidGlassButtonShape::Circle)
                .size(side)
                .accessibility_label(Some(label))
                .on_press(move |window, cx| action(window, cx))
                .children(IconUi::sized(symbol, px(18.0), tint, false)),
        )
        .child(div().absolute().inset_0().rounded_full().border_2().border_color(ring))
}
