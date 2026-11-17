use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rise_canon::RiseLimits;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseChatListWireSnapshot, RiseConversationEntityStamp,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;

#[derive(Clone)]
pub struct RiseChatListPresentationSnapshot {
    pub generation: u64,
    pub source: RiseChatListWireSnapshot,
    pub chats: Arc<Vec<ChatInfo>>,
    pub wires: Arc<Vec<RiseStoredChat>>,
    pub index: Arc<HashMap<String, ChatInfo>>,
    pub confirmed_removed_chat_ids: HashSet<String>,
}

pub enum RiseChatPresentationInputPolicy {}

impl RiseChatPresentationInputPolicy {
    pub fn has_same_content(lhs: &RiseChatListWireSnapshot, rhs: &RiseChatListWireSnapshot) -> bool {
        lhs.account_id == rhs.account_id
            && lhs.kind == rhs.kind
            && lhs.local_ids == rhs.local_ids
            && lhs.entity_stamps == rhs.entity_stamps
            && lhs.loaded_row_count == rhs.loaded_row_count
            && lhs.server_total_count == rhs.server_total_count
            && lhs.server_total_unread_count == rhs.server_total_unread_count
            && lhs.server_unread_chats_count == rhs.server_unread_chats_count
            && lhs.server_total_unread_media_count == rhs.server_total_unread_media_count
            && lhs.cursor == rhs.cursor
            && lhs.has_more == rhs.has_more
            && lhs.load_state == rhs.load_state
            && lhs.error_code == rhs.error_code
    }
}

struct RiseChatPresentationPreparerCacheEntry {
    stamp: RiseConversationEntityStamp,
    model: ChatInfo,
}

pub struct RiseChatPresentationPreparer {
    model_cache: HashMap<String, RiseChatPresentationPreparerCacheEntry>,
    snapshots: HashMap<RiseChatListKind, RiseChatListPresentationSnapshot>,
    confirmed_removed_chat_ids: HashSet<String>,
    generation: u64,
    model_cache_limit: usize,
}

impl Default for RiseChatPresentationPreparer {
    fn default() -> Self {
        Self {
            model_cache: HashMap::new(),
            snapshots: HashMap::new(),
            confirmed_removed_chat_ids: HashSet::new(),
            generation: 0,
            model_cache_limit: usize::try_from(Self::model_cache_entry_limit()).unwrap_or(0),
        }
    }
}

impl RiseChatPresentationPreparer {
    fn model_cache_entry_limit() -> i64 {
        RiseLimits::client_value("chat_presentation_model_cache_entries")
    }

    pub fn prepare(
        &mut self,
        source: &RiseChatListWireSnapshot,
        confirmed_removals: &HashSet<String>,
    ) -> Option<RiseChatListPresentationSnapshot> {
        if source.rows.len() != source.local_ids.len() || source.rows.len() != source.entity_stamps.len() {
            return None;
        }
        let prepared = self.prepare_snapshot(source, confirmed_removals);
        while self.model_cache.len() > self.model_cache_limit {
            let Some(key) = self.model_cache.keys().next().cloned() else { break };
            self.model_cache.remove(&key);
        }
        prepared
    }

    fn prepare_snapshot(
        &mut self,
        source: &RiseChatListWireSnapshot,
        confirmed_removals: &HashSet<String>,
    ) -> Option<RiseChatListPresentationSnapshot> {
        let previous = self.snapshots.get(&source.kind);
        let reusable = previous
            .filter(|previous| previous.source.local_ids == source.local_ids && previous.source.entity_stamps == source.entity_stamps);
        let (chats, wires, index) = if let Some(previous) = reusable {
            (previous.chats.clone(), previous.wires.clone(), previous.index.clone())
        } else {
            let mut models: Vec<ChatInfo> = Vec::with_capacity(source.rows.len());
            let mut prepared_wires: Vec<RiseStoredChat> = Vec::with_capacity(source.rows.len());
            let mut prepared_index: HashMap<String, ChatInfo> = HashMap::with_capacity(source.rows.len());
            for offset in 0..source.rows.len() {
                let local_id = &source.local_ids[offset];
                let stamp = &source.entity_stamps[offset];
                let wire = &source.rows[offset].wire;
                let model = match self.model_cache.get(local_id) {
                    Some(cached) if cached.stamp == *stamp => cached.model.clone(),
                    _ => {
                        let model = wire.to_chat_info();
                        self.model_cache.insert(
                            local_id.clone(),
                            RiseChatPresentationPreparerCacheEntry { stamp: *stamp, model: model.clone() },
                        );
                        model
                    }
                };
                prepared_index.entry(model.id.clone()).or_insert_with(|| model.clone());
                models.push(model);
                prepared_wires.push(wire.clone());
            }
            (Arc::new(models), Arc::new(prepared_wires), Arc::new(prepared_index))
        };
        self.confirmed_removed_chat_ids.extend(confirmed_removals.iter().cloned());
        self.confirmed_removed_chat_ids.retain(|chat_id| !index.contains_key(chat_id));
        if let Some(previous) = previous
            && RiseChatPresentationInputPolicy::has_same_content(&previous.source, source)
            && previous.confirmed_removed_chat_ids == self.confirmed_removed_chat_ids
        {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        let prepared = RiseChatListPresentationSnapshot {
            generation: self.generation,
            source: source.clone(),
            chats,
            wires,
            index,
            confirmed_removed_chat_ids: self.confirmed_removed_chat_ids.clone(),
        };
        self.snapshots.insert(source.kind, prepared.clone());
        Some(prepared)
    }
}

#[derive(Default)]
pub struct RiseChatListPresentationBox {
    pub value: Option<RiseChatListPresentationSnapshot>,
}
