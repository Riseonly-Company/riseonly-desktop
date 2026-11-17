use gpui::App;

use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatUnreadBadge, RiseChatUnreadBadgePresentation,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

impl ChatServicesStore {
    pub fn inbox_unread_badge(cx: &mut App) -> RiseChatUnreadBadge {
        RiseChatUnreadBadgePresentation::shared(cx).read(cx).badge()
    }

    pub fn inbox_unread_server_summary(cx: &mut App) -> Option<RiseChatUnreadBadge> {
        RiseChatUnreadBadgePresentation::shared(cx).read(cx).server_summary()
    }
}
