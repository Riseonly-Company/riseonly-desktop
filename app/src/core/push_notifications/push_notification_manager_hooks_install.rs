use gpui::App;

use crate::core::push_notifications::push_notification_manager::PushNotificationManager;
use crate::core::push_notifications::push_notification_manager_hooks::PushNotificationManagerHooks;

pub fn install_push_notification_manager_hooks() {
    PushNotificationManagerHooks::install(PushNotificationManagerHooks {
        device_token: Box::new(|cx: &App| {
            rise_ui::shared::try_read::<PushNotificationManager>(cx)
                .and_then(|manager| manager.device_token().map(str::to_owned))
        }),
        activate_for_authenticated_account: Box::new(|cx: &mut App| {
            PushNotificationManager::shared(cx)
                .update(cx, |manager, cx| manager.activate_for_authenticated_account(cx));
        }),
        prepare_for_account_switch: Box::new(|cx: &mut App| {
            PushNotificationManager::shared(cx)
                .update(cx, |manager, cx| manager.prepare_for_account_switch(cx));
        }),
        handle_account_cleared: Box::new(|cx: &mut App| {
            PushNotificationManager::shared(cx)
                .update(cx, |manager, cx| manager.handle_account_cleared(cx));
        }),
        request_permission_if_needed: Box::new(|cx: &mut App| {
            PushNotificationManager::request_permission_if_needed(cx);
        }),
        remove_delivered_notifications: Box::new(
            |chat_id: &str, topic_id: Option<&str>, cx: &mut App| {
                PushNotificationManager::remove_delivered_notifications(chat_id, topic_id, cx);
            },
        ),
        refresh_system_authorization: Box::new(|cx: &mut App| {
            PushNotificationManager::shared(cx)
                .update(cx, |manager, cx| manager.refresh_system_authorization(cx));
        }),
        system_authorization_denied: Box::new(|cx: &App| {
            rise_ui::shared::try_read::<PushNotificationManager>(cx)
                .is_some_and(PushNotificationManager::system_authorization_denied)
        }),
    });
}
