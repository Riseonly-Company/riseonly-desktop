use gpui::IntoElement;

use crate::modules::chat::components::chat::input_picker::chat_input_emoji_tab::ChatInputEmojiTab;
use crate::modules::post::components::repost::repost_opinion_editor_hooks::{
    RepostOpinionEditorHooks, set_repost_opinion_editor_hooks,
};

pub fn install_repost_opinion_editor_hooks() {
    set_repost_opinion_editor_hooks(RepostOpinionEditorHooks {
        emoji_tab: |id, on_select_emoji, on_select_custom_emoji, on_backspace, is_active, uses_flat_chrome| {
            ChatInputEmojiTab::new(id, on_select_emoji, on_select_custom_emoji, on_backspace)
                .safe_area_bottom(0.0)
                .is_active(is_active)
                .reserves_picker_bottom_bar(false)
                .uses_flat_chrome(uses_flat_chrome)
                .shows_backspace(false)
                .into_any_element()
        },
    });
}
