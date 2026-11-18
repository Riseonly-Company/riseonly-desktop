use std::rc::Rc;
use std::sync::Arc;

use gpui::App;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use super::message_actions::MessageActionsStore;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_unread_marks::{
    RiseUnreadMarkCandidate, RiseUnreadMarkKind, RiseUnreadMarkViewport,
};
use crate::modules::chat::engine::rise_conversation_domain::{RiseConversationDomain, RiseConversationDomainRegistry};

fn active_domain(cx: &mut App) -> Option<Arc<RiseConversationDomain>> {
    RiseConversationDomainRegistry::shared(cx).read(cx).active()
}

fn message_scope(chat_id: &str, topic_id: Option<&str>) -> RiseMessageScope {
    RiseMessageScope::new(chat_id.to_owned(), topic_id.map(str::to_owned), None)
}

impl MessageActionsStore {
    pub fn observe_unread_marks_action(&self, chat_id: &str, topic_id: Option<&str>, cx: &mut App) {
        let Some(domain) = active_domain(cx) else {
            return;
        };
        let scope = message_scope(chat_id, topic_id);
        drop(RiseAccountRuntime::current().spawn(async move {
            domain.observe_unread_marks(&scope).await;
        }));
    }

    pub fn release_unread_marks_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        seen_candidates: Vec<RiseUnreadMarkCandidate>,
        cx: &mut App,
    ) {
        let Some(domain) = active_domain(cx) else {
            return;
        };
        let scope = message_scope(chat_id, topic_id);
        drop(RiseAccountRuntime::current().spawn(async move {
            if !seen_candidates.is_empty() {
                domain.consume_unread_marks(&scope, seen_candidates).await;
            }
            domain.release_unread_marks(&scope).await;
        }));
    }

    pub fn update_unread_mark_viewport_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        viewport: RiseUnreadMarkViewport,
        cx: &mut App,
    ) {
        let Some(domain) = active_domain(cx) else {
            return;
        };
        let scope = message_scope(chat_id, topic_id);
        drop(RiseAccountRuntime::current().spawn(async move {
            domain.update_unread_mark_viewport(&scope, viewport).await;
        }));
    }

    pub fn consume_unread_marks_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        candidates: Vec<RiseUnreadMarkCandidate>,
        cx: &mut App,
    ) {
        let Some(domain) = active_domain(cx) else {
            return;
        };
        if candidates.is_empty() {
            return;
        }
        let scope = message_scope(chat_id, topic_id);
        drop(RiseAccountRuntime::current().spawn(async move {
            domain.consume_unread_marks(&scope, candidates).await;
        }));
    }

    pub fn next_unread_mark_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        kind: RiseUnreadMarkKind,
        seen_candidates: Vec<RiseUnreadMarkCandidate>,
        completion: Rc<dyn Fn(Option<String>, &mut App)>,
        cx: &mut App,
    ) {
        let Some(domain) = active_domain(cx) else {
            cx.defer(move |cx| completion(None, cx));
            return;
        };
        let scope = message_scope(chat_id, topic_id);
        cx.spawn(async move |cx| {
            let lookup = RiseAccountRuntime::current().spawn(async move {
                if !seen_candidates.is_empty() {
                    domain.consume_unread_marks(&scope, seen_candidates).await;
                }
                domain.next_unread_mark(&scope, kind).await
            });
            let target = lookup.await.ok().flatten();
            cx.update(|cx| completion(target, cx));
        })
        .detach();
    }

    pub fn clear_unread_marks_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        kind: RiseUnreadMarkKind,
        cx: &mut App,
    ) {
        let Some(domain) = active_domain(cx) else {
            return;
        };
        let scope = message_scope(chat_id, topic_id);
        drop(RiseAccountRuntime::current().spawn(async move {
            domain.clear_unread_marks(&scope, kind).await;
        }));
    }
}
