use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::engine::chat_engine_hooks::{ChatEngineComponentsHooks, set_chat_engine_components_hooks};

pub(crate) fn install_chat_engine_components_hooks() {
    set_chat_engine_components_hooks(ChatEngineComponentsHooks { is_own_bubble: MessageList::is_own_bubble });
}
