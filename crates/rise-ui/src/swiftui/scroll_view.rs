use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, Context, DispatchPhase, Div, ElementId, Entity, HitboxBehavior, InteractiveElement,
    IntoElement, ListState, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    RenderOnce, ScrollHandle, ScrollWheelEvent, Stateful, StatefulInteractiveElement, StyleRefinement, Styled, Task,
    TouchPhase, Window, canvas, div, fill, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::wheel_gesture::WHEEL_SETTLE_DELAY;
use smallvec::SmallVec;

use crate::guide_anchor_ui::guide_scroll_host::{GuideScrollHostMount, GuideScrollHostScope};
use crate::swiftui::environment::{EnvKey, EnvScope, env, env_scope};
use crate::swiftui::progress_view::progress_view;

pub const INDICATOR_WIDTH: f32 = 3.0;

pub const INDICATOR_INSET: f32 = 3.0;

pub const INDICATOR_MIN_LENGTH: f32 = 36.0;

pub const INDICATOR_OPACITY: f32 = 0.35;

pub fn indicator_fill(theme: &rise_theme::AppTheme) -> gpui::Hsla {
    theme.primary._100.opacity(INDICATOR_OPACITY)
}

pub const INDICATOR_LINGER: Duration = Duration::from_millis(600);

pub const REFRESH_TRIGGER_DISTANCE: f32 = 60.0;

pub const REFRESH_INSET: f32 = 60.0;

pub const RUBBER_BAND_COEFFICIENT: f32 = 0.55;

pub const PULL_RELEASE_DELAY: Duration = Duration::from_millis(400);

const TOP_TOLERANCE: f32 = 0.5;

/// visible pull for a raw trackpad pull past the edge of a view
/// rubber banding, slows down the further it goes and never reaches the view length
pub fn rubber_band(pull: f32, dimension: f32) -> f32 {
    if pull <= 0.0 || dimension <= 0.0 {
        return 0.0;
    }
    (1.0 - 1.0 / (pull * RUBBER_BAND_COEFFICIENT / dimension + 1.0)) * dimension
}

/// raw pull after one scroll step, delta is positive when pulling down
/// ovERscroll is how far content sits past its top after the step
pub fn pull_step(pull: f32, delta: f32, overscroll: f32) -> f32 {
    if delta >= 0.0 {
        pull + delta.min(overscroll.max(0.0))
    } else {
        (pull + delta).max(0.0)
    }
}

/// top and length of the scroll thumb, none when nothing scrolls
pub fn indicator_thumb(viewport: f32, max_offset: f32, scrolled: f32) -> Option<(f32, f32)> {
    if viewport <= 0.0 || max_offset <= 0.0 {
        return None;
    }
    let track = viewport - 2.0 * INDICATOR_INSET;
    if track <= 0.0 {
        return None;
    }
    let content = viewport + max_offset;
    let length = (track * viewport / content).clamp(INDICATOR_MIN_LENGTH.min(track), track);
    let progress = (scrolled / max_offset).clamp(0.0, 1.0);
    Some((INDICATOR_INSET + (track - length) * progress, length))
}

/// action that refreshable runs, the task it returns is the await of its body
pub type RefreshAction = Rc<dyn Fn(&mut Window, &mut App) -> Task<()>>;

/// what one scroll step did to the pull
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PullStep {
    pub(crate) starts: bool,
    pub(crate) absorbed: f32,
}

impl PullStep {
    const NONE: Self = Self { starts: false, absorbed: 0.0 };
}

/// pull past the top edge and the refresh it starts, shared by every scrollable with refreshable
pub(crate) struct RefreshPull {
    pull: f32,
    pull_spent: bool,
    refreshing: bool,
    coast_seen: Option<Duration>,
    _refresh_task: Option<Task<()>>,
    _release_task: Option<Task<()>>,
}

impl RefreshPull {
    pub(crate) fn new() -> Self {
        Self {
            pull: 0.0,
            pull_spent: false,
            refreshing: false,
            coast_seen: None,
            _refresh_task: None,
            _release_task: None,
        }
    }

    pub(crate) fn pull(&self) -> f32 {
        self.pull
    }

    pub(crate) fn is_refreshing(&self) -> bool {
        self.refreshing
    }

    /// how far the content sits below its place in a view of this size
    pub(crate) fn shown_pull(&self, dimension: f32) -> f32 {
        if self.refreshing { REFRESH_INSET } else { rubber_band(self.pull, dimension) }
    }

    pub(crate) fn track_pull(
        &mut self,
        event: &ScrollWheelEvent,
        delta: f32,
        overscroll: f32,
        dimension: f32,
    ) -> PullStep {
        if !event.delta.precise() {
            self.pull = 0.0;
            return PullStep::NONE;
        }
        let now = RiseClock::monotonic();
        match event.touch_phase {
            TouchPhase::Started => {
                self.pull = 0.0;
                self.pull_spent = false;
                self.coast_seen = None;
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.pull = 0.0;
                self.pull_spent = false;
                self.coast_seen = Some(now);
                return PullStep::NONE;
            }
            TouchPhase::Moved => {
                if let Some(seen) = self.coast_seen {
                    if now.saturating_sub(seen) < WHEEL_SETTLE_DELAY {
                        self.coast_seen = Some(now);
                        self.pull = 0.0;
                        return PullStep::NONE;
                    }
                    self.coast_seen = None;
                    self.pull_spent = false;
                }
            }
        }
        if self.pull_spent {
            return PullStep::NONE;
        }
        let before = self.pull;
        self.pull = pull_step(self.pull, delta, overscroll);
        let absorbed = if delta < 0.0 { (before - self.pull).max(0.0) } else { 0.0 };
        if self.refreshing || rubber_band(self.pull, dimension) < REFRESH_TRIGGER_DISTANCE {
            return PullStep { starts: false, absorbed };
        }
        self.pull = 0.0;
        self.pull_spent = true;
        PullStep { starts: true, absorbed }
    }

    /// springs an open pull back once no step came for the release delay, call after every tracked step
    /// this finds the pull again inside the owning state
    pub(crate) fn arm_release<T: 'static>(&mut self, this: fn(&mut T) -> &mut RefreshPull, cx: &mut Context<T>) {
        if self.pull <= 0.0 {
            self._release_task = None;
            return;
        }
        let executor = cx.background_executor().clone();
        self._release_task = Some(cx.spawn(async move |state, cx| {
            executor.timer(PULL_RELEASE_DELAY).await;
            state
                .update(cx, |state, cx| {
                    let refresh = this(state);
                    if refresh.pull > 0.0 {
                        refresh.pull = 0.0;
                        cx.notify();
                    }
                })
                .ok();
        }));
    }

    /// starts the refresh, this finds the pull again in the owning state once work is done
    pub(crate) fn begin_refresh<T: 'static>(
        &mut self,
        work: Task<()>,
        this: fn(&mut T) -> &mut RefreshPull,
        cx: &mut Context<T>,
    ) {
        self.refreshing = true;
        self._refresh_task = Some(cx.spawn(async move |state, cx| {
            work.await;
            state
                .update(cx, |state, cx| {
                    this(state).refreshing = false;
                    cx.notify();
                })
                .ok();
        }));
        cx.notify();
    }
}

/// band above the content as tall as the pull, with the spinner
pub(crate) fn refresh_header(shown_pull: f32, refreshing: bool) -> Div {
    let reveal = if refreshing { 1.0 } else { (shown_pull / REFRESH_TRIGGER_DISTANCE).min(1.0) };
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(shown_pull))
        .flex()
        .items_center()
        .justify_center()
        .opacity(reveal)
        .child(progress_view())
}

/// bottom content margin a subtree hands to its scroll views
pub struct ScrollContentBottomMarginKey;

impl EnvKey for ScrollContentBottomMarginKey {
    type Value = f32;

    fn default_value() -> f32 {
        0.0
    }
}

pub trait ContentMarginsModifiers: IntoElement + Sized {
    fn content_margins_bottom(self, length: f32) -> EnvScope<ScrollContentBottomMarginKey> {
        env_scope::<ScrollContentBottomMarginKey>(length).child(self)
    }
}

impl<E: IntoElement> ContentMarginsModifiers for E {}

/// bottom content margin of the enclosing subtree, read in render inside the scope
pub fn scroll_content_bottom_margin(cx: &App) -> f32 {
    env::<ScrollContentBottomMarginKey>(cx)
}

/// vertical scroll view, fills the space its parent offers
pub fn scroll_view(id: impl Into<ElementId>) -> ScrollView {
    let id = id.into();
    ScrollView {
        base: div().id(id.clone()).relative().size_full().overflow_hidden(),
        id,
        children: SmallVec::new(),
        shows_indicators: true,
        scroll_disabled: false,
        restricts_to_axis: false,
        scroll_handle: None,
        refresh: None,
        content_top_margin: None,
    }
}

#[derive(IntoElement)]
pub struct ScrollView {
    id: ElementId,
    base: Stateful<Div>,
    children: SmallVec<[AnyElement; 2]>,
    shows_indicators: bool,
    scroll_disabled: bool,
    restricts_to_axis: bool,
    scroll_handle: Option<ScrollHandle>,
    refresh: Option<RefreshAction>,
    content_top_margin: Option<fn(&App) -> f32>,
}

impl ScrollView {
    pub fn shows_indicators(mut self, shows: bool) -> Self {
        self.shows_indicators = shows;
        self
    }

    pub fn scroll_disabled(mut self, disabled: bool) -> Self {
        self.scroll_disabled = disabled;
        self
    }

    /// handle of a scroll whose offset someone else drives, read once when the view first appears
    pub fn scroll_handle(mut self, handle: ScrollHandle) -> Self {
        self.scroll_handle = Some(handle);
        self
    }

    /// top padding inside the scroller, rows start below it and scroll up through it
    /// takes a source not a value, the margin comes from an env a parent publishes
    pub fn content_top_margin(mut self, source: fn(&App) -> f32) -> Self {
        self.content_top_margin = Some(source);
        self
    }

    /// a pure horizontal trackpad step stays away from this view, it sits on a horizontal pager
    pub fn restrict_scroll_to_axis(mut self, restricts: bool) -> Self {
        self.restricts_to_axis = restricts;
        self
    }

    /// pull to refresh, a pull while the last refresh still runs starts none
    pub fn refreshable(mut self, action: impl Fn(&mut Window, &mut App) -> Task<()> + 'static) -> Self {
        self.refresh = Some(Rc::new(action));
        self
    }
}

impl ParentElement for ScrollView {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for ScrollView {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

/// what a scroll view keeps between frames
struct ScrollViewState {
    handle: ScrollHandle,
    last_scroll: Option<Duration>,
    refresh: RefreshPull,
    _fade: Option<Task<()>>,
    guide_host: GuideScrollHostMount,
}

impl ScrollViewState {
    fn new(handle: ScrollHandle) -> Self {
        Self {
            guide_host: GuideScrollHostMount::new(handle.clone()),
            handle,
            last_scroll: None,
            refresh: RefreshPull::new(),
            _fade: None,
        }
    }

    fn scrolled(&mut self, has_thumb: bool, cx: &mut Context<Self>) {
        let was_visible = self.indicator_visible();
        self.last_scroll = Some(RiseClock::monotonic());
        if !has_thumb {
            self._fade = None;
            return;
        }
        let executor = cx.background_executor().clone();
        self._fade = Some(cx.spawn(async move |state, cx| {
            executor.timer(INDICATOR_LINGER).await;
            state.update(cx, |_, cx| cx.notify()).ok();
        }));
        if !was_visible {
            cx.notify();
        }
    }

    fn indicator_visible(&self) -> bool {
        self.last_scroll
            .is_some_and(|at| RiseClock::monotonic().saturating_sub(at) < INDICATOR_LINGER)
    }
}

impl RenderOnce for ScrollView {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let initial_handle = self.scroll_handle.take().unwrap_or_else(ScrollHandle::new);
        let state = window.use_keyed_state(self.id.clone(), cx, move |_, _| ScrollViewState::new(initial_handle));
        let (handle, refreshing, indicator_visible, guide_host) = {
            let current = state.read(cx);
            (
                current.handle.clone(),
                current.refresh.is_refreshing(),
                current.indicator_visible(),
                current.guide_host.host(),
            )
        };
        let disabled = self.scroll_disabled;
        guide_host.set_scroll_enabled(!disabled);
        let viewport = f32::from(handle.bounds().size.height);
        let shown_pull = state.read(cx).refresh.shown_pull(viewport);
        let top_margin = self.content_top_margin.map_or(0.0, |source| source(cx));
        let bottom_margin = scroll_content_bottom_margin(cx);

        let scroller = div().id("scroll_view_content").size_full().track_scroll(&handle);
        let scroller = if disabled { scroller.overflow_y_hidden() } else { scroller.overflow_y_scroll() };
        let scroller = if self.restricts_to_axis { scroller.restrict_scroll_to_axis() } else { scroller };
        let scroller = scroller.child(
            div().flex().flex_col().w_full().pt(px(top_margin + shown_pull)).pb(px(bottom_margin)).children(self.children),
        );

        let header = (refreshing || shown_pull > 0.0).then(|| refresh_header(shown_pull, refreshing).top(px(top_margin)));

        let thumb = (self.shows_indicators && !disabled && indicator_visible)
            .then(|| {
                indicator_thumb(viewport, f32::from(handle.max_offset().y), -f32::from(handle.offset().y))
            })
            .flatten()
            .map(|(top, length)| {
                div()
                    .absolute()
                    .top(px(top))
                    .right(px(INDICATOR_INSET))
                    .w(px(INDICATOR_WIDTH))
                    .h(px(length))
                    .rounded(px(INDICATOR_WIDTH / 2.0))
                    .bg(indicator_fill(crate::theme(cx)))
            });

        let wheel_state: Entity<ScrollViewState> = state;
        let wheel_handle = handle;
        let wheel_guide_host = guide_host.clone();
        let refresh = self.refresh;
        let shows_indicators = self.shows_indicators;
        self.base
            .on_scroll_wheel(move |event, window, cx| {
                if disabled {
                    return;
                }
                wheel_guide_host.note_wheel(event);
                let delta = f32::from(event.delta.pixel_delta(window.line_height()).y);
                let offset = f32::from(wheel_handle.offset().y);
                let overscroll = if offset >= -TOP_TOLERANCE { offset.max(0.0) } else { 0.0 };
                let dimension = f32::from(wheel_handle.bounds().size.height);
                let has_thumb = shows_indicators && f32::from(wheel_handle.max_offset().y) > 0.0;
                let step = wheel_state.update(cx, |state, cx| {
                    state.scrolled(has_thumb, cx);
                    if refresh.is_none() {
                        return PullStep::NONE;
                    }
                    let pull_before = state.refresh.shown_pull(dimension);
                    let step = state.refresh.track_pull(event, delta, overscroll, dimension);
                    state.refresh.arm_release(|state: &mut ScrollViewState| &mut state.refresh, cx);
                    if state.refresh.shown_pull(dimension) != pull_before {
                        cx.notify();
                    }
                    step
                });
                if step.absorbed > 0.0 {
                    let moved = wheel_handle.offset();
                    wheel_handle.set_offset(point(moved.x, px((f32::from(moved.y) + step.absorbed).min(0.0))));
                }
                if let (true, Some(action)) = (step.starts, refresh.as_ref()) {
                    let work = action(window, cx);
                    wheel_state.update(cx, |state, cx| {
                        state.refresh.begin_refresh(work, |state: &mut ScrollViewState| &mut state.refresh, cx);
                    });
                }
            })
            .children(header)
            .child(GuideScrollHostScope::new(guide_host, scroller))
            .children(thumb)
    }
}

const LIST_SCROLLBAR_WIDTH: f32 = 12.0;

const LIST_SCROLLBAR_THUMB_WIDTH: f32 = 6.0;

const LIST_SCROLLBAR_ACTIVE_OPACITY: f32 = 0.6;

fn list_scrollbar_thumb(track: f32, viewport: f32, max_offset: f32, scrolled: f32) -> Option<(f32, f32)> {
    let usable = track - 2.0 * INDICATOR_INSET;
    if usable <= 0.0 || viewport <= 0.0 || max_offset < 1.0 {
        return None;
    }
    let length = (usable * viewport / (viewport + max_offset)).clamp(INDICATOR_MIN_LENGTH.min(usable), usable);
    let progress = (scrolled / max_offset).clamp(0.0, 1.0);
    Some((INDICATOR_INSET + (usable - length) * progress, length))
}

fn list_scrollbar_offset(track: f32, length: f32, max_offset: f32, thumb_top: f32) -> f32 {
    let travel = track - 2.0 * INDICATOR_INSET - length;
    if travel <= 0.0 {
        return 0.0;
    }
    ((thumb_top - INDICATOR_INSET) / travel).clamp(0.0, 1.0) * max_offset
}

#[derive(IntoElement)]
pub struct ListScrollbar {
    id: ElementId,
    state: ListState,
    top_inset: f32,
}

impl ListScrollbar {
    pub fn new(id: impl Into<ElementId>, state: ListState) -> Self {
        Self { id: id.into(), state, top_inset: 0.0 }
    }

    pub fn top_inset(mut self, top_inset: f32) -> Self {
        self.top_inset = top_inset;
        self
    }
}

impl RenderOnce for ListScrollbar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let grab = window.use_keyed_state(self.id, cx, |_, _| None::<f32>);
        let primary = crate::theme(cx).primary._100;
        let state = self.state;
        let track = canvas(
            {
                let state = state.clone();
                move |track, window, _| {
                    let viewport = f32::from(state.viewport_bounds().size.height);
                    let max_offset = f32::from(state.max_offset_for_scrollbar().y);
                    let scrolled = -f32::from(state.scroll_px_offset_for_scrollbar().y);
                    let thumb = list_scrollbar_thumb(f32::from(track.size.height), viewport, max_offset, scrolled)?;
                    Some((thumb, max_offset, window.insert_hitbox(track, HitboxBehavior::BlockMouseExceptScroll)))
                }
            },
            move |track, prepainted, window, cx| {
                let Some(((top, length), max_offset, hitbox)) = prepainted else {
                    return;
                };
                let height = f32::from(track.size.height);
                let thumb = Bounds::new(
                    point(track.right() - px(INDICATOR_INSET + LIST_SCROLLBAR_THUMB_WIDTH), track.top() + px(top)),
                    size(px(LIST_SCROLLBAR_THUMB_WIDTH), px(length)),
                );
                let hovered = hitbox.is_hovered(window);
                let active = grab.read(cx).is_some() || hovered;
                let opacity = if active { LIST_SCROLLBAR_ACTIVE_OPACITY } else { INDICATOR_OPACITY };
                window.paint_quad(fill(thumb, primary.opacity(opacity)).corner_radii(px(LIST_SCROLLBAR_THUMB_WIDTH / 2.0)));
                let view = window.current_view();
                let scroll_to = {
                    let state = state.clone();
                    move |thumb_top: Pixels| {
                        let offset = list_scrollbar_offset(height, length, max_offset, f32::from(thumb_top - track.top()));
                        state.set_offset_from_scrollbar(point(px(0.0), px(-offset)));
                    }
                };
                {
                    let grab = grab.clone();
                    let state = state.clone();
                    let scroll_to = scroll_to.clone();
                    let hitbox = hitbox.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left || !hitbox.is_hovered(window)
                        {
                            return;
                        }
                        cx.stop_propagation();
                        let offset = if thumb.contains(&event.position) {
                            f32::from(event.position.y - thumb.top())
                        } else {
                            let offset = length / 2.0;
                            scroll_to(event.position.y - px(offset));
                            offset
                        };
                        state.scrollbar_drag_started();
                        grab.update(cx, |grab, _| *grab = Some(offset));
                        cx.notify(view);
                    });
                }
                {
                    let grab = grab.clone();
                    let state = state.clone();
                    let hitbox = hitbox.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture {
                            return;
                        }
                        let Some(offset) = *grab.read(cx) else {
                            if hitbox.is_hovered(window) != hovered {
                                cx.notify(view);
                            }
                            return;
                        };
                        if event.pressed_button != Some(MouseButton::Left) {
                            grab.update(cx, |grab, _| *grab = None);
                            state.scrollbar_drag_ended();
                            cx.notify(view);
                            return;
                        }
                        scroll_to(event.position.y - px(offset));
                        cx.notify(view);
                    });
                }
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || grab.read(cx).is_none() {
                        return;
                    }
                    grab.update(cx, |grab, _| *grab = None);
                    state.scrollbar_drag_ended();
                    cx.notify(view);
                });
            },
        )
        .size_full();
        div().absolute().top(px(self.top_inset)).bottom_0().right_0().w(px(LIST_SCROLLBAR_WIDTH)).child(track)
    }
}
