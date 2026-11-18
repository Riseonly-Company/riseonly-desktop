use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::chat as key;
use rise_canon::{RiseEvent, RiseLimits, RiseWireKey};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_store::rise_store_entities::RiseEntityViewLoadState;
use rise_events::name;
use rise_ui::shared::try_read;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseChatListWireSnapshot, RiseConversationPresentation,
};
use crate::modules::chat::shared::utils::chat_list_utils::ChatListUtils;
use crate::modules::chat::stores::chats::chat_actions::chat_archive_types::{
    ChatArchiveCursorPolicy, ChatArchiveMutationFailurePolicy, ChatArchiveMutationSnapshot,
    ChatArchiveOptimisticRowPolicy, ChatArchiveRealtimeResolution,
};
use crate::modules::chat::stores::chats::chat_actions::chat_archive_wire_types::RiseStoredArchivedChatPage;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatMuteScopes};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessagePayloadCodec;

fn active_presentation(cx: &App) -> Option<&RiseConversationPresentation> {
    try_read::<RiseConversationPresentation>(cx).filter(|presentation| presentation.is_active())
}

fn archive_state(cx: &App) -> Option<&RiseChatListWireSnapshot> {
    active_presentation(cx)?.chat_state(RiseChatListKind::Archive)
}

fn encoded_row<T: Serialize>(value: &T) -> Option<Map<String, Value>> {
    match serde_json::to_value(value) {
        Ok(Value::Object(row)) => Some(row),
        _ => None,
    }
}

impl ChatServicesStore {
    pub fn archived_chats_limit() -> i64 {
        RiseLimits::client_value("chat_services_store_archived_chats_limit")
    }

    pub fn archived_chats_cache_key_for_user(user_id: &str) -> String {
        SaiCache::new(["getArchivedChats", user_id]).key
    }

    pub fn archived_chats_cache_key(&self, cx: &App) -> Option<String> {
        let user_id = try_read::<AuthServicesStore>(cx)
            .and_then(AuthServicesStore::effective_user_id_for_wire)
            .or_else(SaiAccountCacheScope::active_user_id)?;
        if user_id.is_empty() {
            return None;
        }

        Some(Self::archived_chats_cache_key_for_user(&user_id))
    }

    pub fn preload_archived_chats_slice_from_disk_if_needed(&self, cx: &mut App) {
        let Some(cache_key) = self.archived_chats_cache_key(cx) else {
            return;
        };

        let slice = self.get_archived_chats_sai.clone();
        SaiUpdater::hydrate_slice_from_disk(&slice, &cache_key, None, cx);
    }

    pub fn archived_chats(&self, cx: &mut App) -> Vec<ChatInfo> {
        if let Some(presentation) = active_presentation(cx) {
            return presentation.chats(RiseChatListKind::Archive).to_vec();
        }
        self.archived_chats_dto(cx)
            .and_then(|dto| dto.chats)
            .unwrap_or_default()
            .iter()
            .map(|chat| chat.to_chat_info())
            .collect()
    }

    pub fn archived_chats_has_more(&self, cx: &mut App) -> bool {
        if let Some(state) = archive_state(cx) {
            return state.has_more;
        }
        self.archived_chats_dto(cx).and_then(|dto| dto.is_have_more).unwrap_or(false)
    }

    pub fn archived_chats_relative_id(&self, cx: &mut App) -> Option<String> {
        if let Some(state) = archive_state(cx) {
            return state.cursor.clone();
        }
        self.archived_chats_dto(cx).and_then(|dto| dto.relative_id)
    }

    pub fn archived_chats_total_count(&self, cx: &mut App) -> i64 {
        let count = self.archived_chats(cx).len() as i64;
        if let Some(state) = archive_state(cx) {
            return count.max(state.server_total_count.unwrap_or(count));
        }
        count.max(self.archived_chats_dto(cx).and_then(|dto| dto.total_count).unwrap_or(count))
    }

    pub fn archived_chats_total_unread_count(&self, cx: &mut App) -> i64 {
        let loaded_unread: i64 = self.archived_chats(cx).iter().map(|chat| chat.unread_count.max(0)).sum();
        if let Some(state) = archive_state(cx) {
            return loaded_unread.max(state.server_total_unread_count.unwrap_or(loaded_unread));
        }
        loaded_unread.max(self.archived_chats_dto(cx).and_then(|dto| dto.total_unread_count).unwrap_or(loaded_unread))
    }

    pub fn archived_chats_unread_chats_count(&self, cx: &mut App) -> i64 {
        let loaded_unread_chats = self.archived_chats(cx).iter().filter(|chat| chat.unread_count > 0).count() as i64;
        if let Some(state) = archive_state(cx) {
            return loaded_unread_chats.max(state.server_unread_chats_count.unwrap_or(loaded_unread_chats));
        }
        loaded_unread_chats
            .max(self.archived_chats_dto(cx).and_then(|dto| dto.unread_chats_count).unwrap_or(loaded_unread_chats))
    }

    pub fn contains_archived_chat(&self, chat_id: &str, cx: &mut App) -> bool {
        if let Some(presentation) = active_presentation(cx) {
            return presentation.is_chat_archived(chat_id);
        }
        self.raw_chat_row(chat_id, true, cx).is_some()
    }

    pub fn archived_chat_membership_is_fully_resolved(&self, cx: &mut App) -> bool {
        if let Some(state) = archive_state(cx) {
            let Some(total_count) = state.server_total_count else {
                return false;
            };
            if state.load_state != RiseEntityViewLoadState::Loaded || state.has_more {
                return false;
            }
            return state.rows.len() as i64 >= total_count.max(0);
        }
        let Some(dto) = self.archived_chats_dto(cx) else {
            return false;
        };
        let Some(total_count) = dto.total_count else {
            return false;
        };
        if dto.success == Some(false) || dto.is_have_more != Some(false) {
            return false;
        }

        self.archived_chats(cx).len() as i64 >= total_count.max(0)
    }

    pub fn chat_info(&self, chat_id: &str, cx: &App) -> Option<ChatInfo> {
        if let Some(draft) = self.draft_direct_chat_by_id.get(chat_id) {
            return Some(draft.clone());
        }
        if let Some(presentation) = active_presentation(cx) {
            return presentation.chat(chat_id).cloned();
        }
        self.rebuild_chat_info_index_if_needed(cx);
        if let Some(chat) = self.chat_info_index.borrow().get(chat_id) {
            return Some(chat.clone());
        }
        if let Some(preview) = self.community_preview_by_chat_id.get(chat_id) {
            return Some(preview.clone());
        }
        self.get_chat_by_id_sai
            .data()
            .and_then(|data| data.chat)
            .map(|chat| chat.to_chat_info())
            .filter(|chat| chat.id == chat_id)
    }

    pub fn chat_topics_enabled(&self, chat_id: &str, cx: &App) -> bool {
        if let Some(draft) = self.draft_direct_chat_by_id.get(chat_id) {
            return draft.topics_enabled;
        }
        if let Some(presentation) = active_presentation(cx) {
            return presentation.chat(chat_id).is_some_and(|chat| chat.topics_enabled);
        }
        self.rebuild_chat_info_index_if_needed(cx);
        if let Some(chat) = self.chat_info_index.borrow().get(chat_id) {
            return chat.topics_enabled;
        }
        self.community_preview_by_chat_id.get(chat_id).is_some_and(|preview| preview.topics_enabled)
    }

    pub fn is_archive_mutation_pending(&self, chat_id: &str) -> bool {
        self.archive_mutations_by_chat.contains_key(chat_id)
    }

    pub fn apply_archive_mutation_optimistic(
        &mut self,
        chat: &ChatInfo,
        moves_to_archive: bool,
        cx: &mut Context<Self>,
    ) -> Option<Uuid> {
        if self.archive_mutations_by_chat.contains_key(&chat.id) {
            return None;
        }

        let active_row = self.raw_chat_row(&chat.id, false, cx);
        let archived_row = self.raw_chat_row(&chat.id, true, cx);
        if moves_to_archive == archived_row.is_some() {
            return None;
        }

        let fallback_row = encoded_row(&chat.to_wire_dto(None))?;
        let source_row = if moves_to_archive { active_row.clone().unwrap_or(fallback_row) } else { archived_row.clone()? };

        let optimistic_row = ChatArchiveOptimisticRowPolicy::destination_row(&source_row, moves_to_archive);
        let unread_count = Self::raw_unread_count(&source_row);
        let token = Uuid::new_v4();
        let source_was_cached = if moves_to_archive { active_row.is_some() } else { archived_row.is_some() };
        let source_needs_refresh_after_commit =
            self.chat_list_requires_refresh_after_removing(&chat.id, !moves_to_archive, cx);
        let snapshot = ChatArchiveMutationSnapshot {
            token,
            chat_id: chat.id.clone(),
            moves_to_archive,
            source_row,
            optimistic_row: optimistic_row.clone(),
            source_was_cached,
            destination_row: if moves_to_archive { archived_row } else { active_row },
            archived_count_delta: if moves_to_archive { 1 } else { -1 },
            archived_unread_delta: if moves_to_archive { unread_count } else { -unread_count },
            archived_unread_chats_delta: match (unread_count > 0, moves_to_archive) {
                (false, _) => 0,
                (true, true) => 1,
                (true, false) => -1,
            },
            source_needs_refresh_after_commit,
            confirmed_by_realtime: false,
        };
        self.archive_mutations_by_chat.insert(chat.id.clone(), snapshot.clone());
        cx.notify();

        if !self.move_raw_chat(
            &chat.id,
            optimistic_row,
            moves_to_archive,
            snapshot.archived_count_delta,
            snapshot.archived_unread_delta,
            snapshot.archived_unread_chats_delta,
            cx,
        ) {
            self.archive_mutations_by_chat.remove(&chat.id);
            self.restore_archive_snapshot(&snapshot, cx);
            return None;
        }

        Some(token)
    }

    pub fn complete_archive_mutation(
        &mut self,
        chat_id: &str,
        token: Uuid,
        moves_to_archive: bool,
        chat: Option<&RiseStoredChat>,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.archive_mutations_by_chat.get(chat_id).filter(|snapshot| snapshot.token == token)
        else {
            return;
        };
        let source_needs_refresh = snapshot.source_needs_refresh_after_commit;

        self.archive_mutations_by_chat.remove(chat_id);
        cx.notify();
        if let Some(row) = chat.and_then(encoded_row) {
            self.replace_raw_chat(chat_id, row, moves_to_archive, cx);
        }
        if source_needs_refresh {
            self.refresh_chat_lists_after_archive_event(cx);
        }
    }

    pub fn revert_archive_mutation(&mut self, chat_id: &str, token: Uuid, cx: &mut Context<Self>) -> bool {
        let Some(snapshot) =
            self.archive_mutations_by_chat.get(chat_id).filter(|snapshot| snapshot.token == token).cloned()
        else {
            return false;
        };

        self.archive_mutations_by_chat.remove(chat_id);
        cx.notify();
        if !ChatArchiveMutationFailurePolicy::should_revert(snapshot.confirmed_by_realtime) {
            self.refresh_chat_lists_after_archive_event(cx);
            return false;
        }
        self.restore_archive_snapshot(&snapshot, cx);
        true
    }

    pub fn reconcile_chat_archives_after_fetch(&mut self, cx: &mut Context<Self>) {
        let archived_ids: HashSet<String> = self.archived_chats(cx).into_iter().map(|chat| chat.id).collect();
        if !archived_ids.is_empty() {
            self.remove_raw_chats(&archived_ids, false, cx);
        }

        let snapshots: Vec<ChatArchiveMutationSnapshot> = self.archive_mutations_by_chat.values().cloned().collect();
        for snapshot in snapshots {
            if self.raw_chat_row(&snapshot.chat_id, snapshot.moves_to_archive, cx).is_some() {
                continue;
            }

            let _ = self.move_raw_chat(
                &snapshot.chat_id,
                snapshot.optimistic_row.clone(),
                snapshot.moves_to_archive,
                snapshot.archived_count_delta,
                snapshot.archived_unread_delta,
                snapshot.archived_unread_chats_delta,
                cx,
            );
        }
    }

    pub fn upsert_archived_chat(&mut self, chat: &ChatInfo, allowing_insert: bool, cx: &mut Context<Self>) {
        let Some(row) = encoded_row(&chat.to_wire_dto(None)) else {
            return;
        };
        if self.contains_archived_chat(&chat.id, cx) {
            self.replace_raw_chat(&chat.id, row, true, cx);
            return;
        }
        if !allowing_insert {
            return;
        }

        let unread_count = chat.unread_count.max(0);
        let _ = self.move_raw_chat(&chat.id, row, true, 1, unread_count, i64::from(unread_count > 0), cx);
    }

    pub fn remove_archived_chat_local(&mut self, chat_id: &str, cx: &mut Context<Self>) -> bool {
        if !self.contains_archived_chat(chat_id, cx) {
            return false;
        }
        self.archive_mutations_by_chat.remove(chat_id);
        self.remove_raw_chats(&HashSet::from([chat_id.to_owned()]), true, cx);
        cx.notify();
        true
    }

    pub fn restore_archived_chat_local(&mut self, chat: &ChatInfo, cx: &mut Context<Self>) {
        if self.contains_archived_chat(&chat.id, cx) {
            return;
        }
        let Some(row) = encoded_row(&chat.to_wire_dto(None)) else {
            return;
        };

        let unread_count = chat.unread_count.max(0);
        let mut rows = self.raw_chat_rows(true, cx);
        rows.push(row);
        let _ = self.write_chat_rows(
            rows,
            true,
            true,
            1,
            unread_count,
            i64::from(unread_count > 0),
            &HashSet::new(),
            cx,
        );
    }

    pub fn update_archived_chat_from_realtime(
        &mut self,
        chat_id: &str,
        payload: &Map<String, Value>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.mutate_archived_chat_row(
            chat_id,
            |row| {
                if let Some(title) = payload.get(key::TITLE).and_then(Value::as_str)
                    && !title.is_empty()
                {
                    row.insert(key::TITLE.to_owned(), Value::from(title));
                }
                if let Some(description) = payload.get(key::DESCRIPTION).and_then(Value::as_str) {
                    row.insert(key::DESCRIPTION.to_owned(), Value::from(description));
                }
                if let Some(logo_url) = payload.get(key::LOGO_URL).and_then(Value::as_str) {
                    row.insert(key::LOGO_URL.to_owned(), Value::from(logo_url));
                }
            },
            cx,
        )
    }

    pub fn adjust_archived_chat_member_count(&mut self, chat_id: &str, delta: i64, cx: &mut Context<Self>) -> bool {
        self.mutate_archived_chat_row(
            chat_id,
            |row| {
                let count = Self::integer(row.get(key::MEMBER_COUNT)).unwrap_or(0);
                row.insert(key::MEMBER_COUNT.to_owned(), Value::from((count + delta).max(0)));
            },
            cx,
        )
    }

    pub fn set_archived_topics_enabled_local(&mut self, chat_id: &str, enabled: bool, cx: &mut Context<Self>) -> bool {
        self.mutate_archived_chat_row(
            chat_id,
            |row| {
                row.insert(key::TOPICS_ENABLED.to_owned(), Value::Bool(enabled));
            },
            cx,
        )
    }

    pub fn set_archived_mute_local(
        &mut self,
        chat_id: &str,
        muted: bool,
        until: Option<f64>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.mutate_archived_chat_row(
            chat_id,
            |row| {
                row.insert(key::IS_MUTED.to_owned(), Value::Bool(muted));
                let mute_until = match until {
                    Some(until) if muted => Value::from(until),
                    _ => Value::Null,
                };
                row.insert(key::MUTE_UNTIL.to_owned(), mute_until);
            },
            cx,
        )
    }

    pub fn set_archived_notification_scopes_local(
        &mut self,
        chat_id: &str,
        scopes: ChatMuteScopes,
        cx: &mut Context<Self>,
    ) -> bool {
        self.mutate_archived_chat_row(
            chat_id,
            |row| {
                row.insert(key::IS_MUTED.to_owned(), Value::Bool(scopes.is_fully_muted()));
                row.insert(key::NOTIFICATIONS_ENABLED.to_owned(), Value::Bool(!scopes.is_fully_muted()));
                if scopes.is_all_enabled() {
                    row.insert(key::MUTE_SCOPES.to_owned(), Value::Null);
                } else if let Some(encoded) = scopes.encoded_json() {
                    row.insert(key::MUTE_SCOPES.to_owned(), Value::from(encoded));
                } else {
                    row.remove(key::MUTE_SCOPES);
                }
            },
            cx,
        )
    }

    pub fn set_archived_allowed_reactions_local(
        &mut self,
        chat_id: &str,
        reactions: &[String],
        cx: &mut Context<Self>,
    ) -> bool {
        self.mutate_archived_chat_row(
            chat_id,
            |row| {
                row.insert(key::ALLOWED_REACTIONS.to_owned(), Value::from(reactions.to_vec()));
            },
            cx,
        )
    }

    pub fn chat_archived_event_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(data) = any else {
            return;
        };
        let Ok(RiseEvent::ChatArchived(event)) = RiseEvent::read(name::CHAT_ARCHIVED, data.clone()) else {
            return;
        };
        self.apply_chat_archive_event(&event.chat_id, event.is_archived, cx);
    }

    pub fn chat_unarchived_event_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(data) = any else {
            return;
        };
        let Ok(RiseEvent::ChatUnarchived(event)) = RiseEvent::read(name::CHAT_UNARCHIVED, data.clone()) else {
            return;
        };
        self.apply_chat_archive_event(&event.chat_id, event.is_archived, cx);
    }

    pub fn apply_message_created_to_archived_chat(
        &mut self,
        payload: &Map<String, Value>,
        chat_id: &str,
        sender_id: &str,
        current_user_id: &str,
        _message_topic_id: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut chats = self.archived_chats(cx);
        let Some(index) = chats.iter().position(|chat| chat.id == chat_id) else {
            return false;
        };

        let previous_unread = chats[index].unread_count.max(0);
        let is_incoming = !sender_id.is_empty() && sender_id != current_user_id;
        let message = ChatListUtils::map_inbound_message(payload);
        chats[index].updated_at = message.created_at;
        chats[index].last_message = Some(message);
        if let Some(live_voice) = Self::live_voice_change(payload) {
            chats[index].has_live_voice = live_voice;
        }
        if is_incoming {
            chats[index].unread_count = previous_unread + 1;
        }

        let total_count = self.archived_chats_total_count(cx);
        let total_unread_count = self.archived_chats_total_unread_count(cx) + i64::from(is_incoming);
        let unread_chats_count =
            self.archived_chats_unread_chats_count(cx) + i64::from(is_incoming && previous_unread == 0);
        self.set_archived_chats_payload(
            ChatListUtils::sort(&chats),
            total_count,
            total_unread_count,
            unread_chats_count,
            cx,
        );
        true
    }

    pub fn apply_message_edited_to_archived_chat(
        &mut self,
        payload: &Map<String, Value>,
        chat_id: &str,
        message_id: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut chats = self.archived_chats(cx);
        let Some(index) = chats.iter().position(|chat| chat.id == chat_id) else {
            return false;
        };
        if !chats[index].last_message.as_ref().is_some_and(|message| message.matches_message_id(message_id)) {
            return true;
        }

        chats[index].last_message = Some(ChatListUtils::map_inbound_message(payload));
        self.rewrite_archived_chats_keeping_totals(chats, cx);
        true
    }

    pub fn apply_message_deleted_to_archived_chat(
        &mut self,
        chat_id: &str,
        message_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut chats = self.archived_chats(cx);
        let Some(index) = chats.iter().position(|chat| chat.id == chat_id) else {
            return false;
        };
        let Some(message_id) = message_id else {
            return true;
        };
        if !chats[index].last_message.as_ref().is_some_and(|message| message.matches_message_id(message_id)) {
            return true;
        }

        chats[index].last_message = None;
        self.rewrite_archived_chats_keeping_totals(chats, cx);
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mark_archived_last_message_read_local(
        &mut self,
        chat_id: &str,
        sender_id: &str,
        topic_id: Option<&str>,
        cutoff: f64,
        same_millisecond_message_ids: Option<&HashSet<String>>,
        read_message_ids: Option<&HashSet<String>>,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut chats = self.archived_chats(cx);
        let Some(index) = chats.iter().position(|chat| chat.id == chat_id) else {
            return false;
        };
        let Some(mut message) = chats[index].last_message.clone() else {
            return true;
        };
        if !Self::message_matches_read_receipt(
            &message,
            sender_id,
            topic_id,
            cutoff,
            same_millisecond_message_ids,
            read_message_ids,
        ) {
            return true;
        }

        message.is_read = true;
        chats[index].last_message = Some(message);
        self.rewrite_archived_chats_keeping_totals(chats, cx);
        true
    }

    fn rewrite_archived_chats_keeping_totals(&mut self, chats: Vec<ChatInfo>, cx: &mut Context<Self>) {
        let total_count = self.archived_chats_total_count(cx);
        let total_unread_count = self.archived_chats_total_unread_count(cx);
        let unread_chats_count = self.archived_chats_unread_chats_count(cx);
        self.set_archived_chats_payload(chats, total_count, total_unread_count, unread_chats_count, cx);
    }

    pub(crate) fn archived_chats_memory_only(&self, cx: &App) -> Vec<ChatInfo> {
        if let Some(presentation) = active_presentation(cx) {
            return presentation.chats(RiseChatListKind::Archive).to_vec();
        }
        let payload = self
            .archived_chats_cache_key(cx)
            .and_then(|cache_key| self.memory_or_disk_payload(&cache_key, cx));
        let dto: Option<RiseStoredArchivedChatPage> = match payload {
            Some(payload) => serde_json::from_value(payload).ok(),
            None => self.get_archived_chats_sai.data(),
        };
        dto.and_then(|dto| dto.chats).unwrap_or_default().iter().map(|chat| chat.to_chat_info()).collect()
    }

    fn archived_chats_dto(&self, cx: &mut App) -> Option<RiseStoredArchivedChatPage> {
        let payload = self.archived_chats_cache_key(cx).and_then(|cache_key| {
            SaiWsResponseCache::observed_payload(
                &cache_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
        });
        match payload {
            Some(payload) => serde_json::from_value(payload).ok(),
            None => self.get_archived_chats_sai.data(),
        }
    }

    fn set_archived_chats_payload(
        &mut self,
        items: Vec<ChatInfo>,
        total_count: i64,
        total_unread_count: i64,
        unread_chats_count: i64,
        cx: &mut Context<Self>,
    ) {
        let previous = self.archived_chats_dto(cx);
        let previous_chats = previous.as_ref().and_then(|dto| dto.chats.as_deref()).unwrap_or_default();
        let existing_by_id: HashMap<&str, &RiseStoredChat> =
            previous_chats.iter().map(|chat| (chat.id.as_str(), chat)).collect();
        let dto = RiseStoredArchivedChatPage::new(
            Some(items.iter().map(|chat| chat.to_wire_dto(existing_by_id.get(chat.id.as_str()).copied())).collect()),
            previous.as_ref().and_then(|dto| dto.relative_id.clone()),
            Some(previous.as_ref().and_then(|dto| dto.limit).unwrap_or_else(Self::archived_chats_limit)),
            Some(previous.as_ref().and_then(|dto| dto.is_have_more).unwrap_or(false)),
            Some((items.len() as i64).max(total_count)),
            Some(total_unread_count.max(0)),
            Some(unread_chats_count.max(0)),
            Some(true),
            None,
        );
        let Some(root) = encoded_row(&dto) else {
            return;
        };

        let _ = self.write_chat_list_root(root, true, cx);
    }

    fn apply_chat_archive_event(&mut self, chat_id: &str, moves_to_archive: bool, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }

        let pending_mutation = self.archive_mutations_by_chat.get(chat_id).cloned();
        let resolution = ChatArchiveRealtimeResolution::resolve(
            pending_mutation.as_ref().map(|mutation| mutation.moves_to_archive),
            moves_to_archive,
        );
        let source_needs_refresh = match resolution {
            ChatArchiveRealtimeResolution::ConfirmsPendingMutation => {
                pending_mutation.as_ref().is_some_and(|mutation| mutation.source_needs_refresh_after_commit)
            }
            ChatArchiveRealtimeResolution::NoPendingMutation => {
                self.chat_list_requires_refresh_after_removing(chat_id, !moves_to_archive, cx)
            }
            ChatArchiveRealtimeResolution::ConflictsWithPendingMutation => false,
        };
        if resolution == ChatArchiveRealtimeResolution::ConfirmsPendingMutation
            && let Some(mutation) = self.archive_mutations_by_chat.get_mut(chat_id)
        {
            mutation.confirmed_by_realtime = true;
        }
        if resolution.discards_pending_mutation() {
            self.archive_mutations_by_chat.remove(chat_id);
        }
        cx.notify();

        if self.raw_chat_row(chat_id, moves_to_archive, cx).is_some() {
            self.remove_raw_chats(&HashSet::from([chat_id.to_owned()]), !moves_to_archive, cx);
            if resolution.discards_pending_mutation() || source_needs_refresh {
                self.refresh_chat_lists_after_archive_event(cx);
            }
            return;
        }

        let source_row = self.raw_chat_row(chat_id, !moves_to_archive, cx).or_else(|| {
            if resolution == ChatArchiveRealtimeResolution::NoPendingMutation {
                None
            } else {
                pending_mutation.as_ref().map(|mutation| mutation.optimistic_row.clone())
            }
        });
        let Some(source_row) = source_row else {
            self.refresh_chat_lists_after_archive_event(cx);
            return;
        };

        let unread_count = Self::raw_unread_count(&source_row);
        let did_move = self.move_raw_chat(
            chat_id,
            source_row,
            moves_to_archive,
            if moves_to_archive { 1 } else { -1 },
            if moves_to_archive { unread_count } else { -unread_count },
            match (unread_count > 0, moves_to_archive) {
                (false, _) => 0,
                (true, true) => 1,
                (true, false) => -1,
            },
            cx,
        );
        if !did_move || resolution.discards_pending_mutation() || source_needs_refresh {
            self.refresh_chat_lists_after_archive_event(cx);
        }
    }

    pub fn refresh_chat_lists_after_archive_event(&self, cx: &mut App) {
        cx.defer(|cx| {
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.refresh_chats(cx);
                store.refresh_archived_chats(cx);
            });
        });
    }

    fn restore_archive_snapshot(&mut self, snapshot: &ChatArchiveMutationSnapshot, cx: &mut Context<Self>) {
        let mut active_rows = self.raw_chat_rows(false, cx);
        let mut archived_rows = self.raw_chat_rows(true, cx);
        active_rows.retain(|row| Self::raw_chat_id(row) != snapshot.chat_id);
        archived_rows.retain(|row| Self::raw_chat_id(row) != snapshot.chat_id);

        if snapshot.source_was_cached {
            if snapshot.moves_to_archive {
                active_rows.push(snapshot.source_row.clone());
            } else {
                archived_rows.push(snapshot.source_row.clone());
            }
        }
        if let Some(destination_row) = &snapshot.destination_row {
            if snapshot.moves_to_archive {
                archived_rows.push(destination_row.clone());
            } else {
                active_rows.push(destination_row.clone());
            }
        }

        let no_removals = HashSet::new();
        let active_root_exists = self.raw_chat_list_root(false, cx).is_some();
        if active_root_exists || !active_rows.is_empty() {
            let create_if_missing = !active_rows.is_empty();
            let _ = self.write_chat_rows(active_rows, false, create_if_missing, 0, 0, 0, &no_removals, cx);
        }
        let create_if_missing = !archived_rows.is_empty();
        let _ = self.write_chat_rows(
            archived_rows,
            true,
            create_if_missing,
            -snapshot.archived_count_delta,
            -snapshot.archived_unread_delta,
            -snapshot.archived_unread_chats_delta,
            &no_removals,
            cx,
        );
    }

    fn mutate_archived_chat_row(
        &mut self,
        chat_id: &str,
        mutation: impl FnOnce(&mut Map<String, Value>),
        cx: &mut Context<Self>,
    ) -> bool {
        let mut rows = self.raw_chat_rows(true, cx);
        let Some(index) = rows.iter().position(|row| Self::raw_chat_id(row) == chat_id) else {
            return false;
        };

        mutation(&mut rows[index]);
        let _ = self.write_chat_rows(rows, true, false, 0, 0, 0, &HashSet::new(), cx);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn move_raw_chat(
        &mut self,
        chat_id: &str,
        row: Map<String, Value>,
        moves_to_archive: bool,
        archived_count_delta: i64,
        archived_unread_delta: i64,
        archived_unread_chats_delta: i64,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut active_rows = self.raw_chat_rows(false, cx);
        let mut archived_rows = self.raw_chat_rows(true, cx);
        let active_root_exists = self.raw_chat_list_root(false, cx).is_some();
        let archived_root_exists = self.raw_chat_list_root(true, cx).is_some();
        active_rows.retain(|row| Self::raw_chat_id(row) != chat_id);
        archived_rows.retain(|row| Self::raw_chat_id(row) != chat_id);
        let removed_chat_ids = HashSet::from([chat_id.to_owned()]);
        let no_removals = HashSet::new();

        if moves_to_archive {
            archived_rows.push(row);
            let destination_write_succeeded = self.write_chat_rows(
                archived_rows,
                true,
                true,
                archived_count_delta,
                archived_unread_delta,
                archived_unread_chats_delta,
                &no_removals,
                cx,
            );
            let source_write_succeeded = !active_root_exists
                || self.write_chat_rows(active_rows, false, false, 0, 0, 0, &removed_chat_ids, cx);
            destination_write_succeeded && source_write_succeeded
        } else {
            active_rows.push(row);
            let destination_write_succeeded =
                self.write_chat_rows(active_rows, false, true, 0, 0, 0, &no_removals, cx);
            let source_write_succeeded = !archived_root_exists
                || self.write_chat_rows(
                    archived_rows,
                    true,
                    false,
                    archived_count_delta,
                    archived_unread_delta,
                    archived_unread_chats_delta,
                    &removed_chat_ids,
                    cx,
                );
            destination_write_succeeded && source_write_succeeded
        }
    }

    fn replace_raw_chat(&mut self, chat_id: &str, row: Map<String, Value>, archived: bool, cx: &mut Context<Self>) {
        let mut rows = self.raw_chat_rows(archived, cx);
        let Some(index) = rows.iter().position(|existing| Self::raw_chat_id(existing) == chat_id) else {
            return;
        };

        rows[index] = row;
        let _ = self.write_chat_rows(rows, archived, false, 0, 0, 0, &HashSet::new(), cx);
    }

    fn remove_raw_chats(&mut self, chat_ids: &HashSet<String>, archived: bool, cx: &mut Context<Self>) {
        if chat_ids.is_empty() || self.raw_chat_list_root(archived, cx).is_none() {
            return;
        }

        let (removed_rows, remaining_rows): (Vec<_>, Vec<_>) = self
            .raw_chat_rows(archived, cx)
            .into_iter()
            .partition(|row| chat_ids.contains(Self::raw_chat_id(row)));
        if removed_rows.is_empty() {
            return;
        }

        let unread_delta = -removed_rows.iter().map(Self::raw_unread_count).sum::<i64>();
        let unread_chats_delta = -(removed_rows.iter().filter(|row| Self::raw_unread_count(row) > 0).count() as i64);
        let _ = self.write_chat_rows(
            remaining_rows,
            archived,
            false,
            if archived { -(removed_rows.len() as i64) } else { 0 },
            if archived { unread_delta } else { 0 },
            if archived { unread_chats_delta } else { 0 },
            chat_ids,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn write_chat_rows(
        &mut self,
        rows: Vec<Map<String, Value>>,
        archived: bool,
        create_if_missing: bool,
        total_count_delta: i64,
        total_unread_delta: i64,
        unread_chats_delta: i64,
        removed_chat_ids: &HashSet<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let root = self.raw_chat_list_root(archived, cx);
        if root.is_none() && !create_if_missing {
            return true;
        }

        let previous_rows =
            MessagePayloadCodec::array_of_dictionaries(root.as_ref().and_then(|root| root.get(key::CHATS)));
        let mut updated_root = root.unwrap_or_else(|| {
            let limit = if archived { Self::archived_chats_limit() } else { Self::chats_limit() };
            let mut root = Map::new();
            root.insert(key::CHATS.to_owned(), Value::Array(Vec::new()));
            root.insert(key::LIMIT.to_owned(), Value::from(limit));
            root.insert(key::IS_HAVE_MORE.to_owned(), Value::Bool(false));
            root
        });
        let sorted_rows = Self::stable_sorted_raw_chats(rows);
        updated_root.insert(
            key::CHATS.to_owned(),
            Value::Array(sorted_rows.iter().cloned().map(Value::Object).collect()),
        );
        if !removed_chat_ids.is_empty() {
            let camel_relative_id = RiseWireKey::camel(key::RELATIVE_ID);
            let current_cursor = updated_root
                .get(key::RELATIVE_ID)
                .and_then(Value::as_str)
                .or_else(|| updated_root.get(&camel_relative_id).and_then(Value::as_str))
                .map(str::to_owned);
            let remaining_chat_ids: Vec<String> =
                sorted_rows.iter().map(|row| Self::raw_chat_id(row).to_owned()).collect();
            let repaired_cursor = ChatArchiveCursorPolicy::repaired_cursor(
                current_cursor.as_deref(),
                removed_chat_ids,
                &remaining_chat_ids,
            );
            if repaired_cursor != current_cursor {
                updated_root.insert(key::RELATIVE_ID.to_owned(), repaired_cursor.map_or(Value::Null, Value::from));
                updated_root.remove(&camel_relative_id);
            }
        }
        if archived {
            let previous_total = Self::integer(updated_root.get(key::TOTAL_COUNT)).unwrap_or(previous_rows.len() as i64);
            let previous_unread = Self::integer(updated_root.get(key::TOTAL_UNREAD_COUNT))
                .unwrap_or_else(|| previous_rows.iter().map(Self::raw_unread_count).sum());
            let previous_unread_chats = Self::integer(updated_root.get(key::UNREAD_CHATS_COUNT))
                .unwrap_or_else(|| previous_rows.iter().filter(|row| Self::raw_unread_count(row) > 0).count() as i64);
            let loaded_unread: i64 = sorted_rows.iter().map(Self::raw_unread_count).sum();
            let loaded_unread_chats = sorted_rows.iter().filter(|row| Self::raw_unread_count(row) > 0).count() as i64;
            updated_root.insert(
                key::TOTAL_COUNT.to_owned(),
                Value::from((sorted_rows.len() as i64).max(previous_total + total_count_delta)),
            );
            updated_root.insert(
                key::TOTAL_UNREAD_COUNT.to_owned(),
                Value::from(loaded_unread.max(previous_unread + total_unread_delta)),
            );
            updated_root.insert(
                key::UNREAD_CHATS_COUNT.to_owned(),
                Value::from(loaded_unread_chats.max(previous_unread_chats + unread_chats_delta)),
            );
        }

        self.write_chat_list_root(updated_root, archived, cx)
    }

    fn write_chat_list_root(&mut self, root: Map<String, Value>, archived: bool, cx: &mut Context<Self>) -> bool {
        let cache_key = if archived { self.archived_chats_cache_key(cx) } else { self.chats_cache_key(cx) };
        let Some(cache_key) = cache_key else {
            return false;
        };

        let slice: Rc<dyn SaiSliceState> =
            if archived { self.get_archived_chats_sai.clone() } else { self.get_chats_sai.clone() };
        SaiWsResponseCache::note_active_slice(Some(slice), Some(cache_key.as_str()), None, cx);
        if SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::Cache, cx).is_err() {
            return false;
        }

        if archived {
            Self::persist_archived_chats_cache(&cache_key, cx);
        } else {
            self.mark_chats_cache_for_disk_persistence(cx);
        }
        cx.notify();
        true
    }

    fn persist_archived_chats_cache(cache_key: &str, cx: &mut App) {
        let Some(payload) = SaiWsResponseCache::observed_memory_only(cache_key, cx) else {
            return;
        };

        SaiWsDiskCache::save_asynchronously(cache_key.to_owned(), payload, |_, _| {}, cx);
    }

    fn raw_chat_list_root(&self, archived: bool, cx: &mut App) -> Option<Map<String, Value>> {
        let cache_key = if archived { self.archived_chats_cache_key(cx) } else { self.chats_cache_key(cx) }?;
        let payload = SaiWsResponseCache::observed_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )?;

        AnyDictionaryBridge::dictionary(Some(&payload))
    }

    fn raw_chat_rows(&self, archived: bool, cx: &mut App) -> Vec<Map<String, Value>> {
        let root = self.raw_chat_list_root(archived, cx);
        MessagePayloadCodec::array_of_dictionaries(root.as_ref().and_then(|root| root.get(key::CHATS)))
    }

    fn raw_chat_row(&self, chat_id: &str, archived: bool, cx: &mut App) -> Option<Map<String, Value>> {
        self.raw_chat_rows(archived, cx).into_iter().find(|row| Self::raw_chat_id(row) == chat_id)
    }

    fn raw_chat_id(row: &Map<String, Value>) -> &str {
        row.get(key::ID).and_then(Value::as_str).unwrap_or_default()
    }

    fn raw_unread_count(row: &Map<String, Value>) -> i64 {
        Self::integer(row.get(key::UNREAD_COUNT)).unwrap_or(0).max(0)
    }

    pub fn chat_list_requires_refresh_after_removing(&self, chat_id: &str, archived: bool, cx: &mut App) -> bool {
        let Some(root) = self.raw_chat_list_root(archived, cx) else {
            return false;
        };
        let has_more = root
            .get(key::IS_HAVE_MORE)
            .and_then(Value::as_bool)
            .or_else(|| root.get(&RiseWireKey::camel(key::IS_HAVE_MORE)).and_then(Value::as_bool))
            .unwrap_or(false);
        if !has_more {
            return false;
        }
        let rows = MessagePayloadCodec::array_of_dictionaries(root.get(key::CHATS));
        rows.len() == 1 && Self::raw_chat_id(&rows[0]) == chat_id
    }

    fn integer(value: Option<&Value>) -> Option<i64> {
        let value = value?;
        value.as_i64().or_else(|| value.as_f64().map(|number| number as i64))
    }

    fn stable_sorted_raw_chats(rows: Vec<Map<String, Value>>) -> Vec<Map<String, Value>> {
        let mut decorated: Vec<(usize, Option<ChatInfo>, Map<String, Value>)> = rows
            .into_iter()
            .enumerate()
            .map(|(offset, row)| {
                let info = serde_json::from_value::<RiseStoredChat>(Value::Object(row.clone()))
                    .ok()
                    .map(|chat| chat.to_chat_info());
                (offset, info, row)
            })
            .collect();
        decorated.sort_by(|lhs, rhs| match (&lhs.1, &rhs.1) {
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => lhs.0.cmp(&rhs.0),
            (Some(left), Some(right)) => {
                if left.is_pinned != right.is_pinned {
                    return if left.is_pinned { Ordering::Less } else { Ordering::Greater };
                }
                if left.is_pinned && right.is_pinned {
                    let left_position = left.pin_position.unwrap_or(i64::MAX);
                    let right_position = right.pin_position.unwrap_or(i64::MAX);
                    if left_position != right_position {
                        return left_position.cmp(&right_position);
                    }
                }
                let left_activity = ChatListUtils::last_activity(left);
                let right_activity = ChatListUtils::last_activity(right);
                if left_activity != right_activity {
                    return right_activity.total_cmp(&left_activity);
                }
                lhs.0.cmp(&rhs.0)
            }
        });
        decorated.into_iter().map(|(_, _, row)| row).collect()
    }
}
