use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::modules::chat::engine::core::rise_conversation_resource_repository::{
    RiseConversationResourceKey, RiseConversationResourceKeyPersistence,
};

pub enum RiseConversationResourceKeys {}

impl RiseConversationResourceKeys {
    pub fn folders(account_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["folders".to_owned(), account_id.to_owned()])
    }

    pub fn folder_search(account_id: &str, query: &str) -> RiseConversationResourceKey {
        Self::persistent(vec![
            "folder-search".to_owned(),
            account_id.to_owned(),
            Self::normalized_query(query),
        ])
    }

    pub fn members(chat_id: &str, topic_id: Option<&str>) -> RiseConversationResourceKey {
        Self::persistent(vec![
            "members".to_owned(),
            chat_id.to_owned(),
            Self::optional_component(topic_id),
        ])
    }

    pub fn admins(chat_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["admins".to_owned(), chat_id.to_owned()])
    }

    pub fn blocked(chat_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["blocked".to_owned(), chat_id.to_owned()])
    }

    pub fn audit_history(chat_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["audit".to_owned(), chat_id.to_owned()])
    }

    pub fn topics(chat_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["topics".to_owned(), chat_id.to_owned()])
    }

    pub fn forum(chat_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["forum".to_owned(), chat_id.to_owned()])
    }

    pub fn effective_permissions(
        chat_id: &str,
        participant_id: Option<&str>,
        topic_id: Option<&str>,
    ) -> RiseConversationResourceKey {
        Self::persistent(vec![
            "effective-permissions".to_owned(),
            chat_id.to_owned(),
            Self::optional_component(participant_id),
            Self::optional_component(topic_id),
        ])
    }

    pub fn topic_overwrites(chat_id: &str, topic_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec![
            "topic-overwrites".to_owned(),
            chat_id.to_owned(),
            topic_id.to_owned(),
        ])
    }

    pub fn section_overwrites(chat_id: &str, section_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec![
            "section-overwrites".to_owned(),
            chat_id.to_owned(),
            section_id.to_owned(),
        ])
    }

    pub fn join_requests(chat_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["join-requests".to_owned(), chat_id.to_owned()])
    }

    pub fn invite_links(chat_id: &str) -> RiseConversationResourceKey {
        Self::memory_only(vec!["invite-links".to_owned(), chat_id.to_owned()])
    }

    pub fn settings(chat_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["settings".to_owned(), chat_id.to_owned()])
    }

    pub fn linked_chat(chat_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["linked-chat".to_owned(), chat_id.to_owned()])
    }

    pub fn analytics(chat_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["analytics".to_owned(), chat_id.to_owned()])
    }

    pub fn pinned_messages(chat_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["pinned-messages".to_owned(), chat_id.to_owned()])
    }

    pub fn message_read_receipts(chat_id: &str, message_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec![
            "message-read-receipts".to_owned(),
            chat_id.to_owned(),
            message_id.to_owned(),
        ])
    }

    pub fn message_tab(kind: &str, chat_id: &str, topic_id: Option<&str>) -> RiseConversationResourceKey {
        Self::persistent(vec![
            "message-tab".to_owned(),
            kind.to_owned(),
            chat_id.to_owned(),
            Self::optional_component(topic_id),
        ])
    }

    /// media index, identities plus how far the walk got
    /// not the media tab key since message projection invalidation would drop the total on every change
    pub fn media_gallery_index(chat_id: &str, topic_id: Option<&str>) -> RiseConversationResourceKey {
        Self::persistent(vec![
            "media-gallery-index".to_owned(),
            chat_id.to_owned(),
            Self::optional_component(topic_id),
        ])
    }

    /// rows shown last time so the next open draws before the network answers
    pub fn media_gallery_window(chat_id: &str, topic_id: Option<&str>) -> RiseConversationResourceKey {
        Self::persistent(vec![
            "media-gallery-window".to_owned(),
            chat_id.to_owned(),
            Self::optional_component(topic_id),
        ])
    }

    pub fn global_message_search(kind: &str, query: &str) -> RiseConversationResourceKey {
        Self::memory_only(vec![
            "global-message-search".to_owned(),
            kind.to_owned(),
            Self::normalized_query(query),
        ])
    }

    pub fn chat_overlay_entities(query: &str) -> RiseConversationResourceKey {
        Self::memory_only(vec!["chat-overlay-entities".to_owned(), Self::normalized_query(query)])
    }

    pub fn chat_overlay_chats(query: &str) -> RiseConversationResourceKey {
        Self::memory_only(vec!["chat-overlay-chats".to_owned(), Self::normalized_query(query)])
    }

    pub fn chat_overlay_posts(query: &str) -> RiseConversationResourceKey {
        Self::memory_only(vec!["chat-overlay-posts".to_owned(), Self::normalized_query(query)])
    }

    pub fn chat_search_recents(account_id: &str) -> RiseConversationResourceKey {
        Self::persistent(vec!["chat-search-recents".to_owned(), account_id.to_owned()])
    }

    /// one drawn month of the chat calendar for the open scope
    /// window is part of the key so two months or a topic view never share a cached month
    pub fn chat_calendar(
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        window_start_ms: i64,
        window_end_ms: i64,
    ) -> RiseConversationResourceKey {
        Self::persistent(vec![
            "chat-calendar".to_owned(),
            chat_id.to_owned(),
            Self::optional_component(topic_id),
            Self::optional_component(thread_root_id),
            window_start_ms.to_string(),
            window_end_ms.to_string(),
        ])
    }

    fn persistent(components: Vec<String>) -> RiseConversationResourceKey {
        RiseConversationResourceKey::new(components)
    }

    fn memory_only(components: Vec<String>) -> RiseConversationResourceKey {
        RiseConversationResourceKey::with_persistence(components, RiseConversationResourceKeyPersistence::MemoryOnly)
    }

    fn optional_component(value: Option<&str>) -> String {
        match value {
            Some(value) => format!("<some>{value}"),
            None => "<nil>".to_owned(),
        }
    }

    fn normalized_query(value: &str) -> String {
        trimming_whitespaces_and_newlines(value).to_lowercase()
    }
}
