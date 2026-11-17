use gpui::{App, Entity};
use rise_canon::RiseLimits;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::chat::engine::chat_engine_hooks::chat_engine_hooks;
use crate::modules::chat::engine::core::rise_chat_overlay_search_presentation::{
    RiseChatOverlaySearchPage, RiseChatOverlaySearchPresentationState, RiseChatOverlaySearchScope,
    RiseChatSearchRecentItem,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::messages::rise_message_search_session::{
    ChatSearchChatKindFilter, RiseMessageSearchPresentationState,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatSearchMessageKind {
    Media,
    Links,
    Files,
    Music,
    Voices,
}

impl ChatSearchMessageKind {
    pub const ALL_CASES: [Self; 5] = [Self::Media, Self::Links, Self::Files, Self::Music, Self::Voices];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::Links => "links",
            Self::Files => "files",
            Self::Music => "music",
            Self::Voices => "voices",
        }
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|kind| kind.raw_value() == raw_value)
    }

    pub fn wire_content_type(self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::Links => "link",
            Self::Files => "file",
            Self::Music => "music",
            Self::Voices => "voice",
        }
    }
}

pub struct ChatSearchOverlayStore;

impl ChatSearchOverlayStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_: &mut gpui::Context<Self>) -> Self {
        Self
    }

    pub fn users_limit() -> i64 {
        RiseLimits::client_value("chat_search_overlay_store_users_limit")
    }

    pub fn posts_limit() -> i64 {
        RiseLimits::client_value("chat_search_overlay_store_posts_limit")
    }

    pub fn chats_limit() -> i64 {
        RiseLimits::client_value("chat_search_overlay_store_chats_limit")
    }

    pub fn recent_items(&self, cx: &mut App) -> Vec<RiseChatSearchRecentItem> {
        let presentation = RiseConversationPresentation::shared(cx);
        presentation.read(cx).chat_search_recent_items()
    }

    pub fn entities_page(&self, query: &str, cx: &mut App) -> RiseChatOverlaySearchPage<SearchEntityRow> {
        self.state(query, cx).entities
    }

    pub fn chats_page(&self, query: &str, cx: &mut App) -> RiseChatOverlaySearchPage<ChatInfo> {
        self.state(query, cx).chats
    }

    pub fn posts_page(&self, query: &str, cx: &mut App) -> RiseChatOverlaySearchPage<GetPostFeedResponse> {
        self.state(query, cx).posts
    }

    pub fn messages_state(
        &self,
        query: &str,
        chat_kind: ChatSearchChatKindFilter,
        cx: &mut App,
    ) -> RiseMessageSearchPresentationState {
        match chat_engine_hooks() {
            Some(hooks) => (hooks.chat_overlay_message_search_state)(query, chat_kind, cx),
            None => RiseMessageSearchPresentationState::empty(),
        }
    }

    fn state(&self, query: &str, cx: &mut App) -> RiseChatOverlaySearchPresentationState {
        let Some(scope) = RiseChatOverlaySearchScope::new(query) else {
            return RiseChatOverlaySearchPresentationState::empty();
        };
        let presentation = RiseConversationPresentation::shared(cx);
        presentation.read(cx).chat_overlay_search_state(&scope)
    }
}
