use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{App, Context};
use rise_canon::{RiseTimingCanon, RiseTimingDebounce};

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::{
    SaiDebouncedCallback, SaiDebouncer, SaiDebouncerOptions,
};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_unread_marks::{RiseUnreadMarkKind, RiseUnreadMarkViewport};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

static CONSUME_DEBOUNCE: LazyLock<RiseTimingDebounce> =
    LazyLock::new(|| RiseTimingCanon::debounce("unread_marks_consume"));

const UNREAD_MARKS_GROUP_KEY: &str = "unreadMarks";

impl MessageInteractionsStore {
    pub fn observe_unread_marks(&mut self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        if self.resolved_wire_chat_id(chat_id, cx).is_none() {
            return;
        }
        let key = Self::consume_key(chat_id, topic_id);
        let count = self.unread_mark_observation_counts.get(&key).copied().unwrap_or(0);
        self.unread_mark_observation_counts.insert(key, count + 1);
        if count != 0 {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.observe_unread_marks_action(chat_id, topic_id, cx));
    }

    pub fn release_unread_marks(&mut self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }
        let key = Self::consume_key(chat_id, topic_id);
        let Some(count) = self.unread_mark_observation_counts.get(&key).copied() else {
            return;
        };
        if count > 1 {
            self.unread_mark_observation_counts.insert(key, count - 1);
            return;
        }
        self.unread_mark_observation_counts.remove(&key);
        let seen = self.pending_unread_mark_candidates.remove(&key).unwrap_or_default();
        SaiDebouncer::cancel(&key, UNREAD_MARKS_GROUP_KEY, cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.release_unread_marks_action(chat_id, topic_id, seen.into_iter().collect(), cx);
        });
    }

    pub fn unread_mark_candidates_became_visible(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        viewport: RiseUnreadMarkViewport,
        cx: &mut Context<Self>,
    ) {
        let Some(wire_chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        let candidates = viewport.entered_candidates.clone();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_unread_mark_viewport_action(chat_id, topic_id, viewport, cx);
        });
        if candidates.is_empty() {
            return;
        }
        let services = ChatServicesStore::shared(cx);
        let is_channel = services
            .read(cx)
            .chat_info(&wire_chat_id, cx)
            .is_some_and(|chat| matches!(chat.r#type, ChatType::Channel));
        if is_channel {
            let message_ids = candidates.iter().map(|candidate| candidate.message_id.clone()).collect();
            Self::actions(cx).update(cx, |actions, cx| {
                actions.mark_channel_posts_viewed_action(&wire_chat_id, message_ids, cx);
            });
        }
        let key = Self::consume_key(chat_id, topic_id);
        self.pending_unread_mark_candidates.entry(key.clone()).or_default().extend(candidates);

        let this = cx.weak_entity();
        let batch_key = key.clone();
        let batch_chat_id = chat_id.to_owned();
        let batch_topic_id = topic_id.map(str::to_owned);
        let action: SaiDebouncedCallback = Box::new(move |cx: &mut App| {
            let Ok(Some(batch)) = this.update(cx, |store, _| store.pending_unread_mark_candidates.remove(&batch_key))
            else {
                return;
            };
            if batch.is_empty() {
                return;
            }
            Self::actions(cx).update(cx, |actions, cx| {
                actions.consume_unread_marks_action(
                    &batch_chat_id,
                    batch_topic_id.as_deref(),
                    batch.into_iter().collect(),
                    cx,
                );
            });
        });
        let _ = SaiDebouncer::debounced_action_id(
            &key,
            action,
            SaiDebouncerOptions {
                delay_milliseconds: CONSUME_DEBOUNCE.client_debounce_ms,
                group_key: UNREAD_MARKS_GROUP_KEY.to_owned(),
                ..SaiDebouncerOptions::default()
            },
            cx,
        );
    }

    pub fn next_unread_mark(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        kind: RiseUnreadMarkKind,
        completion: Rc<dyn Fn(Option<String>, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        if self.resolved_wire_chat_id(chat_id, cx).is_none() {
            cx.defer(move |cx| completion(None, cx));
            return;
        }
        let key = Self::consume_key(chat_id, topic_id);
        let seen = self.pending_unread_mark_candidates.remove(&key).unwrap_or_default();
        if !seen.is_empty() {
            SaiDebouncer::cancel(&key, UNREAD_MARKS_GROUP_KEY, cx);
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.next_unread_mark_action(chat_id, topic_id, kind, seen.into_iter().collect(), completion, cx);
        });
    }

    pub fn read_all_unread_marks(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        kind: RiseUnreadMarkKind,
        cx: &mut Context<Self>,
    ) {
        if self.resolved_wire_chat_id(chat_id, cx).is_none() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.clear_unread_marks_action(chat_id, topic_id, kind, cx));
    }

    fn consume_key(chat_id: &str, topic_id: Option<&str>) -> String {
        let scope = RiseMessageScope::new(chat_id.to_owned(), topic_id.map(str::to_owned), None);
        format!("unread-marks-{}", scope.storage_key())
    }
}
