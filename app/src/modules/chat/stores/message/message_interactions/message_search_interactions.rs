use gpui::App;
use rise_canon::RiseTimingCanon;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::modules::chat::engine::messages::rise_message_search_session::ChatSearchChatKindFilter;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

const SEARCH_MAX_CACHE_DATA: i64 = 8;

pub enum ChatMessageSearchDebounce {}

impl ChatMessageSearchDebounce {
    pub const GROUP_KEY: &str = "chat-message-search";
}

impl MessageInteractionsStore {
    pub fn search_chat_messages(&self, chat_id: &str, query: &str, cx: &mut App) {
        let chat = trimming_whitespaces_and_newlines(chat_id);
        let normalized = trimming_whitespaces_and_newlines(query);
        if chat.is_empty() {
            return;
        }
        if normalized.is_empty() {
            SaiDebouncer::cancel(chat, ChatMessageSearchDebounce::GROUP_KEY, cx);
            return;
        }
        let actions = Self::actions(cx);
        let (action_chat, action_query) = (chat.to_owned(), normalized.to_owned());
        let _ = SaiDebouncer::debounced_action_delay_milliseconds(
            chat,
            Box::new(move |cx: &mut App| {
                actions.update(cx, |actions, cx| actions.search_chat_messages_action(&action_chat, &action_query, cx));
            }),
            RiseTimingCanon::debounce("chat_message_search").client_debounce_ms,
            ChatMessageSearchDebounce::GROUP_KEY,
            SEARCH_MAX_CACHE_DATA,
            None,
            cx,
        );
    }

    pub fn load_more_chat_message_search(&self, chat_id: &str, query: &str, cx: &mut App) {
        let chat = trimming_whitespaces_and_newlines(chat_id);
        let normalized = trimming_whitespaces_and_newlines(query);
        if chat.is_empty() || normalized.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.load_more_chat_message_search_action(chat, normalized, cx));
    }

    pub fn search_chat_overlay_messages(&self, query: &str, chat_kind: ChatSearchChatKindFilter, cx: &mut App) {
        let normalized = trimming_whitespaces_and_newlines(query);
        let id = format!("overlay:{}", chat_kind.raw_value());
        if normalized.is_empty() {
            SaiDebouncer::cancel(&id, ChatMessageSearchDebounce::GROUP_KEY, cx);
            return;
        }
        let actions = Self::actions(cx);
        let action_query = normalized.to_owned();
        let _ = SaiDebouncer::debounced_action_delay_milliseconds(
            &id,
            Box::new(move |cx: &mut App| {
                actions.update(cx, |actions, cx| {
                    actions.search_chat_overlay_messages_action(&action_query, chat_kind, cx);
                });
            }),
            RiseTimingCanon::debounce("chat_list_message_search").client_debounce_ms,
            ChatMessageSearchDebounce::GROUP_KEY,
            SEARCH_MAX_CACHE_DATA,
            None,
            cx,
        );
    }

    pub fn load_more_chat_overlay_messages(&self, query: &str, chat_kind: ChatSearchChatKindFilter, cx: &mut App) {
        let normalized = trimming_whitespaces_and_newlines(query);
        if normalized.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.load_more_chat_overlay_messages_action(normalized, chat_kind, cx);
        });
    }

    pub fn load_newer_chat_overlay_messages(&self, query: &str, chat_kind: ChatSearchChatKindFilter, cx: &mut App) {
        let normalized = trimming_whitespaces_and_newlines(query);
        if normalized.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.load_newer_chat_overlay_messages_action(normalized, chat_kind, cx);
        });
    }

    pub fn end_chat_message_search(&self, chat_id: &str, cx: &mut App) {
        let chat = trimming_whitespaces_and_newlines(chat_id);
        if chat.is_empty() {
            return;
        }
        SaiDebouncer::cancel(chat, ChatMessageSearchDebounce::GROUP_KEY, cx);
        Self::actions(cx).update(cx, |actions, cx| actions.clear_chat_message_search_action(chat, cx));
    }
}
