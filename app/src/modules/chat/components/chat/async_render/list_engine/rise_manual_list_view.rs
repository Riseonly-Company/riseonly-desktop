use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::{Index, Range};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AccessibleAction, AnyElement, AnyView, App, AppContext, Bounds, Context, Edges, ElementId, EntityId, Hsla, InteractiveElement,
    IntoElement, ParentElement, PathBuilder, Pixels, Point, Render, RenderImage, Role, Size, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, div, fill, point, px, size, white,
};
use gpui::{
    AnyWindowHandle, FocusHandle, KeyDownEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollHandle,
    ScrollWheelEvent, StyleRefinement, TouchPhase, canvas,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::accessibility::reduce_motion;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::swiftui::environment::{LayoutDirection, layout_direction};
use rise_ui::swiftui::motion::Spring;
use rise_platform::display_cadence::maximum_frames_per_second;
use rise_ui::hold_context_menu_ui::rise_strict_tap_gesture_recognizer::{
    RiseScrollMotionProbe, RiseStrictTapGestureRecognizer, RiseStrictTapGestureRecognizerState,
};

use crate::core::desktop_platform::rise_app_events::{
    RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::list_engine::rise_manual_list_integration::{RiseManualListConformances, RiseManualListInstallCost};
use crate::core::widgets::navigations::main_tab_scroll_to_top::MainTabScrollToTopListMount;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::list_engine::rise_list_jump_debug::{
    RiseListJumpDebug, RiseListJumpDebugRow,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_removal_dust::{
    RiseManualListRemovalDust, RiseManualListRemovalDustHost, RiseManualListRemovalDustSource,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_scroll_view::RiseManualListScrollView;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view_hooks::RiseManualListViewHooks;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListBottomStickyHeader, RiseManualListBottomStickyHeaderLayout, RiseManualListCubicBezier, RiseManualListItem, RiseManualListItemMountPolicy,
    RiseManualListKeyboardPresentationPolicy, RiseManualListScrollAnimationPolicy, RiseManualListScrollMetrics,
    RiseManualListSelectionPresentation, RiseManualListTransaction, RiseManualListTransactionAnchor,
    RiseManualListTransactionEdgeMutation, RiseManualListTransactionJumpAnimation,
    RiseManualListTransactionJumpAnimationDirection, RiseManualListTransactionUpdateAnimation,
    RiseManualListTopStickyHeaderLayout, RiseManualListViewportAnchor, RiseManualListViewportAnchorAnimationPolicy,
    RiseManualListViewportResizeAnimation,
};

fn current_media_time() -> f64 {
    RiseClock::monotonic().as_secs_f64()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseManualListMountEnvelope {
    pub ahead_distance: Pixels,
    pub behind_distance: Pixels,
    pub immediate_ahead_distance: Pixels,
    pub immediate_behind_distance: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseManualListDeferredMountBatch {
    pub maximum_mounts: usize,
    pub time_budget: f64,
}

/// one display frame in seconds of the monotonic clock
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseManualListMountFrame {
    pub timestamp: f64,
    pub target_timestamp: f64,
}

impl RiseManualListMountFrame {
    pub fn new(timestamp: f64, target_timestamp: f64) -> Option<Self> {
        if !timestamp.is_finite() || !target_timestamp.is_finite() || target_timestamp <= timestamp {
            return None;
        }
        Some(Self { timestamp, target_timestamp })
    }

    pub fn duration(&self) -> f64 {
        self.target_timestamp - self.timestamp
    }

    pub fn frames_per_second(&self) -> usize {
        (1.0 / self.duration()).round().clamp(1.0, 240.0) as usize
    }

    pub fn work_deadline(&self) -> f64 {
        self.target_timestamp - (self.duration() * 0.12).clamp(0.001, 0.002)
    }

    pub fn can_start_mount(&self, now: f64, predicted_cost: f64) -> bool {
        let admission_cost = predicted_cost.max(0.000_2).min(self.duration() * 0.5);
        self.work_deadline() - now >= admission_cost
    }
}

pub enum RiseManualListDeferredRecyclePolicy {}

impl RiseManualListDeferredRecyclePolicy {
    pub const QUIET_PERIOD: f64 = 0.5;
    pub const MAXIMUM_IDENTITY_CACHED_ITEMS: usize = 24;
    pub const PREFERRED_GENERIC_OVERFLOW_LIMIT: usize = 16;
    pub const MAXIMUM_RECYCLES_PER_BATCH: usize = 1;
    pub const TIME_BUDGET: f64 = 0.0015;
    pub const PRESERVES_BOUNDED_IDENTITY_CACHE_WHILE_IDLE: bool = true;

    pub fn should_defer_preparation(uses_deferred_mount_policy: bool) -> bool {
        uses_deferred_mount_policy
    }

    pub fn remaining_quiet_time(idle_since: f64, now: f64) -> f64 {
        (Self::QUIET_PERIOD - (now - idle_since).max(0.0)).max(0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseManualListDrawBand {
    top: Pixels,
    bottom: Pixels,
}

impl RiseManualListDrawBand {
    pub const MINIMUM_MARGIN: f32 = 120.0;
    pub const MARGIN_SCREENS: f32 = 0.12;

    /// viewport_height is the clip height, host_origin_y is where the item host sits in it
    pub fn around(viewport_height: Pixels, host_origin_y: Pixels) -> Self {
        let height = f32::from(viewport_height).max(0.0);
        let margin = px((height * Self::MARGIN_SCREENS).max(Self::MINIMUM_MARGIN));
        Self {
            top: Pixels::ZERO - host_origin_y - margin,
            bottom: px(height) - host_origin_y + margin,
        }
    }

    pub fn contains(&self, frame: Bounds<Pixels>) -> bool {
        frame.origin.y < self.bottom && frame.origin.y + frame.size.height > self.top
    }

    /// how far frame lies outside the band and whether it lies below it
    pub fn gap(&self, frame: Bounds<Pixels>) -> (f32, bool) {
        let top = frame.origin.y;
        let bottom = frame.origin.y + frame.size.height;
        if top >= self.bottom {
            (f32::from(top - self.bottom), true)
        } else {
            (f32::from(self.top - bottom).max(0.0), false)
        }
    }
}

pub enum RiseManualListMountEnvelopePolicy {}

impl RiseManualListMountEnvelopePolicy {
    pub const MAXIMUM_FALLBACK_HOSTED_MOUNTS_PER_IDLE_BATCH: usize = 1;
    pub const FAST_SCROLL_VELOCITY_THRESHOLD: f32 = 1_100.0;
    pub const MINIMUM_AHEAD_SCREENS: f32 = 1.25;
    pub const FALLBACK_HOSTED_REUSE_IDENTIFIER_PREFIX: &str = "RiseManualMessageHostingView";

    pub fn default_preload_distance(viewport_height: Pixels) -> Pixels {
        px((f32::from(viewport_height).max(0.0) * 0.58).clamp(360.0, 560.0))
    }

    pub fn resolve(preload_distance: Pixels, viewport_height: Pixels, velocity_y: f32) -> RiseManualListMountEnvelope {
        let preload = f32::from(preload_distance).max(0.0);
        if preload <= 0.0 {
            return RiseManualListMountEnvelope {
                ahead_distance: Pixels::ZERO,
                behind_distance: Pixels::ZERO,
                immediate_ahead_distance: Pixels::ZERO,
                immediate_behind_distance: Pixels::ZERO,
            };
        }

        let height = f32::from(viewport_height).max(1.0);
        let speed = velocity_y.abs().min(12_000.0);
        let fast_progress = ((speed - 500.0) / 4_500.0).clamp(0.0, 1.0);
        let ahead_boost = (height * 0.72).min(speed * 0.065);
        let behind_distance = (preload * (1.0 - fast_progress * 0.45)).max(160.0);
        let immediate_base = preload.min(180.0_f32.max((height * 0.42).min(380.0)).max((speed * 0.045).min(460.0)));
        let immediate_ahead_distance = immediate_base.min(180.0).max(immediate_base * (1.0 - fast_progress * 0.08));
        let immediate_behind_distance = immediate_base.min(120.0).max(immediate_base * (1.0 - fast_progress * 0.42));
        RiseManualListMountEnvelope {
            ahead_distance: px((preload + ahead_boost).max(height * Self::MINIMUM_AHEAD_SCREENS)),
            behind_distance: px(behind_distance),
            immediate_ahead_distance: px(immediate_ahead_distance),
            immediate_behind_distance: px(immediate_behind_distance),
        }
    }

    pub fn deferred_mount_batch(
        _velocity_y: f32,
        frames_per_second: usize,
        _representative_row_height: Pixels,
        is_idle: bool,
    ) -> RiseManualListDeferredMountBatch {
        let frames_per_second = frames_per_second.clamp(30, 240);
        let frame_duration = 1.0 / frames_per_second as f64;
        if !is_idle {
            return RiseManualListDeferredMountBatch {
                maximum_mounts: 1,
                time_budget: (frame_duration * 0.12).clamp(0.001, 0.002),
            };
        }
        RiseManualListDeferredMountBatch {
            maximum_mounts: 2,
            time_budget: (frame_duration * 0.24).clamp(0.0015, 0.004),
        }
    }

    pub fn is_fallback_hosted_row(reuse_identifier: &str) -> bool {
        reuse_identifier
            .strip_prefix(Self::FALLBACK_HOSTED_REUSE_IDENTIFIER_PREFIX)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    }
}

type RiseManualListSelectionToggle = Rc<dyn Fn(String, &mut Window, &mut App)>;

struct RiseManualListSelectionAccessibilityElement {
    activation: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    accessibility_label: String,
    accessibility_value: Option<String>,
    is_selected: bool,
    accessibility_frame_in_container_space: Bounds<Pixels>,
}

impl RiseManualListSelectionAccessibilityElement {
    fn accessibility_activate(&self, window: &mut Window, cx: &mut App) -> bool {
        let Some(activation) = self.activation.as_ref() else {
            return false;
        };
        activation(window, cx);
        true
    }

    fn element(self: &Rc<Self>, id: ElementId) -> AnyElement {
        let frame = self.accessibility_frame_in_container_space;
        let activated = self.clone();
        let mut element = div()
            .id(id)
            .absolute()
            .left(frame.origin.x)
            .top(frame.origin.y)
            .w(frame.size.width)
            .h(frame.size.height)
            .role(Role::Button)
            .aria_label(self.accessibility_label.clone())
            .aria_selected(self.is_selected)
            .on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                activated.accessibility_activate(window, cx);
            });
        if let Some(value) = self.accessibility_value.clone() {
            element = element.aria_value(value);
        }
        element.into_any_element()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct RiseManualListSelectionOverlayViewRow {
    selection_id: String,
    frame: Bounds<Pixels>,
    indicator_center_x: Pixels,
    is_selected: bool,
    accessibility_label: String,
}

/// selection circles drawn over the rows while selection mode is on
/// list paints it above its rows and forwards the pointer only while active
struct RiseManualListSelectionOverlayView {
    tap_gesture: RiseStrictTapGestureRecognizer,
    rows: Vec<RiseManualListSelectionOverlayViewRow>,
    on_toggle: Option<RiseManualListSelectionToggle>,
    indicator_fill_color: Hsla,
    indicator_ovals: Vec<Bounds<Pixels>>,
    checkmark_polylines: Vec<[Point<Pixels>; 3]>,
    is_hidden: bool,
    is_user_interaction_enabled: bool,
    accessibility_elements_hidden: bool,
    accessibility_elements: Option<Vec<Rc<RiseManualListSelectionAccessibilityElement>>>,
}

impl RiseManualListSelectionOverlayView {
    const INDICATOR_DIAMETER: f32 = 28.0;
    const INDICATOR_RING_WIDTH: f32 = 1.8;
    const CHECKMARK_LINE_WIDTH: f32 = 1.35;
    const CHECKMARK_OPACITY: f32 = 0.96;

    fn new() -> Self {
        let mut overlay = Self {
            tap_gesture: RiseStrictTapGestureRecognizer::new(),
            rows: Vec::new(),
            on_toggle: None,
            indicator_fill_color: Hsla::transparent_black(),
            indicator_ovals: Vec::new(),
            checkmark_polylines: Vec::new(),
            is_hidden: true,
            is_user_interaction_enabled: false,
            accessibility_elements_hidden: true,
            accessibility_elements: None,
        };
        overlay.set_active(false);
        overlay
    }

    fn apply(
        &mut self,
        is_active: bool,
        rows: Vec<RiseManualListSelectionOverlayViewRow>,
        tint_color: Hsla,
        visible_rect: Bounds<Pixels>,
        on_toggle: Option<RiseManualListSelectionToggle>,
    ) {
        self.rows = if is_active {
            let mut rows = rows;
            rows.sort_by(|lhs, rhs| lhs.frame.top().cmp(&rhs.frame.top()));
            rows
        } else {
            Vec::new()
        };
        self.on_toggle = if is_active { on_toggle } else { None };
        self.set_active(is_active);

        let mut indicator_ovals = Vec::new();
        let mut checkmark_polylines = Vec::new();
        for row in &self.rows {
            let diameter = px(Self::INDICATOR_DIAMETER.min((f32::from(row.frame.size.height) - 2.0).max(20.0)));
            let outer_rect = Bounds::new(
                point(row.indicator_center_x - diameter * 0.5, row.frame.center().y - diameter * 0.5),
                size(diameter, diameter),
            );
            indicator_ovals.push(outer_rect);

            if row.is_selected {
                let at = |x: f32, y: f32| {
                    point(outer_rect.origin.x + diameter * x, outer_rect.origin.y + diameter * y)
                };
                checkmark_polylines.push([at(0.27, 0.52), at(0.43, 0.68), at(0.75, 0.34)]);
            } else {
                indicator_ovals.push(outer_rect.inset(px(Self::INDICATOR_RING_WIDTH)));
            }
        }

        self.indicator_fill_color = tint_color;
        self.indicator_ovals = indicator_ovals;
        self.checkmark_polylines = checkmark_polylines;
        self.rebuild_accessibility_elements(visible_rect);
    }

    fn update_accessibility(&mut self, visible_rect: Bounds<Pixels>) {
        self.rebuild_accessibility_elements(visible_rect);
    }

    fn set_active(&mut self, active: bool) {
        self.is_hidden = !active;
        self.is_user_interaction_enabled = active;
        self.accessibility_elements_hidden = !active;
        if !active {
            self.accessibility_elements = None;
            self.tap_gesture.reset();
        }
    }

    fn rebuild_accessibility_elements(&mut self, visible_rect: Bounds<Pixels>) {
        if self.is_hidden {
            self.accessibility_elements = None;
            return;
        }
        let select = tr("ctx_select");
        let selected_value = tr("chat_selected_messages");
        let elements = self
            .rows
            .iter()
            .filter(|row| row.frame.intersects(&visible_rect))
            .map(|row| {
                let activation: Option<Rc<dyn Fn(&mut Window, &mut App)>> = self.on_toggle.clone().map(|on_toggle| {
                    let selection_id = row.selection_id.clone();
                    Rc::new(move |window: &mut Window, cx: &mut App| on_toggle(selection_id.clone(), window, cx))
                        as Rc<dyn Fn(&mut Window, &mut App)>
                });
                Rc::new(RiseManualListSelectionAccessibilityElement {
                    activation,
                    accessibility_label: if row.accessibility_label.is_empty() {
                        select.clone()
                    } else {
                        format!("{select}. {}", row.accessibility_label)
                    },
                    accessibility_value: row.is_selected.then(|| selected_value.clone()),
                    is_selected: row.is_selected,
                    accessibility_frame_in_container_space: row.frame,
                })
            })
            .collect();
        self.accessibility_elements = Some(elements);
    }

    /// draws the indicator, ring for an unselected row and disc for a selected one even odd fill
    /// checked mark is stroked with round caps and joins
    fn paint(&self, origin: Point<Pixels>, window: &mut Window) {
        if self.is_hidden {
            return;
        }
        if !self.indicator_ovals.is_empty() {
            let mut indicator_path = PathBuilder::fill();
            for oval in &self.indicator_ovals {
                let radius = oval.size.width * 0.5;
                let left = point(origin.x + oval.origin.x, origin.y + oval.center().y);
                let right = point(left.x + oval.size.width, left.y);
                indicator_path.move_to(left);
                indicator_path.arc_to(point(radius, radius), Pixels::ZERO, false, true, right);
                indicator_path.arc_to(point(radius, radius), Pixels::ZERO, false, true, left);
                indicator_path.close();
            }
            if let Ok(path) = indicator_path.build() {
                window.paint_path(path, self.indicator_fill_color);
            }
        }

        let checkmark_color = white().opacity(Self::CHECKMARK_OPACITY);
        for polyline in &self.checkmark_polylines {
            let points = polyline.map(|at| point(origin.x + at.x, origin.y + at.y));
            let mut checkmark_path = PathBuilder::stroke(px(Self::CHECKMARK_LINE_WIDTH));
            checkmark_path.add_polygon(&points, false);
            if let Ok(path) = checkmark_path.build() {
                window.paint_path(path, checkmark_color);
            }
            for at in points {
                Self::paint_round_cap(at, window, checkmark_color);
            }
        }
    }

    fn paint_round_cap(at: Point<Pixels>, window: &mut Window, color: Hsla) {
        let half = px(Self::CHECKMARK_LINE_WIDTH * 0.5);
        let bounds = Bounds::new(point(at.x - half, at.y - half), size(half * 2.0, half * 2.0));
        window.paint_quad(fill(bounds, color).corner_radii(half));
    }

    fn accessibility_children(&self, id: &ElementId) -> Option<AnyElement> {
        if self.accessibility_elements_hidden {
            return None;
        }
        let elements = self.accessibility_elements.as_ref()?;
        Some(
            div()
                .id((id.clone(), "selection-overlay"))
                .absolute()
                .size_full()
                .role(Role::List)
                .children(elements.iter().enumerate().map(|(index, element)| {
                    element.element(ElementId::NamedInteger("selection-row".into(), index as u64))
                }))
                .into_any_element(),
        )
    }

    fn touches_began(&mut self, window_location: Point<Pixels>) {
        if !self.is_user_interaction_enabled {
            return;
        }
        self.tap_gesture.touches_began(window_location);
    }

    fn touches_moved(&mut self, window_location: Point<Pixels>) {
        if !self.is_user_interaction_enabled {
            return;
        }
        self.tap_gesture.touches_moved(window_location);
    }

    /// pointer up, location is in the list own space
    /// the press stays with the rows under the overlay too
    fn touches_ended(&mut self, window_location: Point<Pixels>, location: Point<Pixels>, window: &mut Window, cx: &mut App) {
        if !self.is_user_interaction_enabled {
            return;
        }
        if self.tap_gesture.touches_ended(window_location) {
            self.handle_tap(self.tap_gesture.state(), location, window, cx);
        }
    }

    fn handle_tap(
        &self,
        state: RiseStrictTapGestureRecognizerState,
        location: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if state != RiseStrictTapGestureRecognizerState::Recognized {
            return;
        }
        let Some(row) = self
            .rows
            .iter()
            .find(|row| location.y >= row.frame.top() && location.y < row.frame.bottom())
        else {
            tracing::warn!(
                target: "riseonly::X1870",
                y = ?location.y,
                rows = ?self.rows.iter().map(|row| (row.selection_id.clone(), row.frame.top(), row.frame.bottom())).collect::<Vec<_>>(),
                "selection tap hit no row"
            );
            return;
        };
        tracing::warn!(target: "riseonly::X1870", y = ?location.y, selection_id = %row.selection_id, has_toggle = self.on_toggle.is_some(), "selection tap row");
        if let Some(on_toggle) = self.on_toggle.as_ref() {
            on_toggle(row.selection_id.clone(), window, cx);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseManualListPreparedViewport {
    pub layout_generation: u64,
    pub visible_ids: Vec<String>,
    pub content_offset_y: Pixels,
    pub natural_height_mismatches: HashMap<String, Pixels>,
}

pub(crate) struct RiseManualListViewPlaceholderView;

impl Render for RiseManualListViewPlaceholderView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        tracing::warn!(target: "riseonly::X1693", "list placeholder drawn");
        div()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RiseManualListViewMountMotionClass {
    Idle,
    Active,
    Fast,
}

struct RiseManualListViewLayoutItem {
    id: String,
    content: RiseManualListItem,
    frame: Bounds<Pixels>,
}

type RiseManualListViewLayoutItemRef = Rc<RefCell<RiseManualListViewLayoutItem>>;

impl RiseManualListViewLayoutItem {
    fn new(id: String, content: RiseManualListItem, frame: Bounds<Pixels>) -> RiseManualListViewLayoutItemRef {
        Rc::new(RefCell::new(Self { id, content, frame }))
    }
}

/// rows in timeline order with cheap growth at both edges
#[derive(Default)]
struct RiseManualListViewLayoutDeque {
    storage: VecDeque<RiseManualListViewLayoutItemRef>,
}

impl RiseManualListViewLayoutDeque {
    fn new() -> Self {
        Self::default()
    }

    fn from_items(items: Vec<RiseManualListViewLayoutItemRef>) -> Self {
        let mut storage = VecDeque::with_capacity(16.max(items.len() * 2));
        storage.extend(items);
        Self { storage }
    }

    fn count(&self) -> usize {
        self.storage.len()
    }

    fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    fn first(&self) -> Option<RiseManualListViewLayoutItemRef> {
        self.storage.front().cloned()
    }

    fn last(&self) -> Option<RiseManualListViewLayoutItemRef> {
        self.storage.back().cloned()
    }

    fn prepend(&mut self, contents_of: Vec<RiseManualListViewLayoutItemRef>) {
        for item in contents_of.into_iter().rev() {
            self.storage.push_front(item);
        }
    }

    fn append(&mut self, contents_of: Vec<RiseManualListViewLayoutItemRef>) {
        self.storage.extend(contents_of);
    }

    fn remove_first(&mut self) -> Option<RiseManualListViewLayoutItemRef> {
        self.storage.pop_front()
    }

    fn remove_last(&mut self) -> Option<RiseManualListViewLayoutItemRef> {
        self.storage.pop_back()
    }

    fn iter(&self) -> impl DoubleEndedIterator<Item = &RiseManualListViewLayoutItemRef> + ExactSizeIterator {
        self.storage.iter()
    }
}

impl Index<usize> for RiseManualListViewLayoutDeque {
    type Output = RiseManualListViewLayoutItemRef;

    fn index(&self, index: usize) -> &Self::Output {
        &self.storage[index]
    }
}

#[derive(Clone)]
struct RiseManualListViewMountedItem {
    view: AnyView,
    frame: Bounds<Pixels>,
    host: RiseManualListViewUpdateTransitionHost,
    reuse_identifier: String,
    content_version: i64,
    layout_generation: u64,
    is_placeholder: bool,
    defers_during_fast_scroll: bool,
}

#[derive(Clone)]
struct RiseManualListViewMountedBottomStickyHeader {
    view: AnyView,
    frame: Bounds<Pixels>,
    reuse_identifier: String,
    content_version: i64,
}

#[derive(Clone)]
struct RiseManualListViewVisibleBottomStickyHeader {
    descriptor: RiseManualListBottomStickyHeader,
    frame: Bounds<Pixels>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RiseManualListViewTopStickyHeaderRange {
    first_index: usize,
    last_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RiseManualListViewBottomStickyHeaderRange {
    first_index: usize,
    last_index: usize,
}

/// everything sticky placement reads
/// while it is unchanged the frames a keyboard transition gave the animation are still right
#[derive(Clone, Copy, Debug, PartialEq)]
struct RiseManualListViewKeyboardStickySignature {
    layout_generation: u64,
    content_offset_y: Pixels,
    layout_origin_y: Pixels,
    layout_coordinate_offset_y: Pixels,
    size: Size<Pixels>,
    top_inset: Pixels,
    bottom_inset: Pixels,
    sticky_bottom_inset: Option<Pixels>,
}

impl RiseManualListViewKeyboardStickySignature {
    /// which inputs differ, for the re-plan log
    fn changes(&self, to: &RiseManualListViewKeyboardStickySignature) -> String {
        let other = to;
        let mut fields: Vec<&str> = Vec::new();
        if self.layout_generation != other.layout_generation {
            fields.push("generation");
        }
        if self.content_offset_y != other.content_offset_y {
            fields.push("offset");
        }
        if self.layout_origin_y != other.layout_origin_y {
            fields.push("origin");
        }
        if self.layout_coordinate_offset_y != other.layout_coordinate_offset_y {
            fields.push("coordinates");
        }
        if self.size != other.size {
            fields.push("size");
        }
        if self.top_inset != other.top_inset {
            fields.push("top");
        }
        if self.bottom_inset != other.bottom_inset {
            fields.push("bottom");
        }
        if self.sticky_bottom_inset != other.sticky_bottom_inset {
            fields.push("sticky");
        }
        fields.join(",")
    }
}

#[derive(Clone, Debug)]
struct RiseManualListViewKeyboardStickyPlan {
    signature: RiseManualListViewKeyboardStickySignature,
    top_ids: HashSet<String>,
    bottom_ids: HashSet<String>,
}

#[derive(Clone)]
struct RiseManualListViewUpdateTransitionItem {
    view: AnyView,
    frame: Bounds<Pixels>,
    alpha: f32,
    snapshot: Option<AnyView>,
    image: Option<Arc<RenderImage>>,
}

/// the host an outgoing update snapshot is drawn in
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RiseManualListViewUpdateTransitionHost {
    Item,
    TopSticky,
    BottomSticky,
}

/// position and opacity a row shows while an update transition runs
/// keyed by the view so the motion follows it
#[derive(Clone, Copy, Debug, PartialEq)]
struct RiseManualListViewRowMotion {
    from_offset: Point<Pixels>,
    from_alpha: f32,
    to_alpha: f32,
    to_scale: f32,
    started_at: f64,
    duration: f64,
    damping_ratio: f32,
    alpha_fade: Option<RiseManualListViewAlphaFade>,
    clip_height: Option<(Pixels, Pixels)>,
}

/// ease in or ease out fade on the alpha of a row
#[derive(Clone, Copy, Debug, PartialEq)]
struct RiseManualListViewAlphaFade {
    duration: f64,
    curve: RiseManualListCubicBezier,
}

impl RiseManualListViewAlphaFade {
    const EASE_OUT: RiseManualListCubicBezier = RiseManualListCubicBezier::new(0.0, 0.0, 0.58, 1.0);
    const EASE_IN: RiseManualListCubicBezier = RiseManualListCubicBezier::new(0.42, 0.0, 1.0, 1.0);

    fn progress(&self, elapsed: f64) -> f64 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        self.curve.value((elapsed / self.duration).clamp(0.0, 1.0))
    }
}

/// on screen slice of a leaving row, rect is in the row own bounds
#[derive(Clone)]
struct RiseManualListViewRemovedRowImage {
    image: Arc<RenderImage>,
    scale: f32,
    rect: Bounds<Pixels>,
}

/// snapshot showing only a band of its row
/// row drawn at content_size and moved up by offset_y inside the snapshot frame
#[derive(Clone, Copy, Debug, PartialEq)]
struct RiseManualListViewSnapshotCrop {
    content_size: Size<Pixels>,
    offset_y: Pixels,
}

impl RiseManualListViewRowMotion {
    /// spring progress from 0 to 1 at now
    fn progress(&self, now: f64) -> f32 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        let elapsed = (now - self.started_at).max(0.0);
        if elapsed >= self.duration {
            return 1.0;
        }
        let spring = Spring::new(self.duration, f64::from(self.damping_ratio));
        (1.0 - spring.state(1.0, 0.0, elapsed).0) as f32
    }

    fn is_running(&self, now: f64) -> bool {
        let fade = self.alpha_fade.map_or(0.0, |fade| fade.duration);
        now < self.started_at + self.duration.max(fade)
    }

    fn presentation_clip_height(&self, now: f64) -> Option<Pixels> {
        let (from, to) = self.clip_height?;
        Some(from + (to - from) * self.progress(now))
    }

    fn presentation_offset(&self, now: f64) -> Point<Pixels> {
        let remaining = 1.0 - self.progress(now);
        point(self.from_offset.x * remaining, self.from_offset.y * remaining)
    }

    fn presentation_alpha(&self, now: f64) -> f32 {
        let progress = match self.alpha_fade {
            Some(fade) => fade.progress((now - self.started_at).max(0.0)) as f32,
            None => self.progress(now),
        };
        (self.from_alpha + (self.to_alpha - self.from_alpha) * progress).clamp(0.0, 1.0)
    }

    fn presentation_scale(&self, now: f64) -> f32 {
        1.0 + (self.to_scale - 1.0) * self.progress(now)
    }
}

/// outgoing row kept on screen while it fades
#[derive(Clone)]
struct RiseManualListViewUpdateTransitionSnapshot {
    view: AnyView,
    frame: Bounds<Pixels>,
    host: RiseManualListViewUpdateTransitionHost,
    motion: RiseManualListViewRowMotion,
    crop: Option<RiseManualListViewSnapshotCrop>,
}

#[derive(Clone, Default)]
struct RiseManualListViewUpdateTransitionState {
    items_by_id: HashMap<String, RiseManualListViewUpdateTransitionItem>,
    bottom_sticky_headers_by_id: HashMap<String, RiseManualListViewUpdateTransitionItem>,
    removed_images_by_id: HashMap<String, RiseManualListViewRemovedRowImage>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RiseManualListViewDeferredMountCandidate {
    index: usize,
    distance: Pixels,
    is_ahead: bool,
    defers_during_fast_scroll: bool,
}

#[derive(Clone)]
struct RiseManualListViewDeferredRecycleItem {
    id: String,
    view: AnyView,
    reuse_identifier: String,
    content_version: i64,
}

#[derive(Clone, Debug, PartialEq)]
struct RiseManualListViewVisibleAnchor {
    id: String,
    absolute_min_y: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RiseManualListViewVisibleItemRangeCacheKey {
    layout_generation: u64,
    item_count: usize,
    content_offset_y: Pixels,
    layout_origin_y: Pixels,
    layout_coordinate_offset_y: Pixels,
    viewport_height: Pixels,
}

/// transaction callback, runs inside the list update so it gets the window and app
pub type RiseManualListTransactionCallback = Box<dyn FnOnce(&mut Window, &mut App)>;

struct RiseManualListViewPendingTransaction {
    value: RiseManualListTransaction,
    completion: Option<RiseManualListTransactionCallback>,
    discard: Option<RiseManualListTransactionCallback>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RiseManualListViewViewportClipPresentationState {
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
}

/// where a programmatic travel lands
/// resolved again every frame because rows and insets can move under a running travel
#[derive(Clone, Debug, PartialEq)]
enum RiseManualListViewProgrammaticScrollTarget {
    Top,
    Bottom,
    Item { id: String, viewport_offset: Pixels },
    Offset(Pixels),
}

#[derive(Clone, Debug, PartialEq)]
struct RiseManualListViewProgrammaticScroll {
    target: RiseManualListViewProgrammaticScrollTarget,
    started_at: f64,
    duration: f64,
    initial_distance: Pixels,
}

/// what was on screen before a jump committed
/// pixels for a contiguous slide and one row that may survive the commit, it tells how far the
/// viewport really moved
#[derive(Clone)]
struct RiseManualListViewJumpOrigin {
    snapshot: Option<RiseManualListViewJumpSnapshot>,
    reference_id: Option<String>,
    reference_viewport_y: Pixels,
}

/// rows and avatars on screen before a far jump
#[derive(Clone)]
struct RiseManualListViewJumpSnapshot {
    rows: Vec<(AnyView, Bounds<Pixels>)>,
    avatars: Vec<(AnyView, Bounds<Pixels>)>,
    layer: RiseManualListViewHostLayer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RiseManualListViewDeferredMountQueueSignature {
    layout_generation: u64,
    mounted_lower_bound: usize,
    mounted_upper_bound: usize,
    immediate_lower_bound: usize,
    immediate_upper_bound: usize,
    direction_is_down: bool,
    motion_class: RiseManualListViewMountMotionClass,
    has_deferred_reconfigurations: bool,
}

/// running translation of one host layer, eased from where it was shown when it started
#[derive(Clone, Copy, Debug, PartialEq)]
struct RiseManualListViewTranslationAnimation {
    from_y: Pixels,
    started_at: f64,
    duration: f64,
    curve: Option<RiseManualListCubicBezier>,
}

/// running opacity change
#[derive(Clone, Copy, Debug, PartialEq)]
struct RiseManualListViewOpacityAnimation {
    from_alpha: f32,
    started_at: f64,
    duration: f64,
    curve: RiseManualListCubicBezier,
}

/// running frame change of a host, bounds and position animated together
#[derive(Clone, Copy, Debug, PartialEq)]
struct RiseManualListViewFrameAnimation {
    from_frame: Bounds<Pixels>,
    started_at: f64,
    duration: f64,
}

/// frame and translation and opacity of one host view, plus its running translation
#[derive(Clone, Copy, Debug, PartialEq)]
struct RiseManualListViewHostLayer {
    frame: Bounds<Pixels>,
    translation_y: Pixels,
    alpha: f32,
    is_hidden: bool,
    translation_animation: Option<RiseManualListViewTranslationAnimation>,
    alpha_animation: Option<RiseManualListViewOpacityAnimation>,
    frame_animation: Option<RiseManualListViewFrameAnimation>,
}

impl Default for RiseManualListViewHostLayer {
    fn default() -> Self {
        Self {
            frame: Bounds::default(),
            translation_y: Pixels::ZERO,
            alpha: 1.0,
            is_hidden: false,
            translation_animation: None,
            alpha_animation: None,
            frame_animation: None,
        }
    }
}

/// default ease in out curve
fn rise_manual_list_ease_in_out(progress: f64) -> f64 {
    progress * progress * (3.0 - 2.0 * progress)
}

fn whole_point_inset(inset: Pixels) -> Pixels {
    px(f32::from(inset.max(Pixels::ZERO)).round())
}

impl RiseManualListViewHostLayer {
    /// the model value, a running animation keeps showing its own path until it ends
    fn set_translation_y(&mut self, translation_y: Pixels) {
        self.translation_y = translation_y;
    }

    /// animates translation from the value on screen now
    fn animate_translation_y(&mut self, to: Pixels, duration: f64, now: f64) {
        let from_y = self.presentation_translation_y(now);
        self.translation_y = to;
        self.translation_animation = (duration > 0.0 && from_y != to)
            .then_some(RiseManualListViewTranslationAnimation { from_y, started_at: now, duration, curve: None });
    }

    /// translation animation from its start to the model value
    fn add_translation_animation(&mut self, animation: RiseManualListViewTranslationAnimation) {
        self.translation_animation =
            (animation.duration > 0.0 && animation.from_y != self.translation_y).then_some(animation);
    }

    fn animate_alpha(&mut self, to: f32, duration: f64, curve: RiseManualListCubicBezier, now: f64) {
        let from_alpha = self.presentation_alpha(now);
        self.alpha = to;
        self.alpha_animation = (duration > 0.0 && from_alpha != to)
            .then_some(RiseManualListViewOpacityAnimation { from_alpha, started_at: now, duration, curve });
    }

    fn stop_alpha_animation(&mut self, now: f64) {
        self.alpha = self.presentation_alpha(now);
        self.alpha_animation = None;
    }

    fn animate_frame(&mut self, from_frame: Bounds<Pixels>, to: Bounds<Pixels>, duration: f64, now: f64) {
        self.frame = to;
        self.frame_animation = (duration > 0.0 && from_frame != to)
            .then_some(RiseManualListViewFrameAnimation { from_frame, started_at: now, duration });
    }

    fn remove_all_animations(&mut self) {
        self.translation_animation = None;
        self.alpha_animation = None;
        self.frame_animation = None;
    }

    fn has_running_animation(&self, now: f64) -> bool {
        self.translation_animation
            .is_some_and(|animation| now < animation.started_at + animation.duration)
            || self.alpha_animation.is_some_and(|animation| now < animation.started_at + animation.duration)
            || self.frame_animation.is_some_and(|animation| now < animation.started_at + animation.duration)
    }

    fn presentation_alpha(&self, now: f64) -> f32 {
        let Some(animation) = self.alpha_animation else {
            return self.alpha;
        };
        let progress = ((now - animation.started_at) / animation.duration).clamp(0.0, 1.0);
        if progress >= 1.0 {
            return self.alpha;
        }
        let eased = animation.curve.value(progress) as f32;
        animation.from_alpha + (self.alpha - animation.from_alpha) * eased
    }

    fn presentation_frame(&self, now: f64) -> Bounds<Pixels> {
        let Some(animation) = self.frame_animation else {
            return self.frame;
        };
        let progress = ((now - animation.started_at) / animation.duration).clamp(0.0, 1.0);
        if progress >= 1.0 {
            return self.frame;
        }
        let eased = rise_manual_list_ease_in_out(progress) as f32;
        let from = animation.from_frame;
        let to = self.frame;
        Bounds::new(
            point(
                from.origin.x + (to.origin.x - from.origin.x) * eased,
                from.origin.y + (to.origin.y - from.origin.y) * eased,
            ),
            size(
                from.size.width + (to.size.width - from.size.width) * eased,
                from.size.height + (to.size.height - from.size.height) * eased,
            ),
        )
    }

    fn presentation_translation_y(&self, now: f64) -> Pixels {
        let Some(animation) = self.translation_animation else {
            return self.translation_y;
        };
        let progress = ((now - animation.started_at) / animation.duration).clamp(0.0, 1.0);
        if progress >= 1.0 {
            return self.translation_y;
        }
        let eased = match animation.curve {
            Some(curve) => curve.value(progress) as f32,
            None => rise_manual_list_ease_in_out(progress) as f32,
        };
        let travelled = (f32::from(self.translation_y - animation.from_y) * eased).round();
        animation.from_y + px(travelled)
    }
}

/// recognizer the owner installs on the list scroll surface
pub trait RiseManualListScrollGestureRecognizing {
    fn pointer_down(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut App) -> bool;
    fn pointer_moved(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut App) -> bool;
    fn pointer_up(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut App);
}

struct RiseManualListViewScrollGestureRecognizer {
    recognizer: Rc<RefCell<dyn RiseManualListScrollGestureRecognizing>>,
    scroll_pan_requires_failure: bool,
}

pub struct RiseManualListView {
    bounds: Bounds<Pixels>,
    viewport_clip_view: RiseManualListViewHostLayer,
    scroll_view: RiseManualListScrollView,
    scroll_view_clips_to_bounds: bool,
    item_host_view: RiseManualListViewHostLayer,
    selection_overlay_view: RiseManualListSelectionOverlayView,
    selection_overlay_layer: RiseManualListViewHostLayer,
    top_sticky_header_host_view: RiseManualListViewHostLayer,
    bottom_sticky_header_host_view: RiseManualListViewHostLayer,
    bottom_sticky_header_tap_gesture: RiseStrictTapGestureRecognizer,
    top_sticky_header_tap_gesture: RiseStrictTapGestureRecognizer,
    scroll_motion: Rc<Cell<bool>>,
    scroll_gesture_recognizers: Vec<RiseManualListViewScrollGestureRecognizer>,

    layout_items: RiseManualListViewLayoutDeque,
    item_by_id: HashMap<String, RiseManualListViewLayoutItemRef>,
    jump_highlight_view: RiseManualListViewHostLayer,
    jump_highlight_color: Hsla,
    jump_highlight_item_id: Option<String>,
    jump_highlight_animator: Option<Task<()>>,
    mounted_items: HashMap<String, RiseManualListViewMountedItem>,
    mounted_bottom_sticky_headers: HashMap<String, RiseManualListViewMountedBottomStickyHeader>,
    top_sticky_header_ranges: Vec<RiseManualListViewTopStickyHeaderRange>,
    bottom_sticky_header_ranges: HashMap<String, RiseManualListViewBottomStickyHeaderRange>,
    sticky_header_ranges_generation: Option<u64>,
    top_sticky_item_frames: HashMap<String, Bounds<Pixels>>,
    insert_travel_row_ids: HashSet<String>,
    insert_travel_completion_ids: Vec<String>,
    pinned_top_sticky_item_ids: HashSet<String>,
    detached_top_sticky_item_ids: HashSet<String>,
    top_sticky_touch_claim_id: Option<String>,
    top_sticky_headers_flashing: bool,
    top_sticky_flash_generation: u64,
    visible_top_sticky_item_frames_buffer: HashMap<String, Bounds<Pixels>>,
    top_sticky_item_ids_pending_restore: Vec<String>,
    visible_bottom_sticky_headers_buffer: HashMap<String, RiseManualListViewVisibleBottomStickyHeader>,
    bottom_sticky_header_ids_pending_unmount: Vec<String>,
    reusable_views_by_identifier: HashMap<String, Vec<AnyView>>,
    reusable_view_total_count: usize,
    ids_pending_unmount: Vec<String>,
    pending_transaction: Option<RiseManualListViewPendingTransaction>,
    awaits_render_after_transaction: bool,
    is_render_after_transaction_check_scheduled: bool,
    last_scroll_safe_retry_range: Option<Range<usize>>,
    is_scroll_safe_retry_scheduled: bool,
    layout_origin_y: Pixels,
    layout_coordinate_offset_y: Pixels,
    layout_height: Pixels,
    applied_inter_item_spacing: Pixels,
    last_viewport_size: Size<Pixels>,
    layout_generation: u64,
    visible_generation: u64,
    mounted_range: Range<usize>,
    visible_range: Range<usize>,
    readable_range: Range<usize>,
    readable_generation: u64,
    animated_media_visible_range: Range<usize>,
    animated_media_visible_generation: u64,
    cached_visible_item_range: Option<(RiseManualListViewVisibleItemRangeCacheKey, Range<usize>)>,
    immediate_mount_range: Range<usize>,
    deferred_mount_generation: u64,
    deferred_mount_scheduled_generation: Option<u64>,
    deferred_mount_display_link: Option<Task<()>>,
    deferred_mount_needs_work: bool,
    deferred_mount_queue_frame_owed: bool,
    deferred_mount_queue_signature: Option<RiseManualListViewDeferredMountQueueSignature>,
    deferred_mount_queue: Vec<RiseManualListViewDeferredMountCandidate>,
    deferred_mount_queue_index: usize,
    deferred_mount_direction_is_down: Option<bool>,
    deferred_mount_motion_class: Option<RiseManualListViewMountMotionClass>,
    mount_budget_window_started_at: f64,
    mount_budget_consumed: f64,
    mount_budget_last_finished_at: f64,
    mount_budget_frame: Option<RiseManualListMountFrame>,
    deferred_mount_average_cost: f64,
    deferred_mount_average_cost_by_reuse_identifier: HashMap<String, f64>,
    deferred_reconfigure_ids: HashSet<String>,
    deferred_reconfigure_ids_pending_removal: Vec<String>,
    detached_identity_items: Vec<RiseManualListViewDeferredRecycleItem>,
    deferred_recycle_items: Vec<RiseManualListViewDeferredRecycleItem>,
    deferred_recycle_generation: u64,
    deferred_recycle_scheduled_generation: Option<u64>,
    deferred_recycle_idle_since: Option<f64>,
    identity_cache_exclusion_ids: HashSet<String>,
    last_mount_offset_y: Option<Pixels>,
    mount_direction: f32,
    estimated_scroll_velocity_y: Pixels,
    last_scroll_sample_offset_y: Option<Pixels>,
    last_scroll_sample_time: Option<f64>,
    is_applying_transaction: bool,
    is_updating_scroll_geometry: bool,
    is_programmatic_scroll_animating: bool,
    pending_geometry_refresh: bool,
    pending_geometry_keeps_bottom_anchor: bool,
    pending_geometry_keeps_top_anchor: bool,
    pending_item_relayout: bool,
    pending_geometry_allows_application_during_scroll: bool,
    #[cfg(debug_assertions)]
    user_scrolling_override_for_testing: Option<bool>,
    last_notified_viewport_width: Pixels,
    visible_viewport_preparation_generation: u64,
    visible_viewport_preparation_depth: usize,
    selection_presentation: RiseManualListSelectionPresentation,
    active_jump_snapshot: Option<RiseManualListViewJumpSnapshot>,
    active_jump_generation: u64,
    jump_slide_layer: RiseManualListViewHostLayer,
    programmatic_scroll: Option<RiseManualListViewProgrammaticScroll>,
    programmatic_scroll_display_link: Option<Task<()>>,
    viewport_resize_animation: Option<RiseManualListViewportResizeAnimation>,
    viewport_resize_session_generation: u64,
    viewport_resize_terminal_generation: Option<u64>,
    viewport_resize_terminal_cleanup_scheduled_generation: Option<u64>,
    viewport_resize_gesture_ownership_generation: Option<u64>,
    viewport_resize_retained_mounted_ids: HashSet<String>,
    viewport_anchor_animation_generation: u64,
    viewport_anchor_animation_is_active: bool,
    viewport_anchor_animation_presentation_committed: bool,
    viewport_anchor_animation_start_offset_y: Pixels,
    viewport_clip_animation_generation: u64,
    viewport_clip_animation_is_active: bool,
    viewport_clip_animation_presentation_committed: bool,
    viewport_clip_animation_start_state: Option<RiseManualListViewViewportClipPresentationState>,
    keyboard_viewport_animation_generation: u64,
    keyboard_sticky_display_link: Option<Task<()>>,
    keyboard_timing_reference_view: RiseManualListViewHostLayer,
    keyboard_sticky_plan: Option<RiseManualListViewKeyboardStickyPlan>,
    keyboard_viewport_transition_is_active: bool,
    keyboard_viewport_keeps_bottom_anchor: bool,
    keyboard_viewport_target_insets: Option<Edges<Pixels>>,
    keyboard_viewport_target_sticky_bottom_inset: Option<Pixels>,
    keyboard_viewport_bottom_anchor_override: Option<bool>,
    #[cfg(debug_assertions)]
    last_viewport_anchor_animation_offset_for_testing: Option<Pixels>,
    #[cfg(debug_assertions)]
    last_viewport_clip_animation_start_height_for_testing: Option<Pixels>,
    #[cfg(debug_assertions)]
    keyboard_content_translation_for_testing: Pixels,
    #[cfg(debug_assertions)]
    keyboard_sticky_translation_for_testing: Pixels,
    #[cfg(debug_assertions)]
    visible_item_range_computation_count_for_testing: usize,
    #[cfg(debug_assertions)]
    visible_synchronization_pass_count_for_testing: usize,
    programmatic_navigation_owns_scroll: bool,
    update_transition_motions: HashMap<EntityId, RiseManualListViewRowMotion>,
    update_transition_snapshots: Vec<RiseManualListViewUpdateTransitionSnapshot>,
    update_transition_generation: u64,
    update_transition_task: Option<Task<()>>,
    removal_dust_host: RiseManualListRemovalDustHost,
    effective_layout_direction_is_left_to_right: bool,
    selection_interaction_disabled_ids: HashSet<EntityId>,

    inter_item_spacing: Pixels,
    pub rests_short_content_at_top: bool,
    prevents_bottom_overscroll: bool,
    pub leading_scroll_boundary: Option<Rc<dyn Fn(&App) -> Option<Pixels>>>,
    viewport_insets: Edges<Pixels>,
    bottom_sticky_viewport_inset: Option<Pixels>,
    top_sticky_viewport_inset: Option<Pixels>,
    preload_distance: Pixels,
    pub warms_mounted_rows: bool,
    drawn_row_ids: HashSet<EntityId>,
    reuse_pool_limit: usize,

    pub on_keyboard_viewport_transition_did_finish: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    keyboard_viewport_watchdog_task: Option<Task<()>>,
    keyboard_viewport_animation_task: Option<Task<()>>,

    pub on_visible_ids_changed: Option<Rc<dyn Fn(&[String], &mut Window, &mut App)>>,
    pub on_active_scroll_metrics_changed: Option<Rc<dyn Fn(&RiseManualListScrollMetrics, &mut Window, &mut App)>>,
    pub on_scroll_metrics_changed: Option<Rc<dyn Fn(&RiseManualListScrollMetrics, &mut Window, &mut App)>>,
    pub on_viewport_width_changed: Option<Rc<dyn Fn(Pixels, &mut Window, &mut App)>>,
    pub on_readable_ids_changed: Option<Rc<dyn Fn(&[String], &mut Window, &mut App)>>,
    pub on_drag_ended: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,

    visible_ids: Vec<String>,
    readable_ids: Vec<String>,

    is_in_window: bool,
    memory_warning_observer: Option<RiseSubscription>,
    memory_warning_task: Option<Task<()>>,
    window_activation_observer: Option<Subscription>,
    focus_handle: FocusHandle,
    wheel_settle_task: Option<Task<()>>,
    integration_scroll_handle: ScrollHandle,
    window_handle: AnyWindowHandle,
    main_tab_scroll_to_top_mount: MainTabScrollToTopListMount,
    pub on_main_tab_scroll_to_top: Option<Rc<dyn Fn(bool, &mut Window, &mut App) -> bool>>,
    pub on_wheel_past_top: Option<Rc<dyn Fn(Pixels, &mut Window, &mut App)>>,
}

impl RiseManualListView {
    const TOP_STICKY_FLASH_LINGER: f64 = 0.85;
    const JUMP_SLIDE_ANIMATION_KEY: &'static str = "rise.jump.slide";
    const KEYBOARD_STICKY_ANIMATION_KEY: &'static str = "rise.keyboard.sticky";
    const READABLE_ROW_SHARE: f32 = 0.5;

    pub fn inter_item_spacing(&self) -> Pixels {
        self.inter_item_spacing
    }

    pub fn set_inter_item_spacing(&mut self, inter_item_spacing: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        let normalized = inter_item_spacing.max(Pixels::ZERO);
        let old_value = self.inter_item_spacing;
        self.inter_item_spacing = normalized;
        if old_value == normalized {
            return;
        }
        self.pending_geometry_keeps_bottom_anchor =
            self.pending_geometry_keeps_bottom_anchor || self.is_at_bottom(self.bounds.size.height, None);
        self.pending_geometry_refresh = true;
        self.pending_item_relayout = true;
        self.flush_deferred_geometry_if_possible(window, cx);
    }

    pub fn prevents_bottom_overscroll(&self) -> bool {
        self.prevents_bottom_overscroll
    }

    pub fn set_prevents_bottom_overscroll(&mut self, prevents_bottom_overscroll: bool, cx: &mut Context<Self>) {
        let old_value = self.prevents_bottom_overscroll;
        self.prevents_bottom_overscroll = prevents_bottom_overscroll;
        if old_value == prevents_bottom_overscroll || !prevents_bottom_overscroll {
            return;
        }
        self.clamp_bottom_overscroll_if_needed(cx);
        cx.notify();
    }

    pub fn content_top_offset_y(&self) -> Pixels {
        self.layout_origin_y - self.viewport_insets.top
    }

    fn resolved_top_overscroll_limit_y(&self, cx: &App) -> Option<Pixels> {
        if self.is_updating_scroll_geometry
            || self.is_applying_transaction
            || !self.has_usable_viewport()
            || self.layout_items.is_empty()
        {
            return None;
        }
        let leading_scroll_boundary = self.leading_scroll_boundary.as_ref()?;
        let boundary = leading_scroll_boundary(cx)?;
        if !boundary.as_f32().is_finite() {
            return None;
        }
        Some(self.content_top_offset_y() + boundary.max(Pixels::ZERO))
    }

    /// writes an offset through the scroll view with the limits read from the closures at that moment
    fn set_content_offset_within_overscroll_limits(&mut self, content_offset: Point<Pixels>, cx: &App) {
        self.scroll_view.top_overscroll_limit_y = self.resolved_top_overscroll_limit_y(cx);
        self.scroll_view.bottom_overscroll_limit_y = self.resolved_bottom_overscroll_limit_y();
        self.scroll_view.set_content_offset(content_offset);
        self.scroll_view.top_overscroll_limit_y = None;
        self.scroll_view.bottom_overscroll_limit_y = None;
    }

    pub fn viewport_insets(&self) -> Edges<Pixels> {
        self.viewport_insets
    }

    pub fn set_viewport_insets(&mut self, viewport_insets: Edges<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let normalized = Edges {
            top: whole_point_inset(viewport_insets.top),
            right: Pixels::ZERO,
            bottom: whole_point_inset(viewport_insets.bottom),
            left: Pixels::ZERO,
        };
        let old_value = self.viewport_insets;
        self.viewport_insets = normalized;
        if old_value == normalized {
            return;
        }
        RiseListJumpDebug::log(&format!(
            "INSETS top {}->{} bottom {}->{} scrolling={} offsetY={}",
            RiseListJumpDebug::f(old_value.top),
            RiseListJumpDebug::f(normalized.top),
            RiseListJumpDebug::f(old_value.bottom),
            RiseListJumpDebug::f(normalized.bottom),
            self.is_user_scrolling(),
            RiseListJumpDebug::f(self.scroll_view.content_offset.y),
        ), cx);
        if let Some(keyboard_viewport_bottom_anchor_override) = self.keyboard_viewport_bottom_anchor_override {
            self.pending_geometry_keeps_top_anchor = false;
            self.pending_geometry_keeps_bottom_anchor = keyboard_viewport_bottom_anchor_override;
        } else if self.gesture_owns_viewport_geometry() {
            if !self.pending_item_relayout {
                self.pending_geometry_keeps_top_anchor = false;
                self.pending_geometry_keeps_bottom_anchor = false;
            }
        } else {
            let viewport_height = self.bounds.size.height;
            self.pending_geometry_keeps_top_anchor =
                self.pending_geometry_keeps_top_anchor || self.is_at_top(Some(old_value.top));
            self.pending_geometry_keeps_bottom_anchor = self.pending_geometry_keeps_bottom_anchor
                || self.is_at_bottom(viewport_height, Some(old_value.bottom))
                || self.content_fits_viewport(viewport_height, old_value.bottom);
        }
        self.pending_geometry_refresh = true;
        self.pending_geometry_allows_application_during_scroll = true;
        if self.has_active_viewport_resize_session() {
            cx.notify();
        } else {
            self.flush_deferred_geometry_if_possible(window, cx);
            self.update_sticky_headers(window, cx);
        }
    }

    pub fn bottom_sticky_viewport_inset(&self) -> Option<Pixels> {
        self.bottom_sticky_viewport_inset
    }

    pub fn set_bottom_sticky_viewport_inset(
        &mut self,
        bottom_sticky_viewport_inset: Option<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bottom_sticky_viewport_inset = bottom_sticky_viewport_inset.map(whole_point_inset);
        let old_value = self.bottom_sticky_viewport_inset;
        self.bottom_sticky_viewport_inset = bottom_sticky_viewport_inset;
        if old_value == bottom_sticky_viewport_inset {
            return;
        }
        RiseListJumpDebug::log(&format!(
            "STICKY-INSET {}->{}",
            RiseListJumpDebug::f_optional(old_value),
            RiseListJumpDebug::f_optional(bottom_sticky_viewport_inset),
        ), cx);
        if self.has_active_viewport_resize_session() {
            cx.notify();
        } else {
            self.update_sticky_headers(window, cx);
        }
    }

    pub fn top_sticky_viewport_inset(&self) -> Option<Pixels> {
        self.top_sticky_viewport_inset
    }

    pub fn set_top_sticky_viewport_inset(
        &mut self,
        top_sticky_viewport_inset: Option<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let normalized = top_sticky_viewport_inset.map(|inset| inset.max(Pixels::ZERO));
        let old_value = self.top_sticky_viewport_inset;
        self.top_sticky_viewport_inset = normalized;
        if old_value == normalized {
            return;
        }
        self.update_sticky_headers(window, cx);
    }

    pub fn preload_distance(&self) -> Pixels {
        self.preload_distance
    }

    pub fn set_preload_distance(&mut self, preload_distance: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        self.preload_distance = preload_distance.max(Pixels::ZERO);
        self.update_mounted_items(HashSet::new(), window, cx);
    }

    pub fn reuse_pool_limit(&self) -> usize {
        self.reuse_pool_limit
    }

    pub fn set_reuse_pool_limit(&mut self, reuse_pool_limit: usize) {
        self.reuse_pool_limit = reuse_pool_limit;
        self.trim_reuse_pool();
    }

    pub fn is_scroll_enabled(&self) -> bool {
        self.scroll_view.is_scroll_enabled
    }

    pub fn set_is_scroll_enabled(&mut self, is_scroll_enabled: bool, cx: &mut Context<Self>) {
        if self.scroll_view.is_scroll_enabled == is_scroll_enabled {
            return;
        }
        self.scroll_view.is_scroll_enabled = is_scroll_enabled;
        cx.notify();
    }

    pub fn prepare_viewport_resize_animation(
        &mut self,
        animation: RiseManualListViewportResizeAnimation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let duration = if animation.duration.is_finite() { animation.duration.max(0.0) } else { 0.0 };
        let gesture_already_owned_session = self.viewport_resize_gesture_ownership_generation.is_some();
        self.viewport_resize_session_generation = self.viewport_resize_session_generation.wrapping_add(1);
        let generation = self.viewport_resize_session_generation;
        self.viewport_resize_terminal_generation = None;
        self.viewport_resize_terminal_cleanup_scheduled_generation = None;
        self.viewport_resize_retained_mounted_ids.extend(self.mounted_items.keys().cloned());

        if self.is_user_scrolling() || gesture_already_owned_session {
            self.take_ownership_of_viewport_anchor_animation(window, cx);
            self.viewport_resize_gesture_ownership_generation = Some(generation);
        } else {
            self.viewport_resize_gesture_ownership_generation = None;
        }
        if duration <= 0.0 {
            self.viewport_resize_animation = None;
            return;
        }

        self.viewport_resize_animation = Some(RiseManualListViewportResizeAnimation { duration, ..animation });
    }

    pub fn finish_viewport_resize_animation_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.viewport_resize_session_generation == 0 {
            return;
        }
        self.viewport_resize_terminal_generation = Some(self.viewport_resize_session_generation);
        self.schedule_terminal_viewport_resize_cleanup(self.viewport_resize_session_generation, window, cx);
    }

    pub fn has_active_keyboard_viewport_transition(&self) -> bool {
        self.keyboard_viewport_transition_is_active
    }

    /// composer or picker panel changed the reserved bottom space, rows follow it on the same curve
    pub fn animate_keyboard_viewport_transition(
        &mut self,
        to: Edges<Pixels>,
        sticky_bottom_inset: Option<Pixels>,
        duration: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target_insets = to;
        let normalized_insets = Edges {
            top: whole_point_inset(target_insets.top),
            right: Pixels::ZERO,
            bottom: whole_point_inset(target_insets.bottom),
            left: Pixels::ZERO,
        };
        let normalized_sticky_inset = sticky_bottom_inset.map(whole_point_inset);

        if self.keyboard_viewport_transition_is_active
            && self.keyboard_viewport_target_insets == Some(normalized_insets)
            && self.keyboard_viewport_target_sticky_bottom_inset == normalized_sticky_inset
        {
            return;
        }

        let viewport_height = self.bounds.size.height;
        if !self.keyboard_viewport_transition_is_active {
            self.keyboard_timing_reference_view.remove_all_animations();
            self.keyboard_timing_reference_view
                .set_translation_y(self.bottom_sticky_viewport_inset.unwrap_or(self.viewport_insets.bottom));
            let committed_bottom_inset = self.viewport_insets.bottom;
            let top_offset_y = self.layout_origin_y - self.viewport_insets.top;
            let bottom_offset_y = self.layout_origin_y + self.layout_height - viewport_height + committed_bottom_inset;
            let current_offset_y = self.scroll_view.content_offset.y;
            let released_bottom_space = (committed_bottom_inset - normalized_insets.bottom).max(Pixels::ZERO);
            let preserves_valid_bottom_distance = released_bottom_space > px(0.5)
                && top_offset_y <= bottom_offset_y
                && current_offset_y >= top_offset_y - px(2.0)
                && current_offset_y <= bottom_offset_y + px(2.0)
                && bottom_offset_y - current_offset_y < released_bottom_space;
            self.keyboard_viewport_keeps_bottom_anchor = self.is_at_bottom(viewport_height, Some(committed_bottom_inset))
                || preserves_valid_bottom_distance
                || self.is_programmatic_scroll_heading_to_bottom()
                || self.content_fits_viewport(viewport_height, committed_bottom_inset);
        }
        self.keyboard_viewport_transition_is_active = true;
        self.keyboard_viewport_target_insets = Some(normalized_insets);
        self.keyboard_viewport_target_sticky_bottom_inset = normalized_sticky_inset;
        self.keyboard_viewport_animation_generation = self.keyboard_viewport_animation_generation.wrapping_add(1);
        let generation = self.keyboard_viewport_animation_generation;

        self.cancel_active_jump_animation(cx);
        let content_translation_y = RiseManualListKeyboardPresentationPolicy::content_translation_y(
            self.viewport_insets.bottom,
            normalized_insets.bottom,
            self.keyboard_viewport_keeps_bottom_anchor,
        );
        #[cfg(debug_assertions)]
        tracing::debug!(
            "[RISE-KB][list] animate inset {:?}->{:?} offset={:?} keepsBottom={} translation={:?}",
            self.viewport_insets.bottom,
            normalized_insets.bottom,
            self.scroll_view.content_offset.y,
            self.keyboard_viewport_keeps_bottom_anchor,
            content_translation_y,
        );
        let sticky_translation_y = RiseManualListKeyboardPresentationPolicy::sticky_translation_y(
            self.bottom_sticky_viewport_inset,
            normalized_sticky_inset,
        );
        let target_sticky_bottom_inset = normalized_sticky_inset.unwrap_or(normalized_insets.bottom);

        let normalized_duration = if duration.is_finite() { duration.max(0.0) } else { 0.0 };
        if normalized_duration <= 1.0 / 240.0 {
            self.apply_keyboard_target_transforms(
                content_translation_y,
                sticky_translation_y,
                target_sticky_bottom_inset,
                None,
            );
            if self.gesture_owns_viewport_geometry() {
                if !self.plan_keyboard_sticky_motion("zero-duration") {
                    self.start_keyboard_sticky_tracking(window, cx);
                    self.update_top_sticky_headers(window, cx);
                    self.update_bottom_sticky_headers(window, cx);
                }
                self.schedule_keyboard_viewport_transition_watchdog(generation, window, cx);
                return;
            }
            self.finish_keyboard_viewport_transition(normalized_insets, normalized_sticky_inset, window, cx);
            return;
        }

        let now = current_media_time();
        self.apply_keyboard_target_transforms(
            content_translation_y,
            sticky_translation_y,
            target_sticky_bottom_inset,
            Some((normalized_duration, now)),
        );
        cx.notify();

        let completion_delay = Duration::from_secs_f64(normalized_duration);
        self.keyboard_viewport_animation_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(completion_delay).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.keyboard_viewport_animation_generation != generation
                    || !this.keyboard_viewport_transition_is_active
                    || this.keyboard_viewport_target_insets != Some(normalized_insets)
                    || this.keyboard_viewport_target_sticky_bottom_inset != normalized_sticky_inset
                {
                    return;
                }
                this.finish_keyboard_viewport_transition(normalized_insets, normalized_sticky_inset, window, cx);
            });
        }));

        if !self.plan_keyboard_sticky_motion("keyboard") {
            self.start_keyboard_sticky_tracking(window, cx);
            self.update_top_sticky_headers(window, cx);
            self.update_bottom_sticky_headers(window, cx);
        }
    }

    /// applies the target transforms, animation is (duration start) or none for at once
    fn apply_keyboard_target_transforms(
        &mut self,
        content_translation_y: Pixels,
        sticky_translation_y: Pixels,
        target_sticky_bottom_inset: Pixels,
        animation: Option<(f64, f64)>,
    ) {
        match animation {
            Some((duration, now)) => {
                self.item_host_view.animate_translation_y(content_translation_y, duration, now);
                self.selection_overlay_layer.animate_translation_y(content_translation_y, duration, now);
                self.keyboard_timing_reference_view.animate_translation_y(target_sticky_bottom_inset, duration, now);
            }
            None => {
                self.item_host_view.set_translation_y(content_translation_y);
                self.selection_overlay_layer.set_translation_y(content_translation_y);
                self.keyboard_timing_reference_view.set_translation_y(target_sticky_bottom_inset);
            }
        }
        #[cfg(debug_assertions)]
        {
            self.keyboard_content_translation_for_testing = content_translation_y;
            self.keyboard_sticky_translation_for_testing = sticky_translation_y;
        }
        #[cfg(not(debug_assertions))]
        let _ = sticky_translation_y;
    }

    pub fn keyboard_viewport_transition_target(&self) -> Option<(Edges<Pixels>, Option<Pixels>)> {
        if !self.keyboard_viewport_transition_is_active {
            return None;
        }
        let insets = self.keyboard_viewport_target_insets?;
        Some((insets, self.keyboard_viewport_target_sticky_bottom_inset))
    }

    fn schedule_keyboard_viewport_transition_watchdog(&mut self, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.keyboard_viewport_watchdog_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.keyboard_viewport_transition_is_active
                    || this.keyboard_viewport_animation_generation != generation
                {
                    return;
                }
                let Some(target_insets) = this.keyboard_viewport_target_insets else {
                    return;
                };
                let sticky_bottom_inset = this.keyboard_viewport_target_sticky_bottom_inset;
                this.finish_keyboard_viewport_transition(target_insets, sticky_bottom_inset, window, cx);
            });
        }));
    }

    pub fn finish_keyboard_viewport_transition(
        &mut self,
        at: Edges<Pixels>,
        sticky_bottom_inset: Option<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.keyboard_viewport_watchdog_task = None;
        self.keyboard_viewport_animation_task = None;
        let target_insets = at;
        let normalized_insets = Edges {
            top: whole_point_inset(target_insets.top),
            right: Pixels::ZERO,
            bottom: whole_point_inset(target_insets.bottom),
            left: Pixels::ZERO,
        };
        let normalized_sticky_inset = sticky_bottom_inset.map(whole_point_inset);
        let viewport_height = self.bounds.size.height;
        let committed_bottom_inset = self.viewport_insets.bottom;
        let transition_was_active = self.keyboard_viewport_transition_is_active;
        let keeps_bottom_anchor = if transition_was_active {
            self.keyboard_viewport_keeps_bottom_anchor
        } else {
            self.is_at_bottom(viewport_height, Some(committed_bottom_inset))
                || self.content_fits_viewport(viewport_height, committed_bottom_inset)
        };
        let content_translation_y = RiseManualListKeyboardPresentationPolicy::content_translation_y(
            committed_bottom_inset,
            normalized_insets.bottom,
            keeps_bottom_anchor,
        );
        let still_at_committed_bottom = self.is_at_bottom(viewport_height, Some(committed_bottom_inset))
            || self.content_fits_viewport(viewport_height, committed_bottom_inset);
        let pins_bottom = keeps_bottom_anchor && still_at_committed_bottom;

        #[cfg(debug_assertions)]
        tracing::debug!(
            "[RISE-KB][list] finish inset {:?}->{:?} offset={:?} keepsBottom={} pinsBottom={} translation={:?}",
            committed_bottom_inset,
            normalized_insets.bottom,
            self.scroll_view.content_offset.y,
            keeps_bottom_anchor,
            pins_bottom,
            content_translation_y,
        );

        self.keyboard_viewport_animation_generation = self.keyboard_viewport_animation_generation.wrapping_add(1);
        self.keyboard_viewport_transition_is_active = false;
        self.stop_keyboard_sticky_tracking();
        self.keyboard_viewport_target_insets = None;
        self.keyboard_viewport_target_sticky_bottom_inset = None;

        RiseListRemovalDiag::log(|| "keyboard reset A kills host animations".to_string(), true, cx);
        self.item_host_view.remove_all_animations();
        self.selection_overlay_layer.remove_all_animations();
        Self::remove_animations_keeping_jump_slide(&mut self.bottom_sticky_header_host_view);
        self.discard_keyboard_sticky_motion();

        if transition_was_active && keeps_bottom_anchor && !pins_bottom {
            self.layout_origin_y += content_translation_y;
            self.update_host_frame();
        }

        self.item_host_view.set_translation_y(Pixels::ZERO);
        self.selection_overlay_layer.set_translation_y(Pixels::ZERO);
        self.bottom_sticky_header_host_view.set_translation_y(Pixels::ZERO);
        #[cfg(debug_assertions)]
        {
            self.keyboard_content_translation_for_testing = Pixels::ZERO;
            self.keyboard_sticky_translation_for_testing = Pixels::ZERO;
        }

        self.keyboard_viewport_bottom_anchor_override = Some(pins_bottom);
        self.set_viewport_insets(normalized_insets, window, cx);
        self.keyboard_viewport_bottom_anchor_override = None;
        self.set_bottom_sticky_viewport_inset(normalized_sticky_inset, window, cx);
        self.keyboard_viewport_keeps_bottom_anchor = false;
        if transition_was_active && !self.has_active_viewport_resize_session() {
            self.update_sticky_headers(window, cx);
        }
        cx.notify();
        if let Some(on_finish) = self.on_keyboard_viewport_transition_did_finish.clone() {
            on_finish(window, cx);
        }
    }

    pub fn cancel_keyboard_viewport_transition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keyboard_viewport_watchdog_task = None;
        self.keyboard_viewport_animation_task = None;
        self.keyboard_viewport_animation_generation = self.keyboard_viewport_animation_generation.wrapping_add(1);
        self.keyboard_viewport_transition_is_active = false;
        self.stop_keyboard_sticky_tracking();
        self.keyboard_viewport_keeps_bottom_anchor = false;
        self.keyboard_viewport_target_insets = None;
        self.keyboard_viewport_target_sticky_bottom_inset = None;
        self.keyboard_viewport_bottom_anchor_override = None;
        RiseListRemovalDiag::log(|| "keyboard reset B kills host animations".to_string(), true, cx);
        self.item_host_view.remove_all_animations();
        self.selection_overlay_layer.remove_all_animations();
        Self::remove_animations_keeping_jump_slide(&mut self.bottom_sticky_header_host_view);
        self.discard_keyboard_sticky_motion();
        self.item_host_view.set_translation_y(Pixels::ZERO);
        self.selection_overlay_layer.set_translation_y(Pixels::ZERO);
        self.bottom_sticky_header_host_view.set_translation_y(Pixels::ZERO);
        #[cfg(debug_assertions)]
        {
            self.keyboard_content_translation_for_testing = Pixels::ZERO;
            self.keyboard_sticky_translation_for_testing = Pixels::ZERO;
        }
        if !self.has_active_viewport_resize_session() {
            self.update_sticky_headers(window, cx);
        }
        cx.notify();
    }

    pub fn visible_ids(&self) -> &[String] {
        &self.visible_ids
    }

    pub fn readable_ids(&self) -> &[String] {
        &self.readable_ids
    }

    pub fn scroll_metrics(&self) -> RiseManualListScrollMetrics {
        let minimum_offset_y = -self.scroll_view.content_inset.top;
        let maximum_offset_y = minimum_offset_y.max(
            self.scroll_view.content_size.height - self.scroll_view.frame.size.height
                + self.scroll_view.content_inset.bottom,
        );
        let offset_y = self.scroll_view.content_offset.y;
        let reports_user_gesture = !self.programmatic_navigation_owns_scroll;
        RiseManualListScrollMetrics {
            content_offset_y: offset_y,
            minimum_content_offset_y: minimum_offset_y,
            maximum_content_offset_y: maximum_offset_y,
            viewport_size: self.scroll_view.frame.size,
            content_height: self.layout_height,
            distance_from_top: (offset_y - minimum_offset_y).max(Pixels::ZERO),
            distance_from_bottom: (maximum_offset_y - offset_y).max(Pixels::ZERO),
            velocity_y: self.effective_estimated_scroll_velocity_y(),
            is_tracking: reports_user_gesture && self.scroll_view.is_tracking,
            is_dragging: reports_user_gesture && self.scroll_view.is_dragging,
            is_decelerating: reports_user_gesture && self.scroll_view.is_decelerating,
        }
    }

    /// list scroll surface for shared element capture and hit testing
    /// never write its offset or inset or content size, the layout origin owns them
    pub(crate) fn integration_scroll_handle(&self) -> ScrollHandle {
        self.integration_scroll_handle.clone()
    }

    pub(crate) fn window_handle(&self) -> AnyWindowHandle {
        self.window_handle
    }

    pub fn integration_scroll_view(&self) -> &RiseManualListScrollView {
        &self.scroll_view
    }

    /// lets drags that start on control rows scroll the list
    pub fn cancels_control_touches(&self) -> bool {
        self.scroll_view.cancels_control_touches
    }

    pub fn set_cancels_control_touches(&mut self, cancels_control_touches: bool) {
        self.scroll_view.cancels_control_touches = cancels_control_touches;
    }

    /// a drag or momentum or animated programmatic scroll owns the offset
    /// transactions that arent scroll safe wait for it to end
    pub fn is_scroll_in_progress(&self) -> bool {
        self.is_user_scrolling()
    }

    pub fn scroll_motion_flags(&self) -> (bool, bool, bool, bool) {
        (
            self.scroll_view.is_tracking,
            self.scroll_view.is_dragging,
            self.scroll_view.is_decelerating,
            self.is_programmatic_scroll_animating,
        )
    }

    /// a transaction is waiting for idle or a usable viewport or the next turn
    pub fn has_pending_transaction(&self) -> bool {
        self.pending_transaction.is_some()
    }

    pub fn item_count(&self) -> usize {
        self.layout_items.count()
    }

    pub fn add_scroll_gesture_recognizer(
        &mut self,
        recognizer: Rc<RefCell<dyn RiseManualListScrollGestureRecognizing>>,
        scroll_pan_requires_failure: bool,
    ) {
        self.scroll_gesture_recognizers
            .retain(|installed| !Rc::ptr_eq(&installed.recognizer, &recognizer));
        self.scroll_gesture_recognizers.push(RiseManualListViewScrollGestureRecognizer {
            recognizer,
            scroll_pan_requires_failure,
        });
    }

    pub fn remove_scroll_gesture_recognizer(&mut self, recognizer: &Rc<RefCell<dyn RiseManualListScrollGestureRecognizing>>) {
        self.scroll_gesture_recognizers
            .retain(|installed| !Rc::ptr_eq(&installed.recognizer, recognizer));
    }

    /// configured view of row id, none while unmounted or a placeholder
    pub fn mounted_view(&self, id: &str) -> Option<AnyView> {
        let mounted = self.mounted_items.get(id)?;
        if mounted.is_placeholder {
            return None;
        }
        Some(mounted.view.clone())
    }

    pub fn mounted_ids(&self) -> Vec<String> {
        self.mounted_items.keys().cloned().collect()
    }

    /// layout frame of row id in this view coordinates, mounted or not none for ids outside the
    /// applied layout
    /// sticky clamping and keyboard translation are not applied
    pub fn viewport_frame(&self, id: &str) -> Option<Bounds<Pixels>> {
        let item = self.item_by_id.get(id)?;
        let mut content_frame = self.resolved_frame(&item.borrow());
        content_frame.origin.y += self.layout_origin_y;
        content_frame.origin.y -= self.scroll_view.content_offset.y;
        content_frame.origin.y += self.scroll_view.frame.origin.y;
        content_frame.origin.x += self.scroll_view.frame.origin.x;
        Some(content_frame)
    }

    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let scroll_motion = Rc::new(Cell::new(false));
        let governing_probe: RiseScrollMotionProbe = {
            let scroll_motion = scroll_motion.clone();
            Rc::new(move || scroll_motion.get())
        };
        let mut bottom_sticky_header_tap_gesture = RiseStrictTapGestureRecognizer::new();
        bottom_sticky_header_tap_gesture.governing_scroll_view = Some(governing_probe.clone());
        let mut top_sticky_header_tap_gesture = RiseStrictTapGestureRecognizer::new();
        top_sticky_header_tap_gesture.governing_scroll_view = Some(governing_probe);

        let (memory_warning_sender, mut memory_warning_receiver) = mpsc::unbounded::<RiseNotificationName>();
        let memory_warning_observer = RiseAppEvents::default_center().observe(
            UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION,
            move |notification| {
                let _ = memory_warning_sender.unbounded_send(notification.name);
            },
        );
        let memory_warning_task = cx.spawn(async move |this, cx| {
            while let Some(name) = memory_warning_receiver.next().await {
                if this.update(cx, |this, _| this.handle_memory_warning(name)).is_err() {
                    break;
                }
            }
        });
        let main_tab_scroll_to_top_mount = {
            let list = cx.weak_entity();
            let wheel_list = list.clone();
            MainTabScrollToTopListMount::with_wheel(
                move |refreshes_feed, window, cx| {
                    let Some(entity) = list.upgrade() else {
                        return;
                    };
                    let feed_jump = entity.read(cx).on_main_tab_scroll_to_top.clone();
                    if feed_jump.is_some_and(|feed_jump| feed_jump(refreshes_feed, window, cx)) {
                        return;
                    }
                    entity.update(cx, |this, cx| {
                        this.smart_scroll_to_top(true, window, cx);
                    });
                },
                move |event, window, cx| {
                    let _ = wheel_list.update(cx, |this, cx| this.handle_scroll_wheel(event, window, cx));
                },
            )
        };
        let window_activation_observer = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.resume_deferred_mount_display_link(window, cx);
            } else {
                this.application_will_resign_active(window, cx);
            }
        });

        Self {
            bounds: Bounds::default(),
            viewport_clip_view: RiseManualListViewHostLayer::default(),
            scroll_view: RiseManualListScrollView::default(),
            scroll_view_clips_to_bounds: false,
            item_host_view: RiseManualListViewHostLayer::default(),
            selection_overlay_view: RiseManualListSelectionOverlayView::new(),
            selection_overlay_layer: RiseManualListViewHostLayer::default(),
            top_sticky_header_host_view: RiseManualListViewHostLayer::default(),
            bottom_sticky_header_host_view: RiseManualListViewHostLayer::default(),
            bottom_sticky_header_tap_gesture,
            top_sticky_header_tap_gesture,
            scroll_motion,
            scroll_gesture_recognizers: Vec::new(),
            layout_items: RiseManualListViewLayoutDeque::new(),
            item_by_id: HashMap::new(),
            jump_highlight_view: RiseManualListViewHostLayer { alpha: 0.0, ..RiseManualListViewHostLayer::default() },
            jump_highlight_color: gpui::transparent_black(),
            jump_highlight_item_id: None,
            jump_highlight_animator: None,
            mounted_items: HashMap::new(),
            mounted_bottom_sticky_headers: HashMap::new(),
            top_sticky_header_ranges: Vec::new(),
            bottom_sticky_header_ranges: HashMap::new(),
            sticky_header_ranges_generation: None,
            top_sticky_item_frames: HashMap::new(),
            pinned_top_sticky_item_ids: HashSet::new(),
            detached_top_sticky_item_ids: HashSet::new(),
            top_sticky_touch_claim_id: None,
            top_sticky_headers_flashing: false,
            top_sticky_flash_generation: 0,
            visible_top_sticky_item_frames_buffer: HashMap::new(),
            top_sticky_item_ids_pending_restore: Vec::new(),
            visible_bottom_sticky_headers_buffer: HashMap::new(),
            bottom_sticky_header_ids_pending_unmount: Vec::new(),
            reusable_views_by_identifier: HashMap::new(),
            reusable_view_total_count: 0,
            ids_pending_unmount: Vec::new(),
            pending_transaction: None,
            awaits_render_after_transaction: false,
            is_render_after_transaction_check_scheduled: false,
            last_scroll_safe_retry_range: None,
            is_scroll_safe_retry_scheduled: false,
            layout_origin_y: Pixels::ZERO,
            layout_coordinate_offset_y: Pixels::ZERO,
            layout_height: Pixels::ZERO,
            applied_inter_item_spacing: Pixels::ZERO,
            last_viewport_size: Size::default(),
            layout_generation: 0,
            visible_generation: 0,
            mounted_range: 0..0,
            visible_range: 0..0,
            animated_media_visible_range: 0..0,
            animated_media_visible_generation: 0,
            cached_visible_item_range: None,
            immediate_mount_range: 0..0,
            deferred_mount_generation: 0,
            deferred_mount_scheduled_generation: None,
            deferred_mount_display_link: None,
            deferred_mount_needs_work: false,
            deferred_mount_queue_frame_owed: false,
            deferred_mount_queue_signature: None,
            deferred_mount_queue: Vec::new(),
            deferred_mount_queue_index: 0,
            deferred_mount_direction_is_down: None,
            deferred_mount_motion_class: None,
            mount_budget_window_started_at: 0.0,
            mount_budget_consumed: 0.0,
            mount_budget_last_finished_at: 0.0,
            mount_budget_frame: None,
            deferred_mount_average_cost: 0.000_75,
            deferred_mount_average_cost_by_reuse_identifier: HashMap::new(),
            deferred_reconfigure_ids: HashSet::new(),
            deferred_reconfigure_ids_pending_removal: Vec::new(),
            detached_identity_items: Vec::new(),
            deferred_recycle_items: Vec::new(),
            deferred_recycle_generation: 0,
            deferred_recycle_scheduled_generation: None,
            deferred_recycle_idle_since: None,
            identity_cache_exclusion_ids: HashSet::new(),
            last_mount_offset_y: None,
            mount_direction: 0.0,
            estimated_scroll_velocity_y: Pixels::ZERO,
            last_scroll_sample_offset_y: None,
            last_scroll_sample_time: None,
            is_applying_transaction: false,
            is_updating_scroll_geometry: false,
            is_programmatic_scroll_animating: false,
            pending_geometry_refresh: false,
            pending_geometry_keeps_bottom_anchor: false,
            pending_geometry_keeps_top_anchor: false,
            pending_item_relayout: false,
            pending_geometry_allows_application_during_scroll: false,
            #[cfg(debug_assertions)]
            user_scrolling_override_for_testing: None,
            last_notified_viewport_width: Pixels::ZERO,
            visible_viewport_preparation_generation: 0,
            visible_viewport_preparation_depth: 0,
            selection_presentation: RiseManualListSelectionPresentation::inactive(),
            active_jump_snapshot: None,
            active_jump_generation: 0,
            jump_slide_layer: RiseManualListViewHostLayer::default(),
            programmatic_scroll: None,
            programmatic_scroll_display_link: None,
            viewport_resize_animation: None,
            viewport_resize_session_generation: 0,
            viewport_resize_terminal_generation: None,
            viewport_resize_terminal_cleanup_scheduled_generation: None,
            viewport_resize_gesture_ownership_generation: None,
            viewport_resize_retained_mounted_ids: HashSet::new(),
            viewport_anchor_animation_generation: 0,
            viewport_anchor_animation_is_active: false,
            viewport_anchor_animation_presentation_committed: false,
            viewport_anchor_animation_start_offset_y: Pixels::ZERO,
            viewport_clip_animation_generation: 0,
            viewport_clip_animation_is_active: false,
            viewport_clip_animation_presentation_committed: false,
            viewport_clip_animation_start_state: None,
            keyboard_viewport_animation_generation: 0,
            keyboard_sticky_display_link: None,
            keyboard_timing_reference_view: RiseManualListViewHostLayer {
                frame: Bounds::new(point(Pixels::ZERO, Pixels::ZERO), size(px(1.0), px(1.0))),
                ..RiseManualListViewHostLayer::default()
            },
            keyboard_sticky_plan: None,
            keyboard_viewport_transition_is_active: false,
            keyboard_viewport_keeps_bottom_anchor: false,
            keyboard_viewport_target_insets: None,
            keyboard_viewport_target_sticky_bottom_inset: None,
            keyboard_viewport_bottom_anchor_override: None,
            #[cfg(debug_assertions)]
            last_viewport_anchor_animation_offset_for_testing: None,
            #[cfg(debug_assertions)]
            last_viewport_clip_animation_start_height_for_testing: None,
            #[cfg(debug_assertions)]
            keyboard_content_translation_for_testing: Pixels::ZERO,
            #[cfg(debug_assertions)]
            keyboard_sticky_translation_for_testing: Pixels::ZERO,
            #[cfg(debug_assertions)]
            visible_item_range_computation_count_for_testing: 0,
            #[cfg(debug_assertions)]
            visible_synchronization_pass_count_for_testing: 0,
            programmatic_navigation_owns_scroll: false,
            update_transition_motions: HashMap::new(),
            update_transition_snapshots: Vec::new(),
            update_transition_generation: 0,
            update_transition_task: None,
            removal_dust_host: RiseManualListRemovalDustHost::default(),
            effective_layout_direction_is_left_to_right: true,
            insert_travel_row_ids: HashSet::new(),
            insert_travel_completion_ids: Vec::new(),
            readable_range: 0..0,
            readable_generation: 0,
            on_readable_ids_changed: None,
            on_drag_ended: None,
            readable_ids: Vec::new(),
            selection_interaction_disabled_ids: HashSet::new(),
            inter_item_spacing: Pixels::ZERO,
            rests_short_content_at_top: false,
            warms_mounted_rows: false,
            drawn_row_ids: HashSet::new(),
            prevents_bottom_overscroll: false,
            leading_scroll_boundary: None,
            viewport_insets: Edges::default(),
            bottom_sticky_viewport_inset: None,
            top_sticky_viewport_inset: None,
            preload_distance: px(800.0),
            reuse_pool_limit: 32,
            on_keyboard_viewport_transition_did_finish: None,
            keyboard_viewport_watchdog_task: None,
            keyboard_viewport_animation_task: None,
            on_visible_ids_changed: None,
            on_active_scroll_metrics_changed: None,
            on_scroll_metrics_changed: None,
            on_viewport_width_changed: None,
            visible_ids: Vec::new(),
            is_in_window: true,
            memory_warning_observer: Some(memory_warning_observer),
            memory_warning_task: Some(memory_warning_task),
            window_activation_observer: Some(window_activation_observer),
            focus_handle: cx.focus_handle(),
            wheel_settle_task: None,
            integration_scroll_handle: ScrollHandle::new(),
            window_handle: window.window_handle(),
            main_tab_scroll_to_top_mount,
            on_main_tab_scroll_to_top: None,
            on_wheel_past_top: None,
        }
    }

    /// true if the list is in a window, from the last did_move_to_window report
    pub fn is_in_window(&self) -> bool {
        self.is_in_window
    }

    pub fn did_move_to_window(&mut self, is_in_window: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.is_in_window = is_in_window;
        if !is_in_window {
            self.main_tab_scroll_to_top_mount.did_leave_window();
            self.pause_deferred_mount_display_link();
            self.complete_programmatic_scroll(window, cx);
        } else {
            self.schedule_deferred_mounting_if_needed(window, cx);
        }
    }

    fn handle_memory_warning(&mut self, name: RiseNotificationName) {
        let identity_count = self.detached_identity_items.len();
        let generic_count = self.deferred_recycle_items.len();
        let reusable_count = self.reusable_view_total_count;
        if identity_count + generic_count + reusable_count == 0 {
            return;
        }

        self.deferred_recycle_generation = self.deferred_recycle_generation.wrapping_add(1);
        self.deferred_recycle_scheduled_generation = None;
        self.deferred_recycle_idle_since = None;
        self.detached_identity_items = Vec::new();
        self.deferred_recycle_items = Vec::new();
        self.reusable_views_by_identifier = HashMap::new();
        self.reusable_view_total_count = 0;
        ChatPerformanceTrace::shared().record(
            "list.detached_cache_purge",
            HashMap::from([
                ("identity".to_string(), identity_count.to_string()),
                ("generic".to_string(), generic_count.to_string()),
                ("reusable".to_string(), reusable_count.to_string()),
                ("reason".to_string(), name.raw_value().to_string()),
            ]),
        );
    }

    pub(crate) fn layout_subviews(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.bounds = bounds;
        let local_bounds = Bounds::new(Point::default(), bounds.size);
        let previous_size = self.last_viewport_size;
        let width_changed = (previous_size.width - bounds.size.width).abs() > px(0.5);
        let height_changed = (previous_size.height - bounds.size.height).abs() > px(0.5);
        let previous_clip_state = height_changed.then(|| self.current_viewport_clip_presentation_state());
        let was_at_bottom = height_changed && self.is_at_bottom(previous_size.height, None);

        if width_changed || height_changed {
            self.cancel_visible_viewport_preparation();
            if RiseListRemovalDiag::is_active(cx) {
                let offset_y = self.scroll_view.content_offset.y;
                RiseListRemovalDiag::log(
                    || {
                        format!(
                            "resize {}->{} offset={}",
                            RiseListRemovalDiag::f(Some(previous_size.height)),
                            RiseListRemovalDiag::f(Some(bounds.size.height)),
                            RiseListRemovalDiag::f(Some(offset_y))
                        )
                    },
                    false,
                    cx,
                );
            }
        }

        let target_sticky_bounds = Bounds::new(Point::default(), self.viewport_clip_view.frame.size);
        let preserves_content_offset_across_resize = height_changed && !self.gesture_owns_viewport_geometry();
        let content_offset_before_resize = self.scroll_view.content_offset;
        if self.viewport_clip_view.frame != local_bounds
            || self.scroll_view.frame != Bounds::new(Point::default(), self.viewport_clip_view.frame.size)
            || self.top_sticky_header_host_view.frame != target_sticky_bounds
            || self.bottom_sticky_header_host_view.frame != target_sticky_bounds
        {
            let was_updating_scroll_geometry = self.is_updating_scroll_geometry;
            if preserves_content_offset_across_resize {
                self.is_updating_scroll_geometry = true;
            }
            self.viewport_clip_view.frame = local_bounds;
            self.scroll_view.frame = Bounds::new(Point::default(), self.viewport_clip_view.frame.size);
            if preserves_content_offset_across_resize {
                self.scroll_view.set_content_offset(content_offset_before_resize);
            }
            self.layout_sticky_header_host_views();
            self.is_updating_scroll_geometry = was_updating_scroll_geometry;
        }
        self.last_viewport_size = bounds.size;

        if let Some(previous_clip_state) = previous_clip_state {
            self.animate_viewport_clip_transition(previous_clip_state, window, cx);
        }

        if !self.has_usable_viewport() {
            return;
        }

        if width_changed {
            self.cancel_deferred_mounting();
        }

        if width_changed && (self.last_notified_viewport_width - bounds.size.width).abs() > px(0.5) {
            self.last_notified_viewport_width = bounds.size.width;
            if let Some(on_viewport_width_changed) = self.on_viewport_width_changed.clone() {
                on_viewport_width_changed(bounds.size.width, window, cx);
            }
        }

        if self.pending_transaction.is_some() {
            self.drain_transactions(window, cx);
            return;
        }

        if width_changed && !self.layout_items.is_empty() {
            self.update_sticky_headers(window, cx);
            return;
        }

        if height_changed {
            if self.gesture_owns_viewport_geometry() {
                if !self.pending_item_relayout {
                    self.pending_geometry_keeps_top_anchor = false;
                    self.pending_geometry_keeps_bottom_anchor = false;
                }
                self.pending_geometry_allows_application_during_scroll = true;
            } else {
                self.pending_geometry_keeps_bottom_anchor = self.pending_geometry_keeps_bottom_anchor || was_at_bottom;
            }
            self.pending_geometry_refresh = true;
        }

        if self.pending_geometry_refresh {
            self.flush_deferred_geometry_if_possible(window, cx);
        } else {
            self.update_host_frame();
            self.update_scroll_geometry();
            self.update_mounted_items(HashSet::new(), window, cx);
            self.notify_scroll_metrics_changed(window, cx);
        }
    }
}

/// callback of prepare visible viewport, runs inside the list update
pub type RiseManualListPreparedViewportCompletion = Box<dyn FnOnce(RiseManualListPreparedViewport, &mut Window, &mut App)>;

impl RiseManualListView {
    pub fn enqueue(
        &mut self,
        transaction: RiseManualListTransaction,
        completion: Option<RiseManualListTransactionCallback>,
        discard: Option<RiseManualListTransactionCallback>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_visible_viewport_preparation();
        self.last_scroll_safe_retry_range = None;
        let allows_application_during_scroll = transaction.allows_application_during_scroll;
        let superseded = self.pending_transaction.replace(RiseManualListViewPendingTransaction {
            value: transaction,
            completion,
            discard,
        });
        if let Some(discard) = superseded.and_then(|superseded| superseded.discard) {
            discard(window, cx);
        }
        if allows_application_during_scroll && self.is_user_scrolling() {
            cx.defer_in(window, |this, window, cx| {
                this.drain_transactions(window, cx);
            });
        } else {
            self.drain_transactions(window, cx);
        }
    }

    pub fn prepare_visible_viewport(
        &mut self,
        completion: RiseManualListPreparedViewportCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.visible_viewport_preparation_generation = self.visible_viewport_preparation_generation.wrapping_add(1);
        let preparation_generation = self.visible_viewport_preparation_generation;
        let expected_layout_generation = self.layout_generation;
        let expected_size = self.bounds.size;
        self.prepare_visible_viewport_pass(
            preparation_generation,
            expected_layout_generation,
            expected_size,
            0,
            completion,
            window,
            cx,
        );
    }

    pub fn prepared_visible_viewport_snapshot_if_stable(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<RiseManualListPreparedViewport> {
        if !self.has_usable_viewport()
            || self.pending_transaction.is_some()
            || self.is_applying_transaction
            || self.is_user_scrolling()
        {
            return None;
        }

        self.visible_viewport_preparation_depth += 1;
        self.update_mounted_items(HashSet::new(), window, cx);
        let range = self.current_visible_item_range();
        self.synchronize_visible_items(range.clone(), window, cx);
        self.layout_mounted_views(range.clone(), cx);
        let is_current = self.visible_items_are_current_and_mounted(range.clone());
        self.visible_viewport_preparation_depth -= 1;
        if !is_current {
            return None;
        }
        self.update_visible_ids(true, window, cx);
        Some(self.prepared_viewport_snapshot(range, cx))
    }

    pub fn cancel_visible_viewport_preparation(&mut self) {
        self.visible_viewport_preparation_generation = self.visible_viewport_preparation_generation.wrapping_add(1);
    }

    pub fn apply_selection_presentation(
        &mut self,
        presentation: RiseManualListSelectionPresentation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !presentation.is_active && !self.selection_presentation.is_active {
            return;
        }
        let mode_changed = self.selection_presentation.is_active != presentation.is_active;
        let content_presentation_changed =
            mode_changed || self.selection_presentation.items_by_item_id != presentation.items_by_item_id;
        let drawn_presentation_changed = content_presentation_changed
            || self.selection_presentation.selected_ids != presentation.selected_ids
            || self.selection_presentation.tint_color != presentation.tint_color;
        let is_active = presentation.is_active;
        self.selection_presentation = presentation;
        if mode_changed {
            self.bottom_sticky_header_host_view.is_hidden = is_active;
        }
        if content_presentation_changed {
            self.apply_selection_presentation_to_mounted_views(cx);
        }
        self.refresh_selection_overlay();
        if mode_changed && !is_active {
            self.update_sticky_headers(window, cx);
        }
        if drawn_presentation_changed {
            cx.notify();
        }
    }

    pub fn stop_scrolling_for_immediate_transaction(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_viewport_anchor_animation();
        self.cancel_active_jump_animation(cx);
        self.cancel_programmatic_scroll(window, cx);

        let offset = self.scroll_view.content_offset;
        self.programmatic_navigation_owns_scroll = true;
        self.is_programmatic_scroll_animating = false;

        RiseListRemovalDiag::log(
            || format!("scroll stop at offset {}", RiseListRemovalDiag::f(Some(offset.y))),
            true,
            cx,
        );
        self.scroll_view.set_content_offset(offset);
        self.scroll_view.is_tracking = false;
        self.scroll_view.is_dragging = false;
        self.scroll_view.is_decelerating = false;

        self.reset_estimated_scroll_velocity();
        self.update_mounted_items(HashSet::new(), window, cx);
        self.notify_scroll_metrics_changed(window, cx);
    }

    /// chrome and pull headers settle through the list owner after the release
    /// the destination survives origin rebasing
    pub fn settle_content_offset_from_top(&mut self, offset: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        if self.scroll_view.is_tracking || self.scroll_view.is_dragging || self.is_applying_transaction {
            return;
        }
        self.stop_scrolling_for_immediate_transaction(window, cx);
        let _ = self.set_content_offset_y(self.content_top_offset_y() + offset, true, true, window, cx);
    }

    /// temporary wash on the jump target row, appears instantly and holds until the travel landed
    /// and the row was seen then fades
    /// retargeting during a fade cancels the previous one
    pub fn flash_row_highlight(
        &mut self,
        id: &str,
        color: Hsla,
        hold_after_landing: f64,
        fade_duration: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = current_media_time();
        self.jump_highlight_animator = None;
        self.jump_highlight_view.stop_alpha_animation(now);
        if !self.item_by_id.contains_key(id) {
            return;
        }
        self.jump_highlight_item_id = Some(id.to_string());
        self.jump_highlight_color = color;
        self.reposition_jump_highlight();
        self.jump_highlight_view.alpha = 1.0;
        let duration = if reduce_motion() { fade_duration * 0.5 } else { fade_duration };
        let delay = self.remaining_travel_duration() + hold_after_landing;
        cx.notify();
        self.jump_highlight_animator = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay.max(0.0))).await;
            let started = this.update(cx, |this, cx| {
                let ease_in = RiseManualListCubicBezier::new(0.42, 0.0, 1.0, 1.0);
                this.jump_highlight_view.animate_alpha(0.0, duration, ease_in, current_media_time());
                cx.notify();
            });
            if started.is_err() {
                return;
            }
            cx.background_executor().timer(Duration::from_secs_f64(duration.max(0.0))).await;
            let _ = this.update(cx, |this, cx| {
                this.jump_highlight_item_id = None;
                cx.notify();
            });
        }));
    }

    fn remaining_travel_duration(&self) -> f64 {
        if let Some(programmatic_scroll) = &self.programmatic_scroll {
            return (programmatic_scroll.started_at + programmatic_scroll.duration - current_media_time()).max(0.0);
        }
        if self.active_jump_snapshot.is_none() { 0.0 } else { RiseManualListScrollAnimationPolicy::DURATION }
    }

    pub fn cancel_row_highlight(&mut self, cx: &mut Context<Self>) {
        self.jump_highlight_animator = None;
        self.jump_highlight_item_id = None;
        self.jump_highlight_view.remove_all_animations();
        self.jump_highlight_view.alpha = 0.0;
        cx.notify();
    }

    /// re-derived on every layout pass like the selection overlay
    /// edge mutations rebase layout_origin_y mid fade and the wash must stay glued to its row
    fn reposition_jump_highlight(&mut self) {
        let Some(id) = self.jump_highlight_item_id.as_ref() else {
            return;
        };
        let Some(item) = self.item_by_id.get(id).cloned() else {
            self.jump_highlight_view.remove_all_animations();
            self.jump_highlight_view.alpha = 0.0;
            return;
        };
        let mut frame = self.resolved_frame(&item.borrow());
        frame.origin.y += self.layout_origin_y;
        let target = Bounds::new(
            point(Pixels::ZERO, frame.origin.y),
            size(self.scroll_view.frame.size.width, frame.size.height),
        );
        if self.jump_highlight_view.frame != target {
            self.jump_highlight_view.frame = target;
        }
    }

    pub fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_row_highlight(cx);
        self.cancel_keyboard_viewport_transition(window, cx);
        self.cancel_viewport_anchor_animation();
        self.cancel_viewport_clip_animation();
        self.viewport_resize_animation = None;
        self.viewport_resize_session_generation = self.viewport_resize_session_generation.wrapping_add(1);
        self.viewport_resize_terminal_generation = None;
        self.viewport_resize_terminal_cleanup_scheduled_generation = None;
        self.viewport_resize_gesture_ownership_generation = None;
        self.viewport_resize_retained_mounted_ids = HashSet::new();
        self.cancel_active_jump_animation(cx);
        self.cancel_programmatic_scroll(window, cx);
        self.cancel_visible_viewport_preparation();
        if let Some(discard) = self.pending_transaction.take().and_then(|pending| pending.discard) {
            discard(window, cx);
        }
        self.cancel_deferred_mounting();
        self.deferred_recycle_generation = self.deferred_recycle_generation.wrapping_add(1);
        self.deferred_recycle_scheduled_generation = None;
        self.deferred_recycle_idle_since = None;
        self.detached_identity_items = Vec::new();
        self.deferred_recycle_items = Vec::new();
        self.restore_all_top_sticky_items(window, cx);
        self.unmount_all_bottom_sticky_headers(cx);
        self.selection_presentation = RiseManualListSelectionPresentation::inactive();
        self.programmatic_navigation_owns_scroll = false;
        self.bottom_sticky_header_host_view.is_hidden = false;
        self.apply_selection_presentation_to_mounted_views(cx);
        self.refresh_selection_overlay();
        cx.notify();
    }

    pub fn capture_visible_anchor(&mut self) -> Option<RiseManualListViewportAnchor> {
        let anchor = self.visible_anchor()?;
        Some(RiseManualListViewportAnchor::new(
            anchor.id,
            f32::from(anchor.absolute_min_y - self.scroll_view.content_offset.y),
        ))
    }

    pub fn capture_viewport_anchor(&self, id: &str) -> Option<RiseManualListViewportAnchor> {
        let item = self.item_by_id.get(id)?;
        let mut frame = self.resolved_frame(&item.borrow());
        frame.origin.y += self.layout_origin_y;
        let viewport = Bounds::new(self.scroll_view.content_offset, self.scroll_view.frame.size);
        if !frame.intersects(&viewport) {
            return None;
        }
        Some(RiseManualListViewportAnchor::new(
            id.to_string(),
            f32::from(frame.origin.y - self.scroll_view.content_offset.y),
        ))
    }

    pub fn scroll_to(
        &mut self,
        anchor: &RiseManualListViewportAnchor,
        animated: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_user_scrolling() || self.is_applying_transaction {
            return false;
        }
        let Some(item) = self.item_by_id.get(&anchor.id).cloned() else {
            return false;
        };
        let target_y = self.layout_origin_y + self.resolved_frame(&item.borrow()).origin.y - px(anchor.viewport_offset);
        self.set_content_offset_y(target_y, animated, true, window, cx)
    }

    pub fn set_content_offset_y(
        &mut self,
        value: Pixels,
        animated: bool,
        clamped: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_user_scrolling() || self.is_applying_transaction {
            return false;
        }
        let metrics = self.scroll_metrics();
        let target_y = if clamped {
            value.max(metrics.minimum_content_offset_y).min(metrics.maximum_content_offset_y)
        } else {
            value
        };
        if (self.scroll_view.content_offset.y - target_y).abs() <= px(0.5) {
            return true;
        }
        self.is_programmatic_scroll_animating = animated;
        if animated {
            self.pause_deferred_mounting_for_active_scroll();
            let target = self.scroll_view.set_content_offset_animated(point(Pixels::ZERO, target_y), true);
            self.programmatic_scroll_display_link = None;
            self.programmatic_scroll = None;
            let distance = self.scroll_view.content_offset.y - target.y;
            self.is_programmatic_scroll_animating = true;
            self.ride_programmatic_scroll(
                RiseManualListViewProgrammaticScroll {
                    target: RiseManualListViewProgrammaticScrollTarget::Offset(target.y),
                    started_at: current_media_time(),
                    duration: RiseManualListScrollAnimationPolicy::DURATION,
                    initial_distance: distance,
                },
                window,
                cx,
            );
            return true;
        }
        self.set_content_offset_within_overscroll_limits(point(Pixels::ZERO, target_y), cx);
        self.scroll_view_did_scroll(window, cx);
        self.update_mounted_items(HashSet::new(), window, cx);
        self.notify_scroll_metrics_changed(window, cx);
        true
    }

    pub fn scroll_to_bottom(&mut self, animated: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let maximum_offset_y = self.scroll_metrics().maximum_content_offset_y;
        self.set_content_offset_y(maximum_offset_y, animated, true, window, cx)
    }

    pub fn smart_scroll_to_bottom(&mut self, animated: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.smart_scroll(
            RiseManualListViewProgrammaticScrollTarget::Bottom,
            RiseManualListTransactionJumpAnimationDirection::TowardNewer,
            animated,
            window,
            cx,
        )
    }

    /// status bar tap and a re-tap of the open tab (on desktop Home and a re-click of the tab)
    /// the travel holds commits back like a finger does
    pub fn smart_scroll_to_top(&mut self, animated: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.scroll_view.is_tracking || self.is_applying_transaction || !self.has_usable_viewport() {
            return false;
        }
        if self.is_user_scrolling() {
            self.stop_scrolling_for_immediate_transaction(window, cx);
        }
        self.smart_scroll(
            RiseManualListViewProgrammaticScrollTarget::Top,
            RiseManualListTransactionJumpAnimationDirection::TowardOlder,
            animated,
            window,
            cx,
        )
    }

    /// a programmatic travel owns the offset like a finger does
    /// work computed against the viewport now would land only after it stops
    pub fn is_programmatic_scroll_in_flight(&self) -> bool {
        self.is_programmatic_scroll_animating
    }

    pub fn smart_scroll_to_item_centered(
        &mut self,
        id: &str,
        direction: RiseManualListTransactionJumpAnimationDirection,
        animated: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item) = self.item_by_id.get(id).cloned() else {
            return false;
        };

        let item_frame = self.resolved_frame(&item.borrow());
        let absolute_min_y = self.layout_origin_y + item_frame.origin.y;
        let absolute_max_y = absolute_min_y + item_frame.size.height;
        let offset_y = self.scroll_view.content_offset.y;
        let visible_top = offset_y + self.viewport_insets.top;
        let visible_bottom = offset_y + self.bounds.size.height - self.viewport_insets.bottom;
        if absolute_min_y >= visible_top - px(0.5) && absolute_max_y <= visible_bottom + px(0.5) {
            return !self.is_user_scrolling() && !self.is_applying_transaction;
        }

        let visible_height =
            (self.bounds.size.height - self.viewport_insets.top - self.viewport_insets.bottom).max(px(1.0));
        let item_height = px(item.borrow().content.height);
        let viewport_offset = self.viewport_insets.top + ((visible_height - item_height) * 0.5).max(Pixels::ZERO);
        self.smart_scroll(
            RiseManualListViewProgrammaticScrollTarget::Item { id: id.to_string(), viewport_offset },
            direction,
            animated,
            window,
            cx,
        )
    }

    /// brings an item to the top of the viewport and not its middle
    /// a date separator has to sit under the chrome with the first messages of the day below
    pub fn smart_scroll_to_item_at_top(
        &mut self,
        id: &str,
        direction: RiseManualListTransactionJumpAnimationDirection,
        animated: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.item_by_id.contains_key(id) {
            return false;
        }
        let viewport_offset = self.viewport_insets.top;
        self.smart_scroll(
            RiseManualListViewProgrammaticScrollTarget::Item { id: id.to_string(), viewport_offset },
            direction,
            animated,
            window,
            cx,
        )
    }

    /// two travels like telegram, within a viewport it really scrolls frame by frame
    /// farther than that the list lands at once and the old viewport slides away
    fn smart_scroll(
        &mut self,
        target: RiseManualListViewProgrammaticScrollTarget,
        direction: RiseManualListTransactionJumpAnimationDirection,
        animated: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_user_scrolling() || self.is_applying_transaction {
            return false;
        }
        let Some(target_y) = self.programmatic_scroll_offset(&target) else {
            return false;
        };

        let distance = (self.scroll_view.content_offset.y - target_y).abs();
        if distance <= px(0.5) {
            return true;
        }
        if !animated {
            return self.set_content_offset_y(target_y, false, true, window, cx);
        }

        if f32::from(distance)
            <= RiseManualListScrollAnimationPolicy::continuous_distance_limit(f32::from(self.bounds.size.height))
        {
            self.begin_programmatic_scroll(target, target_y, window, cx);
            return true;
        }

        let snapshot = self.capture_jump_snapshot(cx);
        if !self.set_content_offset_y(target_y, false, true, window, cx) {
            return false;
        }
        let Some(snapshot) = snapshot else {
            return true;
        };

        self.animate_jump(
            snapshot,
            RiseManualListTransactionJumpAnimation::new(direction, RiseManualListScrollAnimationPolicy::DURATION),
            cx,
        );
        true
    }

    /// bottom inset the viewport is settling on
    /// a composer or picker transition that doesnt carry the rows keeps the old taller inset
    /// committed until it finishes
    fn landing_bottom_inset(&self) -> Pixels {
        if !self.keyboard_viewport_transition_is_active || self.keyboard_viewport_keeps_bottom_anchor {
            return self.viewport_insets.bottom;
        }
        let Some(target_insets) = self.keyboard_viewport_target_insets else {
            return self.viewport_insets.bottom;
        };
        self.viewport_insets.bottom.min(target_insets.bottom)
    }

    fn programmatic_scroll_offset(&self, target: &RiseManualListViewProgrammaticScrollTarget) -> Option<Pixels> {
        let value = match target {
            RiseManualListViewProgrammaticScrollTarget::Top => self.content_top_offset_y(),
            RiseManualListViewProgrammaticScrollTarget::Bottom => {
                self.layout_origin_y + self.layout_height - self.bounds.size.height + self.landing_bottom_inset()
            }
            RiseManualListViewProgrammaticScrollTarget::Item { id, viewport_offset } => {
                let item = self.item_by_id.get(id)?;
                self.layout_origin_y + self.resolved_frame(&item.borrow()).origin.y - *viewport_offset
            }
            RiseManualListViewProgrammaticScrollTarget::Offset(offset_y) => *offset_y,
        };
        if !f32::from(value).is_finite() {
            return None;
        }
        let metrics = self.scroll_metrics();
        Some(value.max(metrics.minimum_content_offset_y).min(metrics.maximum_content_offset_y))
    }

    fn programmatic_scroll_target(
        &self,
        anchor: &RiseManualListTransactionAnchor,
        destination_y: Pixels,
    ) -> RiseManualListViewProgrammaticScrollTarget {
        match anchor {
            RiseManualListTransactionAnchor::Bottom => RiseManualListViewProgrammaticScrollTarget::Bottom,
            RiseManualListTransactionAnchor::Item { id, viewport_offset } => {
                RiseManualListViewProgrammaticScrollTarget::Item { id: id.clone(), viewport_offset: px(*viewport_offset) }
            }
            RiseManualListTransactionAnchor::Top | RiseManualListTransactionAnchor::PreserveVisible => {
                RiseManualListViewProgrammaticScrollTarget::Offset(destination_y)
            }
        }
    }

    fn begin_programmatic_scroll(
        &mut self,
        target: RiseManualListViewProgrammaticScrollTarget,
        destination_y: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_programmatic_scroll(window, cx);
        self.cancel_active_jump_animation(cx);
        let distance = self.scroll_view.content_offset.y - destination_y;
        if distance.abs() <= px(0.5) {
            return;
        }
        ChatPerformanceTrace::shared().record(
            "list.travel",
            HashMap::from([
                ("mode".to_string(), "scroll".to_string()),
                ("distance".to_string(), (f32::from(distance.abs()).round() as i64).to_string()),
            ]),
        );

        let duration = if reduce_motion() {
            RiseManualListScrollAnimationPolicy::DURATION * 0.6
        } else {
            RiseManualListScrollAnimationPolicy::DURATION
        };
        self.is_programmatic_scroll_animating = true;
        self.pause_deferred_mounting_for_active_scroll();
        self.ride_programmatic_scroll(
            RiseManualListViewProgrammaticScroll {
                target,
                started_at: current_media_time(),
                duration,
                initial_distance: distance,
            },
            window,
            cx,
        );
    }

    /// installs scroll and the frame loop that drives it
    fn ride_programmatic_scroll(
        &mut self,
        scroll: RiseManualListViewProgrammaticScroll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.programmatic_scroll = Some(scroll);
        self.programmatic_scroll_display_link =
            Some(Self::add_display_link(window, cx, Self::programmatic_scroll_did_tick));
    }

    /// display link at the display own rate, tick runs once per frame with its timestamps
    /// until the returned task is dropped or the list is gone
    fn add_display_link(
        window: &mut Window,
        cx: &mut Context<Self>,
        tick: fn(&mut Self, RiseManualListMountFrame, &mut Window, &mut Context<Self>),
    ) -> Task<()> {
        let maximum_fps = maximum_frames_per_second(window).max(60);
        let interval = 1.0 / f64::from(maximum_fps);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs_f64(interval)).await;
                let timestamp = current_media_time();
                let Some(frame) = RiseManualListMountFrame::new(timestamp, timestamp + interval) else {
                    continue;
                };
                if this.update_in(cx, |this, window, cx| tick(this, frame, window, cx)).is_err() {
                    break;
                }
            }
        })
    }

    fn programmatic_scroll_did_tick(&mut self, link: RiseManualListMountFrame, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scroll) = self.programmatic_scroll.clone() else {
            self.programmatic_scroll_display_link = None;
            return;
        };
        if !self.is_in_window || !self.has_usable_viewport() {
            self.complete_programmatic_scroll(window, cx);
            return;
        }
        let elapsed = (link.target_timestamp - scroll.started_at).max(0.0);
        let fraction = if scroll.duration > 0.0 { (elapsed / scroll.duration).min(1.0) } else { 1.0 };
        let destination_y = if fraction < 1.0 { self.programmatic_scroll_offset(&scroll.target) } else { None };
        let Some(destination_y) = destination_y else {
            self.complete_programmatic_scroll(window, cx);
            return;
        };
        let remaining = scroll.initial_distance * (1.0 - RiseManualListScrollAnimationPolicy::progress(fraction)) as f32;
        self.apply_programmatic_scroll_offset(destination_y + remaining, window, cx);
    }

    /// writes the offset through the scroll view and its callback
    /// so rows mount and sticky headers move and metrics go out like for the wheel
    fn apply_programmatic_scroll_offset(&mut self, offset_y: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        if !f32::from(offset_y).is_finite() || (self.scroll_view.content_offset.y - offset_y).abs() <= px(0.01) {
            return;
        }
        self.set_content_offset_within_overscroll_limits(point(Pixels::ZERO, offset_y), cx);
        self.scroll_view_did_scroll(window, cx);
    }

    /// lands the travel on its destination and settles like a native animated scroll
    fn complete_programmatic_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scroll) = self.programmatic_scroll.clone() else {
            return;
        };
        let destination_y = self.programmatic_scroll_offset(&scroll.target);
        self.programmatic_scroll_display_link = None;
        self.programmatic_scroll = None;
        if let Some(destination_y) = destination_y {
            self.apply_programmatic_scroll_offset(destination_y, window, cx);
        }
        self.is_programmatic_scroll_animating = false;
        self.settle_after_programmatic_scroll(window, cx);
    }

    fn is_programmatic_scroll_heading_to_bottom(&self) -> bool {
        matches!(
            self.programmatic_scroll.as_ref().map(|scroll| &scroll.target),
            Some(RiseManualListViewProgrammaticScrollTarget::Bottom)
        )
    }

    /// stops a travel where it is, for an owner that takes the offset over itself
    fn cancel_programmatic_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.programmatic_scroll.is_none() && self.programmatic_scroll_display_link.is_none() {
            return;
        }
        self.programmatic_scroll_display_link = None;
        self.programmatic_scroll = None;
        self.is_programmatic_scroll_animating = false;
        self.schedule_top_sticky_flash_end(window, cx);
    }

    fn application_will_resign_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pause_deferred_mount_display_link();
        self.complete_programmatic_scroll(window, cx);
    }

    fn capture_jump_origin(&mut self, cx: &mut Context<Self>) -> RiseManualListViewJumpOrigin {
        let snapshot = self.capture_jump_snapshot(cx);
        let reference = self.visible_anchor();
        let offset_y = self.scroll_view.content_offset.y;
        RiseManualListViewJumpOrigin {
            snapshot,
            reference_viewport_y: reference.as_ref().map_or(Pixels::ZERO, |reference| reference.absolute_min_y) - offset_y,
            reference_id: reference.map(|reference| reference.id),
        }
    }

    /// plays the jump a committed transaction asked for
    /// a row that survived tells how far the viewport moved, not at all plays nothing
    /// up to a viewport scrolls for real, farther slides the old pixels away
    fn finish_jump_transition(
        &mut self,
        origin: Option<RiseManualListViewJumpOrigin>,
        transaction: &RiseManualListTransaction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(origin), Some(animation)) = (origin, transaction.jump_animation) else {
            return;
        };
        if let Some(item) = origin.reference_id.as_ref().and_then(|id| self.item_by_id.get(id)).cloned() {
            let viewport_y =
                self.layout_origin_y + self.resolved_frame(&item.borrow()).origin.y - self.scroll_view.content_offset.y;
            let displacement = viewport_y - origin.reference_viewport_y;
            if displacement.abs() <= px(0.5) {
                return;
            }
            if f32::from(displacement.abs())
                <= RiseManualListScrollAnimationPolicy::continuous_distance_limit(f32::from(self.bounds.size.height))
            {
                let destination_y = self.scroll_view.content_offset.y;
                let metrics = self.scroll_metrics();
                let start_y = (destination_y + displacement)
                    .max(metrics.minimum_content_offset_y)
                    .min(metrics.maximum_content_offset_y);
                self.apply_programmatic_scroll_offset(start_y, window, cx);
                self.update_mounted_items(HashSet::new(), window, cx);
                self.notify_scroll_metrics_changed(window, cx);
                let target = self.programmatic_scroll_target(&transaction.anchor, destination_y);
                self.begin_programmatic_scroll(target, destination_y, window, cx);
                return;
            }
        }
        let Some(snapshot) = origin.snapshot else {
            return;
        };
        self.animate_jump(snapshot, animation, cx);
    }

    /// rows and avatars beside them as they are on screen now
    /// the floating date is left out, it stays and shows the destination day
    fn capture_jump_snapshot(&mut self, cx: &mut Context<Self>) -> Option<RiseManualListViewJumpSnapshot> {
        self.cancel_viewport_anchor_animation();
        self.cancel_active_jump_animation(cx);
        if !self.is_in_window
            || self.bounds.size.width <= px(1.0)
            || self.bounds.size.height <= px(1.0)
        {
            return None;
        }

        let offset_y = self.scroll_view.content_offset.y;
        let viewport = Bounds::new(Point::default(), self.viewport_clip_view.frame.size);
        let mut rows = Vec::new();
        for (id, mounted) in &self.mounted_items {
            let Some(item) = self.item_by_id.get(id) else {
                continue;
            };
            let mut frame = self.resolved_frame(&item.borrow());
            frame.origin.y += self.layout_origin_y - offset_y + self.scroll_view.frame.origin.y;
            frame.origin.x += self.scroll_view.frame.origin.x;
            if frame.intersects(&viewport) {
                rows.push((mounted.view.clone(), frame));
            }
        }
        let mut avatars = Vec::new();
        if !self.bottom_sticky_header_host_view.is_hidden {
            for (id, visible) in &self.visible_bottom_sticky_headers_buffer {
                let Some(mounted) = self.mounted_bottom_sticky_headers.get(id) else {
                    continue;
                };
                let mut frame = visible.frame;
                frame.origin.x += self.bottom_sticky_header_host_view.frame.origin.x;
                frame.origin.y += self.bottom_sticky_header_host_view.frame.origin.y;
                avatars.push((mounted.view.clone(), frame));
            }
        }
        Some(RiseManualListViewJumpSnapshot {
            rows,
            avatars,
            layer: RiseManualListViewHostLayer { frame: viewport, ..RiseManualListViewHostLayer::default() },
        })
    }

    /// far jump like telegram, the destination is laid out at once and arrives from one viewport away
    /// touching the old viewport which leaves by the same distance
    fn animate_jump(
        &mut self,
        mut snapshot: RiseManualListViewJumpSnapshot,
        animation: RiseManualListTransactionJumpAnimation,
        cx: &mut Context<Self>,
    ) {
        self.cancel_active_jump_animation(cx);
        ChatPerformanceTrace::shared().record(
            "list.travel",
            HashMap::from([
                ("mode".to_string(), "slide".to_string()),
                (
                    "direction".to_string(),
                    match animation.direction {
                        RiseManualListTransactionJumpAnimationDirection::TowardNewer => "newer",
                        RiseManualListTransactionJumpAnimationDirection::TowardOlder => "older",
                    }
                    .to_string(),
                ),
            ]),
        );

        let now = current_media_time();
        snapshot.layer.frame = Bounds::new(Point::default(), self.viewport_clip_view.frame.size);
        self.active_jump_generation = self.active_jump_generation.wrapping_add(1);
        let generation = self.active_jump_generation;

        let duration = if reduce_motion() {
            snapshot.layer.alpha = 0.0;
            snapshot.layer.alpha_animation = Some(RiseManualListViewOpacityAnimation {
                from_alpha: 1.0,
                started_at: now,
                duration: 0.18,
                curve: RiseManualListCubicBezier::new(0.0, 0.0, 0.58, 1.0),
            });
            0.18
        } else {
            let travel = self.viewport_clip_view.frame.size.height.max(px(1.0));
            let incoming_y = match animation.direction {
                RiseManualListTransactionJumpAnimationDirection::TowardOlder => -travel,
                RiseManualListTransactionJumpAnimationDirection::TowardNewer => travel,
            };
            if !self.viewport_clip_animation_is_active && !self.keyboard_viewport_transition_is_active {
                self.scroll_view_clips_to_bounds = true;
            }
            snapshot.layer.set_translation_y(-incoming_y);
            snapshot
                .layer
                .add_translation_animation(Self::jump_slide_animation(Pixels::ZERO, animation.duration, now));
            self.jump_slide_layer.set_translation_y(Pixels::ZERO);
            self.jump_slide_layer
                .add_translation_animation(Self::jump_slide_animation(incoming_y, animation.duration, now));
            animation.duration
        };
        self.active_jump_snapshot = Some(snapshot);
        cx.notify();

        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(duration.max(0.0))).await;
            let _ = this.update(cx, |this, cx| {
                if this.active_jump_generation != generation {
                    return;
                }
                this.active_jump_snapshot = None;
                this.scroll_view_clips_to_bounds = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// translation from start_y to the layer model value on the scroll travel curve
    fn jump_slide_animation(start_y: Pixels, duration: f64, now: f64) -> RiseManualListViewTranslationAnimation {
        RiseManualListViewTranslationAnimation {
            from_y: start_y,
            started_at: now,
            duration,
            curve: Some(Self::jump_slide_curve()),
        }
    }

    fn jump_slide_curve() -> RiseManualListCubicBezier {
        let timing_function = RiseManualListScrollAnimationPolicy::timing_function();
        RiseManualListCubicBezier::new(timing_function.0, timing_function.1, timing_function.2, timing_function.3)
    }

    fn cancel_active_jump_animation(&mut self, cx: &mut Context<Self>) {
        self.active_jump_generation = self.active_jump_generation.wrapping_add(1);
        self.jump_slide_layer.remove_all_animations();
        self.jump_slide_layer.set_translation_y(Pixels::ZERO);
        if self.active_jump_snapshot.take().is_some() {
            cx.notify();
        }
        self.scroll_view_clips_to_bounds = false;
    }

    /// everything but a running jump slide, which has its own cancellation
    /// keyboard and viewport cleanups stripped it from the avatar host mid flight and left avatars behind
    fn remove_animations_keeping_jump_slide(layer: &mut RiseManualListViewHostLayer) {
        layer.remove_all_animations();
    }
}

impl RiseManualListView {
    fn current_viewport_anchor_presentation_translation_y(&self) -> Pixels {
        if !self.viewport_anchor_animation_is_active {
            return Pixels::ZERO;
        }
        if self.viewport_anchor_animation_presentation_committed {
            let value = self.item_host_view.presentation_translation_y(current_media_time());
            if f32::from(value).is_finite() {
                return value;
            }
        }
        self.viewport_anchor_animation_start_offset_y
    }

    fn cancel_viewport_anchor_animation(&mut self) {
        self.viewport_anchor_animation_generation = self.viewport_anchor_animation_generation.wrapping_add(1);
        self.viewport_anchor_animation_is_active = false;
        self.viewport_anchor_animation_presentation_committed = false;
        self.viewport_anchor_animation_start_offset_y = Pixels::ZERO;
        for target in self.viewport_anchor_animation_targets() {
            target.remove_all_animations();
            target.set_translation_y(Pixels::ZERO);
        }
    }

    fn viewport_anchor_animation_targets(&mut self) -> [&mut RiseManualListViewHostLayer; 3] {
        [&mut self.item_host_view, &mut self.selection_overlay_layer, &mut self.bottom_sticky_header_host_view]
    }

    fn take_ownership_of_viewport_anchor_animation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.viewport_anchor_animation_is_active {
            return;
        }
        let presentation_translation_y = self.current_viewport_anchor_presentation_translation_y();
        self.cancel_viewport_anchor_animation();
        if !f32::from(presentation_translation_y).is_finite() || presentation_translation_y.abs() <= px(0.01) {
            return;
        }

        self.layout_origin_y += presentation_translation_y;
        self.update_host_frame();
        self.update_scroll_geometry();
        self.update_mounted_items(HashSet::new(), window, cx);
        self.notify_scroll_metrics_changed(window, cx);
    }

    fn animate_viewport_anchor_transition(
        &mut self,
        previous_origin_y: Pixels,
        previous_presentation_translation_y: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = current_media_time();
        let remaining_duration = if self.gesture_owns_viewport_geometry() {
            None
        } else {
            self.viewport_resize_animation.and_then(|animation| animation.remaining_duration(now))
        };
        let Some(remaining_duration) = remaining_duration else {
            if self.viewport_anchor_animation_is_active {
                self.cancel_viewport_anchor_animation();
            }
            return;
        };

        let inverse_translation_y = px(RiseManualListViewportAnchorAnimationPolicy::inverse_translation_y(
            f32::from(previous_origin_y),
            f32::from(self.layout_origin_y - self.scroll_view.content_offset.y),
            f32::from(previous_presentation_translation_y),
        ));
        if !f32::from(inverse_translation_y).is_finite() || inverse_translation_y.abs() <= px(0.25) {
            if self.viewport_anchor_animation_is_active {
                self.cancel_viewport_anchor_animation();
            }
            return;
        }

        self.cancel_active_jump_animation(cx);
        self.cancel_viewport_anchor_animation();

        for target in self.viewport_anchor_animation_targets() {
            target.set_translation_y(inverse_translation_y);
        }

        self.viewport_anchor_animation_generation = self.viewport_anchor_animation_generation.wrapping_add(1);
        let generation = self.viewport_anchor_animation_generation;
        self.viewport_anchor_animation_is_active = true;
        self.viewport_anchor_animation_presentation_committed = false;
        self.viewport_anchor_animation_start_offset_y = inverse_translation_y;
        #[cfg(debug_assertions)]
        {
            self.last_viewport_anchor_animation_offset_for_testing = Some(inverse_translation_y);
        }

        for target in self.viewport_anchor_animation_targets() {
            target.animate_translation_y(Pixels::ZERO, remaining_duration, now);
        }
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(remaining_duration)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.viewport_anchor_animation_generation != generation {
                    return;
                }
                this.viewport_anchor_animation_is_active = false;
                this.viewport_anchor_animation_presentation_committed = false;
                this.viewport_anchor_animation_start_offset_y = Pixels::ZERO;
                this.try_complete_terminal_viewport_resize_session(this.viewport_resize_session_generation, window, cx);
            });
        })
        .detach();
        cx.on_next_frame(window, move |this, _, _| {
            if this.viewport_anchor_animation_generation != generation || !this.viewport_anchor_animation_is_active {
                return;
            }
            this.viewport_anchor_animation_presentation_committed = true;
        });
    }

    fn viewport_clip_presentation_state(frame: Bounds<Pixels>) -> RiseManualListViewViewportClipPresentationState {
        RiseManualListViewViewportClipPresentationState {
            bounds: Bounds::new(Point::default(), frame.size),
            position: frame.center(),
        }
    }

    fn viewport_clip_frame(state: RiseManualListViewViewportClipPresentationState) -> Bounds<Pixels> {
        Bounds::new(
            point(
                state.position.x - state.bounds.size.width * 0.5,
                state.position.y - state.bounds.size.height * 0.5,
            ),
            state.bounds.size,
        )
    }

    fn current_viewport_clip_presentation_state(&self) -> RiseManualListViewViewportClipPresentationState {
        if self.viewport_clip_animation_is_active {
            if self.viewport_clip_animation_presentation_committed {
                let presentation = self.viewport_clip_view.presentation_frame(current_media_time());
                if [presentation.origin.x, presentation.origin.y, presentation.size.width, presentation.size.height]
                    .iter()
                    .all(|value| f32::from(*value).is_finite())
                {
                    return Self::viewport_clip_presentation_state(presentation);
                }
            }
            if let Some(start_state) = self.viewport_clip_animation_start_state {
                return start_state;
            }
        }
        Self::viewport_clip_presentation_state(self.viewport_clip_view.frame)
    }

    fn cancel_viewport_clip_animation(&mut self) {
        self.viewport_clip_animation_generation = self.viewport_clip_animation_generation.wrapping_add(1);
        self.viewport_clip_animation_is_active = false;
        self.viewport_clip_animation_presentation_committed = false;
        self.viewport_clip_animation_start_state = None;
        self.viewport_clip_view.remove_all_animations();
        self.viewport_clip_view.frame = Bounds::new(Point::default(), self.bounds.size);
    }

    fn animate_viewport_clip_transition(
        &mut self,
        previous_state: RiseManualListViewViewportClipPresentationState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let final_state = Self::viewport_clip_presentation_state(self.viewport_clip_view.frame);
        let changed = (previous_state.bounds.size.height - final_state.bounds.size.height).abs() > px(0.25)
            || (previous_state.bounds.size.width - final_state.bounds.size.width).abs() > px(0.25)
            || (previous_state.position.x - final_state.position.x).abs() > px(0.25)
            || (previous_state.position.y - final_state.position.y).abs() > px(0.25);
        if !changed {
            return;
        }

        let now = current_media_time();
        let remaining_duration = if self.viewport_resize_gesture_ownership_generation.is_none()
            && !self.is_user_scrolling()
            && previous_state.bounds.size.width > Pixels::ZERO
            && previous_state.bounds.size.height > Pixels::ZERO
        {
            self.viewport_resize_animation.and_then(|animation| animation.remaining_duration(now))
        } else {
            None
        };
        let Some(remaining_duration) = remaining_duration else {
            self.cancel_viewport_clip_animation();
            return;
        };

        self.viewport_clip_animation_generation = self.viewport_clip_animation_generation.wrapping_add(1);
        let generation = self.viewport_clip_animation_generation;
        self.viewport_clip_animation_is_active = true;
        self.scroll_view_clips_to_bounds = false;
        self.viewport_clip_animation_presentation_committed = false;
        self.viewport_clip_animation_start_state = Some(previous_state);
        #[cfg(debug_assertions)]
        {
            self.last_viewport_clip_animation_start_height_for_testing = Some(previous_state.bounds.size.height);
        }

        self.viewport_clip_view.remove_all_animations();
        self.viewport_clip_view.animate_frame(
            Self::viewport_clip_frame(previous_state),
            Self::viewport_clip_frame(final_state),
            remaining_duration,
            now,
        );
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(remaining_duration)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.viewport_clip_animation_generation != generation {
                    return;
                }
                this.viewport_clip_animation_is_active = false;
                this.viewport_clip_animation_presentation_committed = false;
                this.viewport_clip_animation_start_state = None;
                this.try_complete_terminal_viewport_resize_session(this.viewport_resize_session_generation, window, cx);
            });
        })
        .detach();
        cx.on_next_frame(window, move |this, _, _| {
            if this.viewport_clip_animation_generation != generation || !this.viewport_clip_animation_is_active {
                return;
            }
            this.viewport_clip_animation_presentation_committed = true;
        });
    }

    fn claim_viewport_resize_for_gesture_if_needed(&mut self) {
        if self.viewport_resize_animation.is_none()
            && !self.viewport_anchor_animation_is_active
            && !self.viewport_clip_animation_is_active
            && self.viewport_resize_terminal_generation.is_none()
        {
            return;
        }
        let now = current_media_time();
        let has_fresh_resize_animation = self
            .viewport_resize_animation
            .is_some_and(|animation| animation.remaining_duration(now).is_some());
        if !has_fresh_resize_animation
            && !self.viewport_anchor_animation_is_active
            && !self.viewport_clip_animation_is_active
            && self.viewport_resize_terminal_generation.is_none()
        {
            return;
        }
        self.viewport_resize_gesture_ownership_generation = Some(self.viewport_resize_session_generation);
    }

    fn schedule_terminal_viewport_resize_cleanup(&mut self, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.viewport_resize_terminal_generation != Some(generation)
            || self.viewport_resize_terminal_cleanup_scheduled_generation == Some(generation)
        {
            return;
        }
        self.viewport_resize_terminal_cleanup_scheduled_generation = Some(generation);
        cx.defer_in(window, move |this, window, cx| {
            if this.viewport_resize_terminal_generation != Some(generation)
                || this.viewport_resize_session_generation != generation
            {
                return;
            }
            this.viewport_resize_terminal_cleanup_scheduled_generation = None;
            let bounds = this.bounds;
            this.layout_subviews(bounds, window, cx);

            cx.defer_in(window, move |this, window, cx| {
                this.try_complete_terminal_viewport_resize_session(generation, window, cx);
            });
        });
    }

    fn try_complete_terminal_viewport_resize_session(&mut self, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.viewport_resize_terminal_generation != Some(generation)
            || self.viewport_resize_session_generation != generation
            || self.is_user_scrolling()
            || self.pending_geometry_refresh
            || self.viewport_anchor_animation_is_active
            || self.viewport_clip_animation_is_active
        {
            return;
        }

        self.viewport_resize_animation = None;
        self.viewport_resize_terminal_generation = None;
        self.viewport_resize_terminal_cleanup_scheduled_generation = None;
        self.viewport_resize_gesture_ownership_generation = None;
        if !self.viewport_resize_retained_mounted_ids.is_empty() {
            self.viewport_resize_retained_mounted_ids.clear();
            self.mounted_range = 0..0;
            self.immediate_mount_range = 0..0;
            self.update_mounted_items(HashSet::new(), window, cx);
        }
    }
}

impl RiseManualListView {
    pub fn scroll_view_did_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let trace_token = ChatPerformanceTrace::shared().begin_operation("scroll.callback", HashMap::new());
        'body: {
            self.clamp_bottom_overscroll_if_needed(cx);
            let active_metrics = self.scroll_metrics();
            let reports_active_gesture =
                active_metrics.is_tracking || active_metrics.is_dragging || active_metrics.is_decelerating;
            if reports_active_gesture && !self.is_updating_scroll_geometry {
                self.update_estimated_scroll_velocity();
                if let Some(on_active_scroll_metrics_changed) = self.on_active_scroll_metrics_changed.clone() {
                    on_active_scroll_metrics_changed(&self.scroll_metrics(), window, cx);
                }
            }
            if reports_active_gesture || self.is_programmatic_scroll_animating {
                self.flash_top_sticky_headers(window, cx);
            }
            if self.is_updating_scroll_geometry || self.is_applying_transaction {
                break 'body;
            }
            if self.is_user_scrolling() {
                self.claim_viewport_resize_for_gesture_if_needed();
            }
            if !reports_active_gesture {
                self.update_estimated_scroll_velocity();
            }
            self.update_mounted_items(HashSet::new(), window, cx);
            self.retry_pending_scroll_safe_transaction_if_needed(window, cx);
            self.notify_scroll_metrics_changed(window, cx);
        }
        ChatPerformanceTrace::shared().end_operation(trace_token, 2.0, HashMap::new());
        self.update_performance_trace_context(None);
    }

    fn retry_pending_scroll_safe_transaction_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_user_scrolling()
            || self.is_applying_transaction
            || self.is_scroll_safe_retry_scheduled
            || !self
                .pending_transaction
                .as_ref()
                .is_some_and(|pending| pending.value.allows_application_during_scroll)
        {
            return;
        }
        let range = self.current_visible_item_range();
        if self.last_scroll_safe_retry_range.as_ref() == Some(&range) {
            return;
        }
        self.last_scroll_safe_retry_range = Some(range);
        self.is_scroll_safe_retry_scheduled = true;
        cx.defer_in(window, |this, window, cx| {
            this.is_scroll_safe_retry_scheduled = false;
            this.drain_transactions(window, cx);
        });
    }

    pub fn scroll_view_will_begin_dragging(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let offset_y = self.scroll_view.content_offset.y;
        ChatPerformanceTrace::shared().scroll_began(f32::from(offset_y));
        RiseListJumpDebug::begin_session(cx.entity_id(), cx);
        let message = format!(
            "SCROLL-BEGIN offsetY={} bounds={} insetTop={} insetBottom={} stickyInset={} originY={} layoutH={} items={} mounted={} preventsBottomOverscroll={} clampDisabled={} maxOffsetY={}",
            RiseListJumpDebug::f(offset_y),
            RiseListJumpDebug::f(self.bounds.size.height),
            RiseListJumpDebug::f(self.viewport_insets.top),
            RiseListJumpDebug::f(self.viewport_insets.bottom),
            RiseListJumpDebug::f_optional(self.bottom_sticky_viewport_inset),
            RiseListJumpDebug::f(self.layout_origin_y),
            RiseListJumpDebug::f(self.layout_height),
            self.layout_items.count(),
            self.mounted_items.len(),
            self.prevents_bottom_overscroll,
            RiseListJumpDebug::disables_bottom_clamp(),
            RiseListJumpDebug::f(
                self.layout_origin_y + self.layout_height - self.bounds.size.height + self.viewport_insets.bottom
            ),
        );
        RiseListJumpDebug::log(&message, cx);
        self.cancel_programmatic_scroll(window, cx);
        self.cancel_active_jump_animation(cx);
        self.take_ownership_of_viewport_anchor_animation(window, cx);
        self.pause_deferred_mounting_for_active_scroll();
        self.claim_viewport_resize_for_gesture_if_needed();
        self.programmatic_navigation_owns_scroll = false;
        self.is_programmatic_scroll_animating = false;
        self.last_scroll_sample_offset_y = Some(offset_y);
        self.last_scroll_sample_time = Some(current_media_time());
        self.estimated_scroll_velocity_y = Pixels::ZERO;
    }

    pub fn scroll_view_will_end_dragging(&mut self, velocity: Point<f32>, target_content_offset: &mut Point<Pixels>) {
        ChatPerformanceTrace::shared().scroll_will_end(velocity.y, f32::from(target_content_offset.y));
        self.claim_viewport_resize_for_gesture_if_needed();
        let projected_distance = f32::from(target_content_offset.y - self.scroll_view.content_offset.y);
        if projected_distance.abs() <= 1.0 {
            return;
        }
        let projected_velocity = 12_000.0_f32.min(projected_distance.abs() / 0.24);
        let gesture_velocity =
            12_000.0_f32.min(velocity.y.abs() * 320.0_f32.max(f32::from(self.scroll_view.bounds().size.height)));
        let resolved_velocity =
            projected_velocity.max(gesture_velocity.max(f32::from(self.estimated_scroll_velocity_y).abs()));
        self.estimated_scroll_velocity_y =
            px(if projected_distance > 0.0 { resolved_velocity } else { -resolved_velocity });
    }

    /// scroll view did end dragging hook, decelerate says if momentum follows
    pub fn scroll_view_did_end_dragging(&mut self, decelerate: bool, window: &mut Window, cx: &mut Context<Self>) {
        if decelerate {
            self.update_performance_trace_context(Some("decelerating"));
            self.notify_drag_ended(decelerate, window, cx);
            return;
        }
        ChatPerformanceTrace::shared().scroll_ended("drag", f32::from(self.scroll_view.content_offset.y));
        let trace_token = ChatPerformanceTrace::shared().begin_operation("scroll.settle", HashMap::new());
        self.reset_estimated_scroll_velocity();
        self.drain_transactions(window, cx);
        self.flush_deferred_geometry_if_possible(window, cx);
        self.update_mounted_items(HashSet::new(), window, cx);
        self.notify_scroll_metrics_changed(window, cx);
        if let Some(generation) = self.viewport_resize_terminal_generation {
            self.try_complete_terminal_viewport_resize_session(generation, window, cx);
        }
        self.schedule_top_sticky_flash_end(window, cx);
        self.settle_scroll_geometry_at_rest(window, cx);
        self.update_performance_trace_context(None);
        ChatPerformanceTrace::shared().end_operation(
            trace_token,
            1.0,
            HashMap::from([("reason".to_string(), "drag".to_string())]),
        );
        self.notify_drag_ended(decelerate, window, cx);
    }

    fn notify_drag_ended(&self, will_decelerate: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_drag_ended) = self.on_drag_ended.clone() {
            on_drag_ended(will_decelerate, window, cx);
        }
    }

    pub fn scroll_view_did_end_decelerating(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ChatPerformanceTrace::shared().scroll_ended("deceleration", f32::from(self.scroll_view.content_offset.y));
        let trace_token = ChatPerformanceTrace::shared().begin_operation("scroll.settle", HashMap::new());
        self.reset_estimated_scroll_velocity();
        self.drain_transactions(window, cx);
        self.flush_deferred_geometry_if_possible(window, cx);
        self.update_mounted_items(HashSet::new(), window, cx);
        self.notify_scroll_metrics_changed(window, cx);
        if let Some(generation) = self.viewport_resize_terminal_generation {
            self.try_complete_terminal_viewport_resize_session(generation, window, cx);
        }
        self.schedule_top_sticky_flash_end(window, cx);
        self.settle_scroll_geometry_at_rest(window, cx);
        self.update_performance_trace_context(None);
        ChatPerformanceTrace::shared().end_operation(
            trace_token,
            1.0,
            HashMap::from([("reason".to_string(), "deceleration".to_string())]),
        );
    }

    pub fn scroll_view_did_end_scrolling_animation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.is_programmatic_scroll_animating = false;
        self.settle_after_programmatic_scroll(window, cx);
    }

    pub fn scroll_view_should_scroll_to_top(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.smart_scroll_to_top(true, window, cx);
        false
    }

    /// end of a programmatic travel, the platform one or the list one
    fn settle_after_programmatic_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ChatPerformanceTrace::shared().scroll_ended("animation", f32::from(self.scroll_view.content_offset.y));
        let trace_token = ChatPerformanceTrace::shared().begin_operation("scroll.settle", HashMap::new());
        self.reset_estimated_scroll_velocity();
        self.drain_transactions(window, cx);
        self.flush_deferred_geometry_if_possible(window, cx);
        self.update_mounted_items(HashSet::new(), window, cx);
        self.notify_scroll_metrics_changed(window, cx);
        if let Some(generation) = self.viewport_resize_terminal_generation {
            self.try_complete_terminal_viewport_resize_session(generation, window, cx);
        }
        self.schedule_top_sticky_flash_end(window, cx);
        self.settle_scroll_geometry_at_rest(window, cx);
        self.update_performance_trace_context(None);
        ChatPerformanceTrace::shared().end_operation(
            trace_token,
            1.0,
            HashMap::from([("reason".to_string(), "animation".to_string())]),
        );
    }

    fn has_usable_viewport(&self) -> bool {
        self.bounds.size.width > Pixels::ZERO && self.bounds.size.height > Pixels::ZERO
    }

    /// offset past which the newest message would leave the bottom edge
    /// none while the list is free to bounce or the layout is short enough that the top anchor
    /// owns the offset
    fn resolved_bottom_overscroll_limit_y(&self) -> Option<Pixels> {
        if RiseListJumpDebug::disables_bottom_clamp()
            || !self.prevents_bottom_overscroll
            || self.is_updating_scroll_geometry
            || !self.has_usable_viewport()
            || self.layout_items.is_empty()
        {
            return None;
        }
        let top_aligned_offset_y = self.layout_origin_y - self.viewport_insets.top;
        let bottom_aligned_offset_y =
            self.layout_origin_y + self.layout_height - self.bounds.size.height + self.viewport_insets.bottom;
        if top_aligned_offset_y > bottom_aligned_offset_y {
            return None;
        }
        Some(bottom_aligned_offset_y)
    }

    fn clamp_bottom_overscroll_if_needed(&mut self, cx: &mut App) {
        let Some(maximum_offset_y) = self.resolved_bottom_overscroll_limit_y() else {
            return;
        };
        let offset_y = self.scroll_view.content_offset.y;
        if offset_y <= maximum_offset_y {
            return;
        }
        let message = format!(
            "OVERSCROLL-CLAMP offsetY={}->{} delta={} tracking={} decelerating={} originY={} layoutH={} insetBottom={}",
            RiseListJumpDebug::f(offset_y),
            RiseListJumpDebug::f(maximum_offset_y),
            RiseListJumpDebug::f(maximum_offset_y - offset_y),
            self.scroll_view.is_tracking,
            self.scroll_view.is_decelerating,
            RiseListJumpDebug::f(self.layout_origin_y),
            RiseListJumpDebug::f(self.layout_height),
            RiseListJumpDebug::f(self.viewport_insets.bottom),
        );
        RiseListJumpDebug::log(&message, cx);
        self.is_updating_scroll_geometry = true;
        self.scroll_view.set_content_offset(point(Pixels::ZERO, maximum_offset_y));
        self.is_updating_scroll_geometry = false;
    }

    fn has_active_viewport_resize_session(&self) -> bool {
        self.viewport_resize_animation.is_some()
            || self.viewport_resize_terminal_generation.is_some()
            || self.viewport_resize_gesture_ownership_generation.is_some()
            || self.viewport_anchor_animation_is_active
            || self.viewport_clip_animation_is_active
    }

    fn is_user_scrolling(&self) -> bool {
        #[cfg(debug_assertions)]
        if let Some(user_scrolling_override_for_testing) = self.user_scrolling_override_for_testing {
            return user_scrolling_override_for_testing;
        }
        self.is_programmatic_scroll_animating
            || (!self.programmatic_navigation_owns_scroll
                && (self.scroll_view.is_tracking || self.scroll_view.is_dragging || self.scroll_view.is_decelerating))
    }

    fn gesture_owns_viewport_geometry(&self) -> bool {
        self.is_user_scrolling() || self.viewport_resize_gesture_ownership_generation.is_some()
    }
}

#[cfg(debug_assertions)]
impl RiseManualListView {
    pub fn set_user_scrolling_override_for_testing(
        &mut self,
        is_scrolling: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_scrolling = self.is_user_scrolling();
        self.user_scrolling_override_for_testing = is_scrolling;
        if self.is_user_scrolling() {
            self.pause_deferred_mounting_for_active_scroll();
        } else {
            self.flush_deferred_geometry_if_possible(window, cx);
            if was_scrolling {
                self.reset_estimated_scroll_velocity();
                self.update_mounted_items(HashSet::new(), window, cx);
            }
        }
    }

    pub fn is_item_mounted_for_testing(&self, id: &str) -> bool {
        self.mounted_items.contains_key(id)
    }

    pub fn is_item_placeholder_for_testing(&self, id: &str) -> bool {
        self.mounted_items.get(id).is_some_and(|mounted| mounted.is_placeholder)
    }

    pub fn is_item_warm_mounted_for_testing(&self, id: &str) -> bool {
        self.mounted_items.get(id).is_some_and(|mounted| !mounted.is_placeholder)
    }

    pub fn mounted_view_for_testing(&self, id: &str) -> Option<AnyView> {
        self.mounted_items.get(id).map(|mounted| mounted.view.clone())
    }

    /// is user interaction enabled on a mounted row, for tests
    pub fn is_user_interaction_enabled_for_testing(&self, view: &AnyView) -> bool {
        !self.selection_interaction_disabled_ids.contains(&view.entity_id())
    }

    pub fn is_item_top_sticky_for_testing(&self, id: &str) -> bool {
        self.top_sticky_item_frames.contains_key(id) && self.mounted_items.contains_key(id)
    }

    pub fn mounted_frame_in_viewport_for_testing(&self, id: &str) -> Option<Bounds<Pixels>> {
        if !self.mounted_items.contains_key(id) {
            return None;
        }
        if let Some(frame) = self.top_sticky_item_frames.get(id) {
            let host_origin = self.top_sticky_header_host_view.frame.origin
                + self.viewport_clip_view.frame.origin
                + point(Pixels::ZERO, self.top_sticky_header_host_view.translation_y);
            return Some(Bounds::new(host_origin + frame.origin, frame.size));
        }
        let item = self.item_by_id.get(id)?.clone();
        let frame = self.mounted_frame(&item.borrow());
        let host_origin = self.viewport_clip_view.frame.origin + self.scroll_view.frame.origin
            - self.scroll_view.content_offset
            + self.item_host_view.frame.origin
            + point(Pixels::ZERO, self.item_host_view.translation_y);
        Some(Bounds::new(host_origin + frame.origin, frame.size))
    }

    pub fn drain_deferred_mounting_for_testing(
        &mut self,
        maximum_batches: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_user_scrolling() {
            return;
        }
        let mut remaining_batches = maximum_batches;
        while self.deferred_mount_needs_work
            && self.deferred_mount_queue_index < self.deferred_mount_queue.len()
            && remaining_batches > 0
        {
            self.deferred_mount_scheduled_generation = Some(self.deferred_mount_generation);
            self.perform_deferred_mount_batch(self.deferred_mount_generation, None, window, cx);
            remaining_batches -= 1;
        }
    }

    pub fn scheduled_deferred_mount_generation_for_testing(&self) -> Option<u64> {
        self.deferred_mount_scheduled_generation
    }

    pub fn perform_deferred_mount_batch_for_testing(
        &mut self,
        generation: u64,
        frame: Option<RiseManualListMountFrame>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.perform_deferred_mount_batch(generation, frame, window, cx);
    }

    pub fn has_deferred_mount_display_link_for_testing(&self) -> bool {
        self.deferred_mount_display_link.is_some()
    }

    pub fn set_active_scroll_content_offset_y_for_testing(
        &mut self,
        value: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_user_scrolling() || self.is_applying_transaction {
            return false;
        }
        let metrics = self.scroll_metrics();
        let target_y = value.max(metrics.minimum_content_offset_y).min(metrics.maximum_content_offset_y);
        if (self.scroll_view.content_offset.y - target_y).abs() <= px(0.5) {
            return true;
        }
        self.scroll_view.set_content_offset(point(Pixels::ZERO, target_y));
        self.scroll_view_did_scroll(window, cx);
        true
    }

    /// tracked and stored reusable view counts, for tests
    pub fn reusable_view_counts_for_testing(&self) -> (usize, usize) {
        (
            self.reusable_view_total_count,
            self.reusable_views_by_identifier.values().map(Vec::len).sum(),
        )
    }

    pub fn deferred_recycle_counts_for_testing(&self) -> (usize, usize) {
        (self.detached_identity_items.len(), self.deferred_recycle_items.len())
    }

    #[cfg(debug_assertions)]
    pub fn last_viewport_anchor_animation_offset_for_testing(&self) -> Option<Pixels> {
        self.last_viewport_anchor_animation_offset_for_testing
    }

    #[cfg(debug_assertions)]
    pub fn last_viewport_clip_animation_start_height_for_testing(&self) -> Option<Pixels> {
        self.last_viewport_clip_animation_start_height_for_testing
    }

    #[cfg(debug_assertions)]
    pub fn keyboard_content_translation_for_testing(&self) -> Pixels {
        self.keyboard_content_translation_for_testing
    }

    #[cfg(debug_assertions)]
    pub fn keyboard_sticky_translation_for_testing(&self) -> Pixels {
        self.keyboard_sticky_translation_for_testing
    }

    #[cfg(debug_assertions)]
    pub fn visible_item_range_computation_count_for_testing(&self) -> usize {
        self.visible_item_range_computation_count_for_testing
    }

    #[cfg(debug_assertions)]
    pub fn visible_synchronization_pass_count_for_testing(&self) -> usize {
        self.visible_synchronization_pass_count_for_testing
    }
}

impl RiseManualListView {
    fn mount_motion_class(&self) -> RiseManualListViewMountMotionClass {
        if !self.is_user_scrolling() {
            return RiseManualListViewMountMotionClass::Idle;
        }
        if f32::from(self.effective_estimated_scroll_velocity_y()).abs()
            >= RiseManualListMountEnvelopePolicy::FAST_SCROLL_VELOCITY_THRESHOLD
        {
            return RiseManualListViewMountMotionClass::Fast;
        }
        RiseManualListViewMountMotionClass::Active
    }

    fn effective_estimated_scroll_velocity_y(&self) -> Pixels {
        match self.last_scroll_sample_time {
            Some(last_scroll_sample_time) if current_media_time() - last_scroll_sample_time <= 0.12 => {
                self.estimated_scroll_velocity_y
            }
            _ => Pixels::ZERO,
        }
    }

    fn update_estimated_scroll_velocity(&mut self) {
        let now = current_media_time();
        let offset_y = self.scroll_view.content_offset.y;
        let previous = self.last_scroll_sample_offset_y.zip(self.last_scroll_sample_time);
        self.last_scroll_sample_offset_y = Some(offset_y);
        self.last_scroll_sample_time = Some(now);
        let Some((previous_offset_y, previous_time)) = previous else {
            return;
        };
        let elapsed = now - previous_time;
        if !(1.0 / 240.0..=0.15).contains(&elapsed) {
            return;
        }
        let instantaneous = f32::from(offset_y - previous_offset_y) / elapsed as f32;
        if !instantaneous.is_finite() {
            return;
        }
        let estimated = f32::from(self.estimated_scroll_velocity_y);
        self.estimated_scroll_velocity_y = if estimated == 0.0 || instantaneous.signum() != estimated.signum() {
            px(instantaneous)
        } else {
            px(instantaneous * 0.68 + estimated * 0.32)
        };
    }

    fn reset_estimated_scroll_velocity(&mut self) {
        self.estimated_scroll_velocity_y = Pixels::ZERO;
        self.last_scroll_sample_offset_y = None;
        self.last_scroll_sample_time = None;
    }

    fn drain_transactions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_usable_viewport() || self.is_applying_transaction {
            return;
        }
        if self.is_user_scrolling()
            && !self
                .pending_transaction
                .as_ref()
                .is_some_and(|pending| pending.value.allows_application_during_scroll)
        {
            return;
        }

        let trace_token = ChatPerformanceTrace::shared().begin_operation("transaction.drain", HashMap::new());
        self.is_applying_transaction = true;
        let mut completions: Vec<RiseManualListTransactionCallback> = Vec::new();
        while let Some(pending) = self.pending_transaction.take() {
            if self.is_user_scrolling() && !pending.value.allows_application_during_scroll {
                self.pending_transaction = Some(pending);
                break;
            }
            if let Some(expected_width) = pending.value.expected_width
                && (expected_width - f32::from(self.bounds.size.width)).abs() > 0.5
            {
                if let Some(discard) = pending.discard {
                    discard(window, cx);
                }
                continue;
            }
            if !self.apply(&pending.value, window, cx) {
                self.pending_transaction = Some(pending);
                break;
            }
            if let Some(completion) = pending.completion {
                completions.push(completion);
            }
        }
        self.is_applying_transaction = false;
        for completion in completions {
            completion(window, cx);
        }
        'tail: {
            if self.pending_transaction.is_some() && !self.is_user_scrolling() {
                self.drain_transactions(window, cx);
                break 'tail;
            }
            if !self.is_user_scrolling() {
                self.flush_deferred_geometry_if_possible(window, cx);
            }
            self.notify_scroll_metrics_changed(window, cx);
        }
        ChatPerformanceTrace::shared().end_operation(
            trace_token,
            1.0,
            HashMap::from([
                (
                    "pending".to_string(),
                    if self.pending_transaction.is_none() { "0" } else { "1" }.to_string(),
                ),
                ("items".to_string(), self.layout_items.count().to_string()),
            ]),
        );
    }

    fn apply(&mut self, transaction: &RiseManualListTransaction, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(animation) = transaction.animation.as_ref()
            && !animation.removed_ids.is_empty()
            && RiseListRemovalDiag::is_active(cx)
        {
            let line = format!(
                "capture bounds={} offset={} hostY={} mounted={} removed={} dust={}",
                RiseListRemovalDiag::f_rect(Some(self.bounds)),
                RiseListRemovalDiag::f(Some(self.scroll_view.content_offset.y)),
                RiseListRemovalDiag::f(Some(self.item_host_view.frame.origin.y)),
                self.mounted_items.len(),
                animation.removed_ids.len(),
                animation.dust_ids.len()
            );
            RiseListRemovalDiag::log(|| line, false, cx);
        }
        let mut transition_state = transaction.animation.as_ref().map(|animation| {
            self.capture_update_transition_state(
                &animation.content_ids,
                &animation.removed_ids,
                &animation.dust_ids,
                &animation.reveals_growth_ids,
            )
        });
        let jump_origin = transaction.jump_animation.map(|_| self.capture_jump_origin(cx));
        if let Some(before_layout) = transaction.before_layout.clone() {
            before_layout(self, window, cx);
        }
        if let Some(edge_mutation) = transaction.edge_mutation.as_ref() {
            match self.apply_edge_mutation(edge_mutation, transaction, transition_state, window, cx) {
                Ok(()) => {
                    self.finish_jump_transition(jump_origin, transaction, window, cx);
                    self.notify_applied_transaction(window, cx);
                    return true;
                }
                Err(returned_state) => transition_state = returned_state,
            }
        }
        if transaction.allows_application_during_scroll && self.is_user_scrolling() {
            return false;
        }
        self.apply_full_transaction(transaction, transition_state, window, cx);
        self.finish_jump_transition(jump_origin, transaction, window, cx);
        self.notify_applied_transaction(window, cx);
        true
    }

    fn notify_applied_transaction(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        self.awaits_render_after_transaction = true;
        if self.is_render_after_transaction_check_scheduled {
            return;
        }
        self.is_render_after_transaction_check_scheduled = true;
        cx.on_next_frame(window, |this, _, cx| {
            this.is_render_after_transaction_check_scheduled = false;
            if this.awaits_render_after_transaction {
                tracing::warn!(target: "riseonly::X2257", "applied transaction re-notified after a draw");
                cx.notify();
            }
        });
    }

    fn apply_full_transaction(
        &mut self,
        transaction: &RiseManualListTransaction,
        transition_state: Option<RiseManualListViewUpdateTransitionState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "transaction.full",
            HashMap::from([
                ("items".to_string(), transaction.ids.len().to_string()),
                (
                    "updated".to_string(),
                    transaction.updated_ids.as_ref().map_or(transaction.ids.len(), HashSet::len).to_string(),
                ),
            ]),
        );
        let unique_ids: HashSet<String> = transaction.ids.iter().cloned().collect();

        let preserved_anchor = self.visible_anchor_retained_by(&unique_ids);
        let width = self.bounds.size.width;
        let width_changed = self
            .layout_items
            .first()
            .is_some_and(|first| (first.borrow().frame.size.width - width).abs() > px(0.5));
        let mut next_items: Vec<RiseManualListViewLayoutItemRef> = Vec::with_capacity(transaction.ids.len());
        let mut next_items_by_id: HashMap<String, RiseManualListViewLayoutItemRef> =
            HashMap::with_capacity(transaction.ids.len());
        let mut reconfigure_ids: HashSet<String> =
            HashSet::with_capacity(transaction.updated_ids.as_ref().map_or(0, HashSet::len));

        let mut debug_height_changes: Vec<String> = Vec::new();
        let mut next_y = Pixels::ZERO;
        for id in &transaction.ids {
            let previous_item = self.item_by_id.get(id).cloned();
            let should_update = width_changed
                || previous_item.is_none()
                || transaction.updated_ids.as_ref().is_none_or(|updated_ids| updated_ids.contains(id));
            let content = match previous_item.as_ref() {
                Some(previous_item) if !should_update => previous_item.borrow().content.clone(),
                _ => self.validated_content((transaction.provider)(id, f32::from(width), window, cx)),
            };
            if RiseListJumpDebug::is_enabled()
                && let Some(previous_item) = previous_item.as_ref()
            {
                let previous_height = previous_item.borrow().content.height;
                if (previous_height - content.height).abs() > 0.01 {
                    debug_height_changes.push(format!(
                        "{} {}->{}",
                        id,
                        RiseListJumpDebug::f(previous_height),
                        RiseListJumpDebug::f(content.height),
                    ));
                }
            }
            if !next_items.is_empty() {
                next_y += self.inter_item_spacing;
            }
            if let Some(previous_item) = previous_item.as_ref() {
                let previous_item = previous_item.borrow();
                if width_changed
                    || previous_item.content.content_version != content.content_version
                    || previous_item.content.reuse_identifier != content.reuse_identifier
                {
                    reconfigure_ids.insert(id.clone());
                }
            }
            let height = px(content.height);
            let item = RiseManualListViewLayoutItem::new(
                id.clone(),
                content,
                Bounds::new(point(Pixels::ZERO, next_y), size(width, height)),
            );
            next_items.push(item.clone());
            next_items_by_id.insert(id.clone(), item);
            next_y += height;
        }

        let current_offset_y = self.scroll_view.content_offset.y;
        let previous_ids: HashSet<String> = self.item_by_id.keys().cloned().collect();
        let mut factory_refresh_ids: HashSet<String>;
        if let Some(updated_ids) = transaction.updated_ids.as_ref() {
            factory_refresh_ids = previous_ids.symmetric_difference(&unique_ids).cloned().collect();
            factory_refresh_ids.extend(updated_ids.iter().cloned());
            self.invalidate_identity_cached_items(&factory_refresh_ids);
        } else {
            factory_refresh_ids = previous_ids.union(&unique_ids).cloned().collect();
            self.invalidate_identity_cached_factories(None);
        }
        factory_refresh_ids.extend(reconfigure_ids.iter().cloned());
        let debug_previous_origin_y = self.layout_origin_y;
        self.layout_items = RiseManualListViewLayoutDeque::from_items(next_items);
        self.item_by_id = next_items_by_id;
        self.layout_coordinate_offset_y = Pixels::ZERO;
        self.layout_height = next_y;
        self.layout_origin_y = self.resolved_origin_y(&transaction.anchor, current_offset_y, preserved_anchor.as_ref());
        if RiseListJumpDebug::is_enabled() && !debug_height_changes.is_empty() {
            let message = format!(
                "TX-FULL scrolling={} anchor={:?} anchorId={} originY={}->{} heights[{}]={}",
                self.is_user_scrolling(),
                transaction.anchor,
                preserved_anchor.as_ref().map_or("nil", |anchor| anchor.id.as_str()),
                RiseListJumpDebug::f(debug_previous_origin_y),
                RiseListJumpDebug::f(self.layout_origin_y),
                debug_height_changes.len(),
                debug_height_changes.iter().take(6).cloned().collect::<Vec<_>>().join(", "),
            );
            RiseListJumpDebug::log(&message, cx);
        }
        self.finish_applied_layout(
            reconfigure_ids,
            transition_state,
            transaction.animation.as_ref(),
            factory_refresh_ids,
            window,
            cx,
        );
        ChatPerformanceTrace::shared().end_operation(trace_token, 1.0, HashMap::new());
    }

    fn apply_edge_mutation(
        &mut self,
        mutation: &RiseManualListTransactionEdgeMutation,
        transaction: &RiseManualListTransaction,
        transition_state: Option<RiseManualListViewUpdateTransitionState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Option<RiseManualListViewUpdateTransitionState>> {
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "transaction.edge",
            HashMap::from([
                ("old_items".to_string(), self.layout_items.count().to_string()),
                ("new_items".to_string(), transaction.ids.len().to_string()),
                (
                    "updated".to_string(),
                    transaction.updated_ids.as_ref().map_or(0, HashSet::len).to_string(),
                ),
                ("insert_prefix".to_string(), mutation.inserted_prefix_count.to_string()),
                ("insert_suffix".to_string(), mutation.inserted_suffix_count.to_string()),
                ("remove_prefix".to_string(), mutation.removed_prefix_count.to_string()),
                ("remove_suffix".to_string(), mutation.removed_suffix_count.to_string()),
            ]),
        );
        let result: Result<(), Option<RiseManualListViewUpdateTransitionState>> = 'body: {
            let old_count = self.layout_items.count();
            let new_count = transaction.ids.len();
            let width = self.bounds.size.width;
            if self.pending_item_relayout
                || self.applied_inter_item_spacing != self.inter_item_spacing
                || !self
                    .layout_items
                    .first()
                    .is_none_or(|first| (first.borrow().frame.size.width - width).abs() <= px(0.5))
                || mutation.removed_prefix_count + mutation.removed_suffix_count > old_count
                || mutation.inserted_prefix_count + mutation.inserted_suffix_count > new_count
            {
                break 'body Err(transition_state);
            }

            let retained_count = old_count - mutation.removed_prefix_count - mutation.removed_suffix_count;
            let new_retained_count = new_count - mutation.inserted_prefix_count - mutation.inserted_suffix_count;
            if retained_count != new_retained_count {
                break 'body Err(transition_state);
            }

            if retained_count > 0 {
                let old_first = self.layout_items[mutation.removed_prefix_count].borrow().id.clone();
                let old_last = self.layout_items[mutation.removed_prefix_count + retained_count - 1].borrow().id.clone();
                let new_first = &transaction.ids[mutation.inserted_prefix_count];
                let new_last = &transaction.ids[mutation.inserted_prefix_count + retained_count - 1];
                if &old_first != new_first || &old_last != new_last {
                    break 'body Err(transition_state);
                }
            }

            let updated_ids: HashSet<String> = if let Some(transaction_updated_ids) = transaction.updated_ids.as_ref() {
                transaction_updated_ids.clone()
            } else if retained_count == 0 {
                HashSet::new()
            } else {
                break 'body Err(transition_state);
            };

            let mut removed_ids: HashSet<String> =
                HashSet::with_capacity(mutation.removed_prefix_count + mutation.removed_suffix_count);
            for index in 0..mutation.removed_prefix_count {
                removed_ids.insert(self.layout_items[index].borrow().id.clone());
            }
            let suffix_start = old_count - mutation.removed_suffix_count;
            for index in suffix_start..old_count {
                removed_ids.insert(self.layout_items[index].borrow().id.clone());
            }

            let inserted_prefix_ids: Vec<String> =
                transaction.ids.iter().take(mutation.inserted_prefix_count).cloned().collect();
            let inserted_suffix_ids: Vec<String> =
                transaction.ids[new_count - mutation.inserted_suffix_count..].to_vec();
            let mut inserted_ids: HashSet<String> =
                HashSet::with_capacity(inserted_prefix_ids.len() + inserted_suffix_ids.len());
            for id in inserted_prefix_ids.iter().chain(inserted_suffix_ids.iter()) {
                if !(!self.item_by_id.contains_key(id) || removed_ids.contains(id)) || !inserted_ids.insert(id.clone()) {
                    break 'body Err(transition_state);
                }
            }

            if transaction.allows_application_during_scroll && self.is_user_scrolling() {
                let live_visible_range = self.current_visible_item_range();
                if live_visible_range.is_empty() {
                    break 'body Err(transition_state);
                }
                for index in live_visible_range {
                    let id = self.layout_items[index].borrow().id.clone();
                    if removed_ids.contains(&id) && !transaction.scroll_removable_ids.contains(&id) {
                        break 'body Err(transition_state);
                    }
                }
                if self.visible_anchor_excluding(&removed_ids).is_none() {
                    break 'body Err(transition_state);
                }
            }

            let preserved_anchor = self.visible_anchor_excluding(&removed_ids);
            let mut staged_updates: Vec<(RiseManualListViewLayoutItemRef, RiseManualListItem)> =
                Vec::with_capacity(updated_ids.len());
            let mut reconfigure_ids: HashSet<String> = HashSet::with_capacity(updated_ids.len());
            let mut debug_height_changes: Vec<String> = Vec::new();
            for id in &updated_ids {
                let Some(item) = self.item_by_id.get(id).cloned() else {
                    continue;
                };
                if removed_ids.contains(id) {
                    continue;
                }
                let content = self.validated_content((transaction.provider)(id, f32::from(width), window, cx));
                {
                    let current = item.borrow();
                    if RiseListJumpDebug::is_enabled() && (current.content.height - content.height).abs() > 0.01 {
                        debug_height_changes.push(format!(
                            "{} {}->{}",
                            id,
                            RiseListJumpDebug::f(current.content.height),
                            RiseListJumpDebug::f(content.height),
                        ));
                    }
                    if content.content_version != current.content.content_version
                        || content.reuse_identifier != current.content.reuse_identifier
                    {
                        reconfigure_ids.insert(id.clone());
                    }
                }
                staged_updates.push((item, content));
            }

            let prefix_items: Vec<RiseManualListViewLayoutItemRef> = inserted_prefix_ids
                .iter()
                .map(|id| {
                    let content = self.validated_content((transaction.provider)(id, f32::from(width), window, cx));
                    RiseManualListViewLayoutItem::new(id.clone(), content, Bounds::default())
                })
                .collect();
            let suffix_items: Vec<RiseManualListViewLayoutItemRef> = inserted_suffix_ids
                .iter()
                .map(|id| {
                    let content = self.validated_content((transaction.provider)(id, f32::from(width), window, cx));
                    RiseManualListViewLayoutItem::new(id.clone(), content, Bounds::default())
                })
                .collect();

            for _ in 0..mutation.removed_prefix_count {
                if let Some(item) = self.layout_items.remove_first() {
                    self.item_by_id.remove(&item.borrow().id);
                }
            }
            for _ in 0..mutation.removed_suffix_count {
                if let Some(item) = self.layout_items.remove_last() {
                    self.item_by_id.remove(&item.borrow().id);
                }
            }
            let retained_geometry_changed =
                staged_updates.iter().any(|(item, content)| item.borrow().content.height != content.height);
            for (item, content) in staged_updates {
                item.borrow_mut().content = content;
            }

            if self.layout_items.is_empty() {
                let items: Vec<RiseManualListViewLayoutItemRef> =
                    prefix_items.into_iter().chain(suffix_items).collect();
                let mut next_y = Pixels::ZERO;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        next_y += self.inter_item_spacing;
                    }
                    let mut layout_item = item.borrow_mut();
                    let height = px(layout_item.content.height);
                    layout_item.frame = Bounds::new(point(Pixels::ZERO, next_y), size(width, height));
                    next_y += height;
                    self.item_by_id.insert(layout_item.id.clone(), item.clone());
                }
                self.layout_items = RiseManualListViewLayoutDeque::from_items(items);
            } else {
                if !prefix_items.is_empty() {
                    let prefix_height: Pixels = prefix_items.iter().map(|item| px(item.borrow().content.height)).sum();
                    let mut next_y = self.layout_items.first().map_or(Pixels::ZERO, |first| first.borrow().frame.origin.y)
                        - prefix_height
                        - self.inter_item_spacing * prefix_items.len() as f32;
                    for item in &prefix_items {
                        let mut layout_item = item.borrow_mut();
                        let height = px(layout_item.content.height);
                        layout_item.frame = Bounds::new(point(Pixels::ZERO, next_y), size(width, height));
                        next_y += height + self.inter_item_spacing;
                        self.item_by_id.insert(layout_item.id.clone(), item.clone());
                    }
                    self.layout_items.prepend(prefix_items);
                }
                if !suffix_items.is_empty() {
                    let mut next_y = self.layout_items.last().map_or(Pixels::ZERO, |last| last.borrow().frame.bottom())
                        + self.inter_item_spacing;
                    for item in &suffix_items {
                        let mut layout_item = item.borrow_mut();
                        let height = px(layout_item.content.height);
                        layout_item.frame = Bounds::new(point(Pixels::ZERO, next_y), size(width, height));
                        next_y += height + self.inter_item_spacing;
                        self.item_by_id.insert(layout_item.id.clone(), item.clone());
                    }
                    self.layout_items.append(suffix_items);
                }
            }

            if retained_geometry_changed {
                let mut next_y = Pixels::ZERO;
                for (index, item) in self.layout_items.iter().enumerate() {
                    if index > 0 {
                        next_y += self.inter_item_spacing;
                    }
                    let mut layout_item = item.borrow_mut();
                    let height = px(layout_item.content.height);
                    layout_item.frame = Bounds::new(point(Pixels::ZERO, next_y), size(width, height));
                    next_y += height;
                }
                self.layout_coordinate_offset_y = Pixels::ZERO;
                self.layout_height = next_y;
            } else {
                let first_y = self.layout_items.first().map_or(Pixels::ZERO, |first| first.borrow().frame.origin.y);
                self.layout_coordinate_offset_y = -first_y;
                self.layout_height =
                    self.layout_items.last().map_or(first_y, |last| last.borrow().frame.bottom()) - first_y;
            }
            assert_eq!(self.layout_items.count(), transaction.ids.len());
            if let Some(first) = self.layout_items.first() {
                assert_eq!(Some(&first.borrow().id), transaction.ids.first());
            }
            if let Some(last) = self.layout_items.last() {
                assert_eq!(Some(&last.borrow().id), transaction.ids.last());
            }

            let current_offset_y = self.scroll_view.content_offset.y;
            let debug_previous_origin_y = self.layout_origin_y;
            self.layout_origin_y = self.resolved_origin_y(&transaction.anchor, current_offset_y, preserved_anchor.as_ref());
            if RiseListJumpDebug::is_enabled()
                && (!debug_height_changes.is_empty()
                    || mutation.inserted_prefix_count
                        + mutation.inserted_suffix_count
                        + mutation.removed_prefix_count
                        + mutation.removed_suffix_count
                        > 0)
            {
                let message = format!(
                    "TX-EDGE scrolling={} ins={}/{} rem={}/{} anchorId={} originY={}->{} heights[{}]={}",
                    self.is_user_scrolling(),
                    mutation.inserted_prefix_count,
                    mutation.inserted_suffix_count,
                    mutation.removed_prefix_count,
                    mutation.removed_suffix_count,
                    preserved_anchor.as_ref().map_or("nil", |anchor| anchor.id.as_str()),
                    RiseListJumpDebug::f(debug_previous_origin_y),
                    RiseListJumpDebug::f(self.layout_origin_y),
                    debug_height_changes.len(),
                    debug_height_changes.iter().take(6).cloned().collect::<Vec<_>>().join(", "),
                );
                RiseListJumpDebug::log(&message, cx);
            }
            let mut factory_refresh_ids = removed_ids;
            factory_refresh_ids.extend(inserted_ids);
            factory_refresh_ids.extend(updated_ids);
            factory_refresh_ids.extend(reconfigure_ids.iter().cloned());
            self.invalidate_identity_cached_items(&factory_refresh_ids);
            self.finish_applied_layout(
                reconfigure_ids,
                transition_state,
                transaction.animation.as_ref(),
                factory_refresh_ids,
                window,
                cx,
            );
            Ok(())
        };
        ChatPerformanceTrace::shared().end_operation(
            trace_token,
            1.0,
            HashMap::from([(
                "applied".to_string(),
                if result.is_ok() { "1" } else { "0" }.to_string(),
            )]),
        );
        result
    }

    fn validated_content(&self, content: RiseManualListItem) -> RiseManualListItem {
        assert!(
            content.height.is_finite() && content.height > 0.0,
            "Manual list item heights must be finite and greater than zero"
        );
        assert!(!content.reuse_identifier.is_empty(), "Manual list reuse identifiers must not be empty");
        if let Some(header) = content.bottom_sticky_header.as_ref() {
            assert!(!header.id.is_empty(), "Sticky header ids must not be empty");
            assert!(!header.reuse_identifier.is_empty(), "Sticky header reuse identifiers must not be empty");
            let width = f32::from(header.size.width);
            let height = f32::from(header.size.height);
            assert!(
                width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0,
                "Sticky header sizes must be finite and greater than zero"
            );
        }
        content
    }

    fn resolved_origin_y(
        &self,
        anchor: &RiseManualListTransactionAnchor,
        current_offset_y: Pixels,
        preserved_anchor: Option<&RiseManualListViewVisibleAnchor>,
    ) -> Pixels {
        match anchor {
            RiseManualListTransactionAnchor::PreserveVisible => {
                if let Some(preserved_anchor) = preserved_anchor
                    && let Some(item) = self.item_by_id.get(&preserved_anchor.id)
                {
                    return preserved_anchor.absolute_min_y - self.resolved_frame(&item.borrow()).origin.y;
                }
                current_offset_y
            }
            RiseManualListTransactionAnchor::Top => current_offset_y + self.viewport_insets.top,
            RiseManualListTransactionAnchor::Bottom => {
                current_offset_y + self.bounds.size.height - self.landing_bottom_inset() - self.layout_height
            }
            RiseManualListTransactionAnchor::Item { id, viewport_offset } => {
                if let Some(item) = self.item_by_id.get(id) {
                    return current_offset_y + px(*viewport_offset) - self.resolved_frame(&item.borrow()).origin.y;
                }
                current_offset_y
            }
        }
    }

    fn finish_applied_layout(
        &mut self,
        reconfigure_ids: HashSet<String>,
        transition_state: Option<RiseManualListViewUpdateTransitionState>,
        animation: Option<&RiseManualListTransactionUpdateAnimation>,
        identity_cache_exclusion_ids: HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_identity_cache_exclusion_ids = self.identity_cache_exclusion_ids.clone();
        self.identity_cache_exclusion_ids.extend(identity_cache_exclusion_ids);
        self.applied_inter_item_spacing = self.inter_item_spacing;
        self.layout_generation = self.layout_generation.wrapping_add(1);
        self.cancel_deferred_mounting();
        self.pending_geometry_refresh = false;
        self.pending_geometry_keeps_bottom_anchor = false;
        self.pending_geometry_keeps_top_anchor = false;
        self.pending_item_relayout = false;
        self.pending_geometry_allows_application_during_scroll = false;

        let diag_offset_before = self.scroll_view.content_offset.y;
        self.perform_without_animations(|this| {
            this.update_host_frame();
            this.update_scroll_geometry();
        });
        if animation.is_some_and(|animation| !animation.removed_ids.is_empty()) && RiseListRemovalDiag::is_active(cx) {
            let line = format!(
                "applied offset {}->{} transition={}",
                RiseListRemovalDiag::f(Some(diag_offset_before)),
                RiseListRemovalDiag::f(Some(self.scroll_view.content_offset.y)),
                transition_state.is_some()
            );
            RiseListRemovalDiag::log(|| line, false, cx);
        }
        self.update_mounted_items(reconfigure_ids, window, cx);
        self.update_sticky_headers(window, cx);
        if (1..=3).contains(&self.layout_items.count()) {
            let rows: Vec<String> = self
                .mounted_items
                .iter()
                .map(|(id, mounted)| {
                    format!(
                        "{id}@{:.0},{:.0} {:.0}x{:.0} item_host={}",
                        f32::from(mounted.frame.origin.x),
                        f32::from(mounted.frame.origin.y),
                        f32::from(mounted.frame.size.width),
                        f32::from(mounted.frame.size.height),
                        mounted.host == RiseManualListViewUpdateTransitionHost::Item
                    )
                })
                .collect();
            tracing::warn!(
                target: "riseonly::X2257",
                items = self.layout_items.count(),
                bounds_w = f32::from(self.bounds.size.width),
                bounds_h = f32::from(self.bounds.size.height),
                offset_before = f32::from(diag_offset_before),
                offset = f32::from(self.scroll_view.content_offset.y),
                origin_y = f32::from(self.layout_origin_y),
                layout_h = f32::from(self.layout_height),
                inset_top = f32::from(self.viewport_insets.top),
                inset_bottom = f32::from(self.viewport_insets.bottom),
                host_y = f32::from(self.item_host_view.frame.origin.y),
                host_alpha = self.item_host_view.alpha,
                clip_y = f32::from(self.viewport_clip_view.frame.origin.y),
                clip_h = f32::from(self.viewport_clip_view.frame.size.height),
                transition = transition_state.is_some(),
                inserted = animation.map_or(0, |animation| animation.inserted_ids.len()),
                rows = ?rows,
                "applied short layout"
            );
        }
        #[cfg(debug_assertions)]
        if let Some(animation) = animation
            && !animation.inserted_ids.is_empty()
        {
            let mounted = animation.inserted_ids.iter().filter(|id| self.mounted_items.contains_key(*id)).count();
            tracing::debug!(
                "[RISE-KB][insert] inserted={} mounted={} transition={} retained={} offset={} inset={}",
                animation.inserted_ids.len(),
                mounted,
                transition_state.is_some(),
                transition_state.as_ref().map_or(-1, |state| state.items_by_id.len() as i64),
                f32::from(self.scroll_view.content_offset.y),
                f32::from(self.viewport_insets.bottom)
            );
        }
        if let (Some(transition_state), Some(animation)) = (transition_state, animation) {
            self.animate_update_transition(transition_state, animation, window, cx);
        }
        if let Some(on_scroll_metrics_changed) = self.on_scroll_metrics_changed.clone() {
            on_scroll_metrics_changed(&self.scroll_metrics(), window, cx);
        }
        self.identity_cache_exclusion_ids = previous_identity_cache_exclusion_ids;
    }
}

impl RiseManualListView {
    /// origin of the item host in the list own coordinates, as the model sees it
    fn item_host_origin_in_list(&self) -> Point<Pixels> {
        self.viewport_clip_view.frame.origin + self.scroll_view.frame.origin - self.scroll_view.content_offset
            + self.item_host_view.frame.origin
            + point(Pixels::ZERO, self.item_host_view.translation_y)
    }

    fn capture_update_transition_state(
        &self,
        content_ids: &HashSet<String>,
        removed_ids: &HashSet<String>,
        dust_ids: &HashSet<String>,
        reveal_ids: &HashSet<String>,
    ) -> RiseManualListViewUpdateTransitionState {
        let now = current_media_time();
        let item_host_origin = self.item_host_origin_in_list();
        let top_sticky_host_origin = self.viewport_clip_view.frame.origin + self.top_sticky_header_host_view.frame.origin;
        let bottom_sticky_host_origin =
            self.viewport_clip_view.frame.origin + self.bottom_sticky_header_host_view.frame.origin;
        let reduces_motion = reduce_motion();
        let renders_removal_dust = !dust_ids.is_empty() && !reduces_motion;
        let list_bounds = Bounds::new(Point::default(), self.bounds.size);

        let mut items_by_id: HashMap<String, RiseManualListViewUpdateTransitionItem> =
            HashMap::with_capacity(self.mounted_items.len());
        let mut removed_images_by_id: HashMap<String, RiseManualListViewRemovedRowImage> = HashMap::new();
        for (id, mounted) in &self.mounted_items {
            let (host_origin, local_frame) = if let Some(frame) = self.top_sticky_item_frames.get(id) {
                (top_sticky_host_origin, *frame)
            } else if let Some(item) = self.item_by_id.get(id) {
                (item_host_origin, self.mounted_frame(&item.borrow()))
            } else {
                continue;
            };
            let motion = self.update_transition_motions.get(&mounted.view.entity_id());
            let presented_offset = motion.map_or(Point::default(), |motion| motion.presentation_offset(now));
            let frame = Bounds::new(host_origin + local_frame.origin + presented_offset, local_frame.size);
            let snapshot = (content_ids.contains(id) || removed_ids.contains(id)).then(|| mounted.view.clone());
            if renders_removal_dust && dust_ids.contains(id) {
                let visible = frame.intersect(&list_bounds);
                if visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO {
                    let rect = Bounds::new(visible.origin - frame.origin, visible.size);
                    if let Some((image, scale)) = Self::render_row_image(&mounted.view, rect) {
                        removed_images_by_id
                            .insert(id.clone(), RiseManualListViewRemovedRowImage { image, scale, rect });
                    }
                }
            }
            let image = if reveal_ids.contains(id)
                && !reduces_motion
                && local_frame.size.width > Pixels::ZERO
                && local_frame.size.height > Pixels::ZERO
            {
                Self::render_row_image(&mounted.view, Bounds::new(Point::default(), local_frame.size))
                    .map(|(image, _)| image)
            } else {
                None
            };
            items_by_id.insert(
                id.clone(),
                RiseManualListViewUpdateTransitionItem {
                    view: mounted.view.clone(),
                    frame,
                    alpha: motion.map_or(1.0, |motion| motion.presentation_alpha(now)),
                    snapshot,
                    image,
                },
            );
        }

        let mut bottom_sticky_headers_by_id: HashMap<String, RiseManualListViewUpdateTransitionItem> =
            HashMap::with_capacity(self.mounted_bottom_sticky_headers.len());
        for (id, mounted) in &self.mounted_bottom_sticky_headers {
            let motion = self.update_transition_motions.get(&mounted.view.entity_id());
            let presented_offset = motion.map_or(Point::default(), |motion| motion.presentation_offset(now));
            let local_frame = mounted.frame;
            bottom_sticky_headers_by_id.insert(
                id.clone(),
                RiseManualListViewUpdateTransitionItem {
                    view: mounted.view.clone(),
                    frame: Bounds::new(
                        bottom_sticky_host_origin + local_frame.origin + presented_offset,
                        local_frame.size,
                    ),
                    alpha: motion.map_or(1.0, |motion| motion.presentation_alpha(now)),
                    snapshot: Some(mounted.view.clone()),
                    image: None,
                },
            );
        }
        RiseManualListViewUpdateTransitionState { items_by_id, bottom_sticky_headers_by_id, removed_images_by_id }
    }

    /// pixels of the row inside rect (row coordinates) and their scale
    fn render_row_image(_view: &AnyView, _rect: Bounds<Pixels>) -> Option<(Arc<RenderImage>, f32)> {
        None
    }

    fn animate_update_transition(
        &mut self,
        state: RiseManualListViewUpdateTransitionState,
        animation: &RiseManualListTransactionUpdateAnimation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reduces_motion = reduce_motion();
        let now = current_media_time();
        let duration = if reduces_motion { 0.15_f64.min(animation.duration) } else { animation.duration };
        let damping_ratio = if reduces_motion { 1.0 } else { animation.damping_ratio };
        let motion = |from_offset: Point<Pixels>, from_alpha: f32, to_alpha: f32| RiseManualListViewRowMotion {
            from_offset,
            from_alpha,
            to_alpha,
            to_scale: 1.0,
            started_at: now,
            duration,
            damping_ratio,
            alpha_fade: None,
            clip_height: None,
        };
        let item_host_origin = self.item_host_origin_in_list();
        let top_sticky_host_origin = self.viewport_clip_view.frame.origin + self.top_sticky_header_host_view.frame.origin;
        let bottom_sticky_host_origin =
            self.viewport_clip_view.frame.origin + self.bottom_sticky_header_host_view.frame.origin;

        let mut targets: Vec<(EntityId, RiseManualListViewRowMotion)> =
            Vec::with_capacity(self.mounted_items.len() + self.mounted_bottom_sticky_headers.len());
        let mut outgoing_snapshots: Vec<RiseManualListViewUpdateTransitionSnapshot> = Vec::with_capacity(
            animation.content_ids.len() + animation.removed_ids.len() + state.bottom_sticky_headers_by_id.len(),
        );
        let mut replaced_snapshots: Vec<RiseManualListViewUpdateTransitionSnapshot> = Vec::new();
        let mut dust_sources: Vec<RiseManualListRemovalDustSource> = Vec::new();
        let snapshot_motion = RiseManualListViewRowMotion {
            from_offset: Point::default(),
            from_alpha: 1.0,
            to_alpha: 0.0,
            to_scale: if reduces_motion { 1.0 } else { 0.985 },
            started_at: now,
            duration,
            damping_ratio,
            alpha_fade: None,
            clip_height: None,
        };

        let glides_bottom_resize = !animation.bottom_aligned_ids.is_empty()
            && animation.inserted_ids.is_empty()
            && animation.removed_ids.is_empty();
        let mut inserted_travel_ids: Vec<String> = Vec::new();
        let mut growth_footers: Vec<RiseManualListViewUpdateTransitionSnapshot> = Vec::new();

        for (id, mounted) in &self.mounted_items {
            let Some(previous) = state.items_by_id.get(id) else {
                continue;
            };
            let (host, host_origin, final_frame) = if let Some(frame) = self.top_sticky_item_frames.get(id) {
                (RiseManualListViewUpdateTransitionHost::TopSticky, top_sticky_host_origin, *frame)
            } else if let Some(item) = self.item_by_id.get(id) {
                (RiseManualListViewUpdateTransitionHost::Item, item_host_origin, self.mounted_frame(&item.borrow()))
            } else {
                continue;
            };
            let old_frame = Bounds::new(previous.frame.origin - host_origin, previous.frame.size);
            if old_frame.size.width <= Pixels::ZERO || old_frame.size.height <= Pixels::ZERO {
                continue;
            }
            let is_same_view = previous.view.entity_id() == mounted.view.entity_id();
            let glides_grown_row =
                glides_bottom_resize && is_same_view && animation.bottom_aligned_ids.contains(id);
            let mut from_offset = Point::default();
            if !reduces_motion {
                let aligns_bottom =
                    is_same_view && animation.bottom_aligned_ids.contains(id) && !glides_bottom_resize;
                let travel_y = if aligns_bottom {
                    old_frame.bottom() - final_frame.bottom()
                } else {
                    old_frame.top() - final_frame.top()
                };
                from_offset = point(old_frame.left() - final_frame.left(), travel_y);
            }

            let reveals_growth = animation.reveals_growth_ids.contains(id);
            let mut clip_height = None;
            if !reduces_motion
                && (reveals_growth || glides_grown_row || animation.clips_growth_ids.contains(id))
                && final_frame.size.height > old_frame.size.height + px(0.5)
            {
                let footer = if reveals_growth {
                    px(animation.reveals_growth_footer_height).min(old_frame.size.height)
                } else {
                    Pixels::ZERO
                };
                let clip_footer = if footer > px(0.5) { footer } else { Pixels::ZERO };
                let toggle_band = if clip_footer > px(0.5) && reveals_growth && is_same_view && previous.image.is_some() {
                    px(animation.reveals_growth_toggle_height)
                        .min((old_frame.size.height - clip_footer).max(Pixels::ZERO))
                } else {
                    Pixels::ZERO
                };
                clip_height = Some((
                    old_frame.size.height - clip_footer - toggle_band,
                    final_frame.size.height - clip_footer,
                ));
                if clip_footer > Pixels::ZERO {
                    growth_footers.push(RiseManualListViewUpdateTransitionSnapshot {
                        view: mounted.view.clone(),
                        frame: Bounds::new(
                            point(final_frame.left(), final_frame.bottom() - footer),
                            size(final_frame.size.width, footer),
                        ),
                        host,
                        motion: motion(
                            point(old_frame.left() - final_frame.left(), old_frame.bottom() - final_frame.bottom()),
                            1.0,
                            1.0,
                        ),
                        crop: Some(RiseManualListViewSnapshotCrop {
                            content_size: final_frame.size,
                            offset_y: final_frame.size.height - footer,
                        }),
                    });
                }
            }

            let mut from_alpha = 1.0;
            if is_same_view {
                from_alpha = previous.alpha;
            }
            let reshaped_in_place =
                is_same_view && !animation.removed_ids.is_empty() && animation.content_ids.contains(id);
            let is_replacement = (!is_same_view || reshaped_in_place) && previous.snapshot.is_some();
            let mut alpha_fade = None;
            if is_replacement && let Some(snapshot) = previous.snapshot.clone() {
                from_alpha = 0.0;
                let travel = if reduces_motion { Point::default() } else { final_frame.origin - old_frame.origin };
                replaced_snapshots.push(RiseManualListViewUpdateTransitionSnapshot {
                    view: snapshot,
                    frame: Bounds::new(old_frame.origin + travel, old_frame.size),
                    host,
                    motion: RiseManualListViewRowMotion {
                        from_offset: point(Pixels::ZERO - travel.x, Pixels::ZERO - travel.y),
                        from_alpha: 1.0,
                        to_alpha: 0.0,
                        to_scale: 1.0,
                        started_at: now,
                        duration,
                        damping_ratio,
                        alpha_fade: Some(RiseManualListViewAlphaFade {
                            duration: RiseManualListReplacementFade::OLD_LOOK_DURATION,
                            curve: RiseManualListViewAlphaFade::EASE_IN,
                        }),
                        clip_height: None,
                    },
                    crop: None,
                });
                alpha_fade = Some(RiseManualListViewAlphaFade {
                    duration: RiseManualListReplacementFade::NEW_LOOK_DURATION,
                    curve: RiseManualListViewAlphaFade::EASE_OUT,
                });
            }
            targets.push((
                mounted.view.entity_id(),
                RiseManualListViewRowMotion {
                    from_offset,
                    from_alpha,
                    to_alpha: 1.0,
                    to_scale: 1.0,
                    started_at: now,
                    duration,
                    damping_ratio,
                    alpha_fade,
                    clip_height,
                },
            ));
        }

        let mut insert_tallest = Pixels::ZERO;
        let mut insert_total = Pixels::ZERO;
        for id in &animation.inserted_ids {
            if state.items_by_id.contains_key(id) {
                continue;
            }
            if !self.mounted_items.contains_key(id)
                && let Some(sticky_item) = self.item_by_id.get(id).cloned()
            {
                let is_top_sticky = sticky_item.borrow().content.top_sticky_header.is_some();
                if is_top_sticky {
                    let item = sticky_item.borrow();
                    self.mount(&item, window, cx);
                }
            }
            if !self.mounted_items.contains_key(id) {
                continue;
            }
            if !self.pinned_top_sticky_item_ids.contains(id) {
                if self.top_sticky_item_frames.contains_key(id) {
                    self.restore_top_sticky_item(id, cx);
                }
                self.insert_travel_row_ids.insert(id.clone());
            }
            let height = self
                .item_by_id
                .get(id)
                .map_or(Pixels::ZERO, |item| self.mounted_frame(&item.borrow()).size.height);
            inserted_travel_ids.push(id.clone());
            insert_tallest = insert_tallest.max(height);
            insert_total += height;
        }
        let travel = if reduces_motion {
            Pixels::ZERO
        } else {
            px(24.0).max((self.bounds.size.height * 0.55).min((insert_tallest * 1.4).max(insert_total)))
        };
        for id in &inserted_travel_ids {
            let Some(mounted) = self.mounted_items.get(id) else {
                continue;
            };
            targets.push((mounted.view.entity_id(), motion(point(Pixels::ZERO, travel), 0.0, 1.0)));
        }

        for id in &animation.removed_ids {
            if self.mounted_items.contains_key(id) {
                continue;
            }
            let Some(previous) = state.items_by_id.get(id) else {
                continue;
            };
            let row_frame = Bounds::new(previous.frame.origin - item_host_origin, previous.frame.size);
            if !reduces_motion && let Some(removed) = state.removed_images_by_id.get(id) {
                dust_sources.push(RiseManualListRemovalDustSource {
                    image: removed.image.clone(),
                    scale: removed.scale,
                    frame: Bounds::new(row_frame.origin + removed.rect.origin, removed.rect.size),
                });
                continue;
            }
            let Some(snapshot) = previous.snapshot.clone() else {
                continue;
            };
            outgoing_snapshots.push(RiseManualListViewUpdateTransitionSnapshot {
                view: snapshot,
                frame: row_frame,
                host: RiseManualListViewUpdateTransitionHost::Item,
                motion: snapshot_motion,
                crop: None,
            });
        }

        for (id, mounted) in &self.mounted_bottom_sticky_headers {
            let final_frame = mounted.frame;
            if let Some(previous) = state.bottom_sticky_headers_by_id.get(id) {
                let old_frame = Bounds::new(previous.frame.origin - bottom_sticky_host_origin, previous.frame.size);
                if old_frame.size.width <= Pixels::ZERO || old_frame.size.height <= Pixels::ZERO {
                    continue;
                }
                let from_offset =
                    if reduces_motion { Point::default() } else { old_frame.origin - final_frame.origin };
                let mut from_alpha = 1.0;
                if previous.view.entity_id() == mounted.view.entity_id() {
                    from_alpha = previous.alpha;
                }
                if previous.view.entity_id() != mounted.view.entity_id()
                    && let Some(snapshot) = previous.snapshot.clone()
                {
                    from_alpha = 0.0;
                    outgoing_snapshots.push(RiseManualListViewUpdateTransitionSnapshot {
                        view: snapshot,
                        frame: old_frame,
                        host: RiseManualListViewUpdateTransitionHost::BottomSticky,
                        motion: snapshot_motion,
                        crop: None,
                    });
                }
                targets.push((mounted.view.entity_id(), motion(from_offset, from_alpha, 1.0)));
            } else {
                targets.push((mounted.view.entity_id(), motion(Point::default(), 0.0, 1.0)));
            }
        }

        for (id, previous) in &state.bottom_sticky_headers_by_id {
            if self.mounted_bottom_sticky_headers.contains_key(id) {
                continue;
            }
            let Some(snapshot) = previous.snapshot.clone() else {
                continue;
            };
            outgoing_snapshots.push(RiseManualListViewUpdateTransitionSnapshot {
                view: snapshot,
                frame: Bounds::new(previous.frame.origin - bottom_sticky_host_origin, previous.frame.size),
                host: RiseManualListViewUpdateTransitionHost::BottomSticky,
                motion: snapshot_motion,
                crop: None,
            });
        }

        if !dust_sources.is_empty() {
            RiseManualListRemovalDust::play(
                &mut self.removal_dust_host,
                &dust_sources,
                self.effective_layout_direction_is_left_to_right,
            );
        }
        if !animation.reveals_growth_ids.is_empty() {
            let offset_y = self.scroll_view.content_offset.y;
            let content_height = self.scroll_view.content_size.height;
            RiseListRemovalDiag::log(
                || {
                    format!(
                        "grow offset={} size={} rows={}",
                        RiseListRemovalDiag::f(Some(offset_y)),
                        RiseListRemovalDiag::f(Some(content_height)),
                        targets.len()
                    )
                },
                false,
                cx,
            );
        }
        if !animation.removed_ids.is_empty() && RiseListRemovalDiag::is_active(cx) {
            let offset_y = self.scroll_view.content_offset.y;
            RiseListRemovalDiag::log(
                || {
                    format!(
                        "animate targets={} outgoing={} replaced={} dust={} dustImages={} duration={} offset={} reduceMotion={}",
                        targets.len(),
                        outgoing_snapshots.len(),
                        replaced_snapshots.len(),
                        dust_sources.len(),
                        state.removed_images_by_id.len(),
                        animation.duration,
                        RiseListRemovalDiag::f(Some(offset_y)),
                        reduces_motion
                    )
                },
                false,
                cx,
            );
        }
        if targets.is_empty() && outgoing_snapshots.is_empty() && replaced_snapshots.is_empty() {
            return;
        }
        let close_delay = if dust_sources.is_empty() { 0.0 } else { RiseManualListRemovalDust::GAP_HOLD_SECONDS };
        self.insert_travel_completion_ids.extend(inserted_travel_ids);
        for (entity_id, mut motion) in targets {
            motion.started_at += close_delay;
            self.update_transition_motions.insert(entity_id, motion);
        }
        for mut snapshot in outgoing_snapshots.into_iter().chain(replaced_snapshots).chain(growth_footers) {
            snapshot.motion.started_at += close_delay;
            self.update_transition_snapshots.push(snapshot);
        }
        let lifetime = close_delay + duration.max(RiseManualListReplacementFade::OLD_LOOK_DURATION);
        self.update_transition_generation = self.update_transition_generation.wrapping_add(1);
        let generation = self.update_transition_generation;
        self.update_transition_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(lifetime.max(0.0))).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.update_transition_generation == generation {
                    this.update_transition_snapshots.clear();
                    this.update_transition_motions.clear();
                    this.update_transition_task = None;
                }
                let held_insert_ids = std::mem::take(&mut this.insert_travel_completion_ids);
                if !held_insert_ids.is_empty() {
                    for id in &held_insert_ids {
                        this.insert_travel_row_ids.remove(id);
                    }
                    this.update_top_sticky_headers(window, cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn update_host_frame(&mut self) {
        let width = self.layout_items.first().map_or(self.bounds.size.width, |first| first.borrow().frame.size.width);
        let host_frame = Bounds::new(point(Pixels::ZERO, self.layout_origin_y), size(width, self.layout_height));
        if self.item_host_view.frame != host_frame {
            self.item_host_view.frame = host_frame;
        }
        if self.selection_overlay_layer.frame != host_frame {
            self.selection_overlay_layer.frame = host_frame;
        }
        self.refresh_selection_overlay();
        self.reposition_jump_highlight();
    }

    /// content translation presented right now
    /// keyboard transition slides item_host_view, not the offset or frames
    /// sticky placement needs it or the date chip lags
    fn live_keyboard_content_translation_y(&self) -> Pixels {
        if !self.keyboard_viewport_transition_is_active {
            return Pixels::ZERO;
        }
        self.item_host_view.presentation_translation_y(current_media_time())
    }

    /// re-places the floating date on each frame of a keyboard transition
    fn start_keyboard_sticky_tracking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.keyboard_sticky_display_link.is_some() {
            return;
        }
        self.keyboard_sticky_display_link =
            Some(Self::add_display_link(window, cx, |this, _, window, cx| this.keyboard_sticky_tracking_did_tick(window, cx)));
    }

    fn stop_keyboard_sticky_tracking(&mut self) {
        self.keyboard_sticky_display_link = None;
    }

    fn keyboard_sticky_tracking_did_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.keyboard_viewport_transition_is_active {
            self.stop_keyboard_sticky_tracking();
            return;
        }
        self.update_top_sticky_headers(window, cx);
        self.update_bottom_sticky_headers(window, cx);
    }

    fn live_keyboard_sticky_bottom_inset(&self) -> Pixels {
        if !self.keyboard_viewport_transition_is_active {
            return self.bottom_sticky_viewport_inset.unwrap_or(self.viewport_insets.bottom);
        }
        self.keyboard_timing_reference_view.presentation_translation_y(current_media_time())
    }

    fn keyboard_visible_item_range(&mut self, content_translation_y: Pixels) -> Range<usize> {
        if content_translation_y.abs() <= px(0.01) {
            return self.current_visible_item_range();
        }
        let local_minimum_y = self.scroll_view.content_offset.y - content_translation_y - self.layout_origin_y;
        self.item_range(local_minimum_y, local_minimum_y + self.bounds.size.height)
    }

    fn current_keyboard_sticky_signature(&self) -> RiseManualListViewKeyboardStickySignature {
        RiseManualListViewKeyboardStickySignature {
            layout_generation: self.layout_generation,
            content_offset_y: self.scroll_view.content_offset.y,
            layout_origin_y: self.layout_origin_y,
            layout_coordinate_offset_y: self.layout_coordinate_offset_y,
            size: self.bounds.size,
            top_inset: self.top_sticky_viewport_inset.unwrap_or(self.viewport_insets.top),
            bottom_inset: self.viewport_insets.bottom,
            sticky_bottom_inset: self.bottom_sticky_viewport_inset,
        }
    }

    /// hands every sticky header of a keyboard transition to the render server
    /// returns whether headers now move on their own, else the caller tracks them per frame
    fn plan_keyboard_sticky_motion(&mut self, reason: &str) -> bool {
        if !self.keyboard_viewport_transition_is_active {
            return false;
        }
        self.remove_keyboard_sticky_plan_motion();
        #[cfg(debug_assertions)]
        tracing::debug!("[RISE-KB][list] sticky plan unavailable reason={reason}");
        #[cfg(not(debug_assertions))]
        let _ = reason;
        false
    }

    fn remove_keyboard_sticky_plan_motion(&mut self) {
        self.keyboard_sticky_plan = None;
    }

    /// ends the sticky side of a keyboard transition without re-placing anything, callers lay
    /// headers out right after
    fn discard_keyboard_sticky_motion(&mut self) {
        self.remove_keyboard_sticky_plan_motion();
        self.keyboard_timing_reference_view.remove_all_animations();
        self.keyboard_timing_reference_view.set_translation_y(Pixels::ZERO);
    }

    fn layout_sticky_header_host_views(&mut self) {
        let target_frame = Bounds::new(Point::default(), self.viewport_clip_view.frame.size);
        if self.top_sticky_header_host_view.frame != target_frame {
            self.top_sticky_header_host_view.frame = target_frame;
        }
        if self.bottom_sticky_header_host_view.frame != target_frame {
            self.bottom_sticky_header_host_view.frame = target_frame;
        }
    }

    fn apply_selection_presentation_to_mounted_views(&mut self, cx: &mut Context<Self>) {
        let mounted: Vec<(String, AnyView)> =
            self.mounted_items.iter().map(|(id, mounted)| (id.clone(), mounted.view.clone())).collect();
        for (id, view) in mounted {
            self.apply_selection_presentation_to(&view, &id, cx);
        }
    }

    fn apply_selection_presentation_to(&mut self, view: &AnyView, item_id: &str, cx: &mut Context<Self>) {
        let item = self.selection_presentation.items_by_item_id.get(item_id);
        let is_active_message = self.selection_presentation.is_active && item.is_some();
        let content_translation_x = if is_active_message { item.map_or(0.0, |item| item.content_translation_x) } else { 0.0 };
        if is_active_message {
            self.selection_interaction_disabled_ids.insert(view.entity_id());
        } else {
            self.selection_interaction_disabled_ids.remove(&view.entity_id());
        }
        RiseManualListConformances::apply_manual_list_selection_presentation(
            view,
            is_active_message,
            content_translation_x,
            cx,
        );
    }

    fn refresh_selection_overlay(&mut self) {
        let visible_rect = self.selection_visible_rect();
        let rows: Vec<RiseManualListSelectionOverlayViewRow> = if self.selection_presentation.is_active {
            let overlay_width = self.selection_overlay_layer.frame.size.width.max(self.bounds.size.width);
            self.selection_presentation
                .items_by_item_id
                .values()
                .filter_map(|descriptor| {
                    let selection_id = descriptor.selection_id.clone()?;
                    let item = self.item_by_id.get(&descriptor.item_id)?;
                    let frame = self.resolved_frame(&item.borrow());
                    Some(RiseManualListSelectionOverlayViewRow {
                        is_selected: self.selection_presentation.selected_ids.contains(&selection_id),
                        selection_id,
                        frame: Bounds::new(point(Pixels::ZERO, frame.origin.y), size(overlay_width, frame.size.height)),
                        indicator_center_x: px(descriptor.indicator_center_x),
                        accessibility_label: descriptor.accessibility_label.clone(),
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        self.selection_overlay_view.apply(
            self.selection_presentation.is_active,
            rows,
            self.selection_presentation.tint_color,
            visible_rect,
            self.selection_presentation.on_toggle.clone(),
        );
    }

    fn selection_visible_rect(&self) -> Bounds<Pixels> {
        Bounds::new(
            point(Pixels::ZERO, self.scroll_view.content_offset.y - self.layout_origin_y),
            size(self.bounds.size.width.max(px(1.0)), self.bounds.size.height.max(px(1.0))),
        )
    }

    fn refresh_selection_accessibility(&mut self) {
        if !self.selection_presentation.is_active {
            return;
        }
        let visible_rect = self.selection_visible_rect();
        self.selection_overlay_view.update_accessibility(visible_rect);
    }

    /// offset range the rows themselves span before any widening, as (minimum maximum)
    fn natural_scroll_offset_range(&self, current_offset_y: Pixels) -> (Pixels, Pixels) {
        let top_aligned_offset_y = self.layout_origin_y - self.viewport_insets.top;
        let bottom_aligned_offset_y =
            self.layout_origin_y + self.layout_height - self.bounds.size.height + self.viewport_insets.bottom;
        if top_aligned_offset_y <= bottom_aligned_offset_y {
            return (top_aligned_offset_y, bottom_aligned_offset_y);
        }
        let preferred_offset_y = if !self.rests_short_content_at_top
            && (current_offset_y - bottom_aligned_offset_y).abs() < (current_offset_y - top_aligned_offset_y).abs()
        {
            bottom_aligned_offset_y
        } else {
            top_aligned_offset_y
        };
        (preferred_offset_y, preferred_offset_y)
    }

    /// update_scroll_geometry widens an edge to the offset while it moves and nothing narrows it again
    /// so a window that shrank under a fling parks the list past its last row, this fixes it at rest
    fn settle_scroll_geometry_at_rest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_usable_viewport()
            || self.scroll_view.is_tracking
            || self.scroll_view.is_decelerating
            || self.is_programmatic_scroll_animating
            || self.is_applying_transaction
            || self.pending_transaction.is_some()
            || self.has_active_viewport_resize_session()
        {
            return;
        }
        let current_offset_y = self.scroll_view.content_offset.y;
        let (natural_minimum, natural_maximum) = self.natural_scroll_offset_range(current_offset_y);
        let target_y = if current_offset_y > natural_maximum + px(0.5) {
            Some(natural_maximum)
        } else if current_offset_y < natural_minimum - px(0.5) && !self.keeps_leading_reveal(cx) {
            Some(natural_minimum)
        } else {
            None
        };
        let Some(target_y) = target_y else {
            self.perform_without_animations(|this| this.update_scroll_geometry());
            return;
        };
        self.is_programmatic_scroll_animating = true;
        self.pause_deferred_mounting_for_active_scroll();
        let target = self.scroll_view.set_content_offset_animated(point(Pixels::ZERO, target_y), true);
        self.programmatic_scroll_display_link = None;
        self.programmatic_scroll = None;
        let distance = self.scroll_view.content_offset.y - target.y;
        self.ride_programmatic_scroll(
            RiseManualListViewProgrammaticScroll {
                target: RiseManualListViewProgrammaticScrollTarget::Offset(target.y),
                started_at: current_media_time(),
                duration: RiseManualListScrollAnimationPolicy::DURATION,
                initial_distance: distance,
            },
            window,
            cx,
        );
    }

    fn keeps_leading_reveal(&self, cx: &App) -> bool {
        self.leading_scroll_boundary.as_ref().is_some_and(|boundary| boundary(cx).is_some())
    }

    fn update_scroll_geometry(&mut self) {
        let viewport_height = self.bounds.size.height;
        let scroll_content_height = self.layout_height.max(viewport_height).max(px(1.0));
        let current_offset_y = self.scroll_view.content_offset.y;
        let (natural_minimum_offset_y, natural_maximum_offset_y) = self.natural_scroll_offset_range(current_offset_y);
        let is_bouncing_from_top =
            self.scroll_view.is_decelerating && !self.scroll_view.is_tracking && current_offset_y < natural_minimum_offset_y;
        let minimum_offset_y = if is_bouncing_from_top {
            natural_minimum_offset_y
        } else {
            natural_minimum_offset_y.min(current_offset_y)
        };
        let maximum_offset_y = natural_maximum_offset_y.max(current_offset_y);

        let content_size = size(self.bounds.size.width, scroll_content_height);
        let content_inset = Edges {
            top: -minimum_offset_y,
            right: Pixels::ZERO,
            bottom: maximum_offset_y - (scroll_content_height - viewport_height),
            left: Pixels::ZERO,
        };
        if self.scroll_view.content_size == content_size && self.scroll_view.content_inset == content_inset {
            return;
        }

        self.is_updating_scroll_geometry = true;
        let shrinks = content_size.height < self.scroll_view.content_size.height;
        if shrinks {
            if self.scroll_view.content_inset != content_inset {
                self.scroll_view.content_inset = content_inset;
            }
            if self.scroll_view.content_size != content_size {
                self.scroll_view.content_size = content_size;
            }
        } else {
            if self.scroll_view.content_size != content_size {
                self.scroll_view.content_size = content_size;
            }
            if self.scroll_view.content_inset != content_inset {
                self.scroll_view.content_inset = content_inset;
            }
        }
        if (self.scroll_view.content_offset.y - current_offset_y).abs() > px(0.5) {
            if RiseListJumpDebug::is_enabled() {
                tracing::debug!(
                    "GEOMETRY offset clamped {}->{} restored",
                    RiseListJumpDebug::f(current_offset_y),
                    RiseListJumpDebug::f(self.scroll_view.content_offset.y),
                );
            }
            self.scroll_view.set_content_offset(point(Pixels::ZERO, current_offset_y));
        }
        self.is_updating_scroll_geometry = false;
    }
}

impl RiseManualListView {
    fn update_mounted_items(&mut self, reconfigure_ids: HashSet<String>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_usable_viewport() {
            return;
        }
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "list.update_mounted",
            HashMap::from([("reconfigure".to_string(), reconfigure_ids.len().to_string())]),
        );
        'body: {
            let offset_y = self.scroll_view.content_offset.y;
            if let Some(last_mount_offset_y) = self.last_mount_offset_y {
                let delta = offset_y - last_mount_offset_y;
                if delta.abs() > px(0.5) {
                    self.mount_direction = f32::from(delta);
                }
            }
            self.last_mount_offset_y = Some(offset_y);

            let local_visible_minimum_y = offset_y - self.layout_origin_y;
            let local_visible_maximum_y = local_visible_minimum_y + self.bounds.size.height;
            let effective_velocity_y = f32::from(self.effective_estimated_scroll_velocity_y());
            let direction_is_down = if effective_velocity_y.abs() > 80.0 {
                effective_velocity_y > 0.0
            } else {
                self.mount_direction >= 0.0
            };
            let envelope = RiseManualListMountEnvelopePolicy::resolve(
                self.preload_distance,
                self.bounds.size.height,
                effective_velocity_y,
            );
            let local_minimum_y = local_visible_minimum_y
                - if direction_is_down { envelope.behind_distance } else { envelope.ahead_distance };
            let local_maximum_y = local_visible_maximum_y
                + if direction_is_down { envelope.ahead_distance } else { envelope.behind_distance };
            let range = self.item_range(local_minimum_y, local_maximum_y);
            let local_immediate_minimum_y = local_visible_minimum_y
                - if direction_is_down { envelope.immediate_behind_distance } else { envelope.immediate_ahead_distance };
            let local_immediate_maximum_y = local_visible_maximum_y
                + if direction_is_down { envelope.immediate_ahead_distance } else { envelope.immediate_behind_distance };
            let immediate_range = self.item_range(local_immediate_minimum_y, local_immediate_maximum_y);
            let motion_class = self.mount_motion_class();
            let actual_visible_range = self.current_visible_item_range();

            self.invalidate_identity_cached_items(&reconfigure_ids);
            let has_reconfigure_ids = !reconfigure_ids.is_empty();
            self.deferred_reconfigure_ids.extend(reconfigure_ids);

            if !has_reconfigure_ids
                && self.mounted_range == range
                && self.immediate_mount_range == immediate_range
                && self.deferred_mount_direction_is_down == Some(direction_is_down)
                && self.deferred_mount_motion_class == Some(motion_class)
            {
                self.synchronize_visible_items_if_needed(actual_visible_range, window, cx);
                self.update_visible_ids(false, window, cx);
                self.schedule_deferred_mounting_if_needed(window, cx);
                self.schedule_deferred_recycling_if_needed(window, cx);
                break 'body;
            }

            let mut ids_pending_unmount = std::mem::take(&mut self.ids_pending_unmount);
            ids_pending_unmount.clear();
            for (id, mounted) in &self.mounted_items {
                let Some(item) = self.item_by_id.get(id) else {
                    ids_pending_unmount.push(id.clone());
                    continue;
                };
                if self.top_sticky_item_frames.contains_key(id) {
                    continue;
                }
                let frame = self.resolved_frame(&item.borrow());
                let uses_immediate_retention = mounted.is_placeholder && mounted.defers_during_fast_scroll;
                let retention_minimum_y =
                    if uses_immediate_retention { local_immediate_minimum_y } else { local_minimum_y };
                let retention_maximum_y =
                    if uses_immediate_retention { local_immediate_maximum_y } else { local_maximum_y };
                if frame.bottom() <= retention_minimum_y || frame.top() >= retention_maximum_y {
                    if self.viewport_resize_retained_mounted_ids.contains(id) {
                        continue;
                    }
                    ids_pending_unmount.push(id.clone());
                }
            }
            for id in &ids_pending_unmount {
                self.unmount(id, cx);
                self.deferred_reconfigure_ids.remove(id);
            }
            self.ids_pending_unmount = ids_pending_unmount;
            let mut pending_removal = std::mem::take(&mut self.deferred_reconfigure_ids_pending_removal);
            pending_removal.clear();
            pending_removal.extend(
                self.deferred_reconfigure_ids
                    .iter()
                    .filter(|id| !self.mounted_items.contains_key(*id))
                    .cloned(),
            );
            for id in &pending_removal {
                self.deferred_reconfigure_ids.remove(id);
            }
            pending_removal.clear();
            self.deferred_reconfigure_ids_pending_removal = pending_removal;

            self.synchronize_visible_items_if_needed(actual_visible_range.clone(), window, cx);
            self.reconcile_immediate_items_if_needed(
                immediate_range.clone(),
                actual_visible_range,
                motion_class,
                window,
                cx,
            );

            self.mounted_range = range;
            self.immediate_mount_range = immediate_range.clone();
            self.rebuild_deferred_mount_queue(direction_is_down, motion_class, immediate_range);
            self.update_visible_ids(false, window, cx);
            self.schedule_deferred_mounting_if_needed(window, cx);
            self.schedule_deferred_recycling_if_needed(window, cx);
        }
        ChatPerformanceTrace::shared().end_operation(
            trace_token,
            2.0,
            HashMap::from([("mounted".to_string(), self.mounted_items.len().to_string())]),
        );
    }

    fn mount(&mut self, item: &RiseManualListViewLayoutItem, window: &mut Window, cx: &mut Context<Self>) {
        let install_started_at = current_media_time();
        let identity_cached_view = self.take_identity_cached_view(item);
        let reusable_view = if identity_cached_view.is_none() {
            self.take_reusable_view(&item.content.reuse_identifier)
        } else {
            None
        };
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "row.mount",
            HashMap::from([
                ("reuse".to_string(), if reusable_view.is_none() { "0" } else { "1" }.to_string()),
                ("identity_hit".to_string(), if identity_cached_view.is_none() { "0" } else { "1" }.to_string()),
                ("type".to_string(), item.content.reuse_identifier.clone()),
                (
                    "visible".to_string(),
                    if self.is_frame_visible(self.resolved_frame(item)) { "1" } else { "0" }.to_string(),
                ),
            ]),
        );
        let is_identity_hit = identity_cached_view.is_some();
        let view = match identity_cached_view {
            Some(view) => view,
            None => (item.content.view)(reusable_view.clone(), window, cx),
        };
        if !is_identity_hit
            && let Some(reusable_view) = reusable_view
            && reusable_view.entity_id() != view.entity_id()
        {
            self.recycle(reusable_view, &item.content.reuse_identifier, cx);
        }
        let frame = self.mounted_frame(item);
        let host = self.mounted_host_view(item);
        self.mounted_items.insert(
            item.id.clone(),
            RiseManualListViewMountedItem {
                view: view.clone(),
                frame,
                host,
                reuse_identifier: item.content.reuse_identifier.clone(),
                content_version: item.content.content_version,
                layout_generation: self.layout_generation,
                is_placeholder: false,
                defers_during_fast_scroll: item.content.mount_policy
                    == RiseManualListItemMountPolicy::DeferredDuringFastScroll,
            },
        );
        self.apply_selection_presentation_to(&view, &item.id, cx);
        self.apply_animated_media_viewport_visibility(&view, item, cx);
        ChatPerformanceTrace::shared().end_operation(trace_token, 1.0, HashMap::new());
        self.record_mount_cost(install_started_at, item, window);
    }

    fn reconcile_immediate_items_if_needed(
        &mut self,
        range: Range<usize>,
        actual_visible_range: Range<usize>,
        motion_class: RiseManualListViewMountMotionClass,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let has_deferred_reconfigurations = !self.deferred_reconfigure_ids.is_empty();
        for index in range.clone() {
            if actual_visible_range.contains(&index) {
                continue;
            }
            let item_ref = self.layout_items[index].clone();
            let item = item_ref.borrow();
            let is_current = self.mounted_items.get(&item.id).is_some_and(|mounted| {
                !mounted.is_placeholder
                    && (!has_deferred_reconfigurations || !self.deferred_reconfigure_ids.contains(&item.id))
                    && mounted.content_version == item.content.content_version
                    && mounted.reuse_identifier == item.content.reuse_identifier
                    && mounted.layout_generation == self.layout_generation
                    && mounted.frame == self.mounted_frame(&item)
                    && mounted.host == self.mounted_host_view(&item)
            });
            if !is_current {
                drop(item);
                self.perform_without_animations(|this| {
                    this.reconcile_immediate_items(range, actual_visible_range, motion_class, window, cx);
                });
                return;
            }
        }
    }

    fn reconcile_immediate_items(
        &mut self,
        range: Range<usize>,
        actual_visible_range: Range<usize>,
        motion_class: RiseManualListViewMountMotionClass,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reconcile_token = ChatPerformanceTrace::shared().begin_operation("list.reconcile_immediate", HashMap::new());
        let mut mount_count = 0usize;
        let mut configure_count = 0usize;
        let mut placeholder_count = 0usize;
        for index in range.clone() {
            if actual_visible_range.contains(&index) {
                continue;
            }
            let item_ref = self.layout_items[index].clone();
            let item = item_ref.borrow();
            let has_prepared_view = self.has_identity_cached_view(&item);
            let defers_mount =
                item.content.mount_policy == RiseManualListItemMountPolicy::DeferredDuringFastScroll && !has_prepared_view;
            let defer_for_speed = motion_class == RiseManualListViewMountMotionClass::Fast && !has_prepared_view;
            if let Some(mounted) = self.mounted_items.get(&item.id).cloned() {
                let needs_configuration = mounted.is_placeholder
                    || self.deferred_reconfigure_ids.contains(&item.id)
                    || mounted.content_version != item.content.content_version
                    || mounted.reuse_identifier != item.content.reuse_identifier;
                if needs_configuration {
                    if defers_mount || defer_for_speed {
                        placeholder_count += 1;
                        self.preserve_or_install_placeholder(mounted, &item, cx);
                    } else {
                        configure_count += 1;
                        self.configure_mounted_view(mounted, &item, window, cx);
                        self.deferred_reconfigure_ids.remove(&item.id);
                    }
                } else if mounted.layout_generation != self.layout_generation
                    || mounted.frame != self.mounted_frame(&item)
                    || mounted.host != self.mounted_host_view(&item)
                {
                    self.update_mounted_frame(mounted, &item, cx);
                }
            } else if has_prepared_view {
                mount_count += 1;
                self.mount(&item, window, cx);
                self.deferred_reconfigure_ids.remove(&item.id);
            } else if defers_mount || defer_for_speed {
                placeholder_count += 1;
                self.mount_placeholder(&item, window, cx);
            } else {
                mount_count += 1;
                self.mount(&item, window, cx);
                self.deferred_reconfigure_ids.remove(&item.id);
            }
        }
        let motion = match motion_class {
            RiseManualListViewMountMotionClass::Idle => "idle",
            RiseManualListViewMountMotionClass::Active => "active",
            RiseManualListViewMountMotionClass::Fast => "fast",
        };
        ChatPerformanceTrace::shared().end_operation(
            reconcile_token,
            1.0,
            HashMap::from([
                ("span".to_string(), range.len().to_string()),
                ("mounts".to_string(), mount_count.to_string()),
                ("configures".to_string(), configure_count.to_string()),
                ("placeholders".to_string(), placeholder_count.to_string()),
                ("motion".to_string(), motion.to_string()),
            ]),
        );
    }

    fn configure_mounted_view(
        &mut self,
        mounted: RiseManualListViewMountedItem,
        item: &RiseManualListViewLayoutItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let install_started_at = current_media_time();
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "row.configure",
            HashMap::from([
                ("placeholder".to_string(), if mounted.is_placeholder { "1" } else { "0" }.to_string()),
                ("type".to_string(), item.content.reuse_identifier.clone()),
                (
                    "visible".to_string(),
                    if self.is_frame_visible(self.resolved_frame(item)) { "1" } else { "0" }.to_string(),
                ),
            ]),
        );
        let (identity_cached_view, reusable_view) = if mounted.is_placeholder {
            let identity_cached_view = self.take_identity_cached_view(item);
            let reusable_view = if identity_cached_view.is_none() {
                self.take_reusable_view(&item.content.reuse_identifier)
            } else {
                None
            };
            (identity_cached_view, reusable_view)
        } else if mounted.reuse_identifier == item.content.reuse_identifier
            && !self.is_held_by_jump_snapshot(&mounted.view)
        {
            (None, Some(mounted.view.clone()))
        } else {
            self.recycle(mounted.view.clone(), &mounted.reuse_identifier, cx);
            (None, self.take_reusable_view(&item.content.reuse_identifier))
        };

        let is_identity_hit = identity_cached_view.is_some();
        let configured_view = match identity_cached_view {
            Some(view) => view,
            None => (item.content.view)(reusable_view.clone(), window, cx),
        };
        if !is_identity_hit
            && let Some(reusable_view) = reusable_view
            && reusable_view.entity_id() != configured_view.entity_id()
        {
            self.recycle(reusable_view, &item.content.reuse_identifier, cx);
        }
        let frame = self.mounted_frame(item);
        let host = self.mounted_host_view(item);
        self.mounted_items.insert(
            item.id.clone(),
            RiseManualListViewMountedItem {
                view: configured_view.clone(),
                frame,
                host,
                reuse_identifier: item.content.reuse_identifier.clone(),
                content_version: item.content.content_version,
                layout_generation: self.layout_generation,
                is_placeholder: false,
                defers_during_fast_scroll: item.content.mount_policy
                    == RiseManualListItemMountPolicy::DeferredDuringFastScroll,
            },
        );
        self.apply_selection_presentation_to(&configured_view, &item.id, cx);
        self.apply_animated_media_viewport_visibility(&configured_view, item, cx);
        ChatPerformanceTrace::shared().end_operation(trace_token, 1.0, HashMap::new());
        self.record_mount_cost(install_started_at, item, window);
    }

    fn mount_placeholder(&mut self, item: &RiseManualListViewLayoutItem, window: &mut Window, cx: &mut Context<Self>) {
        let frame = self.mounted_frame(item);
        let view = match item.content.placeholder_view.as_ref() {
            Some(placeholder_view) => placeholder_view(window, cx),
            None => AnyView::from(cx.new(|_| RiseManualListViewPlaceholderView)),
        };
        let host = self.mounted_host_view(item);
        self.mounted_items.insert(
            item.id.clone(),
            RiseManualListViewMountedItem {
                view: view.clone(),
                frame,
                host,
                reuse_identifier: item.content.reuse_identifier.clone(),
                content_version: item.content.content_version,
                layout_generation: self.layout_generation,
                is_placeholder: true,
                defers_during_fast_scroll: true,
            },
        );
        self.apply_selection_presentation_to(&view, &item.id, cx);
    }

    fn preserve_or_install_placeholder(
        &mut self,
        mounted: RiseManualListViewMountedItem,
        item: &RiseManualListViewLayoutItem,
        cx: &mut Context<Self>,
    ) {
        if mounted.is_placeholder {
            let host = self.mounted_host_view(item);
            let frame = self.mounted_frame(item);
            self.mounted_items.insert(
                item.id.clone(),
                RiseManualListViewMountedItem {
                    view: mounted.view,
                    frame,
                    host,
                    reuse_identifier: item.content.reuse_identifier.clone(),
                    content_version: item.content.content_version,
                    layout_generation: self.layout_generation,
                    is_placeholder: true,
                    defers_during_fast_scroll: true,
                },
            );
        } else {
            self.update_mounted_frame(mounted, item, cx);
        }
    }

    fn update_mounted_frame(
        &mut self,
        mounted: RiseManualListViewMountedItem,
        item: &RiseManualListViewLayoutItem,
        cx: &mut Context<Self>,
    ) {
        let host = self.mounted_host_view(item);
        let frame = self.mounted_frame(item);
        let view = mounted.view.clone();
        self.mounted_items.insert(
            item.id.clone(),
            RiseManualListViewMountedItem {
                view: mounted.view,
                frame,
                host,
                reuse_identifier: mounted.reuse_identifier,
                content_version: mounted.content_version,
                layout_generation: self.layout_generation,
                is_placeholder: mounted.is_placeholder,
                defers_during_fast_scroll: mounted.defers_during_fast_scroll,
            },
        );
        self.apply_selection_presentation_to(&view, &item.id, cx);
        self.apply_animated_media_viewport_visibility(&view, item, cx);
    }

    fn apply_animated_media_viewport_visibility(
        &self,
        view: &AnyView,
        item: &RiseManualListViewLayoutItem,
        cx: &mut Context<Self>,
    ) {
        let is_visible = self.is_frame_visible(self.resolved_frame(item));
        RiseManualListConformances::set_animated_media_visible_in_viewport(view, is_visible, cx);
    }

    fn cancel_deferred_mounting(&mut self) {
        self.pause_deferred_mount_display_link();
        self.deferred_mount_queue_signature = None;
        self.deferred_mount_generation = self.deferred_mount_generation.wrapping_add(1);
        self.deferred_mount_scheduled_generation = None;
        self.deferred_mount_needs_work = false;
        self.deferred_mount_queue.clear();
        self.deferred_mount_queue_index = 0;
        self.deferred_mount_direction_is_down = None;
        self.deferred_mount_motion_class = None;
        self.deferred_reconfigure_ids.clear();
        self.mounted_range = 0..0;
        self.immediate_mount_range = 0..0;
    }

    fn pause_deferred_mounting_for_active_scroll(&mut self) {
        self.deferred_mount_queue_signature = None;
        self.pause_deferred_recycling_for_active_scroll();
        self.pause_deferred_mount_display_link();
        if self.deferred_mount_scheduled_generation.is_none() {
            return;
        }
        self.deferred_mount_generation = self.deferred_mount_generation.wrapping_add(1);
        self.deferred_mount_scheduled_generation = None;
    }

    fn schedule_deferred_mounting_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.deferred_mount_needs_work
            || self.mounted_range.is_empty()
            || self.deferred_mount_queue_index >= self.deferred_mount_queue.len()
        {
            self.deferred_mount_scheduled_generation = None;
            self.pause_deferred_mount_display_link();
            return;
        }

        self.deferred_mount_scheduled_generation = Some(self.deferred_mount_generation);
        if self.deferred_mount_display_link.is_some()
            || !self.is_in_window
            || !self.has_usable_viewport()
            || !Self::window_counts_as_active(window)
        {
            return;
        }

        self.deferred_mount_display_link = Some(Self::add_display_link(window, cx, Self::deferred_mount_did_tick));
    }

    fn window_counts_as_active(window: &Window) -> bool {
        window.is_window_active() || crate::app::dev_drive::ENABLED
    }

    fn pause_deferred_mount_display_link(&mut self) {
        self.deferred_mount_display_link = None;
    }

    fn resume_deferred_mount_display_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.schedule_deferred_mounting_if_needed(window, cx);
    }

    fn deferred_mount_did_tick(&mut self, link: RiseManualListMountFrame, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.deferred_mount_scheduled_generation;
        let Some(generation) = generation
            .filter(|_| self.is_in_window && self.has_usable_viewport() && Self::window_counts_as_active(window))
        else {
            self.pause_deferred_mount_display_link();
            return;
        };
        let frame = link;
        if self.mount_budget_last_finished_at < frame.timestamp {
            self.mount_budget_consumed = 0.0;
        }
        self.mount_budget_window_started_at = frame.timestamp;
        self.mount_budget_frame = Some(frame);
        self.perform_deferred_mount_batch(generation, Some(frame), window, cx);
    }

    fn perform_deferred_mount_batch(
        &mut self,
        generation: u64,
        frame: Option<RiseManualListMountFrame>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if generation != self.deferred_mount_generation || self.deferred_mount_scheduled_generation != Some(generation) {
            return;
        }
        self.deferred_mount_scheduled_generation = None;
        if !self.has_usable_viewport() {
            self.pause_deferred_mount_display_link();
            return;
        }

        let trace_token = ChatPerformanceTrace::shared().begin_operation("list.deferred_mount_batch", HashMap::new());
        let mut did_mount_rows = false;
        'batch: {
            let frames_per_second = frame.map_or_else(
                || maximum_frames_per_second(window).max(60) as usize,
                |frame| frame.frames_per_second(),
            );
            let batch = RiseManualListMountEnvelopePolicy::deferred_mount_batch(
                f32::from(self.effective_estimated_scroll_velocity_y()),
                frames_per_second,
                Pixels::ZERO,
                !self.is_user_scrolling(),
            );
            let now = current_media_time();
            self.refresh_mount_budget(now, window);
            let remaining_budget = (batch.time_budget - self.mount_budget_consumed).max(0.0);
            if batch.maximum_mounts == 0 || remaining_budget <= 0.0 {
                self.schedule_deferred_mounting_if_needed(window, cx);
                break 'batch;
            }
            let maximum_mounts = batch.maximum_mounts;
            let semantic_anchor = self.capture_visible_anchor();
            let deadline = (now + remaining_budget).min(frame.map_or(f64::INFINITY, |frame| frame.work_deadline()));
            let mut mounted_count = 0usize;
            let mut drawn_change_count = 0usize;
            let mut fallback_hosted_mount_count = 0usize;
            while self.deferred_mount_queue_index < self.deferred_mount_queue.len() && mounted_count < maximum_mounts {
                let mut candidate = self.deferred_mount_queue[self.deferred_mount_queue_index];
                if !self.mounted_range.contains(&candidate.index) {
                    self.deferred_mount_queue_index += 1;
                    continue;
                }
                let mut item_ref = self.layout_items[candidate.index].clone();
                if self.deferred_item_requires_fallback_host_install(&item_ref.borrow())
                    && fallback_hosted_mount_count
                        >= RiseManualListMountEnvelopePolicy::MAXIMUM_FALLBACK_HOSTED_MOUNTS_PER_IDLE_BATCH
                {
                    let Some(next_eligible_index) =
                        self.next_deferred_mount_candidate_index(self.deferred_mount_queue_index, true)
                    else {
                        break;
                    };
                    self.deferred_mount_queue.swap(self.deferred_mount_queue_index, next_eligible_index);
                    candidate = self.deferred_mount_queue[self.deferred_mount_queue_index];
                    item_ref = self.layout_items[candidate.index].clone();
                }
                let item = item_ref.borrow();
                let predicted_cost = self
                    .deferred_mount_average_cost_by_reuse_identifier
                    .get(&item.content.reuse_identifier)
                    .copied()
                    .unwrap_or(self.deferred_mount_average_cost)
                    .max(0.000_2);
                let admission_time = current_media_time();
                if let Some(frame) = frame
                    && !frame.can_start_mount(admission_time, predicted_cost)
                {
                    break;
                }
                if mounted_count > 0 && deadline - admission_time < predicted_cost {
                    break;
                }
                self.deferred_mount_queue_index += 1;
                let requires_fallback_host_install = self.deferred_item_requires_fallback_host_install(&item);
                let mut configured_content = false;
                if let Some(mounted) = self.mounted_items.get(&item.id).cloned() {
                    if mounted.is_placeholder
                        || self.deferred_reconfigure_ids.contains(&item.id)
                        || mounted.content_version != item.content.content_version
                        || mounted.reuse_identifier != item.content.reuse_identifier
                    {
                        self.configure_mounted_view(mounted, &item, window, cx);
                        self.deferred_reconfigure_ids.remove(&item.id);
                        configured_content = true;
                    } else if mounted.layout_generation != self.layout_generation
                        || mounted.frame != self.mounted_frame(&item)
                        || mounted.host != self.mounted_host_view(&item)
                    {
                        if mounted.frame != self.mounted_frame(&item) || mounted.host != self.mounted_host_view(&item) {
                            drawn_change_count += 1;
                        }
                        self.update_mounted_frame(mounted, &item, cx);
                    } else {
                        if current_media_time() >= deadline {
                            break;
                        }
                        continue;
                    }
                } else {
                    self.mount(&item, window, cx);
                    self.deferred_reconfigure_ids.remove(&item.id);
                    configured_content = true;
                }
                mounted_count += 1;
                if configured_content {
                    drawn_change_count += 1;
                }
                if configured_content && requires_fallback_host_install {
                    fallback_hosted_mount_count += 1;
                }
                if current_media_time() >= deadline {
                    break;
                }
            }

            did_mount_rows = drawn_change_count > 0;
            if did_mount_rows
                && let Some(semantic_anchor) = semantic_anchor
                && !self.is_user_scrolling()
            {
                let _ = self.scroll_to(&semantic_anchor, false, window, cx);
            }

            if self.deferred_mount_queue_index >= self.deferred_mount_queue.len() {
                self.deferred_mount_needs_work = false;
            }
            self.schedule_deferred_mounting_if_needed(window, cx);
        }
        ChatPerformanceTrace::shared().end_operation(trace_token, 1.0, HashMap::new());
        let frame_owed = std::mem::take(&mut self.deferred_mount_queue_frame_owed);
        if did_mount_rows || frame_owed {
            cx.notify();
        }
    }

    fn deferred_item_requires_fallback_host_install(&self, item: &RiseManualListViewLayoutItem) -> bool {
        if item.content.install_cost != RiseManualListInstallCost::HostedFallback || self.has_identity_cached_view(item) {
            return false;
        }
        let Some(mounted) = self.mounted_items.get(&item.id) else {
            return true;
        };
        mounted.is_placeholder
            || self.deferred_reconfigure_ids.contains(&item.id)
            || mounted.content_version != item.content.content_version
            || mounted.reuse_identifier != item.content.reuse_identifier
    }

    fn next_deferred_mount_candidate_index(&self, after: usize, skipping_fallback_host_install: bool) -> Option<usize> {
        let queue_index = after;
        if queue_index + 1 >= self.deferred_mount_queue.len() {
            return None;
        }
        for candidate_index in (queue_index + 1)..self.deferred_mount_queue.len() {
            let candidate = self.deferred_mount_queue[candidate_index];
            if !self.mounted_range.contains(&candidate.index) {
                continue;
            }
            if !skipping_fallback_host_install
                || !self.deferred_item_requires_fallback_host_install(&self.layout_items[candidate.index].borrow())
            {
                return Some(candidate_index);
            }
        }
        None
    }

    fn rebuild_deferred_mount_queue(
        &mut self,
        direction_is_down: bool,
        motion_class: RiseManualListViewMountMotionClass,
        immediate_range: Range<usize>,
    ) {
        let signature = RiseManualListViewDeferredMountQueueSignature {
            layout_generation: self.layout_generation,
            mounted_lower_bound: self.mounted_range.start,
            mounted_upper_bound: self.mounted_range.end,
            immediate_lower_bound: immediate_range.start,
            immediate_upper_bound: immediate_range.end,
            direction_is_down,
            motion_class,
            has_deferred_reconfigurations: !self.deferred_reconfigure_ids.is_empty(),
        };
        if self.deferred_mount_queue_signature == Some(signature)
            && self.deferred_mount_queue_index < self.deferred_mount_queue.len()
        {
            return;
        }
        self.deferred_mount_queue_signature = Some(signature);
        let mut queue = std::mem::take(&mut self.deferred_mount_queue);
        queue.clear();
        queue.reserve(self.mounted_range.len());

        if !self.mounted_range.is_empty() {
            let local_minimum_y = self.scroll_view.content_offset.y - self.layout_origin_y;
            let local_maximum_y = local_minimum_y + self.bounds.size.height;
            let has_deferred_reconfigurations = !self.deferred_reconfigure_ids.is_empty();
            let warms_deferred_rows_across_outer_envelope = motion_class != RiseManualListViewMountMotionClass::Fast;

            for index in self.mounted_range.clone() {
                let item = self.layout_items[index].borrow();
                let defers_during_fast_scroll =
                    item.content.mount_policy == RiseManualListItemMountPolicy::DeferredDuringFastScroll;
                if defers_during_fast_scroll
                    && !immediate_range.contains(&index)
                    && !warms_deferred_rows_across_outer_envelope
                {
                    continue;
                }
                if let Some(mounted) = self.mounted_items.get(&item.id)
                    && !mounted.is_placeholder
                    && mounted.layout_generation == self.layout_generation
                    && mounted.content_version == item.content.content_version
                    && mounted.reuse_identifier == item.content.reuse_identifier
                    && (!has_deferred_reconfigurations || !self.deferred_reconfigure_ids.contains(&item.id))
                {
                    continue;
                }

                let frame = self.resolved_frame(&item);
                let distance = if frame.bottom() <= local_minimum_y {
                    local_minimum_y - frame.bottom()
                } else if frame.top() >= local_maximum_y {
                    frame.top() - local_maximum_y
                } else {
                    Pixels::ZERO
                };
                let is_ahead =
                    if direction_is_down { frame.top() >= local_maximum_y } else { frame.bottom() <= local_minimum_y };
                queue.push(RiseManualListViewDeferredMountCandidate {
                    index,
                    distance,
                    is_ahead,
                    defers_during_fast_scroll,
                });
            }

            let has_directional_motion = self.mount_direction.abs() > 0.5;
            let precedes = |lhs: &RiseManualListViewDeferredMountCandidate,
                            rhs: &RiseManualListViewDeferredMountCandidate| {
                let lhs_visible = lhs.distance <= px(0.5);
                let rhs_visible = rhs.distance <= px(0.5);
                if lhs_visible != rhs_visible {
                    return lhs_visible;
                }
                if lhs.is_ahead != rhs.is_ahead && has_directional_motion {
                    return lhs.is_ahead;
                }
                if motion_class == RiseManualListViewMountMotionClass::Fast
                    && lhs.defers_during_fast_scroll != rhs.defers_during_fast_scroll
                {
                    return !lhs.defers_during_fast_scroll;
                }
                if (lhs.distance - rhs.distance).abs() > px(0.5) {
                    return lhs.distance < rhs.distance;
                }
                if direction_is_down { lhs.index < rhs.index } else { lhs.index > rhs.index }
            };
            for sorted_end in 1..queue.len() {
                let mut position = sorted_end;
                while position > 0 && precedes(&queue[position], &queue[position - 1]) {
                    queue.swap(position, position - 1);
                    position -= 1;
                }
            }
        }
        self.deferred_mount_queue = queue;

        self.deferred_mount_queue_index = 0;
        self.deferred_mount_direction_is_down = Some(direction_is_down);
        self.deferred_mount_motion_class = Some(motion_class);
        self.deferred_mount_needs_work = !self.deferred_mount_queue.is_empty();
        self.deferred_mount_queue_frame_owed = true;
    }

    fn refresh_mount_budget(&mut self, now: f64, window: &Window) {
        if let Some(frame) = self.mount_budget_frame
            && now < frame.target_timestamp
        {
            return;
        }
        let frame_duration = self
            .mount_budget_frame
            .map_or_else(|| 1.0 / f64::from(maximum_frames_per_second(window).max(60)), |frame| frame.duration());
        if now - self.mount_budget_window_started_at >= frame_duration {
            self.mount_budget_window_started_at = now;
            self.mount_budget_consumed = 0.0;
        }
    }

    fn record_mount_cost(&mut self, since: f64, item: &RiseManualListViewLayoutItem, window: &Window) {
        let started_at = since;
        self.refresh_mount_budget(started_at, window);
        let finished_at = current_media_time();
        let cost = finished_at - started_at;
        self.mount_budget_consumed += cost;
        self.mount_budget_last_finished_at = finished_at;
        self.record_deferred_mount_cost(cost, &item.content.reuse_identifier);
    }

    fn record_deferred_mount_cost(&mut self, raw_cost: f64, reuse_identifier: &str) {
        if !raw_cost.is_finite() || raw_cost <= 0.0 {
            return;
        }
        let cost = raw_cost.clamp(0.000_1, 0.02);
        self.deferred_mount_average_cost = self.deferred_mount_average_cost * 0.78 + cost * 0.22;
        let previous = self
            .deferred_mount_average_cost_by_reuse_identifier
            .get(reuse_identifier)
            .copied()
            .unwrap_or(self.deferred_mount_average_cost);
        self.deferred_mount_average_cost_by_reuse_identifier
            .insert(reuse_identifier.to_string(), previous * 0.7 + cost * 0.3);
    }

    fn unmount(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(mounted) = self.mounted_items.remove(id) else {
            return;
        };
        if self.top_sticky_item_frames.remove(id).is_some() {
            RiseManualListConformances::set_top_sticky_header_faded(&mounted.view, false, false, cx);
        }
        self.detached_top_sticky_item_ids.remove(id);
        RiseManualListConformances::set_animated_media_visible_in_viewport(&mounted.view, false, cx);
        if mounted.is_placeholder {
            return;
        }
        if RiseManualListDeferredRecyclePolicy::should_defer_preparation(mounted.defers_during_fast_scroll) {
            let detached_item = RiseManualListViewDeferredRecycleItem {
                id: id.to_string(),
                view: mounted.view,
                reuse_identifier: mounted.reuse_identifier,
                content_version: mounted.content_version,
            };
            if self.item_by_id.contains_key(id) && !self.identity_cache_exclusion_ids.contains(id) {
                self.detached_identity_items.push(detached_item);
                if self.detached_identity_items.len() > RiseManualListDeferredRecyclePolicy::MAXIMUM_IDENTITY_CACHED_ITEMS {
                    let evicted = self.detached_identity_items.remove(0);
                    self.deferred_recycle_items.push(evicted);
                }
            } else {
                self.deferred_recycle_items.push(detached_item);
            }
        } else {
            self.recycle(mounted.view, &mounted.reuse_identifier, cx);
        }
    }

    fn take_identity_cached_view(&mut self, item: &RiseManualListViewLayoutItem) -> Option<AnyView> {
        if self.deferred_reconfigure_ids.contains(&item.id) {
            return None;
        }
        let index = self.detached_identity_items.iter().rposition(|detached| {
            detached.id == item.id
                && detached.content_version == item.content.content_version
                && detached.reuse_identifier == item.content.reuse_identifier
        })?;
        let view = self.detached_identity_items.remove(index).view;
        ChatPerformanceTrace::shared().record(
            "row.identity_cache_hit",
            HashMap::from([("type".to_string(), item.content.reuse_identifier.clone())]),
        );
        self.reset_deferred_recycle_schedule_if_empty();
        Some(view)
    }

    fn has_identity_cached_view(&self, item: &RiseManualListViewLayoutItem) -> bool {
        if self.deferred_reconfigure_ids.contains(&item.id) {
            return false;
        }
        self.detached_identity_items.iter().any(|detached| {
            detached.id == item.id
                && detached.content_version == item.content.content_version
                && detached.reuse_identifier == item.content.reuse_identifier
        })
    }

    fn invalidate_identity_cached_items(&mut self, ids: &HashSet<String>) {
        if ids.is_empty() || self.detached_identity_items.is_empty() {
            return;
        }
        let count_before_invalidation = self.detached_identity_items.len();
        for index in (0..self.detached_identity_items.len()).rev() {
            if ids.contains(&self.detached_identity_items[index].id) {
                let invalidated = self.detached_identity_items.remove(index);
                self.deferred_recycle_items.push(invalidated);
            }
        }
        let invalidated_count = count_before_invalidation - self.detached_identity_items.len();
        if invalidated_count > 0 {
            ChatPerformanceTrace::shared().record(
                "row.identity_cache_invalidate",
                HashMap::from([("count".to_string(), invalidated_count.to_string())]),
            );
        }
    }

    fn invalidate_identity_cached_factories(&mut self, updated_ids: Option<&HashSet<String>>) {
        if self.detached_identity_items.is_empty() {
            return;
        }
        let Some(updated_ids) = updated_ids else {
            let invalidated_count = self.detached_identity_items.len();
            let detached = std::mem::take(&mut self.detached_identity_items);
            self.deferred_recycle_items.extend(detached);
            ChatPerformanceTrace::shared().record(
                "row.identity_cache_invalidate",
                HashMap::from([("count".to_string(), invalidated_count.to_string())]),
            );
            return;
        };
        self.invalidate_identity_cached_items(updated_ids);
    }

    fn take_reusable_view(&mut self, identifier: &str) -> Option<AnyView> {
        if let Some(deferred_index) = self
            .deferred_recycle_items
            .iter()
            .rposition(|deferred| deferred.reuse_identifier == identifier && !self.is_held_by_jump_snapshot(&deferred.view))
        {
            let view = self.deferred_recycle_items.remove(deferred_index).view;
            self.reset_deferred_recycle_schedule_if_empty();
            return Some(view);
        }
        if self.deferred_recycle_items.len() > RiseManualListDeferredRecyclePolicy::PREFERRED_GENERIC_OVERFLOW_LIMIT
            && RiseManualListMountEnvelopePolicy::is_fallback_hosted_row(identifier)
            && let Some(deferred_index) = self.deferred_recycle_items.iter().rposition(|deferred| {
                RiseManualListMountEnvelopePolicy::is_fallback_hosted_row(&deferred.reuse_identifier)
                    && RiseManualListViewHooks::is_message_hosting_view_or_default(&deferred.view)
                    && !self.is_held_by_jump_snapshot(&deferred.view)
            })
        {
            let view = self.deferred_recycle_items.remove(deferred_index).view;
            self.reset_deferred_recycle_schedule_if_empty();
            return Some(view);
        }
        let index = self
            .reusable_views_by_identifier
            .get(identifier)?
            .iter()
            .rposition(|view| !self.is_held_by_jump_snapshot(view))?;
        let views = self.reusable_views_by_identifier.get_mut(identifier)?;
        let view = views.remove(index);
        let is_empty = views.is_empty();
        self.reusable_view_total_count -= 1;
        if is_empty {
            self.reusable_views_by_identifier.remove(identifier);
        }
        Some(view)
    }

    /// true if the running jump slide still draws this view
    fn is_held_by_jump_snapshot(&self, view: &AnyView) -> bool {
        self.active_jump_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.rows.iter().any(|(row, _)| row.entity_id() == view.entity_id()))
    }

    fn recycle(&mut self, view: AnyView, identifier: &str, cx: &mut App) {
        RiseManualListConformances::prepare_for_manual_list_reuse(&view, cx);
        if self.reusable_view_total_count >= self.reuse_pool_limit {
            return;
        }
        self.reusable_views_by_identifier.entry(identifier.to_string()).or_default().push(view);
        self.reusable_view_total_count += 1;
    }

    fn schedule_deferred_recycling_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_deferred_recycle_work() {
            self.reset_deferred_recycle_schedule_if_empty();
            return;
        }
        if self.is_user_scrolling() {
            self.pause_deferred_recycling_for_active_scroll();
            return;
        }
        let now = current_media_time();
        if self.deferred_recycle_idle_since.is_none() {
            self.deferred_recycle_idle_since = Some(now);
        }
        if self.deferred_recycle_scheduled_generation == Some(self.deferred_recycle_generation) {
            return;
        }
        let Some(idle_since) = self.deferred_recycle_idle_since else {
            return;
        };
        let generation = self.deferred_recycle_generation;
        self.deferred_recycle_scheduled_generation = Some(generation);
        let frames_per_second = maximum_frames_per_second(window).max(60);
        let frame_interval = 1.0 / f64::from(frames_per_second);
        let delay = frame_interval.max(RiseManualListDeferredRecyclePolicy::remaining_quiet_time(idle_since, now));
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            let _ = this.update_in(cx, |this, window, cx| this.perform_deferred_recycle_batch(generation, window, cx));
        })
        .detach();
    }

    fn perform_deferred_recycle_batch(&mut self, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        if generation != self.deferred_recycle_generation
            || self.deferred_recycle_scheduled_generation != Some(generation)
        {
            return;
        }
        self.deferred_recycle_scheduled_generation = None;
        if self.is_user_scrolling() {
            self.deferred_recycle_idle_since = None;
            return;
        }
        let now = current_media_time();
        let Some(idle_since) = self.deferred_recycle_idle_since else {
            self.schedule_deferred_recycling_if_needed(window, cx);
            return;
        };
        if RiseManualListDeferredRecyclePolicy::remaining_quiet_time(idle_since, now) > 0.0 {
            self.schedule_deferred_recycling_if_needed(window, cx);
            return;
        }

        let starting_identity_count = self.detached_identity_items.len();
        let starting_deferred_count = self.deferred_recycle_items.len();
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "list.deferred_recycle_batch",
            HashMap::from([
                ("identity".to_string(), starting_identity_count.to_string()),
                ("overflow".to_string(), starting_deferred_count.to_string()),
            ]),
        );
        let deadline = now + RiseManualListDeferredRecyclePolicy::TIME_BUDGET;
        let mut recycled_count = 0usize;
        while recycled_count < RiseManualListDeferredRecyclePolicy::MAXIMUM_RECYCLES_PER_BATCH {
            let Some(item) = self.deferred_recycle_items.pop() else {
                break;
            };
            if self.reusable_view_total_count < self.reuse_pool_limit {
                self.recycle(item.view, &item.reuse_identifier, cx);
            }
            recycled_count += 1;
            if current_media_time() >= deadline {
                break;
            }
        }
        if !self.has_deferred_recycle_work() {
            self.deferred_recycle_idle_since = None;
        }
        self.schedule_deferred_recycling_if_needed(window, cx);
        ChatPerformanceTrace::shared().end_operation(trace_token, 0.5, HashMap::new());
    }

    fn has_deferred_recycle_work(&self) -> bool {
        !self.deferred_recycle_items.is_empty()
    }

    fn pause_deferred_recycling_for_active_scroll(&mut self) {
        if self.deferred_recycle_scheduled_generation.is_none() && self.deferred_recycle_idle_since.is_none() {
            return;
        }
        self.deferred_recycle_generation = self.deferred_recycle_generation.wrapping_add(1);
        self.deferred_recycle_scheduled_generation = None;
        self.deferred_recycle_idle_since = None;
    }

    fn reset_deferred_recycle_schedule_if_empty(&mut self) {
        if self.has_deferred_recycle_work() {
            return;
        }
        if self.deferred_recycle_scheduled_generation.is_some() {
            self.deferred_recycle_generation = self.deferred_recycle_generation.wrapping_add(1);
            self.deferred_recycle_scheduled_generation = None;
        }
        self.deferred_recycle_idle_since = None;
    }

    fn trim_reuse_pool(&mut self) {
        while self.reusable_view_total_count > self.reuse_pool_limit {
            let Some(identifier) = self
                .reusable_views_by_identifier
                .iter()
                .find(|(_, views)| !views.is_empty())
                .map(|(identifier, _)| identifier.clone())
            else {
                break;
            };
            let Some(views) = self.reusable_views_by_identifier.get_mut(&identifier) else {
                break;
            };
            views.pop();
            let is_empty = views.is_empty();
            self.reusable_view_total_count -= 1;
            if is_empty {
                self.reusable_views_by_identifier.remove(&identifier);
            }
        }
    }

    fn flush_deferred_geometry_if_possible(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let can_apply_during_scroll = self.pending_geometry_allows_application_during_scroll && !self.pending_item_relayout;
        if !self.pending_geometry_refresh
            || !self.has_usable_viewport()
            || self.pending_transaction.is_some()
            || self.is_applying_transaction
            || (self.is_user_scrolling() && !can_apply_during_scroll)
        {
            return;
        }

        let keeps_bottom_anchor = self.pending_geometry_keeps_bottom_anchor;
        let keeps_top_anchor = self.pending_geometry_keeps_top_anchor && !keeps_bottom_anchor;
        let needs_item_relayout = self.pending_item_relayout;
        self.pending_geometry_refresh = false;
        self.pending_geometry_keeps_bottom_anchor = false;
        self.pending_geometry_keeps_top_anchor = false;
        self.pending_item_relayout = false;
        self.pending_geometry_allows_application_during_scroll = false;
        if needs_item_relayout {
            self.relayout_current_items(keeps_bottom_anchor, keeps_top_anchor, window, cx);
        } else {
            self.refresh_scroll_geometry(keeps_bottom_anchor, keeps_top_anchor, window, cx);
        }
    }

    fn refresh_scroll_geometry(
        &mut self,
        keeps_bottom_anchor: bool,
        keeps_top_anchor: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_origin_y = self.layout_origin_y - self.scroll_view.content_offset.y;
        let previous_presentation_translation_y = self.current_viewport_anchor_presentation_translation_y();
        if keeps_bottom_anchor {
            self.layout_origin_y = self.scroll_view.content_offset.y + self.bounds.size.height
                - self.viewport_insets.bottom
                - self.layout_height;
        } else if keeps_top_anchor {
            self.layout_origin_y = self.scroll_view.content_offset.y + self.viewport_insets.top;
        }
        self.perform_without_animations(|this| {
            this.update_host_frame();
            this.update_scroll_geometry();
        });
        self.update_mounted_items(HashSet::new(), window, cx);
        self.notify_scroll_metrics_changed(window, cx);
        if keeps_bottom_anchor {
            self.animate_viewport_anchor_transition(previous_origin_y, previous_presentation_translation_y, window, cx);
        }
    }

    fn relayout_current_items(
        &mut self,
        keeps_bottom_anchor: bool,
        keeps_top_anchor: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_origin_y = self.layout_origin_y - self.scroll_view.content_offset.y;
        let previous_presentation_translation_y = self.current_viewport_anchor_presentation_translation_y();
        let anchor = self.visible_anchor();
        let width = self.layout_items.first().map_or(self.bounds.size.width, |first| first.borrow().frame.size.width);
        let mut next_y = Pixels::ZERO;
        for (index, item) in self.layout_items.iter().enumerate() {
            if index > 0 {
                next_y += self.inter_item_spacing;
            }
            let mut item = item.borrow_mut();
            let height = px(item.content.height);
            item.frame = Bounds::new(point(Pixels::ZERO, next_y), size(width, height));
            next_y += height;
        }
        self.layout_coordinate_offset_y = Pixels::ZERO;
        self.layout_height = next_y;

        if keeps_bottom_anchor {
            self.layout_origin_y =
                self.scroll_view.content_offset.y + self.bounds.size.height - self.viewport_insets.bottom - next_y;
        } else if keeps_top_anchor {
            self.layout_origin_y = self.scroll_view.content_offset.y + self.viewport_insets.top;
        } else if let Some(anchor) = anchor
            && let Some(item) = self.item_by_id.get(&anchor.id)
        {
            self.layout_origin_y = anchor.absolute_min_y - self.resolved_frame(&item.borrow()).origin.y;
        }
        self.finish_applied_layout(HashSet::new(), None, None, HashSet::new(), window, cx);
        if keeps_bottom_anchor {
            self.animate_viewport_anchor_transition(previous_origin_y, previous_presentation_translation_y, window, cx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_visible_viewport_pass(
        &mut self,
        preparation_generation: u64,
        expected_layout_generation: u64,
        expected_size: Size<Pixels>,
        pass: usize,
        completion: RiseManualListPreparedViewportCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if preparation_generation != self.visible_viewport_preparation_generation
            || expected_layout_generation != self.layout_generation
            || (expected_size.width - self.bounds.size.width).abs() > px(0.5)
            || (expected_size.height - self.bounds.size.height).abs() > px(0.5)
            || self.pending_transaction.is_some()
            || self.is_applying_transaction
            || self.is_user_scrolling()
        {
            return;
        }

        self.visible_viewport_preparation_depth += 1;
        self.update_mounted_items(HashSet::new(), window, cx);
        let range = self.current_visible_item_range();
        self.perform_without_animations(|this| {
            this.synchronize_visible_items(range.clone(), window, cx);
            this.layout_mounted_views(range.clone(), cx);
        });
        let is_current = self.visible_items_are_current_and_mounted(range.clone());
        self.visible_viewport_preparation_depth -= 1;
        if !is_current {
            self.retry_visible_viewport_preparation(
                preparation_generation,
                expected_layout_generation,
                expected_size,
                pass,
                completion,
                window,
                cx,
            );
            return;
        }

        let expected_offset_y = self.scroll_view.content_offset.y;
        cx.notify();
        cx.on_next_frame(window, move |this, window, cx| {
            this.finish_visible_viewport_preparation(
                preparation_generation,
                expected_layout_generation,
                expected_size,
                expected_offset_y,
                pass,
                completion,
                window,
                cx,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_visible_viewport_preparation(
        &mut self,
        preparation_generation: u64,
        expected_layout_generation: u64,
        expected_size: Size<Pixels>,
        expected_offset_y: Pixels,
        pass: usize,
        completion: RiseManualListPreparedViewportCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if preparation_generation != self.visible_viewport_preparation_generation
            || expected_layout_generation != self.layout_generation
            || (expected_size.width - self.bounds.size.width).abs() > px(0.5)
            || (expected_size.height - self.bounds.size.height).abs() > px(0.5)
            || self.pending_transaction.is_some()
            || self.is_applying_transaction
            || self.is_user_scrolling()
        {
            return;
        }

        let range = self.current_visible_item_range();
        self.perform_without_animations(|this| {
            this.synchronize_visible_items(range.clone(), window, cx);
            this.layout_mounted_views(range.clone(), cx);
        });
        if (self.scroll_view.content_offset.y - expected_offset_y).abs() > px(0.5)
            || !self.visible_items_are_current_and_mounted(range.clone())
        {
            self.retry_visible_viewport_preparation(
                preparation_generation,
                expected_layout_generation,
                expected_size,
                pass,
                completion,
                window,
                cx,
            );
            return;
        }

        self.update_visible_ids(true, window, cx);
        let snapshot = self.prepared_viewport_snapshot(range, cx);
        completion(snapshot, window, cx);
    }

    fn prepared_viewport_snapshot(&self, range: Range<usize>, cx: &mut App) -> RiseManualListPreparedViewport {
        let maximum_measurements_per_pass = RiseManualListViewHooks::maximum_measurements_per_pass_or_default();
        let mut mismatches: HashMap<String, Pixels> = HashMap::with_capacity(range.len().min(maximum_measurements_per_pass));
        let mut measurement_count = 0usize;
        for index in RiseManualListViewHooks::prioritized_indices_or_default(range.clone()) {
            let id = self.layout_items[index].borrow().id.clone();
            let Some(validating_view) = self.mounted_items.get(&id).map(|mounted| mounted.view.clone()) else {
                continue;
            };
            if RiseManualListConformances::needs_prepared_viewport_height_validation(&validating_view, cx) != Some(true)
                || measurement_count >= maximum_measurements_per_pass
            {
                continue;
            }
            measurement_count += 1;
            if let Some(height) = RiseManualListConformances::prepared_viewport_natural_height_mismatch(&validating_view, cx) {
                mismatches.insert(id, height);
            }
        }
        RiseManualListPreparedViewport {
            layout_generation: self.layout_generation,
            visible_ids: range.map(|index| self.layout_items[index].borrow().id.clone()).collect(),
            content_offset_y: self.scroll_view.content_offset.y,
            natural_height_mismatches: mismatches,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn retry_visible_viewport_preparation(
        &mut self,
        preparation_generation: u64,
        expected_layout_generation: u64,
        expected_size: Size<Pixels>,
        pass: usize,
        completion: RiseManualListPreparedViewportCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if pass >= 1 {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let _ = this.update_in(cx, |this, window, cx| {
                this.prepare_visible_viewport_pass(
                    preparation_generation,
                    expected_layout_generation,
                    expected_size,
                    pass + 1,
                    completion,
                    window,
                    cx,
                );
            });
        })
        .detach();
    }

    fn current_visible_item_range(&mut self) -> Range<usize> {
        let key = RiseManualListViewVisibleItemRangeCacheKey {
            layout_generation: self.layout_generation,
            item_count: self.layout_items.count(),
            content_offset_y: self.scroll_view.content_offset.y,
            layout_origin_y: self.layout_origin_y,
            layout_coordinate_offset_y: self.layout_coordinate_offset_y,
            viewport_height: self.bounds.size.height,
        };
        if let Some((cached_key, cached_range)) = self.cached_visible_item_range.as_ref()
            && *cached_key == key
        {
            return cached_range.clone();
        }

        let local_minimum_y = key.content_offset_y - key.layout_origin_y;
        let range = self.item_range(local_minimum_y, local_minimum_y + key.viewport_height);
        #[cfg(debug_assertions)]
        {
            self.visible_item_range_computation_count_for_testing += 1;
        }
        self.cached_visible_item_range = Some((key, range.clone()));
        range
    }

    /// layout pass on each mounted row in the range
    fn layout_mounted_views(&self, range: Range<usize>, cx: &mut App) {
        for index in range {
            let id = self.layout_items[index].borrow().id.clone();
            let Some(mounted) = self.mounted_items.get(&id) else {
                continue;
            };
            if RiseManualListConformances::caches_row(&mounted.view, cx) {
                continue;
            }
            cx.notify(mounted.view.entity_id());
        }
    }

    fn synchronize_visible_items_if_needed(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let has_deferred_reconfigurations = !self.deferred_reconfigure_ids.is_empty();
        for index in range.clone() {
            let item_ref = self.layout_items[index].clone();
            let item = item_ref.borrow();
            let is_current = self.mounted_items.get(&item.id).is_some_and(|mounted| {
                !mounted.is_placeholder
                    && (!has_deferred_reconfigurations || !self.deferred_reconfigure_ids.contains(&item.id))
                    && mounted.content_version == item.content.content_version
                    && mounted.reuse_identifier == item.content.reuse_identifier
                    && mounted.layout_generation == self.layout_generation
                    && mounted.frame == self.mounted_frame(&item)
                    && mounted.host == self.mounted_host_view(&item)
            });
            if !is_current {
                drop(item);
                self.perform_without_animations(|this| this.synchronize_visible_items(range, window, cx));
                #[cfg(debug_assertions)]
                {
                    self.visible_synchronization_pass_count_for_testing += 1;
                }
                return;
            }
        }
    }

    fn synchronize_visible_items(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "list.synchronize_visible",
            HashMap::from([("count".to_string(), range.len().to_string())]),
        );
        for index in range {
            let item_ref = self.layout_items[index].clone();
            let item = item_ref.borrow();
            if let Some(mounted) = self.mounted_items.get(&item.id).cloned() {
                if mounted.is_placeholder
                    || self.deferred_reconfigure_ids.contains(&item.id)
                    || mounted.content_version != item.content.content_version
                    || mounted.reuse_identifier != item.content.reuse_identifier
                {
                    self.configure_mounted_view(mounted, &item, window, cx);
                    self.deferred_reconfigure_ids.remove(&item.id);
                } else if mounted.layout_generation != self.layout_generation
                    || mounted.frame != self.mounted_frame(&item)
                    || mounted.host != self.mounted_host_view(&item)
                {
                    self.update_mounted_frame(mounted, &item, cx);
                }
            } else {
                self.mount(&item, window, cx);
                self.deferred_reconfigure_ids.remove(&item.id);
            }
        }
        ChatPerformanceTrace::shared().end_operation(trace_token, 1.0, HashMap::new());
    }

    fn visible_items_are_current_and_mounted(&self, range: Range<usize>) -> bool {
        for index in range {
            let item = self.layout_items[index].borrow();
            let Some(mounted) = self.mounted_items.get(&item.id) else {
                return false;
            };
            if mounted.is_placeholder
                || mounted.layout_generation != self.layout_generation
                || mounted.content_version != item.content.content_version
                || mounted.reuse_identifier != item.content.reuse_identifier
                || mounted.host != self.mounted_host_view(&item)
            {
                return false;
            }
            let expected_frame = self.mounted_frame(&item);
            if (mounted.frame.origin.x - expected_frame.origin.x).abs() > px(0.5)
                || (mounted.frame.origin.y - expected_frame.origin.y).abs() > px(0.5)
                || (mounted.frame.size.width - expected_frame.size.width).abs() > px(0.5)
                || (mounted.frame.size.height - expected_frame.size.height).abs() > px(0.5)
            {
                return false;
            }
        }
        true
    }

    fn visible_anchor(&self) -> Option<RiseManualListViewVisibleAnchor> {
        self.visible_anchor_matching(|_| true)
    }

    fn visible_anchor_retained_by(&self, ids: &HashSet<String>) -> Option<RiseManualListViewVisibleAnchor> {
        self.visible_anchor_matching(|item| ids.contains(&item.id))
    }

    fn visible_anchor_excluding(&self, ids: &HashSet<String>) -> Option<RiseManualListViewVisibleAnchor> {
        self.visible_anchor_matching(|item| !ids.contains(&item.id))
    }

    fn visible_anchor_matching(
        &self,
        predicate: impl Fn(&RiseManualListViewLayoutItem) -> bool,
    ) -> Option<RiseManualListViewVisibleAnchor> {
        if self.layout_items.is_empty() {
            return None;
        }

        let local_top = self.scroll_view.content_offset.y - self.layout_origin_y;
        let visible_range = self.item_range(local_top, local_top + self.bounds.size.height);
        for index in visible_range {
            let item = self.layout_items[index].borrow();
            if predicate(&item) {
                return Some(RiseManualListViewVisibleAnchor {
                    id: item.id.clone(),
                    absolute_min_y: self.layout_origin_y + self.resolved_frame(&item).origin.y,
                });
            }
        }

        None
    }

    /// viewport inset by the chrome, it gates animated media playback
    /// raw bounds counted a sticker hidden behind the composer as visible and kept decoding frames
    fn animated_media_viewport(&self) -> Bounds<Pixels> {
        Bounds::new(
            point(
                Pixels::ZERO,
                self.scroll_view.content_offset.y - self.layout_origin_y + self.viewport_insets.top,
            ),
            size(
                self.bounds.size.width,
                (self.bounds.size.height - self.viewport_insets.top - self.viewport_insets.bottom).max(Pixels::ZERO),
            ),
        )
    }

    fn current_readable_item_range(&self) -> Range<usize> {
        let viewport = self.animated_media_viewport();
        if viewport.is_empty() {
            return 0..0;
        }
        let mut range = self.item_range(viewport.top(), viewport.bottom());
        while range.start < range.end && !self.is_readable(range.start, viewport) {
            range.start += 1;
        }
        while range.start < range.end && !self.is_readable(range.end - 1, viewport) {
            range.end -= 1;
        }
        range
    }

    fn is_readable(&self, index: usize, viewport: Bounds<Pixels>) -> bool {
        let frame = self.resolved_frame(&self.layout_items[index].borrow());
        let shown = frame.bottom().min(viewport.bottom()) - frame.top().max(viewport.top());
        shown > Pixels::ZERO && shown >= frame.size.height.min(viewport.size.height) * Self::READABLE_ROW_SHARE
    }

    fn is_frame_visible(&self, frame: Bounds<Pixels>) -> bool {
        let viewport = self.animated_media_viewport();
        !viewport.is_empty() && frame.intersects(&viewport)
    }

    fn update_performance_trace_context(&self, phase: Option<&str>) {
        let trace = ChatPerformanceTrace::shared();
        if !trace.is_enabled() {
            return;
        }
        let phase = match phase {
            Some(override_phase) => override_phase,
            None if self.scroll_view.is_tracking || self.scroll_view.is_dragging => "dragging",
            None if self.scroll_view.is_decelerating || self.is_programmatic_scroll_animating => "decelerating",
            None => "settling",
        };
        let placeholders = self.mounted_items.values().filter(|mounted| mounted.is_placeholder).count();
        trace.update_scroll_context(
            phase,
            f32::from(self.scroll_view.content_offset.y),
            f32::from(self.effective_estimated_scroll_velocity_y()),
            self.visible_ids.len() as i64,
            self.mounted_items.len() as i64,
            placeholders as i64,
        );
    }

    fn item_range(&self, lower: Pixels, upper: Pixels) -> Range<usize> {
        let lower_bound = self.first_item_index_ending_after(lower);
        let upper_bound = lower_bound.max(self.first_item_index_starting_at_or_after(upper));
        lower_bound..upper_bound
    }

    fn first_item_index_ending_after(&self, y: Pixels) -> usize {
        let mut lower_bound = 0;
        let mut upper_bound = self.layout_items.count();
        while lower_bound < upper_bound {
            let middle = lower_bound + (upper_bound - lower_bound) / 2;
            if self.resolved_frame(&self.layout_items[middle].borrow()).bottom() <= y {
                lower_bound = middle + 1;
            } else {
                upper_bound = middle;
            }
        }
        lower_bound
    }

    fn first_item_index_starting_at_or_after(&self, y: Pixels) -> usize {
        let mut lower_bound = 0;
        let mut upper_bound = self.layout_items.count();
        while lower_bound < upper_bound {
            let middle = lower_bound + (upper_bound - lower_bound) / 2;
            if self.resolved_frame(&self.layout_items[middle].borrow()).top() < y {
                lower_bound = middle + 1;
            } else {
                upper_bound = middle;
            }
        }
        lower_bound
    }

    fn resolved_frame(&self, item: &RiseManualListViewLayoutItem) -> Bounds<Pixels> {
        Bounds::new(
            point(item.frame.origin.x, item.frame.origin.y + self.layout_coordinate_offset_y),
            item.frame.size,
        )
    }

    fn mounted_frame(&self, item: &RiseManualListViewLayoutItem) -> Bounds<Pixels> {
        self.top_sticky_item_frames.get(&item.id).copied().unwrap_or_else(|| self.resolved_frame(item))
    }

    fn mounted_host_view(&self, item: &RiseManualListViewLayoutItem) -> RiseManualListViewUpdateTransitionHost {
        if self.top_sticky_item_frames.contains_key(&item.id) {
            RiseManualListViewUpdateTransitionHost::TopSticky
        } else {
            RiseManualListViewUpdateTransitionHost::Item
        }
    }

    /// runs updates inside a transaction with actions disabled, so no animation
    fn perform_without_animations(&mut self, updates: impl FnOnce(&mut Self)) {
        updates(self);
    }
}

impl RiseManualListView {
    fn update_visible_ids(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let range = self.current_visible_item_range();
        let media_viewport = self.animated_media_viewport();
        let media_range = if media_viewport.is_empty() {
            0..0
        } else {
            self.item_range(media_viewport.top(), media_viewport.bottom())
        };
        if force
            || self.animated_media_visible_generation != self.layout_generation
            || self.animated_media_visible_range != media_range
        {
            let mounted: Vec<(String, AnyView, bool)> = self
                .mounted_items
                .iter()
                .map(|(id, mounted)| (id.clone(), mounted.view.clone(), mounted.is_placeholder))
                .collect();
            for (id, view, is_placeholder) in mounted {
                let Some(item) = self.item_by_id.get(&id).cloned() else {
                    continue;
                };
                if is_placeholder {
                    RiseManualListConformances::set_animated_media_visible_in_viewport(&view, false, cx);
                } else {
                    self.apply_animated_media_viewport_visibility(&view, &item.borrow(), cx);
                }
            }
            self.animated_media_visible_range = media_range;
            self.animated_media_visible_generation = self.layout_generation;
        }
        let next_readable_range = self.current_readable_item_range();
        if force || self.readable_generation != self.layout_generation || self.readable_range != next_readable_range {
            self.readable_range = next_readable_range.clone();
            self.readable_generation = self.layout_generation;
            let next_readable_ids: Vec<String> =
                next_readable_range.map(|index| self.layout_items[index].borrow().id.clone()).collect();
            if next_readable_ids != self.readable_ids {
                self.readable_ids = next_readable_ids.clone();
                if let Some(on_readable_ids_changed) = self.on_readable_ids_changed.clone() {
                    on_readable_ids_changed(&next_readable_ids, window, cx);
                }
            }
        }
        if !force && self.visible_generation == self.layout_generation && self.visible_range == range {
            return;
        }
        let next_visible_ids: Vec<String> =
            range.clone().map(|index| self.layout_items[index].borrow().id.clone()).collect();
        self.visible_range = range;
        self.visible_generation = self.layout_generation;
        if next_visible_ids == self.visible_ids {
            return;
        }
        self.visible_ids = next_visible_ids.clone();
        self.refresh_selection_accessibility();
        if let Some(on_visible_ids_changed) = self.on_visible_ids_changed.clone() {
            on_visible_ids_changed(&next_visible_ids, window, cx);
        }
    }

    fn is_at_bottom(&self, viewport_height: Pixels, bottom_inset: Option<Pixels>) -> bool {
        if viewport_height <= Pixels::ZERO || self.layout_items.is_empty() {
            return false;
        }
        let bottom_offset_y = self.layout_origin_y + self.layout_height - viewport_height
            + bottom_inset.unwrap_or(self.viewport_insets.bottom);
        (self.scroll_view.content_offset.y - bottom_offset_y).abs() <= px(2.0)
    }

    /// rows shorter than the viewport have one resting place, the bottom edge
    /// the 2 pt offset tolerance lost that anchor on an inset change
    fn content_fits_viewport(&self, viewport_height: Pixels, bottom_inset: Pixels) -> bool {
        if self.rests_short_content_at_top || viewport_height <= Pixels::ZERO || self.layout_items.is_empty() {
            return false;
        }
        self.layout_height + self.viewport_insets.top + bottom_inset <= viewport_height + px(0.5)
    }

    fn is_at_top(&self, top_inset: Option<Pixels>) -> bool {
        if self.layout_items.is_empty() {
            return false;
        }
        let top_offset_y = self.layout_origin_y - top_inset.unwrap_or(self.viewport_insets.top);
        (self.scroll_view.content_offset.y - top_offset_y).abs() <= px(2.0)
    }

    fn notify_scroll_metrics_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mirror_integration_scroll_offset();
        self.update_sticky_headers(window, cx);
        self.debug_sample_row_jumps(cx);
        if self.visible_viewport_preparation_depth != 0 {
            return;
        }
        if let Some(on_scroll_metrics_changed) = self.on_scroll_metrics_changed.clone() {
            on_scroll_metrics_changed(&self.scroll_metrics(), window, cx);
        }
    }

    fn debug_sample_row_jumps(&self, cx: &mut Context<Self>) {
        if !RiseListJumpDebug::is_enabled() || !self.has_usable_viewport() {
            return;
        }
        let now = current_media_time();
        let mut rows: Vec<RiseListJumpDebugRow> = Vec::with_capacity(self.mounted_items.len());
        for (id, mounted) in &self.mounted_items {
            let Some(item) = self.item_by_id.get(id) else {
                continue;
            };
            let model_frame = self.resolved_frame(&item.borrow());
            let view_frame = mounted.frame;
            let motion = self.update_transition_motions.get(&mounted.view.entity_id());
            let presentation_offset_y = motion.map_or(Pixels::ZERO, |motion| motion.presentation_offset(now).y);
            rows.push(RiseListJumpDebugRow {
                id: id.clone(),
                model_min_y: f32::from(model_frame.top()),
                view_min_y: f32::from(view_frame.top()),
                presentation_min_y: Some(f32::from(view_frame.top() + presentation_offset_y)),
                animation_count: i64::from(motion.is_some_and(|motion| motion.is_running(now))),
                height: f32::from(view_frame.size.height),
                transform_y: f32::from(presentation_offset_y),
            });
        }

        let phase = if self.scroll_view.is_tracking || self.scroll_view.is_dragging {
            "drag"
        } else if self.scroll_view.is_decelerating || self.is_programmatic_scroll_animating {
            "decel"
        } else {
            "idle"
        };

        let host = &self.item_host_view;
        let host_animation_count = [
            host.translation_animation.is_some(),
            host.alpha_animation.is_some(),
            host.frame_animation.is_some(),
        ]
        .into_iter()
        .filter(|is_present| *is_present)
        .count();
        RiseListJumpDebug::sample(
            cx.entity_id(),
            &rows,
            f32::from(self.scroll_view.content_offset.y),
            0.0,
            f32::from(self.layout_origin_y),
            f32::from(self.layout_coordinate_offset_y),
            f32::from(host.translation_y),
            f32::from(host.presentation_frame(now).top() - host.frame.top()),
            host_animation_count as i64,
            self.layout_generation,
            f32::from(self.bounds.size.height - self.viewport_insets.bottom),
            phase,
            cx,
        );
    }

    fn update_sticky_headers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(keyboard_sticky_plan) = self.keyboard_sticky_plan.as_ref() {
            let signature = self.current_keyboard_sticky_signature();
            if keyboard_sticky_plan.signature == signature {
                return;
            }
            let reason = keyboard_sticky_plan.signature.changes(&signature);
            if self.plan_keyboard_sticky_motion(&reason) {
                return;
            }
            self.start_keyboard_sticky_tracking(window, cx);
        }
        self.update_top_sticky_headers(window, cx);
        self.update_bottom_sticky_headers(window, cx);
    }

    fn update_top_sticky_headers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_usable_viewport() || self.layout_items.is_empty() {
            self.restore_all_top_sticky_items(window, cx);
            return;
        }

        self.rebuild_sticky_header_ranges_if_needed();
        if self.top_sticky_header_ranges.is_empty() {
            self.restore_all_top_sticky_items(window, cx);
            return;
        }
        self.layout_sticky_header_host_views();

        let live_translation_y = self.live_keyboard_content_translation_y();
        let visible_range = self.keyboard_visible_item_range(live_translation_y);
        if visible_range.is_empty() {
            self.restore_all_top_sticky_items(window, cx);
            return;
        }

        let content_offset_y = self.scroll_view.content_offset.y - live_translation_y;
        let viewport_top = self
            .bounds
            .size
            .height
            .min(self.top_sticky_viewport_inset.unwrap_or(self.viewport_insets.top));
        let viewport_bottom = viewport_top.max(self.bounds.size.height - self.viewport_insets.bottom);
        let mut visible_frames = std::mem::take(&mut self.visible_top_sticky_item_frames_buffer);
        let mut pending_restore = std::mem::take(&mut self.top_sticky_item_ids_pending_restore);
        visible_frames.clear();
        pending_restore.clear();

        let mut range_index = self.first_top_sticky_header_range_index(visible_range.start);
        while range_index < self.top_sticky_header_ranges.len() {
            let range = self.top_sticky_header_ranges[range_index];
            if range.first_index >= visible_range.end {
                break;
            }

            let first_item = self.layout_items[range.first_index].clone();
            let first_item = first_item.borrow();
            let last_item = self.layout_items[range.last_index].clone();
            let last_item = last_item.borrow();
            let first_frame = self.resolved_frame(&first_item);
            let group_top = self.layout_origin_y + first_frame.top() - content_offset_y;
            let group_bottom = self.layout_origin_y + self.resolved_frame(&last_item).bottom() - content_offset_y;

            if group_bottom > viewport_top
                && group_top < viewport_bottom
                && !self.insert_travel_row_ids.contains(&first_item.id)
            {
                let header_height = first_item.content.height;
                let y = px(RiseManualListTopStickyHeaderLayout::origin_y(
                    f32::from(group_top),
                    f32::from(group_bottom),
                    f32::from(viewport_top),
                    header_height,
                ));
                visible_frames.insert(
                    first_item.id.clone(),
                    Bounds::new(point(Pixels::ZERO, y), size(first_frame.size.width, px(header_height))),
                );
                if (y - group_top).abs() > px(0.5) {
                    self.pinned_top_sticky_item_ids.insert(first_item.id.clone());
                } else {
                    self.pinned_top_sticky_item_ids.remove(&first_item.id);
                }
                if (y - group_top).abs() > px(header_height * 0.5) {
                    self.detached_top_sticky_item_ids.insert(first_item.id.clone());
                } else {
                    self.detached_top_sticky_item_ids.remove(&first_item.id);
                }
            }
            range_index += 1;
        }

        for id in self.top_sticky_item_frames.keys() {
            if !visible_frames.contains_key(id) {
                pending_restore.push(id.clone());
            }
        }
        for id in &pending_restore {
            self.restore_top_sticky_item(id, cx);
        }

        for (id, frame) in &visible_frames {
            self.mount_or_update_top_sticky_item(id, *frame, window, cx);
        }
        visible_frames.clear();
        pending_restore.clear();
        self.visible_top_sticky_item_frames_buffer = visible_frames;
        self.top_sticky_item_ids_pending_restore = pending_restore;
        self.apply_top_sticky_fade(cx);
    }

    fn is_top_sticky_item_faded(&self, id: &str) -> bool {
        !self.top_sticky_headers_flashing && self.detached_top_sticky_item_ids.contains(id)
    }

    fn apply_top_sticky_fade(&mut self, cx: &mut Context<Self>) {
        let faded_views: Vec<(AnyView, bool)> = self
            .top_sticky_item_frames
            .keys()
            .filter_map(|id| Some((self.mounted_items.get(id)?.view.clone(), self.is_top_sticky_item_faded(id))))
            .collect();
        for (view, faded) in faded_views {
            RiseManualListConformances::set_top_sticky_header_faded(&view, faded, true, cx);
        }
    }

    /// called on every frame of a moving list, only the first one changes anything
    fn flash_top_sticky_headers(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.top_sticky_flash_generation = self.top_sticky_flash_generation.wrapping_add(1);
        if self.top_sticky_headers_flashing {
            return;
        }
        self.top_sticky_headers_flashing = true;
        self.apply_top_sticky_fade(cx);
    }

    fn schedule_top_sticky_flash_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.top_sticky_flash_generation = self.top_sticky_flash_generation.wrapping_add(1);
        let generation = self.top_sticky_flash_generation;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(Self::TOP_STICKY_FLASH_LINGER)).await;
            let _ = this.update(cx, |this, cx| {
                if this.top_sticky_flash_generation != generation || this.is_user_scrolling() {
                    return;
                }
                this.top_sticky_headers_flashing = false;
                this.apply_top_sticky_fade(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn update_bottom_sticky_headers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_usable_viewport() || self.layout_items.is_empty() {
            self.unmount_all_bottom_sticky_headers(cx);
            return;
        }

        self.rebuild_sticky_header_ranges_if_needed();
        if self.bottom_sticky_header_ranges.is_empty() {
            if !self.mounted_bottom_sticky_headers.is_empty() {
                self.unmount_all_bottom_sticky_headers(cx);
            }
            return;
        }
        self.layout_sticky_header_host_views();

        let follows_keyboard = self.keyboard_viewport_transition_is_active;
        let live_translation_y =
            if follows_keyboard { self.live_keyboard_content_translation_y() } else { Pixels::ZERO };
        let visible_range = self.keyboard_visible_item_range(live_translation_y);
        if visible_range.is_empty() {
            self.unmount_all_bottom_sticky_headers(cx);
            return;
        }

        let content_offset_y = self.scroll_view.content_offset.y - live_translation_y;
        let sticky_bottom_inset = if follows_keyboard {
            self.live_keyboard_sticky_bottom_inset()
        } else {
            self.bottom_sticky_viewport_inset.unwrap_or(self.viewport_insets.bottom)
        };
        let viewport_bottom = (self.bounds.size.height - sticky_bottom_inset).max(Pixels::ZERO);
        let mut visible_headers = std::mem::take(&mut self.visible_bottom_sticky_headers_buffer);
        let mut pending_unmount = std::mem::take(&mut self.bottom_sticky_header_ids_pending_unmount);
        visible_headers.clear();
        pending_unmount.clear();
        let mut index = visible_range.start;

        while index < visible_range.end {
            let Some(descriptor) = self.layout_items[index].borrow().content.bottom_sticky_header.clone() else {
                index += 1;
                continue;
            };
            let Some(range) = self.bottom_sticky_header_ranges.get(&descriptor.id).copied() else {
                index += 1;
                continue;
            };
            let first_index = range.first_index;
            let last_index = range.last_index;

            let first_item = self.layout_items[first_index].clone();
            let first_item = first_item.borrow();
            let last_item = self.layout_items[last_index].clone();
            let last_item = last_item.borrow();
            let first_top_inset =
                first_item.content.bottom_sticky_header.as_ref().map_or(descriptor.top_inset, |header| header.top_inset);
            let last_bottom_inset = last_item
                .content
                .bottom_sticky_header
                .as_ref()
                .map_or(descriptor.bottom_inset, |header| header.bottom_inset);
            let group_top =
                self.layout_origin_y + self.resolved_frame(&first_item).top() - content_offset_y + px(first_top_inset);
            let group_bottom =
                self.layout_origin_y + self.resolved_frame(&last_item).bottom() - content_offset_y - px(last_bottom_inset);

            if group_bottom > Pixels::ZERO && group_top < viewport_bottom {
                let y = px(RiseManualListBottomStickyHeaderLayout::origin_y(
                    f32::from(group_top),
                    f32::from(group_bottom),
                    f32::from(viewport_bottom),
                    f32::from(descriptor.size.height),
                ));
                let x = if self.effective_layout_direction_is_left_to_right {
                    px(descriptor.leading_inset)
                } else {
                    self.bounds.size.width - px(descriptor.leading_inset) - descriptor.size.width
                };
                let frame = Bounds::new(point(x, y), descriptor.size);
                visible_headers.insert(
                    descriptor.id.clone(),
                    RiseManualListViewVisibleBottomStickyHeader { descriptor, frame },
                );
            }
            index = last_index + 1;
        }

        for id in self.mounted_bottom_sticky_headers.keys() {
            if !visible_headers.contains_key(id) {
                pending_unmount.push(id.clone());
            }
        }
        for id in &pending_unmount {
            self.unmount_bottom_sticky_header(id, cx);
        }

        for (id, visible_header) in &visible_headers {
            self.mount_or_update_bottom_sticky_header(id, visible_header, window, cx);
        }
        visible_headers.clear();
        pending_unmount.clear();
        self.visible_bottom_sticky_headers_buffer = visible_headers;
        self.bottom_sticky_header_ids_pending_unmount = pending_unmount;
    }

    fn rebuild_sticky_header_ranges_if_needed(&mut self) {
        if self.sticky_header_ranges_generation == Some(self.layout_generation) {
            return;
        }
        self.sticky_header_ranges_generation = Some(self.layout_generation);
        self.top_sticky_header_ranges.clear();
        self.bottom_sticky_header_ranges.clear();

        let count = self.layout_items.count();
        let mut active_top_header_index: Option<usize> = None;
        let mut index = 0;
        while index < count {
            if self.layout_items[index].borrow().content.top_sticky_header.is_some() {
                if let Some(active_top_header_index) = active_top_header_index {
                    self.top_sticky_header_ranges.push(RiseManualListViewTopStickyHeaderRange {
                        first_index: active_top_header_index,
                        last_index: index - 1,
                    });
                }
                active_top_header_index = Some(index);
            }

            let Some(header_id) =
                self.layout_items[index].borrow().content.bottom_sticky_header.as_ref().map(|header| header.id.clone())
            else {
                index += 1;
                continue;
            };
            let first_index = index;
            index += 1;
            while index < count
                && self.layout_items[index]
                    .borrow()
                    .content
                    .bottom_sticky_header
                    .as_ref()
                    .is_some_and(|header| header.id == header_id)
            {
                index += 1;
            }
            self.bottom_sticky_header_ranges.insert(
                header_id,
                RiseManualListViewBottomStickyHeaderRange { first_index, last_index: index - 1 },
            );
        }

        if let Some(active_top_header_index) = active_top_header_index {
            self.top_sticky_header_ranges.push(RiseManualListViewTopStickyHeaderRange {
                first_index: active_top_header_index,
                last_index: count - 1,
            });
        }
    }

    fn first_top_sticky_header_range_index(&self, ending_at_or_after: usize) -> usize {
        let mut lower_bound = 0;
        let mut upper_bound = self.top_sticky_header_ranges.len();
        while lower_bound < upper_bound {
            let middle = lower_bound + (upper_bound - lower_bound) / 2;
            if self.top_sticky_header_ranges[middle].last_index < ending_at_or_after {
                lower_bound = middle + 1;
            } else {
                upper_bound = middle;
            }
        }
        lower_bound
    }

    fn mount_or_update_top_sticky_item(
        &mut self,
        id: &str,
        frame: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.item_by_id.get(id).cloned() else {
            return;
        };
        let item = item.borrow();
        if !self.mounted_items.contains_key(id) && self.keyboard_viewport_transition_is_active {
            return;
        }
        self.top_sticky_item_frames.insert(id.to_string(), frame);

        let Some(mounted) = self.mounted_items.get(id).cloned() else {
            self.mount(&item, window, cx);
            return;
        };
        if self.keyboard_viewport_transition_is_active
            && !mounted.is_placeholder
            && mounted.layout_generation == self.layout_generation
            && mounted.content_version == item.content.content_version
            && mounted.reuse_identifier == item.content.reuse_identifier
            && mounted.host == RiseManualListViewUpdateTransitionHost::TopSticky
        {
            if mounted.frame != frame
                && let Some(mounted) = self.mounted_items.get_mut(id)
            {
                mounted.frame = frame;
            }
            return;
        }
        if mounted.is_placeholder
            || mounted.content_version != item.content.content_version
            || mounted.reuse_identifier != item.content.reuse_identifier
        {
            self.configure_mounted_view(mounted, &item, window, cx);
        } else if mounted.layout_generation != self.layout_generation
            || mounted.frame != frame
            || mounted.host != RiseManualListViewUpdateTransitionHost::TopSticky
        {
            self.update_mounted_frame(mounted, &item, cx);
        }
    }

    fn restore_top_sticky_item(&mut self, id: &str, cx: &mut Context<Self>) {
        self.pinned_top_sticky_item_ids.remove(id);
        self.detached_top_sticky_item_ids.remove(id);
        if self.top_sticky_item_frames.remove(id).is_none() {
            return;
        }
        let Some(item) = self.item_by_id.get(id).cloned() else {
            return;
        };
        let Some(mounted) = self.mounted_items.get(id).cloned() else {
            return;
        };
        RiseManualListConformances::set_top_sticky_header_faded(&mounted.view, false, false, cx);
        self.update_mounted_frame(mounted, &item.borrow(), cx);
    }

    fn restore_all_top_sticky_items(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.top_sticky_item_frames.keys().cloned().collect();
        for id in ids {
            self.restore_top_sticky_item(&id, cx);
        }
    }

    fn mount_or_update_bottom_sticky_header(
        &mut self,
        id: &str,
        visible_header: &RiseManualListViewVisibleBottomStickyHeader,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let descriptor = &visible_header.descriptor;
        if let Some(mounted) = self.mounted_bottom_sticky_headers.get_mut(id)
            && mounted.reuse_identifier == descriptor.reuse_identifier
            && mounted.content_version == descriptor.content_version
        {
            if mounted.frame != visible_header.frame {
                mounted.frame = visible_header.frame;
            }
            return;
        }

        let reusable_view = if let Some(mounted) = self.mounted_bottom_sticky_headers.remove(id) {
            if mounted.reuse_identifier == descriptor.reuse_identifier {
                Some(mounted.view)
            } else {
                self.recycle(mounted.view, &mounted.reuse_identifier, cx);
                self.take_reusable_view(&descriptor.reuse_identifier)
            }
        } else {
            self.take_reusable_view(&descriptor.reuse_identifier)
        };

        let view = (descriptor.view)(reusable_view.clone(), window, cx);
        if let Some(reusable_view) = reusable_view
            && reusable_view.entity_id() != view.entity_id()
        {
            self.recycle(reusable_view, &descriptor.reuse_identifier, cx);
        }
        self.mounted_bottom_sticky_headers.insert(
            id.to_string(),
            RiseManualListViewMountedBottomStickyHeader {
                view,
                frame: visible_header.frame,
                reuse_identifier: descriptor.reuse_identifier.clone(),
                content_version: descriptor.content_version,
            },
        );
    }

    /// taps on a top sticky row
    /// row cant answer itself, its lifted host has hit testing off
    /// the list hit tests the known frames and tells the row which form was tapped
    fn top_sticky_header_target(
        &self,
        at: Point<Pixels>,
        admits_fading_chip: bool,
        cx: &App,
    ) -> Option<(String, AnyView)> {
        for (id, frame) in &self.top_sticky_item_frames {
            if !frame.contains(&at) {
                continue;
            }
            let Some(mounted) = self.mounted_items.get(id) else {
                continue;
            };
            let point_in_view = at - frame.origin;
            if self.is_top_sticky_item_faded(id) && self.top_sticky_touch_claim_id.as_deref() != Some(id.as_str()) {
                if !admits_fading_chip
                    || RiseManualListConformances::is_top_sticky_header_on_screen(&mounted.view, cx) != Some(true)
                {
                    continue;
                }
            }
            if RiseManualListConformances::top_sticky_header_contains(&mounted.view, point_in_view, cx) != Some(true) {
                continue;
            }
            return Some((id.clone(), mounted.view.clone()));
        }
        None
    }

    fn bottom_sticky_header_target(&self, at: Point<Pixels>) -> Option<AnyView> {
        if self.bottom_sticky_header_host_view.is_hidden {
            return None;
        }
        self.mounted_bottom_sticky_headers
            .values()
            .find(|mounted| mounted.frame.contains(&at))
            .map(|mounted| mounted.view.clone())
    }

    fn handle_top_sticky_header_tap(&mut self, location: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if self.top_sticky_header_tap_gesture.state() != RiseStrictTapGestureRecognizerState::Recognized {
            return;
        }
        let Some((id, view)) = self.top_sticky_header_target(location, false, cx) else {
            return;
        };
        let is_pinned = self.pinned_top_sticky_item_ids.contains(&id);
        RiseManualListConformances::activate_top_sticky_header(&view, is_pinned, window, cx);
    }

    fn handle_bottom_sticky_header_tap(&mut self, location: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if self.bottom_sticky_header_tap_gesture.state() != RiseStrictTapGestureRecognizerState::Recognized {
            return;
        }
        let Some(view) = self.bottom_sticky_header_target(location) else {
            return;
        };
        RiseManualListConformances::activate_bottom_sticky_header(&view, window, cx);
    }

    /// whether the pointer is over a chip, for the two sticky taps
    fn sticky_header_gesture_claims_touch(&self, is_top: bool, location: Point<Pixels>, cx: &App) -> bool {
        if is_top {
            self.top_sticky_header_target(location, false, cx).is_some()
        } else {
            self.bottom_sticky_header_target(location).is_some()
        }
    }

    fn top_sticky_header_gesture_should_receive(&mut self, location: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        self.top_sticky_touch_claim_id = None;
        let Some((id, _)) = self.top_sticky_header_target(location, true, cx) else {
            return;
        };
        let is_faded = self.is_top_sticky_item_faded(&id);
        self.top_sticky_touch_claim_id = Some(id);
        if is_faded {
            self.flash_top_sticky_headers(window, cx);
            self.schedule_top_sticky_flash_end(window, cx);
        }
    }

    fn sticky_headers_pointer_down(
        &mut self,
        window_location: Point<Pixels>,
        location: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.top_sticky_header_gesture_should_receive(location, window, cx);
        self.top_sticky_header_tap_gesture.touches_began(window_location);
        self.bottom_sticky_header_tap_gesture.touches_began(window_location);
        self.sticky_header_gesture_claims_touch(true, location, cx)
            || self.sticky_header_gesture_claims_touch(false, location, cx)
    }

    fn sticky_headers_pointer_moved(&mut self, window_location: Point<Pixels>) {
        self.top_sticky_header_tap_gesture.touches_moved(window_location);
        self.bottom_sticky_header_tap_gesture.touches_moved(window_location);
    }

    fn sticky_headers_pointer_up(
        &mut self,
        window_location: Point<Pixels>,
        location: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let top_ended = self.top_sticky_header_tap_gesture.touches_ended(window_location);
        let bottom_ended = self.bottom_sticky_header_tap_gesture.touches_ended(window_location);
        let mut handled = false;
        if top_ended && self.sticky_header_gesture_claims_touch(true, location, cx) {
            self.handle_top_sticky_header_tap(location, window, cx);
            handled = true;
        }
        if bottom_ended && self.sticky_header_gesture_claims_touch(false, location, cx) {
            self.handle_bottom_sticky_header_tap(location, window, cx);
            handled = true;
        }
        handled
    }

    fn unmount_bottom_sticky_header(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(mounted) = self.mounted_bottom_sticky_headers.remove(id) else {
            return;
        };
        self.recycle(mounted.view, &mounted.reuse_identifier, cx);
    }

    fn unmount_all_bottom_sticky_headers(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.mounted_bottom_sticky_headers.keys().cloned().collect();
        for id in ids {
            self.unmount_bottom_sticky_header(&id, cx);
        }
    }
}

impl RiseManualListView {
    const LINE_SCROLL_STEP: f32 = 40.0;
    const WARM_BEHIND_PENALTY: f32 = 100_000.0;
    const WHEEL_SETTLE_DELAY: Duration = Duration::from_millis(120);
    const WHEEL_DRAG_STALE_DELAY: Duration = Duration::from_millis(600);
    const RUBBER_BAND_COEFFICIENT: f32 = 0.55;

    /// how far offset_y sits past the row ends, positive below the last row and negative above the
    /// first and zero inside
    /// a resolved leading reveal (archive row) is no overshoot
    fn natural_overshoot(&self, offset_y: Pixels, cx: &App) -> Pixels {
        let (minimum, maximum) = self.natural_scroll_offset_range(offset_y);
        if offset_y > maximum + px(0.5) {
            offset_y - maximum
        } else if offset_y < minimum - px(0.5) && !self.keeps_leading_reveal(cx) {
            offset_y - minimum
        } else {
            Pixels::ZERO
        }
    }

    fn rubber_banded_drag_target(&self, offset_y: Pixels, target_y: Pixels, cx: &App) -> Pixels {
        let (minimum, maximum) = self.natural_scroll_offset_range(offset_y);
        let (offset, target) = (f32::from(offset_y), f32::from(target_y));
        let (past, travel, outward) = if target > offset {
            (offset - f32::from(maximum), target - offset, 1.0)
        } else if target < offset && !self.keeps_leading_reveal(cx) {
            (f32::from(minimum) - offset, offset - target, -1.0)
        } else {
            return target_y;
        };
        let free = travel.min((-past).max(0.0));
        let banded = travel - free;
        if banded <= 0.0 {
            return target_y;
        }
        let dimension = f32::from(self.bounds.size.height).max(1.0);
        let overshoot = past.max(0.0);
        let factor = Self::RUBBER_BAND_COEFFICIENT * (1.0 - (overshoot / dimension).min(1.0)).powi(2);
        px(offset + outward * (free + banded * factor))
    }

    fn bounce_back_from_overshoot(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.is_programmatic_scroll_animating {
            return true;
        }
        self.wheel_settle_task = None;
        if self.scroll_view.is_decelerating {
            self.scroll_view.is_decelerating = false;
            self.scroll_view_did_end_decelerating(window, cx);
        } else {
            self.settle_scroll_geometry_at_rest(window, cx);
        }
        self.is_programmatic_scroll_animating
    }

    fn mirror_integration_scroll_offset(&self) {
        let offset = self.scroll_view.content_offset;
        self.integration_scroll_handle.set_offset(point(-offset.x, -offset.y));
    }

    fn is_scroll_moving(&self) -> bool {
        self.scroll_view.is_tracking
            || self.scroll_view.is_dragging
            || self.scroll_view.is_decelerating
            || self.is_programmatic_scroll_animating
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.scroll_view.is_scroll_enabled {
            return;
        }
        match event.touch_phase {
            TouchPhase::Started => {
                self.wheel_settle_task = None;
                if self.scroll_view.is_decelerating {
                    self.scroll_view.is_decelerating = false;
                    self.scroll_view_did_end_decelerating(window, cx);
                }
                self.scroll_view.is_tracking = true;
                self.scroll_view.is_dragging = true;
                self.scroll_view_will_begin_dragging(window, cx);
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.end_wheel_drag(window, cx);
            }
            TouchPhase::Moved => {}
        }
        let delta = event.delta.pixel_delta(px(Self::LINE_SCROLL_STEP));
        if delta.y != Pixels::ZERO
            && !self.scroll_view.is_dragging
            && self.natural_overshoot(self.scroll_view.content_offset.y, cx) != Pixels::ZERO
            && self.bounce_back_from_overshoot(window, cx)
        {
            self.scroll_motion.set(self.is_scroll_moving());
            cx.notify();
            return;
        }
        if delta.y != Pixels::ZERO {
            if self.programmatic_scroll.is_some() {
                self.cancel_programmatic_scroll(window, cx);
            }
            if !self.scroll_view.is_dragging && self.programmatic_navigation_owns_scroll {
                self.programmatic_navigation_owns_scroll = false;
                self.is_programmatic_scroll_animating = false;
            }
            let offset = self.scroll_view.content_offset;
            let mut target_y = offset.y - delta.y;
            if self.scroll_view.is_dragging {
                target_y = self.rubber_banded_drag_target(offset.y, target_y, cx);
            }
            if !self.scroll_view.is_dragging && delta.y < Pixels::ZERO {
                let (_, natural_maximum) = self.natural_scroll_offset_range(offset.y);
                target_y = target_y.min(natural_maximum.max(offset.y));
            }
            let mut wheel_past_top = Pixels::ZERO;
            if !self.scroll_view.is_dragging && delta.y > Pixels::ZERO {
                let (natural_minimum, _) = self.natural_scroll_offset_range(offset.y);
                if offset.y <= natural_minimum + px(0.5) {
                    wheel_past_top = natural_minimum - target_y;
                }
                target_y = target_y.max(natural_minimum.min(offset.y));
            }
            if wheel_past_top > Pixels::ZERO
                && let Some(on_wheel_past_top) = self.on_wheel_past_top.clone()
            {
                window.defer(cx, move |window, cx| on_wheel_past_top(wheel_past_top, window, cx));
            }
            self.set_content_offset_within_overscroll_limits(point(offset.x, target_y), cx);
            if !self.scroll_view.is_dragging {
                self.scroll_view.is_decelerating = true;
                self.schedule_wheel_settle(window, cx);
            }
            self.scroll_view_did_scroll(window, cx);
        }
        if self.scroll_view.is_dragging {
            self.schedule_wheel_settle(window, cx);
        }
        self.scroll_motion.set(self.is_scroll_moving());
        cx.notify();
    }

    fn end_wheel_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.scroll_view.is_dragging {
            return;
        }
        self.scroll_view.is_tracking = false;
        let velocity = point(0.0, f32::from(self.effective_estimated_scroll_velocity_y()));
        let mut target_content_offset = self.scroll_view.content_offset;
        self.scroll_view_will_end_dragging(velocity, &mut target_content_offset);
        self.scroll_view.is_dragging = false;
        let decelerate = velocity.y.abs() > 1.0
            && self.natural_overshoot(self.scroll_view.content_offset.y, cx) == Pixels::ZERO;
        if decelerate {
            self.scroll_view.is_decelerating = true;
            self.schedule_wheel_settle(window, cx);
        }
        self.scroll_view_did_end_dragging(decelerate, window, cx);
    }

    fn schedule_wheel_settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let delay = if self.scroll_view.is_dragging { Self::WHEEL_DRAG_STALE_DELAY } else { Self::WHEEL_SETTLE_DELAY };
        self.wheel_settle_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.wheel_settle_task = None;
                if this.scroll_view.is_dragging {
                    tracing::warn!(target: "riseonly::post_list_pagination", "wheel drag without events ended by watchdog");
                    this.end_wheel_drag(window, cx);
                    this.scroll_motion.set(this.is_scroll_moving());
                    cx.notify();
                    return;
                }
                if !this.scroll_view.is_decelerating {
                    return;
                }
                this.scroll_view.is_decelerating = false;
                this.scroll_view_did_end_decelerating(window, cx);
                this.scroll_motion.set(this.is_scroll_moving());
                cx.notify();
            });
        }));
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.scroll_view.is_scroll_enabled || !self.has_usable_viewport() {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        let jumps_to_edge = modifiers.platform || modifiers.control;
        let page_step = (self.bounds.size.height - self.viewport_insets.top - self.viewport_insets.bottom).max(px(1.0));
        let offset_y = self.scroll_view.content_offset.y;
        let handled = match event.keystroke.key.as_str() {
            "down" if jumps_to_edge => self.smart_scroll_to_bottom(true, window, cx),
            "up" if jumps_to_edge => self.smart_scroll_to_top(true, window, cx),
            "end" => self.smart_scroll_to_bottom(true, window, cx),
            "home" => self.smart_scroll_to_top(true, window, cx),
            "pagedown" => self.set_content_offset_y(offset_y + page_step, true, true, window, cx),
            "pageup" => self.set_content_offset_y(offset_y - page_step, true, true, window, cx),
            "down" => self.set_content_offset_y(offset_y + px(Self::LINE_SCROLL_STEP), false, true, window, cx),
            "up" => self.set_content_offset_y(offset_y - px(Self::LINE_SCROLL_STEP), false, true, window, cx),
            _ => false,
        };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn handle_pointer_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        ScreenFocus::claim(&self.focus_handle, window, cx);
        let location = event.position - self.bounds.origin;
        if self.sticky_headers_pointer_down(event.position, location, window, cx) {
            cx.stop_propagation();
        }
    }

    fn handle_pointer_moved(&mut self, event: &MouseMoveEvent) {
        if event.pressed_button.is_none() {
            return;
        }
        self.sticky_headers_pointer_moved(event.position);
    }

    fn handle_selection_pointer_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selection_presentation.is_active {
            return;
        }
        ScreenFocus::claim(&self.focus_handle, window, cx);
        tracing::warn!(target: "riseonly::X1870", position = ?event.position, button = ?event.button, "selection pointer down");
        self.selection_overlay_view.touches_began(event.position);
    }

    fn handle_selection_pointer_moved(&mut self, event: &MouseMoveEvent) {
        if event.pressed_button.is_none() || !self.selection_presentation.is_active {
            return;
        }
        self.selection_overlay_view.touches_moved(event.position);
    }

    fn handle_selection_pointer_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selection_presentation.is_active {
            return;
        }
        let location_in_host = event.position - self.bounds.origin - self.item_host_origin_in_list();
        tracing::warn!(target: "riseonly::X1870", position = ?event.position, host_y = ?location_in_host.y, tap_state = ?self.selection_overlay_view.tap_gesture.state(), "selection pointer up");
        self.selection_overlay_view.touches_ended(event.position, location_in_host, window, cx);
        cx.notify();
    }

    fn handle_pointer_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let location = event.position - self.bounds.origin;
        if self.sticky_headers_pointer_up(event.position, location, window, cx) {
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// item host origin in the list as presented now
    /// model origin plus the running keyboard and anchor and jump slide translations
    fn presented_item_host_origin(&self, now: f64) -> Point<Pixels> {
        let host_frame = self.item_host_view.presentation_frame(now);
        self.scroll_view.frame.origin - self.scroll_view.content_offset
            + host_frame.origin
            + point(
                Pixels::ZERO,
                self.item_host_view.presentation_translation_y(now) + self.jump_slide_layer.presentation_translation_y(now),
            )
    }

    fn row_element(
        view: &AnyView,
        frame: Bounds<Pixels>,
        motion: Option<&RiseManualListViewRowMotion>,
        blocks_own_pointer: bool,
        now: f64,
    ) -> AnyElement {
        Self::row_element_with(view, frame, motion, blocks_own_pointer, now, false, false)
    }

    /// cached means the row type registered register_cached_row, so it replays its last frame
    /// unless it notified or moved
    /// replays_moved means it registered the moved variant
    fn row_element_with(
        view: &AnyView,
        frame: Bounds<Pixels>,
        motion: Option<&RiseManualListViewRowMotion>,
        blocks_own_pointer: bool,
        now: f64,
        cached: bool,
        replays_moved: bool,
    ) -> AnyElement {
        let content = match (cached, replays_moved) {
            (true, true) => view
                .clone()
                .cached(StyleRefinement::default().size_full())
                .replay_when_moved()
                .into_any_element(),
            (true, false) => view.clone().cached(StyleRefinement::default().size_full()).into_any_element(),
            (false, _) => view.clone().into_any_element(),
        };
        let offset = motion.map_or(Point::default(), |motion| motion.presentation_offset(now));
        let alpha = motion.map_or(1.0, |motion| motion.presentation_alpha(now));
        let scale = motion.map_or(1.0, |motion| motion.presentation_scale(now));
        let offset = if replays_moved && alpha >= 1.0 && scale == 1.0 {
            point(px(f32::from(offset.x).round()), px(f32::from(offset.y).round()))
        } else {
            offset
        };
        let width = frame.size.width * scale;
        let height = frame.size.height * scale;
        let origin = point(
            frame.origin.x + offset.x + (frame.size.width - width) * 0.5,
            frame.origin.y + offset.y + (frame.size.height - height) * 0.5,
        );
        let clip_height = motion.and_then(|motion| motion.presentation_clip_height(now));
        let row = match clip_height {
            Some(clip_height) => div()
                .absolute()
                .left(origin.x)
                .top(origin.y)
                .w(width)
                .h(clip_height.max(Pixels::ZERO).min(height))
                .overflow_hidden()
                .opacity(alpha)
                .child(div().absolute().left(Pixels::ZERO).top(Pixels::ZERO).w(width).h(height).child(content)),
            None => div()
                .absolute()
                .left(origin.x)
                .top(origin.y)
                .w(width)
                .h(height)
                .opacity(alpha)
                .child(content),
        };
        if blocks_own_pointer {
            row.child(div().absolute().size_full().block_mouse_except_scroll()).into_any_element()
        } else {
            row.into_any_element()
        }
    }

    /// outgoing or replaced or riding snapshot, a cropped one shows only its band of the row
    fn snapshot_element(snapshot: &RiseManualListViewUpdateTransitionSnapshot, now: f64, cx: &App) -> AnyElement {
        let Some(crop) = snapshot.crop else {
            let cached = RiseManualListConformances::caches_row(&snapshot.view, cx);
            let replays_moved = RiseManualListConformances::replays_moved_row(&snapshot.view, cx);
            return Self::row_element_with(
                &snapshot.view,
                snapshot.frame,
                Some(&snapshot.motion),
                false,
                now,
                cached,
                replays_moved,
            );
        };
        let offset = snapshot.motion.presentation_offset(now);
        div()
            .absolute()
            .left(snapshot.frame.origin.x + offset.x)
            .top(snapshot.frame.origin.y + offset.y)
            .w(snapshot.frame.size.width)
            .h(snapshot.frame.size.height)
            .overflow_hidden()
            .opacity(snapshot.motion.presentation_alpha(now))
            .child(
                div()
                    .absolute()
                    .left(Pixels::ZERO)
                    .top(Pixels::ZERO - crop.offset_y)
                    .w(crop.content_size.width)
                    .h(crop.content_size.height)
                    .child(snapshot.view.clone()),
            )
            .child(div().absolute().size_full().block_mouse_except_scroll())
            .into_any_element()
    }

    fn has_running_presentation(&self, now: f64) -> bool {
        [
            &self.viewport_clip_view,
            &self.item_host_view,
            &self.jump_highlight_view,
            &self.jump_slide_layer,
            &self.selection_overlay_layer,
            &self.top_sticky_header_host_view,
            &self.bottom_sticky_header_host_view,
            &self.keyboard_timing_reference_view,
        ]
        .into_iter()
        .any(|layer| layer.has_running_animation(now))
            || self.active_jump_snapshot.as_ref().is_some_and(|snapshot| snapshot.layer.has_running_animation(now))
            || self.update_transition_motions.values().any(|motion| motion.is_running(now))
            || self.update_transition_snapshots.iter().any(|snapshot| snapshot.motion.is_running(now))
            || self.removal_dust_host.is_running(now)
    }
}

impl Render for RiseManualListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = current_media_time();
        self.awaits_render_after_transaction = false;
        self.effective_layout_direction_is_left_to_right =
            matches!(layout_direction(cx), LayoutDirection::LeftToRight);
        self.scroll_motion.set(self.is_scroll_moving());
        self.mirror_integration_scroll_offset();
        if self.has_running_presentation(now) {
            window.request_animation_frame();
        }

        let clip_frame = self.viewport_clip_view.presentation_frame(now);
        let host_origin = self.presented_item_host_origin(now);
        let item_host_alpha = self.item_host_view.presentation_alpha(now);

        let draw_band = RiseManualListDrawBand::around(clip_frame.size.height, host_origin.y);
        let mut item_rows: Vec<AnyElement> = Vec::with_capacity(self.mounted_items.len());
        let previously_drawn_rows = std::mem::take(&mut self.drawn_row_ids);
        let mut drawn_row_ids: HashSet<EntityId> = HashSet::with_capacity(previously_drawn_rows.len() + 1);
        let mut cold_band_rows = 0_usize;
        let mut warm_candidates = 0_usize;
        let mut warm_candidate: Option<(f32, &RiseManualListViewMountedItem)> = None;
        for (id, mounted) in &self.mounted_items {
            if mounted.host != RiseManualListViewUpdateTransitionHost::Item {
                continue;
            }
            let entity_id = mounted.view.entity_id();
            let motion = self.update_transition_motions.get(&entity_id);
            let in_band = motion.is_some_and(|motion| motion.is_running(now)) || draw_band.contains(mounted.frame);
            let cached = RiseManualListConformances::caches_row(&mounted.view, cx);
            let replays_moved = RiseManualListConformances::replays_moved_row(&mounted.view, cx);
            let warms = self.warms_mounted_rows && cached && replays_moved && !mounted.is_placeholder;
            let was_drawn = previously_drawn_rows.contains(&entity_id);
            if !in_band {
                if !warms {
                    continue;
                }
                if !was_drawn {
                    if !self.deferred_reconfigure_ids.contains(id) {
                        warm_candidates += 1;
                        let (gap, below) = draw_band.gap(mounted.frame);
                        let ahead = below == (self.mount_direction >= 0.0);
                        let score = if ahead { gap } else { gap + Self::WARM_BEHIND_PENALTY };
                        if warm_candidate.is_none_or(|(best, _)| score < best) {
                            warm_candidate = Some((score, mounted));
                        }
                    }
                    continue;
                }
            } else if warms && !was_drawn {
                cold_band_rows += 1;
            }
            drawn_row_ids.insert(entity_id);
            let blocks_own_pointer =
                mounted.is_placeholder || self.selection_interaction_disabled_ids.contains(&entity_id);
            item_rows.push(Self::row_element_with(
                &mounted.view,
                mounted.frame,
                motion,
                blocks_own_pointer,
                now,
                cached,
                replays_moved,
            ));
        }
        if cold_band_rows == 0
            && let Some((_, mounted)) = warm_candidate
        {
            let entity_id = mounted.view.entity_id();
            drawn_row_ids.insert(entity_id);
            warm_candidates -= 1;
            item_rows.push(Self::row_element_with(
                &mounted.view,
                mounted.frame,
                self.update_transition_motions.get(&entity_id),
                self.selection_interaction_disabled_ids.contains(&entity_id),
                now,
                true,
                true,
            ));
        }
        if (1..=3).contains(&self.layout_items.count()) {
            tracing::warn!(
                target: "riseonly::X2257",
                items = self.layout_items.count(),
                mounted = self.mounted_items.len(),
                drawn = drawn_row_ids.len(),
                band_top = f32::from(draw_band.top),
                band_bottom = f32::from(draw_band.bottom),
                clip_y = f32::from(clip_frame.origin.y),
                clip_h = f32::from(clip_frame.size.height),
                host_x = f32::from(host_origin.x),
                host_y = f32::from(host_origin.y),
                host_alpha = item_host_alpha,
                motions = self.update_transition_motions.len(),
                running = self.update_transition_motions.values().filter(|motion| motion.is_running(now)).count(),
                "short list render"
            );
        }
        self.drawn_row_ids = drawn_row_ids;
        if warm_candidates > 0 {
            window.request_animation_frame();
        }
        for snapshot in &self.update_transition_snapshots {
            if snapshot.host == RiseManualListViewUpdateTransitionHost::Item {
                item_rows.push(Self::snapshot_element(snapshot, now, cx));
            }
        }
        let jump_highlight_alpha = self.jump_highlight_view.presentation_alpha(now);
        let jump_highlight = (jump_highlight_alpha > 0.0).then(|| {
            let frame = self.jump_highlight_view.frame;
            let content_origin = self.scroll_view.frame.origin - self.scroll_view.content_offset;
            div()
                .absolute()
                .left(content_origin.x + frame.origin.x)
                .top(content_origin.y + frame.origin.y)
                .w(frame.size.width)
                .h(frame.size.height)
                .bg(self.jump_highlight_color)
                .opacity(jump_highlight_alpha)
        });
        let item_host = div()
            .absolute()
            .left(host_origin.x)
            .top(host_origin.y)
            .w(self.item_host_view.frame.size.width)
            .h(self.item_host_view.frame.size.height)
            .opacity(item_host_alpha)
            .children(item_rows);

        let jump_snapshot = self.active_jump_snapshot.as_ref().map(|snapshot| {
            let translation_y = snapshot.layer.presentation_translation_y(now);
            let alpha = snapshot.layer.presentation_alpha(now);
            div()
                .absolute()
                .top(translation_y)
                .left(Pixels::ZERO)
                .size_full()
                .opacity(alpha)
                .children(
                    snapshot
                        .rows
                        .iter()
                        .chain(snapshot.avatars.iter())
                        .map(|(view, frame)| Self::row_element(view, *frame, None, false, now)),
                )
        });

        let top_sticky_frame = self.top_sticky_header_host_view.presentation_frame(now);
        let mut top_sticky_rows: Vec<AnyElement> = Vec::new();
        for mounted in self.mounted_items.values() {
            if mounted.host == RiseManualListViewUpdateTransitionHost::TopSticky {
                let entity_id = mounted.view.entity_id();
                let cached = RiseManualListConformances::caches_row(&mounted.view, cx);
                let replays_moved = RiseManualListConformances::replays_moved_row(&mounted.view, cx);
                top_sticky_rows.push(Self::row_element_with(
                    &mounted.view,
                    mounted.frame,
                    self.update_transition_motions.get(&entity_id),
                    false,
                    now,
                    cached,
                    replays_moved,
                ));
            }
        }
        for snapshot in &self.update_transition_snapshots {
            if snapshot.host == RiseManualListViewUpdateTransitionHost::TopSticky {
                top_sticky_rows.push(Self::snapshot_element(snapshot, now, cx));
            }
        }
        let top_sticky_host = div()
            .absolute()
            .left(top_sticky_frame.origin.x)
            .top(top_sticky_frame.origin.y + self.top_sticky_header_host_view.presentation_translation_y(now))
            .w(top_sticky_frame.size.width)
            .h(top_sticky_frame.size.height)
            .children(top_sticky_rows);

        let bottom_sticky_host = (!self.bottom_sticky_header_host_view.is_hidden).then(|| {
            let frame = self.bottom_sticky_header_host_view.presentation_frame(now);
            let mut rows: Vec<AnyElement> = self
                .mounted_bottom_sticky_headers
                .values()
                .map(|mounted| {
                    let cached = RiseManualListConformances::caches_row(&mounted.view, cx);
                    let replays_moved = RiseManualListConformances::replays_moved_row(&mounted.view, cx);
                    Self::row_element_with(&mounted.view, mounted.frame, None, false, now, cached, replays_moved)
                })
                .collect();
            for snapshot in &self.update_transition_snapshots {
                if snapshot.host == RiseManualListViewUpdateTransitionHost::BottomSticky {
                    rows.push(Self::snapshot_element(snapshot, now, cx));
                }
            }
            div()
                .absolute()
                .left(frame.origin.x)
                .top(frame.origin.y + self.bottom_sticky_header_host_view.presentation_translation_y(now))
                .w(frame.size.width)
                .h(frame.size.height)
                .opacity(self.bottom_sticky_header_host_view.presentation_alpha(now))
                .children(rows)
        });

        let list_id = ElementId::from(("rise-manual-list", cx.entity_id().as_u64()));
        let selection_accessibility = self.selection_overlay_view.accessibility_children(&list_id);
        let selection_pointer = self.selection_presentation.is_active.then(|| {
            div()
                .absolute()
                .size_full()
                .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.handle_selection_pointer_down(event, window, cx);
                }))
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, _| {
                    this.handle_selection_pointer_moved(event);
                }))
                .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.handle_selection_pointer_up(event, window, cx);
                }))
        });
        let entity = cx.entity();
        let layout_entity = entity.clone();
        let layout_and_overlay = canvas(
            move |bounds, window, cx| {
                layout_entity.update(cx, |this, cx| {
                    this.main_tab_scroll_to_top_mount.did_lay_out(bounds, window);
                    if this.bounds.size != bounds.size {
                        this.layout_subviews(bounds, window, cx);
                        cx.on_next_frame(window, |_, _, cx| cx.notify());
                    } else if this.bounds.origin != bounds.origin {
                        this.bounds = bounds;
                    }
                });
            },
            move |bounds, (), window, cx| {
                let this = entity.read(cx);
                if this.selection_presentation.is_active {
                    let now = current_media_time();
                    let origin = bounds.origin + this.viewport_clip_view.presentation_frame(now).origin
                        + this.presented_item_host_origin(now);
                    this.selection_overlay_view.paint(origin, window);
                }
                entity.update(cx, |this, _| {
                    let now = current_media_time();
                    if this.removal_dust_host.is_running(now) {
                        let origin = bounds.origin
                            + this.viewport_clip_view.presentation_frame(now).origin
                            + this.presented_item_host_origin(now);
                        this.removal_dust_host.paint(origin, now, window);
                    }
                });
            },
        )
        .absolute()
        .size_full();

        div()
            .id(list_id)
            .relative()
            .size_full()
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                this.handle_scroll_wheel(event, window, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_key_down(event, window, cx);
            }))
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                this.handle_pointer_down(event, window, cx);
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, _| {
                this.handle_pointer_moved(event);
            }))
            .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, window, cx| {
                this.handle_pointer_up(event, window, cx);
            }))
            .child(
                div()
                    .absolute()
                    .left(clip_frame.origin.x)
                    .top(clip_frame.origin.y)
                    .w(clip_frame.size.width)
                    .h(clip_frame.size.height)
                    .opacity(self.viewport_clip_view.presentation_alpha(now))
                    .overflow_hidden()
                    .children(jump_highlight)
                    .child(item_host)
                    .children(jump_snapshot)
                    .child(top_sticky_host)
                    .children(bottom_sticky_host),
            )
            .child(layout_and_overlay)
            .children(selection_pointer)
            .children(selection_accessibility)
    }
}

impl Drop for RiseManualListView {
    fn drop(&mut self) {
        self.deferred_mount_display_link = None;
        self.programmatic_scroll_display_link = None;
        self.wheel_settle_task = None;
        self.memory_warning_observer = None;
        self.memory_warning_task = None;
        self.window_activation_observer = None;
    }
}

/// seconds of the look swap of a row whose view was replaced in place
pub enum RiseManualListReplacementFade {}

impl RiseManualListReplacementFade {
    pub const NEW_LOOK_DURATION: f64 = 0.12;
    pub const OLD_LOOK_DURATION: f64 = 0.2;
}

pub enum RiseListRemovalDiag {}

impl RiseListRemovalDiag {
    const ACTIVE_WINDOW: Duration = Duration::from_secs(4);

    pub fn is_active(cx: &mut App) -> bool {
        let restored_at = HoldContextMenuPresenter::shared(cx).read(cx).source_restored_at();
        !restored_at.is_zero() && RiseClock::monotonic().saturating_sub(restored_at) < Self::ACTIVE_WINDOW
    }

    pub fn log(message: impl FnOnce() -> String, if_active: bool, cx: &mut App) {
        if if_active && !Self::is_active(cx) {
            return;
        }
        let text = message();
        tracing::info!(target: "ListRemovalDiag", "[f9d9] {text}");
    }

    pub fn f(value: Option<Pixels>) -> String {
        match value {
            Some(value) => format!("{:.1}", f32::from(value)),
            None => "nil".to_string(),
        }
    }

    pub fn f_rect(rect: Option<Bounds<Pixels>>) -> String {
        match rect {
            Some(rect) => format!(
                "({:.1},{:.1} {:.1}x{:.1})",
                f32::from(rect.origin.x),
                f32::from(rect.origin.y),
                f32::from(rect.size.width),
                f32::from(rect.size.height)
            ),
            None => "nil".to_string(),
        }
    }
}
