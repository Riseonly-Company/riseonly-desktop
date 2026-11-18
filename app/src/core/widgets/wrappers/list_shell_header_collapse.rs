use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui::{
    AnyElement, AnyWeakEntity, App, Bounds, Context, DispatchPhase, ElementId, Entity, HitboxBehavior,
    IntoElement, ListState, ParentElement, Pixels, Point, RenderOnce, ScrollHandle, ScrollWheelEvent, Styled,
    TouchPhase, WeakEntity, Window, canvas, div, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out};
use rise_ui::swiftui::shapes::pass_through;
use rise_widgets::wrappers::list_shell_header_collapse_hooks::ListShellHeaderCollapseHooks;

use crate::core::widgets::wrappers::list_shell_ui::list_shell_floating_top_inset;

/// geometry of a chrome that folds under the title bar as the list scrolls
pub struct ListShellHeaderCollapsePolicy;

impl ListShellHeaderCollapsePolicy {
    pub const EPSILON: f32 = 0.5;

    pub const DECELERATION_TRAVEL_FACTOR: f32 = 0.5;

    pub const REST_CHECK_DELAY: Duration = Duration::from_millis(120);

    pub fn clamped(delta: f32, range: f32) -> f32 {
        delta.clamp(0.0, range.max(0.0))
    }

    /// both ends land exactly, anything between publishes only past EPSILON
    pub fn should_publish(current: f32, next: f32, range: f32) -> bool {
        if next == current {
            return false;
        }
        if next == 0.0 || next == range {
            return true;
        }
        (next - current).abs() > Self::EPSILON
    }

    /// content offset delta that brings a released list to a resting chrome
    /// none if it would stop on one by itself, velocity_y positive is finger moving down
    pub fn settle_content_offset_delta(delta: f32, velocity_y: f32, range: f32) -> Option<f32> {
        if range <= 0.0 || delta < 0.0 {
            return None;
        }
        let projected = delta - velocity_y * Self::DECELERATION_TRAVEL_FACTOR;
        if projected <= 0.0 || projected >= range {
            return None;
        }
        Some(Self::resting_offset(projected, range) - delta)
    }

    /// chrome rests fully open or fully folded, never in between
    pub fn resting_offset(delta: f32, range: f32) -> f32 {
        let clamped_delta = Self::clamped(delta, range);
        if clamped_delta < range / 2.0 { 0.0 } else { range.max(0.0) }
    }

    /// for a list resting half folded with no release to settle it
    /// like programmatic scroll or stopped momentum
    pub fn rest_settle_content_offset_delta(delta: f32, range: f32) -> Option<f32> {
        if range <= 0.0 || delta <= Self::EPSILON || delta >= range - Self::EPSILON {
            return None;
        }
        Some(Self::resting_offset(delta, range) - delta)
    }

    pub fn content_lift(held_offset: f32, delta: f32, range: f32) -> f32 {
        (held_offset - Self::clamped(delta, range)).max(0.0)
    }
}

/// how far the folding part of a screen chrome is hidden under the title bar
/// one entity per screen, paged screens keep a value per page and show the active one
pub struct ListShellHeaderCollapse {
    offset: f32,
    content_lift: f32,
    is_held: bool,
    range: f32,
    active_page_id: Option<String>,
    offsets: HashMap<String, f32>,
    pages: HashMap<String, WeakEntity<ListShellHeaderCollapseProbeCoordinator>>,
    offset_motion: Animated<f32>,
    content_lift_motion: Animated<f32>,
}

impl Default for ListShellHeaderCollapse {
    fn default() -> Self {
        Self::new()
    }
}

impl ListShellHeaderCollapse {
    pub fn new() -> Self {
        Self {
            offset: 0.0,
            content_lift: 0.0,
            is_held: false,
            range: 0.0,
            active_page_id: None,
            offsets: HashMap::new(),
            pages: HashMap::new(),
            offset_motion: Animated::new(0.0),
            content_lift_motion: Animated::new(0.0),
        }
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    pub fn content_lift(&self) -> f32 {
        self.content_lift
    }

    pub fn is_held(&self) -> bool {
        self.is_held
    }

    pub fn range(&self) -> f32 {
        self.range
    }

    pub fn active_page_id(&self) -> Option<&str> {
        self.active_page_id.as_deref()
    }

    /// offset as it stands on screen this frame, animated writes included
    pub fn presented_offset(&self, window: &Window) -> f32 {
        self.offset_motion.frame_value(RiseClock::monotonic(), window)
    }

    pub fn presented_content_lift(&self, window: &Window) -> f32 {
        self.content_lift_motion.frame_value(RiseClock::monotonic(), window)
    }

    pub fn register(&mut self, coordinator: WeakEntity<ListShellHeaderCollapseProbeCoordinator>, page_id: &str) {
        self.pages.insert(page_id.to_string(), coordinator);
    }

    pub fn unregister(&mut self, coordinator: &WeakEntity<ListShellHeaderCollapseProbeCoordinator>, page_id: &str) {
        if self
            .pages
            .get(page_id)
            .is_some_and(|page| page.entity_id() == coordinator.entity_id())
        {
            self.pages.remove(page_id);
        }
    }

    pub fn set_range(&mut self, range: f32, cx: &mut Context<Self>) {
        let next = range.max(0.0);
        if next == self.range {
            return;
        }
        self.range = next;
        for value in self.offsets.values_mut() {
            *value = ListShellHeaderCollapsePolicy::clamped(*value, next);
        }
        let clamped_offset = ListShellHeaderCollapsePolicy::clamped(self.offset, next);
        if clamped_offset != self.offset {
            self.write_offset(clamped_offset, None, cx);
        }
    }

    /// makes page_id drive the chrome
    /// a page resting inside the fold range follows the chrome, one scrolled past it moves the chrome
    pub fn activate(&mut self, page_id: &str, animation: Option<MotionSpec>, cx: &mut Context<Self>) {
        if self.active_page_id.as_deref() == Some(page_id) {
            return;
        }
        self.active_page_id = Some(page_id.to_string());
        if self.is_held {
            return;
        }
        let mut target = self.offsets.get(page_id).copied().unwrap_or(0.0);
        if target == self.offset {
            return;
        }
        let Some(page) = self.pages.get(page_id).and_then(WeakEntity::upgrade) else {
            return;
        };
        let (offset, range) = (self.offset, self.range);
        if let Some(aligned) = page.update(cx, |page, cx| page.align(offset, range, cx)) {
            self.offsets.insert(page_id.to_string(), aligned);
            target = aligned;
        }
        if target == self.offset {
            return;
        }
        self.write_offset(target, animation, cx);
        self.align_inactive_pages(cx);
    }

    /// called on every scroll frame, publishes only when the active page moved visibly
    pub fn publish(&mut self, delta: f32, page_id: &str, cx: &mut Context<Self>) {
        let next = ListShellHeaderCollapsePolicy::clamped(delta, self.range);
        self.offsets.insert(page_id.to_string(), next);
        if self.active_page_id.as_deref() != Some(page_id) {
            return;
        }
        if self.is_held {
            let lift = ListShellHeaderCollapsePolicy::content_lift(self.offset, next, self.range);
            if ListShellHeaderCollapsePolicy::should_publish(self.content_lift, lift, self.offset) {
                self.write_content_lift(lift, None, cx);
            }
            return;
        }
        if !ListShellHeaderCollapsePolicy::should_publish(self.offset, next, self.range) {
            return;
        }
        self.write_offset(next, None, cx);
        if next == 0.0 || next == self.range {
            self.align_inactive_pages(cx);
        }
    }

    /// chrome came to an end so pages beside the active one that rest inside the fold range move to it now
    /// off screen, so the next swipe finds them aligned
    fn align_inactive_pages(&mut self, cx: &mut Context<Self>) {
        let (offset, range) = (self.offset, self.range);
        let pages: Vec<(String, WeakEntity<ListShellHeaderCollapseProbeCoordinator>)> = self
            .pages
            .iter()
            .filter(|(page_id, _)| self.active_page_id.as_deref() != Some(page_id.as_str()))
            .map(|(page_id, page)| (page_id.clone(), page.clone()))
            .collect();
        for (page_id, page) in pages {
            let Some(coordinator) = page.upgrade() else {
                self.pages.remove(&page_id);
                continue;
            };
            if let Some(aligned) = coordinator.update(cx, |coordinator, cx| coordinator.align(offset, range, cx)) {
                self.offsets.insert(page_id, aligned);
            }
        }
    }

    pub fn hold(&mut self) {
        self.is_held = true;
    }

    /// hands the chrome back to the list and settles it where the active page stands
    pub fn release(&mut self, animation: Option<MotionSpec>, cx: &mut Context<Self>) {
        if !self.is_held {
            return;
        }
        self.is_held = false;
        let active = self.active_page_id.clone().unwrap_or_default();
        let target = ListShellHeaderCollapsePolicy::resting_offset(
            self.offsets.get(&active).copied().unwrap_or(0.0),
            self.range,
        );
        if target == self.offset && self.content_lift == 0.0 {
            return;
        }
        self.write_offset(target, animation, cx);
        self.write_content_lift(0.0, animation, cx);
        self.align_inactive_pages(cx);
    }

    pub fn reveal_folded(collapse: &Entity<Self>, cx: &mut App) -> bool {
        let page = {
            let this = collapse.read(cx);
            if this.is_held || this.offset < ListShellHeaderCollapsePolicy::EPSILON {
                return false;
            }
            this.active_page_id
                .as_ref()
                .and_then(|page_id| this.pages.get(page_id))
                .and_then(WeakEntity::upgrade)
        };
        let Some(page) = page else {
            return false;
        };
        page.update(cx, |page, cx| page.scroll_to_rest(cx))
    }

    pub fn retain(&mut self, page_ids: &HashSet<String>) {
        self.offsets.retain(|page_id, _| page_ids.contains(page_id));
    }

    fn write_offset(&mut self, value: f32, animation: Option<MotionSpec>, cx: &mut Context<Self>) {
        self.offset = value;
        self.offset_motion.set_in(value, animation, RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn write_content_lift(&mut self, value: f32, animation: Option<MotionSpec>, cx: &mut Context<Self>) {
        self.content_lift = value;
        self.content_lift_motion.set_in(value, animation, RiseClock::monotonic(), cx);
        cx.notify();
    }
}

/// slides a chrome layer up by the current collapse
/// reading it in the wrapper keeps the wrapped view from rebuilding on scroll
/// hides_when_folded stops taking clicks once folding starts
#[derive(IntoElement)]
pub struct ListShellHeaderCollapseSlide {
    collapse: Entity<ListShellHeaderCollapse>,
    hides_when_folded: bool,
    content: AnyElement,
}

impl ListShellHeaderCollapseSlide {
    pub fn new(collapse: Entity<ListShellHeaderCollapse>, content: impl IntoElement) -> Self {
        Self {
            collapse,
            hides_when_folded: false,
            content: content.into_any_element(),
        }
    }

    pub fn hides_when_folded(mut self, hides_when_folded: bool) -> Self {
        self.hides_when_folded = hides_when_folded;
        self
    }
}

impl RenderOnce for ListShellHeaderCollapseSlide {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let offset = self.collapse.read(cx).presented_offset(window);
        let moved = div().relative().top(px(-offset));
        if !self.hides_when_folded || offset < 1.0 {
            return moved.child(self.content);
        }
        moved.child(pass_through().child(self.content))
    }
}

/// lifts the list under a held chrome, same contract as the slide
#[derive(IntoElement)]
pub struct ListShellHeaderCollapseContentLift {
    collapse: Entity<ListShellHeaderCollapse>,
    content: AnyElement,
}

impl ListShellHeaderCollapseContentLift {
    pub fn new(collapse: Entity<ListShellHeaderCollapse>, content: impl IntoElement) -> Self {
        Self {
            collapse,
            content: content.into_any_element(),
        }
    }
}

impl RenderOnce for ListShellHeaderCollapseContentLift {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let lift = self.collapse.read(cx).presented_content_lift(window);
        div().relative().top(px(-lift)).size_full().child(self.content)
    }
}

#[derive(IntoElement)]
pub struct ListShellBelowCollapsedChrome {
    collapse: Entity<ListShellHeaderCollapse>,
    content: AnyElement,
}

impl ListShellBelowCollapsedChrome {
    pub fn new(collapse: Entity<ListShellHeaderCollapse>, content: impl IntoElement) -> Self {
        Self {
            collapse,
            content: content.into_any_element(),
        }
    }
}

impl RenderOnce for ListShellBelowCollapsedChrome {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let inset = list_shell_floating_top_inset(cx);
        let offset = self.collapse.read(cx).presented_offset(window);
        div().size_full().pt(px((inset - offset).max(0.0))).child(self.content)
    }
}

#[derive(IntoElement)]
pub struct ListShellClipBelowCollapsedChrome {
    collapse: Entity<ListShellHeaderCollapse>,
    active: bool,
    content: AnyElement,
}

impl ListShellClipBelowCollapsedChrome {
    pub fn new(collapse: Entity<ListShellHeaderCollapse>, active: bool, content: impl IntoElement) -> Self {
        Self {
            collapse,
            active,
            content: content.into_any_element(),
        }
    }
}

impl RenderOnce for ListShellClipBelowCollapsedChrome {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !self.active {
            return div().size_full().child(self.content);
        }
        let inset = list_shell_floating_top_inset(cx);
        let clip = (inset - self.collapse.read(cx).presented_offset(window)).max(0.0);
        div().relative().size_full().child(
            div()
                .absolute()
                .top(px(clip))
                .bottom_0()
                .left_0()
                .right_0()
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .top(px(-clip))
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .child(self.content),
                ),
        )
    }
}

pub trait ListShellHeaderCollapseModifiers: IntoElement + Sized {
    fn list_shell_header_collapse_slide(
        self,
        collapse: &Entity<ListShellHeaderCollapse>,
        hides_when_folded: bool,
    ) -> ListShellHeaderCollapseSlide {
        ListShellHeaderCollapseSlide::new(collapse.clone(), self).hides_when_folded(hides_when_folded)
    }
}

impl<E: IntoElement> ListShellHeaderCollapseModifiers for E {}

fn settle_scroll_animation() -> MotionSpec {
    ease_in_out(0.3)
}

/// scroll offset of a gpui scroller counted from the inset edge, positive while scrolled down
/// the top margin is padding inside the scroller so the inset edge is the content start
fn scroll_y(scroll_view: &ListShellCollapseScroller) -> f32 {
    -f32::from(scroll_view.offset().y)
}

/// the scroller a probe follows, a plain scroll view or a virtualized list measured over its
/// laid out rows
#[derive(Clone)]
pub enum ListShellCollapseScroller {
    View(ScrollHandle),
    List(ListState),
}

impl From<ScrollHandle> for ListShellCollapseScroller {
    fn from(scroll_view: ScrollHandle) -> Self {
        Self::View(scroll_view)
    }
}

impl From<ListState> for ListShellCollapseScroller {
    fn from(list: ListState) -> Self {
        Self::List(list)
    }
}

impl ListShellCollapseScroller {
    /// content offset, y is negative as the content scrolls up
    pub fn offset(&self) -> Point<Pixels> {
        match self {
            Self::View(scroll_view) => scroll_view.offset(),
            Self::List(list) => list.scroll_px_offset_for_scrollbar(),
        }
    }

    pub fn max_offset(&self) -> Point<Pixels> {
        match self {
            Self::View(scroll_view) => scroll_view.max_offset(),
            Self::List(list) => list.max_offset_for_scrollbar(),
        }
    }

    pub fn set_offset(&self, offset: Point<Pixels>) {
        match self {
            Self::View(scroll_view) => scroll_view.set_offset(offset),
            Self::List(list) => list.set_offset_from_scrollbar(offset),
        }
    }

    pub fn bounds(&self) -> Bounds<Pixels> {
        match self {
            Self::View(scroll_view) => scroll_view.bounds(),
            Self::List(list) => list.viewport_bounds(),
        }
    }
}

/// reports one page scroll to the header collapse
/// probe reads the offset each paint, wheel in capture phase
/// rest_offset is the fully open offset
pub struct ListShellHeaderCollapseProbeCoordinator {
    collapse: Entity<ListShellHeaderCollapse>,
    page_id: String,
    rest_offset: f32,
    scroll_view: Option<ListShellCollapseScroller>,
    scroll_view_id: Option<ElementId>,
    manual_list: Option<AnyWeakEntity>,
    observed_offset_y: Option<Pixels>,
    rest_check_generation: u64,
    pan_generation: u64,
    is_tracking: bool,
    last_momentum_at: Option<Duration>,
    settle_motion: Option<Animated<f32>>,
}

impl ListShellHeaderCollapseProbeCoordinator {
    pub fn new(collapse: Entity<ListShellHeaderCollapse>, page_id: impl Into<String>, rest_offset: f32) -> Self {
        Self {
            collapse,
            page_id: page_id.into(),
            rest_offset,
            scroll_view: None,
            scroll_view_id: None,
            manual_list: None,
            observed_offset_y: None,
            rest_check_generation: 0,
            pan_generation: 0,
            is_tracking: false,
            last_momentum_at: None,
            settle_motion: None,
        }
    }

    pub fn update(&mut self, page_id: impl Into<String>, rest_offset: f32, cx: &mut Context<Self>) {
        let page_id = page_id.into();
        let changed = page_id != self.page_id || rest_offset != self.rest_offset;
        if page_id != self.page_id && self.scroll_view.is_some() {
            let coordinator = cx.weak_entity();
            let previous = self.page_id.clone();
            self.collapse.update(cx, |collapse, _| {
                collapse.unregister(&coordinator, &previous);
                collapse.register(coordinator.clone(), &page_id);
            });
        }
        self.page_id = page_id;
        self.rest_offset = rest_offset;
        if changed {
            self.pan_generation = self.pan_generation.wrapping_add(1);
            self.rest_check_generation = self.rest_check_generation.wrapping_add(1);
            self.schedule_report(cx);
        }
    }

    pub fn attach(
        &mut self,
        scroll_view_id: impl Into<ElementId>,
        scroll_view: impl Into<ListShellCollapseScroller>,
        cx: &mut Context<Self>,
    ) {
        let scroll_view_id = scroll_view_id.into();
        let scroll_view = scroll_view.into();
        if self.scroll_view.is_some() && self.scroll_view_id.as_ref() == Some(&scroll_view_id) {
            return;
        }
        self.detach(cx);
        self.observed_offset_y = Some(scroll_view.offset().y);
        self.scroll_view = Some(scroll_view);
        self.scroll_view_id = Some(scroll_view_id);
        let coordinator = cx.weak_entity();
        let page_id = self.page_id.clone();
        self.collapse
            .update(cx, |collapse, _| collapse.register(coordinator, &page_id));
        self.schedule_report(cx);
    }

    pub fn attach_list(&mut self, list: AnyWeakEntity, cx: &mut Context<Self>) {
        let Some(hooks) = ListShellHeaderCollapseHooks::installed() else {
            return;
        };
        let Some(scroll_view) = (hooks.integration_scroll_view)(&list, cx) else {
            return;
        };
        self.attach(ElementId::View(list.entity_id()), scroll_view, cx);
        self.manual_list = Some(list);
    }

    pub fn content_top_did_change(&mut self, cx: &mut Context<Self>) {
        self.schedule_report(cx);
    }

    /// attach and update run inside a render pass where the chrome mustnt be written
    /// so the report lands one main thread turn later
    fn schedule_report(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |coordinator, cx| {
                if let Some(scroll_view) = coordinator.scroll_view.clone() {
                    coordinator.report(&scroll_view, cx);
                }
            });
        })
        .detach();
    }

    pub fn detach(&mut self, cx: &mut Context<Self>) {
        let coordinator = cx.weak_entity();
        let page_id = self.page_id.clone();
        self.collapse
            .update(cx, |collapse, _| collapse.unregister(&coordinator, &page_id));
        self.observed_offset_y = None;
        self.rest_check_generation = self.rest_check_generation.wrapping_add(1);
        self.pan_generation = self.pan_generation.wrapping_add(1);
        self.is_tracking = false;
        self.last_momentum_at = None;
        self.settle_motion = None;
        self.scroll_view = None;
        self.scroll_view_id = None;
        self.manual_list = None;
    }

    fn delta(&self, scroll_view: &ListShellCollapseScroller, cx: &App) -> f32 {
        if let Some(manual_list) = &self.manual_list
            && let Some(hooks) = ListShellHeaderCollapseHooks::installed()
            && let Some(content_top_offset_y) = (hooks.content_top_offset_y)(manual_list, cx)
        {
            if (hooks.item_count)(manual_list, cx).unwrap_or(0) == 0 {
                return 0.0;
            }
            return scroll_y(scroll_view) - content_top_offset_y - self.rest_offset;
        }
        scroll_y(scroll_view) - self.rest_offset
    }

    pub fn scroll_to_rest(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(scroll_view) = self.scroll_view.clone() else {
            return false;
        };
        if self.is_tracking {
            return false;
        }
        let current = self.delta(&scroll_view, cx);
        if current <= ListShellHeaderCollapsePolicy::EPSILON {
            return false;
        }
        self.settle_motion = None;
        self.rest_check_generation = self.rest_check_generation.wrapping_add(1);
        let target_y = scroll_y(&scroll_view) - current;
        if let Some(manual_list) = self.manual_list.clone()
            && let Some(hooks) = ListShellHeaderCollapseHooks::installed()
        {
            (hooks.set_content_offset_y)(&manual_list, target_y, cx);
        } else {
            let maximum_y = f32::from(scroll_view.max_offset().y).max(0.0);
            scroll_view.set_offset(point(scroll_view.offset().x, px(-target_y.clamp(0.0, maximum_y))));
        }
        self.report(&scroll_view, cx);
        cx.refresh_windows();
        true
    }

    /// moves a list resting inside the fold range to the chrome target, no animation returns the
    /// fold it reached
    /// none if scrolled past the fold, pulled open under a finger or still moving
    pub fn align(&mut self, target: f32, range: f32, cx: &mut Context<Self>) -> Option<f32> {
        let scroll_view = self.scroll_view.clone()?;
        if self.is_tracking || self.is_decelerating() || self.is_settling() {
            return None;
        }
        let hooks = ListShellHeaderCollapseHooks::installed();
        if let Some(manual_list) = &self.manual_list
            && let Some(hooks) = hooks
            && (hooks.item_count)(manual_list, cx).unwrap_or(0) == 0
        {
            return None;
        }
        let epsilon = ListShellHeaderCollapsePolicy::EPSILON;
        let current = self.delta(&scroll_view, cx);
        if range <= 0.0 || current <= -epsilon || current >= range + epsilon {
            return None;
        }
        let shift = ListShellHeaderCollapsePolicy::clamped(target, range) - current;
        if shift.abs() > epsilon {
            if let Some(manual_list) = self.manual_list.clone()
                && let Some(hooks) = hooks
            {
                (hooks.set_content_offset_y)(&manual_list, scroll_y(&scroll_view) + shift, cx);
            } else {
                let minimum_y = 0.0;
                let maximum_y = f32::from(scroll_view.max_offset().y).max(minimum_y);
                let target_y = (scroll_y(&scroll_view) + shift).clamp(minimum_y, maximum_y);
                scroll_view.set_offset(point(scroll_view.offset().x, px(-target_y)));
            }
        }
        Some(ListShellHeaderCollapsePolicy::clamped(self.delta(&scroll_view, cx), range))
    }

    fn report(&mut self, scroll_view: &ListShellCollapseScroller, cx: &mut Context<Self>) {
        let delta = self.delta(scroll_view, cx);
        let page_id = self.page_id.clone();
        self.collapse.update(cx, |collapse, cx| collapse.publish(delta, &page_id, cx));
        self.schedule_rest_check(scroll_view, cx);
    }

    fn drives_chrome(&self, cx: &App) -> bool {
        let collapse = self.collapse.read(cx);
        !collapse.is_held() && collapse.active_page_id() == Some(self.page_id.as_str())
    }

    fn schedule_rest_check(&mut self, scroll_view: &ListShellCollapseScroller, cx: &mut Context<Self>) {
        self.rest_check_generation = self.rest_check_generation.wrapping_add(1);
        if !self.drives_chrome(cx) {
            return;
        }
        let range = self.collapse.read(cx).range();
        if ListShellHeaderCollapsePolicy::rest_settle_content_offset_delta(self.delta(scroll_view, cx), range)
            .is_none()
        {
            return;
        }
        let generation = self.rest_check_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(ListShellHeaderCollapsePolicy::REST_CHECK_DELAY)
                .await;
            let _ = this.update(cx, |coordinator, cx| {
                if coordinator.rest_check_generation == generation {
                    coordinator.settle_if_resting_half_folded(cx);
                }
            });
        })
        .detach();
    }

    /// momentum and notched wheel come as wheel events outside a trackpad touch
    /// while they keep coming the list is still moving
    fn is_decelerating(&self) -> bool {
        self.last_momentum_at.is_some_and(|at| {
            RiseClock::monotonic().saturating_sub(at) < ListShellHeaderCollapsePolicy::REST_CHECK_DELAY
        })
    }

    fn is_settling(&self) -> bool {
        self.settle_motion
            .as_ref()
            .is_some_and(|motion| motion.is_running(RiseClock::monotonic()))
    }

    fn settle_if_resting_half_folded(&mut self, cx: &mut Context<Self>) {
        let Some(scroll_view) = self.scroll_view.clone() else {
            return;
        };
        if !self.drives_chrome(cx) {
            return;
        }
        if self.is_tracking || self.is_decelerating() || self.is_settling() {
            self.schedule_rest_check(&scroll_view, cx);
            return;
        }
        let range = self.collapse.read(cx).range();
        let Some(delta) =
            ListShellHeaderCollapsePolicy::rest_settle_content_offset_delta(self.delta(&scroll_view, cx), range)
        else {
            return;
        };
        self.settle(&scroll_view, delta, cx);
    }

    pub fn handle_pan(&mut self, phase: TouchPhase, cx: &mut Context<Self>) {
        let Some(scroll_view) = self.scroll_view.clone() else {
            return;
        };
        match phase {
            TouchPhase::Started => {
                self.is_tracking = true;
                self.settle_motion = None;
                self.pan_generation = self.pan_generation.wrapping_add(1);
                self.rest_check_generation = self.rest_check_generation.wrapping_add(1);
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.is_tracking = false;
                let velocity_y = 0.0;
                let released_delta = self.delta(&scroll_view, cx);
                let range = self.collapse.read(cx).range();
                let correction =
                    ListShellHeaderCollapsePolicy::settle_content_offset_delta(released_delta, velocity_y, range);
                self.pan_generation = self.pan_generation.wrapping_add(1);
                let generation = self.pan_generation;
                let scroll_view_id = self.scroll_view_id.clone();
                cx.spawn(async move |this, cx| {
                    let _ = this.update(cx, |coordinator, cx| {
                        let Some(scroll_view) = coordinator.scroll_view.clone() else {
                            return;
                        };
                        if coordinator.scroll_view_id != scroll_view_id
                            || coordinator.pan_generation != generation
                            || !coordinator.drives_chrome(cx)
                            || coordinator.is_tracking
                        {
                            return;
                        }
                        if let Some(correction) = correction {
                            let delta = released_delta + correction - coordinator.delta(&scroll_view, cx);
                            coordinator.settle(&scroll_view, delta, cx);
                        }
                        coordinator.report(&scroll_view, cx);
                    });
                })
                .detach();
            }
            TouchPhase::Moved => {
                if self.is_tracking {
                    return;
                }
                self.last_momentum_at = Some(RiseClock::monotonic());
                self.settle_motion = None;
                self.pan_generation = self.pan_generation.wrapping_add(1);
            }
        }
    }

    fn settle(&mut self, scroll_view: &ListShellCollapseScroller, delta: f32, cx: &mut Context<Self>) {
        if delta.abs() <= ListShellHeaderCollapsePolicy::EPSILON {
            return;
        }
        if let Some(manual_list) = self.manual_list.clone()
            && let Some(hooks) = ListShellHeaderCollapseHooks::installed()
        {
            if let Some(content_top_offset_y) = (hooks.content_top_offset_y)(&manual_list, cx) {
                (hooks.settle_content_offset_from_top)(
                    &manual_list,
                    scroll_y(scroll_view) - content_top_offset_y + delta,
                    cx,
                );
            }
            return;
        }
        let minimum_y = 0.0;
        let maximum_y = f32::from(scroll_view.max_offset().y).max(minimum_y);
        let current_y = scroll_y(scroll_view);
        let target_y = (current_y + delta).clamp(minimum_y, maximum_y);
        if (current_y - target_y).abs() <= ListShellHeaderCollapsePolicy::EPSILON {
            return;
        }
        let mut motion = Animated::new(current_y);
        motion.set_in(target_y, settle_scroll_animation(), RiseClock::monotonic(), cx);
        self.settle_motion = Some(motion);
        cx.notify();
    }

    /// runs in the probe paint, moves a running settle one frame
    /// then reads the offset and reports a change after the frame
    fn frame(&mut self, window: &mut Window) -> bool {
        let Some(scroll_view) = self.scroll_view.clone() else {
            return false;
        };
        let now = RiseClock::monotonic();
        if let Some((y, running)) = self.settle_motion.as_ref().map(|motion| (motion.value(now), motion.is_running(now))) {
            scroll_view.set_offset(point(scroll_view.offset().x, px(-y)));
            if running {
                window.request_animation_frame();
            } else {
                self.settle_motion = None;
            }
        }
        let offset_y = scroll_view.offset().y;
        if self.observed_offset_y == Some(offset_y) {
            return false;
        }
        self.observed_offset_y = Some(offset_y);
        true
    }

    fn report_attached(&mut self, cx: &mut Context<Self>) {
        if let Some(scroll_view) = self.scroll_view.clone() {
            self.report(&scroll_view, cx);
        }
    }
}

/// mount it inside the page scroller behind the content
/// gpui has no view walk so the page passes its scroller handle
#[derive(IntoElement)]
pub struct ListShellHeaderCollapseProbe {
    id: ElementId,
    collapse: Entity<ListShellHeaderCollapse>,
    page_id: String,
    rest_offset: f32,
    scroll_view: Option<ListShellCollapseScroller>,
    manual_list: Option<AnyWeakEntity>,
    coordinator: Option<Entity<ListShellHeaderCollapseProbeCoordinator>>,
}

impl ListShellHeaderCollapseProbe {
    pub fn new(id: impl Into<ElementId>, collapse: Entity<ListShellHeaderCollapse>, page_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            collapse,
            page_id: page_id.into(),
            rest_offset: 0.0,
            scroll_view: None,
            manual_list: None,
            coordinator: None,
        }
    }

    pub fn rest_offset(mut self, rest_offset: f32) -> Self {
        self.rest_offset = rest_offset;
        self
    }

    pub fn scroll_view(mut self, scroll_view: ScrollHandle) -> Self {
        self.scroll_view = Some(scroll_view.into());
        self
    }

    /// follows a virtualized list the way scroll_view follows a scroll view
    pub fn list_state(mut self, list: ListState) -> Self {
        self.scroll_view = Some(list.into());
        self
    }

    /// follows a manual list through attach_list instead of a plain scroll handle
    pub fn manual_list(mut self, manual_list: AnyWeakEntity) -> Self {
        self.manual_list = Some(manual_list);
        self
    }

    /// coordinator the owner keeps, for an owner that reports content_top_did_change to it
    pub fn coordinator(mut self, coordinator: Entity<ListShellHeaderCollapseProbeCoordinator>) -> Self {
        self.coordinator = Some(coordinator);
        self
    }
}

impl RenderOnce for ListShellHeaderCollapseProbe {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let collapse = self.collapse.clone();
        let initial_page_id = self.page_id.clone();
        let rest_offset = self.rest_offset;
        let coordinator = match self.coordinator {
            Some(coordinator) => coordinator,
            None => window.use_keyed_state(self.id.clone(), cx, move |_, _| {
                ListShellHeaderCollapseProbeCoordinator::new(collapse, initial_page_id, rest_offset)
            }),
        };
        let page_id = self.page_id.clone();
        coordinator.update(cx, |coordinator, cx| coordinator.update(page_id, rest_offset, cx));
        if let Some(manual_list) = self.manual_list {
            coordinator.update(cx, |coordinator, cx| coordinator.attach_list(manual_list, cx));
        } else if let Some(scroll_view) = self.scroll_view.clone() {
            let scroll_view_id = self.id.clone();
            coordinator.update(cx, |coordinator, cx| coordinator.attach(scroll_view_id, scroll_view, cx));
        }
        let scroll_view = self.scroll_view;
        canvas(
            move |bounds, window, _cx| {
                let region = scroll_view
                    .as_ref()
                    .map(ListShellCollapseScroller::bounds)
                    .filter(|region| region.size.width > Pixels::ZERO)
                    .unwrap_or(bounds);
                (region, window.insert_hitbox(region, HitboxBehavior::Normal))
            },
            move |_, (region, hitbox), window, cx| {
                let moved = coordinator.update(cx, |coordinator, _| coordinator.frame(window));
                if moved {
                    let coordinator = coordinator.clone();
                    window.defer(cx, move |_, cx| {
                        coordinator.update(cx, |coordinator, cx| coordinator.report_attached(cx));
                    });
                }
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    if !region.contains(&event.position) || !hitbox.should_handle_scroll(window) {
                        return;
                    }
                    let touch_phase = event.touch_phase;
                    coordinator.update(cx, |coordinator, cx| coordinator.handle_pan(touch_phase, cx));
                });
            },
        )
        .absolute()
        .inset_0()
    }
}
