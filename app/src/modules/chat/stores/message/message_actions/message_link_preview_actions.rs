use gpui::{App, Task};

use super::message_actions::{MessageActionsStore, on_account_runtime};
use super::message_wire_types::MessageLinkMetadataWireDTO;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;

impl MessageActionsStore {
    pub fn load_link_preview_action(&self, url: &str, cx: &mut App) -> Task<anyhow::Result<MessageLinkMetadataWireDTO>> {
        let url = url.to_owned();
        cx.spawn(async move |cx| {
            let domain = RiseConversationActionExecutor::require_domain(cx).await?;
            on_account_runtime(async move { domain.auxiliary.link_preview(&url).await }).await?
        })
    }
}
