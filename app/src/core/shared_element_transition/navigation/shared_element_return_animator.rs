use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{App, AsyncApp, Bounds, Pixels, Point, Size, point, px};
use rise_core::foundation::foundation_date::RiseClock;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentPolicy, SharedElementEndpoint, SharedElementGeometry, SharedElementGesturePolicy,
    SharedElementGesturePolicyAxis, SharedElementGesturePolicyDecision, SharedElementGesturePolicyInput, SharedElementKey,
    SharedElementReleaseOutcome, SharedElementSessionPolicy, SharedElementSessionPolicyLandingAction,
};
use crate::core::shared_element_transition::navigation::shared_element_navigation_driver::{
    SharedElementFrameTicker, SharedElementInteractivePop, SharedElementNavigationDriver, SharedElementReturnResumable,
    SharedElementTransitionSupport, SharedElementTransitionWatchdog, SharedElementViewAnimatingState,
    SharedElementViewPropertyAnimator,
};
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::{
    SharedElementPanGesture, SharedElementPanState, SharedElementTransitionContextKey, SharedElementTransitionView,
    SharedElementViewControllerAnimatedTransitioning, SharedElementViewControllerContextTransitioning,
    SharedElementViewControllerInteractiveTransitioning,
};
use crate::core::shared_element_transition::renderer::shared_element_flight_runner::{
    SharedElementFlightAlongside, SharedElementFlightRunner,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementFlightContent, SharedElementReturnPlan, SharedElementStopFrameResult,
    SharedElementTargetResolution,
};
use crate::core::shared_element_transition::runtime::shared_element_lift_board::SharedElementLiftBoard;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::{
    SharedElementIntent, SharedElementTransitionCoordinator,
};

const LOG_NAME: &str = "SharedElement";

type TransitionContext = Rc<dyn SharedElementViewControllerContextTransitioning>;

#[derive(Clone, Copy)]
struct PanSample {
    translation: Point<Pixels>,
    velocity: Point<Pixels>,
}

impl PanSample {
    fn of(pan: &dyn SharedElementPanGesture) -> Self {
        Self {
            translation: SharedElementTransitionSupport::vector(pan.translation()),
            velocity: SharedElementTransitionSupport::vector(pan.velocity()),
        }
    }

    fn zero() -> Self {
        Self { translation: point(px(0.0), px(0.0)), velocity: point(px(0.0), px(0.0)) }
    }
}

/// list to grid, the most visible post flies back into its cell
/// animated from the back button or driven by the back pan
pub struct SharedElementReturnAnimator {
    pub epoch: u64,
    did_start: Cell<bool>,

    intent: Rc<SharedElementIntent>,
    interactive: bool,
    pan: Option<Weak<dyn SharedElementPanGesture>>,
    watchdog: SharedElementTransitionWatchdog,
    ticker: SharedElementFrameTicker,
    axis: SharedElementGesturePolicyAxis,
    this: Weak<SharedElementReturnAnimator>,

    context: RefCell<Option<TransitionContext>>,
    from_view: RefCell<Option<SharedElementTransitionView>>,
    to_view: RefCell<Option<SharedElementTransitionView>>,
    runner: RefCell<Option<SharedElementFlightRunner>>,
    chrome_animator: RefCell<Option<SharedElementViewPropertyAnimator>>,
    capture: RefCell<Option<SharedElementCapture>>,
    source: Cell<Option<SharedElementEndpoint>>,
    target: Cell<Option<SharedElementEndpoint>>,
    plan: RefCell<Option<SharedElementReturnPlan>>,
    resolved_frame: Cell<Option<Bounds<Pixels>>>,
    is_resolution_unavailable: Cell<bool>,
    applied_stop_frame: Cell<bool>,
    readiness_started_at: Cell<Duration>,
    is_waiting_for_readiness: Cell<bool>,
    is_content_gate_open: Cell<bool>,
    touch_start: Cell<Point<Pixels>>,
    layout_sign: Cell<f32>,
    last_sample: Cell<Option<PanSample>>,
    release_outcome: Cell<Option<SharedElementReleaseOutcome>>,
    flew: Cell<bool>,
    is_completed: Cell<bool>,
}

impl SharedElementReturnAnimator {
    const STOP_FRAME_IDENTITY: &str = "stopframe";

    pub fn new(
        intent: Rc<SharedElementIntent>,
        interactive: bool,
        pan: Option<Rc<dyn SharedElementPanGesture>>,
        axis: SharedElementGesturePolicyAxis,
        _cx: &mut App,
    ) -> Rc<Self> {
        let epoch = intent.epoch;
        Rc::new_cyclic(|this| Self {
            epoch,
            did_start: Cell::new(false),
            intent,
            interactive,
            pan: pan.as_ref().map(Rc::downgrade),
            watchdog: SharedElementTransitionWatchdog::new(),
            ticker: SharedElementFrameTicker::new(),
            axis,
            this: this.clone(),
            context: RefCell::new(None),
            from_view: RefCell::new(None),
            to_view: RefCell::new(None),
            runner: RefCell::new(None),
            chrome_animator: RefCell::new(None),
            capture: RefCell::new(None),
            source: Cell::new(None),
            target: Cell::new(None),
            plan: RefCell::new(None),
            resolved_frame: Cell::new(None),
            is_resolution_unavailable: Cell::new(false),
            applied_stop_frame: Cell::new(false),
            readiness_started_at: Cell::new(Duration::ZERO),
            is_waiting_for_readiness: Cell::new(false),
            is_content_gate_open: Cell::new(false),
            touch_start: Cell::new(point(px(0.0), px(0.0))),
            layout_sign: Cell::new(1.0),
            last_sample: Cell::new(None),
            release_outcome: Cell::new(None),
            flew: Cell::new(false),
            is_completed: Cell::new(false),
        })
    }

    /// runs work only if self is still alive
    fn weak(&self, work: impl Fn(&Rc<Self>, &mut App) + 'static) -> Box<dyn Fn(&mut App)> {
        let this = self.this.clone();
        Box::new(move |cx| {
            if let Some(this) = this.upgrade() {
                work(&this, cx);
            }
        })
    }

    fn after(&self, delay: f64, cx: &mut App, work: impl Fn(&Rc<Self>, &mut App) + 'static) {
        SharedElementTransitionSupport::after(delay, self.epoch, self.weak(work), cx);
    }

    fn context(&self) -> Option<TransitionContext> {
        self.context.borrow().clone()
    }

    fn pan(&self) -> Option<Rc<dyn SharedElementPanGesture>> {
        self.pan.as_ref().and_then(Weak::upgrade)
    }

    fn from_view(&self) -> Option<SharedElementTransitionView> {
        self.from_view.borrow().clone()
    }

    fn to_view(&self) -> Option<SharedElementTransitionView> {
        self.to_view.borrow().clone()
    }

    fn set_from_view_alpha(&self, alpha: f32, cx: &mut App) {
        if let Some(from_view) = self.from_view() {
            from_view.set_alpha(alpha, cx);
        }
    }

    fn runner(&self) -> Option<SharedElementFlightRunner> {
        self.runner.borrow().clone()
    }

    fn begun_runner(&self) -> Option<SharedElementFlightRunner> {
        self.runner().filter(SharedElementFlightRunner::has_begun)
    }

    fn capture(&self) -> Option<SharedElementCapture> {
        self.capture.borrow().clone()
    }

    fn has_plan(&self) -> bool {
        self.plan.borrow().is_some()
    }

    fn coordinator_epoch(cx: &mut App) -> u64 {
        SharedElementTransitionCoordinator::shared(cx).read(cx).epoch()
    }

    fn transition_did_end(&self, cx: &mut App) {
        let epoch = self.epoch;
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.transition_did_end(epoch, cx));
    }

    fn release_inline_playback(&self, cx: &mut App) {
        let epoch = self.epoch;
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.release_inline_playback(epoch, cx));
    }

    fn land_lift_board(&self, cx: &mut App) {
        let epoch = self.epoch;
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.land(epoch, cx));
    }

    fn grid_keys(&self) -> HashSet<SharedElementKey> {
        SharedElementTransitionSupport::grid_keys(self.intent.origin().as_ref(), &self.intent.item_id)
    }

    /// sets the from view alpha alongside the flight, from where the list stands
    fn from_view_alpha_alongside(&self, to: f32, cx: &App) -> Option<SharedElementFlightAlongside> {
        let from_view = self.from_view()?;
        let from = from_view.alpha(cx);
        let alongside: SharedElementFlightAlongside = Rc::new(move |progress: f32, cx: &mut App| {
            from_view.set_alpha(SharedElementGeometry::lerp(from, to, progress), cx);
        });
        Some(alongside)
    }

    fn fade_from_view(&self, to: f32, duration: f64, cx: &mut App, completion: impl Fn(&Rc<Self>, &mut App) + 'static) {
        let animator =
            SharedElementTransitionSupport::fade(self.from_view(), to, duration, self.epoch, self.weak(completion), cx);
        *self.chrome_animator.borrow_mut() = Some(animator);
    }

    fn animate(&self, transition_context: TransitionContext, cx: &mut App) {
        if !self.begin(transition_context, cx) {
            return;
        }
        self.watchdog.arm(self.weak(|this, cx| this.force_end(cx)), cx);
        if !self.has_plan() {
            let _ = self.settle_content(true, cx);
            self.fall_back(cx);
            return;
        }
        self.readiness_started_at.set(RiseClock::monotonic());
        self.is_waiting_for_readiness.set(true);
        self.after(0.0, cx, |this, cx| {
            if !this.is_waiting_for_readiness.get() || this.is_completed.get() {
                return;
            }
            if let Some(to_view) = this.to_view() {
                to_view.layout_if_needed(cx);
            }
            this.check_readiness(cx);
            if !this.is_waiting_for_readiness.get() {
                return;
            }
            this.ticker.start(this.weak(|this, cx| this.check_readiness(cx)), cx);
        });
    }

    /// ready when target and content are both ready or the resolve timeout passed
    fn check_readiness(&self, cx: &mut App) {
        if !self.is_waiting_for_readiness.get() || self.is_completed.get() {
            return;
        }
        if Self::coordinator_epoch(cx) != self.epoch {
            self.stop_waiting();
            return;
        }
        self.resolve_target(cx);
        if self.is_resolution_unavailable.get() {
            self.stop_waiting();
            let _ = self.settle_content(true, cx);
            self.fall_back(cx);
            return;
        }
        let content_ready = self.settle_content(false, cx);
        if let Some(resolved_frame) = self.resolved_frame.get()
            && content_ready
        {
            self.stop_waiting();
            self.fly(resolved_frame, cx);
            return;
        }
        let waited = RiseClock::monotonic().saturating_sub(self.readiness_started_at.get()).as_secs_f64();
        if waited >= SharedElementMotion::EXIT_RESOLVE_TIMEOUT {
            self.stop_waiting();
            let _ = self.settle_content(true, cx);
            self.fall_back(cx);
        }
    }

    fn fly(&self, frame: Bounds<Pixels>, cx: &mut App) {
        self.begin_flight(true, cx);
        let (Some(runner), Some(capture), Some(source)) = (self.begun_runner(), self.capture(), self.source.get()) else {
            self.fall_back(cx);
            return;
        };
        let target = SharedElementTransitionSupport::return_landing(&capture, &source, frame, 0.0);
        self.target.set(Some(target));
        self.flew.set(true);
        runner.animate(
            target,
            Some(SharedElementMotion::COVER_FADE_START_PROGRESS),
            self.from_view_alpha_alongside(0.0, cx),
            Box::new(self.weak(|this, cx| this.move_did_finish(cx))),
            cx,
        );
        self.schedule_retarget(cx);
    }

    fn start_interactive(&self, transition_context: TransitionContext, cx: &mut App) {
        self.did_start.set(true);
        if !self.begin(transition_context, cx) {
            return;
        }
        self.after(0.0, cx, |this, cx| {
            if this.is_completed.get() || this.release_outcome.get().is_some() {
                return;
            }
            if let Some(to_view) = this.to_view() {
                to_view.layout_if_needed(cx);
            }
            this.resolve_target(cx);
        });

        self.layout_sign.set(SharedElementTransitionSupport::layout_sign(cx));
        if let Some(pan) = self.pan() {
            self.touch_start.set(SharedElementTransitionSupport::screen_point(pan.location()));
            self.last_sample.set(Some(PanSample::of(pan.as_ref())));
        } else if let Some(source) = self.source.get() {
            self.touch_start.set(source.clip.center());
        }

        if self.settle_content(false, cx) {
            self.open_content_gate(cx);
        } else if let Some(request) = self.capture.borrow().as_ref().and_then(|capture| capture.stop_frame.clone()) {
            let (settled, on_settled) = oneshot::channel::<()>();
            request.on_settle(move |_: Option<SharedElementStopFrameResult>| {
                let _ = settled.send(());
            });
            let this = self.this.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                if on_settled.await.is_err() {
                    return;
                }
                cx.update(|cx| {
                    if let Some(this) = this.upgrade() {
                        this.open_content_gate_if_ready(false, cx);
                    }
                });
            })
            .detach();
            let elapsed = RiseClock::monotonic().saturating_sub(request.created_at).as_secs_f64();
            let remaining = SharedElementMotion::STOP_FRAME_BUDGET - elapsed;
            self.after(remaining, cx, |this, cx| this.open_content_gate_if_ready(true, cx));
        }
        self.ticker.start(self.weak(|this, cx| this.check_pan_liveness(cx)), cx);
        self.check_pan_liveness(cx);
    }

    fn check_pan_liveness(&self, cx: &mut App) {
        if self.is_completed.get() || self.release_outcome.get().is_some() {
            self.ticker.stop();
            return;
        }
        let state = self.pan().map(|pan| pan.state());
        if matches!(state, Some(SharedElementPanState::Began | SharedElementPanState::Changed)) {
            return;
        }
        self.ticker.stop();
        let sample = self.last_sample.get().unwrap_or_else(PanSample::zero);
        self.release(sample, state != Some(SharedElementPanState::Ended), cx);
    }

    fn open_content_gate_if_ready(&self, force_budget: bool, cx: &mut App) {
        if !self.did_start.get()
            || self.is_completed.get()
            || self.is_content_gate_open.get()
            || self.release_outcome.get().is_some()
            || Self::coordinator_epoch(cx) != self.epoch
            || !self.settle_content(force_budget, cx)
        {
            return;
        }
        self.open_content_gate(cx);
    }

    fn open_content_gate(&self, cx: &mut App) {
        if self.is_content_gate_open.get() {
            return;
        }
        self.is_content_gate_open.set(true);
        self.begin_flight(self.has_plan(), cx);
        if let Some(last_sample) = self.last_sample.get() {
            self.apply(last_sample, cx);
        }
    }

    fn apply(&self, sample: PanSample, cx: &mut App) {
        let (Some(context), Some(source)) = (self.context(), self.source.get()) else { return };
        let viewport = context.container_bounds(cx).size;
        let frame = SharedElementGesturePolicy::sample(&self.input(sample, source, viewport));
        if let Some(runner) = self.begun_runner() {
            runner.follow(frame, source, cx);
        }
        self.set_from_view_alpha(frame.chrome_alpha, cx);
        context.update_interactive_transition(frame.progress, cx);
    }

    fn release(&self, sample: PanSample, force_cancel: bool, cx: &mut App) {
        let Some(context) = self.context() else { return };
        let epoch = self.epoch;
        let gate = self.is_content_gate_open.get();
        let plan = self.has_plan();
        let resolved = self.resolved_frame.get().is_some();
        Sai::log(
            LOG_NAME,
            move || {
                format!(
                    "return release epoch={epoch} forceCancel={force_cancel} gate={gate} plan={plan} resolved={resolved}"
                )
            },
            SaiLogKind::Ui,
            cx,
        );
        self.watchdog.arm(self.weak(|this, cx| this.force_end(cx)), cx);
        if !self.is_content_gate_open.get() {
            self.open_content_gate_if_ready(true, cx);
        }
        let Some(source) = self.source.get() else {
            self.release_outcome.set(Some(SharedElementReleaseOutcome::Cancel));
            context.cancel_interactive_transition(cx);
            self.cancel_did_finish(cx);
            return;
        };
        let viewport = context.container_bounds(cx).size;
        let decision = if force_cancel {
            SharedElementGesturePolicyDecision::Cancel
        } else {
            SharedElementGesturePolicy::decision(&self.input(sample, source, viewport))
        };
        match decision {
            SharedElementGesturePolicyDecision::Finish => {
                self.release_outcome.set(Some(SharedElementReleaseOutcome::Finish));
                self.finish(sample, &context, cx);
            }
            SharedElementGesturePolicyDecision::Cancel => {
                self.release_outcome.set(Some(SharedElementReleaseOutcome::Cancel));
                self.cancel(sample, &context, source, cx);
            }
        }
    }

    fn finish(&self, sample: PanSample, context: &TransitionContext, cx: &mut App) {
        context.finish_interactive_transition(cx);
        self.resolve_target(cx);
        let frame = self.resolved_frame.get().or_else(|| self.plan.borrow().as_ref().map(|plan| plan.predicted_frame));
        let (Some(frame), Some(runner), Some(capture), Some(source)) =
            (frame, self.begun_runner(), self.capture(), self.source.get())
        else {
            self.fall_back(cx);
            return;
        };
        let target = SharedElementTransitionSupport::return_landing(&capture, &source, frame, 0.0);
        self.target.set(Some(target));
        self.flew.set(true);
        runner.release(
            SharedElementReleaseOutcome::Finish,
            sample.velocity,
            target,
            self.from_view_alpha_alongside(0.0, cx),
            Box::new(self.weak(|this, cx| this.move_did_finish(cx))),
            cx,
        );
        self.schedule_retarget(cx);
    }

    fn cancel(
        &self,
        sample: PanSample,
        context: &TransitionContext,
        source: SharedElementEndpoint,
        cx: &mut App,
    ) {
        context.cancel_interactive_transition(cx);
        if let Some(runner) = self.begun_runner() {
            runner.release(
                SharedElementReleaseOutcome::Cancel,
                sample.velocity,
                source,
                self.from_view_alpha_alongside(1.0, cx),
                Box::new(self.weak(|this, cx| this.cancel_did_finish(cx))),
                cx,
            );
        } else {
            self.fade_from_view(1.0, SharedElementMotion::CANCEL_DURATION, cx, |this, cx| this.cancel_did_finish(cx));
        }
    }

    /// order matters, the profile leaves the window before the scroller restores its position
    /// so the unbind that removal triggers cant overwrite it
    fn cancel_did_finish(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        let Some(context) = self.context() else { return };
        self.land_lift_board(cx);
        self.release_inline_playback(cx);
        if let Some(to_view) = self.to_view() {
            to_view.remove_from_superview(cx);
        }
        if self.has_plan()
            && let Some(origin) = self.intent.origin()
        {
            origin.cancel_return(self.epoch, cx);
        }
        self.set_from_view_alpha(1.0, cx);
        self.set_interaction(true, cx);
        self.complete(&context, cx);
        if let Some(runner) = self.runner() {
            runner.remove_after_commit(cx);
        }
        self.transition_did_end(cx);
    }

    /// validates the intent then seeds the profile while
    /// its still off window and inserts it below the list
    /// false when the context was already completed
    fn begin(&self, transition_context: TransitionContext, cx: &mut App) -> bool {
        let (Some(from_view), Some(to_view)) = (
            SharedElementTransitionView::of(&transition_context, SharedElementTransitionContextKey::From),
            SharedElementTransitionView::of(&transition_context, SharedElementTransitionContextKey::To),
        ) else {
            self.end_without_flight(transition_context, cx);
            return false;
        };
        let capture = self.intent.capture();
        let (true, Some(capture)) = (SharedElementTransitionSupport::is_live(&self.intent, cx), capture) else {
            if !self.interactive {
                transition_context.apply_final_frame(SharedElementTransitionContextKey::To, cx);
                transition_context.insert_subview_below(
                    SharedElementTransitionContextKey::To,
                    SharedElementTransitionContextKey::From,
                    cx,
                );
            }
            self.end_without_flight(transition_context, cx);
            return false;
        };

        *self.context.borrow_mut() = Some(transition_context.clone());
        *self.from_view.borrow_mut() = Some(from_view);
        *self.to_view.borrow_mut() = Some(to_view);
        self.source.set(Some(SharedElementTransitionSupport::source(&capture)));
        *self.capture.borrow_mut() = Some(capture);
        let epoch = self.epoch;
        if let Some(this) = self.this.upgrade() {
            let resumable: Rc<dyn SharedElementReturnResumable> = this.clone();
            SharedElementNavigationDriver::shared().transition_did_begin(epoch, this, Some(resumable));
        }
        let on_resign_active = self.weak(|this, cx| this.application_will_resign_active(cx));
        let on_force_end = self.weak(|this, cx| this.force_end(cx));
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, _| {
            coordinator.attach_transition_hooks(epoch, Some(on_resign_active), on_force_end);
        });
        self.set_interaction(false, cx);

        let plan = self.intent.origin().and_then(|origin| origin.prepare_return(&self.intent.item_id, epoch, cx));
        *self.plan.borrow_mut() = plan;
        transition_context.insert_subview_below(
            SharedElementTransitionContextKey::To,
            SharedElementTransitionContextKey::From,
            cx,
        );
        transition_context.apply_final_frame(SharedElementTransitionContextKey::To, cx);
        true
    }

    fn resolve_target(&self, cx: &mut App) {
        if self.resolved_frame.get().is_some() || self.is_resolution_unavailable.get() {
            return;
        }
        let origin = self.intent.origin();
        let (true, Some(origin)) = (self.has_plan(), origin) else {
            self.is_resolution_unavailable.set(true);
            return;
        };
        let resolution = origin.resolve_return(self.epoch, cx);
        match resolution {
            SharedElementTargetResolution::Ready(frame) => self.resolved_frame.set(Some(frame)),
            SharedElementTargetResolution::Deferred => {}
            SharedElementTargetResolution::Unavailable => self.is_resolution_unavailable.set(true),
        }
    }

    /// true once the flying content is final, no stop frame or a
    /// delivered one or a missed or expired one that keeps the poster
    fn settle_content(&self, force_budget: bool, cx: &mut App) -> bool {
        let Some(request) = self.capture.borrow().as_ref().and_then(|capture| capture.stop_frame.clone()) else {
            return true;
        };
        if let Some(result) = request.result() {
            self.apply_stop_frame(result);
            return true;
        }
        if !request.is_pending() {
            return true;
        }
        if force_budget || request.is_expired(RiseClock::monotonic(), SharedElementMotion::STOP_FRAME_BUDGET) {
            request.cancel();
            return true;
        }
        false
    }

    fn apply_stop_frame(&self, result: SharedElementStopFrameResult) {
        if self.applied_stop_frame.get() || self.runner.borrow().is_some() {
            return;
        }
        let Some(mut updated) = self.capture() else { return };
        self.applied_stop_frame.set(true);
        updated.content = SharedElementFlightContent::Bitmap(result.image.clone(), result.aspect);
        updated.content_rect = result.content_rect;
        self.source.set(Some(SharedElementTransitionSupport::source(&updated)));
        self.intent.set_capture(Some(updated.clone()));
        *self.capture.borrow_mut() = Some(updated);
    }

    /// what actually flies, for the cover decision
    /// a capture that promised a stop frame but flies its poster has no known identity
    fn flying_identity(&self) -> Option<String> {
        if self.applied_stop_frame.get() {
            return Some(Self::STOP_FRAME_IDENTITY.to_owned());
        }
        let capture = self.capture.borrow();
        let capture = capture.as_ref()?;
        if capture.stop_frame.is_some() && capture.flying_identity.as_deref() == Some(Self::STOP_FRAME_IDENTITY) {
            return None;
        }
        capture.flying_identity.clone()
    }

    fn begin_flight(&self, lifts_grid: bool, cx: &mut App) {
        if self.runner.borrow().is_some() {
            return;
        }
        let (Some(context), Some(capture), Some(source)) = (self.context(), self.capture(), self.source.get()) else {
            return;
        };
        let epoch = self.epoch;
        let runner = SharedElementFlightRunner::new(context.container_view(cx), epoch, cx);
        *self.runner.borrow_mut() = Some(runner.clone());
        if let Some(placeholder) = capture.placeholder.clone() {
            runner.flight_view().update(cx, |flight_view, cx| flight_view.set_placeholder(placeholder, cx));
        }
        runner.begin(capture.content.clone(), source, capture.backdrop.clone(), cx);
        let mut keys = HashSet::from([capture.source_key.clone()]);
        if let Some(destination) = self.intent.destination() {
            keys.extend(destination.lift_keys(&capture.item_id, capture.part, cx));
        }
        if lifts_grid {
            if let Some(cover) = self.intent.cover.clone()
                && SharedElementContentPolicy::needs_cover(self.flying_identity().as_deref(), Some(cover.identity.as_str()))
            {
                runner.set_cover(Some(cover), cx);
            }
            keys.extend(self.grid_keys());
        }
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.lift(keys, epoch, cx));
        if capture.stop_frame.is_some() {
            SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.hold_inline_playback(epoch, cx));
        }
        self.watch_live_video(cx);
    }

    fn watch_live_video(&self, cx: &mut App) {
        let Some(readiness) = self.capture.borrow().as_ref().and_then(|capture| capture.live_readiness.clone()) else {
            return;
        };
        if !readiness.is_pending() {
            return;
        }
        let (settled, on_settled) = oneshot::channel::<bool>();
        readiness.on_settle(move |ready: bool| {
            let _ = settled.send(ready);
        });
        let this = self.this.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let Ok(ready) = on_settled.await else { return };
            if ready {
                return;
            }
            cx.update(|cx| {
                if let Some(this) = this.upgrade() {
                    this.reveal_placeholder(cx);
                }
            });
        })
        .detach();
        SharedElementTransitionSupport::after(
            SharedElementMotion::LIVE_VIDEO_READY_BUDGET,
            self.epoch,
            Box::new(move |_: &mut App| readiness.cancel()),
            cx,
        );
    }

    fn reveal_placeholder(&self, cx: &mut App) {
        if self.is_completed.get() || Self::coordinator_epoch(cx) != self.epoch {
            return;
        }
        let (Some(runner), Some(placeholder)) =
            (self.begun_runner(), self.capture.borrow().as_ref().and_then(|capture| capture.placeholder.clone()))
        else {
            return;
        };
        runner.reveal_placeholder(placeholder, cx);
    }

    fn schedule_retarget(&self, cx: &mut App) {
        self.after(SharedElementMotion::RETARGET_CHECK_DELAY, cx, |this, cx| this.retarget_to_measured_frame(cx));
    }

    fn retarget_to_measured_frame(&self, cx: &mut App) {
        if self.is_completed.get() || !self.flew.get() {
            return;
        }
        let (Some(runner), Some(capture), Some(source), Some(target), Some(origin)) =
            (self.runner(), self.capture(), self.source.get(), self.target.get(), self.intent.origin())
        else {
            return;
        };
        let Some(measured) = origin.measured_target_frame(&self.intent.item_id, cx) else { return };
        if SharedElementGeometry::max_edge_delta(measured, target.clip) <= SharedElementMotion::LANDING_MISMATCH {
            return;
        }
        let updated = SharedElementTransitionSupport::return_landing(&capture, &source, measured, 0.0);
        self.target.set(Some(updated));
        runner.retarget(updated, cx);
    }

    /// in transition fallback, the flight fades and shrinks
    /// while the list fades and reveals the grid wherever it is
    fn fall_back(&self, cx: &mut App) {
        self.begin_flight(false, cx);
        let keys = self.grid_keys();
        let epoch = self.epoch;
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| {
            for key in &keys {
                board.unlift(key, epoch, cx);
            }
        });
        self.flew.set(false);
        if let Some(runner) = self.begun_runner() {
            runner.fade_out(
                SharedElementMotion::EXIT_FALLBACK_FADE,
                SharedElementMotion::EXIT_FALLBACK_SCALE,
                self.from_view_alpha_alongside(0.0, cx),
                Box::new(self.weak(|this, cx| this.move_did_finish(cx))),
                cx,
            );
        } else {
            self.fade_from_view(0.0, SharedElementMotion::EXIT_FALLBACK_FADE, cx, |this, cx| this.move_did_finish(cx));
        }
    }

    fn move_did_finish(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        let Some(context) = self.context() else { return };
        self.stop_waiting();
        let epoch = self.epoch;
        if self.flew.get()
            && let (Some(target), Some(runner)) = (self.target.get(), self.runner())
        {
            let measured = self.intent.origin().and_then(|origin| origin.measured_target_frame(&self.intent.item_id, cx));
            let mismatch = measured.map_or(f32::INFINITY, |measured| SharedElementGeometry::max_edge_delta(measured, target.clip));
            match SharedElementSessionPolicy::landing(mismatch, SharedElementMotion::LANDING_MISMATCH) {
                SharedElementSessionPolicyLandingAction::Land => self.land_lift_board(cx),
                SharedElementSessionPolicyLandingAction::CrossfadeReveal => {
                    let keys = self.grid_keys();
                    SharedElementLiftBoard::shared(cx).update(cx, |board, cx| {
                        for key in &keys {
                            board.unlift(key, epoch, cx);
                        }
                    });
                    runner.fade_out(SharedElementMotion::LANDING_CROSSFADE, 1.0, None, Box::new(|_: &mut App| {}), cx);
                }
            }
        } else {
            self.land_lift_board(cx);
        }
        if self.has_plan()
            && let Some(origin) = self.intent.origin()
        {
            origin.finish_return(epoch, cx);
        }
        self.set_from_view_alpha(1.0, cx);
        self.set_interaction(true, cx);
        self.complete(&context, cx);
        if let Some(runner) = self.runner() {
            runner.remove_after_commit(cx);
        }
        self.release_inline_playback(cx);
        self.transition_did_end(cx);
    }

    fn application_will_resign_active(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        if self.is_waiting_for_readiness.get() {
            self.stop_waiting();
            let _ = self.settle_content(true, cx);
            self.fall_back(cx);
        }
        if self.interactive && self.release_outcome.get().is_none() {
            return;
        }
        if let Some(runner) = self.begun_runner() {
            runner.jump_to_end(cx);
        }
        let chrome_animator = self.chrome_animator.borrow().clone();
        if let Some(chrome_animator) = chrome_animator
            && chrome_animator.state() == SharedElementViewAnimatingState::Active
        {
            chrome_animator.stop_animation(false);
            chrome_animator.finish_animation_at_end(cx);
        }
    }

    /// watchdog and account reset
    /// after a reset the epoch moved on so the scroller is left alone
    /// it goes away with the re-identified stack
    fn force_end(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        let Some(context) = self.context() else { return };
        self.stop_waiting();
        if let Some(request) = self.capture.borrow().as_ref().and_then(|capture| capture.stop_frame.clone()) {
            request.cancel();
        }
        let chrome_animator = self.chrome_animator.borrow_mut().take();
        if let Some(chrome_animator) = chrome_animator {
            chrome_animator.stop_animation(true);
        }
        if let Some(runner) = self.runner() {
            runner.abort(cx);
        }
        self.land_lift_board(cx);
        self.release_inline_playback(cx);

        let owns_scroller = Self::coordinator_epoch(cx) == self.epoch && self.has_plan();
        let cancelled = if self.interactive && self.release_outcome.get().is_none() {
            context.cancel_interactive_transition(cx);
            true
        } else {
            context.transition_was_cancelled(cx)
        };
        if cancelled {
            if let Some(to_view) = self.to_view() {
                to_view.remove_from_superview(cx);
            }
            if owns_scroller && let Some(origin) = self.intent.origin() {
                origin.cancel_return(self.epoch, cx);
            }
        } else if owns_scroller && let Some(origin) = self.intent.origin() {
            origin.finish_return(self.epoch, cx);
        }
        self.set_from_view_alpha(1.0, cx);
        self.set_interaction(true, cx);
        self.complete(&context, cx);
        self.transition_did_end(cx);
    }

    fn end_without_flight(&self, transition_context: TransitionContext, cx: &mut App) {
        self.is_completed.set(true);
        if self.interactive {
            cx.defer(move |cx| {
                transition_context.cancel_interactive_transition(cx);
                transition_context.complete_transition(false, cx);
            });
        } else {
            transition_context.complete_transition(!transition_context.transition_was_cancelled(cx), cx);
        }
        let epoch = self.epoch;
        SharedElementNavigationDriver::shared().transition_did_finish(epoch);
        self.transition_did_end(cx);
    }

    fn complete(&self, context: &TransitionContext, cx: &mut App) {
        self.is_completed.set(true);
        self.stop_waiting();
        self.watchdog.cancel();
        let epoch = self.epoch;
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, _| coordinator.detach_transition_hooks(epoch));
        context.complete_transition(!context.transition_was_cancelled(cx), cx);
        *self.context.borrow_mut() = None;
        SharedElementNavigationDriver::shared().transition_did_finish(epoch);
    }

    fn input(&self, sample: PanSample, source: SharedElementEndpoint, viewport: Size<Pixels>) -> SharedElementGesturePolicyInput {
        SharedElementGesturePolicyInput {
            translation: sample.translation,
            velocity: sample.velocity,
            touch_start: self.touch_start.get(),
            source: source.clip,
            viewport,
            layout_sign: self.layout_sign.get(),
            axis: self.axis,
        }
    }

    fn stop_waiting(&self) {
        self.is_waiting_for_readiness.set(false);
        self.ticker.stop();
    }

    fn set_interaction(&self, enabled: bool, cx: &mut App) {
        for view in [self.from_view(), self.to_view()].into_iter().flatten() {
            view.set_user_interaction_enabled(enabled, cx);
        }
    }
}

impl SharedElementViewControllerAnimatedTransitioning for SharedElementReturnAnimator {
    fn transition_duration(&self, _: Option<&TransitionContext>) -> f64 {
        SharedElementMotion::MOVE_DURATION
    }

    fn animate_transition(&self, transition_context: TransitionContext, cx: &mut App) {
        self.animate(transition_context, cx);
    }
}

impl SharedElementViewControllerInteractiveTransitioning for SharedElementReturnAnimator {
    fn wants_interactive_start(&self) -> bool {
        self.interactive
    }

    fn start_interactive_transition(&self, transition_context: TransitionContext, cx: &mut App) {
        self.start_interactive(transition_context, cx);
    }
}

impl SharedElementInteractivePop for SharedElementReturnAnimator {
    fn epoch(&self) -> u64 {
        self.epoch
    }

    fn did_start(&self) -> bool {
        self.did_start.get()
    }

    fn handle_pan(&self, pan: &Rc<dyn SharedElementPanGesture>, cx: &mut App) {
        if !self.did_start.get() || self.is_completed.get() || self.release_outcome.get().is_some() || self.context().is_none() {
            return;
        }
        let sample = PanSample::of(pan.as_ref());
        self.last_sample.set(Some(sample));
        match pan.state() {
            SharedElementPanState::Changed => {
                if self.is_content_gate_open.get() {
                    self.apply(sample, cx);
                }
            }
            SharedElementPanState::Ended => self.release(sample, false, cx),
            SharedElementPanState::Cancelled | SharedElementPanState::Failed => self.release(sample, true, cx),
            SharedElementPanState::Possible | SharedElementPanState::Began => {}
        }
    }

    fn as_animated(self: Rc<Self>) -> Rc<dyn SharedElementViewControllerAnimatedTransitioning> {
        self
    }

    fn as_interactive(self: Rc<Self>) -> Rc<dyn SharedElementViewControllerInteractiveTransitioning> {
        self
    }
}

impl SharedElementReturnResumable for SharedElementReturnAnimator {
    fn return_did_become_resumable(&self, epoch: u64, cx: &mut App) {
        if epoch != self.epoch || self.is_completed.get() {
            return;
        }
        if self.is_waiting_for_readiness.get() {
            self.check_readiness(cx);
        } else if self.interactive && self.release_outcome.get() != Some(SharedElementReleaseOutcome::Cancel) {
            let had_frame = self.resolved_frame.get().is_some();
            self.resolve_target(cx);
            if !had_frame
                && self.flew.get()
                && let (Some(frame), Some(runner), Some(capture), Some(source)) =
                    (self.resolved_frame.get(), self.runner(), self.capture(), self.source.get())
            {
                let updated = SharedElementTransitionSupport::return_landing(&capture, &source, frame, 0.0);
                self.target.set(Some(updated));
                runner.retarget(updated, cx);
            }
        }
    }
}
