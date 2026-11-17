use gpui::IntoElement;

use crate::modules::chat::components::chat::input_picker::chat_input_emoji_tab::ChatInputEmojiTab;
use crate::modules::stickers::components::sticker_asset_composer_emoji_hooks::{
    StickerAssetComposerEmojiHooks, set_sticker_asset_composer_emoji_hooks,
};

pub fn install_sticker_asset_composer_emoji_hooks() {
    set_sticker_asset_composer_emoji_hooks(StickerAssetComposerEmojiHooks {
        system_emoji_tab: |id, on_select_emoji, on_select_custom_emoji, on_backspace, is_active| {
            ChatInputEmojiTab::new(id, on_select_emoji, on_select_custom_emoji, on_backspace)
                .safe_area_bottom(0.0)
                .is_active(is_active)
                .reserves_picker_bottom_bar(false)
                .uses_flat_chrome(true)
                .defers_loading_to_parent(true)
                .system_emoji_only(true)
                .shows_backspace(false)
                .into_any_element()
        },
    });
}
