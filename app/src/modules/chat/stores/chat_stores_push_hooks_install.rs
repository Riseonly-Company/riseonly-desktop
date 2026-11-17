use crate::core::push_notifications::push_notification_manager::PushNotificationManager;
use crate::modules::chat::stores::chat_stores_hooks::{ChatStoresPushHooks, set_chat_stores_push_hooks};

pub fn install_chat_stores_push_hooks() {
    set_chat_stores_push_hooks(ChatStoresPushHooks {
        remove_delivered_notifications: PushNotificationManager::remove_delivered_notifications,
    });
}
