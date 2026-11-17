use std::collections::HashMap;

use gpui::{App, Context, Entity};
use rise_canon::RiseGuideCanon;
use rise_core::foundation::foundation_date::Date;
use rise_i18n::tr;
use rise_platform::haptics::{NotificationFeedbackGenerator, NotificationKind};

use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::guides::engine::guide_auto_start::GuideAutoStart;
use crate::modules::guides::engine::guide_runner::{GuideRunner, GuideRunnerOrigin};
use crate::modules::guides::stores::guides_actions::guides_actions::GuidesActionsStore;
use crate::modules::guides::stores::guides_services::guides_services::GuidesServicesStore;

pub struct GuidesInteractionsStore {
    actions: Entity<GuidesActionsStore>,
    services: Entity<GuidesServicesStore>,
    last_offered_at: HashMap<String, Date>,
    return_row_key: Option<String>,
}

impl GuidesInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            actions: GuidesActionsStore::shared(cx),
            services: GuidesServicesStore::shared(cx),
            last_offered_at: HashMap::new(),
            return_row_key: None,
        }
    }

    pub fn refresh(&mut self, force: bool, cx: &mut Context<Self>) {
        self.actions.update(cx, |actions, cx| actions.refresh_action(force, cx));
    }

    pub fn start(&mut self, key: String, cx: &mut Context<Self>) {
        if self.services.read(cx).is_running(cx) || Self::refused_by_interruption(cx) {
            return;
        }
        self.actions.update(cx, |actions, cx| actions.start_action(key, GuideRunnerOrigin::Manual, cx));
    }

    pub fn start_from_catalogue(&mut self, key: String, cx: &mut Context<Self>) {
        if self.services.read(cx).is_running(cx) || Self::refused_by_interruption(cx) {
            return;
        }
        self.return_row_key = Some(key.clone());
        self.actions.update(cx, |actions, cx| actions.start_action(key, GuideRunnerOrigin::Manual, cx));
    }

    pub fn take_return_row(&mut self, keys: &[String]) -> Option<String> {
        let key = self.return_row_key.as_ref()?;
        if !keys.contains(key) {
            return None;
        }
        self.return_row_key.take()
    }

    pub fn start_id(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.services.read(cx).is_running(cx) || Self::refused_by_interruption(cx) {
            return;
        }
        self.actions.update(cx, |actions, cx| actions.start_action_id(id, GuideRunnerOrigin::Manual, cx));
    }

    pub fn offer_from_empty_state(&mut self, key: String, cx: &mut Context<Self>) {
        if self.services.read(cx).is_running(cx) || !self.can_offer(&key) {
            return;
        }
        self.note_offered(&key);
        self.actions.update(cx, |actions, cx| actions.start_action(key, GuideRunnerOrigin::EmptyState, cx));
    }

    pub fn set_auto_start(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if enabled == self.services.read(cx).auto_start(cx) {
            return;
        }
        self.actions.update(cx, |actions, cx| actions.set_auto_start_action(enabled, cx));
    }

    /// called right after this device created the account, the welcome is owed to it
    pub fn note_account_created(&mut self, user_id: &str, cx: &mut Context<Self>) {
        GuideAutoStart::note_account_created(user_id, cx);
    }

    /// true if the account was created here and its welcome is still ahead, it opens in the feed
    pub fn is_welcome_owed(&self, user_id: &str) -> bool {
        GuideAutoStart::is_welcome_owed(user_id)
    }

    pub fn start_watching_screens(&mut self, cx: &mut Context<Self>) {
        self.actions.update(cx, |actions, cx| actions.start_watching_triggers_action(cx));
    }

    pub fn stop_watching_screens(&mut self, cx: &mut Context<Self>) {
        self.actions.update(cx, |actions, cx| actions.stop_watching_triggers_action(cx));
    }

    /// a guide started during a call is dropped at once by the runner
    /// a manual start says why, the call matters more so its never ended
    fn refused_by_interruption(cx: &mut App) -> bool {
        let Some(reason) = GuideRunner::interruption(cx) else {
            return false;
        };
        if reason == "call" {
            let _ = NotificationFeedbackGenerator::new().notification_occurred(NotificationKind::Error);
            Notifier::send(
                tr("guide_unavailable_in_call"),
                NotifierSendOptions {
                    r#type: NotifierType::Warning,
                    position: NotifierPosition::Bottom,
                    icon: NotifierIcon::System("phone.fill".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        }
        true
    }

    fn can_offer(&self, key: &str) -> bool {
        let Some(last) = self.last_offered_at.get(key) else {
            return true;
        };
        let cooldown = RiseGuideCanon::TIMINGS_REOFFER_COOLDOWN_MS as f64 / 1000.0;
        Date::now().time_interval_since(last) >= cooldown
    }

    fn note_offered(&mut self, key: &str) {
        self.last_offered_at.insert(key.to_owned(), Date::now());
    }
}
