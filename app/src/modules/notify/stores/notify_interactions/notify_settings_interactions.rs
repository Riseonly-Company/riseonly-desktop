use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp, Context, Global};
use rise_canon::{RiseTimingCanon, rise_key};
use serde_json::{Map, Value};

use super::notify_interactions::NotifyInteractionsStore;
use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SAI_WS_DID_CONNECT;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::push_notifications::push_notification_manager_hooks::PushNotificationManagerHooks;
use crate::modules::notify::engine::core::rise_notification_settings_outbox::RiseNotificationSettingsOutbox;
use crate::modules::notify::stores::notify_actions::notify_actions::NotifyActionsStore;
use crate::modules::notify::stores::notify_actions::notify_settings_types::{
    MusicNotificationSetting, NotificationCategory, NotificationSettingsDebounce,
};
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;

const NOTIFICATION_SETTINGS_MAX_CACHE_DATA: i64 = 50;

pub enum NotificationSettingsDrain {}

impl NotificationSettingsDrain {
    pub fn drain_notification_settings_mutations(cx: &mut App) {
        if NotifyActionsStore::shared(cx).read(cx).update_notification_settings_sai.is_pending() {
            return;
        }
        let mutation = NotifyServicesStore::shared(cx)
            .update(cx, |service, cx| service.take_pending_notification_settings_mutation(cx));
        let Some(mutation) = mutation else {
            return;
        };
        NotifyActionsStore::update_notification_settings_action(mutation, cx);
    }
}

fn notification_settings_patch(key: &str, value: bool) -> Map<String, Value> {
    Map::from_iter([(key.to_owned(), Value::Bool(value))])
}

impl NotifyInteractionsStore {
    pub fn ensure_notification_settings_loaded(&mut self, cx: &mut Context<Self>) {
        NotificationSettingsReconnectRetry::install_if_needed(cx);
        let service = Self::service(cx);
        service.update(cx, |service, cx| {
            service.restore_unsent_notification_settings(cx);
            service.reapply_pending_notification_settings_patch(cx);
            service.reload_notification_settings_dto(cx);
        });
        if service.read(cx).notification_settings_did_load(cx) {
            NotificationSettingsDrain::drain_notification_settings_mutations(cx);
            return;
        }
        if NotifyActionsStore::shared(cx).read(cx).get_notification_settings_sai.is_pending() {
            return;
        }
        NotifyActionsStore::get_notification_settings_action(cx);
    }

    pub fn start_notification_settings_outbox(&mut self, cx: &mut Context<Self>) {
        NotificationSettingsReconnectRetry::install_if_needed(cx);
        NotificationSettingsReconnectRetry::resend_unsent_if_needed(cx);
    }

    pub fn reload_notification_settings(&mut self, cx: &mut Context<Self>) {
        NotifyActionsStore::get_notification_settings_action(cx);
    }

    pub fn refresh_system_notification_permission(&mut self, cx: &mut Context<Self>) {
        PushNotificationManagerHooks::refresh_system_authorization(cx);
    }

    pub fn open_system_notification_settings(&mut self, cx: &mut Context<Self>) {
        rise_platform::notifications::open_notification_settings(cx);
    }

    fn stage_notification_settings_patch(patch: Map<String, Value>, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.stage_notification_settings_patch(patch, cx));
    }

    pub fn set_all_notifications_enabled(&mut self, value: bool, cx: &mut Context<Self>) {
        Self::stage_notification_settings_patch(
            notification_settings_patch(rise_key::notify::ALL_ENABLED, value),
            cx,
        );
        self.schedule_notification_settings_flush(cx);
    }

    pub fn set_notification_category(&mut self, category: NotificationCategory, enabled: bool, cx: &mut Context<Self>) {
        Self::stage_notification_settings_patch(notification_settings_patch(category.wire_key(), enabled), cx);
        self.schedule_notification_settings_flush(cx);
    }

    pub fn set_music_notifications_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        Self::stage_notification_settings_patch(
            notification_settings_patch(rise_key::notify::MUSIC_ENABLED, enabled),
            cx,
        );
        self.schedule_notification_settings_flush(cx);
    }

    pub fn set_music_notification_setting(
        &mut self,
        setting: MusicNotificationSetting,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        Self::stage_notification_settings_patch(notification_settings_patch(setting.wire_key(), enabled), cx);
        self.schedule_notification_settings_flush(cx);
    }

    pub fn flush_notification_settings(&mut self, cx: &mut Context<Self>) {
        SaiDebouncer::flush(
            NotificationSettingsDebounce::ID,
            NotificationSettingsDebounce::GROUP_KEY,
            cx,
        );
        NotificationSettingsDrain::drain_notification_settings_mutations(cx);
    }

    fn schedule_notification_settings_flush(&mut self, cx: &mut Context<Self>) {
        SaiDebouncer::debounced_action_delay_milliseconds(
            NotificationSettingsDebounce::ID,
            Box::new(NotificationSettingsDrain::drain_notification_settings_mutations),
            RiseTimingCanon::debounce("notification_settings").client_debounce_ms,
            NotificationSettingsDebounce::GROUP_KEY,
            NOTIFICATION_SETTINGS_MAX_CACHE_DATA,
            None,
            cx,
        );
    }
}

struct NotificationSettingsReconnectRetryObserver;

impl Global for NotificationSettingsReconnectRetryObserver {}

enum NotificationSettingsReconnectRetry {}

impl NotificationSettingsReconnectRetry {
    fn install_if_needed(cx: &mut App) {
        if cx.has_global::<NotificationSettingsReconnectRetryObserver>() {
            return;
        }
        cx.set_global(NotificationSettingsReconnectRetryObserver);
        let (did_connect, mut on_did_connect) = mpsc::unbounded::<()>();
        RiseAppEvents::default_center()
            .observe(SAI_WS_DID_CONNECT, move |_| {
                let _ = did_connect.unbounded_send(());
            })
            .detach();
        cx.spawn(async move |cx: &mut AsyncApp| {
            while on_did_connect.next().await.is_some() {
                cx.update(|cx| {
                    Self::retry_if_failed(cx);
                    Self::resend_unsent_if_needed(cx);
                });
            }
        })
        .detach();
    }

    fn resend_unsent_if_needed(cx: &mut App) {
        let Some(user_id) = NotifyServicesStore::effective_user_id_for_wire(cx) else {
            tracing::warn!(target: "riseonly::X1405", "resend skipped: no user");
            return;
        };
        let unsent = RiseNotificationSettingsOutbox::load(&user_id);
        tracing::warn!(target: "riseonly::X1405", "resend check unsent={unsent:?}");
        if unsent.is_empty() {
            return;
        }
        let actions = NotifyActionsStore::shared(cx);
        let actions = actions.read(cx);
        if actions.get_notification_settings_sai.is_pending() || actions.update_notification_settings_sai.is_pending() {
            return;
        }
        NotifyActionsStore::get_notification_settings_action(cx);
    }

    fn retry_if_failed(cx: &mut App) {
        let rejected = matches!(
            NotifyActionsStore::shared(cx).read(cx).get_notification_settings_sai.status(),
            SaiWsSliceStatus::Rejected
        );
        if !rejected || NotifyServicesStore::shared(cx).read(cx).notification_settings_has_data(cx) {
            return;
        }
        NotifyActionsStore::get_notification_settings_action(cx);
    }
}
