use std::collections::HashSet;

use gpui::Context;
use rise_canon::rise_key::message as message_key;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::get_sai_server_id::{GetSaiServerId, GetSaiServerIdOptions};
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    DeleteAllMessagesFromUserGatewayDTO, DeleteMessagesGatewayDTO, MessagePayloadCodec,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

#[derive(Clone, Debug, PartialEq)]
pub struct MessageDeleteSnapshotEntry {
    pub index: usize,
    pub row: Map<String, Value>,
}

impl MessageServicesStore {
    pub fn prepare_delete_optimistic(
        &mut self,
        message_ids: &[String],
        chat_id: &str,
        cx: &mut Context<Self>,
    ) -> (Vec<String>, Vec<MessageDeleteSnapshotEntry>) {
        let wire_ids: Vec<String> =
            message_ids.iter().map(|message_id| self.server_message_id(message_id, chat_id, cx)).collect();
        let aliases: Vec<String> = message_ids.iter().chain(wire_ids.iter()).cloned().collect();
        let snapshot = self.apply_delete_optimistic(&aliases, chat_id, false, cx);
        (wire_ids, snapshot)
    }

    pub fn apply_delete_optimistic(
        &mut self,
        message_ids: &[String],
        chat_id: &str,
        mirrors_latest: bool,
        cx: &mut Context<Self>,
    ) -> Vec<MessageDeleteSnapshotEntry> {
        let set: HashSet<String> = message_ids.iter().cloned().collect();
        let cache_key = Self::messages_cache_key(chat_id);
        let (did_mutate, snapshot) = Self::remove_messages(&set, &cache_key, cx);
        if did_mutate && !snapshot.is_empty() {
            self.mark_messages_cache_for_disk_persistence(chat_id, cx);
        }
        if mirrors_latest {
            let latest_cache_key = Self::latest_messages_cache_key(chat_id);
            let has_latest = SaiWsResponseCache::current_payload(
                &latest_cache_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
            .is_some();
            if has_latest {
                let (latest_did_mutate, _) = Self::remove_messages(&set, &latest_cache_key, cx);
                if latest_did_mutate {
                    self.mark_latest_messages_cache_for_disk_persistence(chat_id, cx);
                }
            }
        }
        snapshot
    }

    pub fn delete_messages_success_handler(
        &mut self,
        any: Option<&Value>,
        chat_id: &str,
        snapshot: Vec<MessageDeleteSnapshotEntry>,
        cx: &mut Context<Self>,
    ) {
        let deleted = MessagePayloadCodec::decode::<DeleteMessagesGatewayDTO>(any)
            .map(|dto| dto.deleted_message_ids)
            .unwrap_or_default();

        if !deleted.is_empty() {
            self.apply_delete_optimistic(&deleted, chat_id, true, cx);
        }
        let deleted_set: HashSet<&str> = deleted.iter().map(String::as_str).collect();
        let options = GetSaiServerIdOptions::default();
        let (confirmed, unconfirmed): (Vec<_>, Vec<_>) = snapshot.into_iter().partition(|entry| {
            deleted_set.contains(GetSaiServerId::from_dictionary(&entry.row, &options).as_str())
        });
        let confirmed_indices: HashSet<usize> = confirmed.iter().map(|entry| entry.index).collect();
        self.restore_delete_optimistic(unconfirmed, chat_id, &confirmed_indices, cx);
    }

    pub fn delete_messages_error_handler(
        &mut self,
        error: &anyhow::Error,
        chat_id: &str,
        snapshot: Vec<MessageDeleteSnapshotEntry>,
        cx: &mut Context<Self>,
    ) {
        Sai::log("Message", || format!("delete_messages({chat_id}) error: {error}"), SaiLogKind::Network, cx);
        self.restore_delete_optimistic(snapshot, chat_id, &HashSet::new(), cx);
    }

    pub fn restore_delete_optimistic(
        &mut self,
        mut snapshot: Vec<MessageDeleteSnapshotEntry>,
        chat_id: &str,
        confirmed_indices: &HashSet<usize>,
        cx: &mut Context<Self>,
    ) {
        if snapshot.is_empty() {
            return;
        }
        snapshot.sort_by_key(|entry| entry.index);

        let cache_key = Self::messages_cache_key(chat_id);
        let did_mutate = SaiUpdater::update_json_value(
            &cache_key,
            |context| {
                let mut root =
                    MessagePayloadCodec::dictionary(context.payload.as_ref()).unwrap_or_else(Self::empty_messages_root);
                let mut rows = MessagePayloadCodec::array_of_dictionaries(root.get(message_key::MESSAGES));
                for entry in snapshot {
                    let aliases = Self::delete_row_aliases(&entry.row);
                    if !rows.iter().all(|row| Self::delete_row_aliases(row).is_disjoint(&aliases)) {
                        continue;
                    }
                    let confirmed_before = confirmed_indices.iter().filter(|index| **index < entry.index).count();
                    let restored_index = entry.index.saturating_sub(confirmed_before);
                    rows.insert(restored_index.min(rows.len()), entry.row);
                }
                root.insert(message_key::MESSAGES.to_owned(), Self::rows_value(rows));
                Ok(Value::Object(root))
            },
            SaiUpdaterWriteScope::Cache,
            cx,
        )
        .is_ok();
        if did_mutate {
            self.mark_messages_cache_for_disk_persistence(chat_id, cx);
        }
    }

    fn empty_messages_root() -> Map<String, Value> {
        Map::from_iter([(message_key::MESSAGES.to_owned(), Value::Array(Vec::new()))])
    }

    fn rows_value(rows: Vec<Map<String, Value>>) -> Value {
        Value::Array(rows.into_iter().map(Value::Object).collect())
    }

    fn delete_row_aliases(row: &Map<String, Value>) -> HashSet<String> {
        let options = GetSaiServerIdOptions::default();
        [message_key::ID, options.path_to_server_id.as_str(), message_key::TEMP_ID]
            .into_iter()
            .filter_map(|key| {
                let trimmed = trimming_whitespaces_and_newlines(row.get(key)?.as_str()?);
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            })
            .collect()
    }

    fn remove_messages(
        aliases: &HashSet<String>,
        cache_key: &str,
        cx: &mut Context<Self>,
    ) -> (bool, Vec<MessageDeleteSnapshotEntry>) {
        let mut snapshot = Vec::new();
        let did_mutate = SaiUpdater::update_json_value(
            cache_key,
            |context| {
                let mut root =
                    MessagePayloadCodec::dictionary(context.payload.as_ref()).unwrap_or_else(Self::empty_messages_root);
                let rows = MessagePayloadCodec::array_of_dictionaries(root.get(message_key::MESSAGES));
                let mut kept = Vec::with_capacity(rows.len());
                for (index, row) in rows.into_iter().enumerate() {
                    if Self::delete_row_aliases(&row).is_disjoint(aliases) {
                        kept.push(row);
                    } else {
                        snapshot.push(MessageDeleteSnapshotEntry { index, row });
                    }
                }
                root.insert(message_key::MESSAGES.to_owned(), Self::rows_value(kept));
                Ok(Value::Object(root))
            },
            SaiUpdaterWriteScope::Cache,
            cx,
        )
        .is_ok();
        (did_mutate, snapshot)
    }

    pub fn apply_delete_all_from_user_optimistic(
        &mut self,
        chat_id: &str,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.mutate_messages_list(
            chat_id,
            SaiUpdaterWriteScope::Cache,
            |messages| messages.retain(|message| message.sender_id != target_user_id),
            cx,
        );
    }

    pub fn delete_all_messages_from_user_success_handler(
        &mut self,
        any: Option<&Value>,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(dto) = MessagePayloadCodec::decode::<DeleteAllMessagesFromUserGatewayDTO>(any) else {
            return;
        };

        if !dto.deleted_message_ids.is_empty() {
            self.apply_delete_optimistic(&dto.deleted_message_ids, chat_id, true, cx);
        }
    }

    pub fn delete_all_messages_from_user_error_handler(
        &mut self,
        error: &anyhow::Error,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) {
        Sai::log(
            "Message",
            || format!("delete_all_messages_from_user({chat_id}) error: {error}"),
            SaiLogKind::Network,
            cx,
        );
    }
}
