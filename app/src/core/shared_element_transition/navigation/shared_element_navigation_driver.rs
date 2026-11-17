use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{App, AsyncApp, Bounds, Entity, Pixels, Point};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::LanguageStore;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::{CubicBezier, duration_from_seconds};

use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementDirection, SharedElementEndpoint, SharedElementExitStyle,
    SharedElementGeometry, SharedElementGesturePolicyAxis, SharedElementKey, SharedElementOpenStyle, SharedElementPart,
    SharedElementPopPanMode, SharedElementSessionPolicy, SharedElementSessionPolicyPhase,
};
use crate::core::shared_element_transition::navigation::right_to_left_interactive_pop::RightToLeftInteractivePop;
use crate::core::shared_element_transition::navigation::shared_element_crossfade_animator::SharedElementCrossfadeAnimator;
use crate::core::shared_element_transition::navigation::shared_element_navigation_delegate_proxy::SharedElementNavigationDelegateProxy;
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::{
    SharedElementNavigationController, SharedElementNavigationHost, SharedElementNavigationOperation,
    SharedElementPanGesture, SharedElementTransitionContextKey, SharedElementTransitionView,
    SharedElementViewControllerAnimatedTransitioning, SharedElementViewControllerInteractiveTransitioning,
};
use crate::core::shared_element_transition::navigation::shared_element_open_animator::SharedElementOpenAnimator;
use crate::core::shared_element_transition::navigation::shared_element_return_animator::SharedElementReturnAnimator;
use crate::core::shared_element_transition::renderer::shared_element_flight_runner::SharedElementFlightRunner;
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementDestinationContainer, SharedElementFlightContent, SharedElementOriginContainer,
};
use crate::core::shared_element_transition::runtime::shared_element_lift_board::SharedElementLiftBoard;
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::{
    SharedElementIntent, SharedElementTransitionCoordinator,
};
use crate::modules::post::engine::rise_post_engine_models::RisePostPageScope;

const LOG_NAME: &str = "SharedElement";

/// interactive pop controller the driver hands to the stack and feeds with pan samples
pub trait SharedElementInteractivePop:
    SharedElementViewControllerAnimatedTransitioning + SharedElementViewControllerInteractiveTransitioning
{
    fn epoch(&self) -> u64;
    /// set synchronously by start_interactive_transition
    fn did_start(&self) -> bool;
    fn handle_pan(&self, pan: &Rc<dyn SharedElementPanGesture>, cx: &mut App);
    fn as_animated(self: Rc<Self>) -> Rc<dyn SharedElementViewControllerAnimatedTransitioning>;
    fn as_interactive(self: Rc<Self>) -> Rc<dyn SharedElementViewControllerInteractiveTransitioning>;
}

/// running return transition that rechecks its target when the scroller finishes a deferred layout
pub trait SharedElementReturnResumable {
    fn return_did_become_resumable(&self, epoch: u64, cx: &mut App);
}

#[derive(Clone)]
struct PendingInteraction {
    epoch: u64,
    controller: Rc<dyn SharedElementInteractivePop>,
}

struct StockFade {
    epoch: u64,
    started: Cell<bool>,
}

impl StockFade {
    fn new(epoch: u64) -> Rc<Self> {
        Rc::new(Self { epoch, started: Cell::new(false) })
    }
}

/// controller that runs the current transition, kept alive until it completes its context
struct ActiveController {
    _controller: Rc<dyn Any>,
    resumable: Option<Rc<dyn SharedElementReturnResumable>>,
}

thread_local! {
    static SHARED: Rc<SharedElementNavigationDriver> = Rc::new_cyclic(|this| SharedElementNavigationDriver {
        this: this.clone(),
        pending_interaction: RefCell::new(None),
        active_epoch: Cell::new(None),
        active_controller: RefCell::new(None),
        stock_fade: RefCell::new(None),
        stock_armed_epoch: Cell::new(None),
        right_to_left_pop: RefCell::new(Weak::new()),
    });
}

/// owns the delegate proxy on the route stack and the back pan mode latch
/// also the interactive begin and the stock transition media fade
pub struct SharedElementNavigationDriver {
    this: Weak<Self>,
    pending_interaction: RefCell<Option<PendingInteraction>>,
    active_epoch: Cell<Option<u64>>,
    active_controller: RefCell<Option<ActiveController>>,
    stock_fade: RefCell<Option<Rc<StockFade>>>,
    stock_armed_epoch: Cell<Option<u64>>,
    right_to_left_pop: RefCell<Weak<RightToLeftInteractivePop>>,
}

impl SharedElementNavigationDriver {
    pub fn shared() -> Rc<Self> {
        SHARED.with(Rc::clone)
    }

    fn coordinator(cx: &mut App) -> Entity<SharedElementTransitionCoordinator> {
        SharedElementTransitionCoordinator::shared(cx)
    }

    pub fn attach(&self, nav: &Rc<dyn SharedElementNavigationController>, cx: &mut App) {
        let current = nav.delegate();
        let associated = nav.associated_delegate_proxy();
        if let (Some(associated), Some(current)) = (associated.as_ref(), current.as_ref())
            && std::ptr::addr_eq(Rc::as_ptr(associated), Rc::as_ptr(current))
        {
            return;
        }
        let base = match current.as_ref() {
            Some(current) => match current.as_delegate_proxy() {
                Some(foreign) => foreign.base(),
                None => Some(current.clone()),
            },
            None => None,
        };
        let proxy = Rc::new(SharedElementNavigationDelegateProxy::new(base.as_ref()));
        nav.set_associated_delegate_proxy(Some(proxy.clone()));
        nav.set_delegate(Some(proxy));
        let has_base = base.is_some();
        let rewrap = associated.is_some();
        Self::log(move || format!("attach base={} rewrap={rewrap}", if has_base { "delegate" } else { "nil" }), cx);
    }

    /// registrations only, no capture and no layout
    pub fn pop_pan_mode(
        &self,
        nav: &Rc<dyn SharedElementNavigationController>,
        _pan: &Rc<dyn SharedElementPanGesture>,
        cx: &mut App,
    ) -> SharedElementPopPanMode {
        if !SharedElementNavigationPolicy::flights_enabled() {
            return SharedElementPopPanMode::Stock;
        }
        let top = nav.top_view_controller(cx);
        let coordinator = Self::coordinator(cx);
        let destination = coordinator.read(cx).destination_hosted_by(top);
        let origin = destination.as_ref().and_then(|destination| self.registered_origin(destination, top, cx));
        let can_begin = destination.as_ref().is_some_and(|destination| destination.can_begin_interactive_return(cx));
        let phase_idle = coordinator.read(cx).phase() == SharedElementSessionPolicyPhase::Idle;
        SharedElementSessionPolicy::pop_pan_mode(
            SharedElementNavigationPolicy::flights_enabled(),
            SharedElementNavigationPolicy::interactive_enabled(),
            destination.is_some(),
            can_begin,
            origin.is_some(),
            phase_idle,
            Self::preferred_transition_is_nil(nav, top, cx),
            reduce_motion(cx),
        )
    }

    pub fn arm_exit_for_back(nav: &Rc<dyn SharedElementNavigationController>, cx: &mut App) {
        let top = nav.top_view_controller(cx);
        let coordinator = Self::coordinator(cx);
        let destination = coordinator.read(cx).destination_hosted_by(top);
        if let Some(destination) = destination {
            let _ = SharedElementTransitionCoordinator::arm_exit(&destination, false, cx);
        }
    }

    /// arms the exit for a latched pan on began and
    /// returns the mode that drives the rest of the gesture
    pub fn begin_interactive_pop(
        &self,
        nav: &Rc<dyn SharedElementNavigationController>,
        pan: &Rc<dyn SharedElementPanGesture>,
        mode: SharedElementPopPanMode,
        axis: SharedElementGesturePolicyAxis,
        cx: &mut App,
    ) -> SharedElementPopPanMode {
        match mode {
            SharedElementPopPanMode::Stock | SharedElementPopPanMode::Ignore => return mode,
            SharedElementPopPanMode::StockArmed | SharedElementPopPanMode::Flight | SharedElementPopPanMode::Crossfade => {}
        }
        let top = nav.top_view_controller(cx);
        let coordinator = Self::coordinator(cx);
        let Some(destination) = coordinator.read(cx).destination_hosted_by(top) else {
            return SharedElementPopPanMode::Stock;
        };
        let Some(intent) = SharedElementTransitionCoordinator::arm_exit(&destination, true, cx) else {
            return SharedElementPopPanMode::Stock;
        };

        let controller: Option<Rc<dyn SharedElementInteractivePop>> = if mode == SharedElementPopPanMode::StockArmed {
            None
        } else {
            match intent.exit_style {
                Some(SharedElementExitStyle::Flight) => {
                    Some(SharedElementReturnAnimator::new(intent.clone(), true, Some(pan.clone()), axis, cx))
                }
                Some(SharedElementExitStyle::Crossfade) => {
                    Some(SharedElementCrossfadeAnimator::new(intent.clone(), true, Some(pan.clone()), axis))
                }
                Some(SharedElementExitStyle::StockWithFade) | Some(SharedElementExitStyle::Stock) | None => None,
            }
        };
        let Some(controller) = controller else {
            self.stock_armed_epoch.set(Some(intent.epoch));
            return SharedElementPopPanMode::StockArmed;
        };

        *self.pending_interaction.borrow_mut() =
            Some(PendingInteraction { epoch: intent.epoch, controller: controller.clone() });
        nav.pop_view_controller(true, cx);
        let epoch = intent.epoch;
        if !controller.did_start() {
            let this = self.this.clone();
            SharedElementTransitionSupport::after(
                SharedElementMotion::INTENT_TTL,
                epoch,
                move |cx: &mut App| {
                    let Some(this) = this.upgrade() else { return };
                    let pending = this.pending_interaction.borrow().clone();
                    let Some(pending) = pending else { return };
                    if pending.epoch != epoch || pending.controller.did_start() {
                        return;
                    }
                    *this.pending_interaction.borrow_mut() = None;
                    Self::coordinator(cx)
                        .update(cx, |coordinator, cx| coordinator.abandon(epoch, "interactive_not_started", cx));
                },
                cx,
            );
        }
        let style = intent.exit_style;
        let started = controller.did_start();
        Self::log(move || format!("interactive begin epoch={epoch} style={style:?} started={started}"), cx);
        if intent.exit_style == Some(SharedElementExitStyle::Crossfade) {
            SharedElementPopPanMode::Crossfade
        } else {
            SharedElementPopPanMode::Flight
        }
    }

    pub fn begin_right_to_left_pop(
        &self,
        nav: &Rc<dyn SharedElementNavigationController>,
        cx: &mut App,
    ) -> Rc<RightToLeftInteractivePop> {
        let pop = RightToLeftInteractivePop::new();
        *self.right_to_left_pop.borrow_mut() = Rc::downgrade(&pop);
        nav.pop_view_controller(true, cx);
        pop
    }

    pub fn handle_interactive_pan(&self, pan: &Rc<dyn SharedElementPanGesture>, cx: &mut App) {
        let pending = self.pending_interaction.borrow().clone();
        let Some(pending) = pending else { return };
        if pending.epoch != Self::coordinator(cx).read(cx).epoch() {
            *self.pending_interaction.borrow_mut() = None;
            return;
        }
        pending.controller.handle_pan(pan, cx);
    }

    pub fn animation_controller(
        &self,
        operation: SharedElementNavigationOperation,
        from: SharedElementNavigationHost,
        to: SharedElementNavigationHost,
        cx: &mut App,
    ) -> Option<Rc<dyn SharedElementViewControllerAnimatedTransitioning>> {
        if operation == SharedElementNavigationOperation::Pop {
            let pop = self.right_to_left_pop.replace(Weak::new()).upgrade();
            if let Some(pop) = pop {
                return Some(pop);
            }
        }
        let coordinator = Self::coordinator(cx);
        if coordinator.read(cx).phase() != SharedElementSessionPolicyPhase::Armed {
            return None;
        }
        let armed = coordinator.read(cx).intent()?;
        let (armed_epoch, armed_direction) = (armed.epoch, armed.direction);
        let raw_operation = operation.raw_value();
        Self::log(
            move || format!("delegate op={raw_operation} epoch={armed_epoch} direction={armed_direction:?}"),
            cx,
        );
        if self.stock_armed_epoch.get() == Some(armed.epoch) {
            return None;
        }

        let pending = self.pending_interaction.borrow().clone();
        if let Some(pending) = pending {
            if pending.epoch == armed.epoch {
                let consumed = coordinator.update(cx, |coordinator, cx| coordinator.consume(operation, from, to, cx));
                if !consumed.is_some_and(|intent| Rc::ptr_eq(&intent, &armed)) {
                    *self.pending_interaction.borrow_mut() = None;
                    return None;
                }
                return Some(pending.controller.as_animated());
            }
            *self.pending_interaction.borrow_mut() = None;
        }

        let intent = coordinator.update(cx, |coordinator, cx| coordinator.consume(operation, from, to, cx))?;
        match intent.direction {
            SharedElementDirection::Open => match intent.open_style {
                Some(SharedElementOpenStyle::Flight) => {
                    if intent.capture().is_none() {
                        return Some(SharedElementCrossfadeAnimator::new(
                            intent,
                            false,
                            None,
                            SharedElementGesturePolicyAxis::Horizontal,
                        ));
                    }
                    Some(SharedElementOpenAnimator::new(intent, cx))
                }
                Some(SharedElementOpenStyle::Crossfade) => Some(SharedElementCrossfadeAnimator::new(
                    intent,
                    false,
                    None,
                    SharedElementGesturePolicyAxis::Horizontal,
                )),
                Some(SharedElementOpenStyle::Stock) | None => {
                    let epoch = intent.epoch;
                    coordinator.update(cx, |coordinator, cx| coordinator.abandon(epoch, "open_stock", cx));
                    None
                }
            },
            SharedElementDirection::Exit => match intent.exit_style {
                Some(SharedElementExitStyle::Flight) => Some(SharedElementReturnAnimator::new(
                    intent,
                    false,
                    None,
                    SharedElementGesturePolicyAxis::Horizontal,
                    cx,
                )),
                Some(SharedElementExitStyle::Crossfade) => Some(SharedElementCrossfadeAnimator::new(
                    intent,
                    false,
                    None,
                    SharedElementGesturePolicyAxis::Horizontal,
                )),
                Some(SharedElementExitStyle::StockWithFade) => {
                    self.await_stock_fade(intent.epoch, cx);
                    None
                }
                Some(SharedElementExitStyle::Stock) | None => {
                    let epoch = intent.epoch;
                    coordinator.update(cx, |coordinator, cx| coordinator.abandon(epoch, "exit_stock", cx));
                    None
                }
            },
        }
    }

    pub fn interaction_controller(
        &self,
        animator: &Rc<dyn SharedElementViewControllerAnimatedTransitioning>,
        cx: &mut App,
    ) -> Option<Rc<dyn SharedElementViewControllerInteractiveTransitioning>> {
        if let Some(pop) = animator.clone().as_right_to_left_pop() {
            return Some(pop);
        }
        let pending = self.pending_interaction.borrow().clone()?;
        if pending.epoch != Self::coordinator(cx).read(cx).epoch()
            || !std::ptr::addr_eq(Rc::as_ptr(&pending.controller), Rc::as_ptr(animator))
        {
            return None;
        }
        Some(pending.controller.as_interactive())
    }

    pub fn will_show(
        &self,
        nav: &Rc<dyn SharedElementNavigationController>,
        vc: SharedElementNavigationHost,
        _animated: bool,
        cx: &mut App,
    ) {
        if !SharedElementNavigationPolicy::flights_enabled() {
            return;
        }
        let coordinator = Self::coordinator(cx);
        let fade = self.stock_fade.borrow().clone();
        if let Some(fade) = fade.filter(|fade| !fade.started.get()) {
            let live = {
                let coordinator = coordinator.read(cx);
                fade.epoch == coordinator.epoch() && coordinator.phase() == SharedElementSessionPolicyPhase::Running
            };
            if !live {
                *self.stock_fade.borrow_mut() = None;
                return;
            }
            self.start_stock_fade(&fade, nav, cx);
            return;
        }

        let (phase, armed) = {
            let coordinator = coordinator.read(cx);
            (coordinator.phase(), coordinator.intent())
        };
        if phase != SharedElementSessionPolicyPhase::Armed {
            return;
        }
        let Some(armed) = armed else { return };
        if armed.direction != SharedElementDirection::Exit
            || !(self.stock_armed_epoch.get() == Some(armed.epoch)
                || armed.exit_style == Some(SharedElementExitStyle::StockWithFade))
        {
            return;
        }
        let Some(transition) = nav.transition_coordinator(cx) else { return };
        let Some(from) = transition.view_controller(SharedElementTransitionContextKey::From) else { return };
        let Some(intent) = coordinator.update(cx, |coordinator, cx| {
            coordinator.consume(SharedElementNavigationOperation::Pop, from, vc, cx)
        }) else {
            return;
        };
        let fade = StockFade::new(intent.epoch);
        *self.stock_fade.borrow_mut() = Some(fade.clone());
        self.start_stock_fade(&fade, nav, cx);
    }

    pub fn did_show(
        &self,
        nav: &Rc<dyn SharedElementNavigationController>,
        vc: SharedElementNavigationHost,
        _animated: bool,
        cx: &mut App,
    ) {
        if let Some(associated) = nav.associated_delegate_proxy() {
            let replaced = nav
                .delegate()
                .is_none_or(|delegate| !std::ptr::addr_eq(Rc::as_ptr(&delegate), Rc::as_ptr(&associated)));
            if replaced {
                Self::log(|| "delegate replaced after attach".to_owned(), cx);
            }
        }
        if !SharedElementNavigationPolicy::flights_enabled() {
            return;
        }
        let coordinator = Self::coordinator(cx);
        let hosts_destination = coordinator.read(cx).destination_hosted_by(Some(vc)).is_some();
        nav.set_interactive_content_pop_enabled(!hosts_destination, cx);
        let fade = self.stock_fade.borrow().clone();
        if let Some(fade) = fade.filter(|fade| !fade.started.get()) {
            *self.stock_fade.borrow_mut() = None;
            if self.stock_armed_epoch.get() == Some(fade.epoch) {
                self.stock_armed_epoch.set(None);
            }
            let epoch = fade.epoch;
            coordinator.update(cx, |coordinator, cx| coordinator.transition_did_end(epoch, cx));
        }
    }

    /// keeps the running controller alive until it completes its context, whatever the stack retains
    pub fn transition_did_begin(
        &self,
        epoch: u64,
        controller: Rc<dyn Any>,
        resumable: Option<Rc<dyn SharedElementReturnResumable>>,
    ) {
        self.active_epoch.set(Some(epoch));
        *self.active_controller.borrow_mut() = Some(ActiveController { _controller: controller, resumable });
    }

    pub fn transition_did_finish(&self, epoch: u64) {
        if self.active_epoch.get() == Some(epoch) {
            self.active_epoch.set(None);
            let finished = self.active_controller.borrow_mut().take();
            drop(finished);
        }
        let pending_matches = self.pending_interaction.borrow().as_ref().is_some_and(|pending| pending.epoch == epoch);
        if pending_matches {
            *self.pending_interaction.borrow_mut() = None;
        }
        if self.stock_armed_epoch.get() == Some(epoch) {
            self.stock_armed_epoch.set(None);
        }
    }

    pub fn return_did_become_resumable(&self, epoch: u64, cx: &mut App) {
        if self.active_epoch.get() != Some(epoch) || Self::coordinator(cx).read(cx).epoch() != epoch {
            return;
        }
        let resumable = self.active_controller.borrow().as_ref().and_then(|active| active.resumable.clone());
        if let Some(resumable) = resumable {
            resumable.return_did_become_resumable(epoch, cx);
        }
    }

    fn await_stock_fade(&self, epoch: u64, cx: &mut App) {
        let fade = StockFade::new(epoch);
        *self.stock_fade.borrow_mut() = Some(fade.clone());
        let this = self.this.clone();
        SharedElementTransitionSupport::after(
            SharedElementMotion::WATCHDOG,
            epoch,
            move |cx: &mut App| {
                let Some(this) = this.upgrade() else { return };
                let pending = this.stock_fade.borrow().clone();
                let Some(pending) = pending else { return };
                if !Rc::ptr_eq(&pending, &fade) || pending.started.get() {
                    return;
                }
                *this.stock_fade.borrow_mut() = None;
                Self::coordinator(cx).update(cx, |coordinator, cx| coordinator.transition_did_end(epoch, cx));
            },
            cx,
        );
    }

    fn start_stock_fade(&self, fade: &Rc<StockFade>, nav: &Rc<dyn SharedElementNavigationController>, cx: &mut App) {
        fade.started.set(true);
        let epoch = fade.epoch;
        let Some(transition) = nav.transition_coordinator(cx) else {
            self.finish_stock_fade(fade, None, cx);
            return;
        };
        let intent = Self::coordinator(cx).read(cx).intent().filter(|intent| intent.epoch == epoch);
        let capture = intent.as_ref().and_then(|intent| intent.capture());
        let from_view = transition.flight_container(SharedElementTransitionContextKey::From, cx);
        let (Some(intent), Some(capture), Some(from_view)) = (intent, capture, from_view) else {
            self.finish_stock_fade(fade, None, cx);
            return;
        };
        if let Some(stop_frame) = capture.stop_frame.as_ref() {
            stop_frame.cancel();
        }
        if let Some(live_readiness) = capture.live_readiness.as_ref() {
            live_readiness.cancel();
        }
        let runner = SharedElementFlightRunner::new(from_view, epoch, cx);
        if let Some(placeholder) = capture.placeholder.clone() {
            runner.flight_view().update(cx, |flight_view, cx| flight_view.set_placeholder(placeholder, cx));
        }
        runner.begin(
            capture.content.clone(),
            SharedElementTransitionSupport::source(&capture),
            capture.backdrop.clone(),
            cx,
        );
        let mut keys: HashSet<SharedElementKey> = HashSet::from([capture.source_key.clone()]);
        if let Some(destination) = intent.destination() {
            keys.extend(destination.lift_keys(&capture.item_id, capture.part, cx));
        }
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.lift(keys, epoch, cx));

        let flight_view = runner.flight_view();
        let scale = SharedElementMotion::STOCK_FADE_SCALE;
        let this = self.this.clone();
        let finishing_fade = fade.clone();
        let finishing_runner = runner.clone();
        let started = transition.animate_alongside_transition(
            Rc::new(move |progress: f32, cx: &mut App| {
                flight_view.update(cx, |flight_view, cx| {
                    flight_view.set_alpha(1.0 - progress, cx);
                    flight_view.set_transform_scale(SharedElementGeometry::lerp(1.0, scale, progress), cx);
                });
            }),
            Box::new(move |cx: &mut App| {
                if let Some(this) = this.upgrade() {
                    this.finish_stock_fade(&finishing_fade, Some(finishing_runner), cx);
                }
            }),
            cx,
        );
        if !started {
            self.finish_stock_fade(fade, Some(runner), cx);
        }
    }

    fn finish_stock_fade(&self, fade: &Rc<StockFade>, runner: Option<SharedElementFlightRunner>, cx: &mut App) {
        let is_current = self.stock_fade.borrow().as_ref().is_some_and(|current| Rc::ptr_eq(current, fade));
        if is_current {
            *self.stock_fade.borrow_mut() = None;
        }
        if self.stock_armed_epoch.get() == Some(fade.epoch) {
            self.stock_armed_epoch.set(None);
        }
        let coordinator = Self::coordinator(cx);
        if coordinator.read(cx).epoch() != fade.epoch {
            if let Some(runner) = runner {
                runner.abort(cx);
            }
            return;
        }
        let epoch = fade.epoch;
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.land(epoch, cx));
        if let Some(runner) = runner {
            runner.remove_after_commit(cx);
        }
        coordinator.update(cx, |coordinator, cx| coordinator.transition_did_end(epoch, cx));
    }

    fn registered_origin(
        &self,
        destination: &Rc<dyn SharedElementDestinationContainer>,
        fallback_host: Option<SharedElementNavigationHost>,
        cx: &mut App,
    ) -> Option<Rc<dyn SharedElementOriginContainer>> {
        let coordinator = Self::coordinator(cx);
        let host = destination
            .host_view_controller(cx)
            .or_else(|| coordinator.read(cx).registered_host_of_destination(destination))
            .or(fallback_host)?;
        coordinator.read(cx).origin_below(host, &destination.origin_surface(), cx)
    }

    fn preferred_transition_is_nil(
        nav: &Rc<dyn SharedElementNavigationController>,
        controller: Option<SharedElementNavigationHost>,
        cx: &App,
    ) -> bool {
        let Some(controller) = controller else { return true };
        nav.preferred_transition_is_nil(controller, cx)
    }

    fn log(message: impl FnOnce() -> String, cx: &mut App) {
        Sai::log(LOG_NAME, message, SaiLogKind::Ui, cx);
    }
}

impl SharedElementNavigationPolicy {
    /// a post list opened from an enabled profile tab goes through
    /// the delegate animators so it must not carry the zoom transition
    /// with flights off it keeps the zoom
    pub fn uses_shared_element_navigation_post_detail_origin(origin: Option<&RisePostPageScope>) -> bool {
        if !Self::flights_enabled() {
            return false;
        }
        let Some(RisePostPageScope::Profile { kind, .. }) = origin else { return false };
        Self::ENABLED_KINDS.contains(kind)
    }

    /// riffs viewer opened from a profile tab in VIEWER_KINDS
    /// or a riff only row of a tab in ENABLED_KINDS
    /// those go to the viewer instead of the list, same zoom rule
    pub fn uses_shared_element_navigation_short_viewer_origin(origin: Option<&RisePostPageScope>) -> bool {
        if !Self::flights_enabled() {
            return false;
        }
        let Some(RisePostPageScope::Profile { user_id, kind, .. }) = origin else { return false };
        !user_id.is_empty() && (Self::VIEWER_KINDS.contains(kind) || Self::ENABLED_KINDS.contains(kind))
    }

    /// whether a pushed route flies and so carries no zoom transition
    pub fn uses_shared_element_navigation_route(route: &RootRoute) -> bool {
        match route {
            RootRoute::PostDetail { origin, .. } => Self::uses_shared_element_navigation_post_detail_origin(origin.as_ref()),
            RootRoute::ShortViewer { origin, .. } => {
                Self::uses_shared_element_navigation_short_viewer_origin(origin.as_ref())
            }
            _ => false,
        }
    }
}

/// geometry and scheduling shared by the open and return and crossfade animators
pub enum SharedElementTransitionSupport {}

impl SharedElementTransitionSupport {
    /// runs work on the main thread after delay while the coordinator is still on that epoch
    pub fn after(delay: f64, epoch: u64, work: impl FnOnce(&mut App) + 'static, cx: &mut App) {
        let delay = duration_from_seconds(delay.max(0.0));
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(delay).await;
            let _ = cx.update(move |cx| {
                if SharedElementTransitionCoordinator::shared(cx).read(cx).epoch() != epoch {
                    return;
                }
                work(cx);
            });
        })
        .detach();
    }

    pub fn is_live(intent: &Rc<SharedElementIntent>, cx: &mut App) -> bool {
        let coordinator = SharedElementTransitionCoordinator::shared(cx);
        let coordinator = coordinator.read(cx);
        coordinator.epoch() == intent.epoch
            && coordinator.intent().is_some_and(|current| Rc::ptr_eq(&current, intent))
    }

    pub fn source(capture: &SharedElementCapture) -> SharedElementEndpoint {
        SharedElementEndpoint { clip: capture.clip, content: capture.content_rect, corner_radius: 0.0, mode: capture.mode }
    }

    pub fn content_aspect(capture: &SharedElementCapture) -> f32 {
        match &capture.content {
            SharedElementFlightContent::Bitmap(_, aspect) if aspect.is_finite() && *aspect > 0.0 => *aspect,
            SharedElementFlightContent::Replicant(_, size)
                if f32::from(size.width) > 0.0 && f32::from(size.height) > 0.0 =>
            {
                f32::from(size.width) / f32::from(size.height)
            }
            _ => {
                let rect = capture.content_rect;
                let (width, height) = (f32::from(rect.size.width), f32::from(rect.size.height));
                if width > 0.0 && height > 0.0 { width / height } else { 1.0 }
            }
        }
    }

    /// list landing of an open, the row fill or fit box or the text block at the list width
    pub fn open_landing(
        capture: &SharedElementCapture,
        clip: Bounds<Pixels>,
        mode: SharedElementContentMode,
    ) -> SharedElementEndpoint {
        let aspect = Self::content_aspect(capture);
        let (content, landing_mode) = match (capture.part, mode) {
            (SharedElementPart::Text, _) | (SharedElementPart::Media, SharedElementContentMode::AnchoredWidthFit) => (
                SharedElementGeometry::anchored_width_fit_rect(capture.clip.size, clip),
                SharedElementContentMode::AnchoredWidthFit,
            ),
            (SharedElementPart::Media, SharedElementContentMode::Fill) => {
                (SharedElementGeometry::fill_rect(aspect, clip), SharedElementContentMode::Fill)
            }
            (SharedElementPart::Media, SharedElementContentMode::Fit { .. }) => {
                (SharedElementGeometry::fit_rect(aspect, clip), mode)
            }
        };
        SharedElementEndpoint { clip, content, corner_radius: 0.0, mode: landing_mode }
    }

    /// grid landing of an exit, photo and video fill the cell
    /// text and plates and strips keep their width anchored aspect
    pub fn return_landing(
        capture: &SharedElementCapture,
        source: &SharedElementEndpoint,
        clip: Bounds<Pixels>,
        corner_radius: f32,
    ) -> SharedElementEndpoint {
        match (capture.part, capture.mode) {
            (SharedElementPart::Text, _) | (SharedElementPart::Media, SharedElementContentMode::AnchoredWidthFit) => {
                SharedElementEndpoint {
                    clip,
                    content: SharedElementGeometry::anchored_width_fit_rect(source.content.size, clip),
                    corner_radius,
                    mode: SharedElementContentMode::AnchoredWidthFit,
                }
            }
            (SharedElementPart::Media, SharedElementContentMode::Fill)
            | (SharedElementPart::Media, SharedElementContentMode::Fit { .. }) => SharedElementEndpoint {
                clip,
                content: SharedElementGeometry::fill_rect(Self::content_aspect(capture), clip),
                corner_radius,
                mode: SharedElementContentMode::Fill,
            },
        }
    }

    /// both keys since the grid anchors a post by media or by text, whichever its cell draws
    pub fn grid_keys(origin: Option<&Rc<dyn SharedElementOriginContainer>>, item_id: &str) -> HashSet<SharedElementKey> {
        let Some(origin) = origin else { return HashSet::new() };
        HashSet::from([
            SharedElementKey::new(origin.surface(), item_id, SharedElementPart::Media),
            SharedElementKey::new(origin.surface(), item_id, SharedElementPart::Text),
        ])
    }

    /// the desktop window is the screen of a transition so window points are already screen points
    pub fn screen_point(from_window_point: Point<Pixels>) -> Point<Pixels> {
        from_window_point
    }

    /// layout direction sign of the app, the language store forces it
    pub fn layout_sign(cx: &App) -> f32 {
        if LanguageStore::active(cx).is_some_and(LanguageStore::is_rtl) { -1.0 } else { 1.0 }
    }

    pub fn vector(point: Point<Pixels>) -> Point<Pixels> {
        point
    }

    /// ease out alpha animation, completion runs only while the coordinator is on that epoch
    pub fn fade(
        view: Option<SharedElementTransitionView>,
        to: f32,
        duration: f64,
        epoch: u64,
        completion: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) -> SharedElementViewPropertyAnimator {
        let animator = SharedElementViewPropertyAnimator::new(
            view,
            to,
            duration.max(0.0),
            Box::new(move |cx: &mut App| {
                if SharedElementTransitionCoordinator::shared(cx).read(cx).epoch() != epoch {
                    return;
                }
                completion(cx);
            }),
        );
        animator.start_animation(cx);
        animator
    }
}

/// state of a view animation
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SharedElementViewAnimatingState {
    Inactive,
    Active,
    Stopped,
}

struct PropertyAnimatorState {
    view: Option<SharedElementTransitionView>,
    from: Cell<f32>,
    to: f32,
    duration: f64,
    started_at: Cell<Duration>,
    state: Cell<SharedElementViewAnimatingState>,
    ticker: SharedElementFrameTicker,
    completion: RefCell<Option<Box<dyn FnOnce(&mut App)>>>,
}

/// ease out property animator over the alpha of one transition view
#[derive(Clone)]
pub struct SharedElementViewPropertyAnimator {
    inner: Rc<PropertyAnimatorState>,
}

impl SharedElementViewPropertyAnimator {
    fn new(
        view: Option<SharedElementTransitionView>,
        to: f32,
        duration: f64,
        completion: Box<dyn FnOnce(&mut App)>,
    ) -> Self {
        Self {
            inner: Rc::new(PropertyAnimatorState {
                view,
                from: Cell::new(to),
                to,
                duration,
                started_at: Cell::new(Duration::ZERO),
                state: Cell::new(SharedElementViewAnimatingState::Inactive),
                ticker: SharedElementFrameTicker::new(),
                completion: RefCell::new(Some(completion)),
            }),
        }
    }

    pub fn state(&self) -> SharedElementViewAnimatingState {
        self.inner.state.get()
    }

    pub fn start_animation(&self, cx: &mut App) {
        let inner = &self.inner;
        inner.from.set(inner.view.as_ref().map_or(inner.to, |view| view.alpha(cx)));
        inner.started_at.set(RiseClock::monotonic());
        inner.state.set(SharedElementViewAnimatingState::Active);
        let this = self.clone();
        inner.ticker.start(move |cx: &mut App| this.tick(cx), cx);
    }

    fn tick(&self, cx: &mut App) {
        let inner = &self.inner;
        if inner.state.get() != SharedElementViewAnimatingState::Active {
            inner.ticker.stop();
            return;
        }
        let elapsed = RiseClock::monotonic().saturating_sub(inner.started_at.get()).as_secs_f64();
        let fraction = if inner.duration > 0.0 { (elapsed / inner.duration).min(1.0) } else { 1.0 };
        let progress = CubicBezier::EASE_OUT.progress(fraction) as f32;
        if let Some(view) = inner.view.as_ref() {
            view.set_alpha(SharedElementGeometry::lerp(inner.from.get(), inner.to, progress), cx);
        }
        if fraction >= 1.0 {
            self.end(cx);
        }
    }

    fn end(&self, cx: &mut App) {
        let inner = &self.inner;
        inner.ticker.stop();
        inner.state.set(SharedElementViewAnimatingState::Inactive);
        if let Some(view) = inner.view.as_ref() {
            view.set_alpha(inner.to, cx);
        }
        let completion = inner.completion.borrow_mut().take();
        if let Some(completion) = completion {
            completion(cx);
        }
    }

    /// true leaves the view where it is and drops the completion
    /// false leaves it stopped for finish_animation_at_end
    pub fn stop_animation(&self, without_finishing: bool) {
        let inner = &self.inner;
        inner.ticker.stop();
        if without_finishing {
            inner.state.set(SharedElementViewAnimatingState::Inactive);
            inner.completion.borrow_mut().take();
        } else {
            inner.state.set(SharedElementViewAnimatingState::Stopped);
        }
    }

    /// finishes a stopped animator at its end
    pub fn finish_animation_at_end(&self, cx: &mut App) {
        if self.inner.state.get() != SharedElementViewAnimatingState::Stopped {
            return;
        }
        self.end(cx);
    }
}

struct FrameTickerState {
    generation: Cell<u64>,
    running: Cell<bool>,
    on_tick: RefCell<Option<Box<dyn FnMut(&mut App)>>>,
}

/// display frame ticker that never retains its owner, ticks on the frame loop of the active window
/// invariant: on_tick runs with no borrow of the ticker held so it may call stop or start
pub struct SharedElementFrameTicker {
    inner: Rc<FrameTickerState>,
}

impl Default for SharedElementFrameTicker {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedElementFrameTicker {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(FrameTickerState {
                generation: Cell::new(0),
                running: Cell::new(false),
                on_tick: RefCell::new(None),
            }),
        }
    }

    pub fn is_running(&self) -> bool {
        self.inner.running.get()
    }

    pub fn start(&self, on_tick: impl FnMut(&mut App) + 'static, cx: &mut App) {
        self.stop();
        *self.inner.on_tick.borrow_mut() = Some(Box::new(on_tick));
        self.inner.running.set(true);
        Self::schedule(Rc::downgrade(&self.inner), self.inner.generation.get(), cx);
    }

    pub fn stop(&self) {
        let inner = &self.inner;
        inner.generation.set(inner.generation.get().wrapping_add(1));
        inner.running.set(false);
        let dropped = inner.on_tick.borrow_mut().take();
        drop(dropped);
    }

    fn schedule(state: Weak<FrameTickerState>, generation: u64, cx: &mut App) {
        let Some(window) = cx.active_window().or_else(|| cx.windows().into_iter().next()) else { return };
        let _ = window.update(cx, move |_, window, _| {
            window.on_next_frame(move |_, cx| Self::fire(state, generation, cx));
        });
    }

    fn fire(state: Weak<FrameTickerState>, generation: u64, cx: &mut App) {
        let Some(inner) = state.upgrade() else { return };
        if inner.generation.get() != generation {
            return;
        }
        let on_tick = inner.on_tick.borrow_mut().take();
        let Some(mut on_tick) = on_tick else { return };
        on_tick(cx);
        if inner.generation.get() != generation {
            return;
        }
        *inner.on_tick.borrow_mut() = Some(on_tick);
        Self::schedule(Rc::downgrade(&inner), generation, cx);
    }
}

/// forces a stuck transition to complete
/// guarded by its own arming token and not the epoch since
/// an account reset completes the context before bumping
pub struct SharedElementTransitionWatchdog {
    token: Rc<Cell<u64>>,
}

impl Default for SharedElementTransitionWatchdog {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedElementTransitionWatchdog {
    pub fn new() -> Self {
        Self { token: Rc::new(Cell::new(0)) }
    }

    pub fn arm(&self, on_fire: impl FnOnce(&mut App) + 'static, cx: &mut App) {
        self.token.set(self.token.get().wrapping_add(1));
        let armed = self.token.get();
        let token = Rc::downgrade(&self.token);
        let delay = duration_from_seconds(SharedElementMotion::WATCHDOG);
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(delay).await;
            let _ = cx.update(move |cx| {
                let Some(token) = token.upgrade() else { return };
                if token.get() != armed {
                    return;
                }
                token.set(token.get().wrapping_add(1));
                on_fire(cx);
            });
        })
        .detach();
    }

    pub fn cancel(&self) {
        self.token.set(self.token.get().wrapping_add(1));
    }
}
