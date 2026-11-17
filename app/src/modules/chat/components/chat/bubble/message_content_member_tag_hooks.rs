use gpui::{AnyElement, SharedString};

use crate::core::feature_hooks::FeatureHookSlot;

/// slot for the bubble to draw the member tag chip of chat settings
/// without the installer the chip isnt drawn
pub(crate) struct MessageContentMemberTagHooks {
    pub(crate) member_tag_chip: fn(title: SharedString, color_hex: SharedString) -> AnyElement,
}

pub(crate) static MESSAGE_CONTENT_MEMBER_TAG_HOOKS: FeatureHookSlot<MessageContentMemberTagHooks> = FeatureHookSlot::new("MESSAGE_CONTENT_MEMBER_TAG_HOOKS");

pub(crate) fn set_message_content_member_tag_hooks(hooks: MessageContentMemberTagHooks) {
    let _ = MESSAGE_CONTENT_MEMBER_TAG_HOOKS.set(hooks);
}

pub(crate) fn message_content_member_tag_hooks() -> Option<&'static MessageContentMemberTagHooks> {
    MESSAGE_CONTENT_MEMBER_TAG_HOOKS.get()
}
