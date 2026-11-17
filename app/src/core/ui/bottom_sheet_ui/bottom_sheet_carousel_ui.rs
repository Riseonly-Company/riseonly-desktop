use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AccessibleAction, App, Bounds, Context, DispatchPhase, ElementId, Entity, FocusHandle, GlobalElementId, Hsla,
    InspectorElementId, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollHandle, ScrollWheelEvent, SharedString, Size, TouchPhase, WeakEntity, Window, black, canvas, point,
    px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::bottom_sheet_ui::bottom_sheet_axis_ownership::{
    BottomSheetAxisOwnership, BottomSheetGestureRecognizer, BottomSheetGestureState,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::IconUi;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::motion::{CubicBezier, Spring, duration_from_seconds, ease_out};
use rise_ui::swiftui::prelude::*;

use crate::core::lib::device_info::DeviceInfo;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager_gesture_gate::{
    AnimatedTabsPagerAxisOwners, AnimatedTabsPagerAxisOwnersKey, AnimatedTabsPagerPanIntentPolicy,
};
use crate::core::ui::bottom_sheet_ui::bottom_sheet_full_page_ui::BottomSheetFullPageUi;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_page_back_swipe_gate::BottomSheetPageBackSwipePolicy;
use crate::core::ui::bottom_sheet_ui::experimental_bottom_sheet_presentation_host::ExperimentalBottomSheetPresentationHost;

const PAN_HYSTERESIS: f32 = 10.0;
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
const WHEEL_STEP_DISTANCE: f32 = 24.0;
const WHEEL_QUIET: Duration = Duration::from_millis(400);
const EDGE_BUTTON_SIZE: f32 = 40.0;
const EDGE_BUTTON_INSET: f32 = 16.0;

pub type BottomSheetCarouselAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type BottomSheetCarouselSelect<ID> = Rc<dyn Fn(ID, &mut Window, &mut App)>;
type CarouselSlot<ID> = Rc<dyn Fn(&ID, &mut Window, &mut App) -> AnyElement>;
type CarouselSnapFractions<ID> = Rc<dyn Fn(&ID) -> Vec<f32>>;
/// a page by id, the flag says if it appears in place instead of rising
type CarouselPage<ID> = Rc<dyn Fn(&ID, bool, &mut Window, &mut App) -> AnyElement>;

#[derive(IntoElement)]
pub struct BottomSheetCarouselUi<ID>
where
    ID: Clone + Eq + Hash + 'static,
{
    id: ElementId,
    is_bottom_sheet: Binding<bool>,
    page_ids: Vec<ID>,
    selection: Option<ID>,
    on_select: BottomSheetCarouselSelect<ID>,
    snap_fractions: CarouselSnapFractions<ID>,
    spacing: f32,
    content_under_handle: bool,
    content_scrollable: bool,
    footer_shows_top_border: bool,
    sheet_background: Option<Hsla>,
    footer_background: Option<Hsla>,
    on_approach_end: BottomSheetCarouselAction,
    on_did_dismiss: BottomSheetCarouselAction,
    on_presentation_refused: BottomSheetCarouselAction,
    sheet_content: CarouselSlot<ID>,
    footer: CarouselSlot<ID>,
}

impl<ID> BottomSheetCarouselUi<ID>
where
    ID: Clone + Eq + Hash + 'static,
{
    pub fn new<C: IntoElement, F: IntoElement>(
        id: impl Into<ElementId>,
        is_bottom_sheet: Binding<bool>,
        page_ids: Vec<ID>,
        selection: Option<ID>,
        on_select: impl Fn(ID, &mut Window, &mut App) + 'static,
        sheet_content: impl Fn(&ID, &mut Window, &mut App) -> C + 'static,
        footer: impl Fn(&ID, &mut Window, &mut App) -> F + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            is_bottom_sheet,
            page_ids,
            selection,
            on_select: Rc::new(on_select),
            snap_fractions: Rc::new(|_: &ID| vec![0.6, 0.93]),
            spacing: BottomSheetMetrics::CAROUSEL_PAGE_SPACING,
            content_under_handle: false,
            content_scrollable: false,
            footer_shows_top_border: true,
            sheet_background: None,
            footer_background: None,
            on_approach_end: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_did_dismiss: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_presentation_refused: Rc::new(|_: &mut Window, _: &mut App| {}),
            sheet_content: Rc::new(move |id: &ID, window: &mut Window, cx: &mut App| {
                sheet_content(id, window, cx).into_any_element()
            }),
            footer: Rc::new(move |id: &ID, window: &mut Window, cx: &mut App| {
                footer(id, window, cx).into_any_element()
            }),
        }
    }

    pub fn snap_fractions(mut self, snap_fractions: impl Fn(&ID) -> Vec<f32> + 'static) -> Self {
        self.snap_fractions = Rc::new(snap_fractions);
        self
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn content_under_handle(mut self, content_under_handle: bool) -> Self {
        self.content_under_handle = content_under_handle;
        self
    }

    pub fn content_scrollable(mut self, content_scrollable: bool) -> Self {
        self.content_scrollable = content_scrollable;
        self
    }

    pub fn footer_shows_top_border(mut self, shows: bool) -> Self {
        self.footer_shows_top_border = shows;
        self
    }

    pub fn sheet_background(mut self, background: Option<Hsla>) -> Self {
        self.sheet_background = background;
        self
    }

    pub fn footer_background(mut self, background: Option<Hsla>) -> Self {
        self.footer_background = background;
        self
    }

    pub fn on_approach_end(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_approach_end = Rc::new(action);
        self
    }

    pub fn on_did_dismiss(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_did_dismiss = Rc::new(action);
        self
    }

    pub fn on_presentation_refused(
        mut self,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_presentation_refused = Rc::new(action);
        self
    }

    fn strip(self: &Rc<Self>, cx: &App) -> BottomSheetCarouselStrip<ID> {
        let carousel = self.clone();
        BottomSheetCarouselStrip {
            id: (self.id.clone(), "strip").into(),
            page_ids: self.page_ids.clone(),
            selection: self.selection.clone(),
            spacing: self.spacing,
            is_presented: self.is_bottom_sheet.get(cx),
            on_select: self.on_select.clone(),
            on_approach_end: self.on_approach_end.clone(),
            page: Rc::new(move |id: &ID, appears_in_place: bool, _: &mut Window, _: &mut App| {
                carousel.page(id, appears_in_place).into_any_element()
            }),
        }
    }

    fn page(&self, id: &ID, appears_in_place: bool) -> BottomSheetFullPageUi {
        let sheet_content = self.sheet_content.clone();
        let footer = self.footer.clone();
        let content_id = id.clone();
        let footer_id = id.clone();
        BottomSheetFullPageUi::new(
            (self.id.clone(), page_key(id)),
            self.is_bottom_sheet.clone(),
            move |window, cx| sheet_content(&content_id, window, cx),
            move |window, cx| footer(&footer_id, window, cx),
        )
        .snap_fractions((self.snap_fractions)(id))
        .content_under_handle(self.content_under_handle)
        .footer_shows_top_border(self.footer_shows_top_border)
        .content_scrollable(self.content_scrollable)
        .sheet_background(self.sheet_background)
        .footer_background(self.footer_background)
        .backdrop_opacity(Some(0.0))
        .uses_full_page_layout(false)
        .top_corner_radius(Some(px(BottomSheetMetrics::EXPERIMENTAL_TOP_CORNER_RADIUS)))
        .appears_in_place(appears_in_place)
    }
}

impl<ID> RenderOnce for BottomSheetCarouselUi<ID>
where
    ID: Clone + Eq + Hash + 'static,
{
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let host_id = (self.id.clone(), "experimental-host");
        let is_presented = self.is_bottom_sheet.clone();
        let on_did_dismiss = self.on_did_dismiss.clone();
        let on_presentation_refused = self.on_presentation_refused.clone();
        let carousel = Rc::new(self);
        ExperimentalBottomSheetPresentationHost::new(host_id, is_presented, move |_, cx| carousel.strip(cx))
            .on_did_dismiss(move |window, cx| on_did_dismiss(window, cx))
            .on_presentation_refused(move |window, cx| on_presentation_refused(window, cx))
    }
}

/// element id of a page, stable for the page id across frames and list changes
fn page_key<ID: Hash>(id: &ID) -> SharedString {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    format!("page-{:016x}", hasher.finish()).into()
}

/// what keeps its own drag where a pointer lands so the strip declines it
/// carousel set is wider than the pager one, buttons are owners too
#[derive(Clone)]
pub enum BottomSheetCarouselAxisOwner {
    Control,
    TextView,
    ScrollView(ScrollHandle),
}

/// axis owners laid out on the carousel pages this frame
/// gpui has no hit test for which view is under a point so owners declare themselves
/// and the strip looks them up by bounds
#[derive(Clone, Default)]
pub struct BottomSheetCarouselAxisOwners(Rc<RefCell<Vec<(Bounds<Pixels>, BottomSheetCarouselAxisOwner)>>>);

impl BottomSheetCarouselAxisOwners {
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub fn register(&self, bounds: Bounds<Pixels>, owner: BottomSheetCarouselAxisOwner) {
        self.0.borrow_mut().push((bounds, owner));
    }

    /// true if any owner is under the point and claims it
    pub fn owns(&self, location: Point<Pixels>) -> bool {
        self.0.borrow().iter().any(|(bounds, owner)| {
            bounds.contains(&location)
                && match owner {
                    BottomSheetCarouselAxisOwner::Control | BottomSheetCarouselAxisOwner::TextView => true,
                    BottomSheetCarouselAxisOwner::ScrollView(scroll_view) => {
                        AnimatedTabsPagerPanIntentPolicy::scrolls_horizontally(scroll_view)
                    }
                }
        })
    }
}

/// registry of the carousel the subtree is in, none outside a carousel
pub struct BottomSheetCarouselAxisOwnersKey;

impl EnvKey for BottomSheetCarouselAxisOwnersKey {
    type Value = Option<BottomSheetCarouselAxisOwners>;

    fn default_value() -> Self::Value {
        None
    }
}

pub fn bottom_sheet_carousel_axis_owner(
    owner: BottomSheetCarouselAxisOwner,
    child: impl IntoElement,
) -> BottomSheetCarouselAxisOwnerElement {
    BottomSheetCarouselAxisOwnerElement { owner, child: child.into_any_element() }
}

pub struct BottomSheetCarouselAxisOwnerElement {
    owner: BottomSheetCarouselAxisOwner,
    child: AnyElement,
}

impl IntoElement for BottomSheetCarouselAxisOwnerElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for BottomSheetCarouselAxisOwnerElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(registry) = env::<BottomSheetCarouselAxisOwnersKey>(cx) {
            let visible = bounds.intersect(&window.content_mask().bounds);
            registry.register(visible, self.owner.clone());
        }
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

/// pages of the bottom sheet carousel
/// host layer already has the layout direction and stores are app globals here
/// so nothing is re applied
#[derive(IntoElement)]
struct BottomSheetCarouselStrip<ID>
where
    ID: Clone + Eq + Hash + 'static,
{
    id: ElementId,
    page_ids: Vec<ID>,
    selection: Option<ID>,
    spacing: f32,
    is_presented: bool,
    on_select: BottomSheetCarouselSelect<ID>,
    on_approach_end: BottomSheetCarouselAction,
    page: CarouselPage<ID>,
}

impl<ID> RenderOnce for BottomSheetCarouselStrip<ID>
where
    ID: Clone + Eq + Hash + 'static,
{
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let controller = Self::make_ui_view_controller(self.id.clone(), window, cx);
        let right_to_left = layout_direction(cx) == LayoutDirection::RightToLeft;
        let update = CarouselUpdate {
            page_ids: self.page_ids,
            selection: self.selection,
            spacing: self.spacing,
            is_presented: self.is_presented,
            right_to_left,
            on_select: self.on_select,
            on_approach_end: self.on_approach_end,
            page: self.page,
        };
        Self::update_ui_view_controller(&controller, update, window, cx);

        let now = RiseClock::monotonic();
        let (pages, position, dim_alpha, direction, stride, page, container_size) = {
            let state = controller.read(cx);
            let position = match &state.animator {
                Some(settle) => {
                    window.request_animation_frame();
                    settle.position(now)
                }
                None => state.position,
            };
            let pages: Vec<(usize, ID, bool)> = state
                .page_ids
                .iter()
                .enumerate()
                .filter(|(_, id)| state.hosts.contains(*id))
                .map(|(slot, id)| (slot, id.clone(), state.appears_in_place.get(id).copied().unwrap_or(true)))
                .collect();
            (
                pages,
                position,
                state.dim_alpha.frame_value(now, window),
                state.direction(),
                state.page_stride(),
                state.page.clone(),
                state.container_size(),
            )
        };

        let corner_radius = px(DeviceInfo::screen_corner_radius());
        let mut strip = div().absolute().inset_0();
        for (slot, id, appears_in_place) in pages {
            let x = direction * (slot as f32 - position) * stride;
            let off_strip = (slot as f32 - position).abs() >= 1.0;
            strip = strip.child(
                div()
                    .id((self.id.clone(), page_key(&id)))
                    .when(off_strip, |page| page.invisible())
                    .absolute()
                    .top_0()
                    .left(px(x))
                    .w(container_size.width)
                    .h(container_size.height)
                    .rounded_bl(corner_radius)
                    .rounded_br(corner_radius)
                    .overflow_hidden()
                    .child(page(&id, appears_in_place, window, cx)),
            );
        }

        let (axis_owners, pager_axis_owners) = {
            let state = controller.read(cx);
            (state.axis_owners.clone(), state.pager_axis_owners.clone())
        };
        axis_owners.clear();
        pager_axis_owners.clear();
        let strip = env_scope::<BottomSheetCarouselAxisOwnersKey>(Some(axis_owners)).child(strip);
        let strip = env_scope::<AnimatedTabsPagerAxisOwnersKey>(Some(pager_axis_owners)).child(strip);

        let dim = div()
            .absolute()
            .inset_0()
            .bg(black().opacity(BottomSheetMetrics::BACKDROP_OPACITY as f32 * dim_alpha));

        let measured = controller.downgrade();
        let attached = controller.downgrade();
        let pointer = controller.downgrade();
        let probe = canvas(
            move |bounds, _, cx| {
                measured.update(cx, |controller, _| controller.view_did_layout_subviews(bounds)).ok();
                bounds
            },
            move |_, bounds, window, _| {
                BottomSheetCarouselController::attach(&attached, window);
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let inside = bounds.contains(&event.position);
                    pointer
                        .update(cx, |controller, cx| {
                            if std::mem::replace(&mut controller.hovered, inside) != inside {
                                cx.notify();
                            }
                        })
                        .ok();
                });
            },
        )
        .absolute()
        .inset_0();

        let (focus, wants_focus, edges) = controller.update(cx, |controller, cx| {
            let focus = controller.focus_handle.get_or_insert_with(|| cx.focus_handle()).clone();
            if !controller.is_presented {
                controller.keys_focused = false;
            } else if focus.contains_focused(window, cx) {
                controller.keys_focused = true;
            }
            let wants_focus = controller.is_presented && !controller.keys_focused;
            let count = controller.page_ids.len();
            let has_previous = controller.index > 0;
            let has_next = controller.index + 1 < count;
            let edges = (controller.hovered && controller.is_presented && count > 1 && !controller.is_moving())
                .then(|| if controller.right_to_left { (has_next, has_previous) } else { (has_previous, has_next) });
            (focus, wants_focus, edges)
        });
        if wants_focus {
            window.focus(&focus, cx);
        }
        let (shows_left, shows_right) = edges.unwrap_or((false, false));
        let left_button = shows_left.then(|| {
            Self::edge_button((self.id.clone(), "edge-left").into(), "chevron.left", true, AccessibilityScroll::Right, controller.downgrade())
        });
        let right_button = shows_right.then(|| {
            Self::edge_button((self.id.clone(), "edge-right").into(), "chevron.right", false, AccessibilityScroll::Left, controller.downgrade())
        });
        let keys = controller.downgrade();
        let key_focus = focus.clone();

        let next = controller.downgrade();
        let previous = controller.downgrade();
        let teardown = controller;
        div()
            .id((self.id.clone(), "root"))
            .relative()
            .size_full()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified()
                    || !key_focus.contains_focused(window, cx)
                    || ScreenFocus::text_field_is_focused(window, cx)
                {
                    return;
                }
                let scroll = match event.keystroke.key.as_str() {
                    "left" => AccessibilityScroll::Right,
                    "right" => AccessibilityScroll::Left,
                    _ => return,
                };
                BottomSheetCarouselController::accessibility_scroll(&keys, scroll, window, cx);
                cx.stop_propagation();
            })
            .on_a11y_action(AccessibleAction::ScrollRight, move |_, window, cx| {
                BottomSheetCarouselController::accessibility_scroll(&next, AccessibilityScroll::Left, window, cx);
            })
            .on_a11y_action(AccessibleAction::ScrollLeft, move |_, window, cx| {
                BottomSheetCarouselController::accessibility_scroll(&previous, AccessibilityScroll::Right, window, cx);
            })
            .child(dim)
            .child(probe)
            .child(strip)
            .children(left_button)
            .children(right_button)
            .on_disappear((self.id, "dismantle"), move |_, cx| {
                Self::dismantle_ui_view_controller(&teardown, cx);
            })
    }
}

impl<ID> BottomSheetCarouselStrip<ID>
where
    ID: Clone + Eq + Hash + 'static,
{
    fn make_ui_view_controller(
        id: ElementId,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<BottomSheetCarouselController<ID>> {
        window.use_keyed_state(id, cx, |_, _| BottomSheetCarouselController::new())
    }

    fn update_ui_view_controller(
        controller: &Entity<BottomSheetCarouselController<ID>>,
        update: CarouselUpdate<ID>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let screen_size = window.viewport_size();
        let effects = controller.update(cx, |controller, cx| {
            controller.screen_size = screen_size;
            controller.update(update, cx)
        });
        if effects.is_empty() {
            return;
        }
        let deferred = controller.clone();
        window.defer(cx, move |window, cx| {
            BottomSheetCarouselController::run_effects(&deferred, effects, window, cx)
        });
    }

    fn dismantle_ui_view_controller(controller: &Entity<BottomSheetCarouselController<ID>>, cx: &mut App) {
        controller.update(cx, |controller, cx| controller.teardown(cx));
    }

    /// round chevron at a screen edge that pages one sheet toward it
    fn edge_button(
        id: ElementId,
        symbol: &'static str,
        at_left: bool,
        scroll: AccessibilityScroll,
        controller: WeakEntity<BottomSheetCarouselController<ID>>,
    ) -> AnyElement {
        let side = px(EDGE_BUTTON_SIZE);
        let column = div().absolute().top_0().bottom_0().flex().items_center();
        let column = if at_left { column.left(px(EDGE_BUTTON_INSET)) } else { column.right(px(EDGE_BUTTON_INSET)) };
        column
            .child(
                div().size(side).rounded_full().occlude().child(
                    plain(id)
                        .size(side)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(black().opacity(0.45))
                        .children(IconUi::sized(symbol, side * 0.5, white(), false))
                        .on_press(move |window, cx| {
                            BottomSheetCarouselController::accessibility_scroll(&controller, scroll, window, cx);
                        }),
                ),
            )
            .into_any_element()
    }
}

/// scroll directions the carousel answers for accessibility
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccessibilityScroll {
    Left,
    Right,
}

/// what the strip passes on every update
struct CarouselUpdate<ID> {
    page_ids: Vec<ID>,
    selection: Option<ID>,
    spacing: f32,
    is_presented: bool,
    right_to_left: bool,
    on_select: BottomSheetCarouselSelect<ID>,
    on_approach_end: BottomSheetCarouselAction,
    page: CarouselPage<ID>,
}

/// owner callbacks a change produced, run outside the controller borrow
enum CarouselEffect<ID> {
    Select(ID),
    ApproachEnd,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum SettleTiming {
    EaseOut,
    Spring { damping: f32, initial_velocity: f32 },
}

/// running settle animator of the strip
#[derive(Clone, Copy, Debug)]
struct CarouselSettle {
    from: f32,
    to: f32,
    start: Duration,
    duration: f64,
    timing: SettleTiming,
    generation: u64,
}

impl CarouselSettle {
    fn progress(&self, now: Duration) -> f32 {
        let elapsed = now.saturating_sub(self.start).as_secs_f64();
        if elapsed >= self.duration || self.duration <= 0.0 {
            return 1.0;
        }
        let fraction = elapsed / self.duration;
        match self.timing {
            SettleTiming::EaseOut => CubicBezier::EASE_OUT.progress(fraction) as f32,
            SettleTiming::Spring { damping, initial_velocity } => {
                let spring = Spring::new(self.duration, f64::from(damping));
                let settling = spring.settling_time(1.0, 0.0, 0.001).max(f64::EPSILON);
                let scale = settling / self.duration;
                let (displacement, _) =
                    spring.state(-1.0, f64::from(initial_velocity) / scale, elapsed * scale);
                (1.0 + displacement) as f32
            }
        }
    }

    fn position(&self, now: Duration) -> f32 {
        self.from + (self.to - self.from) * self.progress(now)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackingSource {
    Mouse,
    Trackpad,
}

/// one pointer session from press to release, before and after the pan is recognized
struct Tracking {
    source: TrackingSource,
    origin: Point<Pixels>,
    translation: Point<Pixels>,
    samples: Vec<(Instant, Point<Pixels>)>,
}

impl Tracking {
    fn new(source: TrackingSource, location: Point<Pixels>, now: Instant) -> Self {
        Self {
            source,
            origin: location,
            translation: Point::default(),
            samples: vec![(now, Point::default())],
        }
    }

    fn record(&mut self, now: Instant) {
        self.samples.push((now, self.translation));
        self.samples
            .retain(|(time, _)| now.saturating_duration_since(*time) <= VELOCITY_WINDOW);
    }

    /// points per second
    fn velocity(&self) -> Point<Pixels> {
        let (Some(first), Some(last)) = (self.samples.first(), self.samples.last()) else {
            return Point::default();
        };
        let seconds = last.0.saturating_duration_since(first.0).as_secs_f32();
        if seconds <= f32::EPSILON {
            return Point::default();
        }
        point(
            px(f32::from(last.1.x - first.1.x) / seconds),
            px(f32::from(last.1.y - first.1.y) / seconds),
        )
    }
}

/// owns the strip, which pages are mounted and where it stands
/// also the horizontal pan and the settle
/// positions are continuous page indices, page i stands i strides along and the strip shows position
struct BottomSheetCarouselController<ID>
where
    ID: Clone + Eq + Hash + 'static,
{
    hosts: HashSet<ID>,
    appears_in_place: HashMap<ID, bool>,
    page_ids: Vec<ID>,
    latest_page_ids: Vec<ID>,
    latest_selection: Option<ID>,
    index: usize,
    position: f32,
    spacing: f32,
    right_to_left: bool,
    is_presented: bool,
    has_opened: bool,
    reported_id: Option<ID>,
    page: CarouselPage<ID>,
    on_select: BottomSheetCarouselSelect<ID>,
    on_approach_end: BottomSheetCarouselAction,

    pan: BottomSheetGestureRecognizer,
    tracking: Option<Tracking>,
    animator: Option<CarouselSettle>,
    animator_generation: u64,
    is_tracking: bool,
    gesture_index: usize,
    drag_start_position: f32,
    activation_translation: Pixels,

    dim_alpha: Animated<f32>,
    screen_size: Size<Pixels>,
    view_bounds: Option<Bounds<Pixels>>,
    axis_owners: BottomSheetCarouselAxisOwners,
    pager_axis_owners: AnimatedTabsPagerAxisOwners,

    focus_handle: Option<FocusHandle>,
    keys_focused: bool,
    hovered: bool,
    wheel_accumulated: f32,
    wheel_quiet_until: Option<Instant>,
}

impl<ID> BottomSheetCarouselController<ID>
where
    ID: Clone + Eq + Hash + 'static,
{
    const HORIZONTAL_DOMINANCE: f32 = 1.08;

    fn new() -> Self {
        Self {
            hosts: HashSet::new(),
            appears_in_place: HashMap::new(),
            page_ids: Vec::new(),
            latest_page_ids: Vec::new(),
            latest_selection: None,
            index: 0,
            position: 0.0,
            spacing: BottomSheetMetrics::CAROUSEL_PAGE_SPACING,
            right_to_left: false,
            is_presented: false,
            has_opened: false,
            reported_id: None,
            page: Rc::new(|_: &ID, _: bool, _: &mut Window, _: &mut App| div().into_any_element()),
            on_select: Rc::new(|_: ID, _: &mut Window, _: &mut App| {}),
            on_approach_end: Rc::new(|_: &mut Window, _: &mut App| {}),
            pan: BottomSheetGestureRecognizer::new(),
            tracking: None,
            animator: None,
            animator_generation: 0,
            is_tracking: false,
            gesture_index: 0,
            drag_start_position: 0.0,
            activation_translation: px(0.0),
            dim_alpha: Animated::new(0.0),
            screen_size: Size::default(),
            view_bounds: None,
            axis_owners: BottomSheetCarouselAxisOwners::default(),
            pager_axis_owners: AnimatedTabsPagerAxisOwners::default(),
            focus_handle: None,
            keys_focused: false,
            hovered: false,
            wheel_accumulated: 0.0,
            wheel_quiet_until: None,
        }
    }

    fn direction(&self) -> f32 {
        if self.right_to_left { -1.0 } else { 1.0 }
    }

    /// carousel covers the window, until its first layout the window is its size
    fn container_size(&self) -> Size<Pixels> {
        match self.view_bounds {
            Some(bounds) if bounds.size.width > px(0.0) && bounds.size.height > px(0.0) => bounds.size,
            _ => self.screen_size,
        }
    }

    fn page_stride(&self) -> f32 {
        f32::from(self.container_size().width).max(1.0) + self.spacing
    }

    fn view_did_layout_subviews(&mut self, bounds: Bounds<Pixels>) {
        self.view_bounds = Some(bounds);
    }

    fn is_moving(&self) -> bool {
        self.is_tracking || self.animator.is_some()
    }

    fn update(&mut self, update: CarouselUpdate<ID>, cx: &mut Context<Self>) -> Vec<CarouselEffect<ID>> {
        self.on_select = update.on_select;
        self.on_approach_end = update.on_approach_end;
        self.page = update.page;
        self.spacing = update.spacing;
        self.right_to_left = update.right_to_left;

        if update.is_presented != self.is_presented {
            self.is_presented = update.is_presented;
            self.fade_dim(update.is_presented, cx);
            if !update.is_presented {
                self.has_opened = false;
                for appears_in_place in self.appears_in_place.values_mut() {
                    *appears_in_place = false;
                }
                self.cancel_tracking(cx);
            }
        }

        if !update.is_presented {
            return Vec::new();
        }

        self.latest_page_ids = update.page_ids;
        self.latest_selection = update.selection;
        if self.is_moving() {
            return Vec::new();
        }
        self.reconcile(cx)
    }

    fn teardown(&mut self, cx: &mut Context<Self>) {
        self.animator = None;
        self.cancel_tracking(cx);
        self.hosts.clear();
        self.appears_in_place.clear();
    }

    fn reconcile(&mut self, cx: &mut Context<Self>) -> Vec<CarouselEffect<ID>> {
        let ids = self.latest_page_ids.clone();
        if ids.is_empty() {
            return Vec::new();
        }

        let shown = self.page_ids.get(self.index).cloned();
        let kept = shown
            .filter(|_| self.has_opened)
            .and_then(|shown| ids.iter().position(|id| *id == shown));
        let selected = self
            .latest_selection
            .as_ref()
            .and_then(|selection| ids.iter().position(|id| id == selection));
        let target = kept.or(selected).unwrap_or_else(|| self.index.min(ids.len() - 1));

        let changed = !self.has_opened
            || self.index != target
            || self.position != target as f32
            || self.page_ids != ids;
        self.page_ids = ids;
        self.index = target;
        self.position = target as f32;
        self.mount_window();
        self.has_opened = true;
        if changed {
            cx.notify();
        }

        let mut effects = Vec::new();
        let id = self.page_ids[target].clone();
        if self.reported_id.is_none() {
            self.reported_id = Some(self.latest_selection.clone().unwrap_or_else(|| id.clone()));
        }
        if self.reported_id.as_ref() != Some(&id) {
            self.reported_id = Some(id.clone());
            effects.push(CarouselEffect::Select(id));
        }
        effects
    }

    fn window_indices(&self) -> Option<RangeInclusive<usize>> {
        if self.page_ids.is_empty() {
            return None;
        }
        let last = self.page_ids.len() - 1;
        let clamped = self.index.min(last);
        Some(clamped.saturating_sub(1)..=(clamped + 1).min(last))
    }

    /// page on screen and its neighbours stay mounted, the rest leave
    /// pages mounted with the opening rise with it and later ones just appear
    fn mount_window(&mut self) {
        let Some(window) = self.window_indices() else {
            return;
        };
        let wanted: HashSet<ID> = window.clone().map(|slot| self.page_ids[slot].clone()).collect();
        self.hosts.retain(|id| wanted.contains(id));
        self.appears_in_place.retain(|id, _| wanted.contains(id));
        for slot in window {
            let id = self.page_ids[slot].clone();
            if self.hosts.contains(&id) {
                continue;
            }
            self.appears_in_place.insert(id.clone(), self.has_opened);
            self.hosts.insert(id);
        }
    }

    fn fade_dim(&mut self, visible: bool, cx: &mut Context<Self>) {
        let duration = if reduce_motion(cx) { 0.01 } else { BottomSheetMetrics::BACKDROP_FADE_DURATION };
        let target = if visible { 1.0 } else { 0.0 };
        self.dim_alpha.set(target, ease_out(duration), RiseClock::monotonic(), false);
        cx.notify();
    }

    fn touches_began(&mut self, source: TrackingSource, location: Point<Pixels>, now: Instant) {
        if self.is_tracking || self.tracking.is_some() {
            return;
        }
        if !self.view_bounds.is_some_and(|bounds| bounds.contains(&location)) {
            return;
        }
        if self.starts_on_own_axis_owner(location) {
            return;
        }
        self.tracking = Some(Tracking::new(source, location, now));
    }

    fn touches_moved(
        &mut self,
        translation: Point<Pixels>,
        now: Instant,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tracking) = self.tracking.as_mut() else {
            return false;
        };
        tracking.translation = translation;
        tracking.record(now);
        let source = tracking.source;
        let origin = tracking.origin;
        let velocity = tracking.velocity();

        if self.is_tracking {
            self.pan.set_state(BottomSheetGestureState::Changed);
            let moved = -self.direction() * f32::from(translation.x - self.activation_translation)
                / self.page_stride();
            self.position = self.constrained(self.drag_start_position + moved);
            cx.notify();
            return true;
        }

        let distance = f32::from(translation.x).hypot(f32::from(translation.y));
        if distance < PAN_HYSTERESIS {
            return source == TrackingSource::Trackpad
                && self.is_presented
                && self.page_ids.len() > 1
                && translation.x.abs() > translation.y.abs() * Self::HORIZONTAL_DOMINANCE;
        }

        if !self.gesture_recognizer_should_begin(origin, translation, velocity, cx) {
            self.tracking = None;
            return false;
        }

        let now_motion = RiseClock::monotonic();
        self.interrupt_settle(now_motion);
        self.is_tracking = true;
        self.pan.set_state(BottomSheetGestureState::Began);
        BottomSheetAxisOwnership::claim_horizontal(&self.pan, cx);
        self.activation_translation = translation.x;
        self.drag_start_position = self.position;
        self.gesture_index = self.index;
        cx.notify();
        true
    }

    fn touches_ended(&mut self, now: Instant, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(mut tracking) = self.tracking.take() else {
            return false;
        };
        if !self.is_tracking {
            return false;
        }
        tracking.record(now);
        let velocity = tracking.velocity().x;
        self.finish_tracking(cx);
        self.settle(f32::from(velocity), window, cx);
        true
    }

    fn touches_cancelled(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.tracking = None;
        if !self.is_tracking {
            return false;
        }
        self.finish_tracking(cx);
        self.settle(0.0, window, cx);
        true
    }

    /// stays within one page either side of where the gesture started
    /// past the first and last page the strip resists instead of stopping dead
    fn constrained(&self, raw: f32) -> f32 {
        let lower = self.gesture_index.saturating_sub(1) as f32;
        let upper = (self.gesture_index + 1).min(self.page_ids.len().saturating_sub(1)) as f32;
        if raw < lower {
            return lower - Self::edge_overshoot(lower - raw);
        }
        if raw > upper {
            return upper + Self::edge_overshoot(raw - upper);
        }
        raw
    }

    fn edge_overshoot(distance: f32) -> f32 {
        BottomSheetMetrics::CAROUSEL_EDGE_MAXIMUM_OVERSHOOT
            .min(distance * BottomSheetMetrics::CAROUSEL_EDGE_RESISTANCE)
    }

    fn finish_tracking(&mut self, cx: &mut Context<Self>) {
        self.is_tracking = false;
        self.activation_translation = px(0.0);
        self.pan.set_state(BottomSheetGestureState::Ended);
        BottomSheetAxisOwnership::release_horizontal(&self.pan, cx);
    }

    fn cancel_tracking(&mut self, cx: &mut Context<Self>) {
        self.tracking = None;
        if !self.is_tracking {
            return;
        }
        self.finish_tracking(cx);
        self.position = self.index as f32;
        cx.notify();
    }

    /// release rule of the in sheet back swipe, applied only in the direction the pointer moved
    /// a flick back from a half shown page returns to the start instead of throwing the strip past it
    fn settle(&mut self, velocity_x: f32, window: &mut Window, cx: &mut Context<Self>) {
        let width = px(f32::from(self.container_size().width).max(1.0));
        let stride = self.page_stride();
        let finger_toward_next = (self.position - self.drag_start_position) * stride;
        let velocity_toward_next = -self.direction() * velocity_x;
        let mut target = self.gesture_index;
        if self.gesture_index + 1 < self.page_ids.len()
            && finger_toward_next >= 0.0
            && BottomSheetPageBackSwipePolicy::commits(
                px(finger_toward_next),
                px(velocity_toward_next),
                width,
            )
        {
            target = self.gesture_index + 1;
        } else if self.gesture_index > 0
            && finger_toward_next <= 0.0
            && BottomSheetPageBackSwipePolicy::commits(
                px(-finger_toward_next),
                px(-velocity_toward_next),
                width,
            )
        {
            target = self.gesture_index - 1;
        }
        self.animate(target, velocity_toward_next / stride, window, cx);
    }

    fn animate(&mut self, target: usize, page_velocity: f32, window: &mut Window, cx: &mut Context<Self>) {
        let distance = target as f32 - self.position;
        let reduce_motion = reduce_motion(cx);
        let timing = if reduce_motion {
            SettleTiming::EaseOut
        } else {
            let relative = if distance.abs() > 0.001 { page_velocity / distance } else { 0.0 };
            SettleTiming::Spring {
                damping: BottomSheetMetrics::CAROUSEL_SETTLE_DAMPING,
                initial_velocity: relative.clamp(-30.0, 30.0),
            }
        };
        let duration = if reduce_motion { 0.12 } else { BottomSheetMetrics::CAROUSEL_SETTLE_DURATION };
        self.animator_generation = self.animator_generation.wrapping_add(1);
        let generation = self.animator_generation;
        self.animator = Some(CarouselSettle {
            from: self.position,
            to: target as f32,
            start: RiseClock::monotonic(),
            duration,
            timing,
            generation,
        });
        cx.notify();

        let delay = duration_from_seconds(duration);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Some(controller) = this.upgrade() else {
                return;
            };
            cx.update(|window, cx| {
                let effects = controller.update(cx, |controller, cx| controller.animation_ended(generation, target, cx));
                Self::run_effects(&controller, effects, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn animation_ended(&mut self, generation: u64, target: usize, cx: &mut Context<Self>) -> Vec<CarouselEffect<ID>> {
        if self.animator.is_none_or(|settle| settle.generation != generation) {
            return Vec::new();
        }
        self.animator = None;
        self.position = target as f32;
        self.finish_settle(target, cx)
    }

    fn interrupt_settle(&mut self, now: Duration) {
        let Some(settle) = self.animator.take() else {
            return;
        };
        self.position = settle.position(now);
        let last = self.page_ids.len().saturating_sub(1);
        let nearest = (self.position.round().max(0.0) as usize).min(last);
        if nearest != self.index {
            self.index = nearest;
            self.mount_window();
        }
    }

    fn finish_settle(&mut self, target: usize, cx: &mut Context<Self>) -> Vec<CarouselEffect<ID>> {
        self.index = target;
        let mut effects = if self.latest_page_ids != self.page_ids {
            self.reconcile(cx)
        } else {
            self.mount_window();
            Vec::new()
        };
        cx.notify();
        let Some(id) = self.page_ids.get(self.index).cloned() else {
            return effects;
        };
        if self.reported_id.as_ref() != Some(&id) {
            self.reported_id = Some(id.clone());
            effects.push(CarouselEffect::Select(id));
        }
        if self.index + 2 >= self.page_ids.len() {
            effects.push(CarouselEffect::ApproachEnd);
        }
        effects
    }

    /// scroll for accessibility
    /// no gpui twin for the page scrolled announcement, the moved strip is what the reader reads next
    fn scroll_for_accessibility(
        &mut self,
        scroll: AccessibilityScroll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_presented || self.is_moving() || self.page_ids.len() <= 1 {
            return false;
        }
        let step: isize = match scroll {
            AccessibilityScroll::Left => 1,
            AccessibilityScroll::Right => -1,
        };
        let step = if self.right_to_left { -step } else { step };
        let Some(target) = self.index.checked_add_signed(step) else {
            return false;
        };
        if target >= self.page_ids.len() {
            return false;
        }
        self.gesture_index = self.index;
        self.animate(target, 0.0, window, cx);
        true
    }

    fn wheel_step(
        &mut self,
        position: Point<Pixels>,
        delta: Point<Pixels>,
        now: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_presented || self.page_ids.len() <= 1 || BottomSheetAxisOwnership::vertical_is_driving(cx) {
            return false;
        }
        if !self.view_bounds.is_some_and(|bounds| bounds.contains(&position)) || self.starts_on_own_axis_owner(position) {
            return false;
        }
        let dx = f32::from(delta.x);
        if dx.abs() <= f32::from(delta.y).abs() * Self::HORIZONTAL_DOMINANCE {
            return false;
        }
        if self.is_moving() || self.wheel_quiet_until.is_some_and(|until| now < until) {
            self.wheel_quiet_until = Some(now + WHEEL_QUIET);
            self.wheel_accumulated = 0.0;
            return true;
        }
        self.wheel_accumulated += dx;
        if self.wheel_accumulated.abs() < WHEEL_STEP_DISTANCE {
            return true;
        }
        let scroll = if self.wheel_accumulated < 0.0 { AccessibilityScroll::Left } else { AccessibilityScroll::Right };
        self.wheel_accumulated = 0.0;
        self.wheel_quiet_until = Some(now + WHEEL_QUIET);
        self.scroll_for_accessibility(scroll, window, cx);
        true
    }

    fn accessibility_scroll(
        this: &WeakEntity<Self>,
        scroll: AccessibilityScroll,
        window: &mut Window,
        cx: &mut App,
    ) {
        this.update(cx, |controller, cx| controller.scroll_for_accessibility(scroll, window, cx))
            .ok();
    }

    fn gesture_recognizer_should_begin(
        &self,
        start: Point<Pixels>,
        translation: Point<Pixels>,
        velocity: Point<Pixels>,
        cx: &App,
    ) -> bool {
        if !self.is_presented
            || self.page_ids.len() <= 1
            || BottomSheetAxisOwnership::vertical_is_driving(cx)
        {
            return false;
        }

        if translation.x.abs() <= translation.y.abs() * Self::HORIZONTAL_DOMINANCE {
            return false;
        }
        if velocity.x.abs() < velocity.y.abs() {
            return false;
        }

        self.view_bounds.is_some_and(|bounds| bounds.contains(&start)) && !self.starts_on_own_axis_owner(start)
    }

    /// a control, text view or horizontal list under the pointer keeps its own swipe
    fn starts_on_own_axis_owner(&self, point: Point<Pixels>) -> bool {
        self.axis_owners.owns(point) || self.pager_axis_owners.owns(point)
    }

    fn tracks(&self, source: TrackingSource) -> bool {
        self.tracking.as_ref().is_some_and(|tracking| tracking.source == source)
    }

    fn run_effects(
        this: &Entity<Self>,
        effects: Vec<CarouselEffect<ID>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if effects.is_empty() {
            return;
        }
        let (on_select, on_approach_end) = {
            let controller = this.read(cx);
            (controller.on_select.clone(), controller.on_approach_end.clone())
        };
        for effect in effects {
            match effect {
                CarouselEffect::Select(id) => on_select(id, window, cx),
                CarouselEffect::ApproachEnd => on_approach_end(window, cx),
            }
        }
    }

    fn attach(this: &WeakEntity<Self>, window: &mut Window) {
        let down = this.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            let now = cx.background_executor().now();
            down.update(cx, |controller, _| {
                controller.touches_began(TrackingSource::Mouse, event.position, now)
            })
            .ok();
        });

        let moved = this.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            let now = cx.background_executor().now();
            let consumed = moved
                .update(cx, |controller, cx| {
                    if !controller.tracks(TrackingSource::Mouse) {
                        return false;
                    }
                    if event.pressed_button != Some(MouseButton::Left) {
                        return controller.touches_cancelled(window, cx);
                    }
                    let Some(origin) = controller.tracking.as_ref().map(|tracking| tracking.origin) else {
                        return false;
                    };
                    controller.touches_moved(event.position - origin, now, cx)
                })
                .unwrap_or(false);
            if consumed {
                cx.stop_propagation();
            }
        });

        let up = this.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            let now = cx.background_executor().now();
            let consumed = up
                .update(cx, |controller, cx| {
                    controller.tracks(TrackingSource::Mouse) && controller.touches_ended(now, window, cx)
                })
                .unwrap_or(false);
            if consumed {
                cx.stop_propagation();
            }
        });

        let wheel = this.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            let now = cx.background_executor().now();
            let line_height = window.line_height();
            let consumed = wheel
                .update(cx, |controller, cx| match event.touch_phase {
                    _ if !event.delta.precise()
                        || (event.touch_phase == TouchPhase::Moved && controller.tracking.is_none()) =>
                    {
                        controller.wheel_step(event.position, event.delta.pixel_delta(line_height), now, window, cx)
                    }
                    TouchPhase::Started => {
                        controller.touches_began(TrackingSource::Trackpad, event.position, now);
                        false
                    }
                    TouchPhase::Moved => {
                        let Some(tracking) = controller.tracking.as_ref() else {
                            return false;
                        };
                        if tracking.source != TrackingSource::Trackpad {
                            return false;
                        }
                        let translation = tracking.translation + event.delta.pixel_delta(line_height);
                        controller.touches_moved(translation, now, cx)
                    }
                    TouchPhase::Ended => {
                        if !controller.tracks(TrackingSource::Trackpad) {
                            return false;
                        }
                        controller.wheel_quiet_until = Some(now + WHEEL_QUIET);
                        controller.touches_ended(now, window, cx)
                    }
                    TouchPhase::Cancelled => {
                        controller.tracks(TrackingSource::Trackpad)
                            && controller.touches_cancelled(window, cx)
                    }
                })
                .unwrap_or(false);
            if consumed {
                cx.stop_propagation();
            }
        });
    }
}

impl<ID> Drop for BottomSheetCarouselController<ID>
where
    ID: Clone + Eq + Hash + 'static,
{
    fn drop(&mut self) {
        self.pan.set_state(BottomSheetGestureState::Cancelled);
    }
}

