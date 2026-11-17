use gpui::App;

use crate::modules::chat::stores::chat_stores_hooks::{ChatStoresShareHooks, set_chat_stores_share_hooks};
use crate::modules::share_extension::services::rise_share_app_bridge::RiseShareAppBridge;

pub fn install_chat_stores_share_hooks() {
    set_chat_stores_share_hooks(ChatStoresShareHooks {
        donate_incoming_interaction: |chat_id: &str, sender_id: &str, _cx: &mut App| {
            RiseShareAppBridge::donate_incoming_interaction(chat_id, sender_id)
        },
        donate_outgoing_interaction: RiseShareAppBridge::donate_outgoing_interaction,
    });
}
