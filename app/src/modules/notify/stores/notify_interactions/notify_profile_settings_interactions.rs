use gpui::{App, Context};
use rise_canon::{RiseTimingCanon, rise_key};
use serde_json::{Map, Value};

use super::notify_interactions::NotifyInteractionsStore;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::modules::notify::stores::notify_services::notify_guide_sandbox_hooks::notify_guide_sandbox_hooks;
use crate::modules::notify::stores::notify_actions::notify_actions::NotifyActionsStore;
use crate::modules::notify::stores::notify_actions::notify_profile_settings_wire_types::{
    ProfileNotificationScope, ProfileNotificationSettingsDebounce,
};

const PROFILE_NOTIFICATION_SETTINGS_MAX_CACHE_DATA: i64 = 50;

pub enum ProfileNotificationSettingsDrain {}

impl ProfileNotificationSettingsDrain {
    pub fn drain_profile_notification_settings_mutations(target_user_id: &str, cx: &mut App) {
        if NotifyActionsStore::shared(cx)
            .read(cx)
            .update_profile_notification_settings_sai
            .is_pending()
        {
            return;
        }
        let mutation = NotifyInteractionsStore::service(cx).update(cx, |service, _| {
            service.take_pending_profile_notification_settings_mutation(target_user_id)
        });
        let Some(mutation) = mutation else {
            return;
        };
        NotifyActionsStore::update_profile_notification_settings_action(mutation, cx);
    }
}

impl NotifyInteractionsStore {
    fn guide_owns_person_notifications(target_user_id: &str, cx: &mut App) -> bool {
        (notify_guide_sandbox_hooks().owns_person_notifications)(target_user_id, cx)
    }

    pub fn ensure_profile_notification_settings_loaded(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        if target_user_id.is_empty() {
            return;
        }
        if Self::guide_owns_person_notifications(target_user_id, cx) {
            return;
        }
        if NotifyActionsStore::shared(cx)
            .read(cx)
            .get_profile_notification_settings_sai
            .is_pending()
        {
            return;
        }
        NotifyActionsStore::get_profile_notification_settings_action(target_user_id, cx);
    }

    pub fn reload_profile_notification_settings(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        NotifyActionsStore::get_profile_notification_settings_action(target_user_id, cx);
    }

    pub fn set_profile_notification_scope(
        &mut self,
        target_user_id: &str,
        scope: ProfileNotificationScope,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if Self::guide_owns_person_notifications(target_user_id, cx) {
            (notify_guide_sandbox_hooks().set_person_notification)(scope, enabled, cx);
            return;
        }
        let patch = Map::from_iter([(scope.wire_key().to_owned(), Value::Bool(enabled))]);
        Self::service(cx).update(cx, |service, cx| {
            service.stage_profile_notification_settings_patch(target_user_id, patch, cx);
        });
        self.schedule_profile_notification_settings_flush(target_user_id, cx);
    }

    pub fn set_all_profile_notifications(&mut self, target_user_id: &str, enabled: bool, cx: &mut Context<Self>) {
        if Self::guide_owns_person_notifications(target_user_id, cx) {
            (notify_guide_sandbox_hooks().set_all_person_notifications)(enabled, cx);
            return;
        }
        let patch: Map<String, Value> = ProfileNotificationScope::ALL_CASES
            .iter()
            .map(|scope| (scope.wire_key().to_owned(), Value::Bool(enabled)))
            .collect();
        Self::service(cx).update(cx, |service, cx| {
            service.stage_profile_notification_settings_patch(target_user_id, patch, cx);
        });
        self.schedule_profile_notification_settings_flush(target_user_id, cx);
    }

    pub fn set_mute_all_profiles(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let patch = Map::from_iter([(rise_key::notify::MUTE_ALL_PROFILES.to_owned(), Value::Bool(enabled))]);
        Self::service(cx).update(cx, |service, cx| service.stage_notification_settings_patch(patch, cx));
        self.flush_notification_settings(cx);
    }

    pub fn flush_profile_notification_settings(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        if Self::guide_owns_person_notifications(target_user_id, cx) {
            return;
        }
        SaiDebouncer::flush(target_user_id, ProfileNotificationSettingsDebounce::GROUP_KEY, cx);
        ProfileNotificationSettingsDrain::drain_profile_notification_settings_mutations(target_user_id, cx);
    }

    fn schedule_profile_notification_settings_flush(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        let target = target_user_id.to_owned();
        SaiDebouncer::debounced_action_delay_milliseconds(
            target_user_id,
            Box::new(move |cx: &mut App| {
                ProfileNotificationSettingsDrain::drain_profile_notification_settings_mutations(&target, cx);
            }),
            RiseTimingCanon::debounce("profile_notification_settings").client_debounce_ms,
            ProfileNotificationSettingsDebounce::GROUP_KEY,
            PROFILE_NOTIFICATION_SETTINGS_MAX_CACHE_DATA,
            None,
            cx,
        );
    }
}
