use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct PushNotificationManagerHooks {
    pub device_token: Box<dyn Fn(&App) -> Option<String> + Send + Sync>,
    pub activate_for_authenticated_account: Box<dyn Fn(&mut App) + Send + Sync>,
    pub prepare_for_account_switch: Box<dyn Fn(&mut App) + Send + Sync>,
    pub handle_account_cleared: Box<dyn Fn(&mut App) + Send + Sync>,
    pub request_permission_if_needed: Box<dyn Fn(&mut App) + Send + Sync>,
    pub remove_delivered_notifications: Box<dyn Fn(&str, Option<&str>, &mut App) + Send + Sync>,
    pub refresh_system_authorization: Box<dyn Fn(&mut App) + Send + Sync>,
    pub system_authorization_denied: Box<dyn Fn(&App) -> bool + Send + Sync>,
}

pub(crate) static PUSH_NOTIFICATION_MANAGER_HOOKS: FeatureHookSlot<PushNotificationManagerHooks> = FeatureHookSlot::new("PUSH_NOTIFICATION_MANAGER_HOOKS");

impl PushNotificationManagerHooks {
    pub fn install(hooks: Self) {
        let _ = PUSH_NOTIFICATION_MANAGER_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        PUSH_NOTIFICATION_MANAGER_HOOKS.get()
    }

    /// device token, none until the manager installs the slot
    pub fn device_token(cx: &App) -> Option<String> {
        Self::installed().and_then(|hooks| (hooks.device_token)(cx))
    }

    pub fn activate_for_authenticated_account(cx: &mut App) {
        if let Some(hooks) = Self::installed() {
            (hooks.activate_for_authenticated_account)(cx);
        }
    }

    pub fn prepare_for_account_switch(cx: &mut App) {
        if let Some(hooks) = Self::installed() {
            (hooks.prepare_for_account_switch)(cx);
        }
    }

    pub fn handle_account_cleared(cx: &mut App) {
        if let Some(hooks) = Self::installed() {
            (hooks.handle_account_cleared)(cx);
        }
    }

    pub fn request_permission_if_needed(cx: &mut App) {
        if let Some(hooks) = Self::installed() {
            (hooks.request_permission_if_needed)(cx);
        }
    }

    pub fn remove_delivered_notifications(chat_id: &str, topic_id: Option<&str>, cx: &mut App) {
        if let Some(hooks) = Self::installed() {
            (hooks.remove_delivered_notifications)(chat_id, topic_id, cx);
        }
    }

    pub fn refresh_system_authorization(cx: &mut App) {
        if let Some(hooks) = Self::installed() {
            (hooks.refresh_system_authorization)(cx);
        }
    }

    /// false until the manager installs the slot, no permission asked so none refused
    pub fn system_authorization_denied(cx: &App) -> bool {
        Self::installed().is_some_and(|hooks| (hooks.system_authorization_denied)(cx))
    }
}
