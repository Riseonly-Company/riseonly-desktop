use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{App, Bounds, Entity, Pixels, Point, Size};
use rise_core::foundation::foundation_date::RiseClock;

use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementDirection, SharedElementGesturePolicy, SharedElementGesturePolicyAxis,
    SharedElementGesturePolicyDecision, SharedElementGesturePolicyInput, SharedElementReleaseOutcome,
};
use crate::core::shared_element_transition::navigation::shared_element_navigation_driver::{
    SharedElementFrameTicker, SharedElementInteractivePop, SharedElementNavigationDriver,
    SharedElementReturnResumable, SharedElementTransitionSupport, SharedElementTransitionWatchdog,
    SharedElementViewAnimatingState, SharedElementViewPropertyAnimator,
};
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::{
    SharedElementPanGesture, SharedElementPanState, SharedElementTransitionContextKey, SharedElementTransitionView,
    SharedElementViewControllerAnimatedTransitioning, SharedElementViewControllerContextTransitioning,
    SharedElementViewControllerInteractiveTransitioning,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementReturnPlan, SharedElementTargetResolution,
};
use crate::core::shared_element_transition::runtime::shared_element_lift_board::SharedElementLiftBoard;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::{
    SharedElementIntent, SharedElementTransitionCoordinator,
};

type TransitionContext = Rc<dyn SharedElementViewControllerContextTransitioning>;

/// reduce motion or an open without a capture
/// plain alpha crossfades with the same return positioning
/// and cancel ordering and safety exits as a flight
pub struct SharedElementCrossfadeAnimator {
    pub epoch: u64,
    did_start: Cell<bool>,

    this: Weak<Self>,
    intent: Rc<SharedElementIntent>,
    interactive: bool,
    pan: Option<Weak<dyn SharedElementPanGesture>>,
    watchdog: SharedElementTransitionWatchdog,
    ticker: SharedElementFrameTicker,

    context: RefCell<Option<TransitionContext>>,
    from_view: RefCell<Option<SharedElementTransitionView>>,
    to_view: RefCell<Option<SharedElementTransitionView>>,
    plan: RefCell<Option<SharedElementReturnPlan>>,
    is_resolved: Cell<bool>,
    readiness_started_at: Cell<Duration>,
    is_waiting_for_readiness: Cell<bool>,
    touch_start: Cell<Point<Pixels>>,
    layout_sign: Cell<f32>,
    release_outcome: Cell<Option<SharedElementReleaseOutcome>>,
    fade_animator: RefCell<Option<SharedElementViewPropertyAnimator>>,
    is_completed: Cell<bool>,

    axis: SharedElementGesturePolicyAxis,
}

impl SharedElementCrossfadeAnimator {
    pub fn new(
        intent: Rc<SharedElementIntent>,
        interactive: bool,
        pan: Option<Rc<dyn SharedElementPanGesture>>,
        axis: SharedElementGesturePolicyAxis,
    ) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            epoch: intent.epoch,
            did_start: Cell::new(false),
            this: this.clone(),
            intent,
            interactive,
            pan: pan.as_ref().map(Rc::downgrade),
            watchdog: SharedElementTransitionWatchdog::new(),
            ticker: SharedElementFrameTicker::new(),
            context: RefCell::new(None),
            from_view: RefCell::new(None),
            to_view: RefCell::new(None),
            plan: RefCell::new(None),
            is_resolved: Cell::new(false),
            readiness_started_at: Cell::new(Duration::ZERO),
            is_waiting_for_readiness: Cell::new(false),
            touch_start: Cell::new(Point::default()),
            layout_sign: Cell::new(1.0),
            release_outcome: Cell::new(None),
            fade_animator: RefCell::new(None),
            is_completed: Cell::new(false),
            axis,
        })
    }

    fn coordinator(cx: &mut App) -> Entity<SharedElementTransitionCoordinator> {
        SharedElementTransitionCoordinator::shared(cx)
    }

    fn pan(&self) -> Option<Rc<dyn SharedElementPanGesture>> {
        self.pan.as_ref().and_then(Weak::upgrade)
    }

    fn context(&self) -> Option<TransitionContext> {
        self.context.borrow().clone()
    }

    fn from_view(&self) -> Option<SharedElementTransitionView> {
        self.from_view.borrow().clone()
    }

    fn to_view(&self) -> Option<SharedElementTransitionView> {
        self.to_view.borrow().clone()
    }

    /// runs work only if self is still alive
    fn weak(&self, work: impl FnOnce(&Rc<Self>, &mut App) + 'static) -> impl FnOnce(&mut App) + 'static {
        let this = self.this.clone();
        move |cx: &mut App| {
            if let Some(this) = this.upgrade() {
                work(&this, cx);
            }
        }
    }

    fn weak_tick(&self, work: impl Fn(&Rc<Self>, &mut App) + 'static) -> impl FnMut(&mut App) + 'static {
        let this = self.this.clone();
        move |cx: &mut App| {
            if let Some(this) = this.upgrade() {
                work(&this, cx);
            }
        }
    }

    fn arm_watchdog(&self, cx: &mut App) {
        self.watchdog.arm(self.weak(|this, cx| this.force_end(cx)), cx);
    }

    fn fade(
        &self,
        view: Option<SharedElementTransitionView>,
        to: f32,
        cx: &mut App,
        completion: impl FnOnce(&Rc<Self>, &mut App) + 'static,
    ) {
        let animator = SharedElementTransitionSupport::fade(
            view,
            to,
            SharedElementMotion::REDUCE_MOTION_FADE,
            self.epoch,
            self.weak(completion),
            cx,
        );
        *self.fade_animator.borrow_mut() = Some(animator);
    }

    fn check_pan_liveness(&self, cx: &mut App) {
        if self.is_completed.get() || self.release_outcome.get().is_some() {
            self.ticker.stop();
            return;
        }
        let Some(context) = self.context() else {
            self.ticker.stop();
            return;
        };
        let state = self.pan().map(|pan| pan.state());
        if matches!(state, Some(SharedElementPanState::Began) | Some(SharedElementPanState::Changed)) {
            return;
        }
        self.ticker.stop();
        self.release(SharedElementReleaseOutcome::Cancel, &context, cx);
    }

    fn run_open(&self, transition_context: TransitionContext, cx: &mut App) {
        let (Some(from_view), Some(to_view)) = (
            SharedElementTransitionView::of(&transition_context, SharedElementTransitionContextKey::From),
            SharedElementTransitionView::of(&transition_context, SharedElementTransitionContextKey::To),
        ) else {
            self.end_without_transition(&transition_context, cx);
            return;
        };
        transition_context.apply_final_frame(SharedElementTransitionContextKey::To, cx);
        if !SharedElementTransitionSupport::is_live(&self.intent, cx) {
            transition_context.add_subview(SharedElementTransitionContextKey::To, cx);
            self.end_without_transition(&transition_context, cx);
            return;
        }
        self.start(&transition_context, from_view, to_view.clone(), cx);
        let epoch = self.epoch;
        Self::coordinator(cx).update(cx, |coordinator, cx| coordinator.hold_inline_playback(epoch, cx));
        self.arm_watchdog(cx);
        to_view.set_alpha(0.0, cx);
        transition_context.add_subview(SharedElementTransitionContextKey::To, cx);
        self.fade(Some(to_view), 1.0, cx, |this, cx| this.open_did_finish(cx));
    }

    fn open_did_finish(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        let Some(context) = self.context() else { return };
        let epoch = self.epoch;
        Self::coordinator(cx).update(cx, |coordinator, cx| coordinator.release_inline_playback(epoch, cx));
        if let Some(to_view) = self.to_view() {
            to_view.set_alpha(1.0, cx);
        }
        self.set_interaction(true, cx);
        self.complete(&context, cx);
        Self::coordinator(cx).update(cx, |coordinator, cx| coordinator.transition_did_end(epoch, cx));
    }

    fn run_exit(&self, transition_context: TransitionContext, cx: &mut App) {
        if !self.begin_exit(&transition_context, cx) {
            return;
        }
        self.arm_watchdog(cx);
        if self.plan.borrow().is_none() {
            self.fade_out_chrome(cx);
            return;
        }
        self.readiness_started_at.set(RiseClock::monotonic());
        self.is_waiting_for_readiness.set(true);
        SharedElementTransitionSupport::after(
            0.0,
            self.epoch,
            self.weak(|this, cx| {
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
                this.ticker.start(this.weak_tick(|this, cx| this.check_readiness(cx)), cx);
            }),
            cx,
        );
    }

    fn begin_exit(&self, transition_context: &TransitionContext, cx: &mut App) -> bool {
        let (Some(from_view), Some(to_view)) = (
            SharedElementTransitionView::of(transition_context, SharedElementTransitionContextKey::From),
            SharedElementTransitionView::of(transition_context, SharedElementTransitionContextKey::To),
        ) else {
            self.end_without_transition(transition_context, cx);
            return false;
        };
        transition_context.apply_final_frame(SharedElementTransitionContextKey::To, cx);
        if !SharedElementTransitionSupport::is_live(&self.intent, cx) {
            if !self.interactive {
                transition_context.insert_subview_below(
                    SharedElementTransitionContextKey::To,
                    SharedElementTransitionContextKey::From,
                    cx,
                );
            }
            self.end_without_transition(transition_context, cx);
            return false;
        }
        self.start(transition_context, from_view, to_view, cx);
        let plan = self.intent.origin().and_then(|origin| origin.prepare_return(&self.intent.item_id, self.epoch, cx));
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
        if self.is_resolved.get() {
            return;
        }
        let origin = self.intent.origin();
        let (true, Some(origin)) = (self.plan.borrow().is_some(), origin) else {
            self.is_resolved.set(true);
            return;
        };
        match origin.resolve_return(self.epoch, cx) {
            SharedElementTargetResolution::Ready(_) | SharedElementTargetResolution::Unavailable => {
                self.is_resolved.set(true);
            }
            SharedElementTargetResolution::Deferred => {}
        }
    }

    fn check_readiness(&self, cx: &mut App) {
        if !self.is_waiting_for_readiness.get() || self.is_completed.get() {
            return;
        }
        if Self::coordinator(cx).read(cx).epoch() != self.epoch {
            self.stop_waiting();
            return;
        }
        self.resolve_target(cx);
        let waited = RiseClock::monotonic().saturating_sub(self.readiness_started_at.get()).as_secs_f64();
        if self.is_resolved.get() || waited >= SharedElementMotion::EXIT_RESOLVE_TIMEOUT {
            self.stop_waiting();
            self.fade_out_chrome(cx);
        }
    }

    fn fade_out_chrome(&self, cx: &mut App) {
        self.fade(self.from_view(), 0.0, cx, |this, cx| this.exit_did_finish(cx));
    }

    fn exit_did_finish(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        let Some(context) = self.context() else { return };
        if self.plan.borrow().is_some()
            && let Some(origin) = self.intent.origin()
        {
            origin.finish_return(self.epoch, cx);
        }
        if let Some(from_view) = self.from_view() {
            from_view.set_alpha(1.0, cx);
        }
        self.set_interaction(true, cx);
        self.complete(&context, cx);
        let epoch = self.epoch;
        Self::coordinator(cx).update(cx, |coordinator, cx| coordinator.transition_did_end(epoch, cx));
    }

    fn release(&self, outcome: SharedElementReleaseOutcome, context: &TransitionContext, cx: &mut App) {
        self.release_outcome.set(Some(outcome));
        self.arm_watchdog(cx);
        match outcome {
            SharedElementReleaseOutcome::Finish => {
                context.finish_interactive_transition(cx);
                self.resolve_target(cx);
                self.fade_out_chrome(cx);
            }
            SharedElementReleaseOutcome::Cancel => {
                context.cancel_interactive_transition(cx);
                self.fade(self.from_view(), 1.0, cx, |this, cx| this.cancel_did_finish(cx));
            }
        }
    }

    /// removes the profile before the scroller restores its position
    /// so the unbind that removal triggers cant overwrite the restore
    fn cancel_did_finish(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        let Some(context) = self.context() else { return };
        let epoch = self.epoch;
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.land(epoch, cx));
        Self::coordinator(cx).update(cx, |coordinator, cx| coordinator.release_inline_playback(epoch, cx));
        if let Some(to_view) = self.to_view() {
            to_view.remove_from_superview(cx);
        }
        if self.plan.borrow().is_some()
            && let Some(origin) = self.intent.origin()
        {
            origin.cancel_return(epoch, cx);
        }
        if let Some(from_view) = self.from_view() {
            from_view.set_alpha(1.0, cx);
        }
        self.set_interaction(true, cx);
        self.complete(&context, cx);
        Self::coordinator(cx).update(cx, |coordinator, cx| coordinator.transition_did_end(epoch, cx));
    }

    fn start(
        &self,
        transition_context: &TransitionContext,
        from_view: SharedElementTransitionView,
        to_view: SharedElementTransitionView,
        cx: &mut App,
    ) {
        *self.context.borrow_mut() = Some(transition_context.clone());
        *self.from_view.borrow_mut() = Some(from_view);
        *self.to_view.borrow_mut() = Some(to_view);
        if let Some(this) = self.this.upgrade() {
            let resumable: Rc<dyn SharedElementReturnResumable> = this.clone();
            SharedElementNavigationDriver::shared().transition_did_begin(self.epoch, this, Some(resumable));
        }
        let epoch = self.epoch;
        let on_resign_active: Box<dyn Fn(&mut App)> = Box::new(self.weak_hook(|this, cx| {
            this.application_will_resign_active(cx);
        }));
        let on_force_end: Box<dyn Fn(&mut App)> = Box::new(self.weak_hook(|this, cx| this.force_end(cx)));
        Self::coordinator(cx).update(cx, |coordinator, _| {
            coordinator.attach_transition_hooks(epoch, Some(on_resign_active), on_force_end);
        });
        self.set_interaction(false, cx);
    }

    fn weak_hook(&self, work: impl Fn(&Rc<Self>, &mut App) + 'static) -> impl Fn(&mut App) + 'static {
        let this = self.this.clone();
        move |cx: &mut App| {
            if let Some(this) = this.upgrade() {
                work(&this, cx);
            }
        }
    }

    fn end_without_transition(&self, transition_context: &TransitionContext, cx: &mut App) {
        self.is_completed.set(true);
        if self.interactive {
            let context = transition_context.clone();
            cx.defer(move |cx| {
                context.cancel_interactive_transition(cx);
                context.complete_transition(false, cx);
            });
        } else {
            let did_complete = !transition_context.transition_was_cancelled(cx);
            transition_context.complete_transition(did_complete, cx);
        }
        let epoch = self.epoch;
        SharedElementNavigationDriver::shared().transition_did_finish(epoch);
        Self::coordinator(cx).update(cx, |coordinator, cx| coordinator.transition_did_end(epoch, cx));
    }

    fn application_will_resign_active(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        if self.is_waiting_for_readiness.get() {
            self.stop_waiting();
            self.fade_out_chrome(cx);
        }
        if self.interactive && self.release_outcome.get().is_none() {
            return;
        }
        let fade_animator = self.fade_animator.borrow().clone();
        let Some(fade_animator) = fade_animator else { return };
        if fade_animator.state() != SharedElementViewAnimatingState::Active {
            return;
        }
        fade_animator.stop_animation(false);
        fade_animator.finish_animation_at_end(cx);
    }

    fn force_end(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        let Some(context) = self.context() else { return };
        self.stop_waiting();
        let fade_animator = self.fade_animator.borrow_mut().take();
        if let Some(fade_animator) = fade_animator {
            fade_animator.stop_animation(true);
        }
        let epoch = self.epoch;
        match self.intent.direction {
            SharedElementDirection::Open => {
                Self::coordinator(cx).update(cx, |coordinator, cx| coordinator.release_inline_playback(epoch, cx));
                if let Some(to_view) = self.to_view() {
                    to_view.set_alpha(1.0, cx);
                }
            }
            SharedElementDirection::Exit => {
                let owns_scroller = Self::coordinator(cx).read(cx).epoch() == epoch && self.plan.borrow().is_some();
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
                        origin.cancel_return(epoch, cx);
                    }
                } else if owns_scroller && let Some(origin) = self.intent.origin() {
                    origin.finish_return(epoch, cx);
                }
                if let Some(from_view) = self.from_view() {
                    from_view.set_alpha(1.0, cx);
                }
            }
        }
        self.set_interaction(true, cx);
        self.complete(&context, cx);
        Self::coordinator(cx).update(cx, |coordinator, cx| coordinator.transition_did_end(epoch, cx));
    }

    fn complete(&self, context: &TransitionContext, cx: &mut App) {
        self.is_completed.set(true);
        self.stop_waiting();
        self.watchdog.cancel();
        let fade_animator = self.fade_animator.borrow_mut().take();
        drop(fade_animator);
        let epoch = self.epoch;
        Self::coordinator(cx).update(cx, |coordinator, _| coordinator.detach_transition_hooks(epoch));
        let did_complete = !context.transition_was_cancelled(cx);
        context.complete_transition(did_complete, cx);
        *self.context.borrow_mut() = None;
        SharedElementNavigationDriver::shared().transition_did_finish(epoch);
    }

    fn stop_waiting(&self) {
        self.is_waiting_for_readiness.set(false);
        self.ticker.stop();
    }

    fn set_interaction(&self, enabled: bool, cx: &mut App) {
        if let Some(from_view) = self.from_view() {
            from_view.set_user_interaction_enabled(enabled, cx);
        }
        if let Some(to_view) = self.to_view() {
            to_view.set_user_interaction_enabled(enabled, cx);
        }
    }
}

impl SharedElementViewControllerAnimatedTransitioning for SharedElementCrossfadeAnimator {
    fn transition_duration(&self, _: Option<&TransitionContext>) -> f64 {
        SharedElementMotion::REDUCE_MOTION_FADE
    }

    fn animate_transition(&self, transition_context: TransitionContext, cx: &mut App) {
        match self.intent.direction {
            SharedElementDirection::Open => self.run_open(transition_context, cx),
            SharedElementDirection::Exit => self.run_exit(transition_context, cx),
        }
    }
}

impl SharedElementViewControllerInteractiveTransitioning for SharedElementCrossfadeAnimator {
    fn start_interactive_transition(&self, transition_context: TransitionContext, cx: &mut App) {
        self.did_start.set(true);
        if self.intent.direction != SharedElementDirection::Exit {
            self.run_open(transition_context, cx);
            return;
        }
        if !self.begin_exit(&transition_context, cx) {
            return;
        }
        SharedElementTransitionSupport::after(
            0.0,
            self.epoch,
            self.weak(|this, cx| {
                if this.is_completed.get() || this.release_outcome.get().is_some() {
                    return;
                }
                if let Some(to_view) = this.to_view() {
                    to_view.layout_if_needed(cx);
                }
                this.resolve_target(cx);
            }),
            cx,
        );
        self.layout_sign.set(SharedElementTransitionSupport::layout_sign(cx));
        if let Some(pan) = self.pan() {
            self.touch_start.set(SharedElementTransitionSupport::screen_point(pan.location()));
        }
        self.ticker.start(self.weak_tick(|this, cx| this.check_pan_liveness(cx)), cx);
        self.check_pan_liveness(cx);
    }

    fn wants_interactive_start(&self) -> bool {
        self.interactive
    }
}

impl SharedElementInteractivePop for SharedElementCrossfadeAnimator {
    fn epoch(&self) -> u64 {
        self.epoch
    }

    fn did_start(&self) -> bool {
        self.did_start.get()
    }

    fn handle_pan(&self, pan: &Rc<dyn SharedElementPanGesture>, cx: &mut App) {
        if !self.did_start.get() || self.is_completed.get() || self.release_outcome.get().is_some() {
            return;
        }
        let Some(context) = self.context() else { return };
        let touch_start = self.touch_start.get();
        let input = SharedElementGesturePolicyInput {
            translation: SharedElementTransitionSupport::vector(pan.translation()),
            velocity: SharedElementTransitionSupport::vector(pan.velocity()),
            touch_start,
            source: self.intent.capture().map_or_else(
                || Bounds { origin: touch_start, size: Size::default() },
                |capture| capture.clip,
            ),
            viewport: context.container_bounds(cx).size,
            layout_sign: self.layout_sign.get(),
            axis: self.axis,
        };
        match pan.state() {
            SharedElementPanState::Changed => {
                let frame = SharedElementGesturePolicy::sample(&input);
                if let Some(from_view) = self.from_view() {
                    from_view.set_alpha(frame.chrome_alpha, cx);
                }
                context.update_interactive_transition(frame.progress, cx);
            }
            SharedElementPanState::Ended => {
                let outcome = if SharedElementGesturePolicy::decision(&input) == SharedElementGesturePolicyDecision::Finish {
                    SharedElementReleaseOutcome::Finish
                } else {
                    SharedElementReleaseOutcome::Cancel
                };
                self.release(outcome, &context, cx);
            }
            SharedElementPanState::Cancelled | SharedElementPanState::Failed => {
                self.release(SharedElementReleaseOutcome::Cancel, &context, cx);
            }
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

impl SharedElementReturnResumable for SharedElementCrossfadeAnimator {
    fn return_did_become_resumable(&self, epoch: u64, cx: &mut App) {
        if epoch != self.epoch || self.is_completed.get() {
            return;
        }
        if self.is_waiting_for_readiness.get() {
            self.check_readiness(cx);
        } else if self.interactive && self.release_outcome.get() != Some(SharedElementReleaseOutcome::Cancel) {
            self.resolve_target(cx);
        }
    }
}
