use std::collections::{HashMap, HashSet};

use gpui::{App, Context};
use rise_canon::rise_key;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::shared::try_read;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::core::rise_undeliverable_direct_draft_loader::{
    RiseUndeliverableDirectDraftLoader, RiseUndeliverableDirectDraftLoaderEntry,
};
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_user_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, User};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageDeliveryStatus,
};

const PEER_NAME: &str = "peer_name";
const PEER_TAG: &str = "peer_tag";
const PEER_LOGO: &str = "peer_logo";

fn is_contact_join_virtual_chat_id(id: &str) -> bool {
    chat_stores_user_hooks().is_some_and(|hooks| (hooks.is_contact_join_virtual_chat_id)(id))
}

impl ChatServicesStore {
    pub const DRAFT_DIRECT_CHAT_PREFIX: &'static str = "temp_direct_";

    pub const DRAFT_DIRECT_CHAT_LIMIT: usize = 32;

    const UNDELIVERABLE_DRAFTS_LOGICAL_KEY: &'static str = "undeliverableDirectDrafts";

    pub fn draft_direct_chat_id(peer_user_id: &str) -> String {
        let peer = trimming_whitespaces_and_newlines(peer_user_id);
        if peer.is_empty() {
            return String::new();
        }
        format!("{}{peer}", Self::DRAFT_DIRECT_CHAT_PREFIX)
    }

    pub fn is_draft_direct_chat_id(id: &str) -> bool {
        id.starts_with(Self::DRAFT_DIRECT_CHAT_PREFIX)
    }

    pub fn draft_direct_chat_peer_id(id: &str) -> String {
        id.strip_prefix(Self::DRAFT_DIRECT_CHAT_PREFIX).unwrap_or_default().to_owned()
    }

    pub fn draft_direct_chat(&self, id: &str) -> Option<ChatInfo> {
        self.draft_direct_chat_by_id.get(id).cloned()
    }

    pub fn register_draft_direct_chat(&mut self, participant: &User, cx: &mut Context<Self>) -> String {
        self.restore_undeliverable_draft_direct_chats(cx);
        let peer_id = trimming_whitespaces_and_newlines(&participant.id);
        let id = Self::draft_direct_chat_id(peer_id);
        if id.is_empty() {
            return String::new();
        }

        if let Some(previous) = self.temp_chat_id_to_real.get(&id)
            && (self.locally_removed_chat_ids().contains(previous)
                || try_read::<RiseConversationPresentation>(cx)
                    .is_some_and(|presentation| presentation.confirmed_removed_chat_ids().contains(previous)))
        {
            self.temp_chat_id_to_real.remove(&id);
        }

        let now = RiseClock::now().time_interval_since_1970();
        let mut chat = self.draft_direct_chat_by_id.get(&id).cloned().unwrap_or_else(|| ChatInfo::new(id.clone()));
        chat.id = id.clone();
        chat.r#type = ChatType::Direct;
        chat.title = participant.name.clone();
        chat.participant = participant.clone();
        chat.created_at = now;
        chat.updated_at = now;

        self.draft_direct_chat_by_id.insert(id.clone(), chat);
        self.draft_direct_chat_order.retain(|existing| existing != &id);
        self.draft_direct_chat_order.push(id.clone());
        while self.draft_direct_chat_order.len() > Self::DRAFT_DIRECT_CHAT_LIMIT {
            let evicted = self.draft_direct_chat_order.remove(0);
            self.draft_direct_chat_by_id.remove(&evicted);
            self.undeliverable_draft_direct_chat_ids.remove(&evicted);
            if self.undeliverable_drafts_restore_in_flight {
                self.undeliverable_drafts_restore_excluded_ids.insert(evicted);
            }
        }
        cx.notify();
        id
    }

    pub fn remove_draft_direct_chat(&mut self, id: &str, cx: &mut Context<Self>) {
        self.restore_undeliverable_draft_direct_chats(cx);
        if self.undeliverable_drafts_restore_in_flight {
            self.undeliverable_drafts_restore_excluded_ids.insert(id.to_owned());
            self.undeliverable_drafts_persistence_pending = true;
        }
        self.draft_direct_chat_ids_awaiting_connection.remove(id);
        if self.draft_direct_chat_by_id.remove(id).is_none() {
            return;
        }

        self.draft_direct_chat_order.retain(|existing| existing != id);
        if self.undeliverable_draft_direct_chat_ids.remove(id) {
            self.persist_undeliverable_draft_direct_chats(cx);
        }
        cx.notify();
    }

    pub fn mark_draft_direct_chat_undeliverable(&mut self, id: &str, cx: &mut Context<Self>) {
        self.restore_undeliverable_draft_direct_chats(cx);
        if !self.draft_direct_chat_by_id.contains_key(id) {
            return;
        }

        if self.undeliverable_draft_direct_chat_ids.insert(id.to_owned()) {
            self.persist_undeliverable_draft_direct_chats(cx);
            cx.notify();
        }
    }

    pub fn restore_undeliverable_draft_direct_chats(&mut self, cx: &mut Context<Self>) {
        let cache_key = SaiAccountCacheScope::scoped_key(Self::UNDELIVERABLE_DRAFTS_LOGICAL_KEY);
        if cache_key == Self::UNDELIVERABLE_DRAFTS_LOGICAL_KEY
            || self.restored_undeliverable_drafts_key.as_deref() == Some(cache_key.as_str())
        {
            return;
        }

        if let Some(operation) = self.undeliverable_drafts_restore_operation.take() {
            operation.cancel();
        }
        let token = Uuid::new_v4();
        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        self.undeliverable_drafts_restore_token = token;
        self.restored_undeliverable_drafts_key = Some(cache_key.clone());
        self.undeliverable_drafts_restore_in_flight = true;
        let this = cx.weak_entity();
        let expected_key = cache_key.clone();
        let operation = RiseUndeliverableDirectDraftLoader::load(
            cache_key,
            Self::DRAFT_DIRECT_CHAT_PREFIX.to_owned(),
            Self::DRAFT_DIRECT_CHAT_LIMIT as i64,
            move |entries: Vec<RiseUndeliverableDirectDraftLoaderEntry>, cx: &mut App| {
                let _ = this.update(cx, |store, cx| {
                    if store.undeliverable_drafts_restore_token != token
                        || store.restored_undeliverable_drafts_key.as_deref() != Some(expected_key.as_str())
                        || !SaiAccountCacheScope::is_current_account_epoch(
                            account_id.as_deref(),
                            account_epoch.as_deref(),
                        )
                    {
                        return;
                    }

                    store.install_undeliverable_draft_direct_chats(&entries);
                    store.undeliverable_drafts_restore_operation = None;
                    store.undeliverable_drafts_restore_in_flight = false;
                    store.undeliverable_drafts_restore_excluded_ids.clear();
                    if store.undeliverable_drafts_persistence_pending {
                        store.undeliverable_drafts_persistence_pending = false;
                        store.persist_undeliverable_draft_direct_chats(cx);
                    }
                    cx.notify();
                });
            },
            cx,
        );
        self.undeliverable_drafts_restore_operation = Some(operation);
    }

    fn install_undeliverable_draft_direct_chats(&mut self, entries: &[RiseUndeliverableDirectDraftLoaderEntry]) {
        let mut restored_ids: Vec<String> = Vec::new();
        for entry in entries {
            let id = &entry.id;
            if self.undeliverable_drafts_restore_excluded_ids.contains(id)
                || self.temp_chat_id_to_real.contains_key(id)
            {
                continue;
            }
            let existing = self.draft_direct_chat_by_id.get(id).cloned();
            if existing.is_none()
                && self.draft_direct_chat_order.len() + restored_ids.len() >= Self::DRAFT_DIRECT_CHAT_LIMIT
            {
                continue;
            }

            let mut participant = existing
                .as_ref()
                .map(|chat| chat.participant.clone())
                .unwrap_or_else(|| User { id: entry.peer_id.clone(), ..User::default() });
            if existing.is_none() {
                participant.name = entry.peer_name.clone();
                participant.tag = entry.peer_tag.clone();
                participant.more.logo = entry.peer_logo.clone();
            }

            let is_new = existing.is_none();
            let mut chat = existing.unwrap_or_else(|| ChatInfo::new(id.clone()));
            chat.r#type = ChatType::Direct;
            chat.title = participant.name.clone();
            chat.participant = participant;
            if chat.last_message.is_none() {
                chat.created_at = entry.created_at;
                chat.updated_at = entry.updated_at;
                chat.last_activity = Some(entry.updated_at);
            }
            if chat.last_message.is_none()
                && let Some(preview) = &entry.preview
            {
                let mut message = GetMessageMessage::new(String::new());
                message.chat_id = id.clone();
                message.sender_id = entry.sender_id.clone();
                message.content_type = MessageContentType::from_raw_value(&entry.content_type);
                message.text = preview.clone();
                message.created_at = entry.updated_at;
                message.delivery_status = MessageDeliveryStatus::Failed;
                chat.last_message = Some(message);
            }
            self.draft_direct_chat_by_id.insert(id.clone(), chat);
            self.undeliverable_draft_direct_chat_ids.insert(id.clone());
            if is_new {
                restored_ids.push(id.clone());
            }
        }
        if restored_ids.is_empty() {
            return;
        }

        restored_ids.append(&mut self.draft_direct_chat_order);
        self.draft_direct_chat_order = restored_ids;
    }

    fn persist_undeliverable_draft_direct_chats(&mut self, cx: &mut Context<Self>) {
        let cache_key = SaiAccountCacheScope::scoped_key(Self::UNDELIVERABLE_DRAFTS_LOGICAL_KEY);
        if cache_key == Self::UNDELIVERABLE_DRAFTS_LOGICAL_KEY {
            return;
        }
        if self.undeliverable_drafts_restore_in_flight {
            self.undeliverable_drafts_persistence_pending = true;
            return;
        }

        let entries: Vec<Value> = self
            .draft_direct_chat_order
            .iter()
            .filter_map(|id| {
                if !self.undeliverable_draft_direct_chat_ids.contains(id) {
                    return None;
                }
                let draft = self.draft_direct_chat_by_id.get(id)?;
                let mut entry = Map::new();
                entry.insert(rise_key::bot::ID.to_owned(), Value::from(id.as_str()));
                entry.insert(PEER_NAME.to_owned(), Value::from(draft.participant.name.as_str()));
                entry.insert(PEER_TAG.to_owned(), Value::from(draft.participant.tag.as_str()));
                entry.insert(PEER_LOGO.to_owned(), Value::from(draft.participant.more.logo.as_str()));
                entry.insert(rise_key::bot::CREATED_AT.to_owned(), Value::from(draft.created_at));
                entry.insert(rise_key::bot::UPDATED_AT.to_owned(), Value::from(draft.updated_at));
                if let Some(message) = &draft.last_message {
                    entry.insert(rise_key::music::PREVIEW.to_owned(), Value::from(message.text.as_str()));
                    entry.insert(
                        rise_key::bot::CONTENT_TYPE.to_owned(),
                        Value::from(message.content_type.raw_value()),
                    );
                    entry.insert(rise_key::bot::SENDER_ID.to_owned(), Value::from(message.sender_id.as_str()));
                }
                Some(Value::Object(entry))
            })
            .collect();
        SaiWsDiskCache::save_asynchronously(cache_key, Value::Array(entries), |_, _| {}, cx);
    }

    pub fn is_draft_direct_chat_awaiting_connection(&self, id: &str) -> bool {
        self.draft_direct_chat_ids_awaiting_connection.contains(id)
    }

    pub fn is_undeliverable_draft_direct_chat(&self, id: &str) -> bool {
        self.undeliverable_draft_direct_chat_ids.contains(id)
    }

    pub fn note_undeliverable_draft_message(
        &mut self,
        id: &str,
        preview: &str,
        content_type: MessageContentType,
        cx: &mut Context<Self>,
    ) {
        self.restore_undeliverable_draft_direct_chats(cx);
        let sender_id =
            try_read::<AuthServicesStore>(cx).and_then(AuthServicesStore::effective_user_id_for_wire).unwrap_or_default();
        let Some(draft) = self.draft_direct_chat_by_id.get_mut(id) else {
            return;
        };

        let now = RiseClock::now().time_interval_since_1970();
        let mut message = GetMessageMessage::new(String::new());
        message.chat_id = id.to_owned();
        message.sender_id = sender_id;
        message.content_type = content_type;
        message.text = preview.to_owned();
        message.created_at = now;
        message.delivery_status = MessageDeliveryStatus::Failed;

        draft.updated_at = now;
        draft.last_activity = Some(now);
        draft.last_message = Some(message);
        if self.undeliverable_draft_direct_chat_ids.contains(id) {
            self.persist_undeliverable_draft_direct_chats(cx);
        }
        cx.notify();
    }

    pub fn known_direct_chat_id(&self, peer_user_id: &str, cx: &App) -> String {
        let peer = trimming_whitespaces_and_newlines(peer_user_id);
        if peer.is_empty() {
            return String::new();
        }

        let removed = self.removal_marks_for_known_direct_chats(cx);
        let mut known = self.current_chat_infos(cx);
        known.extend(self.archived_chats_memory_only(cx));
        known.extend(self.message_requests_memory_only(cx));
        known
            .into_iter()
            .find(|chat| {
                chat.r#type == ChatType::Direct
                    && !is_contact_join_virtual_chat_id(&chat.id)
                    && !Self::is_draft_direct_chat_id(&chat.id)
                    && !removed.contains(&chat.id)
                    && chat.participant.id == peer
            })
            .map(|chat| chat.id)
            .unwrap_or_default()
    }

    pub fn locally_removed_chat_ids(&self) -> HashSet<String> {
        self.pending_delete_chat_ids
            .iter()
            .chain(&self.confirmed_removed_chat_ids)
            .chain(&self.left_chat_ids)
            .cloned()
            .collect()
    }

    fn removal_marks_for_known_direct_chats(&self, cx: &App) -> HashSet<String> {
        if try_read::<RiseConversationPresentation>(cx).is_some_and(RiseConversationPresentation::is_active) {
            self.pending_delete_chat_ids.clone()
        } else {
            self.locally_removed_chat_ids()
        }
    }

    pub fn adopt_known_chats_for_draft_direct_chats(&mut self, cx: &mut Context<Self>) {
        if self.draft_direct_chat_by_id.is_empty() {
            return;
        }

        let mut draft_id_by_peer: HashMap<String, String> = HashMap::new();
        for (draft_id, draft) in &self.draft_direct_chat_by_id {
            if self.temp_chat_id_to_real.contains_key(draft_id)
                || self.undeliverable_draft_direct_chat_ids.contains(draft_id)
            {
                continue;
            }
            let peer_id = trimming_whitespaces_and_newlines(&draft.participant.id);
            if peer_id.is_empty() || self.opening_direct_chat_for.contains(peer_id) {
                continue;
            }
            draft_id_by_peer.insert(peer_id.to_owned(), draft_id.clone());
        }
        if draft_id_by_peer.is_empty() {
            return;
        }

        let removed = self.removal_marks_for_known_direct_chats(cx);
        let mut known = self.current_chat_infos(cx);
        known.extend(self.archived_chats(cx));
        known.extend(self.message_requests(cx));
        let mut adopted: Vec<(String, String)> = Vec::new();
        for chat in known.iter().filter(|chat| chat.r#type == ChatType::Direct) {
            let Some(draft_id) = draft_id_by_peer.get(&chat.participant.id) else {
                continue;
            };
            if is_contact_join_virtual_chat_id(&chat.id)
                || Self::is_draft_direct_chat_id(&chat.id)
                || removed.contains(&chat.id)
            {
                continue;
            }
            adopted.push((draft_id.clone(), chat.id.clone()));
            draft_id_by_peer.remove(&chat.participant.id);
            if draft_id_by_peer.is_empty() {
                break;
            }
        }
        for (draft_id, real_id) in adopted {
            if self.left_chat_ids.contains(&real_id) || self.confirmed_removed_chat_ids.contains(&real_id) {
                self.clear_pending_delete_marks(&real_id, cx);
            }
            self.finalize_created_chat(&draft_id, &real_id, cx);
        }
    }
}

pub struct DirectChatWaiter {
    pub on_open: Box<dyn FnOnce(String, &mut App)>,
    pub on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
}

impl ChatServicesStore {
    pub fn enqueue_direct_chat_waiter(
        &mut self,
        target_id: &str,
        on_open: Box<dyn FnOnce(String, &mut App)>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
    ) {
        self.direct_chat_waiters_by_target
            .entry(target_id.to_owned())
            .or_default()
            .push(DirectChatWaiter { on_open, on_error });
    }

    pub fn begin_opening_direct_chat(&mut self, target_id: &str, cx: &mut Context<Self>) -> bool {
        let inserted = self.opening_direct_chat_for.insert(target_id.to_owned());
        cx.notify();
        inserted
    }

    pub fn finish_opening_direct_chat(
        &mut self,
        target_id: &str,
        real_id: Option<&str>,
        error: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.opening_direct_chat_for.remove(target_id);
        cx.notify();
        let waiters = self.direct_chat_waiters_by_target.remove(target_id).unwrap_or_default();
        if waiters.is_empty() {
            return;
        }
        let real_id = real_id.filter(|id| !id.is_empty()).map(str::to_owned);
        let error = error.unwrap_or_default().to_owned();
        cx.defer(move |cx| {
            for waiter in waiters {
                match &real_id {
                    Some(real_id) => (waiter.on_open)(real_id.clone(), cx),
                    None => {
                        if let Some(on_error) = waiter.on_error {
                            on_error(error.clone(), cx);
                        }
                    }
                }
            }
        });
    }
}
