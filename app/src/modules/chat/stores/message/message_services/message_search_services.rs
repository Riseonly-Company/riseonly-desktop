use gpui::App;

use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::messages::rise_message_search_session::{
    ChatSearchChatKindFilter, RiseMessageSearchPresentationState, RiseMessageSearchScope,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

impl MessageServicesStore {
    pub fn chat_message_search_state(
        &self,
        chat_id: &str,
        query: &str,
        cx: &mut App,
    ) -> RiseMessageSearchPresentationState {
        let presentation = RiseConversationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        if !presentation.is_active() {
            return RiseMessageSearchPresentationState::empty();
        }
        let Some(scope) = RiseMessageSearchScope::new(chat_id, query) else {
            return RiseMessageSearchPresentationState::empty();
        };
        presentation.message_search_state(&scope)
    }

    pub fn chat_overlay_message_search_state(
        &self,
        query: &str,
        chat_kind: ChatSearchChatKindFilter,
        cx: &mut App,
    ) -> RiseMessageSearchPresentationState {
        let presentation = RiseConversationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        if !presentation.is_active() {
            return RiseMessageSearchPresentationState::empty();
        }
        let Some(scope) = RiseMessageSearchScope::global_query(query, chat_kind) else {
            return RiseMessageSearchPresentationState::empty();
        };
        presentation.message_search_state(&scope)
    }
}
