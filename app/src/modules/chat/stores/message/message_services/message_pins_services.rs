use gpui::{App, Entity};

use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::chat::stores::message::message_services::message_tabs_services::MessageTabPaginationState;

fn active_presentation(cx: &mut App) -> Option<Entity<RiseConversationPresentation>> {
    let presentation = RiseConversationPresentation::shared(cx);
    presentation.read(cx).is_active().then_some(presentation)
}

impl MessageServicesStore {
    pub fn has_pinned_messages_cache(&self, chat_id: &str, cx: &mut App) -> bool {
        active_presentation(cx)
            .is_some_and(|presentation| presentation.read(cx).pinned_messages_has_resolved_initial(chat_id))
    }

    pub fn pinned_messages(&self, chat_id: &str, cx: &mut App) -> Vec<GetMessageMessage> {
        if chat_id.is_empty() {
            return Vec::new();
        }
        active_presentation(cx)
            .map(|presentation| presentation.read(cx).pinned_messages(chat_id).to_vec())
            .unwrap_or_default()
    }

    pub fn pinned_messages_for_topic(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        cx: &mut App,
    ) -> Vec<GetMessageMessage> {
        let pins = self.pinned_messages(chat_id, cx);
        let Some(topic_id) = topic_id else {
            return pins;
        };
        pins.into_iter().filter(|pin| pin.topic_id.as_deref().unwrap_or_default() == topic_id).collect()
    }

    pub fn pinned_messages_pagination(&self, chat_id: &str, cx: &mut App) -> MessageTabPaginationState {
        let Some(presentation) = active_presentation(cx) else {
            return MessageTabPaginationState { relative_id: None, has_more: false };
        };
        let presentation = presentation.read(cx);
        MessageTabPaginationState {
            relative_id: presentation.pinned_messages_cursor(chat_id).map(str::to_owned),
            has_more: presentation.pinned_messages_has_more(chat_id),
        }
    }

    pub fn pinned_messages_presentation_revision(&self, chat_id: &str, cx: &mut App) -> u64 {
        active_presentation(cx).map_or(0, |presentation| presentation.read(cx).pinned_messages_revision(chat_id))
    }

    pub fn pinned_messages_error(&self, chat_id: &str, cx: &mut App) -> Option<String> {
        active_presentation(cx)
            .and_then(|presentation| presentation.read(cx).pinned_messages_error(chat_id).map(str::to_owned))
    }
}
