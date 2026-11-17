use crate::app::aside::aside_navigator;
use crate::modules::chat::pages::chats::chats_aside_hooks::{ChatsAsideHooks, set_chats_aside_hooks};

pub fn install_chats_aside_hooks() {
    set_chats_aside_hooks(ChatsAsideHooks { follow_topic: aside_navigator::follow_topic });
}
