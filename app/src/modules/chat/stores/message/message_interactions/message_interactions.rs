use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::Shared;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::foundation::foundation_date::TimeInterval;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseColdOpenTrace;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportDropWatch, RiseTransportStatus};
use crate::core::ui::clever_image_disk_cache::CleverImagePrefetchPolicy;
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftStore;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;
use crate::modules::chat::stores::message::message_interactions::message_reactions_interactions::MessageReactionSendLane;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::messages::rise_message_scope::{
    RiseMessageJumpOutcome, RiseMessagePageDirection, RiseMessageScope, RiseMessageWindowConfiguration,
};
use crate::modules::chat::engine::messages::rise_message_unread_marks::RiseUnreadMarkCandidate;
use crate::modules::chat::engine::rise_conversation_domain::{RiseConversationDomain, RiseConversationDomainRegistry};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_actions::{
    MessageActionFlagHandler, MessageActionsStore,
};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageDeliveryStatus, MessageHistoryDirection,
    MessageHistoryPaginationDispatch,
};
use crate::modules::chat::stores::message::message_services::message_pagination_trace::MessagePaginationTrace;
use crate::modules::chat::stores::message::message_services::message_presentation_window_projector::{
    MessagePresentationWindowProjector, MessagePresentationWindowShift,
};
use crate::modules::chat::stores::message::message_services::message_services::{
    MessageHistoryWarmCacheLimits, MessageHistoryWarmCachePolicy, MessageServicesStore,
};

const TEMPORARY_CHAT_ID_PREFIX: &str = "temp_";

pub type MessageDraftSendReplay = Rc<dyn Fn(String, &mut App)>;
pub type MessageJumpCompletion = Box<dyn FnOnce(RiseMessageJumpOutcome, &mut App)>;
pub type MessageDraftHistoryHandler = Box<dyn FnOnce(&mut App)>;

pub struct MessageInteractionsStoreParkedDraftSend {
    pub local_id: String,
    pub replay: MessageDraftSendReplay,
}

pub struct MessageInteractionsStore {
    pub last_typing_sent_by_chat: HashMap<String, TimeInterval>,
    pub observation_owner_generations: HashMap<String, u64>,
    pub pending_unread_mark_candidates: HashMap<String, HashSet<RiseUnreadMarkCandidate>>,
    pub unread_mark_observation_counts: HashMap<String, i64>,
    pub refused_draft_replay_chat_ids: HashSet<String>,
    pub parked_draft_sends: HashMap<String, Vec<MessageInteractionsStoreParkedDraftSend>>,
    pub draft_reconnect_watches: HashMap<String, RiseTransportDropWatch>,
    pub parked_reaction_sends: HashMap<String, RiseTransportDropWatch>,
    pub durable_reaction_keys: HashSet<String>,
    pub reaction_send_lanes: HashMap<String, MessageReactionSendLane>,
    draft_history_clear_tasks: HashMap<String, Shared<Task<()>>>,
}

fn active_domain(cx: &mut App) -> Option<Arc<RiseConversationDomain>> {
    RiseConversationDomainRegistry::shared(cx).read(cx).active()
}

fn message_scope(chat_id: &str, topic_id: Option<&str>, thread_root_id: Option<&str>) -> RiseMessageScope {
    RiseMessageScope::new(chat_id.to_owned(), topic_id.map(str::to_owned), thread_root_id.map(str::to_owned))
}

fn history_direction(direction: RiseMessagePageDirection) -> MessageHistoryDirection {
    match direction {
        RiseMessagePageDirection::Older => MessageHistoryDirection::Older,
        RiseMessagePageDirection::Newer => MessageHistoryDirection::Newer,
    }
}

fn window_shift(direction: RiseMessagePageDirection) -> MessagePresentationWindowShift {
    match direction {
        RiseMessagePageDirection::Older => MessagePresentationWindowShift::Older,
        RiseMessagePageDirection::Newer => MessagePresentationWindowShift::Newer,
    }
}

fn pagination_diagnostic(
    event: &str,
    scope: &str,
    direction: RiseMessagePageDirection,
    detail: &str,
    cx: &mut App,
) {
    MessagePaginationTrace::diagnostic(event, scope, Some(history_direction(direction)), None, detail, 0.0, cx);
}

impl MessageInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            last_typing_sent_by_chat: HashMap::new(),
            observation_owner_generations: HashMap::new(),
            pending_unread_mark_candidates: HashMap::new(),
            unread_mark_observation_counts: HashMap::new(),
            refused_draft_replay_chat_ids: HashSet::new(),
            parked_draft_sends: HashMap::new(),
            draft_reconnect_watches: HashMap::new(),
            parked_reaction_sends: HashMap::new(),
            durable_reaction_keys: HashSet::new(),
            reaction_send_lanes: HashMap::new(),
            draft_history_clear_tasks: HashMap::new(),
        }
    }

    pub fn actions(cx: &mut App) -> Entity<MessageActionsStore> {
        MessageActionsStore::shared(cx)
    }

    pub fn service(cx: &mut App) -> Entity<MessageServicesStore> {
        MessageServicesStore::shared(cx)
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.draft_history_clear_tasks.clear();

        self.last_typing_sent_by_chat.clear();
        self.observation_owner_generations.clear();
        self.pending_unread_mark_candidates.clear();
        self.unread_mark_observation_counts.clear();
        self.refused_draft_replay_chat_ids.clear();
        self.parked_draft_sends.clear();
        self.draft_reconnect_watches.values().for_each(RiseTransportDropWatch::stop);
        self.draft_reconnect_watches.clear();
        self.parked_reaction_sends.values().for_each(RiseTransportDropWatch::stop);
        self.parked_reaction_sends.clear();
        self.durable_reaction_keys.clear();
        self.reaction_send_lanes.clear();
        cx.notify();
    }

    pub fn discard_draft_chat_history(
        &mut self,
        chat_id: &str,
        on_error: MessageDraftHistoryHandler,
        completion: MessageDraftHistoryHandler,
        cx: &mut Context<Self>,
    ) {
        if self.draft_history_clear_tasks.contains_key(chat_id) {
            return;
        }
        if self.draft_history_clear_tasks.len() >= ChatServicesStore::DRAFT_DIRECT_CHAT_LIMIT {
            cx.defer(move |cx| on_error(cx));
            return;
        }

        let weak_store = cx.weak_entity();
        let cleared_chat_id = chat_id.to_owned();
        let handler: MessageActionFlagHandler = Box::new(move |cleared: bool, cx: &mut App| {
            let finished = weak_store.update(cx, |store, _| {
                store.draft_history_clear_tasks.remove(&cleared_chat_id);
                if cleared {
                    store.parked_draft_sends.remove(&cleared_chat_id);
                    store.refused_draft_replay_chat_ids.remove(&cleared_chat_id);
                }
            });
            if finished.is_err() {
                return;
            }
            if cleared {
                completion(cx);
            } else {
                on_error(cx);
            }
        });
        let task =
            Self::actions(cx).update(cx, |actions, cx| actions.clear_draft_chat_history_action(chat_id, handler, cx));
        if let Some(task) = task {
            self.draft_history_clear_tasks.insert(chat_id.to_owned(), task.shared());
        }
    }

    pub fn load_message_read_receipts(&mut self, chat_id: &str, message_id: &str, force: bool, cx: &mut Context<Self>) {
        let normalized_chat_id = trimming_whitespaces_and_newlines(chat_id);
        let normalized_message_id = trimming_whitespaces_and_newlines(message_id);
        if normalized_chat_id.is_empty() || normalized_message_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.load_message_read_receipts_action(normalized_chat_id, normalized_message_id, force, cx);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn load_initial_messages(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        scroll_position_key: Option<&str>,
        observation_owner_id: Option<Uuid>,
        jump_message_id: Option<&str>,
        jump_completion: Option<MessageJumpCompletion>,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() {
            if let Some(jump_completion) = jump_completion {
                cx.defer(move |cx| jump_completion(RiseMessageJumpOutcome::Failed, cx));
            }
            return;
        }
        let jump_anchor = jump_message_id.map(|id| trimming_whitespaces_and_newlines(id).to_owned());
        let scope = message_scope(chat_id, topic_id, thread_root_id);
        let anchor_storage_key = scroll_position_key
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| scope.storage_key());
        let saved_position = ChatScrollPositionStore::saved_position(&anchor_storage_key, cx);
        let saved_anchor = MessagePresentationWindowProjector::normalize_anchor_id(
            saved_position.as_ref().and_then(|position| position.anchor_id.as_deref()),
        )
        .filter(|anchor| !GetSaiServerId::is_temp_id(Some(anchor)));
        let owner_generation = observation_owner_id
            .as_ref()
            .map(|owner_id| self.next_observation_owner_generation(owner_id, &scope.storage_key(), cx));
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            RiseColdOpenTrace::mark(|| "messages intent".to_owned());
            let outcome = match RiseConversationActionExecutor::require_cached_domain(cx).await {
                Ok(domain) => {
                    RiseColdOpenTrace::mark(|| "messages domain resolved".to_owned());
                    let outcome = domain
                        .load_initial_messages(
                            &scope,
                            saved_anchor.as_deref(),
                            jump_anchor.as_deref(),
                            Some(&anchor_storage_key),
                            observation_owner_id,
                            owner_generation,
                        )
                        .await;
                    RiseColdOpenTrace::mark(|| format!("messages initial outcome={}", RiseColdOpenTrace::jump_outcome(&outcome)));
                    outcome
                }
                Err(error) => {
                    let text = format!("Rise initial messages({}) failed: {error}", scope.storage_key());
                    cx.update(|cx| Sai::log("Message", move || text, SaiLogKind::Network, cx));
                    RiseMessageJumpOutcome::Failed
                }
            };
            if let Some(jump_completion) = jump_completion {
                cx.update(|cx| jump_completion(outcome, cx));
            }
        })
        .detach();
    }

    pub fn load_messages_around(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        anchor_message_id: &str,
        completion: Option<MessageJumpCompletion>,
        cx: &mut Context<Self>,
    ) {
        let anchor = trimming_whitespaces_and_newlines(anchor_message_id).to_owned();
        if chat_id.is_empty() || anchor.is_empty() {
            if let Some(completion) = completion {
                cx.defer(move |cx| completion(RiseMessageJumpOutcome::Failed, cx));
            }
            return;
        }
        let scope = message_scope(chat_id, topic_id, thread_root_id);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let outcome = match RiseConversationActionExecutor::require_domain(cx).await {
                Ok(domain) => domain.jump_to_message(&scope, &anchor, RiseMessageWindowConfiguration::default()).await,
                Err(error) => {
                    let text = format!("Rise jump({}) failed: {error}", scope.storage_key());
                    cx.update(|cx| Sai::log("Message", move || text, SaiLogKind::Network, cx));
                    RiseMessageJumpOutcome::Failed
                }
            };
            if let Some(completion) = completion {
                cx.update(|cx| completion(outcome, cx));
            }
        })
        .detach();
    }

    pub fn cancel_pending_message_navigation_request(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() {
            return;
        }
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        Self::service(cx).update(cx, |service, cx| service.cancel_message_seed_request(&scope, cx));
        if let Some(domain) = active_domain(cx) {
            let rise_scope = message_scope(chat_id, topic_id, thread_root_id);
            cx.spawn(async move |_, _: &mut AsyncApp| domain.cancel_message_navigation(&rise_scope).await).detach();
        }
    }

    pub fn prepare_initial_messages_for_presentation(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        completion: MessageActionFlagHandler,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() {
            cx.defer(move |cx| completion(false, cx));
            return;
        }

        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        Self::service(cx).update(cx, |service, cx| service.preload_messages_cache_from_disk(&scope, completion, cx));
    }

    pub fn prioritize_message_history_warmup(
        &mut self,
        visible_chat_ids: &[String],
        recent_chat_ids: &[String],
        cx: &mut Context<Self>,
    ) {
        let chat_ids = MessageHistoryWarmCachePolicy::prioritized_scopes(
            visible_chat_ids,
            recent_chat_ids,
            MessageHistoryWarmCacheLimits::production().maximum_entry_count,
        );
        if let Some(domain) = active_domain(cx) {
            let scopes = if CleverImagePrefetchPolicy::allows_prefetch_now(cx) {
                chat_ids.iter().map(|chat_id| message_scope(chat_id, None, None)).collect()
            } else {
                Vec::new()
            };
            cx.spawn(async move |_, _: &mut AsyncApp| domain.replace_message_history_warmup(scopes).await).detach();
            return;
        }
        let ordered_scopes =
            chat_ids.iter().map(|chat_id| MessageServicesStore::message_scope(chat_id, None, None)).collect();
        Self::service(cx).update(cx, |service, cx| service.update_message_history_warm_candidates(ordered_scopes, cx));
    }

    pub fn cancel_message_history_warmup(&mut self, cx: &mut Context<Self>) {
        if let Some(domain) = active_domain(cx) {
            cx.spawn(async move |_, _: &mut AsyncApp| domain.replace_message_history_warmup(Vec::new()).await).detach();
        }
        Self::service(cx).update(cx, |service, cx| service.update_message_history_warm_candidates(Vec::new(), cx));
    }

    pub fn set_message_list_scrolling(&mut self, is_scrolling: bool, chat_id: &str, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.set_message_list_scrolling(is_scrolling, chat_id, cx));
    }

    pub fn flush_pending_messages_to_disk(&mut self, cx: &mut Context<Self>) {
        if RiseConversationPresentation::shared(cx).read(cx).is_active() {
            return;
        }
        Self::service(cx).update(cx, |service, cx| service.flush_pending_messages_to_disk(cx));
    }

    pub fn checkpoint_messages(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() || RiseConversationPresentation::shared(cx).read(cx).is_active() {
            return;
        }

        Self::service(cx).update(cx, |service, cx| service.flush_messages_disk_persistence(chat_id, cx));
    }

    pub fn checkpoint_pending_chat_caches(&mut self, cx: &mut Context<Self>) {
        if !RiseConversationPresentation::shared(cx).read(cx).is_active() {
            Self::service(cx).update(cx, |service, cx| service.flush_pending_messages_to_disk(cx));
            ChatServicesStore::shared(cx).update(cx, |services, cx| services.flush_chats_disk_persistence(cx));
        }
        ChatScrollPositionStore::flush_pending_to_disk(cx);
        ChatDraftStore::shared(cx).update(cx, |drafts, cx| drafts.flush_pending_to_disk(cx));
    }

    pub fn revalidate_forum_feed(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }
        let Some(domain) = active_domain(cx) else {
            return;
        };
        let chat_id = chat_id.to_owned();
        cx.spawn(async move |_, _: &mut AsyncApp| domain.revalidate_forum_feed(&chat_id).await).detach();
    }

    pub fn forget_lost_topics(&mut self, chat_id: &str, topic_ids: &[String], cx: &mut Context<Self>) {
        if chat_id.is_empty() || topic_ids.is_empty() {
            return;
        }
        let Some(domain) = active_domain(cx) else {
            return;
        };
        let chat_id = chat_id.to_owned();
        let topic_ids: HashSet<String> = topic_ids.iter().cloned().collect();
        cx.spawn(async move |_, _: &mut AsyncApp| domain.forget_lost_topic_rows(&chat_id, topic_ids).await).detach();
    }

    pub fn refresh_messages(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() {
            return;
        }
        if let Some(domain) = active_domain(cx) {
            let scope = message_scope(chat_id, topic_id, thread_root_id);
            cx.spawn(async move |_, _: &mut AsyncApp| domain.refresh_messages(&scope).await).detach();
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_messages_action(
                chat_id,
                None,
                topic_id,
                thread_root_id,
                None,
                MessageHistoryDirection::Older,
                None,
                true,
                Some(false),
                cx,
            );
        });
    }

    pub fn forget_reading_position(&mut self, scroll_position_key: &str, cx: &mut Context<Self>) {
        ChatScrollPositionStore::clear(scroll_position_key, cx);
    }

    pub fn jump_to_latest_messages(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        completion: Option<MessageActionFlagHandler>,
        cx: &mut Context<Self>,
    ) -> bool {
        if chat_id.is_empty() {
            return false;
        }

        if let Some(domain) = active_domain(cx) {
            let scope = message_scope(chat_id, topic_id, thread_root_id);
            let storage_key = scope.storage_key();
            let position_key = Self::service(cx).update(cx, |service, _| service.scroll_position_key(&storage_key));
            ChatScrollPositionStore::clear(&position_key, cx);
            cx.spawn(async move |_, cx: &mut AsyncApp| {
                let landed = domain.jump_to_latest_messages(&scope).await;
                if let Some(completion) = completion {
                    cx.update(|cx| completion(landed, cx));
                }
            })
            .detach();
            return false;
        }

        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let installed = Self::service(cx).update(cx, |service, cx| service.request_latest_messages_jump(&scope, cx));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_messages_action(
                chat_id,
                None,
                topic_id,
                thread_root_id,
                None,
                MessageHistoryDirection::Older,
                None,
                true,
                Some(false),
                cx,
            );
        });
        installed
    }

    pub fn observe_draft_chat_messages(
        &mut self,
        chat_id: &str,
        observation_owner_id: Option<Uuid>,
        cx: &mut Context<Self>,
    ) {
        if !ChatServicesStore::is_draft_direct_chat_id(chat_id) || self.resolved_wire_chat_id(chat_id, cx).is_some() {
            return;
        }
        let Some(domain) = active_domain(cx) else {
            return;
        };
        let scope = message_scope(chat_id, None, None);
        let owner_generation = observation_owner_id
            .as_ref()
            .map(|owner_id| self.next_observation_owner_generation(owner_id, &scope.storage_key(), cx));
        cx.spawn(async move |_, _: &mut AsyncApp| {
            domain.observe_local_messages(&scope, observation_owner_id, owner_generation).await;
        })
        .detach();
    }

    pub fn release_messages(&mut self, scope: &str, observation_owner_id: Option<Uuid>, cx: &mut Context<Self>) {
        if scope.is_empty() {
            return;
        }
        let Some(domain) = active_domain(cx) else {
            return;
        };
        let owner_generation = observation_owner_id
            .as_ref()
            .map(|owner_id| self.next_observation_owner_generation(owner_id, scope, cx));
        let storage_key = scope.to_owned();
        cx.spawn(async move |_, _: &mut AsyncApp| {
            domain.release_messages_storage_key(&storage_key, observation_owner_id, owner_generation).await;
        })
        .detach();
    }

    fn next_observation_owner_generation(&mut self, owner_id: &Uuid, scope: &str, cx: &mut Context<Self>) -> u64 {
        let generation = self.observation_owner_generations.entry(format!("{owner_id}|{scope}")).or_insert(0);
        *generation = generation.wrapping_add(1);
        let next = *generation;
        cx.notify();
        next
    }

    pub fn cancel_pending_latest_messages_jump(&mut self, scope: &str, cx: &mut Context<Self>) {
        if scope.is_empty() {
            return;
        }
        Self::service(cx).update(cx, |service, cx| service.cancel_pending_latest_messages_jump(scope, cx));
    }

    pub fn prefetch_message_page(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        direction: Option<RiseMessagePageDirection>,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() {
            return;
        }
        let Some(domain) = active_domain(cx) else {
            return;
        };
        let scope = message_scope(chat_id, topic_id, thread_root_id);
        let admitted_direction = if CleverImagePrefetchPolicy::allows_prefetch_now(cx) { direction } else { None };
        cx.spawn(async move |_, _: &mut AsyncApp| domain.prefetch_message_page(&scope, admitted_direction).await)
            .detach();
    }

    pub fn load_more_messages(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        shifts_cached_presentation_window: bool,
        visible_anchor_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.load_history_page(
            chat_id,
            topic_id,
            thread_root_id,
            shifts_cached_presentation_window,
            visible_anchor_id,
            RiseMessagePageDirection::Older,
            cx,
        );
    }

    pub fn load_newer_messages(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        shifts_cached_presentation_window: bool,
        visible_anchor_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.load_history_page(
            chat_id,
            topic_id,
            thread_root_id,
            shifts_cached_presentation_window,
            visible_anchor_id,
            RiseMessagePageDirection::Newer,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn load_history_page(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        shifts_cached_presentation_window: bool,
        visible_anchor_id: Option<&str>,
        direction: RiseMessagePageDirection,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() {
            return;
        }

        if let Some(domain) = active_domain(cx) {
            self.load_history_page_from_domain(
                domain,
                message_scope(chat_id, topic_id, thread_root_id),
                shifts_cached_presentation_window,
                visible_anchor_id,
                direction,
                cx,
            );
            return;
        }

        let history = history_direction(direction);
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let service = Self::service(cx);
        if shifts_cached_presentation_window
            && service.update(cx, |service, cx| {
                service.shift_message_presentation_window(&scope, window_shift(direction), visible_anchor_id, cx)
            })
        {
            return;
        }
        let page = service.update(cx, |service, cx| service.message_history_page_state(&scope, history, cx));
        if !page.has_more {
            let cached_count = service.update(cx, |service, cx| service.cached_message_count(&scope, cx));
            MessagePaginationTrace::record(
                "edge.skip",
                &scope,
                Some(history),
                None,
                page.relative_id.as_deref(),
                None,
                Some(cached_count),
                None,
                None,
                Some(false),
                Some("reason=end-reached"),
                cx,
            );
            return;
        }
        let Some(relative_id) = page.relative_id else {
            let cached_count = service.update(cx, |service, cx| service.cached_message_count(&scope, cx));
            MessagePaginationTrace::record(
                "edge.skip",
                &scope,
                Some(history),
                None,
                None,
                None,
                Some(cached_count),
                None,
                None,
                Some(true),
                Some("reason=missing-cursor"),
                cx,
            );
            return;
        };
        let Some(request_token) =
            service.update(cx, |service, cx| service.begin_message_history_request(&scope, history, cx))
        else {
            return;
        };

        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_messages_action(
                chat_id,
                None,
                topic_id,
                thread_root_id,
                Some(&relative_id),
                history,
                Some(request_token),
                false,
                None,
                cx,
            );
        });
    }

    fn load_history_page_from_domain(
        &mut self,
        domain: Arc<RiseConversationDomain>,
        scope: RiseMessageScope,
        shifts_cached_presentation_window: bool,
        visible_anchor_id: Option<&str>,
        direction: RiseMessagePageDirection,
        cx: &mut Context<Self>,
    ) {
        let storage_key = scope.storage_key();
        let anchor_label = visible_anchor_id.unwrap_or("none");
        pagination_diagnostic(
            "dispatch.begin",
            &storage_key,
            direction,
            &format!("anchor={anchor_label} shiftCached={shifts_cached_presentation_window}"),
            cx,
        );
        let shared_cx = RefCell::new(&mut *cx);
        let did_shift_cached_window = MessageHistoryPaginationDispatch::perform(
            || {
                let mut cx = shared_cx.borrow_mut();
                let cx: &mut Context<Self> = &mut cx;
                let did_shift = shifts_cached_presentation_window
                    && Self::service(cx).update(cx, |service, cx| {
                        service.shift_message_presentation_window(
                            &storage_key,
                            window_shift(direction),
                            visible_anchor_id,
                            cx,
                        )
                    });
                pagination_diagnostic("dispatch.cache", &storage_key, direction, &format!("shifted={did_shift}"), cx);
                did_shift
            },
            || {
                let mut cx = shared_cx.borrow_mut();
                let cx: &mut Context<Self> = &mut cx;
                pagination_diagnostic(
                    "dispatch.remote.begin",
                    &storage_key,
                    direction,
                    &format!("anchor={anchor_label} detached=true"),
                    cx,
                );
                let finished_key = storage_key.clone();
                cx.spawn(async move |_, cx: &mut AsyncApp| {
                    let outcome = domain.load_message_page(&scope, direction).await;
                    cx.update(|cx| {
                        pagination_diagnostic(
                            "dispatch.remote.finish",
                            &finished_key,
                            direction,
                            &format!("outcome={outcome:?}"),
                            cx,
                        );
                    });
                })
                .detach();
            },
        );
        let cx = shared_cx.into_inner();
        pagination_diagnostic(
            "dispatch.enqueued",
            &storage_key,
            direction,
            &format!("cachedShift={did_shift_cached_window}"),
            cx,
        );
    }

    pub fn resolved_wire_chat_id(&self, chat_id: &str, cx: &mut App) -> Option<String> {
        let resolved = ChatServicesStore::shared(cx)
            .update(cx, |services, cx| services.real_chat_id(chat_id, cx))
            .unwrap_or_else(|| chat_id.to_owned());
        if resolved.is_empty() || resolved.starts_with(TEMPORARY_CHAT_ID_PREFIX) {
            return None;
        }

        Some(resolved)
    }

    pub fn resolved_send_chat_id(&self, chat_id: &str, cx: &mut App) -> Option<String> {
        if ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.slow_mode_blocks_send(chat_id, cx)) {
            return None;
        }
        if let Some(resolved) = self.resolved_wire_chat_id(chat_id, cx) {
            return Some(resolved);
        }
        let services = ChatServicesStore::shared(cx);
        let services = services.read(cx);
        if !services.is_undeliverable_draft_direct_chat(chat_id)
            && !services.is_draft_direct_chat_awaiting_connection(chat_id)
        {
            return None;
        }

        Some(chat_id.to_owned())
    }

    pub fn defer_send_until_direct_chat_exists(
        &mut self,
        chat_id: &str,
        replay: MessageDraftSendReplay,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(clear_task) = self.draft_history_clear_tasks.get(chat_id).cloned() {
            let account_user_id = SaiAccountCacheScope::active_user_id();
            let account_epoch = SaiAccountCacheScope::active_epoch();
            let participant = ChatServicesStore::shared(cx)
                .read(cx)
                .draft_direct_chat(chat_id)
                .map(|draft| draft.participant.clone());
            let chat_id = chat_id.to_owned();
            cx.spawn(async move |_, cx: &mut AsyncApp| {
                clear_task.await;
                if account_user_id.is_none()
                    || account_epoch.is_none()
                    || !SaiAccountCacheScope::is_current_account_epoch(
                        account_user_id.as_deref(),
                        account_epoch.as_deref(),
                    )
                {
                    return;
                }
                cx.update(|cx| {
                    let services = ChatServicesStore::shared(cx);
                    if services.read(cx).draft_direct_chat(&chat_id).is_none()
                        && let Some(participant) = participant
                    {
                        services.update(cx, |services, cx| {
                            services.register_draft_direct_chat(&participant, cx);
                        });
                    }
                    replay(chat_id, cx);
                });
            })
            .detach();
            return true;
        }

        if !ChatServicesStore::is_draft_direct_chat_id(chat_id)
            || ChatServicesStore::shared(cx).update(cx, |services, cx| services.real_chat_id(chat_id, cx)).is_some()
        {
            return false;
        }

        if self.refused_draft_replay_chat_ids.remove(chat_id) {
            return false;
        }

        if !RiseTransportStatus::is_connected(cx) {
            self.park_draft_send(chat_id, true, replay, cx);
            return true;
        }

        let weak_store = cx.weak_entity();
        let draft_id = chat_id.to_owned();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.materialize_draft_direct_chat(
                chat_id,
                move |real_id: Option<String>, cx: &mut App| {
                    let Some(real_id) = real_id.filter(|real_id| !real_id.is_empty()) else {
                        let awaiting_connection = !RiseTransportStatus::is_connected(cx);
                        let _ = weak_store.update(cx, |store, cx| {
                            store.park_draft_send(&draft_id, awaiting_connection, replay, cx);
                        });
                        return;
                    };
                    let parked_replays = weak_store
                        .update(cx, |store, cx| store.take_parked_draft_sends(&draft_id, cx))
                        .unwrap_or_default();
                    for parked_replay in parked_replays {
                        parked_replay(real_id.clone(), cx);
                    }
                    replay(real_id, cx);
                },
                cx,
            );
        });
        true
    }

    fn park_draft_send(
        &mut self,
        chat_id: &str,
        awaiting_connection: bool,
        replay: MessageDraftSendReplay,
        cx: &mut Context<Self>,
    ) {
        let services = ChatServicesStore::shared(cx);
        if awaiting_connection {
            services.update(cx, |services, cx| {
                services.draft_direct_chat_ids_awaiting_connection.insert(chat_id.to_owned());
                cx.notify();
            });
            self.retry_parked_draft_sends_on_reconnect(chat_id, cx);
        } else {
            services.update(cx, |services, cx| services.mark_draft_direct_chat_undeliverable(chat_id, cx));
        }
        let weak_store = cx.weak_entity();
        let chat_id = chat_id.to_owned();
        cx.defer(move |cx| {
            let _ = weak_store.update(cx, |store, _| {
                store.refused_draft_replay_chat_ids.insert(chat_id.clone());
            });
            let actions = Self::actions(cx);
            actions.update(cx, |actions, cx| {
                actions.last_draft_send_local_id = None;
                cx.notify();
            });
            replay(chat_id.clone(), cx);
            let parked_local_id = actions.update(cx, |actions, cx| {
                let parked_local_id = actions.last_draft_send_local_id.take();
                cx.notify();
                parked_local_id
            });
            let _ = weak_store.update(cx, |store, _| {
                store.refused_draft_replay_chat_ids.remove(&chat_id);
                if let Some(local_id) = parked_local_id {
                    store
                        .parked_draft_sends
                        .entry(chat_id)
                        .or_default()
                        .push(MessageInteractionsStoreParkedDraftSend { local_id, replay });
                }
            });
        });
    }

    fn retry_parked_draft_sends_on_reconnect(&mut self, draft_id: &str, cx: &mut Context<Self>) {
        if self.draft_reconnect_watches.contains_key(draft_id) {
            return;
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let weak_store = cx.weak_entity();
        let watched_draft_id = draft_id.to_owned();
        let watch = RiseTransportStatus::on_next_connect(
            move |cx: &mut App| {
                let _ = weak_store.update(cx, |store, cx| {
                    if store.draft_reconnect_watches.remove(&watched_draft_id).is_none()
                        || !SaiAccountCacheScope::is_current_account_epoch(
                            account_user_id.as_deref(),
                            account_epoch.as_deref(),
                        )
                    {
                        return;
                    }
                    store.retry_parked_draft_sends(&watched_draft_id, cx);
                });
            },
            cx,
        );
        self.draft_reconnect_watches.insert(draft_id.to_owned(), watch);
    }

    fn fail_awaiting_draft_sends(&mut self, draft_id: &str, cx: &mut Context<Self>) {
        let was_awaiting = ChatServicesStore::shared(cx).update(cx, |services, cx| {
            let was_awaiting = services.draft_direct_chat_ids_awaiting_connection.remove(draft_id);
            cx.notify();
            was_awaiting
        });
        if !was_awaiting {
            return;
        }
        let Some(parked) = self.parked_draft_sends.remove(draft_id).filter(|parked| !parked.is_empty()) else {
            return;
        };
        Self::cancel_parked_rows(draft_id, parked.iter().map(|send| send.local_id.clone()).collect(), cx);
        for send in parked {
            self.park_draft_send(draft_id, false, send.replay, cx);
        }
    }

    fn cancel_parked_rows(draft_id: &str, local_ids: Vec<String>, cx: &mut Context<Self>) {
        let scope = message_scope(draft_id, None, None);
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
                return;
            };
            if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
                || !cx.update(|cx| domain.is_registry_active(cx))
            {
                return;
            }
            for local_id in local_ids {
                domain.cancel_outgoing_message(&local_id, &scope).await;
            }
        })
        .detach();
    }

    pub fn is_parked_draft_send(&self, local_id: &str, chat_id: &str) -> bool {
        self.parked_draft_sends.get(chat_id).is_some_and(|parked| parked.iter().any(|send| send.local_id == local_id))
    }

    pub fn drop_parked_draft_send(&mut self, local_id: &str, chat_id: &str) {
        let Some(parked) = self.parked_draft_sends.get_mut(chat_id) else {
            return;
        };
        parked.retain(|send| send.local_id != local_id);
        if parked.is_empty() {
            self.parked_draft_sends.remove(chat_id);
        }
    }

    pub fn can_repark_draft_send(&self, message: &GetMessageMessage, chat_id: &str) -> bool {
        ChatServicesStore::is_draft_direct_chat_id(chat_id)
            && message.delivery_status == MessageDeliveryStatus::Failed
            && message.content_type == MessageContentType::Text
            && !trimming_whitespaces_and_newlines(&message.text).is_empty()
            && !self.is_parked_draft_send(&message.id, chat_id)
    }

    pub fn repark_draft_send(&mut self, message: &GetMessageMessage, chat_id: &str, cx: &mut Context<Self>) {
        if !self.can_repark_draft_send(message, chat_id) {
            return;
        }
        let text = message.text.clone();
        let entities = message.entities.clone();
        let weak_store = cx.weak_entity();
        self.parked_draft_sends.entry(chat_id.to_owned()).or_default().push(MessageInteractionsStoreParkedDraftSend {
            local_id: message.id.clone(),
            replay: Rc::new(move |real_id: String, cx: &mut App| {
                let _ = weak_store.update(cx, |store, cx| {
                    store.send_text(
                        &real_id,
                        &text,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        entities.as_deref(),
                        None,
                        None,
                        None,
                        cx,
                    );
                });
            }),
        });
    }

    pub fn retry_parked_draft_sends(&mut self, draft_id: &str, cx: &mut Context<Self>) {
        if self.parked_draft_sends.get(draft_id).is_none_or(Vec::is_empty) {
            return;
        }
        let weak_store = cx.weak_entity();
        let parked_draft_id = draft_id.to_owned();
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.materialize_draft_direct_chat(
                draft_id,
                move |real_id: Option<String>, cx: &mut App| {
                    let Some(real_id) = real_id.filter(|real_id| !real_id.is_empty()) else {
                        let services = ChatServicesStore::shared(cx);
                        if !RiseTransportStatus::is_connected(cx)
                            && services.read(cx).is_draft_direct_chat_awaiting_connection(&parked_draft_id)
                        {
                            let _ = weak_store.update(cx, |store, cx| {
                                store.retry_parked_draft_sends_on_reconnect(&parked_draft_id, cx);
                            });
                            return;
                        }
                        services.update(cx, |services, cx| {
                            services.mark_draft_direct_chat_undeliverable(&parked_draft_id, cx);
                        });
                        let _ = weak_store.update(cx, |store, cx| store.fail_awaiting_draft_sends(&parked_draft_id, cx));
                        return;
                    };
                    let _ = weak_store.update(cx, |store, cx| {
                        store.flush_parked_draft_sends(&parked_draft_id, &real_id, cx);
                    });
                },
                cx,
            );
        });
    }

    pub fn flush_parked_draft_sends(&mut self, draft_id: &str, real_id: &str, cx: &mut Context<Self>) {
        let parked_replays = self.take_parked_draft_sends(draft_id, cx);
        if parked_replays.is_empty() {
            return;
        }
        let real_id = real_id.to_owned();
        cx.defer(move |cx| {
            for parked_replay in parked_replays {
                parked_replay(real_id.clone(), cx);
            }
        });
    }

    fn take_parked_draft_sends(&mut self, draft_id: &str, cx: &mut Context<Self>) -> Vec<MessageDraftSendReplay> {
        let Some(parked) = self.parked_draft_sends.remove(draft_id).filter(|parked| !parked.is_empty()) else {
            return Vec::new();
        };
        Self::cancel_parked_rows(draft_id, parked.iter().map(|send| send.local_id.clone()).collect(), cx);
        parked.into_iter().map(|send| send.replay).collect()
    }
}
