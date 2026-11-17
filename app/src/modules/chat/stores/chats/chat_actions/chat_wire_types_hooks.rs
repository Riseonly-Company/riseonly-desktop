use rise_canon::rise_proto::chat::ProtoMessagePreview;
use rise_canon::rise_view::chat::ChatInfo as CanonChatInfo;

use super::chat_wire_types::{RiseStoredChat, RiseStoredChatPreview};

use crate::core::feature_hooks::FeatureHookSlot;

pub(crate) struct ChatWireTypesHooks {
    pub(crate) stored_chat_from_canon: fn(&CanonChatInfo) -> RiseStoredChat,
    pub(crate) stored_chat_preview_from_canon: fn(&ProtoMessagePreview, &str) -> RiseStoredChatPreview,
}

pub(crate) static CHAT_WIRE_TYPES_HOOKS: FeatureHookSlot<ChatWireTypesHooks> = FeatureHookSlot::new("CHAT_WIRE_TYPES_HOOKS");

/// second install is ignored, first owner keeps the slot
pub(crate) fn set_chat_wire_types_hooks(hooks: ChatWireTypesHooks) {
    let _ = CHAT_WIRE_TYPES_HOOKS.set(hooks);
}

pub(crate) fn chat_wire_types_hooks() -> Option<&'static ChatWireTypesHooks> {
    CHAT_WIRE_TYPES_HOOKS.get()
}
