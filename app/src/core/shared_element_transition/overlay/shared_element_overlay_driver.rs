use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{AnyWindowHandle, App, AppContext, Bounds, Entity, Pixels, Point, point, px};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::environment::reduce_motion;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentPolicy, SharedElementEndpoint, SharedElementGeometry, SharedElementGesturePolicy,
    SharedElementGesturePolicyAxis, SharedElementGesturePolicyDecision, SharedElementGesturePolicyInput,
    SharedElementKey, SharedElementPart, SharedElementReleaseOutcome, SharedElementSessionPolicy,
    SharedElementSessionPolicyLandingAction, SharedElementSessionPolicyPhase, SharedElementSurface,
};
use crate::core::shared_element_transition::navigation::shared_element_navigation_driver::{
    SharedElementFrameTicker, SharedElementTransitionSupport, SharedElementTransitionWatchdog,
};
use crate::core::shared_element_transition::overlay::shared_element_overlay_container::SharedElementOverlayContainerView;
use crate::core::shared_element_transition::overlay::shared_element_overlay_contracts::{
    SharedElementOverlayDestination, SharedElementOverlayOrigin, SharedElementOverlayPhase,
    SharedElementOverlayRegistry,
};
use crate::core::shared_element_transition::renderer::shared_element_flight_runner::{
    SharedElementFlightAlongside, SharedElementFlightContainer, SharedElementFlightRunner,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementReturnPlan, SharedElementTargetResolution,
};
use crate::core::shared_element_transition::runtime::shared_element_lift_board::SharedElementLiftBoard;
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;

const LOG_NAME: &str = "SharedElement";

/// what an overlay exit needs, gathered by the destination at dismiss
pub struct SharedElementOverlayExitRequest {
    pub destination: Rc<dyn SharedElementOverlayDestination>,
    pub origin_surface: SharedElementSurface,
    pub origin_item_id: String,
    pub capture: Option<SharedElementCapture>,
    pub window: AnyWindowHandle,
    pub touch_start: Option<Point<Pixels>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Direction {
    Open,
    Exit,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<SharedElementOverlayDriver>>> = const { RefCell::new(None) };
}

/// one flight between an origin and an overlay presentation that is not a route
/// one session at a time and never beside a navigation flight
/// every path ends in finish or force_end and both are idempotent
pub struct SharedElementOverlayDriver {
    pub epoch: u64,
    direction: Direction,
    interactive: bool,
    origin: Weak<dyn SharedElementOverlayOrigin>,
    origin_item_id: String,
    destination_surface: SharedElementSurface,
    destination: RefCell<Option<Weak<dyn SharedElementOverlayDestination>>>,
    container: Entity<SharedElementOverlayContainerView>,
    ticker: SharedElementFrameTicker,
    watchdog: SharedElementTransitionWatchdog,
    this: Weak<SharedElementOverlayDriver>,

    runner: RefCell<Option<SharedElementFlightRunner>>,
    capture: RefCell<Option<SharedElementCapture>>,
    source: Cell<Option<SharedElementEndpoint>>,
    target: Cell<Option<SharedElementEndpoint>>,
    plan: RefCell<Option<SharedElementReturnPlan>>,
    resolved_frame: Cell<Option<Bounds<Pixels>>>,
    is_resolution_unavailable: Cell<bool>,
    did_prepare: Cell<bool>,
    content_settled: Cell<bool>,
    last_read: Cell<Option<Bounds<Pixels>>>,
    ticks: Cell<usize>,
    is_waiting: Cell<bool>,
    is_lingering: Cell<bool>,
    linger_started_at: Cell<Duration>,
    readiness_started_at: Cell<Duration>,
    touch_start: Cell<Point<Pixels>>,
    release_outcome: Cell<Option<SharedElementReleaseOutcome>>,
    flew: Cell<bool>,
    is_finished: Cell<bool>,
}

impl SharedElementOverlayDriver {
    const REMOVAL_DELAY: f64 = 0.05;
    const COMMIT_FRAMES: usize = 2;

    #[allow(clippy::too_many_arguments)]
    fn new(
        epoch: u64,
        direction: Direction,
        interactive: bool,
        origin: &Rc<dyn SharedElementOverlayOrigin>,
        origin_item_id: String,
        destination_surface: SharedElementSurface,
        destination: Option<&Rc<dyn SharedElementOverlayDestination>>,
        cx: &mut App,
    ) -> Rc<Self> {
        let container = cx.new(SharedElementOverlayContainerView::new);
        Rc::new_cyclic(|this| Self {
            epoch,
            direction,
            interactive,
            origin: Rc::downgrade(origin),
            origin_item_id,
            destination_surface,
            destination: RefCell::new(destination.map(Rc::downgrade)),
            container,
            ticker: SharedElementFrameTicker::new(),
            watchdog: SharedElementTransitionWatchdog::new(),
            this: this.clone(),
            runner: RefCell::new(None),
            capture: RefCell::new(None),
            source: Cell::new(None),
            target: Cell::new(None),
            plan: RefCell::new(None),
            resolved_frame: Cell::new(None),
            is_resolution_unavailable: Cell::new(false),
            did_prepare: Cell::new(false),
            content_settled: Cell::new(false),
            last_read: Cell::new(None),
            ticks: Cell::new(0),
            is_waiting: Cell::new(false),
            is_lingering: Cell::new(false),
            linger_started_at: Cell::new(Duration::ZERO),
            readiness_started_at: Cell::new(Duration::ZERO),
            touch_start: Cell::new(Point::default()),
            release_outcome: Cell::new(None),
            flew: Cell::new(false),
            is_finished: Cell::new(false),
        })
    }

    fn current() -> Option<Rc<Self>> {
        CURRENT.with(|current| current.borrow().clone())
    }

    fn set_current(driver: Option<Rc<Self>>) {
        CURRENT.with(|current| *current.borrow_mut() = driver);
    }

    fn origin(&self) -> Option<Rc<dyn SharedElementOverlayOrigin>> {
        self.origin.upgrade()
    }

    fn destination(&self) -> Option<Rc<dyn SharedElementOverlayDestination>> {
        self.destination.borrow().as_ref().and_then(Weak::upgrade)
    }

    fn runner(&self) -> Option<SharedElementFlightRunner> {
        self.runner.borrow().clone()
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

    fn arm_watchdog(&self, cx: &mut App) {
        self.watchdog.arm(self.weak(|this, cx| this.force_end(cx)), cx);
    }

    fn after(&self, delay: f64, cx: &mut App, work: impl Fn(&Rc<Self>, &mut App) + 'static) {
        SharedElementTransitionSupport::after(delay, self.epoch, self.weak(work), cx);
    }

    pub fn is_running_epoch(epoch: u64) -> bool {
        let Some(current) = Self::current() else { return false };
        current.epoch == epoch && !current.is_finished.get()
    }

    pub fn is_running_destination_surface(destination_surface: &SharedElementSurface) -> bool {
        let Some(current) = Self::current() else { return false };
        current.destination_surface == *destination_surface && !current.is_finished.get()
    }

    /// ends the session of a destination that left the screen
    pub fn abort(destination_surface: &SharedElementSurface, cx: &mut App) {
        let Some(current) = Self::current() else { return };
        if current.destination_surface != *destination_surface {
            return;
        }
        current.force_end(cx);
    }

    pub fn return_did_become_resumable(epoch: u64, cx: &mut App) {
        let Some(driver) = Self::current() else { return };
        if driver.epoch != epoch || driver.is_finished.get() {
            return;
        }
        driver.resume_return(cx);
    }

    pub fn window_did_resize(cx: &mut App) {
        let Some(driver) = Self::current() else { return };
        if driver.is_finished.get() {
            return;
        }
        match driver.direction {
            Direction::Open => driver.retarget_open(cx),
            Direction::Exit => {
                let measured = driver
                    .origin()
                    .and_then(|origin| origin.measured_target_frame(&driver.origin_item_id, cx));
                if let Some(measured) = measured {
                    driver.retarget_exit(measured, cx);
                }
            }
        }
    }

    /// begins an open flight from the origin into the destination surface
    /// none when nothing was armed and then the presentation keeps its own entrance
    pub fn open(
        origin: &Rc<dyn SharedElementOverlayOrigin>,
        item_id: &str,
        destination_surface: SharedElementSurface,
        cx: &mut App,
    ) -> Option<u64> {
        if !SharedElementNavigationPolicy::stories_enabled() || Self::current().is_some() {
            return None;
        }
        let window = origin.overlay_window(cx)?;
        let surfaces: HashSet<SharedElementSurface> =
            [origin.surface(), destination_surface.clone()].into_iter().collect();
        let epoch = SharedElementTransitionCoordinator::shared(cx)
            .update(cx, |coordinator, cx| coordinator.begin_overlay_session(item_id, surfaces, false, cx))?;
        let capture = if reduce_motion(cx) { None } else { origin.open_capture(item_id, cx) };
        let driver = Self::new(
            epoch,
            Direction::Open,
            false,
            origin,
            item_id.to_string(),
            destination_surface,
            None,
            cx,
        );
        Self::set_current(Some(driver.clone()));
        driver.start_open(window, capture, cx);
        Some(epoch)
    }

    fn start_open(&self, window: AnyWindowHandle, capture: Option<SharedElementCapture>, cx: &mut App) {
        self.attach_hooks(cx);
        self.arm_watchdog(cx);
        self.container.update(cx, |container, cx| {
            container.install(window, 0.0, cx);
            container.fade_curtain(1.0, SharedElementMotion::OVERLAY_CURTAIN_FADE, cx);
        });
        if let Some(capture) = capture {
            let corner_radius = self
                .origin()
                .map(|origin| origin.overlay_corner_radius(&self.origin_item_id, cx))
                .unwrap_or(0.0);
            let source_key = capture.source_key.clone();
            self.begin_runner(capture, corner_radius, cx);
            let epoch = self.epoch;
            SharedElementLiftBoard::shared(cx)
                .update(cx, |board, cx| board.lift(HashSet::from([source_key]), epoch, cx));
        }
        self.is_waiting.set(true);
        self.after(0.0, cx, |this, cx| {
            if !this.is_waiting.get() || this.is_finished.get() {
                return;
            }
            this.read_open_target(cx);
            if !this.is_waiting.get() {
                return;
            }
            this.ticker.start(
                this.weak(|this, cx| {
                    this.ticks.set(this.ticks.get() + 1);
                    this.read_open_target(cx);
                }),
                cx,
            );
        });
    }

    fn read_open_target(&self, cx: &mut App) {
        if !self.is_waiting.get() || self.is_finished.get() {
            return;
        }
        let found = self.destination().or_else(|| {
            SharedElementOverlayRegistry::shared(cx).read(cx).destination(&self.destination_surface)
        });
        let frame = found.as_ref().and_then(|found| found.media_frame(cx));
        if let (Some(found), Some(frame), Some(last_read)) = (&found, frame, self.last_read.get())
            && SharedElementGeometry::max_edge_delta(frame, last_read) <= SharedElementMotion::OPEN_TARGET_STABLE_DELTA
        {
            self.stop_waiting();
            *self.destination.borrow_mut() = Some(Rc::downgrade(found));
            self.fly_open(frame, found.clone(), cx);
            return;
        }
        self.last_read.set(frame);
        if self.ticks.get() >= SharedElementMotion::OPEN_TARGET_WAIT_TICKS {
            self.stop_waiting();
            *self.destination.borrow_mut() = found.as_ref().map(Rc::downgrade);
            self.open_fallback(cx);
        }
    }

    fn fly_open(&self, clip: Bounds<Pixels>, destination: Rc<dyn SharedElementOverlayDestination>, cx: &mut App) {
        let capture = self.capture.borrow().clone();
        let (Some(runner), Some(capture)) = (self.runner().filter(SharedElementFlightRunner::has_begun), capture)
        else {
            self.open_fallback(cx);
            return;
        };
        let mode = destination.open_landing_mode(SharedElementTransitionSupport::content_aspect(&capture), clip, cx);
        let target = SharedElementTransitionSupport::open_landing(&capture, clip, mode);
        self.target.set(Some(target));
        self.arm_watchdog(cx);
        runner.animate(
            target,
            None,
            None,
            Box::new(self.weak(|this, cx| this.open_move_did_finish(cx))),
            cx,
        );
        self.after(SharedElementMotion::RETARGET_CHECK_DELAY, cx, |this, cx| this.retarget_open(cx));
    }

    fn retarget_open(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        let capture = self.capture.borrow().clone();
        let (Some(runner), Some(capture), Some(target), Some(destination)) =
            (self.runner(), capture, self.target.get(), self.destination())
        else {
            return;
        };
        let Some(fresh) = destination.media_frame(cx) else { return };
        if SharedElementGeometry::max_edge_delta(fresh, target.clip) <= SharedElementMotion::LANDING_MISMATCH {
            return;
        }
        let mode = destination.open_landing_mode(SharedElementTransitionSupport::content_aspect(&capture), fresh, cx);
        let updated = SharedElementTransitionSupport::open_landing(&capture, fresh, mode);
        self.target.set(Some(updated));
        runner.retarget(updated, cx);
    }

    fn open_move_did_finish(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        self.arm_watchdog(cx);
        self.linger_started_at.set(RiseClock::monotonic());
        self.is_lingering.set(true);
        self.check_linger(cx);
        if !self.is_lingering.get() {
            return;
        }
        self.ticker.start(self.weak(|this, cx| this.check_linger(cx)), cx);
    }

    fn check_linger(&self, cx: &mut App) {
        if !self.is_lingering.get() || self.is_finished.get() {
            return;
        }
        let mut ready = true;
        if let Some(destination) = self.destination()
            && let Some(item_id) = destination.current_item_id(cx)
        {
            ready = destination.media_is_ready(&item_id, cx);
        }
        let held = RiseClock::monotonic().saturating_sub(self.linger_started_at.get()).as_secs_f64();
        if !(ready || held >= SharedElementMotion::FULL_IMAGE_HOLD) {
            return;
        }
        self.is_lingering.set(false);
        self.ticker.stop();
        self.land_open(cx);
    }

    fn land_open(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        if let Some(destination) = self.destination() {
            destination.apply_overlay_phase(SharedElementOverlayPhase::OpenLanded, self.epoch, cx);
        }
        let epoch = self.epoch;
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.land(epoch, cx));
        self.after_frames(cx, |this, cx| {
            if this.is_finished.get() {
                return;
            }
            this.container.update(cx, |container, cx| container.set_curtain_alpha(0.0, cx));
            let Some(runner) = this.runner().filter(SharedElementFlightRunner::has_begun) else {
                this.finish(cx);
                return;
            };
            runner.fade_out(
                SharedElementMotion::FULL_IMAGE_CROSSFADE,
                1.0,
                None,
                Box::new(this.weak(|this, cx| this.finish(cx))),
                cx,
            );
        });
    }

    /// no capture or no destination in time or reduce motion, the presentation fades in
    fn open_fallback(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        self.arm_watchdog(cx);
        let source_key = self.capture.borrow().as_ref().map(|capture| capture.source_key.clone());
        if let Some(source_key) = source_key {
            let epoch = self.epoch;
            SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.unlift(&source_key, epoch, cx));
        }
        let Some(destination) = self.destination() else {
            self.fade_out_open(cx);
            return;
        };
        destination.apply_overlay_phase(SharedElementOverlayPhase::OpenFallback, self.epoch, cx);
        self.after_frames(cx, |this, cx| this.fade_out_open(cx));
    }

    fn fade_out_open(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        self.container.update(cx, |container, cx| {
            container.fade_curtain(0.0, SharedElementMotion::OPEN_FALLBACK_FADE, cx);
        });
        if let Some(runner) = self.runner().filter(SharedElementFlightRunner::has_begun) {
            runner.fade_out(
                SharedElementMotion::OPEN_FALLBACK_FADE,
                SharedElementMotion::OPEN_FALLBACK_SCALE,
                None,
                Box::new(self.weak(|this, cx| this.finish(cx))),
                cx,
            );
        } else {
            self.after(SharedElementMotion::OPEN_FALLBACK_FADE, cx, |this, cx| this.finish(cx));
        }
    }

    pub fn begin_exit(request: SharedElementOverlayExitRequest, cx: &mut App) -> Option<Rc<Self>> {
        let interactive = request.touch_start.is_some();
        let reduce_motion = reduce_motion(cx);
        let refuse = |request: &SharedElementOverlayExitRequest| {
            if let Some(readiness) = request.capture.as_ref().and_then(|capture| capture.live_readiness.as_ref()) {
                readiness.cancel();
            }
            None
        };
        if !SharedElementNavigationPolicy::stories_enabled()
            || (interactive
                && !(SharedElementNavigationPolicy::stories_interactive_enabled()
                    && !reduce_motion
                    && request.capture.is_some()))
            || Self::current().is_some()
        {
            return refuse(&request);
        }
        let coordinator = SharedElementTransitionCoordinator::shared(cx);
        if coordinator.read(cx).phase() != SharedElementSessionPolicyPhase::Idle {
            return refuse(&request);
        }
        let Some(origin) = coordinator
            .read(cx)
            .origin(&request.origin_surface, cx)
            .and_then(|origin| origin.as_overlay_origin())
        else {
            return refuse(&request);
        };
        if !origin.can_return(&request.origin_item_id, cx) {
            return refuse(&request);
        }
        let surfaces: HashSet<SharedElementSurface> =
            [origin.surface(), request.destination.surface()].into_iter().collect();
        let item_id = request.origin_item_id.clone();
        let Some(epoch) = coordinator
            .update(cx, |coordinator, cx| coordinator.begin_overlay_session(&item_id, surfaces, interactive, cx))
        else {
            return refuse(&request);
        };
        let driver = Self::new(
            epoch,
            Direction::Exit,
            interactive,
            &origin,
            request.origin_item_id.clone(),
            request.destination.surface(),
            Some(&request.destination),
            cx,
        );
        Self::set_current(Some(driver.clone()));
        let capture = if reduce_motion { None } else { request.capture.clone() };
        if capture.is_none()
            && let Some(readiness) = request.capture.as_ref().and_then(|capture| capture.live_readiness.as_ref())
        {
            readiness.cancel();
        }
        driver.touch_start.set(request.touch_start.unwrap_or_default());
        driver.start_exit(request.window, capture, cx);
        Some(driver)
    }

    fn start_exit(&self, window: AnyWindowHandle, capture: Option<SharedElementCapture>, cx: &mut App) {
        self.attach_hooks(cx);
        self.container.update(cx, |container, cx| container.install(window, 1.0, cx));
        if let Some(capture) = capture {
            let cover = self.origin().and_then(|origin| origin.return_cover(&self.origin_item_id, cx));
            let flying_identity = capture.flying_identity.clone();
            self.begin_runner(capture, 0.0, cx);
            if let Some(cover) = cover
                && SharedElementContentPolicy::needs_cover(flying_identity.as_deref(), Some(cover.identity.as_str()))
                && let Some(runner) = self.runner()
            {
                runner.set_cover(Some(cover), cx);
            }
            let keys = self.origin_keys();
            let epoch = self.epoch;
            SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.lift(keys, epoch, cx));
        }
        if let Some(destination) = self.destination() {
            destination.apply_overlay_phase(SharedElementOverlayPhase::ExitBegan, self.epoch, cx);
        }
        if !self.interactive {
            self.arm_watchdog(cx);
        }
        self.readiness_started_at.set(RiseClock::monotonic());
        self.after(0.0, cx, |this, cx| {
            if this.is_finished.get() {
                return;
            }
            if !this.did_prepare.get() {
                this.prepare_and_resolve(cx);
            }
            if this.interactive {
                return;
            }
            this.is_waiting.set(true);
            this.check_exit_readiness(cx);
            if !this.is_waiting.get() {
                return;
            }
            this.ticker.start(this.weak(|this, cx| this.check_exit_readiness(cx)), cx);
        });
    }

    fn origin_keys(&self) -> HashSet<SharedElementKey> {
        let Some(origin) = self.origin() else { return HashSet::new() };
        HashSet::from([SharedElementKey::new(origin.surface(), self.origin_item_id.clone(), SharedElementPart::Media)])
    }

    fn unlift_origin_keys(&self, cx: &mut App) {
        let keys = self.origin_keys();
        let epoch = self.epoch;
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| {
            for key in &keys {
                board.unlift(key, epoch, cx);
            }
        });
    }

    fn prepare_and_resolve(&self, cx: &mut App) {
        self.did_prepare.set(true);
        let Some(origin) = self.origin() else {
            self.is_resolution_unavailable.set(true);
            return;
        };
        let plan = origin.prepare_return(&self.origin_item_id, self.epoch, cx);
        *self.plan.borrow_mut() = plan;
        self.resolve_target(cx);
    }

    fn resolve_target(&self, cx: &mut App) {
        if self.resolved_frame.get().is_some() || self.is_resolution_unavailable.get() {
            return;
        }
        let has_plan = self.plan.borrow().is_some();
        let Some(origin) = self.origin().filter(|_| has_plan) else {
            self.is_resolution_unavailable.set(true);
            return;
        };
        match origin.resolve_return(self.epoch, cx) {
            SharedElementTargetResolution::Ready(frame) => self.resolved_frame.set(Some(frame)),
            SharedElementTargetResolution::Deferred => {}
            SharedElementTargetResolution::Unavailable => self.is_resolution_unavailable.set(true),
        }
    }

    fn resume_return(&self, cx: &mut App) {
        if self.is_waiting.get() {
            self.check_exit_readiness(cx);
        } else if self.interactive && self.release_outcome.get() != Some(SharedElementReleaseOutcome::Cancel) {
            let had_frame = self.resolved_frame.get().is_some();
            self.resolve_target(cx);
            if !had_frame
                && self.flew.get()
                && let Some(frame) = self.resolved_frame.get()
            {
                self.retarget_exit(frame, cx);
            }
        }
    }

    fn check_exit_readiness(&self, cx: &mut App) {
        if !self.is_waiting.get() || self.is_finished.get() {
            return;
        }
        self.resolve_target(cx);
        if self.is_resolution_unavailable.get() {
            self.stop_waiting();
            self.settle_content(true, cx);
            self.exit_fallback(cx);
            return;
        }
        let content_ready = self.settle_content(false, cx);
        if let Some(resolved_frame) = self.resolved_frame.get()
            && content_ready
        {
            self.stop_waiting();
            self.fly_exit(resolved_frame, cx);
            return;
        }
        let waited = RiseClock::monotonic().saturating_sub(self.readiness_started_at.get()).as_secs_f64();
        if waited >= SharedElementMotion::EXIT_RESOLVE_TIMEOUT {
            self.stop_waiting();
            self.settle_content(true, cx);
            self.exit_fallback(cx);
        }
    }

    /// true once the flying content is final, either video or frame is shown or a miss got the placeholder
    fn settle_content(&self, force: bool, cx: &mut App) -> bool {
        if self.content_settled.get() {
            return true;
        }
        let (readiness, placeholder) = {
            let capture = self.capture.borrow();
            (
                capture.as_ref().and_then(|capture| capture.live_readiness.clone()),
                capture.as_ref().and_then(|capture| capture.placeholder.clone()),
            )
        };
        let Some(readiness) = readiness else {
            self.content_settled.set(true);
            return true;
        };
        if readiness.is_ready() {
            self.content_settled.set(true);
            return true;
        }
        if readiness.is_pending() && !force && !readiness.is_expired(RiseClock::monotonic()) {
            return false;
        }
        readiness.cancel();
        if let Some(placeholder) = placeholder
            && let Some(runner) = self.runner()
        {
            runner.reveal_placeholder(placeholder, cx);
        }
        self.content_settled.set(true);
        true
    }

    fn exit_landing(&self, clip: Bounds<Pixels>, cx: &App) -> Option<SharedElementEndpoint> {
        let capture = self.capture.borrow();
        let (Some(capture), Some(source)) = (capture.as_ref(), self.source.get()) else { return None };
        let corner_radius = self
            .origin()
            .map(|origin| origin.overlay_corner_radius(&self.origin_item_id, cx))
            .unwrap_or(0.0);
        Some(SharedElementTransitionSupport::return_landing(capture, &source, clip, corner_radius))
    }

    /// curtain follows the move from wherever its alpha is now
    fn curtain_alongside(&self, to: f32, cx: &App) -> SharedElementFlightAlongside {
        let from = self.container.read(cx).presented_curtain_alpha(RiseClock::monotonic());
        let container = self.container.downgrade();
        Rc::new(move |progress: f32, cx: &mut App| {
            let _ = container.update(cx, |container, cx| {
                container.set_curtain_alpha(SharedElementGeometry::lerp(from, to, progress), cx);
            });
        })
    }

    fn fly_exit(&self, frame: Bounds<Pixels>, cx: &mut App) {
        let (Some(runner), Some(target)) =
            (self.runner().filter(SharedElementFlightRunner::has_begun), self.exit_landing(frame, cx))
        else {
            self.exit_fallback(cx);
            return;
        };
        self.target.set(Some(target));
        self.flew.set(true);
        self.arm_watchdog(cx);
        let alongside = self.curtain_alongside(0.0, cx);
        runner.animate(
            target,
            Some(SharedElementMotion::COVER_FADE_START_PROGRESS),
            Some(alongside),
            Box::new(self.weak(|this, cx| this.exit_move_did_finish(cx))),
            cx,
        );
        self.schedule_exit_retarget(cx);
    }

    fn schedule_exit_retarget(&self, cx: &mut App) {
        self.after(SharedElementMotion::RETARGET_CHECK_DELAY, cx, |this, cx| {
            if this.is_finished.get() {
                return;
            }
            let measured = this.origin().and_then(|origin| origin.measured_target_frame(&this.origin_item_id, cx));
            if let Some(measured) = measured {
                this.retarget_exit(measured, cx);
            }
        });
    }

    fn retarget_exit(&self, frame: Bounds<Pixels>, cx: &mut App) {
        if self.is_finished.get() || !self.flew.get() {
            return;
        }
        let (Some(runner), Some(target)) = (self.runner(), self.target.get()) else { return };
        if SharedElementGeometry::max_edge_delta(frame, target.clip) <= SharedElementMotion::LANDING_MISMATCH {
            return;
        }
        let Some(updated) = self.exit_landing(frame, cx) else { return };
        self.target.set(Some(updated));
        runner.retarget(updated, cx);
    }

    fn exit_move_did_finish(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        if self.flew.get()
            && let (Some(target), Some(runner)) = (self.target.get(), self.runner())
        {
            let measured = self.origin().and_then(|origin| origin.measured_target_frame(&self.origin_item_id, cx));
            let mismatch = measured
                .map(|measured| SharedElementGeometry::max_edge_delta(measured, target.clip))
                .unwrap_or(f32::INFINITY);
            if SharedElementSessionPolicy::landing(mismatch, SharedElementMotion::LANDING_MISMATCH)
                == SharedElementSessionPolicyLandingAction::CrossfadeReveal
            {
                self.unlift_origin_keys(cx);
                runner.fade_out(
                    SharedElementMotion::LANDING_CROSSFADE,
                    1.0,
                    None,
                    Box::new(self.weak(|this, cx| this.complete_exit(cx))),
                    cx,
                );
                return;
            }
        }
        self.complete_exit(cx);
    }

    /// fallback inside the session, curtain and flight fade out and show the origin wherever it is
    fn exit_fallback(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        self.arm_watchdog(cx);
        self.flew.set(false);
        self.unlift_origin_keys(cx);
        self.container.update(cx, |container, cx| {
            container.fade_curtain(0.0, SharedElementMotion::EXIT_FALLBACK_FADE, cx);
        });
        if let Some(runner) = self.runner().filter(SharedElementFlightRunner::has_begun) {
            runner.fade_out(
                SharedElementMotion::EXIT_FALLBACK_FADE,
                SharedElementMotion::EXIT_FALLBACK_SCALE,
                None,
                Box::new(self.weak(|this, cx| this.complete_exit(cx))),
                cx,
            );
        } else {
            self.after(SharedElementMotion::EXIT_FALLBACK_FADE, cx, |this, cx| this.complete_exit(cx));
        }
    }

    fn complete_exit(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        if self.plan.borrow().is_some()
            && let Some(origin) = self.origin()
        {
            origin.finish_return(self.epoch, cx);
        }
        if let Some(destination) = self.destination() {
            destination.apply_overlay_phase(SharedElementOverlayPhase::ExitFinished, self.epoch, cx);
        }
        self.finish(cx);
    }

    /// one pointer sample, writes the flight and curtain once and nothing else
    pub fn follow(&self, translation: Point<Pixels>, velocity: Point<Pixels>, cx: &mut App) {
        if !self.interactive || self.is_finished.get() || self.release_outcome.get().is_some() {
            return;
        }
        let (Some(runner), Some(source)) = (self.runner().filter(SharedElementFlightRunner::has_begun), self.source.get())
        else {
            return;
        };
        self.settle_content(false, cx);
        let frame = SharedElementGesturePolicy::sample(&self.input(translation, velocity, source, cx));
        runner.follow(frame, source, cx);
        self.container.update(cx, |container, cx| container.set_curtain_alpha(frame.chrome_alpha, cx));
    }

    pub fn release(&self, translation: Point<Pixels>, velocity: Point<Pixels>, cx: &mut App) {
        if !self.interactive || self.is_finished.get() || self.release_outcome.get().is_some() {
            return;
        }
        self.arm_watchdog(cx);
        let Some(source) = self.source.get() else {
            self.release_outcome.set(Some(SharedElementReleaseOutcome::Cancel));
            self.cancel_did_finish(cx);
            return;
        };
        self.settle_content(true, cx);
        match SharedElementGesturePolicy::decision(&self.input(translation, velocity, source, cx)) {
            SharedElementGesturePolicyDecision::Finish => {
                self.release_outcome.set(Some(SharedElementReleaseOutcome::Finish));
                self.finish_interactive(velocity, cx);
            }
            SharedElementGesturePolicyDecision::Cancel => {
                self.release_outcome.set(Some(SharedElementReleaseOutcome::Cancel));
                self.cancel_interactive(velocity, cx);
            }
        }
    }

    pub fn cancel_interaction(&self, cx: &mut App) {
        if !self.interactive || self.is_finished.get() || self.release_outcome.get().is_some() {
            return;
        }
        self.arm_watchdog(cx);
        self.release_outcome.set(Some(SharedElementReleaseOutcome::Cancel));
        self.cancel_interactive(point(px(0.0), px(0.0)), cx);
    }

    fn finish_interactive(&self, velocity: Point<Pixels>, cx: &mut App) {
        if self.did_prepare.get() {
            self.resolve_target(cx);
        } else {
            self.prepare_and_resolve(cx);
        }
        let predicted = self.plan.borrow().as_ref().map(|plan| plan.predicted_frame);
        let landing = self.resolved_frame.get().or(predicted).and_then(|frame| self.exit_landing(frame, cx));
        let (Some(runner), Some(target)) = (self.runner().filter(SharedElementFlightRunner::has_begun), landing) else {
            self.exit_fallback(cx);
            return;
        };
        self.target.set(Some(target));
        self.flew.set(true);
        let alongside = self.curtain_alongside(0.0, cx);
        runner.release(
            SharedElementReleaseOutcome::Finish,
            velocity,
            target,
            Some(alongside),
            Box::new(self.weak(|this, cx| this.exit_move_did_finish(cx))),
            cx,
        );
        self.schedule_exit_retarget(cx);
    }

    fn cancel_interactive(&self, velocity: Point<Pixels>, cx: &mut App) {
        let (Some(runner), Some(source)) = (self.runner().filter(SharedElementFlightRunner::has_begun), self.source.get())
        else {
            self.cancel_did_finish(cx);
            return;
        };
        let alongside = self.curtain_alongside(1.0, cx);
        runner.release(
            SharedElementReleaseOutcome::Cancel,
            velocity,
            source,
            Some(alongside),
            Box::new(self.weak(|this, cx| this.cancel_did_finish(cx))),
            cx,
        );
    }

    /// origin is put back behind the opaque curtain before the destination returns
    fn cancel_did_finish(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        self.container.update(cx, |container, cx| container.set_curtain_alpha(1.0, cx));
        if self.plan.borrow().is_some()
            && let Some(origin) = self.origin()
        {
            origin.cancel_return(self.epoch, cx);
        }
        if let Some(destination) = self.destination() {
            destination.apply_overlay_phase(SharedElementOverlayPhase::ExitCancelled, self.epoch, cx);
        }
        self.finish(cx);
    }

    fn input(
        &self,
        translation: Point<Pixels>,
        velocity: Point<Pixels>,
        source: SharedElementEndpoint,
        cx: &App,
    ) -> SharedElementGesturePolicyInput {
        SharedElementGesturePolicyInput {
            translation,
            velocity,
            touch_start: self.touch_start.get(),
            source: source.clip,
            viewport: self.container.read(cx).bounds_size(),
            layout_sign: 1.0,
            axis: SharedElementGesturePolicyAxis::Vertical,
        }
    }

    fn begin_runner(&self, capture: SharedElementCapture, corner_radius: f32, cx: &mut App) {
        let source = SharedElementEndpoint {
            clip: capture.clip,
            content: capture.content_rect,
            corner_radius,
            mode: capture.mode,
        };
        self.source.set(Some(source));
        let container: Rc<dyn SharedElementFlightContainer> = Rc::new(self.container.downgrade());
        let runner = SharedElementFlightRunner::new(container, self.epoch, cx);
        *self.runner.borrow_mut() = Some(runner.clone());
        if let Some(placeholder) = capture.placeholder.clone() {
            runner.flight_view().update(cx, |flight_view, cx| flight_view.set_placeholder(placeholder, cx));
        }
        runner.begin(capture.content.clone(), source, capture.backdrop.clone(), cx);
        *self.capture.borrow_mut() = Some(capture);
    }

    fn attach_hooks(&self, cx: &mut App) {
        let epoch = self.epoch;
        let on_resign_active = self.weak(|this, cx| this.application_will_resign_active(cx));
        let on_force_end = self.weak(|this, cx| this.force_end(cx));
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, _| {
            coordinator.attach_transition_hooks(epoch, Some(on_resign_active), on_force_end);
        });
    }

    fn application_will_resign_active(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        match self.direction {
            Direction::Open => {
                if self.is_waiting.get() || self.is_lingering.get() {
                    self.force_end(cx);
                } else if let Some(runner) = self.runner() {
                    runner.jump_to_end(cx);
                }
            }
            Direction::Exit => {
                if self.is_waiting.get() {
                    self.stop_waiting();
                    self.settle_content(true, cx);
                    self.exit_fallback(cx);
                } else if (!self.interactive || self.release_outcome.get().is_some())
                    && let Some(runner) = self.runner()
                {
                    runner.jump_to_end(cx);
                }
            }
        }
    }

    /// for the watchdog, account reset and a destination that left the screen
    /// after a reset the session is gone already so the end call is a no-op
    fn force_end(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        self.stop_waiting();
        self.is_lingering.set(false);
        let readiness = self.capture.borrow().as_ref().and_then(|capture| capture.live_readiness.clone());
        if let Some(readiness) = readiness {
            readiness.cancel();
        }
        if let Some(runner) = self.runner() {
            runner.abort(cx);
        }
        match self.direction {
            Direction::Open => {
                if let Some(destination) = self.destination() {
                    destination.apply_overlay_phase(SharedElementOverlayPhase::OpenAborted, self.epoch, cx);
                }
            }
            Direction::Exit => {
                let restores =
                    self.interactive && self.release_outcome.get() != Some(SharedElementReleaseOutcome::Finish);
                if self.plan.borrow().is_some()
                    && let Some(origin) = self.origin()
                {
                    if restores {
                        origin.cancel_return(self.epoch, cx);
                    } else {
                        origin.finish_return(self.epoch, cx);
                    }
                }
                if let Some(destination) = self.destination() {
                    let phase = if restores {
                        SharedElementOverlayPhase::ExitCancelled
                    } else {
                        SharedElementOverlayPhase::ExitFinished
                    };
                    destination.apply_overlay_phase(phase, self.epoch, cx);
                }
            }
        }
        self.container.update(cx, |container, cx| container.remove_now(cx));
        self.end_session(cx);
    }

    fn finish(&self, cx: &mut App) {
        if self.is_finished.get() {
            return;
        }
        let runner = self.runner();
        let container = self.container.clone();
        self.end_session(cx);
        let delay = Duration::from_secs_f64(Self::REMOVAL_DELAY);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(delay).await;
            cx.update(|cx| {
                if let Some(runner) = &runner {
                    runner.remove_after_commit(cx);
                }
                cx.defer(move |cx| container.update(cx, |container, cx| container.remove_now(cx)));
            });
        })
        .detach();
    }

    fn end_session(&self, cx: &mut App) {
        self.is_finished.set(true);
        self.stop_waiting();
        self.is_lingering.set(false);
        self.ticker.stop();
        self.watchdog.cancel();
        let epoch = self.epoch;
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, cx| {
            coordinator.detach_transition_hooks(epoch);
            coordinator.end_overlay_session(epoch, cx);
        });
        let flew = self.flew.get();
        Sai::log(LOG_NAME, move || format!("overlay driver end epoch={epoch} flew={flew}"), SaiLogKind::Ui, cx);
        let is_current = Self::current().is_some_and(|current| std::ptr::eq(Rc::as_ptr(&current), self));
        if is_current {
            Self::set_current(None);
        }
    }

    fn stop_waiting(&self) {
        self.is_waiting.set(false);
        self.ticker.stop();
    }

    /// runs work after the presentation had COMMIT_FRAMES frames to draw a phase
    fn after_frames(&self, cx: &mut App, work: impl Fn(&Rc<Self>, &mut App) + 'static) {
        let remaining = Cell::new(Self::COMMIT_FRAMES);
        self.ticker.start(
            self.weak(move |this, cx| {
                remaining.set(remaining.get().saturating_sub(1));
                if remaining.get() > 0 {
                    return;
                }
                this.ticker.stop();
                work(this, cx);
            }),
            cx,
        );
    }
}
