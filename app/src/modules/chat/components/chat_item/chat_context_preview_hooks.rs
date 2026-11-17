use gpui::{AnyElement, App, Entity, Window};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::components::chat_item::chat_context_preview::ChatContextPreviewSession;

pub struct ChatContextPreviewHooks {
    pub chat_screen: fn(Option<String>, bool, Option<Uuid>, &mut Window, &mut App) -> AnyElement,
    pub chat_screen_context_preview:
        fn(Option<String>, Option<String>, bool, Entity<ChatContextPreviewSession>, &mut Window, &mut App) -> AnyElement,
}

pub(crate) static CHAT_CONTEXT_PREVIEW_HOOKS: FeatureHookSlot<ChatContextPreviewHooks> = FeatureHookSlot::new("CHAT_CONTEXT_PREVIEW_HOOKS");

pub(crate) fn set_chat_context_preview_hooks(hooks: ChatContextPreviewHooks) {
    let _ = CHAT_CONTEXT_PREVIEW_HOOKS.set(hooks);
}

pub(crate) fn chat_context_preview_hooks() -> Option<&'static ChatContextPreviewHooks> {
    CHAT_CONTEXT_PREVIEW_HOOKS.get()
}
