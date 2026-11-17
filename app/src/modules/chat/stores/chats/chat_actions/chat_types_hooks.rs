use crate::core::feature_hooks::FeatureHookSlot;

/// calls chat_types makes into stores of a later phase
/// until installed every call answers the safe default
pub(crate) struct ChatTypesHooks {
    pub(crate) is_joined_member_role: fn(Option<&str>) -> bool,
}

pub(crate) static CHAT_TYPES_HOOKS: FeatureHookSlot<ChatTypesHooks> = FeatureHookSlot::new("CHAT_TYPES_HOOKS");

pub(crate) fn set_chat_types_hooks(hooks: ChatTypesHooks) {
    let _ = CHAT_TYPES_HOOKS.set(hooks);
}

pub(crate) fn chat_types_hooks() -> Option<&'static ChatTypesHooks> {
    CHAT_TYPES_HOOKS.get()
}
