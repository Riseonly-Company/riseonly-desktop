use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{App, Bounds, Pixels};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementEndpoint, SharedElementGeometry, SharedElementKey, SharedElementPart,
};
use crate::core::shared_element_transition::navigation::shared_element_navigation_driver::{
    SharedElementFrameTicker, SharedElementNavigationDriver, SharedElementTransitionSupport,
    SharedElementTransitionWatchdog,
};
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::{
    SharedElementTransitionContextKey, SharedElementTransitionView, SharedElementUserInputWatch,
    SharedElementViewControllerAnimatedTransitioning, SharedElementViewControllerContextTransitioning,
};
use crate::core::shared_element_transition::renderer::shared_element_flight_runner::{
    SharedElementFlightAlongside, SharedElementFlightRunner,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementDestinationContainer, SharedElementFlightContent,
};
use crate::core::shared_element_transition::runtime::shared_element_lift_board::SharedElementLiftBoard;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::{
    SharedElementIntent, SharedElementTransitionCoordinator,
};
use crate::core::ui::clever_image_disk_cache::CleverImageBitmapLoader;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;

const LOG_NAME: &str = "SharedElement";

type TransitionContext = Rc<dyn SharedElementViewControllerContextTransitioning>;

/// grid tap to list, the cell content flies onto the row box while the list fades in
pub struct SharedElementOpenAnimator {
    intent: Rc<SharedElementIntent>,
    epoch: u64,
    watchdog: SharedElementTransitionWatchdog,
    ticker: SharedElementFrameTicker,
    this: Weak<SharedElementOpenAnimator>,

    context: RefCell<Option<TransitionContext>>,
    from_view: RefCell<Option<SharedElementTransitionView>>,
    to_view: RefCell<Option<SharedElementTransitionView>>,
    destination: RefCell<Option<Weak<dyn SharedElementDestinationContainer>>>,
    runner: RefCell<Option<SharedElementFlightRunner>>,
    capture: RefCell<Option<SharedElementCapture>>,
    list_key: RefCell<Option<SharedElementKey>>,
    destination_keys: RefCell<HashSet<SharedElementKey>>,
    target: Cell<Option<SharedElementEndpoint>>,
    did_resolve_destination: Cell<bool>,
    last_target_read: Cell<Option<Bounds<Pixels>>>,
    ticks: Cell<usize>,
    is_waiting_for_target: Cell<bool>,
    is_completed: Cell<bool>,
    is_lingering: Cell<bool>,
    linger_touch_watch: RefCell<Option<SharedElementUserInputWatch>>,
    linger_list_frame: Cell<Option<Bounds<Pixels>>>,
}

impl SharedElementOpenAnimator {
    const LINGER_POLL_INTERVAL: f64 = 0.1;

    pub fn new(intent: Rc<SharedElementIntent>, _cx: &mut App) -> Rc<Self> {
        let epoch = intent.epoch;
        Rc::new_cyclic(|this| Self {
            intent,
            epoch,
            watchdog: SharedElementTransitionWatchdog::new(),
            ticker: SharedElementFrameTicker::new(),
            this: this.clone(),
            context: RefCell::new(None),
            from_view: RefCell::new(None),
            to_view: RefCell::new(None),
            destination: RefCell::new(None),
            runner: RefCell::new(None),
            capture: RefCell::new(None),
            list_key: RefCell::new(None),
            destination_keys: RefCell::new(HashSet::new()),
            target: Cell::new(None),
            did_resolve_destination: Cell::new(false),
            last_target_read: Cell::new(None),
            ticks: Cell::new(0),
            is_waiting_for_target: Cell::new(false),
            is_completed: Cell::new(false),
            is_lingering: Cell::new(false),
            linger_touch_watch: RefCell::new(None),
            linger_list_frame: Cell::new(None),
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

    fn context(&self) -> Option<TransitionContext> {
        self.context.borrow().clone()
    }

    fn to_view(&self) -> Option<SharedElementTransitionView> {
        self.to_view.borrow().clone()
    }

    fn runner(&self) -> Option<SharedElementFlightRunner> {
        self.runner.borrow().clone()
    }

    fn capture(&self) -> Option<SharedElementCapture> {
        self.capture.borrow().clone()
    }

    fn destination(&self) -> Option<Rc<dyn SharedElementDestinationContainer>> {
        self.destination.borrow().as_ref().and_then(Weak::upgrade)
    }

    fn coordinator_epoch(cx: &mut App) -> u64 {
        SharedElementTransitionCoordinator::shared(cx).read(cx).epoch()
    }

    fn transition_did_end(&self, cx: &mut App) {
        let epoch = self.epoch;
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.transition_did_end(epoch, cx));
    }

    /// sets the to view alpha to 1 alongside the flight
    fn to_view_alpha_alongside(&self) -> Option<SharedElementFlightAlongside> {
        let to_view = self.to_view();
        let alongside: SharedElementFlightAlongside = Rc::new(move |progress: f32, cx: &mut App| {
            if let Some(to_view) = &to_view {
                to_view.set_alpha(SharedElementGeometry::lerp(0.0, 1.0, progress), cx);
            }
        });
        Some(alongside)
    }

    fn begin(&self, transition_context: TransitionContext, cx: &mut App) {
        let (Some(from_view), Some(to_view)) = (
            SharedElementTransitionView::of(&transition_context, SharedElementTransitionContextKey::From),
            SharedElementTransitionView::of(&transition_context, SharedElementTransitionContextKey::To),
        ) else {
            self.is_completed.set(true);
            transition_context.complete_transition(!transition_context.transition_was_cancelled(cx), cx);
            self.transition_did_end(cx);
            return;
        };
        transition_context.apply_final_frame(SharedElementTransitionContextKey::To, cx);
        let capture = self.intent.capture();
        let (true, Some(capture)) = (SharedElementTransitionSupport::is_live(&self.intent, cx), capture) else {
            self.is_completed.set(true);
            transition_context.add_subview(SharedElementTransitionContextKey::To, cx);
            transition_context.complete_transition(!transition_context.transition_was_cancelled(cx), cx);
            self.transition_did_end(cx);
            return;
        };

        *self.context.borrow_mut() = Some(transition_context.clone());
        *self.from_view.borrow_mut() = Some(from_view);
        *self.to_view.borrow_mut() = Some(to_view.clone());
        *self.capture.borrow_mut() = Some(capture.clone());
        let epoch = self.epoch;
        if let Some(this) = self.this.upgrade() {
            SharedElementNavigationDriver::shared().transition_did_begin(epoch, this, None);
        }
        let on_resign_active = self.weak(|this, cx| this.application_will_resign_active(cx));
        let on_force_end = self.weak(|this, cx| this.force_end(cx));
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, _| {
            coordinator.attach_transition_hooks(epoch, Some(on_resign_active), on_force_end);
        });
        self.watchdog.arm(self.weak(|this, cx| this.force_end(cx)), cx);
        self.set_interaction(false, cx);

        to_view.set_alpha(0.0, cx);
        transition_context.add_subview(SharedElementTransitionContextKey::To, cx);

        let runner = SharedElementFlightRunner::new(transition_context.container_view(cx), epoch, cx);
        *self.runner.borrow_mut() = Some(runner.clone());
        runner.begin(
            capture.content.clone(),
            SharedElementTransitionSupport::source(&capture),
            capture.backdrop.clone(),
            cx,
        );
        let source_key = capture.source_key.clone();
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.lift(HashSet::from([source_key]), epoch, cx));
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.hold_inline_playback(epoch, cx));

        self.is_waiting_for_target.set(true);
        SharedElementTransitionSupport::after(
            0.0,
            epoch,
            self.weak(|this, cx| {
                if !this.is_waiting_for_target.get() || this.is_completed.get() {
                    return;
                }
                if let Some(to_view) = this.to_view() {
                    to_view.layout_if_needed(cx);
                }
                this.read_target(cx);
                if !this.is_waiting_for_target.get() {
                    return;
                }
                this.ticker.start(
                    this.weak(|this, cx| {
                        this.ticks.set(this.ticks.get() + 1);
                        this.read_target(cx);
                    }),
                    cx,
                );
            }),
            cx,
        );
    }

    fn read_target(&self, cx: &mut App) {
        if !self.is_waiting_for_target.get() || self.is_completed.get() {
            return;
        }
        let Some(capture) = self.capture() else { return };
        if Self::coordinator_epoch(cx) != self.epoch {
            self.stop_waiting();
            return;
        }
        let destination = self.resolve_destination(capture.part, cx);
        let read = destination
            .as_ref()
            .and_then(|destination| destination.open_target(&self.intent.item_id, capture.part, cx));
        if let (Some(destination), Some(read), Some(previous)) = (destination, read, self.last_target_read.get())
            && SharedElementGeometry::max_edge_delta(read, previous) <= SharedElementMotion::OPEN_TARGET_STABLE_DELTA
        {
            self.stop_waiting();
            self.fly(read, destination, cx);
            return;
        }
        self.last_target_read.set(read);
        if self.ticks.get() >= SharedElementMotion::OPEN_TARGET_WAIT_TICKS {
            self.stop_waiting();
            self.fall_back(cx);
        }
    }

    fn resolve_destination(&self, part: SharedElementPart, cx: &mut App) -> Option<Rc<dyn SharedElementDestinationContainer>> {
        if self.did_resolve_destination.get() {
            return self.destination();
        }
        let found = self.intent.destination().or_else(|| {
            let host = self.context().and_then(|context| context.view_controller(SharedElementTransitionContextKey::To));
            SharedElementTransitionCoordinator::shared(cx).read(cx).destination_hosted_by(host)
        })?;
        self.did_resolve_destination.set(true);
        *self.destination.borrow_mut() = Some(Rc::downgrade(&found));
        self.intent.set_destination(Some(&found));
        if part == SharedElementPart::Media {
            *self.list_key.borrow_mut() =
                Some(SharedElementKey::new(found.surface(), self.intent.item_id.clone(), SharedElementPart::Media));
        }
        let keys = found.lift_keys(&self.intent.item_id, part, cx);
        if !keys.is_empty() {
            *self.destination_keys.borrow_mut() = keys.clone();
            let epoch = self.epoch;
            SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.lift(keys, epoch, cx));
            self.last_target_read.set(None);
        }
        Some(found)
    }

    fn landing(
        &self,
        clip: Bounds<Pixels>,
        destination: &Rc<dyn SharedElementDestinationContainer>,
        capture: &SharedElementCapture,
        cx: &mut App,
    ) -> SharedElementEndpoint {
        let mode = match capture.part {
            SharedElementPart::Media => destination.open_landing_mode(
                &self.intent.item_id,
                SharedElementTransitionSupport::content_aspect(capture),
                clip,
                cx,
            ),
            SharedElementPart::Text => SharedElementContentMode::AnchoredWidthFit,
        };
        SharedElementTransitionSupport::open_landing(capture, clip, mode)
    }

    fn fly(&self, clip: Bounds<Pixels>, destination: Rc<dyn SharedElementDestinationContainer>, cx: &mut App) {
        let (Some(runner), Some(capture)) = (self.runner(), self.capture()) else { return };
        let target = self.landing(clip, &destination, &capture, cx);
        self.target.set(Some(target));
        runner.animate(
            target,
            None,
            self.to_view_alpha_alongside(),
            Box::new(self.weak(|this, cx| this.move_did_finish(true, cx))),
            cx,
        );
        if capture.part == SharedElementPart::Text {
            runner.add_flight_fade_out(SharedElementMotion::COVER_FADE_START_PROGRESS, cx);
        }
        SharedElementTransitionSupport::after(
            SharedElementMotion::RETARGET_CHECK_DELAY,
            self.epoch,
            self.weak(|this, cx| this.retarget_if_needed(cx)),
            cx,
        );
    }

    fn retarget_if_needed(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        let (Some(runner), Some(capture), Some(target), Some(destination)) =
            (self.runner(), self.capture(), self.target.get(), self.destination())
        else {
            return;
        };
        let Some(fresh) = destination.open_target(&self.intent.item_id, capture.part, cx) else { return };
        if SharedElementGeometry::max_edge_delta(fresh, target.clip) <= SharedElementMotion::LANDING_MISMATCH {
            return;
        }
        let updated = self.landing(fresh, &destination, &capture, cx);
        self.target.set(Some(updated));
        runner.retarget(updated, cx);
    }

    fn fall_back(&self, cx: &mut App) {
        let Some(runner) = self.runner() else {
            self.move_did_finish(false, cx);
            return;
        };
        self.unlift_destination(cx);
        runner.fade_out(
            SharedElementMotion::OPEN_FALLBACK_FADE,
            SharedElementMotion::OPEN_FALLBACK_SCALE,
            self.to_view_alpha_alongside(),
            Box::new(self.weak(|this, cx| this.move_did_finish(false, cx))),
            cx,
        );
    }

    fn move_did_finish(&self, flew: bool, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        let Some(context) = self.context() else { return };
        let lingers = flew && self.begin_linger_if_needed(cx);
        let epoch = self.epoch;
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.land(epoch, cx));
        if let Some(to_view) = self.to_view() {
            to_view.set_alpha(1.0, cx);
        }
        self.set_interaction(true, cx);
        self.complete(&context, cx);
        if !lingers && let Some(runner) = self.runner() {
            runner.remove_after_commit(cx);
        }
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.release_inline_playback(epoch, cx));
        self.transition_did_end(cx);
    }

    fn begin_linger_if_needed(&self, cx: &mut App) -> bool {
        let (Some(runner), Some(context), Some(capture)) = (self.runner(), self.context(), self.capture()) else {
            return false;
        };
        if capture.part != SharedElementPart::Media || !matches!(capture.content, SharedElementFlightContent::Bitmap { .. }) {
            return false;
        }
        if let Some(destination) = self.destination()
            && let Some(ready) = destination.open_landing_is_ready(&self.intent.item_id, cx)
        {
            if ready {
                return false;
            }
            let item_id = self.intent.item_id.clone();
            let destination = Rc::downgrade(&destination);
            self.start_linger(&runner, &context, cx);
            self.watch_linger(
                &runner,
                &context,
                Rc::new(move |cx: &mut App| {
                    destination
                        .upgrade()
                        .and_then(|destination| destination.open_landing_is_ready(&item_id, cx))
                        .unwrap_or(true)
                }),
                cx,
            );
            return true;
        }
        let Some(source) = capture.flying_identity.clone() else { return false };
        if Url::from_string(&source).and_then(|url| url.scheme().map(str::to_owned)).is_none() {
            return false;
        }
        let same_picture = self
            .intent
            .origin()
            .map(|origin| origin.open_lands_on_same_picture(&self.intent.item_id, cx))
            .unwrap_or(true);
        if same_picture && Self::has_full_size_bitmap(&source) {
            return false;
        }
        self.start_linger(&runner, &context, cx);
        if !same_picture {
            self.release_linger(cx);
            return true;
        }
        self.watch_linger(&runner, &context, Rc::new(move |_: &mut App| Self::has_full_size_bitmap(&source)), cx);
        true
    }

    fn start_linger(&self, runner: &SharedElementFlightRunner, context: &TransitionContext, cx: &mut App) {
        if let Some(container) = context.view_flight_container(SharedElementTransitionContextKey::To, cx) {
            runner.reparent(container, cx);
        }
        self.is_lingering.set(true);
        let this = self.this.clone();
        let weak_runner = runner.downgrade();
        let abort: Box<dyn Fn(&mut App)> = Box::new(move |cx| {
            if let Some(this) = this.upgrade() {
                this.end_linger(cx);
            }
            if let Some(runner) = weak_runner.upgrade() {
                runner.abort(cx);
            }
        });
        let owner = self.owner_id();
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, _| coordinator.attach_open_linger(owner, abort));
    }

    fn watch_linger(
        &self,
        runner: &SharedElementFlightRunner,
        context: &TransitionContext,
        is_ready: Rc<dyn Fn(&mut App) -> bool>,
        cx: &mut App,
    ) {
        self.observe_list_scroll(cx);
        self.observe_list_touches(context, cx);
        self.poll_linger(RiseClock::monotonic(), runner.clone(), is_ready, cx);
    }

    /// holds the animator until the linger ends, a superseded epoch removes the flight at once
    fn poll_linger(
        &self,
        started_at: Duration,
        runner: SharedElementFlightRunner,
        is_ready: Rc<dyn Fn(&mut App) -> bool>,
        cx: &mut App,
    ) {
        let Some(this) = self.this.upgrade() else { return };
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs_f64(Self::LINGER_POLL_INTERVAL)).await;
            let _ = cx.update(|cx| {
                if !this.is_lingering.get() {
                    return;
                }
                if Self::coordinator_epoch(cx) != this.epoch {
                    this.end_linger(cx);
                    runner.abort(cx);
                    return;
                }
                let held = RiseClock::monotonic().saturating_sub(started_at).as_secs_f64();
                if held >= SharedElementMotion::FULL_IMAGE_HOLD || is_ready(cx) {
                    this.release_linger(cx);
                } else {
                    this.poll_linger(started_at, runner, is_ready, cx);
                }
            });
        })
        .detach();
    }

    fn observe_list_scroll(&self, cx: &mut App) {
        let Some(frame_key) = self.list_key.borrow().as_ref().map(SharedElementKey::frame_registry_key) else {
            return;
        };
        let Some(frame) = MediaCellScreenFrameRegistry::attached_frame(&frame_key) else { return };
        self.linger_list_frame.set(Some(frame));
        self.ticker.start(
            self.weak(move |this, cx| {
                if !this.is_lingering.get() {
                    this.ticker.stop();
                    return;
                }
                this.list_offset_did_change(MediaCellScreenFrameRegistry::attached_frame(&frame_key), cx);
            }),
            cx,
        );
    }

    fn list_offset_did_change(&self, frame: Option<Bounds<Pixels>>, cx: &mut App) {
        let (Some(frame), Some(initial)) = (frame, self.linger_list_frame.get()) else { return };
        if SharedElementGeometry::max_edge_delta(frame, initial) <= SharedElementMotion::OPEN_TARGET_STABLE_DELTA {
            return;
        }
        self.release_linger(cx);
    }

    fn observe_list_touches(&self, context: &TransitionContext, cx: &mut App) {
        let this = self.this.clone();
        let watch = context.add_touch_watch(
            SharedElementTransitionContextKey::To,
            Rc::new(move |cx: &mut App| {
                if let Some(this) = this.upgrade() {
                    this.release_linger(cx);
                }
            }),
            cx,
        );
        *self.linger_touch_watch.borrow_mut() = Some(watch);
    }

    fn release_linger(&self, cx: &mut App) {
        if !self.is_lingering.get() {
            return;
        }
        self.end_linger(cx);
        let Some(runner) = self.runner() else { return };
        let weak_runner = runner.downgrade();
        let this = self.this.clone();
        runner.fade_out(
            SharedElementMotion::FULL_IMAGE_CROSSFADE,
            1.0,
            None,
            Box::new(move |cx: &mut App| {
                if let Some(runner) = weak_runner.upgrade() {
                    runner.remove_after_commit(cx);
                }
                if let Some(this) = this.upgrade() {
                    let owner = this.owner_id();
                    SharedElementTransitionCoordinator::shared(cx)
                        .update(cx, |coordinator, _| coordinator.detach_open_linger(owner));
                }
            }),
            cx,
        );
    }

    fn end_linger(&self, cx: &mut App) {
        self.is_lingering.set(false);
        self.linger_list_frame.set(None);
        self.ticker.stop();
        let watch = self.linger_touch_watch.borrow_mut().take();
        if let Some(watch) = watch {
            watch.remove(cx);
        }
    }

    fn has_full_size_bitmap(source: &str) -> bool {
        let user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        [false, true].into_iter().any(|is_own| {
            CleverImageBitmapLoader::memory_image(
                source,
                is_own,
                user_id.as_deref(),
                account_epoch.as_deref(),
                None,
                PostMediaCachePolicy::image_policy(is_own),
                None,
                false,
            )
            .is_some()
        })
    }

    /// identity of the animator for the coordinator linger slot, keyed by address
    fn owner_id(&self) -> usize {
        self.this.as_ptr() as *const () as usize
    }

    fn application_will_resign_active(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        if !self.is_waiting_for_target.get() {
            if let Some(runner) = self.runner() {
                runner.jump_to_end(cx);
            }
            return;
        }
        self.stop_waiting();
        self.unlift_destination(cx);
        if let Some(runner) = self.runner() {
            runner.abort(cx);
        }
        self.move_did_finish(false, cx);
    }

    fn force_end(&self, cx: &mut App) {
        if self.is_completed.get() {
            return;
        }
        let Some(context) = self.context() else { return };
        self.end_linger(cx);
        if let Some(runner) = self.runner() {
            runner.abort(cx);
        }
        let epoch = self.epoch;
        let owner = self.owner_id();
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, _| coordinator.detach_open_linger(owner));
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| board.land(epoch, cx));
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.release_inline_playback(epoch, cx));
        if let Some(to_view) = self.to_view() {
            to_view.set_alpha(1.0, cx);
        }
        self.set_interaction(true, cx);
        self.complete(&context, cx);
        self.transition_did_end(cx);
    }

    fn complete(&self, context: &TransitionContext, cx: &mut App) {
        let epoch = self.epoch;
        let cancelled = context.transition_was_cancelled(cx);
        Sai::log(LOG_NAME, move || format!("open complete epoch={epoch} cancelled={cancelled}"), SaiLogKind::Ui, cx);
        self.is_completed.set(true);
        self.stop_waiting();
        self.watchdog.cancel();
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, _| coordinator.detach_transition_hooks(epoch));
        context.complete_transition(!cancelled, cx);
        *self.context.borrow_mut() = None;
        SharedElementNavigationDriver::shared().transition_did_finish(epoch);
    }

    fn stop_waiting(&self) {
        self.is_waiting_for_target.set(false);
        self.ticker.stop();
    }

    fn unlift_destination(&self, cx: &mut App) {
        let keys = self.destination_keys.borrow().clone();
        let epoch = self.epoch;
        SharedElementLiftBoard::shared(cx).update(cx, |board, cx| {
            for key in &keys {
                board.unlift(key, epoch, cx);
            }
        });
    }

    fn set_interaction(&self, enabled: bool, cx: &mut App) {
        let views = [self.from_view.borrow().clone(), self.to_view()];
        for view in views.into_iter().flatten() {
            view.set_user_interaction_enabled(enabled, cx);
        }
    }
}

impl SharedElementViewControllerAnimatedTransitioning for SharedElementOpenAnimator {
    fn transition_duration(&self, _: Option<&TransitionContext>) -> f64 {
        SharedElementMotion::MOVE_DURATION
    }

    fn animate_transition(&self, transition_context: TransitionContext, cx: &mut App) {
        self.begin(transition_context, cx);
    }
}
