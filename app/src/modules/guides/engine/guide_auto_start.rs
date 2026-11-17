use std::collections::{HashMap, HashSet};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Subscription, Task};
use rise_canon::{RiseGuideCanon, RiseGuideScreen, RiseGuideSignal, RiseGuideTrigger};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::GUIDES_WELCOME_OWED_PREFIX;
use rise_ui::top_presenter::TopPresenter;

use crate::app::rise_launch_splash::RiseLaunchSplashState;
use crate::core::desktop_platform::rise_app_events::{
    RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication, UIResponder,
};
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_screen_resolver::{GuideScreenObservation, GuideScreenObserver, GuideScreenResolver};
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::guides::guide_trace::GuideTrace;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{SaiActiveScreen, SaiNavigationBridge};
use crate::core::widgets::overlay::achievement_earned_store::AchievementEarnedStore;
use crate::core::widgets::overlay::premium_gift_celebration_store::PremiumGiftCelebrationStore;
use crate::core::widgets::overlay::premium_promo_store::PremiumPromoStore;
use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat_search::chat_search_overlay_presenter::ChatSearchOverlayPresenter;
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet_state::CreateChatSheetState;
use crate::modules::guides::engine::guide_runner::{GuideRunner, GuideRunnerOrigin};
use crate::modules::guides::engine::rise_guides_models::{RiseGuide, RiseGuideAck, RiseGuideAutoStartPolicy};
use crate::modules::guides::engine::rise_guides_presentation::RiseGuidesPresentation;
use crate::modules::notify::engine::rise_launch_interruptions::RiseLaunchInterruptions;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

/// starts guides nobody asked for, on a screen or fact or once for a fresh account
/// the one place deciding if the panda may appear unasked, never when tips are off
/// only when quiet and in budget, not after another one
pub struct GuideAutoStart {
    is_watching: bool,
    screen_observation: Option<GuideScreenObservation>,
    catalogue_watch: Option<Subscription>,
    catalogue_revision: Option<u64>,
    lifecycle_observers: Vec<RiseSubscription>,
    lifecycle_inbox: Option<Task<()>>,
    keyboard_visible: bool,

    screen_guides: HashMap<RiseGuideScreen, Vec<RiseGuide>>,
    signal_guides: HashMap<RiseGuideSignal, Vec<RiseGuide>>,
    welcome_guides: Vec<RiseGuide>,

    pending: Option<GuideAutoStartCandidate>,
    attempts: Option<Task<()>>,
    last_automatic_end: Option<Date>,
    paused_until: Option<Date>,
}

struct GuideAutoStartCandidate {
    guide: RiseGuide,
    origin: GuideRunnerOrigin,
    screen: Option<RiseGuideScreen>,
    fact: Option<RiseGuideSignal>,
    on_tab_root: bool,
}

enum GuideAutoStartAttempt {
    Started,
    Dropped,
    Waiting(String),
}

#[derive(Clone, Copy)]
enum GuideAutoStartLifecycle {
    KeyboardWillShow,
    KeyboardWillHide,
    WillResignActive,
    DidBecomeActive,
}

impl GuideAutoStart {
    const SETTLE: Duration = Duration::from_millis(RiseGuideCanon::TIMINGS_TRIGGER_SETTLE_MS.unsigned_abs());
    const WELCOME_ROOTS: [RiseGuideScreen; 3] = [RiseGuideScreen::Home, RiseGuideScreen::Search, RiseGuideScreen::Chats];
    const MINIMUM_GAP: TimeInterval = RiseGuideCanon::TIMINGS_AUTO_START_MIN_GAP_MS as TimeInterval / 1000.0;
    const PAUSE_AFTER_SKIP: TimeInterval = RiseGuideCanon::TIMINGS_AUTO_PAUSE_AFTER_SKIP_MS as TimeInterval / 1000.0;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            is_watching: false,
            screen_observation: None,
            catalogue_watch: None,
            catalogue_revision: None,
            lifecycle_observers: Vec::new(),
            lifecycle_inbox: None,
            keyboard_visible: false,
            screen_guides: HashMap::new(),
            signal_guides: HashMap::new(),
            welcome_guides: Vec::new(),
            pending: None,
            attempts: None,
            last_automatic_end: None,
            paused_until: None,
        }
    }

    pub fn start_watching(&mut self, cx: &mut Context<Self>) {
        if self.is_watching {
            return;
        }
        self.is_watching = true;
        AccountStateResetRegistry::register(&cx.entity(), cx);
        let this = cx.entity().downgrade();
        self.screen_observation = Some(GuideScreenObserver::observe(move |screen: &SaiActiveScreen, cx: &mut App| {
            let this = this.clone();
            let screen = screen.clone();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| this.screen_did_change(&screen, cx)).ok();
            });
        }));
        self.install_observers(cx);
        self.watch_catalogue(cx);
        cx.notify();
    }

    pub fn stop_watching(&mut self, cx: &mut Context<Self>) {
        if !self.is_watching {
            return;
        }
        self.is_watching = false;
        if let Some(observation) = self.screen_observation.take() {
            observation.cancel();
        }
        self.catalogue_watch = None;
        self.catalogue_revision = None;
        self.remove_observers();
        self.drop_pending();
        self.install_triggers(HashMap::new(), HashMap::new(), Vec::new(), cx);
        cx.notify();
    }

    /// the device just created this account so the welcome is owed until finished or skipped
    /// kept per account on the device since only it knows the account was born here, server
    /// receipt still decides
    pub fn note_account_created(user_id: &str, cx: &mut App) {
        let account = trimming_whitespaces_and_newlines(user_id);
        if account.is_empty() {
            return;
        }
        UserDefaults::standard().set(&Self::welcome_owed_key(account), true);
        Self::trace("auto-start: welcome owed to a new account", cx);
        cx.defer(|cx| {
            let shared = Self::shared(cx);
            shared.update(cx, |this, cx| this.reconsider_current_place(cx));
        });
    }

    /// diagnostic trace, lives in the sai log only so the tester log never sees why the welcome
    /// stayed away
    fn trace(message: &str, cx: &mut App) {
        tracing::warn!(target: "riseonly::X1176", "{message}");
        GuideTrace::note(message, cx);
    }

    pub fn is_welcome_owed(account: &str) -> bool {
        UserDefaults::standard().bool(&Self::welcome_owed_key(account))
    }

    fn settle_welcome(account: &str) {
        UserDefaults::standard().remove_object(&Self::welcome_owed_key(account));
    }

    fn welcome_owed_key(account: &str) -> String {
        [GUIDES_WELCOME_OWED_PREFIX, account].concat()
    }

    pub fn guide_did_finish(&mut self, origin: GuideRunnerOrigin, ack: RiseGuideAck, cx: &mut Context<Self>) {
        if origin != GuideRunnerOrigin::Manual {
            self.last_automatic_end = Some(RiseClock::now());
            if ack == RiseGuideAck::Skipped {
                self.paused_until = Some(RiseClock::now().adding_time_interval(Self::PAUSE_AFTER_SKIP));
                Self::trace("auto-start paused after a skip", cx);
            }
        }
        if origin == GuideRunnerOrigin::FirstLaunch
            && matches!(ack, RiseGuideAck::Completed | RiseGuideAck::Skipped)
            && let Some(account) = AuthServicesStore::try_read(cx).and_then(|auth| auth.effective_user_id_for_wire())
        {
            Self::settle_welcome(&account);
        }
        cx.notify();
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            this.update(cx, |this, cx| this.reconsider_current_place(cx)).ok();
        });
    }

    fn watch_catalogue(&mut self, cx: &mut Context<Self>) {
        let presentation = RiseGuidesPresentation::shared(cx);
        self.catalogue_revision = Some(presentation.read(cx).snapshot().revision);
        self.catalogue_watch = Some(cx.observe(&presentation, |this, presentation, cx| {
            let revision = presentation.read(cx).snapshot().revision;
            if this.catalogue_revision == Some(revision) {
                return;
            }
            this.catalogue_revision = Some(revision);
            this.rebuild_triggers(cx);
        }));
        self.rebuild_triggers(cx);
    }

    fn rebuild_triggers(&mut self, cx: &mut Context<Self>) {
        let presentation = RiseGuidesPresentation::shared(cx);
        let (auto_start, catalogue) = {
            let snapshot = presentation.read(cx).snapshot();
            (snapshot.auto_start, snapshot.catalogue.clone())
        };
        if !auto_start && let Some(pending) = &self.pending {
            let message = format!("auto-start dropped {}: tips are off", pending.guide.key);
            Self::trace(&message, cx);
            self.drop_pending();
        }
        let mut screens: HashMap<RiseGuideScreen, Vec<RiseGuide>> = HashMap::new();
        let mut signals: HashMap<RiseGuideSignal, Vec<RiseGuide>> = HashMap::new();
        let mut welcome: Vec<RiseGuide> = Vec::new();
        for guide in catalogue {
            match guide.trigger {
                RiseGuideTrigger::ScreenEnter => {
                    if let Some(screen) = guide.trigger_screen {
                        screens.entry(screen).or_default().push(guide);
                    }
                }
                RiseGuideTrigger::Signal => {
                    if let Some(signal) = guide.trigger_signal {
                        signals.entry(signal).or_default().push(guide);
                    }
                }
                RiseGuideTrigger::FirstLaunch => welcome.push(guide),
                RiseGuideTrigger::Manual | RiseGuideTrigger::EmptyState => {}
            }
        }
        tracing::warn!(target: "riseonly::X1176", auto_start, welcome = welcome.len(), "catalogue rebuilt");
        self.install_triggers(screens, signals, welcome, cx);
        self.reconsider_current_place(cx);
        cx.notify();
    }

    fn install_triggers(
        &mut self,
        screens: HashMap<RiseGuideScreen, Vec<RiseGuide>>,
        signals: HashMap<RiseGuideSignal, Vec<RiseGuide>>,
        welcome: Vec<RiseGuide>,
        cx: &mut Context<Self>,
    ) {
        let states: HashSet<RiseGuideSignal> = signals.keys().copied().filter(|signal| signal.is_state()).collect();
        let events: HashSet<RiseGuideSignal> = signals.keys().copied().filter(|signal| !signal.is_state()).collect();
        self.screen_guides = screens;
        self.signal_guides = signals;
        self.welcome_guides = welcome;
        let facts_target = cx.entity().downgrade();
        GuideFacts::install_triggers(states, move |fact, cx| {
            let this = facts_target.clone();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| this.fact_became_true(fact, cx)).ok();
            });
        });
        let events_target = cx.entity().downgrade();
        GuideSignalBus::install_triggers(
            events,
            move |event, cx| {
                let this = events_target.clone();
                cx.defer(move |cx| {
                    this.update(cx, |this, cx| this.event_fired(event, cx)).ok();
                });
            },
            cx,
        );
    }

    /// level not edge, a fact that already holds and the screen the reader is on count when a
    /// catalogue arrives or a guide ends
    /// since no second edge is coming
    fn reconsider_current_place(&mut self, cx: &mut Context<Self>) {
        if !self.is_watching {
            return;
        }
        if let Some(screen) = GuideScreenResolver::current_active_screen(cx) {
            let guides = self.screen_guides.get(&screen).cloned().unwrap_or_default();
            self.propose(&guides, GuideRunnerOrigin::ScreenEnter, Some(screen), None, false, cx);
        }
        let holding: Vec<(RiseGuideSignal, Vec<RiseGuide>)> = self
            .signal_guides
            .iter()
            .filter(|(fact, _)| fact.is_state() && GuideFacts::is_true(**fact))
            .map(|(fact, guides)| (*fact, guides.clone()))
            .collect();
        for (fact, guides) in holding {
            self.propose(&guides, GuideRunnerOrigin::Signal, None, Some(fact), false, cx);
        }
        self.propose_welcome_if_owed(cx);
    }

    /// the welcome, when this device created the current account and the reader stands on a tab
    /// root it may begin on
    fn propose_welcome_if_owed(&mut self, cx: &mut Context<Self>) {
        if self.welcome_guides.is_empty() {
            return;
        }
        let Some(account) = AuthServicesStore::try_read(cx).and_then(|auth| auth.effective_user_id_for_wire()) else {
            tracing::warn!(target: "riseonly::X1176", "welcome skipped: no account");
            return;
        };
        if !Self::is_welcome_owed(&account) || !Self::is_on_welcome_root(cx) {
            let screen = GuideScreenResolver::current_active_screen(cx).map(|screen| screen.raw_value());
            let raw = SaiNavigationBridge::current_screen(cx).map(|active| active.id);
            tracing::warn!(
                target: "riseonly::X1176",
                owed = Self::is_welcome_owed(&account),
                ?screen,
                ?raw,
                "welcome skipped"
            );
            return;
        }
        let welcome = self.welcome_guides.clone();
        self.propose(&welcome, GuideRunnerOrigin::FirstLaunch, None, None, true, cx);
    }

    fn is_on_welcome_root(cx: &mut App) -> bool {
        GuideScreenResolver::current_active_screen(cx).is_some_and(|screen| Self::WELCOME_ROOTS.contains(&screen))
    }

    fn screen_did_change(&mut self, active: &SaiActiveScreen, cx: &mut Context<Self>) {
        let screen = GuideScreenResolver::screen(&active.id);
        let left = self.pending.as_ref().and_then(|pending| {
            pending
                .screen
                .filter(|place| Some(*place) != screen)
                .map(|place| format!("auto-start dropped {}: the reader left {}", pending.guide.key, place.raw_value()))
        });
        if let Some(message) = left {
            Self::trace(&message, cx);
            self.drop_pending();
        }
        if let Some(screen) = screen
            && let Some(guides) = self.screen_guides.get(&screen).cloned()
        {
            self.propose(&guides, GuideRunnerOrigin::ScreenEnter, Some(screen), None, false, cx);
        }
        self.propose_welcome_if_owed(cx);
        cx.notify();
    }

    fn fact_became_true(&mut self, fact: RiseGuideSignal, cx: &mut Context<Self>) {
        let Some(guides) = self.signal_guides.get(&fact).cloned() else { return };
        self.propose(&guides, GuideRunnerOrigin::Signal, None, Some(fact), false, cx);
    }

    fn event_fired(&mut self, event: RiseGuideSignal, cx: &mut Context<Self>) {
        let Some(guides) = self.signal_guides.get(&event).cloned() else { return };
        let screen = GuideScreenResolver::current_active_screen(cx);
        self.propose(&guides, GuideRunnerOrigin::Signal, screen, None, false, cx);
    }

    /// first guide the policy lets start becomes the candidate
    /// a newer proposal replaces an older one since it describes where the reader is now
    fn propose(
        &mut self,
        guides: &[RiseGuide],
        origin: GuideRunnerOrigin,
        screen: Option<RiseGuideScreen>,
        fact: Option<RiseGuideSignal>,
        on_tab_root: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.is_watching || guides.is_empty() || Self::running_guide(cx) {
            return;
        }
        let Some(guide) = guides.iter().find(|guide| Self::allows_auto_start(guide, cx)) else {
            if origin == GuideRunnerOrigin::FirstLaunch {
                let presentation = RiseGuidesPresentation::shared(cx);
                let snapshot = presentation.read(cx).snapshot();
                let receipts: Vec<String> =
                    guides.iter().map(|guide| format!("{}={:?}", guide.key, snapshot.receipts.get(&guide.id))).collect();
                tracing::warn!(target: "riseonly::X1176", auto_start = snapshot.auto_start, ?receipts, "welcome refused by policy");
            }
            return;
        };
        if self.pending.as_ref().is_some_and(|pending| pending.guide.id == guide.id) {
            return;
        }
        let message = format!("auto-start candidate {} from {}", guide.key, origin.trigger().raw_value());
        self.pending = Some(GuideAutoStartCandidate {
            guide: guide.clone(),
            origin,
            screen,
            fact,
            on_tab_root,
        });
        Self::trace(&message, cx);
        self.schedule_attempts(cx);
        cx.notify();
    }

    /// tries after the canon settle window and again every window while the app is busy
    /// ends when the candidate starts or stops applying or the app leaves the foreground
    fn schedule_attempts(&mut self, cx: &mut Context<Self>) {
        self.attempts = Some(cx.spawn(async move |this, cx| {
            let mut last_reason = String::new();
            loop {
                cx.background_executor().timer(Self::SETTLE).await;
                let Ok(outcome) = this.update(cx, |this, cx| this.attempt(cx)) else { return };
                match outcome {
                    GuideAutoStartAttempt::Started | GuideAutoStartAttempt::Dropped => return,
                    GuideAutoStartAttempt::Waiting(reason) => {
                        if reason != last_reason {
                            let message = format!("auto-start waits: {reason}");
                            if this.update(cx, |_, cx| Self::trace(&message, cx)).is_err() {
                                return;
                            }
                            last_reason = reason;
                        }
                    }
                }
            }
        }));
    }

    fn attempt(&mut self, cx: &mut Context<Self>) -> GuideAutoStartAttempt {
        let Some(candidate) = self.pending.as_ref() else {
            return GuideAutoStartAttempt::Dropped;
        };
        if !Self::still_applies(candidate, cx) {
            let message = format!("auto-start dropped {}: no longer applies", candidate.guide.key);
            self.pending = None;
            Self::trace(&message, cx);
            cx.notify();
            return GuideAutoStartAttempt::Dropped;
        }
        if let Some(reason) = self.budget_reason().or_else(|| self.busy_reason(cx)) {
            return GuideAutoStartAttempt::Waiting(reason);
        }
        let allowed = Self::allows_auto_start(&candidate.guide, cx);
        let Some(candidate) = self.pending.take() else {
            return GuideAutoStartAttempt::Dropped;
        };
        cx.notify();
        if !allowed {
            return GuideAutoStartAttempt::Dropped;
        }
        let message = format!("auto-start {} from {}", candidate.guide.key, candidate.origin.trigger().raw_value());
        Self::trace(&message, cx);
        let GuideAutoStartCandidate { guide, origin, .. } = candidate;
        cx.defer(move |cx| {
            let runner = GuideRunner::shared(cx);
            runner.update(cx, |runner, cx| runner.start(guide, origin, cx));
        });
        GuideAutoStartAttempt::Started
    }

    fn still_applies(candidate: &GuideAutoStartCandidate, cx: &mut App) -> bool {
        if let Some(fact) = candidate.fact
            && !GuideFacts::is_true(fact)
        {
            return false;
        }
        if let Some(screen) = candidate.screen
            && GuideScreenResolver::current_active_screen(cx) != Some(screen)
        {
            return false;
        }
        if candidate.on_tab_root && !Self::is_on_welcome_root(cx) {
            return false;
        }
        true
    }

    fn allows_auto_start(guide: &RiseGuide, cx: &mut App) -> bool {
        let presentation = RiseGuidesPresentation::shared(cx);
        let snapshot = presentation.read(cx).snapshot();
        if !snapshot.auto_start {
            return false;
        }
        RiseGuideAutoStartPolicy::allows_auto_start(
            snapshot.receipts.get(&guide.id),
            guide.revision,
            guide.reteach,
            (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64,
        )
    }

    fn running_guide(cx: &mut App) -> bool {
        let runner = GuideRunner::shared(cx);
        runner.read(cx).running_guide_key().is_some()
    }

    fn budget_reason(&self) -> Option<String> {
        let now = RiseClock::now();
        if let Some(paused_until) = self.paused_until
            && now < paused_until
        {
            return Some("paused after a skip".to_owned());
        }
        if let Some(last_automatic_end) = self.last_automatic_end
            && now.time_interval_since(&last_automatic_end) < Self::MINIMUM_GAP
        {
            return Some("gap after the previous automatic guide".to_owned());
        }
        None
    }

    fn busy_reason(&self, cx: &mut App) -> Option<String> {
        if Self::running_guide(cx) {
            return Some("a guide is running".to_owned());
        }
        if let Some(reason) = GuideRunner::interruption(cx) {
            return Some(reason);
        }
        if cx.active_window().is_none() && !crate::app::dev_drive::DevDrive::is_stealth(cx) {
            return Some("not active".to_owned());
        }
        let has_session = AuthServicesStore::try_read(cx)
            .is_some_and(|auth| auth.is_authenticated() && !auth.is_account_blocked());
        let user_services = UserServicesStore::shared(cx);
        if !has_session || !user_services.read(cx).account_session_is_published(cx) {
            return Some("no session".to_owned());
        }
        if RiseLaunchSplashState::is_visible() {
            return Some("launch artwork".to_owned());
        }
        if self.keyboard_visible {
            return Some("keyboard".to_owned());
        }
        let chat_search = ChatSearchOverlayPresenter::shared(cx);
        let create_chat = CreateChatSheetState::shared(cx);
        if chat_search.read(cx).is_open() || create_chat.read(cx).is_open() {
            return Some("chat search or new chat".to_owned());
        }
        if TopPresenter::has_presented(cx) {
            return Some("presented sheet or alert".to_owned());
        }
        let achievement = AchievementEarnedStore::shared(cx);
        if achievement.read(cx).is_visible() {
            return Some("achievement celebration".to_owned());
        }
        let gift = PremiumGiftCelebrationStore::shared(cx);
        if gift.read(cx).is_visible() {
            return Some("gift celebration".to_owned());
        }
        let interruptions = RiseLaunchInterruptions::shared(cx);
        if interruptions.read(cx).is_owed(cx) {
            return Some("release notes or announcement".to_owned());
        }
        if user_services.read(cx).should_present_contact_sync_onboarding() {
            return Some("contact sync".to_owned());
        }
        let promo = PremiumPromoStore::shared(cx);
        if promo.read(cx).is_presented() {
            return Some("premium promo".to_owned());
        }
        None
    }

    fn drop_pending(&mut self) {
        self.pending = None;
        self.attempts = None;
    }

    fn install_observers(&mut self, cx: &mut Context<Self>) {
        let (sender, mut inbox) = mpsc::unbounded::<GuideAutoStartLifecycle>();
        let center = RiseAppEvents::default_center();
        let observe = |name: RiseNotificationName, signal: GuideAutoStartLifecycle| {
            let sender = sender.clone();
            center.observe(name, move |_| {
                let _ = sender.unbounded_send(signal);
            })
        };
        self.lifecycle_observers = vec![
            observe(UIResponder::KEYBOARD_WILL_SHOW_NOTIFICATION, GuideAutoStartLifecycle::KeyboardWillShow),
            observe(UIResponder::KEYBOARD_WILL_HIDE_NOTIFICATION, GuideAutoStartLifecycle::KeyboardWillHide),
            observe(UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION, GuideAutoStartLifecycle::WillResignActive),
            observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, GuideAutoStartLifecycle::DidBecomeActive),
        ];
        self.lifecycle_inbox = Some(cx.spawn(async move |this, cx| {
            while let Some(signal) = inbox.next().await {
                if this.update(cx, |this, cx| this.lifecycle_did_change(signal, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn lifecycle_did_change(&mut self, signal: GuideAutoStartLifecycle, cx: &mut Context<Self>) {
        match signal {
            GuideAutoStartLifecycle::KeyboardWillShow => self.keyboard_visible = true,
            GuideAutoStartLifecycle::KeyboardWillHide => self.keyboard_visible = false,
            GuideAutoStartLifecycle::WillResignActive => self.attempts = None,
            GuideAutoStartLifecycle::DidBecomeActive => {
                if self.pending.is_some() {
                    self.schedule_attempts(cx);
                }
            }
        }
        cx.notify();
    }

    fn remove_observers(&mut self) {
        self.lifecycle_observers.clear();
        self.lifecycle_inbox = None;
        self.keyboard_visible = false;
    }
}

impl AccountResettable for GuideAutoStart {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.drop_pending();
        self.last_automatic_end = None;
        self.paused_until = None;
        cx.notify();
    }
}
