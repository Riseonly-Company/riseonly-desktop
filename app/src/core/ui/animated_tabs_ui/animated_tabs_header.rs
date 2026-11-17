use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    App, Background, ElementId, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, Path,
    Pixels, Point, RenderOnce, ScrollHandle, ScrollWheelEvent, SharedString, StatefulInteractiveElement, TextRun,
    TouchPhase, WeakEntity, Window, canvas, linear_color_stop, linear_gradient, point, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::theme::theme_store::ThemeStore;
use rise_theme::{AppTheme, alpha};
use rise_ui::IconUi;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsMetrics, TabHeaderConfig, TabHeaderCustomEmoji};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::swiftui::animated::Animated;
use rise_ui::liquid_chrome::{LiquidChrome, chrome_surface_depth};
use rise_ui::swiftui::motion::MotionSpec;
use rise_ui::swiftui::prelude::*;

use super::animated_tabs_pager::AnimatedTabsPagerLiveState;
use super::animated_tabs_pager_gesture_gate::{AnimatedTabsPagerAxisOwner, animated_tabs_pager_axis_owner};
use crate::core::media_viewer::media_viewer_video_scrubber::rounded_rect_path;
use crate::core::ui::formatted_text_ui::FormattedTextStyle;
use crate::modules::chat::components::chat::bubble::inline_custom_emoji_text::InlineCustomEmojiText;
use crate::modules::chat::stores::message::message_actions::message_types::MessageCustomEmojiEntity;

const NARROW_STRIP_WIDTH: f32 = 260.0;
const NARROW_STRIP_ARROW_WIDTH: f32 = 24.0;
const NARROW_STRIP_FADE_SHARE: f32 = 0.12;
const STEPPER_ARROW_WIDTH: f32 = 20.0;

pub type AnimatedTabsDragProgress = Rc<dyn Fn(&App) -> f32>;
pub type AnimatedTabsDragBaseIndex = Rc<dyn Fn(&App) -> Option<i64>>;
/// animation the last progress write ran under
/// none settles with releaseAnimation
pub type AnimatedTabsDragReleaseAnimation = Rc<dyn Fn(&App) -> Option<MotionSpec>>;
pub type AnimatedTabsTapTab = Rc<dyn Fn(i64, &mut Window, &mut App)>;
pub type AnimatedTabsSuppressesTap = Rc<dyn Fn(&App) -> bool>;

/// rendered width of each tab, collected from labels after layout
/// plus a revision bumped per accepted width so a frame spots a change without walking the map
#[derive(Clone, Debug, PartialEq)]
struct TabWidthKey(HashMap<i64, f32>, u64);

impl TabWidthKey {
    fn default_value() -> Self {
        Self(HashMap::new(), 0)
    }

    /// each label reports alone after its own layout so the latest width replaces the entry
    /// sub pixel jitter is ignored so a report never starts another frame
    fn reduce(&mut self, id: i64, width: f32) -> bool {
        if self.0.get(&id).is_some_and(|current| (current - width).abs() <= 1.01) {
            return false;
        }
        self.0.insert(id, width);
        self.1 = self.1.wrapping_add(1);
        true
    }
}

/// own state of the header, tab widths and the strip scroll position
struct AnimatedTabsHeaderState {
    tab_widths: TabWidthKey,
    outer_width: f32,
    scroll_handle: ScrollHandle,
    strip_scroll: Animated<f32>,
    scroll_gesture_start: Option<f32>,
    user_scrolled: bool,
}

impl AnimatedTabsHeaderState {
    fn new() -> Self {
        Self {
            tab_widths: TabWidthKey::default_value(),
            outer_width: 0.0,
            scroll_handle: ScrollHandle::new(),
            strip_scroll: Animated::new(0.0),
            user_scrolled: false,
            scroll_gesture_start: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LayoutMode {
    Distribute,
    Compact { tab_padding: f32 },
    Scroll,
}

#[derive(IntoElement)]
pub struct AnimatedTabsHeader {
    id: ElementId,
    tabs: Vec<TabHeaderConfig>,
    active_tab: Binding<i64>,
    drag_progress: AnimatedTabsDragProgress,
    drag_base_index: AnimatedTabsDragBaseIndex,
    drag_release_animation: AnimatedTabsDragReleaseAnimation,
    horizontal_padding: f32,
    on_tap_tab: AnimatedTabsTapTab,
    shows_pill_background: bool,
    suppresses_tap: AnimatedTabsSuppressesTap,
    force_scrollable: bool,
    max_tab_width_fraction: Option<f32>,
    fade_surface: Option<Hsla>,
    arrows_step_selection: bool,
}

impl AnimatedTabsHeader {
    pub fn new(
        id: impl Into<ElementId>,
        tabs: Vec<TabHeaderConfig>,
        active_tab: Binding<i64>,
        drag_progress: AnimatedTabsDragProgress,
        horizontal_padding: f32,
        on_tap_tab: AnimatedTabsTapTab,
    ) -> Self {
        Self {
            id: id.into(),
            tabs,
            active_tab,
            drag_progress,
            drag_base_index: Rc::new(|_| None),
            drag_release_animation: Rc::new(|_| None),
            horizontal_padding,
            on_tap_tab,
            shows_pill_background: true,
            suppresses_tap: Rc::new(|_| false),
            force_scrollable: false,
            max_tab_width_fraction: None,
            fade_surface: None,
            arrows_step_selection: false,
        }
    }

    pub fn arrows_step_selection(mut self, steps: bool) -> Self {
        self.arrows_step_selection = steps;
        self
    }

    pub fn fade_surface(mut self, surface: Hsla) -> Self {
        self.fade_surface = Some(surface);
        self
    }

    pub fn drag_base_index(mut self, drag_base_index: AnimatedTabsDragBaseIndex) -> Self {
        self.drag_base_index = drag_base_index;
        self
    }

    pub fn drag_release_animation(mut self, drag_release_animation: AnimatedTabsDragReleaseAnimation) -> Self {
        self.drag_release_animation = drag_release_animation;
        self
    }

    pub fn shows_pill_background(mut self, shows: bool) -> Self {
        self.shows_pill_background = shows;
        self
    }

    pub fn suppresses_tap(mut self, suppresses_tap: AnimatedTabsSuppressesTap) -> Self {
        self.suppresses_tap = suppresses_tap;
        self
    }

    pub fn force_scrollable(mut self, force: bool) -> Self {
        self.force_scrollable = force;
        self
    }

    pub fn max_tab_width_fraction(mut self, fraction: Option<f32>) -> Self {
        self.max_tab_width_fraction = fraction;
        self
    }

    /// decided from measured titles, not rendered widths since those depend on the mode itself
    /// every title is measured at the active weight so switching tabs never flips the mode
    fn layout_mode(&self, outer_width: f32, window: &Window) -> LayoutMode {
        if self.force_scrollable || self.tabs.is_empty() {
            return LayoutMode::Scroll;
        }
        let strip_width = outer_width - f32::from(AnimatedTabsMetrics::BAR_INNER_PADDING) * 2.0;
        if strip_width <= 0.0 {
            return LayoutMode::Scroll;
        }
        let font = Font::system_weight(f32::from(AnimatedTabsMetrics::TAB_FONT_SIZE), FontWeight::SEMIBOLD);
        let cap = self.max_tab_text_width(outer_width);
        let (text_width, widest) = self.tabs.iter().fold((0.0_f32, 0.0_f32), |(sum, widest), tab| {
            let natural = Self::measured_text_width(&tab.text, font, window).ceil();
            let width = cap.map_or(natural, |cap| natural.min(cap));
            (sum + width, widest.max(width))
        });
        let count = self.tabs.len() as f32;
        let fits_all = text_width + count * f32::from(AnimatedTabsMetrics::TAB_HORIZONTAL_PADDING) * 2.0 <= strip_width;
        if fits_all && widest <= strip_width / count {
            return LayoutMode::Distribute;
        }
        let room = strip_width - text_width - f32::from(AnimatedTabsMetrics::COMPACT_ROUNDING_SLACK);
        let padding = (room / (count * 2.0)).floor();
        if padding < f32::from(AnimatedTabsMetrics::COMPACT_MIN_TAB_PADDING) {
            return LayoutMode::Scroll;
        }
        if fits_all {
            return LayoutMode::Compact { tab_padding: padding };
        }
        LayoutMode::Compact { tab_padding: padding.min(f32::from(AnimatedTabsMetrics::TAB_HORIZONTAL_PADDING)) }
    }

    /// text width measured through the same text stack that draws the label
    fn measured_text_width(text: &SharedString, font: Font, window: &Window) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let run = TextRun {
            len: text.len(),
            font: gpui::Font {
                family: font.family(),
                features: font.features(),
                fallbacks: None,
                weight: font.weight,
                style: font.style(),
            },
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(window.text_system().shape_line(text.clone(), px(font.size), &[run], None).width())
    }

    fn max_tab_text_width(&self, outer_width: f32) -> Option<f32> {
        let fraction = self.max_tab_width_fraction?;
        let strip_width = outer_width - f32::from(AnimatedTabsMetrics::BAR_INNER_PADDING) * 2.0;
        Some(
            f32::from(AnimatedTabsMetrics::TAB_MIN_TAP_WIDTH)
                .max(strip_width * fraction - f32::from(AnimatedTabsMetrics::TAB_HORIZONTAL_PADDING) * 2.0),
        )
    }

    fn tab_leading_edges(tabs: &[TabHeaderConfig], tab_widths: &TabWidthKey) -> Vec<f32> {
        let mut edges = Vec::with_capacity(tabs.len());
        let mut x = 0.0;
        for tab in tabs {
            let Some(width) = tab_widths.0.get(&tab.id) else { return Vec::new() };
            edges.push(x);
            x += width;
        }
        edges
    }

    /// scroll offset that centers the active tab, clamped to the strip
    fn centred_offset(tabs: &[TabHeaderConfig], tab_widths: &TabWidthKey, active_tab: i64, handle: &ScrollHandle) -> Option<f32> {
        let edges = Self::tab_leading_edges(tabs, tab_widths);
        let index = tabs.iter().position(|tab| tab.id == active_tab)?;
        let edge = *edges.get(index)?;
        let width = *tab_widths.0.get(&active_tab)?;
        let viewport = f32::from(handle.bounds().size.width);
        if viewport <= 0.0 {
            return None;
        }
        let max_x = f32::from(handle.max_offset().x).max(0.0);
        let edge = if LayoutDirection::current().is_right_to_left() { viewport + max_x - edge - width } else { edge };
        Some((edge + width / 2.0 - viewport / 2.0).clamp(0.0, max_x))
    }

    fn page_strip(state: &mut AnimatedTabsHeaderState, leading: bool, cx: &mut gpui::Context<AnimatedTabsHeaderState>) {
        let viewport = f32::from(state.scroll_handle.bounds().size.width);
        let max_x = f32::from(state.scroll_handle.max_offset().x).max(0.0);
        if viewport <= 0.0 || max_x <= 0.0 {
            return;
        }
        let current = -f32::from(state.scroll_handle.offset().x);
        let step = viewport * 0.75;
        let target = if leading { (current - step).max(0.0) } else { (current + step).min(max_x) };
        state.user_scrolled = true;
        state.scroll_gesture_start = None;
        state.strip_scroll.jump(current);
        state.strip_scroll.set_in(target, AnimatedTabsMetrics::release_animation(), RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn edge_fades(is_scroll: bool, handle: &ScrollHandle) -> (bool, bool) {
        if !is_scroll {
            return (false, false);
        }
        let offset = -f32::from(handle.offset().x);
        let max_offset = f32::from(handle.max_offset().x);
        (offset > 1.0, max_offset > 1.0 && offset < max_offset - 1.0)
    }

    /// solid color of the pill, the chrome fill every button and pill shares
    fn pill_fill(theme: &AppTheme, depth: u32) -> Hsla {
        LiquidChrome::surface_fill(theme, depth)
    }

    fn bar_background(theme: &AppTheme, depth: u32) -> Div {
        div()
            .absolute()
            .inset_0()
            .capsule()
            .bg(Self::pill_fill(theme, depth))
            .child(centred_stroke(alpha(theme.border._200, 0.55), 0.5, Shape::Capsule))
    }
}

impl RenderOnce for AnimatedTabsHeader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Rc::new(ThemeStore::shared(cx).read(cx).current_theme().clone());
        let chrome_depth = chrome_surface_depth(cx);
        let right_to_left = layout_direction(cx) == LayoutDirection::RightToLeft;
        let active_tab = self.active_tab.get(cx);
        let id = self.id.clone();

        let state = window.with_global_id(id.clone(), |_, window| {
            window.use_keyed_state("animated-tabs-header-state", cx, |_, _| AnimatedTabsHeaderState::new())
        });
        let (outer_width, tab_widths, scroll_handle) = {
            let state = state.read(cx);
            (
                state.outer_width - self.horizontal_padding * 2.0,
                state.tab_widths.clone(),
                state.scroll_handle.clone(),
            )
        };
        let mode = self.layout_mode(outer_width, window);
        let strip_width = outer_width - f32::from(AnimatedTabsMetrics::BAR_INNER_PADDING) * 2.0;
        let stepper = self.arrows_step_selection && mode == LayoutMode::Scroll && strip_width < NARROW_STRIP_WIDTH;
        let scroll_tab_padding = if stepper {
            f32::from(AnimatedTabsMetrics::COMPACT_MIN_TAB_PADDING)
        } else {
            f32::from(AnimatedTabsMetrics::TAB_HORIZONTAL_PADDING)
        };

        let row = match mode {
            LayoutMode::Distribute => self.tabs_row(&theme, &tab_widths, active_tab, true, strip_width, None, f32::from(AnimatedTabsMetrics::TAB_HORIZONTAL_PADDING), &state, chrome_depth, false),
            LayoutMode::Compact { tab_padding } => self.tabs_row(
                &theme,
                &tab_widths,
                active_tab,
                false,
                0.0,
                self.max_tab_text_width(outer_width),
                tab_padding,
                &state,
                chrome_depth,
                false,
            ),
            LayoutMode::Scroll => self.tabs_row(
                &theme,
                &tab_widths,
                active_tab,
                false,
                0.0,
                self.max_tab_text_width(outer_width),
                scroll_tab_padding,
                &state,
                chrome_depth,
                stepper,
            ),
        };

        let content = if mode == LayoutMode::Scroll {
            let tabs = self.tabs.clone();
            let change_state = state.clone();
            let change_tabs = tabs.clone();
            window.with_global_id(id.clone(), |_, window| {
                on_change("animated-tabs-header-active", active_tab, true, window, cx, move |old, new_value, _, cx| {
                    let first = old == new_value;
                    let new_value = *new_value;
                    change_state.update(cx, |state, cx| {
                        state.user_scrolled = false;
                        let Some(target) =
                            Self::centred_offset(&change_tabs, &state.tab_widths, new_value, &state.scroll_handle)
                        else {
                            return;
                        };
                        if first {
                            state.strip_scroll.jump(target);
                            state.scroll_handle.set_offset(point(px(-target), px(0.0)));
                        } else {
                            let current = -f32::from(state.scroll_handle.offset().x);
                            if (target - current).abs() <= 0.5 {
                                state.strip_scroll.jump(target);
                                return;
                            }
                            state.strip_scroll.jump(current);
                            state.strip_scroll.set_in(
                                target,
                                AnimatedTabsMetrics::release_animation(),
                                RiseClock::monotonic(),
                                cx,
                            );
                        }
                        cx.notify();
                    });
                });
            });
            let width_state = state.clone();
            let width_tabs = tabs.clone();
            window.with_global_id(id.clone(), |_, window| {
                on_change("animated-tabs-header-width", outer_width.to_bits(), false, window, cx, move |_, _, _, cx| {
                    width_state.update(cx, |state, cx| {
                        if let Some(target) =
                            Self::centred_offset(&width_tabs, &state.tab_widths, active_tab, &state.scroll_handle)
                        {
                            state.strip_scroll.jump(target);
                            state.scroll_handle.set_offset(point(px(-target), px(0.0)));
                            cx.notify();
                        }
                    });
                });
            });

            let widths_state = state.clone();
            let widths_tabs = tabs.clone();
            let widths_revision = tab_widths.1;
            window.with_global_id(id.clone(), |_, window| {
                on_change("animated-tabs-header-tab-widths", widths_revision, false, window, cx, move |_, _, _, cx| {
                    widths_state.update(cx, |state, cx| {
                        let Some(target) =
                            Self::centred_offset(&widths_tabs, &state.tab_widths, active_tab, &state.scroll_handle)
                        else {
                            return;
                        };
                        let current = -f32::from(state.scroll_handle.offset().x);
                        if (target - current).abs() <= 0.5 {
                            state.strip_scroll.jump(target);
                            return;
                        }
                        state.strip_scroll.jump(current);
                        state.strip_scroll.set_in(
                            target,
                            AnimatedTabsMetrics::release_animation(),
                            RiseClock::monotonic(),
                            cx,
                        );
                        cx.notify();
                    });
                });
            });

            let now = RiseClock::monotonic();
            let scrolling = state.read(cx).strip_scroll.is_running(now);
            if scrolling {
                let offset = state.read(cx).strip_scroll.frame_value(now, window);
                scroll_handle.set_offset(point(px(-offset), px(0.0)));
            } else {
                let settle_tabs = tabs.clone();
                state.update(cx, |state, _| {
                    if state.user_scrolled {
                        return;
                    }
                    let Some(target) =
                        Self::centred_offset(&settle_tabs, &state.tab_widths, active_tab, &state.scroll_handle)
                    else {
                        return;
                    };
                    let current = -f32::from(state.scroll_handle.offset().x);
                    if (target - current).abs() > 0.5 {
                        state.strip_scroll.jump(target);
                        state.scroll_handle.set_offset(point(px(-target), px(0.0)));
                    }
                });
            }

            let snap_state = state.clone();
            let snap_tabs = tabs;
            let strip_owner = AnimatedTabsPagerAxisOwner::ScrollView(scroll_handle.clone());
            let strip = div()
                .id("animated-tabs-header-scroll")
                .w_full()
                .h_full()
                .flex()
                .items_center()
                .overflow_x_scroll()
                .restrict_scroll_to_axis()
                .track_scroll(&scroll_handle)
                .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                    if !event.delta.precise() {
                        let delta = event.delta.pixel_delta(window.line_height());
                        let wheel = f32::from(delta.y);
                        if f32::from(delta.x) == 0.0 && wheel != 0.0 {
                            let handled = snap_state.update(cx, |state, cx| {
                                let max_x = f32::from(state.scroll_handle.max_offset().x);
                                if max_x <= 0.0 {
                                    return false;
                                }
                                let current = f32::from(state.scroll_handle.offset().x);
                                let next = (current + wheel).clamp(-max_x, 0.0);
                                state.user_scrolled = true;
                                state.scroll_gesture_start = None;
                                state.strip_scroll.jump(-next);
                                state.scroll_handle.set_offset(point(px(next), px(0.0)));
                                cx.notify();
                                true
                            });
                            if handled {
                                cx.stop_propagation();
                            }
                            return;
                        }
                    }
                    snap_state.update(cx, |state, cx| {
                        state.user_scrolled = true;
                        let offset = -f32::from(state.scroll_handle.offset().x);
                        match event.touch_phase {
                            TouchPhase::Started => state.scroll_gesture_start = Some(offset),
                            TouchPhase::Ended => {
                                let Some(start) = state.scroll_gesture_start.take() else { return };
                                let behavior = AnimatedTabsStripSnapBehavior {
                                    tab_edges: Self::tab_leading_edges(&snap_tabs, &state.tab_widths),
                                    snaps_to_tabs: !right_to_left,
                                };
                                let viewport = f32::from(state.scroll_handle.bounds().size.width);
                                let content_width = viewport + f32::from(state.scroll_handle.max_offset().x);
                                let target = behavior.update_target(offset, start, viewport, content_width);
                                if (target - offset).abs() > 0.5 {
                                    state.strip_scroll.jump(offset);
                                    state.strip_scroll.set_in(
                                        target,
                                        AnimatedTabsMetrics::release_animation(),
                                        RiseClock::monotonic(),
                                        cx,
                                    );
                                    cx.notify();
                                }
                            }
                            TouchPhase::Moved | TouchPhase::Cancelled => {}
                        }
                    });
                })
                .child(row.when(right_to_left, |row| row.min_w_full()));
            animated_tabs_pager_axis_owner(strip_owner, strip).into_any_element()
        } else {
            row.into_any_element()
        };

        let is_scroll = mode == LayoutMode::Scroll;
        let (fade_leading, fade_trailing) = Self::edge_fades(is_scroll, &scroll_handle);
        let (shows_fade_leading, shows_fade_trailing) =
            if stepper { (false, false) } else { (fade_leading, fade_trailing) };
        let probe_state = state.clone();
        let probe_handle = scroll_handle.clone();
        let fade_probe = canvas(
            move |_, window, cx| {
                if Self::edge_fades(is_scroll, &probe_handle) != (fade_leading, fade_trailing) {
                    window.defer(cx, move |_, cx| probe_state.update(cx, |_, cx| cx.notify()));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0();
        let fade_color = if let Some(surface) = self.fade_surface {
            surface
        } else if self.shows_pill_background {
            Self::pill_fill(&theme, chrome_depth)
        } else {
            theme.bg._100
        };
        let edge_inset = AnimatedTabsMetrics::BAR_INNER_PADDING;
        let edge_radius = AnimatedTabsMetrics::INDICATOR_CORNER_RADIUS;
        let narrow_strip = is_scroll && strip_width < NARROW_STRIP_WIDTH;
        let arrow_width = if stepper {
            px(STEPPER_ARROW_WIDTH)
        } else if narrow_strip {
            px(NARROW_STRIP_ARROW_WIDTH)
        } else {
            px(AnimatedTabsTabLabel::label_height())
        };
        let fade_width = if narrow_strip {
            AnimatedTabsMetrics::STRIP_EDGE_FADE_WIDTH.min(px((strip_width * NARROW_STRIP_FADE_SHARE).floor()))
        } else {
            AnimatedTabsMetrics::STRIP_EDGE_FADE_WIDTH
        };
        let theme_for_arrows = theme.clone();
        let active_index = self.tabs.iter().position(|tab| tab.id == active_tab);
        let neighbour = |leading: bool| -> Option<i64> {
            let index = active_index?;
            let towards_start = leading != right_to_left;
            let next = if towards_start { index.checked_sub(1)? } else { index + 1 };
            self.tabs.get(next).map(|tab| tab.id)
        };
        let steps = self.arrows_step_selection && is_scroll;
        let (arrow_leading, arrow_trailing) =
            if steps { (neighbour(true).is_some(), neighbour(false).is_some()) } else { (fade_leading, fade_trailing) };
        let edge_fade = |leading: bool| {
            let (from, to) = if leading { (1.0, 0.0) } else { (0.0, 1.0) };
            div()
                .absolute()
                .top(edge_inset)
                .bottom(edge_inset)
                .w(fade_width)
                .when(leading, |fade| fade.left(edge_inset + if arrow_leading { arrow_width } else { px(0.0) }))
                .when(!leading, |fade| fade.right(edge_inset + if arrow_trailing { arrow_width } else { px(0.0) }))
                .bg(linear_gradient(
                    90.0,
                    linear_color_stop(alpha(fade_color, from), 0.0),
                    linear_color_stop(alpha(fade_color, to), 1.0),
                ))
        };

        let edge_arrow = |leading: bool| {
            let arrow_state = state.clone();
            let step_target = if steps { neighbour(leading) } else { None };
            let step_tap = self.on_tap_tab.clone();
            let glyph = if leading { "chevron.left" } else { "chevron.right" };
            div()
                .absolute()
                .top(edge_inset)
                .bottom(edge_inset)
                .w(arrow_width)
                .when(leading, |arrow| arrow.left(edge_inset).rounded_tl(edge_radius).rounded_bl(edge_radius))
                .when(!leading, |arrow| arrow.right(edge_inset).rounded_tr(edge_radius).rounded_br(edge_radius))
                .bg(fade_color)
                .block_mouse_except_scroll()
                .child(
                    plain(SharedString::from(if leading { "animated-tabs-page-back" } else { "animated-tabs-page-forward" }))
                        .on_press(move |window, cx| {
                            if let Some(tab_id) = step_target {
                                step_tap(tab_id, window, cx);
                                return;
                            }
                            arrow_state.update(cx, |state, cx| Self::page_strip(state, leading, cx));
                        })
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized(glyph, px(12.0), theme_for_arrows.text.secondary, false)),
                )
        };

        let cut_leading = !stepper && arrow_leading;
        let cut_trailing = !stepper && arrow_trailing;
        let content = if cut_leading || cut_trailing {
            let leading_cut = if cut_leading { edge_radius } else { px(0.0) };
            let trailing_cut = if cut_trailing { edge_radius } else { px(0.0) };
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(edge_inset + leading_cut)
                .right(edge_inset + trailing_cut)
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(-leading_cut)
                        .right(-trailing_cut)
                        .flex()
                        .items_center()
                        .when(right_to_left, |row| row.justify_end())
                        .child(content),
                )
                .into_any_element()
        } else {
            content.into_any_element()
        };

        let measure_state = state.clone();
        let bar = div()
            .relative()
            .h(AnimatedTabsMetrics::BAR_HEIGHT)
            .mx(px(self.horizontal_padding))
            .when(self.shows_pill_background, |bar| bar.child(Self::bar_background(&theme, chrome_depth)))
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .items_center()
                    .when(right_to_left, |inner| inner.justify_end())
                    .px(AnimatedTabsMetrics::BAR_INNER_PADDING)
                    .when(stepper && arrow_leading, |inner| inner.pl(edge_inset + arrow_width))
                    .when(stepper && arrow_trailing, |inner| inner.pr(edge_inset + arrow_width))
                    .child(content)
                    .when(shows_fade_leading, |inner| inner.child(edge_fade(true)))
                    .when(shows_fade_trailing, |inner| inner.child(edge_fade(false)))
                    .when(arrow_leading, |inner| inner.child(edge_arrow(true)))
                    .when(arrow_trailing, |inner| inner.child(edge_arrow(false)))
                    .child(fade_probe),
            );

        div()
            .id(id)
            .relative()
            .w_full()
            .h(AnimatedTabsMetrics::BAR_HEIGHT)
            .child(bar)
            .child(measure("animated-tabs-header-width-probe", move |bounds, _window, cx| {
                let width = f32::from(bounds.size.width);
                measure_state.update(cx, |state, cx| {
                    if state.outer_width != width {
                        state.outer_width = width;
                        cx.notify();
                    }
                });
            }))
    }
}

impl AnimatedTabsHeader {
    #[allow(clippy::too_many_arguments)]
    fn tabs_row(
        &self,
        theme: &Rc<AppTheme>,
        tab_widths: &TabWidthKey,
        active_tab: i64,
        distribute: bool,
        total_width: f32,
        max_text_width: Option<f32>,
        tab_padding: f32,
        state: &gpui::Entity<AnimatedTabsHeaderState>,
        chrome_depth: u32,
        solo_active: bool,
    ) -> Div {
        let right_to_left = LayoutDirection::current().is_right_to_left();
        let measured_width = if distribute && !self.tabs.is_empty() {
            (total_width / self.tabs.len() as f32).max(f32::from(AnimatedTabsMetrics::TAB_MIN_TAP_WIDTH))
        } else {
            0.0
        };
        let labels = self.tabs.iter().map(|tab| {
            let on_tap_tab = self.on_tap_tab.clone();
            let tab_id = tab.id;
            let width_state = state.clone();
            let hidden = solo_active && tab.id != active_tab;
            let label = AnimatedTabsTabLabel {
                tab: tab.clone(),
                active: tab.id == active_tab,
                explicit_width: if distribute { Some(measured_width) } else { None },
                max_text_width: if distribute { None } else { max_text_width },
                tab_padding,
                on_tap: Rc::new(move |window, cx| on_tap_tab(tab_id, window, cx)),
                suppresses_tap: self.suppresses_tap.clone(),
                theme: theme.clone(),
                on_width: Rc::new(move |width, cx| {
                    width_state.update(cx, |state, cx| {
                        if state.tab_widths.reduce(tab_id, width) && !distribute {
                            cx.notify();
                        }
                    });
                }),
            }
            .guide_anchor(tab.guide_anchor);
            if hidden {
                div().flex_none().invisible().child(label).into_any_element()
            } else {
                label.into_any_element()
            }
        });

        let indicator = AnimatedTabsIndicator {
            id: ElementId::Name("animated-tabs-indicator".into()),
            tabs: self.tabs.clone(),
            active_tab,
            tab_widths: tab_widths.0.clone(),
            distribute,
            total_width,
            drag_progress: self.drag_progress.clone(),
            drag_base_index: self.drag_base_index.clone(),
            drag_release_animation: self.drag_release_animation.clone(),
            theme: theme.clone(),
            chrome_depth,
        };

        div()
            .relative()
            .flex()
            .when(right_to_left, |row| row.flex_row_reverse())
            .when(!right_to_left, |row| row.flex_row())
            .items_center()
            .when(distribute, |row| row.w_full())
            .when(!distribute, |row| row.flex_none())
            .when(!solo_active, |row| row.child(indicator))
            .children(labels)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct IndicatorGeometry {
    x: f32,
    width: f32,
}

/// animation state of the pill
struct AnimatedTabsIndicatorState {
    x: Animated<f32>,
    width: Animated<f32>,
    active_index: Option<i64>,
    progress: f32,
    painted: Rc<Cell<Option<AnimatedTabsIndicatorPaint>>>,
    swap_armed: bool,
    swaps_ok: bool,
}

#[derive(Clone, Copy, Debug)]
struct AnimatedTabsIndicatorPaint {
    row_width: f32,
    right_to_left: bool,
    height: f32,
    fill: Hsla,
    edge: Hsla,
}

impl AnimatedTabsIndicatorState {
    fn shown(&self, now: std::time::Duration) -> (f32, f32, bool) {
        (self.x.value(now), self.width.value(now), self.x.is_running(now) || self.width.is_running(now))
    }
}

#[derive(IntoElement)]
struct AnimatedTabsIndicator {
    id: ElementId,
    tabs: Vec<TabHeaderConfig>,
    active_tab: i64,
    tab_widths: HashMap<i64, f32>,
    distribute: bool,
    total_width: f32,
    drag_progress: AnimatedTabsDragProgress,
    drag_base_index: AnimatedTabsDragBaseIndex,
    drag_release_animation: AnimatedTabsDragReleaseAnimation,
    theme: Rc<AppTheme>,
    chrome_depth: u32,
}

impl AnimatedTabsIndicator {
    fn resolved_active_index(&self, cx: &App) -> Option<i64> {
        if self.tabs.is_empty() {
            return None;
        }
        if let Some(base) = (self.drag_base_index)(cx) {
            return Some(base.clamp(0, self.tabs.len() as i64 - 1));
        }
        self.tabs.iter().position(|tab| tab.id == self.active_tab).map(|index| index as i64)
    }

    fn indicator_geometry(&self, active_index: i64, progress: f32) -> Option<IndicatorGeometry> {
        if self.tabs.is_empty() {
            return None;
        }
        let distributed_width =
            (self.total_width / self.tabs.len() as f32).max(f32::from(AnimatedTabsMetrics::TAB_MIN_TAP_WIDTH));
        let width = |index: i64| -> f32 {
            if self.distribute {
                return distributed_width;
            }
            usize::try_from(index)
                .ok()
                .and_then(|index| self.tabs.get(index))
                .and_then(|tab| self.tab_widths.get(&tab.id).copied())
                .unwrap_or(f32::from(AnimatedTabsMetrics::TAB_MIN_TAP_WIDTH))
        };

        let mut cur_x = 0.0;
        for index in 0..active_index {
            cur_x += width(index);
        }
        let cur_w = width(active_index);

        let abs_p = progress.abs().min(1.0);
        let count = self.tabs.len() as i64;
        let neighbor_index = if progress < -0.001 && active_index - 1 >= 0 {
            Some(active_index - 1)
        } else if progress > 0.001 && active_index + 1 < count {
            Some(active_index + 1)
        } else {
            None
        };

        let Some(n_idx) = neighbor_index else {
            return Some(IndicatorGeometry { x: cur_x, width: cur_w });
        };

        let n_w = width(n_idx);
        let n_x = if n_idx > active_index { cur_x + cur_w } else { cur_x - n_w };

        Some(IndicatorGeometry { x: cur_x + (n_x - cur_x) * abs_p, width: cur_w + (n_w - cur_w) * abs_p })
    }
}

impl RenderOnce for AnimatedTabsIndicator {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let right_to_left = layout_direction(cx) == LayoutDirection::RightToLeft;
        let progress = (self.drag_progress)(cx);
        let Some(active_index) = self.resolved_active_index(cx) else {
            return div().into_any_element();
        };
        let Some(geometry) = self.indicator_geometry(active_index, progress) else {
            return div().into_any_element();
        };

        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| AnimatedTabsIndicatorState {
            x: Animated::new(geometry.x),
            width: Animated::new(geometry.width),
            active_index: Some(active_index),
            progress,
            painted: Rc::new(Cell::new(None)),
            swap_armed: false,
            swaps_ok: true,
        });
        let now = RiseClock::monotonic();
        let reduce_motion = reduce_motion(cx);
        let release_spec = (self.drag_release_animation)(cx).unwrap_or_else(AnimatedTabsMetrics::release_animation);
        let (x, width, running, swaps_ok, painted) = state.update(cx, |state, _| {
            let released = state.progress != 0.0 && progress == 0.0;
            let switched = state.active_index != Some(active_index);
            if switched {
                state.x.set(geometry.x, AnimatedTabsMetrics::release_animation(), now, reduce_motion);
                state.width.set(geometry.width, AnimatedTabsMetrics::release_animation(), now, reduce_motion);
            } else if released {
                state.x.set(geometry.x, release_spec, now, reduce_motion);
                state.width.set(geometry.width, release_spec, now, reduce_motion);
            } else if !state.x.is_running(now) {
                state.x.jump(geometry.x);
                state.width.jump(geometry.width);
            }
            state.active_index = Some(active_index);
            state.progress = progress;
            let (x, width, running) = state.shown(now);
            if !running {
                state.swaps_ok = true;
            }
            state.painted.set(None);
            (x, width, running, state.swaps_ok, state.painted.clone())
        });
        if running {
            if swaps_ok {
                Self::arm_swaps(&state, window, cx);
            } else {
                window.request_animation_frame();
            }
        }

        let height = f32::from(AnimatedTabsMetrics::BAR_HEIGHT) - f32::from(AnimatedTabsMetrics::BAR_INNER_PADDING) * 2.0;
        let fill = LiquidChrome::selected_segment_fill(&self.theme, self.chrome_depth);
        let edge: Hsla = white().opacity(0.12);
        let base = state.entity_id().as_u64();
        canvas(|_, _, _| (), move |bounds, (), window, _| {
            let paint = AnimatedTabsIndicatorPaint {
                row_width: f32::from(bounds.size.width),
                right_to_left,
                height,
                fill,
                edge,
            };
            let Some(swaps) = Self::swaps(base, paint, x, width, bounds.origin) else {
                return;
            };
            for (key, path, color) in swaps {
                window.paint_swappable_path(path, color, key, bounds.origin);
            }
            painted.set(Some(paint));
        })
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .h(px(height))
        .into_any_element()
    }
}

impl AnimatedTabsIndicator {
    fn swap_key(base: u64, shape: u64) -> u64 {
        base ^ (shape << 56)
    }

    /// pill as two filled capsules, the centered half point stroke as an outer one
    /// and the fill inset on top by the inner half of the stroke
    /// offset follows reading direction so in rtl the pill moves from the right edge
    fn swaps(
        base: u64,
        paint: AnimatedTabsIndicatorPaint,
        x: f32,
        width: f32,
        origin: Point<Pixels>,
    ) -> Option<Vec<(u64, Path<Pixels>, Background)>> {
        const HALF_STROKE: f32 = 0.25;
        let left = if paint.right_to_left { paint.row_width - x - width } else { x };
        let edge = rounded_rect_path(
            left - HALF_STROKE,
            -HALF_STROKE,
            width + HALF_STROKE * 2.0,
            paint.height + HALF_STROKE * 2.0,
            origin,
        )?;
        let body = rounded_rect_path(
            left + HALF_STROKE,
            HALF_STROKE,
            width - HALF_STROKE * 2.0,
            paint.height - HALF_STROKE * 2.0,
            origin,
        )?;
        Some(vec![
            (Self::swap_key(base, 5), edge, paint.edge.into()),
            (Self::swap_key(base, 6), body, paint.fill.into()),
        ])
    }

    fn arm_swaps(state: &Entity<AnimatedTabsIndicatorState>, window: &mut Window, cx: &mut App) {
        if state.read(cx).swap_armed {
            return;
        }
        state.update(cx, |state, _| state.swap_armed = true);
        let weak = state.downgrade();
        window.on_next_frame(move |window, cx| Self::swap_step(weak, window, cx));
    }

    fn swap_step(weak: WeakEntity<AnimatedTabsIndicatorState>, window: &mut Window, cx: &mut App) {
        let Some(state) = weak.upgrade() else {
            return;
        };
        let now = RiseClock::monotonic();
        let (step, painted) = {
            let state = state.read(cx);
            (state.shown(now), state.painted.get())
        };
        let Some(paint) = painted else {
            state.update(cx, |state, _| state.swap_armed = false);
            return;
        };
        let (x, width, running) = step;
        let swapped = Self::swaps(state.entity_id().as_u64(), paint, x, width, Point::default())
            .is_some_and(|swaps| window.swap_paths(swaps));
        if !swapped {
            state.update(cx, |state, cx| {
                state.swap_armed = false;
                state.swaps_ok = false;
                cx.notify();
            });
            return;
        }
        if running {
            window.on_next_frame(move |window, cx| Self::swap_step(weak, window, cx));
        } else {
            state.update(cx, |state, _| state.swap_armed = false);
        }
    }
}

/// header that reads the live drag state itself
/// the screen that builds the pages never reads drag progress in its render
#[derive(IntoElement)]
pub struct AnimatedTabsLiveHeader {
    id: ElementId,
    tabs: Vec<TabHeaderConfig>,
    active_tab: Binding<i64>,
    live_drag_state: gpui::Entity<AnimatedTabsPagerLiveState>,
    horizontal_padding: f32,
    shows_pill_background: bool,
    force_scrollable: bool,
    on_tap_tab: AnimatedTabsTapTab,
}

impl AnimatedTabsLiveHeader {
    pub fn new(
        id: impl Into<ElementId>,
        tabs: Vec<TabHeaderConfig>,
        active_tab: Binding<i64>,
        live_drag_state: gpui::Entity<AnimatedTabsPagerLiveState>,
        on_tap_tab: AnimatedTabsTapTab,
    ) -> Self {
        Self {
            id: id.into(),
            tabs,
            active_tab,
            live_drag_state,
            horizontal_padding: 10.0,
            shows_pill_background: true,
            force_scrollable: false,
            on_tap_tab,
        }
    }

    pub fn horizontal_padding(mut self, padding: f32) -> Self {
        self.horizontal_padding = padding;
        self
    }

    pub fn shows_pill_background(mut self, shows: bool) -> Self {
        self.shows_pill_background = shows;
        self
    }

    pub fn force_scrollable(mut self, force: bool) -> Self {
        self.force_scrollable = force;
        self
    }
}

impl RenderOnce for AnimatedTabsLiveHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let progress_state = self.live_drag_state.clone();
        let base_state = self.live_drag_state.clone();
        let release_state = self.live_drag_state.clone();
        let tap_state = self.live_drag_state;
        AnimatedTabsHeader::new(
            self.id,
            self.tabs,
            self.active_tab,
            Rc::new(move |cx| progress_state.read(cx).drag_progress()),
            self.horizontal_padding,
            self.on_tap_tab,
        )
        .drag_base_index(Rc::new(move |cx| base_state.read(cx).drag_base_index()))
        .drag_release_animation(Rc::new(move |cx| release_state.read(cx).drag_release_animation()))
        .shows_pill_background(self.shows_pill_background)
        .suppresses_tap(Rc::new(move |cx| tap_state.read(cx).suppresses_tab_taps()))
        .force_scrollable(self.force_scrollable)
    }
}

struct AnimatedTabsStripSnapBehavior {
    tab_edges: Vec<f32>,
    snaps_to_tabs: bool,
}

impl AnimatedTabsStripSnapBehavior {
    fn update_target(&self, target: f32, start: f32, viewport: f32, content_width: f32) -> f32 {
        let max_x = (content_width - viewport).max(0.0);
        if viewport <= 0.0 || max_x <= 0.0 {
            return target;
        }
        let proposed = target.max(start - viewport).max(0.0).min(start + viewport).min(max_x);
        if !self.snaps_to_tabs || self.tab_edges.is_empty() {
            return proposed;
        }
        let mut best = max_x;
        for &edge in &self.tab_edges {
            if edge < max_x && (edge - proposed).abs() < (best - proposed).abs() {
                best = edge;
            }
        }
        best
    }
}

type AnimatedTabsLabelWidth = Rc<dyn Fn(f32, &mut App)>;

#[derive(IntoElement)]
struct AnimatedTabsTabLabel {
    tab: TabHeaderConfig,
    active: bool,
    explicit_width: Option<f32>,
    max_text_width: Option<f32>,
    tab_padding: f32,
    on_tap: Rc<dyn Fn(&mut Window, &mut App)>,
    suppresses_tap: AnimatedTabsSuppressesTap,
    theme: Rc<AppTheme>,
    on_width: AnimatedTabsLabelWidth,
}

impl AnimatedTabsTabLabel {
    fn label_height() -> f32 {
        f32::from(AnimatedTabsMetrics::BAR_HEIGHT) - f32::from(AnimatedTabsMetrics::BAR_INNER_PADDING) * 2.0
    }
}

impl RenderOnce for AnimatedTabsTabLabel {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let weight = if self.active { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM };
        let fits_share = self.explicit_width.is_some_and(|share| {
            let font = Font::system_weight(f32::from(AnimatedTabsMetrics::TAB_FONT_SIZE), FontWeight::SEMIBOLD);
            AnimatedTabsHeader::measured_text_width(&self.tab.text, font, window).ceil() <= share
        });
        let color = if self.active { self.theme.text.primary } else { self.theme.text.secondary };
        let hover_color = self.theme.text.primary;
        let natural = self.explicit_width.is_none();

        let title = if self.tab.custom_emojis.is_empty() {
            self.tab.text.clone().into_any_element()
        } else {
            let mut style = FormattedTextStyle::new(color, color, self.theme.text.secondary);
            style.font_size = f32::from(AnimatedTabsMetrics::TAB_FONT_SIZE);
            InlineCustomEmojiText::new(
                SharedString::from(format!("animated-tab-title-{}", self.tab.id)),
                self.tab.text.to_string(),
                style,
                self.tab.custom_emojis.iter().map(MessageCustomEmojiEntity::from).collect(),
            )
            .into_any_element()
        };
        let text = div()
            .swiftui_font(Font::system_weight(f32::from(AnimatedTabsMetrics::TAB_FONT_SIZE), weight))
            .text_color(color)
            .whitespace_nowrap()
            .when(!fits_share, |text| text.text_ellipsis())
            .when_some(self.max_text_width.or(self.explicit_width).filter(|_| !fits_share), |text, max| {
                text.max_w(px(max))
            })
            .flex_none()
            .child(title);

        let on_width = self.on_width;
        let suppresses_tap = self.suppresses_tap;
        let on_tap = self.on_tap;
        plain(SharedString::from(format!("animated-tab-{}", self.tab.id)))
            .on_press(move |window, cx| {
                if suppresses_tap(cx) {
                    tracing::warn!(target: "riseonly::animated_tabs", "tab tap suppressed by a pager drag");
                    return;
                }
                on_tap(window, cx);
            })
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .h(px(Self::label_height()))
            .when_some(self.explicit_width, |label, width| label.w(px(width)))
            .when(natural, |label| label.flex_none().px(px(self.tab_padding)))
            .child(div().hover(move |style| style.text_color(hover_color)).child(text))
            .child(measure("animated-tab-width", move |bounds, _window, cx| {
                on_width(f32::from(bounds.size.width), cx);
            }))
    }
}

impl From<&MessageCustomEmojiEntity> for TabHeaderCustomEmoji {
    fn from(entity: &MessageCustomEmojiEntity) -> Self {
        Self {
            offset: entity.offset,
            length: entity.length,
            custom_emoji_id: entity.custom_emoji_id.clone(),
            file_url: entity.file_url.clone(),
            asset_type: entity.asset_type.clone(),
            width: entity.width,
            height: entity.height,
            pack_id: entity.pack_id.clone(),
        }
    }
}

impl From<&TabHeaderCustomEmoji> for MessageCustomEmojiEntity {
    fn from(emoji: &TabHeaderCustomEmoji) -> Self {
        Self {
            offset: emoji.offset,
            length: emoji.length,
            custom_emoji_id: emoji.custom_emoji_id.clone(),
            file_url: emoji.file_url.clone(),
            asset_type: emoji.asset_type.clone(),
            width: emoji.width,
            height: emoji.height,
            pack_id: emoji.pack_id.clone(),
        }
    }
}
