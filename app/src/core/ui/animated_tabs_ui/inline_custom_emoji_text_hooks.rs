use gpui::{AnyElement, ElementId, Empty, IntoElement};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::core::ui::formatted_text_ui::{FormattedTextOptions, FormattedTextStyle};
use crate::modules::chat::components::chat::bubble::chat_mention_tap_layer::ChatMentionTap;
use crate::modules::chat::stores::message::message_actions::message_types::MessageCustomEmojiEntity;

pub struct InlineCustomEmojiTextHooks {
    pub element: fn(
        ElementId,
        String,
        FormattedTextStyle,
        Vec<MessageCustomEmojiEntity>,
        FormattedTextOptions,
        Option<ChatMentionTap>,
        Option<ChatMentionTap>,
    ) -> AnyElement,
}

pub(crate) static INLINE_CUSTOM_EMOJI_TEXT_HOOKS: FeatureHookSlot<InlineCustomEmojiTextHooks> =
    FeatureHookSlot::new("INLINE_CUSTOM_EMOJI_TEXT_HOOKS");

pub fn set_inline_custom_emoji_text_hooks(hooks: InlineCustomEmojiTextHooks) {
    let _ = INLINE_CUSTOM_EMOJI_TEXT_HOOKS.set(hooks);
}

pub fn inline_custom_emoji_text_hooks() -> Option<&'static InlineCustomEmojiTextHooks> {
    INLINE_CUSTOM_EMOJI_TEXT_HOOKS.get()
}

/// inline emoji text as an element, empty until the hooks are installed
pub fn inline_custom_emoji_text_element(
    id: impl Into<ElementId>,
    text: impl Into<String>,
    style: FormattedTextStyle,
    entities: Vec<MessageCustomEmojiEntity>,
    options: FormattedTextOptions,
    on_mention_tap: Option<ChatMentionTap>,
    on_link_tap: Option<ChatMentionTap>,
) -> AnyElement {
    match inline_custom_emoji_text_hooks() {
        Some(hooks) => (hooks.element)(id.into(), text.into(), style, entities, options, on_mention_tap, on_link_tap),
        None => Empty.into_any_element(),
    }
}
