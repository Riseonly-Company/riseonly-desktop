use gpui::App;

use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MessageUnreadMarksPresentation {
    pub mentions_count: i64,
    pub replies_count: i64,
    pub reactions_count: i64,
    pub is_loaded: bool,
}

impl MessageUnreadMarksPresentation {
    pub const EMPTY: Self = Self { mentions_count: 0, replies_count: 0, reactions_count: 0, is_loaded: false };
}

impl MessageServicesStore {
    pub fn unread_marks(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        cx: &mut App,
    ) -> MessageUnreadMarksPresentation {
        if chat_id.is_empty() {
            return MessageUnreadMarksPresentation::EMPTY;
        }
        let presentation = RiseConversationPresentation::shared(cx);
        let scope = RiseMessageScope::new(chat_id.to_owned(), topic_id.map(str::to_owned), None);
        let Some(snapshot) = presentation.read(cx).unread_marks(&scope) else {
            return MessageUnreadMarksPresentation::EMPTY;
        };
        MessageUnreadMarksPresentation {
            mentions_count: snapshot.mentions_count,
            replies_count: snapshot.replies_count,
            reactions_count: snapshot.reactions_count,
            is_loaded: snapshot.is_loaded,
        }
    }
}
