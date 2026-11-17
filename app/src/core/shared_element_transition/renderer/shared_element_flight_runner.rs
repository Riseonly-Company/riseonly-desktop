use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use rise_core::foundation::foundation_date::RiseClock;

use gpui::{App, AppContext as _, AnyView, Bounds, Entity, EntityId, Pixels, Point, RenderImage, Task, point, px, size};

use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementCoverSchedule, SharedElementEndpoint, SharedElementGeometry,
    SharedElementGesturePolicy, SharedElementGesturePolicyFrame, SharedElementReleaseOutcome,
};
use crate::core::shared_element_transition::renderer::shared_element_flight_view::{
    SharedElementFlightAnimation, SharedElementFlightTiming, SharedElementFlightView, SharedElementFlightViewLayout,
    SharedElementOpacityKeyframes,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCover, SharedElementFlightContent,
};
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;

/// view the flight is added to, its space is window space
/// a desktop container covers its window from the origin
pub trait SharedElementFlightContainer {
    fn add_flight_view(&self, view: AnyView, cx: &mut App);
    fn remove_flight_view(&self, view: EntityId, cx: &mut App);
}

/// block called every frame with the animation progress on its own curve, and with 1 when it completes
pub type SharedElementFlightAlongside = Rc<dyn Fn(f32, &mut App)>;

type SharedElementFlightCompletion = Box<dyn FnOnce(&mut App)>;

struct SharedElementFlightAnimator {
    completion: Option<SharedElementFlightCompletion>,
    _timer: Task<()>,
}

struct SharedElementFlightRunnerState {
    flight_view: Entity<SharedElementFlightView>,
    epoch: u64,
    has_begun: Cell<bool>,
    current_endpoint: Cell<Option<SharedElementEndpoint>>,
    container: RefCell<Option<Rc<dyn SharedElementFlightContainer>>>,
    content_aspect: Cell<f32>,
    cover_aspect: Cell<Option<f32>>,
    animator: RefCell<Option<SharedElementFlightAnimator>>,
    animation_token: Cell<u64>,
    linger_token: Cell<u64>,
    linger: RefCell<Option<Task<()>>>,
    is_lingering: Cell<bool>,
    is_aborted: Cell<bool>,
    removal_requested: Cell<bool>,
    removal_scheduled: Cell<bool>,
}

#[derive(Clone)]
pub struct SharedElementFlightRunner {
    state: Rc<SharedElementFlightRunnerState>,
}

/// weak handle to the runner, doesnt keep the flight alive
#[derive(Clone)]
pub struct WeakSharedElementFlightRunner {
    state: Weak<SharedElementFlightRunnerState>,
}

impl WeakSharedElementFlightRunner {
    pub fn upgrade(&self) -> Option<SharedElementFlightRunner> {
        SharedElementFlightRunner::upgrade(&self.state)
    }
}

impl SharedElementFlightRunner {
    pub fn new(container: Rc<dyn SharedElementFlightContainer>, epoch: u64, cx: &mut App) -> Self {
        let flight_view = cx.new(SharedElementFlightView::new);
        Self {
            state: Rc::new(SharedElementFlightRunnerState {
                flight_view,
                epoch,
                has_begun: Cell::new(false),
                current_endpoint: Cell::new(None),
                container: RefCell::new(Some(container)),
                content_aspect: Cell::new(1.0),
                cover_aspect: Cell::new(None),
                animator: RefCell::new(None),
                animation_token: Cell::new(0),
                linger_token: Cell::new(0),
                linger: RefCell::new(None),
                is_lingering: Cell::new(false),
                is_aborted: Cell::new(false),
                removal_requested: Cell::new(false),
                removal_scheduled: Cell::new(false),
            }),
        }
    }

    pub fn downgrade(&self) -> WeakSharedElementFlightRunner {
        WeakSharedElementFlightRunner { state: Rc::downgrade(&self.state) }
    }

    pub fn flight_view(&self) -> Entity<SharedElementFlightView> {
        self.state.flight_view.clone()
    }

    pub fn epoch(&self) -> u64 {
        self.state.epoch
    }

    pub fn has_begun(&self) -> bool {
        self.state.has_begun.get()
    }

    pub fn current_endpoint(&self) -> Option<SharedElementEndpoint> {
        self.state.current_endpoint.get()
    }

    pub fn begin(
        &self,
        content: SharedElementFlightContent,
        source: SharedElementEndpoint,
        backdrop: Option<Arc<RenderImage>>,
        cx: &mut App,
    ) {
        let state = &self.state;
        if state.has_begun.get() || state.is_aborted.get() {
            return;
        }
        let Some(container) = state.container.borrow().clone() else { return };
        state.content_aspect.set(Self::aspect_of_content(&content).or(Self::aspect_of_rect(source.content)).unwrap_or(1.0));
        state.flight_view.update(cx, |view, cx| {
            view.set_content(content, cx);
            view.set_backdrop_image(backdrop, cx);
            view.set_alpha(1.0, cx);
        });
        container.add_flight_view(AnyView::from(state.flight_view.clone()), cx);
        state.has_begun.set(true);
        self.write_immediately(source, cx);
    }

    /// cover starts transparent
    /// animate fades it in on the curve and a finishing release fades it in with a delay
    pub fn set_cover(&self, cover: Option<SharedElementCover>, cx: &mut App) {
        let state = &self.state;
        if state.is_aborted.get() {
            return;
        }
        state.cover_aspect.set(cover.as_ref().and_then(|cover| Self::usable(cover.aspect)));
        state.flight_view.update(cx, |view, cx| {
            view.remove_cover_keyframes(cx);
            view.set_cover_image(cover.map(|cover| cover.image), cx);
            view.set_cover_alpha(0.0, cx);
        });
        if let Some(current) = state.current_endpoint.get() {
            self.write_immediately(current, cx);
        }
    }

    pub fn update_backdrop(&self, image: Option<Arc<RenderImage>>, cx: &mut App) {
        let state = &self.state;
        if !state.has_begun.get() || state.is_aborted.get() {
            return;
        }
        state.flight_view.update(cx, |view, cx| view.set_backdrop_image(image, cx));
    }

    pub fn reveal_placeholder(&self, image: Arc<RenderImage>, cx: &mut App) {
        let state = &self.state;
        if !state.has_begun.get() || state.is_aborted.get() {
            return;
        }
        let pixels = image.size(0);
        let aspect = if pixels.height.0 > 0 { pixels.width.0 as f32 / pixels.height.0 as f32 } else { 1.0 };
        state.flight_view.update(cx, |view, cx| {
            view.set_content(SharedElementFlightContent::Bitmap(image, aspect), cx);
            view.set_placeholder(None, cx);
        });
    }

    /// cubic move over MOVE_DURATION
    pub fn animate(
        &self,
        target: SharedElementEndpoint,
        cover_start_progress: Option<f32>,
        alongside: Option<SharedElementFlightAlongside>,
        completion: SharedElementFlightCompletion,
        cx: &mut App,
    ) {
        if self.state.is_aborted.get() {
            return;
        }
        self.cancel_running_animator(cx);
        let now = RiseClock::monotonic();
        let moves = self.state.has_begun.get();
        let layout = if moves { Some(self.layout_animation_to(target, cx)) } else { None };
        let animation = SharedElementFlightAnimation {
            start: now,
            duration: SharedElementMotion::MOVE_DURATION,
            timing: SharedElementFlightTiming::Cubic(SharedElementMotion::CURVE),
            layout,
            layout_base_progress: 0.0,
            alpha: None,
            cover_alpha: None,
            alongside,
        };
        if moves
            && let Some(cover_start_progress) = cover_start_progress
            && self.state.flight_view.read(cx).has_cover_image()
        {
            self.add_opacity_keyframes(true, cover_start_progress, now, cx);
        }
        self.run(animation, completion, cx);
    }

    pub fn add_flight_fade_out(&self, start_progress: f32, cx: &mut App) {
        let state = &self.state;
        if !state.has_begun.get() || state.is_aborted.get() {
            return;
        }
        let start = state.flight_view.read(cx).animation_start().unwrap_or_else(RiseClock::monotonic);
        self.add_opacity_keyframes(false, start_progress, start, cx);
    }

    /// adds the new end state to the running animator for its remaining time
    pub fn retarget(&self, target: SharedElementEndpoint, cx: &mut App) {
        let state = &self.state;
        if !state.has_begun.get() || state.is_aborted.get() {
            return;
        }
        if state.animator.borrow().is_some() {
            let layout = self.layout(target);
            state.current_endpoint.set(Some(target));
            let now = RiseClock::monotonic();
            let retargeted = state.flight_view.update(cx, |view, cx| view.retarget_animation(layout, now, cx));
            if retargeted {
                return;
            }
        }
        self.write_immediately(target, cx);
    }

    /// finger follow write, no animation
    pub fn follow(&self, frame: SharedElementGesturePolicyFrame, source: SharedElementEndpoint, cx: &mut App) {
        let state = &self.state;
        if !state.has_begun.get() || state.is_aborted.get() {
            return;
        }
        self.cancel_running_animator(cx);
        let scale = frame.scale;
        let content = Bounds {
            origin: point(
                frame.clip.origin.x + (source.content.origin.x - source.clip.origin.x) * scale,
                frame.clip.origin.y + (source.content.origin.y - source.clip.origin.y) * scale,
            ),
            size: size(source.content.size.width * scale, source.content.size.height * scale),
        };
        let endpoint = SharedElementEndpoint {
            clip: frame.clip,
            content,
            corner_radius: source.corner_radius * scale,
            mode: source.mode,
        };
        self.write(endpoint, cx);
    }

    /// spring from the last follow write to target
    pub fn release(
        &self,
        outcome: SharedElementReleaseOutcome,
        velocity: Point<Pixels>,
        target: SharedElementEndpoint,
        alongside: Option<SharedElementFlightAlongside>,
        completion: SharedElementFlightCompletion,
        cx: &mut App,
    ) {
        if self.state.is_aborted.get() {
            return;
        }
        self.cancel_running_animator(cx);
        let target_center = target.clip.center();
        let from = self.state.current_endpoint.get().map(|endpoint| endpoint.clip.center()).unwrap_or(target_center);
        let relative = SharedElementGesturePolicy::relative_spring_velocity(velocity, from, target_center);
        let finishes = outcome == SharedElementReleaseOutcome::Finish;
        let duration =
            if finishes { SharedElementMotion::MOVE_DURATION } else { SharedElementMotion::CANCEL_DURATION };
        let damping = if finishes { SharedElementMotion::FINISH_DAMPING } else { SharedElementMotion::CANCEL_DAMPING };
        let moves = self.state.has_begun.get();
        let layout = if moves { Some(self.layout_animation_to(target, cx)) } else { None };
        let cover_alpha = if moves && finishes && self.state.flight_view.read(cx).has_cover_image() {
            Some((self.state.flight_view.read(cx).model_cover_alpha(), 1.0, SharedElementMotion::COVER_FADE_START_PROGRESS))
        } else {
            None
        };
        let animation = SharedElementFlightAnimation {
            start: RiseClock::monotonic(),
            duration,
            timing: SharedElementFlightTiming::spring(duration, damping, relative),
            layout,
            layout_base_progress: 0.0,
            alpha: None,
            cover_alpha,
            alongside,
        };
        self.run(animation, completion, cx);
    }

    /// replaces a running animation without running its completion
    pub fn fade_out(
        &self,
        duration: f64,
        scale: f32,
        alongside: Option<SharedElementFlightAlongside>,
        completion: SharedElementFlightCompletion,
        cx: &mut App,
    ) {
        let state = &self.state;
        if state.is_aborted.get() {
            return;
        }
        state.is_lingering.set(false);
        state.linger_token.set(state.linger_token.get().wrapping_add(1));
        self.start_fade(duration, scale, alongside, completion, cx);
    }

    pub fn reparent(&self, container: Rc<dyn SharedElementFlightContainer>, cx: &mut App) {
        let state = &self.state;
        if !state.has_begun.get() || state.is_aborted.get() || state.removal_scheduled.get() {
            return;
        }
        let Some(current) = state.current_endpoint.get() else { return };
        self.cancel_running_animator(cx);
        let view_id = state.flight_view.entity_id();
        if let Some(previous) = state.container.borrow_mut().replace(container.clone()) {
            previous.remove_flight_view(view_id, cx);
        }
        container.add_flight_view(AnyView::from(state.flight_view.clone()), cx);
        self.write_immediately(current, cx);
        state.is_lingering.set(true);
        state.linger_token.set(state.linger_token.get().wrapping_add(1));
        let token = state.linger_token.get();
        let weak = Rc::downgrade(&self.state);
        let timer = cx.background_executor().timer(Duration::from_secs_f64(SharedElementMotion::FULL_IMAGE_HOLD));
        let task = cx.spawn(async move |cx| {
            timer.await;
            cx.update(|cx| {
                let Some(runner) = Self::upgrade(&weak) else { return };
                let state = &runner.state;
                if !state.is_lingering.get() || state.linger_token.get() != token || state.is_aborted.get() {
                    return;
                }
                state.is_lingering.set(false);
                state.removal_requested.set(true);
                runner.start_fade(SharedElementMotion::FULL_IMAGE_CROSSFADE, 1.0, None, Box::new(|_| {}), cx);
            });
        });
        *state.linger.borrow_mut() = Some(task);
    }

    /// completions run with the end state
    pub fn jump_to_end(&self, cx: &mut App) {
        let state = &self.state;
        if state.is_aborted.get() || state.animator.borrow().is_none() {
            return;
        }
        state.flight_view.update(cx, |view, cx| {
            view.remove_cover_keyframes(cx);
            view.remove_flight_keyframes(cx);
        });
        let token = state.animation_token.get();
        let runner = self.clone();
        cx.defer(move |cx| runner.animator_did_complete(token, cx));
    }

    /// stops everything and removes the view now, no completion runs
    pub fn abort(&self, cx: &mut App) {
        let state = &self.state;
        if state.is_aborted.get() {
            return;
        }
        state.is_aborted.set(true);
        state.is_lingering.set(false);
        state.linger_token.set(state.linger_token.get().wrapping_add(1));
        state.linger.borrow_mut().take();
        self.cancel_running_animator(cx);
        self.remove_view_now(cx);
    }

    /// removes the view once the current commit is on screen, a running animation finishes first
    pub fn remove_after_commit(&self, cx: &mut App) {
        let state = &self.state;
        if state.is_aborted.get() {
            return;
        }
        state.removal_requested.set(true);
        state.is_lingering.set(false);
        state.linger_token.set(state.linger_token.get().wrapping_add(1));
        if state.animator.borrow().is_some() {
            return;
        }
        self.schedule_removal(cx);
    }

    fn upgrade(weak: &Weak<SharedElementFlightRunnerState>) -> Option<Self> {
        weak.upgrade().map(|state| Self { state })
    }

    fn start_fade(
        &self,
        duration: f64,
        scale: f32,
        alongside: Option<SharedElementFlightAlongside>,
        completion: SharedElementFlightCompletion,
        cx: &mut App,
    ) {
        let state = &self.state;
        let interrupted = state.animator.borrow().is_some();
        self.cancel_running_animator(cx);
        let scaled = match state.current_endpoint.get() {
            Some(current) if state.has_begun.get() && !interrupted && (scale - 1.0).abs() > 0.0001 => {
                Some(Self::scaled(current, scale))
            }
            _ => None,
        };
        let layout = scaled.map(|scaled| self.layout_animation_to(scaled, cx));
        let from_alpha = state.flight_view.read(cx).model_alpha();
        let animation = SharedElementFlightAnimation {
            start: RiseClock::monotonic(),
            duration: duration.max(0.0),
            timing: SharedElementFlightTiming::EaseOut,
            layout,
            layout_base_progress: 0.0,
            alpha: Some((from_alpha, 0.0)),
            cover_alpha: None,
            alongside,
        };
        self.run(animation, completion, cx);
    }

    fn run(&self, animation: SharedElementFlightAnimation, completion: SharedElementFlightCompletion, cx: &mut App) {
        let state = &self.state;
        state.animation_token.set(state.animation_token.get().wrapping_add(1));
        let token = state.animation_token.get();
        let duration = Duration::from_secs_f64(animation.duration.max(0.0));
        let weak = Rc::downgrade(&self.state);
        let timer = cx.background_executor().timer(duration);
        let task = cx.spawn(async move |cx| {
            timer.await;
            cx.update(|cx| {
                if let Some(runner) = Self::upgrade(&weak) {
                    runner.animator_did_complete(token, cx);
                }
            });
        });
        *state.animator.borrow_mut() = Some(SharedElementFlightAnimator { completion: Some(completion), _timer: task });
        state.flight_view.update(cx, |view, cx| view.start_animation(animation, cx));
    }

    /// completion block of the animator
    fn animator_did_complete(&self, token: u64, cx: &mut App) {
        let state = &self.state;
        if token != state.animation_token.get() {
            return;
        }
        let completion = state.animator.borrow_mut().take().and_then(|mut animator| animator.completion.take());
        let alongside = state.flight_view.update(cx, |view, cx| view.finish_animation(cx));
        if let Some(alongside) = alongside {
            alongside(1.0, cx);
        }
        if !state.is_aborted.get() {
            let current_epoch = SharedElementTransitionCoordinator::shared(cx).read(cx).epoch();
            if current_epoch == state.epoch {
                if let Some(completion) = completion {
                    completion(cx);
                }
            } else {
                state.removal_requested.set(true);
            }
        }
        if state.removal_requested.get() && !state.is_aborted.get() && token == state.animation_token.get() {
            self.schedule_removal(cx);
        }
    }

    fn cancel_running_animator(&self, cx: &mut App) {
        let state = &self.state;
        if state.animator.borrow_mut().take().is_none() {
            return;
        }
        state.animation_token.set(state.animation_token.get().wrapping_add(1));
        let now = RiseClock::monotonic();
        state.flight_view.update(cx, |view, cx| view.stop_animation(now, cx));
    }

    fn schedule_removal(&self, cx: &mut App) {
        let state = &self.state;
        if state.removal_scheduled.get() {
            return;
        }
        state.removal_scheduled.set(true);
        state.is_lingering.set(false);
        let runner = self.clone();
        cx.defer(move |cx| runner.detach_view(cx));
    }

    fn remove_view_now(&self, cx: &mut App) {
        self.state.removal_scheduled.set(true);
        self.detach_view(cx);
    }

    fn detach_view(&self, cx: &mut App) {
        let state = &self.state;
        let view_id = state.flight_view.entity_id();
        state.flight_view.update(cx, |view, cx| view.remove_all_animations(cx));
        let container = state.container.borrow().clone();
        if let Some(container) = container {
            container.remove_flight_view(view_id, cx);
        }
        state.flight_view.update(cx, |view, cx| view.clear_contents(cx));
    }

    fn write_immediately(&self, endpoint: SharedElementEndpoint, cx: &mut App) {
        self.write(endpoint, cx);
    }

    fn write(&self, endpoint: SharedElementEndpoint, cx: &mut App) {
        self.state.current_endpoint.set(Some(endpoint));
        let layout = self.layout(endpoint);
        self.state.flight_view.update(cx, |view, cx| view.apply(layout, cx));
    }

    /// moves the model endpoint at once while the view animates from where it is
    fn layout_animation_to(
        &self,
        target: SharedElementEndpoint,
        cx: &App,
    ) -> (SharedElementFlightViewLayout, SharedElementFlightViewLayout) {
        self.state.current_endpoint.set(Some(target));
        let to = self.layout(target);
        let from = self.state.flight_view.read(cx).model_layout().unwrap_or(to);
        (from, to)
    }

    fn layout(&self, endpoint: SharedElementEndpoint) -> SharedElementFlightViewLayout {
        let state = &self.state;
        let clip = Self::container_rect(endpoint.clip);
        let content = Self::container_rect(endpoint.content);
        let local = Bounds { origin: point(px(0.0), px(0.0)), size: clip.size };
        let content_aspect = state.content_aspect.get();
        SharedElementFlightViewLayout {
            clip,
            corner_radius: endpoint.corner_radius,
            content_frame: Bounds {
                origin: point(content.origin.x - clip.origin.x, content.origin.y - clip.origin.y),
                size: content.size,
            },
            backdrop_frame: SharedElementGeometry::fill_rect(content_aspect, local),
            backdrop_alpha: if endpoint.mode == (SharedElementContentMode::Fit { backdrop: true }) { 1.0 } else { 0.0 },
            cover_frame: SharedElementGeometry::fill_rect(state.cover_aspect.get().unwrap_or(content_aspect), local),
        }
    }

    fn container_rect(screen_rect: Bounds<Pixels>) -> Bounds<Pixels> {
        screen_rect
    }

    fn add_opacity_keyframes(&self, cover: bool, start_progress: f32, start: Duration, cx: &mut App) {
        let (values, key_times) = SharedElementCoverSchedule::opacity_keyframes(
            SharedElementMotion::CURVE,
            start_progress,
            SharedElementMotion::COVER_KEYFRAME_SAMPLES,
        );
        let fades_in = cover;
        self.state.flight_view.update(cx, |view, cx| {
            if cover {
                view.set_cover_alpha(1.0, cx);
            } else {
                view.set_alpha(0.0, cx);
            }
        });
        if values.len() < 2 || values.len() != key_times.len() {
            return;
        }
        let mut last_time = 0.0_f32;
        let key_times: Vec<f32> = key_times
            .into_iter()
            .map(|time| {
                last_time = last_time.max(time.clamp(0.0, 1.0));
                last_time
            })
            .collect();
        let values: Vec<f32> = values
            .into_iter()
            .map(|value| {
                let clamped = value.clamp(0.0, 1.0);
                if fades_in { clamped } else { 1.0 - clamped }
            })
            .collect();
        let keyframes =
            SharedElementOpacityKeyframes { start, duration: SharedElementMotion::MOVE_DURATION, values, key_times };
        self.state.flight_view.update(cx, |view, cx| {
            if cover {
                view.add_cover_keyframes(keyframes, cx);
            } else {
                view.add_flight_keyframes(keyframes, cx);
            }
        });
    }

    fn usable(aspect: f32) -> Option<f32> {
        (aspect.is_finite() && aspect > 0.0).then_some(aspect)
    }

    fn aspect_of_content(content: &SharedElementFlightContent) -> Option<f32> {
        match content {
            SharedElementFlightContent::Bitmap(_, aspect) => Self::usable(*aspect),
            SharedElementFlightContent::Replicant(_, size) => {
                if size.height > px(0.0) { Self::usable(size.width / size.height) } else { None }
            }
        }
    }

    fn aspect_of_rect(rect: Bounds<Pixels>) -> Option<f32> {
        if rect.size.height > px(0.0) { Self::usable(rect.size.width / rect.size.height) } else { None }
    }

    /// scales every rect around the clip center so a shrinking fade never needs a transform
    fn scaled(endpoint: SharedElementEndpoint, scale: f32) -> SharedElementEndpoint {
        let pivot = endpoint.clip.center();
        let map = |rect: Bounds<Pixels>| Bounds {
            origin: point(pivot.x + (rect.origin.x - pivot.x) * scale, pivot.y + (rect.origin.y - pivot.y) * scale),
            size: size(rect.size.width * scale, rect.size.height * scale),
        };
        SharedElementEndpoint {
            clip: map(endpoint.clip),
            content: map(endpoint.content),
            corner_radius: endpoint.corner_radius * scale,
            mode: endpoint.mode,
        }
    }
}
