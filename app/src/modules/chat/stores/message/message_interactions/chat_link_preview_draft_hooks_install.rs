use crate::modules::chat::components::chat::composer::chat_link_preview_draft_hooks::{
    ChatLinkPreviewDraftHooks, set_chat_link_preview_draft_hooks,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

pub(crate) fn install_chat_link_preview_draft_hooks() {
    set_chat_link_preview_draft_hooks(ChatLinkPreviewDraftHooks {
        load_link_preview: MessageInteractionsStore::load_link_preview,
    });
}
