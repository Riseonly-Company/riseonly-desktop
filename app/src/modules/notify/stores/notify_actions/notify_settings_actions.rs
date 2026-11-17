use std::any::Any;
use std::rc::Rc;

use gpui::App;

use super::notify_actions::NotifyActionsStore;
use super::notify_wire_types::UpdateNotificationSettingsPatch;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::modules::notify::stores::notify_interactions::notify_settings_interactions::NotificationSettingsDrain;
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;
use crate::modules::notify::stores::notify_services::notify_settings_services::NotificationSettingsMutation;

impl NotifyActionsStore {
    pub fn get_notification_settings_action(cx: &mut App) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };

        let body = rise_canon::rise_request::notify::GetNotificationSettings {};
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::notify::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::notify::GET_NOTIFICATION_SETTINGS.to_owned(),
            cache_key: Some(NotifyServicesStore::notification_settings_cache_key()),
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

        let on_success: SaiWsOnSuccess = Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
            Self::slices_did_change(cx);
            if Self::effective_user_id_for_wire(cx).as_deref() != Some(user_id.as_str()) {
                return;
            }
            NotifyServicesStore::shared(cx).update(cx, |service, cx| {
                service.restore_unsent_notification_settings(cx);
                service.reapply_pending_notification_settings_patch(cx);
            });
            NotificationSettingsDrain::drain_notification_settings_mutations(cx);
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::slices_did_change(cx);
            NotifyServicesStore::shared(cx).update(cx, |service, cx| {
                service.notification_settings_error_handler(error, cx);
            });
        });

        let slice = Self::shared(cx).read(cx).get_notification_settings_sai.clone();
        SaiWs::request_slice(&slice, body, options, Some(on_success), Some(on_error), cx);
        Self::slices_did_change(cx);
    }

    pub fn update_notification_settings_action(mutation: NotificationSettingsMutation, cx: &mut App) {
        if Self::effective_user_id_for_wire(cx).is_none() {
            NotifyServicesStore::shared(cx).update(cx, |service, cx| {
                service.finish_notification_settings_mutation(&mutation, false, cx);
            });
            return;
        }
        if mutation.patch.is_empty() {
            return;
        }

        let body = rise_canon::rise_request::notify::UpdateNotificationSettings::from_patch(&mutation.patch);
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::notify::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::notify::UPDATE_NOTIFICATION_SETTINGS.to_owned(),
            cache_key: Some(NotifyServicesStore::notification_settings_cache_key()),
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            force_bypass_cache_read: true,
            ..SaiWsRequestOptionsInit::default()
        });

        let mutation = Rc::new(mutation);
        let on_success: SaiWsOnSuccess = {
            let mutation = Rc::clone(&mutation);
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                Self::slices_did_change(cx);
                NotifyServicesStore::shared(cx).update(cx, |service, cx| {
                    service.finish_notification_settings_mutation(&mutation, true, cx);
                });
                NotificationSettingsDrain::drain_notification_settings_mutations(cx);
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::slices_did_change(cx);
            let is_revalidating = NotifyServicesStore::shared(cx).update(cx, |service, cx| {
                service.update_notification_settings_error_handler(error, &mutation, cx)
            });
            if !is_revalidating {
                NotificationSettingsDrain::drain_notification_settings_mutations(cx);
            }
        });

        let slice = Self::shared(cx).read(cx).update_notification_settings_sai.clone();
        SaiWs::request_slice(&slice, body, options, Some(on_success), Some(on_error), cx);
        Self::slices_did_change(cx);
    }
}
