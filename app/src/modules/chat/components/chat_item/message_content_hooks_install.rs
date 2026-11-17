use crate::modules::chat::components::chat::bubble::message_content_hooks::{
    MessageContentHooks, set_message_content_hooks,
};
use crate::modules::chat::components::chat_item::chat_list_preview_row::ChatListPreviewFactory;

pub fn install_message_content_hooks() {
    set_message_content_hooks(MessageContentHooks { chat_title: ChatListPreviewFactory::chat_title });
}
