use std::collections::HashMap;

use gpui::{App, Context};
use serde_json::{Map, Value};

use super::notify_services::NotifyServicesStore;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::push_notifications::push_notification_manager_hooks::PushNotificationManagerHooks;
use crate::modules::notify::engine::core::rise_notification_settings_outbox::RiseNotificationSettingsOutbox;
use crate::modules::notify::stores::notify_actions::notify_actions::NotifyActionsStore;
use crate::modules::notify::stores::notify_actions::notify_settings_types::NotificationSettingsModel;
use crate::modules::notify::stores::notify_actions::notify_wire_types::{
    NotificationSettingsGatewayDTO, NotificationSettingsGatewayModel,
};

const NOTIFICATION_SETTINGS_CONFLICT_CODE: &str = "409";

pub struct NotificationSettingsMutation {
    pub user_id: String,
    pub patch: Map<String, Value>,
    pub rollback_patch: Map<String, Value>,
    pub field_revisions: HashMap<String, u64>,
}

impl NotifyServicesStore {
    pub fn notification_settings_cache_key() -> String {
        SaiCache::new(["getNotificationSettings"]).key
    }

    fn notification_settings_dto(observed: bool, cx: &mut App) -> Option<NotificationSettingsGatewayDTO> {
        let key = Self::notification_settings_cache_key();
        let payload = if observed {
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
        SaiJSONCodec::decode::<NotificationSettingsGatewayDTO>(Some(&payload))
    }

    pub fn reload_notification_settings_dto(&mut self, cx: &mut Context<Self>) {
        let revisions = SaiCacheRevisionTracker::shared(cx);
        self.observed_notification_settings_revision =
            revisions.read(cx).revision(&Self::notification_settings_cache_key());
        self.observed_notification_settings_dto = Self::notification_settings_dto(true, cx);
    }

    pub fn system_notifications_denied(&self, cx: &App) -> bool {
        PushNotificationManagerHooks::system_authorization_denied(cx)
    }

    pub fn notification_settings(&self, _cx: &App) -> NotificationSettingsModel {
        self.observed_notification_settings_dto
            .as_ref()
            .map(|dto| dto.to_model())
            .unwrap_or_default()
    }

    pub fn notification_settings_has_data(&self, _cx: &App) -> bool {
        self.observed_notification_settings_dto.is_some()
    }

    pub fn notification_settings_did_load(&self, cx: &App) -> bool {
        matches!(
            self.actions.read(cx).get_notification_settings_sai.status(),
            SaiWsSliceStatus::Fulfilled
        )
    }

    pub fn notification_settings_pending(&self, cx: &App) -> bool {
        self.actions.read(cx).get_notification_settings_sai.is_pending()
    }

    pub fn notification_settings_error_text(&self, cx: &App) -> Option<String> {
        self.actions.read(cx).get_notification_settings_sai.error()
    }

    pub fn notification_settings_failed(&self, cx: &App) -> bool {
        !self.notification_settings_has_data(cx)
            && matches!(
                self.actions.read(cx).get_notification_settings_sai.status(),
                SaiWsSliceStatus::Rejected
            )
    }

    pub fn notification_settings_loading(&self, cx: &App) -> bool {
        if self.notification_settings_has_data(cx) {
            return false;
        }
        self.actions.read(cx).get_notification_settings_sai.is_pending()
    }

    pub(super) fn fields_in_patch(source: &Map<String, Value>, patch: &Map<String, Value>) -> Map<String, Value> {
        source
            .iter()
            .filter(|(key, _)| patch.contains_key(key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    pub(super) fn revisions_in_patch(source: &HashMap<String, u64>, patch: &Map<String, Value>) -> HashMap<String, u64> {
        source
            .iter()
            .filter(|(key, _)| patch.contains_key(key.as_str()))
            .map(|(key, revision)| (key.clone(), *revision))
            .collect()
    }

    pub fn stage_notification_settings_patch(&mut self, patch: Map<String, Value>, cx: &mut Context<Self>) {
        let key = Self::notification_settings_cache_key();
        let current = SaiWsResponseCache::current_payload(
            &key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
        let Some(current_root) = AnyDictionaryBridge::dictionary(current.as_ref()) else {
            return;
        };

        for (field, value) in &patch {
            if !self.pending_notification_settings_patch.contains_key(field)
                && let Some(previous) = current_root.get(field)
            {
                self.pending_notification_settings_rollback_patch
                    .insert(field.clone(), previous.clone());
            }
            self.notification_settings_revision = self.notification_settings_revision.wrapping_add(1);
            self.notification_settings_field_revisions
                .insert(field.clone(), self.notification_settings_revision);
            self.pending_notification_settings_patch.insert(field.clone(), value.clone());
        }
        self.save_unsent_notification_settings(cx);
        let _ = SaiUpdater::patch_root(&key, &patch, SaiUpdaterWriteScope::All, cx);
        cx.notify();
    }

    pub fn take_pending_notification_settings_mutation(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<NotificationSettingsMutation> {
        if self.pending_notification_settings_patch.is_empty() {
            return None;
        }
        let patch = std::mem::take(&mut self.pending_notification_settings_patch);
        let rollback_patch = Self::fields_in_patch(&self.pending_notification_settings_rollback_patch, &patch);
        let field_revisions = Self::revisions_in_patch(&self.notification_settings_field_revisions, &patch);
        self.pending_notification_settings_rollback_patch.clear();
        self.in_flight_notification_settings_patch = patch.clone();
        Some(NotificationSettingsMutation {
            user_id: Self::effective_user_id_for_wire(cx).unwrap_or_default(),
            patch,
            rollback_patch,
            field_revisions,
        })
    }

    pub fn finish_notification_settings_mutation(
        &mut self,
        mutation: &NotificationSettingsMutation,
        succeeded: bool,
        cx: &mut Context<Self>,
    ) {
        let effective = Self::effective_user_id_for_wire(cx);
        if effective.as_deref() != Some(mutation.user_id.as_str()) {
            RiseNotificationSettingsOutbox::forget(&mutation.patch, &mutation.user_id);
            return;
        }
        self.in_flight_notification_settings_patch.clear();
        self.save_unsent_notification_settings(cx);
        let key = Self::notification_settings_cache_key();
        if !succeeded {
            let rollback: Map<String, Value> = mutation
                .rollback_patch
                .iter()
                .filter(|(field, _)| {
                    self.notification_settings_field_revisions.get(field.as_str())
                        == mutation.field_revisions.get(field.as_str())
                })
                .map(|(field, value)| (field.clone(), value.clone()))
                .collect();
            if !rollback.is_empty() {
                let _ = SaiUpdater::patch_root(&key, &rollback, SaiUpdaterWriteScope::All, cx);
            }
        }

        if !self.pending_notification_settings_patch.is_empty() {
            let _ = SaiUpdater::patch_root(
                &key,
                &self.pending_notification_settings_patch,
                SaiUpdaterWriteScope::All,
                cx,
            );
        }
        cx.notify();
    }

    fn requeue_notification_settings_mutation(&mut self, mutation: &NotificationSettingsMutation, cx: &mut Context<Self>) {
        if Self::effective_user_id_for_wire(cx).as_deref() != Some(mutation.user_id.as_str()) {
            return;
        }
        self.in_flight_notification_settings_patch.clear();
        for (field, value) in &mutation.patch {
            if self.pending_notification_settings_patch.contains_key(field) {
                continue;
            }
            if let Some(previous) = mutation.rollback_patch.get(field) {
                self.pending_notification_settings_rollback_patch
                    .insert(field.clone(), previous.clone());
            }
            self.pending_notification_settings_patch.insert(field.clone(), value.clone());
        }
        self.save_unsent_notification_settings(cx);
        self.reapply_pending_notification_settings_patch(cx);
        cx.notify();
    }

    /// queues the disk outbox again
    /// a value equal to the cached root is still sent
    /// after a relaunch the cache may hold a value the server never got
    pub fn restore_unsent_notification_settings(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        let unsent = RiseNotificationSettingsOutbox::load(&user_id);
        if unsent.is_empty() {
            return;
        }
        tracing::warn!(target: "riseonly::X1405", "restore unsent fields={unsent:?}");
        let current = SaiWsResponseCache::current_payload(
            &Self::notification_settings_cache_key(),
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
        let server_root = AnyDictionaryBridge::dictionary(current.as_ref()).unwrap_or_default();
        for (field, value) in unsent {
            if self.pending_notification_settings_patch.contains_key(&field)
                || self.in_flight_notification_settings_patch.contains_key(&field)
            {
                continue;
            }
            if let Some(previous) = server_root.get(&field)
                && previous.as_bool() != Some(value)
            {
                self.pending_notification_settings_rollback_patch
                    .insert(field.clone(), previous.clone());
            }
            self.notification_settings_revision = self.notification_settings_revision.wrapping_add(1);
            self.notification_settings_field_revisions
                .insert(field.clone(), self.notification_settings_revision);
            self.pending_notification_settings_patch.insert(field, Value::Bool(value));
        }
        self.save_unsent_notification_settings(cx);
    }

    fn save_unsent_notification_settings(&self, cx: &mut App) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        let mut fields = self.in_flight_notification_settings_patch.clone();
        fields.extend(self.pending_notification_settings_patch.clone());
        RiseNotificationSettingsOutbox::save(&fields, &user_id);
    }

    pub fn reapply_pending_notification_settings_patch(&mut self, cx: &mut Context<Self>) {
        let mut fields = self.in_flight_notification_settings_patch.clone();
        fields.extend(self.pending_notification_settings_patch.clone());
        if fields.is_empty() {
            return;
        }
        let _ = SaiUpdater::patch_root(
            &Self::notification_settings_cache_key(),
            &fields,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    pub fn notification_settings_error_requires_revalidation(&self, error: &anyhow::Error) -> bool {
        let code = CanonErrors::failure(error).and_then(|failure| failure.code);
        if code.as_deref() == Some(NOTIFICATION_SETTINGS_CONFLICT_CODE) {
            return true;
        }
        let text = error.to_string().to_lowercase();
        text.contains("conflict") || text.contains("version mismatch") || text.contains("revision mismatch")
    }

    pub fn notification_settings_error_handler(&self, error: &anyhow::Error, cx: &mut App) {
        let description = error.to_string();
        Sai::log(
            "Notify",
            move || format!("get_notification_settings error: {description}"),
            SaiLogKind::Network,
            cx,
        );
    }

    pub fn update_notification_settings_error_handler(
        &mut self,
        error: &anyhow::Error,
        mutation: &NotificationSettingsMutation,
        cx: &mut Context<Self>,
    ) -> bool {
        let description = error.to_string();
        Sai::log(
            "Notify",
            move || format!("update_notification_settings error: {description}"),
            SaiLogKind::Network,
            cx,
        );
        let server_verdict = CanonErrors::failure(error).is_some();
        tracing::warn!(
            target: "riseonly::X1405",
            "update failed connection_loss={} server_verdict={server_verdict} error={error:?}",
            CanonErrors::is_connection_loss(error)
        );
        if CanonErrors::is_connection_loss(error) || !server_verdict {
            self.requeue_notification_settings_mutation(mutation, cx);
            return true;
        }
        self.finish_notification_settings_mutation(mutation, false, cx);
        if self.notification_settings_error_requires_revalidation(error) {
            cx.defer(NotifyActionsStore::get_notification_settings_action);
            return true;
        }
        false
    }
}
