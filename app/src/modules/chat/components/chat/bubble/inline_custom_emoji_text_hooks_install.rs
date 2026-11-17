use gpui::IntoElement;

use crate::core::ui::animated_tabs_ui::inline_custom_emoji_text_hooks::{
    InlineCustomEmojiTextHooks, set_inline_custom_emoji_text_hooks,
};
use crate::modules::chat::components::chat::bubble::inline_custom_emoji_text::InlineCustomEmojiText;

pub fn install_inline_custom_emoji_text_hooks() {
    set_inline_custom_emoji_text_hooks(InlineCustomEmojiTextHooks {
        element: |id, text, style, entities, options, on_mention_tap, on_link_tap| {
            InlineCustomEmojiText::new(id, text, style, entities)
                .options(options)
                .on_mention_tap(on_mention_tap)
                .on_link_tap(on_link_tap)
                .into_any_element()
        },
    });
}
