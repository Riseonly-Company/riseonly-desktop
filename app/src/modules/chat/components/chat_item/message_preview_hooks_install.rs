use crate::modules::chat::components::chat_item::chat_list_preview_row::ChatListPreviewFactory;
use crate::modules::chat::shared::utils::message_preview_hooks::{MessagePreviewHooks, set_message_preview_hooks};

pub(crate) fn install_message_preview_hooks() {
    set_message_preview_hooks(MessagePreviewHooks {
        contact_name: ChatListPreviewFactory::contact_name,
        media_thumbnail_source: ChatListPreviewFactory::media_thumbnail_source,
    });
}
