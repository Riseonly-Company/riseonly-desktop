use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, ContentMask, Context, Element, ElementId, Entity, EntityId, GlobalElementId, InspectorElementId,
    InteractiveElement, IntoElement, KeyDownEvent, LayoutId, ParentElement, Pixels, RenderOnce, Styled, Task, Window,
    black, div, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::accessibility::reduce_motion;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::rise_frame_rate::RiseMotionCadence;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::{EnvKey, env, env_scope};
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::motion::{MotionSpec, timing_curve};
use rise_ui::swiftui::transition::{TransitionEffect, TransitionModifiers, UnitPoint};
use rise_widgets::modal_ui::modal_ui::{DismissModal, KEY_CONTEXT, install_key_bindings};

use crate::core::widgets::wrappers::list_shell_header_collapse::ListShellHeaderCollapse;
use crate::core::widgets::wrappers::list_shell_ui::ListShellFloatingTopInsetKey;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::pages::chats::embedded_forum_horizontal_scroll_gate::{
    EmbeddedForumHorizontalPanCallbacks, EmbeddedForumHorizontalPanCallbacksKey,
};
use crate::modules::chat::pages::chats::embedded_forum_panel_hooks::{
    EmbeddedForumPanelForumTopicListArgs, embedded_forum_panel_hooks,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatTopic};
use crate::modules::chat::stores::chats::chat_services::chat_roles_services::ForumTopicListSnapshot;

/// plain callback with no args used by the layer closures
pub type EmbeddedForumAction = Rc<dyn Fn(&mut Window, &mut App)>;
/// callback to open a topic, none means no topic
pub type EmbeddedForumOpenTopicAction = Rc<dyn Fn(Option<ChatTopic>, &mut Window, &mut App)>;

pub struct EmbeddedForumPresentationStore {
    pub requested_chat: Option<ChatInfo>,
}

impl EmbeddedForumPresentationStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { requested_chat: None })
    }

    pub fn present(&mut self, chat: ChatInfo, cx: &mut Context<Self>) {
        self.requested_chat = Some(chat);
        cx.notify();
    }

    pub fn consume(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if self.requested_chat.as_ref().is_none_or(|chat| chat.id != chat_id) {
            return;
        }
        self.requested_chat = None;
        cx.notify();
    }
}

pub enum EmbeddedForumTransitionMetrics {}

impl EmbeddedForumTransitionMetrics {
    pub const AVATAR_RAIL_WIDTH: f32 = 70.0;
    pub const SWIPE_MINIMUM_DISTANCE: f32 = 8.0;
    pub const HEADER_PROGRESS_STEP_COUNT: f32 = 120.0;
    pub const ANIMATION_DURATION: f64 = 0.34;
    pub const PRESENTATION_SETTLE_MARGIN: f64 = 0.12;
    pub const PRESENTATION_SETTLE_DURATION: f64 = Self::ANIMATION_DURATION + Self::PRESENTATION_SETTLE_MARGIN;
    pub const INITIAL_INTERACTION_PROTECTION_DURATION: f64 = 0.44;
    pub const BACKGROUND_WORK_DELAY_DURATION: f64 =
        Self::PRESENTATION_SETTLE_DURATION + Self::INITIAL_INTERACTION_PROTECTION_DURATION;
    pub const BACKGROUND_WORK_RETRY_INTERVAL: f64 = 0.12;
    pub const BACKGROUND_REFRESH_DELAY_DURATION: f64 =
        Self::BACKGROUND_WORK_DELAY_DURATION + Self::BACKGROUND_WORK_RETRY_INTERVAL;

    pub fn animation() -> MotionSpec {
        timing_curve(0.22, 1.0, 0.36, 1.0, Self::ANIMATION_DURATION)
    }

    pub fn travel_distance(container_width: f32) -> f32 {
        (container_width - Self::AVATAR_RAIL_WIDTH).max(1.0)
    }

    pub fn progress(is_presented: bool, drag_offset: f32, container_width: f32) -> f32 {
        if !is_presented {
            return 0.0;
        }
        let distance = Self::travel_distance(container_width);
        (1.0 - drag_offset.max(0.0) / distance).clamp(0.0, 1.0)
    }

    pub fn panel_offset(is_presented: bool, drag_offset: f32, container_width: f32) -> f32 {
        let distance = Self::travel_distance(container_width);
        if !is_presented {
            return distance;
        }
        drag_offset.max(0.0).min(distance)
    }

    pub fn should_close(translation: f32, predicted_translation: f32, container_width: f32) -> bool {
        let distance = Self::travel_distance(container_width);
        translation > distance * 0.24 || predicted_translation > distance * 0.42
    }

    pub fn sampled_header_progress(progress: f32) -> f32 {
        let clamped = progress.clamp(0.0, 1.0);
        (clamped * Self::HEADER_PROGRESS_STEP_COUNT).round() / Self::HEADER_PROGRESS_STEP_COUNT
    }
}

/// when the topic panel is open the chat list folds into an avatar rail on the left
/// avatars shrink into it and row bodies slide left and fade
pub enum EmbeddedForumRailMetrics {}

impl EmbeddedForumRailMetrics {
    pub const RAIL_AVATAR_SIZE: f32 = 50.0;
    pub const CONTENT_TRAVEL_FRACTION: f32 = 0.7;

    pub fn avatar_scale(progress: f32, row_avatar_size: f32) -> f32 {
        if row_avatar_size <= 0.0 {
            return 1.0;
        }
        1.0 - progress * (1.0 - Self::RAIL_AVATAR_SIZE / row_avatar_size)
    }

    /// avatar x shift between rest and rail
    /// at rest it centers on padding plus half size, in the rail on the middle of the rail width
    pub fn avatar_shift(progress: f32, row_avatar_size: f32, horizontal_padding: f32) -> f32 {
        let rest_center = horizontal_padding + row_avatar_size / 2.0;
        let rail_center = EmbeddedForumTransitionMetrics::AVATAR_RAIL_WIDTH / 2.0;
        progress * (rail_center - rest_center)
    }

    pub fn content_clip_extension(row_avatar_size: f32, horizontal_padding: f32) -> f32 {
        let rest_trailing_edge = horizontal_padding + row_avatar_size;
        let rail_trailing_edge = (EmbeddedForumTransitionMetrics::AVATAR_RAIL_WIDTH + Self::RAIL_AVATAR_SIZE) / 2.0;
        (rest_trailing_edge - rail_trailing_edge).max(0.0)
    }

    pub fn content_shift(progress: f32, container_width: f32) -> f32 {
        -progress
            * Self::CONTENT_TRAVEL_FRACTION
            * (container_width - EmbeddedForumTransitionMetrics::AVATAR_RAIL_WIDTH).max(0.0)
    }

    pub fn content_opacity(progress: f32) -> f32 {
        1.0 - progress.clamp(0.0, 1.0)
    }
}

/// one stable ref for the whole list
/// rows read only the driver, reading the transition from a row would redraw every row per frame
#[derive(Clone)]
pub struct EmbeddedForumRailContext {
    pub transition: Entity<EmbeddedForumTransitionState>,
    pub rail_motion: Rc<EmbeddedForumRailMotionDriver>,
    pub container_width: f32,
}

impl EmbeddedForumRailContext {
    pub fn new(transition: Entity<EmbeddedForumTransitionState>, container_width: f32, cx: &App) -> Self {
        let rail_motion = transition.read(cx).rail_motion().clone();
        Self { transition, rail_motion, container_width }
    }

    pub fn progress(&self, cx: &App) -> f32 {
        self.transition.read(cx).progress(self.container_width)
    }
}

impl PartialEq for EmbeddedForumRailContext {
    fn eq(&self, other: &Self) -> bool {
        self.transition == other.transition && self.container_width == other.container_width
    }
}

pub struct EmbeddedForumRailContextKey;

impl EnvKey for EmbeddedForumRailContextKey {
    type Value = Option<EmbeddedForumRailContext>;

    fn default_value() -> Self::Value {
        None
    }
}

/// each inbox row keeps its layout and only the painted surface moves
#[derive(IntoElement)]
pub struct EmbeddedForumRailAvatarMotion {
    row_avatar_size: f32,
    horizontal_padding: f32,
    content: AnyElement,
}

impl EmbeddedForumRailAvatarMotion {
    pub fn new(row_avatar_size: f32, horizontal_padding: f32, content: AnyElement) -> Self {
        Self { row_avatar_size, horizontal_padding, content }
    }
}

impl RenderOnce for EmbeddedForumRailAvatarMotion {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        match env::<EmbeddedForumRailContextKey>(cx) {
            Some(rail) => EmbeddedForumRailSurface {
                rail,
                motion: EmbeddedForumRailSurfaceMotion::Avatar {
                    size: self.row_avatar_size,
                    padding: self.horizontal_padding,
                },
                content: self.content,
            }
            .into_any_element(),
            None => self.content,
        }
    }
}

pub struct EmbeddedForumRailContentMotion {
    pub clip_extension: f32,
}

impl EmbeddedForumRailContentMotion {
    pub fn body(self, content: AnyElement, cx: &mut App) -> AnyElement {
        match env::<EmbeddedForumRailContextKey>(cx) {
            Some(rail) => EmbeddedForumRailSurface {
                rail,
                motion: EmbeddedForumRailSurfaceMotion::Content { clip_extension: self.clip_extension },
                content,
            }
            .into_any_element(),
            None => content,
        }
    }
}

pub struct EmbeddedForumRailFade;

impl EmbeddedForumRailFade {
    pub fn body(self, content: AnyElement, cx: &mut App) -> AnyElement {
        match env::<EmbeddedForumRailContextKey>(cx) {
            Some(rail) => {
                EmbeddedForumRailSurface { rail, motion: EmbeddedForumRailSurfaceMotion::Fade, content }
                    .into_any_element()
            }
            None => content,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EmbeddedForumRailSnapshot {
    pub is_presented: bool,
    pub drag_offset: f32,
}

impl EmbeddedForumRailSnapshot {
    pub fn progress(&self, container_width: f32) -> f32 {
        EmbeddedForumTransitionMetrics::progress(self.is_presented, self.drag_offset, container_width)
    }
}

pub trait EmbeddedForumRailMotionTarget {
    fn apply_rail_motion(&self, snapshot: &EmbeddedForumRailSnapshot);
}

/// weak registrations tied to mounted surfaces, not chat ids
/// so dropped pages are not kept alive by a long lived transition
#[derive(Default)]
pub struct EmbeddedForumRailMotionDriver {
    targets: RefCell<HashMap<usize, Weak<dyn EmbeddedForumRailMotionTarget>>>,
    presence: RefCell<EmbeddedForumRailAnimator>,
    drag_offset: RefCell<EmbeddedForumRailAnimator>,
    snapshot: Cell<EmbeddedForumRailSnapshot>,
    readers: RefCell<HashSet<EntityId>>,
    row_readers: RefCell<HashSet<EntityId>>,
    was_running: Cell<bool>,
    settled_at: Cell<Option<(f32, f32)>>,
    groups: RefCell<HashMap<u64, EmbeddedForumRailGroup>>,
}

/// a row surface painted under a shift key plus the progress it was painted at
struct EmbeddedForumRailGroup {
    view: EntityId,
    motion: EmbeddedForumRailSurfaceMotion,
    bounds: Bounds<Pixels>,
    painted_progress: f32,
    container_width: f32,
}

impl EmbeddedForumRailGroup {
    fn ride(&self, key: u64, progress: f32, window: &mut Window) -> bool {
        let painted = self.painted_progress;
        match self.motion {
            EmbeddedForumRailSurfaceMotion::Avatar { size, padding } => {
                let painted_scale = EmbeddedForumRailMetrics::avatar_scale(painted, size);
                if painted_scale <= f32::EPSILON {
                    return false;
                }
                let scale = EmbeddedForumRailMetrics::avatar_scale(progress, size) / painted_scale;
                let painted_shift = EmbeddedForumRailMetrics::avatar_shift(painted, size, padding);
                let shift = EmbeddedForumRailMetrics::avatar_shift(progress, size, padding);
                let center_x = f32::from(self.bounds.origin.x) + f32::from(self.bounds.size.width) / 2.0;
                let center_y = f32::from(self.bounds.origin.y) + f32::from(self.bounds.size.height) / 2.0;
                let translation = point(
                    px(center_x + shift - scale * (center_x + painted_shift)),
                    px(center_y * (1.0 - scale)),
                );
                window.move_group(key, scale, translation, 1.0)
            }
            EmbeddedForumRailSurfaceMotion::Content { .. } => {
                let painted_opacity = EmbeddedForumRailMetrics::content_opacity(painted);
                if progress < painted || painted_opacity <= 0.0 {
                    return false;
                }
                let shift = EmbeddedForumRailMetrics::content_shift(progress, self.container_width)
                    - EmbeddedForumRailMetrics::content_shift(painted, self.container_width);
                let fade = EmbeddedForumRailMetrics::content_opacity(progress) / painted_opacity;
                window.move_group(key, 1.0, point(px(shift), px(0.0)), fade)
            }
            EmbeddedForumRailSurfaceMotion::Fade => {
                let painted_opacity = EmbeddedForumRailMetrics::content_opacity(painted);
                if painted_opacity <= 0.0 {
                    return false;
                }
                let fade = EmbeddedForumRailMetrics::content_opacity(progress) / painted_opacity;
                window.move_group(key, 1.0, point(px(0.0), px(0.0)), fade)
            }
        }
    }
}

struct EmbeddedForumRailAnimator(Animated<f32>);

impl Default for EmbeddedForumRailAnimator {
    fn default() -> Self {
        Self(Animated::new(0.0))
    }
}

impl EmbeddedForumRailMotionDriver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> EmbeddedForumRailSnapshot {
        self.snapshot.get()
    }

    pub fn attach(&self, target: Rc<dyn EmbeddedForumRailMotionTarget>) {
        self.targets.borrow_mut().insert(Self::target_key(&target), Rc::downgrade(&target));
        target.apply_rail_motion(&self.snapshot.get());
    }

    pub fn detach(&self, target: &Rc<dyn EmbeddedForumRailMotionTarget>) {
        self.targets.borrow_mut().remove(&Self::target_key(target));
    }

    pub fn update(&self, snapshot: EmbeddedForumRailSnapshot, animated: bool) {
        let now = RiseClock::monotonic();
        if self.snapshot.get() == snapshot && (animated || !self.is_running(now)) {
            return;
        }
        let was_presented = self.snapshot.get().is_presented;
        let was_closing = !was_presented && self.presence.borrow().0.is_running(now);
        self.snapshot.set(snapshot);
        let presence = if snapshot.is_presented { 1.0 } else { 0.0 };
        if animated {
            let spec = EmbeddedForumTransitionMetrics::animation();
            let reduced = reduce_motion();
            self.presence.borrow_mut().0.set(presence, spec, now, reduced);
            if snapshot.is_presented {
                let mut drag_offset = self.drag_offset.borrow_mut();
                if was_presented || was_closing {
                    drag_offset.0.set(snapshot.drag_offset, spec, now, reduced);
                } else {
                    drag_offset.0.jump(snapshot.drag_offset);
                }
            }
        } else {
            self.presence.borrow_mut().0.jump(presence);
            self.drag_offset.borrow_mut().0.jump(snapshot.drag_offset);
        }
        let live: Vec<Rc<dyn EmbeddedForumRailMotionTarget>> = {
            let mut targets = self.targets.borrow_mut();
            targets.retain(|_, target| target.strong_count() > 0);
            targets.values().filter_map(Weak::upgrade).collect()
        };
        for target in live {
            target.apply_rail_motion(&snapshot);
        }
    }

    /// the progress on screen right now
    pub fn presented_progress(&self, container_width: f32) -> f32 {
        let now = RiseClock::monotonic();
        let presence = self.presence.borrow().0.value(now);
        let drag_offset = self.drag_offset.borrow().0.value(now);
        presence * EmbeddedForumTransitionMetrics::progress(true, drag_offset, container_width)
    }

    pub fn presented_panel_offset(&self, container_width: f32) -> f32 {
        self.panel_offset_at(RiseClock::monotonic(), container_width)
    }

    pub fn panel_offset_at(&self, now: Duration, container_width: f32) -> f32 {
        let presence = self.presence.borrow().0.value(now);
        let drag_offset = self.drag_offset.borrow().0.value(now);
        let distance = EmbeddedForumTransitionMetrics::travel_distance(container_width);
        (1.0 - presence) * distance
            + presence * EmbeddedForumTransitionMetrics::panel_offset(true, drag_offset, container_width)
    }

    /// remembers the view being drawn as one that paints the motion values
    pub fn note_reader(&self, window: &Window) {
        self.readers.borrow_mut().insert(window.current_view());
    }

    pub fn note_row_reader(&self, window: &Window) {
        self.row_readers.borrow_mut().insert(window.current_view());
    }

    fn note_visible_reader(&self, window: &Window) {
        self.readers.borrow_mut().insert(window.current_view());
    }

    fn note_group(&self, key: u64, group: EmbeddedForumRailGroup) {
        self.groups.borrow_mut().insert(key, group);
    }

    /// one motion frame, redraws the views that painted the motion last
    pub(crate) fn notify_readers(&self, window: &mut Window, cx: &mut App) {
        let now = RiseClock::monotonic();
        let running = self.is_running(now);
        let started = running && !self.was_running.replace(running);
        let mut readers = std::mem::take(&mut *self.readers.borrow_mut());
        if started || !running {
            let presented = (self.presence.borrow().0.value(now), self.drag_offset.borrow().0.value(now));
            if !running && self.settled_at.get() == Some(presented) {
                self.readers.borrow_mut().extend(readers);
                return;
            }
            self.settled_at.set((!running).then_some(presented));
            readers.extend(std::mem::take(&mut *self.row_readers.borrow_mut()));
            self.groups.borrow_mut().clear();
        }
        let mut moved = HashSet::new();
        if running && !started {
            let mut failed = HashSet::new();
            self.groups.borrow_mut().retain(|key, group| {
                let progress = self.presented_progress(group.container_width);
                let rode = group.ride(*key, progress, window);
                if rode {
                    moved.insert(group.view);
                } else {
                    failed.insert(group.view);
                }
                rode
            });
            readers.retain(|view| !moved.contains(view) || failed.contains(view));
            readers.extend(failed);
        }
        if readers.is_empty() {
            if moved.is_empty() {
                window.refresh();
            }
            return;
        }
        for reader in readers {
            cx.notify(reader);
        }
    }

    fn is_running(&self, now: Duration) -> bool {
        self.presence.borrow().0.is_running(now) || self.drag_offset.borrow().0.is_running(now)
    }

    fn target_key(target: &Rc<dyn EmbeddedForumRailMotionTarget>) -> usize {
        Rc::as_ptr(target) as *const () as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum EmbeddedForumRailSurfaceMotion {
    Avatar { size: f32, padding: f32 },
    Content { clip_extension: f32 },
    Fade,
}

impl EmbeddedForumRailSurfaceMotion {
    fn kind(&self) -> u8 {
        match self {
            Self::Avatar { .. } => 0,
            Self::Content { .. } => 1,
            Self::Fade => 2,
        }
    }
}

/// content keeps its layout slot, a pan or open and close only changes where it is painted
/// the shift is local to the row
#[derive(IntoElement)]
struct EmbeddedForumRailSurface {
    rail: EmbeddedForumRailContext,
    motion: EmbeddedForumRailSurfaceMotion,
    content: AnyElement,
}

impl RenderOnce for EmbeddedForumRailSurface {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let container_width = self.rail.container_width;
        self.rail.rail_motion.note_row_reader(window);
        let progress = self.rail.rail_motion.presented_progress(container_width);
        let surface = match self.motion {
            EmbeddedForumRailSurfaceMotion::Avatar { size, padding } => {
                let scale = EmbeddedForumRailMetrics::avatar_scale(progress, size);
                let shift = EmbeddedForumRailMetrics::avatar_shift(progress, size, padding);
                let offset_fraction = if size > 0.0 { shift / size } else { 0.0 };
                div()
                    .flex_none()
                    .w(px(size))
                    .h(px(size))
                    .child(self.content)
                    .transition_effect(TransitionEffect {
                        offset: point(offset_fraction, 0.0),
                        scale,
                        ..TransitionEffect::IDENTITY
                    })
                    .into_any_element()
            }
            EmbeddedForumRailSurfaceMotion::Content { clip_extension } => EmbeddedForumMotionLayer {
                shift_x: EmbeddedForumRailMetrics::content_shift(progress, container_width),
                leading_clip_extension: Some(clip_extension),
                child: div()
                    .w_full()
                    .min_w_0()
                    .opacity(EmbeddedForumRailMetrics::content_opacity(progress))
                    .child(self.content)
                    .into_any_element(),
            }
            .into_any_element(),
            EmbeddedForumRailSurfaceMotion::Fade => div()
                .w_full()
                .min_w_0()
                .opacity(EmbeddedForumRailMetrics::content_opacity(progress))
                .child(self.content)
                .into_any_element(),
        };
        EmbeddedForumVisibleReaderProbe {
            rail_motion: self.rail.rail_motion,
            motion: self.motion,
            painted_progress: progress,
            container_width,
            child: surface,
        }
    }
}

struct EmbeddedForumVisibleReaderProbe {
    rail_motion: Rc<EmbeddedForumRailMotionDriver>,
    motion: EmbeddedForumRailSurfaceMotion,
    painted_progress: f32,
    container_width: f32,
    child: AnyElement,
}

impl IntoElement for EmbeddedForumVisibleReaderProbe {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EmbeddedForumVisibleReaderProbe {
    type RequestLayoutState = ();
    type PrepaintState = bool;

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
        let visible = window.content_mask().bounds.intersects(&bounds);
        if visible {
            self.rail_motion.note_visible_reader(window);
        }
        self.child.prepaint(window, cx);
        visible
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        visible: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !*visible {
            self.child.paint(window, cx);
            return;
        }
        let view = window.current_view();
        let mut hasher = DefaultHasher::new();
        view.hash(&mut hasher);
        self.motion.kind().hash(&mut hasher);
        f32::from(bounds.origin.x).to_bits().hash(&mut hasher);
        f32::from(bounds.origin.y).to_bits().hash(&mut hasher);
        let key = hasher.finish() | 1;
        let child = &mut self.child;
        window.with_shift_group(key, |window| child.paint(window, cx));
        self.rail_motion.note_group(
            key,
            EmbeddedForumRailGroup {
                view,
                motion: self.motion,
                bounds,
                painted_progress: self.painted_progress,
                container_width: self.container_width,
            },
        );
    }
}

/// horizontal paint offset with optional clip to the left of the slot
/// adds no layout node so the row lays out like it wasnt there
struct EmbeddedForumMotionLayer {
    shift_x: f32,
    leading_clip_extension: Option<f32>,
    child: AnyElement,
}

impl IntoElement for EmbeddedForumMotionLayer {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EmbeddedForumMotionLayer {
    type RequestLayoutState = ();
    type PrepaintState = Option<ContentMask<Pixels>>;

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
        let mask = self.leading_clip_extension.map(|extension| {
            let extension = px(extension);
            ContentMask {
                bounds: Bounds {
                    origin: point(bounds.origin.x - extension, bounds.origin.y),
                    size: gpui::size(bounds.size.width + extension, bounds.size.height),
                },
            }
        });
        let offset = point(px(self.shift_x), px(0.0));
        let child = &mut self.child;
        window.with_content_mask(mask.clone(), |window| {
            window.with_element_offset(offset, |window| child.prepaint(window, cx))
        });
        mask
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        mask: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let child = &mut self.child;
        window.with_content_mask(mask.clone(), |window| child.paint(window, cx));
    }
}

pub enum EmbeddedForumPresentationSessionPolicy {}

impl EmbeddedForumPresentationSessionPolicy {
    pub fn is_current(
        current_chat_id: Option<&str>,
        current_epoch: u64,
        expected_chat_id: &str,
        expected_epoch: u64,
    ) -> bool {
        current_chat_id == Some(expected_chat_id) && current_epoch == expected_epoch
    }

    pub fn should_cleanup(
        current_chat_id: Option<&str>,
        current_epoch: u64,
        is_presented: bool,
        closing_chat_id: &str,
        closing_epoch: u64,
    ) -> bool {
        !is_presented && Self::is_current(current_chat_id, current_epoch, closing_chat_id, closing_epoch)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddedForumSwipeAxisLockDecision {
    Undecided,
    Horizontal,
    Rejected,
}

#[derive(Clone, Copy, Debug)]
pub struct EmbeddedForumSwipeAxisLock {
    activation_distance: f32,
    horizontal_dominance_ratio: f32,
    decision: EmbeddedForumSwipeAxisLockDecision,
    horizontal_activation_offset: f32,
}

impl Default for EmbeddedForumSwipeAxisLock {
    fn default() -> Self {
        Self::new(EmbeddedForumTransitionMetrics::SWIPE_MINIMUM_DISTANCE, Self::DEFAULT_DOMINANCE_RATIO)
    }
}

impl EmbeddedForumSwipeAxisLock {
    pub const DEFAULT_DOMINANCE_RATIO: f32 = 1.2;

    pub fn new(activation_distance: f32, horizontal_dominance_ratio: f32) -> Self {
        Self {
            activation_distance,
            horizontal_dominance_ratio,
            decision: EmbeddedForumSwipeAxisLockDecision::Undecided,
            horizontal_activation_offset: 0.0,
        }
    }

    pub fn decision(&self) -> EmbeddedForumSwipeAxisLockDecision {
        self.decision
    }

    pub fn horizontal_activation_offset(&self) -> f32 {
        self.horizontal_activation_offset
    }

    pub fn accepts_close_drag(&self) -> bool {
        self.decision == EmbeddedForumSwipeAxisLockDecision::Horizontal
    }

    pub fn suppresses_vertical_scroll(&self) -> bool {
        self.decision == EmbeddedForumSwipeAxisLockDecision::Horizontal
    }

    pub fn accepts_rightward_horizontal_intent(horizontal: f32, vertical: f32, dominance_ratio: f32) -> bool {
        horizontal > 0.0 && horizontal.abs() > vertical.abs() * dominance_ratio
    }

    pub fn update(&mut self, dx: f32, dy: f32) -> EmbeddedForumSwipeAxisLockDecision {
        if self.decision != EmbeddedForumSwipeAxisLockDecision::Undecided {
            return self.decision;
        }
        if dx.abs().max(dy.abs()) < self.activation_distance {
            return self.decision;
        }
        if Self::accepts_rightward_horizontal_intent(dx, dy, self.horizontal_dominance_ratio) {
            self.decision = EmbeddedForumSwipeAxisLockDecision::Horizontal;
            self.horizontal_activation_offset = dx.max(0.0).min(self.activation_distance);
        } else {
            self.decision = EmbeddedForumSwipeAxisLockDecision::Rejected;
        }
        self.decision
    }

    pub fn interactive_translation(&self, dx: f32) -> f32 {
        if self.decision != EmbeddedForumSwipeAxisLockDecision::Horizontal {
            return 0.0;
        }
        (dx - self.horizontal_activation_offset).max(0.0)
    }

    pub fn reset(&mut self) {
        self.decision = EmbeddedForumSwipeAxisLockDecision::Undecided;
        self.horizontal_activation_offset = 0.0;
    }
}

pub struct EmbeddedForumTransitionState {
    is_presented: bool,
    drag_offset: f32,
    is_interactive_gesture_active: bool,
    is_interaction_settling: bool,
    structural_updates_blocked: bool,
    structural_refresh_generation: u64,
    interaction_generation: u64,
    motion_cadence: RiseMotionCadence,
    motion_cadence_requested: Cell<bool>,
    rail_motion: Rc<EmbeddedForumRailMotionDriver>,
    settle_task: Option<Task<()>>,
}

impl Default for EmbeddedForumTransitionState {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddedForumTransitionState {
    pub fn new() -> Self {
        Self {
            is_presented: false,
            drag_offset: 0.0,
            is_interactive_gesture_active: false,
            is_interaction_settling: false,
            structural_updates_blocked: true,
            structural_refresh_generation: 0,
            interaction_generation: 0,
            motion_cadence: RiseMotionCadence::new(),
            motion_cadence_requested: Cell::new(false),
            rail_motion: Rc::new(EmbeddedForumRailMotionDriver::new()),
            settle_task: None,
        }
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn drag_offset(&self) -> f32 {
        self.drag_offset
    }

    pub fn is_interactive_gesture_active(&self) -> bool {
        self.is_interactive_gesture_active
    }

    pub fn is_interaction_settling(&self) -> bool {
        self.is_interaction_settling
    }

    pub fn structural_updates_blocked(&self) -> bool {
        self.structural_updates_blocked
    }

    pub fn structural_refresh_generation(&self) -> u64 {
        self.structural_refresh_generation
    }

    pub fn rail_motion(&self) -> &Rc<EmbeddedForumRailMotionDriver> {
        &self.rail_motion
    }

    pub fn has_interactive_offset(&self) -> bool {
        self.drag_offset > 0.5
    }

    pub fn prepare_for_presentation(&mut self, cx: &mut Context<Self>) {
        self.stop_motion_cadence();
        self.is_presented = false;
        self.drag_offset = 0.0;
        self.is_interactive_gesture_active = false;
        self.is_interaction_settling = false;
        self.structural_updates_blocked = true;
        self.interaction_generation = self.interaction_generation.wrapping_add(1);
        self.publish_rail_motion(false);
        cx.notify();
    }

    pub fn set_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if self.is_presented == presented {
            return;
        }
        self.motion_cadence_requested.set(true);
        ChatPerformanceTrace::shared()
            .record(if presented { "forum.open_begin" } else { "forum.close_begin" }, HashMap::new());
        self.is_presented = presented;
        if !presented {
            self.structural_updates_blocked = true;
            self.interaction_generation = self.interaction_generation.wrapping_add(1);
            self.is_interactive_gesture_active = false;
            self.is_interaction_settling = false;
        }
        self.publish_rail_motion(true);
        cx.notify();
    }

    pub fn update_drag_offset(&mut self, offset: f32, container_width: f32, cx: &mut Context<Self>) {
        let travel = EmbeddedForumTransitionMetrics::travel_distance(container_width);
        let next = offset.max(0.0).min(travel);
        if (next - self.drag_offset).abs() <= 0.25 {
            return;
        }
        self.motion_cadence_requested.set(true);
        self.drag_offset = next;
        self.publish_rail_motion(false);
        cx.notify();
    }

    pub fn reset_drag_offset(&mut self, cx: &mut Context<Self>) {
        if self.drag_offset == 0.0 {
            return;
        }
        self.drag_offset = 0.0;
        if self.is_presented {
            self.publish_rail_motion(true);
        }
        cx.notify();
    }

    pub fn begin_interactive_gesture(&mut self, cx: &mut Context<Self>) {
        if self.is_interactive_gesture_active {
            return;
        }
        self.structural_updates_blocked = true;
        self.interaction_generation = self.interaction_generation.wrapping_add(1);
        self.is_interaction_settling = false;
        self.is_interactive_gesture_active = true;
        cx.notify();
    }

    pub fn end_interactive_gesture(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_gesture_active {
            return;
        }
        self.is_interactive_gesture_active = false;
        cx.notify();
    }

    /// logical state goes to zero at once while the painted panel still moves
    /// the settle phase keeps cache and ui work from resuming during the snap back
    pub fn snap_back_after_interaction(&mut self, cx: &mut Context<Self>) {
        self.motion_cadence_requested.set(true);
        self.is_interactive_gesture_active = false;
        self.interaction_generation = self.interaction_generation.wrapping_add(1);
        let generation = self.interaction_generation;
        let needs_visual_settle = self.has_interactive_offset();
        self.is_interaction_settling = needs_visual_settle;
        self.reset_drag_offset(cx);
        cx.notify();
        if !needs_visual_settle {
            self.release_structural_updates_if_idle(cx);
            return;
        }

        self.settle_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(EmbeddedForumTransitionMetrics::ANIMATION_DURATION))
                .await;
            let _ = this.update(cx, |state, cx| {
                if state.interaction_generation != generation
                    || !state.is_presented
                    || state.is_interactive_gesture_active
                {
                    return;
                }
                state.is_interaction_settling = false;
                cx.notify();
                state.release_structural_updates_if_idle(cx);
            });
        }));
    }

    /// publishes at most one hierarchy refresh while the panel is still
    /// safe from delayed warmup and gesture settle
    pub fn release_structural_updates_if_idle(&mut self, cx: &mut Context<Self>) {
        if !self.is_presented
            || self.is_interactive_gesture_active
            || self.is_interaction_settling
            || self.has_interactive_offset()
            || !self.structural_updates_blocked
        {
            return;
        }
        self.structural_updates_blocked = false;
        self.structural_refresh_generation = self.structural_refresh_generation.wrapping_add(1);
        cx.notify();
    }

    pub fn dismiss_immediately(&mut self, cx: &mut Context<Self>) {
        self.stop_motion_cadence();
        self.is_presented = false;
        self.drag_offset = 0.0;
        self.is_interactive_gesture_active = false;
        self.is_interaction_settling = false;
        self.structural_updates_blocked = true;
        self.interaction_generation = self.interaction_generation.wrapping_add(1);
        self.publish_rail_motion(false);
        cx.notify();
    }

    fn stop_motion_cadence(&self) {
        self.motion_cadence_requested.set(false);
        self.motion_cadence.stop();
    }

    fn publish_rail_motion(&self, animated: bool) {
        self.rail_motion.update(
            EmbeddedForumRailSnapshot {
                is_presented: self.is_presented,
                drag_offset: if self.is_presented { self.drag_offset } else { 0.0 },
            },
            animated,
        );
    }

    pub(crate) fn drive_motion_cadence(this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let state = this.read(cx);
        if !state.motion_cadence_requested.replace(false) {
            return;
        }
        let cadence = state.motion_cadence.clone();
        let driver = state.rail_motion.clone();
        let duration = Duration::from_secs_f64(EmbeddedForumTransitionMetrics::PRESENTATION_SETTLE_DURATION);
        let on_frame: Rc<dyn Fn(&mut Window, &mut App)> =
            Rc::new(move |window, cx| driver.notify_readers(window, cx));
        cadence.request(duration, Some(on_frame), window, cx);
    }

    pub fn progress(&self, container_width: f32) -> f32 {
        EmbeddedForumTransitionMetrics::progress(self.is_presented, self.drag_offset, container_width)
    }

    pub fn panel_offset(&self, container_width: f32) -> f32 {
        EmbeddedForumTransitionMetrics::panel_offset(self.is_presented, self.drag_offset, container_width)
    }
}

/// reads the driver on-screen progress not the logical one, nothing interpolates the logical
/// value here
#[derive(IntoElement)]
pub struct EmbeddedForumProgressReader {
    transition: Entity<EmbeddedForumTransitionState>,
    container_width: f32,
    content_revision: String,
    content: Box<dyn FnOnce(f32, &mut Window, &mut App) -> AnyElement>,
}

impl EmbeddedForumProgressReader {
    pub fn new(
        transition: Entity<EmbeddedForumTransitionState>,
        container_width: f32,
        content_revision: String,
        content: impl FnOnce(f32, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self { transition, container_width, content_revision, content: Box::new(content) }
    }
}

impl RenderOnce for EmbeddedForumProgressReader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let rail_motion = self.transition.read(cx).rail_motion();
        rail_motion.note_reader(window);
        let progress =
            EmbeddedForumTransitionMetrics::sampled_header_progress(rail_motion.presented_progress(self.container_width));
        EmbeddedForumStableProgressContent {
            progress,
            content_revision: self.content_revision,
            content: self.content,
        }
    }
}

#[derive(IntoElement)]
struct EmbeddedForumStableProgressContent {
    progress: f32,
    content_revision: String,
    content: Box<dyn FnOnce(f32, &mut Window, &mut App) -> AnyElement>,
}

impl PartialEq for EmbeddedForumStableProgressContent {
    fn eq(&self, other: &Self) -> bool {
        self.progress == other.progress && self.content_revision == other.content_revision
    }
}

impl RenderOnce for EmbeddedForumStableProgressContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        (self.content)(self.progress, window, cx)
    }
}

#[derive(IntoElement)]
pub struct EmbeddedForumTransitionLayer {
    pub chat_id: String,
    pub presentation_epoch: u64,
    pub initial_snapshot: ForumTopicListSnapshot,
    pub container_width: f32,
    pub bottom_inset: f32,
    pub chrome_collapse: Entity<ListShellHeaderCollapse>,
    pub transition: Entity<EmbeddedForumTransitionState>,
    pub on_open_all: EmbeddedForumAction,
    pub on_open_topic: EmbeddedForumOpenTopicAction,
    pub on_create_section: EmbeddedForumAction,
    pub on_close: EmbeddedForumAction,
    pub on_mounted: EmbeddedForumAction,
}

impl RenderOnce for EmbeddedForumTransitionLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        EmbeddedForumTransitionState::drive_motion_cadence(&self.transition, window, cx);
        let epoch = self.presentation_epoch;

        let mounted = window.use_keyed_state(("embedded-forum-layer-mounted", epoch), cx, |_, _| Cell::new(false));
        if !mounted.read(cx).replace(true) {
            let on_mounted = self.on_mounted.clone();
            window.defer(cx, move |window, cx| on_mounted(window, cx));
        }

        let top_inset = env::<ListShellFloatingTopInsetKey>(cx);
        let chrome_fold = self.chrome_collapse.read(cx).presented_offset(window).clamp(0.0, top_inset.max(0.0));

        div()
            .id(("embedded-forum-transition-layer", epoch))
            .relative()
            .size_full()
            .overflow_hidden()
            .child(div().absolute().inset_0().child(EmbeddedForumBackdrop {
                container_width: self.container_width,
                transition: self.transition.clone(),
            }))
            .child(div().absolute().inset_0().child(EmbeddedForumPanel {
                chat_id: self.chat_id,
                presentation_epoch: epoch,
                initial_snapshot: self.initial_snapshot,
                container_width: self.container_width,
                top_inset,
                chrome_fold,
                bottom_inset: self.bottom_inset,
                transition: self.transition,
                on_open_all: self.on_open_all,
                on_open_topic: self.on_open_topic,
                on_create_section: self.on_create_section,
                on_close: self.on_close,
            }))
    }
}

#[derive(IntoElement)]
struct EmbeddedForumBackdrop {
    container_width: f32,
    transition: Entity<EmbeddedForumTransitionState>,
}

impl RenderOnce for EmbeddedForumBackdrop {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let rail_motion = self.transition.read(cx).rail_motion();
        rail_motion.note_reader(window);
        let progress = rail_motion.presented_progress(self.container_width);
        div()
            .flex()
            .flex_row()
            .size_full()
            .child(div().flex_none().w(px(EmbeddedForumTransitionMetrics::AVATAR_RAIL_WIDTH)))
            .child(
                div().flex_1().h_full().opacity(progress).child(
                    LinearGradient::colors(
                        [black().opacity(0.36), black().opacity(0.52)],
                        UnitPoint::LEADING,
                        UnitPoint::TRAILING,
                    )
                    .view()
                    .size_full(),
                ),
            )
    }
}

#[derive(IntoElement)]
pub struct EmbeddedForumPanel {
    pub chat_id: String,
    pub presentation_epoch: u64,
    pub initial_snapshot: ForumTopicListSnapshot,
    pub container_width: f32,
    pub top_inset: f32,
    pub chrome_fold: f32,
    pub bottom_inset: f32,
    pub transition: Entity<EmbeddedForumTransitionState>,
    pub on_open_all: EmbeddedForumAction,
    pub on_open_topic: EmbeddedForumOpenTopicAction,
    pub on_create_section: EmbeddedForumAction,
    pub on_close: EmbeddedForumAction,
}

impl EmbeddedForumPanel {
    fn horizontal_pan_callbacks(&self) -> EmbeddedForumHorizontalPanCallbacks {
        let container_width = self.container_width;
        let changed = self.transition.clone();
        let ended = self.transition.clone();
        let cancelled = self.transition.clone();
        let on_close = self.on_close.clone();
        EmbeddedForumHorizontalPanCallbacks {
            on_changed: Rc::new(move |offset: f32, _: &mut Window, cx: &mut App| {
                changed.update(cx, |transition, cx| {
                    if !transition.is_presented() {
                        return;
                    }
                    transition.begin_interactive_gesture(cx);
                    transition.update_drag_offset(offset, container_width, cx);
                });
            }),
            on_ended: Rc::new(move |translation: f32, predicted_translation: f32, window: &mut Window, cx: &mut App| {
                let closes = ended.update(cx, |transition, cx| {
                    if !transition.is_presented() {
                        return None;
                    }
                    transition.end_interactive_gesture(cx);
                    Some(EmbeddedForumTransitionMetrics::should_close(
                        translation,
                        predicted_translation,
                        container_width,
                    ))
                });
                match closes {
                    Some(true) => on_close(window, cx),
                    Some(false) => ended.update(cx, |transition, cx| transition.snap_back_after_interaction(cx)),
                    None => {}
                }
            }),
            on_cancelled: Rc::new(move |_: &mut Window, cx: &mut App| {
                cancelled.update(cx, |transition, cx| transition.snap_back_after_interaction(cx));
            }),
        }
    }
}

impl RenderOnce for EmbeddedForumPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = rise_ui::theme(cx).bg._100;
        let border = rise_ui::theme(cx).border._100;
        let epoch = self.presentation_epoch;
        let callbacks = self.horizontal_pan_callbacks();
        let rail_motion = self.transition.read(cx).rail_motion();
        rail_motion.note_reader(window);
        let panel_offset = rail_motion.presented_panel_offset(self.container_width);
        let is_presented = self.transition.read(cx).is_presented();

        let focus = window.use_keyed_state(("embedded-forum-panel-focus", epoch), cx, |_, cx| {
            install_key_bindings(cx);
            cx.focus_handle()
        });
        let focus = focus.read(cx).clone();
        let focused_once = window.use_keyed_state(("embedded-forum-panel-focused", epoch), cx, |_, _| Cell::new(false));
        let is_first_render = !focused_once.read(cx).replace(true);
        if is_first_render || (is_presented && ScreenFocus::is_idle(window, cx)) {
            let focus = focus.clone();
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
        let escape_close = self.on_close.clone();
        let escape_focus = focus.clone();
        let arrow_close = self.on_close.clone();
        let arrow_focus = focus.clone();

        let topic_list = EmbeddedForumStableTopicList {
            chat_id: self.chat_id,
            presentation_epoch: epoch,
            initial_snapshot: self.initial_snapshot,
            top_inset: self.top_inset,
            chrome_fold: self.chrome_fold,
            bottom_inset: self.bottom_inset,
            transition: self.transition,
            on_open_all: self.on_open_all,
            on_open_topic: self.on_open_topic,
            on_create_section: self.on_create_section,
        };

        let panel = div()
            .flex()
            .flex_row()
            .size_full()
            .child(div().flex_none().w(px(EmbeddedForumTransitionMetrics::AVATAR_RAIL_WIDTH)))
            .child(
                div()
                    .key_context(KEY_CONTEXT)
                    .track_focus(&focus)
                    .on_action(move |_: &DismissModal, window: &mut Window, cx: &mut App| {
                        if !escape_focus.is_focused(window) && escape_focus.contains_focused(window, cx) {
                            tracing::debug!(target: "riseonly", "X1053 forum escape passed to inner layer");
                            cx.propagate();
                            return;
                        }
                        escape_close(window, cx)
                    })
                    .on_key_down(move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                        if !arrow_focus.is_focused(window)
                            || event.keystroke.key != "left"
                            || event.keystroke.modifiers.modified()
                        {
                            return;
                        }
                        cx.stop_propagation();
                        arrow_close(window, cx);
                    })
                    .occlude()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(background)
                    .child(env_scope::<EmbeddedForumHorizontalPanCallbacksKey>(Some(callbacks)).child(topic_list))
                    .child(
                        div().absolute().top_0().bottom_0().left(px(-14.0)).w(px(14.0)).child(
                            LinearGradient::colors(
                                [background.opacity(0.0), background.opacity(0.22)],
                                UnitPoint::LEADING,
                                UnitPoint::TRAILING,
                            )
                            .view()
                            .size_full(),
                        ),
                    )
                    .child(div().absolute().top_0().bottom_0().left_0().w(px(0.5)).bg(border.opacity(0.75))),
            )
            .into_any_element();

        EmbeddedForumMotionLayer { shift_x: panel_offset, leading_clip_extension: None, child: panel }
    }
}

#[derive(IntoElement)]
struct EmbeddedForumStableTopicList {
    chat_id: String,
    presentation_epoch: u64,
    initial_snapshot: ForumTopicListSnapshot,
    top_inset: f32,
    chrome_fold: f32,
    bottom_inset: f32,
    transition: Entity<EmbeddedForumTransitionState>,
    on_open_all: EmbeddedForumAction,
    on_open_topic: EmbeddedForumOpenTopicAction,
    on_create_section: EmbeddedForumAction,
}

impl RenderOnce for EmbeddedForumStableTopicList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let epoch = self.presentation_epoch;
        let list = match embedded_forum_panel_hooks() {
            Some(hooks) => (hooks.forum_topic_list)(
                EmbeddedForumPanelForumTopicListArgs {
                    chat_id: self.chat_id,
                    top_inset: self.top_inset,
                    bottom_inset: self.bottom_inset,
                    interaction_scroll_locked: false,
                    installs_horizontal_swipe_gate: true,
                    initial_presentation_snapshot: self.initial_snapshot,
                    presentation_epoch: epoch,
                    presentation_transition: self.transition,
                    on_open_all: self.on_open_all,
                    on_open_topic: self.on_open_topic,
                    on_create_section: self.on_create_section,
                },
                window,
                cx,
            ),
            None => div().into_any_element(),
        };
        div().id(("embedded-forum-topic-list", epoch)).relative().size_full().child(
            div()
                .absolute()
                .top(px(-self.chrome_fold))
                .bottom_0()
                .left_0()
                .right_0()
                .child(list),
        )
    }
}
