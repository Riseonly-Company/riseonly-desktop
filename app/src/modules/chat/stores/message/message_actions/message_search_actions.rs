use gpui::App;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use super::message_actions::MessageActionsStore;
use super::message_tabs_actions::{effective_user_id, run_on_required_domain};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::messages::rise_message_search_session::{
    ChatSearchChatKindFilter, RiseMessageSearchScope,
};

impl MessageActionsStore {
    pub fn search_chat_messages_action(&self, chat_id: &str, query: &str, cx: &mut App) {
        let Some(scope) = RiseMessageSearchScope::new(chat_id, query) else {
            return;
        };
        self.search_messages_action(scope, cx);
    }

    pub fn load_more_chat_message_search_action(&self, chat_id: &str, query: &str, cx: &mut App) {
        let Some(scope) = RiseMessageSearchScope::new(chat_id, query) else {
            return;
        };
        self.page_message_search_action(scope, false, cx);
    }

    pub fn clear_chat_message_search_action(&self, chat_id: &str, cx: &mut App) {
        self.clear_message_search_action(trimming_whitespaces_and_newlines(chat_id), cx);
    }

    pub fn search_chat_overlay_messages_action(&self, query: &str, chat_kind: ChatSearchChatKindFilter, cx: &mut App) {
        let Some(scope) = RiseMessageSearchScope::global_query(query, chat_kind) else {
            return;
        };
        self.search_messages_action(scope, cx);
    }

    pub fn load_more_chat_overlay_messages_action(&self, query: &str, chat_kind: ChatSearchChatKindFilter, cx: &mut App) {
        let Some(scope) = RiseMessageSearchScope::global_query(query, chat_kind) else {
            return;
        };
        self.page_message_search_action(scope, false, cx);
    }

    pub fn load_newer_chat_overlay_messages_action(&self, query: &str, chat_kind: ChatSearchChatKindFilter, cx: &mut App) {
        let Some(scope) = RiseMessageSearchScope::global_query(query, chat_kind) else {
            return;
        };
        self.page_message_search_action(scope, true, cx);
    }

    fn search_messages_action(&self, scope: RiseMessageSearchScope, cx: &mut App) {
        let Some(user_id) = effective_user_id(cx) else {
            return;
        };
        run_on_required_domain(
            "Rise message search failed".to_owned(),
            SaiLogKind::Network,
            move |domain| async move {
                domain.search_messages(scope, &user_id).await;
            },
            cx,
        );
    }

    fn page_message_search_action(&self, scope: RiseMessageSearchScope, newer: bool, cx: &mut App) {
        let Some(user_id) = effective_user_id(cx) else {
            return;
        };
        run_on_required_domain(
            "Rise message search page failed".to_owned(),
            SaiLogKind::Network,
            move |domain| async move {
                if newer {
                    domain.load_newer_message_search(scope, &user_id).await;
                } else {
                    domain.load_more_message_search(scope, &user_id).await;
                }
            },
            cx,
        );
    }

    fn clear_message_search_action(&self, session_key: &str, cx: &mut App) {
        if session_key.is_empty() {
            return;
        }
        let session_key = session_key.to_owned();
        cx.spawn(async move |cx| {
            let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
                return;
            };
            drop(RiseAccountRuntime::current().spawn(async move {
                domain.clear_message_search(&session_key).await;
            }));
        })
        .detach();
    }
}
