use gpui::{App, Context};
use rise_canon::rise_key;
use serde_json::{Map, Value};

use super::notify_services::NotifyServicesStore;
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::notify::stores::notify_actions::notify_profile_settings_wire_types::{
    ProfileNotificationSettingsGatewayDTO, ProfileNotificationSettingsGatewayModel, ProfileNotificationSettingsModel,
    ProfileNotificationSettingsMutation,
};

impl NotifyServicesStore {
    pub fn profile_notification_settings_cache_key(target_user_id: &str) -> String {
        SaiCache::new(["getProfileNotificationSettings", target_user_id]).key
    }

    fn profile_notification_settings_dto(
        &mut self,
        target_user_id: &str,
        observed: bool,
        cx: &mut Context<Self>,
    ) -> Option<ProfileNotificationSettingsGatewayDTO> {
        let key = Self::profile_notification_settings_cache_key(target_user_id);
        let payload = if observed {
            let revisions = SaiCacheRevisionTracker::shared(cx);
            let revision = revisions.read(cx).revision(&key);
            self.observed_profile_notification_settings_revisions
                .insert(key.clone(), revision);
            SaiWsResponseCache::observed_payload(
                &key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
        } else {
            SaiWsResponseCache::current_payload(
                &key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
        }?;
        SaiJSONCodec::decode::<ProfileNotificationSettingsGatewayDTO>(Some(&payload))
    }

    pub fn profile_notification_settings(
        &mut self,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<ProfileNotificationSettingsModel> {
        if let Some(practice) = GuideSandbox::shared(cx).read(cx).person_notifications(target_user_id) {
            return Some(practice);
        }
        self.profile_notification_settings_dto(target_user_id, true, cx)
            .map(|dto| dto.to_model())
    }

    pub fn profile_notification_settings_has_data(&mut self, target_user_id: &str, cx: &mut Context<Self>) -> bool {
        self.profile_notification_settings_dto(target_user_id, true, cx).is_some()
    }

    pub fn profile_notification_settings_pending(&self, cx: &App) -> bool {
        self.actions.read(cx).get_profile_notification_settings_sai.is_pending()
    }

    pub fn profile_notification_settings_failed(&self, cx: &App) -> bool {
        matches!(
            self.actions.read(cx).get_profile_notification_settings_sai.status(),
            SaiWsSliceStatus::Rejected
        )
    }

    pub fn stage_profile_notification_settings_patch(
        &mut self,
        target_user_id: &str,
        patch: Map<String, Value>,
        cx: &mut Context<Self>,
    ) {
        let key = Self::profile_notification_settings_cache_key(target_user_id);
        let current = SaiWsResponseCache::current_payload(
            &key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
        let Some(current_root) = AnyDictionaryBridge::dictionary(current.as_ref()) else {
            return;
        };

        let mut pending = self
            .pending_profile_notification_patches
            .get(target_user_id)
            .cloned()
            .unwrap_or_default();
        let mut rollback = self
            .pending_profile_notification_rollbacks
            .get(target_user_id)
            .cloned()
            .unwrap_or_default();
        let mut revisions = self
            .profile_notification_field_revisions
            .get(target_user_id)
            .cloned()
            .unwrap_or_default();
        for (field, value) in &patch {
            if !pending.contains_key(field)
                && let Some(previous) = current_root.get(field)
            {
                rollback.insert(field.clone(), previous.clone());
            }
            self.profile_notification_revision = self.profile_notification_revision.wrapping_add(1);
            revisions.insert(field.clone(), self.profile_notification_revision);
            pending.insert(field.clone(), value.clone());
        }
        self.pending_profile_notification_patches
            .insert(target_user_id.to_owned(), pending);
        self.pending_profile_notification_rollbacks
            .insert(target_user_id.to_owned(), rollback);
        self.profile_notification_field_revisions
            .insert(target_user_id.to_owned(), revisions);

        let mut fields = patch;
        fields.insert(rise_key::notify::HAS_OVERRIDE.to_owned(), Value::Bool(true));
        let _ = SaiUpdater::patch_root(&key, &fields, SaiUpdaterWriteScope::All, cx);
        cx.notify();
    }

    pub fn take_pending_profile_notification_settings_mutation(
        &mut self,
        target_user_id: &str,
    ) -> Option<ProfileNotificationSettingsMutation> {
        let patch = self.pending_profile_notification_patches.get(target_user_id)?;
        if patch.is_empty() {
            return None;
        }
        let patch = patch.clone();
        let rollback_patch = self
            .pending_profile_notification_rollbacks
            .get(target_user_id)
            .map(|rollback| Self::fields_in_patch(rollback, &patch))
            .unwrap_or_default();
        let field_revisions = self
            .profile_notification_field_revisions
            .get(target_user_id)
            .map(|revisions| Self::revisions_in_patch(revisions, &patch))
            .unwrap_or_default();
        self.pending_profile_notification_patches.remove(target_user_id);
        self.pending_profile_notification_rollbacks.remove(target_user_id);
        self.in_flight_profile_notification_patches
            .insert(target_user_id.to_owned(), patch.clone());
        Some(ProfileNotificationSettingsMutation {
            target_user_id: target_user_id.to_owned(),
            patch,
            rollback_patch,
            field_revisions,
        })
    }

    pub fn finish_profile_notification_settings_mutation(
        &mut self,
        mutation: &ProfileNotificationSettingsMutation,
        succeeded: bool,
        cx: &mut Context<Self>,
    ) {
        let target = mutation.target_user_id.as_str();
        let key = Self::profile_notification_settings_cache_key(target);
        self.in_flight_profile_notification_patches.remove(target);
        if !succeeded {
            let revisions = self.profile_notification_field_revisions.get(target);
            let rollback: Map<String, Value> = mutation
                .rollback_patch
                .iter()
                .filter(|(field, _)| {
                    revisions.and_then(|revisions| revisions.get(field.as_str()))
                        == mutation.field_revisions.get(field.as_str())
                })
                .map(|(field, value)| (field.clone(), value.clone()))
                .collect();
            if !rollback.is_empty() {
                let _ = SaiUpdater::patch_root(&key, &rollback, SaiUpdaterWriteScope::All, cx);
            }
        }

        if let Some(pending) = self.pending_profile_notification_patches.get(target)
            && !pending.is_empty()
        {
            let _ = SaiUpdater::patch_root(&key, pending, SaiUpdaterWriteScope::All, cx);
        }
        cx.notify();
    }

    pub fn reapply_pending_profile_notification_settings_patch(
        &mut self,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) {
        let mut fields = self
            .in_flight_profile_notification_patches
            .get(target_user_id)
            .cloned()
            .unwrap_or_default();
        if let Some(pending) = self.pending_profile_notification_patches.get(target_user_id) {
            fields.extend(pending.clone());
        }
        if fields.is_empty() {
            return;
        }
        let _ = SaiUpdater::patch_root(
            &Self::profile_notification_settings_cache_key(target_user_id),
            &fields,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }
}
