use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::pin::pin;
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::{Either, select};
use gpui::{
    AnyWindowHandle, App, AsyncApp, Bounds, Context, Entity, Pixels, PromptButton, PromptLevel,
    Subscription, Task, WeakEntity, px,
};
use rise_canon::{
    RiseGuideAdvance, RiseGuideAnchor, RiseGuideCanon, RiseGuideFailure, RiseGuideGesture,
    RiseGuideNarrator, RiseGuidePose, RiseGuideScreen, RiseGuideSignal, RiseGuideSpotlightShape,
    RiseGuideStage, RiseGuideSurface, RiseGuideTrigger,
};
use rise_canon::rise_key::guide_screen::camel as guide_screen_keys;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_url::Url;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::{URLRequest, URLSession};
use rise_i18n::i18n::{FormatArg, format_printf, tr};
use rise_ui::guide_anchor_ui::guide_anchor_registry::{
    GuideAnchorCapture, GuideAnchorRegistry, GuideAnchorSighting,
};
use rise_ui::guide_anchor_ui::guide_anchor_tap_tracker::GuideAnchorTapTracker;
use rise_ui::guide_anchor_ui::guide_anchor_tracker::{GuideAnchorTracker, GuideAnchorTrackerReport};
use rise_ui::guide_anchor_ui::guide_system_menu_probe::GuideSystemMenuProbe;
use rise_ui::guide_overlay_ui::guide_overlay_theme::GuideOverlayMotion;
use rise_ui::guide_overlay_ui::guide_overlay_window::{
    GuideOverlayHost, GuideOverlayStep, GuideOverlayStepID,
};
use rise_ui::guide_overlay_ui::guide_spotlight_layer::GuideSpotlightCutout;
use rise_ui::guide_overlay_ui::guide_surface_views::{
    GuideBubbleContent, GuideCaptionContent, GuideCardContent, GuideOverlayAction,
    GuideOverlaySurfaceContent, GuideVideoContent,
};

use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::{GuideConditions, GuideFacts};
use crate::core::guides::guide_screen_resolver::{
    GuideNavigationBridge, GuideNavigationSnapshot, GuideScreenObservation, GuideScreenObserver,
    GuideScreenResolver,
};
use crate::core::guides::guide_signal_bus::{GuideEventAwaiter, GuideEventWait, GuideSignalBus};
use crate::core::guides::guide_trace::GuideTrace;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{SaiActiveScreen, SaiNavigationBridge};
use crate::core::lib::sai::sai::Sai;
use crate::core::maintenance::maintenance_gate::MaintenanceGate;
use crate::core::ui::guide_anchor_ui::guide_anchor_scroller::GuideAnchorScroller;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::auth::services::account_state_reset_coordinator::{
    AccountResettable, AccountStateResetRegistry,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::engine::guide_auto_start::GuideAutoStart;
use crate::modules::guides::engine::rise_guides_domain::RiseGuidesActionExecutor;
use crate::modules::guides::engine::rise_guides_models::{RiseGuide, RiseGuideAck, RiseGuideStepModel};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::rtc::components::rise_rtc_zoomable_video_surface::RiseRTCVideoZoomContainerView;
use crate::modules::rtc::engine::rise_rtc_presentation::RiseRTCPresentation;

/// runs one guide step by step
/// never presses anything, only highlights and explains and waits
/// screens change by route and never by a fake click that could land on send or delete
pub struct GuideRunner {
    running_guide_key: Option<String>,
    session: Option<GuideRunnerSessionRef>,
    task: Option<Task<()>>,
    screen_observation: Option<GuideScreenObservation>,
    keyboard_height: Pixels,
    lifecycle_observers: Vec<RiseSubscription>,
    lifecycle_task: Option<Task<()>>,
    interruption_watch: Vec<Subscription>,
    background_abort: Option<Task<()>>,
    backgrounded_at: Option<Date>,
    apology_gate: Option<GuideEventAwaiter>,
    anchor_tracker: GuideAnchorTracker,
    leave_check: Option<Task<()>>,
    menu_watch: Option<Task<()>>,
}

/// what started the run, travels with the receipt
/// a shown from any origin but manual counts one automatic start against the guide budget
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuideRunnerOrigin {
    Manual,
    ScreenEnter,
    FirstLaunch,
    EmptyState,
    Signal,
}

impl GuideRunnerOrigin {
    pub fn trigger(self) -> RiseGuideTrigger {
        match self {
            Self::Manual => RiseGuideTrigger::Manual,
            Self::ScreenEnter => RiseGuideTrigger::ScreenEnter,
            Self::FirstLaunch => RiseGuideTrigger::FirstLaunch,
            Self::EmptyState => RiseGuideTrigger::EmptyState,
            Self::Signal => RiseGuideTrigger::Signal,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::ScreenEnter => "screenEnter",
            Self::FirstLaunch => "firstLaunch",
            Self::EmptyState => "emptyState",
            Self::Signal => "signal",
        }
    }
}

struct GuideRunnerSession {
    guide: RiseGuide,
    origin: GuideRunnerOrigin,
    index: usize,
    reached_step: i64,
    last_anchor_frame: Option<Bounds<Pixels>>,
    last_anchor_shape: Option<RiseGuideSpotlightShape>,
    did_write_shown: bool,
    entry_navigation: Option<GuideNavigationSnapshot>,
    expected_screen: Option<SaiActiveScreen>,
    expected_navigation: Option<GuideNavigationSnapshot>,
    is_steering: bool,
    is_asking_about_leaving: bool,
}

impl GuideRunnerSession {
    fn new(guide: RiseGuide, origin: GuideRunnerOrigin) -> Self {
        Self {
            guide,
            origin,
            index: 0,
            reached_step: 0,
            last_anchor_frame: None,
            last_anchor_shape: None,
            did_write_shown: false,
            entry_navigation: None,
            expected_screen: None,
            expected_navigation: None,
            is_steering: true,
            is_asking_about_leaving: false,
        }
    }
}

type GuideRunnerSessionRef = Rc<RefCell<GuideRunnerSession>>;

enum GuideRunnerStepOutcome {
    Advanced,
    Exited,
    Failed(RiseGuideFailure),
}

enum GuideRunnerSettlement {
    Frame(Option<Bounds<Pixels>>),
    Satisfied,
}

fn as_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn millis(value: i64) -> Duration {
    Duration::from_millis(u64::try_from(value).unwrap_or(0))
}

impl GuideRunner {
    const WAIT_FOREVER: i64 = 12 * 60 * 60 * 1000;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            running_guide_key: None,
            session: None,
            task: None,
            screen_observation: None,
            keyboard_height: px(0.0),
            lifecycle_observers: Vec::new(),
            lifecycle_task: None,
            interruption_watch: Vec::new(),
            background_abort: None,
            backgrounded_at: None,
            apology_gate: None,
            anchor_tracker: GuideAnchorTracker::new(),
            leave_check: None,
            menu_watch: None,
        }
    }

    /// the guide on screen or none, only the catalogue screen observes it
    pub fn running_guide_key(&self) -> Option<&str> {
        self.running_guide_key.as_deref()
    }

    /// starts guide and replaces anything running since the overlay cant render two guides and
    /// the newer request is the one just made
    pub fn start(&mut self, guide: RiseGuide, origin: GuideRunnerOrigin, cx: &mut Context<Self>) {
        if guide.steps.is_empty() {
            return;
        }
        self.stop(RiseGuideAck::Failed(RiseGuideFailure::Interrupted), "replaced", cx);

        let entry = GuideNavigationBridge::snapshot(cx);
        let guide = Self::sandboxed_catalogue_replay(guide, origin, entry.as_ref());
        let session = Rc::new(RefCell::new(GuideRunnerSession::new(guide.clone(), origin)));
        self.session = Some(session.clone());
        self.running_guide_key = Some(guide.key.clone());
        self.install_observers(cx);
        let this = cx.weak_entity();
        self.screen_observation = Some(GuideScreenObserver::observe(move |screen, cx| {
            let this = this.clone();
            let screen = screen.clone();
            cx.defer(move |cx| {
                this.update(cx, |runner, cx| runner.screen_did_change(screen, cx)).ok();
            });
        }));
        self.watch_interruptions(cx);
        let entity = cx.entity();
        AccountStateResetRegistry::register(&entity, cx);
        GuideAnchorCapture::begin();
        session.borrow_mut().entry_navigation = entry;
        if GuideSandbox::is_wanted(&guide) {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.arm(&guide, cx));
        }
        let this = cx.weak_entity();
        GuideOverlayHost::set_on_interrupted(
            move |cx| {
                this.update(cx, |runner, cx| {
                    runner.finish(RiseGuideAck::Failed(RiseGuideFailure::Interrupted), true, cx)
                })
                .ok();
            },
            cx,
        );

        GuideTrace::started(&guide.key, as_i64(guide.steps.len()), origin.description(), cx);
        self.task = Some(cx.spawn(async move |this, cx| Self::run(this, session, cx).await));
        cx.notify();
    }

    fn sandboxed_catalogue_replay(
        mut guide: RiseGuide,
        origin: GuideRunnerOrigin,
        entry: Option<&GuideNavigationSnapshot>,
    ) -> RiseGuide {
        if origin != GuideRunnerOrigin::Manual {
            return guide;
        }
        let from_catalogue = matches!(
            entry.and_then(|entry| entry.path.last()),
            Some(RootRoute::GuidesCatalogue | RootRoute::GuidesCategory { .. })
        );
        if !from_catalogue || guide.steps.iter().any(|step| !step.route_params.is_empty()) {
            return guide;
        }
        let Some(first_index) = guide
            .steps
            .iter()
            .position(|step| step.anchor.is_some() || step.route_screen.is_some())
        else {
            return guide;
        };
        let first = &mut guide.steps[first_index];
        let lands_on_playlist = first.route_screen.is_none()
            && first
                .anchor
                .and_then(|anchor| GuideScreenResolver::screen(anchor.screen()))
                == Some(RiseGuideScreen::MusicPlaylist);
        if !lands_on_playlist {
            return guide;
        }
        first.route_screen = Some(RiseGuideScreen::MusicPlaylist);
        first.route_params.insert(guide_screen_keys::PLAYLIST_ID.to_owned(), GuideSandbox::playlist_id());
        if let Some(greeting) = guide.steps[..first_index].first_mut() {
            greeting.route_screen = Some(RiseGuideScreen::Music);
        }
        guide
    }

    /// ends the running guide from outside like a call or maintenance or long background with a
    /// receipt
    /// and puts the reader back where they were
    pub fn stop(&mut self, ack: RiseGuideAck, reason: &str, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        let key = session.borrow().guide.key.clone();
        GuideTrace::note(&format!("{key} stopped: {reason}"), cx);
        self.finish(ack, true, cx);
    }

    fn is_current(this: &WeakEntity<Self>, session: &GuideRunnerSessionRef, cx: &AsyncApp) -> bool {
        this.read_with(cx, |runner, _| {
            runner.session.as_ref().is_some_and(|running| Rc::ptr_eq(running, session))
        })
        .unwrap_or(false)
    }

    fn owns(&self, session: &GuideRunnerSessionRef) -> bool {
        self.session.as_ref().is_some_and(|running| Rc::ptr_eq(running, session))
    }

    async fn run(this: WeakEntity<Self>, session: GuideRunnerSessionRef, cx: &mut AsyncApp) {
        let guide = session.borrow().guide.clone();
        Self::prefetch_videos(&guide);
        let total = guide.steps.len();

        loop {
            let index = session.borrow().index;
            if index >= total {
                break;
            }
            if !Self::is_current(&this, &session, cx) {
                return;
            }
            let step = guide.steps[index].clone();
            let writes_shown = {
                let mut running = session.borrow_mut();
                running.reached_step = running.reached_step.max(step.position);
                let first = !running.did_write_shown;
                running.did_write_shown = true;
                first
            };
            if writes_shown {
                this.update(cx, |runner, cx| runner.acknowledge(&session, RiseGuideAck::Shown, cx)).ok();
            }

            cx.update(|cx| GuideTrace::step(as_i64(index), as_i64(total), &step, cx));
            match Self::present(&this, &step, &session, cx).await {
                GuideRunnerStepOutcome::Advanced => {
                    cx.update(|cx| GuideTrace::advanced(as_i64(index), cx));
                    session.borrow_mut().is_steering = true;
                    if step.anchor == Some(RiseGuideAnchor::CallMainVideo) {
                        cx.update(RiseRTCVideoZoomContainerView::reset_all_zoom);
                    }
                    if step.advance == RiseGuideAdvance::Screen {
                        this.update(cx, |runner, cx| runner.remember_place(&session, cx)).ok();
                    }
                    session.borrow_mut().index += 1;
                }
                GuideRunnerStepOutcome::Exited => return,
                GuideRunnerStepOutcome::Failed(reason) => {
                    if !Self::is_current(&this, &session, cx) {
                        return;
                    }
                    Self::apologise(&this, reason, &session, cx).await;
                    if !Self::is_current(&this, &session, cx) {
                        return;
                    }
                    this.update(cx, |runner, cx| runner.finish(RiseGuideAck::Failed(reason), true, cx)).ok();
                    return;
                }
            }
        }

        this.update(cx, |runner, cx| runner.finish(RiseGuideAck::Completed, true, cx)).ok();
    }

    async fn apologise(
        this: &WeakEntity<Self>,
        reason: RiseGuideFailure,
        session: &GuideRunnerSessionRef,
        cx: &mut AsyncApp,
    ) {
        let body = if reason == RiseGuideFailure::AnchorMissing {
            match Self::required_right_key(session) {
                Some(right_key) => format_printf(
                    &tr("guide_failed_anchor_missing_right_body"),
                    &[FormatArg::from(tr(right_key))],
                ),
                None => tr("guide_failed_anchor_missing_body"),
            }
        } else {
            tr("guide_failed_body")
        };
        let (guide_id, key) = {
            let running = session.borrow();
            (running.guide.id, running.guide.key.clone())
        };
        let gate = GuideEventAwaiter::new();
        let presented = this.update(cx, |runner, cx| {
            let settle = gate.clone();
            let step = GuideOverlayStep::new(
                GuideOverlayStepID::new(format!("{guide_id}:failed")),
                RiseGuideStage::Dim,
            )
            .narrator(RiseGuideNarrator::Panda)
            .pose(Some(RiseGuidePose::Apologise))
            .surface(GuideOverlaySurfaceContent::Bubble(GuideBubbleContent::new(body).primary(
                GuideOverlayAction::new(tr("guide_done"), move |_, _| {
                    settle.settle(true);
                }),
            )))
            .keyboard_height(runner.keyboard_height);
            runner.apology_gate = Some(gate.clone());
            GuideOverlayHost::present(step, Self::active_scene(cx), None, cx);
        });
        if presented.is_err() {
            return;
        }
        GuideEventWait::run(gate, Self::WAIT_FOREVER, |awaiter: &GuideEventAwaiter| {
            awaiter.settle(false);
        }, cx)
        .await;
        this.update(cx, |runner, cx| {
            runner.apology_gate = None;
            let message = format!("{key} gave up: {}", reason.raw_value());
            Sai::log("Guides", move || message, SaiLogKind::Custom("guide".to_owned()), cx);
        })
        .ok();
    }

    fn required_right_key(session: &GuideRunnerSessionRef) -> Option<&'static str> {
        let running = session.borrow();
        let steps = &running.guide.steps;
        let anchor = steps.get(running.index)?.anchor?;
        match anchor {
            RiseGuideAnchor::ForumAddTopic => {
                if steps.iter().any(|step| step.anchor == Some(RiseGuideAnchor::TopicEditorVoiceKind)) {
                    Some("perm_manage_voice_channel")
                } else {
                    Some("perm_manage_topics")
                }
            }
            RiseGuideAnchor::TopicEditorVoiceKind => Some("perm_manage_voice_channel"),
            RiseGuideAnchor::ForumCreateSection => Some("perm_manage_sections"),
            RiseGuideAnchor::ForumMenuVisibility
            | RiseGuideAnchor::VisibilityPrivate
            | RiseGuideAnchor::VisibilityRoles => Some("perm_manage_roles"),
            RiseGuideAnchor::ChatProfileEdit
            | RiseGuideAnchor::ChatSettingsTopics
            | RiseGuideAnchor::ChatTopicsEnable => Some("perm_manage_group"),
            RiseGuideAnchor::RtcMenuForceMute => Some("perm_mute_voice_members"),
            RiseGuideAnchor::RtcMenuPromoteSpeaker => Some("perm_manage_stage"),
            _ => None,
        }
    }

    /// records where the running step lives and the navigation back to it
    /// reads the live screen from the navigation store not the observer last report since they
    /// disagree for exactly this frame
    fn remember_place(&mut self, session: &GuideRunnerSessionRef, cx: &mut Context<Self>) {
        let screen = SaiNavigationBridge::current_screen(cx).or_else(GuideScreenObserver::current);
        let navigation = GuideNavigationBridge::snapshot(cx);
        {
            let mut running = session.borrow_mut();
            running.expected_screen = screen;
            running.expected_navigation = navigation;
        }
        self.leave_check = None;
    }

    fn end_steering(&mut self, session: &GuideRunnerSessionRef, cx: &mut Context<Self>) {
        if !session.borrow().is_steering {
            return;
        }
        session.borrow_mut().is_steering = false;
        self.schedule_leave_check(session, cx);
    }

    fn screen_did_change(&mut self, screen: SaiActiveScreen, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if session.borrow().is_steering {
            let key = session.borrow().guide.key.clone();
            GuideTrace::moved_while_steering(&key, &screen.id, cx);
            return;
        }
        self.schedule_leave_check(&session, cx);
    }

    /// asks once the app has stopped moving, the window is canon trigger_settle_ms so the
    /// runtime keeps one meaning of settled
    fn schedule_leave_check(&mut self, session: &GuideRunnerSessionRef, cx: &mut Context<Self>) {
        let session = session.clone();
        self.leave_check = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(millis(RiseGuideCanon::TIMINGS_TRIGGER_SETTLE_MS))
                .await;
            this.update(cx, |runner, cx| runner.confirm_leaving(&session, cx)).ok();
        }));
    }

    fn confirm_leaving(&mut self, session: &GuideRunnerSessionRef, cx: &mut Context<Self>) {
        if !self.owns(session) || self.apology_gate.is_some() {
            return;
        }
        let (step, expected, origin, trigger_screen, index, key) = {
            let running = session.borrow();
            if running.is_steering || running.is_asking_about_leaving {
                return;
            }
            let Some(expected) = running.expected_screen.clone() else {
                return;
            };
            let Some(step) = running.guide.steps.get(running.index).cloned() else {
                return;
            };
            (
                step,
                expected,
                running.origin,
                running.guide.trigger_screen,
                running.index,
                running.guide.key.clone(),
            )
        };
        if origin == GuideRunnerOrigin::ScreenEnter
            && index == 0
            && step.route_screen.is_none()
            && step.advance != RiseGuideAdvance::Screen
            && let Some(trigger) = trigger_screen
            && let Some(screen) = SaiNavigationBridge::current_screen(cx)
            && GuideScreenResolver::screen(&screen.id) != Some(trigger)
            && step.anchor.map(|anchor| anchor.screen()) != Some(screen.id.as_str())
        {
            GuideTrace::note(
                &format!("{key} started on its own; the reader left {}", trigger.raw_value()),
                cx,
            );
            self.finish(RiseGuideAck::Failed(RiseGuideFailure::Interrupted), false, cx);
            return;
        }
        if origin == GuideRunnerOrigin::Manual
            && index == 0
            && step.anchor.is_none()
            && step.route_screen.is_none()
            && step.advance != RiseGuideAdvance::Screen
            && let Some(screen) = SaiNavigationBridge::current_screen(cx)
            && !Self::is_same_screen(&screen, &expected)
        {
            GuideTrace::note(&format!("{key} replay greeting; the reader left {}", expected.id), cx);
            self.finish(RiseGuideAck::Failed(RiseGuideFailure::Interrupted), false, cx);
            return;
        }
        if step.anchor.is_none() && step.route_screen.is_none() && step.advance != RiseGuideAdvance::Screen {
            self.remember_place(session, cx);
            return;
        }
        let Some(screen) = SaiNavigationBridge::current_screen(cx) else {
            return;
        };
        if Self::is_same_screen(&screen, &expected) {
            return;
        }
        let shown = SaiNavigationBridge::visible_screens(cx);
        if shown.iter().any(|visible| Self::is_same_screen(visible, &expected)) {
            return;
        }
        if step.advance == RiseGuideAdvance::Screen
            && let Some(destination) = step.advance_screen
            && GuideScreenResolver::matches(&screen, destination, &HashMap::new())
        {
            return;
        }
        if step.advance != RiseGuideAdvance::Screen
            && let Some(anchor) = step.anchor
            && let Some(home) = shown.iter().find(|visible| visible.id == anchor.screen())
        {
            GuideTrace::note(
                &format!(
                    "{key} is on {}, where {} lives; taking that as the step's place",
                    home.id,
                    anchor.raw_value()
                ),
                cx,
            );
            let home = home.clone();
            self.remember_place(session, cx);
            session.borrow_mut().expected_screen = Some(home);
            return;
        }

        GuideTrace::note(
            &format!("{key} left {} for {}; asking whether to return", expected.id, screen.id),
            cx,
        );
        self.present_leave_alert(session, cx);
    }

    fn present_leave_alert(&mut self, session: &GuideRunnerSessionRef, cx: &mut Context<Self>) {
        let Some(host) = GuideOverlayHost::shared(cx).alert_host() else {
            return;
        };
        let title = tr("guide_left_title");
        let body = tr("guide_left_body");
        let answers = [
            PromptButton::new(tr("guide_left_exit")),
            PromptButton::cancel(tr("guide_left_return")),
        ];
        let receiver = host
            .update(cx, |_, window, cx| {
                window.prompt(PromptLevel::Warning, &title, Some(body.as_str()), &answers, cx)
            })
            .ok();
        let Some(receiver) = receiver else {
            return;
        };
        session.borrow_mut().is_asking_about_leaving = true;
        let session = session.clone();
        cx.spawn(async move |this, cx| {
            let answer = receiver.await.ok();
            this.update(cx, |runner, cx| {
                if !runner.owns(&session) {
                    return;
                }
                session.borrow_mut().is_asking_about_leaving = false;
                if answer == Some(0) {
                    runner.finish(RiseGuideAck::Skipped, true, cx);
                    return;
                }
                let place = session.borrow().expected_navigation.clone();
                if let Some(place) = place {
                    GuideNavigationBridge::restore(place, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// same screen means same id and every parameter the remembered screen names
    fn is_same_screen(active: &SaiActiveScreen, expected: &SaiActiveScreen) -> bool {
        if active.id != expected.id {
            return false;
        }
        expected
            .params
            .iter()
            .all(|(key, value)| active.params.get(key) == Some(value))
    }

    fn reveal_window_ms(step: &RiseGuideStepModel) -> i64 {
        RiseGuideCanon::TIMINGS_ANCHOR_REVEAL_MS
            .max(step.timeout_ms)
            .min(RiseGuideCanon::TIMINGS_ANCHOR_REVEAL_MAX_MS)
    }

    fn catalogue_entry_route(
        step: &RiseGuideStepModel,
        session: &GuideRunnerSessionRef,
    ) -> Option<RiseGuideScreen> {
        let running = session.borrow();
        if running.origin != GuideRunnerOrigin::Manual {
            return None;
        }
        let entry_route = running.entry_navigation.as_ref()?.path.last()?;
        let first_anchored = running
            .guide
            .steps
            .iter()
            .position(|candidate| candidate.anchor.is_some() || candidate.route_screen.is_some());
        if first_anchored != Some(running.index) {
            return None;
        }
        let anchor = step.anchor?;
        match entry_route {
            RootRoute::GuidesCatalogue | RootRoute::GuidesCategory { .. } => {
                GuideScreenResolver::screen(anchor.screen())
            }
            _ => None,
        }
    }

    async fn present(
        this: &WeakEntity<Self>,
        step: &RiseGuideStepModel,
        session: &GuideRunnerSessionRef,
        cx: &mut AsyncApp,
    ) -> GuideRunnerStepOutcome {
        let advance_gate = GuideEventAwaiter::new();
        let event_mark = Self::event_mark(step);
        let route = step.route_screen.or_else(|| Self::catalogue_entry_route(step, session));

        if let Some(route) = route {
            let accepted = this.update(cx, |runner, cx| {
                GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.publish_scene(cx));
                let overlay = runner.overlay_step(step, session, None, &advance_gate, cx);
                GuideOverlayHost::present(overlay, Self::active_scene(cx), None, cx);
                let accepted = GuideNavigationBridge::navigate(route, &step.route_params, cx);
                GuideTrace::routing(route, accepted, cx);
                accepted
            });
            match accepted {
                Ok(true) => {}
                Ok(false) => return GuideRunnerStepOutcome::Failed(RiseGuideFailure::ScreenUnreachable),
                Err(_) => return GuideRunnerStepOutcome::Exited,
            }
            if !GuideScreenObserver::wait_for(route, &step.route_params, Self::WAIT_FOREVER, cx).await {
                return GuideRunnerStepOutcome::Failed(RiseGuideFailure::ScreenUnreachable);
            }
            cx.update(|cx| GuideTrace::arrived(route, cx));
            this.update(cx, |runner, cx| runner.remember_place(session, cx)).ok();
        } else if session.borrow().expected_screen.is_none() {
            this.update(cx, |runner, cx| runner.remember_place(session, cx)).ok();
        }

        this.update(cx, |runner, cx| runner.end_steering(session, cx)).ok();

        if let Some(signal) = Self::state_signal(step)
            && GuideFacts::is_true(signal)
        {
            cx.update(|cx| GuideTrace::condition_already_true(signal, cx));
            return GuideRunnerStepOutcome::Advanced;
        }

        let mut anchor_frame: Option<Bounds<Pixels>> = None;
        if let Some(anchor) = step.anchor {
            Self::reveal_anchor(anchor, true, cx).await;
            match Self::settle(anchor, RiseGuideCanon::TIMINGS_ANCHOR_REVEAL_MS, step, cx).await {
                GuideRunnerSettlement::Satisfied => {
                    let message = format!(
                        "{} step {} came true while its anchor loaded",
                        session.borrow().guide.key,
                        step.position
                    );
                    cx.update(|cx| GuideTrace::note(&message, cx));
                    return GuideRunnerStepOutcome::Advanced;
                }
                GuideRunnerSettlement::Frame(frame) => anchor_frame = frame,
            }
        }

        if !Self::is_current(this, session, cx) {
            return GuideRunnerStepOutcome::Exited;
        }

        let anchor_level = step.anchor.and_then(GuideAnchorRegistry::window_level);
        this.update(cx, |runner, cx| {
            if route.is_none() {
                let overlay = runner.overlay_step(step, session, anchor_frame, &advance_gate, cx);
                GuideOverlayHost::present(overlay, Self::active_scene(cx), anchor_level, cx);
            } else if let Some(frame) = anchor_frame {
                GuideOverlayHost::raise(anchor_level, cx);
                GuideOverlayHost::move_anchor(frame, true, None, cx);
            }
        })
        .ok();

        if let Some(anchor) = step.anchor
            && anchor_frame.is_none()
        {
            cx.update(|cx| GuideTrace::anchor_pending(anchor, cx));
            let remaining = (Self::reveal_window_ms(step) - RiseGuideCanon::TIMINGS_ANCHOR_REVEAL_MS).max(0);
            loop {
                if let Some(outcome) =
                    Self::wait_while_anchor_is_covered(this, anchor, step, event_mark, session, cx).await
                {
                    return outcome;
                }
                Self::reveal_anchor(anchor, false, cx).await;
                match Self::settle(anchor, remaining, step, cx).await {
                    GuideRunnerSettlement::Satisfied => {
                        let message = format!(
                            "{} step {} came true while its anchor was missing",
                            session.borrow().guide.key,
                            step.position
                        );
                        cx.update(|cx| GuideTrace::note(&message, cx));
                        return GuideRunnerStepOutcome::Advanced;
                    }
                    GuideRunnerSettlement::Frame(frame) => anchor_frame = frame,
                }
                if !Self::is_current(this, session, cx) {
                    return GuideRunnerStepOutcome::Exited;
                }
                if anchor_frame.is_some() || GuideAnchorRegistry::sighting(anchor) != GuideAnchorSighting::Covered {
                    break;
                }
            }
            if !Self::is_current(this, session, cx) {
                return GuideRunnerStepOutcome::Exited;
            }
            let Some(frame) = anchor_frame else {
                if Self::event_came(event_mark) {
                    return GuideRunnerStepOutcome::Advanced;
                }
                let registered = GuideAnchorRegistry::is_registered(anchor);
                cx.update(|cx| GuideTrace::anchor_failed(anchor, registered, cx));
                return GuideRunnerStepOutcome::Failed(if registered {
                    RiseGuideFailure::AnchorOffscreen
                } else {
                    RiseGuideFailure::AnchorMissing
                });
            };
            cx.update(|cx| {
                GuideOverlayHost::raise(GuideAnchorRegistry::window_level(anchor), cx);
                GuideOverlayHost::move_anchor(frame, true, None, cx);
            });
        }

        if let Some(anchor) = step.anchor
            && let Some(frame) = anchor_frame
        {
            this.update(cx, |runner, cx| {
                GuideTrace::anchor_resolved(anchor, frame, cx);
                {
                    let mut running = session.borrow_mut();
                    running.last_anchor_frame = Some(frame);
                    running.last_anchor_shape = Some(step.spotlight_shape);
                }
                runner.expect_tap(step, frame, &advance_gate, session, cx);
                Self::report_shape_if_adjusted(anchor, frame, step.spotlight_shape, cx);
                runner.follow(anchor, frame, step, session, &advance_gate, cx);
            })
            .ok();
        }

        let outcome = Self::wait_for_advance(step, advance_gate, event_mark, cx).await;
        this.update(cx, |runner, _| {
            runner.anchor_tracker.stop();
            GuideAnchorTapTracker::stop_expecting();
            runner.menu_watch = None;
        })
        .ok();
        outcome
    }

    /// the event a signal step waits on with its emission count as the step began
    fn event_mark(step: &RiseGuideStepModel) -> Option<(RiseGuideSignal, u64)> {
        if step.advance != RiseGuideAdvance::Signal {
            return None;
        }
        let signal = step.advance_signal.filter(|signal| !signal.is_state())?;
        Some((signal, GuideSignalBus::emission_count(signal)))
    }

    /// whether the step event fired since event_mark was taken
    fn event_came(mark: Option<(RiseGuideSignal, u64)>) -> bool {
        mark.is_some_and(|(signal, count)| GuideSignalBus::emission_count(signal) > count)
    }

    fn state_signal(step: &RiseGuideStepModel) -> Option<RiseGuideSignal> {
        if step.advance != RiseGuideAdvance::Signal {
            return None;
        }
        step.advance_signal.filter(|signal| signal.is_state())
    }

    async fn wait_while_anchor_is_covered(
        this: &WeakEntity<Self>,
        anchor: RiseGuideAnchor,
        step: &RiseGuideStepModel,
        event_mark: Option<(RiseGuideSignal, u64)>,
        session: &GuideRunnerSessionRef,
        cx: &mut AsyncApp,
    ) -> Option<GuideRunnerStepOutcome> {
        if !Self::is_current(this, session, cx) {
            return Some(GuideRunnerStepOutcome::Exited);
        }
        if GuideAnchorRegistry::sighting(anchor) != GuideAnchorSighting::Covered {
            return None;
        }
        cx.update(|cx| GuideOverlayHost::narrate_aside(cx));
        let mut outcome = None;
        while GuideAnchorRegistry::sighting(anchor) == GuideAnchorSighting::Covered {
            if !Self::is_current(this, session, cx) {
                outcome = Some(GuideRunnerStepOutcome::Exited);
                break;
            }
            if let Some(signal) = Self::state_signal(step)
                && GuideFacts::is_true(signal)
            {
                outcome = Some(GuideRunnerStepOutcome::Advanced);
                break;
            }
            if Self::event_came(event_mark) {
                outcome = Some(GuideRunnerStepOutcome::Advanced);
                break;
            }
            if step.advance == RiseGuideAdvance::Screen
                && let Some(screen) = step.advance_screen
                && GuideScreenObserver::wait_for(screen, &HashMap::new(), 0, cx).await
            {
                outcome = Some(GuideRunnerStepOutcome::Advanced);
                break;
            }
            cx.background_executor()
                .timer(millis(GuideAnchorRegistry::follow_interval_ms()))
                .await;
        }
        if Self::is_current(this, session, cx) {
            cx.update(|cx| GuideOverlayHost::set_aside(false, cx));
        } else if outcome.is_none() {
            outcome = Some(GuideRunnerStepOutcome::Exited);
        }
        outcome
    }

    /// looks for the anchor and for a step waiting on a state it watches that state at the same time
    /// the state can come true while the element is still being looked for and then the element
    /// is often gone for that reason
    async fn settle(
        anchor: RiseGuideAnchor,
        within: i64,
        step: &RiseGuideStepModel,
        cx: &AsyncApp,
    ) -> GuideRunnerSettlement {
        let Some(signal) = Self::state_signal(step) else {
            return GuideRunnerSettlement::Frame(GuideAnchorRegistry::settled_frame(anchor, Some(within), cx).await);
        };
        if GuideFacts::is_true(signal) {
            return GuideRunnerSettlement::Satisfied;
        }
        let frame = pin!(async {
            GuideRunnerSettlement::Frame(GuideAnchorRegistry::settled_frame(anchor, Some(within), cx).await)
        });
        let fact = pin!(async {
            if GuideFacts::wait_until_true(signal, Self::WAIT_FOREVER, cx).await {
                GuideRunnerSettlement::Satisfied
            } else {
                GuideRunnerSettlement::Frame(None)
            }
        });
        match select(frame, fact).await {
            Either::Left((first, _)) | Either::Right((first, _)) => first,
        }
    }

    /// scrolls the element into view, the one motion the runner may make for the reader
    /// reduce motion turns off the travel not the reveal
    async fn reveal_anchor(anchor: RiseGuideAnchor, waiting_for_mount: bool, cx: &mut AsyncApp) {
        let animated = cx.update(|cx| GuideOverlayMotion::current(cx).animates_transitions());
        let mount_window = if waiting_for_mount { GuideAnchorScroller::mount_window_ms() } else { 0 };
        let scrolled = GuideAnchorScroller::reveal(anchor, mount_window, animated, cx).await;
        if !scrolled {
            return;
        }
        cx.update(|cx| GuideTrace::anchor_scrolled(anchor, cx));
    }

    /// keeps the light on the element for the life of the step
    /// losing the anchor never ends the step, the overlay stands aside and returns when the
    /// element does
    fn follow(
        &mut self,
        anchor: RiseGuideAnchor,
        frame: Bounds<Pixels>,
        step: &RiseGuideStepModel,
        session: &GuideRunnerSessionRef,
        gate: &GuideEventAwaiter,
        cx: &mut Context<Self>,
    ) {
        let was_lost = Rc::new(Cell::new(false));
        let this = cx.weak_entity();
        let step = step.clone();
        let session = session.clone();
        let gate = gate.clone();
        self.anchor_tracker.follow(
            anchor,
            frame,
            move |report, cx| {
                this.update(cx, |runner, cx| {
                    runner.anchor_did_report(report, anchor, &step, &session, &gate, &was_lost, cx)
                })
                .ok();
            },
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn anchor_did_report(
        &mut self,
        report: GuideAnchorTrackerReport,
        anchor: RiseGuideAnchor,
        step: &RiseGuideStepModel,
        session: &GuideRunnerSessionRef,
        gate: &GuideEventAwaiter,
        was_lost: &Cell<bool>,
        cx: &mut Context<Self>,
    ) {
        if !self.owns(session) {
            tracing::warn!(target: "riseonly::X1738", ?report, "runner report: session not owned");
            return;
        }
        if !matches!(report, GuideAnchorTrackerReport::Moved(_)) {
            tracing::warn!(target: "riseonly::X1738", ?report, anchor = anchor.raw_value(), "runner report");
        }
        match report {
            GuideAnchorTrackerReport::Moved(moved) => {
                self.menu_watch = None;
                if was_lost.get() {
                    was_lost.set(false);
                    GuideTrace::anchor_returned(anchor, moved, cx);
                }
                GuideOverlayHost::move_anchor(moved, false, None, cx);
                session.borrow_mut().last_anchor_frame = Some(moved);
                self.expect_tap(step, moved, gate, session, cx);
            }
            GuideAnchorTrackerReport::Settled(settled) => {
                self.menu_watch = None;
                GuideOverlayHost::move_anchor(settled, true, None, cx);
                session.borrow_mut().last_anchor_frame = Some(settled);
                GuideTrace::anchor_moved(anchor, settled, cx);
                self.expect_tap(step, settled, gate, session, cx);
            }
            GuideAnchorTrackerReport::Lost(sighting) => {
                was_lost.set(true);
                if sighting == GuideAnchorSighting::Covered && GuideSystemMenuProbe::menu_frame().is_some() {
                    self.light_system_menu(anchor, session, cx);
                } else if sighting == GuideAnchorSighting::Covered {
                    GuideOverlayHost::set_aside(true, cx);
                } else {
                    GuideOverlayHost::clear_anchor(cx);
                }
                GuideAnchorTapTracker::stop_expecting();
                GuideTrace::anchor_lost(anchor, sighting, cx);
            }
        }
    }

    fn light_system_menu(
        &mut self,
        anchor: RiseGuideAnchor,
        session: &GuideRunnerSessionRef,
        cx: &mut Context<Self>,
    ) {
        let session = session.clone();
        self.menu_watch = Some(cx.spawn(async move |this, cx| {
            let mut light = GuideRunnerMenuLight::new(None);
            loop {
                if !Self::is_current(&this, &session, cx) {
                    return;
                }
                let menu_gone = cx.update(|cx| match GuideSystemMenuProbe::menu_frame() {
                    Some(menu) => {
                        light.track(menu, cx);
                        false
                    }
                    None => {
                        match GuideAnchorRegistry::sighting(anchor) {
                            GuideAnchorSighting::Covered => GuideOverlayHost::set_aside(true, cx),
                            GuideAnchorSighting::Absent => GuideOverlayHost::clear_anchor(cx),
                            GuideAnchorSighting::Visible(_) => {}
                        }
                        true
                    }
                });
                if menu_gone {
                    this.update(cx, |runner, _| runner.menu_watch = None).ok();
                    return;
                }
                cx.background_executor()
                    .timer(millis(GuideAnchorRegistry::follow_interval_ms()))
                    .await;
            }
        }));
    }

    fn light_menu_opened(
        &mut self,
        anchor_frame: Bounds<Pixels>,
        step: RiseGuideStepModel,
        session: GuideRunnerSessionRef,
        gate: GuideEventAwaiter,
        cx: &mut Context<Self>,
    ) {
        self.menu_watch = Some(cx.spawn(async move |this, cx| {
            let mut light = GuideRunnerMenuLight::new(Some(anchor_frame));
            let deadline = RiseClock::monotonic() + millis(RiseGuideCanon::TIMINGS_ANCHOR_REVEAL_MS);
            loop {
                if !Self::is_current(&this, &session, cx) {
                    return;
                }
                let menu = GuideSystemMenuProbe::menu_frame();
                if let Some(menu) = menu {
                    cx.update(|cx| light.track(menu, cx));
                } else if light.is_shown() || RiseClock::monotonic() >= deadline {
                    let frame = session.borrow().last_anchor_frame.unwrap_or(anchor_frame);
                    let shown = light.is_shown();
                    this.update(cx, |runner, cx| {
                        if shown {
                            GuideOverlayHost::move_anchor(frame, true, None, cx);
                        }
                        runner.menu_watch = None;
                        runner.expect_tap(&step, frame, &gate, &session, cx);
                    })
                    .ok();
                    return;
                }
                cx.background_executor()
                    .timer(millis(GuideAnchorRegistry::follow_interval_ms()))
                    .await;
            }
        }));
    }

    fn expect_tap(
        &self,
        step: &RiseGuideStepModel,
        rect: Bounds<Pixels>,
        gate: &GuideEventAwaiter,
        session: &GuideRunnerSessionRef,
        cx: &mut Context<Self>,
    ) {
        if step.advance == RiseGuideAdvance::TapAnchor {
            let gate = gate.clone();
            GuideAnchorTapTracker::expect(rect, move |_| {
                gate.settle(true);
            });
            return;
        }
        let this = cx.weak_entity();
        let step = step.clone();
        let session = session.clone();
        let gate = gate.clone();
        GuideAnchorTapTracker::expect(rect, move |cx| {
            let (step, session, gate) = (step.clone(), session.clone(), gate.clone());
            this.update(cx, |runner, cx| runner.light_menu_opened(rect, step, session, gate, cx)).ok();
        });
    }

    fn report_shape_if_adjusted(
        anchor: RiseGuideAnchor,
        frame: Bounds<Pixels>,
        requested: RiseGuideSpotlightShape,
        cx: &mut App,
    ) {
        let drawn = GuideSpotlightCutout::new(frame, requested).effective_shape();
        if drawn == requested {
            return;
        }
        GuideTrace::shape_adjusted(anchor, requested, drawn, cx);
    }

    async fn wait_for_advance(
        step: &RiseGuideStepModel,
        gate: GuideEventAwaiter,
        event_mark: Option<(RiseGuideSignal, u64)>,
        cx: &mut AsyncApp,
    ) -> GuideRunnerStepOutcome {
        match step.advance {
            RiseGuideAdvance::TapContinue | RiseGuideAdvance::TapAnchor | RiseGuideAdvance::VideoEnd => {
                let delivered = GuideEventWait::run(gate, Self::WAIT_FOREVER, |awaiter: &GuideEventAwaiter| {
                    awaiter.settle(false);
                }, cx)
                .await;
                GuideAnchorTapTracker::stop_expecting();
                if delivered { GuideRunnerStepOutcome::Advanced } else { GuideRunnerStepOutcome::Exited }
            }
            RiseGuideAdvance::Signal => {
                let Some(signal) = step.advance_signal else {
                    return GuideRunnerStepOutcome::Failed(RiseGuideFailure::StepTimeout);
                };
                if Self::event_came(event_mark) {
                    return GuideRunnerStepOutcome::Advanced;
                }
                cx.update(|cx| GuideTrace::waiting(signal, cx));
                let delivered = GuideConditions::wait(signal, Self::WAIT_FOREVER, cx).await;
                if delivered { GuideRunnerStepOutcome::Advanced } else { GuideRunnerStepOutcome::Exited }
            }
            RiseGuideAdvance::Screen => {
                let Some(screen) = step.advance_screen else {
                    return GuideRunnerStepOutcome::Failed(RiseGuideFailure::StepTimeout);
                };
                let arrived = GuideScreenObserver::wait_for(screen, &HashMap::new(), Self::WAIT_FOREVER, cx).await;
                if arrived { GuideRunnerStepOutcome::Advanced } else { GuideRunnerStepOutcome::Exited }
            }
            RiseGuideAdvance::Delay => {
                cx.background_executor().timer(millis(step.delay_ms.max(0))).await;
                GuideRunnerStepOutcome::Advanced
            }
        }
    }

    fn overlay_step(
        &self,
        step: &RiseGuideStepModel,
        session: &GuideRunnerSessionRef,
        anchor_frame: Option<Bounds<Pixels>>,
        gate: &GuideEventAwaiter,
        cx: &mut Context<Self>,
    ) -> GuideOverlayStep {
        let running = session.borrow();
        let total = running.guide.steps.len();
        let step_label = (total > 1).then(|| {
            format_printf(
                &tr("guide_step_of"),
                &[FormatArg::from((step.position + 1).to_string()), FormatArg::from(total.to_string())],
            )
        });
        let is_last = running.index + 1 == total;
        let continue_title = if is_last { tr("guide_finish") } else { tr("guide_next") };

        let advance_action = (step.advance == RiseGuideAdvance::TapContinue).then(|| {
            let gate = gate.clone();
            GuideOverlayAction::new(continue_title, move |_, _| {
                gate.settle(true);
            })
        });
        let exit = if is_last && advance_action.is_some() {
            None
        } else {
            let this = cx.weak_entity();
            Some(GuideOverlayAction::new(tr("guide_skip"), move |_, cx| {
                let this = this.clone();
                cx.defer(move |cx| {
                    this.update(cx, |runner, cx| runner.finish(RiseGuideAck::Skipped, true, cx)).ok();
                });
            }))
        };

        let title = (!step.title.is_empty()).then(|| step.title.clone());
        let body = Self::desktop_body(&running.guide, step);
        let card = || {
            GuideOverlaySurfaceContent::Card(
                GuideCardContent::new(body.clone())
                    .title(title.clone())
                    .step_label(step_label.clone())
                    .primary(advance_action.clone())
                    .secondary(exit.clone()),
            )
        };
        let surface = match step.surface {
            RiseGuideSurface::None => GuideOverlaySurfaceContent::None,
            RiseGuideSurface::Bubble => GuideOverlaySurfaceContent::Bubble(
                GuideBubbleContent::new(body.clone())
                    .step_label(step_label.clone())
                    .primary(advance_action.clone())
                    .secondary(exit.clone()),
            ),
            RiseGuideSurface::Caption => {
                GuideOverlaySurfaceContent::Caption(GuideCaptionContent::new(body.clone()))
            }
            RiseGuideSurface::Card => card(),
            RiseGuideSurface::Video => match Url::from_string(&step.media_url) {
                Some(url) => {
                    let gate = gate.clone();
                    GuideOverlaySurfaceContent::Video(
                        GuideVideoContent::new(url, body.clone())
                            .title(title.clone())
                            .on_finished(move |_| {
                                gate.settle(true);
                            }),
                    )
                }
                None => card(),
            },
        };

        let previous_cutout = if step.stage == RiseGuideStage::SpotlightPath {
            running.last_anchor_frame.map(|frame| {
                GuideSpotlightCutout::new(
                    frame,
                    running.last_anchor_shape.unwrap_or(RiseGuideSpotlightShape::RoundedRect),
                )
            })
        } else {
            None
        };
        let mut overlay = GuideOverlayStep::new(
            GuideOverlayStepID::new(format!("{}:{}", running.guide.id, step.position)),
            step.stage,
        )
        .anchor(anchor_frame)
        .previous_cutout(previous_cutout)
        .shape(step.spotlight_shape)
        .narrator(step.narrator)
        .pose(step.pose)
        .placement(step.placement)
        .surface(surface)
        .keyboard_height(self.keyboard_height)
        .exit(exit)
        .gesture(step.gesture);
        overlay.compact_body = Self::menu_item_hint(step.anchor);
        overlay
    }

    fn desktop_body(guide: &RiseGuide, step: &RiseGuideStepModel) -> String {
        if step.anchor == Some(RiseGuideAnchor::CallMainVideo) && step.gesture == Some(RiseGuideGesture::DoubleTap) {
            return tr("desktop_guide_call_video_zoom");
        }
        if step.anchor == Some(RiseGuideAnchor::MusicLocalTracks) {
            return tr("desktop_guide_music_intro_local_tracks");
        }
        if step.anchor == Some(RiseGuideAnchor::CallFlipCamera) {
            return tr("desktop_guide_call_flip_camera");
        }
        let has_anchor = |anchor: RiseGuideAnchor| guide.steps.iter().any(|other| other.anchor == Some(anchor));
        let is_local_playlist = has_anchor(RiseGuideAnchor::ChatAudioMessage);
        if step.anchor == Some(RiseGuideAnchor::PlaylistAddLocalTrack) {
            return if is_local_playlist {
                tr("desktop_guide_local_playlist_add_file")
            } else {
                tr("desktop_guide_playlist_intro_add_file")
            };
        }
        let is_screen_share = guide
            .steps
            .iter()
            .any(|other| other.advance_signal == Some(RiseGuideSignal::CallScreenShareOn));
        if is_screen_share && guide.steps.last().map(|last| last.position) == Some(step.position) {
            return tr("desktop_guide_screen_share_outro");
        }
        if has_anchor(RiseGuideAnchor::SessionGrantRequested)
            && step.anchor.is_none()
            && guide.steps.first().map(|first| first.position) != Some(step.position)
            && guide.steps.last().map(|last| last.position) != Some(step.position)
        {
            return tr("desktop_guide_listen_together_effects_heat");
        }
        if !is_local_playlist {
            return step.body.clone();
        }
        if step.anchor == Some(RiseGuideAnchor::ChatAudioMessage)
            && step.advance_signal == Some(RiseGuideSignal::HoldMenuOpened)
        {
            return tr("desktop_guide_local_playlist_hold_track");
        }
        if step.anchor.is_none() {
            if guide.steps.first().map(|first| first.position) == Some(step.position) {
                return tr("desktop_guide_local_playlist_intro");
            }
            if guide.steps.last().map(|last| last.position) == Some(step.position) {
                return tr("desktop_guide_local_playlist_outro");
            }
        }
        step.body.clone()
    }

    pub fn desktop_summary(guide: &RiseGuide) -> String {
        let has_anchor = |anchor: RiseGuideAnchor| guide.steps.iter().any(|step| step.anchor == Some(anchor));
        if has_anchor(RiseGuideAnchor::ChatAudioMessage) && has_anchor(RiseGuideAnchor::PlaylistAddLocalTrack) {
            return tr("desktop_guide_local_playlist_summary");
        }
        if has_anchor(RiseGuideAnchor::MusicLocalTracks) {
            return tr("desktop_guide_music_intro_summary");
        }
        if guide.key == "music_listen_together_host" || has_anchor(RiseGuideAnchor::SessionGrantRequested) {
            return tr("desktop_guide_listen_together_rights_summary");
        }
        guide.summary.clone()
    }

    fn menu_item_hint(anchor: Option<RiseGuideAnchor>) -> Option<String> {
        match anchor {
            Some(RiseGuideAnchor::ProfileAccountMenu) => Some(format_printf(
                &tr("guide_choose_menu_item"),
                &[FormatArg::from(tr("add_account"))],
            )),
            _ => None,
        }
    }

    /// the window the reader is using, frontmost one that can show a step else the active one
    /// else the first open
    fn active_scene(cx: &App) -> Option<AnyWindowHandle> {
        GuideOverlayHost::front_hosting_window(cx)
            .or_else(|| cx.active_window())
            .or_else(|| cx.windows().first().copied())
    }

    fn prefetch_videos(guide: &RiseGuide) {
        let urls: Vec<Url> = guide
            .steps
            .iter()
            .filter(|step| step.surface == RiseGuideSurface::Video)
            .filter_map(|step| Url::from_string(&step.media_url))
            .collect();
        if urls.is_empty() {
            return;
        }
        for url in urls {
            let request = URLRequest::new(url);
            drop(RiseAccountRuntime::current().spawn(async move {
                let _ = URLSession::shared().data_for(request).await;
            }));
        }
    }

    fn finish(&mut self, ack: RiseGuideAck, restoring_navigation: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let (key, origin) = {
            let running = session.borrow();
            (running.guide.key.clone(), running.origin)
        };
        let failure = ack.failure().map(|failure| failure.raw_value());
        GuideTrace::finished(&key, ack.outcome().raw_value(), failure, cx);
        self.teardown(restoring_navigation, cx);
        self.acknowledge(&session, ack, cx);
        cx.defer(move |cx| {
            GuideAutoStart::shared(cx).update(cx, |auto_start, cx| auto_start.guide_did_finish(origin, ack, cx));
        });
    }

    fn teardown(&mut self, restoring_navigation: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let entry = session.borrow().entry_navigation.clone();
        if restoring_navigation && let Some(entry) = entry {
            if HoldContextMenuPresenter::shared(cx).read(cx).active_request().is_some() {
                HoldContextMenuPresenter::dismiss(cx);
            }
            let place = Self::keeping_readers_cover(entry, &session, cx);
            GuideNavigationBridge::restore(place, cx);
        }
        self.task = None;
        self.leave_check = None;
        self.session = None;
        self.running_guide_key = None;
        GuideNavigationBridge::guide_did_end();
        self.remove_observers();
        self.anchor_tracker.stop();
        self.menu_watch = None;
        GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.disarm(cx));
        GuideAnchorCapture::end();
        GuideAnchorScroller::forget_hosts();
        GuideOverlayHost::set_on_interrupted(|_| {}, cx);
        GuideAnchorTapTracker::stop_expecting();
        GuideOverlayHost::dismiss(cx);
        cx.notify();
    }

    pub fn hold_guide_for_bridge_test(&mut self, key: &str) {
        let guide = RiseGuide {
            id: 0,
            key: key.to_owned(),
            revision: 0,
            tier: rise_canon::RiseGuideTier::Hint,
            trigger: RiseGuideTrigger::Manual,
            trigger_screen: None,
            trigger_signal: None,
            reteach: false,
            priority: 0,
            locale: String::new(),
            title: String::new(),
            summary: String::new(),
            min_app_version: String::new(),
            topic: None,
            icon: None,
            steps: Vec::new(),
        };
        self.session = Some(Rc::new(RefCell::new(GuideRunnerSession::new(guide, GuideRunnerOrigin::Manual))));
        self.running_guide_key = Some(key.to_owned());
    }

    pub fn end_held_guide_for_bridge_test(&mut self, cx: &mut Context<Self>) {
        self.teardown(false, cx);
    }

    fn keeping_readers_cover(
        entry: GuideNavigationSnapshot,
        session: &GuideRunnerSessionRef,
        cx: &App,
    ) -> GuideNavigationSnapshot {
        let is_adding_account = AuthServicesStore::try_read(cx).is_some_and(|auth| auth.is_adding_account());
        if entry.adding_account || !is_adding_account {
            return entry;
        }
        let running = session.borrow();
        let reached = (running.index + 1).min(running.guide.steps.len());
        let guide_led_there = running.guide.steps[..reached].iter().any(|step| {
            step.advance == RiseGuideAdvance::Signal
                && step.advance_signal == Some(RiseGuideSignal::AddAccountFlowOpened)
        });
        if guide_led_there {
            return entry;
        }
        GuideNavigationSnapshot {
            account_id: entry.account_id,
            path: entry.path,
            tab: entry.tab,
            adding_account: true,
            creating_chat: entry.creating_chat,
            embedded_forum_chat_id: entry.embedded_forum_chat_id,
        }
    }

    /// writes the receipt fire and forget, repository applies it locally first so the guide
    /// stops being offered whether or not the round trip lands
    fn acknowledge(&self, session: &GuideRunnerSessionRef, ack: RiseGuideAck, cx: &mut Context<Self>) {
        let (guide_id, revision, reached, origin) = {
            let running = session.borrow();
            (running.guide.id, running.guide.revision, running.reached_step, running.origin.trigger())
        };
        let domain = RiseGuidesActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.acknowledge(guide_id, revision, ack, reached, origin).await;
        })
        .detach();
    }

    fn install_observers(&mut self, cx: &mut Context<Self>) {
        let (sender, mut inbox) = mpsc::unbounded::<bool>();
        let background = sender.clone();
        let center = RiseAppEvents::default_center();
        self.lifecycle_observers = vec![
            center.observe(UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, move |_| {
                let _ = background.unbounded_send(true);
            }),
            center.observe(UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION, move |_| {
                let _ = sender.unbounded_send(false);
            }),
        ];
        self.lifecycle_task = Some(cx.spawn(async move |this, cx| {
            while let Some(entered_background) = inbox.next().await {
                let delivered = this.update(cx, |runner, cx| {
                    if entered_background {
                        runner.did_enter_background(cx);
                    } else {
                        runner.will_enter_foreground(cx);
                    }
                });
                if delivered.is_err() {
                    return;
                }
            }
        }));
    }

    fn did_enter_background(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        self.backgrounded_at = Some(RiseClock::now());
        let abort_ms = RiseGuideCanon::TIMINGS_BACKGROUND_ABORT_MS;
        self.background_abort = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(millis(abort_ms)).await;
            this.update(cx, |runner, cx| {
                if runner.backgrounded_at.is_none() {
                    return;
                }
                runner.stop(
                    RiseGuideAck::Failed(RiseGuideFailure::Interrupted),
                    &format!("background {abort_ms}ms"),
                    cx,
                );
            })
            .ok();
        }));
    }

    fn will_enter_foreground(&mut self, cx: &mut Context<Self>) {
        self.background_abort = None;
        let Some(since) = self.backgrounded_at.take() else {
            return;
        };
        let elapsed_ms = (RiseClock::now().time_interval_since(&since) * 1000.0) as i64;
        if self.session.is_none() || elapsed_ms < RiseGuideCanon::TIMINGS_BACKGROUND_ABORT_MS {
            return;
        }
        self.stop(
            RiseGuideAck::Failed(RiseGuideFailure::Interrupted),
            &format!("background {elapsed_ms}ms"),
            cx,
        );
    }

    fn watch_interruptions(&mut self, cx: &mut Context<Self>) {
        let rtc = RiseRTCPresentation::shared(cx);
        let maintenance = MaintenanceGate::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        self.interruption_watch = vec![
            cx.observe(&rtc, |runner, _, cx| runner.check_interruption(cx)),
            cx.observe(&maintenance, |runner, _, cx| runner.check_interruption(cx)),
            cx.observe(&sandbox, |runner, _, cx| runner.check_interruption(cx)),
        ];
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            this.update(cx, |runner, cx| runner.check_interruption(cx)).ok();
        });
    }

    fn check_interruption(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        if let Some(reason) = Self::interruption(cx) {
            self.stop(RiseGuideAck::Failed(RiseGuideFailure::Interrupted), &reason, cx);
        }
    }

    /// why the app cant host a guide right now or none, also read before an automatic start
    pub fn interruption(cx: &mut App) -> Option<String> {
        let rtc = RiseRTCPresentation::shared(cx);
        let sandbox = GuideSandbox::shared(cx);
        let active_room_id = rtc.read(cx).current_active_room().map(|room| room.config.room_id.clone());
        if let Some(room_id) = active_room_id {
            let sandbox = sandbox.read(cx);
            if !sandbox.owns_voice_room_id(Some(room_id.as_str()))
                && !sandbox.owns_call_room_id(Some(room_id.as_str()))
            {
                return Some("call".to_owned());
            }
        }
        if rtc.read(cx).current_incoming_call().is_some() {
            return Some("call".to_owned());
        }
        if MaintenanceGate::shared(cx).read(cx).snapshot().is_closed {
            return Some("maintenance".to_owned());
        }
        None
    }

    fn remove_observers(&mut self) {
        self.lifecycle_observers.clear();
        self.lifecycle_task = None;
        self.interruption_watch.clear();
        self.background_abort = None;
        self.backgrounded_at = None;
        if let Some(observation) = self.screen_observation.take() {
            observation.cancel();
        }
        self.keyboard_height = px(0.0);
    }
}

impl AccountResettable for GuideRunner {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        let key = session.borrow().guide.key.clone();
        GuideTrace::finished(&key, "abandoned", Some("account_reset"), cx);
        self.teardown(false, cx);
    }
}

/// moves the light onto a system menu and re-solves the chrome once the menu stops moving
struct GuideRunnerMenuLight {
    origin: Option<Bounds<Pixels>>,
    shown: Option<Bounds<Pixels>>,
    resolved: bool,
    stable_since: Duration,
}

impl GuideRunnerMenuLight {
    fn new(origin: Option<Bounds<Pixels>>) -> Self {
        Self { origin, shown: None, resolved: false, stable_since: RiseClock::monotonic() }
    }

    fn is_shown(&self) -> bool {
        self.shown.is_some()
    }

    fn track(&mut self, menu: Bounds<Pixels>, cx: &mut App) {
        let now = RiseClock::monotonic();
        if !GuideAnchorRegistry::are_same_frame(self.shown, Some(menu)) {
            GuideOverlayHost::move_anchor(menu, self.shown.is_none(), self.origin, cx);
            self.shown = Some(menu);
            self.resolved = false;
            self.stable_since = now;
        } else if !self.resolved
            && now.saturating_sub(self.stable_since) >= millis(RiseGuideCanon::TIMINGS_ANCHOR_SETTLE_MS)
        {
            GuideOverlayHost::move_anchor(menu, true, self.origin, cx);
            self.resolved = true;
        }
    }
}
