use futures::future::LocalBoxFuture;
use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageLinkMetadataWireDTO;

pub(crate) struct ChatLinkPreviewDraftHooks {
    pub load_link_preview:
        fn(url: String, cx: &mut App) -> LocalBoxFuture<'static, anyhow::Result<MessageLinkMetadataWireDTO>>,
}

pub(crate) static CHAT_LINK_PREVIEW_DRAFT_HOOKS: FeatureHookSlot<ChatLinkPreviewDraftHooks> = FeatureHookSlot::new("CHAT_LINK_PREVIEW_DRAFT_HOOKS");

/// installs the hooks once, a second call keeps the first ones
pub(crate) fn set_chat_link_preview_draft_hooks(hooks: ChatLinkPreviewDraftHooks) {
    let _ = CHAT_LINK_PREVIEW_DRAFT_HOOKS.set(hooks);
}

pub(crate) fn chat_link_preview_draft_hooks() -> Option<&'static ChatLinkPreviewDraftHooks> {
    CHAT_LINK_PREVIEW_DRAFT_HOOKS.get()
}
