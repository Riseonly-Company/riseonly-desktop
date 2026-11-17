use std::collections::HashMap;

use gpui::{App, Context, Entity, Subscription};
use serde_json::{Map, Value};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::notify::engine::rise_announcements_presentation::RiseAnnouncementsPresentation;
use crate::modules::notify::engine::rise_notifications_presentation::{
    RiseNotificationsLoadState, RiseNotificationsPresentation, RiseNotificationsSnapshot,
};
use crate::modules::notify::engine::rise_notify_unread_presentation::RiseNotifyUnreadPresentation;
use crate::modules::notify::engine::rise_release_notes_presentation::RiseReleaseNotesPresentation;
use crate::modules::notify::stores::notify_actions::notify_actions::NotifyActionsStore;
use crate::modules::notify::stores::notify_actions::notify_wire_types::NotificationSettingsGatewayDTO;

pub struct NotifyServicesStore {
    pub pending_notification_settings_patch: Map<String, Value>,
    pub pending_notification_settings_rollback_patch: Map<String, Value>,
    pub in_flight_notification_settings_patch: Map<String, Value>,
    pub notification_settings_field_revisions: HashMap<String, u64>,
    pub notification_settings_revision: u64,
    pub pending_profile_notification_patches: HashMap<String, Map<String, Value>>,
    pub pending_profile_notification_rollbacks: HashMap<String, Map<String, Value>>,
    pub in_flight_profile_notification_patches: HashMap<String, Map<String, Value>>,
    pub profile_notification_field_revisions: HashMap<String, HashMap<String, u64>>,
    pub profile_notification_revision: u64,
    pub(super) actions: Entity<NotifyActionsStore>,
    pub(super) announcements_presentation: Entity<RiseAnnouncementsPresentation>,
    pub(super) release_notes_presentation: Entity<RiseReleaseNotesPresentation>,
    notifications_presentation: Entity<RiseNotificationsPresentation>,
    unread_presentation: Entity<RiseNotifyUnreadPresentation>,
    pub(super) observed_notification_settings_dto: Option<NotificationSettingsGatewayDTO>,
    pub(super) observed_notification_settings_revision: i64,
    pub(super) observed_profile_notification_settings_revisions: HashMap<String, i64>,
    _subscriptions: Vec<Subscription>,
}

impl NotifyServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let actions = NotifyActionsStore::shared(cx);
        let announcements_presentation = RiseAnnouncementsPresentation::shared(cx);
        let release_notes_presentation = RiseReleaseNotesPresentation::shared(cx);
        let notifications_presentation = RiseNotificationsPresentation::shared(cx);
        let unread_presentation = RiseNotifyUnreadPresentation::shared(cx);
        let revisions = SaiCacheRevisionTracker::shared(cx);
        let subscriptions = vec![
            cx.observe(&actions, |_, _, cx| cx.notify()),
            cx.observe(&announcements_presentation, |_, _, cx| cx.notify()),
            cx.observe(&release_notes_presentation, |_, _, cx| cx.notify()),
            cx.observe(&notifications_presentation, |_, _, cx| cx.notify()),
            cx.observe(&unread_presentation, |_, _, cx| cx.notify()),
            cx.observe(&revisions, |store: &mut Self, revisions, cx| {
                store.cache_revisions_did_change(&revisions, cx);
            }),
        ];
        let mut store = Self {
            pending_notification_settings_patch: Map::new(),
            pending_notification_settings_rollback_patch: Map::new(),
            in_flight_notification_settings_patch: Map::new(),
            notification_settings_field_revisions: HashMap::new(),
            notification_settings_revision: 0,
            pending_profile_notification_patches: HashMap::new(),
            pending_profile_notification_rollbacks: HashMap::new(),
            in_flight_profile_notification_patches: HashMap::new(),
            profile_notification_field_revisions: HashMap::new(),
            profile_notification_revision: 0,
            actions,
            announcements_presentation,
            release_notes_presentation,
            notifications_presentation,
            unread_presentation,
            observed_notification_settings_dto: None,
            observed_notification_settings_revision: 0,
            observed_profile_notification_settings_revisions: HashMap::new(),
            _subscriptions: subscriptions,
        };
        store.reload_notification_settings_dto(cx);
        store
    }

    fn cache_revisions_did_change(&mut self, revisions: &Entity<SaiCacheRevisionTracker>, cx: &mut Context<Self>) {
        let revisions = revisions.read(cx);
        let mut did_change = false;
        for (key, observed) in &mut self.observed_profile_notification_settings_revisions {
            let revision = revisions.revision(key);
            if revision != *observed {
                *observed = revision;
                did_change = true;
            }
        }
        let settings_revision = revisions.revision(&Self::notification_settings_cache_key());
        if settings_revision != self.observed_notification_settings_revision {
            self.reload_notification_settings_dto(cx);
            did_change = true;
        }
        if did_change {
            cx.notify();
        }
    }

    pub fn effective_user_id_for_wire(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|user_id| !user_id.is_empty())
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.pending_notification_settings_patch.clear();
        self.pending_notification_settings_rollback_patch.clear();
        self.in_flight_notification_settings_patch.clear();
        self.notification_settings_field_revisions.clear();
        self.notification_settings_revision = 0;
        self.pending_profile_notification_patches.clear();
        self.pending_profile_notification_rollbacks.clear();
        self.in_flight_profile_notification_patches.clear();
        self.profile_notification_field_revisions.clear();
        self.profile_notification_revision = 0;
        self.observed_profile_notification_settings_revisions.clear();
        self.reload_notification_settings_dto(cx);
        cx.notify();
    }

    pub fn notifications_snapshot<'a>(&self, cx: &'a App) -> &'a RiseNotificationsSnapshot {
        self.notifications_presentation.read(cx).snapshot()
    }

    pub fn total_unread(&self, cx: &App) -> i64 {
        self.notifications_snapshot(cx).total_unread
    }

    pub fn has_unread(&self, cx: &App) -> bool {
        self.notifications_snapshot(cx).has_unread()
    }

    pub fn notifications_has_more(&self, cx: &App) -> bool {
        self.notifications_snapshot(cx).has_more
    }

    pub fn notifications_has_newer(&self, cx: &App) -> bool {
        self.notifications_snapshot(cx).has_newer
    }

    pub fn notifications_did_load(&self, cx: &App) -> bool {
        matches!(self.notifications_snapshot(cx).load_state, RiseNotificationsLoadState::Loaded)
    }

    pub fn unread_badge_count(&self, cx: &App) -> i64 {
        self.unread_presentation.read(cx).unread_count()
    }

    pub fn handle_register_token_error(&self, error: &anyhow::Error, cx: &mut App) {
        let description = error.to_string();
        Sai::log(
            "Notify",
            move || format!("device token registration failed: {description}"),
            SaiLogKind::Network,
            cx,
        );
    }
}
