use std::any::Any;
use std::rc::Rc;

use gpui::App;

use super::notify_actions::NotifyActionsStore;
use super::notify_profile_settings_wire_types::{
    ProfileNotificationSettingsMutation, UpdateProfileNotificationSettingsPatch,
};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;
use crate::modules::notify::stores::notify_interactions::notify_profile_settings_interactions::ProfileNotificationSettingsDrain;
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;

impl NotifyActionsStore {
    pub fn get_profile_notification_settings_action(target_user_id: &str, cx: &mut App) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if target_user_id.is_empty() || target_user_id == user_id {
            return;
        }

        let body = rise_canon::rise_request::notify::GetProfileNotificationSettings {
            target_user_id: Some(target_user_id.to_owned()),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::notify::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::notify::GET_PROFILE_NOTIFICATION_SETTINGS.to_owned(),
            cache_key: Some(NotifyServicesStore::profile_notification_settings_cache_key(target_user_id)),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            ..SaiWsRequestOptionsInit::default()
        });

        let target = target_user_id.to_owned();
        let on_success: SaiWsOnSuccess = Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
            Self::slices_did_change(cx);
            NotifyServicesStore::shared(cx).update(cx, |service, cx| {
                service.reapply_pending_profile_notification_settings_patch(&target, cx);
            });
            ProfileNotificationSettingsDrain::drain_profile_notification_settings_mutations(&target, cx);
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::slices_did_change(cx);
            let description = error.to_string();
            Sai::log(
                "Notify",
                move || format!("get_profile_notification_settings error: {description}"),
                SaiLogKind::Network,
                cx,
            );
        });

        let slice = Self::shared(cx).read(cx).get_profile_notification_settings_sai.clone();
        SaiWs::request_slice(&slice, body, options, Some(on_success), Some(on_error), cx);
        Self::slices_did_change(cx);
    }

    pub fn update_profile_notification_settings_action(mutation: ProfileNotificationSettingsMutation, cx: &mut App) {
        if Self::effective_user_id_for_wire(cx).is_none() {
            NotifyServicesStore::shared(cx).update(cx, |service, cx| {
                service.finish_profile_notification_settings_mutation(&mutation, false, cx);
            });
            return;
        }
        if mutation.patch.is_empty() {
            return;
        }

        let body = rise_canon::rise_request::notify::UpdateProfileNotificationSettings::from_patch(
            &mutation.target_user_id,
            &mutation.patch,
        );
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::notify::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::notify::UPDATE_PROFILE_NOTIFICATION_SETTINGS.to_owned(),
            cache_key: Some(NotifyServicesStore::profile_notification_settings_cache_key(
                &mutation.target_user_id,
            )),
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            ..SaiWsRequestOptionsInit::default()
        });

        let mutation = Rc::new(mutation);
        let on_success: SaiWsOnSuccess = {
            let mutation = Rc::clone(&mutation);
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                Self::slices_did_change(cx);
                NotifyServicesStore::shared(cx).update(cx, |service, cx| {
                    service.finish_profile_notification_settings_mutation(&mutation, true, cx);
                });
                ProfileNotificationSettingsDrain::drain_profile_notification_settings_mutations(
                    &mutation.target_user_id,
                    cx,
                );
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::slices_did_change(cx);
            let description = error.to_string();
            Sai::log(
                "Notify",
                move || format!("update_profile_notification_settings error: {description}"),
                SaiLogKind::Network,
                cx,
            );
            NotifyServicesStore::shared(cx).update(cx, |service, cx| {
                service.finish_profile_notification_settings_mutation(&mutation, false, cx);
            });
            Self::get_profile_notification_settings_action(&mutation.target_user_id, cx);
        });

        let slice = Self::shared(cx).read(cx).update_profile_notification_settings_sai.clone();
        SaiWs::request_slice(&slice, body, options, Some(on_success), Some(on_error), cx);
        Self::slices_did_change(cx);
    }
}
