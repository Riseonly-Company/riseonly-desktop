use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use gpui::{App, Context, Entity, EventEmitter, Task};
use rise_canon::{rise_wire_methods, RiseLimits};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_isolated::RiseIsolated;
use rise_engine::rise_store::rise_store_entities::RiseEntityViewLoadState;
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileFavEntry, ChatProfileLinkEntry, ChatProfileMediaEntry, ChatProfileSection, ChatProfileSectioner,
};
use crate::modules::chat::engine::chat_engine_hooks::{
    chat_engine_badge_hooks, chat_engine_hooks, ChatEngineWarmCacheLimits,
};
use crate::modules::chat::engine::chats::rise_chat_repository_models::RiseChatListAggregates;
use crate::modules::chat::engine::core::rise_chat_overlay_search_presentation::{
    RiseChatOverlayChatSearchSnapshot, RiseChatOverlayEntitySearchSnapshot, RiseChatOverlayPostSearchSnapshot,
    RiseChatOverlaySearchPresentationBox, RiseChatOverlaySearchPresentationState, RiseChatOverlaySearchScope,
    RiseChatSearchRecentItem,
};
use crate::modules::chat::engine::core::rise_chat_presentation_preparer::{
    RiseChatListPresentationBox, RiseChatListPresentationSnapshot,
};
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationAuxiliaryLoadState, RiseConversationAuxiliaryPageSnapshot, RiseConversationMessageListTabKind,
};
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseChatEntityRecord, RiseMessageEntityRecord, RiseMessageLocalPhase,
};
use crate::modules::chat::engine::core::rise_conversation_store_layout::{
    RiseConversationStoreLayout, RiseConversationStoreLayoutChatView,
};
use crate::modules::chat::engine::core::rise_global_message_search_presentation::{
    RiseGlobalMessageSearchPresentationBox, RiseGlobalMessageSearchPresentationSnapshot,
    RiseGlobalMessageSearchPresentationState, RiseGlobalMessageSearchScope,
};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_search_session::{
    RiseMessageSearchPresentationBox, RiseMessageSearchPresentationSnapshot, RiseMessageSearchPresentationState,
    RiseMessageSearchScope,
};
use crate::modules::chat::engine::messages::rise_message_unread_marks::RiseUnreadMarksSnapshot;
use crate::modules::chat::engine::rise_conversation_domain::RiseMessagePresentationInputPolicy;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageDeliveryStatus, MessageLinkPreview, MessageLinkPreviewSize,
    MessageMediaKind, MessageThreadOrder,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    LinkMessageWireDTO, MediaItemWireDTO, MessageWireDTO,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiseChatListKind {
    Inbox,
    Archive,
    Requests,
}

impl RiseChatListKind {
    pub const ALL_CASES: [Self; 3] = [Self::Inbox, Self::Archive, Self::Requests];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Archive => "archive",
            Self::Requests => "requests",
        }
    }

    pub fn store_view(self) -> RiseConversationStoreLayoutChatView {
        match self {
            Self::Inbox => RiseConversationStoreLayoutChatView::Inbox,
            Self::Archive => RiseConversationStoreLayoutChatView::Archive,
            Self::Requests => RiseConversationStoreLayoutChatView::Requests,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RiseConversationEntityStamp {
    pub version: i64,
    pub updated_at_milliseconds: i64,
}

#[derive(Clone)]
pub struct RiseChatListWireSnapshot {
    pub account_id: String,
    pub kind: RiseChatListKind,
    pub revision: i64,
    pub rows: Vec<RiseChatEntityRecord>,
    pub local_ids: Vec<String>,
    pub entity_stamps: Vec<RiseConversationEntityStamp>,
    pub loaded_row_count: i64,
    pub server_total_count: Option<i64>,
    pub server_total_unread_count: Option<i64>,
    pub server_unread_chats_count: Option<i64>,
    pub server_total_unread_media_count: Option<i64>,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub load_state: RiseEntityViewLoadState,
    pub error_code: Option<String>,
}

/// chats tab badge as one small value so the tab bar watches three numbers and not the whole list
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseChatUnreadBadge {
    pub unread_count: i64,
    pub unread_chats_count: i64,
    pub unread_media_count: i64,
    pub unread_replies_count: i64,
}

impl RiseChatUnreadBadge {
    pub const EMPTY: Self =
        Self { unread_count: 0, unread_chats_count: 0, unread_media_count: 0, unread_replies_count: 0 };

    pub fn new(unread_count: i64, unread_chats_count: i64, unread_media_count: i64) -> Self {
        Self::with_replies(unread_count, unread_chats_count, unread_media_count, 0)
    }

    pub fn with_replies(
        unread_count: i64,
        unread_chats_count: i64,
        unread_media_count: i64,
        unread_replies_count: i64,
    ) -> Self {
        let unread_count = unread_count.max(0);
        Self {
            unread_count,
            unread_chats_count: unread_chats_count.max(0),
            unread_media_count: unread_media_count.max(0).min(unread_count),
            unread_replies_count: unread_replies_count.max(0),
        }
    }

    pub fn from_snapshot(snapshot: &RiseChatListWireSnapshot) -> Self {
        Self::new(
            snapshot.server_total_unread_count.unwrap_or(0),
            snapshot.server_unread_chats_count.unwrap_or(0),
            snapshot.server_total_unread_media_count.unwrap_or(0),
        )
    }

    pub fn from_aggregates(aggregates: &RiseChatListAggregates) -> Self {
        Self::with_replies(
            aggregates.total_unread_count.unwrap_or(0),
            aggregates.unread_chats_count.unwrap_or(0),
            aggregates.total_unread_media_count.unwrap_or(0),
            aggregates.total_unread_replies_count.unwrap_or(0),
        )
    }
}

pub struct RiseChatUnreadBadgePresentation {
    badge: RiseChatUnreadBadge,
    server_summary: Option<RiseChatUnreadBadge>,
}

impl RiseChatUnreadBadgePresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { badge: RiseChatUnreadBadge::EMPTY, server_summary: None }
    }

    pub fn badge(&self) -> RiseChatUnreadBadge {
        self.badge
    }

    pub fn server_summary(&self) -> Option<RiseChatUnreadBadge> {
        self.server_summary
    }

    pub fn publish(&mut self, next: RiseChatUnreadBadge, cx: &mut Context<Self>) {
        if next == self.badge {
            return;
        }
        self.badge = next;
        cx.notify();
        Self::sync_app_icon_badge(cx);
    }

    pub fn confirm_server_summary(&mut self, summary: RiseChatUnreadBadge, cx: &mut Context<Self>) {
        if Some(summary) == self.server_summary {
            return;
        }
        self.server_summary = Some(summary);
        cx.notify();
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.badge = RiseChatUnreadBadge::EMPTY;
        self.server_summary = None;
        cx.notify();
        Self::sync_app_icon_badge(cx);
    }

    fn sync_app_icon_badge(cx: &mut Context<Self>) {
        cx.defer(|cx| {
            if let Some(hooks) = chat_engine_badge_hooks() {
                (hooks.sync)(false, cx);
            }
        });
    }
}

#[derive(Clone)]
pub struct RiseMessageWireSnapshot {
    pub account_id: String,
    pub scope: RiseMessageScope,
    pub revision: i64,
    pub rows: Vec<RiseMessageEntityRecord>,
    pub local_ids: Vec<String>,
    pub entity_stamps: Vec<RiseConversationEntityStamp>,
    pub anchor_local_id: Option<String>,
    pub has_cached_before: bool,
    pub has_cached_after: bool,
    pub before_cursor: Option<String>,
    pub after_cursor: Option<String>,
    pub has_more_before: bool,
    pub has_more_after: bool,
    pub load_state: RiseEntityViewLoadState,
    pub error_code: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RiseMessagePresentationContentStamp {
    pub entity: RiseConversationEntityStamp,
    pub local_revision: u64,
    pub local_phase: RiseMessageLocalPhase,
}

#[derive(Clone)]
pub struct RiseMessageScopePresentationSnapshot {
    pub revision: u64,
    pub source: RiseMessageWireSnapshot,
    pub messages: Vec<GetMessageMessage>,
    pub wires: Vec<MessageWireDTO>,
    pub visible_local_ids: Vec<String>,
    pub content_stamps_by_local_id: HashMap<String, RiseMessagePresentationContentStamp>,
    pub anchor_offset: i64,
    pub pinned_messages: Vec<GetMessageMessage>,
    pub latest_reply_keyboard_message: Option<GetMessageMessage>,
}

#[derive(Clone, PartialEq)]
pub struct RiseMessagePresentationPublishReceipt {
    pub account_id: String,
    pub scope: RiseMessageScope,
    pub source_revision: i64,
    pub presentation_revision: u64,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct RiseMessagePresentationPreparerCacheKey {
    scope: RiseMessageScope,
    local_id: String,
}

struct RiseMessagePresentationPreparerCacheEntry {
    stamp: RiseConversationEntityStamp,
    local_phase: RiseMessageLocalPhase,
    model: GetMessageMessage,
}

struct RiseMessagePresentationPreparerReusableEntry {
    snapshot: Arc<RiseMessageScopePresentationSnapshot>,
    cost: i64,
    expires_at: TimeInterval,
    access: TimeInterval,
}

struct RiseMessagePresentationPreparerState {
    prepared_snapshots_by_scope: HashMap<RiseMessageScope, Arc<RiseMessageScopePresentationSnapshot>>,
    reusable_snapshots: HashMap<RiseMessageScope, RiseMessagePresentationPreparerReusableEntry>,
    presentation_revision: u64,
    reuse_limits: ChatEngineWarmCacheLimits,
    model_cache: HashMap<RiseMessagePresentationPreparerCacheKey, RiseMessagePresentationPreparerCacheEntry>,
    visible_keys_by_scope: HashMap<RiseMessageScope, HashSet<RiseMessagePresentationPreparerCacheKey>>,
    local_ids_by_scope: HashMap<RiseMessageScope, HashSet<String>>,
    scopes_by_local_id: HashMap<String, HashSet<RiseMessageScope>>,
    revisions_by_scope: HashMap<RiseMessageScope, u64>,
    hidden_message_local_ids: HashSet<String>,
    hidden_message_order: Vec<String>,
    hidden_media_slot_indices_by_local_id: HashMap<String, HashSet<i64>>,
    hidden_media_owner_order: Vec<String>,
    local_content_revisions_by_local_id: HashMap<String, u64>,
    model_cache_limit: usize,
    local_override_limit: usize,
}

impl RiseMessagePresentationPreparerState {
    fn new() -> Self {
        Self {
            prepared_snapshots_by_scope: HashMap::new(),
            reusable_snapshots: HashMap::new(),
            presentation_revision: 0,
            reuse_limits: chat_engine_hooks().map_or(
                ChatEngineWarmCacheLimits { maximum_entry_count: 0, maximum_estimated_cost: 0, time_to_live: 0.0 },
                |hooks| (hooks.warm_cache_limits_production)(),
            ),
            model_cache: HashMap::new(),
            visible_keys_by_scope: HashMap::new(),
            local_ids_by_scope: HashMap::new(),
            scopes_by_local_id: HashMap::new(),
            revisions_by_scope: HashMap::new(),
            hidden_message_local_ids: HashSet::new(),
            hidden_message_order: Vec::new(),
            hidden_media_slot_indices_by_local_id: HashMap::new(),
            hidden_media_owner_order: Vec::new(),
            local_content_revisions_by_local_id: HashMap::new(),
            model_cache_limit: usize::try_from(RiseLimits::client_value("message_presentation_model_cache_entries"))
                .unwrap_or(0),
            local_override_limit: usize::try_from(RiseLimits::client_value(
                "message_presentation_local_override_entries",
            ))
            .unwrap_or(0),
        }
    }

    fn prepare(
        &mut self,
        snapshot: RiseMessageWireSnapshot,
        reusing_prepared: bool,
    ) -> Option<Arc<RiseMessageScopePresentationSnapshot>> {
        if snapshot.rows.len() != snapshot.local_ids.len() || snapshot.rows.len() != snapshot.entity_stamps.len() {
            return None;
        }

        self.evict_reusable_snapshots();
        let reusable = self.reusable_snapshots.remove(&snapshot.scope).map(|entry| entry.snapshot);
        self.update_scope_membership(&snapshot);
        if reusing_prepared {
            let cached = self.prepared_snapshots_by_scope.get(&snapshot.scope).cloned().or(reusable);
            if let Some(cached) =
                cached.filter(|cached| RiseMessagePresentationInputPolicy::has_same_content(&cached.source, &snapshot))
            {
                self.presentation_revision = self.presentation_revision.wrapping_add(1);
                self.revisions_by_scope.insert(snapshot.scope.clone(), self.presentation_revision);
                let scope = snapshot.scope.clone();
                let prepared = Arc::new(RiseMessageScopePresentationSnapshot {
                    revision: self.presentation_revision,
                    source: snapshot,
                    messages: cached.messages.clone(),
                    wires: cached.wires.clone(),
                    visible_local_ids: cached.visible_local_ids.clone(),
                    content_stamps_by_local_id: cached.content_stamps_by_local_id.clone(),
                    anchor_offset: cached.anchor_offset,
                    pinned_messages: cached.pinned_messages.clone(),
                    latest_reply_keyboard_message: cached.latest_reply_keyboard_message.clone(),
                });
                self.prepared_snapshots_by_scope.insert(scope, Arc::clone(&prepared));
                return Some(prepared);
            }
        }

        let row_count = snapshot.rows.len();
        let mut messages: Vec<GetMessageMessage> = Vec::with_capacity(row_count);
        let mut wires: Vec<MessageWireDTO> = Vec::with_capacity(row_count);
        let mut visible_local_ids: Vec<String> = Vec::with_capacity(row_count);
        let mut content_stamps_by_local_id: HashMap<String, RiseMessagePresentationContentStamp> =
            HashMap::with_capacity(row_count);
        let mut pinned_messages: Vec<GetMessageMessage> = Vec::with_capacity(row_count.min(16));
        let mut latest_reply_keyboard_message: Option<GetMessageMessage> = None;
        let mut anchor_offset: i64 = 0;
        let mut visible_keys: HashSet<RiseMessagePresentationPreparerCacheKey> = HashSet::with_capacity(row_count);

        for index in 0..row_count {
            let record = &snapshot.rows[index];
            let wire = &record.wire;
            let local_id = &snapshot.local_ids[index];
            let stamp = snapshot.entity_stamps[index];
            let cache_key =
                RiseMessagePresentationPreparerCacheKey { scope: snapshot.scope.clone(), local_id: local_id.clone() };

            if record.local_phase == RiseMessageLocalPhase::Remote {
                self.hidden_media_slot_indices_by_local_id.remove(local_id);
                self.hidden_media_owner_order.retain(|owner| owner != local_id);
                self.local_content_revisions_by_local_id.remove(local_id);
            }
            let is_cached = self
                .model_cache
                .get(&cache_key)
                .is_some_and(|cached| cached.stamp == stamp && cached.local_phase == record.local_phase);
            if !is_cached {
                let mapped = Self::mapped_message(record, wire, local_id, &snapshot.scope);
                self.model_cache.insert(
                    cache_key.clone(),
                    RiseMessagePresentationPreparerCacheEntry { stamp, local_phase: record.local_phase, model: mapped },
                );
            }

            let resolved = if self.hidden_message_local_ids.contains(local_id) {
                None
            } else {
                self.model_cache.get(&cache_key).map(|cached| self.apply_hidden_media(&cached.model, local_id))
            };
            visible_keys.insert(cache_key);
            let Some(resolved) = resolved else { continue };
            if resolved.is_pinned {
                pinned_messages.push(resolved.clone());
            }
            if resolved.reply_markup.is_some() {
                latest_reply_keyboard_message = Some(resolved.clone());
            }
            messages.push(resolved);
            wires.push(wire.clone());
            visible_local_ids.push(local_id.clone());
            content_stamps_by_local_id.insert(
                local_id.clone(),
                RiseMessagePresentationContentStamp {
                    entity: stamp,
                    local_revision: self.local_content_revisions_by_local_id.get(local_id).copied().unwrap_or(0),
                    local_phase: record.local_phase,
                },
            );
            if Some(local_id) == snapshot.anchor_local_id.as_ref() {
                anchor_offset = i64::try_from(messages.len()).unwrap_or(i64::MAX) - 1;
            }
        }

        let scope = snapshot.scope.clone();
        self.visible_keys_by_scope.insert(scope.clone(), visible_keys);
        self.evict_if_needed();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        self.revisions_by_scope.insert(scope.clone(), self.presentation_revision);
        let prepared = Arc::new(RiseMessageScopePresentationSnapshot {
            revision: self.presentation_revision,
            source: snapshot,
            messages,
            wires,
            visible_local_ids,
            content_stamps_by_local_id,
            anchor_offset,
            pinned_messages,
            latest_reply_keyboard_message,
        });
        self.prepared_snapshots_by_scope.insert(scope, Arc::clone(&prepared));
        Some(prepared)
    }

    fn mapped_message(
        record: &RiseMessageEntityRecord,
        wire: &MessageWireDTO,
        local_id: &str,
        scope: &RiseMessageScope,
    ) -> GetMessageMessage {
        let mut mapped = wire.to_message();
        mapped.id = local_id.to_owned();
        if mapped.server_id.is_none() {
            mapped.server_id = RiseConversationStoreLayout::canonical_message_server_id(wire);
        }
        if mapped.chat_id.is_empty() {
            mapped.chat_id = scope.chat_id.clone();
        }
        match record.local_phase {
            RiseMessageLocalPhase::Remote => {}
            RiseMessageLocalPhase::Queued | RiseMessageLocalPhase::Uploading | RiseMessageLocalPhase::Sending => {
                mapped.is_temp = true;
                mapped.delivery_status = MessageDeliveryStatus::Sending;
            }
            RiseMessageLocalPhase::Failed => {
                mapped.is_temp = true;
                mapped.delivery_status = MessageDeliveryStatus::Failed;
                mapped.can_retry_send = record.is_replayable == Some(true)
                    || record.upload_retry.as_ref().is_some_and(|retry| retry.is_resendable())
                    || record.forward_retry.is_some();
                mapped.send_failure_code = record.last_error_code.clone();
            }
        }
        mapped
    }

    fn has_warm_snapshot(&mut self, scope: &RiseMessageScope) -> bool {
        self.evict_reusable_snapshots();
        self.prepared_snapshots_by_scope.contains_key(scope) || self.reusable_snapshots.contains_key(scope)
    }

    fn prepare_warm_snapshot(&mut self, snapshot: RiseMessageWireSnapshot) {
        if self.prepared_snapshots_by_scope.contains_key(&snapshot.scope) {
            return;
        }
        self.evict_reusable_snapshots();
        if let Some(cached) = self.reusable_snapshots.get(&snapshot.scope).map(|entry| &entry.snapshot) {
            if cached.source.revision > snapshot.revision {
                return;
            }
            if RiseMessagePresentationInputPolicy::has_same_content(&cached.source, &snapshot) {
                return;
            }
        }
        if snapshot.rows.is_empty() {
            self.reusable_snapshots.remove(&snapshot.scope);
            return;
        }
        let scope = snapshot.scope.clone();
        let Some(prepared) = self.prepare(snapshot, true) else { return };
        self.retire(&scope, Some(prepared.revision));
    }

    fn hide_outgoing_message(&mut self, local_id: &str) -> Vec<Arc<RiseMessageScopePresentationSnapshot>> {
        if local_id.is_empty() {
            return Vec::new();
        }
        self.hidden_message_local_ids.insert(local_id.to_owned());
        self.hidden_message_order.retain(|hidden| hidden != local_id);
        self.hidden_message_order.push(local_id.to_owned());
        while self.hidden_message_order.len() > self.local_override_limit {
            let evicted = self.hidden_message_order.remove(0);
            self.hidden_message_local_ids.remove(&evicted);
        }
        self.reprepare_scopes(local_id)
    }

    fn hide_outgoing_media_slot(
        &mut self,
        local_id: &str,
        local_slot_index: i64,
    ) -> Vec<Arc<RiseMessageScopePresentationSnapshot>> {
        if local_id.is_empty() || local_slot_index < 0 {
            return Vec::new();
        }
        self.hidden_media_slot_indices_by_local_id.entry(local_id.to_owned()).or_default().insert(local_slot_index);
        self.hidden_media_owner_order.retain(|owner| owner != local_id);
        self.hidden_media_owner_order.push(local_id.to_owned());
        while self.hidden_media_owner_order.len() > self.local_override_limit {
            let evicted_local_id = self.hidden_media_owner_order.remove(0);
            self.hidden_media_slot_indices_by_local_id.remove(&evicted_local_id);
            self.local_content_revisions_by_local_id.remove(&evicted_local_id);
        }
        let revision = self.local_content_revisions_by_local_id.entry(local_id.to_owned()).or_insert(0);
        *revision = revision.wrapping_add(1);
        self.reprepare_scopes(local_id)
    }

    fn release(&mut self, scope: &RiseMessageScope) {
        self.reusable_snapshots.remove(scope);
        self.prepared_snapshots_by_scope.remove(scope);
        self.remove_scope_membership(scope);
        self.revisions_by_scope.remove(scope);
        self.visible_keys_by_scope.remove(scope);
        self.model_cache.retain(|key, _| key.scope != *scope);
    }

    fn release_if_revision(&mut self, scope: &RiseMessageScope, expected_revision: Option<u64>) -> bool {
        if self.revisions_by_scope.get(scope).copied() != expected_revision {
            return false;
        }
        self.release(scope);
        true
    }

    fn retire(&mut self, scope: &RiseMessageScope, expected_revision: Option<u64>) -> bool {
        if self.revisions_by_scope.get(scope).copied() != expected_revision {
            return false;
        }
        let snapshot = self.prepared_snapshots_by_scope.get(scope).cloned();
        self.release(scope);
        if let Some(snapshot) = snapshot {
            let now = RiseClock::monotonic().as_secs_f64();
            let message_count = i64::try_from(snapshot.source.rows.len()).unwrap_or(i64::MAX);
            let cost = chat_engine_hooks().map_or(i64::MAX, |hooks| (hooks.estimated_resident_cost)(message_count));
            if cost <= self.reuse_limits.maximum_estimated_cost {
                self.reusable_snapshots.insert(
                    scope.clone(),
                    RiseMessagePresentationPreparerReusableEntry {
                        snapshot,
                        cost,
                        expires_at: now + self.reuse_limits.time_to_live,
                        access: now,
                    },
                );
            }
        }
        self.evict_reusable_snapshots();
        true
    }

    fn evict_reusable_snapshots(&mut self) {
        let now = RiseClock::monotonic().as_secs_f64();
        self.reusable_snapshots.retain(|_, entry| entry.expires_at > now);
        let mut cost: i64 = self.reusable_snapshots.values().map(|entry| entry.cost).sum();
        while i64::try_from(self.reusable_snapshots.len()).unwrap_or(i64::MAX) > self.reuse_limits.maximum_entry_count
            || cost > self.reuse_limits.maximum_estimated_cost
        {
            let victim = self
                .reusable_snapshots
                .iter()
                .min_by(|lhs, rhs| lhs.1.access.total_cmp(&rhs.1.access))
                .map(|(scope, entry)| (scope.clone(), entry.cost));
            let Some((scope, victim_cost)) = victim else { break };
            cost -= victim_cost;
            self.reusable_snapshots.remove(&scope);
        }
    }

    fn reprepare_scopes(&mut self, local_id: &str) -> Vec<Arc<RiseMessageScopePresentationSnapshot>> {
        self.reusable_snapshots
            .retain(|_, entry| !entry.snapshot.source.local_ids.iter().any(|candidate| candidate == local_id));
        let mut scopes: Vec<RiseMessageScope> =
            self.scopes_by_local_id.get(local_id).map(|scopes| scopes.iter().cloned().collect()).unwrap_or_default();
        scopes.sort_by_cached_key(|scope| scope.storage_key());
        scopes
            .into_iter()
            .filter_map(|scope| {
                let snapshot = self.prepared_snapshots_by_scope.get(&scope)?.source.clone();
                self.prepare(snapshot, false)
            })
            .collect()
    }

    fn update_scope_membership(&mut self, snapshot: &RiseMessageWireSnapshot) {
        self.remove_scope_membership(&snapshot.scope);
        let local_ids: HashSet<String> = snapshot.local_ids.iter().cloned().collect();
        for local_id in &local_ids {
            self.scopes_by_local_id.entry(local_id.clone()).or_default().insert(snapshot.scope.clone());
        }
        self.local_ids_by_scope.insert(snapshot.scope.clone(), local_ids);
    }

    fn remove_scope_membership(&mut self, scope: &RiseMessageScope) {
        let Some(local_ids) = self.local_ids_by_scope.remove(scope) else { return };
        for local_id in local_ids {
            if let Some(scopes) = self.scopes_by_local_id.get_mut(&local_id) {
                scopes.remove(scope);
                if scopes.is_empty() {
                    self.scopes_by_local_id.remove(&local_id);
                }
            }
        }
    }

    fn apply_hidden_media(&self, source: &GetMessageMessage, local_id: &str) -> GetMessageMessage {
        let mut message = source.clone();
        if message.media_items.is_empty() {
            return message;
        }
        let Some(hidden_slots) = self.hidden_media_slot_indices_by_local_id.get(local_id) else { return message };
        for item in &mut message.media_items {
            if hidden_slots.contains(&item.local_slot_index) {
                item.is_cancelled = true;
            }
        }
        message
    }

    fn evict_if_needed(&mut self) {
        if self.model_cache.len() <= self.model_cache_limit {
            return;
        }
        let visible_keys = &self.visible_keys_by_scope;
        self.model_cache.retain(|key, _| visible_keys.values().any(|keys| keys.contains(key)));
        while self.model_cache.len() > self.model_cache_limit {
            let Some(key) = self.model_cache.keys().next().cloned() else { break };
            self.model_cache.remove(&key);
        }
    }
}

pub struct RiseMessagePresentationPreparer {
    state: RiseIsolated<RiseMessagePresentationPreparerState>,
}

impl Default for RiseMessagePresentationPreparer {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseMessagePresentationPreparer {
    pub fn new() -> Self {
        Self { state: RiseIsolated::new(RiseMessagePresentationPreparerState::new()) }
    }

    pub async fn prepare(
        &self,
        snapshot: RiseMessageWireSnapshot,
        reusing_prepared: bool,
    ) -> Option<Arc<RiseMessageScopePresentationSnapshot>> {
        self.state.with(|state| state.prepare(snapshot, reusing_prepared))
    }

    pub async fn has_warm_snapshot(&self, scope: &RiseMessageScope) -> bool {
        self.state.with(|state| state.has_warm_snapshot(scope))
    }

    pub async fn prepare_warm_snapshot(&self, snapshot: RiseMessageWireSnapshot) {
        self.state.with(|state| state.prepare_warm_snapshot(snapshot));
    }

    pub async fn hide_outgoing_message(&self, local_id: &str) -> Vec<Arc<RiseMessageScopePresentationSnapshot>> {
        self.state.with(|state| state.hide_outgoing_message(local_id))
    }

    pub async fn hide_outgoing_media_slot(
        &self,
        local_id: &str,
        local_slot_index: i64,
    ) -> Vec<Arc<RiseMessageScopePresentationSnapshot>> {
        self.state.with(|state| state.hide_outgoing_media_slot(local_id, local_slot_index))
    }

    pub async fn release(&self, scope: &RiseMessageScope) {
        self.state.with(|state| state.release(scope));
    }

    pub async fn release_if_revision(&self, scope: &RiseMessageScope, expected_revision: Option<u64>) -> bool {
        self.state.with(|state| state.release_if_revision(scope, expected_revision))
    }

    pub async fn retire(&self, scope: &RiseMessageScope, expected_revision: Option<u64>) -> bool {
        self.state.with(|state| state.retire(scope, expected_revision))
    }

    pub async fn reset(&self) {
        self.state.with(|state| {
            let presentation_revision = state.presentation_revision;
            let reuse_limits = state.reuse_limits;
            *state = RiseMessagePresentationPreparerState::new();
            state.presentation_revision = presentation_revision;
            state.reuse_limits = reuse_limits;
        });
    }
}

#[derive(Clone)]
pub struct RisePinnedMessagesPresentationSnapshot {
    pub account_id: String,
    pub chat_id: String,
    pub source_revision: i64,
    pub messages: Vec<GetMessageMessage>,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub load_state: RiseConversationAuxiliaryLoadState,
    pub error: Option<String>,
}

struct RisePinnedMessagesPresentationPreparerCacheEntry {
    revision: i64,
    messages: Vec<GetMessageMessage>,
}

#[derive(Default)]
struct RisePinnedMessagesPresentationPreparerState {
    cache: HashMap<String, RisePinnedMessagesPresentationPreparerCacheEntry>,
    order: Vec<String>,
}

pub struct RisePinnedMessagesPresentationPreparer {
    state: RiseIsolated<RisePinnedMessagesPresentationPreparerState>,
}

impl Default for RisePinnedMessagesPresentationPreparer {
    fn default() -> Self {
        Self::new()
    }
}

impl RisePinnedMessagesPresentationPreparer {
    const LIMIT: usize = 24;

    pub fn new() -> Self {
        Self { state: RiseIsolated::new(RisePinnedMessagesPresentationPreparerState::default()) }
    }

    pub async fn prepare(
        &self,
        account_id: &str,
        chat_id: &str,
        page: &RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>,
    ) -> RisePinnedMessagesPresentationSnapshot {
        self.state.with(|state| {
            let messages = match state.cache.get(chat_id).filter(|cached| cached.revision == page.revision) {
                Some(cached) => cached.messages.clone(),
                None => {
                    let messages = MessageThreadOrder::chronological(
                        page.items
                            .iter()
                            .map(|wire| {
                                let mut message = wire.to_message();
                                if message.chat_id.is_empty() {
                                    message.chat_id = chat_id.to_owned();
                                }
                                message
                            })
                            .collect(),
                    );
                    state.cache.insert(
                        chat_id.to_owned(),
                        RisePinnedMessagesPresentationPreparerCacheEntry {
                            revision: page.revision,
                            messages: messages.clone(),
                        },
                    );
                    messages
                }
            };
            state.order.retain(|candidate| candidate != chat_id);
            state.order.push(chat_id.to_owned());
            while state.order.len() > Self::LIMIT {
                let evicted = state.order.remove(0);
                state.cache.remove(&evicted);
            }
            RisePinnedMessagesPresentationSnapshot {
                account_id: account_id.to_owned(),
                chat_id: chat_id.to_owned(),
                source_revision: page.revision,
                messages,
                cursor: page.cursor.clone(),
                has_more: page.has_more,
                load_state: page.load_state,
                error: page.error.clone(),
            }
        })
    }

    pub async fn reset(&self) {
        self.state.with(|state| *state = RisePinnedMessagesPresentationPreparerState::default());
    }

    pub async fn release(&self, chat_id: &str) {
        self.state.with(|state| {
            state.cache.remove(chat_id);
            state.order.retain(|candidate| candidate != chat_id);
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseMessageTabKind {
    Media,
    Files,
    Music,
    Voice,
    Gif,
    Favorites,
    Links,
}

impl RiseMessageTabKind {
    pub const ALL_CASES: [Self; 7] =
        [Self::Media, Self::Files, Self::Music, Self::Voice, Self::Gif, Self::Favorites, Self::Links];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::Files => "files",
            Self::Music => "music",
            Self::Voice => "voice",
            Self::Gif => "gif",
            Self::Favorites => "favorites",
            Self::Links => "links",
        }
    }

    pub fn from_cache_id(cache_id: &str) -> Option<Self> {
        match cache_id {
            "getMediaMessages" => Some(Self::Media),
            rise_wire_methods::message::GET_FILE_MESSAGES => Some(Self::Files),
            rise_wire_methods::message::GET_MUSIC_MESSAGES => Some(Self::Music),
            rise_wire_methods::message::GET_AUDIO_MESSAGES => Some(Self::Voice),
            rise_wire_methods::message::GET_GIF_MESSAGES => Some(Self::Gif),
            rise_wire_methods::message::GET_FAV_MESSAGES => Some(Self::Favorites),
            "getLinkMessages" => Some(Self::Links),
            _ => None,
        }
    }

    pub fn message_list_kind(self) -> Option<RiseConversationMessageListTabKind> {
        match self {
            Self::Files => Some(RiseConversationMessageListTabKind::Files),
            Self::Music => Some(RiseConversationMessageListTabKind::Music),
            Self::Voice => Some(RiseConversationMessageListTabKind::Voice),
            Self::Gif => Some(RiseConversationMessageListTabKind::Gif),
            Self::Favorites => Some(RiseConversationMessageListTabKind::Favorites),
            Self::Media | Self::Links => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseMessageTabScope {
    pub kind: RiseMessageTabKind,
    pub chat_id: String,
    pub topic_id: Option<String>,
}

impl RiseMessageTabScope {
    pub fn new(kind: RiseMessageTabKind, chat_id: String, topic_id: Option<String>) -> Self {
        Self { kind, chat_id, topic_id }
    }

    pub fn from_storage_scope(kind: RiseMessageTabKind, storage_scope: &str) -> Option<Self> {
        let message_scope = RiseMessageScope::parse(storage_scope)?;
        if message_scope.thread_root_id.is_some() {
            return None;
        }
        Some(Self::new(kind, message_scope.chat_id, message_scope.topic_id))
    }
}

/// prepared tab lists, behind one Arc so copies are cheap
pub struct RiseMessageTabPresentationContent {
    pub media_entries: Vec<ChatProfileMediaEntry>,
    pub favorite_entries: Vec<ChatProfileFavEntry>,
    pub link_entries: Vec<ChatProfileLinkEntry>,
    pub media_sections: Vec<ChatProfileSection<ChatProfileMediaEntry>>,
    pub link_sections: Vec<ChatProfileSection<ChatProfileLinkEntry>>,
    pub entry_index_by_id: HashMap<String, i64>,
}

impl RiseMessageTabPresentationContent {
    fn entry_index(
        media_entries: &[ChatProfileMediaEntry],
        favorite_entries: &[ChatProfileFavEntry],
        link_entries: &[ChatProfileLinkEntry],
    ) -> HashMap<String, i64> {
        let ids = media_entries
            .iter()
            .map(|entry| &entry.id)
            .chain(favorite_entries.iter().map(|entry| &entry.id))
            .chain(link_entries.iter().map(|entry| &entry.id));
        let mut index = HashMap::with_capacity(media_entries.len() + favorite_entries.len() + link_entries.len());
        for (offset, id) in ids.enumerate() {
            index.entry(id.clone()).or_insert(i64::try_from(offset).unwrap_or(i64::MAX));
        }
        index
    }
}

#[derive(Clone)]
pub struct RiseMessageTabPresentationSnapshot {
    pub account_id: String,
    pub scope: RiseMessageTabScope,
    pub source_revision: i64,
    pub content: Arc<RiseMessageTabPresentationContent>,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub load_state: RiseConversationAuxiliaryLoadState,
    pub error: Option<String>,
}

struct RiseMessageTabPresentationPreparerCacheEntry {
    revision: i64,
    content: Arc<RiseMessageTabPresentationContent>,
}

#[derive(Default)]
struct RiseMessageTabPresentationPreparerState {
    cache: HashMap<RiseMessageTabScope, RiseMessageTabPresentationPreparerCacheEntry>,
    order: Vec<RiseMessageTabScope>,
}

pub struct RiseMessageTabPresentationPreparer {
    state: RiseIsolated<RiseMessageTabPresentationPreparerState>,
}

impl Default for RiseMessageTabPresentationPreparer {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseMessageTabPresentationPreparer {
    const LIMIT: usize = 32;

    pub fn new() -> Self {
        Self { state: RiseIsolated::new(RiseMessageTabPresentationPreparerState::default()) }
    }

    pub async fn prepare_media(
        &self,
        account_id: &str,
        scope: RiseMessageTabScope,
        page: &RiseConversationAuxiliaryPageSnapshot<MediaItemWireDTO>,
    ) -> RiseMessageTabPresentationSnapshot {
        self.prepare(account_id, scope, page, |_, page| Self::make_media_cache_entry(page))
    }

    pub async fn prepare_messages(
        &self,
        account_id: &str,
        scope: RiseMessageTabScope,
        page: &RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>,
    ) -> RiseMessageTabPresentationSnapshot {
        self.prepare(account_id, scope, page, |scope, page| {
            let messages: Vec<GetMessageMessage> = page
                .items
                .iter()
                .map(|wire| {
                    let mut message = wire.to_message();
                    if message.chat_id.is_empty() {
                        message.chat_id = scope.chat_id.clone();
                    }
                    message
                })
                .collect();
            match scope.kind {
                RiseMessageTabKind::Favorites => Self::make_cache_entry(
                    page.revision,
                    Vec::new(),
                    messages
                        .into_iter()
                        .map(|message| ChatProfileFavEntry { id: format!("fav-{}", message.id), message })
                        .collect(),
                    Vec::new(),
                    false,
                    false,
                ),
                RiseMessageTabKind::Files
                | RiseMessageTabKind::Music
                | RiseMessageTabKind::Voice
                | RiseMessageTabKind::Gif => Self::make_cache_entry(
                    page.revision,
                    Self::flatten(
                        &messages,
                        &Self::content_types(scope.kind),
                        (scope.kind == RiseMessageTabKind::Voice).then_some(&[MessageMediaKind::Voice][..]),
                    ),
                    Vec::new(),
                    Vec::new(),
                    scope.kind != RiseMessageTabKind::Gif,
                    false,
                ),
                RiseMessageTabKind::Media | RiseMessageTabKind::Links => {
                    Self::make_cache_entry(page.revision, Vec::new(), Vec::new(), Vec::new(), false, false)
                }
            }
        })
    }

    pub async fn prepare_links(
        &self,
        account_id: &str,
        scope: RiseMessageTabScope,
        page: &RiseConversationAuxiliaryPageSnapshot<LinkMessageWireDTO>,
    ) -> RiseMessageTabPresentationSnapshot {
        self.prepare(account_id, scope, page, |_, page| {
            Self::make_cache_entry(
                page.revision,
                Vec::new(),
                Vec::new(),
                page.items.iter().filter_map(Self::link_entry).collect(),
                false,
                true,
            )
        })
    }

    pub async fn reset(&self) {
        self.state.with(|state| *state = RiseMessageTabPresentationPreparerState::default());
    }

    pub async fn release(&self, chat_id: &str) {
        self.state.with(|state| {
            state.cache.retain(|scope, _| scope.chat_id != chat_id);
            state.order.retain(|scope| scope.chat_id != chat_id);
        });
    }

    fn prepare<Item>(
        &self,
        account_id: &str,
        scope: RiseMessageTabScope,
        page: &RiseConversationAuxiliaryPageSnapshot<Item>,
        make_entry: impl FnOnce(
            &RiseMessageTabScope,
            &RiseConversationAuxiliaryPageSnapshot<Item>,
        ) -> RiseMessageTabPresentationPreparerCacheEntry,
    ) -> RiseMessageTabPresentationSnapshot {
        self.state.with(|state| {
            let content = match state.cache.get(&scope).filter(|cached| cached.revision == page.revision) {
                Some(cached) => Arc::clone(&cached.content),
                None => {
                    let entry = make_entry(&scope, page);
                    let content = Arc::clone(&entry.content);
                    state.cache.insert(scope.clone(), entry);
                    content
                }
            };
            state.order.retain(|candidate| *candidate != scope);
            state.order.push(scope.clone());
            while state.order.len() > Self::LIMIT {
                let evicted = state.order.remove(0);
                state.cache.remove(&evicted);
            }
            RiseMessageTabPresentationSnapshot {
                account_id: account_id.to_owned(),
                scope,
                source_revision: page.revision,
                content,
                cursor: page.cursor.clone(),
                has_more: page.has_more,
                load_state: page.load_state,
                error: page.error.clone(),
            }
        })
    }

    fn make_cache_entry(
        revision: i64,
        media_entries: Vec<ChatProfileMediaEntry>,
        favorite_entries: Vec<ChatProfileFavEntry>,
        link_entries: Vec<ChatProfileLinkEntry>,
        section_media: bool,
        section_links: bool,
    ) -> RiseMessageTabPresentationPreparerCacheEntry {
        let media_sections = if section_media {
            ChatProfileSectioner::section(&media_entries, |entry| entry.created_at)
        } else {
            Vec::new()
        };
        let link_sections = if section_links {
            ChatProfileSectioner::section(&link_entries, |entry| entry.created_at)
        } else {
            Vec::new()
        };
        let entry_index_by_id =
            RiseMessageTabPresentationContent::entry_index(&media_entries, &favorite_entries, &link_entries);
        RiseMessageTabPresentationPreparerCacheEntry {
            revision,
            content: Arc::new(RiseMessageTabPresentationContent {
                media_entries,
                favorite_entries,
                link_entries,
                media_sections,
                link_sections,
                entry_index_by_id,
            }),
        }
    }

    fn make_media_cache_entry(
        page: &RiseConversationAuxiliaryPageSnapshot<MediaItemWireDTO>,
    ) -> RiseMessageTabPresentationPreparerCacheEntry {
        let mut entries = Vec::with_capacity(page.items.len());
        for (index, wire) in page.items.iter().enumerate() {
            if let Some(entry) = Self::media_entry(wire, index) {
                entries.push(entry);
            }
        }
        Self::make_cache_entry(page.revision, entries, Vec::new(), Vec::new(), false, false)
    }

    fn link_entry(wire: &LinkMessageWireDTO) -> Option<ChatProfileLinkEntry> {
        let link = Self::normalized(wire.link.as_deref())?;
        let message_id = Self::normalized(wire.message_id.as_deref())?;
        let metadata = wire.link_metadata.as_ref();
        let is_big = metadata
            .and_then(|metadata| metadata.link_preview_type.as_deref())
            .is_some_and(|preview_type| preview_type.to_uppercase().starts_with("BIG"));
        let sender = wire.sender.as_ref();
        Some(ChatProfileLinkEntry {
            id: format!("link-{message_id}"),
            created_at: Self::normalized_timestamp(wire.created_at.unwrap_or(0.0)),
            preview: MessageLinkPreview {
                url: link.clone(),
                title: metadata.and_then(|metadata| metadata.title.clone()),
                description: metadata.and_then(|metadata| metadata.description.clone()),
                image_url: metadata.and_then(|metadata| metadata.image_url.clone()),
                size: if is_big { MessageLinkPreviewSize::Big } else { MessageLinkPreviewSize::Small },
                ..MessageLinkPreview::default()
            }
            .localized_for_own_link(),
            source_text: link,
            sender_name: sender.and_then(|sender| sender.name.clone()).unwrap_or_default(),
            sender_id: sender.and_then(|sender| sender.id.clone()).unwrap_or_default(),
            message_id,
        })
    }

    fn media_entry(wire: &MediaItemWireDTO, index: usize) -> Option<ChatProfileMediaEntry> {
        let media = wire.media_info.as_ref()?;
        let timestamp = wire.created_at.or(wire.timestamp).unwrap_or(0.0);
        let sender = wire.sender.as_ref();
        let sender_id =
            wire.sender_id.clone().or_else(|| sender.and_then(|sender| sender.id.clone())).unwrap_or_default();
        let message_id = Self::normalized(wire.message_id.as_deref())
            .or_else(|| Self::normalized(wire.id.as_deref()))
            .unwrap_or_else(|| format!("media:{timestamp:?}:{sender_id}:{index}"));
        let media_index = wire.media_index.unwrap_or(0);
        let content_type = wire.content_type.as_deref().map(MessageContentType::from_raw_value);
        let item = media.to_media_item(media_index, &message_id, content_type);
        if item.kind != MessageMediaKind::Image && item.kind != MessageMediaKind::Video {
            return None;
        }
        if item.view_limit.is_some() {
            return None;
        }
        Some(ChatProfileMediaEntry {
            id: ChatProfileMediaEntry::entry_id(&message_id, media_index),
            created_at: Self::normalized_timestamp(timestamp),
            item,
            sender_name: wire
                .sender_name
                .clone()
                .or_else(|| sender.and_then(|sender| sender.name.clone()))
                .unwrap_or_default(),
            sender_id,
            has_protected_content: wire.has_protected_content.unwrap_or(false),
            sender_logo: None,
            topic_id: Self::normalized(wire.topic_id.as_deref()),
            message_id,
        })
    }

    fn content_types(kind: RiseMessageTabKind) -> Vec<MessageContentType> {
        match kind {
            RiseMessageTabKind::Files => vec![MessageContentType::Document],
            RiseMessageTabKind::Music => vec![MessageContentType::Audio],
            RiseMessageTabKind::Voice => vec![MessageContentType::Audio, MessageContentType::Voice],
            RiseMessageTabKind::Gif => vec![MessageContentType::Animation],
            RiseMessageTabKind::Media | RiseMessageTabKind::Favorites | RiseMessageTabKind::Links => Vec::new(),
        }
    }

    fn flatten(
        messages: &[GetMessageMessage],
        content_types: &[MessageContentType],
        media_kinds: Option<&[MessageMediaKind]>,
    ) -> Vec<ChatProfileMediaEntry> {
        let mut result = Vec::with_capacity(messages.len());
        for message in messages.iter().filter(|message| content_types.contains(&message.content_type)) {
            for (index, item) in message.media_items.iter().enumerate() {
                if !media_kinds.is_none_or(|kinds| kinds.contains(&item.kind)) || item.view_limit.is_some() {
                    continue;
                }
                result.push(ChatProfileMediaEntry {
                    id: ChatProfileMediaEntry::entry_id(&message.id, i64::try_from(index).unwrap_or(i64::MAX)),
                    message_id: message.id.clone(),
                    created_at: message.created_at,
                    item: item.clone(),
                    sender_name: message.sender_name.clone(),
                    sender_id: message.sender_id.clone(),
                    has_protected_content: message.has_protected_content,
                    sender_logo: message.sender_logo.clone(),
                    topic_id: message.topic_id.clone(),
                });
            }
        }
        result
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let normalized = trimming_whitespaces_and_newlines(value?);
        (!normalized.is_empty()).then(|| normalized.to_owned())
    }

    fn normalized_timestamp(value: f64) -> f64 {
        if value > 1_000_000_000_000.0 {
            value / 1_000.0
        } else {
            value
        }
    }
}

#[derive(Default)]
struct RiseMessageScopePresentationBox {
    value: Option<Arc<RiseMessageScopePresentationSnapshot>>,
}

#[derive(Default)]
struct RiseUnreadMarksPresentationBox {
    value: Option<RiseUnreadMarksSnapshot>,
}

struct RisePinnedMessagesPresentationBoxValue {
    revision: u64,
    source_revision: i64,
    messages: Vec<GetMessageMessage>,
    cursor: Option<String>,
    has_more: bool,
    load_state: RiseConversationAuxiliaryLoadState,
    error: Option<String>,
}

#[derive(Default)]
struct RisePinnedMessagesPresentationBox {
    value: Option<RisePinnedMessagesPresentationBoxValue>,
}

#[derive(Clone)]
struct RiseMessageTabPresentationBoxValue {
    revision: u64,
    source_revision: i64,
    content: Arc<RiseMessageTabPresentationContent>,
    cursor: Option<String>,
    has_more: bool,
    load_state: RiseConversationAuxiliaryLoadState,
    error: Option<String>,
}

struct RiseMessageTabPendingDeletion {
    message_ids: HashSet<String>,
    own_chat: bool,
}

#[derive(Default)]
struct RiseMessageTabPresentationBox {
    value: Option<RiseMessageTabPresentationBoxValue>,
    base_value: Option<RiseMessageTabPresentationBoxValue>,
    pending_deletions: HashMap<Uuid, RiseMessageTabPendingDeletion>,
    confirmed_media_ids: HashSet<String>,
    confirmed_favorite_ids: HashSet<String>,
    preparation_generation: Arc<AtomicU64>,
    preparation_task: Option<Task<()>>,
}

impl RiseMessageTabPresentationBox {
    fn install(
        &mut self,
        next: RiseMessageTabPresentationBoxValue,
        scope: &RiseMessageTabScope,
        cx: &mut Context<RiseConversationPresentation>,
    ) {
        let revision = next.revision;
        self.base_value = Some(next);
        self.confirmed_media_ids.clear();
        self.confirmed_favorite_ids.clear();
        self.rebuild(revision, scope, cx);
    }

    fn begin_deletion(
        &mut self,
        message_ids: HashSet<String>,
        own_chat: bool,
        revision: u64,
        scope: &RiseMessageTabScope,
        cx: &mut Context<RiseConversationPresentation>,
    ) -> Uuid {
        let token = Uuid::new_v4();
        self.pending_deletions.insert(token, RiseMessageTabPendingDeletion { message_ids, own_chat });
        self.rebuild(revision, scope, cx);
        token
    }

    fn finish_deletion(
        &mut self,
        token: Uuid,
        revision: u64,
        scope: &RiseMessageTabScope,
        cx: &mut Context<RiseConversationPresentation>,
    ) {
        self.pending_deletions.remove(&token);
        self.rebuild(revision, scope, cx);
    }

    fn remove_confirmed(
        &mut self,
        message_ids: &HashSet<String>,
        own_chat: bool,
        revision: u64,
        scope: &RiseMessageTabScope,
        cx: &mut Context<RiseConversationPresentation>,
    ) -> bool {
        if self.base_value.is_none() {
            return false;
        }
        if own_chat {
            self.confirmed_media_ids.extend(message_ids.iter().cloned());
        }
        self.confirmed_favorite_ids.extend(message_ids.iter().cloned());
        self.rebuild(revision, scope, cx);
        true
    }

    fn invalidate(&mut self) {
        self.preparation_generation.fetch_add(1, Ordering::SeqCst);
        self.preparation_task = None;
        self.base_value = None;
        self.value = None;
        self.pending_deletions.clear();
        self.confirmed_media_ids.clear();
        self.confirmed_favorite_ids.clear();
    }

    fn rebuild(
        &mut self,
        revision: u64,
        scope: &RiseMessageTabScope,
        cx: &mut Context<RiseConversationPresentation>,
    ) {
        let expected_generation = self.preparation_generation.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        self.preparation_task = None;
        let Some(base_value) = self.base_value.clone() else { return };
        let mut media_ids: HashSet<String> = HashSet::new();
        let mut favorite_ids: HashSet<String> = HashSet::new();
        for pending in self.pending_deletions.values() {
            if pending.own_chat {
                media_ids.extend(pending.message_ids.iter().cloned());
            }
            favorite_ids.extend(pending.message_ids.iter().cloned());
        }
        if media_ids.is_empty()
            && favorite_ids.is_empty()
            && self.confirmed_media_ids.is_empty()
            && self.confirmed_favorite_ids.is_empty()
        {
            self.value = Some(Self::with_revision(&base_value, revision));
            return;
        }
        let confirmed_media_ids = self.confirmed_media_ids.clone();
        let confirmed_favorite_ids = self.confirmed_favorite_ids.clone();
        let generation = Arc::clone(&self.preparation_generation);
        let preparation = cx.background_executor().spawn(async move {
            let is_cancelled = || generation.load(Ordering::SeqCst) != expected_generation;
            let confirmed_base = if confirmed_media_ids.is_empty() && confirmed_favorite_ids.is_empty() {
                base_value
            } else {
                let base_revision = base_value.revision;
                Self::filter(&base_value, &confirmed_media_ids, &confirmed_favorite_ids, base_revision, &is_cancelled)?
            };
            let prepared = if media_ids.is_empty() && favorite_ids.is_empty() {
                Self::with_revision(&confirmed_base, revision)
            } else {
                Self::filter(&confirmed_base, &media_ids, &favorite_ids, revision, &is_cancelled)?
            };
            if is_cancelled() {
                return None;
            }
            Some((prepared, confirmed_base))
        });
        let scope = scope.clone();
        self.preparation_task = Some(cx.spawn(async move |this, cx| {
            let Some((prepared, base)) = preparation.await else { return };
            let _ = this.update(cx, |presentation, cx| {
                presentation.install_prepared_message_tab(&scope, prepared, base, expected_generation, cx);
            });
        }));
    }

    fn install_prepared(
        &mut self,
        prepared: RiseMessageTabPresentationBoxValue,
        base: RiseMessageTabPresentationBoxValue,
        generation: u64,
    ) -> bool {
        if generation != self.preparation_generation.load(Ordering::SeqCst) || self.base_value.is_none() {
            return false;
        }
        self.base_value = Some(base);
        self.confirmed_media_ids.clear();
        self.confirmed_favorite_ids.clear();
        self.value = Some(prepared);
        self.preparation_task = None;
        true
    }

    fn with_revision(
        value: &RiseMessageTabPresentationBoxValue,
        revision: u64,
    ) -> RiseMessageTabPresentationBoxValue {
        RiseMessageTabPresentationBoxValue { revision, ..value.clone() }
    }

    fn filtered<T: Clone>(
        items: &[T],
        keeping: impl Fn(&T) -> bool,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<Vec<T>> {
        let mut result = Vec::with_capacity(items.len());
        for item in items {
            if is_cancelled() {
                return None;
            }
            if keeping(item) {
                result.push(item.clone());
            }
        }
        Some(result)
    }

    fn filter_sections<T: Clone>(
        sections: &[ChatProfileSection<T>],
        keeping: impl Fn(&T) -> bool,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<Vec<ChatProfileSection<T>>> {
        let mut result = Vec::new();
        for section in sections {
            let items = Self::filtered(&section.items, &keeping, is_cancelled)?;
            if !items.is_empty() {
                result.push(ChatProfileSection { id: section.id.clone(), label: section.label.clone(), items });
            }
        }
        Some(result)
    }

    fn filter(
        value: &RiseMessageTabPresentationBoxValue,
        media_ids: &HashSet<String>,
        favorite_ids: &HashSet<String>,
        revision: u64,
        is_cancelled: &impl Fn() -> bool,
    ) -> Option<RiseMessageTabPresentationBoxValue> {
        let content = &value.content;
        let media_entries =
            Self::filtered(&content.media_entries, |entry| !media_ids.contains(&entry.message_id), is_cancelled)?;
        let link_entries =
            Self::filtered(&content.link_entries, |entry| !media_ids.contains(&entry.message_id), is_cancelled)?;
        let favorite_entries = Self::filtered(
            &content.favorite_entries,
            |entry| {
                !favorite_ids.contains(&entry.message.id) && !favorite_ids.contains(&entry.message.sai_server_id())
            },
            is_cancelled,
        )?;
        let media_sections =
            Self::filter_sections(&content.media_sections, |entry| !media_ids.contains(&entry.message_id), is_cancelled)?;
        let link_sections =
            Self::filter_sections(&content.link_sections, |entry| !media_ids.contains(&entry.message_id), is_cancelled)?;
        if is_cancelled() {
            return None;
        }
        let entry_index_by_id =
            RiseMessageTabPresentationContent::entry_index(&media_entries, &favorite_entries, &link_entries);
        Some(RiseMessageTabPresentationBoxValue {
            revision,
            source_revision: value.source_revision,
            content: Arc::new(RiseMessageTabPresentationContent {
                media_entries,
                favorite_entries,
                link_entries,
                media_sections,
                link_sections,
                entry_index_by_id,
            }),
            cursor: value.cursor.clone(),
            has_more: value.has_more,
            load_state: value.load_state,
            error: value.error.clone(),
        })
    }
}

/// what a publish changed
/// gpui notifies whole entities so a view that cares about one part subscribes to this event
#[derive(Clone, Debug)]
pub enum RiseConversationPresentationChange {
    All,
    ChatList(RiseChatListKind),
    ChatDetail,
    FavouritesChat,
    MessageScope(RiseMessageScope),
    UnreadMarks(RiseMessageScope),
    PinnedMessages(String),
    MessageTabs,
    GlobalMessageSearch,
    MessageSearch,
    ChatOverlaySearch,
    ChatSearchRecents,
}

pub struct RiseConversationPresentation {
    account_id: Option<String>,
    generation: u64,
    chat_revision: u64,
    message_revision: u64,
    pinned_presentation_revision: u64,
    message_tab_presentation_revision: u64,
    global_message_search_presentation_revision: u64,
    chat_overlay_search_presentation_revision: u64,
    message_search_presentation_revision: u64,
    chat_search_recents_revision: u64,
    confirmed_removed_chat_ids: HashSet<String>,

    inbox_chats: RiseChatListPresentationBox,
    archived_chats: RiseChatListPresentationBox,
    requested_chats: RiseChatListPresentationBox,
    chat_preparation_generation: u64,
    standalone_chats_by_id: HashMap<String, ChatInfo>,
    favourites_chat_id: Option<String>,
    standalone_chat_order: Vec<String>,
    standalone_archived_chat_ids: HashSet<String>,
    message_scope_boxes: HashMap<RiseMessageScope, RiseMessageScopePresentationBox>,
    pinned_message_boxes: HashMap<String, RisePinnedMessagesPresentationBox>,
    unread_marks_boxes: HashMap<RiseMessageScope, RiseUnreadMarksPresentationBox>,
    message_tab_boxes: HashMap<RiseMessageTabScope, RiseMessageTabPresentationBox>,
    message_tab_box_order: Vec<RiseMessageTabScope>,
    global_message_search_boxes: HashMap<RiseGlobalMessageSearchScope, RiseGlobalMessageSearchPresentationBox>,
    global_message_search_box_order: Vec<RiseGlobalMessageSearchScope>,
    message_search_boxes: HashMap<RiseMessageSearchScope, RiseMessageSearchPresentationBox>,
    message_search_box_order: Vec<RiseMessageSearchScope>,
    chat_overlay_search_boxes: HashMap<RiseChatOverlaySearchScope, RiseChatOverlaySearchPresentationBox>,
    chat_overlay_search_box_order: Vec<RiseChatOverlaySearchScope>,
    chat_search_recents: Vec<RiseChatSearchRecentItem>,

    empty_content_stamps: HashMap<String, RiseMessagePresentationContentStamp>,
    empty_entry_index: HashMap<String, i64>,
}

impl EventEmitter<RiseConversationPresentationChange> for RiseConversationPresentation {}

impl RiseConversationPresentation {
    const STANDALONE_CHAT_LIMIT: usize = 128;
    const MESSAGE_TAB_BOX_LIMIT: usize = 64;
    const GLOBAL_MESSAGE_SEARCH_BOX_LIMIT: usize = 32;
    const CHAT_OVERLAY_SEARCH_BOX_LIMIT: usize = 32;
    const MESSAGE_SEARCH_BOX_LIMIT: usize = 8;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            account_id: None,
            generation: 0,
            chat_revision: 0,
            message_revision: 0,
            pinned_presentation_revision: 0,
            message_tab_presentation_revision: 0,
            global_message_search_presentation_revision: 0,
            chat_overlay_search_presentation_revision: 0,
            message_search_presentation_revision: 0,
            chat_search_recents_revision: 0,
            confirmed_removed_chat_ids: HashSet::new(),
            inbox_chats: RiseChatListPresentationBox::default(),
            archived_chats: RiseChatListPresentationBox::default(),
            requested_chats: RiseChatListPresentationBox::default(),
            chat_preparation_generation: 0,
            standalone_chats_by_id: HashMap::new(),
            favourites_chat_id: None,
            standalone_chat_order: Vec::new(),
            standalone_archived_chat_ids: HashSet::new(),
            message_scope_boxes: HashMap::new(),
            pinned_message_boxes: HashMap::new(),
            unread_marks_boxes: HashMap::new(),
            message_tab_boxes: HashMap::new(),
            message_tab_box_order: Vec::new(),
            global_message_search_boxes: HashMap::new(),
            global_message_search_box_order: Vec::new(),
            message_search_boxes: HashMap::new(),
            message_search_box_order: Vec::new(),
            chat_overlay_search_boxes: HashMap::new(),
            chat_overlay_search_box_order: Vec::new(),
            chat_search_recents: Vec::new(),
            empty_content_stamps: HashMap::new(),
            empty_entry_index: HashMap::new(),
        }
    }

    pub fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn chat_revision(&self) -> u64 {
        self.chat_revision
    }

    pub fn message_revision(&self) -> u64 {
        self.message_revision
    }

    pub fn pinned_presentation_revision(&self) -> u64 {
        self.pinned_presentation_revision
    }

    pub fn message_tab_presentation_revision(&self) -> u64 {
        self.message_tab_presentation_revision
    }

    pub fn global_message_search_presentation_revision(&self) -> u64 {
        self.global_message_search_presentation_revision
    }

    pub fn chat_overlay_search_presentation_revision(&self) -> u64 {
        self.chat_overlay_search_presentation_revision
    }

    pub fn message_search_presentation_revision(&self) -> u64 {
        self.message_search_presentation_revision
    }

    pub fn chat_search_recents_revision(&self) -> u64 {
        self.chat_search_recents_revision
    }

    pub fn confirmed_removed_chat_ids(&self) -> &HashSet<String> {
        &self.confirmed_removed_chat_ids
    }

    pub fn favourites_chat_id(&self) -> Option<&str> {
        self.favourites_chat_id.as_deref()
    }

    pub fn is_active(&self) -> bool {
        self.account_id.is_some()
    }

    pub fn remember_favourites_chat(&mut self, chats: &[ChatInfo], cx: &mut Context<Self>) {
        if let Some(chat) = chats.iter().find(|chat| chat.r#type == ChatType::Favourites) {
            if self.favourites_chat_id.as_deref() == Some(chat.id.as_str()) {
                return;
            }
            self.favourites_chat_id = Some(chat.id.clone());
            Self::changed(RiseConversationPresentationChange::FavouritesChat, cx);
        }
    }

    pub fn activate(&mut self, account_id: &str, cx: &mut Context<Self>) {
        let normalized = trimming_whitespaces_and_newlines(account_id);
        if normalized.is_empty() {
            self.deactivate(cx);
            return;
        }
        if self.account_id.as_deref() == Some(normalized) {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.account_id = Some(normalized.to_owned());
        self.clear_snapshots();
        Self::changed(RiseConversationPresentationChange::All, cx);
    }

    pub fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.account_id = None;
        self.clear_snapshots();
        Self::changed(RiseConversationPresentationChange::All, cx);
        RiseChatUnreadBadgePresentation::shared(cx).update(cx, |presentation, cx| presentation.reset(cx));
    }

    pub fn publish_chat_list(&mut self, snapshot: RiseChatListPresentationSnapshot, cx: &mut Context<Self>) {
        if Some(snapshot.source.account_id.as_str()) != self.account_id.as_deref() {
            return;
        }
        let kind = snapshot.source.kind;
        if snapshot.generation <= self.chat_box(kind).value.as_ref().map_or(0, |value| value.generation) {
            return;
        }
        let badge = (kind == RiseChatListKind::Inbox).then(|| RiseChatUnreadBadge::from_snapshot(&snapshot.source));
        self.remember_favourites_chat(&snapshot.chats, cx);
        if snapshot.generation > self.chat_preparation_generation {
            self.chat_preparation_generation = snapshot.generation;
            self.confirmed_removed_chat_ids = snapshot.confirmed_removed_chat_ids.clone();
        }
        self.chat_box_mut(kind).value = Some(snapshot);
        self.chat_revision = self.chat_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::ChatList(kind), cx);
        if let Some(badge) = badge {
            RiseChatUnreadBadgePresentation::shared(cx).update(cx, |presentation, cx| presentation.publish(badge, cx));
        }
    }

    pub fn publish_chat_detail(
        &mut self,
        wire: RiseStoredChat,
        account_id: &str,
        is_archived: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let id = RiseConversationStoreLayout::canonical_chat_server_id(&wire);
        self.publish_chat_detail_model(wire.to_chat_info(), &id, account_id, is_archived, cx);
    }

    pub fn publish_chat_detail_model(
        &mut self,
        model: ChatInfo,
        id: &str,
        account_id: &str,
        is_archived: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        if self.account_id.as_deref() != Some(account_id) || id.is_empty() {
            return;
        }
        let unchanged = self.standalone_chats_by_id.get(id) == Some(&model)
            && is_archived.is_none_or(|archived| archived == self.standalone_archived_chat_ids.contains(id));
        if unchanged {
            self.standalone_chat_order.retain(|candidate| candidate != id);
            self.standalone_chat_order.push(id.to_owned());
            return;
        }
        self.standalone_chats_by_id.insert(id.to_owned(), model);
        if let Some(is_archived) = is_archived {
            if is_archived {
                self.standalone_archived_chat_ids.insert(id.to_owned());
            } else {
                self.standalone_archived_chat_ids.remove(id);
            }
        }
        self.standalone_chat_order.retain(|candidate| candidate != id);
        self.standalone_chat_order.push(id.to_owned());
        while self.standalone_chat_order.len() > Self::STANDALONE_CHAT_LIMIT {
            let evicted = self.standalone_chat_order.remove(0);
            self.standalone_chats_by_id.remove(&evicted);
            self.standalone_archived_chat_ids.remove(&evicted);
        }
        self.chat_revision = self.chat_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::ChatDetail, cx);
    }

    pub fn is_chat_archived(&self, chat_id: &str) -> bool {
        if self.confirmed_removed_chat_ids.contains(chat_id) {
            return false;
        }
        self.archived_chats.value.as_ref().is_some_and(|value| value.index.contains_key(chat_id))
            || self.standalone_archived_chat_ids.contains(chat_id)
    }

    pub fn remove_chat_detail(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.standalone_chats_by_id.remove(chat_id);
        self.standalone_archived_chat_ids.remove(chat_id);
        self.standalone_chat_order.retain(|candidate| candidate != chat_id);
        self.chat_revision = self.chat_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::ChatDetail, cx);
    }

    /// publishes a message scope snapshot
    /// the preparer gives an Arc so the window reaches the main thread without copying rows
    pub fn publish_message_scope(
        &mut self,
        snapshot: impl Into<Arc<RiseMessageScopePresentationSnapshot>>,
        cx: &mut Context<Self>,
    ) -> Option<RiseMessagePresentationPublishReceipt> {
        let snapshot = snapshot.into();
        if Some(snapshot.source.account_id.as_str()) != self.account_id.as_deref()
            || snapshot.source.scope.chat_id.is_empty()
            || snapshot.messages.len() != snapshot.wires.len()
            || snapshot.messages.len() != snapshot.visible_local_ids.len()
            || snapshot.messages.len() != snapshot.content_stamps_by_local_id.len()
        {
            return None;
        }
        let scope = snapshot.source.scope.clone();
        let current = self.message_scope_box(&scope);
        if snapshot.revision <= current.value.as_ref().map_or(0, |value| value.revision) {
            return None;
        }
        let receipt = RiseMessagePresentationPublishReceipt {
            account_id: snapshot.source.account_id.clone(),
            scope: scope.clone(),
            source_revision: snapshot.source.revision,
            presentation_revision: snapshot.revision,
        };
        current.value = Some(snapshot);
        self.message_revision = self.message_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::MessageScope(scope), cx);
        Some(receipt)
    }

    pub fn publish_pinned_messages(
        &mut self,
        snapshot: RisePinnedMessagesPresentationSnapshot,
        cx: &mut Context<Self>,
    ) {
        if Some(snapshot.account_id.as_str()) != self.account_id.as_deref() || snapshot.chat_id.is_empty() {
            return;
        }
        let current = self.pinned_message_box(&snapshot.chat_id);
        current.value = Some(RisePinnedMessagesPresentationBoxValue {
            revision: current.value.as_ref().map_or(0, |value| value.revision).wrapping_add(1),
            source_revision: snapshot.source_revision,
            messages: snapshot.messages,
            cursor: snapshot.cursor,
            has_more: snapshot.has_more,
            load_state: snapshot.load_state,
            error: snapshot.error,
        });
        self.pinned_presentation_revision = self.pinned_presentation_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::PinnedMessages(snapshot.chat_id), cx);
    }

    pub fn publish_message_tab(&mut self, snapshot: RiseMessageTabPresentationSnapshot, cx: &mut Context<Self>) {
        if Some(snapshot.account_id.as_str()) != self.account_id.as_deref() || snapshot.scope.chat_id.is_empty() {
            return;
        }
        let revision = self.message_tab_presentation_revision.wrapping_add(1);
        let scope = snapshot.scope;
        let value = RiseMessageTabPresentationBoxValue {
            revision,
            source_revision: snapshot.source_revision,
            content: snapshot.content,
            cursor: snapshot.cursor,
            has_more: snapshot.has_more,
            load_state: snapshot.load_state,
            error: snapshot.error,
        };
        self.message_tab_box(&scope).install(value, &scope, cx);
        self.touch_message_tab_box(&scope);
        self.message_tab_presentation_revision = revision;
        Self::changed(RiseConversationPresentationChange::MessageTabs, cx);
    }

    pub fn remove_message_tab_entries(
        &mut self,
        chat_id: &str,
        message_ids: &HashSet<String>,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() || message_ids.is_empty() {
            return;
        }
        let revision = self.message_tab_presentation_revision.wrapping_add(1);
        let mut changed = false;
        for (scope, tab_box) in &mut self.message_tab_boxes {
            if scope.chat_id != chat_id && scope.kind != RiseMessageTabKind::Favorites {
                continue;
            }
            if tab_box.remove_confirmed(message_ids, scope.chat_id == chat_id, revision, scope, cx) {
                changed = true;
            }
        }
        if changed {
            self.message_tab_presentation_revision = revision;
            Self::changed(RiseConversationPresentationChange::MessageTabs, cx);
        }
    }

    pub fn begin_message_tab_deletion(
        &mut self,
        chat_id: &str,
        message_ids: &HashSet<String>,
        cx: &mut Context<Self>,
    ) -> Box<dyn FnOnce(&mut App) + Send> {
        let expected_account_id = self.account_id.clone();
        let expected_generation = self.generation;
        let revision = self.message_tab_presentation_revision.wrapping_add(1);
        let mut pending: Vec<(RiseMessageTabScope, Uuid)> = Vec::new();
        for (scope, tab_box) in &mut self.message_tab_boxes {
            if scope.chat_id != chat_id && scope.kind != RiseMessageTabKind::Favorites {
                continue;
            }
            let token = tab_box.begin_deletion(message_ids.clone(), scope.chat_id == chat_id, revision, scope, cx);
            pending.push((scope.clone(), token));
        }
        self.message_tab_presentation_revision = revision;
        Self::changed(RiseConversationPresentationChange::MessageTabs, cx);
        let this = cx.weak_entity();
        Box::new(move |cx: &mut App| {
            let _ = this.update(cx, |presentation, cx| {
                if presentation.account_id != expected_account_id || presentation.generation != expected_generation {
                    return;
                }
                let revision = presentation.message_tab_presentation_revision.wrapping_add(1);
                for (scope, token) in pending {
                    if let Some(tab_box) = presentation.message_tab_boxes.get_mut(&scope) {
                        tab_box.finish_deletion(token, revision, &scope, cx);
                    }
                }
                presentation.message_tab_presentation_revision = revision;
                Self::changed(RiseConversationPresentationChange::MessageTabs, cx);
            });
        })
    }

    pub fn remove_pinned_entries(&mut self, chat_id: &str, message_ids: &HashSet<String>, cx: &mut Context<Self>) {
        if chat_id.is_empty() || message_ids.is_empty() {
            return;
        }
        let Some(value) = self.pinned_message_boxes.get_mut(chat_id).and_then(|pinned| pinned.value.as_mut()) else {
            return;
        };
        let count = value.messages.len();
        value.messages.retain(|message| !message_ids.contains(&message.id) && !message_ids.contains(&message.sai_server_id()));
        if value.messages.len() == count {
            return;
        }
        value.revision = value.revision.wrapping_add(1);
        self.pinned_presentation_revision = self.pinned_presentation_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::PinnedMessages(chat_id.to_owned()), cx);
    }

    pub fn publish_global_message_search(
        &mut self,
        snapshot: RiseGlobalMessageSearchPresentationSnapshot,
        cx: &mut Context<Self>,
    ) {
        if Some(snapshot.account_id.as_str()) != self.account_id.as_deref() {
            return;
        }
        let current = self.global_message_search_box(&snapshot.scope);
        current.value = Some(RiseGlobalMessageSearchPresentationState {
            results: snapshot.results,
            source_message_count: snapshot.source_message_count,
            cursor: snapshot.cursor,
            has_more: snapshot.has_more,
            load_state: snapshot.load_state,
            error: snapshot.error,
            revision: current.value.as_ref().map_or(0, |value| value.revision).wrapping_add(1),
        });
        self.touch_global_message_search_box(&snapshot.scope);
        self.global_message_search_presentation_revision =
            self.global_message_search_presentation_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::GlobalMessageSearch, cx);
    }

    pub fn publish_message_search(
        &mut self,
        snapshot: RiseMessageSearchPresentationSnapshot,
        cx: &mut Context<Self>,
    ) {
        if Some(snapshot.account_id.as_str()) != self.account_id.as_deref() {
            return;
        }
        let current = self.message_search_box(&snapshot.scope);
        current.value = Some(RiseMessageSearchPresentationState {
            hits: snapshot.hits,
            cursor: snapshot.cursor,
            has_more: snapshot.has_more,
            has_newer: snapshot.has_newer,
            total_count: snapshot.total_count,
            load_state: snapshot.load_state,
            error: snapshot.error,
            revision: current.value.as_ref().map_or(0, |value| value.revision).wrapping_add(1),
        });
        self.touch_message_search_box(&snapshot.scope);
        self.message_search_presentation_revision = self.message_search_presentation_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::MessageSearch, cx);
    }

    pub fn publish_chat_overlay_entity_search(
        &mut self,
        snapshot: RiseChatOverlayEntitySearchSnapshot,
        cx: &mut Context<Self>,
    ) {
        if Some(snapshot.account_id.as_str()) != self.account_id.as_deref() {
            return;
        }
        let current = self.chat_overlay_search_box(&snapshot.scope);
        let state = current.value.take().unwrap_or_else(RiseChatOverlaySearchPresentationState::empty);
        current.value = Some(RiseChatOverlaySearchPresentationState {
            entities: snapshot.page,
            chats: state.chats,
            posts: state.posts,
            revision: state.revision.wrapping_add(1),
        });
        self.finish_chat_overlay_search_publish(&snapshot.scope, cx);
    }

    pub fn publish_chat_overlay_chat_search(
        &mut self,
        snapshot: RiseChatOverlayChatSearchSnapshot,
        cx: &mut Context<Self>,
    ) {
        if Some(snapshot.account_id.as_str()) != self.account_id.as_deref() {
            return;
        }
        let current = self.chat_overlay_search_box(&snapshot.scope);
        let state = current.value.take().unwrap_or_else(RiseChatOverlaySearchPresentationState::empty);
        current.value = Some(RiseChatOverlaySearchPresentationState {
            entities: state.entities,
            chats: snapshot.page,
            posts: state.posts,
            revision: state.revision.wrapping_add(1),
        });
        self.finish_chat_overlay_search_publish(&snapshot.scope, cx);
    }

    pub fn publish_chat_overlay_post_search(
        &mut self,
        snapshot: RiseChatOverlayPostSearchSnapshot,
        cx: &mut Context<Self>,
    ) {
        if Some(snapshot.account_id.as_str()) != self.account_id.as_deref() {
            return;
        }
        let current = self.chat_overlay_search_box(&snapshot.scope);
        let state = current.value.take().unwrap_or_else(RiseChatOverlaySearchPresentationState::empty);
        current.value = Some(RiseChatOverlaySearchPresentationState {
            entities: state.entities,
            chats: state.chats,
            posts: snapshot.page,
            revision: state.revision.wrapping_add(1),
        });
        self.finish_chat_overlay_search_publish(&snapshot.scope, cx);
    }

    fn finish_chat_overlay_search_publish(&mut self, scope: &RiseChatOverlaySearchScope, cx: &mut Context<Self>) {
        self.touch_chat_overlay_search_box(scope);
        self.chat_overlay_search_presentation_revision = self.chat_overlay_search_presentation_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::ChatOverlaySearch, cx);
    }

    pub fn publish_chat_search_recents(
        &mut self,
        account_id: &str,
        items: Vec<RiseChatSearchRecentItem>,
        cx: &mut Context<Self>,
    ) {
        if self.account_id.as_deref() != Some(account_id) {
            return;
        }
        self.chat_search_recents = items;
        self.chat_search_recents_revision = self.chat_search_recents_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::ChatSearchRecents, cx);
    }

    pub fn chats(&self, kind: RiseChatListKind) -> &[ChatInfo] {
        self.chat_box(kind).value.as_ref().map_or(&[], |value| value.chats.as_slice())
    }

    pub fn chat_wires(&self, kind: RiseChatListKind) -> &[RiseStoredChat] {
        self.chat_box(kind).value.as_ref().map_or(&[], |value| value.wires.as_slice())
    }

    pub fn chat_state(&self, kind: RiseChatListKind) -> Option<&RiseChatListWireSnapshot> {
        self.chat_box(kind).value.as_ref().map(|value| &value.source)
    }

    pub fn chat_render_status(&self, kind: RiseChatListKind) -> SaiWsSliceStatus {
        let Some(state) = self.chat_state(kind) else { return SaiWsSliceStatus::Idle };
        match state.load_state {
            RiseEntityViewLoadState::Idle => {
                if state.rows.is_empty() {
                    SaiWsSliceStatus::Idle
                } else {
                    SaiWsSliceStatus::Fulfilled
                }
            }
            RiseEntityViewLoadState::Loading => {
                if state.rows.is_empty() {
                    SaiWsSliceStatus::Pending
                } else {
                    SaiWsSliceStatus::Fulfilled
                }
            }
            RiseEntityViewLoadState::Loaded => SaiWsSliceStatus::Fulfilled,
            RiseEntityViewLoadState::Failed => {
                if state.rows.is_empty() {
                    SaiWsSliceStatus::Rejected
                } else {
                    SaiWsSliceStatus::Fulfilled
                }
            }
        }
    }

    pub fn chat_render_error(&self, kind: RiseChatListKind) -> Option<String> {
        let code = self.chat_state(kind).and_then(|state| state.error_code.as_deref());
        CanonErrors::text(code, &HashMap::new())
    }

    pub fn chat(&self, id: &str) -> Option<&ChatInfo> {
        if self.confirmed_removed_chat_ids.contains(id) {
            return self.standalone_chats_by_id.get(id).filter(|card| card.is_community_preview());
        }
        [&self.inbox_chats, &self.archived_chats, &self.requested_chats]
            .into_iter()
            .find_map(|list| list.value.as_ref().and_then(|value| value.index.get(id)))
            .or_else(|| self.standalone_chats_by_id.get(id))
    }

    pub fn messages(&self, scope: &RiseMessageScope) -> &[GetMessageMessage] {
        self.message_scope_value(scope).map_or(&[], |value| value.messages.as_slice())
    }

    pub fn message_wires(&self, scope: &RiseMessageScope) -> &[MessageWireDTO] {
        self.message_scope_value(scope).map_or(&[], |value| value.wires.as_slice())
    }

    pub fn message_state(&self, scope: &RiseMessageScope) -> Option<&RiseMessageWireSnapshot> {
        self.message_scope_value(scope).map(|value| &value.source)
    }

    pub fn message_revision_for(&self, scope: &RiseMessageScope) -> u64 {
        self.message_scope_value(scope).map_or(0, |value| value.revision)
    }

    pub fn message_content_stamps(
        &self,
        scope: &RiseMessageScope,
    ) -> &HashMap<String, RiseMessagePresentationContentStamp> {
        self.message_scope_value(scope).map_or(&self.empty_content_stamps, |value| &value.content_stamps_by_local_id)
    }

    pub fn message_scope_snapshot(&self, scope: &RiseMessageScope) -> Option<Arc<RiseMessageScopePresentationSnapshot>> {
        self.message_scope_value(scope).cloned()
    }

    pub fn remove_message_scope(&mut self, scope: &RiseMessageScope, cx: &mut Context<Self>) {
        if self.message_scope_boxes.remove(scope).is_none() {
            return;
        }
        self.message_revision = self.message_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::MessageScope(scope.clone()), cx);
    }

    /// removes the message scope only if its revision matches
    pub fn remove_message_scope_if_revision(
        &mut self,
        scope: &RiseMessageScope,
        expected_revision: Option<u64>,
        cx: &mut Context<Self>,
    ) -> bool {
        let current_revision = self.message_scope_value(scope).map(|value| value.revision);
        if current_revision != expected_revision {
            return false;
        }
        if self.message_scope_boxes.remove(scope).is_none() {
            return true;
        }
        self.message_revision = self.message_revision.wrapping_add(1);
        Self::changed(RiseConversationPresentationChange::MessageScope(scope.clone()), cx);
        true
    }

    pub fn clear_message_history(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let message_scopes: Vec<RiseMessageScope> =
            self.message_scope_boxes.keys().filter(|scope| scope.chat_id == chat_id).cloned().collect();
        for scope in &message_scopes {
            self.message_scope_boxes.remove(scope);
        }
        if !message_scopes.is_empty() {
            self.message_revision = self.message_revision.wrapping_add(1);
        }
        for scope in message_scopes {
            Self::changed(RiseConversationPresentationChange::MessageScope(scope), cx);
        }

        if self.pinned_message_boxes.remove(chat_id).is_some() {
            self.pinned_presentation_revision = self.pinned_presentation_revision.wrapping_add(1);
            Self::changed(RiseConversationPresentationChange::PinnedMessages(chat_id.to_owned()), cx);
        }

        let tab_scopes: Vec<RiseMessageTabScope> =
            self.message_tab_boxes.keys().filter(|scope| scope.chat_id == chat_id).cloned().collect();
        for scope in &tab_scopes {
            if let Some(mut tab_box) = self.message_tab_boxes.remove(scope) {
                tab_box.invalidate();
            }
        }
        if !tab_scopes.is_empty() {
            self.message_tab_box_order.retain(|scope| scope.chat_id != chat_id);
            self.message_tab_presentation_revision = self.message_tab_presentation_revision.wrapping_add(1);
            Self::changed(RiseConversationPresentationChange::MessageTabs, cx);
        }
    }

    /// unread mention and reaction ledger of one scope as last published
    /// only the chat showing this scope re renders
    pub fn unread_marks(&self, scope: &RiseMessageScope) -> Option<&RiseUnreadMarksSnapshot> {
        self.unread_marks_boxes.get(scope).and_then(|marks| marks.value.as_ref())
    }

    pub fn publish_unread_marks(
        &mut self,
        snapshot: RiseUnreadMarksSnapshot,
        account_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.account_id.as_deref() != Some(account_id) {
            return;
        }
        let scope = snapshot.scope.clone();
        let current = self.unread_marks_box(&scope);
        if snapshot.revision < current.value.as_ref().map_or(0, |value| value.revision) {
            return;
        }
        if current.value.as_ref() == Some(&snapshot) {
            return;
        }
        current.value = Some(snapshot);
        Self::changed(RiseConversationPresentationChange::UnreadMarks(scope), cx);
    }

    pub fn remove_unread_marks(&mut self, scope: &RiseMessageScope, cx: &mut Context<Self>) {
        if self.unread_marks_boxes.remove(scope).is_none() {
            return;
        }
        Self::changed(RiseConversationPresentationChange::UnreadMarks(scope.clone()), cx);
    }

    fn unread_marks_box(&mut self, scope: &RiseMessageScope) -> &mut RiseUnreadMarksPresentationBox {
        self.unread_marks_boxes.entry(scope.clone()).or_default()
    }

    pub fn pinned_messages(&self, chat_id: &str) -> &[GetMessageMessage] {
        self.pinned_value(chat_id).map_or(&[], |value| value.messages.as_slice())
    }

    pub fn pinned_messages_revision(&self, chat_id: &str) -> u64 {
        self.pinned_value(chat_id).map_or(0, |value| value.revision)
    }

    pub fn pinned_messages_cursor(&self, chat_id: &str) -> Option<&str> {
        self.pinned_value(chat_id).and_then(|value| value.cursor.as_deref())
    }

    pub fn pinned_messages_has_more(&self, chat_id: &str) -> bool {
        self.pinned_value(chat_id).is_some_and(|value| value.has_more)
    }

    pub fn pinned_messages_has_resolved_initial(&self, chat_id: &str) -> bool {
        self.pinned_value(chat_id).is_some_and(|value| {
            value.source_revision > 0
                || matches!(
                    value.load_state,
                    RiseConversationAuxiliaryLoadState::Loaded | RiseConversationAuxiliaryLoadState::Failed
                )
        })
    }

    pub fn pinned_messages_error(&self, chat_id: &str) -> Option<&str> {
        self.pinned_value(chat_id).and_then(|value| value.error.as_deref())
    }

    pub fn message_tab_media_entries(&self, scope: &RiseMessageTabScope) -> &[ChatProfileMediaEntry] {
        self.message_tab_value(scope).map_or(&[], |value| value.content.media_entries.as_slice())
    }

    pub fn message_tab_favorite_entries(&self, scope: &RiseMessageTabScope) -> &[ChatProfileFavEntry] {
        self.message_tab_value(scope).map_or(&[], |value| value.content.favorite_entries.as_slice())
    }

    pub fn message_tab_link_entries(&self, scope: &RiseMessageTabScope) -> &[ChatProfileLinkEntry] {
        self.message_tab_value(scope).map_or(&[], |value| value.content.link_entries.as_slice())
    }

    pub fn message_tab_media_sections(
        &self,
        scope: &RiseMessageTabScope,
    ) -> &[ChatProfileSection<ChatProfileMediaEntry>] {
        self.message_tab_value(scope).map_or(&[], |value| value.content.media_sections.as_slice())
    }

    pub fn message_tab_link_sections(
        &self,
        scope: &RiseMessageTabScope,
    ) -> &[ChatProfileSection<ChatProfileLinkEntry>] {
        self.message_tab_value(scope).map_or(&[], |value| value.content.link_sections.as_slice())
    }

    pub fn message_tab_entry_index_by_id(&self, scope: &RiseMessageTabScope) -> &HashMap<String, i64> {
        self.message_tab_value(scope).map_or(&self.empty_entry_index, |value| &value.content.entry_index_by_id)
    }

    pub fn message_tab_revision(&self, scope: &RiseMessageTabScope) -> u64 {
        self.message_tab_value(scope).map_or(0, |value| value.revision)
    }

    pub fn message_tab_cursor(&self, scope: &RiseMessageTabScope) -> Option<&str> {
        self.message_tab_value(scope).and_then(|value| value.cursor.as_deref())
    }

    pub fn message_tab_has_more(&self, scope: &RiseMessageTabScope) -> bool {
        self.message_tab_value(scope).is_some_and(|value| value.has_more)
    }

    pub fn message_tab_has_resolved_initial(&self, scope: &RiseMessageTabScope) -> bool {
        self.message_tab_value(scope).is_some_and(|value| {
            value.source_revision > 0
                || matches!(
                    value.load_state,
                    RiseConversationAuxiliaryLoadState::Loaded | RiseConversationAuxiliaryLoadState::Failed
                )
        })
    }

    pub fn message_tab_error(&self, scope: &RiseMessageTabScope) -> Option<&str> {
        self.message_tab_value(scope).and_then(|value| value.error.as_deref())
    }

    pub fn global_message_search_state(
        &self,
        scope: &RiseGlobalMessageSearchScope,
    ) -> RiseGlobalMessageSearchPresentationState {
        self.global_message_search_boxes
            .get(scope)
            .and_then(|search| search.value.clone())
            .unwrap_or_else(RiseGlobalMessageSearchPresentationState::empty)
    }

    pub fn message_search_state(&self, scope: &RiseMessageSearchScope) -> RiseMessageSearchPresentationState {
        self.message_search_boxes
            .get(scope)
            .and_then(|search| search.value.clone())
            .unwrap_or_else(RiseMessageSearchPresentationState::empty)
    }

    pub fn chat_overlay_search_state(
        &self,
        scope: &RiseChatOverlaySearchScope,
    ) -> RiseChatOverlaySearchPresentationState {
        self.chat_overlay_search_boxes
            .get(scope)
            .and_then(|search| search.value.clone())
            .unwrap_or_else(RiseChatOverlaySearchPresentationState::empty)
    }

    pub fn chat_search_recent_items(&self) -> Vec<RiseChatSearchRecentItem> {
        self.chat_search_recents.clone()
    }

    fn changed(change: RiseConversationPresentationChange, cx: &mut Context<Self>) {
        cx.emit(change);
        cx.notify();
    }

    fn chat_box(&self, kind: RiseChatListKind) -> &RiseChatListPresentationBox {
        match kind {
            RiseChatListKind::Inbox => &self.inbox_chats,
            RiseChatListKind::Archive => &self.archived_chats,
            RiseChatListKind::Requests => &self.requested_chats,
        }
    }

    fn chat_box_mut(&mut self, kind: RiseChatListKind) -> &mut RiseChatListPresentationBox {
        match kind {
            RiseChatListKind::Inbox => &mut self.inbox_chats,
            RiseChatListKind::Archive => &mut self.archived_chats,
            RiseChatListKind::Requests => &mut self.requested_chats,
        }
    }

    fn message_scope_value(&self, scope: &RiseMessageScope) -> Option<&Arc<RiseMessageScopePresentationSnapshot>> {
        self.message_scope_boxes.get(scope).and_then(|scope_box| scope_box.value.as_ref())
    }

    fn pinned_value(&self, chat_id: &str) -> Option<&RisePinnedMessagesPresentationBoxValue> {
        self.pinned_message_boxes.get(chat_id).and_then(|pinned| pinned.value.as_ref())
    }

    fn message_tab_value(&self, scope: &RiseMessageTabScope) -> Option<&RiseMessageTabPresentationBoxValue> {
        self.message_tab_boxes.get(scope).and_then(|tab_box| tab_box.value.as_ref())
    }

    fn message_scope_box(&mut self, scope: &RiseMessageScope) -> &mut RiseMessageScopePresentationBox {
        self.message_scope_boxes.entry(scope.clone()).or_default()
    }

    fn pinned_message_box(&mut self, chat_id: &str) -> &mut RisePinnedMessagesPresentationBox {
        self.pinned_message_boxes.entry(chat_id.to_owned()).or_default()
    }

    fn message_tab_box(&mut self, scope: &RiseMessageTabScope) -> &mut RiseMessageTabPresentationBox {
        if !self.message_tab_boxes.contains_key(scope) {
            self.message_tab_boxes.insert(scope.clone(), RiseMessageTabPresentationBox::default());
            self.touch_message_tab_box(scope);
        }
        self.message_tab_boxes.entry(scope.clone()).or_default()
    }

    fn touch_message_tab_box(&mut self, scope: &RiseMessageTabScope) {
        self.message_tab_box_order.retain(|candidate| candidate != scope);
        self.message_tab_box_order.push(scope.clone());
        while self.message_tab_box_order.len() > Self::MESSAGE_TAB_BOX_LIMIT {
            let removed_scope = self.message_tab_box_order.remove(0);
            if let Some(mut tab_box) = self.message_tab_boxes.remove(&removed_scope) {
                tab_box.invalidate();
            }
        }
    }

    fn install_prepared_message_tab(
        &mut self,
        scope: &RiseMessageTabScope,
        prepared: RiseMessageTabPresentationBoxValue,
        base: RiseMessageTabPresentationBoxValue,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_box) = self.message_tab_boxes.get_mut(scope) else { return };
        if tab_box.install_prepared(prepared, base, generation) {
            Self::changed(RiseConversationPresentationChange::MessageTabs, cx);
        }
    }

    fn global_message_search_box(
        &mut self,
        scope: &RiseGlobalMessageSearchScope,
    ) -> &mut RiseGlobalMessageSearchPresentationBox {
        if !self.global_message_search_boxes.contains_key(scope) {
            self.global_message_search_boxes.insert(scope.clone(), RiseGlobalMessageSearchPresentationBox::default());
            self.touch_global_message_search_box(scope);
        }
        self.global_message_search_boxes.entry(scope.clone()).or_default()
    }

    fn touch_global_message_search_box(&mut self, scope: &RiseGlobalMessageSearchScope) {
        self.global_message_search_box_order.retain(|candidate| candidate != scope);
        self.global_message_search_box_order.push(scope.clone());
        while self.global_message_search_box_order.len() > Self::GLOBAL_MESSAGE_SEARCH_BOX_LIMIT {
            let removed_scope = self.global_message_search_box_order.remove(0);
            self.global_message_search_boxes.remove(&removed_scope);
        }
    }

    fn message_search_box(&mut self, scope: &RiseMessageSearchScope) -> &mut RiseMessageSearchPresentationBox {
        if !self.message_search_boxes.contains_key(scope) {
            self.message_search_boxes.insert(scope.clone(), RiseMessageSearchPresentationBox::default());
            self.touch_message_search_box(scope);
        }
        self.message_search_boxes.entry(scope.clone()).or_default()
    }

    fn touch_message_search_box(&mut self, scope: &RiseMessageSearchScope) {
        self.message_search_box_order.retain(|candidate| candidate != scope);
        self.message_search_box_order.push(scope.clone());
        while self.message_search_box_order.len() > Self::MESSAGE_SEARCH_BOX_LIMIT {
            let removed_scope = self.message_search_box_order.remove(0);
            self.message_search_boxes.remove(&removed_scope);
        }
    }

    fn chat_overlay_search_box(
        &mut self,
        scope: &RiseChatOverlaySearchScope,
    ) -> &mut RiseChatOverlaySearchPresentationBox {
        if !self.chat_overlay_search_boxes.contains_key(scope) {
            self.chat_overlay_search_boxes.insert(scope.clone(), RiseChatOverlaySearchPresentationBox::default());
            self.touch_chat_overlay_search_box(scope);
        }
        self.chat_overlay_search_boxes.entry(scope.clone()).or_default()
    }

    fn touch_chat_overlay_search_box(&mut self, scope: &RiseChatOverlaySearchScope) {
        self.chat_overlay_search_box_order.retain(|candidate| candidate != scope);
        self.chat_overlay_search_box_order.push(scope.clone());
        while self.chat_overlay_search_box_order.len() > Self::CHAT_OVERLAY_SEARCH_BOX_LIMIT {
            let removed_scope = self.chat_overlay_search_box_order.remove(0);
            self.chat_overlay_search_boxes.remove(&removed_scope);
        }
    }

    fn clear_snapshots(&mut self) {
        self.inbox_chats.value = None;
        self.archived_chats.value = None;
        self.requested_chats.value = None;
        self.chat_preparation_generation = 0;
        self.standalone_chats_by_id = HashMap::new();
        self.favourites_chat_id = None;
        self.standalone_chat_order = Vec::new();
        self.standalone_archived_chat_ids = HashSet::new();
        self.confirmed_removed_chat_ids = HashSet::new();
        self.message_scope_boxes = HashMap::new();
        self.pinned_message_boxes = HashMap::new();
        self.unread_marks_boxes = HashMap::new();
        for tab_box in self.message_tab_boxes.values_mut() {
            tab_box.invalidate();
        }
        self.message_tab_boxes = HashMap::new();
        self.message_tab_box_order = Vec::new();
        self.global_message_search_boxes = HashMap::new();
        self.global_message_search_box_order = Vec::new();
        self.chat_overlay_search_boxes = HashMap::new();
        self.chat_overlay_search_box_order = Vec::new();
        self.message_search_boxes = HashMap::new();
        self.message_search_box_order = Vec::new();
        self.message_search_presentation_revision = self.message_search_presentation_revision.wrapping_add(1);
        self.chat_search_recents = Vec::new();

        self.chat_revision = self.chat_revision.wrapping_add(1);
        self.message_revision = self.message_revision.wrapping_add(1);
        self.chat_search_recents_revision = self.chat_search_recents_revision.wrapping_add(1);
    }
}
