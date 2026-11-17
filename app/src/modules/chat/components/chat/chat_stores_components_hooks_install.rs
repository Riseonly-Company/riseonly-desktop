use crate::modules::chat::components::chat::backdrop::chat_wallpaper_sheet::ChatWallpaperPermissions;
use crate::modules::chat::shared::chat_saved_message_presentation::ChatMessageReactionPolicy;
use crate::modules::chat::shared::utils::message_preview::MessagePreview;
use crate::modules::chat::stores::chat_stores_hooks::{ChatStoresComponentsHooks, set_chat_stores_components_hooks};

pub(crate) fn install_chat_stores_components_hooks() {
    set_chat_stores_components_hooks(ChatStoresComponentsHooks {
        can_set: ChatWallpaperPermissions::can_set,
        can_add: ChatMessageReactionPolicy::can_add,
        plain_custom_emoji: MessagePreview::plain_custom_emoji,
    });
}
