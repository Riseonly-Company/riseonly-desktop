use crate::core::push_notifications::rise_app_icon_badge::RiseAppIconBadge;
use crate::modules::chat::engine::chat_engine_hooks::{
    ChatEngineBadgeHooks, set_chat_engine_badge_hooks,
};

pub fn install_chat_engine_badge_hooks() {
    set_chat_engine_badge_hooks(ChatEngineBadgeHooks { sync: RiseAppIconBadge::sync });
}
